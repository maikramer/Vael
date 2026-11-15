//! Player do Vael — o primeiro corte da aposentadoria da engine.
//!
//! O spawner da engine ainda cria o `<PlayerGLTF>` (Gltf, collider e
//! `TnuaController`), mas com `player_camera:false` os sistemas
//! dela não correm. Este módulo anexa o [`VaelController`] e conduz o herói
//! (WASD, salto, steering da `OrbitCamera`, chão analítico e teleportes); o
//! estado do `bevy_ext_core::Player` é sincronizado para a animação da engine
//! continuar a ler velocidade/chão enquanto essa camada não é portada. A
//! câmara de terceira pessoa vive em [`crate::camera`].

use std::sync::{Arc, Mutex};

use avian3d::prelude::{LinearVelocity, Position, Rotation};
use bevy::prelude::*;
use bevy_ext_core::{
    GROUND_PROBE, KNOCKBACK_EPSILON, Knockback, Player, PlayerBody, TeleportSettle, footstep_due,
    knockback_decay_factor,
};
use bevy_tnua::TnuaToggle;
use bevy_tnua::builtins::TnuaBuiltinJump;
use bevy_tnua::prelude::TnuaController;
use mlua::{Lua, Table, Value};
use viber::luau::host::LuaScriptHost;
use viber::player::HeroScheme;
use viber::recipes::spawn::OrbitCamera;

use crate::components::VaelPlayer;

pub const GRAVITY: f32 = 60.0;
pub const TERMINAL_VELOCITY: f32 = 55.0;
pub const CAMERA_TURN_SPEED: f32 = 2.5;
pub const SIDE_MOVE_FACTOR: f32 = 0.6;
pub const JUMP_BUFFER: f32 = 0.1;
pub const COYOTE_TIME: f32 = 0.1;
pub const JUMP_COOLDOWN: f32 = 0.2;
pub const ACCEL_TAU: f32 = 0.08;
/// Travagem no chão sem input. Era um corte seco (velocidade 0 no frame em
/// que se larga a tecla): o blend da animação saltava de marcha para idle
/// sem tempo para o cross-fade. Curta o bastante para não deslizar (~0,3 m
/// a correr).
pub const DECEL_TAU: f32 = 0.06;
/// Abaixo disto a travagem acaba em zero (o low-pass nunca chega lá).
pub const STOP_EPSILON: f32 = 0.05;
/// Rampa do giro de câmara com A/D: o giro entra e sai em ~0,1 s em vez de
/// arrancar a 143°/s no primeiro frame.
pub const STEER_TAU: f32 = 0.09;
pub const AIR_TAU: f32 = 0.3;
/// Duração do avanço curto (lunge) de um golpe.
pub const LUNGE_SECS: f32 = 0.16;
/// Multiplicador do giro do herói enquanto um golpe o vira para o alvo.
pub const ATTACK_TURN_BOOST: f32 = 2.5;
/// Raio horizontal de interação do herói com a água (m).
pub const PLAYER_WATER_RADIUS: f32 = 0.42;
/// Profundidade (m) acima da qual o arrasto hidrodinâmico começa a atuar.
pub const WADE_DRAG_START_DEPTH: f32 = 0.15;
/// Profundidade (m) acima da qual a flutuabilidade de Arquimedes sustenta o herói.
pub const SWIM_DEPTH: f32 = 0.88;
/// Profundidade de equilíbrio a nadar/flutuar sem afundar.
pub const SWIM_IDLE_DEPTH: f32 = 1.00;
/// Velocidade vertical mínima de queda (m/s) para amortecimento e salpico de entrada.
pub const WATER_ENTRY_SPEED_MIN: f32 = 3.0;

/// Esquiva do herói (`viber.vael.dodge()`), escolhida pelas teclas de
/// movimento no momento do pedido.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Reflect)]
pub enum DodgeKind {
    /// W (ou qualquer direção num interior): rola nessa direção e vira-se.
    Roll,
    /// A/D: salto lateral, sem rodar o corpo (o herói continua de frente).
    Side,
    /// S ou nenhuma tecla: salto curto para trás, de frente para a ameaça.
    Back,
}

impl DodgeKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Roll => "roll",
            Self::Side => "side",
            Self::Back => "back",
        }
    }
    /// (duração s, distância m, altura do salto m)
    pub fn profile(self) -> (f32, f32, f32) {
        match self {
            Self::Roll => (0.5, 4.2, 0.0),
            Self::Side => (0.4, 3.4, 0.55),
            Self::Back => (0.34, 2.6, 0.35),
        }
    }
    pub fn keeps_facing(self) -> bool {
        !matches!(self, Self::Roll)
    }
}

/// Deslocação com perfil ease-out (rápida no início, assenta no fim): a
/// distância total é exata e o fim encaixa na locomoção sem solavanco.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Burst {
    pub dir: Vec3,
    pub distance: f32,
    pub duration: f32,
    pub t: f32,
}

impl Burst {
    pub fn new(dir: Vec3, distance: f32, duration: f32) -> Self {
        Self {
            dir: dir.with_y(0.0).normalize_or_zero(),
            distance,
            duration: duration.max(1e-3),
            t: 0.0,
        }
    }
    /// Velocidade no instante corrente: s(u) = D·(1 − (1 − u)²).
    pub fn speed(&self) -> f32 {
        let u = (self.t / self.duration).clamp(0.0, 1.0);
        2.0 * self.distance / self.duration * (1.0 - u)
    }
    pub fn done(&self) -> bool {
        self.t >= self.duration
    }
}

pub fn dodge_kind(move_x: f32, move_forward: f32, interior: bool) -> DodgeKind {
    let has_input = move_x.abs() > 0.1 || move_forward.abs() > 0.1;
    if interior {
        return if has_input {
            DodgeKind::Roll
        } else {
            DodgeKind::Back
        };
    }
    if move_x.abs() > 0.1 {
        DodgeKind::Side
    } else if move_forward > 0.1 {
        DodgeKind::Roll
    } else {
        DodgeKind::Back
    }
}

/// Direção mundial da esquiva. Lateral e rolamento são relativos à vista;
/// o salto para trás é relativo às COSTAS do herói (mantém a frente).
pub fn dodge_direction(
    kind: DodgeKind,
    move_x: f32,
    move_forward: f32,
    view_yaw_deg: f32,
    facing: Vec3,
    interior: bool,
) -> Vec3 {
    let back = -facing.with_y(0.0).normalize_or(Vec3::Z);
    let dir = match kind {
        DodgeKind::Back => return back,
        DodgeKind::Side => process_input(0.0, move_x.signum(), view_yaw_deg),
        DodgeKind::Roll if interior => process_input(move_forward, move_x, view_yaw_deg),
        DodgeKind::Roll => process_input(move_forward.signum(), 0.0, view_yaw_deg),
    };
    if dir.length_squared() > 0.0 {
        dir
    } else {
        back
    }
}

/// Pedido de movimento de um golpe (`viber.vael.attack_motion`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AttackMotion {
    /// Quanto tempo o golpe trava o passo e manda na frente do herói.
    pub secs: f32,
    /// Multiplicador da velocidade a pé durante `secs` (0..1).
    pub slow: f32,
    /// Avanço curto (m) na direção do golpe.
    pub lunge: f32,
    /// Direção do golpe (para onde o herói se vira); `None` = a frente atual.
    pub face: Option<Vec2>,
}

/// Estado partilhado Lua ↔ controller. O Lua PEDE (esquiva, movimento do
/// golpe); o controller executa no frame seguinte e espelha o que o Lua
/// precisa para decidir (input, chão, esquiva em curso).
#[derive(Debug, Default)]
pub struct HeroMotionShared {
    pub move_x: f32,
    pub move_forward: f32,
    pub grounded: bool,
    pub dodging: bool,
    pub interior: bool,
    pub dodge: Option<DodgeKind>,
    pub attack: Option<AttackMotion>,
}

#[derive(Resource, Clone, Default)]
pub struct HeroMotionBus(pub Arc<Mutex<HeroMotionShared>>);

impl HeroMotionBus {
    fn lock(&self) -> std::sync::MutexGuard<'_, HeroMotionShared> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// `viber.vael.dodge()`: aceita se o herói está no chão e livre.
    pub fn request_dodge(&self) -> Option<DodgeKind> {
        let mut shared = self.lock();
        if !shared.grounded || shared.dodging || shared.dodge.is_some() {
            return None;
        }
        let kind = dodge_kind(shared.move_x, shared.move_forward, shared.interior);
        shared.dodge = Some(kind);
        Some(kind)
    }

    pub fn request_attack(&self, motion: AttackMotion) {
        self.lock().attack = Some(motion);
    }
}

/// Queda abaixo do topo do mundo que já não é gruta: o herói furou o
/// collider e é resgatado para a superfície.
pub const VOID_RESCUE_DEPTH: f32 = 250.0;

/// Estado de movimento do herói. É a cópia local do subconjunto do
/// `bevy_ext_core::Player` que o controlador usa.
#[derive(Component, Debug, Clone, Reflect)]
#[reflect(Component)]
pub struct VaelController {
    pub speed: f32,
    pub sprint_multiplier: f32,
    pub jump_height: f32,
    pub rotation_speed: f32,
    pub vel_y: f32,
    pub vel_x: f32,
    pub vel_z: f32,
    pub grounded: bool,
    pub can_jump: bool,
    pub is_jumping: bool,
    pub jump_cooldown: f32,
    pub last_grounded_time: f32,
    pub jump_buffer_time: f32,
    pub last_steer_time: f32,
    pub last_moving_time: f32,
    /// Giro de câmara corrente (−1..1), suavizado por [`STEER_TAU`].
    pub steer: f32,
    /// Esquiva em curso.
    #[reflect(ignore)]
    pub dodge: Option<(DodgeKind, Burst)>,
    /// Avanço curto de um golpe.
    #[reflect(ignore)]
    pub lunge: Option<Burst>,
    /// Golpe em curso: até quando trava o passo, quanto, e para onde vira.
    pub attack_until: f32,
    pub attack_slow: f32,
    #[reflect(ignore)]
    pub attack_face: Option<Vec3>,
    /// Já viu o [`viber::player::TeleportSettle`] corrente (distingue a
    /// CHEGADA de um teleporte da tutela a meio). Ver `player_movement`.
    pub settle_seen: bool,
    /// Estava submerso no frame anterior (histerese de entrada/som na água).
    pub in_water: bool,
}

impl Default for VaelController {
    fn default() -> Self {
        Self {
            speed: 4.0,
            sprint_multiplier: 1.5,
            jump_height: 2.3,
            rotation_speed: 10.0,
            vel_y: 0.0,
            vel_x: 0.0,
            vel_z: 0.0,
            grounded: true,
            can_jump: true,
            is_jumping: false,
            jump_cooldown: 0.0,
            last_grounded_time: f32::NEG_INFINITY,
            jump_buffer_time: f32::NEG_INFINITY,
            last_steer_time: f32::NEG_INFINITY,
            last_moving_time: f32::INFINITY,
            steer: 0.0,
            dodge: None,
            lunge: None,
            attack_until: f32::NEG_INFINITY,
            attack_slow: 1.0,
            attack_face: None,
            settle_seen: false,
            in_water: false,
        }
    }
}

/// Anexa o controller e a cinemática de água às entidades marcadas com `Player`.
fn attach_controllers(
    mut commands: Commands,
    players: Query<
        (
            Entity,
            Option<&VaelController>,
            Option<&viber::terrain::WaterKinematics>,
        ),
        With<Player>,
    >,
) {
    for (entity, controller, kin) in &players {
        commands.entity(entity).insert((VaelPlayer, PlayerBody));
        if controller.is_none() {
            commands.entity(entity).insert(VaelController::default());
        }
        if kin.is_none() {
            commands
                .entity(entity)
                .insert(viber::terrain::WaterKinematics {
                    velocity: Vec3::ZERO,
                    body_radius: PLAYER_WATER_RADIUS,
                    contact_y_offset: 0.0,
                });
        }
    }
}

pub(crate) fn no_editor_camera(
    cameras: Query<(), With<viber::editor_camera::EditorCamera>>,
) -> bool {
    cameras.is_empty()
}

/// Movimento do herói. A câmara corre depois deste conjunto.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct PlayerMovementSet;

pub struct VaelPlayerPlugin;

impl Plugin for VaelPlayerPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<VaelController>()
            .init_resource::<HeroMotionBus>()
            .add_systems(
                Startup,
                install_motion_api.after(crate::combat::install_combat_api),
            )
            .add_systems(PostStartup, attach_controllers)
            .add_systems(
                viber::scene_loader::SceneLoad,
                attach_controllers.after(viber::recipes::spawn::startup),
            )
            .add_systems(
                Update,
                player_movement
                    .in_set(PlayerMovementSet)
                    // A intenção (basis/salto) tem de estar escrita antes da
                    // lógica do tnua — senão o controller usa a do frame
                    // anterior.
                    .before(bevy_tnua::prelude::TnuaPipelineSystems::Logic)
                    // O driver de animação da engine lê `grounded`/`vel_*`
                    // do `Player` que este sistema espelha; a ordem dele é
                    // contra o `player_movement` da ENGINE (desligado com
                    // `player_camera:false`), por isso sem isto a animação
                    // via o estado de um frame atrás, ao acaso.
                    .before(viber::animation::drive_player_animation)
                    .run_if(no_editor_camera),
            )
            .add_systems(
                Update,
                world_border_clamp
                    .after(PlayerMovementSet)
                    .run_if(no_editor_camera),
            );
    }
}

/// Arranque: `viber.vael.dodge()` e `viber.vael.attack_motion{…}`.
fn install_motion_api(host: Option<Res<LuaScriptHost>>, bus: Res<HeroMotionBus>) {
    let Some(host) = host else {
        warn!("vael: movimento sem host Luau — `viber.vael.dodge` não instalado");
        return;
    };
    match install_motion(&host.lua, bus.clone()) {
        Ok(()) => info!("vael: esquiva/golpe em Rust ligados (`viber.vael`)"),
        Err(err) => error!("vael: falha a instalar `viber.vael.dodge`: {err}"),
    }
}

fn table_f32(table: &Table, key: &str, default: f32) -> f32 {
    match table.raw_get::<Value>(key) {
        Ok(Value::Integer(n)) => n as f32,
        Ok(Value::Number(n)) if n.is_finite() => n as f32,
        _ => default,
    }
}

pub(crate) fn install_motion(lua: &Lua, bus: HeroMotionBus) -> mlua::Result<()> {
    use crate::vault::{vael_fn, vael_table};
    let vael = vael_table(lua)?;
    // viber.vael.dodge() -> kind, duração | nil. `kind` ∈ roll/side/back
    // (o script escolhe o clip); nil = no ar ou já a esquivar.
    vael_fn!(lua, vael, "dodge", [bus = bus.clone()], move |_, ()| {
        let kind = bus.request_dodge();
        Ok((kind.map(DodgeKind::name), kind.map(|kind| kind.profile().0)))
    });
    // viber.vael.attack_motion{ secs, slow, lunge, fx, fz }
    vael_fn!(
        lua,
        vael,
        "attack_motion",
        [bus = bus.clone()],
        move |_, opts: Option<Table>| {
            let Some(opts) = opts else {
                return Ok(());
            };
            let (fx, fz) = (table_f32(&opts, "fx", 0.0), table_f32(&opts, "fz", 0.0));
            let face = Vec2::new(fx, fz);
            bus.request_attack(AttackMotion {
                secs: table_f32(&opts, "secs", 0.3).clamp(0.0, 2.0),
                slow: table_f32(&opts, "slow", 0.3).clamp(0.0, 1.0),
                lunge: table_f32(&opts, "lunge", 0.0).clamp(0.0, 3.0),
                face: (face.length_squared() > 1e-6).then(|| face.normalize()),
            });
            Ok(())
        }
    );
    Ok(())
}

fn low_pass_factor(dt: f32, tau: f32) -> f32 {
    1.0 - (-dt / tau.max(1e-4)).exp()
}

fn process_input(forward: f32, strafe: f32, yaw_deg: f32) -> Vec3 {
    let (sin, cos) = yaw_deg.to_radians().sin_cos();
    let dir = Vec3::new(
        strafe * cos - forward * sin,
        0.0,
        -strafe * sin - forward * cos,
    );
    dir.normalize_or_zero()
}

fn input_magnitude(forward: f32, strafe: f32) -> f32 {
    (forward * forward + strafe * strafe).sqrt().min(1.0)
}

fn movement_tau(grounded: bool, input_active: bool) -> f32 {
    if !grounded {
        AIR_TAU
    } else if input_active {
        ACCEL_TAU
    } else {
        DECEL_TAU
    }
}

fn integrate_horizontal(
    vel_x: f32,
    vel_z: f32,
    desired: Vec3,
    grounded: bool,
    input_active: bool,
    dt: f32,
) -> (f32, f32) {
    let a = low_pass_factor(dt, movement_tau(grounded, input_active));
    let (x, z) = (
        vel_x + (desired.x - vel_x) * a,
        vel_z + (desired.z - vel_z) * a,
    );
    if grounded && !input_active && x * x + z * z < STOP_EPSILON * STOP_EPSILON {
        return (0.0, 0.0);
    }
    (x, z)
}

fn jump_velocity(height: f32) -> f32 {
    (2.0 * GRAVITY * height.max(0.0)).sqrt()
}

/// Fator de velocidade horizontal (0.58..1.0) em três regimes de profundidade:
/// margem rasa (`<= 0.15 m`: sem trava), vau médio (`0.15..0.42 m`: resistência
/// leve nos tornozelos) e nado/água profunda (`0.42..0.88 m`: arrasto pesado até 0.58).
pub fn water_submersion_drag(submersion: f32) -> f32 {
    if submersion <= WADE_DRAG_START_DEPTH {
        return 1.0;
    }
    let knee_depth = 0.42_f32;
    if submersion < knee_depth {
        let t = ((submersion - WADE_DRAG_START_DEPTH) / (knee_depth - WADE_DRAG_START_DEPTH))
            .clamp(0.0, 1.0);
        let smooth = t * t * (3.0 - 2.0 * t);
        return 1.0 - 0.13 * smooth;
    }
    let t = ((submersion - knee_depth) / (SWIM_DEPTH - knee_depth).max(0.1)).clamp(0.0, 1.0);
    let smooth = t * t * (3.0 - 2.0 * t);
    0.87 - 0.29 * smooth
}

/// Correnteza de rio aplicada ao jogador segundo a profundidade: em bordas rasas
/// o atrito com o fundo segura o passo (`0.0`); em águas mais fundas a correnteza
/// empurra suavemente rio abaixo.
pub fn water_flow_drift(submersion: f32, flow: Vec2) -> Vec2 {
    if submersion <= 0.18 || flow.length_squared() <= 1e-6 {
        return Vec2::ZERO;
    }
    let t = ((submersion - 0.18) / (SWIM_DEPTH - 0.18)).clamp(0.0, 1.0);
    let smooth = t * t * (3.0 - 2.0 * t);
    flow * (0.24 * smooth)
}

/// Resultado da resposta vertical à água num passo: velocidade ajustada,
/// estado de contacto atualizado e velocidade de mergulho (0 fora de entradas bruscas).
pub fn integrate_water_vertical(
    mut vel_y: f32,
    submersion: f32,
    was_in_water: bool,
    grounded: bool,
    dt: f32,
) -> (f32, bool, f32) {
    let mut plunge_speed = 0.0;
    let in_water = if was_in_water {
        submersion > 0.12
    } else {
        submersion > 0.18
    };
    if in_water && !was_in_water && vel_y < -WATER_ENTRY_SPEED_MIN {
        plunge_speed = -vel_y;
        // Amortecimento imediato de impacto na entrada da coluna de água.
        vel_y *= 0.35;
    }
    let dt = dt.clamp(0.0, 0.05);
    if submersion > SWIM_DEPTH {
        let spring_accel = GRAVITY * (submersion - SWIM_IDLE_DEPTH) * 1.15;
        vel_y = (vel_y + spring_accel * dt) * (-8.5 * dt).exp();
    } else if !grounded || vel_y > 0.0 {
        vel_y -= GRAVITY * dt;
    }
    (vel_y, in_water, plunge_speed)
}

fn can_perform_jump(
    now: f32,
    buffer_time: f32,
    last_grounded_time: f32,
    can_jump: bool,
    grounded: bool,
) -> bool {
    now - buffer_time <= JUMP_BUFFER
        && can_jump
        && (grounded || now - last_grounded_time <= COYOTE_TIME)
}

fn facing_rotation(dir: Vec3) -> Quat {
    Quat::from_rotation_y(dir.x.atan2(dir.z))
}

fn facing_slerp_factor(current: Quat, target: Quat, rotation_speed: f32, dt: f32) -> f32 {
    let angle = current.angle_between(target);
    if angle <= f32::EPSILON {
        1.0
    } else {
        (rotation_speed * dt / angle).clamp(0.0, 1.0)
    }
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn player_movement(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    menus: Option<Res<bevy_ext_core::MenusOpen>>,
    typing: Option<Res<viber::ui::widgets::UiFocusedInput>>,
    actions: Option<Res<viber::input_map::ActionState>>,
    locomotion: Option<Res<viber::luau::ownership::ScriptSystemOwners>>,
    rt_waves: (
        Option<Res<viber::terrain::runtime::TerrainRuntime>>,
        Option<Res<viber::terrain::WaterWaveWorld>>,
        // Vento/amplitude das ondas: a cota da água que o herói sente é a
        // cota DESENHADA (gerstner), não o espelho plano.
        Option<Res<viber::worldsys::WeatherState>>,
        Option<Res<viber::terrain::water_material::WaterConfigResource>>,
    ),
    collision: Option<Res<viber::physics::TerrainCollisionStatus>>,
    interior: Option<Res<bevy_ext_core::InteriorSceneConfig>>,
    interior_state: Option<Res<bevy_ext_core::InteriorLighting>>,
    mut cameras: Query<&mut OrbitCamera>,
    bus: Option<Res<HeroMotionBus>>,
    mut players: Query<(
        Entity,
        &mut Transform,
        &mut VaelController,
        &mut Player,
        Option<&mut TnuaController<HeroScheme>>,
        Option<&mut LinearVelocity>,
        Option<&mut TnuaToggle>,
        Option<&mut Position>,
        Option<&mut Rotation>,
        Option<&mut TeleportSettle>,
        Option<&mut Knockback>,
        Option<&viber::spawn_gate::SpawnGated>,
        Option<&mut viber::terrain::WaterKinematics>,
        Option<&crate::boat::RidingBoat>,
    )>,
    mut sfx: MessageWriter<bevy_ext_core::SfxEvent>,
) {
    let dt = time.delta_secs().min(0.05);
    let now = time.elapsed_secs();
    let (runtime, waves) = (rt_waves.0.as_deref(), rt_waves.1.as_deref());
    let (weather, water_cfg) = (rt_waves.2.as_deref(), rt_waves.3.as_deref());
    // Env das ondas (mesma fonte do `sync_water_materials`).
    let (wind, wind_strength) = weather
        .map(|w| (w.wind, w.wind_strength))
        .or_else(|| water_cfg.map(|c| (c.0.wind, c.0.wind_strength)))
        .unwrap_or(([0.7, 0.25], 1.5));
    let wave_amp = water_cfg.map_or(1.0, |c| c.0.wave_amp);
    // Um script reclamou o corpo (`viber.own_system("locomotion")` +
    // `viber.set_pose`): o controller a pé não escreve por cima.
    if locomotion
        .as_ref()
        .is_some_and(|owners| owners.owns(viber::luau::ownership::SYSTEM_LOCOMOTION))
    {
        for (_, _, _, _, tnua, velocity, _, _, _, _, _, _, _, _) in &mut players {
            if let Some(mut tnua) = tnua {
                tnua.basis = bevy_tnua::builtins::TnuaBuiltinWalk::default();
            }
            if let Some(mut velocity) = velocity {
                velocity.0 = Vec3::ZERO;
            }
        }
        return;
    }
    // Modal aberto consome o teclado — e limpa a intenção pendente do
    // controller (o tnua reaplicava o último `desired_motion` e o herói
    // deslizava com o menu aberto).
    let blocked = menus.as_ref().is_some_and(|menus| menus.any());
    if blocked {
        for (
            _,
            _,
            mut controller,
            mut engine_player,
            tnua,
            velocity,
            _,
            _,
            _,
            _,
            _,
            _,
            mut kin,
            _,
        ) in &mut players
        {
            controller.vel_x = 0.0;
            controller.vel_z = 0.0;
            controller.vel_y = 0.0;
            controller.dodge = None;
            controller.lunge = None;
            if let Some(mut tnua) = tnua {
                tnua.basis = bevy_tnua::builtins::TnuaBuiltinWalk::default();
            }
            if let Some(mut velocity) = velocity {
                velocity.0 = Vec3::ZERO;
            }
            if let Some(ref mut kin) = kin {
                kin.velocity = Vec3::ZERO;
            }
            sync_engine_player(&mut engine_player, &controller);
        }
        return;
    }
    let typing = typing.as_ref().is_some_and(|focused| focused.0.is_some());
    let (move_x, move_forward, sprint, jump_pressed) = if typing {
        (0.0, 0.0, false, false)
    } else if let Some(actions) = actions.as_deref() {
        // Com ActionMap presente, os binds mandam (rebind de salto/corrida).
        (
            actions.move_x,
            actions.forward,
            actions.sprint,
            actions.jump_just,
        )
    } else {
        let (w, s, a, d) = (
            keys.pressed(KeyCode::KeyW) || keys.pressed(KeyCode::ArrowUp),
            keys.pressed(KeyCode::KeyS) || keys.pressed(KeyCode::ArrowDown),
            keys.pressed(KeyCode::KeyA) || keys.pressed(KeyCode::ArrowLeft),
            keys.pressed(KeyCode::KeyD) || keys.pressed(KeyCode::ArrowRight),
        );
        (
            (d as i32 - a as i32) as f32,
            (w as i32 - s as i32) as f32,
            keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight),
            keys.just_pressed(KeyCode::Space),
        )
    };

    let (dodge_request, attack_request) = bus
        .as_deref()
        .map(|bus| {
            let mut shared = bus.lock();
            (shared.dodge.take(), shared.attack.take())
        })
        .unwrap_or_default();

    for (
        entity,
        mut transform,
        mut controller,
        mut engine_player,
        mut tnua,
        mut velocity,
        mut toggle,
        mut position,
        mut rotation,
        mut settle,
        mut knockback,
        gated,
        mut water_kin,
        riding,
    ) in &mut players
    {
        // O tnua exige o reset da fila de ações em cada frame, ANTES de
        // alimentar (salto) e de escrever a basis.
        if let Some(tnua) = tnua.as_deref_mut() {
            tnua.initiate_action_feeding();
        }
        if gated.is_some() {
            controller.vel_x = 0.0;
            controller.vel_z = 0.0;
            controller.vel_y = 0.0;
            controller.dodge = None;
            controller.lunge = None;
            if let Some(tnua) = tnua.as_deref_mut() {
                tnua.basis = bevy_tnua::builtins::TnuaBuiltinWalk::default();
            }
            if let Some(velocity) = velocity.as_deref_mut() {
                velocity.0 = Vec3::ZERO;
            }
            if let Some(ref mut kin) = water_kin {
                kin.velocity = Vec3::ZERO;
            }
            sync_engine_player(&mut engine_player, &controller);
            continue;
        }
        // A bordo: quem conduz é o `boat.rs` (o leme e os remos escrevem no
        // casco; o herói é assento). O controller a pé não pode puxar o corpo
        // para o chão nem o solver empurrá-lo — o corpo é desligado no
        // embarque (`RigidBodyDisabled`) e volta ao sair.
        if riding.is_some() {
            controller.vel_x = 0.0;
            controller.vel_z = 0.0;
            controller.vel_y = 0.0;
            controller.dodge = None;
            controller.lunge = None;
            if let Some(tnua) = tnua.as_deref_mut() {
                tnua.basis = bevy_tnua::builtins::TnuaBuiltinWalk::default();
            }
            if let Some(velocity) = velocity.as_deref_mut() {
                velocity.0 = Vec3::ZERO;
            }
            if let Some(ref mut kin) = water_kin {
                kin.velocity = Vec3::ZERO;
            }
            sync_engine_player(&mut engine_player, &controller);
            continue;
        }
        // Tutela pós-teleporte: enquanto dura, o chão ANALÍTICO manda mesmo
        // que já exista collider algures — a coluna de destino pode estar a
        // assar e uma queda a 55 m/s atravessa o trimesh antes disso.
        //
        // Settle FRESCO é a CHEGADA de um teleporte (viagem pelo mapa,
        // `viber.teleport_player`, load do save): a chegada tem de ser limpa,
        // sem a inércia nem o knockback do sítio antigo. Quem teleporta repõe
        // o `Player` da engine — agora um espelho que este sync reescreve —
        // por isso o reset do MOVIMENTO vive aqui, no dono.
        let fresh_settle = settle.is_some() && !controller.settle_seen;
        controller.settle_seen = settle.is_some();
        if fresh_settle {
            controller.vel_x = 0.0;
            controller.vel_z = 0.0;
            controller.vel_y = 0.0;
            controller.is_jumping = false;
            controller.jump_cooldown = 0.0;
            controller.can_jump = true;
            controller.in_water = false;
            commands.entity(entity).remove::<Knockback>();
        }
        let settling = match settle.as_deref_mut() {
            Some(state) if state.frames > 0 => {
                state.frames -= 1;
                if state.frames == 0 {
                    commands.entity(entity).remove::<TeleportSettle>();
                }
                true
            }
            Some(_) => {
                commands.entity(entity).remove::<TeleportSettle>();
                false
            }
            None => false,
        };
        if controller.grounded {
            controller.last_grounded_time = now;
            controller.is_jumping = false;
        }
        if controller.jump_cooldown > 0.0 {
            controller.jump_cooldown = (controller.jump_cooldown - dt).max(0.0);
            if controller.jump_cooldown == 0.0 {
                controller.can_jump = true;
            }
        }

        // Numa bolsa de interior a câmara é FIXA: A/D andam nas 8 direções do
        // ecrã. Fora, A/D conduzem a câmara e o herói segue-a.
        let interior_mode = interior
            .as_deref()
            .is_some_and(|scene| scene.contains(transform.translation.x, transform.translation.z));
        let interior_floor = (interior_mode
            && interior_state.as_deref().is_some_and(|state| state.active))
        .then_some(bevy_ext_core::INTERIOR_FLOOR_Y);
        // Uma única query de domínio por frame: superfície, profundidade
        // (máscara de praia das ondas) e corrente saem daqui.
        let water_domain = (!interior_mode)
            .then(|| {
                runtime.and_then(|rt| {
                    rt.water_domain_at(transform.translation.x, transform.translation.z)
                })
            })
            .flatten();
        // Cota DESENHADA: o vértice da água recebe o Gerstner; nadar acompanha.
        let water_y = water_domain.map(|d| {
            d.surface_y
                + viber::terrain::surface_height_at(
                    transform.translation.xz(),
                    now,
                    wind,
                    wind_strength,
                    wave_amp,
                    d.depth,
                )
        });
        let submersion = water_y
            .map(|wy| (wy - transform.translation.y).max(0.0))
            .unwrap_or(0.0);
        let water_drag = water_submersion_drag(submersion);
        let water_flow = if !interior_mode && submersion > 0.18 {
            water_domain
                .map(|d| {
                    waves.map_or(d.flow, |w| {
                        w.effective_flow_at(transform.translation.xz(), d.flow)
                    })
                })
                .unwrap_or(Vec2::ZERO)
        } else {
            Vec2::ZERO
        };
        let (dir, input_mag, view_yaw_deg) = if interior_mode {
            let yaw = interior
                .as_deref()
                .map(|scene| scene.camera_yaw_deg)
                .unwrap_or(0.0);
            (
                process_input(move_forward, move_x, yaw),
                input_magnitude(move_forward, move_x),
                yaw,
            )
        } else {
            let mut camera_yaw_deg = 0.0f32;
            // Durante a esquiva o A/D é a DIREÇÃO do salto, não um giro: a
            // vista rodar a meio do salto lateral desfazia o movimento.
            let steer_input = if controller.dodge.is_some() {
                0.0
            } else {
                move_x
            };
            controller.steer += (steer_input - controller.steer) * low_pass_factor(dt, STEER_TAU);
            if steer_input == 0.0 && controller.steer.abs() < 1e-3 {
                controller.steer = 0.0;
            }
            if let Some(mut camera) = cameras.iter_mut().next() {
                let turn = controller.steer * CAMERA_TURN_SPEED.to_degrees() * dt;
                camera.yaw_deg -= turn;
                // O giro MANUAL entra também no yaw suavizado: o `turn_lag`
                // (0,38 s) é para a auto-câmara. Aplicado ao giro, a vista
                // ficava ~50° atrás do herói durante a curva e continuava a
                // rodar depois de largar a tecla — a curva "a mais".
                camera.smooth_yaw_deg -= turn;
                camera_yaw_deg = camera.yaw_deg;
            }
            if move_x != 0.0 {
                controller.last_steer_time = now;
            }
            // Com W/S, A/D só giram: o strafe somado punha o herói a correr
            // ~31° de lado e a auto-câmara perseguia esse heading depois de
            // largar a tecla (mais giro). Sem W/S, A/D continuam a contornar.
            let strafe = move_x * SIDE_MOVE_FACTOR * (1.0 - move_forward.abs().min(1.0));
            (
                process_input(move_forward, strafe, camera_yaw_deg),
                input_magnitude(move_forward, strafe),
                camera_yaw_deg,
            )
        };

        // ── pedidos do Lua: esquiva e movimento do golpe ──
        // O modelo da pipeline olha +Z.
        let facing = (transform.rotation * Vec3::Z)
            .with_y(0.0)
            .normalize_or(Vec3::Z);
        if let Some(kind) = dodge_request
            && controller.dodge.is_none()
        {
            let burst_dir = dodge_direction(
                kind,
                move_x,
                move_forward,
                view_yaw_deg,
                facing,
                interior_mode,
            );
            let (duration, distance, hop) = kind.profile();
            controller.dodge = Some((kind, Burst::new(burst_dir, distance, duration)));
            if hop > 0.0 && !interior_mode {
                controller.vel_y = jump_velocity(hop);
                controller.grounded = false;
                controller.is_jumping = true;
                controller.can_jump = false;
                controller.jump_cooldown = JUMP_COOLDOWN;
            }
            if !kind.keeps_facing() {
                transform.rotation = facing_rotation(burst_dir);
            }
            // A esquiva cancela o golpe: nada de passo travado a meio do
            // rolamento, nem avanço contra o sentido da fuga.
            controller.attack_until = f32::NEG_INFINITY;
            controller.attack_face = None;
            controller.lunge = None;
        }
        if let Some(attack) = attack_request
            && controller.dodge.is_none()
        {
            controller.attack_until = now + attack.secs;
            controller.attack_slow = attack.slow;
            let face = attack
                .face
                .map(|f| Vec3::new(f.x, 0.0, f.y))
                .unwrap_or(facing);
            controller.attack_face = Some(face);
            controller.lunge =
                (attack.lunge > 0.0).then(|| Burst::new(face, attack.lunge, LUNGE_SECS));
        }
        let attacking = now < controller.attack_until;
        let sprint_mult = if sprint {
            controller.sprint_multiplier
        } else {
            1.0
        };
        let attack_slow = if attacking {
            controller.attack_slow
        } else {
            1.0
        };
        let flow_drift = water_flow_drift(submersion, water_flow);
        let desired = dir * controller.speed * sprint_mult * input_mag * attack_slow * water_drag
            + Vec3::new(flow_drift.x, 0.0, flow_drift.y);
        let input_active = dir.length_squared() > 0.0;
        if let Some((_, burst)) = controller.dodge.as_mut() {
            // A esquiva manda na velocidade horizontal até assentar.
            let speed = burst.speed() * water_drag;
            let burst_dir = burst.dir;
            burst.t += dt;
            let done = burst.done();
            controller.vel_x = burst_dir.x * speed;
            controller.vel_z = burst_dir.z * speed;
            // Conta como "a conduzir": a auto-câmara não persegue o heading
            // lateral do salto.
            controller.last_steer_time = now;
            if done {
                controller.dodge = None;
            }
        } else {
            let (vel_x, vel_z) = integrate_horizontal(
                controller.vel_x,
                controller.vel_z,
                desired,
                controller.grounded || submersion > SWIM_DEPTH,
                input_active,
                dt,
            );
            controller.vel_x = vel_x;
            controller.vel_z = vel_z;
        }
        let (vel_x, vel_z) = (controller.vel_x, controller.vel_z);
        if vel_x * vel_x + vel_z * vel_z > 0.25 {
            controller.last_moving_time = now;
        }
        // Lunge e empurrão entram na VELOCIDADE pedida (m/s): o tnua acelera
        // até ela e o solver resolve paredes/props — escrito no Transform
        // atravessava tudo.
        let mut extra_motion = Vec3::ZERO;
        if let Some(lunge) = controller.lunge.as_mut() {
            extra_motion += lunge.dir * lunge.speed();
            lunge.t += dt;
            if lunge.done() {
                controller.lunge = None;
            }
        }
        if let Some(knockback) = knockback.as_deref_mut() {
            extra_motion += knockback.velocity.with_y(0.0);
            knockback.velocity *= knockback_decay_factor(dt);
            if knockback.velocity.length() < KNOCKBACK_EPSILON {
                knockback.velocity = Vec3::ZERO;
                commands.entity(entity).remove::<Knockback>();
            }
        }
        let desired_motion = Vec3::new(controller.vel_x, 0.0, controller.vel_z) + extra_motion;

        let dodge_keeps_facing = controller
            .dodge
            .is_some_and(|(kind, _)| kind.keeps_facing());
        let face_target = match controller.attack_face {
            Some(face) if attacking => Some((face, controller.rotation_speed * ATTACK_TURN_BOOST)),
            _ if dir.length_squared() > 0.0 => Some((dir, controller.rotation_speed)),
            _ => None,
        };
        if !dodge_keeps_facing && let Some((face, rate)) = face_target {
            let target = facing_rotation(face);
            let factor = facing_slerp_factor(transform.rotation, target, rate, dt);
            transform.rotation = transform.rotation.slerp(target, factor);
            // O writeback do Avian reescreve o `Transform` a partir da
            // `Rotation` do corpo: é aqui que o facing persiste.
            if let Some(rotation) = rotation.as_deref_mut() {
                rotation.0 = transform.rotation;
            }
        }

        if jump_pressed {
            controller.jump_buffer_time = now;
        }
        if !interior_mode
            && controller.dodge.is_none()
            && can_perform_jump(
                now,
                controller.jump_buffer_time,
                controller.last_grounded_time,
                controller.can_jump,
                controller.grounded,
            )
        {
            // Em terra o impulso é a AÇÃO do tnua (altura no asset); na água
            // funda (controller desligado) e sem corpo é o `vel_y` manual.
            if submersion >= SWIM_DEPTH || tnua.is_none() {
                controller.vel_y = jump_velocity(controller.jump_height);
            } else if let Some(tnua) = tnua.as_deref_mut() {
                tnua.action(HeroScheme::Jump(TnuaBuiltinJump::default()));
            }
            controller.is_jumping = true;
            controller.can_jump = false;
            controller.jump_cooldown = JUMP_COOLDOWN;
            controller.jump_buffer_time = f32::NEG_INFINITY;
            sfx.write(bevy_ext_core::SfxEvent {
                clip: bevy_ext_core::SfxClip::Jump,
                position: Some(transform.translation),
            });
        } else if !interior_mode
            && controller.dodge.is_none()
            && submersion >= SWIM_DEPTH
            && controller.jump_cooldown <= 0.0
            && now - controller.jump_buffer_time <= JUMP_BUFFER
        {
            // Impulso para subir à margem quando está a nadar em água funda.
            controller.vel_y = jump_velocity(controller.jump_height) * 0.85;
            controller.is_jumping = true;
            controller.can_jump = false;
            controller.jump_cooldown = JUMP_COOLDOWN;
            controller.jump_buffer_time = f32::NEG_INFINITY;
            sfx.write(bevy_ext_core::SfxEvent {
                clip: bevy_ext_core::SfxClip::FootstepWater,
                position: Some(transform.translation),
            });
        }

        // Chão analítico: a superfície SÓLIDA SOB o herói (respeita grutas e
        // overhangs). É a rede de segurança enquanto a coluna voxel assa ou
        // troca de LOD; com collider carregado (`terrain_ready`) o collider é
        // a autoridade.
        let (px, mut py, pz) = (
            transform.translation.x,
            transform.translation.y,
            transform.translation.z,
        );
        let terrain_ready = collision
            .as_deref()
            .is_some_and(|status| status.is_ready(entity));
        let field = runtime.filter(|rt| rt.in_field(px, pz));
        let top = field
            .map(|rt| rt.sample(px, pz))
            .unwrap_or(f32::NEG_INFINITY);
        if transform.translation.y < top - VOID_RESCUE_DEPTH {
            warn!(
                "vael-player: herói caiu do mundo a y={:.1} (superfície {:.1}) — resgatado",
                transform.translation.y, top
            );
            transform.translation.y = top;
            py = top;
            controller.vel_y = 0.0;
            controller.grounded = true;
        }
        let ground = if let Some(rt) = field {
            match rt.surface_below(px, pz, py + GROUND_PROBE) {
                Some(surface) => surface,
                None if settling => rt.sample(px, pz),
                None => f32::NEG_INFINITY,
            }
        } else {
            f32::NEG_INFINITY
        };

        let submersion = water_y.map(|wy| (wy - py).max(0.0)).unwrap_or(0.0);
        let swimming = submersion >= SWIM_DEPTH;
        let pre_entry_vel_y = controller.vel_y;
        let (next_vy, now_in_water, plunge_speed) = integrate_water_vertical(
            controller.vel_y,
            submersion,
            controller.in_water,
            controller.grounded,
            dt,
        );
        controller.in_water = now_in_water;
        if plunge_speed >= WATER_ENTRY_SPEED_MIN {
            sfx.write(bevy_ext_core::SfxEvent {
                clip: bevy_ext_core::SfxClip::FootstepWater,
                position: Some(Vec3::new(px, water_y.unwrap_or(py), pz)),
            });
        }
        if swimming {
            // Água funda: a integração vertical é nossa (empuxo/corrente).
            controller.vel_y = next_vy.max(-TERMINAL_VELOCITY);
        } else if let Some(velocity) = velocity.as_deref() {
            // Em terra a vertical é do tnua: `vel_y` é a leitura do corpo.
            controller.vel_y = velocity.0.y;
        }

        // O tnua fica DESLIGADO na água funda: aí conduzimos `LinearVelocity`
        // à mão (empuxo, corrente e o impulso para a margem).
        if let Some(toggle) = toggle.as_deref_mut() {
            *toggle = if swimming {
                TnuaToggle::Disabled
            } else {
                TnuaToggle::Enabled
            };
        }

        match tnua.as_deref_mut() {
            Some(tnua) => {
                if swimming {
                    if let Some(velocity) = velocity.as_deref_mut() {
                        velocity.0 = desired_motion + Vec3::Y * controller.vel_y;
                    }
                    controller.grounded = false;
                } else {
                    tnua.basis = bevy_tnua::builtins::TnuaBuiltinWalk {
                        desired_motion: desired_motion.with_y(0.0),
                        desired_forward: None,
                    };
                    controller.grounded = !tnua.is_airborne().unwrap_or(true);
                    if controller.grounded && controller.vel_y < 0.0 {
                        controller.vel_y = 0.0;
                    }
                }
                // Redes analíticas: soalho da bolsa de interior e coluna voxel
                // ainda por assar (a `Position` é a pose autoritativa).
                if let Some(floor) = interior_floor.filter(|floor| py < *floor) {
                    // A folga do `float_height` é OBRIGATÓRIA: com os pés
                    // exactamente na superfície o disco do sensor fica
                    // coplanar com o trimesh e o tnua deixa de ver o chão.
                    if let Some(position) = position.as_deref_mut() {
                        position.0.y = floor + viber::player::HERO_FLOAT_HEIGHT;
                    }
                    if let Some(velocity) = velocity.as_deref_mut() {
                        velocity.0.y = 0.0;
                    }
                    controller.vel_y = 0.0;
                    controller.grounded = true;
                } else if (!terrain_ready || settling) && py <= ground {
                    // Ver o comentário acima: o snap deixa a folga do sensor.
                    if let Some(position) = position.as_deref_mut() {
                        position.0.y = ground + viber::player::HERO_FLOAT_HEIGHT;
                    }
                    if let Some(velocity) = velocity.as_deref_mut() {
                        velocity.0.y = velocity.0.y.max(0.0);
                    }
                    controller.vel_y = 0.0;
                    controller.grounded = true;
                }
            }
            // Sem corpo (testes headless, ou herói antes do plugin): movimento
            // direto como antes, com a gravidade integrada aqui.
            None => {
                let mut motion = desired_motion * dt;
                motion.y = controller.vel_y * dt;
                transform.translation += motion;
                if transform.translation.y <= ground {
                    transform.translation.y = ground;
                    controller.vel_y = 0.0;
                    controller.grounded = true;
                } else {
                    controller.grounded = false;
                }
            }
        }

        let planar =
            (controller.vel_x * controller.vel_x + controller.vel_z * controller.vel_z).sqrt();
        let grounded = controller.grounded;
        if submersion >= SWIM_DEPTH && !grounded {
            if planar > 0.45 {
                engine_player.step_timer -= dt;
                if engine_player.step_timer <= 0.0 {
                    engine_player.step_timer = 0.58;
                    sfx.write(bevy_ext_core::SfxEvent {
                        clip: bevy_ext_core::SfxClip::FootstepWater,
                        position: Some(transform.translation),
                    });
                }
            }
        } else if footstep_due(&mut engine_player.step_timer, grounded, planar, dt).is_some() {
            let clip = if submersion > 0.04 {
                bevy_ext_core::SfxClip::FootstepWater
            } else {
                bevy_ext_core::SfxClip::Footstep
            };
            sfx.write(bevy_ext_core::SfxEvent {
                clip,
                position: Some(transform.translation),
            });
        }
        if let Some(mut kin) = water_kin {
            let impact_vy = if plunge_speed >= WATER_ENTRY_SPEED_MIN {
                pre_entry_vel_y
            } else {
                controller.vel_y
            };
            kin.velocity = Vec3::new(controller.vel_x, impact_vy, controller.vel_z);
            kin.body_radius = PLAYER_WATER_RADIUS;
        }
        sync_engine_player(&mut engine_player, &controller);
        if let Some(bus) = bus.as_deref() {
            let mut shared = bus.lock();
            shared.move_x = move_x;
            shared.move_forward = move_forward;
            shared.grounded = controller.grounded;
            shared.dodging = controller.dodge.is_some();
            shared.interior = interior_mode;
        }
    }
}

/// `<WorldBorder>`: mantém o herói dentro do disco do mapa (fora das
/// bolsas de interior). Corre depois do movimento, no mesmo frame.
fn world_border_clamp(
    border: Option<Res<bevy_ext_core::WorldBorderConfig>>,
    scene: Option<Res<bevy_ext_core::InteriorSceneConfig>>,
    mut players: Query<&mut Transform, With<Player>>,
    mut logged: Local<bool>,
) {
    let Some(border) = border else {
        return;
    };
    let limit = border.radius - border.margin;
    if limit <= 0.0 {
        return;
    }
    for mut transform in &mut players {
        let pos = transform.translation;
        if scene
            .as_deref()
            .is_some_and(|scene| scene.contains(pos.x, pos.z))
        {
            continue;
        }
        let dist_sq = pos.x * pos.x + pos.z * pos.z;
        if dist_sq > limit * limit {
            let scale = limit / dist_sq.sqrt();
            transform.translation.x = pos.x * scale;
            transform.translation.z = pos.z * scale;
            if !*logged {
                *logged = true;
                info!("world border: player returned inside r={limit}");
            }
        } else {
            *logged = false;
        }
    }
}

fn sync_engine_player(engine: &mut Player, controller: &VaelController) {
    engine.speed = controller.speed;
    engine.sprint_multiplier = controller.sprint_multiplier;
    engine.jump_height = controller.jump_height;
    engine.rotation_speed = controller.rotation_speed;
    engine.vel_x = controller.vel_x;
    engine.vel_z = controller.vel_z;
    engine.vel_y = controller.vel_y;
    engine.grounded = controller.grounded;
    engine.can_jump = controller.can_jump;
    engine.is_jumping = controller.is_jumping;
    engine.jump_cooldown = controller.jump_cooldown;
    engine.last_grounded_time = controller.last_grounded_time;
    engine.jump_buffer_time = controller.jump_buffer_time;
    engine.last_steer_time = controller.last_steer_time;
    engine.last_moving_time = controller.last_moving_time;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grounded_without_input_brakes_fast_then_stops() {
        let (x, z) = integrate_horizontal(3.0, -2.0, Vec3::ZERO, true, false, 0.016);
        assert!(
            x > 0.0 && x < 3.0 && z < 0.0 && z > -2.0,
            "trava, não corta"
        );
        let (mut x, mut z) = (4.0, 0.0);
        for _ in 0..30 {
            (x, z) = integrate_horizontal(x, z, Vec3::ZERO, true, false, 0.016);
        }
        assert_eq!((x, z), (0.0, 0.0), "em ~0,5 s está parado");
    }

    #[test]
    fn grounded_input_ramps_toward_desired() {
        let (x, _) = integrate_horizontal(0.0, 0.0, Vec3::X * 4.0, true, true, 0.016);
        assert!(x > 0.0 && x < 4.0);
    }

    #[test]
    fn airborne_keeps_momentum_without_input() {
        let (x, z) = integrate_horizontal(3.0, -2.0, Vec3::ZERO, false, false, 0.016);
        assert!(x > 0.0 && z < 0.0);
    }

    #[test]
    fn jump_needs_buffer_and_ground_or_coyote() {
        assert!(can_perform_jump(1.05, 1.0, 1.0, true, false));
        assert!(!can_perform_jump(1.5, 1.0, 0.0, true, false));
        assert!(!can_perform_jump(1.05, 1.0, 0.0, false, true));
        assert!(can_perform_jump(1.05, 1.0, 1.0, true, false));
    }

    #[test]
    fn burst_covers_exact_distance_and_eases_out() {
        let mut burst = Burst::new(Vec3::X, 4.0, 0.5);
        let first = burst.speed();
        let mut travelled = 0.0;
        let dt = 1.0 / 240.0;
        while !burst.done() {
            travelled += burst.speed() * dt;
            burst.t += dt;
        }
        assert!((travelled - 4.0).abs() < 0.05, "andou {travelled}");
        assert!(first > 4.0 / 0.5, "arranca acima da média");
        assert!(burst.speed() < 1e-3, "assenta no fim");
    }

    #[test]
    fn dodge_kind_follows_the_keys() {
        assert_eq!(dodge_kind(1.0, 0.0, false), DodgeKind::Side);
        assert_eq!(dodge_kind(-1.0, 1.0, false), DodgeKind::Side);
        assert_eq!(dodge_kind(0.0, 1.0, false), DodgeKind::Roll);
        assert_eq!(dodge_kind(0.0, -1.0, false), DodgeKind::Back);
        assert_eq!(dodge_kind(0.0, 0.0, false), DodgeKind::Back);
        assert_eq!(dodge_kind(1.0, 0.0, true), DodgeKind::Roll);
    }

    #[test]
    fn side_dodge_is_lateral_to_the_view_and_back_is_behind_the_hero() {
        // yaw 0 → a vista olha −Z; D (move_x = 1) é +X.
        let side = dodge_direction(DodgeKind::Side, 1.0, 0.0, 0.0, Vec3::NEG_Z, false);
        assert!((side - Vec3::X).length() < 1e-5, "{side:?}");
        let back = dodge_direction(DodgeKind::Back, 0.0, 0.0, 0.0, Vec3::NEG_Z, false);
        assert!((back - Vec3::Z).length() < 1e-5, "{back:?}");
    }

    #[test]
    fn dodge_request_needs_ground_and_no_dodge_in_flight() {
        let bus = HeroMotionBus::default();
        assert_eq!(bus.request_dodge(), None, "no ar não esquiva");
        {
            let mut shared = bus.lock();
            shared.grounded = true;
            shared.move_x = -1.0;
        }
        assert_eq!(bus.request_dodge(), Some(DodgeKind::Side));
        assert_eq!(bus.request_dodge(), None, "pedido pendente");
    }

    /// Os scripts que chamam `viber.vael.dodge`/`attack_motion` compilam no
    /// Luau do jogo (sem isto um erro de sintaxe só aparecia ao correr).
    #[test]
    fn dodge_and_melee_scripts_compile() {
        let lua = Lua::new();
        for (name, source) in [
            (
                "abilities.lua",
                include_str!("../scripts/game/abilities.lua"),
            ),
            ("combat.lua", include_str!("../scripts/game/combat.lua")),
        ] {
            if let Err(err) = lua.load(source).set_name(name).into_function() {
                panic!("{name}: {err}");
            }
        }
    }

    #[test]
    fn install_motion_exposes_dodge_to_lua() {
        let lua = Lua::new();
        lua.globals()
            .set("viber", lua.create_table().unwrap())
            .unwrap();
        let bus = HeroMotionBus::default();
        install_motion(&lua, bus.clone()).unwrap();
        let (kind, _): (Option<String>, Option<f32>) =
            lua.load("return viber.vael.dodge()").eval().unwrap();
        assert_eq!(kind, None, "no ar não esquiva");
        bus.lock().grounded = true;
        let (kind, duration): (Option<String>, Option<f32>) =
            lua.load("return viber.vael.dodge()").eval().unwrap();
        assert_eq!(kind.as_deref(), Some("back"));
        assert!(duration.is_some_and(|d| d > 0.0));
        lua.load("viber.vael.attack_motion({ secs = 0.3, lunge = 0.8, fx = 1, fz = 0 })")
            .exec()
            .unwrap();
        let attack = bus.lock().attack.expect("pedido de golpe");
        assert_eq!(attack.face, Some(Vec2::X));
        assert!((attack.lunge - 0.8).abs() < 1e-6);
    }

    #[test]
    fn input_yaw_zero_walks_negative_z() {
        let dir = process_input(1.0, 0.0, 0.0);
        assert!((dir - Vec3::NEG_Z).length() < 1e-5);
    }

    #[test]
    fn water_drag_and_buoyancy_damp_entry_and_float_at_equilibrium() {
        assert!((water_submersion_drag(0.05) - 1.0).abs() < 1e-6);
        let waist = water_submersion_drag(0.65);
        let chest = water_submersion_drag(1.4);
        assert!(waist < 1.0 && waist > chest);
        assert!((chest - 0.58).abs() < 1e-4);

        // Queda forte na água é amortecida no frame de entrada.
        let (vy_after, in_w, plunge) = integrate_water_vertical(-16.0, 0.45, false, false, 0.016);
        assert!(in_w);
        assert!((plunge - 16.0).abs() < 1e-4);
        assert!(vy_after.abs() < 7.0, "amortecido na entrada: {vy_after}");

        // Fundo mais baixo que SWIM_IDLE_DEPTH faz força de Arquimedes para cima.
        let mut vy = 0.0;
        let mut sub = 1.85;
        for _ in 0..90 {
            let (nv, _, _) = integrate_water_vertical(vy, sub, true, false, 0.016);
            vy = nv;
            sub -= vy * 0.016;
        }
        assert!(
            (sub - SWIM_IDLE_DEPTH).abs() < 0.08,
            "estabiliza na linha de nado: sub={sub}"
        );
    }

    #[test]
    fn water_flow_drift_respects_shallow_shore_friction_and_depth_regimes() {
        let river_flow = Vec2::new(2.0, -1.0);
        // Margem rasa: atrito com o fundo segura o passo (zero deriva).
        assert_eq!(water_flow_drift(0.10, river_flow), Vec2::ZERO);
        let shallow_ankle = water_submersion_drag(0.28);
        let knee_wade = water_submersion_drag(0.55);
        let deep_swim = water_submersion_drag(1.10);
        assert!(shallow_ankle > knee_wade && knee_wade > deep_swim);

        // Água média vs funda: correnteza cresce com a profundidade até o teto de nado.
        let mid_drift = water_flow_drift(0.50, river_flow);
        let deep_drift = water_flow_drift(1.10, river_flow);
        assert!(mid_drift.length() > 0.01);
        assert!(deep_drift.length() > mid_drift.length());
        assert!((deep_drift - river_flow * 0.24).length() < 1e-4);
    }
}

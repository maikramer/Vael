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

use avian3d::prelude::{CollisionLayers, Collider, LinearVelocity, Position, Rotation, Sensor};
use bevy::prelude::*;
use bevy_ext_core::{
    GROUND_PROBE, KNOCKBACK_EPSILON, Knockback, Player, PlayerBody, TeleportSettle, footstep_due,
    knockback_decay_factor,
};
use bevy_tnua::TnuaToggle;
use bevy_tnua::builtins::{
    TnuaBuiltinClimb, TnuaBuiltinClimbConfig, TnuaBuiltinCrouch, TnuaBuiltinCrouchConfig,
    TnuaBuiltinDash, TnuaBuiltinDashConfig, TnuaBuiltinJump, TnuaBuiltinJumpConfig,
    TnuaBuiltinKnockback, TnuaBuiltinKnockbackConfig, TnuaBuiltinWallSlide,
    TnuaBuiltinWallSlideConfig, TnuaBuiltinWalk, TnuaBuiltinWalkConfig, TnuaBuiltinWalkHeadroom,
};
use bevy_tnua::prelude::TnuaController;
use bevy_tnua::radar_lens::{TnuaBlipSpatialRelation, TnuaRadarLens};
use bevy_tnua::{TnuaAnimatingState, TnuaAnimatingStateDirective, TnuaObstacleRadar};
use bevy_tnua::builtins::{TnuaBuiltinCrouchMemory, TnuaBuiltinJumpMemory};
use bevy_tnua_physics_integration_layer::math::{AdjustPrecision, AsF32, Float, Vector3};
use bevy_tnua_physics_integration_layer::spatial_ext::{TnuaPointProjectionResult, TnuaSpatialExt};
use mlua::{Lua, Table, Value};
use viber::luau::components::ScriptProps;
use viber::recipes::spawn::OrbitCamera;

use crate::components::VaelPlayer;

/// O scheme do herói do VAEL (a engine é genérica: `HeroPhysicsPlugin<S>`).
///
/// A base é a `TnuaBuiltinWalk` — a intenção horizontal entra em
/// `desired_motion` e o `player_movement` deste módulo a escreve. As ações de
/// gameplay entram como variants aqui (dash, knockback, crouch, paredes…);
/// a engine standalone mantém o seu `HeroScheme` próprio.
#[derive(bevy_tnua::prelude::TnuaScheme)]
#[scheme(basis = TnuaBuiltinWalk)]
pub enum VaelHeroScheme {
    /// Salto (altura no asset).
    Jump(TnuaBuiltinJump),
    /// Esquiva (roll/side/back) e avanço curto do golpe: deslocação exata na
    /// direção dada — o deslocamento VERTICAL do `displacement` é o hop da
    /// esquiva lateral (sem efeito físico desde a migração Rapier→tnua).
    Dash(TnuaBuiltinDash),
    /// Empurrão (`viber.push_player`): impulso + boundary de Pushover que a
    /// basis não anula. Via `action_interrupt` (incancelável até passar).
    Knockback(TnuaBuiltinKnockback),
    /// Agachar (feed contínuo `action_start`/`action_end`): desce o corpo
    /// (`float_offset`) e só se levanta com headroom livre (sensor de teto).
    Crouch(TnuaBuiltinCrouch),
    /// Deslize numa parede (`TnuaObstacleRadar` dá o blip; o `Entity` é o
    /// payload de gameplay — o tnua ignora-o).
    WallSlide(TnuaBuiltinWallSlide, Entity),
    /// Salto contra a parede: partilha o gatilho do salto (`same_trigger`)
    /// e lança na normal via `horizontal_displacement`.
    #[scheme(same_trigger(Jump))]
    WallJump(TnuaBuiltinJump),
    /// Trepar um prop marcado ([`Climbable`]): W/S sobem/descem; sair do
    /// alcance (radar) solta. O terreno voxel NUNCA é trepável.
    Climb(TnuaBuiltinClimb),
}

/// Prop marcado como trepável no `.bsn` (`climbable="1"` nos props): alvo do
/// `Climb` do tnua. Insere-o [`attach_climbable`].
#[derive(Component)]
pub struct Climbable;

/// Anexa [`Climbable`] aos props marcados com `climbable="1"`.
fn attach_climbable(
    mut commands: Commands,
    props: Query<(Entity, &ScriptProps), (Added<ScriptProps>, Without<Climbable>)>,
) {
    for (entity, props) in &props {
        let climbable = props.0.iter().any(|(key, value)| {
            key.eq_ignore_ascii_case("climbable") && !value.is_empty() && value != "0"
        });
        if climbable {
            commands.entity(entity).insert(Climbable);
        }
    }
}

/// Config do tnua do herói do Vael — o mesmo tuning da engine
/// (`viber::player::hero_config`) enquanto as duas apps não divergirem.
/// O asset é criado por altura de salto pelo `HeroPhysicsPlugin`.
pub fn vael_hero_config(jump_height: f32) -> VaelHeroSchemeConfig {
    VaelHeroSchemeConfig {
        basis: TnuaBuiltinWalkConfig {
            speed: 1.0,
            // Medido a partir do disco elevado (HERO_SENSOR_LIFT na engine).
            float_height: viber::player::HERO_FLOAT_HEIGHT
                + viber::player::HERO_SENSOR_LIFT,
            cling_distance: viber::player::HERO_CLING_DISTANCE,
            spring_strength: 400.0,
            spring_dampening: 1.2,
            acceleration: 60.0,
            air_acceleration: 20.0,
            // Coyote do CRATE: o walk basis conta-o num timer interno quando
            // o chão se perde (`is_airborne` só é true depois de expirar).
            // O mesmo valor do antigo `COYOTE_TIME` do jogo.
            coyote_time: 0.1,
            free_fall_extra_gravity: 0.0,
            max_slope: viber::player::HERO_MAX_SLOPE_DEG.to_radians(),
            // Headroom (sensor de teto que o walk basis cria sozinho): o
            // crouch precisa de saber se pode levantar. A distância é o topo
            // da cápsula de pé (origem = pegada).
            headroom: Some(TnuaBuiltinWalkHeadroom {
                distance_to_collider_top: viber::player::HERO_CAPSULE_CENTER
                    + viber::player::HERO_HALF_HEIGHT
                    + viber::player::HERO_RADIUS,
                sensor_extra_distance: 0.1,
            }),
            ..Default::default()
        },
        jump: TnuaBuiltinJumpConfig {
            height: jump_height.max(0.1),
            // Buffer do CRATE: a ação alimentada fica à espera de chão até
            // esta janela fechar. O jogo alimenta durante `JUMP_BUFFER`.
            input_buffer_time: 0.1,
            ..Default::default()
        },
        // Esquiva/avanço: uma velocidade serve os três perfis de esquiva —
        // 4.2/8.4 ≈ 0.50 s (roll), 3.4/8.4 ≈ 0.40 s (side), 2.6/8.4 ≈ 0.31 s
        // (back), os mesmos do antigo `Burst` — e o `brake_to_speed` baixo
        // faz o assentar final que o ease-out do Burst dava.
        dash: TnuaBuiltinDashConfig {
            speed: 8.4,
            brake_to_speed: 2.0,
            input_buffer_time: 0.0,
            ..Default::default()
        },
        knockback: TnuaBuiltinKnockbackConfig::default(),
        // Agachar: desce o corpo o mesmo que a cápsula encolhe
        // ([`HERO_CROUCH_HEIGHT`]) — os pés continuam à mesma cota.
        crouch: TnuaBuiltinCrouchConfig {
            float_offset: -(viber::player::HERO_CAPSULE_CENTER
                + viber::player::HERO_HALF_HEIGHT
                + viber::player::HERO_RADIUS
                - HERO_CROUCH_HEIGHT),
            ..Default::default()
        },
        // Deslize: cola à parede a cair devagar, sem derrapar lateral.
        wall_slide: TnuaBuiltinWallSlideConfig {
            max_fall_speed: 2.0,
            max_sideways_speed: 0.0,
            ..Default::default()
        },
        // Salto de parede: mesma altura do salto, lançamento na normal.
        wall_jump: TnuaBuiltinJumpConfig {
            height: jump_height.max(0.1),
            horizontal_distance: 2.0,
            ..Default::default()
        },
        // Trepar: 3 m/s sobe/desce (o default do crate é 10 — rápido demais
        // para escada).
        climb: TnuaBuiltinClimbConfig {
            climb_speed: 3.0,
            ..Default::default()
        },
    }
}

pub const GRAVITY: f32 = 60.0;
pub const TERMINAL_VELOCITY: f32 = 55.0;
pub const CAMERA_TURN_SPEED: f32 = 2.5;
pub const SIDE_MOVE_FACTOR: f32 = 0.6;
/// Altura total da cápsula AGACHADA (m): `hero_crouch_collider` troca a
/// cápsula por esta altura enquanto o `Crouch` está em curso — sem isto a
/// cápsula de pé penetra o chão quando o `float_offset` desce o corpo.
pub const HERO_CROUCH_HEIGHT: f32 = 1.15;
/// Multiplicador do passo enquanto agachado.
pub const CROUCH_SLOW: f32 = 0.45;
/// Paredes (deslize + salto de parede) ativas por omissão;
/// `VAEL_WALL_MOVES=0` desliga (A/B contra o comportamento antigo).
static WALL_MOVES: std::sync::LazyLock<bool> =
    std::sync::LazyLock::new(|| std::env::var("VAEL_WALL_MOVES").map(|v| v != "0").unwrap_or(true));
/// Janela (s) do buffer de salto: o pedido alimenta a ação do tnua durante
/// este tempo e o crate dispara quando o sensor de chão o permitir (na hora
/// ou logo à frente da aterragem — `input_buffer_time` do asset).
pub const JUMP_BUFFER: f32 = 0.1;
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
    /// Timestamp do último pedido de salto: abre a janela de feed do `Jump`
    /// (`JUMP_BUFFER`). O chão/coyote em si são do tnua.
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
        // Estados de animação pela ação do tnua → override para o driver da
        // engine (o blending grounded continua do driver).
        commands.entity(entity).insert((
            TnuaAnimatingState::<VaelAnimState>::default(),
            bevy_ext_core::animation::AnimStateOverride::default(),
        ));
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

/// Radar de obstáculos para o deslize/salto de parede (só o herói; o
/// adapter Avian do tnua marca os blips cada passo).
fn attach_hero_radar(
    mut commands: Commands,
    heroes: Query<Entity, (With<Player>, Without<TnuaObstacleRadar>)>,
) {
    for entity in &heroes {
        commands
            .entity(entity)
            .insert(TnuaObstacleRadar::new(0.5, 1.2));
    }
}

/// Parâmetros da Fase paredes/trepar (`player_movement` já ia cheio — o
/// Bevy implementa sistemas até 16 parâmetros; estes três vivem agrupados).
#[derive(bevy::ecs::system::SystemParam)]
pub struct WallProbe<'w, 's> {
    radars: Query<'w, 's, &'static TnuaObstacleRadar>,
    climbables: Query<'w, 's, &'static Climbable>,
    spatial_ext: VaelSpatialExt<'w, 's>,
    /// DIAGNÓSTICO da brecha de queda (o sistema já lotou os 16 params).
    pub breach_log: Local<'s, bevy::time::Stopwatch>,
}

/// O `TnuaSpatialExtAvian3d` do crate lê `Position`/`Rotation` de TODOS os
/// colliders — o herói entre eles. Com o `player_movement` a escrever
/// `&mut Position`/`&mut Rotation` do herói no MESMO sistema, o B0001
/// rebentava a inicialização da query e o frame inteiro morria (medido
/// 2026-11-15). Esta cópia exclui o herói do conjunto de leitura: o radar
/// de paredes nunca precisou do collider dele — os blips são de OUTROS corpos.
#[derive(bevy::ecs::system::SystemParam)]
pub struct VaelSpatialExt<'w, 's> {
    colliders_query:
        Query<'w, 's, (&'static Collider, &'static Position, &'static Rotation), Without<Player>>,
    collision_configuration_query: Query<'w, 's, (Option<&'static CollisionLayers>, Has<Sensor>)>,
}

impl TnuaSpatialExt for VaelSpatialExt<'_, '_> {
    type ColliderData<'a>
        = (&'a Collider, &'a Position, &'a Rotation)
    where
        Self: 'a;

    fn fetch_collider_data(&self, entity: Entity) -> Option<Self::ColliderData<'_>> {
        self.colliders_query.get(entity).ok()
    }

    fn project_point<'a>(
        &'a self,
        point: Vector3,
        solid: bool,
        collider_data: &Self::ColliderData<'a>,
    ) -> TnuaPointProjectionResult {
        let (collider, position, rotation) = collider_data;
        let (projected_point, is_inside) =
            collider.project_point(position.0, **rotation, point, solid);
        if is_inside {
            TnuaPointProjectionResult::Inside(projected_point)
        } else {
            TnuaPointProjectionResult::Outside(projected_point)
        }
    }

    fn cast_ray<'a>(
        &'a self,
        origin: Vector3,
        direction: Vector3,
        max_time_of_impact: Float,
        collider_data: &Self::ColliderData<'a>,
    ) -> Option<(Float, Vector3)> {
        let (collider, position, rotation) = collider_data;
        collider
            .cast_ray(
                position.0,
                **rotation,
                origin.adjust_precision(),
                direction.f32(),
                max_time_of_impact.f32(),
                true,
            )
            .map(|(time_of_impact, normal)| {
                (time_of_impact.adjust_precision(), normal.adjust_precision())
            })
    }

    fn can_interact(&self, entity1: Entity, entity2: Entity) -> bool {
        let Ok([(layers1, is_1_sensor), (layers2, is_2_sensor)]) = self
            .collision_configuration_query
            .get_many([entity1, entity2])
        else {
            return false;
        };
        if is_1_sensor || is_2_sensor {
            return false;
        }
        layers1
            .copied()
            .unwrap_or_default()
            .interacts_with(layers2.copied().unwrap_or_default())
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
            .add_systems(PostStartup, (attach_controllers, attach_hero_radar))
            .add_systems(
                viber::scene_loader::SceneLoad,
                (attach_controllers, attach_hero_radar, attach_climbable)
                    .after(viber::recipes::spawn::startup),
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
            )
            .add_systems(
                FixedUpdate,
                (
                    // O SFX do salto reage ao arranque REAL da ação (o feed
                    // pode ficar no buffer e nunca disparar — tocar no feed
                    // daria um "whoosh" fantasma a cada press perdida).
                    hero_jump_sfx,
                    // A cápsula acompanha o crouch (segue o discriminante,
                    // não a tecla — o levantar é da ação).
                    hero_crouch_collider,
                    // Estados de animação pela ação do tnua.
                    hero_anim_state,
                )
                    .after(bevy_tnua::prelude::TnuaPipelineSystems::Logic),
            );
    }
}

/// Arranque: `viber.vael.dodge()` e `viber.vael.attack_motion{…}`.
fn install_motion_api(bus: Res<HeroMotionBus>) {
    let bus = bus.clone();
    crate::script_host::register_installer(move |lua| install_motion(lua, bus.clone()));
    info!("vael: esquiva/golpe em Rust registados nos contextos BMS (`viber.vael`)");
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

/// SFX do salto: dispara no arranque REAL da ação do tnua (`ActionStarted`
/// do `action_flow_status`, lido depois da lógica do crate no FixedUpdate).
#[allow(clippy::type_complexity)]
fn hero_jump_sfx(
    mut sfx: MessageWriter<bevy_ext_core::SfxEvent>,
    heroes: Query<(&Position, &TnuaController<VaelHeroScheme>)>,
) {
    for (position, tnua) in &heroes {
        if tnua.action_flow_status().just_starting()
            == Some(VaelHeroSchemeActionDiscriminant::Jump)
        {
            sfx.write(bevy_ext_core::SfxEvent {
                clip: bevy_ext_core::SfxClip::Jump,
                position: Some(position.0),
            });
        }
    }
}

/// Marcador: a cápsula AGACHADA está aplicada.
#[derive(Component)]
struct CrouchedCollider;

/// Cápsula agachada ([`HERO_CROUCH_HEIGHT`]), origem na pegada como a de pé.
fn hero_crouch_collider_shape() -> avian3d::prelude::Collider {
    use avian3d::prelude::Collider;
    Collider::compound(vec![(
        Vec3::Y * (HERO_CROUCH_HEIGHT / 2.0),
        Quat::IDENTITY,
        Collider::capsule(
            viber::player::HERO_RADIUS,
            HERO_CROUCH_HEIGHT - 2.0 * viber::player::HERO_RADIUS,
        ),
    )])
}

/// Troca a cápsula do herói enquanto o `Crouch` do tnua está em curso.///
/// A cápsula de pé (1.8 m) penetra o chão quando o `float_offset` do crouch
/// desce o corpo — o solver lutava contra a spring e o herói não descia. A
/// agachada mantém os pés à mesma cota. O LEVANTAR só acontece quando o
/// headroom liberta (a ação passa a `Rising` e só acaba limpa), por isso a
/// troca segue o DISCRIMINANTE da ação, não a tecla.
#[allow(clippy::type_complexity)]
fn hero_crouch_collider(
    mut commands: Commands,
    heroes: Query<
        (
            Entity,
            &TnuaController<VaelHeroScheme>,
            Option<&CrouchedCollider>,
        ),
        With<Player>,
    >,
) {
    for (entity, tnua, crouched) in &heroes {
        let crouching = matches!(
            tnua.action_discriminant(),
            Some(VaelHeroSchemeActionDiscriminant::Crouch)
        );
        match (crouching, crouched.is_some()) {
            (true, false) => {
                commands
                    .entity(entity)
                    .insert((hero_crouch_collider_shape(), CrouchedCollider));
            }
            (false, true) => {
                commands
                    .entity(entity)
                    .remove::<CrouchedCollider>()
                    .insert(viber::player::hero_collider());
            }
            _ => {}
        }
    }
}

/// Estado de animação do herói visto do TNUA. `EngineDecides` devolve a
/// decisão ao driver (idle/walk/run/água pelo deslocamento real); os restantes
/// vêm da AÇÃO corrente — coisa que o movimento medido não revela.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum VaelAnimState {
    #[default]
    EngineDecides,
    Jumping,
    Falling,
    Dashing,
    Knocked,
    Crouching,
    OnWall,
    Climbing,
}

/// Mapeamento para os clips que o herói tem hoje (sem clip próprio para
/// dash/crouch/parede: Run/Walk/Fall servem de fallback; clips novos no GLB
/// do herói entram aqui).
fn map_anim_state(state: VaelAnimState) -> Option<bevy_ext_core::animation::AnimState> {
    use bevy_ext_core::animation::AnimState;
    match state {
        VaelAnimState::EngineDecides => None,
        VaelAnimState::Jumping => Some(AnimState::Jump),
        VaelAnimState::Falling | VaelAnimState::Knocked | VaelAnimState::OnWall => {
            Some(AnimState::Fall)
        }
        VaelAnimState::Dashing => Some(AnimState::Run),
        VaelAnimState::Crouching | VaelAnimState::Climbing => Some(AnimState::Walk),
    }
}

/// Estados de animação pela AÇÃO corrente do tnua: o
/// [`TnuaAnimatingState`] dá a transição (Maintain/Alter) e o driver da
/// engine recebe o resultado via `AnimStateOverride`.
#[allow(clippy::type_complexity)]
fn hero_anim_state(
    mut heroes: Query<(
        &TnuaController<VaelHeroScheme>,
        &mut TnuaAnimatingState<VaelAnimState>,
        &mut bevy_ext_core::animation::AnimStateOverride,
    )>,
) {
    for (tnua, mut animating, mut override_component) in &mut heroes {
        let current = match tnua.current_action.as_ref() {
            Some(VaelHeroSchemeActionState::Jump(state)) => match &state.memory {
                TnuaBuiltinJumpMemory::NoJump => VaelAnimState::EngineDecides,
                TnuaBuiltinJumpMemory::FallSection => VaelAnimState::Falling,
                _ => VaelAnimState::Jumping,
            },
            Some(VaelHeroSchemeActionState::Dash(_)) => VaelAnimState::Dashing,
            Some(VaelHeroSchemeActionState::Knockback(_)) => VaelAnimState::Knocked,
            Some(VaelHeroSchemeActionState::Crouch(state)) => match state.memory {
                TnuaBuiltinCrouchMemory::Rising => VaelAnimState::EngineDecides,
                _ => VaelAnimState::Crouching,
            },
            Some(VaelHeroSchemeActionState::WallSlide(..))
            | Some(VaelHeroSchemeActionState::WallJump(_)) => VaelAnimState::OnWall,
            Some(VaelHeroSchemeActionState::Climb(_)) => VaelAnimState::Climbing,
            None => VaelAnimState::EngineDecides,
        };
        let directive = animating.update_by_discriminant(current);
        let forced = match directive {
            TnuaAnimatingStateDirective::Maintain { state } => map_anim_state(*state),
            TnuaAnimatingStateDirective::Alter { state, .. } => map_anim_state(*state),
        };
        override_component.0 = forced;
    }
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
    mut walls: WallProbe,
    mut players: Query<(
        Entity,
        &mut Transform,
        &mut VaelController,
        &mut Player,
        Option<&mut TnuaController<VaelHeroScheme>>,
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
            entity,
            _,
            mut controller,
            mut engine_player,
            tnua,
            velocity,
            _,
            _,
            _,
            mut settle,
            _,
            _,
            mut kin,
            _,
        ) in &mut players
        {
            // O settle TEM de correr com o menu aberto: o loader de cena
            // espera `!settling` para fechar a fase Settle do load, e o
            // título da cena de menu abre exatamente nesse momento — sem
            // isto o véu ficava preso para sempre (medido 2026-11-15:
            // `settling=1` durante 57 s com o menu aberto).
            if let Some(state) = settle.as_deref_mut()
                && state.frames > 0
                && state.tick(time.delta_secs())
            {
                commands.entity(entity).remove::<TeleportSettle>();
            }
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
    // Agachar é HELD (ação contínua do tnua).
    let crouch_held = if typing {
        false
    } else if let Some(actions) = actions.as_deref() {
        actions.crouch_held
    } else {
        keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight)
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
            // O settle TEM de correr gated: a chegada da cena já o inseriu e
            // a prontidão do mundo espera `!settling` — sem o tick, o véu
            // não abria e o portão (que espera a época fechar) nunca soltava
            // o herói (deadlock medido 2026-11-15: `settling=1 gated=1`).
            if let Some(state) = settle.as_deref_mut()
                && state.frames > 0
                && state.tick(time.delta_secs())
            {
                commands.entity(entity).remove::<TeleportSettle>();
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
            controller.jump_buffer_time = f32::NEG_INFINITY;
            controller.in_water = false;
            commands.entity(entity).remove::<Knockback>();
            // O Knockback do tnua é incancelável pelo feed; uma CHEGADA de
            // teleporte limpa a ação corrente à mão (nada de aterrar ainda a
            // ser empurrado do sítio antigo).
            if let Some(tnua) = tnua.as_deref_mut() {
                tnua.current_action = None;
            }
        }
        let settling = match settle.as_deref_mut() {
        Some(state) if state.frames > 0 => {
                if state.tick(time.delta_secs()) {
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
        // Com corpo e fora de água funda os movimentos especiais são AÇÕES
        // do tnua (Dash/Knockback); sem corpo (testes headless) é o caminho
        // manual de sempre.
        let with_body = tnua.is_some() && submersion < SWIM_DEPTH;
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
            // O Burst fica como REGISTO: a curva ease-out alimenta o espelho
            // (animação/câmara) durante o dash do tnua, e move de facto no
            // caminho sem corpo.
            controller.dodge = Some((kind, Burst::new(burst_dir, distance, duration)));
            if !kind.keeps_facing() {
                transform.rotation = facing_rotation(burst_dir);
            }
            // A esquiva cancela o golpe: nada de passo travado a meio do
            // rolamento, nem avanço contra o sentido da fuga.
            controller.attack_until = f32::NEG_INFINITY;
            controller.attack_face = None;
            controller.lunge = None;
            if with_body && let Some(tnua) = tnua.as_deref_mut() {
                // AÇÃO Dash: distância exata do perfil; o hop lateral entra
                // como componente VERTICAL do `displacement` — efeito físico
                // que se perdeu na migração (era só espelho).
                let hop_y = (hop > 0.0 && !interior_mode).then_some(hop).unwrap_or(0.0);
                tnua.action(VaelHeroScheme::Dash(TnuaBuiltinDash {
                    displacement: burst_dir * distance + Vec3::Y * hop_y,
                    desired_forward: None,
                    allow_in_air: false,
                }));
                if hop > 0.0 {
                    controller.grounded = false;
                }
            } else if hop > 0.0 && !interior_mode {
                // Sem corpo: integração manual como antes.
                controller.vel_y = jump_velocity(hop);
                controller.grounded = false;
            }
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
            if attack.lunge > 0.0 {
                if with_body && let Some(tnua) = tnua.as_deref_mut() {
                    // Avanço do golpe como Dash (mesma ação da esquiva; o
                    // Lua nunca pede os dois no mesmo frame).
                    tnua.action(VaelHeroScheme::Dash(TnuaBuiltinDash {
                        displacement: face * attack.lunge,
                        desired_forward: None,
                        allow_in_air: false,
                    }));
                } else {
                    controller.lunge =
                        (attack.lunge > 0.0).then(|| Burst::new(face, attack.lunge, LUNGE_SECS));
                }
            }
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
        let crouch_slow = if crouch_held && with_body { CROUCH_SLOW } else { 1.0 };
        let desired = dir
            * controller.speed
            * sprint_mult
            * input_mag
            * attack_slow
            * crouch_slow
            * water_drag
            + Vec3::new(flow_drift.x, 0.0, flow_drift.y);
        let input_active = dir.length_squared() > 0.0;
        // Dash em curso (a ação corrente do tnua — um frame atrás do pedido,
        // que é coberto pelo ramo do `dodge`): o espelho segue a curva do
        // Burst para a animação/câmara e quem manda no corpo é a ação.
        let dash_ongoing = tnua.as_ref().is_some_and(|t| {
            matches!(
                t.action_discriminant(),
                Some(VaelHeroSchemeActionDiscriminant::Dash)
            )
        });
        if dash_ongoing && let Some((_, burst)) = controller.dodge.as_mut() {
            // Esquiva via Dash: a velocidade aqui é SÓ espelho (animação/
            // câmara) — a `desired_motion` é zerada abaixo.
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
        } else if tnua.is_none() && let Some((_, burst)) = controller.dodge.as_mut() {
            // Sem corpo: o Burst move de facto (integra no Transform).
            let speed = burst.speed() * water_drag;
            let burst_dir = burst.dir;
            burst.t += dt;
            let done = burst.done();
            controller.vel_x = burst_dir.x * speed;
            controller.vel_z = burst_dir.z * speed;
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
        // Lunge/empurrão SEM corpo entram na VELOCIDADE pedida (m/s); com
        // corpo o lunge já é um Dash e o empurrão é a ação Knockback abaixo.
        let mut extra_motion = Vec3::ZERO;
        if let Some(lunge) = controller.lunge.as_mut() {
            extra_motion += lunge.dir * lunge.speed();
            lunge.t += dt;
            if lunge.done() {
                controller.lunge = None;
            }
        }
        if let Some(knockback) = knockback.as_deref_mut() {
            if with_body && let Some(tnua) = tnua.as_deref_mut() {
                // AÇÃO Knockback: impulso + boundary de Pushover que a basis
                // não anula nem deixa cancelar. O componente é a API externa
                // (Lua `viber.push_player`); consome-o aqui.
                let shove = knockback.velocity.with_y(0.0);
                tnua.action_interrupt(VaelHeroScheme::Knockback(TnuaBuiltinKnockback {
                    shove,
                    force_forward: None,
                }));
                knockback.velocity = Vec3::ZERO;
                commands.entity(entity).remove::<Knockback>();
            } else {
                extra_motion += knockback.velocity.with_y(0.0);
                knockback.velocity *= knockback_decay_factor(dt);
                if knockback.velocity.length() < KNOCKBACK_EPSILON {
                    knockback.velocity = Vec3::ZERO;
                    commands.entity(entity).remove::<Knockback>();
                }
            }
        }
        let desired_motion = if dash_ongoing {
            // A ação Dash formula o motor; a basis fica quiça (sem isto a
            // aceleração do passeio lutava contra o brake do dash).
            Vec3::ZERO
        } else {
            Vec3::new(vel_x, 0.0, vel_z) + extra_motion
        };

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

        // ── salto ──
        // O chão, o coyote e o buffer de aterragem são do CRATE
        // (`coyote_time`/`input_buffer_time` no asset). Aqui só se abre a
        // janela de feed: alimentamos o `Jump` enquanto ela durar e o tnua
        // dispara quando o sensor de chão o permitir. O SFX não é daqui —
        // `hero_jump_sfx` reage ao arranque real da ação.
        if jump_pressed {
            controller.jump_buffer_time = now;
        }
        let jump_buffered = now - controller.jump_buffer_time <= JUMP_BUFFER;
        if !interior_mode && controller.dodge.is_none() && jump_buffered {
            if submersion >= SWIM_DEPTH {
                // Água funda (controller desligado): impulso manual para
                // subir à margem.
                controller.vel_y = jump_velocity(controller.jump_height) * 0.85;
                controller.jump_buffer_time = f32::NEG_INFINITY;
                sfx.write(bevy_ext_core::SfxEvent {
                    clip: bevy_ext_core::SfxClip::FootstepWater,
                    position: Some(transform.translation),
                });
            } else if let Some(tnua) = tnua.as_deref_mut() {
                // Alimentar SEM consumir: a janela continua aberta e o tnua
                // pode segurar o pedido até aterrar. Em deslize de parede o
                // mesmo gatilho dispara o WallJump (lança contra a normal).
                if let Some(VaelHeroSchemeActionState::WallSlide(state, _)) =
                    &tnua.current_action
                {
                    let normal = *state.input.normal;
                    tnua.action(VaelHeroScheme::WallJump(TnuaBuiltinJump {
                        horizontal_displacement: Some(normal),
                        allow_in_air: true,
                        force_forward: None,
                    }));
                } else {
                    tnua.action(VaelHeroScheme::Jump(TnuaBuiltinJump::default()));
                }
            } else {
                // Sem corpo (testes headless): integração manual como antes.
                controller.vel_y = jump_velocity(controller.jump_height);
                controller.jump_buffer_time = f32::NEG_INFINITY;
            }
        }

        // Agachar: feed CONTÍNUO (a ação não é one-shot — o `action_trigger`
        // não serve). O levantar é da ação (`Rising` só com headroom livre);
        // em água funda o toggle do tnua está desligado e não alimenta.
        if with_body
            && !interior_mode
            && let Some(tnua) = tnua.as_deref_mut()
        {
            if crouch_held {
                tnua.action_start(VaelHeroScheme::Crouch(TnuaBuiltinCrouch));
            } else {
                tnua.action_end(VaelHeroSchemeActionDiscriminant::Crouch);
            }
        }

        // Paredes (VAEL_WALL_MOVES): em queda encostado a uma face — radar
        // dá o blip "ao lado" na direção do input — alimenta o deslize. Sem
        // input o deslize corrente mantém-se (a comparação do exemplo do
        // crate: novo exige dot>0.5, corrente basta -0.5<dot).
        if *WALL_MOVES
            && with_body
            && !controller.grounded
            && let Some(radar) = walls.radars.get(entity).ok()
            && let Some(tnua) = tnua.as_deref_mut()
        {
            let lens = TnuaRadarLens::new(radar, &walls.spatial_ext);
            let mut currently_sliding_on = None;
            if let Some(VaelHeroSchemeActionState::WallSlide(_, wall)) = &tnua.current_action {
                currently_sliding_on = Some(*wall);
            }
            for blip in lens.iter_blips() {
                if let TnuaBlipSpatialRelation::Aeside(blip_direction) =
                    blip.spatial_relation(0.5)
                {
                    let should_slide = if currently_sliding_on == Some(blip.entity()) {
                        -0.5 < blip_direction.dot(dir)
                    } else {
                        0.5 < blip_direction.dot(dir)
                    };
                    if should_slide
                        && let Ok(normal) = Dir3::new(blip.normal_from_closest_point())
                    {
                        tnua.action(VaelHeroScheme::WallSlide(
                            TnuaBuiltinWallSlide {
                                contact_point_with_wall: blip.closest_point().get(),
                                normal,
                                force_forward: None,
                            },
                            blip.entity(),
                        ));
                        break;
                    }
                }
            }
        }

        // Trepar (Fase climb): blip de um prop `Climbable` à frente alimenta
        // o Climb — W/S sobem/descem (`desired_climb_motion`); sair do raio
        // do radar solta (coyote do crate). Ações em curso (salto, dash,
        // knockback) têm prioridade. O terreno voxel nunca é climbable.
        if with_body
            && let Some(radar) = walls.radars.get(entity).ok()
            && let Some(tnua) = tnua.as_deref_mut()
            && !matches!(
                tnua.action_discriminant(),
                Some(
                    VaelHeroSchemeActionDiscriminant::Jump
                        | VaelHeroSchemeActionDiscriminant::Dash
                        | VaelHeroSchemeActionDiscriminant::Knockback
                )
            )
        {
            let lens = TnuaRadarLens::new(radar, &walls.spatial_ext);
            for blip in lens.iter_blips() {
                if walls.climbables.get(blip.entity()).is_err() {
                    continue;
                }
                if let TnuaBlipSpatialRelation::Aeside(blip_direction) =
                    blip.spatial_relation(0.5)
                    && blip_direction.dot(dir) < 0.3
                {
                    continue;
                }
                let anchor = blip.closest_point().get();
                let to_anchor = anchor - transform.translation;
                if to_anchor.length_squared() > 1e-6 {
                    tnua.action(VaelHeroScheme::Climb(TnuaBuiltinClimb {
                        anchor,
                        desired_vec_to_anchor: to_anchor,
                        desired_climb_motion: Vec3::Y * move_forward,
                        desired_forward: None,
                        hard_stop_up: None,
                        hard_stop_down: None,
                    }));
                    break;
                }
            }
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
        // Chão analítico ANTES do resgate: o warn da queda reporta o estado
        // real (e o snap de segurança abaixo usa o mesmo valor).
        //
        // `surface_below` devolve None quando o herói está DENTRO da rocha do
        // heightfield (enterrado — o corpo afundou numa coluna cujo collider
        // ainda não assou) ou abaixo dela. Nesses estados a resposta honesta
        // de "não há superfície abaixo" cegava a rede de segurança até ao
        // resgate do vazio (medido 2026-11-15: brecha a 5 m da superfície com
        // ground=-inf e queda livre até −250 m). O chão real nesses casos é o
        // TOPO do heightfield — o snap de segurança devolve-lhe o topo.
        // (Dentro de gruta com chão próprio a resposta é Some — este ramo não
        // é atingido.)
        let ground = if let Some(rt) = field {
            rt.surface_below(px, pz, py + GROUND_PROBE)
                .unwrap_or_else(|| rt.sample(px, pz))
        } else {
            f32::NEG_INFINITY
        };
        if transform.translation.y < top - VOID_RESCUE_DEPTH {
            warn!(
                "vael-player: herói caiu do mundo a y={:.1} (superfície {:.1}) — resgatado \
                 (terrain_ready={} settling={} ground={:.1})",
                transform.translation.y,
                top,
                terrain_ready,
                settling,
                ground
            );
            transform.translation.y = top;
            py = top;
            controller.vel_y = 0.0;
            controller.grounded = true;
            // O corpo é do Avian: a `Position` é a autoridade e o writeback
            // desfazia o `Transform` no frame seguinte — o "resgate" era
            // cosmético e o herói continuava a queda debaixo do mundo.
            // Teleporta o CORPO, zera a velocidade e rearma a tutela
            // analítica (a mesma chegada limpa de um teleporte).
            if let Some(mut position) = position.as_deref_mut() {
                position.0.y = top;
            }
            if let Some(mut velocity) = velocity.as_deref_mut() {
                velocity.0.y = 0.0;
            }
            if settle.is_none() {
                commands.entity(entity).insert(TeleportSettle::default());
            }
        }
        // DIAGNÓSTICO da queda no arranque: o começo da brecha — o herói
        // despenca perto da superfície com a rede analítica supostamente
        // activa. 1/s no máximo.
        if !controller.grounded && controller.vel_y < -8.0 && py > top - 40.0 && py <= top {
            walls.breach_log.tick(time.delta());
            if walls.breach_log.elapsed().as_secs_f32() >= 1.0 {
                walls.breach_log.reset();
                warn!(
                    "vael-player: brecha perto da superfície: py={:.2} ground={:.2} \
                     terrain_ready={} settling={} vel_y={:.1}",
                    py,
                    ground,
                    terrain_ready,
                    settling,
                    controller.vel_y
                );
            }
        }

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
                } else if (!terrain_ready || settling)
                    && py <= ground + viber::player::HERO_FLOAT_HEIGHT
                    && controller.vel_y <= 0.0
                {
                    // Ver o comentário acima: o snap deixa a folga do sensor.
                    // A banda inclui o float (e só apanha a DESCER): com a
                    // coluna ainda sem collider o herói fica preso no float
                    // em vez de oscilar entre o snap e a gravidade.
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
            shared.dodging = dash_ongoing || controller.dodge.is_some();
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
    // `can_jump`/`is_jumping`/`jump_cooldown`/`last_grounded_time` da engine
    // deixaram de ser espelhados: o chão/coyote/buffer do salto são do tnua
    // e ninguém no jogo lia estas pontas.
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
    fn jump_feed_window_matches_the_old_buffer() {
        // A janela de feed do tnua tem a mesma largura do antigo
        // `JUMP_BUFFER`: aberta até 0,1 s do pedido, fechada depois. O
        // chão/coyote em si são decididos pelo crate (testados lá).
        let now = 1.05;
        let pressed_at = 1.0;
        assert!(now - pressed_at <= JUMP_BUFFER, "dentro da janela alimenta");
        assert!(now - f32::NEG_INFINITY > JUMP_BUFFER, "sem pedido: fechada");
        let stale = now - JUMP_BUFFER - 1e-3;
        assert!(now - stale > JUMP_BUFFER, "janela expirada não alimenta");
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

//! Corpos físicos dos personagens (`bevy_ext_puppet`, estilo Euphoria).
//!
//! O herói e as criaturas com rig bípede ganham, **a pedido**, um corpo de
//! 16 peças com músculos XPBD que seguem a animação, equilíbrio por *capture
//! point* IPM + controlo por forças virtuais implícitas (`VirtualForce`),
//! momento angular centroidal, IK de pernas sem twist e queda balística
//! (`CatchFall`). O corpo monta-se quando é preciso (golpe, empurrão, morte,
//! queda) a partir da pose do momento e desmonta-se sozinho uns segundos depois
//! de a animação voltar a mandar — criaturas paradas não custam nada ao solver.
//!
//! Ligações (nenhuma exige alterar o rig ou a cena):
//!
//! * **Arquétipos físicos**: massa, força muscular, limiar de reação parcial e
//!   passos adaptam-se ao arquétipo (`LuaScriptRef`/`Name`: leve, padrão,
//!   escudo, bruto, chefe) e à escala da entidade.
//! * `viber.hit_flash` + `viber.entity_knockback` (golpe do herói / bomba /
//!   Acorde): golpes ligeiros acertam o ombro/peito consoante o ângulo (torção
//!   axial em `HitReaction`); empurrões fortes (`force >= 4.5`) entram como
//!   `puppet_push` para acionar passos de captura IPM + VMC implícito ou queda
//!   com `CatchFall`.
//! * `viber.corpse` (a morte): o corpo morre mole na direção do último golpe
//!   (substitui o ragdoll passivo do `RagdollDeath` quando o rig é bípede).
//! * `viber.push_player` (o herói leva um golpe): reação parcial direcional no
//!   herói, sem lhe tirar o controlo.
//! * [`PuppetEvent`]: quedas geram poeira no impacto + micro shake em criaturas
//!   pesadas; `GettingUp { face_up }` dispara gesto `getup_back`/`getup_front`
//!   quando o `AnimationSet` o define.
//! * `viber.vael.puppet_*`: API Lua completa (`puppet_busy`, `puppet_state`,
//!   `puppet_info`, `puppet_push`, `puppet_hit`, `puppet_stagger`, `puppet_kill`,
//!   `puppet_revive`, `puppet_getup`, `puppet_release`, `puppet_activate`,
//!   `puppet_fall`, `puppet_start`).
//!
//! Camadas: o corpo do herói (tnua) está só na camada [`HERO_LAYER_BIT`]; os
//! corpos dos bonecos (`PUPPET_LAYER_BIT`) não colidem com ele nem entre si
//! — um ragdoll nunca empurra nem prende o herói, e o boneco do herói não luta
//! com a cápsula.
//!
//! Gates: `VAEL_PUPPET=0` desliga tudo; `VAEL_PUPPET_DEBUG=1` desenha os
//! corpos, o centro de massa, o capture point, a região de captura *flywheel*,
//! o momento angular centroidal e o polígono de apoio.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use bevy_ext_luau::commands::{ScriptCommand, ScriptCommandMessage, entity_from_id};
use bevy_ext_puppet::lifecycle::PuppetLifecycleExt;
use bevy_ext_puppet::prelude::*;
use bevy_ext_puppet::rig::find_part_bones;
use mlua::Lua;

use crate::components::VaelPlayer;
use crate::vault::{vael_fn, vael_table};

/// Camada do corpo (tnua) do herói.
pub use vael_engine::player::HERO_LAYER_BIT;

/// Só se montam corpos até esta distância do herói (m).
const MAX_DISTANCE: f32 = 80.0;
/// Segundos de animação tranquila até desmontar o corpo.
const IDLE_DISMANTLE_SECS: f32 = 3.0;
/// Knockback abaixo deste limiar aplica `puppet_hit` localizado (flinch/torção).
const PUSH_KNOCKBACK_THRESHOLD: f32 = 4.5;
/// Knockback de golpe ligeiro (`force < 4.5`) → impulso (N·s).
const HIT_KNOCKBACK_TO_IMPULSE: f32 = 5.5;
/// Knockback de empurrão forte (`force >= 4.5`, bomba/Acorde/investida) → impulso (N·s) em `puppet_push`.
const PUSH_KNOCKBACK_TO_IMPULSE: f32 = 6.5;
/// Impulso de um golpe sem knockback (bosses, golpes bloqueados).
const FLASH_IMPULSE: f32 = 14.0;
/// Impulso base do golpe no herói (sempre parcial: o herói não perde o controlo,
/// mas o tronco e a cabeça reagem claramente à direção 360° do golpe).
const HERO_HIT_IMPULSE: f32 = 54.0;
/// Um golpe conta para a direção da morte durante estes segundos.
const LAST_HIT_WINDOW: f32 = 0.8;
/// Distância máxima para queda pesada tremer a câmara (m).
const FALL_SHAKE_RANGE: f32 = 12.0;
/// Velocidade relativa mínima (m/s) de esbarrão entre personagens (cobre
/// caminhada tranquila em multidões estilo Assassin's Creed).
const BUMP_MIN_SPEED: f32 = 0.7;
/// Cooldown (s) por personagem entre reações a esbarrões consecutivos.
const BUMP_COOLDOWN_SECS: f32 = 0.55;
/// Ganho de impulso (N·s por m/s efetivo) em esbarrões entre personagens.
const BUMP_IMPULSE_GAIN: f32 = 11.0;
/// Impulso mínimo (N·s) para um esbarrão de sprint/dash virar `puppet_push`
/// (passos de abertura IPM + VMC para o lado).
const BUMP_PUSH_IMPULSE_MIN: f32 = 24.0;
/// Velocidade relativa mínima (m/s) para um esbarrão virar `puppet_push` em NPCs.
/// Caminhada e corrida normal (~4 m/s) ficam no jogo de ombro parcial; sprint
/// (~6 m/s) e dash (~8.4 m/s) abrem caminho com passos físicos reais.
const BUMP_CROWD_PUSH_SPEED: f32 = 5.0;
/// Velocidade relativa mínima (m/s) para um esbarrão virar `puppet_push` em hostis.
const BUMP_HOSTILE_PUSH_SPEED: f32 = 5.5;
/// Teto de impulso de esbarrão para NPCs/criaturas (N·s).
const BUMP_MAX_IMPULSE: f32 = 54.0;
/// Teto de impulso parcial de esbarrão no herói (N·s).
const BUMP_HERO_MAX_IMPULSE: f32 = 20.0;
/// Impulso mínimo efetivo (N·s) para disparar reação de esbarrão.
const BUMP_MIN_IMPULSE: f32 = 6.5;
/// Massa de referência do herói para cálculo de troca de momento em esbarrões (kg).
const HERO_REF_MASS: f32 = 75.0;

pub fn enabled() -> bool {
    std::env::var("VAEL_PUPPET").map(|v| v.trim() != "0").unwrap_or(true)
}

/// O rig não tem os ossos de um bípede (lobo, verme…): nunca tentar outra vez.
#[derive(Component, Debug, Clone, Copy)]
pub struct PuppetIncompatible;

/// Último golpe recebido (direção da queda quando morre).
#[derive(Component, Debug, Clone, Copy)]
pub struct PuppetLastHit {
    pub impulse: Vec3,
    pub part: PuppetPart,
    pub at: f32,
}

/// Arquétipo físico de uma criatura bípede.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreatureArchetype {
    Light,
    Standard,
    Shielded,
    Brute,
    Boss,
}

/// Fotografia de propriocepção publicada ao Lua em `viber.vael.puppet_info(id)`.
#[derive(Debug, Clone, Copy)]
pub struct PuppetSnapshot {
    pub mode: PuppetMode,
    pub stance: StanceState,
    pub feet_down: u8,
    pub trunk_tilt: f32,
    pub capture_margin: f32,
    pub capture_radius: f32,
    pub speed: f32,
    pub angular_momentum: f32,
    pub plant_quality: f32,
    pub pelvis_height: f32,
    pub face_up: bool,
    pub trauma: f32,
    pub fatigue: f32,
    pub hit_combo: u32,
    pub ground_clearance: f32,
    pub wall_nearby: bool,
    pub peak_impact_speed: f32,
}

/// Pedidos vindos do Lua (`viber.vael.puppet_*`).
#[derive(Debug, Clone)]
enum Request {
    Hit { entity: Entity, part: PuppetPart, impulse: Vec3, point: Option<Vec3>, kind: HitKind },
    Bump { entity: Entity, part: PuppetPart, impulse: Vec3, bumper_pos: Option<Vec3> },
    Trip { entity: Entity, side: Side, impulse: Vec3 },
    LookAt { entity: Entity, point: Vec3, duration: f32 },
    Push { entity: Entity, impulse: Vec3 },
    Stagger { entity: Entity, direction: Vec3, duration: f32 },
    Kill(Entity),
    Revive(Entity),
    GetUp(Entity),
    Release(Entity),
    Activate(Entity),
    Fall(Entity),
    Start { entity: Entity, behaviour: String },
}

impl Request {
    fn target_entity(&self) -> Entity {
        match *self {
            Request::Hit { entity, .. }
            | Request::Bump { entity, .. }
            | Request::Trip { entity, .. }
            | Request::LookAt { entity, .. }
            | Request::Push { entity, .. }
            | Request::Stagger { entity, .. }
            | Request::Kill(entity)
            | Request::Revive(entity)
            | Request::GetUp(entity)
            | Request::Release(entity)
            | Request::Activate(entity)
            | Request::Fall(entity)
            | Request::Start { entity, .. } => entity,
        }
    }
}

#[derive(Default)]
pub struct PuppetCtl {
    requests: Vec<Request>,
    /// Estado + propriocepção de cada corpo montado (bits da entidade), publicado por frame.
    snapshots: HashMap<u64, PuppetSnapshot>,
}

#[derive(Resource, Clone, Default)]
pub struct VaelPuppetControl(pub Arc<Mutex<PuppetCtl>>);

impl VaelPuppetControl {
    fn lock(&self) -> std::sync::MutexGuard<'_, PuppetCtl> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Descarta pedidos pendentes para `entity` e limpa o snapshot em cache.
    pub fn clear_entity(&self, entity: Entity) {
        let mut ctl = self.lock();
        ctl.requests.retain(|req| req.target_entity() != entity);
        ctl.snapshots.remove(&entity.to_bits());
    }

    /// Enfileira a morte física (`puppet_kill`) a partir do Rust (ex.: DoT).
    pub fn request_kill(&self, entity: Entity) {
        self.lock().requests.push(Request::Kill(entity));
    }
}

/// Desmonta imediatamente qualquer corpo físico (`bevy_ext_puppet`) montado no
/// herói, devolvendo os ossos do esqueleto (e a arma presa a `hand_r`) à
/// animação antes de um teleporte, respawn ou carregamento de save.
pub fn release_player_puppets(world: &mut World) {
    let heroes: Vec<Entity> = world
        .query_filtered::<Entity, Or<(With<VaelPlayer>, With<bevy_ext_core::Player>)>>()
        .iter(world)
        .collect();
    if let Some(control) = world.get_resource::<VaelPuppetControl>().cloned() {
        for &hero in &heroes {
            control.clear_entity(hero);
        }
    }
    for hero in heroes {
        bevy_ext_puppet::lifecycle::dismantle(world, hero);
    }
}

pub struct VaelPuppetPlugin;

impl Plugin for VaelPuppetPlugin {
    fn build(&self, app: &mut App) {
        if !enabled() {
            info!("puppet: VAEL_PUPPET=0 — corpos físicos desligados");
            return;
        }
        app.add_plugins(PuppetPlugin::default());
        if std::env::var("VAEL_PUPPET_DEBUG").is_ok_and(|v| v.trim() != "0") {
            app.add_plugins(PuppetDebugPlugin);
        }
        app.add_message::<crate::actor_contact::ActorBump>()
            .init_resource::<VaelPuppetControl>()
            .add_systems(Startup, install_puppet_api)
            .add_systems(
                Update,
                (
                    // Antes do ragdoll passivo: um cadáver bípede fica com o
                    // corpo do puppet (o `RagdollDeath` sai).
                    die_physically.before(vael_engine::ragdoll::start_ragdolls),
                    react_to_script_commands,
                    react_to_actor_bumps,
                    apply_requests,
                    reconcile_hero_puppet,
                    calm_kinematic_motion,
                    handle_puppet_events,
                    publish_snapshots,
                )
                    .chain(),
            )
            // A pose alvo é a pose FINAL da animação: depois do IK dos pés,
            // dos offsets de ossos e do ajuste das palmas das mãos.
            .configure_sets(
                PostUpdate,
                PuppetSystems::Capture
                    .after(vael_engine::ik::bone_offsets_system)
                    .after(crate::animation::relax_inward_palms),
            );
    }
}

/// Classifica o arquétipo físico pelo script ou nome da entidade.
pub fn archetype_from_hint(script_path: Option<&str>, name: Option<&str>) -> CreatureArchetype {
    let mut hay = String::new();
    if let Some(s) = script_path {
        hay.push_str(&s.to_ascii_lowercase());
        hay.push(' ');
    }
    if let Some(n) = name {
        hay.push_str(&n.to_ascii_lowercase());
    }
    if hay.contains("boss") || hay.contains("bog-warden") || hay.contains("bog_warden") || hay.contains("witch") {
        CreatureArchetype::Boss
    } else if hay.contains("brute") || hay.contains("ogre") || hay.contains("giant") {
        CreatureArchetype::Brute
    } else if hay.contains("shield") || hay.contains("watch-guard") || hay.contains("guard") {
        CreatureArchetype::Shielded
    } else if hay.contains("goblin") || hay.contains("bogling") || hay.contains("shade") {
        CreatureArchetype::Light
    } else {
        CreatureArchetype::Standard
    }
}

/// Afinação física de uma criatura a partir do arquétipo e da escala do transform.
pub fn creature_config_for(archetype: CreatureArchetype, scale: f32) -> PuppetConfig {
    let scale_clamped = if scale.is_finite() && scale > 0.1 { scale.clamp(0.55, 2.2) } else { 1.0 };
    // Massa escala ~s^2.2 (entre área e volume) para que brutos 1.25× pesem mais
    // sem exigir rigidez infinita do solver XPBD.
    let mass_mult = scale_clamped.powf(2.2);
    let (base_mass, strength, partial, max_steps, getup_delay, reaction_delay, collapse_time) = match archetype {
        CreatureArchetype::Light => (52.0, 0.95, 22.0, 4, 1.0, 0.032, 0.24),
        CreatureArchetype::Standard => (70.0, 1.0, 28.0, 4, 1.2, 0.042, 0.30),
        CreatureArchetype::Shielded => (78.0, 1.15, 36.0, 4, 1.2, 0.042, 0.32),
        CreatureArchetype::Brute => (115.0, 1.25, 48.0, 3, 1.5, 0.055, 0.38),
        CreatureArchetype::Boss => (150.0, 1.35, 68.0, 3, 1.4, 0.058, 0.42),
    };
    PuppetConfig {
        total_mass: base_mass * mass_mult,
        strength,
        collision_filters: u32::MAX & !HERO_LAYER_BIT,
        partial_hit_impulse: partial * mass_mult.sqrt(),
        max_steps,
        getup_delay,
        reaction_delay,
        dying_collapse_time: collapse_time,
        ..default()
    }
}

/// Afinação do herói: só reações parciais (nunca perde o controlo por um
/// golpe), morre mole, não se levanta sozinho (o respawn do jogo trata), e
/// mantém o braço da espada livre de clutch de ferida para ataques responsivos.
fn hero_config() -> PuppetConfig {
    PuppetConfig {
        collision_filters: u32::MAX & !HERO_LAYER_BIT,
        partial_hit_impulse: f32::INFINITY,
        getup_delay: -1.0,
        move_root_on_getup: false,
        reach_for_wound: false,
        ..default()
    }
}

/// Escolhe a parte anatómica atingida pela direção de um golpe horizontal.
///
/// Golpes laterais acertam o ombro/braço desse lado para acionar a torção axial
/// da coluna em [`HitReaction`]; golpes frontais/traseiros atingem o peito.
pub fn pick_hit_part(hit_dir: Vec3, target_right: Vec3) -> PuppetPart {
    let dir = Vec3::new(hit_dir.x, 0.0, hit_dir.z).normalize_or_zero();
    let right = Vec3::new(target_right.x, 0.0, target_right.z).normalize_or_zero();
    if dir == Vec3::ZERO || right == Vec3::ZERO {
        return PuppetPart::SpineUpper;
    }
    // `dir` é a direção PARA ONDE o golpe empurra: se empurra para +right, veio
    // do lado esquerdo do alvo (ombro esquerdo).
    let lat = dir.dot(right);
    if lat > 0.35 {
        PuppetPart::UpperArmL
    } else if lat < -0.35 {
        PuppetPart::UpperArmR
    } else {
        PuppetPart::SpineUpper
    }
}

/// Direção de abertura estilo multidão de Assassin's Creed: quando alguém se
/// desloca (`pusher_vel`) contra um NPC de multidão, empurra-o diagonalmente
/// para o lado (`0.40 fwd + 0.92 side`) para abrir um corredor em vez de o
/// empurrar em linha reta na frente do corredor.
pub fn crowd_part_dir(normal_to_target: Vec3, pusher_vel: Vec2) -> Vec3 {
    let n = Vec3::new(normal_to_target.x, 0.0, normal_to_target.z).normalize_or_zero();
    if n == Vec3::ZERO || pusher_vel.length_squared() < 0.25 {
        return n;
    }
    let fwd = Vec3::new(pusher_vel.x, 0.0, pusher_vel.y).normalize();
    let right = Vec3::new(-fwd.z, 0.0, fwd.x);
    let side_sign = if n.dot(right) < -0.05 { -1.0 } else { 1.0 };
    (fwd * 0.40 + right * (side_sign * 0.92)).normalize_or_zero()
}

/// Calcula o impulso (N·s) e se o esbarrão entre dois personagens deve virar
/// `puppet_push` (passos de abertura / queda) ou `puppet_hit` localizado (jogo de ombro).
///
/// * Caminhada suave (`0.7..3.4 m/s`): jogo de ombro localizado (gentle push).
/// * Corrida / sprint (`3.4..6.2 m/s`) em NPC de multidão: `puppet_push` diagonal
///   lateral → o NPC dá 1–2 passos físicos para o lado abrindo caminho.
/// * Dash / investida (`>= 6.2 m/s`): `puppet_push` forte → cambaleio / `CatchFall`.
/// * Herói: sempre `puppet_hit` parcial no ombro, nunca perde controlo.
pub fn compute_bump_impulse(
    closing_speed: f32,
    other_mass: f32,
    self_mass: f32,
    is_hero: bool,
    target_is_crowd_npc: bool,
) -> Option<(f32, bool)> {
    if !closing_speed.is_finite() || closing_speed < BUMP_MIN_SPEED {
        return None;
    }
    let denom = (other_mass + self_mass).max(1.0);
    let ratio = (other_mass / denom).clamp(0.05, 0.95);
    let raw = closing_speed * ratio * BUMP_IMPULSE_GAIN;
    if raw < BUMP_MIN_IMPULSE {
        return None;
    }
    if is_hero {
        Some((raw.min(BUMP_HERO_MAX_IMPULSE), false))
    } else {
        let speed_gate = if target_is_crowd_npc {
            BUMP_CROWD_PUSH_SPEED
        } else {
            BUMP_HOSTILE_PUSH_SPEED
        };
        let is_push = closing_speed >= speed_gate && raw >= BUMP_PUSH_IMPULSE_MIN;
        Some((raw.min(BUMP_MAX_IMPULSE), is_push))
    }
}

/// Queries para montar um corpo a pedido.
#[derive(bevy::ecs::system::SystemParam)]
pub struct PuppetBuilder<'w, 's> {
    children: Query<'w, 's, &'static Children>,
    names: Query<'w, 's, &'static Name>,
    parents: Query<'w, 's, &'static ChildOf>,
    state: Query<
        'w,
        's,
        (
            Has<Puppet>,
            Has<PuppetIncompatible>,
            Has<VaelPlayer>,
            &'static GlobalTransform,
            Option<&'static vael_engine::luau::LuaScriptRef>,
            Option<&'static Name>,
        ),
    >,
    hero: Query<'w, 's, &'static GlobalTransform, With<VaelPlayer>>,
}

impl PuppetBuilder<'_, '_> {
    /// Garante o corpo de `entity` (monta-o se o rig for bípede e estiver
    /// perto do herói). `false` quando não há nem haverá corpo.
    fn ensure(&self, commands: &mut Commands, entity: Entity) -> bool {
        let Ok((has, incompatible, is_hero, gt, script, name)) = self.state.get(entity) else {
            return false;
        };
        if has {
            return true;
        }
        if incompatible {
            return false;
        }
        if !is_hero
            && let Ok(hero) = self.hero.single()
            && hero.translation().distance(gt.translation()) > MAX_DISTANCE
        {
            return false;
        }
        match find_part_bones(entity, &self.children, &self.names, &self.parents) {
            Ok(_) => {
                let config = if is_hero {
                    hero_config()
                } else {
                    let arch = archetype_from_hint(
                        script.map(|s| s.path.as_str()),
                        name.map(Name::as_str),
                    );
                    let scale = gt.to_scale_rotation_translation().0.y;
                    creature_config_for(arch, scale)
                };
                commands.entity(entity).try_insert((
                    Puppet,
                    config,
                    PuppetIdleDismantle::after(IDLE_DISMANTLE_SECS),
                ));
                true
            }
            Err(missing) => {
                // Um GLB ainda a carregar não tem ossos nenhuns: só desistir
                // quando há esqueleto mas não é bípede.
                if missing.len() < 16 {
                    debug!("puppet: {entity} sem rig bípede (faltam {missing:?})");
                    commands.entity(entity).try_insert(PuppetIncompatible);
                }
                false
            }
        }
    }
}

/// `viber.corpse`: o corpo bípede morre mole, tombando com o último golpe.
fn die_physically(
    mut commands: Commands,
    time: Res<Time>,
    builder: PuppetBuilder,
    corpses: Query<
        (Entity, Option<&PuppetLastHit>, &GlobalTransform),
        (Added<vael_engine::corpse::Corpse>, Without<VaelPlayer>),
    >,
) {
    let now = time.elapsed_secs();
    for (entity, last, gt) in &corpses {
        if !builder.ensure(&mut commands, entity) {
            continue;
        }
        // O puppet substitui o ragdoll passivo.
        commands.entity(entity).remove::<vael_engine::ragdoll::RagdollDeath>();
        let (part, impulse) = match last {
            Some(h) if now - h.at <= LAST_HIT_WINDOW => (h.part, h.impulse * 1.5),
            // Sem golpe recente (veneno, queda): cai para trás.
            _ => (PuppetPart::SpineUpper, gt.back() * 25.0),
        };
        commands
            .entity(entity)
            .puppet_hit(part, impulse, None)
            .puppet_kill();
    }
}

/// Golpes do herói nas criaturas e golpes nas costas do herói.
#[allow(clippy::too_many_arguments)]
fn react_to_script_commands(
    mut commands: Commands,
    time: Res<Time>,
    mut messages: MessageReader<ScriptCommandMessage>,
    builder: PuppetBuilder,
    transforms: Query<&GlobalTransform>,
    corpses: Query<(), With<vael_engine::corpse::Corpse>>,
    hero: Query<(Entity, &GlobalTransform), With<VaelPlayer>>,
) {
    // Um golpe chega como `hit_flash` + `entity_knockback` no mesmo frame.
    // Guardamos `(dir, force)` para distinguir flinch leve (`puppet_hit`) de
    // empurrão com passo/desequilíbrio (`puppet_push`).
    let mut hits: HashMap<Entity, Option<(Vec3, f32)>> = HashMap::new();
    let mut hero_push: Option<Vec3> = None;
    for ScriptCommandMessage(cmd) in messages.read() {
        match cmd {
            ScriptCommand::HitFlash { entity, .. } => {
                hits.entry(*entity).or_insert(None);
            }
            ScriptCommand::EntityKnockback { entity, dir, force } => {
                let flat = Vec3::new(dir.x, 0.0, dir.z).normalize_or_zero();
                if flat != Vec3::ZERO {
                    hits.insert(*entity, Some((flat, *force)));
                }
            }
            ScriptCommand::PushPlayer { dir, .. } => {
                let flat = Vec3::new(dir.x, 0.0, dir.z);
                if flat.length_squared() > 1e-6 {
                    hero_push = Some(flat);
                }
            }
            _ => {}
        }
    }
    let now = time.elapsed_secs();
    let hero_pos = hero.single().ok().map(|(_, g)| g.translation());
    for (entity, kb) in hits {
        if corpses.contains(entity) {
            continue;
        }
        let target_gt = transforms.get(entity).ok();
        let target_right = target_gt.map(|g| g.right().as_vec3()).unwrap_or(Vec3::X);
        let (dir, force) = match kb {
            Some((d, f)) => (d, f),
            None => {
                // Golpe sem knockback: empurra para longe do herói.
                let pos = target_gt.map(|g| g.translation()).unwrap_or_default();
                let away = hero_pos.map(|h| (pos - h).with_y(0.0).normalize_or_zero()).unwrap_or(Vec3::ZERO);
                (away, 0.0)
            }
        };
        if dir == Vec3::ZERO || !builder.ensure(&mut commands, entity) {
            continue;
        }
        let part = pick_hit_part(dir, target_right);
        if force >= PUSH_KNOCKBACK_THRESHOLD {
            // Empurrão forte (bomba, Acorde, investida, remate pesado): aciona
            // diretamente o equilíbrio IPM + VMC implícito + passos de captura.
            let impulse = dir * (force * PUSH_KNOCKBACK_TO_IMPULSE);
            commands
                .entity(entity)
                .try_insert(PuppetLastHit { impulse, part: PuppetPart::SpineUpper, at: now })
                .puppet_push(impulse);
        } else {
            let mag = if force > 0.0 { force * HIT_KNOCKBACK_TO_IMPULSE } else { FLASH_IMPULSE };
            let impulse = dir * mag;
            let kind = match part {
                PuppetPart::UpperArmL | PuppetPart::UpperArmR => HitKind::Slash,
                _ if force >= 2.5 => HitKind::Blunt,
                _ => HitKind::Slash,
            };
            commands
                .entity(entity)
                .try_insert(PuppetLastHit { impulse, part, at: now })
                .puppet_hit_kind(part, impulse, None, kind);
        }
    }
    if let (Some(raw), Ok((hero, hero_gt))) = (hero_push, hero.single())
        && builder.ensure(&mut commands, hero)
    {
        let len = raw.length();
        let dir = raw / len;
        let part = pick_hit_part(dir, hero_gt.right().as_vec3());
        // Escala pela força do empurrão, mantendo-se sempre parcial no herói.
        let mag = (HERO_HIT_IMPULSE * (0.80 + 0.25 * len)).clamp(44.0, 72.0);
        let kind = if len > 1.3 { HitKind::Blunt } else { HitKind::Slash };
        commands
            .entity(hero)
            .puppet_hit_kind(part, dir * mag, None, kind);
    }
}

/// Esbarrões entre personagens (`ActorBump` emitido por `actor_contact.rs`):
///
/// * Corrida normal (~4 m/s): jogo de ombro localizado (`puppet_hit` no ombro
///   do lado do contacto → torção axial natural sem derrubar).
/// * Sprint/dash/investida (~6–10 m/s): `puppet_push` em alvos mais leves ou
///   médios → passos de captura IPM + VMC implícito ou queda em colisões fortes.
/// * Herói: sempre reação parcial no ombro, nunca perde controlo.
#[allow(clippy::too_many_arguments)]
fn react_to_actor_bumps(
    mut commands: Commands,
    time: Res<Time>,
    mut bumps: MessageReader<crate::actor_contact::ActorBump>,
    builder: PuppetBuilder,
    actors: Query<(
        &GlobalTransform,
        Has<VaelPlayer>,
        Has<vael_engine::corpse::Corpse>,
        Option<&PuppetState>,
        Option<&PuppetConfig>,
        Option<&vael_engine::luau::LuaScriptRef>,
        Option<&Name>,
    )>,
    mut cooldowns: Local<HashMap<Entity, f32>>,
) {
    let now = time.elapsed_secs();
    if cooldowns.len() > 64 {
        cooldowns.retain(|_, &mut t| now - t < BUMP_COOLDOWN_SECS * 2.0);
    }

    let estimate_mass = |entity: Entity| -> f32 {
        let Ok((gt, is_hero, _, _, cfg, script, name)) = actors.get(entity) else {
            return 70.0;
        };
        if let Some(cfg) = cfg {
            return cfg.total_mass;
        }
        if is_hero {
            HERO_REF_MASS
        } else {
            let arch = archetype_from_hint(
                script.map(|s| s.path.as_str()),
                name.map(Name::as_str),
            );
            let scale = gt.to_scale_rotation_translation().0.y;
            creature_config_for(arch, scale).total_mass
        }
    };

    for bump in bumps.read() {
        if bump.closing_speed < BUMP_MIN_SPEED {
            continue;
        }
        let dir_a = Vec3::new(bump.normal.x, 0.0, bump.normal.y).normalize_or_zero();
        if dir_a == Vec3::ZERO {
            continue;
        }
        let mass_a = estimate_mass(bump.a);
        let mass_b = estimate_mass(bump.b);

        let pos_a = actors.get(bump.a).ok().map(|a| a.0.translation() + Vec3::Y * 1.45);
        let pos_b = actors.get(bump.b).ok().map(|b| b.0.translation() + Vec3::Y * 1.45);

        for (target, other_pos, other_mass, self_mass, raw_dir, pusher_vel, is_crowd_npc) in [
            (bump.a, pos_b, mass_b, mass_a, dir_a, bump.b_vel, bump.a_npc),
            (bump.b, pos_a, mass_a, mass_b, -dir_a, bump.a_vel, bump.b_npc),
        ] {
            if cooldowns.get(&target).is_some_and(|&t| now - t < BUMP_COOLDOWN_SECS) {
                continue;
            }
            let Ok((gt, is_hero, is_corpse, pstate, _, _, _)) = actors.get(target) else {
                continue;
            };
            if is_corpse {
                continue;
            }
            // Não empilhar esbarrão em quem já está em passo/reação física, caindo ou morto.
            if pstate.is_some_and(|s| {
                matches!(
                    s.mode,
                    PuppetMode::Active
                        | PuppetMode::Falling
                        | PuppetMode::Down
                        | PuppetMode::GettingUp
                        | PuppetMode::Dead
                )
            }) {
                continue;
            }
            let Some((imp, is_push)) = compute_bump_impulse(
                bump.closing_speed,
                other_mass,
                self_mass,
                is_hero,
                is_crowd_npc,
            ) else {
                continue;
            };
            if !builder.ensure(&mut commands, target) {
                continue;
            }
            cooldowns.insert(target, now);
            let dir = if is_crowd_npc && !is_hero {
                crowd_part_dir(raw_dir, pusher_vel)
            } else {
                raw_dir
            };
            if is_push {
                commands.entity(target).puppet_push(dir * imp);
                if let Some(look) = other_pos {
                    commands.entity(target).puppet_look_at(look, 0.9);
                }
            } else {
                // Jogo de ombro orgânico (`BumpReaction`): rotação axial do tronco + olhar orientador!
                let part = pick_hit_part(dir, gt.right().as_vec3());
                commands.entity(target).puppet_bump(part, dir * imp, other_pos);
            }
        }
    }
}

fn behaviour_from_name(name: &str) -> Option<Box<dyn Behaviour>> {
    Some(match name {
        "balance" => Box::new(BodyBalance::default()),
        "windmill" => Box::new(ArmsWindmill::default()),
        "catchfall" | "catch_fall" => Box::new(CatchFall::default()),
        "writhe" => Box::new(BodyWrithe::default()),
        "relax" => Box::new(Relax::default()),
        "stagger" | "stagger_fall" => Box::new(StaggerFall::new(Vec3::Z)),
        "wall_brace" | "wallbrace" | "brace" => Box::new(WallBrace::default()),
        "wound" | "reach_for_wound" => {
            Box::new(ReachForWound::new(PuppetPart::SpineUpper, Vec3::ZERO, 0.7))
        }
        "getup" | "physical_getup" => Box::new(PhysicalGetUp::new(true, 1.4)),
        _ => return None,
    })
}

/// Tipo de golpe vindo do Lua → [`HitKind`].
pub fn hit_kind_from_name(name: &str) -> Option<HitKind> {
    match name.trim().to_ascii_lowercase().as_str() {
        "blunt" | "impact" | "crush" => Some(HitKind::Blunt),
        "slash" | "cut" | "blade" => Some(HitKind::Slash),
        "pierce" | "stab" | "arrow" => Some(HitKind::Pierce),
        "blast" | "explosion" | "shock" => Some(HitKind::Blast),
        _ => None,
    }
}

/// Lado vindo do Lua → [`Side`].
pub fn side_from_name(name: &str) -> Option<Side> {
    match name.trim().to_ascii_lowercase().as_str() {
        "left" | "l" | "esq" | "esquerda" => Some(Side::Left),
        "right" | "r" | "dir" | "direita" => Some(Side::Right),
        _ => None,
    }
}

/// Nome de parte do Lua → [`PuppetPart`].
pub fn part_from_name(name: &str) -> Option<PuppetPart> {
    use PuppetPart::*;
    let n = name.trim().to_ascii_lowercase().replace(['-', ' '], "_");
    Some(match n.as_str() {
        "head" | "neck" => Head,
        "chest" | "spine_upper" | "spineupper" | "torso" => SpineUpper,
        "belly" | "spine" | "spine_lower" | "spinelower" => SpineLower,
        "pelvis" | "hips" => Pelvis,
        "arm_l" | "upperarm_l" | "upperarml" => UpperArmL,
        "arm_r" | "upperarm_r" | "upperarmr" => UpperArmR,
        "forearm_l" | "lowerarm_l" | "lowerarml" => LowerArmL,
        "forearm_r" | "lowerarm_r" | "lowerarmr" => LowerArmR,
        "hand_l" | "handl" => HandL,
        "hand_r" | "handr" => HandR,
        "leg_l" | "thigh_l" | "thighl" => ThighL,
        "leg_r" | "thigh_r" | "thighr" => ThighR,
        "shin_l" | "calf_l" | "calfl" => CalfL,
        "shin_r" | "calf_r" | "calfr" => CalfR,
        "foot_l" | "footl" => FootL,
        "foot_r" | "footr" => FootR,
        _ => return None,
    })
}

/// Aplica os pedidos do Lua.
fn apply_requests(
    mut commands: Commands,
    control: Res<VaelPuppetControl>,
    builder: PuppetBuilder,
    puppets: Query<(), With<PuppetRig>>,
) {
    let requests = std::mem::take(&mut control.lock().requests);
    for request in requests {
        match request {
            Request::Release(entity) => {
                commands.dismantle_puppet(entity);
            }
            Request::Hit { entity, part, impulse, point, kind } => {
                if builder.ensure(&mut commands, entity) {
                    commands.entity(entity).puppet_hit_kind(part, impulse, point, kind);
                }
            }
            Request::Bump { entity, part, impulse, bumper_pos } => {
                if builder.ensure(&mut commands, entity) {
                    commands.entity(entity).puppet_bump(part, impulse, bumper_pos);
                }
            }
            Request::Trip { entity, side, impulse } => {
                if builder.ensure(&mut commands, entity) {
                    commands.entity(entity).puppet_trip(side, impulse);
                }
            }
            Request::LookAt { entity, point, duration } => {
                if builder.ensure(&mut commands, entity) {
                    commands.entity(entity).puppet_look_at(point, duration);
                }
            }
            Request::Push { entity, impulse } => {
                if builder.ensure(&mut commands, entity) {
                    commands.entity(entity).puppet_push(impulse);
                }
            }
            Request::Stagger { entity, direction, duration } => {
                if builder.ensure(&mut commands, entity) {
                    let mut stagger = StaggerFall::new(direction);
                    if duration.is_finite() && duration > 0.1 {
                        stagger.duration = duration;
                    }
                    commands
                        .entity(entity)
                        .puppet(PuppetCommand::Activate)
                        .puppet(PuppetCommand::Start(Box::new(stagger)));
                }
            }
            Request::Kill(entity) => {
                if builder.ensure(&mut commands, entity) {
                    commands.entity(entity).puppet_kill();
                }
            }
            Request::Revive(entity) => {
                if puppets.contains(entity) {
                    commands.entity(entity).puppet(PuppetCommand::Revive);
                }
            }
            Request::GetUp(entity) => {
                if puppets.contains(entity) {
                    commands.entity(entity).puppet(PuppetCommand::GetUp);
                }
            }
            Request::Activate(entity) => {
                if builder.ensure(&mut commands, entity) {
                    commands.entity(entity).puppet(PuppetCommand::Activate);
                }
            }
            Request::Fall(entity) => {
                if builder.ensure(&mut commands, entity) {
                    commands.entity(entity).puppet(PuppetCommand::Fall);
                }
            }
            Request::Start { entity, behaviour } => match behaviour_from_name(&behaviour) {
                Some(b) if builder.ensure(&mut commands, entity) => {
                    commands.entity(entity).puppet(PuppetCommand::Start(b));
                }
                Some(_) => {}
                None => warn!("puppet: behaviour desconhecido {behaviour:?}"),
            },
        }
    }
}

/// Verifica se o herói está vivo nas fontes autoritativas (`viber.game()` / `PlayerVitals`).
fn hero_known_alive(vitals: Option<&crate::components::PlayerVitals>) -> bool {
    use vael_engine::luau::game::{game_value_to_json, with_game_store};

    let from_store = with_game_store(|store| {
        let hp = store
            .get("hp")
            .map(game_value_to_json)
            .and_then(|v| crate::compat::json_f32(&v))
            .filter(|v| v.is_finite());
        let dying = store
            .get("dying")
            .map(game_value_to_json)
            .and_then(|v| crate::compat::json_f32(&v))
            .filter(|v| v.is_finite())
            .unwrap_or(0.0);
        let pending = store
            .get("death_pending")
            .map(game_value_to_json)
            .and_then(|v| crate::compat::json_bool(&v))
            .unwrap_or(false);
        if dying > 0.0 || pending {
            return Some(false);
        }
        hp.map(|h| h > 0.0)
    })
    .flatten();

    if let Some(alive) = from_store {
        return alive;
    }
    vitals.is_some_and(|v| v.hp > 0.0)
}

/// Desmonta automaticamente o corpo físico do herói se:
///
/// * O herói estiver em estado `PuppetMode::Dead` mas já tiver renascido (`hp > 0`,
///   sem `dying` nem `death_pending`);
/// * O herói tiver acabado de ser teleportado (`TeleportSettle` ou raiz a > 6 m do
///   centro de massa físico `sense.com`).
///
/// Sem isto, ao morrer longe da praça e clicar "Tentar de novo" (`respawn_now`),
/// `write_back_pose` mantinha os ossos (`Pelvis`..`hand_r`) e a arma presos aos
/// corpos ragdoll deixados onde o herói morreu, tornando o herói e a arma
/// invisíveis no ponto de respawn.
fn reconcile_hero_puppet(
    mut commands: Commands,
    control: Res<VaelPuppetControl>,
    vitals: Option<Res<crate::components::PlayerVitals>>,
    heroes: Query<
        (
            Entity,
            &Transform,
            Option<&PuppetState>,
            Option<&PuppetSense>,
            Has<bevy_ext_core::TeleportSettle>,
        ),
        (
            Or<(With<VaelPlayer>, With<bevy_ext_core::Player>)>,
            With<Puppet>,
        ),
    >,
) {
    let alive = hero_known_alive(vitals.as_deref());
    for (entity, tf, state, sense, settling) in &heroes {
        let dead_puppet = state.is_some_and(|s| s.mode == PuppetMode::Dead);
        let teleported_away = sense.is_some_and(|s| {
            s.total_mass > 0.0 && tf.translation.distance_squared(s.com) > 6.0 * 6.0
        });
        if settling || teleported_away || (dead_puppet && alive) {
            control.clear_entity(entity);
            commands.dismantle_puppet(entity);
        }
    }
}

/// Enquanto o corpo físico manda (`busy`), desliga o knockback cinemático e
/// trava `AiLocomotion` para que a raiz não arraste os pés plantados do VMC.
fn calm_kinematic_motion(
    mut commands: Commands,
    mut puppets: Query<
        (
            Entity,
            &PuppetState,
            Has<vael_engine::physics_fx::Knockback>,
            Option<&mut vael_engine::ai::AiLocomotion>,
        ),
        Without<VaelPlayer>,
    >,
) {
    for (entity, state, has_kb, loco) in &mut puppets {
        if !busy(state.mode) {
            continue;
        }
        if has_kb {
            commands.entity(entity).remove::<vael_engine::physics_fx::Knockback>();
        }
        if let Some(mut loco) = loco {
            loco.halt();
            loco.velocity = Vec2::ZERO;
        }
    }
}

/// Reage aos eventos do ciclo de vida do puppet (`PuppetEvent`):
///
/// * Queda no chão (`Falling → Down`): poeira de impacto no solo + micro shake
///   de câmara proporcional à massa quando criaturas pesadas caem perto do herói.
/// * Levantar (`GettingUp { face_up }`): poeira leve de apoio e gesto opcional
///   `getup_back`/`getup_front` via `UnboundGesture` (no-op quando o rig não o tem).
fn handle_puppet_events(
    mut commands: Commands,
    mut events: MessageReader<PuppetEvent>,
    mut shake: Option<ResMut<vael_engine::camera::CameraShake>>,
    puppets: Query<(&PuppetSense, &PuppetConfig, &GlobalTransform)>,
    hero: Query<&GlobalTransform, With<VaelPlayer>>,
) {
    let hero_pos = hero.single().ok().map(|g| g.translation());
    let dust = crate::collect_fx::burst_spec("ground-dust");

    for ev in events.read() {
        let Ok((sense, cfg, gt)) = puppets.get(ev.puppet) else {
            continue;
        };
        let ground_pos = Vec3::new(sense.com.x, sense.ground + 0.08, sense.com.z);
        match ev.kind {
            PuppetEventKind::ModeChanged { from: PuppetMode::Falling, to: PuppetMode::Down } => {
                let heavy = cfg.total_mass >= 100.0;
                let count = if heavy { 10 } else { 5 };
                vael_engine::particles::spawn_burst(&mut commands, &dust, ground_pos, count);
                if heavy
                    && let Some(hpos) = hero_pos
                {
                    let dist = hpos.distance(gt.translation());
                    if dist < FALL_SHAKE_RANGE {
                        let falloff = 1.0 - (dist / FALL_SHAKE_RANGE);
                        let trauma = (0.06 + 0.12 * (cfg.total_mass / 150.0).min(1.5)) * falloff;
                        crate::collect_fx::subtle_camera_shake(shake.as_deref_mut(), trauma);
                    }
                }
            }
            PuppetEventKind::GettingUp { face_up } => {
                vael_engine::particles::spawn_burst(&mut commands, &dust, ground_pos, 3);
                let clip = if face_up { "getup_back" } else { "getup_front" };
                commands.trigger(vael_engine::luau::runtime::UnboundGesture {
                    entity: ev.puppet,
                    name: clip.to_string(),
                    speed: 1.0,
                });
            }
            PuppetEventKind::BodyImpact { point, impulse, speed, .. } => {
                if speed >= 2.2 || impulse >= 16.0 {
                    let count = if impulse >= 40.0 || speed >= 5.0 { 7 } else { 3 };
                    vael_engine::particles::spawn_burst(&mut commands, &dust, point + Vec3::Y * 0.04, count);
                    if cfg.total_mass >= 90.0
                        && impulse >= 35.0
                        && let Some(hpos) = hero_pos
                    {
                        let dist = hpos.distance(point);
                        if dist < FALL_SHAKE_RANGE {
                            let falloff = 1.0 - (dist / FALL_SHAKE_RANGE);
                            let trauma = (0.04 + 0.08 * (impulse / 70.0).min(1.4)) * falloff;
                            crate::collect_fx::subtle_camera_shake(shake.as_deref_mut(), trauma);
                        }
                    }
                }
            }
            PuppetEventKind::WallBraced { point, .. } => {
                vael_engine::particles::spawn_burst(&mut commands, &dust, point, 2);
            }
            PuppetEventKind::StepTaken { point, .. } => {
                if cfg.total_mass >= 100.0 {
                    vael_engine::particles::spawn_burst(&mut commands, &dust, point + Vec3::Y * 0.03, 2);
                }
            }
            PuppetEventKind::Tripped { .. } => {
                vael_engine::particles::spawn_burst(&mut commands, &dust, ground_pos, 3);
            }
            PuppetEventKind::ModeChanged { .. } | PuppetEventKind::RootMoved => {}
        }
    }
}

/// O corpo físico está a mandar (a IA deve esperar).
pub fn busy(mode: PuppetMode) -> bool {
    matches!(
        mode,
        PuppetMode::Active | PuppetMode::Falling | PuppetMode::Down | PuppetMode::GettingUp
    )
}

pub fn mode_name(mode: PuppetMode) -> &'static str {
    match mode {
        PuppetMode::Animated => "animated",
        PuppetMode::Active => "active",
        PuppetMode::Falling => "falling",
        PuppetMode::Down => "down",
        PuppetMode::GettingUp => "getting_up",
        PuppetMode::Dead => "dead",
    }
}

pub fn stance_name(stance: StanceState) -> &'static str {
    match stance {
        StanceState::Double => "double",
        StanceState::Left => "left",
        StanceState::Right => "right",
        StanceState::Flight => "flight",
    }
}

fn publish_snapshots(
    control: Res<VaelPuppetControl>,
    puppets: Query<(Entity, &PuppetState, &PuppetSense, Option<&PuppetNeuroState>), With<PuppetRig>>,
) {
    let mut ctl = control.lock();
    ctl.snapshots.clear();
    for (entity, state, sense, neuro) in &puppets {
        let plant_quality = 0.5
            * (sense.foot_plant_quality(Side::Left) + sense.foot_plant_quality(Side::Right));
        let (trauma, fatigue, hit_combo) = neuro
            .map(|n| (n.trauma, n.fatigue, n.hit_combo))
            .unwrap_or((0.0, 0.0, 0));
        ctl.snapshots.insert(
            entity.to_bits(),
            PuppetSnapshot {
                mode: state.mode,
                stance: sense.stance,
                feet_down: sense.feet_down() as u8,
                trunk_tilt: sense.trunk_tilt,
                capture_margin: sense.capture_margin,
                capture_radius: sense.capture_region_radius,
                speed: sense.horizontal_speed(),
                angular_momentum: sense.angular_momentum.length(),
                plant_quality,
                pelvis_height: sense.pelvis_height,
                face_up: state.face_up,
                trauma,
                fatigue,
                hit_combo,
                ground_clearance: sense.ground_clearance,
                wall_nearby: sense.nearby_wall.is_some(),
                peak_impact_speed: sense.peak_impact_speed,
            },
        );
    }
}

fn install_puppet_api(control: Res<VaelPuppetControl>) {
    let inner = control.clone();
    crate::script_host::register_installer(move |lua| install_puppet_api_inner(lua, &inner));
}

fn vec3(x: Option<f32>, y: Option<f32>, z: Option<f32>) -> Vec3 {
    Vec3::new(x.unwrap_or(0.0), y.unwrap_or(0.0), z.unwrap_or(0.0))
}

fn install_puppet_api_inner(lua: &Lua, control: &VaelPuppetControl) -> mlua::Result<()> {
    let vael = vael_table(lua)?;

    // `viber.vael.puppet_enabled()` → true quando `VaelPuppetPlugin` está ativo.
    vael.set("puppet_enabled", lua.create_function(|_, ()| Ok(true))?)?;

    // `viber.vael.puppet_busy(id)` → o corpo físico manda (cambaleia, caído, a levantar).
    vael_fn!(lua, vael, "puppet_busy", [control = control.clone()], move |_, id: i64| {
        Ok(control.lock().snapshots.get(&(id as u64)).is_some_and(|s| busy(s.mode)))
    });

    // `viber.vael.puppet_state(id)` → "animated"|"active"|"falling"|"down"|"getting_up"|"dead"|nil.
    vael_fn!(lua, vael, "puppet_state", [control = control.clone()], move |_, id: i64| {
        Ok(control.lock().snapshots.get(&(id as u64)).map(|s| mode_name(s.mode)))
    });

    // `viber.vael.puppet_info(id)` → tabela de propriocepção completa (ou nil).
    vael_fn!(lua, vael, "puppet_info", [control = control.clone()], move |lua, id: i64| {
        let Some(s) = control.lock().snapshots.get(&(id as u64)).copied() else {
            return Ok(mlua::Value::Nil);
        };
        let t = lua.create_table()?;
        t.set("mode", mode_name(s.mode))?;
        t.set("busy", busy(s.mode))?;
        t.set("stance", stance_name(s.stance))?;
        t.set("feet_down", s.feet_down)?;
        t.set("trunk_tilt", s.trunk_tilt)?;
        t.set("capture_margin", s.capture_margin)?;
        t.set("capture_radius", s.capture_radius)?;
        t.set("speed", s.speed)?;
        t.set("angular_momentum", s.angular_momentum)?;
        t.set("plant_quality", s.plant_quality)?;
        t.set("pelvis_height", s.pelvis_height)?;
        t.set("face_up", s.face_up)?;
        t.set("trauma", s.trauma)?;
        t.set("fatigue", s.fatigue)?;
        t.set("hit_combo", s.hit_combo)?;
        t.set("ground_clearance", s.ground_clearance)?;
        t.set("wall_nearby", s.wall_nearby)?;
        t.set("peak_impact_speed", s.peak_impact_speed)?;
        Ok(mlua::Value::Table(t))
    });

    // `viber.vael.puppet_push(id, ix, iy, iz)` — empurrão (N·s) no tronco.
    vael_fn!(
        lua,
        vael,
        "puppet_push",
        [control = control.clone()],
        move |_, (id, x, y, z): (i64, Option<f32>, Option<f32>, Option<f32>)| {
            let entity = entity_from_id(id)?;
            control.lock().requests.push(Request::Push { entity, impulse: vec3(x, y, z) });
            Ok(())
        }
    );

    // `viber.vael.puppet_hit(id, "head", ix, iy, iz [, px, py, pz [, "blunt"|"slash"|"pierce"|"blast"]])`
    vael_fn!(
        lua,
        vael,
        "puppet_hit",
        [control = control.clone()],
        move |_,
              (id, part, x, y, z, px, py, pz, kind_str): (
            i64,
            String,
            Option<f32>,
            Option<f32>,
            Option<f32>,
            Option<f32>,
            Option<f32>,
            Option<f32>,
            Option<String>,
        )| {
            let entity = entity_from_id(id)?;
            let part = part_from_name(&part)
                .ok_or_else(|| mlua::Error::runtime(format!("parte desconhecida: {part}")))?;
            let point = match (px, py, pz) {
                (Some(px), Some(py), Some(pz)) => Some(Vec3::new(px, py, pz)),
                _ => None,
            };
            let kind = match kind_str.as_deref() {
                Some(k) => hit_kind_from_name(k)
                    .ok_or_else(|| mlua::Error::runtime(format!("tipo de golpe desconhecido: {k}")))?,
                None => HitKind::Blunt,
            };
            control.lock().requests.push(Request::Hit {
                entity,
                part,
                impulse: vec3(x, y, z),
                point,
                kind,
            });
            Ok(())
        }
    );

    // `viber.vael.puppet_bump(id, part, ix, iy, iz [, bx, by, bz])` — esbarrão de ombro + olhar orientador.
    vael_fn!(
        lua,
        vael,
        "puppet_bump",
        [control = control.clone()],
        move |_,
              (id, part, x, y, z, bx, by, bz): (
            i64,
            String,
            Option<f32>,
            Option<f32>,
            Option<f32>,
            Option<f32>,
            Option<f32>,
            Option<f32>,
        )| {
            let entity = entity_from_id(id)?;
            let part = part_from_name(&part)
                .ok_or_else(|| mlua::Error::runtime(format!("parte desconhecida: {part}")))?;
            let bumper_pos = match (bx, by, bz) {
                (Some(bx), Some(by), Some(bz)) => Some(Vec3::new(bx, by, bz)),
                _ => None,
            };
            control.lock().requests.push(Request::Bump {
                entity,
                part,
                impulse: vec3(x, y, z),
                bumper_pos,
            });
            Ok(())
        }
    );

    // `viber.vael.puppet_trip(id, "left"|"right", ix, iy, iz)` — tropeço físico na canela.
    vael_fn!(
        lua,
        vael,
        "puppet_trip",
        [control = control.clone()],
        move |_, (id, side_str, x, y, z): (i64, String, Option<f32>, Option<f32>, Option<f32>)| {
            let entity = entity_from_id(id)?;
            let side = side_from_name(&side_str)
                .ok_or_else(|| mlua::Error::runtime(format!("lado desconhecido: {side_str}")))?;
            control.lock().requests.push(Request::Trip {
                entity,
                side,
                impulse: vec3(x, y, z),
            });
            Ok(())
        }
    );

    // `viber.vael.puppet_look_at(id, x, y, z [, duration])` — olhar orientador cabeça/pescoço.
    vael_fn!(
        lua,
        vael,
        "puppet_look_at",
        [control = control.clone()],
        move |_, (id, x, y, z, duration): (i64, f32, f32, f32, Option<f32>)| {
            let entity = entity_from_id(id)?;
            control.lock().requests.push(Request::LookAt {
                entity,
                point: Vec3::new(x, y, z),
                duration: duration.unwrap_or(1.2),
            });
            Ok(())
        }
    );

    // `viber.vael.puppet_stagger(id, dx, dz [, duration])` — cambaleio direcionado até cair.
    vael_fn!(
        lua,
        vael,
        "puppet_stagger",
        [control = control.clone()],
        move |_, (id, dx, dz, duration): (i64, Option<f32>, Option<f32>, Option<f32>)| {
            let entity = entity_from_id(id)?;
            let dir = Vec3::new(dx.unwrap_or(0.0), 0.0, dz.unwrap_or(1.0)).normalize_or(Vec3::Z);
            control.lock().requests.push(Request::Stagger {
                entity,
                direction: dir,
                duration: duration.unwrap_or(1.6),
            });
            Ok(())
        }
    );

    // `viber.vael.puppet_start(id, "windmill"|"writhe"|"balance"|"catchfall"|"relax"|"stagger")`.
    vael_fn!(
        lua,
        vael,
        "puppet_start",
        [control = control.clone()],
        move |_, (id, behaviour): (i64, String)| {
            let entity = entity_from_id(id)?;
            control.lock().requests.push(Request::Start { entity, behaviour });
            Ok(())
        }
    );

    for (name, make) in [
        ("puppet_kill", Request::Kill as fn(Entity) -> Request),
        ("puppet_revive", Request::Revive),
        ("puppet_getup", Request::GetUp),
        ("puppet_release", Request::Release),
        ("puppet_activate", Request::Activate),
        ("puppet_fall", Request::Fall),
    ] {
        let control = control.clone();
        vael.set(
            name,
            lua.create_function(move |_, id: i64| {
                let entity = entity_from_id(id)?;
                control.lock().requests.push(make(entity));
                Ok(())
            })?,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lua_part_names_resolve() {
        assert_eq!(part_from_name("Head"), Some(PuppetPart::Head));
        assert_eq!(part_from_name("chest"), Some(PuppetPart::SpineUpper));
        assert_eq!(part_from_name("arm-l"), Some(PuppetPart::UpperArmL));
        assert_eq!(part_from_name("foot_r"), Some(PuppetPart::FootR));
        assert_eq!(part_from_name("tail"), None);
    }

    #[test]
    fn busy_modes_are_the_physical_ones() {
        assert!(busy(PuppetMode::Down));
        assert!(busy(PuppetMode::Active));
        assert!(!busy(PuppetMode::Animated));
        assert!(!busy(PuppetMode::Dead));
    }

    #[test]
    fn archetype_profiles_scale_mass_and_threshold() {
        assert_eq!(
            archetype_from_hint(Some("enemies/goblin.lua"), None),
            CreatureArchetype::Light
        );
        assert_eq!(
            archetype_from_hint(Some("enemies/goblin-brute.lua"), None),
            CreatureArchetype::Brute
        );
        assert_eq!(
            archetype_from_hint(Some("enemies/bandit-shield.lua"), None),
            CreatureArchetype::Shielded
        );
        assert_eq!(
            archetype_from_hint(Some("bosses/bog-warden.lua"), None),
            CreatureArchetype::Boss
        );
        let light = creature_config_for(CreatureArchetype::Light, 1.0);
        let brute = creature_config_for(CreatureArchetype::Brute, 1.25);
        let boss = creature_config_for(CreatureArchetype::Boss, 1.0);
        assert!(light.total_mass < brute.total_mass);
        assert!(light.partial_hit_impulse < brute.partial_hit_impulse);
        assert!(brute.partial_hit_impulse > 48.0);
        assert!(boss.total_mass > 140.0);
    }

    #[test]
    fn hit_part_selects_lateral_arm_for_side_blows() {
        // Alvo com right = +X: golpe empurrando para +X veio do lado esquerdo.
        assert_eq!(pick_hit_part(Vec3::X, Vec3::X), PuppetPart::UpperArmL);
        assert_eq!(pick_hit_part(Vec3::NEG_X, Vec3::X), PuppetPart::UpperArmR);
        assert_eq!(pick_hit_part(Vec3::Z, Vec3::X), PuppetPart::SpineUpper);
    }

    #[test]
    fn stance_names_cover_all_states() {
        assert_eq!(stance_name(StanceState::Double), "double");
        assert_eq!(stance_name(StanceState::Left), "left");
        assert_eq!(stance_name(StanceState::Right), "right");
        assert_eq!(stance_name(StanceState::Flight), "flight");
    }

    #[test]
    fn bump_impulse_distinguishes_walk_shoulder_and_dash_push() {
        // Parado / ruído (< 0.7 m/s): ignorado.
        assert_eq!(compute_bump_impulse(0.3, 75.0, 70.0, false, true), None);
        // Caminhada (1.8 m/s) e corrida normal (4.0 m/s) em aldeão: jogo de ombro parcial.
        let (imp_walk, push_walk) = compute_bump_impulse(1.8, 75.0, 70.0, false, true).unwrap();
        assert!(!push_walk);
        assert!(imp_walk >= 9.0 && imp_walk < 16.0);
        let (imp_run, push_run) = compute_bump_impulse(4.0, 75.0, 70.0, false, true).unwrap();
        assert!(!push_run);
        assert!(imp_run >= 18.0 && imp_run < 28.0);
        // Sprint (6.0 m/s) em aldeão: vira puppet_push moderado (passos laterais de pé).
        let (imp_sprint, push_sprint) = compute_bump_impulse(6.0, 75.0, 70.0, false, true).unwrap();
        assert!(push_sprint);
        assert!(imp_sprint >= 28.0 && imp_sprint <= 40.0);
        // Dash (8.4 m/s) em goblin hostil (52 kg): vira puppet_push forte.
        let (imp_dash, push_dash) = compute_bump_impulse(8.4, 75.0, 52.0, false, false).unwrap();
        assert!(push_dash);
        assert!(imp_dash >= 45.0);
        // Herói nunca vira push (sempre reação parcial no ombro).
        let (imp_hero, push_hero) = compute_bump_impulse(10.0, 120.0, 75.0, true, false).unwrap();
        assert!(!push_hero);
        assert!(imp_hero <= BUMP_HERO_MAX_IMPULSE);
        // Goblin leve encostando devagar num chefe pesado não abala o chefe.
        assert_eq!(compute_bump_impulse(1.5, 52.0, 150.0, false, false), None);
    }

    #[test]
    fn crowd_part_dir_opens_corridor_sideways() {
        // Corredor indo para +Z encontra NPC ligeiramente à esquerda (-X):
        // deve empurrá-lo diagonalmente para -X (abrindo corredor), não só +Z.
        let dir = crowd_part_dir(Vec3::new(-0.2, 0.0, 1.0), Vec2::new(0.0, 5.0));
        assert!(dir.x < -0.7, "deve abrir para -X, got {dir:?}");
        assert!(dir.z > 0.2, "mantém leve componente +Z, got {dir:?}");
    }

    #[test]
    fn lua_hit_kind_side_and_new_behaviours_resolve() {
        assert_eq!(hit_kind_from_name("blunt"), Some(HitKind::Blunt));
        assert_eq!(hit_kind_from_name("Slash"), Some(HitKind::Slash));
        assert_eq!(hit_kind_from_name("arrow"), Some(HitKind::Pierce));
        assert_eq!(hit_kind_from_name("blast"), Some(HitKind::Blast));
        assert_eq!(hit_kind_from_name("unknown"), None);

        assert_eq!(side_from_name("left"), Some(Side::Left));
        assert_eq!(side_from_name("direita"), Some(Side::Right));
        assert_eq!(side_from_name("center"), None);

        assert!(behaviour_from_name("wall_brace").is_some());
        assert!(behaviour_from_name("wound").is_some());
        assert!(behaviour_from_name("getup").is_some());
    }
}

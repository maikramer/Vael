//! Animações vivas e vibrantes de coleta, impacto e loot no mundo.
//!
//! Dá resposta tátil e visual às colheitas (árvores, rochas, arbustos,
//! cogumelos, musgo, baús) e aos itens largados por props destrutíveis:
//!
//! - [`CollectOrb`]: fragmentos/orbes coloridos do recurso que saltam em leque
//!   do ponto de quebra, ganham impulso em arco espiral até ao peito do herói
//!   deixando pequenos rastros luminosos ([`TrailMote`]) e terminam num anel
//!   expansivo de absorção ([`AbsorbPulse`]) + faíscas.
//! - [`ImpactWobble`]: squash-and-stretch elástico breve ao golpear recursos e
//!   props, preservando exatamente a escala original no fim.
//! - [`PickupOrbitGlint`] e [`PickupGroundRing`]: faíscas orbitais em hélice e
//!   halo de chão vibrante sob os itens largados por destrutíveis.
//! - [`CollectBurstQueue`] e [`HitFlashQueue`]: filas partilhadas entre Rust e
//!   Lua (`viber.vael.collect_burst` / `viber.vael.hit_flash`) para que ambos
//!   os caminhos produzam a mesma coreografia.

use std::f32::consts::{PI, TAU};
use std::sync::{Arc, Mutex};

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy_ext_engine::camera_glue::CameraShake;
use viber::particles::spawn_burst;
use viber::player::Player;
use viber::recipes::ParticleSpec;
use viber::scene_loader::SceneEntity;

use crate::vault::{vael_fn, vael_table};

/// Helper para construir um `ParticleSpec` de rajada pontual (`looping = false`).
pub(crate) fn burst_spec(preset: &str) -> ParticleSpec {
    ParticleSpec {
        preset: preset.to_owned(),
        emission_rate: None,
        life: None,
        speed: None,
        size: None,
        color: None,
        shape_radius: None,
        looping: false,
        world_space: true,
    }
}

/// Limite máximo de orbes simultâneos em voo (proteção contra spikes).
const MAX_ACTIVE_ORBS: usize = 48;
/// Limite máximo de partículas de rastro simultâneas.
const MAX_ACTIVE_MOTES: usize = 160;
/// Intervalo mínimo (s) entre anéis de absorção no herói para não empilhar.
const ABSORB_PULSE_COOLDOWN: f32 = 0.09;

/// Marca uma entidade visual decorativa (glint, anel, rastro, marca) que NÃO
/// deve receber contorno do `bevy_mod_outline`, mesmo sendo filha de um alvo.
#[derive(Component, Debug, Clone, Copy, Default)]
pub(crate) struct IgnoreOutline;

/// Família visual do recurso coletado: decide forma, cor e brilho dos orbes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum CollectVisualKind {
    Wood,
    DarkWood,
    Stone,
    Crystal,
    Berry,
    Herb,
    Gold,
    Life,
    Generic,
}

impl CollectVisualKind {
    /// Resolve a família visual a partir da chave de recurso/item do jogo.
    pub(crate) fn from_resource_kind(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "wood" | "timber" | "log" => Self::Wood,
            "dark-wood" | "dark_wood" => Self::DarkWood,
            "stone" | "rock" | "ore" | "iron" | "copper" | "coal" => Self::Stone,
            "crystal" | "gem" | "shard" => Self::Crystal,
            "berry" | "berries" | "fruit" => Self::Berry,
            "bog-moss" | "bog_moss" | "moss" | "mushroom" | "herb" | "plant" => Self::Herb,
            "gold" | "coin" | "coins" | "chest" | "treasure" => Self::Gold,
            "life" | "potion" | "heal" | "heart" => Self::Life,
            _ => Self::Generic,
        }
    }

    fn palette(self) -> (Color, LinearRgba) {
        match self {
            Self::Wood => (
                Color::srgb(0.95, 0.68, 0.28),
                LinearRgba::rgb(3.2, 1.85, 0.45),
            ),
            Self::DarkWood => (
                Color::srgb(0.74, 0.44, 0.24),
                LinearRgba::rgb(2.6, 1.2, 0.55),
            ),
            Self::Stone => (
                Color::srgb(0.55, 0.88, 0.98),
                LinearRgba::rgb(1.1, 2.7, 3.6),
            ),
            Self::Crystal => (
                Color::srgb(0.45, 0.98, 0.92),
                LinearRgba::rgb(1.4, 3.8, 3.6),
            ),
            Self::Berry => (
                Color::srgb(1.0, 0.28, 0.52),
                LinearRgba::rgb(3.8, 0.75, 1.65),
            ),
            Self::Herb => (
                Color::srgb(0.36, 0.96, 0.56),
                LinearRgba::rgb(0.9, 3.4, 1.5),
            ),
            Self::Gold => (
                Color::srgb(1.0, 0.84, 0.22),
                LinearRgba::rgb(4.4, 3.1, 0.55),
            ),
            Self::Life => (
                Color::srgb(1.0, 0.34, 0.58),
                LinearRgba::rgb(4.2, 0.85, 1.75),
            ),
            Self::Generic => (
                Color::srgb(0.96, 0.88, 0.48),
                LinearRgba::rgb(3.4, 2.8, 1.1),
            ),
        }
    }

    fn base_scale(self) -> Vec3 {
        match self {
            Self::Wood | Self::DarkWood => Vec3::new(0.11, 0.055, 0.065),
            Self::Stone | Self::Crystal => Vec3::splat(0.088),
            Self::Berry | Self::Life => Vec3::splat(0.082),
            Self::Herb => Vec3::new(0.075, 0.105, 0.055),
            Self::Gold => Vec3::new(0.095, 0.032, 0.095),
            Self::Generic => Vec3::splat(0.08),
        }
    }

    fn burst_preset(self) -> &'static str {
        match self {
            Self::Wood | Self::DarkWood | Self::Berry | Self::Herb => "leaves",
            Self::Stone | Self::Crystal | Self::Gold | Self::Life | Self::Generic => "sparkle",
        }
    }
}

/// Pedido de explosão de orbes de coleta.
#[derive(Debug, Clone)]
pub(crate) struct CollectBurstRequest {
    pub kind: CollectVisualKind,
    pub origin: Vec3,
    pub count: u8,
}

/// Fila thread-safe de pedidos de animação de coleta (alimentada por Rust e Lua).
#[derive(Resource, Clone, Default)]
pub(crate) struct CollectBurstQueue(pub Arc<Mutex<Vec<CollectBurstRequest>>>);

impl CollectBurstQueue {
    pub(crate) fn push(&self, kind: CollectVisualKind, origin: Vec3, count: u8) {
        if !origin.is_finite() {
            return;
        }
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(CollectBurstRequest {
                kind,
                origin,
                count: count.clamp(1, 8),
            });
    }
}

/// Pedido de flash de impacto no outline (+ wobble opcional).
#[derive(Debug, Clone, Copy)]
pub(crate) struct HitFlashRequest {
    pub entity: Entity,
    pub wobble: bool,
    pub strength: f32,
}

/// Fila partilhada de flashes de impacto (lida por `src/outline.rs` e `src/collect_fx.rs`).
#[derive(Resource, Clone, Default)]
pub(crate) struct HitFlashQueue(pub Arc<Mutex<Vec<HitFlashRequest>>>);

impl HitFlashQueue {
    pub(crate) fn push(&self, entity: Entity, wobble: bool, strength: f32) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(HitFlashRequest {
                entity,
                wobble,
                strength: strength.clamp(0.2, 2.0),
            });
    }
}

/// Orbe/fragmento visual em voo do ponto de coleta até ao peito do herói.
#[derive(Component, Debug, Clone)]
pub(crate) struct CollectOrb {
    pub kind: CollectVisualKind,
    pub age: f32,
    pub pop_duration: f32,
    pub fly_duration: f32,
    pub velocity: Vec3,
    pub spin: Vec3,
    pub base_scale: Vec3,
    pub fly_start: Vec3,
    pub control_offset: Vec3,
    pub homing: bool,
    pub trail_timer: f32,
    pub emit_absorb_burst: bool,
}

/// Partícula de rastro deixada por um [`CollectOrb`] durante o voo.
#[derive(Component, Debug, Clone)]
struct TrailMote {
    age: f32,
    lifetime: f32,
    start_scale: f32,
    drift: Vec3,
}

/// Anel expansivo luminoso que aparece ao redor do peito do herói na absorção.
#[derive(Component, Debug, Clone)]
struct AbsorbPulse {
    age: f32,
    duration: f32,
    start_scale: f32,
    end_scale: f32,
    material: Handle<StandardMaterial>,
    base_emissive: LinearRgba,
}

/// Deformação elástica breve (squash & stretch) quando um recurso/prop leva golpe.
#[derive(Component, Debug, Clone)]
pub(crate) struct ImpactWobble {
    pub base_scale: Vec3,
    pub age: f32,
    pub duration: f32,
    pub strength: f32,
}

/// Faísca que orbita em hélice viva ao redor de um item largado no chão.
#[derive(Component, Debug, Clone, Copy)]
pub(crate) struct PickupOrbitGlint {
    pub phase: f32,
    pub radius: f32,
    pub base_y: f32,
    pub bob_amp: f32,
    pub speed: f32,
}

/// Anel luminoso de chão sob um pickup (gira e respira suavemente).
#[derive(Component, Debug, Clone, Copy)]
pub(crate) struct PickupGroundRing {
    pub floor_y: f32,
    pub phase: f32,
}

/// Cache de meshes e materiais partilhados para orbes/rastros/anéis.
struct FxArt {
    wood_mesh: Handle<Mesh>,
    stone_mesh: Handle<Mesh>,
    sphere_mesh: Handle<Mesh>,
    coin_mesh: Handle<Mesh>,
    ring_mesh: Handle<Mesh>,
    mote_mesh: Handle<Mesh>,
    materials: std::collections::HashMap<CollectVisualKind, Handle<StandardMaterial>>,
}

impl FxArt {
    fn new(meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>) -> Self {
        let wood_mesh = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
        let stone_mesh = meshes.add(Tetrahedron::default());
        let sphere_mesh = meshes.add(Sphere::new(0.55).mesh().uv(10, 6));
        let coin_mesh = meshes.add(Cylinder::new(0.55, 0.35));
        let ring_mesh = meshes.add(Torus::new(0.88, 1.0));
        let mote_mesh = meshes.add(Sphere::new(0.5).mesh().uv(6, 4));
        let mut mat_map = std::collections::HashMap::new();
        for kind in [
            CollectVisualKind::Wood,
            CollectVisualKind::DarkWood,
            CollectVisualKind::Stone,
            CollectVisualKind::Crystal,
            CollectVisualKind::Berry,
            CollectVisualKind::Herb,
            CollectVisualKind::Gold,
            CollectVisualKind::Life,
            CollectVisualKind::Generic,
        ] {
            let (base_color, emissive) = kind.palette();
            let handle = materials.add(StandardMaterial {
                base_color,
                emissive,
                metallic: if matches!(kind, CollectVisualKind::Gold | CollectVisualKind::Crystal) {
                    0.55
                } else {
                    0.12
                },
                perceptual_roughness: 0.28,
                ..default()
            });
            mat_map.insert(kind, handle);
        }
        Self {
            wood_mesh,
            stone_mesh,
            sphere_mesh,
            coin_mesh,
            ring_mesh,
            mote_mesh,
            materials: mat_map,
        }
    }

    fn mesh_for(&self, kind: CollectVisualKind) -> Handle<Mesh> {
        match kind {
            CollectVisualKind::Wood | CollectVisualKind::DarkWood => self.wood_mesh.clone(),
            CollectVisualKind::Stone | CollectVisualKind::Crystal | CollectVisualKind::Herb => {
                self.stone_mesh.clone()
            }
            CollectVisualKind::Gold => self.coin_mesh.clone(),
            CollectVisualKind::Berry | CollectVisualKind::Life | CollectVisualKind::Generic => {
                self.sphere_mesh.clone()
            }
        }
    }

    fn material_for(&self, kind: CollectVisualKind) -> Handle<StandardMaterial> {
        self.materials
            .get(&kind)
            .cloned()
            .unwrap_or_else(|| self.materials[&CollectVisualKind::Generic].clone())
    }
}

/// Plugin que instala as animações vivas de coleta e impacto.
pub struct VaelCollectFxPlugin;

impl Plugin for VaelCollectFxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CollectBurstQueue>()
            .init_resource::<HitFlashQueue>()
            .add_systems(
                Startup,
                install_lua_fx_api.after(crate::vault::install_vault_api),
            )
            .add_systems(
                Update,
                (
                    spawn_collect_orbs,
                    drive_collect_orbs,
                    drive_trail_motes,
                    drive_absorb_pulses,
                    drive_impact_wobbles,
                    animate_pickup_details,
                ),
            );
    }
}

/// Expõe `viber.vael.collect_burst(kind, x, y, z, count)` e
/// `viber.vael.hit_flash(entity_bits)` aos scripts Lua.
fn install_lua_fx_api(bursts: Res<CollectBurstQueue>, flashes: Res<HitFlashQueue>) {
    let bursts = bursts.clone();
    let flashes = flashes.clone();
    crate::script_host::register_installer(move |lua| -> mlua::Result<()> {
        let vael = vael_table(lua)?;
        vael_fn!(
            lua,
            vael,
            "collect_burst",
            [bursts = bursts.clone()],
            move |_, (kind, x, y, z, count): (String, f32, f32, f32, Option<i64>)| {
                let n = count.unwrap_or(4).clamp(1, 8) as u8;
                bursts.push(
                    CollectVisualKind::from_resource_kind(&kind),
                    Vec3::new(x, y, z),
                    n,
                );
                Ok(())
            }
        );
        vael_fn!(
            lua,
            vael,
            "hit_flash",
            [flashes = flashes.clone()],
            move |_, bits: i64| {
                if let Some(entity) = Entity::try_from_bits(bits as u64) {
                    flashes.push(entity, true, 1.0);
                }
                Ok(())
            }
        );
        Ok(())
    });
}

/// Dispara um micro shake subtil de câmara (respeitando `A11y` na câmara).
pub(crate) fn subtle_camera_shake(shake: Option<&mut CameraShake>, amount: f32) {
    if let Some(shake) = shake
        && amount.is_finite()
        && amount > 0.0
    {
        shake.trauma = (shake.trauma + amount).min(0.45);
    }
}

/// Aplica ou renova [`ImpactWobble`] sem empilhar a deformação sobre a escala já deformada.
pub(crate) fn apply_impact_wobble(
    commands: &mut Commands,
    entity: Entity,
    current_scale: Vec3,
    existing: Option<&ImpactWobble>,
    strength: f32,
) {
    let base_scale = existing.map_or(current_scale, |w| w.base_scale);
    commands.entity(entity).try_insert(ImpactWobble {
        base_scale,
        age: 0.0,
        duration: 0.28,
        strength: strength.clamp(0.03, 0.22),
    });
}

fn ease_out_back(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    const C1: f32 = 1.70158;
    const C3: f32 = C1 + 1.0;
    1.0 + C3 * (t - 1.0).powi(3) + C1 * (t - 1.0).powi(2)
}

fn spawn_collect_orbs(
    mut commands: Commands,
    queue: Res<CollectBurstQueue>,
    time: Res<Time>,
    active_orbs: Query<(), With<CollectOrb>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
    mut art: Local<Option<FxArt>>,
) {
    let requests = std::mem::take(&mut *queue.0.lock().unwrap_or_else(|e| e.into_inner()));
    if requests.is_empty() {
        return;
    }
    let (Some(mut meshes), Some(mut materials)) = (meshes, materials) else {
        return;
    };
    let art = art.get_or_insert_with(|| FxArt::new(&mut meshes, &mut materials));
    let mut live_count = active_orbs.iter().count();
    let seed_phase = time.elapsed_secs() * 5.3;

    for req in requests {
        // Burst ambiental no ponto de origem para leitura imediata de impacto/coleta.
        let spec = burst_spec(req.kind.burst_preset());
        spawn_burst(&mut commands, &spec, req.origin + Vec3::Y * 0.2, 10);

        let mesh = art.mesh_for(req.kind);
        let material = art.material_for(req.kind);
        let base_scale = req.kind.base_scale();
        let total = req.count.max(1);
        for i in 0..total {
            if live_count >= MAX_ACTIVE_ORBS {
                break;
            }
            live_count += 1;
            let frac = i as f32 / total as f32;
            let angle = seed_phase + frac * TAU + (req.origin.x * 1.7 + req.origin.z * 2.3);
            let radial = Vec3::new(angle.cos(), 0.0, angle.sin());
            let side = Vec3::new(-angle.sin(), 0.0, angle.cos());
            let speed_out = 1.45 + 0.85 * ((angle * 2.3).sin() * 0.5 + 0.5);
            let speed_up = 2.65 + 0.95 * ((angle * 1.7).cos() * 0.5 + 0.5);
            let swirl_sign = if i % 2 == 0 { 1.0 } else { -1.0 };
            let stagger = i as f32 * 0.048;

            commands.spawn((
                Name::new("fx:collect_orb"),
                SceneEntity,
                IgnoreOutline,
                NotShadowCaster,
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                Transform::from_translation(req.origin + radial * 0.14 + Vec3::Y * 0.18)
                    .with_rotation(Quat::from_euler(
                        EulerRot::YXZ,
                        angle,
                        0.5 * (i as f32),
                        0.3,
                    ))
                    .with_scale(base_scale * 0.2),
                CollectOrb {
                    kind: req.kind,
                    age: 0.0,
                    pop_duration: 0.24 + stagger,
                    fly_duration: 0.36 + (i % 3) as f32 * 0.04,
                    velocity: radial * speed_out + Vec3::Y * speed_up,
                    spin: Vec3::new(5.5 + i as f32, 8.2, 4.3 * swirl_sign),
                    base_scale,
                    fly_start: req.origin,
                    control_offset: side * (0.75 * swirl_sign) + Vec3::Y * (0.95 + 0.2 * (i % 3) as f32),
                    homing: false,
                    trail_timer: 0.02,
                    emit_absorb_burst: i == 0 || i + 1 == total,
                },
            ));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn drive_collect_orbs(
    mut commands: Commands,
    time: Res<Time>,
    players: Query<&GlobalTransform, With<Player>>,
    mut orbs: Query<(Entity, &mut Transform, &mut CollectOrb)>,
    motes: Query<(), With<TrailMote>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
    mut art: Local<Option<FxArt>>,
    mut pulse_cooldown: Local<f32>,
) {
    let dt = time.delta_secs().min(0.05);
    *pulse_cooldown = (*pulse_cooldown - dt).max(0.0);
    let hero_target = players
        .iter()
        .next()
        .map(|pose| pose.translation() + Vec3::Y * 1.02);
    let mut live_motes = motes.iter().count();
    let mut assets = meshes.zip(materials);

    for (entity, mut tf, mut orb) in &mut orbs {
        orb.age += dt;
        tf.rotate(Quat::from_scaled_axis(orb.spin * dt));

        if !orb.homing {
            // Fase 1: ejeção parabólica com escala elástica.
            orb.velocity.y -= 9.2 * dt;
            tf.translation += orb.velocity * dt;
            let pop_t = (orb.age / orb.pop_duration.max(0.01)).clamp(0.0, 1.0);
            let scale_k = 0.22 + 0.78 * ease_out_back((pop_t * 1.65).min(1.0));
            tf.scale = orb.base_scale * scale_k;

            if orb.age >= orb.pop_duration {
                orb.homing = true;
                orb.fly_start = tf.translation;
            }
            continue;
        }

        // Fase 2: voo em arco Bézier acelerado até ao peito do herói.
        let fly_age = orb.age - orb.pop_duration;
        let t = (fly_age / orb.fly_duration.max(0.01)).clamp(0.0, 1.0);
        let Some(target) = hero_target else {
            // Sem herói (troca de cena): desvanece no sítio.
            if t >= 1.0 {
                commands.entity(entity).try_despawn();
            }
            continue;
        };

        // Curva ease-in-out cúbica com arranque suave e aceleração magnética no fim.
        let u = t * t * (2.2 - 1.2 * t);
        let u = u.clamp(0.0, 1.0);
        let control = orb.fly_start.lerp(target, 0.45) + orb.control_offset;
        let omt = 1.0 - u;
        tf.translation =
            orb.fly_start * (omt * omt) + control * (2.0 * omt * u) + target * (u * u);

        // Leve esticamento e pulso antes da absorção.
        let shrink = if t < 0.75 {
            1.0 + 0.22 * (t * PI).sin()
        } else {
            let tail = (1.0 - t) / 0.25;
            (tail * 1.15).max(0.08)
        };
        tf.scale = orb.base_scale * shrink;

        // Rastro luminoso durante o arco.
        orb.trail_timer -= dt;
        if orb.trail_timer <= 0.0
            && live_motes < MAX_ACTIVE_MOTES
            && let Some((meshes, materials)) = assets.as_mut()
        {
            orb.trail_timer = 0.035;
            live_motes += 1;
            let art = art.get_or_insert_with(|| FxArt::new(meshes, materials));
            commands.spawn((
                Name::new("fx:collect_mote"),
                SceneEntity,
                IgnoreOutline,
                NotShadowCaster,
                Mesh3d(art.mote_mesh.clone()),
                MeshMaterial3d(art.material_for(orb.kind)),
                Transform::from_translation(tf.translation).with_scale(Vec3::splat(0.042)),
                TrailMote {
                    age: 0.0,
                    lifetime: 0.20,
                    start_scale: 0.042,
                    drift: Vec3::Y * 0.22,
                },
            ));
        }

        if t >= 1.0 {
            // Fase 3: absorção vibrante no herói!
            if orb.emit_absorb_burst {
                let spec = burst_spec("sparkle");
                spawn_burst(&mut commands, &spec, target, 8);
            }
            if *pulse_cooldown <= 0.0
                && let Some((meshes, materials)) = assets.as_mut()
            {
                *pulse_cooldown = ABSORB_PULSE_COOLDOWN;
                let art = art.get_or_insert_with(|| FxArt::new(meshes, materials));
                let (base, emissive) = orb.kind.palette();
                let ring_mat = materials.add(StandardMaterial {
                    base_color: base.with_alpha(0.85),
                    emissive: emissive * 1.25,
                    alpha_mode: AlphaMode::Add,
                    unlit: true,
                    cull_mode: None,
                    ..default()
                });
                commands.spawn((
                    Name::new("fx:absorb_pulse"),
                    SceneEntity,
                    IgnoreOutline,
                    NotShadowCaster,
                    Mesh3d(art.ring_mesh.clone()),
                    MeshMaterial3d(ring_mat.clone()),
                    Transform::from_translation(target).with_scale(Vec3::new(0.22, 0.08, 0.22)),
                    AbsorbPulse {
                        age: 0.0,
                        duration: 0.28,
                        start_scale: 0.22,
                        end_scale: 1.05,
                        material: ring_mat,
                        base_emissive: emissive * 1.25,
                    },
                ));
            }
            commands.entity(entity).try_despawn();
        }
    }
}

fn drive_trail_motes(
    mut commands: Commands,
    time: Res<Time>,
    mut motes: Query<(Entity, &mut Transform, &mut TrailMote)>,
) {
    let dt = time.delta_secs().min(0.05);
    for (entity, mut tf, mut mote) in &mut motes {
        mote.age += dt;
        if mote.age >= mote.lifetime {
            commands.entity(entity).try_despawn();
            continue;
        }
        let remain = 1.0 - (mote.age / mote.lifetime);
        tf.translation += mote.drift * dt;
        tf.scale = Vec3::splat((mote.start_scale * remain * remain).max(0.002));
    }
}

fn drive_absorb_pulses(
    mut commands: Commands,
    time: Res<Time>,
    players: Query<&GlobalTransform, With<Player>>,
    mut pulses: Query<(Entity, &mut Transform, &mut AbsorbPulse)>,
    mut materials: Option<ResMut<Assets<StandardMaterial>>>,
) {
    let dt = time.delta_secs().min(0.05);
    let hero_pos = players
        .iter()
        .next()
        .map(|pose| pose.translation() + Vec3::Y * 1.02);
    for (entity, mut tf, mut pulse) in &mut pulses {
        pulse.age += dt;
        if pulse.age >= pulse.duration {
            commands.entity(entity).try_despawn();
            continue;
        }
        let t = (pulse.age / pulse.duration).clamp(0.0, 1.0);
        let ease = 1.0 - (1.0 - t).powi(3);
        let r = pulse.start_scale.lerp(pulse.end_scale, ease);
        if let Some(pos) = hero_pos {
            tf.translation = pos + Vec3::Y * (ease * 0.25);
        }
        tf.rotate_y(dt * 4.5);
        tf.scale = Vec3::new(r, 0.08 * (1.0 - t * 0.6), r);
        if let Some(materials) = materials.as_deref_mut()
            && let Some(mut mat) = materials.get_mut(&pulse.material)
        {
            let fade = (1.0 - t).powi(2);
            mat.emissive = pulse.base_emissive * fade;
            mat.base_color.set_alpha(0.85 * fade);
        }
    }
}

fn drive_impact_wobbles(
    mut commands: Commands,
    time: Res<Time>,
    mut entities: Query<(Entity, &mut Transform, &mut ImpactWobble)>,
) {
    let dt = time.delta_secs().min(0.05);
    for (entity, mut tf, mut wobble) in &mut entities {
        wobble.age += dt;
        if wobble.age >= wobble.duration {
            tf.scale = wobble.base_scale;
            commands.entity(entity).try_remove::<ImpactWobble>();
            continue;
        }
        let t = wobble.age;
        let osc = (-12.5 * t).exp() * (38.0 * t).sin() * wobble.strength;
        tf.scale = Vec3::new(
            wobble.base_scale.x * (1.0 + osc),
            wobble.base_scale.y * (1.0 - osc * 1.15),
            wobble.base_scale.z * (1.0 + osc),
        );
    }
}

/// Anima as faíscas orbitais e os anéis de chão dos itens largados no mundo.
fn animate_pickup_details(
    time: Res<Time>,
    mut glints: Query<(&PickupOrbitGlint, &mut Transform), Without<PickupGroundRing>>,
    mut rings: Query<(&PickupGroundRing, &ChildOf, &mut Transform), Without<PickupOrbitGlint>>,
    parents: Query<&Transform, (Without<PickupOrbitGlint>, Without<PickupGroundRing>)>,
) {
    let now = time.elapsed_secs();
    for (glint, mut tf) in &mut glints {
        let a = now * glint.speed + glint.phase;
        let r = glint.radius * (0.88 + 0.14 * (a * 1.7).sin());
        tf.translation = Vec3::new(
            a.cos() * r,
            glint.base_y + (a * 1.4).sin() * glint.bob_amp,
            a.sin() * r,
        );
        let pulse = 0.72 + 0.45 * ((now * 5.2 + glint.phase).sin() * 0.5 + 0.5);
        tf.scale = Vec3::splat(pulse);
    }
    for (ring, parent, mut tf) in &mut rings {
        let Ok(root_tf) = parents.get(parent.parent()) else {
            continue;
        };
        // Mantém o anel encostado ao chão mesmo enquanto o pickup salta/levita acima.
        let local_y = (ring.floor_y + 0.045 - root_tf.translation.y)
            / root_tf.scale.y.abs().max(0.05);
        tf.translation = Vec3::new(0.0, local_y, 0.0);
        let breath = 0.92 + 0.14 * ((now * 3.4 + ring.phase).sin() * 0.5 + 0.5);
        // Contrarrota o yaw do pai para o anel girar suavemente no seu ritmo próprio.
        tf.rotation = root_tf.rotation.inverse() * Quat::from_rotation_y(now * 1.2 + ring.phase);
        tf.scale = Vec3::new(0.32 * breath, 0.03, 0.32 * breath);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bevy::time::TimeUpdateStrategy;

    use super::*;

    #[test]
    fn resource_kinds_map_to_expected_visual_families() {
        assert_eq!(
            CollectVisualKind::from_resource_kind("wood"),
            CollectVisualKind::Wood
        );
        assert_eq!(
            CollectVisualKind::from_resource_kind("dark-wood"),
            CollectVisualKind::DarkWood
        );
        assert_eq!(
            CollectVisualKind::from_resource_kind("stone"),
            CollectVisualKind::Stone
        );
        assert_eq!(
            CollectVisualKind::from_resource_kind("crystal"),
            CollectVisualKind::Crystal
        );
        assert_eq!(
            CollectVisualKind::from_resource_kind("berry"),
            CollectVisualKind::Berry
        );
        assert_eq!(
            CollectVisualKind::from_resource_kind("bog-moss"),
            CollectVisualKind::Herb
        );
        assert_eq!(
            CollectVisualKind::from_resource_kind("gold"),
            CollectVisualKind::Gold
        );
        assert_eq!(
            CollectVisualKind::from_resource_kind("life"),
            CollectVisualKind::Life
        );
        assert_eq!(
            CollectVisualKind::from_resource_kind("desconhecido"),
            CollectVisualKind::Generic
        );
    }

    #[test]
    fn impact_wobble_deforms_and_restores_exact_base_scale() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
                20,
            )))
            .add_systems(Update, drive_impact_wobbles);
        let base = Vec3::new(1.25, 1.5, 0.9);
        let entity = app
            .world_mut()
            .spawn((
                Transform::from_scale(base),
                ImpactWobble {
                    base_scale: base,
                    age: 0.0,
                    duration: 0.28,
                    strength: 0.14,
                },
            ))
            .id();
        // Primeiro frame (startup) + segundo frame (dt = 20 ms): deve deformar.
        app.update();
        app.update();
        let mid = app.world().get::<Transform>(entity).unwrap().scale;
        assert!(
            mid.distance(base) > 0.01,
            "wobble devia deformar a escala no início: {mid:?}"
        );
        for _ in 0..20 {
            app.update();
        }
        let restored = app.world().get::<Transform>(entity).unwrap().scale;
        assert!(
            restored.abs_diff_eq(base, 1e-5),
            "escala final devia voltar exatamente a {base:?}, ficou {restored:?}"
        );
        assert!(app.world().get::<ImpactWobble>(entity).is_none());
    }

    #[test]
    fn collect_orb_pops_flies_to_hero_and_despawns_with_absorb_pulse() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
                25,
            )))
            .init_resource::<CollectBurstQueue>()
            .add_systems(Update, (spawn_collect_orbs, drive_collect_orbs).chain());

        let hero_pos = Vec3::new(4.0, 1.0, 2.0);
        app.world_mut().spawn((
            Player::default(),
            Transform::from_translation(hero_pos),
            GlobalTransform::from_translation(hero_pos),
        ));
        app.world()
            .resource::<CollectBurstQueue>()
            .push(CollectVisualKind::Gold, Vec3::new(1.0, 1.0, 2.0), 3);

        app.update();
        let spawned = app
            .world_mut()
            .query::<&CollectOrb>()
            .iter(app.world())
            .count();
        assert_eq!(spawned, 3, "devia spawnar 3 orbes de ouro");

        for _ in 0..45 {
            app.update();
        }
        let remaining = app
            .world_mut()
            .query::<&CollectOrb>()
            .iter(app.world())
            .count();
        assert_eq!(remaining, 0, "todos os orbes deviam chegar e despawnar");
        let pulses = app
            .world_mut()
            .query::<&AbsorbPulse>()
            .iter(app.world())
            .count();
        assert!(pulses >= 1, "absorção devia criar anel luminoso");
    }
}

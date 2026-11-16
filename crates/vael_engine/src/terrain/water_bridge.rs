//! Adaptadores entre a crate `bevy_ext_water` e o mundo do Vael.
//!
//! A crate só depende de bevy (+ avian). Tudo o que é ESPECÍFICO deste jogo
//! fica aqui (o `impl WaterSource for TerrainRuntime` vive no próprio crate
//! de terreno — regra de orphan: nem o trait nem o tipo são desta engine):
//!
//! - ambiente (`WeatherState`/`AtmosphereState` → `WaterEnvironment`);
//! - foco da janela (posição do herói → `WaterFocus`);
//! - obstáculos de corrente (colliders avian + sólidos voxel →
//!   `WaterObstacles`, só quando algo muda);
//! - partículas (`WaterSplashEvent` → `spawn_burst`) e o marcador
//!   `WaterBodied` no herói/criaturas.

use bevy::prelude::*;
use avian3d::prelude::Collider;

use bevy_ext_water::obstacles::obstacles_signature;
use bevy_ext_water::{
    SplashPreset, WaterBodied, WaterEnvironment, WaterFocus, WaterObstacles, WaterSplashEvent,
};

use super::runtime::TerrainRuntime;
use super::water_material::WaterConfigResource;
use super::water_obstacles::{
    OBSTACLE_HALO_M, OBSTACLE_SLICE_ABOVE_M, OBSTACLE_SLICE_BELOW_M, WaterObstacle,
    collect_collider_water_obstacles, collect_voxel_water_obstacles, rotated_aabb,
    water_surface_range_in_aabb,
};
use super::water_waves::{DOMAIN_SNAP_M, WAVE_WORLD_SIZE, WaterWaveWorld};
use crate::particles::spawn_burst;
use crate::recipes::ParticleSpec;
use crate::worldsys::{AtmosphereState, WeatherState};

/// Colliders que são paredes/pilares para a corrente (não o chão voxel, não
/// os npcs/herói/corpos de ragdoll).
pub type WaterObstacleFilter = (
    Without<crate::physics::VoxelCollider>,
    Without<crate::ai::AiLocomotion>,
    Without<crate::player::Player>,
    Without<avian3d::prelude::Sensor>,
    Without<crate::ragdoll::RagdollBodyOf>,
);

/// Liga os recursos/eventos da water crate ao mundo do Vael.
pub struct WaterBridgePlugin;

impl Plugin for WaterBridgePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WaterFocus>()
            .init_resource::<WaterObstacles>()
            .init_resource::<WaterEnvironment>()
            .add_systems(
            Update,
            (
                sync_water_focus,
                sync_water_environment,
                sync_water_obstacles,
                tag_water_bodied,
                spawn_water_splashes,
            ),
        );
    }
}

fn sync_water_focus(
    players: Query<&Transform, With<crate::player::Player>>,
    mut focus: ResMut<WaterFocus>,
) {
    if let Some(t) = players.iter().next() {
        focus.set(t.translation.x, t.translation.z);
    }
}

fn sync_water_environment(
    weather: Option<Res<WeatherState>>,
    atmo: Option<Res<AtmosphereState>>,
    cfg: Option<Res<WaterConfigResource>>,
    time: Res<Time>,
    mut env: ResMut<WaterEnvironment>,
) {
    if let Some(w) = weather.as_deref() {
        env.wind = w.wind;
        env.wind_strength = w.wind_strength;
        env.rain = w.rain.clamp(0.0, 1.0);
    } else if let Some(c) = cfg.as_deref() {
        env.wind = c.0.wind;
        env.wind_strength = c.0.wind_strength;
    }
    if let Some(c) = cfg.as_deref() {
        env.wave_amp = c.0.wave_amp;
    }
    if let Some(a) = atmo.as_deref() {
        env.sun_dir = if a.day >= 0.35 {
            a.sun_dir.normalize_or(Vec3::Y)
        } else {
            a.moon_dir.normalize_or(Vec3::Y)
        };
        env.day = a.day;
        env.sun_tint = a.sun_tint;
        env.zenith = a.zenith;
        env.horizon = a.horizon;
    }
    env.time = time.elapsed_secs();
}

/// Círculo que contém a zona carvada de um corpo (`contains` nunca é
/// verdade fora dele). Rio sem casco calculado = alcance infinito.
fn water_body_reach(body: &bevy_ext_water::WaterBody) -> (Vec2, f32) {
    let r = match body.kind {
        bevy_ext_water::WaterKind::Lake => body.carve_radius,
        bevy_ext_water::WaterKind::River if body.hull > 0.0 => body.hull + body.carve_radius,
        bevy_ext_water::WaterKind::River => f32::INFINITY,
    };
    (body.at, r)
}

fn rect_touches_circle(min: Vec2, max: Vec2, center: Vec2, radius: f32) -> bool {
    if !radius.is_finite() {
        return true;
    }
    center.clamp(min, max).distance_squared(center) <= radius * radius
}

/// Intervalo mínimo entre recolhas causadas só por colliders a mexer.
const COLLIDER_REFRESH_S: f32 = 0.125;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct VoxelObstacleKey {
    revision: u64,
    voxel: usize,
    water_bodies: usize,
}

struct VoxelObstacleCache {
    key: VoxelObstacleKey,
    tiles: bevy::platform::collections::HashMap<IVec2, Vec<WaterObstacle>>,
}

#[derive(Default)]
struct ColliderObstacleCache {
    generation: u32,
    map: bevy::platform::collections::HashMap<Entity, CachedCollider>,
}

struct CachedCollider {
    generation: u32,
    translation: Vec3,
    rotation: Quat,
    /// Identidade da forma (`SharedShape` partilhado): troca de collider
    /// com a mesma pose também invalida.
    shape: usize,
    wmin: Vec3,
    wmax: Vec3,
    obstacles: Vec<WaterObstacle>,
}

/// Lado (m) de um tile do cache de obstáculos voxel. Inteiro: as amostras
/// caem nas coordenadas inteiras da janela (`DOMAIN_SNAP_M = 1`).
const VOXEL_TILE_M: f32 = 32.0;

/// Recolhe os obstáculos que cortam a lâmina dentro da janela de 128 m.
///
/// Só corre quando um collider muda (added/moved/removed) ou a janela anda:
/// a conversão de trimesh/compound é cara e a lista é estável a maior parte
/// dos frames.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn sync_water_obstacles(
    terrain: Option<Res<TerrainRuntime>>,
    waves: Option<Res<WaterWaveWorld>>,
    obstacle_q: Query<(Entity, &GlobalTransform, &Collider), WaterObstacleFilter>,
    changed_obs: Query<
        (),
        (
            With<Collider>,
            Or<(Added<Collider>, Changed<GlobalTransform>, Changed<Collider>)>,
        ),
    >,
    mut removed_obs: RemovedComponents<Collider>,
    mut obstacles: ResMut<WaterObstacles>,
    mut last_origin: Local<Option<IVec2>>,
    mut voxel_cache: Local<Option<VoxelObstacleCache>>,
    mut collider_cache: Local<ColliderObstacleCache>,
    mut pending: Local<bool>,
    mut last_run: Local<f32>,
    time: Res<Time>,
) {
    let (Some(rt), Some(waves)) = (terrain, waves) else {
        return;
    };
    let window_moved = last_origin.is_none_or(|o| o != waves.origin_cell_1m);
    // Um collider qualquer a mexer-se (armas, props dinâmicos, hitboxes) fazia
    // isto correr TODOS os frames — e cada corrida refazia o scan voxel de
    // 129×129 amostras da janela (~18 ms/frame medido a 2026-11-15). As
    // mudanças de collider acumulam e aplicam-se no máximo a ~8 Hz; a janela a
    // andar continua imediata.
    if !changed_obs.is_empty() || removed_obs.read().count() > 0 {
        *pending = true;
    }
    let now = time.elapsed_secs();
    let throttled = now - *last_run < COLLIDER_REFRESH_S;
    if !window_moved && !(*pending && !throttled) {
        return;
    }
    *pending = false;
    *last_run = now;
    *last_origin = Some(waves.origin_cell_1m);

    let mut found: Vec<WaterObstacle> = Vec::new();
    if !rt.water.is_empty() {
        let ox = waves.origin_cell_1m.x as f32 * DOMAIN_SNAP_M;
        let oz = waves.origin_cell_1m.y as f32 * DOMAIN_SNAP_M;
        let pad = OBSTACLE_HALO_M + 14.0;
        let win_min = Vec2::new(ox - pad, oz - pad);
        let win_max = Vec2::new(ox + WAVE_WORLD_SIZE + pad, oz + WAVE_WORLD_SIZE + pad);

        // Corpos de água cujo alcance toca a janela: sem nenhum, não há lâmina
        // a cortar e nem os colliders nem o voxel precisam de ser vistos.
        let reaches: Vec<(Vec2, f32)> = rt
            .water
            .iter()
            .map(water_body_reach)
            .filter(|&(c, r)| rect_touches_circle(win_min, win_max, c, r))
            .collect();
        if reaches.is_empty() {
            if !obstacles.obstacles.is_empty() {
                obstacles.set(Vec::new());
            }
            return;
        }
        // Por collider, o resultado (AABB de mundo + cortes na lâmina) só
        // depende da pose e da forma: cache por entidade, refeito só para os
        // que mexeram. Antes cada recolha refazia `compute_local_aabb` (O(nº
        // vértices) nos convexos) e ~36 amostras de água por collider da
        // janela — a ~8 Hz, ~6 ms por recolha ao pé do rio.
        let all_reaches: Vec<(Vec2, f32)> = rt.water.iter().map(water_body_reach).collect();
        collider_cache.generation = collider_cache.generation.wrapping_add(1);
        let generation = collider_cache.generation;
        for (entity, global, collider) in &obstacle_q {
            let placement = global.compute_transform();
            let shape = std::sync::Arc::as_ptr(&collider.shape_scaled().0) as *const () as usize;
            let entry = collider_cache.map.entry(entity).or_insert_with(|| CachedCollider {
                generation,
                translation: Vec3::NAN,
                rotation: Quat::IDENTITY,
                shape: 0,
                wmin: Vec3::ZERO,
                wmax: Vec3::ZERO,
                obstacles: Vec::new(),
            });
            entry.generation = generation;
            if entry.translation != placement.translation
                || entry.rotation != placement.rotation
                || entry.shape != shape
            {
                let iso = Transform {
                    translation: placement.translation,
                    rotation: placement.rotation,
                    scale: Vec3::ONE,
                };
                let local = collider.shape_scaled().compute_local_aabb();
                let local_min = Vec3::new(local.mins.x, local.mins.y, local.mins.z);
                let local_max = Vec3::new(local.maxs.x, local.maxs.y, local.maxs.z);
                let (wmin, wmax) = rotated_aabb(&iso, local_min, local_max);
                entry.translation = placement.translation;
                entry.rotation = placement.rotation;
                entry.shape = shape;
                entry.wmin = wmin;
                entry.wmax = wmax;
                entry.obstacles.clear();
                let wet = all_reaches
                    .iter()
                    .any(|&(c, r)| rect_touches_circle(wmin.xz(), wmax.xz(), c, r + 0.5));
                if wet
                    && let Some((wy_min, wy_max)) =
                        water_surface_range_in_aabb(&rt, wmin.xz(), wmax.xz())
                    && wmax.y >= wy_min - OBSTACLE_SLICE_BELOW_M
                    && wmin.y <= wy_max + OBSTACLE_SLICE_ABOVE_M
                {
                    collect_collider_water_obstacles(
                        collider,
                        iso,
                        wy_min,
                        wy_max,
                        &mut entry.obstacles,
                    );
                }
            }
            let (wmin, wmax) = (entry.wmin, entry.wmax);
            if wmax.x < win_min.x || wmin.x > win_max.x || wmax.z < win_min.y || wmin.z > win_max.y
            {
                continue;
            }
            found.extend(entry.obstacles.iter().cloned());
        }
        collider_cache.map.retain(|_, entry| entry.generation == generation);

        // Os sólidos voxel só dependem do terreno: cache em tiles de
        // `VOXEL_TILE_M` (amostras nas mesmas coordenadas inteiras do scan da
        // janela), invalidado pela revisão das edições/campo voxel. A janela
        // anda 1 m de cada vez; sem isto cada metro andado refazia ~16k
        // amostras.
        let key = VoxelObstacleKey {
            revision: rt.deltas.revision(),
            voxel: std::sync::Arc::as_ptr(&rt.voxel) as usize,
            water_bodies: rt.water.len(),
        };
        let cache = voxel_cache.get_or_insert_with(|| VoxelObstacleCache {
            key,
            tiles: Default::default(),
        });
        if cache.key != key {
            cache.key = key;
            cache.tiles.clear();
        }
        let lo = Vec2::new(ox, oz);
        let hi = Vec2::new(ox + WAVE_WORLD_SIZE, oz + WAVE_WORLD_SIZE);
        let t0 = (lo / VOXEL_TILE_M).floor().as_ivec2();
        let t1 = (hi / VOXEL_TILE_M).floor().as_ivec2();
        for tz in t0.y..=t1.y {
            for tx in t0.x..=t1.x {
                let tile = cache.tiles.entry(IVec2::new(tx, tz)).or_insert_with(|| {
                    let min = Vec2::new(tx as f32, tz as f32) * VOXEL_TILE_M;
                    // `collect` amostra [min, max] inclusive: o último ponto
                    // pertence ao tile seguinte.
                    let max = min + Vec2::splat(VOXEL_TILE_M - 1.0);
                    let mut out = Vec::new();
                    collect_voxel_water_obstacles(&rt, min, max, &mut out);
                    out
                });
                found.extend(tile.iter().filter(|o| {
                    let c = o.center;
                    c.x >= lo.x && c.x <= hi.x && c.y >= lo.y && c.y <= hi.y
                }).cloned());
            }
        }
    }

    if obstacles_signature(&found) != obstacles_signature(&obstacles.obstacles) {
        obstacles.set(found);
    }
}

/// Herói e criaturas salpicam; barcos já chegam pelos `WaterKinematics`.
fn tag_water_bodied(
    mut commands: Commands,
    hero: Query<
        Entity,
        (
            With<crate::player::Player>,
            Without<WaterBodied>,
            With<Transform>,
        ),
    >,
    creatures: Query<
        Entity,
        (
            With<crate::ai::EnemyCreature>,
            Without<WaterBodied>,
            With<Transform>,
        ),
    >,
) {
    for entity in hero.iter().chain(creatures.iter()) {
        commands.entity(entity).insert(WaterBodied);
    }
}

fn burst_spec(preset: &str) -> ParticleSpec {
    ParticleSpec {
        preset: preset.to_string(),
        emission_rate: None,
        life: None,
        speed: None,
        size: None,
        color: None,
        shape_radius: None,
        looping: false,
        world_space: false,
    }
}

/// Traduz os pedidos de partículas da crate para o `spawn_burst` do Vael.
fn spawn_water_splashes(
    mut commands: Commands,
    mut splashes: Option<MessageReader<WaterSplashEvent>>,
) {
    let Some(splashes) = splashes.as_mut() else {
        return;
    };
    for ev in splashes.read() {
        let preset = match ev.preset {
            SplashPreset::Splash => "splash",
            SplashPreset::Wade => "wade",
            SplashPreset::Foam => "foam",
            SplashPreset::Mist => "mist",
            SplashPreset::RainRipple => "rain_ripple",
        };
        let spec = ParticleSpec {
            life: ev.life,
            speed: ev.speed,
            size: ev.size,
            shape_radius: ev.shape_radius,
            ..burst_spec(preset)
        };
        spawn_burst(&mut commands, &spec, ev.at, ev.count);
    }
}

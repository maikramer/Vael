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

use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use avian3d::prelude::Collider;

use bevy_ext_water::obstacles::obstacles_signature;
use bevy_ext_water::waves::bodies_signature;
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
    Without<bevy_ext_water::buoyancy::avian::AvianFloating>,
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
    seeded: bool,
    water_key: Option<(u64, usize, usize)>,
    all_reaches: Vec<(Vec2, f32)>,
    reaches_scratch: Vec<(Vec2, f32)>,
    seen_gen: u32,
    map: HashMap<Entity, CachedCollider>,
    /// Apenas colliders que realmente cortam alguma lâmina (`!obstacles.is_empty()`),
    /// indexados por tiles de `VOXEL_TILE_M`. Evita varrer todos os colliders secos do mundo.
    wet_cells: HashMap<IVec2, Vec<Entity>>,
    dirty: HashSet<Entity>,
    removed: HashSet<Entity>,
}

impl ColliderObstacleCache {
    fn reset_for_water(&mut self, key: (u64, usize, usize), water: &[bevy_ext_water::WaterBody]) {
        self.seeded = false;
        self.water_key = Some(key);
        self.all_reaches.clear();
        self.all_reaches.extend(water.iter().map(water_body_reach));
        self.map.clear();
        self.wet_cells.clear();
        self.dirty.clear();
        self.removed.clear();
    }
}

struct CachedCollider {
    seen_gen: u32,
    translation: Vec3,
    rotation: Quat,
    /// Identidade da forma (`SharedShape` partilhado): troca de collider
    /// com a mesma pose também invalida.
    shape: usize,
    wmin: Vec3,
    wmax: Vec3,
    cell_min: IVec2,
    cell_max: IVec2,
    obstacles: Vec<WaterObstacle>,
}

fn aabb_cell_bounds(wmin: Vec3, wmax: Vec3) -> (IVec2, IVec2) {
    let c0 = (wmin.xz() / VOXEL_TILE_M).floor().as_ivec2();
    let c1 = (wmax.xz() / VOXEL_TILE_M).floor().as_ivec2();
    (c0, c1)
}

fn wet_cells_remove(
    wet_cells: &mut HashMap<IVec2, Vec<Entity>>,
    entity: Entity,
    c0: IVec2,
    c1: IVec2,
) {
    for tz in c0.y..=c1.y {
        for tx in c0.x..=c1.x {
            let key = IVec2::new(tx, tz);
            if let Some(list) = wet_cells.get_mut(&key) {
                if let Some(pos) = list.iter().position(|&e| e == entity) {
                    list.swap_remove(pos);
                }
                if list.is_empty() {
                    wet_cells.remove(&key);
                }
            }
        }
    }
}

fn wet_cells_insert(
    wet_cells: &mut HashMap<IVec2, Vec<Entity>>,
    entity: Entity,
    c0: IVec2,
    c1: IVec2,
) {
    for tz in c0.y..=c1.y {
        for tx in c0.x..=c1.x {
            wet_cells.entry(IVec2::new(tx, tz)).or_default().push(entity);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn compute_collider_entry(
    rt: &TerrainRuntime,
    all_reaches: &[(Vec2, f32)],
    global: &GlobalTransform,
    collider: &Collider,
) -> (Vec3, Quat, usize, Vec3, Vec3, Vec<WaterObstacle>) {
    let placement = global.compute_transform();
    let shape = std::sync::Arc::as_ptr(&collider.shape_scaled().0) as *const () as usize;
    let iso = Transform {
        translation: placement.translation,
        rotation: placement.rotation,
        scale: Vec3::ONE,
    };
    let local = collider.shape_scaled().compute_local_aabb();
    let local_min = Vec3::new(local.mins.x, local.mins.y, local.mins.z);
    let local_max = Vec3::new(local.maxs.x, local.maxs.y, local.maxs.z);
    let (wmin, wmax) = rotated_aabb(&iso, local_min, local_max);
    let mut entry_obstacles = Vec::new();
    let wet = all_reaches
        .iter()
        .any(|&(c, r)| rect_touches_circle(wmin.xz(), wmax.xz(), c, r + 0.5));
    if wet
        && let Some((wy_min, wy_max)) = water_surface_range_in_aabb(rt, wmin.xz(), wmax.xz())
        && wmax.y >= wy_min - OBSTACLE_SLICE_BELOW_M
        && wmin.y <= wy_max + OBSTACLE_SLICE_ABOVE_M
    {
        collect_collider_water_obstacles(collider, iso, wy_min, wy_max, &mut entry_obstacles);
    }
    (
        placement.translation,
        placement.rotation,
        shape,
        wmin,
        wmax,
        entry_obstacles,
    )
}

/// Lado (m) de um tile do cache de obstáculos voxel. Inteiro: as amostras
/// caem nas coordenadas inteiras da janela (`DOMAIN_SNAP_M = 1`).
const VOXEL_TILE_M: f32 = 32.0;

/// Recolhe os obstáculos que cortam a lâmina dentro da janela de 128 m.
///
/// Só corre quando um collider de obstáculo muda (added/moved/removed) ou a
/// janela anda: mantém um cache incremental + grelha espacial dos colliders que
/// tocam água para evitar varrer os milhares de colliders secos do mundo.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn sync_water_obstacles(
    terrain: Option<Res<TerrainRuntime>>,
    waves: Option<Res<WaterWaveWorld>>,
    obstacle_q: Query<(Entity, &GlobalTransform, &Collider), WaterObstacleFilter>,
    changed_obs: Query<
        Entity,
        (
            With<Collider>,
            WaterObstacleFilter,
            Or<(Added<Collider>, Changed<GlobalTransform>, Changed<Collider>)>,
        ),
    >,
    mut removed_obs: RemovedComponents<Collider>,
    mut obstacles: ResMut<WaterObstacles>,
    mut last_origin: Local<Option<IVec2>>,
    mut voxel_cache: Local<Option<VoxelObstacleCache>>,
    mut collider_cache: Local<ColliderObstacleCache>,
    mut found_scratch: Local<Vec<WaterObstacle>>,
    mut pending: Local<bool>,
    mut last_run: Local<f32>,
    time: Res<Time>,
) {
    let (Some(rt), Some(waves)) = (terrain, waves) else {
        return;
    };
    let window_moved = last_origin.is_none_or(|o| o != waves.origin_cell_1m);

    if rt.water.is_empty() {
        if collider_cache.seeded {
            collider_cache.map.clear();
            collider_cache.wet_cells.clear();
            collider_cache.dirty.clear();
            collider_cache.removed.clear();
            collider_cache.seeded = false;
            collider_cache.water_key = None;
        }
        let _ = removed_obs.read().count();
        *last_origin = Some(waves.origin_cell_1m);
        *pending = false;
        if !obstacles.obstacles.is_empty() {
            obstacles.set(Vec::new());
        }
        return;
    }

    let water_key = (
        bodies_signature(&rt.water),
        rt.water.len(),
        rt.water.as_ptr() as usize,
    );
    if collider_cache.water_key != Some(water_key) {
        collider_cache.reset_for_water(water_key, &rt.water);
        *pending = true;
    }

    // Regista mudanças antes do throttle (Bevy consome `Changed`/`RemovedComponents`
    // no fim do sistema mesmo quando há early return).
    if collider_cache.seeded {
        for entity in &changed_obs {
            collider_cache.removed.remove(&entity);
            collider_cache.dirty.insert(entity);
            *pending = true;
        }
        for entity in removed_obs.read() {
            collider_cache.dirty.remove(&entity);
            if collider_cache.map.contains_key(&entity) {
                collider_cache.removed.insert(entity);
                *pending = true;
            }
        }
    } else {
        let _ = removed_obs.read().count();
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

    let ox = waves.origin_cell_1m.x as f32 * DOMAIN_SNAP_M;
    let oz = waves.origin_cell_1m.y as f32 * DOMAIN_SNAP_M;
    let pad = OBSTACLE_HALO_M + 14.0;
    let win_min = Vec2::new(ox - pad, oz - pad);
    let win_max = Vec2::new(ox + WAVE_WORLD_SIZE + pad, oz + WAVE_WORLD_SIZE + pad);

    // Corpos de água cujo alcance toca a janela: sem nenhum, não há lâmina
    // a cortar e nem os colliders nem o voxel precisam de ser vistos.
    let cache = &mut *collider_cache;
    cache.reaches_scratch.clear();
    cache.reaches_scratch.extend(
        cache
            .all_reaches
            .iter()
            .copied()
            .filter(|&(c, r)| rect_touches_circle(win_min, win_max, c, r)),
    );
    if cache.reaches_scratch.is_empty() {
        if !obstacles.obstacles.is_empty() {
            obstacles.set(Vec::new());
        }
        return;
    }

    let mut any_wet_changed = false;
    if !cache.seeded {
        cache.seeded = true;
        cache.dirty.clear();
        cache.removed.clear();
        for (entity, global, collider) in &obstacle_q {
            let (translation, rotation, shape, wmin, wmax, entry_obstacles) =
                compute_collider_entry(&rt, &cache.all_reaches, global, collider);
            let (cell_min, cell_max) = if entry_obstacles.is_empty() {
                (IVec2::ZERO, IVec2::ZERO)
            } else {
                any_wet_changed = true;
                let (c0, c1) = aabb_cell_bounds(wmin, wmax);
                wet_cells_insert(&mut cache.wet_cells, entity, c0, c1);
                (c0, c1)
            };
            cache.map.insert(
                entity,
                CachedCollider {
                    seen_gen: 0,
                    translation,
                    rotation,
                    shape,
                    wmin,
                    wmax,
                    cell_min,
                    cell_max,
                    obstacles: entry_obstacles,
                },
            );
        }
        any_wet_changed = true;
    } else {
        for entity in cache.removed.drain() {
            if let Some(old) = cache.map.remove(&entity)
                && !old.obstacles.is_empty()
            {
                wet_cells_remove(&mut cache.wet_cells, entity, old.cell_min, old.cell_max);
                any_wet_changed = true;
            }
        }
        for entity in cache.dirty.drain() {
            let Ok((_, global, collider)) = obstacle_q.get(entity) else {
                if let Some(old) = cache.map.remove(&entity)
                    && !old.obstacles.is_empty()
                {
                    wet_cells_remove(&mut cache.wet_cells, entity, old.cell_min, old.cell_max);
                    any_wet_changed = true;
                }
                continue;
            };
            let placement = global.compute_transform();
            let shape = std::sync::Arc::as_ptr(&collider.shape_scaled().0) as *const () as usize;
            if let Some(existing) = cache.map.get(&entity)
                && existing.translation == placement.translation
                && existing.rotation == placement.rotation
                && existing.shape == shape
            {
                continue;
            }
            let (translation, rotation, shape, wmin, wmax, entry_obstacles) =
                compute_collider_entry(&rt, &cache.all_reaches, global, collider);
            if let Some(old) = cache.map.get(&entity)
                && !old.obstacles.is_empty()
            {
                wet_cells_remove(&mut cache.wet_cells, entity, old.cell_min, old.cell_max);
                any_wet_changed = true;
            }
            let (cell_min, cell_max) = if entry_obstacles.is_empty() {
                (IVec2::ZERO, IVec2::ZERO)
            } else {
                any_wet_changed = true;
                let (c0, c1) = aabb_cell_bounds(wmin, wmax);
                wet_cells_insert(&mut cache.wet_cells, entity, c0, c1);
                (c0, c1)
            };
            cache.map.insert(
                entity,
                CachedCollider {
                    seen_gen: 0,
                    translation,
                    rotation,
                    shape,
                    wmin,
                    wmax,
                    cell_min,
                    cell_max,
                    obstacles: entry_obstacles,
                },
            );
        }
    }

    let key = VoxelObstacleKey {
        revision: rt.deltas.revision(),
        voxel: std::sync::Arc::as_ptr(&rt.voxel) as usize,
        water_bodies: rt.water.len(),
    };
    let vcache = voxel_cache.get_or_insert_with(|| VoxelObstacleCache {
        key,
        tiles: Default::default(),
    });
    let voxel_key_changed = vcache.key != key;
    if voxel_key_changed {
        vcache.key = key;
        vcache.tiles.clear();
    }

    if !window_moved && !any_wet_changed && !voxel_key_changed {
        return;
    }

    let found = &mut *found_scratch;
    found.clear();

    // Só visita colliders molhados cujos tiles tocam `[win_min, win_max]`.
    cache.seen_gen = cache.seen_gen.wrapping_add(1);
    let seen_gen = cache.seen_gen;
    let wt0 = (win_min / VOXEL_TILE_M).floor().as_ivec2();
    let wt1 = (win_max / VOXEL_TILE_M).floor().as_ivec2();
    let mut stale_entities: Vec<Entity> = Vec::new();
    for tz in wt0.y..=wt1.y {
        for tx in wt0.x..=wt1.x {
            let Some(entities) = cache.wet_cells.get(&IVec2::new(tx, tz)) else {
                continue;
            };
            for &entity in entities {
                let Some(entry) = cache.map.get_mut(&entity) else {
                    continue;
                };
                if entry.seen_gen == seen_gen {
                    continue;
                }
                entry.seen_gen = seen_gen;
                if !obstacle_q.contains(entity) {
                    stale_entities.push(entity);
                    continue;
                }
                let (wmin, wmax) = (entry.wmin, entry.wmax);
                if wmax.x < win_min.x || wmin.x > win_max.x || wmax.z < win_min.y || wmin.z > win_max.y {
                    continue;
                }
                found.extend(entry.obstacles.iter().cloned());
            }
        }
    }
    for entity in stale_entities {
        if let Some(old) = cache.map.remove(&entity)
            && !old.obstacles.is_empty()
        {
            wet_cells_remove(&mut cache.wet_cells, entity, old.cell_min, old.cell_max);
        }
    }

    // Os sólidos voxel só dependem do terreno: cache em tiles de `VOXEL_TILE_M`.
    let lo = Vec2::new(ox, oz);
    let hi = Vec2::new(ox + WAVE_WORLD_SIZE, oz + WAVE_WORLD_SIZE);
    let t0 = (lo / VOXEL_TILE_M).floor().as_ivec2();
    let t1 = (hi / VOXEL_TILE_M).floor().as_ivec2();
    for tz in t0.y..=t1.y {
        for tx in t0.x..=t1.x {
            let tile = vcache.tiles.entry(IVec2::new(tx, tz)).or_insert_with(|| {
                let min = Vec2::new(tx as f32, tz as f32) * VOXEL_TILE_M;
                // `collect` amostra [min, max] inclusive: o último ponto
                // pertence ao tile seguinte.
                let max = min + Vec2::splat(VOXEL_TILE_M - 1.0);
                let mut out = Vec::new();
                collect_voxel_water_obstacles(&rt, min, max, &mut out);
                out
            });
            found.extend(
                tile.iter()
                    .filter(|o| {
                        let c = o.center;
                        c.x >= lo.x && c.x <= hi.x && c.y >= lo.y && c.y <= hi.y
                    })
                    .cloned(),
            );
        }
    }

    if obstacles_signature(found) != obstacles_signature(&obstacles.obstacles) {
        obstacles.set(found.clone());
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

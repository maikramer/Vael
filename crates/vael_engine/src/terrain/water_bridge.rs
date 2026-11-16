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

/// Recolhe os obstáculos que cortam a lâmina dentro da janela de 128 m.
///
/// Só corre quando um collider muda (added/moved/removed) ou a janela anda:
/// a conversão de trimesh/compound é cara e a lista é estável a maior parte
/// dos frames.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn sync_water_obstacles(
    terrain: Option<Res<TerrainRuntime>>,
    waves: Option<Res<WaterWaveWorld>>,
    obstacle_q: Query<(&GlobalTransform, &Collider), WaterObstacleFilter>,
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
) {
    let (Some(rt), Some(waves)) = (terrain, waves) else {
        return;
    };
    let window_moved = last_origin.is_none_or(|o| o != waves.origin_cell_1m);
    let colliders_changed =
        !changed_obs.is_empty() || removed_obs.read().count() > 0 || window_moved;
    if !colliders_changed {
        return;
    }
    *last_origin = Some(waves.origin_cell_1m);

    let mut found: Vec<WaterObstacle> = Vec::new();
    if !rt.water.is_empty() {
        let ox = waves.origin_cell_1m.x as f32 * DOMAIN_SNAP_M;
        let oz = waves.origin_cell_1m.y as f32 * DOMAIN_SNAP_M;
        let pad = OBSTACLE_HALO_M + 14.0;
        let win_min = Vec2::new(ox - pad, oz - pad);
        let win_max = Vec2::new(ox + WAVE_WORLD_SIZE + pad, oz + WAVE_WORLD_SIZE + pad);

        for (global, collider) in &obstacle_q {
            let placement = global.compute_transform();
            let iso = Transform {
                translation: placement.translation,
                rotation: placement.rotation,
                scale: Vec3::ONE,
            };
            let local = collider.shape_scaled().compute_local_aabb();
            let local_min = Vec3::new(local.mins.x, local.mins.y, local.mins.z);
            let local_max = Vec3::new(local.maxs.x, local.maxs.y, local.maxs.z);
            let (wmin, wmax) = rotated_aabb(&iso, local_min, local_max);
            if wmax.x < win_min.x || wmin.x > win_max.x || wmax.z < win_min.y || wmin.z > win_max.y
            {
                continue;
            }
            if let Some((wy_min, wy_max)) = water_surface_range_in_aabb(&rt, wmin.xz(), wmax.xz()) {
                if wmax.y >= wy_min - OBSTACLE_SLICE_BELOW_M
                    && wmin.y <= wy_max + OBSTACLE_SLICE_ABOVE_M
                {
                    collect_collider_water_obstacles(
                        collider,
                        iso,
                        wy_min,
                        wy_max,
                        &mut found,
                    );
                }
            }
        }

        collect_voxel_water_obstacles(
            &rt,
            Vec2::new(ox, oz),
            Vec2::new(ox + WAVE_WORLD_SIZE, oz + WAVE_WORLD_SIZE),
            &mut found,
        );
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

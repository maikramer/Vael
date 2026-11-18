//! O clima do `bevy_elements` a chegar ao chão do Vael.
//!
//! O terreno `.ahgt` não é o do crate. Aqui só se escrevem os resources que
//! [`viber::terrain::ground`] já sabe desenhar: clima e foco do manto, o
//! campo de rastos (`TrailGround` + `Imprint`) e a sombra das nuvens. Os
//! props exteriores passam a `WeatheredMaterial`.
//!
//! Gates (ligados por omissão): `VAEL_SNOW=0`, `VAEL_TRAILS=0`, `VAEL_SURFACE=0`.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use bevy_elements::{
    CloudShadows, ElementsSystems, Imprint, SnowSettings, SurfaceWeathering, TrailGround,
    TrailState, Weather, WeatheredExtension, WeatheredMaterial, WorldClock,
};
use bevy_ext_world::bsn::{WorldNode, WorldOrigin};
use bevy_flora::PooledTree;
use viber::ai::AiLocomotion;
use viber::animation::CharacterAnimator;
use viber::player::Player;
use viber::terrain::{
    GroundCloudField, GroundTrailField, SnowClimate, SnowFocus, SnowSeed, SnowShelter,
    TerrainRuntime,
};

use crate::environment::elements_enabled;

/// `0` / `false` / `off` / `no` desliga. Ausente liga.
pub fn env_enabled(name: &str) -> bool {
    !matches!(
        std::env::var(name).ok().map(|value| value.trim().to_ascii_lowercase()),
        Some(value) if matches!(value.as_str(), "0" | "false" | "off" | "no")
    )
}

/// Só `1` / `true` / `on` / `yes` liga.
fn env_enabled_strict(name: &str) -> bool {
    matches!(
        std::env::var(name).ok().map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("1" | "true" | "on" | "yes")
    )
}

#[allow(clippy::too_many_arguments)]
fn snow_debug(
    time: Res<Time>,
    mut next: Local<f32>,
    env: bevy_elements::Environment,
    weathering: Option<Res<SurfaceWeathering>>,
    trails: Option<Res<TrailState>>,
    ground: Option<Res<TrailGround>>,
    imprints: Query<(&GlobalTransform, &Imprint)>,
    player: Query<&GlobalTransform, With<Player>>,
    terrain: Option<Res<TerrainRuntime>>,
    pack: Option<Res<viber::terrain::SnowState>>,
    rigs: Query<&viber::ik::FootIkRigs, With<Player>>,
    bones: Query<&GlobalTransform>,
) {
    if time.elapsed_secs() < *next {
        return;
    }
    *next = time.elapsed_secs() + 3.0;
    let hero = player.iter().next().map(|t| t.translation());
    let w = weathering.as_deref();
    info!(
        target: "vael::snow",
        hero = ?hero,
        terrain_h = ?hero.and_then(|h| terrain.as_deref().map(|t| t.sample(h.x, h.z))),
        weathering_depth = ?w.map(|w| w.snow_depth_m),
        weathering_cover = ?w.map(|w| w.snow_cover),
        pack_cover = ?hero.and_then(|h| pack.as_deref().map(|p| p.coverage_at(h.xz()))),
        trails = ?trails.as_deref().map(|t| (t.texels, t.texel_m, t.centre)),
        trail_ground = ground.is_some(),
        imprints = imprints.iter().count(),
        sole = ?imprints.iter().next().map(|(t, _)| t.translation()),
        trail_depth = ?hero.and_then(|h| env.trail_depth(h.xz())),
        feet_above_ground = ?rigs.iter().next().map(|rig| {
            rig.legs.map(|leg| {
                bones.get(leg.bones.2).map_or(f32::NAN, |b| {
                    let p = b.translation();
                    p.y - terrain.as_deref().map_or(0.0, |t| t.sample(p.x, p.z))
                })
            })
        }),
        "neve"
    );
}

/// `VAEL_START_SNOW_DEPTH` (m): arranca com o manto já assente (QA, inverno).
#[must_use]
pub fn start_snow_depth() -> Option<f32> {
    std::env::var("VAEL_START_SNOW_DEPTH")
        .ok()
        .and_then(|v| v.trim().parse::<f32>().ok())
        .filter(|d| d.is_finite() && *d > 0.0)
        .map(|d| d.min(1.4))
}

/// `VAEL_SNOW_GAIN`: ganho da queda ao vivo (0 = o do runtime).
fn snow_fall_gain() -> f32 {
    std::env::var("VAEL_SNOW_GAIN")
        .ok()
        .and_then(|v| v.trim().parse::<f32>().ok())
        .filter(|g| g.is_finite() && *g > 0.0)
        .unwrap_or(0.0)
}

#[must_use]
pub fn snow_enabled() -> bool {
    env_enabled("VAEL_SNOW")
}

#[must_use]
pub fn trails_enabled() -> bool {
    env_enabled("VAEL_TRAILS")
}

#[must_use]
pub fn surface_enabled() -> bool {
    env_enabled("VAEL_SURFACE")
}

/// Y da sola. No chão marca (à superfície da neve: o crate só pressiona com a
/// sola a ≤ 0,1 m do chão + pack); no ar ou na água (longe do leito) levanta
/// e pára.
#[must_use]
pub fn sole_height(body_y: f32, ground_y: f32, snow_m: f32) -> f32 {
    if (body_y - ground_y).abs() < 1.8 {
        ground_y + snow_m.max(0.0)
    } else {
        ground_y + 5.0
    }
}

/// Ficheiro de um interior (`scenes/interiors/…`).
#[must_use]
pub fn interior_file(file: &str) -> bool {
    file.contains("interiors")
}

pub struct VaelGroundPlugin;

impl Plugin for VaelGroundPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SnowClimate>()
            .init_resource::<SnowFocus>()
            .init_resource::<SnowShelter>()
            .add_systems(PreUpdate, (push_snow_climate, push_snow_focus));
        if elements_enabled() && trails_enabled() {
            app.add_systems(
                Update,
                (install_trail_ground, sync_soles).before(ElementsSystems::Simulation),
            );
            app.add_systems(Update, push_trail_field.after(ElementsSystems::Simulation));
        }
        if elements_enabled() && snow_enabled() {
            if let Some(depth_m) = start_snow_depth() {
                app.insert_resource(SnowSeed { depth_m });
            }
            // Depois do `update_weathering` do crate (Weather) e antes dos
            // rastos (Simulation): o pack uniforme do crate passa a ser o
            // nosso no ponto do herói.
            app.add_systems(
                Update,
                push_pack_snow
                    .after(ElementsSystems::Weather)
                    .before(ElementsSystems::Simulation),
            );
        }
        if elements_enabled() && env_enabled_strict("VAEL_SNOW_DEBUG") {
            app.add_systems(Update, snow_debug.after(ElementsSystems::Readback));
        }
        if elements_enabled() && surface_enabled() {
            app.add_systems(Update, push_cloud_field.after(ElementsSystems::Simulation));
            app.add_systems(Update, weather_props);
        }
    }
}

fn push_snow_climate(
    time: Res<Time>,
    weather: Option<Res<Weather>>,
    sun: Option<Res<bevy_elements::SunState>>,
    clock: Option<Res<WorldClock>>,
    mut climate: ResMut<SnowClimate>,
) {
    let Some(weather) = weather else {
        if climate.enabled {
            climate.enabled = false;
            climate.game_dt = 0.0;
        }
        return;
    };
    let snow = weather.snow_fraction();
    let paused = clock.as_deref().is_some_and(|clock| clock.paused);
    let scale = clock
        .as_deref()
        .map(|clock| clock.time_scale)
        .unwrap_or(1.0);
    let sun = sun
        .as_deref()
        .map(|sun| sun.altitude_deg.to_radians().sin().max(0.0))
        .unwrap_or(0.0);
    let next = SnowClimate {
        enabled: snow_enabled(),
        snowfall_mm_h: weather.precipitation_mm_h * snow,
        rain_mm_h: weather.precipitation_mm_h * (1.0 - snow),
        temperature_c: weather.temperature_c,
        wind_ms: weather.wind_direction * weather.wind_speed_ms,
        sun,
        game_dt: if paused {
            0.0
        } else {
            time.delta_secs() * scale
        },
        fall_gain: snow_fall_gain(),
    };
    if *climate != next {
        *climate = next;
    }
}

fn push_snow_focus(player: Query<&GlobalTransform, With<Player>>, mut focus: ResMut<SnowFocus>) {
    let Some(transform) = player.iter().next() else {
        return;
    };
    let xz = transform.translation().xz();
    if focus.xz.distance_squared(xz) > 0.05 {
        focus.xz = xz;
    }
}

fn install_trail_ground(
    mut commands: Commands,
    terrain: Option<Res<TerrainRuntime>>,
    existing: Option<Res<TrailGround>>,
    mut key: Local<Option<usize>>,
) {
    let Some(terrain) = terrain else {
        return;
    };
    let next = std::sync::Arc::as_ptr(&terrain.grid) as usize;
    if *key == Some(next) && existing.is_some() {
        return;
    }
    let reader = terrain.reader();
    commands.insert_resource(TrailGround::new(move |xz| reader.sample(xz.x, xz.y)));
    // O pack de neve do crate, no chão alheio, é uniforme (o manto espacial
    // é o nosso). O default chega.
    commands.init_resource::<SnowSettings>();
    *key = Some(next);
}

#[derive(Component)]
struct Sole {
    body: Entity,
    /// Osso do pé (rig com `FootIkRigs`); `None` = sola única no corpo.
    foot: Option<Entity>,
}

/// Altura do osso do pé acima da sola com o pé assente, metros (tornozelo).
const FOOT_ANKLE_M: f32 = 0.1;
/// Folga acima disso em que o pé ainda conta como assente.
const FOOT_CONTACT_M: f32 = 0.06;

/// O pé pressiona? Assente no chão nu, ou enterrado no pack (em neve funda
/// a perna rasga um sulco contínuo).
#[must_use]
pub fn foot_presses(foot_y: f32, ground_y: f32, snow_m: f32) -> bool {
    foot_y - ground_y <= FOOT_ANKLE_M + FOOT_CONTACT_M + snow_m.max(0.0) * 0.7
}

#[allow(clippy::type_complexity)]
fn sync_soles(
    mut commands: Commands,
    terrain: Option<Res<TerrainRuntime>>,
    weathering: Option<Res<SurfaceWeathering>>,
    players: Query<(Entity, &GlobalTransform), With<Player>>,
    npcs: Query<(Entity, &GlobalTransform), (With<AiLocomotion>, Without<Player>)>,
    rigs: Query<&viber::ik::FootIkRigs>,
    bones: Query<&GlobalTransform, Without<Sole>>,
    mut soles: Query<(Entity, &Sole, &mut Transform)>,
) {
    let Some(terrain) = terrain else {
        return;
    };
    // O pack que o crate assume no chão alheio é uniforme: o mesmo aqui.
    let snow = weathering.map_or(0.0, |w| w.snow_depth_m);
    let hero = players
        .iter()
        .next()
        .map(|(_, transform)| transform.translation());
    let body_of = |body: Entity| {
        players
            .get(body)
            .ok()
            .or_else(|| npcs.get(body).ok())
            .map(|(_, t)| *t)
    };
    let feet_of = |body: Entity| -> Option<[Entity; 2]> {
        let rig = rigs.get(body).ok()?;
        Some([rig.legs[0].bones.2, rig.legs[1].bones.2])
    };
    // Pose da sola: no pé (ou no corpo), virada para onde o corpo olha.
    let pose = |body: &GlobalTransform, foot: Option<Entity>| -> Option<Transform> {
        let yaw = {
            let f = body.forward();
            Quat::from_rotation_y(f.x.atan2(f.z))
        };
        let at = body.translation();
        let (xz, y) = match foot {
            Some(foot) => {
                let bone = bones.get(foot).ok()?.translation();
                let ground = terrain.sample(bone.x, bone.z);
                let y = if (at.y - ground).abs() < 1.8 && foot_presses(bone.y, ground, snow) {
                    ground + snow
                } else {
                    ground + 5.0
                };
                (bone.xz(), y)
            }
            None => {
                let ground = terrain.sample(at.x, at.z);
                (at.xz(), sole_height(at.y, ground, snow))
            }
        };
        Some(Transform::from_xyz(xz.x, y, xz.y).with_rotation(yaw))
    };

    let mut live: HashSet<(Entity, Option<Entity>)> = HashSet::new();
    for (entity, sole, mut transform) in &mut soles {
        let Some(body) = body_of(sole.body) else {
            commands.entity(entity).despawn();
            continue;
        };
        // O rig mudou (ligou depois, ou rebind de LOD): a sola antiga sai.
        let feet = feet_of(sole.body);
        let valid = match (sole.foot, feet) {
            (None, None) => true,
            (Some(foot), Some(feet)) => feet.contains(&foot),
            _ => false,
        };
        if !valid {
            commands.entity(entity).despawn();
            continue;
        }
        let Some(next) = pose(&body, sole.foot) else {
            continue;
        };
        live.insert((sole.body, sole.foot));
        if *transform != next {
            *transform = next;
        }
    }
    let mut spawn = |commands: &mut Commands, body: Entity, transform: &GlobalTransform, load_n: f32| {
        let feet: Vec<Option<Entity>> = match feet_of(body) {
            Some(feet) => feet.iter().copied().map(Some).collect(),
            None => vec![None],
        };
        for foot in feet {
            if live.contains(&(body, foot)) {
                continue;
            }
            let Some(at) = pose(transform, foot) else {
                continue;
            };
            live.insert((body, foot));
            commands.spawn((Sole { body, foot }, at, Imprint::new(Vec2::new(0.05, 0.13), load_n)));
        }
    };
    for (body, transform) in &players {
        spawn(&mut commands, body, transform, 760.0);
    }
    for (body, transform) in &npcs {
        let near = hero
            .map(|hero| hero.distance_squared(transform.translation()) < 80.0 * 80.0)
            .unwrap_or(false);
        if near {
            spawn(&mut commands, body, transform, 600.0);
        }
    }
}

fn push_trail_field(
    state: Option<Res<TrailState>>,
    weathering: Option<Res<SurfaceWeathering>>,
    mut commands: Commands,
    existing: Option<Res<GroundTrailField>>,
) {
    let Some(state) = state else {
        if existing.is_some() {
            commands.remove_resource::<GroundTrailField>();
        }
        return;
    };
    let next = GroundTrailField {
        image: state.field.clone(),
        texel_m: state.texel_m,
        texels: state.texels,
        centre: state.centre,
        wetness: weathering.map(|weather| weather.wetness).unwrap_or(0.0),
        enabled: true,
    };
    if existing.as_deref() == Some(&next) {
        return;
    }
    commands.insert_resource(next);
}

fn push_cloud_field(
    shadows: Option<Res<CloudShadows>>,
    mut commands: Commands,
    existing: Option<Res<GroundCloudField>>,
) {
    let Some(shadows) = shadows else {
        if existing.is_some() {
            commands.remove_resource::<GroundCloudField>();
        }
        return;
    };
    let light = shadows.generation_direction();
    let next = GroundCloudField {
        image: shadows.map.clone(),
        origin: shadows.origin,
        extent: shadows.extent,
        light,
        mean_transmittance: shadows.mean_transmittance,
        enabled: shadows.extent > 1.0,
    };
    if existing.as_deref() == Some(&next) {
        return;
    }
    commands.insert_resource(next);
}

#[derive(Component)]
struct SurfaceChecked;

/// Pavimento e estradas são malhas de runtime, sem `WorldNode`. A neve do
/// chunk não as pinta; o material desgastado sim.
#[must_use]
pub fn is_ground_name(name: &str) -> bool {
    let name = name.trim();
    name.starts_with("paving") || name.starts_with("road") || name.starts_with("junction")
}

fn push_pack_snow(
    pack: Option<Res<viber::terrain::SnowState>>,
    weathering: Option<ResMut<SurfaceWeathering>>,
) {
    let (Some(pack), Some(mut weathering)) = (pack, weathering) else {
        return;
    };
    if !pack.draws() {
        return;
    }
    let depth = pack.reference_depth();
    let cover = viber::terrain::ground::snow_coverage(depth);
    if (weathering.snow_depth_m - depth).abs() > 1e-4 {
        weathering.snow_depth_m = depth;
    }
    if (weathering.snow_cover - cover).abs() > 1e-3 {
        weathering.snow_cover = cover;
    }
}

fn weather_props(
    mut commands: Commands,
    meshes: Query<
        (Entity, &MeshMaterial3d<StandardMaterial>, Option<&Name>),
        Without<SurfaceChecked>,
    >,
    parents: Query<&ChildOf>,
    nodes: Query<&WorldNode>,
    origins: Query<&WorldOrigin>,
    blocked: Query<(), Or<(With<CharacterAnimator>, With<PooledTree>, With<Player>)>>,
    standards: Res<Assets<StandardMaterial>>,
    mut weathered: ResMut<Assets<WeatheredMaterial>>,
    mut cache: Local<HashMap<(AssetId<StandardMaterial>, bool), Handle<WeatheredMaterial>>>,
) {
    const BUDGET: usize = 64;
    let mut seen = 0;
    for (entity, material, name) in &meshes {
        if seen >= BUDGET {
            break;
        }
        seen += 1;
        let handle = &material.0;
        let Some(base) = standards.get(handle) else {
            continue;
        };
        let ground = name.is_some_and(|name| is_ground_name(name.as_str()));
        let file = origin_file(entity, &parents, &nodes, &origins);
        let skip = if ground {
            false
        } else {
            file.is_none()
                || file.as_deref().is_some_and(interior_file)
                || ancestor_blocked(entity, &parents, &blocked)
        };
        if skip {
            commands.entity(entity).try_insert(SurfaceChecked);
            continue;
        }
        let id = (handle.id(), ground);
        let weathered_handle = if let Some(existing) = cache.get(&id) {
            existing.clone()
        } else {
            let created = weathered.add(WeatheredMaterial {
                base: base.clone(),
                extension: if ground { WeatheredExtension::with_exposure(0.18) } else { WeatheredExtension::default() },
            });
            cache.insert(id, created.clone());
            created
        };
        commands
            .entity(entity)
            .remove::<MeshMaterial3d<StandardMaterial>>()
            .insert((MeshMaterial3d(weathered_handle), SurfaceChecked));
    }
}

fn origin_file(
    start: Entity,
    parents: &Query<&ChildOf>,
    nodes: &Query<&WorldNode>,
    origins: &Query<&WorldOrigin>,
) -> Option<String> {
    let mut current = start;
    for _ in 0..24 {
        if let Ok(origin) = origins.get(current)
            && !origin.file.is_empty()
        {
            return Some(origin.file.clone());
        }
        if let Ok(node) = nodes.get(current) {
            return Some(node.origin_file.clone());
        }
        let Ok(child) = parents.get(current) else {
            break;
        };
        current = child.parent();
    }
    None
}

fn ancestor_blocked(
    start: Entity,
    parents: &Query<&ChildOf>,
    blocked: &Query<(), Or<(With<CharacterAnimator>, With<PooledTree>, With<Player>)>>,
) -> bool {
    let mut current = start;
    for _ in 0..24 {
        if blocked.get(current).is_ok() {
            return true;
        }
        let Ok(child) = parents.get(current) else {
            break;
        };
        current = child.parent();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_standing_sole_presses_and_a_swimmer_does_not() {
        assert_eq!(sole_height(2.0, 1.9, 0.0), 1.9);
        assert_eq!(sole_height(2.0, 1.9, 0.3), 2.2);
        assert_eq!(sole_height(8.0, 1.0, 0.3), 6.0);
    }

    #[test]
    fn a_lifted_foot_presses_only_inside_deep_snow() {
        // Pé assente (tornozelo a 10 cm) pressiona; no ar a 40 cm não.
        assert!(foot_presses(1.10, 1.0, 0.0));
        assert!(!foot_presses(1.40, 1.0, 0.0));
        // Com 50 cm de neve a perna está dentro do pack: rasga.
        assert!(foot_presses(1.40, 1.0, 0.5));
    }

    #[test]
    fn interior_files_stay_dry() {
        assert!(interior_file("scenes/interiors/barn.bsn"));
        assert!(!interior_file("world/landmarks/village.bsn"));
    }

    #[test]
    fn the_gates_default_on() {
        assert!(env_enabled("VAEL_SNOW_TEST_UNSET_GATE"));
    }

    #[test]
    fn paving_counts_as_ground() {
        assert!(is_ground_name("paving 3"));
        assert!(is_ground_name("road north"));
        assert!(is_ground_name("junction 1"));
        assert!(!is_ground_name("torch-post"));
    }
}

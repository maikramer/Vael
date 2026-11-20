//! Find a short, clear step from the hull to a walkable bank.
use super::{Boat, HULL_HALF, RidingBoat};
use avian3d::prelude::{Collider, Sensor, ShapeCastConfig, SpatialQuery, SpatialQueryFilter};
use bevy::prelude::*;
use bevy_ext_core::Player;
use vael_engine::input_map::{Action, ActionMap};
use vael_engine::luau::components::ScriptInteraction;
use vael_engine::physics::VoxelCollider;
use vael_engine::player::{HERO_CAPSULE_CENTER, HERO_RADIUS};
use vael_engine::terrain::TerrainRuntime;

#[derive(Component)]
pub(super) struct Landing(pub Vec3);

const LABEL: &str = "Desembarcar na margem";

#[allow(clippy::too_many_arguments)]
pub(super) fn update_landing(
    mut commands: Commands,
    riders: Query<(Entity, &RidingBoat), With<Player>>,
    boats: Query<
        (
            Entity,
            &Transform,
            Option<&Landing>,
            Option<&ScriptInteraction>,
        ),
        With<Boat>,
    >,
    terrain: Option<Res<TerrainRuntime>>,
    spatial: SpatialQuery,
    ground: Query<(), With<VoxelCollider>>,
    sensors: Query<(), With<Sensor>>,
    map: Option<Res<ActionMap>>,
    time: Res<Time>,
    mut next_check: Local<f32>,
    actions: Option<Res<vael_engine::input_map::ActionState>>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    let rider = riders.iter().next();
    for (boat, hull, old, prompt) in &boats {
        let who = rider.filter(|(_, r)| r.boat == boat);
        if who.is_none() && old.is_none() {
            continue;
        }
        let mut landing = None;
        if let (Some((hero, _)), Some(terrain)) = (who, terrain.as_deref()) {
            let now = time.elapsed_secs();
            if now < *next_check && !super::interaction_just_pressed(actions.as_deref(), &keys) {
                continue;
            }
            *next_check = now + 0.10;
            let filter = SpatialQueryFilter::default().with_excluded_entities([boat, hero]);
            landing = find_landing(hull, terrain, &spatial, &filter, &ground, &sensors);
        }
        if let Some(spot) = landing {
            if old.is_none_or(|p| p.0 != spot) {
                commands.entity(boat).insert(Landing(spot));
            }
            let key = map
                .as_deref()
                .and_then(|m| crate::input::action_key(m, Action::Interact))
                .unwrap_or(KeyCode::KeyE);
            if prompt.is_none_or(|p| p.key != key || p.label != LABEL) {
                commands.entity(boat).insert(ScriptInteraction {
                    label: LABEL.into(),
                    key,
                    range: 3.0,
                });
            }
        } else if old.is_some() {
            commands
                .entity(boat)
                .remove::<(Landing, ScriptInteraction)>();
        }
    }
}

fn find_landing(
    hull: &Transform,
    terrain: &TerrainRuntime,
    spatial: &SpatialQuery,
    filter: &SpatialQueryFilter,
    ground: &Query<(), With<VoxelCollider>>,
    sensors: &Query<(), With<Sensor>>,
) -> Option<Vec3> {
    let origin = hull.translation;
    let yaw = hull.rotation.to_euler(EulerRot::YXZ).0;
    let capsule = standing_capsule();
    for reach in [0.56, 1.15, 1.8] {
        for sector in 0..16 {
            let (sin, cos) = (std::f32::consts::TAU * sector as f32 / 16.0).sin_cos();
            // Start outside the gunwale, including at the bow and stern.
            let rim = ((cos / HULL_HALF.x).powi(2) + (sin / HULL_HALF.z).powi(2))
                .sqrt()
                .recip();
            let delta = Quat::from_rotation_y(yaw) * Vec3::new(cos, 0.0, sin) * (rim + reach);
            let p = origin + delta;
            if !terrain.in_field(p.x, p.z)
                || terrain
                    .water_domain_at(p.x, p.z)
                    .is_some_and(|d| d.depth > 0.04)
            {
                continue;
            }
            let top = Vec3::new(p.x, origin.y + 1.15, p.z);
            let Some(hit) =
                spatial.cast_ray_predicate(top, Dir3::NEG_Y, 2.45, true, filter, &|e| {
                    ground.contains(e)
                })
            else {
                continue;
            };
            if hit.normal.y < 0.82 {
                continue;
            }
            let foot = top - Vec3::Y * (hit.distance - 0.10);
            if terrain
                .water_surface_at(p.x, p.z)
                .is_some_and(|y| y > foot.y - 0.06)
            {
                continue;
            }
            if clear_step(origin, foot, &capsule, spatial, filter, sensors) {
                return Some(foot);
            }
        }
    }
    None
}

fn standing_capsule() -> Collider {
    Collider::capsule(HERO_RADIUS, 2.0 * (HERO_CAPSULE_CENTER - HERO_RADIUS))
}

fn clear_step(
    origin: Vec3,
    foot: Vec3,
    capsule: &Collider,
    spatial: &SpatialQuery,
    filter: &SpatialQueryFilter,
    sensors: &Query<(), With<Sensor>>,
) -> bool {
    let solid = |e| !sensors.contains(e);
    let center = foot + Vec3::Y * HERO_CAPSULE_CENTER;
    if spatial
        .shape_intersections(capsule, center, Quat::IDENTITY, filter)
        .into_iter()
        .any(solid)
    {
        return false;
    }
    let start = Vec3::new(
        origin.x,
        center.y.max(origin.y + HERO_CAPSULE_CENTER),
        origin.z,
    );
    let step = center - start;
    Dir3::new(step).is_ok_and(|dir| {
        spatial
            .cast_shape_predicate(
                capsule,
                start,
                Quat::IDENTITY,
                dir,
                &ShapeCastConfig::from_max_distance(step.length()),
                filter,
                &solid,
            )
            .is_none()
    })
}

/// When leaving in open water, don't drop the swimmer inside a rock alongside.
pub(super) fn swim_exit(
    hull: &Transform,
    terrain: &TerrainRuntime,
    spatial: &SpatialQuery,
    filter: &SpatialQueryFilter,
    sensors: &Query<(), With<Sensor>>,
) -> Option<Vec3> {
    let capsule = standing_capsule();
    let side = Quat::from_rotation_y(hull.rotation.to_euler(EulerRot::YXZ).0) * Vec3::X;
    for sign in [1.0, -1.0] {
        let foot =
            hull.translation + sign * side * (HULL_HALF.x + HERO_RADIUS + 0.22) + 0.20 * Vec3::Y;
        if terrain
            .water_domain_at(foot.x, foot.z)
            .is_some_and(|w| w.depth > 0.12)
            && clear_step(hull.translation, foot, &capsule, spatial, filter, sensors)
        {
            return Some(foot);
        }
    }
    None
}

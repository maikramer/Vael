//! Keep the rowboat's waterline inside water deep enough for its keel.
use super::{Boat, BoatPose, HULL_HALF, TerrainRuntime, rowboat_body};
use avian3d::prelude::{AngularVelocity, LinearVelocity, Position, Rotation};
use bevy::prelude::*;

/// Deepest point of the keel (54 cm) plus a little under-keel clearance.
pub(super) const MIN_DEPTH: f32 = 0.60;

fn clears(at: Vec2, yaw: f32, depth: impl Fn(Vec2) -> f32) -> bool {
    let (sin, cos) = yaw.sin_cos();
    let forward = Vec2::new(sin, cos) * HULL_HALF.z;
    let side = Vec2::new(cos, -sin) * HULL_HALF.x;
    [
        Vec2::ZERO,
        forward,
        -forward,
        side,
        -side,
        forward * 0.6 + side * 0.85,
        forward * 0.6 - side * 0.85,
        -forward * 0.6 + side * 0.85,
        -forward * 0.6 - side * 0.85,
    ]
    .into_iter()
    .all(|offset| depth(at + offset) >= MIN_DEPTH)
}

fn afloat(terrain: &TerrainRuntime, at: Vec2, yaw: f32) -> bool {
    clears(at, yaw, |p| {
        terrain.water_domain_at(p.x, p.y).map_or(0.0, |d| d.depth)
    })
}

/// Old saves can contain a hull tied on land. Also fit authored boat spawns
/// to the waterline, since terrain edits can move a bank across its mooring.
pub(super) fn fit_pose(terrain: &TerrainRuntime, mut pose: BoatPose) -> Option<BoatPose> {
    if !pose.position.is_finite() || !pose.yaw.is_finite() {
        return None;
    }
    let at = pose.position.xz();
    if afloat(terrain, at, pose.yaw) {
        return Some(pose);
    }
    for ring in 1..=64 {
        for sector in 0..24 {
            let angle = std::f32::consts::TAU * sector as f32 / 24.0;
            let p = at + Vec2::from_angle(angle) * (0.5 * ring as f32);
            if afloat(terrain, p, pose.yaw) {
                let water = terrain.water_surface_at(p.x, p.y)?;
                pose.position = Vec3::new(p.x, water + rowboat_body().rest_height(), p.y);
                pose.mooring = None;
                return Some(pose);
            }
        }
    }
    None
}

/// Avian has advanced Position/Rotation; Transform still holds the previous
/// fixed pose until writeback. Reject only the horizontal motion that would
/// run aground. Backing away remains possible, even with the blades stowed.
pub(super) fn stop_in_shallows(
    terrain: Option<Res<TerrainRuntime>>,
    mut boats: Query<(
        &Boat,
        &Transform,
        &mut Position,
        &mut Rotation,
        &mut LinearVelocity,
        &mut AngularVelocity,
    )>,
) {
    let Some(terrain) = terrain else { return };
    for (boat, before, mut pos, mut rot, mut linear, mut angular) in &mut boats {
        if boat.mooring.is_some() {
            continue;
        }
        let old = before.translation.xz();
        let next = pos.0.xz();
        let (yaw, pitch, roll) = rot.0.to_euler(EulerRot::YXZ);
        let prev_yaw = before.rotation.to_euler(EulerRot::YXZ).0;
        if afloat(&terrain, next, yaw) {
            continue;
        }
        // A turn can put the bow on sand even with no forward motion.
        if !afloat(&terrain, next, prev_yaw) {
            pos.x = old.x;
            pos.z = old.y;
            linear.x = 0.0;
            linear.z = 0.0;
        }
        if !afloat(&terrain, pos.0.xz(), yaw) {
            rot.0 = Quat::from_euler(EulerRot::YXZ, prev_yaw, pitch, roll);
            angular.y = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn banks_stop_the_bow_stern_and_beam_and_allow_backing_off() {
        let depth = |p: Vec2| (1.0 - p.y).max(0.0);
        let shore = Vec2::new(0.0, -HULL_HALF.z + 0.39);
        for yaw in [0.0, std::f32::consts::PI] {
            assert!(clears(shore, yaw, depth));
            assert!(!clears(shore + Vec2::Y * 0.1, yaw, depth));
            assert!(clears(shore - Vec2::Y, yaw, depth));
        }
        assert!(!clears(Vec2::ZERO, std::f32::consts::FRAC_PI_2, depth));
        assert!(!clears(Vec2::ZERO, 0., |_| 0.45));
    }
}

//! The rower's animation and the two oars share the boat's stroke phase.
use super::{
    Boat, OAR_BLADE_Z, OAR_GRIP_Z, Oar, RidingBoat, oar_pivot, oar_rotation, stroke_timing,
};
use bevy::prelude::*;
use bevy_ext_core::Player;
use bevy_ext_core::animation::{AnimState, AnimStateOverride, CharacterAnimator};
use viber::ui::script::{ScriptBindValue, UiScriptBinds};

/// Choose the seat pose on the boarding frame, before the hero driver can
/// mistake boarding from the water for a landing.
pub(super) fn choose_rider_state(
    mut heroes: Query<(&mut AnimStateOverride, Has<RidingBoat>), With<Player>>,
    riders: Query<(), (With<Player>, With<RidingBoat>)>,
    binds: Option<ResMut<UiScriptBinds>>,
    mut occupied: Local<bool>,
) {
    let aboard = !riders.is_empty();
    if aboard != *occupied
        && let Some(mut binds) = binds
    {
        *occupied = aboard;
        binds.0.insert(
            "boat.grip".into(),
            ScriptBindValue::Text(if aboard { "oars" } else { "" }.into()),
        );
    }
    for (mut state, aboard) in &mut heroes {
        if aboard {
            state.0 = Some(AnimState::Row);
        } else if state.0 == Some(AnimState::Row) {
            state.0 = None;
        }
    }
}

/// The hero's baked stroke reaches the catch at sample 27 and finishes at
/// sample 41 (20 Hz). Follow the oar sweep in either direction, including
/// the eased return to a seated hold when the player releases the controls.
pub(super) fn rowing_clip_time(boat: &Boat) -> f32 {
    let extension = stroke_timing(boat.oar_phase).0.cos() * boat.oar_direction * boat.oar_effort;
    let pull = 0.5 * (1.0 - extension);
    (27.0 + 14.0 * pull) / 20.0
}

pub(super) fn sync_rowing_clip(
    riders: Query<(&RidingBoat, &CharacterAnimator), With<Player>>,
    boats: Query<&Boat>,
    mut players: Query<&mut AnimationPlayer>,
) {
    for (rider, animator) in &riders {
        if animator.is_busy() || animator.state != Some(AnimState::Row) {
            continue;
        }
        let Some(node) = animator.current else {
            continue;
        };
        // The state can fall back to idle on a different rig.
        if animator.node_for(AnimState::Idle) == Some(node) {
            continue;
        }
        if let Ok(boat) = boats.get(rider.boat)
            && let Ok(mut player) = players.get_mut(animator.player)
            && let Some(clip) = player.animation_mut(node)
        {
            clip.set_speed(0.0).set_seek_time(rowing_clip_time(boat));
        }
    }
}

/// Pin the handle end in the palm. Preserve the blade depth and fore/aft path
/// by solving the remaining lateral reach of the rigid shaft.
pub(super) fn oar_pose(boat: &Boat, side: f32, grip: Option<Vec3>) -> Transform {
    let rotation = oar_rotation(boat, side);
    let pivot = oar_pivot(side);
    let Some(grip) = grip else {
        return Transform::from_translation(pivot).with_rotation(rotation);
    };
    let axis = rotation * Vec3::Z;
    let delta = pivot + axis * OAR_BLADE_Z - grip;
    let reach = OAR_BLADE_Z - OAR_GRIP_Z;
    let lateral_sq = reach * reach - delta.y * delta.y - delta.z * delta.z;
    let shaft = if lateral_sq > 0.0 {
        Vec3::new(side * lateral_sq.sqrt(), delta.y, delta.z) / reach
    } else {
        delta.normalize_or(axis)
    };
    let aimed = Quat::from_rotation_arc(axis, shaft) * rotation;
    Transform::from_translation(grip - shaft * OAR_GRIP_Z).with_rotation(aimed)
}

// Match the grip baked into the hero's row clip: just past the knuckle crease,
// below the two finger groups, measured using the actual size of this hand.
fn palm_socket(wrist: Vec3, inner: Vec3, outer: Vec3) -> Vec3 {
    let center = (inner + outer) * 0.5;
    let reach = center - wrist;
    let mut socket = center + 0.27 * reach;
    socket.y = inner.y.min(outer.y) - 0.13 * reach.length();
    socket
}

/// Lay unattended oars lengthwise across the thwarts, clear of the boarding side.
pub(super) fn stowed_oar(side: f32) -> Transform {
    Transform::from_xyz(0.30 * side, 0.018, -0.70)
}

/// A rider may bind or switch LOD after boarding. Cache knuckles only for
/// the current animation player. Their world transforms include this frame's
/// animation, sit blend, and hull motion.
pub(super) fn animate_oars(
    time: Res<Time>,
    riders: Query<(&RidingBoat, &CharacterAnimator)>,
    hierarchy: Query<&Children>,
    names: Query<&Name>,
    bones: Query<&GlobalTransform, Without<Oar>>,
    boats: Query<(&Boat, &GlobalTransform), Without<Oar>>,
    mut oars: Query<(&mut Oar, &ChildOf, &mut Transform, &mut GlobalTransform)>,
    mut cache: Local<Option<(Entity, [[Entity; 3]; 2])>>,
) {
    let grips = riders.iter().find_map(|(rider, rig)| {
        if rig.state != Some(AnimState::Row) || rig.is_busy() {
            return None;
        }
        if cache.as_ref().is_none_or(|(entity, hands)| {
            *entity != rig.player || hands.iter().flatten().any(|joint| !bones.contains(*joint))
        }) {
            let mut hands = [[None; 3]; 2];
            for joint in hierarchy.iter_descendants(rig.player) {
                match names.get(joint).ok().map(Name::as_str) {
                    Some("hand_l") => hands[0][0] = Some(joint),
                    Some("LeftHandFinger2") => hands[0][1] = Some(joint),
                    Some("LeftHandFinger3") => hands[0][2] = Some(joint),
                    Some("hand_r") => hands[1][0] = Some(joint),
                    Some("RightHandFinger2") => hands[1][1] = Some(joint),
                    Some("RightHandFinger3") => hands[1][2] = Some(joint),
                    _ => {}
                }
            }
            let complete =
                |[wrist, inner, outer]: [Option<Entity>; 3]| Some([wrist?, inner?, outer?]);
            *cache = Some((rig.player, [complete(hands[0])?, complete(hands[1])?]));
        }
        let (_, joints) = cache.as_ref()?;
        let (_, hull) = boats.get(rider.boat).ok()?;
        let inverse = hull.affine().inverse();
        let position =
            |entity| Some(inverse.transform_point3(bones.get(entity).ok()?.translation()));
        let socket = |[wrist, inner, outer]: [Entity; 3]| {
            Some(palm_socket(
                position(wrist)?,
                position(inner)?,
                position(outer)?,
            ))
        };
        let mut hands = [socket(joints[0])?, socket(joints[1])?];
        hands.sort_by(|a, b| a.x.total_cmp(&b.x));
        Some((rider.boat, hands))
    });
    let ridden = riders.iter().next().map(|(rider, _)| rider.boat);
    for (mut oar, parent, mut local, mut world) in &mut oars {
        let Ok((boat, hull)) = boats.get(parent.parent()) else {
            continue;
        };
        oar.grip = grips
            .filter(|(entity, _)| *entity == parent.parent())
            .map(|(_, hands)| hands[usize::from(oar.side > 0.0)]);
        let pose = if ridden == Some(parent.parent()) {
            oar_pose(boat, oar.side, oar.grip)
        } else {
            let stowed = stowed_oar(oar.side);
            if local.translation.distance_squared(stowed.translation) < 1e-6
                && local.rotation.angle_between(stowed.rotation) < 0.002
            {
                stowed
            } else {
                let t = 1.0 - (-12.0 * time.delta_secs()).exp();
                Transform::from_translation(local.translation.lerp(stowed.translation, t))
                    .with_rotation(local.rotation.slerp(stowed.rotation, t))
            }
        };
        // These leaf meshes are fitted after propagation to use the final
        // hand positions. Publish the corresponding world pose for extraction.
        if *local != pose {
            *local = pose;
            *world = hull.mul_transform(pose);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hands_hold_the_end_through_drive_recovery_and_reversal() {
        let mut boat = Boat::default();
        for target in [1.0, -1.0, 0.0] {
            for _ in 0..140 {
                super::super::advance_stroke(&mut boat, target, false, 1.0 / 60.0);
                for side in [-1.0, 1.0] {
                    let grip = Vec3::new(0.21 * side, 0.31, -0.23);
                    let pose = oar_pose(&boat, side, Some(grip));
                    assert!(pose.transform_point(Vec3::Z * OAR_GRIP_Z).distance(grip) < 1e-5);
                    let blade = Vec3::Z * OAR_BLADE_Z;
                    let planned = oar_pose(&boat, side, None).transform_point(blade);
                    let actual = pose.transform_point(blade);
                    assert!(
                        (actual.y - planned.y).abs() < 1e-5 && (actual.z - planned.z).abs() < 1e-5
                    );
                    assert!((actual.distance(grip) - (OAR_BLADE_Z - OAR_GRIP_Z)).abs() < 1e-5);
                    assert!(actual.x * side > 1.6);
                }
            }
        }
    }
}

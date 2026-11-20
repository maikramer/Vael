//! Continuous contact between the equipped tool head and the selected resource.
//! The authored stroke supplies the approach. The first contact sets its reversal point.
use super::*;
use avian3d::{
    parry::{
        math::Pose,
        query::{self, NonlinearRigidMotion},
    },
    prelude::Collider,
};
use bevy::transform::helper::TransformHelper;
use std::sync::{Arc, Mutex};
use vael_engine::{animation::CharacterAnimator, attachment::AttachmentSlots};

const CONTACT_HOLD: f32 = 0.042;
const SKIN: f32 = 0.004;
// Cross-section of pickaxe_collision.glb in grip-local metres (two bits and the poll).
const PICK_PROFILE: [(f32, f32); 6] = [
    (-0.182, 0.709),
    (-0.180, 0.719),
    (0.000, 0.894),
    (0.178, 0.704),
    (0.183, 0.673),
    (0.000, 0.688),
];

// Grip-local outline from felling_axe_collision.glb, including the flared cutting edge.
const AXE_PROFILE: [(f32, f32); 6] = [
    (-0.169, 0.675),
    (-0.169, 0.755),
    (-0.151, 0.843),
    (0.146, 0.837),
    (0.169, 0.716),
    (0.164, 0.671),
];

#[derive(Clone, Copy, Debug)]
pub(super) struct Surface {
    pub collider: Entity,
    pub point: Vec3,
    pub normal: Vec3,
}

#[derive(Clone, Copy, Default)]
pub(super) enum Contact {
    #[default]
    Pending,
    Touch(Surface),
    Miss,
    // Some legacy resource nodes are interaction points with no collision geometry.
    Unshaped,
}

#[derive(Default)]
struct Request {
    aim: Option<Entity>,
    target: Option<Entity>,
    serial: u64,
    result: Contact,
}

#[derive(Clone, Default, Resource)]
pub(super) struct Mining(Arc<Mutex<Request>>);

impl Mining {
    fn lock(&self) -> std::sync::MutexGuard<'_, Request> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub fn begin(&self, target: Entity) {
        let mut r = self.lock();
        r.target = Some(target);
        r.serial = r.serial.wrapping_add(1);
        r.result = Contact::Pending;
    }
    pub fn result(&self, target: Entity) -> Contact {
        let r = self.lock();
        if r.target == Some(target) {
            r.result
        } else {
            Contact::Miss
        }
    }
    pub fn surface(&self, target: Entity) -> Option<Surface> {
        match self.result(target) {
            Contact::Touch(s) => Some(s),
            _ => None,
        }
    }
    pub fn install(&self, lua: &mlua::Lua, vael: &mlua::Table) -> mlua::Result<()> {
        let mine = self;
        vael_fn!(
            lua,
            vael,
            "aim_pick",
            [mine = mine.clone()],
            move |_, bits: Option<i64>| {
                mine.lock().aim = bits.and_then(|id| Entity::try_from_bits(id as u64));
                Ok(())
            }
        );
        vael_fn!(
            lua,
            vael,
            "begin_mine",
            [mine = mine.clone()],
            move |_, bits: i64| {
                if let Some(e) = Entity::try_from_bits(bits as u64) {
                    mine.begin(e);
                }
                Ok(())
            }
        );
        vael_fn!(
            lua,
            vael,
            "mine_contact",
            [mine = mine.clone()],
            move |_, bits: i64| {
                Ok(
                    match Entity::try_from_bits(bits as u64).map(|e| mine.result(e)) {
                        Some(Contact::Touch(_)) => "hit",
                        Some(Contact::Unshaped) => "unshaped",
                        Some(Contact::Pending) => "pending",
                        _ => "miss",
                    },
                )
            }
        );
        Ok(())
    }
}

struct Head {
    shape: Collider,
    vertices: Vec<Vec3>,
}
impl Default for Head {
    fn default() -> Self {
        Self::new(&PICK_PROFILE, 0.0176)
    }
}
impl Head {
    fn new(profile: &[(f32, f32)], half_depth: f32) -> Self {
        let vertices: Vec<_> = profile
            .iter()
            .flat_map(|&(x, y)| [-half_depth, half_depth].map(|z| Vec3::new(x, y, z)))
            .collect();
        Self {
            shape: Collider::convex_hull(&vertices).expect("tool head has volume"),
            vertices,
        }
    }
}

struct Stone<'a> {
    entity: Entity,
    shape: &'a Collider,
    pose: Pose,
}
impl Stone<'_> {
    fn near(&self, head: &Head, grip: Pose, gap: f32) -> Option<Surface> {
        let c = query::contact(
            &self.pose,
            self.shape.shape_scaled().as_ref(),
            &grip,
            head.shape.shape_scaled().as_ref(),
            gap,
        )
        .ok()??;
        Some(Surface {
            collider: self.entity,
            point: c.point1,
            normal: c.normal1,
        })
    }
    fn sweep(&self, head: &Head, from: Pose, to: Pose) -> Option<(f32, Surface)> {
        if let Some(s) = self.near(head, from, 0.) {
            return Some((0., s));
        }
        let motion = NonlinearRigidMotion::new(
            from,
            Vec3::ZERO,
            to.translation - from.translation,
            (to.rotation * from.rotation.inverse()).to_scaled_axis(),
        );
        let wall = NonlinearRigidMotion::constant_position(self.pose);
        let hit = query::cast_shapes_nonlinear(
            &wall,
            self.shape.shape_scaled().as_ref(),
            &motion,
            head.shape.shape_scaled().as_ref(),
            0.,
            1.,
            true,
        )
        .ok()
        .flatten();
        if let Some(hit) = hit {
            let t = hit.time_of_impact;
            // Recompute the manifold at TOI, including the initially touching case.
            let s = self
                .near(head, motion.position_at_time(t), SKIN)
                .unwrap_or(Surface {
                    collider: self.entity,
                    point: self.pose * hit.witness1,
                    normal: self.pose.rotation * hit.normal1,
                });
            return Some((t, s));
        }
        self.near(head, to, 0.).map(|s| (1., s))
    }
}

// Lift from the shoulder while keeping the grip rigid. The plane supports the convex
// deposit, so keeping the whole head outside it also handles a frame that crosses an edge.
fn lift(head: &Head, grip: Pose, scale: Vec3, shoulder: Vec3, forward: Vec3, s: Surface) -> Quat {
    let axis = forward.cross(Vec3::Y).normalize_or(Vec3::X);
    let outside = |q: Quat| {
        head.vertices.iter().all(|p| {
            let p = shoulder + q * ((grip * (*p * scale)) - shoulder);
            (p - s.point).dot(s.normal) >= SKIN
        })
    };
    if outside(Quat::IDENTITY) {
        return Quat::IDENTITY;
    }
    let reach = grip * (Vec3::Y * 0.76 * scale) - shoulder;
    for axis in [axis, reach.cross(s.normal).normalize_or(axis), -axis] {
        let mut lo = 0.;
        for i in 1..=28 {
            let mut hi = i as f32 * 0.10;
            if outside(Quat::from_axis_angle(axis, hi)) {
                for _ in 0..10 {
                    let mid = 0.5 * (lo + hi);
                    if outside(Quat::from_axis_angle(axis, mid)) {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
                return Quat::from_axis_angle(axis, hi);
            }
            lo = hi;
        }
    }
    Quat::IDENTITY
}

#[derive(Default)]
pub(super) struct Stroke {
    serial: u64,
    age: f32,
    previous: Option<(Entity, Entity, Pose, f32)>,
    rebound: Option<(Entity, bevy::animation::graph::AnimationNodeIndex, f32)>,
    bounced: bool,
    head: Head,
    axe: bool,
}

#[derive(Resource, Default)]
pub(super) struct ArmOverride(Option<(Entity, Quat, Quat)>);

pub(super) fn clear_override(mut saved: ResMut<ArmOverride>, mut bones: Query<&mut Transform>) {
    if let Some((joint, original, applied)) = saved.0.take()
        && let Ok(mut bone) = bones.get_mut(joint)
        && bone.rotation.abs_diff_eq(applied, 1e-5)
    {
        bone.rotation = original;
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn constrain(
    mine: Res<Mining>,
    time: Res<Time>,
    focus: Res<HarvestFocus>,
    slots: Option<Res<AttachmentSlots>>,
    heroes: Query<Entity, With<Player>>,
    hierarchy: Query<(&ChildOf, Option<&Name>)>,
    children: Query<&Children>,
    rocks: Query<(&Collider, &GlobalTransform), Without<avian3d::prelude::Sensor>>,
    rigs: Query<&CharacterAnimator>,
    mut players: Query<&mut AnimationPlayer>,
    mut poses: ParamSet<(TransformHelper, Query<&mut Transform>)>,
    mut state: Local<Stroke>,
    mut saved: ResMut<ArmOverride>,
) {
    // The last hit can despawn the deposit before the hold ends.
    if let Some((entity, node, remaining)) = state.rebound.take() {
        let remaining = remaining - time.delta_secs();
        if remaining > 0. {
            state.rebound = Some((entity, node, remaining));
        } else if let Ok(mut player) = players.get_mut(entity)
            && let Some(a) = player.animation_mut(node)
            && a.speed() == 0.
        {
            a.set_speed(-0.85);
        }
    }
    let (seq, request, lua_aim) = {
        let r = mine.lock();
        (r.serial, r.target, r.aim)
    };
    if seq != state.serial {
        state.serial = seq;
        state.age = 0.;
        state.bounced = false;
        state.previous = None;
    }
    state.age += time.delta_secs();
    let axe = focus.tool == "chop";
    let aim = focus
        .entity
        .filter(|_| axe || focus.tool == "mine")
        .or(lua_aim);
    let active = request.is_some() && request == aim && state.age < 1.05;
    if state.axe != axe {
        state.axe = axe;
        state.head = if axe {
            Head::new(&AXE_PROFILE, 0.0238)
        } else {
            Head::default()
        };
        state.previous = None;
    }
    let Some(target) = (if active { request } else { aim }) else {
        state.previous = None;
        return;
    };
    let stones = || {
        std::iter::once(target)
            .chain(children.get(target).into_iter().flat_map(|c| c.iter()))
            .filter_map(|entity| {
                let (shape, tf) = rocks.get(entity).ok()?;
                Some(Stone {
                    entity,
                    shape,
                    pose: Pose::from_parts(tf.translation(), tf.rotation()),
                })
            })
    };
    let Some(first) = stones().next() else {
        if active && matches!(mine.result(target), Contact::Pending) {
            mine.lock().result = Contact::Unshaped;
        }
        state.previous = None;
        return;
    };
    let Some(hero) = heroes.iter().next() else {
        return;
    };
    // Before the rig or its tool has streamed in, keep the interaction usable.
    let without_rig = || {
        if active && matches!(mine.result(target), Contact::Pending) {
            mine.lock().result = Contact::Unshaped;
        }
    };
    let Some(tool) = slots
        .as_deref()
        .and_then(|s| s.0.get(&(hero, "weapon".into())))
        .copied()
    else {
        without_rig();
        return;
    };
    let mut shoulder = None;
    let mut rig = None;
    let mut bone = tool;
    while let Ok((parent, name)) = hierarchy.get(bone) {
        if name.is_some_and(|n| n.as_str() == "upperarm_r") {
            shoulder = Some(bone);
        }
        if let Ok(r) = rigs.get(bone) {
            rig = Some(r);
        }
        if bone == hero {
            break;
        }
        bone = parent.parent();
    }
    let (Some(shoulder), Some(rig)) = (shoulder, rig.or_else(|| rigs.get(hero).ok())) else {
        without_rig();
        return;
    };
    let clip_name = if axe { "chop" } else { "mine" };
    let Some(node) = rig.node_matching(|n| n == clip_name) else {
        without_rig();
        return;
    };
    let Ok(mut player) = players.get_mut(rig.player) else {
        return;
    };
    let mut clip = player
        .animation_mut(node)
        .filter(|_| rig.current == Some(node));
    let phase = clip
        .as_ref()
        .map_or(0., |a| a.seek_time() / rig.duration_of(node).max(0.001));
    let helper = poses.p0();
    let (Ok(hand), Ok(joint), Ok(body)) = (
        helper.compute_global_transform(tool),
        helper.compute_global_transform(shoulder),
        helper.compute_global_transform(hero),
    ) else {
        return;
    };
    let raw = Pose::from_parts(hand.translation(), hand.rotation());
    let shoulder_pos = joint.translation();
    let scale = hand.scale().abs();
    if !state.head.shape.scale().abs_diff_eq(scale, 1e-4) {
        state.head.shape.set_scale(scale, 8);
    }
    // Steer the downswing toward the reachable face. Rotate the whole arm about
    // the shoulder so the hand and grip keep their authored relative pose.
    let strike_phase = if axe { 0.29 } else { 0.41 };
    let aim_weight = if active && clip.is_some() {
        ((phase - 0.10) / (strike_phase - 0.10)).clamp(0., 1.)
    } else {
        0.
    };
    let edge = raw * (Vec3::new(if axe { -0.15 } else { 0. }, 0.80, 0.) * scale);
    let toward = (focus.contact - shoulder_pos).try_normalize();
    let authored = (edge - shoulder_pos).try_normalize();
    let steer = authored.zip(toward).map_or(Quat::IDENTITY, |(from, to)| {
        Quat::IDENTITY.slerp(Quat::from_rotation_arc(from, to), aim_weight * aim_weight)
    });
    let grip = Pose::from_parts(
        shoulder_pos + steer * (raw.translation - shoulder_pos),
        steer * raw.rotation,
    );
    let previous = state
        .previous
        .filter(|(e, t, _, _)| *e == target && *t == tool);
    let mut surface = None;
    if active
        && !state.bounced
        && clip.as_ref().is_some_and(|a| a.speed() > 0.)
        && ((if axe { 0.17 } else { 0.24 })..=0.80).contains(&phase)
    {
        let (_, _, from, start) = previous
            .filter(|(_, _, _, p)| *p <= phase)
            .unwrap_or((target, tool, grip, phase));
        if let Some((t, s)) = stones()
            .filter_map(|stone| stone.sweep(&state.head, from, grip))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            && let Some(a) = clip.as_deref_mut()
        {
            let contact_time = start.lerp(phase, t) * rig.duration_of(node);
            a.set_seek_time(contact_time).set_speed(0.);
            mine.lock().result = Contact::Touch(s);
            state.bounced = true;
            state.rebound = Some((rig.player, node, CONTACT_HOLD));
            surface = Some(s);
        }
    }
    if active && !state.bounced && (phase > 0.80 || state.age > 0.92) {
        mine.lock().result = Contact::Miss;
    }
    // Also protect the raised/recovering or resting tool when standing close.
    surface = surface.or_else(|| stones().find_map(|stone| stone.near(&state.head, grip, SKIN)));
    let mut corrected = grip;
    let mut arm_rotation = steer;
    if let Some(s) = surface {
        let forward = (first.pose.translation - body.translation())
            .with_y(0.)
            .normalize_or(Vec3::Z);
        let q = lift(&state.head, grip, scale, joint.translation(), forward, s);
        if q != Quat::IDENTITY {
            arm_rotation = q * steer;
            corrected = Pose::from_parts(
                joint.translation() + q * (grip.translation - joint.translation()),
                q * grip.rotation,
            );
        }
    }
    if arm_rotation != Quat::IDENTITY
        && let Ok(mut local) = poses.p1().get_mut(shoulder)
    {
        let original = local.rotation;
        local.rotation = original * joint.rotation().inverse() * arm_rotation * joint.rotation();
        saved.0 = Some((shoulder, original, local.rotation));
    }
    state.previous = Some((target, tool, corrected, phase));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotating_bit_can_hit_between_two_clear_frames() {
        let head = Head::default();
        let hull = Collider::sphere(0.09);
        let rock = Stone {
            entity: Entity::PLACEHOLDER,
            shape: &hull,
            pose: Pose::from_translation(Vec3::Y * 0.84),
        };
        let from = Pose::from_parts(Vec3::ZERO, Quat::from_rotation_z(-0.9));
        let to = Pose::from_parts(Vec3::ZERO, Quat::from_rotation_z(0.9));
        assert!(rock.near(&head, from, 0.).is_none() && rock.near(&head, to, 0.).is_none());
        let (toi, s) = rock.sweep(&head, from, to).expect("bit crossed the stone");
        assert!((0.0..0.5).contains(&toi));
        assert!(((s.point - rock.pose.translation).length() - 0.09).abs() < 0.003);
        assert!(s.normal.is_normalized());
        let miss = Stone {
            entity: Entity::PLACEHOLDER,
            shape: &hull,
            pose: Pose::from_translation(Vec3::Y * 1.4),
        };
        assert!(miss.sweep(&head, from, to).is_none());
    }

    #[test]
    fn shoulder_lift_clears_a_scaled_head_on_a_sloping_face() {
        let head = Head::default();
        let scale = Vec3::splat(1.25);
        let shoulder = Vec3::new(0., 1.45, 0.);
        let grip = Pose::from_parts(Vec3::new(0., 1., 0.4), Quat::from_rotation_x(1.9));
        for n in [Vec3::Y, Vec3::new(0.3, 0.9, -0.1).normalize()] {
            let s = Surface {
                collider: Entity::PLACEHOLDER,
                point: Vec3::new(0., 1.05, 1.2),
                normal: n,
            };
            let rotation = lift(&head, grip, scale, shoulder, Vec3::Z, s);
            assert_ne!(rotation, Quat::IDENTITY);
            for p in &head.vertices {
                let rotated = shoulder + rotation * ((grip * (*p * scale)) - shoulder);
                assert!((rotated - s.point).dot(n) >= SKIN - 1e-4);
            }
        }
    }
}

//! A shore tie fixes the hull in its landing pose until someone boards it.
use super::{Boat, BoatAssets, TerrainRuntime, hull_point};
use avian3d::prelude::{AngularVelocity, LinearVelocity, RigidBody};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Reflect, Serialize, Deserialize)]
pub struct Mooring {
    pub stake: Vec3,
    pub rotation: Quat,
}

impl Mooring {
    pub(super) fn at(foot: Vec3, hull: &Transform, terrain: &TerrainRuntime) -> Self {
        // Drive the stake beside the landing footprint on the same bank.
        let side = (foot - hull.translation).xz().normalize_or_zero().perp() * 0.48;
        let stake = [side, -side]
            .into_iter()
            .find_map(|offset| {
                let p = foot.xz() + offset;
                let y = terrain.surface_below(p.x, p.y, foot.y + 0.3)?;
                ((y - foot.y).abs() < 0.35
                    && terrain.water_surface_at(p.x, p.y).is_none_or(|w| y >= w))
                .then_some(Vec3::new(p.x, y, p.y))
            })
            .unwrap_or(foot - Vec3::Y * 0.1);
        Self {
            stake,
            rotation: hull.rotation,
        }
    }
}

pub(super) fn set(commands: &mut Commands, id: Entity, boat: &mut Boat, mooring: Option<Mooring>) {
    if boat.mooring != mooring {
        boat.mooring = mooring;
        let body = if mooring.is_some() {
            RigidBody::Static
        } else {
            RigidBody::Dynamic
        };
        commands
            .entity(id)
            .insert((body, LinearVelocity::ZERO, AngularVelocity::ZERO));
    }
}

#[derive(Component)]
pub(super) struct ShoreTie(Mooring);

pub(super) fn draw(
    mut commands: Commands,
    assets: Res<BoatAssets>,
    boats: Query<(Entity, &Boat, &Transform, &Children), Changed<Boat>>,
    ties: Query<&ShoreTie>,
) {
    for (id, boat, hull, children) in &boats {
        let old = children
            .iter()
            .find_map(|c| ties.get(c).ok().map(|t| (c, t.0)));
        if old.map(|(_, m)| m) == boat.mooring {
            continue;
        }
        if let Some((child, _)) = old {
            commands.entity(child).despawn();
        }
        let Some(m) = boat.mooring else { continue };
        // All geometry belongs to the boat, so unloading it also removes the tie.
        let root = commands
            .spawn((
                Name::new("Amarra da margem"),
                ShoreTie(m),
                Transform::default(),
                Visibility::Inherited,
                ChildOf(id),
            ))
            .id();
        let inv = hull.compute_affine().inverse();
        let stake = inv.transform_point3(m.stake);
        let up = hull.rotation.inverse() * Vec3::Y;
        let knot = stake + up * 0.47;
        let end = if stake.z > 0.0 { 0.94 } else { 0.06 };
        let cleat = hull_point(end, stake.x.signum(), 0.0);
        let mut rod = |name, a: Vec3, b: Vec3, radius, material: &Handle<StandardMaterial>| {
            let d = b - a;
            let transform = Transform::from_translation((a + b) * 0.5)
                .with_rotation(Quat::from_rotation_arc(Vec3::Y, d.normalize_or(Vec3::Y)))
                .with_scale(Vec3::new(radius, d.length(), radius));
            commands.spawn((
                Name::new(name),
                Mesh3d(assets.rod.clone()),
                MeshMaterial3d(material.clone()),
                transform,
                ChildOf(root),
            ));
        };
        rod(
            "Piquete",
            stake - up * 0.14,
            stake + up * 0.59,
            0.055,
            &assets.trim,
        );
        rod(
            "Volta da corda",
            knot - up * 0.028,
            knot + up * 0.028,
            0.065,
            &assets.rope,
        );
        let mut previous = cleat;
        for i in 1..=5 {
            let t = i as f32 / 5.0;
            let next = cleat.lerp(knot, t) - up * (0.40 * t * (1.0 - t));
            rod("Corda", previous, next, 0.018, &assets.rope);
            previous = next;
        }
    }
}

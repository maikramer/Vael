//! Reservable landing sites on the actual bark and on clear, dry ground.
use avian3d::prelude::{Sensor, SpatialQuery, SpatialQueryFilter};
use bevy::prelude::*;
use bevy_flora::{FloraIndex, FloraUnits, NodeKind, PooledTree, TreePool};
use std::f32::consts::TAU;
use viber::{player::Player, scene_loader::CurrentScene, terrain::TerrainRuntime};

#[derive(Clone, Copy)]
pub(super) struct Perch {
    pub(super) id: Site,
    pub(super) feet: Vec3,
    pub(super) outward: Vec3,
}
#[derive(Clone, Copy, PartialEq, Eq, Hash, Reflect)]
pub(super) enum Site {
    Branch(Entity),
    Clearing(IVec2),
}

impl Site {
    pub(super) fn available(self, trees: &Query<(), With<PooledTree>>) -> bool {
        match self {
            Self::Branch(tree) => trees.contains(tree),
            Self::Clearing(_) => true,
        }
    }
    pub(super) fn ground(self) -> bool {
        matches!(self, Self::Clearing(_))
    }
}

#[derive(Resource, Default)]
pub(super) struct Perches {
    pub(super) sites: Vec<Perch>,
    next_survey: f32,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn survey_perches(
    time: Res<Time>,
    hero: Query<&GlobalTransform, With<Player>>,
    scene: Option<Res<CurrentScene>>,
    index: Option<Res<FloraIndex>>,
    pools: Option<Res<Assets<TreePool>>>,
    units: Option<Res<FloraUnits>>,
    trees: Query<(&PooledTree, &GlobalTransform)>,
    terrain: Option<Res<TerrainRuntime>>,
    spatial: SpatialQuery,
    sensors: Query<(), With<Sensor>>,
    mut branches: ResMut<Perches>,
) {
    let (Some(hero), Some(terrain)) = (hero.iter().next(), terrain) else {
        branches.sites.clear();
        return;
    };
    if scene.as_deref().and_then(|s| s.id.as_deref()) != Some("main") {
        branches.sites.clear();
        return;
    }
    let now = time.elapsed_secs();
    if now < branches.next_survey {
        return;
    }
    branches.next_survey = now + 0.85;
    branches.sites.clear();
    let eye = hero.translation();
    let unit = units.as_deref().map_or(1., |u| u.units_per_metre);
    let filter = SpatialQueryFilter::default();
    let cell = (eye.xz() / 7.0).round().as_ivec2();
    for dz in -3..=3 {
        for dx in -3..=3 {
            let tile = cell + IVec2::new(dx, dz);
            let seed = tile.x.wrapping_mul(127) ^ tile.y.wrapping_mul(331);
            let angle = seed.rem_euclid(360) as f32 * TAU / 360.;
            let dir = Vec2::from_angle(angle);
            let xz = tile.as_vec2() * 7. + dir * 1.6;
            if !terrain.in_field(xz.x, xz.y) || terrain.in_water(xz.x, xz.y) {
                continue;
            }
            let h = terrain.sample(xz.x, xz.y);
            let top = Vec3::new(xz.x, h + 10., xz.y);
            let Some(hit) =
                spatial.cast_ray_predicate(top, Dir3::NEG_Y, 11., true, &filter, &|entity| {
                    !sensors.contains(entity)
                })
            else {
                continue;
            };
            let feet = top - Vec3::Y * hit.distance;
            if (feet.y - h).abs() > 0.32
                || hit.normal.y < 0.88
                || terrain
                    .water_surface_at(xz.x, xz.y)
                    .is_some_and(|w| feet.y < w + 0.05)
            {
                continue;
            }
            branches.sites.push(Perch {
                id: Site::Clearing(tile),
                feet,
                outward: Vec3::new(dir.x, 0., dir.y),
            });
        }
    }
    let ground_count = branches.sites.len();
    let (Some(index), Some(pools)) = (index, pools) else {
        return;
    };
    let mut near: Vec<_> = index.within(eye.xz(), 63.).collect();
    near.sort_by(|a, b| {
        a.base
            .distance_squared(eye)
            .total_cmp(&b.base.distance_squared(eye))
    });
    for t in near {
        let Ok((tree, tf)) = trees.get(t.entity) else {
            continue;
        };
        let Some(pool) = pools.get(&tree.pool) else {
            continue;
        };
        let v = &pool.variants[tree.variant as usize % pool.variants.len()];
        // Pick a reachable, exposed limb. point_at follows the wood's actual bend.
        let site = v
            .skeleton
            .nodes
            .iter()
            .filter(|n| {
                matches!(n.kind, NodeKind::Primary | NodeKind::Split)
                    && n.length > 0.45
                    && n.direction.y.abs() < 0.87
            })
            .filter_map(|n| {
                let along = 0.71;
                let bark = n.base_radius.lerp(n.tip_radius, along);
                let feet = tf.transform_point((n.point_at(along) + Vec3::Y * bark) * unit);
                let offset = feet - t.base;
                let reach = offset.xz().length();
                if !(1.2..10.5).contains(&offset.y) || reach < 0.43 {
                    return None;
                }
                let score = reach - 0.65 * (offset.y - 2.4).abs();
                Some((
                    score,
                    Perch {
                        id: Site::Branch(t.entity),
                        feet,
                        outward: offset.with_y(0.).normalize_or(Vec3::Z),
                    },
                ))
            })
            .max_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((_, site)) = site {
            branches.sites.push(site);
            if branches.sites.len() == ground_count + 48 {
                break;
            }
        }
    }
}

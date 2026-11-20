//! Check the shipped road surfaces against the three placed bridge decks.
use std::{collections::HashMap, path::Path};

use bevy::math::{Affine3A, IVec2, Quat, Vec2, Vec3, Vec3Swizzles};
use viber::{
    physics::ColliderShape,
    recipes::{
        EntitySpec, parse_world,
        spawn::{PendingTerrain, collect_terrain},
        transform::euler_deg_to_quat,
    },
    terrain::{BrushGrid, features::apply_features, heightmap::HeightMapU16},
};

fn bridge_decks(nodes: &[EntitySpec], parent: Affine3A, decks: &mut Vec<(String, [Vec2; 2])>) {
    for node in nodes {
        let t = &node.transform;
        let rotation = t
            .rotation_quat
            .or_else(|| t.euler_deg.map(euler_deg_to_quat))
            .map_or(Quat::IDENTITY, Quat::from_array);
        let world = parent
            * Affine3A::from_scale_rotation_translation(
                t.scale.into(),
                rotation,
                t.translation.into(),
            );
        if let ColliderShape::Mesh { url, .. } = &node.physics.collider {
            // Metres from the model origin to each end of the walkable deck.
            let span = match url.rsplit('/').next().unwrap() {
                "ponte_norte_collision.glb" => Some(68.0),
                "passadeira_da_bruxa_collision.glb" => Some(32.0),
                "ponte_do_ermo_collision.glb" => Some(36.0),
                _ => None,
            };
            if let Some(span) = span {
                let ends =
                    [-0.5, 0.5].map(|sign| world.transform_point3(Vec3::Z * span * sign).xz());
                decks.push((url.clone(), ends));
            }
        }
        bridge_decks(&node.children, world, decks);
    }
}

#[test]
fn main_world_roads_reach_the_plaza_and_meet_both_ends_of_every_bridge() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let world = viber::xml::include::load_world_in(&root.join("world.bsn"), root).unwrap();
    let scene = parse_world(&world.root_attrs, &world.nodes).unwrap();
    let mut decks = Vec::new();
    bridge_decks(&scene.entities, Affine3A::IDENTITY, &mut decks);
    assert_eq!(decks.len(), 3);
    let mut pending = PendingTerrain::default();
    collect_terrain(&scene.entities, &mut pending);
    let (map, meta) =
        HeightMapU16::from_ahgt(&std::fs::read(root.join("world.data/terrain.ahgt")).unwrap())
            .unwrap();
    let grid = BrushGrid::from_height_map(
        &map,
        meta.world_size.unwrap(),
        meta.max_height.unwrap(),
        1.0,
    )
    .unwrap();
    let features = apply_features(&mut grid.clone(), &pending.features);
    let mut paths = Vec::new();
    for road in features.roads {
        for p in &road.stations {
            let bed = grid.sample(p.x, p.y);
            assert!(
                !features.water.iter().any(|body| {
                    body.sample_domain(*p, Some(bed), 0.0)
                        .is_some_and(|sample| sample.depth > 0.2)
                }),
                "road enters open water at {p:?}"
            );
        }
        paths.push(road.stations);
    }
    let road_count = paths.len();
    let mut links = vec![Vec::new(); paths.len() + decks.len()];
    for (name, [a, b]) in decks {
        let deck_index = paths.len();
        for (end, other_end) in [(a, b), (b, a)] {
            let outward = (end - other_end).normalize();
            let road = paths
                .iter()
                .take(road_count)
                .enumerate()
                .find(|(_, p)| {
                    // Check six metres of straight approach as well as the seam itself.
                    let n = p.len().min(16);
                    let target = end + 6.0 * outward;
                    [(p[0], &p[..n]), (p[p.len() - 1], &p[p.len() - n..])]
                        .iter()
                        .any(|(tip, samples)| {
                            tip.distance(end) <= 0.45
                                && samples.iter().any(|point| point.distance(target) < 0.85)
                        })
                })
                .unwrap_or_else(|| panic!("{name} has no aligned approach at {end:?}"))
                .0;
            links[road].push(deck_index);
            links[deck_index].push(road);
        }
        paths.push(vec![a, b]);
    }
    // Stations are sampled at most one metre apart. Spatial buckets keep
    // endpoint-to-interior T junctions cheap to check, even on the long trails.
    let mut cells: HashMap<IVec2, Vec<(usize, Vec2)>> = HashMap::new();
    let mut plaza = None;
    for (i, road) in paths[..road_count].iter().enumerate() {
        for &p in road {
            if p.length() < 1.0 {
                plaza = Some(i);
            }
            let cell = (p / 2.0).floor().as_ivec2();
            for x in -1..=1 {
                for z in -1..=1 {
                    if let Some(others) = cells.get(&(cell + IVec2::new(x, z))) {
                        for &(j, q) in others {
                            if j != i
                                && p.distance_squared(q) < 1.7_f32.powi(2)
                                && !links[i].contains(&j)
                            {
                                links[i].push(j);
                                links[j].push(i);
                            }
                        }
                    }
                }
            }
            cells.entry(cell).or_default().push((i, p));
        }
    }
    let mut reached = vec![false; paths.len()];
    let mut queue = vec![plaza.expect("Discordia plaza")];
    while let Some(i) = queue.pop() {
        if !reached[i] {
            reached[i] = true;
            queue.extend(links[i].iter().copied());
        }
    }
    for (i, ok) in reached.into_iter().enumerate() {
        assert!(ok, "isolated road or bridge {i} at {:?}", paths[i][0]);
    }
}

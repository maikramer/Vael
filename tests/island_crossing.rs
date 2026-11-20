//! The swimming gate must survive edits to the playable terrain, boat and quest.
use std::{collections::BTreeMap, path::Path};

use bevy::prelude::*;
use vael::{
    boat::BoatsConfig,
    content::VaelContent,
    quest::{QuestKv, QuestRules, quest_accept, quest_turn_in},
    quest_events::scan_visits,
    stamina::{CAPACITY, SWIM_DRAIN, SWIM_SPEED_LIMIT},
    vault::VaultRules,
    xp::XpRules,
};
use vael_engine::{
    recipes::{EntityKind, parse_world},
    terrain::{BrushGrid, heightmap::HeightMapU16, water::carve_lake},
};

#[test]
fn the_island_requires_a_boat_and_the_altar_completes_the_return_trip() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let loaded = vael_engine::xml::include::load_world_in(
        &root.join("world/landmarks/lagos-das-vigilias.bsn"),
        root,
    )
    .unwrap();
    let scene = parse_world(&loaded.root_attrs, &loaded.nodes).unwrap();
    let lake = scene
        .entities
        .iter()
        .find_map(|e| match &e.kind {
            EntityKind::Lake { spec } if !spec.islands.is_empty() => Some(spec),
            _ => None,
        })
        .expect("authored island");
    let (heights, meta) =
        HeightMapU16::from_ahgt(&std::fs::read(root.join("world.data/terrain.ahgt")).unwrap())
            .unwrap();
    let grid = BrushGrid::from_height_map(
        &heights,
        meta.world_size.unwrap(),
        meta.max_height.unwrap(),
        1.0,
    )
    .unwrap();
    let body = carve_lake(&mut grid.clone(), lake, 0).unwrap();
    let depth = |p: Vec2| {
        body.sample_domain(p, Some(grid.sample(p.x, p.y)), 0.0)
            .map_or(0.0, |d| d.depth)
    };

    let centre = lake.islands[0].at;
    // Measure the nearest pair of shallow cells, including diagonal routes
    // between an island headland and a mainland cove. A circular ring would
    // miss these details or reject a perfectly safe asymmetric channel.
    let swim_range = CAPACITY / SWIM_DRAIN * SWIM_SPEED_LIMIT;
    let margin = 8.0; // running jump and a metre-grid sampling allowance
    let split = lake.radius * 0.5;
    let scan = (split + swim_range + margin + 1.0).ceil() as i32;
    let mut island_shallows = Vec::new();
    let mut mainland_shallows = Vec::new();
    for x in -scan..=scan {
        for z in -scan..=scan {
            let offset = Vec2::new(x as f32, z as f32);
            if depth(centre + offset) <= 1.5 {
                if offset.length_squared() < split * split {
                    island_shallows.push(offset);
                } else {
                    mainland_shallows.push(offset);
                }
            }
        }
    }
    assert!(!island_shallows.is_empty() && !mainland_shallows.is_empty());
    assert!(
        island_shallows.iter().all(|a| mainland_shallows
            .iter()
            .all(|b| a.distance_squared(*b) > (swim_range + margin).powi(2))),
        "a shallow-water shortcut reaches the island"
    );
    let boats = BoatsConfig::load(root.join("data/boats.json"));
    let boat = boats
        .boats
        .iter()
        .find(|b| b.name == "barco-das-vigilias")
        .unwrap();
    let mooring = Vec2::new(boat.x, boat.z);
    assert!(depth(mooring) > 0.4, "boat must spawn afloat");
    assert!(
        (0..360).any(|a| depth(mooring + Vec2::from_angle((a as f32).to_radians()) * 3.0) == 0.0),
        "boat reachable from shore"
    );
    // The approach from the ferryman's boat to the island beach is navigable in both directions.
    let direction = (mooring - centre).normalize();
    let island = &lake.islands[0];
    let coast = island.shape.expect("authored island coast");
    let landing =
        centre + direction * (coast.contour(island.radius, direction.y.atan2(direction.x)) + 3.0);
    for step in 0..=100 {
        assert!(depth(mooring.lerp(landing, step as f32 / 100.0)) > 0.3);
    }

    let content = VaelContent::load(root);
    let id = "ilha_das_vigilias";
    let quest = &content.quests[id];
    let altar = scene
        .entities
        .iter()
        .find(|e| e.name.as_deref() == Some(&quest.objective.target))
        .unwrap();
    let [x, _, z] = altar.transform.translation;
    assert_eq!(depth(Vec2::new(x, z)), 0.0);
    let targets = BTreeMap::from([(quest.objective.target.clone(), [x as f64, z as f64])]);
    let rules = QuestRules::from_content(&content);
    let mut kv = QuestKv::new();
    assert!(quest_accept(&rules, &mut kv, id));
    assert!(scan_visits(&rules, &mut kv, [boat.x as f64, boat.z as f64], &targets).is_empty());
    assert_eq!(
        scan_visits(&rules, &mut kv, [x as f64, z as f64], &targets).len(),
        1
    );
    let vault = VaultRules::from_items(&content.items);
    let xp = XpRules::from_progression(&content.progression);
    assert!(quest_turn_in(&rules, &vault, &xp, &mut kv, id).ok);
    assert!(!quest_turn_in(&rules, &vault, &xp, &mut kv, id).ok);
}

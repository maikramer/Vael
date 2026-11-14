//! Applies geomorphological shoreline erosion and biome-aware shore rock
//! accumulation to Vael's static world (`world.data/terrain.ahgt`,
//! `world.data/veg/*.vinst`, and `world/spawn/shore-rocks.bsn`).
//!
//! Run with `--dry-run` to inspect terrain bounds, protected landmarks/bridges,
//! and proposed counts without modifying files, or `--apply` to write updates.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use bevy::math::{EulerRot, Quat, Vec2, Vec3};
use bevy::transform::components::{GlobalTransform, Transform};
use bevy_ext_world::PlacedInstance;
use bevy_ext_world::vinst;
use viber::app::load_world;
use viber::recipes::spawn::{PendingTerrain, collect_terrain};
use viber::recipes::{EntityKind, EntitySpec, Shape, TransformSpec};
use viber::terrain::brush::BrushGrid;
use viber::terrain::features::apply_features;
use viber::terrain::heightmap::{AhgtMeta, HeightMapU16};
use viber::terrain::roads::RoadPath;
use viber::terrain::shore_rocks::{
    ShoreRockPlacement, ShoreRocksSpec, lake_rock_clusters, river_rock_clusters,
};
use viber::terrain::splat::{Biome, BiomeField};
use viber::terrain::voxel::paths::nearest_on_path;
use viber::terrain::water::{
    LakeSpec, RiverSpec, WaterBody, WaterKind, bank_erosion_fbm, bank_erosion_rill,
    river_outer_bias, station_curvatures,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    DryRun,
    Apply,
}

fn main() -> Result<()> {
    let mode = match std::env::args().nth(1).as_deref() {
        Some("--dry-run") => Mode::DryRun,
        Some("--apply") => Mode::Apply,
        _ => bail!("usage: shore_realism <--dry-run|--apply>"),
    };

    let world_bsn = Path::new("world.bsn");
    let ahgt_path = Path::new("world.data/terrain.ahgt");

    let parsed = load_world(world_bsn).context("load world.bsn")?;
    let mut pending = PendingTerrain::default();
    collect_terrain(&parsed.entities, &mut pending);
    let terrain_spec = pending
        .terrain
        .clone()
        .context("missing <Terrain> in world.bsn")?;

    let ahgt_bytes = std::fs::read(ahgt_path).context("read world.data/terrain.ahgt")?;
    let (hm, meta) = HeightMapU16::from_ahgt(&ahgt_bytes).context("parse terrain.ahgt")?;
    let world_size = if terrain_spec.extent_authored {
        terrain_spec.world_size
    } else {
        meta.world_size.unwrap_or(terrain_spec.world_size)
    };
    let max_height = if terrain_spec.extent_authored {
        terrain_spec.max_height
    } else {
        meta.max_height.unwrap_or(terrain_spec.max_height)
    };
    let smoothing = terrain_spec.height_smoothing;

    println!(
        "Terrain: {}x{} world_size={:.1} max_height={:.1} texel={:.3}m smoothing={:.2}",
        hm.width,
        hm.depth,
        world_size,
        max_height,
        world_size / (hm.width.max(2) - 1) as f32,
        smoothing,
    );
    println!("Biomes: wedges={} core={:.1} blend={:.1}", meta.biomes.wedges.len(), meta.biomes.core, meta.biomes.blend);

    let orig_grid = BrushGrid::from_height_map(&hm, world_size, max_height, smoothing)?;
    let mut sim_grid = orig_grid.clone();
    let feat_result = apply_features(&mut sim_grid, &pending.features);

    println!(
        "Features: {} lakes, {} rivers, {} roads, {} pads, {} caves, {} arches, {} bridges, {} cliffs",
        pending.features.lakes.len(),
        pending.features.rivers.len(),
        feat_result.roads.len(),
        feat_result.pads.len(),
        pending.features.caves.len(),
        pending.features.arches.len(),
        pending.features.bridges.len(),
        pending.features.cliffs.len(),
    );

    let protect = ProtectField::build(&parsed.entities, &pending, &feat_result);
    println!(
        "Protection zones: {} pads, {} roads, {} voxel-landforms, {} entity-circles, {} entity-boxes, {} exclusions, {} existing-rocks",
        protect.pads.len(),
        protect.roads.len(),
        protect.polylines.len(),
        protect.circles.len(),
        protect.boxes.len(),
        protect.exclusions.len(),
        protect.existing_rocks.len(),
    );

    report_nearby_objects(&feat_result.water, &protect);

    let mut work_grid = orig_grid.clone();
    let mut river_cells_touched = 0usize;
    let mut lake_cells_touched = 0usize;

    // Iterate parallel indices cleanly: `feat_result.water` aligns 1:1 with `feat_result.water_specs`.
    for (w_idx, &(is_lake, spec_idx)) in feat_result.water_specs.iter().enumerate() {
        let body = &feat_result.water[w_idx];
        if is_lake {
            let spec = &pending.features.lakes[spec_idx];
            if !spec.bank.is_voxel() {
                let touched = erode_lake_banks(
                    &orig_grid,
                    &mut work_grid,
                    spec,
                    body,
                    &protect,
                    &meta.biomes,
                );
                lake_cells_touched += touched;
                println!(
                    "  Lake[{spec_idx}] at ({:.1}, {:.1}) r={:.1} water_y={:.2} biome={:?}: {touched} cells reshaped",
                    spec.at.x,
                    spec.at.y,
                    spec.radius,
                    body.water_y,
                    dominant_biome(&meta.biomes, spec.at),
                );
            }
        } else {
            let spec = &pending.features.rivers[spec_idx];
            if !spec.bank.is_voxel() {
                let touched = erode_river_banks(
                    &orig_grid,
                    &mut work_grid,
                    spec,
                    body,
                    &protect,
                    &meta.biomes,
                );
                river_cells_touched += touched;
                println!(
                    "  River[{spec_idx}] ({} stations, width={:.1}): {touched} cells reshaped",
                    body.stations.len(),
                    spec.width,
                );
            }
        }
    }

    // Verify terrain invariants vs original grid & water continuity.
    let stats = verify_terrain_delta(&orig_grid, &work_grid, &feat_result, &protect)?;
    println!(
        "Height delta summary: river_cells={river_cells_touched} lake_cells={lake_cells_touched} changed={} max_cut={:.3}m max_raise={:.3}m max_protected_delta={:.4}m",
        stats.changed_cells, stats.max_cut, stats.max_raise, stats.max_protected_delta
    );
    if stats.max_protected_delta > 0.002 {
        bail!(
            "protected zone modified by {:.4}m (> 0.002m)",
            stats.max_protected_delta
        );
    }
    println!(
        "  Worst cut @ ({:.1}, {:.1}) & worst raise @ ({:.1}, {:.1})",
        stats.max_cut_pos.x,
        stats.max_cut_pos.y,
        stats.max_raise_pos.x,
        stats.max_raise_pos.y
    );

    // Vegetation `.vinst` re-seating preview / execution.
    let vinst_paths = collect_vinst_paths(&parsed.entities);
    let mut vinst_updates = Vec::with_capacity(vinst_paths.len());
    for path in &vinst_paths {
        let mut instances = vinst::read(path).with_context(|| format!("read {}", path.display()))?;
        let before = instances.len();
        let (adjusted, pruned) = reseat_vinst(&mut instances, &orig_grid, &work_grid, &feat_result.water);
        println!(
            "  Vinst {}: {before} -> {} (adjusted={adjusted}, pruned={pruned})",
            path.display(),
            instances.len(),
        );
        vinst_updates.push((path.clone(), instances));
    }

    // Generate shore rock entities for all water bodies.
    let rocks = generate_shore_rocks(
        &pending,
        &feat_result,
        &work_grid,
        &protect,
        &meta.biomes,
    )?;
    println!("Shore rocks generated: {} static GLB instances", rocks.len());
    let mut by_asset: BTreeMap<&str, usize> = BTreeMap::new();
    let mut with_collider = 0usize;
    let mut half_submerged = 0usize;
    let mut max_float_gap = 0.0_f32;
    for r in &rocks {
        *by_asset.entry(r.asset.id).or_default() += 1;
        if r.has_collider {
            with_collider += 1;
        }
        if r.half_submerged {
            half_submerged += 1;
        }
        let g = work_grid.sample(r.translation.x, r.translation.z);
        max_float_gap = max_float_gap.max(r.translation.y - g);
    }
    println!("  Rock asset breakdown: {by_asset:?} colliders={with_collider} half_submerged={half_submerged} max_y_minus_center_ground={max_float_gap:.3}m");

    if mode == Mode::Apply {
        let updated_hm = HeightMapU16 {
            width: work_grid.width(),
            depth: work_grid.depth(),
            data: work_grid.into_raw(),
        };
        let encoded = updated_hm
            .to_ahgt(&AhgtMeta {
                world_size: Some(world_size),
                max_height: Some(max_height),
                biomes: meta.biomes.clone(),
            })
            .context("encode updated terrain.ahgt")?;
        write_bytes_atomic(ahgt_path, &encoded)?;
        println!("Wrote updated {}", ahgt_path.display());

        for (path, instances) in &vinst_updates {
            vinst::write_atomic(path, instances)?;
        }
        println!("Updated {} .vinst files", vinst_updates.len());

        let bsn_path = Path::new("world/spawn/shore-rocks.bsn");
        let bsn_text = emit_shore_rocks_bsn(&rocks);
        write_bytes_atomic(bsn_path, bsn_text.as_bytes())?;
        println!("Wrote {} ({} bytes)", bsn_path.display(), bsn_text.len());
    } else {
        println!("Dry-run complete. No files modified.");
    }

    Ok(())
}

fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    let tmp_path = Path::new(&tmp);
    std::fs::write(tmp_path, bytes)?;
    std::fs::rename(tmp_path, path)?;
    Ok(())
}

fn dominant_biome(field: &BiomeField, p: Vec2) -> Biome {
    let blend = field.sample(p.x, p.y, 0);
    // Use geometric wedge matching beyond neutral core so biome transitions pick crisp local geology.
    if !field.wedges.is_empty() && p.length() > field.core + field.blend * 0.35 {
        let dir = p.normalize_or_zero();
        if let Some(best) = field
            .wedges
            .iter()
            .max_by(|a, b| dir.dot(a.dir).total_cmp(&dir.dot(b.dir)))
        {
            if dir.dot(best.dir) > 0.30 {
                return best.biome;
            }
        }
    }
    // Fallback via ground channel weights inside or between wedges
    let candidates = [
        (blend.ground[0], Biome::Vale),
        (blend.ground[1], Biome::Forest),
        (blend.ground[2], Biome::Desert),
        (blend.ground[3], Biome::Swamp),
        (blend.ground[4], Biome::Peaks),
    ];
    candidates
        .into_iter()
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|x| x.1)
        .unwrap_or(Biome::Vale)
}

#[derive(Clone)]
struct CircleZone {
    label: String,
    center: Vec2,
    core_r: f32,
    fade_r: f32,
}

#[derive(Clone)]
struct BoxZone {
    _label: String,
    center: Vec2,
    inv_rot: Quat,
    half_xz: Vec2,
    pad_core: f32,
    pad_fade: f32,
}

#[derive(Clone)]
struct PolyZone {
    _label: String,
    points: Vec<Vec2>,
    core_r: f32,
    fade_r: f32,
}

struct ProtectField {
    pads: Vec<(Vec2, Vec2, f32, f32)>,
    roads: Vec<RoadPath>,
    polylines: Vec<PolyZone>,
    circles: Vec<CircleZone>,
    boxes: Vec<BoxZone>,
    exclusions: Vec<(String, Vec2, f32)>,
    existing_rocks: Vec<Vec2>,
}

impl ProtectField {
    fn build(
        entities: &[EntitySpec],
        pending: &PendingTerrain,
        feat_result: &viber::terrain::features::FeatureResult,
    ) -> Self {
        let mut out = Self {
            pads: feat_result
                .pads
                .iter()
                .filter_map(|p| {
                    // Preserve village/camp/landmark foundation pads; only exclude a huge clearing pad
                    // whose interior center holds a lake basin (pillars/buildings around its rim still protect via circles/boxes).
                    let wraps_lake = feat_result.water.iter().any(|w| {
                        w.kind == WaterKind::Lake
                            && w.at.distance(p.at) < 10.0
                            && p.size.x.min(p.size.y) >= 48.0
                    });
                    if wraps_lake {
                        None
                    } else {
                        Some((p.at, p.size * 0.5, p.corner_radius, p.falloff))
                    }
                })
                .collect(),
            roads: feat_result.roads.clone(),
            polylines: Vec::new(),
            circles: Vec::new(),
            boxes: Vec::new(),
            exclusions: Vec::new(),
            existing_rocks: Vec::new(),
        };

        for (i, c) in pending.features.caves.iter().enumerate() {
            let max_r = c.radius.iter().copied().fold(3.0_f32, f32::max);
            if !c.path.is_empty() {
                out.polylines.push(PolyZone {
                    _label: format!("cave[{i}]"),
                    points: c.path.clone(),
                    core_r: max_r + 6.0,
                    fade_r: max_r + 12.0,
                });
            }
        }
        for (i, a) in pending.features.arches.iter().enumerate() {
            let pts = if a.path.is_empty() {
                vec![a.at]
            } else {
                a.path.clone()
            };
            out.polylines.push(PolyZone {
                _label: format!("arch[{i}]"),
                points: pts,
                core_r: 7.5,
                fade_r: 13.0,
            });
        }
        for (i, b) in pending.features.bridges.iter().enumerate() {
            if !b.path.is_empty() {
                out.polylines.push(PolyZone {
                    _label: format!("voxel_bridge[{i}]"),
                    points: b.path.clone(),
                    core_r: b.width * 0.5 + 4.0,
                    fade_r: b.width * 0.5 + 9.0,
                });
            }
        }
        for (i, c) in pending.features.cliffs.iter().enumerate() {
            if !c.path.is_empty() {
                out.polylines.push(PolyZone {
                    _label: format!("cliff[{i}]"),
                    points: c.path.clone(),
                    core_r: c.width.max(4.0) + 4.0,
                    fade_r: c.width.max(4.0) + 10.0,
                });
            }
        }

        walk_entities(entities, GlobalTransform::IDENTITY, &feat_result.water, &mut out);
        out
    }

    fn weight_at(&self, p: Vec2) -> f32 {
        let mut w = 0.0_f32;

        for &(at, half, corner_r, falloff) in &self.pads {
            let sd = rounded_box_sd(p - at, half, corner_r);
            let k = 1.0 - smoothstep(2.0, 2.0 + falloff.max(4.5) + 4.0, sd);
            w = w.max(k);
        }

        for road in &self.roads {
            let (min_b, max_b) = road.bounds;
            if p.x < min_b.x - 8.0
                || p.x > max_b.x + 8.0
                || p.y < min_b.y - 8.0
                || p.y > max_b.y + 8.0
            {
                continue;
            }
            let d = road.distance_to_road(p);
            let k = 1.0 - smoothstep(2.0, 7.0, d);
            w = w.max(k);
        }

        for poly in &self.polylines {
            let d = point_polyline_dist(p, &poly.points);
            if d < poly.fade_r {
                w = w.max(1.0 - smoothstep(poly.core_r, poly.fade_r, d));
            }
        }

        for b in &self.boxes {
            let delta = p - b.center;
            if delta.length_squared() > (b.half_xz.length() + b.pad_fade).powi(2) {
                continue;
            }
            let local3 = b.inv_rot * Vec3::new(delta.x, 0.0, delta.y);
            let sd = box_sd_2d(Vec2::new(local3.x, local3.z), b.half_xz);
            let k = 1.0 - smoothstep(b.pad_core, b.pad_fade, sd);
            w = w.max(k);
        }

        for c in &self.circles {
            let d = p.distance(c.center);
            if d < c.fade_r {
                w = w.max(1.0 - smoothstep(c.core_r, c.fade_r, d));
            }
        }

        // Only protect small margins around pre-existing rocks so they stay seated cleanly.
        for &r in &self.existing_rocks {
            let d = p.distance(r);
            if d < 3.0 {
                w = w.max(1.0 - smoothstep(1.2, 3.0, d));
            }
        }

        w.clamp(0.0, 1.0)
    }

    /// True if placement is blocked for new rocks (checks structural protection,
    /// authored `SpawnExclusion` zones, and existing placed rocks — while allowing
    /// boulders near flexible small vegetation like reeds/grass).
    fn rock_blocked(&self, p: Vec2, extra_radius: f32) -> bool {
        for &(at, half, corner_r, falloff) in &self.pads {
            let sd = rounded_box_sd(p - at, half, corner_r);
            if sd < falloff.max(3.5) + extra_radius {
                return true;
            }
        }
        for road in &self.roads {
            let (min_b, max_b) = road.bounds;
            if p.x >= min_b.x - 6.0 && p.x <= max_b.x + 6.0 && p.y >= min_b.y - 6.0 && p.y <= max_b.y + 6.0
                && road.distance_to_road(p) < 2.8 + extra_radius
            {
                return true;
            }
        }
        for poly in &self.polylines {
            if point_polyline_dist(p, &poly.points) < poly.core_r + extra_radius {
                return true;
            }
        }
        for b in &self.boxes {
            let delta = p - b.center;
            if delta.length_squared() <= (b.half_xz.length() + b.pad_core + extra_radius).powi(2) {
                let local3 = b.inv_rot * Vec3::new(delta.x, 0.0, delta.y);
                let sd = box_sd_2d(Vec2::new(local3.x, local3.z), b.half_xz);
                if sd < b.pad_core + extra_radius {
                    return true;
                }
            }
        }
        for c in &self.circles {
            if c.label.starts_with("small_veg:") {
                continue;
            }
            let min_clear = if c.label.starts_with("flora") {
                1.5 + extra_radius
            } else {
                c.core_r + extra_radius * 0.75
            };
            if p.distance(c.center) < min_clear {
                return true;
            }
        }
        for &r in &self.existing_rocks {
            if p.distance(r) < 2.2 + extra_radius {
                return true;
            }
        }
        for (_, center, radius) in &self.exclusions {
            if p.distance(*center) < *radius + extra_radius {
                return true;
            }
        }
        false
    }

    fn explain_block(&self, p: Vec2, extra_radius: f32) -> Option<String> {
        for (i, &(at, half, corner_r, falloff)) in self.pads.iter().enumerate() {
            let sd = rounded_box_sd(p - at, half, corner_r);
            if sd < falloff.max(3.5) + extra_radius {
                return Some(format!("pad[{i}]"));
            }
        }
        for road in &self.roads {
            if road.distance_to_road(p) < 2.8 + extra_radius {
                return Some(format!("road:{}", road.name.as_deref().unwrap_or("?")));
            }
        }
        for b in &self.boxes {
            let delta = p - b.center;
            let local3 = b.inv_rot * Vec3::new(delta.x, 0.0, delta.y);
            let sd = box_sd_2d(Vec2::new(local3.x, local3.z), b.half_xz);
            if sd < b.pad_core + extra_radius {
                return Some(b._label.clone());
            }
        }
        for c in &self.circles {
            if c.label.starts_with("small_veg:") {
                continue;
            }
            let min_clear = if c.label.starts_with("flora") {
                1.5 + extra_radius
            } else {
                c.core_r + extra_radius * 0.75
            };
            if p.distance(c.center) < min_clear {
                return Some(c.label.clone());
            }
        }
        for poly in &self.polylines {
            if point_polyline_dist(p, &poly.points) < poly.core_r + extra_radius {
                return Some(poly._label.clone());
            }
        }
        for (i, &r) in self.existing_rocks.iter().enumerate() {
            if p.distance(r) < 2.2 + extra_radius {
                return Some(format!("existing_rock[{i}]"));
            }
        }
        for (lbl, center, radius) in &self.exclusions {
            if p.distance(*center) < *radius + extra_radius {
                return Some(format!("excl:{lbl}@({:.0},{:.0})", center.x, center.y));
            }
        }
        Some("other".to_string())
    }
}

fn walk_entities(
    specs: &[EntitySpec],
    parent_gt: GlobalTransform,
    waters: &[WaterBody],
    out: &mut ProtectField,
) {
    for spec in specs {
        let local_t = spec_to_transform(&spec.transform);
        let world_gt = parent_gt.mul_transform(local_t);
        let world_t = world_gt.compute_transform();
        let xz = Vec2::new(world_t.translation.x, world_t.translation.z);
        let label = spec.name.clone().unwrap_or_default();

        match &spec.kind {
            EntityKind::SpawnExclusion { center, radius } => {
                let p_xz = Vec2::new(
                    parent_gt.translation().x + center[0],
                    parent_gt.translation().z + center[1],
                );
                // Lake-centered exclusions exist to keep random tree spawners out of the pond
                // mirror — individual docks/boats/NPCs near those lakes are still protected below.
                let covers_lake_center = waters
                    .iter()
                    .any(|w| w.kind == WaterKind::Lake && p_xz.distance(w.at) < 10.0);
                if !covers_lake_center {
                    out.exclusions.push((label, p_xz, *radius));
                    if *radius >= 10.0
                        && *radius <= 45.0
                        && near_any_water(p_xz, *radius + 45.0, waters)
                    {
                        out.circles.push(CircleZone {
                            label: format!("exclusion({})", spec.name.as_deref().unwrap_or("")),
                            center: p_xz,
                            core_r: *radius * 0.72,
                            fade_r: *radius + 2.0,
                        });
                    }
                }
            }
            EntityKind::Primitive { shape, .. } => {
                if near_any_water(xz, 60.0, waters) {
                    let sx = world_t.scale.x.abs();
                    let sz = world_t.scale.z.abs();
                    let half = match shape {
                        Shape::Cuboid { half_size } => Vec2::new(half_size[0] * sx, half_size[2] * sz),
                        Shape::Plane { half_size } => Vec2::new(half_size[0] * sx, half_size[1] * sz),
                        Shape::Cylinder { radius, .. } | Shape::Capsule { radius, .. } | Shape::Sphere { radius } => {
                            Vec2::splat(*radius * sx.max(sz))
                        }
                    };
                    out.boxes.push(BoxZone {
                        _label: format!("prim:{label}"),
                        center: xz,
                        inv_rot: world_t.rotation.inverse(),
                        half_xz: half,
                        pad_core: 2.2,
                        pad_fade: 5.5,
                    });
                }
            }
            EntityKind::GltfScene { url, .. } => {
                let is_existing_rock = label.starts_with("inst-engine-53-")
                    || url.contains("rock_boulder")
                    || url.contains("rock_mossy")
                    || url.contains("moss_rock")
                    || url.contains("flint_rock");
                if url.contains("lily_pad") || url.contains("water_lily") {
                    // Floating surface foliage doesn't anchor terrain height
                    out.existing_rocks.push(xz);
                } else if is_existing_rock {
                    out.existing_rocks.push(xz);
                } else if near_any_water(xz, 50.0, waters) {
                    let max_s = world_t.scale.x.abs().max(world_t.scale.z.abs()).clamp(0.6, 5.0);
                    let small_veg = url.contains("vegetation/")
                        || url.contains("fern")
                        || url.contains("bush")
                        || url.contains("shrub")
                        || url.contains("flower")
                        || url.contains("reed")
                        || url.contains("grass")
                        || url.contains("mushroom")
                        || url.contains("pebble")
                        || url.contains("moss_");
                    let is_tree = url.contains("tree") || url.contains("pine") || url.contains("oak") || url.contains("birch") || url.contains("willow") || url.contains("cypress");
                    if small_veg {
                        out.circles.push(CircleZone {
                            label: format!("small_veg:{label}"),
                            center: xz,
                            core_r: 0.35 * max_s.min(1.5),
                            fade_r: 1.0 * max_s.min(1.5),
                        });
                    } else if is_tree {
                        out.circles.push(CircleZone {
                            label: format!("flora:{label}"),
                            center: xz,
                            core_r: 1.4,
                            fade_r: 2.8,
                        });
                    } else {
                        let c = 2.0 * max_s.min(2.0);
                        out.circles.push(CircleZone {
                            label: format!("gltf:{label}({url})"),
                            center: xz,
                            core_r: c,
                            fade_r: c + 2.8,
                        });
                    }
                }
            }
            EntityKind::NpcBody | EntityKind::PlayerGltf { .. } | EntityKind::ScenePortal { .. } | EntityKind::SceneEntry => {
                if near_any_water(xz, 50.0, waters) {
                    out.circles.push(CircleZone {
                        label: format!("actor:{label}"),
                        center: xz,
                        core_r: 4.5,
                        fade_r: 8.5,
                    });
                }
            }
            EntityKind::PointLight { .. } | EntityKind::ParticleSystem { .. } | EntityKind::Composition { .. } => {
                if near_any_water(xz, 55.0, waters) {
                    out.circles.push(CircleZone {
                        label: format!("prop:{label}"),
                        center: xz,
                        core_r: 2.4,
                        fade_r: 4.8,
                    });
                }
            }
            EntityKind::Group => {
                let is_flora = spec.props.iter().any(|(k, _)| k == "flora");
                if is_flora && near_any_water(xz, 48.0, waters) {
                    out.circles.push(CircleZone {
                        label: format!("flora:{label}"),
                        center: xz,
                        core_r: 1.5,
                        fade_r: 3.0,
                    });
                }
            }
            _ => {}
        }

        walk_entities(&spec.children, world_gt, waters, out);
    }
}

fn near_any_water(p: Vec2, margin: f32, waters: &[WaterBody]) -> bool {
    for w in waters {
        match w.kind {
            WaterKind::Lake => {
                if p.distance(w.at) <= w.carve_radius.max(w.radius) + margin {
                    return true;
                }
            }
            WaterKind::River => {
                if p.distance(w.at) <= w.hull + margin {
                    return true;
                }
            }
        }
    }
    false
}

fn report_nearby_objects(waters: &[WaterBody], protect: &ProtectField) {
    println!("SpawnExclusions near water (< 60m):");
    for (label, c, r) in &protect.exclusions {
        if near_any_water(*c, *r + 30.0, waters) {
            println!("  excl '{label}' at ({:.1}, {:.1}) r={:.1}", c.x, c.y, r);
        }
    }
    let mut actors = Vec::new();
    for c in &protect.circles {
        if (c.label.starts_with("actor:") || c.label.contains("boat") || c.label.contains("ponte"))
            && near_any_water(c.center, 35.0, waters)
        {
            actors.push(format!("{}@({:.1},{:.1})", c.label, c.center.x, c.center.y));
        }
    }
    println!("Protected actors/landmarks near water ({}): {}", actors.len(), actors.into_iter().take(20).collect::<Vec<_>>().join(", "));
}

fn spec_to_transform(t: &TransformSpec) -> Transform {
    let rot = if let Some([x, y, z, w]) = t.rotation_quat {
        let q = Quat::from_xyzw(x, y, z, w);
        if q.length_squared() > 1e-8 { q.normalize() } else { Quat::IDENTITY }
    } else if let Some([rx, ry, rz]) = t.euler_deg {
        Quat::from_euler(EulerRot::XYZ, rx.to_radians(), ry.to_radians(), rz.to_radians())
    } else {
        Quat::IDENTITY
    };
    Transform {
        translation: Vec3::from(t.translation),
        rotation: rot,
        scale: Vec3::from(t.scale),
    }
}

fn erode_river_banks(
    orig: &BrushGrid,
    grid: &mut BrushGrid,
    spec: &RiverSpec,
    body: &WaterBody,
    protect: &ProtectField,
    biomes: &BiomeField,
) -> usize {
    let n = body.stations.len();
    if n < 2 {
        return 0;
    }
    let curvatures = station_curvatures(&body.stations);
    let bank_w = spec.bank_width.max(4.5);

    let mut min_xz = Vec2::splat(f32::INFINITY);
    let mut max_xz = Vec2::splat(f32::NEG_INFINITY);
    for st in &body.stations {
        min_xz = min_xz.min(*st);
        max_xz = max_xz.max(*st);
    }
    let pad = spec.width * 0.5 + bank_w + 14.0;
    min_xz -= Vec2::splat(pad);
    max_xz += Vec2::splat(pad);

    let (ix0, ix1, iz0, iz1) = grid_range(grid, min_xz, max_xz);
    let mut touched = 0usize;

    for iz in iz0..=iz1 {
        for ix in ix0..=ix1 {
            let p = cell_world(grid, ix, iz);
            let Some(hit) = nearest_on_path(&body.stations, p) else {
                continue;
            };
            let s0 = hit.segment.min(n - 2);
            let tangent = (body.stations[s0 + 1] - body.stations[s0]).normalize_or_zero();
            if tangent == Vec2::ZERO {
                continue;
            }
            let right = Vec2::new(-tangent.y, tangent.x);
            let delta_p = p - hit.point;
            let abs_d = delta_p.length();
            let sign = if delta_p.dot(right) >= 0.0 { 1.0 } else { -1.0 };

            let water_y = lerp_vec(&body.surface_y, hit.segment, hit.t, body.water_y);
            let wl_hw = lerp_vec(&body.half_width, hit.segment, hit.t, spec.width * 0.5).max(1.5);
            let max_reach = wl_hw + bank_w + 10.0;
            if abs_d > max_reach || abs_d < wl_hw * 0.45 {
                continue;
            }

            let prot = protect.weight_at(p);
            if prot >= 0.90 {
                continue;
            }

            let outer_bias = river_outer_bias(&body.stations, &curvatures, &hit, p);
            let outer = outer_bias.max(0.0);
            let inner = (-outer_bias).max(0.0);
            let orig_h = orig.cell_height(ix, iz);
            let mut raw_target = orig_h;

            let fbm_macro = bank_erosion_fbm(p, 11.5);
            let fbm_fine = bank_erosion_fbm(p + Vec2::new(71.3, -43.9), 4.4);
            let bank_outward = right * sign;
            let rill = bank_erosion_rill(p, bank_outward, 7.8);
            let u = abs_d / wl_hw;

            // Near cascade drops, calm bank carving so waterfall lips stay clean.
            let near_cascade = body.cascades.iter().any(|c| {
                body.stations
                    .get(c.lip)
                    .is_some_and(|st| p.distance(*st) < wl_hw + 8.0)
            });
            if near_cascade {
                continue;
            }

            if u < 1.0 {
                // Inside water polygon: deepen outer cutbank scour pool AND create subtle
                // cobble/point-bar tongues right along [0.87..1.0] ONLY on shallow shoals.
                let lobe = fbm_macro * 0.65 + fbm_fine * 0.35;
                if u >= 0.87 && outer_bias < 0.10 && lobe > 0.34 && prot < 0.15 && orig_h >= water_y - 0.45 {
                    let tongue = smoothstep(0.34, 0.78, lobe)
                        * smoothstep(0.87, 0.985, u)
                        * (0.45 + 0.55 * inner);
                    let raise_target = (water_y + 0.14 * tongue).min(orig_h + 0.42);
                    if orig_h < raise_target {
                        raw_target = orig_h + (raise_target - orig_h) * 0.85;
                    }
                } else if u >= 0.52 {
                    let scour = (0.36 * outer + 0.16 * (-lobe).max(0.0))
                        * smoothstep(0.52, 0.92, u)
                        * (1.0 - smoothstep(0.92, 0.995, u));
                    raw_target = orig_h.min((orig_h - scour).min(water_y - 0.10));
                }
            } else {
                // Outside water mesh edge (u >= 1.0): NEVER allow dry height < water_y + 0.05!
                let dist_out = abs_d - wl_hw;
                let reach_out = bank_w * (1.05 + 0.45 * inner - 0.15 * outer) + 7.5;
                if dist_out > reach_out {
                    continue;
                }
                let env_in = smoothstep(-0.4, 1.3, dist_out);
                let env_out = 1.0 - smoothstep(reach_out * 0.48, reach_out, dist_out);
                let env = env_in * env_out;

                // Biome rockiness modulates cutbank bluff prominence.
                let biome = dominant_biome(biomes, p);
                let rock_amp = match biome {
                    Biome::Peaks => 1.25,
                    Biome::Swamp => 0.55,
                    Biome::Desert => 0.95,
                    _ => 1.0,
                };

                // Outer cutbank: steeper raw slump brow + terraced slip + deep stormwater gullies.
                let cut_brow = outer
                    * rock_amp
                    * 1.28
                    * smoothstep(0.15, 2.1, dist_out)
                    * (1.0 - smoothstep(2.4, bank_w * 0.95, dist_out))
                    * (0.65 + 0.35 * fbm_macro);
                let terrace_step = outer
                    * (1.0 - smoothstep(0.6, 4.8, dist_out))
                    * ((dist_out * 2.05 + fbm_macro * 1.8).sin() * 0.16);
                let gully = rill
                    * (0.34 + 0.56 * outer)
                    * rock_amp
                    * (1.0 - smoothstep(1.2, reach_out * 0.85, dist_out));

                // Inner point bar: flatter scrolled cobble/sand shelf sloping softly down toward edge.
                let point_bar = -inner
                    * 0.48
                    * (1.0 - smoothstep(1.0, bank_w * 1.15, dist_out))
                    + inner * ((dist_out * 1.15 - fbm_macro).sin() * 0.07);

                // General macro/micro shoreline undulation + waterline edge pocket scallop
                let edge_pocket = (1.0 - smoothstep(0.4, 3.0, dist_out)) * (0.20 * fbm_fine);
                let organic = 0.28 * fbm_macro + 0.14 * fbm_fine;

                let delta = (cut_brow + terrace_step - gully + point_bar + edge_pocket + organic * env)
                    .clamp(-1.25, 0.95);

                // Never jump submerged fringes up +5m; only protect dry ground against our cuts.
                let min_dry = orig_h.min(water_y + 0.05 + 0.03 * dist_out.clamp(0.0, 2.5));
                raw_target = (orig_h + delta).max(min_dry);
            }

            let target_h = orig_h + (raw_target - orig_h) * (1.0 - prot);
            if (target_h - orig_h).abs() > 0.002 && grid.set_cell_height(ix, iz, target_h) {
                touched += 1;
            }
        }
    }
    touched
}

fn erode_lake_banks(
    orig: &BrushGrid,
    grid: &mut BrushGrid,
    spec: &LakeSpec,
    body: &WaterBody,
    protect: &ProtectField,
    biomes: &BiomeField,
) -> usize {
    let reach = body.mirror_reach.max(0.5);
    let max_contour = spec.radius * 1.35 * reach + 16.0;
    let min_xz = spec.at - Vec2::splat(max_contour);
    let max_xz = spec.at + Vec2::splat(max_contour);
    let (ix0, ix1, iz0, iz1) = grid_range(grid, min_xz, max_xz);

    let shape = &body.shape;
    let biome = dominant_biome(biomes, spec.at);
    let mut touched = 0usize;

    for iz in iz0..=iz1 {
        for ix in ix0..=ix1 {
            let p = cell_world(grid, ix, iz);
            let rel = p - spec.at;
            let dist = rel.length();
            if dist < 1.0 {
                continue;
            }
            let theta = rel.y.atan2(rel.x);
            let r_wl = shape.contour(spec.radius, theta) * reach;
            if dist < r_wl * 0.65 || dist > r_wl + 13.5 {
                continue;
            }

            let prot = protect.weight_at(p);
            if prot >= 0.90 {
                continue;
            }

            let orig_h = orig.cell_height(ix, iz);
            let radial = rel / dist;
            let fbm_macro = bank_erosion_fbm(p, 9.5);
            let fbm_fine = bank_erosion_fbm(p + Vec2::new(-53.7, 91.1), 3.8);
            let rill = bank_erosion_rill(p, radial, 6.8);
            let lobe = fbm_macro * 0.62 + fbm_fine * 0.38;
            let u = dist / r_wl;
            let mut raw_target = orig_h;

            if u < 1.0 {
                // Organic shoal tongues & shallow bog/cobble spits inside [0.87..1.0 * r_wl]
                if u >= 0.87 && lobe > 0.30 && prot < 0.15 && orig_h >= body.water_y - 0.45 {
                    let amp = if biome == Biome::Swamp { 0.16 } else { 0.12 };
                    let tongue = smoothstep(0.30, 0.78, lobe) * smoothstep(0.87, 0.985, u);
                    let raise_target = (body.water_y + amp * tongue).min(orig_h + 0.42);
                    if orig_h < raise_target {
                        raw_target = orig_h + (raise_target - orig_h) * 0.85;
                    }
                } else {
                    // Deepen scalloped coves right inside water edge without ever lifting deep bed
                    let scour = 0.32 * (-lobe).max(0.0)
                        * smoothstep(0.65, 0.92, u)
                        * (1.0 - smoothstep(0.92, 0.995, u));
                    raw_target = orig_h.min((orig_h - scour).min(body.water_y - 0.10));
                }
            } else {
                let dist_out = dist - r_wl;
                let reach_out = 12.0;
                let env = smoothstep(-0.3, 1.2, dist_out)
                    * (1.0 - smoothstep(reach_out * 0.45, reach_out, dist_out));

                let (bluff_amp, rill_amp, hum_amp) = match biome {
                    Biome::Swamp => (0.14, 0.22, 0.18),
                    Biome::Desert => (0.34, 0.56, 0.26),
                    Biome::Peaks => (0.58, 0.50, 0.32),
                    _ => (0.36, 0.42, 0.24),
                };

                // Bluff ridges on convex headlands, eroded gullies on concave draw lines
                let ridge = bluff_amp
                    * lobe.max(0.0)
                    * smoothstep(0.2, 2.5, dist_out)
                    * (1.0 - smoothstep(3.0, reach_out * 0.9, dist_out));
                let gully = rill_amp * rill * (1.0 - smoothstep(1.0, reach_out * 0.8, dist_out));
                let micro = (hum_amp * fbm_macro + hum_amp * 0.45 * fbm_fine) * env;

                let delta = ridge - gully + micro;
                let floor_h = orig_h.min(body.water_y + 0.05 + 0.025 * dist_out.clamp(0.0, 2.5));
                raw_target = (orig_h + delta).max(floor_h);
            }

            let target_h = orig_h + (raw_target - orig_h) * (1.0 - prot);
            if (target_h - orig_h).abs() > 0.002 && grid.set_cell_height(ix, iz, target_h) {
                touched += 1;
            }
        }
    }
    touched
}

struct DeltaStats {
    changed_cells: usize,
    max_cut: f32,
    max_cut_pos: Vec2,
    max_raise: f32,
    max_raise_pos: Vec2,
    max_protected_delta: f32,
}

fn verify_terrain_delta(
    orig: &BrushGrid,
    work: &BrushGrid,
    feat_result: &viber::terrain::features::FeatureResult,
    protect: &ProtectField,
) -> Result<DeltaStats> {
    let mut changed = 0usize;
    let mut max_cut = 0.0_f32;
    let mut max_cut_pos = Vec2::ZERO;
    let mut max_raise = 0.0_f32;
    let mut max_raise_pos = Vec2::ZERO;
    let mut max_prot = 0.0_f32;

    for iz in 0..work.depth() {
        for ix in 0..work.width() {
            let o = orig.cell_height(ix, iz);
            let n = work.cell_height(ix, iz);
            let diff = n - o;
            if diff.abs() > 0.002 {
                changed += 1;
                let p = cell_world(work, ix, iz);
                if diff < 0.0 {
                    if -diff > max_cut {
                        max_cut = -diff;
                        max_cut_pos = p;
                    }
                } else if diff > max_raise {
                    max_raise = diff;
                    max_raise_pos = p;
                }
                if protect.weight_at(p) >= 0.90 {
                    max_prot = max_prot.max(diff.abs());
                }
            }
        }
    }
    let _ = feat_result;
    Ok(DeltaStats {
        changed_cells: changed,
        max_cut,
        max_cut_pos,
        max_raise,
        max_raise_pos,
        max_protected_delta: max_prot,
    })
}

fn collect_vinst_paths(entities: &[EntitySpec]) -> Vec<PathBuf> {
    let mut paths = BTreeSet::new();
    fn walk(specs: &[EntitySpec], paths: &mut BTreeSet<PathBuf>) {
        for s in specs {
            if let EntityKind::Vegetation { spec } = &s.kind {
                if let Some(p) = &spec.instances {
                    paths.insert(PathBuf::from(p));
                }
            }
            walk(&s.children, paths);
        }
    }
    walk(entities, &mut paths);
    paths.into_iter().collect()
}

fn reseat_vinst(
    instances: &mut Vec<PlacedInstance>,
    orig: &BrushGrid,
    work: &BrushGrid,
    waters: &[WaterBody],
) -> (usize, usize) {
    let mut adjusted = 0usize;
    let mut pruned = 0usize;
    instances.retain_mut(|inst| {
        let p = Vec2::new(inst.position.x, inst.position.z);
        let old_h = orig.sample(p.x, p.y);
        let new_h = work.sample(p.x, p.y);
        let delta = new_h - old_h;
        if delta.abs() <= 0.004 {
            return true;
        }
        // Drop grass if newly submerged under water or left hanging on freshly steep cutbank
        for w in waters {
            if let Some(wy) = w.surface_y_at(p) {
                if new_h < wy + 0.06 {
                    pruned += 1;
                    return false;
                }
            }
        }
        let nrm = work.sample_normal(p.x, p.y, 1.0);
        if nrm.y < 0.72 && delta.abs() > 0.03 {
            pruned += 1;
            return false;
        }
        inst.position.y += delta;
        adjusted += 1;
        true
    });
    (adjusted, pruned)
}

#[derive(Clone, Copy)]
struct RockAsset {
    id: &'static str,
    lod0: &'static str,
    lod1: &'static str,
    lod2: &'static str,
    collider: &'static str,
    half_xz: f32,
    height: f32,
}

const ROCK_BOULDER: RockAsset = RockAsset {
    id: "rock_boulder",
    lod0: "/assets/meshes/props/rock_boulder_lod0.glb",
    lod1: "/assets/meshes/props/rock_boulder_lod1.glb",
    lod2: "/assets/meshes/props/rock_boulder_lod2.glb",
    collider: "/assets/meshes/props/rock_boulder_collision.glb",
    half_xz: 0.72,
    height: 1.05,
};
const ROCK_MOSSY: RockAsset = RockAsset {
    id: "rock_mossy",
    lod0: "/assets/meshes/props/rock_mossy_lod0.glb",
    lod1: "/assets/meshes/props/rock_mossy_lod1.glb",
    lod2: "/assets/meshes/props/rock_mossy_lod2.glb",
    collider: "/assets/meshes/props/rock_mossy_collision.glb",
    half_xz: 0.70,
    height: 1.00,
};
const MOSS_ROCK: RockAsset = RockAsset {
    id: "moss_rock",
    lod0: "/assets/meshes/swamp/moss_rock_lod0.glb",
    lod1: "/assets/meshes/swamp/moss_rock_lod1.glb",
    lod2: "/assets/meshes/swamp/moss_rock_lod2.glb",
    collider: "/assets/meshes/swamp/moss_rock_collision.glb",
    half_xz: 0.75,
    height: 1.04,
};
const FLINT_ROCK: RockAsset = RockAsset {
    id: "flint_rock",
    lod0: "/assets/meshes/farm/flint_rock_lod0.glb",
    lod1: "/assets/meshes/farm/flint_rock_lod1.glb",
    lod2: "/assets/meshes/farm/flint_rock_lod2.glb",
    collider: "/assets/meshes/farm/flint_rock_collision.glb",
    half_xz: 0.30,
    height: 0.38,
};
const OUTCROP_2: RockAsset = RockAsset {
    id: "form_outcrop_2",
    lod0: "/assets/meshes/terrain/form_outcrop_2_lod0.glb",
    lod1: "/assets/meshes/terrain/form_outcrop_2_lod1.glb",
    lod2: "/assets/meshes/terrain/form_outcrop_2_lod2.glb",
    collider: "/assets/meshes/terrain/form_outcrop_2_collision.glb",
    half_xz: 1.20,
    height: 1.45,
};
const OUTCROP_5: RockAsset = RockAsset {
    id: "form_outcrop_5",
    lod0: "/assets/meshes/terrain/form_outcrop_5_lod0.glb",
    lod1: "/assets/meshes/terrain/form_outcrop_5_lod1.glb",
    lod2: "/assets/meshes/terrain/form_outcrop_5_lod2.glb",
    collider: "/assets/meshes/terrain/form_outcrop_5_collision.glb",
    half_xz: 1.00,
    height: 2.20,
};
const OUTCROP_8: RockAsset = RockAsset {
    id: "form_outcrop_8",
    lod0: "/assets/meshes/terrain/form_outcrop_8_lod0.glb",
    lod1: "/assets/meshes/terrain/form_outcrop_8_lod1.glb",
    lod2: "/assets/meshes/terrain/form_outcrop_8_lod2.glb",
    collider: "/assets/meshes/terrain/form_outcrop_8_collision.glb",
    half_xz: 1.40,
    height: 1.81,
};

struct PlacedRock {
    asset: RockAsset,
    translation: Vec3,
    rotation: Quat,
    euler_deg: Vec3,
    scale: Vec3,
    has_collider: bool,
    half_submerged: bool,
}

fn generate_shore_rocks(
    pending: &PendingTerrain,
    feat_result: &viber::terrain::features::FeatureResult,
    grid: &BrushGrid,
    protect: &ProtectField,
    biomes: &BiomeField,
) -> Result<Vec<PlacedRock>> {
    let mut raw_candidates: Vec<(ShoreRockPlacement, usize)> = Vec::new();

    for (w_idx, &(is_lake, spec_idx)) in feat_result.water_specs.iter().enumerate() {
        let body = &feat_result.water[w_idx];
        if is_lake {
            let mut spec = pending.features.lakes[spec_idx].clone();
            spec.rocks = true;
            spec.rocks_spec = ShoreRocksSpec {
                density: 0.42,
                scale_max: 1.50,
            };
            for c in lake_rock_clusters(&spec, spec_idx) {
                raw_candidates.push((c, w_idx));
            }
        } else {
            let spec = &pending.features.rivers[spec_idx];
            let rocks_spec = ShoreRocksSpec {
                density: 0.72,
                scale_max: 1.50,
            };
            for c in river_rock_clusters(
                &body.stations,
                spec.width,
                spec.depth,
                spec.water_offset,
                spec.bank_width,
                &rocks_spec,
                spec_idx,
            ) {
                raw_candidates.push((c, w_idx));
            }
        }
    }

    let raw_total = raw_candidates.len();
    let mut r_inner = 0usize;
    let mut r_deep = 0usize;
    let mut r_cascade = 0usize;
    let mut r_pad_road = 0usize;
    let mut r_circle_box = 0usize;
    let mut r_excl = 0usize;
    let mut r_overlap = 0usize;
    let mut blockers: BTreeMap<String, usize> = BTreeMap::new();
    let mut per_body: BTreeMap<usize, usize> = BTreeMap::new();

    let mut out = Vec::with_capacity(raw_candidates.len());
    let mut accepted_xz_r: Vec<(Vec2, f32)> = Vec::with_capacity(raw_candidates.len());

    for (idx, (cand, w_idx)) in raw_candidates.into_iter().enumerate() {
        let p = cand.pos;
        let body = &feat_result.water[w_idx];
        let h = hash_u64(idx as u64, p);

        // Keep sparse small cobbles on gentle inner point bars; concentrate boulders on outer cutbanks.
        if cand.outer_bias < -0.35 && (h & 1) != 0 {
            r_inner += 1;
            continue;
        }

        // Skip cascades & deep submerged channels.
        let water_y = body.surface_y_at(p);
        let center_y = grid.sample(p.x, p.y);
        if let Some(wy) = water_y {
            if wy - center_y > 0.42 {
                r_deep += 1;
                continue;
            }
        }
        if body.cascades.iter().any(|c| {
            body.stations
                .get(c.lip)
                .is_some_and(|st| p.distance(*st) < 9.5)
        }) {
            r_cascade += 1;
            continue;
        }

        let biome = dominant_biome(biomes, p);
        let is_primary = cand.scale_factor >= 0.72;
        let is_cutbank_outcrop = is_primary && cand.outer_bias > 0.34 && (h % 5 == 0);
        let asset = pick_rock_asset(biome, is_primary, is_cutbank_outcrop, h);

        // Scale calibrated per asset native footprint so outcrops stay proportional.
        let base_scale = if asset.id.starts_with("form_outcrop") {
            0.78 + cand.scale_factor * 0.58
        } else if asset.id == "flint_rock" {
            0.85 + cand.scale_factor * 1.15
        } else {
            0.62 + cand.scale_factor * 0.72
        };
        let sx = base_scale * (0.88 + unit_f32(h >> 8) * 0.24);
        let sy = base_scale * (0.80 + unit_f32(h >> 16) * 0.30);
        let sz = base_scale * (0.88 + unit_f32(h >> 24) * 0.24);
        let foot_r = asset.half_xz * sx.max(sz);

        if protect.rock_blocked(p, foot_r + 0.6) {
            if let Some(why) = protect.explain_block(p, foot_r + 0.6) {
                if why.starts_with("pad") || why.starts_with("road") {
                    r_pad_road += 1;
                } else if why.starts_with("excl") {
                    r_excl += 1;
                } else {
                    r_circle_box += 1;
                }
                *blockers.entry(why).or_default() += 1;
            }
            continue;
        }
        // Allow snug natural scree clusters (0.50x summed radii).
        if accepted_xz_r
            .iter()
            .any(|&(prev, pr)| p.distance(prev) < (foot_r + pr) * 0.50)
        {
            r_overlap += 1;
            continue;
        }

        // Footprint-min ground anchor prevents downhill corners from floating on steep cutbanks!
        let sample_r = (foot_r * 0.58).max(0.25);
        let h_n = grid.sample(p.x, p.y - sample_r);
        let h_s = grid.sample(p.x, p.y + sample_r);
        let h_e = grid.sample(p.x + sample_r, p.y);
        let h_w = grid.sample(p.x - sample_r, p.y);
        let h_min = center_y.min(h_n).min(h_s).min(h_e).min(h_w);

        let rock_h = asset.height * sy;
        let embed = (0.16 * rock_h).max(0.08);
        let seat_y = (0.35 * center_y + 0.65 * h_min) - embed;

        // Subtle geological tilt toward terrain normal + natural roll noise.
        let nrm = grid.sample_normal(p.x, p.y, 1.0);
        let up = Vec3::Y;
        let mut tilt = Quat::from_rotation_arc(up, nrm.lerp(up, 0.45).normalize());
        let pitch = (unit_f32(h >> 32) - 0.5) * 0.14;
        let roll = (unit_f32(h >> 40) - 0.5) * 0.14;
        let rot = tilt * Quat::from_euler(EulerRot::YXZ, cand.yaw, pitch, roll);
        tilt = rot.normalize();
        let (rx, ry, rz) = tilt.to_euler(EulerRot::XYZ);

        accepted_xz_r.push((p, foot_r));
        *per_body.entry(w_idx).or_default() += 1;
        out.push(PlacedRock {
            asset,
            translation: Vec3::new(p.x, seat_y, p.y),
            rotation: tilt,
            euler_deg: Vec3::new(rx.to_degrees(), ry.to_degrees(), rz.to_degrees()),
            scale: Vec3::new(sx, sy, sz),
            has_collider: foot_r >= 0.46,
            half_submerged: cand.half_submerged,
        });
    }

    let mut top_b: Vec<_> = blockers.into_iter().collect();
    top_b.sort_by(|a, b| b.1.cmp(&a.1));
    top_b.truncate(12);
    println!(
        "  Rock pipeline: raw={raw_total} accepted={} per_water={per_body:?} rejected[inner={r_inner} deep={r_deep} cascade={r_cascade} pad_road={r_pad_road} circle_box={r_circle_box} excl={r_excl} overlap={r_overlap}] top_blockers={top_b:?}",
        out.len()
    );
    Ok(out)
}

fn pick_rock_asset(
    biome: Biome,
    is_primary: bool,
    is_cutbank_outcrop: bool,
    h: u64,
) -> RockAsset {
    if is_cutbank_outcrop {
        return match biome {
            Biome::Peaks => match h % 3 {
                0 => OUTCROP_5,
                1 => OUTCROP_8,
                _ => OUTCROP_2,
            },
            Biome::Desert => OUTCROP_2,
            Biome::Swamp => MOSS_ROCK,
            _ => match h % 2 {
                0 => OUTCROP_2,
                _ => OUTCROP_5,
            },
        };
    }
    match (biome, is_primary) {
        (Biome::Swamp, true) => {
            if (h % 3) != 0 {
                MOSS_ROCK
            } else {
                ROCK_MOSSY
            }
        }
        (Biome::Swamp, false) => {
            if (h % 2) == 0 {
                FLINT_ROCK
            } else {
                MOSS_ROCK
            }
        }
        (Biome::Desert, true) => match h % 3 {
            0 => FLINT_ROCK,
            1 => ROCK_BOULDER,
            _ => OUTCROP_2,
        },
        (Biome::Desert, false) => FLINT_ROCK,
        (Biome::Peaks, true) => match h % 4 {
            0 => ROCK_BOULDER,
            1 => OUTCROP_2,
            2 => OUTCROP_5,
            _ => OUTCROP_8,
        },
        (Biome::Peaks, false) => {
            if (h % 2) == 0 {
                FLINT_ROCK
            } else {
                ROCK_BOULDER
            }
        }
        (Biome::Vale | Biome::Forest, true) => match h % 3 {
            0 => ROCK_MOSSY,
            1 => ROCK_BOULDER,
            _ => MOSS_ROCK,
        },
        (Biome::Vale | Biome::Forest, false) => match h % 3 {
            0 | 1 => FLINT_ROCK,
            _ => ROCK_MOSSY,
        },
    }
}

fn emit_shore_rocks_bsn(rocks: &[PlacedRock]) -> String {
    let mut s = String::with_capacity(rocks.len() * 680 + 512);
    s.push_str(
        "#\"world\"\n\
bevy_transform::components::transform::Transform {\n\
    translation: glam::Vec3 { x: 0.0, y: 0.0, z: 0.0 },\n\
    rotation: glam::Quat { x: 0.0, y: 0.0, z: 0.0, w: 1.0 },\n\
    scale: glam::Vec3 { x: 1.0, y: 1.0, z: 1.0 },\n\
}\n\
bevy_world_xml::bsn::WorldNode {\n\
    tag: \"world\",\n\
    attrs: [\n\
        bevy_world_xml::bsn::WorldProp { key: \"static\", value: \"true\" },\n\
    ],\n\
    text: \"\",\n\
    origin_file: \"world/spawn/shore-rocks.xml\",\n\
    origin_path: [0],\n\
    pose: true,\n\
}\n\
bevy_ecs::hierarchy::Children [\n\
    #\"shore-rocks\"\n\
    bevy_transform::components::transform::Transform {\n\
        translation: glam::Vec3 { x: 0.0, y: 0.0, z: 0.0 },\n\
        rotation: glam::Quat { x: 0.0, y: 0.0, z: 0.0, w: 1.0 },\n\
        scale: glam::Vec3 { x: 1.0, y: 1.0, z: 1.0 },\n\
    }\n\
    bevy_world_xml::bsn::WorldNode {\n\
        tag: \"Group\",\n\
        attrs: [\n\
            bevy_world_xml::bsn::WorldProp { key: \"name\", value: \"shore-rocks\" },\n\
        ],\n\
        text: \"\",\n\
        origin_file: \"world/spawn/shore-rocks.xml\",\n\
        origin_path: [0],\n\
        pose: false,\n\
    }\n\
    jackdaw_scene_types::node_id::SceneNodeId(6852620029000000000)\n\
    bevy_ecs::hierarchy::Children [\n",
    );

    for (i, r) in rocks.iter().enumerate() {
        if i > 0 {
            s.push_str("        --\n");
        }
        let name = format!("shore-rock-{i:04}");
        let node_id = 6852620029000000001u64 + i as u64;
        let _ = writeln!(s, "        #\"{name}\"");
        let _ = writeln!(
            s,
            "        bevy_transform::components::transform::Transform {{\n\
            translation: glam::Vec3 {{ x: {:.4}, y: {:.4}, z: {:.4} }},\n\
            rotation: glam::Quat {{ x: {:.6}, y: {:.6}, z: {:.6}, w: {:.6} }},\n\
            scale: glam::Vec3 {{ x: {:.4}, y: {:.4}, z: {:.4} }},\n\
        }}",
            r.translation.x,
            r.translation.y,
            r.translation.z,
            r.rotation.x,
            r.rotation.y,
            r.rotation.z,
            r.rotation.w,
            r.scale.x,
            r.scale.y,
            r.scale.z,
        );
        if r.has_collider {
            let _ = writeln!(
                s,
                "        bevy_ext_scene::ColliderSpec {{\n\
                shape: bevy_ext_scene::ColliderShape::Mesh {{ url: \"{}\", base: true }},\n\
            }}\n\
            bevy_ext_scene::BodySpec {{\n\
                kind: bevy_ext_scene::BodyKind::Fixed,\n\
            }}",
                r.asset.collider
            );
        }
        let _ = writeln!(
            s,
            "        bevy_ext_scene::ModelLod {{\n\
            lod1: core::option::Option::Some(\"{}\"),\n\
            lod2: core::option::Option::Some(\"{}\"),\n\
            near: core::option::Option::Some(50.0),\n\
            mid: core::option::Option::Some(120.0),\n\
        }}\n\
        bevy_world_xml::bsn::WorldNode {{\n\
            tag: \"GltfScene\",\n\
            attrs: [\n\
                bevy_world_xml::bsn::WorldProp {{ key: \"url\", value: \"{}\" }},\n\
                bevy_world_xml::bsn::WorldProp {{ key: \"name\", value: \"{name}\" }},\n\
                bevy_world_xml::bsn::WorldProp {{ key: \"translation\", value: \"{:.4} {:.4} {:.4}\" }},\n\
                bevy_world_xml::bsn::WorldProp {{ key: \"euler\", value: \"{:.3} {:.3} {:.3}\" }},\n\
                bevy_world_xml::bsn::WorldProp {{ key: \"scale\", value: \"{:.4} {:.4} {:.4}\" }},\n\
            ],\n\
            text: \"\",\n\
            origin_file: \"world/spawn/shore-rocks.xml\",\n\
            origin_path: [0, {i}],\n\
            pose: true,\n\
        }}\n\
        jackdaw_scene_types::node_id::SceneNodeId({node_id})",
            r.asset.lod1,
            r.asset.lod2,
            r.asset.lod0,
            r.translation.x,
            r.translation.y,
            r.translation.z,
            r.euler_deg.x,
            r.euler_deg.y,
            r.euler_deg.z,
            r.scale.x,
            r.scale.y,
            r.scale.z,
        );
    }

    s.push_str("    ]\n]\n");
    s
}

fn cell_world(grid: &BrushGrid, ix: usize, iz: usize) -> Vec2 {
    let half = grid.world_size() * 0.5;
    let sx = grid.world_size() / (grid.width().max(2) - 1) as f32;
    let sz = grid.world_size() / (grid.depth().max(2) - 1) as f32;
    Vec2::new(ix as f32 * sx - half, iz as f32 * sz - half)
}

fn grid_range(grid: &BrushGrid, min: Vec2, max: Vec2) -> (usize, usize, usize, usize) {
    let half = grid.world_size() * 0.5;
    let sx = grid.world_size() / (grid.width().max(2) - 1) as f32;
    let sz = grid.world_size() / (grid.depth().max(2) - 1) as f32;
    let ix0 = (((min.x + half) / sx).floor() as isize).clamp(0, grid.width() as isize - 1) as usize;
    let ix1 = (((max.x + half) / sx).ceil() as isize).clamp(0, grid.width() as isize - 1) as usize;
    let iz0 = (((min.y + half) / sz).floor() as isize).clamp(0, grid.depth() as isize - 1) as usize;
    let iz1 = (((max.y + half) / sz).ceil() as isize).clamp(0, grid.depth() as isize - 1) as usize;
    (ix0, ix1, iz0, iz1)
}

fn lerp_vec(v: &[f32], seg: usize, t: f32, fallback: f32) -> f32 {
    if v.is_empty() {
        return fallback;
    }
    let a = v[seg.min(v.len() - 1)];
    let b = v[(seg + 1).min(v.len() - 1)];
    a + (b - a) * t
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0).max(1e-5)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn rounded_box_sd(p: Vec2, half: Vec2, corner_r: f32) -> f32 {
    let r = corner_r.min(half.x).min(half.y).max(0.0);
    let q = p.abs() - (half - Vec2::splat(r));
    q.max(Vec2::ZERO).length() + q.x.max(q.y).min(0.0) - r
}

fn box_sd_2d(p: Vec2, half: Vec2) -> f32 {
    let q = p.abs() - half;
    q.max(Vec2::ZERO).length() + q.x.max(q.y).min(0.0)
}

fn point_polyline_dist(p: Vec2, pts: &[Vec2]) -> f32 {
    if pts.is_empty() {
        return f32::INFINITY;
    }
    if pts.len() == 1 {
        return p.distance(pts[0]);
    }
    let mut best = f32::INFINITY;
    for w in pts.windows(2) {
        let a = w[0];
        let b = w[1];
        let ab = b - a;
        let denom = ab.length_squared();
        let proj = if denom > 1e-6 {
            ((p - a).dot(ab) / denom).clamp(0.0, 1.0)
        } else {
            0.0
        };
        best = best.min(p.distance(a + ab * proj));
    }
    best
}

fn hash_u64(i: u64, p: Vec2) -> u64 {
    let mut x = i
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (p.x.to_bits() as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9)
        ^ (p.y.to_bits() as u64).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

fn unit_f32(h: u64) -> f32 {
    ((h & 0xFFFF) as f32) / 65535.0
}


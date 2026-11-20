//! Quarry stones share seeded geometry, collision and surface maps across the world.
mod crystal;
pub(crate) use crystal::CrystalGrowth;
mod restore;
pub(crate) use restore::reload;
pub(crate) mod snow;

use std::collections::BTreeMap;
use std::f32::consts::{FRAC_PI_2, PI, TAU};

use avian3d::prelude::{Collider, RigidBody};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy_elements::{SurfaceWeathering, Weather, WorldClock};
use vael_engine::luau::ScriptProps;

const VARIANTS: u32 = 24;
const WIDTH: usize = 192;
const HEIGHT: usize = 96;
const MOSS_KEY: &str = "quarry.moss";
const INITIAL_MOSS: f32 = 0.48;

pub struct QuarryPlugin;

impl Plugin for QuarryPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Quarry>()
            .register_type::<QuarryStone>()
            .register_type::<crystal::CrystalGrowth>()
            .add_observer(restore::remember)
            .add_systems(
                vael_engine::scene_loader::SceneLoad,
                (|mut q: ResMut<Quarry>| q.removed.clear()).before(vael_engine::recipes::spawn::startup),
            )
            .add_systems(Update, (place, crystal::grow, weather_stone).chain());
    }
}

#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct QuarryStone {
    pub(crate) seed: u32,
    pub(crate) variant: u32,
    pub(crate) segments: u8,
}

#[derive(Resource, Default)]
struct Quarry {
    removed: Vec<restore::Deposit>,
    variants: BTreeMap<u32, Variant>,
    last_hour: Option<f64>,
    next_sample: f32,
}

struct Variant {
    mesh: Handle<Mesh>,
    hull: Collider,
    surface: Collider,
    material: Handle<StandardMaterial>,
    texture: Handle<Image>,
    pixels: Vec<Texel>,
    coverage: u8,
}

fn hash(mut n: u32) -> u32 {
    n ^= n >> 16;
    n = n.wrapping_mul(0x7feb352d);
    n ^= n >> 15;
    n = n.wrapping_mul(0x846ca68b);
    n ^ (n >> 16)
}

pub(crate) fn random(n: u32) -> f32 {
    (hash(n) >> 8) as f32 / 16777216.
}

fn noise(p: Vec3) -> f32 {
    let i = p.floor().as_ivec3().as_uvec3();
    let f = p - p.floor();
    let w = f * f * (Vec3::splat(3.) - 2. * f);
    let cell =
        i.x.wrapping_add(i.y.wrapping_mul(1987))
            .wrapping_add(i.z.wrapping_mul(65537));
    let plane = |d: u32| {
        let row = |j| {
            random(cell.wrapping_add(d).wrapping_add(j)).lerp(
                random(cell.wrapping_add(d).wrapping_add(j).wrapping_add(1)),
                w.x,
            )
        };
        row(0).lerp(row(1987), w.y)
    };
    plane(0).lerp(plane(65537), w.z)
}

/// A weathered block clipped by oblique fracture planes. Each family has its own bedding.
struct Shape {
    planes: Vec<(Vec3, f32)>,
    axes: Vec3,
}

impl Shape {
    fn new(id: u32) -> Self {
        let rand = |i| random(id * 149 + i);
        let mut planes = Vec::new();
        for i in 0..7 {
            let a = TAU * (i as f32 + rand(i) * 0.45) / 7.;
            planes.push((
                Vec3::new(a.cos(), rand(i + 8) * 0.85 - 0.32, a.sin()).normalize(),
                0.76 + 0.21 * rand(19 + i),
            ));
        }
        for i in 0..5 {
            let a = TAU * (i as f32 + rand(i + 33)) / 5.;
            planes.push((
                Vec3::new(a.cos(), 0.85 + rand(i + 47), a.sin()).normalize(),
                0.72 + 0.23 * rand(i + 60),
            ));
        }
        planes.push((
            Vec3::new(rand(74) * 0.5 - 0.25, 1., rand(75) * 0.4 - 0.2).normalize(),
            0.75,
        ));
        planes.push((Vec3::NEG_Y, 0.67));
        let axes = match id % 3 {
            0 => Vec3::new(0.89, 0.51, 0.66), // bedded slate
            1 => Vec3::new(0.73, 0.76, 0.73), // jointed granite
            _ => Vec3::new(0.82, 0.66, 0.63), // sandstone
        };
        Self { planes, axes }
    }

    fn point(&self, d: Vec3) -> Vec3 {
        let cut = self
            .planes
            .iter()
            .fold(1.0_f32, |r, (n, b)| r.min(b / d.dot(*n).max(0.01)));
        (d * (cut * 0.92 + 0.08) + Vec3::Y * 0.69) * self.axes
    }

    fn geometry(&self, rotation: Quat) -> (Mesh, Collider, Collider) {
        let mut mesh = Sphere::new(1.).mesh().uv(32, 18);
        let Some(VertexAttributeValues::Float32x3(directions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            unreachable!()
        };
        let mut positions = Vec::with_capacity(directions.len());
        let mut normals = Vec::with_capacity(directions.len());
        for dir in directions {
            let d = Vec3::from_array(*dir);
            let u = d.any_orthonormal_vector();
            let v = d.cross(u);
            let tangent = |axis: Vec3| {
                self.point((d + axis * 0.045).normalize())
                    - self.point((d - axis * 0.045).normalize())
            };
            positions.push(rotation * self.point(d));
            normals.push((rotation * tangent(u).cross(tangent(v)).normalize_or(d)).to_array());
        }
        let surface = Collider::convex_hull(&positions).expect("solid quarry profile");
        // As facetas superiores da pedra são chanfradas (~20°–45°) e a base
        // recolhe para dentro, o que deixava o hemisfério inferior da cápsula
        // do herói (centro a y = 0.39 m) e o probe_steps (y <= 0.40 m)
        // subirem pela lateral e atravessarem a malha. Extrudimos o contorno
        // horizontal visual como coluna vertical de y = 0 até acima do limiar
        // de degrau/cápsula (mesmo em instâncias escaladas a ~0.7).
        let top_y = positions.iter().map(|p| p.y).fold(0.0_f32, f32::max);
        let wall_top = top_y.max(0.62);
        let mut hull_points = Vec::with_capacity(positions.len() * 2);
        for &p in &positions {
            hull_points.push(Vec3::new(p.x, 0.0, p.z));
            hull_points.push(Vec3::new(p.x, wall_top, p.z));
        }
        let hull = Collider::convex_hull(&hull_points).expect("solid quarry column");
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            positions
                .into_iter()
                .map(|p| p.to_array())
                .collect::<Vec<_>>(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
        (mesh, hull, surface)
    }
}

fn segment_count(id: u32) -> u8 {
    match id % 5 {
        0 | 1 => 1,
        2 | 3 => 2,
        _ => 3,
    }
}

/// Seat successively smaller blocks into the flank. Keep their convex pieces
/// separate in physics so the indentations between blocks remain hittable.
fn formation(id: u32) -> (Mesh, Collider, Collider) {
    let (mut mesh, main, main_surface) = Shape::new(id).geometry(Quat::IDENTITY);
    let count = segment_count(id);
    if count == 1 {
        return (mesh, main, main_surface);
    }
    let mut pieces = vec![(Vec3::ZERO, Quat::IDENTITY, main)];
    let mut surfaces = vec![(Vec3::ZERO, Quat::IDENTITY, main_surface)];
    let heading = TAU * random(id + 808);
    for i in 1..u32::from(count) {
        let a = heading + (i - 1) as f32 * (1.55 + 0.65 * random(id + 99));
        let mut shape = Shape::new(id + i * 24);
        let scale = if i == 1 { 0.73 } else { 0.54 };
        shape.axes *= Vec3::new(scale, scale * (0.82 + 0.34 * random(id + i)), scale);
        let offset = Vec3::new(a.cos(), 0., a.sin()) * (0.54 + 0.12 * random(id + i * 175));
        let rotation = Quat::from_rotation_y(a + 1.3);
        let (block, hull, surface) = shape.geometry(rotation);
        mesh.merge(&block.transformed_by(Transform::from_translation(offset)))
            .expect("quarry segments share a vertex format");
        pieces.push((offset, Quat::IDENTITY, hull));
        surfaces.push((offset, Quat::IDENTITY, surface));
    }
    (mesh, Collider::compound(pieces), Collider::compound(surfaces))
}

#[derive(Clone, Copy)]
struct Texel {
    stone: [u8; 3],
    moss: [u8; 3],
    threshold: f32,
}

impl Texel {
    fn at(dir: Vec3, id: u32) -> Self {
        let p = dir + Vec3::splat(id as f32 * 3.73);
        let broad = noise(p * 2.8);
        let grain = noise(p * 74.);
        let detail = noise(p * 12.);
        let bed = dir.y + dir.x * 0.19 - dir.z * 0.28 + broad * 0.23 + detail * 0.035;
        let vein = (1. - (bed * 12. + id as f32).sin().abs() * 7.).max(0.);
        let (base, highlight) = match id % 3 {
            0 => ([86., 96., 105.], 25. * vein + 7. * (bed * 65.).sin()),
            1 => ([128., 119., 114.], 39. * (grain - 0.48) + 22. * vein),
            _ => ([146., 114., 85.], 15. * (bed * 31.).sin() + 13. * vein),
        };
        // Weathered joints follow bedding and taper into the grain on exposed faces.
        let joint_plane = dir.x
            + (random(id + 84) - 0.5) * dir.z
            + 0.42 * dir.y
            + (broad - 0.5) * 0.58
            + (detail - 0.5) * 0.11;
        let joint = (1. - (joint_plane - 0.22).abs() / 0.025).clamp(0., 1.);
        let fork = (1. - (joint_plane - 0.22 + 0.68 * (bed - 0.14)).abs() / 0.016).clamp(0., 1.)
            * (1. - ((bed - 0.30) / 0.16).abs()).max(0.);
        let fissure = joint.max(fork) * ((broad - 0.28) * 4.).clamp(0., 1.);
        let shade = -47. * fissure
            + 40. * (broad - 0.5)
            + 25. * (detail - 0.5)
            + 38. * (grain - 0.5)
            + highlight;
        let green = 49. * (detail - 0.5) + 44. * (grain - 0.5);
        Self {
            stone: base.map(|b| (b + shade) as u8),
            moss: [78. + green, 91. + green, 47. + green * 0.6].map(|b| b as u8),
            threshold: 0.60 + (broad - 0.5) * 1.05 + (detail - 0.5) * 0.46 + (grain - 0.5) * 0.23
                - 0.16 * dir.z
                - 0.10 * dir.y,
        }
    }

    fn paint(&self, coverage: u8, rgba: &mut [u8]) {
        let m = ((coverage as f32 / 100. - self.threshold) * 18. + 0.5).clamp(0., 1.);
        for (i, channel) in rgba[..3].iter_mut().enumerate() {
            *channel = (self.stone[i] as f32).lerp(self.moss[i] as f32, m) as u8;
        }
        rgba[3] = 255;
    }
}

impl Variant {
    fn new(
        id: u32,
        meshes: &mut Assets<Mesh>,
        images: &mut Assets<Image>,
        materials: &mut Assets<StandardMaterial>,
    ) -> Self {
        let (mesh, hull, surface) = formation(id);
        let mut pixels = Vec::with_capacity(WIDTH * HEIGHT);
        let coverage = (INITIAL_MOSS * 100.) as u8;
        let mut data = vec![0; WIDTH * HEIGHT * 4];
        for y in 0..HEIGHT {
            let t = FRAC_PI_2 - PI * (y as f32 + 0.5) / HEIGHT as f32;
            for x in 0..WIDTH {
                let a = TAU * (x as f32 + 0.5) / WIDTH as f32;
                let texel = Texel::at(Vec3::new(t.cos() * a.cos(), t.cos() * a.sin(), t.sin()), id);
                let i = pixels.len() * 4;
                texel.paint(coverage, &mut data[i..i + 4]);
                pixels.push(texel);
            }
        }
        let texture = images.add(Image::new(
            Extent3d {
                width: WIDTH as u32,
                height: HEIGHT as u32,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            data,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        ));
        let material = materials.add(StandardMaterial {
            base_color_texture: Some(texture.clone()),
            perceptual_roughness: 0.94,
            ..default()
        });
        Self {
            mesh: meshes.add(mesh),
            hull,
            surface,
            material,
            texture,
            pixels,
            coverage,
        }
    }
}

fn is_deposit(props: &ScriptProps) -> bool {
    props.0.iter().any(|(k, v)| k == "outcrop" && v == "stone")
}

fn place(
    mut commands: Commands,
    added: Query<(Entity, &Name, &ScriptProps), (Changed<ScriptProps>, Without<QuarryStone>)>,
    mut quarry: ResMut<Quarry>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut crystals: Local<crystal::Palette>,
) {
    for (entity, name, props) in &added {
        if !is_deposit(props) {
            continue;
        }
        let seed = name.as_bytes().iter().fold(2166136261u32, |s, b| {
            (s ^ u32::from(*b)).wrapping_mul(16777619)
        });
        let id = hash(seed) % VARIANTS;
        let rock = quarry
            .variants
            .entry(id)
            .or_insert_with(|| Variant::new(id, &mut meshes, &mut images, &mut materials));
        commands.entity(entity).insert((
            QuarryStone {
                seed,
                variant: id,
                segments: segment_count(id),
            },
            Mesh3d(rock.mesh.clone()),
            MeshMaterial3d(rock.material.clone()),
            RigidBody::Static,
            rock.hull.clone(),
        ));
        if crystal::mineralized(seed) {
            crystals.attach(
                &mut commands,
                entity,
                seed,
                &rock.surface,
                &mut meshes,
                &mut materials,
            );
        }
    }
}

fn grow(coverage: f32, wetness: f32, temperature: f32, hours: f32) -> f32 {
    if temperature <= 1. {
        return coverage;
    }
    let wet = wetness.clamp(0., 1.);
    let goal = 0.30 + 0.48 * wet;
    let rate = if goal > coverage { 0.052 } else { 0.012 };
    coverage.lerp(goal, 1. - (-hours * rate).exp())
}

fn weather_stone(
    time: Res<Time>,
    clock: Option<Res<WorldClock>>,
    weather: Option<Res<Weather>>,
    surface: Option<Res<SurfaceWeathering>>,
    mut quarry: ResMut<Quarry>,
    mut images: ResMut<Assets<Image>>,
) {
    let (Some(clock), Some(weather), Some(surface)) = (clock, weather, surface) else {
        return;
    };
    if quarry.variants.is_empty() {
        return;
    }
    // Store one habitat value in the normal world save. A restored slot also restores its moss.
    let now = time.elapsed_secs();
    if now < quarry.next_sample {
        return;
    }
    quarry.next_sample = now + 1.;
    let hour = clock.hours_since_2000();
    let elapsed = (hour - quarry.last_hour.replace(hour).unwrap_or(hour)).clamp(0., 24.) as f32;
    let coverage = vael_engine::luau::game::game_get_json(MOSS_KEY)
        .and_then(|v| v.as_f64())
        .unwrap_or(INITIAL_MOSS as f64)
        .clamp(0., 1.) as f32;
    let grown = grow(coverage, surface.wetness, weather.temperature_c, elapsed);
    if (grown - coverage).abs() > f32::EPSILON {
        vael_engine::luau::game::apply_patch_json(&BTreeMap::from([(
            MOSS_KEY.to_string(),
            serde_json::json!(grown),
        )]));
    }
    let level = (grown * 50.).round() as u8 * 2;
    for variant in quarry
        .variants
        .values_mut()
        .filter(|v| v.coverage != level)
        .take(4)
    {
        if let Some(mut img) = images.get_mut(&variant.texture)
            && let Some(bytes) = img.data.as_mut()
        {
            for (texel, dest) in variant.pixels.iter().zip(bytes.chunks_exact_mut(4)) {
                texel.paint(level, dest);
            }
            variant.coverage = level;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_profiles_stand_on_their_authored_base_and_have_valid_normals() {
        for id in 0..VARIANTS {
            let (mesh, _, _) = Shape::new(id).geometry(Quat::IDENTITY);
            let Some(VertexAttributeValues::Float32x3(p)) =
                mesh.attribute(Mesh::ATTRIBUTE_POSITION)
            else {
                panic!()
            };
            let bottom = p.iter().map(|v| v[1]).fold(f32::INFINITY, f32::min);
            assert!(bottom.abs() < 0.025);
            let Some(VertexAttributeValues::Float32x3(n)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL)
            else {
                panic!()
            };
            assert!(n.iter().all(|v| Vec3::from_array(*v).is_normalized()));
        }
    }
    #[test]
    fn collision_and_mesh_keep_the_same_individual_segments() {
        for id in 0..VARIANTS {
            let (mesh, hull, _) = formation(id);
            let count = usize::from(segment_count(id));
            let base = Shape::new(id).geometry(Quat::IDENTITY).0.count_vertices();
            assert_eq!(mesh.count_vertices(), base * count);
            assert_eq!(
                hull.shape().as_compound().map_or(1, |c| c.shapes().len()),
                count
            );
            let Some(VertexAttributeValues::Float32x3(points)) =
                mesh.attribute(Mesh::ATTRIBUTE_POSITION)
            else {
                panic!()
            };
            for (i, block) in points.chunks_exact(base).enumerate() {
                let p =
                    Vec3::from_array(*block.iter().max_by(|a, b| a[1].total_cmp(&b[1])).unwrap());
                let hit = hull
                    .cast_ray(
                        Vec3::ZERO,
                        Quat::IDENTITY,
                        p + Vec3::Y * 2.,
                        Vec3::NEG_Y,
                        3.,
                        false,
                    )
                    .unwrap();
                assert!(
                    hit.0 <= 2.001,
                    "missing visible segment {i} in variant {id}"
                );
            }
        }
    }
    #[test]
    fn lateral_walls_block_hero_capsule_and_step_probe_across_all_variants() {
        let step_probe_y = vael_engine::player::HERO_STEP_HEIGHT + 0.05;
        let max_walkable_normal_y = vael_engine::player::HERO_MAX_SLOPE_DEG.to_radians().cos();
        for id in 0..VARIANTS {
            let (mesh, mut hull, _) = formation(id);
            let Some(VertexAttributeValues::Float32x3(points)) =
                mesh.attribute(Mesh::ATTRIBUTE_POSITION)
            else {
                panic!()
            };
            // Even when an authored rock instance is scaled down to 0.70, lateral
            // rays at capsule-hemisphere height must hit vertical non-walkable walls
            // covering the full horizontal visual contour, and step probes must not
            // find a walkable ledge <= HERO_STEP_HEIGHT.
            for scale in [1.0_f32, 0.70_f32] {
                hull.set_scale(Vec3::splat(scale), 4);
                for k in 0..16 {
                    let angle = TAU * (k as f32 / 16.0);
                    let dir_xz = Vec2::new(angle.cos(), angle.sin());
                    let v_max = points
                        .iter()
                        .map(|v| Vec2::new(v[0], v[2]) * scale)
                        .max_by(|a, b| a.dot(dir_xz).total_cmp(&b.dot(dir_xz)))
                        .unwrap();
                    let ray_dir = Vec3::new(-dir_xz.x, 0.0, -dir_xz.y);
                    let ray_origin = Vec3::new(
                        v_max.x + dir_xz.x * 2.0,
                        vael_engine::player::HERO_FLOAT_HEIGHT + vael_engine::player::HERO_RADIUS,
                        v_max.y + dir_xz.y * 2.0,
                    );
                    let (toi, normal) = hull
                        .cast_ray(Vec3::ZERO, Quat::IDENTITY, ray_origin, ray_dir, 3.0, false)
                        .expect("lateral ray must hit quarry hull");
                    assert!(
                        toi <= 2.001,
                        "variant {id} scale {scale} angle {k}: hull missed outer vertex (toi={toi:.3})"
                    );
                    assert!(
                        normal.y < max_walkable_normal_y,
                        "variant {id} scale {scale} angle {k}: lateral normal.y {:.3} is walkable",
                        normal.y
                    );
                    let probe = Vec3::new(
                        v_max.x - dir_xz.x * 0.02,
                        step_probe_y,
                        v_max.y - dir_xz.y * 0.02,
                    );
                    if let Some((down_dist, down_normal)) = hull.cast_ray(
                        Vec3::ZERO,
                        Quat::IDENTITY,
                        probe,
                        Vec3::NEG_Y,
                        vael_engine::player::HERO_STEP_HEIGHT + 0.15,
                        true,
                    ) {
                        let surface = probe.y - down_dist;
                        let steppable =
                            (0.03..=vael_engine::player::HERO_STEP_HEIGHT).contains(&surface)
                                && down_normal.y >= 0.95;
                        assert!(
                            !steppable,
                            "variant {id} scale {scale} angle {k}: step probe found ledge at y={surface:.3}"
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn moisture_expands_patches_and_frost_suspends_growth() {
        let wet = grow(INITIAL_MOSS, 1., 12., 24.);
        assert!(wet > INITIAL_MOSS && grow(wet, 0., 12., 24.) < wet);
        assert_eq!(grow(wet, 1., -3., 24.), wet);
        assert_eq!(grow(wet, 1., 12., 0.), wet);
    }
}

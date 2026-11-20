//! Wood splinters and mineral chips leave the struck surface and fall under gravity.
use super::*;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy)]
pub(super) struct ToolHit {
    pub stone: bool,
    pub surface: Option<mining::Surface>,
    pub base: Vec3,
    pub outward: Vec2,
    pub height: f32,
    pub target: Entity,
    pub final_hit: bool,
}

#[derive(Resource, Clone, Default)]
pub(super) struct HitQueue(pub Arc<Mutex<Vec<ToolHit>>>);

impl HitQueue {
    pub fn push(&self, hit: ToolHit) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).push(hit);
    }
}

#[derive(Component)]
pub(super) struct Chip {
    velocity: Vec3,
    spin: Vec3,
    remaining: f32,
    floor: f32,
}

/// Use the same pooled variant and placement as the standing trunk, including slopes.
#[derive(bevy::ecs::system::SystemParam)]
pub(super) struct Bark<'w, 's> {
    trees: Query<
        'w,
        's,
        (
            &'static GlobalTransform,
            Option<&'static PooledTree>,
            Option<&'static regrowth::Rootstock>,
        ),
    >,
    pools: Option<Res<'w, Assets<bevy_flora::TreePool>>>,
    ground: Option<Res<'w, bevy_flora::FloraGround>>,
}

impl Bark<'_, '_> {
    fn contact(&self, hit: &ToolHit, normal: Vec2) -> Option<Vec3> {
        let (pose, tree, root) = self.trees.get(hit.target).ok()?;
        let tree = tree.or_else(|| root.map(|r| &r.pooled))?;
        let pool = self.pools.as_ref()?.get(&tree.pool)?;
        let variant = pool.variants.get(tree.variant as usize)?;
        let at = pose.translation().xz();
        let foot_radius = pool.params.base_radius() * pose.scale().x;
        let mut floor = pose.translation().y;
        if let Some(ground) = self.ground.as_ref() {
            floor = floor.max(ground.height(at + normal * foot_radius));
        }
        let y = floor + hit.height;
        let wood = variant
            .shape
            .trunk
            .iter()
            .map(|cap| cap.placed(pose))
            .find(|cap| cap.b.y >= y)?;
        let t = ((y - wood.a.y) / (wood.b.y - wood.a.y).max(0.001)).clamp(0.0, 1.0);
        let p = wood.a.lerp(wood.b, t).xz() + normal * wood.radius;
        Some(Vec3::new(p.x, y, p.y))
    }
}

pub(super) fn emit(
    mut commands: Commands,
    hits: Res<HitQueue>,
    bark: Bark,
    mut last_stone: Local<Option<(Entity, Vec3)>>,
    rocks: Query<(&crate::rocks::QuarryStone, Option<&Fracture>)>,
    colliders: Query<(&avian3d::prelude::Collider, &GlobalTransform)>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
    mut shared: Local<Option<(Handle<Mesh>, [Handle<StandardMaterial>; 5])>>,
    mut shakes: Option<ResMut<TreeShakes>>,
    time: Res<Time>,
) {
    let hits = std::mem::take(&mut *hits.0.lock().unwrap_or_else(|e| e.into_inner()));
    if hits.is_empty() {
        return;
    }
    let (Some(mut meshes), Some(mut materials)) = (meshes, materials) else {
        return;
    };
    let (mesh, palette) = shared.get_or_insert_with(|| {
        (
            meshes.add(Tetrahedron::default()),
            [
                Color::srgb(0.75, 0.48, 0.24),
                Color::srgb(0.46, 0.25, 0.12),
                Color::srgb(0.42, 0.40, 0.37),
                Color::srgb(0.62, 0.60, 0.54),
                Color::srgb(0.14, 0.13, 0.12),
            ]
            .map(|color| {
                materials.add(StandardMaterial {
                    base_color: color,
                    perceptual_roughness: 0.95,
                    ..default()
                })
            }),
        )
    });
    for hit in hits {
        let normal = hit.outward.normalize_or(Vec2::X);
        let side = normal.perp();
        let direction = Vec3::new(normal.x, 0.0, normal.y);
        let contact = if hit.stone {
            rocks
                .get(hit.target)
                .ok()
                .and_then(|(rock, old)| {
                    let struck = hit.surface.map_or(hit.target, |s| s.collider);
                    let (hull, pose) = colliders.get(struck).ok()?;
                    let extent = pose.scale().abs().max_element();
                    let probe = hit.base + direction * extent * 0.9 + Vec3::Y * extent * 0.89;
                    let closest = |p| {
                        hull.project_point(pose.translation(), pose.rotation(), p, false)
                            .0
                    };
                    let at = hit.surface.map_or_else(|| closest(probe), |s| s.point);
                    let n = hit
                        .surface
                        .map_or_else(|| (probe - at).normalize_or(Vec3::Y), |s| s.normal);
                    if !hit.final_hit {
                        let mut fracture = old.cloned().unwrap_or_default();
                        let local = pose.affine().inverse().transform_point3(at);
                        let local_normal =
                            (pose.affine().matrix3.transpose() * n).normalize_or(Vec3::Y);
                        let index = fracture.seams.iter().position(|s| {
                            s.struck == struck
                                && s.origin.distance(local) < 0.19
                                && s.normal.dot(local_normal) > 0.65
                        });
                        let mut seam = if let Some(i) = index {
                            let mut seam = fracture.seams[i];
                            commands.entity(seam.mark).try_despawn();
                            seam.depth = (seam.depth + 1).min(5);
                            seam
                        } else {
                            let cell = (local * 24.).round().as_ivec3().as_uvec3();
                            Seam {
                                struck,
                                origin: local,
                                normal: local_normal,
                                depth: 1,
                                mark: Entity::PLACEHOLDER,
                                seed: rock.seed
                                    ^ cell.x.rotate_left(9)
                                    ^ cell.y.rotate_left(18)
                                    ^ cell.z,
                            }
                        };
                        let normal = (pose.affine().matrix3.inverse().transpose() * seam.normal)
                            .normalize_or(n);
                        // Cast back onto the exposed face so folds between segments stay visible.
                        let project = |p| {
                            let start = p + normal * extent * 0.28;
                            hull.cast_ray(
                                pose.translation(),
                                pose.rotation(),
                                start,
                                -normal,
                                extent * 0.62,
                                false,
                            )
                            .map_or_else(|| closest(p), |(t, _)| start - normal * t)
                        };
                        let scar = scar_mesh(&seam, normal, extent, pose, project);
                        seam.mark = commands
                            .spawn((
                                Name::new("mining fracture"),
                                ChildOf(struck),
                                Mesh3d(meshes.add(scar)),
                                MeshMaterial3d(palette[4].clone()),
                                crate::snow_caps::IgnoreSnowMesh,
                                crate::collect_fx::IgnoreOutline,
                            ))
                            .id();
                        if let Some(i) = index {
                            fracture.seams[i] = seam;
                        } else {
                            fracture.seams.push(seam);
                        }
                        commands.entity(hit.target).try_insert(fracture);
                    }
                    let contact = at + n * 0.018;
                    *last_stone = Some((hit.target, contact));
                    Some(contact)
                })
                .or_else(|| hit.surface.map(|s| s.point + s.normal * 0.018))
                .or_else(|| {
                    last_stone
                        .filter(|(e, _)| *e == hit.target)
                        .map(|(_, at)| at)
                })
        } else {
            hit.surface.map(|s| s.point).or_else(|| bark.contact(&hit, normal))
        };
        let surface = contact.unwrap_or(
            hit.base + Vec3::new(normal.x * STUMP_RADIUS, hit.height, normal.y * STUMP_RADIUS),
        );
        if contact.is_some()
            && !hit.stone
            && !hit.final_hit
            && let Some(shakes) = shakes.as_deref_mut()
        {
            shakes.shake(
                TreeShake {
                    at: hit.base.xz(),
                    radius: 1.4,
                    strength: 0.65,
                    direction: -normal,
                    duration: 0.32,
                    start: 0.0,
                },
                &time,
            );
        }
        let phase = time.elapsed_secs() * 7.1 + hit.base.x;
        for i in 0..if hit.final_hit { 14 } else { 9 } {
            let angle = phase + i as f32 * 2.4;
            let lateral = normal * (0.85 + angle.cos().abs()) + side * angle.sin() * 1.55;
            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(palette[usize::from(hit.stone) * 2 + i % 2].clone()),
                Transform::from_translation(surface)
                    .with_rotation(Quat::from_rotation_z(angle))
                    .with_scale(if hit.stone {
                        Vec3::new(0.085, 0.065, 0.055)
                            * (0.7 + angle.cos().abs())
                            * if hit.final_hit { 1.65 } else { 1. }
                    } else {
                        Vec3::new(0.035, 0.10 + 0.065 * angle.sin().abs(), 0.025)
                    }),
                Chip {
                    velocity: Vec3::new(
                        lateral.x,
                        1.4 + 1.35 * (angle * 1.7).cos().abs(),
                        lateral.y,
                    ),
                    spin: Vec3::new(6.0, angle.sin() * 10.0, 4.0),
                    remaining: 0.75,
                    floor: hit.base.y,
                },
            ));
        }
    }
}

pub(super) fn fly(
    mut commands: Commands,
    time: Res<Time>,
    mut chips: Query<(Entity, &mut Chip, &mut Transform)>,
) {
    let dt = time.delta_secs();
    for (entity, mut chip, mut transform) in &mut chips {
        chip.remaining -= dt;
        chip.velocity.y -= 9.81 * dt;
        transform.translation += chip.velocity * dt;
        transform.rotate(Quat::from_scaled_axis(chip.spin * dt));
        if chip.remaining <= 0.0 || transform.translation.y < chip.floor {
            commands.entity(entity).despawn();
        }
    }
}

#[derive(Component, Clone, Default)]
pub(crate) struct Fracture {
    seams: Vec<Seam>,
}

#[derive(Clone, Copy)]
struct Seam {
    struck: Entity,
    mark: Entity,
    origin: Vec3,
    normal: Vec3,
    seed: u32,
    depth: u8,
}

fn scar_mesh(
    seam: &Seam,
    normal: Vec3,
    extent: f32,
    pose: &GlobalTransform,
    project: impl Fn(Vec3) -> Vec3,
) -> Mesh {
    use bevy::{asset::RenderAssetUsages, mesh::PrimitiveTopology};
    use std::f32::consts::TAU;
    let u = normal.any_orthonormal_vector();
    let v = normal.cross(u);
    let inv = pose.affine().inverse();
    let at = pose.transform_point(seam.origin);
    let point = |p: Vec2| {
        inv.transform_point3(
            project(at + (u * p.x + v * p.y) * extent) + normal * (0.0035 * extent),
        )
        .to_array()
    };
    let mut vertices = Vec::new();
    let mut ribbon = |a: Vec2, b: Vec2, w: f32, tip: f32| {
        let side = (b - a).normalize_or(Vec2::X).perp();
        let [l, r, end_l, end_r] =
            [a - side * w, a + side * w, b - side * tip, b + side * tip].map(point);
        vertices.extend([l, end_l, r, r, end_l, end_r]);
    };
    let rand = |n| crate::rocks::random(seam.seed ^ n);
    let branches = 3 + u32::from(rand(9) > 0.58);
    let steps = 3 + u32::from(seam.depth);
    for arm in 0..branches {
        let angle = TAU * (rand(5) + (arm as f32 + 0.38 * rand(400 + arm)) / branches as f32);
        let mut heading = Vec2::from_angle(angle);
        let mut p = Vec2::ZERO;
        for i in 0..steps {
            let salt = 10 + arm * 100 + i * 6;
            heading = Vec2::from_angle((rand(salt) - 0.5) * 0.82).rotate(heading);
            let next = p + heading * (0.025 + 0.027 * rand(salt + 1));
            let width =
                (0.005 + 0.0014 * f32::from(seam.depth)) * (1. - 0.72 * i as f32 / steps as f32);
            ribbon(
                p,
                next,
                width,
                if i + 1 == steps { 0.0005 } else { width * 0.85 },
            );
            if i > 0 && i + 1 < steps && rand(salt + 2) < 0.48 {
                let side = if rand(salt + 3) < 0.5 { -1. } else { 1. };
                let branch =
                    Vec2::from_angle(side * (0.52 + rand(salt + 4) * 0.54)).rotate(heading);
                let elbow = next + branch * 0.026;
                ribbon(next, elbow, width * 0.64, width * 0.32);
                ribbon(
                    elbow,
                    elbow + (branch + heading * 0.4) * 0.022,
                    width * 0.32,
                    0.0004,
                );
            }
            p = next;
        }
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vertices);
    mesh.compute_flat_normals();
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn impact_network_repeats_for_a_seed_and_extends_on_further_hits() {
        let mut seam = Seam {
            struck: Entity::PLACEHOLDER,
            mark: Entity::PLACEHOLDER,
            origin: Vec3::ZERO,
            normal: Vec3::Y,
            seed: 29,
            depth: 1,
        };
        let points = |s: &Seam| {
            let mesh = scar_mesh(s, Vec3::Y, 1., &GlobalTransform::IDENTITY, |p| p.with_y(0.));
            let Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) =
                mesh.attribute(Mesh::ATTRIBUTE_POSITION)
            else {
                panic!()
            };
            p.clone()
        };
        let first = points(&seam);
        assert_eq!(first, points(&seam));
        seam.depth = 3;
        let grown = points(&seam);
        assert!(grown.len() > first.len());
        let reach = |p: &Vec<[f32; 3]>| {
            p.iter()
                .map(|v| Vec3::from_array(*v).length())
                .fold(0., f32::max)
        };
        assert!(reach(&grown) > reach(&first));
        seam.seed += 1;
        assert_ne!(grown, points(&seam));
    }
}

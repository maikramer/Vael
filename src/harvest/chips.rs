//! Wood splinters and mineral chips leave the struck surface and fall under gravity.
use super::*;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy)]
pub(super) struct ToolHit {
    pub stone: bool,
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
    rocks: Query<
        (
            &avian3d::prelude::Collider,
            &GlobalTransform,
            Option<&Fracture>,
        ),
        With<crate::rocks::QuarryStone>,
    >,
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
                .map(|(hull, pose, old)| {
                    let extent = pose.scale().abs().max_element();
                    let probe = hit.base + direction * extent * 0.9 + Vec3::Y * extent * 0.89;
                    let project = |p| {
                        hull.project_point(pose.translation(), pose.rotation(), p, false)
                            .0
                    };
                    let at = project(probe);
                    let n = (probe - at).normalize_or(Vec3::Y);
                    if !hit.final_hit {
                        let count = old.map_or(1, |f| f.count.saturating_add(1).min(5));
                        if let Some(f) = old {
                            commands.entity(f.mark).try_despawn();
                        }
                        let scar = scar_mesh(at, n, extent, count, pose, project);
                        let mark = commands
                            .spawn((
                                Name::new("mining fracture"),
                                ChildOf(hit.target),
                                Mesh3d(meshes.add(scar)),
                                MeshMaterial3d(palette[4].clone()),
                                crate::snow_caps::IgnoreSnowMesh,
                            ))
                            .id();
                        commands
                            .entity(hit.target)
                            .try_insert(Fracture { count, mark });
                    }
                    let contact = at + n * 0.018;
                    *last_stone = Some((hit.target, contact));
                    contact
                })
                .or_else(|| {
                    last_stone
                        .filter(|(e, _)| *e == hit.target)
                        .map(|(_, at)| at)
                })
        } else {
            bark.contact(&hit, normal)
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

#[derive(Component)]
pub(crate) struct Fracture {
    count: u8,
    mark: Entity,
}

// Tapered, crooked seams wrap the actual struck face. Keep just the latest mark per rock.
fn scar_mesh(
    at: Vec3,
    normal: Vec3,
    extent: f32,
    count: u8,
    pose: &GlobalTransform,
    project: impl Fn(Vec3) -> Vec3,
) -> Mesh {
    use bevy::{asset::RenderAssetUsages, mesh::PrimitiveTopology};
    let u = normal.any_orthonormal_vector();
    let v = normal.cross(u);
    let inv = pose.affine().inverse();
    let size = extent * (0.085 + 0.046 * f32::from(count));
    let mut vertices = Vec::new();
    for branch in 0..3 {
        let angle = 2.15 * branch as f32 + 0.42;
        let radial = u * angle.cos() + v * angle.sin();
        let sideways = normal.cross(radial);
        let edge = |step: usize, sign: f32| {
            let t = step as f32 / 4.;
            let zig = [0., 0.13, -0.06, 0.14, 0.04][step];
            let offset = radial * t + sideways * (zig + sign * (0.085 * (1. - t) + 0.006));
            inv.transform_point3(project(at + offset * size + normal * 0.012) + normal * 0.009)
                .to_array()
        };
        for i in 0..4 {
            vertices.extend([
                edge(i, -1.),
                edge(i + 1, -1.),
                edge(i, 1.),
                edge(i, 1.),
                edge(i + 1, -1.),
                edge(i + 1, 1.),
            ]);
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

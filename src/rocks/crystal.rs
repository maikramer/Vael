//! Seeded hexagonal crystals rooted in an exposed facet. The saved world
//! calendar determines growth, including after streaming or loading a slot.
use super::{hash, random};
use avian3d::prelude::Collider;
use bevy::{asset::RenderAssetUsages, mesh::PrimitiveTopology, prelude::*};
use bevy_elements::WorldClock;
use std::{collections::BTreeMap, f32::consts::TAU};

const EPOCH: &str = "quarry.crystal_epoch";
pub(super) fn mineralized(seed: u32) -> bool {
    hash(seed ^ 0x8fa6_2b31) % 100 < 14
}

#[derive(Component, Reflect)]
#[reflect(Component)]
pub(crate) struct CrystalGrowth {
    seed: u32,
    maturity: f32,
}

fn maturity(seed: u32, hours: f64) -> f32 {
    let initial = 0.22 + 0.40 * random(seed ^ 773);
    let duration = 48. + 72. * random(seed ^ 647);
    let t = (initial + hours.max(0.) as f32 / duration).min(1.);
    t * t * (3. - 2. * t)
}

fn scale(age: f32) -> Vec3 {
    Vec3::new(0.38 + 0.62 * age, age, 0.38 + 0.62 * age)
}

pub(super) fn grow(
    clock: Option<Res<WorldClock>>,
    mut clusters: Query<(&mut CrystalGrowth, &mut Transform)>,
) {
    let Some(clock) = clock else { return };
    if clusters.is_empty() {
        return;
    }
    let hour = clock.hours_since_2000();
    let epoch = viber::luau::game::game_get_json(EPOCH)
        .and_then(|v| v.as_f64())
        .unwrap_or_else(|| {
            viber::luau::game::apply_patch_json(&BTreeMap::from([(
                EPOCH.to_string(),
                serde_json::json!(hour),
            )]));
            hour
        });
    for (mut cluster, mut pose) in &mut clusters {
        let age = maturity(cluster.seed, hour - epoch);
        // Update the physics shape only when growth is visible, including after rewinding a save.
        if (age - cluster.maturity).abs() > 0.004 || (age == 1. && cluster.maturity != 1.) {
            pose.scale = scale(age);
            cluster.maturity = age;
        }
    }
}

#[derive(Default)]
pub(super) struct Palette([Option<Handle<StandardMaterial>>; 3]);

impl Palette {
    pub(super) fn attach(
        &mut self,
        commands: &mut Commands,
        parent: Entity,
        seed: u32,
        rock: &Collider,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<StandardMaterial>,
    ) {
        let r = |n| random(seed ^ n);
        let angle = r(70) * TAU;
        let direction = Vec3::new(angle.cos(), 0.8 + r(42) * 1.1, angle.sin()).normalize();
        let origin = Vec3::Y * 0.4 + direction * 3.;
        let Some((distance, normal)) =
            rock.cast_ray(Vec3::ZERO, Quat::IDENTITY, origin, -direction, 5., false)
        else {
            return;
        };
        let anchor = origin - direction * distance - normal * 0.008;
        let (mesh, collider) = cluster_geometry(seed);
        let family = (hash(seed ^ 783) % 3) as usize;
        let color = [
            Color::srgb(0.32, 0.82, 0.78),
            Color::srgb(0.68, 0.48, 0.87),
            Color::srgb(0.91, 0.72, 0.40),
        ][family];
        let material = self.0[family].get_or_insert_with(|| {
            materials.add(StandardMaterial {
                base_color: color,
                perceptual_roughness: 0.22,
                reflectance: 0.68,
                ..default()
            })
        });
        let age = maturity(seed, 0.);
        commands.spawn((
            Name::new("crystal cluster"),
            ChildOf(parent),
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(material.clone()),
            collider,
            crate::snow_caps::IgnoreSnowMesh,
            Transform::from_translation(anchor)
                .with_rotation(Quat::from_rotation_arc(Vec3::Y, normal))
                .with_scale(scale(age)),
            CrystalGrowth {
                seed,
                maturity: age,
            },
        ));
    }
}

// One closed solid per prism. The visual facets and the collision hull use identical vertices.
fn cluster_geometry(seed: u32) -> (Mesh, Collider) {
    let mut vertices = Vec::new();
    let mut colors = Vec::new();
    let mut hulls = Vec::new();
    let count = 3 + hash(seed ^ 77) % 4;
    for i in 0..count {
        let rand = |k| random(seed ^ (i * 100 + k));
        let azimuth = TAU * (i as f32 / (count - 1) as f32 + rand(3) * 0.11);
        let outward = Vec3::new(azimuth.cos(), 0., azimuth.sin());
        let height = if i == 0 {
            0.33 + 0.13 * rand(4)
        } else {
            0.14 + 0.17 * rand(4)
        };
        let width = height * (0.15 + 0.07 * rand(5));
        let (base, axis) = if i == 0 {
            (Vec3::ZERO, Vec3::Y)
        } else {
            (
                outward * (0.035 + 0.038 * rand(6)),
                (Vec3::Y + outward * (0.24 + 0.68 * rand(7))).normalize(),
            )
        };
        let rot = Quat::from_rotation_arc(Vec3::Y, axis);
        let mut points = Vec::with_capacity(13);
        for layer in 0..2 {
            for side in 0..6 {
                let a = TAU * (side as f32 / 6. + rand(9));
                let rad = if layer == 0 { width * 0.78 } else { width };
                let y = if layer == 0 {
                    -0.025
                } else {
                    height * (0.68 + 0.08 * rand(side + 10))
                };
                points.push(base + rot * Vec3::new(rad * a.cos(), y, rad * a.sin()));
            }
        }
        points.push(base + rot * Vec3::new(width * 0.14, height, 0.));
        let mut tri = |a: usize, b: usize, c: usize, light: f32| {
            vertices.extend([points[a], points[b], points[c]].map(|p| p.to_array()));
            colors.extend([[light, light, light, 1.]; 3]);
        };
        for j in 0..6 {
            let k = (j + 1) % 6;
            let light = 0.60 + 0.36 * rand(30 + j as u32);
            tri(j, j + 6, k, light);
            tri(k, j + 6, k + 6, light);
            tri(j + 6, 12, k + 6, (light + 0.14).min(1.));
        }
        for j in 1..5 {
            tri(0, j, j + 1, 0.5);
        }
        hulls.push((
            Vec3::ZERO,
            Quat::IDENTITY,
            Collider::convex_hull(&points).expect("solid prism"),
        ));
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vertices)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.compute_flat_normals();
    (mesh, Collider::compound(hulls))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rare_hosts_and_growth_are_reproducible_after_a_rewind() {
        let hosts = (0..4096).filter(|&s| mineralized(s)).count();
        assert!((460..690).contains(&hosts));
        for seed in 0..64 {
            let initial = maturity(seed, 0.);
            assert!(initial > 0. && maturity(seed, 18.) > initial);
            assert_eq!(maturity(seed, -24.), initial);
            assert_eq!(maturity(seed, 200.), 1.);
            let (_, hull) = cluster_geometry(seed);
            assert!(
                hull.cast_ray(Vec3::ZERO, Quat::IDENTITY, Vec3::Y, Vec3::NEG_Y, 1., false)
                    .is_some()
            );
        }
    }
}

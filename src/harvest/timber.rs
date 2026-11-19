//! The cut starts on the tree's hinge. The upper trunk then becomes an independent
//! rigid body. Its saved lifetime belongs to the wood, independently of its stump.
use super::*;
use avian3d::prelude::{
    AngularDamping, AngularVelocity, Collider, Friction, LinearDamping, LinearVelocity, Mass,
    Restitution, RigidBody, RigidBodyDisabled, Sleeping,
};
use bevy::{asset::RenderAssetUsages, mesh::PrimitiveTopology};
use bevy_flora::TreePool;
use serde::{Deserialize, Serialize};
use std::f32::consts::TAU;

#[derive(Component)]
pub(super) struct FallingTrunk {
    pub restored: bool,
}

#[derive(Component, Reflect)]
#[reflect(Component)]
pub(super) struct GroundLog {
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedLog {
    position: Vec3,
    rotation: Quat,
    velocity: Vec3,
    spin: Vec3,
    length: f32,
    radius: f32,
    tip_radius: f32,
    kind: String,
    amount: i64,
    remaining: i64,
}

pub(super) struct TimberPlugin;
impl Plugin for TimberPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<GroundLog>()
            .add_systems(
                Update,
                (release.after(regrowth::fit_stumps), sleep_distant_logs),
            )
            .add_systems(
                PostUpdate,
                track.after(bevy::transform::TransformSystems::Propagate),
            )
            .add_systems(
                viber::scene_loader::SceneLoad,
                restore.after(viber::recipes::spawn::startup),
            );
    }
}

impl SavedLog {
    fn spawn(
        &self,
        commands: &mut Commands,
        id: String,
        meshes: &mut Assets<Mesh>,
        material: Handle<StandardMaterial>,
    ) -> (Entity, Entity) {
        let collider = self.collider();
        let body = commands
            .spawn((
                Name::new(format!("Tronco • {id}")),
                GroundLog { id },
                Visibility::Inherited,
                Transform::from_translation(self.position).with_rotation(self.rotation),
                RigidBody::Dynamic,
                collider,
                Mass((self.length * self.radius.powi(2) * 680.).clamp(16., 480.)),
                LinearVelocity(self.velocity),
                AngularVelocity(self.spin),
                Friction::new(0.68),
                Restitution::new(0.12),
                LinearDamping(0.38),
                AngularDamping(0.7),
            ))
            .insert(Harvestable {
                hits: 3,
                remaining: self.remaining,
                style: BreakStyle::Shatter,
                kind: self.kind.clone(),
                amount: self.amount,
                popup: if self.kind == "dark-wood" {
                    "Madeira escura"
                } else {
                    "Madeira"
                }
                .into(),
                popup_color: DEFAULT_POPUP_COLOR,
                range: RANGE_DEFAULT,
                cut_height: self.radius,
                replacement: String::new(),
                replace_scale: 1.,
                name: String::new(),
                stable_key: None,
            })
            .id();
        let visual = commands
            .spawn((
                ChildOf(body),
                Mesh3d(meshes.add(self.mesh())),
                MeshMaterial3d(material),
            ))
            .id();
        (body, visual)
    }

    fn ring(&self, side: usize, t: f32) -> Vec3 {
        let angle = TAU * side as f32 / 16.;
        let uneven = 1. + 0.043 * (angle * 3. + 0.6).sin() + 0.021 * (angle * 7.).cos();
        let r = self.radius.lerp(self.tip_radius, t) * uneven;
        Vec3::new(r * angle.cos(), (t - 0.5) * self.length, r * angle.sin())
    }

    fn collider(&self) -> Collider {
        let points: Vec<_> = (0..16)
            .flat_map(|s| [self.ring(s, 0.), self.ring(s, 1.)])
            .collect();
        Collider::convex_hull(&points).expect("solid tapered trunk")
    }

    fn mesh(&self) -> Mesh {
        let mut pos = Vec::new();
        let mut colors = Vec::new();
        let bark = if self.kind == "dark-wood" {
            Vec3::new(0.105, 0.061, 0.039)
        } else {
            Vec3::new(0.19, 0.095, 0.045)
        };
        let mut tri = |a: Vec3, b: Vec3, c: Vec3, color: Vec3| {
            pos.extend([a, b, c].map(|v| v.to_array()));
            colors.extend([color.extend(1.).to_array(); 3]);
        };
        for s in 0..16 {
            let k = (s + 1) % 16;
            let [a, b, c, d] = [
                self.ring(s, 0.),
                self.ring(s, 1.),
                self.ring(k, 0.),
                self.ring(k, 1.),
            ];
            let shade = bark * (0.72 + 0.32 * (s as f32 * 2.41).cos().abs());
            tri(a, b, c, shade);
            tri(c, b, d, shade);
            for (t, p, q) in [(0., a, c), (1., d, b)] {
                let center = Vec3::Y * ((t - 0.5) * self.length);
                let inside = |point: Vec3, r: f32| center.lerp(point, r);
                let mut outer = 1.;
                for (inner, color) in [
                    (0.91, shade),
                    (0.65, Vec3::new(0.55, 0.31, 0.14)),
                    (0.59, Vec3::new(0.34, 0.17, 0.073)),
                    (0.31, Vec3::new(0.48, 0.25, 0.11)),
                    (0.26, Vec3::new(0.31, 0.14, 0.065)),
                ] {
                    let (p0, q0, p1, q1) = (
                        inside(p, outer),
                        inside(q, outer),
                        inside(p, inner),
                        inside(q, inner),
                    );
                    tri(p0, q0, p1, color);
                    tri(p1, q0, q1, color);
                    outer = inner;
                }
                tri(
                    inside(p, outer),
                    inside(q, outer),
                    center,
                    Vec3::new(0.42, 0.22, 0.09),
                );
            }
        }
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors);
        mesh.compute_flat_normals();
        mesh
    }
}

#[derive(bevy::ecs::system::SystemParam)]
pub(super) struct TimberArt<'w, 's> {
    meshes: Option<ResMut<'w, Assets<Mesh>>>,
    materials: Option<ResMut<'w, Assets<StandardMaterial>>>,
    material: Local<'s, Option<Handle<StandardMaterial>>>,
}
impl TimberArt<'_, '_> {
    fn assets(&mut self) -> Option<(&mut Assets<Mesh>, Handle<StandardMaterial>)> {
        let materials = self.materials.as_deref_mut()?;
        let material = self
            .material
            .get_or_insert_with(|| {
                materials.add(StandardMaterial {
                    perceptual_roughness: 0.92,
                    ..default()
                })
            })
            .clone();
        Some((self.meshes.as_deref_mut()?, material))
    }
}

// Until contact the original branches and leaves ride the body. Only then is
// the clean trunk exposed. The stump is already a separate world entity.
#[derive(Component)]
pub(super) struct Crown {
    tree: Entity,
    stump: Entity,
    visual: Entity,
}

fn release(
    mut commands: Commands,
    mut trees: Query<
        (Entity, &mut Felled, &Transform, &FallingTrunk),
        (Without<BuckTree>, Without<ChildOf>),
    >,
    stumps: Query<&TreeStump>,
    roots: Query<&regrowth::Rootstock>,
    pools: Option<Res<Assets<TreePool>>>,
    mut save: Option<ResMut<VaelSave>>,
    mut art: TimberArt,
) {
    let (Some(save), Some(pools)) = (save.as_deref_mut(), pools.as_deref()) else {
        return;
    };
    for (tree, mut fell, tf, source) in &mut trees {
        if fell.angle < 0.90 && !fell.at_rest {
            continue;
        }
        let Some(root) = roots.iter().find(|r| r.tree == tree) else {
            continue;
        };
        let Some(key) = root.key.as_deref() else {
            continue;
        };
        let field = format!("flora-fell.{key}.detached");
        if viber::luau::game::game_get_json(&field).is_some_and(|v| v == true) {
            commands.entity(tree).try_despawn();
            continue;
        }
        let Some((meshes, material)) = art.assets() else {
            continue;
        };
        let Some(variant) = pools
            .get(&root.pooled.pool)
            .and_then(|p| p.variants.get(root.pooled.variant as usize))
        else {
            continue;
        };
        let Ok(stump) = stumps.get(fell.stump) else {
            continue;
        };
        let caps = &variant.shape.trunk;
        let cut = stump.capsule.b.y;
        let mut upper = caps.iter().filter_map(|cap| cap.above(cut));
        let Some(butt) = upper.next() else { continue };
        let top = upper.last().unwrap_or(butt);
        // The top twigs leave with the crown. Keep the usable length of the bole.
        let global = GlobalTransform::from(*tf);
        let a = global.transform_point(butt.a);
        let b = global.transform_point(butt.a.lerp(top.b, 0.82));
        let axis = (b - a).normalize_or(Vec3::Y);
        let scale = tf.scale.x.abs();
        let rolling_axis = Vec3::Y.cross(axis).normalize_or(Vec3::X);
        let moving = !source.restored;
        let spin = if moving {
            rolling_axis * 1.6 + axis * 0.8
        } else {
            Vec3::ZERO
        };
        let log = SavedLog {
            position: (a + b) * 0.5,
            rotation: Quat::from_rotation_arc(Vec3::Y, axis),
            length: a.distance(b).max(0.25),
            radius: (butt.radius * scale).max(0.05),
            tip_radius: (top.radius * scale * 0.7).clamp(0.035, (butt.radius * scale).max(0.05)),
            velocity: spin.cross((b - a) * 0.5),
            spin,
            kind: root.harvest.kind.clone(),
            amount: root.harvest.amount.max(1),
            remaining: 3,
        };
        let mut generation = 0;
        let id = loop {
            let id = format!("{key}/{generation}");
            if !save.timber.contains_key(&id) {
                break id;
            }
            generation += 1;
        };
        let (body, visual) = log.spawn(&mut commands, id.clone(), meshes, material);
        if moving {
            fell.at_rest = true;
            let body_pose = GlobalTransform::from(
                Transform::from_translation(log.position).with_rotation(log.rotation),
            );
            commands.entity(visual).insert(Visibility::Hidden);
            commands
                .entity(tree)
                .insert((ChildOf(body), global.reparented_to(&body_pose)));
            commands.entity(body).insert(Crown {
                tree,
                stump: fell.stump,
                visual,
            });
        } else {
            commands.entity(tree).try_despawn();
        }
        save.timber.insert(id, log);
        save.legacy.insert(field.clone(), Json::Bool(true));
        viber::luau::game::apply_patch_json(&BTreeMap::from([(field, Json::Bool(true))]));
    }
}

// Terrain colliders stream around the player. Keep distant timber in place while its terrain is unloaded.
fn sleep_distant_logs(
    mut commands: Commands,
    players: Query<&GlobalTransform, With<Player>>,
    logs: Query<(Entity, &Transform, Has<RigidBodyDisabled>), With<GroundLog>>,
) {
    let Some(player) = players.iter().next() else {
        return;
    };
    for (e, tf, disabled) in &logs {
        let dist = tf
            .translation
            .xz()
            .distance_squared(player.translation().xz());
        if !disabled && dist > 48_f32.powi(2) {
            commands.entity(e).insert(RigidBodyDisabled);
        }
        if disabled && dist < 36_f32.powi(2) {
            commands.entity(e).remove::<RigidBodyDisabled>();
        }
    }
}

fn track(
    mut commands: Commands,
    mut save: Option<ResMut<VaelSave>>,
    ground: Option<Res<bevy_flora::FloraGround>>,
    mut landed: Option<MessageWriter<TreeLanded>>,
    logs: Query<(
        Entity,
        &GroundLog,
        &Transform,
        &LinearVelocity,
        &AngularVelocity,
        &Harvestable,
        Option<&Crown>,
        Has<Sleeping>,
    )>,
) {
    let Some(save) = save.as_deref_mut() else {
        return;
    };
    for (e, tag, tf, v, w, h, crown, sleeping) in &logs {
        let Some(log) = save.timber.get_mut(&tag.id) else {
            continue;
        };
        log.position = tf.translation;
        log.rotation = tf.rotation;
        log.velocity = v.0;
        log.spin = w.0;
        log.remaining = h.remaining;
        if let Some(crown) = crown {
            let end = tf.rotation * Vec3::Y * (log.length * 0.5);
            let contact = [
                (tf.translation - end, log.radius),
                (tf.translation + end, log.tip_radius),
            ]
            .into_iter()
            .find(|(p, r)| {
                ground
                    .as_ref()
                    .is_some_and(|g| p.y - r <= g.height(p.xz()) + 0.12)
            });
            // A log can also come to rest on a boulder or another log above the terrain.
            let impact = contact
                .map(|(p, _)| p)
                .or_else(|| sleeping.then_some(tf.translation));
            if let Some(at) = impact {
                commands.entity(crown.tree).try_despawn();
                commands
                    .entity(crown.visual)
                    .try_insert(Visibility::Inherited);
                commands.entity(e).remove::<Crown>();
                if let Some(bus) = landed.as_mut() {
                    bus.write(TreeLanded {
                        tree: crown.tree,
                        stump: crown.stump,
                        at,
                    });
                }
            }
        }
    }
}

pub(super) fn restore(
    mut commands: Commands,
    logs: Query<Entity, With<GroundLog>>,
    scene: Option<Res<CurrentScene>>,
    save: Option<Res<VaelSave>>,
    mut art: TimberArt,
) {
    for e in &logs {
        commands.entity(e).try_despawn();
    }
    if scene.as_deref().and_then(|s| s.id.as_deref()) != Some("main") {
        return;
    }
    let Some(save) = save else { return };
    let Some((meshes, material)) = art.assets() else {
        return;
    };
    for (id, log) in &save.timber {
        if log.position.is_finite()
            && log.rotation.is_normalized()
            && log.length > 0.
            && log.radius > 0.
            && log.tip_radius > 0.
            && log.remaining > 0
        {
            log.spawn(&mut commands, id.clone(), meshes, material.clone());
        }
    }
}

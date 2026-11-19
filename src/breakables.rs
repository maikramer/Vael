//! Loose wooden props. Adopt the same asset in prefabs and inline interior scenes.
//! The weapon contact comes from combat.lua, after windup and input arbitration.
use std::sync::{Arc, Mutex};

use avian3d::prelude::*;
use bevy::prelude::*;
use bevy_ext_core::{SfxClip, SfxEvent};
use viber::luau::{LuaScriptRef, ScriptProps};
use viber::recipes::spawn::{GltfScenePending, PendingWorld, startup};
use viber::recipes::{EntityKind, EntitySpec};
use viber::scene_loader::{CurrentScene, SceneEntity, SceneLoad};

use crate::vault::{vael_fn, vael_table};

const VIAL: &str = "assets/meshes/props/life_vial_lod0.glb";
const PROP: &str = "breakable-prop";

pub(super) struct BreakablesPlugin;
impl Plugin for BreakablesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Strikes>()
            .register_type::<Breakable>()
            .add_systems(Startup, (prepare.before(startup), install))
            .add_systems(SceneLoad, prepare.before(startup))
            .add_systems(Update, (attach, activate, strike, expire_debris).chain());
    }
}

#[derive(Component, Reflect)]
#[reflect(Component)]
struct Breakable {
    kind: Kind,
    hits_left: u8,
}

#[derive(Clone, Copy, PartialEq, Reflect)]
enum Kind {
    Barrel,
    Crate,
    Stool,
}

impl Kind {
    fn asset(spec: &EntitySpec) -> Option<&'static str> {
        let EntityKind::GltfScene { url, .. } = &spec.kind else {
            return None;
        };
        match url.trim_start_matches('/') {
            "assets/meshes/village/wooden_barrel_lod0.glb" => Some("barrel"),
            "assets/meshes/village/wooden_crate_lod0.glb" => Some("crate"),
            "assets/meshes/interiors/stool_wood_lod0.glb" => Some("stool"),
            _ => None,
        }
    }

    fn dimensions(self) -> Vec3 {
        // Feet-origin AABBs from the pool's *_precompute.json, in metres.
        match self {
            Self::Barrel => Vec3::new(0.61, 0.9, 0.58),
            Self::Crate => Vec3::splat(0.8),
            Self::Stool => Vec3::new(0.46, 0.60, 0.45),
        }
    }
}

fn prepare(
    pending: Option<ResMut<PendingWorld>>,
    scene: Option<Res<CurrentScene>>,
    strikes: Res<Strikes>,
) {
    strikes.0.lock().unwrap().clear();
    if scene.as_deref().and_then(|s| s.id.as_deref()) == Some("menu") {
        return;
    }
    if let Some(mut pending) = pending {
        for node in &mut pending.world.entities {
            prepare_node(node);
        }
    }
}

fn prepare_node(node: &mut EntitySpec) {
    if node.script.is_some()
        || node.physics.trigger
        || node
            .props
            .iter()
            .any(|(k, _)| matches!(k.as_str(), "destructible" | "resource-kind"))
    {
        return;
    }
    if let Some(kind) = Kind::asset(node) {
        node.physics = Default::default();
        node.props.push((PROP.into(), kind.into()));
        return;
    }
    // The collider in a placement often lives on an anonymous wrapper, while
    // the GLB child carries its own scale/rotation. Transfer the whole body
    // to that child. A scripted chest or compound scenery keeps its own physics.
    if !node.physics.is_empty() {
        if matches!(node.kind, EntityKind::Group)
            && let [model] = &node.children[..]
            && model.script.is_none()
            && Kind::asset(model).is_some()
            && model.props.is_empty()
        {
            node.physics = Default::default();
        } else {
            return;
        }
    }
    for child in &mut node.children {
        prepare_node(child);
    }
}

fn attach(mut commands: Commands, new: Query<(Entity, &ScriptProps), Added<ScriptProps>>) {
    for (e, props) in &new {
        let Some((_, value)) = props.0.iter().find(|(k, _)| k == PROP) else {
            continue;
        };
        let kind = match value.as_str() {
            "barrel" => Kind::Barrel,
            "crate" => Kind::Crate,
            "stool" => Kind::Stool,
            _ => continue,
        };
        let d = kind.dimensions();
        let hull = if kind == Kind::Barrel {
            Collider::cylinder(d.z * 0.5, d.y)
        } else {
            Collider::cuboid(d.x, d.y, d.z)
        };
        commands.entity(e).insert((
            Breakable {
                kind,
                hits_left: if kind == Kind::Stool { 1 } else { 2 },
            },
            Collider::compound(vec![(Vec3::Y * d.y * 0.5, Quat::IDENTITY, hull)]),
            RigidBody::Static,
            Mass(if kind == Kind::Stool { 3. } else { 9. }),
            NoAutoMass,
            Friction::new(0.65),
            AngularDamping(1.25),
        ));
    }
}

// Outdoor floor colliders stream around the hero. A distant prop must wait
// for real support before gravity starts, including after fast travel.
fn activate(
    mut commands: Commands,
    hero: Query<(Entity, &GlobalTransform), With<viber::player::Player>>,
    props: Query<(Entity, &GlobalTransform, &RigidBody), With<Breakable>>,
    spatial: SpatialQuery,
    sensors: Query<(), With<Sensor>>,
) {
    let Ok((player, hero)) = hero.single() else {
        return;
    };
    for (e, pose, body) in &props {
        let at = pose.translation();
        let dist = hero.translation().distance_squared(at);
        if *body == RigidBody::Static && dist < 10.0_f32.powi(2) {
            let filter = SpatialQueryFilter::from_excluded_entities([e, player]);
            if spatial
                .cast_ray_predicate(at + Vec3::Y * 0.6, Dir3::NEG_Y, 5., true, &filter, &|hit| {
                    !sensors.contains(hit) && !props.contains(hit)
                })
                .is_some()
            {
                commands.entity(e).insert(RigidBody::Dynamic);
            }
        } else if *body == RigidBody::Dynamic && dist > 18.0_f32.powi(2) {
            commands.entity(e).insert((
                RigidBody::Static,
                LinearVelocity::ZERO,
                AngularVelocity::ZERO,
            ));
        }
    }
}

struct Swing {
    at: Vec3,
    aim: Vec2,
    reach: f32,
    arc: f32,
    finisher: bool,
}

impl Swing {
    fn reaches(&self, center: Vec3) -> bool {
        let delta = center - self.at;
        let flat = delta.xz();
        delta.y.abs() <= 1.5
            && flat.length_squared() <= self.reach * self.reach
            && (flat.length_squared() < 0.001 || self.aim.dot(flat.normalize()) >= self.arc)
    }
}

#[derive(Resource, Default)]
struct Strikes(Arc<Mutex<Vec<Swing>>>);

fn install(queue: Res<Strikes>) {
    let queue = queue.0.clone();
    crate::script_host::register_installer(move |lua| {
        let vael = vael_table(lua)?;
        vael_fn!(
            lua,
            vael,
            "strike_props",
            [queue = queue.clone()],
            move |_,
                  (x, y, z, fx, fz, reach, arc, finisher): (
                f32,
                f32,
                f32,
                f32,
                f32,
                f32,
                f32,
                bool
            )| {
                queue.lock().unwrap().push(Swing {
                    at: Vec3::new(x, y, z),
                    aim: Vec2::new(fx, fz).normalize_or_zero(),
                    reach,
                    arc,
                    finisher,
                });
                Ok(())
            }
        );
        Ok(())
    });
}

#[derive(Component)]
struct Debris(f32);

struct Art {
    plank: Handle<Mesh>,
    coin: Handle<Mesh>,
    glint: Handle<Mesh>,
    oak: Handle<StandardMaterial>,
    gold: Handle<StandardMaterial>,
    glow: Handle<StandardMaterial>,
    vial: Handle<Gltf>,
}

impl Art {
    fn new(
        mesh: &mut Assets<Mesh>,
        mat: &mut Assets<StandardMaterial>,
        server: &AssetServer,
    ) -> Self {
        Self {
            plank: mesh.add(Cuboid::default()),
            coin: mesh.add(Cylinder::new(0.11, 0.035)),
            glint: mesh.add(Sphere::new(0.016).mesh().uv(6, 4)),
            oak: mat.add(Color::srgb(0.54, 0.28, 0.10)),
            gold: mat.add(StandardMaterial {
                base_color: Color::srgb(1., 0.68, 0.12),
                metallic: 0.65,
                perceptual_roughness: 0.3,
                ..default()
            }),
            glow: mat.add(StandardMaterial {
                emissive: LinearRgba::rgb(5., 2.8, 0.7),
                ..default()
            }),
            vial: viber::meshopt::load_gltf(server, VIAL.to_owned()),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn strike(
    mut commands: Commands,
    queue: Res<Strikes>,
    mut props: Query<(Entity, &GlobalTransform, &mut Breakable)>,
    spatial: SpatialQuery,
    sensors: Query<(), With<Sensor>>,
    hero: Query<Entity, With<viber::player::Player>>,
    server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut art: Local<Option<Art>>,
    mut sfx: MessageWriter<SfxEvent>,
) {
    for swing in std::mem::take(&mut *queue.0.lock().unwrap()) {
        let visible_filter = SpatialQueryFilter::from_excluded_entities(hero.iter());
        for (e, pose, mut prop) in &mut props {
            let size = prop.kind.dimensions();
            let center = pose.transform_point(Vec3::Y * size.y * 0.5);
            if prop.hits_left == 0 || !swing.reaches(center) {
                continue;
            }
            let hand = swing.at + Vec3::Y * 0.75;
            let line = center - hand;
            if let Ok(direction) = Dir3::new(line)
                && spatial
                    .cast_ray_predicate(
                        hand,
                        direction,
                        line.length(),
                        true,
                        &visible_filter,
                        &|hit| !sensors.contains(hit),
                    )
                    .is_some_and(|hit| hit.entity != e)
            {
                continue;
            }
            prop.hits_left = if swing.finisher {
                0
            } else {
                prop.hits_left - 1
            };
            let broken = prop.hits_left == 0;
            sfx.write(SfxEvent {
                clip: if broken {
                    SfxClip::ChopBreak
                } else {
                    SfxClip::ChopHit
                },
                position: Some(center),
            });
            let push = Vec3::new(swing.aim.x, 0.35, swing.aim.y);
            if !broken {
                commands.entity(e).remove::<Sleeping>().insert((
                    LinearVelocity(push * 1.25),
                    AngularVelocity(Vec3::Y.cross(push) * 1.8),
                ));
                continue;
            }
            let art = art.get_or_insert_with(|| Art::new(&mut meshes, &mut materials, &server));
            commands.entity(e).despawn();
            let filter =
                SpatialQueryFilter::from_excluded_entities(std::iter::once(e).chain(hero.iter()));
            let floor = spatial
                .cast_ray_predicate(center, Dir3::NEG_Y, 12., true, &filter, &|hit| {
                    !sensors.contains(hit)
                })
                .map_or(pose.translation().y, |hit| center.y - hit.distance);
            let roll = ((center.x * 7.).round() as i32)
                .wrapping_add((center.z * 11.).round() as i32)
                .rem_euclid(4);
            let kind = if prop.kind == Kind::Stool || roll == 3 {
                "wood"
            } else if roll == 2 {
                "gold"
            } else {
                "life"
            };
            spawn_pickup(
                &mut commands,
                art,
                kind,
                center.with_y(floor),
                (center.y - floor).max(0.1),
            );
            for i in 0..7 {
                let angle = i as f32 * std::f32::consts::TAU / 7.;
                let dir = Vec3::new(angle.cos(), 0., angle.sin());
                let dims = size * pose.scale().abs() * Vec3::new(0.16, 0.55, 0.09);
                let tf = Transform::from_translation(center + dir * size.x * 0.38)
                    .with_rotation(pose.rotation() * Quat::from_rotation_y(-angle));
                let part = commands
                    .spawn((
                        SceneEntity,
                        Debris(1.4),
                        tf,
                        Visibility::Inherited,
                        RigidBody::Dynamic,
                        Collider::cuboid(dims.x, dims.y, dims.z),
                        LinearVelocity(dir * 2. + Vec3::Y * (1.7 + angle.cos().abs()) + push * 0.4),
                        AngularVelocity(Vec3::new(angle.sin(), 0.5, angle.cos()) * 6.),
                    ))
                    .id();
                commands.spawn((
                    ChildOf(part),
                    Mesh3d(art.plank.clone()),
                    MeshMaterial3d(art.oak.clone()),
                    Transform::from_scale(dims),
                ));
            }
        }
    }
}

fn spawn_pickup(commands: &mut Commands, art: &Art, kind: &str, floor: Vec3, height: f32) {
    let root = commands
        .spawn((
            Name::new(format!("Loot • {kind}")),
            SceneEntity,
            Transform::from_translation(floor + Vec3::Y * height),
            Visibility::Inherited,
            LuaScriptRef {
                path: "game/prop_pickup.lua".into(),
            },
            ScriptProps(Arc::new(vec![
                ("loot".into(), kind.into()),
                ("floor".into(), floor.y.to_string()),
                ("height".into(), height.to_string()),
            ])),
        ))
        .id();
    match kind {
        "life" => {
            commands.spawn((
                ChildOf(root),
                Transform::default(),
                Visibility::Inherited,
                GltfScenePending::single(art.vial.clone()),
            ));
        }
        "gold" => {
            commands.spawn((
                ChildOf(root),
                Mesh3d(art.coin.clone()),
                MeshMaterial3d(art.gold.clone()),
                Transform::from_rotation(Quat::from_rotation_x(1.2)),
            ));
        }
        _ => {
            for i in 0..2 {
                commands.spawn((
                    ChildOf(root),
                    Mesh3d(art.plank.clone()),
                    MeshMaterial3d(art.oak.clone()),
                    Transform::from_xyz(0., i as f32 * 0.055, 0.)
                        .with_rotation(Quat::from_rotation_y(i as f32 * 0.32))
                        .with_scale(Vec3::new(0.29, 0.055, 0.10)),
                ));
            }
        }
    }
    for offset in [
        Vec3::new(-0.14, 0.14, 0.),
        Vec3::new(0.12, 0.23, 0.04),
        Vec3::new(0.04, 0.05, -0.13),
    ] {
        commands.spawn((
            ChildOf(root),
            Mesh3d(art.glint.clone()),
            MeshMaterial3d(art.glow.clone()),
            Transform::from_translation(offset),
        ));
    }
}

fn expire_debris(
    mut commands: Commands,
    time: Res<Time>,
    mut pieces: Query<(Entity, &mut Debris)>,
) {
    for (e, mut piece) in &mut pieces {
        piece.0 -= time.delta_secs();
        if piece.0 <= 0. {
            commands.entity(e).despawn();
        }
    }
}

#[cfg(test)]
mod tests;

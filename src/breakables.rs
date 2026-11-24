//! Loose wooden props. Adopt the same asset in prefabs and inline interior scenes.
//! The weapon contact comes from combat.lua, after windup and input arbitration.
use std::sync::{Arc, Mutex};

use avian3d::prelude::*;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy_ext_core::{SfxClip, SfxEvent};
use bevy_ext_engine::camera_glue::CameraShake;
use vael_engine::luau::{LuaScriptRef, ScriptProps};
use vael_engine::particles::spawn_burst;
use vael_engine::recipes::spawn::{GltfScenePending, PendingWorld, startup};
use vael_engine::recipes::{EntityKind, EntitySpec};
use vael_engine::scene_loader::{CurrentScene, SceneEntity, SceneLoad};

use crate::collect_fx::{
    HitFlashQueue, IgnoreOutline, PickupGroundRing, PickupOrbitGlint, subtle_camera_shake,
};
use crate::vault::{vael_fn, vael_table};

const VIAL: &str = "assets/meshes/props/life_vial_lod0.glb";
const PROP: &str = "breakable-prop";

pub(super) struct BreakablesPlugin;
impl Plugin for BreakablesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Strikes>()
            .register_type::<Breakable>()
            .register_type::<PropPickup>()
            .add_systems(Startup, (prepare.before(startup), install))
            .add_systems(SceneLoad, prepare.before(startup))
            .add_systems(
                Update,
                (attach, free_wrapper, activate, strike, expire_debris).chain(),
            );
    }
}

#[derive(Component, Reflect)]
#[reflect(Component)]
pub(crate) struct Breakable {
    pub(crate) kind: Kind,
    pub(crate) hits_left: u8,
}

#[derive(Clone, Copy, PartialEq, Eq, Reflect)]
pub(crate) enum Kind {
    Barrel,
    Crate,
    Stool,
}

/// Tipo de item largado por um prop destrutível.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Reflect)]
pub(crate) enum PickupKind {
    Life,
    Gold,
    Wood,
}

impl PickupKind {
    fn from_raw(raw: &str) -> Self {
        match raw {
            "life" => Self::Life,
            "gold" => Self::Gold,
            _ => Self::Wood,
        }
    }
}

/// Marca uma entidade de loot largada por um prop destrutível (usada para
/// outline vibrante contínuo e efeitos de coleta).
#[derive(Component, Debug, Clone, Copy, Reflect)]
#[reflect(Component)]
pub(crate) struct PropPickup {
    pub(crate) kind: PickupKind,
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

/// O wrapper do `.bsn` às vezes fica com o corpo estático. Esse volume come
/// o golpe e prende o prop. O corpo que vale é o do `Breakable`.
fn free_wrapper(
    mut commands: Commands,
    fresh: Query<Entity, Added<Breakable>>,
    parents: Query<&ChildOf>,
    shells: Query<(), (With<RigidBody>, Without<Breakable>)>,
) {
    for prop in &fresh {
        let Ok(of) = parents.get(prop) else {
            continue;
        };
        let parent = of.parent();
        if shells.contains(parent) {
            commands.entity(parent).remove::<(RigidBody, Collider)>();
        }
    }
}

// Outdoor floor colliders stream around the hero. A distant prop must wait
// for real support before gravity starts, including after fast travel.
fn activate(
    mut commands: Commands,
    hero: Query<(Entity, &GlobalTransform), With<vael_engine::player::Player>>,
    props: Query<(Entity, &GlobalTransform, &RigidBody), With<Breakable>>,
    spatial: SpatialQuery,
    sensors: Query<(), With<Sensor>>,
) {
    let Some((player, hero)) = hero.iter().next() else {
        return;
    };
    for (e, pose, body) in &props {
        let at = pose.translation();
        let dist = hero.translation().distance_squared(at);
        if *body == RigidBody::Static && dist < 10.0_f32.powi(2) {
            let filter = SpatialQueryFilter::from_excluded_entities([e, player]);
            let supported = spatial
                .cast_ray_predicate(at + Vec3::Y * 1.5, Dir3::NEG_Y, 8., true, &filter, &|hit| {
                    !sensors.contains(hit) && !props.contains(hit)
                })
                .is_some();
            // Caixote encostado ao herói (atalaia, lama): o raio curto nascia
            // dentro do volume e não achava chão, e o corpo ficava estático.
            let beside = dist < 4.0_f32.powi(2) && (at.y - hero.translation().y).abs() < 1.6;
            if supported || beside {
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

/// Parede entre a mão e o prop. O chão e o colisor do próprio volume
/// (o wrapper estático fica a ~raio do centro) não contam.
fn blocks_swing(hit_distance: f32, line_len: f32, radius: f32, normal_y: f32) -> bool {
    if normal_y > 0.65 {
        return false;
    }
    line_len - hit_distance > radius + 0.05
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
    ring: Handle<Mesh>,
    oak: Handle<StandardMaterial>,
    gold: Handle<StandardMaterial>,
    glow_life: Handle<StandardMaterial>,
    glow_gold: Handle<StandardMaterial>,
    glow_wood: Handle<StandardMaterial>,
    ring_life: Handle<StandardMaterial>,
    ring_gold: Handle<StandardMaterial>,
    ring_wood: Handle<StandardMaterial>,
    vial: Handle<Gltf>,
}

impl Art {
    fn new(
        mesh: &mut Assets<Mesh>,
        mat: &mut Assets<StandardMaterial>,
        server: &AssetServer,
    ) -> Self {
        let make_ring = |mat: &mut Assets<StandardMaterial>, base: Color, emissive: LinearRgba| {
            mat.add(StandardMaterial {
                base_color: base,
                emissive,
                alpha_mode: AlphaMode::Add,
                unlit: true,
                cull_mode: None,
                ..default()
            })
        };
        Self {
            plank: mesh.add(Cuboid::default()),
            coin: mesh.add(Cylinder::new(0.11, 0.035)),
            glint: mesh.add(Sphere::new(0.019).mesh().uv(6, 4)),
            ring: mesh.add(Torus::new(0.86, 1.0)),
            oak: mat.add(Color::srgb(0.54, 0.28, 0.10)),
            gold: mat.add(StandardMaterial {
                base_color: Color::srgb(1., 0.72, 0.14),
                emissive: LinearRgba::rgb(0.45, 0.28, 0.03),
                metallic: 0.72,
                perceptual_roughness: 0.25,
                ..default()
            }),
            glow_life: mat.add(StandardMaterial {
                base_color: Color::srgb(1.0, 0.45, 0.68),
                emissive: LinearRgba::rgb(5.2, 1.2, 2.2),
                unlit: true,
                ..default()
            }),
            glow_gold: mat.add(StandardMaterial {
                base_color: Color::srgb(1.0, 0.88, 0.32),
                emissive: LinearRgba::rgb(5.5, 3.6, 0.65),
                unlit: true,
                ..default()
            }),
            glow_wood: mat.add(StandardMaterial {
                base_color: Color::srgb(1.0, 0.72, 0.34),
                emissive: LinearRgba::rgb(4.4, 2.3, 0.65),
                unlit: true,
                ..default()
            }),
            ring_life: make_ring(
                mat,
                Color::srgba(1.0, 0.34, 0.58, 0.72),
                LinearRgba::rgb(3.6, 0.75, 1.55),
            ),
            ring_gold: make_ring(
                mat,
                Color::srgba(1.0, 0.84, 0.24, 0.72),
                LinearRgba::rgb(4.0, 2.7, 0.45),
            ),
            ring_wood: make_ring(
                mat,
                Color::srgba(0.98, 0.68, 0.28, 0.68),
                LinearRgba::rgb(3.0, 1.65, 0.45),
            ),
            vial: vael_engine::meshopt::load_gltf(server, VIAL.to_owned()),
        }
    }

    fn glow_for(&self, kind: PickupKind) -> Handle<StandardMaterial> {
        match kind {
            PickupKind::Life => self.glow_life.clone(),
            PickupKind::Gold => self.glow_gold.clone(),
            PickupKind::Wood => self.glow_wood.clone(),
        }
    }

    fn ring_for(&self, kind: PickupKind) -> Handle<StandardMaterial> {
        match kind {
            PickupKind::Life => self.ring_life.clone(),
            PickupKind::Gold => self.ring_gold.clone(),
            PickupKind::Wood => self.ring_wood.clone(),
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
    hero: Query<Entity, With<vael_engine::player::Player>>,
    server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut art: Local<Option<Art>>,
    mut sfx: MessageWriter<SfxEvent>,
    flashes: Option<Res<HitFlashQueue>>,
    mut shake: Option<ResMut<CameraShake>>,
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
            let radius = size.x.max(size.z) * 0.5;
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
                    .is_some_and(|hit| {
                        hit.entity != e
                            && blocks_swing(hit.distance, line.length(), radius, hit.normal.y)
                    })
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
                if let Some(flashes) = flashes.as_deref() {
                    flashes.push(e, true, 1.0);
                }
                spawn_burst(
                    &mut commands,
                    &crate::collect_fx::burst_spec("ground-dust"),
                    center,
                    8,
                );
                commands.entity(e).remove::<Sleeping>().insert((
                    LinearVelocity(push * 1.25),
                    AngularVelocity(Vec3::Y.cross(push) * 1.8),
                ));
                continue;
            }
            subtle_camera_shake(shake.as_deref_mut(), 0.11);
            spawn_burst(
                &mut commands,
                &crate::collect_fx::burst_spec("ground-dust"),
                center,
                16,
            );
            spawn_burst(
                &mut commands,
                &crate::collect_fx::burst_spec("sparkle"),
                center + Vec3::Y * 0.15,
                12,
            );
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
    let pickup_kind = PickupKind::from_raw(kind);
    let root = commands
        .spawn((
            Name::new(format!("Loot • {kind}")),
            SceneEntity,
            PropPickup { kind: pickup_kind },
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
    // Halo luminoso no chão (excluído do outline para manter a silhueta do item nítida).
    commands.spawn((
        ChildOf(root),
        IgnoreOutline,
        NotShadowCaster,
        Mesh3d(art.ring.clone()),
        MeshMaterial3d(art.ring_for(pickup_kind)),
        Transform::from_xyz(0., 0.045 - height, 0.).with_scale(Vec3::new(0.32, 0.03, 0.32)),
        PickupGroundRing {
            floor_y: floor.y,
            phase: (floor.x * 1.3 + floor.z * 0.7).abs(),
        },
    ));
    // Faíscas orbitais em hélice viva ao redor do item.
    let glow_mat = art.glow_for(pickup_kind);
    for (i, (radius, base_y, bob_amp, speed)) in [
        (0.18, 0.14, 0.065, 2.8),
        (0.21, 0.22, 0.08, -2.3),
        (0.15, 0.08, 0.055, 3.4),
    ]
    .into_iter()
    .enumerate()
    {
        let phase = i as f32 * std::f32::consts::TAU / 3.0;
        commands.spawn((
            ChildOf(root),
            IgnoreOutline,
            NotShadowCaster,
            Mesh3d(art.glint.clone()),
            MeshMaterial3d(glow_mat.clone()),
            Transform::from_xyz(phase.cos() * radius, base_y, phase.sin() * radius),
            PickupOrbitGlint {
                phase,
                radius,
                base_y,
                bob_amp,
                speed,
            },
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

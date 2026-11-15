//! Barcos do Vael — casco procedural, física de flutuação e condução a remo.
//!
//! O barco é uma entidade própria com [`viber::terrain::FloatingBody`] (a
//! engine integra Arquimedes, corrente e encalhe) e [`Boat`] (este módulo
//! aplica o impulso dos remos). O herói continua a ser a entidade do
//! `Player`: ao entrar, o `RigidBodyDisabled` desliga o corpo do herói
//! passa a ser colocado no banco do barco a cada frame e o `Player` da engine
//! espelha a velocidade do casco — a câmara (que segue o `Player`) fica atrás
//! do barco sem nenhum caminho novo.
//!
//! Entrada/saída: a ação `interact` (E) a menos de [`BOARD_RADIUS`] metros do
//! barco. Os barcos nascem de `data/boats.json` em sítios com água; a engine
//! `WaterFxPlugin` vê-os como nadadores (contacto pela quilha) e a esteira
//! Kelvin chega ao solver de ondas.

use std::f32::consts::PI;
use std::path::{Path, PathBuf};

use avian3d::prelude::RigidBodyDisabled;
use bevy::prelude::*;
use bevy_ext_core::Player;
use serde::Deserialize;

use crate::player::VaelController;
use viber::recipes::spawn::OrbitCamera;
use viber::terrain::runtime::TerrainRuntime;
use viber::terrain::{FloatState, FloatingBody, WaterKinematics};

/// Distância (m) do herói ao barco para entrar.
pub const BOARD_RADIUS: f32 = 3.4;
/// Assento do herói no barco (pés no piso interior, entre os travessões).
pub const SEAT_LOCAL: Vec3 = Vec3::new(0.0, -0.38, -0.42);
/// Meia-boca, meia-altura e meia-comprimento do casco (m).
pub const HULL_HALF: Vec3 = Vec3::new(0.80, 0.30, 2.20);
/// Centro da caixa de carena no referencial do barco (origem = linha de água
/// nominal; a quilha fica em `hull_center.y - hull_half.y`).
pub const HULL_CENTER: Vec3 = Vec3::new(0.0, -0.24, 0.0);
/// Massa do barco a remos com herói e carga (kg).
pub const ROWBOAT_MASS: f32 = 260.0;
/// Altura da quilha ao fundo do casco (m) — usada para fechar o piso interior.
const FLOOR_ABOVE_KEEL: f32 = 0.10;

/// Um barco conduzível por remos.
#[derive(Component, Debug, Clone)]
pub struct Boat {
    /// Aceleração a pleno remo (m/s²).
    pub accel: f32,
    /// Fração da aceleração a remar para trás.
    pub reverse: f32,
    /// Velocidade máxima a remos (m/s); o sprint multiplica.
    pub max_speed: f32,
    /// Multiplicador de sprint (Shift).
    pub sprint_mult: f32,
    /// Taxa de guinada a velocidade de cruzeiro (rad/s).
    pub turn_rate: f32,
    /// Fase da remada (animação dos remos).
    pub oar_phase: f32,
}

impl Default for Boat {
    fn default() -> Self {
        Self {
            accel: 3.2,
            reverse: 0.45,
            max_speed: 4.0,
            sprint_mult: 1.5,
            turn_rate: 0.95,
            oar_phase: 0.0,
        }
    }
}

/// O herói está a bordo deste barco (componente do `Player`).
#[derive(Component)]
pub struct RidingBoat {
    pub boat: Entity,
}

/// Remo (filho do barco); a fase vem do [`Boat`] para os dois remos baterem
/// ao mesmo tempo.
#[derive(Component)]
pub struct Oar {
    /// -1 = bombordo, +1 = estibordo.
    pub side: f32,
}

/// Os barcos de `data/boats.json` já nasceram nesta cena.
#[derive(Resource, Default)]
pub struct BoatsSpawned(pub bool);

/// Configuração dos barcos do mundo.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct BoatsConfig {
    #[serde(default)]
    pub boats: Vec<BoatSpawn>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BoatSpawn {
    pub name: String,
    pub x: f32,
    pub z: f32,
    #[serde(default)]
    pub yaw: f32,
}

impl BoatsConfig {
    /// Lê `data/boats.json`; um ficheiro ausente é um mundo sem barcos.
    pub fn load(path: impl AsRef<Path>) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|err| {
                warn!("vael: `data/boats.json` inválido: {err}");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }
}

/// Corpo físico de um barco a remos (calibrado para assentar a ~7 cm).
pub fn rowboat_body() -> FloatingBody {
    FloatingBody {
        mass: ROWBOAT_MASS,
        hull_half: HULL_HALF,
        hull_center: HULL_CENTER,
        center_of_mass: Vec3::new(0.0, -0.16, 0.0),
        displacement_factor: 0.55,
        buoyancy_gain: 1.0,
        linear_drag: 1.0,
        quadratic_drag: 1.8,
        angular_drag: 3.1,
        heave_drag: 5.0,
        heave_quadratic: 0.9,
        wave_response: 1.0,
        max_speed: 9.0,
        ground_stiffness: 14000.0,
        ground_damping: 1200.0,
        ground_friction: 5.0,
    }
}

// ── condução (puro, testável) ──────────────────────────────────────────────

/// Impulso da remada: aceleração ao longo da proa com clamp de velocidade.
/// `throttle` ∈ [-1, 1]; `sprint` multiplica só o avanço.
pub fn row_thrust(
    velocity: Vec3,
    yaw: f32,
    throttle: f32,
    sprint: bool,
    boat: &Boat,
    dt: f32,
) -> Vec3 {
    let (fx, fz) = yaw.sin_cos();
    let forward = Vec3::new(fx, 0.0, fz);
    let mut next = velocity;
    let t = throttle.clamp(-1.0, 1.0);
    if t.abs() > 1e-3 {
        let gain = if t > 0.0 {
            boat.accel * if sprint { boat.sprint_mult } else { 1.0 }
        } else {
            boat.accel * boat.reverse
        };
        next += forward * (t * gain * dt);
    }
    // Só a componente ao longo da proa é limitada: corrente/correnteza podem
    // empurrar de lado sem o barco "colar" no limite.
    let along = next.dot(forward);
    let limit = boat.max_speed * if sprint { boat.sprint_mult } else { 1.0 };
    if along.abs() > limit {
        next += forward * ((along.signum() * limit) - along);
    }
    next
}

/// Guinada do leme: `steer` ∈ [-1, 1] (D = +1), escalada pela velocidade
/// (parado, o barco quase não vira; a remar vira).
pub fn row_yaw(yaw: f32, steer: f32, speed: f32, boat: &Boat, dt: f32) -> f32 {
    let speed_factor = (speed / 1.2).clamp(0.0, 1.0) * 0.75 + 0.25;
    yaw + steer.clamp(-1.0, 1.0) * boat.turn_rate * speed_factor * dt
}

// ── malha procedural do casco ──────────────────────────────────────────────

#[derive(Default)]
struct MeshBuilder {
    positions: Vec<[f32; 3]>,
}

impl MeshBuilder {
    fn tri(&mut self, a: Vec3, b: Vec3, c: Vec3) {
        self.positions.push(a.to_array());
        self.positions.push(b.to_array());
        self.positions.push(c.to_array());
    }

    fn quad(&mut self, a: Vec3, b: Vec3, c: Vec3, d: Vec3) {
        self.tri(a, b, c);
        self.tri(a, c, d);
    }

    fn box_part(&mut self, center: Vec3, half: Vec3) {
        let c = center;
        let h = half;
        let p = |x: f32, y: f32, z: f32| Vec3::new(c.x + x * h.x, c.y + y * h.y, c.z + z * h.z);
        // topo / fundo
        self.quad(
            p(-1., 1., -1.),
            p(1., 1., -1.),
            p(1., 1., 1.),
            p(-1., 1., 1.),
        );
        self.quad(
            p(-1., -1., 1.),
            p(1., -1., 1.),
            p(1., -1., -1.),
            p(-1., -1., -1.),
        );
        // lados
        self.quad(
            p(-1., -1., 1.),
            p(-1., 1., 1.),
            p(1., 1., 1.),
            p(1., -1., 1.),
        );
        self.quad(
            p(1., -1., -1.),
            p(1., 1., -1.),
            p(-1., 1., -1.),
            p(-1., -1., -1.),
        );
        self.quad(
            p(-1., -1., -1.),
            p(-1., 1., -1.),
            p(-1., 1., 1.),
            p(-1., -1., 1.),
        );
        self.quad(
            p(1., -1., 1.),
            p(1., 1., 1.),
            p(1., 1., -1.),
            p(1., -1., -1.),
        );
    }

    fn into_mesh(self) -> Mesh {
        let mut mesh = Mesh::new(
            bevy::render::mesh::PrimitiveTopology::TriangleList,
            bevy::asset::RenderAssetUsages::MAIN_WORLD
                | bevy::asset::RenderAssetUsages::RENDER_WORLD,
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.positions);
        mesh.compute_flat_normals();
        mesh
    }
}

/// Meia-boca do casco ao longo do comprimento (`t` 0 = popa, 1 = proa).
fn hull_taper(t: f32) -> f32 {
    let s = (PI * t).sin().max(0.0);
    0.16 + 0.84 * s.powf(0.55)
}

/// Cota da borda (gunwale) ao longo do comprimento — sobe nas pontas.
fn hull_sheer(t: f32) -> f32 {
    0.06 + 0.16 * (1.0 - (PI * t).sin().powf(0.8))
}

/// Cota da quilha ao longo do comprimento — rocker nas pontas.
fn hull_keel(t: f32) -> f32 {
    -0.54 + 0.22 * (1.0 - (PI * t).sin().powf(0.9))
}

/// Ponto do casco: `u` ∈ [-1, 1] (bombordo → estibordo) e `inset` 0 = casco
/// exterior, 1 = superfície interior (espessura e piso).
fn hull_point(t: f32, u: f32, inset: f32) -> Vec3 {
    let taper = hull_taper(t);
    let half = 0.80 * taper * (1.0 - 0.075 * inset);
    let bottom = hull_keel(t) + FLOOR_ABOVE_KEEL * inset + 0.05 * inset * (1.0 - u * u);
    let top = hull_sheer(t);
    let y = bottom + (top - bottom) * u * u;
    Vec3::new(u * half, y, -2.20 + 4.40 * t)
}

/// Malha exterior+interior do casco, com o rio (gunwale) e os bancos.
pub fn rowboat_hull_mesh() -> Mesh {
    const STATIONS: usize = 16;
    const US: usize = 6;
    let mut b = MeshBuilder::default();
    let us: Vec<f32> = (0..US)
        .map(|i| (i as f32 / (US - 1) as f32) * 2.0 - 1.0)
        .collect();

    // Casco exterior e interior.
    for inset in [0.0_f32, 1.0] {
        for i in 0..STATIONS {
            let t0 = i as f32 / STATIONS as f32;
            let t1 = (i + 1) as f32 / STATIONS as f32;
            for j in 0..US - 1 {
                b.quad(
                    hull_point(t0, us[j], inset),
                    hull_point(t1, us[j], inset),
                    hull_point(t1, us[j + 1], inset),
                    hull_point(t0, us[j + 1], inset),
                );
            }
        }
    }
    // Rio: fecha a espessura entre a borda exterior e a interior.
    for i in 0..STATIONS {
        let t0 = i as f32 / STATIONS as f32;
        let t1 = (i + 1) as f32 / STATIONS as f32;
        for j in 0..US - 1 {
            b.quad(
                hull_point(t0, us[j], 0.0),
                hull_point(t0, us[j + 1], 0.0),
                hull_point(t0, us[j + 1], 1.0),
                hull_point(t0, us[j], 1.0),
            );
            let _ = t1;
        }
    }
    // Transom (fecha popa e proa entre exterior e interior).
    for t in [0.0_f32, 1.0] {
        for j in 0..US - 1 {
            b.quad(
                hull_point(t, us[j], 0.0),
                hull_point(t, us[j + 1], 0.0),
                hull_point(t, us[j + 1], 1.0),
                hull_point(t, us[j], 1.0),
            );
        }
    }

    // Bancos (thwarts): dois travessões e o assento do remador.
    b.box_part(Vec3::new(0.0, -0.27, -0.85), Vec3::new(0.63, 0.045, 0.15));
    b.box_part(Vec3::new(0.0, -0.27, 0.75), Vec3::new(0.58, 0.045, 0.14));
    b.box_part(Vec3::new(0.0, -0.27, 0.05), Vec3::new(0.60, 0.045, 0.16));

    b.into_mesh()
}

/// Malha de um remo: haste ao longo de +Z e pá no extremo.
pub fn rowboat_oar_mesh() -> Mesh {
    let mut b = MeshBuilder::default();
    b.box_part(Vec3::new(0.0, 0.0, 0.95), Vec3::new(0.035, 0.035, 0.95));
    b.box_part(Vec3::new(0.0, 0.0, 1.90), Vec3::new(0.11, 0.016, 0.22));
    b.into_mesh()
}

// ── plugin ─────────────────────────────────────────────────────────────────

pub struct VaelBoatPlugin;

impl Plugin for VaelBoatPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BoatsSpawned>()
            .add_systems(Startup, load_boat_assets)
            .add_systems(viber::scene_loader::SceneLoad, despawn_boats_on_scene_load)
            .add_systems(
                Update,
                (
                    spawn_boats,
                    recover_missing_boat,
                    board_or_leave_boat,
                    drive_boats,
                    seat_riders,
                    animate_oars,
                )
                    .chain()
                    .after(crate::player::PlayerMovementSet)
                    .run_if(crate::player::no_editor_camera),
            );
    }
}

/// Materiais/malhas partilhados pelos barcos (os handles vivem num recurso).
#[derive(Resource, Default)]
struct BoatAssets {
    hull: Handle<Mesh>,
    oars: Handle<Mesh>,
    wood: Handle<StandardMaterial>,
    trim: Handle<StandardMaterial>,
}

fn load_boat_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let wood = materials.add(StandardMaterial {
        base_color: Color::srgb(0.30, 0.19, 0.11),
        perceptual_roughness: 0.86,
        cull_mode: None,
        ..Default::default()
    });
    let trim = materials.add(StandardMaterial {
        base_color: Color::srgb(0.52, 0.35, 0.19),
        perceptual_roughness: 0.78,
        cull_mode: None,
        ..Default::default()
    });
    commands.insert_resource(BoatAssets {
        hull: meshes.add(rowboat_hull_mesh()),
        oars: meshes.add(rowboat_oar_mesh()),
        wood,
        trim,
    });
}

fn despawn_boats_on_scene_load(
    mut commands: Commands,
    boats: Query<Entity, With<Boat>>,
    mut spawned: ResMut<BoatsSpawned>,
) {
    for entity in &boats {
        commands.entity(entity).try_despawn();
    }
    spawned.0 = false;
}

#[allow(clippy::too_many_arguments)]
fn spawn_boats(
    mut commands: Commands,
    mut spawned: ResMut<BoatsSpawned>,
    assets: Option<Res<BoatAssets>>,
    terrain: Option<Res<TerrainRuntime>>,
    scene: Option<Res<viber::scene_loader::CurrentScene>>,
    root: Option<Res<crate::VaelProjectRoot>>,
) {
    if spawned.0 {
        return;
    }
    let (Some(assets), Some(terrain)) = (assets, terrain) else {
        return;
    };
    // Os barcos de `data/boats.json` são do MAPA PRINCIPAL: as coordenadas são
    // do `world.bsn`. No menu e nos interiores não há essa água — sem este
    // portão cada cena leve avisava "barco sem água" para pontos que nem
    // existem no seu terreno.
    if scene
        .as_ref()
        .and_then(|scene| scene.id.as_deref())
        .is_some_and(|id| id != viber::project::MAIN_SCENE)
    {
        spawned.0 = true;
        return;
    }
    spawned.0 = true;

    // A raiz do projeto, não o cwd: `vael-tools run`/`jd` mudam de pasta e o
    // contrato dos loaders é lerem `data/` de `VaelProjectRoot`.
    let config_path = root
        .as_deref()
        .map(|root| root.0.join("data/boats.json"))
        .unwrap_or_else(|| PathBuf::from("data/boats.json"));
    let config = BoatsConfig::load(&config_path);
    for spawn in &config.boats {
        let Some(surface) = terrain.water_surface_at(spawn.x, spawn.z) else {
            warn!(
                "vael: barco `{}` sem água em ({}, {})",
                spawn.name, spawn.x, spawn.z
            );
            continue;
        };
        let body = rowboat_body();
        let mut state = FloatState {
            yaw: spawn.yaw,
            ..Default::default()
        };
        let mut position = Vec3::new(spawn.x, surface + body.rest_height(), spawn.z);
        // Assenta já no equilíbrio (evita o "plop" de 0,5 m no arranque).
        let sample = |p: Vec2| {
            terrain
                .water_domain_at(p.x, p.y)
                .map(|d| viber::terrain::FloatSample {
                    surface_y: Some(d.surface_y),
                    bed_y: terrain.in_field(p.x, p.y).then(|| terrain.sample(p.x, p.y)),
                    flow: d.flow,
                })
                .unwrap_or(viber::terrain::FloatSample::dry(
                    terrain.in_field(p.x, p.y).then(|| terrain.sample(p.x, p.y)),
                ))
        };
        for _ in 0..4 {
            viber::terrain::water_buoyancy::step_float(
                &body,
                &mut state,
                &mut position,
                1.0 / 30.0,
                sample,
                |_, _| 0.0,
            );
        }

        let spawn_tf = Transform {
            translation: position,
            rotation: state.rotation(),
            ..Default::default()
        };
        let boat = commands
            .spawn((
                Name::new(spawn.name.clone()),
                spawn_tf,
                GlobalTransform::from(spawn_tf),
                Visibility::Inherited,
                body,
                state,
                Boat::default(),
                WaterKinematics {
                    velocity: Vec3::ZERO,
                    body_radius: HULL_HALF.z.max(HULL_HALF.x) + 0.4,
                    contact_y_offset: HULL_CENTER.y - HULL_HALF.y,
                },
            ))
            .id();
        commands.spawn((
            Name::new(format!("{}:casco", spawn.name)),
            Mesh3d(assets.hull.clone()),
            MeshMaterial3d(assets.wood.clone()),
            ChildOf(boat),
        ));
        for side in [-1.0_f32, 1.0] {
            commands.spawn((
                Name::new(format!("{}:remo", spawn.name)),
                Mesh3d(assets.oars.clone()),
                MeshMaterial3d(assets.trim.clone()),
                Transform::from_xyz(0.76 * side, 0.02, 0.22),
                Oar { side },
                ChildOf(boat),
            ));
        }
    }
}

fn interaction_just_pressed(
    actions: Option<&viber::input_map::ActionState>,
    keys: &ButtonInput<KeyCode>,
) -> bool {
    actions.map_or_else(
        || keys.just_pressed(KeyCode::KeyE),
        |state| state.interact_just,
    )
}

/// Um `SceneLoad` despawna os barcos (e outro cenário os volta a semear). Se
/// o herói estava a bordo, o corpo tem de voltar — senão ficava desligado e
/// preso ao assento de um barco que já não existe.
fn recover_missing_boat(
    mut commands: Commands,
    players: Query<(Entity, &RidingBoat)>,
    boats: Query<(), With<Boat>>,
) {
    for (player, riding) in &players {
        if boats.get(riding.boat).is_ok() {
            continue;
        }
        commands
            .entity(player)
            .remove::<(RidingBoat, RigidBodyDisabled)>();
    }
}

#[allow(clippy::too_many_arguments)]
fn board_or_leave_boat(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    actions: Option<Res<viber::input_map::ActionState>>,
    mut players: Query<
        (
            Entity,
            &Transform,
            Option<&RidingBoat>,
            Option<&RigidBodyDisabled>,
        ),
        With<Player>,
    >,
    boats: Query<(Entity, &Transform, &FloatState), With<Boat>>,
    mut cameras: Query<&mut OrbitCamera>,
) {
    let pressed = interaction_just_pressed(actions.as_deref(), &keys);
    if !pressed {
        return;
    }
    let Some((player, player_tf, riding, _)) = players.iter_mut().next() else {
        return;
    };

    if let Some(riding) = riding {
        // Sair: repõe o corpo e larga o herói ao lado do casco.
        let Ok((_, boat_tf, float)) = boats.get(riding.boat) else {
            commands
                .entity(player)
                .remove::<(RidingBoat, RigidBodyDisabled)>();
            return;
        };
        let side = boat_tf.rotation * Vec3::X;
        let mut drop = boat_tf.translation + side * 1.15;
        drop.y = boat_tf.translation.y + 0.20;
        let rotation = Quat::from_rotation_y(float.yaw);
        commands.entity(player).insert((
            Transform {
                translation: drop,
                rotation,
                ..*player_tf
            },
            // O corpo é dinâmico: a `Position` é a pose autoritativa e o
            // `Transform` sozinho só chegaria no frame seguinte.
            avian3d::prelude::Position(drop),
            avian3d::prelude::Rotation(rotation),
            avian3d::prelude::LinearVelocity::ZERO,
        ));
        commands
            .entity(player)
            .remove::<(RidingBoat, RigidBodyDisabled)>();
        return;
    }

    // Entrar: o barco mais próximo dentro do raio.
    let mut best: Option<(Entity, f32)> = None;
    for (entity, boat_tf, _) in &boats {
        let d = boat_tf.translation.distance(player_tf.translation);
        if d <= BOARD_RADIUS && best.is_none_or(|(_, bd)| d < bd) {
            best = Some((entity, d));
        }
    }
    let Some((boat, _)) = best else {
        return;
    };
    // A bordo o corpo do herói fica DESLIGADO (o KCC era retirado e reposto).
    commands
        .entity(player)
        .insert((RidingBoat { boat }, RigidBodyDisabled));
    // A câmara já nasce atrás do barco em vez de varrer 180°.
    if let Ok((_, _, float)) = boats.get(boat) {
        for mut cam in &mut cameras {
            cam.yaw_deg = float.yaw.to_degrees() + 180.0;
            cam.smooth_yaw_deg = cam.yaw_deg;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn drive_boats(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    actions: Option<Res<viber::input_map::ActionState>>,
    typing: Option<Res<viber::ui::widgets::UiFocusedInput>>,
    menus: Option<Res<bevy_ext_core::MenusOpen>>,
    mut boats: Query<(&mut Boat, &mut FloatState), With<Boat>>,
    riders: Query<&RidingBoat>,
) {
    let Some(riding) = riders.iter().next() else {
        return;
    };
    let Ok((mut boat, mut float)) = boats.get_mut(riding.boat) else {
        return;
    };
    let blocked = menus.as_ref().is_some_and(|menus| menus.any())
        || typing.as_ref().is_some_and(|typing| typing.0.is_some());
    let (throttle, steer, sprint) = if blocked {
        (0.0, 0.0, false)
    } else if let Some(actions) = actions.as_deref() {
        (
            actions.forward.clamp(-1.0, 1.0),
            actions.move_x.clamp(-1.0, 1.0),
            actions.sprint,
        )
    } else {
        let forward =
            (keys.pressed(KeyCode::KeyW) as i32 - keys.pressed(KeyCode::KeyS) as i32) as f32;
        let steer =
            (keys.pressed(KeyCode::KeyD) as i32 - keys.pressed(KeyCode::KeyA) as i32) as f32;
        let sprint = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
        (forward, steer, sprint)
    };
    let dt = time.delta_secs().min(0.05);
    if dt <= 0.0 {
        return;
    }
    let speed = float.velocity.xz().length();
    float.velocity = row_thrust(float.velocity, float.yaw, throttle, sprint, &boat, dt);
    float.yaw = row_yaw(float.yaw, steer, speed, &boat, dt);
    // A remada anima com o esforço (avançar/recuar), não com a velocidade.
    boat.oar_phase += (0.6 + throttle.abs() * 2.6) * dt * throttle.signum();
}

#[allow(clippy::type_complexity)]
fn seat_riders(
    mut players: Query<
        (
            &mut Transform,
            &mut Player,
            &mut VaelController,
            &RidingBoat,
        ),
        With<Player>,
    >,
    boats: Query<(&Transform, &FloatState), (With<Boat>, Without<Player>)>,
) {
    for (mut transform, mut engine_player, mut controller, riding) in &mut players {
        let Ok((boat_tf, float)) = boats.get(riding.boat) else {
            continue;
        };
        transform.translation = boat_tf.translation + boat_tf.rotation * SEAT_LOCAL;
        transform.rotation = Quat::from_rotation_y(float.yaw);
        // Velocidade ZERO para a animação (o herói vai sentado/em pé, não a
        // caminhar a 3 m/s). A câmara não precisa dela: assenta atrás do
        // `facing` do herói, que é o yaw do barco.
        engine_player.vel_x = 0.0;
        engine_player.vel_z = 0.0;
        engine_player.vel_y = 0.0;
        engine_player.grounded = true;
        engine_player.is_jumping = false;
        controller.grounded = true;
        controller.vel_x = 0.0;
        controller.vel_z = 0.0;
        controller.vel_y = 0.0;
    }
}

fn animate_oars(boats: Query<&Boat>, mut oars: Query<(&Oar, &ChildOf, &mut Transform)>) {
    for (oar, parent, mut transform) in &mut oars {
        let Ok(boat) = boats.get(parent.parent()) else {
            continue;
        };
        let phase = boat.oar_phase;
        let sweep = phase.sin() * 0.55 * oar.side;
        let dip = (phase.cos() * 0.16 + 0.05) * oar.side;
        transform.rotation = Quat::from_rotation_y(sweep) * Quat::from_rotation_x(-0.22 + dip);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_thrust_accelerates_clamps_and_reverses() {
        let boat = Boat::default();
        let v = row_thrust(Vec3::ZERO, 0.0, 1.0, false, &boat, 0.5);
        assert!(v.z > 1.0, "acelera na proa (+Z): {v:?}");

        // Nunca passa o máximo ao longo da proa.
        let fast = Vec3::new(0.0, 0.0, 40.0);
        let capped = row_thrust(fast, 0.0, 1.0, false, &boat, 0.5);
        assert!((capped.z - boat.max_speed).abs() < 1e-4, "{capped:?}");

        // Sprint sobe o teto.
        let sprint = row_thrust(fast, 0.0, 1.0, true, &boat, 0.5);
        assert!(sprint.z > boat.max_speed + 0.1);

        // Remar para trás acelera no sentido oposto.
        let back = row_thrust(Vec3::ZERO, 0.0, -1.0, false, &boat, 0.5);
        assert!(back.z < -0.1, "{back:?}");
    }

    #[test]
    fn row_yaw_scales_with_speed_and_sign() {
        let boat = Boat::default();
        let still = row_yaw(0.0, 1.0, 0.0, &boat, 0.5);
        let fast = row_yaw(0.0, 1.0, 4.0, &boat, 0.5);
        assert!(fast > still, "a velocidade aumenta a guinada");
        assert!(still > 0.0, "parado ainda vira devagar (remos)");
        let left = row_yaw(0.0, -1.0, 4.0, &boat, 0.5);
        assert!(left < 0.0);
    }

    #[test]
    fn hull_mesh_has_volume_and_oars_are_longer_than_the_hull_beam() {
        let hull = rowboat_hull_mesh();
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(pos)) =
            hull.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("posições do casco");
        };
        assert!(pos.len() > 600, "casco detalhado: {} vértices", pos.len());
        let zs = pos.iter().map(|p| p[2]);
        let z_min = zs.clone().fold(f32::INFINITY, f32::min);
        let z_max = zs.fold(f32::NEG_INFINITY, f32::max);
        assert!((z_max - z_min - 4.4).abs() < 0.1, "comprimento do casco");
        assert!(hull.attribute(Mesh::ATTRIBUTE_NORMAL).is_some());

        let oars = rowboat_oar_mesh();
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(opos)) =
            oars.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("posições dos remos");
        };
        let max_z = opos.iter().map(|p| p[2]).fold(f32::NEG_INFINITY, f32::max);
        assert!(max_z > 2.0, "o remo passa do bordo: {max_z}");
    }

    #[test]
    fn boats_config_parses_and_tolerates_missing_fields() {
        let cfg: BoatsConfig = serde_json::from_str(
            r#"{"boats":[{"name":"barco-a","x":-80.0,"z":104.0},{"name":"barco-b","x":0.0,"z":0.0,"yaw":1.5}]}"#,
        )
        .expect("config válida");
        assert_eq!(cfg.boats.len(), 2);
        assert_eq!(cfg.boats[0].yaw, 0.0);
        assert!((cfg.boats[1].yaw - 1.5).abs() < 1e-6);
    }
}

//! Barcos do Vael — casco procedural, física de flutuação e condução a remo.
//!
//! O barco é uma entidade própria com [`vael_engine::terrain::FloatingBody`] (a
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

mod draft;
mod mooring;
mod rowing;
mod shore;
pub use mooring::Mooring;

use avian3d::prelude::{
    AngularInertia, AngularVelocity, CenterOfMass, Collider, Friction, LinearVelocity, Mass,
    MaxLinearSpeed, NoAutoAngularInertia, NoAutoCenterOfMass, NoAutoMass, Position, RigidBody,
    RigidBodyDisabled, Rotation, SleepingDisabled, SweptCcd,
};
use bevy::prelude::*;
use bevy_ext_core::Player;
use bevy_ext_water::buoyancy::avian::{AvianBuoyancyPlugin, AvianFloating};
use serde::{Deserialize, Serialize};

use crate::player::VaelController;
use vael_engine::recipes::spawn::OrbitCamera;
use vael_engine::terrain::runtime::TerrainRuntime;
use vael_engine::terrain::{
    FloatState, FloatingBody, WaterKinematics, WaterWaveWorld, WaveImpulseRequest,
};

/// Distância (m) do herói ao barco para entrar.
pub const BOARD_RADIUS: f32 = 3.4;
/// Assento do herói no barco (pés no piso interior, entre os travessões).
pub const SEAT_LOCAL: Vec3 = Vec3::new(0.0, -0.38, 0.09);
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
    /// Amplitude atual das pás (0 = recolhidas, 1 = remada completa).
    pub oar_effort: f32,
    /// Sentido da puxada (1 para avançar, -1 para recuar).
    pub oar_direction: f32,
    /// Contacto anterior de cada pá, para detetar a entrada na água.
    pub blade_wet: [bool; 2],
    pub mooring: Option<Mooring>,
}

impl Default for Boat {
    fn default() -> Self {
        Self {
            accel: 3.2,
            reverse: 0.78,
            max_speed: 4.0,
            sprint_mult: 1.5,
            turn_rate: 0.95,
            oar_phase: 0.0,
            oar_effort: 0.0,
            oar_direction: 1.0,
            blade_wet: [false; 2],
            mooring: None,
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
    /// Last evaluated palm socket, in hull space, for the water contact solve.
    grip: Option<Vec3>,
}

/// Os barcos de `data/boats.json` já nasceram nesta cena.
#[derive(Resource, Default)]
pub struct BoatsSpawned(pub bool);

/// Pose guardada de um barco: o `name` (a identidade do `data/boats.json`), a
/// posição de equilíbrio e o yaw.
///
/// Os barcos são despawnados em cada `SceneLoad` e só nascem no `main`, por
/// isso a pose tem de sobreviver fora das entidades ([`BoatStates`]) e viajar
/// no save ([`crate::save::VaelSave::boats`]) — sem ela, sair de `main` e
/// voltar (ou carregar um save a bordo) punha o barco no sítio autorado.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Reflect)]
#[serde(default, rename_all = "snake_case")]
pub struct BoatPose {
    pub name: String,
    pub position: Vec3,
    pub yaw: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mooring: Option<Mooring>,
}

/// Estado dos barcos entre cenas: o `spawn_boats` consulta as poses ANTES das
/// coordenadas autoradas do `data/boats.json`.
#[derive(Resource, Debug, Default, Clone)]
pub struct BoatStates {
    pub poses: Vec<BoatPose>,
    /// Barco em que o herói seguia. O `RidingBoat` guarda a entidade, que não
    /// sobrevive ao `SceneLoad` nem ao load de um save: o nome é a chave que
    /// volta a ligá-lo.
    pub riding: Option<String>,
    /// As poses vieram de um save e ainda não foram aplicadas aos barcos
    /// vivos. Enquanto é `true`, o espelho não as sobrepõe.
    pub pending: bool,
    /// O `riding` veio de um save e ainda não foi reposto (uma vez, quando o
    /// barco e o herói existirem).
    pub riding_pending: bool,
}

impl BoatStates {
    /// Estado a partir do save (as poses e o embarque ficam por aplicar).
    pub fn from_save(poses: Vec<BoatPose>, riding: Option<String>) -> Self {
        Self {
            riding_pending: riding.is_some(),
            poses,
            riding,
            pending: true,
        }
    }
}

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
        wave_response: 1.8,
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
        // As faces abaixo percorrem a caixa vistas de dentro.
        let mut face = |a, b, c, d| self.quad(d, c, b, a);
        // topo / fundo
        face(
            p(-1., 1., -1.),
            p(1., 1., -1.),
            p(1., 1., 1.),
            p(-1., 1., 1.),
        );
        face(
            p(-1., -1., 1.),
            p(1., -1., 1.),
            p(1., -1., -1.),
            p(-1., -1., -1.),
        );
        // lados
        face(
            p(-1., -1., 1.),
            p(-1., 1., 1.),
            p(1., 1., 1.),
            p(1., -1., 1.),
        );
        face(
            p(1., -1., -1.),
            p(1., 1., -1.),
            p(-1., 1., -1.),
            p(-1., -1., -1.),
        );
        face(
            p(-1., -1., -1.),
            p(-1., 1., -1.),
            p(-1., 1., 1.),
            p(-1., -1., 1.),
        );
        face(
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
    0.06 + 0.16 * (1.0 - (PI * t).sin().max(0.0).powf(0.8))
}

/// Cota da quilha ao longo do comprimento — rocker nas pontas.
fn hull_keel(t: f32) -> f32 {
    -0.54 + 0.22 * (1.0 - (PI * t).sin().max(0.0).powf(0.9))
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
                let a = hull_point(t0, us[j], inset);
                let b1 = hull_point(t1, us[j], inset);
                let c = hull_point(t1, us[j + 1], inset);
                let d = hull_point(t0, us[j + 1], inset);
                if inset == 0.0 {
                    b.quad(d, c, b1, a);
                } else {
                    b.quad(a, b1, c, d);
                }
            }
        }
    }
    // A borda acompanha o comprimento, ligando apenas os dois lábios.
    for i in 0..STATIONS {
        let t0 = i as f32 / STATIONS as f32;
        let t1 = (i + 1) as f32 / STATIONS as f32;
        for side in [-1.0, 1.0] {
            let (a, z) = if side > 0.0 { (t1, t0) } else { (t0, t1) };
            b.quad(
                hull_point(a, side, 0.0),
                hull_point(z, side, 0.0),
                hull_point(z, side, 1.0),
                hull_point(a, side, 1.0),
            );
        }
    }
    for t in [0.0_f32, 1.0] {
        let top = Vec3::new(0.0, hull_sheer(t), hull_point(t, 0.0, 0.0).z);
        for pair in us.windows(2) {
            let a = hull_point(t, pair[0], 0.0);
            let c = hull_point(t, pair[1], 0.0);
            if t > 0.5 {
                b.tri(a, c, top);
            } else {
                b.tri(c, a, top);
            }
        }
    }

    // Bancos (thwarts): dois travessões e o assento do remador.
    b.box_part(Vec3::new(0.0, -0.27, -0.85), Vec3::new(0.63, 0.045, 0.15));
    b.box_part(Vec3::new(0.0, -0.27, 0.75), Vec3::new(0.58, 0.045, 0.14));
    b.box_part(Vec3::new(0.0, -0.05, 0.05), Vec3::new(0.60, 0.045, 0.16));

    b.into_mesh()
}

fn hull_physics(body: &FloatingBody) -> impl Bundle {
    let inertia = Vec3::new(
        HULL_HALF.y.powi(2) + HULL_HALF.z.powi(2),
        HULL_HALF.x.powi(2) + HULL_HALF.z.powi(2),
        HULL_HALF.x.powi(2) + HULL_HALF.y.powi(2),
    ) * (body.mass / 3.0);
    let points: Vec<_> = (0..=8)
        .flat_map(|i| (-2..=2).map(move |j| hull_point(i as f32 / 8.0, j as f32 / 2.0, 0.0)))
        .collect();
    (
        RigidBody::Dynamic,
        Collider::convex_hull(&points).expect("rowboat hull has volume"),
        Mass(body.mass),
        AngularInertia::new(inertia),
        CenterOfMass(body.center_of_mass),
        NoAutoMass,
        NoAutoAngularInertia,
        NoAutoCenterOfMass,
        SleepingDisabled,
        SweptCcd::new(),
        Friction::new(0.6),
        MaxLinearSpeed(body.max_speed),
        AvianFloating,
    )
}

// Local +Z runs from the inboard handle to the blade.
const OAR_GRIP_Z: f32 = -0.55;
const OAR_BUTT_Z: f32 = OAR_GRIP_Z - 0.035;
const OAR_BLADE_Z: f32 = 1.90;

/// Turned wooden handle, tapered loom and thin blade, in one continuous surface.
pub fn rowboat_oar_mesh() -> Mesh {
    let mut mesh = MeshBuilder::default();
    let rings = [
        (OAR_BUTT_Z, 0.015, 0.015),
        (OAR_BUTT_Z + 0.006, 0.023, 0.023),
        (OAR_GRIP_Z + 0.080, 0.021, 0.021),
        (-0.32, 0.026, 0.026),
        (0.02, 0.034, 0.034),
        (1.42, 0.026, 0.026),
        (1.62, 0.056, 0.022),
        (OAR_BLADE_Z, 0.101, 0.015),
        (2.09, 0.104, 0.010),
        (2.14, 0.087, 0.008),
        (2.16, 0.055, 0.006),
    ];
    const SIDES: usize = 12;
    let vertex = |(z, rx, ry): (f32, f32, f32), index: usize| {
        let (sin, cos) = (2.0 * PI * index as f32 / SIDES as f32).sin_cos();
        Vec3::new(rx * cos, ry * sin, z)
    };
    let last = rings[rings.len() - 1];
    for i in 0..SIDES {
        let j = (i + 1) % SIDES;
        mesh.tri(
            Vec3::Z * OAR_BUTT_Z,
            vertex(rings[0], j),
            vertex(rings[0], i),
        );
        for pair in rings.windows(2) {
            mesh.quad(
                vertex(pair[0], i),
                vertex(pair[0], j),
                vertex(pair[1], j),
                vertex(pair[1], i),
            );
        }
        mesh.tri(Vec3::Z * last.0, vertex(last, i), vertex(last, j));
    }
    mesh.into_mesh()
}

// ── plugin ─────────────────────────────────────────────────────────────────

pub struct VaelBoatPlugin;

impl Plugin for VaelBoatPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(AvianBuoyancyPlugin::<TerrainRuntime>::default())
            .init_resource::<BoatsSpawned>()
            .init_resource::<BoatStates>()
            .add_systems(Startup, load_boat_assets)
            .add_systems(
                FixedPostUpdate,
                draft::stop_in_shallows
                    .after(avian3d::prelude::PhysicsSystems::StepSimulation)
                    .before(avian3d::prelude::PhysicsSystems::Writeback),
            )
            .add_systems(vael_engine::scene_loader::SceneLoad, despawn_boats_on_scene_load)
            .add_systems(
                Update,
                (
                    spawn_boats,
                    drive_boat_states,
                    recover_missing_boat,
                    shore::update_landing,
                    board_or_leave_boat,
                    drive_boats,
                    seat_riders,
                    rowing::choose_rider_state,
                    mooring::draw,
                )
                    .chain()
                    .after(crate::player::PlayerMovementSet)
                    .before(vael_engine::animation::drive_player_animation)
                    .run_if(crate::player::no_editor_camera),
            )
            .add_systems(
                Update,
                rowing::sync_rowing_clip.after(vael_engine::animation::drive_player_animation),
            )
            .add_systems(
                PostUpdate,
                rowing::animate_oars.after(TransformSystems::Propagate),
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
    rope: Handle<StandardMaterial>,
    rod: Handle<Mesh>,
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
        rod: meshes.add(Cylinder::new(1.0, 1.0).mesh().resolution(10)),
        rope: materials.add(Color::srgb(0.69, 0.50, 0.26)),
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
    scene: Option<Res<vael_engine::scene_loader::CurrentScene>>,
    root: Option<Res<crate::VaelProjectRoot>>,
    states: Option<Res<BoatStates>>,
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
        .is_some_and(|id| id != vael_engine::project::MAIN_SCENE)
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
        let body = rowboat_body();
        let saved = states.as_deref().and_then(|states| {
            states
                .poses
                .iter()
                .find(|pose| pose.name == spawn.name)
                .and_then(|pose| draft::fit_pose(&terrain, pose.clone()))
        });
        let Some(pose) = saved.or_else(|| {
            let y = terrain
                .water_surface_at(spawn.x, spawn.z)
                .unwrap_or_default()
                + body.rest_height();
            draft::fit_pose(
                &terrain,
                BoatPose {
                    name: spawn.name.clone(),
                    position: Vec3::new(spawn.x, y, spawn.z),
                    yaw: spawn.yaw,
                    mooring: None,
                },
            )
        }) else {
            warn!(
                "vael: sem profundidade para o barco `{}` junto a ({}, {})",
                spawn.name, spawn.x, spawn.z
            );
            continue;
        };
        let [x, y, z] = pose.position.to_array();
        let mooring = pose.mooring;
        let mut state = FloatState {
            yaw: pose.yaw,
            ..Default::default()
        };
        let mut position = Vec3::new(x, y, z);
        if mooring.is_none() {
            // Assenta já no equilíbrio (evita o "plop" de 0,5 m no arranque).
            let sample = |p: Vec2| {
                terrain
                    .water_domain_at(p.x, p.y)
                    .map(|d| vael_engine::terrain::FloatSample {
                        surface_y: Some(d.surface_y),
                        bed_y: terrain.in_field(p.x, p.y).then(|| terrain.sample(p.x, p.y)),
                        flow: d.flow,
                    })
                    .unwrap_or(vael_engine::terrain::FloatSample::dry(
                        terrain.in_field(p.x, p.y).then(|| terrain.sample(p.x, p.y)),
                    ))
            };
            for _ in 0..4 {
                vael_engine::terrain::water_buoyancy::step_float(
                    &body,
                    &mut state,
                    &mut position,
                    1.0 / 30.0,
                    sample,
                    |_, _| 0.0,
                );
            }
        }
        let spawn_tf = Transform {
            translation: position,
            rotation: mooring.map_or_else(|| state.rotation(), |m| m.rotation),
            ..Default::default()
        };
        let physics = hull_physics(&body);
        let boat = commands
            .spawn((
                Name::new(spawn.name.clone()),
                spawn_tf,
                physics,
                GlobalTransform::from(spawn_tf),
                Visibility::Inherited,
                body,
                state,
                Boat {
                    mooring,
                    ..Default::default()
                },
                WaterKinematics {
                    velocity: Vec3::ZERO,
                    body_radius: HULL_HALF.z.max(HULL_HALF.x) + 0.4,
                    contact_y_offset: HULL_CENTER.y - HULL_HALF.y,
                },
            ))
            .id();
        if mooring.is_some() {
            commands.entity(boat).insert(RigidBody::Static);
        }
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
                rowing::stowed_oar(side),
                Oar { side, grip: None },
                ChildOf(boat),
            ));
        }
    }
}

/// Tira o herói do barco em que estiver, sem tocar no resto do estado.
///
/// Um load manda na pose: o `RidingBoat` da sessão anterior prendia o herói ao
/// assento (`seat_riders`) em vez de o deixar no sítio guardado, por isso o
/// load limpa-o antes do teleporte e o `BoatStates` do save volta a embarcá-lo
/// se for esse o caso.
pub fn leave_boat_now(world: &mut World) {
    let Some(player) = world
        .query_filtered::<Entity, With<Player>>()
        .iter(world)
        .next()
    else {
        return;
    };
    world
        .entity_mut(player)
        .remove::<(RidingBoat, RigidBodyDisabled)>();
}

/// Mantém o [`BoatStates`] vivo entre cenas e aplica o que veio de um save.
///
/// - `pending`: as poses vieram do load; aplicam-se aos barcos vivos (os que
///   nasceram neste frame já usaram as poses no `spawn_boats`).
/// - `riding_pending`: o embarque guardado volta assim que o barco e o herói
///   existem (uma vez só — um E a seguir tira o herói e ninguém o põe lá
///   outra vez).
/// - sem nada por aplicar: espelha as poses dos barcos VIVOS. Sem barcos
///   (interior, menu) conserva a última pose conhecida do `main` — é isso que
///   faz um barco voltar ao sítio onde ficou quando o herói reentra no mundo.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn drive_boat_states(
    mut commands: Commands,
    terrain: Option<Res<TerrainRuntime>>,
    mut states: ResMut<BoatStates>,
    // `ParamSet`, não três queries soltas: `p0` escreve `Transform`/`FloatState`
    // e os filtros `Changed<>` de `p1`/`p2` LEEM os mesmos componentes, e o
    // Bevy valida isso no arranque do sistema (`error[B0001]`) mesmo que nunca
    // sejam usados no mesmo frame.
    mut params: ParamSet<(
        Query<(Entity, &Name, &mut Transform, &mut FloatState, &mut Boat)>,
        Query<
            (),
            (
                With<Boat>,
                Or<(Changed<Boat>, Changed<Transform>, Changed<FloatState>)>,
            ),
        >,
        Query<(), (With<Player>, Or<(Added<RidingBoat>, Changed<RidingBoat>)>)>,
    )>,
    mut removed_boats: RemovedComponents<Boat>,
    names: Query<(Entity, &Name), With<Boat>>,
    players: Query<(Entity, Option<&RidingBoat>), With<Player>>,
    mut removed_riders: RemovedComponents<RidingBoat>,
    mut needs_sync: Local<bool>,
) {
    let player = players.iter().next();

    let applied_pending = states.pending;
    if applied_pending {
        states.pending = false;
        *needs_sync = true;
        // A query sai do `ParamSet` uma vez: `params.p0().iter_mut()` deixava o
        // temporário cair a meio do `let … else` (E0716).
        let mut boats = params.p0();
        for pose in &states.poses {
            let Some((entity, _, mut transform, mut float, mut boat)) = boats
                .iter_mut()
                .find(|(_, name, _, _, _)| name.as_str() == pose.name)
            else {
                continue;
            };
            let pose = match terrain.as_deref() {
                Some(terrain) => draft::fit_pose(terrain, pose.clone()),
                None => Some(pose.clone()),
            };
            let Some(pose) = pose else { continue };
            transform.translation = pose.position;
            let mooring = pose
                .mooring
                .filter(|_| states.riding.as_deref() != Some(pose.name.as_str()));
            mooring::set(&mut commands, entity, &mut boat, mooring);
            transform.rotation =
                mooring.map_or_else(|| Quat::from_rotation_y(pose.yaw), |m| m.rotation);
            commands.entity(entity).insert((
                Position(pose.position),
                Rotation(transform.rotation),
                LinearVelocity::ZERO,
                AngularVelocity::ZERO,
            ));
            float.yaw = pose.yaw;
            float.velocity = Vec3::ZERO;
            float.pitch = 0.0;
            float.roll = 0.0;
            float.pitch_vel = 0.0;
            float.roll_vel = 0.0;
        }
    }

    if states.riding_pending {
        // O `RidingBoat` guarda a entidade, que o save não pode guardar: o
        // nome do barco é a chave. Só se repõe quando ninguém já está a bordo.
        let Some(name) = states.riding.clone() else {
            states.riding_pending = false;
            return;
        };
        // Procura por nome numa query imutável: o `iter_mut` do `match`
        // prolongava o empréstimo mutável até ao guard do braço `None`.
        let boat = names
            .iter()
            .find(|(_, boat_name)| boat_name.as_str() == name)
            .map(|(boat, _)| boat);
        let have_boats = !names.is_empty();
        match boat {
            Some(boat) => {
                if let Some((player, riding)) = player {
                    if riding.is_none() {
                        commands
                            .entity(player)
                            .insert((RidingBoat { boat }, RigidBodyDisabled));
                    }
                    states.riding_pending = false;
                    *needs_sync = true;
                }
            }
            // Os barcos já nasceram e nenhum é o guardado: o barco saiu do
            // mundo (nome mudou no `data/boats.json`) — não insistir.
            None if have_boats => {
                states.riding_pending = false;
                states.riding = None;
            }
            None => {}
        }
        // Enquanto o embarque está por repor, as poses do save continuam a
        // mandar (nada de espelhar por cima delas).
        return;
    }
    if applied_pending {
        // As poses do save acabaram de ser aplicadas: não as sobrepor já com
        // o espelho neste frame.
        return;
    }

    let removed_boat_count = removed_boats.read().count();
    let removed_rider_count = removed_riders.read().count();
    if !*needs_sync
        && params.p1().is_empty()
        && removed_boat_count == 0
        && params.p2().is_empty()
        && removed_rider_count == 0
    {
        return;
    }
    *needs_sync = false;

    let desired_riding = player.and_then(|(_, riding)| {
        riding.and_then(|r| names.get(r.boat).ok().map(|(_, n)| n.as_str()))
    });
    let riding_differs = player.is_some() && states.riding.as_deref() != desired_riding;

    let boat_count = params.p0().iter().len();
    let poses_differ = if boat_count == 0 {
        false
    } else if states.poses.len() != boat_count {
        true
    } else {
        params.p0().iter().any(|(_, name, transform, float, boat)| {
            match states.poses.iter().find(|p| p.name == name.as_str()) {
                Some(prev) => {
                    prev.position != transform.translation
                        || prev.yaw != float.yaw
                        || prev.mooring != boat.mooring
                }
                None => true,
            }
        })
    };

    if poses_differ {
        // Atualiza no lugar sempre que a lista de nomes não mudou, evitando alocar `Vec`/`String`.
        let names_match = states.poses.len() == boat_count
            && params
                .p0()
                .iter()
                .all(|(_, n, _, _, _)| states.poses.iter().any(|p| p.name == n.as_str()));
        if names_match {
            for (_, name, transform, float, boat) in params.p0().iter() {
                if let Some(p) = states.poses.iter_mut().find(|p| p.name == name.as_str()) {
                    p.position = transform.translation;
                    p.yaw = float.yaw;
                    p.mooring = boat.mooring;
                }
            }
        } else {
            let mut poses: Vec<BoatPose> = params
                .p0()
                .iter()
                .map(|(_, name, transform, float, boat)| BoatPose {
                    name: name.as_str().to_string(),
                    position: transform.translation,
                    yaw: float.yaw,
                    mooring: boat.mooring,
                })
                .collect();
            poses.sort_by(|a, b| a.name.cmp(&b.name));
            states.poses = poses;
        }
    }
    if riding_differs {
        states.riding = desired_riding.map(str::to_string);
    }
}

fn interaction_just_pressed(
    actions: Option<&vael_engine::input_map::ActionState>,
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
    actions: Option<Res<vael_engine::input_map::ActionState>>,
    typing: Option<Res<vael_engine::ui::widgets::UiFocusedInput>>,
    menus: Option<Res<bevy_ext_core::MenusOpen>>,
    terrain: Option<Res<TerrainRuntime>>,
    spatial: avian3d::prelude::SpatialQuery,
    sensors: Query<(), With<avian3d::prelude::Sensor>>,
    mut players: Query<
        (
            Entity,
            &Transform,
            Option<&RidingBoat>,
            Option<&RigidBodyDisabled>,
        ),
        With<Player>,
    >,
    mut boats: Query<(
        Entity,
        &Transform,
        &FloatState,
        Option<&shore::Landing>,
        &mut Boat,
    )>,
    mut cameras: Query<&mut OrbitCamera>,
) {
    // Um menu aberto (mapa/diário/pausa) ou a barra de fala com o foco
    // bloqueiam o E — no caminho do `ActionState` quem bloqueia é o
    // `gather_actions`, mas o fallback de teclas cruas entrava no barco por
    // baixo do menu.
    let blocked = menus.as_ref().is_some_and(|menus| menus.any())
        || typing.as_ref().is_some_and(|typing| typing.0.is_some());
    let pressed = !blocked && interaction_just_pressed(actions.as_deref(), &keys);
    if !pressed {
        return;
    }
    let Some((player, player_tf, riding, _)) = players.iter_mut().next() else {
        return;
    };

    if let Some(riding) = riding {
        // Sair: repõe o corpo e larga o herói ao lado do casco.
        let Ok((_, boat_tf, float, landing, mut boat)) = boats.get_mut(riding.boat) else {
            commands
                .entity(player)
                .remove::<(RidingBoat, RigidBodyDisabled)>();
            return;
        };
        let filter = avian3d::prelude::SpatialQueryFilter::default()
            .with_excluded_entities([player, riding.boat]);
        let Some(drop) = landing.map(|s| s.0).or_else(|| {
            shore::swim_exit(boat_tf, terrain.as_deref()?, &spatial, &filter, &sensors)
        }) else {
            return;
        };
        if let (Some(spot), Some(rt)) = (landing, terrain.as_deref()) {
            mooring::set(
                &mut commands,
                riding.boat,
                &mut boat,
                Some(Mooring::at(spot.0, boat_tf, rt)),
            );
        }
        if landing.is_none() {
            let vessel = riding.boat;
            commands.queue(move |world: &mut World| {
                if let (Some(name), Some(bus)) = (world.get::<Name>(vessel), world.get_resource::<crate::player::HeroMotionBus>()) {
                    bus.lock().stamina.boat = Some(name.to_string());
                }
            });
        }
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
    for (entity, boat_tf, _, _, boat) in &boats {
        let mut d = boat_tf.translation.distance(player_tf.translation);
        if let Some(m) = boat.mooring
            && m.stake.distance(player_tf.translation) < 1.4
        {
            d = d.min(BOARD_RADIUS);
        }
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
    if let Ok((_, _, float, _, mut vessel)) = boats.get_mut(boat) {
        mooring::set(&mut commands, boat, &mut vessel, None);
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
    actions: Option<Res<vael_engine::input_map::ActionState>>,
    typing: Option<Res<vael_engine::ui::widgets::UiFocusedInput>>,
    menus: Option<Res<bevy_ext_core::MenusOpen>>,
    terrain: Option<Res<TerrainRuntime>>,
    mut waves: Option<ResMut<WaterWaveWorld>>,
    oars: Query<(&ChildOf, &Oar)>,
    mut boats: Query<(
        Entity,
        &Transform,
        &mut Boat,
        &FloatState,
        &mut LinearVelocity,
        &mut AngularVelocity,
    )>,
    riders: Query<&RidingBoat>,
) {
    let ridden = riders.iter().next().map(|rider| rider.boat);
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
    for (entity, hull, mut boat, float, mut linear, mut angular) in &mut boats {
        let aboard = Some(entity) == ridden;
        if !aboard && boat.oar_effort == 0.0 {
            continue;
        }
        let target = if aboard { throttle } else { 0.0 };
        advance_stroke(&mut boat, target, sprint, dt);
        let effort = boat.oar_effort;
        let pulling = target * boat.oar_direction > 0.001;
        let mut grips = [None; 2];
        for (parent, oar) in &oars {
            if parent.parent() == entity {
                grips[usize::from(oar.side > 0.0)] = oar.grip;
            }
        }
        let mut immersed = 0;
        for (i, side) in [-1.0, 1.0].into_iter().enumerate() {
            let pose = rowing::oar_pose(&boat, side, grips[i]);
            let tip = hull.transform_point(pose.transform_point(Vec3::Z * OAR_BLADE_Z));
            let wet = aboard
                && effort > 0.001
                && terrain
                    .as_ref()
                    .and_then(|t| t.water_domain_at(tip.x, tip.z))
                    .is_some_and(|d| d.depth > 0.10 && tip.y <= d.surface_y);
            if wet {
                immersed += 1;
                if !boat.blade_wet[i]
                    && let Some(waves) = waves.as_deref_mut()
                {
                    waves.push_impulse(WaveImpulseRequest {
                        pos: tip.xz(),
                        vel: Vec2::ZERO,
                        radius: 0.40,
                        strength: 0.10 + 0.08 * effort,
                        foam: 0.28 + 0.18 * effort,
                        is_wake: false,
                    });
                }
            }
            boat.blade_wet[i] = wet;
        }
        if aboard {
            let yaw = hull.rotation.to_euler(EulerRot::YXZ).0;
            let speed = linear.0.xz().length();
            let forward = Vec3::new(yaw.sin(), 0.0, yaw.cos());
            let along = linear.0.dot(forward);
            // Check the boat's way immediately when the rower backs water.
            if target * along < 0.0 && float.submerged > 0.01 {
                linear.0 -= forward * along.signum() * along.abs().min(5.5 * target.abs() * dt);
            }
            // A bow on the bank can leave both blades above water. Push off
            // the bed only if there is water within reach beyond the stern.
            if target < -0.001 && float.grounded {
                let stern = hull.translation - forward * (HULL_HALF.z + 0.65);
                if terrain
                    .as_ref()
                    .and_then(|t| t.water_domain_at(stern.x, stern.z))
                    .is_some_and(|d| d.depth > 0.12)
                {
                    let push = (along + 1.5).max(0.0).min(12.0 * target.abs() * dt);
                    linear.0 -= forward * push;
                }
            }
            // A bank can roll a light hull past its stable range. Right an
            // occupied boat progressively, leaving normal wave motion alone.
            let deck_up = hull.rotation * Vec3::Y;
            if deck_up.y < 0.82 {
                let axis = deck_up.cross(Vec3::Y).try_normalize().unwrap_or(forward);
                let righting = 12.0 * (0.82 - deck_up.y);
                let tilt_rate = angular.0 - angular.y * Vec3::Y;
                angular.0 += (axis * righting - 3.0 * tilt_rate) * dt;
            }
            if pulling {
                let (phase, rate) = stroke_timing(boat.oar_phase);
                let drive = phase.sin().max(0.0) * rate * PI * immersed as f32 * 0.5;
                linear.0 = row_thrust(
                    linear.0,
                    yaw,
                    boat.oar_direction * effort,
                    sprint,
                    &boat,
                    dt * drive,
                );
            }
            if steer.abs() > 0.001 {
                let target = row_yaw(0.0, steer, speed, &boat, 1.0);
                angular.y += (target - angular.y).clamp(-2.8 * dt, 2.8 * dt);
            }
        }
    }
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
        let Ok((boat_tf, _)) = boats.get(riding.boat) else {
            continue;
        };
        transform.translation = boat_tf.translation + boat_tf.rotation * SEAT_LOCAL;
        transform.rotation = boat_tf.rotation * Quat::from_rotation_y(PI);
        // A velocidade do casco não é marcha do herói sentado.
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

fn oar_pivot(side: f32) -> Vec3 {
    Vec3::new(0.76 * side, 0.17, -0.07)
}

fn advance_stroke(boat: &mut Boat, target: f32, sprint: bool, dt: f32) {
    let current = boat.oar_direction * boat.oar_effort;
    let effort = current + (target - current).clamp(-4.0 * dt, 4.0 * dt);
    boat.oar_effort = effort.abs();
    if effort != 0.0 {
        if effort.signum() != boat.oar_direction {
            // Cross neutral with the hands centered; start the next drive now
            // instead of waiting through the previous direction's recovery.
            boat.oar_phase = 0.45;
        }
        boat.oar_direction = effort.signum();
        let cadence = if sprint && target > 0.0 { 4.7 } else { 3.5 };
        boat.oar_phase = (boat.oar_phase + cadence * dt).rem_euclid(2.0 * PI);
    } else if target == 0.0 {
        boat.oar_phase = 0.0;
    }
}

// Quicker loaded drive and a longer recovery, with continuous angular speed.
// The derivative also preserves the integrated thrust per full stroke.
fn stroke_timing(clock: f32) -> (f32, f32) {
    (clock + 0.28 * (1.0 - clock.cos()), 1.0 + 0.28 * clock.sin())
}

fn oar_rotation(boat: &Boat, side: f32) -> Quat {
    let (pull, sweep) = stroke_timing(boat.oar_phase).0.sin_cos();
    let yaw = side * (PI * 0.5 - 0.30 * sweep * boat.oar_direction * boat.oar_effort);
    let dip = 0.045 + boat.oar_effort * (0.155 + 0.29 * pull);
    // Square just before entry, hold through the drive and feather at extraction.
    let square = ((pull + 0.34) / 0.72).clamp(0.0, 1.0);
    let feather = side * boat.oar_effort * 1.38 * square * square * (3.0 - 2.0 * square);
    Quat::from_rotation_y(yaw) * Quat::from_rotation_x(dip) * Quat::from_rotation_z(feather)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lake_with_rowboat() -> (App, Entity) {
        use avian3d::prelude::PhysicsPlugins;
        use bevy::time::TimeUpdateStrategy;
        use bevy_ext_water::{WaterBody, WaterDomainSample, WaterSource, obstacles::WaterObstacle};
        #[derive(Resource)]
        struct Lake;
        impl WaterSource for Lake {
            fn bodies(&self) -> &[WaterBody] {
                &[]
            }
            fn bed_y(&self, _: f32, _: f32) -> Option<f32> {
                Some(-3.0)
            }
            fn domain_at(&self, _: Vec2, _: &[WaterObstacle]) -> Option<WaterDomainSample> {
                Some(WaterDomainSample {
                    surface_y: 0.,
                    depth: 3.,
                    flow: Vec2::ZERO,
                    plunge: 0.,
                })
            }
        }
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            TransformPlugin,
            AssetPlugin::default(),
            PhysicsPlugins::default(),
            bevy_ext_water::WaterBuoyancyPlugin::<Lake>::default(),
            AvianBuoyancyPlugin::<Lake>::default(),
        ));
        app.init_asset::<Mesh>();
        app.insert_resource(Lake);
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f64(1.0 / 60.0),
        ));
        let body = rowboat_body();
        let boat = app
            .world_mut()
            .spawn((
                hull_physics(&body),
                Transform::from_xyz(0., body.rest_height(), 0.),
                body,
                FloatState::default(),
            ))
            .id();
        app.finish();
        app.cleanup();
        (app, boat)
    }

    #[test]
    fn moored_hull_resumes_floating_without_a_launch_impulse() {
        let (mut app, boat) = lake_with_rowboat();
        for _ in 0..120 {
            app.update();
        }
        app.world_mut().entity_mut(boat).insert((
            RigidBody::Static,
            LinearVelocity::ZERO,
            AngularVelocity::ZERO,
        ));
        let tied_y = app.world().get::<Position>(boat).unwrap().y;
        for _ in 0..600 {
            app.update();
        }
        assert_eq!(app.world().get::<Position>(boat).unwrap().y, tied_y);
        app.world_mut().entity_mut(boat).insert(RigidBody::Dynamic);
        for _ in 0..120 {
            app.update();
            let rise = app.world().get::<Position>(boat).unwrap().y - tied_y;
            assert!(rise < 0.25, "hull rose {rise:.3} m on release");
        }
    }

    #[test]
    fn floating_hull_rocks_and_stops_against_a_thin_obstacle() {
        let (mut app, boat) = lake_with_rowboat();
        app.world_mut().spawn((
            RigidBody::Static,
            Collider::cuboid(12.0, 6.0, 0.16),
            Transform::from_xyz(0., 0., 5.0),
        ));
        let mut min_roll = f32::INFINITY;
        let mut max_roll = f32::NEG_INFINITY;
        for _ in 0..150 {
            app.update();
            let state = app.world().get::<FloatState>(boat).unwrap();
            min_roll = min_roll.min(state.roll);
            max_roll = max_roll.max(state.roll);
        }
        assert!(
            max_roll - min_roll > 0.02,
            "roll range {min_roll}..{max_roll}"
        );
        assert!(app.world().get::<Position>(boat).unwrap().y.abs() < 0.8);
        app.world_mut().get_mut::<LinearVelocity>(boat).unwrap().0 = Vec3::Z * 8.5;
        let mut furthest = 0.0_f32;
        for _ in 0..100 {
            app.update();
            furthest = furthest.max(app.world().get::<Position>(boat).unwrap().z);
        }
        assert!(
            (2.5..3.1).contains(&furthest),
            "bow contact at centre z={furthest}"
        );
    }

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
    fn rower_tracks_the_oars_at_rest_under_power_and_in_reverse() {
        let mut boat = Boat::default();
        let seated = rowing::rowing_clip_time(&boat);
        boat.oar_effort = 1.0;
        let catch = rowing::rowing_clip_time(&boat);
        boat.oar_phase = PI;
        let finish = rowing::rowing_clip_time(&boat);
        assert!(catch < seated && seated < finish);
        boat.oar_direction = -1.0;
        assert!(rowing::rowing_clip_time(&boat) < seated);
        boat.oar_effort = 0.0;
        assert_eq!(rowing::rowing_clip_time(&boat), seated);
    }

    #[test]
    fn blades_dip_together_outside_the_hull_then_clear_the_surface() {
        let height = rowboat_body().rest_height();
        let mut boat = Boat {
            oar_effort: 1.0,
            ..Default::default()
        };
        for side in [-1.0, 1.0] {
            for (phase, wet) in [(PI * 0.5, true), (PI * 1.5, false)] {
                boat.oar_phase = phase;
                let tip = oar_pivot(side) + oar_rotation(&boat, side) * (Vec3::Z * OAR_BLADE_Z);
                assert!(tip.x * side > HULL_HALF.x + 1.0);
                assert_eq!(height + tip.y < 0.0, wet);
            }
        }
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
        assert!(pos.iter().flatten().all(|v| v.is_finite()));
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
        let butt = opos.iter().map(|p| p[2]).fold(f32::INFINITY, f32::min);
        assert!((OAR_GRIP_Z - butt - 0.035).abs() < 1e-5);
        assert!(
            opos.iter()
                .filter(|p| p[2] < OAR_GRIP_Z + 0.085)
                .all(|p| p[0].hypot(p[1]) <= 0.024)
        );
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

    /// O estado aplicado pelo load: posição/yaw aos barcos vivos e o embarque
    /// outra vez ligado ao barco pelo nome.
    #[test]
    fn loaded_mooring_freezes_the_hull_and_a_free_save_releases_it() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<BoatStates>()
            .add_systems(Update, drive_boat_states);
        let id = app
            .world_mut()
            .spawn((
                Name::new("barco"),
                Boat::default(),
                Transform::default(),
                FloatState::default(),
                RigidBody::Dynamic,
            ))
            .id();
        let tie = Mooring {
            stake: Vec3::new(8.0, 1.0, 5.0),
            rotation: Quat::from_euler(EulerRot::YXZ, 0.5, 0.04, -0.1),
        };
        let mut pose = BoatPose {
            name: "barco".into(),
            position: Vec3::new(6.0, 1.0, 4.0),
            yaw: 0.5,
            mooring: Some(tie),
        };
        app.insert_resource(BoatStates::from_save(vec![pose.clone()], None));
        app.update();
        assert_eq!(app.world().get::<RigidBody>(id), Some(&RigidBody::Static));
        assert_eq!(
            app.world().get::<Transform>(id).unwrap().rotation,
            tie.rotation
        );
        assert_eq!(app.world().get::<Boat>(id).unwrap().mooring, Some(tie));
        pose.mooring = None;
        app.insert_resource(BoatStates::from_save(vec![pose], None));
        app.update();
        assert_eq!(app.world().get::<RigidBody>(id), Some(&RigidBody::Dynamic));
        assert_eq!(app.world().get::<Boat>(id).unwrap().mooring, None);
    }

    #[test]
    fn boat_states_aplicam_a_pose_guardada_e_o_embarque() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<BoatStates>()
            .add_systems(Update, drive_boat_states);
        *app.world_mut().resource_mut::<BoatStates>() = BoatStates::from_save(
            vec![BoatPose {
                name: "barco-a".into(),
                position: Vec3::new(10.0, 1.0, -4.0),
                yaw: 0.75,
                mooring: None,
            }],
            Some("barco-a".into()),
        );
        let boat = app
            .world_mut()
            .spawn((
                Name::new("barco-a"),
                Transform::default(),
                FloatState::default(),
                Boat::default(),
            ))
            .id();
        let player = app.world_mut().spawn(Player::default()).id();

        app.update();

        let transform = *app.world().get::<Transform>(boat).unwrap();
        assert_eq!(transform.translation, Vec3::new(10.0, 1.0, -4.0));
        assert!((app.world().get::<FloatState>(boat).unwrap().yaw - 0.75).abs() < 1e-6);
        let riding = app.world().get::<RidingBoat>(player).expect("a bordo");
        assert_eq!(riding.boat, boat);
        assert!(app.world().get::<RigidBodyDisabled>(player).is_some());
        let states = app.world().resource::<BoatStates>();
        assert!(!states.pending);
        assert!(!states.riding_pending);
    }

    /// O embarque espera pelo barco (num load que troca de cena o barco nasce
    /// depois) e não se repete depois de um E.
    #[test]
    fn o_embarque_do_save_espera_pelo_barco_e_nao_se_repete() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<BoatStates>()
            .add_systems(Update, drive_boat_states);
        *app.world_mut().resource_mut::<BoatStates>() =
            BoatStates::from_save(Vec::new(), Some("barco-a".into()));
        let player = app.world_mut().spawn(Player::default()).id();

        // O barco ainda não existe: o embarque fica à espera.
        app.update();
        assert!(app.world().resource::<BoatStates>().riding_pending);
        assert!(app.world().get::<RidingBoat>(player).is_none());

        // Nasceu: o herói volta a bordo.
        let boat = app
            .world_mut()
            .spawn((
                Name::new("barco-a"),
                Transform::default(),
                FloatState::default(),
                Boat::default(),
            ))
            .id();
        app.update();
        assert_eq!(app.world().get::<RidingBoat>(player).unwrap().boat, boat);
        assert!(!app.world().resource::<BoatStates>().riding_pending);

        // O jogador sai (E): o estado segue o ECS, não o save.
        app.world_mut().entity_mut(player).remove::<RidingBoat>();
        app.update();
        assert!(app.world().get::<RidingBoat>(player).is_none());
        assert_eq!(app.world().resource::<BoatStates>().riding, None);
    }

    /// Sem barcos (interior/menu) o estado guardado NÃO é limpo: é ele que
    /// devolve o barco ao sítio onde ficou quando o herói volta ao `main`.
    #[test]
    fn boat_states_sobrevivem_a_uma_cena_sem_barcos() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<BoatStates>()
            .add_systems(Update, drive_boat_states);
        *app.world_mut().resource_mut::<BoatStates>() = BoatStates::from_save(
            vec![BoatPose {
                name: "barco-a".into(),
                position: Vec3::new(10.0, 1.0, -4.0),
                yaw: 0.75,
                mooring: None,
            }],
            None,
        );
        app.world_mut().resource_mut::<BoatStates>().pending = false;

        app.update();

        let states = app.world().resource::<BoatStates>();
        assert_eq!(states.poses.len(), 1, "a pose continua guardada");
        assert_eq!(states.poses[0].name, "barco-a");
    }

    /// Com barcos vivos o espelho segue a física e o embarque real.
    #[test]
    fn boat_states_espelham_os_barcos_vivos() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<BoatStates>()
            .add_systems(Update, drive_boat_states);
        let boat = app
            .world_mut()
            .spawn((
                Name::new("barco-b"),
                Transform::from_xyz(3.0, 0.5, 5.0),
                FloatState {
                    yaw: 0.25,
                    ..Default::default()
                },
                Boat::default(),
            ))
            .id();
        app.world_mut()
            .spawn((Player::default(), RidingBoat { boat }));

        app.update();

        let states = app.world().resource::<BoatStates>();
        assert_eq!(states.poses.len(), 1);
        assert_eq!(states.poses[0].name, "barco-b");
        assert_eq!(states.poses[0].position, Vec3::new(3.0, 0.5, 5.0));
        assert!((states.poses[0].yaw - 0.25).abs() < 1e-6);
        assert_eq!(states.riding.as_deref(), Some("barco-b"));
    }

    /// A pose viaja pelo serde (é o campo `boats` do `VaelSave`).
    #[test]
    fn boat_pose_serde_ida_e_volta() {
        let pose = BoatPose {
            name: "barco-a".into(),
            position: Vec3::new(1.0, 2.0, 3.0),
            yaw: -0.5,
            mooring: None,
        };
        let text = serde_json::to_string(&pose).expect("serializa");
        assert_eq!(serde_json::from_str::<BoatPose>(&text).unwrap(), pose);
        // Um objeto antigo/vazio cai nos defaults.
        assert_eq!(BoatPose::default(), serde_json::from_str("{}").unwrap());
    }
}

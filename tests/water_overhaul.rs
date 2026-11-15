//! Integration tests for Vael's optical/physical water overhaul:
//! - Dense tessellated meshes with flow attributes (`lake_water_mesh_dense`, `river_water_mesh_dense`)
//! - Dispersive iWave kernel & 128 m rolling compute window/domain rasterizer (`WaterWaveWorld`)
//! - Two-pass `Multiply` + `Add` `WaterMaterial` environment/weather sync (`WaterWavePlugin`)
//! - Swept plunger craters & continuous Kelvin bow-stern wakes (`WaterFxPlugin` + `WaterKinematics`)
//! - Player submersion drag, entry impact damping, and Archimedes swimming buoyancy

use std::sync::Arc;

use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use viber::ambient::AmbientPlugin;
use viber::terrain::brush::BrushGrid;
use viber::terrain::delta::DeltaGrid;
use viber::terrain::paint::PaintGrid;
use viber::terrain::runtime::TerrainRuntime;
use viber::terrain::spec::TerrainSpec;
use viber::terrain::splat::SplatTuning;
use viber::terrain::voxel::VoxelField;
use viber::terrain::water::{
    LakeSpec, RiverSpec, WaterBody, carve_lake, carve_river, lake_water_mesh_dense,
    river_water_mesh_dense,
};
use viber::terrain::water_buoyancy::{FloatState, FloatingBody, WaterBuoyancyPlugin};
use viber::terrain::water_fx::{
    WADE_ENTER_DEPTH, WaterContact, WaterFxPlugin, compute_entry_impulse, compute_wake_impulse,
    swept_entry_xz,
};
use viber::terrain::water_material::{WATER_WGSL, WaterExtension, WaterMaterial, WaterUniforms};
use viber::terrain::water_waves::{
    DOMAIN_GRID_SIZE, WATER_WAVES_WGSL, WaterKinematics, WaterWavePlugin, WaterWaveWorld,
    compute_iwave_kernel, rasterize_domain_into_bytes,
};
use viber::water_ssr::WATER_SSR_WGSL;
use viber::worldsys::{AtmosphereState, WeatherState};

fn make_flat_grid(world_size: f32, side: usize, y_m: f32) -> BrushGrid {
    let max_h = 50.0_f32;
    let raw = ((y_m / max_h).clamp(0.0, 1.0) * u16::MAX as f32).round() as u16;
    BrushGrid::new(vec![raw; side * side], side, side, world_size, max_h, 0.0)
        .expect("valid test brush grid")
}

fn make_test_runtime(grid: BrushGrid, water: Vec<WaterBody>) -> TerrainRuntime {
    let mut spec = TerrainSpec::default();
    spec.world_size = grid.world_size();
    let side = grid.width();
    TerrainRuntime {
        spec,
        grid: Arc::new(grid),
        water,
        roads: Vec::new(),
        pads: Vec::new(),
        voxel: Arc::new(VoxelField::new(Vec::new(), 256.0, 64.0)),
        deltas: Arc::new(DeltaGrid::default()),
        paint: Arc::new(PaintGrid::new(256.0, side)),
        splat_tuning: SplatTuning::default(),
    }
}

#[test]
fn dense_lake_and_river_meshes_carry_flow_velocity_and_domain_samples() {
    let mut grid = make_flat_grid(256.0, 128, 12.0);
    let mut lake_spec = LakeSpec::default();
    lake_spec.at = Vec2::new(-40.0, -30.0);
    lake_spec.radius = 16.0;
    lake_spec.depth = 3.2;

    let lake_body = carve_lake(&mut grid, &lake_spec, 0).expect("lake carves cleanly");
    let lake_mesh = lake_water_mesh_dense(&lake_spec, lake_body.water_y, &grid);
    assert!(
        lake_mesh.mesh.positions.len() > 600,
        "dense lake should have high tessellation, got {}",
        lake_mesh.mesh.positions.len()
    );
    assert_eq!(lake_mesh.mesh.positions.len(), lake_mesh.uv_b.len());
    assert!(
        lake_mesh.uv_b.iter().all(|v| v[0] == 0.0 && v[1] == 0.0),
        "calm lake carries zero downstream velocity"
    );

    let mut river_spec = RiverSpec::default();
    river_spec.path = vec![
        Vec2::new(10.0, -40.0),
        Vec2::new(14.0, -10.0),
        Vec2::new(22.0, 20.0),
        Vec2::new(30.0, 50.0),
    ];
    river_spec.width = 8.0;
    river_spec.depth = 2.0;

    let river_body = carve_river(&mut grid, &river_spec, 1, &[]).expect("river carves cleanly");
    let river_mesh = river_water_mesh_dense(&river_spec, &river_body);
    assert!(
        river_mesh.mesh.positions.len() > 400,
        "dense river should have fine longitudinal + 15-vertex cross tessellation, got {}",
        river_mesh.mesh.positions.len()
    );
    assert_eq!(river_mesh.mesh.positions.len(), river_mesh.uv_b.len());

    let max_flow_speed = river_mesh
        .uv_b
        .iter()
        .map(|v| (v[0] * v[0] + v[1] * v[1]).sqrt())
        .fold(0.0_f32, f32::max);
    assert!(
        max_flow_speed > 0.8,
        "river flow speed must be positive: {max_flow_speed}"
    );

    let sample_center = river_body
        .sample_domain(Vec2::new(14.0, -10.0), None, 2.0)
        .expect("center inside river");
    assert!(sample_center.depth > 0.5);
    assert!(sample_center.flow.length() > 0.5);
    assert!(
        lake_body
            .sample_domain(Vec2::new(-40.0, -30.0), None, 3.2)
            .expect("inside lake")
            .depth
            > 1.0
    );
}

#[test]
fn wave_kernel_rolling_window_and_domain_rasterizer() {
    let (packed, g_eff) = compute_iwave_kernel();
    assert!(g_eff > 15.0);

    // Zero DC sum & non-negative 2D Brillouin spectrum.
    let weight_at = |dx: isize, dz: isize| -> f64 {
        let idx = dz.unsigned_abs() * 6 + dx.unsigned_abs();
        packed[idx >> 2][idx & 3] as f64
    };
    let mut sum = 0.0;
    for dz in -5isize..=5 {
        for dx in -5isize..=5 {
            sum += weight_at(dx, dz);
        }
    }
    assert!(sum.abs() < 1e-4, "DC sum must vanish: {sum}");

    let mut min_spec = f64::INFINITY;
    for kz_i in 0..=24 {
        let kz = (kz_i as f64 / 24.0) * std::f64::consts::PI;
        for kx_i in 0..=24 {
            let kx = (kx_i as f64 / 24.0) * std::f64::consts::PI;
            let mut re = 0.0;
            for dz in -5isize..=5 {
                for dx in -5isize..=5 {
                    re += weight_at(dx, dz) * (kx * dx as f64 + kz * dz as f64).cos();
                }
            }
            min_spec = min_spec.min(re);
        }
    }
    assert!(min_spec >= -1e-4, "negative eigenvalue {min_spec}");

    // Domain rasterizer populates non-zero texels around a carved lake.
    let mut grid = make_flat_grid(256.0, 128, 10.0);
    let mut lake_spec = LakeSpec::default();
    lake_spec.at = Vec2::new(20.0, 20.0);
    lake_spec.radius = 12.0;
    lake_spec.depth = 3.0;
    let body = carve_lake(&mut grid, &lake_spec, 0).expect("lake carved");
    let rt = make_test_runtime(grid, vec![body]);

    let mut bytes = vec![0u8; DOMAIN_GRID_SIZE as usize * DOMAIN_GRID_SIZE as usize * 8];
    let any_wet = rasterize_domain_into_bytes(&rt, IVec2::new(-44, -44), &mut bytes);
    assert!(any_wet, "domain centered over lake must detect wet water");
    assert!(
        bytes.iter().any(|&b| b != 0),
        "rasterized Rgba16Float domain buffer should contain non-zero depth texels"
    );
}

#[test]
fn two_pass_water_materials_sync_with_atmosphere_weather_and_preserve_passes() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f32(1.0 / 60.0),
    ));
    app.init_asset::<Image>()
        .init_asset::<StandardMaterial>()
        .init_asset::<Shader>()
        .init_asset::<WaterMaterial>();

    app.add_plugins(WaterWavePlugin);
    app.insert_resource(AtmosphereState {
        sun_dir: Vec3::new(0.2, 0.7, -0.5).normalize(),
        moon_dir: Vec3::NEG_Y,
        day: 1.0,
        night: 0.0,
        golden: 0.25,
        zenith: [0.12, 0.34, 0.78],
        horizon: [0.48, 0.66, 0.85],
        fog: [0.45, 0.56, 0.68],
        sun_tint: [1.0, 0.92, 0.78],
        ..Default::default()
    });
    app.insert_resource(WeatherState {
        wind: [0.8, 0.6],
        wind_strength: 1.25,
        clouds: 0.7,
        rain: 0.85,
        cycle: true,
    });
    // A crate lê o ambiente neutro (o `WaterBridgePlugin` da engine é quem o
    // escreve a partir do `WeatherState`/`AtmosphereState`).
    app.insert_resource(viber::terrain::WaterEnvironment {
        wind: [0.8, 0.6],
        wind_strength: 1.25,
        rain: 0.85,
        sun_dir: Vec3::new(0.2, 0.7, -0.5).normalize(),
        day: 1.0,
        zenith: [0.12, 0.34, 0.78],
        horizon: [0.48, 0.66, 0.85],
        sun_tint: [1.0, 0.92, 0.78],
        ..Default::default()
    });

    let absorb_handle =
        app.world_mut()
            .resource_mut::<Assets<WaterMaterial>>()
            .add(WaterMaterial {
                base: StandardMaterial {
                    alpha_mode: bevy::material::AlphaMode::Multiply,
                    depth_bias: 0.0,
                    ..Default::default()
                },
                extension: WaterExtension {
                    params: WaterUniforms::absorb_default(),
                    wave_tex: Handle::default(),
                    domain_tex: Handle::default(),
                    obstacle_tex: Handle::default(),
                },
            });
    let surface_handle =
        app.world_mut()
            .resource_mut::<Assets<WaterMaterial>>()
            .add(WaterMaterial {
                base: StandardMaterial {
                    alpha_mode: bevy::material::AlphaMode::Add,
                    depth_bias: 0.05,
                    ..Default::default()
                },
                extension: WaterExtension {
                    params: WaterUniforms::surface_default(),
                    wave_tex: Handle::default(),
                    domain_tex: Handle::default(),
                    obstacle_tex: Handle::default(),
                },
            });

    app.update();
    app.update();

    let mats = app.world().resource::<Assets<WaterMaterial>>();
    let absorb = mats.get(&absorb_handle).unwrap();
    let surface = mats.get(&surface_handle).unwrap();
    assert!(
        (absorb.extension.params.pass_params.x - 0.0).abs() < 1e-5,
        "Pass 1 Multiply mode must never be overwritten"
    );
    assert!(
        (surface.extension.params.pass_params.x - 1.0).abs() < 1e-5,
        "Pass 2 Add mode must never be overwritten"
    );
    assert!(
        (surface.extension.params.sky_zenith_rain.w - 0.85).abs() < 1e-4,
        "rain must sync to WaterUniforms"
    );
    assert!(
        (surface.extension.params.wind_params.z - 1.25).abs() < 1e-4,
        "wind_strength must sync to WaterUniforms"
    );
    assert!(
        (surface.extension.params.sky_zenith_rain.x - 0.12).abs() < 1e-4,
        "AtmosphereState zenith color must sync cleanly"
    );
}

#[test]
fn water_contact_queues_swept_craters_and_kelvin_wakes_without_legacy_discs() {
    let hit = swept_entry_xz(
        Vec3::new(0.0, 12.0, 0.0),
        Vec3::new(4.0, 8.0, 0.0),
        10.22,
        WADE_ENTER_DEPTH,
    );
    assert!((hit - Vec2::new(2.0, 0.0)).length() < 1e-4);

    let jump = compute_entry_impulse(14.0, 4.0, 0.42, 1.0);
    let step = compute_entry_impulse(0.2, 2.0, 0.42, 0.35);
    assert!(jump.strength > step.strength * 2.5);
    assert!(jump.foam > step.foam);

    let wake = compute_wake_impulse(4.8, 0.42, 0.8);
    assert!(wake.strength > 0.01 && wake.foam > 0.15);

    let mut grid = make_flat_grid(256.0, 128, 12.0);
    let mut lake_spec = LakeSpec::default();
    lake_spec.at = Vec2::ZERO;
    lake_spec.radius = 18.0;
    lake_spec.depth = 4.0;
    let body = carve_lake(&mut grid, &lake_spec, 0).expect("lake carved");
    let surface_y = body.water_y;
    let rt = make_test_runtime(grid, vec![body]);

    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f32(1.0 / 60.0),
    ));
    app.init_asset::<Image>()
        .init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        .init_asset::<Shader>();
    app.insert_resource(rt);
    app.add_plugins((WaterWavePlugin, WaterFxPlugin));

    let swimmer = app
        .world_mut()
        .spawn((
            Transform::from_translation(Vec3::new(0.0, surface_y + 1.5, 0.0)),
            GlobalTransform::from_translation(Vec3::new(0.0, surface_y + 1.5, 0.0)),
            WaterContact::default(),
            WaterKinematics {
                velocity: Vec3::new(0.0, -12.0, 0.0),
                body_radius: 0.42,
                contact_y_offset: 0.0,
            },
        ))
        .id();

    app.update();
    // Simulate plunge into deep lake on frame 2.
    {
        let mut e = app.world_mut().entity_mut(swimmer);
        e.insert((
            Transform::from_translation(Vec3::new(0.0, surface_y - 0.8, 0.0)),
            GlobalTransform::from_translation(Vec3::new(0.0, surface_y - 0.8, 0.0)),
        ));
    }
    app.update();

    let waves = app.world().resource::<WaterWaveWorld>();
    assert_eq!(waves.impulses.len(), 1, "plunge queues crater impulse");
    assert!(!waves.impulses[0].is_wake, "plunge must be radial crater");
    assert!(waves.impulses[0].strength > 0.3);

    // Frame 3: swimming horizontally emits a continuous Kelvin dipole wake.
    {
        let mut e = app.world_mut().entity_mut(swimmer);
        e.insert((
            Transform::from_translation(Vec3::new(0.1, surface_y - 0.8, 0.0)),
            GlobalTransform::from_translation(Vec3::new(0.1, surface_y - 0.8, 0.0)),
            WaterKinematics {
                velocity: Vec3::new(4.2, 0.0, 0.0),
                body_radius: 0.42,
                contact_y_offset: 0.0,
            },
        ));
    }
    app.update();

    let waves = app.world().resource::<WaterWaveWorld>();
    assert!(
        waves.impulses.iter().any(|i| i.is_wake),
        "moving swimmer emits Kelvin bow-stern dipole wake"
    );

    // Ensure no legacy floating Ripple entities were spawned.
    let mut ripple_q = app.world_mut().query::<&viber::terrain::water_fx::Ripple>();
    assert_eq!(ripple_q.iter(app.world()).count(), 0);
}

#[test]
fn water_shaders_preserve_two_pass_and_ssr_contracts() {
    assert!(WATER_WGSL.contains("out.color = main_pass_post_lighting_processing"));
    assert!(WATER_WGSL.contains("prepass_depth"));
    assert!(WATER_WGSL.contains("pass1_mul"));
    assert!(WATER_WGSL.contains("fog_zero"));
    assert!(WATER_WGSL.contains("sample_domain_bank_grad"));
    assert!(WATER_WGSL.contains("bank_shear_foam"));
    assert!(WATER_WGSL.contains("sample_obstacle"));
    assert!(WATER_WGSL.contains("sample_domain_flow"));
    assert!(WATER_WGSL.contains("obs_collide_foam"));
    assert!(WATER_WAVES_WGSL.contains("fn step_sim"));
    assert!(WATER_WAVES_WGSL.contains("fn gradient_export"));
    assert!(WATER_WAVES_WGSL.contains("shore_break_foam"));
    assert!(WATER_WAVES_WGSL.contains("obs_collide_foam"));
    assert!(WATER_SSR_WGSL.contains("let water_m = 0.0"));
    assert!(WATER_SSR_WGSL.contains("ground_m"));
    let _ = std::marker::PhantomData::<AmbientPlugin>;
}

#[test]
fn depth_regimes_and_shore_friction_stratify_impulses() {
    use viber::terrain::water_fx::{WaterDepthZone, classify_depth_zone};
    assert_eq!(classify_depth_zone(0.22), WaterDepthZone::Shallow);
    assert_eq!(classify_depth_zone(0.60), WaterDepthZone::Wading);
    assert_eq!(classify_depth_zone(1.25), WaterDepthZone::Deep);

    let shallow_splash = compute_entry_impulse(5.0, 2.5, 0.42, 0.25);
    let deep_splash = compute_entry_impulse(5.0, 2.5, 0.42, 1.25);
    assert!(deep_splash.strength > shallow_splash.strength * 1.4);
    assert!(deep_splash.radius > shallow_splash.radius * 1.2);
}

#[test]
fn river_current_collides_with_boulders_and_updates_when_broken() {
    use avian3d::prelude::Collider;
    use viber::terrain::water_obstacles::{WaterObstacle, deflect_flow_by_obstacles};
    use viber::terrain::water_waves::rasterize_domain_with_obstacles;

    let mut grid = make_flat_grid(256.0, 128, 12.0);
    let mut river_spec = RiverSpec::default();
    river_spec.path = vec![
        Vec2::new(0.0, -30.0),
        Vec2::new(0.0, 0.0),
        Vec2::new(0.0, 30.0),
    ];
    river_spec.width = 12.0;
    river_spec.depth = 2.4;
    let body = carve_river(&mut grid, &river_spec, 0, &[]).expect("river carves cleanly");
    let surface_y = body
        .surface_y_at(Vec2::ZERO)
        .expect("river surface at origin");
    let base_flow = body
        .sample_domain(Vec2::ZERO, None, 1.5)
        .expect("domain at origin")
        .flow;
    let rt = make_test_runtime(grid, vec![body]);

    let boulder = WaterObstacle::circle(Vec2::ZERO, 1.8);
    let len = DOMAIN_GRID_SIZE as usize * DOMAIN_GRID_SIZE as usize * 8;
    let mut dom_bytes = vec![0u8; len];
    let mut obs_bytes = vec![0u8; len];
    let origin = IVec2::new(-64, -64);

    assert!(rasterize_domain_with_obstacles(
        &rt,
        origin,
        &[boulder.clone()],
        &mut dom_bytes,
        Some(&mut obs_bytes),
    ));
    assert!(
        obs_bytes.iter().any(|&b| b != 0),
        "obstacle_tex should record non-zero SDF halo around rock in water"
    );
    let sheltered = deflect_flow_by_obstacles(Vec2::new(0.0, 3.6), base_flow, &[boulder]);
    assert!(
        sheltered.length() < base_flow.length() * 0.65,
        "downstream pocket behind boulder should shelter current: {sheltered:?} vs {base_flow:?}"
    );

    // Live ECS verification: spawning and despawning a river boulder collider
    // automatically rebuilds obstacle_tex and clears active_obstacles.
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()));
    app.init_asset::<Image>()
        .init_asset::<StandardMaterial>()
        .init_asset::<Shader>()
        .init_asset::<WaterMaterial>();
    app.add_plugins(WaterWavePlugin);
    // Os obstáculos vêm dos colliders avian via o adapter do jogo.
    app.add_plugins(viber::terrain::WaterBridgePlugin);
    app.insert_resource(rt);

    let rock_entity = app
        .world_mut()
        .spawn((
            Transform::from_xyz(0.0, surface_y, 0.0),
            GlobalTransform::from_xyz(0.0, surface_y, 0.0),
            Collider::sphere(1.8),
        ))
        .id();
    app.update();

    let waves = app.world().resource::<WaterWaveWorld>();
    assert_eq!(waves.active_obstacles.len(), 1);
    let obs_img = app
        .world()
        .resource::<Assets<Image>>()
        .get(&waves.obstacle_tex)
        .expect("obstacle_tex image allocated");
    assert!(
        obs_img
            .data
            .as_ref()
            .is_some_and(|d| d.iter().any(|&b| b != 0)),
        "obstacle_tex asset should hold rock halo"
    );

    // Shattering/removing the boulder clears the collision mask on next frame.
    app.world_mut().despawn(rock_entity);
    app.update();

    let waves = app.world().resource::<WaterWaveWorld>();
    assert!(waves.active_obstacles.is_empty());
    let obs_img = app
        .world()
        .resource::<Assets<Image>>()
        .get(&waves.obstacle_tex)
        .unwrap();
    assert!(
        obs_img
            .data
            .as_ref()
            .is_some_and(|d| d.iter().all(|&b| b == 0)),
        "despawned boulder must clear obstacle_tex"
    );
}

/// Casco a remos: mesmas dimensões do `src/boat.rs` (o teste não pode divergir
/// do barco real sem falhar).
fn rowboat_body() -> FloatingBody {
    FloatingBody {
        mass: 260.0,
        hull_half: Vec3::new(0.80, 0.30, 2.20),
        hull_center: Vec3::new(0.0, -0.24, 0.0),
        center_of_mass: Vec3::new(0.0, -0.16, 0.0),
        displacement_factor: 0.55,
        buoyancy_gain: 1.0,
        linear_drag: 1.15,
        quadratic_drag: 2.3,
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

/// A engine integra o casco e publica a velocidade em `WaterKinematics`: o
/// barco flutua na cota do deslocamento DESENHADO (gerstner), com heave
/// dentro do físico e sem afundar/descolar do espelho.
#[test]
fn buoyancy_floats_a_rowboat_on_the_drawn_swell_and_publishes_kinematics() {
    let mut grid = make_flat_grid(256.0, 128, 12.0);
    let mut lake_spec = LakeSpec::default();
    lake_spec.at = Vec2::ZERO;
    lake_spec.radius = 18.0;
    lake_spec.depth = 4.0;
    let body = carve_lake(&mut grid, &lake_spec, 0).expect("lake carved");
    let surface_y = body.water_y;
    let rt = make_test_runtime(grid, vec![body]);
    let hull = rowboat_body();

    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f32(1.0 / 60.0),
    ));
    app.insert_resource(rt);
    app.add_plugins(WaterBuoyancyPlugin);

    let boat = app
        .world_mut()
        .spawn((
            Transform::from_translation(Vec3::new(0.0, surface_y + hull.rest_height(), 0.0)),
            GlobalTransform::default(),
            hull,
            FloatState::default(),
            WaterKinematics {
                velocity: Vec3::ZERO,
                body_radius: 2.6,
                contact_y_offset: hull.hull_center.y - hull.hull_half.y,
            },
        ))
        .id();

    // 6 s a 60 Hz: assenta e depois mede o heave.
    for _ in 0..360 {
        app.update();
    }
    let (mut min_y, mut max_y, mut sum_y, mut samples) = (f32::MAX, f32::MIN, 0.0_f32, 0);
    let mut bobbed = false;
    for _ in 0..120 {
        app.update();
        let (y, kin) = {
            let world = app.world();
            let tf = world.get::<Transform>(boat).expect("boat transform");
            let kin = world.get::<WaterKinematics>(boat).expect("boat kinematics");
            (tf.translation.y, kin.velocity)
        };
        min_y = min_y.min(y);
        max_y = max_y.max(y);
        sum_y += y;
        samples += 1;
        if kin.y.abs() > 1e-3 {
            bobbed = true;
        }
    }
    let mean = sum_y / samples as f32;
    let rest = surface_y + hull.rest_height();
    assert!(
        (mean - rest).abs() < 0.12,
        "flutua na linha de água desenhada: mean {mean} vs rest {rest}"
    );
    assert!(
        max_y - min_y > 0.005,
        "o swell dá heave real: range {}",
        max_y - min_y
    );
    assert!(
        max_y - min_y < 0.8,
        "sem ejeção do casco: range {}",
        max_y - min_y
    );
    assert!(bobbed, "a velocidade vertical chega ao WaterKinematics");
}

/// Um casco com a quilha submersa poucos centímetros tem de entrar como
/// "molhado" e emitir esteira Kelvin — o limiar dos pés (0,22 m) nunca o
/// daria como na água e não havia esteira nenhuma.
#[test]
fn boat_keel_contact_offset_emits_a_kelvin_wake() {
    let mut grid = make_flat_grid(256.0, 128, 12.0);
    let mut lake_spec = LakeSpec::default();
    lake_spec.at = Vec2::ZERO;
    lake_spec.radius = 18.0;
    lake_spec.depth = 4.0;
    let body = carve_lake(&mut grid, &lake_spec, 0).expect("lake carved");
    let surface_y = body.water_y;
    let rt = make_test_runtime(grid, vec![body]);
    let hull = rowboat_body();
    let boat_y = surface_y + hull.rest_height();

    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f32(1.0 / 60.0),
    ));
    app.init_asset::<Image>()
        .init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        .init_asset::<Shader>();
    app.insert_resource(rt);
    app.add_plugins((WaterWavePlugin, WaterFxPlugin));

    let boat = app
        .world_mut()
        .spawn((
            Transform::from_translation(Vec3::new(-2.0, boat_y, 0.0)),
            GlobalTransform::from_translation(Vec3::new(-2.0, boat_y, 0.0)),
            WaterContact::default(),
            WaterKinematics {
                velocity: Vec3::new(2.6, 0.0, 0.0),
                body_radius: 2.6,
                contact_y_offset: hull.hull_center.y - hull.hull_half.y,
            },
        ))
        .id();

    // O casco avança para leste a 2.6 m/s (a posição tem de andar: a velocidade
    // do passo e a do `WaterKinematics` são comparadas).
    let mut x = -2.0_f32;
    let mut saw_wake = false;
    for _ in 0..6 {
        x += 2.6 / 60.0;
        {
            let mut entity = app.world_mut().entity_mut(boat);
            entity.insert((
                Transform::from_translation(Vec3::new(x, boat_y, 0.0)),
                GlobalTransform::from_translation(Vec3::new(x, boat_y, 0.0)),
            ));
        }
        app.update();
        let waves = app.world().resource::<WaterWaveWorld>();
        saw_wake |= waves.impulses.iter().any(|i| i.is_wake);
        if saw_wake {
            break;
        }
    }
    assert!(saw_wake, "o barco a 2.6 m/s tem de abrir esteira Kelvin");
}

/// O raio da esteira acompanha o casco (o clamp antigo de 1,5 m apertava um
/// barco de 4 m ao tamanho de um nadador).
#[test]
fn boat_wake_radius_scales_with_hull_size() {
    let swimmer = compute_wake_impulse(2.6, 0.42, 0.35);
    let boat = compute_wake_impulse(2.6, 2.6, 0.35);
    assert!(
        boat.radius > swimmer.radius * 2.0,
        "esteira do barco {boat:?} vs nadador {swimmer:?}"
    );
    assert!(boat.radius <= 2.6 * 2.6 + 1e-3, "raio limitado pelo casco");
    assert!(boat.foam > swimmer.foam, "casco desloca mais espuma");
}

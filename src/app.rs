//! Composição da App do Vael.
//!
//! Era `viber::app::build_app` com opções; agora é este ficheiro. O jogo
//! compõe explicitamente o `DefaultPlugins` e os plugins do runtime de
//! gameplay (`viber`/`bevy_ext_viber_engine`) na configuração fixa do Vael:
//! sem controller/câmara da engine (`player_camera`), sem regras de jogo da
//! engine (`gameplay`), save externo (`moonshine-save`), scripts Luau em
//! `Compat` e IA em modo serviço. O que era opção morta não existe aqui —
//! as branches de `RunMode::Edit`, `SavePlugin`, projéteis e
//! `ensure_player_vitals` ficaram no `viber run` (crate `viber_engine`).

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use bevy::app::PluginGroupBuilder;
use bevy::prelude::*;
use bevy_kira_audio::prelude::{Mix, ReverbBuilder};
use bevy_kira_audio::AudioApp;

use viber::profiler::{Group, timed};
use viber::recipes::spawn::{self, PendingWorld};

/// Opções do arranque do Vael. Tudo o resto da App é fixo (ver o módulo).
pub struct VaelAppOptions {
    /// `--entry <nome>`: entrada de chegada na cena de arranque.
    pub start_entry: Option<String>,
    /// Porta da bridge de debug; `None` = sem bridge.
    pub bridge_port: Option<u16>,
    /// Mundo COMO O CLI O RECEBEU (antes do `start-scene`): identidade do
    /// registo de sessão/`viber.ping`. A bridge responde por este caminho.
    pub identity: PathBuf,
    /// Espelho por frame de `viber.ecs` (opt-in: `VIBER_ECS_MIRROR=1`).
    pub ecs_mirror: bool,
    /// Wrapper do grupo `DefaultPlugins` (o Vael usa o PIE do Jackdaw).
    pub default_plugins: Option<fn(PluginGroupBuilder) -> PluginGroupBuilder>,
}

/// Tamanho do shadow map da DIRECIONAL (`VIBER_DIR_SHADOW_SIZE`, default
/// 4096). É UMA textura de 4 camadas (~268 MB a 4096, metade a 2048).
fn dir_shadow_size() -> usize {
    std::env::var("VIBER_DIR_SHADOW_SIZE")
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok())
        .filter(|size| *size >= 256 && size % 2 == 0)
        .unwrap_or_else(viber::graphics::dir_shadow_size)
}

/// Tamanho do cube shadow map das PointLight (`VIBER_POINT_SHADOW_SIZE`).
/// Default 1024 (~25 MB/lanterna); `1536` devolve o valor do passe r1.
fn point_shadow_size() -> usize {
    std::env::var("VIBER_POINT_SHADOW_SIZE")
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok())
        .filter(|size| *size >= 4 && size % 2 == 0)
        .unwrap_or(1024)
}

/// Profiling do lado do RENDER (`VIBER_PROF_GPU=1`): queries de timestamp no
/// device + `RenderDiagnosticsPlugin`. Opt-in — adapter sem timestamps falha
/// a criação do renderer.
fn gpu_profiling_enabled() -> bool {
    matches!(
        std::env::var("VIBER_PROF_GPU").as_deref(),
        Ok("1" | "true" | "on")
    )
}

/// Monta a App do Vael a partir do mundo já resolvido (`resolve_run_path`).
///
/// O parceiro de leitura disto é `viber::app::build_app` (o `viber run`):
/// as fases estão na mesma ordem; o que difere está comentado no sítio.
pub fn build_app(path: &Path, opts: VaelAppOptions) -> Result<App> {
    let VaelAppOptions {
        start_entry,
        bridge_port,
        identity,
        ecs_mirror,
        default_plugins,
    } = opts;
    // Absoluto desde já: o asset root e o base_dir do terreno não podem
    // depender do CWD (o bevy resolve roots relativas contra o exe).
    let path = &std::path::absolute(path)?;
    let world = viber::app::load_world(path)?;
    // O runtime só corre mundos ESTÁTICOS (`<world static="true">`).
    if !world.static_world {
        bail!(
            "{}: mundo procedural (sem `<world static=\"true\">`) — converte-o com `viber bake {}`.",
            path.display(),
            path.display()
        );
    }
    // config.yaml do jogo é OBRIGATÓRIO e lido antes de tudo: asset roots e
    // diretórios vêm daí. Os perfis (gráficos, pós-FX, reflexos) decidem-se
    // ANTES da App: o spawn do mundo e o attach da câmara leem-nos no arranque.
    let world_dir_pre = viber::app::world_base_dir(path).unwrap_or_else(|| PathBuf::from("."));
    let config = viber::config::load(&world_dir_pre)?;
    let graphics = viber::graphics::resolve(config.graphics.as_deref());
    viber::postfx::resolve_profile(config.postfx.as_ref());
    viber::water_ssr::resolve_profile(config.reflections.as_ref());
    info!("graphics preset: {}", graphics.name());
    let title = config
        .title
        .clone()
        .map(|title| title.trim().to_owned())
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| "Vael".to_string());
    let (world_dir, asset_roots) = viber::app::world_asset_dirs(path, &config);
    let asset_root = asset_roots[0].clone();
    // Shaders ESPECIALIZADOS por mundo (céu/água/camadas): consts WGSL do
    // projeto — vêm SEMPRE da cena principal (contrato de conteúdo).
    let project = viber::project::Project::discover(path);
    let main_world = if project.declared
        && project.scene_id(path).as_deref() != Some(viber::project::MAIN_SCENE)
    {
        match viber::app::load_world(&project.main_scene_path()) {
            Ok(main) => Some(main),
            Err(error) => {
                warn!(
                    "cena principal do projeto não carrega ({error:#}) — shaders da cena de arranque"
                );
                None
            }
        }
    } else {
        None
    };
    let shader_entities = &main_world.as_ref().unwrap_or(&world).entities;
    let sky_config = viber::sky::SkyConfig::from_world(shader_entities).with_env_override();
    // As configs de água/camadas vêm do `terrain_host` (a extração do
    // terreno para o `bevy_ext_terrain_runtime` mudou a assinatura).
    let water_config = viber::terrain_host::water_surface_config_from_world(shader_entities);
    let layers_config = viber::terrain_host::terrain_chunk_config_from_world(shader_entities);
    let shaders_dir = asset_root.join("shaders");
    let _ = std::fs::create_dir_all(&shaders_dir);
    for (name, contents) in [
        ("sky.wesl", sky_config.render_world_shader()),
        ("water.wesl", water_config.render_world_shader()),
    ] {
        if let Err(e) = std::fs::write(shaders_dir.join(name), contents) {
            eprintln!("vael: falha ao escrever {}/{name}: {e}", shaders_dir.display());
        }
    }
    if let Some(layers_config) = &layers_config
        && let Err(e) = std::fs::write(
            shaders_dir.join("terrain_chunk.wesl"),
            layers_config.render_world_shader(),
        )
    {
        eprintln!("vael: falha ao escrever {}/terrain_chunk.wesl: {e}", shaders_dir.display());
    }
    // SSR da água: shader ESTÁTICO, só com o gate ligado.
    if viber::water_ssr::water_ssr_requested()
        && let Err(e) = std::fs::write(
            shaders_dir.join("water_ssr.wesl"),
            viber::water_ssr::WATER_SSR_WGSL,
        )
    {
        eprintln!("vael: falha ao escrever {}/water_ssr.wesl: {e}", shaders_dir.display());
    }
    // O loader de glTF é recursivo por nó: pool de IO com stack 32 MiB ANTES
    // do `TaskPoolPlugin` (que respeita o global já criado).
    let io_threads = std::thread::available_parallelism()
        .map(|n| (n.get() / 4).clamp(1, 4))
        .unwrap_or(1);
    bevy::tasks::IoTaskPool::get_or_init(|| {
        bevy::tasks::TaskPoolBuilder::new()
            .num_threads(io_threads)
            .thread_name("IO Task Pool".to_string())
            .stack_size(32 * 1024 * 1024)
            .build()
    });

    let mut app = App::new();
    // Cinemáticas do mundo (`<Cinematic>`): biblioteca antes do plugin.
    app.insert_resource(viber::cinematic::CinematicLibrary(
        world
            .cinematics
            .iter()
            .map(|spec| (spec.id.clone(), spec.clone()))
            .collect(),
    ));
    // O modo Game viaja como resource: os run conditions (`scripts_enabled`)
    // leem-no. (O modo Edit e a pausa de tempo ficaram no `viber run`.)
    app.insert_resource(bevy_ext_core::RunMode::Game);
    // Fonte de assets multi-root + meshopt, ANTES do `AssetPlugin` (que
    // snapshot-a as fontes quando constrói).
    viber::meshopt::register_asset_source(&mut app, asset_roots.clone());
    // O config viaja COMO RESOURCE: ambient (sfx_dir), save (dir), terreno
    // (textures_dir) e o spawn (bgm_dir) leem-no de lá.
    app.insert_resource(config.clone());
    app.insert_resource(viber::save::SaveDir(Some(config.save_dir().to_path_buf())));
    // O modelo do céu: o IBL e os probes regionais usam a MESMA radiância.
    app.insert_resource(viber::sky::SkyModelState::from_config(&sky_config));

    let mut plugins = bevy::DefaultPlugins
        .set(bevy::window::WindowPlugin {
            primary_window: Some(bevy::window::Window {
                title,
                present_mode: viber::graphics::present_mode(),
                ..Default::default()
            }),
            ..Default::default()
        })
        .set(bevy::asset::AssetPlugin {
            file_path: asset_root.to_string_lossy().into_owned(),
            ..Default::default()
        });
    if bridge_port.is_some() {
        // A layer de logs do bridge tem de ser instalada no LogPlugin no boot.
        plugins = plugins.set(viber::bridge::logs::log_plugin_with_bridge());
    }
    if gpu_profiling_enabled() {
        use bevy::render::settings::{WgpuFeatures, WgpuSettings};
        let base = WgpuSettings::default();
        plugins = plugins.set(bevy::render::RenderPlugin {
            render_creation: WgpuSettings {
                features: base.features
                    | WgpuFeatures::TIMESTAMP_QUERY
                    | WgpuFeatures::TIMESTAMP_QUERY_INSIDE_ENCODERS
                    | WgpuFeatures::TIMESTAMP_QUERY_INSIDE_PASSES,
                ..base
            }
            .into(),
            ..Default::default()
        });
    }
    if let Some(configure) = default_plugins {
        plugins = configure(plugins);
    }
    app.add_plugins(plugins);

    // Reflexão dos tipos de jogo no `AppTypeRegistry` (BRP/`viber.query`).
    app.add_plugins(viber::reflect::ReflectPlugin);
    // Picking: só o backend de UI alimenta a `Interaction`; o mesh picking
    // fica com `require_markers` (não raycasta as malhas todas por frame).
    app.insert_resource(bevy::picking::mesh_picking::MeshPickingSettings {
        require_markers: true,
        ..Default::default()
    });
    if gpu_profiling_enabled() {
        app.add_plugins(bevy::render::diagnostic::RenderDiagnosticsPlugin);
    }
    // SSR de reflexões raster (opt-in `VIBER_WATER_SSR=1`) e probes regionais
    // (bounce de bioma, default ON `VIBER_PROBES=0` desliga).
    app.add_plugins(viber::water_ssr::WaterSsrPlugin);
    app.add_plugins(viber::probes::RegionalProbesPlugin);
    info!(
        "sky: modelo {} (attr <Sky model> / VIBER_SKY_MODEL)",
        match sky_config.model {
            viber::sky::SkyModel::Nishita => "nishita",
            viber::sky::SkyModel::Analytic => "analytic",
        }
    );
    // Áudio: backend kira com buses tipados (música/SFX/voz/ambiente) e
    // reverb por track; o bevy_audio/rodio fica sem players.
    app.add_plugins(bevy_kira_audio::AudioPlugin);
    app.add_audio_channel::<viber::music::MusicBus>();
    let mut sfx_track = bevy_kira_audio::AudioTrack::new();
    let sfx_reverb = sfx_track.add_effect(ReverbBuilder::new().mix(Mix(0.0)));
    app.add_audio_channel_with_track::<viber::music::SfxBus>(sfx_track);
    let mut voice_track = bevy_kira_audio::AudioTrack::new();
    let voice_reverb = voice_track.add_effect(ReverbBuilder::new().mix(Mix(0.0)));
    app.add_audio_channel_with_track::<viber::music::VoiceBus>(voice_track);
    app.add_audio_channel::<viber::music::AmbienceBus>();
    app.init_resource::<viber::music::AudioDuck>();
    app.init_resource::<viber::music::AudioReverb>();
    app.insert_resource(viber::music::ReverbHandles {
        sfx: Some(sfx_reverb),
        voice: Some(voice_reverb),
    });
    // Bridge de debug + registo de sessão (o agente que lançou o jogo lê a
    // linha do eprintln para apontar o `viber debug` a ESTA engine).
    if let Some(port) = bridge_port {
        app.add_plugins(viber::bridge::BridgePlugin { port });
        app.insert_resource(viber::bridge::BridgeIdentity {
            world: identity.display().to_string(),
        });
        let session = viber::session::SessionPaths::for_world(&identity);
        let register = match session.engine_info() {
            Some(existing) if existing.pid != std::process::id() => {
                !viber::bridge::client::port_alive(existing.port)
            }
            _ => true,
        };
        if register {
            let _ = session.write_engine(&viber::session::EngineInfo {
                pid: std::process::id(),
                port,
                world: identity.display().to_string(),
                started_at_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0),
                log: String::new(),
            });
        }
        eprintln!(
            "vael: debug bridge at http://127.0.0.1:{port} — viber debug --world {} …",
            identity.display()
        );
    }
    // Documento editável do mundo: num mundo `.bsn` não é carregado (autoria
    // no Jackdaw; `viber.scene.*` devolve "sem documento").
    app.add_plugins((viber::scene::live::ScenePlugin, viber::scene::sync::SceneSyncPlugin));
    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("bsn"))
    {
        info!("scene doc: {} é .bsn — edição de cena via Jackdaw", path.display());
    } else {
        app.insert_resource(viber::scene::doc::SceneDoc::load(path)?);
    }
    // O save é do JOGO (projeto), não da cena: a chave é a da cena principal.
    let save_key_path = if project.declared {
        project.main_scene_path()
    } else {
        path.to_path_buf()
    };
    app.insert_resource(viber::save::WorldSaveKey(viber::save::world_save_key(&save_key_path)));
    let scene_title = world.title.clone();
    // Autoloads do projeto: parseados aqui, spawnados antes da 1.ª cena.
    if project.declared {
        let mut autoload = Vec::new();
        for file in project.autoload_paths() {
            // `Context::` explícito: o `with_context` do bevy_ecs (error
            // handling 0.20) é o outro candidato no scope.
            let loaded = anyhow::Context::with_context(
                viber::xml::include::load_world_in(&file, &project.root),
                || format!("autoload {}", file.display()),
            )?;
            let parsed = anyhow::Context::with_context(
                viber::recipes::parse_world(&loaded.root_attrs, &loaded.nodes),
                || format!("autoload {}", file.display()),
            )?;
            autoload.push(parsed);
        }
        if !autoload.is_empty() {
            app.insert_resource(viber::scene_loader::PendingAutoload(autoload));
        }
    }
    app.insert_resource(viber::scene_loader::CurrentScene::for_path(path, scene_title));
    if let Some(entry) = start_entry {
        app.insert_resource(viber::scene_loader::StartEntry(Some(entry)));
    }
    app.insert_resource(PendingWorld {
        world,
        base_dir: viber::app::world_base_dir(path),
    });
    // `sun_drive` aponta a direcional a partir do SunState; luz de interior.
    app.init_resource::<viber::worldsys::SunState>();
    app.init_resource::<viber::worldsys::InteriorLighting>();
    // Sombras: direcional 4096 (r1, árvores ao pé do herói), ponto 1024.
    app.insert_resource(bevy::light::DirectionalLightShadowMap {
        size: dir_shadow_size(),
    });
    app.insert_resource(bevy::light::PointLightShadowMap {
        size: point_shadow_size(),
    });
    // Widgets nativos (minimapa/bússola/seta) obedecem ao esconder-HUD.
    app.init_resource::<viber::hud::HudHidden>();
    // Materiais do céu/água/camadas: os bootstraps precisam dos Assets.
    app.add_plugins(bevy::pbr::MaterialPlugin::<viber::sky::SkyMaterial>::default());
    app.add_plugins(bevy::pbr::MaterialPlugin::<
        viber::terrain::water_material::WaterMaterial,
    >::default());
    app.add_plugins(bevy::pbr::MaterialPlugin::<
        viber::terrain::layer_material::TerrainChunkMaterial,
    >::default());
    // Primitivas procedurais das `<Composition>`: sem o plugin o spawn cai
    // no caminho StandardMaterial antigo.
    app.add_plugins(bevy_gradmesh::GradMeshPlugin);
    app.add_plugins(viber::animation::AnimationPlugin);
    app.add_plugins(viber::decal_fx::DecalFxPlugin);
    app.add_plugins(viber::ik::IkPlugin);
    app.add_plugins(viber::ragdoll::RagdollPlugin);
    app.add_plugins(viber::input_map::InputMapPlugin);
    app.add_plugins(bevy_ext_core::a11y::A11yPlugin);
    app.add_plugins(viber::audio_space::AudioSpacePlugin);
    app.add_plugins(bevy_ext_core::locale::LocalePlugin {
        dir: world_dir_pre.join("locale"),
    });
    app.add_plugins(viber::streaming::StreamingPlugin);
    app.add_plugins(viber::foliage::FoliagePlugin);
    app.add_plugins(viber::draw_diag::DrawDiagPlugin);
    app.add_plugins(viber::frame_pacing::FramePacingPlugin);
    app.add_plugins(viber::netplay::NetPlayPlugin);
    app.add_plugins(bevy_ext_physics::physics::PhysicsPlugin {
        debug: std::env::var_os("VIBER_PHYSICS_DEBUG").is_some(),
    });
    app.add_plugins(viber::postfx::PostFxPlugin);
    app.add_plugins(viber::postfx_toggle::PostFxDebugTogglePlugin);
    app.add_plugins(viber::adaptive_quality::AdaptiveQualityPlugin);
    app.add_plugins(viber::spawn_gate::SpawnGatePlugin);
    // (Projéteis: gameplay da engine — o Vael corre com `gameplay:false`.)
    app.add_plugins(viber::ibl::SkyIblPlugin);
    app.add_plugins(viber::terrain::TerrainPlugin);
    app.add_plugins(viber::terrain::runtime::TerrainFeaturesPlugin);
    app.add_plugins(viber::grass::GrassPlugin);
    app.add_plugins(viber::textures::TexturesPlugin);
    // Host Luau (`scripts/` do config, `on_update(dt)`); o árbitro das
    // interações é registado pelo próprio plugin.
    app.add_plugins(viber::luau::LuauScriptPlugin {
        scripts_dir: config.scripts_dir_on(&world_dir),
        ecs_mirror,
    });
    // Serviços que o movimento, o som e o HUD leem.
    app.init_resource::<viber::menus::MenusOpen>();
    app.init_resource::<viber::music::MusicLayerOverride>();
    app.init_resource::<viber::music::CombatMusicState>();
    app.init_resource::<viber::hud::markers::HudMarkers>();
    app.init_resource::<viber::music::ScriptSoundLoops>();
    app.add_plugins(viber::menus::MenusPlugin);
    // Save externo: recursos da ponte Lua + importador de recortes de
    // terreno; os slots/envelope são do Vael (`moonshine-save`). O
    // importador é genérico sobre o loader de cena (contrato
    // `SceneSwitchBusy`) — concretiza com o `SceneLoader` do runtime.
    app.init_resource::<bevy_ext_save::SaveRequest>()
        .init_resource::<bevy_ext_save::PendingSavePatches>()
        .add_systems(
            bevy::app::Last,
            bevy_ext_save::apply_pending_save_patches::<viber::scene_loader::SceneLoader>
                .after(viber::scene_loader::drive_scene_loader),
        );
    // Props em ossos (`viber.attach`).
    app.add_plugins(viber::attachment::AttachmentPlugin);
    // FX puros sem preset RPG: kick de FOV e hit-stop.
    app.init_resource::<bevy_ext_engine::camera_glue::CameraFx>();
    app.add_systems(bevy::app::Update, bevy_ext_engine::camera_glue::fov_kick_system);
    app.init_resource::<viber::time_fx::HitStop>();
    app.init_resource::<viber::time_fx::BaseTimeScale>();
    app.add_systems(bevy::app::Update, viber::time_fx::hit_stop_system);
    // Mundo vivo (fog/tint por bioma, orçamento de luzes, gestos, SFX) e o
    // day_tint dos GltfScene.
    app.add_plugins(viber::ambient::AmbientPlugin);
    app.add_plugins(viber::prop_tint::PropTintPlugin);
    app.add_plugins(viber::physics_fx::PhysicsFxPlugin);
    app.add_plugins(viber::particles::BurstPlugin);
    app.add_plugins(viber::impact::ImpactFxPlugin);
    app.add_plugins(viber::terrain::water_fx::WaterFxPlugin);
    // Trauma do shake + solavanco direcional. (O `ScriptCameraFree`/
    // `CameraSnapshot` que os sistemas do Vael leem são os do CORE — o
    // `VaelCameraPlugin` inicializa-os; os gémeos `bevy_ext_engine::camera_glue` só têm
    // leitor nos sistemas `player_camera` da engine, fora do jogo.)
    app.init_resource::<bevy_ext_engine::camera_glue::CameraShake>();
    app.init_resource::<bevy_ext_engine::camera_glue::CameraKick>();
    app.add_plugins(viber::cinematic::CinematicPlugin);
    // IA em modo serviço: sentidos + vitais das criaturas + o consumidor
    // ÚNICO do `AiLocomotion` (sem ele as criaturas atacam paradas).
    app.add_plugins(viber::ai::AiServicePlugin);
    // Navmesh Recast + A*/RVO (bevy_landmass), preferência por estradas.
    app.add_plugins(viber::nav::NavPlugin);
    // LOD de render: `VIBER_RENDER_LOD=0` desliga (A/B honesto).
    if std::env::var("VIBER_RENDER_LOD").as_deref() != Ok("0") {
        app.add_plugins(viber::render_lod::RenderLodPlugin);
    }
    app.add_plugins(viber::profiler::ProfilerPlugin);
    // UI declarativa da engine (fala/prompts/NLU; o HUD do jogo é o
    // `VaelUiPlugin`, em bevy_pf).
    app.add_plugins(viber::ui::UiPlugin);
    // Feedback de impacto (números/mensagens); o pool 2D é decisão do jogo:
    // o Vael desenha o texto no mundo (bevy_rich_text3d).
    app.add_plugins(bevy_ext_engine::feedback::FeedbackPlugin);
    app.insert_resource(bevy_ext_engine::feedback::DamageNumbers2d(false));
    // `viber.ui` tem de ganhar ao runtime Luau (script do frame 1 via
    // `luau_on_add`): instalação antes, mutações enfileiradas depois, e a
    // UI PUBLICA antes dos scripts lerem (cliques no mesmo frame).
    app.add_systems(
        bevy::app::Update,
        viber::ui::install_ui_script_api
            .before(viber::luau::luau_on_add)
            .before(viber::luau::luau_update),
    );
    app.configure_sets(
        bevy::app::Update,
        viber::ui::UiSet::Script.after(viber::luau::luau_update),
    );
    app.configure_sets(
        bevy::app::Update,
        (
            viber::ui::UiSet::Collect,
            viber::ui::UiSet::Build,
            viber::ui::UiSet::Bind,
        )
            .before(viber::luau::luau_update),
    );
    // Spawn do mundo: arranque e trocas de cena (o mesmo spawner).
    app.add_systems(bevy::app::Startup, spawn::startup);
    app.add_systems(viber::scene_loader::SceneLoad, spawn::startup);
    app.add_plugins(viber::scene_loader::SceneLoaderPlugin);
    // Câmara de editor desligada no jogo (o Vael não usa `--camera free`).
    app.add_plugins(bevy_ext_engine::editor_camera::EditorCameraPlugin { enabled: false });
    // Spawn RUNTIME de prototypes: exclusivo no PostUpdate, antes da
    // propagação de transforms (senão nasce 1 frame na origem).
    app.add_systems(
        bevy::app::PostUpdate,
        spawn::apply_script_spawns
            .before(bevy::transform::TransformSystems::Propagate)
            .run_if(spawn::has_pending_script_spawns),
    );
    app.add_systems(
        bevy::app::Update,
        (
            timed(Group::Hud, viber::hud::hud_balloon_update),
            viber::hud::apply_subtitle_scale,
        ),
    );
    // (Sem `player_camera` nem `gameplay`: o controller/câmara são do Vael
    // — `src/player.rs`/`src/camera.rs` — e os vitais do herói são da
    // autoridade Rust, `src/rules_authority.rs`.)
    app.add_systems(
        bevy::app::Update,
        (
            timed(Group::Spawner, spawn::gltf_scene_spawner),
            viber::hud::compass::hud_compass_update,
            viber::hud::compass::hud_compass_landmarks_update,
            viber::hud::hide_world_hud_in_interior,
            viber::hud::waypoint::update_waypoint_arrow,
            timed(Group::Hud, viber::hud::hud_minimap_update),
            timed(Group::Hud, viber::hud::hud_minimap_landmarks_update),
            timed(Group::World, viber::music::audio_loop_starter),
            timed(Group::World, viber::music::mixer_sync),
            timed(Group::World, viber::music::apply_reverb),
            timed(Group::World, viber::music::music_driver),
            timed(Group::World, viber::music::script_sound_loops),
            // Interior decide ANTES dos drivers do mundo (mesmo frame).
            timed(Group::World, viber::worldsys::interior_lighting_drive)
                .before(viber::worldsys::daycycle_drive),
            timed(Group::World, viber::worldsys::daycycle_drive),
            timed(Group::World, viber::worldsys::interior_lighting_apply)
                .after(viber::worldsys::sun_drive)
                .after(viber::worldsys::daycycle_drive),
            timed(Group::World, viber::worldsys::sun_drive),
        ),
    );
    // Tuplo dividido: o Bevy limita tuples de sistemas a 20 elementos.
    app.add_systems(
        bevy::app::Update,
        (
            viber::sky::sky_follow_camera,
            viber::hud::hud_toggle,
            // Lifecycle de FX ambiente por proximidade, encadeado antes do
            // update (recém-montados simulam já neste frame).
            (
                timed(Group::Fx, viber::particles::ambient_fx_manager),
                timed(Group::Fx, viber::particles::particle_emitter_update),
            )
                .chain(),
            timed(Group::Spawner, viber::spawner::instantiate_spawn_groups),
        ),
    );
    Ok(app)
}

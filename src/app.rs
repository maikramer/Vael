//! Composição da App do Vael.
//!
//! Era `viber::app::build_app` com opções; agora é este ficheiro. O jogo
//! compõe explicitamente o `DefaultPlugins` e os plugins do runtime de
//! gameplay (`viber`/`bevy_ext_viber_engine`) na configuração fixa do Vael:
//! sem controller/câmara da engine (`player_camera`), sem regras de jogo da
//! engine (`gameplay`), save externo (`moonshine-save`), scripts Luau em
//! `Compat` e IA em modo serviço. O que era opção morta não existe aqui —
//! as branches de `RunMode::Edit`, `SavePlugin`, projéteis e
//! `ensure_player_vitals` ficaram no monólito (crate `bevy_ext_viber_engine`).

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use bevy::app::PluginGroupBuilder;
use bevy::prelude::*;
use bevy_kira_audio::AudioApp;
use bevy_kira_audio::prelude::{Mix, ReverbBuilder};

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
/// O parceiro de leitura disto é `viber::app::build_app` (o monólito):
/// as fases estão na mesma ordem; o que difere está comentado no sítio.
pub fn build_app(path: &Path, opts: VaelAppOptions) -> Result<App> {
    let VaelAppOptions {
        start_entry,
        bridge_port,
        identity,
        default_plugins,
    } = opts;
    // Absoluto desde já: o asset root e o base_dir do terreno não podem
    // depender do CWD (o bevy resolve roots relativas contra o exe).
    let path = &std::path::absolute(path)?;
    let world = viber::app::load_world(path)?;
    // O runtime só corre mundos ESTÁTICOS (`<world static="true">`).
    if !world.static_world {
        bail!(
            "{}: mundo procedural (sem `<world static=\"true\">`) — converte-o no Jackdaw ou usa um mundo estático.",
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
    // Pasta `scripts/` do projeto para o `viber.load`/`load_json` (loader de
    // módulos): em modo BMS o publisher só escreve o static do store quando
    // o estado publicado traz `Some`, portanto semeia-se AQUI uma vez (o
    // host antigo recebia o dir no plugin).
    bevy_ext_luau::game::set_scripts_dir(Some(config.scripts_dir_on(&world_dir)));
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
    // As configs de água/camadas vêm do glue IR→shader do monólito
    // (`viber::terrain_host` — o walk do IR de parsing não vive no
    // `bevy_ext_terrain_runtime`).
    let water_config = viber::terrain_host::water_surface_config_from_world(shader_entities);
    let layers_config = viber::terrain_host::terrain_chunk_config_from_world(shader_entities);
    let shaders_dir = asset_root.join("shaders");
    let _ = std::fs::create_dir_all(&shaders_dir);
    for (name, contents) in [
        ("sky.wesl", sky_config.render_world_shader()),
        ("water.wesl", water_config.render_world_shader()),
    ] {
        if let Err(e) = std::fs::write(shaders_dir.join(name), contents) {
            eprintln!(
                "vael: falha ao escrever {}/{name}: {e}",
                shaders_dir.display()
            );
        }
    }
    if let Some(layers_config) = &layers_config
        && let Err(e) = std::fs::write(
            shaders_dir.join("terrain_chunk.wesl"),
            layers_config.render_world_shader(),
        )
    {
        eprintln!(
            "vael: falha ao escrever {}/terrain_chunk.wesl: {e}",
            shaders_dir.display()
        );
    }
    // SSR da água: shader ESTÁTICO, só com o gate ligado.
    if viber::water_ssr::water_ssr_requested()
        && let Err(e) = std::fs::write(
            shaders_dir.join("water_ssr.wesl"),
            viber::water_ssr::WATER_SSR_WGSL,
        )
    {
        eprintln!(
            "vael: falha ao escrever {}/water_ssr.wesl: {e}",
            shaders_dir.display()
        );
    }
    // O loader de glTF é recursivo por nó: pool de IO com stack 32 MiB ANTES
    // do `TaskPoolPlugin` (que respeita o global já criado).
    //
    // O número de threads é o da engine (`cores - 2`, `VIBER_IO_THREADS`):
    // aqui chegou a ser `cores/4` (teto 4), o que deixava ~1,5 GB de GLB do
    // Vael numa fila de 3–4 threads com o resto da máquina parado — o arranque
    // pagava isso em dezenas de segundos.
    let io_threads = viber::app::io_thread_count();
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
    // leem-no. (O modo Edit e a pausa de tempo ficaram no monólito.)
    app.insert_resource(bevy_ext_core::RunMode::Game);
    // Fonte de assets multi-root + meshopt, ANTES do `AssetPlugin` (que
    // snapshot-a as fontes quando constrói).
    viber::meshopt::register_asset_source(&mut app, asset_roots.clone());
    // O config viaja COMO RESOURCE: ambient (sfx_dir), save (dir), terreno
    // (textures_dir) e o spawn (bgm_dir) leem-no de lá.
    app.insert_resource(config.clone());
    // O runtime de terreno lê `TerrainWorldAssets` (não o `GameConfig`): sem
    // este recorte as layers resolvem contra a pasta do mundo
    // (`grass/albedo.ktx2`) e cada albedo vira um `Path not found` no Bevy.
    viber::terrain_host::install_world_assets(&mut app, &config, &world_dir);
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
    // Warnings de TERCEIROS que não têm acção nossa no arranque: o `arboard`
    // cai para o X11 quando o compositor não expõe `ext-data-control`
    // (Wayland) e o `sctk_adwaita` não conhece um botão da decoração. Sem os
    // silenciar, cada linha aparece no log e o QA (que lê o log) aprende a
    // ignorar WARN. `RUST_LOG` sobrepõe-se a este filtro (contrato do
    // bevy_log), portanto um debug destes alvos continua possível.
    const QUIET_TARGETS: &str = "arboard=error,sctk_adwaita=error";
    let log_filter = format!("{},{QUIET_TARGETS}", bevy::log::DEFAULT_FILTER);
    let mut log_plugin = if bridge_port.is_some() {
        // A layer de logs do bridge tem de ser instalada no LogPlugin no boot.
        viber::bridge::logs::log_plugin_with_bridge()
    } else {
        bevy::log::LogPlugin::default()
    };
    log_plugin.filter = log_filter;
    plugins = plugins.set(log_plugin);
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
    // linha do eprintln para apontar o `vael-tools debug` a ESTA engine).
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
            "vael: debug bridge at http://127.0.0.1:{port} — vael-tools debug --world {} …",
            identity.display()
        );
    }
    // Documento editável do mundo: num mundo `.bsn` não é carregado (autoria
    // no Jackdaw; `viber.scene.*` devolve "sem documento").
    // A edição viva é do Jackdaw (`src/editor_link.rs`/`src/live_edit.rs`):
    // os plugins de `viber.scene --live` reformaram-se.
    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("bsn"))
    {
        info!(
            "scene doc: {} é .bsn — edição de cena via Jackdaw",
            path.display()
        );
    } else {
        app.insert_resource(viber::scene::doc::SceneDoc::load(path)?);
    }
    // O save é do JOGO (projeto), não da cena: a chave é a da cena principal.
    let save_key_path = if project.declared {
        project.main_scene_path()
    } else {
        path.to_path_buf()
    };
    app.insert_resource(viber::save::WorldSaveKey(viber::save::world_save_key(
        &save_key_path,
    )));
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
    app.insert_resource(viber::scene_loader::CurrentScene::for_path(
        path,
        scene_title,
    ));
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
    // Residência posicional dos GLB (`<GltfScene>`): o véu espera só o anel
    // próximo e o resto do mundo entra atrás do herói. `VIBER_GLTF_STREAMING=0`
    // volta a carregar tudo no arranque.
    app.add_plugins(viber::gltf_residency::GltfResidencyPlugin);
    app.add_plugins(viber::foliage::FoliagePlugin);
    app.add_plugins(viber::draw_diag::DrawDiagPlugin);
    app.add_plugins(viber::frame_pacing::FramePacingPlugin);
    app.add_plugins(viber::netplay::NetPlayPlugin);
    // Física (Avian) do `bevy_ext_physics` — o mesmo plugin que o monólito
    // expõe em `viber::physics::PhysicsPlugin`. O streaming dos colliders das
    // colunas voxel é host-side (lê o `TerrainRuntime` do monólito) e entra
    // pelo `TerrainCollidersPlugin` ao lado, como na app da engine.
    app.add_plugins(viber::physics::PhysicsPlugin {
        debug: std::env::var_os("VIBER_PHYSICS_DEBUG").is_some(),
    });
    app.add_plugins(viber::physics::TerrainCollidersPlugin);
    // Controller do herói (tnua) e step assist. O `PhysicsPlugin` acima só
    // faz colliders; o herói é do JOGO: o scheme e o tuning do tnua vivem
    // aqui (`VaelHeroScheme`/`vael_hero_config` em `src/player.rs`) — o
    // plugin da engine é genérico sobre o scheme.
    app.add_plugins(
        viber::player::HeroPhysicsPlugin::<crate::player::VaelHeroScheme>::new(
            crate::player::vael_hero_config,
        ),
    );
    app.add_plugins(viber::postfx::PostFxPlugin);
    app.add_plugins(viber::postfx_toggle::PostFxDebugTogglePlugin);
    app.add_plugins(viber::adaptive_quality::AdaptiveQualityPlugin);
    app.add_plugins(viber::spawn_gate::SpawnGatePlugin);
    // (Projéteis: gameplay da engine — o Vael corre com `gameplay:false`.)
    app.add_plugins(viber::ibl::SkyIblPlugin);
    app.add_plugins(viber::terrain::TerrainPlugin);
    app.add_plugins(viber::terrain::runtime::TerrainFeaturesPlugin);
    // A relva (`crate::grass::VaelGrassPlugin`) NÃO entra aqui: é plugin do
    // jogo e o `VaelPlugin` (lib.rs) já a regista — duplicar dá panic
    // ("plugin was already added").
    app.add_plugins(viber::textures::TexturesPlugin);
    // Host Luau (BMS 0.24): a superfície `viber.*` vive no bevy_ext_luau
    // (ondas F2-F8) e o `BmsBridgePlugin` do monólito publica o estado do
    // mundo (ScriptHostState) e aplica os comandos dos scripts com a
    // maquinaria de sempre. Os contextos são por entidade: os instaladores
    // `viber.vael` do jogo (cofre/XP/quests/…) correm por contexto novo
    // através do registo (`crate::script_host`).
    app.add_plugins(bevy_ext_luau::VaelScriptingPlugin {
        extra_initializers: crate::script_host::extra_initializers(),
    });
    app.add_plugins(viber::luau::bms_bridge::BmsBridgePlugin);
    // Adaptadores de host da G-1: publicam o mundo para os crates de
    // áudio/IA (sem eles music_driver/script_sound_loops ficam mudos e a
    // locomoção perde o snap de Y).
    app.add_plugins(viber::audio_host::AudioHostPlugin);
    app.add_plugins(viber::ai_host::AiHostPlugin);
    // Serviços que o movimento, o som e o HUD leem.
    app.init_resource::<viber::menus::MenusOpen>();
    app.init_resource::<viber::music::MusicLayerOverride>();
    app.init_resource::<viber::music::CombatMusicState>();
    app.init_resource::<viber::hud::markers::HudMarkers>();
    app.init_resource::<viber::music::ScriptSoundLoops>();
    // O véu de loading: sai só com o mundo inteiro (GLBs, GPU, herói assente).
    app.add_plugins(viber::world_ready::WorldReadyPlugin);
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
    app.add_systems(
        bevy::app::Update,
        bevy_ext_engine::camera_glue::fov_kick_system,
    );
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
    app.add_plugins(viber::terrain::water_waves::WaterWavePlugin);
    app.add_plugins(viber::terrain::water_fx::WaterFxPlugin);
    // Adaptadores do jogo (fonte do terreno, ambiente, obstáculos avian/voxel,
    // partículas e marcador `WaterBodied`).
    app.add_plugins(viber::terrain::water_bridge::WaterBridgePlugin);
    // Corpos que flutuam (barcos/jangadas/bóias): integra Arquimedes por
    // coluna do casco, currante e encalhe.
    app.add_plugins(viber::terrain::water_buoyancy::WaterBuoyancyPlugin);
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
    // O retrato do MUNDO que os binds leem (`publish_ui_world_state` pede-o por
    // `ResMut`): o `UiPlugin` do crate não o inicializa, e o campo em falta
    // faria a system falhar a validação em TODOS os frames.
    app.init_resource::<viber::ui::host::UiWorldState>();
    // Feedback de impacto (números/mensagens); o pool 2D é decisão do jogo:
    // o Vael desenha o texto no mundo (bevy_rich_text3d).
    app.add_plugins(bevy_ext_engine::feedback::FeedbackPlugin);
    app.insert_resource(bevy_ext_engine::feedback::DamageNumbers2d(false));
    // O retrato do MUNDO (relógio/chuva/zona + `display-name`) e a fonte do HUD
    // chegam ao crate `bevy_ext_ui` antes do collect ler os binds; sem eles os
    // binds de tempo/clima ficavam vazios e as listas caíam na fonte omissa.
    app.add_systems(
        bevy::app::Update,
        (
            viber::ui::publish_ui_world_state,
            viber::ui::publish_ui_font,
        )
            .before(viber::ui::UiSet::Collect),
    );
    // `viber.ui` e o runtime Luau (BMS): os scripts correm no Update
    // (`viber::luau::bms_bridge::VaelScripts` — o handler `on_update`),
    // enfileiram mutações `viber.ui.*`, e o `UiSet::Script` aplica-as
    // DEPOIS — sem esta ordem o HUD (`combat.lua` combo, profiler) ficava
    // um frame atrás e o `viber.ui.is_open` do mesmo frame mentia.
    // O dreno dos comandos BMS corre entre os scripts e a aplicação
    // (`apply_bms_commands`), também antes do set de UI.
    //
    // NÃO re-adicionar a system aqui: o `BmsBridgePlugin` já a regista no
    // `Update` (com `.after(drain_commands)`), e uma 2.ª instância torna o
    // `SystemTypeSet` ambíguo — o schedule `Update` inteiro falha a
    // inicialização (e depois corre um executável vazio, em silêncio: sem
    // movimento, sem portão de spawn, sem spawner de GLB). Aqui só liga a
    // ORDEM: o set de UI é depois da instância única do plugin.
    app.configure_sets(
        bevy::app::Update,
        viber::ui::UiSet::Script.after(viber::luau::bms_bridge::apply_bms_commands),
    );
    // Spawn do mundo: arranque e trocas de cena (o mesmo spawner). O spawn
    // junta-se ao set do contrato do nav crate: o `NavPlugin` aplica a
    // `<NavMesh>` e cria o arquipélago DEPOIS dele (Startup) e o handle do
    // terreno (`nav::publish_nav_ground`) sai a seguir ao spawn.
    app.add_systems(
        bevy::app::Startup,
        (spawn::startup, viber::nav::publish_nav_ground)
            .chain()
            .in_set(viber::nav::WorldSpawned),
    );
    // Trocas de cena: o mesmo spawn, num schedule re-executável. O par
    // tag+arquipélago do crate só entra no `SceneLoad` do CORE e o runtime
    // corre O SEU (`viber::scene_loader::SceneLoad`) — volta a registar-se
    // aqui, a seguir ao spawn e ao handle (o par não é idempotente: corre num
    // schedule que corre mesmo).
    app.add_systems(
        viber::scene_loader::SceneLoad,
        (
            spawn::startup,
            viber::nav::publish_nav_ground,
            viber::nav::apply_navmesh_tag,
            viber::nav::spawn_archipelago,
        )
            .chain(),
    );
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

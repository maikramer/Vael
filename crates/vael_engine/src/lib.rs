//! Viber — native Bevy engine for AiGameKit declarative world XML.
//!
//! Library crate exposing the XML loader (`xml`), the recipe IR (`recipes`)
//! and gameplay modules (`terrain`); the `viber` binary is a thin CLI on top.

// Os sistemas Bevy exprimem queries grandes e os testes das VMs constroem o
// estado campo a campo; estes lints são ruído de estilo neste código.
#![allow(
    clippy::assertions_on_constants,
    clippy::field_reassign_with_default,
    clippy::too_many_arguments,
    clippy::type_complexity
)]

// NOTA(exit2/scene, 2026-11-15): `config`, `interact`, `meshopt`,
// `particles`, `animation` (o `AnimatedScene`), `hud`, `music`, `streaming`,
// `spawn_gate` e `attachment` TIVERAM gémeos no `bevy_ext_scene_runtime` —
// o crate foi APAGADO (órfão: zero consumidores; cópias reconciliadas aqui
// em `f565750`). Este crate voltou a ser a FONTE ÚNICA destes módulos: na
// saída vão DIRECTOS para os crates de domínio (fases D/F/G do
// DELETE-MONOLITH.md, no Vael) — sem crate-hub novo; a carga de mundo é o
// formato bsn/Jackdaw (`bevy_ext_world::bsn` + specs tipadas
// `bevy_ext_scene`). O `terrain` e o `physics` JÁ são shims para os crates
// extraídos; `a11y`/`locale`/`vitals`/`camera`/`feedback`/`editor_camera`
// também (core/engine, exit2/luau).
// Acessibilidade movida ao `bevy_ext_core`; o caminho `viber::a11y` continua
// a resolver e o recurso é O MESMO (ver o `pub use` na lista em baixo).
pub mod adaptive_quality;
// IA movida ao `bevy_ext_ai` (exit4/G1): sentidos (`senses`), FSM simples,
// serviço e o integrador de locomoção — os TypePaths `viber::ai`/`viber::
// senses` ficam congelados no crate. O adaptador de host é o `ai_host`
// (o `Ground` do crate passa a ser o `TerrainRuntime` da engine).
pub use bevy_ext_ai::ai as ai;
pub mod ai_host;
pub mod ambient;
pub mod animation;
pub mod app;
pub mod attachment;
// Som 3D + oclusão movidos ao `bevy_ext_audio` (onda exit3/wave-b); o
// caminho `viber::audio_space` reexporta o módulo e o TypePath do
// `SpatialOneShot` fica congelado lá.
pub use bevy_ext_audio::audio_space as audio_space;
// Contratos de HOST do `bevy_ext_audio` (exit4/G1): o espelho
// `AudioWorldState`, os paths de clips (`AudioPaths`) e o marker `AudioEar`
// saem dos recursos da engine — os sistemas `music::*` do crate leem-nos
// daqui (o adaptador registava-se junto dos sistemas de música no `app.rs`).
pub mod audio_host;
pub mod audit;
pub mod bake;
pub mod bridge;
// Câmara de jogo (third-person/script/FX) movida ao `bevy_ext_engine`
// (onda exit2/luau); os contratos (`OrbitCamera`, `ScriptCameraFree`,
// `CameraSnapshot`) são do `bevy_ext_core` — o caminho `viber::camera`
// reexporta o módulo e os tipos são OS MESMOS no jogo.
pub use bevy_ext_engine::camera_glue as camera;
// Cinemáticas movidas ao `bevy_ext_engine` (exit5/G-2); o caminho
// `viber::cinematic` reexporta o módulo e os TypePaths `viber::cinematic::`
// ficam congelados no crate. O gémeo tem os formatadores de pose locais
// (réplica dos `scene::pose` puros) e o gate de pausa por relva ocupada.
pub use bevy_ext_engine::cinematic as cinematic;
pub mod cinematic_cli;
// Contrato `config.yaml` movido ao `bevy_ext_engine` (onda exit3/wave-b) —
// o MESMO tipo no jogo; o `fixture()` dos testes do audit também vem daí.
pub use bevy_ext_engine::config as config;
// Módulo movido ao `bevy_ext_physics` (onda exit2/hubs); o caminho
// `viber::corpse` continua a resolver e o tipo é o MESMO no jogo.
pub use bevy_ext_physics::corpse;
pub mod decal_fx;
pub mod dialogue;
pub mod draw_diag;
// Câmara de editor movida ao `bevy_ext_engine` (onda exit2/hubs); o
// `EditorCameraPlugin` é o MESMO tipo no jogo.
pub mod editor_camera;
// Feedback de impacto movido ao `bevy_ext_engine` (onda exit2/hubs); o
// caminho `viber::feedback` reexporta o módulo — o `FeedbackPlugin`
// registado no `app.rs` é o do engine (mesma mensagem para o Vael).
pub use bevy_ext_engine::feedback;
pub mod foliage;
pub mod frame_pacing;
pub mod fx_specs;
pub mod glb_bounds;
pub mod gltf_residency;
pub mod instance_residency;
pub mod graphics;
pub mod hud;
pub mod ibl;
pub use bevy_ext_physics::ik;
pub mod impact;
pub mod input_map;
pub mod interact;
pub mod lint;
// Acessibilidade movida ao `bevy_ext_core` (onda exit2/luau); o caminho
// `viber::a11y` reexporta o módulo — um só recurso `A11y` no jogo.
pub use bevy_engine_core::a11y;
// Texto localizado movido ao `bevy_ext_core` (onda exit2/luau); o host de
// UI (`bevy_ext_ui::ui::host`) reexporta O MESMO `Locale`.
pub use bevy_engine_core::locale;
// Vitais (`Health`/`Hostile`/dano) movidos ao `bevy_ext_core` (onda
// exit2/luau); `bevy_ext_engine` e `bevy_ext_ai` reexportam o MESMO módulo.
pub use bevy_engine_core::vitals;
pub mod luau;
pub mod menus;
pub mod meshopt;
// Música/buses/ducking/reverb movidos ao `bevy_ext_audio` (exit4/G1); o
// caminho `viber::music` reexporta o módulo e os TypePaths `viber::music::`
// ficam congelados lá. O adaptador de host é o `audio_host` — o
// `music_driver`/`script_sound_loops` do crate leem `AudioWorldState`/
// `AudioPaths` em vez dos recursos da engine.
pub use bevy_ext_audio::music as music;
pub mod nav;
pub mod netplay;
pub mod particles;
pub mod physics;
pub mod physics_fx;
pub mod placement;
pub mod player;
pub mod postfx;
pub mod postfx_toggle;
pub mod probes;
pub mod profiler;
pub mod project;
pub mod projectile;
pub mod prop_tint;
pub mod prune;
// Ragdoll movido ao `bevy_ext_physics` (onda D): o caminho
// `viber::ragdoll` reexporta o módulo e os tipos são OS MESMOS no jogo.
pub use bevy_ext_physics::ragdoll;
pub mod recipes;
pub mod reflect;
pub use bevy_ext_engine::render_lod;
pub mod rng;
pub mod save;
pub mod scaffold;
pub mod scene;
pub mod scene_loader;
pub mod script_kind;
// Sentidos (visão/audição da FSM) movidos ao `bevy_ext_ai` (exit4/G1); o
// caminho `viber::senses` reexporta o módulo e os TypePaths `viber::senses::
// …` ficam congelados no crate.
pub use bevy_ext_ai::senses as senses;
pub mod session;
pub mod sky;
pub use bevy_ext_engine::sky_nishita;
pub mod spawn_gate;
pub mod spawner;
pub mod streaming;
pub mod world_ready;
pub mod terrain;
/// Glue IR→shader do terreno (fase D de `docs/DELETE-MONOLITH.md`): o walk
/// de `<DayCycle>`/`<Weather>`/`<Lake>`/`<Sky>` não pode viver no
/// `bevy_ext_terrain_runtime` (o IR de parsing não é dependência dele). O
/// `app.rs` do Vael cita `viber::terrain_host::{water_surface_config_from_world,
/// terrain_chunk_config_from_world}` no arranque.
pub mod terrain_host;
pub mod textures;
pub mod time_fx;
pub mod ui;
pub mod vinst;
pub mod water_ssr;
pub mod xml;

// Drivers do mundo movidos ao `bevy_ext_engine` (exit5/G-2): relógio/sol/
// clima/atmosfera/interior + seating/`place` no terreno + `PendingWorldSystems`
// (o acumulador que o loader alimenta). O caminho `viber::worldsys` reexporta
// o módulo e os TypePaths `viber::worldsys::` ficam congelados no crate; os
// ESTADOS que a lente lê (`AtmosphereState`/`WeatherState`/`SunState`/biomas)
// têm o dono único no `bevy_ext_postfx::env` desde a exit3/wave-c.
pub use bevy_ext_engine::worldsys as worldsys;

/// Checkout do jogo Vael, irmão deste repo.
///
/// `VAEL_ROOT` ganha. Sem a variável, é a raiz do Vael (dois acima do
/// crate. Testes que leem o mundo do jogo saltam quando isto é `None`
/// (CI da engine sozinha).
pub fn vael_root() -> Option<std::path::PathBuf> {
    let candidate = match std::env::var_os("VAEL_ROOT") {
        Some(value) => std::path::PathBuf::from(value),
        None => std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."),
    };
    let candidate = candidate.canonicalize().unwrap_or(candidate);
    candidate.is_dir().then_some(candidate)
}

//! Scripting do monólito APÓS a onda final do DELETE-MONOLITH (2026-11-15):
//! o host Luau vivo (`host`/`runtime`/`api`/`ctx::ScriptCtx`/`timers`/
//! `entity`/`contacts`/`signals`/`props`/`fx`/`terrain` + o hot-reload de
//! chunks + o `LuauScriptPlugin`) foi APAGADO. O jogo (Vael) corre os
//! scripts no `bevy_ext_luau` (BMS 0.24) desde `218423e`; a engine
//! standalone (`viber run`) corre SEM scripts (`ScriptsMode::Off`).
//!
//! O que aqui ficou:
//! - [`bms_bridge`]: a ponte VIVA com o BMS — espelha `LuaScriptRef` →
//!   `ScriptComponent` (attach por entidade), publica o estado do frame no
//!   `ScriptHostState`, converte e aplica os comandos dos scripts
//!   ([`commands::ScriptCommand`], o vocabulário do applier), entrega
//!   sinais como callbacks BMS e aplica o dreno pós-frame. O jogo regista
//!   `vael_engine::luau::bms_bridge::BmsBridgePlugin`.
//! - [`commands`]: o enum dos comandos script→engine que o applier da ponte
//!   consome. É QUASE o gémeo do `bevy_ext_luau::commands` (o conversor
//!   `convert_bms_command` faz a tradução 1:1 + as variantes sem geméa
//!   aplicadas inline) — unificá-los é cosmético e fica para depois.
//! - [`ctx`]: só os registos de superfície (estradas/água por arco) que a
//!   ponte publica pelo seam `SurfaceQueries` e a REPL do bridge consulta.
//! - SHIMS: os caminhos históricos `vael_engine::luau::*` (save/BRP/reflexão do
//!   editor) continuam a resolver por `pub use` dos donos novos
//!   (`bevy_ext_luau` / `bevy_engine_core` / `bevy_ext_engine`):
//!   [`components`], [`events`], [`exports`], [`game`], [`input`], [`nlu`],
//!   [`ownership`], [`decision`], [`sfx`].

// SHIM: TypePaths `viber::luau::components` congelados no bevy_ext_luau.
pub mod components;
// SHIM: a fila de eventos vive no core (UI e scripts partilham o tipo).
pub mod events;
// SHIM: o parser de teclas vive no keys do bevy_ext_luau.
pub mod input;
// SHIM: exports/ownership/game/nlu/decision — vivos no bevy_ext_luau.
pub mod exports;
pub mod game;
pub mod ownership;
pub mod nlu;
pub mod decision;
// SHIM: registry `vael.sound`/gestos — vivos no bevy_ext_engine::luau.
pub mod sfx;
// SHIM: a escrita genérica por reflexão (patch_component) vive no
// bevy_ext_luau::ecs; os testes do maquinaria ficaram aqui.
pub mod ecs;
// A ponte BMS viva (o jogo regista o plugin dela).
pub mod bms_bridge;
// Vocabulário dos comandos script→engine que o applier da ponte consome.
pub mod commands;
// Registos de superfície (estradas/água) para a ponte e a REPL.
pub mod ctx;
// SHIM: o evento UnboundGesture vive no bevy_ext_luau.
pub mod runtime;

#[cfg(test)]
mod tests;

pub use components::{
    DEFAULT_ACTIVATION_RADIUS, Groups, LuaScriptRef, ScriptActivation, ScriptInteraction,
    ScriptProps, ScriptSay, ScriptToast, SignalConnections, link_components,
};
pub use ctx::SurfaceRegistries;
pub use events::{EVENT_QUEUE_CAP, ScriptEventQueue, ScriptGameEvent};
pub use input::{InputCode, input_code_from_str, key_code_from_str};
pub use ownership::ScriptSystemOwners;
pub use runtime::UnboundGesture;
pub use sfx::{SFX_NAME_REGISTRY, match_gesture_clip, sfx_clip_from_str};

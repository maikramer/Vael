//! Fila de eventos engine→Lua — SHIM (2026-11-15, onda final do
//! DELETE-MONOLITH). O CONTRATO dos eventos vive no `bevy_ext_core::scripting`
//! (a UI declarativa empurra `UiAction` para A MESMA fila que os scripts
//! drenam); o `EVENT_QUEUE_CAP` é do `bevy_ext_engine`; a conversão para
//! tabela e o fan-out por contexto correm no `bevy_ext_luau::events` (BMS).
//! Os caminhos históricos `viber::luau::events::*` continuam a resolver.

pub use bevy_engine_core::scripting::{ScriptEventQueue, ScriptGameEvent};
pub use bevy_ext_engine::luau::EVENT_QUEUE_CAP;

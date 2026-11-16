//! Input para scripts: o parser de teclas/rato — SHIM (2026-11-15, onda final
//! do DELETE-MONOLITH). O parser vive agora no `bevy_ext_luau::keys` (portado
//! de aqui, exit4/F4: serve `viber.input.*`, `set_interaction` e
//! `interacted`); os caminhos históricos `viber::luau::input::*` continuam a
//! resolver. A tabela `viber.input` (VM antiga) morreu com o host.

pub use bevy_ext_luau::keys::{InputCode, input_code_from_str, key_code_from_str};

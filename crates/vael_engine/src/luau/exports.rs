//! Props exportadas de scripts (o `@export` do Godot para Luau) — SHIM
//! (exit4/F4). O módulo vive agora no `bevy_ext_luau::exports` (std +
//! serde_json, zero deps do monólito — o parser é o MESMO para o runtime
//! (`viber.prop`), lint, esquema para agentes e Inspector; um só contrato).
//! Os caminhos históricos `vael_engine::luau::exports::*` continuam a resolver.

pub use bevy_ext_luau::exports::*;

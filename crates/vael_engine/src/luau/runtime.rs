//! Runtime Luau — SHIM de um só tipo (2026-11-15, onda final do
//! DELETE-MONOLITH). Os sistemas do host (`luau_on_add`/`luau_update`/
//! `luau_on_remove`/`aggro_alert_system` + o `LuauRuntimeLocals`) morreram
//! com ele; o que sobrou é o EVENTO [`UnboundGesture`] — `viber.gesture`
//! numa entidade sem rig da engine — que agora vive no `bevy_ext_luau` (o
//! caminho histórico `viber::luau::runtime::UnboundGesture`, que o jogo
//! observa, continua a resolver).

pub use bevy_ext_luau::UnboundGesture;

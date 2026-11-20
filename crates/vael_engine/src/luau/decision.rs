//! `viber.decision` — SHIM (exit4/F8). O módulo vive agora no
//! `bevy_ext_luau::decision` (feature `decision` lá, com a MESMA path-dep
//! CelerAI que este crate tinha). Os caminhos históricos
//! `vael_engine::luau::decision` continuam a resolver.

pub use bevy_ext_luau::decision::*;

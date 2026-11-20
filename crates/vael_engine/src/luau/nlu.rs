//! `viber.nlu` — SHIM (exit4/F8). O módulo vive agora no
//! `bevy_ext_luau::nlu` (feature `nlu` lá, com a MESMA path-dep CelerAI que
//! este crate tinha). Sem a feature (ou sem o artefacto), `available()`
//! devolve `false` e os scripts seguem no fluxo antigo — o mesmo contrato.
//! Os caminhos históricos `vael_engine::luau::nlu` continuam a resolver.

pub use bevy_ext_luau::nlu::*;

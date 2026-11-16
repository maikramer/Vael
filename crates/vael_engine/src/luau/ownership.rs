//! Posse de SISTEMAS nativos por scripts (`viber.own_system`) — SHIM
//! (exit4/F4). O módulo vive agora no `bevy_ext_luau::ownership` (o
//! `ScriptSystemOwners` leva o TypePath histórico `viber::luau::ownership`
//! congelado lá); os caminhos `viber::luau::ownership::*` continuam a
//! resolver e o recurso é O MESMO no jogo.

pub use bevy_ext_luau::ownership::*;

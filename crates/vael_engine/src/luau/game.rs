//! Estado de JOGO world-scoped e módulos partilhados — SHIM (exit4/F3).
//!
//! O módulo vive agora no `bevy_ext_luau::game` (o ÚNICO `game.rs`): o
//! proxy de `viber.game()` sobre o store plano partilhado, `viber.state()`,
//! `viber.load`/`load_json`, `evict_module` (hot-reload/bridge) e a interop
//! JSON do save (`game_to_json`/`restore_game_json`). Os caminhos históricos
//! `viber::luau::game::*` continuam a resolver por este re-export.
//!
//! Diferência visível: `viber.game()` é um PROXY (não uma tabela de VM
//! registada) — os primitivos vivem no store do processo (o que o save
//! serializa), tabelas/funções são overlay de sessão da VM e MORREM no load
//! (`restore_game_json` substitui o estado com bump de geração). O semear
//! de testes deixa de ser `named_registry_value("viber_game")` e passa a
//! escrever via Lua (`viber.game().x = …`) ou via `game_snapshot`.

pub use bevy_ext_luau::game::*;

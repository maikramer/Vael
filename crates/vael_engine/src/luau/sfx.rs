//! Registry nome→clip para `viber.sound` (case-insensitive, tabela única —
//! um clip novo no enum SEM linha aqui falha o teste
//! `test_lua_registry_covers_all_clips`) e o fuzzy match de gestos
//! (`viber.gesture`) contra os clips do rig.

// O registry `viber.sound` e o lookup de clips saíram para o
// `bevy_ext_engine::luau` (onda exit2/hubs) — o caminho histórico continua
// a resolver. Aqui fica o fuzzy match de gestos (usa o contrato de
// animação congelado no core) reexportado do engine também.
pub use bevy_ext_engine::luau::{SFX_NAME_REGISTRY, match_gesture_clip, sfx_clip_from_str};


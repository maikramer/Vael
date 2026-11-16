//! Componentes e mensagens da ligação entidade↔script — SHIM (2026-11-15,
//! onda final do DELETE-MONOLITH). Os tipos vivem agora no
//! `bevy_ext_luau::components` (os TypePaths `viber::luau::components` estão
//! CONGELADOS lá — são chaves de save/BRP/reflexão do editor); os caminhos
//! históricos `viber::luau::components::*` continuam a resolver e os tipos
//! são OS MESMOS no jogo.

pub use bevy_ext_luau::components::*;

/// Wrapper com o tipo autoral do nó do monólito ([`crate::recipes::NodeLinks`]):
/// o genérico vive no `bevy_ext_luau` e recebe os dados crus.
pub fn link_components(
    links: &crate::recipes::NodeLinks,
    label: &str,
) -> (Option<Groups>, Option<SignalConnections>) {
    bevy_ext_luau::components::link_components(&links.groups, &links.connections, label)
}

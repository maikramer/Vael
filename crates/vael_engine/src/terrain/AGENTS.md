# AGENTS.md — src/terrain (SHIM)

A partir da fase D de `docs/DELETE-MONOLITH.md` este diretório é um **shim**:
o runtime de terreno vive em
`../../../../../bevy_ext_terrain_runtime` (contratos no `host.rs` e no
`lib.rs` desse crate; TypePaths `viber::terrain::…` congelados lá, guarda de
wire em `delta.rs`). O `mod.rs` faz `pub use bevy_ext_terrain_runtime::*` —
todos os caminhos `viber::terrain::…` continuam a resolver.

Fica aqui apenas o que precisa do monólito:

| Ficheiro | Responsabilidade |
|----------|------------------|
| `water_bridge.rs` | adaptadores do jogo: `WaterSplashEvent` → `spawn_burst` de partículas, filtro de corpos (`player`/`ai`/`ragdoll`), ambiente (`worldsys`) e foco — a crate de propósito não conhece o jogo |

O glue do IR de cena (`<DayCycle>`/`<Weather>`/`<Lake>` → configs de shader)
está em `../terrain_host.rs`. Edits no terreno: compila e testa no crate
(`cargo test -p bevy_ext_terrain_runtime`), não aqui.

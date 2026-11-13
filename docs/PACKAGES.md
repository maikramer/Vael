# PACKAGES.md — pacotes e fases (lado Vael)

> **O roadmap canónico é
> [`../../bevy_extensions/docs/PACKAGES.md`](../../bevy_extensions/docs/PACKAGES.md).**
> Os pacotes chamam-se hoje `bevy_ext_*` (+ `bevy_voxel_terrain`) e vivem no
> monorepo `../bevy_extensions`, com CI e publicação próprios. O registo de
> adoção de dependências da engine é
> [`../../Viber/docs/CRATES.md`](../../Viber/docs/CRATES.md).
>
> Este documento guarda só o que é do **Vael**: o que consome, a sequência da
> Fase 4, os gates e as decisões que afectam o jogo. O plano antigo (família
> `bevy_*`, DAG completo, 15 pacotes) fica na história do git; não o manter
> aqui em paralelo.

## Dependências do Vael hoje

| Dependência | Papel no Vael |
|---|---|
| `bevy 0.20.1` | motor; o Vael fixa a versão e usa o lockfile |
| `viber` (path) | fachada transitória: `build_app`, terreno, nav, física, AI, Luau `Compat` |
| `viber_tools_runtime` (path) | namespace `viber-tools/*` no `bevy_remote` (debug/profiling) |
| `bevy_ext_world` (path) | `WorldNode`/`.bsn`, `XmlNode`, settings autorados |
| `bevy_ext_elements` / `bevy_ext_flora` / `bevy_ext_postfx` (path) | definições `world/settings/*.bsn` (ambiente, flora, pós-processamento) |
| `bevy_elements 0.8` | céu/tempo/clima (ver `LIGHTING.md`) |
| `bevy_pf 0.5.2` | UI do jogador em XAML (ver `MIGRATION.md`) |
| `bevy_flora 0.47` | árvores procedurais (ver `FLORA.md`) |
| `bevy_speech_bubbles 0.3` | falas no mundo (`src/speech.rs`) |
| `bevy_rich_text3d 0.8.1` | texto 3D de dano/cura/combo (`src/world_text.rs`) |
| `bevy_gauge 0.6` | atributos/stats (`src/stats.rs`) |
| `bevy_rapier3d 0.37` | controller/collider do herói |
| `jackdaw_runtime 0.20` | descoberta de componentes e PIE do Jackdaw |
| `vael-content` (path) | esquemas/validação do conteúdo, partilhado com o editor |

O `vael` é `publish = false`; a distribuição é binário + pool de assets. O
root `viber` também é `publish = false` e fica como fachada até à Fase 4e.

## O que já saiu do Viber

Os pacotes 0.1.0 vivem no monorepo `../bevy_extensions` (2026-11-12;
publicação no crates.io pendente de token): `bevy_ext_core`, `bevy_ext_world`,
`bevy_ext_gltf`, `bevy_voxel_terrain`, `bevy_ext_terrain` e `bevy_ext_save`.
O Viber consome-os por path com os aliases históricos
(`bevy_world_xml` → `bevy_ext_world`, etc.) e re-exporta as paths antigas; o
Vael consome `bevy_ext_*` directamente quando precisa do contrato.

O `bevy_audio_buses` continua adiado: `music`/`ambient` dependem de
`player`/`worldsys`/`particles`/`editor_camera`/`luau::sfx`. O próximo passo
é inverter essas dependências para o core, não mover os ficheiros como estão.

## Fase 4 — o Vael assume o runtime

| Onda | Estado |
|---|---|
| 4a — player/câmara no Vael, autoridade de vitais default | **feito** (`player_camera:false`; `VAEL_RULES_RUST=0` volta ao Lua) |
| 4a — merge `world_kv`/`game.vael`, respawn/teleporte/refill em Rust | pendente |
| 4b — FSM/inimigos/bosses/spawns/loot, combate/abilities/hotbar, equipamento visual | pendente; colheita, marcos, mapa e viagem já são Rust |
| 4c — UI/menu/diário/mapa/título/morte | **feito** em `bevy_pf`; story/cinematics e áudio direcional pendentes |
| 4d — diálogo/NLU/riddle/judge/parley em Rust com `celer-*` | pendente; a barra de fala/NLU continua na engine |
| 4e — remover `bevy_luau` e o root `viber` do grafo do Vael | pendente |

Gate da Fase 4: grafo do Vael só com Bevy/`bevy_ext_*`/`celer-*`; suite do
Vael verde; QA de combate/interiores/save; performance não pior que o
baseline medido no HEAD.

## Princípios (continuam válidos)

1. Nenhum tipo específico do Vael (quests, itens, diálogos, `VaelPlayer`) nos
   `bevy_ext_*`.
2. Configuração por resources/plugins; sem layout de projeto hardcoded.
3. Features opcionais para dependências pesadas (AI, voxel, Luau, editor).
4. API pública pequena e documentada; `cargo-semver-checks` no release.
5. Testes unitários autocontidos; testes que dependem do Vael ficam no Vael.
6. Licença dupla MIT/Apache-2.0; modelos/artefactos mantêm as licenças
   upstream.

## Gates

- Vael: `cargo test --lib`, `VIBER_NO_BUILD=1 ./tools/check.sh`,
  `viber analyze world.bsn`, `viber scene lint --world world.bsn`.
- Pacotes: `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`, `cargo package --list` e
  `cargo publish --dry-run --locked`; `.crate` abaixo do limite do crates.io.
- Release: `tools/publish.sh` no `bevy_extensions` (dry-run primeiro); a
  publicação real precisa de token e é feita pelo utilizador.

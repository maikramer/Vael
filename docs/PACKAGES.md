# PACKAGES.md — do Viber monolítico aos pacotes `bevy_*`

Estado: Fase 0 em execução (2026-11-11). Este documento é o canónico das
fases; o `docs/MIGRATION.md` descreve a migração do jogo e o
`../Viber/docs/CRATES.md` a adoção de dependências da engine.

## 1. Contexto

O Viber é hoje um monólito de ~170k linhas Rust (lib + bin, 82 módulos
públicos) mais ~32k linhas de editor. O Vael depende dele por path, usa 28
caminhos internos em 86 referências Rust e 1.867 chamadas `viber.*` em 13,5k
linhas de Luau. O `build_app`/`AppOptions` já separa parcialmente engine e
jogo, mas o Vael continua a ser um plugin dentro da App do Viber.

O objetivo é dividir o Viber em pacotes `bevy_*` publicáveis e mantidos no
repositório, substituir os forks vendorizados por crates do registry e
preparar o dia em que o Vael se compõe diretamente sobre Bevy e o monólito
Viber é aposentado. O critério de sucesso é a manutenção do Vael:
dependências explícitas, semver, builds menores e peças substituíveis sem
arrastar a engine inteira.

Bloqueios atuais: path-deps sem versão; forks vendorizados de navegação;
patch local do `rerecast`; `nlu`/`decision` no default; package acima do
limite do crates.io; falta de licença/metadata; colisão de
`ort::Environment` no CelerAI; acoplamento de módulos “genéricos” a
`player`/`menus`/`hud`/`save`.

## 2. Decisões fixadas

- **Nomes**: família `bevy_*` genérica. Nomes candidatos verificados livres
  no crates.io a 2026-11-11 (exceto `bevy_persist`, que não será usado).
- **Luau**: zero Lua no runtime final do Vael. `bevy_luau` continua
  publicado para editor/exemplos, mas o Vael não o usa no fim.
- **Editor**: o Vael usa o **Jackdaw 0.20** (`vael-editor`) para autoria de
  conteúdo e cenas `.bsn`; o Viber deixou de trazer `vibereditor`. Não há
  pacote `bevy_debug_editor`: a plataforma de debug é o bridge/CLI da engine
  e o editor de jogo é uma App do Vael, fora da família `bevy_*`.
- **Âmbito da Fase 0**: escrever este roadmap e executar a fundação. Não
  extrair pacotes de serviço agora.

### Nomes verificados

| Nome | crates.io em 2026-11-11 |
|---|---|
| `bevy_engine_core` | livre |
| `bevy_world_xml` | livre |
| `bevy_save_slots` | livre |
| `bevy_audio_buses` | livre |
| `bevy_character_anim` | livre |
| `bevy_ui_xml` | livre |
| `bevy_streaming_world` | livre |
| `bevy_fx` | livre |
| `bevy_physics_ext` | livre |
| `bevy_terrain_ahgt` | livre |
| `bevy_voxel_terrain` | livre (reserva se o voxel sair do terreno) |
| `bevy_navmesh_bake` | livre |
| `bevy_luau` | livre |
| `bevy_debug_bridge` | livre |
| `celer-nlu` / `celer-decision` | livres |

Reservar os nomes antes da primeira publicação (0.0.0 ou 0.1.0). O
`bevy_persist` já existe e não entra na família.

## 3. Arquitetura-alvo

### 3.1 DAG

```text
celer-nlu ──► celer-decision

bevy_engine_core
├── bevy_world_xml
├── bevy_save_slots
├── bevy_audio_buses
├── bevy_character_anim
├── bevy_ui_xml
├── bevy_physics_ext
│   └── bevy_terrain_ahgt
│       ├── bevy_streaming_world
│       └── bevy_navmesh_bake
├── bevy_fx
├── bevy_luau (features opcionais)
└── bevy_debug_bridge
```

O root `viber` fica como app/fachada transitória (`publish = false`):
re-exporta os pacotes e mantém `build_app`, player/câmara, menus, HUD, AI e
CLI atuais até à Fase 5. O `vael` é `publish = false`; no fim
depende só de `bevy_*`, `celer-*` e do ecossistema Bevy.

### 3.2 Mapa de módulos

| Pacote | Extrai | Depende de |
|---|---|---|
| `bevy_engine_core` | contratos, schedule sets, markers (`PlayerBody`, `UiSet`, `SceneLoad`), traits (`TerrainQuery`), `rng` | bevy |
| `bevy_world_xml` | `xml`/`values`/`include`/`emit`, `project`/`root_of`, `vinst`, `DeltaPatch`; `bsn` (WorldNode + loader/emissor `.bsn`); `config`/`scene` ficam no root por agora | core |
| `bevy_save_slots` | `save` (envelope, slots, paths, IO atómico; o plugin fica no root) | core, world |
| `bevy_audio_buses` | `music`, `ambient`, `audio_space` | core, world, kira |
| `bevy_character_anim` | `animation`, `attachment`, `ik` | core, world |
| `bevy_ui_xml` | `ui`, `locale`, `a11y`, widgets de HUD | core, world |
| `bevy_streaming_world` | `streaming`, `render_lod`, `meshopt`, `textures`, `glb_bounds`, `prop_tint`, `grass`, `foliage`, vegetação | core, world, terrain |
| `bevy_fx` | `particles`, `decal_fx`, `time_fx`, `impact` | core, world |
| `bevy_physics_ext` | `physics`, `physics_fx`, `projectile`, `ragdoll`, `spawn_gate` | core, world, rapier |
| `bevy_terrain_ahgt` | `terrain` (heightmap, splat, LOD, roads, water, pads, decals, shore) + `terrain::voxel` (feature) | core, world, physics |
| `bevy_navmesh_bake` | `nav` (backend, bake, agentes) | core, world, terrain, physics, `bevy_rerecast`/`bevy_landmass`/`landmass_rerecast` |
| `bevy_luau` | `luau` (host, API base, `ecs`, hot reload), `reflect` | core + features dos serviços |
| `bevy_debug_bridge` | `bridge`, `profiler` | core, world, save, `bevy_remote` |

Fica no root `viber` até morrer: `app`, `player`, `camera`, `menus`, `hud`,
`feedback`, `vitals`, `interact`, `dialogue`, `worldsys`, `spawner`, `ai`,
`senses`, `netplay`, `cinematic`, `dlss`, `editor_camera`, `corpse`,
CLI/tooling (`scaffold`, `audit`, `lint`, `bake`, `session`, `prune`,
`scene::cli`, `cinematic_cli`).

### 3.3 Princípios dos pacotes

1. Nenhum tipo específico do Vael (quests, itens, diálogos, `VaelPlayer`)
   nos `bevy_*`.
2. Configuração por resources/plugins, sem layout de projeto hardcoded.
3. Features opcionais para dependências pesadas (AI, voxel, Luau, editor).
4. API pública pequena e documentada; `cargo-semver-checks` no release.
5. Testes unitários autocontidos; testes que dependem de `worlds/`/Vael
   ficam no root não publicado.
6. Licença dupla MIT/Apache-2.0; modelos/artefactos mantêm as licenças
   upstream.

## 4. Fases

| Fase | Duração | Objetivo | Gate |
|---|---|---|---|
| 0 | 2-4 sem | Fundação publicável e des-vendorização | dry-run verde; Vael sem regressão |
| 1 | 4-6 sem | Contratos + 4 primeiros pacotes | pacotes 0.1.0; Vael 236 testes |
| 2 | 8-12 sem | Mundo/render/física/nav | `main`/`menu`, navmesh, sem regressão visual |
| 3 | 8-12 sem | Luau/bridge/debug editor + fachada | serviços versionados; editor/CLI ok |
| 4 | 12-20 sem | Vael assume o runtime e zero Lua | grafo do Vael sem `viber`; QA completo |
| 5 | 2-4 sem | Fim do Viber | monólito arquivado; pacotes + Vael |

### Fase 0 — Fundação publicável (2026-11-11)

1. **Roadmap**: este documento; ponteiros em `docs/MIGRATION.md` e
   `../Viber/docs/CRATES.md`.
2. **Des-vendorizar navegação**: `bevy_rerecast` 0.6, `bevy_landmass` 0.13 e
   `landmass_rerecast` 0.4 do registry; remover os quatro diretórios de
   `vendor/`; manter `vendor/rerecast` (0.5 + dois fixes) e o
   `[patch.crates-io]` até os fixes entrarem upstream. Adaptar
   `src/nav/backend.rs`; gate: `cargo tree -d` sem duplicar Bevy/glam e
   navmesh do `main` funcional.
3. **CelerAI**: metadata (`description`, `license`, `repository`, `readme`,
   `keywords`, `categories`, `rust-version`); `celer-decision` com
   `celer-nlu = { version = "0.3.0", path = "../nlu" }`; um
   `ort::Environment` partilhado (`OnceLock`) exposto por `celer-nlu` e
   reutilizado por `celer-decision`; teste dos dois runtimes no mesmo
   processo. Modelos continuam fora do crate (GitHub Release +
   `models.py install/verify`), com notices de licença no tarball.
4. **Feature `ai` opt-in**: o default do `viber` deixa de incluir
   `nlu`/`decision`; `ai = ["nlu", "decision"]`; o Vael ativa `ai`.
   `cargo check -p viber --no-default-features` sem CelerAI; testes de
   NLU/decisão gateados pela feature. `bridge`/`profiler`/`dev-tools` ficam
   para a Fase 3 (`cfg`).
5. **Metadata/licença**: `[workspace.package]` (versão, edition, MSRV,
   licença dual, repository); `LICENSE-MIT`/`LICENSE-APACHE` no Viber e
   CelerAI; `README.md`; `NOTICE` com a OFL da Cinzel; `publish = false` em
   `viber` e `vael`; `[package] include/exclude` no root para
   não arrastar `worlds/`, `gauntlet/`, `examples/`, mantendo `src/**` e
   `assets/fonts/**`.
6. **CI/release**: jobs de `cargo package --list` e
   `cargo publish --dry-run --locked`; deps de sistema do Bevy
   (`libasound2-dev`, `libudev-dev`, X11/Wayland, `pkg-config`, `cmake`,
   `g++`); job MSRV 1.97.1; `release.toml`/`release-plz.toml` com versão
   partilhada e ordem de publicação. CI principal sem `ai`; job `ai` usa os
   crates CelerAI publicados (ou checkout irmão até estarem publicados).
7. **Piloto**: `cargo publish --dry-run` de `celer-nlu` e, depois de
   `celer-nlu` publicado/reservado, de `celer-decision`. A publicação real
   fica para o utilizador (token).
8. **Não fazer**: extrair `bevy_*` agora; mudar o comportamento do jogo;
   tocar no WIP não relacionado (`src/flora.rs`, manifests de personagens,
   `qa-*.data`); reverter alterações de outras sessões.

### Fase 1 — Contratos e primeiros pacotes (4-6 semanas)

- Criar `bevy_engine_core` com os contratos partilhados; mover `Player` →
  `PlayerBody` (marker), `MenusOpen`/`UiModalsOpen` → recursos genéricos,
  `SceneLoad`/`UiSet` → schedule sets, `TerrainQuery`/`NavQuery` → traits.
- Extrair `bevy_save_slots`, `bevy_audio_buses` e `bevy_world_xml` (parse;
  o spawn fica no root até à Fase 2).
- Introduzir o `SpawnRegistry` (handlers por tag); o root regista as
  extensões atuais para não mudar comportamento. O `LuaApiRegistry` fica
  para a Fase 3.
- Publicar os quatro pacotes 0.1.0; a fachada `viber` re-exporta
  (`pub use bevy_world_xml as xml;`, etc.).
- Gate: `cargo publish --dry-run` dos quatro; Vael 236 testes; `viber
  run`/editor iguais.

#### Estado da Fase 1 (2026-11-11)

- **`bevy_engine_core` 0.1.0** — extraído: `rng`, `script_kind`,
  `PlayerBody`, `MenusOpen`, `ScriptToast`, `AudioMixerSettings`, `UiSet`,
  `TerrainQuery`/`NavQuery`. O root re-exporta os tipos e mantém as paths
  antigas.
- **`bevy_world_xml` 0.1.0** — extraído: `XmlNode`/`XmlDocument`, o parser
  tolerante (`parse_file`/`parse_fragment`/`normalize_bare_bools`), os
  valores tipados, `vinst`/`PlacedInstance`, `project`/`root_of`, `include`,
  `emit` e o `DeltaPatch` do terreno. `config`/`GameConfig` e o `scene`
  catalog ficam no root (dependem de `terrain::splat`/gráficos e do
  runtime). `CONFIG_FILE`, `escape_attr` e `POSE_KEYS` mudaram-se para o
  pacote; o root re-exporta.
- **`bevy_save_slots` 0.1.0** — extraído: `SaveGame`, `WorldSaveKey`,
  `SaveDir`, paths/slots, `list_save_slots`, `capture`, `save_to_disk`
  (tmp+fsync+rename) e `load_from_disk` (migração legacy). O root `save.rs`
  fica com o `SavePlugin`, `apply_pending_save_patches`,
  `SaveRequest`/`OptionsRows`, `apply_save` e `options_system`. Os
  `TypePath` de `viber::save` são preservados com `#[type_path]`.
- `PlayerBody` está definido e re-exportado, mas ainda não é anexado ao
  spawn do player: o spawn vive em `recipes/spawn.rs`, que está em WIP
  concorrente. O attach entra na Fase 2, quando o `SpawnRegistry` existir.
- **`bevy_audio_buses` continua adiado.** `music`/`ambient` dependem de
  `player`/`worldsys`/`particles`/`editor_camera`/`luau::sfx`; mover os
  ficheiros arrastaria o runtime. O próximo passo é inverter essas
  dependências para o core, não mover os ficheiros como estão.
- Release (2026-11-12): a extração está commitada no Viber em `1ef434e`
  (pacotes + shims + de-vendor + docs/CI). `bevy_world_xml` tem 74 testes
  (69 unit + 5 de integração BSN), `bevy_save_slots` 2 e `bevy_engine_core`
  3; o check combinado dos três com o `viber` no-default ficou verde antes
  do commit. O editor Jackdaw do Vael usa `bevy_world_xml` por path.
  **Publicação pendente de token**: depois de `cargo login`, correr
  [`../Viber/tools/publish_bevy_packages.sh`](../../Viber/tools/publish_bevy_packages.sh)
  (ordem `bevy_engine_core` → `bevy_world_xml` → `bevy_save_slots`; o
  dry-run do terceiro só resolve depois do segundo estar no crates.io).
- O WIP concorrente do Viber (área lights, LOD scan, BSN fora dos pacotes)
  ficou fora do commit `1ef434e` e continua no working tree. O Vael está
  num WIP grande (BSN/Jackdaw/UI), por isso a suite do jogo não foi
  reexecutada nesta onda.

### Fase 2 — Mundo, render, física e navegação (8-12 semanas)

- Extrair `bevy_physics_ext`, `bevy_terrain_ahgt`, `bevy_streaming_world`,
  `bevy_fx`, `bevy_ui_xml`, `bevy_character_anim` e `bevy_navmesh_bake`.
- Inverter acoplamentos: terreno/streaming deixam de consultar
  `player::Player` (passam a `PlayerBody`/`TerrainFocus`); UI deixa de
  depender de `menus`; save deixa de depender do runtime.
- O `SpawnRegistry` cobre colliders, GLTF, luzes, partículas, UI, scripts e
  vegetação; cada pacote regista os seus handlers.
- Gate: `main` e `scenes/menu.xml` correm; navmesh gerada; colliders;
  vegetação; UI; sem regressão visual; Vael ainda na fachada.

### Fase 3 — Luau, bridge, debug editor e fachada (8-12 semanas)

- Extrair `bevy_luau` com a API base e o `LuaApiRegistry`; as APIs de jogo
  (`player_position`, `menu_open`, `save`, `vitals`, `hud`, `netplay`,
  `cinematic`) ficam no root como extensões.
- Extrair `bevy_debug_bridge` (bridge + profiler; `session`/`prune` ficam no
  CLI).
- O editor de conteúdo depois do `vibereditor`: `vael-editor` sobre Jackdaw
  em `Vael/editor` (painéis de conteúdo e cenas `.bsn`); o mundo é editado
  no Jackdaw e a CLI/bridge só toca nos `.xml` legados. O root `viber` 0.2.0
  re-exporta todos os pacotes; o CLI continua a funcionar.
- Vael passa os serviços para crates versionados; o root `viber` fica como
  path dep transitório até à Fase 4. Se o upstream do `rerecast` não tiver
  os fixes, mantém o `[patch]` local e documenta.
- Gate: Vael compila contra os crates publicados; `viber run` igual;
  dry-run de todos os pacotes.

### Fase 4 — Vael assume o runtime e mata o Lua (12-20 semanas, paralela)

- 4a: player/câmara no Vael (`player_camera:false`), autoridade de vitais
  default, merge `world_kv`/`game.vael`, respawn/teleporte/refill em Rust.
- 4b: combate/abilities/hotbar/equipamento visual, FSM/inimigos/bosses/
  spawns/loot, colheita/viagem/mapa.
- 4c: UI/menu/journal/title/death/map, story/cinematics, áudio direcional.
- 4d: diálogo/NLU/riddle/judge/parley em Rust com `celer-*`.
- 4e: remover `bevy_luau` e o root `viber` do grafo do Vael; o Vael compõe
  `DefaultPlugins` + `bevy_*` + `VaelPlugin`.
- Gate: grafo do Vael só com Bevy/`bevy_*`/`celer-*`; 236+ testes; QA de
  combate/interiores/save; performance não pior.

### Fase 5 — Decreto do fim do Viber (2-4 semanas)

- Arquivar/repurpor o monólito Viber; o root `viber` deixa de ser
  dependência do Vael.
- `vael-editor` (Jackdaw) fica como editor de conteúdo; o bridge/CLI da
  engine como plataforma de debug.
- Release do Vael (binário + assets); docs/README/badges; cadência de
  upgrades do Bevy e política semver.
- Gate: nenhum caminho do Vael depende do Viber; pacotes publicados;
  editor de conteúdo no Vael e bridge; CI/release automáticos.

## 5. Política de versões e release

- Todos os `bevy_*` usam a mesma versão (lockstep) e são publicados juntos.
  Um upgrade de Bevy é uma mudança breaking da família: sobe o minor em
  `0.x` (ou o major em `1.x`) e atualiza a matriz de compatibilidade.
- `celer-nlu`/`celer-decision` são lockstep entre si e independentes dos
  `bevy_*`.
- O Vael fixa a versão exata do Bevy e usa o lockfile; os pacotes usam
  `bevy = "0.20.x"` (caret) e documentam o MSRV.
- Ordem de publicação: `celer-nlu` → `celer-decision` →
  `bevy_engine_core` → `bevy_world_xml` → `bevy_save_slots` →
  `bevy_audio_buses` → `bevy_character_anim` → `bevy_physics_ext` →
  `bevy_terrain_ahgt` → `bevy_streaming_world` → `bevy_fx` →
  `bevy_ui_xml` → `bevy_navmesh_bake` → `bevy_luau` →
  `bevy_debug_bridge`.
- Tags: `v0.x.y` para a família; `celer-nlu-v0.x.y`/`celer-decision-v0.x.y`
  para o CelerAI. `cargo-release` com `shared-version` é o mecanismo
  preferido; `release-plz` fica como alternativa.
- `cargo package --list` e `cargo publish --dry-run --locked` são gates de
  CI. O `.crate` tem de ficar abaixo do limite do crates.io (10 MB).

## 6. Riscos

- **`rerecast`**: o patch local dos dois fixes não é publicável nem propaga.
  Mitigação: upstream dos PRs; até lá, `[patch.crates-io]` só no workspace
  de dev/Vael e documentado no README do `bevy_navmesh_bake`.
- **Bevy 0.21+**: todos os `bevy_*` sobem em lockstep; o Vael fixa a versão
  e o lockfile. Documentar a matriz.
- **`ort 2.0.0-rc.15`**: RC + download de binário; o fix do `Environment`
  partilhado é obrigatório para NLU + decision no mesmo processo.
- **Port do Luau**: 13,5k linhas; é o maior esforço da Fase 4. Pode correr
  em paralelo com as Fases 2/3.
- **Editor**: extrair um debug editor genérico de 32k linhas é
  significativo; o editor de autoria fica transitório e fora do pacote.
- **Ciclos**: terreno/física/streaming/nav; resolver com traits/recursos no
  `bevy_engine_core`; se um ciclo persistir, fundir pacotes em vez de criar
  back-edge.
- **Tamanho/testes**: excluir `worlds/`, `gauntlet/`, `examples/`, assets
  grandes; testes de integração ficam no root não publicado.
- **Nomes**: verificados livres a 2026-11-11; reservar antes da primeira
  publicação.

## 7. Verificação

- Viber: `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`, `cargo check -p viber --no-default-features`,
  `cargo check -p viber --features ai`, `cargo tree -d`.
- CelerAI: `cargo test -p celer-nlu -p celer-decision`; teste dos dois
  runtimes no mesmo processo; `cargo publish --dry-run -p celer-nlu`,
  depois `-p celer-decision`.
- Empacotamento: `cargo package --list -p <crate>` e `.crate` < 10 MB.
- Vael: `cargo test` (236), `VIBER_NO_BUILD=1 ./tools/check.sh`,
  `CARGO_TARGET_DIR=../Viber/target cargo run --release -- world.bsn
  --scene main --entry dev --bridge 15706`; QA de navmesh/AI, terreno,
  menu, HUD, combate.
- Git: commits por repo/tema; não reverter WIP de outras sessões; commitar
  paths explícitos.

## 8. Fora de âmbito

- Extrair os pacotes de serviço na Fase 0.
- Publicar o `vael` ou os assets no crates.io.
- Reescrever o editor de autoria do jogo.
- Portar o Luau na Fase 0.
- Mudar o comportamento visível do jogo.

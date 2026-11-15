# Estudo de arquitectura do Vael (2026-11-13)

> Estudo de 2026-11-13 sobre a arquitectura do sistema de cinco checkouts:
> o que é overengineering, que camadas de abstração já não pagam o custo e
> o que poderia viver em crates.io — mesmo com trabalho envolvido. O enxugo
> *interno* do crate `vael` tem o seu próprio estudo
> ([`CODE-SLIM-STUDY.md`](CODE-SLIM-STUDY.md), Fases 1–2 aplicadas no mesmo
> dia); este documento não repete esse material. As contagens levam a data
> (main de 2026-11-13) e foram medidas na altura da escrita. As decisões de
> adopção de crates vivem no
> [`../bevy_extensions/docs/CRATES.md`](../../bevy_extensions/docs/CRATES.md)
> e não são re-litigadas aqui; o estado da migração é da
> [`MIGRATION.md`](MIGRATION.md) e o das publicações é do
> [`PACKAGES.md`](PACKAGES.md) — este estudo referencia, não copia.

## Sumário executivo

- **O maior custo de arquitectura é o aparo de transição no
  `bevy_extensions`:** ~258k LOC, dos quais ~161k são o monólito Viber
  copiado (`bevy_ext_viber_engine`) e ~67k são crates já extraídos **sem
  consumidor**. As cópias gémeas divergem: medido a 2026-11-13, `style.rs`
  difere em 24 linhas, `roads.rs` em 4, `spawn.rs` já divergiu 606. O jogo
  liga os dois lados no mesmo processo (ex.: a UI extraída **e** a cópia da
  UI dentro do `viber`).
- **No crate do jogo**, o mesmo estado de jogo vive em **5
  representações** simultâneas e 3 features têm caminho duplo Lua/Rust com
  gates de env. A camada Lua inteira (~13,2k LOC) é, no fim do plano, a
  abstração a remover (Fases 4b/4d/4e); o que falta é um end-date explícito
  para o aparo intermédio (shadow de paridade, comparador de HUD, fallbacks).
- **crates.io:** tudo o que o jogo consome (`bevy_flora`, `bevy_elements`,
  `bevy_pf`, `bevy_gradmesh`, `bevy_gauge`, `bevy_rich_text3d`,
  `bevy_speech_bubbles`) é de terceiros — verificado na API a 2026-11-13.
  Os nossos publicáveis (família `bevy_ext_*` 0.1.0 + `celer-*`) esperam
  token/sequência do `PACKAGES.md`. As duas consolidações maiores são UI
  (duas stacks no mesmo processo) e save (dois envelopes); o
  `bevy_ext_postfx` (6k) é cortável a menos.
- **Um achado novo com risco real:** o Vael perdeu o patch do `rerecast` no
  exit (`29c270a`) e compila hoje com o `0.5.1` limpo do registry, enquanto
  a engine standalone usa o `0.5.0` com 2 fixes de robustez de navmesh
  (`[patch]` não atravessa path-deps). Correcção catalogada como **F0**.
- **F0 aplicado a 2026-11-13** (código + doc): patch `rerecast` reposto,
  `ui_bind`/`lighting`/`journal.lua` removidos, `attach` dobrado no waypoint,
  workspace dos `bevy_extensions` limpo (26→17 members) e o `viber_engine`
  excluído para correr standalone; falta só o QA vivo de navmesh. No mesmo
  dia o checkout `Viber` foi aposentado — o runtime vive no
  `bevy_ext_viber_engine`, e o `CRATES.md`/contrato foram salvos no
  `bevy_extensions`; o `viber-engine` foi re-apontado ao crate e o bin do
  monólito foi desbloqueado (`extern crate … as viber`, bevy_extensions
  `7b5f720` — estava com 43 erros E0433 desde o exit; o jogo só compila a
  lib).

## Âmbito e números (2026-11-13)

| Checkout | LOC Rust | Papel |
|---|---|---|
| `Vael` | 29 686 (`src/`, 338 `#[test]` em 35 ficheiros) + `content/` | o jogo: regras, HUD/UI, ponte `viber.vael` (61 fn) |
| `Vael` (Lua) | 13 182 (`scripts/`, 83 ficheiros) | o que a Fase 4 vai aposentar |
| `bevy_extensions` | ~258 000, dos quais: `bevy_ext_viber_engine` ~161 000; ~67 000 em 8 crates extraídos sem consumidor; ~16 700 nos 6 publicáveis 0.1.0 | família `bevy_ext_*` + o monólito em transição |
| `Viber` | **aposentado a 2026-11-13**; o runtime vive no `bevy_ext_viber_engine` e os docs no `bevy_extensions` | — |
| `viber-tools` | 4 crates + extensões Jackdaw | CLI `viber`, debug |
| `CelerAI` | 2 crates (NLU/d1) | modelos + inferência; já publicáveis |

O Vael não tem traits nem `dyn` em `src/` (0 ocorrências) — a indirecção do
jogo não está em traits, está em **representações duplicadas de estado** e
**caminhos duplos por feature** (secção seguinte). No lado dos extensions, o
overengineering concentra-se na **duplicação fork↔crate** e nos **wrappers
de <600 LOC**.

## Achados — overengineering e camadas que não pagam o custo

### 1. Fork-duplicação: o monólito copiado vs os crates extraídos (o maior)

`bevy_ext_viber_engine` mantém **cópias vivas** dos subsistemas já extraídos
para crates no mesmo repo. Divergência medida a 2026-11-13 (`diff`):

| Par (crate extraído vs cópia no `viber_engine`) | Linhas diferentes | Tamanhos |
|---|---|---|
| `bevy_ext_scene_runtime/src/recipes/spawn.rs` ↔ `…/recipes/spawn.rs` | **606** | 3 041 vs 3 349 |
| `bevy_ext_postfx/src/runtime.rs` ↔ `…/src/postfx.rs` | 349 | 4 312 vs 4 309 |
| `bevy_ext_nav/src/tile.rs` ↔ `…/src/nav/tile.rs` | 72 | 428 vs 424 |
| `bevy_ext_ui/src/ui/style.rs` ↔ `…/src/ui/style.rs` | 24 | 4 376 vs 4 354 |
| `bevy_ext_terrain_runtime/src/roads.rs` ↔ `…/src/terrain/roads.rs` | 4 | 3 054 vs 3 052 |

O risco é o clássico: fix num lado e não no outro. `spawn.rs` **já divergiu
de facto** (606 linhas — não é mais "quase igual"). E o Vael paga os dois
lados: liga `bevy_ext_ui` directo **e** o `src/ui` dentro do `viber`, no
mesmo binário.

**Recomendação:** inverter a direção — o `viber_engine` passa a *consumir*
os crates extraídos e perde as cópias (as waves 3–5 do plano de exit em
[`../bevy_extensions/docs/PACKAGES.md`](../../bevy_extensions/docs/PACKAGES.md),
cujos passos não se duplicam aqui). A ordem vale a pena sair da divergência
medida: `spawn` primeiro (já divergiu), depois postfx, nav, ui, roads.

### 2. Extração à frente do consumo: ~67k LOC sem consumidor

`bevy_ext_{ai,audio,luau,nav,physics,engine,terrain_runtime,scene_runtime}`
foram extraídos mas nenhum checkout os consome ainda (o `scene_runtime`
foi apagado a 2026-11-15 — o achado vale como lição); os três últimos nem
compilam dentro do workspace (path-dep ao monólito / ao checkout Viber, agora
aposentado — ficam sem compilar até apontarem ao `bevy_ext_viber_engine`). Isto não é
overengineering no código — é **processo codificado**: o custo aparece na
divergência (achado 1), nas versões transitórias e nos aliases (achado 7).
A decisão aqui é de calendário: ou as waves de consumo acontecem em sequência
curta, ou os crates não consumidos deviam congelar (sem refactors) até terem
consumidor — cada refactor num crate sem consumidor é trabalho contra um alvo
móvel.

### 3. O mesmo estado de jogo em 5 representações

Durante a transição, o estado do herói existe em: (1) tabela Luau
`viber.game()`; (2) `VaelSave` tipado (`src/save.rs`); (3) recursos ECS
(`src/components.rs`, escritos pelo sync de 10 Hz); (4) binds de HUD
(`UiScriptBinds`) escritos **dos dois lados**, com um comparador que loga
divergências (`src/hud.rs`); (5) RON/sidecar no disco. Somando o
`rules_shadow.rs` (907 LOC de simulador de paridade), o jogo mantém
maquinaria de comparação de estado a correr dentro de si.

Isto é transição honesta — mas sem end-date vira permanente. **Recomendação:**
registrar o end-state (Rust autoritativo; o KV é formato de persistência,
não runtime) e o end-date de cada peça de comparação: o comparador do HUD e
o shadow morrem quando as 3 features opt-in virarem default (F2) e os
fallbacks do `rules.lua` saírem (Fase 4b).

### 4. Caminhos duplos por feature — a camada Lua como abstração a remover

Três features já têm caminho Rust pronto mas **default Lua**:
`VAEL_HARVEST_RUST` (harvest.rs 2 743 vs `game/harvest.lua` 522),
`VAEL_TRAVEL_RUST` (vs `lib/landmarks.lua` 174), `VAEL_QUEST_EVENTS_RUST`
(vs fallbacks em `rules.lua`). A ponte `viber.vael` (61 funções em 11
módulos) serve ~13,2k LOC de Lua dos quais a maior parte vive exactamente
nas áreas pendentes do plano de migração: FSM/inimigos (4b), diálogo/NLU
(4d), story/áudio.

O fim do plano (4e) remove o Luau do grafo do Vael — ou seja, **a ponte e a
camada Lua são a abstração temporária por natureza**. O que o estudo
acrescenta: os caminhos duplos **não precisam de esperar** pela 4b/4d —
virar os 3 gates para default (com QA no pool) e apagar os caminhos Lua
correspondentes corta já a metade mais barata da duplicação (F2 no plano).

### 5. Código de engine dentro do crate do jogo

| Módulo | LOC | Veredicto |
|---|---|---|
| `src/environment.rs` | 1 068 | **Fica** — é a condução do *jogo* sobre o `bevy_elements` (SFX, chão molhado, lanternas); a parte genérica já vive no crate crates.io |
| `src/camera.rs` | 429 | **Fica** — o jogo é o dono da câmara; cortar os imports a `viber::camera` na 4e |
| `src/world_text.rs` | 424 | Pool genérico de texto 3D sobre `bevy_rich_text3d` — candidato a PR upstream (terceiro, mintlu8); baixa prioridade |
| `src/speech.rs` | 496 | Fica — presenter do jogo |
| `src/stats.rs` | 679 | Smell: um `World` Bevy próprio dentro do `StatEngine` só para as closures Luau síncronas — desaparece naturalmente na 4e |
| `src/lighting.rs` | 336 | **Removido no F0 (2026-11-13)**: o default era inerte e o modo de cadência (> 0) estava partido em Bevy 0.20; as sombras são as do `bevy_elements` |
| `src/settings.rs` | 225 | Fica — releitura viva dos `.bsn` do editor |

### 6. Wrappers e placeholders

- **`bevy_ext_flora` (102 LOC)** e **`bevy_ext_elements` (205)**: um struct
  cada, embrulhando tipos dos crates crates.io `bevy_flora`/`bevy_elements`
  — que são de **terceiros** (ver secção crates.io). Fundir no
  `bevy_ext_scene` antes de publicar, ou PR upstream; manter como crate
  próprio não se justifica.
- **`bevy_ext_gradmesh` (569)**: meio re-export (`pub use` do contrato do
  `bevy_ext_scene` e do `bevy_gradmesh`); existe para o Vael listar uma
  dependência só. Fundir no `bevy_ext_scene` (o render fica feature-gated).
  *Atenção ao mover: conferir o `type_path` reflectido dos structs — o
  `.bsn` e o BRP citam-no.*
- **`src/ui_bind.rs` (22)**: recurso `VaelHud` vazio de propósito, **sem
  leitores** (`hud.rs` usa `VaelHudConfig`/`UiScriptBinds`) — **removido**
  (F0, 2026-11-13).
- **`src/attach.rs` (54)**: tinha um consumidor real (`ui/waypoint.rs`) —
  **dobrado** nesse módulo (F0), sem mudança de comportamento.
- **`scripts/ui/journal.lua` (322)**: sem loader que o carregue (a UI do
  jogador já é `bevy_pf`) — **removido** (F0; analyze e lint verdes).

### 7. Aparato de transição nos manifests

- `bevy_extensions/Cargo.toml`: **9** members duplicados (26 entradas → 17
  únicas; dedupe aplicado no F0), exclude stale
  (`crates/bevy_voxel_terrain_jackdaw` — removido no F0), 6
  crates em `0.1.1` transitório com `TODO(viber-exit)`.
- Aliases de pacote replicados em 3 consumidores (`bevy_world_xml`,
  `bevy_engine_core`, `bevy_save_slots`) — morrem quando o consumo passar a
  ser do registry.
- `.worktrees/` no repo com **cópias integrais** — a auditoria do F0 manteve
  `exit2-hubs`/`exit2-scene` (commits únicos/WIP); na mesma tarde a onda
  exit2 mergeou-as e **removeu as duas** (~27 GB de targets libertados).
  Vivos: `exit2-integrate` (`integrate2/all`) e `exit2-luau2` (A1) — trabalho
  concorrente; não tocar.
- `.jdext`: **não estão versionados** (correcção ao levantamento inicial —
  o agente de exploração errou): 25 bundles, 433 MB locais, ignorados pelo
  `.gitignore` (vão para assets de Release). Higiene: retenção de 1 bundle
  por extensão (≈ 62 MB) — decisão local, sem git.

Tudo documentado e intencional — mas é estado processual codificado; a lista
acima é a checklist de limpeza quando as waves fecharem (F0/F1/F3).

### 8. Drift do patch `rerecast` — achado novo com risco real

- O `bevy_ext_viber_engine` tem `[patch.crates-io] rerecast = { path =
  "vendor/rerecast" }` — `0.5.0` com 2 fixes de robustez de navmesh
  (triangulação falhada abortava a navmesh; `vendor/rerecast/VIBER_PATCH.md`).
- `[patch]` **não atravessa path-deps**: quando o Vael (workspace próprio)
  compila o `viber` por path, o patch é ignorado. O lock do Vael resolve
  `rerecast 0.5.1` do registry; o do Viber, `0.5.0` vendored. Confirmado nos
  dois locks a 2026-11-13.
- A tabela do Vael foi removida no `29c270a` (exit) porque apontava a
  `../Viber/vendor/rerecast` — o que violava "Vael sem o repo Viber". O
  comentário no fim do `Cargo.toml` ficou e agora descreve um patch que não
  existe.
- **Correcção (F0):** repor a tabela apontando ao vendor que agora vive no
  dono do runtime — `../bevy_extensions/crates/bevy_ext_viber_engine/vendor/rerecast`.
  Mantém o Vael independente do checkout Viber e repõe a paridade de navmesh
  entre o jogo e a engine. *(A alternativa — aceitar a divergência e apagar
  o comentário — só vale se os fixes forem irrelevantes para o jogo; o
  `VIBER_PATCH.md` diz o contrário.)* Como muda comportamento de navegação,
  o F0 leva QA de criaturas a perseguir o herói, não só `cargo test`.
- **Aplicado a 2026-11-13:** a tabela foi reposta a apontar ao vendor do
  `bevy_extensions`, o lock resolve `rerecast 0.5.0` (path) e `cargo check` /
  `cargo test --lib` (332 testes) ficaram verdes; falta o QA vivo de
  criaturas (navmesh).

## Oportunidades crates.io (mesmo com trabalho)

**Ownership verificado na API do crates.io a 2026-11-13** — tudo o que o
jogo consome é de terceiros: `bevy_flora` 0.47 (Ladvien), `bevy_elements`
0.8 e `bevy_speech_bubbles` 0.3 (bevy-coven/Ladvien), `bevy_pf` 0.5.2
(edgarhsanchez), `bevy_gradmesh` 0.3.9 (xenacool), `bevy_gauge` 0.6
(DEMIURGE-studio), `bevy_rich_text3d` 0.8.1 (mintlu8). Os nossos
publicáveis: família `bevy_ext_*` 0.1.0 (publicação pendente de token —
dono: `PACKAGES.md`), `celer-*` (CelerAI) e `vramd` (PyPI). Consequência
prática: **integrar um wrapper num crate alheio é PR upstream; fundir
wrappers nossos é trabalho local barato.**

| # | Oportunidade | Custo | Notas |
|---|---|---|---|
| B1 | **Publicar a família `bevy_ext_*`** — pré-requisito de quase tudo o resto | pequeno (o trabalho é sequência/token, não código) | antes de publicar, fundir os 3 wrappers (achado 6); conferir `type_path` reflectidos; `cargo publish --dry-run --locked` por crate |
| B2 | **UI: uma stack no mesmo processo** — aposentar o `bevy_ext_ui` (16,5k de XML/CSS próprio) do grafo do jogo a favor do `bevy_pf` já adotado | grande; spike próprio, não primeira ronda | superfícies restantes da engine no jogo: barra de fala (`ui/say.lua`), profiler/showcase (dev tools). haalka/bevy_quill rejeitados — seria uma 3ª stack |
| B3 | **Save: um envelope só** — `bevy_ext_save` (slots, tmp+fsync+rename) como dono do envelope; `moonshine-save` fica como motor de reflexão por trás | médio | não re-litiga o `bevy_save` de terceiros (rejeitado, stale) — é a consolidação das **duas stacks nossas**; risco: formato dos saves dos jogadores |
| B4 | **`bevy_ext_postfx` encolhido** — Bevy 0.20 traz bloom/DoF/SSAO/AE/TAA nativos; o crate (6k) passa a orquestrar structs nativos + o que é mesmo autoral (split-tone, escalonador) | médio | −~3k LOC estimado; QA visual A/B; a lente frugal já cortou volumétrico/DOF/motion blur (`config.yaml`) |
| B5 | **`bevy_ext_ai` (2,2k FSM/sentidos)** — avaliar `big-brain`/avoidance do `bevy_landmass` | adiado | só quando a FSM Lua migrar (Fase 4b); avaliar antes é contra alvo móvel |
| B6 | **Upstream a terceiros** — submeter os 2 fixes do vendor `rerecast` e aposentar o patch local; acompanhar meshopt/KTX2 no bevy upstream para aposentar o decoder do `bevy_ext_gltf` (2,3k); PR dos settings-structs ao `bevy_flora`/`bevy_elements` (donos activos, atualizados em 2026-11) | pequeno por item | o valor é o patch que deixa de existir |

**Não re-litigar** (decisões vivas com dono): terreno próprio
([`TERRAIN-VOXEL-STUDY.md`](TERRAIN-VOXEL-STUDY.md)), `rand`/`noise`
proibidos (contrato "mesma seed, mesmo mundo"), envelope JSON do save do
`bevy_ext_save` (o `bevy_save` de terceiros é que foi rejeitado),
`bevy_water`/`bevy_sonus`, `bevy_tweening`/`thiserror` (CODE-SLIM),
Dexterous Developer.

## Plano faseado (para rondas futuras; donos citados, não duplicados)

- **F0 — aplicado a 2026-11-13** (código e doc): patch `rerecast` reposto a
  apontar ao vendor do `bevy_extensions` (lock `0.5.0` path; `cargo check` e
  `cargo test --lib` com 332 verdes); `ui_bind.rs` e `lighting.rs` apagados,
  `attach.rs` dobrado no `ui/waypoint.rs`, `journal.lua` removido (analyze e
  lint verdes); members duplicados (26→17) e exclude stale limpos;
  `bevy_ext_viber_engine` excluído do workspace para `cargo test`/`metadata`
  standalone; o wrapper `viber-engine` foi re-apontado ao crate (viber-tools
  `2c85281`). QA vivo de navmesh feito no mesmo dia: boot com `--entry dev`,
  bridge na 15706, **nav tiles #1/#2 prontos** (746 e 1344 obstáculos, 0
  erros no log) e `viber.vael` vivo. `.jdext`: retenção aplicada (436 MB →
  65 MB; 1 bundle por extensão). **Pendente:** comportamento de criaturas a
  perseguir sob carga limpa (o QA correu com build concorrente, load ~20).
- **F1 — crates.io** (dono: `PACKAGES.md`): fundir os 3 wrappers no
  `bevy_ext_scene`; publicar os 6; quando o Viber/viber-tools consumirem do
  registry, apagar aliases e versões transitórias.
- **F2 — virar gates default**: `VAEL_HARVEST_RUST`/`VAEL_TRAVEL_RUST`/
  `VAEL_QUEST_EVENTS_RUST` on por omissão com QA no pool atual; depois
  apagar os caminhos Lua correspondentes; o comparador do HUD e o shadow
  ganham end-date.
- **F3 — consumir em vez de copiar**: waves do plano de exit dos
  `bevy_extensions`, ordenadas pela divergência medida (`spawn` → postfx →
  nav → ui → roads); cada wave: o `viber_engine` passa a depender do crate
  e perde a cópia. **Actualização 2026-11-14** (onda exit2/exit3, dono
  [`EXIT-MAP.md`](EXIT-MAP.md)): a onda de unificação de gémeos e a
  composição própria da App já estão validadas nas branches
  `integrate2/all` + `exit2/app-assembly` (bloqueadas na água do monólito);
  as divergências à data: `spawn.rs` **633 lin** (monólito à frente com
  colliders **avian** + `gradmesh_style` + RoadSpec.flare), água **1.275**
  (`bevy_ext_water` já extraído, avian), postfx 349, nav 72, roads 4. A
  física do jogo é avian+tnua — as cópias extraídas da era Rapier
  (`bevy_ext_physics`, spawn_gate, voxel spawn) **portam-se a avian** antes
  dos swaps. O `viber.ecs` (2.1k) tem zero usos no jogo e morre sem port.
- **F4 — consolidações**: UI uma stack (B2), envelope de save único (B3),
  postfx encolhido (B4); 4b/4d/4e seguem a `MIGRATION.md`/`PACKAGES.md`.

## Riscos — o que não se toca

- **Contrato da bridge**: nomes/assinaturas `viber.vael` são usados por Lua
  vivo; a remoção é da camada inteira (4e), não de assinaturas avulsas.
- **Migração de saves** (`from_world_kv`/`merge_world_kv`) — semântica
  "nada se perde em legacy"; B3 não pode quebrá-la.
- **`shadow_tick`** — duplicação deliberada de paridade até ao F2 fechar.
- **`content/`** — esquemas congelados (gate do editor).
- **Sessões concorrentes**: o `29c270a` removeu o patch deliberadamente
  (independência do checkout Viber) — a correcção F0 preserva esse objectivo
  apontando ao irmão que é agora o dono. Validar sempre num HEAD limpo e
  commitar paths explícitos.

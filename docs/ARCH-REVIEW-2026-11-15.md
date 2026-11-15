# Estudo de arquitectura dos irmãos (2026-11-15) — tools repetidas e seams mortos

> Segundo levantamento de arquitectura, datado de 2026-11-15. O primeiro
> ([`ARCH-REVIEW-STUDY.md`](ARCH-REVIEW-STUDY.md), 2026-11-13) cobriu a
> relação jogo↔monólito e as oportunidades crates.io B1–B6 — **nada daqui
> repete esse material**; o que ele já mapeou é citado por referência e as
> suas ondas (F0 aplicado, F1–F4) continuam com o mesmo dono. Este estudo
> cobre o que aquele não olhou: **tools duplicadas** (CLI, extensões,
> validadores, pipelines Python do AiGameKit), **seams mortos** nos crates
> extraídos, e **CelerAI**. Contagens levam a data (main de 2026-11-15).
> Método: 3 passes de exploração sobre Vael, `bevy_extensions`,
> `AiGameKit`+`CelerAI` (+ `vramd` upstream), com verificação pontual
> byte-a-byte dos achados carregados.

## Sumário executivo

- **Fora das waves do monólito, o maior custo restante é de ferramentas
  duplicadas, não de código de jogo:** o cliente de bridge existe em 4
  cópias, o conteúdo tem 2 validadores em 2 linguagens, a indexação de
  mundo tem 2 implementações em 2 repos, e o `editor/` legado (4.6k) é uma
  cópia quase integral da extensão `vael_content_jackdaw` ainda dentro do
  workspace.
- **Nos crates extraídos, o overengineering novo está em seams para
  serviços que nunca foram ligados:** 2 traits mortos (`TerrainQuery`/
  `NavQuery`, **apagados hoje**), um caminho de loops de água dormindo com
  `TODO(viber-exit)`, gémeos byte-idênticos (528 lin) e 4 structs
  espelhados com `type_path` congelado à espera da wave G.
- **O AiGameKit tem ~30k linhas de esqueleto duplicado entre as suas 19
  tools** (`hardware.py` ×9, `worker_serve_adapter` ×11, `vramd_payload`
  ×8, `cli_rich.py` ×14, vendor Hunyuan copiado 3×) e **o protocolo vramd
  está implementado 2×** (upstream vs `Shared`, ~2.7k lin a mais). É
  consolidação de onda 3 — nada disto bloqueia o jogo.
- **crates.io: nada de grande novo.** O que o jogo consome já é de
  terceiros (verificado no estudo anterior); a oportunidade nova é o
  `Shared` do AiGameKit consumir o pacote PyPI do `vramd` em vez de
  reimplementar o cliente.
- **Onda 0 executada nesta passagem** (registo no fim): 2 traits mortos
  apagados, label stale do `menu_data.rs` sincronizado, 2 tools one-off
  apagadas e 2 tools vivas documentadas.

## Estado vs 2026-11-13 (uma parágrafo, o resto é dos donos)

O F0 do estudo anterior foi aplicado no dia; as waves do
[`DELETE-MONOLITH.md`](DELETE-MONOLITH.md)/[`EXIT-MAP.md`](EXIT-MAP.md)
fecharam A/B/E (+F2–F5+F8 do luau, D de terreno, G-1 music/ai) e têm C 🔄,
F 🔄 (F6/F7/F9), G-2 ⏳ (~35k da raiz + `recipes/` + `scene/` sem destino
desde que o hub `scene_runtime` foi apagado) e H ⏳ pendentes. O achado 1
do estudo anterior (fork-duplicação) mantém-se vivo: medido a 2026-11-15,
`graphics.rs` (285) e `draw_diag.rs` (243) estão **byte-idênticos** nos
dois lados (verificado com `cmp`), `worldsys` diverge 834 lin
(1.588 vs 2.422) e `menu_data.rs` divergia em 1 linha (ver abaixo). Tudo
isso é território das waves C/G — aqui fica só a evidência actualizada.

## Achados — Vael

### V1. `editor/` é a extensão `vael_content_jackdaw` duplicada (4.6k lin)

`diff -rq` entre `editor/src/` e `extensions/vael_content_jackdaw/src/`:
**7 ficheiros byte-idênticos** (verificado: `panels/{quests,dialogues,
items,progression,landmarks,lint}.rs`, `world_index.rs`), 7 com diffs
pequenos (2–85 lin) e só `{main,extension,lib}.rs` exclusivos. O
`editor/README.md` já declara "Legado: os painéis estão portados" — mas o
crate continua **workspace member** (`Cargo.toml:14`) e compila por
`-p vael-editor` para "testes sem janela". Cada bug fix no conteúdo tem de
ser feito nos dois sítios até isto morrer.

**Veredicto:** onda 1 — apagar o crate (a história fica no git); se algum
teste sem janela valer a pena, migra para `content/` ou para a extensão
antes de apagar.

### V2. Cliente de bridge reimplementado 4×

`cli/src/debug.rs` (432 lin) + `extensions/vael_{profiler,mcp,debugger}_jackdaw`
(235/210/153) carregam cada um o seu `BridgeClient`/`port()`/
`is_listening` sobre o mesmo protocolo (`bevy_ext_bridge_protocol`, que já
tem um cliente std-only). O CLI e as extensões falam com o mesmo jogo nas
mesmas portas.

**Veredicto:** onda 1 — as extensões passam a usar o cliente do
`bevy_ext_bridge_protocol` (ou um mini-crate `vael_bridge_client` se o
cliente do protocolo não cobrir os métodos `vael-tools/*`). Não urgente:
as 3 extensões são pequenas e estáveis.

### V3. Dois validadores do mesmo conteúdo

`content/src/validate.rs` (759, Rust) e `tools/lint_content.py` (207,
Python) validam as mesmas regras (items, progression requires/excludes com
simetria, quests, refs ao mundo). O Python chama o binário Rust
(`tools_bin()`/`analyze_world()`) e valida por cima o que falta: recipes e
locale pt/en. É **gate vivo**: `tools/check.sh` corre o `lint_content.py`
e `docs/TOOLS.md` documenta-o.

**Veredicto:** onda 1 — portar as regras únicas (recipes, locale) para o
`validate.rs` e reduzir o Python a wrapper fino (ou apagar, actualizando
`check.sh`). Não é "apagar já": o Python é o único que apanha recipes.

### V4. `cli/src/bsn.rs` (250 lin) vive para geradores XML-era

O comando existe "para os geradores (ex.: `tools/qa.py` ainda emite XML)"
num repo em que "os `.xml` foram aposentados a 2026-11-12". Não reimplementa
nada (delega em `bevy_ext_world::bsn`), mas é superfície de CLI morta
assim que o `qa.py` largar o XML.

**Veredicto:** morre na F2 (quando os gates Rust virarem default e o
caminho Lua/XML do QA sair). Registrado para ninguém o "descobrir" de novo.

### V5. Five profiler surfaces, uma necessidade

`scripts/ui/profiler.lua` (400) + `ui/profiler.css`, o profiler da engine
(P→FRAMES/F3), `vael_profiler_jackdaw` (235), `vael-tools debug prof` e
`scripts/ui/showcase.lua` (144) medem/presentam o mesmo runtime. O estudo
de perf (`vael-perf-bottlenecks`) já fixou "CLI-first". As 5 superfícies
não colidem funcionalmente (jogo vs editor vs CLI), mas o `.lua` de HUD
profiler é o candidato a aposentar quando a UI de dev migrar de vez para
`bevy_pf`/Rust.

**Veredicto:** decisão adiada — registado; não é onda própria.

### V6. Ferramentas python de `tools/` — 2 mortas, 2 vivas sem doc

Medido a 2026-11-15 (grep nos 4 checkouts + docs):
- `audit_terrain.py` (188) — lia `world.xml`, **aposentado a 2026-11-12**;
  zero referências. **Apagado nesta passagem** (história no git).
- `offset_anim.py` (109) — one-off do bake Y dos clips de natação (o GLB
  resultante vive no LFS); zero referências. **Apagado nesta passagem.**
- `make_map.py` (133) — gera `ui/vael_map.png` (hillshade do `.ahgt`).
  **Viva** — documentada no `AGENTS.md` (Mapa) nesta passagem.
- `make_menu_terrain.py` (146) — gera `scenes/menu.data/terrain.ahgt`.
  **Viva** — documentada no `AGENTS.md` (Mapa) nesta passagem.

### V7. Duplicações restantes de média gravidade (inventário)

- **Indexação de mundo ×2 em 2 repos:** `vael_scene_jackdaw/src/index.rs`
  (636, índice por reflexão do ECS via schema do jogo) vs
  `extensions/vael_content_jackdaw/src/world_index.rs` (parsing de
  `world.bsn` via `load_world_in`). Propósitos diferentes (cena vs
  conteúdo) mas a pergunta é a mesma ("que nós existem e onde").
- **`bsn_tidy.py` (200)** vs examples `bsn_organize`/`bsn_prefabify`
  (`bevy_extensions`): manipulação `.bsn` em dois repos; o tidy organiza
  pastas do Outliner, os examples reorganizam/prefabificam — candidata a
  fusão se o tidy sobreviver à vida do mundo estático.
- **Representação dupla dos passos de QA:** `qa.py::steps_for` gera passos
  lógicos `kill/collect/visit/turn_in` que o `simulate` de `src/qa.rs`
  (2.079) reinterpreta. Duas representações do mesmo plano de teste;
  consolidar quando o QA Rust ganhar o seu próprio gerador de passos.
- **`hanabi_fx.rs` (113):** módulo de protótipo atrás de feature
  `hanabi` default-off permanente desde sempre — decidir: feature fora
  (módulo morto) ou experimento com dono.
- **Marca `vael_*` partida por 2 repos:** 5 extensões no Vael (content,
  profiler, mcp, analyze, debugger) e 4 no `bevy_extensions` (scene, voxel
  terrain, vael_jackdaw, grass). Zero colisão de `type_path` (verificado),
  mas packaging/release e o boilerplate `.jackdaw/shim` (21 lin ×9)
  replicam-se. Movimento de repos — onda 3, decisão própria.

## Achados — bevy_extensions

### B1. Crates de 1 consumidor (o custo de extrair à frente do consumo, actualizado)

Dois meses depois do achado 2 do estudo anterior, o quadro por consumidor
real (grep em todos os Cargo.toml dos 4 checkouts):

| Crate | Consumidores | Situação |
|---|---|---|
| `bevy_ext_bridge_runtime` | só Vael | OK — consumidor real |
| `bevy_ext_luau` | só monólito | wave F em voo |
| `bevy_ext_nav`, `bevy_ext_audio`, `bevy_ext_ai`, `bevy_ext_ui` | só monólito | waves C/G/H |
| `bevy_ext_viber_engine` | só Vael | o monólito; morre na H |

O `bevy_ext_ui` (16.3k) é o maior: o Vael recusa-o explicitamente
(`Cargo.toml`, comentário) e o `EXIT-MAP` marca a apresentação (~18k) como
"morre (bevy_pf é o dono)". **Nada disto é novo** — é o achado 2 do estudo
anterior em execução; fica aqui só o mapa de quem consome o quê.

### B2. Gémeos byte-idênticos e cópia stale (evidência 2026-11-15)

- `bevy_ext_engine/src/{graphics.rs,draw_diag.rs}` ↔ cópias no monólito:
  **idênticos byte-a-byte** (verificado `cmp`) — 528 lin que a troca de
  shim mata sem port nenhum. Candidato barato para o dono da wave C/G.
- `bevy_ext_ui/src/ui/menu_data.rs` divergia do monólito em **1 linha**
  ("Rapier 3D" vs "Avian 3D" — a cópia do crate era a stale). **Corrigido
  nesta passagem** (o jogo usa a do monólito; o crate está sem consumidor
  até à troca atómica da wave C).
- Demais gémeos (sky, cinematic, worldsys, water_ssr, editor_camera,
  foliage, netplay, frame_pacing, decal_fx): ~8.9k lin vivos dos dois
  lados — território das waves, não repetido aqui.

### B3. Quatro structs espelhados com o mesmo `type_path` congelado

`"viber::worldsys"` vive em dois sítios com os mesmos campos:
`AtmosphereState` (`bevy_ext_postfx/src/env.rs:32` ↔
`bevy_ext_engine/src/worldsys.rs:901`), `WeatherState` (`:176` ↔ `:321`),
`SunState` (`:98` ↔ `:592`), `SunLightBase` (`:112` ↔ `:583`). Só
`BiomeRegions` tem dono único (reexport). A onda G já lista "UM dono para
AtmosphereState/BiomeRegions" — os outros três não estavam na lista
explícita; entram por aquela wave.

### B4. Seams de "chão" — 5 abstrações para a mesma pergunta, 3 mortas/dormindo

| Seam | Estado |
|---|---|
| `bevy_ext_core::TerrainQuery` / `NavQuery` | **mortos — zero impls/usos no ecossistema. Apagados nesta passagem** (+ README e referência em `ai/ground.rs`) |
| `bevy_ext_ai::GroundQuery` | 1 impl (monólito `ai_host.rs`) + testes; wave G decide |
| `bevy_ext_nav::NavGround` | 1 impl (monólito `nav/mod.rs`) |
| `bevy_ext_luau::SurfaceQueries` | só `FakeSurfaces` de teste; o monólito usa a sua própria `SurfaceRegistries` — seam não ligado (wave F decide: ligar ou cortar) |
| `bevy_ext_audio::WaterBodies` | **dormindo ligado**: `HostWaterBodies` é `init_resource` mas nunca publicado; os 3 sistemas de `ambience.rs` correm em no-op com `TODO(viber-exit)` — a repartição do `ambient.rs` (wave G: "SFX→audio") é quem decide ligar ou cortar. **Não apagado nesta passagem** por estar pendurado numa wave |
| `bevy_ext_water::WaterSource` | 1 impl real; os 4 plugins genéricos `<S: WaterSource>` existem para 1 fonte — monomorfizar quando a água estabilizar (pós-D) |

Moral registrada: o padrão "trait de serviço que não pode depender do
crate de domínio" só paga quando existe um segundo integrador; os 3
primeiros exemplos criados não chegaram lá.

### B5. Aparato duplicado de menor peso

- **Profiling triplicado, 2 cópias no-op:** `bevy_ext_physics/src/
  profiling.rs` (43) e `bevy_ext_terrain_runtime/src/profiling.rs` (29) são
  cópias do contrato `viber::profiler::{Group,timed}` com `timed` que faz
  `let _ = group; system` — o real vive no `bevy_ext_engine`
  (`profiler/timed.rs`, 1.090). Onda C/E.
- **Feature flags mortos:** `bevy_ext_luau` `ui` (o seam F2 nunca compila
  — nenhum consumidor passa features; wave F decide), `bevy_ext_engine`
  `dlss` (nunca ligada; **o gate no monólito está quebrado**: os call-sites
  `#[cfg(feature = "dlss")]` citam `crate::dlss::…` que não existe no
  monólito — se alguém ligasse a feature, E0433; e o `GameConfig.dlss` +
  chave yaml vivem no `config.rs` do engine) — remoção coordenada, onda
  C/G; `bevy_ext_physics` `headless` (só dev-dep do monólito).
- **Shims re-export no `terrain_runtime`** (~660 lin:
  `water_{buoyancy,fx,material,waves,mesh,paths}.rs`): pontes de migração,
  morrem nas waves D/pós-água.
- **Duas relvas:** `engine/src/grass.rs` (1.806, tufts baked) vs
  `bevy_ext_grass` (1.732, sobre `bevy_grass`) — a extraída é a que o jogo
  usa; a baked é cópia no monólito (wave G).
- **Dois envelopes de save:** já no estudo anterior (B3); nota nova: o
  `bevy_ext_save` (774 lin, o menor) **puxa o `bevy_ext_terrain_runtime`
  (29.4k, o 2.º maior)** como dep — inversão desses grafos é pré-requisito
  da consolidação do envelope.
- `expand_home` no `engine/config.rs:405` — o crate `dirs` já dá; enxugo
  de Code-Slim, não onda.

## Achados — AiGameKit (~224.7k py + 7.7k rs em 19 tools)

### A1. O esqueleto de tool copiado ~9–14× (o maior bloco de duplicação do ecossistema)

| Padrão | Cópias | Evidência |
|---|---|---|
| `hardware.py` (perfil hw: `detect_hardware_profile`/`profile_from_specs`/`hw_auto_enabled`) | **9** | Text2D, Texture2D, Paint3D, Skymap2D, Text3D, Rigging3D, Text2Sound, Part3D (87–181 lin cada) + `Shared/src/aigamekit_shared/hardware.py` (108); diffs = docstrings |
| `worker_serve_adapter.py` | **11** | 1.502 lin somadas sobre `Shared/…/worker_serve_adapter_base.py` (183); 8 com `class Adapter`+`def serve` idênticos em forma |
| `vramd_payload.py` | **8** | Text3D 131, Part3D 93, Text2D 85, Texture2D 77, Terrain3D 70, Skymap2D 58, Text2Sound 56, Paint3D 16 + `build_generate_request` repetido 6× |
| `cli_rich.py` (boilerplate click) | **14** | 8–12 lin, só header/footer muda |
| Funções literais | — | `merge_negative_prompt` byte-idêntica em `skymap2d/generator.py` e `texture2d/generator.py`; `_preflight_download`/`_load_pipeline` re-implementados em 5 generators; `default_model_id` ×3 |
| Vendor Hunyuan/Michelangelo | **3×** | `Text3D/src/text3d/hy3dshape/…`, `Rigging3D/src/rigging3d/skintokens/src/model/michelangelo/…`, variantes no Paint3D (`Transformer`, `ResidualAttentionBlock`, `QKVMultiheadAttention`, …) |

O `DiffusionGeneratorBase` (578, no `Shared`) é a prova de que o modelo
certo existe — 4 pipelines já o subclassificam; o resto do esqueleto ficou
fora. **Veredicto:** onda 3 — consolidar hardware/vramd_payload/adapters no
`aigamekit_shared` com a mesma abordagem (base + adapters finos). Ganho
estimado: ~4–6k lin menos e, principalmente, um só sítio para corrigir
bugs de VRAM/offload. Não é urgente: cada cópia tem testes próprios
(66.8k lin de testes, CI real) e as tools estão estáveis.

### A2. O protocolo vramd implementado 2× (e um cliente legacy em zumbi)

- Upstream `vramd/src/vramd/client.py` (965, PyPI) vs
  `Shared/src/aigamekit_shared/vramd_client.py` (1.558) — o mesmo contrato
  JSON/NDJSON re-implementado, com testes dos dois lados.
- Staging remoto também 2×: `Shared/…/{vramd_bundle.py 472, vramd_remote.py
  753}` vs upstream `{transfer,sessions,peers}.py` (2.244).
- Dentro do `vramd_client.py` sobrevive o **ModelServer legacy per-tool**
  (`_SOCKET_FOR_TOOL = {"text2d": "text2d-server.sock", …}`) com suíte de
  testes própria — protocolo que o vramd substituiu.

**Veredicto:** onda 3 — o `Shared` passa a consumir o pacote PyPI (é a
oportunidade crates.io/PyPI nova deste estudo); o legacy morre quando
nenhuma tool o usar. Não é urgente: o cliente local tem extras reais
(auto-start `ensure_vramd_running`, calibração) que o upstream não dá —
a fusão implica decidir onde esses extras vivem.

### A3. Orquestração do GameAssets: medido, não dívida — mas com camadas a vigiar

`paths.py` (786, máquina de estados por linha), `batch_cmd.py` (4.274),
`resume_cmd.py` (2.269), `vramd_batch.py` (1.271), `vramd_coord.py` (594),
`vramd_routing.py` (390), `manifest.py` (560, fingerprint), `profile.py`
(1.150), `dream/` (1.9k, planner LLM) + telemetria em camadas
(`perfstore/` SQLite 582, `profiler/` 695, `pipeline_trace.py` 140,
`step_cache.py` 84) e presets em 3 sítios (`Shared/presets.py` 74,
`GameAssets/presets.py`, `Text2Sound/presets.py` 455). Para um pipeline de
1 estação (o regen do jogo), é mais aparo do que o uso diário justifica —
**mas o regen foi o produto e funcionou** (125/125 assets). **Veredicto:**
não tocar enquanto as tools geram valor; se a geração entrar em
manutenção, é a primeira zona a simplificar (telemetria e presets são os
cortes mais baratos).

### A4. Infra de instalação

`tools.yaml` + `install.sh`/`install.ps1` (131/178) +
`Shared/…/installer/` (1.366) + `monorepo.py` para gerir 19 venvs; o Hi3D
exige venv py3.10 isolado (única exceção, `min/max_python [3,10]` +
`installer/hi3d_extras.py`). Custo real mas cada linha tem motivo
documentado (`aigamekit-per-tool-venvs`); sem ação.

## Achados — CelerAI

### C1. `lab/` duplica `tools/d1/` 1:1 — e está fora do git

`make_bench.py` 346=346 lin, `eval_onnx.py` 83=83, `prune_vocab.py`
214=214, `export_d1_onnx.py` 226 vs 211, `quantize_d1_onnx.py` 96 vs 84.
Como `lab/` não é versionado, **apagar seria irreversível — não tocado
nesta passagem**. **Veredicto:** decisão do dono do CelerAI: ou `lab/`
entra no git e a divergência passa a ser visível, ou os scripts vivos
voltam para `tools/d1/` e o `lab/` é removido.

### C2. `models/nlu-retail` = 97 MB de hardlinks para variar 1 JSON

Mesmos binários (link count 2, custo de disco real ~0) com só
`nlu_config.json` a diferir (1.310 vs 1.138 bytes). O custo é de clareza
(LFS + paths), não de espaço. **Veredicto:** substituir por um config
variante (`nlu_config.retail.json`) quando o release tooling for tocado;
não é urgente.

### C3. As 2 crates (2.082 lin) — fusão opcional, GEMM à mão é decisão

`celer-nlu` (1.520: encoder podado via ORT, embedding/heads int8 fora do
grafo, **GEMM SIMD AVX2/FMA escrita à mão**, formatos `VAELEMB1`/`VAELHEAD1`)
e `celer-decision` (562, reusa `celer_nlu::Embedding` e
`shared_environment` — sem duplicação entre elas). Fusão numa crate só é
possível mas os dois consumidores (`bevy_ext_luau`, monólito) citam-nas
separadamente; ganho pequeno. O GEMM à mão **não é dívida**: é decisão de
latência sobre ops int8 do ORT, documentada aqui para não ser re-auditada.

## Crates.io / PyPI — delta sobre B1–B6

O estudo anterior mapeou B1 (publicar `bevy_ext_*`), B2 (UI uma stack),
B3 (save um envelope), B4 (postfx encolhido), B5 (`big-brain` adiado),
B6 (upstream rerecast/meshopt/settings). **Nada disso muda.** Delta:

| # | Oportunidade | Custo | Nota |
|---|---|---|---|
| N1 | **`Shared` consome o `vramd` do PyPI** em vez de reimplementar o cliente | médio | os extras locais (auto-start, calibração) precisam de decidir onde vivem; ver A2 |
| N2 | **GEMM int8** — substituir a matemática à mão do `celer-nlu` pelas ops quantizadas do ORT | pequeno | só se a latência medida regredir; hoje é decisão, não dívida |
| N3 | PR upstream dos settings-structs de flora/elements | pequeno | já era B6; continua válido |

## Não re-litigar (lista do estudo anterior, com adições)

Mantida integralmente do `ARCH-REVIEW-STUDY.md`: terreno próprio,
`rand`/`noise` proibidos (golden tests), envelope JSON do save,
`bevy_water`/`bevy_sonus`, `bevy_tweening`/`thiserror`, Dexterous
Developer. Adições deste levantamento:

- **GEMM int8 à mão do `celer-nlu`** (decisão de latência, ver N2).
- **A máquina de estados do `GameAssets`** (ver A3 — simplificar só em
  manutenção).
- **`bevy_pf` como dono da UI** — a dupla stack (achado B2 do estudo
  anterior) resolve-se matando o `bevy_ext_ui`, não trazendo uma terceira.

## Plano priorizado

- **Onda 0 — aplicada nesta passagem** (ver registo).
- **Onda 1 — barata a média, local, sem tocar nas branches em voo:**
  V1 (editor/ fora do workspace), V2 (cliente de bridge fundido), V3
  (validador único: portar recipes/locale para `validate.rs`, `check.sh`
  actualizado), decisão V5/V7 (hanabi, bsn_tidy). Cada item isolado com
  gates próprios (`tools/check.sh`, `cargo test --lib`, analyze/lint).
- **Onda 2 — é o plano existente, não duplicar:** gémeos + structs
  espelhados + profiling no-op + dlss (C/G), feature `ui` do luau (F),
  `bevy_ext_ui` morre (H), shims do `terrain_runtime` (pós-água), B1–B4
  do estudo anterior com os donos de sempre. F2/QA mata o `cli/src/bsn.rs`
  (V4).
- **Onda 3 — estratégica, dono próprio:** consolidação do `aigamekit_shared`
  (A1), cliente vramd único (A2/N1), extensões num repo (V7), `lab/` do
  CelerAI (C1), B2/B3/B4 se as waves libertarem espaço.

## Riscos / notas de execução

- Sessões concorrentes nas waves D/F/G/H do monólito: tudo o que tocava
  ficheiros dessas waves foi **saltado e registado** (ver B4/B5), não
  executado.
- A deleção do `WaterBodies` foi abortada pela verificação de grep: os
  sistemas de `ambience.rs` estão ligados ao recurso com `TODO(viber-exit)`
  — é alvo da wave G, não lixo.
- A deleção do `dlss` foi abortada pela mesma verificação: o `GameConfig.dlss`
  vive no `config.rs` do engine e os call-sites do monólito estão em
  `postfx.rs`/`bridge/lua.rs` (wave C) — remoção coordenada, não 3 linhas.

## Registo do executado nesta passagem (onda 0, 2026-11-15)

- `bevy_ext_core`: `TerrainQuery`/`NavQuery` apagados (`src/lib.rs`,
  `README.md`) + referência em `bevy_ext_ai/src/ground.rs` limpa.
- `bevy_ext_ui`: `menu_data.rs:44` "Rapier 3D"→"Avian 3D" (cópia stale
  sincronizada com o monólito).
- Vael: `tools/audit_terrain.py` (lia `world.xml` aposentado) e
  `tools/offset_anim.py` (bake one-off) apagados; `tools/make_map.py` e
  `tools/make_menu_terrain.py` documentados no `AGENTS.md` (Mapa).
- Este doc + entrada no `docs/README.md`.
- Gates: workspace `bevy_extensions` verde (`cargo check --workspace`;
  core 23 + ui 165 testes); Vael `vael-tools analyze`/`lint` exit 0;
  `cargo test --lib` 377–380 verdes com as falhas de `quest`/`state_sync`
  a passar isoladas — a suite completa flakeia em timing
  (`state_sync::tests::gate_corrige…`) sob build concorrente, pré-existente
  e fora do âmbito desta onda.

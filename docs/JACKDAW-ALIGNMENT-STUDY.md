# Estudo: alinhamento do Vael com os padrões do Jackdaw

> **Estudo de 2026-11-15.** O inventário e o backlog (P1–P4) são a referência
> para as rondas seguintes; a matriz "onde se autora" vive no
> [`JACKDAW.md`](JACKDAW.md) e o contrato no [`../AGENTS.md`](../AGENTS.md).
> Os quick wins P0 foram aplicados no próprio dia (§2).

Data: 2026-11-15. Âmbito: comparar o que o projeto ainda autora fora do editor com
as convenções e features da plataforma Jackdaw 0.20 instalada (SDK 0.20.2,
rustc 1.99.0), para decidir onde nos adequamos e onde o desvio é consciente.

Método: três explorações paralelas (a plataforma, a autoria fora do editor,
as extensões `.jdext`) + verificação directa por grep/leitura de todas as
contagens citadas (datadas). As capacidades do SDK foram lidas no source
instalado (`~/.cargo/registry/src/*/jackdaw_api-0.20.2/src/lib.rs`,
`jackdaw_editor_core-0.20.2`, `jackdaw_feathers-0.20.2`).

## 1. Baseline: o que já segue o padrão

- **O mundo é um documento de cena.** `world.bsn` e os 229 módulos
  `world/**/*.bsn` são gravações do `scene.save` do editor: `IsA { source }`
  para os 158 prefabs, `SceneNodeId`/`PrefabEntityId` em todo o lado
  (migração `bsn_migrate --node-ids` de 2026-11-14), pastas transparentes e
  células de 128 m. Zero atributos DSL no raiz.
- **Componentes tipados** (`ColliderSpec`/`BodySpec`/`Destructible`/
  `Harvestable`/`ModelLod`/`FloraStation`/`ScriptRef`/`ParticleSpec`/…)
  com inspector categorizado e gizmos — o padrão `AttrSpec` de
  [`JACKDAW.md`](JACKDAW.md).
- **Cenas de sala e menu autoradas no editor** (`scenes/menu.bsn`,
  `scenes/interiors/*.bsn`) com `CameraSpec`/`InteriorSceneSpec`/
  `AudioMixerSpec`/`MusicLayerSpec`/`BiomeRegionSpec` + painéis e gizmos.
- **Conteúdo principal com painéis**: quests, diálogos, itens, marcos e
  progressão editam-se na extensão `vael_content_jackdaw`.
- **PIE/live-edit/QA integrados**: `SceneNodeId` no Play, re-spawn por nó
  (`src/live_edit.rs`), `[[run]]` para Play/Play (título)/Play (QA),
  schema por reflexão (`--jackdaw-extract-schema`).
- **Higiene de bundles**: os `.jdext` já estão cobertos por
  `bevy_extensions/.gitignore:5` (`extensions/**/*.jdext`) — a auditoria
  inicial que os dava por commitados era um falso positivo.

## 2. Quick wins aplicados a 2026-11-15

| Item | Acção |
|---|---|
| `dialogues/README.md` apontava à app legada (`cargo run -p vael-editor`) e citava o `<DialogueNPC>` morto | Ponteiro trocado para o painel **Vael: Diálogos** da extensão no Jackdaw; `<DialogueNPC>` removido (o id vem de `prop-dialogue_id`) |
| `data/nlu_quest_index.json` órfão no Vael (o builder vive no CelerAI e esperava `CelerAI/quests`, inexistente desde o split) | Movido para `CelerAI/data/`; `CelerAI/tools/nlu/build_quest_index.py` repontado para o irmão `../Vael` e `--model` desacoplado do gate do parley |
| `ui-showcase.bsn` + `ui-showcase.data/` sem consumidores | Apagados (`git rm`); bala removida do `README.md` |

Efeito lateral descoberto ao correr o validador: **drift de conteúdo
pré-existente** — o pipeline NLU/parley cobre 28 quests e há 9 sem bloco
`parley` (`city_forge`, `forest_nightwatch`, `desert_shields`,
`peaks_brutes`, `swamp_hexers`, `story_echoes`, `story_glass_envoy`,
`swamp_drowned_bell`, `forest_glass_witness`). Não é um problema de editor;
fica registado aqui e pertence à frente de conteúdo (P2) ou a uma ronda de
quests — ligá-las é uma decisão de design de jogo, não de ferramenta.

## 3. Desvios encontrados

### A. Conteúdo fora do editor

Oito superfícies de dados editadas à mão, validadas só por CLI
(`tools/lint_content.py` / `tools/check.sh`), fora do painel *Conteúdo* da
extensão:

| Dados | Lê em runtime | Schema em `vael-content`? |
|---|---|---|
| `data/recipes.json` | `scripts/anvil.lua`, `scripts/game/loot.lua`, Rust (`economy.rs`) | sim |
| `data/riddles.json` | `scripts/lib/riddle.lua`, `crystal-shrine.lua`, Rust | sim |
| `data/verdicts.json` | `scripts/lib/judge.lua`, Rust | sim |
| `data/boats.json` | só Rust (`src/boat.rs`) | não |
| `data/quest_puzzles/*.json` (3) | `scripts/npc/decision-quest.lua`, Rust | não |
| `data/nlu_aliases.json` | `scripts/lib/parley.lua`, `decide.lua`, `actions.lua` | não |
| `locale/*.json` (en/pt/strings_en) | `scripts/lib/i18n.lua` | só validação CLI (`tools/i18n_extract.py`) |

### B. Formato do mundo incompleto

Medido a 2026-11-15:

- **103 atributos string `key: "collider"`** em 16 ficheiros (15 módulos
  `world/**` + `scenes/menu.bsn`) — o `ColliderSpec` tipado existe e o loader
  aceita ambos, mas estes nós ainda se editam como texto cru.
- **11 nós de recurso** (`key: "kind"` + `key: "yield"`) em 6 ficheiros,
  incluindo os 3 prefabs `world/prefabs/misc/resourcenode{,-v2,-v3}.bsn` que
  divergem entre si — candidata à migração para `Harvestable`.
- **Tags de composição sem superfície tipada** — editáveis só como atributos
  crus no painel `WorldNode` (sem inspector nem gizmo): `Scatter` ×57,
  `Road` ×28, `DynamicSpawner` ×25, `Prototype` ×21, `Vegetation` ×9,
  `ProjectileTemplate` ×1. Nota: `Scatter`/`Vegetation` são receitas
  bake-time (o mundo é estático; o runtime nem as corre) — o valor do spec
  tipado aqui é visibilidade no editor + lint, não runtime.
- **`world/settings/*.bsn` sem `SceneNodeId`** (os 4 ficheiros) — os painéis
  *Ambiente/PostFX/Flora/Relva* gravam por IO próprio da extensão, num canal
  paralelo ao `scene.save` (sem undo por gravação, sem ids de nó).

### C. SDK do editor subaproveitado (extensões)

Lido no SDK instalado 0.20.2; nenhuma das 9 extensões usa:

- **Keymaps/atalhos** (`DefaultKeymap`, `KeymapPreset`, `PresetInput`) —
  zero atalhos registados; tudo é botão de painel, mesmo Esc do pincel é
  lido à mão.
- **`jackdaw_feathers`**: 34/39 módulos por usar (só a content usa 5) —
  `toast`/`alert` para problemas (hoje é Text cinzeto), `file_browser` para o
  painel *Cenas* (hoje lista ficheiros à mão), `list_view`/`tree_view` para
  os *Assets* (hoje botões de texto), `number_input`, `tooltip`,
  `options_bar`, `inspector_card`…
- **Thumbnails nativos** do editor (`jackdaw_editor_core` fotografa e cacheia
  em `.jackdaw/thumbnails/`) — o painel *Assets* não as mostra. Verificar a
  exposição da API às extensões.
- **`AssetKind`/`AssetKinds`** (catálogo do editor) — a categorização de
  modelos é uma heurística de needles de URL **duplicada** em dois sítios
  (`vael_scene_jackdaw/src/placement.rs::Kind::of` e `index.rs::icon_of`).
- **`document::write_atomic`** do SDK — reimplementado na content
  (`extensions/vael_content_jackdaw/src/io.rs`, acrescenta `.bak`).
- **`SceneOpened`/`SceneDocument`** — nenhum painel reage a cena
  aberta/trocada; a content faz *polling* no `Update` para adivinhar a cena.
- **`pie::PlayState`** — as extensões não sabem quando o Play corre; os
  painéis de runtime (Debugger/Profiler/Cinemáticas) não podem avisar "não
  estás no Play".
- **Operadores nativos por usar**: `entity.place_gltf` (o `place_at` só
  duplica instâncias existentes — *bloqueado*: o loader stock do editor não
  lê meshopt/KTX2, ver D), `viewport.screenshot`/`stats`/`bookmark.*`,
  `transform.nudge_*`/`snap.*`, `scene.save_all`/`save_as`,
  `history.undo`/`redo` (o undo entra só passivamente via
  `creates_history_entry`).
- **Toolbar de tools modais + options bar** (`ToolbarButton`, `ModalInputs`,
  `BrushSystems`) — o pincel de placement faz o seu próprio modo Place/Brush
  com estado em componentes marker, porque os botões do Jackdaw não passam
  parâmetros ao operador.

### D. Débitos estruturais (restrições da plataforma, não a nossa culpa)

- **Validação não veta gravação**: `SceneSaved` chega depois do ficheiro
  escrito (`vael_scene_jackdaw/src/validate.rs`); o "validar e gravar" é um
  operador manual. Precisa de um hook pré-save no editor — acompanhar
  releases.
- **Loader stock do editor sem meshopt/KTX2**: os GLB do pool falham no log
  (esperado); preview real via cache descodificada
  `assets/public/.jackdaw-cache/`. É o que impede `entity.place_gltf` e
  thumbnails de modelos novos.
- **Bridge síncrona no clique** bloqueia a thread do editor no painel
  Cinemáticas (`bridge.rs`); o Profiler já o resolveu com thread própria.
- **Espelho de câmara por layout ABI** (`vael_jackdaw/src/camera.rs`) —
  copia `JackdawCameraSettings` campo a campo e desliga-se sozinho se o
  layout divergir; frágil mas seguro.
- **`PropertyValue` só `&'static str`** e **botões sem parâmetros** —
  contornados (`.param` String, markers); exigem mudança no SDK.
- **0.20 sem overrides de instância** para nós de prefab sem id — dita o
  fluxo "abrir o módulo no separador durante o Play".

## 4. Backlog priorizado

### P1 — Migrações do `.bsn` (mecânico, baixo risco, gates prontos)

1. `bsn_migrate` para os **103 `collider`** (16 ficheiros) e os **11 nós
   kind/yield** (6 ficheiros) → `ColliderSpec`/`Harvestable`. Gate:
   `bsn_equiv` (tem de dar "equivalentes") + `bsn_origin_equiv`; binário de
   release posterior à migração.
2. Unificar os 3 prefabs `resourcenode*` divergentes (a variante simples
   fica com o nome simples; as outras levam o que acrescentam — convenção do
   `AGENTS.md`).
3. Specs tipados para as tags de composição, pela receita `AttrSpec`
   (`ScatterSpec`, `RoadSpec`, `SpawnExclusionSpec` já existe como modelo,
   `DynamicSpawnerSpec`, `PrototypeSpec`, `VegetationSpec`,
   `ProjectileTemplateSpec`) + categoria no inspector + gizmos onde faz
   sentido (Road/SpawnExclusion são o exemplo). O lift é **por tag**;
   `Scatter`/`Vegetation` são bake-time — paridade pelo round-trip, não por
   runtime. Esforço M por tag; começar por `DynamicSpawner` (o único que
   corre em runtime) e `Road`.
4. Decisão `world/settings/*.bsn`: migrar os painéis para
   `scene.open`+`component.set`+`scene.save` (ganha undo e ids) ou manter o
   canal próprio documentado. Custado: os 4 painéis têm de reencaminhar
   escritas por operadores. Recomendação: adiar até doer (P3 se avançar).

### P2 — Painéis de conteúdo em falta (`vael_content_jackdaw`)

1. **Mínimo útil**: schemas serde no `vael-content` para `boats`,
   `quest_puzzles`, `nlu_aliases` + cobertura de validação no painel
   *Conteúdo* (hoje `lint_content.py` valida sozinho fora do editor).
   Esforço S.
2. Painéis de edição onde a forma compensa: `riddles`/`verdicts`
   (estruturas pequenas e regulares), `nlu_aliases` (tabelas de alias).
   `boats.json` (7 linhas) fica à mão. Esforço M por painel.
3. `locale/*.json`: painel de validação de chaves em falta (substitui o gate
   `i18n_extract.py` no editor); edição de traduções só se houver uso real.
4. Decidir o destino das 9 quests sem `parley` (§2) — wiring NLU ou saída
   consciente do fluxo.

### P3 — Alinhamento SDK das extensões

Ordenado por esforço/benefício:

1. **Keymaps** para os operadores frequentes (gizmos, próximo problema,
   validar e gravar, play/pause do debugger). Esforço S, benefício diário.
2. **`toast`/`alert` feathers** nos painéis que hoje reportam em Text
   cinzeto (Problems, Cena, MCP). Esforço S.
3. **`scene.external_change`/`SceneOpened`** na content em vez do polling.
   Esforço S.
4. **`AssetKind`** do SDK no lugar das heurísticas duplicadas do placement.
   Esforço S.
5. **Cinemáticas em thread de background** (padrão do Profiler). Esforço S.
6. **`document::write_atomic`** do SDK na content (decidir o `.bak`: ou o
   SDK ganha opção, ou fica local documentado). Esforço S.
7. **`pie::PlayState`** nos painéis de runtime (estado "não estás no Play").
   Esforço S.
8. **`file_browser`/`list_view` feathers** nos painéis Cenas/Assets.
   Esforço M.
9. **Thumbnails** no painel Assets (depende da exposição da API — verificar
   no SDK primeiro). Esforço M, potencialmente bloqueado.
10. **Toolbar modal + options bar** para o pincel de placement (substitui
    markers). Esforço M.
11. `vael.mcp.add_group` aceitar o nome por parâmetro (hoje hardcoded
    `"MCP"`). Esforço XS.

### P4 — Prefabs nativos do editor (estudo de decisão, sem código nesta ronda)

O editor tem um sistema de prefabs completo (`prefab.pack`,
`prefab.spawn_instance`, variantes, overrides, `prefab.revert_*`,
`prefab.bulk_apply_in_scene`) que o Vael **não usa**: os seus prefabs são
módulos `IsA` + identidade `WorldOrigin { file, path }`, que é a chave dos
saves (`harvest::stable_key`) e do world partition por células.

Trocar para os prefabs nativos mudaria a identidade das instâncias (migração
de saves), o loader e o live-edit — e o 0.20 nem envia overrides de instância
para nós sem id. **Recomendação: manter `IsA`+`WorldOrigin`**; reavaliar
quando o Jackdaw suportar overrides de instância em nós de módulo. O que
cabe agora: usar operadores nativos onde são compatíveis (nada garantido —
`prefab.*` emite o formato dele, não o nosso) e documentar esta decisão como
consciente.

## 5. O que NÃO é desvio (decisões conscientes, não tocar)

- **Lua ~13k linhas/82 ficheiros à mão** — por desenho
  ([`JACKDAW.md`](JACKDAW.md) matriz: "não autorável"); a superfície de
  editor são os `--@export` (props no inspector) e o hot-reload. A migração
  Lua→Rust é o `docs/MIGRATION.md`, não este estudo.
- **Cenas QA geradas por CLI** (`tools/qa.py`) — pipeline de QA para agentes
  (CLI-first); emite XML e converte com `bsn_migrate`, mas o formato final é
  o mesmo e o consumidor é a bridge, não o editor.
- **Pipelines de assets em Python** (`make_lods.py`, `make_map.py`,
  `make_menu_terrain.py`, `regen_audio.py`) — geração, não autoria.
- **`bsn_tidy.py`/`flora_migrate.py`** escrevem `.bsn` fora do editor —
  mass-edits de manutenção com gates próprios; o equivalente manual no
  Outliner não escala.
- **`editor/` legado** (≈4.6k linhas, member do workspace) — morte já
  declarada no `AGENTS.md`/`docs/MIGRATION.md`; a remoção do member é item
  do plano de limpeza, não de alinhamento.

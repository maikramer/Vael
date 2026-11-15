# AGENTS.md — Vael

Jogo. A engine vive em `../bevy_extensions/crates/bevy_ext_viber_engine`
(o checkout `../Viber` foi aposentado a 2026-11-13). O editor oficial é o **Jackdaw
0.20 instalado**: `jd open .` abre este projeto; as extensões do Vael entram
como `.jdext` assinados. O crate `editor/` é legado (já não é o fluxo).
As ferramentas de geração são o checkout irmão `../AiGameKit` (CLIs no PATH).
Os modelos de IA (NLU e decisão) e os seus runtimes são o checkout irmão
`../CelerAI`. Este ficheiro é curto de propósito. O contrato da engine está em
`../bevy_extensions/AGENTS.md`; lê a secção que a tarefa pede, não o ficheiro inteiro.

## Irmãos

| Checkout | Quando |
|----------|--------|
| `.` (Vael) | Mundo, scripts, quests, cenas, pool `assets/` e o CLI `vael-tools` (`cli/`) |
| `../bevy_extensions/crates/bevy_ext_viber_engine` | Bug ou API em falta na engine. `cargo test` lá (excluído do workspace; corre standalone). O runtime de gameplay ainda é o monólito |
| `editor/` | **Legado**: editor próprio sobre Jackdaw. O fluxo é o Jackdaw oficial (`jd open .`) + extensões `.jdext` |
| `../bevy_extensions` | Crates `bevy_ext_*`/`bevy_voxel_terrain`, o bridge (`bevy_ext_bridge_protocol`/`_runtime`) e extensões genéricas em `extensions/` |
| `../AiGameKit` | Gerar ou corrigir uma tool. Não abras o monorepo no workspace por defeito |
| `../CelerAI` | Modelos `models/{nlu,d1}` (LFS) e crates de inferência. Sem ele a engine não compila nem os scripts carregam IA |

Workspace do dia-a-dia: `Vael.code-workspace` (esta pasta +
`../bevy_extensions` + `../CelerAI`).

## Crate Rust (migração)

Este checkout é o worktree da migração para Rust/Bevy (`refactor/bevy-native`).
O jogo arranca pelo crate `vael`:

```bash
cargo run --release -- \
  world.bsn --scene main --entry dev --bridge 15706
```

`gameplay:false` desliga IA/vitais/projéteis da engine; `player_camera:false`
dá o corpo e a câmara ao Vael (`src/player.rs`/`src/camera.rs`; a engine ainda
spawna o `PlayerGLTF` e o collider). O `viber.game()` ainda é a fonte de
verdade; `VaelStateSync` espelha-o e o `hud.rs` publica os binds do HUD. O estado, gates
e dívidas estão em [`docs/MIGRATION.md`](docs/MIGRATION.md). O checkout
principal pode ter trabalho em curso (regen/assets); não o tocar por arrasto.

## Só estático

`world.bsn` (árvore `<world static="true">`) + `world/**/*.bsn` +
`world.data/` são o mundo. **O `.bsn` é a única fonte**: os `.xml` foram
aposentados a 2026-11-12 (estão na história do git; não regenerar com
`vael-tools bsn convert`). As poses do `.bsn` são finais. O runtime não assenta
props nem corre `<StaticSpawner>` / `place` (erro de parse). Única geração em
runtime: `<DynamicSpawner>` (criaturas).

Editar no **Jackdaw oficial** (`jd open .`). Estrutura do `world.bsn`:
- **pastas** do Outliner (Definições, Mundo base, Cidades, Vegetação, …) são
  entidades sem `WorldNode`: o loader trata-as como transparentes (os filhos
  sobem). `tools/bsn_tidy.py` reorganiza um mundo achatado; verificar com
  `cargo run -p bevy_ext_world --example bsn_equiv -- antes.bsn depois.bsn`
  (no `../bevy_extensions`);
- **módulos** (`world/**/*.bsn`) entram como prefabs `IsA { source }` com um
  `WorldNode` `Include`; o editor expande-os e grava overrides no `world.bsn`;
- **definições** (`world/settings/*.bsn`): `ElementsSettings` (céu/clima),
  `PostFxSettings` e `FloraSettings` + uma `FloraSpecies` por espécie. São
  componentes `Reflect` (crates `bevy_ext_{elements,postfx,flora}`) que o
  jogo lê em `src/settings.rs` e relê ao gravar (o jogo a correr segue).
- **componentes tipados** (`bevy_ext_scene`): `ColliderSpec`, `BodySpec`,
  `Destructible` e `Harvestable` substituem as strings `collider=`/
  `rigidbody=`/`destructible=`/`resource-kind/yield` do DSL (3.7k nós
  migrados a 2026-11-13). O loader devolve-os ao parser do Viber como
  atributos; o componente ganha a um atributo homónimo. Migrar mais ficheiros:
  `cargo run -p bevy_ext_world --example bsn_migrate -- <ficheiros.bsn>` (só
  move o que volta igual; idempotente) + `bsn_equiv` contra a cópia de antes;
  a 2026-11-13 juntaram-se `ModelLod` (lod1/lod2/limiares), `FloraStation`
  (`prop-flora`), `ScriptRef` (`script`), `AuthoredProps` (`prop-*`),
  `ParticleSpec`, `SpawnExclusionSpec` e `PortalSpec` (+4.5k nós); a
  2026-11-14 `CameraSpec` (`<OrbitCamera>`), `InteriorSceneSpec` (as 9 salas),
  `AudioMixerSpec`/`MusicLayerSpec`/`BiomeRegionSpec` (`environment.bsn` e
  `menu.bsn`). O jogo lê colheita/destrutível tipados
  (`harvest.rs::from_specs`, paridade testada).
  As `prop-*` vivem no `CustomProperties` **do Jackdaw** (painel nativo do
  inspector; 2026-11-14, `bsn_migrate --props`, 141 nós); o loader ainda lê
  `AuthoredProps` antigos e, numa chave repetida, ganha o do Jackdaw;
- `assets/public/world` é um **symlink** para `world/`: o Jackdaw grava os
  `IsA.source` relativos à pasta de assets (`jackdaw.toml assets_dir`); sem o
  link o `scene.save` recusa o `world.bsn` ("absolute path"). Gravar no editor
  acrescenta `SceneNodeId`/`PrefabEntityId` (equivalente para o loader);
  Precedência: cena > `config.yaml`/constantes; `VAEL_START_*`, `VIBER_NO_*`
  e `VAEL_FLORA_*` ganham (QA). `vael --export-settings [--force]` reescreve
  os ficheiros com os valores de origem.
- **terreno voxel como componentes** (`bevy_ext_terrain`): a âncora
  `VoxelTerrain` no `<Terrain>` do `world.bsn` e as features — `<Cave>`
  (salas/chaminés dentro do componente), `<Arch>` e `<Cliff>` — são
  componentes; o loader devolve-os à vista DSL e o runtime do Viber não sabe
  a diferença. O editor Jackdaw cria-as assim (painel *Voxel Terrain*,
  incl. cliff); as features filhas da âncora **sobem a irmãs** na vista
  (o parser trata features de solo como folhas e ignora filhos do
  `<Terrain>`). Os parâmetros de render (LOD, splat, `layers`) continuam no
  nó DSL `<Terrain>`. Migrar mais ficheiros:
  `cargo run -p bevy_ext_world --features terrain --example bsn_migrate --
  --terrain <ficheiros.bsn>` (só move o que volta igual por valor;
  idempotente) e o gate é `cargo run -p bevy_ext_world --features terrain
  --example bsn_equiv -- <antes.bsn> <depois.bsn>` (features comparam
  ausente ⇔ default do spec, bools por valor e `width`/`span` do arco são o
  mesmo attr). O binário de release tem de ser posterior à migração — um
  binário velho não conhece os componentes.

Hand-editar uma pose à espera que a engine a ajuste não funciona.
O mundo `.bsn` é estático: o runtime não assenta props (`settle`/`scatter` de
CLI eram XML-only e morreram com o `viber`). No editor, o painel **Vael:
Assets** assenta o que colocas no `.ahgt` e o painel **Flora** mexe nas
estações; a vegetação `.vinst` não entra no undo — git ou re-scatter.

`world.data/terrain.ahgt` e `.paint` são estado do mundo. Commita-os.

## Mapa

| Caminho | O que é |
|---------|---------|
| `world.bsn` | raiz estática e os módulos `<Include>` (`world/**/*.bsn`) |
| `world/**/*.bsn` | cidades, criaturas, landmarks, `spawn/…`, HUD (módulos; prefabs `IsA` no `world.bsn`) |
| `world/settings/*.bsn` | definições editáveis no Jackdaw: ambiente, pós-processamento, espécies de flora (`src/settings.rs`) |
| `tools/bsn_tidy.py` | organiza os filhos de topo de um mundo `.bsn` em pastas do Outliner (idempotente) |
| `tools/make_map.py` | gera `ui/vael_map.png` (hillshade reproduzível do `world.data/terrain.ahgt`, sem GPU) |
| `tools/make_menu_terrain.py` | gera o terreno da cena de menu (`scenes/menu.data/terrain.ahgt`) |
| `scenes/menu.bsn` | cena de arranque: acampamento pequeno atrás do título |
| `scenes/interiors/` | salas `.bsn` (cenas próprias, sem terreno) |
| `quests/*.json` | 37 quests + `index.json`; lidas por `scripts/lib/rules.lua` (reiniciar o jogo; sem `cargo build`) |
| `dialogues/*.json` | grafos ambientais; lidos por `scripts/townsfolk.lua` |
| `data/*.json` | itens/loja, marcos, progressão, enigmas (`riddles.json`) e regras do juiz (`verdicts.json`); lidos por `rules.lua`/`shop.lua`/`landmarks.lua`/`riddle.lua`/`judge.lua` |
| `editor/` + `content/` | **Legado**: App própria `vael-editor` (testes/painéis antigos) e esquemas `vael-content`. O fluxo é o Jackdaw oficial (`jd open .`; `jackdaw.toml` fixa `package = "vael"`/`plugin = "VaelPlugin"`) com as extensões `.jdext` em `../bevy_extensions/extensions/` e `extensions/`; os painéis de conteúdo vivem em `vael_content_jackdaw` |
| `scripts/**.lua` | comportamento. Hot-reload ao gravar |
| `assets/public/nlu/` | **symlink** para `../CelerAI/models/nlu` (criado por `tools/fetch_models.sh`). `viber.nlu` na engine; relato de 1 palavra no fecho das quests. `assets/public/d1` idem para `models/d1` (`viber.decision`) |
| `ui/*.xaml` | HUD, menu, título, morte e mapa (`bevy_pf`); os `*.css` que restam são legado de HUD/profiler/showcase |
| `assets/` | pool. `game.yaml` + `manifests/`. Binários em `assets/public/` |
| `docs/README.md` | índice da documentação: o que é vivo, o que é histórico e as convenções de manutenção |
| `docs/AUDIO.md` | direção sonora (Coro de Vael), layers por bioma, manifests e gate de costura; regen com `assets/regen_audio.py` |
| `docs/FLORA.md` | árvores procedurais (`bevy_flora`): estações `prop-flora` no `.bsn`, pools por espécie, colheita provisória e `tools/flora_migrate.py` |
| `config.yaml` | `assets.roots: assets/public` |
| `src/environment.rs` | **dono do céu/tempo** (`bevy_elements`): relógio astronómico, sol/lua/estrelas, clima, nuvens, chuva/neve e relâmpagos. Ponte `Weather` → `WeatherState` (chão molhado, áudio de chuva, vento das árvores), trovão → SFX, visibilidade → névoa e queries em `viber.vael.env_info()`. Gate `VAEL_ELEMENTS=0`. QA: `VAEL_START_HOUR`/`VAEL_START_WEATHER` |
| `docs/LIGHTING.md` | como o céu/exposição/névoa se ligam hoje e o que ficou na engine |
| `docs/JACKDAW.md` | matriz sistema→onde se autora no Jackdaw, componentes tipados, painel de cinemáticas e gatilhos |
| `src/lights.rs` | **luzes práticas** (`bevy_lights` 0.5, pin `=0.5.0`): as luzes de fogo viram `PoolCandidate` (pool orçamentado + flicker + chama billboard); as de preenchimento/cristais ficam autorais. Gate `VAEL_PRACTICAL_LIGHTS=0`. Ver `docs/LIGHTING.md` |
| `docs/WATER.md` | camadas da água (solver iWave, domínio, espelho CPU do swell), flutuação `FloatingBody` (crate `bevy_ext_water`, adapter avian) e barcos a remos (`src/boat.rs`, `data/boats.json`; **E** entra/sai, W/S rema, A/D leme) |
| `docs/PAVING.md` | **pavimento em grelha** (`bevy_ext_terrain_runtime::paving`): `paving="1"` num `<GroundDecal>`/`<Road>`/`<RoadNetwork>` troca a elipse/ribbon por células com ocupação exclusiva, chanfro nos cantos convexos e filete nos côncavos, uma malha por material. Camadas fixas: `DECAL_LIFT 0.040 < PAVING_LIFT 0.064 < ON_PAVING 0.072`; um decal coberto pelo pavimento sobe sozinho. Gate `VIBER_PAVING=0` |

## Cenas

Cada sala é `scenes/interiors/<sala>.bsn` com `<SceneEntry name="from-town">`
e `<ScenePortal name="exit" target="main" entry="@back" auto-radius="1.6">`.
Portas da vila: `world/cities/discordia/portals.bsn`. Jogador, câmara e HUD
são autoloads (`world/autoload/*.bsn`). Sala nova: cria a cena no Jackdaw
(`scenes/interiors/<nome>.bsn`, com chão próprio) e acrescenta um portal na
vila; o painel **Vael: Cena** valida refs/portais no "Validar e gravar".

A cena de menu (`scenes/menu.bsn`) é o `project.start-scene` do `config.yaml`:
sem `--scene`, o `vael-tools run` arranca nela em vez de `main`. É só o acampamento
(o terreno pequeno `scenes/menu.data/terrain.ahgt`) atrás do título; `Novo
Jogo`/`Continuar` trocam para `main`, e a morte/botão "Ecrã de título" voltam
a `menu`. Declara `<NavMesh enabled="0">` — não há navegação no menu.

## Jackdaw no jogo

- **Play**: `jackdaw.toml` → "Play" (`--scene main --entry dev`) e "Play
  (título)". Onde o jogo aparece é preferência local
  (`.jackdaw/settings.json` → `play.separate_window`; hoje em janela
  separada).
- **Componentes nativos** (`src/editor_link.rs`): o que se autora num nó e o
  DSL não conhece (`jackdaw_animation_runtime::*`, `jackdaw_camera_rig::*`,
  `jackdaw_bind::*`, `vael::*`; mais `VAEL_NATIVE_ALLOW=prefixo,…`) chega à
  entidade do jogo. O `bevy_ext_world` guarda-os por nó
  (`specs::native_components_for`).
- **Câmara, áudio e cinemáticas** (`docs/JACKDAW.md`): `CameraSpec`/
  `InteriorSceneSpec` e `AudioMixerSpec`/`MusicLayerSpec`/`BiomeRegionSpec`
  são specs tipadas (inspector *Câmara*/*Áudio*, gizmos no viewport). O
  gatilho de área é o componente `vael::cinematic_trigger::CinematicTrigger`
  (`src/cinematic_trigger.rs`); a extensão tem um **espelho** com o mesmo
  `type_path` (`vael_scene_jackdaw/src/trigger.rs`) para o editor o ver vivo —
  os campos dos dois têm de coincidir. A autoria das cinemáticas é o painel
  **Vael: Cinemáticas** da extensão `vael_scene_jackdaw` (lista planos,
  toca/pára no jogo pelo bridge na 15702, cria plano da câmara do viewport e
  grava-o no `.bsn`). Durante uma cena os sistemas de câmara do Vael não
  correm — o `drive_camera` da engine é o dono.
- **Edições ao vivo no Play**: as entidades recebem o `SceneNodeId` do nó
  (só sob `JACKDAW_PIE`; `VAEL_EDITOR_LINK=1` força) e o `jackdaw_runtime`
  aplica-lhes o `Transform`/componentes que o editor envia. Só nós com id no
  disco (o editor grava-os). `src/live_edit.rs`: `WorldNode`/specs mudados →
  re-spawn do nó pela engine (mesmo pai, pose actual); só props
  (`CustomProperties`) → patch do `ScriptProps` sem reiniciar o script; nó
  novo debaixo de um nosso → montado pela engine; apagar/mover já chega pelo
  runtime. Nós com filhos no mesmo ficheiro re-spawnam com a subárvore do
  load; filhos noutro ficheiro (prefab) e tags só-de-load não. Sob o Play os
  specs/props do nó são espelhados como componentes (`SpecsMirrored`), por
  isso **remover** um spec no editor também chega.
  **Internos de módulos**: o Jackdaw 0.20 não envia overrides de instância
  (nós `PrefabEntityId` sem id) — para editar ao vivo um nó de
  `world/**/*.bsn`, abre o módulo no seu separador (`scene.open`) durante o
  Play. Todos os nós têm `SceneNodeId` no disco desde 2026-11-14
  (`bsn_migrate --node-ids`, só acrescenta linhas; 11.6k nós) — nós novos
  ganham id ao gravar no editor ou com a mesma opção.
- **Animação** (`src/animation.rs`): um `AnimationSet` no nó do modelo tira o
  rig à engine (`AnimatedScene`/`CharacterAnimator`, também num pai) e o
  `jackdaw_animation_runtime` toca-o. `sources: []` usa o `url` do modelo;
  `idle/walk/run` seguem a velocidade; `viber.gesture('x')` toca o estado
  `x` (evento `UnboundGesture` da engine).
- **Input** (`src/input.rs`): `jackdaw_input` com contextos UI (prioridade
  10) e jogo (pára com `MenusOpen`); o `gather_actions` da engine não corre
  (`ExternalActionState`). Debugger remoto: `jackdaw_remote` no BRP (15702).

## Prefabs e organização do mundo

- **Prefabs** em `world/prefabs/<categoria>/<modelo>.bsn` (158, gerados a
  2026-11-14 a partir dos 6.1k objectos repetidos): `village/torch-post`,
  `flora/pine-dark-breakable-dark-wood`, `lights/point-warm-4800`,
  `fx/fire-r18`… A variante mais simples de cada modelo tem o nome simples;
  as outras levam o que acrescentam (`-solid`, `-breakable`, `-<recurso>`).
  Uma instância guarda só nome, `Transform`, `SceneNodeId` e a identidade
  (`WorldOrigin`). Editar o prefab muda todas as cópias (ao recarregar).
- **Identidade** = `WorldOrigin { file, path }` (ou o legado
  `WorldNode.origin_file/path`): é a chave dos saves (`harvest::stable_key`).
  Um `WorldNode` na raiz de uma instância SUBSTITUI o do prefab inteiro —
  nunca pôr um para guardar só a origem.
- **Células** (World Partition): grupos com ≥60 filhos têm pastas
  `célula x<i> z<j>` (128 m). Pastas são transparentes para o loader.
- Ferramentas (`../bevy_extensions`, `-p bevy_ext_world --example`):
  `bsn_prefab_report` (candidatos), `bsn_prefabify --root . [--min 4]`,
  `bsn_organize --root . [--drop-empty-overrides] [--world world.bsn --cells
  128]`, gates `bsn_equiv` (ignora ordem) e `bsn_origin_equiv` (identidades,
  tem de dar 0). Correr os dois gates depois de qualquer reorganização.

## QA: cenas leves e cenários

Testar sem carregar o mundo: `qa/scenarios/**/*.json` (estado de partida —
quest com a cadeia anterior entregue, itens, gear, talentos, nível, hora,
posição — + `steps` + `expect`) e `scenes/qa/*.bsn` (chão plano + entidades
copiadas do mundo; ~10 s). `python3 tools/qa.py sim all` (regras reais, ms;
gate `cargo test --release --test qa_scenarios`), `qa.py scene quest <id>`
(cena + cenários), `qa.py run <cenário>` (jogo vivo, PASS/FAIL), Jackdaw
"Play (QA)" com `qa.py use <cenário>` (gravar o JSON reaplica). Detalhe:
[`docs/QA.md`](docs/QA.md); skill `vael-qa`.

## Gotchas

- **GLB por distância**: o mundo estático já não pede todos os `<GltfScene>` no
  arranque. O `gltf_residency` (engine) deixa a âncora nascer e só pede o GLB
  quando o herói entra no raio da instância — o corte que o `AutoCull` aprendeu
  manda (um arbusto fica residente até ao piso do anel do véu + 60 m; uma
  muralha, com corte no teto de 800 m, até ~860 m), e o *bootstrap* de 800 m
  paga-se **uma vez por URL**. O véu espera só o anel de
  `VIBER_GLTF_READY_RADIUS` (150 m) e o anel largo não corre durante o
  carregamento (competia com o bake dos colliders: ~1 s por frame medido).
  Nunca descarrega nós com `LuaScriptRef`, `CharacterAnimator` ou o herói — o
  rig e o `viber.state` apontam para entidades da subárvore. A/B:
  `VIBER_GLTF_STREAMING=0` volta ao tudo-no-arranque. Detalhe:
  `docs/DEBUG.md`; a prontidão conta só os pendentes do anel (`world_ready.rs`).
- **Relva**: o renderer é o crate `bevy_ext_grass` (em
  `../bevy_extensions/crates/bevy_ext_grass`) sobre `bevy_grass`; o Vael só
  tem o adaptador `src/grass.rs` (regras do `TerrainRuntime`, layout dos
  quadrantes + Vale, vento/tinta/trampling). Paletas e settings são
  `world/settings/grass.bsn` (`GrassSettingsSpec`/`GrassStyleSpec`, editáveis
  no painel "Relva" do Jackdaw). `VIBER_GRASS=0`, `VIBER_GRASS_DENSITY`,
  `GRASS_DEBUG=1` (PGMs em `/tmp`); `graphics:` escolhe o tier. GPU sem
  compute/indirect não desenha relva nenhuma (aviso único no log).
- `Option<T>` em componentes do `.bsn`: o emissor do Jackdaw grava o valor
  nu (`preset: "fire"`) e o `jackdaw_bsn` lia-o como `None` (todas as
  partículas perdiam o preset). O leitor dos specs normaliza
  (`specs::normalize_options`); um componente novo com `Option` passa por
  aí — não confiar no `try_apply` directo do `jackdaw_bsn`.
- `viber.gesture` antes de o rig ligar fica em `PendingGesture` (até 3 s) em
  vez de se perder.

- Acrescentar filhos a `<Entity … />` cria um irmão na origem. Abre a tag (no
  Jackdaw acrescenta pelo Outliner). `analyze`/lint não apanham este erro.
- `shaders/*.wesl` são reescritos em cada `run`. Não editar à mão.
- Uma boca de `<Cave>` em terreno plano é sempre um poço: o tubo começa
  `radius` abaixo da cota. A engine aceita `mound-height`/`mound-radius` para
  levantar um maciço de rocha com face vertical na boca; o túnel subtractivo
  corta o arco na face. O monte é um mod voxel `Union` e tem de vir antes dos
  tubos `Subtract`. Sem monte, uma boca horizontal só existe em encosta.
- **Pavimento**: `paving="1"` num `<GroundDecal>` faz dele uma **área** de
  grelha, não um decal — as arestas snapam para fora e crescem ≤1 módulo
  (7 m de rua → 8 m a 2 m de módulo). Um decal que caia por baixo do pavimento
  **desaparece** (o pavimento é uma camada); o que tem o centro coberto sobe
  sozinho para `on-paving`. As estradas pavimentadas mantêm o `RoadPath` (nav,
  relva, `isPointOnRoad`) e só perdem a ribbon. Detalhe:
  [`docs/PAVING.md`](docs/PAVING.md).
- Luzes de fogo/partículas são `PointLight` SEM `shadows="true"` **no `.bsn`**:
  o cube shadow map custa 6 vistas por luz e pisca no chão. Desde 2026-11-14 as
  luzes de fogo já não ficam autorais: o `src/lights.rs` adota a `PointLight`
  quente mais próxima de cada emissor de fogo como `PoolCandidate` do
  `bevy_lights` (pool de `VAEL_POOL_SIZE`, default 24, flicker + chama
  billboard) e é o pool que decide as sombras (`VAEL_POOL_SHADOWS`, default 2,
  só de noite). As quentes de **preenchimento** (janelas/vila) e os cristais
  frios continuam autorais no `torch_pool_drive` da engine (×8; ×64 só com
  sombra autoral). As luzes que o pool spawna levam `NightGainExempt`, senão a
  base 0 delas (nascem a 0) congelava no ganho nocturno. Os **248** emissores
  `preset: "fire"` também já não correm no sistema de partículas da engine: a
  entidade perde o `AmbientFx`/`ParticleEmitter` e fica com a chama billboard
  do crate (`VAEL_FIRE_MIGRATE=0` mantém as duas para A/B); os outros presets
  (poeira, folhas, fumo, pirilampos) não são tocados. Detalhe:
  `docs/LIGHTING.md`; `VAEL_PRACTICAL_LIGHTS=0` devolve o comportamento
  anterior.
- Com um `<UiInput>` focado (barra de fala do NLU), a engine bloqueia
  `viber.input.*`, `viber.interacted` e o movimento: as letras são texto, não
  acções. O `scripts/ui/say.lua` abre/fecha o foco.
- Refine (`refine.views` no `assets/game.yaml`) está em **mono-vista**
  (`views: 1`). O multiview por Text2D não funciona com o FLUX.3 Klein: é um
  modelo de edição e copia a referência — as vistas `back/left/right` saíam
  ~99% idênticas à frontal (diff médio 2/255, medido a 2026-11-11), sem
  informação nova e a 4-6 passes por asset. Para o religar, as vistas têm de
  ser reais: render do mesh (front/back/left/right) ou modelo
  view-conditioned. Personagens/criaturas nunca entram (o Hi3D muda a pose).
- Paint/xatlas: o `_refined` do Hi3D tem arestas non-manifold; simplificá-lo
  directamente estilhaça a malha (5k componentes/86k bordos) e o xatlas passa
  dos 600 s (watchdog do vramd) com bake trust ~29% = mosaico. O pipeline
  refixa o `_refined` com `--morph-close` (~0,5 mm/m) + `--keep-internal-shells`
  **antes** do simplify; a 265k o unwrap fica ~160 s e o atlas limpo. O
  `to_paint` segue a fórmula (~270k a 2048); não baixar para 150k (ilhas
  minúsculas).
- Depois de editar: `vael-tools analyze` (exit 1 em erro).
- O import do editor Jackdaw 0.20.x rejeita a requirement exata de Bevy: o
  gate sonda `0.20.0` e `0.20.MAX`, e `bevy = "=0.20.1"` não admite nenhum.
  Manter `bevy = "0.20.1"` (como no `bevy_ext_viber_engine`); o `jackdaw.toml` grava
  `bevy = "0.20"`. O `main.rs` aceita `--jackdaw-extract-schema` (descoberta
  de componentes do `jd`).
- Céu e tempo são do `bevy_elements` (`src/environment.rs`): NÃO repor
  `<Sky>`/`<DayCycle>`/`<Weather>` nem `DirectionalLight`/`AmbientLight`
  autorais no `world/environment.bsn`/`scenes/menu.bsn` — a engine conduziria
  um segundo sol/céu. `VAEL_ELEMENTS=0` desliga o crate. A exposição
  automática da engine está cortada no
  `config.yaml` (é do crate).

## Correr

```bash
jd build                      # build oficial: jogo (debug) + schema do editor
jd run .                      # build + lança o jogo (cena de arranque)
jd open .                     # editor oficial
jd doctor .                   # setup: toolchain, plugin, dependências
tools/install.sh              # uma vez: vael-tools no ~/.local/bin
vael-tools run --scene main --entry dev   # debug com argumentos
vael-tools analyze world.bsn              # headless
vael-tools lint                           # lint estrutural
```

**`jd build` e `jd run` são as formas oficiais de buildar e correr o jogo.**
O `jd` compila numa pasta própria (`.jackdaw/target/game`, com o perfil `dev`
deste workspace) e o `jd build` grava também `.jackdaw/{schema,components}.json`
— o schema é o que o editor carrega, por isso é o que se corre depois de mexer
em componentes. O `jd run` lança **sem argumentos**: só o Play do editor lê os
`[[run]]` do `jackdaw.toml`, portanto arranca na cena de arranque
(`scenes/menu.bsn`, atrás do título), não em `main`.

**Tempo de build** (medido a 2026-11-15, 16 cores): o `release` é
incremental (`Cargo.toml`) — mexer num crate do `bevy_extensions` custa
~20–35 s em vez de ~3m20s; a distribuição usa `--profile dist` (thin LTO,
1 CGU, sem incremental). O `jd` compila sempre em `.jackdaw/target/game`;
um symlink local `.jackdaw/target/game -> ../../target` (fora do git) faz o
`jd build` e o `cargo build` partilharem o `target/debug` em vez de
compilarem tudo duas vezes (`mkdir -p .jackdaw/target && ln -sfn
../../target .jackdaw/target/game`; o `jd clean` apaga só o link).

Para `--scene/--entry/--bridge` o atalho é o `vael-tools run`, que lança
`target/release/vael` (outro artefacto: release no `target/` do workspace) ou o
`VAEL_BIN` — `VAEL_BIN=.jackdaw/target/game/debug/vael` reaproveita o build do
`jd`. O `viber` foi aposentado: o CLI vive em `cli/` (`vael-tools`) e o bridge
em `bevy_extensions` (`bevy_ext_bridge_*`). Para testar a engine sozinha: `cd
../bevy_extensions/crates/bevy_ext_viber_engine && cargo run -- run
../Vael/world.bsn`.

Arranque directo: `--scene main` salta a cena de menu e `--entry dev` (a
`<SceneEntry name="dev">` de `world.bsn`) faz `scripts/lib/dev.lua` marcar
`skip_intro`; `title.lua` e `story.lua` saltam o véu e o prólogo. É por
execução (não grava modo nenhum); o autosave seguinte fica com
`story_prologo` marcado. Detalhe: `docs/DEBUG.md`.

QA vivo: uma instância do jogo de cada vez (o `vael-tools debug` fala com a
porta). **Esperar pelo carregamento**: o véu sai com o anel próximo do herói
(`VIBER_GLTF_READY_RADIUS`, 150 m), GPU, herói assente — o resto do mundo entra
por distância depois (ver a *gotcha* dos GLB) e demora — dezenas de segundos,
mais com builds ao lado.
Depois de lançar (ou de `debug scene --wait`): `vael-tools debug --port <p>
wait-ready` antes de screenshot/input; o `ready` vem do espelho
`BridgeReadiness` (`src/bridge_ready.rs`) no ping novo (e do `viber.ping` em
jogos velhos). Uma bridge lenta a responder durante o carregamento é normal —
não matar o jogo. Para agentes: `debug --json` dá JSON em tudo e exit
semântico; as primitivas de teste são `pause/step/play` (determinismo por
frames), `state`, `expect`, `until`, `scene` e `playbook` (passos compostos
com transcript) — detalhe e ressalvas do `scene`: `docs/TOOLS.md`.

Tools MCP nativas (jogo com ferramentas ligadas): o servidor stdio `brp`
(`bevy_brp_mcp` no config do ZCode) fala com o BRP da 15702 — query/insert/
mutate/watch de ECS + `brp_extras/*` (screenshot inline, input, FPS,
shutdown; plugin em `src/main.rs`, `VAEL_BRP_EXTRAS=0` desliga). O CLI fica
para o determinístico (`wait-ready`/`expect`/playbooks); `brp_launch` não se
usa no Vael.

GPU remota (opcional): `source ~/.config/vramd/remote.env` aponta o `vramd` ao
servidor (`VRAMD_REMOTE=192.168.0.100:8790`); o `gameassets resume` passa a
correr as fases GPU lá e as CPU aqui, com upload/download automático dos
ficheiros. O servidor tem **só** `vramd` (systemd, sem Docker/AiGameKit): o
código viaja num bundle de sessão por backend/wave e as envs são cacheadas
por lock; os modelos ficam em `~/.cache/huggingface`. Atalho:
`tools/regen_remote.sh <pack>`. Cada asset aceita `gpu: remote|local|auto` no
manifesto, e as tools/`resume`/`batch` aceitam `--gpu` (ou `VRAMD_GPU_MODE`);
`auto` usa o servidor se `VRAMD_REMOTE` estiver definido. O `rigging3d`
também aceita `--gpu remote` (SkinTokens no servidor; o `gpu: remote` do
manifesto propaga-se ao pipeline). O **Hi3D refine** fica na GPU local (torch
2.5+cu124, única exceção cu12; não entra nas sessões). Detalhe:
`docs/REMOTE_GPU.md`.

Profiling sem screenshots (jogo com `--bridge`): o runtime
(`bevy_ext_bridge_runtime`) responde a `vael-tools debug prof --tab frames`
(p95/hitches/RSS), `--hitches`, `--record 30 --hz 2 -o perf.jsonl` e
`vael-tools debug key/text/click/move` (todos com `input.*` no runtime novo).
`debug lua`/`burst`/`scene` ainda usam os métodos legacy `viber.*` do
monólito. No jogo, **P → FRAMES** e **F3** continuam a ser o profiler da
engine. Detalhe: [`docs/TOOLS.md`](docs/TOOLS.md).

## Fluxo depois da separação

Quatro checkouts irmãos, cada um com o seu remoto:

| Repo | Papel |
|------|--------|
| `Vael` | Mundo, quests, scripts, pool `assets/` e o CLI `vael-tools` (`cli/`). Binários versionados vão em Git LFS |
| `bevy_extensions` | Engine de gameplay (`crates/bevy_ext_viber_engine`), o bridge (`bevy_ext_bridge_protocol`/`_runtime`) e os pacotes publicáveis `bevy_ext_*`. O checkout `Viber` foi aposentado a 2026-11-13 |
| `AiGameKit` | Geração. CLIs no PATH (`gameassets`, `text3d`, `vramd`) |
| `CelerAI` | Modelos NLU/d1 (`models/`, LFS) e crates de inferência. A engine compila-as por path-dep `../CelerAI/crates/*` |

Clone novo de qualquer um precisa dos quatro irmãos lado a lado; no Vael,
correr `tools/fetch_models.sh` uma vez (cria os symlinks `assets/public/{nlu,d1}`).

`tools/install.sh` instala `vael-tools` em `~/.local/bin` (symlink para o
release). O editor oficial: `jd open .` (abre `world.bsn`/as cenas `.bsn`).
Extensões do Vael (`../bevy_extensions/extensions/` e `extensions/`):
`bevy_voxel_terrain_jackdaw` (terreno + paint, preview progressivo de
props/terreno, regista o `WorldNode`), `vael_scene_jackdaw` (ícones do Outliner, categorias Physics/Gameplay/DSL
no inspector, gizmos de colisores/exclusões/spawners/portais/luzes, painel
**Vael: Cena** com problemas e File → "Vael: validar e gravar", e painel
**Vael: Assets**: modelos da cena por categoria, "Colocar" (um por clique) e
"Pincel" (espalha no raio). Cada cópia é um `entity.duplicate` de uma
instância `GltfScene` existente — herda colisor/colheita/LOD — assente no
`.ahgt`; um traço = um undo. Para agentes: operador `vael.assets.place_at`
(`url`, `x`, `z`, `count`, `radius`)), `vael_jackdaw` (painéis **Ambiente**, **PostFX** e **Flora**:
hora/clima com sol de preview, efeitos com "lente do jogo" no viewport,
espécies com preview real do `bevy_flora` e árvores nas estações) e
`vael_content_jackdaw` (painéis **Itens**, **Marcos**, **Progressão**,
**Diálogos**, **Missões**, **Conteúdo** e **Cenas**: edita os JSON de
`data/`, `quests/` e `dialogues/` e valida-os; substitui os painéis do
`editor/` legado), `vael_analyze_jackdaw` (painel **Vael: Problems**, operador
`vael.analyze.run` sobre a lib `vael_tools`), `vael_profiler_jackdaw`
(p95/hitches/RSS do runtime em polling), `vael_debugger_jackdaw`
(play/pause/step/F3) e `vael_mcp_jackdaw` (operadores de
grupo/input/screenshot visíveis no `jd mcp`). Só UMA
extensão pode registar um tipo partilhado (`WorldNode`): cada dylib tem a sua
cópia e a última a registar rouba o type path às outras — as restantes leem-no
por reflexão. Uma entidade autorada sem `Transform` não entra na hierarquia
do editor. Os modelos NÃO levam `GltfSource` no `.bsn`: o loader do editor não lê meshopt/KTX2 e
cada um era uma leitura falhada do GLB; o preview real vem da cache descodificada
da `bevy_voxel_terrain_jackdaw` (`assets/public/.jackdaw-cache/`). A câmara do
viewport escala a velocidade pela distância ao chão (`vael_jackdaw`, `camera.rs`). Ciclo: `jd build` →
`jd extension pack` → `jd extension install --trust-publisher`; ver
`extensions/bevy_voxel_terrain_jackdaw/README.md`.

Assets: `cd assets && gameassets resume --profile game.yaml --manifest manifests/<pack>`.
Output em `assets/public`. Intermediários (`meshes/_intermediate`, imagens de
trabalho) ficam de fora do git. O que entra no repo (`.ktx2`, `.ogg`, `.png`,
`.webp`, `.ahgt`, `.vinst`, `.wasm`, `.glb`) é Git LFS — actualizar quando
um pack muda, não a cada frame de geração.

Dos GLB, só o **modelo base** vai para o LFS: `*_lod0.glb`, as colisões
`*_collision.glb` e os que não têm ladder (`vegetation/*.glb`). LOD1/LOD2 são
derivados e ficam fora do git — reconstrói-os com
`python3 tools/make_lods.py` (a partir da raiz; `--jobs N`, `--force`,
`--only 'characters/*'`). O script usa o `text3d lod` + `text3d finish` com os
parâmetros do pipeline (`assets/game.yaml`: ratios 0.40/0.22, textura 2048,
`rig_max_level: 2`), por isso o mesmo base dá sempre a mesma ladder. Um clone
só precisa de correr o script uma vez depois de `gameassets resume`.

Cofre, XP e quests vivem em `scripts/lib/rules.lua`. A engine não expõe
`viber.vault_*` / `viber.add_xp`. O kit inicial (30 ouro, 1 poção) semeia-se
aí quando o save ainda não tem inventário.

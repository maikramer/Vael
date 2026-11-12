# AGENTS.md — Vael

Jogo. A engine é o checkout irmão `../Viber`. O editor oficial é o **Jackdaw
0.20 instalado**: `jd open .` abre este projeto; as extensões do Vael entram
como `.jdext` assinados. O crate `editor/` é legado (já não é o fluxo).
As ferramentas de geração são o checkout irmão `../AiGameKit` (CLIs no PATH).
Os modelos de IA (NLU e decisão) e os seus runtimes são o checkout irmão
`../CelerAI`. Este ficheiro é curto de propósito. O contrato da engine está em
`../Viber/AGENTS.md`; lê a secção que a tarefa pede, não o ficheiro inteiro.

## Irmãos

| Checkout | Quando |
|----------|--------|
| `.` (Vael) | Mundo, scripts, quests, cenas, pool `assets/` |
| `../Viber` | Bug ou API em falta na engine. `cargo test` lá. `viber run` a partir desse checkout reconstrói |
| `editor/` | **Legado**: editor próprio sobre Jackdaw. O fluxo é o Jackdaw oficial (`jd open .`) + extensões `.jdext` |
| `../bevy_extensions` | Crates `bevy_ext_*`/`bevy_voxel_terrain` e as extensões oficiais em `extensions/` |
| `../AiGameKit` | Gerar ou corrigir uma tool. Não abras o monorepo no workspace por defeito |
| `../CelerAI` | Modelos `models/{nlu,d1}` (LFS) e crates de inferência. Sem ele a engine não compila nem os scripts carregam IA |

Workspace do dia-a-dia: `Vael.code-workspace` (esta pasta + `../Viber` +
`../CelerAI`).

## Crate Rust (migração)

Este checkout é o worktree da migração para Rust/Bevy (`refactor/bevy-native`).
O jogo arranca pelo crate `vael`:

```bash
CARGO_TARGET_DIR=../Viber/target cargo run --release -- \
  world.bsn --scene main --entry dev --bridge 15706
```

`gameplay:false` desliga IA/vitais/projéteis da engine; `player_camera:true`
mantém o movimento/câmara da engine enquanto o Vael não os substitui. O
`viber.game()` ainda é a fonte de verdade; `VaelStateSync` espelha-o e
`VaelHud` publica o HUD. O estado, gates e dívidas estão em
[`docs/MIGRATION.md`](docs/MIGRATION.md). O checkout principal pode ter
trabalho em curso (regen/assets); não o tocar por arrasto.

## Só estático

`world.bsn` (árvore `<world static="true">`) + `world/**/*.bsn` +
`world.data/` são o mundo. O `world.xml` e os módulos `.xml` ficam como
legado da CLI de cena. As poses do `.bsn` são finais. O runtime não assenta
props nem corre `<StaticSpawner>` / `place` (erro de parse). Única geração em
runtime: `<DynamicSpawner>` (criaturas).

Editar com `viber scene` / `viber tool` / `viber terrain` (offline ou `--live`,
nos `.xml` legados) ou no **Jackdaw oficial** (`jd open .`; cenas `.bsn`).
Hand-editar uma pose à espera que a engine a ajuste não funciona.
Depois de pincéis: `viber tool settle`. Árvores e dressing:
`viber tool scatter`, não um re-bake. Vegetação `.vinst` reescreve-se com
`viber tool scatter` e não entra no undo — git ou re-scatter.

`world.data/terrain.ahgt` e `.paint` são estado do mundo. Commita-os.

## Mapa

| Caminho | O que é |
|---------|---------|
| `world.bsn` | raiz estática e os módulos `<Include>` (`world/**/*.bsn`) |
| `world/**/*.bsn` | cidades, criaturas, landmarks, `spawn/…`, HUD (fonte; o `.xml` legado fica ao lado) |
| `scenes/menu.bsn` | cena de arranque: acampamento pequeno atrás do título |
| `scenes/interiors/` | salas `.bsn` (cenas próprias, sem terreno) |
| `quests/*.json` | 37 quests + `index.json`; lidas por `scripts/lib/rules.lua` (reiniciar o jogo; sem `cargo build`) |
| `dialogues/*.json` | grafos ambientais; lidos por `scripts/townsfolk.lua` |
| `data/*.json` | itens/loja, marcos, progressão, enigmas (`riddles.json`) e regras do juiz (`verdicts.json`); lidos por `rules.lua`/`shop.lua`/`landmarks.lua`/`riddle.lua`/`judge.lua` |
| `editor/` + `content/` | **Legado**: App própria `vael-editor` (testes/painéis antigos) e esquemas `vael-content`. O fluxo é o Jackdaw oficial (`jd open .`; `jackdaw.toml` fixa `package = "vael"`/`plugin = "VaelPlugin"`) com as extensões `.jdext` em `../bevy_extensions/extensions/` |
| `scripts/**.lua` | comportamento. Hot-reload ao gravar |
| `assets/public/nlu/` | **symlink** para `../CelerAI/models/nlu` (criado por `tools/fetch_models.sh`). `viber.nlu` na engine; relato de 1 palavra no fecho das quests. `assets/public/d1` idem para `models/d1` (`viber.decision`) |
| `ui/*.css` | HUD e menu |
| `assets/` | pool. `game.yaml` + `manifests/`. Binários em `assets/public/` |
| `docs/AUDIO.md` | direção sonora (Coro de Vael), layers por bioma, manifests e gate de costura; regen com `assets/regen_audio.py` |
| `docs/FLORA.md` | árvores procedurais (`bevy_flora`): estações `prop-flora` no XML, pools por espécie, colheita provisória e `tools/flora_migrate.py` |
| `config.yaml` | `assets.roots: assets/public` |
| `src/environment.rs` | **dono do céu/tempo** (`bevy_elements`): relógio astronómico, sol/lua/estrelas, clima, nuvens, chuva/neve e relâmpagos. Ponte `Weather` → `WeatherState` (chão molhado, áudio de chuva, vento das árvores), trovão → SFX, visibilidade → névoa e queries em `viber.vael.env_info()`. Gate `VAEL_ELEMENTS=0`. QA: `VAEL_START_HOUR`/`VAEL_START_WEATHER` |
| `docs/LIGHTING.md` | como o céu/exposição/névoa se ligam hoje e o que ficou na engine |
| `qa-elements.bsn` | **cópia** do `world.bsn` para QA do ambiente (corre-se sem escolher cena; save `qa-elements` isolado) |
| `qa-*.bsn` | mundos QA. Incluem `world/**/*.bsn` — editar o módulo muda o mundo |

## Cenas

Cada sala é `scenes/interiors/<sala>.bsn` com `<SceneEntry name="from-town">`
e `<ScenePortal name="exit" target="main" entry="@back" auto-radius="1.6">`.
Portas da vila: `world/cities/discordia/portals.xml`. Jogador, câmara e HUD
são autoloads (`world/autoload/*.bsn`). Sala nova:
`viber scene new-scene interiors/<nome> --interior "20 16"` mais um portal
na vila. `viber scene lint` valida alvos.

A cena de menu (`scenes/menu.bsn`) é o `project.start-scene` do `config.yaml`:
sem `--scene`, o `viber run` arranca nela em vez de `main`. É só o acampamento
(o terreno pequeno `scenes/menu.data/terrain.ahgt`) atrás do título; `Novo
Jogo`/`Continuar` trocam para `main`, e a morte/botão "Ecrã de título" voltam
a `menu`. Declara `<NavMesh enabled="0">` — não há navegação no menu.

## Gotchas

- Acrescentar filhos a `<Entity … />` cria um irmão na origem. Abre a tag.
  `viber scene insert` não tem este problema. O `analyze` não apanha.
- `shaders/*.wesl` são reescritos em cada `run`. Não editar à mão.
- Uma boca de `<Cave>` em terreno plano é sempre um poço: o tubo começa
  `radius` abaixo da cota. A engine aceita `mound-height`/`mound-radius` para
  levantar um maciço de rocha com face vertical na boca; o túnel subtractivo
  corta o arco na face. O monte é um mod voxel `Union` e tem de vir antes dos
  tubos `Subtract`. Sem monte, uma boca horizontal só existe em encosta.
- Luzes de fogo/partículas são `PointLight` SEM `shadows="true"`: o cube
  shadow map custa 6 vistas por luz e pisca no chão. A luz simples ilumina
  igual; o pool noturno fica em `WARM_FILL_NIGHT_GAIN` (×2), não no ×8 das
  tochas com sombra.
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
- Depois de editar: `viber analyze` (exit 1 em erro).
- O import do editor Jackdaw 0.20.x rejeita a requirement exata de Bevy: o
  gate sonda `0.20.0` e `0.20.MAX`, e `bevy = "=0.20.1"` não admite nenhum.
  Manter `bevy = "0.20.1"` (como no `../Viber`); o `jackdaw.toml` grava
  `bevy = "0.20"`. O `main.rs` aceita `--jackdaw-extract-schema` (descoberta
  de componentes do `jd`).
- Céu e tempo são do `bevy_elements` (`src/environment.rs`): NÃO repor
  `<Sky>`/`<DayCycle>`/`<Weather>` nem `DirectionalLight`/`AmbientLight`
  autorais no `world/environment.bsn`/`scenes/menu.bsn` — a engine conduziria
  um segundo sol/céu. Os `qa-*.bsn` antigos ainda os declaram; correr com
  `VAEL_ELEMENTS=0`. A exposição automática da engine está cortada no
  `config.yaml` (é do crate).

## Correr

```bash
cd ../Viber && cargo run -- run ../Vael/world.bsn
cd ../Viber && cargo run -- analyze ../Vael/world.bsn
viber run          # cwd nesta pasta: binário do PATH, não recompila ../Viber
viber run --scene main --entry dev   # debug: directo ao jogo, sem título/prólogo
```

Arranque directo: `--scene main` salta a cena de menu e `--entry dev` (a
`<SceneEntry name="dev">` de `world.bsn`) faz `scripts/lib/dev.lua` marcar
`skip_intro`; `title.lua` e `story.lua` saltam o véu e o prólogo. É por
execução (não grava modo nenhum); o autosave seguinte fica com
`story_prologo` marcado. Detalhe: `docs/DEBUG.md`.

QA vivo: skill `viber-engine` (sessão `viber session`, uma engine de cada vez).

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

Profiling sem screenshots (engine com `--bridge`): `viber debug prof --tab
frames` (frame/p95/topo/hitches), `--hitches` (picos com atribuição),
`--record 30 --hz 2 -o perf.jsonl` (série + resumo), `viber debug trace 10
-o trace.json` (Perfetto) / `--status` / `--stop`, e `viber debug mark "…"`
para correlacionar. No jogo: **P → FRAMES** e o overlay **F3** (p95, topo,
hitches, RSS). Detalhe: `../Viber/docs/PERFORMANCE.md`.

## Fluxo depois da separação

Quatro checkouts irmãos, cada um com o seu remoto:

| Repo | Papel |
|------|--------|
| `Vael` | Mundo, quests, scripts, pool `assets/`. Binários versionados vão em Git LFS |
| `Viber` | Engine. `~/.local/bin/viber` é um wrapper que segue o checkout |
| `AiGameKit` | Geração. CLIs no PATH (`gameassets`, `text3d`, `vramd`) |
| `CelerAI` | Modelos NLU/d1 (`models/`, LFS) e crates de inferência. O Viber compila-as por path-dep `../CelerAI/crates/*` |

Clone novo de qualquer um precisa dos quatro irmãos lado a lado; no Vael,
correr `tools/fetch_models.sh` uma vez (cria os symlinks `assets/public/{nlu,d1}`).

`~/.local/bin/viber` é o `../Viber/scripts/viber-latest.sh`: tira um
fingerprint das fontes da engine e só chama `cargo build --release` quando
mudaram; senão corre a cache (`~/.local/share/viber/bin/viber-release`). Build
falhado → corre o último binário bom e não volta a tentar até a árvore mudar.
`debug`/`session` nunca compilam. `VIBER_NO_BUILD=1` salta, `VIBER_FORCE_BUILD=1`
força. NÃO correr `./install.sh` (viber) por cima — substitui o wrapper por um
binário fixo que deixa de ver o código. Reinstalar o wrapper:
`install -m 755 ../Viber/scripts/viber-latest.sh ~/.local/bin/viber`. O editor
oficial: `jd open .` (abre `world.bsn`/as cenas `.bsn`; a CLI de cena continua
XML). Extensões do Vael: `bevy_extensions/extensions/` → `jd build` →
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

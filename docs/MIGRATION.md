# Migração do Vael para Rust/Bevy

Este documento é o estado da migração. O objetivo é o Viber ficar só com o que
o Bevy não tem (terreno, navegação, física/Rapier, meshopt, partículas, IK,
céu/água custom) e o Vael passar a ser um crate Rust que
usa `bevy` diretamente. O Luau do jogo é transitório; o editor de conteúdo
saiu do Viber e vive em `Vael/editor` (Jackdaw), sem Lua.

O plano de empacotamento — fases, DAG `bevy_*`, ordem de publicação e gates —
está em [`PACKAGES.md`](PACKAGES.md). Este documento fica com o estado da
migração do jogo; o `PACKAGES.md` manda nas fases e na fronteira
engine/pacotes.

## Onde vive

- O código e o jogo vivem no checkout principal do Vael, na `main` (o crate e
  as ondas de migração estão fundidos; topo `2da39e0`).
- O worktree `../refactor/Vael` fica como backup do pool LFS do checkpoint;
  o `refactor/Viber` foi integrado na `main` do Viber.
- Checkpoint do Vael: `59a57ff`, tag `pre-bevy-migration`.
- Commits: Viber `92c1f42` (reflection/ECS/UI/save), `e79c4fc`
  (`player_camera`), `f259627` (AiMode), `5cf9d8f` (apply_patch),
  `5ec7b9d` (UiWriter), `5dc6b64` (IO stack); Vael `528149c` (controlos no
  Lua), `f462a23` (crate Rust), `1eabf60` (sync + HUD), `3b19050`
  (autoridade), `0808883` (HUD comportamental),
  `798a1f8`/`b59c924`/`2c2811c`/`fc586cd`/`6291c37`/`209f078`
  (cofre/XP/equipamento/quests/combate/relógio).

## O que mudou no Viber

- `src/reflect.rs`: `ReflectPlugin` regista 342 componentes/resources.
  `bevy_remote` passa a ver `Health`, `Transform`, `UiBar`, etc. O BRP expõe
  `world.query`/`world.get_resources` (não `bevy/query`).
- `src/luau/ecs.rs`: `viber.ecs.{get,set,insert,remove,query,spawn,despawn,types}`
  sobre reflexão, com whitelist. `set_scale`/`set_pose`/`set_visible` passam
  por lá.
- `AppOptions` ganhou `gameplay`, `player_camera`, `scripts: ScriptsMode`, `ui`
  e `profiler`; `build_app_with_world` permite ao jogo compor a App. O default
  preserva o comportamento do CLI.
- `AiMode { Off, Service, Full }`: `AiServicePlugin` (sentidos, vitais de
  criaturas, `apply_ai_locomotion`) separa-se de `SimpleAiPlugin` (`sense_ai`
  para criaturas sem script). Em `Service`, `drain_sense_bus` evita o
  `SenseBus` a crescer. O Vael usa `Service`.
- `apply_patch` (host Luau) e `UiWriter` (fila de `UiCommand` para o jogo
  dirigir a UI declarativa em Rust) são as duas pontas que o crate usa.
- Widgets/modal/foco/gamepad usam `bevy_ui_widgets`/`bevy_input_focus`; o XML e
  o CSS declarativos mantêm-se.
- `CullDistance` passou a `VisibilityRange`; o save tem um envelope genérico
  com `world_kv` + secção `game` opaca; o código RPG morto saiu do engine.
- Profiler `timed.rs` e atmosfera/IBL do Bevy ficaram como `TODO(migration)`:
  não são bloqueantes e têm risco visual/perf.

## O que mudou no Vael

- `Cargo.toml` + `src/main.rs`: crate `vael`, Bevy `=0.20.1`, `viber` por path.
  Arranque: `build_app(..., gameplay:false, player_camera:true,
  scripts:ScriptsMode::Compat)` + `VaelPlugin`.
- `src/content.rs`: loaders serde dos JSON actuais (22 itens, 17 skills, 8
  receitas, 22 marcos, 3 enigmas, 1 veredicto, 3 puzzles, 37 quests, 6
  diálogos, locale). Os schemas não mudaram.
- `src/save.rs`: `VaelSave` tipado, migração do `world_kv` plano e espelho
  `to_world_kv`. A secção `game.vael` é a fonte de verdade futura.
- `src/attach.rs`: `VaelScriptTags` (path + props) a partir de
  `LuaScriptRef`/`ScriptProps`; 199 entidades no `main`.
- `src/state_sync.rs`: lê `viber.game()` via `game_to_json` a 10 Hz e espelha
  para `PlayerVitals`/`XpState`/`Inventory`/`QuestLog`/`VaelSave`.
- `src/hud.rs`: publica vida/XP/talentos/economia/status/quest antes de
  `UiSet::Bind`. O `rules.publish` deixou de escrever esses binds; ainda
  escreve netplay/alvo/chefe/recargas/quests.others. `VAEL_HUD_RUST=0` desliga
  o HUD Rust. O primeiro publish é corrigido com `is_some_and`.
- `src/rules_shadow.rs`: simulador shadow de vitais/status/morte (veneno
  4 dps, queimadura 6 dps, chuva ×2, i-frames, `dying` 2,2 s, `death_wait`
  5 s, respawn). É opt-in (`VAEL_RULES_SHADOW=1`) e não escreve no jogo.
  Paridade ao vivo: veneno 100→92 nos dois; morte `death_pending`/`wait=5`;
  respawn `hp=100`.
- `src/rules_authority.rs`: autoridade opt-in (`VAEL_RULES_RUST=1`). Lê o KV
  do Lua depois de `luau_update`, corre o `shadow_tick` e escreve de volta
  `hp`, `iframes`, estados, `dying`, `death_pending`, `death_wait` via
  `apply_patch`. O `rules.lua` salta só as secções portadas; dano/cura, o ecrã
  de morte e o respawn manual continuam no Lua. QA vivo: veneno 100→92 sem
  duplicar, respawn manual `hp=100`/iframes 2→0, `damage_player(10)` lido
  como `hp=90`, HUD `0.9/90/100`. Default **off** até QA de combate,
  interiores e save/load.
- `src/hud_ui.rs`: o comportamento do `hud.lua` (outras missões, clique do
  DIÁRIO, TTL da barra de chefe, shake do combo) passou a sistemas Bevy com
  `UiWriter`; o `hud.lua` ficou só com `say.tick`.
- `src/vault.rs`, `src/xp.rs`, `src/gear.rs`, `src/quest.rs`,
  `src/combat.rs`, `src/clock.rs`, `src/economy.rs`: cofre/caps e kit inicial,
  XP/níveis/talentos, equipamento/stats, ciclo de quests, guarda/dano/cura/
  i-frames, relógio (hoje `WorldClock`/`Weather` do `bevy_elements`) e
  transações/preços de loja/forja vivem em Rust. Uma ponte `viber.vael`
  instalada no `Startup` expõe as funções aos chamadores Lua; os wrappers
  guardam toasts, sons, `sync_collect`, `start_death`, `report_craft` e o
  resto da apresentação/UI.
- `src/environment.rs`: o ambiente inteiro passou a `bevy_elements` — sol/lua/
  estrelas, clima, nuvens, chuva/neve, névoa e relâmpagos. O `<Sky>`,
  `<DayCycle>` e `<Weather>` saíram do mundo (a engine já não é dona do céu);
  o `AtmosphereState` é sincronizado e a névoa por bioma continua na engine.
  Ver `docs/LIGHTING.md`. `VAEL_ELEMENTS=0` volta ao céu antigo.
- `src/stats.rs`: atributos e stats do herói no `bevy_gauge` 0.6. Força,
  Agilidade, Vitalidade, Precisão e Espírito alimentam expressões
  (`Stat.damage`, `Stat.max_hp`, `Stat.crit`, `Stat.attack_speed`, …); os
  efeitos de talentos/itens são modificadores e os requisitos de equipamento
  (`"requires": "Strength >= 7"`) são expressões do mesmo grafo. A bridge
  acrescenta `viber.vael.attribute/attributes/stat_sheet` e
  `gear_requirement*`; `player_stats()` e `stat()` mantêm as chaves antigas.
  O `StatEngine` tem uma `World` mínima própria porque as funções Lua correm
  dentro do `luau_update` — o snapshot é síncrono e só reconstrói quando o
  conjunto equipado/talentos muda.
- `src/ui/talent_tree.rs`: a aba Talentos passou de lista plana a árvore
  visual no `bevy_pf` (`Canvas` + `Line` + `ItemsControl`). O layout é
  derivado do próprio grafo (`requires`/`excludes`) — 4 raízes, 3 tiers,
  15 arestas e 4 pares de exclusão — e não há coordenadas no
  `data/progression.json`. O clique seleciona o nó; o painel lateral mostra
  efeitos/requisitos e o botão Aprender. Os 19 ícones são gerados por
  `text2d` (`assets/talent-icons.json`) e lidos como
  `assets/icons/talent_<id>.png`.
- `src/harvest.rs`: `Harvestable` a partir de `ScriptProps`, foco com
  `ScriptInteraction`, golpe J, durabilidade, loot/XP e persistência por chave
  estável. `VAEL_HARVEST_RUST=1`; o `harvest.lua` faz early-return e fica como
  fallback.
- `src/travel.rs`: catálogo/nearest/label/reveal dos 22 marcos; bridge
  `viber.vael.travel_*`. `VAEL_TRAVEL_RUST=1`; `landmarks.lua` delega com
  fallback. Teleporte, marcadores no mundo e mapa continuam Lua.
- `src/quest_events.rs`: `report_kill`/`scan_visits`/`sync_collect` com fila
  de eventos e sistema Bevy depois de `luau_update`, patch no `viber_game` e
  espelho `QuestLog`. `VAEL_QUEST_EVENTS_RUST=1`; `rules.lua` mantém o caminho
  Lua como fallback.
- `src/world_text.rs`: o texto flutuante no mundo (dano, cura, esquiva,
  bloqueio, coleta e combo) passou do pool de UI 2D projectado para
  `bevy_rich_text3d` — quads por glifo com o atlas da Cinzel do HUD, pool de
  32 slots, billboard para a câmara, subida e pop de escala iguais aos
  antigos, fade no alpha do material (mudar `Text3dStyle.color` por frame
  re-tesselava). A boca continua a ser o `DamageNumberEvent`:
  `viber.damage_number` e o `harvest.rs` não mudaram. A engine ganhou
  `AppOptions::damage_numbers_2d` (default `true`); o Vael arranca com
  `false`. O `combat.lua` passou a soltar também um "N HITS" 3D acima do
  herói a partir do 2.º golpe, mantendo o contador do HUD.
- `src/speech.rs`: as falas do `viber.say` saíram do balão do HUD e passaram
  a balões no MUNDO (`bevy_speech_bubbles`): texto rasterizado na CPU num quad
  billboard, ancorado ao NPC/quadro/altar, oculto por paredes. A engine ganhou
  `ScriptSay { speaker, text, seconds }`, o 3.º argumento
  `viber.say(texto, segs, orador)` e `viber.entity_id()`; o Vael instala
  `ExternalSayPresenter` e o balão nativo fica desligado. Linhas de callbacks
  que correm no HUD caem para o alvo do [E] e, sem ele, para um toast. O
  `scripts/ui/say.lua` guarda o orador no `ask` e expõe `say.say(texto, segs)`.
- O jogo passa a **exigir** o crate `vael` em runtime: `viber.vael` tem de
  existir antes de `lib/rules.lua`/`lib/gear.lua`/`lib/clock.lua` correrem.

## Correr e validar

```bash
# a partir da raiz do Vael (checkout principal, com o pool atual)
CARGO_TARGET_DIR=../Viber/target cargo run --release -- \
  world.bsn --scene main --entry dev --bridge 15706
```

- `--scene main` salta o menu/título; `--entry dev` salta o prólogo.
- O bridge fica em `127.0.0.1:15706`; `curl` com `viber.lua`, `world.query`,
  `world.get_resources`, `viber.ecs`.
- `VIBER_NO_BUILD=1 ./tools/check.sh` valida conteúdo/Lua/analyze/scene lint.
- `CARGO_TARGET_DIR=../Viber/target cargo test` no Vael: **236 testes**.
- Editor oficial: `jd open .`, `jd doctor --project .`; as extensões do Vael
  (terreno voxel/painéis) vivem em `../bevy_extensions/extensions/` e entram
  por `jd build` → `jd extension pack` → `jd extension install`.
  `cargo test -p vael-content` continua o gate dos esquemas; o crate
  `editor/` é legado (`cargo test -p vael-editor --test smoke`).
- Mundo `.bsn`: `viber bsn convert --check ../Vael/world.xml` re-converte a
  partir do XML legado; `viber analyze world.bsn` e `viber scene lint` são os
  gates do formato novo.
- `cargo test` no Viber: 1646 passam; a falha
  `luau::decision::tests::lua_api_intent_pt_multi_question` é modelo/ambiente.

## Mundo em `.bsn` (2026-11-12)

O mundo passou a ser autorado em `.bsn` (Jackdaw), mas o runtime continua a
usar o `parse_world`/spawner do Viber. Cada nó XML é uma entidade com
`bevy_world_xml::bsn::WorldNode { tag, attrs, text, origin, pose }`; o loader
`load_world` lê o `.bsn`, reconstrói os `XmlNode` e entrega-os ao parser.
`Transform`/`Name`/`GltfSource`/luzes nativas são traduzidos de volta para
atributos (pose/nome/url/cor/intensidade), para o editor poder mover e
ajustar sem perder a semântica.

- `world.bsn` + `world/**/*.bsn` preservam os 64 `<Include>` via nós
  `Include` (o leitor prefere o irmão `.bsn`); `scenes/menu.bsn` e
  `scenes/interiors/*.bsn` são as cenas/áreas internas.
- `world.data/` (`.ahgt`, `.paint`, `.vinst`, `bake.json`) fica como estava;
  os attrs `heightmap`/`instances` continuam a apontar para lá.
- O conversor é `viber bsn convert [--check] [--force] [--no-preview]`; o
  `--check` compara XML e `.bsn` por pose/luz/cor com tolerância (não por
  texto). O `.xml` fica no repo como fallback da CLI de cena.
- `viber scene`, `viber tool settle/scatter` e `viber bake` continuam
  XML-only nesta fase; `viber analyze` e `viber scene lint` já leem `.bsn`.
- No editor, `Vael: Cenas` abre `world.bsn`, o menu e cada interior como abas;
  o painel `WorldNode` mostra/edita tag e attrs. Terreno/composições/UI ainda
  não têm preview no viewport.

## Estado e próximo

| Fase | Estado |
|------|--------|
| Fase 0 — checkpoint/baseline | feito |
| Fase 1 — reflection, ECS, AppOptions, widgets, culling, save | feito (profiler/atmosfera adiados) |
| Fase 2 — crate Vael, conteúdo, save, attach, arranque | feito |
| Onda 2a — state sync + HUD Rust | feito |
| Onda 2b — vitais/status/morte autoritativos em Rust | autoridade opt-in validada ao vivo; default off |
| Regras/hub — HUD, cofre, XP, equipamento/stats, quests, combate do jogador, relógio, loja/forja | feito em Rust com ponte `viber.vael` |
| Colheita, marcos e eventos de quest | feito em Rust (opt-in), validado live (colheita no pool atual; quests/marcos no pool anterior) |
| UI do jogador — HUD, diário, mapa/viagem, título e morte em `bevy_pf` | feito em Rust (XAML + view-models + controlador M/G/F); barra de fala/NLU continua na engine |
| Editor + mundo — Jackdaw oficial + extensões `.jdext` + `vael-content` + `.bsn` | feito; Grip adiado; CLI de cena continua XML |
| Restante do jogo — FSM/inimigos/bosses, viagem/mapa (teleporte/marcadores), diálogo/NLU, story/áudio | pendente |
| Fase 4 — mover player/câmera/AI/vitais para o Vael; recipe extensions; docs | pendente |

## QA no checkout principal

O código migrado está na `main` do Vael e o jogo corre daqui, com o **pool
atual** (`assets/public`, hero novo de 8,7 MB). O worktree `refactor/Vael`
fica como backup do pool do checkpoint (hero antigo de 14,9 MB) e **não**
serve para QA visual.

O Viber `main` (`5dc6b64`) inicializa o `IoTaskPool` com **32 MiB** antes do
`TaskPoolPlugin`: os GLBs novos do text3d têm hierarquias mais fundas e o
stack default de 2 MiB estourava no loader. Já não é preciso `RUST_MIN_STACK`;
o jogo arranca com o pool atual (screenshot do hero novo, `item.stone=4` na
colheita, gates `VAEL_*` ligados, 22 marcos).

O pool atual está **commitado** em LFS (`1057171`, personagens na A-pose
nova; hero de 8,7 MB). A `main` está um commit à frente da `origin/main`
(este doc); nada do pool ficou por commitar.

## Dívidas conhecidas

- `VaelSave` é substituído inteiro pelo snapshot de 10 Hz; **não** gravar
  `game.vael` enquanto isto não for um merge (perde campos só do Rust).
- `VaelSave` não serializa no BRP por `BTreeSet` sem `ReflectSerialize`;
  usar `Vec` ou registar o type data antes de expor o save no bridge.
- O worktree `refactor/Vael` (backup) não tem LOD1/LOD2; o checkout principal
  tem o pool atual. Os 404 de LODs no backup são esperados.
- A câmara/player ainda são da engine (`player_camera:true`). O Vael só os
  desliga quando tiver o seu controller/câmara; sem isso o render vai a OOM.
- O dano/cura já são Rust (`src/combat.rs`), mas os portões de menu/título/
  god, os FX/toasts/shake e o `start_death` continuam no wrapper Lua. A
  autoridade das vitais lê/escreve as mesmas chaves depois do `luau_update`.
  O `M.god` do dev mode é espelhado em `vael_god` pela autoridade (o F2
  repõe o valor todos os frames via `devmode.lua`; o teste tem de segurar a
  tecla, não chamar `set_god` uma vez).
- O respawn de fallback da autoridade repõe as vitais mas não teleporta nem
  reabastece frascos; isso é o `M.respawn` manual do ecrã de morte. Levar o
  teleporte/refill para Rust é uma decisão da próxima onda.
- `rules.lua` ainda tem `report_kill`/`scan_visits`/`sync_collect` como
  fallback, parley/decisão e o `award_kill`; `fsm.lua`/combate/inimigos,
  viagem (teleporte/marcadores/mapa), diálogo/NLU e UI/story/áudio continuam
  Lua. Colheita, marcos e eventos de quest já têm caminho Rust opt-in. A
  loja/forja tem transações Rust, mas as listas/formatação continuam Lua.
- O `crystal-outpost` tem `destructible`/`resource-kind` num `<Entity>` com
  `<GltfScene>` filho; esses props não chegam a `ScriptProps`/`Harvestable`
  (o `player_nearby` também os não vê). O QA de colheita usou os nós de pedra
  do `spawn/ring.xml`. Investigar o parser de recipes ou autorar os props no
  filho.
- `VAEL_HARVEST_RUST`, `VAEL_TRAVEL_RUST` e `VAEL_QUEST_EVENTS_RUST` são
  opt-in; o fallback Lua mantém-se. Ligar por default depois de QA com o pool
  atual.
- `rules.publish` ainda publica netplay/alvo/combo/`abilities.active`/
  `vitals.active`/`xp.recent`/`quest.recent`; os binds de vida/XP/economia/
  status/quest/chefe/quests.others já são do `VaelHud`/`HudUiPlugin`.
- O jogo passa a exigir o crate `vael` em runtime (`viber.vael`); `viber run`
  direto do checkout da engine já não serve para o mundo do Vael.
- `VAEL_RULES_RUST` continua opt-in; o QA de combate/interiores/save ficou
  pendente porque a regen ocupa o pool/GPU. O shadow (`VAEL_RULES_SHADOW=1`)
  fica como ferramenta de paridade.

## UI do jogador em bevy_pf

A apresentação do jogo passou para `bevy_pf` 0.5.2 (XAML/WPF sobre `bevy_ui`,
Bevy `0.20.1` exato). O crate é externo e evolui sem alterações nossas; o
Vael só consome `PfUiPlugin`, `Bindable`, `xaml!`/`include_xaml!`, estilos,
templates e comandos.

| Peça | Onde |
|------|------|
| Plugin e HUD | `src/ui/mod.rs`, `ui/hud.xaml` |
| Menu, título, morte | `src/ui/screens.rs`, `ui/{menu,title,death}.xaml` |
| View-models | `HudVm`, `ScreensVm` (`#[derive(Reflect)]` + `Bindable`) |
| Barra de fala/NLU | `world/say.xml` + `scripts/ui/say.lua` (ainda engine) |
| Mapa/viagem | `src/ui/map.rs` + `src/ui/waypoint.rs` (Rust; `ui/map.xaml`) |

- O HUD lê `VaelSave`, `VaelContent`, `UiData` e os binds de script; as
  barras são `ProgressBar` ligadas a frações e os chips/dock usam
  `Visibility`/`Opacity` bindados.
- O menu, o título e a morte têm **lógica Rust**: as linhas são construídas
  a partir de `VaelSave`/`VaelContent` (`quest_rows`, `bag_rows`,
  `skill_rows`, `shop_list_rows`, `forge_rows`, `a11y_rows`, `system_rows`)
  e as ações usam as regras já portadas (`shop_trade`, `craft_make`,
  `gear::toggle`, `add_xp`, `vault`) e escrevem as chaves partilhadas no
  `viber.game()` com `apply_patch`. Não há `dispatch` de Lua: os antigos
  `scripts/ui/{menu,journal,title,death}.lua` deixaram de ser carregados
  pelos roots (o `journal.lua` fica como ficheiro legado).
- `world/menu.xml`, `title.xml` e `death.xml` continuam no autoload apenas
  para o estado dos modais (`UiModalsOpen`/`MenusOpen`), `UiTabs` e para o
  `viber.ui.is_open("title"/"death")` dos scripts de jogo; os roots estão
  escondidos por `screens::hide_legacy_roots` e perderam `script=`.
- `VAEL_UI_PF=0` desliga a UI bevy_pf (o HUD declarativo de `world/hud.xml`
  foi removido; o autoload usa `world/say.xml` e o HUD é `ui/hud.xaml`).
- O título abre no arranque em `main`/`menu` (salvo `--entry dev`), fecha
  depois do load e reabre após as Definições; o Novo Jogo limpa o KV
  preservando `ui_scale`/`lang`/`a11y.*`/`vol.*`, incrementa `session_id`,
  fecha o título e troca para `main`. O ecrã de morte reage a
  `death_pending`/`hp <= 0` e tem Tentar de novo/Carregar/Título em Rust.
- Cache de slots: `SaveSlotsCache` relê 0..=4 a 0,5 s; `latest_slot` ignora
  a cena `menu` e escolhe o `saved_at` mais recente; o carregamento usa
  `SaveRequest` (`slot 4` → legado, `slot 0` autosave).
- Preferências: `apply_preferences` repõe `a11y.*`, `ui_scale`, `lang` e
  `vol.*` guardados no save em `A11y`, `UiUserScale`, `Locale` e
  `AudioMixerSettings`.

### Validação feita

- `cargo check` do Vael com o target em `/tmp/check-target`: o compilador
  já só reporta erros nos ficheiros em curso da sessão concorrente
  (`src/lighting.rs`, `src/content.rs` com `vael_content`); o módulo `ui`
  não tem erros.
- Instanciação headless dos três XAML (`XamlScene::parse` +
  `instantiate_document` + 5 `app.update()`): `warnings=0` em
  `menu.xaml`, `title.xaml` e `death.xaml`, e zero avisos de bindings com
  um view-model de teste.
- Testes puros novos para `fmt_playtime`, `latest_slot` e `load_label`.

### Próximo

- Barra de fala/NLU: o runtime (`viber.nlu`/`viber.decision`) só existe no
  Luau; portá-lo ou manter a barra na engine até lá.
- Barra de fala/NLU: o `viber.nlu`/`viber.decision` só existe em Lua; portar
  o runtime ou manter a barra na engine até lá.
- Profiler/showcase continuam ferramentas de dev da engine.

## Mapa e viagem em Rust

O mapa regional e a viagem rápida passaram para `src/ui/map.rs` +
`ui/map.xaml`:

- **Input**: M abre/fecha o mapa, G abre/fecha a viagem (só junto a uma
  fogueira; auto-fecha ao sair do raio), F assina o marco disponível mais
  próximo (3D, com `radius`), Esc fecha. Tudo bloqueado por `UiFocusedInput`
  (barra de fala), menu/título/morte/profiler, `MenusOpen.modal/shop/cinematic`
  e herói caído.
- **Estado**: `MapTravelState`; espelhado em `MenusOpen.map`/`.travel` para o
  `viber.menu_open()` e para o resto do gameplay parar.
- **Ações**: `pin-<nome>` escreve `pin` no `viber.game()`; `go-<nome>` resolve
  a posição em `TravelState`, teleporta com `teleport_landing` (exclui a
  entidade do marco) e assina com `report_visit` as visitas de quest.
- **Linhas**: `VaelTravelRules` + `TravelState.positions` + `VaelSave`
  (`mark.*`/`biome.*`); o mapa lista todos os disponíveis com posição, a
  viagem só os assinados. A imagem é `ui/vael_map.png` (o `AssetServer` do
  projeto tem a raiz do Vael), com os marcadores de jogador/waypoint em
  `Canvas.Left/Top` projetados por `TerrainRuntime.spec.world_size`.
- **Waypoint/marcadores**: `src/ui/waypoint.rs` publica `HudMarkers` em Rust
  (blips/beacons dos marcos, pin com limpeza a 4 m, e a seta da quest:
  pronta → dador; ativa → objetivo kill/visit/collect; sem quest → dador
  novo mais próximo). Hostis vêm de `Hostile` + `VaelScriptTags`, recursos
  de `Harvestable` e NPCs de `Name`.
- `world/explore.xml`, `scripts/ui/map.lua`, `scripts/game/travel.lua` e
  `travel_host.lua` foram removidos; o host `game-lua-travel` saiu de
  `world/gameplay.xml`. O diálogo "Perguntar por um lugar" usa
  `viber.vael.travel_pin`.

Validação: XAML de HUD/menu/título/morte/mapa instancia headless com
`warnings=0`; os testes cobrem `Margin` dos marcadores, troca de tab e
visibilidade das páginas. QA vivo (binário dev, `--scene main --entry dev`
e cena `menu`): HUD, diário (Missões/Mochila/Talentos), mapa e título/
créditos renderizam e respondem a cliques/teclas; capturas em
`/tmp/vael-*.png`. O pin do mapa foi validado ao vivo: o `HudMarkers`
publicado em Rust acendeu a seta/caption do destino e o marcador do mapa.
`cargo test --lib`: **275 passam**, 2 falhas de sessões concorrentes
(`lighting::tests::long_frame_renders_and_caps_the_debt` e
`speech::tests::a_spoken_line_becomes_a_world_bubble`); o `ui` passa todo. O motor ainda aborta de forma intermitente
com `bevy_mesh` "Mesh has been extracted to RenderWorld" e, noutro arranque,
`Out of Memory` do render — caminhos de LOD/terreno concorrentes, não o UI.

## Polimento da UI (2026-11-12)

- Menu: fundo sólido, cabeçalho com fio de ouro, tab ativa com fundo/linha
  própria (estado escrito por `sync_menu_pages`, porque `Visibility` bindado
  em `ScrollViewer`/conteúdo de `Button` não colapsava de forma fiável),
  páginas trocadas por `Node.display`, grelha de mochila em `WrapPanel` com
  ícones de item, botões com template de hover escuro, e linhas com largura
  fixa para o título/tag não ficarem centrados.
- Título: painel esquerdo sólido com fio vertical de ouro, logo maior,
  botão Continuar primário dourado, cartão de metadados do save e créditos
  centrados.
- Morte: cartão escuro, fio de brasa, botão Tentar de novo primário e
  cartão do último save.
- HUD: aro de ouro nos slots prontos (C/E/R), faixa de vida baixa, ícones e
  tooltips estáticos nos slots, faixas de dano sólidas.
- Mapa: moldura sólida, `Margin` por marcador (glow por marcador), legenda
  com “Destino: <label>”, lista e painel de viagem.

Lacunas conhecidas do bevy_pf (0.5.2): `LinearGradientBrush` em
`Border.Background`/`Grid.Background` não pinta no runtime (validado com o
ecrã de loading visível através do menu); `Visibility` bindado dentro de
conteúdo de `Button` e em `ScrollViewer` não colapsa — daí os sistemas
`sync_menu_pages`/nomes de destaque. Gradientes ficaram fora até haver
suporte/verificação.

## Skin Vael

A UI deixou de ser “WPF por omissão” e passa a ter uma identidade própria:

- **Paleta**: tinta verde-noite (`#0B0F0D`/`#0D1210`), pedra (`#141B18`),
  pergaminho (`#F2E8D5`), âmbar (`#E7C987`), âmbar fundo (`#A8854C`), folha
  (`#8FBF9F`), brasa (`#D9774F`) e sangue (`#C8564B`).
- **Tipografia**: títulos em **Noto Serif Display** (OFL, `ui/fonts/`, carregada
  no `Startup`), corpo em Fira Sans do `bevy_pf`. `load_skin_fonts` guarda os
  `Handle<Font>` no recurso `VaelSkinFonts`: o `Assets<Font>` do Bevy 0.20
  remove um font cujo handle forte morre no update seguinte, e sem isso a
  família nunca chegava ao `FontCx` (o XAML caía em Fira Sans).
- **Brasão vetorial**: escudo com a Árvore de Vael (tronco, ramos e folhas em
  `Path`, 92 px) no título; escudo + V (30/24 px) no menu e no mapa; espada
  partida no ecrã de morte. Divisor de losangos em `Path`; escudo do nível
  no HUD.
- **Ícones vetoriais**: `PackIcon` nas tabs (List, Grid, Star, Location,
  Settings, Menu, Eye, Info) e nos botões de título/morte (ArrowRight, Star,
  Settings, Info, Power, Refresh, Download, Home).
- O template de botões escuros ganhou hover âmbar; a mochila combina
  `WrapPanel` + ícones de item.
- **Cofre na UI**: `inventory_kv` usa as chaves `item.<id>` do `viber.game()`
  (as mesmas de `vault_get`/`shop_rows`/`craft_status`). Sem o prefixo, a
  mochila/loja/forja liam 0 para tudo; `patch_items` devolve as chaves já
  prefixadas, sem as voltar a prefixar.

Validação: todos os XAML instanciam headless com `warnings=0`, os testes
`ui::` passam (28) e a skin foi capturada ao vivo no mundo `qa-hud.bsn`
(título, HUD, menu/tabs, morte). O `qa-hud.data/terrain.ahgt` em falta é
esperado nesse mundo; o render pode abortar por OOM quando há builds
concorrentes.

# Migração do Vael para Rust/Bevy

Este documento é o estado da migração. O objetivo é o Viber ficar só com o que
o Bevy não tem (terreno, navegação, física/Rapier, meshopt, partículas, IK,
céu/água custom, autoria XML/editor) e o Vael passar a ser um crate Rust que
usa `bevy` diretamente. O Luau do jogo é transitório; os plugins do editor
continuam em Luau.

## Onde vive

- Worktrees isolados: `../refactor/Vael` e `../refactor/Viber` (a partir dos
  checkouts principais, que continuam a ter trabalho em curso).
- Branch: `refactor/bevy-native` nos dois.
- Checkpoint do Vael: `59a57ff`, tag `pre-bevy-migration`.
- Commits: Viber `92c1f42` (reflection/ECS/UI/save), `e79c4fc`
  (`player_camera`), `f259627` (AiMode), `5cf9d8f` (apply_patch),
  `5ec7b9d` (UiWriter); Vael `528149c` (controlos no Lua), `f462a23` (crate
  Rust), `1eabf60` (sync + HUD), `3b19050` (autoridade), `0808883` (HUD
  comportamental), `798a1f8`/`b59c924`/`2c2811c`/`fc586cd`/`6291c37`/
  `209f078` (cofre/XP/equipamento/quests/combate/relógio).
- Os checkouts principais **não** foram tocados. O principal do Vael tem uma
  regen de personagens e uma engine vivas; não mexer.

## O que mudou no Viber

- `src/reflect.rs`: `ReflectPlugin` regista 342 componentes/resources.
  `bevy_remote` passa a ver `Health`, `Transform`, `UiBar`, etc. O BRP expõe
  `world.query`/`world.get_resources` (não `bevy/query`).
- `src/luau/ecs.rs`: `viber.ecs.{get,set,insert,remove,query,spawn,despawn,types}`
  sobre reflexão, com whitelist. `set_scale`/`set_pose`/`set_visible` passam
  por lá.
- `AppOptions` ganhou `gameplay`, `player_camera`, `scripts: ScriptsMode`, `ui`
  e `profiler`; `build_app_with_world` permite ao jogo compor a App. O default
  preserva o comportamento do CLI/editor.
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
  `src/combat.rs`, `src/clock.rs`: cofre/caps e kit inicial, XP/níveis/
  talentos, equipamento/stats, ciclo de quests, guarda/dano/cura/i-frames e
  relógio (DayCycle/Weather) vivem em Rust. Uma ponte `viber.vael` instalada
  no `Startup` expõe as funções aos chamadores Lua; os wrappers guardam
  toasts, sons, `sync_collect`, `start_death` e o resto da apresentação.
- O jogo passa a **exigir** o crate `vael` em runtime: `viber.vael` tem de
  existir antes de `lib/rules.lua`/`lib/gear.lua`/`lib/clock.lua` correrem.

## Correr e validar

```bash
# a partir de ../refactor/Vael
CARGO_TARGET_DIR=../Viber/target cargo run --release -- \
  world.xml --scene main --entry dev --bridge 15706
```

- `--scene main` salta o menu/título; `--entry dev` salta o prólogo.
- O bridge fica em `127.0.0.1:15706`; `curl` com `viber.lua`, `world.query`,
  `world.get_resources`, `viber.ecs`.
- `VIBER_NO_BUILD=1 ./tools/check.sh` valida conteúdo/Lua/analyze/scene lint.
- `CARGO_TARGET_DIR=../Viber/target cargo test` no Vael: **147 testes**.
- `cargo test` no Viber: 1646 passam; a falha
  `luau::decision::tests::lua_api_intent_pt_multi_question` é modelo/ambiente.

## Estado e próximo

| Fase | Estado |
|------|--------|
| Fase 0 — checkpoint/baseline | feito |
| Fase 1 — reflection, ECS, AppOptions, widgets, culling, save | feito (profiler/atmosfera adiados) |
| Fase 2 — crate Vael, conteúdo, save, attach, arranque | feito |
| Onda 2a — state sync + HUD Rust | feito |
| Onda 2b — vitais/status/morte autoritativos em Rust | autoridade opt-in validada ao vivo; default off |
| Regras/hub — HUD, cofre, XP, equipamento/stats, quests, combate do jogador, relógio | feito em Rust com ponte `viber.vael` |
| Restante do jogo — FSM/inimigos/bosses, report/scan/collect de quests, viagem/marcos, colheita, loja/forja, diálogo/NLU, UI/story/áudio | pendente |
| Fase 4 — mover player/câmera/AI/vitais para o Vael; recipe extensions; docs | pendente |

## Dívidas conhecidas

- `VaelSave` é substituído inteiro pelo snapshot de 10 Hz; **não** gravar
  `game.vael` enquanto isto não for um merge (perde campos só do Rust).
- `VaelSave` não serializa no BRP por `BTreeSet` sem `ReflectSerialize`;
  usar `Vec` ou registar o type data antes de expor o save no bridge.
- O worktree Vael não tem LOD1/LOD2 (derivados, fora do git): os 404 de assets
  no smoke são esperados. Correr `tools/make_lods.py` ou copiar do checkout
  principal.
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
- `rules.lua` ainda tem `report_kill`, `scan_visits`, `sync_collect`,
  parley/decisão e o `award_kill`; `fsm.lua`/combate/inimigos, viagem,
  colheita, loja/forja, diálogo/NLU e UI continuam Lua.
- `rules.publish` ainda publica netplay/alvo/combo/`abilities.active`/
  `vitals.active`/`xp.recent`/`quest.recent`; os binds de vida/XP/economia/
  status/quest/chefe/quests.others já são do `VaelHud`/`HudUiPlugin`.
- O jogo passa a exigir o crate `vael` em runtime (`viber.vael`); `viber run`
  direto do checkout da engine já não serve para o mundo do Vael.
- `VAEL_RULES_RUST` continua opt-in; o QA de combate/interiores/save ficou
  pendente porque o Paint3D ocupa a GPU. O shadow (`VAEL_RULES_SHADOW=1`)
  fica como ferramenta de paridade.
- `own_system("abilities"/"hotbar"/"guard"/"bomb")` do Lua gera warnings de
  no-op desde a limpeza do Viber; desaparecem quando esses scripts migrarem.

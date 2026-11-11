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
- Commits: Viber `92c1f42` (reflection/ECS/UI/save) e `e79c4fc`
  (`player_camera`); Vael `528149c` (controlos no Lua), `f462a23` (crate
  Rust), `1eabf60` (sync de estado + HUD Rust).
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
- `CARGO_TARGET_DIR=../Viber/target cargo test` no Vael: 16 testes.
- `cargo test` no Viber: 1646 passam; a falha
  `luau::decision::tests::lua_api_intent_pt_multi_question` é modelo/ambiente.

## Estado e próximo

| Fase | Estado |
|------|--------|
| Fase 0 — checkpoint/baseline | feito |
| Fase 1 — reflection, ECS, AppOptions, widgets, culling, save | feito (profiler/atmosfera adiados) |
| Fase 2 — crate Vael, conteúdo, save, attach, arranque | feito |
| Onda 2a — state sync + HUD Rust | feito |
| Onda 2b — vitais/status/morte autoritativos em Rust | próximo |
| Ondas 3+ — combate, FSM/inimigos, quests/diálogo, viagem/economia, UI/story | pendente |
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
- O `rules.publish` ainda publica netplay/alvo/chefe/recargas/quests.others;
  esses binds migram com os donos (combate/FSM/UI).
- `own_system("abilities"/"hotbar"/"guard"/"bomb")` do Lua gera warnings de
  no-op desde a limpeza do Viber; desaparecem quando esses scripts migrarem.

# DELETE-MONOLITH — eliminar o `bevy_ext_viber_engine`

> **2026-11-15 (pós-CLI):** o CLI `viber` e o checkout `../viber-tools` foram
> aposentados — `analyze`/`lint`/`bsn`/`run`/`debug` vivem em `cli/`
> (`vael-tools`, ver [`TOOLS.md`](TOOLS.md)) e o bridge em
> `bevy_extensions` (`bevy_ext_bridge_protocol`/`_runtime`). O que resta no
> monólito são os métodos legacy `viber.*` (`lua`, `burst`, `trace`,
> `raycast`) e o runtime de gameplay.

Estudo de 2026-11-15 para a **eliminação completa** do crate
`crates/bevy_ext_viber_engine` (o monólito do runtime de gameplay). Sucede o
[`EXIT-MAP.md`](EXIT-MAP.md) (que mapeou peça→destino) e o
[`ARCH-REVIEW-STUDY.md`](ARCH-REVIEW-STUDY.md) (fases F0–F4): com as waves
A/B/C/E aterradas no `main` dos dois repos, o que resta é o inventário do
que ainda vive no monólito, o destino de cada peça e a ordem até
`rm -rf crates/bevy_ext_viber_engine`.

## Estado medido (2026-11-15, fim do dia — pós K/D/G-1)

| Métrica | Valor |
|---|---|
| LOC do monólito (integração `exit4/integrate`) | **105.520** (136.7k pós-água: −2k ecs, −27k terrain, −2.3k music/ai) |
| Distribuição | luau/ 15.0k (F em curso) · bridge/ 14.0k (H, sessão concorrente) · recipes/ 11.1k + scene/ 9.1k (G-2) · raiz ~50.4k (G-2) · profiler/ 3.1k · hud/ 2.3k · terrain/ **297** · nav/ 147 |
| Refs `viber::` no Vael (Rust) | 408 (maioria resolve a shims de crates) |
| Call-sites Lua | 1.807 em 106 namespaces (~113 fns + grupos compostos; **os scripts não mudam no port** — muda o host) |
| `bevy_ext_scene_runtime` | **APAGADO a 2026-11-15** (`3d11d27`) — órfão, gémeos reconciliados no monólito |
| Waves aterradas | A (composição+gémeos) · B · C · E + **avian-total** (`e952f0c`) · água · **K** (`737eabf`: ecs −2k, dlss, nav-mortos) · **D** (`exit4/integrate`: terrain, 291 testes) · **G-1** (music 20 testes, ai 21) |
| Fase H em curso | sessão concorrente: `viber_tools_runtime` → `bevy_ext_bridge_runtime` no monorepo, `cli/` novo no Vael, skills `vael-tools` |

## Execução 2026-11-15 (sessão de apagamento) — estado

- **K** ✅ `737eabf` no main: tabela `viber.ecs` + espelho + pool morrem
  (−2.011 lin); `patch_component`/`EcsValue` ficam (servem
  set_scale/set_visible/set_position); `dlss.rs` + `nav/{backend,tile}.rs`
  mortos. Falha do bridge "Static vs Fixed" = fallout avian pré-existente.
- **D** ✅ `exit4/terrain` (3 commits): terrain/ 27.6k → **540 lin** no
  monólito (shim + water_bridge + terrain_host); água/splat v7/shore_rocks/
  caves portados ao twin; 291/291 testes.
- **G-1** ✅ `exit4/g1` (2 commits): music.rs → `bevy_ext_audio` (twin em
  paridade; adaptador `audio_host.rs`), ai.rs+senses.rs → `bevy_ext_ai`
  (`ai_host.rs` publica o Ground sobre o TerrainRuntime); `LuaScriptRef`
  → core com o TypePath certo (`viber::luau::components`); 41 testes.
- **F2–F5+F8** ✅ `exit4/luau` (7 commits): `bevy_ext_luau` 2.4k → **8.0k**
  (contratos de scripting no core — fecha o ciclo ui↔luau; núcleo+game
  UNIFICADO com bump de geração; percepção/props com seam `SurfaceQueries`;
  efeitos/áudio; nlu/decision com features CelerAI; wrappers de compat
  apply_patch/game_to_json para o Vael). Monólito luau/ ~13.8k (host vivo).
- **Integração** ✅ `exit4/integrate` @ `dbf0e0f` = K+D+G-1+F mergeados;
  workspace check ✓, BMS 68 testes, monólito lib ✓. **FF no main do
  monorepo quando o WIP da sessão concorrente aterrar.**
- **H em curso** pela sessão concorrente: `bevy_ext_bridge_{runtime,protocol}`
  no monorepo, `cli/` no Vael, `scene_runtime` APAGADO (`3d11d27` — os
  gémeos spawn/recipes/scene_loader ficaram reconciliados NO monólito; a
  G-2 deles precisa de novo destino).
- **F6/F7** ✅ `0b68972`+`15bc610` no main: a superfície Lua do BMS está
  COMPLETA (IA/combate/cinemática/save/terreno/fala/UI/sinais — 14 comandos
  novos + ~20 closures com a validação do monólito; conversor Lua→JSON para
  args de sinais). Falta o tick de timers (`after`/`every`), que é parte da
  INTEGRAÇÃO do host.
- **G-2** ✅ `exit5/g2` (9 commits, monólito 100.5k → **86.2k**): worldsys,
  animation (núcleo), cinematic, ambient repartido, postfx runtime, hud →
  engine/postfx/audio. `scene/` (9.1k) e o grupo spawner/recipes/
  scene_loader (~17.5k) ficaram para a última onda (bloqueados por
  physics/luau/contratos — documentado no relatório do agente).
- **INTEGRAÇÃO do host** ✅ **PASSOU** (`exit5/F-int` monorepo `c6acb2a`+
  `d595f62`, Vael `218423e`; 2026-11-15): o host BMS corre os scripts do
  jogo. As 3 peças da lista abaixo: (1) espelho `LuaScriptRef` →
  `ScriptComponent` (attach `EntityScript(entity, handle)`) no
  BmsBridgePlugin, com remoção no streaming unload; (2) asset source —
  **nada foi preciso**: o leitor multi-root do meshopt
  (`register_asset_source`) já resolve `scripts/<path>` contra o
  `world_dir`, primeira asset root; (3) dispatch `on_update(dt)`
  (`send_on_update` + `event_handler` no set `VaelScripts`, antes do
  dreno) com o seed de timers via `ContextPreHandlingInitializer`
  (`seed_script_identity`). Sinais entregues como callbacks BMS; os
  instaladores `viber.vael` dos plugins do jogo correm por contexto novo
  via registo (`script_host.rs` no Vael + `add_context_installer` no
  crate). Fixes ao vivo que a troca destapou: `viber.ui` nil nos
  contextos (o snapshot do `UiScriptState` ia no build, antes do
  `UiPlugin` — adiado para `Startup`; shop.lua rebentava ~883k×/boot) e
  `viber.profiler` nunca instalado (exigia `ResMut<LuaScriptHost>` —
  agora via o registo, Arcs partilhados com o recurso). QA vivo (15714):
  36 lin `viber::luau`/boot (slimes FSM), boot sem panics, wait-ready OK,
  KeyW move o herói, HUD/script-UI vivo (fogueira/quests/hotbar), F3
  (FRAMES da engine) com dados reais — entidades 89k, **scripts 473**.
  **Falta ainda**: (4) publisher do `HostWaterBodies` (o áudio de água
  ambia com warn por frame — `bevy_ext_audio::ambience` exige o
  recurso); (5) adapter BRP do REPL `viber.lua` (`qa.py`/`debug
  state`/`until`/`expect` morreram com a VM); `qa.rs` (god/snapshot de QA
  escrevia globals na VM); o fallback Lua das visitas
  (`rules.scan_visits_lua`) e o registo no `lib/actions.lua` viviam na VM
  partilhada; `cargo test --lib` do Vael com 2 falhas em `player.rs` de
  WIP concorrente (não é da troca). **O `luau/` vivo continua no monólito
  `viber run`** (host.rs/api.rs/runtime.rs/ctx.rs/… + bridge/lua.rs REPL +
  hot_reload): a deleção exige re-rotear ~29 consumidores externos, ≥7
  deles em WIP concorrente a_terrar (scene_loader, player, recipes,
  menus, input_map, gltf_residency) + o binário da própria engine —
  fica para quando esses ficheiros commitarem (ver o mapa de
  consumidores no relatório do agente).

## Achados do inventário (2026-11-15, módulo-a-módulo)

- **`viber::terrain_host` ainda NÃO EXISTE** — o `src/app.rs` do Vael chama
  `viber::terrain_host::{water_surface_config_from_world,terrain_chunk_config_from_world}`
  (a wave D/água vai criá-lo; as funções hoje vivem em
  `terrain/{water_material,layer_material}.rs`). O `main` do Vael só
  compila quando a sessão de água o aterrar.
- **Código morto compilável-off**: `nav/{backend,tile}.rs` (802 lin — o
  `nav/mod.rs` já é shim e não os declara), `dlss.rs` (a feature `dlss`
  não está no `[features]`), `scene/live.rs` (reformado). Morrem sem port.
- **[obsoleto 2026-11-15] O ciclo travava ~22k lin de gémeos**: o
  `bevy_ext_scene_runtime` tinha cópias prontas de
  animation/attachment/hud/interact/meshopt/particles/recipes/scene_loader/
  spawn_gate/streaming — o crate foi **APAGADO** (órfão, zero consumidores)
  e o monólito voltou a ser a única cópia; a saída é directa para os crates
  de domínio na G.
- **`reflect.rs` (310) é o ÚLTIMO a sair** — regista no BRP os tipos de
  todos os módulos; sai quando o último módulo sair.
- 0 refs do Vael: dialogue, dlss, fx_specs, glb_bounds, hot_reload, lint,
  placement, prune, scaffold, senses, script_kind, vinst, rng, audit, bake,
  bsn_cli, cinematic_cli — a maioria é bin-only ou morre direto.

## O crate-hub morreu antes de receber (2026-11-15)

O `bevy_ext_scene_runtime` — o "hub" das waves A–E, onde spawn/
scene_loader/cinematic/hud iam aterrar — foi **apagado**: era órfão
(nenhum `Cargo.toml` o dependia; o jogo vivo corre nas cópias gémeas do
monólito, reconciliadas em `f565750`) e moldava o runtime ao DSL XML
aposentado. Decisão do dono: **o formato manda** — a carga de mundo é
`.bsn`/Jackdaw (`bevy_ext_world::bsn`: `WorldNode` + specs tipadas do
`bevy_ext_scene` + `IsA` + `WorldOrigin`/`SceneNodeId`) e os pedaços de
runtime saem do monólito DIRECTOS para os crates de domínio nas fases
D/F/G. Nenhum crate novo volta a nascer moldado ao DSL; o sumário e as
fases em baixo já reflectem isto.

## Sumário por destino (LOC calibrado — soma exacta 136.671)

| destino | LOC | conteúdo |
|---|---|---|
| `bevy_ext_terrain_runtime` | **27.592** | terrain/ inteiro [WIP-água] + o glue `terrain_host` novo |
| `bevy_ext_engine` | **22.545** | app-glue, cinematic, worldsys, ambient, sky, spawner, water_ssr, player, session, profiler/, hud-glue, menus, ibl, physics_fx, prop_tint, graphics, editor_camera, netplay, frame_pacing, foliage, decal_fx, draw_diag, probes, impact, time_fx… (maioria JÁ tem twin lá) |
| `bevy_ext_luau` | **16.405** | luau/ 15.947 + hot_reload — o último grande bloco, destrava o resto |
| distribuição G (sem hub — o `bevy_ext_scene_runtime` foi apagado a 2026-11-15) | **21.968** | recipes/→`bevy_ext_world` (formato bsn/Jackdaw) + spawn; scene_loader/hud→`bevy_ext_engine`; spawn_gate→`bevy_ext_physics`; attachment→`bevy_ext_gltf`; particles→crate pequeno; interact/streaming/animation/placement por domínio na G |
| `viber-tools` (debug) | **14.056** | bridge/ inteiro (`viber.lua` REPL adapter incl.) |
| **morre** | **12.588** | main.rs, lib.rs, audit, bake, bsn_cli, prune, scaffold, dlss, projectile, nav/{backend,tile}, scene/live |
| `bevy_ext_world` | **10.301** | scene/ (autoria; só `doc::SceneDoc` é do jogo), lint, project, vinst, xml |
| `bevy_ext_postfx` | **5.192** | postfx, adaptive_quality, postfx_toggle |
| `bevy_ext_physics` | **2.661** | physics (KCC — avaliar morrer), ragdoll |
| `bevy_ext_ai` | **1.379** | ai, senses |
| `bevy_ext_audio` | **946** | music (cópia divergida) |
| `bevy_ext_save` | **713** | save (partes bin-only morrem) |
| shims finais (gltf/core/world/nav) | ~330 | apagar as linhas |

**Ordem de desbloqueio pelo grafo:** (1) **luau → `bevy_ext_luau`** —
destrava a saída de engine/postfx/audio/nav; (2) **reflect**
para o fim (reflete tipos de todos); (3) **terrain** termina com a WIP de
água (inclui criar o `terrain_host` que o Vael já referencia); (4)
**bridge + bin** morrem juntos por último (o adapter BRP do `viber.lua`
em `viber_tools_runtime` vem ANTES — o `qa.py` depende).

## Os três bloqueios e a ordem

1. **WIP de água (território da sessão concorrente)** — `terrain/` (27.6k),
   `water_ssr.rs` (1.3k) e `postfx.rs` (4.3k, re-misturado no merge) estão a
   ser editados AGORA. Nada se move daqui até a água commitar (wave D).
2. **O host Luau** — enquanto o monólito correr os scripts do jogo, os
   plugins `luau::LuauScriptPlugin`, `luau_update`, a tabela `viber.*` e os
   13 ficheiros Rust que agarram o handle `host.lua` cru não saem. É a
   **Fase F** (abaixo), a maior.
3. **O binário `viber run` + CLI** — `app.rs` (1.19k) + `bridge/` (14k) +
   `session.rs` (742) + `profiler/` (3.1k) + as tools editor-only
   (`bake`, `bsn_cli`, `lint`, `cinematic_cli`, `audit`, `scaffold`,
   `prune`, `placement`, `glb_bounds`) só morrem quando o jogo deixar de
   compilar o monólito E o `viber.lua` (REPL na VM, usado pelo `qa.py`)
   tiver adapter BRP no host novo.

## Fase D — terreno/água (bloqueada na água; dono: sessão concorrente)

`terrain/` inteiro → `bevy_ext_terrain_runtime` (contratos do `host.rs` já
documentados lá): `water.rs` fork 3.6k, `runtime.rs` 3.1k, `roads.rs` 3.1k
(já trocado na C? verificar no merge), `splat.rs` 1.9k, `plugin.rs` 1.7k,
`cliffs.rs` 1.5k, voxel/. O `water_ssr.rs` (1.3k) → `bevy_ext_postfx` (o Vael
tem os reflexos cortados no `config.yaml`; o crate decide se portas ou
aposentas). Gate: `cargo test -p bevy_ext_terrain_runtime` (289 baseline) +
QA de água/nado/vau vivo.

## Fase F — host Luau → `bevy_ext_luau` (o desbloqueio de tudo)

O port que mata o monólito. 15.9k lin, com o `bevy_ext_luau` (BMS 0.24) já a
correr 23 fn. Superfície a portar por onda de call-sites (do maior para o
menor risco):

| Onda | Superfície | Call-sites | Conteúdo |
|---|---|---|---|
| F1 | `viber.ecs` | **0** | **MORRE** — apagar `ecs.rs` (2.1k) + gate `ecs_mirror` no Vael (`src/main.rs`) e no app.rs |
| F2 | `viber.ui.*` | 256 | a camada de dados é do `bevy_ext_ui` (C já trocou); portar `ui/script.rs`→BMS API (`set/set_text/is_open/list/get/read/focused/clicked/open/create`) |
| F3 | núcleo do script | ~500 | `log/time/position/set_position/distance_to_player/player_position/player_forward/player_hp/scene_name/scene_entry/biome/load/toast/state` (o `game.rs`/`ctx.rs` do BMS já tem metade) |
| F4 | percepção/props | ~200 | `nearby/player_nearby/hostiles/ground_below/road/road_at/on_road/in_water/find/face_player/face_towards/prop/attach/detach/set_visible/set_pose/set_scale/decal/burst/spawn_prototype/own_system/own_action/hit_flash/topple/corpse` |
| F5 | efeitos/áudio | ~250 | `play_clip/sound/sound_loop/sound_stop/music/audio/shake/fov_kick/hit_stop/damage_number/ring/punch/kick/lantern/gesture/camera_free/camera_pose/camera_state/bone_offset/clip_duration` |
| F6 | interação/IA/combate | ~180 | `set_interaction/interact_range/interacted/sense/wander_target/next_state/set_locomotion/radial_damage/entity_*/fire_projectile/push_player/knockback` |
| F7 | save/câmara/otros | ~120 | `save/save_slot/load_save/load_slot/save_slots/saved_at/slot/quit/locale/localize/prompt/say/prompt/menu_open/net_role/title/playtime/level/gold/markers/home` |
| F8 | `viber.vael` + runtimes IA | 80+43 | é do PRÓPRIO jogo (`src/vault.rs:205`, `qa.rs`) — move com o jogo; `nlu.rs`/`decision.rs` (1.2k) portam com features `nlu`/`decision` no `bevy_ext_luau` (mesma path-dep CelerAI) |
| F9 | testes | — | portar `tests.rs` (2.860 lin) por onda (a rede de segurança); exports.rs (1.1k, `viber.debug.*`) morre com o binário |

**Rust lado-a-lado:** as 62 refs `viber::luau` do Vael (`host::LuaScriptHost`
~19, `game::{apply_patch,game_to_json,json_to_game}`, componentes,
`luau_update`, ownership) religam às equivalentes BMS
(`game_snapshot`/`publish_game_patch` existem; o handle cru mlua morre —
o BMS gere contextos por entidade). **O crate-hub `bevy_ext_scene_runtime`
foi apagado (2026-11-15)** — os gémeos morreram com ele e o monólito é a
fonte única (`AuthoredShadowLight`,
`player::{hero_collider,hero_controller_bundle,landing_position,
player_movement}`, os re-exports `pub use viber::{…}`). Na F os tipos do
player vão para o core e o `AuthoredShadowLight` para o postfx/engine; os
módulos que o hub ia receber saem daqui DIRECTOS para os crates de domínio
na G (a via dos swaps de 1 linha morreu com o crate).

Gate por onda: `rg 'viber\.[a-z]+' scripts/` a descer + `cargo test
-p bevy_ext_luau` a subir + QA vivo dos scripts tocados.

## Fase G — o resto do runtime-vivo

Destinos calibrados pelo inventário (a tabela completa por módulo está no
sumário acima); os ajustes vs. o rascunho de 2026-11-14:

- `cinematic.rs` (1.583) → **`bevy_ext_engine`** (twin lá; a via
  scene_runtime morreu com o crate apagado — destino directo).
- `physics.rs` (2.138, KCC) → **reavaliar na hora**: o monólito já tem a
  versão avian e `spawner.rs`/`water_bridge.rs` podem consumi-la; morre só
  se o jogo confirmar que não regista o `PhysicsPlugin` da engine.
- `hud/` (2.311) → **`bevy_ext_engine`** (hud-glue; o hub
  `bevy_ext_scene_runtime` foi apagado a 2026-11-15), não "morre com
  bevy_pf" — o HUD do mundo é autoria `.bsn`, diferente do HUD XAML do
  jogo. O Vael usa `spawn_hud`/compass/minimap/waypoint/`HudMarkers`.
- `scene/` (9k) → **`bevy_ext_world`** (doc/sync/tools/ops/…), exceto
  `live.rs` (morre, reformado) e `cli.rs` (morre com o bin).
- `bridge/` (14k) → **`viber-tools`** (o crate de debug), não morre: é o
  servidor de debug que o Vael liga (`BridgePlugin`, `log_plugin_with_bridge`,
  `port_alive`).

## Fase H — o apagamento

Ordem final (cada passo com o jogo a compilar):

1. `viber.lua` adapter BRP no `viber_tools_runtime` (repo viber-tools; o
   `qa.py` deixa de precisar do binário).
2. Vael: apagar a dep `viber` do `Cargo.toml` (os últimos re-exports de
   helpers: `load_world`, `world_base_dir`, `world_asset_dirs`, `RunMode`,
   `scripts_enabled` → core/engine).
3. Monorepo: `cargo tree -i viber` **vazio**; remover
   `crates/bevy_ext_viber_engine/` (o vendor/rerecast já foi upstream-deciso
   ou migra para o `bevy_ext_nav`); apagar `main.rs`/bins.
4. QA vivo completo (protocolo da memória: boot/andar/save/morte/respawn/
   interiores/sessão longa + água/nado + criaturas a perseguir).
5. TypePaths de wire congelados continuam nos crates
   (`viber::menus/music/save/project/terrain::delta`, `bevy_world_xml`,
   `bevy_engine_core`) — a linha 0.x não parte saves/BRP/editor.

## O que o monólito deixa de ser

O monólito NÃO é "a engine" — é o **conjunto de plugins de gameplay** que o
`src/app.rs` do Vael compõe + o binário de ferramentas. Tudo o que o jogo
usa tem casa no grafo `bevy_ext_*` + Jackdaw + Bevy 0.20. Depois da Fase H,
o `bevy_extensions` fica só com os pacotes publicáveis; o binário `viber
run` (bake/audit/lint/prune/bsn_cli/scaffold/cinematic_cli, ~12.6k) morre —
a autoria é o Jackdaw + `.bsn`.

## Riscos

- **A água e o postfx re-misturados**: mover `terrain/` antes da água
  aterrar = conflito garantido; a wave D espera.
- **O host Luau é a peça com mais chamada por frame** (Luau no hot path);
  o port BMS muda a gestão de contextos (por entidade) — perf a validar com
  `viber debug prof` A/B.
- **`viber.lua`/qa.py**: sem o adapter BRP, o QA morre antes do monólito —
  F0 do apagamento, não opcional.
- **Fork-remanescente (obsoleto 2026-11-15)**: com o
  `bevy_ext_scene_runtime` apagado, o monólito voltou a ser a única cópia de
  spawn_gate/interact/attachment/streaming/… — editar directamente. A regra
  "não editar o gémeo" volta a valer quando a wave D/F criar novos gémeos.

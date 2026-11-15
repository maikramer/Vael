# DELETE-MONOLITH — eliminar o `bevy_ext_viber_engine`

Estudo de 2026-11-15 para a **eliminação completa** do crate
`crates/bevy_ext_viber_engine` (o monólito do runtime de gameplay). Sucede o
[`EXIT-MAP.md`](EXIT-MAP.md) (que mapeou peça→destino) e o
[`ARCH-REVIEW-STUDY.md`](ARCH-REVIEW-STUDY.md) (fases F0–F4): com as waves
A/B/C/E aterradas no `main` dos dois repos, o que resta é o inventário do
que ainda vive no monólito, o destino de cada peça e a ordem até
`rm -rf crates/bevy_ext_viber_engine`.

## Estado medido (2026-11-15, pós-merges)

| Métrica | Valor |
|---|---|
| LOC do monólito | **136.671** (era 162,5k no exit; −26k em shims/ports) |
| Refs `viber::` no Vael (Rust) | **348** (era 257 — a composição própria passa a citar mais módulos, mas a maioria resolve a shims de crates) |
| Módulos citados pelo `src/app.rs` do Vael | 55 (deles: 5 já só re-export de crates — config/audio_space/ik/render_lod/terrain_host; meshopt/project/textures/xml shims de 2–12 lin) |
| Call-sites Lua | 1.807 em 106 namespaces (superfície: ~113 fns em `luau/api.rs` + grupos compostos input/events/timers/entity/game/props/terrain + `viber.ui.*` 256 + `viber.vael` 80 do próprio jogo) |
| Ponte `bevy_ext_scene_runtime` → monólito | **3 lin** (era 18 módulos) — morre com o luau |
| `luau/` no monólito | **15.947** (24 ficheiros; testes 2.860) |
| `terrain/` no monólito | **27.592** — WIP de água da sessão concorrente aterra aqui primeiro |
| `bridge/` + `session.rs` + `profiler/` | ~17.9k — infra do binário `viber run`/CLI |
| Waves já aterradas | A (composição própria + gémeos), B (config/audio_space/spawn_gate avian/attachment/interact), C (nav/postfx-glue/spawn re-sync/**ui atómica**), E (physics a avian3d) |

## Achados do inventário (2026-11-15, módulo-a-módulo)

- **`viber::terrain_host` ainda NÃO EXISTE** — o `src/app.rs` do Vael chama
  `viber::terrain_host::{water_surface_config_from_world,terrain_chunk_config_from_world}`
  (a wave D/água vai criá-lo; as funções hoje vivem em
  `terrain/{water_material,layer_material}.rs`). O `main` do Vael só
  compila quando a sessão de água o aterrar.
- **Código morto compilável-off**: `nav/{backend,tile}.rs` (802 lin — o
  `nav/mod.rs` já é shim e não os declara), `dlss.rs` (a feature `dlss`
  não está no `[features]`), `scene/live.rs` (reformado). Morrem sem port.
- **O ciclo trava ~22k lin de gémeos**: `bevy_ext_scene_runtime` tem cópias
  prontas de animation/attachment/hud/interact/meshopt/particles/recipes/
  scene_loader/spawn_gate/streaming — o swap de 1 linha de cada espera a
  morte do `luau/` do monólito (32 ficheiros usam `crate::luau`).
- **`reflect.rs` (310) é o ÚLTIMO a sair** — regista no BRP os tipos de
  todos os módulos; sai quando o último módulo sair.
- 0 refs do Vael: dialogue, dlss, fx_specs, glb_bounds, hot_reload, lint,
  placement, prune, scaffold, senses, script_kind, vinst, rng, audit, bake,
  bsn_cli, cinematic_cli — a maioria é bin-only ou morre direto.

## Sumário por destino (LOC calibrado — soma exacta 136.671)

| destino | LOC | conteúdo |
|---|---|---|
| `bevy_ext_terrain_runtime` | **27.592** | terrain/ inteiro [WIP-água] + o glue `terrain_host` novo |
| `bevy_ext_engine` | **22.545** | app-glue, cinematic, worldsys, ambient, sky, spawner, water_ssr, player, session, profiler/, hud-glue, menus, ibl, physics_fx, prop_tint, graphics, editor_camera, netplay, frame_pacing, foliage, decal_fx, draw_diag, probes, impact, time_fx… (maioria JÁ tem twin lá) |
| `bevy_ext_luau` | **16.405** | luau/ 15.947 + hot_reload — o último grande bloco, destrava o resto |
| `bevy_ext_scene_runtime` | **21.968** | recipes/, scene_loader, particles, animation, interact, attachment, streaming, spawn_gate, placement, hud/ — gémeos prontos, swap travado pelo luau |
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
destrava os swaps de engine/scene_runtime/postfx/audio/nav; (2) **reflect**
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
o BMS gere contextos por entidade). **A ponte do `scene_runtime` (3 lin:
`AuthoredShadowLight`, `player::{hero_collider,hero_controller,
landing_position,player_movement}`, `pub use viber::{...}`) morre na F** —
os tipos do player vão para o core; o `AuthoredShadowLight` para o
postfx/engine. É isto que desbloqueia os swaps de 1 linha armados na B
(spawn_gate/interact/attachment/streaming/config no scene_runtime).

Gate por onda: `rg 'viber\.[a-z]+' scripts/` a descer + `cargo test
-p bevy_ext_luau` a subir + QA vivo dos scripts tocados.

## Fase G — o resto do runtime-vivo

Destinos calibrados pelo inventário (a tabela completa por módulo está no
sumário acima); os ajustes vs. o rascunho de 2026-11-14:

- `cinematic.rs` (1.583) → **`bevy_ext_engine`** (twin lá; o scene_runtime
  estava bloqueado pelo ciclo — com o luau morto, ambos resolvem).
- `physics.rs` (2.138, KCC) → **reavaliar na hora**: o monólito já tem a
  versão avian e `spawner.rs`/`water_bridge.rs` podem consumi-la; morre só
  se o jogo confirmar que não regista o `PhysicsPlugin` da engine.
- `hud/` (2.311) → **`bevy_ext_scene_runtime`** (twin pronto; o Vael usa
  `spawn_hud`/compass/minimap/waypoint/`HudMarkers`), não "morre com
  bevy_pf" — o HUD do mundo é autoria `.bsn`, diferente do HUD XAML do
  jogo.
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
- **Fork-remanescente**: os módulos "shim pendente do ciclo" (spawn_gate/
  interact/attachment/streaming no monólito) têm cópias vivas até à F — não
  editar delas (só a cópia do crate).

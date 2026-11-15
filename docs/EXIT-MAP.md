# EXIT-MAP — saída do monólito `bevy_ext_viber_engine`

Estado vivo do mapeamento **peça → destino → gap → onda** para o fim do
monólito (162,5k LOC). Dono deste doc: o plano de saída; o estado da
migração fica no [`MIGRATION.md`](MIGRATION.md), as fases F0–F4 no
[`ARCH-REVIEW-STUDY.md`](ARCH-REVIEW-STUDY.md), as ondas/pacotes no
`bevy_extensions/docs/PACKAGES.md`. Actualizado a 2026-11-14.

## Decisões (locked-in, 2026-11-14)

1. **avian3d é o keeper** (o jogo corre avian+tnua; o monólito migrou
   spawn_gate/voxel a avian; `bevy_ext_water` já nasceu avian): as cópias
   extraídas da era Rapier (`bevy_ext_physics`, spawn_gate no
   scene_runtime, voxel spawn) **portam-se a avian**; o `physics.rs`
   (KCC Rapier) do monólito morre — só `replace_with_stump_collider`
   migra (3 refs no Vael).
2. **`viber.scene --live` e `tool --live` reformam-se** — o Jackdaw tomou
   a edição viva (`src/editor_link.rs`, `src/live_edit.rs`,
   `jackdam_remote`, painel Vael: Cinemáticas). **`viber.lua` mantém-se**
   via adapter BRP sobre o host novo (o `qa.py` usa Lua pesadamente);
   profiler segue o `viber_tools_runtime`.
3. **TypePaths de wire congelados para sempre** na linha 0.x — vivem nos
   crates, não no monólito: `viber::menus`, `viber::music`, `viber::save`,
   `viber::project`, `viber::terrain::delta`, `bevy_world_xml::bsn/xml`,
   `bevy_engine_core`.
4. **Wave A = branch `exit2/app-assembly`** (composição própria da App no
   `src/app.rs`, gémeos unificados, 335 testes + QA vivo: fumo, morte com
   modal e respawn pelo botão, interiores, sessão longa). Aterra quando o
   WIP de água da sessão concorrente commitar — runbook na memória
   (`vael-viber-exit.md`): monorepo main ← integrate2/all ← água, depois
   Vael main ← exit2/app-assembly (stash AGENTS.md/player.rs;
   modify/delete de lighting/attach/ui_bind resolvem-se como DELETE).

Padrão por peça: **extrair → re-sync do fork (o que o monólito tem à
frente vai PARA o crate) → re-export swap no monólito (`pub use`) →
trocar imports no jogo → apagar**. Módulos de runtime trocam-se
ATOMICAMENTE num commit (o incidente da UI de 2026-11-14: troca
incremental de gémeos parte o jogo em silêncio).

## O mapa

| Peça (monólito) | Destino | Gap | Onda | Estado |
|---|---|---|---|---|
| app.rs `build_app` (1.162) | morre — `src/app.rs` do Vael | aterrar branch exit2 | **A** | ✅ validada, bloqueada na água |
| gémeos vitals/input_map/a11y/locale/dialogue/camera-glue/save-bridge | core/engine/save | só aterrar | **A** | ✅ em `integrate2/all` (A1) |
| streaming.rs | scene_runtime | fork ao nível; swap bloqueado no ciclo | **B** | ✅ fork pronto (`af65a86`) |
| config.rs (594) | **`bevy_ext_engine::config`** | nenhum — shim COMPLETO nos dois lados | **B** | ✅ (`66d9224`) |
| interact.rs (188) | scene_runtime | re-sync feito; swap no ciclo | **B** | ✅ (`77eb1b0`) |
| spawn_gate.rs (422, monólito avian) | scene_runtime | **port avian feito**; swap no ciclo | **B** | ✅ (`10a62f9`) |
| save `apply_save` | — só-binário | morre na H | **B** | ✅ documentado (`6365a4b`) |
| attachment (288) | scene_runtime (`GltfScenePending`) | extraído; swap no ciclo | **B** | ✅ (`1c29919`) |
| audio_space (291) | `bevy_ext_audio` | nenhum — shim COMPLETO | **B** | ✅ (`e1173c2`) |
| hud DISPLAY_FONT (313) | engine `hud_assets` | já feito (A3) | **B** | ✅ |
| input_map `gather_actions` | morre com `ExternalActionState` | apagar na aterragem da wave A | **B** | ⏳ |
| postfx fork (349) | `bevy_ext_postfx` | port semântico + shim | **C** | 🔄 |
| nav fork (72) | `bevy_ext_nav` | re-sync + shim | **C** | 🔄 |
| roads (4) / textures | `bevy_ext_terrain_runtime` | re-sync + shim | **C** | 🔄 |
| recipes/spawn.rs (633 à frente: avian, 113 gradmesh) | `bevy_ext_scene_runtime` | port + swap (no ciclo) | **C** | 🔄 |
| ui data layer (UiData/UiWriter/UiScriptBinds/UiModalsOpen/UiFocusedInput) | `bevy_ext_ui` | **troca ATÓMICA** — só 234 lin de diff, zero ficheiros só-no-monólito | **C** | 🔄 |
| ui presentation (~18k tree/style/tween) | morre (bevy_pf é o dono) | nenhum no jogo | **H** | — |
| terrain runtime + water fork (1.275) + cliffs/shore + voxel avian | `bevy_ext_terrain_runtime` + `bevy_ext_water` (já avian) | portar pós-água; contratos do `host.rs` | **D** | 🚫 bloqueada na água |
| physics.rs KCC (2.052) | morre (jogo é avian+tnua) | salvar `stump_collider`; monólito tem `bounds_cuboid` errado | **E/H** | ✅ fix no crate |
| bevy_ext_physics (Rapier-era) | **port a avian3d 0.8** | feito | **E** | ✅ (`exit3/avian`, 50/50) |
| luau host (16.1k) | `bevy_ext_luau` (BMS) | famílias por call-sites (ui 256, load 149, toast 108, game 96, sound 87, prop 62, burst 53, interact 49…); `tests.rs` (2.8k); 13 ficheiros do Vael largam o `host.lua` cru | **F** | ⏳ |
| `viber.ecs` mirror (2.1k) | **morre** (0 usos no jogo) | remover módulo + gate `ecs_mirror` | **F** | ⏳ |
| `viber.lua` REPL | adapter BRP no `viber_tools_runtime` | eval na VM do host novo | **F** | ⏳ |
| `viber.scene --live` / `tool --live` | **reformados** (Jackdaw live-edit) | — | **F** | decisão ✅ |
| **ponte do scene_runtime ao monólito (18 módulos)** | morre com o luau | **desbloqueia TODOS os swaps de B/C** | **F** | 🔑 |
| worldsys drivers (daycycle/sun/weather/atmosphere/interior) | `bevy_ext_engine::worldsys` (já lá) | UM dono p/ `AtmosphereState`/`BiomeRegions` (postfx::env vs engine); Vael troca 7 refs | **G** | ⏳ |
| ambient.rs (1.665) | repartir: SFX→audio, névoa→postfx/elements, sombras→bevy_lights, gestos→animation | esvaziar | **G** | ⏳ |
| music.rs cópia (946) | `bevy_ext_audio::music` | apagar cópia | **G** | ⏳ |
| ai/senses (`features=["ai"]`) | `bevy_ext_nav` | trocar a feature para o crate | **G** | ⏳ |
| `drive_player_animation` | morre (jackdaw AnimationSet) | `d8d4db7` gestos sem rig já no main | **G** | ⏳ |
| bridge legacy / session.rs | `viber_tools_core` / morre com o binário | — | **H** | — |
| dep `viber` no Cargo.toml + arquivar monólito | — | gate `cargo tree -i viber` vazio | **H** | 🚫 pós A+D+F |

Paralelos baratos: **F1** publicar a família 0.1.0 (token crates.io);
**F2** virar `VAEL_HARVEST_RUST`/`VAEL_TRAVEL_RUST`/`VAEL_QUEST_EVENTS_RUST`
a default com a infra `qa.py` nova e apagar os caminhos Lua.

## Ondas e estado de execução

- **A** — aterrar `integrate2/all` + `exit2/app-assembly` (bloqueada no
  WIP de água do monólito; runbook na memória).
- **B** ✅ (`exit3/wave-b`, 7 commits) — streaming (forks ao nível),
  config → **`bevy_ext_engine::config`** com shim COMPLETO no monólito e no
  scene_runtime (um só `GameConfig` hoje), interact re-sync, spawn_gate
  **portado a avian** no scene_runtime (espelha o WIP do monólito),
  attachment extraído, audio_space → `bevy_ext_audio` com shim COMPLETO,
  save `apply_save`/`SavePlugin` documentados só-binário (morrem na H),
  hud font confirmado (A3). Gates: workspace **1081/0**, monólito
  **1010/3-fixtures**, scene_runtime 63/0.
- **C** 🔄 (`exit3/wave-c`) — nav/roads/textures/postfx(349 semântico)/
  spawn(633 avian+gradmesh)/**ui data layer atómica** (234 lin de diff,
  zero ficheiros só-no-monólito).
- **E** ✅ (`exit3/avian`, 3 commits) — `bevy_ext_physics` portado a
  **avian3d 0.8**, 50/50 testes; apanhou divergência real do monólito
  (`bounds_cuboid` a metade do AABB — validar na wave H).
- **Descoberta estrutural (B):** o swap literal para o `scene_runtime` é
  bloqueado por **ciclo** — o crate ainda dep do monólito pela ponte de 18
  módulos (`viber::{luau, scene, xml, …}`). O desbloqueio é a **morte da
  ponta do luau (wave F)**: depois disso, os forks sincronizados em B/C
  trocam com 1 linha cada. Destinos sem ciclo (engine/audio/core/save/ui)
  já têm shim completo.
- **D** — portar o terreno/água para o `bevy_ext_terrain_runtime` DEPOIS
  de a água da sessão concorrente aterrar (não portar alvo em movimento).
- **F** — morte do `viber.ecs` + famílias luau por call-sites no BMS +
  adapter BRP do `viber.lua`; **é o desbloqueio dos swaps do scene_runtime**.
- **G** — worldsys/ambient/music/ai/animation (pós-C; o `d8d4db7` "gestos
  sem rig" do main já mata o `drive_player_animation`).
- **H** — apagar a dep `viber`, arquivar o monólito; gate final
  `cargo tree -i viber` vazio + QA vivo completo (boot/andar/save/morte/
  interiores/sessão longa).

## Superfície actual (medida 2026-11-14)

- Vael main: **257 refs `viber::`** em Rust, 32 módulos (topo: luau 68,
  ui 28, terrain 22, recipes 17, input_map 17, scene_loader 15, app 12,
  worldsys 11). Lua: **1807 call sites**, 106 namespaces (viber.ui 256,
  load 149, toast 108, game 96, sound 87; `viber.vael` 80 é do próprio
  jogo). Monólito: **162.501 LOC**. Crates extraídos sem consumidor:
  **54.691 LOC** (terrain_runtime 27,4k; engine 10,8k; scene_runtime 5,2k;
  physics 3k; luau-BMS 2,4k; ai 2,2k; audio 1,9k; nav 1,7k).

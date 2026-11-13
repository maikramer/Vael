# viber-tools — CLI, runtime e extensões

O `viber-tools` substitui o CLI do monólito Viber. O repo irmão vive em
`../viber-tools`; o comando instalado é:

```bash
cd ../viber-tools && tools/install.sh
which viber                 # ~/.local/bin/viber -> viber-tools
```

O instalador preserva o wrapper antigo como `viber-engine` (usado por
`bake`/`tool scatter`/`trace`/`lua` enquanto a extração não termina) e faz
`viber` apontar para `viber-tools`. O engine `viber` continua a ser a
dependência de gameplay do Vael; a remoção do engine é a fase seguinte.

## Comandos

| Comando | Implementação |
|---|---|
| `viber analyze [world.bsn]` | `viber_tools_core` (headless) |
| `viber scene lint --world …` | `viber_tools_core` |
| `viber bsn check/convert` | `bevy_ext_world` via core |
| `viber debug …` | runtime novo; fallback `viber.*` |
| `viber session status/list` | `engine.json` novo + legado |
| `viber run` | lança o Vael (`VAEL_BIN` ou `cargo run` em `../Vael`) |
| `viber mcp` | servidor MCP stdio |
| `viber bake`, `viber tool`, `viber scene` (exceto lint), `viber terrain`, `viber cinematic`, `viber create/prune` | delega no `viber-engine` |

O contrato de saída (JSON de `analyze`, exit codes, `session claim`=3) é
mantido. `viber debug prof --tab frames`, `--hitches`, `--record` e
`viber debug key/text/click/move/screenshot` falam com o runtime novo.

## Runtime no jogo

`viber_tools_runtime::ViberToolsPlugin` é adicionado pelo `vael` depois do
`build_app` quando há `--bridge`, `VAEL_TOOLS=1` ou `JACKDAW_PIE`. Reusa o
`bevy_remote` já aberto e acrescenta o namespace `viber-tools/*`:

- `viber-tools/ping`, `frame_stats`, `hitches`, `memory`, `tree`, `logs`;
- `viber-tools/input.key`, `step`, `play`, `pause`;
- `viber-tools/screenshot`/`burst`/`trace` (métodos planeados/legado).

O `viber.*` legacy continua no mesmo servidor para compatibilidade.

## Extensões Jackdaw

Em `../viber-tools/extensions/`:

- `vael_analyze_jackdaw` — painel **Vael: Problems**, operador
  `vael.analyze.run` (JSON em `reports` do `jd mcp`);
- `vael_profiler_jackdaw` — painel **Vael: Profiler**, polling de
  frames/p95/hitches/RSS;
- `vael_debugger_jackdaw` — play/pause/step e F3;
- `vael_mcp_jackdaw` — operadores de seleção/grupo/input/screenshot visíveis
  no `jd mcp`.

Ciclo: `jd build` → `jd extension pack` → `jd extension install
<bundle>.jdext --trust-publisher` → reiniciar o editor; validar com
`jackdaw check-extensions --expect …`. As extensões não correm `analyze` em
CI: isso é `viber analyze`/`viber-tools`.

## MCP

`viber mcp` (stdio, JSON-RPC por linha) expõe `viber_analyze`, `viber_lint`,
`viber_profiler_frames`, `viber_input_key`, `viber_screenshot`,
`jackdaw_operator`, `jackdaw_scene_summary`, `jackdaw_screenshot` e
`jackdaw_wait`. Liga-se ao editor por `.jackdaw/editor.json` (15703) e ao jogo
por 15702 (`VIBER_TOOLS_PORT`). Config de cliente:

```json
{ "command": "viber", "args": ["mcp"] }
```

## Limites desta fase

- `analyze`/`lint` nativos são a primeira versão (estrutura/assets/scripts);
  as regras finas de protótipos/NPCs/marcos ainda não substituem o
  `tools/lint_content.py` do Vael.
- `bake`, `tool scatter`, `scene` (exceto lint), `terrain`, `cinematic`,
  `trace`, `lua` e tabs de profiler de mundo/física/áudio continuam no
  `viber-engine`.
- O engine `viber` não sai do `Cargo.toml` do Vael até player/câmara, Lua e
  `build_app` serem portados.

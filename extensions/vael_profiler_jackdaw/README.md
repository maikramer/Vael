# vael_profiler_jackdaw

Painel **Vael: Profiler** no Jackdaw. Uma thread de background faz polling ao
runtime do jogo (`vael-tools/frame_stats`, `hitches`, `memory`) e o painel
mostra p50/p95, FPS, hitches e RSS. O operador `vael.profiler.snapshot`
devolve o último JSON em `report_to_caller` (visível no `jd mcp`).

Requer o jogo com o `bevy_ext_bridge_runtime` e o `--bridge` ligados. A porta vem
de `VAEL_TOOLS_PORT` (default 15702). Sem jogo, o painel mostra "bridge
desligado" e continua a tentar.

# vael_debugger_jackdaw

Painel **Vael: Debugger** do Jackdaw. Os operadores `vael.debug.play`,
`vael.debug.pause`, `vael.debug.step` e `vael.debug.toggle_profiler` chamam o
runtime do jogo (`vael-tools/play|pause|step|input.key`) e devolvem o
resultado em `report_to_caller` (visível no `jd mcp`).

O Jackdaw PIE não expõe o transporte do jogo às extensões; o painel usa o BRP
lateral do `bevy_ext_bridge_runtime`. A porta vem de `VAEL_TOOLS_PORT` (default
15702); sem jogo, os operadores avisam no `warnings` do operador.

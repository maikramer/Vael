# vael_mcp_jackdaw

Operadores de manipulação do Vael visíveis ao `jd mcp`:

- `vael.mcp.add_group` — cria um grupo `MCP` sob a seleção atual;
- `vael.mcp.input_f3` — envia F3 ao jogo (toggle do profiler);
- `vael.mcp.screenshot` — guarda um screenshot do jogo em `.jackdaw/`.

O `jd mcp` não permite a uma extensão acrescentar tools; qualquer operador
registado aparece em `list_operators`/`call_operator` e os resultados vão em
`reports`. Para parâmetros arbitrários (nome, tecla, screenshot) os agentes
usam `call_operator` no `jd mcp` ou o CLI `vael-tools debug`.

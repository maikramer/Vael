# vael_debugger_jackdaw

Painel **Vael: Debugger** do Jackdaw. Os operadores `vael.debug.play`,
`vael.debug.pause`, `vael.debug.step` e `vael.debug.toggle_profiler` chamam o
runtime do jogo (`vael-tools/play|pause|step|input.key`) e devolvem o
resultado em `report_to_caller` (visível no `jd mcp`).

O Jackdaw PIE não expõe o transporte do jogo às extensões; o painel usa o BRP
lateral do `bevy_ext_bridge_runtime`. A porta vem de `VAEL_TOOLS_PORT` (default
15702); sem jogo, os operadores avisam no `warnings` do operador.

A extensão também permite deslocar a vista 3D com o botão do meio e orbitar
com Alt + botão do meio. **Localizar jogador** e **Seguir / soltar jogador**
usam a posição mundial do jogador no Play. Um anel na vista 3D assinala essa posição. O pedido corre fora da thread do
editor. Ver o [fluxo de edição ao vivo](../../docs/JACKDAW.md#navegar-e-ajustar-durante-o-play).

Para instalar a extensão a partir deste checkout no Linux x86_64, execute
na raiz do Vael e reabra o Jackdaw depois da instalação.

```sh
cd extensions/vael_debugger_jackdaw
jd build
jd extension pack
jd extension install vael_debugger_jackdaw-0.1.3-x86_64-unknown-linux-gnu.jdext
```

Nas outras plataformas, use o nome do pacote que o comando de empacotamento imprimir.

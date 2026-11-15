# vael_analyze_jackdaw

Extensão do Jackdaw com o painel **Vael: Problems** e o operador
`vael.analyze.run`.

O operador usa o `vael_tools` (o mesmo do `vael-tools analyze`) e
publica o relatório JSON em `report_to_caller`; o `jd mcp` vê-o em `reports`.
O painel mostra a contagem de entidades/assets/scripts e os warnings.

```bash
jd build
jd extension pack
jd extension install <bundle>.jdext --trust-publisher
jackdaw check-extensions --expect vael_analyze_jackdaw
```

O Jackdaw não tem um painel Problems nativo; este é o painel próprio da
extensão. Analyze headless/CI continua a ser `vael-tools analyze`.

---
name: vael-tools
description: Usar quando a tarefa precisa do CLI do Vael ou do bridge de debug — analyze/lint do mundo .bsn, bsn convert dos geradores, arranque com --scene/--entry/--bridge, profiling (prof/key/screenshot/burst), primitivas de teste (pause/step/play, state, expect, until, scene, playbook) e QA vivo. O antigo `viber` foi aposentado; o editor é o Jackdaw (jd open .). Vael tools, Bevy, vael-tools run, vael-tools debug.
---

# vael-tools

O CLI vive neste repo em `cli/` (pacote `vael-tools`, lib `vael_tools`); o
bridge in-game está em `../bevy_extensions/crates/bevy_ext_bridge_runtime`.
O antigo CLI `viber` e o checkout `../viber-tools` foram aposentados. A lei
continua a ser o `AGENTS.md` desta pasta e `docs/TOOLS.md`.

## Quando usar

Mundo, conteúdo, scripts e cenas ficam neste repo. Bugs de runtime/API em
falta são da engine: `../bevy_extensions/AGENTS.md` (o runtime de
gameplay do jogo vive em `crates/vael_engine` aqui).

## Estático (sem janela)

```bash
tools/install.sh                         # uma vez: vael-tools no ~/.local/bin
vael-tools analyze world.bsn --json      # contrato com o lint_content.py
vael-tools lint                          # assets/scripts em falta
vael-tools bsn convert --force f.xml     # só geradores (tools/qa.py)
```

Para editar cenas ou consultar o schema, veja a [skill de edição BSN](../vael-bsn/SKILL.md).

## Arranque

```bash
jd build                                         # build oficial: jogo (debug) + schema do editor
jd run                                           # build + lança (sem argumentos: menu/título)
vael-tools run --scene main --entry dev          # debug directo, sem menu/prólogo
vael-tools run --scene main --entry dev --bridge 15706
```

O `jd run` compila `-p vael` em `.jackdaw/target/game` e lança o binário **sem
argumentos** (os `[[run]]` do `jackdaw.toml` só o Play do editor lê). O
`vael-tools run` aceita os argumentos e lança `target/release/vael` (ou o
`VAEL_BIN`; `.jackdaw/target/game/debug/vael` reaproveita o build do `jd`).

## QA vivo

Uma instância por vez; não mates o processo. O carregamento é lento (dezenas
de segundos; mais com builds ao lado) e a bridge só responde entre frames —
pedidos lentos durante o load são normais.

```bash
vael-tools debug --port <p> wait-ready --timeout 300   # ANTES de screenshot/input
vael-tools debug --port <p> screenshot -o shot.png
vael-tools debug --port <p> prof --tab frames
vael-tools debug --port <p> key f3 --hold-ms 120
vael-tools debug --port <p> lua 'return vael.ping()'
```

`ready` vem do espelho `BridgeReadiness` no ping novo; depois de
`debug scene --wait` não precisa de segunda espera (o `--wait` já esperou).
Detalhe: `docs/DEBUG.md`, `docs/TOOLS.md` e a skill `vael-qa`.

## Primitivas para agentes

`debug --json` dá JSON no stdout em tudo (progresso no stderr) e exit
semântico — `wait-ready`/`until`/`expect`/`scene --wait` devolvem 1 em
timeout/falha, e erros JSON-RPC vão para o stderr:

```bash
vael-tools debug --port <p> --json wait-ready --timeout 300
vael-tools debug --port <p> pause && vael-tools debug --port <p> step --frames 30
vael-tools debug --port <p> state                       # quests/itens/ouro/hp em JSON
vael-tools debug --port <p> expect '{"items":{"gold":">=150"}}'   # regras reais; exit 1 em falha
vael-tools debug --port <p> until 'vael.game().hp >= 100' --timeout-ms 5000
vael-tools debug --port <p> tree --filter npc           # entidades por nome
vael-tools debug --port <p> logs --tail 50 --grep error # ficheiro da sessão (ou --source bridge)
vael-tools debug --port <p> engines --json              # sessões com [vivo]/[morto]; --prune apaga mortas
vael-tools debug --port <p> scene interiors/forge --wait
```

Testes compostos sem Python: um playbook JSON com `debug playbook` (passos
`lua/key/text/click/move/wait_ms/until/state/expect/screenshot/pause/play/
step/scene`, transcript JSON, para no 1.º falha) — esquema e exemplo em
`docs/TOOLS.md`. O `qa.py run/steps` usa exatamente estas primitivas por
baixo.

**Ressalva**: um reload do mundo completo por `scene` pode esgotar o teto de
240 s e deixar o loader em busy (quirk do monólito); interiores e o fluxo por
portais funcionam. `qa.py steps` só faz sentido num jogo já na cena do
cenário — para o ciclo completo usa `qa.py run`.

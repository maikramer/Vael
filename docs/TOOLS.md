# vael-tools — CLI, bridge e extensões

As formas oficiais de buildar e correr o jogo são as do editor: `jd build`
(binário do jogo em debug + o schema que o editor carrega) e `jd run .` (build +
lança, **sem argumentos** — só o Play do editor lê os presets `[[run]]` do
`jackdaw.toml`). `jd doctor .` verifica o setup. Este documento cobre o
`vael-tools`, o CLI do jogo: análise headless, lint, bridge de debug e o
arranque **com** argumentos (`--scene/--entry/--bridge/--resolution`), que é o
que o `jd run` não aceita.

O `vael-tools` é o CLI do jogo (a CLI antiga foi aposentada) e vive
no próprio repo, em `cli/`. É um membro do workspace: `cargo run -p vael-tools
-- …`. Para o ter no PATH:

```bash
tools/install.sh            # build release + symlink ~/.local/bin/vael-tools
which vael-tools
```

O `jd run` compila em `.jackdaw/target/game`; o `vael-tools run` lança
`target/release/vael`. Para QA com o binário do `jd` (o mesmo que o Play usa) e
bridge ligada:

```bash
VAEL_BIN=.jackdaw/target/game/debug/vael vael-tools run --scene main --entry dev --bridge
```

## Comandos

| Comando | O que faz |
|---|---|
| `vael-tools analyze [world.bsn] [--json] [--strict]` | Relatório headless (contagens, tags, assets/scripts em falta, `names`, `prototype_ids`) |
| `vael-tools lint [world.bsn] [--json] [--strict]` | Lint estrutural (assets e scripts em falta) |
| `vael-tools bsn inspect/show/schema` | Localizar entidades físicas, ler componentes e consultar campos do editor |
| `vael-tools bsn set/patch/insert` | Editar campos, atributos e filhos com prévia ou gravação atômica. [Receitas de autoria](../.cursor/skills/vael-bsn/SKILL.md) |
| `vael-tools bsn transcode` | Criar uma cópia entre `.bsn` textual e `.bsb` binário |
| `vael-tools bsn check/convert` | Validar mundos ou comparar XML legado com BSN. `convert` importa XML de geradores |
| `vael-tools run [world] --scene S --entry E --bridge [PORT] --resolution WxH` | Lança `target/release/vael` (ou `VAEL_BIN`); sem binário, delega em `cargo run --release` |
| `vael-tools debug …` | Cliente do bridge do jogo (ver abaixo) |

A lib `vael_tools` (`cli/src/lib.rs`) expõe análise, lint, importação de XML e verificação do mundo para a extensão `vael_analyze_jackdaw`. A edição offline fica em `cli/src/bsn_edit/`. O JSON de `analyze` é contrato com o
CI e o `tools/lint_content.py`.

## Bridge de debug

`vael-tools debug` fala JSON-RPC/HTTP com o jogo a correr. A porta é `--port`,
`VAEL_TOOLS_PORT` ou 15702. **Contratos para agentes**: `debug --json` devolve
JSON no stdout (progresso no stderr) em todos os subcomandos; erros vão para o
stderr com exit 1; as primitivas de teste têm exit semântico — `wait-ready`,
`until`, `expect` e `scene --wait` devolvem 1 em timeout/falha.

| Subcomando | Notas |
|---|---|
| `ping`, `probe`, `wait-ready [--json]` | `wait-ready` espera o `ready` (véu de carregamento saído) antes de screenshots/input |
| `engines [--json] [--prune]` | Sessões conhecidas com estado de vida (pid + porta); `--prune` apaga as mortas |
| `screenshot -o f.png`, `burst -o f.png --frames 9 --skip 0` | Captura única ou folha de N frames (screenshot tem 1 retry transitório) |
| `video --seconds 5 --fps 15 -o movimento.mp4` | Grava a janela do jogo. Aceita câmera lenta, recorte e relatório JSON com a contagem de quadros repetidos. [Exemplo na arena](DEBUG.md#gravar-movimentos-na-arena) |
| `tree [--filter TEXTO]`, `logs [--tail N --grep T --source file\|bridge\|auto]`, `memory`, `hitches` | Estado do jogo. `logs` lê o ficheiro da sessão (completo desde o boot) ou o ring do jogo (`bridge`) |
| `prof --tab frames`, `--hitches`, `--record 30 --hz 2 -o perf.jsonl` | Profiler |
| `mark`, `key --hold-ms`, `text`, `click`, `move` | Trace e input sintético |
| `lua CODE --file` | REPL Luau (método legacy `vael.lua`) |
| `pause`, `play`, `step --frames N` | Determinismo por frames: pausa o `Time<Virtual>`, corre N frames e volta a parar |
| `state` | Estado do jogo em JSON (quests/itens/ouro/hp/posição) via `vael.vael.qa_snapshot()` |
| `expect [JSON\|@ficheiro]` | Verifica um expect (mesma sintaxe dos cenários de QA) contra o estado real, com as regras do jogo. Exit 1 em falha |
| `until EXPR [--timeout-ms --interval-ms]` | Sonda uma expressão Luau até ser verdadeira (mata os sleeps). Exit 1 no timeout |
| `scene NOME [--entry E] [--wait]` | Troca de cena (com retry no loader busy) e espera o carregamento novo |
| `playbook FICHEIRO [-o transcript.json] [--continue-on-error]` | Sequência de passos de teste num processo só, com transcript JSON (ver abaixo) |

Os métodos novos são `vael-tools/*`, servidos pelo `bevy_ext_bridge_runtime`;
o cliente cai para o namespace legacy `vael.*` (monólito) quando o método
novo não existe — é o caso de `lua`, `burst` e `trace` até à fase H do
DELETE-MONOLITH. Quando `--world` não casa com sessão nenhuma, o CLI avisa no
stderr antes de cair na porta por omissão.

### Playbook

Um playbook JSON compõe os passos num processo só — o transcript diz que
passo falhou e porquê, e o exit code fecha o ciclo sem loops em Python:

```json
{
  "name": "forja",
  "steps": [
    { "op": "scene", "name": "interiors/forge", "wait": true },
    { "op": "until", "expr": "vael.game().hp >= 100", "timeout_ms": 3000 },
    { "op": "key", "key": "KeyE" },
    { "op": "state" },
    { "op": "expect", "expect": { "items": { "gold": ">=10" } } },
    { "op": "screenshot", "path": "out/fim.png" }
  ]
}
```

Passos: `lua`, `key`, `text`, `click`, `move`, `wait_ms`, `until`, `state`,
`expect`, `screenshot`, `pause`, `play`, `step` (frames), `scene`. Sem
`--continue-on-error`, para no primeiro passo que falha e devolve exit 1;
`-o transcript.json` grava o transcript além de imprimi-lo.

## Runtime no jogo

`bevy_ext_bridge_runtime::BridgeToolsPlugin` é adicionado pelo `vael` depois do
`build_app` quando há `--bridge`, `VAEL_TOOLS=1` ou `JACKDAW_PIE`. Reusa o
`bevy_remote` já aberto e acrescenta `vael-tools/*`:

- `ping` (com `ready` + `loading` quando o recurso `BridgeReadiness` existe —
  o espelho é o `vael::bridge_ready::BridgeReadyPlugin`, que copia o estado do
  véu do monólito cada frame), `frame_stats`, `hitches`, `memory`, `tree`;
- `input.key`, `input.text`, `input.click`, `input.move`;
- `step`, `play`, `pause`.

O `vael.*` legacy continua no mesmo servidor para compatibilidade. Sem o
espelho de readiness (jogo velho ou app mínima), o `ping` novo não traz
`ready`; o `wait-ready` do CLI pergunta então ao `vael.ping` legacy e, na
ausência de ambos, trata como pronto com um aviso no stderr.

**Ressalva conhecida (monólito, pré-existente)**: um reload do mundo completo
por `scene` (`main`→`main`, ou o regresso de um interior com o epoch preso em
"spawn groups") pode esgotar o teto de 240 s (`timed_out: true` no ping) e
deixar o loader em `busy()` — pedidos seguintes levam "já há uma troca de cena
em curso". O fluxo `main`→interior→`main` por portais funciona; o reload
direto do mundo completo fica para a onda H do DELETE-MONOLITH.

## Extensões Jackdaw

Em `extensions/` (projetos standalone, `jd build`; fora do workspace):

- `vael_analyze_jackdaw` — painel **Vael: Problems**, operador
  `vael.analyze.run` (JSON em `reports` do `jd mcp`), usa a lib `vael_tools`;
- `vael_profiler_jackdaw` — painel **Vael: Profiler**, polling de
  frames/p95/hitches/RSS;
- `vael_debugger_jackdaw` — play/pause/step e F3;
- `vael_mcp_jackdaw` — operadores de seleção/grupo/input/screenshot visíveis
  no `jd mcp`;
- `vael_content_jackdaw` — painéis de Itens/Marcos/Progressão/Diálogos/
  Missões/Conteúdo/Cenas (edita os JSON de `data/`, `quests/` e `dialogues/`).

Ciclo: `jd build` → `jd extension pack` → `jd extension install
<bundle>.jdext --trust-publisher` → reiniciar o editor. As extensões não
correm `analyze` em CI: isso é `vael-tools analyze`.

## MCP

O jogo expõe o ECS e ferramentas práticas como **tools MCP nativas** para os
agentes ZCode, em duas camadas sobre o mesmo servidor BRP da 15702:

- **`bevy_brp_mcp` 0.23** (binário stdio, `~/.cargo/bin/bevy_brp_mcp`,
  registado em `~/.zcode/cli/config.json` como `brp`): query/get/insert/
  mutate/watch de componentes e recursos, spawn/despawn, `rpc_discover` e
  `brp_type_guide`. A porta vai **por chamada** de cada tool (default 15702).
  É o que a engine já serve: os builtin `world.*` do `bevy_remote` vivem ao
  lado de `vael-tools/*`, `vael.*` e `jackdaw/*`.
- **`brp_extras/*`** (crate `bevy_brp_extras` 0.23, vendorizado em
  `vendor/bevy_brp_extras`; `src/main.rs`, off-switch `VAEL_BRP_EXTRAS=0`):
  screenshot (imagem inline na conversa), teclas/rato/gamepad sintéticos,
  `get_diagnostics` (FPS) e `shutdown`. O plugin só injeta métodos — deteta o
  `RemoteHttpPlugin` do bridge e ignora a própria porta.

Divisão de trabalho: **MCP é o caminho natural** (explorar o mundo vivo,
mutar componentes, ver screenshots); **o CLI é o determinístico** —
`wait-ready` com exit code, `expect`/`until` com timeout, playbooks e
`step` por frames. Ressalvas:

- teclas que precisam de `just_pressed` no mesmo frame continuam melhores
  pelo `vael-tools debug key` (o `BridgeToolsInput` corre ordenado no
  `PreUpdate`; as teclas do `brp_extras` injetam eventos sem essa ordem);
- **não usar** `brp_launch`/`brp_list_bevy` para o Vael — lançar é
  `vael-tools run --scene main --entry dev --bridge 15702`;
- `brp_extras/shutdown` desliga o jogo (útil para ciclos de QA, mas não é
  o `pause` do CLI).

O editor continua a expor operadores por `jd mcp`; o CLI não tem servidor MCP.

## Limites desta fase

- `analyze`/`lint` cobrem estrutura/assets/scripts; as regras finas de
  protótipos/NPCs/marcos continuam em `tools/lint_content.py`. Os campos
  `skipped_tags`/`unknown_prototypes` do contrato antigo do `analyze` saíram:
  eram inicializados vazios, nunca preenchidos e ninguém os consumia.
- `lua`, `burst`, `trace` e `scene` ainda dependem do monólito (`vael.*`).
- `state`/`expect` usam a API de QA do jogo (`vael.vael.qa_snapshot`/
  `qa_check`), também ela do monólito — o playbook já aceita um adapter
  próprio quando isso sair.
- O ring de logs próprio do runtime (`vael-tools/logs`) entra depois do
  monólito, via `LogPlugin::custom_layer`; enquanto ele viver, `debug logs
  --source bridge` usa o ring do `vael.logs`.
- O runtime `vael_engine` não sai do `Cargo.toml` do Vael até o Luau/AI e o
  `build_app` serem portados (ver `DELETE-MONOLITH.md`).

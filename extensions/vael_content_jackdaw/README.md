# vael_content_jackdaw

Extensão do Jackdaw 0.20 para a autoria de conteúdo do Vael. Porta para o
editor oficial os sete painéis que viviam na app `vael-editor` (legado):

| Janela | Ficheiros | O que edita |
|---|---|---|
| **Vael: Cenas** | `world.bsn`, `scenes/**/*.bsn` | abre o mundo, o menu e cada interior como aba de cena (`scene.open`) |
| **Vael: Itens** | `data/items.json` | label, kind, stack, loja, uso, arma, requisitos, efeitos |
| **Vael: Marcos** | `data/landmarks.json` | name/label/bioma/raio/survey/peregrino |
| **Vael: Progressão** | `data/progression.json` | curva XP, `kill_xp`, talentos e efeitos |
| **Vael: Diálogos** | `dialogues/*.json` | start, nós, falas, escolhas, goto e flags de quest |
| **Vael: Missões** | `quests/*.json` + `index.json` | formulário completo, parley, stages, requires/next |
| **Vael: Conteúdo** | — | validação cruzada e falhas de carregamento |

Os painéis abrem no dock inferior. **Tools → Vael** tem recarregar, gravar
tudo e validar; os botões de cada painel gravam só o ficheiro do painel. O
formulário escreve com os widgets do próprio Jackdaw (`text_edit`,
`combobox`, scroll) e o commit é aplicado ao estado em memória; gravar é
explícito.

## Raiz do projeto

A extensão resolve a raiz por, por ordem:

1. `VAEL_ROOT` (útil em QA);
2. o `.bsn` aberto no editor (sobe até ao `jackdaw.toml` + `world.bsn`);
3. o diretório atual.

Sem raiz, os painéis mostram "abre o world.bsn" e os operadores de gravação
recusam.

## Gravação

- `write_json_atomic`: escreve `.tmp`, copia o atual para `.bak` e faz
  rename. Um crash a meio não deixa JSON truncado.
- JSON com 2 espaços e newline final, como os ficheiros atuais.
- Campos desconhecidos ficam em `extra` e sobrevivem à gravação.
- `quests/index.json` acompanha a gravação de quests.
- Um ficheiro que falhou a carregar nunca é gravado por cima dos defaults.

## Validação

O painel **Conteúdo** corre as regras de `vael-content`: quests (id,
unicidade, npc no mundo, bioma, objetivo, marcos/itens, parley, recompensas,
requires/next/stages), diálogos (start/goto, texto, locale, alcançabilidade,
flags), itens, marcos e progressão. O índice do mundo é lido do `world.bsn`
(ou `world.xml` legado) com o mesmo `load_world` do runtime, que expande os
`<Include>`. Como o mundo tem 16 MB de módulos, o arranque lê só os JSON e o
índice é construído no primeiro **Validar** (ou no teste), para não bloquear
a abertura do editor.

## Build

```bash
jd build
jd extension pack
jd extension install vael_content_jackdaw-*-x86_64-unknown-linux-gnu.jdext --trust-publisher
```

Ativar no projeto: `[extensions] vael_content_jackdaw = true` no
`jackdaw.toml`. O checkout irmão `../Vael` tem de estar ao lado: o crate
`vael-content` e os JSON são lidos por caminho relativo
(`../../../Vael/content`).

## Testes

```bash
CARGO_TARGET_DIR=.jackdaw/target/game cargo test --lib
```

O teste carrega o conteúdo real do checkout irmão (se existir), incluindo a
validação contra os nomes do `world.bsn`; sem o irmão, passa sem exercitar.

# Arranque directo para debug (sem menu e sem prólogo)

O jogo arranca normalmente na cena de menu (`scenes/menu.xml`), mostra o ecrã
de título e, ao entrar em `main`, toca a cinemática `prologo` (~35 s) na
primeira partida. Para iterar em gameplay isso é tempo perdido. O atalho é:

```bash
cd Vael
viber run --scene main --entry dev
```

O resultado é o estado de um **Novo Jogo** sem o título e sem o prólogo:

| Passo do arranque normal | Com `--scene main --entry dev` |
|--------------------------|-------------------------------|
| Cena de menu (acampamento) | Saltado: a primeira cena é `main` |
| Ecrã de título + HUD escondido | Saltado: o HUD nasce visível |
| Prólogo `prologo` (35 s) | Saltado (`story_prologo` fica marcado) |
| Jogador em `(0, 150, 0)` | Igual: a entrada `dev` usa a mesma pose e assenta no chão |
| Regras, quests, loja, saves | Normais — é o jogo a sério |

## Ferramentas de debug ([F1]/[F2])

Num run com `--entry dev`, `scripts/game/devmode.lua` (autoload
`world/autoload/player.xml`) liga três ferramentas:

| Tecla | Efeito |
|-------|--------|
| **F1** | Liga/desliga o **voo livre**. O herói fica invisível e anda com a câmara; é ele que mantém streaming, `player_nearby`, áudio e gatilhos de quest no sítio certo. Ao sair, o herói aterra onde a câmara estava |
| **F2** | **Invulnerável** a pé (dano e veneno bloqueados em `lib/rules.lua`). O voo força-o e devolve o estado do F2 ao sair |
| **Tab** (a voar) | **look-at**: aponta a vista ao prop nomeado/criatura mais alinhado com a direção atual, até 400 m. Repetir Tab passa ao seguinte |

A voar: **WASD** move, **Space** sobe, **Ctrl** desce, **Shift** acelera ×3,
**setas** olham (o runtime não lê o rato). O `player_movement` é reclamado
(`own_system("locomotion")`) enquanto o voo dura e devolvido
(`release_system`) ao sair.

O latch é de SESSÃO (`viber.state()`), não vai para o save. Num arranque
normal (`viber run`, Continuar, Novo Jogo) o `devmode.lua` não faz nada — as
teclas não existem.

QA pela bridge: o REPL não drena os comandos de script (um `change_scene` de
lá é um no-op), mas partilha a VM dos scripts — `_G.viber_dev = true` liga as
ferramentas na sessão viva, sem reiniciar a engine:

```bash
viber debug --world world.xml lua '_G.viber_dev = true return true'
viber debug --world world.xml input key f1     # ou o MCP viber_input
```

Não há save carregado: o estado começa limpo, como no **Novo Jogo** (o kit
inicial de `lib/rules.lua` é semeado). Para testar a partir de um save, usa o
fluxo normal (Continuar) — o atalho é para arranques limpos.

## Como funciona

O sinal é o **nome da entrada de cena**, lido em Lua por
`viber.scene_entry()`. `world.xml` declara duas entradas na mesma pose:

```xml
<SceneEntry name="spawn" translation="0 150 0" />
<SceneEntry name="dev" translation="0 150 0" />
```

`spawn` é a **primeira** de propósito: um `change_scene("main")` sem entrada
(o "Novo Jogo" do título) chega por ela e não pelo modo debug. A ordem importa
— não inverter. `--scene main` escolhe o mundo (em vez do
`project.start-scene`); `--entry dev` chega pela segunda e a engine regista
`dev` como entrada da cena. A partir daí:

- `scripts/lib/dev.lua` → `dev.skip_intro()` devolve `true` quando a cena é
  `main` e a entrada é `dev`.
- `scripts/ui/title.lua` (autoload) vê o sinal no primeiro tick, aplica a
  escala de UI guardada, liga o HUD e marca `title_done` — sem abrir o véu.
- `scripts/game/story.lua` marca `story_prologo` no `viber.game()` para o
  prólogo não disparar nem agora nem ao voltar de um interior (a entidade do
  story renasce com a cena; um latch só de `viber.state()` perdia-se).

Não foi preciso mexer na engine: `--scene` e `--entry` já existem, e o nome
da entrada é observável por Lua. O modo é **por execução** — não fica gravado
em save nem em ficheiro de configuração.

## Efeitos no save

`title_done` e `story_prologo` vivem em `viber.game()` (primitivos, vão para o
save). Consequências:

- Se o jogo **autosavar** durante a sessão de debug, esse save fica com o
  prólogo marcado como visto (não volta a tocar ao Continuar). Um **Novo
  Jogo** limpa a tabela e repõe o prólogo.
- `title_done = true` é o mesmo estado de quem já passou o título: abrir
  "Ecrã de título" pelo diário/morte continua a funcionar normalmente.

## O que NÃO é saltado

- **Outras cinemáticas** (`guardiao-*`, `nota`, `campo-de-batalha`, `epilogo`)
  e os gatilhos de `scripts/cinematic-trigger.lua` continuam a tocar. Todas
  são `skippable="true"`; a tecla de skip da engine continua a valer.
- **Diálogos, quests, economia, combate e saves** — é o jogo normal.
- **A cena de menu** deixa de ser a primeira, mas continua a existir para o
  botão "Ecrã de título" (a morte e o diário usam-na).

## Alternativas

| Quero… | Comando |
|--------|---------|
| Só saltar a cena de menu, manter título e prólogo | `viber run --scene main` |
| Só saltar o menu e o prólogo (debug) | `viber run --scene main --entry dev` |
| Um cenário QA isolado, sem campanha | `viber run qa-cenarios.xml` (etc.) |
| Voltar ao fluxo normal | `viber run` |

`viber run --scene main --entry dev` também aceita caminho explícito
(`viber run world.xml --scene main --entry dev`); o wrapper do CLI trata do
build.

## Onde está o código

| Ficheiro | Papel |
|----------|-------|
| `scripts/lib/dev.lua` | `dev.entry()` / `dev.skip_intro()` — o sinal |
| `scripts/game/devmode.lua` | [F1] voo livre + [F2] invulnerável (autoload) |
| `scripts/lib/rules.lua` | `set_god`/`M.god` — o único portão do dano ao herói |
| `scripts/ui/title.lua` | Salta o véu e liga o HUD no primeiro tick |
| `scripts/game/story.lua` | Trava o prólogo (marca persistente) |
| `world.xml` | `spawn` (normal) e `dev` (debug), nesta ordem — a pose é a mesma |

Se mudares o nome da entrada, muda `DEBUG_ENTRY` em `lib/dev.lua` no mesmo
commit; o resto do jogo lê o sinal por essa constante.

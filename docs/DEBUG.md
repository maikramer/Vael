# Arranque directo para debug (sem menu e sem prólogo)

O jogo arranca normalmente na cena de menu (`scenes/menu.bsn`), mostra o ecrã
de título e, ao entrar em `main`, toca a cinemática `prologo` (~35 s) na
primeira partida. Para iterar em gameplay isso é tempo perdido. O atalho é:

```bash
cd Vael
vael-tools run --scene main --entry dev
```

(Build e run sem argumentos são os do editor — `jd build .` e `jd run .`; o
`jd run` arranca no menu/título e não aceita `--scene/--entry`, ver
[`docs/JACKDAW.md`](JACKDAW.md). Para reaproveitar o binário do `jd` neste
atalho: `VAEL_BIN=.jackdaw/target/game/debug/vael vael-tools run --scene main
--entry dev`.)

O resultado é o estado de um **Novo Jogo** sem o título e sem o prólogo:

| Passo do arranque normal | Com `--scene main --entry dev` |
|--------------------------|-------------------------------|
| Cena de menu (acampamento) | Saltado: a primeira cena é `main` |
| Ecrã de título + HUD escondido | Saltado: o HUD nasce visível |
| Prólogo `prologo` (35 s) | Saltado (`story_prologo` fica marcado) |
| Jogador em `(0, 150, 0)` | Igual: a entrada `dev` usa a mesma pose e assenta no chão |
| Regras, quests, loja, saves | Normais — é o jogo a sério |

## Esperar pelo carregamento (agentes/QA)

O ecrã de carregamento sai com o **anel próximo**: os `<GltfScene>` a menos de
`VIBER_GLTF_READY_RADIUS` (default 200 m) do herói com o GLB resolvido, as
texturas subidas para a GPU, os albedos do terreno, os spawn groups e o herói
assente (`world_ready.rs` na engine). O resto do mundo entra **por distância**
(`gltf_residency.rs`): cada instância pede o GLB quando o corte de render dela
se aproxima e larga-o bem depois de sair de cena. O mesmo véu cobre cada troca
de cena (`Settle` espera pela prontidão antes do fade de volta).
Num portátil o carregamento leva dezenas de segundos — **mais ainda com
compilações a correr ao lado**. Screenshots ou input antes disso apanham o
véu, não o jogo.

```bash
cargo run --release -- world.bsn --scene main --entry dev --bridge 15731 &
vael-tools debug --port 15731 wait-ready --timeout 300   # exit 0 = pronto
vael-tools debug --port 15731 screenshot -o shot.png
# para agentes: JSON no stdout (ready/loading/elapsed) e exit semântico
vael-tools debug --port 15731 --json wait-ready --timeout 300
```

- `viber.ping`, `vael-tools/ping` e `viber.scene.current` devolvem `ready` e
  `loading` (`status`, `progress`, `pending_models`, `gpu_uploading`,
  `last_load_secs`, `timed_out`). O `vael-tools/ping` recebe a prontidão do
  recurso `BridgeReadiness`, espelhado cada frame a partir do véu do monólito
  por `vael::bridge_ready::BridgeReadyPlugin`; sem o espelho (jogo velho), o
  `wait-ready` pergunta ao `viber.ping`. `pending_models` conta só o anel do
  véu. Depois de `viber.scene.change` (ou `vael-tools debug scene --wait`),
  voltar a esperar.
- Um reload do mundo completo por `scene` pode esgotar o teto com
  `spawn groups=false` e deixar o loader em `busy()` (quirk do monólito,
  pré-existente) — os pedidos seguintes levam "já há uma troca de cena em
  curso". Interiores e o fluxo por portais funcionam.
- `mundo pronto em X s` no log é a medição do tempo de carregamento;
  `RUST_LOG=bevy_ext_viber_engine::world_ready=debug` dá o rasto por segundo e
  `RUST_LOG=bevy_ext_viber_engine::gltf_residency=info` o dos anéis
  (`N residentes, M adiados, +X/-Y` a cada 5 s com trabalho).
- Residência: `VIBER_GLTF_STREAMING=0` volta a carregar **todos** os GLB no
  arranque (o braço de A/B); `VIBER_GLTF_READY_RADIUS` (200),
  `VIBER_GLTF_LOAD_RADIUS` (800, o *bootstrap* de um modelo cujo corte de
  render ainda não se conhece), `VIBER_GLTF_LOAD_BUDGET` (8 activações/frame
  em jogo) e `VIBER_GLTF_UNLOAD_BUDGET` (8 descargas/frame) são os botões.
- Teto: 240 s (`VIBER_LOADING_MAX_S`); depois o jogo mostra-se incompleto e o
  `ready` vem com `timed_out: true`.
- Afinação: `VIBER_IO_THREADS` (decode de GLB/KTX2; default cores−2 — é o
  número que a app do Vael passa ao pool) e
  `VIBER_UPLOAD_MB` (subida para a GPU por frame, só atrás do véu; default
  256, `off` desliga). Em jogo não há limite: o Bevy tira da GPU um asset
  modificado quando o orçamento esgota e o mundo ficava negro.

## Ferramentas de debug ([F1]/[F2])

Num run com `--entry dev`, `scripts/game/devmode.lua` (autoload
`world/autoload/player.bsn`) liga três ferramentas:

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
normal (`vael-tools run`, Continuar, Novo Jogo) o `devmode.lua` não faz nada — as
teclas não existem.

QA pela bridge: o REPL não drena os comandos de script (um `change_scene` de
lá é um no-op), mas partilha a VM dos scripts — `_G.viber_dev = true` liga as
ferramentas na sessão viva, sem reiniciar a engine:

```bash
vael-tools debug --world world.bsn lua '_G.viber_dev = true return true'
vael-tools debug --world world.bsn input key f1     # ou o MCP viber_input
```

Não há save carregado: o estado começa limpo, como no **Novo Jogo** (o kit
inicial de `lib/rules.lua` é semeado). Para testar a partir de um save, usa o
fluxo normal (Continuar) — o atalho é para arranques limpos.

## Como funciona

O sinal é o **nome da entrada de cena**, lido em Lua por
`viber.scene_entry()`. `world.bsn` declara duas entradas na mesma pose
(`WorldNode` com `tag: "SceneEntry"` e os attrs `name`/`translation`):
`spawn` e `dev`.

`spawn` é a **primeira** de propósito: um `change_scene("main")` sem entrada
(o "Novo Jogo" do título) chega por ela e não pelo modo debug. A ordem importa
— não inverter. `--scene main` escolhe o mundo (em vez do
`project.start-scene`); `--entry dev` chega pela segunda e a engine regista
`dev` como entrada da cena. A partir daí:

- `scripts/lib/dev.lua` → `dev.skip_intro()` devolve `true` quando a cena é
  `main` e a entrada é `dev`.
- `src/ui/screens.rs` (`drive_title`) vê o sinal no primeiro tick, aplica a
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
  e os gatilhos `CinematicTrigger` continuam a tocar. Todas são
  `skippable="true"`; a tecla de skip da engine continua a valer.
- **Diálogos, quests, economia, combate e saves** — é o jogo normal.
- **A cena de menu** deixa de ser a primeira, mas continua a existir para o
  botão "Ecrã de título" (a morte e o diário usam-na).

## Alternativas

| Quero… | Comando |
|--------|---------|
| Só saltar a cena de menu, manter título e prólogo | `vael-tools run --scene main` |
| Só saltar o menu e o prólogo (debug) | `vael-tools run --scene main --entry dev` |
| Voltar ao fluxo normal | `vael-tools run` |

`vael-tools run --scene main --entry dev` também aceita caminho explícito
(`vael-tools run world.bsn --scene main --entry dev`); o wrapper do CLI trata do
build.

## Onde está o código

| Ficheiro | Papel |
|----------|-------|
| `scripts/lib/dev.lua` | `dev.entry()` / `dev.skip_intro()` — o sinal |
| `scripts/game/devmode.lua` | [F1] voo livre + [F2] invulnerável (autoload) |
| `scripts/lib/rules.lua` | `set_god`/`M.god` — o único portão do dano ao herói |
| `src/ui/screens.rs` (`drive_title`) | Salta o véu e liga o HUD no primeiro tick |
| `scripts/game/story.lua` | Trava o prólogo (marca persistente) |
| `world.bsn` | `spawn` (normal) e `dev` (debug), nesta ordem — a pose é a mesma |

Se mudares o nome da entrada, muda `DEBUG_ENTRY` em `lib/dev.lua` no mesmo
commit; o resto do jogo lê o sinal por essa constante.

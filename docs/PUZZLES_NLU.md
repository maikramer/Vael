# Enigmas mudos — puzzles não verbais com o NLU como juiz

Seis puzzles em que a **pista não é texto** (som, objetos, estátuas, marcos já
medidos) e a resposta é **uma palavra** (ou três, nas oferendas). O modelo não
gera nada: ele lê a resposta curta, extrai as entidades e valida **tipo +
identidade**, aceitando sinónimos e plurais que uma comparação de strings
rejeitaria.

O que o modelo acrescenta a `answer == "lobo"`:

- aceita `lobo`, `lobos`, `alcateia`;
- exige o **tipo** certo (`criatura`, `item`, `local`), não só a palavra;
- extrai **várias** entidades de uma frase (`pedra, madeira e fruta`) e o
  enigma pode exigir todas;
- no futuro, traduz nomes parciais (`poço` → `Poço da Encruzilhada`).

Os seis vivem em `data/riddles.json`; cada `site` é uma entidade do mundo com
`script="crystal-shrine.lua"` e a `CustomProperty` `riddle_id="<id>"` (no DSL
antigo era o atributo `prop-riddle_id`; o script lê `viber.prop("riddle_id")`).

## Os seis enigmas

### 1. O Eco do Bosque — áudio

- **Local:** `bosque-shrine` (Bosque Encantado, `151 74.04 1302`).
- **Pista:** um `growl` toca a cada ~7 s quando o jogador está a menos de 12 m.
  Não há texto nem ícone: só o som.
- **Pergunta:** "Que criatura é esta?" (`Ouvir o santuário`).
- **Resposta esperada:** `criatura = wolf` — `lobos`, `lobo` ou `alcateia`.
- **Recompensa:** 60 XP + `wolf_pelt:1`.
- **Erro:** "O eco não é de um animal qualquer." (a quest não fecha).

### 2. As Três Ofertas — objetos

- **Local:** `vael-shrine` (Alto de Vael, `882 194.76 782`).
- **Pista:** três props filhos do santuário — `riddle-gift-stone`
  (`rock_boulder`), `riddle-gift-wood` (`dead_tree`) e `riddle-gift-berry`
  (`berry_bush`). O jogador lê a imagem, não uma lista.
- **Pergunta:** "Nomeia as três oferendas." (`Examinar as oferendas`).
- **Resposta esperada (todas):** `item = stone`, `item = wood`,
  `item = berry` — em qualquer ordem: "pedra, madeira e fruta".
- **Recompensa:** 80 XP + `nature_amulet:1`.
- **Erro:** "Uma pesa, outra arde, a última alimenta." (falta uma).

Este é o enigma que mais usa o modelo: uma frase curta, três entidades
extraídas, e a validação exige as três. `lenha` vale por `wood`, `frutas` por
`berry`, e a ordem é livre.

### 3. A Estátua Errada — estátuas

- **Local:** `crystal-shrine` (praça de Discordia, `35 0 17`).
- **Pista:** três estátuas filhas do santuário: dois lobos
  (`riddle-statue-wolf-a`/`-b`, `wolf.*`) e uma sombra (`riddle-statue-shade`,
  `shade.*`). A terceira não pertence ao conjunto.
- **Pergunta:** "Qual não pertence?" (`Examinar as estátuas`).
- **Resposta esperada:** `criatura = shade` — `sombra`/`sombras`.
- **Recompensa:** 50 XP + `potion:1`.
- **Erro:** "Duas uivam; a terceira não tem sombra própria."

### 4. A Rota do Menir — marcos medidos com [F]

- **Local:** `menhir-route-shrine` no **Círculo de Menires**
  (`world/landmarks/forest.bsn`, `111 48.92 242.5`), com três pilares de
  pista (`menhir-route-clue-1..3`).
- **Pista:** os três marcos que o jogador tem de ter **medido com [F]** —
  `require_marks: [desert-arch, desert-sun-obelisk, forest-stone-circle]`.
  Sem os três marcados, o [E] responde "Faltam N marcos por medir." e a barra
  de resposta nem abre.
- **Pergunta:** "Que marco mediste nesta rota?" (`Examinar os marcos`).
- **Resposta esperada:** `expect: [{kind: "local", id: "*"}]` — qualquer um
  dos marcos de `require_marks` (Arco do Deserto, Obelisco do Sol ou Círculo
  de Menires), em `example: "Arco do Deserto"`.
- **Recompensa:** 50 XP + 30 ouro.
- **Erro:** "Diz o nome de um dos três marcos que mediste pelo vale."

Este é o único enigma com `require_marks`: o `*` é resolvido contra essa
lista (`matches_entry`/`decision_expect` em `scripts/lib/riddle.lua`), não
contra todos os locais do jogo. Um `*` sem `require_marks` deixa qualquer
local valer — o lint de conteúdo recusa essa combinação.

### 5. O Espelho de Areia — deserto

- **Local:** `desert-venom-shrine` (`186 58.04 -132`, `world/landmarks/desert.bsn`).
- **Pista:** o cristal do santuário (`crystal_blue`) e os escorpiões que
  pastam na região leste do Ermo (a zona dos `enemies/scorpion.lua` cobre
  `x 124..350`).
- **Pergunta:** "O que arde no espelho de areia?" (`Contemplar o espelho de areia`).
- **Resposta esperada (todas):** `criatura = scorpion` + `item = stone`
  (`cristal` é alias de `stone`), em qualquer ordem.
- **Recompensa:** 45 XP + 25 ouro.
- **Erro:** "Um fere com o ferrão; o outro guarda a luz da duna."

### 6. A Vigília do Gelo — picos

- **Local:** `peaks-echo-shrine` (`-117 85.60 -210`, `world/landmarks/peaks.bsn`).
- **Pista:** o cristal do santuário e o pilar `echo-stone-peaks`
  (`stone_pillar`) ao lado.
- **Pergunta:** "O que vela entre o cristal e o gelo?" (`Escutar a vigília do gelo`).
- **Resposta esperada (todas):** `item = stone` + `criatura = shade`.
- **Recompensa:** 55 XP + 30 ouro.
- **Erro:** "Um brilha nas veias da serra; o outro move-se onde falta a luz."

> **Dívida conhecida:** o pilar e o cristal estão no mundo, mas a **sombra**
> da resposta não tem prop nem spawner nos Picos — os `enemies/shade.lua`
> cobrem o norte (`z 160..360`, Bosque/Alto) e o `shade-night` só aparece à
> noite nesses mesmos sítios. Hoje o jogador chega à resposta pelo `hint`
> (aparece depois de uma tentativa falhada); falta assentar uma estátua de
> sombra no `peaks-echo-shrine` — trabalho de mundo, não de dados.

## Como está montado

| Peça | Onde |
|---|---|
| Dados dos enigmas | `data/riddles.json` |
| Esquema partilhado (round-trip, `extra`) | `content/src/lib.rs` (`RiddlesFile`/`RiddleDef`) |
| Lint de conteúdo | `tools/lint_content.py` |
| Juiz (resolve + valida + recompensa) | `scripts/lib/riddle.lua` |
| Modo enigma do santuário | `scripts/crystal-shrine.lua` (`riddle_id`) |
| Resolvedor de entidades | `scripts/lib/parley.lua` + `data/nlu_aliases.json` |
| Barra de resposta | `scripts/ui/say.lua` + `world/say.bsn` + `ui/hud.xaml` |

O santuário mostra o prompt do enigma (`interact`), toca a pista sonora quando
aplica e, no [E], abre a barra com a pergunta. A resposta passa por:

```
texto -> viber.nlu (entidades) -> parley.resolve (ids do jogo) -> riddle.solve
```

`riddle.solve` usa `all: true` para exigir todas as entidades; nos outros basta
uma. O estado fica em `viber.game()["riddle.<id>"]` (persiste no save) e a
barra só abre se `reward_fits` disser que o prémio cabe na bolsa (senão o item
perdia-se).

Sem NLU (feature desligada ou modelo ainda a carregar), o santuário responde
"O santuário espera uma voz, mas o vento leva-a." e não fecha. Com o d1
pronto, cada `expect` vira uma pergunta `rN` e `solve_decision` exige as
mesmas entidades (com `all` a contar todas).

## Autorar um enigma novo

1. Acrescentar uma entrada em `data/riddles.json` (`site`, `interact`, `clue`,
   `prompt`, `example`, `expect`, `all`, `hint`, `solved`, `reward`;
   `require_marks` se a pista for "marcos já medidos").
2. Pôr `riddle_id="<id>"` no santuário (CustomProperty do Jackdaw), com
   `script="crystal-shrine.lua"`.
3. Se a pista for visual, colocar os props no Jackdaw (painel **Vael:
   Assets**, que assenta no `.ahgt`) como filhos do santuário.
4. Testar o juiz no bridge:
   `viber.load("lib/riddle.lua").self_test()`.

## QA

- `python3 tools/lint_content.py` — exit 0 (sites, marcos, recompensas,
  aliases e placeholders).
- `riddle.self_test()` — `true` (lobos vs sombra, três oferendas completas e
  incompletas, ordem trocada, e a rota do menir com 0/2/3 marcos medidos).
- Teste ao vivo dos santuários requer a cena `main`; a última verificação
  headless foi o `lint_content.py`, que valida os seis `site` contra o
  `world.bsn`.

## Nota de desenvolvimento

`riddle.lua` guarda `local parley = viber.load("lib/parley.lua")` no topo. Um
hot-reload de `parley.lua` não atualiza essa referência (o módulo `riddle` não
é recarregado); ao editar o resolvedor, salvar também `riddle.lua` ou reiniciar.

# Enigmas mudos — puzzles não verbais com o NLU como juiz

Três puzzles em que a **pista não é texto** (som, objetos, estátuas) e a
resposta é **uma palavra** (ou três, nas oferendas). O modelo não gera nada:
ele lê a resposta curta, extrai as entidades e valida **tipo + identidade**,
aceitando sinónimos e plurais que uma comparação de strings rejeitaria.

O que o modelo acrescenta a `answer == "lobo"`:

- aceita `lobo`, `lobos`, `alcateia`;
- exige o **tipo** certo (`criatura`, `item`, `local`), não só a palavra;
- extrai **várias** entidades de uma frase (`pedra, madeira e fruta`) e o
  enigma pode exigir todas;
- no futuro, traduz nomes parciais (`poço` → `Poço da Encruzilhada`).

## Os três enigmas

### 1. O Eco do Bosque — áudio

- **Local:** `bosque-shrine` (Bosque Encantado, `151 74.04 1302`).
- **Pista:** um `growl` toca a cada ~7 s quando o jogador está a menos de 12 m.
  Não há texto nem ícone: só o som.
- **Pergunta:** "Que criatura é esta?"
- **Resposta esperada:** `criatura = wolf` — `lobos`, `lobo` ou `alcateia`.
- **Recompensa:** 60 XP + `wolf_pelt:1`.
- **Erro:** "O eco não é de um animal qualquer." (a quest não fecha).

### 2. As Três Ofertas — objetos

- **Local:** `vael-shrine` (Alto de Vael, `882 194.76 782`).
- **Pista:** três props em volta do santuário: um rochedo (`rock_boulder`), uma
  árvore morta (`dead_tree`) e um arbusto de bagas (`berry_bush`). O jogador lê
  a imagem, não uma lista.
- **Pergunta:** "Nomeia as três oferendas."
- **Resposta esperada (todas):** `item = stone`, `item = wood`,
  `item = berry` — em qualquer ordem: "pedra, madeira e fruta".
- **Recompensa:** 80 XP + `nature_amulet:1`.
- **Erro:** "Uma pesa, outra arde, a última alimenta." (falta uma).

Este é o enigma que mais usa o modelo: uma frase curta, três entidades
extraídas, e a validação exige as três. `lenha` vale por `wood`, `frutas` por
`berry`, e a ordem é livre.

### 3. A Estátua Errada — estátuas

- **Local:** `crystal-shrine` (praça de Discordia, `35 0 17`).
- **Pista:** três estátuas: dois lobos e uma sombra (`wolf_lod0.glb` ×2,
  `shade_lod0.glb`). A terceira não pertence ao conjunto.
- **Pergunta:** "Qual não pertence?"
- **Resposta esperada:** `criatura = shade` — `sombra`/`sombras`.
- **Recompensa:** 50 XP + `potion:1`.
- **Erro:** "Duas uivam; a terceira não tem sombra própria."

## Como está montado

| Peça | Onde |
|---|---|
| Dados dos enigmas | `data/riddles.json` |
| Juiz (resolve + valida + recompensa) | `scripts/lib/riddle.lua` |
| Modo enigma do santuário | `scripts/crystal-shrine.lua` (`prop-riddle_id`) |
| Resolvedor de entidades | `scripts/lib/parley.lua` + `data/nlu_aliases.json` |
| Barra de resposta | `scripts/ui/say.lua` + `world/hud.xml` |

O santuário mostra o prompt (`Ouvir o santuário` / `Examinar as oferendas` /
`Examinar as estátuas`), toca a pista sonora quando aplicável e, no [E], abre a
barra com a pergunta. A resposta passa por:

```
texto -> viber.nlu (entidades) -> parley.resolve (ids do jogo) -> riddle.solve
```

`riddle.solve` usa `all: true` para exigir todas as oferendas; nos outros basta
uma. O estado fica em `viber.game()["riddle.<id>"]` (persiste no save).

Sem NLU (feature desligada ou modelo ainda a carregar), o santuário responde
"O santuário espera uma voz, mas o vento leva-a." e não fecha.

## Enigma seguinte (desenhado, não implementado)

**A Rota do Menir** — uma rota não verbal: três miniaturas de marcos em volta de
um santuário (obelisco, arco, pilar) e a exigência de ter medido os três marcos
reais com [F]. No fim, o jogador diz o nome de **um** dos lugares. O bloco
`require_marks` já existe em `riddle.lua`; o que falta é o sítio e os props.

## Autorar um enigma novo

1. Acrescentar uma entrada em `data/riddles.json` (`site`, `interact`, `clue`,
   `prompt`, `example`, `expect`, `all`, `hint`, `solved`, `reward`).
2. Pôr `prop-riddle_id="<id>"` no santuário (ou noutra entidade com o script
   `crystal-shrine.lua`).
3. Se a pista for visual, inserir os props com
   `viber scene insert 'name=<santuário>' '<Entity…><GltfScene…/></Entity>'` e
   assentar com `viber tool settle 'name=<prop>'`.
4. Testar o juiz no bridge:
   `viber.load("lib/riddle.lua").self_test()`.

## QA

- `viber analyze world.xml` — exit 0.
- `riddle.self_test()` — `true` (lobos vs sombra, três oferendas completas e
  incompletas, ordem trocada).
- Teste ao vivo dos três santuários requer a cena `main`; ficou pendente por
  outra sessão estar a usar a engine em interiores durante a implementação.

## Nota de desenvolvimento

`riddle.lua` guarda `local parley = viber.load("lib/parley.lua")` no topo. Um
hot-reload de `parley.lua` não atualiza essa referência (o módulo `riddle` não é
recarregado); ao editar o resolvedor, salvar também `riddle.lua` ou reiniciar.

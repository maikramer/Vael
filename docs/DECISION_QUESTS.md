# Quests de decisão (d1-omni)

As três quests usam o **d1-omni** como camada de decisão semântica: o jogador
escreve uma pergunta, acusação ou postura em texto livre e o modelo escolhe a
opção mais próxima. As regras, pistas e recompensas continuam determinísticas.

## Como funciona

- `scripts/lib/decide.lua` — ponte para `viber.decision`; constrói `choice`/
  `noul`/`score`, resolve rótulos (`item`, `local`, `criatura`) e verifica
  respostas de relatório.
- `scripts/npc/decision-quest.lua` — motor genérico das quests de decisão. Lê
  `data/quest_puzzles/<id>.json`, aceita a quest, faz as perguntas de pista,
  guarda flags em `viber.game()["puzzle.<id>.<pista>"]`, pede o desfecho e
  entrega com `rules.quest_set_progress` + `quest_turn_in`.
- `scripts/ui/say.lua` — modo decisão da barra de fala (worker thread,
  `submit`/`poll`), com cache do texto ao desfocar e 30 s nos puzzles.
- `scripts/lib/judge.lua` — verificação de ações por `noul` por requisito
  (com o registo determinístico `kind/id/verb` como verdade de base).
- `scripts/lib/riddle.lua` e os relatos dos dadores — `noul` por entidade
  esperada em vez da extração do GLiNER2.

O `choice` do d1 tende a `none` com uma **palavra solta**; por isso o quadro de
avisos e o “perguntar por um lugar” usam os aliases autorais como fallback
determinístico. Em frases, o d1 resolve sinónimos e variações.

## 1. O Enviado de Vidro — `story_glass_envoy`

**Onde:** longhouse da Cidadela (`npcs` em `world/cities/discordia/longhouse.xml`),
o enviado `npc_envoy_glass`.

**Cenário:** o poço da praça foi envenenado. O enviado da Cidadela já tem uma
culpada — a curandeira — e pressiona o jogador a concordar. As luvas de vidro
dele estão na mesa; a lamparina foi vista junto ao poço; o livro da curandeira
só tem ervas. O jogador interroga o enviado por texto livre; ele responde com
contradições.

**Pistas (`data/quest_puzzles/story_glass_envoy.json`):**
`poco` (cheiro de amêndoa amarga), `luva` (selo do enviado), `livro` (ervas,
não veneno), `guarda` (lamparina vista). Três bastam.

**Desfecho:** “Quem envenenou o poço?” → `enviado` (correto), `curandeira`,
`guarda`, `ninguem`. Acertar completa; errar dá uma pista.

**Recompensa:** 180 ouro, 220 XP, 2 poções.

## 2. O Dízimo dos Afogados — `swamp_drowned_bell`

**Onde:** pântano, junto ao barco podre (`npc_bell_drowned`, `world/ai/npcs.xml`).

**Cenário:** uma sineira afogada conta moedas que não existem. O rio leva um nome
por cheia; para passar, o jogador tem de dizer **o que o rio guarda**. As pistas
são sensoriais: um sino abafado ao crepúsculo, bronze verde no fundo, uma torre
que já não está de pé mas continua refletida, e a contagem de um nome por cheia.

**Pistas:** `som`, `bronze`, `reflexo`, `tally`; três bastam.

**Desfecho:** “O que guarda o rio?” → `sino` (correto), `torre`, `crianca`,
`lua`. O jogador pode responder com uma frase (“o sino afogado que toca debaixo
da água”): o d1 escolhe a opção semanticamente mais próxima.

**Recompensa:** 260 ouro, 240 XP, 3 poções de musgo.

## 3. A Testemunha de Vidro — `forest_glass_witness`

**Onde:** Círculo de Menires `(104, 236)`, no altar de vidro
(`npc_glass_witness`, `world/ai/npcs.xml`). A vidente espera a sul do altar,
onde o novo `GroundDecal` “glass.pool” faz o chão ler como um lago de vidro.

**Cenário:** no Círculo de Menires, entre os menires e a luz azul do altar, a
vidente “vê o que a água viu”, mas não decide por ninguém. Um caçador jura que
não matou o cervo do ancião. O jogador pergunta por texto livre:
as marcas são de lobo, a corda do arco está seca, a armadilha tem uma conta de
vidro do irmão, e o irmão saiu com uma rede e voltou sem ela.

**Pistas:** `wolf`, `bow`, `bead`, `brother`; três bastam.

**Desfecho moral:** “O que dizes ao ancião?” → quatro posturas, quatro finais:
- `defender` — defender o caçador e expor o irmão poçoeiro (melhor; +1 poção);
- `acusar` — acusar o caçador (o irmão foge);
- `expor` — expor o irmão sem defender o caçador (+1 pele de lobo);
- `silenciar` — deixar o caçador pagar sozinho.

A quest completa em qualquer uma; o desfecho muda o texto e os itens.

**Recompensa base:** 240 ouro, 260 XP, 2 peles de lobo.

## Adicionar outra quest de decisão

1. Acrescentar a def em `quests/decision_quests.json` (objetivo
   `{"type":"decision","target":"<etiqueta>","count":1}`).
2. Criar `data/quest_puzzles/<id>.json` com `intro`, `ask`, `clues`,
   `required`, `final_prompt`, `final_options`, `final_answer` (ou `endings`).
3. Criar um NPC com `script="npc/decision-quest.lua"` e
   `prop-quest_id="<id>"` (mundo ou cena).
4. O editor e o lint já aceitam `objective.type = "decision"` e não exigem
   `parley` (o relato é o puzzle).

## Validado ao vivo

- Q3 de ponta a ponta: aceitar → barra de fala → “como morreu o cervo” →
  pista `wolf` → final “defendo o caçador e acuso o irmão” → `defender` →
  quest `done` + recompensas.
- Q1/Q2 (decisões): `poco`/`enviado`, `bronze`/`sino`.
- Lugar: “poço da encruzilhada” → `forest-crossroads-well` (23 opções).
- Relatório: “lobos” → `noul` 0,695; juiz: `matar lobo` 0,592 contra 0,14/0,15
  das outras opções.
- `viber analyze` OK.

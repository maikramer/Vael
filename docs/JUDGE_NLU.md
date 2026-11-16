# O altar que julga — modelo como juiz de ações

A ideia: o herói **faz** (mata, visita, equipa), o mundo **traduz** essas ações
em frases, o modelo **lê** as frases e o juiz **decide** se o conjunto cumpre a
regra. O veredito volta ao mundo como reação (recompensa, porta, armadilha).

## O que o modelo NÃO faz (testado)

A cabeça de classificação do GLiNER2 foi testada como juiz zero-shot
(`Regra: … Ação: …` → `correto/incorreto`): **5/8** acertos, e com
`sim/não` ou `cumpre/viola` respondeu sempre o primeiro rótulo (0/8). O modelo
foi treinado para extrair spans e classificar por rótulos, **não para
raciocinar** sobre uma regra. Usá-lo como oráculo daria vereditos errados com
confiança.

## O que ele faz bem (e é o que usamos)

O modelo é o **tradutor semântico**:

- lê a frase da ação e extrai as entidades (`criatura`, `item`, `local`);
- aceita sinónimos e plurais (`lobo`/`lobos`/`alcateia`, `tocha`/`fogo`);
- extrai várias entidades de uma frase;
- no futuro, aceita nomes parciais (`poço` → `Poço da Encruzilhada`).

O **juiz** é simbólico: requisitos, ordem e proibições. É determinístico,
explicável e testável — e o modelo é o que permite escrever as ações e as
regras em português em vez de comparar ids.

## As quatro regras (`data/verdicts.json`)

Cada regra é um altar próprio: `script="judge-site.lua"` e a CustomProperty
`verdict_id="<id>"` (o script lê `viber.prop("verdict_id")`). O primeiro [E]
lê as marcas; os seguintes julgam o registo.

| id | site | `require` (com verbo, por esta ordem) | `forbid` | prémio |
|---|---|---|---|---|
| `hunter_oath` | `judge-oath` (Bosque Encantado, a ~6 m do `bosque-shrine`) | matar `wolf` → visitar `forest-crossroads-well` → equipar `torch` | — | 120 XP + `blessed_rod:1` |
| `desert_ascetic` | `judge-desert` (`world/landmarks/desert.bsn`) | visitar `desert-arch` → matar `scorpion` → equipar `sword` | `item potion` | 50 XP + 30 ouro |
| `swamp_purifier` | `judge-swamp` (`world/landmarks/swamp.bsn`) | visitar `swamp-shack` → matar `slime` → usar `antidote` | `criatura bandit` | 55 XP + 35 ouro |
| `peaks_vigil` | `judge-peaks` (`world/landmarks/peaks.bsn`) | equipar `torch` → matar `shade` → visitar `peaks-crystal-mine` | — | 65 XP + 40 ouro |

O `hunter_oath` é o único com **pista não verbal**: três props filhos do
altar, na ordem — `judge-clue-wolf` (`wolf`), `judge-clue-well`
(`medieval_well`) e `judge-clue-torch` (`torch_post`). As outras três regras
dizem-se pelo `intro` no primeiro [E] (o altar não tem props de pista).

`torch` e `sword` são os **kinds da arma na mão** (`scripts/game/equipment.lua`,
o V troca Espada↔Tocha), não itens do `items.json`; `antidote`, `potion` e as
criaturas são ids do conteúdo normal.

### Regra (`data/verdicts.json`)

```json
{
  "id": "hunter_oath",
  "site": "judge-oath",
  "intro": "Três marcas, na ordem. O altar pesa o que fizeste.",
  "require": [
    {"kind": "criatura", "id": "wolf", "verb": "matar"},
    {"kind": "local", "id": "forest-crossroads-well", "verb": "visitar"},
    {"kind": "item", "id": "torch", "verb": "equipar"}
  ],
  "order": ["matar", "visitar", "equipar"],
  "forbid": [],
  "window": 900,
  "verdicts": {"correct": "…", "partial": "…", "wrong": "…"},
  "reward": {"xp": 120, "items": ["blessed_rod:1"]}
}
```

- `require` — todas têm de aparecer; um `verb` só casa com uma ação desse
  verbo (a mesma ação não cumpre dois requisitos).
- `order` — as ações dos requisitos têm de aparecer por esta ordem no registo.
- `forbid` — qualquer entidade proibida invalida o veredito (`wrong`), seja
  qual for o verbo.
- `window` — **reservado**: o `judge.lua` não o usa (o `t0` da leitura das
  marcas já limita a janela). Fica no schema para o editor não o perder.
- `reward` — só no `correct`; `judge.<id>.done` marca a regra no save.

Um requisito `local` tem de ser um marco que uma quest `visit` liste (o lint
de conteúdo recusa um `local` fora desses tokens): assim há sempre um caminho
que escreve `visitar` — a varredura de pisada da quest. O [F] escreve-o na
mesma para qualquer marco, tenha ou não quest.

## Registo de ações (`scripts/lib/actions.lua`)

Cada ação vira uma frase e um registo `{verb, kind, id, text, t}`:

```
o herói matou lobo em Floresta Sombria
o herói visitou Poço da Encruzilhada
o herói equipou tocha em Floresta Sombria
o herói usou poção em Floresta Sombria
o herói comprou poção em Discordia
o herói vendeu pele de lobo em Discordia
```

As entradas vivem no **store partilhado** do `viber.game()` (`act.seq` é o
próximo índice; cada ação ocupa `act.<n>.v/.k/.i/.t`), por isso o Lua e o
Rust escrevem no mesmo registo. Cap de **32** entradas; o `actions.since(t0)`
descarta as entradas de um processo anterior (`t` no futuro face a
`viber.time()`, que é o `Time` do Bevy).

Ganchos ligados:

| Ação | Onde |
|---|---|
| `matar` | `rules.award_kill` (`lib/fsm.lua` manda o abate) |
| `visitar` | `rules.report_visit` (varredura de pisada, com a quest `visit` ativa), **[F]** em `src/ui/map.rs` (escreve `act.*` directamente) e o aceitar de uma quest (credita marcos já medidos) |
| `equipar` | `game/equipment.lua` (V: Espada/Tocha) e o ecrã de equipamento em `src/ui/screens.rs` (itens com `slot`) |
| `usar` | `game/hotbar.lua` (poção/antídoto/poção de musgo) |
| `comprar` / `vender` | `game/shop.lua` |
| `forjar` | `lib/craft.lua` |
| `entoar` | `lib/voice.lua` |

O **binómio do altar**: o `[F]` que mede um marco de uma quest de visita conta
como `visitar` para o juiz; uma simples pisada no raio do marco só conta
enquanto a quest `visit` que o lista estiver **ativa** (é o `scan_visits` que
chama o `report_visit`).

## O juiz (`scripts/lib/judge.lua`)

1. Lê as entradas desde `t0` (cap 16 para não custar 16 inferências).
2. Para cada frase, chama `viber.nlu.parse` e resolve com `parley`.
3. Faz o fallback determinístico com o `kind/id` guardado no registo (sem
   modelo, o jogo não fecha).
4. Calcula `correct` / `partial` / `wrong` com `require` + `order` + `forbid`:
   - `correct` — todas as marcas, na ordem certa;
   - `partial` — todas as marcas mas ordem trocada (`reason = "order"`), ou
     parte das marcas (`reason = "missing"`) — a janela continua aberta e o
     próximo [E] volta a julgar o mesmo registo;
   - `wrong` — uma proibição (`reason = "forbidden"`), nenhuma marca
     (`reason = "empty"`) ou nada feito; o próximo [E] **reinicia** a janela
     (`judge.reset`, as ações antigas não mancham a tentativa nova).
5. `judge.reward` paga o `correct` e marca `judge.<id>.done` no save.

O `judge-site.lua` imprime o `intro` no primeiro [E] e acrescenta
`Marcas cumpridas: got/need` (ou `A ordem ritual foi quebrada`, ou
`Um tabu violou o voto`) ao texto do veredito — o jogador sabe o que faltou
sem texto gerado.

## Autorar uma regra nova

1. Acrescentar a regra em `data/verdicts.json`.
2. Pôr `verdict_id="<id>"` numa entidade com `script="judge-site.lua"`
   (CustomProperty do Jackdaw).
3. Rodear o sítio com os props que contam a pista (sem texto) — ou escrever a
   regra por palavras no `intro`.
4. Testar:
   ```lua
   viber.load("lib/judge.lua").self_test()
   ```
   e, no bridge, injetar um log sintético:
   ```lua
   local a = viber.load("lib/actions.lua")
   local j = viber.load("lib/judge.lua")
   a.clear(); j.start("hunter_oath")
   a.record("matar","criatura","wolf")
   a.record("visitar","local","forest-crossroads-well")
   a.record("equipar","item","torch")
   return j.evaluate("hunter_oath").verdict
   ```

## QA

- `python3 tools/lint_content.py` — exit 0 (sites no mundo, requisitos
  `local` cobertos por quests `visit`, verbos vs `order`, textos e prémios).
- `judge.self_test()` — `true` (ordem certa, trocada, falta uma, proibido,
  nada feito; e um caso por cada uma das quatro regras reais).
- No bridge, com o modelo no loop: `correct got=3/3 order=true model=true`;
  ordem trocada → `partial`.
- O teste ao vivo dos altares exige a cena `main`; a última verificação
  headless é o `lint_content.py` (os quatro `site` existem no `world.bsn`).

## Próximos incrementos (o que torna isto único)

- **Quantidades e tempo**: `matar 3 lobos`, `visitar de noite`, `usar antes de
  matar` — o registo já tem `t` e o modelo extrai `quantidade`, mas o juiz
  ainda só compara verbos/entidades.
- **Regras em prosa** compiladas no build: o designer escreve "mata um lobo,
  visita o poço e depois acende a tocha"; um script usa o modelo para extrair
  `require`/`order` e escreve o JSON (o mesmo padrão do
  `build_quest_index.py`).
- **Veredito como ação do mundo**: hoje o `correct` só paga o `reward`; falta
  abrir portas, armar armadilhas, mexer no preço da loja ou na reputação do
  NPC.
- **Proibições sociais** mais largas: "não mates o guarda", "não uses fogo na
  floresta" — o `forbid` já existe (`forbid` por entidade, sem verbo); falta
  conteúdo que o use para além dos dois altares atuais.

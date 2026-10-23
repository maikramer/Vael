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

## O protótipo: `hunter_oath`

Local: `judge-oath`, a 6 m do `bosque-shrine` (Bosque Encantado).

- **Pista não verbal:** três props na ordem — uma estátua de lobo, um poço em
  miniatura e um poste de tocha.
- **Primeiro [E]:** o altar lê as marcas e abre a janela.
- **Ações exigidas, por esta ordem:**
  1. `matar` um lobo;
  2. `visitar` o Poço da Encruzilhada (com [F] ou pela quest `forest_survey`);
  3. `equipar` a tocha (V).
- **Segundo [E]:** o juiz lê o registo desde a leitura das marcas e responde:
  - `correct` (3/3, ordem certa) → 120 XP + vara abençoada;
  - `partial` (falta uma ou a ordem ainda não fechou) → aviso, pode continuar;
  - `wrong` (nenhuma marca ou ordem impossível) → a janela reinicia no
    próximo [E].

### Regra (`data/verdicts.json`)

```json
{
  "id": "hunter_oath",
  "intro": "Três marcas, na ordem. O altar pesa o que fizeste.",
  "require": [
    {"kind": "criatura", "id": "wolf", "verb": "matar"},
    {"kind": "local", "id": "forest-crossroads-well", "verb": "visitar"},
    {"kind": "item", "id": "torch", "verb": "equipar"}
  ],
  "order": ["matar", "visitar", "equipar"],
  "forbid": [],
  "verdicts": {"correct": "…", "partial": "…", "wrong": "…"},
  "reward": {"xp": 120, "items": ["blessed_rod:1"]}
}
```

- `require` — todas têm de aparecer (cada uma com o verbo, se indicado).
- `order` — os índices dos matches no registo têm de ser crescentes.
- `forbid` — qualquer entidade proibida invalida o veredito.
- `reward` — só no `correct`.

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

Ganchos já ligados:

| Ação | Onde |
|---|---|
| `matar` | `rules.award_kill` |
| `visitar` | `rules.report_visit` (inclui [F] em marcos com `survey`) |
| `equipar` | `game/equipment.lua` (V) |
| `usar` | `game/hotbar.lua` (poção/antídoto) |
| `comprar` / `vender` | `game/shop.lua` |

O registo vive em módulo (sessão), com cap de 32 entradas. O juiz filtra por
`t0`, por isso ações anteriores à leitura da pista não contam.

## O juiz (`scripts/lib/judge.lua`)

1. Lê as entradas desde `t0` (cap 16 para não custar 16 inferências).
2. Para cada frase, chama `viber.nlu.parse` e resolve com `parley`.
3. Faz o fallback determinístico com o `kind/id` guardado no registo (sem
   modelo, o jogo não fecha).
4. Calcula `correct` / `partial` / `wrong` com `require` + `order` + `forbid`.
5. `judge.reward` paga o `correct` e marca `judge.<id>.done` no save.

## Autorar uma regra nova

1. Acrescentar a regra em `data/verdicts.json`.
2. Pôr `prop-verdict_id="<id>"` numa entidade com `script="judge-site.lua"`.
3. Rodear o sítio com os props que contam a pista (sem texto).
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

- `viber analyze world.xml` — exit 0.
- `judge.self_test()` — `true` (ordem certa, trocada, falta uma, proibido,
  nada feito).
- No bridge, com o modelo no loop: `correct got=3/3 order=true model=true`;
  ordem trocada → `partial`.
- O prompt ao vivo do `judge-oath` ficou pendente: a engine partilhada caiu
  durante o reload da cena (sessões concorrentes). O sítio está no XML, com os
  props assentados; o script compila no `analyze`.

## Próximos incrementos (o que torna isto único)

- **Mais verbos**: oferecer, acender, dormir, dar, mentir, subornar — cada um
  com o seu gancho, e o juiz passa a avaliar conduta, não só progresso.
- **Regras em prosa** compiladas no build: o designer escreve "mata um lobo,
  visita o poço e depois acende a tocha"; um script usa o modelo para extrair
  `require`/`order` e escreve o JSON (o mesmo padrão do `build_quest_index.py`).
- **Quantidades e tempo**: `matar 3 lobos`, `visitar de noite`, `usar antes de
  matar` — o registo já tem `t` e o modelo extrai `quantidade`.
- **Proibições sociais**: "não mates o guarda", "não uses fogo na floresta" —
  `forbid` já existe.
- **Veredito como ação do mundo**: portas, armadilhas, descontos, reputação de
  NPC, uma quest que só abre para quem passou no julgamento.
- **Explicação do veredito**: o diff (`missing`, `forbidden`, `order_ok`) já é
  estruturado; o NPC pode dizer exatamente o que faltou, sem gerar texto.
- **Múltiplas testemunhas**: dois altares com regras diferentes sobre o mesmo
  registo de ações — o mesmo ato, vereditos distintos.

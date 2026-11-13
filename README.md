# Vael

Porta do exemplo original (motor TS) para a engine nativa Viber.
O mundo foi convertido uma única vez por um script de migração (já removido).
O jogo
corre de ponta a ponta: as Fases 0–3 da engine estão feitas e as tags do
mundo original já têm runtime, incluindo as tags `EngineConfig`
(`NavMesh`, `SpawnGate`, `ProjectileTemplate`, `AdaptiveQuality`,
`PostFxDebugToggle`).

## Só estático

`world.bsn` + `world/**/*.bsn` + `world.data/` **são o mundo** — estático
(`static="true"`), a fonte da verdade: terreno final pré-gerado
(`world.data/terrain.ahgt`), geradores resolvidos em grupos `spawn/…`
editáveis (receita `<Scatter>` ao lado), vegetação em `.vinst`. É o que corre
e o que se edita no Jackdaw; `viber analyze`/`viber scene lint` e `viber bsn`
leem `.bsn`. O `world.xml` e os módulos `.xml` ficam como legado da CLI de
cena (`viber scene`/`tool`/`terrain`). Não há fonte procedural:
foi convertida no sítio e apagada (2026-10-14; fica no histórico do git).

Regras de edição e QA: [`AGENTS.md`](AGENTS.md). Contrato da engine:
`../Viber/AGENTS.md`, secção "Mundos estáticos + edição por agentes".

## Correr

```bash
cd ../viber-tools && tools/install.sh   # uma vez
cd ../Vael && viber run                 # procura world.bsn
viber analyze world.bsn                 # headless
viber scene lint --world world.bsn
```

O comando `viber` é agora o router do **`viber-tools`** (`~/.local/bin/viber`
→ `viber-tools`). O wrapper antigo do Viber fica preservado como
`viber-engine` e só é usado para `bake`/`tool scatter`/`trace`/`lua` enquanto
a extração não termina. `viber run` lança o binário do Vael; a engine `viber`
continua a ser a dependência de gameplay nesta fase. Fluxo completo (LFS,
assets, editor): [`AGENTS.md`](AGENTS.md); detalhe do CLI novo:
[`docs/VIBER_TOOLS.md`](docs/VIBER_TOOLS.md).

### Arranque directo (debug)

Para iterar em gameplay sem passar pelo menu e pelo prólogo de ~35 s:

```bash
viber run --scene main --entry dev
```

Cai directo em `main`, com o HUD ligado e o estado de um **Novo Jogo** (sem
save carregado). A entrada `dev` de `world.bsn` é o sinal lido por
`scripts/lib/dev.lua`; o título (`src/ui/screens.rs`) e o prólogo
(`game/story.lua`) saltam-se sem tocar na engine. Detalhe, efeitos no save e
o que continua a tocar: [`docs/DEBUG.md`](docs/DEBUG.md).

## Profiling (sem screenshots)

Com o jogo vivo (`viber run --bridge`), o runtime novo (`viber_tools_runtime`)
responde a frames/p95/hitches/RSS e input; `viber tools mcp` expõe o mesmo aos
agentes:

```bash
viber debug prof --tab frames              # frame avg/p95/max, hitches, RSS
viber debug prof --hitches                 # hitches recentes
viber debug prof --record 30 --hz 2 -o perf.jsonl   # série para análise
viber debug key f3 --hold-ms 120           # input sintético
```

`trace`, `lua`, `scene --live` e `tool --live` ainda são servidos pelo
`viber-engine` legado durante a transição; `viber debug trace`/`lua` delegam
nele. No jogo, **P → FRAMES** e **F3** continuam a mostrar o profiler da
engine. Limiar de hitch do runtime novo: `VIBER_HITCH_MS` (default 50 ms).

## Estado atual

Jogável de ponta a ponta: **ecrã de título** (Continuar/Novo Jogo/
Definições/Créditos/Sair), **morte** com Tentar de novo/Carregar/Título,
**player** com combate melee ([J]/clique, alvo [V]
ou ação `lock` — [Z]/gamepad, dash [C], golpe radial [R], bomba [B], guard
[L], talentos com [Q]→Talentos),
**37 quests** (JSON em `quests/`, com etapas e cadeias por dador, diálogo [E] com os NPCs, tracker no HUD,
bounties no notice-board; os NPCs ambientais usam grafos em
`dialogues/*.json`), **economia** (vault gold/wood/stone, colheita de
árvores/pedras, loja no diário [Q]→Loja — falar com o Bram [E] abre-a —,
hotbar [1]/[2], despojos de ouro/poções nos abates), **travel** (A Nota —
12 marcos para assinar com [F], viagem rápida [G] nas fogueiras, mapa
visual [M] com hillshade do heightmap e seta para o objetivo da quest),
**save em slots** (0 = autosave em fogueiras/entrega de quest/chefe,
1–3 manuais em [Q]→Sistema; JSON em `~/.local/share/viber/`), **mundo vivo**
(céu procedural + ciclo dia/noite, clima, fog/tint por BiomeRegion, BGM com
crossfade por zona e música de combate, SFX, emissores de partícula,
reverb em interiores), **combate com feedback** (hit-stop, shake, números de
dano, sparks e decals de impacto) e **física** (character controller
cinemático, knockback, destrutíveis que tombam, ragdoll nos chefes). Os ~65
scripts Luau em `scripts/` (inimigos, bosses, colheita, POIs, HUD) correm via
a API `viber.*` com "LOD de IA" — além do raio de ativação (default 45 m) o
`on_update` nem corre — e têm hot-reload ao gravar. A referência da API está
em `Viber/docs/LUA_API.md`.

O `analyze` imprime o relatório de cobertura — é o roteiro do que falta à
engine.

### Combate (lib/fsm.lua)

Os golpes inimigos são **legíveis**: a criatura trava, toca o clip `attack`
e pulsa um anel âmbar no chão durante o *windup*; o dano só entra se o herói
ainda estiver ao alcance no impacto — recuar, dash [C] ou aparar [L] no
tempo certo resolvem (sai "ESQUIVA"). Depois do golpe há uma janela de
punição (*recover*). Atingir uma criatura fá-la vacilar (clip `hit`) e
cancela o windup. Os **chefes** (Bruxa, Verme, Guardião do Lodo, Ogro) têm
barra própria no HUD, *poise* (só vacilam com dano acumulado), um especial a
cada 3 golpes com anel de aviso colorido (a maldição da Bruxa cai onde o herói
estava) e fúria abaixo de ~40 % de vida. Cada um tem dador de quest.

**Atiradores e elites:** os bandidos do ermo oriental (`enemies/bandit-archer.lua`)
mantêm 5–15 m e disparam o `enemy-arrow` (anel laranja = vai disparar; sai da
linha). Qualquer criatura não-chefe tem 8 % de nascer **elite** — maior, 2,5× a
vida, golpes ×1,4, aura dourada ao acordar — e paga a triplicar (`game/loot.lua`).

**Guia de missões:** o NPC com missão tem "!" (por aceitar) ou "?" (por
entregar) a flutuar e vira-se para o herói; a seta do HUD (`<WaypointArrow>`)
aponta entregar → cumprir (marco por visitar / presa viva mais próxima) →
missão nova. Minimapa e bússola só mostram NPC que valem a pena.

**Exploração:** baús escondidos junto a seis marcos (caravana, barco, menires,
altar, obelisco, mojão) com loot próprio (`prop-*`).

### Progressão, forja e mundo vivo (2026-11)

- **Equipamento** (`lib/gear.lua`, `data/items.json`): itens com `slot`
  (`weapon`/`armor`/`charm`), `effects` (somam aos talentos; lê-se com
  `rules.stat("chave")`) e `weapon` (dano/cadência/alcance do melee). Clicar
  numa célula da Mochila equipa ou guarda.
- **Forja** (`lib/craft.lua`, `data/recipes.json`): [E] na bigorna abre o
  separador Forja. Os materiais caem dos abates (`game/loot.lua`).
- **Talentos com escolha**: `excludes` em `data/progression.json` fecha o ramo
  irmão (Baluarte ↔ Sede de sangue, Brasas ↔ Abalo, …).
- **Cantil** [E]: a cura gasta cargas (2 + talento) que só voltam na fogueira,
  no curandeiro ou ao renascer. Esquiva [C] recarrega em 3 s.
- **Morte**: 25 % do ouro fica numa bolsa onde o herói caiu (brilha; passar por
  cima recupera). Morrer outra vez antes perde-a.
- **Estados**: veneno, queimadura e gelo no herói (`rules.apply_status`);
  queimadura, gelo e choque nas criaturas (`rules.enemy_status`). A poção de
  musgo [3] limpa os do herói. `weak = { burn = 2 }` no script da criatura.
- **Arquétipos** (`lib/fsm.lua`): `charge` (goblin bruto), `shield` (bandido
  de escudo — o remate da chain parte a guarda), `caster` (bogling
  feiticeiro), e `phases` nos chefes (invocar, enterrar, saltar, investir).
- **Renascer e noite** (`game/spawns.lua`): criaturas voltam ao covil 4 min
  depois, longe da vista (os Picos só depois do Ogro cair); entre as 20h e as
  5h nascem sombras da noite fora da vila e os aldeões da rua recolhem. À
  chuva o fogo apaga-se depressa e as criaturas veem pior (`lib/clock.lua`).
- **Quests**: `stages` (etapas), `requires`, `next` (cadeia do mesmo dador),
  objetivo `craft` e alvo `elite`. Ver `quests/chain_quests.json`.
- **Idioma**: `lib/i18n.lua` traduz toasts, prompts, balões e listas a partir
  de `locale/strings_en.json` (a frase PT é a chave); a UI estática usa
  `@chave` em `locale/{pt,en}.json`. `tools/i18n_extract.py` lista o que falta.
- **Opções**: volume por bus (geral, música, efeitos, ambiente).
- **Gate local**: `tools/check.sh` (lint de conteúdo, sintaxe Lua, `analyze`,
  `scene lint`, traduções EN obrigatórias); `tools/check.sh --full` corre
  também os testes do crate `vael`.

## Fronteira engine/jogo

A engine é generalista: render, física, animação, partículas, áudio,
terreno, streaming, save, input, UI declarativa, cinemáticas e a API de
script. **Não** tem combate, colheita, cofre, XP, quests, talentos, viagem,
loja, lock-on, respawn, veneno nem intérprete de diálogo. Essas regras vivem
todas em Lua:

| Sistema | Ficheiro |
|---------|----------|
| melee/chain/crítico/execução | `scripts/game/combat.lua` |
| dash/cura/radial/bomba/guarda | `scripts/game/abilities.lua` + `lib/rules.lua` |
| colheita (golpes, loot, XP) | `scripts/game/harvest.lua` |
| arma na mão (attach ao osso) | `scripts/game/equipment.lua` |
| quests, cofre, XP, veneno, vitais | `scripts/lib/rules.lua` |
| talentos/passivas | `src/ui/screens.rs` (`skill_rows`) + `lib/rules.lua` |
| viagem/Nota/mapa | `src/ui/map.rs` + `src/ui/waypoint.rs` + `lib/landmarks.lua` |
| loja e despojos | `scripts/game/shop.lua`, `game/loot.lua` |
| diálogo por grafo | `scripts/townsfolk.lua` + `dialogues/*.json` |
| IA/FSM das criaturas | `scripts/lib/fsm.lua` + `enemies/`, `bosses/` |

A engine expõe os DADOS autorais (`viber.player_nearby` com `props`,
`viber.markers`) e as primitivas (`entity_damage`, `burst`, `play_clip`,
`prompt`, `topple`, `attach`, `spawn_prototype`…); nunca decide quando os
usar. Os ~2.700 props `destructible=` viajam como props autorais; o parser
do component-string e as regras estão em `game/harvest.lua`.

As teclas de debug ([H] dano, [N] cura total, [K] +XP, F6/F7/F9/F10/F11
teleportes e recursos) só existem com `VIBER_DEBUG_KEYS=1`.

Assets: `/assets/…` resolve contra as asset roots do `config.yaml` deste
jogo — a pasta do jogo SEMPRE primeiro (overrides), o pool partilhado
`assets/public` declarado em
`assets.roots` (`../Viber/docs/ASSETS.md`). A engine
descomprime meshopt/KTX2 à leitura.

## Estrutura

- `world.bsn` — **raiz do mundo (estático, fonte da verdade)**:
  `<world static="true">`, módulos `<Include>` em `world/**/*.bsn`, terreno no
  heightmap final `world.data/terrain.ahgt`. O `.xml` legado fica ao lado
- `world/**/*.bsn` — módulos do mundo (um ficheiro por include, cada um marcado
  `static="true"`): cidades, criaturas, landmarks, `spawn/…` (saída dos
  geradores), vegetação com `instances="world.data/veg/NNN.vinst"`, interiores
- `world.data/` — dados binários: `terrain.ahgt` (+ `.paint`), `veg/*.vinst`,
  `bake.json` (documento da conversão — World Overview)
- `scenes/menu.bsn` + `scenes/menu.data/terrain.ahgt` — cena de arranque
  (`project.start-scene`): acampamento pequeno atrás do título. As salas
  continuam em `scenes/interiors/*.bsn`
- `quests/*.json` + `quests/index.json` — definições das 37 quests (lidas por
  `scripts/lib/rules.lua`; campos: `id, npc, biome, title, lines_*, objective, rewards`)
- `dialogues/*.json` — grafos de diálogo ambientais (lidos por `scripts/townsfolk.lua`)
- `data/items.json`, `data/landmarks.json`, `data/progression.json` — itens/loja,
  marcos da Nota e progressão (XP/talentos), editáveis como conteúdo
- `editor/` — editor de conteúdo sobre Jackdaw (`vael-editor`): painéis de
  quests, diálogos, itens, marcos, progressão e validação; ver
  [`editor/README.md`](editor/README.md). `content/` (`vael-content`) tem os
  esquemas e as regras partilhadas com o jogo
- `scripts/**.lua` — comportamento Luau (inimigos/bosses, colheita, POIs,
  quests, diálogo) via a API `viber.*` (`Viber/docs/LUA_API.md`)
- `config.yaml` — **contrato de paths com a engine** (obrigatório; roots,
  dirs de áudio/texturas/scripts/UI, save) — `Viber/docs/ASSETS.md`
- `ui/*.xaml` + `ui/*.css` — HUD, menu, título, morte e mapa em bevy_pf;
  `world/menu.xml`/`title.xml`/`death.xml` ficam só para o estado dos modais
- `qa-*.bsn` / `ui-showcase.bsn` — mundos QA, também estáticos (incluem
  módulos partilhados de `world/**/*.bsn`)

## Editar

Por operações — `viber scene` / `viber tool` / `viber terrain` (offline ou
`--live`). A autoria do mundo e das cenas é no **Jackdaw oficial**, com este
projeto já preparado (`jackdaw.toml`):

```bash
jd open .                 # abre o Vael no editor oficial
jd build                  # reconstrói o schema do jogo (Play/PIE)
```

As extensões do Vael (terreno voxel, painéis de conteúdo) são projetos em
`../bevy_extensions/extensions/`; empacotam-se e instalam-se como `.jdext`
assinado:

```bash
cd ../bevy_extensions/extensions/bevy_voxel_terrain_jackdaw
jd build && jd extension pack
jd extension install bevy_voxel_terrain_jackdaw-*-x86_64-unknown-linux-gnu.jdext --trust-publisher
```

O editor abre `world.bsn` e as cenas `scenes/**/*.bsn`; a CLI de cena
(`viber scene`/`tool settle`/`scatter`) continua XML e edita os legados.
Re-gerar a partir do exemplo original do motor TS deixou de ser o fluxo:
produz um mundo procedural que teria de ser convertido (`viber bake`) e
substituiria o estático editado. GLBs novos entram por `assets/`
(`gameassets resume`); `viber analyze` confirma que os URLs resolvem.

## Fila aberta (na engine, não neste mundo)

- Instancing GPU para vegetação (hoje cap 800 instâncias/tag)

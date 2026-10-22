# Vael

Porta do exemplo original (motor TS) para a engine nativa Viber.
O mundo foi convertido uma única vez por um script de migração (já removido).
O jogo
corre de ponta a ponta: as Fases 0–3 da engine estão feitas e as tags do
mundo original já têm runtime, incluindo as tags `EngineConfig`
(`NavMesh`, `SpawnGate`, `ProjectileTemplate`, `AdaptiveQuality`,
`PostFxDebugToggle`).

## Só estático

`world.xml` + `world/**` + `world.data/` **são o mundo** — estático
(`static="true"`), a fonte da verdade: terreno final pré-gerado
(`world.data/terrain.ahgt`), geradores resolvidos em grupos `spawn/…`
editáveis (receita `<Scatter>` ao lado), vegetação em `.vinst`. É o que corre
e o que se edita (agentes e ViberEditor, com paridade total via
`viber::scene`). Não há fonte procedural: foi convertida no sítio e apagada
(2026-10-14; fica no histórico do git).

Regras de edição e QA: [`AGENTS.md`](AGENTS.md). Contrato da engine:
`../Viber/AGENTS.md`, secção "Mundos estáticos + edição por agentes".

## Correr

```bash
install -m 755 ../Viber/scripts/viber-latest.sh ~/.local/bin/viber   # uma vez
cd ../Viber && cargo run -- run ../Vael/world.xml
cd ../Viber && cargo run -- analyze ../Vael/world.xml   # headless + cobertura
```

Com o CLI instalado e o cwd nesta pasta: `viber run` (procura `world.xml`).
O wrapper segue o checkout e só recompila quando as fontes mudam; não correr
`./install.sh` por cima (substitui o wrapper). Fluxo completo (LFS, assets,
editor): [`AGENTS.md`](AGENTS.md).

### Arranque directo (debug)

Para iterar em gameplay sem passar pelo menu e pelo prólogo de ~35 s:

```bash
viber run --scene main --entry dev
```

Cai directo em `main`, com o HUD ligado e o estado de um **Novo Jogo** (sem
save carregado). A entrada `dev` de `world.xml` é o sinal lido por
`scripts/lib/dev.lua`; o título (`ui/title.lua`) e o prólogo
(`game/story.lua`) saltam-se sem tocar na engine. Detalhe, efeitos no save e
o que continua a tocar: [`docs/DEBUG.md`](docs/DEBUG.md).

## Profiling (sem screenshots)

Com a engine viva (`viber run --bridge`), o CLI responde ao ciclo completo:

```bash
viber debug prof --tab frames              # frame avg/p95/max, topo, hitches, memória
viber debug prof --hitches                 # cada engasgo + o sistema que o causou
viber debug prof --record 30 --hz 2 -o perf.jsonl   # série + resumo
viber debug trace 10 -o trace.json         # Perfetto (chrome://tracing / ui.perfetto.dev)
viber debug trace --seconds 0              # gravação manual…
viber debug mark "entrei na vila"          # …com marcas no trace
viber debug trace --stop -o trace.json
```

O `trace` imprime o resumo analítico (p50/p95/p99, frames acima do orçamento,
top sistemas por total e por pico, hitches e contadores) e o ficheiro abre em
Perfetto. No jogo, **P → FRAMES** mostra a mesma série, os hitches e os
botões TRAÇAR/PARAR/EXPORTAR; **F3** passou a incluir p95, topo do frame,
hitches e RSS. Limiar de hitch: `VIBER_HITCH_MS` (default 50 ms).

## Estado atual

Jogável de ponta a ponta: **ecrã de título** (Continuar/Novo Jogo/
Definições/Créditos/Sair), **morte** com Tentar de novo/Carregar/Título,
**player** com combate melee ([J]/clique, alvo [V]
ou ação `lock` — [Z]/gamepad, dash [C], golpe radial [R], bomba [B], guard
[L], talentos com [Q]→Talentos),
**28 quests** (JSON em `quests/`, diálogo [E] com os NPCs, tracker no HUD,
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
| quests, cofre, XP, veneno, HUD | `scripts/lib/rules.lua` |
| talentos/passivas | `scripts/ui/journal.lua` + `lib/rules.lua` |
| viagem/Nota/mapa | `scripts/game/travel.lua` + `lib/landmarks.lua` |
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

- `world.xml` — **raiz do mundo (estático, fonte da verdade)**:
  `<world static="true">`, `<Include src="/world/…">`, terreno no heightmap
  final `world.data/terrain.ahgt`
- `world/**.xml` — módulos do mundo (um ficheiro por include, cada um marcado
  `static="true"`): cidades, criaturas, landmarks, `spawn/…` (saída dos
  geradores), vegetação com `instances="world.data/veg/NNN.vinst"`, interiores
- `world.data/` — dados binários: `terrain.ahgt` (+ `.paint`), `veg/*.vinst`,
  `bake.json` (documento da conversão — World Overview do editor)
- `scenes/menu.xml` + `scenes/menu.data/terrain.ahgt` — cena de arranque
  (`project.start-scene`): acampamento pequeno atrás do título. As salas
  continuam em `scenes/interiors/`
- `quests/*.json` + `quests/index.json` — definições das 28 quests (lidas por
  `scripts/lib/rules.lua`; campos: `id, npc, biome, title, lines_*, objective, rewards`)
- `dialogues/*.json` — grafos de diálogo ambientais (lidos por `scripts/townsfolk.lua`)
- `data/items.json`, `data/landmarks.json`, `data/progression.json` — itens/loja,
  marcos da Nota e progressão (XP/talentos), editáveis como conteúdo
- `editor/plugins/*.lua` — plugins de autoria do Vael no ViberEditor; o editor
  em si não tem regras de jogo (`Viber/editor/PLUGINS.md`)
- `scripts/**.lua` — comportamento Luau (inimigos/bosses, colheita, POIs,
  HUD/UI) via a API `viber.*` (`Viber/docs/LUA_API.md`)
- `config.yaml` — **contrato de paths com a engine** (obrigatório; roots,
  dirs de áudio/texturas/scripts/UI, save) — `Viber/docs/ASSETS.md`
- `ui/*.css` + `world/hud.xml`/`world/menu.xml` — HUD e menu declarativos
- `qa-*.xml` / `ui-showcase.xml` — mundos QA, também estáticos (incluem
  módulos partilhados de `world/**`)

## Editar

Por operações — `viber scene` / `viber tool` / `viber terrain` (offline ou
`--live`) ou o ViberEditor. No editor, a autoria de conteúdo vive nos plugins
do projeto (`Vael/editor/plugins/`): quests, diálogos, itens/loja, marcos,
progressão e o grip das armas. Re-gerar a partir do exemplo original do motor TS
deixou de ser o fluxo: produz um
mundo procedural que teria de ser convertido (`viber bake`) e substituiria o
estático editado. GLBs novos entram por `assets/` (`gameassets resume`);
`viber analyze` confirma que os URLs resolvem.

## Fila aberta (na engine, não neste mundo)

- Instancing GPU para vegetação (hoje cap 800 instâncias/tag)

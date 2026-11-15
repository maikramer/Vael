# QA: cenas leves e cenários de teste

Testar uma quest no mundo inteiro custa minutos de carregamento, o prólogo,
andar até ao dador e jogar as etapas anteriores. As ferramentas de QA cortam
isso em três camadas, todas a partir do MESMO ficheiro de cenário:

| Camada | Custo | O que prova | Comando |
|--------|-------|-------------|---------|
| Simulação | ms, sem janela | regras: quests, cadeias, cofre, forja (receitas), XP, entregas | `python3 tools/qa.py sim all` · `cargo test --release --test qa_scenarios` |
| Cena QA ao vivo | 10–30 s | scripts Lua reais, diálogo [E], criaturas, bigorna, HUD | `python3 tools/qa.py run <cenário>` |
| Mundo `main` + cenário | minutos | posição real, streaming, colisões, o resto do mundo | cenário com `"scene": "main"` |
| Jackdaw "Play (QA)" | o Play | iterar a olho, hot reload do cenário e da cena | `python3 tools/qa.py use <cenário>` |

Código: `src/qa.rs` (modelo, resolução, simulação, plugin, `viber.vael.qa_*`),
`tools/qa.py` (CLI), `tests/qa_scenarios.rs` (gate). Skill dos agentes:
`.cursor/skills/vael-qa/SKILL.md`.

## Cenário

`qa/scenarios/<nome>.json` (subpastas valem: `city_forge/active`). O schema é
`Scenario` em `src/qa.rs`; campos desconhecidos são erro.

| Campo | Efeito |
|-------|--------|
| `extends` | cenários base, por ordem (`_base`: meio-dia, céu limpo, vida cheia, ferramentas de dev). Mapas juntam-se, listas somam, o filho ganha |
| `scene` / `entry` | cena de arranque (`main`, `qa/<x>`, `interiors/<x>`). Em `main` a entrada por omissão é `dev` (sem título nem prólogo) |
| `quest`, `state`, `stage`, `progress` | quest em foco: `not_taken`/`active`/`ready`/`done`, etapa 1-based e progresso nela |
| `prerequisites` | (default `true`) a cadeia anterior fica entregue: `requires` transitivos e quem aponta para a quest em `next` |
| `quests` | outras quests: `"id": "done"` ou `{ "state", "stage", "progress" }` |
| `kit`, `items` | mochila: kit inicial de `data/items.json` (default) e quantidades por cima (`0` apaga) |
| `gear` | `weapon`/`armor`/`charm` → id (entra na mochila; o slot é validado) |
| `skills` | talentos de `data/progression.json`, com os `requires` deles |
| `level`, `xp`, `skill_points` | nível pela curva real (os pontos ganhos incluídos) |
| `hp`, `max_hp`, `flasks`, `biomes` | vida, cargas do cantil, biomas revelados |
| `flags` | chaves cruas do `viber.game()` (ganham a tudo; `null` apaga) |
| `hour`, `weather` | hora (0–24) e clima de arranque (`Clear`, `Rain`, `Storm`…) |
| `player` | `pos: [x,y,z]` ou `near: "<entidade>"`/`"@giver"` (o `npc` da quest), `distance`, `god`, `dev` |
| `steps` | passos, por ordem |
| `expect` | verificação final: `quests`, `stage`, `progress`, `items`, `skills`, `level`, `flags` (números exactos ou `">=3"`, `"<2"`, `"!=0"`) |

### Passos

| Passo | Simulação | Jogo vivo |
|-------|-----------|-----------|
| `{"accept": "id"}` | `quest_accept` | `rules.quest_accept` |
| `{"kill": "wolf"}` / `{"kill": {"kind", "count", "elite"}}` | `report_kill` | `rules.report_kill` |
| `{"slay": {...}}` | = `kill` | dano real (`rules.damage_enemy`) em criaturas da cena; a morte credita pelo script da criatura |
| `{"collect": {"item", "count"}}` | cofre + `sync_collect` | `rules.report_collect` |
| `{"craft": "hunter_cloak"}` | `craft_make` (gasta a receita; falha sem materiais) + progresso | `lib/craft.lua` `make` |
| `{"visit": "marco"}` | `report_visit` | `rules.report_visit` |
| `{"progress": {"quest", "n"}}` | `quest_set_progress` (puzzles de decisão) | `rules.quest_set_progress` |
| `{"turn_in": "id"}` | `quest_turn_in` | `rules.quest_turn_in` |
| `{"interact": "npc"}` | o que o `npc/dialogue.lua` faz: entrega a `ready`, senão aceita a disponível | teleporta a 1,2 m e carrega [E] |
| `{"expect": {...}}` | verifica | `viber.vael.qa_check` |
| `lua`, `key`, `wait`, `teleport`, `goto`, `say`, `screenshot` | ignorados (`vivo`) | executados |

## Cenas QA

`scenes/qa/<nome>.bsn`: chão plano com colisor, luz, `SceneEntry "qa"` e
entidades **copiadas do mundo** (a subárvore `.bsn` inteira: script, props,
prefab `IsA`, `NpcBody`), com `SceneNodeId` novos para o Jackdaw. Sem terreno
voxel nem os 10k nós do mundo: carregam em ~10 s.

```bash
python3 tools/qa.py scene find blacksmith                 # nomes para --copy
python3 tools/qa.py scene new forja --copy npc_blacksmith --copy anvil@3,2 --enemy wolf:3
python3 tools/qa.py scene quest forest_wolves             # dador + inimigos + 2 cenários
```

`--enemy kind:n` põe `n` criaturas como entidades fixas (o formato dos
chefes: `script="enemies/<kind>.lua" hostile health ragdoll`, com o modelo
que o mundo usa) numa fila à frente da entrada; `--enemy bosses/witch:1`
para chefes. **Não** usar `DynamicSpawner` numa cena sem terreno: o spawner
espera pelo `TerrainRuntime` e o véu só sai no teto de 240 s.
`--interior` liga a câmara de sala. Abre-se e edita-se no Jackdaw como qualquer cena; regerar com
`--force` quando o original no mundo mudar (é uma cópia).

## Jogo vivo

```bash
python3 tools/qa.py run city_forge/dialogue          # compila, lança, passos, expect, fecha
python3 tools/qa.py run city_forge/dialogue --keep   # deixa aberto (porta 15740)
python3 tools/qa.py steps city_forge/dialogue --port 15740   # repõe e corre outra vez
python3 tools/qa.py check|state --port 15740
python3 tools/qa.py snapshot bug_123 --port 15740    # estado vivo → cenário
python3 tools/qa.py run sandbox --no-build           # binário já compilado (build partido por outros)
```

Artefactos em `$TMPDIR/vael-qa-<cenário>-*/`: `game.log`, `final.png` e os
`screenshot` dos passos. O `interact` confirma pelo estado do jogo que o [E]
fez alguma coisa e repete até 3 vezes (o primeiro [E] logo a seguir a um
teleporte pode cair na cura).

O jogo arranca com `vael --scenario <nome>` (ou `VAEL_SCENARIO`). O
`VaelQaPlugin` escreve o estado no `viber.game()` mal o Lua existe e de novo
quando o mundo fica pronto (o arranque da cena pode escrever por cima), e só
então teleporta o herói e acerta a hora. Os saves desta sessão vão para
`$TMPDIR/vael-qa-saves` (`VAEL_QA_SAVE_DIR`).

**Hot reload**: gravar o cenário (ou um `extends` dele) com o jogo a correr
reaplica-o em ≤0,5 s. Reaplicar **repõe** quests/itens/talentos/equipamento/
biomas: o que não está no plano é apagado. Pela bridge:
`viber.vael.qa_apply(nome?)`, `qa_check(json?)`, `qa_snapshot()`,
`qa_plan(nome)`, `qa_name()`.

O transporte dos passos vivos é o `vael-tools debug` (`--json`): o `qa.py`
não tem cliente JSON-RPC próprio — compõe cenários, e o Rust fala com o jogo.
Para agentes sem Python, os mesmos blocos existem como primitivas do CLI:
`debug state` (qa_snapshot), `debug expect` (qa_check), `debug until`,
`debug pause/step/play` e `debug playbook` (ver `docs/TOOLS.md`). O
`qa.py steps <cenário>` corre os passos num jogo já aberto **na cena do
cenário** — num jogo no mundo `main` os alvos de `interact` são outros e o
`expect` falha por desenho.

**Jackdaw**: `python3 tools/qa.py use city_forge/dialogue` e Play ▸ "Play
(QA)". O cenário escolhe a cena; editar a cena no editor chega ao jogo pelas
edições ao vivo do Jackdaw, e gravar o cenário reaplica o estado.

## Notas

- O [E] alcança metade do alcance autorado (`set_interaction(…, 3.5)` →
  1,75 m); `goto`/`interact` põem o herói a 1,2 m e `player.near` a 1,4 m.
- A entrega de uma quest `craft` não leva o item; a de `collect` leva.
- `hour` entra no arranque e no hot reload; `weather` só no arranque.
- Uma GPU de 6 GB não aguenta dois jogos e o editor: um "Out of Memory" no
  log é falta de VRAM, não o cenário.
- Cenário novo ou mudado → `cargo test --release --test qa_scenarios`.

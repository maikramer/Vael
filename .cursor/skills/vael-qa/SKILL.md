---
name: vael-qa
description: Usar para testar o Vael sem carregar o mundo inteiro — criar cenas de QA leves (scenes/qa), cenários de teste (qa/scenarios/*.json) com posição, quests e pré-condições, itens, equipamento, talentos/habilidades, nível, vida, hora e clima; simular quests sem janela; correr um cenário no jogo vivo pela bridge; hot reload no Play (QA) do Jackdaw; snapshot do estado de uma sessão. Triggers — testar quest, QA, cena de teste, cenário, reproduzir bug de quest, pré-condições, "testa isto rápido", tools/qa.py, VAEL_SCENARIO, --qa-sim.
---

# QA do Vael: cenas leves e cenários

Três peças, um formato:

| Peça | O quê |
|------|-------|
| `scenes/qa/*.bsn` | Cenas pequenas (chão plano, luz, entrada `qa`) com **entidades reais copiadas do mundo** (NPC, bigorna, prefabs) e inimigos em `DynamicSpawner`. Carregam em ~10 s contra minutos do `main`. Editáveis no Jackdaw |
| `qa/scenarios/**/*.json` | **Cenário**: cena + estado de partida (quest em foco com a cadeia anterior entregue, itens, gear, talentos, nível, vida, cantil, hora, clima, posição) + `steps` + `expect` |
| `src/qa.rs` + `tools/qa.py` | Resolve o cenário com as regras REAIS (`quest.rs`, `quest_events.rs`, `economy.rs`, `xp.rs`), simula, aplica no jogo, vigia o ficheiro (hot reload), verifica |

Documentação longa: `docs/QA.md`. O schema é a struct `Scenario` em `src/qa.rs`
(campos desconhecidos são ERRO: um typo não passa por cenário vazio).

## Escolher o caminho (do mais barato ao mais caro)

1. **Simulação** (ms, sem GPU): regras de quest/cofre/forja/XP. Primeiro passo sempre.
   `python3 tools/qa.py sim all` · `cargo test --release --test qa_scenarios`
2. **Cena QA ao vivo** (~10–30 s): scripts Lua reais, diálogo [E], criaturas,
   forja, HUD. `python3 tools/qa.py run <cenário>`
3. **Mundo `main` com cenário** (minutos): bugs que dependem do mundo
   (posição real, streaming, colisões). `"scene": "main"` + `player.pos`/`near`.
4. **Jackdaw "Play (QA)"**: iterar a olho com hot reload (`qa.py use <cenário>`).

## Receitas

**Testar uma quest de ponta a ponta (o caso mais comum):**

```bash
python3 tools/qa.py scene quest city_forge     # cena qa/city-forge + 2 cenários
python3 tools/qa.py sim city_forge/not_taken   # lógica (ms)
python3 tools/qa.py run city_forge/not_taken   # jogo vivo; PASS/FAIL + screenshot
```

`scene quest` copia o dador (`quest.npc`) do mundo, junta a bigorna se há
etapa `craft` e criaturas fixas por alvo de `kill` (abates com `slay` — dano
real, crédito pelo script de morte; herói `god`). Os cenários gerados trazem os passos que cumprem cada etapa —
incluindo os materiais da receita antes de um `craft`.

**Estado a meio de uma quest:** `qa.py new forge_s2 --quest city_forge --stage 2 --scene qa/city-forge`
(etapa 2, cadeia anterior entregue, itens que `ready` de `collect` pede na mochila).

**Cena à medida:** `qa.py scene find blacksmith` → `qa.py scene new forja --copy npc_blacksmith --copy anvil@3,2 --enemy wolf:3`.

**Iterar sem reiniciar** (jogo aberto com `run --keep` ou Play (QA)):
gravar o JSON reaplica sozinho (≤0,5 s); `qa.py steps <cenário> --port P`
repõe o estado e corre os passos; `qa.py check`, `qa.py state`, `qa.py lua '<luau>'`.

**Reproduzir um bug visto a jogar:** com o jogo aberto (`--bridge P`),
`qa.py snapshot bug_xyz --port P` grava quests/itens/talentos/posição como
cenário (`prerequisites:false`, `kit:false` — reprodução exacta).

## Cenário (resumo)

```jsonc
{
  "extends": ["_base"],                  // hora 12, céu limpo, vida cheia, dev
  "scene": "qa/city-forge",              // ou "main", "interiors/forge"
  "quest": "city_forge", "state": "active", "stage": 2, "progress": 0,
  "quests": { "story_notas": "done", "forest_witch": {"state": "active", "progress": 0} },
  "items": { "wolf_pelt": 3, "wood": 4 }, "kit": true,
  "gear": { "weapon": "steel_sword" }, "skills": ["strength2"], "level": 5,
  "hp": 60, "max_hp": 100, "flasks": 2, "biomes": ["dark_forest"],
  "flags": { "story_epilogo": false },
  "hour": 22, "weather": "Rain",
  "player": { "near": "@giver", "god": true },
  "steps": [ {"interact": "npc_blacksmith"}, {"craft": "hunter_cloak"}, {"expect": {"quests": {"city_forge": "ready"}}} ],
  "expect": { "quests": {"city_forge": "done"}, "items": {"gold": ">=150"} }
}
```

Passos lógicos (simulação **e** jogo): `accept`, `kill`, `slay`, `collect`,
`craft` (gasta a receita; falha sem materiais), `visit`, `progress`,
`turn_in`, `interact` (dador: entrega `ready` / aceita a disponível), `expect`.
Só no jogo: `lua`, `key`, `wait`, `teleport`, `goto`, `say`, `screenshot`
(a simulação marca-os `vivo`).

## Armadilhas (aprendidas a testar)

- **Alcance do [E] é metade do autorado** (3,5 m → 1,75 m). `goto`/`interact`
  põem o herói a 1,2 m; `player.near` a 1,4 m. Fora disso o [E] vai para a
  cura ("Vida já está cheia").
- **Reaplicar repõe** quests/itens/talentos/gear/biomas (o que não está no
  plano é apagado). Preferências (`lang`) e o resto ficam.
- `craft` numa quest de forja **não** é tirado na entrega; `collect` é.
- Saves do cenário vão para `$TMPDIR/vael-qa-saves` (`VAEL_QA_SAVE_DIR`); o
  slot do jogador nunca é tocado.
- `hour`/`weather` entram no arranque; o hot reload só muda a hora.
- `run` compila antes de lançar. Build partido por outro agente → `run
  --no-build` com o binário que há, ou esperar (não mexer nos ficheiros dele).
- Sem terreno não há `DynamicSpawner` (espera pelo `TerrainRuntime`: véu
  preso até 240 s). O `scene new --enemy` já põe criaturas fixas. GPU com 6 GB: um jogo de cada vez; ver
  `nvidia-smi` antes de culpar o cenário por um "Out of Memory".
- Nunca `pkill -f` com o nome do cenário/binário: casa com a própria shell.
  `qa.py run` fecha o grupo de processos do jogo; com `--keep`, `kill -TERM -<pid>`.
- Cenas QA são cópias: mudar o NPC no mundo não muda a cena. Regerar com
  `--force` quando o original mudar.
- Cenário novo → `cargo test --release --test qa_scenarios` (gate de todos).

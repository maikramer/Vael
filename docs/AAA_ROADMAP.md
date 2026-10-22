# AAA_ROADMAP.md — Vael sobre uma engine generalista

Este documento é o estudo de técnicas e o programa de melhorias acordado
para levar o Vael a um nível de apresentação/produto “AAA” **sem** deixar de
ter uma engine generalista. A engine só precisa de ser uma base sólida; as
regras de jogo continuam em Lua.

## Estado (2026-10-21)

Feito: Fase 0 (gates, baseline, correções baratas, docs) e a maior parte da
Fase 1 — áudio (buses voice/ambience, ducking, reverb, música de combate),
save slots 0–3 + metadados, locale em runtime + `@chave` na UI, navegação por
gamepad, escala de UI, decals, quit; no Vael: título sobre a cena de menu,
morte com retry/load/título, autosave, áudio de combate/SFX em falta, mapa
visual com marcadores, waypoint para o objetivo, PT/EN parcial. Baseline em
[`PERF_BASELINE.md`](PERF_BASELINE.md): 60 fps em regime estacionário na
vila/floresta/ermo; o teleporte/streaming ainda custa 100 ms+.

Falta desta fase: animation events/root motion, soft particles/trails,
direção do dano, localização completa PT/EN. `cargo clippy` ainda falha em
lints pré-existentes de módulos não tocados (ragdoll, streaming, vitals,
script); o lint novo do gamepad foi corrigido.

**Fase 2 em curso:** orçamento de tempo no passe voxel (hitch de streaming)
e batching estático v2 — instâncias de spawner/vegetação sem LOD, multi-mesh,
chave por `(tile, mesh, material)`, cull com histerese e limpeza quando a
fonte despawna. A medição A/B fica pendente de uma sessão limpa.

## Alvo

- PC single-player (Linux/Vulkan e Windows).
- 60 fps a 1080p numa RTX 4050 Laptop 6 GB.
- “AAA” aqui = produto completo + mundo reativo + combate legível +
  apresentação coesa + performance estável. Não é fotorealismo com ray
  tracing.

## O que a engine já tem (não reescrever)

Bevy 0.20, forward PBR, terreno voxel com LOD, Luau com hot-reload, Rapier,
kira, UI declarativa, cenas, bridge, profiler, editor, save v2, IBL/probes,
SSAO, bloom, TAA/DLSS, motion blur, DOF, color grading, volumétrico global,
SSR de água, partículas CPU, LOD de malha e streaming por anéis.

## Técnicas estudadas — decisão

| Técnica | Decisão |
|---|---|
| Batching estático multi-mesh, impostores, HLOD, streaming por células | Adotar (Fase 2; medir antes) |
| Animation events, root motion, layers/masks, ragdoll completo | Eventos/root motion na Fase 1; layers/ragdoll na Fase 2 |
| Mixer snapshots, ducking, reverb zones, bus de voz/ambiente | Adotado na Fase 1 |
| Decals, soft particles, trails de arma | Decals na Fase 1; soft particles/trails depois |
| Spot lights e volumes de névoa locais | Fase 2 |
| Gamepad UI, a11y semântica, escala de UI/render | Gamepad + escala de UI na Fase 1 |
| Save slots, autosave, metadados, locale em runtime | Fase 1 |
| Meshlets, virtual texturing, SSGI/deferred, LUT grading | Adiar (orçamento 6 GB) |
| Ray tracing, multiplayer/replicação, consolas, motion matching, VO/lip-sync completo | Rejeitar (fora do alvo e da doutrina) |

## Fases

### Fase 0 — Baseline e higiene

- `cargo test`/`clippy` no Viber; `viber analyze`, `scene lint`,
  `script_syntax`, `hud_contract` no Vael.
- Baseline de `viber debug prof --tab frames`, série `--record 30 --hz 2`,
  `viber debug trace 10`, `viber debug hash` em vila, floresta e combate.
- Correções baratas: include do `qa-map`, waypoint para objetivo, contagem
  de sementes, som do baú, música `battle`/`boss`, docs 27/28.

### Fase 1 — Produto + primitivas de apresentação

Engine: buses `voice`/`ambience`, ducking, reverb, `viber.music.combat`;
save slots 0–3 + metadados; `viber.locale.set/get` + UI `@chave`; navegação
por gamepad; escala de UI; `viber.decal`; `viber.quit`.

Vael: ecrã de título (Continuar/Novo Jogo/Definições/Créditos/Sair), morte
com Tentar de novo/Carregar/Título, save slots + autosave em fogueiras,
entrega de quest e chefe; áudio de combate e SFX em falta; decals de
impacto; mapa visual do heightmap com marcadores; waypoint para o objetivo
da quest; opções/acessibilidade; localização PT/EN.

### Fase 2 — Performance e mundo vivo

Batching estático v2 + impostores + HLOD; spot lights e névoa local;
animation layers/masks e ragdoll completo. No Vael: densidade no frontier,
interiores de outposts/grutas, segunda povoação/dungeon, spawns noturnos,
horários de NPC, clima com consequências, patrulhas/emboscadas.

Gate: 60 fps a 1080p; p95/hitches; `world_hash`/goldens; analyze/lint
verdes.

### Fase 3 — Progressão e combate

Itemização (armas/armaduras, raridade/afixos, sets), crafting na forja/
bigorna, materiais de colheita; mais armas/habilidades e talentos com
escolhas; 4 arquétipos de inimigo, minibosses, fases de chefe; status
(queimadura/gelo/choque) e fraquezas; encontros compostos.

### Fase 4 — Conteúdo, cinemática e shipping

Mais interiores/quests/diálogos; câmara de diálogo (planos/contraplanos/
close-ups), retratos, voice barks; passe de áudio/VFX/iluminação; revisão
PT/EN; auditoria de acessibilidade; migração de saves; golden screenshots;
perf CI; checklist de release.

## Doutrina

- **Engine generalista:** zero regras de RPG em Rust. Cada API nova é
  primitiva genérica, documentada em `../Viber/docs/LUA_API.md` (e
  `UI.md`), na tabela de tags de `../Viber/AGENTS.md`, com teste.
- **Jogo no Lua/XML:** estender `scripts/lib/rules.lua`, `fsm.lua`,
  `combat.lua`, `harvest.lua`; não reescrever.
- **Mundo estático e determinístico:** `world_hash`/goldens; edições por
  `viber scene`/`tool`/`terrain`.
- **Profiling CLI-first:** `viber debug prof`/`trace`, sem screenshots de
  painel.
- **Sem commits sem pedido** e sem reverter trabalho de outras sessões.

## Critérios de aceitação

- `cargo test`/`clippy` verdes; `viber analyze`, `scene lint`,
  `script_syntax`, `hud_contract` verdes.
- 60 fps no preset alvo, com p95/hitches medidos antes/depois.
- Fluxo completo: abrir → Novo Jogo/Continuar → jogar → pausar →
  guardar/carregar → morrer → repetir → créditos; mapa/waypoint; áudio
  reage; PT/EN troca sem reiniciar.

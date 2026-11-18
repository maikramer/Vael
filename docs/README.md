# Documentação do Vael — índice

Este índice diz **onde está a verdade** e **o que é histórico**. O contrato do
projeto (mundo, fluxo, gotchas) é o [`../AGENTS.md`](../AGENTS.md); a engine
tem o seu próprio contrato em [`../../bevy_extensions/AGENTS.md`](../../bevy_extensions/AGENTS.md).
Quando um doc e o código divergem, o código e o `AGENTS.md` ganham — e o doc
deve ser corrigido no mesmo commit.

## Mundo e campanha

- [GDD de O Silêncio de Vael](../GDD.md) — experiência, sistemas, progressão, acessibilidade e critérios de produção.
- [Enredo e missões principais](CAMPANHA_PRINCIPAL.md) — história completa, percurso M01–M14 e consequências das escolhas. Contém o final.
- [Bíblia narrativa](../STORY.md) — geografia, elenco, voz e acontecimentos do primeiro capítulo.

## Estado atual

| Documento | Responde a | Estado |
|---|---|---|
| [`MIGRATION.md`](MIGRATION.md) | o que já saiu da engine, o que falta e as dívidas da migração Rust/Bevy | **vivo** |
| [`PACKAGES.md`](PACKAGES.md) | ponte para o roadmap canónico no `bevy_extensions` e sequência da Fase 4 no Vael | **vivo** |
| [`DELETE-MONOLITH.md`](DELETE-MONOLITH.md) | o estudo de eliminação do `bevy_ext_viber_engine`: inventário medido, fases D/F/G/H até `cargo tree -i viber` vazio | **vivo** |
| [`EXIT-MAP.md`](EXIT-MAP.md) | o mapa peça→destino→onda da saída do monólito (precursor do DELETE-MONOLITH; estados das waves A/B/C/E) | vivo |
| [`LIGHTING.md`](LIGHTING.md) | céu, exposição, névoa e sombras com o `bevy_elements` | **vivo** |
| [`PHYSICS-AVIAN.md`](PHYSICS-AVIAN.md) | física Avian 0.8 + tnua, conversão do DSL, autostep do herói e gates | **vivo** |
| [`WATER.md`](WATER.md) | camadas da água, espelho CPU do swell, flutuação (`FloatingBody`) e barcos a remos | **vivo** |
| [`PAVING.md`](PAVING.md) | pavimento em grelha das estradas/praças: ocupação exclusiva, autotile de cantos, camadas de lift e migração | **vivo** |
| [`TOOLS.md`](TOOLS.md) | CLI `vael-tools`, bridge de debug e extensões Jackdaw | **vivo** |
| [`JACKDAW.md`](JACKDAW.md) | o que se autora no Jackdaw (matriz por sistema), componentes tipados, painel de cinemáticas e gatilhos | **vivo** |
| [`SAVE.md`](SAVE.md) | save nativo (`moonshine-save`), slots, merge do estado e o que persiste | **vivo** |
| [`DEBUG.md`](DEBUG.md) | arranque directo (`--scene main --entry dev`), F1/F2 e efeitos no save | **vivo** |
| [`QA.md`](QA.md) | cenas QA leves, cenários de teste (quests, itens, talentos, posição), simulação e `tools/qa.py` | **vivo** |
| [`PLAYTEST.md`](PLAYTEST.md) | bateria `bevy_playtest` in-process (cenários, seeds, soak, invariantes) e oráculo visual+estado `liquid/d1-ultra` | **vivo** |
| [`FLORA.md`](FLORA.md) | árvores procedurais, estações, pools, colheita e gates | **vivo** |
| [`GROUND.md`](GROUND.md) | manto de neve, rastos e sombra de nuvem no chão do `.ahgt` | **vivo** |
| [`PRIMITIVES.md`](PRIMITIVES.md) | tags de primitivas (lathe/sweep/extruded/…), presets de material, colisores e preview no Jackdaw | **vivo** |
| [`AUDIO.md`](AUDIO.md) | direção sonora, layers por bioma, manifests e regen | **vivo** |
| [`REMOTE_GPU.md`](REMOTE_GPU.md) | `vramd` na rede, bundles e GPU remota | **vivo** |
| [`DECISION_QUESTS.md`](DECISION_QUESTS.md) | quests de decisão com o d1 e os seus dados | **vivo** |
| [`JUDGE_NLU.md`](JUDGE_NLU.md) | juiz/NLU dos veredictos | **vivo** |
| [`PUZZLES_NLU.md`](PUZZLES_NLU.md) | enigmas do santuário e resolução por NLU | **vivo** |

## Histórico / estudo (não é o gate atual)

| Documento | Porque fica |
|---|---|
| [`PERF_BASELINE.md`](PERF_BASELINE.md) | medição de 2026-10-21, anterior a `bevy_elements`, UI `bevy_pf` e player/câmara no Vael; serve de ordem de grandeza e método |
| [`AAA_ROADMAP.md`](AAA_ROADMAP.md) | plano de apresentação/performance de 2026-10-21; o pressuposto “regras em Lua” foi substituído pela migração |
| [`TERRAIN-VOXEL-STUDY.md`](TERRAIN-VOXEL-STUDY.md) | estudo de 2026-11-11 que decidiu **não** migrar o terreno para `voxel-framework` |
| [`CODE-SLIM-STUDY.md`](CODE-SLIM-STUDY.md) | estudo de 2026-11-13 sobre enxugar o crate (crates existentes, repetição, compilação); a Fase 1 foi aplicada no mesmo dia |
| [`ARCH-REVIEW-STUDY.md`](ARCH-REVIEW-STUDY.md) | estudo de 2026-11-13 sobre a arquitectura dos cinco checkouts: overengineering, camadas de abstração temporárias e oportunidades crates.io; o plano faseado (F0–F4) fica aqui |
| [`ARCH-REVIEW-2026-11-15.md`](ARCH-REVIEW-2026-11-15.md) | segundo levantamento (2026-11-15): tools repetidas (bridge ×4, validadores ×2, esqueleto Python do AiGameKit), seams mortos nos crates extraídos e CelerAI; onda 0 aplicada no dia |
| [`DEV-STANDARD-STUDY.md`](DEV-STANDARD-STUDY.md) | estudo de 2026-11-14 com o standard de engenharia de Vael+bevy_extensions (tooling, fronteiras, contratos, QA, release, governança), gap analysis e plano S0–S4 |
| [`JACKDAW-ALIGNMENT-STUDY.md`](JACKDAW-ALIGNMENT-STUDY.md) | estudo de 2026-11-15 sobre o que ainda autora fora do editor e o SDK do Jackdaw por aproveitar; backlog P1–P4 (migrações `.bsn`, painéis de conteúdo, alinhamento SDK, prefabs nativos) |
| `perf-baseline-*.jsonl` | séries brutas do profiler que suportam o `PERF_BASELINE.md` |

## Convenções de manutenção

- **Um facto, um dono.** O `AGENTS.md` manda no contrato; o `MIGRATION.md` no
  estado da migração; o `PACKAGES.md` nas fases/pacotes. Não copiar a mesma
  tabela para três sítios.
- **Sem caminhos mortos.** O mundo é `.bsn` desde 2026-11-12; não citar
  `world/**/*.xml`, `world.xml` ou `scenes/*.xml` como fonte. (`origin_file`
  dentro de um `.bsn` é proveniência histórica, não um ficheiro existente.)
- **Datas e contagens.** Contagens de testes, assets e quests mudam por onda:
  preferir “suite verde / ≥N desde &lt;data&gt;” a um número que envelhece mal.
  Uma medição de performance leva sempre data e commit.
- **Ficheiros históricos** começam com um aviso `> **Documento histórico…**`
  e dizem o que os substituiu.
- **Validar antes de fechar:** `vael-tools analyze world.bsn`, `vael-tools lint`
  e `cargo test --lib` no Vael. Docs que citam flags devem
  confirmar o default no código (`src/*.rs`), não no doc anterior.

# Documentação do Vael — índice

Este índice diz **onde está a verdade** e **o que é histórico**. O contrato do
projeto (mundo, fluxo, gotchas) é o [`../AGENTS.md`](../AGENTS.md); a engine
tem o seu próprio contrato em [`../../Viber/AGENTS.md`](../../Viber/AGENTS.md).
Quando um doc e o código divergem, o código e o `AGENTS.md` ganham — e o doc
deve ser corrigido no mesmo commit.

## Estado atual

| Documento | Responde a | Estado |
|---|---|---|
| [`MIGRATION.md`](MIGRATION.md) | o que já saiu da engine, o que falta e as dívidas da migração Rust/Bevy | **vivo** |
| [`PACKAGES.md`](PACKAGES.md) | ponte para o roadmap canónico no `bevy_extensions` e sequência da Fase 4 no Vael | **vivo** |
| [`LIGHTING.md`](LIGHTING.md) | céu, exposição, névoa e sombras com o `bevy_elements` | **vivo** |
| [`VIBER_TOOLS.md`](VIBER_TOOLS.md) | CLI `viber`, runtime de debug e extensões Jackdaw | **vivo** |
| [`DEBUG.md`](DEBUG.md) | arranque directo (`--scene main --entry dev`), F1/F2 e efeitos no save | **vivo** |
| [`FLORA.md`](FLORA.md) | árvores procedurais, estações, pools, colheita e gates | **vivo** |
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
- **Validar antes de fechar:** `viber analyze world.bsn`, `viber scene lint
  --world world.bsn` e `cargo test --lib` no Vael. Docs que citam flags devem
  confirmar o default no código (`src/*.rs`), não no doc anterior.

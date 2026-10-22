# PERF_BASELINE.md — Vael

Primeira linha de base de performance depois das primitivas da Fase 1
(áudio, save slots, locale, gamepad, escala de UI, decals). Medição
**CLI-first** (`viber debug prof` / `hash`), sem screenshots do painel.

## Ambiente

- CPU: 12th Gen Intel Core i7-12650H (10 núcleos).
- GPU: NVIDIA RTX 4050 Laptop, 6 GiB.
- RAM: 61 GiB.
- Driver: NVIDIA 615.71.09, Vulkan.
- Build: release (`cargo build --release`), preset `equilibrado`.
- `config.yaml`: post-FX `volumetrics/dof/motion-blur/contact-shadows/
  chromatic-aberration/vignette/split-tone` desligados; SSAO medium;
  reflexos `auto`.

## Método

```bash
viber debug prof --tab frames --samples 12
viber debug prof --record 12 --hz 2 --top 8 -o docs/perf-baseline-<local>.jsonl
viber debug hash
```

O `--record` é a série (média/pior/fps/hitches); o `--tab frames` dá o topo
por sistema. A primeira série depois de um teleporte inclui o custo de
streaming; a segunda, 15 s depois, é o regime estacionário.

## Resultados

| Local | Frame médio | FPS médio | Pior frame | Hitches novos |
|---|---:|---:|---:|---:|
| Vila (spawn) | 16.7 ms | 59.7 | 22.8 ms | 0 |
| Floresta (após teleporte) | 47.9 ms | 20.0 | 117.9 ms | 116 |
| Floresta (15 s depois) | 16.7 ms | 61.2 | 22.2 ms | 0 |
| Ermo (após teleporte) | 50.0 ms | 19.8 | 106.2 ms | 107 |
| Ermo (15 s depois) | 16.7 ms | 62.2 | 23.1 ms | 0 |

Topo do frame no regime estacionário (média · p95):

- `sched.last` ~8.3 ms · 9.5 ms (pacing/espera do frame).
- `render.render` ~3.8 ms · 4.7 ms.
- `sched.post_update` ~3.5 ms · 4.4 ms.
- `sched.update` ~3.4 ms · 5.4 ms.
- `bevy.transform_propagate` ~2.3 ms · 3.0 ms.
- `viber::luau::runtime::luau_update` ~1.8 ms · 2.5 ms.
- `physics.step` ~1.5 ms · 2.2 ms.

No teleporte, `viber::terrain::plugin::update_voxel_columns` chega a
4.7 ms de média e **41 ms de p95**, e o `sched.last` a ~40 ms — é o custo de
construir as colunas voxel do sítio novo, não o regime.

RSS do processo ~11 GiB (pico 14.4 GiB); VRAM da engine ~3.4 GiB. O RSS
inclui assets e o pool partilhado, não é VRAM.

Mundo vivo (hash): `09b52003b5dba981` antes do QA; `f8e0096fa26de4e6` depois
(a hash inclui poses de entidades vivas, por isso não é um gate de
determinismo — para isso, dois boots limpos do mesmo binário).

## Leitura

1. **Regime estacionário a 60 fps** nos três pontos; o frame é CPU
   (`sched.*` + `post_update`), não a lente.
2. **O teleporte/streaming é o primeiro problema de perceção**: 100–120 ms
   de hitch e ~20 fps durante ~10–15 s. É o alvo natural da Fase 2
   (orçamento de `update_voxel_columns`, HLOD/batching, preaquecimento).
3. **`sched.last` ~8 ms** é o chão do frame; o resto do orçamento está
   distribuído. Antes de cortar post-FX outra vez, medir o trace.

## Próximo passo

- Repetir num boot limpo (sem teleporte) para a hash de determinismo.
- `viber debug trace 10 -o trace.json` na floresta durante o streaming e no
  combate, para atribuir o `update_voxel_columns` e os picos de física.
- Medir com o batching/impostores da Fase 2 e comparar com esta tabela.

## Intervenção em curso (Fase 2)

O passe voxel ganhou um **orçamento de tempo por frame**
(`VOXEL_BUILD_BUDGET_MS = 3.5`, `18 ms` durante `StreamingBoost`): para de
começar caixas/colunas novas quando o frame já gastou o orçamento e deixa o
resto para o seguinte (`state.pending`). O orçamento de caixas (4/frame)
continua como teto. Medir de novo num ambiente só com uma engine e comparar
a primeira série após teleporte com a tabela acima; o regime estacionário não
deve mudar.

O **batching estático v2** passou a incluir instâncias de spawner/vegetação
sem ladder de LOD, com qualquer número de meshes (tronco + folhas), agrupadas
por `(tile de 16 m, mesh, material)`. Os tiles fundidos têm cull com histerese
(96 m), devolvem as fontes ao `Inherited` e são removidos quando a fonte
despawna (troca de cena/streaming) ou quando o tile sai do raio. É o caminho
para cortar os draw calls das ~7 265 instâncias `.vinst`; a medição A/B fica
pendente de uma sessão limpa (as sessões concorrentes ocupam a VRAM).

### QA do batch (mundo pequeno)

`scenes/qa-batch.xml` + `scenes/qa-batch.vinst` (40 plantas no mesmo tile):

- 40 plantas → **4 tiles fundidos** (`batch:foliage`), 40 meshes de origem
  escondidos.
- Afastar a câmara (>96 m): batch **4 → 0**; voltar: **0 → 4**. O cull
  devolve as fontes e o rebuild volta a fundir.

### A/B do mundo principal (bloqueado)

Com a outra engine a ocupar ~3,2 GB de VRAM, o mundo principal a 400 m morre
no teleporte para a vegetação **tanto com o batch ligado como com
`VIBER_FOLIAGE_BATCH=0`** — os mesmos erros de validação de buffers
(skin/UI) e device perdido. Portanto o crash é contenção de VRAM, não uma
regressão do batch; o A/B de frame time/draw calls fica para uma sessão
exclusiva.




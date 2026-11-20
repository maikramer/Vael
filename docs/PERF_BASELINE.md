# PERF_BASELINE.md — Vael

> **Medição histórica (2026-10-21).** Antes do ambiente `bevy_elements`, da UI
> `bevy_pf` e da saída do player/câmara para o Vael. Os números continuam a
> servir de ordem de grandeza e de método, mas **não são o gate atual**:
> medir de novo com `viber debug prof` no HEAD corrente antes de comparar.

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

> Histórico: `scenes/qa-batch.bsn` foi removido a 2026-11-12; os números
> abaixo ficam como referência da medição original.

Cenário: 40 plantas no mesmo tile.

- 40 plantas → **4 tiles fundidos** (`batch:foliage`), 40 meshes de origem
  escondidos.
- Afastar a câmara (>96 m): batch **4 → 0**; voltar: **0 → 4**. O cull
  devolve as fontes e o rebuild volta a fundir.

### A/B do mundo principal (bloqueado)

Com a outra engine a ocupar ~3,2 GB de VRAM, o mundo principal a 400 m morre
no teleporte para a vegetação **tanto com o batch ligado como com
`VAEL_FOLIAGE_BATCH=0`** — os mesmos erros de validação de buffers
(skin/UI) e device perdido. Portanto o crash é contenção de VRAM, não uma
regressão do batch; o A/B de frame time/draw calls fica para uma sessão
exclusiva.

## Refino Bevy 0.19/0.20 (2026-11-12, em curso)

O mundo concorrente tem ~75k entidades e 455 scripts (a baseline acima tinha
~58k); com flora ligada houve OOM de render, e com `VAEL_FLORA=0` o regime
ainda não é comparável ao de 2026-10-21. As mudanças desta ronda são
mensuráveis com os gates:

- `VAEL_LOD_SCAN_HZ=0` — desliga o throttle de 10 Hz do scan da escada
  (controlo); default 10 Hz.
- `VAEL_GPU_CULLING=1` — piloto `NoCpuCulling` em malhas estáticas
  (Vulkan); default off.
- `VAEL_ECS_MIRROR=1` — devolve o espelho por frame de `viber.ecs` no Vael
  (default off).
- `VAEL_NO_CONTACT_SHADOWS=1` / `VAEL_WATER_SSR=0` — desligam os dois
  A/B visuais que o `config.yaml` passou a ligar.

O HUD Rust continua a publicar (`100/100`, `30 de ouro`) com o estado só a
10 Hz e sem republish por frame; a colheita/quests/marcos continuam a
funcionar com os gates em Rust. A medição A/B de frame/hitches fica para uma
sessão sem flora/WIP concorrente.




## Arranque medido de ponta a ponta (2026-11-16)

Primeira medição que **inclui** o trabalho síncrono antes do primeiro
`PostUpdate`: `bevy_ext_core::boot` marca cada fase e o véu imprime a tabela
(`boot: …`, ver `docs/DEBUG.md`). Duas execuções limpas, a mesma pose
(`VAEL_SCENARIO=_base`, `--scene main --entry dev`, `1280x720`,
`VAEL_QUALITY=high`, `VAEL_FPS_CAP=60`):

| fase | ms | o que é |
|---|---:|---|
| `world.parse` | 1577–1698 | `.bsn` + resolução de prefabs (main thread) |
| `main_scene.parse` | +0–1 | segunda leitura do mundo para os shaders (só ≠ 0 com `--scene` ≠ main) |
| `app.build` | +333–375 | plugins, config, materiais |
| `flora.pools` | +115–122 | 7 pools / 42 variantes |
| `spawn.startup` | +231–257 | entidades do mundo |
| `terrain.bootstrap` | +3700–4039 | heightmap 365, features 364, cliffs 476, paint 51, **splat 801**, rocks+mods 207, **voxel_columns 1466**, water 26, roads 59 |
| `first_frame` | +4652–4916 | resto do `Startup` + frame 1 (uploads de GPU, pipelines) |
| `veil` | 20825–26988 | o que o véu espera |
| **total** | **32153–37777** | processo → jogável |

O véu, segundo a segundo (`world_ready=debug`), não espera pelo que se pensa:

1. **t≈6–7 s** — os 629 GLB do anel de 150 m entram **num frame** (`6/629 →
   629/629`); o decode custa 1,4 s de CPU (418 ficheiros, 1.03 GB → 1.19 GB).
2. **t≈7–17 s** — `gpu_fila=true` e `pipelines≈102`: upload orçado a 256 MB/
   frame e compilação; o **frame 2 sozinho leva ~6,5 s**.
3. **t≈17–22 s** — tudo pronto excepto `bakes=64`: as 64 câmaras de impostores
   (42 variantes × 16 câmaras). ~5 s.
4. **t≈23–28 s** — `terreno_ocupado=true` **depois** dos bakes: as câmaras de
   bake morrem, "a primeira `Camera3d` da query" muda e o terreno re-escolhe o
   LOD do mundo inteiro (~5 s a 4 caixas/frame).

Regime em jogo (após o véu, mesma pose): `frame_ms` avg **23,5–24,7 ms**
(p95 ~32, pior 40), 58–62k entidades, 739 colunas, 4 118 colliders, RSS
**10,5 GB**, 217 scripts activos de 473. Os sistemas com nome das crates não
aparecem na tab rica (os shims `timed` delas são pass-through): `sched.post_update`
8,8 ms + `sched.update` 8,4 + `render.render` 7,7 + `render.prepare` 4,0 +
`bevy.check_visibility` 3,7 é onde o tempo está.

**Atenção à medição**: qualquer `cargo`/teste concorrente invalida os números
(com 16 `rustc` a load 26, `world.parse` foi de 1665 → 3948 ms). O
`world.parse` é o sentinela de ruído: acima de ~2000 ms, repetir.

## Correção: o véu levantava antes do mundo estar visível (2026-11-16)

Os números da secção anterior estavam **a favor do lado errado**: o véu saía
com terreno e HUD mas sem props, e o mundo enchia-se nos 2-3 s seguintes. As
capturas tiradas no *instante* do véu (`screenshot` imediatamente após
`wait-ready`) mostram a diferença; tirar a captura 2 s depois esconde-a porque
a vista já se encheu.

Duas condições do `WorldReadiness` estavam a mentir:

- **upload**: `bytes_written >= teto` raramente é verdade — o limiter do Bevy
  clampa o último asset que não cabe, por isso um frame com fila cheia escreve
  muito abaixo do teto (380 KB, 29 MB medidos). Passou a `bytes_written > 0`
  (12 frames sem escrever = fila drenada);
- **instanciação**: âncora com `WorldAssetRoot` e sem `WorldInstance` não
  contava para nada.

Com o gate honesto, o arranque (`_base`, `--scene main --entry dev`, `high`,
1280×720, máquina limpa) fica **~29-35 s** (variância de 5-6 s entre corridas;
`world.parse` 1,5-1,8 s é o sentinela: acima de ~2 s há carga concorrente).
Era o custo que já existia, só não era contado. Upload budget maior é pior
(512 MB → 29,7 s; 1024 MB → 31,8 s; 256 MB → 28,5 s na melhor corrida).

O que **passou a ser visível** no traço e continua por atacar, por ordem:

1. `first_frame` 4,2-5,3 s (623 cenas glTF instanciadas + uploads num punhado
   de frames);
2. `terrain.bootstrap` 3,4-3,8 s (`voxel_columns` ~1,3 s, splat ~0,7 s);
3. `world.parse` 1,5-1,8 s (resolução de prefabs);
4. a fila de upload (~2 GB com teto de 256 MB/frame) — é ela que fixa o fim do
   véu desde a correção.

## Vila e encosta em 18 de novembro de 2026

Build release do Vael, i7-12650H, RTX 4050 Laptop de 6 GiB, Vulkan, janela
1280×720, preset `equilibrado` e VSync padrão. O painel estava a 59,95 Hz.
As duas versões carregaram o mesmo mundo, uma instância do Vael de cada
vez, com saves de QA separados, céu limpo e relógio parado às 8 h.

| Cena | FPS antes | FPS depois | p95 antes | p95 depois |
|---|---:|---:|---:|---:|
| Portão da vila | 30,0 | 56,8 | 37,5 ms | 24,1 ms |
| Encosta com vegetação | 30,0 | 55,2 | 38,5 ms | 27,4 ms |
| Caminhada na encosta | 36,3 | 54,9 | 38,1 ms | 25,3 ms |

Cada célula agrega três janelas disjuntas de 240 frames de
`vael-tools debug prof --json`. O FPS é 1000 dividido pela média dos
tempos de frame. O p95 é o percentil das 720 observações da cena.
O arranque fica fora destas amostras.

Para repetir, abrir `world.bsn --scene main --entry dev --bridge 15887
--resolution 1280x720` com `VAEL_START_HOUR=8` e `VAEL_START_WEATHER=Clear`.
Depois de `wait-ready`, pausar o relógio com `viber.vael.env_pause(true)`.
Nos enquadramentos fixos, usar `viber.camera_free(true)`, colocar o herói
no XZ indicado e esperar 12 s para o streaming. Na tabela abaixo, `h`
é a altura do terreno no XZ do herói. Recolher as três amostras com 10 s
entre leituras.

| Cena | Herói XZ | Câmara XYZ | Alvo XYZ |
|---|---|---|---|
| Portão | (0, 44) | (4, h+4, 50) | (0, h+1,6, 35) |
| Encosta | (50, 75) | (50, h+4, 75) | (40, h+2, 60) |

Para a caminhada, abrir uma sessão com a câmara de jogo, começar em
(50, 75), esperar 4 s e manter W premido. Recolher uma janela ao fim de
10 s e repetir três vezes a partir do mesmo ponto. O deslocamento
registado nas duas versões foi de aproximadamente 40 m por passagem.

Com a versão otimizada e `VAEL_PRESENT_MODE=immediate`, as mesmas vistas
fixas registaram 57,7 FPS na vila e 55,5 FPS na encosta. A apresentação
imediata foi usada só nesta medição adicional.

## Oito cenários e estabilidade em 18 de novembro de 2026

Comparação incremental, depois da otimização da vila e da encosta. As duas
compilações release partiram da mesma cópia do código e usaram o mesmo mundo.
RTX 4050 Laptop de 6 GiB, i7-12650H, Vulkan, 1280 × 720, preset
`equilibrado`, VSync no painel de 59,95 Hz. Um jogo de cada vez na GPU.

O roteiro mede 10 s desde o teleporte, espera 5 s, mede mais 12 s com a
câmara parada e 12 s segurando W com a câmara do jogador. As leituras
sobrepostas do profiler foram unidas para contar cada frame uma vez.
O relógio do jogo, as criaturas e o clima continuaram ativos. A sessão de nevasca começou às 15 h com 0,30 m de neve no chão. A
sessão de chuva começou às 21 h e a de céu limpo às 9 h. Cada sessão
percorreu dois locais.
A distância percorrida e o clima efetivo ficam no JSON de cada local.

| Local | Entrada FPS | Parado FPS | Caminhada FPS | Caminhada p95 (ms) | Caminhada p99 (ms) |
|---|---:|---:|---:|---:|---:|
| Picos Gelados · nevasca | 34,6 → 44,6 | 51,4 → 53,5 | 40,9 → 46,3 | 40,8 → 28,6 | 55,9 → 36,7 |
| Portão da vila · nevasca | 40,1 → 45,9 | 49,7 → 50,1 | 42,0 → 47,2 | 38,8 → 27,5 | 47,5 → 31,2 |
| Pântano · chuva noturna | 33,7 → 44,7 | 38,9 → 47,2 | 30,1 → 39,2 | 53,8 → 33,5 | 66,8 → 40,9 |
| Floresta · chuva noturna | 27,4 → 36,3 | 28,7 → 38,5 | 37,1 → 50,6 | 35,0 → 26,9 | 41,6 → 31,4 |
| Margem do deserto · manhã limpa | 42,5 → 46,2 | 50,7 → 51,1 | 57,2 → 57,8 | 22,5 → 21,6 | 26,7 → 25,8 |
| Ermo distante · manhã limpa | 53,8 → 57,0 | 59,7 → 59,8 | 58,3 → 59,2 | 21,0 → 19,1 | 23,6 → 22,8 |

Cada par mostra antes → depois. O p99 é o tempo abaixo do qual ficaram
99% dos frames. O agregado usa todos os frames das seis rotas em cada
fase. As amostras de entrada incluem o carregamento local após o teleporte.

| Agregado das seis rotas | FPS | p95 (ms) | p99 (ms) | Frames acima de 50 ms |
|---|---:|---:|---:|---:|
| Entrada | 38,7 → 45,8 | 42,1 → 30,9 | 73,3 → 46,9 | 65 → 27 |
| Câmara parada | 46,5 → 50,0 | 34,8 → 27,2 | 38,1 → 30,2 | 2 → 0 |
| Caminhada | 44,2 → 50,0 | 35,0 → 28,0 | 51,9 → 33,9 | 38 → 9 |

A arena e a forja usam os tours de QA do jogo. A arena envia a sequência de
ataques, esquiva e habilidades. Na forja, o tour interage e abre o mapa.
Os resultados incluem o tempo de resposta aos comandos.

| Tour | FPS | p95 (ms) | p99 (ms) |
|---|---:|---:|---:|
| Arena | 59,7 → 59,7 | 18,1 → 17,9 | 19,6 → 19,5 |
| Forja | 59,7 → 59,7 | 18,2 → 18,1 | 22,7 → 20,6 |

Para reproduzir a matriz completa, compilar com `cargo build --release --bin vael` e executar `python tools/perf_routes.py /tmp/vael-rotas` a partir
da raiz do Vael. O roteiro usa saves temporários e grava PNGs, distâncias,
clima, tempos de frame e dados do profiler. As opções `--weather Rain` e
`--tour arena_combat` permitem repetir uma condição. Com
`VAEL_PROF_GPU=1`, cada fase inclui também os tempos das passagens da GPU.

As alterações reutilizam as alturas e as inclinações do terreno ao atualizar
a neve, preparam a cobertura de neve dos objetos em segundo plano e leem as
amostras dos blocos de terreno diretamente pelos índices da grade. As faces
de transição continuam a consultar o terreno a meia célula. O teste de
equivalência compara posições, normais, UVs, cores e índices com o gerador
anterior, incluindo escavações e transições de resolução.

Na floresta chuvosa, a lua permanecia acima do horizonte e as nuvens
reduziam seu feixe a uma fração diminuta de lux. Antes do ajuste lunar,
o perfil de GPU registrou 8,9 ms na passagem de sombras por vista com a
câmera parada. O jogo agora pula o mapa de sombras da lua abaixo de
0,0000001 lux e o retoma quando o luar volta. A lanterna, as fogueiras e
as sombras das noites claras continuam ativas. Na captura equivalente, a
passagem de sombras por vista saiu do perfil. As vistas auxiliares ainda
custaram 11,3 ms. Essas duas medições com timestamps de GPU ficaram fora
das tabelas de FPS.

Nas capturas noturnas de antes e depois, a diferença absoluta média foi
de 0,28 nível de cor por canal na floresta e 0,31 no pântano, em uma
escala de 0 a 255. Criaturas, clima e partículas estavam em movimento.
Na margem do deserto apareceu uma variação de brilho entre partidas. Em
uma repetição com o relógio pausado, a variação se inverteu entre as
versões. Esses registros permitem conferir as cenas, com a equivalência
da geometria coberta separadamente nos testes.

Cada célula da tabela vem de uma passagem por versão. Interprete diferenças
pequenas perto do limite do VSync à luz da variação entre partidas.

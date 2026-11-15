# Pavimento em grelha (`paving`)

O chão das estradas, praças e largos. Substitui o modelo antigo — **um
`<GroundDecal>` em forma de elipse por superfície** — por uma treliça única,
com uma célula por dono, cantos detectados e uma só malha por material.

Vive na engine: `bevy_ext_terrain_runtime::paving`
(`../bevy_extensions/crates/bevy_ext_terrain_runtime/src/paving.rs`). Este doc
é o contrato de autoria e o mapa do que a engine faz com ele.

## Porque existe

O modelo antigo tinha três problemas que não se remendam:

1. **Sobreposição.** Duas superfícies apanhadas da mesma grelha de altura ao
   mesmo `lift` são coplanares e não partilham triangulação: o depth test
   escolhe um vencedor diferente por triângulo e a zona pisca. A resposta era
   `stack_decal_lifts` — subir cada decal alguns milímetros acima do maior que
   ele cobre. É um desempate, não uma resolução: as duas superfícies continuam
   a ocupar o mesmo chão.
2. **Cantos.** Uma esquina era uma elipse esticada, ou duas elipses a
   atravessar-se. Não havia noção de "aqui a rua vira".
3. **Custo.** Cada nó era uma entidade com o seu material `AlphaMode::Blend`.
   A praça de Discordia sozinha eram 15 decals × (borda + miolo) mais as
   ribbons da rede por cima.

## O contrato

### Uma treliça por mundo

Todos os nós pavimentados rasterizam para a **mesma** grelha, ancorada em
`(0, 0)`, com o módulo mais fino entre os nós como passo (`paving-cell`).
É isso que faz as arestas alinharem: a beira da praça e a rua que sai dela
caem nas mesmas linhas.

> Duas grelhas com módulos não múltiplos (1,5 m e 2 m) reclamariam o mesmo chão
> com fronteiras diferentes, e nenhuma regra de prioridade as torna contínuas.
> Por isso o módulo é o **mínimo global**, não por nó.

### Ocupação exclusiva

Uma célula tem **um** dono. Dois nós que queiram a mesma célula resolvem por
`paving-priority` (empate: ganha o que aparece depois no ficheiro, a ordem de
leitura). Não há sobreposição a resolver depois — não pode acontecer.

### Snap para fora

Um retângulo/disco reclama as células cujo **quadrado intersecta** a forma, o
que equivale a arredondar as arestas para fora às linhas da grelha. Duas formas
que se tocam ficam sem fenda; o preço é a forma crescer **≤ 1 módulo por lado**
(34 m → 36 m numa grelha de 2 m). Autorar em múltiplos do módulo dá exacto.

Um corredor (`<Road paving>`) usa a mesma regra a partir das estações já
carvadas (Chaikin + resample a 1 m + flare), por distância **ao segmento** e
não à estação — senão ficavam pinholes nas curvas. O teste é estrito (`<
half_width`): uma célula apenas *tangente* tem sobreposição nula e contá-la
duplicava a largura sempre que a linha central calhava numa linha da grelha.

### Autotile de cantos

Cada célula lê os 8 vizinhos e molda o contorno:

| Estado do canto | Tratamento |
|---|---|
| **convexo** (as duas arestas são parede) | **chanfro** em arco de raio `paving-corner`, limitado a meio módulo. A meio módulo os arcos soldam nos pontos médios das arestas e uma célula isolada fica um disco; ao longo de um degrau de escada o contorno vira facetas, nunca um degrau de 90°. |
| **côncavo** (as duas arestas abertas, diagonal vazia) | **filete**: um remendo entre o canto e um arco tangente às duas paredes *do vizinho*, a bojar para o quadrante vazio. O interior da curva arredonda em vez de fazer bico. O remendo vive todo em espaço não pavimentado — não pode sobrepor-se a outra célula. |
| **recto / interior** | vértice simples. |
| **aresta partilhada com outro material** | vértice simples e **sem** feather: a fronteira entre dois materiais fica na linha da grelha (é um lancil, não uma nuvem). |

Arestas subdivididas a ≤ 1 m antes de triangular, e a leque a partir do centro
da célula: o interior acompanha o declive, e as arestas interiores de duas
células partilham posições exactas (sem fendas).

### Seamless

UVs são **world-space** (`[x / texture-scale, z / texture-scale]`), como nos
decals: o padrão de pedra é um campo contínuo sobre a praça, as ruas e os
apetrechos. Não há frame de UV por nó para alinhar, e como agora é uma malha
só, a continuidade vê-se.

### Uma malha por (material, chunk)

As células são fundidas por grupo de material e por balde de 64 m
(`paving-chunk`), e cada balde é separado em **miolo opaco** + **borda
blended** pelo mesmo `split_ground_surface` dos decals. Numa cidade que eram
dezenas de draws transparentes sobre a praça, sobram ~6 — e o passe
transparente fica só na rampa de feather.

## Camadas (lifts)

A sobreposição entre camadas **não** é resolvida por proximidade: cada camada
tem o seu intervalo e a ordem é fixa.

```text
0.040  DECAL_LIFT            decals no terreno
0.052  DECAL_STACK_CEILING
0.060  RIBBON_LIFT           ribbons legadas (<Road> sem paving)
0.064  PAVING_LIFT           a treliça — UMA camada
0.072  ON_PAVING_LIFT        decals POR CIMA do pavimento
0.082  ON_PAVING_CEILING
0.100  JUNCTION_LIFT         discos de fusão das ribbons legadas
```

`PAVING_LIFT` fica 4 mm **acima** da ribbon de propósito: onde uma trilha de
terra legada cruza uma rua pavimentada, o pavimento ganha de forma
determinística em vez de as duas partilharem um plano.

Um decal cujo **centro** o pavimento cobre sobe sozinho para a faixa
`on-paving` (`stack_decal_lifts` pergunta `paving.covers(at)`): o pavimento é
um chão, e um decal autorado nesse chão deve ver-se. É o que mantém a mancha
de terra do mercado — pintada sobre a praça — visível sem tocar no `.bsn`.
`on-paving="1"` fica como força explícita para o decal cujo centro calha fora
do pavimento mas que deve ficar por cima mesmo assim.

Os discos de junção das redes legadas são suprimidos quando o pavimento cobre o
ponto: já havia o mesmo cuidado para decals, e um disco blended sobre
pavimento opaco seria a quinta camada sobre a rua.

## Gramática

Um `paving` é um **modo de superfície** dos três tipos que já desenham chão.
A migração é um atributo; nenhum nó muda de tag.

```xml
<!-- defaults do mundo, no nó que já existe -->
<Terrain … paving-cell="2" />

<!-- praça / largo / apetrecho: o retângulo vira células -->
<GroundDecal name="city.plaza.floor" at="0 0" size="34 34" paving="1"
             paving-priority="20" feather="3.5"
             texture="/assets/textures/cobblestone_road/albedo.ktx2"
             texture-scale="3.5" base-color="#9d9891" roughness="0.9" />

<!-- rua: a ribbon desaparece, o corredor passa a raster -->
<Road path="…" width="4" flatten="0" paving="1" edge-noise="0.4" … />
<RoadNetwork … paving="1" />

<!-- inset por cima do pavimento (cova de árvore, remendo) -->
<GroundDecal name="oak.pit.ne" at="15 13" radius="2.8" on-paving="1" … />
```

| Atributo | Onde | O que faz |
|---|---|---|
| `paving` | os três | `1`/`grid` liga o modo. `0` desliga (útil para experimentar). |
| `on-paving` | decal | O decal fica **por cima** do pavimento. |
| `paving-cell` | `<Terrain>` e os três | Módulo da treliça em metros (default 2). |
| `paving-corner` | idem | Raio do chanfro nos cantos convexos. Negativo = auto (meio módulo). |
| `paving-inner-corner` | idem | Raio do filete nos cantos côncavos. Negativo = auto (0,35 do módulo). |
| `paving-priority` | idem | Quem ganha uma célula disputada. |
| `paving-chunk` | idem | Balde da malha em metros (default 64; `0` = uma malha só). |
| `paving-round` | decal | Rasteriza o `radius` como disco em vez do retângulo. |

O resto do material é o que já existia: `texture`/`texture-url`,
`texture-scale`, `base-color`, `roughness`, `feather`, `noise`/`edge-noise`,
`seed`. Num `<Road>` o `edge-feather` é uma **fração da meia-largura** (como nas
ribbons); o pavimento converte-a para metros. `paving="1"` num `<Road>` **não**
remove o corredor nem o `RoadPath`: o carve continua a correr e é o que alimenta
`isPointOnRoad`, `distanceToRoadAt`, o pricing de navegação por `NavRoad` e a
folga da relva. Só a ribbon deixa de ser spawnada.

Bridges (`profile="bridge"`) nunca pavimentam: o tabuleiro é um vão suspenso,
não chão.

## O que está migrado hoje

- `world/cities/discordia/roads.bsn` — a praça (priority 20), as 4 ruas
  (priority 10) e os 6 apetrechos de portão. As 4 covas de árvore ficam
  `<GroundDecal on-paving="1">`: são insets deliberados.
- `world/paths/network.bsn` — o `<RoadNetwork>` inteiro. É isto que elimina o
  decal-sobre-decal de hoje: as 21 ribbons de calçada atravessavam a praça e as
  ruas.

Continuam ribbons (não pavimentados): as 28 trilhas de `trails.bsn`,
`desert.bsn`, `gate-arms.bsn` e as pontes — são caminhos de terra orgânicos e o
`<Road>` sem `paving` faz bem esse trabalho. Para os migrar, é acrescentar
`paving="1"` (ver os custos abaixo).

## Custos e armadilhas

- **Quantização de largura.** O snap para fora alarga uma forma até 1 módulo por
  lado: 7 m → 8 m numa grelha de 2 m. Autorar em múltiplos do módulo dá exacto.
- **Fronteira entre materiais é uma linha.** Calçada contra terra fica um corte
  direito na linha da grelha, sem feather. É o que se quer num lancil; se um dia
  se quiser uma transição suave, tem de ser uma rampa alpha **dentro** de cada
  lado (nunca as duas a cair no mesmo sítio, que faz bleed do terreno).
- **Um decal que fica coberto.** Se o centro cair sobre pavimento sobe
  sozinho; se o centro ficar de fora mas o corpo estiver por baixo, o corpo
  desaparece. Marcar `on-paving="1"`.
- **A mancha por baixo do pavimento não z-fighting.** Desaparece. É a regra, não
  um bug: o pavimento é uma camada, não um decal que compete com outros.
- **Feather duplo nos cantos convexos.** As bandas de duas células vizinhas
  sobrepõem-se num ~1 m² no canto (cada uma só cobre a sua aresta, e o
  quadrado do canto é coberto pelas duas). Lê-se como sujidade acumulada no
  canto; não há fenda.

## Verificação

```bash
# engine (crate excluído do workspace, corre standalone)
cd ../bevy_extensions/crates/bevy_ext_terrain_runtime && cargo test --release paving

# mundo
vael-tools analyze world.bsn      # exit 1 em erro
vael-tools lint

# A/B visual: o pavimento desliga e os decals/ribbons antigos voltam
VIBER_PAVING=0 vael-tools run --scene main --entry dev
```

Os testes do módulo cobrem: ocupação única com prioridades, recto sem arcos,
L com chanfro no convexo e filete no côncavo, **nenhum canto de contorno a
virar mais de 45°** a meio módulo, arestas interiores coincidentes (sem fenda),
drape (`|y − (sample + lift)| < 1 mm` numa colina), feather a chegar a alpha 0,
UVs world-space, winding +Y em todos os triângulos, determinismo por hash e a
contagem de malhas por (grupo, chunk). No `features.rs`: um decal pavimentado
não desenha, a faixa de lift de cada tipo de decal, o `RoadPath` de uma estrada
pavimentada continua registado, e o disco de junção desaparece.

## O que ficou de fora

- **Wobble da aresta pavimentada.** O `noise` mexe só na borda exterior da
  feather. Deslocar a própria aresta (inward-only) é possível e seguro, mas
  mexe na garantia de "sem fenda" entre células, e o lancil direito é o que se
  quer numa praça.
- **Feather entre materiais.** Ver acima.
- **Precificação de navegação por célula.** Não é preciso hoje: as estradas
  pavimentadas mantêm o `RoadPath` (que vira `NavRoad`) e a praça está sobre um
  `<TerrainPad>` (que `road_area_volumes` já precifica como estrada). Um largo
  pavimentado *sem* pad nem `Road` não ganharia o preço de estrada.
- **Gizmos no Jackdaw.** O painel nativo mostra os atributos (o `WorldNode`
  leva-os como texto); desenhar a grelha e os arcos no viewport seria trabalho
  de `vael_scene_jackdaw`.

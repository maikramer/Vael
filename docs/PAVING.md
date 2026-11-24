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

### Borda orgânica (o que impede a laje)

Os arcos de canto resolvem o vértice, não a **silhueta**: uma praça de 36 m com
cantos de 1 m continua a ler-se como um retângulo, e um rim diagonal é uma
escada de células. Três coisas, por esta ordem, é que fazem a diferença:

**1. O contorno é suavizado em conjunto, não por célula.** Uma célula só vê o
seu próprio canto, por isso cada uma contribuía com o seu arco e o resultado era
uma fila de escamas de peixe. O `smooth_rims` junta **todos** os segmentos de
fronteira (rim e costura) de todas as células numa polilinha indexada pela
posição no mundo, e relaxa cada nó contra os seus dois vizinhos *ao longo da
fronteira*. Duas células que partilham um nó lêem o mesmo deslocamento, por isso
a malha continua estanque. O nó que não tem exactamente dois vizinhos de
fronteira — fim de linha, pinça, ou o fim de um rim contra uma costura — fica
preso.

O orçamento é sobre o deslocamento **total** (`0,45 × módulo`), não por passagem:
com o limite por passagem o vértice migrava um orçamento a cada passagem e o
contorno dobrava-se sobre si mesmo.

**2. O lancil opaco não ondula.** `paving-wobble` automático é 0: deslocar o
bordo 0,9 m (o antigo `0,45 × módulo`) transformava uma rua de 4 m numa
mancha. O `noise` / `edge-noise` só faz a **largura do feather** respirar,
com as mesmas harmónicas de posição no mundo (~22 m, ~14 m, ~9 m), para a
faixa não parecer uma risca pintada. Um `paving-wobble` explícito ainda
desloca o bordo, com tecto `0,35 × módulo`.

**De onde vem a direção é o contrato.** Não pode vir do contorno da célula:
duas células que partilham um vértice construíram contornos diferentes e
deslizariam o mesmo ponto em sentidos opostos, rasgando a superfície. Vem da
**ocupação da grelha** (`PavingGrid::rim_dir`), que é a mesma pergunta para
todos os donos:

| Situação no ponto da grelha | Direção |
|---|---|
| célula nossa de um lado, **nada** do outro | a normal dessa linha — as duas células ao longo dela concordam, por isso o rim ondula **através** das fronteiras, sem serrilha de 2 m |
| uma célula nossa, três vazias | a diagonal para fora → o canto arredonda |
| duas células nossas opostas | a normal da recta |
| **outro material** de qualquer lado | não mexe: é costura, e o outro lado calcularia o contrário |
| interior da célula (arcos, tangentes) | normal mitrada local, só quando o canto é suave (≤60°) |

**3. Uma estrada é uma fita, não uma escada de células.** A grelha continua
a decidir quem é dono do chão (praça, rua-rectângulo, outro material), mas a
malha de um `<Road paving>` / `<RoadNetwork paving>` é a polilinha já
suavizada (Chaikin, estações a 1 m) extrudida a `half-width`, com mitra.
Largura autoriada, não o módulo. Onde a fita entra numa área (praça, rua
larga) ou num corredor mais largo (empate: o que vem depois no ficheiro),
pára na fronteira — não atravessa por cima, senão as duas superfícies
coplanares piscam. O ruído (`edge-noise`) só faz a faixa de feather respirar;
o lancil opaco não ondula. `paving-wobble` automático é 0.

Em `paving="setts"`, a malha acompanha a distância à borda de todos os ramais
com superfície contínua. O centro sobe cerca de 4 cm em relação à margem. As
esquinas têm uma curva contínua e as pedras do lancil seguem esse contorno,
com juntas estreitas e chanfros. `paving-kerb-texture` escolhe a pedra do
lancil. Na rede de Discórdia usa-se a rocha cinzenta do conjunto do terreno.

Para ligar uma trilha de terra à calçada, marque o nó `Road` com
`paving="trail"` e mantenha sua textura de solo. Coloque a ponta perto do
eixo da rua. O gerador ajusta o extremo à meia pista do lado de chegada
antes de calcular o leito e o caminho de navegação. As duas superfícies
usam a mesma altura e normal no encontro. A terra se mistura à pedra na
entrada, e o lancil acompanha a borda externa. `paving-feather` controla
em metros o desvanecimento da margem externa. As duas entradas na floresta
junto à ponte norte em `world/paths/trails.bsn` são exemplos.

**4. A banda de feather submete-se à curvatura.** Ela é emitida por **nó** (não
por segmento), com direção mitrada para não abrir uma cunha num canto, e:

- o **ruído do rim desvanece-se ao longo dos anéis** (`wob * (1 − t)`), por isso
  os anéis exteriores são deslocamentos do rim *suave*, cuja curvatura é muito
  maior do que a banda;
- a **largura é limitada pelo raio de curvatura local** (`0,8 × R`, com `R`
  estimado do ângulo entre segmentos consecutivos), para uma esquina apertada
  levar uma banda estreita em vez de uma invertida;
- o raio de curvatura só conta quando os **dois** segmentos adjacentes são de
  fronteira. Um nó do rim é muitas vezes também o canto da *célula*, onde o
  segmento seguinte é uma aresta interior: tomar esse 90° como curvatura
  apertava a banda uma vez por célula, e daí o mosaico de triângulos ao longo do
  lancil.

Por último, a rampa de alpha é linear por triângulo, por isso com poucos anéis a
diagonal de cada quad lê-se como uma faceta — `FEATHER_RINGS = 5`.

Na **costura entre materiais** a banda é do material que fica por cima no
ficheiro (`SEAM_LIFT = 1,5 mm` acima do outro) e, quando o traçado é igual nos
dois nós, os dois pertencem ao **mesmo grupo**: mesma textura, escala, cor,
`roughness`, feather, ruído, canto e wobble ⇒ uma malha só e nenhuma costura.
O `paving-priority` decide a ocupação de cada célula.

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
um chão, e um decal autorado nesse chão deve ver-se.
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
<GroundDecal name="exemplo.largo" at="0 0" size="24 24" paving="1" paving-round="1"
             paving-priority="20" feather="1"
             texture="/assets/textures/cobblestone_road/albedo.ktx2"
             texture-scale="3.5" base-color="#9d9891" roughness="0.9" />

<!-- rua: a ribbon desaparece, o corredor passa a raster -->
<Road path="…" width="4" flatten="0" paving="1" edge-noise="0.4" … />
<RoadNetwork … paving="setts" paving-cell="0.75" paving-feather="0.58" />

<!-- inset por cima do pavimento (cova de árvore, remendo) -->
<GroundDecal name="exemplo.marca" at="15 13" radius="2.8" on-paving="1" … />
```

| Atributo | Onde | O que faz |
|---|---|---|
| `paving` | os três | `1` ou `grid` liga a grelha. Em estradas, `setts` acrescenta abaulamento e guias de pedra chanfradas. `trail` cria um ramal de terra que se une à calçada e abre a guia na entrada. `0` desliga. |
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

- O largo oval e a rosa dos ventos ficam em `world/cities/discordia/roads.bsn`.
  São dois decals sobre a rede. O largo mede 24 × 22 m, com 2 m de transição
  na margem e a mesma calçada das ruas. O mosaico ocupa os 8 × 8 m centrais.
- As quatro ruas entre a praça e o anel são segmentos de
  `world/paths/network.bsn`, com 5,8 a 6 m de largura. Os pontos intermédios
  dão a curva a cada rua. A mesma rede continua pelo centro dos portões.
- A terra batida do mercado, do adro e da forja está pintada em
  `world.data/terrain.ahgt.paint` com a camada `dirt_road`. O peso diminui
  sob as árvores e junto à cerca. Os objetos do mercado assentam à cota
  do terreno e o tapete fica 5 mm acima.

Os acessos da Passadeira da Bruxa e da ponte do Ermo usam `paving="trail"`.
A largura acompanha o tabuleiro junto à cabeceira e muda suavemente até a
largura da trilha. Na Ponte Norte, a calçada entra reta pelas duas cabeceiras,
com as bifurcações da floresta recuadas em terra firme. Os demais caminhos
sem `paving` continuam a usar a faixa de terra sobre o relevo.

O teste `cargo test --test road_crossings` verifica no mundo resolvido o
alinhamento das seis cabeceiras, a continuidade até a praça e a passagem das
estradas por terra seca.

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
VAEL_PAVING=0 vael-tools run --scene main --entry dev
```

Os testes do módulo cobrem: ocupação única com prioridades, recto sem arcos,
L com chanfro no convexo e filete no côncavo, **nenhum canto de contorno a
virar mais de 45°** a meio módulo, arestas interiores coincidentes (sem fenda),
drape (`|y − (sample + lift)| < 1 mm` numa colina), feather a chegar a alpha 0,
UVs world-space, winding +Y em todos os triângulos, determinismo por hash, a
contagem de malhas por (grupo, chunk), a fita diagonal na meia-largura
autoriada, o cruzamento em que a rua mais estreita pára no bordo da larga, e
o rectângulo de rua que não é puxado para a artéria. No `features.rs`: um
decal pavimentado não desenha, a faixa de lift de cada tipo de decal, o
`RoadPath` de uma estrada pavimentada continua registado, e o disco de junção
desaparece.

## O que ficou de fora

- **Cantos de praça maiores do que meio módulo.** O arco de canto de uma área
  vive dentro de uma célula; um raio de 4 m numa praça de 36 m precisaria de
  um contorno por região. As estradas não passam por aí: a fita usa a
  polilinha. Hoje o canto de uma área arredonda pelo chanfro (≤ 1 m a 2 m de
  módulo).
- **Feather entre materiais.** Ver acima.
- **Precificação de navegação por célula.** Não é preciso hoje: as estradas
  pavimentadas mantêm o `RoadPath` (que vira `NavRoad`) e a praça está sobre um
  `<TerrainPad>` (que `road_area_volumes` já precifica como estrada). Um largo
  pavimentado *sem* pad nem `Road` não ganharia o preço de estrada.
- **Gizmos no Jackdaw.** O painel nativo mostra os atributos (o `WorldNode`
  leva-os como texto); desenhar a grelha e os arcos no viewport seria trabalho
  de `vael_scene_jackdaw`.

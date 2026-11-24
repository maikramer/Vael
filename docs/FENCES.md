# Cercas procedurais (`bevy_ext_fence`)

Uma cerca é uma **linha** autorada no `.bsn`; a geometria — postes, travessas,
tábuas, paliçada, varas entretecidas, treliça, cruzetas, cordas ou portão —
nasce no jogo a partir dessa linha e de uma predefinição. Substituiu a 2026-11-23
os ~128 segmentos do GLB `farm/fence_segment` (e o `fence_gate`), que eram a
mesma peça rígida repetida: sem continuidade de travessas, sem portas à
medida, com buracos de ~3 m em três cantos do cercado de Discórdia (a cerca
que rodeia a praça da vila).

A arte é a do projecto (*Ofício Talhado & Guache Mate*, [`ART.md`](ART.md)):
madeira desbastada a enxó em facetas planas com chanfros, acabamento mate,
desgaste só onde há uso — e o veio vem das texturas do pool, não de listras
procedurais.

## Autoria

Um `<Entity>` com `FenceSpec`:

```ron
<Entity name="cerca-oeste" translation="-18.5 8.2704 94.2"
        fence-style="pasture_rail"
        fence-points="-24.6 0 0; 5.9 0 0" />
```

`fence-points` é a polilinha no espaço da entidade: `"x z; x z; …"` (à cota
autorada) ou `"x y z; …"` (cota própria de cada ponto). Os postes nascem nos
vértices e intervalados até nenhum vão passar de `spacing`.

| Atributo | O que manda | Por omissão |
|---|---|---|
| `fence-style` | chave de uma predefinição de `world/settings/fences.bsn` | a `default_style` das definições |
| `fence-points` | a linha (obrigatório: sem linha não há cerca) | — |
| `fence-closed` | fecha o perímetro (último → primeiro) | aberta |
| `fence-kind` | `rail`, `plank`, `picket`, `palisade`, `wattle`, `lattice`, `crossbuck`, `rope`, `gate` | o da predefinição |
| `fence-height` | altura ao topo do poste (m) | idem |
| `fence-rails` | travessas horizontais por vão | idem |
| `fence-boards` | tábuas verticais por vão; 0 = caberem pela largura | idem |
| `fence-board-length` | comprimento da tábua, mínimo e máximo (m): cada tábua sorteia dentro | idem |
| `fence-spacing` | distância máxima entre postes (m) | idem |
| `fence-material` | conjunto de texturas (`wood_oak_hewn`, `wood_swamp_tar`, …) | idem |
| `fence-seed` | semente da variação | deriva do nome |
| `fence-collider` | `segment` (uma caixa fina por vão), `posts`, `none` | `segment` |
| `fence-follow` | `terrain` (os postes descem até ao chão) ou `points` (ficam na cota autorada) | `terrain` |
| `fence-open` | abertura da folha do portão em graus (0–150) | a da predefinição (fechado) |

Tudo o que é `Option`: o que não vem é da predefinição, e uma cerca pode
sobrepor só o que quiser sem criar uma predefinição nova.

- **Perímetro**: quatro pontos e `fence-closed="1"` — um poste de canto em cada
  vértice, sem repetir o primeiro.
- **Portão**: uma entidade com `fence-style="gate"` cujos dois pontos são os
  postes; a folha (tábuas, travessa diagonal, dobadouras e trancas em ferro
  batido) nasce entre eles. Cada instância abre com `fence-open="95"`; o
  colisor da folha roda com ela e o vão fica livre.
- **Interiores** (sem terreno): pontos `x y z` e `fence-follow="points"`.
- **Declives**: até ~15° as travessas seguem a rampa; acima passam a degraus.

## Predefinições

`world/settings/fences.bsn` (pasta *Definições* do `world.bsn`) tem
`FenceSettings` e dez `FenceStyle`, editáveis campo a campo no Jackdaw — o
inspector mostra-os na categoria *Cercas*:

| Chave | Tipo | Altura | Notas |
|---|---|---|---|
| `pasture_rail` | travessas | 1,2 m | duas travessas de carvalho; a cerca do Vale |
| `paddock_3rail` | travessas | 1,25 m | três travessas, postes a 2,6 m |
| `plank_board` | tábuas | 1,3 m | 18 tábuas por vão, comprimento 0,95–1,12 m, remate no topo |
| `picket` | estacas | 1,0 m | topo em ponta |
| `palisade` | paliçada | 1,9 m | troncos de oito faces (14 cm) em ponta, pinheiro escuro |
| `wattle` | varas entretecidas | 1,0 m | estacas de aveleira, oito varas tecidas |
| `lattice` | treliça | 1,5 m | ripas a 45°, lavada a cal |
| `crossbuck` | cruzetas | 1,3 m | diagonais em X por vão |
| `rope` | cordas | 0,95 m | duas cordas de cânamo com flecha |
| `gate` | portão | 1,6 m | folha de tábuas com diagonal e ferragens |

Um `FenceStyle` traz tudo o que o gerador usa: secção dos postes e travessas
(`Timber`: talhada a enxó com chanfro, ou tronco de N facetas), reforço dos
postes de canto, quanto se enterram, inclinação, alturas das travessas e o
espigão, tábuas (largura, vão, topo, comprimento), ripa, estaca, vara, corda
(flecha, diâmetro), abertura do portão, material principal e secundário, metros
por repetição da textura, altura e tinta do desgaste de base, variação de tom e
semente. Regenerar só este ficheiro a partir do crate:
`vael::settings::export_fence_settings` (o `--export-settings` do binário
reescreve todos).

## Regras de geração

As regras vivem em `bevy_ext_fence::layout` (puro e determinístico: a mesma
cerca nasce igual em todas as sessões) e a variação sai de um RNG semeado pelo
nome da cerca.

- **Postes**: nos vértices e intervalados; o último vão divide-se por igual
  (sem tocos). Num perímetro fechado não há poste repetido na junção. Os de
  canto, ponta e portão são mais fortes (×1,25). Descem 0,35 m abaixo do chão
  para não abrirem vão em rampa. Inclinação até 2°, rotação e secção com
  oscilação — dois postes iguais a fio denunciavam a moldura.
- **Chão**: a base de cada poste é a cota do terreno nesse ponto (`TerrainRuntime`).
- **Travessas**: entre postes, com as pontas a sair 5 cm da face (espigão e
  morta); ±1,5 cm e ±1,5° por travessa. O número manda nas alturas: com mais
  travessas do que a predefinição tem, distribuem-se por igual entre a mais
  baixa e a mais alta (`fence-rails="3"` numa de duas dá três).
- **Tábuas**: pregadas à frente das travessas, largura e comprimento
  sorteados — é daí que vêm os topos desiguais —, com um par de pregos em cada
  travessa que apanham.
- **Varas**: estacas a cada 0,5 m e varas tecidas à frente e atrás delas,
  afinando até à ponta.
- **Cordas**: catenárias com 8–15% de flecha e um nó em cada ponta.
- **Malha**: facetas planas, UV em metros ao longo do veio com desvio por peça
  (duas tábuas com o mesmo veio denunciavam a repetição), cor de vértice com o
  tom de cada peça, oclusão nos encontros e o desgaste de base (terra e líquen
  nos primeiros 28 cm).

## Texturas

Nove conjuntos na convenção do pool
(`assets/public/assets/textures/<conjunto>/`), gerados por
`tools/gen_fence_textures.py` a partir de
`assets/manifests/wood-textures.yaml`:

| Conjunto | De onde vem (ART.md §4) |
|---|---|
| `wood_oak_hewn` | Vale/Discórdia · Carvalho `#7A5C3A` |
| `wood_pine_dark` | Floresta Sombria · Pinho Negro `#232B24`, Turfa `#3B2F26` |
| `wood_sun_bleached` | Ermo Rubro · Madeira Sol `#C2B39B` |
| `wood_swamp_tar` | Pântano da Bruma · Alcatrão `#3D352E` |
| `wood_shelter_pine` | Picos Gelados · Pinho de Abrigo `#47362B` |
| `wood_weathered_grey` | Campo e Cidadela Caída · Ferro Velho `#6B3E2E` |
| `wood_lime_washed` | Vale/Discórdia · Cal `#E6DEC8` |
| `wood_hazel_rod` | varas com casca (Junco `#8C7A4B`) |
| `rope_hemp` | corda de cânamo (estilo de cordas) |

Cada pasta: `albedo`, `normal`, `orm`, `ao`, `roughness`, `height` em `.webp`
e `.ktx2` (UASTC, 1024², cadeia de mipmaps), e `gen.json` com o prompt, a seed
e os portões. Tudo a 1024² como o resto do pool; o veio corre de cima a baixo
(eixo V) e o gerador alinha-o com o comprimento de cada peça.

- **`orm` é o mapa que o `StandardMaterial` do Bevy 0.20 lê**: R = oclusão,
  G = roughness, B = metálico (a zero na madeira). O `metallic_roughness_texture`
  lê o azul — lá não pode ir um mapa de roughness em tons de cinzento, senão a
  madeira parece metal.
- A roughness já vem remapeada para 0,78–0,95 (ART.md: guache/têmpera mate).
- **Superfície contínua, sem juntas de tábuas**: as tábuas são geometria; uma
  junta desenhada atravessava cada peça. O sufixo comum do prompt evita
  «wood grain texture», que em alguns conjuntos puxava para tábuas.

O pipeline é o do `texture2d` (SD1.5 seamless) → `materialize` (height, normal,
smoothness, AO) → Pillow/numpy (normal atenuada, roughness, `orm`) → `ktx
create`. Os portões de qualidade são os da própria ferramenta, cada um com
re-roll para a seed seguinte (`--attempts 6`), calibrados nestes nove
conjuntos:

- `--max-isolated-detail 0.12` — um nó ou uma mancha que o tiling repetiria em
  cada célula;
- `--max-banding 0.5` — tábuas com frestas (0,87) e veio deitado (0,68) contra
  0,20–0,44 no veio direito; com `--axis v` o que sair deitado é rodado;
- `--max-tail-ratio 5` — flocos sobre um fundo liso (28 contra 1,5–3,2);
- `--match-color` — **a cor é a da paleta**: o SD1.5 tem um prior de tom médio
  (as nove saíam castanhas, com luminância 96–128 de 255, qualquer que fosse o
  prompt), pelo que o albedo é graduado até à cor média pedida (`match:` no
  manifesto) e mantém a estrutura.

O script ainda exige tileabilidade ≥ 0,85 e repete com `--no-vae-tiling` se o
decode cair em tiles. As folhas de verificação (tiling 3×3 + mapas) ficam em
`/tmp/vael-wood-textures/`.

## Como corre no jogo

- **Crate** `bevy_extensions/crates/bevy_ext_fence`: contrato da peça
  (`Timber`, `FenceStyle`, `FenceSettings`), `layout` (linha → postes → peças),
  `mesh` (peças → malhas), `material` (um `StandardMaterial` por conjunto) e
  `FencePlugin` (nasce em `Added<FenceSpec>`, refaz em `Changed`, em lotes de
  8 por frame). A feature `physics` põe na entidade um corpo fixo
  (`RigidBody::Static`) com um colisor Avian composto: uma caixa fina por vão —
  o herói pára nela; um portão fechado bloqueia o vão, aberto a caixa roda com
  a folha. O corpo fixo é obrigatório: com a forma sozinha os raios batiam na
  caixa e o herói atravessava-a (QA de 2026-11-24).
- **Adaptador** `src/fence.rs` (`VaelFencePlugin`): o chão é o `TerrainRuntime`
  (fora do heightmap não há chão); as definições da cena chegam como recursos e
  são copiadas para o crate só quando mudam; fora do Play do Jackdaw o
  `FenceSpec` do load é posto na entidade. Gate `VAEL_FENCE=0` (o `VIBER_FENCE`
  legado continua a valer).
- **O parser da engine ignora os `fence-*`** sem avisar: são o transporte do
  componente tipado, e quem os lê é o `bevy_ext_fence`.
- **Edição ao vivo**: mexer no `FenceSpec` no Play re-spawna a cerca
  (`src/live_edit.rs`).

## Jackdaw

A extensão `bevy_extensions/extensions/bevy_ext_fence_jackdaw` dá ao editor o
mesmo crate do jogo:

- **preview no viewport** (filhos `EditorHidden`/`SkipSerialization`, nunca no
  `.bsn`), refeito a 0,5 s; sem terreno — os postes ficam à cota autorada;
- **gizmo** da linha e dos pontos na cerca seleccionada;
- categoria **Cercas** no inspector (`FenceSpec`, `FenceStyle`, `FenceSettings`) e
  ícone no Outliner;
- painel **Cercas**: estado e um botão por predefinição que cria uma linha nova
  (por `entity.add.empty` + `component.add` + `component.set`: undo e `.bsn`).

Ciclo: `jd build` → `jd extension pack` → `jd extension install
--trust-publisher`; activar em `jackdaw.toml` e reiniciar o editor.

## As cercas do mundo

`tools/fence_migrate.py` fez a troca (2026-11-23): agrupa os segmentos por
linhas, emenda as que se encontram nos cantos — o canto é a **interseção** das
duas linhas, o que fechou os buracos dos cantos do cercado — e escreve um `<Entity>` com
`FenceSpec` por polilinha, no espaço do grupo que continha as cercas.

| Ficheiro | O que ficou |
|---|---|
| `world/cities/discordia/walls.bsn` | o cercado da vila (a cerca à volta da praça, de ±38 m): `cercado-so`, `cercado-se`, `cercado-no`, `cercado-ne` (20 segmentos cada, cotovelos de 3 pontos); as quatro entradas a meio dos lados ficam abertas como estavam |
| `world/cities/discordia/approach.bsn` | socalco oeste: `cerca-w-se` (cotovelo), `cerca-w-2`, `cerca-w-3`, e o `portao-socalco` como portão procedural; socalco este: `cerca-e-2`, `cerca-e-3`, `cerca-e-4`; o protótipo `cerca` saiu |
| `world/quest-stations.bsn` | `cerca-x-1-z0` (perímetro fechado de 15 segmentos — os «pentes» de through viraram as paredes do curral) e `cerca-x-1-z-1` (cotovelo) |
| `world/scenes/smugglers.bsn` | `palicada-contrabandistas`, agora paliçada de verdade (era um segmento a escala 2,2) |
| `scenes/interiors/barn.bsn` | `interior.barn.fence.segment` (pontos a 3D, `fence-follow="points"`) |

Os modelos `farm/fence_segment`/`fence_gate` (GLB, LODs, colisões,
`_precompute`), o prefab `world/prefabs/farm/fence-segment.bsn` e as entradas
do `assets/manifests/farm.yaml` saíram do repositório. O script não é
idempotente: `--check` dá exit 1 se ainda houver cercas de GLB; para repetir,
`git checkout -- <ficheiro>.bsn` antes.

## QA

`scenes/qa/fences.bsn` (chão plano, carrega em ~10 s): uma linha de 20 m por
estilo, um curral fechado, portões fechado e aberto, e exemplos de sobreposição
(tábuas a 6 e altura 1,6; estacas apertadas a 1,5 m; madeira do pântano com
três travessas). `python3 tools/qa.py run fences` lança-a (o cenário é
`qa/scenarios/fences.json`; `qa.py run` só aceita cenários, não cenas).

Ao vivo, num jogo com `--bridge`: `vael-tools debug --port <p> wait-ready`, e
a câmara livre pelo Luau — `vael.camera_free(true)` seguido de
`vael.camera_pose(...)` — para enquadrar cada sítio. Sítios do mundo: o
cercado de Discórdia (`cercado-so`, entrada sul a meio do lado), o socalco
oeste (`portao-socalco`, `cerca-w-2`) e o curral da estação (`cerca-x-1-z0`);
`vael.find(nome)` dá o id e `vael.entity_position(id)` a posição. O
`vael.raycast` da bridge confirma os colisores (os raios batem na caixa de
20 cm de cada vão, passam nas entradas e nos vãos dos portões abertos). A/B
com `VAEL_FENCE=0` (as linhas ficam sem visual nem colisor). Portões do
ART.md §10: silhueta a 20 m, anti-plástico ao meio-dia (`VAEL_START_HOUR=12`),
sem costuras.

O jogo não tem modo sem janela: a QA na GPU abre uma janela no ecrã, que
rouba o foco e engole o que se estiver a escrever — correr em corridas
curtas e scriptadas (lançar, `wait-ready`, imagens/raios, fechar). Em CPU
(Xvfb + lavapipe) a cena de QA carrega em ~100 s e serve para geometria, não
para cor: a frames por segundo baixas o herói atravessa as caixas por
túnel, o que na GPU não acontece.

## Em aberto

- **Editor**: a extensão compila, instala e carrega (`jackdaw check-extensions`),
  mas o preview no viewport nunca foi visto numa janela, e a edição ao vivo no
  Play (mexer no `FenceSpec` re-spawna a cerca) também não foi exercitada.
- **Navegação**: não está verificado se o navmesh trata as cercas como
  obstáculo — os colisores nascem depois do load.
- **Celeiro**: a cerca de interior (`interior.barn.fence.segment`, pontos a 3D)
  parses e analisa sem erro, mas não foi vista ao vivo.
- **Madeiras escuras**: pinheiro escuro (`#2F2D25`) e pinho de abrigo
  (`#47362B`) ficam quase pretos na sombra — é a paleta, mas a cruzeta e os
  postes da corda usam-nas por omissão; troca-se o `material` da predefinição.
- **Texturas**: quatro conjuntos (carvalho, pinho escuro, madeira sol, abrigo)
  mantêm um ponto claro minúsculo por tile que os portões não apanham.
- Gizmo arrastável dos pontos / desenhar a cerca por cliques no editor (v1 é o
  campo `points` + preview).
- As cercas não são destrutíveis nem dão madeira (as antigas também não eram).
- O preview do editor não segue o terreno (o viewport não tem o heightmap do
  jogo).

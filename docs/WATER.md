# Água do Vael

A simulação, o render e a flutuação vivem na crate **`bevy_ext_water`**
(`../bevy_extensions/crates/bevy_ext_water`), que só depende de `bevy` e
`avian3d`. A engine (`bevy_ext_viber_engine`) mantém só os adaptadores do
jogo: o carve do terreno (que conhece a grelha), a fonte `WaterSource` do
`TerrainRuntime`, a conversão de colliders avian em obstáculos, as partículas
e o marcador `WaterBodied`.

Estrutura da crate:

| Módulo | O que é |
|---|---|
| `body` | `WaterBody` (lago, rio, oceano), `sample_domain`, contornos |
| `mirror` | espelhos CPU do shader (`surface_height_at` desenhado, `wave_height_at` normais) |
| `waves` | solver iWave em compute, janela de 128 m, domínio `(depth, flow, plunge)` |
| `material` | material dois passes + `WaterEnvironment` |
| `fx` | contactos, esteira, anéis e `WaterSplashEvent` |
| `buoyancy` | `FloatingBody`/`FloatState` + adapter avian |
| `obstacles` | SDF/deflexão/halo dos obstáculos |
| `source` | `WaterSource`, `WaterFocus`, `WaterObstacles` |

A água tem três camadas que não se podem confundir:

| Camada | Dono | O que responde |
|---|---|---|
| **Domínio hidrodinâmico** | `bevy_ext_viber_engine::terrain::water` (`WaterBody::sample_domain`, `TerrainRuntime::water_domain_at`) | `surface_y`, profundidade, corrente e fervura de queda num XZ |
| **Solver interactivo** | `terrain::water_waves` (`WaterWavePlugin`, `water_waves.wgsl`) | ondas dispersivas iWave numa janela rolante de 128 m (`512²` a 0,25 m) + domínio rasterizado (`domain_tex`) |
| **Superfície desenhada** | `terrain::water.wesl` | Gerstner de 4 trens + detalhe + iWave, com máscaras de praia/objectos |

O solver e o domínio vivem na GPU. A **cota que a física sente** é o espelho CPU
do Gerstner: `terrain::water::surface_height_at(p, t, wind, wind_strength,
wave_amp, depth)` — replica linha a linha o `gerstner_swell(...).y` do vertex
shader, incluindo o clamp ±0,12, o ganho 0,18·(0,45+0,40·vento) e a rampa de
praia `smoothstep(0,38; 1,35; depth)`.

> **Não uses `wave_height_at` para flutuar.** Esse é o espelho do `wave_height`
> do shader (família de senos e ganho diferentes), que alimenta as **normais**.
> Usá-lo punha o barco a subir numa onda que a malha não desenha. Há um teste
> que exige que os dois espelhos sejam diferentes.

O termo interactivo (iWave) **não** é espelhado em CPU: a amplitude desenhada é
≤0,10 m e espelhá-lo exigiria readback da `wave_tex` (2 MB/frame). A física
sente o swell + a corrente; a esteira que o barco *produz* entra no solver pelo
caminho normal (`WaveImpulseRequest`).

## Secção molhada e corrente dos rios

A lâmina de um rio já não usa a largura nominal (`width` da spec, ±20 % de
poços/rápidos). No load, depois de todos os carves (estradas incluídas),
`fill_river_section` (`bevy_ext_terrain_runtime::water`, passo 6 de
`apply_features`) mede cada estação para os **dois lados, separadamente**: a
lâmina estende-se até tocar o terreno carvado, com afinação abaixo do texel.
O resultado fica em `WaterBody::section` (`wet_left`/`wet_right`, `open`,
`hydro`). A malha, o domínio (barco, peixes, solver), a relva e os spawners
leem esta mesma métrica (`water_half_at`). As margens exteriores das curvas, os
taludes recuados por estradas e os alargamentos deixam de ficar como uma faixa
de leito seco à vista.

- Um lado que não toca o terreno dentro de `nominal + bank + feather + 2 m`
  (encosta a descer, confluência) fica **aberto**. Mantém a largura nominal e o
  fade de alfa.
- Nas curvas, o lado interior não passa de `0,85 / κ`, para que as secções
  vizinhas não se cruzem.
- A largura é suavizada (mediana de 5 estações + média ±1), mas nunca recua
  mais de 25 cm para dentro do ponto onde a lâmina toca o terreno.
- Margens voxel (`gorge`/`overhang`) ficam com o contrato antigo, porque a
  parede não está na grelha.

A corrente sai da secção (`bevy_ext_water::hydro`):

| Grandeza | Como |
|---|---|
| velocidade média | Manning `R^(2/3)·S^(1/2)/n` (n = 0,035; S ≥ 0,0012) × continuidade `Q/A` por troço sem quedas (média geométrica): o estreito acelera, o poço abranda |
| perfil lateral | `v̄·(h/h̄)^(2/3)` por faixa vertical + camada-limite de 0,9 m na margem: rápido no talvegue, lento no baixio e junto à margem |
| remanso | alargamento brusco de um lado (`dW/ds`) → nos ~4 trechos a jusante, a água junto a essa margem inverte e gira para o centro |
| regime | Froude da secção: ≳ 1 vira água branca mesmo com pouco declive; < 0,3 e fundo dá um espelho calmo |

`section_flow` é a única fórmula: o vértice da malha e o
`WaterDomainSample.flow` dão a mesma corrente (há teste). A malha leva
`(bank_dist, froude, eddy, open)` no `ATTRIBUTE_TANGENT` (cru, não é uma base
tangente). O `water.wesl` usa-o para o arrasto e a espuma de margem medidos a
partir da margem real, para a água branca em regime supercrítico, para as
linhas de espuma a girar nos remansos e para acalmar os troços lentos e
fundos. Sem a secção (lagos, corpos antigos), o shader volta aos proxies de
profundidade.

Tudo isto corre no load, sobre a grelha já carvada. Não muda o `.ahgt` nem o
`.bsn`, por isso não há rebake.

## Flutuação (crate `bevy_ext_water`)

`terrain::water_buoyancy` — genérico, sem nada de barco:

- [`FloatingBody`] (componente): `mass` (kg), `hull_half` (meia-boca,
  meia-altura, meia-comprimento), `hull_center`, `center_of_mass`,
  `displacement_factor` (V-hull ≈ 0,55), `buoyancy_gain`, `linear_drag`,
  `quadratic_drag`, `angular_drag`, `wave_response`, `max_speed` e a mola de
  encalhe (`ground_*`).
- [`FloatState`] (componente): velocidade linear, `yaw` (autoral/leme),
  `pitch`, `roll`, velocidades angulares, `submerged`, `grounded`.
- `WaterBuoyancyPlugin` integra flutuadores simples. Nos barcos, `AvianBuoyancyPlugin` aplica o empuxo e o torque em cada passo fixo. O Avian resolve os contactos e integra o casco convexo.

Modelo: o casco é uma caixa amostrada em **9 colunas** (3×3). Cada coluna
desloca `área · altura_submersa` no seu próprio XZ; Arquimedes empurra para cima
no centro da coluna e o **momento restaurador sai da geometria** (estabilidade
metacêntrica), não de uma mola. Consequências:

- casco mais largo → rola menos; `center_of_mass` alto (GM negativo) → **emborca**
  (há teste);
- corrente empurra até um equilíbrio abaixo da velocidade da água (arrasto
  linear + quadrático);
- em seco cai e assenta na cama (mola + atrito); na praia raspa e trava;
- `wave_response` escala o swell; o heave é limitado pelo arrasto.

**Origem e equilíbrio:** a origem da entidade é a linha de água *nominal*; a
quilha está em `hull_center.y − hull_half.y`. Em água parada o casco assenta com
`equilibrium_draft()` de calado e a origem fica `rest_height()` acima da
superfície. Nascer em `surface + rest_height()` evita o "plop" inicial.

**Contacto/esteira:** o `WaterBuoyancyPlugin` espelha a velocidade em
`WaterKinematics`. O `WaterKinematics.contact_y_offset` diz ao `water_fx` qual é
a peça que toca a água (pés = 0; barco = quilha, negativo). O limiar de "molhado"
escala com o corpo (`0,22 m · 0,42/raio`), senão uma quilha com 7 cm submersos
nunca entrava como molhada e não havia esteira. O raio da esteira também escala
com o casco (até `2,6·raio`).

## Barcos (Vael)

`src/boat.rs`:

- `data/boats.json` lista `{ name, x, z, yaw }`; nascem onde há
  profundidade para toda a quilha. São entidades de runtime (como as criaturas), não
  nós do `.bsn`; um `SceneLoad` despawna-os e a cena seguinte volta a semeá-los.
- Casco, bancos e remos são **malha procedural** (`rowboat_hull_mesh`,
  `rowboat_oar_mesh`); materiais mate com `cull_mode: None`.
- `Boat` = aceleração, reverse, velocidade máxima, sprint, taxa de guinada e
  fase dos remos. `row_thrust`/`row_yaw` são puros e testados.
- **E** (acção `interact`) a menos de `BOARD_RADIUS` entra; a bordo, **E** sai.
  O corpo dinâmico do herói fica desativado durante a viagem e volta a
  simular ao desembarcar.
- A bordo: **W/S** rema à frente/atrás, **A/D** dá leme, **Shift** sprinta. O
  herói fica sentado de frente para a popa. O estado `Row` usa a fase das pás
  para escolher a pose do clipe `row` do herói. Ao largar W/S, as mãos
  repousam nos cabos. A câmara continua voltada para a proa.

O barco exige 60 cm de profundidade sob a proa, a popa e as laterais.
Ao chegar a um baixio, o casco perde o avanço mesmo com os remos recolhidos.
Remar de ré permite afastar a proa da margem.

Ao chegar à praia, o HUD oferece **Desembarcar na margem** quando encontra
piso seco a um passo do casco, inclinação caminhável e espaço para a cápsula
do herói ao longo do trajeto. A tecla de interação confirma esse ponto.
Ao pisar a margem, o herói prende o barco a um piquete com uma corda.
A amarra fica guardada com o barco e solta-se ao voltar a embarcar.
Em água aberta, a mesma ação permite sair para nadar pelo lado livre.

O volume de colisão acompanha a proa, a popa e a quilha. A detecção contínua
cobre impactos em velocidade. O empuxo em nove pontos inclina o barco com
as ondas, inclusive quando os remos estão recolhidos.

As pás giram nos toletes de bombordo e estibordo. Na puxada, cada pá
submersa contribui com metade da força e abre uma onda no primeiro contacto.
Soltar W/S levanta os remos e deixa o casco deslizar com o embalo e a corrente.

## Margens e pontes do mundo principal

Os onze lagos têm perfis próprios de margem no BSN. O rio em
`world/spawn/ring.bsn` combina meandros largos com curvas menores. Os 201 níveis
de água seguem as estações do leito de montante a jusante. O relevo estático está
em `world.data/terrain.ahgt` e os contornos em `world.data/bake.json`.

O arquivo Blender [Travessias.blend](../art/bridges/Travessias.blend) reúne os
três modelos, cada um em uma cena com coleções de modelo e colisão. Os
tabuleiros ligam as cotas das duas margens.

| Travessia | Comprimento | Construção |
|---|---:|---|
| Ponte Norte | 68 m | Treliças de madeira sobre pilares de pedra com talhamares |
| Passadeira da Bruxa | 32 m | Pranchas sobre estacas, corrimãos de corda e amuletos de cobre |
| Ponte do Ermo | 36 m | Abóbada de arenito com aduelas, contrafortes e parapeitos |

Para reexportar os modelos após editar o Blender, rode na raiz do Vael:

```sh
blender -b art/bridges/Travessias.blend -P art/bridges/export.py
python3 tools/make_lods.py --only 'infra/ponte_*' --only 'infra/passadeira_da_bruxa_*'
```

O exportador reúne as peças de cada ponte em uma malha com vários materiais,
para que o tabuleiro, os apoios e os corrimãos cheguem juntos aos LODs.

## Hidrovias

`world/landmarks/hidrovias.bsn` junta o **Lago do Estaleiro** (canto NW da
Concórdia), o cais e a ponte alta em `world/cities/discordia/shipyard.bsn`.
O `barco-do-estaleiro` nasce amarrado ao cais quando o canteiro fica
habitado. O braço liga o lago ao Pântano conforme o
[GDD](../GDD.md#concórdia--os-canteiros).

| Braço | De → para | Notas |
|---|---|---|
| `ribeira-de-concordia` | Lago do Anel → Lago do Estaleiro | 170 m junto à margem oeste de Concórdia, largura nominal de 17,5 m e uma travessia entre a cidade e o caminho do estaleiro |
| `braco-do-estaleiro` | Estaleiro → pântano (sw1) | 22 m de largura nominal, 2,8 m de leito, ponte sem pilares no eixo navegável |
| `braco-dos-juncos` | sw2 → sw1 | navegável; ponte de pedra na estrada oeste |
| `ribeiro-do-lodo` | sw3 → sw1 | rápidos; ponte de madeira no trilho |
| `ribeiro-do-ermo-verde` | Ermo Verde (wilds) → sw3 | rápidos e uma cascata; ponte de madeira |
| `cascata-do-anel` | Anel → sw2 | o Anel está empoleirado: 13 m de rápidos |
| `riacho-dos-picos` | lago dos Picos → Estaleiro | cascatas no flanco oeste; ponte de pedra na estrada norte, no talvegue |
| `braco-do-areal` | lago do Areal → rio | navegável |
| `estreito-das-vigilias` | Vigília menor → maior | cascatas curtas entre os planaltos |
| `rio-das-vigilias` | Vigílias → rio (estação 9) | cachoeiras na descida do planalto; 2 pontes de madeira |

Os traçados seguem talvegues (rota de custo mínimo sobre o `.ahgt`) e levam
deslocamento lateral fractal por ponto médio (3 oitavas), como os meandros do
rio antigo. Os níveis são explícitos e limitados a ~8 % nos braços navegáveis
(corrente ≤ 2,3 m/s; o barco faz 4–6 m/s); acima de 1,2 m entre estações vira
cascata.

Ficam **duas redes navegáveis**: rio + Areal + Vigílias, e Estaleiro +
pântanos. Entre elas há um colo de 44,7 m a norte do rio, ocupado pela
Passadeira da Bruxa e pelo acampamento da torre; o canal custaria um corte de
~5 m (rampa de ~17 m para cada lado) por cima deles, ou ~21 m por fora. O
Bosque Encantado fica isolado por uma crista de 167 m.

**Mundo estático:** o runtime carva a água numa cópia da grelha. Um lago ou
rio novo regista a lâmina mas não abre o leito até ser gravado no `.ahgt`:

```bash
python3 tools/water_carve_input.py --new world/landmarks/<modulo>.bsn -o /tmp/water.json
cargo run --release -p vael_engine --example carve_water -- \
    world.data/terrain.ahgt /tmp/water.json --dry-run   # sem --dry-run grava
python3 tools/make_map.py                               # mapa do HUD
```

Gravar uma vez por corpo novo: os props e as instâncias `.vinst` dentro do
leito e das rampas do corte têm de sair à mão (a rampa cresce 1,875 × o
corte). As margens de um rio não sobem dentro da lâmina de um corpo anterior
(foz sem dique, `BANK_CONFLUENCE_FADE`).

## Oceano do Poente e lagos exteriores

O Vale Exterior ocupa a nova grelha de 8 × 8 km. A costa a oeste e as
ilhas da Lanterna e dos Juncos são parte do relevo gravado no AHGT.
O relevo seco periférico usa um recorte de 1.440² amostras do heightmap
Terrain3D em `assets/public/assets/terrain/terrain.ahgt`, a partir da
linha 2.280 e da coluna 56. O script `tools/refine_vale_terrain.py`
mistura esse relevo com o terreno de referência fora da área povoada do Vale
central e conserva a costa, as bacias, os pousos e os sítios da campanha.
O `<Ocean>` de [Águas exteriores](../world/expansion/waters.bsn) fixa a
maré em 18 m e deixa a batimetria a cargo do terreno. Dentro da grelha, só
há água onde o fundo fica abaixo da superfície. A malha e o domínio
continuam depois da última célula. A água ao longe mantém a cor mesmo
quando o fundo sai do alcance do renderer.

As balizas ficam a 3,4 km do centro do Vale. A fronteira do jogador fica
a 3,44 km e o limite do casco desconta mais 2,2 m. A embarcação perde
apenas a velocidade para fora quando toca nesse limite, pelo que ainda
pode virar, seguir ao longo das balizas e voltar à costa.

O Poente e as ilhas têm três dos seis cais. Os restantes servem os lagos
dos Freixos, do Vidro e das Sete Margens. Cada cais tem um barco a remos
em [boats.json](../data/boats.json). As ilhas têm desembarque pelo
tabuleiro. Os três lagos são bacias interiores independentes.

Para refazer o bake inicial, parte de uma cópia do AHGT e do paint de
4 km e aplica o carve dos lagos uma vez sobre o resultado:

```bash
python3 tools/expand_vale.py /tmp/base/terrain.ahgt /tmp/terrain.ahgt
python3 tools/water_carve_input.py --new world/expansion/waters.bsn -o /tmp/lagos.json
cargo run --release -p vael_engine --example carve_water -- /tmp/terrain.ahgt /tmp/lagos.json
blender -b -P tools/model_scenery.py
python3 tools/populate_expansion.py /tmp/terrain.ahgt --boats /tmp/novos-barcos.json
```

Depois de gravar o AHGT e o paint em `world.data/`, gera o mapa com
`python3 tools/make_map.py --size 1600`. Se as margens mudaram, sincroniza
as posições e as amarras dos seis barcos com o JSON gerado.

## Travessia das Vigílias

No módulo `world/landmarks/lagos-das-vigilias.bsn`, cada lago e a ilha têm
uma semente de contorno própria. `axis` orienta o alongamento em graus,
`stretch` ajusta a sua amplitude e `lobes` doseia as enseadas. O relevo
gravado no `terrain.ahgt` e a malha da água usam o mesmo contorno.

O barqueiro espera em (636, −519), no Ermo Rubro, junto ao maior dos dois lagos.
Em **O nome sob a água**, empresta o barco para visitar a ilha, ler a inscrição
no altar e regressar à mesma margem. A torre partida vê-se da praia.

A barra **Fôlego** é partilhada pela corrida, pelos golpes e pela natação.
Parar em água funda continua a gastar fôlego. Em terra, caminhar ou descansar
permite recuperá-lo, tal como seguir a bordo. Um nadador exausto regressa à
última margem segura ou ao barco de onde saltou. O save conserva o fôlego e o
ponto de socorro. O teste `island_crossing` mede o percurso no terreno do jogo.

## Margens e vida aquática

Nos lagos, pequenas frentes de espuma avançam pelas curvas de profundidade
da margem. Nos rios, a relação entre a velocidade da corrente e a profundidade
determina onde aparecem cristas curtas sobre os baixios. A absorção de cor
acompanha a distância percorrida pela luz entre a superfície e o fundo.

No lago do barco, no rio e no lago dos picos há trutas. Sobem para se alimentar,
deixam anéis ao romper a superfície e voltam a mergulhar quando o jogador se
aproxima. Os percursos consultam o mesmo leito e a mesma corrente do barco.

Tuning: `ROWBOAT_MASS` vs `HULL_HALF`/`displacement_factor` definem o calado
(agora ~7 cm). Mudar a massa sem mudar o casco muda o calado, o bordo livre e a
estabilidade; validar com os testes de `water_buoyancy`.

## Gates

```bash
# a crate (o `--no-default-features` desliga o adapter avian)
cd ../bevy_extensions && cargo test -p bevy_ext_water
cargo check -p bevy_ext_water --no-default-features
# carve e adaptadores do terreno
cargo test -p bevy_ext_terrain_runtime --lib -- water
# Vael: solver + domínio + malhas + flutuação + esteira de barco + obstáculos avian
cd ../Vael && cargo test --test water_overhaul
vael-tools analyze world.bsn
```

QA vivo: um barco no lago (-80, 104) e outro no rio (4, 215). Teleporta o herói
para a beira, **E** para entrar, **W** para remar; a esteira deve aparecer no
solver e o casco deve subir/descer com o swell. `vael-tools debug wait-ready` antes de
capturar.

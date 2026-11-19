# Chão que responde ao clima

O céu (`bevy_elements`) já calcula neve, rastos e um mapa de sombra de nuvens.
O terreno do Vael é o `.ahgt`, por isso os plugins de terreno, hidrologia e
cobertura do crate continuam desligados. O que entra no chão é um adaptador:
a simulação do manto vive em `bevy_ext_terrain_runtime::ground`, os rastos e
a sombra usam os mapas que o crate já publica para um chão alheio.

Três gates, todos ligados por omissão. `0` / `false` / `off` / `no` desliga.

| Variável | O que desliga |
|---|---|
| `VAEL_SNOW` | o manto (grelha, shader, tinta da relva, neve nas copas) |
| `VAEL_TRAILS` | `TrailsPlugin`, solas e o sulco no terreno |
| `VAEL_SURFACE` | `WeatheredPlugin`, a sombra no terreno e a troca dos props |

Com o flag a 0 o fragmento do chunk não lê o mapa. O albedo fica o de hoje.

## Manto

Janela de 256×256 células de 2 m (~512 m) centrada no herói, integrada no CPU
a cada frame em que há algo para fazer. O jogo escreve `SnowClimate` e
`SnowFocus` (`src/ground.rs`); `simulate_snow` acumula, derrete e empurra.

- A queda vem de `Weather`: mm/h de água vezes a fração de neve. 1 mm de água
  vira 1 cm de neve fresca (densidade 100 kg/m³). 5 cm de profundidade leem
  como cobertura total.
- Derrete com a temperatura acima de zero (grau-dia, mais depressa ao sol) e
  com a chuva.
- Encosta entre 35° e 55° guarda cada vez menos; acima de 55° larga o pack.
- A temperatura da célula é a do herói menos 6,5 °C/km de diferença de cota.
  Um pico 800 m acima do vale fica branco enquanto o vale derrete.
- Vento acima de 5 m/s passa uma fração do pack à célula a favor do vento.

A cobertura (R8, 0..1) entra no material único de chunk (`params.snow`). O
fragmento mistura o albedo `0.80, 0.83, 0.88` pela cobertura e pela normal:
parede vertical quase não branqueia. O passo ao vivo multiplica a queda por
8: a 1,5 mm/h físicos e relógio a 72× o filme só aparecia ao fim de minutos.
As células pavimentadas retêm até 1,2 cm de neve, a berma até 2,5 cm e
as células totalmente abrigadas ficam limpas. O material do pavimento
recebe 18% da exposição exterior. A cobertura dos telhados acompanha a
profundidade ao ar livre, inclusive quando o herói passa sob um beiral.

O mundo arranca a 21 de junho (verão). Sem `VAEL_START_WEATHER=Snow` (ou
`Blizzard`) não neva. O cartão do relógio mostra a data e a estação
(`21 jun · Verão`) e, quando cai, `· neve` ou `· chuva`.

`FloraSeason.snow` segue a cobertura junto ao herói. A relva não amostra
textura (`bevy_grass` tem uma tinta por campo): cada bioma branqueia pela
média do manto nos pontos do polígono que caem na janela.

Forçar neve: `VAEL_START_WEATHER=Snow` ou `Blizzard`. Chuva (`Rain`) derrete.

## Rastos

`VAEL_TRAILS` liga o `TrailsPlugin` e instala um `TrailGround` cuja altura é
`TerrainRuntime::sample`. Sem esse resource o plugin não corre.

O herói e os NPCs a menos de 80 m ganham uma sola (`Imprint`, elipse 10×26 cm)
encostada ao chão. No ar ou sobre um leito de água (o corpo a mais de 1,8 m
do chão) a sola levanta e deixa de marcar. O crate fica com os 64 mais
próximos.

O campo `TrailState` (`rgba16float`: altura, ocupação, derivadas) entra no
chunk. Dentro da janela:

- na neve, o sulco mostra o chão por baixo;
- com o chão molhado (`SurfaceWeathering.wetness`), o sulco escurece e fica
  mais liso.

A relva continua a achatar-se com o trampling que já tinha. O shader dela não
muda.

## Sombra de nuvem e props

`CloudShadows` (1024², 8 km, transmitância no R) entra no chunk com o UV do
crate: `(P.xz − luz.xz · P.y / luz.y − origin) / extent`. Fora do mapa a
transmitância é 1. O fragmento multiplica a cor já iluminada por
`(0.25 + 0.75 T) / (0.25 + 0.75 T̄)` — a luz direta já leva a média, isto só
pinta a mancha. A direção da luz é `CloudShadows::generation_direction`
(o sol baixo é levantado a ~6°).

Props exteriores: cada `StandardMaterial` de uma malha com `WorldNode` /
`WorldOrigin` no ancestral, que não seja o herói, um NPC animado ou uma
árvore, passa a `WeatheredMaterial` (uma vez por `AssetId`). O buffer do
crate trata molhado, listas de chuva, neve nas faces de cima e a mesma sombra.
`scenes/interiors` não entra. Não há abrigo por instância: o material é
partilhado, como o molhado das árvores.

## O que não entra

O `TerrainPlugin`, a hidrologia, a água e o `GroundCoverPlugin` do
`bevy_elements` continuam desligados. O calendário trata da folhagem e das flores. A cobertura de neve das copas vem do manto local.

## Verificar

```bash
cargo test -p bevy_ext_terrain_runtime --lib ground
cargo test -p bevy_ext_grass --lib zero_snow
cargo test -p vael --lib ground
```

No jogo (`--scene main --entry dev`):

```bash
VAEL_START_WEATHER=Snow vael-tools run --scene main --entry dev
VAEL_START_WEATHER=Rain vael-tools run --scene main --entry dev
VAEL_START_WEATHER=Overcast vael-tools run --scene main --entry dev
```

`VAEL_SNOW=0`, `VAEL_TRAILS=0` e `VAEL_SURFACE=0` voltam ao chão sem manto,
sem sulco e sem mancha de nuvem.

## Rochas mineráveis

Os cinco prefabs `rock-mossy-breakable-solid-stone*` marcam as estações com
`prop-outcrop="stone"`. O nome estável da instância escolhe uma de 24 formas
de ardósia, granito ou arenito. A pose, a resistência e o rendimento vêm do
mapa. As formações têm um, dois ou três blocos encaixados. Cada bloco tem o seu próprio casco convexo. A malha e a textura com veios e fissuras naturais são partilhadas por variante.

O limo avança em manchas com a humidade, recua devagar em tempo seco e fica
dormente a 1 °C ou menos. O save guarda a cobertura em `quarry.moss`.
A neve acompanha a profundidade do manto e a inclinação das faces em espaço
de mundo. A borda é recortada na própria malha, inclusive em rochas inclinadas.

Cerca de 14% das estações têm uma drusa com três a seis cristais. A semente da estação define a face onde a drusa nasce, a cor, a inclinação e a altura das pontas. Os cristais crescem a partir dessa face com o calendário do mundo. A data de origem fica no save em `quarry.crystal_epoch`.

Com o aviso **Minerar** ativo, um toque em `J` ou no botão esquerdo dá um golpe. Manter o botão repete a picaretada na mesma jazida a cada 1,04 s. O herói vira-se para a face mais próxima e encurta a distância com um passo. A picareta usa o clip `mine`. O jogo varre a cabeça da ferramenta entre
duas poses animadas da mão. Ao tocar o colisor, o braço limita o avanço,
o clip sustenta o impacto por 42 ms e recua. Um golpe no ar não gasta
a resistência da jazida.
Golpes intermédios soltam fragmentos e abrem fissuras ramificadas no ponto de contacto. Voltar a atingir a mesma zona aprofunda e estende as fissuras existentes. O último golpe
entrega a quantidade de pedra definida na estação quando há espaço na bolsa.

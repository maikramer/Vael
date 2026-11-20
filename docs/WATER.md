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
| `body` | `WaterBody` (lago/rio), `sample_domain`, contorno orgânico |
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

Os dez lagos têm perfis próprios de margem no BSN. O rio em
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

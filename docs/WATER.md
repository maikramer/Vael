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
- [`WaterBuoyancyPlugin`]: corre `step_floating_bodies` em `Update`.

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

- `data/boats.json` lista `{ name, x, z, yaw }`; nascem só onde
  `water_surface_at` responde. São entidades de runtime (como as criaturas), não
  nós do `.bsn`; um `SceneLoad` despawna-os e a cena seguinte volta a semeá-los.
- Casco, bancos e remos são **malha procedural** (`rowboat_hull_mesh`,
  `rowboat_oar_mesh`); materiais mate com `cull_mode: None`.
- `Boat` = aceleração, reverse, velocidade máxima, sprint, taxa de guinada e
  fase dos remos. `row_thrust`/`row_yaw` são puros e testados.
- **E** (acção `interact`) a menos de `BOARD_RADIUS` entra; a bordo, **E** sai.
  O `KinematicCharacterController` do herói é guardado em `RidingBoat` e
  reposto na saída.
- A bordo: **W/S** rema à frente/atrás, **A/D** dá leme, **Shift** sprinta. O
  herói é colocado em `SEAT_LOCAL` todos os frames e o `Player` da engine
  espelha a velocidade do casco — a câmara de terceiros segue o barco sem código
  novo, e `player_movement` ignora o corpo enquanto `RidingBoat` existe.

Tuning: `ROWBOAT_MASS` vs `HULL_HALF`/`displacement_factor` definem o calado
(agora ~7 cm). Mudar a massa sem mudar o casco muda o calado, o bordo livre e a
estabilidade; validar com os testes de `water_buoyancy`.

## Gates

```bash
# a crate (23 testes; o `--no-default-features` desliga o adapter avian)
cd ../bevy_extensions && cargo test -p bevy_ext_water
cargo check -p bevy_ext_water --no-default-features
# carve + shims do terreno no monólito (42 testes de água)
cd crates/bevy_ext_viber_engine && cargo test --lib -- terrain::water
# Vael: solver + domínio + malhas + flutuação + esteira de barco + obstáculos avian
cd ../../../Vael && cargo test --test water_overhaul
vael-tools analyze world.bsn
```

QA vivo: um barco no lago (-80, 104) e outro no rio (4, 215). Teleporta o herói
para a beira, **E** para entrar, **W** para remar; a esteira deve aparecer no
solver e o casco deve subir/descer com o swell. `vael-tools debug wait-ready` antes de
capturar.

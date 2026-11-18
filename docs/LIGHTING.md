# LIGHTING.md — ambiente vivo (`bevy_elements`)

O Vael já não usa o `<Sky>`/`<DayCycle>`/`<Weather>` da engine para o céu.
O ambiente é do crate [`bevy_elements`](https://crates.io/crates/bevy_elements)
0.8 (`src/environment.rs`): relógio astronómico, sol/lua/estrelas, clima,
nuvens volumétricas, chuva/neve, névoa e relâmpagos. O terreno `.ahgt`, a
água, a vegetação e os biomas continuam a ser do Vael/engine — só o **céu e o
tempo** mudaram de dono.

```xml
<!-- world/environment.xml: já não há <Sky>, <DayCycle>, <Weather> nem a
     DirectionalLight/AmbientLight autorais. O bevy_elements spawna o Sol e a
     Lua e conduz a atmosfera. -->
```

## Relógio

- `WorldClock` arranca em **2026-06-21 08:00**, latitude **50°N**
  (`GeoLocation`), a 72 s de jogo por segundo real: o dia continua a durar
  ~20 min reais, como o antigo `minutes-per-real-second="1.2"`.
- O sol de meio-dia fica a ~63° (o antigo `max-sun-elevation="62"`), o pôr do
  sol ~20:04 e o nascer ~03:56 — o `is_night` das 20:00–05:00 do Lua cai quase
  em cima do crepúsculo, sem mudar as regras.
- **Não há saltos de hora em jogo**: o relógio avança contínuo. Um salto
  (`env_set_hour` de QA) faz o crate realocar o probe do céu; com o pool do
  Vael isso chega a estourar os 6 GB da GPU de desenvolvimento. Para QA usa-se
  `VAEL_START_HOUR` (arranca já na hora) em vez de saltar.

## Sombras

O relógio é contínuo (sem saltos: um salto realoca o probe do céu), por isso
o sol move-se ~0.3°/s. O `bevy_elements` desenha o mapa de sombras em **todos
os frames** (sombras ligadas no Sol da câmara do crate). O antigo
`VaelShadowPlugin` (`src/lighting.rs`) desenhava uma cadência a 15 Hz atrás de
`VAEL_SHADOW_HZ`; o modo (> 0) estava **quebrado com o Bevy 0.20** (o mapa de
profundidade não sobrevive aos frames saltados: chão sem sombra e acne) e o
default era inerte — o plugin foi **removido em 2026-11-13** (F0). Não repor a
cadência sem provar que o mapa persiste.

## Exposição

A exposição é do crate (`ElementsCamera { auto_exposure: true }`): EV100
calculado da iluminância do sol/lua/nuvens. A exposição automática da engine
está desligada em `config.yaml` (`postfx.disabled: auto-exposure`) — dois
donos do mesmo `Exposure` davam flicker e a curva nocturna antiga branqueava
o céu físico ao meio-dia.

**Um só dono do EV:** a câmara do crate leva também a marca
`viber::postfx::ExternalExposure`. Sem ela, o `drive_postfx` da engine
continuava a escrever o EV base (`BASE_EV100`, EV100 ~10,2) por cima do EV
físico do crate (~13,3 ao meio-dia) e o céu saía branco/cinza. O
`drive_postfx` continua a conduzir o bloom, o grading e a vinheta; só não
toca no `Exposure` dessas câmaras. O kick de juice (`viber.punch` →
`PostFxState::kick`) é reaplicado pelo Vael, depois do `drive_postfx`
(PostUpdate), e desfeito antes do `auto_exposure` do crate (PreUpdate) para
não realimentar a adaptação. `env_info()` expõe `camera_ev100` e
`elements_ev100` para QA.

## O que a engine continua a conduzir

O `AtmosphereState` da engine (paleta de névoa, grading, `exposure_scale` e a
direção do sol no terreno) é reescrito por `bridge_atmosphere` a partir do
`SunState` do crate. A **névoa de distância por bioma** continua a ser da
engine (`biome_fog_system`); a névoa do crate fica desligada na câmara
(`distance_fog: false`). O HUD (`UiData.clock/weather_rain`) e o
`viber.vael.clock_*` do Lua são alimentados pelo relógio/clima do crate.

## Clima jogável

O `Weather` do crate alimenta o `WeatherState` da engine (`bridge_weather`,
`cycle:false`), com a chuva já multiplicada pelo `rain-scale` do bioma. Isso
reativa o chão molhado do terreno (`terrain_rain_wetness`), o loop de áudio
da chuva, o vento/molhado das árvores (`VaelFloraPlugin::drive_weather`), as
regras de fogo/visão e o `water_ssr` (se ligado).

- **Um só dono visual:** o crate desenha as gotas; `EngineRainVisuals(false)`
  desliga o emissor de partículas CPU da engine. As **ondulações no terreno
  real** (`rain_ripple_spawner`) e o **áudio** continuam na engine.
- **Trovão:** `bridge_thunder` lê `Thunder` (com `loudness` já atenuado pela
  distância) e emite `SfxLoudEvent` (`SfxClip::Thunder`).
- **Visibilidade → névoa:** `apply_weather_visibility` (PostUpdate) sobe a
  densidade da `DistanceFog` da engine para 2 % de contraste à
  `visibility_m` do clima, sem abrir a névoa do bioma. O preset `Fog`
  (300 m) passa a fechar o horizonte; a 40 km nada muda.
- **Lua:** `scripts/lib/environment.lua` embrulha `env_info()`
  (`rain_effective`, `wind`, `wetness`, `snow`, `cloud_shadow`, …).

## Interiores

Dentro de uma bolsa de interior (`InteriorLighting.active`), o crate não
pode iluminar como céu aberto:

- os `DirectionalLight` (Sol/Lua do crate) vão a zero **depois** do
  `drive_lights`;
- o `AmbientLight` da câmara copia o `GlobalAmbientLight` autoral da sala;
- a **atmosfera do crate sai da câmara** (`ElementsCamera.atmosphere=false`):
  sem ela o crate não desenha a camada de nuvens nessa vista (era o véu
  branco no celeiro em tempo nublado); o `GeneratedEnvironmentMapLight` do
  sky probe fica a 0;
- a exposição automática do crate é desligada e a câmara fica em
  `Exposure::INDOOR` (EV100 7).

Sem isto o celeiro saía branco (nuvens/céu do crate por cima da sala). Os
interiores continuam a viver das `PointLight` de teto e do ambiente autoral.
Um `PrecipitationBlocker` cobre o volume da câmara (`interior_precipitation`)
para as gotas do crate não caírem dentro da sala; o trovão também é
silenciado.

## Luzes práticas (fogueiras e lanternas)

O mundo autora ~450 `PointLight`, todas reais e estáticas. À noite o
`torch_pool_drive` da engine compensa a exposição com um ganho global (×8 nas
quentes, ×64 nas que têm `shadows="true"`), uma lavagem quente uniforme. O
`src/lights.rs` troca as **luzes de fogo** por luzes práticas do
[`bevy_lights`](https://crates.io/crates/bevy_lights) 0.5 (`=0.5.0`, pin
exato):

- **Pool orçamentado** (`LightPoolPlugin`): um máximo de luzes reais
  (`VAEL_POOL_SIZE`, default 24) perto do herói, escolhidas por iluminância
  com histerese e cross-fade. O resto fica virtual — a chama/partícula é o
  que se vê.
- **Adoção**: a `PointLight` quente mais próxima de um emissor
  `preset: "fire"` (até 4 m) vira `PoolCandidate` e **perde o `PointLight`**
  (um dono só). As quentes de preenchimento (janelas, brilho da vila) e os
  cristais frios continuam autorais — senão, com `reach` de 60 m, o fim da
  rua apagava.
- **Fogos sem luz**: os braseiros de `clutter.bsn` e parte da cidadela não
  têm `PointLight`; o `fill_dark_fires` cria um candidato com a intensidade
  do modelo do crate (`FIRE_EDGE_LUX` à distância de alcance do tipo), em vez
  de mais um número afinado à mão.
- **Flicker**: o `FlickerPlugin` faz cada luz respirar (fase determinística
  por posição; profundidade e ritmo pelo tipo de fogo). Ver **Um só dono da
  intensidade** abaixo.
- **Chamas**: `FlamePlugin` desenha uma chama billboard aditiva sobre cada
  emissor, com o mesmo `phase`/`diameter`/`depth` da luz (`VAEL_FLAME_BILLBOARDS`).
- **Migração das partículas**: os **248** emissores `preset: "fire"` do mundo
  deixam de correr no sistema CPU da engine. O `lights.rs` remove o
  `AmbientFx` e o bundle que o `ambient_fx_manager` montou
  (`ParticleEmitter`+`Mesh3d`) — a entidade fica (nome, hierarquia,
  `Transform`, `Visibility`) e ganha `VaelFire` + `FlameSite`/`FuelBed`/brasas/
  fumo. Os restantes `ParticleSpec` (poeira, folhas, fumo, pirilampos, neve…)
  não são tocados. `VAEL_FIRE_MIGRATE=0` mantém as duas versões para o A/B.
  O `.bsn` continua a autorar `<ParticleSystem preset="fire">` — a migração é
  de **runtime**, por isso o Jackdaw e o `analyze` não mudam.
- **Sombras**: `max_shadowed` do pool (`VAEL_POOL_SHADOWS`, default 2) só de
  noite. O gotcha das cube shadows continua válido: cada luz custa 6 vistas e
  pisca no chão — o default é conservador e mede-se antes de subir.

### Um só dono da intensidade

O `Flicker` escreve `PointLight.intensity` e o `torch_pool_drive` escreve o
mesmo campo. A regra: **cada luz de fogo tem um dono só.** As candidatas
levam o valor autoral em `VaelPractical::base` e o `lights.rs` reescreve
`candidate.intensity` por frame com as funções públicas da engine
(`torch_pool_gain`/`warm_fill_gain`), preservando a calibração ×64/×8 por
luz. As luzes que o pool spawna levam `viber::postfx::NightGainExempt`: sem
o marcador o drive da engine capturava a base 0 delas (nascem a 0) e
escrevia 0 para sempre — o mesmo bug que já tinha sido resolvido à parte na
`TravellerLantern`.

### Reverter

`VAEL_PRACTICAL_LIGHTS=0` desliga o módulo inteiro e devolve exatamente o
comportamento anterior (luzes autorais + ganho global). É o interruptor de
regressão; o resto da tabela está no topo de `src/lights.rs`
(`VAEL_POOL_SIZE`, `VAEL_POOL_SHADOWS`, `VAEL_FIRE_FILL`,
`VAEL_FLAME_BILLBOARDS`, `VAEL_FIRE_MIGRATE`).

### QA

```bash
# A/B: mesma noite, com e sem o pool. O `prof --tab frames` dá p95/hitches;
# o contador `shadow_lights` confirma o orçamento de sombras.
VAEL_SCENARIO=night_lanterns/night cargo run --release -- \
  world.bsn --scene main --entry dev --bridge 15706
VAEL_PRACTICAL_LIGHTS=0 VAEL_SCENARIO=night_lanterns/night cargo run ...
vael-tools debug --port 15706 wait-ready
vael-tools debug --port 15706 prof --tab frames
vael-tools debug --port 15706 screenshot -o /tmp/noite.png

# Visual: rua das lanternas, acampamento dos contrabandistas, tochas de
# caverna e lareira de interior, com `VAEL_FLAME_BILLBOARDS=0/1`.
```

Cenários em `qa/scenarios/night_lanterns/night.json` e
`night_campfire/night.json` (hora 22, céu limpo, herói teleportado para o
sítio). O veredicto numérico de flicker (luma por frame) é o `burst` do
runtime — `vael-tools/burst` no BRP, como o resto do QA temporal.

### Limites conhecidos

- O crate é muito novo (0.5.0, 2 semanas, 324 downloads) e o README
  declara-o *"Vibe Coded"* — sem revisão humana linha a linha. O pin exato e
  o gate existem por isso; todo o contacto com ele vive em `src/lights.rs`.
- As luzes adotadas deixam de contar em `viber.debug.lights`/no profiler como
  `PointLight`, e um re-spawn do `live_edit` volta a ser adotado no frame
  seguinte (`Added<PointLight>`).
- O template do pool é único: cor por candidato (`PoolCandidate::color`), mas
  `range`/`radius` são os do template (20 m/0,45 m). Os cristais frios, com
  `radius: 5`, ficam de fora.
- Só as luzes de fogo entram no pool; as de preenchimento continuam no ganho
  global da engine.
- Migrar as partículas perde o **look CPU** das chamas antigas (gradiente de
  cor, jitter) a favor da chama billboard + brasas/fumo do crate. O A/B é
  `VAEL_FIRE_MIGRATE=0` (as duas juntas) contra `1` (só chama); com
  `VAEL_FLAME_BILLBOARDS=0` a migração é forçada a 0 para o fogo não
  desaparecer.
- O `BurstPool` da engine não recebe de volta as malhas dos emissores que
  já estavam montados quando a migração corre (≤ orçamento de 8 por scan;
  quads pequenos). Só acontece no arranque/carregamento de cena.

## Performance

- Sombras em todos os frames (o `bevy_elements` desenha-as; o modo de
  cadência foi **removido em 2026-11-13** — ver **Sombras**). Medição antiga
  do modo a 15 Hz a 1280×720: ≈ 31 ms/frame vs ≈ 104 ms em todos os frames —
  feita antes da mudança de câmara, só ordem de grandeza.
- `VAEL_ELEMENTS_QUALITY=low|medium|high|ultra` (default **medium**). O frame
  do Vael é CPU-bound: `low` não mudou o frame de forma mensurável; `medium`
  fica por causa da chuva (60k partículas vs 20k) e das nuvens.
- O probe do céu realoca num **salto de hora**; em jogo o relógio é contínuo.
  Para QA, `VAEL_START_HOUR` arranca já na hora (sem salto).

## QA

```bash
# Dia, crepúsculo, noite e chuva sem saltar o relógio. O run usa o save do
# jogo: faz backup (ou usa um slot separado) antes de comparar.
VAEL_START_HOUR=19.85 vael-tools run --scene main --entry dev
VAEL_START_WEATHER=Rain vael-tools run --scene main --entry dev

# Ao vivo:
vael-tools debug lua 'return viber.vael.env_info()'
vael-tools debug lua 'viber.vael.env_pause(true)'
vael-tools debug lua 'viber.vael.env_set_weather("Thunderstorm")'
```

`VAEL_ELEMENTS=0` desliga o crate e volta ao céu da engine.

## A/B e reverter

`VAEL_ELEMENTS=0` desliga o crate (volta ao céu da engine). Nesse modo é
preciso repor os elementos antigos no `world/environment.bsn` e no
`scenes/menu.bsn`; a exposição automática também não está lá (foi desligada
no `config.yaml`). Sem o crate, as sombras voltam a ser as da engine (o
`VaelShadowPlugin` já não existe).

## Limites conhecidos

- A sombra das nuvens cai no terreno e nos props exteriores
  ([`GROUND.md`](GROUND.md)). `VAEL_SURFACE=0` desliga essa sombra e a troca
  de materiais. O `player_cloud_shadow` de `env_info` continua a vir do
  readback do crate.
- O crate fica no local de **céu/clima**; terreno, hidrologia e água do crate
  estão desligados (`TerrainPlugin`, `WaterPlugin`, …), porque o mundo é
  `.ahgt` estático com rios/poços autorais. O manto de neve é do terreno do
  Vael, não do `GroundCoverPlugin`.
- Props exteriores usam `WeatheredMaterial` (molhado, listas de chuva, neve
  nas faces de cima). As árvores recebem vento, molhado e neve via
  `bevy_flora`. Sem abrigo por instância: o material é partilhado.
- O crate é novo (0.8.0, `Vibe Coded` no README). O gate `VAEL_ELEMENTS` e o
  A/B acima existem para o poder desligar sem reescrever o mundo.

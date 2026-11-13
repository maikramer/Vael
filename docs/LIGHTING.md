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
o sol move-se ~0.3°/s. O `VaelShadowPlugin` (`src/lighting.rs`) desenha o mapa
de sombras em **todos os frames** (`VAEL_SHADOW_HZ=0`, o default). O modo de
cadência (saltar o passe nos frames intermédios e congelar as cascatas) está
**quebrado com o Bevy 0.20**: o mapa de profundidade não sobrevive aos frames
saltados — o resultado é chão sem sombra e acne nas faces voltadas ao sol. O
mecanismo ficou atrás de `VAEL_SHADOW_HZ` (> 0) só para A/B; não voltar a
ligá-lo sem provar que o mapa persiste.

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

## Performance

- `VAEL_SHADOW_HZ` (default **0** = sombras em todos os frames). O modo de
  cadência (> 0) existe só para A/B e com o Bevy 0.20 produz sombras
  ausentes/acne (ver **Sombras**); não usar em jogo. Medição antiga do modo a
  15 Hz a 1280×720: ≈ 31 ms/frame vs ≈ 104 ms em todos os frames — foi feita
  antes da mudança de câmara e serve só de ordem de grandeza.
- `VAEL_ELEMENTS_QUALITY=low|medium|high|ultra` (default **medium**). O frame
  do Vael é CPU-bound: `low` não mudou o frame de forma mensurável; `medium`
  fica por causa da chuva (60k partículas vs 20k) e das nuvens.
- O probe do céu realoca num **salto de hora**; em jogo o relógio é contínuo.
  Para QA, `VAEL_START_HOUR` arranca já na hora (sem salto).

## QA

```bash
# Dia, crepúsculo, noite e chuva sem saltar o relógio. O run usa o save do
# jogo: faz backup (ou usa um slot separado) antes de comparar.
VAEL_START_HOUR=19.85 viber run --scene main --entry dev
VAEL_START_WEATHER=Rain viber run --scene main --entry dev

# Ao vivo:
viber debug lua 'return viber.vael.env_info()'
viber debug lua 'viber.vael.env_pause(true)'
viber debug lua 'viber.vael.env_set_weather("Thunderstorm")'
```

`VAEL_ELEMENTS=0` desliga o crate e volta ao céu da engine.

## A/B e reverter

`VAEL_ELEMENTS=0` desliga o crate (volta ao céu da engine). Nesse modo é
preciso repor os elementos antigos no `world/environment.bsn` e no
`scenes/menu.bsn`; a exposição automática também não está lá (foi desligada
no `config.yaml`). O `VaelShadowPlugin` fica inactivo sem `<DayCycle>` e sem
o crate.

## Limites conhecidos

- As sombras de nuvem do crate não caem no terreno do Vael (os materiais não
  têm `GroundExtension`/`WeatheredExtension`); o `player_cloud_shadow` de
  `env_info` já vem do readback do crate.
- O crate fica no local de **céu/clima**; terreno, hidrologia e água do crate
  estão desligados (`TerrainPlugin`, `WaterPlugin`, …), porque o mundo é
  `.ahgt` estático com rios/poços autorais.
- Props não recebem molhado/neve do crate (`WeatheredMaterial`); as árvores
  recebem-no via `bevy_flora` (`FloraWind`/`FloraWetness`), não pelo crate.
- O crate é novo (0.8.0, `Vibe Coded` no README). O gate `VAEL_ELEMENTS` e o
  A/B acima existem para o poder desligar sem reescrever o mundo.

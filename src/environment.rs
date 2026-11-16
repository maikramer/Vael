//! Ambiente vivo do Vael (`bevy_elements`): relógio astronómico, sol/lua/
//! estrelas, clima, nuvens volumétricas, chuva/neve e relâmpagos.
//!
//! O que muda de dono: o `<Sky>`, o `<DayCycle>`, o `<Weather>` e a luz
//! direcional/ambiente autorais da engine saem do `world/environment.xml`
//! (ver o comentário lá); o crate passa a conduzir a atmosfera, o céu e o
//! clima. O que NÃO muda: o terreno `.ahgt`, a água, a vegetação e os biomas
//! continuam a ser do Vael/engine — o crate só recebe uma `ElementsCamera`,
//! não o `Terrain` dele.
//!
//! A névoa de distância da engine fica ligada (é ela que dá a densidade por
//! bioma); a do crate fica desligada na câmara. Para a névoa/grading/terreno
//! seguirem o sol astronómico, [`bridge_atmosphere`] reescreve o
//! `AtmosphereState` da engine depois do `atmosphere_drive`.
//!
//! Gate: `VAEL_ELEMENTS=0` desliga tudo e deixa o céu antigo (A/B de QA).
//! Para o modo antigo também é preciso repor os elementos no
//! `world/environment.xml`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use bevy::app::PluginGroup;
use bevy::light::{
    AtmosphereEnvironmentMapLight, GeneratedEnvironmentMapLight, GlobalAmbientLight,
};
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;
use bevy_elements::{
    CivilDate, Climate, CloudShadows, DustPlugin, ElementsCamera, ElementsExposure,
    ElementsPlugins, ElementsQuality, ElementsSystems, Environment, GeoLocation, GroundCoverPlugin,
    HydrologyPlugin, PathsPlugin, PrecipitationBlocker, PrecipitationState, RocksPlugin, SunState,
    SurfaceWeathering, TerrainPlugin, Thunder, TrailsPlugin, WaterPlugin, WavesPlugin, Weather,
    WeatherDirector, WeatherPreset, WeatheredPlugin, WorldClock,
};
use bevy_ext_core::Player;
use bevy_ext_core::{EngineRainVisuals, SfxClip, SfxLoudEvent};
use mlua::{Lua, Value};
use viber::postfx::{ExternalExposure, PostFxState};
use viber::worldsys::{
    AtmosphereState, BiomeRegions, InteriorLighting, WeatherState, rain_scale_at,
};

use crate::clock::elements_rain;
use crate::settings::SceneSetting;
use crate::vault::{vael_fn, vael_table};
use bevy_ext_elements::{ElementsQualityLevel, ElementsSettings};

/// Latitude do mundo (graus, norte positivo).
///
/// Vael não tem geografia real; 50° com o solstício de junho dá um sol de
/// meio-dia a ~63° (o antigo `max-sun-elevation="62"`), nascer ~03:56 e pôr
/// ~20:04 — o `is_night` das 20:00–05:00 do Lua cai quase em cima do crepúsculo
/// astronómico.
pub const LATITUDE_DEG: f64 = 50.0;

/// Data de arranque: solstício de junho (o dia mais longo do ano).
pub const START_DATE: CivilDate = CivilDate::new(2026, 6, 21);

/// Hora de arranque (08:00), a mesma do `<DayCycle minute-of-day="480">`.
///
/// `VAEL_START_HOUR` sobrepõe-na (QA de crepúsculo/noite sem saltar o
/// relógio: um salto realoca o probe do céu e, com o pool do Vael, estoura
/// os 6 GB da GPU de desenvolvimento).
pub const START_HOUR: f64 = 8.0;

/// Hora inicial, de `VAEL_START_HOUR` se for válida.
pub fn start_hour() -> f64 {
    start_hour_or(START_HOUR)
}

/// `VAEL_START_HOUR` por cima da hora da cena (`ElementsSettings`).
pub fn start_hour_or(scene: f64) -> f64 {
    std::env::var("VAEL_START_HOUR")
        .ok()
        .and_then(|raw| raw.trim().parse::<f64>().ok())
        .filter(|hour| hour.is_finite())
        .unwrap_or(scene)
        .rem_euclid(24.0)
}

/// Clima forçado no arranque (`VAEL_START_WEATHER=Rain`, QA de chuva/trovoada).
pub fn start_weather() -> Option<WeatherPreset> {
    let name = std::env::var("VAEL_START_WEATHER").ok()?;
    weather_preset(&name)
}

fn weather_preset(name: &str) -> Option<WeatherPreset> {
    WeatherPreset::ALL
        .into_iter()
        .find(|preset| format!("{preset:?}").eq_ignore_ascii_case(name.trim()))
}

/// `VAEL_START_WEATHER` por cima do clima da cena.
fn start_weather_or(scene: &ElementsSettings) -> Option<WeatherPreset> {
    start_weather().or_else(|| scene.start_weather.preset_name().and_then(weather_preset))
}

/// As definições de ambiente da cena (`world/settings/environment.bsn`).
fn scene_settings(app: &App) -> ElementsSettings {
    app.world()
        .get_resource::<SceneSetting<ElementsSettings>>()
        .map(|setting| setting.0.clone())
        .unwrap_or_default()
}

fn quality_of(level: ElementsQualityLevel) -> ElementsQuality {
    match level {
        ElementsQualityLevel::Low => ElementsQuality::Low,
        ElementsQualityLevel::Medium => ElementsQuality::Medium,
        ElementsQualityLevel::High => ElementsQuality::High,
        ElementsQualityLevel::Ultra => ElementsQuality::Ultra,
    }
}

/// Qualidade do crate (`VAEL_ELEMENTS_QUALITY=low|medium|high|ultra`).
///
/// `Medium` por omissão: o frame do Vael é CPU-bound e a 1280×720 o `High`
/// só custa GPU (grelha de vento 64³, mais passos de nuvem/névoa). O `Low`
/// existe para GPUs mais fracas/AA de 6 GB com pouco espaço.
pub fn elements_quality() -> ElementsQuality {
    elements_quality_or(ElementsQuality::Medium)
}

/// `VAEL_ELEMENTS_QUALITY` por cima da qualidade da cena.
pub fn elements_quality_or(scene: ElementsQuality) -> ElementsQuality {
    let Ok(raw) = std::env::var("VAEL_ELEMENTS_QUALITY") else {
        return scene;
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "low" => ElementsQuality::Low,
        "high" => ElementsQuality::High,
        "ultra" => ElementsQuality::Ultra,
        _ => ElementsQuality::Medium,
    }
}

/// 1,2 minuto de jogo por segundo real = 72 s de jogo por segundo real.
/// O dia continua a durar ~20 min reais, como no `<DayCycle>` antigo.
pub const TIME_SCALE: f32 = 72.0;

/// `VAEL_ELEMENTS=0|off|false|no` volta ao ambiente da engine. Default: ligado.
pub fn elements_enabled() -> bool {
    match std::env::var("VAEL_ELEMENTS") {
        Ok(value) => !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "off" | "false" | "no"
        ),
        Err(_) => true,
    }
}

/// Ambiente do Vael: `bevy_elements` por cima da engine.
pub struct VaelEnvironmentPlugin;

impl Plugin for VaelEnvironmentPlugin {
    fn build(&self, app: &mut App) {
        if !elements_enabled() {
            info!("vael: ambiente da engine (VAEL_ELEMENTS=0)");
            return;
        }
        // Domínios do crate que dependem do terreno dele (que não usamos).
        // Ficam de fora: terreno, hidrologia, água, cobertura do chão, poeira,
        // trilhos, caminhos, ondas, materiais desgastados e pedras. O céu,
        // clima, vento, névoa, nuvens, relâmpagos e precipitação entram.
        let group = ElementsPlugins
            .build()
            .disable::<TerrainPlugin>()
            .disable::<HydrologyPlugin>()
            .disable::<WaterPlugin>()
            .disable::<GroundCoverPlugin>()
            .disable::<DustPlugin>()
            .disable::<TrailsPlugin>()
            .disable::<PathsPlugin>()
            .disable::<WavesPlugin>()
            .disable::<WeatheredPlugin>()
            .disable::<RocksPlugin>();
        // Data/hora/latitude/clima/qualidade vêm da cena (Jackdaw); os
        // `VAEL_START_*`/`VAEL_ELEMENTS_QUALITY` ganham (QA).
        let scene = scene_settings(app);
        let start_date = CivilDate::new(scene.start_year, scene.start_month, scene.start_day);
        app.insert_resource(StartWeather(start_weather_or(&scene)))
            .insert_resource(
                WorldClock::new(start_date)
                    .at_hour(start_hour_or(scene.start_hour))
                    .with_time_scale(scene.time_scale),
            )
            .insert_resource(GeoLocation {
                latitude_deg: scene.latitude_deg,
                ..default()
            })
            .insert_resource(Climate::temperate())
            // High é o default do crate (grelha de vento 64³ + passos de névoa
            // altos); o frame do Vael é CPU-bound, por isso começamos em Medium.
            .insert_resource(elements_quality_or(quality_of(scene.quality)))
            // O look físico da chuva é fino a 1280×720; o "readable" alonga e
            // engrossa os traços sem deixar de ser chuva (foi o que o crate
            // escolheu para câmaras afastadas).
            .insert_resource(bevy_elements::PrecipitationLook::readable())
            // Estado de clima da engine: é o que `terrain_rain_wetness`,
            // `rain_ambience_driver` e `drive_weather` (flora) leem. Existe
            // antes do primeiro `Update` para o loop de chuva nascer; o
            // `bridge_weather` escreve-o a partir do `Weather` do crate e
            // `cycle:false` impede o `weather_drive` de o reescrever.
            .insert_resource(WeatherState {
                wind: [1.0, 0.0],
                wind_strength: 0.0,
                clouds: 0.0,
                rain: 0.0,
                cycle: false,
            })
            // O crate desenha as gotas; a engine fica com as ondulações no
            // terreno real e o áudio (sem chuva dupla). Ver `EngineRainVisuals`.
            .insert_resource(EngineRainVisuals(false))
            .init_resource::<VaelEnvControl>()
            // O kick de exposição da engine (`viber.punch`) é reaplicado por
            // cima do EV do crate: ver `VaelExposureKick`.
            .init_resource::<VaelExposureKick>()
            .add_plugins(group)
            // Pedidos de QA entram ANTES do relógio para a hora valer neste frame.
            .add_systems(Update, apply_env_control.before(ElementsSystems::Clock))
            // Desfaz o kick do frame anterior antes do `auto_exposure` do crate
            // ler a exposição (senão o kick realimentava a adaptação).
            .add_systems(PreUpdate, undo_exposure_kick)
            // O céu sobrevive às trocas de cena (ver `persist_sky`).
            .add_observer(persist_on_add::<bevy_elements::Sun>)
            .add_observer(persist_on_add::<bevy_elements::Moon>)
            .add_observer(persist_on_add::<bevy_elements::GroundFog>)
            .add_observer(persist_on_add::<bevy::light::Atmosphere>)
            .add_systems(PostStartup, persist_sky_meshes)
            // Gravar o `environment.bsn` com o jogo a correr aplica-se já.
            .add_systems(Update, apply_scene_settings.before(ElementsSystems::Clock))
            // O clima do crate alimenta o `WeatherState` da engine.
            .add_systems(Update, bridge_weather.after(ElementsSystems::Weather))
            // O trovão chega depois da simulação (o `run_strikes` agenda-o).
            .add_systems(Update, bridge_thunder.after(ElementsSystems::Simulation))
            .add_systems(
                Update,
                collect_env_info
                    .after(ElementsSystems::Celestial)
                    .after(ElementsSystems::Weather)
                    .after(ElementsSystems::Readback)
                    .after(bridge_weather),
            )
            .add_systems(
                Startup,
                (
                    install_env_api.after(crate::combat::install_combat_api),
                    force_start_weather,
                ),
            )
            .add_systems(
                Update,
                (
                    attach_elements_camera,
                    bridge_atmosphere,
                    interior_elements_lighting,
                    interior_precipitation,
                )
                    // O `SunState` do crate é escrito no set `Celestial`; o
                    // `AtmosphereState` da engine no `atmosphere_drive`.
                    .after(ElementsSystems::Celestial)
                    .after(viber::worldsys::atmosphere_drive)
                    .after(viber::worldsys::interior_lighting_drive)
                    .after(viber::worldsys::interior_lighting_apply),
            )
            // Piso de exposição nocturna (ver `night_exposure_floor`).
            .add_systems(
                Update,
                night_exposure_floor
                    .after(ElementsSystems::Celestial)
                    .before(collect_env_info),
            )
            // Cascatas do sol pelo preset do jogo, por cima das do crate.
            .add_systems(Update, fit_vael_sun_cascades.after(ElementsSystems::Celestial))
            // A névoa da engine é inserida por `biome_fog_system` (Update,
            // via Commands); este passe corre depois de os comandos assentarem.
            .add_systems(PostUpdate, (apply_weather_visibility, apply_exposure_kick))
            .add_systems(
                Update,
                // Depois do `collect_ui_data` (que só escreve com `<DayCycle>`) e
                // antes do `UiSet::Bind`, para o HUD ler a hora NO MESMO frame.
                bridge_ui_clock
                    .after(ElementsSystems::Clock)
                    .after(viber::ui::collect::collect_ui_data)
                    .before(viber::ui::UiSet::Bind),
            );
        info!(
            hour = start_hour_or(scene.start_hour),
            latitude = scene.latitude_deg,
            time_scale = scene.time_scale,
            weather = ?start_weather_or(&scene),
            "vael: ambiente `bevy_elements` ligado"
        );
    }
}

/// As entidades de céu do `bevy_elements` (sol, lua, atmosfera, névoa,
/// estrelas, disco da lua) são raízes que o crate cria UMA vez. Sem
/// `Persistent`, o `unload_scene` de uma troca de cena (menu → main,
/// interiores) apagava-as e o crate não as recria (a atmosfera tem um
/// "já spawnei") — depois de "Continuar" o céu ficava preto.
fn persist_on_add<C: Component>(add: On<Add<C>>, mut commands: Commands) {
    if let Ok(mut entity) = commands.get_entity(add.entity) {
        entity.insert(viber::scene_loader::Persistent);
    }
}

/// Estrelas e disco da lua: marcadores privados do crate, nascem no
/// `Startup` — reconhecem-se pelo nome.
fn persist_sky_meshes(mut commands: Commands, named: Query<(Entity, &Name), Without<ChildOf>>) {
    for (entity, name) in &named {
        if matches!(name.as_str(), "Stars" | "Moon disc") {
            commands.entity(entity).insert(viber::scene_loader::Persistent);
        }
    }
}

/// EV100 mínimo da câmara de jogo (noite). O `auto_exposure` do crate é
/// físico: ao luar mede EV ≈ −5 e multiplica a imagem ~2¹⁴ face ao dia. As
/// luzes práticas, as chamas e os materiais do jogo estão calibrados para a
/// exposição antiga (EV ~7–10) — a noite inteira saía branca/verde estourada
/// (visto 2026-11-16 às 23:00 na vila, com e sem bloom). A lanterna do
/// viajante (600 klm a 2,2 m do herói) e as práticas foram afinadas para a
/// câmara a EV ~9,7: abaixo de ~8 o herói brilhava e o frame lavava. A 8 o
/// chão ao luar fica escuro e as lanternas fazem a noite.
/// `VAEL_NIGHT_EV_FLOOR` afina (QA).
const NIGHT_EV_FLOOR: f32 = 8.0;

fn night_ev_floor() -> f32 {
    static FLOOR: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *FLOOR.get_or_init(|| {
        std::env::var("VAEL_NIGHT_EV_FLOOR")
            .ok()
            .and_then(|raw| raw.trim().parse::<f32>().ok())
            .filter(|v| v.is_finite())
            .unwrap_or(NIGHT_EV_FLOOR)
    })
}

/// Aplica [`NIGHT_EV_FLOOR`] às câmaras que o crate expõe sozinho (os
/// interiores desligam o `auto_exposure` e fixam o seu EV) e republica o
/// valor real no `ElementsExposure`, que é o que o resto lê para equilibrar.
fn night_exposure_floor(
    mut cameras: Query<(&ElementsCamera, &mut bevy::camera::Exposure)>,
    mut published: ResMut<ElementsExposure>,
) {
    let floor = night_ev_floor();
    for (camera, mut exposure) in &mut cameras {
        if camera.auto_exposure && exposure.ev100 < floor {
            exposure.ev100 = floor;
            published.ev100 = floor;
            published.exposure = bevy::camera::Exposure { ev100: floor }.exposure();
        }
    }
}

/// Pedidos de QA (`viber.vael.env_*`) para o ambiente: hora, clima e pausa.
#[derive(Debug, Default)]
pub struct EnvControl {
    set_hour: Option<f64>,
    set_weather: Option<String>,
    set_paused: Option<bool>,
    info: EnvInfo,
}

/// Leitura do ambiente para o QA (`viber.vael.env_info()`).
#[derive(Debug, Clone, Copy, Default)]
pub struct EnvInfo {
    pub hour: f64,
    pub sun_altitude_deg: f32,
    pub moon_altitude_deg: f32,
    pub cloud_cover: f32,
    pub rain: f32,
    /// Chuva já com o `rain-scale` do bioma (a que a engine consome).
    pub rain_effective: f32,
    pub temperature_c: f32,
    pub visibility_m: f32,
    pub wind_speed_ms: f32,
    pub wind_dir_x: f32,
    pub wind_dir_z: f32,
    pub gustiness: f32,
    pub storminess: f32,
    pub wetness: f32,
    pub snow_cover: f32,
    pub snow_depth_m: f32,
    pub cloud_transmittance: f32,
    /// Transmitância das nuvens no herói (readback ~4 Hz); `None` antes da
    /// primeira leitura.
    pub player_cloud_shadow: Option<f32>,
    /// Valores vivos da câmara `ElementsCamera` (QA de exposição/ambiente).
    pub camera_ev100: f32,
    /// EV100 que o `bevy_elements` publicou (dono da exposição; deve ser o
    /// mesmo que a câmara tem, salvo o kick de `viber.punch`).
    pub elements_ev100: f32,
    pub camera_ambient: f32,
    pub camera_env_intensity: f32,
    pub camera_fog_density: f32,
    pub camera_volumetric: bool,
    pub precip_rain: u32,
    pub precip_snow: u32,
    pub precip_dust: u32,
    pub interior: bool,
}

/// Handle partilhado do QA: o Lua escreve pedidos, os sistemas Bevy leem.
#[derive(Resource, Clone, Default)]
pub struct VaelEnvControl(pub Arc<Mutex<EnvControl>>);

impl VaelEnvControl {
    fn lock(&self) -> std::sync::MutexGuard<'_, EnvControl> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Dá `ElementsCamera` à câmara **do jogo** e a mais nenhuma.
///
/// O carregamento do mundo cria câmaras transitórias com `OrbitCamera` (22
/// numa única frame, em 2026-11-12): dar `ElementsCamera` a todas fazia o
/// `follow_camera` do crate seguir uma delas e o `mark_view` das nuvens
/// inserir `CloudView` numa entidade já despawnada — o handler de erros do
/// Bevy abortava o processo no boot.
///
/// Política: a câmara tem de ser **vista durante dois frames seguidos**
/// (as transitórias vivem um frame) e depois fica **pegajosa** — mantém-se
/// enquanto existir; só se escolhe outra quando a marcada morre (troca de
/// cena). A marca é removida de qualquer outra, para o crate seguir
/// exatamente uma.
///
/// `auto_exposure: true`: a exposição passa a ser a do crate, calculada da
/// iluminância (sol/lua/nuvens). A exposição automática da engine é desligada
/// no `config.yaml` (`postfx.disabled: auto-exposure`): dois donos do mesmo
/// `Exposure` davam flicker, e a curva nocturna da engine (feita para o céu
/// antigo) branqueava o céu físico ao meio-dia. A marca [`ExternalExposure`]
/// fecha a porta que faltava: o `drive_postfx` da engine escrevia o EV base
/// (`BASE_EV100`, EV100 ~10,2) por cima do EV físico (~13,3).
///
/// `distance_fog`/`volumetric_fog` desligados: a névoa de distância por bioma
/// é da engine (`biome_fog_system`) e os volumétricos já estão cortados no
/// `config.yaml`.
fn attach_elements_camera(
    mut commands: Commands,
    cameras: Query<(Entity, &Camera), (With<Camera3d>, With<viber::recipes::spawn::OrbitCamera>)>,
    marked: Query<Entity, With<ElementsCamera>>,
    mut candidate: Local<Option<Entity>>,
    mut streak: Local<u32>,
    mut reported: Local<u32>,
) {
    // Já há uma câmara marcada e viva? Fica (pegajosa) e limpa as outras.
    let current = marked
        .iter()
        .next()
        .filter(|entity| cameras.contains(*entity));
    // Candidata: a ativa de menor `Camera::order`.
    let pick = cameras
        .iter()
        .filter(|(_, camera)| camera.is_active)
        .min_by_key(|(_, camera)| camera.order)
        .map(|(entity, _)| entity);
    match (current, pick) {
        (Some(_), _) => {}
        (None, Some(pick)) => {
            if *candidate == Some(pick) {
                *streak += 1;
            } else {
                *candidate = Some(pick);
                *streak = 1;
            }
        }
        (None, None) => {
            *candidate = None;
            *streak = 0;
        }
    }

    let chosen = current.or_else(|| {
        // Só depois de dois frames seguidos: as câmaras do carregamento
        // vivem um frame e nunca chegam a ser marcadas.
        (*streak >= 2).then_some(*candidate).flatten()
    });
    let Some(chosen) = chosen else {
        return;
    };
    for entity in &marked {
        if entity != chosen {
            commands.entity(entity).remove::<ElementsCamera>();
        }
    }
    if current.is_none() {
        commands.entity(chosen).insert((
            ElementsCamera {
                auto_exposure: true,
                atmosphere: true,
                distance_fog: false,
                volumetric_fog: false,
                ambient: true,
                msaa_off: true,
            },
            // O EV é do crate (céu físico). Sem a marca, o `drive_postfx` da
            // engine escrevia por cima a curva `BASE_EV100` do domo antigo
            // (EV100 ~10,2 ao meio-dia) e o céu físico saía branco/cinza.
            ExternalExposure,
        ));
        let total = cameras.iter().count();
        info!(
            camera = %chosen,
            orbit_cameras = total,
            "vael: `ElementsCamera` na câmara 3D"
        );
        // Uma linha por arranque/troca: quantas câmaras OrbitCamera andam no
        // mundo (diagnóstico das transitórias do carregamento).
        if *reported == 0 || total > 1 {
            *reported += 1;
            if total > 1 {
                info!(
                    total,
                    "vael: câmaras com OrbitCamera vivas neste frame (só a escolhida segue o ambiente)"
                );
            }
        }
    }
}

/// Repõe o `AtmosphereState` da engine a partir do sol astronómico.
///
/// O `atmosphere_drive` da engine corre sem `<DayCycle>` como se fosse dia
/// pleno (45°); sem isto a névoa, o grading, a exposição e a direção do sol
/// no terreno ficavam presos no dia enquanto o céu do crate anoitecia.
///
/// Dentro de uma bolsa de interior a paleta é de dia por desenho (o
/// `interior_lighting_drive` já o decide) — não se toca.
fn bridge_atmosphere(
    sun: Res<SunState>,
    weather: Res<Weather>,
    interior: Option<Res<InteriorLighting>>,
    mut atmosphere: ResMut<AtmosphereState>,
) {
    if interior.as_deref().is_some_and(|state| state.active) {
        return;
    }
    let mut next = viber::worldsys::atmosphere_for_elevation(sun.altitude_deg);
    next.sun_dir = sun.direction;
    next.moon_dir = sun.moon_direction;
    // A chuva fecha um pouco a exposição (o mesmo law do `atmosphere_drive`).
    next.exposure_scale *= 1.0 - 0.08 * elements_rain(&weather) as f32;
    *atmosphere = next;
}

/// Leva o clima do crate para o `WeatherState` da engine.
///
/// Sem isto, `terrain_rain_wetness`, `rain_ambience_driver`,
/// `VaelFloraPlugin::drive_weather` e as regras de chuva ficavam a ler um
/// recurso ausente (o `<Weather>` saiu do mundo). A chuva leva a escala do
/// bioma (`rain-scale` do deserto), como o `weather_drive` antigo fazia.
///
/// Escreve só quando muda: o `drive_weather` da flora usa `is_changed`.
fn bridge_weather(
    weather: Res<Weather>,
    regions: Option<Res<BiomeRegions>>,
    players: Query<&GlobalTransform, With<Player>>,
    mut state: ResMut<WeatherState>,
) {
    let biome_scale = players
        .iter()
        .next()
        .and_then(|player| {
            let position = player.translation();
            rain_scale_at(regions.as_deref(), position.x, position.z)
        })
        .unwrap_or(1.0);
    let rain = (elements_rain(&weather) as f32 * biome_scale).clamp(0.0, 1.0);
    let direction = if weather.wind_direction.length_squared() > 1e-6 {
        weather.wind_direction.normalize()
    } else {
        Vec2::X
    };
    let wind_strength = (weather.wind_speed_ms / 4.0).clamp(0.0, 1.5);
    let clouds = weather.cloud_cover.clamp(0.0, 1.0);

    let changed = (state.rain - rain).abs() > 1e-3
        || (state.wind_strength - wind_strength).abs() > 1e-3
        || (state.clouds - clouds).abs() > 1e-3
        || (state.wind[0] - direction.x).abs() > 1e-3
        || (state.wind[1] - direction.y).abs() > 1e-3
        || state.cycle;
    if changed {
        state.wind = [direction.x, direction.y];
        state.wind_strength = wind_strength;
        state.clouds = clouds;
        state.rain = rain;
        state.cycle = false;
    }
}

/// Densidade de `FogFalloff::ExponentialSquared` que põe 2 % de contraste à
/// `visibility_m` do clima, sem nunca ABRIR a névoa do bioma.
///
/// Bevy usa `exp(-(density · d)²)`; igualar `(density·vis)²` a `-ln(0.02)`
/// (= 3,912) dá `density = 3,912^0.5 / vis`. A 40 km o termo do clima é
/// desprezável e a névoa por bioma fica intacta; a 300 m (`Fog`) domina.
pub fn weather_fog_density(biome: f32, visibility_m: f32) -> f32 {
    let visibility = visibility_m.max(1.0);
    biome.max(3.912_f32.sqrt() / visibility)
}

/// Aplica a visibilidade do clima à `DistanceFog` que a engine já inseriu.
///
/// Corre em `PostUpdate` porque o `biome_fog_system` insere a `DistanceFog`
/// por `Commands` (4×/s): depois de os comandos assentarem, este sistema
/// sobe só a densidade, preservando cor, bioma e o dono da névoa.
fn apply_weather_visibility(
    weather: Option<Res<Weather>>,
    interior: Option<Res<InteriorLighting>>,
    mut cameras: Query<&mut DistanceFog, With<Camera3d>>,
) {
    let Some(weather) = weather else {
        return;
    };
    if interior.as_deref().is_some_and(|state| state.active) {
        return;
    }
    for mut fog in &mut cameras {
        if let FogFalloff::ExponentialSquared { density } = &mut fog.falloff {
            let next = weather_fog_density(*density, weather.visibility_m);
            if (next - *density).abs() > 1e-6 {
                *density = next;
            }
        }
    }
}

/// Juice de exposição da engine (`viber.punch`) aplicado por cima do EV do
/// crate.
///
/// O `drive_postfx` da engine aplica o kick a `Exposure::ev100`; com
/// [`ExternalExposure`] deixa de tocar na câmara do crate. Para o juice não
/// desaparecer, este par de sistemas aplica-o depois do `drive_postfx`
/// (PostUpdate) e **desfá-lo antes do `auto_exposure` do crate** (PreUpdate):
/// sem o desfazer, a adaptação do crate lia o EV com o kick e o flash ficava
/// lido como cena mais escura, arrastando a exposição para baixo.
///
/// O `viber.punch` negativo (o "ai" do dano recebido) escurece por outro
/// caminho: empurra o `state.ev100` da engine acima do alvo (piso
/// `BASE_EV100 + MAX_DARKEN_EV`) e o drive devolve-o ao alvo a `BLEND_RATE`.
/// Essa escrita também não chega à câmara marcada — o dono aplica o desvio
/// POSITIVO (`ev100 − target`), que é o escurecimento transitório; a parte
/// negativa do blend de transições fica de fora porque a adaptação do crate
/// já cobre o clima por si (não duplicar).
#[derive(Resource, Default)]
struct VaelExposureKick {
    /// `(câmara, EV de juice)` aplicado no fim do frame anterior.
    applied: Vec<(Entity, f32)>,
}

/// Desfaz o juice do frame anterior (corre em `PreUpdate`, antes do
/// `auto_exposure` do crate).
fn undo_exposure_kick(
    mut kick: ResMut<VaelExposureKick>,
    mut cameras: Query<&mut bevy::camera::Exposure>,
) {
    for (entity, ev) in kick.applied.drain(..) {
        if let Ok(mut exposure) = cameras.get_mut(entity) {
            exposure.ev100 += ev;
        }
    }
}

/// Aplica o juice corrente ao EV do crate, no fim do frame (depois de o
/// `drive_postfx` ter decaído o kick e blendado o EV).
fn apply_exposure_kick(
    postfx: Option<Res<PostFxState>>,
    mut kick: ResMut<VaelExposureKick>,
    mut cameras: Query<(Entity, &mut bevy::camera::Exposure), With<ElementsCamera>>,
) {
    let ev = postfx.as_deref().map_or(0.0, |state| {
        state.kick + (state.ev100 - state.target_ev100).max(0.0)
    });
    if ev <= 0.0 {
        return;
    }
    for (entity, mut exposure) in &mut cameras {
        exposure.ev100 -= ev;
        kick.applied.push((entity, ev));
    }
}

/// Regime de interior para o crate: dentro da bolsa, **a atmosfera do crate
/// sai da câmara** (`ElementsCamera.atmosphere = false`). O crate documenta
/// que sem atmosfera a camada de nuvens não é desenhada nessa vista — era ela
/// que tapava o celeiro de branco em tempo nublado. A sala volta ao
/// `GlobalAmbientLight` autoral e às PointLights; fora, a atmosfera volta.
///
/// Escreve só o que muda: uma escrita em `ElementsCamera` marca o componente
/// `Changed` e o `redress_cameras` do crate volta a vestir a câmara nesse
/// frame (inserts/removes e `ClearColor`). O mesmo vale para `Exposure`,
/// `AmbientLight` e os mapas de ambiente, que têm outros donos.
fn interior_elements_lighting(
    interior: Option<Res<InteriorLighting>>,
    global: Option<Res<GlobalAmbientLight>>,
    mut cameras: Query<
        (
            &mut AmbientLight,
            Option<&mut AtmosphereEnvironmentMapLight>,
            &mut bevy::camera::Exposure,
            &mut ElementsCamera,
            Option<&mut GeneratedEnvironmentMapLight>,
        ),
        With<ElementsCamera>,
    >,
    mut suns: Query<&mut DirectionalLight>,
) {
    let inside = interior.as_deref().is_some_and(|state| state.active);
    for (mut ambient, mut environment, mut exposure, mut camera, mut probe) in &mut cameras {
        if inside {
            if camera.atmosphere {
                camera.atmosphere = false;
            }
            let atmosphere_dark = environment
                .as_deref()
                .is_some_and(|environment| environment.intensity != INTERIOR_ATMOSPHERE_INTENSITY);
            if atmosphere_dark
                && let Some(environment) = environment.as_deref_mut()
            {
                environment.intensity = INTERIOR_ATMOSPHERE_INTENSITY;
            }
            let probe_on = probe
                .as_deref()
                .is_some_and(|probe| probe.intensity != 0.0);
            if probe_on
                && let Some(probe) = probe.as_deref_mut()
            {
                probe.intensity = 0.0;
            }
            if let Some(global) = global.as_deref() {
                if ambient.color != global.color {
                    ambient.color = global.color;
                }
                if ambient.brightness != global.brightness {
                    ambient.brightness = global.brightness;
                }
            }
            // O `auto_exposure` do crate mede o CÉU (EV de dia); dentro da
            // sala isso deixava as lâmpadas 3-4 stops subexpostas. Desligar o
            // medidor NA CÂMARA é o que garante que ninguém reescreve o EV.
            if camera.auto_exposure {
                camera.auto_exposure = false;
            }
            if exposure.ev100 != INTERIOR_EV100 {
                exposure.ev100 = INTERIOR_EV100;
            }
            // O `interior_lighting_apply` da engine zero os direcionais, mas o
            // `drive_lights` do crate reescreve-os no set `Celestial`; este
            // sistema corre DEPOIS dele, por isso ganha. Aqui a escrita é por
            // frame de propósito: é o único valor que o crate reescreve.
            for mut light in &mut suns {
                light.illuminance = 0.0;
            }
        } else {
            if !camera.atmosphere {
                camera.atmosphere = true;
            }
            let atmosphere_lit = environment
                .as_deref()
                .is_some_and(|environment| environment.intensity != 1.0);
            if atmosphere_lit
                && let Some(environment) = environment.as_deref_mut()
            {
                environment.intensity = 1.0;
            }
            let probe_lit = probe
                .as_deref()
                .is_some_and(|probe| probe.intensity != 1.0);
            if probe_lit
                && let Some(probe) = probe.as_deref_mut()
            {
                probe.intensity = 1.0;
            }
            if !camera.auto_exposure {
                camera.auto_exposure = true;
            }
            // O `update_ambient`/`drive_lights`/`auto_exposure` do crate
            // reescrevem o resto.
        }
    }
}

/// EV100 de uma sala de interior (o `Exposure::INDOOR` do Bevy).
const INTERIOR_EV100: f32 = 7.0;

/// Intensidade do `AtmosphereEnvironmentMapLight` dentro de uma sala.
const INTERIOR_ATMOSPHERE_INTENSITY: f32 = 0.0;

/// Trovão do crate → SFX da engine.
///
/// O `bevy_elements` emite `Thunder` com atraso `distância / 343` s; o
/// `loudness` (1 / (1 + km)) já traz a atenuação, por isso usa-se o
/// [`SfxLoudEvent`] (ganho explícito) em vez do `SfxEvent` de 40 m.
fn bridge_thunder(
    mut thunders: MessageReader<Thunder>,
    interior: Option<Res<InteriorLighting>>,
    mut sfx: MessageWriter<SfxLoudEvent>,
) {
    let indoors = interior.as_deref().is_some_and(|state| state.active);
    for thunder in thunders.read() {
        if indoors {
            continue;
        }
        let volume = (thunder.loudness * 0.8).clamp(0.03, 0.9);
        sfx.write(SfxLoudEvent {
            clip: SfxClip::Thunder,
            position: Some(thunder.position),
            volume,
        });
        info!(
            distance_m = thunder.distance_m,
            loudness = thunder.loudness,
            "vael: trovão"
        );
    }
}

/// Marcador do bloqueador de precipitação da sala actual.
#[derive(Component)]
struct VaelInteriorBlocker;

/// Precipitação do crate dentro de casa: um volume grande centrado na
/// **câmara** (não na sala).
///
/// A chuva do crate nasce num caixa de ~60 m à volta da câmara; a câmara de
/// interior fica acima/fora da sala, por isso um blocker do tamanho da sala
/// deixava as gotas entre a câmara e o telhado e a sala ficava branca. O
/// volume cobre a caixa de partículas inteira enquanto `InteriorLighting`
/// estiver activo; fora é removido.
fn interior_precipitation(
    interior: Option<Res<InteriorLighting>>,
    cameras: viber::editor_camera::EyeCameraQuery,
    mut commands: Commands,
    mut blockers: Query<
        (Entity, &mut Transform, &mut PrecipitationBlocker),
        With<VaelInteriorBlocker>,
    >,
) {
    let inside = interior.as_deref().is_some_and(|state| state.active);
    let Some(center) = cameras
        .iter()
        .find(|(_, cam, target)| viber::editor_camera::is_window_camera(*cam, *target))
        .map(|(tf, _, _)| tf.translation())
    else {
        for (entity, _, _) in &blockers {
            commands.entity(entity).despawn();
        }
        return;
    };
    if !inside {
        for (entity, _, _) in &blockers {
            commands.entity(entity).despawn();
        }
        return;
    }
    // A caixa de precipitação é 60 m; 120 m de meia-extensão cobre-a com
    // margem mesmo com a câmara a mover-se dentro da sala.
    let half_extents = Vec3::new(120.0, 60.0, 120.0);
    let mut found = false;
    for (entity, mut transform, mut blocker) in &mut blockers {
        if found {
            // Nunca deveria haver dois; se uma troca de cena deixou um para
            // trás, limpa-o em vez de acumular.
            commands.entity(entity).despawn();
            continue;
        }
        if transform.translation != center {
            transform.translation = center;
        }
        if blocker.half_extents != half_extents {
            blocker.half_extents = half_extents;
        }
        found = true;
    }
    if !found {
        commands.spawn((
            VaelInteriorBlocker,
            PrecipitationBlocker { half_extents },
            Transform::from_translation(center),
            Name::new("vael: interior precipitation blocker"),
        ));
    }
}

/// Alimenta o `clock`/`weather` do HUD da engine a partir do crate.
///
/// O `collect_ui_data` da engine só escreve estes campos com
/// `<DayCycle>`/`<Weather>`, que saíram do mundo; sem isto o relógio do HUD
/// ficava congelado no texto autoral e o efeito de chuva do HUD nunca ligava.
/// Escritas condicionais: `UiData` é `ResMut` e o `deref_mut` marca o
/// recurso em `Changed`.
fn bridge_ui_clock(
    clock: Res<WorldClock>,
    weather: Res<Weather>,
    mut data: ResMut<viber::ui::UiData>,
) {
    let minutes = (clock.hour * 60.0).rem_euclid(1440.0);
    let text = format!(
        "{:02}:{:02}",
        (minutes / 60.0).floor() as u32,
        (minutes % 60.0).floor() as u32
    );
    if data.clock != text {
        data.clock = text;
    }
    let fraction = (clock.hour.rem_euclid(24.0) / 24.0) as f32;
    if data.day_fraction != fraction {
        data.day_fraction = fraction;
    }
    let rain = elements_rain(&weather) as f32;
    if data.weather_rain != rain {
        data.weather_rain = rain;
    }
}

// ── QA vivo (`viber.vael.env_*`) ───────────────────────────────────────

/// Força o clima de `VAEL_START_WEATHER` no arranque (antes do primeiro
/// probe do céu; um `force` depois não salta o relógio).
fn force_start_weather(start: Res<StartWeather>, mut director: ResMut<WeatherDirector>) {
    if let Some(preset) = start.0 {
        director.force(preset, Duration::ZERO);
        info!(preset = ?preset, "vael: clima inicial (cena/VAEL_START_WEATHER)");
    }
}

/// Clima de arranque resolvido (cena + `VAEL_START_WEATHER`).
#[derive(Resource, Debug, Clone, Copy)]
struct StartWeather(Option<WeatherPreset>);

/// Edição do `ElementsSettings` com o jogo a correr (o Jackdaw gravou):
/// latitude, escala do tempo e clima seguem; a hora só muda se a da cena
/// mudou (não reescreve o relógio a cada gravação).
fn apply_scene_settings(
    setting: Option<Res<SceneSetting<ElementsSettings>>>,
    mut clock: ResMut<WorldClock>,
    mut geo: ResMut<GeoLocation>,
    mut director: ResMut<WeatherDirector>,
    mut last: Local<Option<ElementsSettings>>,
) {
    let Some(setting) = setting else {
        return;
    };
    let Some(previous) = last.replace(setting.0.clone()) else {
        return;
    };
    if previous == setting.0 {
        return;
    }
    let scene = &setting.0;
    geo.latitude_deg = scene.latitude_deg;
    clock.time_scale = scene.time_scale;
    if previous.start_hour != scene.start_hour {
        clock.hour = scene.start_hour.rem_euclid(24.0);
    }
    if previous.start_weather != scene.start_weather
        && let Some(preset) = scene.start_weather.preset_name().and_then(weather_preset)
    {
        director.force(preset, Duration::from_secs(5));
    }
    info!(
        hour = scene.start_hour,
        latitude = scene.latitude_deg,
        weather = ?scene.start_weather,
        "vael: ambiente da cena atualizado"
    );
}

/// Aplica os pedidos do QA antes do `ElementsSystems::Clock`.
fn apply_env_control(
    control: Res<VaelEnvControl>,
    mut clock: ResMut<WorldClock>,
    mut director: ResMut<WeatherDirector>,
) {
    let mut control = control.lock();
    if let Some(hour) = control.set_hour.take() {
        clock.hour = hour.rem_euclid(24.0);
        info!(hour = clock.hour, "vael: hora forçada (QA)");
    }
    if let Some(paused) = control.set_paused.take() {
        clock.paused = paused;
        info!(paused, "vael: relógio do ambiente (QA)");
    }
    if let Some(name) = control.set_weather.take() {
        let preset = WeatherPreset::ALL
            .into_iter()
            .find(|preset| format!("{preset:?}").eq_ignore_ascii_case(&name));
        match preset {
            Some(preset) => {
                director.force(preset, Duration::ZERO);
                info!(preset = ?preset, "vael: clima forçado (QA)");
            }
            None => warn!(%name, "vael: `env_set_weather` com preset desconhecido"),
        }
    }
}

/// Publica a leitura que o QA lê em `viber.vael.env_info()`.
fn collect_env_info(
    clock: Res<WorldClock>,
    sun: Res<SunState>,
    weather: Res<Weather>,
    state: Res<WeatherState>,
    weathering: Option<Res<SurfaceWeathering>>,
    shadows: Option<Res<CloudShadows>>,
    elements_exposure: Option<Res<ElementsExposure>>,
    environment: Environment,
    players: Query<&GlobalTransform, With<Player>>,
    cameras: Query<
        (
            &AmbientLight,
            &bevy::camera::Exposure,
            Option<&AtmosphereEnvironmentMapLight>,
            Option<&DistanceFog>,
            Has<bevy::light::VolumetricFog>,
        ),
        With<ElementsCamera>,
    >,
    precip: Option<Res<PrecipitationState>>,
    interior: Option<Res<InteriorLighting>>,
    control: Res<VaelEnvControl>,
) {
    let player_cloud_shadow = players
        .iter()
        .next()
        .and_then(|player| environment.cloud_shadow_at(player.translation()));
    let camera = cameras.iter().next();
    let weathering = weathering.as_deref();
    control.lock().info = EnvInfo {
        hour: clock.hour,
        sun_altitude_deg: sun.altitude_deg,
        moon_altitude_deg: sun.moon_altitude_deg,
        cloud_cover: weather.cloud_cover,
        rain: elements_rain(&weather) as f32,
        rain_effective: state.rain,
        temperature_c: weather.temperature_c,
        visibility_m: weather.visibility_m,
        wind_speed_ms: weather.wind_speed_ms,
        wind_dir_x: weather.wind_direction.x,
        wind_dir_z: weather.wind_direction.y,
        gustiness: weather.gustiness,
        storminess: weather.storminess,
        wetness: weathering.map_or(0.0, |w| w.wetness),
        snow_cover: weathering.map_or(0.0, |w| w.snow_cover),
        snow_depth_m: weathering.map_or(0.0, |w| w.snow_depth_m),
        cloud_transmittance: shadows.as_deref().map_or(1.0, |s| s.mean_transmittance),
        player_cloud_shadow,
        camera_ev100: camera.map_or(0.0, |(_, exposure, _, _, _)| exposure.ev100),
        elements_ev100: elements_exposure
            .as_deref()
            .map_or(0.0, |exposure| exposure.ev100),
        camera_ambient: camera.map_or(0.0, |(ambient, _, _, _, _)| ambient.brightness),
        camera_env_intensity: camera.map_or(0.0, |(_, _, environment, _, _)| {
            environment.map_or(0.0, |environment| environment.intensity)
        }),
        camera_fog_density: camera.map_or(0.0, |(_, _, _, fog, _)| {
            fog.map_or(0.0, |fog| match fog.falloff {
                FogFalloff::ExponentialSquared { density } => density,
                FogFalloff::Exponential { density } => density,
                _ => 0.0,
            })
        }),
        camera_volumetric: camera.is_some_and(|(_, _, _, _, volumetric)| volumetric),
        precip_rain: precip.as_deref().map_or(0, |state| state.rain),
        precip_snow: precip.as_deref().map_or(0, |state| state.snow),
        precip_dust: precip.as_deref().map_or(0, |state| state.dust),
        interior: interior.as_deref().is_some_and(|state| state.active),
    };
}

/// Arranque: instala `env_set_hour`, `env_pause`, `env_set_weather` e
/// `env_info` em `viber.vael` (depois do relógio, que cria a tabela).
fn install_env_api(control: Res<VaelEnvControl>) {
    let inner = control.clone();
    crate::script_host::register_installer(move |lua| install_env_api_inner(lua, &inner));
}

fn install_env_api_inner(lua: &Lua, control: &VaelEnvControl) -> mlua::Result<()> {
    let vael = vael_table(lua)?;

    vael_fn!(
        lua,
        vael,
        "env_set_hour",
        [control = control.clone()],
        move |_, hour: f64| {
            control.lock().set_hour = Some(hour);
            Ok(())
        }
    );

    vael_fn!(
        lua,
        vael,
        "env_pause",
        [control = control.clone()],
        move |_, paused: bool| {
            control.lock().set_paused = Some(paused);
            Ok(())
        }
    );

    vael_fn!(
        lua,
        vael,
        "env_set_weather",
        [control = control.clone()],
        move |_, name: String| {
            control.lock().set_weather = Some(name);
            Ok(())
        }
    );

    vael_fn!(
        lua,
        vael,
        "env_info",
        [control = control.clone()],
        move |lua, ()| {
            let info = control.lock().info;
            let table = lua.create_table()?;
            table.set("hour", info.hour)?;
            table.set("sun_altitude", info.sun_altitude_deg)?;
            table.set("moon_altitude", info.moon_altitude_deg)?;
            table.set("cloud", info.cloud_cover)?;
            table.set("rain", info.rain)?;
            table.set("rain_effective", info.rain_effective)?;
            table.set("temperature", info.temperature_c)?;
            table.set("visibility", info.visibility_m)?;
            table.set("wind_speed", info.wind_speed_ms)?;
            table.set("wind_dir_x", info.wind_dir_x)?;
            table.set("wind_dir_z", info.wind_dir_z)?;
            table.set("gustiness", info.gustiness)?;
            table.set("storminess", info.storminess)?;
            table.set("wetness", info.wetness)?;
            table.set("snow_cover", info.snow_cover)?;
            table.set("snow_depth", info.snow_depth_m)?;
            table.set("cloud_transmittance", info.cloud_transmittance)?;
            table.set("camera_ev100", info.camera_ev100)?;
            table.set("elements_ev100", info.elements_ev100)?;
            table.set("camera_ambient", info.camera_ambient)?;
            table.set("camera_env_intensity", info.camera_env_intensity)?;
            table.set("camera_fog_density", info.camera_fog_density)?;
            table.set("camera_volumetric", info.camera_volumetric)?;
            match info.player_cloud_shadow {
                Some(value) => table.set("player_cloud_shadow", value)?,
                None => table.set("player_cloud_shadow", Value::Nil)?,
            }
            table.set("precip_rain", info.precip_rain)?;
            table.set("precip_snow", info.precip_snow)?;
            table.set("precip_dust", info.precip_dust)?;
            table.set("interior", info.interior)?;
            Ok(table)
        }
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2 % de contraste à visibilidade pedida.
    #[test]
    fn fog_visibility_sets_two_percent_contrast() {
        let density = weather_fog_density(0.0, 1000.0);
        let transmittance = (-(density * 1000.0).powi(2)).exp();
        assert!((transmittance - 0.02).abs() < 1e-3, "T = {transmittance}");
    }

    /// Com tempo aberto a névoa do bioma manda; a 300 m o clima fecha-a.
    #[test]
    fn fog_visibility_keeps_biome_floor_but_closes_in_fog() {
        let biome = 0.0008;
        assert!((weather_fog_density(biome, 40_000.0) - biome).abs() < 1e-9);
        assert!(weather_fog_density(0.0001, 300.0) > 0.006);
    }

    /// Só a TRANSIÇÃO escreve na câmara: um `ElementsCamera` tocado todos os
    /// frames faz o `redress_cameras` do crate voltar a vestir a câmara sempre
    /// (inserts/removes e `ClearColor`).
    #[test]
    fn interior_lighting_only_writes_on_a_change() {
        use bevy::ecs::system::RunSystemOnce as _;

        let mut world = World::default();
        world.insert_resource(InteriorLighting {
            active: true,
            ..default()
        });
        world.insert_resource(GlobalAmbientLight {
            color: Color::WHITE,
            brightness: 2_000.0,
            ..default()
        });
        let camera = world
            .spawn((
                ElementsCamera::default(),
                AmbientLight::default(),
                bevy::camera::Exposure::default(),
                AtmosphereEnvironmentMapLight::default(),
            ))
            .id();

        world.run_system_once(interior_elements_lighting).unwrap();
        world.flush();
        let inside = world.get::<ElementsCamera>(camera).expect("câmara");
        assert!(!inside.atmosphere, "dentro da sala a atmosfera sai");
        assert!(!inside.auto_exposure, "e o medidor de céu desliga");
        let first = world
            .get_entity(camera)
            .unwrap()
            .get_change_ticks::<ElementsCamera>()
            .unwrap()
            .changed;

        world.run_system_once(interior_elements_lighting).unwrap();
        world.flush();
        let second = world
            .get_entity(camera)
            .unwrap()
            .get_change_ticks::<ElementsCamera>()
            .unwrap()
            .changed;
        assert_eq!(
            first, second,
            "sem transição não há escrita em `ElementsCamera` (nem `Changed` por frame)"
        );

        // Sair da sala volta a ligar atmosfera/exposição automática.
        world.resource_mut::<InteriorLighting>().active = false;
        world.run_system_once(interior_elements_lighting).unwrap();
        world.flush();
        let outside = world.get::<ElementsCamera>(camera).expect("câmara");
        assert!(outside.atmosphere && outside.auto_exposure);
    }
}

/// O `bevy_elements` dá ao sol 4 cascatas até 600 m (para o nevoeiro
/// volumétrico receber luz até ao horizonte) e reimpõe-nas em cada frame no
/// `fit_shadow_cascades`. O Vael tem o volumétrico desligado
/// (`postfx.disabled`), e o passe de sombras é draw-bound: a 600 m varria
/// todas as malhas do mapa — **10,6 ms de GPU** medidos a 2026-11-15 (RTX
/// 4050, metade do frame). Aqui reaplica-se o alcance do preset da engine
/// (`VIBER_SHADOW_CASCADES`/`VIBER_SHADOW_DISTANCE`; `equilibrado` = 2
/// cascatas, 110 m). `VAEL_ELEMENTS_CASCADES=1` devolve as do crate (A/B).
fn fit_vael_sun_cascades(
    mut suns: Query<&mut bevy::light::CascadeShadowConfig, With<bevy_elements::Sun>>,
    mut want: Local<Option<Option<bevy::light::CascadeShadowConfig>>>,
) {
    let want = want.get_or_insert_with(|| {
        if std::env::var("VAEL_ELEMENTS_CASCADES").ok().as_deref() == Some("1") {
            return None;
        }
        Some(
            bevy::light::CascadeShadowConfigBuilder {
                num_cascades: viber::recipes::sun_shadow_cascades(),
                first_cascade_far_bound: 12.0,
                maximum_distance: viber::recipes::sun_shadow_distance(),
                ..default()
            }
            .build(),
        )
    });
    let Some(want) = want.as_ref() else {
        return;
    };
    for mut config in &mut suns {
        if config.bounds != want.bounds || config.minimum_distance != want.minimum_distance {
            *config = want.clone();
        }
    }
}

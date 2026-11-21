//! Ambiente — o shim do monólito + os visuais de chuva.
//!
//! O `ambient.rs` repartiu-se (exit5/G-2):
//!
//! - **Áudio** (SFX, loops de água/cachoeira/chuva) vive no
//!   `bevy_ext_audio::{sfx, ambience}` — reexportado aqui, os sistemas que o
//!   [`AmbientPlugin`] registava são OS DO CRATE (mesmos TypePaths
//!   `vael_engine::ambient`; o host publica `AudioPaths`/`AudioWorldState`);
//! - **Névoa/bioma** vive no `bevy_ext_postfx::ambient` (a lente lê a mesma
//!   paleta do `env`);
//! - **Sombras de ponto, lanterna, gestos de NPC** vivem no
//!   `bevy_ext_engine::ambient`;
//! - **Contratos** (`SfxClip`/`SfxEvent`/`SfxLoudEvent`/`EngineRainVisuals`/
//!   `AMBIENT_LOOP_FILES`) são do `bevy_engine_core` (reexportados).
//!
//! Fica AQUI: o emissor visual de chuva (partículas do monólito) e o plugin,
//! que ordena contra `worldsys`/`sky`/`particles` do monólito. O caminho
//! `vael_engine::ambient::*` continua a resolver por este módulo.

use bevy::prelude::*;

pub use bevy_engine_core::{
    AMBIENT_LOOP_FILES, EngineRainVisuals, SFX_CLIPS_ALL, SfxClip, SfxEvent, SfxLoudEvent,
};
// Névoa/bioma: dono no postfx (a lente lê a paleta do `env` no mesmo frame).
pub use bevy_ext_postfx::ambient::{
    CurrentBiome, biome_fog_system, fog_density_multiplier, FOG_BASE_DENSITY, FOG_REFERENCE_M,
};
// Sombras/lanterna/gestos: dono na engine.
pub use bevy_ext_engine::ambient::{
    GESTURE_MIN_INTERVAL, LANTERN_MAX_INTENSITY, LANTERN_RANGE, LIGHT_BUDGET,
    LIGHT_BUDGET_INTERVAL, PointShadowsEnabled, SHADOW_LIGHT_BUDGET, SHADOW_LIGHT_DAYLIGHT_MAX,
    SHADOW_LIGHT_MAX_DISTANCE, TorchLightOverride, lantern_driver, light_budget_system,
    npc_gesture_system, set_quality_shadow_cap, shadow_light_budget, shadow_light_daylight_max,
    shadow_light_max_distance, wants_point_shadow,
};
// Fatia de áudio: dono no bevy_ext_audio (os TypePaths `viber::ambient` de
// `WaterAmbienceLoop`/etc. ficam congelados lá).
pub use bevy_ext_audio::{
    RainAmbienceLoop, SfxHandles, WaterAmbienceLoop, WaterfallLoop, load_sfx_assets, pitch_jitter,
    rain_ambience_driver, sfx_loud_player_system, sfx_player_system, setup_rain_ambience,
    setup_water_ambience, setup_waterfall_ambience, water_ambience_driver, waterfall_ambience_driver,
};
// Contratos de luz do postfx env (o `torch_pool_drive` do crate lê o MESMO
// marcador) e o ponto-em-polígono (o mesmo algoritmo — dono único no `env`).
pub use bevy_ext_postfx::env::{AuthoredShadowLight, TravellerLantern, point_in_polygon};

use crate::player::Player;

// ── SFX ─────────────────────────────────────────────────────────────────

/// Marcador: o jogo substituiu a lanterna automática noturna (`lantern_driver`)
/// por um driver próprio (ex.: lanterna de cintura com botão no Vael).
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct ExternalLantern;

fn engine_rain_visuals(flag: Res<EngineRainVisuals>) -> bool {
    flag.0
}

// ── plugin ──────────────────────────────────────────────────────────────

pub struct AmbientPlugin;

impl Plugin for AmbientPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CurrentBiome>()
            .init_resource::<PointShadowsEnabled>()
            .init_resource::<RainLook>()
            .init_resource::<EngineRainVisuals>()
            .init_resource::<TorchLightOverride>()
            // GLUE da atmosfera (peça céu): [`crate::worldsys::AtmosphereState`]
            // é a paleta da hora partilhada por céu/fog/grading. Era escrita
            // por `atmosphere_drive` e lida por três sistemas — mas NINGUÉM a
            // inseria como recurso nem registava os sistemas, pelo que o fog,
            // o grading e o storage buffer do domo falhavam a validação de
            // params todos os frames (céu/fog congelados no default de dia).
            .init_resource::<crate::worldsys::AtmosphereState>()
            .add_message::<SfxEvent>()
            .add_message::<SfxLoudEvent>()
            .add_systems(PostStartup, load_sfx_assets)
            .add_systems(
                Update,
                (
                    // A névoa lê a paleta publicada NO MESMO frame.
                    biome_fog_system.after(crate::worldsys::atmosphere_drive),
                    light_budget_system,
                    npc_gesture_system,
                    sfx_player_system,
                    sfx_loud_player_system,
                    setup_water_ambience,
                    water_ambience_driver,
                    setup_waterfall_ambience,
                    waterfall_ambience_driver,
                    // Chuva + lanterna (WS-A): a chuva lê a intensidade já
                    // rodada pelo scheduler. O MOVER+compensação corre antes
                    // do step dos emissores (main.rs) — as gotas compensam no
                    // mesmo frame em que a âncora anda.
                    rain_emitter_driver
                        .after(crate::worldsys::weather_drive)
                        .before(crate::particles::particle_emitter_update)
                        .run_if(engine_rain_visuals),
                    rain_ripple_spawner,
                    setup_rain_ambience,
                    rain_ambience_driver,
                    // A lanterna REESCREVE a Visibility por frame — tem de
                    // ganhar ao orçamento de luzes (que corre 1×/s). Desliga-se
                    // quando o jogo instala [`ExternalLantern`].
                    lantern_driver
                        .after(light_budget_system)
                        .run_if(not(resource_exists::<ExternalLantern>)),
                ),
            )
            .add_systems(
                Update,
                (
                    // Scheduler do `<Weather cycle>` (WS-A): a intensidade
                    // contínua de chuva entra na paleta NO MESMO frame.
                    crate::profiler::timed(
                        crate::profiler::Group::World,
                        crate::worldsys::weather_drive,
                    )
                    .before(crate::worldsys::atmosphere_drive),
                    // Publica a paleta a partir da posição do sol já
                    // resolvida por `sun_drive` (main.rs)…
                    crate::profiler::timed(
                        crate::profiler::Group::World,
                        crate::worldsys::atmosphere_drive,
                    )
                    .after(crate::worldsys::sun_drive)
                    // A lente (twin) lê a paleta do MESMO frame.
                    .before(crate::postfx::FxDrive),
                    // …e empurra-a para o storage buffer do domo (`SkyUniform`)
                    // — é esta escrita por frame que faz o céu mudar com o
                    // `set_clock` (o relógio do mundo não é `globals.time`).
                    crate::sky::sky_material_drive.after(crate::worldsys::atmosphere_drive),
                    // O tint dia/noite do TERRENO (r5, bissecção da banda
                    // branca noturna) e o chão molhado (r3, canal `walls_b.w`)
                    // NÃO se registam aqui: vivem no `TerrainFeaturesPlugin`
                    // (src/terrain/runtime.rs), embrulhados em `timed` para
                    // aparecerem no profiler. Registá-los também aqui criava
                    // uma SEGUNDA instância (o Bevy não deduplica
                    // `add_systems`): o sistema corria 2× por frame e qualquer
                    // `.after(fn)` contra ele falhava com "more than one
                    // instance".
                ),
            );
    }
}

/// Altura do emissor de chuva acima da cabeça do herói (m).
pub const RAIN_EMITTER_HEIGHT: f32 = 12.0;
/// Rate do emissor de chuva com intensidade 1 (partículas/s).
pub const RAIN_MAX_RATE: f32 = 500.0;
/// Ondinhas por segundo no chão com intensidade 1.
const RAIN_RIPPLES_PER_SEC: f32 = 14.0;
/// Raio (m) dentro do qual as ondinhas nascem.
const RAIN_RIPPLE_RADIUS: f32 = 8.0;
/// Ganho máximo do loop de chuva (antes dos buses do mixer).
const RAIN_MAX_GAIN: f32 = 0.4;

/// Look da chuva, aplicado ao emissor a cada frame — os números que decidem
/// se a tempestade se LÊ ou se vira um véu.
///
/// Vive num recurso (e não só no preset) porque a afinação é visual e
/// iterativa: `vael.debug.rain_look{...}` muda-os na engine viva, sem
/// recompilar nem reiniciar. Os defaults vêm do preset/env.
#[derive(Debug, Clone, Resource, Reflect)]
#[reflect(Resource)]
pub struct RainLook {
    /// Raio (m) do fade junto da câmara (ver `particles::RAIN_NEAR_FADE_M`).
    pub near_fade_m: f32,
    /// Tecto de alpha das gotas.
    pub max_alpha: f32,
    /// Multiplicador da LARGURA do streak (o preset é fino de propósito; o
    /// motion blur come os streaks finos a 20 m/s).
    pub width_scale: f32,
    /// Multiplicador do rate do emissor.
    pub rate_scale: f32,
}

impl Default for RainLook {
    fn default() -> Self {
        Self {
            near_fade_m: crate::particles::rain_near_fade_m(),
            max_alpha: crate::particles::rain_max_alpha(),
            width_scale: 1.0,
            rate_scale: 1.0,
        }
    }
}

/// Marker do emissor de chuva âncora do herói (uma instância por mundo).
/// A posição âncora vive no próprio `Transform` (a compensação do espaço
/// local das gotas compara com ela a cada frame).
#[derive(Debug, Component, Reflect)]
#[reflect(Component)]
pub struct RainEmitter;


fn rain_spec() -> crate::recipes::ParticleSpec {
    crate::recipes::ParticleSpec {
        preset: "rain".into(),
        emission_rate: None,
        life: None,
        speed: None,
        size: None,
        color: None,
        shape_radius: None,
        looping: true,
        world_space: false,
    }
}

fn ripple_spec() -> crate::recipes::ParticleSpec {
    crate::recipes::ParticleSpec {
        preset: "rain_ripple".into(),
        emission_rate: None,
        life: None,
        speed: None,
        size: None,
        color: None,
        shape_radius: None,
        looping: false,
        world_space: false,
    }
}

/// Spawna (uma vez) e conduz o emissor de chuva âncora do herói.
#[allow(clippy::type_complexity)]
fn rain_emitter_driver(
    mut commands: Commands,
    look: Res<RainLook>,
    scene: Option<Res<crate::worldsys::InteriorSceneConfig>>,
    weather: Option<Res<crate::worldsys::WeatherState>>,
    atmosphere: Option<Res<crate::worldsys::AtmosphereState>>,
    players: Query<&GlobalTransform, With<Player>>,
    mut emitters: Query<
        (
            &mut Transform,
            &mut crate::particles::ParticleEmitter,
            &mut Visibility,
        ),
        With<RainEmitter>,
    >,
) {
    let Some(weather) = weather else {
        return;
    };
    // `iter().next()` e não `single()`: ≥2 players não pode matar a chuva.
    let Some(player) = players.iter().next() else {
        return;
    };
    let anchor = player.translation() + Vec3::Y * RAIN_EMITTER_HEIGHT;
    // Telhado implícito: dentro da bolsa de interior não chove — o emissor
    // segue o player e seguiria para dentro da sala.
    let intensity = if scene
        .as_deref()
        .is_some_and(|s| s.contains(anchor.x, anchor.z))
    {
        0.0
    } else {
        weather.rain.clamp(0.0, 1.0)
    };
    let Ok((mut transform, mut emitter, mut visibility)) = emitters.single_mut() else {
        // Ainda não existe: spawna UM. O `Commands::queue` só dá `&mut World`
        // — daí o `spawn_looping_in_world`.
        commands.queue(move |world: &mut World| {
            let entity = crate::particles::spawn_looping_in_world(world, &rain_spec(), anchor);
            world.entity_mut(entity).insert(RainEmitter);
        });
        return;
    };
    // Compensa o espaço LOCAL: gotas já a meio da queda ficam fixas ao mundo.
    let delta = anchor - transform.translation;
    if delta != Vec3::ZERO {
        for particle in &mut emitter.sim.particles {
            particle.pos -= delta;
        }
    }
    transform.translation = anchor;
    // Rate ∝ intensidade; sem chuva o emissor desliga E esconde-se.
    emitter.sim.resolved.emission_rate = RAIN_MAX_RATE * intensity * look.rate_scale;
    // Look ao vivo (`vael.debug.rain_look`): o preset dá o default, o
    // recurso manda. Reaplicado todos os frames — é uma escrita de 4 f32.
    //
    // À NOITE a cortina é o véu nº 1: as gotas são cinza-claro não iluminado
    // e o fundo cai para azul-escuro, portanto o contraste relativo sobe. O
    // near-fade afasta-se da câmara (8 → 12 m) e o alpha cai 55% — a chuva
    // continua a ler-se, a cena por trás deixa de desaparecer. De dia
    // (`night` = 0) os valores ficam EXACTAMENTE como estavam.
    let night = atmosphere
        .as_ref()
        .map(|a| a.night.clamp(0.0, 1.0))
        .unwrap_or(0.0);
    emitter.sim.resolved.near_fade_m = look.near_fade_m * (1.0 + 0.5 * night);
    emitter.sim.resolved.max_alpha = look.max_alpha * (1.0 - 0.55 * night);
    let base = crate::particles::preset("rain");
    emitter.sim.resolved.size = (
        base.size.0 * look.width_scale,
        base.size.1 * look.width_scale,
    );
    let wanted = if intensity <= 0.01 {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
    if *visibility != wanted {
        *visibility = wanted;
    }
}

/// Estado acumulado do spawner de ondinhas (rng próprio = sem lockstep com
/// os emissores).
struct RainRippleState {
    accumulator: f32,
    rng: crate::spawner::Rng,
}

impl Default for RainRippleState {
    fn default() -> Self {
        Self {
            accumulator: 0.0,
            rng: crate::spawner::Rng::new(0x51A1_BEEF),
        }
    }
}

/// Ondinhas `rain_ripple` no chão à volta do herói enquanto chove.
#[allow(clippy::needless_pass_by_value, clippy::too_many_arguments)]
fn rain_ripple_spawner(
    time: Res<Time>,
    mut state: Local<RainRippleState>,
    weather: Option<Res<crate::worldsys::WeatherState>>,
    scene: Option<Res<crate::worldsys::InteriorSceneConfig>>,
    players: Query<&GlobalTransform, With<Player>>,
    runtime: Option<Res<crate::terrain::runtime::TerrainRuntime>>,
    mut commands: Commands,
) {
    let Some(weather) = weather else {
        return;
    };
    let intensity = weather.rain.clamp(0.0, 1.0);
    if intensity <= 0.05 {
        return;
    }
    let Some(player) = players.iter().next() else {
        return;
    };
    let origin = player.translation();
    // Sem telhado não há ondulações (o emissor já se cala no interior).
    if scene
        .as_deref()
        .is_some_and(|scene| scene.contains(origin.x, origin.z))
    {
        return;
    }
    state.accumulator += RAIN_RIPPLES_PER_SEC * intensity * time.delta_secs();
    if state.accumulator < 1.0 {
        return;
    }
    state.accumulator -= state.accumulator.floor();
    for _ in 0..2 {
        let disc = state.rng.unit_disc() * RAIN_RIPPLE_RADIUS;
        let x = origin.x + disc.x;
        let z = origin.z + disc.y;
        let y = runtime
            .as_ref()
            .filter(|r| r.in_field(x, z))
            .map(|r| {
                r.surface_below(x, z, origin.y + crate::player::GROUND_PROBE)
                    .unwrap_or_else(|| r.sample(x, z))
            })
            // Sem terreno o chão é o do herói, não 12 m abaixo (a altura do
            // emissor que só faz sentido para as gotas).
            .unwrap_or(origin.y);
        crate::particles::spawn_burst(&mut commands, &ripple_spec(), Vec3::new(x, y + 0.03, z), 2);
    }
}


// ── testes (o resto moveu com os donos: sombras → engine, névoa → postfx,
// SFX/clips → bevy_ext_audio) ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Os assets de áudio do pool do Vael existem (o registry de clips
    /// aponta para ficheiros reais — o `.ogg` falhado era warn + no-op).
    #[test]
    fn test_juice_audio_assets_exist() {
        let Some(pool) = crate::meshopt::shared_asset_pool() else {
            eprintln!("skip: pool do Vael ausente");
            return;
        };
        for rel in [
            "assets/audio/sfx/ambient/rain_loop.ogg",
            "assets/audio/sfx/ambient/footstep.ogg",
            "assets/audio/sfx/ambient/footstep_water.ogg",
            "assets/audio/sfx/ui/levelup.ogg",
            "assets/audio/sfx/ui/quest_complete.ogg",
            "assets/audio/sfx/combat/swing.ogg",
            "assets/audio/sfx/world/chest_open.ogg",
        ] {
            let path = pool.join(rel);
            assert!(path.is_file(), "falta {}", path.display());
        }
    }
}

//! Adaptador de HOST do `bevy_ext_audio` — o que no crate entra por recursos
//! ([`AudioPaths`], [`AudioWorldState`], [`AudioEar`]) e aqui sai dos
//! recursos da engine (`config::GameConfig`, `scene_loader::CurrentScene`,
//! `ambient::CurrentBiome`, `worldsys::WeatherState`, `editor_camera::EditorCamera`).
//!
//! O `music.rs` do monólito saiu para o crate (exit4/G1) e os sistemas
//! (`music_driver`, `script_sound_loops`) passaram a ler os contratos do
//! host — este plugin publica o espelho por frame. Sem ele os sistemas são
//! no-ops (sem paths de clips, sem zona de cena/bioma) — o jogo compila e
//! corre, mas em silêncio.
//!
//! O análogo no Vael: `app.add_plugins(viber::audio_host::AudioHostPlugin);`
//! junto do registo dos sistemas `viber::music::*`.

use std::sync::Arc;

use bevy::prelude::*;

use bevy_ext_audio::{
    AudioEar, AudioPaths, AudioWorldState, HostWaterBodies, WaterBodies, WaterKind, WaterfallSpot,
};
use bevy_ext_water::body::{WaterBody, WaterKind as BodyWaterKind};

/// Publica os contratos de host do [`bevy_ext_audio`] a partir da engine.
///
/// - `AudioPaths` semeado do `GameConfig.assets.sfx_dir` (Startup — o config
///   é insertado no build da App e nunca muda a correr);
/// - [`publish_audio_world_state`] espelha `WeatherState.rain`,
///   `CurrentScene.id` e `CurrentBiome.id` por frame (Update);
/// - `AudioEar` inserido na câmara de editor (o `ear_position` do crate
///   prefere-a à primeira `Camera3d`, replicando o `active_camera_eye`
///   histórico).
pub struct AudioHostPlugin;

impl Plugin for AudioHostPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AudioPaths>()
            .init_resource::<AudioWorldState>()
            // O `SfxPlugin` do crate também a inicializa; aqui é para o
            // adaptador bastar-se sozinho (apps de teste sem o plugin).
            .init_resource::<HostWaterBodies>()
            .add_systems(Startup, sync_audio_paths)
            .add_systems(
                Update,
                (publish_audio_world_state, mark_audio_ear, publish_water_bodies)
                    // Os loops de ambiente leem o host no MESMO frame em que
                    // o terreno aparece (bootstrap) ou muda (edição viva).
                    .before(bevy_ext_audio::setup_water_ambience)
                    .before(bevy_ext_audio::setup_waterfall_ambience),
            );
    }
}

/// Semeia o [`AudioPaths`] do crate com o `assets.sfx_dir` do config. A
/// conversão é a mesma do `GameConfig::asset_string` (trim da barra final;
/// vazio = sem prefixo) — os paths de clips têm de ficar IDÊNTICOS aos que
/// o `resolve_loop_clip` histórico produzia.
pub fn sync_audio_paths(
    config: Option<Res<crate::config::GameConfig>>,
    mut paths: ResMut<AudioPaths>,
) {
    let Some(config) = config else {
        return;
    };
    let dir = config.assets.sfx_dir.to_string_lossy();
    paths.sfx_dir = dir.trim_end_matches('/').to_string();
}

/// Espelho por frame: o que era `Option<Res<CurrentScene>>`/`CurrentBiome`/
/// `WeatherState` nos sistemas da engine entra no [`AudioWorldState`] do
/// crate. `None` do recurso OU `id: None` viram `None` no espelho — a
/// ausência significa o mesmo ("fora de projeto" / vale central).
pub fn publish_audio_world_state(
    weather: Option<Res<crate::worldsys::WeatherState>>,
    scene: Option<Res<crate::scene_loader::CurrentScene>>,
    biome: Option<Res<crate::ambient::CurrentBiome>>,
    mut mirror: ResMut<AudioWorldState>,
) {
    mirror.rain = weather.map(|w| w.rain).unwrap_or(0.0);
    mirror.scene_id = scene.as_ref().and_then(|s| s.id.clone());
    mirror.biome_id = biome.as_ref().and_then(|b| b.id.clone());
}

/// A câmara de editor é o OUVIDO preferido do crate ([`AudioEar`]) — sem o
/// marker, `ear_position` devolve a primeira `Camera3d` (arbitrária com duas
/// câmaras no `--camera free`) e os loops de script atenuavam à câmara errada.
pub fn mark_audio_ear(
    mut commands: Commands,
    editors: Query<Entity, Added<crate::editor_camera::EditorCamera>>,
) {
    for entity in &editors {
        commands.entity(entity).insert(AudioEar);
    }
}

/// Snapshot dos corpos de água do terreno para o host do `bevy_ext_audio`.
///
/// O crate de áudio não conhece o terreno: consulta distância à linha de
/// água (loops de lago/rio) e as cachoeiras (loop próprio) por este contrato.
/// O `Viber` publicava-o no adaptador do `TerrainRuntime`; com o monólito
/// apagado, o adaptador é este (o terreno é um recurso do jogo).
#[derive(Debug)]
struct TerrainWaterBodies {
    bodies: Vec<WaterBody>,
    spots: Vec<WaterfallSpot>,
}

impl TerrainWaterBodies {
    /// Snapshot puro do registry de corpos (testável sem `TerrainRuntime`).
    fn from_bodies(bodies: &[WaterBody]) -> Self {
        let mut spots = Vec::new();
        for body in bodies {
            for cascade in body.cascades.iter().filter(|c| c.waterfall) {
                // `base` é a primeira estação DEPOIS da queda: o caldeirão.
                if let Some(at) = body.stations.get(cascade.base) {
                    spots.push(WaterfallSpot {
                        at: [at.x, at.y],
                        drop: cascade.drop,
                    });
                }
            }
        }
        Self {
            bodies: bodies.to_vec(),
            spots,
        }
    }
}

impl WaterBodies for TerrainWaterBodies {
    fn min_distance_to_waterline(&self, kind: WaterKind, x: f32, z: f32) -> Option<f32> {
        let p = Vec2::new(x, z);
        let wanted = match kind {
            WaterKind::Lake => BodyWaterKind::Lake,
            WaterKind::River => BodyWaterKind::River,
        };
        self.bodies
            .iter()
            .filter(|body| body.kind == wanted)
            // `distance_to_waterline` é NEGATIVA dentro da água (a linha de
            // água é o bordo); para o ganho do loop "estar dentro" é
            // distância zero, não negativa.
            .map(|body| body.distance_to_waterline(p).max(0.0))
            .filter(|distance| distance.is_finite())
            .reduce(f32::min)
    }

    fn waterfall_spots(&self) -> &[WaterfallSpot] {
        &self.spots
    }
}

/// Publica [`HostWaterBodies`] quando o `TerrainRuntime` entra ou muda
/// (bootstrap do mundo, edição viva do terreno). Sem terreno (interiores sem
/// terreno, menu antes do bake) limpa o host — os loops de água morrem com a
/// cena. A reconstrução é um clone do `Vec<WaterBody>` e só corre em
/// `Changed`; os `WaterBody`s são o registry do carve (já existia na engine).
pub fn publish_water_bodies(
    terrain: Option<Res<crate::terrain::runtime::TerrainRuntime>>,
    mut host: ResMut<HostWaterBodies>,
) {
    let Some(terrain) = terrain else {
        if host.0.is_some() {
            host.0 = None;
        }
        return;
    };
    if !terrain.is_changed() {
        return;
    }
    host.0 = Some(Arc::new(TerrainWaterBodies::from_bodies(&terrain.water)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headless_app() -> App {
        let mut app = App::new();
        // Só o adaptador: os plugins do crate (SfxPlugin/MusicPlugin) pedem
        // `Res<AssetServer>` (AssetPlugin) — o contrato deles é testado lá.
        app.add_plugins(MinimalPlugins).add_plugins(AudioHostPlugin);
        app
    }

    /// Sem config, o sfx_dir fica no default histórico do crate (os sistemas
    /// são no-ops, nenhum pânico — apps de teste headless).
    #[test]
    fn audio_host_without_config_keeps_default_paths() {
        let mut app = headless_app();
        app.update();
        let paths = app.world().resource::<AudioPaths>();
        assert_eq!(paths.sfx_path("combat/swing.ogg"), "assets/audio/sfx/combat/swing.ogg");
    }

    /// O espelho do mundo segue os recursos da engine: cena/bioma/chuva
    /// publicados, e `None` dos recursos → `None` no espelho.
    #[test]
    fn audio_world_state_mirrors_engine_resources() {
        let mut app = headless_app();
        app.insert_resource(crate::scene_loader::CurrentScene {
            root: std::path::PathBuf::from("/w"),
            id: Some("interiors/chapel".into()),
            path: std::path::PathBuf::from("/w/chapel.bsn"),
            title: None,
            entry: None,
            came_from: None,
            return_pose: None,
        });
        app.insert_resource(crate::ambient::CurrentBiome {
            id: Some("picos".into()),
        });
        app.insert_resource(crate::worldsys::WeatherState {
            wind: [0.0, 0.0],
            wind_strength: 0.0,
            clouds: 0.0,
            rain: 0.4,
            cycle: false,
        });
        app.update();
        let mirror = app.world().resource::<AudioWorldState>();
        assert_eq!(mirror.scene_id.as_deref(), Some("interiors/chapel"));
        assert_eq!(mirror.biome_id.as_deref(), Some("picos"));
        assert!((mirror.rain - 0.4).abs() < 1e-6);

        // Mundo fora de projeto: os recursos saem, o espelho limpa-se.
        app.world_mut().remove_resource::<crate::scene_loader::CurrentScene>();
        app.world_mut().remove_resource::<crate::ambient::CurrentBiome>();
        app.world_mut().remove_resource::<crate::worldsys::WeatherState>();
        app.update();
        let mirror = app.world().resource::<AudioWorldState>();
        assert_eq!(mirror.scene_id, None);
        assert_eq!(mirror.biome_id, None);
        assert_eq!(mirror.rain, 0.0);
    }

    /// O `AudioEar` chega à câmara de editor (o ouvido preferido do crate).
    #[test]
    fn editor_camera_becomes_audio_ear() {
        let mut app = headless_app();
        let editor = app
            .world_mut()
            .spawn(crate::editor_camera::EditorCamera)
            .id();
        app.update();
        assert!(app.world().entity(editor).contains::<AudioEar>());
    }

    /// O adaptador responde à linha de água (lago/rio) e extrai as
    /// cachoeiras do registry do carve.
    #[test]
    fn water_adapter_answers_distance_and_waterfalls() {
        use bevy_ext_water::body::{CascadeInfo, LakeShape};
        // Forma circular (`LakeShape::default`) para as distâncias serem
        // exactas — a forma orgânica do `new` tem harmónicos por posição.
        let at = Vec2::new(10.0, 0.0);
        let lake = WaterBody {
            kind: BodyWaterKind::Lake,
            at,
            radius: 5.0,
            carve_radius: 6.25,
            hull: 0.0,
            water_y: 0.0,
            mirror_reach: 1.0,
            shape: LakeShape::default(),
            stations: Vec::new(),
            surface_y: Vec::new(),
            water_width: 0.0,
            half_width: Vec::new(),
            depths: Vec::new(),
            cascades: Vec::new(),
        };
        let river = WaterBody {
            kind: BodyWaterKind::River,
            at: Vec2::ZERO,
            radius: 0.0,
            carve_radius: 3.0,
            hull: 40.0,
            water_y: 0.0,
            mirror_reach: 1.0,
            shape: LakeShape::default(),
            stations: vec![Vec2::new(0.0, 0.0), Vec2::new(0.0, 20.0)],
            surface_y: vec![4.0, 0.0],
            water_width: 4.0,
            half_width: Vec::new(),
            depths: Vec::new(),
            cascades: vec![CascadeInfo {
                lip: 0,
                base: 1,
                drop: 4.0,
                waterfall: true,
                wall: false,
                top_y: 4.0,
                bot_y: 0.0,
            }],
        };
        let only_lake = TerrainWaterBodies::from_bodies(std::slice::from_ref(&lake));
        // Dentro do lago = distância zero (a métrica do carve é negativa na
        // água; o loop de ambiente quer "estar dentro", não o negativo).
        assert_eq!(
            only_lake.min_distance_to_waterline(WaterKind::Lake, 10.0, 0.0),
            Some(0.0)
        );
        assert_eq!(
            only_lake.min_distance_to_waterline(WaterKind::Lake, 20.0, 0.0),
            Some(5.0)
        );
        // Sem rio nenhum: `None` — o loop do rio fica mudo (não a zeros).
        assert_eq!(
            only_lake.min_distance_to_waterline(WaterKind::River, 20.0, 0.0),
            None
        );

        let both = TerrainWaterBodies::from_bodies(&[lake, river]);
        // Cachoeira = caldeirão na estação `base`, com a queda.
        assert_eq!(
            both.waterfall_spots(),
            &[WaterfallSpot {
                at: [0.0, 20.0],
                drop: 4.0
            }]
        );
        // Rio a 20 m, meia-largura 2 m (contrato antigo: `water_width/2`).
        assert_eq!(
            both.min_distance_to_waterline(WaterKind::River, 20.0, 0.0),
            Some(18.0)
        );
    }
}

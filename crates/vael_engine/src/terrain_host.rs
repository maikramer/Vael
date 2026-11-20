//! Glue IR de cena → configs de shader do terreno (fase D de
//! `docs/DELETE-MONOLITH.md`).
//!
//! O runtime de terreno vive no [`bevy_ext_terrain_runtime`]; os paths
//! `vael_engine::terrain::…` resolvem pelo shim (`pub use … as terrain`). O que
//! fica aqui é o que NÃO pode ir para o crate: o walk do IR declarativo
//! ([`crate::recipes::EntitySpec`]) que lê `<DayCycle>`, `<Weather>`,
//! `<Lake>` e a `<HudElement tag="sky">` para configurar a especialização
//! dos shaders — o IR de parsing não é dependência do crate.
//!
//! O `app.rs` do Vael cita `vael_engine::terrain_host::{…}` no arranque
//! (`shaders/{water,terrain_chunk}.wesl` são reescritos por mundo).

pub use bevy_ext_terrain_runtime::layer_material::TerrainChunkConfig;
pub use bevy_ext_terrain_runtime::water_material::WaterSurfaceConfig;

use crate::recipes::{EntityKind, EntitySpec};

/// Ponte IR → [`PendingTerrain`](bevy_ext_terrain_runtime::runtime::PendingTerrain)
/// do crate — o `TODO(viber-exit)` que a extração deixou: o bootstrap do
/// runtime extraído não conhece o IR de parsing, por isso lê a resource
/// [`TerrainCollect`](bevy_ext_terrain_runtime::runtime::TerrainCollect) com
/// ESTE hook. Os dois caminhos do bootstrap antigo:
///
/// * **pós-spawn** (o `bootstrap` no `Startup`/`SceneLoad`, depois do
///   `spawn::startup`): o [`crate::recipes::spawn::PendingTerrain`] que o
///   `spawn_parsed` publica (o antigo era `remove_resource` — o hook
///   consume igual);
/// * **pré-spawn** (o `preload_terrain_textures` no `PreStartup`, antes do
///   `spawn::startup` consumir o [`crate::recipes::spawn::PendingWorld`]):
///   coleciona do IR com os offsets de grupo acumulados
///   (`collect_terrain`).
fn collect_from_spawn(
    world: &mut bevy::prelude::World,
    out: &mut bevy_ext_terrain_runtime::runtime::PendingTerrain,
) {
    if let Some(pending) = world.remove_resource::<crate::recipes::spawn::PendingTerrain>() {
        out.base_dir = pending.base_dir;
        out.terrain = pending.terrain;
        out.features = pending.features;
        return;
    }
    let Some(pending) = world.get_resource::<crate::recipes::spawn::PendingWorld>() else {
        return;
    };
    out.base_dir = pending.base_dir.clone();
    let mut collected = crate::recipes::spawn::PendingTerrain::default();
    crate::recipes::spawn::collect_terrain(&pending.world.entities, &mut collected);
    out.terrain = collected.terrain;
    out.features = collected.features;
}

/// Regista a ponte em cima. Chamar em TODA a app que corre o
/// `terrain::runtime::bootstrap` (o jogo em `app.rs`, o bake headless) — sem
/// ela o bootstrap devolve cedo e o mundo fica sem terreno.
pub fn install_terrain_collect(app: &mut bevy::app::App) {
    app.insert_resource(bevy_ext_terrain_runtime::runtime::TerrainCollect(Some(
        collect_from_spawn,
    )));
}

/// Publica o recorte do `config.yaml` que o runtime de terreno lê
/// ([`TerrainWorldAssets`](bevy_ext_terrain_runtime::host::TerrainWorldAssets))
/// — o `TODO(viber-exit)` do host.
///
/// Sem esta resource o runtime resolve as texturas das layers contra a pasta
/// do mundo (`grass/albedo.ktx2`, sem o `assets/textures/`): cada layer pedia
/// um ficheiro inexistente (`ERROR Path not found` do Bevy) e caía no
/// fallback — o chão desenhava com a textura errada. Os mapas de detalhe
/// (`normal`/`height`/`ao`) ainda passavam pelo `asset_present`, que os
/// descartava em silêncio.
///
/// Chamar ao lado de [`install_terrain_collect`], com o `config` já carregado
/// e a pasta do JOGO (não a da cena).
pub fn install_world_assets(
    app: &mut bevy::app::App,
    config: &crate::config::GameConfig,
    world_dir: &std::path::Path,
) {
    app.insert_resource(bevy_ext_terrain_runtime::host::TerrainWorldAssets {
        // `asset_roots` já devolve a pasta do jogo primeiro; o
        // `TerrainWorldAssets::asset_roots` volta a pô-la e dedupa.
        roots: config.asset_roots(world_dir),
        textures_dir: config.assets.terrain_textures_dir.clone(),
    });
}

/// Configuração do shader de água a partir do mundo declarativo (dia/clima,
/// amplitude das ondas e sol estático).
pub fn water_surface_config_from_world(entities: &[EntitySpec]) -> WaterSurfaceConfig {
    let mut config = WaterSurfaceConfig::default();
    walk_entities(entities, &mut config);
    config
}

fn walk_entities(specs: &[EntitySpec], config: &mut WaterSurfaceConfig) {
    for spec in specs {
        match &spec.kind {
            EntityKind::DayCycle {
                minute_of_day,
                minutes_per_real_second,
                dawn_minute,
                dusk_minute,
                max_sun_elevation,
                sun_azimuth_base,
                min_sun_elevation,
                ..
            } => {
                config.drive = true;
                config.clock_start = *minute_of_day;
                config.clock_speed = *minutes_per_real_second;
                config.dawn = *dawn_minute;
                config.dusk = *dusk_minute;
                config.max_elev = *max_sun_elevation;
                config.min_elev = *min_sun_elevation;
                config.az_base = *sun_azimuth_base;
            }
            EntityKind::Weather {
                wind,
                wind_strength,
                ..
            } => {
                config.wind = *wind;
                config.wind_strength = *wind_strength;
            }
            EntityKind::Lake { spec } => {
                config.wave_amp = config.wave_amp.max(spec.ripple.clamp(0.2, 2.0));
            }
            EntityKind::HudElement { tag, attrs } if tag == "sky" => {
                let f = |key: &str| {
                    attrs
                        .iter()
                        .find(|(k, _)| k == key)
                        .and_then(|(_, v)| v.parse::<f32>().ok())
                        .filter(|v| v.is_finite())
                };
                if let Some(v) = f("sun-elevation") {
                    config.sun_elevation = v;
                }
                if let Some(v) = f("sun-azimuth") {
                    config.sun_azimuth = v;
                }
            }
            _ => {}
        }
        walk_entities(&spec.children, config);
    }
}

/// Config do shader de chunk do primeiro `<Terrain>` com `layers` (o mesmo
/// spec que o bootstrap assa splats para). `None` sem camadas.
pub fn terrain_chunk_config_from_world(entities: &[EntitySpec]) -> Option<TerrainChunkConfig> {
    fn first_layered_terrain(
        specs: &[EntitySpec],
    ) -> Option<&bevy_ext_terrain_runtime::spec::TerrainSpec> {
        for spec in specs {
            if let EntityKind::Terrain { spec } = &spec.kind
                && !spec.layers.is_empty()
            {
                return Some(spec);
            }
            if let Some(found) = first_layered_terrain(&spec.children) {
                return Some(found);
            }
        }
        None
    }
    let spec = first_layered_terrain(entities)?;
    Some(TerrainChunkConfig::from_terrain_spec(spec))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::PhysicsSpec;
    use crate::recipes::TransformSpec;
    use crate::worldsys::DayCycleMode;
    use bevy_ext_terrain_runtime::spec::TerrainSpec;

    fn spec(kind: EntityKind) -> EntitySpec {
        EntitySpec {
            prototype_instance: None,
            name: None,
            tag: None,
            script: None,
            props: Vec::new(),
            links: Default::default(),
            transform: TransformSpec::default(),
            physics: PhysicsSpec::default(),
            kind,
            children: Vec::new(),
            source: None,
        }
    }

    fn with_child(mut parent: EntitySpec, child: EntitySpec) -> EntitySpec {
        parent.children.push(child);
        parent
    }

    /// `<DayCycle>`/`<Weather>`/`<Lake>` preenchem a config com que o shader
    /// é especializado; sem eles ficam os defaults da crate.
    #[test]
    fn water_surface_config_reads_the_declarative_world() {
        let world = vec![with_child(
            with_child(
                spec(EntityKind::Group),
                spec(EntityKind::DayCycle {
                    minute_of_day: 1140.0,
                    minutes_per_real_second: 3.0,
                    dawn_minute: 330.0,
                    dusk_minute: 1170.0,
                    ambient_day: 0.26,
                    ambient_night: 0.07,
                    drive_ambient: true,
                    max_sun_elevation: 55.0,
                    sun_azimuth_base: 180.0,
                    min_sun_elevation: 8.0,
                    day_speed_scale: 1.0,
                    night_speed_scale: 1.0,
                    night_cap_minute: 0.0,
                    mode: DayCycleMode::Continuous,
                    day_minute: 720.0,
                    night_minute: 1320.0,
                    transition_seconds: 4.0,
                    shadow_refresh_hz: 0.0,
                }),
            ),
            spec(EntityKind::Weather {
                wind: [1.0, 0.0],
                wind_strength: 3.0,
                clouds: 0.25,
                rain: 0.0,
                cycle: true,
            }),
        )];
        let config = water_surface_config_from_world(&world);
        assert!(config.drive);
        assert_eq!(config.clock_start, 1140.0);
        assert_eq!(config.max_elev, 55.0);
        assert_eq!(config.min_elev, 8.0);
        assert_eq!(config.wind, [1.0, 0.0]);
        assert_eq!(config.wind_strength, 3.0);
        let bare = water_surface_config_from_world(&[spec(EntityKind::Group)]);
        assert!(!bare.drive);
        assert_eq!(bare.wind_strength, 1.5);
    }

    /// O primeiro `<Terrain>` com camadas (incl. filhos de grupo) dá a
    /// config; sem camadas não há config.
    #[test]
    fn terrain_chunk_config_reads_the_first_layered_terrain() {
        let terrain = |layers: Vec<String>| {
            spec(EntityKind::Terrain {
                spec: TerrainSpec {
                    world_size: 512.0,
                    layers,
                    ..TerrainSpec::default()
                },
            })
        };
        let nested = with_child(spec(EntityKind::Group), terrain(vec!["grass".into()]));
        let config = terrain_chunk_config_from_world(&[spec(EntityKind::Group), nested]);
        assert!(config.expect("layers found").tri_slope > 0.0);

        let bare = terrain_chunk_config_from_world(&[spec(EntityKind::Group)]);
        assert!(bare.is_none(), "no layers → no config");
    }
}

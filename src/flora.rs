//! Árvores procedurais (`bevy_flora`) plantadas nas estações do mundo.
//!
//! O mundo deixou de desenhar árvores GLB. Cada árvore autorada é uma
//! **estação**: um `<Entity>` com a pose, o collider de tronco (cilindro) e os
//! props de colheita (`destructible`/`resource-kind`/…) mais
//! `prop-flora="<espécie>"`. Este módulo lê as estações e transforma cada uma
//! na árvore da espécie (`PooledTree` **na própria entidade**, não num filho):
//! a árvore herda a pose/escala autoradas, e o `bevy_flora` pode tomar conta
//! dela directamente — tombar (`FellTree`), abanar (`TreeShakes`) e recolorir.
//!
//! A altura do GLB autorado é absorvida no **pool** (o preset da crate tem
//! outra altura): as árvores nascem à escala certa com a escala da estação.
//!
//! Vida visual: dois looks por pool (0 = cores da espécie, 1 = frio para a
//! altitude), folhas a cair nas rajadas e no embate das árvores cortadas, e
//! `SeeThroughTrees` na câmara do jogo.
//!
//! Gates:
//! * `VAEL_FLORA=0` desliga o plantio (as estações ficam vazias, sem visual).
//! * `VAEL_FLORA_GROVE=0` desliga só a mata autoral da entrada norte.
//! * `VAEL_FLORA_SHADOW_RADIUS=<m>` muda o raio de sombra (default 90).
//! * `VAEL_FLORA_IMPOSTOR=<m>` muda a distância do impostor (default 260;
//!   `0` desliga). O guard do `water_ssr` no Viber (2026-11-12) permite-o.

use std::time::Instant;

use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy_flora::prelude::*;
use viber::luau::ScriptProps;
use viber::terrain::runtime::TerrainRuntime;
use viber::worldsys::WeatherState;

use crate::components::VaelPlayer;
use crate::settings::{SceneSetting, SceneSettingList};
use bevy_ext_flora::{FloraSettings, FloraSpecies};

use std::collections::{BTreeSet, HashMap};

/// Quantas estações plantar por frame (o mundo tem 1634; um frame só seria um
/// hitch de spawn de entidades).
const BATCH: usize = 192;

// LOD (45/110 m), impostor (260 m), raio de sombra (90 m) e a altura do look
// frio (55 m) vivem em `FloraSettings` (`world/settings/flora.bsn`); as
// espécies em `FloraSpecies` na mesma cena. Os `VAEL_FLORA_*` ganham (QA).

/// Um pool por espécie: preset, nº de variantes, tom e o fator de altura que
/// leva o preset (altura em metros) à altura do GLB autorado.
struct PoolKind {
    key: &'static str,
    preset: TreePreset,
    variants: u32,
    leaves: Color,
    wood: Color,
    /// Árvore morta: o preset gera só o esqueleto (sem folhas). O salgueiro
    /// do pântano (`dead_willow`) é um chorão sem folha.
    bare: bool,
    /// `altura_glb / altura_preset`: aplicado a tronco, raio e folha ao
    /// construir o pool.
    height_scale: f32,
}

const POOLS: &[PoolKind] = &[
    PoolKind {
        key: "pine_dark",
        preset: TreePreset::Pine,
        variants: 8,
        leaves: Color::srgb(0.11, 0.23, 0.12),
        wood: Color::srgb(0.19, 0.14, 0.09),
        bare: false,
        height_scale: 9.0 / 8.0,
    },
    PoolKind {
        key: "tree_pine",
        preset: TreePreset::Pine,
        variants: 6,
        leaves: Color::srgb(0.21, 0.36, 0.17),
        wood: Color::srgb(0.26, 0.19, 0.12),
        bare: false,
        height_scale: 9.0 / 8.0,
    },
    PoolKind {
        key: "tree_oak",
        preset: TreePreset::Oak,
        variants: 8,
        leaves: Color::srgb(0.19, 0.33, 0.10),
        wood: Color::srgb(0.32, 0.21, 0.11),
        bare: false,
        height_scale: 8.0 / 6.0,
    },
    PoolKind {
        key: "tree_birch",
        preset: TreePreset::Birch,
        variants: 6,
        leaves: Color::srgb(0.33, 0.48, 0.19),
        wood: Color::srgb(0.72, 0.70, 0.62),
        bare: false,
        height_scale: 8.0 / 7.0,
    },
    PoolKind {
        key: "dead_tree",
        preset: TreePreset::DeadTree,
        variants: 5,
        leaves: Color::srgb(0.20, 0.18, 0.14),
        wood: Color::srgb(0.17, 0.14, 0.11),
        bare: false,
        height_scale: 1.0,
    },
    PoolKind {
        key: "dead_willow",
        preset: TreePreset::Willow,
        variants: 6,
        leaves: Color::srgb(0.24, 0.24, 0.18),
        wood: Color::srgb(0.19, 0.18, 0.15),
        bare: true,
        height_scale: 5.96 / 5.0,
    },
    PoolKind {
        key: "palm_tree",
        preset: TreePreset::Palm,
        variants: 3,
        leaves: Color::srgb(0.24, 0.40, 0.16),
        wood: Color::srgb(0.31, 0.24, 0.15),
        bare: false,
        height_scale: 1.0,
    },
];

/// Mata autoral da entrada norte: árvores que não existiam no mundo, ao lado
/// do portão (posições relativas ao centro do portão).
struct GroveTree {
    kind: &'static str,
    x: f32,
    dz: f32,
    scale: f32,
    yaw_deg: f32,
}

const GROVE: &[GroveTree] = &[
    GroveTree {
        kind: "tree_oak",
        x: -12.2,
        dz: 3.0,
        scale: 1.35,
        yaw_deg: 24.0,
    },
    GroveTree {
        kind: "tree_birch",
        x: -16.4,
        dz: 5.3,
        scale: 1.15,
        yaw_deg: -38.0,
    },
    GroveTree {
        kind: "tree_oak",
        x: -20.8,
        dz: 2.8,
        scale: 1.28,
        yaw_deg: 152.0,
    },
    GroveTree {
        kind: "pine_dark",
        x: -25.6,
        dz: 5.7,
        scale: 1.0,
        yaw_deg: 66.0,
    },
    GroveTree {
        kind: "tree_oak",
        x: 12.4,
        dz: 3.3,
        scale: 1.32,
        yaw_deg: -12.0,
    },
    GroveTree {
        kind: "tree_birch",
        x: 16.8,
        dz: 5.4,
        scale: 1.12,
        yaw_deg: 88.0,
    },
    GroveTree {
        kind: "tree_oak",
        x: 21.2,
        dz: 2.9,
        scale: 1.3,
        yaw_deg: 205.0,
    },
    GroveTree {
        kind: "pine_dark",
        x: 26.0,
        dz: 5.8,
        scale: 1.05,
        yaw_deg: -74.0,
    },
];

/// Cota do chão do portão norte (sanidade da cena `main`).
const GATE_FLOOR_Y: f32 = 38.730545;
const GATE_ANCHOR: Vec2 = Vec2::new(0.0, 38.0);

/// Estação já plantada (a árvore vive na própria entidade).
#[derive(Component)]
struct FloraPlanted;

/// Raiz da mata autoral (para a limpar numa troca de cena).
#[derive(Component)]
struct FloraGroveRoot;

/// Pools por espécie, construídos uma vez por sessão (e reconstruídos quando
/// as espécies mudam no `world/settings/flora.bsn`).
#[derive(Resource, Default)]
struct FloraPools {
    pools: HashMap<String, Handle<TreePool>>,
    /// Variantes de cada pool (a variante da estação é `hash % variants`).
    variants: HashMap<String, u32>,
    /// Parâmetros de cada espécie (o `footing` da mata autoral usa-os).
    params: HashMap<String, TreeParams>,
    /// Altura do look frio (de `FloraSettings`).
    cold_look_y: f32,
    /// `prop-flora` desconhecidos (logar uma vez, não por instância).
    warned: BTreeSet<String>,
    planted: usize,
}

pub struct VaelFloraPlugin;

impl Plugin for VaelFloraPlugin {
    fn build(&self, app: &mut App) {
        // Depois dos `DefaultPlugins` (materiais/renderer), que o `build_app`
        // já pôs: sem isso não há sombreamento de folha nem impostores.
        app.add_plugins(FloraPlugin)
            .init_resource::<FloraPools>()
            .init_resource::<FallingLeaves>()
            .add_systems(Startup, build_pools)
            .add_systems(Update, rebuild_pools_on_edit)
            .add_systems(
                Update,
                (
                    install_ground,
                    plant_stations,
                    plant_grove,
                    drive_weather,
                    drop_leaves_on_gust,
                    see_through_trees,
                )
                    .chain(),
            );
    }
}

fn env_on(name: &str) -> bool {
    !matches!(
        std::env::var(name).ok().as_deref(),
        Some("0" | "false" | "off")
    )
}

fn shadow_radius_or(default: f32) -> f32 {
    std::env::var("VAEL_FLORA_SHADOW_RADIUS")
        .ok()
        .and_then(|raw| raw.parse().ok())
        .filter(|radius: &f32| *radius > 0.0)
        .unwrap_or(default)
}

/// Distância do impostor: `VAEL_FLORA_IMPOSTOR` (m; `0` desliga), senão
/// o da cena.
fn impostor_distance_or(default: f32) -> f32 {
    match std::env::var("VAEL_FLORA_IMPOSTOR")
        .ok()
        .and_then(|raw| raw.trim().parse::<f32>().ok())
    {
        Some(distance) if distance <= 0.0 => f32::INFINITY,
        Some(distance) => distance,
        None => default,
    }
}

fn lod_settings(settings: &FloraSettings) -> LodSettings {
    LodSettings {
        distances: LodDistances {
            lod1: settings.lod1_distance,
            lod2: settings.lod2_distance,
            impostor: impostor_distance_or(settings.impostor_distance),
        },
        ..default()
    }
}

/// As espécies de antes da cena (`POOLS`): servem para exportar o
/// `world/settings/flora.bsn` inicial e de reserva se a cena não tiver
/// nenhuma.
pub(crate) fn legacy_species() -> Vec<FloraSpecies> {
    POOLS
        .iter()
        .map(|pool| FloraSpecies {
            key: pool.key.to_string(),
            variants: pool.variants,
            cold_look: true,
            params: pool_params(pool),
        })
        .collect()
}

/// As espécies da cena, ou as de reserva.
fn species_catalog(list: Option<&SceneSettingList<FloraSpecies>>) -> Vec<FloraSpecies> {
    match list {
        Some(list) if !list.0.is_empty() => list.0.clone(),
        _ => legacy_species(),
    }
}

#[cfg(test)]
fn pool_of(key: &str) -> Option<&'static PoolKind> {
    POOLS.iter().find(|pool| pool.key == key)
}

/// Desvio do tom para o look frio (altitude): mais azul, menos saturação.
fn cold_tint(color: Color) -> Color {
    let c = color.to_srgba();
    Color::srgb(
        (c.red * 0.86).min(1.0),
        (c.green * 0.93).min(1.0),
        (c.blue * 1.12).min(1.0),
    )
}

/// Parâmetros do preset com o tom da espécie e a altura do GLB autorado
/// absorvida. Usado pelo pool e pelo `footing` da mata autoral.
fn pool_params(pool: &PoolKind) -> TreeParams {
    let mut params = pool.preset.params();
    params.foliage.color = pool.leaves;
    params.trunk_color = pool.wood;
    if pool.bare {
        params.foliage.enabled = false;
    }
    // Escala uniforme: o comprimento dos ramos deriva da altura do tronco.
    let factor = pool.height_scale;
    params.trunk.height *= factor;
    params.trunk.radius *= factor;
    params.foliage.leaf_size *= factor;
    params
}

/// Constrói os pools das espécies (uma vez), com o look frio a mais.
///
/// `build_pool` (e não `BackgroundPools`) porque `add_look` só existe no
/// `TreePool` antes de ir para os assets: o segundo look partilha os meshes e
/// custa só dois materiais. É trabalho de arranque uma vez (logado em ms).
fn build_pools(
    mut commands: Commands,
    mut state: ResMut<FloraPools>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut tree_pools: ResMut<Assets<TreePool>>,
    species: Option<Res<SceneSettingList<FloraSpecies>>>,
    settings: Option<Res<SceneSetting<FloraSettings>>>,
) {
    let settings = settings.map(|s| s.0.clone()).unwrap_or_default();
    commands.insert_resource(FloraShadows {
        radius: shadow_radius_or(settings.shadow_radius),
    });
    if !env_on("VAEL_FLORA") || !state.pools.is_empty() {
        return;
    }
    let species = species_catalog(species.as_deref());
    fill_pools(
        &mut state,
        &species,
        &settings,
        &mut meshes,
        &mut tree_pools,
    );
}

/// Constrói (ou substitui no mesmo handle) o pool de cada espécie.
fn fill_pools(
    state: &mut FloraPools,
    species: &[FloraSpecies],
    settings: &FloraSettings,
    meshes: &mut Assets<Mesh>,
    tree_pools: &mut Assets<TreePool>,
) {
    let started = Instant::now();
    let lod = lod_settings(settings);
    state.cold_look_y = settings.cold_look_y;
    for species in species {
        if species.key.is_empty() {
            continue;
        }
        let variants = species.variants.max(1);
        let mut pool = build_pool(&species.params, variants, f32::INFINITY, &lod, &mut *meshes);
        // Look 0 = cores da espécie; look 1 = frio das serras/planaltos.
        let (wood, leaves) = (species.params.trunk_color, species.params.foliage.color);
        pool.add_look(if species.cold_look {
            PoolColors {
                wood: cold_tint(wood),
                leaves: cold_tint(leaves),
                unlit: false,
            }
        } else {
            PoolColors {
                wood,
                leaves,
                unlit: false,
            }
        });
        match state.pools.get(&species.key) {
            Some(handle) => {
                let _ = tree_pools.insert(handle.id(), pool);
            }
            None => {
                let handle = tree_pools.add(pool);
                state.pools.insert(species.key.clone(), handle);
            }
        }
        state.variants.insert(species.key.clone(), variants);
        state
            .params
            .insert(species.key.clone(), species.params.clone());
    }
    info!(
        pools = state.pools.len(),
        variants = species.iter().map(|species| species.variants).sum::<u32>(),
        build_ms = started.elapsed().as_secs_f64() * 1000.0,
        impostor = lod.distances.impostor,
        "flora: pools das espécies construídos"
    );
}

/// Gravar o `flora.bsn` no editor reconstrói os pools no mesmo handle: as
/// árvores plantadas seguem a edição.
fn rebuild_pools_on_edit(
    mut state: ResMut<FloraPools>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut tree_pools: ResMut<Assets<TreePool>>,
    species: Option<Res<SceneSettingList<FloraSpecies>>>,
    settings: Option<Res<SceneSetting<FloraSettings>>>,
    mut shadows: Option<ResMut<FloraShadows>>,
) {
    if state.pools.is_empty() {
        return;
    }
    let species_changed = species
        .as_ref()
        .is_some_and(|s| s.is_changed() && !s.is_added());
    let settings_changed = settings
        .as_ref()
        .is_some_and(|s| s.is_changed() && !s.is_added());
    if !species_changed && !settings_changed {
        return;
    }
    let settings = settings.map(|s| s.0.clone()).unwrap_or_default();
    let catalog = species_catalog(species.as_deref());
    fill_pools(
        &mut state,
        &catalog,
        &settings,
        &mut meshes,
        &mut tree_pools,
    );
    if let Some(shadows) = shadows.as_mut() {
        shadows.radius = shadow_radius_or(settings.shadow_radius);
    }
}

/// `hash` estável do nome da instância (escolhe a variante do pool).
fn variant_of(name: Option<&Name>, variants: u32) -> u32 {
    let Some(name) = name else {
        return 0;
    };
    let mut hash: u32 = 0x811C_9DC5;
    for byte in name.as_str().bytes() {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash % variants.max(1)
}

/// Põe a árvore do pool **na própria entidade** (a estação), com o look frio
/// acima de `FloraSettings::cold_look_y`. Devolve `false` se o pool ainda não existe.
fn plant_on_entity(
    commands: &mut Commands,
    state: &FloraPools,
    entity: Entity,
    pool_key: &str,
    variant: u32,
    cold: bool,
) -> bool {
    let Some(handle) = state.pools.get(pool_key) else {
        return false;
    };
    let variants = state.variants.get(pool_key).copied().unwrap_or(1);
    let mut tree = commands.entity(entity);
    tree.insert((
        PooledTree {
            pool: handle.clone(),
            variant: variant % variants.max(1),
        },
        FloraPlanted,
        Visibility::Inherited,
    ));
    if cold {
        tree.insert(PooledLook(1));
    }
    true
}

/// Lê as estações do mundo (`prop-flora`) e planta as árvores que faltam.
fn plant_stations(
    mut commands: Commands,
    stations: Query<(Entity, &ScriptProps, Option<&Name>, &Transform), Without<FloraPlanted>>,
    mut state: ResMut<FloraPools>,
) {
    if !env_on("VAEL_FLORA") {
        return;
    }
    let mut planted = 0;
    for (entity, props, name, transform) in &stations {
        let prop = props
            .0
            .iter()
            .find(|(key, _)| key == "flora")
            .map(|(_, value)| value.as_str());
        let Some(prop) = prop else {
            continue;
        };
        if state.pools.is_empty() {
            return;
        }
        if !state.pools.contains_key(prop) {
            if state.warned.insert(prop.to_string()) {
                warn!(
                    prop,
                    "flora: `prop-flora` sem espécie no world/settings/flora.bsn"
                );
            }
            continue;
        }
        let variant = variant_of(name, u32::MAX);
        let cold = transform.translation.y > state.cold_look_y;
        if plant_on_entity(&mut commands, &state, entity, prop, variant, cold) {
            planted += 1;
            if planted >= BATCH {
                break;
            }
        }
    }
    if planted > 0 {
        state.planted += planted;
        debug!(planted, total = state.planted, "flora: estações plantadas");
    }
}

/// O chão da crate (`FloraGround`) segue o `TerrainRuntime`: é o que o felling
/// usa para cortar acima do chão e assentar o tronco. Reinstalado quando a
/// cena troca (a engine remove/reinsere o runtime).
fn install_ground(
    mut commands: Commands,
    terrain: Option<Res<TerrainRuntime>>,
    existing: Option<Res<FloraGround>>,
) {
    let Some(terrain) = terrain else {
        return;
    };
    if existing.is_some() && !terrain.is_changed() {
        return;
    }
    let reader = terrain.reader();
    commands.insert_resource(FloraGround::new(move |at: Vec2| reader.sample(at.x, at.y)));
}

/// A mata autoral da entrada norte (posições relativas ao portão). Vive na
/// cena `main` e é replantada quando a cena volta.
#[allow(clippy::too_many_arguments)]
fn plant_grove(
    mut commands: Commands,
    current: Option<Res<viber::scene_loader::CurrentScene>>,
    terrain: Option<Res<TerrainRuntime>>,
    ground: Option<Res<FloraGround>>,
    mut state: ResMut<FloraPools>,
    roots: Query<(), With<FloraGroveRoot>>,
    mut grove: Local<Option<Entity>>,
) {
    if !env_on("VAEL_FLORA") || !env_on("VAEL_FLORA_GROVE") {
        return;
    }
    let Some(current) = current else {
        return;
    };
    let in_main = current.id.as_deref() == Some("main");
    let root_alive = grove.is_some_and(|root| roots.contains(root));
    if root_alive && in_main {
        return;
    }
    if let Some(root) = grove.take()
        && roots.contains(root)
    {
        commands.entity(root).despawn();
    }
    if !in_main {
        return;
    }
    let (Some(terrain), Some(ground)) = (terrain, ground) else {
        return;
    };
    let probe = terrain.sample(GATE_ANCHOR.x, GATE_ANCHOR.y + 3.0);
    if (probe - GATE_FLOOR_Y).abs() > 3.0 {
        warn!(
            probe,
            "flora: chão do portão fora do esperado; mata não plantada"
        );
        return;
    }
    let root = commands
        .spawn((
            FloraGroveRoot,
            Name::new("flora:entrada-norte"),
            Transform::default(),
            Visibility::Inherited,
        ))
        .id();
    grove.replace(root);

    for (index, tree) in GROVE.iter().enumerate() {
        let Some(params) = state.params.get(tree.kind).cloned() else {
            continue;
        };
        let at = Vec2::new(GATE_ANCHOR.x + tree.x, GATE_ANCHOR.y + tree.dz);
        let foot = ground.footing(at, params.base_radius() * tree.scale);
        let entity = commands
            .spawn((
                Transform::from_xyz(at.x, foot, at.y)
                    .with_rotation(Quat::from_rotation_y(tree.yaw_deg.to_radians()))
                    .with_scale(Vec3::splat(tree.scale)),
                Visibility::Inherited,
                Name::new(format!("flora:mata:{index}")),
                ChildOf(root),
            ))
            .id();
        if plant_on_entity(
            &mut commands,
            &state,
            entity,
            tree.kind,
            index as u32,
            false,
        ) {
            state.planted += 1;
        }
    }
}

/// O vento e a chuva da engine dirigem a crate: a mesma tempestade que move a
/// relva `.vinst` dobra as copas procedurais.
fn drive_weather(
    weather: Option<Res<WeatherState>>,
    wind: Option<ResMut<FloraWind>>,
    wetness: Option<ResMut<FloraWetness>>,
) {
    let Some(weather) = weather else {
        return;
    };
    if !weather.is_changed() {
        return;
    }
    if let Some(mut wind) = wind {
        let direction = Vec2::new(weather.wind[0], weather.wind[1]);
        wind.direction = if direction.length_squared() > 1e-6 {
            direction.normalize()
        } else {
            Vec2::X
        };
        wind.strength = weather.wind_strength.clamp(0.0, 1.5);
    }
    if let Some(mut wetness) = wetness {
        wetness.0 = (weather.rain * 1.6).clamp(0.0, 1.0);
    }
}

/// Numa rajada forte, as árvores perto do herói largam algumas folhas
/// (`DropLeaves`); o `FallingLeaves` da crate trata da queda e do repouso.
fn drop_leaves_on_gust(
    weather: Option<Res<WeatherState>>,
    player: Query<&GlobalTransform, With<VaelPlayer>>,
    trees: Query<(Entity, &GlobalTransform), With<PooledTree>>,
    mut drops: MessageWriter<DropLeaves>,
    time: Res<Time>,
    mut cooldown: Local<f32>,
) {
    *cooldown = (*cooldown - time.delta_secs()).max(0.0);
    let Some(weather) = weather else {
        return;
    };
    if !weather.is_changed() || weather.wind_strength < 0.7 || *cooldown > 0.0 {
        return;
    }
    let Some(chest) = player.iter().next().map(GlobalTransform::translation) else {
        return;
    };
    let mut dropped = 0;
    for (entity, transform) in &trees {
        if transform.translation().distance_squared(chest) > 60.0 * 60.0 {
            continue;
        }
        drops.write(DropLeaves {
            tree: entity,
            count: 6,
        });
        dropped += 1;
        if dropped >= 6 {
            break;
        }
    }
    if dropped > 0 {
        *cooldown = 2.0;
    }
}

/// `SeeThroughTrees` na câmara do jogo: as árvores entre a câmara e o peito
/// do herói fazem dither em vez de o taparem. Só a câmara da janela (as do
/// bake de impostores são `RenderTarget::Image`).
fn see_through_trees(
    mut commands: Commands,
    cameras: Query<(Entity, &Camera, &RenderTarget, Option<&SeeThroughTrees>), With<Camera3d>>,
    player: Query<&GlobalTransform, With<VaelPlayer>>,
) {
    let Some(chest) = player
        .iter()
        .next()
        .map(|transform| transform.translation() + Vec3::Y * 1.2)
    else {
        return;
    };
    for (entity, camera, target, existing) in &cameras {
        if !camera.is_active || !matches!(target, RenderTarget::Window(_)) {
            continue;
        }
        let fade = SeeThroughTrees {
            target: chest,
            radius: 4.0,
            keep: 0.35,
            inner: 0.55,
            in_front: Some(0.8),
            floor: None,
        };
        if existing != Some(&fade) {
            commands.entity(entity).insert(fade);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Toda a espécie que o mundo usa (`prop-flora`) tem de existir nas
    /// espécies de reserva — senão a árvore fica invisível sem cena.
    #[test]
    fn every_prop_maps_to_a_pool() {
        let species = legacy_species();
        for pool in POOLS {
            assert!(
                pool.height_scale > 0.5 && pool.height_scale < 2.0,
                "fator de altura de `{}` fora do razoável: {}",
                pool.key,
                pool.height_scale
            );
        }
        for prop in [
            "pine_dark",
            "tree_pine",
            "tree_oak",
            "tree_birch",
            "dead_tree",
            "dead_willow",
            "palm_tree",
        ] {
            assert!(
                species.iter().any(|species| species.key == prop),
                "espécie `{prop}` sem pool"
            );
        }
    }

    /// O `world/settings/flora.bsn` do projeto cobre as mesmas espécies.
    #[test]
    fn flora_bsn_cobre_as_especies() {
        let world = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("world.bsn");
        let flora = world.with_file_name("world/settings/flora.bsn");
        if !flora.is_file() {
            return;
        }
        let (natives, _) =
            bevy_ext_world::bsn::settings::read_natives(&world).expect("ler world.bsn");
        let species = bevy_ext_world::bsn::settings::native_values::<FloraSpecies>(&natives);
        for legacy in legacy_species() {
            assert!(
                species.iter().any(|species| species.key == legacy.key),
                "`{}` falta no flora.bsn",
                legacy.key
            );
        }
    }

    #[test]
    fn variant_of_is_stable_and_bounded() {
        let name = Name::new("inst-grp-18-0000");
        let first = variant_of(Some(&name), 8);
        assert_eq!(first, variant_of(Some(&name), 8));
        assert!(first < 8);
        // Sem nome não rebenta e fica em 0 (variante base).
        assert_eq!(variant_of(None, 8), 0);
        // O mesmo nome em pools de tamanhos diferentes fica sempre no intervalo.
        for variants in 1..=8 {
            assert!(variant_of(Some(&name), variants) < variants);
        }
    }

    /// A altura do GLB é absorvida no pool: carvalho do mundo = 8 m, bétula = 8 m.
    #[test]
    fn pool_params_match_authored_heights() {
        let oak = pool_of("tree_oak").expect("pool do carvalho");
        let params = pool_params(oak);
        assert!(
            (params.trunk.height - 8.0).abs() < 1e-3,
            "carvalho com {} m (esperado 8)",
            params.trunk.height
        );
        let birch = pool_of("tree_birch").expect("pool da bétula");
        let params = pool_params(birch);
        assert!((params.trunk.height - 8.0).abs() < 1e-3);
    }

    /// O look frio não mexe na luminância de forma absurda (é um desvio de tom).
    #[test]
    fn cold_tint_is_a_grey_blue_shift() {
        let warm = Color::srgb(0.33, 0.21, 0.11);
        let cold = cold_tint(warm);
        let (w, c) = (warm.to_srgba(), cold.to_srgba());
        assert!(c.blue > w.blue, "o look frio tem de puxar para o azul");
        assert!(c.red < w.red, "e tirar vermelho");
    }
}

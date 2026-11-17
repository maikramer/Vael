//! Relva do Vael — adaptador do jogo sobre o `bevy_ext_grass`.
//!
//! O renderer, o bake e o contrato de estilos vivem no crate
//! `../bevy_extensions/crates/bevy_ext_grass`. Aqui fica só o que é do Vael:
//!
//! * [`VaelTerrain`] implementa [`GrassTerrain`] sobre o `TerrainRuntime`:
//!   alturas, água/estradas/pads/falésias/tetos finos/exclusões e os biomas
//!   autorados (`<BiomeRegion>`);
//! * [`VaelLayout`] é o layout dos campos: quatro quadrados rodados 45° com um
//!   vértice na origem (Floresta/Ermo/Pântano/Picos) + o campo central do
//!   Vale, quando há biomas; sem eles, um único campo do mundo inteiro;
//! * vento (`WeatherState` → `GrassWind`), tinta do dia (`DayCycleState` →
//!   `GrassTint`), trampling do herói/NPCs (marcadores da engine → avian) e o
//!   extra "relva" do profiler.
//!
//! A paleta e os settings vêm de `world/settings/grass.bsn`
//! ([`GrassSettingsSpec`]/[`GrassStyleSpec`]), editáveis no Jackdaw.
//!
//! Knobs: `VIBER_GRASS=0` desliga no arranque, `VIBER_GRASS_DENSITY=<f32>`,
//! `GRASS_DEBUG=1` loga o bake e despeja PGMs em `/tmp`.

use std::sync::Arc;

use crate::settings::{SceneSetting, SceneSettingList};
use bevy::math::Vec2;
use bevy::prelude::*;
use bevy_ext_grass::{
    FieldPlan, GrassBakeConfig, GrassBiomeId, GrassFieldLayout, GrassPlantSpec,
    GrassPlugin as ExtGrassPlugin, GrassPluginConfig, GrassProvider, GrassQuality, GrassSettings,
    GrassSettingsSpec, GrassSettingsSpecValue, GrassSite, GrassStyleSpec, GrassStyleSpecSet,
    GrassTerrain, GrassTint, GrassTrampler, GrassTramplerDisc, GrassWind,
};
use viber::ai::AiLocomotion;
use viber::animation::CharacterAnimator;
use viber::graphics::{self, GraphicsPreset};
use viber::player::Player;
use viber::profiler::{Group, timed};
use viber::scene_loader::SceneEpoch;
use viber::spawner::{SpawnExclusion, SpawnExclusions};
use viber::terrain::brush::BrushGrid;
use viber::terrain::cliffs::CliffMask;
use viber::terrain::delta::{DeltaGrid, EditedBase};
use viber::terrain::pads::ResolvedPad;
use viber::terrain::roads::RoadPath;
use viber::terrain::runtime::TerrainRuntime;
use viber::terrain::voxel::{HeightField, VoxelField};
use viber::terrain::water::WaterBody;
use viber::worldsys::{AtmosphereState, BiomeRegions, DayCycleState, WeatherState};

// ───────────────────────────────────────────────────────────── biomas

const BIOME_VALE: u16 = 0;
const BIOME_FOREST: u16 = 1;
const BIOME_DESERT: u16 = 2;
const BIOME_SWAMP: u16 = 3;
const BIOME_PEAKS: u16 = 4;

/// Id de bioma a partir do id autoral da `<BiomeRegion>`.
fn biome_id(id: &str) -> u16 {
    match id {
        "dark-forest" | "forest" => BIOME_FOREST,
        "desert" => BIOME_DESERT,
        "swamp" => BIOME_SWAMP,
        "frozen-peaks" | "peaks" => BIOME_PEAKS,
        _ => BIOME_VALE,
    }
}

fn biome_in_polys(biomes: &[(u16, Vec<[f32; 2]>)], p: Vec2) -> u16 {
    for (id, polygon) in biomes {
        if viber::ambient::point_in_polygon(p.x, p.y, polygon) {
            return *id;
        }
    }
    BIOME_VALE
}

// ───────────────────────────────────────────────────────────── terreno

/// Folga (m) em volta das fitas de estrada.
const ROAD_CLEARANCE: f32 = 1.1;
/// Espessura mínima de uma superfície em pé (banda de arco/beirada).
const MIN_STAND_THICKNESS: f32 = 4.0;

/// Fotografia do terreno e das regras do Vael para o bake do crate.
#[derive(Clone)]
pub struct VaelTerrain {
    world_size: f32,
    grid: Arc<BrushGrid>,
    deltas: Arc<DeltaGrid>,
    voxel: Arc<VoxelField>,
    water: Vec<WaterBody>,
    roads: Vec<RoadPath>,
    /// Treliça do pavimento: a relva cresce nas juntas do decal antigo, mas não
    /// por cima de uma praça ou de uma rua pavimentada.
    paving: Arc<viber::terrain::paving::PavingGrid>,
    pads: Vec<ResolvedPad>,
    cliffs: CliffMask,
    exclusions: Vec<SpawnExclusion>,
    biomes: Vec<(u16, Vec<[f32; 2]>)>,
}

impl VaelTerrain {
    fn base(&self) -> EditedBase<'_> {
        EditedBase {
            grid: &self.grid,
            deltas: &self.deltas,
        }
    }

    /// Molhado = dentro de um corpo de água **abaixo (ou na) linha de água**
    /// (com 0,25 m de folga para a banda molhada). O `contains` do corpo é a
    /// zona CARVADA, maior do que a lâmina: a margem seca do rio/lago fica
    /// acima da superfície e leva relva. O `ground` tem de contar — era o que
    /// faltava aqui (o ramo do `surface_y_at` era código morto e a margem
    /// toda ficava careca).
    fn is_wet(&self, p: Vec2, ground: f32) -> bool {
        self.water.iter().any(|body| {
            body.contains(p)
                && body
                    .surface_y_at(p)
                    .is_some_and(|surface| ground <= surface + 0.25)
        })
    }

    fn is_paved(&self, p: Vec2) -> bool {
        if self.paving.covers(p) {
            return true;
        }
        self.roads.iter().any(|road| road.within(p, ROAD_CLEARANCE))
    }

    fn is_pad(&self, p: Vec2) -> bool {
        self.pads.iter().any(|pad| {
            (p.x - pad.at.x).abs() <= pad.size.x * 0.5 && (p.y - pad.at.y).abs() <= pad.size.y * 0.5
        })
    }

    fn is_excluded(&self, p: Vec2) -> bool {
        self.exclusions.iter().any(|exclusion| {
            let d = p - exclusion.center;
            d.length_squared() <= exclusion.radius * exclusion.radius
        })
    }
}

impl GrassTerrain for VaelTerrain {
    fn world_size(&self) -> f32 {
        self.world_size
    }

    fn revision(&self) -> u64 {
        self.deltas.revision()
    }

    fn height(&self, x: f32, z: f32) -> f32 {
        self.base().sample(x, z)
    }

    fn site(&self, x: f32, z: f32, ground: f32) -> GrassSite {
        let p = Vec2::new(x, z);
        let half = self.world_size * 0.5;
        if x < -half || x > half || z < -half || z > half {
            return GrassSite::BLOCKED;
        }
        if self.is_wet(p, ground)
            || self.is_paved(p)
            || self.is_pad(p)
            || self.is_excluded(p)
            || self.cliffs.is_cliff_at(p)
        {
            return GrassSite::BLOCKED;
        }
        if !self.voxel.is_flat() && !self.voxel.is_flat_at(x, z) {
            let base = self.base();
            if self.voxel.has_thin_roof(&base, x, z, MIN_STAND_THICKNESS) {
                return GrassSite::BLOCKED;
            }
        }
        GrassSite::grass(
            coverage_at(x, z),
            GrassBiomeId(biome_in_polys(&self.biomes, p)),
        )
    }

    fn biome_count(&self) -> u16 {
        self.biomes.len() as u16
    }
}

/// Layout do Vael: quatro quadrantes rodados + Vale (ou um campo único sem
/// biomas).
#[derive(Debug, Default, Clone, Copy)]
pub struct VaelLayout;

impl GrassFieldLayout for VaelLayout {
    fn fields(&self, terrain: &dyn GrassTerrain) -> Vec<FieldPlan> {
        let world = terrain.world_size().max(1.0);
        if terrain.biome_count() == 0 {
            return vec![FieldPlan {
                biome: GrassBiomeId(BIOME_VALE),
                center: Vec2::ZERO,
                yaw: 0.0,
                size: Vec2::splat(world),
            }];
        }
        let half = world * 0.5;
        let side = world / std::f32::consts::SQRT_2;
        let quarter = std::f32::consts::FRAC_PI_4;
        vec![
            FieldPlan {
                biome: GrassBiomeId(BIOME_FOREST),
                center: Vec2::new(0.0, half),
                yaw: -quarter,
                size: Vec2::splat(side),
            },
            FieldPlan {
                biome: GrassBiomeId(BIOME_DESERT),
                center: Vec2::new(half, 0.0),
                yaw: quarter,
                size: Vec2::splat(side),
            },
            FieldPlan {
                biome: GrassBiomeId(BIOME_SWAMP),
                center: Vec2::new(-half, 0.0),
                yaw: -3.0 * quarter,
                size: Vec2::splat(side),
            },
            FieldPlan {
                biome: GrassBiomeId(BIOME_PEAKS),
                center: Vec2::new(0.0, -half),
                yaw: quarter,
                size: Vec2::splat(side),
            },
            FieldPlan {
                biome: GrassBiomeId(BIOME_VALE),
                center: Vec2::ZERO,
                yaw: 0.0,
                size: Vec2::splat(world.min(600.0)),
            },
        ]
    }
}

// ───────────────────────────────────────────────────────────── noise

fn hash01(x: i32, z: i32, salt: u32) -> f32 {
    let mut h = (x as u32)
        .wrapping_mul(0x9E37_79B1)
        .wrapping_add((z as u32).wrapping_mul(0x85EB_CA6B))
        .wrapping_add(salt.wrapping_mul(0xC2B2_AE35));
    h ^= h >> 15;
    h = h.wrapping_mul(0x2545_F491);
    h ^= h >> 13;
    (h & 0x00FF_FFFF) as f32 / 16_777_216.0
}

fn value_noise(x: f32, z: f32, scale: f32, salt: u32) -> f32 {
    let gx = x / scale;
    let gz = z / scale;
    let x0 = gx.floor();
    let z0 = gz.floor();
    let fx = gx - x0;
    let fz = gz - z0;
    let sx = fx * fx * (3.0 - 2.0 * fx);
    let sz = fz * fz * (3.0 - 2.0 * fz);
    let (ix, iz) = (x0 as i32, z0 as i32);
    let n00 = hash01(ix, iz, salt);
    let n10 = hash01(ix + 1, iz, salt);
    let n01 = hash01(ix, iz + 1, salt);
    let n11 = hash01(ix + 1, iz + 1, salt);
    let a = n00 + (n10 - n00) * sx;
    let b = n01 + (n11 - n01) * sx;
    a + (b - a) * sz
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0).max(1e-6)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Cobertura do Vael: clareiras (46 m) com bordas esfarrapadas (11 m).
fn coverage_at(x: f32, z: f32) -> f32 {
    let broad = value_noise(x, z, 46.0, 11);
    let fine = value_noise(x, z, 11.0, 29);
    let clearing = smoothstep(0.24, 0.62, broad);
    let patch = 0.55 + 0.45 * fine;
    (clearing * patch).clamp(0.0, 1.0) * 0.82 + 0.18 * clearing
}

// ───────────────────────────────────────────────────────────── dia/noite

/// Multiplicador de albedo ao longo do dia (0 = noite, 1 = dia cheio).
fn day_tint(day: f32) -> [f32; 3] {
    let day = day.clamp(0.0, 1.0);
    const NIGHT: [f32; 3] = [0.32, 0.36, 0.46];
    const GOLDEN: [f32; 3] = [1.12, 0.97, 0.80];
    let warmth = day * (1.0 - day) * 4.0;
    let mut out = [0.0f32; 3];
    for i in 0..3 {
        let neutral = NIGHT[i] + (1.0 - NIGHT[i]) * day;
        out[i] = neutral * (1.0 - warmth + GOLDEN[i] * warmth);
    }
    out
}

// ───────────────────────────────────────────────────────────── plugin

/// Plugin do Vael: terreno/layout/vento/tinta/trampling sobre o crate.
pub struct VaelGrassPlugin;

impl Plugin for VaelGrassPlugin {
    fn build(&self, app: &mut App) {
        let quality = quality();
        let enabled = gate_on("VIBER_GRASS");
        let density = env_density().unwrap_or_else(graphics::grass_density_scale);
        app.insert_resource(GrassSettings {
            enabled,
            density_scale: density,
            quality,
            shadows: shadows_for(quality),
            wind_scale: 1.0,
            instance_capacity: Some(capacity_for(quality)),
            max_chunk_slices: Some(slices_for(quality)),
        });
        app.add_plugins(ExtGrassPlugin {
            config: GrassPluginConfig {
                layout: Arc::new(VaelLayout),
                bake: GrassBakeConfig::default(),
                // O Vael embrulha o maintain no `timed(Group::Terrain, …)`.
                install_maintain: false,
            },
        });
        app.add_systems(
            Update,
            (
                provide_vael_terrain.before(bevy_ext_grass::GrassSystems::Maintain),
                sync_grass_specs.before(bevy_ext_grass::GrassSystems::SyncSpecs),
                drive_wind,
                drive_tint,
                tag_tramplers.before(bevy_ext_grass::GrassSystems::SyncSpecs),
            ),
        );
        app.add_systems(
            Update,
            timed(Group::Terrain, bevy_ext_grass::maintain_grass)
                .in_set(bevy_ext_grass::GrassSystems::Maintain),
        );
        app.add_systems(Startup, register_profiler_extra);
    }
}

/// Gate ligado-por-omissão: `0`/`false`/`off`/`no` (sem distinguir
/// maiúsculas nem espaços) desliga; ausente/qualquer outro valor liga.
fn gate_on(name: &str) -> bool {
    !matches!(
        std::env::var(name)
            .ok()
            .map(|raw| raw.trim().to_ascii_lowercase())
            .as_deref(),
        Some("0" | "false" | "off" | "no")
    )
}

fn quality() -> GrassQuality {
    match graphics::preset() {
        GraphicsPreset::Alto => GrassQuality::High,
        GraphicsPreset::Equilibrado => GrassQuality::Medium,
        GraphicsPreset::Desempenho => GrassQuality::Low,
    }
}

fn shadows_for(quality: GrassQuality) -> bevy_ext_grass::GrassShadows {
    match quality {
        GrassQuality::Low | GrassQuality::Competitive => bevy_ext_grass::GrassShadows::Off,
        _ => bevy_ext_grass::GrassShadows::Near,
    }
}

fn capacity_for(quality: GrassQuality) -> u32 {
    match quality {
        GrassQuality::High | GrassQuality::Ultra => 320_000,
        GrassQuality::Medium => 150_000,
        _ => 80_000,
    }
}

fn slices_for(quality: GrassQuality) -> u32 {
    match quality {
        GrassQuality::High | GrassQuality::Ultra => 8192,
        GrassQuality::Medium => 4096,
        _ => 2048,
    }
}

/// `VIBER_GRASS_DENSITY` (0..4), ou `None` quando ausente/inválido.
fn env_density() -> Option<f32> {
    std::env::var("VIBER_GRASS_DENSITY")
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|value| value.is_finite())
        .map(|value| value.clamp(0.0, 4.0))
}

/// `GrassQuality` → o valor autorável (`GrassQualitySpec`).
fn quality_spec(quality: GrassQuality) -> bevy_ext_grass::GrassQualitySpec {
    use bevy_ext_grass::GrassQualitySpec;
    match quality {
        GrassQuality::Low | GrassQuality::Competitive => GrassQualitySpec::Low,
        GrassQuality::Medium => GrassQualitySpec::Medium,
        GrassQuality::High => GrassQualitySpec::High,
        GrassQuality::Ultra => GrassQualitySpec::Ultra,
    }
}

/// Spec do crate a partir da cena + os knobs do JOGO.
///
/// O `sync_settings_spec` do `bevy_ext_grass` **substitui** a `GrassSettings`
/// pelo `GrassSettingsSpecValue` (e sem spec cai nos defaults do crate). Como
/// o `world/settings/grass.bsn` ainda não autora campos, o spec era igual ao
/// default e apagava no 1.º `Update` o `VIBER_GRASS=0`, o
/// `VIBER_GRASS_DENSITY` e o tier dos `graphics:` (qualidade/sombras). Aqui a
/// precedência é a do projeto: env de QA > cena (quando autorada) > tier.
fn spec_with_runtime_knobs(spec: &GrassSettingsSpec) -> GrassSettingsSpec {
    let mut out = spec.clone();
    let authored = *spec != GrassSettingsSpec::default();
    if !authored {
        // Nada autorado: o tier dos `graphics:` manda, como o plugin parece
        // (capacidade/fatias continuam a vir dele em qualquer caso).
        out.quality = quality_spec(quality());
        out.shadows = shadows_for(quality()) != bevy_ext_grass::GrassShadows::Off;
        // A densidade também: o `Desempenho` pede 0,6× e o `sync_settings_spec`
        // do crate copia este spec para a `GrassSettings` no 1.º `Update` — sem
        // esta linha o tier era substituído pelo 1,0 do default.
        out.density_scale = graphics::grass_density_scale();
    }
    // `VIBER_GRASS=0` desliga mesmo (a cena pode desligar; não pode religar).
    out.enabled &= gate_on("VIBER_GRASS");
    if let Some(density) = env_density() {
        out.density_scale = density;
    }
    out
}

/// Leva as definições autoradas (`SceneSetting`/`SceneSettingList`) para os
/// recursos que o crate lê. No jogo as definições são recursos; no editor são
/// componentes, e o crate aceita os dois (componentes ganham).
fn sync_grass_specs(
    settings: Res<SceneSetting<GrassSettingsSpec>>,
    styles: Res<SceneSettingList<GrassStyleSpec>>,
    mut out_settings: ResMut<GrassSettingsSpecValue>,
    mut out_styles: ResMut<GrassStyleSpecSet>,
) {
    if settings.is_changed() {
        out_settings.0 = Some(spec_with_runtime_knobs(&settings.0));
    }
    if styles.is_changed() {
        out_styles.0 = styles.0.clone();
    }
}

/// Instala o [`VaelTerrain`] quando o terreno da cena existe (e o remove em
/// cenas sem terreno). A chave `(época, grid)` evita reconstruir a fotografia
/// a cada frame; uma edição viva chega pela `revision()` do provider.
fn provide_vael_terrain(
    mut commands: Commands,
    terrain: Option<Res<TerrainRuntime>>,
    regions: Option<Res<BiomeRegions>>,
    cliffs: Option<Res<CliffMask>>,
    exclusions: Option<Res<SpawnExclusions>>,
    epoch: Option<Res<SceneEpoch>>,
    mut last: Local<Option<(u64, usize)>>,
) {
    let Some(terrain) = terrain else {
        if last.is_some() {
            commands.remove_resource::<GrassProvider>();
            *last = None;
        }
        return;
    };
    let key = (
        epoch.map(|epoch| epoch.0).unwrap_or(0),
        Arc::as_ptr(&terrain.grid) as *const () as usize,
    );
    if *last == Some(key) {
        return;
    }
    let biomes: Vec<(u16, Vec<[f32; 2]>)> = regions
        .as_deref()
        .map(|regions| {
            regions
                .list
                .iter()
                .map(|region| (biome_id(&region.id), region.polygon.clone()))
                .collect()
        })
        .unwrap_or_default();
    let provider = VaelTerrain {
        world_size: terrain.spec.world_size,
        grid: terrain.grid.clone(),
        deltas: terrain.deltas.clone(),
        voxel: terrain.voxel.clone(),
        water: terrain.water.clone(),
        roads: terrain.roads.clone(),
        paving: terrain.paving.clone(),
        pads: terrain.pads.clone(),
        cliffs: cliffs.as_deref().cloned().unwrap_or_default(),
        exclusions: exclusions
            .map(|exclusions| exclusions.0.clone())
            .unwrap_or_default(),
        biomes,
    };
    commands.insert_resource(GrassProvider(Arc::new(provider)));
    *last = Some(key);
}

/// `WeatherState` → `GrassWind` (o modelo de rajadas do crate corre por frame).
fn drive_wind(weather: Option<Res<WeatherState>>, mut wind: ResMut<GrassWind>) {
    let Some(weather) = weather else {
        return;
    };
    let direction = Vec2::new(weather.wind[0], weather.wind[1]);
    let next_dir = (direction.length_squared() > 1e-6).then(|| direction.normalize());
    let strength = (weather.wind_strength * 0.66).clamp(0.0, 1.0);
    let dir_changed = next_dir
        .is_some_and(|dir| wind.as_ref().direction.distance_squared(dir) > 1e-6);
    if !dir_changed && (wind.as_ref().strength - strength).abs() <= 1e-4 {
        return;
    }
    if let Some(dir) = next_dir {
        wind.direction = dir;
    }
    wind.strength = strength;
    wind.gust_scale = (28.0 - 18.0 * strength).clamp(8.0, 30.0);
    wind.gust_speed = (3.0 + 9.0 * strength).clamp(2.0, 12.0);
    wind.gust_contrast = 1.0 + 0.8 * strength;
}

/// `DayCycleState` → `GrassTint`.
///
/// O `<DayCycle>` saiu do mundo quando o `bevy_elements` tomou o céu: o
/// `DayCycleState` da engine já não existe no jogo e a tinta ficava presa no
/// dia (a relva não acompanhava a noite). O `AtmosphereState` — escrito pelo
/// `bridge_atmosphere` da cena a partir do sol astronómico — é o mesmo factor
/// que o resto da iluminação lê.
fn drive_tint(
    clock: Option<Res<DayCycleState>>,
    atmosphere: Option<Res<AtmosphereState>>,
    mut tint: ResMut<GrassTint>,
) {
    let day = clock
        .as_deref()
        .map(DayCycleState::visual_daylight)
        .or_else(|| atmosphere.as_deref().map(|state| state.day.clamp(0.0, 1.0)))
        .unwrap_or(1.0);
    let next = day_tint(day);
    if (tint.0[0] - next[0]).abs() > 1e-3
        || (tint.0[1] - next[1]).abs() > 1e-3
        || (tint.0[2] - next[2]).abs() > 1e-3
    {
        tint.0 = next;
    }
}

/// Herói, NPCs e criaturas animadas dobram a relva. O collider avian dá a
/// pegada; quem ainda não tem collider recebe o disco manual.
fn tag_tramplers(
    mut commands: Commands,
    candidates: Query<
        Entity,
        (
            Or<(With<Player>, With<AiLocomotion>, With<CharacterAnimator>)>,
            Without<GrassTrampler>,
        ),
    >,
    players: Query<
        Entity,
        (
            With<Player>,
            Without<avian3d::prelude::Collider>,
            Without<GrassTramplerDisc>,
        ),
    >,
) {
    for entity in &candidates {
        commands.entity(entity).insert(GrassTrampler);
    }
    for entity in &players {
        commands.entity(entity).insert(GrassTramplerDisc::default());
    }
}

/// Acrescenta o toggle "relva" ao painel Extras do profiler.
fn register_profiler_extra(mut extras: Option<ResMut<viber::profiler::ProfilerExtras>>) {
    let Some(extras) = extras.as_mut() else {
        return;
    };
    if extras.items.iter().any(|item| item.id == "grass") {
        return;
    }
    extras.items.push(viber::profiler::ProfilerExtra {
        id: "grass",
        label: "relva",
        description: "campo de relva GPU (GrassSettings.enabled)",
        is_on: grass_on,
        toggle: toggle_grass,
    });
}

fn grass_on(world: &mut World) -> bool {
    world
        .get_resource::<GrassSettings>()
        .map(|settings| settings.enabled)
        .unwrap_or(false)
}

fn toggle_grass(world: &mut World) {
    if let Some(mut settings) = world.get_resource_mut::<GrassSettings>() {
        settings.enabled = !settings.enabled;
    }
}

// ───────────────────────────────────────────────────────────── paletas

const BASE_DENSITY: f32 = 18.0;

/// As 5 paletas atuais do Vael, para `world/settings/grass.bsn` (e
/// `--export-settings`). Valores copiados dos `Profile` antigos.
#[must_use]
pub fn legacy_grass_styles() -> Vec<GrassStyleSpec> {
    vec![
        GrassStyleSpec {
            biome: BIOME_VALE,
            density: BASE_DENSITY,
            height: Vec2::new(0.335, 0.621),
            width: 0.056,
            lean: 0.138,
            droop: 0.207,
            root_color: Color::linear_rgb(0.045, 0.082, 0.032),
            tip_color: Color::linear_rgb(0.235, 0.310, 0.118),
            dry_color: Color::linear_rgb(0.370, 0.320, 0.140),
            translucency: 0.55,
            specular: 0.25,
            ground_color: Color::linear_rgb(0.120, 0.106, 0.045),
            ground_amount: 0.30,
            wind_scale: 1.0,
            species: vec![
                GrassPlantSpec {
                    share: 0.035,
                    ..white_flowers()
                },
                GrassPlantSpec {
                    share: 0.030,
                    ..yellow_flowers()
                },
                GrassPlantSpec {
                    share: 0.050,
                    ..clover()
                },
            ],
        },
        GrassStyleSpec {
            biome: BIOME_FOREST,
            density: BASE_DENSITY * 1.2,
            height: Vec2::new(0.459, 0.851),
            width: 0.066,
            lean: 0.156,
            droop: 0.234,
            root_color: Color::linear_rgb(0.024, 0.052, 0.030),
            tip_color: Color::linear_rgb(0.140, 0.230, 0.100),
            dry_color: Color::linear_rgb(0.235, 0.215, 0.090),
            translucency: 0.55,
            specular: 0.25,
            ground_color: Color::linear_rgb(0.073, 0.070, 0.034),
            ground_amount: 0.30,
            wind_scale: 0.72,
            species: vec![
                GrassPlantSpec {
                    share: 0.012,
                    clustering: 0.4,
                    ..white_flowers()
                },
                GrassPlantSpec {
                    share: 0.040,
                    ..broad_leaf()
                },
            ],
        },
        GrassStyleSpec {
            biome: BIOME_DESERT,
            density: BASE_DENSITY * 0.30,
            height: Vec2::new(0.260, 0.483),
            width: 0.048,
            lean: 0.078,
            droop: 0.117,
            root_color: Color::linear_rgb(0.140, 0.112, 0.052),
            tip_color: Color::linear_rgb(0.505, 0.418, 0.208),
            dry_color: Color::linear_rgb(0.600, 0.470, 0.205),
            translucency: 0.55,
            specular: 0.25,
            ground_color: Color::linear_rgb(0.245, 0.182, 0.077),
            ground_amount: 0.30,
            wind_scale: 1.12,
            species: Vec::new(),
        },
        GrassStyleSpec {
            biome: BIOME_SWAMP,
            density: BASE_DENSITY * 1.05,
            height: Vec2::new(0.589, 1.093),
            width: 0.068,
            lean: 0.120,
            droop: 0.180,
            root_color: Color::linear_rgb(0.038, 0.048, 0.024),
            tip_color: Color::linear_rgb(0.215, 0.230, 0.090),
            dry_color: Color::linear_rgb(0.330, 0.270, 0.110),
            translucency: 0.55,
            specular: 0.25,
            ground_color: Color::linear_rgb(0.101, 0.081, 0.039),
            ground_amount: 0.30,
            wind_scale: 0.90,
            species: vec![GrassPlantSpec {
                share: 0.050,
                ..seed_heads()
            }],
        },
        GrassStyleSpec {
            biome: BIOME_PEAKS,
            density: BASE_DENSITY * 0.50,
            height: Vec2::new(0.186, 0.345),
            width: 0.050,
            lean: 0.102,
            droop: 0.153,
            root_color: Color::linear_rgb(0.060, 0.076, 0.060),
            tip_color: Color::linear_rgb(0.225, 0.255, 0.200),
            dry_color: Color::linear_rgb(0.310, 0.285, 0.210),
            translucency: 0.55,
            specular: 0.25,
            ground_color: Color::linear_rgb(0.126, 0.115, 0.081),
            ground_amount: 0.30,
            wind_scale: 1.28,
            species: vec![GrassPlantSpec {
                share: 0.012,
                ..white_flowers()
            }],
        },
    ]
}

/// Settings autorados por omissão (usados pelo `--export-settings`).
#[must_use]
pub fn legacy_grass_settings() -> GrassSettingsSpec {
    GrassSettingsSpec::default()
}

fn white_flowers() -> GrassPlantSpec {
    GrassPlantSpec {
        height: Vec2::new(0.35, 0.70),
        width: 0.010,
        lean: 0.08,
        droop: 0.0,
        root_color: Color::srgb(0.05, 0.11, 0.03),
        tip_color: Color::srgb(0.24, 0.42, 0.12),
        share: 0.0,
        clustering: 0.0,
        head_length: 0.12,
        head_width: 0.05,
        head_color: Color::srgb(0.92, 0.92, 0.86),
    }
}

fn yellow_flowers() -> GrassPlantSpec {
    GrassPlantSpec {
        height: Vec2::new(0.30, 0.60),
        head_length: 0.10,
        head_width: 0.035,
        head_color: Color::srgb(0.95, 0.78, 0.12),
        ..white_flowers()
    }
}

fn clover() -> GrassPlantSpec {
    GrassPlantSpec {
        height: Vec2::new(0.08, 0.20),
        width: 0.035,
        lean: 0.35,
        droop: 0.70,
        root_color: Color::srgb(0.04, 0.10, 0.04),
        tip_color: Color::srgb(0.16, 0.34, 0.12),
        head_length: 0.0,
        head_width: 0.03,
        head_color: Color::srgb(0.16, 0.34, 0.12),
        ..white_flowers()
    }
}

fn broad_leaf() -> GrassPlantSpec {
    GrassPlantSpec {
        height: Vec2::new(0.20, 0.45),
        width: 0.060,
        lean: 0.30,
        droop: 0.60,
        root_color: Color::srgb(0.04, 0.09, 0.03),
        tip_color: Color::srgb(0.20, 0.38, 0.10),
        head_length: 0.0,
        head_width: 0.03,
        head_color: Color::srgb(0.20, 0.38, 0.10),
        ..white_flowers()
    }
}

fn seed_heads() -> GrassPlantSpec {
    GrassPlantSpec {
        height: Vec2::new(0.80, 1.40),
        width: 0.012,
        lean: 0.10,
        droop: 0.0,
        root_color: Color::srgb(0.07, 0.12, 0.04),
        tip_color: Color::srgb(0.40, 0.50, 0.20),
        head_length: 0.20,
        head_width: 0.022,
        head_color: Color::srgb(0.62, 0.56, 0.36),
        ..white_flowers()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_palettes_cover_every_biome() {
        let styles = legacy_grass_styles();
        assert_eq!(styles.len(), 5);
        let ids: Vec<u16> = styles.iter().map(|style| style.biome).collect();
        assert_eq!(
            ids,
            vec![
                BIOME_VALE,
                BIOME_FOREST,
                BIOME_DESERT,
                BIOME_SWAMP,
                BIOME_PEAKS
            ]
        );
        assert!(styles.iter().all(|style| style.density > 0.0));
        assert!(styles[BIOME_DESERT as usize].density < styles[BIOME_FOREST as usize].density);
    }

    #[test]
    fn layout_with_biomes_tiles_the_world() {
        struct Terrain;
        impl GrassTerrain for Terrain {
            fn world_size(&self) -> f32 {
                4000.0
            }
            fn revision(&self) -> u64 {
                0
            }
            fn height(&self, _x: f32, _z: f32) -> f32 {
                0.0
            }
            fn site(&self, _x: f32, _z: f32, _g: f32) -> GrassSite {
                GrassSite::grass(1.0, GrassBiomeId(BIOME_VALE))
            }
            fn biome_count(&self) -> u16 {
                4
            }
        }
        let plans = VaelLayout.fields(&Terrain);
        assert_eq!(plans.len(), 5);
        // Cada ponto fora das diagonais cai em exatamente um quadrante.
        for (x, z) in [
            (1990.0, 1000.0),
            (-1990.0, 1000.0),
            (1990.0, -1000.0),
            (-1990.0, -1000.0),
        ] {
            let p = Vec2::new(x, z);
            let hits = plans[..4].iter().filter(|plan| plan.contains(p)).count();
            assert_eq!(hits, 1, "canto ({x},{z}) caiu em {hits} quadrantes");
        }
    }

    /// Sem nada autorado no `grass.bsn`, o tier dos `graphics:` escolhe a
    /// qualidade, as sombras e a densidade (o crate substitui a `GrassSettings`
    /// pelo spec).
    #[test]
    fn spec_default_segue_o_tier_dos_graphics() {
        let out = spec_with_runtime_knobs(&GrassSettingsSpec::default());
        assert_eq!(out.quality, quality_spec(quality()));
        assert_eq!(
            out.shadows,
            shadows_for(quality()) != bevy_ext_grass::GrassShadows::Off
        );
        assert_eq!(
            out.density_scale,
            graphics::grass_density_scale(),
            "a densidade do tier não pode cair no 1,0 do default"
        );
    }

    /// Spec AUTORADO manda: o tier não reescreve qualidade/sombras/vento, e a
    /// cena pode desligar a relva mesmo com o gate ligado.
    #[test]
    fn spec_autorado_nao_e_reescrito_pelo_tier() {
        let spec = GrassSettingsSpec {
            enabled: false,
            quality: bevy_ext_grass::GrassQualitySpec::Low,
            shadows: false,
            wind_scale: 0.5,
            ..GrassSettingsSpec::default()
        };
        let out = spec_with_runtime_knobs(&spec);
        assert_eq!(out.quality, bevy_ext_grass::GrassQualitySpec::Low);
        assert!(!out.shadows);
        assert!((out.wind_scale - 0.5).abs() < 1e-6);
        assert!(!out.enabled, "a cena pode desligar a relva");
    }

    #[test]
    fn layout_without_biomes_is_one_field() {
        struct Terrain;
        impl GrassTerrain for Terrain {
            fn world_size(&self) -> f32 {
                220.0
            }
            fn revision(&self) -> u64 {
                0
            }
            fn height(&self, _x: f32, _z: f32) -> f32 {
                0.0
            }
            fn site(&self, _x: f32, _z: f32, _g: f32) -> GrassSite {
                GrassSite::grass(1.0, GrassBiomeId(BIOME_VALE))
            }
        }
        let plans = VaelLayout.fields(&Terrain);
        assert_eq!(plans.len(), 1);
        assert!(plans[0].contains(Vec2::new(100.0, -100.0)));
    }
}

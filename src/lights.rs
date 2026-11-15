//! Luzes práticas: fogueiras e lanternas com pool orçamentado, flicker e chama.
//!
//! O mundo autora ~450 `PointLight`, todas reais, estáticas e sem sombra. À
//! noite o `torch_pool_drive` da engine compensa a exposição com um ganho
//! global (×8 nas quentes, ×64 nas de sombra autoral) — uma lavagem quente
//! uniforme. Este módulo troca as **luzes de fogo** por `PoolCandidate` do
//! `bevy_lights`: o pool mantém N luzes reais perto do herói, com histerese e
//! cross-fade, e o `Flicker` faz cada uma respirar.
//!
//! # Invariante
//!
//! O pool só possui as luzes de **fogo** (a `PointLight` quente mais próxima de
//! um emissor `preset: "fire"`, até [`ADOPT_RADIUS`] m). As luzes quentes de
//! preenchimento — janelas, brilho da vila — continuam autorais e não passam
//! pelo pool: com `reach` de 60 m, se elas entrassem, o fim da rua apagava.
//! Os cristais frios (`point-cool-*`) também ficam de fora (o pool tem um
//! template único de cor/alcance).
//!
//! # Composição com a engine
//!
//! O `Flicker` do crate escreve `PointLight.intensity` a partir do seu `base`,
//! e o `torch_pool_drive` escreve o mesmo campo a partir da base que captura —
//! dois writers no mesmo campo. A regra é: **a luz de fogo tem um dono só.**
//! O candidato leva o valor autoral em [`VaelPractical::base`] e este módulo
//! reescreve `candidate.intensity` por frame com as funções públicas da engine
//! ([`torch_pool_gain`]/[`warm_fill_gain`]), preservando a calibração ×64/×8
//! por luz. As luzes que o pool spawna levam [`NightGainExempt`], senão o drive
//! da engine capturava a base 0 delas (nascem a 0) e escrevia 0 para sempre.
//!
//! # Gate
//!
//! `VAEL_PRACTICAL_LIGHTS=0` desliga tudo (regressão exacta do comportamento
//! anterior). O resto é afinação:
//!
//! | Env | Default | O quê |
//! |-----|---------|-------|
//! | `VAEL_PRACTICAL_LIGHTS` | `1` | o módulo inteiro |
//! | `VAEL_POOL_SIZE` | `24` | luzes reais do pool |
//! | `VAEL_POOL_SHADOWS` | `2` | quantas podem projectar sombra (0 desliga) |
//! | `VAEL_FIRE_FILL` | `1` | candidato para fogos sem luz (os braseiros de `clutter.bsn`) |
//! | `VAEL_FLAME_BILLBOARDS` | `1` | chama billboard do crate (A/B contra as partículas) |
//! | `VAEL_FIRE_MIGRATE` | `1` | **remove as partículas de fogo da engine** (as 248) e deixa só a chama; `0` mantém as duas para A/B |
//!
//! Os `preset: "fire"` do mundo são migrados: o emissor CPU da engine sai e a
//! entidade fica com [`VaelFire`] + `FlameSite`/`FuelBed`/brasas/fumo. Os
//! outros `ParticleSpec` (poeira, folhas, fumo, pirilampos…) não são tocados.
//!
//! O crate é novo e o README declara-o "Vibe Coded": está pinado `=0.5.0` no
//! `Cargo.toml` e todo o contacto com ele vive neste ficheiro.

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::transform::TransformSystems;
use bevy_lights::{
    FIRE_EDGE_LUX, FireKind, FlamePlugin, FlameSite, FlickerPlugin, FocusRule, FuelBed, LightPool,
    LightPoolPlugin, LightsSystems, PoolCandidate, PoolLight, Rank,
};
use viber::ambient::{AuthoredShadowLight, TravellerLantern};
use viber::particles::{AmbientFx, ParticleEmitter};
use viber::player::Player;
use viber::postfx::{
    NightGainExempt, torch_pool_gain, torch_pool_is_warm, torch_pool_target, warm_fill_gain,
};
// O `ParticleSpec` da ENGINE (o que o `AmbientFx` carrega), não o do
// `bevy_ext_scene`: são tipos distintos com o mesmo nome.
use viber::recipes::ParticleSpec;
use viber::worldsys::AtmosphereState;

/// Distância máxima entre uma luz quente e um emissor de fogo para a adoptar.
pub const ADOPT_RADIUS: f32 = 4.0;
/// Duas entidades mais próximas do que isto são o mesmo fogo já servido.
const PAIR_RADIUS: f32 = 2.0;
/// Luzes reais por omissão.
pub const DEFAULT_POOL_SIZE: usize = 24;
/// Cube shadow maps por omissão à noite. Cada um custa 6 vistas: o gotcha do
/// `AGENTS.md` diz que piscam no chão, por isso o default é conservador.
pub const DEFAULT_MAX_SHADOWED: usize = 2;
/// Fração de noite a partir da qual as sombras do pool ligam.
const SHADOW_NIGHT_MIN: f32 = 0.2;

/// Marker do foco do pool: exatamente um, senão o pool fica escuro.
#[derive(Component)]
struct VaelLightFocus;

/// Base autoral de uma luz de fogo adoptada, e a família de ganho dela.
#[derive(Component, Clone, Copy, Debug)]
struct VaelPractical {
    /// Intensidade do `.bsn` (antes do ganho nocturno).
    base: f32,
    /// `true` = tinha `AuthoredShadowLight` → família ×64, senão ×8.
    torch: bool,
}

/// Emissor de fogo já migrado: o `AmbientFx` da engine pode já não existir
/// (quando `VAEL_FIRE_MIGRATE=1`), mas o fogo continua a ser preciso para
/// parear a luz, preencher os que estão às escuras e desenhar a chama.
#[derive(Component, Clone, Copy, Debug)]
struct VaelFire {
    kind: FireKind,
    phase: f32,
}

/// Candidato de **preenchimento** criado por [`fill_dark_fires`] para um fogo
/// sem `PointLight`. É uma raiz nossa (não vem do spawner da cena): sem ser
/// despawnado na troca de cena ficava a arder onde já não há fogo — e o
/// fogo novo da cena seguinte era dispensado pelo `lit` do preenchimento
/// (a entidade velha já estava a 2 m).
#[derive(Component)]
struct VaelFilledFire;

/// Afinação lida do ambiente no arranque.
#[derive(Resource, Debug, Clone)]
struct PracticalConfig {
    adopt_radius: f32,
    pair_radius: f32,
    pool_size: usize,
    max_shadowed: usize,
    shadow_night_min: f32,
    fill_dark_fires: bool,
    flames: bool,
    migrate_particles: bool,
}

impl Default for PracticalConfig {
    fn default() -> Self {
        Self {
            adopt_radius: ADOPT_RADIUS,
            pair_radius: PAIR_RADIUS,
            pool_size: DEFAULT_POOL_SIZE,
            max_shadowed: DEFAULT_MAX_SHADOWED,
            shadow_night_min: SHADOW_NIGHT_MIN,
            fill_dark_fires: true,
            flames: true,
            migrate_particles: true,
        }
    }
}

/// Plugin das luzes práticas.
pub struct VaelPracticalLightsPlugin;

impl Plugin for VaelPracticalLightsPlugin {
    fn build(&self, app: &mut App) {
        if !enabled() {
            return;
        }
        let config = PracticalConfig {
            pool_size: env_usize("VAEL_POOL_SIZE", DEFAULT_POOL_SIZE).max(1),
            max_shadowed: env_usize("VAEL_POOL_SHADOWS", DEFAULT_MAX_SHADOWED),
            fill_dark_fires: env_flag("VAEL_FIRE_FILL", true),
            flames: env_flag("VAEL_FLAME_BILLBOARDS", true),
            // Sem chama não se remove o fogo da engine: o fogo desaparecia.
            migrate_particles: env_flag("VAEL_FIRE_MIGRATE", true)
                && env_flag("VAEL_FLAME_BILLBOARDS", true),
            ..Default::default()
        };
        app.add_plugins((FlickerPlugin, LightPoolPlugin::<VaelLightFocus>::default()));
        if config.flames {
            app.add_plugins(FlamePlugin);
        }
        app.insert_resource(LightPool {
            size: config.pool_size,
            reach: 60.0,
            gain: 1.0,
            light: PointLight {
                color: fire_color(),
                intensity: 0.0, // ignorada: cada slot usa a do candidato
                range: 20.0,
                radius: 0.45,
                shadow_maps_enabled: true,
                ..default()
            },
            rank: Rank::Illuminance,
            focus: FocusRule::One,
            max_shadowed: Some(0), // `gate_shadows_by_night` decide
            ..default()
        });
        app.insert_resource(config)
            .add_systems(
                Update,
                (
                    drive_focus,
                    drive_practical_intensity,
                    gate_shadows_by_night,
                )
                    .chain()
                    .before(LightsSystems::Pool),
            )
            .add_systems(Update, exempt_pool_lights.after(LightsSystems::Pool))
            // Limpa os candidatos de preenchimento da cena que saiu (antes do
            // `fill_dark_fires` do `PostUpdate`, que cria os da cena nova).
            .add_systems(Update, despawn_filled_fires_on_scene_change)
            // Migração/adoção/preenchimento leem `GlobalTransform`: só depois
            // da propagação, senão no frame de spawn todas as posições são a
            // identidade e os pareamentos luz↔fogo saem errados.
            .add_systems(
                PostUpdate,
                (migrate_fire_emitters, adopt_fire_lights, fill_dark_fires)
                    .chain()
                    .after(TransformSystems::Propagate),
            );
    }
}

fn enabled() -> bool {
    env_flag("VAEL_PRACTICAL_LIGHTS", true)
}

fn env_flag(name: &str, default: bool) -> bool {
    match std::env::var(name)
        .ok()
        .map(|v| v.trim().to_ascii_lowercase())
    {
        Some(v) if matches!(v.as_str(), "1" | "true" | "on" | "yes") => true,
        Some(v) if matches!(v.as_str(), "0" | "false" | "off" | "no") => false,
        _ => default,
    }
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

fn fire_color() -> Color {
    Color::srgb(1.0, 0.66, 0.34)
}

/// O `ParticleSpec` é de fogo?
fn is_fire(spec: &ParticleSpec) -> bool {
    spec.preset.eq_ignore_ascii_case("fire")
}

/// Tipo de fogo a partir do emissor. A taxa de emissão é o único número que o
/// `.bsn` autora por fogo (`fire-r18`, `fire-r30`…), por isso é ela que escala
/// o tamanho do leito. Fica conservador no flicker: lanternas e tochas calmas,
/// braseiros e fogueiras com balanço.
fn fire_kind(spec: &ParticleSpec) -> Option<FireKind> {
    if !is_fire(spec) {
        return None;
    }
    let rate = spec.emission_rate.unwrap_or(18.0);
    Some(if rate <= 20.0 {
        FireKind::Lantern
    } else if rate <= 34.0 {
        FireKind::Brazier
    } else {
        FireKind::Campfire
    })
}

/// Fase determinística por posição: dois fogos não pulsam em passo, e o mesmo
/// fogo tem a mesma fase em todas as execuções (e no save).
fn phase_for(pos: Vec3) -> f32 {
    let bits = (pos.x * 73_856_093.0) as i64
        ^ (pos.y * 19_349_663.0) as i64
        ^ (pos.z * 83_492_791.0) as i64;
    let unit = ((bits as u64) % 10_000) as f32 / 10_000.0;
    unit * std::f32::consts::TAU
}

fn seed_for(pos: Vec3) -> u32 {
    ((pos.x * 73_856_093.0) as i64 ^ (pos.z * 83_492_791.0) as i64) as u64 as u32
}

/// Garante exatamente um foco: o herói (o mais estável), senão a câmara do
/// `bevy_elements`, senão qualquer `Camera3d`.
fn drive_focus(
    mut commands: Commands,
    players: Query<Entity, With<Player>>,
    elements: Query<Entity, (With<bevy_elements::ElementsCamera>, Without<Player>)>,
    cameras: Query<
        Entity,
        (
            With<Camera3d>,
            Without<Player>,
            Without<bevy_elements::ElementsCamera>,
        ),
    >,
    focus: Query<Entity, With<VaelLightFocus>>,
) {
    if focus.iter().count() == 1 {
        return;
    }
    let target = players
        .iter()
        .next()
        .or_else(|| elements.iter().next())
        .or_else(|| cameras.iter().next());
    for entity in &focus {
        commands.entity(entity).remove::<VaelLightFocus>();
    }
    if let Some(entity) = target {
        commands.entity(entity).insert(VaelLightFocus);
    }
}

/// Compõe a noite nas candidatas: `base × ganho da família` — a mesma conta do
/// `torch_pool_drive`, mas por luz e antes de o pool ler o valor.
fn drive_practical_intensity(
    atmosphere: Option<Res<AtmosphereState>>,
    mut candidates: Query<(&mut PoolCandidate, &VaelPractical)>,
) {
    let night = atmosphere.map(|a| a.night).unwrap_or(0.0).clamp(0.0, 1.0);
    let torch = torch_pool_gain(night);
    let fill = warm_fill_gain(night);
    for (mut candidate, practical) in &mut candidates {
        let gain = if practical.torch { torch } else { fill };
        let target = torch_pool_target(practical.base, gain);
        if (candidate.intensity - target).abs() > 1.0 {
            candidate.intensity = target;
        }
    }
}

/// Adopta as luzes de fogo autorais: a `PointLight` quente mais próxima de um
/// emissor passa a `PoolCandidate` e perde o `PointLight` (um dono só).
///
/// Acorda quando entra uma luz **ou** um fogo: os dois podem chegar em frames
/// diferentes (o prefab e o nó inline não são sempre expandidos juntos), e uma
/// luz que nascesse antes do emissor ficava autoral para sempre.
#[allow(clippy::type_complexity)]
fn adopt_fire_lights(
    mut commands: Commands,
    config: Res<PracticalConfig>,
    fires: Query<(&GlobalTransform, &VaelFire)>,
    new_lights: Query<(), Added<PointLight>>,
    new_fires: Query<(), Added<VaelFire>>,
    lights: Query<
        (
            Entity,
            &GlobalTransform,
            &PointLight,
            Option<&AuthoredShadowLight>,
        ),
        (
            Without<PoolCandidate>,
            Without<TravellerLantern>,
            Without<PoolLight>,
        ),
    >,
    adopted: Query<&GlobalTransform, (With<PoolCandidate>, With<VaelPractical>)>,
) {
    if new_lights.is_empty() && new_fires.is_empty() {
        return;
    }
    let candidates: Vec<(Vec3, FireKind, f32)> = fires
        .iter()
        .map(|(transform, fire)| (transform.translation(), fire.kind, fire.phase))
        .collect();
    if candidates.is_empty() {
        return;
    }
    let mut served: Vec<Vec3> = adopted.iter().map(|t| t.translation()).collect();
    for (entity, transform, light, shadow) in &lights {
        // Só fogo: cristais frios e luz de preenchimento ficam autorais.
        if !torch_pool_is_warm(light.color.to_linear()) {
            continue;
        }
        let pos = transform.translation();
        let winner = candidates
            .iter()
            .filter(|(fire, ..)| fire.distance(pos) <= config.adopt_radius)
            .filter(|(fire, ..)| {
                !served
                    .iter()
                    .any(|s| s.distance(*fire) <= config.pair_radius)
            })
            .min_by(|(a, ..), (b, ..)| a.distance_squared(pos).total_cmp(&b.distance_squared(pos)))
            .copied();
        let Some((fire_pos, kind, phase)) = winner else {
            continue;
        };
        served.push(fire_pos);
        commands
            .entity(entity)
            .insert((
                PoolCandidate {
                    offset: Vec3::ZERO,
                    intensity: light.intensity,
                    phase: Some(phase),
                    diameter: Some(kind.diameter()),
                    depth: Some(kind.depth()),
                    color: Some(light.color),
                },
                VaelPractical {
                    base: light.intensity,
                    torch: shadow.is_some(),
                },
            ))
            .remove::<PointLight>();
    }
}

/// Dá luz aos fogos que nenhuma `PointLight` ilumina (os 14 braseiros de
/// `clutter.bsn`, parte da cidadela). A intensidade sai do modelo do crate —
/// [`FIRE_EDGE_LUX`] à distância de alcance do tipo — em vez de mais um número
/// afinado à mão.
#[allow(clippy::type_complexity)]
fn fill_dark_fires(
    mut commands: Commands,
    config: Res<PracticalConfig>,
    fires: Query<(&GlobalTransform, &VaelFire), Added<VaelFire>>,
    adopted: Query<&GlobalTransform, (With<PoolCandidate>, With<VaelPractical>)>,
    lights: Query<
        &GlobalTransform,
        (
            With<PointLight>,
            Without<PoolLight>,
            Without<TravellerLantern>,
        ),
    >,
) {
    if !config.fill_dark_fires || fires.is_empty() {
        return;
    }
    let mut lit: Vec<Vec3> = adopted
        .iter()
        .chain(lights.iter())
        .map(|t| t.translation())
        .collect();
    for (transform, fire) in &fires {
        let kind = fire.kind;
        let pos = transform.translation();
        if lit.iter().any(|l| l.distance(pos) <= config.pair_radius) {
            continue;
        }
        lit.push(pos);
        let over = kind.height() * 0.5;
        let mean = kind.lumens(FIRE_EDGE_LUX, over);
        if !mean.is_finite() || mean <= 0.0 {
            continue;
        }
        let mut candidate = kind.candidate_at_mean(mean);
        candidate.color = Some(fire_color());
        candidate.phase = Some(fire.phase);
        commands.spawn((
            candidate,
            VaelPractical {
                base: mean,
                torch: false,
            },
            VaelFilledFire,
            Transform::from_translation(pos),
            Visibility::Inherited,
            Name::new("vael.fire-light"),
        ));
    }
}

/// Os candidatos de preenchimento morrem com a cena: são raízes nossas e o
/// descarregamento só leva as entidades que o spawner marcou.
fn despawn_filled_fires_on_scene_change(
    mut commands: Commands,
    current: Option<Res<viber::scene_loader::CurrentScene>>,
    filled: Query<Entity, With<VaelFilledFire>>,
    mut last: Local<Option<String>>,
) {
    let Some(current) = current else {
        return;
    };
    let id = current.id.clone().unwrap_or_default();
    if last.as_deref() == Some(id.as_str()) {
        return;
    }
    let first = last.is_none();
    *last = Some(id);
    if first {
        return;
    }
    for entity in &filled {
        commands.entity(entity).despawn();
    }
}

/// O pool spawna as luzes a 0; sem a isenção o `torch_pool_drive` capturava
/// essa base e escrevia 0 para sempre (o bug já documentado da lanterna).
fn exempt_pool_lights(
    mut commands: Commands,
    added: Query<Entity, (Added<PoolLight>, Without<NightGainExempt>)>,
) {
    for entity in &added {
        commands.entity(entity).insert(NightGainExempt);
    }
}

/// As sombras do pool só existem de noite (o custo é 6 vistas por luz) e no
/// máximo `VAEL_POOL_SHADOWS`. Escreve só quando muda: um `ResMut` tocado
/// todos os frames re-copia o template do pool para as luzes.
fn gate_shadows_by_night(
    config: Res<PracticalConfig>,
    atmosphere: Option<Res<AtmosphereState>>,
    mut pool: ResMut<LightPool>,
) {
    let night = atmosphere.map(|a| a.night).unwrap_or(0.0);
    let want = if night >= config.shadow_night_min {
        Some(config.max_shadowed)
    } else {
        Some(0)
    };
    if pool.max_shadowed != want {
        pool.max_shadowed = want;
    }
}

/// Migra o fogo da engine para o crate: marca a entidade com [`VaelFire`],
/// veste a chama (mesma fase/leito que a luz do pool) e, com
/// `VAEL_FIRE_MIGRATE=1`, **remove as partículas CPU** — o `AmbientFx` e o
/// bundle que o `ambient_fx_manager` montou (`ParticleEmitter`+`Mesh3d`).
///
/// A entidade em si fica (nome, hierarquia, `Transform` e `Visibility`: a
/// chama precisa da visibilidade e o `lineage` do mundo não muda). O gestor
/// da engine nunca despawna o marcador, por isso sem esta remoção as
/// partículas continuavam a correr.
fn migrate_fire_emitters(
    mut commands: Commands,
    config: Res<PracticalConfig>,
    fires: Query<(Entity, &GlobalTransform, &AmbientFx), Added<AmbientFx>>,
    already: Query<(), With<VaelFire>>,
) {
    for (entity, transform, fx) in &fires {
        if already.contains(entity) {
            continue;
        }
        let Some(kind) = fire_kind(&fx.spec) else {
            continue;
        };
        let pos = transform.translation();
        let phase = phase_for(pos);
        let mut bundle = commands.entity(entity);
        bundle.try_insert(VaelFire { kind, phase });
        if config.flames {
            bundle.try_insert((
                FlameSite {
                    height: kind.height(),
                    phase,
                    kindle: 0.0,
                    diameter: Some(kind.diameter()),
                    depth: kind.depth(),
                    depth_fade: false,
                    fire: None,
                    color: Some(fire_color()),
                },
                FuelBed {
                    diameter: kind.diameter().max(0.2),
                    seed: seed_for(pos),
                    ..default()
                },
            ));
            if let Some(embers) = kind.embers() {
                bundle.try_insert(embers);
            }
            if let Some(smoke) = kind.smoke() {
                bundle.try_insert(smoke);
            }
        }
        if config.migrate_particles {
            bundle.try_remove::<AmbientFx>();
            bundle.try_remove::<(
                Mesh3d,
                MeshMaterial3d<StandardMaterial>,
                NotShadowCaster,
                ParticleEmitter,
            )>();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(preset: &str, rate: Option<f32>) -> ParticleSpec {
        ParticleSpec {
            preset: preset.to_owned(),
            emission_rate: rate,
            life: None,
            speed: None,
            size: None,
            color: None,
            shape_radius: None,
            looping: true,
            world_space: false,
        }
    }

    #[test]
    fn fire_kind_scales_with_the_emission_rate() {
        assert_eq!(
            fire_kind(&spec("fire", Some(18.0))),
            Some(FireKind::Lantern)
        );
        assert_eq!(
            fire_kind(&spec("fire", Some(30.0))),
            Some(FireKind::Brazier)
        );
        assert_eq!(
            fire_kind(&spec("fire", Some(60.0))),
            Some(FireKind::Campfire)
        );
        // Sem taxa autoral, o default do `.bsn` é a lanterna.
        assert_eq!(fire_kind(&spec("fire", None)), Some(FireKind::Lantern));
    }

    #[test]
    fn not_fire_is_not_a_practical() {
        assert_eq!(fire_kind(&spec("fireflies", Some(20.0))), None);
        assert_eq!(fire_kind(&spec("smoke", Some(20.0))), None);
        assert!(is_fire(&spec("FIRE", None)), "o preset é case-insensitive");
    }

    #[test]
    fn phase_is_deterministic_and_out_of_step() {
        let a = phase_for(Vec3::new(3.0, 1.0, -7.0));
        assert_eq!(a, phase_for(Vec3::new(3.0, 1.0, -7.0)));
        assert!((0.0..std::f32::consts::TAU).contains(&a));
        let b = phase_for(Vec3::new(-7.0, 1.0, 3.0));
        assert_ne!(a, b, "fogos diferentes não pulsam em passo");
    }

    #[test]
    fn a_practical_light_composes_with_the_engine_gains() {
        // A conta que o `drive_practical_intensity` faz, à noite e de dia.
        let base = 120_000.0;
        assert!((torch_pool_target(base, torch_pool_gain(0.0)) - base).abs() < 1.0);
        assert!(
            (torch_pool_target(base, torch_pool_gain(1.0)) - base * 64.0).abs() < 1.0,
            "família tocha ×64"
        );
        assert!(
            (torch_pool_target(base, warm_fill_gain(1.0)) - base * 8.0).abs() < 1.0,
            "família fill ×8"
        );
    }

    #[test]
    fn migrates_a_fire_emitter_and_drops_the_engine_particles() {
        use bevy::ecs::system::RunSystemOnce as _;

        let mut world = World::default();
        world.insert_resource(PracticalConfig::default());
        let fire = world
            .spawn((
                AmbientFx {
                    spec: spec("fire", Some(30.0)),
                },
                GlobalTransform::from_translation(Vec3::new(1.0, 0.0, 2.0)),
                Visibility::Inherited,
            ))
            .id();
        let smoke = world
            .spawn((
                AmbientFx {
                    spec: spec("smoke", Some(20.0)),
                },
                GlobalTransform::default(),
            ))
            .id();
        world.run_system_once(migrate_fire_emitters).unwrap();
        world.flush();

        let vael_fire = world.get::<VaelFire>(fire).expect("marcador de fogo");
        assert_eq!(vael_fire.kind, FireKind::Brazier);
        assert!(
            world.get::<FlameSite>(fire).is_some() && world.get::<FuelBed>(fire).is_some(),
            "a chama e o leito de carvão entram"
        );
        assert!(
            world.get::<AmbientFx>(fire).is_none(),
            "as partículas de fogo da engine saem"
        );
        assert!(
            world.get::<Visibility>(fire).is_some(),
            "a chama precisa da visibilidade"
        );
        assert!(
            world.get::<AmbientFx>(smoke).is_some() && world.get::<VaelFire>(smoke).is_none(),
            "fumo não é fogo: fica na engine"
        );
    }

    #[test]
    fn a_fire_with_no_light_gets_a_filled_candidate() {
        use bevy::ecs::system::RunSystemOnce as _;

        let mut world = World::default();
        world.insert_resource(PracticalConfig::default());
        world.spawn((
            VaelFire {
                kind: FireKind::Brazier,
                phase: 0.5,
            },
            GlobalTransform::from_translation(Vec3::new(5.0, 0.0, 5.0)),
        ));
        world.run_system_once(fill_dark_fires).unwrap();
        world.flush();

        let mut query = world.query::<(&PoolCandidate, &Transform, &VaelFilledFire)>();
        let filled: Vec<_> = query.iter(&world).collect();
        assert_eq!(filled.len(), 1, "um fogo escuro, um candidato");
        assert!(filled[0].0.intensity > 0.0);
        assert_eq!(
            filled[0].0.phase,
            Some(0.5),
            "a chama e a luz partilham a fase"
        );
        assert!(
            filled[0].1.translation.distance(Vec3::new(5.0, 0.0, 5.0)) < 0.01,
            "o candidato nasce no fogo"
        );
    }

    #[test]
    fn adopts_only_the_warm_light_next_to_a_fire() {
        use bevy::ecs::system::RunSystemOnce as _;

        let mut world = World::default();
        world.insert_resource(PracticalConfig::default());
        let fire = |x: f32, z: f32| {
            (
                VaelFire {
                    kind: FireKind::Lantern,
                    phase: 1.25,
                },
                GlobalTransform::from_translation(Vec3::new(x, 0.0, z)),
            )
        };
        world.spawn(fire(10.0, 10.0));
        let light = |pos: Vec3, color: Color, intensity: f32| {
            (
                PointLight {
                    intensity,
                    color,
                    ..default()
                },
                GlobalTransform::from_translation(pos),
            )
        };
        let warm = world
            .spawn(light(
                Vec3::new(10.5, 0.6, 10.0),
                Color::srgb(1.0, 0.65, 0.30),
                4_800.0,
            ))
            .id();
        let cool = world
            .spawn(light(
                Vec3::new(10.5, 0.6, 10.5),
                Color::srgb(0.06, 0.28, 1.0),
                3_800.0,
            ))
            .id();
        let far = world
            .spawn(light(
                Vec3::new(80.0, 0.0, 80.0),
                Color::srgb(1.0, 0.65, 0.30),
                4_800.0,
            ))
            .id();
        world.run_system_once(adopt_fire_lights).unwrap();
        world.flush();

        let adopted = world
            .get::<PoolCandidate>(warm)
            .expect("a luz quente junto ao fogo vira candidata");
        assert_eq!(adopted.intensity, 4_800.0, "intensidade autoral preservada");
        assert_eq!(
            adopted.phase,
            Some(1.25),
            "o candidato herda a fase do fogo"
        );
        assert!(
            adopted.diameter.is_some(),
            "o candidato leva o leito do fogo"
        );
        assert!(
            world.get::<PointLight>(warm).is_none(),
            "um só dono: a PointLight autoral sai"
        );
        assert!(
            world.get::<PoolCandidate>(cool).is_none(),
            "cristal frio não entra no pool"
        );
        assert!(
            world.get::<PoolCandidate>(far).is_none(),
            "fora do raio de adoção fica autoral"
        );
    }

    #[test]
    fn night_gain_rewrites_the_candidate_intensity() {
        use bevy::ecs::system::RunSystemOnce as _;

        let mut world = World::default();
        world.insert_resource(AtmosphereState {
            night: 1.0,
            ..default()
        });
        let fill = world
            .spawn((
                PoolCandidate {
                    intensity: 4_800.0,
                    ..default()
                },
                VaelPractical {
                    base: 4_800.0,
                    torch: false,
                },
            ))
            .id();
        let torch = world
            .spawn((
                PoolCandidate {
                    intensity: 120_000.0,
                    ..default()
                },
                VaelPractical {
                    base: 120_000.0,
                    torch: true,
                },
            ))
            .id();
        world.run_system_once(drive_practical_intensity).unwrap();

        let fill_now = world.get::<PoolCandidate>(fill).unwrap().intensity;
        assert!(
            (fill_now - 4_800.0 * 8.0).abs() < 1.0,
            "fill ×8: {fill_now}"
        );
        let torch_now = world.get::<PoolCandidate>(torch).unwrap().intensity;
        assert!(
            (torch_now - 120_000.0 * 64.0).abs() < 1.0,
            "tocha ×64: {torch_now}"
        );
    }
}

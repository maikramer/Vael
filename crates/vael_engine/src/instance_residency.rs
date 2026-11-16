//! Residência por distância das instâncias **puramente estáticas** dos
//! spawners (vegetação `.vinst`, flores, arbustos, pedras de decoração).
//!
//! O [`crate::gltf_residency`] só gere os `<GltfScene>` do mundo; as
//! instâncias dos `<StaticSpawner>`/`<Vegetation>` ficavam todas com a cena
//! glTF montada desde o arranque. No mundo do Vael eram ~25k malhas (só as
//! flores: 11,5k) que nunca se viam — o `VisibilityRange` cortava-as — mas
//! que o `calculate_bounds`, o `check_visibility`, o reset de visibilidade e
//! a extração do render varriam em todos os frames (medido a 2026-11-15:
//! 89k entidades, 32,7k malhas, 7,8k visíveis).
//!
//! Aqui a instância (a raiz com `Transform`) nasce sempre; a subárvore da
//! cena só existe dentro do raio de render do grupo (+ folga), com histerese
//! na descarga. Só entram instâncias sem script, sem colisor, sem props, sem
//! ladder de LOD e não dinâmicas — nada que o gameplay ou os saves leiam.
//!
//! Medido a 2026-11-16 (spawn `--entry dev`): 89k → 62k entidades, 32,7k →
//! 18,1k malhas, frame 26,3 → 24,5 ms (p95 35 → 28), sem diferença visual.
//! `VIBER_INSTANCE_STREAMING=0` mantém tudo montado (A/B).

use std::sync::OnceLock;

use bevy::prelude::*;
use bevy::world_serialization::{WorldAsset, WorldAssetRoot, WorldInstance, WorldInstanceSpawner};

use crate::player::Player;
use crate::render_lod::CullRangeApplied;

/// Folga (m) entre o corte de render e a montagem da cena.
const LOAD_MARGIN: f32 = 20.0;
/// Factor da descarga sobre o raio de montagem.
const UNLOAD_HYSTERESIS: f32 = 1.3;
/// Avaliação a cada N frames (a lista tem milhares de raízes).
const EVAL_EVERY: u32 = 6;
/// Montagens/descargas por avaliação.
const LOAD_BUDGET: usize = 256;
const UNLOAD_BUDGET: usize = 1024;

pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        !matches!(
            std::env::var("VIBER_INSTANCE_STREAMING").ok().as_deref(),
            Some("0" | "false" | "off")
        )
    })
}

/// Instância estática de spawner cuja cena pode ser desmontada ao longe.
#[derive(Component, Clone)]
pub struct ResidentInstance {
    pub scene: Handle<WorldAsset>,
    /// Raio de montagem (m): o corte de render do grupo + folga.
    pub radius: f32,
    /// Re-arma o batch de folhagem a cada montagem.
    pub foliage: bool,
}

impl ResidentInstance {
    pub fn new(scene: Handle<WorldAsset>, cull_radius: f32, foliage: bool) -> Self {
        Self {
            scene,
            radius: cull_radius.max(1.0) + LOAD_MARGIN,
            foliage,
        }
    }
}

#[derive(Resource, Default, Debug)]
pub struct InstanceResidency {
    pub resident: usize,
    pub asleep: usize,
}

pub struct InstanceResidencyPlugin;

impl Plugin for InstanceResidencyPlugin {
    fn build(&self, app: &mut App) {
        if !enabled() {
            return;
        }
        app.init_resource::<InstanceResidency>().add_systems(
            PostUpdate,
            drive_instance_residency
                .after(bevy::transform::TransformSystems::Propagate)
                .before(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate),
        );
    }
}

#[allow(clippy::type_complexity)]
fn drive_instance_residency(
    mut commands: Commands,
    mut state: ResMut<InstanceResidency>,
    mut spawner: Option<ResMut<WorldInstanceSpawner>>,
    mode: Option<Res<crate::app::RunMode>>,
    players: Query<&GlobalTransform, With<Player>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    instances: Query<(
        Entity,
        &ResidentInstance,
        &GlobalTransform,
        Has<WorldAssetRoot>,
        Option<&WorldInstance>,
    )>,
    mut frame: Local<u32>,
) {
    *frame = frame.wrapping_add(1);
    if mode.as_deref() == Some(&crate::app::RunMode::Edit) {
        return;
    }
    if *frame % EVAL_EVERY != 0 {
        return;
    }
    let hero = players.iter().next().map(GlobalTransform::translation);
    let eye = cameras
        .iter()
        .filter(|(camera, _)| camera.is_active)
        .max_by_key(|(camera, _)| camera.order)
        .map(|(_, global)| global.translation());
    let Some(focus) = hero.or(eye) else {
        return;
    };
    let mut to_load: Vec<(f32, Entity)> = Vec::new();
    let mut to_unload: Vec<(f32, Entity)> = Vec::new();
    let (mut resident, mut asleep) = (0usize, 0usize);
    for (entity, spec, global, root, instance) in &instances {
        let p = global.translation();
        let d = focus
            .distance(p)
            .min(eye.map_or(f32::MAX, |e| e.distance(p)));
        if root || instance.is_some() {
            resident += 1;
            if d > spec.radius * UNLOAD_HYSTERESIS {
                to_unload.push((d, entity));
            }
        } else {
            asleep += 1;
            if d <= spec.radius {
                to_load.push((d, entity));
            }
        }
    }
    // Perto primeiro a montar; longe primeiro a desmontar.
    to_load.sort_by(|a, b| a.0.total_cmp(&b.0));
    to_unload.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut loaded = 0usize;
    for (_, entity) in to_load.into_iter().take(LOAD_BUDGET) {
        let Ok((_, spec, _, _, _)) = instances.get(entity) else {
            continue;
        };
        let mut e = commands.entity(entity);
        e.insert(WorldAssetRoot(spec.scene.clone()));
        if spec.foliage {
            e.insert(crate::foliage::FoliageCandidate { multi: true });
        }
        loaded += 1;
    }
    let mut unloaded = 0usize;
    for (_, entity) in to_unload.into_iter().take(UNLOAD_BUDGET) {
        let Ok((_, _, _, _, instance)) = instances.get(entity) else {
            continue;
        };
        if let (Some(_), Some(instance)) = (spawner.as_mut(), instance) {
            crate::gltf_residency::despawn_instance_now(&mut commands, instance);
        }
        commands.entity(entity).remove::<(
            WorldAssetRoot,
            WorldInstance,
            CullRangeApplied,
            crate::foliage::FoliageCandidate,
        )>();
        unloaded += 1;
    }
    if *frame % 600 == 0 {
        debug!(
            "instance-residency: {resident} montadas, {asleep} adormecidas, +{loaded}/-{unloaded}"
        );
    }
    state.resident = resident + loaded - unloaded.min(resident + loaded);
    state.asleep = asleep + unloaded - loaded.min(asleep + unloaded);
}

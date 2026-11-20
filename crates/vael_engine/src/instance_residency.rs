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

use std::collections::HashMap;
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
            bevy_engine_core::envs::var_lossy("INSTANCE_STREAMING").as_deref(),
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

const CELL_SIZE: f32 = 64.0;

#[inline]
fn cell_of(pos: Vec3) -> IVec2 {
    IVec2::new(
        (pos.x / CELL_SIZE).floor() as i32,
        (pos.z / CELL_SIZE).floor() as i32,
    )
}

#[inline]
fn cell_dist_sq_xz(cell: IVec2, pos: Vec3) -> f32 {
    let min_x = cell.x as f32 * CELL_SIZE;
    let max_x = min_x + CELL_SIZE;
    let min_z = cell.y as f32 * CELL_SIZE;
    let max_z = min_z + CELL_SIZE;
    let cx = pos.x.clamp(min_x, max_x);
    let cz = pos.z.clamp(min_z, max_z);
    let dx = pos.x - cx;
    let dz = pos.z - cz;
    dx * dx + dz * dz
}

#[derive(Default)]
struct InstanceResidencyCache {
    seeded: bool,
    entity_cell: HashMap<Entity, IVec2>,
    by_cell: HashMap<IVec2, Vec<Entity>>,
    max_radius: f32,
    dirty: bool,
    has_overflow: bool,
    last_focus: Option<Vec3>,
    last_eye: Option<Vec3>,
    to_load: Vec<(f32, Entity)>,
    to_unload: Vec<(f32, Entity)>,
}

impl InstanceResidencyCache {
    fn insert_or_move(&mut self, entity: Entity, cell: IVec2) {
        if let Some(&old) = self.entity_cell.get(&entity) {
            if old == cell {
                return;
            }
            self.remove_from_bucket(entity, old);
        }
        self.entity_cell.insert(entity, cell);
        self.by_cell.entry(cell).or_default().push(entity);
    }

    fn remove_entity(&mut self, entity: Entity) {
        if let Some(old) = self.entity_cell.remove(&entity) {
            self.remove_from_bucket(entity, old);
        }
    }

    fn remove_from_bucket(&mut self, entity: Entity, cell: IVec2) {
        if let Some(bucket) = self.by_cell.get_mut(&cell) {
            if let Some(idx) = bucket.iter().position(|&e| e == entity) {
                bucket.swap_remove(idx);
            }
            if bucket.is_empty() {
                self.by_cell.remove(&cell);
            }
        }
    }
}

#[allow(clippy::type_complexity)]
#[allow(clippy::too_many_arguments)]
fn drive_instance_residency(
    mut commands: Commands,
    mut state: ResMut<InstanceResidency>,
    mut spawner: Option<ResMut<WorldInstanceSpawner>>,
    mode: Option<Res<crate::app::RunMode>>,
    players: Query<&GlobalTransform, With<Player>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    all_instances: Query<(Entity, &ResidentInstance, &GlobalTransform)>,
    spatial_updates: Query<
        (Entity, &ResidentInstance, &GlobalTransform),
        Or<(Added<ResidentInstance>, Changed<GlobalTransform>)>,
    >,
    state_changed: Query<
        (),
        (
            With<ResidentInstance>,
            Or<(Added<WorldAssetRoot>, Added<WorldInstance>)>,
        ),
    >,
    mut removed_instances: RemovedComponents<ResidentInstance>,
    mut removed_root: RemovedComponents<WorldAssetRoot>,
    mut removed_world_inst: RemovedComponents<WorldInstance>,
    residents: Query<
        (Entity, &ResidentInstance, &GlobalTransform, Option<&WorldInstance>),
        Or<(With<WorldAssetRoot>, With<WorldInstance>)>,
    >,
    asleep_instances: Query<
        (&ResidentInstance, &GlobalTransform),
        (Without<WorldAssetRoot>, Without<WorldInstance>),
    >,
    mut frame: Local<u32>,
    mut cache: Local<InstanceResidencyCache>,
) {
    let _probe = bevy_engine_core::perf_probe::Probe::new("residency.instance");
    // `Local` chega por `Deref`; o re-empréstimo deixa os campos serem
    // emprestados em separado (`by_cell.get` + `to_load.push`).
    let cache = &mut *cache;
    *frame = frame.wrapping_add(1);
    if mode.as_deref() == Some(&crate::app::RunMode::Edit) {
        return;
    }

    // Drain component removals & spatial updates EVERY frame so 2-frame Bevy
    // removal events are never lost between EVAL_EVERY=6 ticks.
    for entity in removed_instances.read() {
        cache.remove_entity(entity);
        cache.dirty = true;
    }
    for _ in removed_root.read() {
        cache.dirty = true;
    }
    for _ in removed_world_inst.read() {
        cache.dirty = true;
    }
    if !state_changed.is_empty() {
        cache.dirty = true;
    }
    if !cache.seeded {
        for (entity, spec, global) in &all_instances {
            cache.insert_or_move(entity, cell_of(global.translation()));
            cache.max_radius = cache.max_radius.max(spec.radius);
        }
        cache.seeded = true;
        cache.dirty = true;
    } else {
        for (entity, spec, global) in &spatial_updates {
            cache.insert_or_move(entity, cell_of(global.translation()));
            cache.max_radius = cache.max_radius.max(spec.radius);
            cache.dirty = true;
        }
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

    let focus_moved = cache
        .last_focus
        .is_none_or(|prev| prev.distance_squared(focus) > 0.25);
    let eye_moved = match (cache.last_eye, eye) {
        (None, None) => false,
        (Some(a), Some(b)) => a.distance_squared(b) > 0.25,
        _ => true,
    };
    if !cache.dirty && !cache.has_overflow && !focus_moved && !eye_moved {
        return;
    }
    cache.dirty = false;
    cache.last_focus = Some(focus);
    cache.last_eye = eye;

    cache.to_load.clear();
    cache.to_unload.clear();
    let mut resident = 0usize;
    for (entity, spec, global, _instance) in &residents {
        resident += 1;
        let p = global.translation();
        let d_sq = focus
            .distance_squared(p)
            .min(eye.map_or(f32::MAX, |e| e.distance_squared(p)));
        let cutoff = spec.radius * UNLOAD_HYSTERESIS;
        if d_sq > cutoff * cutoff {
            cache.to_unload.push((d_sq, entity));
        }
    }

    let search_r = cache.max_radius;
    if search_r > 0.0 {
        let search_r_sq = search_r * search_r;
        let f_min_x = ((focus.x - search_r) / CELL_SIZE).floor() as i32;
        let f_max_x = ((focus.x + search_r) / CELL_SIZE).floor() as i32;
        let f_min_z = ((focus.z - search_r) / CELL_SIZE).floor() as i32;
        let f_max_z = ((focus.z + search_r) / CELL_SIZE).floor() as i32;

        let mut scan_cell = |cell: IVec2| {
            if cell_dist_sq_xz(cell, focus) > search_r_sq
                && !eye.is_some_and(|e| cell_dist_sq_xz(cell, e) <= search_r_sq)
            {
                return;
            }
            let Some(entities) = cache.by_cell.get(&cell) else {
                return;
            };
            for &entity in entities {
                let Ok((spec, global)) = asleep_instances.get(entity) else {
                    continue;
                };
                let p = global.translation();
                let d_sq = focus
                    .distance_squared(p)
                    .min(eye.map_or(f32::MAX, |e| e.distance_squared(p)));
                if d_sq <= spec.radius * spec.radius {
                    cache.to_load.push((d_sq, entity));
                }
            }
        };

        for cz in f_min_z..=f_max_z {
            for cx in f_min_x..=f_max_x {
                scan_cell(IVec2::new(cx, cz));
            }
        }
        if let Some(e) = eye {
            let e_min_x = ((e.x - search_r) / CELL_SIZE).floor() as i32;
            let e_max_x = ((e.x + search_r) / CELL_SIZE).floor() as i32;
            let e_min_z = ((e.z - search_r) / CELL_SIZE).floor() as i32;
            let e_max_z = ((e.z + search_r) / CELL_SIZE).floor() as i32;
            for cz in e_min_z..=e_max_z {
                for cx in e_min_x..=e_max_x {
                    if cx >= f_min_x && cx <= f_max_x && cz >= f_min_z && cz <= f_max_z {
                        continue;
                    }
                    scan_cell(IVec2::new(cx, cz));
                }
            }
        }
    }

    let asleep = cache.entity_cell.len().saturating_sub(resident);
    cache.has_overflow = cache.to_load.len() > LOAD_BUDGET || cache.to_unload.len() > UNLOAD_BUDGET;
    // Perto primeiro a montar; longe primeiro a desmontar.
    cache.to_load.sort_by(|a, b| a.0.total_cmp(&b.0));
    cache.to_unload.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut loaded = 0usize;
    for &(_, entity) in cache.to_load.iter().take(LOAD_BUDGET) {
        let Ok((spec, _)) = asleep_instances.get(entity) else {
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
    for &(_, entity) in cache.to_unload.iter().take(UNLOAD_BUDGET) {
        let Ok((_, _, _, instance)) = residents.get(entity) else {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spatial_cache_mounts_near_and_unmounts_far() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(bevy::transform::TransformPlugin)
            .add_plugins(InstanceResidencyPlugin);

        let player = app
            .world_mut()
            .spawn((
                Player::default(),
                Transform::from_xyz(0.0, 0.0, 0.0),
                GlobalTransform::from_xyz(0.0, 0.0, 0.0),
            ))
            .id();
        let near = app
            .world_mut()
            .spawn((
                ResidentInstance::new(Handle::default(), 50.0, false),
                Transform::from_xyz(10.0, 0.0, 0.0),
                GlobalTransform::from_xyz(10.0, 0.0, 0.0),
            ))
            .id();
        let far = app
            .world_mut()
            .spawn((
                ResidentInstance::new(Handle::default(), 50.0, false),
                Transform::from_xyz(500.0, 0.0, 0.0),
                GlobalTransform::from_xyz(500.0, 0.0, 0.0),
            ))
            .id();

        for _ in 0..EVAL_EVERY {
            app.update();
        }
        assert!(app.world().get::<WorldAssetRoot>(near).is_some());
        assert!(app.world().get::<WorldAssetRoot>(far).is_none());
        let state = app.world().resource::<InstanceResidency>();
        assert_eq!(state.resident, 1);
        assert_eq!(state.asleep, 1);

        // Move player near `far` -> mounts `far` and unmounts `near`.
        app.world_mut()
            .get_mut::<Transform>(player)
            .unwrap()
            .translation = Vec3::new(490.0, 0.0, 0.0);
        for _ in 0..EVAL_EVERY {
            app.update();
        }
        assert!(app.world().get::<WorldAssetRoot>(far).is_some());
        assert!(app.world().get::<WorldAssetRoot>(near).is_none());
    }
}

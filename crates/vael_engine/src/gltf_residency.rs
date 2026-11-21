//! Residência posicional dos `<GltfScene>` do mundo estático.
//!
//! O mundo do Vael tem ~9 mil `<GltfScene>` (as instâncias de prefab
//! incluídas) sobre um pool de ~550 GLB × 3 tiers. O caminho antigo pedia o
//! handle de **todos** no `Startup` — o `AssetServer` ficava com ~1,5 GB na
//! fila antes do primeiro frame — e o véu só saía com o último deles
//! resolvido (`world_ready` conta `GltfScenePending`). Resultado: dezenas de
//! segundos de carregamento para ver o acampamento inicial.
//!
//! Aqui a âncora do `<GltfScene>` nasce já (Transform, nome, script,
//! colisor — tudo o que é da entidade) e o **GLB passa a ser pedido por
//! distância**:
//!
//! * [`RING_READY`] m — o anel que segura o véu. É só isto que o
//!   `world_ready` espera; o resto entra em segundo plano.
//! * o raio efetivo de uma instância é o **corte de render** que ela já
//!   aprendeu (`CullDistance`, do `AutoCull`), com um piso no anel do véu —
//!   ou o *bootstrap* quando é a primeira vez que se vê aquela URL. Assim um
//!   arbusto (corte ~85 m) deixa de estar residente a 800 m, e uma muralha
//!   (corte no teto, 800 m) continua a carregar antes de ser visível.
//! * a descarga tem histerese ([`UNLOAD_HYSTERESIS`]) e nunca toca em nós com
//!   script, animação ligada ou no herói: só cenário.
//!
//! Aprendizagem por URL ([`GltfResidency::learned`]): a primeira instância de
//! cada modelo paga o *bootstrap*; as outras ~centenas da mesma URL já usam o
//! corte observado. É o que impede o raio grande de puxar o pool inteiro.
//!
//! `VIBER_GLTF_STREAMING=0` volta ao comportamento antigo (tudo no boot).

use std::collections::HashMap;
use std::sync::OnceLock;

use bevy::prelude::*;
use bevy::world_serialization::{WorldAssetRoot, WorldInstance, WorldInstanceSpawner};

use crate::animation::{AnimatedScene, CharacterAnimator};
use crate::luau::LuaScriptRef;
use crate::player::Player;
use crate::recipes::spawn::GltfScenePending;
use crate::render_lod::{AutoCull, CullDistance, CullRangeApplied, MeshLod};
use crate::world_ready::WorldReadiness;

/// Raio do anel que segura o véu de carregamento (m).
///
/// É o "campo próximo" do arranque. Medido no acampamento de `--entry dev`
/// (cidade densa): 342 instâncias a 60 m, 862 a 150 m, 1285 a 200 m de um
/// mundo de ~4200 `<GltfScene>`. O resto do mundo entra logo a seguir, por
/// ordem de distância — e o corte do `AutoCull` decide o que fica residente.
pub const RING_READY: f32 = 150.0;

/// Raio de *bootstrap* (m): quanto carregar de uma instância cujo corte de
/// render ainda não se conhece.
///
/// Tem de cobrir o teto do corte automático (`render_lod::AUTO_CULL_MAX`,
/// 800 m) — senão uma muralha aparecia de repente a meio do vale. Paga-se uma
/// vez por URL: assim que a primeira instância assenta, o `AutoCull` mede o
/// bbox e o raio desce ao que aquele modelo precisa.
pub const RING_LOAD: f32 = 800.0;

/// Folga (m) entre o corte de render e a carga/descarga: o objeto carrega
/// enquanto ainda está invisível e só sai depois de o ser há muito.
pub const CULL_MARGIN: f32 = 60.0;

/// Descarga só bem depois da carga (factor sobre o raio de residência): um
/// herói a passear na fronteira não pode andar a carregar/descarregar o mesmo
/// celeiro.
pub const UNLOAD_HYSTERESIS: f32 = 1.3;

/// Activações por frame em jogo (o resto fica para os frames seguintes, por
/// ordem de distância).
///
/// Durante o carregamento o anel largo **não** corre: ele competiria com o
/// anel do véu e com o bake dos colliders do terreno, e mediu-se ~1 s por
/// frame nesse arranque. Aqui o véu fecha primeiro; o resto entra depois.
pub const DEFAULT_LOAD_BUDGET: usize = 8;

/// Em jogo, a residência avalia-se a cada N frames.
const EVAL_EVERY: u32 = 4;

/// Descargas por frame (um teleporte não pode desmontar o mundo num frame).
pub const DEFAULT_UNLOAD_BUDGET: usize = 8;

/// `VIBER_GLTF_STREAMING=0|false|off` devolve o carregamento antigo.
pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        !matches!(
            bevy_engine_core::envs::var_lossy("GLTF_STREAMING").as_deref(),
            Some("0" | "false" | "off")
        )
    })
}

fn env_radius(key: &str, default: f32) -> f32 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<f32>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
        .map_or(default, |v| v)
}

fn env_budget(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(default)
}

/// Raio do anel do véu (m). `VIBER_GLTF_READY_RADIUS` substitui-o.
///
/// Desligada a residência devolve `INFINITY`: o `world_ready` volta a esperar
/// todos os pendentes, como antes.
pub fn ready_radius() -> f32 {
    if !enabled() {
        return f32::INFINITY;
    }
    static R: OnceLock<f32> = OnceLock::new();
    *R.get_or_init(|| {
        env_radius("VIBER_GLTF_READY_RADIUS", RING_READY) * crate::graphics::cull_scale()
    })
}

/// Raio de *bootstrap* (m). `VIBER_GLTF_LOAD_RADIUS` substitui-o.
pub fn load_radius() -> f32 {
    static R: OnceLock<f32> = OnceLock::new();
    *R.get_or_init(|| {
        env_radius("VIBER_GLTF_LOAD_RADIUS", RING_LOAD) * crate::graphics::cull_scale()
    })
}

fn load_budget() -> usize {
    static B: OnceLock<usize> = OnceLock::new();
    *B.get_or_init(|| env_budget("VIBER_GLTF_LOAD_BUDGET", DEFAULT_LOAD_BUDGET))
}

fn unload_budget() -> usize {
    static B: OnceLock<usize> = OnceLock::new();
    *B.get_or_init(|| env_budget("VIBER_GLTF_UNLOAD_BUDGET", DEFAULT_UNLOAD_BUDGET))
}

/// O que um `<GltfScene>` precisa para voltar a pedir o GLB: vive na âncora
/// desde o spawn e nunca sai. Enquanto não há [`GltfScenePending`] nem
/// `WorldAssetRoot`, a instância está **adiada** (só a âncora existe).
#[derive(Component, Debug, Clone, Reflect)]
#[reflect(Component)]
pub struct GltfSceneSpec {
    pub url: String,
    /// Ladder autoral (`lod1-url`/`lod2-url`); `None` manda descobrir os
    /// irmãos `_lod1`/`_lod2` na activação.
    pub lod1: Option<String>,
    pub lod2: Option<String>,
    pub near: f32,
    pub mid: f32,
    /// Candidato ao batch de vegetação (cena de um só mesh, sem
    /// script/física/props) — o marcador é re-armado em cada activação.
    pub foliage: bool,
}

impl GltfSceneSpec {
    /// A URL pede o patch transmissivo do `prop_tint` (cristais/vidro).
    pub fn transmissive(&self) -> bool {
        let path = self.url.as_str();
        path.contains("crystal") || path.contains("glass")
    }
}

/// O GLB desta instância falhou (asset corrompido/ausente): a residência não
/// volta a pedi-lo — sem isto um URL quebrado re-activava-se todos os frames e
/// gastava o orçamento do anel largo para sempre.
#[derive(Component, Debug, Reflect)]
#[reflect(Component)]
pub struct GltfSceneFailed;

/// Estado da residência: contadores para o profiler/QA e a memória das URLs.
#[derive(Resource, Default, Debug)]
pub struct GltfResidency {
    /// Instâncias com o GLB pedido (residentes ou em voo).
    pub resident: usize,
    /// Âncoras sem GLB, à espera de proximidade.
    pub deferred: usize,
    /// Pedidos de GLB feitos desde o arranque.
    pub activated_total: u64,
    /// Descargas desde o arranque.
    pub unloaded_total: u64,
    /// Ladder implícita por URL (`sibling_lods` faz IO): memo.
    ladders: HashMap<String, (Option<String>, Option<String>)>,
    /// Corte de render observado por URL (m) — a maior instância manda, para
    /// uma cópia em escala não aparecer mais cedo do que a irmã.
    learned: HashMap<String, f32>,
}

impl GltfResidency {
    /// Ladder do modelo: a autoral, ou os irmãos `_lod1`/`_lod2` memoizados.
    fn ladder(
        &mut self,
        url: &str,
        authored1: Option<String>,
        authored2: Option<String>,
    ) -> (Option<String>, Option<String>) {
        if authored1.is_some() || authored2.is_some() {
            return (authored1, authored2);
        }
        if let Some(hit) = self.ladders.get(url) {
            return hit.clone();
        }
        let hit = crate::recipes::spawn::sibling_lods(url);
        self.ladders.insert(url.to_owned(), hit.clone());
        hit
    }

    /// Corte de render aprendido para a URL (m), se alguma instância já o deu.
    pub fn learned_cull(&self, url: &str) -> Option<f32> {
        self.learned.get(url).copied()
    }

    fn learn(&mut self, url: &str, radius: f32) -> bool {
        if let Some(slot) = self.learned.get_mut(url) {
            if radius > *slot {
                *slot = radius;
                return true;
            }
            return false;
        }
        self.learned.insert(url.to_owned(), radius);
        true
    }
}

const CELL_SIZE: f32 = 128.0;

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
pub struct GltfResidencyCache {
    seeded: bool,
    entity_cell: HashMap<Entity, IVec2>,
    by_cell: HashMap<IVec2, Vec<Entity>>,
    max_learned_cull: f32,
    dirty: bool,
    has_load_overflow: bool,
    has_unload_overflow: bool,
    last_booting: Option<bool>,
    last_focus: Option<Vec3>,
    last_eye: Option<Vec3>,
    to_load: Vec<(f32, f32, Entity)>,
    to_unload: Vec<Entity>,
}

impl GltfResidencyCache {
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

/// Raio (m) em que uma instância precisa de estar residente.
///
/// O corte conhecido manda (um arbusto desaparece aos ~85 m: não faz sentido
/// ter o GLB a 800); o anel do véu é o piso, para o campo próximo estar sempre
/// quente. Sem corte conhecido, *bootstrap*.
pub fn residency_radius(learned: Option<f32>) -> f32 {
    let floor = ready_radius();
    learned.map_or_else(load_radius, |cull| cull.max(floor) + CULL_MARGIN)
}

/// O nó (ou um ancestral) tem script? Um NPC com `vael.state` não pode ver a
/// sua subárvore desmontada por distância — o `CharacterAnimator` guarda
/// entidades do rig.
fn scripted(
    entity: Entity,
    parents: &Query<&ChildOf>,
    scripts: &Query<(), With<LuaScriptRef>>,
) -> bool {
    let mut current = Some(entity);
    let mut depth = 0;
    while let Some(node) = current {
        if scripts.contains(node) {
            return true;
        }
        current = parents.get(node).ok().map(|c| c.parent());
        depth += 1;
        if depth > 16 {
            return true;
        }
    }
    false
}

/// Decide, por frame, que `<GltfScene>` pedem o GLB e quais o largam.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::type_complexity)]
pub fn drive_gltf_residency(
    mut commands: Commands,
    mut residency: ResMut<GltfResidency>,
    mut spawner: Option<ResMut<WorldInstanceSpawner>>,
    // Os acessos vão em DOIS tuplos: um sistema do Bevy aceita no máximo 16
    // parâmetros e este driver precisa de 26 (cache espacial, `Added`/`Changed`
    // e `RemovedComponents` do streaming). Um tuplo é UM `SystemParam` — os
    // nomes e os acessos no corpo ficam exatamente os mesmos.
    world: (
        Option<Res<AssetServer>>,
        Option<Res<crate::app::RunMode>>,
        Query<&GlobalTransform, With<crate::editor_camera::EditorCamera>>,
        crate::editor_camera::EyeCameraQuery,
        Query<Option<&GlobalTransform>, With<Player>>,
        Query<(), With<LuaScriptRef>>,
        Query<&ChildOf>,
        Option<Res<WorldReadiness>>,
        Option<Res<Time>>,
        Local<f32>,
        // Último `files` visto no `DecodeStats` — o log do decode só escreve
        // quando entraram ficheiros novos no intervalo.
        Local<u64>,
        Local<u32>,
        Local<GltfResidencyCache>,
    ),
    anchors: (
        Query<
            (
                Entity,
                &GlobalTransform,
                Option<&CullDistance>,
                &GltfSceneSpec,
            ),
            (With<GltfSceneSpec>, Without<GltfSceneFailed>),
        >,
        Query<
            (Entity, &GlobalTransform),
            (
                With<GltfSceneSpec>,
                Without<GltfSceneFailed>,
                Or<(Added<GltfSceneSpec>, Changed<GlobalTransform>)>,
            ),
        >,
        Query<
            (&GltfSceneSpec, &CullDistance),
            (
                With<GltfSceneSpec>,
                Without<GltfSceneFailed>,
                Or<(Added<CullDistance>, Changed<CullDistance>)>,
            ),
        >,
        Query<
            (),
            (
                With<GltfSceneSpec>,
                Or<(
                    Added<GltfScenePending>,
                    Added<WorldAssetRoot>,
                    Added<WorldInstance>,
                    Added<GltfSceneFailed>,
                )>,
            ),
        >,
        Query<Entity, (With<GltfSceneSpec>, Added<GltfSceneFailed>)>,
        RemovedComponents<GltfSceneSpec>,
        RemovedComponents<WorldAssetRoot>,
        RemovedComponents<GltfScenePending>,
        Query<
            (
                Entity,
                &GltfSceneSpec,
                &GlobalTransform,
                Option<&CullDistance>,
                Option<&WorldInstance>,
                Has<CharacterAnimator>,
            ),
            (
                With<GltfSceneSpec>,
                Without<GltfSceneFailed>,
                Or<(
                    With<GltfScenePending>,
                    With<WorldAssetRoot>,
                    With<WorldInstance>,
                )>,
            ),
        >,
        Query<
            (&GltfSceneSpec, &GlobalTransform, Option<&CullDistance>),
            (
                Without<GltfSceneFailed>,
                Without<GltfScenePending>,
                Without<WorldAssetRoot>,
                Without<WorldInstance>,
            ),
        >,
    ),
) {
    let _probe = bevy_engine_core::perf_probe::Probe::new("residency.gltf");
    let (
        server,
        mode,
        editor,
        cameras,
        players,
        scripts,
        parents,
        readiness,
        time,
        mut clock,
        mut decode_files,
        mut eval_frame,
        mut cache,
    ) = world;
    let (
        all_anchors,
        spatial_updates,
        cull_updates,
        state_changed,
        failed_added,
        mut removed_specs,
        mut removed_root,
        mut removed_pending,
        residents,
        deferred_only,
    ) = anchors;
    // `Local` chega aqui por `Deref`: sem este re-empréstimo o borrow checker
    // tratava `cache.by_cell.get(..)` e `cache.to_load.push(..)` (campos
    // disjuntos) como empréstimos do `Local` inteiro e recusava o corpo.
    let cache = &mut *cache;
    if !enabled() {
        return;
    }
    // No editor o mundo não "joga": nada de desmontar cenário sob a câmara.
    if mode.as_deref() == Some(&crate::app::RunMode::Edit) {
        return;
    }

    // Maintain spatial cell index and drain RemovedComponents EVERY frame so no
    // 2-frame Bevy removal event can expire during EVAL_EVERY throttling.
    for entity in removed_specs.read() {
        cache.remove_entity(entity);
        cache.dirty = true;
    }
    for entity in &failed_added {
        cache.remove_entity(entity);
        cache.dirty = true;
    }
    for _ in removed_root.read() {
        cache.dirty = true;
    }
    for _ in removed_pending.read() {
        cache.dirty = true;
    }
    if !state_changed.is_empty() {
        cache.dirty = true;
    }
    if !cache.seeded {
        for (entity, global, cull, spec) in &all_anchors {
            cache.insert_or_move(entity, cell_of(global.translation()));
            if let Some(cull) = cull {
                if residency.learn(&spec.url, cull.radius) {
                    cache.max_learned_cull = cache.max_learned_cull.max(cull.radius);
                }
            }
        }
        cache.seeded = true;
        cache.dirty = true;
    } else {
        for (entity, global) in &spatial_updates {
            cache.insert_or_move(entity, cell_of(global.translation()));
            cache.dirty = true;
        }
        for (spec, cull) in &cull_updates {
            if residency.learn(&spec.url, cull.radius) {
                cache.max_learned_cull = cache.max_learned_cull.max(cull.radius);
            }
            cache.dirty = true;
        }
    }

    let Some(server) = server.as_deref() else {
        return;
    };
    // O herói manda; a câmara cobre as cinemáticas (que voam para longe dele).
    let hero = players.iter().flatten().next().map(|t| t.translation());
    let eye = crate::editor_camera::active_camera_eye(&editor, &cameras).map(|t| t.translation());
    let Some(focus) = hero.or(eye) else {
        return;
    };
    let ready_r = ready_radius();
    let booting = readiness.as_deref().is_none_or(|r| !r.ready);
    // Em jogo a varredura (milhares de âncoras) corre a cada
    // `EVAL_EVERY` frames, com o orçamento multiplicado na mesma proporção
    // (~0,65 ms/frame poupados, medido 2026-11-16). No arranque, todos.
    *eval_frame = eval_frame.wrapping_add(1);
    if !booting && *eval_frame % EVAL_EVERY != 0 {
        return;
    }

    let focus_moved = cache
        .last_focus
        .is_none_or(|prev| prev.distance_squared(focus) > 0.25);
    let eye_moved = match (cache.last_eye, eye) {
        (None, None) => false,
        (Some(a), Some(b)) => a.distance_squared(b) > 0.25,
        _ => true,
    };
    let booting_changed = cache.last_booting != Some(booting);
    let need_eval = cache.dirty
        || cache.has_load_overflow
        || cache.has_unload_overflow
        || focus_moved
        || eye_moved
        || booting_changed;
    if !need_eval {
        return;
    }

    cache.dirty = false;
    cache.last_focus = Some(focus);
    cache.last_eye = eye;
    cache.last_booting = Some(booting);
    let budget_scale = if booting { 1 } else { EVAL_EVERY as usize };

    cache.to_load.clear();
    cache.to_unload.clear();
    let mut resident = 0usize;
    for (entity, spec, global, cull, _instance, animator) in &residents {
        resident += 1;
        let p = global.translation();
        let d_sq = focus
            .distance_squared(p)
            .min(eye.map_or(f32::MAX, |e| e.distance_squared(p)));
        let keep = residency_radius(cull.map(|c| c.radius))
            .max(residency.learned_cull(&spec.url).unwrap_or(0.0));
        let keep_cutoff = keep * UNLOAD_HYSTERESIS;
        if d_sq > keep_cutoff * keep_cutoff {
            // `animator`: a subárvore tem um rig bindado — o
            // `CharacterAnimator` guarda a entidade do `AnimationPlayer`.
            if !animator && !scripted(entity, &parents, &scripts) {
                cache.to_unload.push(entity);
            }
        }
    }

    let search_r = load_radius()
        .max(ready_r + CULL_MARGIN)
        .max(cache.max_learned_cull + CULL_MARGIN);
    let search_r_sq = search_r * search_r;
    let f_min_x = ((focus.x - search_r) / CELL_SIZE).floor() as i32;
    let f_max_x = ((focus.x + search_r) / CELL_SIZE).floor() as i32;
    let f_min_z = ((focus.z - search_r) / CELL_SIZE).floor() as i32;
    let f_max_z = ((focus.z + search_r) / CELL_SIZE).floor() as i32;

    let mut check_cell = |cell: IVec2| {
        if cell_dist_sq_xz(cell, focus) > search_r_sq
            && !eye.is_some_and(|e| cell_dist_sq_xz(cell, e) <= search_r_sq)
        {
            return;
        }
        let Some(entities) = cache.by_cell.get(&cell) else {
            return;
        };
        for &entity in entities {
            let Ok((spec, global, cull)) = deferred_only.get(entity) else {
                continue;
            };
            let p = global.translation();
            let d_sq = focus
                .distance_squared(p)
                .min(eye.map_or(f32::MAX, |e| e.distance_squared(p)));
            let load_at = residency_radius(
                cull.map(|c| c.radius)
                    .or_else(|| residency.learned_cull(&spec.url)),
            );
            if d_sq <= load_at * load_at {
                let d_ready_sq = hero.map_or(f32::MAX, |h| h.distance_squared(p));
                cache.to_load.push((d_sq, d_ready_sq, entity));
            }
        }
    };

    for cz in f_min_z..=f_max_z {
        for cx in f_min_x..=f_max_x {
            check_cell(IVec2::new(cx, cz));
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
                check_cell(IVec2::new(cx, cz));
            }
        }
    }

    let deferred = cache.entity_cell.len().saturating_sub(resident);
    // O mais perto primeiro: o anel do véu fecha antes de o largo começar.
    cache.to_load.sort_by(|a, b| a.0.total_cmp(&b.0));
    // Atrás do véu só entra o anel que o segura: o largo competiria com ele e
    // com o bake dos colliders do terreno (frames de ~1 s medidos no arranque).
    let mut budget = if booting {
        0
    } else {
        load_budget() * budget_scale
    };
    let ready_r_sq = ready_r * ready_r;
    let mut activated = 0usize;
    let mut load_overflow = false;
    for &(_, d_ready_sq, entity) in &cache.to_load {
        let essential = d_ready_sq <= ready_r_sq;
        if !essential {
            if budget == 0 {
                load_overflow = true;
                continue;
            }
            budget -= 1;
        }
        let Ok((spec, _, cull)) = deferred_only.get(entity) else {
            continue;
        };
        activate(
            &mut commands,
            &mut residency,
            server,
            entity,
            spec,
            cull.is_none(),
        );
        activated += 1;
    }
    cache.has_load_overflow = load_overflow && !booting;
    // Os contadores são o retrato do FIM do frame: uma activação conta como
    // residente já neste frame (era `deferred` no início).
    residency.resident = resident + activated;
    residency.deferred = deferred.saturating_sub(activated);

    let mut budget = unload_budget() * budget_scale;
    let mut unloaded = 0usize;
    let mut unload_overflow = false;
    for &entity in &cache.to_unload {
        if budget == 0 {
            unload_overflow = true;
            break;
        }
        budget -= 1;
        let instance = residents.get(entity).ok().and_then(|s| s.4);
        if let (Some(_), Some(instance)) = (spawner.as_mut(), instance) {
            // Só o `WorldInstanceSpawner` sabe desmontar a instância inteira
            // (incluindo entidades que já saíram da hierarquia).
            despawn_instance_now(&mut commands, instance);
        }
        commands.entity(entity).remove::<(
            WorldAssetRoot,
            WorldInstance,
            MeshLod,
            AnimatedScene,
            CullRangeApplied,
            AutoCull,
            GltfScenePending,
            crate::recipes::spawn::PendingLodLadder,
        )>();
        residency.unloaded_total += 1;
        unloaded += 1;
    }
    cache.has_unload_overflow = unload_overflow;
    residency.resident = residency.resident.saturating_sub(unloaded);
    residency.deferred += unloaded;

    // Diagnóstico (`RUST_LOG=bevy_ext_viber_engine::gltf_residency=info`): só
    // escreve quando houve trabalho no intervalo — o streaming é invisível de
    // fora e este é o único rasto para o medir em QA.
    if let Some(time) = time.as_deref() {
        *clock += time.delta_secs();
        if *clock >= 5.0 {
            *clock = 0.0;
            if activated > 0 || unloaded > 0 {
                info!(
                    "gltf: {} residentes, {} adiados, {}+ / {}- neste intervalo (total {} activados, {} descarregados)",
                    residency.resident,
                    residency.deferred,
                    activated,
                    unloaded,
                    residency.activated_total,
                    residency.unloaded_total
                );
            }
            // O custo do decode é invisível de fora e é exactamente o que a
            // passagem única do `bevy_ext_gltf` veio cortar (bytes, tempo de
            // CPU e o pico de memória que o RSS não mostra por si).
            let decode = crate::meshopt::decode_stats();
            if decode.files != *decode_files {
                *decode_files = decode.files;
                info!(
                    "gltf decode: {} ficheiros ({} reescritos), {:.1} MB -> {:.1} MB (×{:.2}), \
                     {:.0} ms de CPU (dequant {:.0} ms), {} relabels UASTC, {} sem suporte",
                    decode.files,
                    decode.rewritten,
                    decode.bytes_in as f64 / 1e6,
                    decode.bytes_out as f64 / 1e6,
                    decode.ratio(),
                    decode.decode_nanos as f64 / 1e6,
                    decode.dequant_nanos as f64 / 1e6,
                    decode.uastc_relabels,
                    decode.unsupported,
                );
            }
        }
    }
}

/// Pede o GLB de uma instância adiada e deixa o resto ao `gltf_scene_spawner`.
fn activate(
    commands: &mut Commands,
    residency: &mut GltfResidency,
    server: &AssetServer,
    entity: Entity,
    spec: &GltfSceneSpec,
    need_auto_cull: bool,
) {
    let (lod1, lod2) = residency.ladder(&spec.url, spec.lod1.clone(), spec.lod2.clone());
    let load =
        |url: &str| crate::meshopt::load_gltf(server, url.trim_start_matches('/').to_owned());
    let handle = load(&spec.url);
    let lod1 = lod1.as_deref().map(load);
    let lod2 = lod2.as_deref().map(load);
    let mut anchor = commands.entity(entity);
    anchor.insert(GltfScenePending {
        handle,
        lod1,
        lod2,
        lod_near: spec.near,
        lod_mid: spec.mid,
    });
    if need_auto_cull {
        anchor.insert(AutoCull);
    }
    if spec.foliage {
        anchor.insert(crate::foliage::FoliageCandidate { multi: false });
    }
    if spec.transmissive() {
        anchor.insert(crate::prop_tint::TransmissiveGltf);
    }
    residency.activated_total += 1;
}

/// Desmonta a instância JÁ (numa command, por ordem): o
/// `WorldInstanceSpawner::despawn_instance` é uma fila, e o hook `on_remove`
/// do `WorldAssetRoot` (removido logo a seguir) desregistava a instância
/// antes de a fila correr — as entidades da cena ficavam no mundo e a
/// "descarga" não libertava nada.
pub fn despawn_instance_now(commands: &mut Commands, instance: &WorldInstance) {
    let id = **instance;
    commands.queue(move |world: &mut World| {
        world.resource_scope(|world, mut spawner: Mut<WorldInstanceSpawner>| {
            spawner.despawn_instance_sync(world, &id);
        });
    });
}

pub struct GltfResidencyPlugin;

impl Plugin for GltfResidencyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GltfResidency>().add_systems(
            // `PostUpdate`, DEPOIS da propagação dos transforms: no `Update`
            // do primeiro frame os `GlobalTransform` ainda são identidade e a
            // decisão via a árvore toda empilhada na origem (tudo "perto").
            PostUpdate,
            drive_gltf_residency
                .after(bevy::transform::TransformSystems::Propagate)
                .before(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_known_cull_radius_beats_the_bootstrap() {
        // Arbusto: corte 85 m ⇒ residente até ao piso do anel do véu + folga.
        let r = residency_radius(Some(85.0));
        assert!(r < load_radius(), "o corte aprendido tem de baixar o raio");
        assert!(r >= ready_radius(), "o anel do véu é o piso");
        // Muralha (corte no teto): mantém-se no bootstrap.
        assert!(residency_radius(Some(800.0)) >= 800.0);
        // Desconhecido: bootstrap.
        assert_eq!(residency_radius(None), load_radius());
    }

    #[test]
    fn learning_keeps_the_largest_radius_of_a_url() {
        let mut r = GltfResidency::default();
        assert_eq!(r.learned_cull("a.glb"), None);
        r.learn("a.glb", 120.0);
        r.learn("a.glb", 300.0);
        r.learn("a.glb", 90.0);
        assert_eq!(r.learned_cull("a.glb"), Some(300.0));
    }

    #[test]
    fn the_ladder_is_resolved_once_per_url() {
        let mut r = GltfResidency::default();
        let first = r.ladder("trees/pine_lod0.glb", None, None);
        let second = r.ladder("trees/pine_lod0.glb", None, None);
        assert_eq!(first, second);
        // Autoral ganha sempre à descoberta.
        let authored = r.ladder(
            "trees/pine_lod0.glb",
            Some("trees/pine_custom_lod1.glb".into()),
            None,
        );
        assert_eq!(authored.0.as_deref(), Some("trees/pine_custom_lod1.glb"));
    }

    fn test_app(hero: Vec3) -> App {
        use bevy::asset::AssetApp;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(bevy::transform::TransformPlugin)
            .add_plugins(bevy::asset::AssetPlugin {
                file_path: "sem-assets".into(),
                ..Default::default()
            })
            .add_plugins(GltfResidencyPlugin);
        app.init_asset::<bevy::gltf::Gltf>();
        app.world_mut().spawn((
            Player::default(),
            Transform::from_translation(hero),
            GlobalTransform::from_translation(hero),
        ));
        app
    }

    fn spec() -> GltfSceneSpec {
        GltfSceneSpec {
            url: "meshes/prop_lod0.glb".into(),
            lod1: None,
            lod2: None,
            near: 45.0,
            mid: 110.0,
            foliage: false,
        }
    }

    fn anchor(app: &mut App, at: Vec3) -> Entity {
        app.world_mut()
            .spawn((
                spec(),
                Transform::from_translation(at),
                GlobalTransform::from_translation(at),
            ))
            .id()
    }

    #[test]
    fn the_near_field_asks_for_the_glb_and_the_far_one_waits() {
        let mut app = test_app(Vec3::ZERO);
        let near = anchor(&mut app, Vec3::new(10.0, 0.0, 0.0));
        let far = anchor(&mut app, Vec3::new(2000.0, 0.0, 0.0));
        app.update();
        assert!(
            app.world().get::<GltfScenePending>(near).is_some(),
            "10 m pede o GLB"
        );
        assert!(
            app.world().get::<GltfScenePending>(far).is_none(),
            "2 km fica adiado"
        );
        let residency = app.world().resource::<GltfResidency>();
        assert_eq!(residency.deferred, 1);
        assert_eq!(residency.resident, 1);
    }

    #[test]
    fn a_small_prop_is_dropped_well_before_the_bootstrap_radius() {
        let mut app = test_app(Vec3::ZERO);
        let prop = anchor(&mut app, Vec3::new(700.0, 0.0, 0.0));
        app.world_mut().entity_mut(prop).insert((
            // Corte aprendido pelo `AutoCull`: 80 m — residente até ~340 m.
            CullDistance::new(80.0),
            WorldAssetRoot(Handle::default()),
        ));
        app.update();
        assert!(
            app.world().get::<WorldAssetRoot>(prop).is_none(),
            "a 700 m um prop de 80 m de corte já não é residente"
        );
        assert_eq!(app.world().resource::<GltfResidency>().unloaded_total, 1);
    }

    #[test]
    fn a_scripted_node_is_never_unloaded() {
        let mut app = test_app(Vec3::ZERO);
        let npc = anchor(&mut app, Vec3::new(700.0, 0.0, 0.0));
        app.world_mut().entity_mut(npc).insert((
            CullDistance::new(80.0),
            WorldAssetRoot(Handle::default()),
            LuaScriptRef {
                path: "scripts/npc.lua".into(),
            },
        ));
        app.update();
        assert!(
            app.world().get::<WorldAssetRoot>(npc).is_some(),
            "NPC com script não perde a subárvore"
        );
    }

    #[test]
    fn spatial_cache_loads_on_approach_and_unloads_when_leaving() {
        let mut app = test_app(Vec3::ZERO);
        // `WorldReadiness` tem campos privados (a época corrente): o default
        // dá-os e só o `ready` público é que o teste precisa de forçar.
        let mut readiness = WorldReadiness::default();
        readiness.ready = true;
        app.insert_resource(readiness);
        let prop = anchor(&mut app, Vec3::new(1000.0, 0.0, 0.0));
        app.world_mut()
            .entity_mut(prop)
            .insert(CullDistance::new(80.0));

        for _ in 0..EVAL_EVERY {
            app.update();
        }
        assert!(app.world().get::<GltfScenePending>(prop).is_none());

        // Move hero close to prop (~100m away).
        let player = app
            .world_mut()
            .query_filtered::<Entity, With<Player>>()
            .single(app.world())
            .unwrap();
        app.world_mut()
            .get_mut::<Transform>(player)
            .unwrap()
            .translation = Vec3::new(900.0, 0.0, 0.0);
        for _ in 0..EVAL_EVERY {
            app.update();
        }
        assert!(app.world().get::<GltfScenePending>(prop).is_some());

        // Stationary ticks keep it resident.
        for _ in 0..EVAL_EVERY {
            app.update();
        }
        assert!(app.world().get::<GltfScenePending>(prop).is_some());

        // Move hero back to origin (1000m > keep * 1.3) -> unloads.
        app.world_mut()
            .get_mut::<Transform>(player)
            .unwrap()
            .translation = Vec3::ZERO;
        for _ in 0..EVAL_EVERY {
            app.update();
        }
        assert!(app.world().get::<GltfScenePending>(prop).is_none());
        assert_eq!(app.world().resource::<GltfResidency>().unloaded_total, 1);
    }

    #[test]
    fn learned_cull_keeps_largest_radius_without_shrinking() {
        let mut residency = GltfResidency::default();
        assert!(residency.learn("trees/oak.glb", 80.0));
        assert!(!residency.learn("trees/oak.glb", 60.0));
        assert_eq!(residency.learned_cull("trees/oak.glb"), Some(80.0));
        assert!(residency.learn("trees/oak.glb", 140.0));
        assert_eq!(residency.learned_cull("trees/oak.glb"), Some(140.0));
    }
}

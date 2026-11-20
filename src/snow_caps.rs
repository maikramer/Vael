//! Neve volumétrica em telhados e props.
//!
//! A neve assenta na superfície **mais alta** de cada ponto. Cada prop
//! exterior do mundo (a raiz com `WorldNode`/`WorldOrigin`, com todas as malhas
//! da subárvore) é rasterizado num mapa de altura do topo em XZ; as células
//! cujo topo é uma face virada para cima (até ~70°) formam uma tampa: uma
//! grelha à altura do topo + a profundidade, com borda arredondada e saia
//! vertical onde o topo acaba ou dá um degrau (beiral, chaminé). A tampa é
//! filha da raiz (morre com ela) e a espessura segue o pack de referência do
//! manto (`SnowState::reference_depth`), com retenção por inclinação:
//! um telhado a 60° quase não guarda.
//!
//! As mesmas células, quando o topo está acima do chão, marcam o
//! [`SnowShelter`]: debaixo de um telhado não neva.
//!
//! Gate: `VAEL_SNOW_CAPS=0` (e `VAEL_SNOW=0`).

use std::collections::{HashMap, HashSet};

use bevy::asset::RenderAssetUsages;
use bevy::math::Affine3A;
use bevy::mesh::{Indices, Mesh, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};
use bevy_ext_world::bsn::{WorldNode, WorldOrigin};
use bevy_flora::PooledTree;
use vael_engine::animation::CharacterAnimator;
use vael_engine::player::Player;
use vael_engine::terrain::{SnowShelter, SnowState, TerrainRuntime};

use crate::environment::elements_enabled;
use crate::ground::{env_enabled, interior_file, is_ground_name, snow_enabled};

/// Albedo linear da neve (o mesmo do chão).
const SNOW_ALBEDO: [f32; 3] = [0.80, 0.83, 0.88];
/// Face mais inclinada que ainda é topo (cos 70°).
const TOP_MIN_NY: f32 = 0.34;
/// Telhados guardam a neve toda até aqui e nada a partir de `SHED_HI_DEG`.
const SHED_LO_DEG: f32 = 42.0;
const SHED_HI_DEG: f32 = 64.0;
/// Topo mais alto que o chão por baixo a partir do qual abriga o chão, m
/// (telhados, alpendres, toldos; não caixotes nem barris).
const SHELTER_MIN_M: f32 = 1.8;
/// Células de lado no máximo (o lado da célula cresce com o prop).
const MAX_CELLS: usize = 160;
/// Espera depois da última malha nova antes de construir (o GLB chega aos
/// bocados), segundos reais.
const SETTLE_S: f32 = 0.75;
/// Mudança de profundidade que reescreve as tampas, metros.
const DEPTH_STEP_M: f32 = 0.004;
/// Tampas reescritas por frame.
const UPDATES_PER_FRAME: usize = 24;

#[must_use]
pub fn snow_caps_enabled() -> bool {
    env_enabled("VAEL_SNOW_CAPS")
}

pub struct VaelSnowCapsPlugin;

impl Plugin for VaelSnowCapsPlugin {
    fn build(&self, app: &mut App) {
        if !(elements_enabled() && snow_enabled() && snow_caps_enabled()) {
            return;
        }
        app.init_resource::<CapQueue>()
            .add_systems(Update, (discover, build, update_caps).chain());
    }
}

/// Malha já vista (pertence a uma raiz em fila ou fora do jogo).
#[derive(Component)]
struct CapScanned;

/// Small surface decals do not contribute to the snow silhouette.
#[derive(Component)]
pub(crate) struct IgnoreSnowMesh;

/// A tampa de uma raiz: posições por profundidade e a grelha que as gera.
#[derive(Component)]
struct SnowCap {
    grid: CapSurface,
    /// Mundo → local da raiz, no momento da construção.
    to_local: Affine3A,
    applied: f32,
}

/// Raiz já construída, com as malhas que entraram (um LOD novo reconstrói).
#[derive(Component)]
struct CapBuilt {
    sources: Vec<(Entity, AssetId<Mesh>)>,
}

#[derive(Resource, Default)]
struct CapQueue {
    /// Raiz → instante real da última malha nova.
    pending: HashMap<Entity, f32>,
    jobs: HashMap<Entity, Task<PreparedCap>>,
    material: Option<Handle<StandardMaterial>>,
}

/// Geometria e abrigo calculados de um snapshot imutável das malhas.
struct PreparedCap {
    sources: Vec<(Entity, AssetId<Mesh>)>,
    to_local: Affine3A,
    grid: Option<CapSurface>,
    shelter_cells: HashSet<IVec2>,
}

enum CapSurface {
    Grid(CapGrid),
    Stone(crate::rocks::snow::SnowSkin),
}

impl CapSurface {
    fn mesh(&self, depth: f32, to_local: &Affine3A) -> Mesh {
        match self {
            Self::Grid(grid) => grid.mesh(depth, to_local),
            Self::Stone(skin) => skin.mesh(depth, to_local),
        }
    }
}

type Blocked = Or<(With<CharacterAnimator>, With<PooledTree>, With<Player>, With<SnowCap>, With<IgnoreSnowMesh>)>;

fn prop_root(
    start: Entity,
    parents: &Query<&ChildOf>,
    nodes: &Query<&WorldNode>,
    origins: &Query<&WorldOrigin>,
    gltfs: &Query<&vael_engine::gltf_residency::GltfSceneSpec>,
) -> Option<(Entity, String)> {
    let mut current = start;
    for _ in 0..24 {
        // Streamed GLBs keep this anchor in standalone play. The editor
        // origin components are optional there.
        if let Ok(spec) = gltfs.get(current) {
            return Some((current, spec.url.clone()));
        }
        if let Ok(origin) = origins.get(current)
            && !origin.file.is_empty()
        {
            return Some((current, origin.file.clone()));
        }
        if let Ok(node) = nodes.get(current) {
            return Some((current, node.origin_file.clone()));
        }
        current = parents.get(current).ok()?.parent();
    }
    None
}

fn ancestor_blocked(start: Entity, parents: &Query<&ChildOf>, blocked: &Query<(), Blocked>) -> bool {
    let mut current = start;
    for _ in 0..24 {
        if blocked.get(current).is_ok() {
            return true;
        }
        let Ok(child) = parents.get(current) else {
            break;
        };
        current = child.parent();
    }
    false
}

#[allow(clippy::too_many_arguments)]
fn discover(
    mut commands: Commands,
    time: Res<Time<Real>>,
    meshes: Query<(Entity, Option<&Name>), (With<Mesh3d>, Without<CapScanned>, Without<SnowCap>)>,
    parents: Query<&ChildOf>,
    nodes: Query<&WorldNode>,
    origins: Query<&WorldOrigin>,
    gltfs: Query<&vael_engine::gltf_residency::GltfSceneSpec>,
    stones: Query<Option<&vael_engine::scene::SceneSource>, With<crate::rocks::QuarryStone>>,
    scene: Option<Res<vael_engine::scene_loader::CurrentScene>>,
    blocked: Query<(), Blocked>,
    mut queue: ResMut<CapQueue>,
) {
    if scene.as_deref().and_then(|s| s.id.as_deref()) != Some("main") { return; }
    const BUDGET: usize = 512;
    let now = time.elapsed_secs();
    let (mut seen, mut rootless, mut skipped) = (0, 0, 0);
    for (entity, name) in meshes.iter().take(BUDGET) {
        seen += 1;
        commands.entity(entity).try_insert(CapScanned);
        // Pavimento e estradas ficam enterrados no manto do chão.
        if name.is_some_and(|name| is_ground_name(name.as_str())) {
            continue;
        }
        let root = stones.get(entity).ok().map(|source| (entity, source.map(|s| s.0.file.display().to_string()).unwrap_or_default()));
        let Some((root, file)) = root.or_else(|| prop_root(entity, &parents, &nodes, &origins, &gltfs)) else {
            rootless += 1;
            continue;
        };
        if interior_file(&file) || ancestor_blocked(entity, &parents, &blocked) {
            skipped += 1;
            continue;
        }
        queue.pending.insert(root, now);
    }
    if seen > 0 {
        debug!(target: "vael::snow", seen, rootless, skipped, pending = queue.pending.len(), "malhas vistas");
    }
}

#[allow(clippy::too_many_arguments)]
fn build(
    mut commands: Commands,
    time: Res<Time<Real>>,
    mut queue: ResMut<CapQueue>,
    roots: Query<(&GlobalTransform, Option<&Children>, Option<&CapBuilt>, Has<crate::rocks::QuarryStone>)>,
    children: Query<&Children>,
    mesh_entities: Query<(&Mesh3d, &GlobalTransform, Option<&Name>), Without<SnowCap>>,
    caps: Query<Entity, With<SnowCap>>,
    blocked: Query<(), Blocked>,
    terrain: Option<Res<TerrainRuntime>>,
    mut shelter: ResMut<SnowShelter>,
    meshes: Res<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let now = time.elapsed_secs();
    let Some(terrain) = terrain else {
        return;
    };
    let mut finished = Vec::new();
    queue.jobs.retain(|&root, task| {
        if let Some(cap) = check_ready(task) {
            finished.push((root, cap));
            false
        } else {
            roots.contains(root)
        }
    });
    let mut shelter_changed = false;
    for (root, cap) in finished {
        let Ok((root_tf, kids, _, _)) = roots.get(root) else {
            continue;
        };
        if queue.pending.contains_key(&root) {
            continue;
        }
        if root_tf.affine().inverse() != cap.to_local
            || cap.sources.iter().any(|(entity, id)| {
                !mesh_entities
                    .get(*entity)
                    .is_ok_and(|(mesh, _, _)| mesh.id() == *id)
            })
        {
            queue.pending.insert(root, now);
            continue;
        }
        if let Some(kids) = kids {
            for kid in kids.iter().filter(|kid| caps.contains(*kid)) {
                commands.entity(kid).despawn();
            }
        }
        commands.entity(root).try_insert(CapBuilt {
            sources: cap.sources,
        });
        for cell in cap.shelter_cells {
            shelter_changed |= shelter.cover_cell(cell);
        }
        if let Some(grid) = cap.grid {
            let material = queue.material.get_or_insert_with(|| {
                materials.add(StandardMaterial {
                    base_color: Color::linear_rgb(SNOW_ALBEDO[0], SNOW_ALBEDO[1], SNOW_ALBEDO[2]),
                    perceptual_roughness: 0.82,
                    reflectance: 0.35,
                    double_sided: true,
                    cull_mode: None,
                    ..default()
                })
            });
            commands.entity(root).with_child((
                Name::new("snow cap"),
                SnowCap {
                    grid,
                    to_local: cap.to_local,
                    applied: 0.0,
                },
                MeshMaterial3d(material.clone()),
                Transform::IDENTITY,
                Visibility::Hidden,
                CapScanned,
            ));
        }
    }
    if shelter_changed {
        shelter.generation = shelter.generation.wrapping_add(1);
    }
    // Duas construções em voo limitam os buffers temporários durante o streaming.
    let ready: Vec<Entity> = queue
        .pending
        .iter()
        .filter(|(root, at)| now - **at >= SETTLE_S && !queue.jobs.contains_key(root))
        .map(|(root, _)| *root)
        .take(2usize.saturating_sub(queue.jobs.len()))
        .collect();
    for root in ready {
        queue.pending.remove(&root);
        let Ok((root_tf, _, built, stone)) = roots.get(root) else {
            continue;
        };
        // Todas as malhas da subárvore, em mundo.
        let mut sources = Vec::new();
        let mut missing = false;
        let mut stack = vec![root];
        while let Some(entity) = stack.pop() {
            if caps.get(entity).is_ok() || (entity != root && blocked.get(entity).is_ok()) {
                continue;
            }
            if let Ok((mesh, _, name)) = mesh_entities.get(entity) {
                if !name.is_some_and(|name| is_ground_name(name.as_str())) {
                    match meshes.get(&mesh.0) {
                        Some(_) => sources.push((entity, mesh.id())),
                        None => missing = true,
                    }
                }
            }
            if let Ok(kids) = children.get(entity) {
                stack.extend(kids.iter());
            }
        }
        if missing {
            // O GLB ainda não chegou todo: volta à fila.
            queue.pending.insert(root, now);
            continue;
        }
        if built.is_some_and(|b| b.sources == sources) {
            continue;
        }
        let mut tris = Vec::new();
        for &(entity, id) in &sources {
            if let (Some(mesh), Ok((_, tf, _))) = (meshes.get(id), mesh_entities.get(entity)) {
                append_triangles(mesh, tf, &mut tris);
            }
        }
        let to_local = root_tf.affine().inverse();
        let stone_skin = if stone {
            sources.iter().find(|(entity, _)| *entity == root)
                .and_then(|(_, id)| meshes.get(*id))
                .and_then(|mesh| crate::rocks::snow::SnowSkin::from_mesh(mesh, root_tf))
        } else { None };
        let reader = terrain.reader();
        let task = AsyncComputeTaskPool::get().spawn(async move {
            let mut shelter_cells = HashSet::new();
            let grid = CapGrid::rasterize(&tris).filter(|grid| {
                for (xz, top) in grid.top_cells() {
                    let cell = (xz / vael_engine::terrain::ground::SNOW_CELL_M)
                        .floor()
                        .as_ivec2();
                    if !shelter_cells.contains(&cell)
                        && top - reader.sample(xz.x, xz.y) > SHELTER_MIN_M
                    {
                        shelter_cells.insert(cell);
                    }
                }
                (0..grid.nz - 1).any(|z| (0..grid.nx - 1).any(|x| grid.has_quad(x, z)))
            });
            PreparedCap {
                sources,
                to_local,
                grid: stone_skin.map(CapSurface::Stone).or_else(|| grid.map(CapSurface::Grid)),
                shelter_cells,
            }
        });
        queue.jobs.insert(root, task);
    }
}

fn update_caps(
    pack: Option<Res<SnowState>>,
    mut commands: Commands,
    mut caps: Query<(Entity, &mut SnowCap, Option<&Mesh3d>, &mut Visibility)>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let depth = pack.as_deref().map_or(0.0, SnowState::reference_depth);
    let mut done = 0;
    for (entity, mut cap, mesh, mut visibility) in &mut caps {
        if (cap.applied - depth).abs() < DEPTH_STEP_M && (depth >= DEPTH_STEP_M) == (cap.applied >= DEPTH_STEP_M) {
            continue;
        }
        if done >= UPDATES_PER_FRAME {
            break;
        }
        done += 1;
        cap.applied = depth;
        let show = depth >= DEPTH_STEP_M;
        let want = if show { Visibility::Inherited } else { Visibility::Hidden };
        if *visibility != want {
            *visibility = want;
        }
        if !show {
            continue;
        }
        let fresh = cap.grid.mesh(depth, &cap.to_local);
        if let Some(mut asset) = mesh.and_then(|mesh| meshes.get_mut(&mesh.0)) {
            *asset = fresh;
        } else {
            commands.entity(entity).insert(Mesh3d(meshes.add(fresh)));
        }
    }
}

fn append_triangles(mesh: &Mesh, tf: &GlobalTransform, out: &mut Vec<[Vec3; 3]>) {
    if mesh.primitive_topology() != PrimitiveTopology::TriangleList {
        return;
    }
    let Some(VertexAttributeValues::Float32x3(pos)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
        return;
    };
    let affine = tf.affine();
    let world: Vec<Vec3> = pos
        .iter()
        .map(|p| affine.transform_point3(Vec3::from_array(*p)))
        .collect();
    let mut push = |a: usize, b: usize, c: usize| {
        if let (Some(a), Some(b), Some(c)) = (world.get(a), world.get(b), world.get(c)) {
            out.push([*a, *b, *c]);
        }
    };
    match mesh.indices() {
        Some(Indices::U16(idx)) => idx
            .chunks_exact(3)
            .for_each(|t| push(t[0] as usize, t[1] as usize, t[2] as usize)),
        Some(Indices::U32(idx)) => idx
            .chunks_exact(3)
            .for_each(|t| push(t[0] as usize, t[1] as usize, t[2] as usize)),
        None => (0..world.len() / 3).for_each(|t| push(3 * t, 3 * t + 1, 3 * t + 2)),
    }
}

/// Quanto um topo inclinado guarda. 1 até [`SHED_LO_DEG`], 0 a partir de
/// [`SHED_HI_DEG`].
#[must_use]
pub fn roof_retention(normal_y: f32) -> f32 {
    let deg = normal_y.clamp(-1.0, 1.0).acos().to_degrees();
    1.0 - smoothstep(SHED_LO_DEG, SHED_HI_DEG, deg)
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Mapa de altura do topo de um prop e o que dele vira tampa.
#[derive(Clone, Debug)]
pub struct CapGrid {
    origin: Vec2,
    cell: f32,
    nx: usize,
    nz: usize,
    /// Altura do topo por célula (`NaN` = nada).
    top: Vec<f32>,
    /// Peso da espessura 0..1 (inclinação × borda × ilha), 0 fora da tampa.
    weight: Vec<f32>,
}

impl CapGrid {
    /// Rasteriza os triângulos (mundo) no topo. `None` sem nenhuma célula de
    /// topo virada para cima.
    #[must_use]
    pub fn rasterize(tris: &[[Vec3; 3]]) -> Option<Self> {
        if tris.is_empty() {
            return None;
        }
        let mut min = Vec2::splat(f32::INFINITY);
        let mut max = Vec2::splat(f32::NEG_INFINITY);
        for tri in tris {
            for p in tri {
                min = min.min(p.xz());
                max = max.max(p.xz());
            }
        }
        let extent = (max - min).max_element();
        if !extent.is_finite() || extent <= 0.05 {
            return None;
        }
        let cell = (extent / 48.0).clamp(0.08, 0.35);
        let nx = (((max.x - min.x) / cell).ceil() as usize + 1).min(MAX_CELLS);
        let nz = (((max.y - min.y) / cell).ceil() as usize + 1).min(MAX_CELLS);
        let cell = cell.max((max.x - min.x) / (nx - 1).max(1) as f32).max((max.y - min.y) / (nz - 1).max(1) as f32);
        let mut top = vec![f32::NAN; nx * nz];
        let mut up = vec![0.0f32; nx * nz];
        for tri in tris {
            let n = (tri[1] - tri[0]).cross(tri[2] - tri[0]);
            let len = n.length();
            if len < 1e-10 {
                continue;
            }
            let n = n / len;
            let a = tri[0].xz();
            let b = tri[1].xz();
            let c = tri[2].xz();
            let lo = a.min(b).min(c);
            let hi = a.max(b).max(c);
            let x0 = ((lo.x - min.x) / cell).ceil().max(0.0) as usize;
            let z0 = ((lo.y - min.y) / cell).ceil().max(0.0) as usize;
            let x1 = (((hi.x - min.x) / cell).floor() as isize).min(nx as isize - 1);
            let z1 = (((hi.y - min.y) / cell).floor() as isize).min(nz as isize - 1);
            if x1 < x0 as isize || z1 < z0 as isize {
                continue;
            }
            let det = (b.y - c.y) * (a.x - c.x) + (c.x - b.x) * (a.y - c.y);
            if det.abs() < 1e-12 {
                continue;
            }
            for z in z0..=z1 as usize {
                for x in x0..=x1 as usize {
                    let p = min + Vec2::new(x as f32, z as f32) * cell;
                    let l0 = ((b.y - c.y) * (p.x - c.x) + (c.x - b.x) * (p.y - c.y)) / det;
                    let l1 = ((c.y - a.y) * (p.x - c.x) + (a.x - c.x) * (p.y - c.y)) / det;
                    let l2 = 1.0 - l0 - l1;
                    if l0 < -1e-4 || l1 < -1e-4 || l2 < -1e-4 {
                        continue;
                    }
                    let y = l0 * tri[0].y + l1 * tri[1].y + l2 * tri[2].y;
                    let i = z * nx + x;
                    if top[i].is_nan() || y > top[i] {
                        top[i] = y;
                        // O winding dos GLB nem sempre é coerente: a face
                        // mais alta está virada para cima por definição.
                        up[i] = n.y.abs();
                    }
                }
            }
        }
        // Peso: topo virado para cima e com inclinação que segura neve.
        let mut weight: Vec<f32> = top
            .iter()
            .zip(&up)
            .map(|(t, ny)| {
                if t.is_nan() || *ny < TOP_MIN_NY {
                    0.0
                } else {
                    roof_retention(*ny)
                }
            })
            .collect();
        if weight.iter().all(|w| *w <= 0.0) {
            return None;
        }
        let mut grid = Self {
            origin: min,
            cell,
            nx,
            nz,
            top,
            weight: vec![0.0; nx * nz],
        };
        // Borda arredondada: a espessura sobe da borda para dentro em ~3
        // células (distância chamfer às células sem tampa ou a um degrau).
        let dist = grid.edge_distance(&weight);
        // Ilha pequena (poste, viga) guarda menos que um telhado.
        let islands = grid.island_areas(&weight);
        for i in 0..weight.len() {
            if weight[i] <= 0.0 {
                continue;
            }
            let edge = 0.4 + 0.6 * smoothstep(0.0, 3.0, dist[i]);
            let island = (islands[i].sqrt() / 1.2).clamp(0.15, 1.0);
            weight[i] *= edge * island;
        }
        grid.weight = weight;
        Some(grid)
    }

    fn index(&self, x: usize, z: usize) -> usize {
        z * self.nx + x
    }

    fn xz(&self, x: usize, z: usize) -> Vec2 {
        self.origin + Vec2::new(x as f32, z as f32) * self.cell
    }

    /// Duas células vizinhas fazem parte da mesma superfície (sem degrau).
    fn joined(&self, a: usize, b: usize) -> bool {
        let (ha, hb) = (self.top[a], self.top[b]);
        !ha.is_nan() && !hb.is_nan() && (ha - hb).abs() <= (self.cell * 2.2).max(0.12)
    }

    fn edge_distance(&self, weight: &[f32]) -> Vec<f32> {
        let (nx, nz) = (self.nx, self.nz);
        let mut d = vec![f32::INFINITY; nx * nz];
        for z in 0..nz {
            for x in 0..nx {
                let i = self.index(x, z);
                if weight[i] <= 0.0 {
                    d[i] = 0.0;
                    continue;
                }
                let border = x == 0 || z == 0 || x + 1 == nx || z + 1 == nz;
                let step = !border
                    && [
                        self.index(x - 1, z),
                        self.index(x + 1, z),
                        self.index(x, z - 1),
                        self.index(x, z + 1),
                    ]
                    .iter()
                    .any(|j| weight[*j] <= 0.0 || !self.joined(i, *j));
                if border || step {
                    d[i] = 0.5;
                }
            }
        }
        // Duas passagens chamfer (3-4).
        for z in 0..nz {
            for x in 0..nx {
                let i = self.index(x, z);
                if x > 0 {
                    d[i] = d[i].min(d[self.index(x - 1, z)] + 1.0);
                }
                if z > 0 {
                    d[i] = d[i].min(d[self.index(x, z - 1)] + 1.0);
                }
            }
        }
        for z in (0..nz).rev() {
            for x in (0..nx).rev() {
                let i = self.index(x, z);
                if x + 1 < nx {
                    d[i] = d[i].min(d[self.index(x + 1, z)] + 1.0);
                }
                if z + 1 < nz {
                    d[i] = d[i].min(d[self.index(x, z + 1)] + 1.0);
                }
            }
        }
        d
    }

    fn island_areas(&self, weight: &[f32]) -> Vec<f32> {
        let n = weight.len();
        let mut label = vec![usize::MAX; n];
        let mut areas = Vec::new();
        for start in 0..n {
            if weight[start] <= 0.0 || label[start] != usize::MAX {
                continue;
            }
            let id = areas.len();
            let mut count = 0usize;
            let mut stack = vec![start];
            label[start] = id;
            while let Some(i) = stack.pop() {
                count += 1;
                let (x, z) = (i % self.nx, i / self.nx);
                let mut visit = |j: usize| {
                    if weight[j] > 0.0 && label[j] == usize::MAX && self.joined(i, j) {
                        label[j] = id;
                        stack.push(j);
                    }
                };
                if x > 0 {
                    visit(i - 1);
                }
                if x + 1 < self.nx {
                    visit(i + 1);
                }
                if z > 0 {
                    visit(i - self.nx);
                }
                if z + 1 < self.nz {
                    visit(i + self.nx);
                }
            }
            areas.push(count as f32 * self.cell * self.cell);
        }
        label
            .iter()
            .map(|l| if *l == usize::MAX { 0.0 } else { areas[*l] })
            .collect()
    }

    /// Centros das células com topo virado para cima e a altura dele.
    pub fn top_cells(&self) -> impl Iterator<Item = (Vec2, f32)> + '_ {
        (0..self.nz).flat_map(move |z| {
            (0..self.nx).filter_map(move |x| {
                let i = self.index(x, z);
                (self.weight[i] > 0.0).then(|| (self.xz(x, z), self.top[i]))
            })
        })
    }

    /// Altura da superfície da neve na célula com `depth` metros de pack.
    fn surface(&self, i: usize, depth: f32) -> f32 {
        let x = i % self.nx;
        let z = i / self.nx;
        let p = self.xz(x, z);
        // Ondulação contínua (~1,5 m): neve fofa não é uma placa.
        let lump = 0.88 + 0.12 * ((p.x * 2.3 + 1.7).sin() * (p.y * 1.9 + 0.4).sin())
            + 0.06 * ((p.x * 5.1 - p.y * 3.7).sin());
        self.top[i] + 0.004 + depth * self.weight[i] * lump
    }

    fn has_quad(&self, x: usize, z: usize) -> bool {
        let ids = [
            self.index(x, z),
            self.index(x + 1, z),
            self.index(x, z + 1),
            self.index(x + 1, z + 1),
        ];
        ids.iter().all(|i| self.weight[*i] > 0.0)
            && self.joined(ids[0], ids[1])
            && self.joined(ids[0], ids[2])
            && self.joined(ids[1], ids[3])
            && self.joined(ids[2], ids[3])
    }

    /// A malha da tampa com `depth` metros, no espaço local `to_local`.
    #[must_use]
    pub fn mesh(&self, depth: f32, to_local: &Affine3A) -> Mesh {
        let (nx, nz) = (self.nx, self.nz);
        let height: Vec<f32> = (0..nx * nz)
            .map(|i| if self.weight[i] > 0.0 { self.surface(i, depth) } else { f32::NAN })
            .collect();

        let mut positions: Vec<[f32; 3]> = Vec::new();
        let mut normals: Vec<[f32; 3]> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let up_local = to_local.transform_vector3(Vec3::Y).normalize_or(Vec3::Y);
        let mut vertex_of = vec![u32::MAX; nx * nz];
        let world_normal = |x: usize, z: usize| -> Vec3 {
            let h = |xx: isize, zz: isize| -> Option<f32> {
                if xx < 0 || zz < 0 || xx as usize >= nx || zz as usize >= nz {
                    return None;
                }
                let v = height[self.index(xx as usize, zz as usize)];
                (!v.is_nan()).then_some(v)
            };
            let (xi, zi) = (x as isize, z as isize);
            let c = h(xi, zi).unwrap_or(0.0);
            let dx = match (h(xi + 1, zi), h(xi - 1, zi)) {
                (Some(a), Some(b)) => (a - b) / (2.0 * self.cell),
                (Some(a), None) => (a - c) / self.cell,
                (None, Some(b)) => (c - b) / self.cell,
                _ => 0.0,
            };
            let dz = match (h(xi, zi + 1), h(xi, zi - 1)) {
                (Some(a), Some(b)) => (a - b) / (2.0 * self.cell),
                (Some(a), None) => (a - c) / self.cell,
                (None, Some(b)) => (c - b) / self.cell,
                _ => 0.0,
            };
            let clamp = |g: f32| g.clamp(-3.0, 3.0);
            Vec3::new(-clamp(dx), 1.0, -clamp(dz)).normalize()
        };
        let push_vertex = |positions: &mut Vec<[f32; 3]>, normals: &mut Vec<[f32; 3]>, p: Vec3, n: Vec3| -> u32 {
            positions.push(to_local.transform_point3(p).to_array());
            normals.push(to_local.transform_vector3(n).normalize_or(up_local).to_array());
            (positions.len() - 1) as u32
        };
        let mut top_vertex = |positions: &mut Vec<[f32; 3]>, normals: &mut Vec<[f32; 3]>, x: usize, z: usize| -> u32 {
            let i = self.index(x, z);
            if vertex_of[i] != u32::MAX {
                return vertex_of[i];
            }
            let xz = self.xz(x, z);
            let v = push_vertex(positions, normals, Vec3::new(xz.x, height[i], xz.y), world_normal(x, z));
            vertex_of[i] = v;
            v
        };
        // Arestas de quads usadas: a saia vai nas que só um quad usa.
        let mut edges: HashMap<(usize, usize), (u8, Vec2)> = HashMap::new();
        for z in 0..nz.saturating_sub(1) {
            for x in 0..nx.saturating_sub(1) {
                if !self.has_quad(x, z) {
                    continue;
                }
                let a = top_vertex(&mut positions, &mut normals, x, z);
                let b = top_vertex(&mut positions, &mut normals, x + 1, z);
                let c = top_vertex(&mut positions, &mut normals, x, z + 1);
                let d = top_vertex(&mut positions, &mut normals, x + 1, z + 1);
                indices.extend_from_slice(&[a, c, b, b, c, d]);
                let centre = self.xz(x, z) + Vec2::splat(self.cell * 0.5);
                for (p, q) in [
                    (self.index(x, z), self.index(x + 1, z)),
                    (self.index(x + 1, z), self.index(x + 1, z + 1)),
                    (self.index(x + 1, z + 1), self.index(x, z + 1)),
                    (self.index(x, z + 1), self.index(x, z)),
                ] {
                    let key = (p.min(q), p.max(q));
                    let entry = edges.entry(key).or_insert((0, centre));
                    entry.0 += 1;
                }
            }
        }
        // Saia: da superfície até ao topo do prop, virada para fora do quad.
        let mut skirt: Vec<((usize, usize), Vec2)> = edges
            .into_iter()
            .filter(|(_, (uses, _))| *uses == 1)
            .map(|(key, (_, centre))| (key, centre))
            .collect();
        skirt.sort_unstable_by_key(|(key, _)| *key);
        for ((p, q), centre) in skirt {
            let (xp, zp) = (p % nx, p / nx);
            let (xq, zq) = (q % nx, q / nx);
            let a = self.xz(xp, zp);
            let b = self.xz(xq, zq);
            let mid = (a + b) * 0.5;
            let out = (mid - centre).normalize_or(Vec2::X);
            let n = Vec3::new(out.x, 0.35, out.y).normalize();
            let ta = push_vertex(&mut positions, &mut normals, Vec3::new(a.x, height[p], a.y), n);
            let tb = push_vertex(&mut positions, &mut normals, Vec3::new(b.x, height[q], b.y), n);
            let ba = push_vertex(&mut positions, &mut normals, Vec3::new(a.x, self.top[p] - 0.01, a.y), n);
            let bb = push_vertex(&mut positions, &mut normals, Vec3::new(b.x, self.top[q] - 0.01, b.y), n);
            indices.extend_from_slice(&[ta, ba, tb, tb, ba, bb]);
        }
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
        mesh.insert_indices(Indices::U32(indices));
        mesh
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Caixa 4×4 m de 2 m de altura (topo e quatro paredes).
    fn crate_tris() -> Vec<[Vec3; 3]> {
        let p = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
        vec![
            [p(0.0, 2.0, 0.0), p(0.0, 2.0, 4.0), p(4.0, 2.0, 0.0)],
            [p(4.0, 2.0, 0.0), p(0.0, 2.0, 4.0), p(4.0, 2.0, 4.0)],
            [p(0.0, 0.0, 0.0), p(4.0, 0.0, 0.0), p(0.0, 2.0, 0.0)],
            [p(4.0, 0.0, 0.0), p(4.0, 2.0, 0.0), p(0.0, 2.0, 0.0)],
        ]
    }

    #[test]
    fn a_flat_top_gets_a_cap_thicker_inside_than_at_the_edge() {
        let grid = CapGrid::rasterize(&crate_tris()).expect("topo");
        let centre = grid.index(grid.nx / 2, grid.nz / 2);
        let edge = grid.index(0, grid.nz / 2);
        assert!(grid.weight[centre] > 0.8, "centro {}", grid.weight[centre]);
        assert!(grid.weight[edge] < grid.weight[centre] * 0.6);
        assert!((grid.top[centre] - 2.0).abs() < 1e-4);
        let mesh = grid.mesh(0.3, &Affine3A::IDENTITY);
        let Some(VertexAttributeValues::Float32x3(pos)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
            panic!("posições");
        };
        let highest = pos.iter().map(|p| p[1]).fold(f32::MIN, f32::max);
        assert!(highest > 2.2 && highest < 2.45, "topo da neve {highest}");
        let lowest = pos.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
        assert!(lowest < 2.0, "a saia desce ao topo da caixa: {lowest}");
    }

    #[test]
    fn steep_roofs_shed_and_walls_are_not_tops() {
        assert_eq!(roof_retention(1.0), 1.0);
        assert!(roof_retention(30f32.to_radians().cos()) > 0.99);
        assert_eq!(roof_retention(70f32.to_radians().cos()), 0.0);
        // Só uma parede: nada de tampa.
        let wall = vec![[
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(4.0, 0.0, 0.0),
            Vec3::new(0.0, 2.0, 0.0),
        ]];
        assert!(CapGrid::rasterize(&wall).is_none());
    }

    #[test]
    fn a_floor_under_a_roof_stays_bare() {
        let mut tris = crate_tris();
        // Chão a 0,1 m dentro da caixa: o topo (2 m) tapa-o.
        tris.push([
            Vec3::new(0.5, 0.1, 0.5),
            Vec3::new(0.5, 0.1, 3.5),
            Vec3::new(3.5, 0.1, 0.5),
        ]);
        let grid = CapGrid::rasterize(&tris).expect("topo");
        assert!(grid.top_cells().all(|(_, y)| (y - 2.0).abs() < 1e-4));
    }
}

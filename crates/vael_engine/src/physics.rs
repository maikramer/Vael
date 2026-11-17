//! Física do jogo — Avian.
//!
//! A forma/colisor/ponte de raycast vem toda do `bevy_ext_physics` (o crate
//! extraído); este módulo fica só com o que ainda está acoplado ao TERMO do
//! monólito: o streaming de colliders por coluna voxel, que lê o
//! `TerrainRuntime` e os `TerrainChunk`/`VoxelChunk` daqui.
//!
//! Quando o terreno do monólito migrar para o `bevy_ext_terrain_runtime`, este
//! ficheiro desaparece: o `stream_voxel_colliders` já existe lá contra os tipos
//! de lá e o `PhysicsPlugin` do crate passa a ser o único registo.
//!
//! ## Tipos partilhados (um só de cada)
//!
//! * `VoxelCollider` — `bevy_ext_core` (o `bevy_ext_nav` filtra obstáculos pelo
//!   MESMO tipo que a física insere), re-exportado pelo `bevy_ext_physics`.
//! * `TerrainCollisionStatus` — `bevy_ext_physics` (`viber::physics`).
//! * `ColumnColliderBake`/`PHYSICS_CHUNK_RADIUS`/`column_xz_distance`/
//!   `nearest_column_distance` — `bevy_ext_terrain_runtime::host` (o contrato
//!   do host para a física do terreno).
//!
//! O plugin do Avian (colliders pendentes, gizmos, diagnostics) é o
//! [`bevy_ext_physics::physics::PhysicsPlugin`]; o streaming do terreno entra
//! pelo [`TerrainCollidersPlugin`] ao lado dele.

use avian3d::prelude::{Collider, RigidBody};
use bevy::prelude::*;

pub use bevy_ext_physics::physics::*;
pub use bevy_ext_terrain_runtime::host::{
    ColumnColliderBake, PHYSICS_CHUNK_RADIUS, column_xz_distance, nearest_column_distance,
};

/// Banda de colisão em metros para um spec.
///
/// `chunk_size × PHYSICS_CHUNK_RADIUS` limitado à banda LOD 0
/// (`lod_distance`): dentro dela o ladder impõe piso de LOD 0 às colunas,
/// por isso "collider == o que se vê" é exato em todo o lado onde o player
/// consegue tocar — nunca há collider assado de geometria mais grosseira do
/// que a desenhada.
pub fn collision_keep_within(spec: &crate::terrain::spec::TerrainSpec) -> f32 {
    (spec.chunk_size * PHYSICS_CHUNK_RADIUS).min(spec.lod_distance())
}

/// Adds and removes the terrain collider on voxel COLUMNS around the player.
///
/// No caminho 100% volumétrico TODA a superfície é coluna voxel com trimesh —
/// este streaming é a única fonte de colisão de terreno em runtime. O bake em
/// si acontece no swap do LOD (`terrain::plugin`) e no spawn da coluna
/// (`terrain::voxel::spawn_column`); aqui vive a manutenção: tirar o collider
/// quando a coluna sai da banda e reparar colunas construídas sem collider
/// (player chegou depois — teleporte/fast-travel), lendo de volta os meshes
/// já residentes.
#[allow(clippy::type_complexity)]
#[allow(clippy::too_many_arguments)]
pub fn stream_voxel_colliders(
    mut commands: Commands,
    mut status: ResMut<TerrainCollisionStatus>,
    mut hero_column_log: Local<Option<(bool, [u32; 2])>>,
    runtime: Option<Res<crate::terrain::runtime::TerrainRuntime>>,
    players: Query<(Entity, &GlobalTransform), With<crate::player::Player>>,
    cameras: Query<
        (
            &GlobalTransform,
            Option<&Camera>,
            Option<&bevy::camera::RenderTarget>,
        ),
        (With<Camera3d>, Without<crate::player::Player>),
    >,
    columns: Query<(
        Entity,
        &crate::terrain::plugin::TerrainChunk,
        Option<&VoxelCollider>,
        Option<&Children>,
    )>,
    boxes: Query<(&crate::terrain::voxel::VoxelChunk, &Mesh3d)>,
    meshes: Res<Assets<Mesh>>,
    mut repairs: Local<u32>,
) {
    let _probe = bevy_engine_core::perf_probe::Probe::new("physics.stream_colliders");
    status.ready = false;
    status.ready_players.clear();
    let Some(runtime) = runtime else { return };
    // TODOS os heróis: cada um precisa da coluna debaixo dos pés. Sem
    // âncora de jogador (menu/editor) a câmara decide onde assar.
    let player_anchors: Vec<(Entity, Vec3)> = players
        .iter()
        .map(|(entity, global)| (entity, global.translation()))
        .collect();
    let mut anchors: Vec<Vec3> = player_anchors.iter().map(|(_, pos)| *pos).collect();
    if anchors.is_empty() {
        anchors.extend(
            cameras
                .iter()
                .find(|(_, cam, target)| crate::editor_camera::is_window_camera(*cam, *target))
                .map(|(t, _, _)| t.translation()),
        );
    }
    if anchors.is_empty() {
        return;
    }
    if runtime.spec.collision_resolution == 0 {
        return;
    }
    let keep_within = collision_keep_within(&runtime.spec);
    let drop_beyond = keep_within * 1.25;
    let half = runtime.spec.world_size * 0.5;
    let edge = crate::terrain::plugin::chunk_edge(&runtime.spec);
    *repairs = 0;

    let mut best_repair: Option<(f32, Entity, &Children)> = None;

    // DIAGNÓSTICO da queda do herói no arranque: transições do collider na
    // coluna que contém o 1.º herói (a que decide `ready_players`).
    if let Some((_, pos)) = player_anchors.first() {
        let mut next: Option<(bool, [u32; 2])> = None;
        for (_, column, has_collider, _) in &columns {
            if column_xz_distance(*pos, column.coords, half, edge) <= 0.0 {
                next = Some((
                    has_collider.is_some(),
                    [column.coords.x, column.coords.y],
                ));
                break;
            }
        }
        if *hero_column_log != next
            && let Some((has, coords)) = next
        {
            debug!(
                "terrain colliders: coluna do herói {:?} {} (y do herói {:.1})",
                coords,
                if has { "COM collider" } else { "SEM collider" },
                pos.y
            );
        }
        *hero_column_log = next;
    }

    for (entity, column, has_collider, children) in &columns {
        let distance = nearest_column_distance(&anchors, column.coords, half, edge);
        match (has_collider.is_some(), distance) {
            (true, d) if d <= keep_within => {
                // `ready` é "há chão SOB os pés", não "há chão algures na
                // banda": a distância mínima é 0 só para a coluna que
                // CONTÉM algum herói. Com o teste antigo (`d <= keep_within`,
                // ~192 m) qualquer coluna vizinha ainda assada punha
                // `ready = true` depois de um teleporte, o chão analítico
                // do `player_movement` desligava-se e o herói atravessava o
                // terreno por assar — a saída de um interior devolvia-o à
                // vila e ele caía até y≈2 (repro 2026-09-12).
                if d <= 0.0 {
                    status.ready = true;
                    // Marca QUEM está dentro desta coluna assada (co-op): o
                    // `player_movement` consulta o próprio herói, não o
                    // agregado.
                    for (player, pos) in &player_anchors {
                        if column_xz_distance(*pos, column.coords, half, edge) <= 0.0 {
                            status.ready_players.insert(*player);
                        }
                    }
                }
            }
            // try_* e não insert/remove: as colunas MORREM sob os pés deste
            // sistema (o swap do LOD e o cull despawnam no mesmo frame) —
            // aplicar num despawnado seria panic de engine.
            (true, d) if d > drop_beyond => {
                commands
                    .entity(entity)
                    .try_remove::<Collider>()
                    .try_remove::<RigidBody>()
                    .try_remove::<VoxelCollider>();
            }
            // Coluna em banda sem collider: escolhe a MAIS PRÓXIMA do herói
            // para assar (1 coluna por frame no máximo).
            (false, d) if d <= keep_within => {
                if let Some(children) = children
                    && best_repair
                        .as_ref()
                        .is_none_or(|(best_d, _, _)| d < *best_d)
                {
                    best_repair = Some((d, entity, children));
                }
            }
            _ => {}
        }
    }

    if *repairs < 1
        && let Some((_, entity, children)) = best_repair
    {
        let mut bake = ColumnColliderBake::new();
        for kid in children.iter() {
            if let Ok((box_chunk, mesh3d)) = boxes.get(kid)
                && let Some(mesh) = meshes.get(&mesh3d.0)
            {
                bake.add_bevy_mesh(box_chunk.origin, mesh);
            }
        }
        // Sem `ready` aqui: a coluna reparada pode não ser a que está
        // sob o herói, e o insert é diferido (o Avian só a vê no
        // frame seguinte) — o ramo `d <= 0` acima publica-o então.
        if let Some(collider) = bake.bake() {
            commands
                .entity(entity)
                .try_insert((collider, RigidBody::Static, VoxelCollider));
            *repairs += 1;
        }
    }
}

/// Acrescenta o streaming de colliders do terreno (o pedaço host-side) ao
/// [`bevy_ext_physics::physics::PhysicsPlugin`].
///
/// Regista-se SEMPRE a seguir ao plugin da física: as colunas que este sistema
/// assa precisam do `PhysicsPlugins` do Avian já montado, e sem ele o
/// `resolve_pending_colliders`/gizmos ficam a faltar.
pub struct TerrainCollidersPlugin;

impl Plugin for TerrainCollidersPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PostUpdate,
            // Depois do LOD: o swap de coluna despacha caixas e spawna as
            // novas no mesmo frame, e sem esta ordem o streaming via a grelha
            // do frame anterior — um ou mais frames sem chão debaixo do
            // herói a cada troca de LOD.
            stream_voxel_colliders.after(crate::terrain::plugin::TerrainSet::Columns),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `collision_keep_within` lê o spec do twin (`bevy_ext_terrain_runtime`):
    /// banda = min(chunk × raio, `lod_distance()` — o ratio × chunk).
    #[test]
    fn test_collision_keep_within_is_the_lod0_band() {
        let mut spec = crate::terrain::spec::TerrainSpec::default();
        spec.chunk_size = 64.0;
        spec.lod_distance_ratio = 1000.0 / 64.0;
        assert_eq!(collision_keep_within(&spec), 64.0 * PHYSICS_CHUNK_RADIUS);
        spec.lod_distance_ratio = 50.0 / 64.0;
        assert_eq!(collision_keep_within(&spec), 50.0);
    }
}

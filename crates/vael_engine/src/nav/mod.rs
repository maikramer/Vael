//! Navigation: a real navmesh, real pathfinding and real local avoidance.
//!
//! SHIM (exit3/wave-c): a navegação vive no crate [`bevy_ext_nav`]; este
//! módulo reexporta a árvore — os caminhos `vael_engine::nav::*` (BRP, `viber.ecs`,
//! bridge) continuam a resolver e os tipos são OS MESMOS no jogo. O que aqui
//! ficou é o ÚNICO ponto de ligação que o crate não pode ter: o contrato
//! [`bevy_ext_nav::NavGround`] sobre o [`TerrainRuntime`] do runtime de
//! terreno, publicado no [`bevy_ext_nav::NavGroundHandle`].
//!
//! Antes deste módulo a IA andava em linha recta até onde queria e escrevia
//! o resultado em `Transform`. A pilha é `bevy_rerecast` (Recast portado) +
//! `landmass_rerecast` + `bevy_landmass` (A*, RVO, custo por tipo de
//! polígono — é tudo o que "preferir estradas" é).
//!
//! Wiring: `app.add_plugins(crate::nav::NavPlugin);` — o plugin regista o par
//! tag+arquipélago no `Startup` (a seguir ao set
//! [`bevy_ext_nav::WorldSpawned`], onde o spawn do mundo se junta) e no
//! `SceneLoad` do core. O runtime do Viber corre o SEU schedule de troca de
//! cena (`crate::scene_loader::SceneLoad`, um tipo só no `bevy_ext_engine`),
//! por isso o `app.rs` volta a registar o par nesse schedule — o par não é
//! idempotente, corre em UM schedule que corre mesmo.

pub use bevy_ext_nav::*;

use bevy::prelude::*;
use std::sync::Arc;

use crate::terrain::cliffs::CliffMask;
use crate::terrain::runtime::TerrainRuntime;
use crate::terrain::water::WaterKind;

/// Snapshot do terreno do runtime sobre o contrato [`NavGround`] (do crate).
///
/// O [`TerrainRuntime`] é imutável depois do bootstrap e os seus campos
/// pesados vivem atrás de `Arc`s — o snapshot clona os `Arc`s e os vetores
/// pequenos (água/estradas/praças, dezenas de entradas), e converte-os para
/// os tipos do contrato. Numa troca de cena para interior o runtime do
/// overworld permanece (interiores não têm terreno), como antes.
struct TerrainGround {
    rt: TerrainRuntime,
    cliff: Option<CliffMask>,
    water: Vec<NavWater>,
    roads: Vec<NavRoad>,
    pads: Vec<NavPad>,
}

impl TerrainGround {
    /// Copia o runtime (Arcs partilhados) e converte água/estradas/praças.
    fn snapshot(rt: &TerrainRuntime, cliff: Option<&CliffMask>) -> Self {
        Self {
            water: rt
                .water
                .iter()
                .map(|w| NavWater {
                    at: w.at,
                    carve_radius: w.carve_radius,
                    kind: match w.kind {
                        WaterKind::Lake => NavWaterKind::Lake,
                        WaterKind::River => NavWaterKind::River,
                    },
                    stations: w.stations.clone(),
                })
                .collect(),
            roads: rt
                .roads
                .iter()
                .map(|r| NavRoad {
                    stations: r.stations.clone(),
                    half_width: r.half_width.clone(),
                    deck_y: r.deck_y,
                })
                .collect(),
            pads: rt
                .pads
                .iter()
                .map(|p| NavPad {
                    at: p.at,
                    size: p.size,
                    height: p.height,
                })
                .collect(),
            cliff: cliff.cloned(),
            rt: TerrainRuntime {
                spec: rt.spec.clone(),
                grid: rt.grid.clone(),
                water: rt.water.clone(),
                roads: rt.roads.clone(),
                paving: rt.paving.clone(),
                pads: rt.pads.clone(),
                voxel: rt.voxel.clone(),
                deltas: rt.deltas.clone(),
                paint: rt.paint.clone(),
                splat_tuning: rt.splat_tuning,
            },
        }
    }
}

impl NavGround for TerrainGround {
    fn in_field(&self, x: f32, z: f32) -> bool {
        self.rt.in_field(x, z)
    }

    fn sample_surface(&self, x: f32, z: f32) -> f32 {
        self.rt.sample_mesh_surface(x, z)
    }

    fn surface_top(&self, x: f32, z: f32) -> f32 {
        self.rt.voxel.surface_top(&self.rt.base(), x, z)
    }

    fn max_height(&self) -> f32 {
        self.rt.spec.max_height
    }

    fn water(&self) -> &[NavWater] {
        &self.water
    }

    fn roads(&self) -> &[NavRoad] {
        &self.roads
    }

    fn pads(&self) -> &[NavPad] {
        &self.pads
    }

    fn is_cliff(&self, p: Vec2) -> bool {
        self.cliff.as_ref().is_some_and(|mask| mask.is_cliff_at(p))
    }
}

/// Publica o [`NavGroundHandle`] sobre o terreno corrente.
///
/// Corre a seguir ao spawn do mundo, no `Startup` e na troca de cena: sem o
/// handle a navegação fica adormecida (o mesmo degradar elegante de
/// `VIBER_NAV=0`). Idempotente — o snapshot clona `Arc`s e vetores pequenos.
pub fn publish_nav_ground(
    runtime: Option<Res<TerrainRuntime>>,
    cliffs: Option<Res<CliffMask>>,
    mut commands: Commands,
) {
    let Some(runtime) = runtime else {
        return;
    };
    let ground = TerrainGround::snapshot(&runtime, cliffs.as_deref());
    commands.insert_resource(NavGroundHandle(Arc::new(ground)));
}

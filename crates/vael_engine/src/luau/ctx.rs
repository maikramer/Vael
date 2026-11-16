//! Registos de superfície partilhados com os scripts (estradas/água) — o
//! que sobrou do ctx da VM antiga (2026-11-15, onda final do
//! DELETE-MONOLITH: `ScriptCtx`, o snapshot por-chamada que as closures
//! `viber.*` liam, morreu com o host; o BMS publica o estado do frame no
//! `bevy_ext_luau::ScriptHostState`).
//!
//! As estradas/água são partilhadas por ARCO com a ponte BMS
//! ([`crate::luau::bms_bridge`], que as publica via o seam
//! `bevy_ext_luau::SurfaceQueries`) e com a REPL do bridge (`viber.debug`
//! consulta ao vivo) — construídas UMA vez (o mundo carvado não muda depois
//! do bootstrap) e depois custam um clone de ponteiro por frame.

use bevy::prelude::*;

/// Estradas e água, partilhadas com os scripts.
#[derive(Debug, Default)]
pub struct SurfaceRegistries {
    pub roads: Vec<crate::terrain::roads::RoadPath>,
    pub water: Vec<crate::terrain::water::WaterBody>,
}

impl SurfaceRegistries {
    /// Ponto na fita de uma estrada (`isPointOnRoad`).
    pub fn on_road(&self, x: f32, z: f32) -> bool {
        let p = Vec2::new(x, z);
        self.roads.iter().any(|road| road.is_on_road(p))
    }

    /// Estrada mais próxima do ponto, mesmo fora da fita (`viber.road`).
    /// `None` quando o mundo não tem estradas.
    pub fn nearest_road(&self, x: f32, z: f32) -> Option<crate::terrain::roads::RoadSample> {
        let p = Vec2::new(x, z);
        self.roads
            .iter()
            .filter_map(|road| road.sample_at(p))
            .min_by(|a, b| a.center_dist.total_cmp(&b.center_dist))
    }

    /// Ponto na estrada mais longa a `s` metros de arco (`viber.road_at`).
    /// O comprimento é INVARIÁVEL pós-bootstrap: computá-lo dentro do
    /// `max_by` percorria as estações de cada estrada em CADA comparação.
    pub fn road_at(&self, s: f32) -> Option<crate::terrain::roads::RoadSample> {
        let (index, _) = self
            .roads
            .iter()
            .enumerate()
            .map(|(index, road)| (index, crate::terrain::paths::path_length(&road.stations)))
            .max_by(|a, b| a.1.total_cmp(&b.1))?;
        self.roads[index].point_at_arc(s)
    }

    /// Ponto dentro de uma zona de carve de água.
    pub fn in_water(&self, x: f32, z: f32) -> bool {
        let p = Vec2::new(x, z);
        self.water.iter().any(|body| body.contains(p))
    }
}

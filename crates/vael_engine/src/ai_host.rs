//! Adaptador de HOST do `bevy_ext_ai` — o contrato de chão
//! ([`bevy_ext_ai::Ground`]) implementado SOBRE o runtime de terreno da
//! engine.
//!
//! O `ai.rs`/`senses.rs` do monólito saíram para o crate (exit4/G1) e o
//! integrador de locomoção (`apply_ai_locomotion`) amosta o Y e clampeia à
//! borda pela fonte de chão que a App liga — o mesmo padrão do
//! `audio_host` para o `bevy_ext_audio`. Sem este plugin as criaturas
//! movem-se sem snap ao terreno (o comportamento headless dos testes do
//! crate).
//!
//! O análogo no Vael: `app.add_plugins(vael_engine::ai_host::AiHostPlugin);`
//! junto do registo dos plugins `vael_engine::ai::*`.

use bevy::prelude::*;

use bevy_ext_ai::Ground;

use crate::terrain::runtime::{TerrainReader, TerrainRuntime};

/// Fonte de chão do [`bevy_ext_ai::GroundQuery`] sobre o runtime de terreno:
/// os mesmos três métodos que o `apply_ai_locomotion` histórico chamava
/// directamente (`spec.world_size`, `in_field`, `surface_below`). O
/// [`TerrainReader`] partilha os `Arc` do runtime — clone grátis, e as
/// edições vivas (o overlay de deltas) chegam ao integrador sem re-publicar.
struct TerrainGround {
    reader: TerrainReader,
    world_size: f32,
}

impl bevy_ext_ai::GroundQuery for TerrainGround {
    fn world_radius(&self) -> Option<f32> {
        Some(self.world_size * 0.5)
    }

    fn in_field(&self, x: f32, z: f32) -> bool {
        let half = self.world_size * 0.5;
        x >= -half && x <= half && z >= -half && z <= half
    }

    fn surface_below(&self, x: f32, z: f32, from_y: f32) -> Option<f32> {
        self.reader
            .voxel
            .surface_below(&self.reader.base(), x, z, from_y)
    }
}

/// Publica o [`Ground`] do crate a partir do [`TerrainRuntime`] da engine.
///
/// O bootstrap insere o runtime depois do `Startup` (as cenas carregam no
/// `SceneLoad`), por isso o `Update` volta a publicar enquanto o recurso
/// mudar — o `TerrainReader` é clone grátis, publicar de novo não copia
/// alturas.
pub struct AiHostPlugin;

impl Plugin for AiHostPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Ground>()
            .add_systems(Update, publish_ground);
    }
}

fn publish_ground(
    runtime: Option<Res<TerrainRuntime>>,
    mut ground: ResMut<Ground>,
    mut seeded: Local<bool>,
) {
    let Some(runtime) = runtime else {
        return;
    };
    // O bootstrap insere o runtime depois do arranque (as cenas carregam no
    // `SceneLoad`); re-publicar só quando ele MUDA — o `TerrainReader` é
    // clone grátis, mas o `Ground::new` aloca um `Arc` por publicação.
    if *seeded && !runtime.is_changed() {
        return;
    }
    *seeded = true;
    let source = TerrainGround {
        reader: runtime.reader(),
        world_size: runtime.spec.world_size,
    };
    *ground = Ground::new(source);
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ext_ai::GroundQuery;
    use std::sync::Arc;

    /// O contrato de chão do adaptador: raio do disco, in_field e o snap de
    /// Y sobre um runtime procedural — os mesmos números que o
    /// `apply_ai_locomotion` histórico via do `TerrainRuntime` directo.
    #[test]
    fn terrain_ground_matches_runtime_contract() {
        use crate::terrain::brush::BrushGrid;
        use crate::terrain::delta::DeltaGrid;
        use crate::terrain::heightmap::HeightMapU16;
        use crate::terrain::paint::PaintGrid;
        use crate::terrain::spec::TerrainSpec;
        use crate::terrain::voxel::VoxelField;

        let spec = TerrainSpec {
            world_size: 128.0,
            max_height: 40.0,
            seed: 5,
            ..TerrainSpec::default()
        };
        let map = HeightMapU16::procedural(&spec, spec.resolution.max(1) as usize);
        let grid = BrushGrid::from_height_map(
            &map,
            spec.world_size,
            spec.max_height,
            spec.height_smoothing,
        )
        .expect("grid builds");
        let runtime = TerrainRuntime {
            spec,
            grid: Arc::new(grid),
            water: vec![],
            roads: vec![],
            pads: vec![],
            voxel: Arc::new(VoxelField::default()),
            deltas: Arc::new(DeltaGrid::default()),
            paint: Arc::new(PaintGrid::default()),
            splat_tuning: Default::default(),
            paving: Default::default(),
        };
        let ground = TerrainGround {
            reader: runtime.reader(),
            world_size: runtime.spec.world_size,
        };
        assert_eq!(ground.world_radius(), Some(64.0));
        assert!(ground.in_field(0.0, 0.0));
        assert!(!ground.in_field(1000.0, 0.0));
        // Piso na coluna: o centro do mundo tem chão (o heightfield começa
        // nele) e `surface_below` do voxel com campo flat devolve o base.
        assert!(ground.surface_below(0.0, 0.0, 50.0).is_some());
    }
}

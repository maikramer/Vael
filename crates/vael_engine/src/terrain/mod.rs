//! Terrain declarativo do Viber — heightfield como input de um campo 3D.
//!
//! # SHIM (fase D de `docs/DELETE-MONOLITH.md`)
//!
//! O runtime vive no [`bevy_ext_terrain_runtime`] — o glob em baixo re-exporta
//! o crate inteiro, por isso TODOS os caminhos `crate::terrain::…`/`viber::
//! terrain::…` históricos (módulos e itens) continuam a resolver: os 98 usos
//! de `terrain::runtime`, o `TerrainPlugin`/`TerrainFeaturesPlugin` que o
//! `app.rs` registra, os tipos reflectidos que o Vael e os saves citam.
//! Os `TypePath` `viber::terrain::…` estão congelados NO crate (guarda de
//! wire em `delta.rs`); o `physics.rs` do monólito alinha os seus marcadores
//! aos do `host.rs` do crate (a coluna voxel spawna com o marker do crate).
//!
//! # Contratos (do crate)
//!
//! * **O heightfield é input, não a forma.** O [`brush::BrushGrid`] é o termo
//!   base do SDF (`densidade(p) = p.y − base.sample(p.xz)`); o
//!   [`voxel::VoxelField`] combina-o em CSG com os mods 3D e o transvoxel
//!   mesha o zero do campo. A grelha não é renderizada nem colide
//!   directamente — é o alvo dos carves e a fonte dos registries.
//! * **Colunas, não chunks 2.5D.** Cada coluna de `chunk-size` metros é
//!   meshada por caixas voxel e colide por um trimesh Avian com os mesmos
//!   triângulos do render (`voxel::spawn`), assado no swap atómico do LOD.
//! * **Duas vias de look:** o tint legado de altura/inclinação em vertex
//!   colors, ou o splat blend de [`splat`] + [`layer_material`] (13 slots de
//!   camada, material bindless único), que substitui o tint.
//! * **Edições vivas** ficam no overlay [`delta::DeltaGrid`]; nenhum
//!   consumidor lê a grelha crua em runtime.
//!
//! # XML
//!
//! ```xml
//! <Terrain heightmap="terrain/height.png" world-size="256" max-height="50"
//!          chunk-size="64" levels="3" collision-resolution="64"
//!          base-color="#ffffff" color-rock="#6b6560" />
//! <!-- ou o blend de camadas (substitui o tint; areia nas margens): -->
//! <Terrain layers="grass vale_grass dirt forest_floor gravel
//!          mountain_stone sand snow_peak" shore-width="6" />
//! <TerrainPad at="20 -10" size="24 16" falloff="8" corner-radius="4" />
//! ```

pub use bevy_ext_terrain_runtime::*;

pub mod water_bridge;

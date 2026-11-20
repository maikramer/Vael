//! Draw the larger snow aggregates from the weather simulation's existing GPU buffer.
use bevy::asset::{RenderAssetUsages, embedded_asset};
use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::{render_resource::AsBindGroup, storage::ShaderBuffer};
use bevy::shader::ShaderRef;
use bevy_elements::{PrecipitationState, SunState};
use vael_engine::worldsys::InteriorLighting;

pub(super) struct SnowfallPlugin;

impl Plugin for SnowfallPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "snowfall.wesl");
        app.add_plugins(MaterialPlugin::<Snowflakes>::default())
            .add_systems(PostUpdate, sync_snowflakes);
    }
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
struct Snowflakes {
    #[storage(0, read_only)]
    particles: Handle<ShaderBuffer>,
    #[uniform(1)]
    daylight: f32,
}

impl Material for Snowflakes {
    fn vertex_shader() -> ShaderRef {
        "embedded://vael/environment/snowfall.wesl".into()
    }

    fn fragment_shader() -> ShaderRef {
        Self::vertex_shader()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
}

// One quad per sample, evenly spaced through the buffer at every weather quality.
fn snow_mesh() -> Mesh {
    const SAMPLES: u32 = 12288;
    let mut positions = Vec::with_capacity(4 * SAMPLES as usize);
    let mut indices = Vec::with_capacity(6 * SAMPLES as usize);
    for i in 0..SAMPLES {
        let sample = (i as f32 + 0.5) / SAMPLES as f32;
        for [x, y] in [[-1., -1.], [1., -1.], [1., 1.], [-1., 1.]] {
            positions.push([x, y, sample]);
        }
        let k = 4 * i;
        indices.extend([k, k + 1, k + 2, k, k + 2, k + 3]);
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_indices(Indices::U32(indices))
}

fn sync_snowflakes(
    mut commands: Commands,
    state: Option<Res<PrecipitationState>>,
    interior: Option<Res<InteriorLighting>>,
    sun: Res<SunState>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<Snowflakes>>,
    mut field: Query<(&MeshMaterial3d<Snowflakes>, &mut Visibility)>,
) {
    let snow = state
        .as_deref()
        .filter(|s| s.snow > 0 && !interior.as_deref().is_some_and(|i| i.active));
    let daylight = ((sun.altitude_deg + 8.0) / 22.0).clamp(0.0, 1.0);
    if let Ok((handle, mut visibility)) = field.single_mut() {
        visibility.set_if_neq(if snow.is_some() {
            Visibility::Visible
        } else {
            Visibility::Hidden
        });
        if let Some(snow) = snow
            && materials.get(handle).is_some_and(|m| {
                m.particles != snow.particles || (m.daylight - daylight).abs() > 0.01
            })
            && let Some(mut material) = materials.get_mut(handle)
        {
            material.particles = snow.particles.clone();
            material.daylight = daylight;
        }
    } else if let Some(snow) = snow {
        commands.spawn((
            Name::new("Flocos de neve"),
            Mesh3d(meshes.add(snow_mesh())),
            MeshMaterial3d(materials.add(Snowflakes {
                particles: snow.particles.clone(),
                daylight,
            })),
            NoFrustumCulling,
            NotShadowCaster,
            crate::snow_caps::IgnoreSnowMesh,
        ));
    }
}

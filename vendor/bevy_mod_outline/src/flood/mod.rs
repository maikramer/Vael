use bevy::anti_alias::contrast_adaptive_sharpening::cas;
use bevy::anti_alias::fxaa::fxaa;
use bevy::anti_alias::smaa::smaa;
use bevy::asset::embedded_asset;
use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::pbr::{MeshInputUniform, MeshUniform};
use bevy::render::batching::gpu_preprocessing::{BatchedInstanceBuffers, GpuPreprocessingSupport};
use bevy::render::render_phase::{
    sort_phase_system, AddRenderCommand, DrawFunctions, SortedRenderPhasePlugin,
};
use bevy::render::RenderDebugFlags;
use bevy::{
    prelude::*,
    render::{
        camera::ExtractedCamera,
        render_resource::{
            Extent3d, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
        },
        renderer::RenderDevice,
        texture::{CachedTexture, TextureCache},
        Render, RenderApp, RenderStartup, RenderSystems,
    },
};
use compose_output::{
    init_compose_output_pipeline, prepare_compose_output_pass, prepare_compose_output_uniform,
    ComposeOutputUniforms,
};
use flood_init::{prepare_flood_phases, queue_flood_meshes};
use jump_flood::init_jump_flood_pipeline;
use node::{flood_render_pass, FloodOutline};
use sobel_init::init_sobel_init_pipeline;
use wgpu_types::{LoadOp, Operations, StoreOp};

use crate::add_dummy_phase_buffer;
use crate::msaa::ResolvedOutlineMsaa;
use crate::node::outline_render_pass;
use crate::pipeline::OutlinePipeline;
use crate::render::DrawOutline;
use crate::uniforms::DrawMode;
use crate::view_uniforms::OutlineViewUniform;

mod compose_output;
mod flood_init;
mod jump_flood;
mod node;
mod sobel_init;

const FLOOD_OPS: Operations<wgpu_types::Color> = Operations {
    load: LoadOp::Clear(wgpu_types::Color {
        r: -10000.0,
        g: -10000.0,
        b: 0.0,
        a: 0.0,
    }),
    store: StoreOp::Store,
};

#[derive(Clone)]
pub(crate) struct FloodCoverageTextures {
    pub msaa_tex: CachedTexture,
    pub resolved: CachedTexture,
}

#[derive(Clone, Component)]
pub(crate) struct FloodTextures {
    pub flip: bool,
    pub texture_a: CachedTexture,
    pub texture_b: CachedTexture,
    pub coverage: Option<FloodCoverageTextures>,
}

impl FloodTextures {
    pub fn input(&self) -> &CachedTexture {
        if self.flip {
            &self.texture_b
        } else {
            &self.texture_a
        }
    }

    pub fn output(&self) -> &CachedTexture {
        if self.flip {
            &self.texture_a
        } else {
            &self.texture_b
        }
    }

    pub fn flip(&mut self) {
        self.flip = !self.flip;
    }
}

pub fn prepare_flood_textures(
    mut commands: Commands,
    mut texture_cache: ResMut<TextureCache>,
    render_device: Res<RenderDevice>,
    cameras: Query<(Entity, &ExtractedCamera, &ResolvedOutlineMsaa), With<OutlineViewUniform>>,
    old_cameras: Query<Entity, (With<FloodTextures>, Without<OutlineViewUniform>)>,
) {
    for (entity, camera, msaa) in cameras.iter() {
        let Some(target_size) = camera.physical_target_size else {
            commands.entity(entity).remove::<FloodTextures>();
            continue;
        };

        let size = Extent3d {
            width: target_size.x,
            height: target_size.y,
            depth_or_array_layers: 1,
        };

        let texture_descriptor = TextureDescriptor {
            label: None,
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rg16Float,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        };

        let coverage = if msaa.samples() > 1 {
            let coverage_descriptor = TextureDescriptor {
                format: TextureFormat::R8Unorm,
                ..texture_descriptor.clone()
            };
            let msaa_tex = texture_cache.get(
                &render_device,
                TextureDescriptor {
                    label: Some("outline_flood_coverage_msaa"),
                    sample_count: msaa.samples(),
                    usage: TextureUsages::RENDER_ATTACHMENT,
                    ..coverage_descriptor.clone()
                },
            );
            let resolved = texture_cache.get(
                &render_device,
                TextureDescriptor {
                    label: Some("outline_flood_coverage_resolved"),
                    ..coverage_descriptor
                },
            );
            Some(FloodCoverageTextures { msaa_tex, resolved })
        } else {
            None
        };

        commands.entity(entity).insert(FloodTextures {
            flip: false,
            texture_a: texture_cache.get(&render_device, texture_descriptor.clone()),
            texture_b: texture_cache.get(&render_device, texture_descriptor),
            coverage,
        });
    }

    for entity in old_cameras.iter() {
        commands.entity(entity).remove::<FloodTextures>();
    }
}

fn add_dummy_phase_buffers(
    mut bibs: ResMut<BatchedInstanceBuffers<MeshUniform, MeshInputUniform>>,
) {
    add_dummy_phase_buffer::<FloodOutline>(&mut bibs);
}

#[derive(Debug)]
pub struct FloodPlugin;

impl Plugin for FloodPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "jump_flood.wesl");
        embedded_asset!(app, "compose_output.wesl");
        embedded_asset!(app, "sobel_init.wesl");

        app.add_plugins(SortedRenderPhasePlugin::<FloodOutline, OutlinePipeline>::new(
            RenderDebugFlags::empty(),
        ))
        .sub_app_mut(RenderApp)
        .init_resource::<ComposeOutputUniforms>()
        .init_resource::<DrawFunctions<FloodOutline>>()
        .add_render_command::<FloodOutline, DrawOutline>()
        .add_systems(
            Render,
            prepare_flood_phases
                .after(RenderSystems::ExtractCommands)
                .before(RenderSystems::QueueMeshes),
        )
        .add_systems(
            Render,
            (prepare_flood_textures, prepare_compose_output_uniform, prepare_compose_output_pass).in_set(RenderSystems::Prepare),
        )
        .add_systems(Render, queue_flood_meshes.in_set(RenderSystems::QueueMeshes))
        .add_systems(
            Render,
            sort_phase_system::<FloodOutline>.in_set(RenderSystems::PhaseSort),
        )
        .add_systems(
            Core3d,
            flood_render_pass
                .after(outline_render_pass)
                .before(fxaa)
                .before(smaa)
                .before(cas)
                .in_set(Core3dSystems::PostProcess),
        );
    }

    fn finish(&self, app: &mut App) {
        let render_app = app.sub_app_mut(RenderApp);
        render_app.add_systems(
            RenderStartup,
            (
                init_sobel_init_pipeline,
                init_jump_flood_pipeline,
                init_compose_output_pipeline,
            ),
        );

        let gpu_preprocessing_support = render_app.world().resource::<GpuPreprocessingSupport>();
        if gpu_preprocessing_support.is_available() {
            render_app.add_systems(
                Render,
                add_dummy_phase_buffers.in_set(RenderSystems::PrepareResourcesCollectPhaseBuffers),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use bevy::app::SubApp;
    use bevy::core_pipeline::tonemapping::tonemapping;
    use bevy::ecs::schedule::graph::Direction;
    use bevy::ecs::schedule::{InternedSystemSet, NodeId};

    use super::*;
    use crate::OutlinePlugin;

    fn has_dependency_path(
        schedule: &Schedule,
        from: InternedSystemSet,
        to: InternedSystemSet,
    ) -> bool {
        let graph = schedule.graph();
        let dep = graph.dependency().graph();
        let hier = graph.hierarchy().graph();
        let find = |set: InternedSystemSet| -> NodeId {
            let key = *graph
                .systems_in_set(set)
                .expect("system set must exist in Core3d")
                .iter()
                .next()
                .expect("system set must contain a system");
            NodeId::System(key)
        };
        let start = find(from);
        let goal = find(to);

        let mut visited = bevy::platform::collections::HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(start);
        visited.insert(start);
        while let Some(node) = queue.pop_front() {
            if node == goal {
                return true;
            }
            for next in dep.neighbors_directed(node, Direction::Outgoing) {
                if visited.insert(next) {
                    queue.push_back(next);
                }
            }
            for parent in hier.neighbors_directed(node, Direction::Incoming) {
                for next in dep.neighbors_directed(parent, Direction::Outgoing) {
                    if visited.insert(next) {
                        queue.push_back(next);
                    }
                }
            }
            if matches!(node, NodeId::Set(_)) {
                for child in hier.neighbors_directed(node, Direction::Outgoing) {
                    if visited.insert(child) {
                        queue.push_back(child);
                    }
                }
            }
        }
        false
    }

    #[test]
    fn flood_render_pass_ordered_before_post_aa_and_cas() {
        fn split_tone_stub() {}

        let mut app = App::new();
        let mut render_app = SubApp::new();
        render_app.add_schedule(Core3d::base_schedule());
        render_app.add_systems(
            Core3d,
            (
                tonemapping.in_set(Core3dSystems::PostProcess),
                fxaa.after(tonemapping).in_set(Core3dSystems::PostProcess),
                smaa.after(tonemapping).in_set(Core3dSystems::PostProcess),
                cas.after(fxaa).after(smaa).in_set(Core3dSystems::PostProcess),
                split_tone_stub
                    .after(tonemapping)
                    .after(cas)
                    .in_set(Core3dSystems::PostProcess),
            ),
        );
        app.insert_sub_app(RenderApp, render_app);
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Shader>()
            .add_plugins(OutlinePlugin::JUMP_FLOOD);

        let render_world = app.sub_app_mut(RenderApp).world_mut();
        render_world
            .try_schedule_scope(Core3d, |world, core3d| {
                core3d.initialize(world).expect("Core3d must initialize");

                let s_tonemap = tonemapping.into_system_set().intern();
                let s_outline = outline_render_pass.into_system_set().intern();
                let s_flood = flood_render_pass.into_system_set().intern();
                let s_fxaa = fxaa.into_system_set().intern();
                let s_smaa = smaa.into_system_set().intern();
                let s_cas = cas.into_system_set().intern();
                let s_split_tone = split_tone_stub.into_system_set().intern();

                assert!(
                    has_dependency_path(core3d, s_tonemap, s_outline),
                    "outline_render_pass must run after tonemapping"
                );
                assert!(
                    has_dependency_path(core3d, s_outline, s_flood),
                    "flood_render_pass must run after outline_render_pass"
                );
                assert!(
                    has_dependency_path(core3d, s_flood, s_fxaa),
                    "flood_render_pass must run strictly before fxaa"
                );
                assert!(
                    has_dependency_path(core3d, s_flood, s_smaa),
                    "flood_render_pass must run strictly before smaa"
                );
                assert!(
                    has_dependency_path(core3d, s_flood, s_cas),
                    "flood_render_pass must run strictly before cas (ViewTarget ping-pong)"
                );
                assert!(
                    has_dependency_path(core3d, s_flood, s_split_tone),
                    "flood_render_pass must run strictly before post-CAS passes (split_tone)"
                );
            })
            .expect("Core3d schedule must exist");
    }
}

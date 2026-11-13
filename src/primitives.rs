//! Procedural render for the world's authored primitives.
//!
//! Until the Vael owns the world spawner, the transitional engine parses the
//! DSL and spawns a StandardMaterial fallback plus a
//! [`PrimitivePart`](bevy_ext_gradmesh::PrimitivePart) marker. This plugin
//! consumes that marker and swaps in the `bevy_ext_gradmesh` mesh, gradient
//! material and exact collider — the same crate the Jackdaw preview uses, so
//! the game and the editor agree on the look.
//!
//! When the Vael spawner replaces the engine one, it can insert the same
//! marker directly (or call [`PrimitiveRenderCache`]); the plugin is already
//! the runtime path.
//!
//! Gates:
//! * `VAEL_PRIMITIVES=standard|off` keeps the engine's standard-material
//!   fallback (A/B and escape hatch).
//! * `VAEL_PRIMITIVES_DETAIL=minimal|low|medium|high|extreme` (default
//!   `high`).

use std::collections::HashMap;

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy_rapier3d::prelude::Collider;

use bevy_ext_gradmesh::{
    GradMeshMaterial, GradMeshPlugin, GradShape, PrimitivePart, ShapeDetail, VERTEX_GRADIENT_DARK,
    bake_vertical_gradient, collider as exact_collider, gradmesh_shape, material_fingerprint,
    style,
};

pub struct VaelPrimitivesPlugin;

impl Plugin for VaelPrimitivesPlugin {
    fn build(&self, app: &mut App) {
        if !primitives_enabled() {
            return;
        }
        if !app.is_plugin_added::<GradMeshPlugin>() {
            app.add_plugins(GradMeshPlugin);
        }
        app.init_resource::<PrimitiveDetail>()
            .init_resource::<PrimitiveRenderCache>()
            .add_systems(PostUpdate, apply_primitive_parts);
    }
}

/// `VAEL_PRIMITIVES=standard|std|off|0|false|no` keeps the engine fallback.
fn primitives_enabled() -> bool {
    !matches!(
        std::env::var("VAEL_PRIMITIVES").ok().as_deref(),
        Some("standard" | "std" | "off" | "0" | "false" | "no")
    )
}

#[derive(Resource, Clone, Copy)]
struct PrimitiveDetail(ShapeDetail);

impl Default for PrimitiveDetail {
    fn default() -> Self {
        Self(
            match std::env::var("VAEL_PRIMITIVES_DETAIL").ok().as_deref() {
                Some("minimal") => ShapeDetail::Minimal,
                Some("low") => ShapeDetail::Low,
                Some("medium") => ShapeDetail::Medium,
                Some("extreme") => ShapeDetail::Extreme,
                _ => ShapeDetail::High,
            },
        )
    }
}

/// Mesh/material dedup across the world (and across scene reloads, since the
/// resources survive the entity despawn).
#[derive(Resource, Default)]
struct PrimitiveRenderCache {
    meshes: HashMap<GradShape, Handle<Mesh>>,
    materials: HashMap<u64, Handle<GradMeshMaterial>>,
}

fn apply_primitive_parts(
    mut commands: Commands,
    mut cache: ResMut<PrimitiveRenderCache>,
    detail: Res<PrimitiveDetail>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GradMeshMaterial>>,
    parts: Query<(Entity, &PrimitivePart, Option<&Collider>), Added<PrimitivePart>>,
) {
    for (entity, part, existing_collider) in &parts {
        // `Plane` (and any future shape without procedural geometry) keeps
        // the engine's standard mesh.
        let Some(grad_shape) = gradmesh_shape(&part.shape, detail.0) else {
            commands.entity(entity).remove::<PrimitivePart>();
            continue;
        };
        let mesh = match cache.meshes.get(&grad_shape) {
            Some(handle) => handle.clone(),
            None => {
                let mut mesh = grad_shape.build();
                bake_vertical_gradient(&mut mesh, VERTEX_GRADIENT_DARK);
                let handle = meshes.add(mesh);
                cache.meshes.insert(grad_shape, handle.clone());
                handle
            }
        };
        let key = material_fingerprint(&part.material);
        let material = match cache.materials.get(&key) {
            Some(handle) => handle.clone(),
            None => {
                let handle = materials.add(style(&part.material).build());
                cache.materials.insert(key, handle.clone());
                handle
            }
        };
        commands
            .entity(entity)
            .remove::<MeshMaterial3d<StandardMaterial>>()
            .insert((Mesh3d(mesh), MeshMaterial3d(material)))
            .remove::<PrimitivePart>();
        // The gradient material still draws translucent entities in the
        // shadow pass (only alpha-test cuts), so opt out explicitly.
        if part.material.opacity.is_some_and(|alpha| alpha < 1.0) {
            commands.entity(entity).insert(NotShadowCaster);
        }
        // Only replace colliders the spawner created (authored/composition
        // `auto`); a primitive with `collider="none"` stays without one.
        if existing_collider.is_some()
            && let Some(exact) = exact_collider(&part.shape, Vec3::ONE)
        {
            commands.entity(entity).remove::<Collider>().insert(exact);
        }
    }
}

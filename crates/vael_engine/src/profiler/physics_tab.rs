//! Snapshot "Física" do profiler — o equivalente do `physics-debug.ts`
//! original, agora sobre o Avian: corpos por tipo e sono, colisores por forma,
//! sensores, controllers do tnua, pendentes de mesh, gravidade e a duração do
//! `PhysicsSystems::StepSimulation` (medida pelas âncoras em
//! [`crate::profiler::timed::physics_anchors`]).
//!
//! As contagens ECS estão sempre disponíveis; `available` distingue os dois
//! casos, como no original.

use std::collections::BTreeMap;

use serde::Serialize;

use avian3d::prelude::{Collider, Gravity, RigidBody, Sensor, Sleeping, SphericalJoint};
use bevy::prelude::*;

use crate::physics::PendingCollider;

use super::timed;

#[derive(Debug, Clone, Default, Serialize)]
pub struct Bodies {
    pub total: usize,
    pub fixed: usize,
    pub dynamic: usize,
    pub kinematic: usize,
    pub sleeping: usize,
    pub awake: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Colliders {
    pub total: usize,
    pub sensors: usize,
    pub by_shape: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EngineWorld {
    pub bodies: usize,
    pub colliders: usize,
    pub impulse_joints: usize,
    /// Passo actual do solver (s) — `Time<Physics>`.
    pub timestep: f32,
}

/// Snapshot do tab Física.
#[derive(Debug, Clone, Serialize)]
pub struct PhysicsSnapshot {
    /// `false` quando não há física no mundo (`Time<Physics>` em falta).
    pub available: bool,
    pub bodies: Bodies,
    pub colliders: Colliders,
    /// Controllers do herói (tnua). O mundo normal tem 1 (o `PlayerGLTF`).
    pub cct: usize,
    pub pending_colliders: usize,
    pub gravity: [f32; 3],
    /// Contagens do motor. A chave JSON continua `rapier` para não partir os
    /// consumidores do painel/bridge.
    #[serde(rename = "rapier")]
    pub engine: Option<EngineWorld>,
    /// ms do último `PhysicsSystems::StepSimulation` + média da janela.
    pub step: Option<StepTiming>,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct StepTiming {
    pub last_ms: f32,
    pub avg_ms: f32,
    pub p95_ms: f32,
}

/// Nome curto da forma do collider (o mesmo rótulo do `shape_type` do parry).
fn shape_name(collider: &Collider) -> String {
    use avian3d::parry::shape::TypedShape;
    match collider.shape_scaled().as_typed_shape() {
        TypedShape::Ball(_) => "Ball".into(),
        TypedShape::Cuboid(_) => "Cuboid".into(),
        TypedShape::Capsule(_) => "Capsule".into(),
        TypedShape::Cylinder(_) => "Cylinder".into(),
        TypedShape::TriMesh(_) => "TriMesh".into(),
        TypedShape::Compound(_) => "Compound".into(),
        TypedShape::ConvexPolyhedron(_) => "ConvexPolyhedron".into(),
        TypedShape::HeightField(_) => "HeightField".into(),
        other => format!("{other:?}")
            .split(['(', ' ', '{'])
            .next()
            .unwrap_or("Shape")
            .to_string(),
    }
}

/// Recolhe contagens de física de um mundo real.
pub fn snapshot(world: &mut World, frame_avg_ms: f32) -> PhysicsSnapshot {
    let mut bodies = Bodies::default();
    let mut q_bodies = world.query::<(&RigidBody, Has<Sleeping>)>();
    for (body, sleeping) in q_bodies.iter(world) {
        bodies.total += 1;
        match body {
            RigidBody::Static => bodies.fixed += 1,
            RigidBody::Dynamic => bodies.dynamic += 1,
            RigidBody::Kinematic => bodies.kinematic += 1,
        }
        // No Avian `Sleeping` é um marcador: presente = a dormir.
        if sleeping {
            bodies.sleeping += 1;
        } else {
            bodies.awake += 1;
        }
    }

    let mut colliders = Colliders::default();
    let mut q_colliders = world.query::<&Collider>();
    for collider in q_colliders.iter(world) {
        colliders.total += 1;
        *colliders.by_shape.entry(shape_name(collider)).or_default() += 1;
    }
    let mut q_sensors = world.query_filtered::<(), With<Sensor>>();
    colliders.sensors = q_sensors.iter(world).count();

    let mut q_cct =
        world.query_filtered::<(), With<bevy_tnua::prelude::TnuaController<crate::player::HeroScheme>>>();
    let cct = q_cct.iter(world).count();

    let mut q_pending = world.query_filtered::<(), With<PendingCollider>>();
    let pending_colliders = q_pending.iter(world).count();

    let mut gravity = [0.0f32; 3];
    if let Some(gravity_res) = world.get_resource::<Gravity>() {
        gravity = [gravity_res.0.x, gravity_res.0.y, gravity_res.0.z];
    }

    // `available`: o plugin de física criou o relógio dedicado.
    let available = world.get_resource::<Time<avian3d::prelude::Physics>>().is_some();
    let engine = available.then(|| {
        let mut q_joints = world.query_filtered::<
            (),
            Or<(With<SphericalJoint>, With<avian3d::prelude::FixedJoint>)>,
        >();
        EngineWorld {
            bodies: bodies.total,
            colliders: colliders.total,
            impulse_joints: q_joints.iter(world).count(),
            timestep: world
                .get_resource::<Time<avian3d::prelude::Physics>>()
                .map(|time| time.delta_secs())
                .unwrap_or(0.0),
        }
    });

    let step = systems_snapshot_step(frame_avg_ms);

    PhysicsSnapshot {
        available,
        bodies,
        colliders,
        cct,
        pending_colliders,
        gravity,
        engine,
        step,
    }
}

/// Extrai o timing `physics.step` (âncoras) do anel de sistemas.
fn systems_snapshot_step(frame_avg_ms: f32) -> Option<StepTiming> {
    timed::systems_snapshot(frame_avg_ms)
        .into_iter()
        .find(|s| s.name == "physics.step")
        .map(|s| StepTiming {
            last_ms: s.last_ms,
            avg_ms: s.avg_ms,
            p95_ms: s.p95_ms,
        })
}

/// Linhas de texto da janela para o tab Física.
pub fn window_lines(snap: &PhysicsSnapshot) -> Vec<String> {
    let mut lines = Vec::new();
    if !snap.available {
        lines.push("física: (sem física no mundo)".into());
        return lines;
    }
    let b = &snap.bodies;
    lines.push(format!(
        "corpos {} (fixos {} · din {} · cin {})",
        b.total, b.fixed, b.dynamic, b.kinematic
    ));
    lines.push(format!(
        "  sono {} dormindo · {} acordados",
        b.sleeping, b.awake
    ));
    let shapes: Vec<String> = snap
        .colliders
        .by_shape
        .iter()
        .map(|(k, v)| format!("{k} {v}"))
        .collect();
    lines.push(format!(
        "colisores {}  sensores={}",
        snap.colliders.total, snap.colliders.sensors
    ));
    lines.push(format!("  {}", shapes.join("  ")));
    lines.push(format!(
        "cct {}  pendentes {}  gravidade {:.1} {:.1} {:.1}",
        snap.cct, snap.pending_colliders, snap.gravity[0], snap.gravity[1], snap.gravity[2]
    ));
    if let Some(r) = &snap.engine {
        lines.push(format!(
            "avian    corpos={}  colisores={}  juntas={}  dt={:.4}",
            r.bodies, r.colliders, r.impulse_joints, r.timestep
        ));
    }
    match snap.step {
        Some(s) => lines.push(format!(
            "step     {:.2} ms  média {:.2}  p95 {:.2}",
            s.last_ms, s.avg_ms, s.p95_ms
        )),
        None => lines.push("step     (sem amostras)".into()),
    }
    lines
}

/// Payload JSON do tab.
pub fn json(snap: &PhysicsSnapshot) -> serde_json::Value {
    serde_json::to_value(snap).unwrap_or(serde_json::Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use avian3d::prelude::{Collider, RigidBody, Sleeping};

    #[test]
    fn test_snapshot_counts_components() {
        let mut world = World::new();
        world.init_resource::<Time>();
        // Sem `Time<Physics>` → available=false mas contagens ECS presentes.
        world.spawn((
            Transform::IDENTITY,
            RigidBody::Static,
            Collider::cuboid(2.0, 2.0, 2.0),
        ));
        world.spawn((
            Transform::IDENTITY,
            RigidBody::Dynamic,
            Collider::sphere(0.5),
            Sleeping,
        ));
        world.spawn((Transform::IDENTITY, Collider::sphere(0.25), Sensor));

        let snap = snapshot(&mut world, 16.0);
        assert!(!snap.available);
        assert_eq!(snap.bodies.total, 2);
        assert_eq!(snap.bodies.fixed, 1);
        assert_eq!(snap.bodies.dynamic, 1);
        assert_eq!(snap.bodies.sleeping, 1);
        assert_eq!(snap.bodies.awake, 1);
        assert_eq!(snap.colliders.total, 3);
        assert_eq!(snap.colliders.sensors, 1);
        assert_eq!(snap.colliders.by_shape.get("Cuboid"), Some(&1), "{snap:?}");
        assert_eq!(snap.colliders.by_shape.get("Ball"), Some(&2));

        let lines = window_lines(&snap);
        assert!(lines[0].contains("sem física"), "{lines:?}");
        let json = json(&snap);
        assert_eq!(json["bodies"]["total"], 2, "{json}");
        assert_eq!(json["available"], false);
    }
}

//! Diagnóstico de performance (só com `VAEL_PERF_DIAG=1`): a cada 2 s conta
//! quem mudou `Transform`/`Visibility` nesse frame, agrupado pelo nome da
//! entidade e do pai. Não altera nada no mundo.

use bevy::prelude::*;
use std::collections::HashMap;

pub struct VaelPerfDiagPlugin;

impl Plugin for VaelPerfDiagPlugin {
    fn build(&self, app: &mut App) {
        if std::env::var("VAEL_PERF_DIAG").ok().as_deref() != Some("1") {
            return;
        }
        app.add_systems(Last, perf_diag);
    }
}

#[allow(clippy::type_complexity)]
fn perf_diag(
    time: Res<Time>,
    mut next: Local<f32>,
    changed_t: Query<(Entity, Option<&Name>, Option<&ChildOf>), Changed<Transform>>,
    changed_v: Query<(Entity, Option<&Name>, Option<&ChildOf>), Changed<Visibility>>,
    changed_gt: Query<(), Changed<GlobalTransform>>,
    all_t: Query<(), With<Transform>>,
    meshes: Query<(), With<Mesh3d>>,
    vis_meshes: Query<&ViewVisibility, With<Mesh3d>>,
    names: Query<&Name>,
    mesh_q: Query<(Option<&ChildOf>, &ViewVisibility, &InheritedVisibility), With<Mesh3d>>,
    parents: Query<&ChildOf>,
    mut dumped: Local<u32>,
    changed_aabb: Query<(Option<&Name>, Option<&ChildOf>), Changed<bevy::camera::primitives::Aabb>>,
    mut mesh_events: MessageReader<AssetEvent<Mesh>>,
) {
    let mut modified = 0usize;
    for ev in mesh_events.read() {
        if matches!(ev, AssetEvent::Modified { .. }) {
            modified += 1;
        }
    }
    *next -= time.delta_secs();
    if *next > 0.0 {
        return;
    }
    *next = 2.0;
    let label = |name: Option<&Name>, parent: Option<&ChildOf>| {
        let own = name
            .map(|n| n.as_str().to_string())
            .unwrap_or_else(|| "?".into());
        let par = parent
            .and_then(|p| names.get(p.parent()).ok())
            .map(|n| n.as_str().to_string())
            .unwrap_or_else(|| "-".into());
        // Números (ids/índices) colapsam para agrupar cópias.
        let squash = |s: String| {
            s.chars()
                .map(|c| if c.is_ascii_digit() { '#' } else { c })
                .collect::<String>()
        };
        format!("{} <- {}", squash(own), squash(par))
    };
    let mut groups: HashMap<String, usize> = HashMap::new();
    let mut nt = 0;
    for (_, n, p) in &changed_t {
        nt += 1;
        *groups.entry(label(n, p)).or_default() += 1;
    }
    let mut top: Vec<_> = groups.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1));
    let mut vgroups: HashMap<String, usize> = HashMap::new();
    let mut nv = 0;
    for (_, n, p) in &changed_v {
        nv += 1;
        *vgroups.entry(label(n, p)).or_default() += 1;
    }
    let mut vtop: Vec<_> = vgroups.into_iter().collect();
    vtop.sort_by(|a, b| b.1.cmp(&a.1));
    let visible = vis_meshes.iter().filter(|v| v.get()).count();
    info!(
        "PERFDIAG transforms={} changed_t={nt} changed_gt={} changed_vis={nv} meshes={} visible_meshes={visible}",
        all_t.iter().count(),
        changed_gt.iter().count(),
        meshes.iter().count(),
    );
    for (k, c) in top.iter().take(25) {
        info!("PERFDIAG  T {c:6} {k}");
    }
    for (k, c) in vtop.iter().take(8) {
        info!("PERFDIAG  V {c:6} {k}");
    }
    let mut ag: HashMap<String, usize> = HashMap::new();
    let mut na = 0;
    for (n, p) in &changed_aabb {
        na += 1;
        *ag.entry(label(n, p)).or_default() += 1;
    }
    let mut at: Vec<_> = ag.into_iter().collect();
    at.sort_by(|a, b| b.1.cmp(&a.1));
    info!("PERFDIAG changed_aabb={na} mesh_modified_events(2s)={modified}");
    for (k, c) in at.iter().take(10) {
        info!("PERFDIAG  A {c:6} {k}");
    }
    *dumped += 1;
    if *dumped == 6 {
        // Malhas por avô (nome da instância) — quem enche o mundo.
        let squash = |s: &str| {
            s.chars()
                .map(|c| if c.is_ascii_digit() { '#' } else { c })
                .collect::<String>()
        };
        let mut by: HashMap<String, [usize; 3]> = HashMap::new();
        for (parent, view, inherited) in &mesh_q {
            let mut at = parent.map(|p| p.parent());
            let mut chain = Vec::new();
            while let Some(e) = at {
                if let Ok(n) = names.get(e) {
                    chain.push(squash(n.as_str()));
                }
                at = parents.get(e).ok().map(|p| p.parent());
                if chain.len() >= 6 {
                    break;
                }
            }
            // O nome mais alto antes das pastas genéricas.
            let key = chain
                .iter()
                .rev()
                .skip(1)
                .take(2)
                .cloned()
                .collect::<Vec<_>>()
                .join(" / ");
            let slot = by.entry(key).or_default();
            slot[0] += 1;
            if inherited.get() {
                slot[1] += 1;
            }
            if view.get() {
                slot[2] += 1;
            }
        }
        let mut v: Vec<_> = by.into_iter().collect();
        v.sort_by(|a, b| b.1[0].cmp(&a.1[0]));
        for (k, c) in v.iter().take(40) {
            info!(
                "PERFDIAG  M total={:6} inherited_vis={:6} view_vis={:6} {k}",
                c[0], c[1], c[2]
            );
        }
    }
}

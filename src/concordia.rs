//! Porta de estado para os volumes e serviços autorados dos canteiros.

use avian3d::prelude::{ColliderDisabled, RigidBodyDisabled};
use bevy::{ecs::entity_disabling::Disabled, prelude::*};
use vael_engine::luau::{ScriptProps, game::game_get_json};

pub(crate) struct ConcordiaPlugin;

impl Plugin for ConcordiaPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreUpdate, stage_buildings);
    }
}

pub(crate) fn stage(id: &str) -> u64 {
    game_get_json(&format!("concordia.{id}"))
        .and_then(|value| value.as_u64())
        .unwrap_or(0)
}

#[derive(Component)]
struct Closed {
    visibility: Option<Visibility>,
    disabled: bool,
    body_disabled: bool,
    collider_disabled: bool,
}

fn stage_buildings(
    world: &mut World,
    mut gates: Local<Option<QueryState<(Entity, &'static ScriptProps), Allow<Disabled>>>>,
) {
    let gates = gates.get_or_insert_with(|| world.query_filtered());
    let mut stages = std::collections::HashMap::new();
    let roots: Vec<_> = gates
        .iter(world)
        .filter_map(|(entity, props)| {
            let (_, gate) = props.0.iter().find(|(key, _)| key == "concordia-stage")?;
            let (id, range) = gate.split_once(':')?;
            let (min, max) = range.split_once('-').unwrap_or((range, range));
            let actual = *stages.entry(id).or_insert_with(|| stage(id));
            Some((
                entity,
                (min.parse::<u64>().ok()?..=max.parse::<u64>().ok()?).contains(&actual),
            ))
        })
        .collect();
    for (root, open) in roots {
        if open && world.get::<Closed>(root).is_none() {
            continue;
        }
        let mut descendants = vec![root];
        let mut next = 0;
        while let Some(&entity) = descendants.get(next) {
            next += 1;
            if let Some(children) = world.get::<Children>(entity) {
                descendants.extend(children.iter());
            }
            let mut target = world.entity_mut(entity);
            if open {
                if let Some(closed) = target.take::<Closed>() {
                    if let Some(visibility) = closed.visibility {
                        target.insert(visibility);
                    }
                    if !closed.disabled {
                        target.remove::<Disabled>();
                    }
                    if !closed.body_disabled {
                        target.remove::<RigidBodyDisabled>();
                    }
                    if !closed.collider_disabled {
                        target.remove::<ColliderDisabled>();
                    }
                }
            } else if !target.contains::<Closed>() {
                let closed = Closed {
                    visibility: target.get::<Visibility>().copied(),
                    disabled: target.contains::<Disabled>(),
                    body_disabled: target.contains::<RigidBodyDisabled>(),
                    collider_disabled: target.contains::<ColliderDisabled>(),
                };
                if closed.visibility.is_some() {
                    target.insert(Visibility::Hidden);
                }
                // Não desactivar os loaders de GLTF. Ao abrir o canteiro as
                // malhas e os colliders já podem estar prontos.
                if target.contains::<vael_engine::luau::LuaScriptRef>()
                    || target.contains::<vael_engine::scene_loader::ScenePortal>()
                {
                    target.insert(Disabled);
                }
                target.insert((closed, RigidBodyDisabled, ColliderDisabled));
            }
        }
    }
}

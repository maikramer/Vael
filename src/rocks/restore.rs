//! Loading an older slot in the same scene must put mined deposits back in place.
use super::{Quarry, QuarryStone};
use crate::harvest::{Harvestable, reaped_key};
use crate::save::VaelSave;
use bevy::prelude::*;
use viber::{luau::ScriptProps, scene::SceneSource};

pub(super) struct Deposit {
    parent: Entity,
    name: Name,
    pose: Transform,
    props: ScriptProps,
    source: SceneSource,
    harvest: Harvestable,
}

pub(super) fn remember(
    event: On<Despawn<Harvestable>>,
    query: Query<(
        &ChildOf,
        &Name,
        &Transform,
        &ScriptProps,
        &SceneSource,
        &Harvestable,
    )>,
    mut quarry: ResMut<Quarry>,
) {
    let Ok((parent, name, pose, props, source, harvest)) = query.get(event.entity) else {
        return;
    };
    if !super::is_deposit(props) {
        return;
    }
    let mut harvest = harvest.clone();
    harvest.remaining = harvest.hits;
    quarry.removed.push(Deposit {
        parent: parent.parent(),
        name: name.clone(),
        pose: *pose,
        props: props.clone(),
        source: source.clone(),
        harvest,
    });
}

pub(crate) fn reload(world: &mut World) {
    if !world.contains_resource::<Quarry>() {
        return;
    }
    // Partial strikes belong to the old session. A fresh entity also clears
    // the Lua controller's per-entity hit count.
    let damaged: Vec<_> = world
        .query_filtered::<Entity, (With<QuarryStone>, With<crate::harvest::MiningFracture>)>()
        .iter(world)
        .collect();
    for entity in damaged {
        world.despawn(entity);
    }
    world.resource_scope(|world, mut quarry: Mut<Quarry>| {
        quarry.last_hour = None;
        quarry.next_sample = 0.;
        quarry.removed.retain(|station| {
            if world.get_entity(station.parent).is_err() {
                return false;
            }
            let h = &station.harvest;
            if world
                .get_resource::<VaelSave>()
                .is_some_and(|s| reaped_key(h, &s.harvested).is_some())
                || std::iter::once(&h.name)
                    .chain(h.stable_key.iter())
                    .any(|k| {
                        viber::luau::game::game_get_json(&format!("harvested.{k}"))
                            .is_some_and(|v| v == true)
                    })
            {
                return true;
            }
            world.spawn((
                ChildOf(station.parent),
                station.name.clone(),
                station.pose,
                Visibility::Inherited,
                station.props.clone(),
                station.source.clone(),
                h.clone(),
            ));
            false
        });
    });
}

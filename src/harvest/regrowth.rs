//! The authored station survives felling. Its calendar date survives a scene or save reload.
use super::*;
use avian3d::prelude::Collider;
use bevy_elements::WorldClock;
use vael_engine::luau::components::LuaScriptRef;

const SPROUT_AFTER_DAYS: f64 = 1.0;
const MATURE_AFTER_DAYS: f64 = 3.0;

#[derive(Component)]
pub(super) struct Rootstock {
    pub(super) tree: Entity,
    pub(super) key: Option<String>,
    cut_day: Option<f64>,
    pub(super) pooled: PooledTree,
    pub(super) harvest: Harvestable,
    props: Option<ScriptProps>,
    script: Option<LuaScriptRef>,
    collider: Option<Collider>,
    shoot: Option<Entity>,
}

pub(super) fn leave_root(
    commands: &mut Commands,
    station: Entity,
    tree: Entity,
    key: Option<String>,
) {
    commands.queue(move |world: &mut World| {
        let Ok(mut e) = world.get_entity_mut(station) else {
            return;
        };
        let Some(pooled) = e.take::<PooledTree>() else {
            return;
        };
        let Some(mut harvest) = e.take::<Harvestable>() else {
            return;
        };
        harvest.remaining = harvest.hits;
        let root = Rootstock {
            tree,
            key,
            cut_day: None,
            pooled,
            harvest,
            props: e.take::<ScriptProps>(),
            script: e.take::<LuaScriptRef>(),
            collider: e.take::<Collider>(),
            shoot: None,
        };
        e.remove::<ScriptInteraction>().insert((
            root,
            FloraFellTree,
            Visibility::Hidden,
            Collider::cylinder(STUMP_RADIUS, 2.0 * STUMP_HALF_HEIGHT),
        ));
    });
}

/// Replace the temporary collider when flora has cut the actual trunk. Its capsule
/// measures the foot and cut plane in the same local space as the authored station.
pub(super) fn fit_stumps(
    mut commands: Commands,
    stumps: Query<&TreeStump, Added<TreeStump>>,
    roots: Query<(Entity, &Rootstock)>,
) {
    for stump in &stumps {
        let Some(tree) = stump.tree else { continue };
        let Some((station, _)) = roots.iter().find(|(_, root)| root.tree == tree) else {
            continue;
        };
        let wood = stump.capsule;
        let axis = wood.b - wood.a;
        if let Some(up) = axis.try_normalize() {
            commands.entity(station).insert(Collider::compound(vec![(
                (wood.a + wood.b) * 0.5,
                Quat::from_rotation_arc(Vec3::Y, up),
                Collider::cylinder(wood.radius.max(0.01), axis.length()),
            )]));
        }
    }
}

pub(super) fn grow(
    mut commands: Commands,
    clock: Option<Res<WorldClock>>,
    mut save: Option<ResMut<VaelSave>>,
    mut stations: Query<(Entity, &mut Rootstock, Option<&PooledLook>)>,
    mut shoots: Query<&mut Transform>,
    remains: Remains,
) {
    let Some(clock) = clock else { return };
    let today = clock.hours_since_2000() / 24.0;
    for (entity, mut root, look) in &mut stations {
        let cut = match root.cut_day {
            Some(day) => day,
            None => {
                let day = root.key.as_deref().map_or(today, |key| {
                    let field = format!("flora-fell.{key}.day");
                    let saved = vael_engine::luau::game::game_get_json(&field)
                        .or_else(|| save.as_ref()?.legacy.get(&field).cloned());
                    if let Some(day) = saved.and_then(|v| v.as_f64()) {
                        return day;
                    }
                    if let Some(save) = save.as_mut() {
                        save.legacy.insert(field.clone(), json!(today));
                    }
                    vael_engine::luau::game::apply_patch_json(&BTreeMap::from([(field, json!(today))]));
                    today
                });
                root.cut_day = Some(day);
                day
            }
        };
        let age = today - cut;
        if age < SPROUT_AFTER_DAYS {
            continue;
        }
        let t = ((age - SPROUT_AFTER_DAYS) / (MATURE_AFTER_DAYS - SPROUT_AFTER_DAYS)).clamp(0., 1.)
            as f32;
        // A small living crown emerges through the stump and reaches the original authored scale.
        if age < MATURE_AFTER_DAYS {
            let scale = Vec3::splat(0.07 + 0.93 * t * t * (3.0 - 2.0 * t));
            if let Some(shoot) = root.shoot {
                if let Ok(mut transform) = shoots.get_mut(shoot) {
                    transform.scale = scale;
                }
            } else {
                // Explicit visibility under the hidden station. Only the shoot scales:
                // the stump collider and the authored terrain placement stay full size.
                let mut shoot = commands.spawn((
                    root.pooled.clone(),
                    ChildOf(entity),
                    Transform::from_scale(scale),
                    Visibility::Visible,
                ));
                if let Some(look) = look {
                    shoot.insert(*look);
                }
                root.shoot = Some(shoot.id());
            }
            continue;
        }
        restore(&mut commands, entity, &mut root, &remains);
        if let Some(key) = &root.key {
            let mut patch = BTreeMap::new();
            for field in ["dx", "dz", "cut", "bucked", "day", "detached"] {
                patch.insert(format!("flora-fell.{key}.{field}"), Json::Null);
            }
            patch.insert(format!("harvested.{key}"), Json::Null);
            // Old Lua saves used the bare station name as well as the stable felling key.
            if !root.harvest.name.is_empty() {
                patch.insert(format!("harvested.{}", root.harvest.name), Json::Null);
            }
            if let Some(save) = save.as_mut() {
                save.harvested.remove(key);
                save.harvested.remove(&root.harvest.name);
                for field in patch.keys() {
                    save.legacy.remove(field);
                }
            }
            vael_engine::luau::game::apply_patch_json(&patch);
        }
    }
}

#[derive(bevy::ecs::system::SystemParam)]
pub(super) struct Remains<'w, 's> {
    stumps: Query<'w, 's, (Entity, &'static TreeStump)>,
    logs: Query<'w, 's, (Entity, &'static TreeLog)>,
}

fn restore(commands: &mut Commands, entity: Entity, root: &mut Rootstock, remains: &Remains) {
    if let Some(shoot) = root.shoot {
        commands.entity(shoot).try_despawn();
    }
    commands.entity(root.tree).try_despawn();
    for (stump_id, stump) in &remains.stumps {
        if stump.tree == Some(root.tree) {
            for (id, log) in &remains.logs {
                if log.stump == stump_id {
                    commands.entity(id).try_despawn();
                }
            }
            commands.entity(stump_id).try_despawn();
        }
    }
    let mut station = commands.entity(entity);
    station
        .remove::<(Rootstock, FloraFellTree, Collider)>()
        .insert((
            root.harvest.clone(),
            root.pooled.clone(),
            Visibility::Inherited,
        ));
    if let Some(props) = root.props.take() {
        station.insert(props);
    }
    if let Some(script) = root.script.take() {
        station.insert(script);
    }
    if let Some(collider) = root.collider.take() {
        station.insert(collider);
    }
}

pub(super) fn reset_on_load(
    mut commands: Commands,
    mut roots: Query<(Entity, &mut Rootstock)>,
    remains: Remains,
) {
    for (entity, mut root) in &mut roots {
        restore(&mut commands, entity, &mut root, &remains);
    }
}

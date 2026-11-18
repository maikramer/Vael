//! Share the GLB scene and clip graph across each species. Playback is per animal.
use bevy::{gltf::Gltf, prelude::*, world_serialization::WorldInstanceReady};
use std::{collections::HashMap, time::Duration};

#[derive(Component)]
pub(super) struct FaunaVisual {
    species: &'static str,
    clip: &'static str,
    speed: f32,
    phase: f32,
}

impl FaunaVisual {
    pub(super) fn new(species: &'static str, phase: f32) -> Self {
        Self {
            species,
            clip: "BreatheIdle",
            speed: 1.,
            phase,
        }
    }

    pub(super) fn play(&mut self, clip: &'static str, speed: f32) {
        self.clip = clip;
        self.speed = speed;
    }
}

struct Model {
    gltf: Handle<Gltf>,
    graph: Option<Handle<AnimationGraph>>,
    clips: HashMap<String, AnimationNodeIndex>,
}

#[derive(Resource, Default)]
struct Models(HashMap<&'static str, Model>);

#[derive(Component)]
struct RigPlayer(Entity);

pub(super) fn install(app: &mut App) {
    app.init_resource::<Models>()
        .add_observer(bind_player)
        .add_systems(Update, attach_scenes)
        .add_systems(PostUpdate, play_clips.before(bevy::app::AnimationSystems));
}

fn attach_scenes(
    mut commands: Commands,
    server: Res<AssetServer>,
    gltfs: Res<Assets<Gltf>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut models: ResMut<Models>,
    waiting: Query<(Entity, &FaunaVisual), Without<WorldAssetRoot>>,
) {
    for (entity, visual) in &waiting {
        let model = models.0.entry(visual.species).or_insert_with(|| Model {
            gltf: viber::meshopt::load_gltf(
                &server,
                format!("assets/meshes/fauna/{}_lod0.glb", visual.species),
            ),
            graph: None,
            clips: HashMap::new(),
        });
        let Some(gltf) = gltfs.get(&model.gltf) else {
            continue;
        };
        let Some(scene) = &gltf.default_scene else {
            continue;
        };
        if model.graph.is_none() {
            let mut graph = AnimationGraph::new();
            for (name, clip) in &gltf.named_animations {
                let node = graph.add_clip(clip.clone(), 1., graph.root);
                model
                    .clips
                    .insert(name.trim_start_matches("Animator3D_").to_owned(), node);
            }
            model.graph = Some(graphs.add(graph));
        }
        commands
            .entity(entity)
            .insert(WorldAssetRoot(scene.clone()));
    }
}

fn bind_player(
    ready: On<WorldInstanceReady>,
    mut commands: Commands,
    visuals: Query<&FaunaVisual>,
    models: Res<Models>,
    children: Query<&Children>,
    players: Query<(), With<AnimationPlayer>>,
) {
    let Ok(visual) = visuals.get(ready.entity) else {
        return;
    };
    let Some(graph) = &models.0[visual.species].graph else {
        return;
    };
    if let Some(player) = children
        .iter_descendants(ready.entity)
        .find(|e| players.contains(*e))
    {
        commands.entity(player).insert((
            AnimationGraphHandle(graph.clone()),
            AnimationTransitions::new(),
        ));
        commands.entity(ready.entity).insert(RigPlayer(player));
    }
}

fn play_clips(
    models: Res<Models>,
    animals: Query<(&FaunaVisual, &RigPlayer)>,
    mut players: Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
) {
    for (visual, rig) in &animals {
        let Some(&node) = models.0[visual.species].clips.get(visual.clip) else {
            continue;
        };
        let Ok((mut player, mut transitions)) = players.get_mut(rig.0) else {
            continue;
        };
        if transitions.get_main_animation() == Some(node) {
            if let Some(active) = player.animation_mut(node) {
                active.set_speed(visual.speed);
            }
        } else {
            let active = transitions.play(&mut player, node, Duration::from_millis(100));
            active.set_speed(visual.speed);
            if visual.clip != "Land" {
                active.repeat().set_seek_time(visual.phase.fract() * 0.35);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{
        animation::AnimationPlugin,
        asset::{
            AssetPlugin,
            io::{AssetSourceBuilder, AssetSourceId, file::FileAssetReader},
        },
        gltf::GltfPlugin,
        image::{CompressedImageFormatSupport, CompressedImageFormats, ImagePlugin},
        mesh::MeshPlugin,
        time::TimeUpdateStrategy,
        world_serialization::WorldSerializationPlugin,
    };
    use std::{path::Path, thread, time::Instant};

    #[test]
    fn shipped_animals_bind_and_switch_their_own_clips() {
        let mut app = App::new();
        let pool = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/public");
        app.register_asset_source(
            AssetSourceId::Default,
            AssetSourceBuilder::new(move || {
                Box::new(viber::meshopt::MeshoptAssetReader::new(Box::new(
                    FileAssetReader::new(&pool),
                )))
            }),
        );
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            TransformPlugin,
            WorldSerializationPlugin,
            ImagePlugin::default(),
            MeshPlugin,
            AnimationPlugin,
            GltfPlugin::default(),
        ))
        .insert_resource(CompressedImageFormatSupport(CompressedImageFormats::BC))
        .init_asset::<StandardMaterial>()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            16,
        )));
        install(&mut app);
        app.finish();
        app.cleanup();
        let sets: &[(&str, &[&str])] = &[
            ("fox", &["BreatheIdle", "Walk", "Run"]),
            ("rat", &["BreatheIdle", "Walk", "Run"]),
            ("hare", &["BreatheIdle", "Jump", "Run"]),
            ("swallow", &["Hover", "Soar"]),
            ("butterfly", &["Hover", "Soar"]),
            ("jay", &["Perch", "Hover", "Soar", "Land"]),
            ("owl", &["Perch", "Hover", "Soar", "Land"]),
            ("trout", &["SwimIdle", "Swim"]),
        ];
        let animals: Vec<_> = sets
            .iter()
            .map(|(species, clips)| {
                let mut visual = FaunaVisual::new(species, 0.);
                visual.play(clips[0], 1.);
                app.world_mut().spawn((visual, Transform::default())).id()
            })
            .collect();
        let start = Instant::now();
        while animals
            .iter()
            .any(|&e| app.world().get::<RigPlayer>(e).is_none())
        {
            app.update();
            for (name, model) in &app.world().resource::<Models>().0 {
                if let Some(bevy::asset::LoadState::Failed(err)) = app
                    .world()
                    .resource::<AssetServer>()
                    .get_load_state(model.gltf.id())
                {
                    panic!("{name} failed to load: {err}");
                }
            }
            assert!(
                start.elapsed() < Duration::from_secs(30),
                "fauna rigs did not bind"
            );
            thread::sleep(Duration::from_millis(2));
        }
        for (&entity, &(species, clips)) in animals.iter().zip(sets) {
            let rig = app.world().get::<RigPlayer>(entity).unwrap().0;
            let mut subtree = vec![entity];
            let joints = loop {
                let node = subtree.pop().expect("animal must have a skinned mesh");
                if let Some(skin) = app.world().get::<bevy::mesh::skinning::SkinnedMesh>(node) {
                    break skin.joints.clone();
                }
                if let Some(children) = app.world().get::<Children>(node) {
                    subtree.extend(children.iter());
                }
            };
            for &clip in clips {
                app.world_mut()
                    .get_mut::<FaunaVisual>(entity)
                    .unwrap()
                    .play(clip, 1.5);
                app.update();
                app.update();
                let node = app.world().resource::<Models>().0[species].clips[clip];
                let transitions = app.world().get::<AnimationTransitions>(rig).unwrap();
                assert_eq!(
                    transitions.get_main_animation(),
                    Some(node),
                    "{species}/{clip}"
                );
                let active = app
                    .world()
                    .get::<AnimationPlayer>(rig)
                    .unwrap()
                    .animation(node)
                    .unwrap();
                assert!(active.seek_time() > 0., "{species}/{clip} did not advance");
                let pose: Vec<_> = joints
                    .iter()
                    .map(|&e| *app.world().get::<Transform>(e).unwrap())
                    .collect();
                for _ in 0..6 {
                    app.update();
                }
                assert!(
                    joints
                        .iter()
                        .zip(&pose)
                        .any(|(&e, before)| { app.world().get::<Transform>(e).unwrap() != before }),
                    "{species}/{clip} did not move the skeleton"
                );
                if clip == "Hover" {
                    let required: &[&str] = if species == "butterfly" {
                        &["WingMid.L", "WingMid.R", "HindwingMid.L", "HindwingMid.R"]
                    } else {
                        &["WingMid.L", "WingMid.R"]
                    };
                    for &name in required {
                        let (joint, before) = joints
                            .iter()
                            .zip(&pose)
                            .find(|(joint, _)| {
                                app.world()
                                    .get::<Name>(**joint)
                                    .is_some_and(|n| n.as_str() == name)
                            })
                            .unwrap_or_else(|| panic!("{species} has no skinned {name} joint"));
                        let after = app.world().get::<Transform>(*joint).unwrap();
                        assert!(
                            before.rotation.angle_between(after.rotation) > 0.005,
                            "{species}/{name} did not flex during the stroke"
                        );
                    }
                }
            }
        }
    }
}

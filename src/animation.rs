//! Animação autorada no Jackdaw (`jackdaw_animation_runtime`).
//!
//! Um nó com `AnimationSet` (estados → clips, transições, `then`) chega à
//! entidade pela ponte de componentes nativos ([`crate::editor_link`]). A
//! partir daí o rig é do runtime do Jackdaw, não do `CharacterAnimator` da
//! engine:
//!
//! - [`cede_rig_to_jackdaw`] tira à entidade o `AnimatedScene` (o pedido de
//!   bind da engine) e um `CharacterAnimator` já ligado. Um set sem
//!   `sources` usa o glTF que a engine já carregou para o modelo
//!   (`AnimationSources`), por isso basta autorar os estados;
//! - [`drive_locomotion`] escreve o `AnimationState` a partir da velocidade
//!   no chão: `run` / `walk` / `idle` (os que o set tiver). Um estado fora
//!   destes (ataque, gesto, morte) é de quem o pediu: só volta à locomoção
//!   quando o runtime o devolve (`then`) a um estado de locomoção;
//! - [`play_unbound_gesture`]: um `viber.gesture('talk')` do Lua num NPC sem
//!   rig da engine toca o estado homónimo do set (o próprio NPC ou o modelo
//!   dentro dele).
//!
//! Entidades sem `AnimationSet` continuam exactamente como estavam.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use jackdaw_animation_runtime::{AnimationSet, AnimationSources, AnimationState};
use viber::animation::{AnimatedScene, CharacterAnimator};

/// Estados de locomoção conduzidos pela velocidade, por ordem de rapidez.
pub const LOCOMOTION: [&str; 3] = ["idle", "walk", "run"];

/// Velocidade horizontal (m/s) acima da qual se anda / corre.
pub const WALK_SPEED: f32 = 0.35;
pub const RUN_SPEED: f32 = 4.5;

pub struct VaelAnimationPlugin;

impl Plugin for VaelAnimationPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(play_unbound_gesture);
        app.add_systems(
            Update,
            (cede_rig_to_jackdaw, drive_locomotion)
                .chain()
                .before(jackdaw_animation_runtime::AnimationSetSystems),
        );
    }
}

/// A engine larga o rig das entidades que trazem um `AnimationSet`.
///
/// O pedido de bind da engine (`AnimatedScene`) e o `CharacterAnimator` ficam
/// na entidade do personagem, que pode ser um antepassado do nó do modelo
/// (o `GltfScene` dentro do NPC): sobe-se pelos pais até encontrar um deles.
pub fn cede_rig_to_jackdaw(
    mut commands: Commands,
    mut sets: Query<(
        Entity,
        &mut AnimationSet,
        Option<&AnimatedScene>,
        Option<&AnimationSources>,
        Option<&viber::scene::SceneSource>,
    )>,
    parents: Query<&ChildOf>,
    engine: Query<(), Or<(With<AnimatedScene>, With<CharacterAnimator>)>>,
    scenes: Query<&AnimatedScene>,
) {
    for (entity, mut set, scene, sources, origin) in &mut sets {
        // Set sem ficheiros: os clips vêm do modelo que a engine carregou —
        // o `url` do nó (`/assets/…glb` → `assets/…glb` no `AssetServer`);
        // a cena instanciada é montada em memória e não tem caminho.
        if set.sources.is_empty() && sources.is_none() {
            let path = scene
                .and_then(|scene| scene.gltf.path())
                .map(|path| path.without_label().to_string())
                .or_else(|| {
                    origin
                        .and_then(|origin| bevy_ext_world::bsn::specs::model_url_for(&origin.0))
                        .map(|url| url.trim_start_matches('/').to_string())
                });
            if let Some(path) = path {
                set.sources = vec![path];
                if let Some(scene) = scene {
                    commands
                        .entity(entity)
                        .insert(AnimationSources(vec![scene.gltf.clone()]));
                }
            }
        }
        let mut at = Some(entity);
        for _ in 0..8 {
            let Some(current) = at else { break };
            if engine.contains(current) {
                if set.sources.is_empty()
                    && let Ok(scene) = scenes.get(current)
                    && let Some(path) = scene.gltf.path()
                {
                    set.sources = vec![path.without_label().to_string()];
                }
                commands
                    .entity(current)
                    .remove::<(AnimatedScene, CharacterAnimator)>();
                break;
            }
            at = parents.get(current).ok().map(ChildOf::parent);
        }
    }
}

/// O estado do set com este nome (sem distinguir maiúsculas).
pub fn state_named<'a>(set: &'a AnimationSet, name: &str) -> Option<&'a str> {
    set.states
        .iter()
        .find(|state| state.name.eq_ignore_ascii_case(name))
        .map(|state| state.name.as_str())
}

/// `viber.gesture` sem rig da engine → `AnimationState` do set do NPC.
fn play_unbound_gesture(
    gesture: On<viber::luau::runtime::UnboundGesture>,
    children: Query<&Children>,
    mut sets: Query<(&AnimationSet, Option<&mut AnimationState>)>,
    mut commands: Commands,
) {
    let mut stack = vec![gesture.entity];
    let mut visited = 0;
    while let Some(entity) = stack.pop() {
        visited += 1;
        if visited > 256 {
            return;
        }
        if let Ok((set, state)) = sets.get_mut(entity)
            && let Some(name) = state_named(set, &gesture.name)
        {
            debug!("vael: gesto '{name}' → AnimationState do Jackdaw em {entity}");
            match state {
                Some(mut state) => state.0 = name.to_string(),
                None => {
                    commands
                        .entity(entity)
                        .try_insert(AnimationState(name.to_string()));
                }
            }
            return;
        }
        if let Ok(kids) = children.get(entity) {
            stack.extend(kids.iter());
        }
    }
}

/// O estado de locomoção para uma velocidade, entre os que o set define.
pub fn locomotion_state(set: &AnimationSet, speed: f32) -> Option<&'static str> {
    let has = |name: &str| set.states.iter().any(|state| state.name == name);
    let wanted: &[&'static str] = if speed >= RUN_SPEED {
        &["run", "walk", "idle"]
    } else if speed >= WALK_SPEED {
        &["walk", "idle"]
    } else {
        &["idle"]
    };
    wanted.iter().copied().find(|name| has(name))
}

fn drive_locomotion(
    time: Res<Time>,
    mut commands: Commands,
    mut sets: Query<(
        Entity,
        &GlobalTransform,
        &AnimationSet,
        Option<&mut AnimationState>,
    )>,
    mut last: Local<HashMap<Entity, Vec3>>,
) {
    let dt = time.delta_secs();
    let mut seen = Vec::new();
    for (entity, transform, set, state) in &mut sets {
        seen.push(entity);
        let position = transform.translation();
        let previous = last.insert(entity, position);
        // Primeiro frame (sem posição anterior) ou `dt` nulo: parado.
        let speed = match previous {
            Some(previous) if dt > 0.0 => {
                Vec2::new(position.x - previous.x, position.z - previous.z).length() / dt
            }
            _ => 0.0,
        };
        let Some(wanted) = locomotion_state(set, speed) else {
            continue;
        };
        match state {
            None => {
                commands
                    .entity(entity)
                    .try_insert(AnimationState(wanted.to_string()));
            }
            Some(mut state) => {
                let ours = state.0.is_empty() || LOCOMOTION.contains(&state.0.as_str());
                if ours && state.0 != wanted {
                    state.0 = wanted.to_string();
                }
            }
        }
    }
    last.retain(|entity, _| seen.contains(entity));
}

#[cfg(test)]
mod tests {
    use super::*;
    use jackdaw_animation_runtime::AnimationStateDef;

    fn set(names: &[&str]) -> AnimationSet {
        AnimationSet {
            states: names
                .iter()
                .map(|name| AnimationStateDef {
                    name: name.to_string(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn escolhe_pela_velocidade_entre_os_estados_que_existem() {
        let full = set(&["idle", "walk", "run"]);
        assert_eq!(locomotion_state(&full, 0.0), Some("idle"));
        assert_eq!(locomotion_state(&full, 1.0), Some("walk"));
        assert_eq!(locomotion_state(&full, 6.0), Some("run"));
        // Sem "run", corre a andar.
        assert_eq!(locomotion_state(&set(&["idle", "walk"]), 6.0), Some("walk"));
        assert_eq!(locomotion_state(&set(&["attack"]), 6.0), None);
    }

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_systems(Update, (cede_rig_to_jackdaw, drive_locomotion).chain());
        app
    }

    #[test]
    fn engine_larga_o_rig() {
        let mut app = app();
        let gltf: Handle<bevy::gltf::Gltf> = Handle::default();
        let entity = app
            .world_mut()
            .spawn((
                set(&["idle"]),
                AnimatedScene { gltf: gltf.clone() },
                GlobalTransform::default(),
            ))
            .id();
        app.update();
        let world = app.world();
        assert!(world.get::<AnimatedScene>(entity).is_none());
        assert_eq!(
            world.get::<AnimationState>(entity).map(|s| s.0.as_str()),
            Some("idle")
        );
    }

    #[test]
    fn gesto_sem_rig_da_engine_toca_o_estado_do_modelo() {
        let mut app = app();
        app.add_observer(play_unbound_gesture);
        let npc = app.world_mut().spawn(GlobalTransform::default()).id();
        let model = app
            .world_mut()
            .spawn((
                set(&["idle", "Talk"]),
                GlobalTransform::default(),
                ChildOf(npc),
            ))
            .id();
        app.update();
        app.world_mut().trigger(viber::luau::runtime::UnboundGesture {
            entity: npc,
            name: "talk".into(),
            speed: 1.0,
        });
        app.world_mut().flush();
        assert_eq!(app.world().get::<AnimationState>(model).unwrap().0, "Talk");
    }

    #[test]
    fn um_ataque_em_curso_nao_e_atropelado() {
        let mut app = app();
        let entity = app
            .world_mut()
            .spawn((
                set(&["idle", "walk", "attack"]),
                AnimationState("attack".into()),
                GlobalTransform::default(),
            ))
            .id();
        app.update();
        app.update();
        assert_eq!(
            app.world().get::<AnimationState>(entity).unwrap().0,
            "attack"
        );
    }
}

//! glTF animation — shim do monólito + os sistemas presos ao host.
//!
//! O núcleo (bind/rig, blend de locomoção, play helpers, hit-react,
//! `drive_player_animation`, `PendingGesture`) vive no
//! `bevy_ext_engine::animation` (exit5/G-2); o caminho `viber::animation`
//! reexporta-o em baixo. Ficam AQUI os sistemas que leem contratos do
//! monólito: `drive_character_animation` e `freeze_distant_animation` leem o
//! raio de ativação do script (`crate::luau::ScriptActivation`) e o
//! `AnimationPlugin` encadeia `drive_player_animation` DEPOIS do
//! `crate::player::player_movement`.
//!
//! O TypePath histórico dos tipos que ficaram (`viber::animation::…`) segue
//! a resolver por este módulo.

pub use bevy_ext_engine::animation::*;

use bevy::animation::graph::{AnimationGraphHandle, AnimationNodeIndex};
use bevy::animation::transition::AnimationTransitions;
use bevy::animation::AnimationPlayer;
use bevy::prelude::*;
/// Drives every other animated character (NPCs, scripted creatures, the Rust
/// FSM enemies) from its own displacement.
///
/// Creatures further than their [`crate::luau::ScriptActivation`] radius are
/// skipped: their clip keeps looping (it costs nothing extra once bound) but the
/// driver does no work and, importantly, drops the position baseline so the
/// character does not read as teleporting when it comes back in range.
#[allow(clippy::type_complexity)]
pub fn drive_character_animation(
    mut characters: Query<
        (
            &GlobalTransform,
            &mut CharacterAnimator,
            Option<&crate::luau::ScriptActivation>,
            Option<&LocomotionProfile>,
        ),
        (Without<crate::player::Player>, Without<ManualAnimation>),
    >,
    hero: Query<&GlobalTransform, With<crate::player::Player>>,
    mut players: PlayerQuery,
    handles: Query<&AnimationGraphHandle>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    time: Res<Time>,
) {
    let _probe = bevy_engine_core::perf_probe::Probe::new("anim.drive");
    let dt = time.delta_secs().max(1e-4);
    let hero_positions: Vec<Vec3> = hero.iter().map(GlobalTransform::translation).collect();
    for (transform, mut animator, activation, profile) in &mut characters {
        if animator.locked {
            continue; // dead: the terminal clip holds the pose
        }
        let position = transform.translation();
        // Longe de TODOS os heróis: congela o driver. Um único herói a menos
        // de `radius` mantém a criatura animada (co-op).
        if let Some(activation) = activation
            && nearest_hero_distance(position, &hero_positions)
                .is_some_and(|distance| distance > activation.radius)
        {
            // Drop the baseline so the first tick back in range measures a
            // frame, not the whole trip.
            animator.last_pos = None;
            continue;
        }
        // A rig with no profile yet falls back to the module-wide creature
        // nominals — the behaviour this driver always had.
        let (speeds, gates) = match profile {
            Some(p) => (profile_speeds(p), profile_gates(p)),
            None => (
                MotionSpeeds::CREATURE,
                MotionGates::from_speeds(MotionSpeeds::CREATURE),
            ),
        };
        // Creatures are always grounded as far as the clip picker is concerned;
        // none of the rigs in the pipeline ship airborne clips for them.
        let state = advance_motion(&mut animator, position, true, 0.0, dt, gates);
        let blended = animator.locomotion.is_some() && !animator.is_busy();
        if let Ok(handle) = handles.get(animator.player) {
            let weights = locomotion_weights(animator.speed, gates, speeds);
            drive_locomotion_blend(
                &mut animator,
                &mut players,
                &mut graphs,
                handle,
                weights,
                speeds,
                blended,
            );
        }
        if !blended {
            play_state(&mut animator, &mut players, state);
        } else {
            animator.state = Some(state);
        }
        if animator.locomotion.is_none() {
            sync_clip_speed(&animator, &mut players, state, speeds);
        }
    }
}

/// Below this camera-independent distance to the hero a bound character
/// always animates, even past its script's activation radius — an idle

pub const ANIM_FREEZE_MIN_DISTANCE: f32 = 70.0;
/// Hysteresis band for [`freeze_distant_animation`]: a frozen rig wakes up
/// only once it is back inside this fraction of the freeze distance, so a
/// creature pacing along the boundary does not flip archetypes every frame.
const ANIM_FREEZE_HYSTERESIS: f32 = 0.9;


/// Distance past which a character's rig stops animating.
fn freeze_distance(activation: Option<&crate::luau::ScriptActivation>) -> f32 {
    activation
        .map_or(0.0, |a| a.radius)
        .max(ANIM_FREEZE_MIN_DISTANCE)
}

/// Whether a rig should be frozen this frame, given its current state.
fn should_freeze(frozen: bool, distance: f32, freeze_at: f32, visible: bool) -> bool {
    if !visible {
        return true;
    }
    if frozen {
        distance > freeze_at * ANIM_FREEZE_HYSTERESIS
    } else {
        distance > freeze_at
    }
}

/// Parks the rigs of characters far from the hero or hidden by the distance
/// cull ([`crate::render_lod::CullDistance`]), and wakes them back up.
///
/// With ~300 creatures in the `simple-rpg`, of which a few dozen are ever
/// near the hero, the parked rigs were the bulk of `animate_targets` and of
/// the ~4 ms `transform_propagate` — every bone of every creature on the map
/// rewritten every frame, for creatures nobody could see move.
#[allow(clippy::type_complexity)]
pub fn freeze_distant_animation(
    mut commands: Commands,
    hero: Query<&GlobalTransform, With<crate::player::Player>>,
    characters: Query<
        (
            &GlobalTransform,
            &CharacterAnimator,
            Option<&crate::luau::ScriptActivation>,
            // O ragdoll é dono do rig: `ragdoll::freeze_ragdoll_graphs`
            // tira o grafo para os ossos seguirem os corpos. Sem esta
            // exclusão este sistema ACORDAVA-o ("está perto, visível"),
            // o ragdoll voltava a congelá-lo no frame seguinte e a pose
            // alternava entre o clip de morte e os corpos a cada frame.
            Option<&crate::ragdoll::RagdollActive>,
        ),
        Without<crate::player::Player>,
    >,
    rigs: Query<(
        Option<&AnimationGraphHandle>,
        Option<&FrozenAnimation>,
        Option<&InheritedVisibility>,
    )>,
) {
    let _probe = bevy_engine_core::perf_probe::Probe::new("anim.freeze");
    // Todos os heróis: com 2+ players o `single()` falhava e NENHUM rig era
    // congelado (a otimização inteira morria em co-op). A distância é à mais
    // próxima — um NPC entre os dois heróis continua a animar.
    let hero_positions: Vec<Vec3> = hero.iter().map(GlobalTransform::translation).collect();
    if hero_positions.is_empty() {
        return;
    }
    for (transform, animator, activation, ragdoll) in &characters {
        if ragdoll.is_some() {
            continue;
        }
        let Ok((graph, frozen, visibility)) = rigs.get(animator.player) else {
            continue;
        };
        let visible = visibility.is_none_or(|v| v.get());
        let Some(distance) = nearest_hero_distance(transform.translation(), &hero_positions) else {
            continue;
        };
        let freeze = should_freeze(
            frozen.is_some(),
            distance,
            freeze_distance(activation),
            visible,
        );
        match (freeze, graph, frozen) {
            (true, Some(graph), None) => {
                commands
                    .entity(animator.player)
                    .remove::<AnimationGraphHandle>()
                    .insert(FrozenAnimation(graph.clone()));
            }
            (false, None, Some(FrozenAnimation(graph))) => {
                commands
                    .entity(animator.player)
                    .remove::<FrozenAnimation>()
                    .insert(graph.clone());
            }
            _ => {}
        }
    }
}

/// Animator mínimo para testes de outros módulos (ragdoll).
#[cfg(test)]
pub fn test_animator(player: Entity) -> CharacterAnimator {
    CharacterAnimator {
        state: Some(AnimState::Idle),
        ..CharacterAnimator::new(
            vec!["idle".into()],
            vec![AnimationNodeIndex::new(1)],
            vec![1.0],
            player,
            None,
        )
    }
}


/// Registers clip binding, action arbitration and the motion drivers.
#[derive(Default)]
pub struct AnimationPlugin;

impl bevy::app::Plugin for AnimationPlugin {
    fn build(&self, app: &mut bevy::app::App) {
        // O bind corre em `SpawnScene`, DEPOIS de o `WorldAssetRoot` virar a
        // subárvore em entidades. No `Update` a cena nova ainda não existia e
        // o `AnimationPlayer` fresco só era encontrado no frame seguinte — a
        // troca de LOD renderizava um frame em T-pose (o "pisca" dos NPCs e
        // criaturas ao cruzar 18/45 m). O `rearm_after_scene_swap` fica no
        // `Update` (só insere `AnimatedScene`), por isso o bind daqui já o vê.
        app.add_systems(
            bevy::app::SpawnScene,
            bind_animations.after(bevy::app::SceneSpawnerSystems::WorldInstanceSpawn),
        );
        app.add_systems(
            bevy::app::Update,
            (
                rearm_after_scene_swap,
                play_pending_gestures,
                tick_animator_actions,
                // O flinch corre antes dos drivers: um one-shot que ele toca
                // bloqueia o `play_state` do driver no mesmo frame (`is_busy`).
                hit_react_system,
                // The hero driver reads `Player::grounded` / `vel_y`, which the
                // controller refreshes in `player_movement`; without the order
                // the airborne state was a frame stale at random.
                drive_player_animation.after(crate::player::player_movement),
                drive_character_animation,
                freeze_distant_animation,
            )
                .chain(),
        );
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn animator(names: &[&str]) -> CharacterAnimator {
        let clip_names: Vec<String> = names.iter().map(|s| s.to_string()).collect();
        let nodes = (0..names.len())
            .map(|i| AnimationNodeIndex::new(i + 1))
            .collect();
        CharacterAnimator::new(
            clip_names,
            nodes,
            vec![1.0; names.len()],
            Entity::PLACEHOLDER,
            None,
        )
    }

    #[test]

    fn test_freeze_distance_never_drops_below_the_floor() {
        assert_eq!(freeze_distance(None), ANIM_FREEZE_MIN_DISTANCE);
        let near = crate::luau::ScriptActivation { radius: 45.0 };
        assert_eq!(freeze_distance(Some(&near)), ANIM_FREEZE_MIN_DISTANCE);
        let far = crate::luau::ScriptActivation { radius: 120.0 };
        assert_eq!(freeze_distance(Some(&far)), 120.0);
    }

    /// Co-op: a distância que conta é a do herói MAIS PRÓXIMO. Com um único
    /// herói o `single()` funcionava; com dois, falhava e o LOD de animação
    /// (driver + freeze) desligava-se no mundo inteiro.

    #[test]

    fn test_should_freeze_has_hysteresis_and_obeys_visibility() {
        // Hidden by the distance cull: frozen regardless of distance.
        assert!(should_freeze(false, 1.0, 70.0, false));
        // Awake rig freezes only past the edge.
        assert!(!should_freeze(false, 69.0, 70.0, true));
        assert!(should_freeze(false, 71.0, 70.0, true));
        // Frozen rig inside the band stays frozen, wakes past it.
        assert!(should_freeze(true, 65.0, 70.0, true));
        assert!(!should_freeze(true, 60.0, 70.0, true));
    }

    #[test]

    fn test_freeze_parks_and_restores_the_graph_handle() {
        let mut app = App::new();
        app.add_systems(Update, freeze_distant_animation);
        app.world_mut()
            .spawn((crate::player::Player::default(), GlobalTransform::default()));
        let graph = AnimationGraphHandle(Handle::default());
        let rig = app.world_mut().spawn(graph).id();
        let creature = app
            .world_mut()
            .spawn((
                GlobalTransform::from_translation(Vec3::new(200.0, 0.0, 0.0)),
                animator(&["idle"]),
            ))
            .id();
        app.world_mut()
            .get_mut::<CharacterAnimator>(creature)
            .unwrap()
            .player = rig;
        app.update();
        assert!(
            app.world().get::<AnimationGraphHandle>(rig).is_none(),
            "far rig parked"
        );
        assert!(app.world().get::<FrozenAnimation>(rig).is_some());
        app.world_mut()
            .entity_mut(creature)
            .insert(GlobalTransform::from_translation(Vec3::new(10.0, 0.0, 0.0)));
        app.update();
        assert!(
            app.world().get::<AnimationGraphHandle>(rig).is_some(),
            "near rig restored"
        );
        assert!(app.world().get::<FrozenAnimation>(rig).is_none());
    }

}

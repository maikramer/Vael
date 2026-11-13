//! Player do Vael — o primeiro corte da aposentadoria da engine.
//!
//! O spawner da engine ainda cria o `<PlayerGLTF>` (Gltf, collider e
//! `KinematicCharacterController`), mas com `player_camera:false` os sistemas
//! dela não correm. Este módulo anexa o [`VaelController`] e conduz o herói
//! (WASD, salto, steering da `OrbitCamera`, chão analítico e teleportes); o
//! estado do `viber::player::Player` é sincronizado para a animação da engine
//! continuar a ler velocidade/chão enquanto essa camada não é portada. A
//! câmara de terceira pessoa vive em [`crate::camera`].

use bevy::prelude::*;
use bevy_rapier3d::prelude::{KinematicCharacterController, KinematicCharacterControllerOutput};
use viber::recipes::spawn::OrbitCamera;

use crate::components::VaelPlayer;

pub const GRAVITY: f32 = 60.0;
pub const TERMINAL_VELOCITY: f32 = 55.0;
pub const CAMERA_TURN_SPEED: f32 = 2.5;
pub const SIDE_MOVE_FACTOR: f32 = 0.6;
pub const JUMP_BUFFER: f32 = 0.1;
pub const COYOTE_TIME: f32 = 0.1;
pub const JUMP_COOLDOWN: f32 = 0.2;
pub const ACCEL_TAU: f32 = 0.08;
pub const DECEL_TAU: f32 = 0.14;
pub const AIR_TAU: f32 = 0.3;
/// Sonda acima dos pés ao perguntar `surface_below` pelo chão: tem de
/// exceder a skin do character controller (0.02).
pub const GROUND_PROBE: f32 = 0.05;
/// Queda abaixo do topo do mundo que já não é gruta: o herói furou o
/// collider e é resgatado para a superfície.
pub const VOID_RESCUE_DEPTH: f32 = 250.0;

/// Estado de movimento do herói. É a cópia local do subconjunto do
/// `viber::player::Player` que o controlador usa.
#[derive(Component, Debug, Clone, Reflect)]
#[reflect(Component)]
pub struct VaelController {
    pub speed: f32,
    pub sprint_multiplier: f32,
    pub jump_height: f32,
    pub rotation_speed: f32,
    pub vel_y: f32,
    pub vel_x: f32,
    pub vel_z: f32,
    pub grounded: bool,
    pub can_jump: bool,
    pub is_jumping: bool,
    pub jump_cooldown: f32,
    pub last_grounded_time: f32,
    pub jump_buffer_time: f32,
    pub last_steer_time: f32,
    pub last_moving_time: f32,
    /// Já viu o [`viber::player::TeleportSettle`] corrente (distingue a
    /// CHEGADA de um teleporte da tutela a meio). Ver `player_movement`.
    pub settle_seen: bool,
}

impl Default for VaelController {
    fn default() -> Self {
        Self {
            speed: 4.0,
            sprint_multiplier: 1.5,
            jump_height: 2.3,
            rotation_speed: 10.0,
            vel_y: 0.0,
            vel_x: 0.0,
            vel_z: 0.0,
            grounded: true,
            can_jump: true,
            is_jumping: false,
            jump_cooldown: 0.0,
            last_grounded_time: f32::NEG_INFINITY,
            jump_buffer_time: f32::NEG_INFINITY,
            last_steer_time: f32::NEG_INFINITY,
            last_moving_time: f32::INFINITY,
            settle_seen: false,
        }
    }
}

/// Anexa o controller às entidades que o spawner marcou com `Player`.
fn attach_controllers(
    mut commands: Commands,
    players: Query<(Entity, Option<&VaelController>), With<viber::player::Player>>,
) {
    for (entity, controller) in &players {
        commands.entity(entity).insert(VaelPlayer);
        if controller.is_none() {
            commands.entity(entity).insert(VaelController::default());
        }
    }
}

fn no_editor_camera(cameras: Query<(), With<viber::editor_camera::EditorCamera>>) -> bool {
    cameras.is_empty()
}

/// Movimento do herói. A câmara corre depois deste conjunto.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct PlayerMovementSet;

pub struct VaelPlayerPlugin;

impl Plugin for VaelPlayerPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<VaelController>()
            .add_systems(PostStartup, attach_controllers)
            .add_systems(
                viber::scene_loader::SceneLoad,
                attach_controllers.after(viber::recipes::spawn::startup),
            )
            .add_systems(
                Update,
                player_movement
                    .in_set(PlayerMovementSet)
                    .run_if(no_editor_camera),
            )
            .add_systems(
                Update,
                world_border_clamp
                    .after(PlayerMovementSet)
                    .run_if(no_editor_camera),
            );
    }
}

fn low_pass_factor(dt: f32, tau: f32) -> f32 {
    1.0 - (-dt / tau.max(1e-4)).exp()
}

fn process_input(forward: f32, strafe: f32, yaw_deg: f32) -> Vec3 {
    let (sin, cos) = yaw_deg.to_radians().sin_cos();
    let dir = Vec3::new(
        strafe * cos - forward * sin,
        0.0,
        -strafe * sin - forward * cos,
    );
    dir.normalize_or_zero()
}

fn input_magnitude(forward: f32, strafe: f32) -> f32 {
    (forward * forward + strafe * strafe).sqrt().min(1.0)
}

fn movement_tau(grounded: bool, input_active: bool) -> f32 {
    if !grounded {
        AIR_TAU
    } else if input_active {
        ACCEL_TAU
    } else {
        DECEL_TAU
    }
}

fn integrate_horizontal(
    vel_x: f32,
    vel_z: f32,
    desired: Vec3,
    grounded: bool,
    input_active: bool,
    dt: f32,
) -> (f32, f32) {
    if grounded && !input_active {
        return (0.0, 0.0);
    }
    let a = low_pass_factor(dt, movement_tau(grounded, input_active));
    (
        vel_x + (desired.x - vel_x) * a,
        vel_z + (desired.z - vel_z) * a,
    )
}

fn jump_velocity(height: f32) -> f32 {
    (2.0 * GRAVITY * height.max(0.0)).sqrt()
}

fn can_perform_jump(
    now: f32,
    buffer_time: f32,
    last_grounded_time: f32,
    can_jump: bool,
    grounded: bool,
) -> bool {
    now - buffer_time <= JUMP_BUFFER
        && can_jump
        && (grounded || now - last_grounded_time <= COYOTE_TIME)
}

fn facing_rotation(dir: Vec3) -> Quat {
    Quat::from_rotation_y(dir.x.atan2(dir.z))
}

fn facing_slerp_factor(current: Quat, target: Quat, rotation_speed: f32, dt: f32) -> f32 {
    let angle = current.angle_between(target);
    if angle <= f32::EPSILON {
        1.0
    } else {
        (rotation_speed * dt / angle).clamp(0.0, 1.0)
    }
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn player_movement(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    menus: Option<Res<viber::menus::MenusOpen>>,
    typing: Option<Res<viber::ui::widgets::UiFocusedInput>>,
    actions: Option<Res<viber::input_map::ActionState>>,
    locomotion: Option<Res<viber::luau::ownership::ScriptSystemOwners>>,
    runtime: Option<Res<viber::terrain::runtime::TerrainRuntime>>,
    collision: Option<Res<viber::physics::TerrainCollisionStatus>>,
    interior: Option<Res<viber::worldsys::InteriorSceneConfig>>,
    interior_state: Option<Res<viber::worldsys::InteriorLighting>>,
    mut cameras: Query<&mut OrbitCamera>,
    mut players: Query<(
        Entity,
        &mut Transform,
        &mut VaelController,
        &mut viber::player::Player,
        Option<&mut KinematicCharacterController>,
        Option<&KinematicCharacterControllerOutput>,
        Option<&mut viber::player::TeleportSettle>,
        Option<&mut viber::physics_fx::Knockback>,
        Option<&viber::spawn_gate::SpawnGated>,
    )>,
    mut sfx: MessageWriter<viber::ambient::SfxEvent>,
) {
    let dt = time.delta_secs();
    let now = time.elapsed_secs();
    let runtime = runtime.as_deref();
    // Um script reclamou o corpo (`viber.own_system("locomotion")` +
    // `viber.set_pose`): o controller a pé não escreve por cima.
    if locomotion
        .as_ref()
        .is_some_and(|owners| owners.owns(viber::luau::ownership::SYSTEM_LOCOMOTION))
    {
        for (_, _, _, _, controller, _, _, _, _) in &mut players {
            if let Some(mut controller) = controller {
                controller.translation = Some(Vec3::ZERO);
            }
        }
        return;
    }
    // Modal aberto consome o teclado — e limpa o `translation` pendente do
    // character controller (persiste em bevy_rapier3d 0.36 e o herói desliza
    // com o menu aberto).
    let blocked = menus.as_ref().is_some_and(|menus| menus.any());
    if blocked {
        for (_, _, mut controller, mut engine_player, rapier, _, _, _, _) in &mut players {
            controller.vel_x = 0.0;
            controller.vel_z = 0.0;
            controller.vel_y = 0.0;
            if let Some(mut rapier) = rapier {
                rapier.translation = Some(Vec3::ZERO);
            }
            sync_engine_player(&mut engine_player, &controller);
        }
        return;
    }
    let typing = typing.as_ref().is_some_and(|focused| focused.0.is_some());
    let (move_x, move_forward, sprint, jump_pressed) = if typing {
        (0.0, 0.0, false, false)
    } else if let Some(actions) = actions.as_deref() {
        // Com ActionMap presente, os binds mandam (rebind de salto/corrida).
        (
            actions.move_x,
            actions.forward,
            actions.sprint,
            actions.jump_just,
        )
    } else {
        let (w, s, a, d) = (
            keys.pressed(KeyCode::KeyW) || keys.pressed(KeyCode::ArrowUp),
            keys.pressed(KeyCode::KeyS) || keys.pressed(KeyCode::ArrowDown),
            keys.pressed(KeyCode::KeyA) || keys.pressed(KeyCode::ArrowLeft),
            keys.pressed(KeyCode::KeyD) || keys.pressed(KeyCode::ArrowRight),
        );
        (
            (d as i32 - a as i32) as f32,
            (w as i32 - s as i32) as f32,
            keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight),
            keys.just_pressed(KeyCode::Space),
        )
    };

    for (
        entity,
        mut transform,
        mut controller,
        mut engine_player,
        rapier,
        output,
        mut settle,
        mut knockback,
        gated,
    ) in &mut players
    {
        if gated.is_some() {
            controller.vel_x = 0.0;
            controller.vel_z = 0.0;
            controller.vel_y = 0.0;
            if let Some(mut rapier) = rapier {
                rapier.translation = Some(Vec3::ZERO);
            }
            sync_engine_player(&mut engine_player, &controller);
            continue;
        }
        // Tutela pós-teleporte: enquanto dura, o chão ANALÍTICO manda mesmo
        // que já exista collider algures — a coluna de destino pode estar a
        // assar e uma queda a 55 m/s atravessa o trimesh antes disso.
        //
        // Settle FRESCO é a CHEGADA de um teleporte (viagem pelo mapa,
        // `viber.teleport_player`, load do save): a chegada tem de ser limpa,
        // sem a inércia nem o knockback do sítio antigo. Quem teleporta repõe
        // o `Player` da engine — agora um espelho que este sync reescreve —
        // por isso o reset do MOVIMENTO vive aqui, no dono.
        let fresh_settle = settle.is_some() && !controller.settle_seen;
        controller.settle_seen = settle.is_some();
        if fresh_settle {
            controller.vel_x = 0.0;
            controller.vel_z = 0.0;
            controller.vel_y = 0.0;
            controller.is_jumping = false;
            controller.jump_cooldown = 0.0;
            controller.can_jump = true;
            commands
                .entity(entity)
                .remove::<viber::physics_fx::Knockback>();
        }
        let settling = match settle.as_deref_mut() {
            Some(state) if state.frames > 0 => {
                state.frames -= 1;
                if state.frames == 0 {
                    commands
                        .entity(entity)
                        .remove::<viber::player::TeleportSettle>();
                }
                true
            }
            Some(_) => {
                commands
                    .entity(entity)
                    .remove::<viber::player::TeleportSettle>();
                false
            }
            None => false,
        };
        if controller.grounded {
            controller.last_grounded_time = now;
            controller.is_jumping = false;
        }
        if controller.jump_cooldown > 0.0 {
            controller.jump_cooldown = (controller.jump_cooldown - dt).max(0.0);
            if controller.jump_cooldown == 0.0 {
                controller.can_jump = true;
            }
        }

        // Numa bolsa de interior a câmara é FIXA: A/D andam nas 8 direções do
        // ecrã. Fora, A/D conduzem a câmara e o herói segue-a.
        let interior_mode = interior
            .as_deref()
            .is_some_and(|scene| scene.contains(transform.translation.x, transform.translation.z));
        let interior_floor = (interior_mode
            && interior_state.as_deref().is_some_and(|state| state.active))
        .then_some(viber::worldsys::INTERIOR_FLOOR_Y);
        let (dir, input_mag) = if interior_mode {
            let yaw = interior
                .as_deref()
                .map(|scene| scene.camera_yaw_deg)
                .unwrap_or(0.0);
            (
                process_input(move_forward, move_x, yaw),
                input_magnitude(move_forward, move_x),
            )
        } else {
            let mut camera_yaw_deg = 0.0f32;
            if let Some(mut camera) = cameras.iter_mut().next() {
                camera.yaw_deg -= move_x * CAMERA_TURN_SPEED.to_degrees() * dt;
                camera_yaw_deg = camera.yaw_deg;
            }
            if move_x != 0.0 {
                controller.last_steer_time = now;
            }
            let strafe = move_x * SIDE_MOVE_FACTOR;
            (
                process_input(move_forward, strafe, camera_yaw_deg),
                input_magnitude(move_forward, strafe),
            )
        };
        let sprint_mult = if sprint {
            controller.sprint_multiplier
        } else {
            1.0
        };
        let desired = dir * controller.speed * sprint_mult * input_mag;
        let input_active = dir.length_squared() > 0.0;
        let (vel_x, vel_z) = integrate_horizontal(
            controller.vel_x,
            controller.vel_z,
            desired,
            controller.grounded,
            input_active,
            dt,
        );
        controller.vel_x = vel_x;
        controller.vel_z = vel_z;
        if vel_x * vel_x + vel_z * vel_z > 0.25 {
            controller.last_moving_time = now;
        }
        let mut motion = Vec3::new(controller.vel_x, 0.0, controller.vel_z) * dt;
        // O empurrão de um golpe entra no MESMO pedido ao character
        // controller (escrito no Transform atravessava paredes e props).
        if let Some(knockback) = knockback.as_deref_mut() {
            motion +=
                knockback.velocity.with_y(0.0) * viber::physics_fx::knockback_travel_factor(dt);
            knockback.velocity *= viber::physics_fx::knockback_decay_factor(dt);
            if knockback.velocity.length() < viber::physics_fx::KNOCKBACK_EPSILON {
                knockback.velocity = Vec3::ZERO;
                commands
                    .entity(entity)
                    .remove::<viber::physics_fx::Knockback>();
            }
        }

        if dir.length_squared() > 0.0 {
            let target = facing_rotation(dir);
            let factor =
                facing_slerp_factor(transform.rotation, target, controller.rotation_speed, dt);
            transform.rotation = transform.rotation.slerp(target, factor);
        }

        if jump_pressed {
            controller.jump_buffer_time = now;
        }
        if !interior_mode
            && can_perform_jump(
                now,
                controller.jump_buffer_time,
                controller.last_grounded_time,
                controller.can_jump,
                controller.grounded,
            )
        {
            controller.vel_y = jump_velocity(controller.jump_height);
            controller.is_jumping = true;
            controller.can_jump = false;
            controller.jump_cooldown = JUMP_COOLDOWN;
            controller.jump_buffer_time = f32::NEG_INFINITY;
            sfx.write(viber::ambient::SfxEvent {
                clip: viber::ambient::SfxClip::Jump,
                position: Some(transform.translation),
            });
        }

        // Chão analítico: a superfície SÓLIDA SOB o herói (respeita grutas e
        // overhangs). É a rede de segurança enquanto a coluna voxel assa ou
        // troca de LOD; com collider carregado (`terrain_ready`) o collider é
        // a autoridade.
        let (px, mut py, pz) = (
            transform.translation.x,
            transform.translation.y,
            transform.translation.z,
        );
        let terrain_ready = collision
            .as_deref()
            .is_some_and(|status| status.is_ready(entity));
        let field = runtime.filter(|rt| rt.in_field(px, pz));
        let top = field
            .map(|rt| rt.sample(px, pz))
            .unwrap_or(f32::NEG_INFINITY);
        if transform.translation.y < top - VOID_RESCUE_DEPTH {
            warn!(
                "vael-player: herói caiu do mundo a y={:.1} (superfície {:.1}) — resgatado",
                transform.translation.y, top
            );
            transform.translation.y = top;
            py = top;
            controller.vel_y = 0.0;
            controller.grounded = true;
        }
        let ground = if let Some(rt) = field {
            match rt.surface_below(px, pz, py + GROUND_PROBE) {
                Some(surface) => surface,
                None if settling => rt.sample(px, pz),
                None => f32::NEG_INFINITY,
            }
        } else {
            f32::NEG_INFINITY
        };

        if !controller.grounded || controller.vel_y > 0.0 {
            controller.vel_y -= GRAVITY * dt;
        }
        controller.vel_y = controller.vel_y.max(-TERMINAL_VELOCITY);
        motion.y += controller.vel_y * dt;

        match rapier {
            Some(mut rapier) => {
                rapier.translation = Some(motion);
                let on_collider = output.is_some_and(|out| out.grounded);
                if on_collider {
                    controller.grounded = true;
                    if controller.vel_y < 0.0 {
                        controller.vel_y = 0.0;
                    }
                } else if interior_floor.is_some_and(|floor| transform.translation.y < floor) {
                    transform.translation.y = interior_floor.unwrap_or(0.0);
                    controller.vel_y = 0.0;
                    controller.grounded = true;
                    rapier.translation = Some(Vec3::new(motion.x, 0.0, motion.z));
                } else if (!terrain_ready || settling) && transform.translation.y <= ground {
                    transform.translation.y = ground;
                    controller.vel_y = 0.0;
                    controller.grounded = true;
                    // A queda DESTE frame ainda está na fila do CCT: sem
                    // isto o Rapier aplica `motion.y`, o herói fura o chão
                    // analítico e `surface_below` deixa de o ver.
                    rapier.translation = Some(Vec3::new(motion.x, 0.0, motion.z));
                } else {
                    controller.grounded = false;
                }
            }
            None => {
                transform.translation += motion;
                if transform.translation.y <= ground {
                    transform.translation.y = ground;
                    controller.vel_y = 0.0;
                    controller.grounded = true;
                } else {
                    controller.grounded = false;
                }
            }
        }

        let planar =
            (controller.vel_x * controller.vel_x + controller.vel_z * controller.vel_z).sqrt();
        let grounded = controller.grounded;
        if viber::player::footstep_due(&mut engine_player.step_timer, grounded, planar, dt)
            .is_some()
        {
            let clip = if runtime.is_some_and(|rt| rt.in_water(px, pz)) {
                viber::ambient::SfxClip::FootstepWater
            } else {
                viber::ambient::SfxClip::Footstep
            };
            sfx.write(viber::ambient::SfxEvent {
                clip,
                position: Some(transform.translation),
            });
        }
        sync_engine_player(&mut engine_player, &controller);
    }
}

/// `<WorldBorder>`: mantém o herói dentro do disco do mapa (fora das
/// bolsas de interior). Corre depois do movimento, no mesmo frame.
fn world_border_clamp(
    border: Option<Res<viber::worldsys::WorldBorderConfig>>,
    scene: Option<Res<viber::worldsys::InteriorSceneConfig>>,
    mut players: Query<&mut Transform, With<viber::player::Player>>,
    mut logged: Local<bool>,
) {
    let Some(border) = border else {
        return;
    };
    let limit = border.radius - border.margin;
    if limit <= 0.0 {
        return;
    }
    for mut transform in &mut players {
        let pos = transform.translation;
        if scene
            .as_deref()
            .is_some_and(|scene| scene.contains(pos.x, pos.z))
        {
            continue;
        }
        let dist_sq = pos.x * pos.x + pos.z * pos.z;
        if dist_sq > limit * limit {
            let scale = limit / dist_sq.sqrt();
            transform.translation.x = pos.x * scale;
            transform.translation.z = pos.z * scale;
            if !*logged {
                *logged = true;
                info!("world border: player returned inside r={limit}");
            }
        } else {
            *logged = false;
        }
    }
}

fn sync_engine_player(engine: &mut viber::player::Player, controller: &VaelController) {
    engine.speed = controller.speed;
    engine.sprint_multiplier = controller.sprint_multiplier;
    engine.jump_height = controller.jump_height;
    engine.rotation_speed = controller.rotation_speed;
    engine.vel_x = controller.vel_x;
    engine.vel_z = controller.vel_z;
    engine.vel_y = controller.vel_y;
    engine.grounded = controller.grounded;
    engine.can_jump = controller.can_jump;
    engine.is_jumping = controller.is_jumping;
    engine.jump_cooldown = controller.jump_cooldown;
    engine.last_grounded_time = controller.last_grounded_time;
    engine.jump_buffer_time = controller.jump_buffer_time;
    engine.last_steer_time = controller.last_steer_time;
    engine.last_moving_time = controller.last_moving_time;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grounded_without_input_stops_immediately() {
        let (x, z) = integrate_horizontal(3.0, -2.0, Vec3::X * 4.0, true, false, 0.016);
        assert_eq!((x, z), (0.0, 0.0));
    }

    #[test]
    fn grounded_input_ramps_toward_desired() {
        let (x, _) = integrate_horizontal(0.0, 0.0, Vec3::X * 4.0, true, true, 0.016);
        assert!(x > 0.0 && x < 4.0);
    }

    #[test]
    fn airborne_keeps_momentum_without_input() {
        let (x, z) = integrate_horizontal(3.0, -2.0, Vec3::ZERO, false, false, 0.016);
        assert!(x > 0.0 && z < 0.0);
    }

    #[test]
    fn jump_needs_buffer_and_ground_or_coyote() {
        assert!(can_perform_jump(1.05, 1.0, 1.0, true, false));
        assert!(!can_perform_jump(1.5, 1.0, 0.0, true, false));
        assert!(!can_perform_jump(1.05, 1.0, 0.0, false, true));
        assert!(can_perform_jump(1.05, 1.0, 1.0, true, false));
    }

    #[test]
    fn input_yaw_zero_walks_negative_z() {
        let dir = process_input(1.0, 0.0, 0.0);
        assert!((dir - Vec3::NEG_Z).length() < 1e-5);
    }
}

//! Câmara de terceira pessoa do Vael (Wave 3 da migração).
//!
//! Porte do `camera::third_person_camera` da engine: follow desacoplado com
//! low-pass, auto-câmara atrás do heading, colisão contra o terreno, câmara
//! fixa de sala nos interiores e shake/kick. Com `player_camera:false` os
//! sistemas da engine não correm; este módulo é o dono da vista.

use bevy::prelude::*;
use bevy_ext_core::{CameraSnapshot, LOOK_PIVOT_HEIGHT, Player, ScriptCameraFree};

use viber::camera::{
    AutoCameraInput, COLLISION_EYE_HEIGHT, CameraKick, CameraShake, INTERIOR_ROOM_LAG, SHAKE_DECAY,
    auto_camera_yaw, behind_yaw_deg, camera_offset, interior_camera_pose, kick_spring_step,
    low_pass_factor, noise_pm1, shake_offset_roll, shortest_angle_delta_deg, terrain_safe_distance,
};
use viber::recipes::spawn::{AutoOrbit, OrbitCamera};
use bevy_ext_core::InteriorSceneConfig;

use crate::player::PlayerMovementSet;

fn no_editor_camera(cameras: Query<(), With<viber::editor_camera::EditorCamera>>) -> bool {
    cameras.is_empty()
}

pub struct VaelCameraPlugin;

impl Plugin for VaelCameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<viber::camera::ScriptCameraFree>()
            .init_resource::<viber::camera::CameraSnapshot>();
        app.add_systems(
            Update,
            (orbit_camera_follow, auto_orbit)
                .before(PlayerMovementSet)
                .run_if(no_editor_camera),
        )
        // A terceira pessoa corre no `PostUpdate`, DEPOIS do writeback do
        // Rapier: o character controller só aplica o `translation` pedido no
        // `PostUpdate`, por isso no `Update` a câmara lia a pose do frame
        // ANTERIOR. O desvio câmara↔herói ficava `vel * dt` — com o dt a
        // variar de frame para frame, o herói tremia no ecrã ("engasgos").
        // Antes da propagação: o `GlobalTransform` da câmara sai deste frame.
        .add_systems(
            PostUpdate,
            (third_person_camera, script_camera)
                .chain()
                .after(bevy_rapier3d::prelude::PhysicsSet::Writeback)
                .before(bevy::transform::TransformSystems::Propagate)
                .run_if(no_editor_camera),
        );
    }
}

/// Follow rígido das câmaras com `target` (ou sem jogador): a pose é
/// `alvo + camera_offset`, sem low-pass. As câmaras de terceira pessoa
/// (sem alvo, com jogador) ficam para [`third_person_camera`].
#[allow(clippy::type_complexity)]
fn orbit_camera_follow(
    mut cameras: Query<(&mut Transform, &OrbitCamera)>,
    names: Query<(Entity, &Name)>,
    globals: Query<&GlobalTransform>,
    players: Query<&GlobalTransform, With<Player>>,
    runtime: Option<Res<viber::terrain::runtime::TerrainRuntime>>,
) {
    let has_player = players.iter().next().is_some();
    for (mut cam, settings) in &mut cameras {
        if settings.target.is_none() && has_player {
            continue; // third-person: a câmara do Vael trata dela
        }
        let target_pos = match &settings.target {
            Some(target_name) => names
                .iter()
                .find(|(_, name)| name.as_str() == target_name)
                .and_then(|(entity, _)| globals.get(entity).ok())
                .map(|global| global.translation())
                .unwrap_or(Vec3::ZERO),
            None => Vec3::ZERO,
        };
        let offset = camera_offset(
            settings.yaw_deg,
            settings.pitch_state_deg,
            settings.distance,
            0.0,
        );
        cam.translation = target_pos + offset;
        if let Some(runtime) = runtime.as_deref() {
            let min_y = runtime
                .surface_below(
                    cam.translation.x,
                    cam.translation.z,
                    cam.translation.y + 0.5,
                )
                .unwrap_or(f32::NEG_INFINITY)
                + 1.0;
            if cam.translation.y < min_y {
                cam.translation.y = min_y;
            }
        }
        cam.look_at(target_pos + Vec3::Y * settings.pivot_height(), Vec3::Y);
    }
}

/// Câmara de recurso de um mundo sem `<OrbitCamera>`: órbita lenta.
fn auto_orbit(time: Res<Time>, mut cameras: Query<(&mut Transform, &mut AutoOrbit)>) {
    for (mut transform, mut orbit) in &mut cameras {
        orbit.yaw += time.delta_secs() * 0.25;
        transform.translation = Vec3::new(
            orbit.yaw.cos() * orbit.radius,
            orbit.height,
            orbit.yaw.sin() * orbit.radius,
        );
        transform.look_at(Vec3::ZERO, Vec3::Y);
    }
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn third_person_camera(
    time: Res<Time>,
    real: Res<Time<Real>>,
    mut shake: ResMut<CameraShake>,
    mut kick: ResMut<CameraKick>,
    mut cameras: Query<(&mut Transform, &mut OrbitCamera), Without<Player>>,
    players: Query<(&Transform, Option<&ChildOf>, &Player), Without<OrbitCamera>>,
    parents: Query<&GlobalTransform>,
    runtime: Option<Res<viber::terrain::runtime::TerrainRuntime>>,
    interior: Option<Res<InteriorSceneConfig>>,
    a11y: Option<Res<viber::a11y::A11y>>,
) {
    let dt = time.delta_secs();
    let now = time.elapsed_secs();
    // O shake decora em tempo REAL: durante o hit-stop a câmara continua a
    // tremer. Decai ANTES do early-return para não congelar no respawn.
    shake.trauma = (shake.trauma - SHAKE_DECAY * real.delta_secs()).max(0.0);
    let shake_mul = a11y.as_deref().map(viber::a11y::shake_scale).unwrap_or(1.0);
    let (mut shake_off, shake_roll) = shake_offset_roll(shake.trauma, real.elapsed_secs());
    shake_off *= shake_mul;
    let shake_roll = shake_roll * shake_mul;
    let (kick_off, kick_vel) = kick_spring_step(kick.offset, kick.vel, dt);
    kick.offset = kick_off;
    kick.vel = kick_vel;
    let Some((local, parent, player)) = players.iter().next() else {
        return;
    };
    // Pose DESTE frame: o Rapier acabou de a escrever no `Transform` e a
    // propagação ainda não correu (o pai é estático; o global dele serve).
    let target = match parent.and_then(|parent| parents.get(parent.parent()).ok()) {
        Some(parent) => parent.mul_transform(*local),
        None => GlobalTransform::from(*local),
    };
    let target_pos = target.translation();
    // Bolsa de interior: JRPG de 16 bits — enquadra a SALA, não o herói.
    let interior_room = interior
        .as_deref()
        .filter(|scene| scene.contains(target_pos.x, target_pos.z));
    for (mut transform, mut cam) in &mut cameras {
        if cam.target.is_some() {
            continue; // câmara com alvo nomeado: follow rígido da engine
        }
        if let Some(scene) = interior_room {
            let (eye, look_at) = interior_camera_pose(scene, target_pos);
            // Primeira vista: salta; depois um ease curto entre salas (um
            // corte seco pisca com sombras/TAA).
            let a = if cam.initialized {
                low_pass_factor(dt, INTERIOR_ROOM_LAG)
            } else {
                1.0
            };
            let (pos, point) = (cam.current_pos, cam.follow_point);
            cam.current_pos = pos + (eye - pos) * a;
            cam.follow_point = point + (look_at - point) * a;
            cam.smooth_yaw_deg = scene.camera_yaw_deg;
            cam.yaw_deg = scene.camera_yaw_deg;
            cam.initialized = true;
            transform.translation = cam.current_pos + shake_off + kick.offset;
            transform.look_at(cam.follow_point, Vec3::Y);
            transform.rotate_local_z(shake_roll);
            continue;
        }
        // Auto-câmara: swing atrás do heading enquanto corre; deriva lenta
        // atrás das costas do herói quando para.
        let speed = (player.vel_x * player.vel_x + player.vel_z * player.vel_z).sqrt();
        let heading = if speed > 1e-3 {
            Some((player.vel_x / speed, player.vel_z / speed))
        } else {
            None
        };
        let (fx, fz) = cam.yaw_deg.to_radians().sin_cos();
        let alignment = heading.map_or(0.0, |(hx, hz)| hx * -fx + hz * -fz);
        // O modelo da pipeline olha +Z (não o -Z do `forward()`).
        let facing = target.rotation() * Vec3::Z;
        let behind_facing = behind_yaw_deg(facing.x, facing.z);
        let settle_allowed = player.last_moving_time.is_finite();
        cam.yaw_deg = auto_camera_yaw(
            cam.yaw_deg,
            AutoCameraInput {
                heading,
                speed,
                heading_alignment: alignment,
                behind_facing_yaw: behind_facing,
                settle_allowed,
                seconds_since_steer: now - player.last_steer_time,
            },
            dt,
        );
        if !cam.initialized {
            cam.follow_point = target_pos;
            cam.smooth_yaw_deg = cam.yaw_deg;
            cam.current_pos = target_pos
                + camera_offset(
                    cam.smooth_yaw_deg,
                    cam.pitch_state_deg,
                    cam.distance,
                    cam.height,
                );
            cam.initialized = true;
        }

        let axz = low_pass_factor(dt, cam.follow_lag);
        let ay = low_pass_factor(dt, cam.follow_lag * 1.8);
        cam.follow_point.x += (target_pos.x - cam.follow_point.x) * axz;
        cam.follow_point.z += (target_pos.z - cam.follow_point.z) * axz;
        cam.follow_point.y += (target_pos.y - cam.follow_point.y) * ay;

        let ayaw = low_pass_factor(dt, cam.turn_lag);
        cam.smooth_yaw_deg += shortest_angle_delta_deg(cam.smooth_yaw_deg, cam.yaw_deg) * ayaw;

        let desired = cam.follow_point
            + camera_offset(
                cam.smooth_yaw_deg,
                cam.pitch_state_deg,
                cam.distance,
                cam.height,
            );

        // Colisão aplicada ao destino ANTES do low-pass final: a suavização
        // persegue sempre um alvo seguro, nunca um bloqueado.
        let mut safe = desired;
        if cam.min_terrain_distance > 0.0
            && let Some(rt) = runtime.as_deref()
        {
            let eye = cam.follow_point + Vec3::Y * COLLISION_EYE_HEIGHT;
            let delta = desired - eye;
            let full_dist = delta.length();
            if full_dist > 0.01 {
                let dir = delta / full_dist;
                let radius = cam.min_terrain_distance.max(0.5);
                let hit = terrain_safe_distance(
                    // Chão SOB a sonda, não o telhado da coluna: debaixo de
                    // uma ponte o topo contava como colisão. Um ponto DENTRO
                    // de rocha conta como chão em `p.y` (a encosta sobe).
                    |p| {
                        if rt.is_solid(p) {
                            p.y
                        } else {
                            rt.surface_below(p.x, p.z, p.y).unwrap_or(f32::NEG_INFINITY)
                        }
                    },
                    eye,
                    dir,
                    full_dist,
                    radius,
                    cam.min_terrain_distance,
                );
                match hit {
                    Some(t) => {
                        let safe_dist = (t - radius).max(0.01);
                        safe = eye + dir * safe_dist;
                    }
                    None => {
                        let min_y = if rt.in_field(desired.x, desired.z) {
                            rt.surface_below(
                                desired.x,
                                desired.z,
                                desired.y + bevy_ext_core::GROUND_PROBE,
                            )
                            .unwrap_or(f32::NEG_INFINITY)
                        } else {
                            f32::NEG_INFINITY
                        } + cam.min_terrain_distance;
                        if desired.y < min_y {
                            safe.y = min_y;
                        }
                    }
                }
            }
        }

        let sf = low_pass_factor(dt, 0.05);
        let cur = cam.current_pos;
        cam.current_pos = cur + (safe - cur) * sf;
        // Respiração orgânica: ruído só na posição RENDERIZADA; o
        // `current_pos` fica limpo e o ruído nunca realimenta.
        let sway = Vec3::new(
            noise_pm1(now * 0.35, 1) * 0.05 + noise_pm1(now * 1.1, 7) * 0.02,
            noise_pm1(now * 0.3, 2) * 0.04,
            noise_pm1(now * 0.35, 3) * 0.05 + noise_pm1(now * 1.1, 8) * 0.02,
        );
        transform.translation = cam.current_pos + sway + shake_off + kick.offset;
        transform.look_at(
            cam.follow_point + Vec3::Y * LOOK_PIVOT_HEIGHT,
            Vec3::Y,
        );
        if shake_roll.abs() > 1e-5 {
            let view = transform.forward();
            transform.rotate_axis(view, shake_roll);
        }
    }
}

/// Aplica a pose do modo livre (`viber.camera_free`/`viber.camera_pose`) e
/// publica a pose viva para `viber.camera_state()`. Corre depois do rig:
/// no modo livre sobrescreve-o; fora dele só lê.
fn script_camera(
    free: Res<ScriptCameraFree>,
    mut snapshot: ResMut<CameraSnapshot>,
    mut cameras: Query<(&mut Transform, &OrbitCamera)>,
) {
    let mut recorded = false;
    for (mut transform, cam) in &mut cameras {
        if cam.target.is_some() {
            continue;
        }
        if free.active {
            transform.translation = free.pos;
            let dir = free.look - free.pos;
            if dir.length_squared() > 1e-8 {
                transform.look_at(free.look, Vec3::Y);
            }
        }
        if !recorded {
            let forward = transform.forward();
            snapshot.pos = transform.translation;
            snapshot.yaw_deg = forward.x.atan2(forward.z).to_degrees();
            snapshot.pitch_deg = forward.y.clamp(-1.0, 1.0).asin().to_degrees();
            recorded = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bevy::time::TimeUpdateStrategy;

    use super::*;

    fn test_camera(yaw: f32) -> OrbitCamera {
        OrbitCamera {
            target: None,
            distance: 5.0,
            height: 1.5,
            pitch_deg: Some(0.0),
            pitch_state_deg: 0.0,
            yaw_deg: yaw,
            mouse_sensitivity: 0.12,
            min_distance: 2.0,
            max_distance: 80.0,
            follow_lag: 0.18,
            turn_lag: 0.38,
            min_terrain_distance: 0.0,
            follow_point: Vec3::ZERO,
            smooth_yaw_deg: yaw,
            current_pos: Vec3::ZERO,
            initialized: false,
        }
    }

    fn camera_app(player_at: Vec3, camera: OrbitCamera) -> (App, Entity, Entity) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
                16,
            )))
            .init_resource::<CameraShake>()
            .init_resource::<CameraKick>()
            .add_systems(Update, third_person_camera);
        let player = app
            .world_mut()
            .spawn((
                Transform::from_translation(player_at),
                GlobalTransform::from(Transform::from_translation(player_at)),
                Player::default(),
            ))
            .id();
        let cam = app.world_mut().spawn((Transform::default(), camera)).id();
        (app, player, cam)
    }

    #[test]
    fn camera_snaps_behind_and_looks_at_the_pivot() {
        let (mut app, _player, cam) = camera_app(Vec3::new(0.0, 10.0, 0.0), test_camera(180.0));
        app.update();
        let transform = *app.world().get::<Transform>(cam).unwrap();
        // yaw 180 → câmara em -Z, height 1.5 acima do follow point.
        let expected = Vec3::new(0.0, 11.5, -5.0);
        assert!(
            (transform.translation - expected).length() < 0.2,
            "camera em {:?}, esperado ~{expected:?}",
            transform.translation
        );
        let look = transform.rotation * Vec3::NEG_Z;
        assert!(
            look.dot(Vec3::Z) > 0.9,
            "a câmara devia apontar para o pivô (+Z), look={look:?}"
        );
    }

    #[test]
    fn interior_camera_frames_the_room_not_the_hero() {
        let (mut app, _player, _cam) = camera_app(Vec3::new(3.0, 0.6, 4.0), test_camera(0.0));
        app.insert_resource(InteriorSceneConfig {
            min: [-10.0, -10.0],
            max: [10.0, 10.0],
            room_size: [0.0, 0.0],
            camera_distance: 12.0,
            camera_pitch_deg: 60.0,
            camera_yaw_deg: 45.0,
            room_origin: [0.0, 0.0],
        });
        app.update();
        let mut query = app.world_mut().query::<(&Transform, &OrbitCamera)>();
        let (transform, camera) = query.iter(app.world()).next().unwrap();
        assert!((camera.yaw_deg - 45.0).abs() < 1e-4);
        assert!((camera.follow_point - Vec3::new(0.0, 1.6, 0.0)).length() < 1e-4);
        // A câmara fica no anel da sala, longe do herói em (3, 0.6, 4).
        assert!(transform.translation.distance(Vec3::new(3.0, 0.6, 4.0)) > 8.0);
    }
}

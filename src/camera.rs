//! Câmara de terceira pessoa do Vael (Wave 3 da migração).
//!
//! Porte do `camera::third_person_camera` da engine: follow desacoplado com
//! low-pass, auto-câmara atrás do heading, colisão contra o terreno, câmara
//! de seguimento nos interiores e shake/kick. Com `player_camera:false` os
//! sistemas da engine não correm; este módulo é o dono da vista.

use bevy::prelude::*;
use bevy_ext_core::{LOOK_PIVOT_HEIGHT, Player};
// `ScriptCameraFree`/`CameraSnapshot` são os da engine: o `viber.camera_*` do
// Lua escreve neles. As cópias do `bevy_ext_core` ainda não são as mesmas.
use viber::camera::{CameraSnapshot, ScriptCameraFree};

use bevy_ext_core::InteriorSceneConfig;
use bevy_ext_engine::camera_glue::{
    AutoCameraInput, COLLISION_EYE_HEIGHT, CameraKick, CameraShake, INTERIOR_ROOM_LAG, SHAKE_DECAY,
    auto_camera_yaw, behind_yaw_deg, camera_offset, kick_spring_step, low_pass_factor, noise_pm1,
    shake_offset_roll, shortest_angle_delta_deg, terrain_safe_distance,
};
use viber::recipes::spawn::{AutoOrbit, OrbitCamera};

use crate::player::PlayerMovementSet;

/// Deslocamento lateral (m) da câmara com o ecrã de título aberto.
const TITLE_FRAMING_SHIFT: f32 = 1.9;

fn no_editor_camera(
    cameras: Query<(), With<bevy_ext_engine::editor_camera::EditorCamera>>,
) -> bool {
    cameras.is_empty()
}

/// Uma cinemática a tocar é dona da câmara: o `drive_camera` da engine
/// escreve o mesmo `OrbitCamera` e, sem esta guarda, quem ganhava era o
/// agendamento (os dois correm no `PostUpdate`, antes da propagação).
fn no_cinematic(playback: Option<Res<viber::cinematic::CinematicPlayback>>) -> bool {
    playback.is_none_or(|playback| playback.playing().is_none())
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct GameplayCameraSet;

pub struct VaelCameraPlugin;

impl Plugin for VaelCameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ScriptCameraFree>()
            .init_resource::<CameraSnapshot>()
            // A engine inicializa-o sempre; o `init` aqui é para apps mínimas
            // de teste que montem só esta parte.
            .init_resource::<viber::cinematic::CinematicPlayback>()
            .add_observer(mark_shadow_lod_origin);
        app.add_systems(
            Update,
            (orbit_camera_follow, auto_orbit)
                .before(PlayerMovementSet)
                .run_if(no_editor_camera)
                .run_if(no_cinematic),
        )
        // A terceira pessoa corre no `PostUpdate`: o Avian corre no
        // `FixedPostUpdate` (antes) e o writeback dele já escreveu o
        // `Transform` do corpo — a câmara lê a pose DESTE frame.
        // Antes da propagação: o `GlobalTransform` da câmara sai deste frame.
        .add_systems(
            PostUpdate,
            (third_person_camera, script_camera)
                .chain()
                .in_set(GameplayCameraSet)
                .before(bevy::transform::TransformSystems::Propagate)
                .run_if(no_editor_camera)
                .run_if(no_cinematic),
        );
    }
}

/// A câmara de jogo é a origem do LOD das sombras das luzes pontuais/spot.
/// Sem marca, o Bevy usa "a câmara que desenha para a janela" — no Play
/// embutido do Jackdaw nenhuma desenha (vão para a imagem do stream) e caía
/// na câmara de menor id (céu/sondas), com o LOD das sombras centrado no
/// sítio errado.
fn mark_shadow_lod_origin(add: On<Add<OrbitCamera>>, mut commands: Commands) {
    commands
        .entity(add.entity)
        .try_insert(bevy::camera::ShadowLodOrigin);
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
            Some(target_name) => {
                let Some((entity, _)) = names.iter().find(|(_, name)| name.as_str() == target_name)
                else {
                    // Alvo nomeado que não existe (gralha no `.bsn`, entidade
                    // ainda por spawnar): segurar a câmara onde está. Antes
                    // saltava para o ORIGEM do mundo, um corte brutal.
                    continue;
                };
                match globals.get(entity) {
                    Ok(global) => global.translation(),
                    Err(_) => continue,
                }
            }
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

/// Junto à parede entre o foco e o olho, elevar a vista em arco mantendo
/// o zoom. A distância é medida na direção autorada, inclusive numa diagonal.
fn interior_eye_offset(scene: &InteriorSceneConfig, focus: Vec3) -> Vec3 {
    let (dx, dz) = scene.camera_yaw_deg.to_radians().sin_cos();
    let clearance = |p: f32, dir: f32, axis: usize| {
        if scene.room_size[axis] > 0.0 || dir.abs() < 1e-4 {
            f32::INFINITY
        } else if dir > 0.0 {
            (scene.max[axis] - p) / dir
        } else {
            (scene.min[axis] - p) / dir
        }
    };
    let remaining = clearance(focus.x, dx, 0).min(clearance(focus.z, dz, 1));
    let t = ((1.05 - remaining / scene.camera_distance.max(0.1)) / 0.78).clamp(0.0, 1.0);
    let rise = t * t * (3.0 - 2.0 * t);
    let pitch = scene.camera_pitch_deg + (84.0 - scene.camera_pitch_deg).max(0.0) * rise;
    camera_offset(scene.camera_yaw_deg, pitch, scene.camera_distance, 0.0)
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn third_person_camera(
    time: Res<Time>,
    real: Res<Time<Real>>,
    mut shake: ResMut<CameraShake>,
    mut kick: ResMut<CameraKick>,
    mut cameras: Query<(&mut Transform, &mut OrbitCamera), Without<Player>>,
    players: Query<
        (
            &Transform,
            Option<&ChildOf>,
            &Player,
            Option<&crate::player::VaelController>,
            Has<bevy_ext_core::TeleportSettle>,
            Has<crate::boat::RidingBoat>,
        ),
        Without<OrbitCamera>,
    >,
    parents: Query<&GlobalTransform>,
    runtime: Option<Res<viber::terrain::runtime::TerrainRuntime>>,
    interior: Option<Res<InteriorSceneConfig>>,
    a11y: Option<Res<bevy_ext_core::a11y::A11y>>,
    modals: Option<Res<viber::ui::modal::UiModalsOpen>>,
    mut title_framing: Local<f32>,
    mut conversation: crate::speech::ConversationShot,
) {
    // Clamp como no `player_movement`: o primeiro frame depois do véu (o
    // `Time<Virtual>` despausa) traz um delta enorme, e a mola do pitch/do
    // low-pass estourava para NaN — a câmera colapsava para dentro do chão
    // no arranque (visto 2026-11-15).
    let dt = time.delta_secs().min(0.05);
    let now = time.elapsed_secs();
    // O shake decora em tempo REAL: durante o hit-stop a câmara continua a
    // tremer. Decai ANTES do early-return para não congelar no respawn.
    shake.trauma = (shake.trauma - SHAKE_DECAY * real.delta_secs()).max(0.0);
    // Um trauma NaN vindo de fora (`viber.shake`, BRP/editor) não pode
    // envenenar os offsets: `f32::max` já o limpa no decaimento, mas o
    // valor lido neste frame ainda seria NaN.
    if !shake.trauma.is_finite() {
        shake.trauma = 0.0;
    }
    // A mola do kick NÃO se auto-cura (é integração linear): um impulso
    // degenerado (`viber.camera_kick(inf)`, escrita BRP) deixava o offset
    // NaN para sempre — a câmara desaparecia. Repor é o único caminho.
    if !kick.offset.is_finite() || !kick.vel.is_finite() {
        *kick = CameraKick::default();
    }
    let shake_mul = a11y
        .as_deref()
        .map(bevy_ext_core::a11y::shake_scale)
        .unwrap_or(1.0);
    let (mut shake_off, shake_roll) = shake_offset_roll(shake.trauma, real.elapsed_secs());
    shake_off *= shake_mul;
    let shake_roll = shake_roll * shake_mul;
    let (kick_off, kick_vel) = kick_spring_step(kick.offset, kick.vel, dt);
    kick.offset = kick_off;
    kick.vel = kick_vel;
    let Some((local, parent, player, controller, teleporting, rowing)) = players.iter().next() else {
        return;
    };
    // Pose DESTE frame: o writeback do Avian acabou de a escrever no `Transform` e a
    // propagação ainda não correu (o pai é estático; o global dele serve).
    let target = match parent.and_then(|parent| parents.get(parent.parent()).ok()) {
        Some(parent) => parent.mul_transform(*local),
        None => GlobalTransform::from(*local),
    };
    let target_pos = target.translation();
    let interior_room = interior
        .as_deref()
        .filter(|scene| scene.contains(target_pos.x, target_pos.z));
    for (mut transform, mut cam) in &mut cameras {
        if cam.target.is_some() {
            continue; // câmara com alvo nomeado: follow rígido da engine
        }
        // Rede de segurança contra NaN: uma pose degenerada (colisão com
        // terreno NaN, `OrbitCamera` autorado com lixo) ficava para sempre —
        // o low-pass propaga o NaN e a câmara nunca mais desenhava. Re-inicia
        // a partir do herói; o `initialized = false` faz o snap do bloco
        // seguinte.
        if !cam.current_pos.is_finite()
            || !cam.follow_point.is_finite()
            || !cam.smooth_yaw_deg.is_finite()
            || !cam.yaw_deg.is_finite()
        {
            cam.yaw_deg = if cam.yaw_deg.is_finite() {
                cam.yaw_deg
            } else {
                0.0
            };
            cam.smooth_yaw_deg = if cam.smooth_yaw_deg.is_finite() {
                cam.smooth_yaw_deg
            } else {
                cam.yaw_deg
            };
            cam.current_pos = target_pos;
            cam.follow_point = target_pos;
            cam.initialized = false;
        }
        if let Some(scene) = interior_room {
            let [cx, cz] = scene.room_center(target_pos.x, target_pos.z);
            // As grelhas autoradas continuam a mudar de sala por eixo.
            // Nas divisões abertas, travar a panorâmica antes da parede para
            // manter o espaço livre à frente da personagem no enquadramento.
            let (dx, dz) = scene.camera_yaw_deg.to_radians().sin_cos();
            let constrain = |value: f32, axis: usize, center: f32, eye_dir: f32| {
                if scene.room_size[axis] > 0.0 {
                    return center;
                }
                // A vista inclinada alcança mais longe no lado oposto ao olho.
                // Reservar essa área traz a porta e o piso para o mesmo plano.
                let inset = |side: f32| scene.camera_distance * (0.72 - 0.29 * side * eye_dir);
                let min = (scene.min[axis] + inset(-1.0)).min(center);
                let max = (scene.max[axis] - inset(1.0)).max(center);
                value.clamp(min, max)
            };
            let focus = Vec3::new(
                constrain(target_pos.x, 0, cx, dx),
                target_pos.y + 0.85,
                constrain(target_pos.z, 1, cz, dz),
            );
            if !cam.initialized
                || teleporting && cam.follow_point.distance(focus) > scene.camera_distance
            {
                cam.follow_point = focus;
            } else {
                // Só o excesso fora da zona morta move a câmara. Assim a
                // respiração do corpo e os pequenos ajustes de direção não
                // se transformam numa panorâmica.
                let delta = focus - cam.follow_point;
                let slack = Vec3::new(0.35, 0.10, 0.28);
                cam.follow_point +=
                    (delta - delta.clamp(-slack, slack)) * low_pass_factor(dt, INTERIOR_ROOM_LAG);
            }
            cam.current_pos = cam.follow_point + interior_eye_offset(scene, cam.follow_point);
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
        // The rower faces the stern. Keep the follow camera looking toward the bow.
        let facing = target.rotation() * if rowing { Vec3::NEG_Z } else { Vec3::Z };
        let behind_facing = behind_yaw_deg(facing.x, facing.z);
        let settle_allowed = player.last_moving_time.is_finite();
        cam.yaw_deg = if let Some(face) = controller.and_then(|c| c.combat_facing) {
            behind_yaw_deg(face.x, face.z)
        } else {
            auto_camera_yaw(
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
            )
        };
        // `TeleportSettle` fresco (portão de spawn, viagem, respawn): o
        // herói SALTou — a câmara salta com ele. Com o low-pass, a chegada
        // ao chão depois do portão mostrava a câmara a descer por cima do
        // mapa a olhar para o chão durante um segundo.
        if !cam.initialized || teleporting {
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

        // O ponto de mira divide a conversa entre os dois corpos. Afastar e
        // deslocar o olho para o ombro separa as silhuetas num encontro frontal.
        let crown = conversation
            .subject(&parents)
            .filter(|_| !teleporting && !rowing && !modals.as_deref().is_some_and(|m| m.any()));
        let blend = conversation.blend(crown.is_some(), dt);
        let follow = crown.map_or(target_pos, |head| {
            target_pos + (head - target_pos - Vec3::Y * LOOK_PIVOT_HEIGHT) * 0.55 + Vec3::Y * 0.20
        });
        let axz = low_pass_factor(dt, cam.follow_lag);
        let ay = low_pass_factor(dt, cam.follow_lag * 1.8);
        cam.follow_point.x += (follow.x - cam.follow_point.x) * axz;
        cam.follow_point.z += (follow.z - cam.follow_point.z) * axz;
        cam.follow_point.y += (follow.y - cam.follow_point.y) * ay;

        let ayaw = low_pass_factor(dt, cam.turn_lag);
        cam.smooth_yaw_deg += shortest_angle_delta_deg(cam.smooth_yaw_deg, cam.yaw_deg) * ayaw;

        let shoulder = Quat::from_rotation_y(cam.smooth_yaw_deg.to_radians()) * Vec3::X;
        let desired = cam.follow_point
            + shoulder * (0.95 * blend)
            + camera_offset(
                cam.smooth_yaw_deg,
                cam.pitch_state_deg,
                cam.distance + 0.85 * blend,
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
        cam.current_pos = if teleporting {
            safe
        } else {
            cur + (safe - cur) * sf
        };
        // Respiração orgânica: ruído só na posição RENDERIZADA; o
        // `current_pos` fica limpo e o ruído nunca realimenta.
        let sway = Vec3::new(
            noise_pm1(now * 0.35, 1) * 0.05 + noise_pm1(now * 1.1, 7) * 0.02,
            noise_pm1(now * 0.3, 2) * 0.04,
            noise_pm1(now * 0.35, 3) * 0.05 + noise_pm1(now * 1.1, 8) * 0.02,
        );
        transform.translation = cam.current_pos + sway + shake_off + kick.offset;
        // Seated chest height keeps both grips and the waterline in view.
        let look_height = if rowing { 0.90 } else { LOOK_PIVOT_HEIGHT };
        transform.look_at(cam.follow_point + Vec3::Y * look_height, Vec3::Y);
        // Ecrã de título: o painel cobre a metade esquerda; a câmara desliza
        // para a esquerda (mesma orientação) e o herói fica no terço direito
        // em vez de cortado pela aresta do painel. Sai suave ao fechar.
        let title_open = modals.as_deref().is_some_and(|m| m.is_open("title"));
        let target = if title_open { 1.0 } else { 0.0 };
        *title_framing += (target - *title_framing) * low_pass_factor(dt.max(1.0 / 60.0), 0.35);
        if *title_framing > 1e-3 {
            let left = transform.left();
            transform.translation += left * (TITLE_FRAMING_SHIFT * *title_framing);
        }
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
        if free.active && free.pos.is_finite() && free.look.is_finite() {
            // Uma pose não finita (`viber.camera_free`/`camera_pose` com lixo
            // vindo do Lua) apagava a câmara de vez: a pose é copiada tal e
            // qual para o `Transform` e o snapshot seguinte lia NaN.
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
    fn lock_frames_the_enemy_while_the_hero_strafes() {
        let (mut app, hero, cam) = camera_app(Vec3::ZERO, test_camera(0.0));
        app.update();
        app.world_mut().entity_mut(hero).insert(crate::player::VaelController {
            combat_facing: Some(Vec3::X),
            ..default()
        });
        app.world_mut().get_mut::<Player>(hero).unwrap().vel_z = 4.0;
        for _ in 0..120 {
            app.update();
        }
        let look = app.world().get::<Transform>(cam).unwrap().rotation * Vec3::NEG_Z;
        assert!(look.dot(Vec3::X) > 0.9);
    }

    #[test]
    fn conversation_frames_the_focused_pair_and_releases_the_orbit() {
        use viber::interact::InteractionFocus;
        use viber::luau::{ScriptSay, ScriptToast};

        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::asset::AssetPlugin::default(),
            bevy::transform::TransformPlugin,
        ))
        .init_asset::<Mesh>()
        .init_asset::<Image>()
        .init_asset::<StandardMaterial>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<CameraShake>()
        .init_resource::<CameraKick>()
        .init_resource::<InteractionFocus>()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
            1. / 60.,
        )))
        .add_message::<ScriptSay>()
        .add_message::<ScriptToast>()
        .add_plugins((
            VaelCameraPlugin,
            crate::speech::VaelSpeechPlugin::<OrbitCamera>::default(),
        ));
        app.world_mut()
            .spawn((Transform::IDENTITY, Player::default()));
        let speaker = app
            .world_mut()
            .spawn((Transform::from_xyz(0., 0., -1.3), Visibility::Inherited))
            .id();
        let camera = app
            .world_mut()
            .spawn((Transform::IDENTITY, test_camera(0.)))
            .id();
        app.update();
        let rest = app.world().get::<OrbitCamera>(camera).unwrap().current_pos;
        app.world_mut().write_message(ScriptSay {
            speaker: Some(speaker),
            text: "Olá, viajante.".into(),
            seconds: 8.,
        });
        for _ in 0..30 {
            app.update();
        }
        assert!(
            app.world()
                .get::<OrbitCamera>(camera)
                .unwrap()
                .current_pos
                .distance(rest)
                < 0.01
        );
        app.world_mut()
            .resource_mut::<InteractionFocus>()
            .consider(KeyCode::KeyE, speaker, 1.3);
        app.update();
        assert!(
            app.world()
                .get::<OrbitCamera>(camera)
                .unwrap()
                .current_pos
                .distance(rest)
                < 0.2
        );
        for _ in 0..90 {
            app.update();
        }
        let orbit = app.world().get::<OrbitCamera>(camera).unwrap();
        assert!(orbit.current_pos.x > 0.8 && orbit.follow_point.z < -0.6);
        assert_eq!(orbit.distance, 5.);
        app.world_mut().resource_mut::<InteractionFocus>().clear();
        for _ in 0..150 {
            app.update();
        }
        assert!(
            app.world()
                .get::<OrbitCamera>(camera)
                .unwrap()
                .current_pos
                .distance(rest)
                < 0.01
        );
    }

    #[test]
    fn interior_follow_is_quiet_at_rest_and_keeps_the_hero_near_the_wall_in_view() {
        let (mut app, player, cam) = camera_app(Vec3::ZERO, test_camera(0.0));
        app.insert_resource(InteriorSceneConfig {
            min: [-10.0, -8.0],
            max: [10.0, 8.0],
            camera_distance: 6.0,
            camera_pitch_deg: 50.0,
            ..default()
        });
        app.update();
        let first = app.world().get::<OrbitCamera>(cam).unwrap().follow_point;
        app.world_mut()
            .get_mut::<Transform>(player)
            .unwrap()
            .translation = Vec3::new(0.12, 0.04, -0.1);
        app.update();
        assert_eq!(
            app.world().get::<OrbitCamera>(cam).unwrap().follow_point,
            first
        );
        app.world_mut()
            .get_mut::<Transform>(player)
            .unwrap()
            .translation
            .x = 9.0;
        app.update();
        let first_step = app.world().get::<OrbitCamera>(cam).unwrap().follow_point.x;
        assert!((0.0..1.0).contains(&first_step) && first_step > 0.0);
        for _ in 0..100 {
            app.update();
        }
        let orbit = app.world().get::<OrbitCamera>(cam).unwrap();
        assert!((5.0..5.7).contains(&orbit.follow_point.x));
        assert!((orbit.current_pos.distance(orbit.follow_point) - 6.0).abs() < 1e-4);
    }

    #[test]
    fn interior_eye_clears_the_foreground_wall_without_changing_zoom() {
        let mut scene = InteriorSceneConfig {
            min: [-9.8; 2],
            max: [9.8; 2],
            camera_distance: 6.2,
            camera_pitch_deg: 52.0,
            ..default()
        };
        for yaw in [0.0_f32, 90.0, 180.0, 270.0] {
            scene.camera_yaw_deg = yaw;
            let direction = Vec3::new(yaw.to_radians().sin(), 0., yaw.to_radians().cos());
            let focus = direction * 7.0 + Vec3::Y;
            let offset = interior_eye_offset(&scene, focus);
            // Mesmo quando o olho passa a parede, o segmento pés→olho tem
            // de passar por cima do reboco (face interna a 7.86, topo a 2.4).
            let feet = direction * 7.6 + Vec3::Y * 0.16;
            let to_eye = focus + offset - feet;
            let wall_fraction = (7.86 - 7.6) / to_eye.dot(direction);
            assert!(wall_fraction >= 1.0 || feet.y + wall_fraction * to_eye.y > 2.4);
            assert!((offset.length() - scene.camera_distance).abs() < 1e-4);
            assert!(offset.y > interior_eye_offset(&scene, Vec3::ZERO).y);
        }
    }

    /// Uma pose/kick degenerado não pode apagar a câmara para sempre: o
    /// low-pass propaga NaN e a mola do kick não se auto-cura.
    #[test]
    fn camera_recupera_de_nan_em_vez_de_desaparecer() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
                16,
            )))
            .init_resource::<CameraShake>()
            .insert_resource(CameraKick {
                offset: Vec3::NAN,
                vel: Vec3::NAN,
            })
            .add_systems(Update, third_person_camera);
        app.world_mut().spawn((
            Transform::from_xyz(0.0, 10.0, 0.0),
            GlobalTransform::from_xyz(0.0, 10.0, 0.0),
            Player::default(),
        ));
        let mut camera = test_camera(180.0);
        camera.current_pos = Vec3::NAN;
        camera.follow_point = Vec3::NAN;
        camera.smooth_yaw_deg = f32::NAN;
        let cam = app.world_mut().spawn((Transform::default(), camera)).id();
        app.update();
        let transform = *app.world().get::<Transform>(cam).unwrap();
        assert!(
            transform.translation.is_finite(),
            "câmara NaN para sempre: {:?}",
            transform.translation
        );
        assert!(
            transform.translation.distance(Vec3::new(0.0, 11.5, -5.0)) < 0.5,
            "volta ao follow normal, em {:?}",
            transform.translation
        );
        let kick = app.world().resource::<CameraKick>();
        assert!(kick.offset.is_finite() && kick.vel.is_finite());
    }
}

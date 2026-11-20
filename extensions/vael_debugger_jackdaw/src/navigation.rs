//! Navigation in the authored viewport. Only the player's world position crosses
//! the bridge. Panning, orbiting and following never move the game camera.
use std::time::{Duration, Instant};

use bevy::picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings};
use bevy::prelude::*;
use bevy::tasks::{IoTaskPool, Task, futures_lite::future};
use bevy_ext_bridge_protocol::{BridgeClient, is_listening};
use jackdaw_api::prelude::ActiveModalQuery;
use jackdaw_extension::AppState;
use jackdaw_extension::op::warn_caller;
use jackdaw_extension::prelude::*;
use jackdaw_extension::viewport::{ActiveViewport, MainViewportCamera, ViewportCursor};
use serde_json::json;

use super::port;

#[derive(Component)]
pub struct LiveStatus;

#[derive(Resource, Default)]
pub(crate) struct LiveView {
    camera: Option<Entity>,
    following: bool,
    previous: Option<Vec3>,
    request: Option<Task<Result<Vec3, String>>>,
    last_request: Option<Instant>,
}

pub fn install(app: &mut App) {
    app.init_resource::<LiveView>().add_systems(
        Update,
        (navigate, update_live_view).run_if(in_state(AppState::Editor)),
    );
}

struct Drag {
    camera: Entity,
    start: Transform,
    pointer: Vec2,
    pivot: Vec3,
    orbit: bool,
}

fn navigate(
    cursor: ViewportCursor,
    modal: ActiveModalQuery,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut drag: Local<Option<Drag>>,
    mut meshes: MeshRayCast,
    mut cameras: Query<(&mut Transform, &Projection), With<MainViewportCamera>>,
) {
    if !mouse.pressed(MouseButton::Middle)
        || mouse.pressed(MouseButton::Right)
        || modal.is_modal_running()
    {
        *drag = None;
        return;
    }
    let Some(pointer) = cursor.viewport_pointer() else {
        *drag = None;
        return;
    };
    if mouse.just_pressed(MouseButton::Middle)
        && let (Some(camera), Some(ray)) = (cursor.camera_entity(), cursor.ray())
        && let Ok((transform, projection)) = cameras.get(camera)
    {
        let orbit = keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]);
        if orbit && !matches!(projection, Projection::Perspective(_)) {
            return;
        }
        let distance = meshes
            .cast_ray(ray, &MeshRayCastSettings::default())
            .first()
            .map(|(_, hit)| hit.distance)
            .unwrap_or_else(|| transform.translation.y.abs().max(5.0));
        *drag = Some(Drag {
            camera,
            start: *transform,
            pointer,
            pivot: ray.get_point(distance),
            orbit,
        });
    }
    let Some(start) = drag.as_ref() else { return };
    if cursor.camera_entity() != Some(start.camera) {
        *drag = None;
        return;
    }
    let Ok((mut camera, _)) = cameras.get_mut(start.camera) else {
        return;
    };
    let delta = pointer - start.pointer;
    if start.orbit {
        let (yaw, pitch, _) = start.start.rotation.to_euler(EulerRot::YXZ);
        let rotation = Quat::from_euler(
            EulerRot::YXZ,
            yaw - delta.x * 0.005,
            (pitch - delta.y * 0.005).clamp(-1.55, 1.55),
            0.0,
        );
        camera.translation = start.pivot
            + rotation * start.start.rotation.inverse() * (start.start.translation - start.pivot);
        camera.rotation = rotation;
    } else if let Some(ray) = cursor.ray() {
        // The grabbed point stays under the pointer, including in orthographic views.
        let plane = InfinitePlane3d::new(*start.start.forward());
        if let Some(t) = ray.intersect_plane(start.pivot, plane)
            && let Some((_, global)) = cursor.camera()
        {
            camera.translation = global.translation() + start.pivot - ray.get_point(t);
        }
    }
}

fn player_position(port: u16) -> Result<Vec3, String> {
    if !is_listening(port) {
        return Err(format!("À espera do Play na porta {port}"));
    }
    let ty = GlobalTransform::type_path();
    let rows = BridgeClient::localhost(port)
        .call("world.query", json!({"data": {"components": [ty]}, "filter": {"with": ["viber::player::Player"]}, "strict": true}))
        .map_err(|error| error.to_string())?;
    let Some([row]) = rows.as_array().map(Vec::as_slice) else {
        return Err("À espera de um jogador ativo".into());
    };
    let transform: GlobalTransform = serde_json::from_value(row["components"][ty].clone())
        .map_err(|error| format!("Posição do jogador indisponível: {error}"))?;
    let position = transform.translation();
    position
        .is_finite()
        .then_some(position)
        .ok_or_else(|| "Posição inválida".into())
}

fn start_focus(world: &mut World, follow: bool) -> OperatorResult {
    let camera = world.resource::<ActiveViewport>().camera.or_else(|| {
        world
            .query_filtered::<Entity, With<MainViewportCamera>>()
            .single(world)
            .ok()
    });
    let Some(camera) = camera else {
        warn_caller(world, "Escolha a vista 3D onde quer localizar o jogador");
        return OperatorResult::Cancelled;
    };
    let mut live = world.resource_mut::<LiveView>();
    live.camera = Some(camera);
    live.following = follow;
    live.previous = None;
    live.last_request = None;
    OperatorResult::Finished
}

#[operator(
    id = "vael.debug.focus_player",
    label = "Vael: localizar jogador",
    allows_undo = false
)]
pub(crate) fn focus_player(_params: In<OperatorParameters>, world: &mut World) -> OperatorResult {
    start_focus(world, false)
}

#[operator(
    id = "vael.debug.follow_player",
    label = "Vael: seguir jogador",
    allows_undo = false
)]
pub(crate) fn follow_player(_params: In<OperatorParameters>, world: &mut World) -> OperatorResult {
    if world.resource::<LiveView>().following {
        let mut live = world.resource_mut::<LiveView>();
        live.following = false;
        live.camera = None;
        return OperatorResult::Finished;
    }
    start_focus(world, true)
}

fn update_live_view(
    mut live: ResMut<LiveView>,
    mut status: Query<&mut Text, With<LiveStatus>>,
    mut gizmos: Gizmos,
    mut cameras: Query<(&mut Transform, &Projection), With<MainViewportCamera>>,
) {
    if let Some(result) = live
        .request
        .as_mut()
        .and_then(|task| future::block_on(future::poll_once(task)))
    {
        live.request = None;
        let message = match result {
            Ok(position) => {
                if let Some(entity) = live.camera
                    && let Ok((mut view, projection)) = cameras.get_mut(entity)
                {
                    if let Some(previous) = live.previous {
                        view.translation += position - previous;
                    } else {
                        // Preserve the current angle and an axis-aligned orthographic view.
                        let forward = view.forward().as_vec3();
                        let distance = if matches!(projection, Projection::Orthographic(_)) {
                            30.0
                        } else {
                            12.0
                        };
                        view.translation = position + Vec3::Y - forward * distance;
                    }
                }
                live.previous = Some(position);
                let mode = if live.following {
                    "A seguir"
                } else {
                    "Jogador"
                };
                format!(
                    "{mode}   X {:.1}   Y {:.1}   Z {:.1}",
                    position.x, position.y, position.z
                )
            }
            Err(reason) => {
                live.previous = None;
                live.following = false;
                reason
            }
        };
        if !live.following {
            live.camera = None;
        }
        for mut text in &mut status {
            if text.0 != message {
                text.0.clone_from(&message);
            }
        }
    }
    if status.is_empty() && live.camera.is_none() {
        return;
    }
    if let Some(feet) = live.previous {
        let color = Color::srgb(0.2, 0.95, 0.8);
        gizmos.circle(
            Isometry3d::new(feet, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
            0.65,
            color,
        );
        gizmos.line(feet, feet + Vec3::Y * 1.8, color);
    }
    let interval = if live.following {
        Duration::from_millis(120)
    } else {
        Duration::from_secs(1)
    };
    if live.request.is_none()
        && live
            .last_request
            .is_none_or(|last| last.elapsed() >= interval)
    {
        live.last_request = Some(Instant::now());
        let port = port();
        live.request = Some(IoTaskPool::get().spawn(async move { player_position(port) }));
    }
}

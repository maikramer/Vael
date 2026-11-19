//! Ancoragem na silhueta e composição final com a câmara deste frame.
use super::{Bubble, BubblePx, SpeechFace};
use bevy::camera::primitives::Aabb;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::transform::helper::TransformHelper;
use bevy_ext_core::a11y::{A11y, subtitle_px};
use bevy_ext_engine::camera_glue::low_pass_factor;
use bevy_speech_bubbles::BalloonScale;
use viber::hud::waypoint::WaypointArrowRoot;
use viber::interact::InteractionFocus;

#[derive(SystemParam)]
pub(super) struct SpeakerGeometry<'w, 's> {
    children: Query<'w, 's, &'static Children>,
    globals: Query<'w, 's, &'static GlobalTransform>,
    meshes: Query<
        'w,
        's,
        (
            &'static Aabb,
            &'static GlobalTransform,
            Option<&'static InheritedVisibility>,
        ),
        With<Mesh3d>,
    >,
}

impl SpeakerGeometry<'_, '_> {
    /// Medir uma vez por fala, no referencial do dono. Inclui a translação e
    /// a escala das malhas filhas (inclusive prefabs), sem contar LODs ocultos.
    pub(super) fn crown(&self, owner: Entity) -> Option<Vec3> {
        let inverse = self.globals.get(owner).ok()?.affine().inverse();
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = -min;
        for part in std::iter::once(owner).chain(self.children.iter_descendants(owner)) {
            let Ok((bounds, global, visible)) = self.meshes.get(part) else {
                continue;
            };
            if visible.is_some_and(|v| !v.get()) {
                continue;
            }
            let local = inverse * global.affine();
            let center = local.transform_point3a(bounds.center);
            let half = local.matrix3.abs() * bounds.half_extents;
            min = min.min((center - half).into());
            max = max.max((center + half).into());
        }
        let middle = min.midpoint(max);
        let crown = Vec3::new(middle.x, max.y, middle.z);
        crown.is_finite().then_some(crown)
    }
}

#[derive(SystemParam)]
pub(crate) struct ConversationShot<'w, 's> {
    focus: Option<Res<'w, InteractionFocus>>,
    faces: Query<'w, 's, (&'static Bubble, &'static SpeechFace, &'static Visibility)>,
    weight: Local<'s, f32>,
}

impl ConversationShot<'_, '_> {
    pub(crate) fn subject(&self, globals: &Query<&GlobalTransform>) -> Option<Vec3> {
        let speaker = self.focus.as_ref()?.winner(KeyCode::KeyE)?;
        let (_, face, _) = self
            .faces
            .iter()
            .find(|(b, _, v)| b.owner == speaker && **v != Visibility::Hidden)?;
        Some(globals.get(speaker).ok()?.transform_point(face.crown))
    }

    pub(crate) fn blend(&mut self, active: bool, dt: f32) -> f32 {
        let target = if active { 1.0 } else { 0.0 };
        *self.weight += (target - *self.weight) * low_pass_factor(dt, 0.32);
        *self.weight
    }
}

/// Preferir acima da cabeça. Se faltar altura, abrir ao lado do rosto e só
/// em último caso por baixo. Todas as medidas estão em píxeis lógicos.
fn safe_rect(view: Rect, tip: Vec2, natural: Vec2, gap: f32) -> Rect {
    let margin = (view.height() * 0.02)
        .clamp(4.0, 20.0)
        .min(view.size().min_element() * 0.2);
    let safe = view.inflate(-margin);
    let size = natural * (safe.size() / natural).min_element().min(1.0);
    let above = tip - Vec2::new(size.x * 0.5, size.y + gap);
    let origin = if above.y >= safe.min.y {
        above
    } else if tip.x + gap + size.x <= safe.max.x {
        tip + Vec2::new(gap, -size.y * 0.5)
    } else if tip.x - gap - size.x >= safe.min.x {
        tip - Vec2::new(gap + size.x, size.y * 0.5)
    } else {
        tip + Vec2::new(-size.x * 0.5, gap)
    };
    let origin = origin.clamp(safe.min, (safe.max - size).max(safe.min));
    Rect::from_corners(origin, origin + size)
}

/// A pequena ponta de ouro liga a borda mais próxima à silhueta, inclusive
/// quando o corpo do balão desliza lateralmente para caber no ecrã.
fn pointer_pose(card: Rect, tip: Vec2, gap: f32) -> Option<Transform> {
    let border = tip.clamp(card.min, card.max);
    let inset = (gap * 0.8).min(card.size().min_element() * 0.25);
    let base = if tip.y < card.min.y || tip.y > card.max.y {
        Vec2::new(
            border.x.clamp(card.min.x + inset, card.max.x - inset),
            border.y,
        )
    } else {
        Vec2::new(
            border.x,
            border.y.clamp(card.min.y + inset, card.max.y - inset),
        )
    };
    // A textura tem uma pequena margem transparente; entrar um pouco no cartão.
    let base = base + (card.center() - base).normalize_or_zero() * (gap * 0.5);
    let to_local = Vec2::new(1.0, -1.0) / card.size();
    let axis = (tip - base) * to_local;
    if card.contains(tip) || axis.length_squared() < 1e-10 {
        return None;
    }
    let tangent = axis.normalize().perp();
    let width = gap / (tangent * card.size()).length();
    Some(Transform {
        translation: ((base - card.center()) * to_local).extend(-0.001),
        rotation: Quat::from_rotation_z(axis.x.atan2(-axis.y)),
        scale: Vec3::new(width, axis.length(), 1.0),
    })
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(super) fn place_faces<C: Component>(
    cameras: Query<(Entity, &Camera), With<C>>,
    faces: Query<(Entity, &Bubble, &BubblePx, &SpeechFace)>,
    mut poses: ParamSet<(
        TransformHelper,
        Query<(&mut Transform, &mut Visibility), Without<WaypointArrowRoot>>,
    )>,
    mut arrows: Query<
        (&Node, &ComputedNode, &mut Visibility),
        (With<WaypointArrowRoot>, Without<SpeechFace>),
    >,
    scale: Res<BalloonScale>,
    a11y: Option<Res<A11y>>,
    focus: Option<Res<InteractionFocus>>,
) {
    if faces.is_empty() {
        return;
    }
    let Some((entity, camera)) = cameras.iter().find(|(_, c)| c.is_active) else {
        return;
    };
    let Some(view) = camera
        .logical_viewport_rect()
        .filter(|r| r.size().min_element() > 1.0)
    else {
        return;
    };
    let Ok(eye) = poses.p0().compute_global_transform(entity) else {
        return;
    };
    let up = eye.rotation() * Vec3::Y;
    for (entity, bubble, px, face) in &faces {
        let Ok(owner) = poses.p0().compute_global_transform(bubble.owner) else {
            continue;
        };
        let mut writers = poses.p1();
        let Ok((mut transform, mut visibility)) = writers.get_mut(entity) else {
            continue;
        };
        // track_bubbles já resolveu Visibility em toda a cadeia do dono.
        if *visibility == Visibility::Hidden {
            continue;
        }
        let crown = owner.transform_point(face.crown);
        let anchor = crown + Vec3::Y * 0.055;
        let tip = anchor + (eye.translation() - anchor).normalize_or_zero() * 0.06;
        let (Ok(p), Ok(q)) = (
            camera.world_to_viewport(&eye, tip),
            camera.world_to_viewport(&eye, tip + up),
        ) else {
            *visibility = Visibility::Hidden;
            continue;
        };
        // Um orador fora do campo não deixa uma fala colada à borda.
        if !view.contains(p)
            && !camera
                .world_to_viewport(&eye, owner.translation())
                .is_ok_and(|foot| view.contains(foot))
        {
            *visibility = Visibility::Hidden;
            continue;
        }
        let pixels_per_metre = p.distance(q);
        if !pixels_per_metre.is_finite() || pixels_per_metre <= 0.0 {
            continue;
        }
        let reference = (view.height() / 720.0).clamp(0.75, 1.6);
        let focused = focus
            .as_ref()
            .is_some_and(|f| f.wins(KeyCode::KeyE, bubble.owner));
        let raster_scale = (pixels_per_metre * scale.world_per_px).clamp(
            if focused { 0.68 * reference } else { 0.0 },
            0.85 * reference,
        );
        let size = px.0.as_vec2()
            * a11y
                .as_deref()
                .map_or(raster_scale, |a| subtitle_px(raster_scale, a));
        let gap = 12.0 * reference;
        let rect = safe_rect(view, p, size, gap);
        let delta = rect.center() - p;
        let right = eye.rotation() * Vec3::X;
        transform.translation = tip + (right * delta.x - up * delta.y) / pixels_per_metre;
        transform.rotation = eye.rotation();
        transform.scale = (rect.size() / pixels_per_metre).extend(1.0);

        if let Ok((mut pointer, mut visible)) = writers.get_mut(face.pointer) {
            if let Some(pose) = pointer_pose(rect, p, gap).filter(|_| view.contains(p)) {
                *pointer = pose;
                visible.set_if_neq(Visibility::Inherited);
            } else {
                visible.set_if_neq(Visibility::Hidden);
            }
        }

        // O HUD já posicionou a seta neste Update. Só cede o lugar quando
        // a seta OU a legenda atravessa a área de leitura; o HUD repõe a
        // visibilidade normal no próximo frame.
        for (node, computed, mut arrow) in &mut arrows {
            let (Val::Percent(x), Val::Percent(y)) = (node.left, node.top) else {
                continue;
            };
            let center = view.min + Vec2::new(x, y) * 0.01 * view.size();
            let size = computed.size() * computed.inverse_scale_factor();
            if size.min_element() <= 0.0
                || !rect
                    .inflate(4.0)
                    .intersect(Rect::from_center_size(center, size))
                    .is_empty()
            {
                arrow.set_if_neq(Visibility::Hidden);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_and_pointer_fit_at_viewport_edges() {
        for size in [
            Vec2::new(1280., 720.),
            Vec2::new(360., 640.),
            Vec2::new(2560., 1080.),
        ] {
            let view = Rect::from_corners(Vec2::new(30., 60.), Vec2::new(30., 60.) + size);
            for corner in [Vec2::splat(0.5), Vec2::ZERO, Vec2::X, Vec2::Y, Vec2::ONE] {
                let tip = view.min + size * corner;
                for natural in [Vec2::new(340., 110.), Vec2::new(900., 1600.)] {
                    let card = safe_rect(view, tip, natural, 12.);
                    assert!(view.contains(card.min) && view.contains(card.max));
                    assert!((card.width() / card.height() - natural.x / natural.y).abs() < 1e-4);
                    if !card.contains(tip) {
                        let pointer = pointer_pose(card, tip, 12.).unwrap();
                        let point = pointer.transform_point(Vec3::NEG_Y).truncate();
                        let actual = card.center() + point * card.size() * Vec2::new(1., -1.);
                        assert!(actual.distance(tip) < 0.01);
                    }
                }
            }
        }
    }
}

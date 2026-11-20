//! Resident birds share the standing forest's spatial index and branch geometry.
use super::FaunaVisual;
use avian3d::prelude::{Collider, Sensor, ShapeCastConfig, SpatialQuery, SpatialQueryFilter};
use bevy::prelude::*;
use bevy_flora::PooledTree;
mod perches;
use perches::{Perches, Site, survey_perches};
use std::{collections::HashSet, f32::consts::TAU};
use vael_engine::{player::Player, scene_loader::SceneEntity, terrain::TerrainRuntime};

pub(super) struct WoodlandBirdsPlugin;

impl Plugin for WoodlandBirdsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Perches>()
            .register_type::<WoodlandBird>()
            .add_systems(Startup, super::calls::load)
            .add_systems(
                Update,
                (survey_perches, populate, fly, super::calls::listen).chain(),
            );
    }
}

#[derive(Clone, Copy, Reflect)]
enum Motion {
    Rest {
        remaining: f32,
    },
    Flight {
        path: [Vec3; 4],
        elapsed: f32,
        duration: f32,
        low: bool,
    },
}

#[derive(Component, Reflect)]
#[reflect(Component)]
pub(super) struct WoodlandBird {
    pub(super) owl: bool,
    perch: Option<Site>, // occupied while resting, reserved while approaching
    motion: Motion,
    pub(super) phase: f32,
    visits: u32,
}

impl WoodlandBird {
    pub(super) fn resting(&self) -> bool {
        matches!(self.motion, Motion::Rest { .. })
    }
}

fn night(clock: Option<&bevy_elements::WorldClock>) -> bool {
    clock.is_some_and(|c| !(5.3..19.7).contains(&c.hour))
}

#[allow(clippy::too_many_arguments)]
fn populate(
    mut commands: Commands,
    time: Res<Time>,
    mut next: Local<f32>,
    hero: Query<&GlobalTransform, With<Player>>,
    clock: Option<Res<bevy_elements::WorldClock>>,
    perches: Res<Perches>,
    birds: Query<&WoodlandBird>,
) {
    let Some(hero) = hero.iter().next() else {
        return;
    };
    let now = time.elapsed_secs();
    if now < *next {
        return;
    }
    *next = now + 0.9;
    let owl = night(clock.as_deref());
    let limit = if owl { 3 } else { 9 };
    let occupied: HashSet<_> = birds.iter().filter_map(|b| b.perch).collect();
    // Leave some branches free for the next landing, even in a small grove.
    let wanted = limit.min(perches.sites.len().div_ceil(2));
    let vacancies = wanted.saturating_sub(
        birds
            .iter()
            .filter(|b| b.owl == owl && b.perch.is_some())
            .count(),
    );
    let on_ground = occupied.iter().filter(|s| s.ground()).count();
    let mut available: Vec<_> = perches
        .sites
        .iter()
        .filter(|s| {
            !occupied.contains(&s.id) && (8.0..49.0).contains(&s.feet.distance(hero.translation()))
        })
        .collect();
    // Make the first clearing encounter visible near the path, keep most birds in trees.
    let ahead = hero.translation() + *hero.back() * 6.;
    available.sort_by_key(|s| {
        let prefer_ground = !owl && on_ground < 2;
        (
            s.id.ground() != prefer_ground,
            (s.feet.distance(ahead) * 100.) as i32,
        )
    });
    for site in available.into_iter().take(vacancies.min(2)) {
        let phase = (site.feet.x * 3.17 + site.feet.z * 1.71).rem_euclid(TAU);
        commands.spawn((
            Name::new(if owl { "Coruja" } else { "Gaio" }),
            SceneEntity,
            WoodlandBird {
                owl,
                perch: Some(site.id),
                motion: Motion::Rest {
                    remaining: 2. + phase,
                },
                phase,
                visits: 0,
            },
            Transform::from_translation(site.feet).with_rotation(facing(site.outward)),
            FaunaVisual::new(if owl { "owl" } else { "jay" }, phase),
        ));
    }
}

fn facing(dir: Vec3) -> Quat {
    Quat::from_euler(
        EulerRot::YXZ,
        dir.x.atan2(dir.z),
        -dir.y.atan2(dir.xz().length()),
        0.,
    )
}

fn make_path(start: Vec3, outward: Vec3, end: Vec3, approach: Vec3) -> [Vec3; 4] {
    let span = start.distance(end);
    let reach = (span * 0.34).clamp(1.2, 7.5);
    let top = start.y.max(end.y) + (span * 0.32).clamp(2., 8.);
    [
        start,
        (start + outward * reach).with_y(top),
        (end + approach * reach).with_y(top),
        end,
    ]
}

fn along_path([a, b, c, d]: [Vec3; 4], t: f32) -> (Vec3, Vec3) {
    let left = a.lerp(b, t).lerp(b.lerp(c, t), t);
    let right = b.lerp(c, t).lerp(c.lerp(d, t), t);
    (left.lerp(right, t), (right - left).normalize_or(Vec3::Z))
}

// Blend down from the branch once clear of the crown and flare back up at
// the far end. The middle of a low pass follows the local terrain at eye level.
fn flight_point(path: [Vec3; 4], t: f32, low: bool, floor: impl Fn(Vec2) -> f32) -> Vec3 {
    let (mut p, _) = along_path(path, t);
    if t <= 0. || t >= 1. {
        return p;
    }
    let ground = floor(p.xz());
    if low {
        let u = (t.min(1. - t) / 0.32).clamp(0., 1.);
        p.y = p.y.lerp(ground + 1.72, u * u * (3. - 2. * u));
    }
    p.y = p.y.max(ground + (t.min(1. - t) * 6.).min(1.) * 0.7);
    p
}

fn route_clear(
    path: [Vec3; 4],
    low: bool,
    floor: &impl Fn(Vec2) -> f32,
    spatial: &SpatialQuery,
    sensors: &Query<(), With<Sensor>>,
) -> bool {
    let radius = Collider::sphere(0.18);
    let filter = SpatialQueryFilter::default();
    let mut prev = path[0] + Vec3::Y * 0.23;
    for i in 1..=12 {
        let next = flight_point(path, i as f32 / 12., low, floor) + Vec3::Y * 0.23;
        let step = next - prev;
        if let Ok(dir) = Dir3::new(step)
            && spatial
                .cast_shape_predicate(
                    &radius,
                    prev,
                    Quat::IDENTITY,
                    dir,
                    &ShapeCastConfig::from_max_distance(step.length()),
                    &filter,
                    &|e| !sensors.contains(e),
                )
                .is_some()
        {
            return false;
        }
        prev = next;
    }
    true
}

#[allow(clippy::too_many_arguments)]
fn fly(
    mut commands: Commands,
    time: Res<Time>,
    hero: Query<&GlobalTransform, With<Player>>,
    trees: Query<(), With<PooledTree>>,
    terrain: Option<Res<TerrainRuntime>>,
    clock: Option<Res<bevy_elements::WorldClock>>,
    spatial: SpatialQuery,
    sensors: Query<(), With<Sensor>>,
    perches: Res<Perches>,
    mut birds: Query<(Entity, &mut WoodlandBird, &mut Transform, &mut FaunaVisual)>,
) {
    let (Some(hero), Some(terrain)) = (hero.iter().next(), terrain) else {
        return;
    };
    let ahead = hero.translation().xz() + hero.back().xz().normalize_or_zero() * 9.;
    let hero = hero.translation();
    let dt = time.delta_secs().min(0.12);
    let now = time.elapsed_secs();
    let owl_time = night(clock.as_deref());
    let floor = |p: Vec2| {
        let ground = terrain.sample(p.x, p.y);
        ground.max(terrain.water_surface_at(p.x, p.y).unwrap_or(ground))
    };
    let mut occupied: HashSet<_> = birds.iter().filter_map(|(_, b, _, _)| b.perch).collect();
    for (entity, mut bird, mut tf, mut visual) in &mut birds {
        let pos = tf.translation;
        if pos.distance_squared(hero) > 85. * 85. {
            commands.entity(entity).try_despawn();
            continue;
        }
        let site = bird.perch.and_then(|id| {
            perches
                .sites
                .iter()
                .find(|p| p.id == id && id.available(&trees))
        });
        let flee = pos.xz().distance_squared(hero.xz()) < (5.4 + bird.phase * 0.28).powi(2)
            && (pos.y - hero.y).abs() < 9.;
        let motion = bird.motion;
        let replan = match motion {
            Motion::Rest { remaining } => {
                visual.play("Perch", 1.);
                if let Some(site) = site {
                    tf.translation = site.feet;
                    let glance = (now * 1.3 + bird.phase).sin() * 0.11;
                    let peck = if site.id.ground() && !bird.owl {
                        ((now * 2.1 + bird.phase).sin().max(0.)).powi(5) * 0.74
                    } else {
                        0.
                    };
                    let heading = if bird.visits > 0 {
                        -site.outward
                    } else {
                        site.outward
                    };
                    let rest = facing(heading)
                        * Quat::from_rotation_y(glance)
                        * Quat::from_rotation_x(peck);
                    tf.rotation = tf.rotation.slerp(rest, 1. - (-dt * 8.).exp());
                }
                bird.motion = Motion::Rest {
                    remaining: remaining - dt,
                };
                remaining <= 0. || flee || site.is_none() || bird.owl != owl_time
            }
            Motion::Flight { .. } => {
                bird.perch.is_some()
                    && site.is_none_or(|s| {
                        s.feet.xz().distance(hero.xz()) < 3.8 && (s.feet.y - hero.y).abs() < 6.
                    })
            }
        };
        if replan {
            let away = (pos - hero).with_y(0.).normalize_or(Vec3::Z);
            let departure = site.map_or_else(
                || (tf.rotation * Vec3::Z).with_y(0.).normalize_or(away),
                |s| s.outward,
            );
            let want_ground = !flee
                && site.is_some_and(|p| !p.id.ground())
                && (bird.visits + bird.phase as u32).is_multiple_of(if bird.owl { 6 } else { 3 });
            let mut choices: Vec<_> = perches
                .sites
                .iter()
                .filter(|p| {
                    bird.owl == owl_time
                        && !occupied.contains(&p.id)
                        && p.id.available(&trees)
                        && (4.5..35.).contains(&p.feet.distance(pos))
                        && p.feet.xz().distance(hero.xz()) > 6.5
                })
                .collect();
            choices.sort_by_cached_key(|p| {
                let d = p.feet - pos;
                let crossing = pos.lerp(p.feet, 0.5).xz().distance(ahead);
                let score = (d.length() - (10. + (bird.phase + bird.visits as f32).rem_euclid(6.)))
                    .abs()
                    + crossing * 0.65
                    - if flee { d.dot(away) * 0.45 } else { 0. };
                (p.id.ground() != want_ground, (score * 100.) as i32)
            });
            let target = choices.into_iter().take(8).find_map(|p| {
                let path = make_path(pos, departure, p.feet, p.outward);
                [true, false]
                    .into_iter()
                    .find(|&low| route_clear(path, low, &floor, &spatial, &sensors))
                    .map(|low| (p, path, low))
            });
            if target.is_none()
                && site.is_some()
                && !flee
                && bird.owl == owl_time
                && matches!(motion, Motion::Rest { .. })
            {
                bird.motion = Motion::Rest { remaining: 3. };
                continue;
            }
            if let Some(old) = bird.perch {
                occupied.remove(&old);
            }
            bird.perch = target.map(|(p, _, _)| p.id);
            if let Some(dest) = bird.perch {
                occupied.insert(dest);
            }
            let (path, low) = target.map_or_else(
                || {
                    let end = pos + away * 55. + Vec3::Y * 16.;
                    (make_path(pos, departure, end, -away), false)
                },
                |(_, path, low)| (path, low),
            );
            let length: f32 = path.windows(2).map(|w| w[0].distance(w[1])).sum();
            bird.motion = Motion::Flight {
                path,
                low,
                elapsed: 0.,
                duration: (length / if bird.owl { 6.4 } else { 8.7 }).max(1.2),
            };
            bird.visits = bird.visits.wrapping_add(1);
        }
        if let Motion::Flight {
            path,
            elapsed,
            duration,
            low,
        } = bird.motion
        {
            let elapsed = (elapsed + dt).min(duration);
            let u = elapsed / duration;
            let t = u * u * (3. - 2. * u);
            let p = flight_point(path, t, low, floor);
            let dir = (p - pos)
                .try_normalize()
                .unwrap_or_else(|| along_path(path, t).1);
            tf.translation = p;
            let level = Vec3::new(dir.x, dir.y.clamp(-0.65, 0.65) * (1. - u.powi(5)), dir.z);
            tf.rotation = tf.rotation.slerp(facing(level), 1. - (-dt * 10.).exp());
            let glide = (0.30..0.74).contains(&t) && (bird.owl || (now + bird.phase).sin() > -0.25);
            let (clip, rate) = if u > 0.88 && bird.perch.is_some() {
                ("Land", 2.2)
            } else if glide {
                ("Soar", 1.0)
            } else {
                ("Hover", if bird.owl { 1.5 } else { 2.6 })
            };
            visual.play(clip, rate);
            if elapsed >= duration {
                if bird.perch.is_none() {
                    commands.entity(entity).try_despawn();
                    continue;
                }
                bird.motion = Motion::Rest {
                    remaining: if bird.owl { 12. } else { 4. }
                        + (bird.phase + bird.visits as f32 * 2.1).rem_euclid(7.),
                };
                visual.play("Perch", 1.);
            } else {
                bird.motion = Motion::Flight {
                    path,
                    low,
                    elapsed,
                    duration,
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_crossing_keeps_eye_height_over_sloping_ground_and_meets_both_branches() {
        let floor = |p: Vec2| 30. + p.x * 0.18;
        let a = Vec3::new(0., 35., 0.);
        let b = Vec3::new(16., 38., 0.);
        let path = make_path(a, Vec3::X, b, Vec3::NEG_X);
        for t in [0.33, 0.5, 0.67] {
            let p = flight_point(path, t, true, floor);
            assert!((p.y - floor(p.xz()) - 1.72).abs() < 1e-4);
        }
        assert_eq!(flight_point(path, 0., true, floor), a);
        assert_eq!(flight_point(path, 1., true, floor), b);
    }

    #[test]
    fn flight_leaves_and_returns_to_the_bark_from_above() {
        let a = Vec3::new(2., 3., 0.);
        let b = Vec3::new(-8., 5., 3.);
        let path = make_path(a, Vec3::X, b, Vec3::NEG_Z);
        assert_eq!(along_path(path, 0.).0, a);
        assert_eq!(along_path(path, 1.).0, b);
        assert!(along_path(path, 0.).1.x > 0.);
        assert!(along_path(path, 0.5).0.y > b.y);
        assert!(along_path(path, 1.).1.y < 0.);
    }
}

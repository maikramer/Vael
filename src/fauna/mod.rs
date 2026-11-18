//! Ambient animals follow short ranges around authored clearings and alleys.
mod birds;
mod calls;
mod visuals;
use visuals::FaunaVisual;

use bevy::prelude::*;
use std::f32::consts::TAU;
use viber::player::Player;
use viber::scene_loader::{CurrentScene, SceneEntity};
use viber::terrain::TerrainRuntime;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Species {
    Fox,
    Rat,
    Swallow,
    Butterfly,
    Trout,
    Hare,
}

impl Species {
    fn name(self) -> &'static str {
        match self {
            Self::Hare => "Lebre",
            Self::Trout => "Truta",
            Self::Fox => "Raposa",
            Self::Rat => "Rato",
            Self::Swallow => "Andorinha",
            Self::Butterfly => "Borboleta",
        }
    }
    fn model(self) -> &'static str {
        match self {
            Self::Fox => "fox",
            Self::Rat => "rat",
            Self::Hare => "hare",
            Self::Trout => "trout",
            Self::Butterfly => "butterfly",
            Self::Swallow => "swallow",
        }
    }
    fn flying(self) -> bool {
        matches!(self, Self::Swallow | Self::Butterfly)
    }
}

// Centre, semi-axes and population. Ground routes run beside buildings, in
// the lee of a wall, or through open country. No ambient animal owns a collider.
type Habitat = (Species, [f32; 2], [f32; 2], usize);
const HABITATS: &[Habitat] = &[
    (Species::Rat, [28., -14.], [2.3, 1.6], 3),
    (Species::Rat, [-33., 21.], [2.4, 1.4], 2),
    (Species::Fox, [-24., 51.], [5.6, 3.2], 2),
    (Species::Fox, [48., -24.], [4.5, 3.], 1),
    (Species::Fox, [-114., 144.], [8., 4.], 2),
    (Species::Fox, [72., -206.], [7., 4.], 1),
    (Species::Swallow, [0., 0.], [18., 13.], 8),
    (Species::Swallow, [-3., 88.], [13., 8.], 7),
    (Species::Swallow, [-125., 160.], [11., 8.], 6),
    (Species::Swallow, [-76., 103.], [17., 9.], 7),
    (Species::Swallow, [5., 210.], [19., 6.], 6),
    (Species::Swallow, [64., -182.], [12., 7.], 6),
    (Species::Swallow, [4., 42.], [13., 8.], 6),
    (Species::Hare, [-12., 46.], [3.8, 2.2], 3),
    (Species::Hare, [-31., 81.], [5., 3.], 2),
    (Species::Hare, [-66., 120.], [4., 2.5], 3),
    (Species::Hare, [-118., 147.], [5., 3.], 3),
    (Species::Hare, [64., -197.], [4., 2.5], 2),
    (Species::Hare, [13., 199.], [4., 2.5], 3),
    (Species::Fox, [-55., 127.], [7., 3.5], 1),
    (Species::Fox, [19., 202.], [6., 3.5], 1),
    (Species::Butterfly, [-11.6, 7.2], [1.7, 1.4], 5),
    (Species::Butterfly, [30.4, -5.7], [2., 2.], 4),
    (Species::Butterfly, [-15., 87.], [2.4, 1.4], 5),
    (Species::Trout, [-80., 104.], [10.4, 6.2], 5),
    (Species::Trout, [4., 215.], [8., 2.0], 4),
    (Species::Trout, [68., -184.], [4.5, 3.0], 3),
];

#[derive(Component)]
struct Fauna {
    habitat: usize,
    individual: usize,
    angle: f32,
    direction: f32,
    gait: f32,
    alert: f32,
}

pub struct FaunaPlugin;
impl Plugin for FaunaPlugin {
    fn build(&self, app: &mut App) {
        visuals::install(app);
        app.add_plugins(birds::WoodlandBirdsPlugin)
            .add_message::<calls::FoxAlarm>()
            .add_systems(Update, (populate, animate, calls::fox_alarm).chain());
    }
}
fn point(h: &Habitat, animal: &Fauna, angle: f32) -> Vec2 {
    let center = Vec2::from_array(h.1);
    let variation = 1.0 - 0.08 * animal.individual as f32;
    center + Vec2::from_array(h.2) * variation * Vec2::new(angle.cos(), angle.sin())
}

fn populate(
    mut commands: Commands,
    player: Query<&GlobalTransform, With<Player>>,
    scene: Option<Res<CurrentScene>>,
    terrain: Option<Res<TerrainRuntime>>,
    clock: Option<Res<bevy_elements::WorldClock>>,
    weather: Option<Res<bevy_elements::Weather>>,
    existing: Query<(Entity, &Fauna)>,
) {
    let hero = player.iter().next().map(|p| p.translation().xz());
    let outdoor =
        terrain.is_some() && scene.as_deref().and_then(|s| s.id.as_deref()) == Some("main");
    let insects_active = clock
        .as_deref()
        .is_none_or(|c| (6.0..19.5).contains(&c.hour))
        && weather
            .as_deref()
            .is_none_or(|w| w.temperature_c > 6.0 && w.precipitation_mm_h < 2.0);
    let active = |kind: Species| match kind {
        Species::Butterfly => insects_active,
        Species::Swallow => clock
            .as_deref()
            .is_none_or(|c| (5.0..20.5).contains(&c.hour)),
        _ => true,
    };
    let mut occupied = [0u16; HABITATS.len()];
    for (e, a) in &existing {
        if !outdoor
            || !active(HABITATS[a.habitat].0)
            || hero.is_none_or(|p| p.distance(Vec2::from_array(HABITATS[a.habitat].1)) > 110.)
        {
            commands.entity(e).try_despawn();
        } else {
            occupied[a.habitat] |= 1 << a.individual;
        }
    }
    let (Some(hero), Some(terrain)) = (hero.filter(|_| outdoor), terrain) else {
        return;
    };
    for (i, h) in HABITATS.iter().enumerate() {
        if !active(h.0) || hero.distance(Vec2::from_array(h.1)) > 84. {
            continue;
        }
        for j in 0..h.3 {
            if occupied[i] & (1 << j) != 0 {
                continue;
            }
            let animal = Fauna {
                habitat: i,
                individual: j,
                angle: (i as f32 * 1.73 + j as f32 * TAU / h.3 as f32) % TAU,
                direction: 1.,
                gait: 0.,
                alert: 0.,
            };
            let pos = point(h, &animal, animal.angle);
            if h.0 == Species::Trout {
                if terrain
                    .water_domain_at(pos.x, pos.y)
                    .is_none_or(|d| d.depth < 0.55)
                {
                    continue;
                }
            } else if !h.0.flying() && terrain.in_water(pos.x, pos.y) {
                continue;
            }
            let transform = Transform::from_xyz(pos.x, terrain.sample(pos.x, pos.y), pos.y);
            commands.spawn((
                Name::new(h.0.name()),
                SceneEntity,
                animal,
                FaunaVisual::new(h.0.model(), j as f32 * 0.37),
                transform,
                Visibility::Hidden,
            ));
        }
    }
}

fn animate(
    time: Res<Time>,
    terrain: Option<Res<TerrainRuntime>>,
    player: Query<&GlobalTransform, With<Player>>,
    mut waves: Option<ResMut<viber::terrain::WaterWaveWorld>>,
    mut foxes: Local<Vec<Vec2>>,
    mut alarms: MessageWriter<calls::FoxAlarm>,
    mut animals: Query<(
        &mut Fauna,
        &mut Transform,
        &mut FaunaVisual,
        &mut Visibility,
    )>,
) {
    let (Some(terrain), Some(hero)) = (terrain, player.iter().next()) else {
        return;
    };
    let dt = time.delta_secs().min(0.12);
    let now = time.elapsed_secs();
    foxes.clear();
    foxes.extend(
        animals
            .iter()
            .filter(|(a, _, _, vis)| {
                HABITATS[a.habitat].0 == Species::Fox && **vis == Visibility::Inherited
            })
            .map(|(_, tf, _, _)| tf.translation.xz()),
    );
    for (mut a, mut tf, mut visual, mut visibility) in &mut animals {
        let h = &HABITATS[a.habitat];
        let kind = h.0;
        let pos = point(h, &a, a.angle);
        let tangent = Vec2::new(-h.2[0] * a.angle.sin(), h.2[1] * a.angle.cos());
        let to_hero = hero.translation().xz() - pos;
        let phase = now + a.individual as f32 * 3.4 + a.habitat as f32;
        if kind == Species::Trout {
            let Some(domain) = terrain.water_domain_at(pos.x, pos.y) else {
                *visibility = Visibility::Hidden;
                continue;
            };
            let alarm = to_hero.length_squared() < 16.0
                && (hero.translation().y - domain.surface_y).abs() < 3.0;
            if alarm && a.alert == 0.0 {
                a.direction = if tangent.dot(to_hero) > 0.0 {
                    -1.0
                } else {
                    1.0
                };
            }
            a.alert = if alarm { 2.0 } else { (a.alert - dt).max(0.0) };
            let heading = tangent.normalize_or_zero() * a.direction;
            let speed = if a.alert > 0.0 { 3.4 } else { 1.2 };
            let speed = (speed + 0.35 * heading.dot(domain.flow)).clamp(0.7, 4.0);
            let step = dt * speed * a.direction / tangent.length().max(0.1);
            let next = point(h, &a, a.angle + step);
            let Some(water) = terrain
                .water_domain_at(next.x, next.y)
                .filter(|d| d.depth >= 0.55)
            else {
                a.direction *= -1.0;
                continue;
            };
            let period = 11.8 + 0.75 * a.individual as f32;
            let (offset, slope) = trout_rise(phase.rem_euclid(period));
            let target = if a.alert > 0.0 { -0.95 } else { offset };
            let target_y = water.surface_y + target.max(0.16 - water.depth);
            let old_y = tf.translation.y;
            let visible = *visibility == Visibility::Inherited;
            let y = if visible {
                old_y + (target_y - old_y).clamp(-5.0 * dt, 5.0 * dt)
            } else {
                target_y
            };
            let landed = old_y >= domain.surface_y && y < water.surface_y;
            let emerged = old_y <= domain.surface_y && y > water.surface_y;
            if visible
                && (landed || emerged)
                && let Some(sim) = waves.as_deref_mut()
            {
                sim.push_impulse(viber::terrain::WaveImpulseRequest {
                    pos: next,
                    vel: heading * speed,
                    radius: if landed { 0.46 } else { 0.30 },
                    strength: if landed { 0.13 } else { 0.06 },
                    foam: if landed { 0.36 } else { 0.12 },
                    is_wake: false,
                });
            }
            a.angle = (a.angle + step).rem_euclid(TAU);
            a.gait = (a.gait + dt * (1.8 + speed)).fract();
            tf.translation = Vec3::new(next.x, y, next.y);
            let pitch = if a.alert > 0.0 {
                0.0
            } else {
                -slope.atan2(speed)
            };
            tf.rotation =
                Quat::from_rotation_y(heading.x.atan2(heading.y)) * Quat::from_rotation_x(pitch);
            *visibility = Visibility::Inherited;
            visual.play("Swim", if a.alert > 0.0 { 2.1 } else { 1.0 });
            continue;
        }
        if kind == Species::Swallow {
            let (position, velocity, bank) = swallow_flight(h, &a, now);
            let y = terrain.sample(position.x, position.z).max(
                terrain
                    .water_surface_at(position.x, position.z)
                    .unwrap_or(f32::NEG_INFINITY),
            );
            tf.translation = position + Vec3::Y * y;
            tf.rotation = Quat::from_euler(
                EulerRot::YXZ,
                velocity.x.atan2(velocity.z),
                -velocity.y.atan2(velocity.xz().length()),
                bank,
            );
            a.gait = (a.gait + dt * (4.6 + a.individual as f32 * 0.12)).fract();
            // Power the climb and glide briefly on the descending part of the swoop.
            let glide = velocity.y < -0.4 && (phase * 1.3).sin() < 0.;
            visual.play(
                if glide { "Soar" } else { "Hover" },
                if glide { 1.0 } else { 2.6 },
            );
            *visibility = Visibility::Inherited;
            continue;
        }
        let mut threat = to_hero;
        if kind == Species::Hare {
            for &fox in foxes.iter() {
                if (fox - pos).length_squared() < threat.length_squared() {
                    threat = fox - pos;
                }
            }
        }
        let nearby = threat.length_squared()
            < match kind {
                Species::Hare => 64.,
                Species::Fox => 100.,
                Species::Rat => 20.,
                _ => 0.,
            }
            && (threat != to_hero
                || (hero.translation().y - terrain.sample(pos.x, pos.y)).abs() < 5.);
        let startled = nearby && a.alert <= 0.;
        if startled {
            a.direction = if tangent.dot(threat) > 0. { -1. } else { 1. };
        }
        a.alert = if nearby { 2.3 } else { (a.alert - dt).max(0.) };
        let (speed, cadence) = match kind {
            Species::Fox if a.alert > 0. => (5.6, 3.6),
            Species::Rat if a.alert > 0. => (2.8, 7.),
            Species::Fox if phase.rem_euclid(9.) < 2. => (0., 0.),
            Species::Rat if phase.rem_euclid(4.) < 0.9 => (0., 0.),
            Species::Fox => (1.8, 1.9),
            Species::Rat => (1.1, 4.3),
            Species::Trout => unreachable!("trout swim in the water domain"),
            Species::Hare if a.alert > 0. => (5.2, 3.1),
            Species::Hare if phase.rem_euclid(8.0) < 4.6 => (0., 0.),
            Species::Hare => (1.65, 1.9),
            Species::Swallow => unreachable!("swallows fly in flocks"),
            Species::Butterfly => (0.75, 5.6),
        };
        let step = dt * speed * a.direction / tangent.length().max(0.1);
        let next = point(h, &a, a.angle + step);
        let ground = terrain.sample(next.x, next.y);
        if !kind.flying()
            && (terrain.in_water(next.x, next.y)
                || (ground - terrain.sample(pos.x, pos.y)).abs() > 0.12 + pos.distance(next) * 0.6)
        {
            a.direction *= -1.;
            // A flooded habitat may already contain the animal at spawn.
            if terrain.in_water(pos.x, pos.y) {
                *visibility = Visibility::Hidden;
            }
            continue;
        }
        *visibility = Visibility::Inherited;
        a.angle = (a.angle + step).rem_euclid(TAU);
        a.gait = if speed == 0. {
            0.
        } else {
            (a.gait + dt * cadence).fract()
        };
        let heading = tangent * a.direction;
        let airborne = match kind {
            Species::Hare => 0.22 * (a.gait * std::f32::consts::PI).sin().powi(2),
            Species::Butterfly => 1.05 + 0.32 * (phase * 2.2).sin(),
            _ => 0.,
        };
        tf.translation = Vec3::new(next.x, ground + airborne, next.y);
        if startled && kind == Species::Fox {
            alarms.write(calls::FoxAlarm(tf.translation));
        }
        tf.rotation = Quat::from_rotation_y(heading.x.atan2(heading.y));
        let (clip, rate) = match kind {
            Species::Butterfly => ("Hover", 2.8),
            _ if speed == 0. => ("BreatheIdle", 1.),
            Species::Hare if a.alert <= 0. => ("Jump", cadence * 1.45),
            _ if a.alert > 0. => ("Run", 1.7),
            _ => ("Walk", 1.4),
        };
        visual.play(clip, rate);
    }
}

// The flock turns on one path. Each pair trails the preceding pair and
// spreads to opposite sides as the birds skim and climb together.
fn swallow_flight(h: &Habitat, bird: &Fauna, time: f32) -> (Vec3, Vec3, f32) {
    let rank = bird.individual.div_ceil(2) as f32;
    let side = if bird.individual % 2 == 1 {
        -rank
    } else {
        rank
    };
    let speed = 7.4 / h.2[0];
    let angle = time * speed + bird.habitat as f32 * 1.7 - rank * 0.12;
    let (sin, cos) = angle.sin_cos();
    let rx = h.2[0] + side * 0.74;
    let rz = h.2[1] + side * 0.74;
    let height = match h.1 {
        [0., 0.] => 13.5,                                // over the city's rooftops
        [-76., 103.] | [5., 210.] | [64., -182.] => 3.2, // skim over the water
        _ => 6.4,
    };
    let swoop = angle * 2. + side * 0.13;
    let pos = Vec3::new(
        h.1[0] + rx * cos,
        height + 1.7 * swoop.sin() + rank * 0.17,
        h.1[1] + rz * sin,
    );
    let velocity = Vec3::new(-rx * sin, 3.4 * swoop.cos(), rz * cos) * speed;
    let bank = (speed * speed * rx * rz / velocity.xz().length().max(0.1) * speed / 9.81).atan();
    (pos, velocity, bank)
}

// One feeding rise per circuit of the feeding timer. Position and velocity
// both join the submerged cruise smoothly at either end of the rise.
fn trout_rise(t: f32) -> (f32, f32) {
    const DURATION: f32 = 1.65;
    if t >= DURATION {
        return (-0.34, 0.0);
    }
    let phase = t * TAU / DURATION;
    (
        -0.34 + 0.5 * (1.0 - phase.cos()),
        0.5 * TAU / DURATION * phase.sin(),
    )
}

#[cfg(test)]
mod trout_tests {
    use super::trout_rise;

    #[test]
    fn feeding_rise_breaks_the_surface_and_returns_to_cruise() {
        let heights: Vec<_> = (0..200).map(|i| trout_rise(i as f32 * 0.01).0).collect();
        assert_eq!(
            heights
                .windows(2)
                .filter(|p| p[0].signum() != p[1].signum())
                .count(),
            2
        );
        assert!(heights.iter().copied().fold(0.0, f32::max) > 0.6);
        assert_eq!(trout_rise(2.0), trout_rise(0.0));
    }
}

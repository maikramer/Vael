//! Contacto lateral entre personagens e multidão macia (estilo Assassin's Creed).
//!
//! A marcha da IA e o recuo dos golpes escrevem o Transform fora do solver Avian
//! que conduz o herói. Aqui resolvemos a separação entre personagens, emitimos
//! [`ActorBump`] para o `bevy_ext_puppet` (jogo de ombro / passos para o lado /
//! trombadões em cadeia) e redirecionamos suavemente a velocidade do herói ao
//! contornar NPCs pacíficos para abrir caminho numa multidão sem travar seco.
use avian3d::prelude::{LinearVelocity, Position, RigidBodyDisabled};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::transform::TransformSystems;
use bevy_ext_core::{
    AiLocomotion, AiLocomotionSystems, EnemyCreature, LuaScriptRef, Player, vitals::Hostile,
};
use bevy_ext_puppet::prelude::{PuppetMode, PuppetSense, PuppetState};
use std::sync::{Arc, Mutex};

use vael_engine::recipes::spawn::NpcCharacter;

/// Evento emitido quando dois personagens colidem com velocidade de aproximação
/// significativa em [`ActorContactPlugin`]. Consumido por `puppet.rs` para
/// gerar jogo de ombro (`puppet_hit`) ou passos de abertura / queda (`puppet_push`).
#[derive(Message, Debug, Clone, Copy)]
pub(crate) struct ActorBump {
    pub a: Entity,
    pub b: Entity,
    /// Normal unitária no plano XZ apontando de `b` para `a`.
    pub normal: Vec2,
    /// Velocidade relativa de aproximação ao longo da normal (m/s).
    pub closing_speed: f32,
    /// Velocidade planar (XZ, m/s) de `a` no frame do contacto.
    pub a_vel: Vec2,
    /// Velocidade planar (XZ, m/s) de `b` no frame do contacto.
    pub b_vel: Vec2,
    /// `a` é um NPC pacífico de multidão (não-hostil, não-herói).
    pub a_npc: bool,
    /// `b` é um NPC pacífico de multidão (não-hostil, não-herói).
    pub b_npc: bool,
}

/// Velocidade relativa mínima (m/s) para emitir [`ActorBump`] (cobre caminhada
/// tranquila através de multidões, mas ignora spawns sobrepostos parados).
const MIN_BUMP_SPEED: f32 = 0.7;

pub(super) struct ActorContactPlugin;

impl Plugin for ActorContactPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ActorBump>()
            .init_resource::<ContactBodies>()
            .add_systems(Startup, install_contact_range)
            .add_systems(
                PostUpdate,
                resolve_contacts
                    .after(AiLocomotionSystems)
                    .before(crate::camera::GameplayCameraSet)
                    .before(TransformSystems::Propagate),
            );
    }
}

// A investida Luau precisa do MESMO alcance quando uma criatura grande
// bloqueia a marcha antes de o centro entrar na largura normal do golpe.
#[derive(Resource, Default, Clone)]
struct ContactBodies(Arc<Mutex<HashMap<u64, (Vec3, f32)>>>);

fn install_contact_range(bodies: Res<ContactBodies>) {
    let bodies = bodies.0.clone();
    crate::script_host::register_installer(move |lua| {
        let map = bodies.clone();
        crate::vault::vael_table(lua)?.set(
            "contact_distance",
            lua.create_function(move |_, (a, b): (u64, u64)| {
                let entries = map.lock().unwrap_or_else(|e| e.into_inner());
                Ok(entries.get(&a).zip(entries.get(&b)).map(|(a, b)| a.1 + b.1))
            })?,
        )
    });
}

#[derive(Clone, Copy)]
struct Footprint {
    radius: f32,
    height: f32,
}

fn is_npc_script(path: &str) -> bool {
    path.starts_with("npc/")
        || matches!(
            path,
            "townsfolk.lua" | "watch-guard.lua" | "merchant.lua" | "healer.lua"
        )
}

fn footprint(hero: bool, hostile: bool, npc: bool, path: &str) -> Option<Footprint> {
    let (radius, height) = if hero {
        (
            vael_engine::player::HERO_RADIUS,
            2.0 * vael_engine::player::HERO_CAPSULE_CENTER,
        )
    } else if hostile {
        match path {
            "enemies/wolf.lua" => (0.65, 0.85),
            "enemies/scorpion.lua" | "enemies/slime.lua" => (0.48, 0.70),
            "enemies/bogling.lua" | "enemies/bogling-hexer.lua" => (0.42, 1.1),
            _ if path.starts_with("bosses/") => (0.85, 2.1),
            _ => (0.43, 1.7),
        }
    } else if npc || is_npc_script(path) {
        (0.35, 1.75)
    } else {
        return None;
    };
    Some(Footprint { radius, height })
}

#[derive(Clone, Copy)]
struct Motion {
    entity: Entity,
    start: Vec3,
    goal: Vec3,
    end: Vec3,
    body: Footprint,
    mobile: bool,
    /// NPC pacífico (multidão): herói contorna suavemente pelo ombro em vez de travar seco.
    crowd_npc: bool,
    /// Puppet físico a conduzir o corpo (`Active`/`GettingUp`): usa o CoM real e não recebe teleporte cinemático.
    puppet_driven: bool,
}

impl Motion {
    fn min_x(&self) -> f32 {
        self.start.x.min(self.end.x) - self.body.radius
    }
    fn max_x(&self) -> f32 {
        self.start.x.max(self.end.x) + self.body.radius
    }
}

#[derive(Clone, Copy)]
struct RawContact {
    a: Entity,
    b: Entity,
    normal: Vec2,
    closing_dist: f32,
    a_step: Vec2,
    b_step: Vec2,
    a_npc: bool,
    b_npc: bool,
}

// Intervalo do frame em que os corpos ocupam o mesmo piso.
fn height_interval(a: &Motion, b: &Motion) -> Option<(f32, f32)> {
    let y = a.start.y - b.start.y;
    let dy = (a.end.y - b.end.y) - y;
    let (lo, hi) = (-a.body.height, b.body.height);
    if dy.abs() < 1e-5 {
        return (lo < y && y < hi).then_some((0., 1.));
    }
    let (t1, t2) = ((lo - y) / dy, (hi - y) / dy);
    let (enter, exit) = (t1.min(t2).max(0.), t1.max(t2).min(1.));
    (enter < exit).then_some((enter, exit))
}

// Contacto entre os discos EM MOVIMENTO. A normal no primeiro toque preserva
// o lado de chegada mesmo se uma investida cruzar o alvo num só frame.
fn separate(a: &mut Motion, b: &mut Motion) -> Option<RawContact> {
    if !(a.mobile || b.mobile) {
        return None;
    }
    let (enter, exit) = height_interval(a, b)?;
    let radius = a.body.radius + b.body.radius;
    let da = (a.end - a.start).xz();
    let db = (b.end - b.start).xz();
    let v = da - db;
    let relative = (a.start - b.start).xz() + v * enter;
    let c = relative.length_squared() - radius * radius;
    let at = if c <= 0. {
        relative
    } else {
        let along = relative.dot(v);
        let speed_sq = v.length_squared();
        let disc = along * along - speed_sq * c;
        if along >= 0. || disc < 0. {
            return None;
        }
        let t = c / (-along + disc.sqrt());
        if t > exit - enter {
            return None;
        }
        relative + v * t
    };
    let normal = at
        .try_normalize()
        .unwrap_or_else(|| v.try_normalize().unwrap_or(Vec2::X));
    let depth = radius - (a.end - b.end).xz().dot(normal);
    if depth < 0.0001 {
        return None;
    }

    // Fechamento relativo ao longo da normal (m neste frame), antes do ajuste.
    let closing_dist = (-v.dot(normal)).max(0.);

    // Quem entrou no contacto perde a componente normal do passo. Quem já
    // estava parado mantém o lugar. Só um spawn já sobreposto divide o ajuste.
    let into_a = if a.mobile {
        (-da.dot(normal)).max(0.)
    } else {
        0.
    };
    let into_b = if b.mobile { db.dot(normal).max(0.) } else { 0. };
    let share = if into_a + into_b > 1e-6 {
        into_a / (into_a + into_b)
    } else if !a.mobile {
        0.
    } else if !b.mobile {
        1.
    } else {
        0.5
    };
    let push = Vec3::new(normal.x, 0., normal.y) * depth;
    a.end += push * share;
    b.end -= push * (1. - share);
    Some(RawContact {
        a: a.entity,
        b: b.entity,
        normal,
        closing_dist,
        a_step: da,
        b_step: db,
        a_npc: a.crowd_npc,
        b_npc: b.crowd_npc,
    })
}

fn solve(motions: &mut [Motion], contacts: &mut Vec<RawContact>) {
    contacts.clear();
    // Broad phase no X varrido; uma segunda passagem apanha contactos criados
    // pela primeira (por exemplo, três lobos a chegar ao mesmo bandido).
    for _ in 0..8 {
        motions.sort_unstable_by(|a, b| {
            a.min_x()
                .total_cmp(&b.min_x())
                .then(a.entity.cmp(&b.entity))
        });
        let mut changed = false;
        for i in 0..motions.len() {
            let (head, rest) = motions.split_at_mut(i + 1);
            let a = &mut head[i];
            for b in rest {
                if b.min_x() > a.max_x() {
                    break;
                }
                if let Some(hit) = separate(a, b) {
                    changed = true;
                    if hit.closing_dist > 0.
                        && !contacts.iter().any(|c| {
                            (c.a == hit.a && c.b == hit.b) || (c.a == hit.b && c.b == hit.a)
                        })
                    {
                        contacts.push(hit);
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
}

type Actors = (
    Or<(
        With<Player>,
        With<EnemyCreature>,
        With<Hostile>,
        With<LuaScriptRef>,
        With<NpcCharacter>,
    )>,
    Without<vael_engine::corpse::Corpse>,
    Without<RigidBodyDisabled>,
    Without<crate::boat::RidingBoat>,
);

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn resolve_contacts(
    mut actors: Query<
        (
            Entity,
            &mut Transform,
            Option<&ChildOf>,
            Option<&LuaScriptRef>,
            Has<Player>,
            Has<Hostile>,
            Has<EnemyCreature>,
            Has<NpcCharacter>,
            Has<crate::player::CrouchedCollider>,
            Option<&Visibility>,
            Option<&mut AiLocomotion>,
            Option<&mut Position>,
            Option<&mut LinearVelocity>,
            Option<&PuppetState>,
            Option<&PuppetSense>,
        ),
        Actors,
    >,
    parents: Query<&GlobalTransform>,
    time: Res<Time>,
    bodies: Res<ContactBodies>,
    mut bumps: MessageWriter<ActorBump>,
    mut motions: Local<Vec<Motion>>,
    mut contacts: Local<Vec<RawContact>>,
    mut crowd_contact_hero: Local<bool>,
) {
    let dt = time.delta_secs();
    if dt == 0. {
        return;
    }
    motions.clear();
    let mut previous = bodies.0.lock().unwrap_or_else(|e| e.into_inner());
    for (
        entity,
        transform,
        parent,
        script,
        hero,
        hostile,
        creature,
        npc,
        crouched,
        visibility,
        loco,
        _,
        _,
        puppet_state,
        puppet_sense,
    ) in &actors
    {
        if visibility == Some(&Visibility::Hidden) {
            continue;
        }
        // Personagens caídos ou mortos no puppet não bloqueiam passagem de pé.
        if !hero
            && puppet_state.is_some_and(|s| {
                matches!(
                    s.mode,
                    PuppetMode::Falling | PuppetMode::Down | PuppetMode::Dead
                )
            })
        {
            continue;
        }
        let path = script.map_or("", |s| s.path.as_str());
        let Some(mut body) = footprint(hero, hostile || creature, npc, path) else {
            continue;
        };
        if hero && crouched {
            body.height = crate::player::HERO_CROUCH_HEIGHT;
        }
        let world = parent
            .and_then(|p| parents.get(p.parent()).ok())
            .copied()
            .unwrap_or_default()
            .mul_transform(*transform);
        let (scale, _, mut goal) = world.to_scale_rotation_translation();
        body.radius *= scale.x.abs().max(scale.z.abs());
        body.height *= scale.y.abs();

        // Quando o puppet físico conduz o corpo (`Active`/`GettingUp`), usamos
        // a posição real do CoM em XZ (acompanha os passos de abertura!) e um
        // raio ligeiramente mais justo para o herói espremer pelo corredor.
        let puppet_driven = !hero
            && puppet_state
                .is_some_and(|s| matches!(s.mode, PuppetMode::Active | PuppetMode::GettingUp));
        if puppet_driven && let Some(sense) = puppet_sense {
            goal.x = sense.com.x;
            goal.z = sense.com.z;
            body.radius = (body.radius * 0.80).max(0.24);
        }

        if !goal.is_finite() || body.radius <= 0. || body.height <= 0. {
            continue;
        }
        let crowd_npc = !hero && !(hostile || creature) && (npc || is_npc_script(path));
        // Teleporte, respawn e troca de cena começam no destino. A folga
        // temporal inclui dash e knockback mesmo num frame longo.
        let start = previous
            .get(&entity.to_bits())
            .map(|entry| entry.0)
            .filter(|p| p.distance(goal) <= 2. + 32. * dt)
            .unwrap_or(goal);
        motions.push(Motion {
            entity,
            body,
            start,
            goal,
            end: goal,
            mobile: hero || hostile || creature || puppet_driven || loco.is_some() || start != goal,
            crowd_npc,
            puppet_driven,
        });
    }
    solve(&mut motions, &mut contacts);

    *crowd_contact_hero = false;
    for c in contacts.iter() {
        let closing_speed = c.closing_dist / dt;
        if closing_speed >= MIN_BUMP_SPEED {
            if c.a_npc || c.b_npc {
                *crowd_contact_hero = true;
            }
            bumps.write(ActorBump {
                a: c.a,
                b: c.b,
                normal: c.normal,
                closing_speed,
                a_vel: c.a_step / dt,
                b_vel: c.b_step / dt,
                a_npc: c.a_npc,
                b_npc: c.b_npc,
            });
        }
    }

    previous.clear();
    for m in motions.iter() {
        previous.insert(m.entity.to_bits(), (m.end, m.body.radius));
        // Puppet físico ativo move-se pelas próprias pernas — não teleportar a raiz.
        if m.puppet_driven {
            continue;
        }
        let correction = m.end - m.goal;
        if correction.length_squared() < 1e-12 {
            continue;
        }
        let Ok((_, mut transform, parent, _, hero, _, _, _, _, _, loco, position, velocity, _, _)) =
            actors.get_mut(m.entity)
        else {
            continue;
        };
        let local_delta = parent
            .and_then(|p| parents.get(p.parent()).ok())
            .map_or(correction, |p| {
                p.affine().inverse().transform_vector3(correction)
            });
        transform.translation += local_delta;
        if let Some(mut pos) = position {
            pos.x = m.end.x;
            pos.z = m.end.z;
        }
        let n = correction.normalize();
        if let Some(mut vel) = velocity {
            let into = vel.0.dot(n).min(0.);
            vel.0 -= n * into;
            // Estilo Assassin's Creed: ao esbarrar num NPC de multidão, converte
            // parte do avanço bloqueado num deslize suave pelo ombro em vez de
            // travar seco contra uma parede.
            if hero && *crowd_contact_hero && into < -0.5 {
                let n2 = n.xz().normalize_or_zero();
                let mut tangent = Vec2::new(-n2.y, n2.x);
                let step_xz = (m.goal - m.start).xz();
                if step_xz.dot(tangent) < 0. {
                    tangent = -tangent;
                }
                let slip = (-into * 0.55).min(3.5);
                vel.0.x += tangent.x * slip;
                vel.0.z += tangent.y * slip;
            }
        }
        if let Some(mut loco) = loco {
            let into = loco.velocity.dot(n.xz()).min(0.);
            loco.velocity -= n.xz() * into;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actor(id: u32, x: f32, goal: f32) -> Motion {
        let start = Vec3::X * x;
        let goal = Vec3::X * goal;
        Motion {
            entity: Entity::from_raw_u32(id).unwrap(),
            start,
            goal,
            end: goal,
            body: Footprint {
                radius: 0.5,
                height: 1.7,
            },
            mobile: true,
            crowd_npc: false,
            puppet_driven: false,
        }
    }

    #[test]
    fn crossing_in_one_frame_stops_the_runner_on_the_near_side() {
        let mut a = actor(1, -3., 4.);
        let mut b = actor(2, 0., 0.);
        let hit = separate(&mut a, &mut b).expect("contacto esperado");
        assert!(hit.closing_dist > 6.9);
        assert_eq!(a.end, -Vec3::X);
        assert_eq!(b.end, Vec3::ZERO);
        let mut left = actor(1, -1., -3.);
        assert!(separate(&mut left, &mut b).is_none());
    }

    #[test]
    fn opposing_dodges_preserve_the_order_and_tangent() {
        let mut a = actor(1, -2., 2.);
        let mut b = actor(2, 2., -2.);
        a.end.z = 1.;
        b.end.z = 1.;
        assert!(separate(&mut a, &mut b).is_some());
        assert_eq!((a.end.x, b.end.x), (-0.5, 0.5));
        assert_eq!((a.end.z, b.end.z), (1., 1.));
    }

    #[test]
    fn height_excludes_a_bridge_but_allows_a_landing_contact() {
        let mut a = actor(1, -2., 2.);
        let mut b = actor(2, 0., 0.);
        a.start.y = 3.;
        a.end.y = 3.;
        assert!(separate(&mut a, &mut b).is_none());
        a.end.y = 0.;
        assert!(separate(&mut a, &mut b).is_some());
        assert_eq!(a.end.y, 0.);
    }

    #[test]
    fn crowd_holds_a_stationary_npc_and_resolves_coincident_spawns() {
        let mut contacts = Vec::new();
        let mut people = [actor(1, -3., 0.), actor(2, -1.2, 0.), actor(3, 0., 0.)];
        people[2].mobile = false;
        solve(&mut people, &mut contacts);
        assert!(
            !contacts.is_empty(),
            "corredor em NPC gera contacto com fecho"
        );
        people.sort_by_key(|m| m.entity.index());
        assert!(people[0].end.x <= -1.99 && people[1].end.x <= -0.999);
        assert_eq!(people[2].end, Vec3::ZERO);
        let mut overlap = [actor(1, 0., 0.), actor(2, 0., 0.)];
        solve(&mut overlap, &mut contacts);
        assert!(
            contacts.is_empty(),
            "spawn sobreposto parado não é esbarrão"
        );
        assert!(overlap[0].end.distance(overlap[1].end) >= 0.999);
    }

    #[test]
    fn game_schedule_updates_physics_and_child_space_and_releases_a_corpse() {
        use std::time::Duration;
        let mut app = App::new();
        app.add_message::<ActorBump>()
            .insert_resource(Time::<()>::default())
            .init_resource::<ContactBodies>()
            .add_systems(PostUpdate, resolve_contacts);
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs_f32(0.1));
        let parent = app
            .world_mut()
            .spawn(GlobalTransform::from_xyz(50., 0., 0.))
            .id();
        let foe = app
            .world_mut()
            .spawn((Hostile, Transform::IDENTITY, ChildOf(parent)))
            .id();
        let hero = app
            .world_mut()
            .spawn((
                Player::default(),
                Transform::from_xyz(48., 0., 0.),
                Position(Vec3::new(48., 0., 0.)),
                LinearVelocity(Vec3::new(8., 2., 0.)),
            ))
            .id();
        app.update();
        app.world_mut()
            .get_mut::<Transform>(hero)
            .unwrap()
            .translation
            .x = 51.;
        app.update();
        let x = app.world().get::<Transform>(hero).unwrap().translation.x;
        assert!((x - 49.22).abs() < 0.001);
        assert_eq!(app.world().get::<Position>(hero).unwrap().x, x);
        assert_eq!(
            app.world().get::<LinearVelocity>(hero).unwrap().0,
            Vec3::Y * 2.
        );
        assert_eq!(
            app.world().get::<Transform>(foe).unwrap().translation,
            Vec3::ZERO
        );
        app.world_mut()
            .entity_mut(foe)
            .insert(vael_engine::corpse::Corpse { timer: 1.0 });
        app.world_mut()
            .get_mut::<Transform>(hero)
            .unwrap()
            .translation
            .x = 51.;
        app.update();
        assert_eq!(
            app.world().get::<Transform>(hero).unwrap().translation.x,
            51.
        );
        // Aldeão estático autorado só com NpcBody, sem IA nem script.
        let villager = app
            .world_mut()
            .spawn((NpcCharacter, Transform::from_xyz(53., 0., 0.)))
            .id();
        app.update();
        app.world_mut()
            .get_mut::<Transform>(hero)
            .unwrap()
            .translation
            .x = 54.;
        app.update();
        assert!((app.world().get::<Transform>(hero).unwrap().translation.x - 52.3).abs() < 1e-4);
        assert_eq!(
            app.world()
                .get::<Transform>(villager)
                .unwrap()
                .translation
                .x,
            53.
        );
    }
}

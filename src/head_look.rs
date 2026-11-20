//! Look-at da cabeça: quem olha para quem.
//!
//! A primitiva vive na engine (`vael_engine::ik::HeadLookAt` — arco clampado 40°,
//! slerp amortecido). O jogo decide os alvos: NPCs com script e criaturas
//! hostis viram a cabeça para o herói dentro do alcance, e voltam ao olhar
//! calado (peso 0) quando ele sai de cena ou do alcance. O componente só
//! entra em rigs com pernas já ligadas ao IK (`vael_engine::ik::FootIkRigs`) —
//! caixas/portas com script não têm cabeça para virar.
//!
//! O componente é NOSSO: quando a entidade deixa de ser elegível (perde o
//! script, a hostilidade ou o rig), ou o herói desaparece, ele sai — sem
//! isto ficava para sempre a apontar para uma entidade morta.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy_ext_core::{Player, vitals::Hostile};
use vael_engine::ik::{FootIkRigs, HEAD_LOOK_WEIGHT, HeadLookAt};
use vael_engine::luau::components::LuaScriptRef;

/// Alcance do olhar: quem tem script (townsfolk, mercadores) até 6 m.
const NPC_RANGE: f32 = 6.0;
/// Hostis (monstros em perseguição) até 10 m.
const HOSTILE_RANGE: f32 = 10.0;
/// Folga de SAÍDA do alcance (× o alcance de entrada). Sem ela, um herói
/// parado exactamente no limite fazia o peso oscilar 0 ↔ 1 frame a frame
/// (a cabeça tremia): entra a `range`, só larga a `range × isto`.
const EXIT_FACTOR: f32 = 1.15;

pub struct VaelHeadLookPlugin;

impl Plugin for VaelHeadLookPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, drive_head_look);
    }
}

/// O que a entidade quer olhar neste frame.
#[derive(Clone, Copy)]
struct Want {
    target: Entity,
    target_pos: Vec3,
    /// Onde a entidade está (posição do rig, não do osso).
    at: Vec3,
    /// Alcance de ENTRADA (a saída é `enter × EXIT_FACTOR`).
    enter: f32,
}

/// Liga/desliga o olhar por proximidade. Corre por frame; só escreve quando
/// o alvo ou o estado mudam (evitar dirty por frame à toa).
#[allow(clippy::type_complexity)]
fn drive_head_look(
    players: Query<(Entity, &GlobalTransform), With<Player>>,
    watch: Query<
        (Entity, &GlobalTransform, Option<&Hostile>),
        (
            With<FootIkRigs>,
            Or<(With<LuaScriptRef>, With<Hostile>)>,
            Without<Player>,
        ),
    >,
    mut looks: Query<(Entity, &mut HeadLookAt), Without<Player>>,
    mut commands: Commands,
    mut wanted: Local<HashMap<Entity, Want>>,
) {
    wanted.clear();
    if let Some((player, player_g)) = players.iter().next() {
        let target_pos = player_g.translation();
        for (entity, g, hostile) in &watch {
            wanted.insert(
                entity,
                Want {
                    target: player,
                    target_pos,
                    at: g.translation(),
                    enter: if hostile.is_some() {
                        HOSTILE_RANGE
                    } else {
                        NPC_RANGE
                    },
                },
            );
        }
    }
    // Já têm o componente: actualiza o alvo/peso (com histerese) ou limpa.
    for (entity, mut look) in &mut looks {
        let Some(want) = wanted.get(&entity) else {
            // Saiu do conjunto (perdeu o script/hostil/rig) ou o herói já não
            // existe: limpar em vez de ficar a apontar para o vazio.
            commands.entity(entity).remove::<HeadLookAt>();
            continue;
        };
        let active = look.weight > 0.0;
        let limit = if active {
            want.enter * EXIT_FACTOR
        } else {
            want.enter
        };
        let within = want.at.distance(want.target_pos) <= limit;
        if look.target != want.target || active != within {
            look.target = want.target;
            look.weight = if within { HEAD_LOOK_WEIGHT } else { 0.0 };
        }
    }
    // Entraram no alcance neste frame e ainda não têm o componente.
    for (entity, want) in &wanted {
        if looks.contains(*entity) {
            continue;
        }
        if want.at.distance(want.target_pos) <= want.enter {
            commands
                .entity(*entity)
                .try_insert(HeadLookAt::toward(want.target));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ext_core::vitals::Hostile;
    use vael_engine::ik::FootIkLeg;
    use vael_engine::luau::components::LuaScriptRef;

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_systems(Update, drive_head_look);
        app
    }

    /// Rig ligado ao IK com entidades de osso fictícias (o olhar só lê a pose).
    fn rig() -> FootIkRigs {
        let bone = Entity::PLACEHOLDER;
        FootIkRigs {
            root: bone,
            legs: [FootIkLeg {
                owner: bone,
                target: bone,
                bones: (bone, bone, bone),
            }; 2],
        }
    }

    fn npc(app: &mut App, at: Vec3) -> Entity {
        app.world_mut()
            .spawn((
                GlobalTransform::from_translation(at),
                rig(),
                LuaScriptRef {
                    path: "scripts/townsfolk.lua".into(),
                },
            ))
            .id()
    }

    fn hero(app: &mut App, at: Vec3) -> Entity {
        app.world_mut()
            .spawn((GlobalTransform::from_translation(at), Player::default()))
            .id()
    }

    fn move_to(app: &mut App, entity: Entity, at: Vec3) {
        app.world_mut()
            .entity_mut(entity)
            .insert(GlobalTransform::from_translation(at));
    }

    fn weight(app: &App, entity: Entity) -> Option<f32> {
        app.world()
            .get::<HeadLookAt>(entity)
            .map(|look| look.weight)
    }

    #[test]
    fn entra_no_alcance_e_sai_com_folga() {
        let mut app = app();
        let player = hero(&mut app, Vec3::ZERO);
        let npc = npc(&mut app, Vec3::new(NPC_RANGE - 0.1, 0.0, 0.0));
        app.update();
        let look = app
            .world()
            .get::<HeadLookAt>(npc)
            .expect("entrou no alcance");
        assert_eq!(look.target, player);
        assert_eq!(look.weight, HEAD_LOOK_WEIGHT);

        // No limite exacto: a histerese mantém o olhar (antes piscava).
        move_to(&mut app, npc, Vec3::new(NPC_RANGE, 0.0, 0.0));
        app.update();
        assert_eq!(weight(&app, npc), Some(HEAD_LOOK_WEIGHT), "dentro da folga");

        // Fora da folga: larga.
        move_to(
            &mut app,
            npc,
            Vec3::new(NPC_RANGE * EXIT_FACTOR + 0.1, 0.0, 0.0),
        );
        app.update();
        assert_eq!(weight(&app, npc), Some(0.0));
        // Já inactivo, o limite volta a ser o de ENTRADA: não reentra a 6.1 m.
        move_to(&mut app, npc, Vec3::new(NPC_RANGE + 0.05, 0.0, 0.0));
        app.update();
        assert_eq!(weight(&app, npc), Some(0.0), "sem histerese a entrar");
    }

    #[test]
    fn hostil_tem_alcance_maior() {
        let mut app = app();
        hero(&mut app, Vec3::ZERO);
        let far = Vec3::new(NPC_RANGE + 1.0, 0.0, 0.0);
        let mob = app
            .world_mut()
            .spawn((GlobalTransform::from_translation(far), rig(), Hostile))
            .id();
        app.update();
        assert_eq!(weight(&app, mob), Some(HEAD_LOOK_WEIGHT));
    }

    #[test]
    fn componente_sai_quando_a_entidade_deixa_de_ser_elegivel_ou_o_heroi_some() {
        let mut app = app();
        let player = hero(&mut app, Vec3::ZERO);
        let npc = npc(&mut app, Vec3::new(1.0, 0.0, 0.0));
        app.update();
        assert!(weight(&app, npc).is_some());

        // O script saiu da entidade (o rig já não é um NPC).
        app.world_mut().entity_mut(npc).remove::<LuaScriptRef>();
        app.update();
        assert!(
            app.world().get::<HeadLookAt>(npc).is_none(),
            "sem script o componente tem de sair"
        );

        // Volta a ser NPC e o herói desaparece: o alvo morreu.
        app.world_mut().entity_mut(npc).insert(LuaScriptRef {
            path: "scripts/townsfolk.lua".into(),
        });
        app.update();
        assert!(weight(&app, npc).is_some());
        app.world_mut().entity_mut(player).despawn();
        app.update();
        assert!(
            app.world().get::<HeadLookAt>(npc).is_none(),
            "alvo despawnado limpa o olhar"
        );
    }
}

//! Look-at da cabeça: quem olha para quem.
//!
//! A primitiva vive na engine (`viber::ik::HeadLookAt` — arco clampado 40°,
//! slerp amortecido). O jogo decide os alvos: NPCs com script e criaturas
//! hostis viram a cabeça para o herói dentro do alcance, e voltam ao olhar
//! calado (peso 0) quando ele sai de cena ou do alcance. O componente só
//! entra em rigs com pernas já ligadas ao IK (`viber::ik::FootIkRigs`) —
//! caixas/portas com script não têm cabeça para virar.

use bevy::prelude::*;
use bevy_ext_core::{Player, vitals::Hostile};
use viber::ik::{FootIkRigs, HeadLookAt, HEAD_LOOK_WEIGHT};
use viber::luau::components::LuaScriptRef;

/// Alcance do olhar: quem tem script (townsfolk, mercadores) até 6 m.
const NPC_RANGE: f32 = 6.0;
/// Hostis (monstros em perseguição) até 10 m.
const HOSTILE_RANGE: f32 = 10.0;

pub struct VaelHeadLookPlugin;

impl Plugin for VaelHeadLookPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, drive_head_look);
    }
}

/// Liga/desliga o olhar por proximidade. Corre por frame; só escreve quando
/// o alvo ou o estado mudam (evitar dirty por frame à toa).
#[allow(clippy::type_complexity)]
fn drive_head_look(
    players: Query<(Entity, &GlobalTransform), With<Player>>,
    watch: Query<
        (
            Entity,
            &GlobalTransform,
            Option<&Hostile>,
            Has<HeadLookAt>,
        ),
        (
            With<FootIkRigs>,
            Or<(With<LuaScriptRef>, With<Hostile>)>,
            Without<Player>,
        ),
    >,
    mut existing: Query<&mut HeadLookAt>,
    mut commands: Commands,
) {
    let Some((player, player_g)) = players.iter().next() else {
        return;
    };
    for (entity, g, hostile, has_look) in &watch {
        let range = if hostile.is_some() {
            HOSTILE_RANGE
        } else {
            NPC_RANGE
        };
        let wants = g.translation().distance(player_g.translation()) <= range;
        if has_look {
            if let Ok(mut look) = existing.get_mut(entity) {
                let active = look.weight > 0.0;
                if look.target != player || active != wants {
                    look.target = player;
                    look.weight = if wants { HEAD_LOOK_WEIGHT } else { 0.0 };
                }
            }
        } else if wants {
            commands.entity(entity).insert(HeadLookAt::toward(player));
        }
    }
}

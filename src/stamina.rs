//! Fôlego partilhado pela locomoção e pelos golpes do herói.
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

pub const CAPACITY: f32 = 100.0;
pub const SWIM_DRAIN: f32 = 8.0;
pub const SWIM_SPEED_LIMIT: f32 = 3.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Reflect)]
#[serde(default)]
pub struct Stamina {
    pub value: f32,
    cooldown: f32,
    winded: bool,
    pub shore: Option<Vec3>,
    pub boat: Option<String>,
}

impl Default for Stamina {
    fn default() -> Self {
        Self {
            value: CAPACITY,
            cooldown: 0.0,
            winded: false,
            shore: None,
            boat: None,
        }
    }
}

impl Stamina {
    fn drain(&mut self, amount: f32) {
        self.value = (self.value - amount).max(0.0);
        self.winded |= self.value == 0.0;
        self.cooldown = 1.0;
    }

    pub fn attack(&mut self) -> bool {
        if self.value < 15.0 {
            return false;
        }
        self.drain(15.0);
        true
    }

    /// Devolve se o multiplicador de corrida pode entrar neste passo.
    pub fn tick(&mut self, dt: f32, swimming: bool, run: bool, resting: bool) -> bool {
        if swimming {
            self.drain(SWIM_DRAIN * dt);
        } else if run && !self.winded {
            self.drain(11.0 * dt);
            return self.value > 0.0;
        } else if resting {
            let recovery = (dt - self.cooldown).max(0.0);
            self.cooldown = (self.cooldown - dt).max(0.0);
            self.value = (self.value + 18.0 * recovery).min(CAPACITY);
            if self.value >= 25.0 {
                self.winded = false;
            }
        }
        false
    }
}

/// Regressa a quem podia socorrer o nadador: o casco de onde saltou ou a última margem.
pub(crate) fn rescue_swimmer(
    world: &mut World,
    hero: Entity,
    shore: Option<Vec3>,
    boat: Option<&str>,
) {
    use crate::boat::{Boat, RidingBoat};
    use avian3d::prelude::{LinearVelocity, Position, RigidBodyDisabled};
    use vael_engine::luau::components::ScriptToast;

    let vessel = boat.and_then(|name| {
        world
            .query_filtered::<(Entity, &Name), With<Boat>>()
            .iter(world)
            .find(|(_, n)| n.as_str() == name)
            .map(|(id, _)| id)
    });
    let (message_key, fallback) = if let Some(boat) = vessel {
        world
            .entity_mut(hero)
            .insert((RidingBoat { boat }, RigidBodyDisabled));
        (
            "stamina.rescue.boat",
            "Sem fôlego. Agarras-te à borda do barco.",
        )
    } else {
        // Saves anteriores ao fôlego podem começar a meio de um lago sem margem registada.
        let from = shore.filter(|p| p.is_finite()).unwrap_or_else(|| {
            let ground = world
                .get_resource::<vael_engine::terrain::TerrainRuntime>()
                .map_or(0.0, |rt| rt.sample(0.0, 0.0));
            Vec3::Y * (ground + 0.3)
        });
        let landing = crate::respawn::teleport_landing(world, from, &[hero]);
        if let Some(mut pose) = world.get_mut::<Transform>(hero) {
            pose.translation = landing;
        }
        world.entity_mut(hero).insert((
            Position(landing),
            LinearVelocity::ZERO,
            bevy_ext_core::TeleportSettle::default(),
        ));
        (
            "stamina.rescue.shore",
            "Sem fôlego. Dás por ti de volta à margem.",
        )
    };
    if world.contains_resource::<Messages<ScriptToast>>() {
        let message = world
            .get_resource::<vael_engine::locale::Locale>()
            .and_then(|locale| locale.table.get(message_key))
            .map_or_else(|| fallback.to_owned(), Clone::clone);
        world.write_message(ScriptToast(message));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missed_swings_still_cost_and_a_refused_swing_costs_nothing() {
        let mut stamina = Stamina::default();
        for _ in 0..6 {
            assert!(stamina.attack());
        }
        assert!(!stamina.attack());
        assert_eq!(stamina.value, 10.0);
        stamina.tick(1.5, false, false, true);
        assert_eq!(stamina.value, 19.0);
    }

    #[test]
    fn swimming_and_running_spend_the_same_reserve() {
        for hz in [20, 60, 120] {
            let mut s = Stamina::default();
            for _ in 0..2 * hz {
                assert!(s.tick(1.0 / hz as f32, false, true, true));
            }
            assert!(s.attack());
            for _ in 0..8 * hz {
                assert!(!s.tick(1.0 / hz as f32, true, false, true));
            }
            assert_eq!(s.value, 0.0);
            assert!(!s.tick(1.0, false, true, true));
            assert!(!s.tick(1.5, false, true, true));
            assert!(s.tick(0.02, false, true, true));
        }
    }
}

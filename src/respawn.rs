//! Respawn único do herói: o mesmo patch para o botão "Tentar de novo", para
//! o fallback sem modal e para o pedido da autoridade de vitais.
//!
//! O `rules.lua` continua dono da perda de bolsa (`drop_purse`), do som de
//! morte e do `M.respawn` quando a autoridade está desligada. Este módulo só
//! garante que, com a autoridade ligada, o estado e o teleporte são os mesmos.

use std::collections::BTreeMap;

use bevy::prelude::*;
use serde_json::{Value, json};
use viber::ambient::{SfxClip, SfxEvent};
use viber::luau::components::ScriptToast;
use viber::luau::host::LuaScriptHost;
use viber::player::{Player, TeleportSettle};
use viber::scene_loader::{Arrival, ChangeScene, CurrentScene, request_change};
use viber::ui::UiWriter;

use crate::save::VaelSave;

/// I-frames depois de renascer (o mesmo valor do `M.respawn` do Lua).
pub const RESPAWN_IFRAMES: f32 = 2.0;
/// Frascos base, antes do bónus de Espírito (`FLASK_BASE` no `rules.lua`).
pub const FLASK_BASE: u32 = 2;

/// Pedido de respawn vindo do fallback da autoridade (sem modal de morte).
/// O sistema de UI consome-o e chama [`respawn_now`].
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct RespawnRequest {
    pub active: bool,
    pub spawn: [f32; 3],
}

/// Frascos a partir do valor de `stat("flasks")` (o Lua faz
/// `2 + floor(stat)`, com um mínimo de 1).
pub fn flasks_from_stat(stat: f64) -> u32 {
    (FLASK_BASE as f64 + stat.floor()).max(1.0) as u32
}

/// Chaves de `viber.game()` que o respawn repõe. Igual ao `M.respawn`:
/// vida cheia, morte desarmada, estados limpos e frascos no máximo.
pub fn respawn_patch(save: &VaelSave, flasks: u32) -> BTreeMap<String, Value> {
    let mut patch = BTreeMap::new();
    patch.insert("hp".to_string(), json!(save.vitals.max_hp.max(1.0)));
    patch.insert("max_hp".to_string(), json!(save.vitals.max_hp.max(1.0)));
    patch.insert("dying".to_string(), json!(0));
    patch.insert("death_pending".to_string(), json!(false));
    patch.insert("death_wait".to_string(), json!(0));
    patch.insert("iframes".to_string(), json!(RESPAWN_IFRAMES));
    patch.insert("venom".to_string(), json!(0));
    patch.insert("burn".to_string(), json!(0));
    patch.insert("frost".to_string(), json!(0));
    patch.insert("venom_tick".to_string(), json!(0));
    patch.insert("burn_tick".to_string(), json!(0));
    patch.insert("frost_tick".to_string(), json!(0));
    patch.insert("flasks".to_string(), json!(flasks));
    patch.insert("title_done".to_string(), json!(true));
    patch
}

/// Teleporta o herói para `requested` com o chão da física (a mesma via do
/// `viber.teleport_player`: `teleport_landing` + `TeleportSettle`).
pub fn teleport_player(world: &mut World, requested: Vec3) {
    let Some(entity) = world
        .query_filtered::<Entity, With<Player>>()
        .iter(world)
        .next()
    else {
        return;
    };
    let landing = viber::player::teleport_landing(world, requested, &[entity]);
    if let Some(mut transform) = world.get_mut::<Transform>(entity) {
        transform.translation = landing;
    }
    if let Some(mut player) = world.get_mut::<Player>(entity) {
        player.vel_x = 0.0;
        player.vel_y = 0.0;
        player.vel_z = 0.0;
    }
    world.entity_mut(entity).insert(TeleportSettle::default());
}

/// Aplica o respawn completo. Chamado pelo botão de morte e pelo fallback.
///
/// Fora de `main`/`menu`, a troca de cena usa `Arrival::Position(spawn)` e
/// deixa `pending_respawn` para a ponte Lua; se a troca falhar, teleporta na
/// mesma (o jogador nunca fica preso no interior).
pub fn respawn_now(world: &mut World, spawn: Vec3, flasks: u32) {
    let save = world.resource::<VaelSave>().clone();
    let mut patch = respawn_patch(&save, flasks);
    let scene = world.get_resource::<CurrentScene>().cloned();
    let interior = scene
        .as_ref()
        .and_then(|scene| scene.id.as_deref())
        .is_some_and(|id| id != "main" && id != "menu");
    if interior {
        patch.insert("pending_respawn".to_string(), json!(true));
    }
    if let Some(host) = world.get_resource::<LuaScriptHost>() {
        viber::luau::game::apply_patch(&host.lua, &patch);
    }
    if let Some(writer) = world.get_resource::<UiWriter>() {
        writer.open("death", false);
        writer.set_visible("hud", true);
    }
    if let Some(mut state) = world.get_resource_mut::<crate::ui::screens::VaelScreensState>() {
        state.hud_visible = true;
        state.death_open_requested = false;
    }
    if interior {
        let change = ChangeScene::new("main", Arrival::Position(spawn));
        if let Err(err) = request_change(world, change) {
            warn!("vael: respawn sem troca de cena: {err}");
            teleport_player(world, spawn);
        }
    } else {
        teleport_player(world, spawn);
    }
    if world.contains_resource::<Messages<ScriptToast>>() {
        world.write_message(ScriptToast("Renasces no último descanso.".into()));
    }
    if world.contains_resource::<Messages<SfxEvent>>() {
        world.write_message(SfxEvent {
            clip: SfxClip::LevelUp,
            position: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{Inventory, PlayerVitals};
    use crate::save::{Equipment, StoryFlags};

    fn save() -> VaelSave {
        VaelSave {
            vitals: PlayerVitals {
                hp: 0.0,
                max_hp: 120.0,
            },
            inventory: Inventory::default(),
            equipment: Equipment::default(),
            story: StoryFlags::default(),
            ..Default::default()
        }
    }

    #[test]
    fn frascos_seguem_a_formula_do_lua() {
        assert_eq!(flasks_from_stat(0.0), 2);
        assert_eq!(flasks_from_stat(0.75), 2, "floor antes da soma");
        assert_eq!(flasks_from_stat(2.0), 4);
        assert_eq!(flasks_from_stat(-10.0), 1, "o mínimo do Lua é 1");
    }

    #[test]
    fn respawn_patch_restores_vitals_status_and_flasks() {
        let patch = respawn_patch(&save(), 3);
        assert_eq!(patch.get("hp"), Some(&json!(120.0)));
        assert_eq!(patch.get("max_hp"), Some(&json!(120.0)));
        assert_eq!(patch.get("dying"), Some(&json!(0)));
        assert_eq!(patch.get("death_pending"), Some(&json!(false)));
        assert_eq!(patch.get("iframes"), Some(&json!(RESPAWN_IFRAMES)));
        assert_eq!(patch.get("venom"), Some(&json!(0)));
        assert_eq!(patch.get("burn_tick"), Some(&json!(0)));
        assert_eq!(patch.get("flasks"), Some(&json!(3)));
    }
}

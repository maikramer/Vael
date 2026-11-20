//! Respawn único do herói: o mesmo patch para o botão "Tentar de novo", para
//! o fallback sem modal e para o pedido da autoridade de vitais.
//!
//! O `rules.lua` continua dono da perda de bolsa (`drop_purse`), do som de
//! morte e do `M.respawn` quando a autoridade está desligada. Este módulo só
//! garante que, com a autoridade ligada, o estado e o teleporte são os mesmos.

use std::collections::BTreeMap;

use bevy::prelude::*;
use bevy_ext_core::{GROUND_PROBE, Player, TeleportSettle};
use bevy_ext_core::{SfxClip, SfxEvent};
use serde_json::{Value, json};
use vael_engine::luau::components::ScriptToast;
use vael_engine::luau::game::{apply_patch_json, game_to_json_process};
use vael_engine::scene_loader::{Arrival, ChangeScene, CurrentScene, request_change};
use vael_engine::ui::UiWriter;

use crate::save::VaelSave;

/// Altura da sonda de aterragem acima do ponto pedido.
pub const TELEPORT_PROBE_HEIGHT: f32 = 2.0;
/// Queda máxima procurada por colliders abaixo do destino (m).
pub const TELEPORT_MAX_DROP: f32 = 400.0;
/// Folga acima da superfície encontrada.
pub const TELEPORT_SKIN: f32 = 0.02;

fn landing_position(
    terrain: Option<&vael_engine::terrain::runtime::TerrainRuntime>,
    requested: Vec3,
) -> Vec3 {
    let Some(terrain) = terrain else {
        return requested;
    };
    if !terrain.in_field(requested.x, requested.z) {
        return requested;
    }
    let surface = terrain
        .surface_below(requested.x, requested.z, requested.y + GROUND_PROBE)
        .unwrap_or_else(|| terrain.sample(requested.x, requested.z));
    Vec3::new(requested.x, surface, requested.z)
}

/// Primeira superfície de física abaixo de `(x, z)` (props, plataformas,
/// pontes e o trimesh do terreno), de `from_y` para baixo.
pub fn physics_ground_below(
    world: &mut World,
    x: f32,
    z: f32,
    from_y: f32,
    max_drop: f32,
    exclude: &[bevy::ecs::entity::Entity],
) -> Option<f32> {
    // O `SpatialQuery` do Avian é um SystemParam: o raio (com a exclusão da
    // hierarquia do prop e dos sensores) vive na engine, partilhado com o
    // teleporte do `vael_engine::player`.
    vael_engine::player::physics_ground_below(world, x, z, from_y, max_drop, exclude)
}

/// Y de aterragem de um teleporte resolvido contra a física, com o terreno
/// analítico como plano B.
pub fn teleport_landing(
    world: &mut World,
    requested: Vec3,
    exclude: &[bevy::ecs::entity::Entity],
) -> Vec3 {
    let from_y = requested.y + TELEPORT_PROBE_HEIGHT;
    if let Some(surface) = physics_ground_below(
        world,
        requested.x,
        requested.z,
        from_y,
        TELEPORT_PROBE_HEIGHT + TELEPORT_MAX_DROP,
        exclude,
    ) {
        return Vec3::new(requested.x, surface + TELEPORT_SKIN, requested.z);
    }
    let terrain = world.get_resource::<vael_engine::terrain::runtime::TerrainRuntime>();
    landing_position(terrain, requested)
}

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
    // O flag é DESTE caminho: nascer em `main` (ou com a troca falhada) não
    // pode deixar um `pending_respawn` de uma morte anterior a disparar um
    // teleporte no próximo `scene_loaded`.
    patch.insert("pending_respawn".to_string(), json!(false));
    patch
}

/// Fração do ouro que a morte larga numa bolsa (`DEATH_GOLD_FRAC` do Lua).
pub const DEATH_GOLD_FRAC: f64 = 0.25;

/// O que o `start_death` do Lua faz além de armar o `dying`: largar a bolsa
/// (25% do ouro no sítio da queda, só no `main`) e tocar `game_over`.
///
/// A morte por DoT nasce no tick da autoridade Rust (`rules_authority`), que
/// não passa pelo `M.start_death` do Lua — sem isto, morrer envenenado não
/// custava ouro nem tocava o sting de morte (o clip `death` fica com o rig,
/// que é do script).
pub fn start_death_side_effects(world: &mut World) {
    // `Transform` (e não a global, que só propaga no PostUpdate): a bolsa
    // fica onde o herói caiu neste frame.
    let player_pose = world
        .query_filtered::<(Entity, &Transform), With<Player>>()
        .iter(world)
        .next()
        .map(|(entity, transform)| (entity, transform.translation));
    let position = player_pose
        .map(|(_, translation)| translation)
        .filter(|position| position.is_finite());
    if let (Some((entity, _)), Some(control)) = (
        player_pose,
        world.get_resource::<crate::puppet::VaelPuppetControl>(),
    ) {
        control.request_kill(entity);
    }
    // `drop_purse`: só no mundo grande — as coordenadas de um interior não
    // valem em `main` (e sem cena, como no Lua, vale).
    let in_main = world
        .get_resource::<CurrentScene>()
        .is_none_or(|scene| scene.id.as_deref().is_none_or(|id| id == "main"));
    if let (true, Some(position)) = (in_main, position) {
        drop_purse(world, position);
    }
    if world.contains_resource::<Messages<SfxEvent>>() {
        world.write_message(SfxEvent {
            clip: SfxClip::GameOver,
            position,
        });
    }
}

/// `drop_purse`: um quarto do ouro fica numa bolsa no sítio da queda, pelo
/// cofre em Rust (`vault_get`/`vault_take`) e com o patch no `viber.game()`
/// que o `tick_purse` do Lua lê.
fn drop_purse(world: &mut World, position: Vec3) {
    let Some(content) = world.get_resource::<crate::content::VaelContent>() else {
        return;
    };
    let rules = crate::vault::VaultRules::from_items(&content.items);
    let gold_key = crate::vault::item_key("gold");
    let store = game_to_json_process();
    let mut items = BTreeMap::new();
    if let Some(gold) = store.get(&gold_key).and_then(crate::compat::json_i64) {
        items.insert(gold_key.clone(), gold.max(0));
    }
    let have = crate::vault::vault_get(&rules, &items, "gold");
    let lost = (have as f64 * DEATH_GOLD_FRAC).floor() as i64;
    if lost < 1 || !crate::vault::vault_take(&rules, &mut items, "gold", lost as f64) {
        return;
    }
    let mut patch = BTreeMap::new();
    if let Some(now) = items.get(&gold_key) {
        patch.insert(gold_key, json!(*now));
    }
    patch.insert("lost_gold".to_string(), json!(lost));
    patch.insert("lost_x".to_string(), json!(position.x));
    patch.insert("lost_y".to_string(), json!(position.y));
    patch.insert("lost_z".to_string(), json!(position.z));
    apply_patch_json(&patch);
}

/// `(dying, death_pending)` do `viber.game()`.
fn death_flags() -> (f32, bool) {
    use vael_engine::luau::game::{game_value_to_json, with_game_store};

    with_game_store(|store| {
        let dying = store.get("dying").map(game_value_to_json);
        let pending = store.get("death_pending").map(game_value_to_json);
        (
            dying.as_ref()
                .and_then(crate::compat::json_f32)
                .filter(|value| value.is_finite())
                .unwrap_or(0.0),
            pending.as_ref()
                .and_then(crate::compat::json_bool)
                .unwrap_or(false),
        )
    })
    .unwrap_or_default()
}

/// A morte COMEÇOU neste frame? `before`/`after` são `death_flags` antes e
/// depois do tick da autoridade.
///
/// Uma morte que já estava em curso (`dying > 0` ou `death_pending` no
/// `before`) não conta — a passagem de `dying` para `death_pending` 2,2 s
/// depois é a MESMA morte e não pode largar outra bolsa.
pub fn death_started(before: (f32, bool), after: (f32, bool)) -> bool {
    if before.0 > 0.0 || before.1 {
        return false;
    }
    after.0 > 0.0 || after.1
}

/// Estado de morte observado antes do tick da autoridade (o `dying` que o Lua
/// escreveu no dano direto está no store nesse ponto).
#[derive(Resource, Debug, Default)]
pub struct DeathWatch {
    before: (f32, bool),
}

fn watch_death_before_tick(mut watch: ResMut<DeathWatch>) {
    watch.before = death_flags();
}

/// Uma morte que nasceu no tick da autoridade (DoT, ou 0 HP de um save) não
/// passou pelo `M.start_death` do Lua: corre aqui o que ele faria.
fn death_started_in_rust(world: &mut World) {
    // Com o tick no Lua (`VAEL_RULES_RUST=0`) o `start_death` já correu lá.
    if !world
        .get_resource::<crate::rules_authority::VaelRulesConfig>()
        .is_some_and(|config| config.rust)
    {
        return;
    }
    let after = death_flags();
    let Some(before) = world.get_resource::<DeathWatch>().map(|watch| watch.before) else {
        return;
    };
    if !death_started(before, after) {
        return;
    }
    start_death_side_effects(world);
}

/// Plugin do respawn: observa o início das mortes do tick autoritativo.
pub struct VaelRespawnPlugin;

impl Plugin for VaelRespawnPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DeathWatch>()
            .add_systems(
                Update,
                watch_death_before_tick
                    .after(vael_engine::luau::bms_bridge::apply_bms_commands)
                    .before(crate::rules_authority::VaelRulesSet),
            )
            .add_systems(
                Update,
                death_started_in_rust.after(crate::rules_authority::VaelRulesSet),
            );
    }
}

/// Teleporta o herói para `requested` com o chão da física (a mesma via do
/// `viber.teleport_player`: `teleport_landing` + `TeleportSettle`).
pub fn teleport_player(world: &mut World, requested: Vec3) {
    crate::puppet::release_player_puppets(world);
    let Some(entity) = world
        .query_filtered::<Entity, With<Player>>()
        .iter(world)
        .next()
    else {
        return;
    };
    let landing = teleport_landing(world, requested, &[entity]);
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
    crate::puppet::release_player_puppets(world);
    let save = world.resource::<VaelSave>().clone();
    let mut patch = respawn_patch(&save, flasks);
    if let Some(bus) = world.get_resource::<crate::player::HeroMotionBus>() {
        bus.lock().stamina = Default::default();
    }
    let scene = world.get_resource::<CurrentScene>().cloned();
    let interior = scene
        .as_ref()
        .and_then(|scene| scene.id.as_deref())
        .is_some_and(|id| id != "main" && id != "menu");
    if interior {
        patch.insert("pending_respawn".to_string(), json!(true));
    }
    // O patch vai para o STORE do processo — o `viber.game()` de todos os
    // contextos BMS lê daqui (sem host, sem handle mlua).
    vael_engine::luau::game::apply_patch_json(&patch);
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
            // Sem troca, o `scene_loaded` que limpa o flag nunca chega:
            // limpá-lo aqui evita um teleporte fantasma no próximo load.
            let mut clear = BTreeMap::new();
            clear.insert("pending_respawn".to_string(), json!(false));
            vael_engine::luau::game::apply_patch_json(&clear);
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
        assert_eq!(
            patch.get("pending_respawn"),
            Some(&json!(false)),
            "o flag não pode ficar de uma morte anterior numa casa"
        );
    }

    #[test]
    fn morte_nova_e_so_o_inicio_dela() {
        // DoT: `dying` nasce no tick da autoridade.
        assert!(death_started((0.0, false), (2.2, false)));
        // Um hitch > 2,2 s esgota o `dying` no mesmo tick e cai no pendente.
        assert!(death_started((0.0, false), (0.0, true)));
        // Já estava a morrer: nada a fazer.
        assert!(!death_started((2.2, false), (2.1, false)));
        // A MESMA morte a chegar ao ecrã (dying → death_pending).
        assert!(!death_started((0.1, false), (0.0, true)));
        assert!(!death_started((0.0, true), (0.0, true)));
    }

    fn cena_interior(id: &str) -> CurrentScene {
        CurrentScene {
            root: std::path::PathBuf::new(),
            id: Some(id.to_string()),
            path: std::path::PathBuf::new(),
            title: None,
            entry: None,
            came_from: None,
            return_pose: None,
        }
    }

    fn mundo_com_morte(scene: &str) -> (World, Entity) {
        let mut world = World::new();
        world.init_resource::<Messages<SfxEvent>>();
        world.insert_resource(crate::content::VaelContent::default());
        world.insert_resource(cena_interior(scene));
        let player = world
            .spawn((Player::default(), Transform::from_xyz(1.0, 2.0, 3.0)))
            .id();
        (world, player)
    }

    /// A morte por DoT não passa pelo `start_death` do Lua: a bolsa e o som
    /// têm de sair daqui (paridade com `drop_purse`/`sound_at_player`).
    #[test]
    fn a_morte_no_rust_larga_a_bolsa_e_toca_game_over() {
        let _store_lock = vael_engine::luau::game::test_support::PROCESS_STORE_TEST_LOCK
            .lock()
            .expect("lock do store");
        vael_engine::luau::game::restore_game_json(&BTreeMap::new());
        let (mut world, _player) = mundo_com_morte("main");
        let mut seed = BTreeMap::new();
        seed.insert("item.gold".into(), json!(100));
        vael_engine::luau::game::apply_patch_json(&seed);

        start_death_side_effects(&mut world);

        let kv = vael_engine::luau::game::game_to_json_process();
        assert_eq!(kv.get("item.gold").and_then(Value::as_i64), Some(75));
        assert_eq!(kv.get("lost_gold").and_then(Value::as_i64), Some(25));
        assert_eq!(kv.get("lost_x").and_then(Value::as_f64), Some(1.0));
        assert_eq!(kv.get("lost_y").and_then(Value::as_f64), Some(2.0));
        assert_eq!(kv.get("lost_z").and_then(Value::as_f64), Some(3.0));
        let sfx: Vec<SfxEvent> = world
            .resource::<Messages<SfxEvent>>()
            .iter_current_update_messages()
            .cloned()
            .collect();
        assert!(
            sfx.iter().any(|event| event.clip == SfxClip::GameOver),
            "o sting de morte tem de tocar: {sfx:?}"
        );
    }

    /// `drop_purse` só vale no `main`: num interior o ouro fica, mas o som
    /// continua (o `start_death` do Lua não tem portão de cena para o som).
    #[test]
    fn num_interior_a_morte_nao_larga_bolsa() {
        let _store_lock = vael_engine::luau::game::test_support::PROCESS_STORE_TEST_LOCK
            .lock()
            .expect("lock do store");
        vael_engine::luau::game::restore_game_json(&BTreeMap::new());
        let (mut world, _player) = mundo_com_morte("interiors/chapel");
        let mut seed = BTreeMap::new();
        seed.insert("item.gold".into(), json!(100));
        vael_engine::luau::game::apply_patch_json(&seed);

        start_death_side_effects(&mut world);

        let kv = vael_engine::luau::game::game_to_json_process();
        assert_eq!(kv.get("item.gold").and_then(Value::as_i64), Some(100));
        assert!(!kv.contains_key("lost_gold"), "sem bolsa num interior");
        let sfx: Vec<SfxEvent> = world
            .resource::<Messages<SfxEvent>>()
            .iter_current_update_messages()
            .cloned()
            .collect();
        assert!(sfx.iter().any(|event| event.clip == SfxClip::GameOver));
    }

    /// Sem ouro suficiente para uma bolsa (ou sem posição do herói) o patch
    /// fica vazio e nada muda no store.
    #[test]
    fn sem_ouro_para_a_bolsa_nao_ha_patch() {
        let _store_lock = vael_engine::luau::game::test_support::PROCESS_STORE_TEST_LOCK
            .lock()
            .expect("lock do store");
        vael_engine::luau::game::restore_game_json(&BTreeMap::new());
        let (mut world, _player) = mundo_com_morte("main");
        let mut seed = BTreeMap::new();
        seed.insert("item.gold".into(), json!(3));
        vael_engine::luau::game::apply_patch_json(&seed);

        start_death_side_effects(&mut world);

        let kv = vael_engine::luau::game::game_to_json_process();
        assert_eq!(kv.get("item.gold").and_then(Value::as_i64), Some(3));
        assert!(!kv.contains_key("lost_gold"), "floor(3 * 0.25) = 0");
    }

    /// Regressão: morrer longe da praça (`puppet_kill` monta `Puppet` em
    /// `PuppetMode::Dead`) e clicar em "Tentar de novo" (`respawn_now`) tem de
    /// desmontar o corpo físico do herói antes do teleporte — caso contrário,
    /// `write_back_pose` mantinha os ossos (`Pelvis`..`HandR`) e a arma filha
    /// de `HandR` presos aos corpos ragdoll deixados no local da morte,
    /// deixando o personagem e a arma invisíveis na praça.
    #[test]
    fn respawn_now_dismantles_dead_hero_puppet_so_hero_and_weapon_stay_visible_at_spawn() {
        use std::time::Duration;
        use avian3d::prelude::*;
        use bevy::time::TimeUpdateStrategy;
        use bevy_ext_puppet::prelude::*;
        use bevy_ext_puppet::skeleton::spawn_test_skeleton_world;

        let _store_lock = vael_engine::luau::game::test_support::PROCESS_STORE_TEST_LOCK
            .lock()
            .expect("lock do store");
        vael_engine::luau::game::restore_game_json(&BTreeMap::new());

        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::transform::TransformPlugin,
            bevy::asset::AssetPlugin::default(),
            PhysicsPlugins::default(),
            crate::puppet::VaelPuppetPlugin,
        ));
        app.init_asset::<Mesh>();
        app.add_message::<bevy_ext_luau::commands::ScriptCommandMessage>();
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1.0 / 60.0,
        )));
        app.insert_resource(Time::<Fixed>::from_hz(60.0));
        app.insert_resource(save());
        app.insert_resource(cena_interior("main"));
        app.finish();
        app.cleanup();

        let death_pos = Vec3::new(50.0, 0.0, 50.0);
        let spawn_pos = Vec3::new(0.0, 0.0, 0.0);

        let hero = app
            .world_mut()
            .spawn((
                Player::default(),
                crate::components::VaelPlayer,
                Transform::from_translation(death_pos),
            ))
            .id();
        let bones = spawn_test_skeleton_world(app.world_mut(), hero);
        let hand_r = bones[PuppetPart::HandR.index()];
        let weapon = app
            .world_mut()
            .spawn((Name::new("weapon"), Transform::from_xyz(0.12, 0.04, 0.04)))
            .id();
        app.world_mut().entity_mut(hand_r).add_child(weapon);

        // Herói morre longe da praça (`puppet_kill`).
        app.world_mut()
            .resource::<crate::puppet::VaelPuppetControl>()
            .request_kill(hero);
        for _ in 0..10 {
            app.update();
        }
        assert!(
            app.world().get::<PuppetRig>(hero).is_some(),
            "o rig físico da morte tem de estar montado"
        );
        assert_eq!(
            app.world().get::<PuppetState>(hero).map(|s| s.mode),
            Some(PuppetMode::Dead)
        );

        // Jogador aperta "Tentar de novo" -> volta para a praça (`spawn_pos`).
        respawn_now(app.world_mut(), spawn_pos, 2);
        app.update();

        assert!(
            app.world().get::<Puppet>(hero).is_none()
                && app.world().get::<PuppetRig>(hero).is_none(),
            "respawn_now tem de desmontar o Puppet do herói"
        );

        let hero_gt = app.world().get::<GlobalTransform>(hero).unwrap().translation();
        let pelvis_gt = app
            .world()
            .get::<GlobalTransform>(bones[PuppetPart::Pelvis.index()])
            .unwrap()
            .translation();
        let weapon_gt = app.world().get::<GlobalTransform>(weapon).unwrap().translation();

        assert!(
            hero_gt.distance(spawn_pos) < 0.5,
            "raiz do herói na praça: {hero_gt:?}"
        );
        assert!(
            pelvis_gt.distance(spawn_pos) < 2.5,
            "esqueleto do herói tem de acompanhar a raiz na praça (e não ficar em {death_pos:?}): {pelvis_gt:?}"
        );
        assert!(
            weapon_gt.distance(spawn_pos) < 2.5,
            "arma em hand_r tem de estar na praça com o herói: {weapon_gt:?}"
        );
    }
}

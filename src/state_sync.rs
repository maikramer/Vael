//! Espelho Rust do estado do jogo (`viber.game()`) — shadow mode.
//!
//! Nesta passagem o Lua continua a ser a fonte de verdade: o plugin só LÊ o
//! snapshot (`game_to_json`) e copia-o para o [`VaelSave`] e para os
//! componentes tipados. Nada é escrito de volta na tabela Lua.

use std::collections::BTreeMap;

use bevy::prelude::*;
use bevy::time::Real;
use serde_json::Value;
use viber::luau::host::LuaScriptHost;

use crate::components::{Inventory, PlayerVitals, QuestLog, XpState};
use crate::save::VaelSave;

/// Período do snapshot. 10 Hz chegam para as ondas seguintes lerem estado
/// fresco; a 60 Hz só se clonava o `viber.game()` inteiro sem necessidade.
pub const SYNC_PERIOD_SECS: f32 = 0.1;

/// Plugin do espelho de estado.
pub struct VaelStateSyncPlugin;

impl Plugin for VaelStateSyncPlugin {
    fn build(&self, app: &mut App) {
        // O espelho escreve nestes resources; a App real não os cria em mais
        // lado nenhum (o `VaelSave` é criado pelo `VaelSavePlugin`).
        app.init_resource::<PlayerVitals>()
            .init_resource::<XpState>()
            .init_resource::<Inventory>()
            .init_resource::<QuestLog>()
            .add_systems(
                // `PreUpdate`: lê o snapshot do frame ANTERIOR (o Lua corre no
                // `Update`). Assim o HUD Rust, que publica antes do `UiSet::Bind`,
                // não cria um ciclo `Bind → luau_update → sync → HUD → Bind`.
                PreUpdate,
                sync_state_from_lua,
            );
    }
}

/// Snapshot periódico → [`VaelSave`] + componentes.
pub(crate) fn sync_state_from_lua(
    time: Res<Time<Real>>,
    // `Option`: um build com scripts desligados não tem host — sem ele o
    // estado fica intacto (nunca um pânico).
    host: Option<Res<LuaScriptHost>>,
    mut timer: Local<Option<Timer>>,
    mut logged: Local<bool>,
    mut vitals: ResMut<PlayerVitals>,
    mut xp: ResMut<XpState>,
    mut inventory: ResMut<Inventory>,
    mut quests: ResMut<QuestLog>,
    mut save: ResMut<VaelSave>,
) {
    let Some(host) = host else {
        return;
    };
    let timer =
        timer.get_or_insert_with(|| Timer::from_seconds(SYNC_PERIOD_SECS, TimerMode::Repeating));
    if !timer.tick(time.delta()).just_finished() {
        return;
    }
    let kv = viber::luau::game::game_to_json(&host.lua);
    if kv.is_empty() {
        // A tabela `viber_game` ainda não existe (arranque): não zerar estado.
        return;
    }
    apply_kv(
        &kv,
        &mut vitals,
        &mut xp,
        &mut inventory,
        &mut quests,
        &mut save,
    );
    if !*logged {
        *logged = true;
        info!(
            chaves = kv.len(),
            "vael: primeiro snapshot do estado Lua aplicado"
        );
    }
}

/// Aplica um snapshot do `viber.game()` ao save tipado e aos componentes.
///
/// Puro: não toca no Lua. As chaves que o schema não conhece (`venom`,
/// `flasks`, `q.<id>.n`, `dialogue.*`, …) ficam em [`VaelSave::legacy`]; um
/// snapshot vazio não altera nada.
pub fn apply_kv(
    kv: &BTreeMap<String, Value>,
    vitals: &mut PlayerVitals,
    xp: &mut XpState,
    inventory: &mut Inventory,
    quests: &mut QuestLog,
    save: &mut VaelSave,
) {
    if kv.is_empty() {
        return;
    }
    let parsed = VaelSave::from_world_kv(kv);
    *vitals = parsed.vitals.clone();
    *xp = parsed.progression.clone();
    *inventory = parsed.inventory.clone();
    *quests = parsed.quests.clone();
    *save = parsed;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn snapshot() -> BTreeMap<String, Value> {
        let mut kv = BTreeMap::new();
        kv.insert("hp".into(), json!(73));
        kv.insert("max_hp".into(), json!(100));
        kv.insert("xp".into(), json!(50));
        kv.insert("xp_next".into(), json!(100));
        kv.insert("level".into(), json!(4));
        kv.insert("skill_points".into(), json!(2));
        kv.insert("item.gold".into(), json!(12));
        kv.insert("item.wood".into(), json!(7));
        kv.insert("q.wolf".into(), json!("active"));
        kv.insert("tracked_quest".into(), json!("wolf"));
        kv.insert("story_prologo".into(), json!(true));
        // Desconhecidas do schema: têm de sobreviver em `legacy`.
        kv.insert("flasks".into(), json!(2));
        kv.insert("venom".into(), json!(1.5));
        kv.insert("q.wolf.n".into(), json!(3));
        kv.insert("mystery".into(), json!({"a": 1}));
        kv
    }

    fn aplica(
        kv: &BTreeMap<String, Value>,
    ) -> (PlayerVitals, XpState, Inventory, QuestLog, VaelSave) {
        let mut vitals = PlayerVitals::default();
        let mut xp = XpState::default();
        let mut inventory = Inventory::default();
        let mut quests = QuestLog::default();
        let mut save = VaelSave::default();
        apply_kv(
            kv,
            &mut vitals,
            &mut xp,
            &mut inventory,
            &mut quests,
            &mut save,
        );
        (vitals, xp, inventory, quests, save)
    }

    #[test]
    fn aplica_snapshot_nos_componentes_e_no_save() {
        let (vitals, xp, inventory, quests, save) = aplica(&snapshot());

        assert_eq!(vitals.hp, 73.0);
        assert_eq!(vitals.max_hp, 100.0);
        assert_eq!(xp.xp, 50);
        assert_eq!(xp.xp_next, 100);
        assert_eq!(xp.level, 4);
        assert_eq!(xp.skill_points, 2);
        assert_eq!(inventory.gold, 12);
        assert_eq!(inventory.wood, 7);
        assert_eq!(quests.states.get("wolf"), Some(&"active".to_string()));
        assert_eq!(quests.tracked.as_deref(), Some("wolf"));

        // O save é o espelho exato dos componentes nesta passagem.
        assert_eq!(save.vitals, vitals);
        assert_eq!(save.progression, xp);
        assert_eq!(save.inventory, inventory);
        assert_eq!(save.quests, quests);
        assert!(save.story.prologue);

        // Nada perdido: o progresso fino e os desconhecidos ficam em legacy.
        assert_eq!(save.legacy.get("flasks"), Some(&json!(2)));
        assert_eq!(save.legacy.get("venom"), Some(&json!(1.5)));
        assert_eq!(save.legacy.get("q.wolf.n"), Some(&json!(3)));
        assert_eq!(save.legacy.get("mystery"), Some(&json!({"a": 1})));
    }

    #[test]
    fn snapshot_vazio_nao_toca_no_estado() {
        let mut vitals = PlayerVitals {
            hp: 10.0,
            max_hp: 20.0,
        };
        let mut xp = XpState {
            xp: 5,
            ..Default::default()
        };
        let mut inventory = Inventory {
            gold: 3,
            ..Default::default()
        };
        let mut quests = QuestLog {
            tracked: Some("wolf".into()),
            ..Default::default()
        };
        let mut save = VaelSave {
            legacy: BTreeMap::from([("mystery".to_string(), json!(1))]),
            ..Default::default()
        };

        apply_kv(
            &BTreeMap::new(),
            &mut vitals,
            &mut xp,
            &mut inventory,
            &mut quests,
            &mut save,
        );

        assert_eq!(vitals.hp, 10.0);
        assert_eq!(vitals.max_hp, 20.0);
        assert_eq!(xp.xp, 5);
        assert_eq!(inventory.gold, 3);
        assert_eq!(quests.tracked.as_deref(), Some("wolf"));
        assert_eq!(save.legacy.get("mystery"), Some(&json!(1)));
    }

    #[test]
    fn snapshot_seguinte_substitui_o_anterior() {
        let mut kv = snapshot();
        kv.insert("item.gold".into(), json!(40));
        kv.insert("q.wolf".into(), json!("done"));
        let (_, _, inventory, quests, save) = aplica(&kv);

        assert_eq!(inventory.gold, 40);
        assert_eq!(quests.states.get("wolf"), Some(&"done".to_string()));
        assert!(quests.done.contains(&"wolf".to_string()));
        assert_eq!(save.inventory.gold, 40);
    }
}

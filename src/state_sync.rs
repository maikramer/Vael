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
use crate::rules_authority::{OwnedVitals, RUST_GATE_KEY, VaelRulesAuthority, VaelRulesConfig};
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
    // `Option`: apps mínimas de teste não têm a autoridade das regras.
    config: Option<Res<VaelRulesConfig>>,
    authority: Option<Res<VaelRulesAuthority>>,
    mut timer: Local<Option<Timer>>,
    mut logged: Local<bool>,
    // Último snapshot aplicado: entre mudanças reais não vale a pena
    // reconstruir o save nem marcar os recursos como `Changed`.
    mut last: Local<Option<BTreeMap<String, Value>>>,
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
    if last.as_ref() == Some(&kv) {
        return;
    }
    *last = Some(kv.clone());
    let rust = config.as_deref().is_some_and(|config| config.rust);
    ensure_rust_gate(&host, rust, &kv);
    // Com a autoridade ligada, os campos dela ganham ao snapshot Lua (que
    // pode estar um tick atrasado); XP/inventário/quests/legacy continuam a
    // vir do Lua.
    let owned = authority
        .as_deref()
        .filter(|_| rust)
        .filter(|auth| auth.seeded)
        .map(VaelRulesAuthority::owned);
    apply_kv_owned(
        &kv,
        &mut vitals,
        &mut xp,
        &mut inventory,
        &mut quests,
        &mut save,
        owned.as_ref(),
    );
    if !*logged {
        *logged = true;
        info!(
            chaves = kv.len(),
            "vael: primeiro snapshot do estado Lua aplicado"
        );
    }
}

/// Corrige o gate `vael_rust_vitals` quando o valor guardado no jogo não
/// corresponde ao modo desta sessão. Um save escrito com `VAEL_RULES_RUST=1`
/// traz `true`; sem esta correção o Lua saltava o tick portado num jogo
/// normal (`VAEL_RULES_RUST=0`).
fn ensure_rust_gate(host: &LuaScriptHost, rust: bool, kv: &BTreeMap<String, Value>) {
    let atual = kv.get(RUST_GATE_KEY);
    let mut patch = BTreeMap::new();
    if rust {
        if atual != Some(&Value::Bool(true)) {
            patch.insert(RUST_GATE_KEY.to_string(), Value::Bool(true));
        }
    } else if atual == Some(&Value::Bool(true)) {
        patch.insert(RUST_GATE_KEY.to_string(), Value::Null);
    }
    if !patch.is_empty() {
        viber::luau::game::apply_patch(&host.lua, &patch);
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
    // Não marcar `Changed` quando o valor é igual: o HUD e a UI reagem ao
    // tick de mudança e uma escrita incondicional a 10 Hz força trabalho em
    // todos os consumidores.
    if *vitals != parsed.vitals {
        *vitals = parsed.vitals.clone();
    }
    if *xp != parsed.progression {
        *xp = parsed.progression.clone();
    }
    if *inventory != parsed.inventory {
        *inventory = parsed.inventory.clone();
    }
    if *quests != parsed.quests {
        *quests = parsed.quests.clone();
    }
    if *save != parsed {
        *save = parsed;
    }
}

/// Aplica o snapshot do Lua e, quando a autoridade Rust está ligada, repõe
/// os campos que ela possui (vida, i-frames, estados e morte).
///
/// Puro: não toca no Lua. `owned = None` é o comportamento de sempre (o Lua é
/// a fonte de verdade). Com `Some`, XP/inventário/quests/`legacy` continuam a
/// vir do snapshot; só as vitais/status/flags de morte são do Rust.
pub fn apply_kv_owned(
    kv: &BTreeMap<String, Value>,
    vitals: &mut PlayerVitals,
    xp: &mut XpState,
    inventory: &mut Inventory,
    quests: &mut QuestLog,
    save: &mut VaelSave,
    owned: Option<&OwnedVitals>,
) {
    if kv.is_empty() {
        // Arranque (tabela ainda vazia): nem o snapshot nem o merge tocam.
        return;
    }
    apply_kv(kv, vitals, xp, inventory, quests, save);
    if let Some(owned) = owned {
        owned.write_state(vitals, save);
    }
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

    #[test]
    fn merge_da_autoridade_preserva_vitals_e_sincroniza_o_resto() {
        let mut vitals = PlayerVitals::default();
        let mut xp = XpState::default();
        let mut inventory = Inventory::default();
        let mut quests = QuestLog::default();
        let mut save = VaelSave::default();

        // O snapshot Lua traz hp 73/veneno 1,5 (um tick atrasado); o estado
        // autoritativo já vai em hp 40/veneno 3/morte pendente.
        let owned = OwnedVitals {
            hp: 40.0,
            max_hp: 100.0,
            iframes: 0.35,
            venom: 3.0,
            burn: 0.0,
            frost: 0.0,
            venom_tick: 0.4,
            burn_tick: 0.0,
            dying: 0.0,
            death_pending: true,
            death_wait: 5.0,
        };
        apply_kv_owned(
            &snapshot(),
            &mut vitals,
            &mut xp,
            &mut inventory,
            &mut quests,
            &mut save,
            Some(&owned),
        );

        // Rust: vitais/status/flags de morte.
        assert_eq!(vitals.hp, 40.0);
        assert_eq!(save.vitals.hp, 40.0);
        assert_eq!(save.legacy.get("venom"), Some(&json!(3.0)));
        let iframes = save.legacy["iframes"].as_f64().unwrap();
        assert!((iframes - 0.35).abs() < 1e-6, "iframes={iframes}");
        assert_eq!(save.legacy.get("death_pending"), Some(&json!(true)));
        assert_eq!(save.legacy.get("death_wait"), Some(&json!(5.0)));
        // Lua: XP, inventário e quests continuam a entrar.
        assert_eq!(xp.xp, 50);
        assert_eq!(xp.level, 4);
        assert_eq!(inventory.gold, 12);
        assert_eq!(inventory.wood, 7);
        assert_eq!(quests.states.get("wolf"), Some(&"active".to_string()));
        assert_eq!(save.progression.xp, 50);
        assert_eq!(save.inventory.gold, 12);
        // Outras chaves desconhecidas continuam em legacy.
        assert_eq!(save.legacy.get("flasks"), Some(&json!(2)));
    }

    #[test]
    fn merge_sem_autoridade_e_o_snapshot_de_sempre() {
        let mut vitals = PlayerVitals::default();
        let mut xp = XpState::default();
        let mut inventory = Inventory::default();
        let mut quests = QuestLog::default();
        let mut save = VaelSave::default();
        apply_kv_owned(
            &snapshot(),
            &mut vitals,
            &mut xp,
            &mut inventory,
            &mut quests,
            &mut save,
            None,
        );
        assert_eq!(vitals.hp, 73.0);
        assert_eq!(save.legacy.get("venom"), Some(&json!(1.5)));
    }

    #[test]
    fn gate_corrige_um_save_escrito_com_a_autoridade_ligada() {
        let host = LuaScriptHost::new(std::path::PathBuf::from("scripts")).expect("host Luau");
        let mut kv = BTreeMap::new();
        kv.insert(RUST_GATE_KEY.into(), json!(true));

        // Jogo normal: o gate que veio do save tem de sair.
        ensure_rust_gate(&host, false, &kv);
        let kv = viber::luau::game::game_to_json(&host.lua);
        assert!(
            !kv.contains_key(RUST_GATE_KEY),
            "VAEL_RULES_RUST=0 tem de devolver o tick ao Lua"
        );

        // Autoridade ligada: volta a escrevê-lo (uma vez, depois é no-op).
        ensure_rust_gate(&host, true, &kv);
        let kv = viber::luau::game::game_to_json(&host.lua);
        assert_eq!(kv.get(RUST_GATE_KEY), Some(&json!(true)));
        ensure_rust_gate(&host, true, &kv);
    }
}

//! Autoridade Rust das vitais/status/morte (default; `VAEL_RULES_RUST=0`
//! devolve o tick ao Lua).
//!
//! A partir do momento em que o flag está ligado, o tick por frame de vida,
//! i-frames, estados e morte é deste módulo: lê o KV do `viber.game()` DEPOIS
//! de `luau_update` (apanha o dano/cura do Lua no mesmo frame), simula com o
//! [`shadow_tick`] do [`crate::rules_shadow`] e escreve de volta só as chaves
//! que possui. O Lua continua a ser dono do dano/cura (`rules.damage_player`/
//! `heal_player`), da UI de morte, do `rules.respawn` manual, do XP, do
//! inventário, das quests e da bolsa.
//!
//! Ordem por frame (Update):
//! `… luau_on_add → luau_update (scripts + rules.tick com as secções portadas
//! desligadas) → vael_rules_authority_system → …`.
//!
//! Limites conhecidos (o shadow já os tinha):
//! - O respawn de fallback (sem modal de morte) repõe as vitais mas NÃO
//!   teleporta o herói, não reabastece frascos e não toca na bolsa: isso é o
//!   `rules.respawn` do Lua, que o ecrã de morte chama no "Tentar de novo".
//! - Uma morte por DoT não toca o som `game_over` nem larga a bolsa (essas
//!   acções vivem no `start_death` do Lua, que só o dano direto corre).
//! - O `M.god` do `devmode.lua` chega via `viber.game().vael_god` (o Lua
//!   espelha-o quando a autoridade está ligada); sem esse espelho o god do
//!   Lua não seria visível daqui.
//!
//! A autoridade é o **default** desde a Onda 4a; `VAEL_RULES_RUST=0`
//! (ou `false`/`off`/`no`) mantém o tick no Lua, como antes. O shadow
//! (`VAEL_RULES_SHADOW=1`) permanece independente.

use std::collections::BTreeMap;

use bevy::prelude::*;
use serde_json::Value;
use viber::luau::game::{apply_patch, game_to_json};
use viber::luau::host::LuaScriptHost;
use bevy_ext_core::MenusOpen;
use viber::scene_loader::CurrentScene;
use viber::ui::modal::UiModalsOpen;
use viber::worldsys::WeatherState;

use crate::components::PlayerVitals;
use crate::rules_shadow::{RAIN_HEAVY, VaelRulesShadow, seed_from_kv, shadow_tick};
use crate::save::VaelSave;

/// Variável de ambiente que liga a autoridade (`1`/`true`/`on`/`yes`).
pub const RUST_RULES_ENV: &str = "VAEL_RULES_RUST";
/// Chave do `viber.game()` que avisa o Lua de que o tick é do Rust.
pub const RUST_GATE_KEY: &str = "vael_rust_vitals";
/// Chave do `viber.game()` com o god do `devmode.lua` (F2).
pub const GOD_KEY: &str = "vael_god";

/// Gate da autoridade, lido do ambiente no arranque.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct VaelRulesConfig {
    pub rust: bool,
}

impl VaelRulesConfig {
    /// Constrói o gate a partir do valor bruto da variável (`None` = ausente).
    pub fn from_raw(raw: Option<&str>) -> Self {
        Self {
            rust: rust_enabled(raw),
        }
    }

    /// Lê `VAEL_RULES_RUST` do ambiente.
    pub fn from_env() -> Self {
        Self::from_raw(std::env::var(RUST_RULES_ENV).ok().as_deref())
    }
}

impl Default for VaelRulesConfig {
    /// Ligada por omissão; a env só a desliga explicitamente.
    fn default() -> Self {
        Self::from_env()
    }
}

/// Estado autoritativo + o último patch escrito no Lua.
#[derive(Resource, Debug, Clone, Default)]
pub struct VaelRulesAuthority {
    /// O estado simulado. Reutiliza o tipo do shadow para não duplicar o
    /// [`shadow_tick`].
    state: VaelRulesShadow,
    /// O primeiro tick já correu; até lá o `state_sync` não faz merge.
    pub seeded: bool,
    /// Último patch aplicado (não se reescreve o KV sem mudanças).
    last_patch: Option<BTreeMap<String, Value>>,
}

impl VaelRulesAuthority {
    /// Copia o KV do Lua para o estado (dano/cura/itens/respawn do frame).
    pub fn seed(&mut self, kv: &BTreeMap<String, Value>) {
        seed_from_kv(&mut self.state, kv);
        self.seeded = true;
    }

    /// Simula um frame com a mesma função do shadow.
    pub fn tick(&mut self, dt: f32, rain: bool, frozen: bool, death_open: bool) {
        shadow_tick(&mut self.state, dt, rain, frozen, death_open);
    }

    /// Ticks aplicados (contador do shadow).
    pub fn ticks(&self) -> u64 {
        self.state.ticks
    }

    /// O último tick chamou o respawn de fallback (sem modal de morte).
    pub fn respawned(&self) -> bool {
        self.state.respawned
    }

    /// Os campos que o Rust possui, para o save/componentes.
    pub fn owned(&self) -> OwnedVitals {
        OwnedVitals {
            hp: self.state.hp,
            max_hp: self.state.max_hp,
            iframes: self.state.iframes,
            venom: self.state.venom,
            burn: self.state.burn,
            frost: self.state.frost,
            venom_tick: self.state.venom_tick,
            burn_tick: self.state.burn_tick,
            dying: self.state.dying,
            death_pending: self.state.death_pending,
            death_wait: self.state.death_wait,
        }
    }

    /// Patch completo do `viber.game()` (só as chaves Rust + o gate).
    pub fn patch(&self) -> BTreeMap<String, Value> {
        let mut patch = BTreeMap::new();
        patch.insert("hp".to_string(), json_num(self.state.hp));
        patch.insert("iframes".to_string(), json_num(self.state.iframes));
        patch.insert("venom".to_string(), json_num(self.state.venom));
        patch.insert("burn".to_string(), json_num(self.state.burn));
        patch.insert("frost".to_string(), json_num(self.state.frost));
        patch.insert("venom_tick".to_string(), json_num(self.state.venom_tick));
        patch.insert("burn_tick".to_string(), json_num(self.state.burn_tick));
        patch.insert("dying".to_string(), json_num(self.state.dying));
        patch.insert(
            "death_pending".to_string(),
            Value::Bool(self.state.death_pending),
        );
        patch.insert("death_wait".to_string(), json_num(self.state.death_wait));
        // A flag que desliga as secções portadas do `rules.tick`.
        patch.insert(RUST_GATE_KEY.to_string(), Value::Bool(true));
        patch
    }

    /// Patch a aplicar, ou `None` se o estado não mudou desde a última escrita.
    pub fn pending_patch(&mut self) -> Option<BTreeMap<String, Value>> {
        let patch = self.patch();
        if self.last_patch.as_ref() == Some(&patch) {
            return None;
        }
        self.last_patch = Some(patch.clone());
        Some(patch)
    }
}

/// Os campos vitais cujo dono é o Rust quando a autoridade está ligada.
///
/// `max_hp` entra para o [`PlayerVitals`]/`VaelSave` ficarem coerentes, mas
/// NÃO é escrito no Lua: as passivas (`sync_passives`) continuam a ser do Lua.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OwnedVitals {
    pub hp: f32,
    pub max_hp: f32,
    pub iframes: f32,
    pub venom: f32,
    pub burn: f32,
    pub frost: f32,
    pub venom_tick: f32,
    pub burn_tick: f32,
    pub dying: f32,
    pub death_pending: bool,
    pub death_wait: f32,
}

impl OwnedVitals {
    /// Repõe os campos do Rust no save tipado e no `legacy` que o HUD lê.
    /// O resto do save (XP, itens, quests, `max_hp` do Lua já aplicado) fica.
    pub fn write_state(&self, vitals: &mut PlayerVitals, save: &mut VaelSave) {
        vitals.hp = self.hp;
        vitals.max_hp = self.max_hp;
        save.vitals = vitals.clone();
        save.legacy.insert("iframes".into(), json_num(self.iframes));
        save.legacy.insert("venom".into(), json_num(self.venom));
        save.legacy.insert("burn".into(), json_num(self.burn));
        save.legacy.insert("frost".into(), json_num(self.frost));
        save.legacy
            .insert("venom_tick".into(), json_num(self.venom_tick));
        save.legacy
            .insert("burn_tick".into(), json_num(self.burn_tick));
        save.legacy.insert("dying".into(), json_num(self.dying));
        save.legacy
            .insert("death_pending".into(), Value::Bool(self.death_pending));
        save.legacy
            .insert("death_wait".into(), json_num(self.death_wait));
    }
}

/// Conjunto dos sistemas da autoridade. A UI ordena os seus sistemas de
/// morte/respawn depois deste set para ler o estado já deste frame.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct VaelRulesSet;

/// Plugin da autoridade Rust.
pub struct VaelRulesAuthorityPlugin;

impl Plugin for VaelRulesAuthorityPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VaelRulesConfig>()
            .init_resource::<VaelRulesAuthority>()
            .init_resource::<crate::respawn::RespawnRequest>()
            // O Lua lê `vael_rust_vitals` no `rules.tick`; tem de existir
            // antes do primeiro `luau_update`.
            .add_systems(Startup, apply_rust_gate)
            .add_systems(
                Update,
                rules_authority_system
                    .in_set(VaelRulesSet)
                    // O tick LÊ o que o Lua acabou de fazer no frame e escreve
                    // as vitais de volta para o `viber.game()`.
                    .after(viber::luau::luau_update)
                    .run_if(viber::app::scripts_enabled)
                    .run_if(viber::scene_loader::scene_settled),
            );
    }
}

/// Arranque: liga o gate no `viber.game()` para o Lua saltar as secções
/// portadas. Desligado, não escreve nada (o jogo fica igual).
fn apply_rust_gate(config: Res<VaelRulesConfig>, host: Option<Res<LuaScriptHost>>) {
    if !config.rust {
        return;
    }
    let Some(host) = host else {
        warn!("vael: VAEL_RULES_RUST sem host Luau — autoridade inativa");
        return;
    };
    let mut patch = BTreeMap::new();
    patch.insert(RUST_GATE_KEY.to_string(), Value::Bool(true));
    apply_patch(&host.lua, &patch);
    info!(
        gate = RUST_GATE_KEY,
        "vael: autoridade Rust das vitais/status/morte ligada"
    );
}

/// Tick autoritativo: semente do KV do Lua → `shadow_tick` → patch de volta.
fn rules_authority_system(
    time: Res<Time>,
    config: Res<VaelRulesConfig>,
    host: Option<Res<LuaScriptHost>>,
    weather: Option<Res<WeatherState>>,
    menus: Option<Res<MenusOpen>>,
    modals: Option<Res<UiModalsOpen>>,
    scene: Option<Res<CurrentScene>>,
    vitals: Option<ResMut<PlayerVitals>>,
    save: Option<ResMut<VaelSave>>,
    mut authority: ResMut<VaelRulesAuthority>,
    mut respawn: ResMut<crate::respawn::RespawnRequest>,
) {
    if !config.rust {
        return;
    }
    let Some(host) = host else {
        return;
    };

    // Dano/cura/`respawn`/passivas do Lua no frame que acabou de correr.
    let kv = game_to_json(&host.lua);
    authority.seed(&kv);

    // O `rules_host` não corre `rules.tick` com o título/menu por cima (o
    // mundo atrás é decoração): a autoridade também não avança — senão uma
    // morte cancelada no "Ecrã de título" rearmava por baixo do véu.
    if !ui_owns_frame(scene.as_deref(), modals.as_deref()) {
        let rain = weather
            .as_deref()
            .is_some_and(|w| w.rain >= RAIN_HEAVY as f32);
        // `frozen` é o portão dos estados: menu aberto ou god do DEV.
        let frozen = menus.as_deref().is_some_and(MenusOpen::any) || kv_flag(&kv, GOD_KEY);
        let death_open = modals.as_deref().is_some_and(|m| m.is_open("death"));
        authority.tick(time.delta_secs(), rain, frozen, death_open);
        if authority.respawned() {
            // O patch de vitais já vai a seguir; a UI consome este pedido para
            // fazer o que o `M.respawn` fazia e o Rust não possui: teleporte,
            // frascos e troca de cena quando a morte aconteceu num interior.
            respawn.active = true;
            respawn.spawn = spawn_from_kv(&kv).unwrap_or([0.0, 150.0, 0.0]);
        }
    }

    // Só as chaves que o Rust possui; `xp`, `item.*`, `q.*`, ouro, etc. não
    // entram no patch.
    if let Some(patch) = authority.pending_patch() {
        apply_patch(&host.lua, &patch);
    }

    // HUD/save: o `state_sync` preserva estes campos no merge.
    if let (Some(mut vitals), Some(mut save)) = (vitals, save) {
        authority.owned().write_state(&mut vitals, &mut save);
    }
}

/// Título/menu por cima: o frame é da UI. Espelha o `ui_owns_frame` do
/// `scripts/game/rules_host.lua` (cena `menu` ou modal `title` aberto).
fn ui_owns_frame(scene: Option<&CurrentScene>, modals: Option<&UiModalsOpen>) -> bool {
    scene.is_some_and(|scene| scene.id.as_deref() == Some("menu"))
        || modals.is_some_and(|modals| modals.is_open("title"))
}

/// Posição de renascimento no KV plano (a mesma que o `stored_spawn` do Lua).
fn spawn_from_kv(kv: &BTreeMap<String, Value>) -> Option<[f32; 3]> {
    let value = |key: &str| {
        kv.get(key)
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite())
            .map(|value| value as f32)
    };
    Some([value("spawn_x")?, value("spawn_y")?, value("spawn_z")?])
}

/// `vael_god`: `true` booleano ou número diferente de zero.
fn kv_flag(kv: &BTreeMap<String, Value>, key: &str) -> bool {
    match kv.get(key) {
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|value| value != 0.0),
        _ => false,
    }
}

/// Número JSON finito (um NaN/inf nunca chega ao Lua nem ao save).
fn json_num(value: f32) -> Value {
    let value = if value.is_finite() { value } else { 0.0 };
    serde_json::Number::from_f64(value as f64)
        .map(Value::Number)
        .unwrap_or_else(|| Value::Number(0.into()))
}

/// Gate `VAEL_RULES_RUST`: ligado por omissão, desligado só com um valor
/// explícito de off (`0`/`false`/`off`/`no`).
fn rust_enabled(raw: Option<&str>) -> bool {
    !matches!(
        raw.map(|value| value.trim().to_ascii_lowercase())
            .as_deref(),
        Some("0" | "false" | "off" | "no")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn kv(vals: &[(&str, Value)]) -> BTreeMap<String, Value> {
        vals.iter()
            .map(|(key, value)| (key.to_string(), value.clone()))
            .collect()
    }

    #[test]
    fn gate_por_variavel_de_ambiente() {
        assert!(rust_enabled(None));
        assert!(rust_enabled(Some("")));
        assert!(!rust_enabled(Some("0")));
        for on in ["1", "true", "on", "yes", " TRUE ", "On"] {
            assert!(rust_enabled(Some(on)), "devia ligar: {on}");
        }
        for off in ["0", "false", "off", "no", "OFF", " False "] {
            assert!(!rust_enabled(Some(off)), "devia desligar: {off}");
        }
        // Sem variável (None) o gate fica LIGADO (default da Onda 4a).
        assert!(VaelRulesConfig::from_raw(None).rust);
        assert!(VaelRulesConfig::from_raw(Some("1")).rust);
        assert!(!VaelRulesConfig::from_raw(Some("0")).rust);
    }

    #[test]
    fn default_le_a_variavel_de_ambiente() {
        let raw = std::env::var(RUST_RULES_ENV).ok();
        assert_eq!(
            VaelRulesConfig::default().rust,
            rust_enabled(raw.as_deref()),
            "o default tem de ler VAEL_RULES_RUST"
        );
        if std::env::var_os(RUST_RULES_ENV).is_none() {
            assert!(
                VaelRulesConfig::default().rust,
                "sem a variável a autoridade fica ligada"
            );
        }
    }

    #[test]
    fn spawn_do_kv_alimenta_o_pedido_de_respawn() {
        let kv = kv(&[
            ("spawn_x", json!(1.5)),
            ("spawn_y", json!(2.5)),
            ("spawn_z", json!(3.5)),
        ]);
        assert_eq!(spawn_from_kv(&kv), Some([1.5, 2.5, 3.5]));
        assert_eq!(
            spawn_from_kv(&BTreeMap::new()),
            None,
            "sem spawn, UI usa o default"
        );
    }

    #[test]
    fn veneno_100_para_92_e_patch_so_com_as_chaves_rust() {
        let mut auth = VaelRulesAuthority::default();
        auth.seed(&kv(&[
            ("hp", json!(100)),
            ("max_hp", json!(100)),
            ("venom", json!(10)),
        ]));
        auth.tick(1.0, false, false, false);
        assert!((auth.owned().hp - 96.0).abs() < 1e-4);

        auth.tick(1.0, false, false, false);
        let owned = auth.owned();
        assert!((owned.hp - 92.0).abs() < 1e-4, "hp={}", owned.hp);
        assert!((owned.venom - 8.0).abs() < 1e-4, "venom={}", owned.venom);
        assert_eq!(auth.ticks(), 2);

        let patch = auth.patch();
        let mut keys: Vec<&str> = patch.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "burn",
                "burn_tick",
                "death_pending",
                "death_wait",
                "dying",
                "frost",
                "hp",
                "iframes",
                "vael_rust_vitals",
                "venom",
                "venom_tick",
            ]
        );
        assert_eq!(patch.get("hp"), Some(&json!(92.0)));
        assert_eq!(patch.get("venom"), Some(&json!(8.0)));
        assert_eq!(patch.get("vael_rust_vitals"), Some(&json!(true)));
        // `max_hp` é do Lua (passivas): nunca entra no patch.
        assert!(!patch.contains_key("max_hp"));
    }

    #[test]
    fn morte_flag_e_respawn_no_patch() {
        let mut auth = VaelRulesAuthority::default();
        auth.seed(&kv(&[
            ("hp", json!(0)),
            ("max_hp", json!(100)),
            ("venom", json!(30)),
            ("burn", json!(30)),
            ("frost", json!(30)),
        ]));

        // O tick que inicia a morte desconta o próprio `dt` do `dying`.
        auth.tick(0.1, false, false, false);
        assert!((auth.owned().dying - 2.1).abs() < 1e-4);

        auth.tick(2.2, false, false, false);
        let patch = auth.patch();
        assert_eq!(patch.get("death_pending"), Some(&json!(true)));
        assert_eq!(patch.get("hp"), Some(&json!(0.0)));
        assert_eq!(patch.get("death_wait"), Some(&json!(5.0)));

        // Sem o modal aberto o fallback conta até ao respawn.
        auth.tick(4.0, false, false, false);
        assert_eq!(auth.patch().get("death_wait"), Some(&json!(1.0)));
        assert_eq!(auth.patch().get("death_pending"), Some(&json!(true)));

        auth.tick(1.0, false, false, false);
        let patch = auth.patch();
        assert_eq!(patch.get("hp"), Some(&json!(100.0)));
        assert_eq!(patch.get("death_pending"), Some(&json!(false)));
        assert_eq!(patch.get("death_wait"), Some(&json!(0.0)));
        assert_eq!(patch.get("venom"), Some(&json!(0.0)));
        assert_eq!(patch.get("burn"), Some(&json!(0.0)));
        assert_eq!(patch.get("frost"), Some(&json!(0.0)));
        assert_eq!(patch.get("iframes"), Some(&json!(2.0)));
    }

    #[test]
    fn semeia_as_mutacoes_do_lua_antes_do_tick() {
        let mut auth = VaelRulesAuthority::default();
        auth.seed(&kv(&[
            ("hp", json!(100)),
            ("max_hp", json!(100)),
            ("iframes", json!(0)),
        ]));
        auth.tick(0.1, false, false, false);
        assert!((auth.owned().iframes - 0.0).abs() < 1e-4);

        // `damage_player` do Lua: hp 60, i-frames 0.35, veneno 3.
        auth.seed(&kv(&[
            ("hp", json!(60)),
            ("max_hp", json!(100)),
            ("iframes", json!(0.35)),
            ("venom", json!(3)),
        ]));
        assert!((auth.owned().hp - 60.0).abs() < 1e-4);
        auth.tick(0.1, false, false, false);
        let owned = auth.owned();
        assert!((owned.iframes - 0.25).abs() < 1e-4, "{}", owned.iframes);
        assert!((owned.venom - 2.9).abs() < 1e-4, "{}", owned.venom);
    }

    #[test]
    fn god_congela_os_estados_mas_nao_a_morte() {
        let mut auth = VaelRulesAuthority::default();
        auth.seed(&kv(&[
            ("hp", json!(100)),
            ("max_hp", json!(100)),
            ("venom", json!(10)),
            ("vael_god", json!(true)),
        ]));
        auth.tick(1.0, false, true, false);
        let owned = auth.owned();
        assert!((owned.venom - 10.0).abs() < 1e-4, "god congela o DoT");
        assert!((owned.hp - 100.0).abs() < 1e-4);
    }

    #[test]
    fn write_state_repoe_vitals_e_legacy_sem_tocar_no_resto() {
        let mut vitals = PlayerVitals::default();
        let mut save = VaelSave::default();
        save.progression.xp = 50;
        save.inventory.gold = 12;

        let owned = OwnedVitals {
            hp: 40.0,
            max_hp: 120.0,
            iframes: 0.35,
            venom: 3.0,
            burn: 0.0,
            frost: 1.5,
            venom_tick: 0.4,
            burn_tick: 0.0,
            dying: 2.2,
            death_pending: false,
            death_wait: 0.0,
        };
        owned.write_state(&mut vitals, &mut save);

        assert_eq!(vitals.hp, 40.0);
        assert_eq!(vitals.max_hp, 120.0);
        assert_eq!(save.vitals, vitals);
        assert_eq!(save.legacy.get("venom"), Some(&json!(3.0)));
        assert_eq!(save.legacy.get("frost"), Some(&json!(1.5)));
        let dying = save.legacy["dying"].as_f64().unwrap();
        assert!((dying - 2.2).abs() < 1e-6, "dying={dying}");
        assert_eq!(save.legacy.get("death_pending"), Some(&json!(false)));
        // XP e inventário não são tocados.
        assert_eq!(save.progression.xp, 50);
        assert_eq!(save.inventory.gold, 12);
    }

    #[test]
    fn pending_patch_nao_reescreve_sem_mudanca() {
        let mut auth = VaelRulesAuthority::default();
        auth.seed(&kv(&[
            ("hp", json!(100)),
            ("max_hp", json!(100)),
            ("iframes", json!(1.0)),
        ]));
        assert!(auth.pending_patch().is_some(), "primeiro patch");
        assert!(auth.pending_patch().is_none(), "sem mudança não reescreve");
        auth.tick(1.0, false, false, false);
        assert!(auth.pending_patch().is_some(), "iframes desceram no tick");
    }

    #[test]
    fn plugin_sem_host_nao_panica() {
        let mut app = App::new();
        app.add_plugins(bevy::MinimalPlugins)
            .init_resource::<PlayerVitals>()
            .init_resource::<VaelSave>()
            .add_plugins(VaelRulesAuthorityPlugin);
        app.insert_resource(VaelRulesConfig { rust: true });
        for _ in 0..3 {
            app.update();
        }
        let authority = app.world().resource::<VaelRulesAuthority>();
        assert!(!authority.seeded, "sem host não semeia");
        assert_eq!(authority.ticks(), 0);
    }

    fn cena(id: &str) -> CurrentScene {
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

    #[test]
    fn ui_owns_frame_com_titulo_ou_cena_menu() {
        let menu = cena("menu");
        let main = cena("main");
        let mut modals = UiModalsOpen::default();

        assert!(ui_owns_frame(Some(&menu), Some(&modals)));
        assert!(!ui_owns_frame(Some(&main), Some(&modals)));
        modals.open.push("title".into());
        assert!(ui_owns_frame(Some(&main), Some(&modals)));
        modals.open.clear();
        modals.open.push("death".into());
        assert!(!ui_owns_frame(Some(&main), Some(&modals)));
        assert!(
            !ui_owns_frame(None, None),
            "mundo avulso (QA) continua a tickar"
        );
    }

    #[test]
    fn autoridade_nao_avanca_com_o_titulo_por_cima() {
        let mut app = App::new();
        app.add_plugins(bevy::MinimalPlugins)
            .insert_resource(VaelRulesConfig { rust: true })
            .insert_resource(LuaScriptHost::new(std::path::PathBuf::from("scripts")).expect("host"))
            .insert_resource(cena("menu"))
            .init_resource::<UiModalsOpen>()
            .init_resource::<PlayerVitals>()
            .init_resource::<VaelSave>()
            .add_plugins(VaelRulesAuthorityPlugin);

        app.update();
        let authority = app.world().resource::<VaelRulesAuthority>();
        assert!(authority.seeded, "semeia o KV mesmo sem tickar");
        assert_eq!(authority.ticks(), 0, "menu: o frame é da UI");

        // De volta a `main`: o tick autoritativo volta a correr.
        *app.world_mut().resource_mut::<CurrentScene>() = cena("main");
        app.update();
        assert_eq!(app.world().resource::<VaelRulesAuthority>().ticks(), 1);
    }
}

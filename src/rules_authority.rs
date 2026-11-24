//! Autoridade Rust das vitais/status/morte (default; `VAEL_RULES_RUST=0`
//! devolve o tick ao Lua).
//!
//! A partir do momento em que o flag está ligado, o tick por frame de vida,
//! i-frames, estados e morte é deste módulo: lê o KV do `vael.game()` DEPOIS
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
//! - Uma morte armada pelo tick (DoT, ou um save com `hp` 0) não corre o
//!   `start_death` do Lua: a largada da bolsa e o som `game_over` são de outro
//!   dono. Para o caminho Rust os ver, o tick emite [`VaelDeathStarted`] no
//!   frame em que arma a morte; quem trata das consequências
//!   (`respawn.rs`/cofre) lê a mensagem e faz o mesmo que o `start_death`.
//! - O `M.god` do `devmode.lua` chega via `vael.game().vael_god` (o Lua
//!   espelha-o quando a autoridade está ligada); sem esse espelho o god do
//!   Lua não seria visível daqui.
//!
//! A autoridade é o **default** desde a Onda 4a; `VAEL_RULES_RUST=0`
//! (ou `false`/`off`/`no`) mantém o tick no Lua, como antes. O shadow
//! (`VAEL_RULES_SHADOW=1`) permanece independente.

use std::collections::BTreeMap;

use bevy::prelude::*;
use bevy_ext_core::MenusOpen;
use serde_json::Value;
use vael_engine::luau::game::{apply_patch_json, game_to_json_process};
use vael_engine::scene_loader::CurrentScene;
use vael_engine::ui::modal::UiModalsOpen;
use vael_engine::worldsys::WeatherState;

use crate::components::PlayerVitals;
use crate::content::VaelContent;
use crate::rules_shadow::{RAIN_HEAVY, StatusResists, VaelRulesShadow, seed_from_kv, shadow_tick};
use crate::save::VaelSave;

/// Variável de ambiente que liga a autoridade (`1`/`true`/`on`/`yes`).
pub const RUST_RULES_ENV: &str = "VAEL_RULES_RUST";
/// Chave do `vael.game()` que avisa o Lua de que o tick é do Rust.
pub const RUST_GATE_KEY: &str = "vael_rust_vitals";
/// Chave do `vael.game()` com o god do `devmode.lua` (F2).
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
    pub fn tick(
        &mut self,
        dt: f32,
        rain: bool,
        frozen: bool,
        death_open: bool,
        resists: StatusResists,
    ) {
        shadow_tick(&mut self.state, dt, rain, frozen, death_open, resists);
    }

    /// Há estados com DPS activos? Só nesse caso as resistências interessam
    /// (e só nesse caso o tick lhes toca).
    pub fn has_status_dot(&self) -> bool {
        self.state.venom > 0.0 || self.state.burn > 0.0
    }

    /// Ticks aplicados (contador do shadow).
    pub fn ticks(&self) -> u64 {
        self.state.ticks
    }

    /// O último tick chamou o respawn de fallback (sem modal de morte).
    pub fn respawned(&self) -> bool {
        self.state.respawned
    }

    /// O último tick armou uma morte (`start_death`) — ver
    /// [`VaelDeathStarted`]. Não cobre o dano direto: esse arma a morte no
    /// `start_death` do Lua antes de o tick correr.
    pub fn death_started(&self) -> bool {
        self.state.death_started
    }

    /// A morte armada pelo tick veio do DoT (veneno/queimadura).
    pub fn death_from_dot(&self) -> bool {
        self.state.death_from_dot
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

    /// Patch completo do `vael.game()` (só as chaves Rust + o gate).
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
    /// Os recursos já têm este estado?
    ///
    /// Compara antes de os escrever: um `ResMut` tocado marca o recurso como
    /// `Changed` e o HUD (`publish_hud_binds`) re-publicava os binds todos os
    /// frames — o `state_sync` evita a mesma armadilha a 10 Hz.
    pub fn matches(&self, vitals: &PlayerVitals, save: &VaelSave) -> bool {
        let legacy = &save.legacy;
        vitals.hp == self.hp
            && vitals.max_hp == self.max_hp
            && save.vitals.hp == self.hp
            && save.vitals.max_hp == self.max_hp
            && legacy_num_is(legacy, "iframes", self.iframes)
            && legacy_num_is(legacy, "venom", self.venom)
            && legacy_num_is(legacy, "burn", self.burn)
            && legacy_num_is(legacy, "frost", self.frost)
            && legacy_num_is(legacy, "venom_tick", self.venom_tick)
            && legacy_num_is(legacy, "burn_tick", self.burn_tick)
            && legacy_num_is(legacy, "dying", self.dying)
            && legacy_num_is(legacy, "death_wait", self.death_wait)
            && matches!(
                legacy.get("death_pending"),
                Some(Value::Bool(flag)) if *flag == self.death_pending
            )
    }

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

/// Mensagem: o tick da autoridade armou uma morte neste frame.
///
/// Handoff para o caminho Rust das consequências (largar a bolsa e tocar o
/// som `game_over`): com a autoridade ligada, o `start_death` do Lua não corre
/// no DoT nem num save carregado a 0 HP, por isso sem este sinal essas
/// mortes ficavam sem bolsa/som. A mensagem é emitida apenas na transição —
/// um tick por morte, nunca por frame.
///
/// **Não ligar dois consumidores.** O `src/respawn.rs`
/// (`death_started_in_rust`) deteta a mesma transição comparando o `dying`/
/// `death_pending` do KV antes e depois deste set; quem consumir esta
/// mensagem tem de deixar de lado essa comparação (senão a bolsa cai duas
/// vezes).
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct VaelDeathStarted {
    /// A morte veio do dano ao longo do tempo (veneno/queimadura); `false`
    /// para o `hp` 0 de um save carregado.
    pub from_dot: bool,
}

/// Plugin da autoridade Rust.
pub struct VaelRulesAuthorityPlugin;

impl Plugin for VaelRulesAuthorityPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VaelRulesConfig>()
            .init_resource::<VaelRulesAuthority>()
            .init_resource::<crate::respawn::RespawnRequest>()
            // Handoff da morte armada pelo tick (DoT/save a 0 HP): o caminho
            // Rust das consequências lê esta mensagem.
            .add_message::<VaelDeathStarted>()
            // O Lua lê `vael_rust_vitals` no `rules.tick`; tem de existir
            // antes do primeiro `luau_update`.
            .add_systems(Startup, apply_rust_gate)
            .add_systems(
                Update,
                rules_authority_system
                    .in_set(VaelRulesSet)
                    // O tick LÊ o que o Lua acabou de fazer no frame e escreve
                    // as vitais de volta para o `vael.game()`.
                    .after(vael_engine::luau::bms_bridge::apply_bms_commands)
                    .run_if(vael_engine::app::scripts_enabled)
                    .run_if(vael_engine::scene_loader::scene_settled),
            );
    }
}

/// Arranque: liga o gate no `vael.game()` para o Lua saltar as secções
/// portadas. Desligado, não escreve nada (o jogo fica igual).
fn apply_rust_gate(config: Res<VaelRulesConfig>) {
    if !config.rust {
        return;
    }
    // O gate vive no STORE do processo: o Lua de qualquer contexto lê-o em
    // `vael.game()` (o store é o proxy de sempre).
    let mut patch = BTreeMap::new();
    patch.insert(RUST_GATE_KEY.to_string(), Value::Bool(true));
    vael_engine::luau::game::apply_patch_json(&patch);
    info!(
        gate = RUST_GATE_KEY,
        "vael: autoridade Rust das vitais/status/morte ligada"
    );
}

/// Tick autoritativo: semente do KV do Lua → `shadow_tick` → patch de volta.
fn rules_authority_system(
    time: Res<Time>,
    config: Res<VaelRulesConfig>,
    content: Option<Res<VaelContent>>,
    weather: Option<Res<WeatherState>>,
    menus: Option<Res<MenusOpen>>,
    modals: Option<Res<UiModalsOpen>>,
    scene: Option<Res<CurrentScene>>,
    vitals: Option<ResMut<PlayerVitals>>,
    save: Option<ResMut<VaelSave>>,
    mut authority: ResMut<VaelRulesAuthority>,
    mut respawn: ResMut<crate::respawn::RespawnRequest>,
    mut death_started: MessageWriter<VaelDeathStarted>,
    mut gear_rules: Local<Option<crate::gear::GearRules>>,
    mut stat_engine: Local<Option<crate::stats::StatEngine>>,
) {
    if !config.rust {
        return;
    }

    // Dano/cura/`respawn`/passivas do Lua no frame que acabou de correr —
    // do STORE do processo (o `vael.game()` dos contextos BMS é este
    // store; sem handle mlua, sem VM partilhada).
    let kv = game_to_json_process();
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
        // O Lua lia `venom_resist`/`burn_resist` de `M.stat(...)` no próprio
        // tick; o Rust tem de as reconstruir do equipamento/talentos do KV.
        // Só quando há DoT activo (é o único consumidor).
        let resists = match content.as_ref() {
            Some(content) if authority.has_status_dot() => {
                if gear_rules.is_none() || content.is_changed() {
                    *gear_rules = Some(crate::gear::GearRules::from_content(content));
                }
                let rules = gear_rules.as_ref().expect("acabou de ser preenchido");
                status_resists(rules, &kv, &mut stat_engine)
            }
            _ => StatusResists::default(),
        };
        authority.tick(time.delta_secs(), rain, frozen, death_open, resists);
        if authority.death_started() {
            // Transição (não por frame): o consumidor larga a bolsa/toca o
            // `game_over` uma única vez, como o `start_death` do Lua.
            death_started.write(VaelDeathStarted {
                from_dot: authority.death_from_dot(),
            });
        }
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
        apply_patch_json(&patch);
    }

    // HUD/save: o `state_sync` preserva estes campos no merge. Só se toca nos
    // recursos quando há mudança — marcá-los a 60 Hz desligava o gate
    // `save.is_changed()` do HUD (e re-publicava 37 quests por frame).
    if let (Some(mut vitals), Some(mut save)) = (vitals, save) {
        let owned = authority.owned();
        if !owned.matches(&vitals, &save) {
            owned.write_state(&mut vitals, &mut save);
        }
    }
}

/// Resistências de veneno/queimadura que o `rules.lua` lia de
/// `M.stat("<estado>_resist")`: a soma dos `effects` das três peças equipadas
/// com os talentos aprendidos, pelo mesmo grafo de atributos do diário.
///
/// O `gear.<slot>` só conta com o item na mochila (`item.<id>` > 0), como no
/// `src/gear.rs`; a malha limpa marcas órfãs no estado local (não escreve no
/// KV — isso é do módulo do equipamento). A conta é a MESMA do shadow
/// (`crate::rules_shadow::status_resists`), para a paridade ser real.
fn status_resists(
    rules: &crate::gear::GearRules,
    kv: &BTreeMap<String, Value>,
    stat_engine: &mut Option<crate::stats::StatEngine>,
) -> StatusResists {
    let mut state = crate::gear::GearState::default();
    for slot in crate::gear::SLOTS {
        if let Some(Value::String(id)) = kv.get(&crate::gear::gear_key(slot)) {
            state.equipped.insert(slot.to_string(), id.clone());
        }
    }
    let equipped: Vec<String> = state.equipped.values().cloned().collect();
    for id in equipped {
        if let Some(count) = kv.get(&crate::gear::item_key(&id)).and_then(Value::as_f64) {
            state.held.insert(id, count);
        }
    }
    let mut learned = std::collections::BTreeSet::new();
    for id in rules.skills.keys() {
        let key = format!("{}{id}", crate::gear::SKILL_KEY_PREFIX);
        if matches!(kv.get(&key), Some(Value::Bool(true))) {
            learned.insert(id.clone());
        }
    }
    crate::rules_shadow::status_resists(rules, &mut state, &learned, stat_engine)
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

/// O `legacy` já tem `value` em `key`? (leitura do [`OwnedVitals::matches`]).
fn legacy_num_is(legacy: &BTreeMap<String, Value>, key: &str, value: f32) -> bool {
    let expected = json_num(value);
    match (legacy.get(key), &expected) {
        (Some(Value::Number(current)), Value::Number(expected)) => {
            current.as_f64() == expected.as_f64()
        }
        _ => false,
    }
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
        auth.tick(1.0, false, false, false, StatusResists::default());
        assert!((auth.owned().hp - 96.0).abs() < 1e-4);

        auth.tick(1.0, false, false, false, StatusResists::default());
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
        auth.tick(0.1, false, false, false, StatusResists::default());
        assert!((auth.owned().dying - 2.1).abs() < 1e-4);

        auth.tick(2.2, false, false, false, StatusResists::default());
        let patch = auth.patch();
        assert_eq!(patch.get("death_pending"), Some(&json!(true)));
        assert_eq!(patch.get("hp"), Some(&json!(0.0)));
        assert_eq!(patch.get("death_wait"), Some(&json!(5.0)));

        // Sem o modal aberto o fallback conta até ao respawn.
        auth.tick(4.0, false, false, false, StatusResists::default());
        assert_eq!(auth.patch().get("death_wait"), Some(&json!(1.0)));
        assert_eq!(auth.patch().get("death_pending"), Some(&json!(true)));

        auth.tick(1.0, false, false, false, StatusResists::default());
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
        auth.tick(0.1, false, false, false, StatusResists::default());
        assert!((auth.owned().iframes - 0.0).abs() < 1e-4);

        // `damage_player` do Lua: hp 60, i-frames 0.35, veneno 3.
        auth.seed(&kv(&[
            ("hp", json!(60)),
            ("max_hp", json!(100)),
            ("iframes", json!(0.35)),
            ("venom", json!(3)),
        ]));
        assert!((auth.owned().hp - 60.0).abs() < 1e-4);
        auth.tick(0.1, false, false, false, StatusResists::default());
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
        auth.tick(1.0, false, true, false, StatusResists::default());
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
    fn matches_nao_marca_os_recursos_sem_mudanca() {
        let owned = OwnedVitals {
            hp: 40.0,
            max_hp: 100.0,
            iframes: 0.35,
            venom: 3.0,
            burn: 0.0,
            frost: 1.5,
            venom_tick: 0.4,
            burn_tick: 0.0,
            dying: 0.0,
            death_pending: false,
            death_wait: 0.0,
        };
        let mut vitals = PlayerVitals::default();
        let mut save = VaelSave::default();
        assert!(!owned.matches(&vitals, &save), "estado ainda não escrito");
        owned.write_state(&mut vitals, &mut save);
        assert!(owned.matches(&vitals, &save), "depois de escrever, é igual");

        // Qualquer campo diferente volta a pedir escrita.
        let mut mais_lento = owned;
        mais_lento.iframes = 0.34;
        assert!(!mais_lento.matches(&vitals, &save));
        let mut morto = owned;
        morto.death_pending = true;
        assert!(!morto.matches(&vitals, &save));
        let mut outro_hp = owned;
        outro_hp.hp = 39.0;
        assert!(!outro_hp.matches(&vitals, &save));

        // Um `legacy` com a chave em texto (save editado à mão) não conta.
        save.legacy
            .insert("venom".into(), Value::String("3".into()));
        assert!(!owned.matches(&vitals, &save));
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
        auth.tick(1.0, false, false, false, StatusResists::default());
        assert!(auth.pending_patch().is_some(), "iframes desceram no tick");
    }

    #[test]
    fn resistencias_do_equipamento_e_dos_talentos_chegam_ao_tick() {
        let mut content = VaelContent::default();
        content.items.items.push(crate::content::ItemDef {
            id: "nature_amulet".into(),
            slot: "charm".into(),
            effects: BTreeMap::from([("venom_resist".to_string(), 0.5)]),
            ..Default::default()
        });
        content.progression.skills.push(crate::content::SkillDef {
            id: "burnward".into(),
            effects: BTreeMap::from([("burn_resist".to_string(), 0.4)]),
            ..Default::default()
        });

        let equipado = kv(&[
            ("gear.charm", json!("nature_amulet")),
            ("item.nature_amulet", json!(1)),
            ("skill.burnward", json!(true)),
        ]);
        let rules = crate::gear::GearRules::from_content(&content);
        let mut engine = None;
        let resists = status_resists(&rules, &equipado, &mut engine);
        assert!(
            (resists.venom - 0.5).abs() < 1e-4,
            "venom={}",
            resists.venom
        );
        assert!((resists.burn - 0.4).abs() < 1e-4, "burn={}", resists.burn);

        // A peça saiu da mochila: o efeito dela não conta (marca órfã).
        let sem_peca = kv(&[
            ("gear.charm", json!("nature_amulet")),
            ("skill.burnward", json!(true)),
        ]);
        let resists = status_resists(&rules, &sem_peca, &mut engine);
        assert_eq!(resists.venom, 0.0);
        assert!(
            (resists.burn - 0.4).abs() < 1e-4,
            "o talento continua aprendido"
        );
    }

    #[test]
    fn o_dot_da_autoridade_usa_a_resistencia_do_equipamento() {
        let mut auth = VaelRulesAuthority::default();
        auth.seed(&kv(&[
            ("hp", json!(100)),
            ("max_hp", json!(100)),
            ("venom", json!(10)),
        ]));
        assert!(auth.has_status_dot());
        auth.tick(
            1.0,
            false,
            false,
            false,
            StatusResists {
                venom: 0.5,
                burn: 0.0,
            },
        );
        assert!((auth.owned().hp - 98.0).abs() < 1e-4);

        // Sem estados activos as resistências não são consultadas.
        let mut limpo = VaelRulesAuthority::default();
        limpo.seed(&kv(&[("hp", json!(100)), ("max_hp", json!(100))]));
        assert!(!limpo.has_status_dot());
    }

    #[test]
    fn morte_armada_pelo_tick_emite_o_sinal_para_o_caminho_rust() {
        let _store_lock = vael_engine::luau::game::test_support::PROCESS_STORE_TEST_LOCK
            .lock()
            .expect("lock de teste do store");
        vael_engine::luau::game::restore_game_json(&std::collections::BTreeMap::new());

        #[derive(Resource, Default)]
        struct DeathSignals(Vec<VaelDeathStarted>);
        fn collect(mut reader: MessageReader<VaelDeathStarted>, mut out: ResMut<DeathSignals>) {
            for signal in reader.read() {
                out.0.push(*signal);
            }
        }

        let mut app = App::new();
        app.add_plugins(bevy::MinimalPlugins)
            .insert_resource(VaelRulesConfig { rust: true })
            .init_resource::<PlayerVitals>()
            .init_resource::<VaelSave>()
            .init_resource::<DeathSignals>()
            .add_plugins(VaelRulesAuthorityPlugin)
            .add_systems(Update, collect.after(VaelRulesSet));

        // Um save com 0 HP sem `dying` arma a morte no primeiro tick em que
        // haja `dt` — o `start_death` do Lua não corre neste caminho.
        let mut patch = BTreeMap::new();
        patch.insert("hp".to_string(), json!(0));
        patch.insert("max_hp".to_string(), json!(100));
        vael_engine::luau::game::apply_patch_json(&patch);
        app.update();
        app.update();

        let signals = &app.world().resource::<DeathSignals>().0;
        assert_eq!(signals.len(), 1, "um sinal por morte, nunca por frame");
        assert!(!signals[0].from_dot, "0 HP de save não é DoT");
        assert!(
            app.world().resource::<VaelRulesAuthority>().owned().dying > 0.0,
            "o tick arma o `dying` (o ecrã de morte lê o estado)"
        );
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

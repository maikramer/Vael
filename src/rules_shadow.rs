//! Shadow das regras vitais do Lua (`scripts/lib/rules.lua`).
//!
//! O Lua continua a ser a fonte de verdade. Este plugin simula em Rust os
//! mesmos temporizadores — i-frames, estados (veneno/queimadura/gelo), queda e
//! respawn — a 60 Hz e compara o resultado com o snapshot de `viber.game()`
//! que o [`VaelStateSyncPlugin`](crate::state_sync::VaelStateSyncPlugin) lê a
//! 10 Hz. Não escreve no Lua, no [`VaelSave`], no ECS nem no HUD.
//!
//! Limites conhecidos:
//! - `rules.god` (modo DEV) é um local do módulo Lua e não viaja em
//!   `viber.game()`; com o god ligado o Lua congela os estados e o shadow
//!   continua a fazer DoT — divergência esperada.
//! - Dano/cura de criaturas, poções e fogueiras entram no Lua entre dois
//!   snapshots; o shadow só os vê no snapshot seguinte (daí a tolerância de
//!   `hp`).
//! - O shadow não simula `sync_passives` (mudanças de `max_hp`), a bolsa
//!   perdida, FX, som, UI, teleporte do respawn nem o refill dos frascos.
//!
//! `VAEL_RULES_SHADOW=1` liga o sistema; por omissão não corre (é QA).

use std::collections::BTreeMap;

use bevy::prelude::*;
use serde_json::Value;
use bevy_ext_core::MenusOpen;
use viber::ui::modal::UiModalsOpen;
use viber::worldsys::WeatherState;

use crate::components::PlayerVitals;
use crate::save::VaelSave;

/// A partir daqui `clock.raining()` diz que chove. Fonte única: o contrato
/// 0,3 é o [`crate::clock::RAIN_HEAVY`] (f64); as comparações com o
/// `WeatherState` da engine (f32) fazem o cast.
pub use crate::clock::RAIN_HEAVY;

/// DPS dos estados (tabela `STATUS` do Lua).
const VENOM_DPS: f32 = 4.0;
const BURN_DPS: f32 = 6.0;
/// A queimadura apaga-se ao dobro da velocidade à chuva.
const BURN_RAIN_RATE: f32 = 2.0;

/// Tempo caído antes do ecrã de morte (`start_death`).
const DEATH_FALL_SECS: f32 = 2.2;
/// Fallback de respawn sem modal de morte (`rules.tick`).
const DEATH_WAIT_SECS: f32 = 5.0;
/// I-frames do `M.respawn`.
const RESPAWN_IFRAMES: f32 = 2.0;

/// Tolerância da comparação de `hp` (Lua vs shadow).
pub const HP_TOLERANCE: f32 = 1.5;
/// Tolerância dos temporizadores (estados, `dying`, `death_wait`), em segundos.
pub const TIMER_TOLERANCE: f32 = 0.2;
/// `max_hp` só muda por passivas (fora do shadow); qualquer diferença é sinal.
const MAX_HP_TOLERANCE: f32 = 0.01;
/// Epsilon para detetar que o snapshot Lua mudou.
const REF_EPSILON: f32 = 1e-4;

/// Plugin do shadow das regras vitais.
pub struct VaelRulesShadowPlugin;

impl Plugin for VaelRulesShadowPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VaelRulesShadow>()
            .add_systems(Update, rules_shadow_system);
    }
}

/// Estado simulado do shadow mais os contadores de comparação.
#[derive(Debug, Clone, Resource)]
pub struct VaelRulesShadow {
    // ── estado simulado ────────────────────────────────────────────────
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
    /// O `shadow_tick` chamou `respawn()` neste frame. Não faz parte da
    /// comparação shadow↔Lua; é o sinal para a UI teleportar/refrescar.
    pub respawned: bool,

    // ── referência Lua e contadores ─────────────────────────────────────
    /// Último snapshot Lua usado para semear. `iframes` entra na cópia mas
    /// NÃO na deteção de mudança (o Lua decrementa-o todos os frames).
    last_lua: Option<LuaSnapshot>,
    /// Comparações shadow↔Lua feitas antes de uma semente.
    pub comparisons: u64,
    /// Comparações que passaram alguma tolerância.
    pub divergences: u64,
    /// Vezes que o shadow foi (re)semeado.
    pub seeds: u64,
    /// Ticks de simulação aplicados.
    pub ticks: u64,
    /// A última comparação divergiu.
    pub divergent: bool,
    /// Houve divergência desde o último resumo de 1 Hz.
    divergent_since_log: bool,
    /// Última divergência registada.
    pub last_divergence: Option<ShadowDivergence>,
    /// Acumulador do resumo de 1 Hz.
    log_acc: f32,
}

impl Default for VaelRulesShadow {
    /// Os defaults de `rules.lua`: 100/100 e nenhum estado ativo.
    fn default() -> Self {
        Self {
            hp: 100.0,
            max_hp: 100.0,
            iframes: 0.0,
            venom: 0.0,
            burn: 0.0,
            frost: 0.0,
            venom_tick: 0.0,
            burn_tick: 0.0,
            dying: 0.0,
            death_pending: false,
            death_wait: 0.0,
            respawned: false,
            last_lua: None,
            comparisons: 0,
            divergences: 0,
            seeds: 0,
            ticks: 0,
            divergent: false,
            divergent_since_log: false,
            last_divergence: None,
            log_acc: 0.0,
        }
    }
}

/// Diferença entre o estado previsto pelo shadow e o snapshot Lua.
#[derive(Debug, Clone)]
pub struct ShadowDivergence {
    /// Campos que passaram a tolerância.
    pub fields: Vec<&'static str>,
    pub hp_lua: f32,
    pub hp_shadow: f32,
    pub max_hp_lua: f32,
    pub max_hp_shadow: f32,
    pub venom_lua: f32,
    pub venom_shadow: f32,
    pub burn_lua: f32,
    pub burn_shadow: f32,
    pub frost_lua: f32,
    pub frost_shadow: f32,
    pub dying_lua: f32,
    pub dying_shadow: f32,
    pub death_pending_lua: bool,
    pub death_pending_shadow: bool,
    pub death_wait_lua: f32,
    pub death_wait_shadow: f32,
}

/// Cópia tipada dos campos do Lua que o shadow lê.
#[derive(Debug, Clone, Copy, PartialEq)]
struct LuaSnapshot {
    hp: f32,
    max_hp: f32,
    iframes: f32,
    venom: f32,
    burn: f32,
    frost: f32,
    venom_tick: f32,
    burn_tick: f32,
    dying: f32,
    death_pending: bool,
    death_wait: f32,
}

impl LuaSnapshot {
    /// Lê o snapshot do [`PlayerVitals`] e do `legacy` do [`VaelSave`]. Os
    /// estados, i-frames, `dying` e `death_wait` viajam em `viber.game()` como
    /// chaves que o schema tipado não conhece, por isso vivem em `legacy`.
    /// Sanea como o `ensure_hp` do Lua.
    fn from_state(vitals: &PlayerVitals, save: &VaelSave) -> Self {
        let max_hp = if vitals.max_hp.is_finite() && vitals.max_hp >= 1.0 {
            vitals.max_hp
        } else {
            100.0
        };
        let hp = if vitals.hp.is_finite() {
            vitals.hp.clamp(0.0, max_hp)
        } else {
            max_hp
        };
        let legacy = &save.legacy;
        Self {
            hp,
            max_hp,
            iframes: legacy_num(legacy, "iframes"),
            venom: legacy_num(legacy, "venom"),
            burn: legacy_num(legacy, "burn"),
            frost: legacy_num(legacy, "frost"),
            venom_tick: legacy_num(legacy, "venom_tick"),
            burn_tick: legacy_num(legacy, "burn_tick"),
            dying: legacy_num(legacy, "dying"),
            death_pending: legacy_flag(legacy, "death_pending"),
            death_wait: legacy_num(legacy, "death_wait"),
        }
    }

    /// Mudou algum campo que justifica uma nova semente? `iframes` e os
    /// acumuladores `*_tick` não são referência de jogo (mudam a cada frame);
    /// os timers de estado entram, com a tolerância da comparação.
    fn changed(&self, other: &Self) -> bool {
        !approx(self.hp, other.hp, REF_EPSILON)
            || !approx(self.max_hp, other.max_hp, REF_EPSILON)
            || !approx(self.venom, other.venom, REF_EPSILON)
            || !approx(self.burn, other.burn, REF_EPSILON)
            || !approx(self.frost, other.frost, REF_EPSILON)
            || !approx(self.dying, other.dying, REF_EPSILON)
            || self.death_pending != other.death_pending
            || !approx(self.death_wait, other.death_wait, REF_EPSILON)
    }
}

impl VaelRulesShadow {
    /// Copia o snapshot Lua para o estado simulado e regista a semente.
    fn seed_from(&mut self, lua: LuaSnapshot) {
        self.hp = lua.hp;
        self.max_hp = lua.max_hp;
        self.iframes = lua.iframes;
        self.venom = lua.venom;
        self.burn = lua.burn;
        self.frost = lua.frost;
        self.venom_tick = lua.venom_tick;
        self.burn_tick = lua.burn_tick;
        self.dying = lua.dying;
        self.death_pending = lua.death_pending;
        self.death_wait = lua.death_wait;
        self.last_lua = Some(lua);
        self.seeds = self.seeds.saturating_add(1);
    }

    /// Compara o estado previsto com o snapshot Lua novo. Devolve `None`
    /// quando tudo está dentro das tolerâncias.
    fn compare(&self, lua: &LuaSnapshot) -> Option<ShadowDivergence> {
        let mut divergence = ShadowDivergence {
            fields: Vec::new(),
            hp_lua: lua.hp,
            hp_shadow: self.hp,
            max_hp_lua: lua.max_hp,
            max_hp_shadow: self.max_hp,
            venom_lua: lua.venom,
            venom_shadow: self.venom,
            burn_lua: lua.burn,
            burn_shadow: self.burn,
            frost_lua: lua.frost,
            frost_shadow: self.frost,
            dying_lua: lua.dying,
            dying_shadow: self.dying,
            death_pending_lua: lua.death_pending,
            death_pending_shadow: self.death_pending,
            death_wait_lua: lua.death_wait,
            death_wait_shadow: self.death_wait,
        };
        if !approx(self.hp, lua.hp, HP_TOLERANCE) {
            divergence.fields.push("hp");
        }
        if !approx(self.max_hp, lua.max_hp, MAX_HP_TOLERANCE) {
            divergence.fields.push("max_hp");
        }
        if !approx(self.venom, lua.venom, TIMER_TOLERANCE) {
            divergence.fields.push("venom");
        }
        if !approx(self.burn, lua.burn, TIMER_TOLERANCE) {
            divergence.fields.push("burn");
        }
        if !approx(self.frost, lua.frost, TIMER_TOLERANCE) {
            divergence.fields.push("frost");
        }
        if !approx(self.dying, lua.dying, TIMER_TOLERANCE) {
            divergence.fields.push("dying");
        }
        if self.death_pending != lua.death_pending {
            divergence.fields.push("death_pending");
        }
        if !approx(self.death_wait, lua.death_wait, TIMER_TOLERANCE) {
            divergence.fields.push("death_wait");
        }
        (!divergence.fields.is_empty()).then_some(divergence)
    }

    /// `M.status_clear()` do Lua.
    fn status_clear(&mut self) {
        self.venom = 0.0;
        self.burn = 0.0;
        self.frost = 0.0;
        self.venom_tick = 0.0;
        self.burn_tick = 0.0;
    }
}

/// Simula um tick de `rules.tick` para as partes portadas.
///
/// `frozen` é o portão dos estados (`viber.menu_open()`, mais o título);
/// `menu_open` é o ecrã de morte aberto (`ui.is_open("death")`), que repõe os
/// 5 s do fallback. Não toca em ouro, FX, som, UI, save nem ECS.
pub fn shadow_tick(
    state: &mut VaelRulesShadow,
    dt: f32,
    rain: bool,
    frozen: bool,
    menu_open: bool,
) {
    state.respawned = false;
    sanitize(state);
    if !dt.is_finite() || dt <= 0.0 {
        return;
    }
    state.ticks = state.ticks.saturating_add(1);

    // I-frames: decrescem sempre, mesmo durante a morte e com menus abertos.
    state.iframes = (state.iframes - dt).max(0.0);

    // ── estados com DPS (veneno, queimadura) ────────────────────────────
    // O Lua só acumula ticks de DPS enquanto `dying <= 0`; o gelo não tem DPS.
    // Com o timer já a zero o acumulador do tick é limpo (o `elseif` do Lua).
    if state.venom > 0.0 && !frozen {
        state.venom = (state.venom - dt).max(0.0);
        if state.dying <= 0.0 {
            state.venom_tick += dt;
            let ticks = state.venom_tick.floor();
            if ticks > 0.0 {
                state.venom_tick -= ticks;
                apply_damage(state, ticks * VENOM_DPS);
            }
        }
    } else if state.venom <= 0.0 {
        state.venom = 0.0;
        state.venom_tick = 0.0;
    }

    if state.burn > 0.0 && !frozen {
        let rate = if rain { BURN_RAIN_RATE } else { 1.0 };
        state.burn = (state.burn - dt * rate).max(0.0);
        if state.dying <= 0.0 {
            state.burn_tick += dt;
            let ticks = state.burn_tick.floor();
            if ticks > 0.0 {
                state.burn_tick -= ticks;
                apply_damage(state, ticks * BURN_DPS);
            }
        }
    } else if state.burn <= 0.0 {
        state.burn = 0.0;
        state.burn_tick = 0.0;
    }

    if state.frost > 0.0 && !frozen {
        state.frost = (state.frost - dt).max(0.0);
    } else if state.frost <= 0.0 {
        state.frost = 0.0;
    }

    // ── morte e respawn ─────────────────────────────────────────────────
    // Um save carregado com o herói vivo não pode manter o ecrã de morte.
    if state.death_pending && state.hp > 0.0 {
        state.death_pending = false;
        state.dying = 0.0;
    }

    // 0 HP sem `dying`/`death_pending` (save corrompido): começa a morte.
    if state.hp <= 0.0 && state.dying <= 0.0 && !state.death_pending {
        start_death(state);
    }

    if state.dying > 0.0 {
        state.dying = (state.dying - dt).max(0.0);
        if state.dying <= 0.0 && !state.death_pending {
            state.death_pending = true;
            state.death_wait = DEATH_WAIT_SECS;
        }
    } else if state.death_pending {
        if menu_open {
            // Ecrã de morte aberto: o Lua repõe os 5 s a cada tick.
            state.death_wait = DEATH_WAIT_SECS;
        } else {
            state.death_wait = (state.death_wait - dt).max(0.0);
            if state.death_wait <= 0.0 {
                respawn(state);
            }
        }
    }

    state.hp = state.hp.clamp(0.0, state.max_hp);
}

/// `start_death`: só arma se ainda não está a morrer nem pendente.
fn start_death(state: &mut VaelRulesShadow) {
    if state.dying > 0.0 || state.death_pending {
        return;
    }
    state.dying = DEATH_FALL_SECS;
}

/// Aplica dano clampado, como o `max(0, hp - dmg)` do Lua.
fn apply_damage(state: &mut VaelRulesShadow, amount: f32) {
    if !amount.is_finite() || amount <= 0.0 {
        return;
    }
    state.hp = (state.hp - amount).max(0.0);
    if state.hp <= 0.0 {
        start_death(state);
    }
}

/// `M.respawn`: vida cheia, estados limpos, i-frames e flags de morte a zero.
/// A bolsa, os frascos, o teleporte, a UI e o som ficam para o Lua.
fn respawn(state: &mut VaelRulesShadow) {
    state.hp = state.max_hp;
    state.dying = 0.0;
    state.death_pending = false;
    state.death_wait = 0.0;
    state.status_clear();
    state.iframes = RESPAWN_IFRAMES;
    state.respawned = true;
}

/// Semeia o estado a partir do KV plano do `viber.game()`.
///
/// É a leitura da autoridade Rust: corre DEPOIS do Lua no frame, por isso vê
/// o dano/cura (`M.damage_player`/`M.heal_player`), os itens, o `respawn`
/// manual e as passivas (`max_hp`) do mesmo frame. Sanea como o `ensure_hp`.
/// Ao contrário de [`LuaSnapshot::from_state`], lê `hp`/`max_hp` do KV.
pub fn seed_from_kv(state: &mut VaelRulesShadow, kv: &BTreeMap<String, Value>) {
    let max_hp = match kv.get("max_hp").and_then(Value::as_f64) {
        Some(value) if value.is_finite() && value >= 1.0 => value as f32,
        _ => 100.0,
    };
    let hp = match kv.get("hp").and_then(Value::as_f64) {
        Some(value) if value.is_finite() => (value as f32).clamp(0.0, max_hp),
        _ => max_hp,
    };
    state.max_hp = max_hp;
    state.hp = hp;
    state.iframes = legacy_num(kv, "iframes");
    state.venom = legacy_num(kv, "venom");
    state.burn = legacy_num(kv, "burn");
    state.frost = legacy_num(kv, "frost");
    state.venom_tick = legacy_num(kv, "venom_tick");
    state.burn_tick = legacy_num(kv, "burn_tick");
    state.dying = legacy_num(kv, "dying");
    state.death_pending = legacy_flag(kv, "death_pending");
    state.death_wait = legacy_num(kv, "death_wait");
}

/// Guardas do `ensure_hp`: `max_hp` inválido vale 100, `hp` não finito vale
/// `max_hp`, e os timers nunca ficam negativos ou NaN.
fn sanitize(state: &mut VaelRulesShadow) {
    if !state.max_hp.is_finite() || state.max_hp < 1.0 {
        state.max_hp = 100.0;
    }
    if !state.hp.is_finite() {
        state.hp = state.max_hp;
    }
    state.hp = state.hp.clamp(0.0, state.max_hp);
    state.iframes = sanitize_timer(state.iframes);
    state.venom = sanitize_timer(state.venom);
    state.burn = sanitize_timer(state.burn);
    state.frost = sanitize_timer(state.frost);
    state.venom_tick = sanitize_timer(state.venom_tick);
    state.burn_tick = sanitize_timer(state.burn_tick);
    state.dying = sanitize_timer(state.dying);
    state.death_wait = sanitize_timer(state.death_wait);
}

fn sanitize_timer(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

/// Simula e compara a cada frame. Semeia no primeiro frame e sempre que o
/// snapshot Lua muda (a comparação é feita ANTES de semear).
fn rules_shadow_system(
    time: Res<Time>,
    vitals: Option<Res<PlayerVitals>>,
    save: Option<Res<VaelSave>>,
    weather: Option<Res<WeatherState>>,
    menus: Option<Res<MenusOpen>>,
    modals: Option<Res<UiModalsOpen>>,
    mut shadow: ResMut<VaelRulesShadow>,
    mut enabled: Local<Option<bool>>,
) {
    let enabled = *enabled
        .get_or_insert_with(|| shadow_enabled(std::env::var("VAEL_RULES_SHADOW").ok().as_deref()));
    if !enabled {
        return;
    }
    let (Some(vitals), Some(save)) = (vitals, save) else {
        return;
    };

    let lua = LuaSnapshot::from_state(&vitals, &save);
    if shadow
        .last_lua
        .is_none_or(|previous| previous.changed(&lua))
    {
        if shadow.last_lua.is_some() {
            shadow.comparisons = shadow.comparisons.saturating_add(1);
            match shadow.compare(&lua) {
                Some(divergence) => {
                    shadow.divergences = shadow.divergences.saturating_add(1);
                    shadow.divergent = true;
                    shadow.divergent_since_log = true;
                    shadow.last_divergence = Some(divergence);
                }
                None => shadow.divergent = false,
            }
        }
        shadow.seed_from(lua);
    }

    let rain = weather
        .as_deref()
        .is_some_and(|w| w.rain >= RAIN_HEAVY as f32);
    // `viber.menu_open()` é o `MenusOpen::any()`; o título também congela.
    let frozen = menus.as_deref().is_some_and(MenusOpen::any)
        || modals.as_deref().is_some_and(|m| m.is_open("title"));
    let death_open = modals.as_deref().is_some_and(|m| m.is_open("death"));
    let dt = time.delta_secs();
    shadow_tick(&mut shadow, dt, rain, frozen, death_open);

    // Resumo de 1 Hz (sem spam). O flag cobre o último segundo, não só a
    // última comparação.
    if dt.is_finite() && dt > 0.0 {
        shadow.log_acc += dt;
    }
    if shadow.log_acc >= 1.0 {
        shadow.log_acc = 0.0;
        log_summary(&shadow);
        shadow.divergent_since_log = false;
    }
}

fn log_summary(shadow: &VaelRulesShadow) {
    let Some(lua) = shadow.last_lua else {
        return;
    };
    info!(
        hp_lua = lua.hp,
        hp_shadow = shadow.hp,
        max_hp_lua = lua.max_hp,
        max_hp_shadow = shadow.max_hp,
        venom_lua = lua.venom,
        venom_shadow = shadow.venom,
        burn_lua = lua.burn,
        burn_shadow = shadow.burn,
        frost_lua = lua.frost,
        frost_shadow = shadow.frost,
        dying_lua = lua.dying,
        dying_shadow = shadow.dying,
        death_pending_lua = lua.death_pending,
        death_pending_shadow = shadow.death_pending,
        death_wait_lua = lua.death_wait,
        death_wait_shadow = shadow.death_wait,
        divergent = shadow.divergent_since_log || shadow.divergent,
        comparisons = shadow.comparisons,
        divergences = shadow.divergences,
        seeds = shadow.seeds,
        ticks = shadow.ticks,
        "vael: shadow das regras vitais"
    );
    if shadow.divergent_since_log
        && let Some(divergence) = &shadow.last_divergence
    {
        info!(
            fields = ?divergence.fields,
            hp_lua = divergence.hp_lua,
            hp_shadow = divergence.hp_shadow,
            max_hp_lua = divergence.max_hp_lua,
            max_hp_shadow = divergence.max_hp_shadow,
            venom_lua = divergence.venom_lua,
            venom_shadow = divergence.venom_shadow,
            burn_lua = divergence.burn_lua,
            burn_shadow = divergence.burn_shadow,
            frost_lua = divergence.frost_lua,
            frost_shadow = divergence.frost_shadow,
            dying_lua = divergence.dying_lua,
            dying_shadow = divergence.dying_shadow,
            death_pending_lua = divergence.death_pending_lua,
            death_pending_shadow = divergence.death_pending_shadow,
            death_wait_lua = divergence.death_wait_lua,
            death_wait_shadow = divergence.death_wait_shadow,
            "vael: shadow divergiu do Lua"
        );
    }
}

/// Gate `VAEL_RULES_SHADOW` (opt-in: só corre com `1`/`true`/`on`/`yes`).
fn shadow_enabled(raw: Option<&str>) -> bool {
    matches!(
        raw.map(|value| value.trim().to_ascii_lowercase())
            .as_deref(),
        Some("1" | "true" | "on" | "yes")
    )
}

fn approx(a: f32, b: f32, tolerance: f32) -> bool {
    (a - b).abs() <= tolerance
}

fn legacy_num(legacy: &std::collections::BTreeMap<String, Value>, key: &str) -> f32 {
    legacy
        .get(key)
        .and_then(Value::as_f64)
        .and_then(|value| {
            let value = value as f32;
            value.is_finite().then_some(value.max(0.0))
        })
        .unwrap_or(0.0)
}

fn legacy_flag(legacy: &std::collections::BTreeMap<String, Value>, key: &str) -> bool {
    match legacy.get(key) {
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(value)) => value.as_f64().is_some_and(|value| value != 0.0),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tick(state: &mut VaelRulesShadow, dt: f32) {
        shadow_tick(state, dt, false, false, false);
    }

    #[test]
    fn veneno_aplica_4_dps_por_segundo_inteiro() {
        let mut state = VaelRulesShadow::default();
        state.venom = 10.0;

        tick(&mut state, 1.0);
        assert!((state.hp - 96.0).abs() < 1e-4, "hp={}", state.hp);
        assert_eq!(state.venom_tick, 0.0);

        tick(&mut state, 1.0);
        assert!((state.hp - 92.0).abs() < 1e-4, "hp={}", state.hp);

        // Um frame longo aplica TODOS os ticks devidos, como o Lua.
        let mut longo = VaelRulesShadow::default();
        longo.venom = 10.0;
        tick(&mut longo, 2.0);
        assert!((longo.hp - 92.0).abs() < 1e-4, "hp={}", longo.hp);
        assert_eq!(longo.venom, 8.0);
    }

    #[test]
    fn queimadura_na_chuva_decai_a_dobro_e_faz_6_dps() {
        let mut state = VaelRulesShadow::default();
        state.burn = 10.0;

        shadow_tick(&mut state, 1.0, true, false, false);
        assert!((state.burn - 8.0).abs() < 1e-4, "burn={}", state.burn);
        assert!((state.hp - 94.0).abs() < 1e-4, "hp={}", state.hp);

        shadow_tick(&mut state, 1.0, true, false, false);
        assert!((state.burn - 6.0).abs() < 1e-4, "burn={}", state.burn);
        assert!((state.hp - 88.0).abs() < 1e-4, "hp={}", state.hp);

        // Sem chuva o timer cai 1 s por segundo.
        let mut seco = VaelRulesShadow::default();
        seco.burn = 10.0;
        tick(&mut seco, 1.0);
        assert!((seco.burn - 9.0).abs() < 1e-4, "burn={}", seco.burn);
    }

    #[test]
    fn iframes_decrementam_e_ficam_em_zero() {
        let mut state = VaelRulesShadow::default();
        state.iframes = 0.35;

        tick(&mut state, 0.1);
        assert!(
            (state.iframes - 0.25).abs() < 1e-4,
            "iframes={}",
            state.iframes
        );

        tick(&mut state, 0.5);
        assert_eq!(state.iframes, 0.0);

        tick(&mut state, 1.0);
        assert_eq!(state.iframes, 0.0, "nunca fica negativo");
    }

    #[test]
    fn hp_a_zero_entra_em_dying_e_respawn_aos_5s() {
        let mut state = VaelRulesShadow::default();
        state.hp = 0.0;
        state.venom = 30.0;
        state.burn = 30.0;
        state.frost = 30.0;
        state.venom_tick = 0.5;
        state.burn_tick = 0.5;

        // O tick que inicia a morte também desconta o seu `dt` do `dying`,
        // como o `rules.tick`.
        tick(&mut state, 0.1);
        assert!((state.dying - 2.1).abs() < 1e-4, "dying={}", state.dying);
        assert!(!state.death_pending);

        // 2,2 s caído (com o resto do primeiro tick) chega ao ecrã de morte.
        tick(&mut state, 2.2);
        assert!(state.dying < 1e-4, "dying={}", state.dying);
        assert!(state.death_pending);
        assert!(
            (state.death_wait - 5.0).abs() < 1e-4,
            "wait={}",
            state.death_wait
        );

        // Sem modal de morte, o fallback conta 5 s.
        tick(&mut state, 4.0);
        assert!(state.death_pending);
        assert!(
            (state.death_wait - 1.0).abs() < 1e-4,
            "wait={}",
            state.death_wait
        );

        tick(&mut state, 1.0);
        assert!(!state.death_pending);
        assert_eq!(state.dying, 0.0);
        assert_eq!(state.death_wait, 0.0);
        assert_eq!(state.hp, 100.0);
        assert_eq!(state.venom, 0.0);
        assert_eq!(state.burn, 0.0);
        assert_eq!(state.frost, 0.0);
        assert_eq!(state.venom_tick, 0.0);
        assert_eq!(state.burn_tick, 0.0);
        assert!(
            (state.iframes - 2.0).abs() < 1e-4,
            "iframes={}",
            state.iframes
        );
        assert!(state.respawned, "o fallback tem de sinalizar a UI");
        tick(&mut state, 0.1);
        assert!(!state.respawned, "o sinal dura um tick");
    }

    #[test]
    fn guardas_de_max_hp_e_nao_finitos() {
        // max_hp a zero vale 100, como o `ensure_hp`.
        let mut state = VaelRulesShadow::default();
        state.max_hp = 0.0;
        state.hp = 50.0;
        tick(&mut state, 1.0);
        assert_eq!(state.max_hp, 100.0);
        assert_eq!(state.hp, 50.0);

        // Tudo não finito é saneado; o hp cai para o max_hp.
        let mut state = VaelRulesShadow::default();
        state.max_hp = f32::NAN;
        state.hp = f32::NAN;
        state.iframes = f32::NEG_INFINITY;
        state.venom = f32::NAN;
        state.venom_tick = f32::INFINITY;
        state.dying = f32::NAN;
        state.death_wait = f32::NAN;
        tick(&mut state, 1.0);
        assert!(state.hp.is_finite() && state.max_hp.is_finite());
        assert_eq!(state.hp, state.max_hp);
        assert_eq!(state.iframes, 0.0);
        assert_eq!(state.venom, 0.0);
        assert_eq!(state.venom_tick, 0.0);
        assert_eq!(state.dying, 0.0);
        assert_eq!(state.death_wait, 0.0);
        assert_eq!(state.ticks, 1);

        // Um `dt` não finito não simula nem estraga o estado.
        let mut state = VaelRulesShadow::default();
        state.iframes = 1.0;
        shadow_tick(&mut state, f32::NAN, false, false, false);
        assert_eq!(state.iframes, 1.0);
        assert_eq!(state.ticks, 0);
    }

    #[test]
    fn frozen_congela_estados_mas_nao_os_tempos_de_morte() {
        let mut state = VaelRulesShadow::default();
        state.hp = 0.0;
        state.venom = 10.0;
        state.burn = 10.0;
        state.frost = 10.0;

        // O menu congela os estados, mas o `dying` arranca e conta.
        shadow_tick(&mut state, 0.1, false, true, false);
        assert!((state.dying - 2.1).abs() < 1e-4, "dying={}", state.dying);
        assert_eq!(state.venom, 10.0);
        assert_eq!(state.burn, 10.0);
        assert_eq!(state.frost, 10.0);
        assert_eq!(state.venom_tick, 0.0);
        assert_eq!(state.burn_tick, 0.0);
        assert_eq!(state.hp, 0.0);

        shadow_tick(&mut state, 1.0, false, true, false);
        assert!((state.dying - 1.1).abs() < 1e-4, "dying={}", state.dying);

        shadow_tick(&mut state, 1.2, false, true, false);
        assert!(state.death_pending);
        assert!(
            (state.death_wait - 5.0).abs() < 1e-4,
            "wait={}",
            state.death_wait
        );

        shadow_tick(&mut state, 5.0, false, true, false);
        assert!(!state.death_pending);
        assert_eq!(state.hp, 100.0);
        assert_eq!(state.venom, 0.0);
        assert_eq!(state.burn, 0.0);
        assert_eq!(state.frost, 0.0);
    }

    #[test]
    fn ecra_de_morte_aberto_repoe_o_fallback() {
        let mut state = VaelRulesShadow::default();
        state.death_pending = true;
        state.hp = 0.0;
        state.death_wait = 1.0;

        shadow_tick(&mut state, 0.5, false, false, true);
        assert!(
            (state.death_wait - 5.0).abs() < 1e-4,
            "wait={}",
            state.death_wait
        );
        assert!(state.death_pending);
    }

    #[test]
    fn semeia_do_legacy_e_deteta_divergencia() {
        let mut vitals = PlayerVitals::default();
        vitals.hp = 40.0;
        let mut save = VaelSave::default();
        save.legacy.insert("venom".into(), json!(3.5));
        save.legacy.insert("burn".into(), json!(1e300)); // f64 finito -> f32 inf
        save.legacy.insert("iframes".into(), json!(0.35));
        save.legacy.insert("death_pending".into(), json!(false));

        let lua = LuaSnapshot::from_state(&vitals, &save);
        assert_eq!(lua.hp, 40.0);
        assert_eq!(lua.venom, 3.5);
        assert_eq!(lua.burn, 0.0, "fora do alcance de f32 é descartado");
        assert_eq!(lua.iframes, 0.35);
        assert!(!lua.death_pending);

        let mut shadow = VaelRulesShadow::default();
        shadow.seed_from(lua);
        assert_eq!(shadow.hp, 40.0);
        assert_eq!(shadow.venom, 3.5);
        assert_eq!(shadow.seeds, 1);

        // O previsto ficou 10 abaixo do snapshot: só `hp` divergiu.
        shadow.hp = 30.0;
        let divergence = shadow.compare(&lua).expect("hp fora da tolerância");
        assert_eq!(divergence.fields, vec!["hp"]);
        assert!((divergence.hp_lua - 40.0).abs() < 1e-4);
        assert!((divergence.hp_shadow - 30.0).abs() < 1e-4);

        // `iframes` e os acumuladores mudam todos os frames; não podem
        // disparar uma semente. `hp` dispara.
        let mut outro = lua;
        outro.iframes = 99.0;
        outro.venom_tick = 0.9;
        assert!(!lua.changed(&outro), "iframes/tick não reseedam");
        outro.hp = 41.0;
        assert!(lua.changed(&outro), "hp tem de reseedar");
    }

    #[test]
    fn plugin_corre_sem_recursos_de_engine() {
        let mut app = App::new();
        app.add_plugins(bevy::MinimalPlugins)
            .init_resource::<PlayerVitals>()
            .init_resource::<VaelSave>()
            .add_plugins(VaelRulesShadowPlugin);
        app.update();
        assert!(app.world().get_resource::<VaelRulesShadow>().is_some());
    }

    #[test]
    fn gate_por_variavel_de_ambiente() {
        assert!(!shadow_enabled(None));
        assert!(!shadow_enabled(Some("")));
        assert!(shadow_enabled(Some("1")));
        assert!(shadow_enabled(Some("on")));
        for off in ["0", "false", "off", "no", "OFF", " False "] {
            assert!(!shadow_enabled(Some(off)), "devia desligar: {off}");
        }
    }
}

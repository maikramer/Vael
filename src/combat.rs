//! Guarda/parry, dano, cura e i-frames em Rust: as primitivas de combate do
//! `rules.lua`.
//!
//! As funções puras trabalham sobre [`CombatKv`] — o subconjunto de
//! `viber.game()` que estas regras leem/escrevem — para poderem ser testadas
//! sem a VM. A bridge [`install`] acrescenta `guard`/`damage_player`/
//! `heal_player`/`grant_iframes` a `viber.vael` depois das quests e
//! lê/escreve a tabela `viber_game`, como o cofre/XP/equipamento.
//!
//! O Lua continua dono dos portões de menu/título e do `M.god`, do
//! `ensure_hp`, dos efeitos (toast, número de dano, som, shake) e do
//! `start_death` (`dying`, bolsa, clip, som). A guarda
//! (`guard_until`/`guard_start`/`guard_lock`) é estado de SESSÃO em memória do
//! Rust: acompanha o `session_id` do jogo e nunca entra no save — um
//! timestamp gravado deixaria o herói invencível até o relógio novo o
//! alcançar.
//!
//! Regras portadas de `scripts/lib/rules.lua`:
//! - `guard`: não reabre enquanto a guarda está activa nem durante o lock;
//! - `damage`: i-frames/dying/death_pending cortam o golpe; um parry dentro da
//!   janela não entra e larga a guarda; o bloqueio reduz para 25 %; a defesa
//!   corta até 60 %; `hp` nunca fica negativo; o golpe que entra dá 0,35 s de
//!   i-frames;
//! - `heal`: cadáver/ecrã de morte recusam; `hp` para em `max_hp`;
//! - `grant_iframes`: só sobe (nunca encurta i-frames activos).

use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use mlua::{Function, Lua, Table, Value};
use viber::luau::host::LuaScriptHost;

/// Duração da guarda [L].
pub const GUARD_SECS: f64 = 0.5;
/// Janela de parry depois de abrir a guarda.
pub const PARRY_WINDOW: f64 = 0.2;
/// Fração do dano que passa com a guarda levantada.
pub const GUARD_REDUCTION: f64 = 0.25;
/// Folga entre guardas (soma ao `guard_secs` no `guard_lock`).
pub const GUARD_LOCK_SLACK: f64 = 0.2;
/// Tecto da defesa (armadura + talentos).
pub const DEFENSE_CAP: f64 = 0.6;
/// I-frames de um golpe que entra.
pub const HIT_IFRAMES: f64 = 0.35;
/// I-frames mínimos de um parry.
pub const PARRY_IFRAMES: f64 = 0.25;
/// `max_hp` de um save sem valor válido.
pub const DEFAULT_MAX_HP: f64 = 100.0;

/// Chaves vitais no `viber.game()`.
pub const KEY_HP: &str = "hp";
pub const KEY_MAX_HP: &str = "max_hp";
pub const KEY_IFRAMES: &str = "iframes";
pub const KEY_DYING: &str = "dying";
pub const KEY_DEATH_PENDING: &str = "death_pending";
/// Sessão do jogo (o Lua incrementa-a no "Novo Jogo").
pub const KEY_SESSION_ID: &str = "session_id";

/// Números da guarda/parry (`GUARD_SECS`, `PARRY_WINDOW`, `GUARD_REDUCTION`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CombatRules {
    pub guard_secs: f64,
    pub parry_window: f64,
    pub guard_reduction: f64,
}

impl Default for CombatRules {
    fn default() -> Self {
        Self {
            guard_secs: GUARD_SECS,
            parry_window: PARRY_WINDOW,
            guard_reduction: GUARD_REDUCTION,
        }
    }
}

/// Estado de combate lido do `viber.game()` + os timers de sessão da guarda.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CombatKv {
    pub hp: f64,
    pub max_hp: f64,
    pub iframes: f64,
    pub dying: f64,
    pub death_pending: bool,
    /// Fim da guarda activa (`now < guard_until` = a guardar).
    pub guard_until: f64,
    /// Instante em que a guarda abriu (`None` = guarda largada).
    pub guard_start: Option<f64>,
    /// Não reabre a guarda antes deste instante.
    pub guard_lock: f64,
}

/// O que um [`damage`] fez. `amount` é o dano final (depois de guarda/defesa).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DamageResult {
    pub entered: bool,
    pub parried: bool,
    pub blocked: bool,
    pub died: bool,
    pub amount: f64,
}

/// O mesmo `ensure_hp` do Lua: sanear `max_hp`/`hp` antes de mexer na vida.
fn ensure_hp(kv: &mut CombatKv) {
    if !kv.max_hp.is_finite() || kv.max_hp < 1.0 {
        kv.max_hp = DEFAULT_MAX_HP;
    }
    if !kv.hp.is_finite() {
        kv.hp = kv.max_hp;
    }
    if kv.hp < 0.0 {
        kv.hp = 0.0;
    }
    if kv.hp > kv.max_hp {
        kv.hp = kv.max_hp;
    }
}

/// Abre a guarda [L] em `now`.
///
/// Não reabre enquanto a guarda está activa (`guard_until`) nem durante o
/// lock que impede premir [L] em cadência e manter o parry aberto para sempre.
pub fn guard(kv: &mut CombatKv, rules: &CombatRules, now: f64) {
    if now < kv.guard_lock || now < kv.guard_until {
        return;
    }
    kv.guard_until = now + rules.guard_secs;
    kv.guard_start = Some(now);
    kv.guard_lock = now + rules.guard_secs + GUARD_LOCK_SLACK;
}

/// Aplica um golpe ao herói.
///
/// Ordem do Lua: portões de i-frames/morte, `ensure_hp`, montante válido,
/// guarda/parry, defesa e só então a vida. Um parry não entra e larga a guarda
/// (com lock); o bloqueio reduz para `guard_reduction`. `died` marca o golpe
/// que levou o `hp` a 0 — quem chama decide o `start_death` (o Lua é dono do
/// `dying`, da bolsa e do som).
pub fn damage(
    kv: &mut CombatKv,
    rules: &CombatRules,
    stats_defense: f64,
    amount: f64,
    now: f64,
) -> DamageResult {
    if kv.iframes > 0.0 || kv.dying > 0.0 || kv.death_pending {
        return DamageResult::default();
    }
    ensure_hp(kv);
    if !amount.is_finite() || amount <= 0.0 {
        return DamageResult::default();
    }

    let mut blocked = false;
    let mut effective = amount;
    if now < kv.guard_until {
        let parried = kv
            .guard_start
            .is_some_and(|start| (now - start) <= rules.parry_window);
        if parried {
            // Aparo: larga a guarda e trava o [L] por `guard_secs`.
            kv.guard_until = 0.0;
            kv.guard_start = None;
            kv.guard_lock = now + rules.guard_secs;
            kv.iframes = kv.iframes.max(PARRY_IFRAMES);
            return DamageResult {
                entered: false,
                parried: true,
                blocked: false,
                died: false,
                amount: 0.0,
            };
        }
        effective *= rules.guard_reduction;
        blocked = true;
    }

    // Defesa (armadura + talentos): corta uma fração, até 60 %. Um valor não
    // finito não é defesa (o `M.stat` do Rust já o filtra).
    let defense = if stats_defense.is_finite() {
        stats_defense.clamp(0.0, DEFENSE_CAP)
    } else {
        0.0
    };
    effective *= 1.0 - defense;

    kv.hp = (kv.hp - effective).max(0.0);
    kv.iframes = HIT_IFRAMES;
    DamageResult {
        entered: true,
        parried: false,
        blocked,
        died: kv.hp <= 0.0,
        amount: effective,
    }
}

/// Cura o herói até `max_hp`. Recusa montantes inválidos e cadáver/ecrã de
/// morte (uma poção não ressuscita no sítio da queda).
///
/// `true` quando a cura foi aceite (mesmo com a vida já cheia).
pub fn heal(kv: &mut CombatKv, amount: f64) -> bool {
    ensure_hp(kv);
    if !amount.is_finite() || amount <= 0.0 {
        return false;
    }
    if kv.dying > 0.0 || kv.death_pending {
        return false;
    }
    kv.hp = kv.max_hp.min(kv.hp + amount);
    true
}

/// Sobe os i-frames para pelo menos `secs` (nunca os encurta).
///
/// `NaN` conta como "sem pedido" (o `math.max` do Lua devolve o valor actual);
/// um `iframes` guardado não finito conta como 0.
pub fn grant_iframes(kv: &mut CombatKv, secs: f64) {
    let current = if kv.iframes.is_finite() {
        kv.iframes
    } else {
        0.0
    };
    let secs = if secs.is_nan() {
        f64::NEG_INFINITY
    } else {
        secs
    };
    kv.iframes = current.max(secs);
}

// ── bridge Luau ─────────────────────────────────────────────────────────

/// Timers da guarda (sessão, nunca no save).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct GuardState {
    session_id: f64,
    until: f64,
    start: Option<f64>,
    lock: f64,
}

impl GuardState {
    /// Estado recomeçado (Novo Jogo/save de outra partida).
    fn reset(&mut self, session_id: f64) {
        *self = Self {
            session_id,
            ..Self::default()
        };
    }

    /// Copia os timers para o KV depois de acertar a sessão.
    fn load(&mut self, game: &Table, kv: &mut CombatKv) -> mlua::Result<()> {
        let session_id = read_session_id(game)?;
        if session_id != self.session_id {
            self.reset(session_id);
        }
        kv.guard_until = self.until;
        kv.guard_start = self.start;
        kv.guard_lock = self.lock;
        Ok(())
    }

    /// Guarda os timers que as funções puras acabaram de mexer.
    fn store(&mut self, kv: &CombatKv) {
        self.until = kv.guard_until;
        self.start = kv.guard_start;
        self.lock = kv.guard_lock;
    }
}

/// Plugin do combate: instala as primitivas em `viber.vael` depois das
/// quests.
pub struct VaelCombatPlugin;

impl Plugin for VaelCombatPlugin {
    fn build(&self, app: &mut App) {
        // `Startup` corre depois do `PreStartup` que carrega o `VaelContent`.
        app.add_systems(
            Startup,
            install_combat_api
                .after(crate::vault::install_vault_api)
                .after(crate::xp::install_xp_api)
                .after(crate::gear::install_gear_api)
                .after(crate::quest::install_quest_api),
        );
    }
}

/// Arranque: acrescenta as primitivas de combate à tabela `viber.vael`.
///
/// Público dentro do crate para o plugin do relógio ordenar depois dele.
pub(crate) fn install_combat_api(host: Option<Res<LuaScriptHost>>) {
    let Some(host) = host else {
        warn!("vael: combate sem host Luau — `viber.vael` não estendido");
        return;
    };
    match install(&host.lua, CombatRules::default()) {
        Ok(()) => info!("vael: combate em Rust ligado (`viber.vael`)"),
        Err(err) => error!("vael: falha a instalar combate em `viber.vael`: {err}"),
    }
}

/// Acrescenta `guard`, `damage_player`, `heal_player` e `grant_iframes` a
/// `viber.vael` sem substituir as funções que já lá estão.
pub(crate) fn install(lua: &Lua, rules: CombatRules) -> mlua::Result<()> {
    let rules = Arc::new(rules);
    let guard_session = Arc::new(Mutex::new(GuardState::default()));
    let viber: Table = lua.globals().get("viber")?;
    let vael: Table = match viber.get::<Value>("vael")? {
        Value::Table(table) => table,
        _ => {
            let table = lua.create_table()?;
            viber.set("vael", table.clone())?;
            table
        }
    };

    {
        let rules = Arc::clone(&rules);
        let session = Arc::clone(&guard_session);
        vael.set(
            "guard",
            lua.create_function(move |lua, now: Option<Value>| {
                let now = lua_number(now.unwrap_or(Value::Nil));
                let game: Table = lua.named_registry_value("viber_game")?;
                let mut kv = CombatKv::default();
                let mut session = lock_session(&session);
                session.load(&game, &mut kv)?;
                guard(&mut kv, &rules, now);
                session.store(&kv);
                Ok(())
            })?,
        )?;
    }

    {
        let rules = Arc::clone(&rules);
        let session = Arc::clone(&guard_session);
        vael.set(
            "damage_player",
            lua.create_function(move |lua, (amount, now): (Option<Value>, Option<Value>)| {
                let amount = lua_number(amount.unwrap_or(Value::Nil));
                let now = lua_number(now.unwrap_or(Value::Nil));
                let game: Table = lua.named_registry_value("viber_game")?;
                let mut kv = read_vitals(&game)?;
                {
                    let mut session = lock_session(&session);
                    session.load(&game, &mut kv)?;
                }
                let defense = stats_defense(lua)?;
                let result = damage(&mut kv, &rules, defense, amount, now);
                write_vitals(&game, &kv)?;
                {
                    let mut session = lock_session(&session);
                    session.store(&kv);
                }
                result_table(lua, result)
            })?,
        )?;
    }

    {
        vael.set(
            "heal_player",
            lua.create_function(move |lua, amount: Option<Value>| {
                let amount = lua_number(amount.unwrap_or(Value::Nil));
                let game: Table = lua.named_registry_value("viber_game")?;
                let mut kv = read_vitals(&game)?;
                let healed = heal(&mut kv, amount);
                write_vitals(&game, &kv)?;
                Ok(healed)
            })?,
        )?;
    }

    {
        vael.set(
            "grant_iframes",
            lua.create_function(move |lua, secs: Option<Value>| {
                // `secs or 0` do Lua: nil vira 0; um NaN continua NaN (o
                // `math.max` devolve o valor actual).
                let secs = match secs.unwrap_or(Value::Nil) {
                    Value::Integer(n) => n as f64,
                    Value::Number(n) => n,
                    _ => 0.0,
                };
                let game: Table = lua.named_registry_value("viber_game")?;
                let mut kv = read_vitals(&game)?;
                grant_iframes(&mut kv, secs);
                write_iframes(&game, &kv)
            })?,
        )?;
    }

    Ok(())
}

/// `Mutex` sem pânico por envenenamento: a guarda é um timer, não um convite
/// a derrubar o jogo.
fn lock_session(session: &Arc<Mutex<GuardState>>) -> std::sync::MutexGuard<'_, GuardState> {
    session.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// Lê as vitais do `viber_game`; os campos da guarda ficam no default.
fn read_vitals(game: &Table) -> mlua::Result<CombatKv> {
    Ok(CombatKv {
        // Ausente/não numérico vira NaN para o `ensure_hp` repor `max_hp`.
        hp: read_number(game, KEY_HP, f64::NAN)?,
        max_hp: read_number(game, KEY_MAX_HP, f64::NAN)?,
        // `num(chave, 0)` do Lua: o que não for número finito vale 0.
        iframes: read_number(game, KEY_IFRAMES, 0.0)?,
        dying: read_number(game, KEY_DYING, 0.0)?,
        death_pending: truthy(game.raw_get::<Value>(KEY_DEATH_PENDING)?),
        ..CombatKv::default()
    })
}

/// Escreve a vida e os i-frames; NaN nunca entra no `viber_game`.
fn write_vitals(game: &Table, kv: &CombatKv) -> mlua::Result<()> {
    write_number(game, KEY_HP, kv.hp)?;
    write_number(game, KEY_MAX_HP, kv.max_hp)?;
    write_number(game, KEY_IFRAMES, kv.iframes)
}

/// Escreve só os i-frames (`grant_iframes` não toca na vida).
fn write_iframes(game: &Table, kv: &CombatKv) -> mlua::Result<()> {
    write_number(game, KEY_IFRAMES, kv.iframes)
}

/// Lê um número do jogo; chave ausente/não numérica cai em `missing`.
fn read_number(game: &Table, key: &str, missing: f64) -> mlua::Result<f64> {
    Ok(match game.raw_get::<Value>(key)? {
        Value::Integer(n) => n as f64,
        Value::Number(n) if n.is_finite() => n,
        _ => missing,
    })
}

/// Escreve inteiro quando o valor é inteiro, `float` quando é fracionário.
fn write_number(game: &Table, key: &str, value: f64) -> mlua::Result<()> {
    if value.is_nan() {
        return Ok(());
    }
    if value.is_finite()
        && value.fract() == 0.0
        && (i64::MIN as f64..=i64::MAX as f64).contains(&value)
    {
        game.raw_set(key, value as i64)
    } else {
        game.raw_set(key, value)
    }
}

/// `session_id` do jogo; ausente/não numérico vale 0.
fn read_session_id(game: &Table) -> mlua::Result<f64> {
    read_number(game, KEY_SESSION_ID, 0.0)
}

/// Defesa do equipamento/talentos (`viber.vael.stat("defense")`).
///
/// O `stat` é instalado pelo [`crate::gear`]; sem ele a defesa é 0 (a bridge
/// funciona sozinha em testes e num host sem equipamento).
fn stats_defense(lua: &Lua) -> mlua::Result<f64> {
    let viber: Table = lua.globals().get("viber")?;
    let Some(vael) = viber.get::<Option<Table>>("vael")? else {
        return Ok(0.0);
    };
    let Some(stat) = vael.get::<Option<Function>>("stat")? else {
        return Ok(0.0);
    };
    Ok(match stat.call::<Value>("defense")? {
        Value::Integer(n) => n as f64,
        Value::Number(n) => n,
        _ => 0.0,
    })
}

/// `amount`/`now` do Lua: `nil`/não numérico vira NaN e é ignorado pelas
/// funções puras.
fn lua_number(value: Value) -> f64 {
    match value {
        Value::Integer(n) => n as f64,
        Value::Number(n) => n,
        _ => f64::NAN,
    }
}

/// Verdade do Lua (`death_pending`): só `nil` e `false` são falsos.
fn truthy(value: Value) -> bool {
    !matches!(value, Value::Nil | Value::Boolean(false))
}

fn result_table(lua: &Lua, result: DamageResult) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("entered", result.entered)?;
    table.set("parried", result.parried)?;
    table.set("blocked", result.blocked)?;
    table.set("died", result.died)?;
    table.set("amount", result.amount)?;
    Ok(table)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{ItemDef, ItemsFile};
    use crate::gear::{GearRules, install as install_gear};
    use crate::vault::{VaultRules, install as install_vault};
    use crate::xp::{XpRules, install as install_xp};

    fn regras() -> CombatRules {
        CombatRules::default()
    }

    fn kv_cheio() -> CombatKv {
        CombatKv {
            hp: 100.0,
            max_hp: 100.0,
            ..CombatKv::default()
        }
    }

    #[test]
    fn regras_por_omissao_sao_as_do_lua() {
        let rules = regras();
        assert_eq!(rules.guard_secs, 0.5);
        assert_eq!(rules.parry_window, 0.2);
        assert_eq!(rules.guard_reduction, 0.25);
    }

    #[test]
    fn guard_abre_a_janela_e_respeita_o_lock() {
        let rules = regras();
        let mut kv = CombatKv::default();
        guard(&mut kv, &rules, 10.0);
        assert_eq!(kv.guard_until, 10.5);
        assert_eq!(kv.guard_start, Some(10.0));
        assert_eq!(kv.guard_lock, 10.7);

        // Premir outra vez a meio não renova o parry.
        guard(&mut kv, &rules, 10.2);
        assert_eq!(kv.guard_start, Some(10.0));
        assert_eq!(kv.guard_until, 10.5);

        // A guarda ainda activa mas o lock já passou: reabre.
        guard(&mut kv, &rules, 10.7);
        assert_eq!(kv.guard_start, Some(10.7));
        assert!((kv.guard_until - 11.2).abs() < 1e-9);
        assert!((kv.guard_lock - 11.4).abs() < 1e-9);

        // Estado adulterado: `guard_until` no futuro trava mesmo sem lock.
        let mut travado = CombatKv {
            guard_until: 20.0,
            guard_lock: 0.0,
            ..CombatKv::default()
        };
        guard(&mut travado, &rules, 10.0);
        assert_eq!(travado.guard_start, None);
        assert_eq!(travado.guard_until, 20.0);
    }

    #[test]
    fn parry_dentro_da_janela_nao_entra_e_larga_a_guarda() {
        let rules = regras();
        let mut kv = kv_cheio();
        guard(&mut kv, &rules, 10.0);

        let result = damage(&mut kv, &rules, 0.0, 30.0, 10.1);
        assert_eq!(
            result,
            DamageResult {
                entered: false,
                parried: true,
                blocked: false,
                died: false,
                amount: 0.0,
            }
        );
        assert_eq!(kv.hp, 100.0);
        assert_eq!(kv.iframes, PARRY_IFRAMES);
        assert_eq!(kv.guard_until, 0.0);
        assert_eq!(kv.guard_start, None);
        assert!(
            (kv.guard_lock - 10.6).abs() < 1e-9,
            "lock sem a folga de 0,2 s"
        );
    }

    #[test]
    fn parry_so_vale_ate_a_janela() {
        let rules = regras();
        // 0,2 s exactos ainda aparra.
        let mut dentro = kv_cheio();
        guard(&mut dentro, &rules, 10.0);
        assert!(damage(&mut dentro, &rules, 0.0, 30.0, 10.2).parried);

        // 0,21 s já é bloqueio.
        let mut fora = kv_cheio();
        guard(&mut fora, &rules, 10.0);
        let result = damage(&mut fora, &rules, 0.0, 30.0, 10.21);
        assert!(!result.parried);
        assert!(result.blocked);
    }

    #[test]
    fn bloqueio_reduz_para_um_quarto() {
        let rules = regras();
        let mut kv = kv_cheio();
        guard(&mut kv, &rules, 10.0);

        let result = damage(&mut kv, &rules, 0.0, 30.0, 10.3);
        assert!(result.entered);
        assert!(result.blocked);
        assert!(!result.parried);
        assert!(!result.died);
        assert!((result.amount - 7.5).abs() < 1e-9);
        assert!((kv.hp - 92.5).abs() < 1e-9);
        assert_eq!(kv.iframes, HIT_IFRAMES);
    }

    #[test]
    fn defesa_corta_o_dano_e_fica_limitada() {
        let rules = regras();

        // Sem guarda e sem defesa: o golpe entra inteiro.
        let mut sem = kv_cheio();
        let result = damage(&mut sem, &rules, 0.0, 30.0, 1.0);
        assert!((result.amount - 30.0).abs() < 1e-9);

        // Guarda + defesa de 15 %.
        let mut kv = kv_cheio();
        guard(&mut kv, &rules, 10.0);
        let result = damage(&mut kv, &rules, 0.15, 30.0, 10.3);
        assert!((result.amount - 6.375).abs() < 1e-9);
        assert!((kv.hp - 93.625).abs() < 1e-9);

        // Defesa acima do tecto corta 60 %.
        let mut tecto = kv_cheio();
        guard(&mut tecto, &rules, 10.0);
        let result = damage(&mut tecto, &rules, 10.0, 30.0, 10.3);
        assert!((result.amount - 3.0).abs() < 1e-9);

        // Defesa negativa vale 0.
        let mut negativa = kv_cheio();
        guard(&mut negativa, &rules, 10.0);
        let result = damage(&mut negativa, &rules, -5.0, 30.0, 10.3);
        assert!((result.amount - 7.5).abs() < 1e-9);

        // Defesa não finita não é defesa.
        let mut invalida = kv_cheio();
        let result = damage(&mut invalida, &rules, f64::NAN, 30.0, 1.0);
        assert!((result.amount - 30.0).abs() < 1e-9);
    }

    #[test]
    fn portoes_de_iframes_e_morte_cortam_o_dano() {
        let rules = regras();
        for mut kv in [
            CombatKv {
                iframes: 0.1,
                ..kv_cheio()
            },
            CombatKv {
                dying: 1.0,
                ..kv_cheio()
            },
            CombatKv {
                death_pending: true,
                ..kv_cheio()
            },
        ] {
            let iframes = kv.iframes;
            let result = damage(&mut kv, &rules, 0.0, 30.0, 1.0);
            assert_eq!(result, DamageResult::default());
            assert_eq!(kv.hp, 100.0);
            assert_eq!(kv.iframes, iframes, "o portão não mexe nos i-frames");
        }
    }

    #[test]
    fn montantes_invalidos_sao_ignorados_mas_o_hp_e_saneado() {
        let rules = regras();
        let mut kv = CombatKv {
            hp: 250.0,
            max_hp: 100.0,
            ..CombatKv::default()
        };
        for amount in [0.0, -5.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let result = damage(&mut kv, &rules, 0.0, amount, 1.0);
            assert!(!result.entered, "montante inválido: {amount}");
            assert_eq!(kv.hp, 100.0, "ensure_hp correu antes do montante");
            assert_eq!(kv.iframes, 0.0);
        }
    }

    #[test]
    fn golpe_que_entra_leva_ao_zero_e_marca_died() {
        let rules = regras();
        let mut kv = CombatKv {
            hp: 5.0,
            max_hp: 100.0,
            ..CombatKv::default()
        };
        let result = damage(&mut kv, &rules, 0.0, 30.0, 1.0);
        assert!(result.entered);
        assert!(result.died);
        assert!((result.amount - 30.0).abs() < 1e-9);
        assert_eq!(kv.hp, 0.0, "hp nunca fica negativo");
        assert_eq!(kv.iframes, HIT_IFRAMES);
        // `dying` é escrito pelo `start_death` do Lua; a regra pura não o toca.
        assert_eq!(kv.dying, 0.0);
    }

    #[test]
    fn heal_cura_ate_max_hp_e_recusa_invalidos() {
        let mut kv = CombatKv {
            hp: 50.0,
            max_hp: 100.0,
            ..CombatKv::default()
        };
        assert!(heal(&mut kv, 30.0));
        assert_eq!(kv.hp, 80.0);
        assert!(heal(&mut kv, 30.0));
        assert_eq!(kv.hp, 100.0, "cura para no teto");

        for amount in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(!heal(&mut kv, amount), "montante inválido: {amount}");
            assert_eq!(kv.hp, 100.0);
        }
    }

    #[test]
    fn heal_recusa_cadaver_e_ecra_de_morte() {
        let mut dying = CombatKv {
            hp: 10.0,
            max_hp: 100.0,
            dying: 1.0,
            ..CombatKv::default()
        };
        assert!(!heal(&mut dying, 50.0));
        assert_eq!(dying.hp, 10.0);

        let mut pending = CombatKv {
            hp: 10.0,
            max_hp: 100.0,
            death_pending: true,
            ..CombatKv::default()
        };
        assert!(!heal(&mut pending, 50.0));
        assert_eq!(pending.hp, 10.0);
    }

    #[test]
    fn heal_sanea_max_hp_e_hp_invalidos() {
        let mut kv = CombatKv {
            hp: f64::NAN,
            max_hp: f64::NAN,
            ..CombatKv::default()
        };
        assert!(heal(&mut kv, 10.0));
        assert_eq!(kv.max_hp, DEFAULT_MAX_HP);
        assert_eq!(kv.hp, DEFAULT_MAX_HP);
    }

    #[test]
    fn grant_iframes_so_sobe() {
        let mut kv = CombatKv::default();
        grant_iframes(&mut kv, 0.5);
        assert_eq!(kv.iframes, 0.5);
        grant_iframes(&mut kv, 0.2);
        assert_eq!(kv.iframes, 0.5, "não encurta");
        grant_iframes(&mut kv, 2.0);
        assert_eq!(kv.iframes, 2.0);

        // NaN não é um pedido: o `math.max` do Lua devolve o valor actual.
        grant_iframes(&mut kv, f64::NAN);
        assert_eq!(kv.iframes, 2.0);

        // Um `iframes` guardado inválido conta como 0.
        let mut invalido = CombatKv {
            iframes: f64::NAN,
            ..CombatKv::default()
        };
        grant_iframes(&mut invalido, 0.3);
        assert_eq!(invalido.iframes, 0.3);
    }

    fn lua_com_bridge() -> (Lua, Table) {
        let lua = Lua::new();
        lua.globals()
            .set("viber", lua.create_table().expect("tabela viber"))
            .expect("global viber");
        let game = lua.create_table().expect("tabela do jogo");
        lua.set_named_registry_value("viber_game", game.clone())
            .expect("registar viber_game");
        install(&lua, regras()).expect("instalar viber.vael");
        (lua, game)
    }

    #[test]
    fn bridge_guarda_parry_e_bloqueio_no_viber_game() {
        let (lua, game) = lua_com_bridge();
        game.raw_set(KEY_HP, 100).expect("hp");
        game.raw_set(KEY_MAX_HP, 100).expect("max_hp");

        lua.load("viber.vael.guard(10.0)")
            .exec()
            .expect("abrir guarda");
        let parried: bool = lua
            .load("return viber.vael.damage_player(30, 10.1).parried")
            .eval()
            .expect("parry");
        assert!(parried);
        assert_eq!(game.raw_get::<f64>(KEY_HP).expect("hp"), 100.0);
        assert!((game.raw_get::<f64>(KEY_IFRAMES).expect("iframes") - PARRY_IFRAMES).abs() < 1e-9);

        // O parry deu i-frames; limpa-os para o golpe seguinte entrar. O lock
        // ainda trava o [L] até 10,5.
        game.raw_set(KEY_IFRAMES, 0).expect("iframes");
        let guardado: bool = lua
            .load("viber.vael.guard(10.4); return viber.vael.damage_player(30, 10.45).blocked")
            .eval()
            .expect("guarda fechada");
        assert!(!guardado, "o lock impede reabrir a guarda");
        assert_eq!(game.raw_get::<f64>(KEY_HP).expect("hp"), 70.0);

        // Depois do lock a guarda abre e o bloqueio reduz o dano.
        game.raw_set(KEY_IFRAMES, 0).expect("iframes");
        let blocked: bool = lua
            .load("viber.vael.guard(20.0); return viber.vael.damage_player(30, 20.3).blocked")
            .eval()
            .expect("bloqueio");
        assert!(blocked);
        assert_eq!(game.raw_get::<f64>(KEY_HP).expect("hp"), 62.5);
        assert_eq!(
            game.raw_get::<f64>(KEY_IFRAMES).expect("iframes"),
            HIT_IFRAMES
        );
    }

    #[test]
    fn bridge_cura_e_i_frames() {
        let (lua, game) = lua_com_bridge();
        game.raw_set(KEY_HP, 50).expect("hp");
        game.raw_set(KEY_MAX_HP, 100).expect("max_hp");

        let healed: bool = lua
            .load("return viber.vael.heal_player(30)")
            .eval()
            .expect("heal");
        assert!(healed);
        assert_eq!(game.raw_get::<f64>(KEY_HP).expect("hp"), 80.0);

        let healed: bool = lua
            .load("return viber.vael.heal_player(999)")
            .eval()
            .expect("heal no tecto");
        assert!(healed);
        assert_eq!(game.raw_get::<f64>(KEY_HP).expect("hp"), 100.0);

        lua.load("viber.vael.grant_iframes(0.9)")
            .exec()
            .expect("grant");
        assert_eq!(game.raw_get::<f64>(KEY_IFRAMES).expect("iframes"), 0.9);
        lua.load("viber.vael.grant_iframes(0.2)")
            .exec()
            .expect("grant menor");
        assert_eq!(game.raw_get::<f64>(KEY_IFRAMES).expect("iframes"), 0.9);
    }

    #[test]
    fn bridge_resultado_de_dano_e_morte() {
        let (lua, game) = lua_com_bridge();
        game.raw_set(KEY_HP, 5).expect("hp");
        game.raw_set(KEY_MAX_HP, 100).expect("max_hp");

        let died: bool = lua
            .load("return viber.vael.damage_player(30, 1.0).died")
            .eval()
            .expect("dano fatal");
        assert!(died);
        assert_eq!(game.raw_get::<f64>(KEY_HP).expect("hp"), 0.0);

        // O `dying` não é da regra: fica para o `start_death` do Lua.
        assert!(matches!(
            game.raw_get::<Value>(KEY_DYING).expect("dying"),
            Value::Nil
        ));
    }

    #[test]
    fn bridge_respeita_os_portoes_e_a_sessao() {
        let (lua, game) = lua_com_bridge();
        game.raw_set(KEY_HP, 100).expect("hp");
        game.raw_set(KEY_MAX_HP, 100).expect("max_hp");

        game.raw_set(KEY_IFRAMES, 0.2).expect("iframes");
        let entered: bool = lua
            .load("return viber.vael.damage_player(30, 1.0).entered")
            .eval()
            .expect("portão");
        assert!(!entered);
        assert_eq!(game.raw_get::<f64>(KEY_HP).expect("hp"), 100.0);

        // Uma sessão nova esquece a guarda da partida anterior.
        game.raw_set(KEY_IFRAMES, 0).expect("iframes");
        lua.load("viber.vael.guard(10.0)").exec().expect("guarda");
        game.raw_set(KEY_SESSION_ID, 1).expect("sessão");
        let parried: bool = lua
            .load("return viber.vael.damage_player(30, 10.1).parried")
            .eval()
            .expect("guarda da sessão antiga");
        assert!(!parried, "a sessão nova não herda a guarda");
    }

    #[test]
    fn bridge_usa_a_defesa_do_equipamento() {
        let lua = Lua::new();
        lua.globals()
            .set("viber", lua.create_table().expect("tabela viber"))
            .expect("global viber");
        let game = lua.create_table().expect("tabela do jogo");
        lua.set_named_registry_value("viber_game", game.clone())
            .expect("registar viber_game");
        install_vault(&lua, VaultRules::from_items(&ItemsFile::default())).expect("cofre");
        install_xp(&lua, XpRules::default()).expect("xp");

        let file = ItemsFile {
            items: vec![ItemDef {
                id: "manto".into(),
                slot: "armor".into(),
                effects: [("defense".to_string(), 0.15)].into_iter().collect(),
                ..Default::default()
            }],
            ..Default::default()
        };
        install_gear(&lua, GearRules::from_items(&file)).expect("equipamento");
        install(&lua, regras()).expect("combate");

        game.raw_set("item.manto", 1).expect("manto");
        game.raw_set("gear.armor", "manto").expect("slot");
        game.raw_set(KEY_HP, 100).expect("hp");
        game.raw_set(KEY_MAX_HP, 100).expect("max_hp");

        lua.load("viber.vael.guard(10.0)").exec().expect("guarda");
        let amount: f64 = lua
            .load("return viber.vael.damage_player(30, 10.3).amount")
            .eval()
            .expect("dano com defesa");
        // 30 * 0,25 (guarda) * 0,85 (defesa de 15 %).
        assert!((amount - 6.375).abs() < 1e-9, "amount={amount}");
        assert!((game.raw_get::<f64>(KEY_HP).expect("hp") - 93.625).abs() < 1e-9);
    }

    #[test]
    fn bridge_convive_com_cofre_xp_equipamento_e_quests() {
        let lua = Lua::new();
        lua.globals()
            .set("viber", lua.create_table().expect("tabela viber"))
            .expect("global viber");
        let game = lua.create_table().expect("tabela do jogo");
        lua.set_named_registry_value("viber_game", game)
            .expect("registar viber_game");
        install_vault(&lua, VaultRules::from_items(&ItemsFile::default())).expect("cofre");
        install_xp(&lua, XpRules::default()).expect("xp");
        install_gear(&lua, GearRules::default()).expect("equipamento");
        crate::quest::install(
            &lua,
            crate::quest::QuestRules::default(),
            VaultRules::from_items(&ItemsFile::default()),
            XpRules::default(),
        )
        .expect("quests");
        install(&lua, regras()).expect("combate");

        let coexistem: bool = lua
            .load(
                "return type(viber.vael.vault_add) == 'function' \
                 and type(viber.vael.add_xp) == 'function' \
                 and type(viber.vael.gear_toggle) == 'function' \
                 and type(viber.vael.quest_state) == 'function' \
                 and type(viber.vael.guard) == 'function' \
                 and type(viber.vael.damage_player) == 'function' \
                 and type(viber.vael.heal_player) == 'function' \
                 and type(viber.vael.grant_iframes) == 'function'",
            )
            .eval()
            .expect("funções instaladas");
        assert!(coexistem, "combate partilha `viber.vael`");
    }

    #[test]
    fn plugin_sem_host_nao_panica() {
        let mut app = App::new();
        app.add_plugins(bevy::MinimalPlugins)
            .add_plugins(VaelCombatPlugin);
        for _ in 0..2 {
            app.update();
        }
    }

    #[test]
    fn startup_instala_combate_na_mesma_tabela() {
        let mut app = App::new();
        app.add_plugins(bevy::MinimalPlugins)
            .insert_resource(LuaScriptHost::new(std::path::PathBuf::from("scripts")).expect("host"))
            .init_resource::<crate::content::VaelContent>()
            .add_plugins((
                crate::vault::VaelVaultPlugin,
                crate::xp::VaelXpPlugin,
                crate::gear::VaelGearPlugin,
                crate::quest::VaelQuestPlugin,
                VaelCombatPlugin,
            ));
        app.update();

        let host = app.world().resource::<LuaScriptHost>();
        let coexistem: bool = host
            .lua
            .load(
                "return type(viber.vael.vault_add) == 'function' \
                 and type(viber.vael.add_xp) == 'function' \
                 and type(viber.vael.gear_toggle) == 'function' \
                 and type(viber.vael.quest_state) == 'function' \
                 and type(viber.vael.guard) == 'function' \
                 and type(viber.vael.damage_player) == 'function' \
                 and type(viber.vael.heal_player) == 'function' \
                 and type(viber.vael.grant_iframes) == 'function'",
            )
            .eval()
            .expect("funções instaladas");
        assert!(coexistem, "combate tem de partilhar `viber.vael`");
    }
}

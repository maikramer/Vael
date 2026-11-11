//! XP, nível e pontos de talento em Rust: a aritmética do `rules.lua`.
//!
//! As funções puras trabalham sobre um `BTreeMap<String, f64>` — as quatro
//! chaves de `viber.game()` (`xp`, `xp_next`, `level`, `skill_points`) — para
//! poderem ser testadas sem a VM. Os valores são `f64` como no Lua: o
//! `ensure_xp` do Lua não trunca `xp` (só o nível e os pontos), por isso um
//! ganho fracionário continua fracionário. A cópia tipada
//! [`crate::components::XpState`] é `i64`: [`ensure_xp`] devolve-a com
//! `xp`/`xp_next` truncados e `skills` vazio (o saneamento não toca nos
//! talentos) — o mapa é que fica fiel à VM.
//!
//! A bridge [`install`] instala `viber.vael.ensure_xp` / `add_xp` /
//! `add_skill_points` depois de o [`crate::vault`] criar a tabela; lê e
//! escreve a tabela `viber_game` da VM partilhada. O `rules.lua` fica só com
//! os embrulhos finos.
//!
//! Regras portadas de `scripts/lib/rules.lua`:
//! - `ensure_xp`: `xp >= 0`, `xp_next >= XP_BASE`, `level >= 1` e
//!   `skill_points >= 0`; nível e pontos passam por `floor`;
//! - `add_xp`: ganho não finito ou ≤ 0 é ignorado; sobe vários níveis num só
//!   ganho; `xp_next` cresce com `ceil(xp_next * growth)` e cada nível paga
//!   `points_per_level`;
//! - `add_skill_points`: valor não finito ou ≤ 0 é ignorado; a carteira fica
//!   inteira (o `n` é truncado, como o `floor` seguinte do `ensure_xp`).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use bevy::prelude::*;
use mlua::{Lua, Table, Value};
use viber::luau::host::LuaScriptHost;

use crate::components::XpState;
use crate::content::{ProgressionFile, VaelContent};

/// Defaults do `rules.lua` quando o `data/progression.json` não traz o campo.
pub const DEFAULT_XP_BASE: i64 = 100;
pub const DEFAULT_XP_GROWTH: f64 = 1.5;
pub const DEFAULT_SKILL_POINTS_PER_LEVEL: i64 = 1;

/// Chaves de progressão no `viber.game()`.
pub const KEY_XP: &str = "xp";
pub const KEY_XP_NEXT: &str = "xp_next";
pub const KEY_LEVEL: &str = "level";
pub const KEY_SKILL_POINTS: &str = "skill_points";

/// As quatro chaves numéricas da progressão, na ordem em que são escritas.
pub const XP_KEYS: [&str; 4] = [KEY_XP, KEY_XP_NEXT, KEY_LEVEL, KEY_SKILL_POINTS];

/// Mapa Lua-friendly da progressão. `f64` como os números do Luau.
pub type XpMap = BTreeMap<String, f64>;

/// Curva de XP derivada de `data/progression.json`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct XpRules {
    /// XP do primeiro nível (`XP_BASE`), nunca < 1.
    pub base: i64,
    /// Fator da meta seguinte (`XP_GROWTH`).
    pub growth: f64,
    /// Pontos de talento por nível (`SKILL_POINTS_PER_LEVEL`), nunca < 0.
    pub points_per_level: i64,
}

impl Default for XpRules {
    fn default() -> Self {
        Self {
            base: DEFAULT_XP_BASE,
            growth: DEFAULT_XP_GROWTH,
            points_per_level: DEFAULT_SKILL_POINTS_PER_LEVEL,
        }
    }
}

impl XpRules {
    /// Constrói a curva a partir do conteúdo carregado.
    ///
    /// O serde não distingue um campo ausente de um `0` explícito; como no
    /// [`crate::vault`], um `0` conta como ausente e cai no default. Um
    /// `base` negativo vale 1 e pontos negativos valem 0 (o `max` do Lua).
    pub fn from_progression(file: &ProgressionFile) -> Self {
        Self {
            base: if file.xp.base != 0 {
                file.xp.base.max(1)
            } else {
                DEFAULT_XP_BASE
            },
            growth: if file.xp.growth != 0.0 && file.xp.growth.is_finite() {
                file.xp.growth
            } else {
                DEFAULT_XP_GROWTH
            },
            points_per_level: if file.xp.skill_points_per_level != 0 {
                file.xp.skill_points_per_level.max(0)
            } else {
                DEFAULT_SKILL_POINTS_PER_LEVEL
            },
        }
    }
}

/// O que um `add_xp` rendeu: níveis subidos, pontos ganhos e o estado final.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct XpGain {
    pub levels: i64,
    pub points: i64,
    pub xp: i64,
    pub xp_next: i64,
}

/// Saneia as quatro chaves como o `ensure_xp` do Lua e devolve a cópia tipada.
///
/// `xp` e `xp_next` não são truncados: o Lua só os corrige quando negativos
/// (ou não finitos). Nível e pontos são `floor`. A cópia [`XpState`] trunca
/// `xp`/`xp_next` para `i64` e deixa `skills` vazio — são só os números.
pub fn ensure_xp(kv: &mut XpMap, rules: &XpRules) -> XpState {
    let xp = match kv.get(KEY_XP).copied() {
        Some(value) if value.is_finite() && value >= 0.0 => value,
        _ => 0.0,
    };
    let xp_next = match kv.get(KEY_XP_NEXT).copied() {
        Some(value) if value.is_finite() && value >= 1.0 => value,
        _ => rules.base.max(1) as f64,
    };
    let level = match kv.get(KEY_LEVEL).copied() {
        Some(value) if value.is_finite() && value >= 1.0 => value.floor(),
        _ => 1.0,
    };
    let skill_points = match kv.get(KEY_SKILL_POINTS).copied() {
        Some(value) if value.is_finite() && value >= 0.0 => value.floor(),
        _ => 0.0,
    };

    kv.insert(KEY_XP.into(), xp);
    kv.insert(KEY_XP_NEXT.into(), xp_next);
    kv.insert(KEY_LEVEL.into(), level);
    kv.insert(KEY_SKILL_POINTS.into(), skill_points);

    XpState {
        xp: xp as i64,
        xp_next: xp_next as i64,
        level: level as i64,
        skill_points: skill_points as i64,
        skills: BTreeSet::new(),
    }
}

/// Soma XP e sobe todos os níveis que o ganho atravessar.
///
/// Ganho não finito ou ≤ 0 é ignorado sem tocar no mapa (o `return` do Lua
/// acontece antes do `ensure_xp`). A meta seguinte cresce com `ceil` e nunca
/// desce de 1.
pub fn add_xp(kv: &mut XpMap, rules: &XpRules, gain: f64) -> XpGain {
    if !gain.is_finite() || gain <= 0.0 {
        return XpGain::default();
    }

    ensure_xp(kv, rules);

    let mut xp = kv.get(KEY_XP).copied().unwrap_or(0.0);
    let mut xp_next = kv
        .get(KEY_XP_NEXT)
        .copied()
        .unwrap_or(rules.base.max(1) as f64);
    let mut level = kv.get(KEY_LEVEL).copied().unwrap_or(1.0) as i64;
    let mut skill_points = kv.get(KEY_SKILL_POINTS).copied().unwrap_or(0.0) as i64;
    let points_per_level = rules.points_per_level.max(0);

    xp += gain;

    let mut levels = 0i64;
    while xp >= xp_next {
        xp -= xp_next;
        xp_next = (xp_next * rules.growth).ceil().max(1.0);
        level = level.saturating_add(1);
        skill_points = skill_points.saturating_add(points_per_level);
        levels = levels.saturating_add(1);
    }

    kv.insert(KEY_XP.into(), xp);
    kv.insert(KEY_XP_NEXT.into(), xp_next);
    kv.insert(KEY_LEVEL.into(), level as f64);
    kv.insert(KEY_SKILL_POINTS.into(), skill_points as f64);

    XpGain {
        levels,
        points: levels.saturating_mul(points_per_level),
        xp: xp as i64,
        xp_next: xp_next as i64,
    }
}

/// Soma pontos de talento inteiros; `n` não finito ou ≤ 0 é ignorado.
///
/// O `n` é truncado (`floor`, porque é positivo), o que dá o mesmo total que
/// o `floor` do `ensure_xp` seguinte. A carteira guardada é saneada aqui para
/// nunca ficar negativa, mesmo sem um `ensure_xp` antes.
pub fn add_skill_points(kv: &mut XpMap, n: f64) {
    if !n.is_finite() || n <= 0.0 {
        return;
    }
    let current = match kv.get(KEY_SKILL_POINTS).copied() {
        Some(value) if value.is_finite() && value >= 0.0 => value.floor(),
        _ => 0.0,
    };
    kv.insert(KEY_SKILL_POINTS.into(), current + n.floor());
}

// ── bridge Luau ─────────────────────────────────────────────────────────

/// Plugin do XP: instala as funções em `viber.vael` depois do cofre.
pub struct VaelXpPlugin;

impl Plugin for VaelXpPlugin {
    fn build(&self, app: &mut App) {
        // `Startup` corre depois do `PreStartup` que carrega o `VaelContent`;
        // `.after` garante que a tabela `viber.vael` do cofre já existe.
        app.add_systems(
            Startup,
            install_xp_api.after(crate::vault::install_vault_api),
        );
    }
}

/// Arranque: acrescenta as funções de XP à tabela `viber.vael`.
///
/// Público dentro do crate para o plugin do equipamento ordenar depois dele.
pub(crate) fn install_xp_api(host: Option<Res<LuaScriptHost>>, content: Option<Res<VaelContent>>) {
    let Some(host) = host else {
        warn!("vael: XP sem host Luau — `viber.vael` não estendido");
        return;
    };
    let Some(content) = content else {
        warn!("vael: XP sem conteúdo carregado — `viber.vael` não estendido");
        return;
    };
    let rules = XpRules::from_progression(&content.progression);
    match install(&host.lua, rules) {
        Ok(()) => info!(
            base = rules.base,
            crescimento = rules.growth,
            pontos_por_nivel = rules.points_per_level,
            "vael: XP em Rust ligado (`viber.vael`)"
        ),
        Err(err) => error!("vael: falha a instalar XP em `viber.vael`: {err}"),
    }
}

/// Acrescenta `ensure_xp`, `add_xp` e `add_skill_points` a `viber.vael`.
///
/// Usa a tabela já criada pelo cofre; só cria uma nova se esta não existir
/// (nunca substitui as funções do cofre).
pub(crate) fn install(lua: &Lua, rules: XpRules) -> mlua::Result<()> {
    let rules = Arc::new(rules);
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
        vael.set(
            "ensure_xp",
            lua.create_function(move |lua, ()| {
                let game: Table = lua.named_registry_value("viber_game")?;
                let mut kv = read_xp(&game)?;
                ensure_xp(&mut kv, &rules);
                write_xp(&game, &kv)
            })?,
        )?;
    }

    {
        let rules = Arc::clone(&rules);
        vael.set(
            "add_xp",
            lua.create_function(move |lua, gain: Option<Value>| {
                let gain = lua_number(gain.unwrap_or(Value::Nil));
                if !gain.is_finite() || gain <= 0.0 {
                    return Ok(());
                }
                let game: Table = lua.named_registry_value("viber_game")?;
                let mut kv = read_xp(&game)?;
                add_xp(&mut kv, &rules, gain);
                write_xp(&game, &kv)
            })?,
        )?;
    }

    {
        let rules = Arc::clone(&rules);
        vael.set(
            "add_skill_points",
            lua.create_function(move |lua, n: Option<Value>| {
                let n = lua_number(n.unwrap_or(Value::Nil));
                if !n.is_finite() || n <= 0.0 {
                    return Ok(());
                }
                let game: Table = lua.named_registry_value("viber_game")?;
                let mut kv = read_xp(&game)?;
                // O Lua chama `ensure_xp()` antes de somar.
                ensure_xp(&mut kv, &rules);
                add_skill_points(&mut kv, n);
                write_xp(&game, &kv)
            })?,
        )?;
    }

    Ok(())
}

/// Lê as quatro chaves numéricas do `viber_game`; o resto fica de fora para o
/// `ensure_xp` aplicar os defaults.
fn read_xp(game: &Table) -> mlua::Result<XpMap> {
    let mut kv = XpMap::new();
    for key in XP_KEYS {
        match game.raw_get::<Value>(key)? {
            Value::Integer(n) => {
                kv.insert(key.to_string(), n as f64);
            }
            Value::Number(n) => {
                kv.insert(key.to_string(), n);
            }
            _ => {}
        }
    }
    Ok(kv)
}

/// Escreve as chaves presentes no mapa de volta no `viber_game`.
fn write_xp(game: &Table, kv: &XpMap) -> mlua::Result<()> {
    for key in XP_KEYS {
        if let Some(value) = kv.get(key).copied() {
            write_number(game, key, value)?;
        }
    }
    Ok(())
}

/// Escreve inteiro quando o valor é inteiro, `float` quando é fracionário.
fn write_number(game: &Table, key: &str, value: f64) -> mlua::Result<()> {
    if value.is_finite()
        && value.fract() == 0.0
        && (i64::MIN as f64..=i64::MAX as f64).contains(&value)
    {
        game.raw_set(key, value as i64)
    } else {
        game.raw_set(key, value)
    }
}

/// Argumento numérico do Lua: `nil`/não numérico vira NaN e é ignorado.
fn lua_number(value: Value) -> f64 {
    match value {
        Value::Integer(n) => n as f64,
        Value::Number(n) => n,
        _ => f64::NAN,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{ItemsFile, ProgressionFile};
    use crate::vault::{VaultRules, install as install_vault};

    fn regras() -> XpRules {
        XpRules {
            base: 100,
            growth: 1.5,
            points_per_level: 1,
        }
    }

    fn mapa(pares: &[(&str, f64)]) -> XpMap {
        pares
            .iter()
            .map(|(chave, valor)| (chave.to_string(), *valor))
            .collect()
    }

    #[test]
    fn regras_vem_do_conteudo_e_caem_nos_defaults() {
        let mut file = ProgressionFile::default();
        assert_eq!(XpRules::from_progression(&file), XpRules::default());

        file.xp.base = 50;
        file.xp.growth = 2.0;
        file.xp.skill_points_per_level = 2;
        assert_eq!(
            XpRules::from_progression(&file),
            XpRules {
                base: 50,
                growth: 2.0,
                points_per_level: 2,
            }
        );

        file.xp.base = -3;
        file.xp.skill_points_per_level = -2;
        let rules = XpRules::from_progression(&file);
        assert_eq!(rules.base, 1);
        assert_eq!(rules.points_per_level, 0);
    }

    #[test]
    fn ensure_xp_sanea_negativos_e_ausentes() {
        let rules = regras();
        let mut kv = mapa(&[
            (KEY_XP, -5.0),
            (KEY_XP_NEXT, 0.0),
            (KEY_LEVEL, 0.0),
            (KEY_SKILL_POINTS, -2.0),
        ]);
        let state = ensure_xp(&mut kv, &rules);
        assert_eq!(kv[KEY_XP], 0.0);
        assert_eq!(kv[KEY_XP_NEXT], 100.0);
        assert_eq!(kv[KEY_LEVEL], 1.0);
        assert_eq!(kv[KEY_SKILL_POINTS], 0.0);
        assert_eq!(
            state,
            XpState {
                xp: 0,
                xp_next: 100,
                level: 1,
                skill_points: 0,
                skills: BTreeSet::new(),
            }
        );

        // Chaves ausentes: os mesmos defaults.
        let mut vazio = XpMap::new();
        ensure_xp(&mut vazio, &rules);
        assert_eq!(vazio[KEY_XP], 0.0);
        assert_eq!(vazio[KEY_XP_NEXT], 100.0);
        assert_eq!(vazio[KEY_LEVEL], 1.0);
        assert_eq!(vazio[KEY_SKILL_POINTS], 0.0);
    }

    #[test]
    fn ensure_xp_sanea_nan_e_arredonda_nivel_e_pontos() {
        let rules = regras();
        let mut kv = mapa(&[
            (KEY_XP, f64::NAN),
            (KEY_XP_NEXT, f64::INFINITY),
            (KEY_LEVEL, 3.7),
            (KEY_SKILL_POINTS, 2.9),
        ]);
        let state = ensure_xp(&mut kv, &rules);
        assert_eq!(kv[KEY_XP], 0.0);
        assert_eq!(kv[KEY_XP_NEXT], 100.0);
        assert_eq!(kv[KEY_LEVEL], 3.0);
        assert_eq!(kv[KEY_SKILL_POINTS], 2.0);
        assert_eq!(state.level, 3);
        assert_eq!(state.skill_points, 2);

        // O Lua não trunca `xp`/`xp_next`: a cópia tipada é que trunca.
        let mut fracionario = mapa(&[(KEY_XP, 4.5), (KEY_XP_NEXT, 149.5)]);
        let state = ensure_xp(&mut fracionario, &rules);
        assert_eq!(fracionario[KEY_XP], 4.5);
        assert_eq!(fracionario[KEY_XP_NEXT], 149.5);
        assert_eq!(state.xp, 4);
        assert_eq!(state.xp_next, 149);
    }

    #[test]
    fn add_xp_ignora_ganhos_invalidos_sem_sanear() {
        let rules = regras();
        let mut kv = mapa(&[(KEY_XP, -5.0), (KEY_LEVEL, 0.0)]);
        for gain in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let out = add_xp(&mut kv, &rules, gain);
            assert_eq!(out, XpGain::default(), "ganho inválido: {gain}");
        }
        // O `return` do Lua acontece antes do `ensure_xp`.
        assert_eq!(kv[KEY_XP], -5.0);
        assert_eq!(kv[KEY_LEVEL], 0.0);
    }

    #[test]
    fn add_xp_sobe_um_nivel() {
        let rules = regras();
        let mut kv = mapa(&[
            (KEY_XP, 90.0),
            (KEY_XP_NEXT, 100.0),
            (KEY_LEVEL, 1.0),
            (KEY_SKILL_POINTS, 0.0),
        ]);
        let out = add_xp(&mut kv, &rules, 20.0);
        assert_eq!(
            out,
            XpGain {
                levels: 1,
                points: 1,
                xp: 10,
                xp_next: 150,
            }
        );
        assert_eq!(kv[KEY_XP], 10.0);
        assert_eq!(kv[KEY_XP_NEXT], 150.0);
        assert_eq!(kv[KEY_LEVEL], 2.0);
        assert_eq!(kv[KEY_SKILL_POINTS], 1.0);
    }

    #[test]
    fn add_xp_sobe_varios_niveis_e_arredonda_a_meta() {
        let rules = regras();
        let mut kv = mapa(&[
            (KEY_XP, 0.0),
            (KEY_XP_NEXT, 100.0),
            (KEY_LEVEL, 1.0),
            (KEY_SKILL_POINTS, 0.0),
        ]);
        // 250 = 100 + 150: dois níveis, meta seguinte 225 e zero de sobra.
        let out = add_xp(&mut kv, &rules, 250.0);
        assert_eq!(
            out,
            XpGain {
                levels: 2,
                points: 2,
                xp: 0,
                xp_next: 225,
            }
        );
        assert_eq!(kv[KEY_LEVEL], 3.0);
        assert_eq!(kv[KEY_SKILL_POINTS], 2.0);

        // 225 * 1.5 = 337.5 → ceil → 338; um ganho de 337.5 fica abaixo.
        let out = add_xp(&mut kv, &rules, 562.5);
        assert_eq!(out.levels, 1);
        assert_eq!(out.xp, 337);
        assert_eq!(out.xp_next, 338);
        assert_eq!(kv[KEY_XP], 337.5);
        assert_eq!(kv[KEY_XP_NEXT], 338.0);
        assert_eq!(kv[KEY_LEVEL], 4.0);
        assert_eq!(kv[KEY_SKILL_POINTS], 3.0);
    }

    #[test]
    fn add_xp_sem_pontos_por_nivel_nao_mexe_na_carteira() {
        let rules = XpRules {
            base: 10,
            growth: 2.0,
            points_per_level: 0,
        };
        let mut kv = mapa(&[
            (KEY_XP, 0.0),
            (KEY_XP_NEXT, 10.0),
            (KEY_LEVEL, 1.0),
            (KEY_SKILL_POINTS, 7.0),
        ]);
        let out = add_xp(&mut kv, &rules, 30.0);
        assert_eq!(out.levels, 2);
        assert_eq!(out.points, 0);
        assert_eq!(kv[KEY_LEVEL], 3.0);
        assert_eq!(kv[KEY_SKILL_POINTS], 7.0);
    }

    #[test]
    fn add_skill_points_ignora_invalidos() {
        let mut kv = mapa(&[(KEY_SKILL_POINTS, 2.0)]);
        for n in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            add_skill_points(&mut kv, n);
        }
        assert_eq!(kv[KEY_SKILL_POINTS], 2.0);
    }

    #[test]
    fn add_skill_points_mantem_a_carteira_inteira() {
        let mut kv = mapa(&[(KEY_SKILL_POINTS, 2.0)]);
        add_skill_points(&mut kv, 1.0);
        assert_eq!(kv[KEY_SKILL_POINTS], 3.0);
        // Fração: fica o inteiro (o `floor` do `ensure_xp` seguinte daria 4).
        add_skill_points(&mut kv, 1.9);
        assert_eq!(kv[KEY_SKILL_POINTS], 4.0);
        // Saldo negativo sem `ensure_xp` antes: corrige para 0 + n.
        let mut negativo = mapa(&[(KEY_SKILL_POINTS, -3.0)]);
        add_skill_points(&mut negativo, 2.0);
        assert_eq!(negativo[KEY_SKILL_POINTS], 2.0);
    }

    fn lua_com_bridge() -> (Lua, Table) {
        let lua = Lua::new();
        lua.globals()
            .set("viber", lua.create_table().expect("tabela viber"))
            .expect("global viber");
        let game = lua.create_table().expect("tabela do jogo");
        lua.set_named_registry_value("viber_game", game.clone())
            .expect("registar viber_game");
        // Ordem de produção: cofre primeiro, XP depois.
        install_vault(&lua, VaultRules::from_items(&ItemsFile::default())).expect("instalar cofre");
        install(&lua, regras()).expect("instalar xp");
        (lua, game)
    }

    #[test]
    fn bridge_add_xp_le_e_escreve_no_viber_game() {
        let (lua, game) = lua_com_bridge();
        game.raw_set(KEY_XP, 90).expect("xp");
        game.raw_set(KEY_XP_NEXT, 100).expect("xp_next");
        game.raw_set(KEY_LEVEL, 1).expect("level");
        game.raw_set(KEY_SKILL_POINTS, 0).expect("skill_points");

        lua.load("viber.vael.add_xp(20)").exec().expect("add_xp");

        assert_eq!(game.raw_get::<f64>(KEY_XP).expect("xp"), 10.0);
        assert_eq!(game.raw_get::<f64>(KEY_XP_NEXT).expect("xp_next"), 150.0);
        assert_eq!(game.raw_get::<f64>(KEY_LEVEL).expect("level"), 2.0);
        assert_eq!(game.raw_get::<f64>(KEY_SKILL_POINTS).expect("pontos"), 1.0);

        // `nil`/negativo: o Lua antigo ignorava.
        lua.load("viber.vael.add_xp(nil) viber.vael.add_xp(-5)")
            .exec()
            .expect("add_xp inválido");
        assert_eq!(game.raw_get::<f64>(KEY_XP).expect("xp"), 10.0);
        assert_eq!(game.raw_get::<f64>(KEY_LEVEL).expect("level"), 2.0);
    }

    #[test]
    fn bridge_ensure_xp_sanea_o_jogo() {
        let (lua, game) = lua_com_bridge();
        game.raw_set(KEY_XP, -5).expect("xp");
        game.raw_set(KEY_XP_NEXT, f64::NAN).expect("xp_next");
        game.raw_set(KEY_LEVEL, 0).expect("level");
        game.raw_set(KEY_SKILL_POINTS, -2).expect("pontos");

        lua.load("viber.vael.ensure_xp()")
            .exec()
            .expect("ensure_xp");

        assert_eq!(game.raw_get::<f64>(KEY_XP).expect("xp"), 0.0);
        assert_eq!(game.raw_get::<f64>(KEY_XP_NEXT).expect("xp_next"), 100.0);
        assert_eq!(game.raw_get::<f64>(KEY_LEVEL).expect("level"), 1.0);
        assert_eq!(game.raw_get::<f64>(KEY_SKILL_POINTS).expect("pontos"), 0.0);
    }

    #[test]
    fn bridge_add_skill_points_e_cofre_sobrevivem() {
        let (lua, game) = lua_com_bridge();
        game.raw_set(KEY_SKILL_POINTS, 2).expect("pontos");

        lua.load("viber.vael.add_skill_points(3)")
            .exec()
            .expect("add_skill_points");
        assert_eq!(game.raw_get::<f64>(KEY_SKILL_POINTS).expect("pontos"), 5.0);

        lua.load("viber.vael.add_skill_points(nil)")
            .exec()
            .expect("add_skill_points inválido");
        assert_eq!(game.raw_get::<f64>(KEY_SKILL_POINTS).expect("pontos"), 5.0);

        // A tabela é a do cofre: as funções antigas continuam lá.
        let added: i64 = lua
            .load("return viber.vael.vault_add('potion', 2)")
            .eval()
            .expect("vault_add");
        assert_eq!(added, 2);
        assert_eq!(game.raw_get::<i64>("item.potion").expect("potion"), 2);
    }

    #[test]
    fn plugin_sem_host_nao_panica() {
        let mut app = App::new();
        app.add_plugins(bevy::MinimalPlugins)
            .add_plugins((crate::vault::VaelVaultPlugin, VaelXpPlugin));
        for _ in 0..2 {
            app.update();
        }
    }

    #[test]
    fn startup_instala_xp_na_mesma_tabela_do_cofre() {
        let mut app = App::new();
        app.add_plugins(bevy::MinimalPlugins)
            .insert_resource(LuaScriptHost::new(std::path::PathBuf::from("scripts")).expect("host"))
            .init_resource::<VaelContent>()
            .add_plugins((crate::vault::VaelVaultPlugin, VaelXpPlugin));
        app.update();

        let host = app.world().resource::<LuaScriptHost>();
        let lua = &host.lua;
        let coexistem: bool = lua
            .load(
                "return type(viber.vael.vault_add) == 'function' \
                 and type(viber.vael.ensure_xp) == 'function' \
                 and type(viber.vael.add_xp) == 'function' \
                 and type(viber.vael.add_skill_points) == 'function'",
            )
            .eval()
            .expect("funções instaladas");
        assert!(coexistem, "cofre e XP têm de partilhar `viber.vael`");

        lua.load("viber.vael.add_xp(150) viber.vael.add_skill_points(1)")
            .exec()
            .expect("add_xp/add_skill_points");
        let level: i64 = lua.load("return viber.game().level").eval().expect("nível");
        let pontos: i64 = lua
            .load("return viber.game().skill_points")
            .eval()
            .expect("pontos");
        assert_eq!(level, 2);
        assert_eq!(pontos, 2);
    }
}

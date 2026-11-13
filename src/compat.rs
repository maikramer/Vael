//! Coerções partilhadas entre o Lua, o `viber_game` e as env vars de QA.
//!
//! Cada módulo da bridge `viber.vael` lia argumentos e chaves do jogo com a
//! mesma tolerância do Lua antigo (`tonumber`, "não numérico vale 0", a
//! verdade do Lua) — cada um com a sua cópia local. Estas funções são essa
//! semântica escrita uma vez; quando duas variantes coexistiam (número
//! estrito vs `tonumber` com strings), ambas têm nome próprio em vez de um
//! parâmetro a escolher o comportamento às escondidas.

use mlua::{Table, Value};

/// Argumento numérico do Lua: `nil`/não numérico vira NaN e é ignorado
/// pelas funções puras (cofre/XP/combate).
pub(crate) fn lua_number(value: Value) -> f64 {
    match value {
        Value::Integer(n) => n as f64,
        Value::Number(n) => n,
        _ => f64::NAN,
    }
}

/// O `tonumber` do Lua: além de números, strings numéricas contam (as
/// quests chegam com contagens em texto). O resto vira NaN.
pub(crate) fn lua_tonumber(value: Value) -> f64 {
    match value {
        Value::String(text) => text
            .to_str()
            .ok()
            .and_then(|text| text.trim().parse::<f64>().ok())
            .unwrap_or(f64::NAN),
        other => lua_number(other),
    }
}

/// Contagem de um valor Lua; não numérico ou não finito vale 0.
pub(crate) fn lua_count(value: Value) -> i64 {
    match value {
        Value::Integer(n) => n,
        Value::Number(n) if n.is_finite() => n as i64,
        _ => 0,
    }
}

/// Verdade do Lua: só `nil` e `false` são falsos (0 e a string vazia
/// contam como verdadeiros, como no Lua).
pub(crate) fn truthy(value: Value) -> bool {
    !matches!(value, Value::Nil | Value::Boolean(false))
}

/// Escreve no `viber_game` inteiro quando o valor é inteiro, `float` quando
/// é fracionário; NaN nunca entra no jogo.
pub(crate) fn write_number(game: &Table, key: &str, value: f64) -> mlua::Result<()> {
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

/// Gate opt-in das env vars (o mesmo conjunto de valores do
/// `VAEL_RULES_RUST`): só `1`/`true`/`on`/`yes` ligam.
pub(crate) fn env_enabled(raw: Option<&str>) -> bool {
    matches!(
        raw.map(|value| value.trim().to_ascii_lowercase())
            .as_deref(),
        Some("1" | "true" | "on" | "yes")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Value::String` só nasce de um `Lua` — uma instância barata por teste.
    fn string(lua: &mlua::Lua, text: &str) -> Value {
        Value::String(lua.create_string(text).expect("string Lua"))
    }

    #[test]
    fn lua_numero_estrito_rejeita_o_resto() {
        let lua = mlua::Lua::new();
        assert_eq!(lua_number(Value::Integer(7)), 7.0);
        assert_eq!(lua_number(Value::Number(2.5)), 2.5);
        assert!(lua_number(Value::Nil).is_nan());
        assert!(lua_number(string(&lua, "7")).is_nan());
        assert!(lua_number(Value::Boolean(true)).is_nan());
    }

    #[test]
    fn lua_tonumber_aceita_strings_numericas() {
        let lua = mlua::Lua::new();
        assert_eq!(lua_tonumber(Value::Integer(7)), 7.0);
        assert_eq!(lua_tonumber(string(&lua, " 3 ")), 3.0);
        assert!(lua_tonumber(string(&lua, "abc")).is_nan());
        assert!(lua_tonumber(Value::Nil).is_nan());
    }

    #[test]
    fn contagem_nao_numerica_vale_zero() {
        let lua = mlua::Lua::new();
        assert_eq!(lua_count(Value::Integer(9)), 9);
        assert_eq!(lua_count(Value::Number(2.7)), 2);
        assert_eq!(lua_count(Value::Number(f64::INFINITY)), 0);
        assert_eq!(lua_count(string(&lua, "5")), 0);
        assert_eq!(lua_count(Value::Nil), 0);
    }

    #[test]
    fn verdade_do_lua() {
        let lua = mlua::Lua::new();
        assert!(truthy(Value::Integer(0)));
        assert!(truthy(string(&lua, "")));
        assert!(!truthy(Value::Nil));
        assert!(!truthy(Value::Boolean(false)));
        assert!(truthy(Value::Boolean(true)));
    }

    #[test]
    fn write_number_escreve_inteiro_ou_float_e_engole_nan() {
        let lua = mlua::Lua::new();
        let game = lua.create_table().unwrap();
        write_number(&game, "a", 3.0).unwrap();
        write_number(&game, "b", 2.5).unwrap();
        write_number(&game, "c", f64::NAN).unwrap();
        assert_eq!(game.raw_get::<i64>("a").unwrap(), 3);
        assert_eq!(game.raw_get::<f64>("b").unwrap(), 2.5);
        // NaN nunca entra: a chave fica simplesmente ausente.
        assert_eq!(game.raw_get::<Value>("c").unwrap(), Value::Nil);
    }

    #[test]
    fn gate_opt_in_so_liga_com_um_verdadeiro() {
        for on in ["1", "true", "ON", " yes "] {
            assert!(env_enabled(Some(on)), "{on} devia ligar");
        }
        for off in [
            None,
            Some(""),
            Some("0"),
            Some("false"),
            Some("off"),
            Some("no"),
        ] {
            assert!(!env_enabled(off), "{off:?} devia ficar desligado");
        }
    }
}

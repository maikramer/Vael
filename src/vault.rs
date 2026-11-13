//! Cofre/inventário em Rust: a aritmética e os tetos de `data/items.json`.
//!
//! As funções puras trabalham sobre um `BTreeMap<String, i64>` (chave
//! `item.<kind>`, como o `viber.game()`) para poderem ser testadas sem a VM.
//! A bridge [`install`] instala `viber.vael` no arranque e lê/escreve a
//! tabela `viber_game` da VM partilhada; o `rules.lua` fica só com os
//! embrulhos finos (espelho na engine antiga e `sync_collect`).
//!
//! Regras portadas de `scripts/lib/rules.lua`:
//! - a contagem guardada nunca é negativa (o antigo `held` do Lua);
//! - recursos param em `resource_cap`, os outros itens no `stack` do próprio
//!   item ou em `stack_max`;
//! - `vault_add` devolve o que somou de facto e `vault_take` só desconta com
//!   saldo suficiente;
//! - o kit de arranque (`starting`) só entra quando o save ainda não tem
//!   `item.gold` nem `item.potion` — um 0 explícito não volta a encher.

use std::collections::BTreeMap;
use std::sync::Arc;

use bevy::prelude::*;
use mlua::{Lua, Table, Value};
use viber::luau::host::LuaScriptHost;

use crate::compat::lua_count;
use crate::content::{ItemsFile, VaelContent};

/// Prefixo das chaves do cofre no `viber.game()`.
pub const ITEM_KEY_PREFIX: &str = "item.";

/// Valor por omissão de `stack_max` quando o JSON não traz o campo (o
/// default do Lua).
pub const DEFAULT_STACK_MAX: i64 = 99;
/// Valor por omissão de `resource_cap` quando o JSON não traz o campo (o
/// default do Lua).
pub const DEFAULT_RESOURCE_CAP: i64 = 1_000_000;

/// Definição mínima de um item para o cofre: o tipo e o teto da pilha.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultItem {
    pub kind: String,
    pub stack: Option<i64>,
}

/// Regras do cofre derivadas de `data/items.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultRules {
    pub stack_max: i64,
    pub resource_cap: i64,
    pub starting: BTreeMap<String, i64>,
    /// Chave = id do item.
    pub items: BTreeMap<String, VaultItem>,
}

impl VaultRules {
    pub fn from_items(file: &ItemsFile) -> Self {
        let stack_max = if file.stack_max > 0 {
            file.stack_max
        } else {
            DEFAULT_STACK_MAX
        };
        let resource_cap = if file.resource_cap > 0 {
            file.resource_cap
        } else {
            DEFAULT_RESOURCE_CAP
        };
        let mut items = BTreeMap::new();
        for def in &file.items {
            if def.id.is_empty() {
                continue;
            }
            items.insert(
                def.id.clone(),
                VaultItem {
                    kind: def.kind.clone(),
                    stack: def.stack,
                },
            );
        }
        Self {
            stack_max,
            resource_cap,
            starting: file.starting.clone(),
            items,
        }
    }

    /// `true` quando o item é um recurso (`kind == "resource"`).
    pub fn is_resource(&self, kind: &str) -> bool {
        self.items
            .get(kind)
            .is_some_and(|item| item.kind == "resource")
    }

    /// Teto de uma pilha: recurso → `resource_cap`; item com `stack` → esse
    /// valor; caso contrário → `stack_max`. Nunca negativo.
    pub fn cap_for(&self, kind: &str) -> i64 {
        if self.is_resource(kind) {
            return self.resource_cap.max(0);
        }
        match self.items.get(kind).and_then(|item| item.stack) {
            Some(stack) => stack.max(0),
            None => self.stack_max.max(0),
        }
    }
}

/// Chave do cofre no `viber.game()`.
pub fn item_key(kind: &str) -> String {
    format!("{ITEM_KEY_PREFIX}{kind}")
}

/// Quantidade guardada de `kind`, nunca negativa.
pub fn vault_get(_rules: &VaultRules, game_kv: &BTreeMap<String, i64>, kind: &str) -> i64 {
    if kind.is_empty() {
        return 0;
    }
    game_kv.get(&item_key(kind)).copied().unwrap_or(0).max(0)
}

/// Soma `amount` e devolve o que foi realmente adicionado.
///
/// `amount` não finito, ≤ 0 ou que não chegue a uma unidade é ignorado (o
/// Lua antigo validava com `finite`); o teto vem de [`VaultRules::cap_for`].
pub fn vault_add(
    rules: &VaultRules,
    game_kv: &mut BTreeMap<String, i64>,
    kind: &str,
    amount: f64,
) -> i64 {
    if kind.is_empty() {
        return 0;
    }
    let Some(amount) = amount_units(amount) else {
        return 0;
    };
    let key = item_key(kind);
    let have = game_kv.get(&key).copied().unwrap_or(0).max(0);
    let cap = rules.cap_for(kind);
    let next = cap.min(have.saturating_add(amount));
    let added = next - have;
    if added <= 0 {
        return 0;
    }
    game_kv.insert(key, next);
    added
}

/// Desconta `amount`; `false` quando o saldo não chega.
pub fn vault_take(
    _rules: &VaultRules,
    game_kv: &mut BTreeMap<String, i64>,
    kind: &str,
    amount: f64,
) -> bool {
    if kind.is_empty() {
        return false;
    }
    let Some(amount) = amount_units(amount) else {
        return false;
    };
    let key = item_key(kind);
    let have = game_kv.get(&key).copied().unwrap_or(0).max(0);
    if have < amount {
        return false;
    }
    game_kv.insert(key, have - amount);
    true
}

/// Semeia o kit de arranque só quando `item.gold` e `item.potion` estão
/// ambos ausentes. Devolve quantas chaves escreveu.
pub fn seed_starting(rules: &VaultRules, game_kv: &mut BTreeMap<String, i64>) -> usize {
    if game_kv.contains_key(&item_key("gold")) || game_kv.contains_key(&item_key("potion")) {
        return 0;
    }
    let mut seeded = 0;
    for (id, count) in &rules.starting {
        if id.is_empty() || *count <= 0 {
            continue;
        }
        game_kv.insert(item_key(id), *count);
        seeded += 1;
    }
    seeded
}

/// Converte o `amount` do Lua em unidades inteiras; `None` = ignorar.
///
/// Positivo com fração trunca (o cofre guarda contagens inteiras) e valores
/// enormes saturam em `i64::MAX` em vez de transbordar.
fn amount_units(amount: f64) -> Option<i64> {
    if !amount.is_finite() || amount <= 0.0 {
        return None;
    }
    let units = amount.trunc();
    if units < 1.0 {
        return None;
    }
    Some(units as i64)
}

// ── bridge Luau ─────────────────────────────────────────────────────────

/// Plugin do cofre: instala `viber.vael` antes de os scripts correrem.
pub struct VaelVaultPlugin;

impl Plugin for VaelVaultPlugin {
    fn build(&self, app: &mut App) {
        // `Startup` corre depois do `PreStartup` que carrega o `VaelContent`.
        app.add_systems(Startup, install_vault_api);
    }
}

/// Arranque: instala `viber.vael` na VM partilhada.
///
/// Público dentro do crate para o plugin do XP ordenar depois dele.
pub(crate) fn install_vault_api(
    host: Option<Res<LuaScriptHost>>,
    content: Option<Res<VaelContent>>,
) {
    let Some(host) = host else {
        warn!("vael: cofre sem host Luau — `viber.vael` não instalado");
        return;
    };
    let Some(content) = content else {
        warn!("vael: cofre sem conteúdo carregado — `viber.vael` não instalado");
        return;
    };
    let rules = VaultRules::from_items(&content.items);
    match install(&host.lua, rules) {
        Ok(()) => info!(
            itens = content.items.items.len(),
            "vael: cofre em Rust ligado (`viber.vael`)"
        ),
        Err(err) => error!("vael: falha a instalar `viber.vael`: {err}"),
    }
}

/// Obtém a tabela `viber.vael` da VM partilhada, criando-a (dentro de
/// `viber`) se ainda não existir. O cofre instala-a primeiro no arranque;
/// os módulos seguintes estendem a mesma tabela — nunca substituem as
/// funções uns dos outros.
pub(crate) fn vael_table(lua: &Lua) -> mlua::Result<Table> {
    let viber: Table = lua.globals().get("viber")?;
    match viber.get::<Value>("vael")? {
        Value::Table(table) => Ok(table),
        _ => {
            let table = lua.create_table()?;
            viber.set("vael", table.clone())?;
            Ok(table)
        }
    }
}

/// Regista uma função na tabela `viber.vael`.
///
/// As capturas (`[nome = expr, …]`) são avaliadas uma vez por função — a
/// closure tem de ser `'static` — e ficam visíveis para o corpo com o nome
/// dado. Sem capturas, dispensa-se a lista:
///
/// ```ignore
/// vael_fn!(lua, vael, "vault_get", [rules = Arc::clone(&rules)],
///     move |lua, kind: Option<String>| Ok(vault_get(&rules, &kv, &kind)));
/// vael_fn!(lua, vael, "heal_player", move |lua, amount: Option<Value>| { … });
/// ```
macro_rules! vael_fn {
    ($lua:expr, $vael:expr, $name:literal, [$($cap:ident = $cap_expr:expr),* $(,)?], $func:expr) => {{
        $(let $cap = $cap_expr;)*
        $vael.set($name, $lua.create_function($func)?)?;
    }};
    ($lua:expr, $vael:expr, $name:literal, $func:expr) => {{
        $vael.set($name, $lua.create_function($func)?)?;
    }};
}
pub(crate) use vael_fn;

/// Instala as quatro funções do cofre na tabela `viber.vael`.
pub(crate) fn install(lua: &Lua, rules: VaultRules) -> mlua::Result<()> {
    let rules = Arc::new(rules);
    let vael = vael_table(lua)?;

    vael_fn!(
        lua,
        vael,
        "vault_get",
        [rules = Arc::clone(&rules)],
        move |lua, kind: Option<String>| {
            let kind = kind.unwrap_or_default();
            if kind.is_empty() {
                return Ok(0i64);
            }
            let game: Table = lua.named_registry_value("viber_game")?;
            let mut kv = BTreeMap::new();
            if let Some(count) = read_item(&game, &kind)? {
                kv.insert(item_key(&kind), count);
            }
            Ok(vault_get(&rules, &kv, &kind))
        }
    );

    vael_fn!(
        lua,
        vael,
        "vault_add",
        [rules = Arc::clone(&rules)],
        move |lua, (kind, amount): (Option<String>, Value)| {
            let kind = kind.unwrap_or_default();
            if kind.is_empty() {
                return Ok(0i64);
            }
            let game: Table = lua.named_registry_value("viber_game")?;
            let mut kv = BTreeMap::new();
            if let Some(count) = read_item(&game, &kind)? {
                kv.insert(item_key(&kind), count);
            }
            let added = vault_add(&rules, &mut kv, &kind, lua_amount(amount));
            if added > 0
                && let Some(value) = kv.get(&item_key(&kind))
            {
                game.raw_set(item_key(&kind), *value)?;
            }
            Ok(added)
        }
    );

    vael_fn!(
        lua,
        vael,
        "vault_take",
        [rules = Arc::clone(&rules)],
        move |lua, (kind, amount): (Option<String>, Value)| {
            let kind = kind.unwrap_or_default();
            if kind.is_empty() {
                return Ok(false);
            }
            let game: Table = lua.named_registry_value("viber_game")?;
            let mut kv = BTreeMap::new();
            if let Some(count) = read_item(&game, &kind)? {
                kv.insert(item_key(&kind), count);
            }
            let ok = vault_take(&rules, &mut kv, &kind, lua_amount(amount));
            if ok && let Some(value) = kv.get(&item_key(&kind)) {
                game.raw_set(item_key(&kind), *value)?;
            }
            Ok(ok)
        }
    );

    vael_fn!(
        lua,
        vael,
        "seed_starting",
        [rules = Arc::clone(&rules)],
        move |lua, ()| {
            let game: Table = lua.named_registry_value("viber_game")?;
            // Só o gate (`gold`/`potion`) e as chaves do kit: um valor
            // não numérico conta como PRESENTE, como no Lua.
            let mut keys: Vec<&str> = rules.starting.keys().map(String::as_str).collect();
            keys.push("gold");
            keys.push("potion");
            keys.sort_unstable();
            keys.dedup();
            let mut kv = BTreeMap::new();
            for kind in keys {
                if let Some(count) = read_item(&game, kind)? {
                    kv.insert(item_key(kind), count);
                }
            }
            let before = kv.clone();
            let seeded = seed_starting(&rules, &mut kv);
            for (key, value) in &kv {
                if before.get(key) != Some(value) {
                    game.raw_set(key.as_str(), *value)?;
                }
            }
            Ok(seeded as i64)
        }
    );

    Ok(())
}

/// Lê `item.<kind>` do jogo: `None` = chave ausente.
fn read_item(game: &Table, kind: &str) -> mlua::Result<Option<i64>> {
    let value: Value = game.raw_get(item_key(kind))?;
    Ok(match value {
        Value::Nil => None,
        other => Some(lua_count(other)),
    })
}

/// `amount` do Lua: `nil`/`false` valem 1 (`amount or 1`); o resto não
/// numérico vira NaN e é ignorado pelas funções puras.
fn lua_amount(value: Value) -> f64 {
    match value {
        Value::Nil | Value::Boolean(false) => 1.0,
        Value::Integer(n) => n as f64,
        Value::Number(n) => n,
        _ => f64::NAN,
    }
}

/// Mapa `chave → contagem` para os testes da bridge (cofre e loja/forja).
#[cfg(test)]
pub(crate) fn kv(pares: &[(&str, i64)]) -> BTreeMap<String, i64> {
    pares
        .iter()
        .map(|(chave, valor)| (chave.to_string(), *valor))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{ItemDef, ItemsFile};

    fn regras() -> VaultRules {
        let file = ItemsFile {
            stack_max: 5,
            resource_cap: 10,
            starting: BTreeMap::from([("gold".to_string(), 30), ("potion".to_string(), 1)]),
            items: vec![
                ItemDef {
                    id: "gold".into(),
                    kind: "resource".into(),
                    ..Default::default()
                },
                ItemDef {
                    id: "potion".into(),
                    kind: "item".into(),
                    ..Default::default()
                },
                ItemDef {
                    id: "semente".into(),
                    kind: "item".into(),
                    stack: Some(3),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        VaultRules::from_items(&file)
    }

    #[test]
    fn get_clampa_negativo_e_ausente() {
        let rules = regras();
        assert_eq!(vault_get(&rules, &kv(&[("item.gold", -3)]), "gold"), 0);
        assert_eq!(vault_get(&rules, &kv(&[("item.gold", 7)]), "gold"), 7);
        assert_eq!(vault_get(&rules, &BTreeMap::new(), "gold"), 0);
        assert_eq!(vault_get(&rules, &kv(&[("item.gold", 7)]), ""), 0);
    }

    #[test]
    fn add_respeita_o_teto_de_recurso_e_devolve_o_delta() {
        let rules = regras();
        let mut inventario = BTreeMap::new();
        assert_eq!(vault_add(&rules, &mut inventario, "gold", 6.0), 6);
        assert_eq!(inventario["item.gold"], 6);
        // 6 + 6 = 12, mas o teto é 10: só entram 4.
        assert_eq!(vault_add(&rules, &mut inventario, "gold", 6.0), 4);
        assert_eq!(inventario["item.gold"], 10);
        assert_eq!(vault_add(&rules, &mut inventario, "gold", 1.0), 0);
        assert_eq!(inventario["item.gold"], 10);
    }

    #[test]
    fn add_respeita_o_stack_do_item_e_o_global() {
        let rules = regras();
        let mut inventario = BTreeMap::new();
        // `potion` não tem `stack`: cai no `stack_max` (5).
        assert_eq!(vault_add(&rules, &mut inventario, "potion", 9.0), 5);
        // `semente` tem `stack` 3.
        assert_eq!(vault_add(&rules, &mut inventario, "semente", 2.0), 2);
        assert_eq!(vault_add(&rules, &mut inventario, "semente", 2.0), 1);
        assert_eq!(inventario["item.semente"], 3);
    }

    #[test]
    fn add_ignora_valores_invalidos() {
        let rules = regras();
        let mut inventario = BTreeMap::new();
        for amount in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.5] {
            assert_eq!(
                vault_add(&rules, &mut inventario, "potion", amount),
                0,
                "amount inválido devia ser ignorado: {amount}"
            );
        }
        assert!(inventario.is_empty());
        assert_eq!(vault_add(&rules, &mut inventario, "", 1.0), 0);
    }

    #[test]
    fn add_sobre_saldo_negativo_corrige_o_save() {
        let rules = regras();
        let mut inventario = kv(&[("item.gold", -4)]);
        assert_eq!(vault_get(&rules, &inventario, "gold"), 0);
        assert_eq!(vault_add(&rules, &mut inventario, "gold", 2.0), 2);
        assert_eq!(inventario["item.gold"], 2);
    }

    #[test]
    fn take_exige_saldo_e_desconta_exacto() {
        let rules = regras();
        let mut inventario = kv(&[("item.potion", 5)]);
        assert!(vault_take(&rules, &mut inventario, "potion", 3.0));
        assert_eq!(inventario["item.potion"], 2);
        assert!(!vault_take(&rules, &mut inventario, "potion", 3.0));
        assert_eq!(inventario["item.potion"], 2);
        assert!(vault_take(&rules, &mut inventario, "potion", 2.0));
        assert_eq!(inventario["item.potion"], 0);
    }

    #[test]
    fn take_ignora_invalidos_e_saldo_negativo() {
        let rules = regras();
        let mut inventario = kv(&[("item.potion", -2)]);
        for amount in [0.0, -1.0, f64::NAN, f64::INFINITY, 0.5] {
            assert!(
                !vault_take(&rules, &mut inventario, "potion", amount),
                "amount inválido não pode descontar: {amount}"
            );
        }
        assert!(!vault_take(&rules, &mut inventario, "potion", 1.0));
        assert!(!vault_take(&rules, &mut inventario, "", 1.0));
        assert_eq!(inventario["item.potion"], -2);
    }

    #[test]
    fn seed_so_quando_gold_e_potion_ausentes() {
        let rules = regras();
        let mut vazio = BTreeMap::new();
        assert_eq!(seed_starting(&rules, &mut vazio), 2);
        assert_eq!(vazio["item.gold"], 30);
        assert_eq!(vazio["item.potion"], 1);

        // `item.gold = 0` é uma chave PRESENTE (ouro gasto): não volta a encher.
        let mut gasto = kv(&[("item.gold", 0)]);
        assert_eq!(seed_starting(&rules, &mut gasto), 0);
        assert!(!gasto.contains_key("item.potion"));

        let mut com_pocao = kv(&[("item.potion", 1)]);
        assert_eq!(seed_starting(&rules, &mut com_pocao), 0);
        assert!(!com_pocao.contains_key("item.gold"));
    }

    #[test]
    fn seed_ignora_contagens_nao_positivas() {
        let mut file = ItemsFile {
            starting: BTreeMap::from([
                ("gold".to_string(), 0),
                ("potion".to_string(), -1),
                ("corda".to_string(), 2),
            ]),
            ..Default::default()
        };
        file.items.clear();
        let rules = VaultRules::from_items(&file);
        let mut vazio = BTreeMap::new();
        assert_eq!(seed_starting(&rules, &mut vazio), 1);
        assert_eq!(vazio["item.corda"], 2);
        assert!(!vazio.contains_key("item.gold"));
        assert!(!vazio.contains_key("item.potion"));
    }

    fn lua_com_bridge(rules: VaultRules) -> (Lua, Table) {
        let lua = Lua::new();
        lua.globals()
            .set("viber", lua.create_table().expect("tabela viber"))
            .expect("global viber");
        let game = lua.create_table().expect("tabela do jogo");
        lua.set_named_registry_value("viber_game", game.clone())
            .expect("registar viber_game");
        install(&lua, rules).expect("instalar viber.vael");
        (lua, game)
    }

    #[test]
    fn bridge_le_e_escreve_no_viber_game() {
        let (lua, game) = lua_com_bridge(regras());
        let added: i64 = lua
            .load("return viber.vael.vault_add('potion', 2)")
            .eval()
            .expect("vault_add");
        assert_eq!(added, 2);
        assert_eq!(game.raw_get::<i64>("item.potion").expect("potion"), 2);

        // Sem `amount` o Lua antigo usava 1.
        let added: i64 = lua
            .load("return viber.vael.vault_add('potion')")
            .eval()
            .expect("vault_add sem amount");
        assert_eq!(added, 1);

        let got: i64 = lua
            .load("return viber.vael.vault_get('potion')")
            .eval()
            .expect("vault_get");
        assert_eq!(got, 3);

        let ok: bool = lua
            .load("return viber.vael.vault_take('potion', 5)")
            .eval()
            .expect("vault_take sem saldo");
        assert!(!ok);
        let ok: bool = lua
            .load("return viber.vael.vault_take('potion', 1)")
            .eval()
            .expect("vault_take");
        assert!(ok);
        assert_eq!(game.raw_get::<i64>("item.potion").expect("potion"), 2);
    }

    #[test]
    fn bridge_semeia_o_kit_de_arranque_uma_vez() {
        let (lua, game) = lua_com_bridge(regras());
        lua.load("viber.vael.seed_starting()").exec().expect("seed");
        assert_eq!(game.raw_get::<i64>("item.gold").expect("gold"), 30);
        assert_eq!(game.raw_get::<i64>("item.potion").expect("potion"), 1);

        // Ouro gasto (0 explícito) não volta a encher.
        game.raw_set("item.gold", 0).expect("zerar gold");
        lua.load("viber.vael.seed_starting()")
            .exec()
            .expect("segundo seed");
        assert_eq!(game.raw_get::<i64>("item.gold").expect("gold"), 0);
    }

    #[test]
    fn plugin_sem_host_nao_panica() {
        let mut app = App::new();
        app.add_plugins(bevy::MinimalPlugins)
            .add_plugins(VaelVaultPlugin);
        for _ in 0..2 {
            app.update();
        }
    }
}

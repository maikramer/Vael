//! Loja e forja em Rust: catálogo, preços, tetos e a negociação.
//!
//! As funções puras trabalham sobre o `BTreeMap<String, i64>` do cofre
//! (chave `item.<id>`) e reutilizam [`crate::vault`] para somar/descontar, por
//! isso podem ser testadas sem a VM. A bridge [`install`] instala
//! `viber.vael.shop_*`/`craft_*` no arranque e lê/escreve a tabela
//! `viber_game`; o `game/shop.lua` e o `lib/craft.lua` ficam com a UI, o som,
//! os toasts e os efeitos de quest (`report_craft`/`add_xp`).
//!
//! Regras portadas de `scripts/game/shop.lua` e `scripts/lib/craft.lua`:
//! - só entram no catálogo itens com `shop` e um preço válido; um preço
//!   negativo é ignorado e 0 só é válido na compra (`zero_ok`);
//! - o teto de pilha é o `cap_of` do Lua: `stack` (> 0) ou `stack_max`. O
//!   `resource_cap` só corta no `vault_add`; é essa diferença que os
//!   rollbacks de compra/forja recuperam;
//! - comprar: o ouro sai e o item entra; sem espaço na mochila o ouro volta;
//! - vender: o item sai e o ouro entra; se o ouro não couber todo, o parcial
//!   é retirado e o item regressa ao cofre;
//! - forjar: os custos saem e o produto entra; uma falha (falta/cheio)
//!   devolve tudo o que foi tirado.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use bevy::prelude::*;
use mlua::{Lua, Table, Value};
use viber::luau::host::LuaScriptHost;

use crate::content::{ItemsFile, RecipesFile, VaelContent};
use crate::vault::{VaultRules, item_key, vault_add, vault_get, vault_take};

// ── loja ────────────────────────────────────────────────────────────────

/// Uma linha do catálogo do mercador (`data/items.json` → `shop`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShopEntry {
    /// Texto da banca: `shop.label`, senão o `label` do item, senão o id.
    pub label: String,
    /// Preço de compra; `None` = não se vende ao herói.
    pub buy: Option<i64>,
    /// Preço de venda; `None` = não se compra ao herói.
    pub sell: Option<i64>,
}

/// Catálogo da loja derivado de `data/items.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShopRules {
    /// Tetos e aritmética do cofre (o mesmo `VaultRules` do `src/vault.rs`).
    pub vault: VaultRules,
    /// Chave = id do item.
    pub catalog: BTreeMap<String, ShopEntry>,
}

impl ShopRules {
    pub fn from_items(file: &ItemsFile) -> Self {
        let vault = VaultRules::from_items(file);
        let mut catalog = BTreeMap::new();
        for def in &file.items {
            if def.id.is_empty() {
                continue;
            }
            let Some(shop) = def.shop.as_ref() else {
                continue;
            };
            let buy = price_of(shop.buy, true);
            let sell = price_of(shop.sell, false);
            if buy.is_none() && sell.is_none() {
                continue;
            }
            let label = if !shop.label.is_empty() {
                shop.label.clone()
            } else if !def.label.is_empty() {
                def.label.clone()
            } else {
                def.id.clone()
            };
            catalog.insert(def.id.clone(), ShopEntry { label, buy, sell });
        }
        Self { vault, catalog }
    }
}

/// Preço do item para a acção (`shop_price` da bridge); `None` se não houver.
pub fn shop_price(rules: &ShopRules, id: &str, action: TradeAction) -> Option<i64> {
    let entry = rules.catalog.get(id)?;
    match action {
        TradeAction::Buy => entry.buy,
        TradeAction::Sell => entry.sell,
    }
}

/// Preço válido do `shop`: ≥ 0 e (0 só com `zero_ok`).
///
/// Um valor negativo nunca entra (vender a 0/negativo apagava o item sem
/// pagar); `zero_ok` preserva a linha de oferta a 0 ouro na compra.
pub fn price_of(value: Option<i64>, zero_ok: bool) -> Option<i64> {
    match value {
        Some(value) if value >= 0 && (value > 0 || zero_ok) => Some(value),
        _ => None,
    }
}

/// Teto da pilha como o `cap_of` do Lua: `stack` quando > 0, senão
/// `stack_max`. O `resource_cap` é do `vault_add` (ver o doc do módulo).
pub fn cap_of(vault: &VaultRules, id: &str) -> i64 {
    match vault.items.get(id).and_then(|item| item.stack) {
        Some(stack) if stack > 0 => stack,
        _ => vault.stack_max.max(0),
    }
}

/// Acção da negociação.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeAction {
    Buy,
    Sell,
}

impl TradeAction {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "buy" => Some(Self::Buy),
            "sell" => Some(Self::Sell),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Buy => "buy",
            Self::Sell => "sell",
        }
    }
}

/// Porque uma negociação falhou.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeReason {
    /// O id não está no catálogo.
    UnknownItem,
    /// A acção não é `buy`/`sell`.
    UnknownAction,
    /// O item não tem preço para esta acção.
    NoPrice,
    /// O ouro não chega.
    NoGold,
    /// A pilha já está no teto.
    Full,
    /// O item não coube na mochila (o ouro volta).
    NoSpace,
    /// Não há stock para vender.
    NoStock,
    /// O ouro não coube na bolsa (o item volta).
    GoldFull,
}

impl TradeReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnknownItem => "unknown_item",
            Self::UnknownAction => "unknown_action",
            Self::NoPrice => "no_price",
            Self::NoGold => "no_gold",
            Self::Full => "full",
            Self::NoSpace => "no_space",
            Self::NoStock => "no_stock",
            Self::GoldFull => "gold_full",
        }
    }
}

/// Resultado de uma negociação (o que o Lua usa para o toast).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TradeOutcome {
    pub ok: bool,
    /// `None` no sucesso.
    pub reason: Option<TradeReason>,
    /// Preço da acção, quando existe.
    pub price: Option<i64>,
    /// Label da banca, quando o item é conhecido.
    pub label: Option<String>,
    /// Teto da pilha, quando a compra o consultou.
    pub cap: Option<i64>,
}

fn trade_fail(
    reason: TradeReason,
    price: Option<i64>,
    label: Option<String>,
    cap: Option<i64>,
) -> TradeOutcome {
    TradeOutcome {
        ok: false,
        reason: Some(reason),
        price,
        label,
        cap,
    }
}

/// Estado visual de uma linha da loja (a classe CSS do `status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShopStatus {
    Buy,
    Sell,
    Locked,
    Unaffordable,
}

impl ShopStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Buy => "buy",
            Self::Sell => "sell",
            Self::Locked => "locked",
            Self::Unaffordable => "unaffordable",
        }
    }
}

/// Uma linha da lista `shop`, em dados planos (o Lua formata a UI).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShopRow {
    pub id: String,
    /// Label do catálogo (sem o " ×N" da venda: isso é formatação).
    pub label: String,
    /// `true` quando a linha é de venda (preço com prefixo "+" na UI).
    pub selling: bool,
    /// Preço efectivo: `sell` na venda, `buy` na compra.
    pub price: i64,
    pub stock: i64,
    /// Teto da pilha (`cap_of`), para a linha "cheia".
    pub cap: i64,
    pub status: ShopStatus,
}

/// Monta as linhas da loja na ordem da UI (label, depois id).
pub fn shop_rows(rules: &ShopRules, kv: &BTreeMap<String, i64>) -> Vec<ShopRow> {
    let gold = vault_get(&rules.vault, kv, "gold");
    let mut rows = Vec::with_capacity(rules.catalog.len());
    for (id, entry) in &rules.catalog {
        let stock = vault_get(&rules.vault, kv, id);
        let cap = cap_of(&rules.vault, id);
        let (selling, price, status) = if let Some(sell) = entry.sell {
            // A venda tem prioridade, como no `rows()` do Lua: o label leva o
            // stock e o preço leva "+".
            let status = if stock > 0 {
                ShopStatus::Sell
            } else {
                ShopStatus::Locked
            };
            (true, sell, status)
        } else {
            let Some(buy) = entry.buy else {
                continue;
            };
            let status = if stock >= cap {
                ShopStatus::Locked
            } else if gold >= buy {
                ShopStatus::Buy
            } else {
                ShopStatus::Unaffordable
            };
            (false, buy, status)
        };
        rows.push(ShopRow {
            id: id.clone(),
            label: entry.label.clone(),
            selling,
            price,
            stock,
            cap,
            status,
        });
    }
    // Desempate pelo id só para a ordem ser estável entre execuções.
    rows.sort_by(|a, b| a.label.cmp(&b.label).then_with(|| a.id.cmp(&b.id)));
    rows
}

/// Compra/venda sobre o cofre, com o mesmo fluxo do `trade` do Lua.
pub fn shop_trade(
    rules: &ShopRules,
    kv: &mut BTreeMap<String, i64>,
    action: TradeAction,
    id: &str,
) -> TradeOutcome {
    let Some(entry) = rules.catalog.get(id) else {
        return trade_fail(TradeReason::UnknownItem, None, None, None);
    };
    match action {
        TradeAction::Buy => buy(rules, kv, id, entry),
        TradeAction::Sell => sell(rules, kv, id, entry),
    }
}

fn buy(
    rules: &ShopRules,
    kv: &mut BTreeMap<String, i64>,
    id: &str,
    entry: &ShopEntry,
) -> TradeOutcome {
    let label = entry.label.clone();
    let Some(price) = entry.buy else {
        return trade_fail(TradeReason::NoPrice, None, Some(label), None);
    };
    if vault_get(&rules.vault, kv, "gold") < price {
        return trade_fail(TradeReason::NoGold, Some(price), Some(label), None);
    }
    let cap = cap_of(&rules.vault, id);
    if vault_get(&rules.vault, kv, id) >= cap {
        return trade_fail(TradeReason::Full, Some(price), Some(label), Some(cap));
    }
    // O retorno do `take` é ignorado como no Lua: um preço 0 não desconta.
    let _ = vault_take(&rules.vault, kv, "gold", price as f64);
    if vault_add(&rules.vault, kv, id, 1.0) < 1 {
        // Não coube: o ouro volta (um preço 0 não muda nada).
        let _ = vault_add(&rules.vault, kv, "gold", price as f64);
        return trade_fail(TradeReason::NoSpace, Some(price), Some(label), Some(cap));
    }
    TradeOutcome {
        ok: true,
        reason: None,
        price: Some(price),
        label: Some(label),
        cap: Some(cap),
    }
}

fn sell(
    rules: &ShopRules,
    kv: &mut BTreeMap<String, i64>,
    id: &str,
    entry: &ShopEntry,
) -> TradeOutcome {
    let label = entry.label.clone();
    let Some(price) = entry.sell else {
        return trade_fail(TradeReason::NoPrice, None, Some(label), None);
    };
    // A linha pode ter stock de um frame anterior: o `take` tem de confirmar.
    if vault_get(&rules.vault, kv, id) == 0 || !vault_take(&rules.vault, kv, id, 1.0) {
        return trade_fail(TradeReason::NoStock, Some(price), Some(label), None);
    }
    let paid = vault_add(&rules.vault, kv, "gold", price as f64);
    if paid < price {
        // O ouro tem teto global: o parcial volta a sair e o item regressa.
        if paid > 0 {
            let _ = vault_take(&rules.vault, kv, "gold", paid as f64);
        }
        let _ = vault_add(&rules.vault, kv, id, 1.0);
        return trade_fail(TradeReason::GoldFull, Some(price), Some(label), None);
    }
    TradeOutcome {
        ok: true,
        reason: None,
        price: Some(price),
        label: Some(label),
        cap: None,
    }
}

// ── forja ───────────────────────────────────────────────────────────────

/// Uma receita de `data/recipes.json` normalizada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CraftRecipe {
    pub id: String,
    pub out: String,
    /// Quantidade autoral (o uso clampa em ≥ 1).
    pub count: i64,
    /// Custos positivos, ordenados por item (texto estável na lista).
    pub cost: Vec<(String, i64)>,
}

impl CraftRecipe {
    /// Quantidade realmente produzida (`math.max(1, floor(count))` do Lua).
    pub fn output_count(&self) -> i64 {
        self.count.max(1)
    }
}

/// Receitas da forja + os tetos do cofre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CraftRules {
    pub vault: VaultRules,
    pub recipes: Vec<CraftRecipe>,
}

impl CraftRules {
    /// Receitas de `data/recipes.json` com os tetos de `data/items.json`.
    ///
    /// Como no Lua, uma receita sem id ou repetida fica de fora (a primeira
    /// ganha) e custos não positivos são ignorados.
    pub fn from_content(items: &ItemsFile, file: &RecipesFile) -> Self {
        let vault = VaultRules::from_items(items);
        let mut recipes = Vec::new();
        let mut seen = BTreeSet::new();
        for def in &file.recipes {
            if def.id.is_empty() || !seen.insert(def.id.clone()) {
                continue;
            }
            let mut cost: Vec<(String, i64)> = def
                .cost
                .iter()
                .filter(|(_, n)| **n > 0)
                .map(|(item, n)| (item.clone(), *n))
                .collect();
            cost.sort_by(|a, b| a.0.cmp(&b.0));
            recipes.push(CraftRecipe {
                id: def.id.clone(),
                out: def.out.clone(),
                count: def.count,
                cost,
            });
        }
        Self { vault, recipes }
    }

    pub fn get(&self, id: &str) -> Option<&CraftRecipe> {
        if id.is_empty() {
            return None;
        }
        self.recipes.iter().find(|recipe| recipe.id == id)
    }
}

/// Receitas na ordem de `data/recipes.json`, já normalizadas
/// (`craft_recipes` da bridge).
pub fn craft_recipes(rules: &CraftRules) -> &[CraftRecipe] {
    &rules.recipes
}

/// Estado de uma receita: pronta, falta material ou mochila cheia.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CraftStatus {
    Ok,
    Missing,
    Full,
}

impl CraftStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Missing => "missing",
            Self::Full => "full",
        }
    }
}

/// Porque uma forja falhou.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CraftReason {
    /// Id desconhecido.
    Unknown,
    /// Falta material.
    Missing,
    /// Não cabe na mochila.
    Full,
}

impl CraftReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Missing => "missing",
            Self::Full => "full",
        }
    }
}

/// Resultado de uma forja.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CraftOutcome {
    pub ok: bool,
    /// Item produzido (vazio com id desconhecido).
    pub output: String,
    /// Quantidade produzida (0 com id desconhecido).
    pub count: i64,
    /// `None` no sucesso.
    pub reason: Option<CraftReason>,
}

/// `"ok"`/`"missing"`/`"full"`; `None` quando a receita não existe.
pub fn craft_status(
    rules: &CraftRules,
    kv: &BTreeMap<String, i64>,
    id: &str,
) -> Option<CraftStatus> {
    let recipe = rules.get(id)?;
    Some(recipe_status(rules, kv, recipe))
}

fn recipe_status(
    rules: &CraftRules,
    kv: &BTreeMap<String, i64>,
    recipe: &CraftRecipe,
) -> CraftStatus {
    for (item, n) in &recipe.cost {
        if vault_get(&rules.vault, kv, item) < *n {
            return CraftStatus::Missing;
        }
    }
    let count = recipe.output_count();
    if vault_get(&rules.vault, kv, &recipe.out).saturating_add(count)
        > cap_of(&rules.vault, &recipe.out)
    {
        return CraftStatus::Full;
    }
    CraftStatus::Ok
}

/// Executa a receita: tira os custos, acrescenta o produto e reverte falhas.
pub fn craft_make(rules: &CraftRules, kv: &mut BTreeMap<String, i64>, id: &str) -> CraftOutcome {
    let Some(recipe) = rules.get(id) else {
        return CraftOutcome {
            ok: false,
            output: String::new(),
            count: 0,
            reason: Some(CraftReason::Unknown),
        };
    };
    let output = recipe.out.clone();
    let count = recipe.output_count();
    match recipe_status(rules, kv, recipe) {
        CraftStatus::Ok => {}
        CraftStatus::Missing => {
            return CraftOutcome {
                ok: false,
                output,
                count,
                reason: Some(CraftReason::Missing),
            };
        }
        CraftStatus::Full => {
            return CraftOutcome {
                ok: false,
                output,
                count,
                reason: Some(CraftReason::Full),
            };
        }
    }

    let mut taken: Vec<(&str, i64)> = Vec::with_capacity(recipe.cost.len());
    for (item, n) in &recipe.cost {
        if !vault_take(&rules.vault, kv, item, *n as f64) {
            // Não devia acontecer (o status acabou de confirmar): devolve tudo.
            refund(rules, kv, &taken);
            return CraftOutcome {
                ok: false,
                output,
                count,
                reason: Some(CraftReason::Missing),
            };
        }
        taken.push((item.as_str(), *n));
    }
    let got = vault_add(&rules.vault, kv, &output, count as f64);
    if got < count {
        if got > 0 {
            let _ = vault_take(&rules.vault, kv, &output, got as f64);
        }
        refund(rules, kv, &taken);
        return CraftOutcome {
            ok: false,
            output,
            count,
            reason: Some(CraftReason::Full),
        };
    }
    CraftOutcome {
        ok: true,
        output,
        count,
        reason: None,
    }
}

fn refund(rules: &CraftRules, kv: &mut BTreeMap<String, i64>, taken: &[(&str, i64)]) {
    for (item, n) in taken {
        let _ = vault_add(&rules.vault, kv, item, *n as f64);
    }
}

// ── bridge Luau ─────────────────────────────────────────────────────────

/// Plugin da economia: instala `viber.vael.shop_*`/`craft_*` depois do
/// relógio.
pub struct VaelEconomyPlugin;

impl Plugin for VaelEconomyPlugin {
    fn build(&self, app: &mut App) {
        // `Startup` corre depois do `PreStartup` que carrega o `VaelContent`;
        // `.after` garante a ordem dos instaladores da mesma tabela.
        app.add_systems(
            Startup,
            install_economy_api.after(crate::clock::install_clock_api),
        );
    }
}

/// Arranque: acrescenta as funções da loja/forja à tabela `viber.vael`.
///
/// Público dentro do crate para a viagem/marcos ordenar depois dele.
pub(crate) fn install_economy_api(
    host: Option<Res<LuaScriptHost>>,
    content: Option<Res<VaelContent>>,
) {
    let Some(host) = host else {
        warn!("vael: economia sem host Luau — `viber.vael` não estendido");
        return;
    };
    let Some(content) = content else {
        warn!("vael: economia sem conteúdo carregado — `viber.vael` não estendido");
        return;
    };
    let shop = ShopRules::from_items(&content.items);
    let craft = CraftRules::from_content(&content.items, &content.recipes);
    let receitas = craft.recipes.len();
    match install(&host.lua, shop, craft) {
        Ok(()) => info!(
            itens = content.items.items.len(),
            receitas, "vael: loja/forja em Rust ligadas (`viber.vael`)"
        ),
        Err(err) => error!("vael: falha a instalar loja/forja em `viber.vael`: {err}"),
    }
}

/// Acrescenta as funções da loja/forja a `viber.vael` sem substituir as que
/// já lá estão (cofre, XP, equipamento, quests, combate, relógio).
pub(crate) fn install(lua: &Lua, shop: ShopRules, craft: CraftRules) -> mlua::Result<()> {
    let shop = Arc::new(shop);
    let craft = Arc::new(craft);
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
        let shop = Arc::clone(&shop);
        vael.set(
            "shop_price",
            lua.create_function(move |_, (id, action): (Option<String>, Option<String>)| {
                let id = id.unwrap_or_default();
                let Some(action) = TradeAction::parse(&action.unwrap_or_default()) else {
                    return Ok(None);
                };
                Ok(shop_price(&shop, &id, action))
            })?,
        )?;
    }

    {
        let shop = Arc::clone(&shop);
        vael.set(
            "shop_rows",
            lua.create_function(move |lua, ()| {
                let game: Table = lua.named_registry_value("viber_game")?;
                let mut kinds: Vec<&str> = Vec::with_capacity(shop.catalog.len() + 1);
                kinds.push("gold");
                kinds.extend(shop.catalog.keys().map(String::as_str));
                let kv = read_items(&game, &kinds)?;
                let rows = shop_rows(&shop, &kv);
                let table = lua.create_table()?;
                for (index, row) in rows.iter().enumerate() {
                    let item = lua.create_table()?;
                    item.set("id", row.id.as_str())?;
                    item.set("label", row.label.as_str())?;
                    item.set("selling", row.selling)?;
                    item.set("price", row.price)?;
                    item.set("stock", row.stock)?;
                    item.set("cap", row.cap)?;
                    item.set("status", row.status.as_str())?;
                    table.set(index + 1, item)?;
                }
                Ok(table)
            })?,
        )?;
    }

    {
        let shop = Arc::clone(&shop);
        vael.set(
            "shop_trade",
            lua.create_function(move |lua, (action, id): (Option<String>, Option<String>)| {
                let id = id.unwrap_or_default();
                let Some(action) = TradeAction::parse(&action.unwrap_or_default()) else {
                    return trade_table(
                        lua,
                        &trade_fail(TradeReason::UnknownAction, None, None, None),
                    );
                };
                let game: Table = lua.named_registry_value("viber_game")?;
                let mut kv = read_items(&game, &[id.as_str(), "gold"])?;
                let before = kv.clone();
                let out = shop_trade(&shop, &mut kv, action, &id);
                write_items(&game, &before, &kv)?;
                trade_table(lua, &out)
            })?,
        )?;
    }

    {
        let craft = Arc::clone(&craft);
        vael.set(
            "craft_recipes",
            lua.create_function(move |lua, ()| {
                let table = lua.create_table()?;
                for (index, recipe) in craft_recipes(&craft).iter().enumerate() {
                    let row = lua.create_table()?;
                    row.set("id", recipe.id.as_str())?;
                    row.set("out", recipe.out.as_str())?;
                    row.set("count", recipe.count)?;
                    let cost = lua.create_table()?;
                    for (item, n) in &recipe.cost {
                        cost.set(item.as_str(), *n)?;
                    }
                    row.set("cost", cost)?;
                    table.set(index + 1, row)?;
                }
                Ok(table)
            })?,
        )?;
    }

    {
        let craft = Arc::clone(&craft);
        vael.set(
            "craft_status",
            lua.create_function(move |lua, id: Option<String>| {
                let id = id.unwrap_or_default();
                let Some(recipe) = craft.get(&id) else {
                    return Ok(Value::Nil);
                };
                let game: Table = lua.named_registry_value("viber_game")?;
                let kv = read_items(&game, &craft_keys(recipe))?;
                Ok(match craft_status(&craft, &kv, &id) {
                    Some(status) => Value::String(lua.create_string(status.as_str())?),
                    None => Value::Nil,
                })
            })?,
        )?;
    }

    {
        let craft = Arc::clone(&craft);
        vael.set(
            "craft_make",
            lua.create_function(move |lua, id: Option<String>| {
                let id = id.unwrap_or_default();
                let game: Table = lua.named_registry_value("viber_game")?;
                let mut kv = match craft.get(&id) {
                    Some(recipe) => read_items(&game, &craft_keys(recipe))?,
                    None => BTreeMap::new(),
                };
                let before = kv.clone();
                let out = craft_make(&craft, &mut kv, &id);
                write_items(&game, &before, &kv)?;
                let table = lua.create_table()?;
                table.set("ok", out.ok)?;
                table.set("output", out.output.as_str())?;
                table.set("count", out.count)?;
                table.set("reason", out.reason.map(CraftReason::as_str))?;
                Ok(table)
            })?,
        )?;
    }

    Ok(())
}

fn trade_table(lua: &Lua, out: &TradeOutcome) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("ok", out.ok)?;
    table.set("reason", out.reason.map(TradeReason::as_str))?;
    table.set("price", out.price)?;
    table.set("label", out.label.as_deref())?;
    table.set("cap", out.cap)?;
    Ok(table)
}

/// Chaves do cofre que uma receita lê: custos + produto.
fn craft_keys(recipe: &CraftRecipe) -> Vec<&str> {
    let mut kinds: Vec<&str> = recipe.cost.iter().map(|(item, _)| item.as_str()).collect();
    kinds.push(recipe.out.as_str());
    kinds
}

/// Lê as chaves `item.<kind>` do jogo (ausente = fora do mapa, como o Lua).
fn read_items(game: &Table, kinds: &[&str]) -> mlua::Result<BTreeMap<String, i64>> {
    let mut kv = BTreeMap::new();
    for kind in kinds {
        if kind.is_empty() {
            continue;
        }
        let key = item_key(kind);
        match game.raw_get::<Value>(key.as_str())? {
            Value::Nil => {}
            value => {
                kv.insert(key, lua_count(value));
            }
        }
    }
    Ok(kv)
}

/// Escreve só as chaves que mudaram (uma falha não toca no save).
fn write_items(
    game: &Table,
    before: &BTreeMap<String, i64>,
    after: &BTreeMap<String, i64>,
) -> mlua::Result<()> {
    for (key, value) in after {
        if before.get(key) != Some(value) {
            game.raw_set(key.as_str(), *value)?;
        }
    }
    Ok(())
}

/// Contagem de um valor Lua; não numérico ou não finito vale 0 (como o
/// `lua_count` do cofre).
fn lua_count(value: Value) -> i64 {
    match value {
        Value::Integer(n) => n,
        Value::Number(n) if n.is_finite() => n as i64,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{ItemDef, ItemShop, ItemsFile, RecipeDef, RecipesFile};

    /// Cofre pequeno de propósito: `resource_cap` (3) < `stack_max` (5) faz
    /// os rollbacks de compra/forja serem alcançáveis.
    fn items_file() -> ItemsFile {
        let item = |id: &str, label: &str, kind: &str| ItemDef {
            id: id.into(),
            label: label.into(),
            kind: kind.into(),
            ..Default::default()
        };
        ItemsFile {
            stack_max: 5,
            resource_cap: 3,
            items: vec![
                item("gold", "Ouro", "resource"),
                ItemDef {
                    shop: Some(ItemShop {
                        buy: Some(2),
                        sell: None,
                        label: "Comprar poção".into(),
                    }),
                    ..item("pocao", "Poção", "item")
                },
                ItemDef {
                    shop: Some(ItemShop {
                        buy: None,
                        sell: Some(2),
                        label: "Vender erva".into(),
                    }),
                    ..item("erva", "Erva", "item")
                },
                ItemDef {
                    stack: Some(3),
                    shop: Some(ItemShop {
                        buy: Some(1),
                        sell: Some(1),
                        label: String::new(),
                    }),
                    ..item("semente", "Semente", "item")
                },
                ItemDef {
                    shop: Some(ItemShop {
                        buy: Some(0),
                        sell: None,
                        label: "Levar oferta".into(),
                    }),
                    ..item("oferta", "Oferta", "item")
                },
                ItemDef {
                    shop: Some(ItemShop {
                        buy: Some(1),
                        sell: None,
                        label: "Comprar seiva".into(),
                    }),
                    ..item("seiva", "Seiva", "resource")
                },
                ItemDef {
                    shop: Some(ItemShop {
                        buy: Some(-5),
                        sell: Some(0),
                        label: "Inválida".into(),
                    }),
                    ..item("ruim", "Ruim", "item")
                },
                ItemDef {
                    shop: Some(ItemShop {
                        buy: None,
                        sell: Some(4),
                        label: "Vender relíquia".into(),
                    }),
                    ..item("reliquia", "Relíquia", "item")
                },
                item("sem-loja", "Sem loja", "item"),
            ],
            ..Default::default()
        }
    }

    fn recipes_file() -> RecipesFile {
        let recipe = |id: &str, out: &str, count: i64, cost: &[(&str, i64)]| RecipeDef {
            id: id.into(),
            out: out.into(),
            count,
            cost: cost
                .iter()
                .map(|(item, n)| ((*item).to_string(), *n))
                .collect(),
        };
        RecipesFile {
            recipes: vec![
                recipe("pocao", "pocao", 2, &[("erva", 2), ("gold", 1)]),
                recipe("muita", "pocao", 2, &[("erva", 1)]),
                recipe("sem", "pocao", 1, &[("erva", 5)]),
                recipe("seiva", "seiva", 2, &[("erva", 1)]),
                // Custo zero/negativo é ignorado; count 0 vira 1.
                recipe("oferta", "pocao", 0, &[("erva", 0), ("gold", -1)]),
                recipe("pocao", "erva", 1, &[("erva", 1)]),
                recipe("", "erva", 1, &[]),
            ],
            ..Default::default()
        }
    }

    fn shop() -> ShopRules {
        ShopRules::from_items(&items_file())
    }

    fn craft() -> CraftRules {
        CraftRules::from_content(&items_file(), &recipes_file())
    }

    fn kv(pares: &[(&str, i64)]) -> BTreeMap<String, i64> {
        pares
            .iter()
            .map(|(chave, valor)| (chave.to_string(), *valor))
            .collect()
    }

    #[test]
    fn preco_ignora_negativo_e_zero_na_venda() {
        assert_eq!(price_of(Some(5), false), Some(5));
        assert_eq!(price_of(Some(0), true), Some(0));
        assert_eq!(price_of(Some(0), false), None);
        assert_eq!(price_of(Some(-1), true), None);
        assert_eq!(price_of(None, true), None);
    }

    #[test]
    fn catalogo_filtra_itens_sem_preco_valido() {
        let shop = shop();
        assert!(shop.catalog.contains_key("pocao"));
        assert!(shop.catalog.contains_key("erva"));
        // Preço negativo/0 na venda: as duas entradas são inválidas.
        assert!(!shop.catalog.contains_key("ruim"));
        assert!(!shop.catalog.contains_key("sem-loja"));
        // `shop.label` vazio cai no label do item.
        assert_eq!(shop.catalog["semente"].label, "Semente");
        assert_eq!(shop_price(&shop, "pocao", TradeAction::Buy), Some(2));
        assert_eq!(shop_price(&shop, "pocao", TradeAction::Sell), None);
    }

    #[test]
    fn teto_usa_stack_ou_stack_max_como_o_lua() {
        let shop = shop();
        assert_eq!(cap_of(&shop.vault, "semente"), 3);
        assert_eq!(cap_of(&shop.vault, "pocao"), 5);
        assert_eq!(cap_of(&shop.vault, "desconhecido"), 5);
        // O `cap_of` do Lua não consulta o `resource_cap` (3): é o `vault_add`
        // que o aplica, e é por isso que o rollback existe.
        assert_eq!(cap_of(&shop.vault, "seiva"), 5);
    }

    #[test]
    fn linhas_marcam_compra_venda_e_bloqueio() {
        let shop = shop();
        let rows = shop_rows(&shop, &kv(&[("item.gold", 1), ("item.erva", 2)]));
        let por_id = |id: &str| rows.iter().find(|row| row.id == id).cloned();

        let erva = por_id("erva").expect("erva");
        assert!(erva.selling);
        assert_eq!(erva.status, ShopStatus::Sell);
        assert_eq!(erva.price, 2);
        assert_eq!(erva.stock, 2);

        let pocao = por_id("pocao").expect("pocao");
        assert!(!pocao.selling);
        // 1 ouro < 2: não chega.
        assert_eq!(pocao.status, ShopStatus::Unaffordable);

        let oferta = por_id("oferta").expect("oferta");
        assert_eq!(oferta.status, ShopStatus::Buy);

        // Semente tem venda: a linha é de venda, não de compra.
        let semente = por_id("semente").expect("semente");
        assert!(semente.selling);

        // Sem stock a venda nasce bloqueada.
        let reliquia = por_id("reliquia").expect("reliquia");
        assert_eq!(reliquia.status, ShopStatus::Locked);

        // Ordem da UI: label, depois id.
        let labels: Vec<&str> = rows.iter().map(|row| row.label.as_str()).collect();
        let mut sorted = labels.clone();
        sorted.sort_unstable();
        assert_eq!(labels, sorted);
    }

    #[test]
    fn compra_com_sucesso_tira_o_ouro_e_entrega_o_item() {
        let shop = shop();
        let mut inventario = kv(&[("item.gold", 2)]);
        let out = shop_trade(&shop, &mut inventario, TradeAction::Buy, "pocao");
        assert!(out.ok, "razão: {:?}", out.reason);
        assert_eq!(out.price, Some(2));
        assert_eq!(out.label.as_deref(), Some("Comprar poção"));
        assert_eq!(inventario["item.gold"], 0);
        assert_eq!(inventario["item.pocao"], 1);
    }

    #[test]
    fn compra_sem_ouro_nao_toca_no_cofre() {
        let shop = shop();
        let mut inventario = kv(&[("item.gold", 1)]);
        let out = shop_trade(&shop, &mut inventario, TradeAction::Buy, "pocao");
        assert!(!out.ok);
        assert_eq!(out.reason, Some(TradeReason::NoGold));
        assert_eq!(inventario["item.gold"], 1);
        assert!(!inventario.contains_key("item.pocao"));
    }

    #[test]
    fn compra_com_pilha_cheia_nao_gasta_ouro() {
        let shop = shop();
        let mut inventario = kv(&[("item.gold", 3), ("item.semente", 3)]);
        let out = shop_trade(&shop, &mut inventario, TradeAction::Buy, "semente");
        assert!(!out.ok);
        assert_eq!(out.reason, Some(TradeReason::Full));
        assert_eq!(out.cap, Some(3));
        assert_eq!(inventario["item.gold"], 3);
        assert_eq!(inventario["item.semente"], 3);
    }

    #[test]
    fn compra_sem_espaco_devolve_o_ouro() {
        let shop = shop();
        // `seiva` é recurso e tem `resource_cap` 3: o teto da banca (5)
        // deixa passar, o `vault_add` corta e a compra reembolsa.
        let mut inventario = kv(&[("item.gold", 2), ("item.seiva", 3)]);
        let out = shop_trade(&shop, &mut inventario, TradeAction::Buy, "seiva");
        assert!(!out.ok);
        assert_eq!(out.reason, Some(TradeReason::NoSpace));
        assert_eq!(inventario["item.gold"], 2, "o ouro tem de voltar");
        assert_eq!(inventario["item.seiva"], 3);
    }

    #[test]
    fn compra_a_zero_nao_desconta_ouro() {
        let shop = shop();
        let mut inventario = kv(&[("item.gold", 0)]);
        let out = shop_trade(&shop, &mut inventario, TradeAction::Buy, "oferta");
        assert!(out.ok, "razão: {:?}", out.reason);
        assert_eq!(inventario["item.gold"], 0);
        assert_eq!(inventario["item.oferta"], 1);
    }

    #[test]
    fn venda_com_sucesso_tira_o_item_e_paga() {
        let shop = shop();
        let mut inventario = kv(&[("item.erva", 1)]);
        let out = shop_trade(&shop, &mut inventario, TradeAction::Sell, "erva");
        assert!(out.ok, "razão: {:?}", out.reason);
        assert_eq!(out.price, Some(2));
        assert_eq!(inventario["item.erva"], 0);
        assert_eq!(inventario["item.gold"], 2);
    }

    #[test]
    fn venda_sem_preco_ou_sem_stock_falha() {
        let shop = shop();
        let mut inventario = kv(&[("item.gold", 0)]);
        let out = shop_trade(&shop, &mut inventario, TradeAction::Sell, "pocao");
        assert!(!out.ok);
        assert_eq!(out.reason, Some(TradeReason::NoPrice));

        let mut inventario = kv(&[("item.gold", 0)]);
        let out = shop_trade(&shop, &mut inventario, TradeAction::Sell, "erva");
        assert!(!out.ok);
        assert_eq!(out.reason, Some(TradeReason::NoStock));
        assert_eq!(inventario["item.gold"], 0);
    }

    #[test]
    fn venda_com_ouro_cheio_devolve_o_item() {
        let shop = shop();
        // Ouro 2, teto 3, preço 4: entram só 1 e o parcial tem de sair.
        let mut inventario = kv(&[("item.gold", 2), ("item.reliquia", 1)]);
        let out = shop_trade(&shop, &mut inventario, TradeAction::Sell, "reliquia");
        assert!(!out.ok);
        assert_eq!(out.reason, Some(TradeReason::GoldFull));
        assert_eq!(inventario["item.gold"], 2, "o parcial tem de sair");
        assert_eq!(inventario["item.reliquia"], 1, "o item tem de voltar");
    }

    #[test]
    fn accao_desconhecida_e_item_desconhecido() {
        let shop = shop();
        let mut inventario = BTreeMap::new();
        let out = shop_trade(&shop, &mut inventario, TradeAction::Buy, "nao-existe");
        assert!(!out.ok);
        assert_eq!(out.reason, Some(TradeReason::UnknownItem));
        // Preços inválidos (negativo na compra, 0 na venda) ficam fora do
        // catálogo: nem uma compra os encontra.
        let out = shop_trade(&shop, &mut inventario, TradeAction::Buy, "ruim");
        assert_eq!(out.reason, Some(TradeReason::UnknownItem));
        assert!(inventario.is_empty());
    }

    #[test]
    fn receitas_filtram_id_vazio_repetido_e_custos_invalidos() {
        let craft = craft();
        assert_eq!(craft.recipes.len(), 5);
        assert!(craft.get("").is_none());
        // A primeira `pocao` ganha: produz poção, não erva.
        assert_eq!(craft.get("pocao").expect("pocao").out, "pocao");
        // count 0 → 1 unidade; custos não positivos ficam de fora.
        let oferta = craft.get("oferta").expect("oferta");
        assert_eq!(oferta.output_count(), 1);
        assert!(oferta.cost.is_empty());
    }

    #[test]
    fn forja_com_sucesso_tira_custos_e_entrega_o_produto() {
        let craft = craft();
        let mut inventario = kv(&[("item.erva", 2), ("item.gold", 1)]);
        let out = craft_make(&craft, &mut inventario, "pocao");
        assert!(out.ok, "razão: {:?}", out.reason);
        assert_eq!(out.output, "pocao");
        assert_eq!(out.count, 2);
        assert_eq!(inventario["item.erva"], 0);
        assert_eq!(inventario["item.gold"], 0);
        assert_eq!(inventario["item.pocao"], 2);
    }

    #[test]
    fn forja_sem_material_nao_toca_no_cofre() {
        let craft = craft();
        let mut inventario = kv(&[("item.erva", 4)]);
        let out = craft_make(&craft, &mut inventario, "sem");
        assert!(!out.ok);
        assert_eq!(out.reason, Some(CraftReason::Missing));
        assert_eq!(inventario["item.erva"], 4);
        assert!(!inventario.contains_key("item.pocao"));
    }

    #[test]
    fn forja_com_mochila_cheia_nao_tira_custos() {
        let craft = craft();
        // Teto da poção = 5; 4 + 2 não cabe.
        let mut inventario = kv(&[("item.erva", 1), ("item.pocao", 4)]);
        let out = craft_make(&craft, &mut inventario, "muita");
        assert!(!out.ok);
        assert_eq!(out.reason, Some(CraftReason::Full));
        assert_eq!(inventario["item.erva"], 1);
        assert_eq!(inventario["item.pocao"], 4);
    }

    #[test]
    fn forja_reverte_quando_o_produto_nao_cabe_no_teto_do_cofre() {
        let craft = craft();
        // `seiva` é recurso: o cap da receita (5) deixa passar, o cofre (3)
        // corta a produção a 1 e os custos voltam.
        let mut inventario = kv(&[("item.erva", 2), ("item.seiva", 2)]);
        let out = craft_make(&craft, &mut inventario, "seiva");
        assert!(!out.ok);
        assert_eq!(out.reason, Some(CraftReason::Full));
        assert_eq!(inventario["item.erva"], 2, "custos devolvidos");
        assert_eq!(inventario["item.seiva"], 2, "produção parcial revertida");
    }

    #[test]
    fn forja_desconhecida_nao_toca_no_cofre() {
        let craft = craft();
        let mut inventario = kv(&[("item.erva", 2)]);
        let out = craft_make(&craft, &mut inventario, "nao-existe");
        assert!(!out.ok);
        assert_eq!(out.reason, Some(CraftReason::Unknown));
        assert_eq!(out.count, 0);
        assert_eq!(inventario["item.erva"], 2);
    }

    #[test]
    fn status_da_receita_segue_o_cofre() {
        let craft = craft();
        let vazio = kv(&[]);
        assert_eq!(
            craft_status(&craft, &vazio, "pocao"),
            Some(CraftStatus::Missing)
        );
        assert_eq!(craft_status(&craft, &vazio, "nao-existe"), None);

        let pronto = kv(&[("item.erva", 2), ("item.gold", 1)]);
        assert_eq!(
            craft_status(&craft, &pronto, "pocao"),
            Some(CraftStatus::Ok)
        );

        let cheio = kv(&[("item.erva", 1), ("item.pocao", 4)]);
        assert_eq!(
            craft_status(&craft, &cheio, "muita"),
            Some(CraftStatus::Full)
        );
    }

    // ── bridge ──────────────────────────────────────────────────────────

    fn lua_com_bridge() -> (Lua, Table) {
        let lua = Lua::new();
        lua.globals()
            .set("viber", lua.create_table().expect("tabela viber"))
            .expect("global viber");
        let game = lua.create_table().expect("tabela do jogo");
        lua.set_named_registry_value("viber_game", game.clone())
            .expect("registar viber_game");
        install(&lua, shop(), craft()).expect("instalar viber.vael");
        (lua, game)
    }

    #[test]
    fn bridge_preco_e_linhas() {
        let (lua, game) = lua_com_bridge();
        game.raw_set("item.gold", 1).expect("gold");
        let price: i64 = lua
            .load("return viber.vael.shop_price('pocao', 'buy')")
            .eval()
            .expect("shop_price");
        assert_eq!(price, 2);
        assert!(
            lua.load("return viber.vael.shop_price('pocao', 'sell') == nil")
                .eval::<bool>()
                .expect("sell nil")
        );
        assert!(
            lua.load("return viber.vael.shop_price('nao-existe', 'buy') == nil")
                .eval::<bool>()
                .expect("desconhecido nil")
        );

        let rows: Table = lua
            .load("return viber.vael.shop_rows()")
            .eval()
            .expect("shop_rows");
        assert!(rows.raw_len() > 0, "lista vazia");
        let first: Table = rows.get(1).expect("primeira linha");
        assert!(first.get::<String>("id").expect("id").len() > 0);
        assert!(first.get::<String>("status").expect("status").len() > 0);
        assert!(first.get::<i64>("price").expect("price") >= 0);
        // O ouro do `viber_game` chega à linha da poção (preço 2 > 1).
        let pocao = rows
            .sequence_values::<Table>()
            .flatten()
            .find(|row| row.get::<String>("id").is_ok_and(|id| id == "pocao"))
            .expect("linha da poção");
        assert_eq!(
            pocao.get::<String>("status").expect("status"),
            "unaffordable"
        );
    }

    #[test]
    fn bridge_negocia_no_viber_game() {
        let (lua, game) = lua_com_bridge();
        game.raw_set("item.gold", 2).expect("gold");
        let ok: bool = lua
            .load("return viber.vael.shop_trade('buy', 'pocao').ok")
            .eval()
            .expect("compra");
        assert!(ok);
        assert_eq!(game.raw_get::<i64>("item.gold").expect("gold"), 0);
        assert_eq!(game.raw_get::<i64>("item.pocao").expect("pocao"), 1);

        let reason: String = lua
            .load("return viber.vael.shop_trade('sell', 'pocao').reason")
            .eval()
            .expect("venda sem preço");
        assert_eq!(reason, "no_price");
        assert!(
            lua.load("return viber.vael.shop_trade('give', 'pocao').reason == 'unknown_action'")
                .eval::<bool>()
                .expect("acção inválida")
        );
    }

    #[test]
    fn bridge_lista_receitas_status_e_forja() {
        let (lua, game) = lua_com_bridge();
        let recipes: Table = lua
            .load("return viber.vael.craft_recipes()")
            .eval()
            .expect("craft_recipes");
        assert_eq!(recipes.raw_len(), 5);
        let first: Table = recipes.get(1).expect("primeira receita");
        assert_eq!(first.get::<String>("id").expect("id"), "pocao");
        assert_eq!(first.get::<String>("out").expect("out"), "pocao");
        let cost: Table = first.get("cost").expect("cost");
        assert_eq!(cost.get::<i64>("erva").expect("erva"), 2);

        game.raw_set("item.erva", 2).expect("erva");
        game.raw_set("item.gold", 1).expect("gold");
        let status: String = lua
            .load("return viber.vael.craft_status('pocao')")
            .eval()
            .expect("craft_status");
        assert_eq!(status, "ok");
        assert!(
            lua.load("return viber.vael.craft_status('nao-existe') == nil")
                .eval::<bool>()
                .expect("status desconhecido")
        );

        let ok: bool = lua
            .load("return viber.vael.craft_make('pocao').ok")
            .eval()
            .expect("craft_make");
        assert!(ok);
        assert_eq!(game.raw_get::<i64>("item.erva").expect("erva"), 0);
        assert_eq!(game.raw_get::<i64>("item.gold").expect("gold"), 0);
        assert_eq!(game.raw_get::<i64>("item.pocao").expect("pocao"), 2);

        let out: Table = lua
            .load("return viber.vael.craft_make('nao-existe')")
            .eval()
            .expect("forja desconhecida");
        assert!(!out.get::<bool>("ok").expect("ok"));
        assert_eq!(out.get::<String>("reason").expect("reason"), "unknown");
    }

    #[test]
    fn bridge_nao_substitui_funcoes_existentes() {
        let lua = Lua::new();
        lua.globals()
            .set("viber", lua.create_table().expect("tabela viber"))
            .expect("global viber");
        lua.load("viber.vael = { vault_add = function() return 7 end }")
            .exec()
            .expect("vael antigo");
        install(&lua, shop(), craft()).expect("instalar");
        let valor: i64 = lua
            .load("return viber.vael.vault_add()")
            .eval()
            .expect("vault_add intacto");
        assert_eq!(valor, 7);
        assert!(
            lua.load("return type(viber.vael.shop_rows) == 'function'")
                .eval::<bool>()
                .expect("shop_rows")
        );
    }

    #[test]
    fn bridge_devolve_a_tabela_que_o_lua_consome() {
        // A tabela de resultado é a que o `lib/craft.lua` lê no `make`.
        let (lua, game) = lua_com_bridge();
        game.raw_set("item.erva", 1).expect("erva");
        game.raw_set("item.pocao", 4).expect("pocao");
        let result: Table = lua
            .load("return viber.vael.craft_make('muita')")
            .eval()
            .expect("craft_make");
        assert!(!result.get::<bool>("ok").expect("ok"));
        assert_eq!(result.get::<i64>("count").expect("count"), 2);
        assert_eq!(result.get::<String>("output").expect("output"), "pocao");
        assert_eq!(result.get::<String>("reason").expect("reason"), "full");
    }

    #[test]
    fn plugin_sem_host_nao_panica() {
        let mut app = App::new();
        app.add_plugins(bevy::MinimalPlugins)
            .add_plugins(VaelEconomyPlugin);
        for _ in 0..2 {
            app.update();
        }
    }

    // ── módulos Lua contra a bridge ─────────────────────────────────────

    /// Stubs mínimos do `viber` (regras/UI/som) e do `viber_game`.
    const STUBS: &str = r#"
lists, toasts, sounds, state, events = {}, {}, {}, {}, {}
syncs, recorded, reported = 0, 0, {}
labels = { pocao = "Poção", erva = "Erva", semente = "Semente",
           oferta = "Oferta", seiva = "Seiva", reliquia = "Relíquia" }
local rules = {
  vault_get = function(kind) return game["item." .. kind] or 0 end,
  item_label = function(id) return labels[id] or id end,
  item_def = function() return { desc = "" } end,
  sync_collect = function() syncs = syncs + 1 end,
  report_craft = function(out, count)
    reported.out, reported.count = out, count
  end,
  add_xp = function(n) reported.xp = (reported.xp or 0) + n end,
}
local actions = { record = function() recorded = recorded + 1 end }
viber.load = function(path)
  if path == "lib/rules.lua" then return rules end
  if path == "lib/actions.lua" then return actions end
  error("módulo inesperado: " .. tostring(path))
end
viber.sound = function(name) sounds[#sounds + 1] = name end
viber.player_position = function() return false end
viber.toast = function(text) toasts[#toasts + 1] = text end
viber.ui = { list = function(name, rows) lists[name] = rows end }
viber.own_action = function() end
viber.state = function() return state end
viber.every = function() return 1 end
viber.timer_cancel = function() end
viber.events = function() return events end
"#;

    fn lua_com_modulos() -> (Lua, Table) {
        let lua = Lua::new();
        lua.globals()
            .set("viber", lua.create_table().expect("tabela viber"))
            .expect("global viber");
        let game = lua.create_table().expect("tabela do jogo");
        lua.globals()
            .set("game", game.clone())
            .expect("global game");
        lua.set_named_registry_value("viber_game", game.clone())
            .expect("registar viber_game");
        install(&lua, shop(), craft()).expect("instalar viber.vael");
        lua.load(STUBS).set_name("stubs").exec().expect("stubs");
        (lua, game)
    }

    fn linha(list: &Table, id: &str) -> Table {
        list.sequence_values::<Table>()
            .flatten()
            .find(|row| row.get::<String>("id").is_ok_and(|value| value == id))
            .unwrap_or_else(|| panic!("linha {id} ausente"))
    }

    #[test]
    fn shop_lua_formata_as_linhas_e_negocia_pela_bridge() {
        let (lua, game) = lua_com_modulos();
        game.raw_set("item.gold", 0).expect("gold");
        game.raw_set("item.erva", 1).expect("erva");
        let module: Table = lua
            .load(include_str!("../scripts/game/shop.lua"))
            .set_name("game/shop.lua")
            .eval()
            .expect("carregar shop.lua");

        // A lista nasce preenchida no top-level do módulo.
        let rows: Table = lua.load("return lists.shop").eval().expect("lista");
        let erva = linha(&rows, "erva");
        assert_eq!(
            erva.get::<String>("label").expect("label"),
            "Vender erva ×1"
        );
        assert_eq!(erva.get::<String>("price").expect("price"), "+2");
        assert_eq!(erva.get::<String>("status").expect("status"), "sell");
        let pocao = linha(&rows, "pocao");
        assert_eq!(pocao.get::<String>("price").expect("price"), "2");
        assert_eq!(
            pocao.get::<String>("status").expect("status"),
            "unaffordable"
        );

        let trade: mlua::Function = module.get("trade").expect("M.trade");
        trade.call::<()>(("sell", "erva")).expect("vender erva");
        assert_eq!(game.raw_get::<i64>("item.gold").expect("gold"), 2);
        assert_eq!(game.raw_get::<i64>("item.erva").expect("erva"), 0);
        let toast: String = lua.load("return toasts[#toasts]").eval().expect("toast");
        assert_eq!(toast, "Vendido por 2 de ouro.");
        let syncs: i64 = lua.load("return syncs").eval().expect("syncs");
        assert_eq!(syncs, 1);

        // Compra com o ouro que a venda rendeu.
        trade.call::<()>(("buy", "pocao")).expect("comprar poção");
        assert_eq!(game.raw_get::<i64>("item.gold").expect("gold"), 0);
        assert_eq!(game.raw_get::<i64>("item.pocao").expect("pocao"), 1);
        let toast: String = lua
            .load("return toasts[#toasts]")
            .eval()
            .expect("toast da compra");
        assert_eq!(toast, "Comprado poção.");

        // Sem ouro, a falha chega ao toast (e não mexe no cofre).
        trade.call::<()>(("buy", "pocao")).expect("compra falhada");
        let toast: String = lua
            .load("return toasts[#toasts]")
            .eval()
            .expect("toast de falha");
        assert_eq!(toast, "Comprar poção custa 2 de ouro — não chega.");
        assert_eq!(game.raw_get::<i64>("item.pocao").expect("pocao"), 1);
    }

    #[test]
    fn craft_lua_delega_a_bridge_e_relata_a_quest() {
        let (lua, game) = lua_com_modulos();
        game.raw_set("item.erva", 2).expect("erva");
        game.raw_set("item.gold", 1).expect("gold");
        let module: Table = lua
            .load(include_str!("../scripts/lib/craft.lua"))
            .set_name("lib/craft.lua")
            .eval()
            .expect("carregar craft.lua");

        let make: mlua::Function = module.get("make").expect("M.make");
        let ok: bool = make.call("pocao").expect("forjar poção");
        assert!(ok);
        assert_eq!(game.raw_get::<i64>("item.erva").expect("erva"), 0);
        assert_eq!(game.raw_get::<i64>("item.gold").expect("gold"), 0);
        assert_eq!(game.raw_get::<i64>("item.pocao").expect("pocao"), 2);

        // O relato da quest e o XP continuam a ser do Lua, depois do Rust.
        let out: String = lua.load("return reported.out").eval().expect("out");
        assert_eq!(out, "pocao");
        let count: i64 = lua.load("return reported.count").eval().expect("count");
        assert_eq!(count, 2);
        let xp: i64 = lua.load("return reported.xp").eval().expect("xp");
        assert_eq!(xp, 10);
        let toast: String = lua.load("return toasts[#toasts]").eval().expect("toast");
        assert_eq!(toast, "Forjado: Poção ×2.");

        // A lista `forge` usa o `craft_status` da bridge.
        let refresh: mlua::Function = module.get("refresh").expect("M.refresh");
        refresh.call::<()>(()).expect("refresh");
        let rows: Table = lua.load("return lists.forge").eval().expect("linhas");
        let pocao = linha(&rows, "pocao");
        assert_eq!(pocao.get::<String>("status").expect("status"), "missing");
        assert_eq!(pocao.get::<String>("label").expect("label"), "Poção ×2");

        // Receita desconhecida não faz toast nem mexe no cofre.
        let ok: bool = make.call("nao-existe").expect("forja desconhecida");
        assert!(!ok);
    }
}

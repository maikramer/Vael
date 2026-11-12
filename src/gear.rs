//! Equipamento e passivas em Rust: as regras de `scripts/lib/gear.lua` e o
//! `learned_stats` do `rules.lua`.
//!
//! O estado do jogo são dois mapas: as contagens da mochila
//! ([`ItemCounts`], `BTreeMap<String, f64>` como no [`crate::xp`]) e os slots
//! equipados ([`EquippedItems`], `gear.<slot>` → id — o valor é uma string no
//! Luau e não cabe num mapa de números). As funções puras recebem os dois em
//! [`GearState`] e não tocam na VM; a bridge [`install`] instala
//! `viber.vael.gear_*`/`player_stats`/`stat` no arranque e lê/escreve a
//! tabela `viber_game`, como o cofre e o XP.
//!
//! As passivas já não são somadas à mão: os efeitos alimentam o grafo de
//! atributos do [`crate::stats`] (`bevy_gauge`) e as stats derivadas vêm de
//! um [`crate::stats::StatSnapshot`]. O `Stats` plano continua a ser a vista
//! compatível com o Lua (`bonus_damage`, `crit_bonus`, `speed_mult`,
//! `max_hp_bonus`, `fx`).
//!
//! Regras portadas de `scripts/lib/gear.lua` e `scripts/lib/rules.lua`:
//! - um item só conta com `item.<id>` numérico, não NaN e > 0;
//! - `gear.<slot>` só vale para um item do mesmo slot e ainda na mochila —
//!   uma marca órfã é limpa na leitura (o diário não mostra um fantasma);
//! - `toggle` devolve `"on"`/`"off"`, `"blocked"` quando os atributos não
//!   satisfazem o `requires` do item, e `nil` para item não equipável/ausente;
//! - os `effects` somam por slot (`weapon`, `armor`, `charm`) e ignoram NaN;
//! - chaves `strength`/`agility`/`vitality`/`precision`/`spirit` mexem nos
//!   atributos; as restantes são `Stat.<chave>` (as antigas `damage`, `crit`,
//!   `speed`, `max_hp` mantêm o campo próprio nas [`Stats`]);
//! - a arma vazia é a espada base (25 de dano, 1.4 de velocidade, 3.0 de
//!   alcance, grip `espada`); um `weapon` parcial completa com esses valores
//!   (0/ausente no JSON cai no default — o serde não distingue os dois).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard};

use bevy::prelude::*;
use bevy_gauge::prelude::AttributesPlugin;
use mlua::{Lua, Table, Value};
use viber::luau::host::LuaScriptHost;

use crate::content::{ItemsFile, VaelContent, WeaponDef};
use crate::stats::{AttributeRules, StatEngine, StatSnapshot};

/// Slots de equipamento, na ordem em que os efeitos são somados.
pub const SLOTS: [&str; 3] = ["weapon", "armor", "charm"];

/// Prefixos das chaves no `viber.game()`.
pub const ITEM_KEY_PREFIX: &str = "item.";
pub const GEAR_KEY_PREFIX: &str = "gear.";
pub const SKILL_KEY_PREFIX: &str = "skill.";

/// Defaults da espada base (`M.DEFAULT_WEAPON` do Lua).
pub const DEFAULT_DAMAGE: f64 = 25.0;
pub const DEFAULT_SPEED: f64 = 1.4;
pub const DEFAULT_RANGE: f64 = 3.0;
pub const DEFAULT_GRIP: &str = "espada";

/// Contagens da mochila: id do item → unidades.
pub type ItemCounts = BTreeMap<String, f64>;

/// Slots equipados: slot → id do item.
pub type EquippedItems = BTreeMap<String, String>;

/// Estado do equipamento no jogo.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GearState {
    /// Contagens da mochila (chave = id; só interessa > 0).
    pub held: ItemCounts,
    /// Slots equipados (`gear.<slot>` do save).
    pub equipped: EquippedItems,
}

/// Definição de um item para o equipamento.
#[derive(Debug, Clone, Default)]
pub struct GearItem {
    pub id: String,
    pub label: String,
    /// `weapon | armor | charm` (validado por [`GearRules::slot_of`]).
    pub slot: String,
    pub effects: BTreeMap<String, f64>,
    pub weapon: Option<WeaponDef>,
    /// Expressão de requisito do gauge (`"Strength >= 7"`), vazia = sem gate.
    pub requires: String,
    /// Texto curto do requisito para o diário (`"Força 7"`).
    pub requires_text: String,
}

/// Regras do equipamento derivadas de `data/items.json` (e dos talentos de
/// `data/progression.json`).
#[derive(Debug, Clone, Default)]
pub struct GearRules {
    /// Chave = id do item.
    pub items: BTreeMap<String, GearItem>,
    /// Chave = id do talento → efeitos.
    pub skills: BTreeMap<String, BTreeMap<String, f64>>,
}

impl GearRules {
    /// Só o equipamento (`VaelContent.items`).
    pub fn from_items(file: &ItemsFile) -> Self {
        let mut items = BTreeMap::new();
        for def in &file.items {
            if def.id.is_empty() {
                continue;
            }
            items.insert(
                def.id.clone(),
                GearItem {
                    id: def.id.clone(),
                    label: def.label.clone(),
                    slot: def.slot.clone(),
                    effects: def.effects.clone(),
                    weapon: def.weapon.clone(),
                    requires: def.requires.clone(),
                    requires_text: def.requires_text.clone(),
                },
            );
        }
        Self {
            items,
            skills: BTreeMap::new(),
        }
    }

    /// Equipamento + efeitos dos talentos (`VaelContent`).
    pub fn from_content(content: &VaelContent) -> Self {
        let mut rules = Self::from_items(&content.items);
        for skill in &content.progression.skills {
            if skill.id.is_empty() {
                continue;
            }
            rules.skills.insert(skill.id.clone(), skill.effects.clone());
        }
        rules
    }

    /// Slot equipável do item, ou `None` (consumível, id desconhecido, slot
    /// fora de `weapon|armor|charm`).
    pub fn slot_of(&self, id: &str) -> Option<&'static str> {
        let slot = self.items.get(id)?.slot.as_str();
        SLOTS.iter().copied().find(|known| *known == slot)
    }
}

/// Chave da mochila no `viber.game()`.
pub fn item_key(id: &str) -> String {
    format!("{ITEM_KEY_PREFIX}{id}")
}

/// Chave do slot equipado no `viber.game()`.
pub fn gear_key(slot: &str) -> String {
    format!("{GEAR_KEY_PREFIX}{slot}")
}

/// O item está na mochila? (`type(n) == "number" and n == n and n > 0`).
fn held(state: &GearState, id: &str) -> bool {
    if id.is_empty() {
        return false;
    }
    state
        .held
        .get(id)
        .is_some_and(|count| !count.is_nan() && *count > 0.0)
}

/// Id equipado no slot, ou `None`.
///
/// Uma marca de um item vendido/entregue ou de outro slot é limpa do estado
/// (como o `viber.game()[key] = nil` do Lua).
pub fn equipped(rules: &GearRules, state: &mut GearState, slot: &str) -> Option<String> {
    let id = state.equipped.get(slot)?.clone();
    if id.is_empty() {
        return None;
    }
    if rules.slot_of(&id) != Some(slot) || !held(state, &id) {
        state.equipped.remove(slot);
        return None;
    }
    Some(id)
}

/// O item está equipado no seu slot?
pub fn is_equipped(rules: &GearRules, state: &mut GearState, id: &str) -> bool {
    let Some(slot) = rules.slot_of(id) else {
        return false;
    };
    equipped(rules, state, slot).is_some_and(|current| current == id)
}

/// Resultado de [`toggle`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toggle {
    On,
    Off,
}

impl Toggle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::On => "on",
            Self::Off => "off",
        }
    }
}

/// Equipa (ou tira, se já estava). `None` = não equipável ou fora da mochila.
pub fn toggle(rules: &GearRules, state: &mut GearState, id: &str) -> Option<Toggle> {
    let slot = rules.slot_of(id)?;
    if !held(state, id) {
        return None;
    }
    if state
        .equipped
        .get(slot)
        .is_some_and(|current| current == id)
    {
        state.equipped.remove(slot);
        return Some(Toggle::Off);
    }
    state.equipped.insert(slot.to_string(), id.to_string());
    Some(Toggle::On)
}

/// Soma dos `effects` de tudo o que está equipado: chave → total.
pub fn effects(rules: &GearRules, state: &mut GearState) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    for slot in SLOTS {
        let Some(id) = equipped(rules, state, slot) else {
            continue;
        };
        let Some(def) = rules.items.get(&id) else {
            continue;
        };
        for (key, value) in &def.effects {
            // NaN não é efeito; ±inf fica e o `add_effects` filtra-o.
            if !value.is_nan() {
                *out.entry(key.clone()).or_insert(0.0) += *value;
            }
        }
    }
    out
}

/// Números do melee de uma arma.
#[derive(Debug, Clone, PartialEq)]
pub struct Weapon {
    pub id: String,
    pub label: String,
    pub damage: f64,
    pub speed: f64,
    pub range: f64,
    pub grip: String,
}

impl Default for Weapon {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: "Espada".to_string(),
            damage: DEFAULT_DAMAGE,
            speed: DEFAULT_SPEED,
            range: DEFAULT_RANGE,
            grip: DEFAULT_GRIP.to_string(),
        }
    }
}

/// Números do melee da arma equipada (ou da espada base).
pub fn weapon(rules: &GearRules, state: &mut GearState) -> Weapon {
    let Some(id) = equipped(rules, state, "weapon") else {
        return Weapon::default();
    };
    let Some(def) = rules.items.get(&id) else {
        return Weapon::default();
    };
    let Some(def_weapon) = def.weapon.as_ref() else {
        return Weapon::default();
    };
    let base = Weapon::default();
    Weapon {
        id: id.clone(),
        label: if def.label.is_empty() {
            id
        } else {
            def.label.clone()
        },
        damage: positive(def_weapon.damage).unwrap_or(base.damage),
        speed: positive(def_weapon.speed).unwrap_or(base.speed),
        range: positive(def_weapon.range).unwrap_or(base.range),
        grip: if def_weapon.grip.is_empty() {
            base.grip
        } else {
            def_weapon.grip.clone()
        },
    }
}

/// Valor positivo e finito; 0/negativo/NaN/inf caem no default.
fn positive(value: f64) -> Option<f64> {
    (value.is_finite() && value > 0.0).then_some(value)
}

/// Passivas do herói (talentos + equipamento).
#[derive(Debug, Clone, PartialEq)]
pub struct Stats {
    pub bonus_damage: f64,
    pub crit_bonus: f64,
    pub speed_mult: f64,
    pub max_hp_bonus: f64,
    /// Chaves genéricas (`defense`, `lifesteal`, `dash_cd`, …).
    pub fx: BTreeMap<String, f64>,
}

impl Default for Stats {
    fn default() -> Self {
        Self {
            bonus_damage: 0.0,
            crit_bonus: 0.0,
            speed_mult: 1.0,
            max_hp_bonus: 0.0,
            fx: BTreeMap::new(),
        }
    }
}

impl From<&StatSnapshot> for Stats {
    /// Vista compatível do snapshot do gauge. As quatro stats antigas têm
    /// campo próprio; tudo o resto (incluindo as novas `attack_speed`,
    /// `knockback`, `heal_power` e efeitos arbitrários) vai para `fx`.
    fn from(snapshot: &StatSnapshot) -> Self {
        let mut fx = BTreeMap::new();
        for (key, value) in &snapshot.stats {
            if !matches!(key.as_str(), "damage" | "crit" | "speed" | "max_hp") {
                fx.insert(key.clone(), *value);
            }
        }
        Self {
            bonus_damage: snapshot.stat("damage"),
            crit_bonus: snapshot.stat("crit"),
            speed_mult: snapshot.speed_mult(),
            max_hp_bonus: snapshot.stat("max_hp"),
            fx,
        }
    }
}

/// Passivas dos talentos aprendidos + equipamento.
///
/// Conveniência para testes/chamadas isoladas: cria um [`StatEngine`] novo.
/// A bridge usa o motor partilhado (`install`) para não reconstruir o grafo
/// a cada leitura.
pub fn learned_stats(
    rules: &GearRules,
    state: &mut GearState,
    learned: &BTreeSet<String>,
) -> Stats {
    let mut engine = StatEngine::new(AttributeRules::default());
    Stats::from(engine.snapshot(rules, state, learned))
}

/// O mesmo que [`learned_stats`]: as passivas do jogador no jogo.
pub fn player_stats(rules: &GearRules, state: &mut GearState, learned: &BTreeSet<String>) -> Stats {
    learned_stats(rules, state, learned)
}

/// Uma chave das passivas (`defense`, `lifesteal`, `attack_speed`, …);
/// identidades (`attack_speed`/`knockback`/`heal_power`) valem 1.0, o resto 0.
pub fn stat(
    rules: &GearRules,
    state: &mut GearState,
    learned: &BTreeSet<String>,
    key: &str,
) -> f64 {
    let mut engine = StatEngine::new(AttributeRules::default());
    engine.snapshot(rules, state, learned).stat(key)
}

// ── bridge Luau ─────────────────────────────────────────────────────────

/// Plugin do equipamento: instala as funções em `viber.vael` depois do
/// cofre e do XP.
pub struct VaelGearPlugin;

impl Plugin for VaelGearPlugin {
    fn build(&self, app: &mut App) {
        // O grafo de atributos usa os recursos/schedules do gauge; o
        // `StatEngine` da bridge tem uma `World` própria (ver `stats.rs`),
        // por isso o plugin não é obrigatório para as funções Lua — fica
        // para o dia em que a entidade do herói viver na App principal.
        app.add_plugins(AttributesPlugin);
        // `Startup` corre depois do `PreStartup` que carrega o `VaelContent`.
        app.add_systems(
            Startup,
            install_gear_api
                .after(crate::vault::install_vault_api)
                .after(crate::xp::install_xp_api),
        );
    }
}

/// Arranque: acrescenta as funções de equipamento à tabela `viber.vael`.
///
/// Público dentro do crate para as quests ordenarem depois dele.
pub(crate) fn install_gear_api(
    host: Option<Res<LuaScriptHost>>,
    content: Option<Res<VaelContent>>,
) {
    let Some(host) = host else {
        warn!("vael: equipamento sem host Luau — `viber.vael` não estendido");
        return;
    };
    let Some(content) = content else {
        warn!("vael: equipamento sem conteúdo carregado — `viber.vael` não estendido");
        return;
    };
    let rules = GearRules::from_content(&content);
    match install(&host.lua, rules) {
        Ok(()) => info!(
            itens = content.items.items.len(),
            talentos = content.progression.skills.len(),
            "vael: equipamento em Rust ligado (`viber.vael`)"
        ),
        Err(err) => error!("vael: falha a instalar equipamento em `viber.vael`: {err}"),
    }
}

/// Acrescenta as funções de equipamento a `viber.vael`.
///
/// Usa a tabela já criada pelo cofre/XP; só cria uma nova se esta não existir.
/// O [`StatEngine`] é partilhado por todas as funções que leem as passivas —
/// reconstrói o grafo só quando o conjunto equipado/talentos muda.
pub(crate) fn install(lua: &Lua, rules: GearRules) -> mlua::Result<()> {
    let rules = Arc::new(rules);
    let stats = Arc::new(Mutex::new(StatEngine::new(AttributeRules::default())));
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
            "gear_slot_of",
            lua.create_function(move |_, id: Option<String>| {
                Ok(rules.slot_of(&id.unwrap_or_default()).map(str::to_owned))
            })?,
        )?;
    }

    {
        let rules = Arc::clone(&rules);
        vael.set(
            "gear_equipped",
            lua.create_function(move |lua, slot: Option<String>| {
                let slot = slot.unwrap_or_default();
                let (game, mut state) = read_state(lua)?;
                let out = equipped(&rules, &mut state, &slot);
                write_gear(&game, &state)?;
                Ok(out)
            })?,
        )?;
    }

    {
        let rules = Arc::clone(&rules);
        vael.set(
            "gear_is_equipped",
            lua.create_function(move |lua, id: Option<String>| {
                let id = id.unwrap_or_default();
                let (game, mut state) = read_state(lua)?;
                let out = is_equipped(&rules, &mut state, &id);
                write_gear(&game, &state)?;
                Ok(out)
            })?,
        )?;
    }

    {
        let rules = Arc::clone(&rules);
        let stats = Arc::clone(&stats);
        vael.set(
            "gear_toggle",
            lua.create_function(move |lua, id: Option<String>| {
                let id = id.unwrap_or_default();
                let (game, mut state) = read_state(lua)?;
                read_held(&game, &id, &mut state)?;
                let learned = read_learned(&game, &rules)?;
                let requirement = rules
                    .items
                    .get(&id)
                    .map(|item| item.requires.clone())
                    .unwrap_or_default();
                // Tirar o que já está equipado nunca é bloqueado: o requisito
                // é para ENTRAR, não para sair (um item que dá o atributo não
                // pode prender o jogador).
                let already_equipped = is_equipped(&rules, &mut state, &id);
                let met = already_equipped || {
                    let mut engine = lock_stats(&stats);
                    let _ = engine.snapshot(&rules, &mut state, &learned);
                    engine.requirement_met(&requirement)
                };
                if !met {
                    // O snapshot pode ter limpado uma marca órfã; grava-a.
                    write_gear(&game, &state)?;
                    return Ok(Some("blocked"));
                }
                let out = toggle(&rules, &mut state, &id).map(Toggle::as_str);
                write_gear(&game, &state)?;
                Ok(out)
            })?,
        )?;
    }

    {
        let rules = Arc::clone(&rules);
        vael.set(
            "gear_requirement",
            lua.create_function(move |_, id: Option<String>| {
                let id = id.unwrap_or_default();
                Ok(rules
                    .items
                    .get(&id)
                    .map(|item| item.requires.clone())
                    .filter(|requires| !requires.is_empty()))
            })?,
        )?;
    }

    {
        let rules = Arc::clone(&rules);
        vael.set(
            "gear_requirement_text",
            lua.create_function(move |_, id: Option<String>| {
                let id = id.unwrap_or_default();
                Ok(rules
                    .items
                    .get(&id)
                    .map(|item| item.requires_text.clone())
                    .filter(|text| !text.is_empty()))
            })?,
        )?;
    }

    {
        let rules = Arc::clone(&rules);
        let stats = Arc::clone(&stats);
        vael.set(
            "gear_requirement_met",
            lua.create_function(move |lua, id: Option<String>| {
                let id = id.unwrap_or_default();
                let Some(item) = rules.items.get(&id) else {
                    return Ok(false);
                };
                let (game, mut state) = read_state(lua)?;
                let learned = read_learned(&game, &rules)?;
                let met = {
                    let mut engine = lock_stats(&stats);
                    let _ = engine.snapshot(&rules, &mut state, &learned);
                    engine.requirement_met(&item.requires)
                };
                write_gear(&game, &state)?;
                Ok(met)
            })?,
        )?;
    }

    {
        let rules = Arc::clone(&rules);
        vael.set(
            "gear_effects",
            lua.create_function(move |lua, ()| {
                let (game, mut state) = read_state(lua)?;
                let effects = effects(&rules, &mut state);
                write_gear(&game, &state)?;
                let table = lua.create_table()?;
                for (key, value) in effects {
                    table.set(key, value)?;
                }
                Ok(table)
            })?,
        )?;
    }

    {
        let rules = Arc::clone(&rules);
        vael.set(
            "gear_weapon",
            lua.create_function(move |lua, ()| {
                let (game, mut state) = read_state(lua)?;
                let weapon = weapon(&rules, &mut state);
                write_gear(&game, &state)?;
                weapon_table(lua, &weapon)
            })?,
        )?;
    }

    {
        let rules = Arc::clone(&rules);
        let stats = Arc::clone(&stats);
        vael.set(
            "player_stats",
            lua.create_function(move |lua, ()| {
                let (game, mut state) = read_state(lua)?;
                let learned = read_learned(&game, &rules)?;
                let snapshot = {
                    let mut engine = lock_stats(&stats);
                    engine.snapshot(&rules, &mut state, &learned).clone()
                };
                write_gear(&game, &state)?;
                stats_table(lua, &Stats::from(&snapshot))
            })?,
        )?;
    }

    {
        let rules = Arc::clone(&rules);
        let stats = Arc::clone(&stats);
        vael.set(
            "stat",
            lua.create_function(move |lua, key: Option<String>| {
                let key = key.unwrap_or_default();
                let (game, mut state) = read_state(lua)?;
                let learned = read_learned(&game, &rules)?;
                let value = {
                    let mut engine = lock_stats(&stats);
                    engine.snapshot(&rules, &mut state, &learned).stat(&key)
                };
                write_gear(&game, &state)?;
                Ok(value)
            })?,
        )?;
    }

    {
        let rules = Arc::clone(&rules);
        let stats = Arc::clone(&stats);
        vael.set(
            "attribute",
            lua.create_function(move |lua, key: Option<String>| {
                let key = key.unwrap_or_default();
                let (game, mut state) = read_state(lua)?;
                let learned = read_learned(&game, &rules)?;
                let value = {
                    let mut engine = lock_stats(&stats);
                    engine
                        .snapshot(&rules, &mut state, &learned)
                        .attribute(&key)
                };
                write_gear(&game, &state)?;
                Ok(value)
            })?,
        )?;
    }

    {
        let rules = Arc::clone(&rules);
        let stats = Arc::clone(&stats);
        vael.set(
            "attributes",
            lua.create_function(move |lua, ()| {
                let (game, mut state) = read_state(lua)?;
                let learned = read_learned(&game, &rules)?;
                let snapshot = {
                    let mut engine = lock_stats(&stats);
                    engine.snapshot(&rules, &mut state, &learned).clone()
                };
                write_gear(&game, &state)?;
                let table = lua.create_table()?;
                for (key, value) in &snapshot.attributes {
                    table.set(key.as_str(), *value)?;
                }
                Ok(table)
            })?,
        )?;
    }

    {
        let rules = Arc::clone(&rules);
        let stats = Arc::clone(&stats);
        vael.set(
            "stat_sheet",
            lua.create_function(move |lua, ()| {
                let (game, mut state) = read_state(lua)?;
                let learned = read_learned(&game, &rules)?;
                let snapshot = {
                    let mut engine = lock_stats(&stats);
                    engine.snapshot(&rules, &mut state, &learned).clone()
                };
                write_gear(&game, &state)?;
                stat_sheet_table(lua, &snapshot)
            })?,
        )?;
    }

    Ok(())
}

/// Trava o motor do stat sheet, recuperando de um pânico num closure Lua.
fn lock_stats(engine: &Arc<Mutex<StatEngine>>) -> MutexGuard<'_, StatEngine> {
    engine
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Lê os slots e as contagens que as funções puras precisam de validar.
///
/// Só os `item.<id>` dos ids equipados são lidos; `gear_toggle` acrescenta a
/// contagem do id pedido.
fn read_state(lua: &Lua) -> mlua::Result<(Table, GearState)> {
    let game: Table = lua.named_registry_value("viber_game")?;
    let mut state = GearState::default();
    for slot in SLOTS {
        if let Value::String(text) = game.raw_get::<Value>(gear_key(slot))? {
            state
                .equipped
                .insert(slot.to_string(), text.to_str()?.to_string());
        }
    }
    let equipped: Vec<String> = state.equipped.values().cloned().collect();
    for id in equipped {
        read_held(&game, &id, &mut state)?;
    }
    Ok((game, state))
}

/// Lê `item.<id>` do jogo; ausente/não numérico não entra no mapa.
fn read_held(game: &Table, id: &str, state: &mut GearState) -> mlua::Result<()> {
    match game.raw_get::<Value>(item_key(id))? {
        Value::Integer(count) => {
            state.held.insert(id.to_string(), count as f64);
        }
        Value::Number(count) => {
            state.held.insert(id.to_string(), count);
        }
        _ => {}
    }
    Ok(())
}

/// Talentos aprendidos: `skill.<id> == true`, como o Lua.
fn read_learned(game: &Table, rules: &GearRules) -> mlua::Result<BTreeSet<String>> {
    let mut learned = BTreeSet::new();
    for id in rules.skills.keys() {
        let key = format!("{SKILL_KEY_PREFIX}{id}");
        if matches!(game.raw_get::<Value>(key)?, Value::Boolean(true)) {
            learned.insert(id.clone());
        }
    }
    Ok(learned)
}

/// Escreve os três slots de volta (limpa as marcas órfãs que as funções
/// puras removeram).
fn write_gear(game: &Table, state: &GearState) -> mlua::Result<()> {
    for slot in SLOTS {
        match state.equipped.get(slot) {
            Some(id) => game.raw_set(gear_key(slot), id.as_str())?,
            None => game.raw_set(gear_key(slot), Value::Nil)?,
        }
    }
    Ok(())
}

fn stats_table(lua: &Lua, stats: &Stats) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("bonus_damage", stats.bonus_damage)?;
    table.set("crit_bonus", stats.crit_bonus)?;
    table.set("speed_mult", stats.speed_mult)?;
    table.set("max_hp_bonus", stats.max_hp_bonus)?;
    let fx = lua.create_table()?;
    for (key, value) in &stats.fx {
        fx.set(key.as_str(), *value)?;
    }
    table.set("fx", fx)?;
    Ok(table)
}

/// `viber.vael.stat_sheet()`: atributos e stats derivadas para o diário.
fn stat_sheet_table(lua: &Lua, snapshot: &StatSnapshot) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    let attributes = lua.create_table()?;
    for (key, value) in &snapshot.attributes {
        attributes.set(key.as_str(), *value)?;
    }
    let stats = lua.create_table()?;
    for (key, value) in &snapshot.stats {
        stats.set(key.as_str(), *value)?;
    }
    table.set("attributes", attributes)?;
    table.set("stats", stats)?;
    table.set("speed_mult", snapshot.speed_mult())?;
    Ok(table)
}

fn weapon_table(lua: &Lua, weapon: &Weapon) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("id", weapon.id.as_str())?;
    table.set("label", weapon.label.as_str())?;
    table.set("damage", weapon.damage)?;
    table.set("speed", weapon.speed)?;
    table.set("range", weapon.range)?;
    table.set("grip", weapon.grip.as_str())?;
    Ok(table)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{ItemDef, ItemsFile, ProgressionFile, SkillDef, VaelContent};
    use crate::vault::{VaultRules, install as install_vault};
    use crate::xp::{XpRules, install as install_xp};

    fn arma(dano: f64) -> WeaponDef {
        WeaponDef {
            damage: dano,
            speed: 1.0,
            range: 2.0,
            grip: "machado".into(),
            ..Default::default()
        }
    }

    fn item(id: &str, slot: &str, effects: &[(&str, f64)]) -> ItemDef {
        ItemDef {
            id: id.into(),
            label: id.into(),
            slot: slot.into(),
            effects: effects
                .iter()
                .map(|(key, value)| (key.to_string(), *value))
                .collect(),
            ..Default::default()
        }
    }

    fn regras() -> GearRules {
        let file = ItemsFile {
            items: vec![
                ItemDef {
                    weapon: Some(arma(38.0)),
                    ..item("machado", "weapon", &[("crit", 0.05)])
                },
                ItemDef {
                    weapon: Some(arma(50.0)),
                    requires: "Strength >= 7".into(),
                    requires_text: "Força 7".into(),
                    ..item("machado_pesado", "weapon", &[])
                },
                item("manto", "armor", &[("defense", 0.15), ("max_hp", 15.0)]),
                item("amuleto", "charm", &[("damage", 8.0), ("lifesteal", 0.04)]),
                item("pocao", "", &[("max_hp", 999.0)]),
            ],
            ..Default::default()
        };
        let mut rules = GearRules::from_items(&file);
        rules.skills.insert(
            "forca".into(),
            BTreeMap::from([
                ("damage".to_string(), 5.0),
                ("speed".to_string(), 0.1),
                ("strike_burn".to_string(), 3.0),
            ]),
        );
        rules.skills.insert(
            "strength1".into(),
            BTreeMap::from([("strength".to_string(), 2.0)]),
        );
        rules
    }

    fn estado(pares: &[(&str, f64)]) -> GearState {
        GearState {
            held: pares
                .iter()
                .map(|(id, count)| (id.to_string(), *count))
                .collect(),
            equipped: EquippedItems::new(),
        }
    }

    #[test]
    fn slot_of_so_aceita_os_tres_slots() {
        let rules = regras();
        assert_eq!(rules.slot_of("machado"), Some("weapon"));
        assert_eq!(rules.slot_of("manto"), Some("armor"));
        assert_eq!(rules.slot_of("amuleto"), Some("charm"));
        assert_eq!(rules.slot_of("pocao"), None);
        assert_eq!(rules.slot_of("desconhecido"), None);
        assert_eq!(rules.slot_of(""), None);
    }

    #[test]
    fn toggle_liga_e_desliga() {
        let rules = regras();
        let mut state = estado(&[("machado", 1.0)]);
        assert_eq!(toggle(&rules, &mut state, "machado"), Some(Toggle::On));
        assert_eq!(state.equipped.get("weapon"), Some(&"machado".to_string()));
        assert_eq!(toggle(&rules, &mut state, "machado"), Some(Toggle::Off));
        assert!(!state.equipped.contains_key("weapon"));
        assert!(!is_equipped(&rules, &mut state, "machado"));
    }

    #[test]
    fn toggle_recusa_item_ausente_nao_equipavel_ou_contagem_invalida() {
        let rules = regras();
        let mut state = estado(&[("machado", 0.0), ("manto", -1.0), ("amuleto", f64::NAN)]);
        assert_eq!(toggle(&rules, &mut state, "machado"), None);
        assert_eq!(toggle(&rules, &mut state, "manto"), None);
        assert_eq!(toggle(&rules, &mut state, "amuleto"), None);
        assert_eq!(toggle(&rules, &mut state, "pocao"), None);
        assert_eq!(toggle(&rules, &mut state, "desconhecido"), None);
        assert!(state.equipped.is_empty());
    }

    #[test]
    fn equipped_limpa_marca_orfa_ou_de_outro_slot() {
        let rules = regras();
        let mut state = estado(&[("machado", 1.0)]);
        state.equipped.insert("weapon".into(), "machado".into());
        assert_eq!(
            equipped(&rules, &mut state, "weapon").as_deref(),
            Some("machado")
        );

        // Vendido: a marca sai do save.
        state.held.insert("machado".into(), 0.0);
        assert_eq!(equipped(&rules, &mut state, "weapon"), None);
        assert!(!state.equipped.contains_key("weapon"));

        // Item de outro slot não vale como arma.
        state.held.insert("manto".into(), 1.0);
        state.equipped.insert("weapon".into(), "manto".into());
        assert_eq!(equipped(&rules, &mut state, "weapon"), None);
        assert!(!state.equipped.contains_key("weapon"));

        // String vazia: nil, mas a marca fica (o Lua também não a limpa).
        state.equipped.insert("weapon".into(), String::new());
        assert_eq!(equipped(&rules, &mut state, "weapon"), None);
        assert_eq!(state.equipped.get("weapon"), Some(&String::new()));
    }

    #[test]
    fn is_equipped_so_para_o_proprio_slot() {
        let rules = regras();
        let mut state = estado(&[("machado", 1.0), ("manto", 1.0)]);
        assert!(!is_equipped(&rules, &mut state, "machado"));
        toggle(&rules, &mut state, "machado");
        assert!(is_equipped(&rules, &mut state, "machado"));
        assert!(!is_equipped(&rules, &mut state, "manto"));
        assert!(!is_equipped(&rules, &mut state, "pocao"));
    }

    #[test]
    fn effects_soma_os_tres_slots() {
        let rules = regras();
        let mut state = estado(&[("machado", 1.0), ("manto", 1.0), ("amuleto", 1.0)]);
        for (id, slot) in [
            ("machado", "weapon"),
            ("manto", "armor"),
            ("amuleto", "charm"),
        ] {
            state.equipped.insert(slot.into(), id.into());
        }
        let effects = effects(&rules, &mut state);
        assert_eq!(effects.get("damage"), Some(&8.0));
        assert_eq!(effects.get("defense"), Some(&0.15));
        assert_eq!(effects.get("crit"), Some(&0.05));
        assert_eq!(effects.get("max_hp"), Some(&15.0));
        assert_eq!(effects.get("lifesteal"), Some(&0.04));
    }

    #[test]
    fn effects_de_item_com_nan_nao_entra() {
        let mut rules = regras();
        let mut efeitos = BTreeMap::new();
        efeitos.insert("damage".to_string(), f64::NAN);
        efeitos.insert("defense".to_string(), 0.2);
        rules.items.get_mut("manto").expect("manto").effects = efeitos;
        let mut state = estado(&[("manto", 1.0)]);
        state.equipped.insert("armor".into(), "manto".into());
        let effects = effects(&rules, &mut state);
        assert!(!effects.contains_key("damage"));
        assert_eq!(effects.get("defense"), Some(&0.2));
    }

    #[test]
    fn weapon_default_quando_nao_ha_arma() {
        let rules = regras();
        let mut state = estado(&[]);
        let weapon = weapon(&rules, &mut state);
        assert_eq!(
            weapon,
            Weapon {
                id: String::new(),
                label: "Espada".into(),
                damage: 25.0,
                speed: 1.4,
                range: 3.0,
                grip: "espada".into(),
            }
        );
    }

    #[test]
    fn weapon_equipada_usa_os_numeros_do_json() {
        let rules = regras();
        let mut state = estado(&[("machado", 1.0)]);
        state.equipped.insert("weapon".into(), "machado".into());
        let weapon = weapon(&rules, &mut state);
        assert_eq!(weapon.id, "machado");
        assert_eq!(weapon.label, "machado");
        assert_eq!(weapon.damage, 38.0);
        assert_eq!(weapon.speed, 1.0);
        assert_eq!(weapon.range, 2.0);
        assert_eq!(weapon.grip, "machado");
    }

    #[test]
    fn weapon_parcial_completa_com_a_espada_base() {
        let mut file = ItemsFile::default();
        file.items.push(ItemDef {
            id: "adaga".into(),
            label: String::new(),
            slot: "weapon".into(),
            weapon: Some(WeaponDef {
                damage: 12.0,
                ..Default::default()
            }),
            ..Default::default()
        });
        let rules = GearRules::from_items(&file);
        let mut state = estado(&[("adaga", 1.0)]);
        state.equipped.insert("weapon".into(), "adaga".into());
        let weapon = weapon(&rules, &mut state);
        assert_eq!(weapon.id, "adaga");
        // Sem label no JSON, o id faz de nome (`def.label or id`).
        assert_eq!(weapon.label, "adaga");
        assert_eq!(weapon.damage, 12.0);
        assert_eq!(weapon.speed, 1.4);
        assert_eq!(weapon.range, 3.0);
        assert_eq!(weapon.grip, "espada");
    }

    #[test]
    fn weapon_sem_def_cai_na_espada_base() {
        let mut file = ItemsFile::default();
        file.items.push(ItemDef {
            id: "cetro".into(),
            label: "Cetro".into(),
            slot: "weapon".into(),
            ..Default::default()
        });
        let rules = GearRules::from_items(&file);
        let mut state = estado(&[("cetro", 1.0)]);
        state.equipped.insert("weapon".into(), "cetro".into());
        assert_eq!(weapon(&rules, &mut state), Weapon::default());
    }

    #[test]
    fn stats_somam_talentos_e_equipamento() {
        let rules = regras();
        let mut state = estado(&[("machado", 1.0), ("manto", 1.0), ("amuleto", 1.0)]);
        state.equipped.insert("weapon".into(), "machado".into());
        state.equipped.insert("armor".into(), "manto".into());
        state.equipped.insert("charm".into(), "amuleto".into());

        let stats = learned_stats(&rules, &mut state, &BTreeSet::from(["forca".into()]));
        assert_eq!(stats.bonus_damage, 13.0); // 5 (talento) + 8 (amuleto)
        assert_eq!(stats.crit_bonus, 0.05);
        assert_eq!(stats.speed_mult, 1.1);
        assert_eq!(stats.max_hp_bonus, 15.0);
        assert_eq!(stats.fx.get("defense"), Some(&0.15));
        assert_eq!(stats.fx.get("lifesteal"), Some(&0.04));
        assert_eq!(stats.fx.get("strike_burn"), Some(&3.0));
    }

    #[test]
    fn stats_sem_nada_tem_velocidade_1() {
        let rules = regras();
        let mut state = estado(&[]);
        let stats = learned_stats(&rules, &mut state, &BTreeSet::new());
        assert_eq!(stats.bonus_damage, 0.0);
        assert_eq!(stats.crit_bonus, 0.0);
        assert_eq!(stats.max_hp_bonus, 0.0);
        assert_eq!(stats.speed_mult, 1.0);
        assert_eq!(stats.fx.get("attack_speed"), Some(&1.0));
        assert_eq!(stats.fx.get("knockback"), Some(&1.0));
    }

    #[test]
    fn stat_le_as_chaves_genericas_e_devolve_zero() {
        let rules = regras();
        let mut state = estado(&[("amuleto", 1.0)]);
        state.equipped.insert("charm".into(), "amuleto".into());
        let learned = BTreeSet::from(["forca".into()]);
        assert_eq!(stat(&rules, &mut state, &learned, "lifesteal"), 0.04);
        assert_eq!(stat(&rules, &mut state, &learned, "strike_burn"), 3.0);
        assert_eq!(stat(&rules, &mut state, &learned, "defense"), 0.0);
        assert_eq!(stat(&rules, &mut state, &learned, ""), 0.0);
    }

    #[test]
    fn stats_ignoram_efeitos_nao_finitos_dos_talentos() {
        let mut rules = regras();
        rules.skills.insert(
            "quebrado".into(),
            BTreeMap::from([
                ("damage".to_string(), f64::NAN),
                ("speed".to_string(), f64::INFINITY),
                ("defense".to_string(), 0.1),
            ]),
        );
        let mut state = estado(&[]);
        let stats = learned_stats(&rules, &mut state, &BTreeSet::from(["quebrado".into()]));
        assert_eq!(stats.bonus_damage, 0.0);
        assert_eq!(stats.speed_mult, 1.0);
        assert_eq!(stats.fx.get("defense"), Some(&0.1));
    }

    #[test]
    fn regras_vem_do_conteudo() {
        let mut content = VaelContent::default();
        content
            .items
            .items
            .push(item("manto", "armor", &[("max_hp", 5.0)]));
        content.progression = ProgressionFile {
            skills: vec![SkillDef {
                id: "vitalidade".into(),
                effects: BTreeMap::from([("max_hp".to_string(), 20.0)]),
                ..Default::default()
            }],
            ..Default::default()
        };
        let rules = GearRules::from_content(&content);
        assert_eq!(rules.slot_of("manto"), Some("armor"));
        assert_eq!(
            rules
                .skills
                .get("vitalidade")
                .expect("talento")
                .get("max_hp"),
            Some(&20.0)
        );
    }

    fn lua_com_bridge(rules: GearRules) -> (Lua, Table) {
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
    fn bridge_toggle_le_e_escreve_gear_no_viber_game() {
        let (lua, game) = lua_com_bridge(regras());
        game.raw_set("item.machado", 2).expect("machado");

        let result: Option<String> = lua
            .load("return viber.vael.gear_toggle('machado')")
            .eval()
            .expect("gear_toggle");
        assert_eq!(result.as_deref(), Some("on"));
        assert_eq!(
            game.raw_get::<String>("gear.weapon").expect("slot"),
            "machado"
        );
        assert!(
            lua.load("return viber.vael.gear_is_equipped('machado')")
                .eval::<bool>()
                .expect("is_equipped")
        );

        let result: Option<String> = lua
            .load("return viber.vael.gear_toggle('machado')")
            .eval()
            .expect("gear_toggle off");
        assert_eq!(result.as_deref(), Some("off"));
        assert!(matches!(
            game.raw_get::<Value>("gear.weapon").expect("slot"),
            Value::Nil
        ));

        // Item fora da mochila: nil e nenhuma chave criada.
        assert!(
            lua.load("return viber.vael.gear_toggle('manto') == nil")
                .eval::<bool>()
                .expect("toggle sem item")
        );
    }

    #[test]
    fn bridge_equipped_limpa_marca_orfa() {
        let (lua, game) = lua_com_bridge(regras());
        game.raw_set("gear.weapon", "machado").expect("slot");
        // `item.machado` ausente: a arma equipada deixou de contar.
        assert!(
            lua.load("return viber.vael.gear_equipped('weapon') == nil")
                .eval::<bool>()
                .expect("equipped")
        );
        assert!(matches!(
            game.raw_get::<Value>("gear.weapon").expect("slot"),
            Value::Nil
        ));
    }

    #[test]
    fn bridge_weapon_default_e_equipada() {
        let (lua, game) = lua_com_bridge(regras());
        let damage: f64 = lua
            .load("return viber.vael.gear_weapon().damage")
            .eval()
            .expect("arma base");
        assert_eq!(damage, 25.0);
        let grip: String = lua
            .load("return viber.vael.gear_weapon().grip")
            .eval()
            .expect("grip base");
        assert_eq!(grip, "espada");

        game.raw_set("item.machado", 1).expect("machado");
        game.raw_set("gear.weapon", "machado").expect("slot");
        let damage: f64 = lua
            .load("return viber.vael.gear_weapon().damage")
            .eval()
            .expect("arma equipada");
        assert_eq!(damage, 38.0);
    }

    #[test]
    fn bridge_effects_e_player_stats_somam_talentos_e_gear() {
        let (lua, game) = lua_com_bridge(regras());
        game.raw_set("item.manto", 1).expect("manto");
        game.raw_set("gear.armor", "manto").expect("slot");
        game.raw_set("skill.forca", true).expect("talento");

        let defense: f64 = lua
            .load("return viber.vael.gear_effects().defense")
            .eval()
            .expect("effects");
        assert_eq!(defense, 0.15);

        let damage: f64 = lua
            .load("return viber.vael.player_stats().bonus_damage")
            .eval()
            .expect("bonus_damage");
        assert_eq!(damage, 5.0);
        let speed: f64 = lua
            .load("return viber.vael.player_stats().speed_mult")
            .eval()
            .expect("speed_mult");
        assert!((speed - 1.1).abs() < f64::EPSILON);
        let burn: f64 = lua
            .load("return viber.vael.player_stats().fx.strike_burn")
            .eval()
            .expect("fx");
        assert_eq!(burn, 3.0);

        let defense: f64 = lua
            .load("return viber.vael.stat('defense')")
            .eval()
            .expect("stat defense");
        assert_eq!(defense, 0.15);
        let ausente: f64 = lua
            .load("return viber.vael.stat('nao-existe')")
            .eval()
            .expect("stat ausente");
        assert_eq!(ausente, 0.0);
    }

    #[test]
    fn bridge_atributos_e_stat_sheet_propagam_do_talento() {
        let (lua, game) = lua_com_bridge(regras());
        // Sem talentos: base 5 e dano derivado 0.
        let base: f64 = lua
            .load("return viber.vael.attribute('strength')")
            .eval()
            .expect("strength base");
        assert_eq!(base, 5.0);
        let damage: f64 = lua
            .load("return viber.vael.stat('damage')")
            .eval()
            .expect("damage base");
        assert_eq!(damage, 0.0);

        game.raw_set("skill.strength1", true).expect("talento");
        let strength: f64 = lua
            .load("return viber.vael.attribute('strength')")
            .eval()
            .expect("strength");
        assert_eq!(strength, 7.0);
        let damage: f64 = lua
            .load("return viber.vael.stat('damage')")
            .eval()
            .expect("damage");
        assert_eq!(damage, 6.0);

        let sheet: Table = lua
            .load("return viber.vael.stat_sheet()")
            .eval()
            .expect("stat_sheet");
        let attributes: Table = sheet.get("attributes").expect("attributes");
        assert_eq!(attributes.get::<f64>("strength").expect("força"), 7.0);
        let stats: Table = sheet.get("stats").expect("stats");
        assert_eq!(stats.get::<f64>("damage").expect("dano"), 6.0);
        assert_eq!(stats.get::<f64>("attack_speed").expect("ataque"), 1.0);
        assert_eq!(sheet.get::<f64>("speed_mult").expect("velocidade"), 1.0);
    }

    #[test]
    fn equipamento_respeita_requisito_de_atributo() {
        let (lua, game) = lua_com_bridge(regras());
        game.raw_set("item.machado_pesado", 1).expect("item");

        assert!(
            lua.load("return viber.vael.gear_requirement('machado_pesado') == 'Strength >= 7'")
                .eval::<bool>()
                .expect("expressão")
        );
        assert!(
            lua.load("return viber.vael.gear_requirement_text('machado_pesado') == 'Força 7'")
                .eval::<bool>()
                .expect("texto")
        );
        assert!(
            !lua.load("return viber.vael.gear_requirement_met('machado_pesado')")
                .eval::<bool>()
                .expect("requisito")
        );

        // Base 5: o machado pesado fica bloqueado e o slot não muda.
        let blocked: Option<String> = lua
            .load("return viber.vael.gear_toggle('machado_pesado')")
            .eval()
            .expect("toggle bloqueado");
        assert_eq!(blocked.as_deref(), Some("blocked"));
        assert!(matches!(
            game.raw_get::<Value>("gear.weapon").expect("slot"),
            Value::Nil
        ));

        // Um talento de Força chega aos 7 e o item equipa.
        game.raw_set("skill.strength1", true).expect("talento");
        assert!(
            lua.load("return viber.vael.gear_requirement_met('machado_pesado')")
                .eval::<bool>()
                .expect("requisito cumprido")
        );
        let on: Option<String> = lua
            .load("return viber.vael.gear_toggle('machado_pesado')")
            .eval()
            .expect("toggle");
        assert_eq!(on.as_deref(), Some("on"));
        assert_eq!(
            game.raw_get::<String>("gear.weapon").expect("slot"),
            "machado_pesado"
        );
    }

    #[test]
    fn equipamento_com_requisito_pode_sempre_sair() {
        let (lua, game) = lua_com_bridge(regras());
        game.raw_set("item.machado_pesado", 1).expect("item");
        // Marca equipada sem satisfazer Força 7 (save editado, por exemplo).
        game.raw_set("gear.weapon", "machado_pesado").expect("slot");

        let off: Option<String> = lua
            .load("return viber.vael.gear_toggle('machado_pesado')")
            .eval()
            .expect("toggle off");
        assert_eq!(off.as_deref(), Some("off"));
        assert!(matches!(
            game.raw_get::<Value>("gear.weapon").expect("slot"),
            Value::Nil
        ));
    }

    #[test]
    fn bridge_slot_of_delega_a_tabela_de_itens() {
        let (lua, _game) = lua_com_bridge(regras());
        assert_eq!(
            lua.load("return viber.vael.gear_slot_of('manto')")
                .eval::<Option<String>>()
                .expect("slot_of"),
            Some("armor".to_string())
        );
        assert!(
            lua.load("return viber.vael.gear_slot_of('pocao') == nil")
                .eval::<bool>()
                .expect("slot_of sem slot")
        );
    }

    #[test]
    fn gear_lua_delega_para_a_bridge() {
        let host = LuaScriptHost::new(std::path::PathBuf::from("scripts")).expect("host Luau");
        install_vault(&host.lua, VaultRules::from_items(&ItemsFile::default())).expect("cofre");
        install_xp(&host.lua, XpRules::default()).expect("xp");
        install(&host.lua, regras()).expect("gear");
        let game: Table = host
            .lua
            .named_registry_value("viber_game")
            .expect("viber_game");
        game.raw_set("item.machado", 1).expect("machado");

        let equipped: Option<String> = host
            .lua
            .load(
                r#"
                local gear = viber.load("lib/gear.lua")
                assert(gear.DEFAULT_WEAPON.damage == 25)
                assert(gear.slot_of("machado") == "weapon")
                assert(gear.toggle("machado") == "on")
                assert(gear.is_equipped("machado"))
                assert(gear.weapon().damage == 38)
                assert(gear.effects().crit == 0.05)
                return gear.equipped("weapon")
                "#,
            )
            .eval()
            .expect("gear.lua");
        assert_eq!(equipped.as_deref(), Some("machado"));
        assert_eq!(
            game.raw_get::<String>("gear.weapon").expect("slot"),
            "machado"
        );
    }

    #[test]
    fn bridge_convive_com_cofre_e_xp() {
        let lua = Lua::new();
        lua.globals()
            .set("viber", lua.create_table().expect("tabela viber"))
            .expect("global viber");
        let game = lua.create_table().expect("tabela do jogo");
        lua.set_named_registry_value("viber_game", game.clone())
            .expect("registar viber_game");
        install_vault(&lua, VaultRules::from_items(&ItemsFile::default())).expect("cofre");
        install_xp(&lua, XpRules::default()).expect("xp");
        install(&lua, regras()).expect("gear");

        let coexistem: bool = lua
            .load(
                "return type(viber.vael.vault_add) == 'function' \
                 and type(viber.vael.add_xp) == 'function' \
                 and type(viber.vael.gear_toggle) == 'function' \
                 and type(viber.vael.player_stats) == 'function'",
            )
            .eval()
            .expect("funções instaladas");
        assert!(coexistem, "cofre, XP e equipamento partilham `viber.vael`");
    }

    #[test]
    fn plugin_sem_host_nao_panica() {
        let mut app = App::new();
        app.add_plugins(bevy::MinimalPlugins)
            .add_plugins(VaelGearPlugin);
        for _ in 0..2 {
            app.update();
        }
    }

    #[test]
    fn startup_instala_equipamento_na_mesma_tabela() {
        let mut app = App::new();
        app.add_plugins(bevy::MinimalPlugins)
            .insert_resource(LuaScriptHost::new(std::path::PathBuf::from("scripts")).expect("host"))
            .init_resource::<VaelContent>()
            .add_plugins((
                crate::vault::VaelVaultPlugin,
                crate::xp::VaelXpPlugin,
                VaelGearPlugin,
            ));
        app.update();

        let host = app.world().resource::<LuaScriptHost>();
        let lua = &host.lua;
        let coexistem: bool = lua
            .load(
                "return type(viber.vael.vault_add) == 'function' \
                 and type(viber.vael.add_xp) == 'function' \
                 and type(viber.vael.gear_toggle) == 'function' \
                 and type(viber.vael.gear_weapon) == 'function' \
                 and type(viber.vael.gear_effects) == 'function' \
                 and type(viber.vael.gear_slot_of) == 'function' \
                 and type(viber.vael.gear_requirement) == 'function' \
                 and type(viber.vael.gear_requirement_met) == 'function' \
                 and type(viber.vael.player_stats) == 'function' \
                 and type(viber.vael.attribute) == 'function' \
                 and type(viber.vael.attributes) == 'function' \
                 and type(viber.vael.stat_sheet) == 'function' \
                 and type(viber.vael.stat) == 'function'",
            )
            .eval()
            .expect("funções instaladas");
        assert!(coexistem, "equipamento tem de partilhar `viber.vael`");
    }
}

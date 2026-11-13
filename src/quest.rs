//! Quests em Rust: estado, progresso, disponibilidade, aceitação, entrega e
//! recompensas — a aritmética de `scripts/lib/rules.lua`.
//!
//! As funções puras trabalham sobre um [`QuestKv`] (`BTreeMap` de chaves
//! primitivas, como o `viber.game()`) para poderem ser testadas sem a VM. A
//! bridge [`install`] instala `viber.vael.quest_*` depois do equipamento e
//! lê/escreve a tabela `viber_game`; o `rules.lua` fica com os embrulhos
//! (toasts, sons, `sync_collect`, `save_at`) e com tudo o que não é o ciclo
//! da quest (`report_kill`, `scan_visits`, parley, diálogo).
//!
//! Regras portadas de `scripts/lib/rules.lua`:
//! - `q.<id>` guarda o estado (`not_taken`/`active`/`ready`/`done`); um valor
//!   desconhecido conta como `not_taken` para uma def existente e `unknown`
//!   para uma def ausente;
//! - `q.<id>.need` sobrepõe o `count` do objetivo (entrega parcial de
//!   `collect`), `q.<id>.n` é o progresso e `q.<id>.stage` a etapa actual
//!   (1-based, limitada às etapas existentes);
//! - uma etapa intermédia cumprida avança `stage`, repõe o progresso a 0,
//!   apaga os marcadores `q.<id>.v.*` e grava o `need` da etapa seguinte; a
//!   última etapa põe a quest `ready`;
//! - `accept` só de `not_taken` e com os `requires` entregues; `turn_in` só
//!   de `ready`, tira os itens de `collect` do cofre e devolve a quest a
//!   `active` quando faltam;
//! - `npc == "notice_board"` é repetível: a entrega volta a `not_taken` em
//!   vez de `done`; as outras entregas limpam `tracked_quest` quando era a
//!   missão seguida;
//! - a entrega limpa `q.<id>.n`, `.stage`, `.need`, `.said` e todos os
//!   `q.<id>.v.*`, e paga ouro/XP/itens pelas regras puras do cofre e do XP.

use std::collections::BTreeMap;
use std::sync::Arc;

use bevy::prelude::*;
use mlua::{Lua, Table, Value};
use viber::luau::game::{apply_patch, game_to_json};
use viber::luau::host::LuaScriptHost;

use crate::compat::lua_tonumber;
use crate::content::{Objective, Reward, VaelContent};
use crate::vault::{VaultRules, item_key, vael_fn, vael_table, vault_add, vault_take};
use crate::xp::{XP_KEYS, XpMap, XpRules, add_xp};

/// Prefixo das chaves de quest no `viber.game()`.
pub const QUEST_KEY_PREFIX: &str = "q.";
/// Chave da missão seguida pelo HUD.
pub const KEY_TRACKED_QUEST: &str = "tracked_quest";

pub const STATE_NOT_TAKEN: &str = "not_taken";
pub const STATE_ACTIVE: &str = "active";
pub const STATE_READY: &str = "ready";
pub const STATE_DONE: &str = "done";
pub const STATE_UNKNOWN: &str = "unknown";

/// Limite do encadeamento `next` (o `while seen < 16` do Lua).
pub const MAX_CHAIN: usize = 16;

/// Estado primitivo do jogo: as chaves de `viber.game()`.
pub type QuestKv = BTreeMap<String, serde_json::Value>;

/// Definição de uma quest com o que as regras precisam (sem falas/retratos).
#[derive(Debug, Clone, Default)]
pub struct QuestEntry {
    pub id: String,
    pub npc: String,
    pub title: String,
    pub objective: Objective,
    pub stages: Vec<Objective>,
    pub requires: Vec<String>,
    pub next: String,
    pub rewards: Reward,
}

impl QuestEntry {
    /// Aviso do quadro: a entrega volta a `not_taken` em vez de `done`.
    pub fn repeatable(&self) -> bool {
        self.npc == "notice_board"
    }
}

/// Todas as definições de quest, por id, na ordem do `load_defs` do Lua.
#[derive(Debug, Clone, Default)]
pub struct QuestRules {
    pub quests: BTreeMap<String, QuestEntry>,
    /// Ids por ordem alfabética (a lista que o Lua publica).
    pub order: Vec<String>,
}

impl QuestRules {
    pub fn from_content(content: &VaelContent) -> Self {
        let mut quests = BTreeMap::new();
        for (id, def) in &content.quests {
            if id.is_empty() {
                continue;
            }
            quests.insert(
                id.clone(),
                QuestEntry {
                    id: id.clone(),
                    npc: def.npc.clone(),
                    title: def.title.clone(),
                    objective: def.objective.clone(),
                    stages: def.stages.clone(),
                    requires: def.requires.clone(),
                    next: def.next.clone(),
                    rewards: def.rewards.clone(),
                },
            );
        }
        let order = quests.keys().cloned().collect();
        Self { quests, order }
    }
}

/// Chave de estado de uma quest (`q.<id>`).
pub fn quest_key(id: &str) -> String {
    format!("{QUEST_KEY_PREFIX}{id}")
}

fn key_with(id: &str, suffix: &str) -> String {
    format!("{}.{suffix}", quest_key(id))
}

fn visits_prefix(id: &str) -> String {
    format!("{}.v.", quest_key(id))
}

fn raw_string<'a>(kv: &'a QuestKv, key: &str) -> Option<&'a str> {
    match kv.get(key) {
        Some(serde_json::Value::String(text)) => Some(text.as_str()),
        _ => None,
    }
}

fn raw_state<'a>(kv: &'a QuestKv, id: &str) -> Option<&'a str> {
    raw_string(kv, &quest_key(id))
}

/// Número de uma chave (só valores numéricos, como o `finite` do Lua).
fn kv_number(kv: &QuestKv, key: &str) -> Option<f64> {
    match kv.get(key) {
        Some(serde_json::Value::Number(number)) => number.as_f64(),
        _ => None,
    }
}

/// Número com `tonumber`: um save editado à mão pode guardar `"3"` onde o
/// Lua aceitava string numérica (só o `.need` usava `tonumber`).
fn kv_tonumber(kv: &QuestKv, key: &str) -> Option<f64> {
    match kv.get(key) {
        Some(serde_json::Value::Number(number)) => number.as_f64(),
        Some(serde_json::Value::String(text)) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn finite(value: f64) -> Option<f64> {
    value.is_finite().then_some(value)
}

fn json_int(value: i64) -> serde_json::Value {
    serde_json::Value::Number(value.into())
}

fn json_state(state: &str) -> serde_json::Value {
    serde_json::Value::String(state.to_string())
}

/// Número JSON: inteiro quando não tem fração, `f64` caso contrário.
fn json_number(value: f64) -> serde_json::Value {
    if value.is_finite()
        && value.fract() == 0.0
        && (i64::MIN as f64..=i64::MAX as f64).contains(&value)
    {
        return json_int(value as i64);
    }
    serde_json::Number::from_f64(value)
        .map(serde_json::Value::Number)
        .unwrap_or(serde_json::Value::Null)
}

/// Etapa actual (1-based) ou `None` quando a quest não tem `stages`.
fn stage_index(kv: &QuestKv, def: &QuestEntry) -> Option<i64> {
    if def.stages.is_empty() {
        return None;
    }
    let raw = finite(kv_number(kv, &key_with(&def.id, "stage")).unwrap_or(1.0)).unwrap_or(1.0);
    let at = raw.floor() as i64;
    Some(at.clamp(1, def.stages.len() as i64))
}

/// Objetivo em curso: a etapa actual ou o `objective` único.
fn objective<'a>(kv: &QuestKv, def: &'a QuestEntry) -> &'a Objective {
    match stage_index(kv, def) {
        Some(at) => &def.stages[(at - 1) as usize],
        None => &def.objective,
    }
}

/// `count: 0`/ausente é um passo, nunca uma divisão por zero.
fn count_of(obj: &Objective) -> i64 {
    obj.count.max(1)
}

/// Meta efectiva: o `q.<id>.need` sobrepõe o `count` do objectivo.
fn need_of(kv: &QuestKv, def: &QuestEntry) -> i64 {
    if let Some(over) = finite(kv_tonumber(kv, &key_with(&def.id, "need")).unwrap_or(f64::NAN))
        && over >= 1.0
    {
        return over.floor() as i64;
    }
    count_of(objective(kv, def))
}

fn progress_of(kv: &QuestKv, id: &str) -> f64 {
    finite(kv_number(kv, &key_with(id, "n")).unwrap_or(0.0)).unwrap_or(0.0)
}

fn set_progress(kv: &mut QuestKv, id: &str, n: i64) {
    kv.insert(key_with(id, "n"), json_int(n.max(0)));
}

fn set_need(kv: &mut QuestKv, id: &str, n: i64) {
    if n >= 1 {
        kv.insert(key_with(id, "need"), json_int(n));
    } else {
        kv.remove(&key_with(id, "need"));
    }
}

/// Apaga todos os `q.<id>.v.*`, incluindo marcadores órfãos de saves antigos.
fn clear_visits(kv: &mut QuestKv, id: &str) {
    let prefix = visits_prefix(id);
    let stale: Vec<String> = kv
        .keys()
        .filter(|key| key.starts_with(&prefix))
        .cloned()
        .collect();
    for key in stale {
        kv.remove(&key);
    }
}

/// Estado de uma quest: um valor desconhecido cai em `not_taken`/`unknown`.
pub fn quest_state(rules: &QuestRules, kv: &QuestKv, id: &str) -> String {
    if let Some(state) = raw_state(kv, id)
        && matches!(
            state,
            STATE_ACTIVE | STATE_READY | STATE_DONE | STATE_NOT_TAKEN
        )
    {
        return state.to_string();
    }
    if rules.quests.contains_key(id) {
        STATE_NOT_TAKEN.to_string()
    } else {
        STATE_UNKNOWN.to_string()
    }
}

/// Progresso e meta (`have`, `need`), com o `q.<id>.need` a mandar.
pub fn quest_progress(rules: &QuestRules, kv: &QuestKv, id: &str) -> (i64, i64) {
    let have = progress_of(kv, id) as i64;
    if let Some(over) = finite(kv_tonumber(kv, &key_with(id, "need")).unwrap_or(f64::NAN))
        && over >= 1.0
    {
        return (have, over.floor() as i64);
    }
    let need = rules
        .quests
        .get(id)
        .map(|def| count_of(objective(kv, def)))
        .unwrap_or(1);
    (have, need)
}

/// Objectivo em curso e a posição da etapa (`obj`, `at`, `total`).
#[derive(Debug, Clone, Default)]
pub struct ObjectiveView {
    /// O JSON chama-lhe `type` (palavra reservada em Rust).
    pub objective_type: String,
    pub target: String,
    pub count: i64,
    pub title: String,
    pub at: i64,
    pub total: i64,
}

impl ObjectiveView {
    fn empty() -> Self {
        Self {
            at: 1,
            total: 1,
            ..Self::default()
        }
    }
}

pub fn quest_objective(rules: &QuestRules, kv: &QuestKv, id: &str) -> ObjectiveView {
    let Some(def) = rules.quests.get(id) else {
        return ObjectiveView::empty();
    };
    let at = stage_index(kv, def);
    let obj = objective(kv, def);
    ObjectiveView {
        objective_type: obj.objective_type.clone(),
        target: obj.target.clone(),
        count: obj.count,
        title: obj.title.clone(),
        at: at.unwrap_or(1),
        total: at.map(|_| def.stages.len() as i64).unwrap_or(1),
    }
}

/// `requires`: todas as quests listadas têm de estar ENTREGUES (`done`).
pub fn quest_available(rules: &QuestRules, kv: &QuestKv, id: &str) -> bool {
    let Some(def) = rules.quests.get(id) else {
        return false;
    };
    def.requires
        .iter()
        .all(|other| raw_state(kv, other) == Some(STATE_DONE))
}

/// Cadeia de um dador: enquanto a quest estiver entregue e tiver `next`.
pub fn quest_chain(rules: &QuestRules, kv: &QuestKv, id: &str) -> String {
    let mut current = id.to_string();
    for _ in 0..MAX_CHAIN {
        let Some(def) = rules.quests.get(&current) else {
            break;
        };
        if def.next.is_empty()
            || raw_state(kv, &current) != Some(STATE_DONE)
            || !rules.quests.contains_key(&def.next)
        {
            break;
        }
        current = def.next.clone();
    }
    current
}

/// Resultado de um `quest_set_progress`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SetProgressResult {
    pub ok: bool,
    /// A etapa intermédia foi cumprida (`stage` é a etapa CONCLUÍDA).
    pub stage_advanced: bool,
    /// Etapa concluída (ou a actual) e total de etapas.
    pub stage: i64,
    pub total: i64,
    pub title: String,
    /// A quest passou a `ready` com este progresso.
    pub ready: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct MarkReady {
    advanced: bool,
    ready: bool,
    stage: i64,
    total: i64,
}

/// `mark_ready`: avança a etapa ou promove a `ready` quando o progresso
/// chega à meta. Só age sobre uma quest `active`.
fn mark_ready(kv: &mut QuestKv, def: &QuestEntry) -> MarkReady {
    let need = need_of(kv, def);
    let mut out = MarkReady::default();
    if progress_of(kv, &def.id) < need as f64 || raw_state(kv, &def.id) != Some(STATE_ACTIVE) {
        return out;
    }

    let at = stage_index(kv, def);
    let total = def.stages.len() as i64;
    if let Some(at) = at
        && at < total
    {
        // Etapa intermédia: avança e a etapa nova pode já estar satisfeita.
        kv.insert(key_with(&def.id, "stage"), json_int(at + 1));
        set_progress(kv, &def.id, 0);
        clear_visits(kv, &def.id);
        let nxt = count_of(objective(kv, def));
        kv.insert(key_with(&def.id, "need"), json_int(nxt));
        out.advanced = true;
        out.stage = at;
        out.total = total;
        return out;
    }

    kv.insert(quest_key(&def.id), json_state(STATE_READY));
    out.ready = true;
    out.stage = at.unwrap_or(1);
    out.total = at.map(|_| total).unwrap_or(1);
    out
}

/// `quest_set_progress` (quests de decisão/craft): grava o progresso e
/// promove a etapa ou a `ready`.
pub fn quest_set_progress(
    rules: &QuestRules,
    kv: &mut QuestKv,
    id: &str,
    n: f64,
) -> SetProgressResult {
    let Some(def) = rules.quests.get(id) else {
        return SetProgressResult::default();
    };
    if quest_state(rules, kv, id) != STATE_ACTIVE {
        return SetProgressResult::default();
    }
    let n = finite(n).map(|value| value.floor().max(0.0)).unwrap_or(0.0);
    set_progress(kv, id, n as i64);
    let marked = mark_ready(kv, def);
    SetProgressResult {
        ok: true,
        stage_advanced: marked.advanced,
        stage: marked.stage,
        total: marked.total,
        title: def.title.clone(),
        ready: marked.ready,
    }
}

/// `quest_accept`: só de `not_taken` e com os `requires` entregues.
pub fn quest_accept(rules: &QuestRules, kv: &mut QuestKv, id: &str) -> bool {
    let Some(def) = rules.quests.get(id) else {
        return false;
    };
    if quest_state(rules, kv, id) != STATE_NOT_TAKEN || !quest_available(rules, kv, id) {
        return false;
    }
    kv.insert(quest_key(id), json_state(STATE_ACTIVE));
    kv.remove(&key_with(id, "stage"));
    set_progress(kv, id, 0);
    // A etapa 1 é a actual depois de limpar o `.stage`.
    let need = count_of(objective(kv, def));
    set_need(kv, id, need);
    kv.remove(&key_with(id, "said"));
    clear_visits(kv, id);
    true
}

/// Porque é que uma entrega não passou.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnInReason {
    /// A quest não existe nas definições.
    Unknown,
    /// O estado não era `ready`.
    NotReady,
    /// O cofre já não tinha os itens do objectivo `collect`.
    MissingItems,
}

impl TurnInReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::NotReady => "not_ready",
            Self::MissingItems => "missing_items",
        }
    }
}

/// Resultado de um `quest_turn_in`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnInResult {
    pub ok: bool,
    /// Ouro da definição (mesmo que o cofre o tenha limitado).
    pub gold: i64,
    /// Itens que não couberam na mochila.
    pub missing_items: i64,
    pub title: String,
    /// `None` quando a entrega passou.
    pub reason: Option<TurnInReason>,
}

/// Contagem de `item.<kind>` no KV, nunca negativa.
fn item_count(kv: &QuestKv, kind: &str) -> i64 {
    finite(kv_number(kv, &item_key(kind)).unwrap_or(0.0))
        .map(|value| value as i64)
        .unwrap_or(0)
        .max(0)
}

/// `vault_take` puro sobre o KV.
fn take_item(kv: &mut QuestKv, rules: &VaultRules, kind: &str, amount: f64) -> bool {
    if kind.is_empty() {
        return false;
    }
    let key = item_key(kind);
    let mut items = BTreeMap::from([(key.clone(), item_count(kv, kind))]);
    let ok = vault_take(rules, &mut items, kind, amount);
    if ok && let Some(value) = items.get(&key) {
        kv.insert(key, json_int(*value));
    }
    ok
}

/// `vault_add` puro sobre o KV; devolve o que entrou de facto.
fn add_item(kv: &mut QuestKv, rules: &VaultRules, kind: &str, amount: f64) -> i64 {
    if kind.is_empty() {
        return 0;
    }
    let key = item_key(kind);
    let mut items = BTreeMap::from([(key.clone(), item_count(kv, kind))]);
    let added = vault_add(rules, &mut items, kind, amount);
    if added > 0
        && let Some(value) = items.get(&key)
    {
        kv.insert(key, json_int(*value));
    }
    added
}

/// `add_xp` puro: lê as quatro chaves, soma e escreve as que ficaram.
fn grant_xp(kv: &mut QuestKv, rules: &XpRules, gain: f64) {
    let mut map = XpMap::new();
    for key in XP_KEYS {
        if let Some(value) = finite(kv_number(kv, key).unwrap_or(f64::NAN)) {
            map.insert(key.to_string(), value);
        }
    }
    add_xp(&mut map, rules, gain);
    for key in XP_KEYS {
        if let Some(value) = map.get(key) {
            kv.insert(key.to_string(), json_number(*value));
        }
    }
}

/// Divide uma recompensa `"potion:3"` em nome e quantidade.
///
/// O padrão do Lua tolera espaços (`" item : 3 "`) e conta 1 quando não há
/// `:N`; um sufixo que não seja só dígitos invalida a entrada, como lá.
fn parse_item_reward(spec: &str) -> Option<(String, i64)> {
    let spec = spec.trim();
    if spec.is_empty() {
        return None;
    }
    let (name, count) = match spec.split_once(':') {
        Some((name, count)) => (name.trim(), Some(count.trim())),
        None => (spec, None),
    };
    if name.is_empty() {
        return None;
    }
    let want = match count {
        None | Some("") => 1,
        Some(text) if text.chars().all(|ch| ch.is_ascii_digit()) => text.parse::<i64>().ok()?,
        Some(_) => return None,
    };
    Some((name.to_string(), want))
}

/// Paga ouro/XP/itens da definição; devolve quantos itens não couberam.
fn grant_rewards(kv: &mut QuestKv, vault: &VaultRules, xp: &XpRules, rewards: &Reward) -> i64 {
    if rewards.gold > 0 {
        add_item(kv, vault, "gold", rewards.gold as f64);
    }
    if rewards.xp > 0 {
        grant_xp(kv, xp, rewards.xp as f64);
    }
    let mut missing = 0i64;
    for spec in &rewards.items {
        let Some((name, want)) = parse_item_reward(spec) else {
            continue;
        };
        if want <= 0 {
            continue;
        }
        let added = add_item(kv, vault, &name, want as f64);
        if added < want {
            missing = missing.saturating_add(want - added);
        }
    }
    missing
}

/// `quest_turn_in`: só de `ready`; `collect` tira os itens do cofre.
pub fn quest_turn_in(
    rules: &QuestRules,
    vault: &VaultRules,
    xp: &XpRules,
    kv: &mut QuestKv,
    id: &str,
) -> TurnInResult {
    let Some(def) = rules.quests.get(id) else {
        return TurnInResult {
            reason: Some(TurnInReason::Unknown),
            ..TurnInResult::default()
        };
    };
    if quest_state(rules, kv, id) != STATE_READY {
        return TurnInResult {
            title: def.title.clone(),
            reason: Some(TurnInReason::NotReady),
            ..TurnInResult::default()
        };
    }

    let obj = objective(kv, def);
    if obj.objective_type == "collect" {
        let need = need_of(kv, def);
        if !take_item(kv, vault, &obj.target, need as f64) {
            // O cofre perdeu os itens depois de a quest ficar pronta.
            kv.insert(quest_key(id), json_state(STATE_ACTIVE));
            return TurnInResult {
                title: def.title.clone(),
                reason: Some(TurnInReason::MissingItems),
                ..TurnInResult::default()
            };
        }
    }

    if def.repeatable() {
        kv.insert(quest_key(id), json_state(STATE_NOT_TAKEN));
    } else {
        kv.insert(quest_key(id), json_state(STATE_DONE));
        if raw_string(kv, KEY_TRACKED_QUEST) == Some(id) {
            kv.remove(KEY_TRACKED_QUEST);
        }
    }

    clear_visits(kv, id);
    kv.remove(&key_with(id, "n"));
    kv.remove(&key_with(id, "stage"));
    set_need(kv, id, 0);
    kv.remove(&key_with(id, "said"));

    let missing = grant_rewards(kv, vault, xp, &def.rewards);
    TurnInResult {
        ok: true,
        gold: def.rewards.gold,
        missing_items: missing,
        title: def.title.clone(),
        reason: None,
    }
}

// ── bridge Luau ─────────────────────────────────────────────────────────

/// Plugin das quests: instala as funções em `viber.vael` depois do
/// equipamento.
pub struct VaelQuestPlugin;

impl Plugin for VaelQuestPlugin {
    fn build(&self, app: &mut App) {
        // `Startup` corre depois do `PreStartup` que carrega o `VaelContent`.
        app.add_systems(
            Startup,
            install_quest_api
                .after(crate::vault::install_vault_api)
                .after(crate::xp::install_xp_api)
                .after(crate::gear::install_gear_api),
        );
    }
}

/// Arranque: acrescenta as funções de quest à tabela `viber.vael`.
pub(crate) fn install_quest_api(
    host: Option<Res<LuaScriptHost>>,
    content: Option<Res<VaelContent>>,
) {
    let Some(host) = host else {
        warn!("vael: quests sem host Luau — `viber.vael` não estendido");
        return;
    };
    let Some(content) = content else {
        warn!("vael: quests sem conteúdo carregado — `viber.vael` não estendido");
        return;
    };
    let rules = QuestRules::from_content(&content);
    let vault = VaultRules::from_items(&content.items);
    let xp = XpRules::from_progression(&content.progression);
    match install(&host.lua, rules, vault, xp) {
        Ok(()) => info!(
            quests = content.quests.len(),
            "vael: quests em Rust ligadas (`viber.vael`)"
        ),
        Err(err) => error!("vael: falha a instalar quests em `viber.vael`: {err}"),
    }
}

/// Acrescenta as funções `quest_*` a `viber.vael` sem substituir as que já lá
/// estão (cofre, XP, equipamento).
pub(crate) fn install(
    lua: &Lua,
    rules: QuestRules,
    vault: VaultRules,
    xp: XpRules,
) -> mlua::Result<()> {
    let rules = Arc::new(rules);
    let vault = Arc::new(vault);
    let xp = Arc::new(xp);
    let vael = vael_table(lua)?;

    vael_fn!(
        lua,
        vael,
        "quest_state",
        [rules = Arc::clone(&rules)],
        move |lua, id: Option<String>| {
            let id = id.unwrap_or_default();
            let game: Table = lua.named_registry_value("viber_game")?;
            let kv = read_keys(&game, &[quest_key(&id)])?;
            Ok(quest_state(&rules, &kv, &id))
        }
    );

    vael_fn!(
        lua,
        vael,
        "quest_progress",
        [rules = Arc::clone(&rules)],
        move |lua, id: Option<String>| {
            let id = id.unwrap_or_default();
            let game: Table = lua.named_registry_value("viber_game")?;
            let kv = read_keys(
                &game,
                &[
                    key_with(&id, "n"),
                    key_with(&id, "need"),
                    key_with(&id, "stage"),
                ],
            )?;
            let (have, need) = quest_progress(&rules, &kv, &id);
            let table = lua.create_table()?;
            table.set("have", have)?;
            table.set("need", need)?;
            Ok(table)
        }
    );

    vael_fn!(
        lua,
        vael,
        "quest_objective",
        [rules = Arc::clone(&rules)],
        move |lua, id: Option<String>| {
            let id = id.unwrap_or_default();
            let game: Table = lua.named_registry_value("viber_game")?;
            let kv = read_keys(&game, &[key_with(&id, "stage")])?;
            let view = quest_objective(&rules, &kv, &id);
            let obj = lua.create_table()?;
            obj.set("type", view.objective_type.as_str())?;
            obj.set("target", view.target.as_str())?;
            obj.set("count", view.count)?;
            obj.set("title", view.title.as_str())?;
            let table = lua.create_table()?;
            table.set("obj", obj)?;
            table.set("at", view.at)?;
            table.set("total", view.total)?;
            Ok(table)
        }
    );

    vael_fn!(
        lua,
        vael,
        "quest_available",
        [rules = Arc::clone(&rules)],
        move |lua, id: Option<String>| {
            let id = id.unwrap_or_default();
            let Some(def) = rules.quests.get(&id) else {
                return Ok(false);
            };
            let game: Table = lua.named_registry_value("viber_game")?;
            let keys: Vec<String> = def.requires.iter().map(|other| quest_key(other)).collect();
            let kv = read_keys(&game, &keys)?;
            Ok(quest_available(&rules, &kv, &id))
        }
    );

    vael_fn!(
        lua,
        vael,
        "quest_chain",
        [rules = Arc::clone(&rules)],
        move |lua, id: Option<String>| {
            let id = id.unwrap_or_default();
            let game: Table = lua.named_registry_value("viber_game")?;
            // A cadeia é curta (máx. 16 saltos): só os estados percorridos.
            let mut keys = Vec::new();
            let mut current = id.as_str();
            for _ in 0..=MAX_CHAIN {
                keys.push(quest_key(current));
                let Some(def) = rules.quests.get(current) else {
                    break;
                };
                if def.next.is_empty() {
                    break;
                }
                current = def.next.as_str();
            }
            let kv = read_keys(&game, &keys)?;
            Ok(quest_chain(&rules, &kv, &id))
        }
    );

    vael_fn!(
        lua,
        vael,
        "quest_set_progress",
        [rules = Arc::clone(&rules)],
        move |lua, (id, n): (Option<String>, Option<Value>)| {
            let id = id.unwrap_or_default();
            let n = lua_tonumber(n.unwrap_or(Value::Nil));
            let before = game_to_json(lua);
            let mut kv = before.clone();
            let out = quest_set_progress(&rules, &mut kv, &id, n);
            write_kv(lua, &before, &kv);
            let table = lua.create_table()?;
            table.set("ok", out.ok)?;
            table.set("stage_advanced", out.stage_advanced)?;
            table.set("stage", out.stage)?;
            table.set("total", out.total)?;
            table.set("title", out.title.as_str())?;
            table.set("ready", out.ready)?;
            Ok(table)
        }
    );

    vael_fn!(
        lua,
        vael,
        "quest_accept",
        [rules = Arc::clone(&rules)],
        move |lua, id: Option<String>| {
            let id = id.unwrap_or_default();
            let before = game_to_json(lua);
            let mut kv = before.clone();
            let ok = quest_accept(&rules, &mut kv, &id);
            write_kv(lua, &before, &kv);
            Ok(ok)
        }
    );

    vael_fn!(
        lua,
        vael,
        "quest_turn_in",
        [
            rules = Arc::clone(&rules),
            vault = Arc::clone(&vault),
            xp = Arc::clone(&xp)
        ],
        move |lua, id: Option<String>| {
            let id = id.unwrap_or_default();
            let before = game_to_json(lua);
            let mut kv = before.clone();
            let out = quest_turn_in(&rules, &vault, &xp, &mut kv, &id);
            write_kv(lua, &before, &kv);
            let table = lua.create_table()?;
            table.set("ok", out.ok)?;
            table.set("gold", out.gold)?;
            table.set("missing_items", out.missing_items)?;
            table.set("title", out.title.as_str())?;
            table.set(
                "reason",
                out.reason.map(TurnInReason::as_str).unwrap_or_default(),
            )?;
            Ok(table)
        }
    );

    Ok(())
}

/// Valor Lua primitivo → JSON plano; tabelas/funções ficam de fora.
fn lua_to_json(value: Value) -> Option<serde_json::Value> {
    match value {
        Value::Boolean(flag) => Some(serde_json::Value::Bool(flag)),
        Value::Integer(number) => Some(json_int(number)),
        Value::Number(number) => Some(json_number(number)),
        Value::String(text) => Some(serde_json::Value::String(text.to_str().ok()?.to_string())),
        _ => None,
    }
}

/// Lê só as chaves pedidas do `viber_game` (as leituras do HUD correm por
/// quest e todos os frames; clonar o jogo inteiro seria caro).
fn read_keys(game: &Table, keys: &[String]) -> mlua::Result<QuestKv> {
    let mut kv = QuestKv::new();
    for key in keys {
        if let Some(value) = lua_to_json(game.raw_get::<Value>(key.as_str())?) {
            kv.insert(key.clone(), value);
        }
    }
    Ok(kv)
}

/// Escreve no `viber_game` só o que mudou entre duas leituras.
fn write_kv(lua: &Lua, before: &QuestKv, after: &QuestKv) {
    let mut patch = QuestKv::new();
    for (key, value) in after {
        if before.get(key) != Some(value) {
            patch.insert(key.clone(), value.clone());
        }
    }
    for key in before.keys() {
        if !after.contains_key(key) {
            patch.insert(key.clone(), serde_json::Value::Null);
        }
    }
    if !patch.is_empty() {
        apply_patch(lua, &patch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{ItemDef, ItemsFile};

    fn objective(objective_type: &str, target: &str, count: i64) -> Objective {
        Objective {
            objective_type: objective_type.into(),
            target: target.into(),
            count,
            ..Default::default()
        }
    }

    fn entry(id: &str, npc: &str, title: &str, objective: Objective) -> QuestEntry {
        QuestEntry {
            id: id.into(),
            npc: npc.into(),
            title: title.into(),
            objective,
            ..Default::default()
        }
    }

    fn regras() -> QuestRules {
        let mut quests = BTreeMap::new();
        quests.insert(
            "wolves".to_string(),
            QuestEntry {
                rewards: Reward {
                    gold: 80,
                    xp: 50,
                    items: vec!["potion:1".to_string()],
                    ..Default::default()
                },
                ..entry(
                    "wolves",
                    "guard",
                    "Recompensa: lobos",
                    objective("kill", "wolf", 3),
                )
            },
        );
        quests.insert(
            "pelt".to_string(),
            QuestEntry {
                requires: vec!["wolves".to_string()],
                rewards: Reward {
                    gold: 10,
                    xp: 20,
                    items: vec!["potion:1".to_string()],
                    ..Default::default()
                },
                ..entry(
                    "pelt",
                    "hunter",
                    "Peles",
                    objective("collect", "wolf_pelt", 3),
                )
            },
        );
        quests.insert(
            "staged".to_string(),
            QuestEntry {
                stages: vec![
                    objective("collect", "wolf_pelt", 2),
                    objective("craft", "hunter_cloak", 1),
                ],
                ..entry(
                    "staged",
                    "smith",
                    "Etapas",
                    objective("collect", "wolf_pelt", 2),
                )
            },
        );
        quests.insert(
            "board".to_string(),
            QuestEntry {
                rewards: Reward {
                    gold: 5,
                    xp: 5,
                    items: Vec::new(),
                    ..Default::default()
                },
                ..entry(
                    "board",
                    "notice_board",
                    "Aviso",
                    objective("kill", "bandit", 1),
                )
            },
        );
        quests.insert(
            "hoard".to_string(),
            QuestEntry {
                rewards: Reward {
                    gold: 0,
                    xp: 0,
                    items: vec!["potion:3".to_string()],
                    ..Default::default()
                },
                ..entry(
                    "hoard",
                    "guard",
                    "Muitas poções",
                    objective("kill", "wolf", 1),
                )
            },
        );
        quests.insert(
            "a".to_string(),
            entry("a", "npc", "A", objective("kill", "x", 1)),
        );
        quests.insert(
            "b".to_string(),
            entry("b", "npc", "B", objective("kill", "x", 1)),
        );
        quests.insert(
            "c".to_string(),
            entry("c", "npc", "C", objective("kill", "x", 1)),
        );
        quests.insert(
            "chain_a".to_string(),
            QuestEntry {
                next: "chain_b".into(),
                ..entry("chain_a", "npc", "A", objective("kill", "x", 1))
            },
        );
        quests.insert(
            "chain_b".to_string(),
            QuestEntry {
                next: "chain_c".into(),
                ..entry("chain_b", "npc", "B", objective("kill", "x", 1))
            },
        );
        quests.insert(
            "chain_c".to_string(),
            entry("chain_c", "npc", "C", objective("kill", "x", 1)),
        );
        let order = quests.keys().cloned().collect();
        QuestRules { quests, order }
    }

    fn vault_rules() -> VaultRules {
        let file = ItemsFile {
            stack_max: 5,
            resource_cap: 1_000,
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
                    id: "hunter_cloak".into(),
                    kind: "item".into(),
                    ..Default::default()
                },
                ItemDef {
                    id: "wolf_pelt".into(),
                    kind: "item".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        VaultRules::from_items(&file)
    }

    fn kv(pares: &[(&str, serde_json::Value)]) -> QuestKv {
        pares
            .iter()
            .map(|(key, value)| (key.to_string(), value.clone()))
            .collect()
    }

    fn textos(pares: &[(&str, &str)]) -> QuestKv {
        pares
            .iter()
            .map(|(key, value)| (key.to_string(), json_state(value)))
            .collect()
    }

    #[test]
    fn estado_por_omissao_e_estados_validos() {
        let rules = regras();
        let vazio = QuestKv::new();
        assert_eq!(quest_state(&rules, &vazio, "wolves"), STATE_NOT_TAKEN);
        assert_eq!(quest_state(&rules, &vazio, "nao-existe"), STATE_UNKNOWN);
        for state in [STATE_ACTIVE, STATE_READY, STATE_DONE, STATE_NOT_TAKEN] {
            let kv = textos(&[("q.wolves", state)]);
            assert_eq!(quest_state(&rules, &kv, "wolves"), state);
        }
        // Valor inválido (ou não string) cai no default.
        let kv = kv(&[("q.wolves", json_int(3))]);
        assert_eq!(quest_state(&rules, &kv, "wolves"), STATE_NOT_TAKEN);
        let kv = textos(&[("q.wolves", "estranho")]);
        assert_eq!(quest_state(&rules, &kv, "wolves"), STATE_NOT_TAKEN);
    }

    #[test]
    fn progresso_usa_need_override_e_count_do_objectivo() {
        let rules = regras();
        let mut kv = QuestKv::new();
        assert_eq!(quest_progress(&rules, &kv, "wolves"), (0, 3));
        kv.insert("q.wolves.n".into(), json_int(2));
        assert_eq!(quest_progress(&rules, &kv, "wolves"), (2, 3));
        kv.insert("q.wolves.need".into(), json_int(5));
        assert_eq!(quest_progress(&rules, &kv, "wolves"), (2, 5));
        // `need` < 1 é ignorado (volta ao count).
        kv.insert("q.wolves.need".into(), json_int(0));
        assert_eq!(quest_progress(&rules, &kv, "wolves"), (2, 3));
        assert_eq!(
            quest_progress(&rules, &QuestKv::new(), "nao-existe"),
            (0, 1)
        );
    }

    #[test]
    fn objective_devolve_a_etapa_actual_limitada() {
        let rules = regras();
        let mut kv = QuestKv::new();
        let view = quest_objective(&rules, &kv, "staged");
        assert_eq!((view.at, view.total), (1, 2));
        assert_eq!(view.objective_type, "collect");
        assert_eq!(view.target, "wolf_pelt");
        assert_eq!(view.count, 2);

        kv.insert("q.staged.stage".into(), json_int(2));
        let view = quest_objective(&rules, &kv, "staged");
        assert_eq!((view.at, view.total), (2, 2));
        assert_eq!(view.objective_type, "craft");
        assert_eq!(view.target, "hunter_cloak");
        // A meta do progresso também vem da etapa actual.
        assert_eq!(quest_progress(&rules, &kv, "staged"), (0, 1));

        // Fora dos limites: clampa à última/primeira etapa.
        kv.insert("q.staged.stage".into(), json_int(9));
        assert_eq!(quest_objective(&rules, &kv, "staged").at, 2);
        kv.insert("q.staged.stage".into(), json_int(0));
        assert_eq!(quest_objective(&rules, &kv, "staged").at, 1);

        // Sem `stages`: vale o objective único.
        let view = quest_objective(&rules, &kv, "wolves");
        assert_eq!((view.at, view.total), (1, 1));
        assert_eq!(view.objective_type, "kill");

        let view = quest_objective(&rules, &kv, "nao-existe");
        assert_eq!((view.at, view.total), (1, 1));
        assert!(view.objective_type.is_empty());
    }

    #[test]
    fn disponivel_exige_requires_entregues() {
        let rules = regras();
        let mut kv = QuestKv::new();
        assert!(!quest_available(&rules, &kv, "pelt"));
        kv.insert("q.wolves".into(), json_state(STATE_DONE));
        assert!(quest_available(&rules, &kv, "pelt"));
        // Uma quest sem requires está sempre disponível (se existir).
        assert!(quest_available(&rules, &kv, "wolves"));
        assert!(!quest_available(&rules, &kv, "nao-existe"));
    }

    #[test]
    fn cadeia_segue_next_so_com_a_anterior_entregue() {
        let rules = regras();
        let mut kv = QuestKv::new();
        kv.insert("q.chain_a".into(), json_state(STATE_DONE));
        assert_eq!(quest_chain(&rules, &kv, "chain_a"), "chain_b");
        kv.insert("q.chain_b".into(), json_state(STATE_DONE));
        assert_eq!(quest_chain(&rules, &kv, "chain_a"), "chain_c");
        // c não tem `next`: a cadeia pára.
        kv.insert("q.chain_c".into(), json_state(STATE_DONE));
        assert_eq!(quest_chain(&rules, &kv, "chain_a"), "chain_c");
        // A primeira ainda não entregue: fica na mesma.
        kv.insert("q.chain_a".into(), json_state(STATE_ACTIVE));
        assert_eq!(quest_chain(&rules, &kv, "chain_a"), "chain_a");
        assert_eq!(quest_chain(&rules, &kv, "nao-existe"), "nao-existe");
    }

    #[test]
    fn accept_so_de_not_taken_e_disponivel() {
        let rules = regras();
        let mut kv = textos(&[
            ("q.pelt.v.serra", "lixo"),
            ("q.pelt.said", "lixo"),
            ("q.pelt.stage", "lixo"),
            ("q.outra.v.serra", "lixo"),
        ]);
        // `pelt` exige `wolves` entregue.
        assert!(!quest_accept(&rules, &mut kv, "pelt"));
        kv.insert("q.wolves".into(), json_state(STATE_DONE));
        assert!(quest_accept(&rules, &mut kv, "pelt"));
        assert_eq!(quest_state(&rules, &kv, "pelt"), STATE_ACTIVE);
        assert_eq!(kv.get("q.pelt.n"), Some(&json_int(0)));
        assert_eq!(kv.get("q.pelt.need"), Some(&json_int(3)));
        assert!(!kv.contains_key("q.pelt.stage"));
        assert!(!kv.contains_key("q.pelt.said"));
        assert!(!kv.contains_key("q.pelt.v.serra"));
        // A visita de OUTRA quest não é tocada.
        assert!(kv.contains_key("q.outra.v.serra"));
        // Uma segunda aceitação falha.
        assert!(!quest_accept(&rules, &mut kv, "pelt"));
    }

    #[test]
    fn set_progress_avanca_etapa_e_depois_fica_pronta() {
        let rules = regras();
        let mut kv = QuestKv::new();
        assert!(quest_accept(&rules, &mut kv, "staged"));
        kv.insert("q.staged.v.serra".into(), serde_json::Value::Bool(true));

        let out = quest_set_progress(&rules, &mut kv, "staged", 2.0);
        assert!(out.ok);
        assert!(out.stage_advanced);
        assert!(!out.ready);
        assert_eq!((out.stage, out.total), (1, 2));
        assert_eq!(out.title, "Etapas");
        assert_eq!(quest_state(&rules, &kv, "staged"), STATE_ACTIVE);
        assert_eq!(kv.get("q.staged.stage"), Some(&json_int(2)));
        assert_eq!(kv.get("q.staged.n"), Some(&json_int(0)));
        assert_eq!(kv.get("q.staged.need"), Some(&json_int(1)));
        assert!(!kv.contains_key("q.staged.v.serra"));

        let out = quest_set_progress(&rules, &mut kv, "staged", 1.0);
        assert!(out.ok);
        assert!(!out.stage_advanced);
        assert!(out.ready);
        assert_eq!(quest_state(&rules, &kv, "staged"), STATE_READY);

        // Fora de `active` não mexe.
        let out = quest_set_progress(&rules, &mut kv, "staged", 9.0);
        assert!(!out.ok);
        assert_eq!(quest_state(&rules, &kv, "staged"), STATE_READY);
        assert!(!quest_set_progress(&rules, &mut kv, "nao-existe", 1.0).ok);
    }

    #[test]
    fn set_progress_sem_etapas_promove_a_ready() {
        let rules = regras();
        let mut kv = QuestKv::new();
        quest_accept(&rules, &mut kv, "wolves");
        let out = quest_set_progress(&rules, &mut kv, "wolves", 3.0);
        assert!(out.ok);
        assert!(!out.stage_advanced);
        assert!(out.ready);
        assert_eq!(quest_state(&rules, &kv, "wolves"), STATE_READY);
        assert_eq!(kv.get("q.wolves.n"), Some(&json_int(3)));
    }

    #[test]
    fn entrega_paga_ouro_xp_itens_e_limpa_as_chaves() {
        let rules = regras();
        let vault = vault_rules();
        let xp = XpRules::default();
        let mut kv = textos(&[
            ("q.wolves", STATE_READY),
            ("q.wolves.n", "3"),
            ("q.wolves.stage", "1"),
            ("q.wolves.need", "3"),
            ("q.wolves.said", "lobo"),
            ("q.wolves.v.serra", "true"),
            ("tracked_quest", "wolves"),
        ]);
        kv.insert("item.gold".into(), json_int(10));

        let out = quest_turn_in(&rules, &vault, &xp, &mut kv, "wolves");
        assert!(out.ok, "entrega devia passar: {out:?}");
        assert_eq!(out.gold, 80);
        assert_eq!(out.missing_items, 0);
        assert_eq!(out.title, "Recompensa: lobos");
        assert_eq!(out.reason, None);

        assert_eq!(quest_state(&rules, &kv, "wolves"), STATE_DONE);
        assert_eq!(kv.get("item.gold"), Some(&json_int(90)));
        assert_eq!(kv.get("item.potion"), Some(&json_int(1)));
        assert_eq!(kv.get("xp"), Some(&json_int(50)));
        assert_eq!(kv.get("xp_next"), Some(&json_int(100)));
        assert_eq!(kv.get("level"), Some(&json_int(1)));
        assert!(!kv.contains_key("q.wolves.n"));
        assert!(!kv.contains_key("q.wolves.stage"));
        assert!(!kv.contains_key("q.wolves.need"));
        assert!(!kv.contains_key("q.wolves.said"));
        assert!(!kv.contains_key("q.wolves.v.serra"));
        assert!(!kv.contains_key("tracked_quest"));
    }

    #[test]
    fn entrega_so_de_ready() {
        let rules = regras();
        let vault = vault_rules();
        let xp = XpRules::default();
        let mut kv = textos(&[("q.wolves", STATE_ACTIVE)]);
        let out = quest_turn_in(&rules, &vault, &xp, &mut kv, "wolves");
        assert!(!out.ok);
        assert_eq!(out.reason, Some(TurnInReason::NotReady));
        assert_eq!(quest_state(&rules, &kv, "wolves"), STATE_ACTIVE);

        let out = quest_turn_in(&rules, &vault, &xp, &mut kv, "nao-existe");
        assert_eq!(out.reason, Some(TurnInReason::Unknown));
    }

    #[test]
    fn entrega_collect_sem_itens_volta_a_active() {
        let rules = regras();
        let vault = vault_rules();
        let xp = XpRules::default();
        let mut kv = textos(&[("q.pelt", STATE_READY), ("q.pelt.need", "3")]);
        kv.insert("item.wolf_pelt".into(), json_int(2));

        let out = quest_turn_in(&rules, &vault, &xp, &mut kv, "pelt");
        assert!(!out.ok);
        assert_eq!(out.reason, Some(TurnInReason::MissingItems));
        assert_eq!(quest_state(&rules, &kv, "pelt"), STATE_ACTIVE);
        // O cofre fica intacto e o `need` original (string incluída) também.
        assert_eq!(kv.get("item.wolf_pelt"), Some(&json_int(2)));
        assert_eq!(
            kv.get("q.pelt.need"),
            Some(&serde_json::Value::String("3".into()))
        );
    }

    #[test]
    fn entrega_collect_com_itens_desconta_do_cofre() {
        let rules = regras();
        let vault = vault_rules();
        let xp = XpRules::default();
        let mut kv = textos(&[("q.pelt", STATE_READY)]);
        kv.insert("q.wolves".into(), json_state(STATE_DONE));
        kv.insert("item.wolf_pelt".into(), json_int(4));

        let out = quest_turn_in(&rules, &vault, &xp, &mut kv, "pelt");
        assert!(out.ok, "{out:?}");
        assert_eq!(kv.get("item.wolf_pelt"), Some(&json_int(1)));
        assert_eq!(quest_state(&rules, &kv, "pelt"), STATE_DONE);
    }

    #[test]
    fn entrega_repetivel_volta_a_not_taken() {
        let rules = regras();
        let vault = vault_rules();
        let xp = XpRules::default();
        let mut kv = kv(&[
            ("q.board", json_state(STATE_READY)),
            ("tracked_quest", json_state("board")),
            ("item.gold", json_int(0)),
        ]);

        let out = quest_turn_in(&rules, &vault, &xp, &mut kv, "board");
        assert!(out.ok);
        assert_eq!(quest_state(&rules, &kv, "board"), STATE_NOT_TAKEN);
        assert_eq!(kv.get("item.gold"), Some(&json_int(5)));
        // Repetível não limpa `tracked_quest` (só a entrega final o faz).
        assert_eq!(kv.get("tracked_quest"), Some(&json_state("board")));
    }

    #[test]
    fn entrega_conta_itens_que_nao_cabem() {
        let rules = regras();
        let vault = vault_rules();
        let xp = XpRules::default();
        let mut kv = textos(&[("q.hoard", STATE_READY)]);
        // stack 5, tinha 4: entram 1 de 3, faltam 2.
        kv.insert("item.potion".into(), json_int(4));

        let out = quest_turn_in(&rules, &vault, &xp, &mut kv, "hoard");
        assert!(out.ok);
        assert_eq!(out.missing_items, 2);
        assert_eq!(kv.get("item.potion"), Some(&json_int(5)));
    }

    #[test]
    fn recompensas_aceitam_espacos_e_rejeitam_lixo() {
        assert_eq!(
            parse_item_reward(" potion : 3 "),
            Some(("potion".to_string(), 3))
        );
        assert_eq!(
            parse_item_reward("hunter_cloak"),
            Some(("hunter_cloak".to_string(), 1))
        );
        assert_eq!(
            parse_item_reward("potion:"),
            Some(("potion".to_string(), 1))
        );
        assert_eq!(
            parse_item_reward("potion:0"),
            Some(("potion".to_string(), 0))
        );
        assert_eq!(parse_item_reward("potion:x"), None);
        assert_eq!(parse_item_reward("a:b:3"), None);
        assert_eq!(parse_item_reward("  "), None);
    }

    fn lua_com_bridge() -> (Lua, Table) {
        let lua = Lua::new();
        lua.globals()
            .set("viber", lua.create_table().expect("tabela viber"))
            .expect("global viber");
        let game = lua.create_table().expect("tabela do jogo");
        lua.set_named_registry_value("viber_game", game.clone())
            .expect("registar viber_game");
        let vault = vault_rules();
        crate::vault::install(&lua, vault.clone()).expect("instalar cofre");
        crate::xp::install(&lua, XpRules::default()).expect("instalar xp");
        install(&lua, regras(), vault, XpRules::default()).expect("instalar quests");
        (lua, game)
    }

    #[test]
    fn bridge_le_e_escreve_o_estado_das_quests() {
        let (lua, game) = lua_com_bridge();
        game.raw_set("q.wolves", STATE_READY).expect("estado");
        game.raw_set("q.wolves.n", 3).expect("progresso");
        game.raw_set("q.wolves.need", 3).expect("meta");
        game.raw_set("item.gold", 0).expect("ouro");

        let (ok, gold, missing, title, reason): (bool, i64, i64, String, String) = lua
            .load(
                "local r = viber.vael.quest_turn_in('wolves') \
                 return r.ok, r.gold, r.missing_items, r.title, r.reason",
            )
            .eval()
            .expect("quest_turn_in");
        assert!(ok);
        assert_eq!(gold, 80);
        assert_eq!(missing, 0);
        assert_eq!(title, "Recompensa: lobos");
        assert_eq!(reason, "");
        assert_eq!(
            game.raw_get::<String>("q.wolves").expect("estado"),
            STATE_DONE
        );
        assert_eq!(game.raw_get::<i64>("item.gold").expect("ouro"), 80);

        // `pelt` exige `wolves` entregue (já está) e aceita.
        assert!(
            lua.load("return viber.vael.quest_accept('pelt')")
                .eval::<bool>()
                .expect("accept")
        );
        assert_eq!(
            game.raw_get::<String>("q.pelt").expect("estado"),
            STATE_ACTIVE
        );
        assert_eq!(game.raw_get::<i64>("q.pelt.n").expect("n"), 0);
        assert_eq!(game.raw_get::<i64>("q.pelt.need").expect("need"), 3);

        let (have, need): (i64, i64) = lua
            .load("local r = viber.vael.quest_progress('pelt'); return r.have, r.need")
            .eval()
            .expect("progress");
        assert_eq!((have, need), (0, 3));

        let (kind, at, total): (String, i64, i64) = lua
            .load(
                "local r = viber.vael.quest_objective('staged') \
                 return r.obj.type, r.at, r.total",
            )
            .eval()
            .expect("objective");
        assert_eq!((kind.as_str(), at, total), ("collect", 1, 2));

        assert!(
            lua.load("return viber.vael.quest_available('pelt')")
                .eval::<bool>()
                .expect("available")
        );
        assert_eq!(
            lua.load("return viber.vael.quest_state('wolves')")
                .eval::<String>()
                .expect("state"),
            STATE_DONE
        );
    }

    #[test]
    fn bridge_avanca_etapa_e_devolve_os_toasts() {
        let (lua, game) = lua_com_bridge();
        assert!(
            lua.load("return viber.vael.quest_accept('staged')")
                .eval::<bool>()
                .expect("accept")
        );
        game.raw_set("q.staged.v.serra", true).expect("visita");

        let (ok, advanced, stage, total, ready): (bool, bool, i64, i64, bool) = lua
            .load(
                "local r = viber.vael.quest_set_progress('staged', 2) \
                 return r.ok, r.stage_advanced, r.stage, r.total, r.ready",
            )
            .eval()
            .expect("set_progress");
        assert!(ok);
        assert!(advanced);
        assert_eq!((stage, total), (1, 2));
        assert!(!ready);
        assert_eq!(game.raw_get::<i64>("q.staged.stage").expect("stage"), 2);
        assert!(matches!(
            game.raw_get::<Value>("q.staged.v.serra").expect("visita"),
            Value::Nil
        ));

        let ready: bool = lua
            .load("return viber.vael.quest_set_progress('staged', 1).ready")
            .eval()
            .expect("pronta");
        assert!(ready);
        assert_eq!(
            game.raw_get::<String>("q.staged").expect("estado"),
            STATE_READY
        );
    }

    #[test]
    fn bridge_mantem_as_funcoes_existentes() {
        let lua = Lua::new();
        lua.globals()
            .set("viber", lua.create_table().expect("tabela viber"))
            .expect("global viber");
        let game = lua.create_table().expect("tabela do jogo");
        lua.set_named_registry_value("viber_game", game)
            .expect("registar viber_game");
        let vault = vault_rules();
        crate::vault::install(&lua, vault.clone()).expect("cofre");
        crate::xp::install(&lua, XpRules::default()).expect("xp");
        install(&lua, regras(), vault, XpRules::default()).expect("quests");

        let coexistem: bool = lua
            .load(
                "return type(viber.vael.vault_add) == 'function' \
                 and type(viber.vael.add_xp) == 'function' \
                 and type(viber.vael.quest_state) == 'function' \
                 and type(viber.vael.quest_turn_in) == 'function'",
            )
            .eval()
            .expect("funções");
        assert!(coexistem, "cofre, XP e quests partilham `viber.vael`");
    }

    #[test]
    fn plugin_sem_host_nao_panica() {
        let mut app = App::new();
        app.add_plugins(bevy::MinimalPlugins)
            .add_plugins(VaelQuestPlugin);
        for _ in 0..2 {
            app.update();
        }
    }
}

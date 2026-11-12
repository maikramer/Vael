//! Esquemas e validação do conteúdo estático do Vael.
//!
//! Os ficheiros JSON (`data/items.json`, `data/progression.json`,
//! `data/landmarks.json`, `quests/*.json`, `dialogues/*.json`) são lidos pelo
//! jogo via `viber.load_json` e editados pelo `vael-editor`. Este crate não
//! depende de Bevy: os tipos e as regras de validação são partilhados pelos
//! dois binários.
//!
//! Regras de serialização:
//! - campos opcionais ficam `Option` + `skip_serializing_if` para que uma
//!   chave ausente continue ausente depois de gravar;
//! - `#[serde(flatten)] extra` guarda campos que este crate ainda não conhece,
//!   para o editor não os apagar;
//! - `requires` aceita `"id"` ou `["id", …]` e volta a escrever a forma curta
//!   quando há um só id.

use std::collections::BTreeMap;

use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub mod validate;

pub use validate::{
    ContentSnapshot, Finding, QuestFile, Severity, WorldNames, validate_all, validate_dialogues,
    validate_items, validate_landmarks, validate_progression, validate_quests,
};

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_zero_i64(value: &i64) -> bool {
    *value == 0
}

fn is_zero_f64(value: &f64) -> bool {
    *value == 0.0
}

/// Escreve um `f64` como inteiro quando não tem parte fracionária. Os JSON do
/// Vael usam `8`, não `8.0`; sem isto cada gravação mudava números que o
/// utilizador não tocou.
fn serialize_f64<S>(value: &f64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serialize_f64_value(*value, serializer)
}

fn serialize_f64_value<S>(value: f64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if value.is_finite()
        && value.fract() == 0.0
        && value >= i64::MIN as f64
        && value <= i64::MAX as f64
    {
        serializer.serialize_i64(value as i64)
    } else {
        serializer.serialize_f64(value)
    }
}

struct CompactF64(f64);

impl Serialize for CompactF64 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize_f64_value(self.0, serializer)
    }
}

fn serialize_f64_map<S>(map: &BTreeMap<String, f64>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let mut state = serializer.serialize_map(Some(map.len()))?;
    for (key, value) in map {
        state.serialize_entry(key, &CompactF64(*value))?;
    }
    state.end()
}

// ── data/items.json ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct ItemsFile {
    pub version: u32,
    pub stack_max: i64,
    pub resource_cap: i64,
    pub starting: BTreeMap<String, i64>,
    pub items: Vec<ItemDef>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct ItemDef {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub order: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub desc: String,
    /// `weapon | armor | charm`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub slot: String,
    /// Teto da pilha próprio do item (`def.stack` do Lua); ausente = `stack_max`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shop: Option<ItemShop>,
    /// O JSON chama-lhe `use` (palavra reservada em Rust).
    #[serde(rename = "use", skip_serializing_if = "Option::is_none")]
    pub use_item: Option<ItemUse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weapon: Option<WeaponDef>,
    #[serde(
        skip_serializing_if = "BTreeMap::is_empty",
        serialize_with = "serialize_f64_map"
    )]
    pub effects: BTreeMap<String, f64>,
    /// Requisito de atributos para equipar (expressão do gauge, ex.
    /// `"Strength >= 7"`); vazio = sem gate.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub requires: String,
    /// Texto curto do requisito para o diário (`"Força 7"`).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub requires_text: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct ItemShop {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sell: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub buy: Option<i64>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct ItemUse {
    #[serde(skip_serializing_if = "is_zero_f64", serialize_with = "serialize_f64")]
    pub heal: f64,
    #[serde(skip_serializing_if = "is_false")]
    pub cure: bool,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct WeaponDef {
    #[serde(serialize_with = "serialize_f64")]
    pub damage: f64,
    #[serde(serialize_with = "serialize_f64")]
    pub speed: f64,
    #[serde(serialize_with = "serialize_f64")]
    pub range: f64,
    pub grip: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

// ── data/progression.json ───────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct ProgressionFile {
    pub version: u32,
    pub xp: XpCurve,
    /// XP por tipo de criatura; `default` é o piso.
    pub kill_xp: BTreeMap<String, i64>,
    pub song_seed_talent_every: i64,
    pub skills: Vec<SkillDef>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct XpCurve {
    pub base: i64,
    #[serde(serialize_with = "serialize_f64")]
    pub growth: f64,
    pub skill_points_per_level: i64,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct SkillDef {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requires: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excludes: Option<String>,
    #[serde(
        skip_serializing_if = "BTreeMap::is_empty",
        serialize_with = "serialize_f64_map"
    )]
    pub effects: BTreeMap<String, f64>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

// ── data/recipes.json ───────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct RecipesFile {
    pub version: u32,
    pub recipes: Vec<RecipeDef>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct RecipeDef {
    pub id: String,
    pub out: String,
    pub count: i64,
    pub cost: BTreeMap<String, i64>,
}

// ── data/landmarks.json ─────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct LandmarksFile {
    pub version: u32,
    #[serde(serialize_with = "serialize_f64")]
    pub campfire_range: f64,
    pub biome_labels: BTreeMap<String, String>,
    pub landmarks: Vec<LandmarkDef>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct LandmarkDef {
    pub name: String,
    pub biome: String,
    pub label: String,
    #[serde(serialize_with = "serialize_f64")]
    pub radius: f64,
    /// Quest de levantamento; ausente = marco sem survey.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub survey: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub pilgrim: bool,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

// ── data/riddles.json ───────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct RiddlesFile {
    pub version: u32,
    pub riddles: Vec<RiddleDef>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct RiddleDef {
    pub id: String,
    pub site: String,
    pub interact: String,
    pub clue: RiddleClue,
    pub prompt: String,
    pub example: String,
    pub expect: Vec<Expect>,
    pub all: bool,
    pub hint: String,
    pub solved: String,
    pub reward: Reward,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct RiddleClue {
    pub kind: String,
    pub sound: String,
    #[serde(skip_serializing_if = "is_zero_f64", serialize_with = "serialize_f64")]
    pub period: f64,
    #[serde(skip_serializing_if = "is_zero_f64", serialize_with = "serialize_f64")]
    pub radius: f64,
}

/// Um alvo do NLU: `kind` (criatura/item/local) + id.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, rename_all = "snake_case")]
pub struct Expect {
    pub kind: String,
    pub id: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

// ── data/verdicts.json ──────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct VerdictsFile {
    pub version: u32,
    pub rules: Vec<VerdictDef>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct VerdictDef {
    pub id: String,
    pub site: String,
    pub intro: String,
    pub require: Vec<Requirement>,
    pub order: Vec<String>,
    pub forbid: Vec<Requirement>,
    #[serde(serialize_with = "serialize_f64")]
    pub window: f64,
    pub verdicts: VerdictText,
    pub reward: Reward,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct Requirement {
    pub kind: String,
    pub id: String,
    pub verb: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct VerdictText {
    pub correct: String,
    pub partial: String,
    pub wrong: String,
}

/// Recompensa comum a quests, enigmas e veredictos (os campos que não se
/// aplicam ficam a zero).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct Reward {
    #[serde(skip_serializing_if = "is_zero_i64")]
    pub gold: i64,
    #[serde(skip_serializing_if = "is_zero_i64")]
    pub xp: i64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

// ── data/quest_puzzles/*.json ───────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct PuzzleDef {
    pub giver: String,
    pub intro: Vec<String>,
    pub ask: String,
    pub example: String,
    pub clues: Vec<PuzzleClue>,
    pub required: i64,
    pub no_answer: String,
    pub more: String,
    pub reveal: String,
    pub cancel: String,
    pub final_prompt: String,
    pub final_example: String,
    pub final_options: Vec<PuzzleOption>,
    pub final_answer: String,
    pub final_reply_ok: String,
    pub final_reply_bad: String,
    pub final_cancel: String,
    pub hint: String,
    pub endings: BTreeMap<String, PuzzleEnding>,
    pub after: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct PuzzleClue {
    pub key: String,
    pub description: String,
    pub reply: String,
    pub repeat: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct PuzzleOption {
    pub key: String,
    pub description: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct PuzzleEnding {
    pub text: String,
    pub items: Vec<String>,
}

// ── quests/*_quests.json ────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct QuestDef {
    pub id: String,
    pub npc: String,
    pub biome: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub portrait: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice: Option<String>,
    pub lines_intro: Vec<String>,
    pub lines_progress: Vec<String>,
    pub lines_complete: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub lines_locked: Vec<String>,
    pub objective: Objective,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub stages: Vec<Objective>,
    pub rewards: Reward,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parley: Option<Parley>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub next: String,
    /// `requires` é um id (string) ou uma lista de ids no JSON.
    #[serde(
        default,
        deserialize_with = "string_ou_lista",
        serialize_with = "lista_ou_string",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub requires: Vec<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// Objetivo de quest (e cada etapa de `stages`).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct Objective {
    /// O JSON chama-lhe `type` (palavra reservada em Rust).
    #[serde(rename = "type")]
    pub objective_type: String,
    pub target: String,
    #[serde(skip_serializing_if = "is_zero_i64")]
    pub count: i64,
    #[serde(skip_serializing_if = "is_zero_f64", serialize_with = "serialize_f64")]
    pub radius: f64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct Parley {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<Report>,
    pub partial: bool,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct Report {
    pub prompt: String,
    pub example: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub expect: Vec<Expect>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub hint: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// `requires: "a"` ou `requires: ["a", "b"]`.
fn string_ou_lista<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum UmOuMuitos {
        Um(String),
        Muitos(Vec<String>),
    }

    Ok(match Option::<UmOuMuitos>::deserialize(deserializer)? {
        None => Vec::new(),
        Some(UmOuMuitos::Um(id)) if id.is_empty() => Vec::new(),
        Some(UmOuMuitos::Um(id)) => vec![id],
        Some(UmOuMuitos::Muitos(ids)) => ids,
    })
}

fn lista_ou_string<S>(ids: &[String], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match ids {
        [one] => serializer.serialize_str(one),
        many => serializer.collect_seq(many),
    }
}

// ── dialogues/*.json ────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct DialogueFile {
    pub start: String,
    pub nodes: BTreeMap<String, DialogueNode>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct DialogueNode {
    pub speaker: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub portrait: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<DialogueChoice>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct DialogueChoice {
    pub text: String,
    pub goto: String,
    #[serde(skip_serializing_if = "is_false")]
    pub quest_accept: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub quest_turn_in: bool,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// `quests/index.json`: a lista de ficheiros de quests que o runtime lê.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(transparent)]
pub struct QuestIndex(pub Vec<String>);

impl QuestIndex {
    pub fn contains(&self, file: &str) -> bool {
        self.0.iter().any(|entry| entry == file)
    }

    /// Acrescenta o ficheiro se faltar e mantém a lista ordenada.
    pub fn insert_sorted(&mut self, file: &str) {
        if !self.contains(file) {
            self.0.push(file.to_string());
            self.0.sort();
        }
    }
}

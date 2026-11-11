//! Conteúdo do Vael: os JSON que o jogo já lê via `viber.load_json`.
//!
//! Os schemas são os ficheiros existentes — nenhum campo é renomeado nem
//! reescrito. Campos desconhecidos são ignorados (edições do editor e
//! conteúdo novo não bloqueiam o arranque) e um ficheiro ausente/inválido
//! regista `error!` e cai no default: o jogo arranca sempre.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use bevy::prelude::*;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde::de::Deserializer;

// ── data/items.json ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct ItemsFile {
    pub version: u32,
    pub stack_max: i64,
    pub resource_cap: i64,
    pub starting: BTreeMap<String, i64>,
    pub items: Vec<ItemDef>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct ItemDef {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub order: i64,
    pub desc: String,
    /// `weapon | armor | charm`.
    pub slot: String,
    /// Teto da pilha próprio do item (`def.stack` do Lua); ausente = `stack_max`.
    pub stack: Option<i64>,
    pub shop: Option<ItemShop>,
    /// O JSON chama-lhe `use` (palavra reservada em Rust).
    #[serde(rename = "use")]
    pub use_item: Option<ItemUse>,
    pub weapon: Option<WeaponDef>,
    pub effects: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct ItemShop {
    pub sell: Option<i64>,
    pub buy: Option<i64>,
    pub label: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct ItemUse {
    pub heal: f64,
    pub cure: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct WeaponDef {
    pub damage: f64,
    pub speed: f64,
    pub range: f64,
    pub grip: String,
}

// ── data/progression.json ───────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct ProgressionFile {
    pub version: u32,
    pub xp: XpCurve,
    /// XP por tipo de criatura; `default` é o piso.
    pub kill_xp: BTreeMap<String, i64>,
    pub song_seed_talent_every: i64,
    pub skills: Vec<SkillDef>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct XpCurve {
    pub base: i64,
    pub growth: f64,
    pub skill_points_per_level: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct SkillDef {
    pub id: String,
    pub label: String,
    pub requires: String,
    pub excludes: String,
    pub effects: BTreeMap<String, f64>,
}

// ── data/recipes.json ───────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct RecipesFile {
    pub version: u32,
    pub recipes: Vec<RecipeDef>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct RecipeDef {
    pub id: String,
    pub out: String,
    pub count: i64,
    pub cost: BTreeMap<String, i64>,
}

// ── data/landmarks.json ─────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct LandmarksFile {
    pub version: u32,
    pub campfire_range: f64,
    pub biome_labels: BTreeMap<String, String>,
    pub landmarks: Vec<LandmarkDef>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct LandmarkDef {
    pub name: String,
    pub biome: String,
    pub label: String,
    pub radius: f64,
    pub survey: String,
    pub pilgrim: bool,
}

// ── data/riddles.json ───────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct RiddlesFile {
    pub version: u32,
    pub riddles: Vec<RiddleDef>,
}

#[derive(Debug, Clone, Default, Deserialize)]
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

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct RiddleClue {
    pub kind: String,
    pub sound: String,
    pub period: f64,
    pub radius: f64,
}

/// Um alvo do NLU: `kind` (criatura/item/local) + id.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "snake_case")]
pub struct Expect {
    pub kind: String,
    pub id: String,
}

// ── data/verdicts.json ──────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct VerdictsFile {
    pub version: u32,
    pub rules: Vec<VerdictDef>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct VerdictDef {
    pub id: String,
    pub site: String,
    pub intro: String,
    pub require: Vec<Requirement>,
    pub order: Vec<String>,
    pub forbid: Vec<Requirement>,
    pub window: f64,
    pub verdicts: VerdictText,
    pub reward: Reward,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct Requirement {
    pub kind: String,
    pub id: String,
    pub verb: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct VerdictText {
    pub correct: String,
    pub partial: String,
    pub wrong: String,
}

/// Recompensa comum a quests, enigmas e veredictos (os campos que não se
/// aplicam ficam a zero).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct Reward {
    pub gold: i64,
    pub xp: i64,
    pub items: Vec<String>,
}

// ── data/quest_puzzles/*.json ───────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize)]
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

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct PuzzleClue {
    pub key: String,
    pub description: String,
    pub reply: String,
    pub repeat: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct PuzzleOption {
    pub key: String,
    pub description: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct PuzzleEnding {
    pub text: String,
    pub items: Vec<String>,
}

// ── quests/*_quests.json ────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct QuestDef {
    pub id: String,
    pub npc: String,
    pub biome: String,
    pub title: String,
    pub portrait: String,
    pub voice: String,
    pub lines_intro: Vec<String>,
    pub lines_progress: Vec<String>,
    pub lines_complete: Vec<String>,
    pub lines_locked: Vec<String>,
    pub objective: Objective,
    pub stages: Vec<Objective>,
    pub rewards: Reward,
    pub parley: Option<Parley>,
    pub next: String,
    /// `requires` é um id (string) ou uma lista de ids no JSON.
    #[serde(default, deserialize_with = "string_ou_lista")]
    pub requires: Vec<String>,
}

/// Objetivo de quest (e cada etapa de `stages`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct Objective {
    /// O JSON chama-lhe `type` (palavra reservada em Rust).
    #[serde(rename = "type")]
    pub objective_type: String,
    pub target: String,
    pub count: i64,
    pub radius: f64,
    pub title: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct Parley {
    pub report: Option<Report>,
    pub partial: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct Report {
    pub prompt: String,
    pub example: String,
    pub expect: Vec<Expect>,
    pub hint: String,
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

// ── dialogues/*.json ────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct DialogueFile {
    pub start: String,
    pub nodes: BTreeMap<String, DialogueNode>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct DialogueNode {
    pub speaker: String,
    pub text: String,
    pub choices: Vec<DialogueChoice>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct DialogueChoice {
    pub text: String,
    pub goto: String,
}

// ── catálogo carregado ──────────────────────────────────────────────────

/// Todo o conteúdo estático do Vael, carregado uma vez no arranque.
#[derive(Resource, Debug, Clone, Default)]
pub struct VaelContent {
    pub items: ItemsFile,
    pub progression: ProgressionFile,
    pub recipes: RecipesFile,
    pub landmarks: LandmarksFile,
    pub riddles: RiddlesFile,
    pub verdicts: VerdictsFile,
    /// Chave = nome do ficheiro sem `.json`.
    pub puzzles: BTreeMap<String, PuzzleDef>,
    /// Chave = `id` da quest.
    pub quests: BTreeMap<String, QuestDef>,
    /// Chave = nome do ficheiro sem `.json` (`chapel_priest`, …).
    pub dialogues: BTreeMap<String, DialogueFile>,
    /// `pt`/`en` → chave → texto.
    pub locale: BTreeMap<String, BTreeMap<String, String>>,
    /// Ficheiros que falharam (o jogo arranca na mesma).
    pub errors: Vec<String>,
}

impl VaelContent {
    /// Lê tudo a partir da raiz do projeto. Nunca entra em pânico.
    pub fn load(root: &Path) -> Self {
        let mut errors = Vec::new();
        let mut content = Self::default();

        content.items = load_required(root, "data/items.json", &mut errors);
        content.progression = load_required(root, "data/progression.json", &mut errors);
        content.recipes = load_required(root, "data/recipes.json", &mut errors);
        content.landmarks = load_required(root, "data/landmarks.json", &mut errors);
        content.riddles = load_required(root, "data/riddles.json", &mut errors);
        content.verdicts = load_required(root, "data/verdicts.json", &mut errors);

        content.puzzles = load_json_dir(root, "data/quest_puzzles", &mut errors);
        content.quests = load_quests(root, &mut errors);
        content.dialogues = load_json_dir(root, "dialogues", &mut errors);

        for lang in ["pt", "en"] {
            let rel = format!("locale/{lang}.json");
            match read_json::<BTreeMap<String, String>>(root, &rel) {
                Ok(strings) => {
                    content.locale.insert(lang.to_string(), strings);
                }
                Err(_) if !root.join(&rel).exists() => {
                    warn!("vael: {rel} ausente (opcional)");
                }
                Err(err) => {
                    error!("vael: conteúdo inválido: {err}");
                    errors.push(err);
                }
            }
        }

        content.errors = errors;
        content
    }
}

/// Plugin de conteúdo: carrega antes de qualquer `Startup` do jogo.
pub struct VaelContentPlugin;

impl Plugin for VaelContentPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VaelContent>()
            .add_systems(PreStartup, load_content);
    }
}

fn load_content(root: Res<crate::VaelProjectRoot>, mut content: ResMut<VaelContent>) {
    if root.0.as_os_str().is_empty() {
        error!("vael: VaelProjectRoot vazio — conteúdo não carregado");
        return;
    }
    info!("vael: conteúdo de {}", root.0.display());
    *content = VaelContent::load(&root.0);
    if !content.errors.is_empty() {
        error!(
            "vael: {} ficheiro(s) de conteúdo falharam — o jogo arranca com defaults",
            content.errors.len()
        );
    }
}

// ── loaders ─────────────────────────────────────────────────────────────

fn read_json<T: DeserializeOwned>(root: &Path, rel: &str) -> Result<T, String> {
    let path = root.join(rel);
    let text = std::fs::read_to_string(&path).map_err(|err| format!("{rel}: {err}"))?;
    serde_json::from_str(&text).map_err(|err| format!("{rel}: {err}"))
}

fn load_required<T: DeserializeOwned + Default>(
    root: &Path,
    rel: &str,
    errors: &mut Vec<String>,
) -> T {
    match read_json(root, rel) {
        Ok(value) => value,
        Err(err) => {
            error!("vael: conteúdo: {err}");
            errors.push(err);
            T::default()
        }
    }
}

/// Lê todos os `*.json` de uma pasta, chaveados pelo nome do ficheiro.
fn load_json_dir<T: DeserializeOwned>(
    root: &Path,
    rel: &str,
    errors: &mut Vec<String>,
) -> BTreeMap<String, T> {
    let dir = root.join(rel);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        let err = format!("{rel}: pasta ausente ou ilegível");
        error!("vael: conteúdo: {err}");
        errors.push(err);
        return BTreeMap::new();
    };

    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        })
        .collect();
    paths.sort();

    let mut out = BTreeMap::new();
    for path in paths {
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let rel_file = format!("{rel}/{name}");
        match read_json::<T>(root, &rel_file) {
            Ok(value) => {
                out.insert(stem.to_string(), value);
            }
            Err(err) => {
                error!("vael: conteúdo inválido: {err}");
                errors.push(err);
            }
        }
    }
    out
}

/// `quests/index.json` manda; sem ele, varre `quests/*_quests.json`.
fn load_quests(root: &Path, errors: &mut Vec<String>) -> BTreeMap<String, QuestDef> {
    let index = match read_json::<Vec<String>>(root, "quests/index.json") {
        Ok(list) => Some(list),
        Err(err) => {
            warn!("vael: quests/index.json indisponível ({err}); a varrer quests/*_quests.json");
            None
        }
    };
    let files = match index {
        Some(list) if !list.is_empty() => list,
        _ => scan_quest_files(root, errors),
    };

    let mut out = BTreeMap::new();
    for name in files {
        if name.trim().is_empty() {
            continue;
        }
        let rel = format!("quests/{name}");
        match read_json::<Vec<QuestDef>>(root, &rel) {
            Ok(quests) => {
                for quest in quests {
                    if quest.id.is_empty() {
                        warn!("vael: quest sem id em {rel} — ignorada");
                        continue;
                    }
                    if out.contains_key(&quest.id) {
                        warn!(
                            "vael: quest `{}` repetida em {rel} — a última ganha",
                            quest.id
                        );
                    }
                    out.insert(quest.id.clone(), quest);
                }
            }
            Err(err) => {
                error!("vael: conteúdo inválido: {err}");
                errors.push(err);
            }
        }
    }
    out
}

fn scan_quest_files(root: &Path, errors: &mut Vec<String>) -> Vec<String> {
    let dir = root.join("quests");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        let err = "quests: pasta ausente ou ilegível".to_string();
        error!("vael: conteúdo: {err}");
        errors.push(err);
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
        .filter(|name| name.ends_with("_quests.json"))
        .collect();
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn carrega_todo_o_conteudo_do_projeto() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let content = VaelContent::load(root);

        assert!(
            content.errors.is_empty(),
            "ficheiros com erro: {:?}",
            content.errors
        );
        assert!(!content.items.items.is_empty(), "items vazio");
        assert!(!content.progression.skills.is_empty(), "skills vazio");
        assert!(!content.recipes.recipes.is_empty(), "recipes vazio");
        assert!(!content.landmarks.landmarks.is_empty(), "landmarks vazio");
        assert!(!content.riddles.riddles.is_empty(), "riddles vazio");
        assert!(!content.verdicts.rules.is_empty(), "verdicts vazio");
        assert!(!content.puzzles.is_empty(), "puzzles vazio");
        assert!(!content.quests.is_empty(), "quests vazio");
        assert!(!content.dialogues.is_empty(), "dialogues vazio");
        assert!(content.locale.contains_key("pt"), "locale pt ausente");
        assert!(content.locale.contains_key("en"), "locale en ausente");
    }
}

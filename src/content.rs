//! Conteúdo do Vael: os JSON que o jogo já lê via `viber.load_json`.
//!
//! Os schemas são os ficheiros existentes — nenhum campo é renomeado nem
//! reescrito. Campos desconhecidos são ignorados (edições do editor e
//! conteúdo novo não bloqueiam o arranque) e um ficheiro ausente/inválido
//! regista `error!` e cai no default: o jogo arranca sempre.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use bevy::prelude::*;
use serde::de::DeserializeOwned;

pub use vael_content::{
    DialogueChoice, DialogueFile, DialogueNode, Expect, ItemDef, ItemShop, ItemUse, ItemsFile,
    LandmarkDef, LandmarksFile, Objective, Parley, ProgressionFile, PuzzleClue, PuzzleDef,
    PuzzleEnding, PuzzleOption, QuestDef, QuestIndex, RecipeDef, RecipesFile, Report, Reward,
    RiddleClue, RiddleDef, RiddlesFile, SkillDef, VerdictDef, VerdictText, VerdictsFile, WeaponDef,
    XpCurve,
};

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

//! Estado do editor: raiz do projeto, conteúdo carregado e problemas.
//!
//! O editor não usa o ECS do jogo. Carrega os mesmos JSON que o runtime lê,
//! valida-os com o crate `vael-content` e grava-os de volta com
//! [`crate::io::write_json_atomic`].

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use bevy::prelude::*;
use vael_content::{
    ContentSnapshot, DialogueFile, Finding, ItemsFile, LandmarksFile, ProgressionFile, QuestFile,
    QuestIndex, Severity, validate_all,
};

use crate::io;
use crate::world_index::WorldIndex;

/// Raiz do projeto Vael (`world.bsn`, `data/`, `quests/`, `dialogues/`).
#[derive(Resource, Clone, Debug)]
pub struct VaelRoot(pub PathBuf);

/// Todo o conteúdo editável, carregado do disco.
#[derive(Resource, Default, Clone)]
pub struct VaelState {
    pub items: ItemsFile,
    pub progression: ProgressionFile,
    pub landmarks: LandmarksFile,
    pub quest_files: Vec<QuestFile>,
    pub quest_index: QuestIndex,
    pub dialogues: BTreeMap<String, DialogueFile>,
    pub locale: BTreeMap<String, BTreeMap<String, String>>,
    pub world: WorldIndex,
    /// Ficheiros que falharam ao carregar (o editor abre na mesma).
    pub errors: Vec<String>,
    /// Caminhos relativos que falharam a carregar. Gravar um destes alvos
    /// escreveria os defaults por cima do ficheiro: o editor recusa.
    pub failed: BTreeSet<String>,
    pub loaded: bool,
}

impl VaelState {
    /// Lê tudo a partir da raiz do projeto. Nunca entra em pânico.
    pub fn load(root: &Path) -> Self {
        let mut errors = Vec::new();
        let mut failed = BTreeSet::new();
        let items = load_required(root, "data/items.json", &mut errors, &mut failed);
        let progression = load_required(root, "data/progression.json", &mut errors, &mut failed);
        let landmarks = load_required(root, "data/landmarks.json", &mut errors, &mut failed);

        let quest_index: QuestIndex =
            load_required(root, "quests/index.json", &mut errors, &mut failed);
        let mut quest_files = Vec::new();
        match io::list_json_files(&root.join("quests")) {
            Ok(names) => {
                for file in names {
                    let rel = format!("quests/{file}");
                    match io::read_json::<Vec<vael_content::QuestDef>>(&root.join(&rel)) {
                        Ok(quests) => quest_files.push(QuestFile::new(file, quests)),
                        Err(err) => {
                            errors.push(err);
                            failed.insert(rel);
                        }
                    }
                }
            }
            Err(err) => errors.push(err),
        }

        let mut dialogues = BTreeMap::new();
        match io::list_json_files(&root.join("dialogues")) {
            Ok(names) => {
                for file in names {
                    let rel = format!("dialogues/{file}");
                    match io::read_json::<DialogueFile>(&root.join(&rel)) {
                        Ok(dialogue) => {
                            let stem = file.trim_end_matches(".json").to_string();
                            dialogues.insert(stem, dialogue);
                        }
                        Err(err) => {
                            errors.push(err);
                            failed.insert(rel);
                        }
                    }
                }
            }
            Err(err) => errors.push(err),
        }

        let mut locale = BTreeMap::new();
        for lang in ["pt", "en"] {
            let rel = format!("locale/{lang}.json");
            let path = root.join(&rel);
            if path.is_file() {
                match io::read_json::<BTreeMap<String, String>>(&path) {
                    Ok(strings) => {
                        locale.insert(lang.to_string(), strings);
                    }
                    Err(err) => errors.push(err),
                }
            }
        }

        let world = WorldIndex::load(root);
        Self {
            items,
            progression,
            landmarks,
            quest_files,
            quest_index,
            dialogues,
            locale,
            world,
            errors,
            failed,
            loaded: true,
        }
    }

    pub fn reload(&mut self, root: &Path) {
        *self = Self::load(root);
    }

    /// Todas as regras de validação sobre o estado atual.
    pub fn findings(&self) -> Vec<Finding> {
        let snapshot = ContentSnapshot {
            items: &self.items,
            progression: &self.progression,
            landmarks: &self.landmarks,
            quest_files: &self.quest_files,
            dialogues: &self.dialogues,
            locale: &self.locale,
            quest_index: &self.quest_index,
            world: &self.world.names,
        };
        validate_all(&snapshot)
    }

    pub fn findings_for(&self, kind: &str) -> Vec<Finding> {
        self.findings()
            .into_iter()
            .filter(|finding| finding.kind == kind)
            .collect()
    }

    pub fn errors_of(&self, severity: Severity) -> usize {
        self.findings()
            .iter()
            .filter(|finding| finding.severity == severity)
            .count()
    }

    pub fn quest(&self, id: &str) -> Option<&vael_content::QuestDef> {
        self.quest_files
            .iter()
            .flat_map(|file| file.quests.iter())
            .find(|quest| quest.id == id)
    }

    pub fn quest_mut(&mut self, id: &str) -> Option<&mut vael_content::QuestDef> {
        self.quest_files
            .iter_mut()
            .flat_map(|file| file.quests.iter_mut())
            .find(|quest| quest.id == id)
    }

    /// Ficheiro de quests que contém `id`, se existir.
    pub fn quest_file_of(&self, id: &str) -> Option<usize> {
        self.quest_files
            .iter()
            .position(|file| file.quests.iter().any(|quest| quest.id == id))
    }
}

fn load_required<T: serde::de::DeserializeOwned + Default>(
    root: &Path,
    rel: &str,
    errors: &mut Vec<String>,
    failed: &mut BTreeSet<String>,
) -> T {
    match io::read_json(&root.join(rel)) {
        Ok(value) => value,
        Err(err) => {
            errors.push(err);
            failed.insert(rel.to_string());
            T::default()
        }
    }
}

//! Regras de validação cruzada do conteúdo do Vael.
//!
//! As regras vêm dos antigos plugins do editor (`quests.lua`,
//! `dialogues.lua`, `content_items.lua`, `content_landmarks.lua`,
//! `content_progression.lua` e `content_lint.lua`). O editor usa-as nos
//! painéis e no painel "Conteúdo"; o jogo não as corre no arranque.
//!
//! `Warning` nunca impede gravar; `Error` deve impedir. A validação não vê o
//! ECS: os nomes de entidades vêm de um [`WorldNames`] construído a partir do
//! `world.xml`.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    DialogueFile, ItemsFile, LandmarkDef, LandmarksFile, Objective, ProgressionFile, QuestDef,
    QuestIndex,
};

/// Biomas aceites por quests (o `biome` do mapa/história).
pub const QUEST_BIOMES: &[&str] = &["city", "dark_forest", "desert", "swamp", "frozen_peaks"];
/// Biomas aceites por marcos (inclui a atalaia e o Alto de Vael).
pub const LANDMARK_BIOMES: &[&str] = &[
    "tower",
    "dark_forest",
    "desert",
    "swamp",
    "frozen_peaks",
    "alto",
];
/// Tipos de objetivo que o runtime interpreta. `craft` existe no Lua e nos
/// dados; o plugin antigo não o aceitava.
pub const OBJECTIVE_TYPES: &[&str] = &["kill", "collect", "visit", "decision", "craft"];
/// Tipos de alvo do NLU (parley).
pub const EXPECT_KINDS: &[&str] = &["criatura", "item", "local"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Self::Error => "erro",
            Self::Warning => "aviso",
        }
    }
}

/// Um problema encontrado no conteúdo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub severity: Severity,
    pub kind: &'static str,
    pub message: String,
}

impl Finding {
    pub fn error(kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            kind,
            message: message.into(),
        }
    }

    pub fn warning(kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            kind,
            message: message.into(),
        }
    }
}

/// Nomes de entidades do mundo estático (`world.xml` + `<Include>`).
///
/// O editor constrói-o sem carregar o ECS; os testes podem construí-lo à mão.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorldNames {
    names: BTreeSet<String>,
}

impl WorldNames {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, name: impl Into<String>) {
        let name = name.into();
        if !name.is_empty() {
            self.names.insert(name);
        }
    }

    pub fn contains(&self, name: &str) -> bool {
        self.names.contains(name)
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &String> {
        self.names.iter()
    }
}

impl FromIterator<String> for WorldNames {
    fn from_iter<T: IntoIterator<Item = String>>(iter: T) -> Self {
        let mut names = Self::new();
        for name in iter {
            names.insert(name);
        }
        names
    }
}

/// Um ficheiro de `quests/` com o nome e as quests que contém.
#[derive(Debug, Clone)]
pub struct QuestFile {
    pub file: String,
    pub quests: Vec<QuestDef>,
}

impl QuestFile {
    pub fn new(file: impl Into<String>, quests: Vec<QuestDef>) -> Self {
        Self {
            file: file.into(),
            quests,
        }
    }
}

/// Tudo o que a validação cruzada precisa. Os painéis preenchem-no a partir
/// dos ficheiros carregados; nenhum campo é copiado.
pub struct ContentSnapshot<'a> {
    pub items: &'a ItemsFile,
    pub progression: &'a ProgressionFile,
    pub landmarks: &'a LandmarksFile,
    pub quest_files: &'a [QuestFile],
    pub dialogues: &'a BTreeMap<String, DialogueFile>,
    pub locale: &'a BTreeMap<String, BTreeMap<String, String>>,
    pub quest_index: &'a QuestIndex,
    pub world: &'a WorldNames,
}

/// Corre todas as regras e devolve os problemas na ordem dos ficheiros.
pub fn validate_all(snapshot: &ContentSnapshot<'_>) -> Vec<Finding> {
    let quest_ids = quest_ids(snapshot.quest_files);
    let landmark_names = landmark_names(snapshot.landmarks);
    let item_ids: BTreeSet<&str> = snapshot
        .items
        .items
        .iter()
        .map(|item| item.id.as_str())
        .filter(|id| !id.is_empty())
        .collect();

    let mut findings = Vec::new();
    validate_index(snapshot.quest_files, snapshot.quest_index, &mut findings);
    findings.extend(validate_items(snapshot.items));
    findings.extend(validate_progression(snapshot.progression));
    findings.extend(validate_landmarks(
        snapshot.landmarks,
        snapshot.world,
        &quest_ids,
    ));
    findings.extend(validate_quests(
        snapshot.quest_files,
        snapshot.world,
        &item_ids,
        &landmark_names,
        &quest_ids,
    ));
    findings.extend(validate_dialogues(
        snapshot.dialogues,
        &quest_ids,
        snapshot.locale,
    ));
    findings
}

fn validate_index(files: &[QuestFile], index: &QuestIndex, out: &mut Vec<Finding>) {
    for file in files {
        if !index.contains(&file.file) {
            out.push(Finding::error(
                "quest-index",
                format!(
                    "{}: não está em quests/index.json — o runtime não lê este ficheiro",
                    file.file
                ),
            ));
        }
    }
    for entry in &index.0 {
        if entry.trim().is_empty() {
            out.push(Finding::error(
                "quest-index",
                "quests/index.json: entrada vazia".to_string(),
            ));
        } else if !files.iter().any(|file| &file.file == entry) {
            out.push(Finding::warning(
                "quest-index",
                format!("{entry}: listado em quests/index.json mas não existe em quests/"),
            ));
        }
    }
}

pub fn validate_items(file: &ItemsFile) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for item in &file.items {
        if item.id.is_empty() {
            out.push(Finding::error("items", "item com id vazio"));
        } else if !seen.insert(item.id.as_str()) {
            out.push(Finding::error(
                "items",
                format!("{}: id de item repetido", item.id),
            ));
        }
        if item.label.is_empty() {
            out.push(Finding::warning(
                "items",
                format!("{}: item sem label", item.id),
            ));
        }
        if let Some(shop) = &item.shop
            && (shop.buy.is_some_and(|price| price < 0) || shop.sell.is_some_and(|price| price < 0))
        {
            out.push(Finding::error(
                "items",
                format!("{}: preço de loja negativo", item.id),
            ));
        }
    }
    out
}

pub fn validate_progression(file: &ProgressionFile) -> Vec<Finding> {
    let mut out = Vec::new();
    if file.xp.base < 1 {
        out.push(Finding::error(
            "progression",
            format!("xp.base tem de ser >= 1 (é {})", file.xp.base),
        ));
    }
    if file.xp.growth <= 1.0 {
        out.push(Finding::error(
            "progression",
            format!("xp.growth tem de ser > 1 (é {})", file.xp.growth),
        ));
    }
    for (creature, xp) in &file.kill_xp {
        if *xp < 0 {
            out.push(Finding::error(
                "progression",
                format!("kill_xp.{creature}: XP negativo ({xp})"),
            ));
        }
    }

    let mut seen = BTreeSet::new();
    for skill in &file.skills {
        if skill.id.is_empty() {
            out.push(Finding::error("progression", "talento com id vazio"));
        } else if !seen.insert(skill.id.as_str()) {
            out.push(Finding::error(
                "progression",
                format!("{}: id de talento repetido", skill.id),
            ));
        }
        if skill.label.is_empty() {
            out.push(Finding::warning(
                "progression",
                format!("{}: talento sem label", skill.id),
            ));
        }
        if let Some(requires) = skill.requires.as_deref().filter(|id| !id.is_empty())
            && !file.skills.iter().any(|other| other.id == requires)
        {
            out.push(Finding::warning(
                "progression",
                format!("{}: requires `{requires}` não existe", skill.id),
            ));
        }
    }
    out
}

pub fn validate_landmarks(
    file: &LandmarksFile,
    world: &WorldNames,
    quest_ids: &BTreeSet<String>,
) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for landmark in &file.landmarks {
        validate_landmark(landmark, world, quest_ids, &mut seen, &mut out);
    }
    out
}

fn validate_landmark(
    landmark: &LandmarkDef,
    world: &WorldNames,
    quest_ids: &BTreeSet<String>,
    seen: &mut BTreeSet<String>,
    out: &mut Vec<Finding>,
) {
    if landmark.name.is_empty() {
        out.push(Finding::error("landmarks", "marco com name vazio"));
    } else if !seen.insert(landmark.name.clone()) {
        out.push(Finding::error(
            "landmarks",
            format!("{}: name de marco repetido", landmark.name),
        ));
    }
    if landmark.label.is_empty() {
        out.push(Finding::warning(
            "landmarks",
            format!("{}: marco sem label", landmark.name),
        ));
    }
    if !(landmark.radius.is_finite() && landmark.radius > 0.0) {
        out.push(Finding::error(
            "landmarks",
            format!("{}: radius tem de ser > 0", landmark.name),
        ));
    }
    if !LANDMARK_BIOMES.contains(&landmark.biome.as_str()) {
        out.push(Finding::warning(
            "landmarks",
            format!("{}: bioma `{}` desconhecido", landmark.name, landmark.biome),
        ));
    }
    if !landmark.name.is_empty() && !world.contains(&landmark.name) {
        out.push(Finding::error(
            "landmarks",
            format!(
                "{}: não existe nenhuma entidade com este nome",
                landmark.name
            ),
        ));
    }
    if let Some(survey) = landmark.survey.as_deref().filter(|id| !id.is_empty())
        && !quest_ids.contains(survey)
    {
        out.push(Finding::warning(
            "landmarks",
            format!("{}: survey `{survey}` não é uma quest", landmark.name),
        ));
    }
}

pub fn validate_quests(
    files: &[QuestFile],
    world: &WorldNames,
    item_ids: &BTreeSet<&str>,
    landmark_names: &BTreeSet<&str>,
    quest_ids: &BTreeSet<String>,
) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for file in files {
        for quest in &file.quests {
            if !quest.id.is_empty() {
                *counts.entry(quest.id.as_str()).or_default() += 1;
            }
        }
    }

    for file in files {
        for quest in &file.quests {
            validate_quest(
                &file.file,
                quest,
                &counts,
                world,
                item_ids,
                landmark_names,
                quest_ids,
                &mut out,
            );
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn validate_quest(
    file: &str,
    quest: &QuestDef,
    counts: &BTreeMap<&str, usize>,
    world: &WorldNames,
    item_ids: &BTreeSet<&str>,
    landmark_names: &BTreeSet<&str>,
    quest_ids: &BTreeSet<String>,
    out: &mut Vec<Finding>,
) {
    let at = |problem: &str| format!("{file}: {}: {problem}", quest.id);
    if quest.id.is_empty() {
        out.push(Finding::error("quest-ref", format!("{file}: quest sem id")));
    } else if !quest
        .id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
    {
        out.push(Finding::error(
            "quest-ref",
            at("id só pode ter letras, números, `_`, `-` e `.`"),
        ));
    }
    if counts.get(quest.id.as_str()).copied().unwrap_or(0) > 1 {
        out.push(Finding::error(
            "quest-ref",
            at("id de quest repetido (a última definição ganha no runtime)"),
        ));
    }
    if quest.title.is_empty() {
        out.push(Finding::error("quest-ref", at("sem title")));
    }
    if !QUEST_BIOMES.contains(&quest.biome.as_str()) {
        out.push(Finding::error(
            "quest-ref",
            at(&format!("bioma `{}` desconhecido", quest.biome)),
        ));
    }
    if quest.npc.is_empty() {
        out.push(Finding::error("quest-ref", at("sem npc")));
    } else if quest.npc != "notice_board" && !world.contains(&quest.npc) {
        out.push(Finding::error(
            "quest-ref",
            at(&format!("npc `{}` não existe no mundo", quest.npc)),
        ));
    }

    validate_objective(&quest.objective, &at, item_ids, landmark_names, out);
    for (index, stage) in quest.stages.iter().enumerate() {
        validate_objective(
            stage,
            &|problem| at(&format!("etapa {}: {problem}", index + 1)),
            item_ids,
            landmark_names,
            out,
        );
    }

    if quest.lines_intro.is_empty() {
        out.push(Finding::warning("quest-ref", at("lines_intro vazio")));
    }
    if quest.lines_complete.is_empty() {
        out.push(Finding::warning("quest-ref", at("lines_complete vazio")));
    }

    validate_rewards(quest, &at, item_ids, out);
    validate_parley(quest, &at, item_ids, landmark_names, out);

    for required in &quest.requires {
        if !quest_ids.contains(required) {
            out.push(Finding::warning(
                "quest-ref",
                at(&format!("requires `{required}` não é uma quest")),
            ));
        }
    }
    if !quest.next.is_empty() {
        if quest.next == quest.id {
            out.push(Finding::warning(
                "quest-ref",
                at("next aponta para a própria quest"),
            ));
        } else if !quest_ids.contains(&quest.next) {
            out.push(Finding::warning(
                "quest-ref",
                at(&format!("next `{}` não é uma quest", quest.next)),
            ));
        }
    }
}

fn validate_objective(
    objective: &Objective,
    at: &impl Fn(&str) -> String,
    item_ids: &BTreeSet<&str>,
    landmark_names: &BTreeSet<&str>,
    out: &mut Vec<Finding>,
) {
    if !OBJECTIVE_TYPES.contains(&objective.objective_type.as_str()) {
        out.push(Finding::error(
            "quest-ref",
            at(&format!(
                "objetivo `{}` desconhecido (aceites: {})",
                objective.objective_type,
                OBJECTIVE_TYPES.join(", ")
            )),
        ));
    }
    if objective.target.is_empty() {
        out.push(Finding::error("quest-ref", at("objetivo sem target")));
    }
    if objective.count < 1 {
        out.push(Finding::warning(
            "quest-ref",
            at("objetivo com count < 1 (o runtime trata como 1)"),
        ));
    }

    match objective.objective_type.as_str() {
        "visit" => {
            for name in objective.target.split_whitespace() {
                if !landmark_names.contains(name) {
                    out.push(Finding::warning(
                        "quest-ref",
                        at(&format!("visit `{name}` não é um marco")),
                    ));
                }
            }
        }
        "collect"
            if !objective.target.is_empty() && !item_ids.contains(objective.target.as_str()) =>
        {
            out.push(Finding::error(
                "quest-ref",
                at(&format!("collect `{}` não é um item", objective.target)),
            ));
        }
        _ => {}
    }
}

fn validate_rewards(
    quest: &QuestDef,
    at: &impl Fn(&str) -> String,
    item_ids: &BTreeSet<&str>,
    out: &mut Vec<Finding>,
) {
    for entry in &quest.rewards.items {
        let (id, count) = match entry.split_once(':') {
            Some((id, count)) => (id.trim(), Some(count.trim())),
            None => (entry.trim(), None),
        };
        if id.is_empty() {
            out.push(Finding::error("quest-ref", at("recompensa com item vazio")));
            continue;
        }
        if !item_ids.contains(id) {
            out.push(Finding::error(
                "quest-ref",
                at(&format!("recompensa `{id}` não é um item")),
            ));
        }
        if let Some(count) = count {
            match count.parse::<i64>() {
                Ok(count) if count >= 1 => {}
                _ => out.push(Finding::error(
                    "quest-ref",
                    at(&format!("recompensa `{entry}`: quantidade inválida")),
                )),
            }
        }
    }
}

fn validate_parley(
    quest: &QuestDef,
    at: &impl Fn(&str) -> String,
    item_ids: &BTreeSet<&str>,
    landmark_names: &BTreeSet<&str>,
    out: &mut Vec<Finding>,
) {
    let Some(parley) = &quest.parley else {
        if quest.objective.objective_type != "decision" {
            out.push(Finding::warning(
                "quest-ref",
                at("sem parley (a entrega não pede relatório ao NLU)"),
            ));
        }
        return;
    };
    let Some(report) = &parley.report else {
        out.push(Finding::error("quest-ref", at("parley sem report")));
        return;
    };
    if report.prompt.is_empty() {
        out.push(Finding::error("quest-ref", at("parley.report sem prompt")));
    }
    if report.example.is_empty() {
        out.push(Finding::error("quest-ref", at("parley.report sem example")));
    }
    if report.expect.is_empty() {
        out.push(Finding::error(
            "quest-ref",
            at("parley.report.expect vazio"),
        ));
    }
    if report.hint.is_empty() {
        out.push(Finding::warning("quest-ref", at("parley.report sem hint")));
    }

    // O alvo do NLU deve corresponder ao tipo de objetivo (exceto em quests
    // de decisão, onde o alvo é um desfecho livre).
    let expected_kind = match quest.objective.objective_type.as_str() {
        "kill" => Some("criatura"),
        "collect" => Some("item"),
        "visit" => Some("local"),
        _ => None,
    };

    for expect in &report.expect {
        if !EXPECT_KINDS.contains(&expect.kind.as_str()) {
            out.push(Finding::error(
                "quest-ref",
                at(&format!("parley: kind `{}` inválido", expect.kind)),
            ));
            continue;
        }
        if let Some(expected) = expected_kind
            && expect.kind != expected
        {
            out.push(Finding::warning(
                "quest-ref",
                at(&format!(
                    "parley: kind `{}` não corresponde ao objetivo (esperado `{expected}`)",
                    expect.kind
                )),
            ));
        }
        match expect.kind.as_str() {
            "item" if !item_ids.contains(expect.id.as_str()) => out.push(Finding::error(
                "quest-ref",
                at(&format!("parley: item `{}` desconhecido", expect.id)),
            )),
            "local" if !landmark_names.contains(expect.id.as_str()) => out.push(Finding::warning(
                "quest-ref",
                at(&format!("parley: local `{}` não é um marco", expect.id)),
            )),
            _ => {}
        }
    }
}

pub fn validate_dialogues(
    dialogues: &BTreeMap<String, DialogueFile>,
    quest_ids: &BTreeSet<String>,
    locale: &BTreeMap<String, BTreeMap<String, String>>,
) -> Vec<Finding> {
    let mut out = Vec::new();
    for (file, dialogue) in dialogues {
        let at = |problem: &str| format!("{file}: {problem}");
        if dialogue.start.is_empty() {
            out.push(Finding::error("dialogue-ref", at("sem start")));
        } else if !dialogue.nodes.contains_key(&dialogue.start) {
            out.push(Finding::error(
                "dialogue-ref",
                at(&format!("start `{}` não existe", dialogue.start)),
            ));
        }

        for (id, node) in &dialogue.nodes {
            let node_at = |problem: &str| at(&format!("{id}: {problem}"));
            if id.is_empty() {
                out.push(Finding::error("dialogue-ref", at("nó com id vazio")));
            }
            if node.text.is_empty() {
                out.push(Finding::error("dialogue-ref", node_at("sem texto")));
            } else if let Some(key) = node.text.strip_prefix('@') {
                for lang in ["pt", "en"] {
                    if locale
                        .get(lang)
                        .is_none_or(|strings| !strings.contains_key(key))
                    {
                        out.push(Finding::warning(
                            "dialogue-ref",
                            node_at(&format!("locale `{key}` em falta em {lang}.json")),
                        ));
                    }
                }
            }
            for choice in &node.choices {
                if choice.text.is_empty() {
                    out.push(Finding::warning(
                        "dialogue-ref",
                        node_at("escolha sem texto"),
                    ));
                }
                if !choice.goto.is_empty() && !dialogue.nodes.contains_key(&choice.goto) {
                    out.push(Finding::error(
                        "dialogue-ref",
                        node_at(&format!("goto `{}` não existe", choice.goto)),
                    ));
                }
                if (choice.quest_accept || choice.quest_turn_in) && !quest_ids.contains(file) {
                    out.push(Finding::error(
                        "dialogue-ref",
                        node_at(&format!("flag de quest mas não há quest com id `{file}`")),
                    ));
                }
            }
        }

        for id in unreachable_nodes(dialogue) {
            out.push(Finding::warning(
                "dialogue-ref",
                at(&format!(
                    "nó `{id}` inalcançável a partir de `{}`",
                    dialogue.start
                )),
            ));
        }
    }
    out
}

fn unreachable_nodes(dialogue: &DialogueFile) -> Vec<&str> {
    let mut reached = BTreeSet::new();
    let mut stack = vec![dialogue.start.as_str()];
    while let Some(id) = stack.pop() {
        if id.is_empty() || !dialogue.nodes.contains_key(id) || !reached.insert(id) {
            continue;
        }
        if let Some(node) = dialogue.nodes.get(id) {
            for choice in &node.choices {
                if !choice.goto.is_empty() {
                    stack.push(choice.goto.as_str());
                }
            }
        }
    }
    dialogue
        .nodes
        .keys()
        .map(String::as_str)
        .filter(|id| !reached.contains(id))
        .collect()
}

fn quest_ids(files: &[QuestFile]) -> BTreeSet<String> {
    files
        .iter()
        .flat_map(|file| file.quests.iter())
        .map(|quest| quest.id.clone())
        .filter(|id| !id.is_empty())
        .collect()
}

fn landmark_names(file: &LandmarksFile) -> BTreeSet<&str> {
    file.landmarks
        .iter()
        .map(|landmark| landmark.name.as_str())
        .filter(|name| !name.is_empty())
        .collect()
}

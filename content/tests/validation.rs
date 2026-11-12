//! Testes das regras de validação partilhadas.

use std::collections::BTreeMap;

use vael_content::{
    ContentSnapshot, DialogueChoice, DialogueFile, DialogueNode, Expect, ItemsFile, LandmarkDef,
    LandmarksFile, Objective, Parley, ProgressionFile, QuestDef, QuestFile, QuestIndex, Report,
    Reward, Severity, WorldNames, XpCurve, validate_all, validate_dialogues,
};

fn item(id: &str, label: &str) -> vael_content::ItemDef {
    vael_content::ItemDef {
        id: id.into(),
        label: label.into(),
        kind: "item".into(),
        ..Default::default()
    }
}

fn obj(objective_type: &str, target: &str, count: i64) -> Objective {
    Objective {
        objective_type: objective_type.into(),
        target: target.into(),
        count,
        ..Default::default()
    }
}

fn parley(expect: Vec<Expect>) -> Parley {
    Parley {
        report: Some(Report {
            prompt: "O que encontraste?".into(),
            example: "uma coisa".into(),
            expect,
            hint: "Diz o nome.".into(),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn quest(id: &str) -> QuestDef {
    QuestDef {
        id: id.into(),
        npc: "smith".into(),
        biome: "city".into(),
        title: "Teste".into(),
        lines_intro: vec!["Olá.".into()],
        lines_complete: vec!["Feito.".into()],
        objective: obj("craft", "sword", 1),
        rewards: Reward {
            gold: 10,
            xp: 5,
            ..Default::default()
        },
        parley: Some(parley(vec![Expect {
            kind: "item".into(),
            id: "ore".into(),
            ..Default::default()
        }])),
        ..Default::default()
    }
}

fn world() -> WorldNames {
    let mut world = WorldNames::new();
    world.insert("smith");
    world.insert("forge");
    world
}

fn items() -> ItemsFile {
    ItemsFile {
        items: vec![item("ore", "Minério")],
        ..Default::default()
    }
}

fn landmarks() -> LandmarksFile {
    LandmarksFile {
        campfire_range: 12.0,
        landmarks: vec![LandmarkDef {
            name: "forge".into(),
            biome: "tower".into(),
            label: "Forja".into(),
            radius: 8.0,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn progression() -> ProgressionFile {
    ProgressionFile {
        xp: XpCurve {
            base: 100,
            growth: 1.5,
            skill_points_per_level: 1,
            ..Default::default()
        },
        ..Default::default()
    }
}

#[test]
fn craft_e_um_objetivo_valido() {
    let files = vec![QuestFile::new(
        "craft_quests.json",
        vec![quest("craft_test")],
    )];
    let items = items();
    let landmarks = landmarks();
    let world = world();
    let index = QuestIndex(vec!["craft_quests.json".to_string()]);
    let progression = progression();
    let dialogues = BTreeMap::new();
    let locale = BTreeMap::new();
    let snapshot = ContentSnapshot {
        items: &items,
        progression: &progression,
        landmarks: &landmarks,
        quest_files: &files,
        dialogues: &dialogues,
        locale: &locale,
        quest_index: &index,
        world: &world,
    };
    let findings = validate_all(&snapshot);
    let errors: Vec<_> = findings
        .iter()
        .filter(|finding| finding.severity == Severity::Error)
        .collect();
    assert!(errors.is_empty(), "erros inesperados: {errors:?}");
}

#[test]
fn npc_inexistente_e_erro() {
    let mut quest = quest("q");
    quest.npc = "fantasma".into();
    let files = vec![QuestFile::new("q_quests.json", vec![quest])];
    let items = items();
    let landmarks = landmarks();
    let world = world();
    let index = QuestIndex(vec!["q_quests.json".to_string()]);
    let findings = validate_all(&ContentSnapshot {
        items: &items,
        progression: &ProgressionFile::default(),
        landmarks: &landmarks,
        quest_files: &files,
        dialogues: &BTreeMap::new(),
        locale: &BTreeMap::new(),
        quest_index: &index,
        world: &world,
    });
    assert!(
        findings.iter().any(|finding| finding
            .message
            .contains("npc `fantasma` não existe no mundo")),
        "faltou o erro de npc: {findings:?}"
    );
}

#[test]
fn ficheiro_fora_do_indice_e_erro() {
    let files = vec![QuestFile::new(
        "lost_quests.json",
        vec![quest("craft_test")],
    )];
    let findings = validate_all(&ContentSnapshot {
        items: &items(),
        progression: &progression(),
        landmarks: &landmarks(),
        quest_files: &files,
        dialogues: &BTreeMap::new(),
        locale: &BTreeMap::new(),
        quest_index: &QuestIndex::default(),
        world: &world(),
    });
    assert!(
        findings
            .iter()
            .any(|finding| finding.kind == "quest-index" && finding.severity == Severity::Error),
        "faltou o erro de índice: {findings:?}"
    );
}

#[test]
fn requires_desconhecido_e_aviso() {
    let mut quest = quest("craft_test");
    quest.requires = vec!["nao_existe".into()];
    let files = vec![QuestFile::new("craft_quests.json", vec![quest])];
    let index = QuestIndex(vec!["craft_quests.json".to_string()]);
    let findings = validate_all(&ContentSnapshot {
        items: &items(),
        progression: &progression(),
        landmarks: &landmarks(),
        quest_files: &files,
        dialogues: &BTreeMap::new(),
        locale: &BTreeMap::new(),
        quest_index: &index,
        world: &world(),
    });
    assert!(
        findings
            .iter()
            .any(|finding| finding.severity == Severity::Warning
                && finding.message.contains("requires `nao_existe`")),
        "faltou o aviso de requires: {findings:?}"
    );
}

#[test]
fn dialogo_valida_start_goto_e_locale() {
    let mut nodes = BTreeMap::new();
    nodes.insert(
        "start".to_string(),
        DialogueNode {
            speaker: "Nela".into(),
            text: "@em_falta".into(),
            choices: vec![DialogueChoice {
                text: "Ir".into(),
                goto: "nao_existe".into(),
                ..Default::default()
            }],
            ..Default::default()
        },
    );
    let mut dialogues = BTreeMap::new();
    dialogues.insert(
        "nela".to_string(),
        DialogueFile {
            start: "start".into(),
            nodes,
            ..Default::default()
        },
    );
    let findings = validate_dialogues(&dialogues, &Default::default(), &BTreeMap::new());
    assert!(
        findings
            .iter()
            .any(|finding| finding.message.contains("goto `nao_existe` não existe")),
        "faltou o erro de goto: {findings:?}"
    );
    assert!(
        findings
            .iter()
            .any(|finding| finding.message.contains("locale `em_falta`")),
        "faltou o aviso de locale: {findings:?}"
    );
}

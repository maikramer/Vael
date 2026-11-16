//! Round-trip dos JSON reais do projeto: carregar num tipo do
//! `vael-content`, voltar a serializar e comparar com o original.
//!
//! Um campo que o crate não conhece tem de sobreviver no `extra`; um campo
//! opcional ausente não pode aparecer como `""`/`0`/`[]`.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use vael_content::{
    DialogueFile, ItemsFile, LandmarksFile, ProgressionFile, PuzzleDef, QuestDef, QuestIndex,
    RecipesFile, RiddlesFile, VerdictsFile,
};

fn project_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("content/ tem de viver dentro do Vael")
        .to_path_buf()
}

fn original(rel: &str) -> Value {
    let path = project_root().join(rel);
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

fn roundtrip<T>(rel: &str)
where
    T: Serialize + DeserializeOwned,
{
    let before = original(rel);
    let typed: T = serde_json::from_value(before.clone())
        .unwrap_or_else(|err| panic!("{rel}: não desserializa: {err}"));
    let after =
        serde_json::to_value(&typed).unwrap_or_else(|err| panic!("{rel}: não serializa: {err}"));
    assert_same(rel, &before, &after);
}

fn assert_same(rel: &str, before: &Value, after: &Value) {
    let mut diffs = Vec::new();
    diff(before, after, rel, &mut diffs);
    if !diffs.is_empty() {
        panic!(
            "{rel}: o round-trip mudou {} caminho(s):\n{}",
            diffs.len(),
            diffs.into_iter().take(12).collect::<Vec<_>>().join("\n")
        );
    }
}

fn diff(before: &Value, after: &Value, path: &str, out: &mut Vec<String>) {
    match (before, after) {
        (Value::Object(before), Value::Object(after)) => {
            for (key, value) in before {
                let child = format!("{path}.{key}");
                match after.get(key) {
                    Some(other) => diff(value, other, &child, out),
                    None => out.push(format!("{child}: desapareceu")),
                }
            }
            for key in after.keys() {
                if !before.contains_key(key) {
                    out.push(format!("{path}.{key}: apareceu"));
                }
            }
        }
        (Value::Array(before), Value::Array(after)) => {
            if before.len() != after.len() {
                out.push(format!(
                    "{path}: comprimento {} -> {}",
                    before.len(),
                    after.len()
                ));
            }
            for (index, (left, right)) in before.iter().zip(after).enumerate() {
                diff(left, right, &format!("{path}[{index}]"), out);
            }
        }
        // `3` e `3.0` são o mesmo número JSON; o crate escreve a forma
        // compacta quando não há parte fracionária.
        (Value::Number(before), Value::Number(after)) if before.as_f64() == after.as_f64() => {}
        _ if before != after => out.push(format!("{path}: {before} -> {after}")),
        _ => {}
    }
}

#[test]
fn itens_fazem_roundtrip() {
    roundtrip::<ItemsFile>("data/items.json");
}

#[test]
fn progressao_faz_roundtrip() {
    roundtrip::<ProgressionFile>("data/progression.json");
}

#[test]
fn marcos_fazem_roundtrip() {
    roundtrip::<LandmarksFile>("data/landmarks.json");
}

#[test]
fn receitas_fazem_roundtrip() {
    roundtrip::<RecipesFile>("data/recipes.json");
}

#[test]
fn enigmas_fazem_roundtrip() {
    roundtrip::<RiddlesFile>("data/riddles.json");
}

#[test]
fn veredictos_fazem_roundtrip() {
    roundtrip::<VerdictsFile>("data/verdicts.json");
}

#[test]
fn puzzles_fazem_roundtrip() {
    let dir = project_root().join("data").join("quest_puzzles");
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "data/quest_puzzles/ vazio");
    for path in files {
        let name = path.file_name().unwrap().to_str().unwrap();
        roundtrip::<PuzzleDef>(&format!("data/quest_puzzles/{name}"));
    }
}

#[test]
fn quests_index_faz_roundtrip() {
    roundtrip::<QuestIndex>("quests/index.json");
}

#[test]
fn todas_as_quests_fazem_roundtrip() {
    let index: QuestIndex = serde_json::from_value(original("quests/index.json")).unwrap();
    for file in &index.0 {
        let rel = format!("quests/{file}");
        let before = original(&rel);
        let quests: Vec<QuestDef> = serde_json::from_value(before.clone())
            .unwrap_or_else(|err| panic!("{rel}: não desserializa: {err}"));
        let after = serde_json::to_value(&quests).unwrap();
        assert_same(&rel, &before, &after);
    }
}

#[test]
fn todos_os_dialogos_fazem_roundtrip() {
    let dir = project_root().join("dialogues");
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "dialogues/ vazio");
    for path in files {
        let name = path.file_name().unwrap().to_str().unwrap();
        roundtrip::<DialogueFile>(&format!("dialogues/{name}"));
    }
}

#[test]
fn campos_desconhecidos_sobrevivem() {
    let json = serde_json::json!({
        "id": "teste",
        "label": "Teste",
        "kind": "item",
        "order": 0,
        "campo_futuro": { "a": 1 }
    });
    let item: vael_content::ItemDef = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(item.extra.get("campo_futuro"), json.get("campo_futuro"));
    let after = serde_json::to_value(&item).unwrap();
    assert_eq!(after.get("campo_futuro"), json.get("campo_futuro"));
}

/// Os quatro documentos que não tinham `extra` (receitas, enigmas,
/// veredictos e puzzles) têm de devolver o que não conhecem. Um campo novo do
/// escritor não pode desaparecer só porque o editor foi gravado antes.
#[test]
fn campos_desconhecidos_sobrevivem_nos_schemas_novos() {
    let futuro = serde_json::json!({ "por_autorar": true });

    let recipe = serde_json::json!({
        "id": "r", "out": "potion", "count": 1, "cost": { "berry": 1 },
        "campo_futuro": futuro
    });
    let recipe: vael_content::RecipeDef = serde_json::from_value(recipe).unwrap();
    assert_eq!(recipe.extra.get("campo_futuro"), Some(&futuro));
    let after = serde_json::to_value(&recipe).unwrap();
    assert_eq!(after.get("campo_futuro"), Some(&futuro));

    let riddle = serde_json::json!({
        "id": "r", "site": "s", "interact": "i",
        "clue": { "kind": "props", "campo_futuro": futuro },
        "prompt": "p", "example": "e",
        "expect": [{ "kind": "item", "id": "stone" }],
        "all": false, "hint": "h", "solved": "s",
        "reward": { "xp": 1 },
        "campo_futuro": futuro
    });
    let riddle: vael_content::RiddleDef = serde_json::from_value(riddle).unwrap();
    assert_eq!(riddle.extra.get("campo_futuro"), Some(&futuro));
    assert_eq!(riddle.clue.extra.get("campo_futuro"), Some(&futuro));
    let after = serde_json::to_value(&riddle).unwrap();
    assert_eq!(after.get("campo_futuro"), Some(&futuro));
    assert_eq!(after["clue"].get("campo_futuro"), Some(&futuro));

    let verdict = serde_json::json!({
        "id": "v", "site": "s", "intro": "i",
        "require": [{ "kind": "item", "id": "torch", "verb": "equipar",
                      "campo_futuro": futuro }],
        "order": ["equipar"],
        "forbid": [{ "kind": "item", "id": "potion" }],
        "window": 900,
        "verdicts": { "correct": "c", "partial": "p", "wrong": "w",
                      "campo_futuro": futuro },
        "reward": {},
        "campo_futuro": futuro
    });
    let verdict: vael_content::VerdictDef = serde_json::from_value(verdict).unwrap();
    assert_eq!(verdict.extra.get("campo_futuro"), Some(&futuro));
    assert_eq!(verdict.require[0].extra.get("campo_futuro"), Some(&futuro));
    assert_eq!(verdict.verdicts.extra.get("campo_futuro"), Some(&futuro));
    let after = serde_json::to_value(&verdict).unwrap();
    assert_eq!(after.get("campo_futuro"), Some(&futuro));
    assert_eq!(after["require"][0].get("campo_futuro"), Some(&futuro));
    assert_eq!(after["verdicts"].get("campo_futuro"), Some(&futuro));

    let puzzle = serde_json::json!({
        "giver": "g", "intro": ["a"], "ask": "a", "example": "e",
        "clues": [{ "key": "k", "description": "d", "reply": "r",
                    "campo_futuro": futuro }],
        "required": 1,
        "no_answer": "n", "more": "m", "reveal": "r", "cancel": "c",
        "final_prompt": "p", "final_example": "e",
        "final_options": [{ "key": "k", "description": "d",
                            "campo_futuro": futuro }],
        "final_answer": "k",
        "final_reply_ok": "o", "final_reply_bad": "b", "final_cancel": "c",
        "hint": "h",
        "endings": { "k": { "text": "t", "items": [],
                            "campo_futuro": futuro } },
        "after": ["a"],
        "campo_futuro": futuro
    });
    let puzzle: vael_content::PuzzleDef = serde_json::from_value(puzzle).unwrap();
    assert_eq!(puzzle.extra.get("campo_futuro"), Some(&futuro));
    assert_eq!(puzzle.clues[0].extra.get("campo_futuro"), Some(&futuro));
    assert_eq!(
        puzzle.final_options[0].extra.get("campo_futuro"),
        Some(&futuro)
    );
    assert_eq!(puzzle.endings["k"].extra.get("campo_futuro"), Some(&futuro));
    let after = serde_json::to_value(&puzzle).unwrap();
    assert_eq!(after.get("campo_futuro"), Some(&futuro));
    assert_eq!(after["clues"][0].get("campo_futuro"), Some(&futuro));
    assert_eq!(after["final_options"][0].get("campo_futuro"), Some(&futuro));
    assert_eq!(after["endings"]["k"].get("campo_futuro"), Some(&futuro));
}

#[test]
fn requires_aceita_string_e_lista() {
    let um: QuestDef = serde_json::from_value(serde_json::json!({
        "id": "a",
        "requires": "b"
    }))
    .unwrap();
    assert_eq!(um.requires, vec!["b".to_string()]);

    let muitos: QuestDef = serde_json::from_value(serde_json::json!({
        "id": "a",
        "requires": ["b", "c"]
    }))
    .unwrap();
    assert_eq!(muitos.requires, vec!["b".to_string(), "c".to_string()]);

    let after = serde_json::to_value(&um).unwrap();
    assert_eq!(after.get("requires"), Some(&serde_json::json!("b")));
}

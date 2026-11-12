//! Operadores do editor do Vael.
//!
//! Tudo o que muda conteúdo ou ficheiros passa por aqui: os botões dos
//! painéis só dispatcham ids de operador. Gravar é `allows_undo = false,
//! writes_disk = true` (o undo do Jackdaw é do documento de cena, não de
//! ficheiros externos).

use std::collections::BTreeSet;

use bevy::prelude::*;
use jackdaw_editor_core::prelude::*;
use vael_content::{
    DialogueChoice, DialogueFile, DialogueNode, ItemDef, LandmarkDef, Objective, QuestDef, SkillDef,
};

use crate::io;
use crate::state::{VaelRoot, VaelState};
use crate::ui::{PanelId, PanelState};

pub(crate) fn parse_panel(name: &str) -> Option<PanelId> {
    match name {
        "items" => Some(PanelId::Items),
        "landmarks" => Some(PanelId::Landmarks),
        "progression" => Some(PanelId::Progression),
        "dialogues" => Some(PanelId::Dialogues),
        "quests" => Some(PanelId::Quests),
        _ => None,
    }
}

pub(crate) fn panel_name(panel: PanelId) -> &'static str {
    match panel {
        PanelId::Items => "items",
        PanelId::Landmarks => "landmarks",
        PanelId::Progression => "progression",
        PanelId::Dialogues => "dialogues",
        PanelId::Quests => "quests",
    }
}

pub(crate) fn unique(existing: impl IntoIterator<Item = String>, base: &str) -> String {
    let existing: BTreeSet<String> = existing.into_iter().collect();
    if !existing.contains(base) {
        return base.to_string();
    }
    for index in 2.. {
        let candidate = format!("{base}_{index}");
        if !existing.contains(&candidate) {
            return candidate;
        }
    }
    unreachable!("o iterador de inteiros é infinito")
}

/// Seleciona um objeto num painel.
#[operator(
    id = "vael.content.select",
    label = "Vael: selecionar",
    allows_undo = false,
    params(
        panel(String, doc = "items | landmarks | progression | dialogues | quests"),
        id(String, doc = "Chave do objeto (id, nome ou nó)."),
    )
)]
pub(crate) fn select_content(
    In(params): In<OperatorParameters>,
    state: Res<VaelState>,
    mut panel: ResMut<PanelState>,
) -> OperatorResult {
    let Some(panel_id) = params.as_str("panel").and_then(parse_panel) else {
        return OperatorResult::Cancelled;
    };
    let Some(id) = params.as_str("id") else {
        return OperatorResult::Cancelled;
    };
    if panel_id == PanelId::Quests
        && let Some(index) = state.quest_file_of(id)
    {
        panel.quest_file = Some(state.quest_files[index].file.clone());
    }
    panel.select(panel_id, Some(id.to_string()));
    OperatorResult::Finished
}

/// Seleciona um ficheiro (diálogos/quests).
#[operator(
    id = "vael.content.select_file",
    label = "Vael: abrir ficheiro",
    allows_undo = false,
    params(
        panel(String, doc = "dialogues | quests"),
        file(String, doc = "Nome do ficheiro com extensão."),
    )
)]
pub(crate) fn select_file(
    In(params): In<OperatorParameters>,
    state: Res<VaelState>,
    mut panel: ResMut<PanelState>,
) -> OperatorResult {
    let Some(panel_id) = params.as_str("panel").and_then(parse_panel) else {
        return OperatorResult::Cancelled;
    };
    let Some(file) = params.as_str("file") else {
        return OperatorResult::Cancelled;
    };
    match panel_id {
        PanelId::Dialogues => {
            panel.dialogue_file = Some(file.to_string());
            panel.dialogue_node = state
                .dialogues
                .get(file)
                .and_then(|dialogue| dialogue.nodes.keys().next().cloned());
            panel.dialogue_choice = 0;
        }
        PanelId::Quests => {
            panel.quest_file = Some(file.to_string());
            panel.quest = state
                .quest_files
                .iter()
                .find(|quest_file| quest_file.file == file)
                .and_then(|quest_file| quest_file.quests.first().map(|quest| quest.id.clone()));
        }
        _ => return OperatorResult::Cancelled,
    }
    OperatorResult::Finished
}

/// Cria um objeto novo no painel (id/nome gerado sem colidir).
#[operator(
    id = "vael.content.add",
    label = "Vael: novo",
    allows_undo = false,
    params(panel(String, doc = "items | landmarks | progression | dialogues | quests"))
)]
pub(crate) fn add_content(
    In(params): In<OperatorParameters>,
    mut state: ResMut<VaelState>,
    mut panel: ResMut<PanelState>,
) -> OperatorResult {
    let Some(panel_id) = params.as_str("panel").and_then(parse_panel) else {
        return OperatorResult::Cancelled;
    };
    let Some(id) = add_to(panel_id, &mut state, &mut panel) else {
        return OperatorResult::Cancelled;
    };
    panel.select(panel_id, Some(id));
    panel.status = "objeto novo: grava para persistir".to_string();
    OperatorResult::Finished
}

pub(crate) fn add_to(panel: PanelId, state: &mut VaelState, ui: &mut PanelState) -> Option<String> {
    match panel {
        PanelId::Items => {
            let id = unique(
                state.items.items.iter().map(|item| item.id.clone()),
                "novo_item",
            );
            let order = state.items.items.len() as i64;
            state.items.items.push(ItemDef {
                id: id.clone(),
                label: "Novo item".to_string(),
                kind: "item".to_string(),
                order,
                ..Default::default()
            });
            Some(id)
        }
        PanelId::Landmarks => {
            let name = unique(
                state
                    .landmarks
                    .landmarks
                    .iter()
                    .map(|landmark| landmark.name.clone()),
                "novo-marco",
            );
            state.landmarks.landmarks.push(LandmarkDef {
                name: name.clone(),
                biome: "tower".to_string(),
                label: "Novo marco".to_string(),
                radius: 10.0,
                ..Default::default()
            });
            Some(name)
        }
        PanelId::Progression => {
            let id = unique(
                state
                    .progression
                    .skills
                    .iter()
                    .map(|skill| skill.id.clone()),
                "novo_talento",
            );
            state.progression.skills.push(SkillDef {
                id: id.clone(),
                label: "Novo talento".to_string(),
                ..Default::default()
            });
            Some(id)
        }
        PanelId::Dialogues => {
            let file_name = ui
                .dialogue_file
                .clone()
                .or_else(|| state.dialogues.keys().next().cloned())
                .unwrap_or_else(|| "novo_dialogo".to_string());
            if !state.dialogues.contains_key(&file_name) {
                state.dialogues.insert(
                    file_name.clone(),
                    DialogueFile {
                        start: "start".to_string(),
                        nodes: [(
                            "start".to_string(),
                            DialogueNode {
                                speaker: String::new(),
                                text: String::new(),
                                ..Default::default()
                            },
                        )]
                        .into_iter()
                        .collect(),
                        ..Default::default()
                    },
                );
            }
            let dialogue = state.dialogues.get_mut(&file_name)?;
            let id = unique(dialogue.nodes.keys().cloned(), "novo_no");
            dialogue.nodes.insert(
                id.clone(),
                DialogueNode {
                    speaker: String::new(),
                    text: String::new(),
                    ..Default::default()
                },
            );
            if dialogue.start.is_empty() {
                dialogue.start = id.clone();
            }
            ui.dialogue_file = Some(file_name);
            ui.dialogue_choice = 0;
            Some(id)
        }
        PanelId::Quests => {
            let file_name = ui
                .quest_file
                .clone()
                .or_else(|| state.quest_files.first().map(|file| file.file.clone()))
                .unwrap_or_else(|| "nova_quests.json".to_string());
            if !state.quest_files.iter().any(|file| file.file == file_name) {
                state
                    .quest_files
                    .push(vael_content::QuestFile::new(file_name.clone(), Vec::new()));
            }
            let existing = state
                .quest_files
                .iter()
                .flat_map(|file| file.quests.iter())
                .map(|quest| quest.id.clone());
            let id = unique(existing, "nova_quest");
            let file = state
                .quest_files
                .iter_mut()
                .find(|file| file.file == file_name)?;
            file.quests.push(QuestDef {
                id: id.clone(),
                npc: String::new(),
                biome: "city".to_string(),
                title: "Nova missão".to_string(),
                objective: Objective {
                    objective_type: "kill".to_string(),
                    target: "wolf".to_string(),
                    count: 1,
                    ..Default::default()
                },
                ..Default::default()
            });
            ui.quest_file = Some(file_name);
            Some(id)
        }
    }
}

/// Apaga o objeto, em dois cliques (o primeiro arma, o segundo apaga).
#[operator(
    id = "vael.content.delete",
    label = "Vael: apagar",
    allows_undo = false,
    params(
        panel(String, doc = "items | landmarks | progression | dialogues | quests"),
        id(String, doc = "Chave do objeto."),
    )
)]
pub(crate) fn delete_content(
    In(params): In<OperatorParameters>,
    mut state: ResMut<VaelState>,
    mut panel: ResMut<PanelState>,
) -> OperatorResult {
    let Some(panel_id) = params.as_str("panel").and_then(parse_panel) else {
        return OperatorResult::Cancelled;
    };
    let Some(id) = params.as_str("id") else {
        return OperatorResult::Cancelled;
    };
    if panel.delete_armed.as_deref() != Some(id) {
        panel.delete_armed = Some(id.to_string());
        panel.status = format!("{id}: prime Apagar outra vez para confirmar");
        return OperatorResult::Finished;
    }

    let next = match panel_id {
        PanelId::Items => {
            state.items.items.retain(|item| item.id != id);
            state.items.items.first().map(|item| item.id.clone())
        }
        PanelId::Landmarks => {
            state
                .landmarks
                .landmarks
                .retain(|landmark| landmark.name != id);
            state
                .landmarks
                .landmarks
                .first()
                .map(|landmark| landmark.name.clone())
        }
        PanelId::Progression => {
            state.progression.skills.retain(|skill| skill.id != id);
            state
                .progression
                .skills
                .first()
                .map(|skill| skill.id.clone())
        }
        PanelId::Dialogues => {
            let Some(file_name) = panel.dialogue_file.clone() else {
                return OperatorResult::Cancelled;
            };
            let Some(dialogue) = state.dialogues.get_mut(&file_name) else {
                return OperatorResult::Cancelled;
            };
            dialogue.nodes.remove(id);
            dialogue.nodes.keys().next().cloned()
        }
        PanelId::Quests => {
            for file in &mut state.quest_files {
                file.quests.retain(|quest| quest.id != id);
            }
            let next = state
                .quest_files
                .iter()
                .flat_map(|file| file.quests.iter())
                .next()
                .map(|quest| quest.id.clone());
            if let Some(next) = &next
                && let Some(index) = state.quest_file_of(next)
            {
                panel.quest_file = Some(state.quest_files[index].file.clone());
            }
            next
        }
    };
    panel.select(panel_id, next);
    panel.status = format!("{id}: apagado (por gravar)");
    OperatorResult::Finished
}

/// Grava o ficheiro do painel atual.
#[operator(
    id = "vael.content.save",
    label = "Vael: gravar",
    allows_undo = false,
    writes_disk = true,
    params(panel(String, doc = "items | landmarks | progression | dialogues | quests"))
)]
pub(crate) fn save_content(
    In(params): In<OperatorParameters>,
    root: Res<VaelRoot>,
    state: Res<VaelState>,
    mut panel: ResMut<PanelState>,
) -> OperatorResult {
    let Some(panel_id) = params.as_str("panel").and_then(parse_panel) else {
        return OperatorResult::Cancelled;
    };
    let result = match panel_id {
        PanelId::Items => require_loaded(&state, "data/items.json")
            .and_then(|()| io::write_json_atomic(&root.0.join("data/items.json"), &state.items)),
        PanelId::Landmarks => require_loaded(&state, "data/landmarks.json").and_then(|()| {
            io::write_json_atomic(&root.0.join("data/landmarks.json"), &state.landmarks)
        }),
        PanelId::Progression => require_loaded(&state, "data/progression.json").and_then(|()| {
            io::write_json_atomic(&root.0.join("data/progression.json"), &state.progression)
        }),
        PanelId::Dialogues => {
            let Some(file) = panel.dialogue_file.clone() else {
                panel.status = "sem ficheiro de diálogo selecionado".to_string();
                return OperatorResult::Cancelled;
            };
            let rel = format!("dialogues/{file}.json");
            if let Err(err) = require_loaded(&state, &rel) {
                panel.status = err;
                return OperatorResult::Cancelled;
            }
            let Some(dialogue) = state.dialogues.get(&file) else {
                panel.status = format!("diálogo `{file}` não carregado");
                return OperatorResult::Cancelled;
            };
            io::write_json_atomic(
                &root.0.join("dialogues").join(format!("{file}.json")),
                dialogue,
            )
        }
        PanelId::Quests => {
            let Some(file) = panel.quest_file.clone() else {
                panel.status = "sem ficheiro de quests selecionado".to_string();
                return OperatorResult::Cancelled;
            };
            save_quest_file(&root.0, &state, &file)
        }
    };
    finish_write(result, &mut panel)
}

fn require_loaded(state: &VaelState, rel: &str) -> Result<(), String> {
    if state.failed.contains(rel) {
        Err(format!(
            "{rel} falhou a carregar; gravar escreveria os defaults por cima do ficheiro"
        ))
    } else {
        Ok(())
    }
}

pub(crate) fn save_quest_file(
    root: &std::path::Path,
    state: &VaelState,
    file: &str,
) -> Result<(), String> {
    require_loaded(state, &format!("quests/{file}"))?;
    require_loaded(state, "quests/index.json")?;
    let Some(quest_file) = state.quest_files.iter().find(|entry| entry.file == file) else {
        return Err(format!("ficheiro de quests `{file}` não carregado"));
    };
    io::write_json_atomic(&root.join("quests").join(file), &quest_file.quests)?;
    let mut index = state.quest_index.clone();
    index.insert_sorted(file);
    io::write_json_atomic(&root.join("quests/index.json"), &index)
}

/// Grava todos os ficheiros de conteúdo.
#[operator(
    id = "vael.content.save_all",
    label = "Vael: gravar tudo",
    allows_undo = false,
    writes_disk = true
)]
pub(crate) fn save_all(
    _params: In<OperatorParameters>,
    root: Res<VaelRoot>,
    state: Res<VaelState>,
    mut panel: ResMut<PanelState>,
) -> OperatorResult {
    let result = save_everything(&root.0, &state);
    finish_write(result, &mut panel)
}

pub(crate) fn save_everything(root: &std::path::Path, state: &VaelState) -> Result<(), String> {
    if !state.failed.is_empty() {
        let mut failed: Vec<&str> = state.failed.iter().map(String::as_str).collect();
        failed.sort_unstable();
        return Err(format!(
            "não grava: {} ficheiro(s) falharam a carregar ({})",
            failed.len(),
            failed.join(", ")
        ));
    }
    io::write_json_atomic(&root.join("data/items.json"), &state.items)?;
    io::write_json_atomic(&root.join("data/landmarks.json"), &state.landmarks)?;
    io::write_json_atomic(&root.join("data/progression.json"), &state.progression)?;
    for (name, dialogue) in &state.dialogues {
        io::write_json_atomic(
            &root.join("dialogues").join(format!("{name}.json")),
            dialogue,
        )?;
    }
    for file in &state.quest_files {
        io::write_json_atomic(&root.join("quests").join(&file.file), &file.quests)?;
    }
    let mut index = state.quest_index.clone();
    for file in &state.quest_files {
        index.insert_sorted(&file.file);
    }
    io::write_json_atomic(&root.join("quests/index.json"), &index)
}

pub(crate) fn finish_write(result: Result<(), String>, panel: &mut PanelState) -> OperatorResult {
    match result {
        Ok(()) => {
            panel.status = "gravado".to_string();
            OperatorResult::Finished
        }
        Err(err) => {
            panel.status = format!("erro ao gravar: {err}");
            OperatorResult::Cancelled
        }
    }
}

/// Recarrega o conteúdo do disco.
#[operator(
    id = "vael.content.reload",
    label = "Vael: recarregar conteúdo",
    allows_undo = false
)]
pub(crate) fn reload_content(
    _params: In<OperatorParameters>,
    root: Res<VaelRoot>,
    mut state: ResMut<VaelState>,
    mut panel: ResMut<PanelState>,
) -> OperatorResult {
    state.reload(&root.0);
    repair_selections(&state, &mut panel);
    panel.status = "recarregado".to_string();
    OperatorResult::Finished
}

pub(crate) fn repair_selections(state: &VaelState, panel: &mut PanelState) {
    if panel
        .item
        .as_ref()
        .is_some_and(|id| !state.items.items.iter().any(|item| &item.id == id))
    {
        panel.item = state.items.items.first().map(|item| item.id.clone());
    }
    if panel.landmark.as_ref().is_some_and(|name| {
        !state
            .landmarks
            .landmarks
            .iter()
            .any(|landmark| &landmark.name == name)
    }) {
        panel.landmark = state
            .landmarks
            .landmarks
            .first()
            .map(|landmark| landmark.name.clone());
    }
    if panel
        .skill
        .as_ref()
        .is_some_and(|id| !state.progression.skills.iter().any(|skill| &skill.id == id))
    {
        panel.skill = state
            .progression
            .skills
            .first()
            .map(|skill| skill.id.clone());
    }
    if panel
        .quest
        .as_ref()
        .is_some_and(|id| state.quest(id).is_none())
    {
        panel.quest = state
            .quest_files
            .iter()
            .flat_map(|file| file.quests.iter())
            .next()
            .map(|quest| quest.id.clone());
    }
    if let Some(quest) = &panel.quest
        && let Some(index) = state.quest_file_of(quest)
    {
        panel.quest_file = Some(state.quest_files[index].file.clone());
    }
    if panel
        .dialogue_file
        .as_ref()
        .is_some_and(|file| !state.dialogues.contains_key(file))
    {
        panel.dialogue_file = state.dialogues.keys().next().cloned();
    }
    if panel.dialogue_node.as_ref().is_some_and(|node| {
        panel
            .dialogue_file
            .as_ref()
            .and_then(|file| state.dialogues.get(file))
            .is_none_or(|dialogue| !dialogue.nodes.contains_key(node))
    }) {
        panel.dialogue_node = panel
            .dialogue_file
            .as_ref()
            .and_then(|file| state.dialogues.get(file))
            .and_then(|dialogue| dialogue.nodes.keys().next().cloned());
    }
}

/// Alterna um booleano (curar, peregrino, partial, flags de escolha).
#[operator(
    id = "vael.content.toggle",
    label = "Vael: alternar",
    allows_undo = false,
    params(
        panel(String, doc = "items | landmarks | progression | dialogues | quests"),
        id(String, doc = "Chave do objeto/nó."),
        field(
            String,
            doc = "use_cure | pilgrim | parley_partial | quest_accept | quest_turn_in"
        ),
        index(i64, default = 0, doc = "Índice da escolha no diálogo."),
    )
)]
pub(crate) fn toggle_content(
    In(params): In<OperatorParameters>,
    mut state: ResMut<VaelState>,
    panel: Res<PanelState>,
) -> OperatorResult {
    let Some(panel_id) = params.as_str("panel").and_then(parse_panel) else {
        return OperatorResult::Cancelled;
    };
    let Some(id) = params.as_str("id") else {
        return OperatorResult::Cancelled;
    };
    let Some(field) = params.as_str("field") else {
        return OperatorResult::Cancelled;
    };
    let index = params.as_int("index").unwrap_or(0).max(0) as usize;

    let ok = match (panel_id, field) {
        (PanelId::Items, "use_cure") => state
            .items
            .items
            .iter_mut()
            .find(|item| item.id == id)
            .map(|item| {
                let use_item = item.use_item.get_or_insert_with(Default::default);
                use_item.cure = !use_item.cure;
            })
            .is_some(),
        (PanelId::Landmarks, "pilgrim") => state
            .landmarks
            .landmarks
            .iter_mut()
            .find(|landmark| landmark.name == id)
            .map(|landmark| landmark.pilgrim = !landmark.pilgrim)
            .is_some(),
        (PanelId::Quests, "parley_partial") => state
            .quest_mut(id)
            .map(|quest| {
                let parley = quest.parley.get_or_insert_with(Default::default);
                parley.partial = !parley.partial;
            })
            .is_some(),
        (PanelId::Dialogues, "quest_accept" | "quest_turn_in") => {
            let Some(file_name) = panel.dialogue_file.clone() else {
                return OperatorResult::Cancelled;
            };
            state
                .dialogues
                .get_mut(&file_name)
                .and_then(|dialogue| dialogue.nodes.get_mut(id))
                .and_then(|node| node.choices.get_mut(index))
                .map(|choice| {
                    if field == "quest_accept" {
                        choice.quest_accept = !choice.quest_accept;
                    } else {
                        choice.quest_turn_in = !choice.quest_turn_in;
                    }
                })
                .is_some()
        }
        _ => false,
    };
    if ok {
        OperatorResult::Finished
    } else {
        OperatorResult::Cancelled
    }
}

/// Acrescenta uma escolha a um nó de diálogo.
#[operator(
    id = "vael.dialogue.add_choice",
    label = "Vael: nova escolha",
    allows_undo = false,
    params(node(String, doc = "Nó do diálogo."))
)]
pub(crate) fn add_choice(
    In(params): In<OperatorParameters>,
    mut state: ResMut<VaelState>,
    mut panel: ResMut<PanelState>,
) -> OperatorResult {
    let Some(node_id) = params.as_str("node") else {
        return OperatorResult::Cancelled;
    };
    let Some(file_name) = panel.dialogue_file.clone() else {
        return OperatorResult::Cancelled;
    };
    let Some(node) = state
        .dialogues
        .get_mut(&file_name)
        .and_then(|dialogue| dialogue.nodes.get_mut(node_id))
    else {
        return OperatorResult::Cancelled;
    };
    node.choices.push(DialogueChoice {
        text: String::new(),
        goto: String::new(),
        ..Default::default()
    });
    panel.dialogue_choice = node.choices.len().saturating_sub(1);
    OperatorResult::Finished
}

/// Remove uma escolha de um nó de diálogo.
#[operator(
    id = "vael.dialogue.delete_choice",
    label = "Vael: apagar escolha",
    allows_undo = false,
    params(
        node(String, doc = "Nó do diálogo."),
        index(i64, default = 0, doc = "Índice da escolha."),
    )
)]
pub(crate) fn delete_choice(
    In(params): In<OperatorParameters>,
    mut state: ResMut<VaelState>,
    mut panel: ResMut<PanelState>,
) -> OperatorResult {
    let Some(node_id) = params.as_str("node") else {
        return OperatorResult::Cancelled;
    };
    let index = params.as_int("index").unwrap_or(0).max(0) as usize;
    let Some(file_name) = panel.dialogue_file.clone() else {
        return OperatorResult::Cancelled;
    };
    let Some(node) = state
        .dialogues
        .get_mut(&file_name)
        .and_then(|dialogue| dialogue.nodes.get_mut(node_id))
    else {
        return OperatorResult::Cancelled;
    };
    if index < node.choices.len() {
        node.choices.remove(index);
    }
    panel.dialogue_choice = panel
        .dialogue_choice
        .min(node.choices.len().saturating_sub(1));
    OperatorResult::Finished
}

/// Seleciona uma escolha dentro do nó aberto.
#[operator(
    id = "vael.dialogue.select_choice",
    label = "Vael: selecionar escolha",
    allows_undo = false,
    params(index(i64, default = 0, doc = "Índice da escolha."))
)]
pub(crate) fn select_choice(
    In(params): In<OperatorParameters>,
    mut panel: ResMut<PanelState>,
) -> OperatorResult {
    panel.dialogue_choice = params.as_int("index").unwrap_or(0).max(0) as usize;
    OperatorResult::Finished
}

/// Acrescenta uma criatura a `kill_xp`.
#[operator(
    id = "vael.progression.add_kill_xp",
    label = "Vael: nova criatura em kill_xp",
    allows_undo = false
)]
pub(crate) fn add_kill_xp(
    _params: In<OperatorParameters>,
    mut state: ResMut<VaelState>,
) -> OperatorResult {
    let key = unique(state.progression.kill_xp.keys().cloned(), "nova_criatura");
    state.progression.kill_xp.insert(key, 1);
    OperatorResult::Finished
}

/// Remove uma criatura de `kill_xp`.
#[operator(
    id = "vael.progression.remove_kill_xp",
    label = "Vael: remover criatura de kill_xp",
    allows_undo = false,
    params(id(String, doc = "Nome da criatura."))
)]
pub(crate) fn remove_kill_xp(
    In(params): In<OperatorParameters>,
    mut state: ResMut<VaelState>,
) -> OperatorResult {
    let Some(id) = params.as_str("id") else {
        return OperatorResult::Cancelled;
    };
    if state.progression.kill_xp.remove(id).is_some() {
        OperatorResult::Finished
    } else {
        OperatorResult::Cancelled
    }
}

/// Recalcula os problemas e mostra o resumo no status.
#[operator(
    id = "vael.content.validate",
    label = "Vael: validar conteúdo",
    allows_undo = false
)]
pub(crate) fn validate_content(
    _params: In<OperatorParameters>,
    state: Res<VaelState>,
    mut panel: ResMut<PanelState>,
) -> OperatorResult {
    let findings = state.findings();
    let errors = findings
        .iter()
        .filter(|finding| finding.severity == vael_content::Severity::Error)
        .count();
    let warnings = findings.len().saturating_sub(errors);
    panel.status = format!("{errors} erro(s), {warnings} aviso(s)");
    OperatorResult::Finished
}

/// Assinatura usada pelo painel para o nome do painel no status.
pub fn panel_label(panel: PanelId) -> &'static str {
    panel_name(panel)
}

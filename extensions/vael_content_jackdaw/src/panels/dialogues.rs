//! Painel de diálogos (`dialogues/*.json`).

use bevy::prelude::*;

use vael_content::DialogueNode;

use crate::state::VaelState;
use crate::ui::{
    ChoiceFieldBinding, FieldId, PanelId, PanelPopulated, PanelState, TextFieldBinding,
    action_button, body, button_row, choice_field, heading, index_button, label, list_button,
    separator, status_line, text_field, toolbar,
};

#[derive(Component)]
pub struct DialoguesRoot;

pub fn rebuild(
    mut commands: Commands,
    roots: Query<Entity, With<DialoguesRoot>>,
    populated: Query<(), With<PanelPopulated>>,
    state: Res<VaelState>,
    panel: Res<PanelState>,
) {
    let stale = state.is_changed() || panel.is_changed();
    super::rebuild_panel(
        &mut commands,
        roots.iter(),
        &populated,
        stale,
        &state,
        &panel,
        build,
    );
}

pub fn build(world: &mut World, root: Entity, state: &VaelState, panel: &PanelState) {
    let top = toolbar(world, root);
    heading(world, top, "Diálogos");

    let files: Vec<String> = state.dialogues.keys().cloned().collect();
    if files.is_empty() {
        label(world, root, "Sem ficheiros em dialogues/.", true);
        return;
    }
    let selected_file = panel
        .dialogue_file
        .clone()
        .filter(|file| state.dialogues.contains_key(file))
        .unwrap_or_else(|| files[0].clone());
    choice_field(
        world,
        top,
        "ficheiro",
        ChoiceFieldBinding {
            panel: PanelId::Dialogues,
            field: FieldId::File,
            key: String::new(),
            sub: 0,
        },
        files
            .iter()
            .map(|file| (file.clone(), file.clone()))
            .collect(),
        files
            .iter()
            .position(|file| file == &selected_file)
            .unwrap_or(0),
    );
    action_button(
        world,
        top,
        "Novo nó",
        "vael.content.add",
        &[("panel", "dialogues".to_string())],
    );
    action_button(
        world,
        top,
        "Gravar",
        "vael.content.save",
        &[("panel", "dialogues".to_string())],
    );
    action_button(world, top, "Recarregar", "vael.content.reload", &[]);
    action_button(world, top, "Validar", "vael.content.validate", &[]);
    if !panel.status.is_empty() {
        status_line(world, root, panel.status.clone());
    }

    let Some(dialogue) = state.dialogues.get(&selected_file) else {
        label(world, root, "Ficheiro não carregado.", true);
        return;
    };
    let (list, form) = body(world, root);

    let mut nodes: Vec<(&String, &DialogueNode)> = dialogue.nodes.iter().collect();
    nodes.sort_by(|a, b| a.0.cmp(b.0));
    let selected_node = panel
        .dialogue_node
        .clone()
        .filter(|node| dialogue.nodes.contains_key(node))
        .or_else(|| nodes.first().map(|(id, _)| (*id).clone()));
    for (id, node) in &nodes {
        list_button(
            world,
            list,
            format!("{} · {}", id, first_chars(&node.text, 40)),
            selected_node.as_deref() == Some(id.as_str()),
            "vael.content.select",
            id,
        );
    }

    text_field(
        world,
        form,
        "start",
        TextFieldBinding {
            panel: PanelId::Dialogues,
            field: FieldId::Start,
            key: String::new(),
            sub: 0,
        },
        &dialogue.start,
        false,
        false,
    );
    separator(world, form);

    let Some(node_id) = selected_node else {
        label(world, form, "Seleciona um nó.", true);
        return;
    };
    let Some(node) = dialogue.nodes.get(&node_id) else {
        return;
    };
    let bind = |field: FieldId| TextFieldBinding {
        panel: PanelId::Dialogues,
        field,
        key: node_id.clone(),
        sub: 0,
    };
    text_field(
        world,
        form,
        "id",
        bind(FieldId::NodeId),
        &node_id,
        false,
        false,
    );
    text_field(
        world,
        form,
        "speaker",
        bind(FieldId::NodeSpeaker),
        &node.speaker,
        false,
        false,
    );
    text_field(
        world,
        form,
        "text",
        bind(FieldId::NodeText),
        &node.text,
        false,
        true,
    );
    text_field(
        world,
        form,
        "voice",
        bind(FieldId::NodeVoice),
        node.voice.clone().unwrap_or_default(),
        false,
        false,
    );
    text_field(
        world,
        form,
        "portrait",
        bind(FieldId::NodePortrait),
        node.portrait.clone().unwrap_or_default(),
        false,
        false,
    );

    separator(world, form);
    label(world, form, "Escolhas", false);
    for (index, choice) in node.choices.iter().enumerate() {
        let target = if choice.goto.is_empty() {
            "(fim)".to_string()
        } else {
            choice.goto.clone()
        };
        index_button(
            world,
            form,
            format!("{index}. {} → {target}", first_chars(&choice.text, 40)),
            panel.dialogue_choice == index,
            "vael.dialogue.select_choice",
            index,
        );
    }
    action_button(
        world,
        form,
        "Nova escolha",
        "vael.dialogue.add_choice",
        &[("node", node_id.clone())],
    );

    if let Some(choice) = node.choices.get(panel.dialogue_choice) {
        let choice_bind = |field: FieldId| TextFieldBinding {
            panel: PanelId::Dialogues,
            field,
            key: node_id.clone(),
            sub: panel.dialogue_choice,
        };
        text_field(
            world,
            form,
            "texto da escolha",
            choice_bind(FieldId::ChoiceText),
            &choice.text,
            false,
            false,
        );
        text_field(
            world,
            form,
            "goto (vazio = fim)",
            choice_bind(FieldId::ChoiceGoto),
            &choice.goto,
            false,
            false,
        );
        action_button(
            world,
            form,
            if choice.quest_accept {
                "quest_accept: sim"
            } else {
                "quest_accept: não"
            },
            "vael.content.toggle",
            &[
                ("panel", "dialogues".to_string()),
                ("id", node_id.clone()),
                ("field", "quest_accept".to_string()),
                ("index", panel.dialogue_choice.to_string()),
            ],
        );
        action_button(
            world,
            form,
            if choice.quest_turn_in {
                "quest_turn_in: sim"
            } else {
                "quest_turn_in: não"
            },
            "vael.content.toggle",
            &[
                ("panel", "dialogues".to_string()),
                ("id", node_id.clone()),
                ("field", "quest_turn_in".to_string()),
                ("index", panel.dialogue_choice.to_string()),
            ],
        );
        action_button(
            world,
            form,
            "Apagar escolha",
            "vael.dialogue.delete_choice",
            &[
                ("node", node_id.clone()),
                ("index", panel.dialogue_choice.to_string()),
            ],
        );
    }

    separator(world, form);
    let row = button_row(world, form);
    action_button(
        world,
        row,
        if panel.delete_armed.as_deref() == Some(node_id.as_str()) {
            "Confirmar apagar nó"
        } else {
            "Apagar nó"
        },
        "vael.content.delete",
        &[("panel", "dialogues".to_string()), ("id", node_id.clone())],
    );
}

fn first_chars(text: &str, max: usize) -> String {
    let mut chars = text.chars();
    let prefix: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

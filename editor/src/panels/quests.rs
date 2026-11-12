//! Painel de missões (`quests/*.json` + `quests/index.json`).

use bevy::prelude::*;

use vael_content::QuestDef;
use vael_content::validate::{OBJECTIVE_TYPES, QUEST_BIOMES};

use crate::state::VaelState;
use crate::ui::{
    ChoiceFieldBinding, FieldId, PanelId, PanelPopulated, PanelState, TextFieldBinding,
    action_button, body, button_row, choice_field, float_field, heading, label, list_button,
    separator, status_line, text_field, toolbar,
};

#[derive(Component)]
pub struct QuestsRoot;

pub fn rebuild(
    mut commands: Commands,
    roots: Query<Entity, With<QuestsRoot>>,
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
    heading(world, top, "Missões");

    if state.quest_files.is_empty() {
        label(world, root, "Sem ficheiros em quests/.", true);
        return;
    }
    let selected_file = panel
        .quest_file
        .clone()
        .filter(|file| state.quest_files.iter().any(|entry| &entry.file == file))
        .unwrap_or_else(|| state.quest_files[0].file.clone());
    choice_field(
        world,
        top,
        "ficheiro",
        ChoiceFieldBinding {
            panel: PanelId::Quests,
            field: FieldId::QuestFile,
            key: String::new(),
            sub: 0,
        },
        state
            .quest_files
            .iter()
            .map(|file| (file.file.clone(), file.file.clone()))
            .collect(),
        state
            .quest_files
            .iter()
            .position(|file| file.file == selected_file)
            .unwrap_or(0),
    );
    text_field(
        world,
        top,
        "buscar",
        TextFieldBinding {
            panel: PanelId::Quests,
            field: FieldId::QuestSearch,
            key: String::new(),
            sub: 0,
        },
        &panel.quest_search,
        false,
        false,
    );
    action_button(
        world,
        top,
        "Nova",
        "vael.content.add",
        &[("panel", "quests".to_string())],
    );
    action_button(
        world,
        top,
        "Gravar",
        "vael.content.save",
        &[("panel", "quests".to_string())],
    );
    action_button(world, top, "Recarregar", "vael.content.reload", &[]);
    action_button(world, top, "Validar", "vael.content.validate", &[]);
    if !panel.status.is_empty() {
        status_line(world, root, panel.status.clone());
    }
    let problems = state.findings_for("quest-ref");
    if !problems.is_empty() {
        status_line(
            world,
            root,
            format!(
                "{} problema(s) de quests (ver painel Conteúdo)",
                problems.len()
            ),
        );
    }

    let Some(file) = state
        .quest_files
        .iter()
        .find(|file| file.file == selected_file)
    else {
        return;
    };
    let (list, form) = body(world, root);

    let search = panel.quest_search.to_lowercase();
    let mut quests: Vec<&QuestDef> = file
        .quests
        .iter()
        .filter(|quest| {
            search.is_empty()
                || quest.id.to_lowercase().contains(&search)
                || quest.title.to_lowercase().contains(&search)
                || quest.npc.to_lowercase().contains(&search)
        })
        .collect();
    quests.sort_by(|a, b| a.id.cmp(&b.id));
    let selected = panel
        .quest
        .clone()
        .filter(|id| file.quests.iter().any(|quest| &quest.id == id))
        .or_else(|| quests.first().map(|quest| quest.id.clone()));
    for quest in &quests {
        list_button(
            world,
            list,
            format!("{} · {}", quest.title, quest.id),
            selected.as_deref() == Some(quest.id.as_str()),
            "vael.content.select",
            &quest.id,
        );
    }

    let Some(quest) = quests
        .iter()
        .find(|quest| selected.as_deref() == Some(quest.id.as_str()))
        .copied()
    else {
        label(world, form, "Seleciona uma missão.", true);
        return;
    };
    let key = quest.id.clone();
    let bind = |field: FieldId| TextFieldBinding {
        panel: PanelId::Quests,
        field,
        key: key.clone(),
        sub: 0,
    };

    text_field(
        world,
        form,
        "id",
        bind(FieldId::QuestId),
        &quest.id,
        false,
        false,
    );
    text_field(
        world,
        form,
        "npc (nome da entidade)",
        bind(FieldId::Npc),
        &quest.npc,
        false,
        false,
    );
    text_field(
        world,
        form,
        "title",
        bind(FieldId::Title),
        &quest.title,
        false,
        false,
    );
    choice_field(
        world,
        form,
        "biome",
        ChoiceFieldBinding {
            panel: PanelId::Quests,
            field: FieldId::Biome,
            key: key.clone(),
            sub: 0,
        },
        super::options(QUEST_BIOMES),
        super::selected_index(QUEST_BIOMES, &quest.biome),
    );

    separator(world, form);
    label(world, form, "Objetivo", false);
    choice_field(
        world,
        form,
        "type",
        ChoiceFieldBinding {
            panel: PanelId::Quests,
            field: FieldId::ObjectiveType,
            key: key.clone(),
            sub: 0,
        },
        super::options(OBJECTIVE_TYPES),
        super::selected_index(OBJECTIVE_TYPES, &quest.objective.objective_type),
    );
    text_field(
        world,
        form,
        "target",
        bind(FieldId::ObjectiveTarget),
        &quest.objective.target,
        false,
        false,
    );
    text_field(
        world,
        form,
        "count",
        bind(FieldId::ObjectiveCount),
        quest.objective.count.to_string(),
        true,
        false,
    );
    float_field(
        world,
        form,
        "radius",
        bind(FieldId::ObjectiveRadius),
        super::compact(quest.objective.radius),
    );
    text_field(
        world,
        form,
        "título do objetivo",
        bind(FieldId::ObjectiveTitle),
        &quest.objective.title,
        false,
        false,
    );

    separator(world, form);
    label(world, form, "Recompensa", false);
    text_field(
        world,
        form,
        "gold",
        bind(FieldId::RewardGold),
        quest.rewards.gold.to_string(),
        true,
        false,
    );
    text_field(
        world,
        form,
        "xp",
        bind(FieldId::RewardXp),
        quest.rewards.xp.to_string(),
        true,
        false,
    );
    text_field(
        world,
        form,
        "items (id:count por linha)",
        bind(FieldId::RewardItems),
        super::lines_text(&quest.rewards.items),
        false,
        true,
    );

    separator(world, form);
    label(world, form, "Texto", false);
    text_field(
        world,
        form,
        "lines_intro",
        bind(FieldId::LinesIntro),
        super::lines_text(&quest.lines_intro),
        false,
        true,
    );
    text_field(
        world,
        form,
        "lines_progress",
        bind(FieldId::LinesProgress),
        super::lines_text(&quest.lines_progress),
        false,
        true,
    );
    text_field(
        world,
        form,
        "lines_complete",
        bind(FieldId::LinesComplete),
        super::lines_text(&quest.lines_complete),
        false,
        true,
    );
    text_field(
        world,
        form,
        "lines_locked",
        bind(FieldId::LinesLocked),
        super::lines_text(&quest.lines_locked),
        false,
        true,
    );
    text_field(
        world,
        form,
        "portrait",
        bind(FieldId::QuestPortrait),
        quest.portrait.clone().unwrap_or_default(),
        false,
        false,
    );
    text_field(
        world,
        form,
        "voice",
        bind(FieldId::QuestVoice),
        quest.voice.clone().unwrap_or_default(),
        false,
        false,
    );

    separator(world, form);
    label(world, form, "Cadeia", false);
    text_field(
        world,
        form,
        "requires (ids por linha)",
        bind(FieldId::Requires),
        super::lines_text(&quest.requires),
        false,
        true,
    );
    text_field(
        world,
        form,
        "next",
        bind(FieldId::Next),
        &quest.next,
        false,
        false,
    );

    separator(world, form);
    label(world, form, "Parley (NLU)", false);
    let parley = quest.parley.clone().unwrap_or_default();
    let report = parley.report.clone().unwrap_or_default();
    text_field(
        world,
        form,
        "prompt",
        bind(FieldId::ParleyPrompt),
        &report.prompt,
        false,
        false,
    );
    text_field(
        world,
        form,
        "example",
        bind(FieldId::ParleyExample),
        &report.example,
        false,
        false,
    );
    text_field(
        world,
        form,
        "hint",
        bind(FieldId::ParleyHint),
        &report.hint,
        false,
        false,
    );
    text_field(
        world,
        form,
        "expect (kind:id por linha)",
        bind(FieldId::ParleyExpect),
        super::expects_text(&report.expect),
        false,
        true,
    );
    action_button(
        world,
        form,
        if parley.partial {
            "partial: sim"
        } else {
            "partial: não"
        },
        "vael.content.toggle",
        &[
            ("panel", "quests".to_string()),
            ("id", key.clone()),
            ("field", "parley_partial".to_string()),
        ],
    );

    separator(world, form);
    text_field(
        world,
        form,
        "stages (JSON; vazio = sem etapas)",
        bind(FieldId::StagesJson),
        serde_json::to_string_pretty(&quest.stages).unwrap_or_default(),
        false,
        true,
    );

    separator(world, form);
    let row = button_row(world, form);
    action_button(
        world,
        row,
        if panel.delete_armed.as_deref() == Some(quest.id.as_str()) {
            "Confirmar apagar"
        } else {
            "Apagar missão"
        },
        "vael.content.delete",
        &[("panel", "quests".to_string()), ("id", key.clone())],
    );
}

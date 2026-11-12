//! Painel de marcos (`data/landmarks.json`).

use bevy::prelude::*;

use vael_content::LandmarkDef;
use vael_content::validate::LANDMARK_BIOMES;

use crate::state::VaelState;
use crate::ui::{
    ChoiceFieldBinding, FieldId, PanelId, PanelPopulated, PanelState, TextFieldBinding,
    action_button, body, button_row, choice_field, float_field, heading, label, list_button,
    separator, status_line, text_field, toolbar,
};

#[derive(Component)]
pub struct LandmarksRoot;

pub fn rebuild(
    mut commands: Commands,
    roots: Query<Entity, With<LandmarksRoot>>,
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
    heading(world, top, "Marcos");
    action_button(
        world,
        top,
        "Novo",
        "vael.content.add",
        &[("panel", "landmarks".to_string())],
    );
    action_button(
        world,
        top,
        "Gravar",
        "vael.content.save",
        &[("panel", "landmarks".to_string())],
    );
    action_button(world, top, "Recarregar", "vael.content.reload", &[]);
    action_button(world, top, "Validar", "vael.content.validate", &[]);
    if !panel.status.is_empty() {
        status_line(world, root, panel.status.clone());
    }
    let (list, form) = body(world, root);

    let mut landmarks: Vec<&LandmarkDef> = state.landmarks.landmarks.iter().collect();
    landmarks.sort_by(|a, b| a.name.cmp(&b.name));
    let selected = panel
        .landmark
        .as_deref()
        .or_else(|| landmarks.first().map(|landmark| landmark.name.as_str()))
        .map(str::to_string);
    for landmark in &landmarks {
        list_button(
            world,
            list,
            format!("{} · {}", landmark.name, landmark.label),
            selected.as_deref() == Some(landmark.name.as_str()),
            "vael.content.select",
            &landmark.name,
        );
    }

    let Some(landmark) = landmarks
        .iter()
        .find(|landmark| selected.as_deref() == Some(landmark.name.as_str()))
        .copied()
    else {
        label(world, form, "Seleciona um marco.", true);
        return;
    };
    let key = landmark.name.clone();
    let bind = |field: FieldId| TextFieldBinding {
        panel: PanelId::Landmarks,
        field,
        key: key.clone(),
        sub: 0,
    };

    text_field(
        world,
        form,
        "name",
        bind(FieldId::Name),
        &landmark.name,
        false,
        false,
    );
    text_field(
        world,
        form,
        "label",
        bind(FieldId::Label),
        &landmark.label,
        false,
        false,
    );
    choice_field(
        world,
        form,
        "biome",
        ChoiceFieldBinding {
            panel: PanelId::Landmarks,
            field: FieldId::Biome,
            key: key.clone(),
            sub: 0,
        },
        super::options(LANDMARK_BIOMES),
        super::selected_index(LANDMARK_BIOMES, &landmark.biome),
    );
    float_field(
        world,
        form,
        "radius",
        bind(FieldId::Radius),
        super::compact(landmark.radius),
    );
    text_field(
        world,
        form,
        "survey (quest id)",
        bind(FieldId::Survey),
        landmark.survey.clone().unwrap_or_default(),
        false,
        false,
    );
    action_button(
        world,
        form,
        if landmark.pilgrim {
            "peregrino: sim"
        } else {
            "peregrino: não"
        },
        "vael.content.toggle",
        &[
            ("panel", "landmarks".to_string()),
            ("id", key.clone()),
            ("field", "pilgrim".to_string()),
        ],
    );

    separator(world, form);
    let row = button_row(world, form);
    action_button(
        world,
        row,
        if panel.delete_armed.as_deref() == Some(landmark.name.as_str()) {
            "Confirmar apagar"
        } else {
            "Apagar"
        },
        "vael.content.delete",
        &[("panel", "landmarks".to_string()), ("id", key)],
    );
}

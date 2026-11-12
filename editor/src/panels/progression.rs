//! Painel de progressão (`data/progression.json`).

use bevy::prelude::*;

use vael_content::SkillDef;

use crate::state::VaelState;
use crate::ui::{
    FieldId, PanelId, PanelPopulated, PanelState, TextFieldBinding, action_button, body,
    button_row, field_row, float_field, heading, label, list_button, raw_text_field, separator,
    status_line, text_field, toolbar,
};

#[derive(Component)]
pub struct ProgressionRoot;

pub fn rebuild(
    mut commands: Commands,
    roots: Query<Entity, With<ProgressionRoot>>,
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
    heading(world, top, "Progressão");
    action_button(
        world,
        top,
        "Novo talento",
        "vael.content.add",
        &[("panel", "progression".to_string())],
    );
    action_button(
        world,
        top,
        "Gravar",
        "vael.content.save",
        &[("panel", "progression".to_string())],
    );
    action_button(world, top, "Recarregar", "vael.content.reload", &[]);
    action_button(world, top, "Validar", "vael.content.validate", &[]);
    if !panel.status.is_empty() {
        status_line(world, root, panel.status.clone());
    }
    let (list, form) = body(world, root);

    let mut skills: Vec<&SkillDef> = state.progression.skills.iter().collect();
    skills.sort_by(|a, b| a.id.cmp(&b.id));
    let selected = panel
        .skill
        .as_deref()
        .or_else(|| skills.first().map(|skill| skill.id.as_str()))
        .map(str::to_string);
    for skill in &skills {
        list_button(
            world,
            list,
            format!("{} · {}", skill.id, skill.label),
            selected.as_deref() == Some(skill.id.as_str()),
            "vael.content.select",
            &skill.id,
        );
    }

    // Curva de XP.
    let xp = &state.progression.xp;
    text_field(
        world,
        form,
        "xp.base",
        TextFieldBinding {
            panel: PanelId::Progression,
            field: FieldId::XpBase,
            key: String::new(),
            sub: 0,
        },
        xp.base.to_string(),
        true,
        false,
    );
    float_field(
        world,
        form,
        "xp.growth",
        TextFieldBinding {
            panel: PanelId::Progression,
            field: FieldId::XpGrowth,
            key: String::new(),
            sub: 0,
        },
        super::compact(xp.growth),
    );
    text_field(
        world,
        form,
        "xp.skill_points_per_level",
        TextFieldBinding {
            panel: PanelId::Progression,
            field: FieldId::XpSkillPoints,
            key: String::new(),
            sub: 0,
        },
        xp.skill_points_per_level.to_string(),
        true,
        false,
    );
    text_field(
        world,
        form,
        "song_seed_talent_every",
        TextFieldBinding {
            panel: PanelId::Progression,
            field: FieldId::SongSeedEvery,
            key: String::new(),
            sub: 0,
        },
        state.progression.song_seed_talent_every.to_string(),
        true,
        false,
    );

    separator(world, form);
    label(world, form, "XP por criatura (kill_xp)", false);
    for (creature, value) in &state.progression.kill_xp {
        let row = field_row(world, form, creature);
        raw_text_field(
            world,
            row,
            TextFieldBinding {
                panel: PanelId::Progression,
                field: FieldId::KillXpLine,
                key: creature.clone(),
                sub: 0,
            },
            format!("{creature}:{value}"),
            false,
            false,
        );
        action_button(
            world,
            row,
            "remover",
            "vael.progression.remove_kill_xp",
            &[("id", creature.clone())],
        );
    }
    action_button(
        world,
        form,
        "Nova criatura",
        "vael.progression.add_kill_xp",
        &[],
    );

    separator(world, form);
    let Some(skill) = skills
        .iter()
        .find(|skill| selected.as_deref() == Some(skill.id.as_str()))
        .copied()
    else {
        label(world, form, "Seleciona um talento.", true);
        return;
    };
    let key = skill.id.clone();
    let bind = |field: FieldId| TextFieldBinding {
        panel: PanelId::Progression,
        field,
        key: key.clone(),
        sub: 0,
    };
    super::read_only(world, form, "id", &skill.id);
    text_field(
        world,
        form,
        "label",
        bind(FieldId::Label),
        &skill.label,
        false,
        false,
    );
    text_field(
        world,
        form,
        "requires",
        bind(FieldId::Requires),
        skill.requires.clone().unwrap_or_default(),
        false,
        false,
    );
    text_field(
        world,
        form,
        "excludes",
        bind(FieldId::Excludes),
        skill.excludes.clone().unwrap_or_default(),
        false,
        false,
    );
    text_field(
        world,
        form,
        "effects (efeito:valor por linha)",
        bind(FieldId::Effects),
        super::effects_text(&skill.effects),
        false,
        true,
    );

    let row = button_row(world, form);
    action_button(
        world,
        row,
        if panel.delete_armed.as_deref() == Some(skill.id.as_str()) {
            "Confirmar apagar"
        } else {
            "Apagar talento"
        },
        "vael.content.delete",
        &[("panel", "progression".to_string()), ("id", key)],
    );
}

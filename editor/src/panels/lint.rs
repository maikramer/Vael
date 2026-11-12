//! Painel de validação cruzada do conteúdo.

use bevy::prelude::*;

use vael_content::Severity;

use crate::state::VaelState;
use crate::ui::{
    PanelPopulated, PanelState, action_button, heading, label, scroll_column, status_line, toolbar,
};

#[derive(Component)]
pub struct LintRoot;

pub fn rebuild(
    mut commands: Commands,
    roots: Query<Entity, With<LintRoot>>,
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
    heading(world, top, "Conteúdo");
    action_button(world, top, "Validar tudo", "vael.content.validate", &[]);
    action_button(world, top, "Recarregar", "vael.content.reload", &[]);
    if !panel.status.is_empty() {
        status_line(world, root, panel.status.clone());
    }

    let findings = state.findings();
    let errors = findings
        .iter()
        .filter(|finding| finding.severity == Severity::Error)
        .count();
    let warnings = findings.len().saturating_sub(errors);
    status_line(
        world,
        root,
        format!(
            "{errors} erro(s), {warnings} aviso(s) · mundo: {} ficheiro(s), {} nome(s) · {} erro(s) de carga",
            state.world.files,
            state.world.names.len(),
            state.errors.len(),
        ),
    );

    let list = scroll_column(world, root, percent(100));
    if findings.is_empty() {
        label(world, list, "Sem problemas de conteúdo.", true);
    }
    for finding in &findings {
        label(
            world,
            list,
            format!(
                "[{}] {}: {}",
                finding.severity.label(),
                finding.kind,
                finding.message
            ),
            finding.severity == Severity::Warning,
        );
    }

    if !state.errors.is_empty() {
        heading(world, list, "Falhas de carregamento");
        for error in &state.errors {
            label(world, list, error.clone(), true);
        }
    }
    if !state.world.errors.is_empty() {
        heading(world, list, "Falhas do índice do mundo");
        for error in &state.world.errors {
            label(world, list, error.clone(), true);
        }
    }
}

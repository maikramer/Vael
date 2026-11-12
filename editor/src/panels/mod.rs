//! Painéis de autoria do Vael no dock do Jackdaw.
//!
//! Cada painel é uma janela própria. O build inicial e os rebuilds passam
//! pelo mesmo `build`; os sistemas só reconstroem quando o estado mudou ou a
//! janela acabou de abrir.

pub mod dialogues;
pub mod items;
pub mod landmarks;
pub mod lint;
pub mod progression;
pub mod quests;
pub mod scenes;

use std::collections::BTreeMap;

use bevy::prelude::*;
use jackdaw_editor_core::prelude::AppState;

use crate::state::VaelState;
use crate::ui::{PanelPopulated, PanelState};

pub struct VaelPanelsPlugin;

impl Plugin for VaelPanelsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                items::rebuild,
                landmarks::rebuild,
                progression::rebuild,
                dialogues::rebuild,
                quests::rebuild,
                lint::rebuild,
                scenes::rebuild,
            )
                .run_if(in_state(AppState::Editor)),
        );
    }
}

type BuildFn = fn(&mut World, Entity, &VaelState, &PanelState);

/// Rebuild genérico: limpa os filhos da raiz e volta a construir o painel.
pub fn rebuild_panel(
    commands: &mut Commands,
    roots: impl Iterator<Item = Entity>,
    populated: &Query<(), With<PanelPopulated>>,
    stale: bool,
    state: &VaelState,
    panel: &PanelState,
    build: BuildFn,
) {
    for root in roots {
        if !stale && populated.contains(root) {
            continue;
        }
        let state = state.clone();
        let panel = panel.clone();
        commands.queue(move |world: &mut World| {
            // A janela pode ter sido fechada entre o rebuild e a fila.
            if world.get_entity(root).is_err() {
                return;
            }
            world.entity_mut(root).despawn_children();
            build(world, root, &state, &panel);
            if let Ok(mut entity) = world.get_entity_mut(root) {
                entity.insert(PanelPopulated);
            }
        });
    }
}

/// Opções de combobox a partir de valores fixos (label = valor).
pub fn options(values: &[&str]) -> Vec<(String, String)> {
    values
        .iter()
        .map(|value| ((*value).to_string(), (*value).to_string()))
        .collect()
}

pub fn selected_index(values: &[&str], current: &str) -> usize {
    values
        .iter()
        .position(|value| *value == current)
        .unwrap_or(0)
}

pub fn lines_text(lines: &[String]) -> String {
    lines.join("\n")
}

pub fn effects_text(effects: &BTreeMap<String, f64>) -> String {
    effects
        .iter()
        .map(|(key, value)| format!("{key}:{}", compact(*value)))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn expects_text(expects: &[vael_content::Expect]) -> String {
    expects
        .iter()
        .map(|expect| format!("{}:{}", expect.kind, expect.id))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn compact(value: f64) -> String {
    if value.is_finite() && value.fract() == 0.0 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// Linha só de leitura, para ids.
pub fn read_only(world: &mut World, parent: Entity, label: &str, value: &str) {
    let row = crate::ui::field_row(world, parent, label);
    crate::ui::label(world, row, value.to_string(), false);
}

/// Status do painel no topo.
pub fn status(world: &mut World, root: Entity, panel: &PanelState) {
    if !panel.status.is_empty() {
        crate::ui::status_line(world, root, panel.status.clone());
    }
}

//! Painel de cenas: mundo principal, menu e áreas internas.
//!
//! O Jackdaw não tem um conceito de "área interna"; no Vael uma sala é um
//! `.bsn` próprio em `scenes/interiors/`. Este painel lista-os e abre cada um
//! como aba de cena do Jackdaw (`scene.open`).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use bevy::prelude::*;

use crate::state::{VaelRoot, VaelState};
use crate::ui::{
    PanelPopulated, PanelState, action_button, heading, label, scroll_column, separator,
    status_line, toolbar,
};

#[derive(Component)]
pub struct ScenesRoot;

pub fn rebuild(
    mut commands: Commands,
    roots: Query<Entity, With<ScenesRoot>>,
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

pub fn build(world: &mut World, root: Entity, _state: &VaelState, panel: &PanelState) {
    let top = toolbar(world, root);
    heading(world, top, "Cenas");
    action_button(world, top, "Recarregar", "vael.content.reload", &[]);
    if !panel.status.is_empty() {
        status_line(world, root, panel.status.clone());
    }

    let project_root = world.get_resource::<VaelRoot>().map(|root| root.0.clone());
    let Some(project_root) = project_root else {
        label(world, root, "Sem raiz de projeto.", true);
        return;
    };

    let list = scroll_column(world, root, percent(100));
    let world_scene = first_existing(&project_root, &["world.bsn", "world.xml"]);
    if let Some(path) = world_scene {
        label(world, list, "Mundo", false);
        scene_button(world, list, "Abrir mundo", &path);
        separator(world, list);
    }

    let menu = first_existing(&project_root, &["scenes/menu.bsn", "scenes/menu.xml"]);
    if let Some(path) = menu {
        label(world, list, "Menu", false);
        scene_button(world, list, "Abrir menu", &path);
        separator(world, list);
    }

    let rooms = interior_scenes(&project_root.join("scenes/interiors"));
    if rooms.is_empty() {
        label(world, list, "Sem interiores em scenes/interiors/.", true);
    } else {
        label(world, list, "Áreas internas", false);
        for (name, path) in rooms {
            scene_button(world, list, format!("Abrir {name}"), &path);
        }
    }
}

fn scene_button(world: &mut World, parent: Entity, text: impl Into<String>, path: &Path) {
    action_button(
        world,
        parent,
        text,
        "scene.open",
        &[("path", path.display().to_string())],
    );
}

fn first_existing(root: &Path, candidates: &[&str]) -> Option<PathBuf> {
    candidates
        .iter()
        .map(|rel| root.join(rel))
        .find(|path| path.is_file())
}

fn interior_scenes(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut scenes: BTreeMap<String, PathBuf> = BTreeMap::new();
    for path in entries.flatten().map(|entry| entry.path()) {
        if !path.is_file() {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("");
        let is_bsn = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("bsn"));
        let is_xml = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("xml"));
        if !is_bsn && !is_xml {
            continue;
        }
        match scenes.get(name) {
            Some(_) if !is_bsn => {}
            _ => {
                scenes.insert(name.to_string(), path);
            }
        }
    }
    scenes.into_iter().collect()
}

//! Extensão do Vael para o editor Jackdaw.
//!
//! A extensão regista o `WorldNode` do mundo/cenas `.bsn`, os painéis de
//! conteúdo do Vael, os operadores que gravam os JSON e o carregamento
//! inicial a partir da raiz do projeto.

use bevy::prelude::*;
use bevy_world_xml::bsn::{WorldNode, WorldProp};
use jackdaw_editor_core::prelude::*;

use crate::fields::VaelFieldsPlugin;
use crate::ops::*;
use crate::panels::VaelPanelsPlugin;
use crate::panels::dialogues::DialoguesRoot;
use crate::panels::items::ItemsRoot;
use crate::panels::landmarks::LandmarksRoot;
use crate::panels::lint::LintRoot;
use crate::panels::progression::ProgressionRoot;
use crate::panels::quests::QuestsRoot;
use crate::panels::scenes::ScenesRoot;
use crate::state::{VaelRoot, VaelState};
use crate::ui::{PanelState, panel_root};

/// Extensão zero-sized (o `ExtensionPlugin` do Jackdaw exige `ZST`).
#[derive(Default)]
pub struct VaelExtension;

impl JackdawExtension for VaelExtension {
    fn id(&self) -> String {
        "vael".to_string()
    }

    fn label(&self) -> String {
        "Vael".to_string()
    }

    fn description(&self) -> String {
        "Autoria do conteúdo do Vael (quests, diálogos, itens, marcos, progressão)".to_string()
    }

    fn register(&self, ctx: &mut ExtensionRegistrar<'_>) {
        ctx.init_resource::<VaelState>();
        ctx.init_resource::<PanelState>();

        ctx.register_operator::<SelectContentOp>()
            .register_operator::<SelectFileOp>()
            .register_operator::<AddContentOp>()
            .register_operator::<DeleteContentOp>()
            .register_operator::<SaveContentOp>()
            .register_operator::<SaveAllOp>()
            .register_operator::<ReloadContentOp>()
            .register_operator::<ValidateContentOp>()
            .register_operator::<ToggleContentOp>()
            .register_operator::<AddChoiceOp>()
            .register_operator::<DeleteChoiceOp>()
            .register_operator::<SelectChoiceOp>()
            .register_operator::<AddKillXpOp>()
            .register_operator::<RemoveKillXpOp>();

        ctx.register_menu_entry::<ReloadContentOp>(TopLevelMenu::Tools)
            .register_menu_entry::<SaveAllOp>(TopLevelMenu::Tools)
            .register_menu_entry::<ValidateContentOp>(TopLevelMenu::Tools);

        ctx.register_window(
            WindowDescriptor::new("vael.scenes")
                .with_name("Vael: Cenas")
                .with_default_area(DefaultArea::BottomDock)
                .with_priority(39)
                .with_build(|window| {
                    let parent = window.target_entity();
                    let _ = panel_root(window.world_mut(), parent, ScenesRoot);
                }),
        );
        ctx.register_window(
            WindowDescriptor::new("vael.items")
                .with_name("Vael: Itens")
                .with_default_area(DefaultArea::BottomDock)
                .with_priority(40)
                .with_build(|window| {
                    let parent = window.target_entity();
                    let _ = panel_root(window.world_mut(), parent, ItemsRoot);
                }),
        );
        ctx.register_window(
            WindowDescriptor::new("vael.landmarks")
                .with_name("Vael: Marcos")
                .with_default_area(DefaultArea::BottomDock)
                .with_priority(41)
                .with_build(|window| {
                    let parent = window.target_entity();
                    let _ = panel_root(window.world_mut(), parent, LandmarksRoot);
                }),
        );
        ctx.register_window(
            WindowDescriptor::new("vael.progression")
                .with_name("Vael: Progressão")
                .with_default_area(DefaultArea::BottomDock)
                .with_priority(42)
                .with_build(|window| {
                    let parent = window.target_entity();
                    let _ = panel_root(window.world_mut(), parent, ProgressionRoot);
                }),
        );
        ctx.register_window(
            WindowDescriptor::new("vael.dialogues")
                .with_name("Vael: Diálogos")
                .with_default_area(DefaultArea::BottomDock)
                .with_priority(43)
                .with_build(|window| {
                    let parent = window.target_entity();
                    let _ = panel_root(window.world_mut(), parent, DialoguesRoot);
                }),
        );
        ctx.register_window(
            WindowDescriptor::new("vael.quests")
                .with_name("Vael: Missões")
                .with_default_area(DefaultArea::BottomDock)
                .with_priority(44)
                .with_build(|window| {
                    let parent = window.target_entity();
                    let _ = panel_root(window.world_mut(), parent, QuestsRoot);
                }),
        );
        ctx.register_window(
            WindowDescriptor::new("vael.lint")
                .with_name("Vael: Conteúdo")
                .with_default_area(DefaultArea::BottomDock)
                .with_priority(45)
                .with_build(|window| {
                    let parent = window.target_entity();
                    let _ = panel_root(window.world_mut(), parent, LintRoot);
                }),
        );
    }

    fn app_plugin(&self) -> Option<Box<dyn Plugin>> {
        Some(Box::new(VaelEditorPlugin))
    }
}

pub struct VaelEditorPlugin;

impl Plugin for VaelEditorPlugin {
    fn build(&self, app: &mut App) {
        // `WorldNode` é a ponte `.bsn`↔XML; sem estes tipos o inspetor não
        // mostra/edita os attrs do nó.
        app.register_type::<WorldNode>()
            .register_type::<WorldProp>();
        app.add_plugins((VaelFieldsPlugin, VaelPanelsPlugin));
        // `PostStartup`: o `ExtensionPlugin` regista a extensão (e os seus
        // resources) em `Startup`; correr antes via `ResMut` falhava.
        app.add_systems(PostStartup, load_content);
    }
}

/// Carrega o conteúdo do projeto aberto e escolhe a primeira entrada de cada
/// painel.
fn load_content(
    root: Option<Res<VaelRoot>>,
    state: Option<ResMut<VaelState>>,
    panel: Option<ResMut<PanelState>>,
) {
    let (Some(root), Some(mut state), Some(mut panel)) = (root, state, panel) else {
        warn!("vael: VaelRoot/VaelState ainda não existem; os painéis ficam vazios");
        return;
    };
    *state = VaelState::load(&root.0);
    panel.item = state.items.items.first().map(|item| item.id.clone());
    panel.landmark = state
        .landmarks
        .landmarks
        .first()
        .map(|landmark| landmark.name.clone());
    panel.skill = state
        .progression
        .skills
        .first()
        .map(|skill| skill.id.clone());
    panel.quest_file = state.quest_files.first().map(|file| file.file.clone());
    panel.quest = state
        .quest_files
        .first()
        .and_then(|file| file.quests.first())
        .map(|quest| quest.id.clone());
    panel.dialogue_file = state.dialogues.keys().next().cloned();
    panel.dialogue_node = panel
        .dialogue_file
        .as_ref()
        .and_then(|file| state.dialogues.get(file))
        .and_then(|dialogue| dialogue.nodes.keys().next().cloned());
    panel.status = format!(
        "{} quest(s), {} item(s), {} marco(s), {} diálogo(s); {} erro(s) de carregamento",
        state
            .quest_files
            .iter()
            .map(|file| file.quests.len())
            .sum::<usize>(),
        state.items.items.len(),
        state.landmarks.landmarks.len(),
        state.dialogues.len(),
        state.errors.len(),
    );
}

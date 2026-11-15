//! Extensão do Jackdaw para a autoria de conteúdo do Vael.
//!
//! Porta para o editor oficial os painéis que viviam na app `vael-editor`
//! (legado): **Itens**, **Marcos**, **Progressão**, **Diálogos**, **Missões**,
//! **Conteúdo** (validação cruzada) e **Cenas**. Os ficheiros editados são os
//! mesmos JSON que o runtime lê (`data/`, `quests/`, `dialogues/`).
//!
//! O estado (`VaelState`) carrega os ficheiros da raiz do projeto e é
//! partilhado pelos painéis; os botões só dispatcham operadores. Gravar é
//! atómico, deixa `.bak` e recusa alvos que falharam a carregar. A raiz é
//! resolvida pelo `.bsn` aberto no editor, por `VAEL_ROOT` ou pelo cwd.

mod fields;
mod io;
mod ops;
mod panels;
mod state;
mod ui;
mod world_index;

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use jackdaw_api::document::SceneFilePath;
use jackdaw_api::prelude::DefaultArea;
use jackdaw_extension::AppState;
use jackdaw_extension::prelude::*;

use panels::dialogues::DialoguesRoot;
use panels::items::ItemsRoot;
use panels::landmarks::LandmarksRoot;
use panels::lint::LintRoot;
use panels::progression::ProgressionRoot;
use panels::quests::QuestsRoot;
use panels::scenes::ScenesRoot;
use state::{VaelRoot, VaelState};
use ui::PanelState;

/// Id da extensão (tem de ser igual ao do manifesto do bundle).
pub const EXTENSION_ID: &str = "vael_content_jackdaw";

#[derive(Default)]
pub struct VaelContentExtension;

impl JackdawExtension for VaelContentExtension {
    fn id(&self) -> String {
        EXTENSION_ID.to_string()
    }

    fn label(&self) -> String {
        "Vael: Conteúdo".to_string()
    }

    fn description(&self) -> String {
        "Autoria do conteúdo do Vael no Jackdaw: itens, marcos, progressão, \
         diálogos, missões, validação cruzada e cenas"
            .to_string()
    }

    fn register(&self, ctx: &mut ExtensionRegistrar<'_>) {
        ctx.init_resource::<VaelState>()
            .init_resource::<PanelState>()
            .init_resource::<VaelRoot>();

        ctx.register_operator::<ops::SelectContentOp>()
            .register_operator::<ops::SelectFileOp>()
            .register_operator::<ops::AddContentOp>()
            .register_operator::<ops::DeleteContentOp>()
            .register_operator::<ops::SaveContentOp>()
            .register_operator::<ops::SaveAllOp>()
            .register_operator::<ops::ReloadContentOp>()
            .register_operator::<ops::ValidateContentOp>()
            .register_operator::<ops::ToggleContentOp>()
            .register_operator::<ops::AddChoiceOp>()
            .register_operator::<ops::DeleteChoiceOp>()
            .register_operator::<ops::SelectChoiceOp>()
            .register_operator::<ops::AddKillXpOp>()
            .register_operator::<ops::RemoveKillXpOp>();

        ctx.register_menu_entry::<ops::ReloadContentOp>(TopLevelMenu::Tools)
            .register_menu_entry::<ops::SaveAllOp>(TopLevelMenu::Tools)
            .register_menu_entry::<ops::ValidateContentOp>(TopLevelMenu::Tools);

        // As prioridades 40–43 já são do Problems/Profiler/Debugger/MCP;
        // os painéis de conteúdo ficam depois deles.
        for (id, name, priority, kind) in [
            ("vael.scenes", "Vael: Cenas", 50, PanelKind::Scenes),
            ("vael.items", "Vael: Itens", 51, PanelKind::Items),
            ("vael.landmarks", "Vael: Marcos", 52, PanelKind::Landmarks),
            (
                "vael.progression",
                "Vael: Progressão",
                53,
                PanelKind::Progression,
            ),
            ("vael.dialogues", "Vael: Diálogos", 54, PanelKind::Dialogues),
            ("vael.quests", "Vael: Missões", 55, PanelKind::Quests),
            ("vael.lint", "Vael: Conteúdo", 56, PanelKind::Lint),
        ] {
            ctx.register_window(
                WindowDescriptor::new(id)
                    .with_name(name)
                    .with_default_area(DefaultArea::BottomDock)
                    .with_priority(priority)
                    .with_build(move |window| {
                        let parent = window.target_entity();
                        kind.spawn(window.world_mut(), parent);
                    }),
            );
        }
    }

    fn app_plugin(&self) -> Option<Box<dyn Plugin>> {
        // Uma closure, não um `impl Plugin`: o `jd` trata um crate que declara
        // o seu próprio `Plugin` como jogo e não como extensão.
        Some(Box::new(|app: &mut App| {
            fields::app(app);
            panels::app(app);
            // `PostStartup`: o `ExtensionPlugin` regista a extensão (e os seus
            // resources) em `Startup`; correr antes via `ResMut` falhava.
            app.add_systems(PostStartup, load_content);
            // O `.bsn` aberto pode chegar depois do arranque (ou trocar de
            // projeto): tenta carregar até a raiz resolverse uma vez.
            app.add_systems(
                Update,
                ensure_content_loaded.run_if(in_state(AppState::Editor)),
            );
        }))
    }
}

/// Marcador de raiz de cada painel, escolhido no registo da janela.
#[derive(Clone, Copy)]
enum PanelKind {
    Scenes,
    Items,
    Landmarks,
    Progression,
    Dialogues,
    Quests,
    Lint,
}

impl PanelKind {
    fn spawn(self, world: &mut World, parent: Entity) {
        match self {
            Self::Scenes => {
                ui::panel_root(world, parent, ScenesRoot);
            }
            Self::Items => {
                ui::panel_root(world, parent, ItemsRoot);
            }
            Self::Landmarks => {
                ui::panel_root(world, parent, LandmarksRoot);
            }
            Self::Progression => {
                ui::panel_root(world, parent, ProgressionRoot);
            }
            Self::Dialogues => {
                ui::panel_root(world, parent, DialoguesRoot);
            }
            Self::Quests => {
                ui::panel_root(world, parent, QuestsRoot);
            }
            Self::Lint => {
                ui::panel_root(world, parent, LintRoot);
            }
        }
    }
}

fn load_content(world: &mut World) {
    let Some(root) = resolve_root(world) else {
        if let Some(mut panel) = world.get_resource_mut::<PanelState>() {
            panel.status = "Vael: abre o world.bsn do projeto para carregar o conteúdo".to_string();
        }
        return;
    };
    load_from(world, root);
}

fn ensure_content_loaded(world: &mut World) {
    if world
        .get_resource::<VaelState>()
        .is_some_and(|state| state.loaded)
    {
        return;
    }
    let Some(root) = resolve_root(world) else {
        return;
    };
    load_from(world, root);
}

/// Lê todos os JSON da raiz e escolhe a primeira entrada de cada painel.
fn load_from(world: &mut World, root: PathBuf) {
    let started = std::time::Instant::now();
    let state = VaelState::load(&root);
    info!(
        "vael: conteúdo carregado de {} em {:.1?}",
        root.display(),
        started.elapsed()
    );
    let counts = format!(
        "{} quest(s), {} item(s), {} marco(s), {} diálogo(s); {} erro(s) de carregamento; \
         índice do mundo sob pedido (Validar)",
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

    if let Some(mut panel) = world.get_resource_mut::<PanelState>() {
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
        panel.status = counts;
    }

    world.resource_mut::<VaelRoot>().0 = Some(root);
    world.insert_resource(state);
}

/// Raiz do projeto Vael: `VAEL_ROOT`, o `.bsn` aberto no editor, ou o cwd.
pub(crate) fn resolve_root(world: &World) -> Option<PathBuf> {
    if let Some(value) = std::env::var_os("VAEL_ROOT") {
        let path = PathBuf::from(value);
        if let Some(root) = find_project_root(&path) {
            return Some(root);
        }
        if path.join("data").is_dir() {
            return Some(path);
        }
    }

    if let Some(path) = world
        .get_resource::<SceneFilePath>()
        .and_then(|file| file.path.as_deref())
    {
        let path = PathBuf::from(path);
        let path = if path.is_absolute() {
            path
        } else {
            std::env::current_dir().ok()?.join(path)
        };
        if let Some(root) = find_project_root(&path) {
            return Some(root);
        }
    }

    std::env::current_dir()
        .ok()
        .and_then(|cwd| find_project_root(&cwd))
}

fn find_project_root(start: &Path) -> Option<PathBuf> {
    let start = if start.is_dir() {
        start
    } else {
        start.parent()?
    };
    start
        .ancestors()
        .find(|dir| dir.join("jackdaw.toml").is_file() && dir.join("world.bsn").is_file())
        .map(Path::to_path_buf)
}

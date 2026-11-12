//! Smoke tests do editor sem janela: extensão registada, conteúdo carregado,
//! índice do mundo e escrita atómica.

use std::path::{Path, PathBuf};

use bevy::state::app::AppExtStates;
use jackdaw_api_internal::lifecycle::ExtensionAppExt as _;
use jackdaw_editor_core::prelude::AppState;
use jackdaw_panels::WindowRegistry;
use jackdaw_panels::tree::{DockNode, DockTree};
use vael_editor::VaelExtension;
use vael_editor::state::{VaelRoot, VaelState};
use vael_editor::world_index::WorldIndex;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("editor/ tem de viver dentro do Vael")
        .to_path_buf()
}

#[test]
fn extensao_regista_janelas_e_operadores() {
    let mut app = jackdaw_editor_core::testing::editor_test_app_with(None, |app| {
        app.insert_resource(VaelRoot(root()));
        app.register_extension::<VaelExtension>();
    });
    app.update();

    let registry = app.world().resource::<WindowRegistry>();
    for id in [
        "vael.scenes",
        "vael.items",
        "vael.landmarks",
        "vael.progression",
        "vael.dialogues",
        "vael.quests",
        "vael.lint",
    ] {
        assert!(registry.get(id).is_some(), "janela `{id}` não registada");
    }

    let state = app.world().resource::<VaelState>();
    assert!(state.loaded, "o conteúdo não foi carregado no Startup");
    assert_eq!(
        state
            .quest_files
            .iter()
            .map(|file| file.quests.len())
            .sum::<usize>(),
        37,
        "quests esperadas: 37"
    );
    assert!(!state.items.items.is_empty());
    assert!(!state.landmarks.landmarks.is_empty());
    assert!(!state.dialogues.is_empty());
}

#[test]
fn indice_do_mundo_le_includes_e_cenas() {
    let index = WorldIndex::load(&root());
    assert!(
        index.names.contains("npc_desert_scorpions"),
        "npc de quest em falta no índice ({} nomes, {} erros: {:?})",
        index.names.len(),
        index.errors.len(),
        index.errors
    );
    assert!(index.names.contains("atalaia-do-vale"));
    assert!(index.files > 1, "world.bsn + includes + scenes");
    assert!(
        index.errors.is_empty(),
        "erros no índice do mundo: {:?}",
        index.errors
    );
}

#[test]
fn escrita_atomica_deixa_bak_e_roundtrip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("data/items.json");
    let value = serde_json::json!({ "version": 1, "items": [] });
    vael_editor::io::write_json_atomic(&path, &value).expect("primeira escrita");
    assert!(path.is_file());
    assert!(!path.with_file_name("items.json.bak").exists());

    vael_editor::io::write_json_atomic(&path, &value).expect("segunda escrita");
    assert!(
        path.with_file_name("items.json.bak").is_file(),
        "a segunda escrita tem de deixar .bak"
    );
    let read: serde_json::Value = vael_editor::io::read_json(&path).expect("ler");
    assert_eq!(read, value);
}

#[test]
fn layout_default_inclui_os_paineis_vael() {
    let mut app = jackdaw_editor_core::testing::editor_test_app_with(None, |app| {
        app.insert_resource(VaelRoot(root()));
        app.register_extension::<VaelExtension>();
    });
    // O editor de teste arranca em `ProjectSelect`; entrar em `Editor` é o
    // que constrói a dock tree a partir das janelas registadas.
    app.insert_state(AppState::Editor);
    app.update();

    let tree = app.world().resource::<DockTree>();
    let mut ids = Vec::new();
    for node in tree.nodes.values() {
        if let DockNode::Leaf(leaf) = node {
            ids.extend(leaf.windows.iter().map(|tab| tab.window_id.clone()));
        }
    }
    for id in [
        "vael.scenes",
        "vael.items",
        "vael.landmarks",
        "vael.progression",
        "vael.dialogues",
        "vael.quests",
        "vael.lint",
    ] {
        assert!(
            ids.iter().any(|window| window == id),
            "layout default sem `{id}`: {ids:?}"
        );
    }
}

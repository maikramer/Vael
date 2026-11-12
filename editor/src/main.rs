//! `vael-editor` — editor de conteúdo do Vael sobre Jackdaw.
//!
//! Uso:
//! ```text
//! cargo run -p vael-editor -- [--root <pasta>] [--assets <pasta>]
//! ```
//!
//! Sem `--root`, usa a pasta atual (ou a pasta do `world.bsn` indicado como
//! argumento posicional). O asset root do Jackdaw aponta para
//! `assets/public` (o pool do jogo), para o viewport poder mostrar os GLB.

use std::path::{Path, PathBuf};

use bevy::asset::AssetPlugin;
use bevy::prelude::*;
use bevy::state::app::AppExtStates;
use bevy::window::PrimaryWindow;
use jackdaw_editor_core::prelude::*;
use jackdaw_editor_core::project::{ProjectConfig, ProjectRoot};

use vael_editor::VaelExtension;
use vael_editor::state::VaelRoot;

#[derive(Debug, Clone)]
struct Options {
    root: PathBuf,
    assets: PathBuf,
}

fn main() -> AppExit {
    match run() {
        Ok(exit) => exit,
        Err(err) => {
            eprintln!("vael-editor: {err:#}");
            AppExit::from_code(2)
        }
    }
}

fn run() -> anyhow::Result<AppExit> {
    let options = parse_args()?;
    let root = absolute(&options.root)?;
    let assets = absolute(&options.assets)?;
    if !assets.is_dir() {
        anyhow::bail!(
            "asset root inexistente: {} (passa --assets <pasta>)",
            assets.display()
        );
    }
    if !root.join("world.bsn").is_file() && !root.join("world.xml").is_file() {
        eprintln!(
            "vael-editor: aviso: {} não tem world.bsn/world.xml; o editor abre na mesma",
            root.display()
        );
    }

    let mut app = App::new();
    // O pool do Vael é meshopt/KTX2; o reader do `bevy_ext_gltf` descodifica
    // os buffer views e normaliza as extensões antes do loader glTF do Bevy.
    // Tem de ser registado antes do `AssetPlugin`.
    bevy_ext_gltf::register_asset_source(&mut app, vec![assets.clone()]);
    app.add_plugins(DefaultPlugins.set(editor_window_plugin()).set(AssetPlugin {
        file_path: assets.to_string_lossy().into_owned(),
        ..default()
    }));
    app.add_plugins(EditorCorePlugin);
    app.add_plugins(ExtensionPlugin::new().with_extension::<VaelExtension>());
    app.insert_resource(ProjectRoot {
        root: root.clone(),
        config: ProjectConfig {
            name: "Vael".to_string(),
            description: "Editor de conteúdo do Vael".to_string(),
            assets_dir: Some(assets.clone()),
            ..default()
        },
    });
    app.insert_resource(VaelRoot(root.clone()));
    // Saltamos a launcher (AppState::ProjectSelect) e entramos diretamente
    // no editor: não há project build nem jackdaw.toml.
    app.insert_state(AppState::Editor);
    app.add_systems(Startup, title_window);
    let exit = app.run();
    let _ = exit;
    Ok(exit)
}

fn title_window(mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    if let Ok(mut window) = windows.single_mut() {
        window.title = "Vael Editor".to_string();
    }
}

fn parse_args() -> anyhow::Result<Options> {
    let mut root: Option<PathBuf> = None;
    let mut assets: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => {
                let Some(value) = args.next() else {
                    anyhow::bail!("--root precisa de uma pasta");
                };
                root = Some(PathBuf::from(value));
            }
            "--assets" => {
                let Some(value) = args.next() else {
                    anyhow::bail!("--assets precisa de uma pasta");
                };
                assets = Some(PathBuf::from(value));
            }
            "--help" | "-h" => {
                println!(
                    "uso: vael-editor [--root <pasta>] [--assets <pasta>]\n\
                     \n\
                     --root    raiz do projeto (world.bsn, data/, quests/, dialogues/).\n\
                     --assets  asset root do Jackdaw (default: <root>/assets/public).\n\
                     \n\
                     O editor abre o mundo e as cenas .bsn; o `world.xml` legado não é aberto."
                );
                std::process::exit(0);
            }
            flag if flag.starts_with('-') => {
                anyhow::bail!("argumento desconhecido: {flag}");
            }
            path => {
                // Um `.bsn` posicional escolhe a pasta do mundo.
                let path = PathBuf::from(path);
                root = Some(if path.is_file() {
                    path.parent().map(Path::to_path_buf).unwrap_or(path)
                } else {
                    path
                });
            }
        }
    }

    let root = match root {
        Some(root) => root,
        None => std::env::current_dir()?,
    };
    let assets = assets.unwrap_or_else(|| default_assets(&root));
    Ok(Options { root, assets })
}

fn default_assets(root: &Path) -> PathBuf {
    let public = root.join("assets").join("public");
    if public.is_dir() {
        public
    } else {
        root.join("assets")
    }
}

fn absolute(path: &Path) -> anyhow::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::path::absolute(path)?)
    }
}

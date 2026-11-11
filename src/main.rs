//! Arranque do Vael: `vael [world.xml] [--entry <nome>]`.
//!
//! A engine monta o mundo; o crate do jogo entra por cima com
//! [`viber::app::AppOptions::gameplay`] desligado, a IA em
//! [`viber::app::AiMode::Service`] e os scripts Luau em `Compat`. O `--entry`
//! continua a ser a entrada de cena do `viber run`.

use std::path::{Path, PathBuf};

use anyhow::Result;
use bevy::prelude::*;

use vael::{VaelPlugin, VaelProjectRoot};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut world = PathBuf::from("world.xml");
    let mut entry: Option<String> = None;
    let mut scene: Option<String> = None;
    let mut bridge: Option<u16> = None;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--entry" => {
                let Some(name) = args.get(i + 1) else {
                    anyhow::bail!("--entry precisa do nome da entrada");
                };
                entry = Some(name.clone());
                i += 2;
            }
            "--scene" => {
                let Some(name) = args.get(i + 1) else {
                    anyhow::bail!("--scene precisa do nome da cena");
                };
                scene = Some(name.clone());
                i += 2;
            }
            "--bridge" => {
                let Some(port) = args.get(i + 1).and_then(|raw| raw.parse::<u16>().ok()) else {
                    anyhow::bail!("--bridge precisa de uma porta (ex.: 15704)");
                };
                bridge = Some(port);
                i += 2;
            }
            "--help" | "-h" => {
                println!(
                    "uso: vael [world.xml] [--scene <nome>] [--entry <nome>] [--bridge <porta>]"
                );
                return Ok(());
            }
            flag if flag.starts_with('-') => {
                anyhow::bail!("argumento desconhecido: {flag}");
            }
            path => {
                world = PathBuf::from(path);
                i += 1;
            }
        }
    }

    // Absoluto: o cwd não pode decidir onde vivem o config, os assets e o save.
    let requested = std::path::absolute(&world)
        .map_err(|err| anyhow::anyhow!("mundo inválido: {} ({err})", world.display()))?;
    // `--scene` escolhe outra cena do projeto; sem ele, a cena principal cede
    // ao `start-scene` (o acampamento atrás do título), como o `viber run`.
    let world = viber::scene_loader::resolve_run_path(&requested, scene.as_deref())?;
    let root = viber::app::world_base_dir(&world)
        .or_else(|| world.parent().map(Path::to_path_buf))
        .ok_or_else(|| anyhow::anyhow!("não foi possível resolver a raiz do projeto"))?;

    let mut app = viber::app::build_app(
        &world,
        viber::app::AppOptions {
            gameplay: false,
            // A FSM simples da engine fica desligada, mas o serviço de IA
            // (sentidos + `AiLocomotion`) mantém-se para os scripts Luau
            // moverem as criaturas via `viber.move_towards`.
            ai: viber::app::AiMode::Service,
            // O crate do jogo ainda não tem controller/câmara; usa os da
            // engine enquanto as regras migram para Rust.
            player_camera: true,
            scripts: viber::app::ScriptsMode::Compat,
            start_entry: entry.clone(),
            bridge_port: bridge,
            // A identidade da bridge é o mundo que o utilizador pediu, não a
            // cena de arranque resolvida (senão `--scene main` respondia menu).
            world_path: Some(requested.clone()),
            ..Default::default()
        },
    )?;
    app.insert_resource(VaelProjectRoot(root.clone()));
    app.add_plugins(VaelPlugin);

    info!(
        world = %world.display(),
        requested = %requested.display(),
        root = %root.display(),
        scene = ?scene,
        entry = ?entry,
        bridge = ?bridge,
        gameplay = false,
        scripts = "compat",
        "vael: arranque"
    );

    let _ = app.run();
    Ok(())
}

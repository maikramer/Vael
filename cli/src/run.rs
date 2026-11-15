//! `vael-tools run` — lança o binário do jogo com argumentos.
//!
//! O `jd run` do Jackdaw compila e lança o jogo SEM argumentos (os `[[run]]`
//! do `jackdaw.toml` só o Play do editor lê), por isso o atalho de debug
//! `--scene main --entry dev --bridge 15706` continua a viver aqui.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Context, Result, bail};

use bevy_ext_bridge_protocol::DEFAULT_TOOLS_PORT;

/// Arranca o jogo. O cwd passa a ser a raiz do projeto para os caminhos
/// relativos (`world.bsn`, `config.yaml`) resolverem como no arranque manual.
pub fn run_vael(
    path: Option<PathBuf>,
    scene: Option<String>,
    entry: Option<String>,
    bridge: Option<Option<u16>>,
    resolution: Option<String>,
    no_cargo: bool,
) -> Result<ExitCode> {
    let root = project_root()?;
    let mut args = Vec::new();
    if let Some(path) = path {
        // O cwd do jogo passa a ser a raiz do projeto. Um caminho relativo que
        // existe relativo ao cwd de quem invoca tem de ser ancorado AGORA —
        // senão era resolvido contra a raiz (ex.: `cli/../world.bsn`).
        // Um caminho que só existe na raiz fica relativo, como antes.
        let path = if path.is_absolute() || !path.exists() {
            path
        } else {
            // `canonicalize` normaliza `..`/symlinks; o ficheiro existe.
            std::fs::canonicalize(&path)
                .or_else(|_| std::path::absolute(&path))
                .with_context(|| format!("caminho inválido: {}", path.display()))?
        };
        args.push(path.display().to_string());
    }
    if let Some(scene) = scene {
        args.push("--scene".into());
        args.push(scene);
    }
    if let Some(entry) = entry {
        args.push("--entry".into());
        args.push(entry);
    }
    if let Some(bridge) = bridge {
        args.push("--bridge".into());
        args.push(bridge.unwrap_or(DEFAULT_TOOLS_PORT).to_string());
    }
    if let Some(resolution) = resolution {
        args.push("--resolution".into());
        args.push(resolution);
    }
    if let Some(bin) = vael_binary(&root)? {
        let status = Command::new(&bin)
            .args(&args)
            .current_dir(&root)
            .status()
            .with_context(|| format!("a lançar {}", bin.display()))?;
        return Ok(exit_code(status));
    }
    if no_cargo {
        bail!(
            "binário do Vael não encontrado; define VAEL_BIN ou corre `cargo build --release` (alvo: {})",
            root.join("target/release/vael").display()
        );
    }
    let status = Command::new("cargo")
        .args(["run", "--release", "--manifest-path"])
        .arg(root.join("Cargo.toml"))
        .arg("--")
        .args(&args)
        .current_dir(&root)
        .status()
        .context("a delegar em `cargo run` do Vael")?;
    Ok(exit_code(status))
}

/// Raiz do projeto: a pasta acima que tem `jackdaw.toml` ou `world.bsn`.
fn project_root() -> Result<PathBuf> {
    let cwd = std::env::current_dir().context("sem cwd")?;
    for dir in cwd.ancestors() {
        if dir.join("jackdaw.toml").is_file() || dir.join("world.bsn").is_file() {
            return Ok(dir.to_path_buf());
        }
    }
    Ok(cwd)
}

fn vael_binary(root: &Path) -> Result<Option<PathBuf>> {
    // `VAEL_BIN` é uma ordem explícita: se está definido e não aponta para um
    // ficheiro, NÃO se cai em silêncio para outro binário/cargo (lançava o
    // jogo errado — ex.: um caminho com gralha).
    if let Some(raw) = std::env::var_os("VAEL_BIN").filter(|value| !value.is_empty()) {
        let path = PathBuf::from(raw);
        if !path.is_file() {
            bail!("VAEL_BIN inválido: {} não é um ficheiro", path.display());
        }
        // Absoluto: o filho corre com cwd na raiz e um caminho relativo
        // resolveria contra ela.
        return Ok(Some(std::path::absolute(&path).unwrap_or(path)));
    }
    Ok([
        root.join("target/release/vael"),
        root.join("target/debug/vael"),
    ]
    .into_iter()
    .find(|candidate| candidate.is_file()))
}

fn exit_code(status: std::process::ExitStatus) -> ExitCode {
    ExitCode::from(status.code().unwrap_or(1).clamp(0, 255) as u8)
}

//! `vael-tools` — ferramentas do mundo do Vael.
//!
//! Substitui o CLI `viber` (aposentado com o produto Viber). `analyze`,
//! `lint` e `bsn` são estáticos; `run` lança o jogo com argumentos (o
//! `jd run` do Jackdaw lança sem argumentos) e `debug` fala com o bridge
//! do jogo a correr.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};

mod debug;
mod playbook;
mod run;

#[derive(Parser)]
#[command(
    name = "vael-tools",
    version,
    about = "Ferramentas do mundo e bridge de debug do Vael"
)]
struct Cli {
    #[command(subcommand)]
    command: CommandKind,
}

#[derive(Subcommand)]
enum CommandKind {
    /// Analisa um mundo `.bsn` sem abrir janela.
    Analyze {
        path: Option<PathBuf>,
        /// Mundo alvo, equivalente ao positional (os gates dos docs usam
        /// `--world`, como o `debug`).
        #[arg(long, value_name = "PATH", conflicts_with = "path")]
        world: Option<PathBuf>,
        #[arg(long)]
        strict: bool,
        #[arg(long)]
        json: bool,
    },
    /// Lint estrutural do mundo (assets e scripts em falta).
    Lint {
        path: Option<PathBuf>,
        /// Mundo alvo, equivalente ao positional.
        #[arg(long, value_name = "PATH", conflicts_with = "path")]
        world: Option<PathBuf>,
        #[arg(long)]
        strict: bool,
        #[arg(long)]
        json: bool,
    },
    /// Converte/valida XML ↔ `.bsn` (os geradores ainda emitem XML).
    Bsn {
        #[command(subcommand)]
        command: BsnCommand,
    },
    /// Arranca o jogo com os argumentos do binário do Vael.
    Run {
        path: Option<PathBuf>,
        #[arg(long)]
        scene: Option<String>,
        #[arg(long)]
        entry: Option<String>,
        /// Sem valor usa a porta por omissão (15702).
        #[arg(long, num_args = 0..=1)]
        bridge: Option<Option<u16>>,
        #[arg(long)]
        resolution: Option<String>,
        /// Não delegar em `cargo run`; exige um binário do Vael.
        #[arg(long)]
        no_cargo: bool,
    },
    /// Cliente do bridge de debug do jogo a correr.
    Debug {
        /// Mundo alvo (resolve a porta pelo `engine.json`). Aceite antes ou
        /// depois do subcomando (o plugin MCP passa-o no fim).
        #[arg(long, global = true)]
        world: Option<PathBuf>,
        /// Porta do bridge (default: `VAEL_TOOLS_PORT` ou 15702).
        #[arg(long, global = true)]
        port: Option<u16>,
        /// Resultados em JSON no stdout (progresso vai para o stderr).
        #[arg(long, global = true)]
        json: bool,
        #[command(subcommand)]
        command: debug::DebugCommand,
    },
}

#[derive(Subcommand)]
enum BsnCommand {
    /// Converte ficheiros XML (e os seus includes) para `.bsn`.
    Convert {
        /// Raiz do projeto (default: a do primeiro ficheiro).
        #[arg(long)]
        root: Option<PathBuf>,
        /// Pasta de saída (default: ao lado de cada XML).
        #[arg(long)]
        out: Option<PathBuf>,
        /// Compara cada XML com o `.bsn` gerado.
        #[arg(long)]
        check: bool,
        /// Sobrescreve `.bsn` existente mesmo que difira.
        #[arg(long)]
        force: bool,
        /// Não emite `GltfSource`/luzes nativas (só `WorldNode`).
        #[arg(long)]
        no_preview: bool,
        /// Ficheiros ou pastas (default: `world.xml`).
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,
    },
    /// Compara XML e `.bsn` pelo significado (pose, luzes, includes); um
    /// `.bsn` passado sozinho é validado (carrega-o com os includes).
    Check {
        /// Raiz do projeto (default: a do primeiro ficheiro).
        #[arg(long)]
        root: Option<PathBuf>,
        /// Pasta de saída usada na conversão, se não for ao lado do XML.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Ficheiros ou pastas (default: `world.xml`).
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,
    },
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("erro: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    match cli.command {
        CommandKind::Analyze {
            path,
            world,
            strict,
            json,
        } => {
            let report = vael_tools::analyze(world.or(path).as_deref());
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print_analyze(&report);
            }
            Ok(
                if report.error.is_some() || (strict && report.strict_failed()) {
                    ExitCode::FAILURE
                } else {
                    ExitCode::SUCCESS
                },
            )
        }
        CommandKind::Lint {
            path,
            world,
            strict,
            json,
        } => {
            let report = vael_tools::lint(world.or(path).as_deref());
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print_lint(&report);
            }
            Ok(
                if report.has_errors() || (strict && report.warnings() > 0) {
                    ExitCode::FAILURE
                } else {
                    ExitCode::SUCCESS
                },
            )
        }
        CommandKind::Bsn { command } => match command {
            BsnCommand::Convert {
                root,
                out,
                check,
                force,
                no_preview,
                paths,
            } => {
                let converted = vael_tools::bsn::convert(
                    &paths,
                    root.as_deref(),
                    out.as_deref(),
                    force,
                    check,
                    !no_preview,
                )?;
                println!("bsn: {converted} ficheiro(s) convertido(s)");
                Ok(ExitCode::SUCCESS)
            }
            BsnCommand::Check { root, out, paths } => {
                let checked = vael_tools::bsn::check(&paths, root.as_deref(), out.as_deref())?;
                // `check` valida BSNs soltos e compara XML↔BSN; "verificado"
                // cobre os dois (o anterior "iguais" mentia no 1.º caso).
                println!("bsn: {checked} ficheiro(s) verificado(s)");
                Ok(ExitCode::SUCCESS)
            }
        },
        CommandKind::Run {
            path,
            scene,
            entry,
            bridge,
            resolution,
            no_cargo,
        } => run::run_vael(path, scene, entry, bridge, resolution, no_cargo),
        CommandKind::Debug {
            world,
            port,
            json,
            command,
        } => debug::run(world, port, json, command),
    }
}

fn print_analyze(report: &vael_tools::AnalyzeReport) {
    if let Some(error) = &report.error {
        // Diagnóstico (exit 1) vai para o stderr, como os erros do `run()`:
        // parsers de stdout (CI/agentes) não o podem confundir com o relatório.
        eprintln!("erro: {error}");
        return;
    }
    println!("mundo: {}", report.world);
    println!(
        "estático: {} | {} entidades | {} assets ({} em falta) | {} scripts",
        report.static_world,
        report.entities.total,
        report.assets.references,
        report.assets.missing.len(),
        report.scripts.len()
    );
    for (tag, count) in &report.entities.by_tag {
        println!("  {tag:24} {count}");
    }
    for missing in &report.assets.missing {
        println!("  asset em falta: {missing}");
    }
    for warning in &report.warnings {
        println!("  aviso: {warning}");
    }
    for info in &report.infos {
        println!("  info: {info}");
    }
}

fn print_lint(report: &vael_tools::LintReport) {
    for finding in &report.findings {
        println!(
            "{:?}: [{}] {}",
            finding.severity, finding.kind, finding.message
        );
    }
    println!(
        "lint: {} erro(s), {} aviso(s)",
        report.errors(),
        report.warnings()
    );
}

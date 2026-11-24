//! Gate de playtest in-process do Vael.
//!
//! A bateria por subprocesso do `bevy_playtest` arranca o jogo e faz
//! fast-forward de `duration_s × tps` frames sem esperar pelos assets
//! assíncronos do Vael. Aqui construímos a App completa, bootamos até
//! `WorldReadiness.ready` (e deixamos o cenário QA assentar) e só então
//! entregamos a App ao `run_scenario` — o bot começa com o mundo pronto.
//!
//! Com a feature `playtest` (ativada pelo self dev-dependency do Cargo.toml):
//!
//! ```bash
//! cargo test --features playtest --test playtest_scenarios -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! Filtros: `PLAYTEST_ONLY=menu,sandbox_chaos` e `PLAYTEST_SEEDS=42,7`.
//! Relatórios em `target/playtest-reports/`.

#![cfg(feature = "playtest")]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use bevy::app::PluginsState;
use bevy::prelude::*;
use bevy_playtest::enums::PlaytestStatus;
use bevy_playtest::scenario::Scenario;
use tempfile::TempDir;
use vael::app::{VaelGameOptions, build_game_app, world_ready};

const QA_SETTLE_FRAMES: u32 = 120;

/// Oráculo conhecido como ruído no Vael: o default do crate é ±1000 e o
/// mundo autorado tem entidades legítimas fora disso. Ver `run_case`.
const PINNED_ORACLE_RULE: &str = "nodes_in_bounds";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Fast,
    World,
    Soak,
}

struct Case {
    name: &'static str,
    /// Caminho do JSON do playtest, relativo à raiz do projeto.
    file: &'static str,
    /// Cenário QA aplicado no arranque (`vael::qa`); `None` = start-scene.
    qa: Option<&'static str>,
    scope: Scope,
}

const CASES: &[Case] = &[
    Case {
        name: "menu",
        file: "playtests/menu.json",
        qa: None,
        scope: Scope::Fast,
    },
    Case {
        name: "sandbox_chaos",
        file: "playtests/sandbox_chaos.json",
        qa: Some("sandbox"),
        scope: Scope::Fast,
    },
    Case {
        name: "arena_chaos",
        file: "playtests/arena_chaos.json",
        qa: Some("combat/arena"),
        scope: Scope::Fast,
    },
    Case {
        name: "arena_scripted",
        file: "playtests/arena_scripted.json",
        qa: Some("combat/arena"),
        scope: Scope::Fast,
    },
    Case {
        name: "interior_forge",
        file: "playtests/interior_forge.json",
        qa: Some("interiors/forge"),
        scope: Scope::Fast,
    },
    Case {
        name: "main_landmark",
        file: "playtests/main_landmark.json",
        qa: Some("world/forge_stage2"),
        scope: Scope::World,
    },
    Case {
        name: "soak_sandbox",
        file: "playtests/soak_sandbox.json",
        qa: Some("sandbox"),
        scope: Scope::Soak,
    },
];

#[test]
#[ignore = "GPU: correr com --ignored"]
fn playtest_scenarios_fast() {
    run_scope(Scope::Fast);
}

#[test]
#[ignore = "GPU + main world: correr com --ignored"]
fn playtest_scenarios_world() {
    run_scope(Scope::World);
}

#[test]
#[ignore = "soak longo: correr com --ignored"]
fn playtest_soak() {
    run_scope(Scope::Soak);
}

fn run_scope(scope: Scope) {
    let only = env_list("PLAYTEST_ONLY");
    let seeds = seeds(scope);
    let mut failures = Vec::new();
    let mut ran = 0u32;
    for case in CASES.iter().filter(|case| case.scope == scope) {
        if let Some(only) = &only
            && !only.iter().any(|name| name == case.name)
        {
            continue;
        }
        for seed in &seeds {
            print!("playtest: {} seed {seed} ... ", case.name);
            let _ = std::io::Write::flush(&mut std::io::stdout());
            match run_case_child(case, *seed) {
                Ok(()) => println!("PASS"),
                Err(err) => {
                    println!("FAIL");
                    failures.push(format!("{} (seed {seed}):\n{err}", case.name));
                }
            }
            ran += 1;
        }
    }
    assert!(ran > 0, "nenhum cenário correspondeu ao filtro");
    assert!(
        failures.is_empty(),
        "playtests falharam:\n\n{}",
        failures.join("\n\n")
    );
}

/// O `run_scenario` do crate tem um buffer de execução **global por
/// processo**: o segundo cenário no mesmo processo é rejeitado antes de
/// produzir relatório. Cada caso corre no seu próprio processo (o próprio
/// binário de teste com `--exact playtest_child`), o que também isola GPU,
/// subscriber de logs e run buffer.
fn run_case_child(case: &Case, seed: u64) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|err| format!("current_exe: {err}"))?;
    let mut child = std::process::Command::new(&exe)
        .args([
            "--exact",
            "playtest_child",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("VAEL_PLAYTEST_CASE", case.name)
        .env("VAEL_PLAYTEST_SEED", seed.to_string())
        .spawn()
        .map_err(|err| format!("spawn {}: {err}", exe.display()))?;
    // O `run_scenario` in-process não tem watchdog; um frame pendurado não
    // pode pendurar o gate inteiro.
    let timeout = match case.scope {
        Scope::World => Duration::from_secs(1200),
        Scope::Fast | Scope::Soak => Duration::from_secs(600),
    };
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "child {} (seed {seed}) excedeu {timeout:?}",
                    case.name
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(err) => return Err(format!("try_wait {}: {err}", case.name)),
        }
    };
    if !status.success() {
        return Err(format!(
            "child {} (seed {seed}) saiu com {}",
            case.name,
            status
                .code()
                .map_or_else(|| "sinal".to_string(), |code| code.to_string())
        ));
    }
    Ok(())
}

/// Processo filho de UM cenário. Sem `VAEL_PLAYTEST_CASE` é um no-op para o
/// `cargo test` normal; `run_case_child` passa o caso/seed no ambiente.
#[test]
fn playtest_child() {
    let Ok(name) = std::env::var("VAEL_PLAYTEST_CASE") else {
        return;
    };
    let seed = std::env::var("VAEL_PLAYTEST_SEED")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .unwrap_or(42);
    let case = CASES
        .iter()
        .find(|case| case.name == name)
        .unwrap_or_else(|| panic!("caso de playtest desconhecido: {name}"));
    if let Err(err) = run_case(case, seed) {
        panic!("{err}");
    }
}

fn run_case(case: &Case, seed: u64) -> Result<(), String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let scenario_path = root.join(case.file);
    let raw = std::fs::read_to_string(&scenario_path)
        .map_err(|err| format!("ler {}: {err}", scenario_path.display()))?;
    let mut scenario: Scenario =
        serde_json::from_str(&raw).map_err(|err| format!("cenário {}: {err}", case.file))?;
    scenario.bot.seed = seed;
    // Um soak curto para o gate local sem esperar os 600 s do ficheiro.
    if case.scope == Scope::Soak
        && let Some(seconds) = std::env::var("PLAYTEST_SOAK_SECONDS")
            .ok()
            .and_then(|raw| raw.parse::<f32>().ok())
            .filter(|seconds| *seconds > 0.0 && *seconds <= 3600.0)
    {
        scenario.duration_s = seconds;
    }

    // O grupo headless (`PlaytestDefaultPlugins`) remove winit/LogPlugin
    // quando `BEVY_PLAYTEST_SCENARIO` existe; o plugin de invariantes do
    // Vael usa o mesmo gate. O resto é limpeza de estado herdado do processo.
    //
    // SAFETY: os testes do ficheiro correm num único teste de cada vez; não
    // há threads da App antes do `build_game_app` e o ambiente é lido no
    // arranque.
    unsafe {
        std::env::set_var("BEVY_PLAYTEST_SCENARIO", &scenario_path);
        std::env::remove_var("VAEL_SCENARIO");
        std::env::remove_var("VAEL_START_HOUR");
        std::env::remove_var("VAEL_START_WEATHER");
        std::env::remove_var("VAEL_TOOLS");
        std::env::remove_var("JACKDAW_PIE");
    }

    let qa_dir = TempDir::new().map_err(|err| format!("tempdir do QA: {err}"))?;
    let mut app = build_game_app(VaelGameOptions {
        world: root.join("world.bsn"),
        scene: None,
        entry: None,
        scenario: case.qa.map(str::to_string),
        bridge_port: None,
        resolution: Some((640, 360)),
        qa_save_dir: Some(qa_dir.path().to_path_buf()),
    })
    .map_err(|err| format!("build_game_app: {err:#}"))?;

    let timeout = match case.scope {
        Scope::World => Duration::from_secs(600),
        Scope::Fast | Scope::Soak => Duration::from_secs(180),
    };
    boot_to_ready(&mut app, timeout)?;
    // O QA aplica 3 frames depois do ready; 120 dão folga ao teleporte e à
    // física antes de o cenário começar a contar.
    for _ in 0..QA_SETTLE_FRAMES {
        app.update();
    }

    let report = bevy_playtest::run_scenario(&mut app, &scenario)
        .map_err(|err| format!("run_scenario: {err:?}"))?;

    let report_path = root
        .join("target/playtest-reports")
        .join(format!("{}-seed{seed}.json", case.name));
    if let Some(parent) = report_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(&report) {
        let _ = std::fs::write(&report_path, json);
    }

    if report.log_capture_available == Some(false) {
        return Err(
            "log capture indisponível: os invariantes via ERROR e o oráculo de \
             log_error não estão a ver nada (LogPlugin instalado primeiro?)"
                .to_string(),
        );
    }
    if report.status != PlaytestStatus::Pass {
        // O oráculo global do crate usa ±1000, mas o mundo AUTORADO do Vael
        // tem props/vegetação legítimos em ±1500–2000. Fixamos só esta regra;
        // NaN, frozen, panic, log_error e os invariantes do herói continuam a
        // falhar o teste. Um dia o crate pode expor o extent configurável.
        let unexpected: Vec<(&str, &str)> = report
            .violations
            .iter()
            .filter(|violation| {
                if violation.rule == PINNED_ORACLE_RULE {
                    return false;
                }
                // `log_warn` só é gating com `strict_warnings` (o default é
                // informativo; o crate documenta isto). `log_capture_lost`
                // NÃO é filtrado: sem logs os invariantes não contam.
                if violation.rule == "log_warn" && !scenario.strict_warnings {
                    return false;
                }
                // Nenhum `log_error` está fixado: um navmesh que o landmass
                // recuse é apanhado pelo `bevy_ext_nav` e só loga `ERROR`
                // quando a recusa se repete (ver docs/PLAYTEST.md).
                true
            })
            .map(|violation| (violation.rule.as_str(), violation.target.as_str()))
            .collect();
        if !unexpected.is_empty() || report.status == PlaytestStatus::Crash {
            return Err(report.findings_summary(case.file));
        }
        println!(
            "({} findings de `{PINNED_ORACLE_RULE}` fixados; ver docs/PLAYTEST.md)",
            report
                .violations
                .iter()
                .filter(|violation| violation.rule == PINNED_ORACLE_RULE)
                .count()
        );
    }
    Ok(())
}

/// Fecha os plugins, corre o Startup e espera pelo `WorldReadiness.ready`.
/// O sleep curto deixa os pool de IO/GPU avançar entre frames — o driver do
/// harness não faz espera nenhuma pelo carregamento.
fn boot_to_ready(app: &mut App, timeout: Duration) -> Result<(), String> {
    while app.plugins_state() == PluginsState::Adding {
        bevy::tasks::tick_global_task_pools_on_main_thread();
    }
    app.finish();
    app.cleanup();
    let start = Instant::now();
    let mut frames = 0u64;
    loop {
        if world_ready(app) {
            return Ok(());
        }
        if start.elapsed() > timeout {
            return Err(format!(
                "mundo sem `WorldReadiness.ready` em {timeout:?} ({frames} frames)"
            ));
        }
        app.update();
        frames += 1;
        if frames % 5 == 0 {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

fn seeds(scope: Scope) -> Vec<u64> {
    env_list("PLAYTEST_SEEDS")
        .map(|raw| {
            raw.iter()
                .filter_map(|seed| seed.parse::<u64>().ok())
                .collect::<Vec<_>>()
        })
        .filter(|seeds| !seeds.is_empty())
        .unwrap_or_else(|| match scope {
            // A bateria rápida faz o sweep; o main world e o soak são caros.
            Scope::Fast => vec![42, 7, 1337],
            Scope::World | Scope::Soak => vec![42],
        })
}

fn env_list(key: &str) -> Option<Vec<String>> {
    let raw = std::env::var(key).ok()?;
    let values = raw
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    (!values.is_empty()).then_some(values)
}

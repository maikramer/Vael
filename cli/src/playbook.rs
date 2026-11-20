//! Playbook de testes: uma sequência de passos executados num processo só,
//! com transcript JSON e exit semântico — o que deixa um agente correr um
//! teste vivo sem escrever Python nem loops de polling.
//!
//! Esquema do ficheiro:
//!
//! ```json
//! {
//!   "name": "forja",
//!   "steps": [
//!     { "op": "scene", "name": "qa/city-forge", "wait": true },
//!     { "op": "step", "frames": 10 },
//!     { "op": "key", "key": "KeyE" },
//!     { "op": "until", "expr": "vael.qa_snapshot():find('craft') ~= nil", "timeout_ms": 5000 },
//!     { "op": "expect", "expect": {"items": {"hunter_cloak": 1}} },
//!     { "op": "screenshot", "path": "out/fim.png" }
//!   ]
//! }
//! ```
//!
//! O executor para no primeiro passo que falha (salvo `--continue-on-error`)
//! e devolve exit 1; o transcript diz exatamente que passo e porquê.
//!
//! Caminhos relativos dentro do playbook (`screenshot.path`, `expect.file`)
//! resolvem contra a pasta do PRÓPRIO playbook; um caminho que só exista
//! relativo ao cwd (playbooks antigos) continua a valer. No transcript,
//! `steps` é o nº de passos DECLARADOS e `executed` o nº que correu (com
//! `--continue-on-error` são iguais; numa paragem, `executed < steps`).

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use bevy_ext_bridge_protocol::BridgeClient;

use crate::debug::{
    capture_screenshot, expect_check, lua_response, poll_ready_for_scene, scene_change,
    scene_marker, send_key, state_snapshot, until_poll,
};

/// Passos do playbook. A tag `op` escolhe a variante (`snake_case`).
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PlaybookStep {
    /// Luau cru no jogo (só no monólito, como `debug lua`).
    Lua { code: String },
    Key {
        key: String,
        state: Option<String>,
        hold_ms: Option<u64>,
    },
    Text { text: String },
    Click {
        x: f32,
        y: f32,
        state: Option<String>,
    },
    Move { x: f32, y: f32 },
    /// Espera em tempo REAL (uso raro; prefere `until`).
    WaitMs { ms: u64 },
    /// Poll de uma expressão Luau até ser verdadeira.
    Until {
        expr: String,
        timeout_ms: Option<u64>,
        interval_ms: Option<u64>,
    },
    /// Estado do jogo no transcript (quests/itens/...).
    State,
    /// Expect contra as regras reais (`qa_check`); sem `expect` usa o do
    /// cenário de QA aplicado.
    Expect {
        expect: Option<Value>,
        file: Option<PathBuf>,
    },
    Screenshot { path: Option<PathBuf> },
    Pause,
    Play,
    /// Corre N frames pausado (determinismo).
    Step { frames: u32 },
    Scene {
        name: String,
        entry: Option<String>,
        wait: Option<bool>,
        wait_timeout: Option<u64>,
    },
}

impl PlaybookStep {
    fn op(&self) -> &'static str {
        match self {
            Self::Lua { .. } => "lua",
            Self::Key { .. } => "key",
            Self::Text { .. } => "text",
            Self::Click { .. } => "click",
            Self::Move { .. } => "move",
            Self::WaitMs { .. } => "wait_ms",
            Self::Until { .. } => "until",
            Self::State => "state",
            Self::Expect { .. } => "expect",
            Self::Screenshot { .. } => "screenshot",
            Self::Pause => "pause",
            Self::Play => "play",
            Self::Step { .. } => "step",
            Self::Scene { .. } => "scene",
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct PlaybookFile {
    pub name: Option<String>,
    #[serde(default)]
    pub steps: Vec<PlaybookStep>,
}

#[derive(Serialize)]
struct Entry {
    index: usize,
    op: &'static str,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    result: Value,
    elapsed_ms: u128,
}

#[derive(Serialize)]
struct Transcript {
    name: Option<String>,
    ok: bool,
    /// Passos DECLARADOS no ficheiro (o `executed` é o que correu de facto).
    steps: usize,
    /// Passos executados até ao fim (ou até à paragem).
    executed: usize,
    elapsed_ms: u128,
    entries: Vec<Entry>,
}

pub fn execute(
    client: &BridgeClient,
    port: u16,
    file: &Path,
    continue_on_error: bool,
    json_out: bool,
    transcript_out: Option<&Path>,
) -> Result<ExitCode> {
    let raw = std::fs::read_to_string(file)
        .with_context(|| format!("a ler o playbook {}", file.display()))?;
    let playbook: PlaybookFile = serde_json::from_str(&raw)
        .with_context(|| format!("playbook inválido: {}", file.display()))?;
    if playbook.steps.is_empty() {
        bail!("playbook sem passos: {}", file.display());
    }

    let started = Instant::now();
    // Pasta do playbook: os caminhos relativos dos passos resolvem contra ela
    // (não contra o cwd de quem lança o CLI).
    let base = playbook_base(file);
    let mut entries = Vec::new();
    let mut any_failed = false;
    for (index, step) in playbook.steps.iter().enumerate() {
        let at = Instant::now();
        let outcome = run_step(client, port, step, &base);
        let ok = outcome.is_ok();
        let (error, result) = match outcome {
            Ok(result) => (None, result),
            Err(error) => (Some(format!("{error:#}")), Value::Null),
        };
        let entry = Entry {
            index,
            op: step.op(),
            ok,
            error,
            result,
            elapsed_ms: at.elapsed().as_millis(),
        };
        if json_out {
            eprintln!(
                "[{index:>3}] {:>10} {} ({:.1}s)",
                entry.op,
                if ok { "ok" } else { "FALHOU" },
                entry.elapsed_ms as f64 / 1000.0
            );
        } else {
            println!(
                "[{index:>3}] {:>10} {} ({:.1}s)",
                entry.op,
                if ok { "ok" } else { "FALHOU" },
                entry.elapsed_ms as f64 / 1000.0
            );
            if let Some(error) = &entry.error {
                println!("      {error}");
            }
        }
        any_failed |= !ok;
        let failed_here = !ok;
        entries.push(entry);
        if failed_here && !continue_on_error {
            break;
        }
    }
    let transcript = Transcript {
        name: playbook.name,
        ok: !any_failed,
        steps: playbook.steps.len(),
        executed: entries.len(),
        elapsed_ms: started.elapsed().as_millis(),
        entries,
    };
    let rendered = serde_json::to_string_pretty(&transcript)?;
    if let Some(path) = transcript_out {
        std::fs::write(path, &rendered)
            .with_context(|| format!("a escrever {}", path.display()))?;
    }
    println!("{rendered}");
    Ok(if any_failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

/// Pasta do playbook (o cwd quando o ficheiro não tem pasta — `x.json`).
fn playbook_base(file: &Path) -> PathBuf {
    match file.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// Resolve um caminho de um passo relativo à pasta do playbook, aceitando os
/// playbooks antigos que o punham relativo ao cwd: um caminho que já existe
/// (ou, para escrita, cuja pasta existe) no cwd ganha; senão é relativo ao
/// playbook.
fn resolve_step_path(base: &Path, path: &Path, writing: bool) -> PathBuf {
    // Absoluto, ou o playbook está no cwd (nada a ancorar).
    if path.is_absolute() || base == Path::new(".") {
        return path.to_path_buf();
    }
    let anchored = base.join(path);
    let present = |candidate: &Path| {
        if writing {
            candidate
                .parent()
                .map(Path::is_dir)
                .unwrap_or_else(|| candidate.is_dir())
        } else {
            candidate.is_file()
        }
    };
    if present(&anchored) || !present(path) {
        // O playbook é o sítio certo — um caminho que não existe em lado
        // nenhum fica ancorado (o erro diz onde o procurar).
        anchored
    } else {
        // Playbook antigo que escrevia/lia do cwd.
        path.to_path_buf()
    }
}

fn run_step(
    client: &BridgeClient,
    port: u16,
    step: &PlaybookStep,
    base: &Path,
) -> Result<Value> {
    match step {
        PlaybookStep::Lua { code } => Ok(lua_response(client, code)?
            .get("result")
            .cloned()
            .unwrap_or(Value::Null)),
        PlaybookStep::Key {
            key,
            state,
            hold_ms,
        } => {
            let state = state.as_deref().unwrap_or("click");
            // hold: press + espera real + release (mesmo contrato do CLI).
            if let Some(ms) = hold_ms.filter(|ms| *ms > 0).filter(|_| state != "release") {
                let press = send_key(client, key, "press")?;
                std::thread::sleep(Duration::from_millis(ms));
                let release = send_key(client, key, "release")?;
                Ok(json!({ "pressed": press, "released": release, "hold_ms": ms }))
            } else {
                send_key(client, key, state)
            }
        }
        PlaybookStep::Text { text } => {
            use bevy_ext_bridge_protocol::LEGACY_TEXT;
            client
                .text(text)
                .or_else(|_| client.call(LEGACY_TEXT, json!({ "text": text })))?;
            Ok(json!({ "chars": text.chars().count() }))
        }
        PlaybookStep::Click { x, y, state } => {
            use bevy_ext_bridge_protocol::LEGACY_CLICK;
            let state = state.as_deref().unwrap_or("click");
            client
                .click(*x, *y, state)
                .or_else(|_| client.call(LEGACY_CLICK, json!({ "x": x, "y": y, "state": state })))?;
            Ok(json!({ "x": x, "y": y, "state": state }))
        }
        PlaybookStep::Move { x, y } => {
            use bevy_ext_bridge_protocol::LEGACY_MOVE;
            client
                .move_cursor(*x, *y)
                .or_else(|_| client.call(LEGACY_MOVE, json!({ "x": x, "y": y })))?;
            Ok(json!({ "x": x, "y": y }))
        }
        PlaybookStep::WaitMs { ms } => {
            std::thread::sleep(Duration::from_millis(*ms));
            Ok(json!({ "ms": ms }))
        }
        PlaybookStep::Until {
            expr,
            timeout_ms,
            interval_ms,
        } => {
            let outcome = until_poll(
                client,
                expr,
                timeout_ms.unwrap_or(10_000),
                interval_ms.unwrap_or(500),
                true,
            )?;
            let passed = outcome.passed;
            let out = json!({
                "ok": passed,
                "value": outcome.value,
                "polls": outcome.polls,
                "elapsed_ms": outcome.elapsed_ms,
            });
            if !passed {
                bail!("until sem verdade no timeout ({} polls)", outcome.polls);
            }
            Ok(out)
        }
        PlaybookStep::State => Ok(state_snapshot(client)?),
        PlaybookStep::Expect { expect, file } => {
            let raw = match (expect, file) {
                (Some(expect), None) => Some(serde_json::to_string(expect)?),
                (None, Some(file)) => {
                    let file = resolve_step_path(base, file, false);
                    Some(
                        std::fs::read_to_string(&file)
                            .with_context(|| format!("a ler {}", file.display()))?,
                    )
                }
                (Some(_), Some(_)) => bail!("usa `expect` ou `file`, não ambos"),
                (None, None) => None,
            };
            let verdict = expect_check(client, raw.as_deref())?;
            let ok = verdict.get("ok").and_then(Value::as_bool).unwrap_or(false);
            if !ok {
                bail!("expect falhou: {}", verdict["failures"]);
            }
            Ok(verdict)
        }
        PlaybookStep::Screenshot { path } => {
            let path = resolve_step_path(
                base,
                &path.clone().unwrap_or_else(|| "screenshot.png".into()),
                true,
            );
            let (png, _source) = capture_screenshot(client, 3000)?;
            std::fs::write(&path, &png)
                .with_context(|| format!("a escrever {}", path.display()))?;
            Ok(json!({ "path": path.display().to_string(), "bytes": png.len() }))
        }
        PlaybookStep::Pause => {
            use bevy_ext_bridge_protocol::METHOD_PAUSE;
            Ok(client.call(METHOD_PAUSE, json!({}))?)
        }
        PlaybookStep::Play => {
            use bevy_ext_bridge_protocol::METHOD_PLAY;
            Ok(client.call(METHOD_PLAY, json!({}))?)
        }
        PlaybookStep::Step { frames } => {
            use bevy_ext_bridge_protocol::METHOD_STEP;
            Ok(client.call(METHOD_STEP, json!({ "frames": frames }))?)
        }
        PlaybookStep::Scene {
            name,
            entry,
            wait,
            wait_timeout,
        } => {
            let wait = wait.unwrap_or(true);
            // Marca a cena ANTES da troca: o `ready` do ping pode ser o da
            // cena anterior por um frame (o mesmo cuidado do `debug scene`).
            let before = wait.then(|| scene_marker(client));
            let requested = scene_change(client, name, entry.as_deref())?;
            let mut result = json!({ "requested": name, "entry": entry, "change": requested });
            if let Some(before) = &before {
                let ready = poll_ready_for_scene(
                    client,
                    port,
                    name,
                    before,
                    wait_timeout.unwrap_or(300),
                    true,
                )?;
                result["ready"] = json!(ready);
                if !ready {
                    bail!("cena `{name}` não ficou pronta no timeout");
                }
            }
            Ok(result)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_paths_resolve_against_the_playbook_folder() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("qa/playbooks");
        std::fs::create_dir_all(base.join("out")).unwrap();
        // Escrita: a pasta existe ao lado do playbook → ancora lá.
        assert_eq!(
            resolve_step_path(&base, Path::new("out/fim.png"), true),
            base.join("out/fim.png")
        );
        // Leitura: o ficheiro existe ao lado do playbook.
        std::fs::write(base.join("expect.json"), "{}").unwrap();
        assert_eq!(
            resolve_step_path(&base, Path::new("expect.json"), false),
            base.join("expect.json")
        );
        // Retrocompat: um caminho que só existe relativo ao cwd (o playbook
        // antigo escrevia no cwd).
        assert_eq!(
            resolve_step_path(&base, Path::new("Cargo.toml"), false),
            Path::new("Cargo.toml")
        );
        // Um caminho que não existe em lado nenhum fica ancorado (o erro diz
        // onde o procurar) e um absoluto não é tocado.
        assert_eq!(
            resolve_step_path(&base, Path::new("falta.json"), false),
            base.join("falta.json")
        );
        assert_eq!(
            resolve_step_path(&base, Path::new("/tmp/x.png"), true),
            Path::new("/tmp/x.png")
        );
        // Playbook no cwd: nada a ancorar.
        assert_eq!(
            resolve_step_path(Path::new("."), Path::new("out/fim.png"), true),
            Path::new("out/fim.png")
        );
    }

    #[test]
    fn transcript_counts_declared_and_executed_steps() {
        // Regressão: o transcript dizia `steps: 3` com 2 `entries` e nada
        // dizia quantos passos tinham corrido.
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("playbook.json");
        std::fs::write(
            &file,
            r#"{"steps":[
                {"op":"lua","code":"return 1"},
                {"op":"lua","code":"return 2"},
                {"op":"lua","code":"return 3"}
            ]}"#,
        )
        .unwrap();
        let port = crate::testutil::mock_bridge(vec![
            ("viber.lua", Ok(json!({ "ok": true, "result": 1 }))),
            ("viber.lua", Err("lua: falhou")),
        ]);
        let client = BridgeClient::localhost(port);
        let transcript_path = dir.path().join("transcript.json");
        let code = execute(&client, port, &file, false, true, Some(&transcript_path)).unwrap();
        assert_eq!(code, ExitCode::FAILURE);
        let transcript: Value =
            serde_json::from_str(&std::fs::read_to_string(&transcript_path).unwrap()).unwrap();
        assert_eq!(transcript["steps"], 3);
        assert_eq!(transcript["executed"], 2);
        assert_eq!(transcript["entries"].as_array().unwrap().len(), 2);
        assert_eq!(transcript["ok"], false);
    }
}

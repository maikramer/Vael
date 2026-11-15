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
//!     { "op": "until", "expr": "viber.vael.qa_snapshot():find('craft') ~= nil", "timeout_ms": 5000 },
//!     { "op": "expect", "expect": {"items": {"hunter_cloak": 1}} },
//!     { "op": "screenshot", "path": "out/fim.png" }
//!   ]
//! }
//! ```
//!
//! O executor para no primeiro passo que falha (salvo `--continue-on-error`)
//! e devolve exit 1; o transcript diz exatamente que passo e porquê.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use bevy_ext_bridge_protocol::BridgeClient;

use crate::debug::{
    capture_screenshot, expect_check, lua_response, poll_ready, scene_change, send_key,
    state_snapshot, until_poll,
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
    steps: usize,
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
    let mut entries = Vec::new();
    let mut any_failed = false;
    for (index, step) in playbook.steps.iter().enumerate() {
        let at = Instant::now();
        let outcome = run_step(client, port, step);
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

fn run_step(
    client: &BridgeClient,
    port: u16,
    step: &PlaybookStep,
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
                (None, Some(file)) => Some(std::fs::read_to_string(file).with_context(
                    || format!("a ler {}", file.display()),
                )?),
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
            let path = path.clone().unwrap_or_else(|| "screenshot.png".into());
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
            let requested = scene_change(client, name, entry.as_deref())?;
            let mut result = json!({ "requested": name, "change": requested });
            if wait.unwrap_or(true) {
                let ready = poll_ready(client, port, wait_timeout.unwrap_or(300), true)?;
                result["ready"] = json!(ready);
                if !ready {
                    bail!("cena `{name}` não ficou pronta no timeout");
                }
            }
            Ok(result)
        }
    }
}

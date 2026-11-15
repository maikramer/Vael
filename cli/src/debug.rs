//! Cliente do bridge de debug do jogo.
//!
//! Os métodos `vael-tools/*` são servidos pelo `bevy_ext_bridge_runtime`
//! dentro do jogo; o fallback `viber.*` é do monólito (Luau, burst e trace
//! continuam lá até à fase H do DELETE-MONOLITH). O transporte é JSON-RPC
//! sobre HTTP na porta 15702 (ou `--bridge`/`VAEL_TOOLS_PORT`).
//!
//! Contratos para agentes: `--json` em tudo (resultados em stdout, erros
//! JSON-RPC e de uso no stderr com exit 1); primitivas de teste com exit
//! semântico (`until`/`expect`/`wait-ready`/`scene --wait` devolvem 1 em
//! timeout/falha); `playbook` compõe os passos num processo só.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use serde_json::{Value, json};

use bevy_ext_bridge_protocol::{
    BridgeClient, LEGACY_CLICK, LEGACY_KEY, LEGACY_LOGS, LEGACY_LUA, LEGACY_MOVE, LEGACY_PING,
    LEGACY_PROFILER_MARK, LEGACY_PROFILER_TAB, LEGACY_TEXT, LEGACY_TREE, METHOD_FRAME_STATS,
    METHOD_HITCHES, METHOD_MARK, METHOD_MEMORY, METHOD_PAUSE, METHOD_PLAY, METHOD_STEP,
    METHOD_TREE, SessionPaths, is_listening, normalize_key, resolve_port,
};

/// Troca de cena (só no monólito; a duração depende do mundo).
const LEGACY_SCENE_CHANGE: &str = "viber.scene.change";

#[derive(Subcommand)]
pub enum DebugCommand {
    /// Confirma que o bridge responde.
    Ping,
    /// Espera até o jogo estar pronto: bridge a ouvir e mundo carregado
    /// (`ready` do ping — o véu de carregamento já saiu). Exit 0 quando
    /// pronto, 1 no timeout. Correr ANTES de screenshots/input.
    WaitReady {
        /// Segundos máximos de espera.
        #[arg(long, default_value_t = 300)]
        timeout: u64,
        /// Silencioso (só o resultado final).
        #[arg(long, short)]
        quiet: bool,
    },
    /// Lista as engines conhecidas (`engine.json` das sessões), com estado
    /// de vida (pid + porta). `--prune` apaga as mortas.
    Engines {
        /// Apaga as sessões mortas (o `engine.json` é cache).
        #[arg(long)]
        prune: bool,
    },
    /// Sonda a porta: diz se há um bridge a ouvir e o ping, sem falhar.
    Probe,
    /// Guarda um screenshot PNG.
    Screenshot {
        #[arg(short, long, default_value = "screenshot.png")]
        output: PathBuf,
        #[arg(long, default_value_t = 3000)]
        timeout_ms: u64,
        /// Espera N ms antes de pedir o frame (câmara a assentar).
        #[arg(long, default_value_t = 0)]
        delay_ms: u64,
    },
    /// Captura N frames numa folha única (flicker/movimento sem N PNGs).
    Burst {
        #[arg(short, long, default_value = "burst.png")]
        output: PathBuf,
        /// Nº de frames: 4 (2×2), 9 (3×3) ou 16 (4×4).
        #[arg(long, default_value_t = 9)]
        frames: u32,
        /// Frames renderizados a saltar entre capturas (0 = consecutivos).
        #[arg(long, default_value_t = 0)]
        skip: u32,
        #[arg(long, default_value_t = 60_000)]
        timeout_ms: u64,
        /// Imprime o veredicto numérico de flicker (luma por frame).
        #[arg(long)]
        stats: bool,
    },
    /// Árvore de entidades (`vael-tools/tree` ou `viber.tree`).
    Tree {
        /// Só entidades cujo nome contém este texto (sem maiúsculas).
        #[arg(long)]
        filter: Option<String>,
    },
    /// Logs do jogo. Por omissão lê o ficheiro de log da sessão (completo
    /// desde o boot); `--source bridge` pergunta ao jogo (ring de ~1000).
    Logs {
        /// Últimas N linhas (ficheiro) ou entradas (bridge).
        #[arg(long, alias = "limit", default_value_t = 100)]
        tail: usize,
        /// Só linhas que contêm este texto (sem maiúsculas).
        #[arg(long)]
        grep: Option<String>,
        /// `auto` (ficheiro da sessão, se houver; senão bridge) | `file` | `bridge`.
        #[arg(long, default_value = "auto")]
        source: String,
    },
    /// Memória do processo (RSS/HWM/threads).
    Memory,
    /// Hitches recentes.
    Hitches,
    /// Snapshot do profiler (`--record` grava uma série).
    Prof {
        #[arg(long, default_value = "frames")]
        tab: String,
        #[arg(long)]
        json: bool,
        /// Lista os hitches recentes em vez do snapshot de frames.
        #[arg(long)]
        hitches: bool,
        /// Grava uma série durante N segundos (`-o` para JSONL).
        #[arg(long)]
        record: Option<u64>,
        /// Amostras por segundo do `--record`.
        #[arg(long, default_value_t = 2.0)]
        hz: f64,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Marca um ponto no trace.
    Mark {
        label: String,
    },
    /// Input sintético: tecla (com `--hold-ms`), texto, clique, movimento.
    Key {
        key: String,
        #[arg(long, default_value = "click")]
        state: String,
        #[arg(long)]
        hold_ms: Option<u64>,
    },
    Text {
        text: String,
    },
    Click {
        x: f32,
        y: f32,
        #[arg(long, default_value = "click")]
        state: String,
    },
    Move {
        x: f32,
        y: f32,
    },
    /// REPL Luau (só no monólito por enquanto: `viber.lua`).
    Lua {
        code: Option<String>,
        #[arg(long)]
        file: Option<PathBuf>,
    },
    /// Pausa o jogo (`Time<Virtual>` parado; o runtime continua a responder).
    Pause,
    /// Retoma o jogo pausado.
    Play,
    /// Corre N frames pausado e volta a parar (determinismo por frames).
    Step {
        #[arg(long, default_value_t = 1)]
        frames: u32,
    },
    /// Estado do jogo em JSON (quests/itens/ouro/hp/posição) via
    /// `viber.vael.qa_snapshot()`. Requer o jogo com a API de QA (monólito).
    State,
    /// Verifica um `expect` (mesma sintaxe dos cenários de QA: `{"itens":
    /// {"ouro": ">=150"}}`) contra o estado real, com as regras do jogo.
    /// Sem argumento usa o expect do cenário aplicado. Exit 1 em falha.
    Expect {
        /// JSON do expect (ou `@ficheiro.json`).
        expect: Option<String>,
        /// Lê o expect de um ficheiro.
        #[arg(long)]
        file: Option<PathBuf>,
    },
    /// Sonda uma expressão Luau até ser verdadeira (mata os sleeps):
    /// `until 'viber.vael.qa_snapshot():find("done") ~= nil'`. Exit 1 no
    /// timeout.
    Until {
        /// Expressão Luau (avaliada como `return (<expr>)`).
        expr: String,
        #[arg(long, default_value_t = 10_000)]
        timeout_ms: u64,
        /// Intervalo entre polls.
        #[arg(long, default_value_t = 500)]
        interval_ms: u64,
    },
    /// Troca de cena (`viber.scene.change`) e, com `--wait`, espera o
    /// carregamento novo. Exit 1 se a troca falhar ou o `--wait` esgotar.
    Scene {
        name: String,
        /// Entrada da cena (`@back` = portal de onde veio).
        #[arg(long)]
        entry: Option<String>,
        /// Espera o mundo novo ficar pronto.
        #[arg(long)]
        wait: bool,
        /// Segundos máximos de espera do `--wait`.
        #[arg(long, default_value_t = 300)]
        wait_timeout: u64,
    },
    /// Executa um playbook JSON (sequência de passos de teste) num processo
    /// só, com transcript JSON. Exit 1 no primeiro passo que falha (ou no
    /// fim, com `--continue-on-error`, se algum falhou).
    Playbook {
        /// Ficheiro do playbook (`{"steps": [{"op": ...}, ...]}`).
        file: PathBuf,
        /// Não parar no primeiro falha; o transcript marca-o.
        #[arg(long)]
        continue_on_error: bool,
        /// Grava o transcript JSON também neste ficheiro.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

pub fn run(
    world: Option<PathBuf>,
    port: Option<u16>,
    json_out: bool,
    command: DebugCommand,
) -> Result<ExitCode> {
    if let DebugCommand::Engines { prune } = command {
        return engines(prune, json_out);
    }
    // Aviso quando `--world` não casa com sessão nenhuma: `resolve_port`
    // cai no default em silêncio e o agente fala com o jogo errado.
    if port.is_none()
        && std::env::var_os("VAEL_TOOLS_PORT").is_none()
        && let Some(target) = &world
    {
        if SessionPaths::for_world(target).engine_info().is_none() {
            eprintln!(
                "aviso: nenhuma sessão para {}; a usar a porta por omissão (VAEL_TOOLS_PORT ou 15702)",
                target.display()
            );
        }
    }
    let port = resolve_port(port, world.as_deref());
    if let DebugCommand::WaitReady { timeout, quiet } = command {
        return wait_ready(port, timeout, quiet, json_out);
    }
    if matches!(command, DebugCommand::Probe) {
        let probe = if is_listening(port) {
            let client = BridgeClient::localhost(port);
            json!({ "listening": true, "ping": client.ping().ok() })
        } else {
            json!({ "listening": false, "ping": Value::Null })
        };
        println!("{}", serde_json::to_string_pretty(&probe)?);
        return Ok(ExitCode::SUCCESS);
    }
    // Falha em 150 ms em vez dos 40 s de retry do cliente quando não há jogo.
    if !is_listening(port) {
        bail!("bridge desligado na porta {port}; arranca `vael-tools run --bridge {port}`");
    }
    let client = BridgeClient::localhost(port);
    match command {
        DebugCommand::Ping => {
            println!("{}", serde_json::to_string_pretty(&client.ping()?)?);
        }
        DebugCommand::Engines { .. } | DebugCommand::Probe | DebugCommand::WaitReady { .. } => {
            unreachable!("tratados antes do probe")
        }
        DebugCommand::Screenshot {
            output,
            timeout_ms,
            delay_ms,
        } => {
            if delay_ms > 0 {
                std::thread::sleep(std::time::Duration::from_millis(delay_ms));
            }
            let (png, _path) = capture_screenshot(&client, timeout_ms)?;
            std::fs::write(&output, &png)
                .with_context(|| format!("a escrever {}", output.display()))?;
            emit(
                json_out,
                format!("screenshot: {} ({} bytes)", output.display(), png.len()),
                &json!({ "path": output.display().to_string(), "bytes": png.len() }),
            )?;
        }
        DebugCommand::Burst {
            output,
            frames,
            skip,
            timeout_ms,
            stats,
        } => {
            let (png, source, width, height, status) = client
                .burst(frames, skip, timeout_ms)
                .or_else(|_| client.legacy_burst(frames, skip, timeout_ms))?;
            std::fs::write(&output, &png)
                .with_context(|| format!("a escrever {}", output.display()))?;
            let frame_stats = status
                .get("frame_stats")
                .or_else(|| status.get("stats"))
                .cloned()
                .unwrap_or(Value::Null);
            if stats && !json_out {
                println!("{}", serde_json::to_string_pretty(&frame_stats)?);
            }
            emit(
                json_out,
                format!(
                    "burst: {} ({}×{}; {} bytes; origem {source})",
                    output.display(),
                    width,
                    height,
                    png.len()
                ),
                &json!({
                    "path": output.display().to_string(),
                    "width": width,
                    "height": height,
                    "bytes": png.len(),
                    "source": source,
                    "stats": frame_stats,
                }),
            )?;
        }
        DebugCommand::Tree { filter } => {
            let value = call_fallback(
                &client,
                METHOD_TREE,
                LEGACY_TREE,
                json!({ "filter": filter }),
            )?;
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        DebugCommand::Logs {
            tail,
            grep,
            source,
        } => {
            let mode = match source.as_str() {
                "auto" | "file" | "bridge" => source,
                other => bail!("--source inválido `{other}` (auto | file | bridge)"),
            };
            let log_path = if mode == "bridge" {
                None
            } else {
                session_log_path(world.as_deref())
            };
            match log_path {
                Some(path) if mode != "bridge" => {
                    let lines = read_log_lines(&path, tail, grep.as_deref())?;
                    emit(
                        json_out,
                        lines.join("\n"),
                        &json!({
                            "source": "file",
                            "path": path.display().to_string(),
                            "lines": lines,
                        }),
                    )?;
                }
                _ => {
                    if mode == "file" {
                        bail!("--source file: sem ficheiro de log de sessão (lança com `vael-tools run --bridge`)");
                    }
                    // Ring do monólito (`viber.logs`): o runtime não serve
                    // `vael-tools/logs` — o stub que devolvia `[]` mascarava
                    // este ring real via fallback (que só dispara em erro).
                    let value = client.call(LEGACY_LOGS, json!({ "limit": tail }))?;
                    let entries = filter_log_entries(&value, grep.as_deref());
                    emit(
                        json_out,
                        serde_json::to_string_pretty(&entries)?,
                        &json!({ "source": "bridge", "entries": entries }),
                    )?;
                }
            }
        }
        DebugCommand::Memory => {
            let value = client.call(METHOD_MEMORY, json!({}))?;
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        DebugCommand::Hitches => {
            let value = client.call(METHOD_HITCHES, json!({}))?;
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        DebugCommand::Prof {
            tab,
            json,
            hitches,
            record,
            hz,
            output,
        } => {
            let json = json || json_out;
            if hitches {
                let value = client.call(METHOD_HITCHES, json!({}))?;
                println!("{}", serde_json::to_string_pretty(&value)?);
            } else if let Some(seconds) = record {
                let hz = if hz.is_finite() && hz >= 0.1 { hz } else { 2.0 };
                let samples = (seconds as f64 * hz).round().max(1.0) as u64;
                let mut file = output
                    .as_ref()
                    .map(|path| {
                        std::fs::File::create(path)
                            .with_context(|| format!("a criar {}", path.display()))
                    })
                    .transpose()?;
                for _ in 0..samples {
                    let value = client
                        .call(METHOD_FRAME_STATS, json!({}))
                        .or_else(|_| client.call(LEGACY_PROFILER_TAB, json!({ "tab": tab })))?;
                    if let Some(file) = file.as_mut() {
                        writeln!(file, "{value}")?;
                    } else if json {
                        println!("{value}");
                    }
                    std::thread::sleep(std::time::Duration::from_secs_f64(1.0 / hz));
                }
                if let Some(path) = &output {
                    println!("perf: {} amostra(s) em {}", samples, path.display());
                }
            } else {
                let value = client
                    .call(METHOD_FRAME_STATS, json!({}))
                    .or_else(|_| client.call(LEGACY_PROFILER_TAB, json!({ "tab": tab })))?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&value)?);
                } else {
                    println!("{value}");
                }
            }
        }
        DebugCommand::Mark { label } => {
            let value = call_fallback(
                &client,
                METHOD_MARK,
                LEGACY_PROFILER_MARK,
                json!({ "label": label }),
            )?;
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        DebugCommand::Key {
            key,
            state,
            hold_ms,
        } => {
            // `--hold-ms` tem de separar press e release em chamadas
            // diferentes: o runtime aplica a tecla no `PreUpdate` seguinte, e
            // um click (press+release no mesmo lote) só dura um frame.
            if let Some(ms) = hold_ms.filter(|ms| *ms > 0).filter(|_| state != "release") {
                let press = send_key(&client, &key, "press")?;
                std::thread::sleep(std::time::Duration::from_millis(ms));
                let release = send_key(&client, &key, "release")?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "pressed": press,
                        "released": release,
                        "hold_ms": ms,
                    }))?
                );
            } else {
                let value = send_key(&client, &key, &state)?;
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
        }
        DebugCommand::Text { text } => {
            let value = client
                .text(&text)
                .or_else(|_| client.call(LEGACY_TEXT, json!({ "text": text })))?;
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        DebugCommand::Click { x, y, state } => {
            let value = client
                .click(x, y, &state)
                .or_else(|_| client.call(LEGACY_CLICK, json!({ "x": x, "y": y, "state": state })))?;
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        DebugCommand::Move { x, y } => {
            let value = client
                .move_cursor(x, y)
                .or_else(|_| client.call(LEGACY_MOVE, json!({ "x": x, "y": y })))?;
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        DebugCommand::Lua { code, file } => {
            let code = match (code, file) {
                (Some(code), None) => code,
                (None, Some(file)) => std::fs::read_to_string(&file)
                    .with_context(|| format!("a ler {}", file.display()))?,
                _ => bail!("`debug lua` precisa de CODE ou --file"),
            };
            let value = client.call(LEGACY_LUA, json!({ "code": code }))?;
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        DebugCommand::Pause => {
            let value = client.call(METHOD_PAUSE, json!({}))?;
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        DebugCommand::Play => {
            let value = client.call(METHOD_PLAY, json!({}))?;
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        DebugCommand::Step { frames } => {
            let value = client.call(METHOD_STEP, json!({ "frames": frames }))?;
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        DebugCommand::State => {
            let snapshot = state_snapshot(&client)?;
            println!("{}", serde_json::to_string_pretty(&snapshot)?);
        }
        DebugCommand::Expect { expect, file } => {
            let raw = match (expect, file) {
                (Some(raw), None) => Some(raw),
                (None, Some(file)) => Some(std::fs::read_to_string(&file).with_context(|| {
                    format!("a ler {}", file.display())
                })?),
                (Some(_), Some(_)) => bail!("usa EXPECT ou --file, não ambos"),
                (None, None) => None,
            };
            let raw = match raw.as_deref() {
                // `@ficheiro` no argumento posicional (convenção do CLI).
                Some(raw) if raw.starts_with('@') => {
                    Some(std::fs::read_to_string(&raw[1..]).with_context(|| {
                        format!("a ler {}", &raw[1..])
                    })?)
                }
                other => other.map(str::to_string),
            };
            let verdict = expect_check(&client, raw.as_deref())?;
            let ok = verdict.get("ok").and_then(Value::as_bool).unwrap_or(false);
            println!("{}", serde_json::to_string_pretty(&verdict)?);
            if !ok {
                return Ok(ExitCode::FAILURE);
            }
        }
        DebugCommand::Until {
            expr,
            timeout_ms,
            interval_ms,
        } => {
            let outcome = until_poll(&client, &expr, timeout_ms, interval_ms, json_out)?;
            let result = json!({
                "ok": outcome.passed,
                "value": outcome.value,
                "polls": outcome.polls,
                "elapsed_ms": outcome.elapsed_ms,
            });
            println!("{}", serde_json::to_string_pretty(&result)?);
            if !outcome.passed {
                return Ok(ExitCode::FAILURE);
            }
        }
        DebugCommand::Scene {
            name,
            entry,
            wait,
            wait_timeout,
        } => {
            let requested = scene_change(&client, &name, entry.as_deref())?;
            let mut result = json!({ "requested": name, "entry": entry, "change": requested });
            let mut ok = true;
            if wait {
                let ready = poll_ready(&client, port, wait_timeout, json_out)?;
                result["ready"] = json!(ready);
                ok = ready;
            }
            println!("{}", serde_json::to_string_pretty(&result)?);
            if !ok {
                return Ok(ExitCode::FAILURE);
            }
        }
        DebugCommand::Playbook {
            file,
            continue_on_error,
            output,
        } => {
            return crate::playbook::execute(
                &client,
                port,
                &file,
                continue_on_error,
                json_out,
                output.as_deref(),
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

// ── primitivas partilhadas (o playbook usa as mesmas) ────────────────────

/// Resultado de um poll de `until`.
pub(crate) struct UntilOutcome {
    pub passed: bool,
    pub value: Value,
    pub polls: u64,
    pub elapsed_ms: u128,
}

/// Resposta crua do `viber.lua` ({ok, result|error, ...}).
pub(crate) fn lua_response(client: &BridgeClient, code: &str) -> Result<Value> {
    let value = client.call(LEGACY_LUA, json!({ "code": code }))?;
    if value.get("ok").and_then(Value::as_bool) != Some(true) {
        let error = value
            .get("error")
            .map(|error| error.as_str().unwrap_or("sem mensagem").to_string())
            .unwrap_or_else(|| value.to_string());
        bail!("lua: {error}");
    }
    Ok(value)
}

/// Avalia `code` e devolve o resultado Luau serializado.
pub(crate) fn lua_eval(client: &BridgeClient, code: &str) -> Result<Value> {
    Ok(lua_response(client, code)?
        .get("result")
        .cloned()
        .unwrap_or(Value::Null))
}

/// Verdade Luau: `false` e `nil` (null) são falsos; o resto é verdadeiro.
fn luau_truthy(value: &Value) -> bool {
    !matches!(value, Value::Null | Value::Bool(false))
}

/// Poll de uma expressão Luau até ser verdadeira. `json_out` silencia o
/// progresso (agentes lêem o resultado final).
pub(crate) fn until_poll(
    client: &BridgeClient,
    expr: &str,
    timeout_ms: u64,
    interval_ms: u64,
    _json_out: bool,
) -> Result<UntilOutcome> {
    let code = format!("return ({expr})");
    let started = Instant::now();
    let deadline = Duration::from_millis(timeout_ms.max(1));
    let interval = Duration::from_millis(interval_ms.max(50));
    let mut polls = 0u64;
    loop {
        polls += 1;
        let last = match lua_eval(client, &code) {
            Ok(value) => value,
            // Expressão com erro de sintaxe/execução: falha imediata com o
            // erro (pollar um erro 20× só atrasa o diagnóstico).
            Err(error) => bail!("{error}"),
        };
        if luau_truthy(&last) {
            return Ok(UntilOutcome {
                passed: true,
                value: last,
                polls,
                elapsed_ms: started.elapsed().as_millis(),
            });
        }
        if started.elapsed() >= deadline {
            return Ok(UntilOutcome {
                passed: false,
                value: last,
                polls,
                elapsed_ms: started.elapsed().as_millis(),
            });
        }
        std::thread::sleep(interval.min(deadline.saturating_sub(started.elapsed())));
    }
}

/// Estado do jogo via `viber.vael.qa_snapshot()` (JSON string do jogo).
pub(crate) fn state_snapshot(client: &BridgeClient) -> Result<Value> {
    let raw = lua_eval(client, "return viber.vael.qa_snapshot()")?;
    match &raw {
        // O jogo devolve uma string JSON; alguns caminhos podem devolver a
        // tabela já serializada — aceitar os dois.
        Value::String(text) => Ok(serde_json::from_str(text)
            .with_context(|| "qa_snapshot devolveu JSON inválido")?),
        value @ Value::Object(_) => Ok(value.clone()),
        other => bail!("qa_snapshot devolveu {other}; esperado JSON ou tabela"),
    }
}

/// Verifica um `expect` com as regras reais do jogo (`qa_check`). Sem
/// `raw`, usa o expect do cenário de QA aplicado no arranque.
pub(crate) fn expect_check(client: &BridgeClient, raw: Option<&str>) -> Result<Value> {
    let code = match raw {
        Some(raw) => format!("return viber.vael.qa_check({})", lua_string_literal(raw)),
        None => "return viber.vael.qa_check()".to_string(),
    };
    let value = lua_eval(client, &code)?;
    if value.get("failures").is_none() || value.get("ok").is_none() {
        bail!("qa_check devolveu {value}; esperado {{ok, failures}}");
    }
    Ok(value)
}

/// String literal Luau com escaping mínimo (para embutir JSON num chunk).
fn lua_string_literal(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 2);
    out.push('"');
    for c in raw.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Troca de cena com retry: o `ready` do interior anterior pode piscar antes
/// de o loader limpar o `busy()`, e o pedido imediato leva "já há uma troca
/// de cena em curso" — corrida transitória que um agente não devolve ter de
/// tratar. Até 5 tentativas com 2 s entre elas.
pub(crate) fn scene_change(
    client: &BridgeClient,
    name: &str,
    entry: Option<&str>,
) -> Result<Value> {
    let mut last = Value::Null;
    for attempt in 0..5 {
        match client.call(
            LEGACY_SCENE_CHANGE,
            json!({ "scene": name, "entry": entry }),
        ) {
            Ok(value) => return Ok(value),
            Err(error) => {
                let busy = error.to_string().contains("troca de cena em curso");
                last = json!({ "error": error.to_string() });
                if !busy || attempt == 4 {
                    bail!("{error}");
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    }
    Ok(last)
}

/// Screenshot com um retry: durante o assentar de uma cena nova o pedido
/// pode sair com EAGAIN transitório.
pub(crate) fn capture_screenshot(
    client: &BridgeClient,
    timeout_ms: u64,
) -> Result<(Vec<u8>, String)> {
    match client.screenshot(timeout_ms).or_else(|_| client.legacy_screenshot(timeout_ms)) {
        Ok(capture) => Ok(capture),
        Err(error) => {
            std::thread::sleep(Duration::from_secs(1));
            client
                .screenshot(timeout_ms)
                .or_else(|_| client.legacy_screenshot(timeout_ms))
                .with_context(|| format!("screenshot falhou duas vezes: {error}"))
        }
    }
}

/// Press/release/click de tecla com fallback legacy (o mapa
/// `normalize_key` é o que a UI do monólito lê).
pub(crate) fn send_key(client: &BridgeClient, key: &str, state: &str) -> Result<Value> {
    client.key(key, state, None).or_else(|_| {
        client.call(LEGACY_KEY, json!({ "key": normalize_key(key), "state": state }))
    })?;
    Ok(json!({ "key": key, "state": state }))
}

/// Poll de `ready` (véu de carregamento). Partilhado por `wait-ready`,
/// `scene --wait` e o playbook. Devolve false no timeout.
pub(crate) fn poll_ready(
    client: &BridgeClient,
    port: u16,
    timeout_secs: u64,
    quiet: bool,
) -> Result<bool> {
    let started = Instant::now();
    let deadline = Duration::from_secs(timeout_secs);
    let mut last_status = String::new();
    let mut warned_no_readiness = false;
    while started.elapsed() < deadline {
        if is_listening(port) {
            if let Ok(pong) = client.ping() {
                let (ready, status) = readiness_of(&pong, client, &mut warned_no_readiness, quiet);
                if ready {
                    return Ok(true);
                }
                if !quiet && status != last_status {
                    eprintln!(
                        "[{:>5.1}s] {status}",
                        started.elapsed().as_secs_f32()
                    );
                    last_status = status;
                }
            }
        } else if !quiet && last_status.is_empty() {
            eprintln!("à espera da bridge na porta {port}...");
            last_status = "-".into();
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Ok(false)
}

/// Lê `ready` de um pong, com o cinto e suspensório da transição: o ping
/// novo sem espelho de prontidão não traz `ready` — nesse caso pergunta ao
/// ping legacy (que tem o estado do véu enquanto o monólito viver). Sem
/// legacy nem espelho, é um runtime sem véu: aviso e trata como pronto.
fn readiness_of(
    pong: &Value,
    client: &BridgeClient,
    warned: &mut bool,
    quiet: bool,
) -> (bool, String) {
    match pong.get("ready") {
        Some(ready) => (
            ready.as_bool().unwrap_or(false),
            pong.pointer("/loading/status")
                .and_then(Value::as_str)
                .unwrap_or("a carregar")
                .to_string(),
        ),
        None => {
            if let Ok(legacy) = client.call(LEGACY_PING, json!({})) {
                if let Some(ready) = legacy.get("ready").and_then(Value::as_bool) {
                    let status = legacy
                        .pointer("/loading/status")
                        .and_then(Value::as_str)
                        .unwrap_or("a carregar")
                        .to_string();
                    return (ready, status);
                }
            }
            if !*warned {
                *warned = true;
                if !quiet {
                    eprintln!(
                        "aviso: pong sem prontidão (runtime sem espelho nem monólito); a tratar como pronto"
                    );
                }
            }
            (true, "sem sinal de prontidão".into())
        }
    }
}

/// `wait-ready` (saída humana histórica + `--json`).
fn wait_ready(port: u16, timeout: u64, quiet: bool, json_out: bool) -> Result<ExitCode> {
    let started = Instant::now();
    let client = BridgeClient::localhost(port);
    let ready = poll_ready(&client, port, timeout, quiet || json_out)?;
    if ready {
        // Detalhe do carregamento para o resultado (o ping atual tem o
        // estado da última época).
        let pong = client.ping().ok().unwrap_or(Value::Null);
        let secs = pong
            .pointer("/loading/last_load_secs")
            .and_then(Value::as_f64);
        let timed_out = pong
            .pointer("/loading/timed_out")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if json_out {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "ready": true,
                    "elapsed_s": started.elapsed().as_secs_f64(),
                    "loading": pong.get("loading").cloned().unwrap_or(Value::Null),
                }))?
            );
        } else {
            match (secs, timed_out) {
                (_, true) => println!(
                    "pronto (INCOMPLETO: o carregamento esgotou o teto) após {:.1}s",
                    started.elapsed().as_secs_f32()
                ),
                (Some(load), _) => println!(
                    "pronto (mundo carregou em {load:.1}s; espera {:.1}s)",
                    started.elapsed().as_secs_f32()
                ),
                (None, _) => println!("pronto"),
            }
        }
        return Ok(ExitCode::SUCCESS);
    }
    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "ready": false,
                "timeout_s": timeout,
                "elapsed_s": started.elapsed().as_secs_f64(),
            }))?
        );
    } else {
        eprintln!("timeout: o jogo não ficou pronto em {timeout}s (porta {port})");
    }
    Ok(ExitCode::from(1))
}

/// `debug engines`: sessões com estado de vida (pid + porta TCP).
fn engines(prune: bool, json_out: bool) -> Result<ExitCode> {
    let mut sessions = Vec::new();
    for (slug, paths) in SessionPaths::all() {
        let info = paths.engine_info();
        let alive = info
            .as_ref()
            .map(|info| pid_alive(info.pid) && is_listening(info.port))
            .unwrap_or(false);
        if prune && !alive && info.is_some() {
            paths.clear_engine();
        }
        sessions.push(json!({
            "slug": slug,
            "alive": alive,
            "engine_file": info.is_some(),
            "info": info,
        }));
    }
    if json_out {
        println!("{}", serde_json::to_string_pretty(&json!({ "sessions": sessions }))?);
    } else {
        let mut out = std::io::stdout().lock();
        for session in &sessions {
            let slug = session["slug"].as_str().unwrap_or("?");
            let alive = session["alive"].as_bool().unwrap_or(false);
            match &session["info"] {
                Value::Null => {
                    // `head`/pipes fecham o stdout; um CLI de lista não pode
                    // panicar com `Broken pipe` (o `println!` panica).
                    let _ = writeln!(out, "{slug}: (sem engine.json)");
                }
                info => {
                    let _ = writeln!(
                        out,
                        "{slug}: pid={} port={} world={} {}",
                        info["pid"],
                        info["port"],
                        info["world"],
                        if alive { "[vivo]" } else { "[morto]" },
                    );
                }
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// O ficheiro de log da sessão (`engine.json` → `log`). Com `--world`, a
/// sessão desse mundo; sem, a única sessão viva (para não apanhar a errada
/// quando há duas engines em portas diferentes).
fn session_log_path(world: Option<&Path>) -> Option<PathBuf> {
    let candidates: Vec<_> = match world {
        Some(target) => {
            let paths = SessionPaths::for_world(target);
            paths.engine_info().map(|info| vec![info]).unwrap_or_default()
        }
        None => SessionPaths::all()
            .into_iter()
            .filter_map(|(_, paths)| paths.engine_info())
            .filter(|info| pid_alive(info.pid))
            .collect(),
    };
    if candidates.len() != 1 {
        return None;
    }
    let path = PathBuf::from(&candidates[0].log);
    (path.is_file()).then_some(path)
}

/// Últimas `tail` linhas do ficheiro, filtradas por substring (sem
/// maiúsculas).
fn read_log_lines(path: &Path, tail: usize, grep: Option<&str>) -> Result<Vec<String>> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("a ler o log da sessão {}", path.display()))?;
    let needle = grep.map(str::to_lowercase);
    let lines: Vec<String> = raw
        .lines()
        .filter(|line| {
            needle
                .as_deref()
                .map(|needle| line.to_lowercase().contains(needle))
                .unwrap_or(true)
        })
        .map(str::to_string)
        .collect();
    let start = lines.len().saturating_sub(tail.max(1));
    Ok(lines[start..].to_vec())
}

/// Filtra as entradas do ring da bridge (`viber.logs`:
/// `{time, level, target, message}[]`) por substring da mensagem.
fn filter_log_entries(value: &Value, grep: Option<&str>) -> Value {
    let Some(grep) = grep else {
        return value.clone();
    };
    let needle = grep.to_lowercase();
    let entries = value
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|entry| {
            entry
                .get("message")
                .and_then(Value::as_str)
                .map(|message| message.to_lowercase().contains(&needle))
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    json!(entries)
}

fn pid_alive(pid: u32) -> bool {
    Path::new("/proc").join(pid.to_string()).exists()
}

fn emit(json_out: bool, human: String, value: &Value) -> Result<()> {
    if json_out {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else {
        println!("{human}");
    }
    Ok(())
}

fn call_fallback(
    client: &BridgeClient,
    method: &str,
    legacy: &str,
    params: Value,
) -> Result<Value> {
    client
        .call(method, params.clone())
        .or_else(|_| client.call(legacy, params))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lua_string_literal_escapes_quotes_and_controls() {
        // JSON do expect leva `"` em quantidade; sem escaping o chunk não
        // compila. `\n` literal (e não newline real) no chunk também.
        let literal = lua_string_literal(r#"{"quests":{"a":"done"}}"#);
        assert_eq!(literal, r#""{\"quests\":{\"a\":\"done\"}}""#);
        assert_eq!(lua_string_literal("a\nb"), "\"a\\nb\"");
        assert_eq!(lua_string_literal("sim"), "\"sim\"");
    }

    #[test]
    fn luau_truthy_only_false_and_null_are_falsy() {
        assert!(!luau_truthy(&Value::Null));
        assert!(!luau_truthy(&Value::Bool(false)));
        // Luau: 0 é verdadeiro (só false/nil são falsos).
        assert!(luau_truthy(&json!(0)));
        assert!(luau_truthy(&Value::Bool(true)));
        assert!(luau_truthy(&json!("")));
    }

    #[test]
    fn read_log_lines_tail_and_grep_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("game.log");
        std::fs::write(
            &path,
            "INFO a arrancar\nERROR script falhou\nwarn: GPU lenta\nINFO pronto\n",
        )
        .unwrap();
        let lines = read_log_lines(&path, 100, None).unwrap();
        assert_eq!(lines.len(), 4);
        let errors = read_log_lines(&path, 100, Some("error")).unwrap();
        assert_eq!(errors, vec!["ERROR script falhou".to_string()]);
        // O tail é DEPOIS do filtro: 2 linhas batem o grep, pede-se 1.
        let last = read_log_lines(&path, 1, Some("info")).unwrap();
        assert_eq!(last, vec!["INFO pronto".to_string()]);
    }

    #[test]
    fn filter_log_entries_matches_message_only() {
        let value = json!([
            { "time": 1.0, "level": "INFO", "target": "vael", "message": "mundo pronto" },
            { "time": 2.0, "level": "ERROR", "target": "lua", "message": "quest falhou" }
        ]);
        let filtered = filter_log_entries(&value, Some("pronto"));
        assert_eq!(filtered.as_array().unwrap().len(), 1);
        assert_eq!(filtered[0]["level"], "INFO");
        // Sem grep devolve tudo.
        assert_eq!(
            filter_log_entries(&value, None).as_array().unwrap().len(),
            2
        );
    }
}

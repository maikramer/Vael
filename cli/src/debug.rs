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

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use clap::Subcommand;
use serde_json::{Value, json};

use bevy_ext_bridge_protocol::{
    BridgeClient, LEGACY_CLICK, LEGACY_KEY, LEGACY_LOGS, LEGACY_LUA, LEGACY_MOVE, LEGACY_PING,
    LEGACY_PROFILER, LEGACY_PROFILER_MARK, LEGACY_PROFILER_TAB, LEGACY_TEXT, LEGACY_TREE,
    METHOD_FRAME_STATS, METHOD_HITCHES, METHOD_LOGS, METHOD_MARK, METHOD_MEMORY, METHOD_PAUSE,
    METHOD_PLAY, METHOD_STEP, METHOD_TREE, SessionPaths, is_listening, normalize_key, resolve_port,
};

/// Troca de cena (só no monólito; a duração depende do mundo).
const LEGACY_SCENE_CHANGE: &str = "viber.scene.change";
/// Cena carregada + época + prontidão (só no monólito). É o que permite
/// esperar pela cena NOVA sem aceitar o `ready` da anterior.
const LEGACY_SCENE_CURRENT: &str = "viber.scene.current";
/// Leituras `ready` seguidas que bastam quando o bridge não expõe cena nem
/// época (runtime novo): 4 × 500 ms ≈ 2 s de estabilidade em vez de um frame.
const SCENE_STABLE_POLLS: u32 = 4;
/// Bloco da leitura do log de trás para a frente (`read_log_lines`).
const LOG_CHUNK: u64 = 64 * 1024;

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
    // `logs` do ficheiro da sessão NÃO precisa de bridge vivo — o log
    // pós-crash (jogo morto) é precisamente o que se quer ler. Só o ring do
    // jogo (`--source bridge` ou `auto` sem ficheiro) exige a porta ocupada.
    if let DebugCommand::Logs { tail, grep, source } = &command {
        let mode = log_source_mode(source)?;
        if mode != "bridge" {
            if logs_from_file(world.as_deref(), *tail, grep.as_deref(), json_out)? {
                return Ok(ExitCode::SUCCESS);
            }
            if mode == "file" {
                bail!(
                    "--source file: sem ficheiro de log de sessão (lança com `vael-tools run --bridge`)"
                );
            }
        }
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
            // O `viber.tree` do monólito só aceita `max_depth`: no fallback o
            // filtro é aplicado aqui à árvore devolvida (antes era ignorado em
            // silêncio e `--filter` parecia funcionar com tudo).
            let value = client
                .call(METHOD_TREE, json!({ "filter": filter }))
                .or_else(|_| {
                    client
                        .call(LEGACY_TREE, json!({}))
                        .map(|tree| filter_tree(&tree, filter.as_deref()))
                })?;
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        DebugCommand::Logs { tail, grep, .. } => {
            // Só se chega aqui com `--source bridge` ou `auto` sem ficheiro
            // de sessão (o ficheiro é servido antes do probe do bridge).
            let value = call_fallback(
                &client,
                METHOD_LOGS,
                LEGACY_LOGS,
                json!({ "limit": tail }),
            )?;
            let entries = filter_log_entries(&value, grep.as_deref());
            emit(
                json_out,
                serde_json::to_string_pretty(&entries)?,
                &json!({ "source": "bridge", "entries": entries }),
            )?;
        }
        DebugCommand::Memory => {
            let value = profiler_field(&client, METHOD_MEMORY, "memory")?;
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        DebugCommand::Hitches => {
            let value = profiler_field(&client, METHOD_HITCHES, "hitches")?;
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
                let mut series: Vec<Value> = Vec::new();
                for sample in 0..samples {
                    let value = client
                        .call(METHOD_FRAME_STATS, json!({}))
                        .or_else(|_| client.call(LEGACY_PROFILER_TAB, json!({ "tab": tab })))?;
                    match file.as_mut() {
                        Some(file) => {
                            writeln!(file, "{value}")?;
                        }
                        // Sem `-o` as amostras são JSONL no stdout. Com
                        // `--json` o stdout tem de ser UM documento: a série
                        // é impressa junta no fim (antes iam linhas soltas).
                        None if json => series.push(value),
                        // Sem `-o`, as amostras iam para o vazio (o comando
                        // saía com 0 e sem output): JSONL no stdout.
                        None => println!("{value}"),
                    }
                    // Sem `sleep` depois da última amostra (com `--hz` baixo
                    // eram segundos a mais no fim do comando).
                    if sample + 1 < samples {
                        std::thread::sleep(std::time::Duration::from_secs_f64(1.0 / hz));
                    }
                }
                if let Some(path) = &output {
                    if json {
                        // `--json` = só JSON no stdout (o "perf:" era texto
                        // solto que os agentes não conseguiam parsear).
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&json!({
                                "samples": samples,
                                "path": path.display().to_string(),
                            }))?
                        );
                    } else {
                        println!("perf: {} amostra(s) em {}", samples, path.display());
                    }
                } else if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&json!({
                            "samples": samples,
                            "series": series,
                        }))?
                    );
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
                (Some(_), Some(_)) => bail!("usa CODE ou --file, não ambos"),
                (None, None) => bail!("`debug lua` precisa de CODE ou --file"),
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
            // Marca a cena ANTES do pedido: o `ready` do ping pode ainda ser o
            // da cena anterior por um frame (e uma troca para a MESMA cena tem
            // de esperar a época, senão o `--wait` saía de imediato).
            let before = wait.then(|| scene_marker(&client));
            let requested = scene_change(&client, &name, entry.as_deref())?;
            let mut result = json!({ "requested": name, "entry": entry, "change": requested });
            let mut ok = true;
            if let Some(before) = &before {
                let ready =
                    poll_ready_for_scene(&client, port, &name, before, wait_timeout, json_out)?;
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

/// Fotografia da cena antes de pedir uma troca (`poll_ready_for_scene`). O
/// id da cena não entra: a leitura nova é comparada com o pedido; o que a
/// marca dá é a ÉPOCA (para a troca para a mesma cena) e o `ready` (para o
/// runtime antigo, sem cena).
#[derive(Debug, Clone, Default)]
pub(crate) struct SceneMarker {
    /// Época do loader: sobe a cada troca (apanha a troca para a MESMA cena).
    epoch: Option<u64>,
    /// `ready` do ping (o runtime novo não expõe cena nem época).
    ready: Option<bool>,
}

/// Estado da cena agora: `viber.scene.current` (monólito) ou o `ready` do
/// ping. O runtime novo só tem o segundo.
struct SceneSnapshot {
    scene: Option<String>,
    epoch: Option<u64>,
    ready: Option<bool>,
    status: Option<String>,
}

fn scene_snapshot(client: &BridgeClient) -> Result<SceneSnapshot> {
    if let Ok(value) = client.call(LEGACY_SCENE_CURRENT, json!({}))
        && value.is_object()
    {
        let text = |value: Option<&Value>| value.and_then(Value::as_str).map(str::to_string);
        return Ok(SceneSnapshot {
            scene: text(value.get("scene")),
            epoch: value.get("epoch").and_then(Value::as_u64),
            ready: value.get("ready").and_then(Value::as_bool),
            status: text(value.pointer("/loading/status")),
        });
    }
    let pong = client.ping()?;
    let text = |value: Option<&Value>| value.and_then(Value::as_str).map(str::to_string);
    Ok(SceneSnapshot {
        scene: text(pong.get("scene")),
        epoch: pong.get("epoch").and_then(Value::as_u64),
        ready: pong.get("ready").and_then(Value::as_bool),
        status: text(pong.pointer("/loading/status")),
    })
}

/// A marca da cena actual (para tirar ANTES de `scene_change`). Sem bridge
/// que responda, tudo `None` — a espera degrada para o `ready` do ping.
pub(crate) fn scene_marker(client: &BridgeClient) -> SceneMarker {
    match scene_snapshot(client) {
        Ok(snapshot) => SceneMarker {
            epoch: snapshot.epoch,
            ready: snapshot.ready,
        },
        Err(_) => SceneMarker::default(),
    }
}

/// Normaliza um nome de cena para comparar o pedido com o `current`:
/// `scenes/menu.bsn`, `./menu.bsn` e `menu` são a mesma cena.
fn normalize_scene(name: &str) -> String {
    let name = name.trim().trim_end_matches('/');
    let name = name.strip_prefix("./").unwrap_or(name);
    let name = name.strip_prefix("scenes/").unwrap_or(name);
    name.strip_suffix(".bsn")
        .or_else(|| name.strip_suffix(".xml"))
        .unwrap_or(name)
        .to_string()
}

fn scene_matches(want: &str, got: &str) -> bool {
    let want = normalize_scene(want);
    let got = normalize_scene(got);
    if want.is_empty() || got.is_empty() {
        return want == got;
    }
    want == got || want.ends_with(&format!("/{got}")) || got.ends_with(&format!("/{want}"))
}

/// Estado acumulado da espera da cena nova.
#[derive(Debug, Default, Clone, Copy)]
struct SceneWait {
    /// Já se viu `ready: false` depois do pedido (o véu subiu).
    seen_not_ready: bool,
    /// Leituras seguidas com a cena nova pronta.
    stable: u32,
}

/// Veredicto de uma leitura na espera da cena nova.
#[derive(Debug, PartialEq, Eq)]
enum SceneWaitStep {
    /// A cena nova está pronta — a espera acabou.
    Ready,
    /// Ainda não (ou o sinal pode ser o da cena anterior).
    Wait,
    /// Não há como distinguir (bridge sem época e véu que nunca apareceu):
    /// aceitar ao fim da janela de estabilidade, com aviso.
    Stale,
}

/// Consome uma leitura do bridge. Função pura: é a regra que fecha a corrida
/// do `scene --wait` (o `ready` da cena anterior não conta como pronto).
fn scene_wait_step(
    wait: &mut SceneWait,
    before: &SceneMarker,
    snapshot: &SceneSnapshot,
    want: &str,
) -> SceneWaitStep {
    if snapshot.ready == Some(false) {
        wait.seen_not_ready = true;
    }
    // Sem espelho de prontidão não há véu a esperar (o mesmo tratamento do
    // `wait-ready`).
    let ready = snapshot.ready.unwrap_or(true);
    let scene_ok = snapshot
        .scene
        .as_deref()
        .map(|got| scene_matches(want, got))
        .unwrap_or(true);
    if !ready || !scene_ok {
        wait.stable = 0;
        return SceneWaitStep::Wait;
    }
    // Vimos o véu a subir DEPOIS do pedido: o `ready` é desta cena.
    if wait.seen_not_ready {
        return SceneWaitStep::Ready;
    }
    // Sem ter visto o véu, exigir duas leituras seguidas com a época à
    // frente: há um frame em que a época já subiu mas o véu ainda não
    // recomeçou (troca para a MESMA cena, ou `ready` velho).
    let epoch_fresh = match (before.epoch, snapshot.epoch) {
        (Some(before), Some(now)) => now > before,
        // Sem época no bridge, só é atribuível à cena nova se o `ready` de
        // antes era falso.
        _ => before.ready != Some(true),
    };
    wait.stable += 1;
    if epoch_fresh && wait.stable >= 2 {
        return SceneWaitStep::Ready;
    }
    if wait.stable >= SCENE_STABLE_POLLS {
        return SceneWaitStep::Stale;
    }
    SceneWaitStep::Wait
}

/// Espera a cena NOVA ficar pronta. `before` é a marca tirada antes do pedido
/// de troca: sem ela, o `ready` do ping pode ser o da cena anterior por um
/// frame (o bug do `scene --wait` que voltava "pronto" antes da troca).
pub(crate) fn poll_ready_for_scene(
    client: &BridgeClient,
    port: u16,
    want: &str,
    before: &SceneMarker,
    timeout_secs: u64,
    quiet: bool,
) -> Result<bool> {
    let started = Instant::now();
    let deadline = Duration::from_secs(timeout_secs);
    let mut last_status = String::new();
    let mut warned_no_readiness = false;
    let mut wait = SceneWait::default();
    while started.elapsed() < deadline {
        if is_listening(port) {
            let snapshot = match scene_snapshot(client) {
                Ok(snapshot) => Some(snapshot),
                // Bridge a tremer durante o carregamento é normal: voltar a
                // tentar em vez de abortar a espera.
                Err(_) => None,
            };
            if let Some(snapshot) = snapshot {
                let status = snapshot
                    .status
                    .clone()
                    .unwrap_or_else(|| "a carregar".into());
                if !quiet && status != last_status {
                    eprintln!("[{:>5.1}s] {status}", started.elapsed().as_secs_f32());
                    last_status = status;
                }
                if snapshot.ready.is_none() && !warned_no_readiness {
                    warned_no_readiness = true;
                    if !quiet {
                        eprintln!(
                            "aviso: pong sem prontidão; a espera da cena usa o pedido de troca"
                        );
                    }
                }
                match scene_wait_step(&mut wait, before, &snapshot, want) {
                    SceneWaitStep::Ready => return Ok(true),
                    SceneWaitStep::Stale => {
                        if !quiet {
                            eprintln!(
                                "aviso: bridge sem época de cena; `--wait` aceita o `ready` após \
                                 {SCENE_STABLE_POLLS} leituras (o véu pode ser o da cena anterior)"
                            );
                        }
                        return Ok(true);
                    }
                    SceneWaitStep::Wait => {}
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

/// Valida `--source` (o erro de uso sai antes de qualquer probe à porta).
fn log_source_mode(source: &str) -> Result<&str> {
    match source {
        "auto" | "file" | "bridge" => Ok(source),
        other => bail!("--source inválido `{other}` (auto | file | bridge)"),
    }
}

/// Serve os logs do ficheiro da sessão (não precisa do bridge vivo).
/// `Ok(false)` quando não há ficheiro de sessão para ler.
fn logs_from_file(
    world: Option<&Path>,
    tail: usize,
    grep: Option<&str>,
    json_out: bool,
) -> Result<bool> {
    let Some(path) = session_log_path(world) else {
        return Ok(false);
    };
    let lines = read_log_lines(&path, tail, grep)?;
    emit(
        json_out,
        lines.join("\n"),
        &json!({
            "source": "file",
            "path": path.display().to_string(),
            "lines": lines,
        }),
    )?;
    Ok(true)
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
///
/// A leitura é de trás para a frente em blocos de [`LOG_CHUNK`]: um log de
/// centenas de MB não entra todo em memória (antes era `read_to_string` +
/// `lines()`) e, sem `--grep` (ou com os matches recentes), pára mal junta as
/// linhas pedidas. As linhas nunca são cortadas: os blocos são juntados às
/// sobras da linha anterior antes de partir por `\n`.
fn read_log_lines(path: &Path, tail: usize, grep: Option<&str>) -> Result<Vec<String>> {
    // `--tail 0` = nenhuma linha; o `max(1)` antigo devolvia sempre a última.
    if tail == 0 {
        return Ok(Vec::new());
    }
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("a ler o log da sessão {}", path.display()))?;
    let file_len = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    let needle = grep.map(str::to_lowercase);
    let matches = |line: &str| {
        needle
            .as_deref()
            .map(|needle| line.to_lowercase().contains(needle))
            .unwrap_or(true)
    };
    let mut length = file_len;
    // `str::lines` não devolve a linha vazia depois do `\n` final: ler até
    // antes desse `\n` mantém a contagem igual (a linha em branco que o
    // ficheiro tenha antes continua a sair).
    if length > 0 {
        let mut last = [0u8; 1];
        if file.seek(SeekFrom::Start(length - 1)).is_ok()
            && file.read_exact(&mut last).is_ok()
            && last[0] == b'\n'
        {
            length -= 1;
        }
    }
    // Só um `\n` é UMA linha em branco (`str::lines("\\n")`); vazio é zero.
    if length == 0 {
        return Ok(if file_len > 0 && matches("") {
            vec![String::new()]
        } else {
            Vec::new()
        });
    }
    let mut collected: Vec<String> = Vec::new(); // mais recente primeiro
    let mut pending: Vec<u8> = Vec::new(); // início da linha ainda por fechar
    let mut pos = length;
    'blocks: while pos > 0 {
        let read_len = LOG_CHUNK.min(pos);
        pos -= read_len;
        let mut block = vec![0u8; read_len as usize];
        file.seek(SeekFrom::Start(pos))?;
        file.read_exact(&mut block)?;
        block.extend_from_slice(&pending);
        pending.clear();
        let mut end = block.len();
        while end > 0 {
            let Some(split) = block[..end].iter().rposition(|&byte| byte == b'\n') else {
                break;
            };
            let line = decode_log_line(&block[split + 1..end]);
            if matches(&line) {
                collected.push(line);
                if collected.len() >= tail {
                    break 'blocks;
                }
            }
            end = split;
        }
        pending.extend_from_slice(&block[..end]);
    }
    // A primeira linha do ficheiro (a que sobra depois de chegar ao início)
    // não termina em `\n` e não passou pelo ciclo — mesmo quando é vazia
    // (um ficheiro que começa por `\n` tem uma primeira linha em branco).
    if pos == 0 && length > 0 && collected.len() < tail {
        let line = decode_log_line(&pending);
        if matches(&line) {
            collected.push(line);
        }
    }
    collected.reverse();
    Ok(collected)
}

/// Bytes de uma linha do log → `String` (lossy: um log com um byte inválido
/// não pode rebentar a leitura) sem o `\r` do CRLF.
fn decode_log_line(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    text.strip_suffix('\r').unwrap_or(&text).to_string()
}

/// Filtra as entradas do ring da bridge (`viber.logs`:
/// `{time, level, target, message}[]`) por substring. Procura em `message`,
/// `level` e `target` (o `--grep` promete "linhas que contêm o texto"; só na
/// mensagem, `--grep ERROR` não via nenhum erro). Aceita a lista nua e o
/// objecto `{entries: […], …}` (a forma de um ring novo): no objecto o filtro
/// aplica-se a `entries` e o resto do payload fica.
fn filter_log_entries(value: &Value, grep: Option<&str>) -> Value {
    let Some(grep) = grep else {
        return value.clone();
    };
    let needle = grep.to_lowercase();
    let keep = |entries: &Vec<Value>| -> Vec<Value> {
        entries
            .iter()
            .filter(|entry| {
                ["message", "level", "target"].iter().any(|key| {
                    entry
                        .get(*key)
                        .and_then(Value::as_str)
                        .map(|field| field.to_lowercase().contains(&needle))
                        .unwrap_or(false)
                })
            })
            .cloned()
            .collect()
    };
    let Some(entries) = value.as_array() else {
        // Forma com `entries`: filtra lá dentro (antes o `--grep` devolvia
        // tudo em silêncio quando o ring vinha embrulhado num objecto).
        if let (Some(entries), Some(object)) = (
            value.get("entries").and_then(Value::as_array),
            value.as_object(),
        ) {
            let mut out = object.clone();
            out.insert("entries".into(), json!(keep(entries)));
            return Value::Object(out);
        }
        // Forma inesperada: devolver o que veio em vez de `[]` (mascarava).
        return value.clone();
    };
    json!(keep(entries))
}

/// Aplica o `--filter` à árvore devolvida pelo `viber.tree` (o monólito só
/// aceita `max_depth`). Aceita a lista nua de nós (`viber.tree`) e o objecto
/// `{entities|nodes: […], truncated}`.
fn filter_tree(tree: &Value, filter: Option<&str>) -> Value {
    let Some(filter) = filter else {
        return tree.clone();
    };
    let needle = filter.to_lowercase();
    let keep = |nodes: &Vec<Value>| -> Value {
        json!(
            nodes
                .iter()
                .filter(|node| {
                    node.get("name")
                        .and_then(Value::as_str)
                        .map(|name| name.to_lowercase().contains(&needle))
                        .unwrap_or(false)
                })
                .cloned()
                .collect::<Vec<_>>()
        )
    };
    match tree {
        Value::Array(nodes) => keep(nodes),
        Value::Object(object) => {
            let mut out = object.clone();
            for key in ["entities", "nodes"] {
                if let Some(Value::Array(nodes)) = object.get(key) {
                    out.insert(key.into(), keep(nodes));
                }
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

/// Snapshot de `method` com fallback para o `viber.profiler` do monólito
/// (que publica `memory` e `hitches`). Sem nenhum dos dois é ERRO: um sucesso
/// vazio escondia que o bridge não serve o dado.
fn profiler_field(client: &BridgeClient, method: &str, field: &str) -> Result<Value> {
    let native = match client.call(method, json!({})) {
        Ok(value) => return Ok(value),
        Err(error) => error,
    };
    let profiler = client
        .call(LEGACY_PROFILER, json!({}))
        .map_err(|legacy| anyhow!("{method} e viber.profiler falharam ({native:#}; {legacy:#})"))?;
    match profiler.get(field) {
        Some(value) if !value.is_null() => Ok(value.clone()),
        _ => bail!(
            "o bridge não publica `{field}` (nem `{method}` nem `viber.profiler` com `{field}`)"
        ),
    }
}

fn pid_alive(pid: u32) -> bool {
    // `/proc` só existe em Linux. Noutra plataforma não se pode provar que o
    // pid morreu: assumir vivo evita o `engines --prune` apagar sessões
    // válidas (e o `logs` deixar de encontrar o ficheiro da sessão).
    if !cfg!(target_os = "linux") {
        return true;
    }
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
        // `--tail 0` não devolve a última linha (o `max(1)` fazia-o).
        assert!(read_log_lines(&path, 0, None).unwrap().is_empty());
    }

    #[test]
    fn filter_log_entries_matches_message_level_or_target() {
        let value = json!([
            { "time": 1.0, "level": "INFO", "target": "vael", "message": "mundo pronto" },
            { "time": 2.0, "level": "ERROR", "target": "lua", "message": "quest falhou" }
        ]);
        let filtered = filter_log_entries(&value, Some("pronto"));
        assert_eq!(filtered.as_array().unwrap().len(), 1);
        assert_eq!(filtered[0]["level"], "INFO");
        // O nível também conta como "texto da linha" (`--grep ERROR`).
        let errors = filter_log_entries(&value, Some("error"));
        assert_eq!(errors.as_array().unwrap().len(), 1);
        assert_eq!(errors[0]["target"], "lua");
        // O target também.
        assert_eq!(
            filter_log_entries(&value, Some("VAEL"))
                .as_array()
                .unwrap()
                .len(),
            1
        );
        // Sem grep devolve tudo.
        assert_eq!(
            filter_log_entries(&value, None).as_array().unwrap().len(),
            2
        );
        // Forma inesperada: devolve o valor em vez de uma lista vazia.
        let object = json!({ "entries": [] });
        assert_eq!(filter_log_entries(&object, Some("x")), object);
    }

    #[test]
    fn filter_log_entries_filters_inside_an_entries_object() {
        // Um ring novo pode vir embrulhado; o `--grep` tem de filtrar lá
        // dentro (antes devolvia o objecto inteiro em silêncio).
        let value = json!({
            "entries": [
                { "level": "INFO", "message": "mundo pronto" },
                { "level": "ERROR", "message": "quest falhou" }
            ],
            "dropped": 3
        });
        let filtered = filter_log_entries(&value, Some("falhou"));
        assert_eq!(filtered["entries"].as_array().unwrap().len(), 1);
        assert_eq!(filtered["entries"][0]["message"], "quest falhou");
        assert_eq!(filtered["dropped"], 3);
    }

    #[test]
    fn filter_tree_applies_the_filter_to_the_legacy_list_and_object() {
        // Regressão: `debug tree --filter` contra o `viber.tree` (só aceita
        // `max_depth`) devolvia a árvore inteira — o filtro era ignorado.
        let list = json!([
            { "id": "1", "name": "Hero" },
            { "id": "2", "name": "wolf" },
            { "id": "3" }
        ]);
        let filtered = filter_tree(&list, Some("WOLF"));
        assert_eq!(filtered.as_array().unwrap().len(), 1);
        assert_eq!(filtered[0]["name"], "wolf");
        assert_eq!(filter_tree(&list, None), list);
        let object = json!({
            "entities": [{ "name": "Hero" }, { "name": "crate" }],
            "truncated": true
        });
        let filtered = filter_tree(&object, Some("crate"));
        assert_eq!(filtered["entities"].as_array().unwrap().len(), 1);
        assert_eq!(filtered["entities"][0]["name"], "crate");
        assert_eq!(filtered["truncated"], true);
    }

    #[test]
    fn scene_names_match_across_paths_and_extensions() {
        assert!(scene_matches("main", "main"));
        assert!(scene_matches("scenes/menu.bsn", "menu"));
        assert!(scene_matches("menu", "scenes/menu.bsn"));
        assert!(scene_matches("qa/city-forge", "qa/city-forge"));
        assert!(!scene_matches("menu", "main"));
        assert!(!scene_matches("", "main"));
        assert!(!scene_matches("menu", ""));
    }

    fn marker(epoch: Option<u64>, ready: Option<bool>) -> SceneMarker {
        SceneMarker { epoch, ready }
    }

    fn snapshot(scene: Option<&str>, epoch: Option<u64>, ready: Option<bool>) -> SceneSnapshot {
        SceneSnapshot {
            scene: scene.map(str::to_string),
            epoch,
            ready,
            status: None,
        }
    }

    #[test]
    fn scene_wait_does_not_accept_the_previous_scene_ready() {
        // O bug: o `ready` do ping ainda era o da cena anterior. A primeira
        // leitura (mesma cena, mesma época) não pode fechar a espera.
        let before = marker(Some(5), Some(true));
        let mut wait = SceneWait::default();
        assert_eq!(
            scene_wait_step(
                &mut wait,
                &before,
                &snapshot(Some("main"), Some(5), Some(true)),
                "main"
            ),
            SceneWaitStep::Wait
        );
        // A troca começou: época nova e véu em pé.
        assert_eq!(
            scene_wait_step(
                &mut wait,
                &before,
                &snapshot(Some("main"), Some(6), Some(false)),
                "main"
            ),
            SceneWaitStep::Wait
        );
        // Véu saiu: agora é a cena nova.
        assert_eq!(
            scene_wait_step(
                &mut wait,
                &before,
                &snapshot(Some("main"), Some(6), Some(true)),
                "main"
            ),
            SceneWaitStep::Ready
        );
    }

    #[test]
    fn scene_wait_needs_two_readings_when_the_veil_is_not_seen() {
        // Troca para a MESMA cena: há um frame com a época já à frente mas
        // com o `ready` ainda da anterior. Uma leitura não basta.
        let before = marker(Some(5), Some(true));
        let mut wait = SceneWait::default();
        assert_eq!(
            scene_wait_step(
                &mut wait,
                &before,
                &snapshot(Some("main"), Some(6), Some(true)),
                "main"
            ),
            SceneWaitStep::Wait
        );
        assert_eq!(
            scene_wait_step(
                &mut wait,
                &before,
                &snapshot(Some("main"), Some(6), Some(true)),
                "main"
            ),
            SceneWaitStep::Ready
        );
    }

    #[test]
    fn scene_wait_ignores_the_old_scene_while_the_new_one_loads() {
        let before = marker(Some(5), Some(true));
        let mut wait = SceneWait::default();
        assert_eq!(
            scene_wait_step(
                &mut wait,
                &before,
                &snapshot(Some("main"), Some(6), Some(true)),
                "house"
            ),
            SceneWaitStep::Wait
        );
        assert_eq!(
            scene_wait_step(
                &mut wait,
                &before,
                &snapshot(Some("house"), Some(6), Some(false)),
                "house"
            ),
            SceneWaitStep::Wait
        );
        assert_eq!(
            scene_wait_step(
                &mut wait,
                &before,
                &snapshot(Some("house"), Some(6), Some(true)),
                "house"
            ),
            SceneWaitStep::Ready
        );
    }

    #[test]
    fn scene_wait_without_scene_info_falls_back_to_stability() {
        // Runtime sem `viber.scene.current`: o véu visto chega; sem véu (um
        // `ready` que nunca caiu) só ao fim da janela de estabilidade.
        let before = marker(None, Some(true));
        let mut wait = SceneWait::default();
        for _ in 0..SCENE_STABLE_POLLS - 1 {
            assert_eq!(
                scene_wait_step(&mut wait, &before, &snapshot(None, None, Some(true)), "main"),
                SceneWaitStep::Wait
            );
        }
        assert_eq!(
            scene_wait_step(&mut wait, &before, &snapshot(None, None, Some(true)), "main"),
            SceneWaitStep::Stale
        );
        // Com o véu visto, a primeira leitura pronta fecha.
        let mut wait = SceneWait::default();
        assert_eq!(
            scene_wait_step(&mut wait, &before, &snapshot(None, None, Some(false)), "main"),
            SceneWaitStep::Wait
        );
        assert_eq!(
            scene_wait_step(&mut wait, &before, &snapshot(None, None, Some(true)), "main"),
            SceneWaitStep::Ready
        );
    }

    #[test]
    fn read_log_lines_is_chunked_and_keeps_lines_intact() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.log");
        // Uma linha maior do que um bloco (o corte cai a meio dela) e muitas
        // linhas pequenas: a leitura de trás para a frente tem de a manter
        // inteira.
        let long = format!("INICIO {}", "x".repeat((LOG_CHUNK as usize) + 123));
        let mut content = String::new();
        content.push_str(&long);
        content.push('\n');
        for index in 0..5_000 {
            content.push_str(&format!("linha {index}\n"));
        }
        std::fs::write(&path, &content).unwrap();

        let last = read_log_lines(&path, 3, None).unwrap();
        assert_eq!(
            last,
            vec![
                "linha 4997".to_string(),
                "linha 4998".to_string(),
                "linha 4999".to_string()
            ]
        );
        // O grep pode casar só na primeira linha: tem de ler o ficheiro todo
        // e devolver a linha longa intacta.
        let hits = read_log_lines(&path, 10, Some("inicio")).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0], long);
        // `str::lines` de referência (a contagem do ficheiro inteiro bate).
        assert_eq!(
            read_log_lines(&path, usize::MAX, None).unwrap(),
            content.lines().map(str::to_string).collect::<Vec<_>>()
        );
    }

    #[test]
    fn read_log_lines_matches_str_lines_on_edge_cases() {
        let dir = tempfile::tempdir().unwrap();
        for (name, content) in [
            ("empty.log", ""),
            ("blank.log", "\n"),
            ("no-eol.log", "a\nb"),
            ("crlf.log", "a\r\nb\r\n"),
            ("leading.log", "\na\nb\n"),
            ("blanks.log", "a\n\n\nb\n"),
        ] {
            let path = dir.path().join(name);
            std::fs::write(&path, content).unwrap();
            assert_eq!(
                read_log_lines(&path, usize::MAX, None).unwrap(),
                content.lines().map(str::to_string).collect::<Vec<_>>(),
                "{name}"
            );
        }
    }

    #[test]
    fn profiler_field_falls_back_to_the_legacy_profiler() {
        // Regressão: `debug memory`/`hitches` não tinham fallback e falhavam
        // no monólito ("method not found").
        let port = crate::testutil::mock_bridge(vec![
            ("vael-tools/memory", Err("method not found")),
            (
                "viber.profiler",
                Ok(json!({ "memory": { "rss_kb": 42 }, "hitches": {"count": 1} })),
            ),
        ]);
        let client = BridgeClient::localhost(port);
        let memory = profiler_field(&client, METHOD_MEMORY, "memory").unwrap();
        assert_eq!(memory["rss_kb"], 42);
    }

    #[test]
    fn profiler_field_is_an_error_when_nobody_publishes_it() {
        let port = crate::testutil::mock_bridge(vec![
            ("vael-tools/hitches", Err("method not found")),
            ("viber.profiler", Ok(json!({ "memory": {} }))),
        ]);
        let client = BridgeClient::localhost(port);
        let error = profiler_field(&client, METHOD_HITCHES, "hitches").unwrap_err();
        assert!(format!("{error:#}").contains("hitches"), "{error:#}");
    }
}

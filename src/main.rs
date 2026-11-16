//! Arranque do Vael: `vael [world.bsn] [--entry <nome>]`.
//!
//! A App é composta aqui e em [`vael::app`] (sem o builder da engine): o
//! runtime de gameplay (`viber`) fornece os plugins do mundo e os serviços,
//! o crate do jogo traz o controller/câmara (`src/player.rs`) e as regras
//! (autoridade Rust). O `--entry` continua a ser a entrada de cena do
//! `vael-tools run`.

use std::path::{Path, PathBuf};

use anyhow::Result;
use bevy::prelude::*;
use clap::Parser as CliParser;

use vael::{VaelPlugin, VaelProjectRoot, ui::VaelUiPlugin};

/// Vael: o jogo sobre a engine Viber (`world.bsn`).
#[derive(CliParser)]
#[command(
    name = "vael",
    about = "Vael",
    disable_version_flag = true,
    // `uso: vael [world.bsn] [--scene <nome>] [--entry <nome>] [--bridge <porta>]`
    override_usage = "vael [world.bsn] [OPTIONS]"
)]
struct Args {
    /// Ficheiro do mundo (default: `world.bsn`, com fallback a `world.xml`).
    world: Option<PathBuf>,
    /// Entrada de cena do arranque (ex.: `dev`).
    #[arg(long)]
    entry: Option<String>,
    /// Cena de arranque (ex.: `main`); sem ela, o `start-scene` decide.
    #[arg(long)]
    scene: Option<String>,
    /// Porta da bridge de debug (ex.: 15706).
    #[arg(long)]
    bridge: Option<u16>,
    /// Resolução da janela `WxH`, ex.: `1280x720` (ganha a `VAEL_RESOLUTION`).
    #[arg(long, value_parser = parse_resolution_arg)]
    resolution: Option<(u32, u32)>,
    /// Reescreve `world/settings/*.bsn` com os valores de antes da cena e sai.
    #[arg(long)]
    export_settings: bool,
    /// Com `--export-settings`: reescreve mesmo sem mudanças.
    #[arg(long, requires = "export_settings")]
    force: bool,
    /// Cenário de QA (`qa/scenarios/<nome>.json`, `@active` ou caminho):
    /// estado do herói/quests/itens e posição; ganha a `VAEL_SCENARIO`.
    #[arg(long)]
    scenario: Option<String>,
    /// Simula o cenário (ou `all`) sem janela e sai (exit 1 se falhar).
    #[arg(long, value_name = "CENÁRIO")]
    qa_sim: Option<String>,
    /// Com `--qa-sim`: relatório em JSON.
    #[arg(long, requires = "qa_sim")]
    json: bool,
    /// Passada pelo `jd` para extrair o schema refletido; trata dela o
    /// `jackdaw_runtime::JackdawPlugin` no `VaelPlugin` — aqui só é aceite.
    #[arg(long, hide = true)]
    jackdaw_extract_schema: bool,
}

fn main() -> Result<()> {
    // O IBL da engine pinta um cubemap a partir do `<Sky>`/`AtmosphereState`
    // antigos. Com o ambiente novo (`bevy_elements`) a luz do céu vem da
    // atmosfera do crate (`AtmosphereEnvironmentMapLight`); dois IBLs
    // somavam luz do céu errado. `VIBER_NO_IBL` é lido no `SkyIblPlugin`,
    // antes de qualquer thread dos `DefaultPlugins` nascer — daí o `set_var`.
    if vael::elements_enabled() {
        // SAFETY: single-threaded, antes da composição da App.
        unsafe { std::env::set_var("VIBER_NO_IBL", "1") };
    }
    let args = Args::parse();

    // O `.bsn` é a fonte de verdade; o `.xml` fica como fallback nos
    // checkouts ainda não convertidos. Resolvido ANTES dos comandos que não
    // abrem a App: `--export-settings`/`--qa-sim` leem/escrevem ficheiros do
    // projeto e têm de usar a raiz DELE (a pasta do mundo), não a cwd de quem
    // os invoca.
    let world = args.world.clone().unwrap_or_else(|| {
        if Path::new("world.bsn").is_file() {
            PathBuf::from("world.bsn")
        } else {
            PathBuf::from("world.xml")
        }
    });
    let project_root = project_root(&world)?;

    // Escreve `world/settings/*.bsn` com os valores de antes da cena
    // (ambiente, PostFX e espécies de flora) e sai.
    if args.export_settings {
        for path in vael::settings::export_settings(&project_root, args.force)? {
            println!("{}", path.display());
        }
        return Ok(());
    }

    // Simulação de cenários: só regras e conteúdo, sem engine nem GPU.
    if let Some(name) = args.qa_sim.as_deref() {
        std::process::exit(vael::qa::run_cli(&project_root, name, args.json));
    }

    let mut entry = args.entry;
    let mut scene = args.scene;
    let bridge = args.bridge;
    // Cenário de QA: escolhe a cena/entrada/hora/clima quando a linha de
    // comandos não as dá (o Play (QA) do Jackdaw só passa a env).
    let scenario_name = args
        .scenario
        .clone()
        .or_else(|| std::env::var("VAEL_SCENARIO").ok())
        .filter(|name| !name.trim().is_empty());
    let scenario = match scenario_name.as_deref() {
        Some(name) => {
            let base = std::path::absolute(&world)?
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default();
            let loaded = vael::qa::load_scenario(&base, name)
                .map_err(|err| anyhow::anyhow!("cenário {name}: {err}"))?;
            let s = &loaded.scenario;
            if scene.is_none() {
                scene = s.scene.clone();
            }
            if entry.is_none() {
                entry = s.entry.clone().or_else(|| {
                    // Em `main`, `dev` salta título e prólogo.
                    (scene.as_deref() == Some("main")).then(|| "dev".to_string())
                });
            }
            // SAFETY: single-threaded, antes de `build_app`.
            if let Some(hour) = s.hour
                && std::env::var_os("VAEL_START_HOUR").is_none()
            {
                unsafe { std::env::set_var("VAEL_START_HOUR", hour.to_string()) };
            }
            if let Some(weather) = &s.weather
                && std::env::var_os("VAEL_START_WEATHER").is_none()
            {
                unsafe { std::env::set_var("VAEL_START_WEATHER", weather) };
            }
            Some(loaded)
        }
        None => None,
    };
    // Resolução da janela: `--resolution WxH` > `VAEL_RESOLUTION` > default
    // da engine (1280x720). Num portátil com a GPU no teto, é o corte de
    // fill-rate mais direto: a 1600x900 o frame é 100% GPU-bound. A env
    // inválida é ignorada em silêncio, como sempre foi.
    let resolution = args.resolution.or_else(|| {
        std::env::var("VAEL_RESOLUTION")
            .ok()
            .and_then(|raw| parse_resolution(&raw))
    });

    // Absoluto: o cwd não pode decidir onde vivem o config, os assets e o save.
    let requested = std::path::absolute(&world)
        .map_err(|err| anyhow::anyhow!("mundo inválido: {} ({err})", world.display()))?;
    // `--scene` escolhe outra cena do projeto; sem ele, a cena principal cede
    // ao `start-scene` (o acampamento atrás do título), como o `vael-tools run`.
    let world = viber::scene_loader::resolve_run_path(&requested, scene.as_deref())?;
    let root = viber::app::world_base_dir(&world)
        .or_else(|| world.parent().map(Path::to_path_buf))
        .ok_or_else(|| anyhow::anyhow!("não foi possível resolver a raiz do projeto"))?;

    let mut app = vael::app::build_app(
        &world,
        vael::app::VaelAppOptions {
            start_entry: entry.clone(),
            bridge_port: bridge,
            // A identidade da bridge é o mundo que o utilizador pediu, não a
            // cena de arranque resolvida (senão `--scene main` respondia menu).
            identity: requested.clone(),
            // Play do Jackdaw em modo embedded: o runtime desliga a janela
            // quando o editor pede `JACKDAW_PIE_WINDOWLESS`. Fora do PIE o
            // wrapper devolve o grupo intacto.
            default_plugins: Some(jackdaw_plugins),
        },
    )?;
    // A janela primária nasce no `WindowPlugin` (já no `build_app`): mudar a
    // resolução aqui chega antes do `run` criar a janela do SO.
    if let Some((width, height)) = resolution {
        let mut windows = app.world_mut().query::<&mut Window>();
        if let Some(mut window) = windows.iter_mut(app.world_mut()).next() {
            window.resolution = bevy::window::WindowResolution::new(width, height);
        }
    }
    app.insert_resource(VaelProjectRoot(root.clone()));
    if let Some(loaded) = &scenario {
        // Saves de QA à parte: um autosave do cenário não pode pisar o slot
        // do jogador. `VAEL_QA_SAVE_DIR` muda a pasta.
        let dir = std::env::var_os("VAEL_QA_SAVE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("vael-qa-saves"));
        let _ = std::fs::create_dir_all(&dir);
        app.insert_resource(bevy_ext_save::SaveDir(Some(dir)));
        app.insert_resource(vael::qa::QaRequest {
            name: scenario_name.clone().unwrap_or_else(|| loaded.name.clone()),
        });
    }
    app.add_plugins(VaelPlugin);
    // UI do jogo em XAML (bevy_pf): precisa do `DefaultPlugins` e dos
    // resources do Vael já registados.
    app.add_plugins(VaelUiPlugin);
    // Runtime de ferramentas: acrescenta `vael-tools/*` ao `bevy_remote` que
    // o `build_app` já abriu (ou abre o seu, quando não há --bridge). O
    // bridge legacy `viber.*` continua no mesmo servidor para compatibilidade.
    if bridge.is_some() || tools_enabled() {
        let port = bridge.unwrap_or(default_tools_port());
        app.add_plugins(bevy_ext_bridge_runtime::BridgeToolsPlugin { port });
        // Prontidão do carregamento para o `vael-tools/ping` (sem isto, o
        // pong do runtime novo não tinha `ready` e o `wait-ready` do CLI
        // voltava cedo, com o véu em pé).
        app.add_plugins(vael::bridge_ready::BridgeReadyPlugin);
        // O `input.key` sintético é aplicado no `PreUpdate`; tem de correr
        // ANTES do `gather_actions` do jogo para o `just_pressed` (salto,
        // ataque, interagir) entrar no `ActionState` no mesmo frame.
        app.configure_sets(
            PreUpdate,
            bevy_ext_bridge_runtime::BridgeToolsInput
                .before(viber::input_map::gather_actions)
                .before(jackdaw_input::ActionSystems),
        );
        // Debugger remoto do Jackdaw no mesmo servidor BRP: o `RemotePlugin`
        // já existe, por isso o plugin só injeta os métodos `jackdaw/*` no
        // `finish` (snapshot da cena, `entity_bsn`/`apply_bsn`, pause/step,
        // diagnósticos, archetypes/schedules). O editor liga-se a 15702 por
        // defeito — a porta do runtime quando o Play não passa `--bridge`.
        app.add_plugins(
            jackdaw_remote::JackdawRemotePlugin::default()
                .with_port(port)
                .with_app_name("Vael"),
        );
        // Tools MCP nativas (`bevy_brp_mcp` 0.23 → stdio no ZCode): o plugin
        // injeta `brp_extras/*` (screenshot, teclas/rato/gamepad, diagnósticos,
        // shutdown) no mesmo servidor BRP — o `RemoteHttpPlugin` já existe, por
        // isso a porta dele é ignorada. `VAEL_BRP_EXTRAS=0` desliga (A/B).
        if brp_extras_enabled() {
            app.add_plugins(bevy_brp_extras::BrpExtrasPlugin::default());
        }
    }

    info!(
        world = %world.display(),
        requested = %requested.display(),
        root = %root.display(),
        scene = ?scene,
        entry = ?entry,
        scenario = ?scenario.as_ref().map(|loaded| loaded.name.as_str()),
        bridge = ?bridge,
        gameplay = false,
        scripts = "compat",
        "vael: arranque"
    );

    let _ = app.run();
    Ok(())
}

/// Grupo `DefaultPlugins` do jogo, passado ao `build_app` com o wrapper do
/// PIE: o `maybe_windowless` só altera o grupo quando o editor pede uma
/// sessão sem janela (`JACKDAW_PIE_WINDOWLESS`); sem PIE é o grupo da engine.
fn jackdaw_plugins(plugins: bevy::app::PluginGroupBuilder) -> bevy::app::PluginGroupBuilder {
    jackdaw_runtime::maybe_windowless(plugins)
}

/// Porta do runtime de ferramentas quando o jogo não pede `--bridge`.
fn default_tools_port() -> u16 {
    // A mesma porta histórica da bridge; o namespace separa os métodos.
    15702
}

/// `VAEL_TOOLS=1` ou PIE ligam o runtime sem `--bridge`.
fn tools_enabled() -> bool {
    let flag = std::env::var("VAEL_TOOLS")
        .map(|raw| {
            !matches!(
                raw.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "off" | "no"
            )
        })
        .unwrap_or(false);
    flag || std::env::var_os("JACKDAW_PIE").is_some()
}

/// `VAEL_BRP_EXTRAS=0` desliga o `BrpExtrasPlugin` (default ligado quando as
/// ferramentas estão): é A/B com o CLI, não um segundo interruptor para lembrar.
fn brp_extras_enabled() -> bool {
    !matches!(
        std::env::var("VAEL_BRP_EXTRAS")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "0" | "false" | "off" | "no"
    )
}

/// Raiz do projeto de um mundo: a pasta com `config.yaml`, `data/`, `quests/`
/// e `world/` — a do próprio mundo quando ele é a raiz, ou a que o `Project`
/// descobre para uma cena em `scenes/`.
///
/// Os comandos headless (`--export-settings`, `--qa-sim`) resolvem por aqui e
/// não pela cwd: `vael <outro-sitio>/world.bsn --qa-sim all` tem de ler os
/// cenários/conteúdo DELE, mesmo quando o processo arranca noutra pasta.
fn project_root(world: &Path) -> Result<PathBuf> {
    let requested = std::path::absolute(world)
        .map_err(|err| anyhow::anyhow!("mundo inválido: {} ({err})", world.display()))?;
    viber::app::world_base_dir(&requested)
        .or_else(|| requested.parent().map(Path::to_path_buf))
        .ok_or_else(|| anyhow::anyhow!("não foi possível resolver a raiz do projeto"))
}

/// `WxH` (ex.: `1280x720`) → tamanho físico da janela. Teto defensivo: um
/// valor absurdo na env não pode abrir uma janela de 100000 px.
fn parse_resolution(raw: &str) -> Option<(u32, u32)> {
    let (width, height) = raw.trim().split_once(['x', 'X'])?;
    let width: u32 = width.trim().parse().ok()?;
    let height: u32 = height.trim().parse().ok()?;
    let ok = (320..=8192).contains(&width) && (200..=8192).contains(&height);
    ok.then_some((width, height))
}

/// O mesmo parse, na forma que o clap espera para `--resolution`.
fn parse_resolution_arg(raw: &str) -> Result<(u32, u32), String> {
    parse_resolution(raw).ok_or_else(|| format!("resolução inválida: {raw} (WxH)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// O contrato com o `vael-tools run`: flags com valor separado por
    /// espaço e o mundo como posicional.
    #[test]
    fn flags_da_bridge_e_da_cena_parseiam() {
        let args = Args::try_parse_from([
            "vael",
            "outro.bsn",
            "--scene",
            "main",
            "--entry",
            "dev",
            "--bridge",
            "15706",
            "--resolution",
            "1280x720",
        ])
        .expect("args válidas");
        assert_eq!(args.world, Some(PathBuf::from("outro.bsn")));
        assert_eq!(args.scene.as_deref(), Some("main"));
        assert_eq!(args.entry.as_deref(), Some("dev"));
        assert_eq!(args.bridge, Some(15706));
        assert_eq!(args.resolution, Some((1280, 720)));
    }

    /// A flag silenciosa do `jd` é aceite; o `--force` só existe com
    /// `--export-settings` (antes era bandeira solta ignorada fora do ramo).
    #[test]
    fn flag_do_jd_e_aceite_e_forca_exige_export() {
        assert!(Args::try_parse_from(["vael", "--jackdaw-extract-schema"]).is_ok());
        assert!(Args::try_parse_from(["vael", "--force"]).is_err());
        assert!(Args::try_parse_from(["vael", "--export-settings", "--force"]).is_ok());
    }

    /// Teto defensivo do `WxH` (a env não pode abrir janelas absurdas).
    #[test]
    fn resolucao_fora_dos_limites_e_rejeitada() {
        assert!(Args::try_parse_from(["vael", "--resolution", "100x100"]).is_err());
        assert!(Args::try_parse_from(["vael", "--resolution", "abc"]).is_err());
        assert!(Args::try_parse_from(["vael", "--resolution", "1280x720"]).is_ok());
    }

    /// A raiz dos comandos headless vem do mundo, não da cwd — e uma cena em
    /// `scenes/` resolve para a raiz do projeto.
    #[test]
    fn a_raiz_do_projeto_vem_do_mundo() {
        let base = Path::new(env!("CARGO_MANIFEST_DIR"));
        let expected = base.canonicalize().expect("raiz canónica");
        for world in ["world.bsn", "scenes/menu.bsn"] {
            let root = project_root(&base.join(world)).expect("raiz");
            assert_eq!(
                root.canonicalize().expect("canónica"),
                expected,
                "{world} tem de resolver para a raiz do projeto"
            );
        }
    }
}

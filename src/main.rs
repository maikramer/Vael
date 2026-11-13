//! Arranque do Vael: `vael [world.bsn] [--entry <nome>]`.
//!
//! A engine monta o mundo; o crate do jogo entra por cima com
//! [`viber::app::AppOptions::gameplay`] desligado, a IA em
//! [`viber::app::AiMode::Service`] e os scripts Luau em `Compat`. O `--entry`
//! continua a ser a entrada de cena do `viber run`.

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
    /// Passada pelo `jd` para extrair o schema refletido; trata dela o
    /// `jackdaw_runtime::JackdawPlugin` no `VaelPlugin` — aqui só é aceite.
    #[arg(long, hide = true)]
    jackdaw_extract_schema: bool,
}

fn main() -> Result<()> {
    // O IBL da engine pinta um cubemap a partir do `<Sky>`/`AtmosphereState`
    // antigos. Com o ambiente novo (`bevy_elements`) a luz do céu vem da
    // atmosfera do crate (`AtmosphereEnvironmentMapLight`); dois IBLs
    // somavam luz do céu errado. `VIBER_NO_IBL` é lido no `build_app`, antes
    // de qualquer thread dos `DefaultPlugins` nascer — daí o `set_var`.
    if vael::elements_enabled() {
        // SAFETY: single-threaded, antes de `build_app`.
        unsafe { std::env::set_var("VIBER_NO_IBL", "1") };
    }
    let args = Args::parse();

    // Escreve `world/settings/*.bsn` com os valores de antes da cena
    // (ambiente, PostFX e espécies de flora) e sai.
    if args.export_settings {
        let root = std::env::current_dir()?;
        for path in vael::settings::export_settings(&root, args.force)? {
            println!("{}", path.display());
        }
        return Ok(());
    }

    // O `.bsn` é a fonte de verdade; o `.xml` fica como fallback nos
    // checkouts ainda não convertidos.
    let world = args.world.unwrap_or_else(|| {
        if Path::new("world.bsn").is_file() {
            PathBuf::from("world.bsn")
        } else {
            PathBuf::from("world.xml")
        }
    });
    let entry = args.entry;
    let scene = args.scene;
    let bridge = args.bridge;
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
    // ao `start-scene` (o acampamento atrás do título), como o `viber run`.
    let world = viber::scene_loader::resolve_run_path(&requested, scene.as_deref())?;
    let root = viber::app::world_base_dir(&world)
        .or_else(|| world.parent().map(Path::to_path_buf))
        .ok_or_else(|| anyhow::anyhow!("não foi possível resolver a raiz do projeto"))?;

    let mut app = viber::app::build_app(
        &world,
        viber::app::AppOptions {
            gameplay: false,
            // O texto de dano/cura/coleta/combo é do Vael desde a migração
            // para `bevy_rich_text3d`; o pool 2D projectado da engine
            // desenhava-o duas vezes e com outra fonte.
            damage_numbers_2d: false,
            // A FSM simples da engine fica desligada, mas o serviço de IA
            // (sentidos + `AiLocomotion`) mantém-se para os scripts Luau
            // moverem as criaturas via `viber.move_towards`.
            ai: viber::app::AiMode::Service,
            // O Vael já traz o controller/câmara em `src/player.rs`; a engine
            // continua a spawnar o corpo e o collider, mas não o move.
            player_camera: false,
            // Slots e envelope são do Vael (`moonshine-save`); o SavePlugin
            // da engine continua fora do caminho. `SaveRequest` fica vivo
            // para o Lua pedir autosaves.
            external_save: true,
            scripts: viber::app::ScriptsMode::Compat,
            // O jogo não usa `viber.ecs`: o espelho refletido por frame é
            // trabalho morto e exclusivo. QA/bridge pode reativá-lo com
            // `VIBER_ECS_MIRROR=1`.
            ecs_mirror: ecs_mirror_enabled(),
            start_entry: entry.clone(),
            bridge_port: bridge,
            // A identidade da bridge é o mundo que o utilizador pediu, não a
            // cena de arranque resolvida (senão `--scene main` respondia menu).
            world_path: Some(requested.clone()),
            // Play do Jackdaw em modo embedded: o runtime desliga a janela
            // quando o editor pede `JACKDAW_PIE_WINDOWLESS`. Fora do PIE o
            // wrapper devolve o grupo da engine intacto.
            default_plugins: Some(jackdaw_plugins),
            ..Default::default()
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
    app.add_plugins(VaelPlugin);
    // UI do jogo em XAML (bevy_pf): precisa do `DefaultPlugins` e dos
    // resources do Vael já registados.
    app.add_plugins(VaelUiPlugin);
    // Runtime de ferramentas: acrescenta `viber-tools/*` ao `bevy_remote` que
    // o `build_app` já abriu (ou abre o seu, quando não há --bridge). O
    // bridge legacy `viber.*` continua no mesmo servidor para compatibilidade.
    if bridge.is_some() || tools_enabled() {
        let port = bridge.unwrap_or(default_tools_port());
        app.add_plugins(viber_tools_runtime::ViberToolsPlugin { port });
        // O `input.key` sintético é aplicado no `PreUpdate`; tem de correr
        // ANTES do `gather_actions` do jogo para o `just_pressed` (salto,
        // ataque, interagir) entrar no `ActionState` no mesmo frame.
        app.configure_sets(
            PreUpdate,
            viber_tools_runtime::ViberToolsInput.before(viber::input_map::gather_actions),
        );
    }

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

/// `viber.ecs` por frame é opt-in no Vael (default off).
fn ecs_mirror_enabled() -> bool {
    matches!(
        std::env::var("VIBER_ECS_MIRROR")
            .ok()
            .map(|raw| raw.trim().to_ascii_lowercase())
            .as_deref(),
        Some("1" | "true" | "on" | "yes")
    )
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

    /// O contrato com o `viber run`/wrapper: flags com valor separado por
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
}

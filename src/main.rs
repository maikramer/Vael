//! Arranque do Vael: `vael [world.bsn] [--entry <nome>]`.
//!
//! A App é composta aqui e em [`vael::app`] (sem o builder da engine): o
//! runtime de gameplay (`viber`) fornece os plugins do mundo e os serviços,
//! o crate do jogo traz o controller/câmara (`src/player.rs`) e as regras
//! (autoridade Rust). O `--entry` continua a ser a entrada de cena do
//! `vael-tools run`.

use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Parser as CliParser;

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
    // Traço do arranque: a primeira marca é o processo, não o primeiro frame.
    bevy_ext_core::boot::start();
    // O IBL da engine pinta um cubemap a partir do `<Sky>`/`AtmosphereState`
    // antigos. Com o ambiente novo (`bevy_elements`) a luz do céu vem da
    // atmosfera do crate (`AtmosphereEnvironmentMapLight`); dois IBLs
    // somavam luz do céu errado. `VIBER_NO_IBL` é lido no `SkyIblPlugin`,
    // antes de qualquer thread dos `DefaultPlugins` nascer — daí o `set_var`.
    if vael::elements_enabled() {
        // SAFETY: single-threaded, antes da composição da App.
        unsafe { std::env::set_var("VIBER_NO_IBL", "1") };
    }
    // Culling das malhas estáticas na GPU (`NoCpuCulling` + indirect draws).
    // O piloto do Bevy 0.19 era opt-in por causa de backends sem GPU culling
    // (DX12/WebGL); o Vael corre em Vulkan com GPU preprocessing completo
    // ("GPU preprocessing is fully supported", log de arranque) e mediu-se
    // ~1,4 ms/frame de `check_visibility` a menos. Quem precisar do caminho
    // antigo põe `VIBER_GPU_CULLING=0`.
    if std::env::var_os("VIBER_GPU_CULLING").is_none() {
        // SAFETY: idem — nenhuma outra thread lê o ambiente ainda.
        unsafe { std::env::set_var("VIBER_GPU_CULLING", "1") };
    }
    // On Linux, CPAL double-buffers the ALSA period. Give the PipeWire ALSA
    // client at least 2048 stereo f32 frames per period (43 ms at 48 kHz)
    // so a short scheduling delay during rendering does not empty the ring.
    #[cfg(target_os = "linux")]
    if std::env::var_os("PIPEWIRE_ALSA").is_none() {
        // SAFETY: still single-threaded, before initializing Bevy or audio.
        unsafe {
            std::env::set_var(
                "PIPEWIRE_ALSA",
                "{ alsa.period-bytes = { min = 16384 max = 2097152 } }",
            )
        };
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

    // Cena, entrada e cenário QA são resolvidos no `build_game_app` (a mesma
    // via que os testes de playtest usam).
    // Resolução da janela: `--resolution WxH` > `VAEL_RESOLUTION` > default
    // da engine (1280x720). Num portátil com a GPU no teto, é o corte de
    // fill-rate mais direto: a 1600x900 o frame é 100% GPU-bound. A env
    // inválida é ignorada em silêncio, como sempre foi.
    let resolution = args.resolution.or_else(|| {
        std::env::var("VAEL_RESOLUTION")
            .ok()
            .and_then(|raw| parse_resolution(&raw))
    });
    let mut app = vael::app::build_game_app(vael::app::VaelGameOptions {
        world: world.clone(),
        scene: args.scene,
        entry: args.entry,
        scenario: args.scenario,
        bridge_port: args.bridge,
        resolution,
        qa_save_dir: None,
    })?;
    // O takeover do playtest é last-set-wins: tem de entrar depois de todos os
    // grupos que instalam runner (o `build_app` já montou o `DefaultPlugins`).
    #[cfg(feature = "playtest")]
    app.add_plugins(bevy_playtest::PlaytestPlugin);
    let _ = app.run();
    Ok(())
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
    vael_engine::app::world_base_dir(&requested)
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

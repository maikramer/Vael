//! Grava a janela do jogo através do bridge local, com cadência de reprodução
//! baseada no relógio monotónico. Só o cliente precisa de FFmpeg.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use bevy_ext_bridge_protocol::BridgeClient;
use clap::Args;
use serde::Serialize;
use serde_json::json;
use tempfile::{NamedTempFile, TempDir};

#[derive(Args)]
pub struct VideoArgs {
    #[arg(short, long, default_value = "movimento.mp4")]
    pub output: PathBuf,
    /// Duração da captura em segundos reais.
    #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u32).range(1..=120))]
    seconds: u32,
    /// Máximo de capturas por segundo.
    #[arg(long, default_value_t = 15, value_parser = clap::value_parser!(u32).range(1..=60))]
    fps: u32,
    /// Fator de câmera lenta na reprodução (2 = metade da velocidade).
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..=8))]
    slow_motion: u32,
    /// Recorte fixo em pixels da janela, a partir do canto superior esquerdo.
    #[arg(long, value_name = "X,Y,LARGURA,ALTURA", value_parser = parse_region)]
    region: Option<[u32; 4]>,
}

fn parse_region(value: &str) -> Result<[u32; 4]> {
    let values: Vec<u32> = value
        .split(',')
        .map(|v| v.trim().parse())
        .collect::<Result<_, _>>()?;
    let rect: [u32; 4] = values
        .try_into()
        .map_err(|_| anyhow::anyhow!("use X,Y,LARGURA,ALTURA"))?;
    ensure!(
        rect[2] > 0 && rect[3] > 0,
        "largura e altura precisam ser positivas"
    );
    Ok(rect)
}

#[derive(Serialize)]
pub struct VideoReport<'a> {
    path: &'a Path,
    pub captured_frames: u32,
    pub video_frames: u32,
    pub capture_hz: f64,
    repeated_frames: u32,
    capture_seconds: u32,
    playback_seconds: u32,
}

struct FrameSource<'a> {
    client: &'a BridgeClient,
    dir: TempDir,
    requests: u32,
    legacy_capture: bool,
    region: Option<[u32; 4]>,
}

impl FrameSource<'_> {
    fn next(&mut self) -> Result<Vec<u8>> {
        if !self.legacy_capture {
            // O BRP pode deduplicar pedidos iguais em frames adjacentes.
            self.requests += 1;
            let path = std::path::absolute(self.dir.path())?.join(format!("{}.png", self.requests));
            let mut request = json!({"path": path});
            if let Some([x, y, width, height]) = self.region {
                request["rect"] = json!({ "x": x, "y": y, "width": width, "height": height });
            }
            match self.client.call("brp_extras/screenshot", request) {
                Ok(reply) => {
                    ensure!(
                        reply["success"] == true,
                        "captura do bridge falhou: {reply}"
                    );
                    let png = std::fs::read(&path).context("a ler o PNG do bridge local")?;
                    std::fs::remove_file(path)?;
                    return Ok(png);
                }
                Err(e) if self.requests == 1 && e.to_string().contains("bridge error -32601:") => {
                    self.legacy_capture = true
                }
                Err(e) => return Err(e),
            }
        }
        Ok(crate::debug::capture_screenshot(self.client, 3000)?.0)
    }
}

fn png_size(png: &[u8]) -> Result<(u32, u32)> {
    ensure!(
        png.len() >= 24 && png.starts_with(b"\x89PNG\r\n\x1a\n") && &png[12..16] == b"IHDR",
        "o bridge devolveu um PNG inválido"
    );
    Ok((
        u32::from_be_bytes(png[16..20].try_into()?),
        u32::from_be_bytes(png[20..24].try_into()?),
    ))
}

fn encoder_filter(region: Option<[u32; 4]>, (width, height): (u32, u32)) -> Result<String> {
    let mut filter = String::new();
    if let Some([x, y, w, h]) = region {
        ensure!(
            w <= width && x <= width - w && h <= height && y <= height - h,
            "recorte fora da janela de {width}×{height} pixels"
        );
        filter = format!("crop=w={w}:h={h}:x={x}:y={y}:exact=1,");
    }
    filter.push_str("pad=ceil(iw/2)*2:ceil(ih/2)*2");
    Ok(filter)
}

// Cada imagem permanece visível até à amostra seguinte. Atrasos do
// renderer repetem o último quadro em vez de acelerar o movimento no MP4.
fn frame_slot(elapsed: Duration, fps: u32, end: u32) -> u32 {
    ((elapsed.as_secs_f64() * f64::from(fps)).round() as u32).min(end)
}

fn write_frames(
    source: &mut FrameSource<'_>,
    mut png: Vec<u8>,
    sink: &mut impl Write,
    args: &VideoArgs,
) -> Result<u32> {
    let size = png_size(&png)?;
    let total = args.seconds * args.fps;
    let period = Duration::from_secs_f64(1.0 / f64::from(args.fps));
    let start = Instant::now();
    let mut next_request = start + period;
    let mut slot = 0;
    let mut captured = 1;
    while slot + 1 < total {
        std::thread::sleep(next_request.saturating_duration_since(Instant::now()));
        if frame_slot(start.elapsed(), args.fps, total) >= total {
            break;
        }
        next_request = Instant::now() + period;
        let next = source.next()?;
        let next_slot = frame_slot(start.elapsed(), args.fps, total);
        ensure!(
            png_size(&next)? == size,
            "a resolução da janela mudou durante a captura"
        );
        if slot == next_slot {
            continue;
        }
        for _ in slot..next_slot {
            sink.write_all(&png)?;
        }
        slot = next_slot;
        if slot < total {
            png = next;
            captured += 1;
        }
    }
    for _ in slot..total {
        sink.write_all(&png)?;
    }
    Ok(captured)
}

pub fn record<'a>(client: &BridgeClient, args: &'a VideoArgs) -> Result<VideoReport<'a>> {
    let parent = args
        .output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let output = NamedTempFile::new_in(parent)
        .with_context(|| format!("a criar MP4 em {}", parent.display()))?;
    let mut source = FrameSource {
        client,
        dir: tempfile::tempdir()?,
        requests: 0,
        legacy_capture: false,
        region: args.region,
    };
    let first = source.next()?;
    // O método rápido já publicou o recorte. No bridge antigo, o encoder
    // recorta o PNG inteiro para oferecer o mesmo resultado.
    let crop_in_encoder = if source.legacy_capture {
        args.region
    } else {
        None
    };
    let filter = encoder_filter(crop_in_encoder, png_size(&first)?)?;
    let rate = format!("{}/{}", args.fps, args.slow_motion);
    let mut encoder = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-y",
            "-f",
            "image2pipe",
            "-probesize",
            "32",
            "-framerate",
            &rate,
            "-c:v",
            "png",
            "-i",
            "pipe:0",
            "-an",
            "-vf",
            &filter,
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            "18",
            "-pix_fmt",
            "yuv420p",
            "-movflags",
            "+faststart",
            "-f",
            "mp4",
        ])
        .arg(output.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .context("a iniciar ffmpeg (instale FFmpeg com libx264 e coloque-o no PATH)")?;
    let mut stdin = encoder.stdin.take().expect("stdin configurado como pipe");
    let result = write_frames(&mut source, first, &mut stdin, args);
    drop(stdin);
    if result.is_err() {
        let _ = encoder.kill();
    }
    let status = encoder.wait()?;
    let captured = result.context("a gravar os quadros do bridge no FFmpeg")?;
    if !status.success() {
        bail!("FFmpeg falhou ({status})");
    }
    output
        .persist(&args.output)
        .with_context(|| format!("a guardar {}", args.output.display()))?;
    let total = args.fps * args.seconds;
    Ok(VideoReport {
        path: &args.output,
        captured_frames: captured,
        video_frames: total,
        capture_hz: f64::from(captured) / f64::from(args.seconds),
        repeated_frames: total - captured,
        capture_seconds: args.seconds,
        playback_seconds: args.seconds * args.slow_motion,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tempo_de_captura_mapeia_atrasos_sem_esticar_o_video() {
        assert_eq!(frame_slot(Duration::from_millis(340), 15, 30), 5);
        assert_eq!(frame_slot(Duration::from_secs(4), 15, 30), 30);
    }

    #[test]
    fn bridge_sem_extras_seleciona_o_fallback_uma_vez() {
        let shot = || Ok(json!({"status": "captured", "png_base64": "YWJj"}));
        let port = crate::testutil::mock_bridge(vec![
            ("brp_extras/screenshot", Err("Method not found")),
            ("vael-tools/screenshot", Ok(json!({"id": 1}))),
            ("vael-tools/screenshot_status", shot()),
            ("vael-tools/screenshot", Ok(json!({"id": 2}))),
            ("vael-tools/screenshot_status", shot()),
        ]);
        let client = BridgeClient::localhost(port);
        let mut source = FrameSource {
            client: &client,
            dir: tempfile::tempdir().unwrap(),
            requests: 0,
            legacy_capture: false,
            region: None,
        };
        assert_eq!(source.next().unwrap(), b"abc");
        assert_eq!(source.next().unwrap(), b"abc");
    }

    #[test]
    fn recorte_aceita_borda_impar_e_rejeita_excesso_sem_overflow() {
        let size = (800, 450);
        assert!(encoder_filter(Some(parse_region("21,20,779,429").unwrap()), size).is_ok());
        assert!(encoder_filter(Some([u32::MAX, 0, 2, 2]), size).is_err());
        assert!(parse_region("1,1,0,12").is_err());
        assert!(parse_region("1,2,3").is_err());
    }
}

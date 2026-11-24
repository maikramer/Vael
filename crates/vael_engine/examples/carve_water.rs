//! Grava lagos e rios NOVOS no heightmap de um mundo estático.
//!
//! Num mundo `static="true"` o runtime carva as features numa CÓPIA da grelha
//! (só para resolver a água) — o relevo final vem do `.ahgt`. Um lago ou rio
//! acrescentado ao `.bsn` depois do bake regista a lâmina mas não abre o
//! leito. Este exemplo aplica o carve dos corpos novos ao `.ahgt` no sítio.
//!
//! Entrada: um JSON com `prior` (corpos que já estão no `.ahgt`: só servem
//! para a confluência e para a máscara das margens, carvados numa cópia) e
//! `new` (corpos a gravar, por ordem de documento — lagos antes de rios,
//! como em `apply_features`).
//!
//! ```bash
//! cargo run --release -p vael_engine --example carve_water -- \
//!     world.data/terrain.ahgt water.json [--dry-run]
//! ```
//!
//! Formato de cada corpo: `{"kind":"lake", "at":[x,z], "radius", "depth",
//! "water_offset", "water_level", "seed", "axis", "stretch", "lobes"}` ou
//! `{"kind":"river", "path":[[x,z],…], "width", "depth", "water_offset",
//! "bank_width", "bank_height", "pool_spacing", "water_levels":[…]}`.
//! Os níveis têm de ser explícitos (o contrato idempotente do bake): o
//! runtime volta a carvar estes corpos sobre a grelha gravada e tem de dar
//! a mesma lâmina.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use bevy::math::Vec2;
use serde_json::Value;
use bevy_ext_viber_engine::terrain::brush::BrushGrid;
use bevy_ext_viber_engine::terrain::heightmap::HeightMapU16;
use bevy_ext_viber_engine::terrain::water::{
    LakeSpec, RiverSpec, WaterBody, carve_lake, carve_river,
};

fn f(v: &Value, key: &str) -> Option<f32> {
    v.get(key).and_then(Value::as_f64).map(|x| x as f32)
}

fn vec2(v: &Value) -> Result<Vec2> {
    let a = v.as_array().context("ponto [x, z]")?;
    Ok(Vec2::new(
        a[0].as_f64().context("x")? as f32,
        a[1].as_f64().context("z")? as f32,
    ))
}

fn lake_spec(v: &Value) -> Result<LakeSpec> {
    let mut spec = LakeSpec {
        at: vec2(&v["at"])?,
        radius: f(v, "radius").context("lake.radius")?,
        depth: f(v, "depth").context("lake.depth")?,
        water_offset: f(v, "water_offset").unwrap_or(0.5),
        water_level: Some(f(v, "water_level").context("lake.water_level (explícito)")?),
        ..LakeSpec::default()
    };
    spec.shape.seed = v.get("seed").and_then(Value::as_u64);
    spec.shape.axis_deg = f(v, "axis");
    spec.shape.stretch = f(v, "stretch");
    spec.shape.lobes = f(v, "lobes");
    Ok(spec)
}

fn river_spec(v: &Value) -> Result<RiverSpec> {
    let path = v["path"]
        .as_array()
        .context("river.path")?
        .iter()
        .map(vec2)
        .collect::<Result<Vec<_>>>()?;
    let levels = v["water_levels"]
        .as_array()
        .context("river.water_levels (explícitos)")?
        .iter()
        .map(|x| x.as_f64().map(|x| x as f32).context("nível"))
        .collect::<Result<Vec<_>>>()?;
    Ok(RiverSpec {
        path,
        width: f(v, "width").context("river.width")?,
        depth: f(v, "depth").context("river.depth")?,
        water_offset: f(v, "water_offset").unwrap_or(0.3),
        water_levels: Some(levels),
        bank_width: f(v, "bank_width").unwrap_or(2.0),
        bank_height: f(v, "bank_height").unwrap_or(0.9),
        pool_spacing: f(v, "pool_spacing").unwrap_or(0.0),
        ..RiverSpec::default()
    })
}

/// Carva um corpo e devolve o registry. `None` = specs degeneradas.
fn carve(grid: &mut BrushGrid, body: &Value, index: usize, prior: &[WaterBody]) -> Result<Option<WaterBody>> {
    Ok(match body["kind"].as_str() {
        Some("lake") => carve_lake(grid, &lake_spec(body)?, index),
        Some("river") => {
            let spec = river_spec(body)?;
            let stations = spec.water_levels.as_ref().map_or(0, Vec::len);
            let out = carve_river(grid, &spec, index, prior);
            if let Some(b) = &out
                && b.surface_y.len() != stations
            {
                bail!(
                    "rio {index}: {stations} níveis para {} estações — o runtime ia ignorá-los",
                    b.surface_y.len()
                );
            }
            out
        }
        other => bail!("kind desconhecido: {other:?}"),
    })
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dry = args.iter().any(|a| a == "--dry-run");
    let files: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let [ahgt_path, json_path] = files.as_slice() else {
        bail!("uso: carve_water <terrain.ahgt> <water.json> [--dry-run]");
    };
    let ahgt_path = PathBuf::from(ahgt_path);
    let bytes = std::fs::read(&ahgt_path).with_context(|| format!("ler {}", ahgt_path.display()))?;
    let (map, meta) = HeightMapU16::from_ahgt(&bytes)?;
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(json_path)?)?;
    let world_size = meta.world_size.context("o .ahgt não declara worldSize")?;
    let max_height = meta.max_height.context("o .ahgt não declara maxHeight")?;
    let smoothing = doc.get("height_smoothing").and_then(Value::as_f64).unwrap_or(1.0) as f32;
    let mut grid = BrushGrid::from_height_map(&map, world_size, max_height, smoothing)?;

    // Corpos já gravados: carvados numa cópia (o carve deles sobre o .ahgt é
    // idempotente, mas não é este exemplo que os grava).
    let mut scratch = grid.clone();
    let mut bodies: Vec<WaterBody> = Vec::new();
    let empty = Value::Array(Vec::new());
    let prior = doc.get("prior").unwrap_or(&empty).as_array().context("prior")?;
    let new = doc.get("new").unwrap_or(&empty).as_array().context("new")?;
    for (i, body) in prior.iter().enumerate() {
        if let Some(b) = carve(&mut scratch, body, i, &bodies)? {
            bodies.push(b);
        }
    }
    let before = grid.raw().to_vec();
    for (i, body) in new.iter().enumerate() {
        let name = body.get("name").and_then(Value::as_str).unwrap_or("?");
        match carve(&mut grid, body, prior.len() + i, &bodies)? {
            Some(b) => {
                println!("{name}: carvado (lâmina média {:.2} m)", b.water_y);
                bodies.push(b);
            }
            None => println!("{name}: specs degeneradas — ignorado"),
        }
    }
    let scale = max_height / u16::MAX as f32;
    let (mut changed, mut lowered, mut raised) = (0usize, 0.0f32, 0.0f32);
    for (a, b) in before.iter().zip(grid.raw()) {
        if a != b {
            changed += 1;
            let d = (*b as f32 - *a as f32) * scale;
            lowered = lowered.min(d);
            raised = raised.max(d);
        }
    }
    println!("{changed} texels mudaram (mais fundo {lowered:.2} m, mais alto +{raised:.2} m)");
    if dry {
        println!("--dry-run: nada gravado");
        return Ok(());
    }
    let out = HeightMapU16 {
        width: map.width,
        depth: map.depth,
        data: grid.raw().to_vec(),
    };
    std::fs::write(&ahgt_path, out.to_ahgt(&meta)?)?;
    println!("gravado {}", ahgt_path.display());
    Ok(())
}

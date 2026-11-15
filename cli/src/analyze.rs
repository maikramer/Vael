//! `analyze` headless: carrega o mundo (`.bsn` com includes) e produz
//! um relatório estável para o CLI e a extensão do Jackdaw.
//!
//! O contrato é deliberadamente conservador: contagens, tags, assets em falta
//! e scripts em falta. As regras mais finas do `recipes`/`audit` do Viber vão
//! sendo portadas sem mudar o formato de topo.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use bevy_ext_world::xml::XmlNode;
use bevy_ext_world::xml::include::load_world;
use serde::Serialize;

/// Relatório de `analyze`. O JSON é o contrato com agentes/CI.
#[derive(Debug, Clone, Serialize)]
pub struct AnalyzeReport {
    pub world: String,
    pub static_world: bool,
    pub entities: EntityStats,
    pub ambient_light: Option<BTreeMap<String, String>>,
    pub terrain: Option<BTreeMap<String, String>>,
    pub ui: UiStats,
    pub spawn_groups: BTreeMap<String, usize>,
    pub prototypes: Vec<String>,
    /// Nomes de entidades/objetos (`name=`); alimenta o lint de quests/NPCs.
    pub names: Vec<String>,
    /// Ids de `<Prototype>`; alimenta a checagem dos `mob-*` dos scripts.
    pub prototype_ids: Vec<String>,
    pub assets: AssetStats,
    pub scripts: Vec<String>,
    pub infos: Vec<String>,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct EntityStats {
    pub total: usize,
    pub by_tag: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct UiStats {
    pub nodes: usize,
    pub ids: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AssetStats {
    pub references: usize,
    pub missing: Vec<String>,
}

impl AnalyzeReport {
    /// `--strict`: qualquer warning falha o gate. (Os campos
    /// `skipped_tags`/`unknown_prototypes` do contrato antigo saíram: eram
    /// inicializados vazios e nunca preenchidos — as regras finas do audit
    /// do Viber ainda não foram portadas e ninguém os consumia.)
    pub fn strict_failed(&self) -> bool {
        !self.warnings.is_empty() || self.error.is_some()
    }
}

/// Resolve um ficheiro de mundo a partir de um caminho/pasta.
pub fn resolve_world_path(input: Option<&Path>) -> Result<PathBuf> {
    let base = input.unwrap_or_else(|| Path::new("."));
    let absolute = std::path::absolute(base)
        .with_context(|| format!("caminho inválido: {}", base.display()))?;
    if absolute.is_file() {
        return Ok(absolute);
    }
    let dir = if absolute.is_dir() {
        absolute
    } else {
        std::env::current_dir()?
    };
    for name in ["world.bsn", "world.xml"] {
        let path = dir.join(name);
        if path.is_file() {
            return Ok(path);
        }
    }
    // `config.yaml` é YAML; para não trazer serde_yaml ao core, lê a linha
    // `main-scene:` diretamente. O parser completo fica no `bevy_ext_world`.
    if let Ok(config) = std::fs::read_to_string(dir.join("config.yaml"))
        && let Some(scene) = config.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            (key.trim() == "main-scene").then(|| value.trim().trim_matches('"').to_string())
        })
    {
        let path = dir.join(scene);
        if path.is_file() {
            return Ok(path);
        }
    }
    let worlds = dir.join("worlds");
    if let Ok(entries) = std::fs::read_dir(&worlds) {
        let mut candidates: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| matches!(ext, "bsn" | "xml"))
            })
            .collect();
        candidates.sort();
        if let Some(path) = candidates.into_iter().next() {
            return Ok(path);
        }
    }
    bail!(
        "sem mundo em {} (procurei world.bsn, world.xml, config.yaml e worlds/*)",
        dir.display()
    )
}

/// Analisa um mundo. Erros de leitura/parse devolvem um relatório com `error`
/// em vez de `Err`, para o CLI conseguir imprimir JSON mesmo em falha.
pub fn analyze(input: Option<&Path>) -> AnalyzeReport {
    let path = match resolve_world_path(input) {
        Ok(path) => path,
        Err(error) => return error_report(None, format!("{error:#}")),
    };
    let loaded = match load_world(&path) {
        Ok(loaded) => loaded,
        Err(error) => return error_report(Some(&path), format!("{error:#}")),
    };
    let root = path.parent().unwrap_or(Path::new("."));
    let mut report = AnalyzeReport {
        world: path.display().to_string(),
        static_world: root_attr(&loaded.root_attrs, "static")
            .is_some_and(|value| value.eq_ignore_ascii_case("true") || value == "1"),
        entities: EntityStats::default(),
        ambient_light: None,
        terrain: None,
        ui: UiStats::default(),
        spawn_groups: BTreeMap::new(),
        prototypes: Vec::new(),
        names: Vec::new(),
        prototype_ids: Vec::new(),
        assets: AssetStats::default(),
        scripts: Vec::new(),
        infos: Vec::new(),
        warnings: Vec::new(),
        error: None,
    };
    let mut names: BTreeMap<String, usize> = BTreeMap::new();
    let mut assets = BTreeSet::new();
    let mut scripts = BTreeSet::new();
    let mut prototype_defs = BTreeSet::new();
    walk(
        &loaded.nodes,
        &mut report,
        &mut names,
        &mut assets,
        &mut scripts,
        &mut prototype_defs,
    );
    report.names = names.keys().cloned().collect();
    report.prototype_ids = prototype_defs.iter().cloned().collect();
    report.scripts = scripts.into_iter().collect();
    report.assets.references = assets.len();
    report.assets.missing = assets
        .into_iter()
        .filter(|asset| !asset_exists(root, asset))
        .collect();
    for (name, count) in names {
        if count > 1 {
            report
                .warnings
                .push(format!("nome duplicado `{name}` ({count}×)"));
        }
    }
    for script in &report.scripts {
        let rel = script.trim_start_matches('/');
        if !root.join("scripts").join(rel).is_file() {
            report
                .warnings
                .push(format!("script em falta: scripts/{rel}"));
        }
    }
    if loaded.sources.len() > 1 {
        report.infos.push(format!(
            "{} ficheiro(s) carregado(s) para montar o mundo",
            loaded.sources.len()
        ));
    }
    report.infos.push(format!(
        "{} entidade(s), {} asset(s), {} script(s)",
        report.entities.total,
        report.assets.references,
        report.scripts.len()
    ));
    report
}

fn error_report(path: Option<&Path>, error: String) -> AnalyzeReport {
    AnalyzeReport {
        world: path
            .map(|path| path.display().to_string())
            .unwrap_or_default(),
        static_world: false,
        entities: EntityStats::default(),
        ambient_light: None,
        terrain: None,
        ui: UiStats::default(),
        spawn_groups: BTreeMap::new(),
        prototypes: Vec::new(),
        names: Vec::new(),
        prototype_ids: Vec::new(),
        assets: AssetStats::default(),
        scripts: Vec::new(),
        infos: Vec::new(),
        warnings: Vec::new(),
        error: Some(error),
    }
}

fn walk(
    nodes: &[XmlNode],
    report: &mut AnalyzeReport,
    names: &mut BTreeMap<String, usize>,
    assets: &mut BTreeSet<String>,
    scripts: &mut BTreeSet<String>,
    prototype_defs: &mut BTreeSet<String>,
) {
    for node in nodes {
        let tag = node.tag.to_ascii_lowercase();
        report.entities.total += 1;
        *report.entities.by_tag.entry(tag.clone()).or_default() += 1;
        if tag.starts_with("ui") || tag.contains("widget") || tag == "style" {
            report.ui.nodes += 1;
        }
        if node.attr("id").is_some() {
            report.ui.ids += 1;
        }
        if tag == "terrain" && report.terrain.is_none() {
            report.terrain = Some(attrs(node));
        }
        if tag == "ambientlight" && report.ambient_light.is_none() {
            report.ambient_light = Some(attrs(node));
        }
        if matches!(
            tag.as_str(),
            "scatter" | "spawngroup" | "spawner" | "dynamicspawner"
        ) {
            *report.spawn_groups.entry(tag.clone()).or_default() += 1;
        }
        if tag == "prototype"
            && let Some(id) = node.attr("id").or_else(|| node.attr("name"))
        {
            prototype_defs.insert(id.to_string());
            report.prototypes.push(id.to_string());
        }
        if let Some(name) = node.attr("name") {
            *names.entry(name.to_string()).or_default() += 1;
        }
        if tag != "include" {
            for key in [
                "url",
                "model",
                "mesh-url",
                "heightmap",
                "texture",
                "normal-map",
                "icon",
            ] {
                if let Some(value) = node.attr(key)
                    && is_asset_path(value)
                {
                    assets.insert(value.to_string());
                }
            }
            if let Some(script) = node.attr("script") {
                scripts.insert(script.to_string());
            }
        }
        walk(
            &node.children,
            report,
            names,
            assets,
            scripts,
            prototype_defs,
        );
    }
}

fn attrs(node: &XmlNode) -> BTreeMap<String, String> {
    node.attrs.iter().cloned().collect()
}

fn root_attr<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
}

fn is_asset_path(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && !value.starts_with("http://")
        && !value.starts_with("https://")
        && !value.starts_with("data:")
        && !value.starts_with('#')
        && !value.contains("{{")
}

/// O pool do Vael vive em `assets/public`; URLs `/assets/...` resolvem aí.
fn asset_exists(root: &Path, value: &str) -> bool {
    let rel = value.trim_start_matches('/');
    root.join(rel).is_file() || root.join("assets/public").join(rel).is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_world_bsn_before_xml() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("world.xml"), "<world/>").unwrap();
        std::fs::write(dir.path().join("world.bsn"), "").unwrap();
        assert!(
            resolve_world_path(Some(dir.path()))
                .unwrap()
                .ends_with("world.bsn")
        );
    }

    #[test]
    fn analyze_counts_nodes_assets_and_scripts() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("scripts")).unwrap();
        std::fs::write(dir.path().join("scripts/hero.lua"), "").unwrap();
        std::fs::write(
            dir.path().join("world.xml"),
            r#"<World static="true">
                <Group name="g">
                    <Entity name="hero" script="hero.lua" url="/assets/meshes/hero.glb" />
                    <Terrain at="0 0" radius="10" />
                    <UiText id="hint">Oi</UiText>
                </Group>
            </World>"#,
        )
        .unwrap();
        let report = analyze(Some(&dir.path().join("world.xml")));
        assert!(report.error.is_none(), "{:?}", report.error);
        assert!(report.static_world);
        assert_eq!(report.entities.total, 4);
        assert_eq!(report.scripts, vec!["hero.lua"]);
        assert_eq!(report.assets.missing, vec!["/assets/meshes/hero.glb"]);
        assert!(report.terrain.is_some());
        assert_eq!(report.ui.nodes, 1);
    }

    #[test]
    fn analyze_reports_parse_errors_as_json_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("world.xml"), "<World><Group>").unwrap();
        let report = analyze(Some(&dir.path().join("world.xml")));
        assert!(report.error.is_some());
        assert!(report.strict_failed());
    }
}

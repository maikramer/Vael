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
use bevy_ext_world::xml::include::load_world_in;
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
    /// `--strict`: qualquer warning (ou asset em falta) falha o gate. (Os
    /// campos `skipped_tags`/`unknown_prototypes` do contrato antigo saíram:
    /// eram inicializados vazios e nunca preenchidos — as regras finas do
    /// audit do Viber ainda não foram portadas e ninguém os consumia.)
    pub fn strict_failed(&self) -> bool {
        // Os assets em falta vivem em `assets.missing`, não em `warnings`:
        // sem esta linha o `--strict` passava um mundo com assets partidos
        // (o lint já os promove a aviso).
        !self.assets.missing.is_empty() || !self.warnings.is_empty() || self.error.is_some()
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
    // Um caminho EXPLÍCITO que não existe não pode cair no mundo do diretório
    // atual: um nome mal escrito no gate analisava outro mundo e dava exit 0.
    if let Some(input) = input
        && !absolute.is_dir()
    {
        bail!("caminho inexistente: {}", input.display());
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
            let (key, value) = config_entry(line)?;
            (key == "main-scene" && !value.is_empty()).then(|| value.to_string())
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

/// Raiz do projeto de um ficheiro de mundo/módulo: o ancestral mais próximo
/// com `world.bsn` ou `config.yaml`. Os `Include` `/…` e os assets resolvem
/// SEMPRE contra ela (como no runtime) — com a pasta do ficheiro,
/// `world/cities/discordia.bsn` procurava `world/cities/world/…` e
/// `scenes/menu.bsn` dava os assets do pool por em falta.
///
/// Sem nenhum marcador (um mundo solto, ex.: um `qa/*.bsn` ao lado) fica a
/// pasta do próprio ficheiro, como sempre foi.
pub fn project_root_of(path: &Path) -> PathBuf {
    let absolute = path
        .canonicalize()
        .or_else(|_| std::path::absolute(path))
        .unwrap_or_else(|_| path.to_path_buf());
    // Um ficheiro começa na pasta dele; uma pasta em si mesma.
    let start = if absolute.is_dir() {
        absolute.clone()
    } else {
        absolute
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    };
    for candidate in start.ancestors() {
        if candidate.join("world.bsn").is_file() || candidate.join("config.yaml").is_file() {
            return candidate.to_path_buf();
        }
    }
    start
}

/// Uma entrada `chave: valor` de uma linha do `config.yaml`: a chave aparada e
/// o valor sem comentário inline nem quotes. O parser completo é o do
/// `bevy_ext_world`; o `analyze` só precisa de uma chave e não pode assumir o
/// projeto todo (o `config.yaml` pode ter blocos que o CLI não conhece).
fn config_entry(line: &str) -> Option<(&str, &str)> {
    let (key, value) = line.split_once(':')?;
    Some((key.trim(), yaml_scalar(value)))
}

/// Valor escalar de YAML: corta o comentário `#` (só fora de quotes e no
/// início ou depois de espaço, como o YAML) e as quotes à volta. O antigo
/// `value.trim().trim_matches('"')` lia `world.bsn  # a cena` inteiro como
/// caminho e a linha não resolvia.
fn yaml_scalar(raw: &str) -> &str {
    let raw = raw.trim_start();
    let mut quote: Option<char> = None;
    let mut end = raw.len();
    for (index, c) in raw.char_indices() {
        match c {
            '"' | '\'' => {
                if quote == Some(c) {
                    quote = None;
                } else if quote.is_none() {
                    quote = Some(c);
                }
            }
            '#' if quote.is_none()
                && (index == 0
                    || raw[..index]
                        .chars()
                        .next_back()
                        .is_some_and(char::is_whitespace)) =>
            {
                end = index;
                break;
            }
            _ => {}
        }
    }
    raw[..end]
        .trim()
        .trim_matches(|c| c == '"' || c == '\'')
        .trim()
}

/// Analisa um mundo. Erros de leitura/parse devolvem um relatório com `error`
/// em vez de `Err`, para o CLI conseguir imprimir JSON mesmo em falha.
pub fn analyze(input: Option<&Path>) -> AnalyzeReport {
    let path = match resolve_world_path(input) {
        Ok(path) => path,
        Err(error) => return error_report(None, format!("{error:#}")),
    };
    // A raiz do PROJETO (não a pasta do ficheiro): é o que o runtime usa para
    // os `/…` dos includes, os scripts e o pool de assets.
    let root = project_root_of(&path);
    let loaded = match load_world_in(&path, &root) {
        Ok(loaded) => loaded,
        Err(error) => return error_report(Some(&path), format!("{error:#}")),
    };
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
        .filter(|asset| !asset_exists(&root, asset))
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
        let is_ui = tag.starts_with("ui") || tag.contains("widget") || tag == "style";
        if is_ui {
            report.ui.nodes += 1;
            // `ui.ids` conta ids de UI; os ids de `<Prototype>` (e outros) não
            // são nós de UI e inflavam a contagem.
            if node.attr("id").is_some() {
                report.ui.ids += 1;
            }
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
    fn explicit_missing_path_is_an_error_not_the_cwd_world() {
        // Regressão: um caminho explícito inexistente caía no `current_dir` e
        // analisava o mundo do cwd (exit 0 com um mundo que não foi pedido).
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("world.bsn"), "").unwrap();
        let missing = dir.path().join("typo.bsn");
        let error = resolve_world_path(Some(&missing)).unwrap_err();
        assert!(format!("{error:#}").contains("inexistente"), "{error:#}");
        // Sem caminho, a pasta atual continua a resolver.
        assert!(resolve_world_path(Some(dir.path())).is_ok());
    }

    #[test]
    fn ui_ids_count_only_ui_nodes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("world.xml"),
            r#"<World static="true">
                <Prototype id="mob-wolf" />
                <UiText id="hint">Oi</UiText>
            </World>"#,
        )
        .unwrap();
        let report = analyze(Some(&dir.path().join("world.xml")));
        assert!(report.error.is_none(), "{:?}", report.error);
        // O id do <Prototype> não é um id de UI.
        assert_eq!(report.ui.nodes, 1);
        assert_eq!(report.ui.ids, 1);
    }

    #[test]
    fn strict_fails_on_missing_assets() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("world.xml"),
            r#"<World><Entity name="e" url="/assets/falta.glb" /></World>"#,
        )
        .unwrap();
        let report = analyze(Some(&dir.path().join("world.xml")));
        assert!(report.error.is_none(), "{:?}", report.error);
        assert_eq!(report.assets.missing.len(), 1);
        // Os assets em falta não entram em `warnings`; o `--strict` falhava.
        assert!(report.warnings.is_empty());
        assert!(report.strict_failed());
    }

    #[test]
    fn analyze_reports_parse_errors_as_json_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("world.xml"), "<World><Group>").unwrap();
        let report = analyze(Some(&dir.path().join("world.xml")));
        assert!(report.error.is_some());
        assert!(report.strict_failed());
    }

    #[test]
    fn project_root_is_the_ancestor_with_the_world() {
        // Regressão: `analyze world/cities/discordia.bsn` procurava os
        // `Include` em `world/cities/world/…` (pasta do módulo) e falhava.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("world/cities")).unwrap();
        std::fs::write(dir.path().join("world.bsn"), "").unwrap();
        let module = dir.path().join("world/cities/discordia.bsn");
        std::fs::write(&module, "").unwrap();
        let expected = dir.path().canonicalize().unwrap();
        assert_eq!(project_root_of(&module), expected);
        // A pasta do projeto também resolve para ela mesma.
        assert_eq!(project_root_of(&expected), expected);
    }

    #[test]
    fn project_root_falls_back_to_the_file_folder_for_a_loose_world() {
        let dir = tempfile::tempdir().unwrap();
        let loose = dir.path().join("qa-arena.bsn");
        std::fs::write(&loose, "").unwrap();
        assert_eq!(project_root_of(&loose), dir.path().canonicalize().unwrap());
    }

    #[test]
    fn yaml_scalar_strips_comments_only_outside_quotes() {
        // Regressão: `main-scene: world.bsn   # a cena` virava o caminho com
        // o comentário colado e não resolvia.
        assert_eq!(yaml_scalar(" world.bsn   # a cena"), "world.bsn");
        assert_eq!(yaml_scalar("\"world.bsn\""), "world.bsn");
        assert_eq!(yaml_scalar("'world.bsn'  # nota"), "world.bsn");
        assert_eq!(yaml_scalar("a#b.bsn"), "a#b.bsn");
        assert_eq!(yaml_scalar("\"a#b.bsn\"  # x"), "a#b.bsn");
        assert_eq!(yaml_scalar(""), "");
    }

    #[test]
    fn config_main_scene_ignores_inline_comment() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("maps")).unwrap();
        std::fs::write(dir.path().join("maps/game.xml"), "<world/>").unwrap();
        std::fs::write(
            dir.path().join("config.yaml"),
            "project:\n  main-scene: maps/game.xml   # a cena de arranque\n",
        )
        .unwrap();
        let resolved = resolve_world_path(Some(dir.path())).unwrap();
        assert!(
            resolved.ends_with("maps/game.xml"),
            "{}",
            resolved.display()
        );
    }
}

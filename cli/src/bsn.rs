//! `vael-tools bsn` — conversão e verificação do mundo em `.bsn` (Jackdaw).
//!
//! O `.bsn` é uma árvore de entidades com `WorldNode { tag, attrs, text }`;
//! o loader do jogo reconstrói os `XmlNode` e usa o parser/spawner atuais.
//! A CLI converte XML→BSN e compara os dois pelo significado. O `.xml` já
//! não é fonte do mundo: o `convert` serve os geradores (ex.: `tools/qa.py`).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use bevy_ext_world::bsn::{EmitOptions, emit_file_checked, worlds_equivalent};
use bevy_ext_world::xml::include::{load_world_in, resolve_include_path};

/// Converte XML (+ includes) para `.bsn`. Devolve quantos ficheiros escreveu.
pub fn convert(
    paths: &[PathBuf],
    root: Option<&Path>,
    out: Option<&Path>,
    force: bool,
    check: bool,
    preview: bool,
) -> Result<usize> {
    let paths = default_paths(paths);
    let root = resolve_root(&paths, root)?;
    let options = EmitOptions { preview };
    convert_files(&paths, &root, out, force, check, options)
}

/// Compara XML e `.bsn` pelo significado. Devolve quantos ficheiros conferiu.
///
/// Um `.bsn` passado explicitamente (o `world.bsn` do docs/MIGRATION.md) não
/// tem XML com que comparar: nesse caso o check é **carregá-lo** — um BSN
/// malformado falha o gate em vez de ser comparado consigo mesmo (exit 0
/// sempre).
pub fn check(paths: &[PathBuf], root: Option<&Path>, out: Option<&Path>) -> Result<usize> {
    let paths = default_paths(paths);
    let root = resolve_root(&paths, root)?;
    let mut checked = 0;
    for path in discover_xml(&paths)? {
        if is_bsn(&path) {
            load_world_in(&path, &root)
                .with_context(|| format!("validar BSN {}", path.display()))?;
            checked += 1;
            continue;
        }
        let bsn = output_path(&path, &root, out);
        compare(&path, &bsn, &root)?;
        checked += 1;
    }
    Ok(checked)
}

fn default_paths(paths: &[PathBuf]) -> Vec<PathBuf> {
    if paths.is_empty() {
        vec![PathBuf::from("world.xml")]
    } else {
        paths.to_vec()
    }
}

fn resolve_root(paths: &[PathBuf], root: Option<&Path>) -> Result<PathBuf> {
    if let Some(root) = root {
        return root
            .canonicalize()
            .with_context(|| format!("raiz inválida: {}", root.display()));
    }
    let first = paths
        .first()
        .ok_or_else(|| anyhow::anyhow!("sem caminhos"))?;
    let first = first
        .canonicalize()
        .with_context(|| format!("caminho inválido: {}", first.display()))?;
    // `root_of` trata o caminho como um ficheiro. Passar-lhe uma pasta dava o
    // avô do projeto (ex.: `Vael/` → `GitClones/`), e os includes `/world/…`
    // deixavam de resolver. Ancora num ficheiro: o primeiro XML da pasta.
    let anchor = if first.is_dir() {
        if first.join(bevy_ext_world::project::CONFIG_FILE).is_file() {
            return Ok(first);
        }
        let mut xmls = Vec::new();
        walk_xml(&first, &mut xmls)?;
        xmls.into_iter().next().unwrap_or(first)
    } else {
        first
    };
    Ok(bevy_ext_world::project::root_of(&anchor))
}

fn convert_files(
    paths: &[PathBuf],
    root: &Path,
    out: Option<&Path>,
    force: bool,
    check: bool,
    options: EmitOptions,
) -> Result<usize> {
    let mut work: BTreeSet<PathBuf> = discover_xml(paths)?.into_iter().collect();
    let mut converted = 0;
    while let Some(xml) = work.pop_first() {
        let text = emit_file_checked(&xml, root, options)?;
        let target = output_path(&xml, root, out);
        if target.exists() {
            let current = std::fs::read_to_string(&target).unwrap_or_default();
            if current != text && !force {
                bail!(
                    "{} já existe e difere; usa --force para o substituir",
                    target.display()
                );
            }
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("criar {}", parent.display()))?;
        }
        std::fs::write(&target, &text).with_context(|| format!("gravar {}", target.display()))?;
        converted += 1;

        if check {
            compare(&xml, &target, root)?;
        }
        for include in includes_of(&xml, root)? {
            work.insert(include);
        }
    }
    Ok(converted)
}

fn discover_xml(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for path in paths {
        if path.is_dir() {
            walk_xml(path, &mut out)?;
        } else if path.is_file() {
            out.push(path.canonicalize()?);
        } else {
            bail!("caminho inexistente: {}", path.display());
        }
    }
    out.sort();
    out.dedup();
    // Zero ficheiros fazia o `convert`/`check` sair com 0 e imprimir
    // "0 ficheiro(s) …": um gate que não verificou nada passava.
    if out.is_empty() {
        bail!(
            "nenhum ficheiro .xml encontrado em {}",
            paths
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    Ok(out)
}

fn is_bsn(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("bsn"))
}

fn walk_xml(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("ler {}", dir.display()))?
        .flatten()
        .map(|entry| entry.path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if matches!(name, "assets" | "target" | ".git" | ".jackdaw") || name.ends_with(".data")
            {
                continue;
            }
            walk_xml(&path, out)?;
        } else if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("xml"))
        {
            out.push(path.canonicalize()?);
        }
    }
    Ok(())
}

fn includes_of(xml: &Path, root: &Path) -> Result<Vec<PathBuf>> {
    let doc = bevy_ext_world::xml::parse_file(xml)?;
    let mut out = Vec::new();
    collect_includes(&doc.children, xml, root, &mut out)?;
    Ok(out)
}

fn collect_includes(
    nodes: &[bevy_ext_world::xml::XmlNode],
    file: &Path,
    root: &Path,
    out: &mut Vec<PathBuf>,
) -> Result<()> {
    let dir = file.parent().unwrap_or(root);
    for node in nodes {
        if node.tag.eq_ignore_ascii_case("include")
            && let Some(src) = node.attr("src")
        {
            let resolved = resolve_include_path(src, dir, root);
            if resolved
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("xml"))
                && resolved.is_file()
            {
                out.push(resolved.canonicalize()?);
            }
        }
        collect_includes(&node.children, file, root, out)?;
    }
    Ok(())
}

fn output_path(xml: &Path, root: &Path, out: Option<&Path>) -> PathBuf {
    match out {
        Some(dir) => {
            // Fora da raiz, `strip_prefix` falha e o antigo `dir.join(abs)`
            // devolvia o caminho absoluto: `--out` era ignorado em silêncio e
            // o ficheiro era escrito ao lado do XML. Usar o nome resolve.
            let relative = xml
                .strip_prefix(root)
                .unwrap_or_else(|_| Path::new(xml.file_name().unwrap_or_else(|| xml.as_os_str())));
            dir.join(relative).with_extension("bsn")
        }
        None => xml.with_extension("bsn"),
    }
}

fn compare(xml: &Path, bsn: &Path, root: &Path) -> Result<()> {
    let from_xml =
        load_world_in(xml, root).with_context(|| format!("carregar XML {}", xml.display()))?;
    let from_bsn =
        load_world_in(bsn, root).with_context(|| format!("carregar BSN {}", bsn.display()))?;
    worlds_equivalent(
        &from_xml.root_attrs,
        &from_xml.nodes,
        &from_bsn.root_attrs,
        &from_bsn.nodes,
    )
    .map_err(|err| anyhow::anyhow!("{}: {err}", xml.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regressão: `resolve_root` passava a pasta à `root_of`, que a trata como
    /// ficheiro e devolvia o avô; os includes `/world/…` deixavam de resolver.
    #[test]
    fn raiz_do_projeto_a_partir_de_ficheiro_ou_pasta() {
        let dir = std::env::temp_dir().join(format!("vael-tools-bsn-root-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("world")).expect("criar pasta");
        std::fs::write(
            dir.join("config.yaml"),
            "project:\n  main-scene: world.bsn\n",
        )
        .expect("config");
        std::fs::write(dir.join("world.xml"), "<world static=\"true\" />").expect("world");
        std::fs::write(dir.join("world/mod.xml"), "<world static=\"true\" />").expect("mod");

        let expected = dir.canonicalize().expect("canonicalizar");
        assert_eq!(
            resolve_root(&[dir.join("world.xml")], None).unwrap(),
            expected
        );
        assert_eq!(
            resolve_root(std::slice::from_ref(&dir), None).unwrap(),
            expected
        );

        std::fs::remove_dir_all(&dir).expect("limpar");
    }

    #[test]
    fn discover_sem_xml_falha_em_vez_de_verificar_zero() {
        // Regressão: uma pasta sem XML dava "0 ficheiro(s)" e exit 0 — um gate
        // que não verificou nada passava.
        let dir = std::env::temp_dir().join(format!("vael-tools-bsn-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("criar pasta");
        let error = discover_xml(std::slice::from_ref(&dir)).unwrap_err();
        assert!(
            format!("{error:#}").contains("nenhum ficheiro"),
            "{error:#}"
        );
        assert!(check(std::slice::from_ref(&dir), None, None).is_err());
        std::fs::remove_dir_all(&dir).expect("limpar");
    }

    #[test]
    fn check_de_bsn_valida_o_ficheiro_em_vez_de_o_comparar_consigo() {
        // Regressão: `bsn check <ficheiro.bsn>` comparava o ficheiro consigo
        // mesmo (sempre "iguais", exit 0) — um BSN malformado passava o gate.
        let dir = std::env::temp_dir().join(format!("vael-tools-bsn-check-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("criar pasta");
        let bsn = dir.join("world.bsn");
        std::fs::write(&bsn, "World { Entity(").expect("escrever bsn");
        let error = check(std::slice::from_ref(&bsn), None, None).unwrap_err();
        assert!(format!("{error:#}").contains("BSN"), "{error:#}");
        std::fs::remove_dir_all(&dir).expect("limpar");
    }

    #[test]
    fn out_honrado_para_ficheiro_fora_da_raiz() {
        // Regressão: `dir.join(absolute)` devolvia o absoluto e `--out` era
        // ignorado, escrevendo ao lado do XML.
        let root = Path::new("/projeto");
        let xml = Path::new("/fora/docs/world.xml");
        let out = Path::new("/saida");
        assert_eq!(
            output_path(xml, root, Some(out)),
            Path::new("/saida/world.bsn")
        );
    }
}

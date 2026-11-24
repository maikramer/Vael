//! Offline source editing. Work on a single physical document, including
//! native-only components; resolving prefabs first would lose ownership.
mod document;
mod settle;
#[cfg(test)]
mod tests;

use anyhow::{Context, Result, bail, ensure};
use clap::{Args, Subcommand};
use document::{index, select};
use serde::Deserialize;
use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Subcommand)]
pub enum Command {
    /// Pré-visualiza ou grava o assentamento de props no heightmap do mundo.
    Settle(settle::Options),
    /// Lista as entidades físicas, caminhos, IDs e componentes.
    Inspect {
        file: PathBuf,
        #[arg(long)]
        tag: Option<String>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        component: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Mostra o BSN original de uma entidade, incluindo os filhos.
    Show {
        file: PathBuf,
        #[arg(long)]
        node: String,
    },
    /// Altera um atributo WorldNode ou um campo de componente.
    Set {
        #[command(flatten)]
        target: Target,
        #[command(flatten)]
        edit: Edit,
    },
    /// Aplica em ordem um array JSON de edições, com uma única gravação.
    Patch {
        #[command(flatten)]
        target: Target,
        edits: PathBuf,
    },
    /// Acrescenta uma entidade BSN (com a sua subárvore) aos filhos locais.
    Insert {
        #[command(flatten)]
        target: Target,
        #[arg(long)]
        parent: String,
        fragment: PathBuf,
    },
    /// Cria uma cópia .bsb de um .bsn, ou uma cópia textual de um .bsb.
    Transcode { input: PathBuf, output: PathBuf },
    /// Consulta campos, variantes e defaults do schema extraído pelo Jackdaw.
    Schema {
        component: String,
        #[arg(long, default_value = ".")]
        project: PathBuf,
    },
}

#[derive(Args)]
pub struct Target {
    /// Documento .bsn no disco.
    file: PathBuf,
    /// Emite o documento resultante em stdout, sem gravar.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Args, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    /// Caminho de inspect (0/1), id:N ou name:NOME (exato e único).
    #[arg(long)]
    node: String,
    /// Chave WorldNode.attrs (texto). Cria a chave se estiver ausente.
    #[arg(long, conflicts_with = "field", required_unless_present = "field")]
    attr: Option<String>,
    /// Componente.campo[.subcampo], por nome completo ou curto único.
    #[arg(long, conflicts_with = "attr", required_unless_present = "attr")]
    field: Option<String>,
    /// Texto literal para --attr ou um valor BSN para --field.
    #[arg(long, allow_hyphen_values = true)]
    value: String,
}

fn source_path(file: &Path) -> Result<PathBuf> {
    ensure!(
        file.extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("bsn")),
        "esperado documento .bsn: {}",
        file.display()
    );
    file.canonicalize()
        .with_context(|| format!("abrir {}", file.display()))
}

fn read_scene(path: &Path) -> Result<String> {
    ensure!(
        jackdaw_bsn::file::is_document_path(path),
        "esperado .bsn ou .bsb"
    );
    Ok(jackdaw_bsn::file::read_document_text(path)?)
}

fn transcode(input: &Path, output: &Path) -> Result<()> {
    use jackdaw_bsn::file;
    ensure!(
        file::is_document_path(input)
            && file::is_document_path(output)
            && file::is_binary_path(input) != file::is_binary_path(output),
        "indique uma origem .bsn e um destino .bsb, ou vice-versa"
    );
    let text = read_scene(input)?;
    jackdaw_bsn::parse_bsn_text(&text).context("ler cena")?;
    let bytes = file::document_bytes(output, &text)?;
    // Check that the native writer can read its output before publishing it.
    file::document_from_bytes(&bytes, output)?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(&bytes)?;
    tmp.persist_noclobber(output)
        .with_context(|| format!("criar {} (o destino precisa ser novo)", output.display()))?;
    println!("{}", output.display());
    Ok(())
}

fn update(target: Target, edit: impl FnOnce(&mut String) -> Result<()>) -> Result<()> {
    let path = source_path(&target.file)?;
    let file = fs::File::open(&path)?;
    // Serialize CLI writers; writers already waiting on a replaced inode
    // compare their snapshot with the live path before publishing.
    if !target.dry_run {
        file.lock().context("bloquear documento")?;
    }
    let original = std::io::read_to_string(&file)?;
    let mut text = original.clone();
    edit(&mut text)?;
    jackdaw_bsn::parse_bsn_text(&text).context("resultado rejeitado pelo Jackdaw")?;
    if target.dry_run {
        print!("{text}");
        return Ok(());
    }
    if original == text {
        return Ok(());
    }
    let mut tmp = tempfile::NamedTempFile::new_in(path.parent().context("ficheiro sem pasta")?)?;
    tmp.as_file()
        .set_permissions(file.metadata()?.permissions())?;
    tmp.write_all(text.as_bytes())?;
    tmp.as_file().sync_all()?;
    ensure!(
        fs::read(&path)? == original.as_bytes(),
        "o documento mudou durante a edição; releia-o"
    );
    tmp.persist(&path)
        .with_context(|| format!("gravar {}", path.display()))?;
    println!("{}", path.display());
    Ok(())
}

fn apply(text: &mut String, edit: &Edit) -> Result<()> {
    let nodes = index(text)?;
    let node = select(&nodes, &edit.node)?;
    let (range, replacement) = match (&edit.attr, &edit.field) {
        (Some(key), None) => document::attribute(text, node, key, &edit.value)?,
        (None, Some(field)) => {
            document::value(&edit.value)?;
            (
                document::field(text, node, field)?,
                edit.value.trim().into(),
            )
        }
        _ => bail!("indique exatamente um de attr e field"),
    };
    text.replace_range(range, &replacement);
    Ok(())
}

impl Command {
    pub fn run(self) -> Result<()> {
        match self {
            Self::Settle(options) => settle::run(options)?,
            Self::Inspect {
                file,
                tag,
                name,
                component,
                json,
            } => {
                let text = read_scene(&file)?;
                let mut nodes = index(&text)?;
                nodes.retain(|n| {
                    tag.as_ref().is_none_or(|t| n.tag.as_ref() == Some(t))
                        && name.as_ref().is_none_or(|s| n.name.as_ref() == Some(s))
                        && component.as_ref().is_none_or(|c| {
                            n.components
                                .iter()
                                .any(|p| p == c || p.rsplit("::").next() == Some(c.as_str()))
                        })
                });
                if json {
                    println!("{}", serde_json::to_string_pretty(&nodes)?);
                } else {
                    for n in nodes {
                        println!(
                            "{}\tL{}\t{}\t{}\t{}\t{}",
                            n.path,
                            n.line,
                            n.id.as_deref().unwrap_or("-"),
                            n.tag.as_deref().unwrap_or("-"),
                            n.name.as_deref().unwrap_or("-"),
                            n.components.join(", ")
                        );
                    }
                }
            }
            Self::Show { file, node } => {
                let text = read_scene(&file)?;
                let nodes = index(&text)?;
                println!("{}", &text[select(&nodes, &node)?.range.clone()]);
            }
            Self::Set { target, edit } => update(target, |text| apply(text, &edit))?,
            Self::Patch { target, edits } => {
                let edits: Vec<Edit> = serde_json::from_slice(
                    &fs::read(&edits).with_context(|| format!("ler {}", edits.display()))?,
                )?;
                ensure!(!edits.is_empty(), "lista de edições vazia");
                update(target, |text| {
                    for (i, edit) in edits.iter().enumerate() {
                        apply(text, edit)
                            .with_context(|| format!("edição {} ({})", i + 1, edit.node))?;
                    }
                    Ok(())
                })?;
            }
            Self::Insert {
                target,
                parent,
                fragment,
            } => {
                let fragment = fs::read_to_string(source_path(&fragment)?)?;
                let subtree = index(&fragment)?;
                ensure!(
                    !subtree.is_empty()
                        && subtree.iter().filter(|n| !n.path.contains('/')).count() == 1,
                    "o fragmento deve conter uma entidade raiz"
                );
                update(target, |text| {
                    let nodes = index(text)?;
                    let mut ids: HashSet<_> = nodes.iter().filter_map(|n| n.id.as_ref()).collect();
                    for id in subtree.iter().filter_map(|n| n.id.as_ref()) {
                        ensure!(ids.insert(id), "SceneNodeId repetido: {id}");
                    }
                    let (range, block) =
                        document::insert_child(text, select(&nodes, &parent)?, &fragment)?;
                    text.replace_range(range, &block);
                    Ok(())
                })?;
            }
            Self::Transcode { input, output } => transcode(&input, &output)?,
            Self::Schema { component, project } => {
                let path = project.join(".jackdaw/schema.json");
                let schema: serde_json::Value =
                    serde_json::from_slice(&fs::read(&path).with_context(|| {
                        format!(
                            "{} ausente ou ilegível; jd build extrai o schema",
                            path.display()
                        )
                    })?)?;
                let entries = schema["components"]
                    .as_array()
                    .context("schema sem components")?;
                let matches: Vec<_> = entries
                    .iter()
                    .filter(|c| {
                        c["type_path"].as_str() == Some(&component)
                            || c["short_name"].as_str() == Some(&component)
                    })
                    .collect();
                ensure!(
                    !matches.is_empty(),
                    "componente ausente no schema: {component}"
                );
                println!("{}", serde_json::to_string_pretty(&matches)?);
            }
        }
        Ok(())
    }
}

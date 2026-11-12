//! Leitura e escrita dos ficheiros de conteúdo.
//!
//! A escrita é atómica e deixa um `.bak` ao lado: o editor pode ser fechado a
//! meio de uma gravação sem deixar um JSON truncado. O formato é o mesmo dos
//! ficheiros atuais (pretty, 2 espaços, newline final).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// Lê um JSON com o caminho no erro.
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    let text = fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))?;
    serde_json::from_str(&text).map_err(|err| format!("{}: {err}", path.display()))
}

/// Lê um JSON que pode não existir: ausente devolve `Default`.
pub fn read_json_or_default<T: DeserializeOwned + Default>(path: &Path) -> Result<T, String> {
    match read_json(path) {
        Ok(value) => Ok(value),
        Err(err) if !path.exists() => {
            let _ = err;
            Ok(T::default())
        }
        Err(err) => Err(err),
    }
}

/// Escreve JSON com 2 espaços e newline final, atomicamente, guardando `.bak`.
pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let mut text =
        serde_json::to_string_pretty(value).map_err(|err| format!("{}: {err}", path.display()))?;
    text.push('\n');

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("{}: {err}", parent.display()))?;
    }

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("{}: nome de ficheiro inválido", path.display()))?;
    let tmp = path.with_file_name(format!(".{file_name}.tmp"));
    let backup = path.with_file_name(format!("{file_name}.bak"));

    {
        let mut file = fs::File::create(&tmp).map_err(|err| format!("{}: {err}", tmp.display()))?;
        file.write_all(text.as_bytes())
            .map_err(|err| format!("{}: {err}", tmp.display()))?;
        file.sync_all()
            .map_err(|err| format!("{}: {err}", tmp.display()))?;
    }

    if path.exists() {
        fs::copy(path, &backup).map_err(|err| format!("{}: {err}", backup.display()))?;
    }
    replace_file(&tmp, path).map_err(|err| format!("{}: {err}", path.display()))?;
    Ok(())
}

#[cfg(unix)]
fn replace_file(tmp: &Path, path: &Path) -> std::io::Result<()> {
    fs::rename(tmp, path)
}

#[cfg(not(unix))]
fn replace_file(tmp: &Path, path: &Path) -> std::io::Result<()> {
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(tmp, path)
}

/// Nomes (`*.json`) de uma pasta, sem `index.json`, ordenados.
pub fn list_json_files(dir: &Path) -> Result<Vec<String>, String> {
    let entries = fs::read_dir(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
        .filter(|name| {
            name.ends_with(".json")
                && name != "index.json"
                && !name.ends_with(".bak")
                && !name.starts_with('.')
        })
        .collect();
    names.sort();
    Ok(names)
}

/// Caminho de um ficheiro dentro da raiz do projeto.
pub fn in_root(root: &Path, rel: impl AsRef<Path>) -> PathBuf {
    root.join(rel)
}

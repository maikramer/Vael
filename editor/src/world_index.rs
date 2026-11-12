//! Índice de nomes de entidades do mundo estático.
//!
//! O Jackdaw não aplica o mundo como ECS do jogo, por isso o editor não tem o
//! ECS vivo. Para validar `npc`, marcos e `visit`, lê `world.bsn` (ou o
//! `world.xml` legado) pelo mesmo `load_world` do runtime — que já expande
//! includes — e recolhe os atributos `name`. É só leitura e nunca falha por um
//! ficheiro inválido: acumula o erro e continua.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use bevy_world_xml::xml::XmlNode;
use bevy_world_xml::xml::include::load_world_in;
use vael_content::WorldNames;

#[derive(Debug, Clone, Default)]
pub struct WorldIndex {
    pub names: WorldNames,
    pub files: usize,
    pub errors: Vec<String>,
}

impl WorldIndex {
    /// Lê a cena principal e todas as cenas em `scenes/`. Sem mundo, devolve
    /// um índice vazio com erro.
    pub fn load(root: &Path) -> Self {
        let mut index = Self::default();
        let main = ["world.bsn", "world.xml"]
            .into_iter()
            .map(|name| root.join(name))
            .find(|path| path.is_file());
        match main {
            Some(world) => index.visit(&world, root),
            None => index.errors.push(format!(
                "{}: sem world.bsn/world.xml — a validação de npc/marcos fica limitada",
                root.display()
            )),
        }
        index.scan_dir(&root.join("scenes"), root);
        index
    }

    fn scan_dir(&mut self, dir: &Path, root: &Path) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
        paths.sort();
        let mut scenes: BTreeMap<PathBuf, PathBuf> = BTreeMap::new();
        for path in paths {
            if path.is_dir() {
                self.scan_dir(&path, root);
                continue;
            }
            let is_scene = path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| {
                    ext.eq_ignore_ascii_case("bsn") || ext.eq_ignore_ascii_case("xml")
                });
            if !is_scene {
                continue;
            }
            let key = path.with_extension("");
            let is_bsn = path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("bsn"));
            match scenes.get(&key) {
                Some(_) if !is_bsn => {}
                _ => {
                    scenes.insert(key, path);
                }
            }
        }
        for path in scenes.into_values() {
            self.visit(&path, root);
        }
    }

    fn visit(&mut self, path: &Path, root: &Path) {
        match load_world_in(path, root) {
            Ok(loaded) => {
                self.files += 1;
                collect_names(&loaded.nodes, &mut self.names);
            }
            Err(err) => self.errors.push(format!("{err:#}")),
        }
    }
}

fn collect_names(nodes: &[XmlNode], names: &mut WorldNames) {
    for node in nodes {
        if let Some(name) = node.attr("name") {
            names.insert(name);
        }
        collect_names(&node.children, names);
    }
}

//! Batch seating for the static BSN world. Measure resolved instances and
//! edit only their physical Transform, keeping prefab definitions and IDs.
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    ops::Range,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use bevy_ext_gltf::glb_bounds::{GlbBounds, read_json_chunk};
use bevy_ext_terrain::ahgt::AhgtHeightField;
use bevy_ext_world::{
    bsn::{load_bsn_world, read_pose, specs::scene_node_id_for},
    vinst,
    xml::XmlNode,
};
use bevy_math::{Affine3A, Vec2, Vec3, Vec3Swizzles};
use bevy_voxel_terrain::HeightField;
use clap::Args;
use serde::Serialize;

use super::{Target, document};

#[derive(Args)]
pub struct Options {
    /// Raiz .bsn do mundo, com o terreno e os módulos ativos.
    world: PathBuf,
    /// AHGT de destino. Por defeito lê o declarado pelo mundo.
    #[arg(long)]
    terrain: Option<PathBuf>,
    /// Permite correções grandes quando a base apoiava no terreno anterior.
    #[arg(long)]
    reference: Option<PathBuf>,
    /// Pasta de assets adicional para GLBs ausentes no pool principal.
    #[arg(long)]
    asset_root: Vec<PathBuf>,
    /// Restringe a um ficheiro ou pasta do projeto. Aceita vários --scope.
    #[arg(long)]
    scope: Vec<PathBuf>,
    /// Maior folga ou enterro que se corrige automaticamente, em metros.
    #[arg(long, default_value_t = 7.0)]
    limit: f32,
    /// JSON com deslocamentos e motivos das exclusões.
    #[arg(long)]
    report: Option<PathBuf>,
    /// Grava os ficheiros. Sem esta opção só calcula a prévia.
    #[arg(long)]
    apply: bool,
}

struct TargetPose {
    name: String,
    translation: Vec3,
    delta: f32,
    position: Vec2,
}

#[derive(Serialize)]
struct Move {
    file: PathBuf,
    id: String,
    name: String,
    delta_m: f32,
    world_xz: [f32; 2],
}

#[derive(Default, Serialize)]
struct Report {
    objects: Vec<Move>,
    vegetation: BTreeMap<PathBuf, usize>,
    skipped: BTreeMap<&'static str, usize>,
}

impl Report {
    fn skip(&mut self, reason: &'static str) {
        *self.skipped.entry(reason).or_default() += 1;
    }
}

struct Measure<'a> {
    root: &'a Path,
    asset_roots: Vec<PathBuf>,
    terrain: &'a AhgtHeightField,
    reference: Option<&'a AhgtHeightField>,
    limit: f32,
    scoped_ids: Option<HashSet<u64>>,
    bounds: HashMap<String, Option<GlbBounds>>,
    targets: HashMap<u64, Option<TargetPose>>,
    batches: BTreeMap<PathBuf, Vec<String>>,
    report: Report,
}

fn frame(node: &XmlNode) -> Result<Affine3A> {
    let pose = read_pose(&node.attrs)?;
    Ok(Affine3A::from_scale_rotation_translation(
        pose.scale,
        pose.rotation,
        pose.translation,
    ))
}

fn terrain_node(nodes: &[XmlNode]) -> Option<&XmlNode> {
    nodes.iter().find_map(|node| {
        if node.tag.eq_ignore_ascii_case("terrain") {
            Some(node)
        } else {
            terrain_node(&node.children)
        }
    })
}

fn in_field(terrain: &AhgtHeightField, p: Vec3) -> bool {
    p.is_finite() && p.x.abs().max(p.z.abs()) <= terrain.world_size() * 0.5
}

fn is_model(node: &XmlNode) -> bool {
    matches!(
        node.tag.to_ascii_lowercase().as_str(),
        "gltfscene" | "gltfloader"
    )
}

fn protected(node: &XmlNode, url: Option<&str>) -> bool {
    if matches!(node.attr("settle"), Some("keep" | "none")) {
        return true;
    }
    let Some(url) = url else { return false };
    let name = node.attr("name").unwrap_or_default().to_lowercase();
    let asset = url.rsplit('/').next().unwrap_or_default().to_lowercase();
    // Pieces whose support is another object, a fitted landform, or a waterline.
    ["/infra/", "/concordia/", "/terrain/", "/interiors/"]
        .iter()
        .any(|part| url.contains(part))
        || [
            "bridge", "ponte", "jetty", "deck", "wreck", "lantern", "roof", "lily_pad",
        ]
        .iter()
        .any(|word| asset.contains(word))
        || ["ponte", "cais", "baliza", "doca", "telhado"]
            .iter()
            .any(|word| name.starts_with(word))
        || name.ends_with("-crate-top")
}

impl Measure<'_> {
    fn visit(&mut self, nodes: &[XmlNode], parent: Affine3A) -> Result<()> {
        for node in nodes {
            if matches!(node.attr("settle"), Some("keep" | "none")) {
                continue;
            }
            let tag = node.tag.to_ascii_lowercase();
            if matches!(
                tag.as_str(),
                "prototype"
                    | "scatter"
                    | "dynamicspawner"
                    | "use"
                    | "cave"
                    | "bridge"
                    | "arc"
                    | "arch"
            ) {
                continue;
            }
            if tag == "vegetation"
                && let Some(file) = node.attr("instances")
            {
                self.batches.insert(
                    self.root.join(file.trim_start_matches('/')),
                    node.attr("meshes")
                        .unwrap_or_default()
                        .split_whitespace()
                        .map(str::to_owned)
                        .collect(),
                );
            }
            let local = frame(node)?;
            let world = parent * local;
            if node.attr("prop-flora").is_some() {
                if let Some(id) = self.measurement_id(node, parent, world.translation.into()) {
                    let foot = Vec3::from(world.translation);
                    let delta = self.terrain.sample(foot.x, foot.z) - foot.y;
                    self.seat(id, node, parent, local, foot, delta);
                }
                continue;
            }
            let child = if matches!(
                tag.as_str(),
                "entity" | "gameobject" | "npcbody" | "resourcenode"
            ) {
                let mut models = node.children.iter().filter(|child| is_model(child));
                models.next().filter(|_| models.next().is_none())
            } else {
                None
            };
            if is_model(node) || child.is_some() {
                let asset = child.unwrap_or(node);
                let url = asset.attr("url");
                if !protected(node, url) && !protected(asset, url) {
                    let asset_world = if child.is_some() {
                        world * frame(asset)?
                    } else {
                        world
                    };
                    self.measure(node, url, parent, local, asset_world);
                }
                // Translate the assembly once, including collider, effects and model.
                continue;
            }
            self.visit(&node.children, world)?;
        }
        Ok(())
    }

    fn measure(
        &mut self,
        node: &XmlNode,
        url: Option<&str>,
        parent: Affine3A,
        local: Affine3A,
        asset: Affine3A,
    ) {
        let Some(url) = url.filter(|url| url.to_ascii_lowercase().ends_with(".glb")) else {
            return;
        };
        let Some(id) = self.measurement_id(node, parent, asset.translation.into()) else {
            return;
        };
        let bounds = self.bounds.entry(url.to_owned()).or_insert_with(|| {
            self.asset_roots
                .iter()
                .find_map(|root| read_json_chunk(&root.join(url.trim_start_matches('/'))))
        });
        let Some(bounds) = *bounds else {
            self.report.skip("glb_ausente_ou_sem_limites");
            return;
        };
        let foot = asset.transform_point3(bounds.center().with_y(bounds.min.y));
        let point = |u: f32, v: f32| {
            asset.transform_point3(Vec3::new(
                bounds.min.x + (bounds.max.x - bounds.min.x) * u,
                bounds.min.y,
                bounds.min.z + (bounds.max.z - bounds.min.z) * v,
            ))
        };
        let building = url.contains("/village/") && bounds.footprint_radius() >= 2.0
            || url.contains("/cenario/abrigo-do-vale");
        let mut clearances = [0.0f32; 9];
        for (index, clearance) in clearances.iter_mut().enumerate() {
            let p = point((index % 3) as f32 * 0.5, (index / 3) as f32 * 0.5);
            if !in_field(self.terrain, p) {
                self.report.skip("pegada_fora_do_terreno");
                return;
            }
            *clearance = self.terrain.sample(p.x, p.z) - p.y;
        }
        clearances.sort_by(f32::total_cmp);
        let delta = if building {
            clearances[0]
        } else {
            clearances[4]
        };
        // Uma encosta funda tem de receber fundação própria.
        if building && clearances[8] - clearances[0] > 2.5 {
            self.report.skip("fundacao_em_declive");
            return;
        }
        self.seat(id, node, parent, local, foot, delta);
    }

    fn measurement_id(&mut self, node: &XmlNode, parent: Affine3A, origin: Vec3) -> Option<u64> {
        let Some(id) = node.origin.as_deref().and_then(scene_node_id_for) else {
            if self.scoped_ids.is_none() {
                self.report.skip("sem_scene_node_id");
            }
            return None;
        };
        if self
            .scoped_ids
            .as_ref()
            .is_some_and(|ids| !ids.contains(&id))
        {
            return None;
        }
        if !in_field(self.terrain, origin) || parent.matrix3.determinant().abs() < 1e-8 {
            self.report.skip("fora_do_terreno_ou_transform_invalido");
            return None;
        }
        Some(id)
    }

    fn seat(
        &mut self,
        id: u64,
        node: &XmlNode,
        parent: Affine3A,
        local: Affine3A,
        foot: Vec3,
        delta: f32,
    ) {
        let before = self
            .reference
            .map(|reference| reference.sample(foot.x, foot.z) - foot.y);
        let known_support = before.is_some_and(|value| value.abs() <= self.limit);
        if !delta.is_finite() || delta.abs() > self.limit && !known_support {
            self.report.skip("desvio_alto_requer_revisao");
            return;
        }
        let translation =
            Vec3::from(local.translation) + parent.inverse().transform_vector3(Vec3::Y * delta);
        self.record(
            id,
            TargetPose {
                name: node.attr("name").unwrap_or(&node.tag).to_owned(),
                translation,
                delta,
                position: foot.xz(),
            },
        );
    }

    fn record(&mut self, id: u64, target: TargetPose) {
        use std::collections::hash_map::Entry;
        match self.targets.entry(id) {
            Entry::Vacant(entry) => {
                entry.insert(Some(target));
            }
            Entry::Occupied(mut entry) => {
                // The same inherited ID at two poses has no unique physical edit.
                if !entry
                    .get()
                    .as_ref()
                    .is_some_and(|old| old.translation.abs_diff_eq(target.translation, 0.02))
                {
                    entry.insert(None);
                    self.report.skip("instancias_com_transform_compartilhado");
                }
            }
        }
    }
}

pub fn run(options: Options) -> Result<()> {
    ensure!(
        options.limit.is_finite() && options.limit > 0.0,
        "--limit deve ser positivo"
    );
    let world = options.world.canonicalize()?;
    let root = vael_tools::analyze::project_root_of(&world);
    let loaded = load_bsn_world(&world)?;
    let terrain_spec = terrain_node(&loaded.nodes).context("o mundo não declara terreno")?;
    let terrain_path = match &options.terrain {
        Some(path) => path.clone(),
        None => root.join(
            terrain_spec
                .attr("heightmap")
                .context("o terreno não declara um AHGT")?
                .trim_start_matches('/'),
        ),
    };
    let terrain = AhgtHeightField::from_path_with(
        &terrain_path,
        terrain_spec
            .attr("world-size")
            .map(str::parse)
            .transpose()?,
        terrain_spec
            .attr("max-height")
            .map(str::parse)
            .transpose()?,
    )?;
    let reference = options
        .reference
        .as_deref()
        .map(AhgtHeightField::from_path)
        .transpose()?;
    let paths = |file: &Path| {
        let rel = file.strip_prefix(&root).unwrap_or(file);
        !rel.starts_with("world/prefabs")
            && (options.scope.is_empty()
                || options.scope.iter().any(|scope| rel.starts_with(scope)))
    };
    let mut files = loaded.sources;
    files.push(world);
    for file in &mut files {
        *file = file.canonicalize()?;
    }
    files.sort_unstable();
    files.dedup();
    let documents = files
        .into_iter()
        .filter(|file| paths(file) && file.extension().is_some_and(|ext| ext == "bsn"))
        .map(|file| {
            let text = fs::read_to_string(&file)?;
            let nodes = document::index(&text)?;
            Ok((file, text, nodes))
        })
        .collect::<Result<Vec<_>>>()?;
    let scoped_ids = (!options.scope.is_empty()).then(|| {
        documents
            .iter()
            .flat_map(|(_, _, nodes)| nodes.iter())
            .filter_map(|node| node.id.as_ref()?.parse().ok())
            .collect()
    });
    let mut measure = Measure {
        root: &root,
        asset_roots: std::iter::once(root.join("assets/public"))
            .chain(options.asset_root)
            .collect(),
        terrain: &terrain,
        reference: reference.as_ref(),
        limit: options.limit,
        scoped_ids,
        bounds: HashMap::new(),
        targets: HashMap::new(),
        batches: BTreeMap::new(),
        report: Report::default(),
    };
    measure.visit(&loaded.nodes, Affine3A::IDENTITY)?;
    let mut found = HashSet::new();
    let mut writes = Vec::<(PathBuf, String, Vec<(Range<usize>, String)>)>::new();
    for (file, text, nodes) in documents {
        let mut changes = Vec::new();
        for node in nodes {
            let Some(id) = node.id.as_ref().and_then(|id| id.parse::<u64>().ok()) else {
                continue;
            };
            let Some(Some(target)) = measure.targets.get(&id) else {
                continue;
            };
            if target.delta.abs() < 0.035 || !found.insert(id) {
                continue;
            }
            let Ok(span) = document::field(&text, &node, "Transform.translation") else {
                measure
                    .report
                    .skip("sem_translation_local_editaria_o_prefab");
                continue;
            };
            let p = target.translation;
            changes.push((
                span,
                format!(
                    "glam::Vec3 {{ x: {:.6}, y: {:.6}, z: {:.6} }}",
                    p.x, p.y, p.z
                ),
            ));
            measure.report.objects.push(Move {
                file: file.strip_prefix(&root).unwrap_or(&file).to_owned(),
                id: id.to_string(),
                name: target.name.clone(),
                delta_m: target.delta,
                world_xz: target.position.to_array(),
            });
        }
        if !changes.is_empty() {
            writes.push((file, text, changes));
        }
    }
    let mut batches = Vec::new();
    for (file, meshes) in measure.batches.into_iter().filter(|(file, _)| paths(file)) {
        let bounds: Vec<_> = meshes
            .into_iter()
            .map(|url| {
                *measure.bounds.entry(url.clone()).or_insert_with(|| {
                    measure
                        .asset_roots
                        .iter()
                        .find_map(|root| read_json_chunk(&root.join(url.trim_start_matches('/'))))
                })
            })
            .collect();
        let mut instances = vinst::read(&file)?;
        let mut moved = 0;
        for instance in &mut instances {
            let p = instance.position;
            if !in_field(&terrain, p) {
                continue;
            }
            let Some(model) = bounds.get(instance.template_index).and_then(|bound| *bound) else {
                measure.report.skip("vegetacao_sem_limites");
                continue;
            };
            let base = instance.rotation * (instance.scale * model.center().with_y(model.min.y));
            let p = p + base;
            let target_y = terrain.sample(p.x, p.z) - base.y - 0.05;
            let known_support = reference.as_ref().is_some_and(|old| {
                in_field(old, p) && (old.sample(p.x, p.z) - p.y).abs() <= options.limit
            });
            let delta = target_y - instance.position.y;
            if delta.abs() >= 0.035 && (delta.abs() <= options.limit || known_support) {
                instance.position.y += delta;
                moved += 1;
            }
        }
        if moved > 0 {
            measure
                .report
                .vegetation
                .insert(file.strip_prefix(&root).unwrap_or(&file).to_owned(), moved);
            if options.apply {
                batches.push((file, instances));
            }
        }
    }
    if options.apply {
        for (file, original, mut changes) in writes {
            changes.sort_unstable_by_key(|(span, _)| std::cmp::Reverse(span.start));
            super::update(
                Target {
                    file,
                    dry_run: false,
                },
                |text| {
                    ensure!(*text == original, "o documento mudou desde a prévia");
                    for (span, value) in changes {
                        text.replace_range(span, &value);
                    }
                    Ok(())
                },
            )?;
        }
        for (file, instances) in batches {
            vinst::write_atomic(&file, &instances)?;
        }
    }
    if let Some(report) = options.report {
        fs::write(report, serde_json::to_vec_pretty(&measure.report)?)?;
    }
    println!(
        "{} {} objetos e {} instâncias de vegetação; {} por rever",
        if options.apply {
            "Assentados"
        } else {
            "Prévia para"
        },
        measure.report.objects.len(),
        measure.report.vegetation.values().sum::<usize>(),
        measure.report.skipped.values().sum::<usize>(),
    );
    Ok(())
}

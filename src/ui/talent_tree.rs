//! Layout e view-models da árvore de talentos.
//!
//! O grafo já existe no conteúdo (`requires`/`excludes`); este módulo deriva
//! a geometria uma vez no arranque e a UI (`ui/menu.xaml`) só posiciona
//! `Canvas`/`Line` com os valores prontos. Não há coordenadas autorais no
//! JSON: acrescentar um talento novo no `data/progression.json` continua a
//! bastar.
//!
//! A árvore tem três tiers (raiz, meio, folha) e dez lanes com os 19 talentos
//! atuais. Os pais ficam centrados sobre os filhos; os pares de exclusão
//! (ex.: `bulwark`/`bloodthirst`) são normalizados para uma única aresta
//! horizontal, desenhada atrás dos nós.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use bevy::prelude::*;

use crate::content::{ProgressionFile, SkillDef};

/// Tamanho do botão/ícone de um nó, em unidades de UI.
pub const NODE_SIZE: f32 = 64.0;
/// Distância horizontal entre lanes.
pub const LANE_WIDTH: f32 = 78.0;
/// Distância vertical entre tiers.
pub const TIER_HEIGHT: f32 = 132.0;
/// Margem à volta do canvas.
pub const MARGIN: f32 = 24.0;

/// Um nó já posicionado no canvas lógico.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TalentLayoutNode {
    pub id: String,
    pub label: String,
    pub short_label: String,
    pub tier: u32,
    pub lane: u32,
    pub x: f64,
    pub y: f64,
    pub requires: Option<String>,
    pub excludes: Option<String>,
}

/// Uma aresta pronta a desenhar (`x/y` é o canto superior esquerdo do
/// contentor; `x1/y1 → x2/y2` são coordenadas locais, nunca negativas).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TalentLayoutEdge {
    pub x: f64,
    pub y: f64,
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
}

/// Layout imutável da árvore; construído no `Startup` a partir do conteúdo.
#[derive(Resource, Debug, Clone, Default)]
pub struct VaelTalentLayout {
    pub nodes: Vec<TalentLayoutNode>,
    /// Arestas pai→filho.
    pub requires_edges: Vec<TalentLayoutEdge>,
    /// Arestas entre pares de exclusão (uma por par).
    pub exclusion_edges: Vec<TalentLayoutEdge>,
    pub canvas_width: f64,
    pub canvas_height: f64,
    /// Raízes na ordem do JSON (para a seleção inicial).
    pub roots: Vec<String>,
    /// Ids com `talent_<id>.png` presente no pool (descoberto no arranque).
    pub icons: BTreeSet<String>,
}

impl VaelTalentLayout {
    /// Deriva tiers/lanes/arestas de `requires`/`excludes`.
    pub fn build(progression: &ProgressionFile) -> Self {
        let by_id: BTreeMap<&str, &SkillDef> = progression
            .skills
            .iter()
            .map(|def| (def.id.as_str(), def))
            .collect();

        let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut roots: Vec<String> = Vec::new();
        for def in &progression.skills {
            let parent = def
                .requires
                .as_deref()
                .filter(|parent| !parent.is_empty() && by_id.contains_key(parent));
            match parent {
                Some(parent) => children
                    .entry(parent.to_string())
                    .or_default()
                    .push(def.id.clone()),
                None => roots.push(def.id.clone()),
            }
        }

        let mut lanes: BTreeMap<String, f64> = BTreeMap::new();
        let mut tiers: BTreeMap<String, u32> = BTreeMap::new();
        let mut next_lane = 0.0f64;
        let mut visiting = BTreeSet::new();
        for root in &roots {
            visit(
                root,
                0,
                &children,
                &mut lanes,
                &mut tiers,
                &mut next_lane,
                &mut visiting,
            );
        }
        // Conteúdo com ciclos/ids órfãos: dá-lhes uma lane para a árvore não
        // desaparecer; o validador de conteúdo já reporta o erro.
        for def in &progression.skills {
            if !lanes.contains_key(&def.id) {
                lanes.insert(def.id.clone(), next_lane);
                tiers.entry(def.id.clone()).or_insert(0);
                next_lane += 1.0;
            }
        }

        let max_tier = tiers.values().copied().max().unwrap_or(0);
        let lanes_count = next_lane.max(1.0);
        let canvas_width =
            f64::from(MARGIN * 2.0 + (lanes_count as f32 - 1.0) * LANE_WIDTH + NODE_SIZE);
        let canvas_height = f64::from(MARGIN * 2.0 + max_tier as f32 * TIER_HEIGHT + NODE_SIZE);

        let mut nodes = Vec::with_capacity(progression.skills.len());
        for def in &progression.skills {
            let lane = lanes.get(&def.id).copied().unwrap_or(0.0);
            let tier = tiers.get(&def.id).copied().unwrap_or(0);
            nodes.push(TalentLayoutNode {
                id: def.id.clone(),
                label: def.label.clone(),
                short_label: short_label(&def.label),
                tier,
                lane: lane as u32,
                x: f64::from(MARGIN + lane as f32 * LANE_WIDTH),
                y: f64::from(MARGIN + tier as f32 * TIER_HEIGHT),
                requires: def
                    .requires
                    .clone()
                    .filter(|parent| !parent.is_empty() && by_id.contains_key(parent.as_str())),
                excludes: def.excludes.clone().filter(|other| !other.is_empty()),
            });
        }

        let positions: BTreeMap<&str, &TalentLayoutNode> =
            nodes.iter().map(|node| (node.id.as_str(), node)).collect();

        let mut requires_edges = Vec::new();
        for node in &nodes {
            let Some(parent_id) = node.requires.as_deref() else {
                continue;
            };
            let Some(parent) = positions.get(parent_id) else {
                continue;
            };
            let parent_cx = parent.x + f64::from(NODE_SIZE / 2.0);
            let parent_bottom = parent.y + f64::from(NODE_SIZE);
            let child_cx = node.x + f64::from(NODE_SIZE / 2.0);
            let left = parent_cx.min(child_cx);
            requires_edges.push(TalentLayoutEdge {
                x: left,
                y: parent_bottom,
                x1: parent_cx - left,
                y1: 0.0,
                x2: child_cx - left,
                y2: (node.y - parent_bottom).max(1.0),
            });
        }

        let mut exclusion_edges = Vec::new();
        let mut seen = BTreeSet::new();
        for node in &nodes {
            let Some(other_id) = node.excludes.as_deref() else {
                continue;
            };
            let Some(other) = positions.get(other_id) else {
                continue;
            };
            // O JSON declara os dois lados; normalizar para um par único.
            let pair = if node.id < other.id {
                (node.id.as_str(), other.id.as_str())
            } else {
                (other.id.as_str(), node.id.as_str())
            };
            if !seen.insert(pair) {
                continue;
            }
            exclusion_edges.push(center_edge(node, other));
        }

        Self {
            nodes,
            requires_edges,
            exclusion_edges,
            canvas_width,
            canvas_height,
            roots,
            icons: BTreeSet::new(),
        }
    }

    /// Nó por id.
    pub fn node(&self, id: &str) -> Option<&TalentLayoutNode> {
        self.nodes.iter().find(|node| node.id == id)
    }
}

/// Regista filho→pai e atribui lanes: folhas recebem lanes consecutivas; um
/// pai fica na média do primeiro e do último filho.
fn visit(
    id: &str,
    depth: u32,
    children: &BTreeMap<String, Vec<String>>,
    lanes: &mut BTreeMap<String, f64>,
    tiers: &mut BTreeMap<String, u32>,
    next_lane: &mut f64,
    visiting: &mut BTreeSet<String>,
) -> f64 {
    if let Some(lane) = lanes.get(id) {
        return *lane;
    }
    if !visiting.insert(id.to_string()) {
        // Ciclo: não desce outra vez; o nó fica com uma lane no fim.
        return *next_lane;
    }
    tiers.insert(id.to_string(), depth);
    let kids = children.get(id).cloned().unwrap_or_default();
    let lane = if kids.is_empty() {
        let lane = *next_lane;
        *next_lane += 1.0;
        lane
    } else {
        let mut first = f64::NAN;
        let mut last = f64::NAN;
        for child in &kids {
            let child_lane = visit(
                child,
                depth + 1,
                children,
                lanes,
                tiers,
                next_lane,
                visiting,
            );
            if first.is_nan() {
                first = child_lane;
            }
            last = child_lane;
        }
        (first + last) / 2.0
    };
    visiting.remove(id);
    lanes.insert(id.to_string(), lane);
    lane
}

/// Aresta genérica entre os centros de dois nós; a parte escondida atrás dos
/// botões não se vê, e a que fica no intervalo lê-se como uma ligação.
fn center_edge(a: &TalentLayoutNode, b: &TalentLayoutNode) -> TalentLayoutEdge {
    let a_cx = a.x + f64::from(NODE_SIZE / 2.0);
    let a_cy = a.y + f64::from(NODE_SIZE / 2.0);
    let b_cx = b.x + f64::from(NODE_SIZE / 2.0);
    let b_cy = b.y + f64::from(NODE_SIZE / 2.0);
    let left = a_cx.min(b_cx);
    let top = a_cy.min(b_cy);
    TalentLayoutEdge {
        x: left,
        y: top,
        x1: a_cx - left,
        y1: a_cy - top,
        x2: b_cx - left,
        y2: b_cy - top,
    }
}

/// Nome curto para o nó: corta o resumo entre parênteses.
pub fn short_label(label: &str) -> String {
    label.split(" (").next().unwrap_or(label).trim().to_string()
}

/// Estado visual de um talento.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TalentState {
    Learned,
    Ready,
    Locked,
    Excluded,
}

impl TalentState {
    pub fn of(node: &TalentLayoutNode, learned: &BTreeSet<String>) -> Self {
        if learned.contains(&node.id) {
            Self::Learned
        } else if node
            .excludes
            .as_deref()
            .is_some_and(|other| learned.contains(other))
        {
            Self::Excluded
        } else if node
            .requires
            .as_deref()
            .is_none_or(|parent| learned.contains(parent))
        {
            Self::Ready
        } else {
            Self::Locked
        }
    }

    pub fn text(self, lang: &str) -> &'static str {
        match (self, lang == "en") {
            (Self::Learned, false) => "aprendida",
            (Self::Learned, true) => "learned",
            (Self::Ready, false) => "disponível",
            (Self::Ready, true) => "available",
            (Self::Locked, false) => "bloqueada",
            (Self::Locked, true) => "locked",
            (Self::Excluded, false) => "ramo fechado",
            (Self::Excluded, true) => "branch closed",
        }
    }
}

/// Nó do view-model. Os booleanos existem para os `DataTrigger` do bevy_pf
/// poderem pintar o estado sem um converter.
#[derive(Reflect, Default, Clone, Debug, PartialEq)]
pub struct TalentNodeVm {
    pub id: String,
    pub action: String,
    pub label: String,
    pub short_label: String,
    pub icon: String,
    pub has_icon: bool,
    pub x: f64,
    pub y: f64,
    pub size: f64,
    pub learned: bool,
    pub ready: bool,
    pub locked: bool,
    pub excluded: bool,
    pub selected: bool,
}

#[derive(Reflect, Default, Clone, Debug, PartialEq)]
pub struct TalentEdgeVm {
    pub x: f64,
    pub y: f64,
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
}

impl From<&TalentLayoutEdge> for TalentEdgeVm {
    fn from(edge: &TalentLayoutEdge) -> Self {
        Self {
            x: edge.x,
            y: edge.y,
            x1: edge.x1,
            y1: edge.y1,
            x2: edge.x2,
            y2: edge.y2,
        }
    }
}

/// Constrói os nós do VM com o estado actual.
pub fn talent_nodes(
    layout: &VaelTalentLayout,
    learned: &BTreeSet<String>,
    selected: Option<&str>,
) -> Vec<TalentNodeVm> {
    layout
        .nodes
        .iter()
        .map(|node| {
            let state = TalentState::of(node, learned);
            let has_icon = layout.icons.contains(&node.id);
            TalentNodeVm {
                id: node.id.clone(),
                action: format!("talent-{}", node.id),
                label: node.label.clone(),
                short_label: node.short_label.clone(),
                icon: if has_icon {
                    talent_icon(&node.id)
                } else {
                    String::new()
                },
                has_icon,
                x: node.x,
                y: node.y,
                size: f64::from(NODE_SIZE),
                learned: state == TalentState::Learned,
                ready: state == TalentState::Ready,
                locked: state == TalentState::Locked,
                excluded: state == TalentState::Excluded,
                selected: selected == Some(node.id.as_str()),
            }
        })
        .collect()
}

/// Caminho do ícone no asset root (`config.yaml` aponta `assets/public`).
pub fn talent_icon(id: &str) -> String {
    format!("assets/icons/talent_{id}.png")
}

/// Descobre os `talent_<id>.png` já gerados no pool. Sem o ficheiro, o nó
/// mostra só o rótulo em vez de pedir ao `AssetServer` uma imagem inexistente.
pub fn discover_icons(root: &Path) -> BTreeSet<String> {
    let dir = root.join("assets/public/assets/icons");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return BTreeSet::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let id = name.strip_prefix("talent_")?.strip_suffix(".png")?;
            Some(id.to_string())
        })
        .collect()
}

/// Resumo legível dos `effects` para o painel de detalhe.
pub fn effects_summary(def: &SkillDef, lang: &str) -> String {
    let en = lang == "en";
    let mut parts = Vec::new();
    for (key, value) in &def.effects {
        let text = match key.as_str() {
            "strength" => format!("{} {}", if en { "STR" } else { "FOR" }, signed(*value)),
            "agility" => format!("{} {}", if en { "AGI" } else { "DES" }, signed(*value)),
            "vitality" => format!("{} {}", if en { "VIT" } else { "VIT" }, signed(*value)),
            "precision" => format!("{} {}", if en { "PRE" } else { "PRE" }, signed(*value)),
            "spirit" => format!("{} {}", if en { "SPI" } else { "ESP" }, signed(*value)),
            "damage" => format!("{} {}", if en { "damage" } else { "dano" }, signed(*value)),
            "max_hp" => format!("{} {}", if en { "health" } else { "vida" }, signed(*value)),
            "speed" => format!(
                "{} {}%",
                if en { "speed" } else { "velocidade" },
                signed(value * 100.0)
            ),
            "crit" => format!(
                "{} {}%",
                if en { "crit" } else { "crítico" },
                signed(value * 100.0)
            ),
            "defense" => format!(
                "{} {}%",
                if en { "defence" } else { "defesa" },
                signed(value * 100.0)
            ),
            "lifesteal" => format!(
                "{} {}%",
                if en { "life steal" } else { "roubo de vida" },
                signed(value * 100.0)
            ),
            "execute" => format!(
                "{} {}%",
                if en { "execute" } else { "execução" },
                signed(value * 100.0)
            ),
            "crit_mult" => format!(
                "{} {}",
                if en {
                    "crit multiplier"
                } else {
                    "multiplicador de crítico"
                },
                signed(*value)
            ),
            "dash_cd" => format!(
                "{} {}%",
                if en {
                    "dash cooldown"
                } else {
                    "recarga da esquiva"
                },
                signed(value * 100.0)
            ),
            "flasks" => format!(
                "{} {}",
                if en {
                    "flask charges"
                } else {
                    "cargas do cantil"
                },
                signed(*value)
            ),
            "strike_burn" => format!(
                "{} {} s",
                if en {
                    "impact burn"
                } else {
                    "queimadura do Impacto"
                },
                signed(*value)
            ),
            "strike_radius" => format!(
                "{} {} m",
                if en {
                    "impact radius"
                } else {
                    "raio do Impacto"
                },
                signed(*value)
            ),
            "strike_damage" => format!(
                "{} {}",
                if en {
                    "impact damage"
                } else {
                    "dano do Impacto"
                },
                signed(*value)
            ),
            "shock_chance" => format!(
                "{} {}%",
                if en {
                    "shock chance"
                } else {
                    "chance de choque"
                },
                signed(value * 100.0)
            ),
            "dash_iframes" => format!(
                "{} {} s",
                if en {
                    "dash i-frames"
                } else {
                    "i-frames da esquiva"
                },
                signed(*value)
            ),
            _ => format!("{key} {}", signed(*value)),
        };
        parts.push(text);
    }
    parts.join(" · ")
}

/// "Requer: X" / "Sem requisitos", com a exclusão quando existe.
pub fn requirement_text(def: &SkillDef, progression: &ProgressionFile, lang: &str) -> String {
    let en = lang == "en";
    let label_of = |id: &str| {
        progression
            .skills
            .iter()
            .find(|skill| skill.id == id)
            .map(|skill| skill.label.clone())
            .unwrap_or_else(|| id.to_string())
    };
    let requires = def.requires.as_deref().unwrap_or("");
    let mut text = if requires.is_empty() {
        if en {
            "No requirements".to_string()
        } else {
            "Sem requisitos".to_string()
        }
    } else {
        format!(
            "{}: {}",
            if en { "Requires" } else { "Requer" },
            label_of(requires)
        )
    };
    if let Some(excludes) = def.excludes.as_deref().filter(|other| !other.is_empty()) {
        text.push_str(&format!(
            " · {} {}",
            if en { "excludes" } else { "exclui" },
            label_of(excludes)
        ));
    }
    text
}

/// Dados do painel lateral para o nó selecionado.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TalentDetail {
    pub visible: bool,
    pub icon: String,
    pub has_icon: bool,
    pub title: String,
    pub effects: String,
    pub requirement: String,
    pub status: String,
    pub learn_label: String,
    pub learn_enabled: bool,
}

/// Preenche o painel de detalhe; `def = None` dá o placeholder.
pub fn talent_detail(
    def: Option<&SkillDef>,
    progression: &ProgressionFile,
    learned: &BTreeSet<String>,
    points: i64,
    lang: &str,
) -> TalentDetail {
    let en = lang == "en";
    let Some(def) = def else {
        return TalentDetail {
            visible: false,
            title: if en {
                "Select a talent".to_string()
            } else {
                "Seleciona um talento".to_string()
            },
            learn_label: if en {
                "Learn".to_string()
            } else {
                "Aprender".to_string()
            },
            ..Default::default()
        };
    };
    let node = TalentLayoutNode {
        id: def.id.clone(),
        label: def.label.clone(),
        short_label: short_label(&def.label),
        requires: def.requires.clone().filter(|parent| !parent.is_empty()),
        excludes: def.excludes.clone().filter(|other| !other.is_empty()),
        ..Default::default()
    };
    let state = TalentState::of(&node, learned);
    let learn_enabled = state == TalentState::Ready && points > 0;
    let learn_label = match state {
        TalentState::Learned => {
            if en {
                "Learned".to_string()
            } else {
                "Aprendida".to_string()
            }
        }
        TalentState::Excluded => {
            if en {
                "Branch closed".to_string()
            } else {
                "Ramo fechado".to_string()
            }
        }
        TalentState::Locked => {
            if en {
                "Locked".to_string()
            } else {
                "Bloqueada".to_string()
            }
        }
        TalentState::Ready if points < 1 => {
            if en {
                "No points".to_string()
            } else {
                "Sem pontos".to_string()
            }
        }
        TalentState::Ready => {
            if en {
                "Learn (1 point)".to_string()
            } else {
                "Aprender (1 ponto)".to_string()
            }
        }
    };
    TalentDetail {
        visible: true,
        icon: talent_icon(&def.id),
        has_icon: true,
        title: def.label.clone(),
        effects: effects_summary(def, lang),
        requirement: requirement_text(def, progression, lang),
        status: state.text(lang).to_string(),
        learn_label,
        learn_enabled,
    }
}

fn signed(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:+.0}")
    } else {
        format!("{value:+.1}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(id: &str, requires: Option<&str>, excludes: Option<&str>) -> SkillDef {
        SkillDef {
            id: id.to_string(),
            label: format!("{id} (teste)"),
            requires: requires.map(str::to_string),
            excludes: excludes.map(str::to_string),
            ..Default::default()
        }
    }

    fn progression() -> ProgressionFile {
        ProgressionFile {
            skills: vec![
                skill("vitality1", None, None),
                skill("strength1", None, None),
                skill("agility1", None, None),
                skill("precision1", None, None),
                skill("vitality2", Some("vitality1"), None),
                skill("strength2", Some("strength1"), None),
                skill("agility2", Some("agility1"), None),
                skill("precision2", Some("precision1"), None),
                skill("spirit1", Some("vitality1"), None),
                skill("spirit2", Some("spirit1"), None),
                skill("flask3", Some("vitality1"), None),
                skill("bulwark", Some("vitality2"), Some("bloodthirst")),
                skill("bloodthirst", Some("vitality2"), Some("bulwark")),
                skill("embers", Some("strength2"), Some("tremor")),
                skill("tremor", Some("strength2"), Some("embers")),
                skill("windstep", Some("agility2"), Some("ghost")),
                skill("ghost", Some("agility2"), Some("windstep")),
                skill("executioner", Some("precision2"), Some("duelist")),
                skill("duelist", Some("precision2"), Some("executioner")),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn layout_cobre_os_19_talentos() {
        let layout = VaelTalentLayout::build(&progression());
        assert_eq!(layout.nodes.len(), 19);
        assert_eq!(layout.requires_edges.len(), 15);
        assert_eq!(layout.exclusion_edges.len(), 4);
        assert_eq!(layout.roots.len(), 4);
        for root in ["vitality1", "strength1", "agility1", "precision1"] {
            assert!(layout.roots.contains(&root.to_string()));
            assert_eq!(layout.node(root).expect("raiz").tier, 0);
        }
    }

    #[test]
    fn layout_tem_tres_tiers_e_sem_posicoes_repetidas() {
        let layout = VaelTalentLayout::build(&progression());
        let max_tier = layout.nodes.iter().map(|node| node.tier).max().unwrap();
        assert_eq!(max_tier, 2);
        let mut positions = BTreeSet::new();
        for node in &layout.nodes {
            assert!(
                positions.insert((node.x as i64, node.y as i64)),
                "posição repetida em {}",
                node.id
            );
            assert!(node.x >= 0.0 && node.y >= 0.0);
            assert!(node.x + f64::from(NODE_SIZE) <= layout.canvas_width);
            assert!(node.y + f64::from(NODE_SIZE) <= layout.canvas_height);
        }
    }

    #[test]
    fn layout_poe_os_filhos_abaixo_dos_pais() {
        let layout = VaelTalentLayout::build(&progression());
        for node in &layout.nodes {
            let Some(parent) = node.requires.as_deref() else {
                continue;
            };
            let parent = layout.node(parent).expect("pai");
            assert!(node.y > parent.y, "{} não está abaixo do pai", node.id);
        }
    }

    #[test]
    fn exclusoes_sao_um_par_unico_por_ramo() {
        let layout = VaelTalentLayout::build(&progression());
        let pairs: BTreeSet<(String, String)> = layout
            .nodes
            .iter()
            .filter_map(|node| {
                let other = node.excludes.as_deref()?;
                Some(if node.id < other.to_string() {
                    (node.id.clone(), other.to_string())
                } else {
                    (other.to_string(), node.id.clone())
                })
            })
            .collect();
        assert_eq!(pairs.len(), 4);
        assert_eq!(layout.exclusion_edges.len(), 4);
    }

    #[test]
    fn estados_seguem_a_prioridade_da_lista() {
        let layout = VaelTalentLayout::build(&progression());
        let learned = BTreeSet::from([
            "vitality1".to_string(),
            "vitality2".to_string(),
            "bulwark".to_string(),
        ]);
        let state = |id: &str| TalentState::of(layout.node(id).expect("nó"), &learned);
        assert_eq!(state("vitality1"), TalentState::Learned);
        assert_eq!(state("vitality2"), TalentState::Learned);
        assert_eq!(state("spirit1"), TalentState::Ready);
        assert_eq!(state("spirit2"), TalentState::Locked);
        assert_eq!(state("bulwark"), TalentState::Learned);
        assert_eq!(state("bloodthirst"), TalentState::Excluded);
    }

    #[test]
    fn vm_marca_estado_e_icone() {
        let mut layout = VaelTalentLayout::build(&progression());
        layout.icons.insert("strength2".to_string());
        let learned = BTreeSet::from(["strength1".to_string()]);
        let nodes = talent_nodes(&layout, &learned, Some("strength2"));
        let strength1 = nodes.iter().find(|node| node.id == "strength1").unwrap();
        let strength2 = nodes.iter().find(|node| node.id == "strength2").unwrap();
        assert!(strength1.learned);
        assert!(strength2.ready);
        assert!(strength2.selected);
        assert_eq!(strength2.icon, "assets/icons/talent_strength2.png");
        assert!(strength2.has_icon);
    }

    #[test]
    fn detalhe_mostra_efeito_e_requisito() {
        let mut def = skill("strength1", None, None);
        def.effects.insert("strength".to_string(), 2.0);
        def.label = "Força I (+2 Força)".to_string();
        let progression = progression();
        let detail = talent_detail(Some(&def), &progression, &BTreeSet::new(), 1, "pt");
        assert!(detail.visible);
        assert!(detail.learn_enabled);
        assert_eq!(detail.title, "Força I (+2 Força)");
        assert_eq!(detail.effects, "FOR +2");
        assert_eq!(detail.requirement, "Sem requisitos");
        assert_eq!(detail.status, "disponível");
        assert_eq!(detail.learn_label, "Aprender (1 ponto)");
    }

    #[test]
    fn detalhe_bloqueia_sem_pontos_e_ramo_fechado() {
        let progression = progression();
        let bulwark = progression
            .skills
            .iter()
            .find(|skill| skill.id == "bulwark")
            .unwrap();
        let learned = BTreeSet::from([
            "vitality1".to_string(),
            "vitality2".to_string(),
            "bloodthirst".to_string(),
        ]);
        let detail = talent_detail(Some(bulwark), &progression, &learned, 1, "pt");
        assert!(!detail.learn_enabled);
        assert_eq!(detail.status, "ramo fechado");
        assert!(detail.requirement.contains("Requer: vitality2"));
        assert!(detail.requirement.contains("exclui bloodthirst"));
    }

    #[test]
    fn short_label_corta_o_resumo() {
        assert_eq!(short_label("Força I (+2 Força)"), "Força I");
        assert_eq!(short_label("Baluarte"), "Baluarte");
    }
}

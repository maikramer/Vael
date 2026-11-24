//! QA: **cenários** de teste (`qa/scenarios/*.json`).
//!
//! Um cenário descreve o estado de arranque de uma sessão — cena, posição do
//! herói, quests (com as pré-condições resolvidas), itens, equipamento,
//! talentos, nível, vida, hora e clima — mais uma lista de `steps` e um
//! `expect` final. Serve três caminhos com o MESMO ficheiro:
//!
//! - **simulação** pura (sem janela, sem GPU, milissegundos):
//!   [`simulate`] corre os passos lógicos (`accept`/`kill`/`collect`/
//!   `craft`/`visit`/`progress`/`turn_in`/`expect`) sobre as mesmas funções
//!   de `quest.rs`/`quest_events.rs` que o jogo usa. `vael --qa-sim all` e o
//!   teste `tests/qa_scenarios.rs` passam todos os cenários;
//! - **arranque** (`vael --scenario <nome>` ou `VAEL_SCENARIO`; o Jackdaw
//!   usa o run config "Play (QA)" com `VAEL_SCENARIO=@active`): o
//!   [`VaelQaPlugin`] escreve o estado no `vael.game()` e teleporta o herói
//!   quando o mundo está pronto. Os saves vão para uma pasta à parte;
//! - **hot reload**: gravar o JSON (ou um `extends` dele) com o jogo a
//!   correr reaplica-o — ajusta itens/etapa/posição sem reiniciar. Pela
//!   bridge: `vael.vael.qa_apply/qa_check/qa_snapshot` (`tools/qa.py`).
//!
//! Pré-condições: com `prerequisites` (default `true`), cada quest pedida
//! fica com toda a cadeia anterior entregue (`requires` + quem a aponta em
//! `next`); um talento traz os `requires` dele; uma quest `ready` de
//! `collect` traz os itens na mochila.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use bevy::prelude::*;
use bevy::time::Real;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::content::VaelContent;
use crate::economy::{CraftRules, craft_make};
use crate::quest::{
    QuestKv, QuestRules, STATE_ACTIVE, STATE_DONE, STATE_NOT_TAKEN, STATE_READY, quest_accept,
    quest_key, quest_objective, quest_progress, quest_set_progress, quest_state, quest_turn_in,
};
use crate::quest_events::{collect_quantities, report_kill, report_visit, sync_collect};
use crate::vault::{VaultRules, item_key};
use crate::xp::{XP_KEYS, XpMap, XpRules, add_xp, ensure_xp};

/// Pasta dos cenários, relativa à raiz do projeto.
pub const SCENARIO_DIR: &str = "qa/scenarios";
/// Ficheiro com o nome do cenário activo (`tools/qa.py use <nome>`).
pub const ACTIVE_FILE: &str = "qa/.active";
/// Nome especial: lê o cenário de [`ACTIVE_FILE`].
pub const ACTIVE_NAME: &str = "@active";
/// Herói a esta distância do alvo de `player.near`: dentro do alcance do
/// [E] (a engine usa METADE do `set_interaction`: 3,5 m → 1,75 m).
const NEAR_DISTANCE: f32 = 1.4;
/// Período (s reais) da verificação de mtime para o hot reload.
const WATCH_PERIOD: f32 = 0.5;

// ── modelo ──────────────────────────────────────────────────────────────

/// Cenário tal como está no JSON. Campos desconhecidos são erro: um typo
/// (`"itens"`) não pode passar por cenário vazio.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Scenario {
    /// Texto livre (o `tools/qa.py list` mostra-o).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Cenários base (nomes em `qa/scenarios`), aplicados por ordem antes deste.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extends: Vec<String>,
    /// Cena de arranque (`main`, `qa/forge`, `interiors/forge`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scene: Option<String>,
    /// Entrada da cena (default: `dev` em `main`, a primeira nas outras).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    /// Quest em foco: fica no `state` pedido com as pré-condições cumpridas.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quest: Option<String>,
    /// Estado da quest em foco (`not_taken`/`active`/`ready`/`done`; default `active`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Etapa (1-based) da quest em foco.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<i64>,
    /// Progresso na etapa da quest em foco.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<i64>,
    /// Resolve as cadeias anteriores (default `true`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prerequisites: Option<bool>,
    /// Outras quests: `"id": "done"` ou `{ "state", "stage", "progress" }`.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub quests: BTreeMap<String, QuestSetup>,
    /// Kit inicial de `data/items.json` por baixo dos `items` (default `true`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kit: Option<bool>,
    /// Mochila: `id -> quantidade` (0 apaga).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub items: BTreeMap<String, i64>,
    /// Equipamento: `weapon`/`armor`/`charm -> id` (o item entra na mochila).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub gear: BTreeMap<String, String>,
    /// Talentos aprendidos (`data/progression.json`, com os `requires`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xp: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_points: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hp: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_hp: Option<f64>,
    /// Cargas do cantil.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flasks: Option<i64>,
    /// Biomas revelados no mapa.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub biomes: Vec<String>,
    /// Chaves cruas do `vael.game()` (ganham a tudo; `null` apaga).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub flags: BTreeMap<String, Value>,
    #[serde(skip_serializing_if = "PlayerSetup::is_empty")]
    pub player: PlayerSetup,
    /// Hora do dia (0..24).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hour: Option<f64>,
    /// Clima de arranque (`Clear`, `Rain`, `Storm`…; só no arranque).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weather: Option<String>,
    /// Passos, por ordem (lógicos na simulação; todos no jogo vivo).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<Step>,
    /// Estado esperado no fim dos passos.
    #[serde(skip_serializing_if = "Expect::is_empty")]
    pub expect: Expect,
}

/// Onde e como o herói começa.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PlayerSetup {
    /// Posição absoluta (`y` é assente no chão).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pos: Option<[f32; 3]>,
    /// Nome de uma entidade (ou `@giver`: o dador da quest em foco).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub near: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distance: Option<f32>,
    /// Invulnerável (liga as ferramentas de dev).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub god: Option<bool>,
    /// Ferramentas de dev [F1]/[F2] (default `true`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dev: Option<bool>,
}

impl PlayerSetup {
    fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

/// Estado de uma quest: só o nome ou o detalhe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum QuestSetup {
    State(String),
    Detail(QuestDetail),
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct QuestDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<i64>,
}

impl QuestSetup {
    fn detail(&self) -> QuestDetail {
        match self {
            Self::State(state) => QuestDetail {
                state: Some(state.clone()),
                ..Default::default()
            },
            Self::Detail(detail) => detail.clone(),
        }
    }
}

/// Um passo. `{"kill": "wolf"}`, `{"kill": {"kind": "wolf", "count": 3}}`…
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    // lógicos (simulação e jogo)
    Accept(String),
    /// Reporta o abate (como o `report_kill`).
    Kill(KillStep),
    /// No jogo: mata criaturas REAIS da cena pelo dano do jogo (morte →
    /// `award_kill` → quest). Na simulação é um `kill`.
    Slay(KillStep),
    Collect(ItemStep),
    Craft(ItemStep),
    Visit(String),
    Progress(ProgressStep),
    TurnIn(String),
    Expect(Expect),
    // só no jogo vivo (`tools/qa.py run`)
    Lua(String),
    Key(String),
    Wait(f64),
    Teleport([f32; 3]),
    Goto(String),
    Interact(String),
    Say(String),
    Screenshot(String),
}

impl Step {
    /// Passo que só existe com o jogo a correr. `interact` com um dador é
    /// simulado como o `npc/dialogue.lua` (entrega ou aceita).
    pub fn is_live_only(&self) -> bool {
        matches!(
            self,
            Self::Lua(_)
                | Self::Key(_)
                | Self::Wait(_)
                | Self::Teleport(_)
                | Self::Goto(_)
                | Self::Say(_)
                | Self::Screenshot(_)
        )
    }

    fn label(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| format!("{self:?}"))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum KillStep {
    Kind(String),
    Detail {
        kind: String,
        #[serde(default = "one")]
        count: i64,
        #[serde(default)]
        elite: bool,
    },
}

impl KillStep {
    fn parts(&self) -> (&str, i64, bool) {
        match self {
            Self::Kind(kind) => (kind, 1, false),
            Self::Detail { kind, count, elite } => (kind, *count, *elite),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ItemStep {
    Id(String),
    Detail {
        item: String,
        #[serde(default = "one")]
        count: i64,
    },
}

impl ItemStep {
    fn parts(&self) -> (&str, i64) {
        match self {
            Self::Id(item) => (item, 1),
            Self::Detail { item, count } => (item, *count),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgressStep {
    pub quest: String,
    pub n: i64,
}

fn one() -> i64 {
    1
}

/// Verificações. Números exactos ou expressões (`">=3"`, `"<2"`, `"!=0"`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Expect {
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub quests: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub stage: BTreeMap<String, i64>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub progress: BTreeMap<String, Cmp>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub items: BTreeMap<String, Cmp>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<Cmp>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub flags: BTreeMap<String, Value>,
}

impl Expect {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    fn merge(&mut self, child: &Expect) {
        self.quests.extend(child.quests.clone());
        self.stage.extend(child.stage.clone());
        self.progress.extend(child.progress.clone());
        self.items.extend(child.items.clone());
        union(&mut self.skills, &child.skills);
        if child.level.is_some() {
            self.level = child.level.clone();
        }
        self.flags.extend(child.flags.clone());
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Cmp {
    Exact(i64),
    Expr(String),
}

impl Cmp {
    /// `true` quando `value` cumpre a comparação; `Err` numa expressão inválida.
    pub fn eval(&self, value: f64) -> Result<bool, String> {
        let raw = match self {
            Self::Exact(n) => return Ok(value == *n as f64),
            Self::Expr(raw) => raw.trim(),
        };
        let (op, rest) = ["<=", ">=", "==", "!=", "<", ">", "="]
            .iter()
            .find_map(|op| raw.strip_prefix(op).map(|rest| (*op, rest)))
            .unwrap_or(("==", raw));
        let n: f64 = rest
            .trim()
            .parse()
            .map_err(|_| format!("comparação inválida: {raw:?}"))?;
        Ok(match op {
            "<=" => value <= n,
            ">=" => value >= n,
            "<" => value < n,
            ">" => value > n,
            "!=" => value != n,
            _ => value == n,
        })
    }
}

impl std::fmt::Display for Cmp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Exact(n) => write!(f, "{n}"),
            Self::Expr(raw) => write!(f, "{raw}"),
        }
    }
}

fn union(into: &mut Vec<String>, from: &[String]) {
    for item in from {
        if !into.contains(item) {
            into.push(item.clone());
        }
    }
}

impl Scenario {
    /// `child` por cima de `self` (o base).
    fn merge(mut self, child: Scenario) -> Scenario {
        macro_rules! take {
            ($($field:ident),*) => {$(
                if child.$field.is_some() {
                    self.$field = child.$field;
                }
            )*};
        }
        take!(
            scene,
            entry,
            quest,
            state,
            stage,
            progress,
            prerequisites,
            kit,
            level,
            xp,
            skill_points,
            hp,
            max_hp,
            flasks,
            hour,
            weather
        );
        if !child.description.is_empty() {
            self.description = child.description;
        }
        self.quests.extend(child.quests);
        self.items.extend(child.items);
        self.gear.extend(child.gear);
        union(&mut self.skills, &child.skills);
        union(&mut self.biomes, &child.biomes);
        self.flags.extend(child.flags);
        let player = child.player;
        if player.pos.is_some() || player.near.is_some() {
            self.player.pos = player.pos;
            self.player.near = player.near;
        }
        if player.distance.is_some() {
            self.player.distance = player.distance;
        }
        if player.god.is_some() {
            self.player.god = player.god;
        }
        if player.dev.is_some() {
            self.player.dev = player.dev;
        }
        self.steps.extend(child.steps);
        self.expect.merge(&child.expect);
        self.extends.clear();
        self
    }
}

// ── carregar ────────────────────────────────────────────────────────────

/// Cenário carregado: o resultado do `extends` e os ficheiros que o compõem
/// (o hot reload vigia todos).
#[derive(Debug, Clone)]
pub struct LoadedScenario {
    pub name: String,
    pub scenario: Scenario,
    pub files: Vec<PathBuf>,
}

/// Caminho de um nome: `qa/scenarios/<nome>.json`, um caminho com `.json`
/// (relativo à raiz) ou `@active` ([`ACTIVE_FILE`]).
pub fn scenario_path(root: &Path, name: &str) -> Result<PathBuf, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("cenário sem nome".into());
    }
    if name == ACTIVE_NAME {
        let active = root.join(ACTIVE_FILE);
        let text = std::fs::read_to_string(&active).map_err(|err| {
            format!(
                "{}: {err} (escolhe um com `tools/qa.py use <cenário>`)",
                active.display()
            )
        })?;
        return scenario_path(root, text.trim());
    }
    if name.ends_with(".json") {
        let path = Path::new(name);
        return Ok(if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        });
    }
    Ok(root.join(SCENARIO_DIR).join(format!("{name}.json")))
}

/// Nome legível de um caminho de cenário.
fn display_name(root: &Path, path: &Path) -> String {
    let dir = root.join(SCENARIO_DIR);
    path.strip_prefix(&dir)
        .ok()
        .and_then(|rel| rel.to_str())
        .map(|rel| rel.trim_end_matches(".json").to_string())
        .unwrap_or_else(|| path.display().to_string())
}

/// Lê o cenário e os `extends` (recursivo, sem ciclos).
pub fn load_scenario(root: &Path, name: &str) -> Result<LoadedScenario, String> {
    let path = scenario_path(root, name)?;
    let mut files = Vec::new();
    let mut stack = Vec::new();
    let scenario = load_rec(root, &path, &mut files, &mut stack)?;
    Ok(LoadedScenario {
        name: display_name(root, &path),
        scenario,
        files,
    })
}

fn load_rec(
    root: &Path,
    path: &Path,
    files: &mut Vec<PathBuf>,
    stack: &mut Vec<PathBuf>,
) -> Result<Scenario, String> {
    if stack.iter().any(|seen| seen == path) {
        return Err(format!("`extends` em ciclo: {}", path.display()));
    }
    if stack.len() > 16 {
        return Err("`extends` demasiado fundo".into());
    }
    let text = std::fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))?;
    let own: Scenario =
        serde_json::from_str(&text).map_err(|err| format!("{}: {err}", path.display()))?;
    files.push(path.to_path_buf());
    stack.push(path.to_path_buf());
    let mut base = Scenario::default();
    for parent in &own.extends {
        let parent_path = scenario_path(root, parent)?;
        let loaded = load_rec(root, &parent_path, files, stack)?;
        base = base.merge(loaded);
    }
    stack.pop();
    Ok(base.merge(own))
}

/// Todos os cenários em `qa/scenarios` (recursivo), por nome.
pub fn list_scenarios(root: &Path) -> Vec<String> {
    let dir = root.join(SCENARIO_DIR);
    let mut out = Vec::new();
    let mut pending = vec![dir.clone()];
    while let Some(next) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&next) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|ext| ext == "json") {
                out.push(display_name(root, &path));
            }
        }
    }
    out.sort();
    out
}

// ── regras ──────────────────────────────────────────────────────────────

/// As regras que a resolução e a simulação usam (do `VaelContent`).
#[derive(Debug, Clone)]
pub struct QaRules {
    pub quests: QuestRules,
    pub vault: VaultRules,
    pub xp: XpRules,
    /// Receitas da forja (`data/recipes.json`): o `craft` gasta os custos.
    pub craft: CraftRules,
    pub starting: BTreeMap<String, i64>,
    pub items: BTreeSet<String>,
    /// `id -> slot` dos itens equipáveis.
    pub slots: BTreeMap<String, String>,
    /// `id -> requires` dos talentos.
    pub skills: BTreeMap<String, Vec<String>>,
    pub landmarks: BTreeSet<String>,
}

impl QaRules {
    pub fn from_content(content: &VaelContent) -> Self {
        let split = |raw: &Option<String>| -> Vec<String> {
            raw.as_deref()
                .unwrap_or("")
                .split([',', ' '])
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .collect()
        };
        Self {
            quests: QuestRules::from_content(content),
            vault: VaultRules::from_items(&content.items),
            xp: XpRules::from_progression(&content.progression),
            craft: CraftRules::from_content(&content.items, &content.recipes),
            starting: content.items.starting.clone(),
            items: content
                .items
                .items
                .iter()
                .map(|item| item.id.clone())
                .collect(),
            slots: content
                .items
                .items
                .iter()
                .filter(|item| !item.slot.is_empty())
                .map(|item| (item.id.clone(), item.slot.clone()))
                .collect(),
            skills: content
                .progression
                .skills
                .iter()
                .map(|skill| (skill.id.clone(), split(&skill.requires)))
                .collect(),
            landmarks: content
                .landmarks
                .landmarks
                .iter()
                .map(|mark| mark.name.clone())
                .collect(),
        }
    }
}

// ── resolver ────────────────────────────────────────────────────────────

/// O que um cenário escreve e faz no arranque.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plan {
    /// Chaves do `vael.game()` (`null` apaga).
    pub kv: QuestKv,
    pub pos: Option<[f32; 3]>,
    /// Nome da entidade ao pé da qual o herói nasce.
    pub near: Option<String>,
    pub distance: f32,
    pub god: bool,
    pub dev: bool,
    pub hour: Option<f64>,
    /// Avisos (ids desconhecidos, pré-condições forçadas…).
    pub warnings: Vec<String>,
}

/// Chave `q.<id>.<suffix>`.
fn qk(id: &str, suffix: &str) -> String {
    format!("{}.{suffix}", quest_key(id))
}

fn num(kv: &QuestKv, key: &str) -> f64 {
    kv.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

/// Cadeia anterior de `id`: `requires` (transitivo) e quem aponta para ela
/// em `next`, por ordem (os mais antigos primeiro).
pub fn ancestors(rules: &QuestRules, id: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    fn walk(rules: &QuestRules, id: &str, seen: &mut BTreeSet<String>, out: &mut Vec<String>) {
        let Some(def) = rules.quests.get(id) else {
            return;
        };
        let mut before: Vec<String> = def.requires.clone();
        before.extend(
            rules
                .quests
                .values()
                .filter(|other| other.next == id)
                .map(|other| other.id.clone()),
        );
        for prev in before {
            if seen.insert(prev.clone()) {
                walk(rules, &prev, seen, out);
                out.push(prev);
            }
        }
    }
    seen.insert(id.to_string());
    walk(rules, id, &mut seen, &mut out);
    out
}

/// Talentos com os `requires`, os pré-requisitos primeiro.
fn skill_closure(rules: &QaRules, wanted: &[String], warnings: &mut Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(
        rules: &QaRules,
        id: &str,
        out: &mut Vec<String>,
        warnings: &mut Vec<String>,
        depth: usize,
    ) {
        if out.iter().any(|seen| seen == id) || depth > 16 {
            return;
        }
        let Some(requires) = rules.skills.get(id) else {
            warnings.push(format!("talento desconhecido: {id}"));
            return;
        };
        for prev in requires {
            walk(rules, prev, out, warnings, depth + 1);
        }
        out.push(id.to_string());
    }
    for id in wanted {
        walk(rules, id, &mut out, warnings, 0);
    }
    out
}

/// Põe `id` no estado pedido, com etapa/progresso, pela lógica real.
fn set_quest(
    rules: &QaRules,
    kv: &mut QuestKv,
    id: &str,
    detail: &QuestDetail,
    warnings: &mut Vec<String>,
) {
    let Some(def) = rules.quests.quests.get(id) else {
        warnings.push(format!("quest desconhecida: {id}"));
        return;
    };
    let state = detail.state.as_deref().unwrap_or(STATE_ACTIVE);
    match state {
        STATE_NOT_TAKEN => {
            let prefix = quest_key(id);
            kv.retain(|key, _| key != &prefix && !key.starts_with(&format!("{prefix}.")));
            return;
        }
        STATE_DONE => {
            kv.insert(quest_key(id), json!(STATE_DONE));
            return;
        }
        STATE_ACTIVE | STATE_READY => {}
        other => {
            warnings.push(format!("{id}: estado inválido {other:?}"));
            return;
        }
    }
    if quest_state(&rules.quests, kv, id) == STATE_NOT_TAKEN && !quest_accept(&rules.quests, kv, id)
    {
        warnings.push(format!(
            "{id}: aceite à força (requires por cumprir; `prerequisites: false`?)"
        ));
        kv.insert(quest_key(id), json!(STATE_ACTIVE));
        kv.insert(qk(id, "n"), json!(0));
    }
    let total = def.stages.len() as i64;
    let stage = match (state, detail.stage) {
        (_, Some(stage)) => Some(stage),
        (STATE_READY, None) if total > 0 => Some(total),
        _ => None,
    };
    if let Some(stage) = stage {
        if total == 0 {
            warnings.push(format!("{id}: `stage` numa quest sem `stages`"));
        } else {
            let stage = stage.clamp(1, total);
            kv.insert(qk(id, "stage"), json!(stage));
            kv.insert(qk(id, "n"), json!(0));
            let count = def.stages[(stage - 1) as usize].count.max(1);
            kv.insert(qk(id, "need"), json!(count));
        }
    }
    let (_, need) = quest_progress(&rules.quests, kv, id);
    let target = match state {
        STATE_READY => Some(need),
        _ => detail.progress,
    };
    if let Some(n) = target {
        let view = quest_objective(&rules.quests, kv, id);
        if view.objective_type == "collect" && !view.target.is_empty() {
            // A entrega tira os itens: `ready` sem eles volta a `active`.
            let key = item_key(&view.target);
            if num(kv, &key) < n as f64 {
                kv.insert(key, json!(n));
            }
        }
        if n > 0 {
            quest_set_progress(&rules.quests, kv, id, n as f64);
        }
    }
}

/// Resolve um cenário contra as regras.
pub fn resolve(rules: &QaRules, scenario: &Scenario) -> Plan {
    let mut warnings = Vec::new();
    let mut kv = QuestKv::new();
    // Um arranque de QA é sempre "depois do título e do prólogo".
    kv.insert("title_done".into(), json!(true));
    kv.insert("story_prologo".into(), json!(true));
    kv.insert("eco_pulled".into(), json!(true));

    // Mochila: kit por baixo, o pedido por cima.
    if scenario.kit != Some(false) {
        for (id, n) in &rules.starting {
            kv.insert(item_key(id), json!(n));
        }
    }
    for (id, n) in &scenario.items {
        if !rules.items.contains(id) {
            warnings.push(format!("item desconhecido: {id}"));
        }
        if *n <= 0 {
            kv.insert(item_key(id), json!(0));
        } else {
            kv.insert(item_key(id), json!(n));
        }
    }
    for (slot, id) in &scenario.gear {
        match rules.slots.get(id) {
            Some(real) if real == slot => {}
            Some(real) => warnings.push(format!("{id} é `{real}`, não `{slot}`")),
            None => warnings.push(format!("{id} não é equipável")),
        }
        kv.insert(format!("gear.{slot}"), json!(id));
        if num(&kv, &item_key(id)) < 1.0 {
            kv.insert(item_key(id), json!(1));
        }
    }

    // Talentos (com os pré-requisitos).
    for id in skill_closure(rules, &scenario.skills, &mut warnings) {
        kv.insert(format!("skill.{id}"), json!(true));
    }

    // Nível pela curva real (pontos ganhos incluídos).
    let mut xp = XpMap::new();
    ensure_xp(&mut xp, &rules.xp);
    let level = scenario.level.unwrap_or(1).max(1) as f64;
    let mut guard = 0;
    while xp.get("level").copied().unwrap_or(1.0) < level && guard < 1000 {
        let missing = xp["xp_next"] - xp["xp"];
        add_xp(&mut xp, &rules.xp, missing.max(1.0));
        guard += 1;
    }
    if let Some(value) = scenario.xp {
        xp.insert("xp".into(), value.max(0) as f64);
    }
    if let Some(points) = scenario.skill_points {
        xp.insert("skill_points".into(), points.max(0) as f64);
    }
    for key in XP_KEYS {
        if let Some(value) = xp.get(key) {
            kv.insert(key.to_string(), json!(*value as i64));
        }
    }

    if let Some(max) = scenario.max_hp {
        kv.insert("max_hp".into(), json!(max.max(1.0)));
    }
    if let Some(hp) = scenario.hp {
        kv.insert("hp".into(), json!(hp.max(0.0)));
    }
    if let Some(flasks) = scenario.flasks {
        kv.insert("flasks".into(), json!(flasks.max(0)));
    }
    for biome in &scenario.biomes {
        kv.insert(format!("biome.{biome}"), json!(true));
    }

    // Quests: a em foco e as outras, cada uma com a cadeia antes.
    let mut wanted: Vec<(String, QuestDetail)> = Vec::new();
    if let Some(id) = &scenario.quest {
        wanted.push((
            id.clone(),
            QuestDetail {
                state: scenario.state.clone(),
                stage: scenario.stage,
                progress: scenario.progress,
            },
        ));
    }
    for (id, setup) in &scenario.quests {
        wanted.push((id.clone(), setup.detail()));
    }
    let explicit: BTreeSet<&str> = wanted.iter().map(|(id, _)| id.as_str()).collect();
    if scenario.prerequisites != Some(false) {
        for (id, detail) in &wanted {
            if detail.state.as_deref() == Some(STATE_NOT_TAKEN)
                && !rules.quests.quests.contains_key(id)
            {
                continue;
            }
            for prev in ancestors(&rules.quests, id) {
                if !explicit.contains(prev.as_str()) {
                    kv.insert(quest_key(&prev), json!(STATE_DONE));
                }
            }
        }
    }
    for (id, detail) in &wanted {
        set_quest(rules, &mut kv, id, detail, &mut warnings);
    }
    // Os objetivos `collect` seguem a mochila, como no jogo.
    let bag = collect_quantities(&rules.quests, &kv);
    sync_collect(&rules.quests, &mut kv, &bag);

    for (key, value) in &scenario.flags {
        kv.insert(key.clone(), value.clone());
    }

    let near = scenario.player.near.as_deref().map(|near| {
        if near == "@giver" {
            scenario
                .quest
                .as_deref()
                .map(|quest| giver_name(&rules.quests, quest))
                .unwrap_or_else(|| {
                    warnings.push("`@giver` sem `quest`".into());
                    String::new()
                })
        } else {
            near.to_string()
        }
    });
    let god = scenario.player.god == Some(true);
    Plan {
        kv,
        pos: scenario.player.pos,
        near: near.filter(|near| !near.is_empty()),
        distance: scenario.player.distance.unwrap_or(NEAR_DISTANCE),
        god,
        dev: god || scenario.player.dev != Some(false),
        hour: scenario.hour,
        warnings,
    }
}

/// Entidade do dador de uma quest: o `npc` da definição (o nome da
/// entidade no mundo e nas cenas de `tools/qa.py scene`); `npc_<id>` sem ele.
pub fn giver_name(rules: &QuestRules, quest: &str) -> String {
    rules
        .quests
        .get(quest)
        .map(|def| def.npc.clone())
        .filter(|npc| !npc.is_empty())
        .unwrap_or_else(|| format!("npc_{quest}"))
}

// ── verificar ───────────────────────────────────────────────────────────

/// Falhas de `expect` sobre o estado `kv` (vazio = passou).
pub fn check(rules: &QaRules, kv: &QuestKv, expect: &Expect) -> Vec<String> {
    let mut failures = Vec::new();
    for (id, want) in &expect.quests {
        let got = quest_state(&rules.quests, kv, id);
        if &got != want {
            failures.push(format!("quest {id}: {got} (esperado {want})"));
        }
    }
    for (id, want) in &expect.stage {
        let view = quest_objective(&rules.quests, kv, id);
        if view.at != *want {
            failures.push(format!("quest {id}: etapa {} (esperado {want})", view.at));
        }
    }
    for (id, cmp) in &expect.progress {
        let (have, _) = quest_progress(&rules.quests, kv, id);
        match cmp.eval(have as f64) {
            Ok(true) => {}
            Ok(false) => failures.push(format!("quest {id}: progresso {have} (esperado {cmp})")),
            Err(err) => failures.push(err),
        }
    }
    for (id, cmp) in &expect.items {
        let have = num(kv, &item_key(id));
        match cmp.eval(have) {
            Ok(true) => {}
            Ok(false) => failures.push(format!("item {id}: {have} (esperado {cmp})")),
            Err(err) => failures.push(err),
        }
    }
    for id in &expect.skills {
        if kv.get(&format!("skill.{id}")) != Some(&json!(true)) {
            failures.push(format!("talento {id} por aprender"));
        }
    }
    if let Some(cmp) = &expect.level {
        let level = num(kv, "level").max(1.0);
        match cmp.eval(level) {
            Ok(true) => {}
            Ok(false) => failures.push(format!("nível {level} (esperado {cmp})")),
            Err(err) => failures.push(err),
        }
    }
    for (key, want) in &expect.flags {
        let got = kv.get(key).cloned().unwrap_or(Value::Null);
        let same = match (got.as_f64(), want.as_f64()) {
            (Some(a), Some(b)) => a == b,
            _ => &got == want,
        };
        if !same {
            failures.push(format!("{key}: {got} (esperado {want})"));
        }
    }
    failures
}

// ── simular ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Ok,
    Fail,
    /// Passo vivo (`key`, `interact`, …): só o `tools/qa.py run` o executa.
    Skipped,
}

#[derive(Debug, Clone, Serialize)]
pub struct StepReport {
    pub index: usize,
    pub step: String,
    pub status: StepStatus,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SimReport {
    pub name: String,
    pub ok: bool,
    pub warnings: Vec<String>,
    pub steps: Vec<StepReport>,
    pub failures: Vec<String>,
}

fn add_items(rules: &QaRules, kv: &mut QuestKv, item: &str, count: i64) {
    let key = item_key(item);
    let cap = rules.vault.cap_for(item).max(1) as f64;
    let next = (num(kv, &key) + count.max(0) as f64).min(cap);
    kv.insert(key, json!(next as i64));
}

fn sync(rules: &QaRules, kv: &mut QuestKv) {
    let bag = collect_quantities(&rules.quests, kv);
    sync_collect(&rules.quests, kv, &bag);
}

/// Um passo lógico sobre `kv` (`Err` = falhou).
pub fn run_step(rules: &QaRules, kv: &mut QuestKv, step: &Step) -> Result<String, String> {
    match step {
        Step::Accept(id) => {
            if quest_accept(&rules.quests, kv, id) {
                sync(rules, kv);
                Ok(format!("{id}: active"))
            } else {
                Err(format!(
                    "{id}: não aceitou (estado {}, disponível {})",
                    quest_state(&rules.quests, kv, id),
                    crate::quest::quest_available(&rules.quests, kv, id)
                ))
            }
        }
        Step::Kill(kill) | Step::Slay(kill) => {
            let (kind, count, elite) = kill.parts();
            let mut hits = 0;
            for _ in 0..count.max(1) {
                hits += report_kill(&rules.quests, kv, kind, elite).len();
            }
            Ok(format!("{kind} ×{count}: {hits} avanço(s)"))
        }
        Step::Collect(item) => {
            let (id, count) = item.parts();
            add_items(rules, kv, id, count);
            sync(rules, kv);
            Ok(format!("{id} = {}", num(kv, &item_key(id))))
        }
        Step::Craft(item) => {
            let (id, count) = item.parts();
            // A receita real: custos fora da mochila, produto dentro (falha
            // sem materiais ou com a pilha cheia, como na bigorna).
            let Some(recipe) = rules
                .craft
                .recipes
                .iter()
                .find(|recipe| recipe.out == id || recipe.id == id)
                .cloned()
            else {
                return Err(format!("{id}: sem receita em data/recipes.json"));
            };
            let mut bag: BTreeMap<String, i64> = kv
                .iter()
                .filter(|(key, _)| key.starts_with("item."))
                .map(|(key, value)| (key.clone(), value.as_f64().unwrap_or(0.0) as i64))
                .collect();
            let mut made = 0;
            for _ in 0..count.max(1) {
                let outcome = craft_make(&rules.craft, &mut bag, &recipe.id);
                if !outcome.ok {
                    let cost: Vec<String> = recipe
                        .cost
                        .iter()
                        .map(|(item, n)| format!("{n}× {item} (tens {})", num(kv, &item_key(item))))
                        .collect();
                    return Err(format!(
                        "forja {}: {:?} — custo {}",
                        recipe.id,
                        outcome.reason,
                        cost.join(", ")
                    ));
                }
                made += outcome.count;
            }
            for (key, value) in bag {
                kv.insert(key, json!(value));
            }
            let mut hits = 0;
            for qid in &rules.quests.order {
                if quest_state(&rules.quests, kv, qid) != STATE_ACTIVE {
                    continue;
                }
                let view = quest_objective(&rules.quests, kv, qid);
                if view.objective_type == "craft" && view.target == recipe.out {
                    let (have, need) = quest_progress(&rules.quests, kv, qid);
                    quest_set_progress(&rules.quests, kv, qid, (have + made).min(need) as f64);
                    hits += 1;
                }
            }
            sync(rules, kv);
            Ok(format!("{} ×{made}: {hits} avanço(s)", recipe.out))
        }
        Step::Visit(place) => {
            let hits = report_visit(&rules.quests, kv, place).len();
            Ok(format!("{place}: {hits} avanço(s)"))
        }
        Step::Progress(progress) => {
            let result = quest_set_progress(&rules.quests, kv, &progress.quest, progress.n as f64);
            if result.ok {
                sync(rules, kv);
                Ok(format!("{} = {}", progress.quest, progress.n))
            } else {
                Err(format!("{}: progresso recusado", progress.quest))
            }
        }
        Step::TurnIn(id) => {
            let result = quest_turn_in(&rules.quests, &rules.vault, &rules.xp, kv, id);
            sync(rules, kv);
            if result.ok {
                Ok(format!("{id}: entregue (+{} ouro)", result.gold))
            } else {
                Err(format!(
                    "{id}: entrega recusada ({})",
                    result.reason.map(|reason| reason.as_str()).unwrap_or("?")
                ))
            }
        }
        Step::Interact(name) => {
            // O que o `npc/dialogue.lua` faz ao [E]: entrega a `ready`, senão
            // aceita a primeira disponível das quests deste dador.
            let giving: Vec<String> = rules
                .quests
                .order
                .iter()
                .filter(|id| {
                    rules
                        .quests
                        .quests
                        .get(*id)
                        .is_some_and(|def| &def.npc == name)
                })
                .cloned()
                .collect();
            if let Some(id) = giving
                .iter()
                .find(|id| quest_state(&rules.quests, kv, id) == STATE_READY)
            {
                return run_step(rules, kv, &Step::TurnIn(id.clone()))
                    .map(|detail| format!("diálogo: {detail}"));
            }
            if let Some(id) = giving.iter().find(|id| {
                quest_state(&rules.quests, kv, id) == STATE_NOT_TAKEN
                    && crate::quest::quest_available(&rules.quests, kv, id)
            }) {
                return run_step(rules, kv, &Step::Accept(id.clone()))
                    .map(|detail| format!("diálogo: {detail}"));
            }
            Ok(if giving.is_empty() {
                format!("{name}: não é dador (sem efeito na simulação)")
            } else {
                "diálogo: nada a aceitar nem a entregar".into()
            })
        }
        Step::Expect(expect) => {
            let failures = check(rules, kv, expect);
            if failures.is_empty() {
                Ok("ok".into())
            } else {
                Err(failures.join("; "))
            }
        }
        _ => Ok(String::new()),
    }
}

/// Simulação completa: plano + passos lógicos + `expect` final.
pub fn simulate(rules: &QaRules, name: &str, scenario: &Scenario) -> SimReport {
    let plan = resolve(rules, scenario);
    let mut kv = plan.kv.clone();
    kv.retain(|_, value| !value.is_null());
    let mut steps = Vec::new();
    let mut failures = Vec::new();
    for (index, step) in scenario.steps.iter().enumerate() {
        // `@giver` nos passos é o dador da quest em foco (como no `qa.py`).
        let resolved;
        let step = match (step, scenario.quest.as_deref()) {
            (Step::Interact(name), Some(quest)) if name == "@giver" => {
                resolved = Step::Interact(giver_name(&rules.quests, quest));
                &resolved
            }
            _ => step,
        };
        if step.is_live_only() {
            steps.push(StepReport {
                index,
                step: step.label(),
                status: StepStatus::Skipped,
                detail: String::new(),
            });
            continue;
        }
        match run_step(rules, &mut kv, step) {
            Ok(detail) => steps.push(StepReport {
                index,
                step: step.label(),
                status: StepStatus::Ok,
                detail,
            }),
            Err(detail) => {
                failures.push(format!("passo {}: {detail}", index + 1));
                steps.push(StepReport {
                    index,
                    step: step.label(),
                    status: StepStatus::Fail,
                    detail,
                });
            }
        }
    }
    failures.extend(check(rules, &kv, &scenario.expect));
    SimReport {
        name: name.to_string(),
        ok: failures.is_empty(),
        warnings: plan.warnings,
        steps,
        failures,
    }
}

/// Validação estática (ids e cena) — não precisa do jogo.
pub fn lint(root: &Path, rules: &QaRules, scenario: &Scenario) -> Vec<String> {
    let mut errors = resolve(rules, scenario).warnings;
    if let Some(scene) = &scenario.scene
        && scene != "main"
    {
        let found = ["bsn", "xml"]
            .iter()
            .any(|ext| root.join("scenes").join(format!("{scene}.{ext}")).is_file());
        if !found {
            errors.push(format!("cena inexistente: scenes/{scene}.bsn"));
        }
    }
    let known_quest = |id: &str, errors: &mut Vec<String>| {
        if !rules.quests.quests.contains_key(id) {
            errors.push(format!("passo com quest desconhecida: {id}"));
        }
    };
    for step in &scenario.steps {
        match step {
            Step::Accept(id) | Step::TurnIn(id) => known_quest(id, &mut errors),
            Step::Progress(progress) => known_quest(&progress.quest, &mut errors),
            Step::Collect(item) | Step::Craft(item) => {
                let (id, _) = item.parts();
                if !rules.items.contains(id) {
                    errors.push(format!("passo com item desconhecido: {id}"));
                }
            }
            Step::Visit(place) if !rules.landmarks.contains(place) => {
                errors.push(format!("visit: marco fora de data/landmarks.json: {place}"));
            }
            _ => {}
        }
    }
    for id in scenario.expect.quests.keys() {
        known_quest(id, &mut errors);
    }
    errors.sort();
    errors.dedup();
    errors
}

// ── instantâneo ─────────────────────────────────────────────────────────

/// Chaves que são estado de frame, não do jogador.
fn volatile(key: &str) -> bool {
    const EXACT: &[&str] = &[
        "dying",
        "death_pending",
        "death_wait",
        "iframes",
        "session_id",
        "pending_respawn",
        "vael_god",
        "title_done",
        "eco_pulled",
        "enemies",
        "xp_next",
        "playtime_secs",
        "vael_rust_vitals",
        "equip_index",
    ];
    const PREFIX: &[&str] = &["dmg_", "ab.", "cd.", "spawn_", "save_"];
    EXACT.contains(&key)
        || PREFIX.iter().any(|prefix| key.starts_with(prefix))
        || key.ends_with("_tick")
}

/// Estados alterados (`venom`/`burn`/`frost`) só contam quando activos.
fn idle_status(key: &str, value: &Value) -> bool {
    matches!(key, "venom" | "burn" | "frost") && value.as_f64().unwrap_or(0.0) <= 0.0
}

/// Cenário que reproduz o estado `kv` (para gravar um momento de jogo).
pub fn snapshot(rules: &QaRules, kv: &QuestKv) -> Scenario {
    let mut out = Scenario {
        prerequisites: Some(false),
        kit: Some(false),
        ..Default::default()
    };
    let mut used = BTreeSet::new();
    for id in &rules.quests.order {
        let state = quest_state(&rules.quests, kv, id);
        if state == STATE_NOT_TAKEN {
            continue;
        }
        let prefix = quest_key(id);
        used.extend(
            kv.keys()
                .filter(|key| *key == &prefix || key.starts_with(&format!("{prefix}.")))
                .cloned(),
        );
        let setup = if state == STATE_DONE {
            QuestSetup::State(state)
        } else {
            let view = quest_objective(&rules.quests, kv, id);
            let (have, _) = quest_progress(&rules.quests, kv, id);
            QuestSetup::Detail(QuestDetail {
                state: Some(state),
                stage: (view.total > 0).then_some(view.at),
                progress: (have > 0).then_some(have),
            })
        };
        out.quests.insert(id.clone(), setup);
    }
    for (key, value) in kv {
        if used.contains(key) || volatile(key) || idle_status(key, value) {
            continue;
        }
        if let Some(id) = key.strip_prefix("item.") {
            let n = value.as_f64().unwrap_or(0.0) as i64;
            if n > 0 {
                out.items.insert(id.to_string(), n);
            }
        } else if let Some(id) = key.strip_prefix("skill.") {
            if value == &json!(true) {
                out.skills.push(id.to_string());
            }
        } else if let Some(slot) = key.strip_prefix("gear.") {
            if let Some(id) = value.as_str().filter(|id| !id.is_empty()) {
                out.gear.insert(slot.to_string(), id.to_string());
            }
        } else if let Some(biome) = key.strip_prefix("biome.") {
            if value == &json!(true) {
                out.biomes.push(biome.to_string());
            }
        } else {
            match key.as_str() {
                "level" => out.level = value.as_i64().or(value.as_f64().map(|v| v as i64)),
                "xp" => out.xp = value.as_f64().map(|v| v as i64),
                "skill_points" => out.skill_points = value.as_f64().map(|v| v as i64),
                "hp" => out.hp = value.as_f64(),
                "max_hp" => out.max_hp = value.as_f64(),
                "flasks" => out.flasks = value.as_f64().map(|v| v as i64),
                _ => {
                    out.flags.insert(key.clone(), value.clone());
                }
            }
        }
    }
    out
}

// ── jogo vivo ───────────────────────────────────────────────────────────

/// O cenário pedido no arranque (`main.rs`: `--scenario`/`VAEL_SCENARIO`).
#[derive(Resource, Debug, Clone)]
pub struct QaRequest {
    pub name: String,
}

/// Pedidos vindos do Lua (`vael.vael.qa_apply`).
#[derive(Default)]
struct QaInbox {
    apply: Option<Option<String>>,
}

#[derive(Resource, Clone, Default)]
struct QaShared {
    inbox: Arc<Mutex<QaInbox>>,
    current: Arc<Mutex<Option<LoadedScenario>>>,
}

/// Fase da aplicação ao vivo.
#[derive(Resource, Default)]
struct QaRuntime {
    /// Plano por aplicar e em que fase está.
    pending: Option<PendingApply>,
    mtimes: Vec<(PathBuf, Option<SystemTime>)>,
    watch_acc: f32,
}

struct PendingApply {
    plan: Plan,
    name: String,
    kv_done: bool,
    /// Frames de espera depois do `ready` (o herói ainda assenta).
    settle: u32,
}

pub struct VaelQaPlugin;

impl Plugin for VaelQaPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<QaShared>()
            .init_resource::<QaRuntime>()
            .add_systems(
                Startup,
                install_qa_api.after(crate::quest::install_quest_api),
            )
            .add_systems(Update, (qa_watch, qa_apply).chain());
    }
}

fn mtimes(files: &[PathBuf]) -> Vec<(PathBuf, Option<SystemTime>)> {
    files
        .iter()
        .map(|path| {
            let time = std::fs::metadata(path)
                .and_then(|meta| meta.modified())
                .ok();
            (path.clone(), time)
        })
        .collect()
}

fn project_root(world: &World) -> PathBuf {
    world
        .get_resource::<crate::VaelProjectRoot>()
        .map(|root| root.0.clone())
        .filter(|root| !root.as_os_str().is_empty())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
}

/// Carrega `name` e põe o plano em fila (com toast/log dos avisos).
fn queue(world: &mut World, name: &str) {
    let root = project_root(world);
    let Some(content) = world.get_resource::<VaelContent>() else {
        return;
    };
    let rules = QaRules::from_content(content);
    match load_scenario(&root, name) {
        Ok(loaded) => {
            let plan = resolve(&rules, &loaded.scenario);
            for warning in &plan.warnings {
                warn!("vael qa [{}]: {warning}", loaded.name);
            }
            let shared = world.resource::<QaShared>().clone();
            let mut runtime = world.resource_mut::<QaRuntime>();
            runtime.mtimes = mtimes(&loaded.files);
            runtime.pending = Some(PendingApply {
                plan,
                name: loaded.name.clone(),
                kv_done: false,
                settle: 0,
            });
            *shared.current.lock().unwrap() = Some(loaded);
        }
        Err(err) => error!("vael qa: {err}"),
    }
}

/// Arranque, pedidos do Lua e hot reload dos ficheiros do cenário.
fn qa_watch(world: &mut World) {
    let first = world.get_resource::<QaRequest>().cloned();
    if let Some(request) = first {
        world.remove_resource::<QaRequest>();
        queue(world, &request.name);
        return;
    }
    let shared = world.resource::<QaShared>().clone();
    let asked = shared.inbox.lock().unwrap().apply.take();
    if let Some(name) = asked {
        let name = name.or_else(|| {
            shared
                .current
                .lock()
                .unwrap()
                .as_ref()
                .map(|loaded| loaded.name.clone())
        });
        if let Some(name) = name {
            queue(world, &name);
        }
        return;
    }
    let dt = world.resource::<Time<Real>>().delta_secs();
    let mut runtime = world.resource_mut::<QaRuntime>();
    if runtime.mtimes.is_empty() {
        return;
    }
    runtime.watch_acc += dt;
    if runtime.watch_acc < WATCH_PERIOD {
        return;
    }
    runtime.watch_acc = 0.0;
    let files: Vec<PathBuf> = runtime
        .mtimes
        .iter()
        .map(|(path, _)| path.clone())
        .collect();
    if mtimes(&files) == runtime.mtimes {
        return;
    }
    let name = shared
        .current
        .lock()
        .unwrap()
        .as_ref()
        .map(|loaded| loaded.name.clone());
    if let Some(name) = name {
        info!("vael qa: {name} mudou no disco — a reaplicar");
        queue(world, &name);
    }
}

/// Estado já (mal o Lua existe); posição e hora quando o mundo está pronto.
fn qa_apply(world: &mut World) {
    let Some(mut pending) = world.resource_mut::<QaRuntime>().pending.take() else {
        return;
    };
    // A REPL dedicada do `bevy_ext_luau` (recurso da app, criado à pressa) —
    // o host antigo (`vael_engine::luau::host::LuaScriptHost`) morreu; o store do
    // estado é do processo e a REPL existe desde o primeiro frame.
    let lua = bevy_ext_luau::repl::repl_or_init(world);
    if !pending.kv_done {
        seed_qa_globals(&pending.plan);
        apply_kv(&lua, &pending.plan);
        pending.kv_done = true;
    }
    let ready = world
        .get_resource::<vael_engine::world_ready::WorldReadiness>()
        .is_none_or(|readiness| readiness.ready);
    if !ready {
        world.resource_mut::<QaRuntime>().pending = Some(pending);
        return;
    }
    if pending.settle < 3 {
        pending.settle += 1;
        world.resource_mut::<QaRuntime>().pending = Some(pending);
        return;
    }
    // De novo depois do carregamento: o arranque da cena (título, saves,
    // seeds) pode ter escrito por cima no meio.
    apply_kv(&lua, &pending.plan);
    if let Some(hour) = pending.plan.hour
        && let Some(mut clock) = world.get_resource_mut::<bevy_elements::WorldClock>()
    {
        clock.hour = hour.rem_euclid(24.0);
    }
    let target = pending.plan.pos.map(Vec3::from).or_else(|| {
        let name = pending.plan.near.as_deref()?;
        let found = find_near(world, name, pending.plan.distance);
        if found.is_none() {
            warn!("vael qa: `player.near` {name:?} não existe nesta cena");
        }
        found
    });
    if let Some(target) = target {
        crate::respawn::teleport_player(world, target);
    }
    info!(
        scenario = %pending.name,
        keys = pending.plan.kv.len(),
        teleport = ?target,
        warnings = pending.plan.warnings.len(),
        "vael qa: cenário aplicado"
    );
    if world.contains_resource::<Messages<vael_engine::luau::components::ScriptToast>>() {
        world.write_message(vael_engine::luau::components::ScriptToast(format!(
            "QA: {}",
            pending.name
        )));
    }
}

/// Prefixos do estado do jogador que um cenário REPÕE (o que não está no
/// plano é apagado): reaplicar a meio de uma sessão volta ao ponto de
/// partida em vez de somar à mochila/quests da corrida anterior.
const OWNED_PREFIXES: &[&str] = &["q.", "item.", "skill.", "gear.", "biome."];

/// O patch completo: o plano + `null` nas chaves de jogador a mais.
pub fn reset_patch(current: &QuestKv, plan: &QuestKv) -> QuestKv {
    let mut patch = plan.clone();
    for key in current.keys() {
        if !plan.contains_key(key) && OWNED_PREFIXES.iter().any(|prefix| key.starts_with(prefix)) {
            patch.insert(key.clone(), Value::Null);
        }
    }
    patch
}

/// Globals de QA (`vael_qa_god`/`vael_dev`) para os contextos BMS.
///
/// A VM antiga era PARTILHADA — um `globals.set` chegava a todos os
/// scripts; os contextos BMS têm globals próprias (um por entidade),
/// portanto o valor viaja por INSTALADOR de contexto (o `bevy_ext_luau`
/// corre-o em cada contexto novo; os valores vivem num static RwLock que
/// o cenário actualiza — re-aplicar um cenário muda os globals a nascer).
fn seed_qa_globals(plan: &Plan) {
    static QA_GLOBALS: std::sync::RwLock<Option<(bool, bool)>> = std::sync::RwLock::new(None);
    if let Ok(mut guard) = QA_GLOBALS.write() {
        *guard = Some((plan.god, plan.dev));
    }
    bevy_ext_luau::add_context_installer(|lua| {
        let Some((god, dev)) = QA_GLOBALS.read().ok().and_then(|guard| *guard) else {
            return Ok(());
        };
        let _ = lua.globals().set("vael_qa_god", god);
        if dev {
            let _ = lua.globals().set("vael_dev", true);
        }
        Ok(())
    });
}

fn apply_kv(lua: &mlua::Lua, plan: &Plan) {
    let current = vael_engine::luau::game::game_to_json(lua);
    vael_engine::luau::game::apply_patch(lua, &reset_patch(&current, &plan.kv));
    let globals = lua.globals();
    let _ = globals.set("vael_qa_god", plan.god);
    if plan.dev {
        let _ = globals.set("vael_dev", true);
    }
}

/// Ponto à frente da entidade `name` (exacto; senão a primeira que o contém).
fn find_near(world: &mut World, name: &str, distance: f32) -> Option<Vec3> {
    let mut query = world.query::<(&Name, &GlobalTransform)>();
    let mut fallback = None;
    for (entity_name, transform) in query.iter(world) {
        let entity_name = entity_name.as_str();
        if entity_name == name {
            return Some(in_front(transform, distance));
        }
        if fallback.is_none() && entity_name.contains(name) {
            fallback = Some(in_front(transform, distance));
        }
    }
    fallback
}

fn in_front(transform: &GlobalTransform, distance: f32) -> Vec3 {
    let (_, rotation, translation) = transform.to_scale_rotation_translation();
    let mut forward = rotation * Vec3::Z;
    forward.y = 0.0;
    let forward = forward.try_normalize().unwrap_or(Vec3::Z);
    translation + forward * distance + Vec3::Y * 0.5
}

/// `vael.vael.qa_*`.
fn install_qa_api(
    content: Option<Res<VaelContent>>,
    shared: Res<QaShared>,
    root: Option<Res<crate::VaelProjectRoot>>,
) {
    let Some(content) = content else {
        return;
    };
    let rules = Arc::new(QaRules::from_content(&content));
    let root = root
        .map(|root| root.0.clone())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let shared = shared.clone();
    crate::script_host::register_installer(move |lua| {
        if let Err(err) = install(lua, rules.clone(), shared.clone(), root.clone()) {
            error!("vael qa: falha a instalar `vael.vael.qa_*`: {err}");
        }
        Ok(())
    });
}

fn install(
    lua: &mlua::Lua,
    rules: Arc<QaRules>,
    shared: QaShared,
    root: PathBuf,
) -> mlua::Result<()> {
    use crate::vault::{vael_fn, vael_table};
    let vael = vael_table(lua)?;

    vael_fn!(
        lua,
        vael,
        "qa_name",
        [shared = shared.clone()],
        move |_, ()| {
            Ok(shared
                .current
                .lock()
                .unwrap()
                .as_ref()
                .map(|loaded| loaded.name.clone()))
        }
    );

    vael_fn!(
        lua,
        vael,
        "qa_apply",
        [shared = shared.clone()],
        move |_, name: Option<String>| {
            shared.inbox.lock().unwrap().apply = Some(name.filter(|name| !name.is_empty()));
            Ok(true)
        }
    );

    // `qa_check()` = o `expect` final do cenário; `qa_check(json)` = outro.
    vael_fn!(
        lua,
        vael,
        "qa_check",
        [rules = Arc::clone(&rules), shared = shared.clone()],
        move |lua, raw: Option<String>| {
            let expect = match raw.filter(|raw| !raw.trim().is_empty()) {
                Some(raw) => serde_json::from_str::<Expect>(&raw)
                    .map_err(|err| mlua::Error::runtime(format!("expect inválido: {err}")))?,
                None => shared
                    .current
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(|loaded| loaded.scenario.expect.clone())
                    .unwrap_or_default(),
            };
            let kv = vael_engine::luau::game::game_to_json(lua);
            let failures = check(&rules, &kv, &expect);
            let out = lua.create_table()?;
            out.set("ok", failures.is_empty())?;
            out.set("failures", failures)?;
            Ok(out)
        }
    );

    vael_fn!(
        lua,
        vael,
        "qa_snapshot",
        [rules = Arc::clone(&rules)],
        move |lua, ()| {
            let kv = vael_engine::luau::game::game_to_json(lua);
            Ok(serde_json::to_string_pretty(&snapshot(&rules, &kv)).unwrap_or_default())
        }
    );

    vael_fn!(
        lua,
        vael,
        "qa_plan",
        [rules = Arc::clone(&rules), root = root.clone()],
        move |_, name: String| {
            let loaded = load_scenario(&root, &name).map_err(mlua::Error::runtime)?;
            let plan = resolve(&rules, &loaded.scenario);
            Ok(serde_json::to_string_pretty(&json!({
                "kv": plan.kv,
                "pos": plan.pos,
                "near": plan.near,
                "warnings": plan.warnings,
            }))
            .unwrap_or_default())
        }
    );
    Ok(())
}

// ── CLI (`vael --qa-sim`) ───────────────────────────────────────────────

/// Simula `name` (ou `all`) e imprime um relatório; devolve o exit code.
pub fn run_cli(root: &Path, name: &str, as_json: bool) -> i32 {
    let content = VaelContent::load(root);
    let rules = QaRules::from_content(&content);
    let names = if name == "all" {
        list_scenarios(root)
            .into_iter()
            .filter(|name| !name.starts_with('_') && !name.contains("/_"))
            .collect()
    } else {
        vec![name.to_string()]
    };
    let mut failed = 0;
    let mut reports = Vec::new();
    for name in &names {
        let report = match load_scenario(root, name) {
            Ok(loaded) => {
                let mut report = simulate(&rules, &loaded.name, &loaded.scenario);
                let lint = lint(root, &rules, &loaded.scenario);
                report.warnings = lint;
                report
            }
            Err(err) => SimReport {
                name: name.clone(),
                ok: false,
                warnings: Vec::new(),
                steps: Vec::new(),
                failures: vec![err],
            },
        };
        if !report.ok {
            failed += 1;
        }
        reports.push(report);
    }
    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&reports).unwrap_or_default()
        );
    } else {
        for report in &reports {
            println!(
                "{} {}",
                if report.ok { "PASS" } else { "FAIL" },
                report.name
            );
            for warning in &report.warnings {
                println!("  aviso: {warning}");
            }
            for step in &report.steps {
                let mark = match step.status {
                    StepStatus::Ok => "ok  ",
                    StepStatus::Fail => "FAIL",
                    StepStatus::Skipped => "vivo",
                };
                println!("  {mark} {} {}", step.step, step.detail);
            }
            for failure in &report.failures {
                println!("  ✗ {failure}");
            }
        }
        println!("{} cenário(s), {failed} a falhar", reports.len());
    }
    i32::from(failed > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{Objective, Reward};
    use crate::quest::QuestEntry;

    fn obj(kind: &str, target: &str, count: i64) -> Objective {
        Objective {
            objective_type: kind.into(),
            target: target.into(),
            count,
            ..Default::default()
        }
    }

    fn rules() -> QaRules {
        let mut quests = BTreeMap::new();
        let mut add = |id: &str, entry: QuestEntry| {
            quests.insert(
                id.to_string(),
                QuestEntry {
                    id: id.into(),
                    ..entry
                },
            );
        };
        add(
            "a",
            QuestEntry {
                objective: obj("kill", "wolf", 2),
                next: "b".into(),
                rewards: Reward {
                    gold: 10,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        add(
            "b",
            QuestEntry {
                objective: obj("craft", "cloak", 1),
                stages: vec![obj("collect", "pelt", 3), obj("craft", "cloak", 1)],
                ..Default::default()
            },
        );
        add(
            "c",
            QuestEntry {
                objective: obj("visit", "tower", 1),
                requires: vec!["b".into()],
                ..Default::default()
            },
        );
        let order = quests.keys().cloned().collect();
        QaRules {
            quests: QuestRules { quests, order },
            starting: BTreeMap::from([("gold".into(), 30)]),
            items: ["gold", "pelt", "cloak", "sword"].map(String::from).into(),
            slots: BTreeMap::from([("sword".into(), "weapon".into())]),
            skills: BTreeMap::from([("s1".into(), vec![]), ("s2".into(), vec!["s1".into()])]),
            vault: VaultRules::from_items(&crate::content::ItemsFile {
                stack_max: 99,
                resource_cap: 999,
                ..Default::default()
            }),
            xp: XpRules::default(),
            craft: CraftRules::from_content(
                &crate::content::ItemsFile {
                    stack_max: 99,
                    resource_cap: 999,
                    ..Default::default()
                },
                &crate::content::RecipesFile {
                    version: 1,
                    recipes: vec![crate::content::RecipeDef {
                        id: "cloak".into(),
                        out: "cloak".into(),
                        count: 1,
                        cost: BTreeMap::from([("pelt".into(), 3)]),
                        extra: Default::default(),
                    }],
                    extra: Default::default(),
                },
            ),
            landmarks: BTreeSet::from(["tower".to_string()]),
        }
    }

    #[test]
    fn pre_condicoes_entregam_a_cadeia_por_requires_e_next() {
        let r = rules();
        assert_eq!(ancestors(&r.quests, "c"), vec!["a", "b"]);
        let plan = resolve(
            &r,
            &Scenario {
                quest: Some("c".into()),
                ..Default::default()
            },
        );
        assert_eq!(plan.kv["q.a"], json!("done"));
        assert_eq!(plan.kv["q.b"], json!("done"));
        assert_eq!(plan.kv["q.c"], json!("active"));
        assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);
    }

    #[test]
    fn etapa_e_ready_de_collect_trazem_os_itens() {
        let r = rules();
        let stage2 = resolve(
            &r,
            &Scenario {
                quest: Some("b".into()),
                stage: Some(2),
                ..Default::default()
            },
        );
        assert_eq!(stage2.kv["q.b.stage"], json!(2));
        assert_eq!(stage2.kv["q.b"], json!("active"));
        let mut kv = stage2.kv.clone();
        let craft = Step::Craft(ItemStep::Id("cloak".into()));
        let err = run_step(&r, &mut kv, &craft).unwrap_err();
        assert!(
            err.contains("pelt"),
            "a forja sem materiais diz o que falta: {err}"
        );
        kv.insert("item.pelt".into(), json!(3));
        assert!(run_step(&r, &mut kv, &craft).is_ok());
        assert_eq!(kv["item.pelt"], json!(0), "a receita gasta os custos");
        assert_eq!(quest_state(&r.quests, &kv, "b"), "ready");
    }

    #[test]
    fn simulacao_do_ciclo_completo_passa_e_falha_com_expect_errado() {
        let r = rules();
        let text = r#"{
            "quest": "a", "state": "not_taken",
            "steps": [
                {"accept": "a"},
                {"kill": {"kind": "wolf", "count": 2}},
                {"expect": {"quests": {"a": "ready"}}},
                {"turn_in": "a"},
                {"accept": "b"},
                {"collect": {"item": "pelt", "count": 3}},
                {"expect": {"stage": {"b": 2}}},
                {"key": "e"}
            ],
            "expect": {"quests": {"a": "done", "b": "active"}, "items": {"gold": ">=40"}}
        }"#;
        let scenario: Scenario = serde_json::from_str(text).unwrap();
        let report = simulate(&r, "t", &scenario);
        assert!(report.ok, "{:?}", report.failures);
        assert_eq!(report.steps.last().unwrap().status, StepStatus::Skipped);

        let mut wrong = scenario.clone();
        wrong.expect.quests.insert("b".into(), "done".into());
        assert!(!simulate(&r, "t", &wrong).ok);
    }

    #[test]
    fn talentos_trazem_requires_e_equipamento_entra_na_mochila() {
        let r = rules();
        let plan = resolve(
            &r,
            &Scenario {
                skills: vec!["s2".into()],
                gear: BTreeMap::from([("weapon".into(), "sword".into())]),
                level: Some(3),
                ..Default::default()
            },
        );
        assert_eq!(plan.kv["skill.s1"], json!(true));
        assert_eq!(plan.kv["skill.s2"], json!(true));
        assert_eq!(plan.kv["item.sword"], json!(1));
        assert_eq!(plan.kv["gear.weapon"], json!("sword"));
        assert_eq!(plan.kv["level"], json!(3));
        assert_eq!(plan.kv["item.gold"], json!(30), "kit por baixo");
    }

    #[test]
    fn extends_junta_por_ordem_e_o_filho_ganha() {
        let base = Scenario {
            items: BTreeMap::from([("pelt".into(), 1), ("gold".into(), 5)]),
            skills: vec!["s1".into()],
            hour: Some(12.0),
            ..Default::default()
        };
        let child = Scenario {
            items: BTreeMap::from([("pelt".into(), 3)]),
            skills: vec!["s2".into()],
            ..Default::default()
        };
        let merged = base.merge(child);
        assert_eq!(merged.items["pelt"], 3);
        assert_eq!(merged.items["gold"], 5);
        assert_eq!(merged.skills, vec!["s1", "s2"]);
        assert_eq!(merged.hour, Some(12.0));
    }

    #[test]
    fn comparacoes_e_campos_desconhecidos() {
        assert_eq!(Cmp::Expr(">=3".into()).eval(3.0), Ok(true));
        assert_eq!(Cmp::Expr("<3".into()).eval(3.0), Ok(false));
        assert_eq!(Cmp::Expr("!= 0".into()).eval(1.0), Ok(true));
        assert_eq!(Cmp::Exact(2).eval(2.0), Ok(true));
        assert!(Cmp::Expr("abc".into()).eval(1.0).is_err());
        assert!(serde_json::from_str::<Scenario>(r#"{"itens": {}}"#).is_err());
        assert!(serde_json::from_str::<Scenario>(r#"{"steps": [{"voar": 1}]}"#).is_err());
    }

    #[test]
    fn reaplicar_repoe_o_estado_do_jogador() {
        let current: QuestKv = [
            ("item.cloak", json!(3)),
            ("q.a", json!("done")),
            ("skill.s9", json!(true)),
            ("lang", json!("pt")),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        let plan: QuestKv = [("q.a".to_string(), json!("active"))].into();
        let patch = reset_patch(&current, &plan);
        assert_eq!(patch["q.a"], json!("active"));
        assert_eq!(patch["item.cloak"], Value::Null);
        assert_eq!(patch["skill.s9"], Value::Null);
        assert!(!patch.contains_key("lang"), "preferências ficam");
    }

    #[test]
    fn instantaneo_reproduz_o_estado() {
        let r = rules();
        let plan = resolve(
            &r,
            &Scenario {
                quest: Some("b".into()),
                stage: Some(2),
                skills: vec!["s1".into()],
                ..Default::default()
            },
        );
        let snap = snapshot(&r, &plan.kv);
        let again = resolve(&r, &snap);
        for key in ["q.a", "q.b", "q.b.stage", "skill.s1", "item.gold", "level"] {
            assert_eq!(plan.kv.get(key), again.kv.get(key), "{key}");
        }
    }
}

//! Eventos de quest em Bevy (`report_kill`/`scan_visits`/`sync_collect`).
//!
//! As funções puras trabalham sobre [`QuestKv`] (o `viber.game()`) e reutilizam
//! o [`QuestRules`] e o `quest_set_progress` do [`crate::quest`] — nenhuma
//! regra de aceitar/entregar é duplicada aqui. O Lua (`scripts/lib/rules.lua`)
//! é a fonte de verdade por omissão; com `VAEL_QUEST_EVENTS_RUST=1` os três
//! relatos passam a ser eventos para o Rust, que lê o `viber_game`, aplica as
//! funções puras e escreve só as chaves que mudaram.
//!
//! Regras portadas de `scripts/lib/rules.lua`:
//! - `report_kill` conta 1 em cada quest `active` de `type: "kill"` cujo alvo
//!   casa (`kind == target` ou o prefixo `target-`); `elite` conta também nos
//!   objetivos `target: "elite"`;
//! - `sync_collect` lê a quantidade do cofre (`item.<target>`), grava
//!   `min(have, need)` e promove a `ready`; uma quest `ready` que perdeu os
//!   itens volta a `active`;
//! - `scan_visits` marca `q.<id>.v.<marco>` quando o herói entra no raio do
//!   marco (por omissão 12 m) e conta os marcadores da etapa;
//! - `report_visit` (chamado pelo passeio/`[F]`/diálogo) continua no Lua; a
//!   varredura Rust replica só a parte de estado desse relato.
//!
//! Sem host Luau, sem conteúdo ou com o gate desligado nada é escrito — o
//! jogo fica exatamente como antes.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard};

use bevy::prelude::*;
use mlua::{Function, Lua, Table, Value};
use serde_json::Value as Json;
use viber::luau::components::ScriptToast;
use viber::luau::game::{apply_patch, game_to_json};
use viber::luau::host::LuaScriptHost;

use crate::compat::env_enabled;
use crate::components::{QuestLog, VaelPlayer};
use crate::content::{Objective, VaelContent};
use crate::quest::{
    QuestKv, QuestRules, STATE_ACTIVE, STATE_DONE, STATE_READY, quest_key, quest_objective,
    quest_progress, quest_set_progress, quest_state,
};
use crate::vault::{item_key, vael_fn, vael_table};

/// Variável de ambiente que liga os eventos de quest em Rust.
pub const RUST_ENV: &str = "VAEL_QUEST_EVENTS_RUST";
/// Chave do `viber.game()` que avisa o Lua de que os relatos são do Rust.
pub const RUST_GATE_KEY: &str = "vael_quest_events_rust";
/// Período da varredura de visitas (o `SCAN_PERIOD` do Lua).
pub const VISIT_PERIOD_SECS: f32 = 0.2;
/// Raio de um marco de visita quando o objetivo não traz um válido.
pub const VISIT_DEFAULT_RADIUS: f64 = 12.0;

// ── gate ────────────────────────────────────────────────────────────────

/// Gate dos eventos de quest, lido do ambiente no arranque.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct VaelQuestEventsConfig {
    pub rust: bool,
}

impl VaelQuestEventsConfig {
    /// Constrói o gate a partir do valor bruto da variável (`None` = ausente).
    pub fn from_raw(raw: Option<&str>) -> Self {
        Self {
            rust: env_enabled(raw),
        }
    }

    /// Lê `VAEL_QUEST_EVENTS_RUST` do ambiente.
    pub fn from_env() -> Self {
        Self::from_raw(std::env::var(RUST_ENV).ok().as_deref())
    }
}

impl Default for VaelQuestEventsConfig {
    /// Opt-in: só liga com `1`/`true`/`on`/`yes`.
    fn default() -> Self {
        Self::from_env()
    }
}

// ── fila de eventos ─────────────────────────────────────────────────────

/// Um relato vindo do Lua. Não devolve valor: o estado aparece no frame
/// seguinte, como o HUD/quests já esperam.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuestEvent {
    /// `report_kill(kind, elite)`.
    ReportKill { kind: String, elite: bool },
    /// `sync_collect()` depois de mexer no cofre.
    SyncCollect,
    /// Pedido de varredura de visitas (o sistema das visitas responde).
    ScanVisits,
}

/// Fila partilhada entre as closures de `viber.vael` e os sistemas Bevy.
#[derive(Resource, Debug, Clone, Default)]
pub struct QuestEventQueue(Arc<Mutex<Vec<QuestEvent>>>);

impl QuestEventQueue {
    fn lock(&self) -> MutexGuard<'_, Vec<QuestEvent>> {
        self.0.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    /// Acrescenta um evento. `SyncCollect`/`ScanVisits` são idempotentes: um
    /// pedido já pendente chega (o `rules.tick` chama o relato todos os
    /// frames e não pode encher a fila).
    pub fn push(&self, event: QuestEvent) {
        let mut queue = self.lock();
        if matches!(event, QuestEvent::SyncCollect | QuestEvent::ScanVisits)
            && queue.contains(&event)
        {
            return;
        }
        queue.push(event);
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    pub fn clear(&self) {
        self.lock().clear();
    }

    /// Retira os eventos de estado, deixando os pedidos de visita.
    pub fn take_non_visit(&self) -> Vec<QuestEvent> {
        let mut queue = self.lock();
        let mut out = Vec::new();
        let mut visits = Vec::new();
        for event in queue.drain(..) {
            match event {
                QuestEvent::ScanVisits => visits.push(event),
                other => out.push(other),
            }
        }
        *queue = visits;
        out
    }

    /// Consome os pedidos de visita e devolve quantos havia.
    pub fn take_visit_requests(&self) -> usize {
        let mut queue = self.lock();
        let before = queue.len();
        queue.retain(|event| !matches!(event, QuestEvent::ScanVisits));
        before - queue.len()
    }

    /// Tira tudo (testes/diagnóstico).
    pub fn drain_all(&self) -> Vec<QuestEvent> {
        self.lock().drain(..).collect()
    }
}

// ── regras puras ────────────────────────────────────────────────────────

fn json_int(value: i64) -> Json {
    Json::Number(value.into())
}

fn json_state(state: &str) -> Json {
    Json::String(state.to_string())
}

/// `true` só para o booleano `true` (o Lua compara com `== true`).
fn kv_true(kv: &QuestKv, key: &str) -> bool {
    matches!(kv.get(key), Some(Json::Bool(true)))
}

/// Chave do marcador de uma visita (`q.<id>.v.<marco>`).
fn visit_marker(id: &str, place: &str) -> String {
    format!("{}.v.{place}", quest_key(id))
}

/// Chave do progresso (`q.<id>.n`).
fn progress_key(id: &str) -> String {
    format!("{}.n", quest_key(id))
}

/// `kill_matches` do Lua: igual ou prefixo `target-`.
///
/// A substring ao contrário ("a" dentro de "bandit") contava abates a mais.
pub fn kind_matches(kind: &str, target: &str) -> bool {
    if kind.is_empty() || target.is_empty() {
        return false;
    }
    kind == target || kind.starts_with(&format!("{target}-"))
}

/// `true` quando a quest tem um objetivo `kill` para este abate.
fn kill_objective(rules: &QuestRules, kv: &QuestKv, id: &str, kind: &str, elite: bool) -> bool {
    let view = quest_objective(rules, kv, id);
    view.objective_type == "kill"
        && (kind_matches(kind, &view.target) || (elite && view.target == "elite"))
}

/// Objetivo em curso (a etapa actual ou o objetivo único).
fn current_objective<'a>(rules: &'a QuestRules, id: &str, at: i64) -> Option<&'a Objective> {
    let def = rules.quests.get(id)?;
    if def.stages.is_empty() {
        return Some(&def.objective);
    }
    def.stages.get(at.saturating_sub(1).max(0) as usize)
}

/// Raio de uma visita: um valor inválido cai no default do Lua.
fn visit_radius(radius: f64) -> f64 {
    if radius.is_finite() && radius > 0.0 {
        radius
    } else {
        VISIT_DEFAULT_RADIUS
    }
}

/// `report_kill`: progresso dos objetivos `kill` da quest activa.
///
/// Devolve os ids que mudaram (ordem de [`QuestRules::order`]).
pub fn report_kill(rules: &QuestRules, kv: &mut QuestKv, kind: &str, elite: bool) -> Vec<String> {
    if kind.is_empty() {
        return Vec::new();
    }
    let mut changed = Vec::new();
    for id in &rules.order {
        if quest_state(rules, kv, id) != STATE_ACTIVE || !kill_objective(rules, kv, id, kind, elite)
        {
            continue;
        }
        let (have, need) = quest_progress(rules, kv, id);
        let next = have.saturating_add(1).min(need);
        if quest_set_progress(rules, kv, id, next as f64).ok {
            changed.push(id.clone());
        }
    }
    changed
}

/// Quantidade de um item no cofre (`item.<kind>`), nunca negativa.
fn item_count(kv: &QuestKv, kind: &str) -> i64 {
    if kind.is_empty() {
        return 0;
    }
    match kv.get(&item_key(kind)) {
        Some(Json::Number(number)) => number
            .as_f64()
            .map(|value| value.max(0.0) as i64)
            .unwrap_or(0),
        _ => 0,
    }
}

/// Quantidades do cofre para os alvos dos objetivos `collect`.
pub fn collect_quantities(rules: &QuestRules, kv: &QuestKv) -> BTreeMap<String, i64> {
    let mut out = BTreeMap::new();
    for id in &rules.order {
        for objective in rules
            .quests
            .get(id)
            .into_iter()
            .flat_map(|def| std::iter::once(&def.objective).chain(def.stages.iter()))
        {
            if objective.objective_type != "collect" || objective.target.is_empty() {
                continue;
            }
            out.entry(objective.target.clone())
                .or_insert_with(|| item_count(kv, &objective.target));
        }
    }
    out
}

/// `sync_collect`: espelha o cofre nos objetivos `collect`.
///
/// `vault` é o mapa `alvo -> quantidade` (ver [`collect_quantities`]).
pub fn sync_collect(
    rules: &QuestRules,
    kv: &mut QuestKv,
    vault: &BTreeMap<String, i64>,
) -> Vec<String> {
    let mut changed = Vec::new();
    for id in &rules.order {
        let state = quest_state(rules, kv, id);
        if state != STATE_ACTIVE && state != STATE_READY {
            continue;
        }
        let view = quest_objective(rules, kv, id);
        if view.objective_type != "collect" {
            continue;
        }
        let (have, need) = quest_progress(rules, kv, id);
        let amount = vault.get(&view.target).copied().unwrap_or(0).max(0);
        let next = amount.min(need);
        if state == STATE_ACTIVE {
            // `set_progress` + `mark_ready`, sem duplicar a promoção.
            if quest_set_progress(rules, kv, id, next as f64).ok {
                changed.push(id.clone());
            }
            continue;
        }
        // `ready`: o Lua grava o progresso e só volta a `active` quando o
        // cofre perdeu o que a entrega pede (`mark_ready` ignora `ready`).
        if next != have {
            kv.insert(progress_key(id), json_int(next));
            changed.push(id.clone());
        }
        if amount < need {
            kv.insert(quest_key(id), json_state(STATE_ACTIVE));
            if !changed.last().is_some_and(|last| last == id) {
                changed.push(id.clone());
            }
        }
    }
    changed
}

/// `report_visit` (parte de estado): marca o marco e conta os marcadores.
///
/// O Lua continua dono do registo de ações e do toast; esta função é a mesma
/// aritmética que o `M.report_visit` faz sobre as quests.
pub fn report_visit(rules: &QuestRules, kv: &mut QuestKv, place: &str) -> Vec<String> {
    if place.is_empty() {
        return Vec::new();
    }
    let mut changed = Vec::new();
    for id in &rules.order {
        if quest_state(rules, kv, id) != STATE_ACTIVE {
            continue;
        }
        let view = quest_objective(rules, kv, id);
        if view.objective_type != "visit" {
            continue;
        }
        let tokens: Vec<&str> = view.target.split_whitespace().collect();
        if !tokens.contains(&place) {
            continue;
        }
        let marker = visit_marker(id, place);
        if kv_true(kv, &marker) {
            continue;
        }
        kv.insert(marker, Json::Bool(true));
        let marked = tokens
            .iter()
            .filter(|token| kv_true(kv, &visit_marker(id, token)))
            .count() as i64;
        let (_, need) = quest_progress(rules, kv, id);
        let next = marked.min(need);
        if quest_set_progress(rules, kv, id, next as f64).ok {
            changed.push(id.clone());
        }
    }
    changed
}

/// Um marco visitado pela varredura (para o toast e o registo de ações).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisitHit {
    pub place: String,
    /// Quests que creditaram a visita.
    pub quests: Vec<String>,
}

/// Marcos ainda por visitar nas quests `active` (tokens dos objetivos).
pub fn pending_visit_tokens(rules: &QuestRules, kv: &QuestKv) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for id in &rules.order {
        if quest_state(rules, kv, id) != STATE_ACTIVE {
            continue;
        }
        let view = quest_objective(rules, kv, id);
        if view.objective_type != "visit" {
            continue;
        }
        for token in view.target.split_whitespace() {
            if !kv_true(kv, &visit_marker(id, token)) {
                out.insert(token.to_string());
            }
        }
    }
    out
}

/// `scan_visits`: marca os marcos dentro do raio do herói (plano XZ).
///
/// `positions` tem de trazer o nome exacto do marco; quem chama resolve os
/// tokens com [`resolve_positions`] (o `viber.find` exacto/substring).
pub fn scan_visits(
    rules: &QuestRules,
    kv: &mut QuestKv,
    player: [f64; 2],
    positions: &BTreeMap<String, [f64; 2]>,
) -> Vec<VisitHit> {
    let mut hits = Vec::new();
    for id in &rules.order {
        if quest_state(rules, kv, id) != STATE_ACTIVE {
            continue;
        }
        let view = quest_objective(rules, kv, id);
        if view.objective_type != "visit" {
            continue;
        }
        let radius = current_objective(rules, id, view.at)
            .map(|objective| visit_radius(objective.radius))
            .unwrap_or(VISIT_DEFAULT_RADIUS);
        for token in view.target.split_whitespace() {
            if kv_true(kv, &visit_marker(id, token)) {
                continue;
            }
            let Some(position) = positions.get(token) else {
                continue;
            };
            let dx = position[0] - player[0];
            let dz = position[1] - player[1];
            if dx * dx + dz * dz > radius * radius {
                continue;
            }
            let quests = report_visit(rules, kv, token);
            if !quests.is_empty() {
                hits.push(VisitHit {
                    place: token.to_string(),
                    quests,
                });
            }
        }
    }
    hits
}

/// Resolve os tokens de visita em posições, com a semântica do `viber.find`:
/// nome exacto primeiro; depois a chave lexicograficamente menor que o contém.
///
/// Devolve as posições e os tokens sem nenhuma entidade no mundo.
pub fn resolve_positions(
    names: &[(String, [f64; 2])],
    tokens: &BTreeSet<String>,
) -> (BTreeMap<String, [f64; 2]>, Vec<String>) {
    let mut positions = BTreeMap::new();
    for (name, position) in names {
        if tokens.contains(name.as_str()) {
            positions.entry(name.clone()).or_insert(*position);
        }
    }
    let mut missing = Vec::new();
    for token in tokens {
        if positions.contains_key(token) {
            continue;
        }
        let mut best: Option<(&str, [f64; 2])> = None;
        for (name, position) in names {
            if !name.contains(token.as_str()) {
                continue;
            }
            if best.is_none_or(|(best_name, _)| name.as_str() < best_name) {
                best = Some((name.as_str(), *position));
            }
        }
        match best {
            Some((_, position)) => {
                positions.insert(token.clone(), position);
            }
            None => missing.push(token.clone()),
        }
    }
    (positions, missing)
}

/// Patch com as chaves que mudaram (`Null` remove as que desapareceram).
pub fn changed_patch(before: &QuestKv, after: &QuestKv) -> QuestKv {
    let mut patch = QuestKv::new();
    for (key, value) in after {
        if before.get(key) != Some(value) {
            patch.insert(key.clone(), value.clone());
        }
    }
    for key in before.keys() {
        if !after.contains_key(key) {
            patch.insert(key.clone(), Json::Null);
        }
    }
    patch
}

/// Actualiza o espelho [`QuestLog`] a partir do estado `q.<id>` final.
///
/// `before` serve só para remover estados que desapareceram do KV; os
/// presentes são reescritos (o espelho pode estar um frame atrasado).
pub fn apply_quest_mirror(before: &QuestKv, after: &QuestKv, quests: &mut QuestLog) {
    for (key, value) in after {
        let Some(id) = key.strip_prefix("q.") else {
            continue;
        };
        if id.is_empty() || id.contains('.') {
            continue;
        }
        match value {
            Json::String(state) => {
                quests.states.insert(id.to_string(), state.clone());
            }
            Json::Null => {
                quests.states.remove(id);
            }
            _ => {}
        }
    }
    for key in before.keys() {
        let Some(id) = key.strip_prefix("q.") else {
            continue;
        };
        if id.is_empty() || id.contains('.') || after.contains_key(key) {
            continue;
        }
        quests.states.remove(id);
    }
    quests.done = quests
        .states
        .iter()
        .filter(|(_, state)| state.as_str() == STATE_DONE)
        .map(|(id, _)| id.clone())
        .collect();
}

/// Aviso que o Lua dava no `mark_ready`: etapa cumprida ou objetivo completo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuestNotice {
    /// A quest passou a `ready`.
    Complete { title: String },
    /// A etapa `stage`/`total` foi cumprida e a seguinte começou.
    Stage {
        title: String,
        stage: i64,
        total: i64,
    },
}

/// Lê `q.<id>.stage` (ausente = 0, para distinguir "etapa 1 implícita").
fn stage_of(kv: &QuestKv, id: &str) -> i64 {
    match kv.get(&format!("{}.stage", quest_key(id))) {
        Some(Json::Number(number)) => number.as_i64().unwrap_or(0),
        _ => 0,
    }
}

fn quest_title(rules: &QuestRules, id: &str) -> String {
    let def = rules.quests.get(id);
    def.map(|def| def.title.clone())
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| id.to_string())
}

/// Avisos de progresso entre duas leituras do KV (a parte visível do
/// `mark_ready`; o estado já foi escrito).
pub fn quest_notices(rules: &QuestRules, before: &QuestKv, after: &QuestKv) -> Vec<QuestNotice> {
    let mut out = Vec::new();
    for id in &rules.order {
        let was = quest_state(rules, before, id);
        let now = quest_state(rules, after, id);
        if was != STATE_ACTIVE {
            continue;
        }
        let title = quest_title(rules, id);
        if now == STATE_READY {
            out.push(QuestNotice::Complete { title });
            continue;
        }
        if now != STATE_ACTIVE {
            continue;
        }
        let before_stage = stage_of(before, id);
        let after_stage = stage_of(after, id);
        if after_stage > before_stage {
            let total = rules
                .quests
                .get(id)
                .map(|def| def.stages.len() as i64)
                .unwrap_or(0);
            // Uma chamada pode fechar mais do que uma etapa (o `mark_ready`
            // do Lua avisa por etapa); a 1.ª é implícita (`.stage` ausente).
            for stage in before_stage.max(1)..after_stage {
                out.push(QuestNotice::Stage {
                    title: title.clone(),
                    stage,
                    total,
                });
            }
        }
    }
    out
}

/// Toast + som do `mark_ready` (best-effort; sem HUD/Lua não faz nada).
fn emit_notices(
    lua: &Lua,
    notices: &[QuestNotice],
    toasts: &mut Option<MessageWriter<ScriptToast>>,
    position: Option<[f32; 3]>,
) {
    for notice in notices {
        let text = match notice {
            QuestNotice::Complete { title } => format!("Objetivo completo: {title}"),
            QuestNotice::Stage {
                title,
                stage,
                total,
            } => format!("Etapa {stage}/{total} cumprida: {title}"),
        };
        if let Some(toasts) = toasts.as_mut() {
            toasts.write(ScriptToast(text));
        }
        play_notification(lua, position);
    }
}

/// `sound_at_player("notification")` do Lua, do lado Rust.
fn play_notification(lua: &Lua, position: Option<[f32; 3]>) {
    let result = (|| -> mlua::Result<()> {
        let viber: Table = lua.globals().get("viber")?;
        let Some(sound) = viber.get::<Option<Function>>("sound")? else {
            return Ok(());
        };
        if let Some([x, y, z]) = position {
            let opts = lua.create_table()?;
            opts.set("x", x)?;
            opts.set("y", y)?;
            opts.set("z", z)?;
            sound.call::<()>(("notification", opts))
        } else {
            sound.call::<()>("notification")
        }
    })();
    if let Err(err) = result {
        debug!("vael: som de quest indisponível: {err}");
    }
}

// ── sistemas ────────────────────────────────────────────────────────────

/// Plugin dos eventos de quest: bridge no `PostStartup` (depois de todos os
/// instaladores de `Startup`), drenagem e visitas no `Update` depois do
/// `luau_update`.
pub struct VaelQuestEventsPlugin;

impl Plugin for VaelQuestEventsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VaelQuestEventsConfig>()
            .init_resource::<QuestEventQueue>()
            // `PostStartup` corre depois de TODOS os `Startup` (economia,
            // viagem): a tabela `viber.vael` já tem as funções dos outros
            // módulos e não há escritas concorrentes na mesma tabela Lua.
            .add_systems(PostStartup, install_quest_events_api)
            // Um save antigo pode trazer o gate a `true`; corrige-se antes
            // dos scripts do frame lerem a flag.
            .add_systems(
                PreUpdate,
                enforce_quest_events_gate.run_if(viber::app::scripts_enabled),
            )
            .add_systems(
                Update,
                (drain_quest_events, scan_visits_system)
                    .chain()
                    // Os sistemas leem o que o Lua acabou de fazer no frame e
                    // escrevem o estado de voltar ao `viber.game()`.
                    .after(viber::luau::luau_update)
                    .run_if(viber::app::scripts_enabled),
            );
    }
}

/// Instala `viber.vael.quest_report_kill`/`quest_sync_collect`/
/// `quest_scan_visits` e o gate do arranque.
pub(crate) fn install_quest_events_api(
    host: Option<Res<LuaScriptHost>>,
    queue: Res<QuestEventQueue>,
    config: Res<VaelQuestEventsConfig>,
) {
    let Some(host) = host else {
        warn!("vael: eventos de quest sem host Luau — `viber.vael` não estendido");
        return;
    };
    match install(&host.lua, &queue) {
        Ok(()) => info!(
            rust = config.rust,
            "vael: eventos de quest em Rust ligados (`viber.vael`)"
        ),
        Err(err) => {
            error!("vael: falha a instalar eventos de quest em `viber.vael`: {err}");
            // Sem as funções, ligar o gate faria o Lua saltar os relatos e
            // ficar sem alternativa: fica no caminho antigo.
            return;
        }
    }
    let kv = game_to_json(&host.lua);
    let mut patch = QuestKv::new();
    gate_patch(&kv, config.rust, &mut patch);
    if !patch.is_empty() {
        apply_patch(&host.lua, &patch);
    }
}

/// Acrescenta as três funções à tabela `viber.vael` existente.
pub(crate) fn install(lua: &Lua, queue: &QuestEventQueue) -> mlua::Result<()> {
    let vael = vael_table(lua)?;

    vael_fn!(
        lua,
        vael,
        "quest_report_kill",
        [queue = queue.clone()],
        move |_, (kind, elite): (Option<String>, Option<bool>)| {
            let kind = kind.unwrap_or_default();
            if kind.is_empty() {
                return Ok(());
            }
            queue.push(QuestEvent::ReportKill {
                kind,
                elite: elite.unwrap_or(false),
            });
            Ok(())
        }
    );

    vael_fn!(
        lua,
        vael,
        "quest_sync_collect",
        [queue = queue.clone()],
        move |_, ()| {
            queue.push(QuestEvent::SyncCollect);
            Ok(())
        }
    );

    vael_fn!(
        lua,
        vael,
        "quest_scan_visits",
        [queue = queue.clone()],
        move |_, ()| {
            queue.push(QuestEvent::ScanVisits);
            Ok(())
        }
    );

    Ok(())
}

/// Preenche o patch que acerta o gate guardado no save.
fn gate_patch(kv: &QuestKv, rust: bool, patch: &mut QuestKv) {
    let actual = kv.get(RUST_GATE_KEY);
    if rust {
        if actual != Some(&Json::Bool(true)) {
            patch.insert(RUST_GATE_KEY.to_string(), Json::Bool(true));
        }
    } else if actual == Some(&Json::Bool(true)) {
        // Save escrito com o gate ligado numa sessão normal tem de voltar ao
        // caminho do Lua (o mesmo tratamento do `vael_rust_vitals`).
        patch.insert(RUST_GATE_KEY.to_string(), Json::Null);
    }
}

/// Acerta o gate e deixa o KV local igual ao que ficou no Lua.
fn ensure_gate(lua: &Lua, kv: &mut QuestKv, rust: bool) {
    let mut patch = QuestKv::new();
    gate_patch(kv, rust, &mut patch);
    if patch.is_empty() {
        return;
    }
    apply_patch(lua, &patch);
    for (key, value) in patch {
        match value {
            Json::Null => {
                kv.remove(&key);
            }
            other => {
                kv.insert(key, other);
            }
        }
    }
}

/// Mantém o gate do save igual ao modo da sessão, antes dos scripts.
///
/// Lê só a chave (não clona o jogo): um save escrito com o gate ligado não
/// pode deixar o Lua a saltar os relatos numa sessão sem Rust.
fn enforce_quest_events_gate(config: Res<VaelQuestEventsConfig>, host: Option<Res<LuaScriptHost>>) {
    let Some(host) = host else {
        return;
    };
    let Ok(game) = host.lua.named_registry_value::<Table>("viber_game") else {
        return;
    };
    let current = matches!(
        game.raw_get::<Value>(RUST_GATE_KEY),
        Ok(Value::Boolean(true))
    );
    if config.rust == current {
        return;
    }
    if config.rust {
        let _ = game.raw_set(RUST_GATE_KEY, true);
    } else {
        let _ = game.raw_remove(RUST_GATE_KEY);
    }
}

/// Drena os relatos de abate/recolha e aplica-os ao `viber.game()`.
fn drain_quest_events(
    config: Res<VaelQuestEventsConfig>,
    queue: Res<QuestEventQueue>,
    host: Option<Res<LuaScriptHost>>,
    content: Option<Res<VaelContent>>,
    player: Query<&GlobalTransform, Or<(With<viber::player::Player>, With<VaelPlayer>)>>,
    mut toasts: Option<MessageWriter<ScriptToast>>,
    mut quests: Option<ResMut<QuestLog>>,
    // Regras construídas uma vez por carregamento de conteúdo, não por lote.
    mut rules_cache: Local<Option<QuestRules>>,
) {
    let events = queue.take_non_visit();
    if events.is_empty() {
        return;
    }
    let Some(host) = host else {
        return;
    };
    if !config.rust {
        return;
    }
    // `before` é também o KV que as funções puras alteram.
    let before = game_to_json(&host.lua);
    if before.is_empty() {
        // A tabela ainda não existe (arranque): não zerar nem escrever.
        return;
    }
    let Some(content) = content else {
        return;
    };
    if content.is_changed() || rules_cache.is_none() {
        *rules_cache = Some(QuestRules::from_content(&content));
    }
    let rules = rules_cache
        .as_ref()
        .expect("cache acabou de ser preenchido");
    let mut kv = before.clone();
    for event in events {
        match event {
            QuestEvent::ReportKill { kind, elite } => {
                report_kill(rules, &mut kv, &kind, elite);
            }
            QuestEvent::SyncCollect => {
                let vault = collect_quantities(rules, &kv);
                sync_collect(rules, &mut kv, &vault);
            }
            // Consumido pelo sistema das visitas.
            QuestEvent::ScanVisits => {}
        }
    }
    let patch = changed_patch(&before, &kv);
    if patch.is_empty() {
        return;
    }
    let notices = quest_notices(rules, &before, &kv);
    apply_patch(&host.lua, &patch);
    if let Some(quests) = quests.as_deref_mut() {
        apply_quest_mirror(&before, &kv, quests);
    }
    if !notices.is_empty() {
        let position = player.iter().next().map(|transform| {
            let position = transform.translation();
            [position.x, position.y, position.z]
        });
        emit_notices(&host.lua, &notices, &mut toasts, position);
    }
}

/// Sistema das visitas: timer de 0,2 s + pedidos vindos do `rules.tick`.
#[allow(clippy::too_many_arguments)]
fn scan_visits_system(
    time: Res<Time>,
    config: Res<VaelQuestEventsConfig>,
    queue: Res<QuestEventQueue>,
    host: Option<Res<LuaScriptHost>>,
    content: Option<Res<VaelContent>>,
    player: Query<&GlobalTransform, Or<(With<viber::player::Player>, With<VaelPlayer>)>>,
    names: Query<(&Name, &GlobalTransform)>,
    menus: Option<Res<bevy_ext_core::MenusOpen>>,
    mut toasts: Option<MessageWriter<ScriptToast>>,
    mut quests: Option<ResMut<QuestLog>>,
    mut timer: Local<Option<Timer>>,
    mut logged_missing: Local<BTreeSet<String>>,
    mut rules_cache: Local<Option<QuestRules>>,
) {
    let requested = queue.take_visit_requests();
    let timer =
        timer.get_or_insert_with(|| Timer::from_seconds(VISIT_PERIOD_SECS, TimerMode::Repeating));
    let due = timer.tick(time.delta()).just_finished();
    if !config.rust || (!due && requested == 0) {
        return;
    }
    // O Lua só varre com o menu fechado; o mapa aberto não conta visitas.
    if menus.as_ref().is_some_and(|menus| menus.any()) {
        return;
    }
    let (Some(host), Some(content)) = (host, content) else {
        return;
    };
    let mut before = game_to_json(&host.lua);
    if before.is_empty() {
        return;
    }
    ensure_gate(&host.lua, &mut before, true);
    if content.is_changed() || rules_cache.is_none() {
        *rules_cache = Some(QuestRules::from_content(&content));
    }
    let rules = rules_cache
        .as_ref()
        .expect("cache acabou de ser preenchido");
    let tokens = pending_visit_tokens(rules, &before);
    if tokens.is_empty() {
        return;
    }
    let Some(player) = player.iter().next() else {
        return;
    };
    let position = player.translation();
    let player = [position.x as f64, position.z as f64];
    // Só os nomes que podem resolver um token (exacto ou substring, como o
    // `resolve_positions`): num mundo com ~79k entidades, alocar uma `String`
    // por nome a 5 Hz era o grosso da varredura.
    let named: Vec<(String, [f64; 2])> = names
        .iter()
        .filter_map(|(name, transform)| {
            let name = name.as_str();
            let relevant =
                tokens.contains(name) || tokens.iter().any(|token| name.contains(token.as_str()));
            if !relevant {
                return None;
            }
            let position = transform.translation();
            Some((name.to_string(), [position.x as f64, position.z as f64]))
        })
        .collect();
    let (positions, missing) = resolve_positions(&named, &tokens);
    for token in &missing {
        if logged_missing.insert(token.clone()) {
            warn!("vael: marco de visita sem posição no mundo: {token}");
        }
    }
    if !missing.is_empty() && call_lua_scan(&host.lua) {
        // O Lua resolveu tudo (incluindo a semântica do `viber.find`).
        return;
    }
    let mut kv = before.clone();
    let hits = scan_visits(rules, &mut kv, player, &positions);
    if hits.is_empty() {
        return;
    }
    let patch = changed_patch(&before, &kv);
    if !patch.is_empty() {
        let notices = quest_notices(rules, &before, &kv);
        apply_patch(&host.lua, &patch);
        if let Some(quests) = quests.as_deref_mut() {
            apply_quest_mirror(&before, &kv, quests);
        }
        if !notices.is_empty() {
            emit_notices(
                &host.lua,
                &notices,
                &mut toasts,
                Some([position.x, position.y, position.z]),
            );
        }
    }
    for hit in &hits {
        let label = landmark_label(&content, &hit.place);
        if let Some(toasts) = toasts.as_mut() {
            toasts.write(ScriptToast(format!("Marco visto: {label}")));
        }
        record_visit_action(&host.lua, &hit.place);
    }
}

/// Rótulo autoral do marco (`data/landmarks.json`); sem entrada, o id.
fn landmark_label(content: &VaelContent, place: &str) -> String {
    content
        .landmarks
        .landmarks
        .iter()
        .find(|landmark| landmark.name == place)
        .map(|landmark| landmark.label.clone())
        .filter(|label| !label.is_empty())
        .unwrap_or_else(|| place.to_string())
}

/// Corrida de segurança quando nenhuma entidade tem o nome do marco: usa a
/// varredura do Lua (o `viber.find` também faz substring).
fn call_lua_scan(lua: &Lua) -> bool {
    let result = (|| -> mlua::Result<bool> {
        let viber: Table = lua.globals().get("viber")?;
        let load: Function = viber.get("load")?;
        let rules: Table = load.call("lib/rules.lua")?;
        let Some(scan) = rules.get::<Option<Function>>("scan_visits_lua")? else {
            return Ok(false);
        };
        scan.call::<()>(())?;
        Ok(true)
    })();
    match result {
        Ok(done) => done,
        Err(err) => {
            debug!("vael: fallback Lua das visitas indisponível: {err}");
            false
        }
    }
}

/// Registo da visita para o juiz (`lib/actions.lua`), best-effort.
fn record_visit_action(lua: &Lua, place: &str) {
    let result = (|| -> mlua::Result<()> {
        let viber: Table = lua.globals().get("viber")?;
        let load: Function = viber.get("load")?;
        let actions: Table = load.call("lib/actions.lua")?;
        let record: Function = actions.get("record")?;
        record.call::<()>(("visitar", "local", place))
    })();
    if let Err(err) = result {
        debug!("vael: registo da visita indisponível: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::QuestDef;
    use crate::quest::QuestEntry;

    fn objective(objective_type: &str, target: &str, count: i64, radius: f64) -> Objective {
        Objective {
            objective_type: objective_type.into(),
            target: target.into(),
            count,
            radius,
            ..Default::default()
        }
    }

    fn entry(id: &str, objective: Objective) -> QuestEntry {
        QuestEntry {
            id: id.into(),
            npc: "npc".into(),
            title: id.into(),
            objective,
            ..Default::default()
        }
    }

    fn def(id: &str, objective: Objective) -> QuestDef {
        QuestDef {
            id: id.into(),
            npc: "npc".into(),
            title: id.into(),
            objective,
            ..Default::default()
        }
    }

    fn regras() -> QuestRules {
        let mut quests = BTreeMap::new();
        quests.insert(
            "wolves".to_string(),
            entry("wolves", objective("kill", "wolf", 3, 0.0)),
        );
        quests.insert(
            "elite_hunt".to_string(),
            entry("elite_hunt", objective("kill", "elite", 2, 0.0)),
        );
        quests.insert(
            "bandits".to_string(),
            entry("bandits", objective("kill", "bandit", 2, 0.0)),
        );
        quests.insert(
            "pelt".to_string(),
            entry("pelt", objective("collect", "wolf_pelt", 3, 0.0)),
        );
        quests.insert(
            "visits".to_string(),
            entry("visits", objective("visit", "well tower", 2, 10.0)),
        );
        quests.insert(
            "staged".to_string(),
            QuestEntry {
                stages: vec![
                    objective("kill", "wolf", 1, 0.0),
                    objective("collect", "wolf_pelt", 1, 0.0),
                ],
                ..entry("staged", objective("kill", "wolf", 1, 0.0))
            },
        );
        let order = quests.keys().cloned().collect();
        QuestRules { quests, order }
    }

    fn kv_text(pairs: &[(&str, &str)]) -> QuestKv {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), json_state(value)))
            .collect()
    }

    fn kv_number(kv: &QuestKv, key: &str) -> i64 {
        match kv.get(key) {
            Some(Json::Number(number)) => number.as_i64().unwrap_or(0),
            _ => 0,
        }
    }

    #[test]
    fn gate_por_variavel_de_ambiente() {
        assert!(!VaelQuestEventsConfig::from_raw(None).rust);
        for off in ["", "0", "false", "off", "no"] {
            assert!(!VaelQuestEventsConfig::from_raw(Some(off)).rust);
        }
        for on in ["1", "true", "on", "yes", " TRUE ", "On"] {
            assert!(VaelQuestEventsConfig::from_raw(Some(on)).rust);
        }
    }

    #[test]
    fn report_kill_conta_so_os_abates_das_quests_activas() {
        let rules = regras();
        let mut kv = kv_text(&[("q.wolves", "active"), ("q.elite_hunt", "active")]);

        report_kill(&rules, &mut kv, "wolf", false);
        assert_eq!(kv_number(&kv, "q.wolves.n"), 1);
        assert_eq!(kv_number(&kv, "q.elite_hunt.n"), 0);

        // Prefixo `bandit-` casa com o alvo `bandit` (a quest está inactiva).
        report_kill(&rules, &mut kv, "bandit-archer", false);
        assert_eq!(kv_number(&kv, "q.bandits.n"), 0, "quest inactiva");

        // Elite conta para o alvo `elite` e também para a espécie.
        report_kill(&rules, &mut kv, "wolf", true);
        assert_eq!(kv_number(&kv, "q.wolves.n"), 2);
        assert_eq!(kv_number(&kv, "q.elite_hunt.n"), 1);

        // Kind vazio não conta.
        report_kill(&rules, &mut kv, "", true);
        assert_eq!(kv_number(&kv, "q.elite_hunt.n"), 1);

        // A meta promove a `ready`.
        report_kill(&rules, &mut kv, "elite", true);
        assert_eq!(
            kv.get("q.elite_hunt"),
            Some(&json_state("ready")),
            "dois abates elite"
        );
    }

    #[test]
    fn report_kill_avanca_a_etapa() {
        let rules = regras();
        let mut kv = kv_text(&[("q.staged", "active")]);
        report_kill(&rules, &mut kv, "wolf", false);
        assert_eq!(kv_number(&kv, "q.staged.stage"), 2, "etapa 1 cumprida");
        assert_eq!(kv_number(&kv, "q.staged.n"), 0);
        assert_eq!(kv.get("q.staged"), Some(&json_state("active")));
    }

    #[test]
    fn sync_collect_espelha_o_cofre() {
        let rules = regras();
        let mut kv = kv_text(&[("q.pelt", "active")]);
        kv.insert("item.wolf_pelt".into(), json_int(2));
        let vault = collect_quantities(&rules, &kv);
        sync_collect(&rules, &mut kv, &vault);
        assert_eq!(kv_number(&kv, "q.pelt.n"), 2);
        assert_eq!(kv.get("q.pelt"), Some(&json_state("active")));

        kv.insert("item.wolf_pelt".into(), json_int(3));
        let vault = collect_quantities(&rules, &kv);
        sync_collect(&rules, &mut kv, &vault);
        assert_eq!(kv_number(&kv, "q.pelt.n"), 3);
        assert_eq!(kv.get("q.pelt"), Some(&json_state("ready")));

        // Uma quest `ready` que perdeu os itens volta a `active`.
        kv.insert("item.wolf_pelt".into(), json_int(1));
        let vault = collect_quantities(&rules, &kv);
        sync_collect(&rules, &mut kv, &vault);
        assert_eq!(kv_number(&kv, "q.pelt.n"), 1);
        assert_eq!(kv.get("q.pelt"), Some(&json_state("active")));
    }

    #[test]
    fn sync_collect_respeita_o_need_do_save() {
        let rules = regras();
        let mut kv = kv_text(&[("q.pelt", "active")]);
        kv.insert("q.pelt.need".into(), json_int(5));
        kv.insert("item.wolf_pelt".into(), json_int(4));
        let vault = collect_quantities(&rules, &kv);
        sync_collect(&rules, &mut kv, &vault);
        assert_eq!(kv_number(&kv, "q.pelt.n"), 4);
        assert_eq!(kv.get("q.pelt"), Some(&json_state("active")));
    }

    #[test]
    fn scan_visits_marca_so_dentro_do_raio_e_uma_vez() {
        let rules = regras();
        let mut kv = kv_text(&[("q.visits", "active")]);
        let positions = BTreeMap::from([
            ("well".to_string(), [0.0, 5.0]),
            ("tower".to_string(), [100.0, 0.0]),
        ]);

        let hits = scan_visits(&rules, &mut kv, [0.0, 0.0], &positions);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].place, "well");
        assert_eq!(kv_number(&kv, "q.visits.n"), 1);
        assert!(kv_true(&kv, "q.visits.v.well"));

        // O mesmo marco não volta a contar nem a fazer toast.
        let hits = scan_visits(&rules, &mut kv, [0.0, 0.0], &positions);
        assert!(hits.is_empty());

        // Chegar ao segundo marco fecha a etapa.
        let hits = scan_visits(&rules, &mut kv, [95.0, 0.0], &positions);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].place, "tower");
        assert_eq!(kv_number(&kv, "q.visits.n"), 2);
        assert_eq!(kv.get("q.visits"), Some(&json_state("ready")));
    }

    #[test]
    fn scan_visits_sem_posicao_nao_conta() {
        let rules = regras();
        let mut kv = kv_text(&[("q.visits", "active")]);
        let hits = scan_visits(&rules, &mut kv, [0.0, 0.0], &BTreeMap::new());
        assert!(hits.is_empty());
        assert_eq!(kv_number(&kv, "q.visits.n"), 0);
    }

    #[test]
    fn resolve_positions_exacto_antes_de_substring() {
        let names = vec![
            ("well".to_string(), [1.0, 2.0]),
            ("well-extra".to_string(), [9.0, 9.0]),
            ("tower".to_string(), [3.0, 4.0]),
        ];
        let tokens = BTreeSet::from([
            "well".to_string(),
            "tower".to_string(),
            "towe".to_string(),
            "missing".to_string(),
        ]);
        let (positions, missing) = resolve_positions(&names, &tokens);
        assert_eq!(positions.get("well"), Some(&[1.0, 2.0]));
        assert_eq!(positions.get("tower"), Some(&[3.0, 4.0]));
        assert_eq!(positions.get("towe"), Some(&[3.0, 4.0]));
        assert_eq!(missing, vec!["missing".to_string()]);
    }

    #[test]
    fn fila_dedupe_os_pedidos_idempotentes() {
        let queue = QuestEventQueue::default();
        queue.push(QuestEvent::SyncCollect);
        queue.push(QuestEvent::SyncCollect);
        queue.push(QuestEvent::ScanVisits);
        queue.push(QuestEvent::ScanVisits);
        queue.push(QuestEvent::ReportKill {
            kind: "wolf".into(),
            elite: false,
        });
        queue.push(QuestEvent::ReportKill {
            kind: "wolf".into(),
            elite: false,
        });
        assert_eq!(queue.len(), 4);

        assert_eq!(queue.take_visit_requests(), 1);
        let events = queue.take_non_visit();
        assert_eq!(
            events,
            vec![
                QuestEvent::SyncCollect,
                QuestEvent::ReportKill {
                    kind: "wolf".into(),
                    elite: false
                },
                QuestEvent::ReportKill {
                    kind: "wolf".into(),
                    elite: false
                },
            ]
        );
        assert!(queue.is_empty());
    }

    #[test]
    fn bridge_lua_empurra_os_tres_eventos() {
        let lua = Lua::new();
        lua.globals()
            .set("viber", lua.create_table().expect("tabela viber"))
            .expect("global viber");
        let queue = QuestEventQueue::default();
        install(&lua, &queue).expect("instalar bridge");

        lua.load(
            "viber.vael.quest_report_kill('wolf', true)\n\
             viber.vael.quest_report_kill('', false)\n\
             viber.vael.quest_sync_collect()\n\
             viber.vael.quest_scan_visits()",
        )
        .exec()
        .expect("chamar bridge");

        assert_eq!(
            queue.drain_all(),
            vec![
                QuestEvent::ReportKill {
                    kind: "wolf".into(),
                    elite: true
                },
                QuestEvent::SyncCollect,
                QuestEvent::ScanVisits,
            ]
        );
    }

    #[test]
    fn changed_patch_marca_remocoes_com_null() {
        let before = kv_text(&[("q.a", "active")]);
        let after = QuestKv::new();
        let patch = changed_patch(&before, &after);
        assert_eq!(patch.get("q.a"), Some(&Json::Null));
    }

    #[test]
    fn espelho_actualiza_estado_e_done() {
        let before = kv_text(&[("q.a", "active"), ("q.b", "active")]);
        let after = kv_text(&[("q.a", "done"), ("q.b", "active")]);
        let mut log = QuestLog::default();
        apply_quest_mirror(&before, &after, &mut log);
        assert_eq!(log.states.get("a").map(String::as_str), Some("done"));
        assert_eq!(log.done, vec!["a".to_string()]);
    }

    #[test]
    fn avisos_de_etapa_e_conclusao() {
        let rules = regras();
        let concluida = quest_notices(
            &rules,
            &kv_text(&[("q.wolves", "active")]),
            &kv_text(&[("q.wolves", "ready")]),
        );
        assert_eq!(
            concluida,
            vec![QuestNotice::Complete {
                title: "wolves".into()
            }]
        );

        let mut antes = kv_text(&[("q.staged", "active")]);
        antes.insert("q.staged.stage".into(), json_int(1));
        let mut depois = kv_text(&[("q.staged", "active")]);
        depois.insert("q.staged.stage".into(), json_int(2));
        let etapa = quest_notices(&rules, &antes, &depois);
        assert_eq!(
            etapa,
            vec![QuestNotice::Stage {
                title: "staged".into(),
                stage: 1,
                total: 2
            }]
        );

        // Voltar de `ready` a `active` (itens perdidos) não avisa.
        assert!(
            quest_notices(
                &rules,
                &kv_text(&[("q.pelt", "ready")]),
                &kv_text(&[("q.pelt", "active")]),
            )
            .is_empty()
        );
    }

    fn app_com_quests() -> App {
        let mut content = VaelContent::default();
        content.quests.insert(
            "wolves".to_string(),
            def("wolves", objective("kill", "wolf", 3, 0.0)),
        );
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(
                LuaScriptHost::new(std::path::PathBuf::from("scripts")).expect("host Luau"),
            )
            .insert_resource(content)
            .insert_resource(VaelQuestEventsConfig { rust: true })
            .init_resource::<QuestLog>()
            .add_plugins(VaelQuestEventsPlugin);
        app.update();
        app
    }

    #[test]
    fn plugin_sem_host_nao_panica() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(VaelQuestEventsPlugin);
        for _ in 0..2 {
            app.update();
        }
    }

    #[test]
    fn gate_do_save_e_corrigido_antes_dos_scripts() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(
                LuaScriptHost::new(std::path::PathBuf::from("scripts")).expect("host Luau"),
            )
            .insert_resource(VaelQuestEventsConfig { rust: false })
            .add_plugins(VaelQuestEventsPlugin);
        app.update();
        let game: Table = {
            let host = app.world().resource::<LuaScriptHost>();
            host.lua
                .named_registry_value("viber_game")
                .expect("viber_game")
        };

        // Save escrito com o gate ligado: a sessão normal apaga a flag.
        game.raw_set(RUST_GATE_KEY, true).expect("flag");
        app.update();
        assert!(!matches!(
            game.raw_get::<Value>(RUST_GATE_KEY),
            Ok(Value::Boolean(true))
        ));

        // A sessão com Rust volta a ligá-la.
        app.world_mut().resource_mut::<VaelQuestEventsConfig>().rust = true;
        app.update();
        assert!(matches!(
            game.raw_get::<Value>(RUST_GATE_KEY),
            Ok(Value::Boolean(true))
        ));
    }

    #[test]
    fn sistema_drena_o_abate_e_escreve_no_viber_game() {
        let mut app = app_com_quests();
        {
            let host = app.world().resource::<LuaScriptHost>();
            let game: Table = host
                .lua
                .named_registry_value("viber_game")
                .expect("viber_game");
            game.raw_set("q.wolves", STATE_ACTIVE).expect("estado");
        }
        app.world()
            .resource::<QuestEventQueue>()
            .push(QuestEvent::ReportKill {
                kind: "wolf".into(),
                elite: false,
            });
        app.update();

        let host = app.world().resource::<LuaScriptHost>();
        let game: Table = host
            .lua
            .named_registry_value("viber_game")
            .expect("viber_game");
        assert_eq!(game.raw_get::<i64>("q.wolves.n").expect("progresso"), 1);
        assert!(
            game.raw_get::<bool>(RUST_GATE_KEY).expect("gate"),
            "o gate fica ligado no viber_game"
        );
        let log = app.world().resource::<QuestLog>();
        assert_eq!(log.states.get("wolves").map(String::as_str), Some("active"));
    }
}

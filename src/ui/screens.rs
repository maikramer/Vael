//! Ecrãs de menu, título e morte em `bevy_pf`, com a lógica em Rust.
//!
//! O HUD/menus desenham-se a partir de `VaelSave` + `VaelContent`; as ações
//! usam as regras já portadas (`economy`, `gear`, `quest`, `vault`, `xp`) e
//! escrevem as chaves partilhadas no `viber.game()` por `apply_patch` (o
//! estado que o resto do jogo ainda lê). Nada aqui chama `dispatch` de Lua.
//!
//! Os roots declarativos de `world/menu.xml`, `title.xml` e `death.xml`
//! continuam no autoload apenas para o estado dos modais/`UiTabs` e para o
//! `viber.ui.is_open("title"/"death")` dos scripts de jogo; estão escondidos
//! e já não têm `script=` — a apresentação e as ações são deste módulo.

use std::collections::{BTreeMap, BTreeSet};

use bevy::app::AppExit;
use bevy::prelude::*;
use bevy::ui::GlobalZIndex;
use bevy_pf::prelude::*;
use serde_json::{Value, json};
use viber::a11y::A11y;
use bevy_ext_core::{SfxClip, SfxEvent};
use viber::locale::Locale;
use viber::luau::components::ScriptToast;
use viber::luau::host::LuaScriptHost;
use viber::music::AudioMixerSettings;
use bevy_ext_save::{SaveDir, SaveSlotInfo, WorldSaveKey};
use viber::save::SaveRequest;
use viber::scene_loader::{Arrival, ChangeScene, CurrentScene, request_change};
use viber::ui::scale::UiUserScale;
use viber::ui::{UiData, UiModalsOpen, UiTabs, UiWriter};

use crate::content::{ItemDef, QuestDef, VaelContent, label};
use crate::economy::{
    CraftRules, ShopRules, TradeAction, craft_make, craft_status, shop_rows, shop_trade,
};
use crate::gear::{GearRules, GearState, SLOTS, Toggle, is_equipped, toggle as gear_toggle};
use crate::quest::{QuestKv, QuestRules, quest_objective, quest_progress, quest_state};
use crate::save::VaelSave;
use crate::stats::{AttributeRules, StatEngine};
use crate::ui::talent_tree::{
    TalentEdgeVm, TalentNodeVm, VaelTalentLayout, talent_detail, talent_nodes,
};
use crate::vault::{ITEM_KEY_PREFIX, VaultRules, item_key, vault_get};
use crate::xp::{KEY_LEVEL, KEY_SKILL_POINTS, KEY_XP, KEY_XP_NEXT, XpMap, XpRules, add_xp};

/// Ordem/z-index das camadas bevy_pf.
const Z_MENU: i32 = 100;
const Z_TITLE: i32 = 200;
const Z_DEATH: i32 = 300;

/// Uma linha genérica de lista.
#[derive(Reflect, Default, Clone, Debug, PartialEq)]
pub struct RowVm {
    pub action: String,
    pub title: String,
    pub subtitle: String,
    pub tag: String,
    pub progress: f64,
    pub has_progress: bool,
    pub clickable: bool,
    pub icon: String,
    pub has_icon: bool,
}

impl RowVm {
    /// Linha base das listas dos menus: clicável, sem progresso nem ícone
    /// (os dois só existem em algumas abas — entram com os métodos abaixo).
    fn menu(
        action: impl Into<String>,
        title: impl Into<String>,
        subtitle: impl Into<String>,
        tag: impl Into<String>,
    ) -> Self {
        Self {
            action: action.into(),
            title: title.into(),
            subtitle: subtitle.into(),
            tag: tag.into(),
            clickable: true,
            ..Self::default()
        }
    }

    /// Marca a linha com uma barra de progresso (0..1).
    fn with_progress(mut self, progress: f64) -> Self {
        self.progress = progress;
        self.has_progress = true;
        self
    }

    /// Dá um ícone à linha (`None` deixa o lugar vazio).
    fn with_icon(mut self, icon: Option<String>) -> Self {
        self.has_icon = icon.is_some();
        self.icon = icon.unwrap_or_default();
        self
    }

    /// Linha informativa: sem `action`, sem clique.
    fn info(mut self) -> Self {
        self.clickable = false;
        self
    }
}

/// Fecha os overlays de jogo (diário, mapa, viagem, profiler). Os modais
/// de topo (morte/título) e o HUD ficam para quem chama.
fn close_overlays(writer: &UiWriter) {
    writer.open("menu", false);
    writer.open("region-map", false);
    writer.open("travel", false);
    writer.open("profiler-win", false);
}

/// Regras derivadas do conteúdo, construídas uma vez.
#[derive(Resource, Clone)]
pub struct VaelUiRules {
    pub shop: ShopRules,
    pub craft: CraftRules,
    pub gear: GearRules,
    pub vault: VaultRules,
    pub quests: QuestRules,
    pub xp: XpRules,
    pub attributes: AttributeRules,
}

/// Estado de sessão dos ecrãs.
#[derive(Resource, Default)]
pub struct VaelScreensState {
    pub booted: bool,
    pub hud_visible: bool,
    pub menu_settings: bool,
    /// Frames até fechar o título depois do load.
    pub pending_title_close: u8,
    /// Frames até trocar para a cena `menu` depois do autosave.
    pub pending_menu_scene: u8,
    /// Talento selecionado na árvore visual (a seleção não gasta pontos).
    pub talent_selected: Option<String>,
    /// O `drive_death_screen` já pediu a abertura do modal de morte.
    pub death_open_requested: bool,
}

/// Cache de metadados dos slots (evita ler o disco todos os frames).
#[derive(Resource, Default)]
pub struct SaveSlotsCache {
    pub slots: Vec<SaveSlotInfo>,
    pub timer: f32,
}

/// View-model dos ecrãs.
#[derive(Reflect, Default, Clone)]
pub struct ScreensVm {
    pub menu_open: bool,
    pub menu_stats_visible: bool,
    pub menu_title: String,
    pub menu_clock: String,
    pub menu_hp_label: String,
    pub menu_hp: String,
    pub menu_level_label: String,
    pub menu_level: String,
    pub menu_gold_label: String,
    pub menu_gold: String,
    pub menu_foot: String,
    pub menu_points_label: String,
    pub menu_points: String,
    /// Linhas compactas da aba Talentos: atributos e stats derivadas.
    pub menu_attributes: String,
    pub menu_derived: String,

    /// Árvore visual de talentos: nós posicionados, arestas e detalhe.
    pub talent_nodes: Vec<TalentNodeVm>,
    pub talent_requires_edges: Vec<TalentEdgeVm>,
    pub talent_exclusion_edges: Vec<TalentEdgeVm>,
    pub talent_canvas_width: f64,
    pub talent_canvas_height: f64,
    pub talent_detail_visible: bool,
    pub talent_selected_icon: String,
    pub talent_has_selected_icon: bool,
    pub talent_selected_title: String,
    pub talent_selected_effects: String,
    pub talent_selected_requirement: String,
    pub talent_selected_status: String,
    pub talent_learn_label: String,
    pub talent_learn_enabled: bool,

    pub tab_quests: bool,
    pub tab_bag: bool,
    pub tab_skills: bool,
    pub tab_shop: bool,
    pub tab_forge: bool,
    pub tab_controls: bool,
    pub tab_a11y: bool,
    pub tab_system: bool,

    pub tab_quests_label: String,
    pub tab_bag_label: String,
    pub tab_skills_label: String,
    pub tab_shop_label: String,
    pub tab_forge_label: String,
    pub tab_controls_label: String,
    pub tab_a11y_label: String,
    pub tab_system_label: String,

    pub quests: Vec<RowVm>,
    pub bag: Vec<RowVm>,
    pub shop: Vec<RowVm>,
    pub forge: Vec<RowVm>,
    pub controls: Vec<RowVm>,
    pub a11y: Vec<RowVm>,
    pub system: Vec<RowVm>,
    pub quests_empty: bool,
    pub bag_empty: bool,
    pub shop_empty: bool,
    pub forge_empty: bool,
    pub quests_empty_text: String,
    pub bag_empty_text: String,
    pub shop_empty_text: String,
    pub forge_empty_text: String,

    pub save1_label: String,
    pub save2_label: String,
    pub save3_label: String,
    pub load1_label: String,
    pub load2_label: String,
    pub load3_label: String,
    pub save_enabled: bool,
    pub load1_enabled: bool,
    pub load2_enabled: bool,
    pub load3_enabled: bool,
    pub title_button_label: String,
    pub profiler_label: String,

    pub title_open: bool,
    pub credits_open: bool,
    pub title_logo: String,
    pub title_kicker: String,
    pub title_sub: String,
    pub title_hint: String,
    pub title_continue_label: String,
    pub title_new_label: String,
    pub title_settings_label: String,
    pub title_credits_label: String,
    pub title_quit_label: String,
    pub title_back_label: String,
    pub title_save_info: String,
    pub title_continue_enabled: bool,
    pub credits_title: String,
    pub credits: Vec<RowVm>,

    pub death_open: bool,
    pub death_kicker: String,
    pub death_title: String,
    pub death_line: String,
    pub death_save_info: String,
    pub death_retry_label: String,
    pub death_load_label: String,
    pub death_title_label: String,
    pub death_load_enabled: bool,

    pub row_command: RelayCommand,
    pub tab_command: RelayCommand,
    pub action_command: RelayCommand,
}

/// Handle partilhado do view-model.
#[derive(Resource, Clone)]
pub struct VaelScreensVm(pub Bindable);

impl ScreensVm {
    pub fn with_commands() -> Self {
        let mut vm = Self::default();
        vm.row_command = RelayCommand::new(|world, param| {
            if let Some(id) = param {
                handle_action(world, &id);
            }
        });
        vm.tab_command = RelayCommand::new(|world, param| {
            let Some(tab) = param else { return };
            if let Some(writer) = world.get_resource::<UiWriter>() {
                writer.select_tab("menu", tab);
            }
        });
        vm.action_command = RelayCommand::new(|world, param| {
            if let Some(id) = param {
                handle_action(world, &id);
            }
        });
        vm
    }
}

// ── helpers de estado ───────────────────────────────────────────────────

fn game_kv(world: &World) -> BTreeMap<String, Value> {
    world
        .get_resource::<LuaScriptHost>()
        .map(|host| viber::luau::game::game_to_json(&host.lua))
        .unwrap_or_default()
}

fn patch_game(world: &mut World, patch: BTreeMap<String, Value>) {
    if patch.is_empty() {
        return;
    }
    if let Some(host) = world.get_resource::<LuaScriptHost>() {
        viber::luau::game::apply_patch(&host.lua, &patch);
    }
}

fn toast(world: &mut World, text: impl Into<String>) {
    if world.contains_resource::<Messages<ScriptToast>>() {
        world.write_message(ScriptToast(text.into()));
    }
}

fn sfx(world: &mut World, clip: SfxClip) {
    if world.contains_resource::<Messages<SfxEvent>>() {
        world.write_message(SfxEvent {
            clip,
            position: None,
        });
    }
}

/// Cofre no espaço de chaves do `viber.game()` (`item.<id>`), que é o que o
/// `vault_get`/`shop_rows`/`craft_status` esperam. O `patch_items` devolve
/// estas mesmas chaves à engine — não voltar a prefixá-las.
fn inventory_kv(save: &VaelSave) -> BTreeMap<String, i64> {
    let mut kv = BTreeMap::new();
    kv.insert(item_key("gold"), save.inventory.gold);
    kv.insert(item_key("wood"), save.inventory.wood);
    kv.insert(item_key("stone"), save.inventory.stone);
    kv.insert(item_key("berry"), save.inventory.berry);
    kv.insert(item_key("korok-seed"), save.inventory.get("korok_seed"));
    kv.insert(item_key("potion"), save.inventory.potion);
    kv.insert(item_key("antidote"), save.inventory.antidote);
    kv.insert(item_key("bomb"), save.inventory.bomb);
    kv.insert(item_key("moss_potion"), save.inventory.moss_potion);
    for (id, n) in &save.inventory.items {
        kv.insert(item_key(id), *n);
    }
    kv
}

fn patch_items(world: &mut World, kv: &BTreeMap<String, i64>) {
    let patch = kv.iter().map(|(id, n)| (id.clone(), json!(n))).collect();
    patch_game(world, patch);
}

pub(crate) fn quest_kv(save: &VaelSave) -> QuestKv {
    let mut kv = save.legacy.clone();
    for (id, state) in &save.quests.states {
        kv.insert(format!("q.{id}"), Value::String(state.clone()));
    }
    kv
}

fn gear_state(save: &VaelSave) -> GearState {
    let mut held = BTreeMap::new();
    for (key, n) in inventory_kv(save) {
        if n > 0 {
            let id = key
                .strip_prefix(ITEM_KEY_PREFIX)
                .unwrap_or(&key)
                .to_string();
            held.insert(id, n as f64);
        }
    }
    let mut equipped = BTreeMap::new();
    if let Some(id) = &save.equipment.weapon {
        equipped.insert("weapon".to_string(), id.clone());
    }
    if let Some(id) = &save.equipment.armor {
        equipped.insert("armor".to_string(), id.clone());
    }
    if let Some(id) = &save.equipment.charm {
        equipped.insert("charm".to_string(), id.clone());
    }
    GearState { held, equipped }
}

fn learned_set(save: &VaelSave) -> BTreeSet<String> {
    let mut learned = save.progression.skills.clone();
    for (key, value) in &save.legacy {
        if let Some(id) = key.strip_prefix("skill.")
            && value.as_bool() == Some(true)
        {
            learned.insert(id.to_string());
        }
    }
    learned
}

fn requirement_met(
    rules: &VaelUiRules,
    state: &mut GearState,
    learned: &BTreeSet<String>,
    id: &str,
) -> bool {
    let Some(item) = rules.gear.items.get(id) else {
        return false;
    };
    if item.requires.trim().is_empty() {
        return true;
    }
    let mut engine = StatEngine::new(rules.attributes);
    let _ = engine.snapshot(&rules.gear, state, learned);
    engine.requirement_met(&item.requires)
}

fn latest_slot(slots: &[SaveSlotInfo]) -> Option<&SaveSlotInfo> {
    slots
        .iter()
        .filter(|slot| slot.exists && slot.scene.as_deref() != Some("menu"))
        .max_by(|a, b| {
            a.saved_at
                .unwrap_or(0.0)
                .partial_cmp(&b.saved_at.unwrap_or(0.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

fn fmt_playtime(secs: Option<f64>) -> Option<String> {
    let n = secs.filter(|n| n.is_finite() && *n >= 0.0)?;
    let n = n.floor() as i64;
    let h = n / 3600;
    let m = (n % 3600) / 60;
    Some(if h > 0 {
        format!("{h} h {m:02} min")
    } else {
        format!("{m} min")
    })
}

fn slot_summary(slot: Option<&SaveSlotInfo>) -> String {
    let Some(slot) = slot else {
        return "Sem jogos guardados".to_string();
    };
    let where_ = if slot.level > 0 {
        format!("Nível {}", slot.level)
    } else {
        "Jogo guardado".to_string()
    };
    match fmt_playtime(slot.playtime) {
        Some(time) => format!("{where_} · {time}"),
        None => where_,
    }
}

fn load_label(n: u32, slot: Option<&SaveSlotInfo>) -> String {
    let Some(slot) = slot else {
        return format!("Carregar {n}");
    };
    if slot.scene.as_deref() == Some("menu") {
        return format!("Carregar {n} · sem jogo");
    }
    let level = if slot.level > 0 {
        format!("Nv {}", slot.level)
    } else {
        "guardado".to_string()
    };
    match fmt_playtime(slot.playtime) {
        Some(time) => format!("Carregar {n} · {level} · {time}"),
        None => format!("Carregar {n} · {level}"),
    }
}

fn in_menu_scene(scene: Option<&CurrentScene>) -> bool {
    scene
        .and_then(|scene| scene.id.as_deref())
        .is_some_and(|id| id == "menu")
}

fn scene_id(scene: Option<&CurrentScene>) -> String {
    scene
        .and_then(|scene| scene.id.clone())
        .unwrap_or_else(|| "—".to_string())
}

/// O save está à espera de mostrar o ecrã de morte. A autoridade Rust
/// escreve `death_pending` no KV; o estado chega aqui pelo `VaelSave`.
fn death_pending(save: &VaelSave) -> bool {
    save.legacy
        .get("death_pending")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Transição do modal de morte. Puro para testar sem UI.
fn death_should_open(save: &VaelSave, modal_open: bool) -> bool {
    death_pending(save) && !modal_open
}

/// Abre o ecrã de morte quando a autoridade o pede. Com a autoridade de
/// vitais ligada, o `rules.lua` já não abre o modal (o bloco está dentro de
/// `not rust_vitals()`); sem este sistema a morte caía no fallback sem UI.
pub(crate) fn drive_death_screen(
    save: Option<Res<VaelSave>>,
    modals: Option<Res<UiModalsOpen>>,
    writer: Option<Res<UiWriter>>,
    mut state: ResMut<VaelScreensState>,
) {
    let (Some(save), Some(writer)) = (save, writer) else {
        return;
    };
    let open = modals.as_deref().is_some_and(|m| m.is_open("death"));
    let pending = death_pending(&save);
    if death_should_open(&save, open) && !state.death_open_requested {
        close_overlays(&writer);
        writer.open("death", true);
        state.death_open_requested = true;
    } else if !pending && open && state.death_open_requested {
        // Load/retry já curaram o herói mas o modal ainda estava aberto.
        writer.open("death", false);
        state.death_open_requested = false;
    }
    if !open && !pending {
        state.death_open_requested = false;
    }
}

fn player_alive(save: &VaelSave) -> bool {
    save.vitals.hp > 0.0
        && !save
            .legacy
            .get("death_pending")
            .and_then(Value::as_bool)
            .unwrap_or(false)
}

// ── acções (Rust) ───────────────────────────────────────────────────────

fn handle_action(world: &mut World, id: &str) {
    match id {
        "menu-close" => {
            if let Some(writer) = world.get_resource::<UiWriter>() {
                writer.open("menu", false);
            }
        }
        "act-profiler" => {
            if let Some(writer) = world.get_resource::<UiWriter>() {
                writer.open("menu", false);
                writer.open("profiler-win", true);
            }
        }
        "act-title" => menu_to_title(world),
        value if value.starts_with("act-save-") => {
            if let Some(n) = value
                .strip_prefix("act-save-")
                .and_then(|n| n.parse::<u32>().ok())
            {
                request_save(world, n);
            }
        }
        value if value.starts_with("act-load-") => {
            if let Some(n) = value
                .strip_prefix("act-load-")
                .and_then(|n| n.parse::<u32>().ok())
            {
                request_load(world, n);
            }
        }
        "talent-learn" => learn_selected_talent(world),
        value if value.starts_with("talent-") => {
            if let Some(id) = value.strip_prefix("talent-") {
                select_talent(world, id);
            }
        }
        value if value.starts_with("skill-") => {
            if let Some(id) = value.strip_prefix("skill-") {
                learn_skill(world, id);
            }
        }
        value if value.starts_with("bag-") => {
            if let Some(id) = value.strip_prefix("bag-") {
                bag_click(world, id);
            }
        }
        value if value.starts_with("forge-") => {
            if let Some(id) = value.strip_prefix("forge-") {
                forge_click(world, id);
            }
        }
        value if value.starts_with("track-") => {
            if let Some(id) = value.strip_prefix("track-") {
                track_quest(world, id);
            }
        }
        value if value.starts_with("shop-") => {
            if let Some(id) = value.strip_prefix("shop-") {
                shop_click(world, id);
            }
        }
        value if value.starts_with("a11y-") => {
            if let Some(id) = value.strip_prefix("a11y-") {
                a11y_click(world, id);
            }
        }
        "title-continue" => title_continue(world),
        "title-new" => title_new(world),
        "title-settings" => title_settings(world),
        "title-credits" => {
            if let Some(writer) = world.get_resource::<UiWriter>() {
                writer.open("credits", true);
            }
        }
        "credits-close" => {
            if let Some(writer) = world.get_resource::<UiWriter>() {
                writer.open("credits", false);
            }
        }
        "title-quit" => {
            if world.contains_resource::<Messages<AppExit>>() {
                world.write_message(AppExit::Success);
            }
        }
        "death-retry" => death_retry(world),
        "death-load" => death_load(world),
        "death-title" => death_to_title(world),
        _ => {}
    }
}

fn request_save(world: &mut World, slot: u32) {
    let save = world.resource::<VaelSave>();
    if in_menu_scene(world.get_resource::<CurrentScene>()) || !player_alive(save) {
        return;
    }
    if let Some(mut request) = world.get_resource_mut::<SaveRequest>() {
        request.save = true;
        request.load = false;
        request.slot = Some(slot);
    }
    sfx(world, SfxClip::Ui);
}

fn request_load(world: &mut World, slot: u32) {
    if let Some(cache) = world.get_resource::<SaveSlotsCache>() {
        let Some(info) = cache.slots.iter().find(|s| s.slot == slot) else {
            toast(world, format!("O slot {slot} está vazio."));
            sfx(world, SfxClip::Error);
            return;
        };
        if !info.exists || info.scene.as_deref() == Some("menu") {
            toast(world, format!("O slot {slot} não tem um jogo guardado."));
            sfx(world, SfxClip::Error);
            return;
        }
    }
    if let Some(mut request) = world.get_resource_mut::<SaveRequest>() {
        request.save = false;
        request.load = true;
        request.slot = Some(slot);
    }
    if let Some(writer) = world.get_resource::<UiWriter>() {
        close_overlays(&writer);
        writer.set_visible("hud", true);
    }
    let mut state = world.resource_mut::<VaelScreensState>();
    state.hud_visible = true;
    state.pending_title_close = 120;
    state.menu_settings = false;
}

fn select_talent(world: &mut World, id: &str) {
    if let Some(mut state) = world.get_resource_mut::<VaelScreensState>() {
        state.talent_selected = Some(id.to_string());
    }
}

fn learn_selected_talent(world: &mut World) {
    let Some(state) = world.get_resource::<VaelScreensState>() else {
        return;
    };
    let Some(id) = state.talent_selected.clone() else {
        return;
    };
    learn_skill(world, &id);
}

fn learn_skill(world: &mut World, id: &str) {
    let Some(rules) = world.get_resource::<VaelUiRules>().cloned() else {
        return;
    };
    let Some(content) = world.get_resource::<VaelContent>().cloned() else {
        return;
    };
    let save = world.resource::<VaelSave>().clone();
    let learned = learned_set(&save);
    if learned.contains(id) {
        return;
    }
    let Some(def) = content.progression.skills.iter().find(|s| s.id == id) else {
        return;
    };
    let requires = def.requires.as_deref().unwrap_or("");
    let excludes = def.excludes.as_deref().unwrap_or("");
    if !requires.is_empty() && !learned.contains(requires) {
        toast(world, "Falta o talento anterior.");
        sfx(world, SfxClip::Error);
        return;
    }
    if !excludes.is_empty() && learned.contains(excludes) {
        let other = content
            .progression
            .skills
            .iter()
            .find(|s| s.id == excludes)
            .map(|s| s.label.clone())
            .unwrap_or_else(|| excludes.to_string());
        toast(world, format!("Já escolheste {other}."));
        sfx(world, SfxClip::Error);
        return;
    }
    if save.progression.skill_points < 1 {
        toast(world, "Sem pontos de talento.");
        sfx(world, SfxClip::Error);
        return;
    }
    let mut patch = BTreeMap::new();
    patch.insert(format!("skill.{id}"), json!(true));
    patch.insert(
        "skill_points".to_string(),
        json!(save.progression.skill_points - 1),
    );
    patch_game(world, patch);
    toast(world, def.label.clone());
    sfx(world, SfxClip::LevelUp);
    let _ = rules;
}

fn bag_click(world: &mut World, id: &str) {
    let Some(rules) = world.get_resource::<VaelUiRules>().cloned() else {
        return;
    };
    let Some(content) = world.get_resource::<VaelContent>().cloned() else {
        return;
    };
    let save = world.resource::<VaelSave>().clone();
    let Some(def) = content.items.items.iter().find(|item| item.id == id) else {
        return;
    };
    let mut state = gear_state(&save);
    let learned = learned_set(&save);
    if rules.gear.slot_of(id).is_none() {
        if !def.desc.is_empty() {
            toast(world, def.desc.clone());
        }
        return;
    }
    if !is_equipped(&rules.gear, &mut state, id)
        && !requirement_met(&rules, &mut state, &learned, id)
    {
        let needs = rules
            .gear
            .items
            .get(id)
            .map(|item| item.requires_text.clone())
            .unwrap_or_default();
        toast(world, format!("Requer {}", needs));
        sfx(world, SfxClip::Error);
        return;
    }
    let Some(result) = gear_toggle(&rules.gear, &mut state, id) else {
        return;
    };
    let mut patch = BTreeMap::new();
    for slot in SLOTS {
        let key = format!("gear.{slot}");
        match state.equipped.get(slot) {
            Some(equipped) => {
                patch.insert(key, json!(equipped));
            }
            None => {
                patch.insert(key, Value::Null);
            }
        }
    }
    patch_game(world, patch);
    match result {
        Toggle::On => {
            toast(world, format!("Equipado: {}", def.label));
            sfx(world, SfxClip::ItemDrop);
        }
        Toggle::Off => {
            toast(world, format!("Guardado: {}", def.label));
            sfx(world, SfxClip::Ui);
        }
    }
}

fn track_quest(world: &mut World, id: &str) {
    let save = world.resource::<VaelSave>().clone();
    let state = save
        .quests
        .states
        .get(id)
        .map(String::as_str)
        .unwrap_or("not_taken");
    if state != "active" && state != "ready" {
        toast(world, "Essa missão já não está em curso.");
        return;
    }
    let mut patch = BTreeMap::new();
    if save.quests.tracked.as_deref() == Some(id) {
        patch.insert("tracked_quest".to_string(), Value::Null);
        patch.insert("pin".to_string(), Value::Null);
        toast(world, "Missão deixada de seguir.");
    } else {
        patch.insert("tracked_quest".to_string(), json!(id));
        patch.insert("pin".to_string(), Value::Null);
        let title = world
            .get_resource::<VaelContent>()
            .and_then(|content| content.quests.get(id))
            .map(|quest| quest.title.clone())
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| id.to_string());
        toast(world, format!("A seguir: {title}"));
    }
    patch_game(world, patch);
}

fn shop_click(world: &mut World, id: &str) {
    let Some(rules) = world.get_resource::<VaelUiRules>().cloned() else {
        return;
    };
    let save = world.resource::<VaelSave>().clone();
    let mut kv = inventory_kv(&save);
    let rows = shop_rows(&rules.shop, &kv);
    let Some(row) = rows.iter().find(|row| row.id == id) else {
        return;
    };
    let action = if row.selling {
        TradeAction::Sell
    } else {
        TradeAction::Buy
    };
    let outcome = shop_trade(&rules.shop, &mut kv, action, id);
    patch_items(world, &kv);
    if outcome.ok {
        match action {
            TradeAction::Buy => {
                toast(world, format!("Comprado {}.", row.label));
                sfx(world, SfxClip::Buy);
            }
            TradeAction::Sell => {
                let price = outcome.price.unwrap_or(0);
                toast(world, format!("Vendido por {price} de ouro."));
                sfx(world, SfxClip::Coin);
            }
        }
    } else {
        let reason = match outcome.reason {
            Some(crate::economy::TradeReason::NoGold) => format!(
                "{} custa {} de ouro — não chega.",
                outcome.label.clone().unwrap_or_default(),
                outcome.price.unwrap_or(0)
            ),
            Some(crate::economy::TradeReason::Full) => {
                format!("Já tens o máximo disso ({}).", outcome.cap.unwrap_or(0))
            }
            Some(crate::economy::TradeReason::NoSpace) => "Não coube na mochila.".to_string(),
            Some(crate::economy::TradeReason::GoldFull) => "Não coube o ouro na bolsa.".to_string(),
            Some(crate::economy::TradeReason::NoStock) => "Sem isso para vender.".to_string(),
            Some(crate::economy::TradeReason::NoPrice) => "Não vendo isso.".to_string(),
            _ => "Não foi possível negociar.".to_string(),
        };
        toast(world, reason);
        sfx(world, SfxClip::Error);
    }
}

fn forge_click(world: &mut World, id: &str) {
    let Some(rules) = world.get_resource::<VaelUiRules>().cloned() else {
        return;
    };
    let Some(content) = world.get_resource::<VaelContent>().cloned() else {
        return;
    };
    let save = world.resource::<VaelSave>().clone();
    let mut kv = inventory_kv(&save);
    let outcome = craft_make(&rules.craft, &mut kv, id);
    patch_items(world, &kv);
    if outcome.ok {
        let mut patch = BTreeMap::new();
        let mut xp_kv: XpMap = BTreeMap::new();
        xp_kv.insert(KEY_XP.into(), save.progression.xp as f64);
        xp_kv.insert(KEY_XP_NEXT.into(), save.progression.xp_next as f64);
        xp_kv.insert(KEY_LEVEL.into(), save.progression.level as f64);
        xp_kv.insert(
            KEY_SKILL_POINTS.into(),
            save.progression.skill_points as f64,
        );
        let gain = add_xp(&mut xp_kv, &rules.xp, 10.0);
        for key in [KEY_XP, KEY_XP_NEXT, KEY_LEVEL, KEY_SKILL_POINTS] {
            if let Some(value) = xp_kv.get(key) {
                patch.insert(key.to_string(), json!(value));
            }
        }
        patch_game(world, patch);
        let label = content
            .items
            .items
            .iter()
            .find(|item| item.id == outcome.output)
            .map(|item| item.label.clone())
            .unwrap_or_else(|| outcome.output.clone());
        let extra = if outcome.count > 1 {
            format!(" ×{}", outcome.count)
        } else {
            String::new()
        };
        toast(world, format!("Forjado: {label}{extra}."));
        sfx(world, SfxClip::MineHit);
        if gain.levels > 0 {
            sfx(world, SfxClip::LevelUp);
        }
    } else {
        let reason = match outcome.reason {
            Some(crate::economy::CraftReason::Missing) => {
                "Falta material para forjar isso.".to_string()
            }
            Some(crate::economy::CraftReason::Full) => {
                "Já não cabe mais disso na mochila.".to_string()
            }
            _ => "Não foi possível forjar.".to_string(),
        };
        toast(world, reason);
        sfx(world, SfxClip::Error);
    }
}

fn a11y_click(world: &mut World, id: &str) {
    match id {
        "shake" | "hitstop" | "interact_hold" => {
            let mut a11y = world.resource_mut::<A11y>();
            let value = match id {
                "shake" => {
                    a11y.shake = !a11y.shake;
                    a11y.shake
                }
                "hitstop" => {
                    a11y.hitstop = !a11y.hitstop;
                    a11y.hitstop
                }
                _ => {
                    a11y.interact_hold = !a11y.interact_hold;
                    a11y.interact_hold
                }
            };
            let mut patch = BTreeMap::new();
            patch.insert(format!("a11y.{id}"), json!(value));
            patch_game(world, patch);
        }
        "subtitle" => {
            let mut a11y = world.resource_mut::<A11y>();
            let values = [1.0, 1.25, 1.5, 2.0];
            let at = values
                .iter()
                .position(|v| (*v - a11y.subtitle_scale).abs() < 0.01)
                .map(|i| (i + 1) % values.len())
                .unwrap_or(0);
            a11y.subtitle_scale = values[at];
            let next = a11y.subtitle_scale;
            let mut patch = BTreeMap::new();
            patch.insert("a11y.subtitle_scale".to_string(), json!(next));
            patch_game(world, patch);
        }
        "ui_scale" => {
            let values = [1.0, 1.25, 1.5];
            let current = world.resource::<UiUserScale>().0;
            let at = values
                .iter()
                .position(|v| (*v - current).abs() < 0.01)
                .map(|i| (i + 1) % values.len())
                .unwrap_or(0);
            world.resource_mut::<UiUserScale>().0 = values[at];
            let mut patch = BTreeMap::new();
            patch.insert("ui_scale".to_string(), json!(values[at]));
            patch_game(world, patch);
        }
        "language" => {
            let current = world
                .get_resource::<Locale>()
                .map(|locale| locale.lang.clone())
                .unwrap_or_else(|| "pt".to_string());
            let next = if current == "pt" { "en" } else { "pt" };
            if let Some(mut locale) = world.get_resource_mut::<Locale>() {
                locale.set_lang(next);
            }
            let mut patch = BTreeMap::new();
            patch.insert("lang".to_string(), json!(next));
            patch_game(world, patch);
        }
        bus @ ("vol-master" | "vol-music" | "vol-sfx" | "vol-ambience") => {
            let values = [1.0, 0.75, 0.5, 0.25, 0.0];
            let bus_name = bus.trim_start_matches("vol-");
            let mut mixer = world.resource_mut::<AudioMixerSettings>();
            let current = match bus_name {
                "master" => mixer.master,
                "music" => mixer.music,
                "sfx" => mixer.sfx,
                _ => mixer.ambience,
            };
            let at = values
                .iter()
                .position(|v| (*v - current).abs() < 0.01)
                .map(|i| (i + 1) % values.len())
                .unwrap_or(0);
            match bus_name {
                "master" => mixer.master = values[at],
                "music" => mixer.music = values[at],
                "sfx" => mixer.sfx = values[at],
                _ => mixer.ambience = values[at],
            }
            let mut patch = BTreeMap::new();
            patch.insert(format!("vol.{bus_name}"), json!(values[at]));
            patch_game(world, patch);
        }
        _ => return,
    }
    sfx(world, SfxClip::Ui);
}

fn menu_to_title(world: &mut World) {
    let scene = world.get_resource::<CurrentScene>().cloned();
    let on_title = in_menu_scene(scene.as_ref());
    let alive = player_alive(world.resource::<VaelSave>());
    if !on_title
        && alive
        && let Some(mut request) = world.get_resource_mut::<SaveRequest>()
    {
        request.save = true;
        request.load = false;
        request.slot = Some(0);
    }
    let mut patch = BTreeMap::new();
    if !on_title && !alive {
        patch.insert("dying".to_string(), json!(0));
        patch.insert("death_pending".to_string(), json!(false));
        patch.insert("death_wait".to_string(), json!(0));
    }
    patch_game(world, patch);
    if let Some(writer) = world.get_resource::<UiWriter>() {
        close_overlays(&writer);
        writer.set_visible("hud", false);
        writer.open("title", true);
    }
    let mut state = world.resource_mut::<VaelScreensState>();
    state.hud_visible = false;
    if !on_title {
        state.pending_menu_scene = 10;
    }
}

fn title_continue(world: &mut World) {
    let Some(best) = world
        .get_resource::<SaveSlotsCache>()
        .and_then(|cache| latest_slot(&cache.slots).cloned())
    else {
        return;
    };
    if let Some(mut request) = world.get_resource_mut::<SaveRequest>() {
        request.save = false;
        request.load = true;
        request.slot = if best.slot == 4 {
            None
        } else {
            Some(best.slot)
        };
    }
    let mut state = world.resource_mut::<VaelScreensState>();
    state.pending_title_close = 120;
    state.hud_visible = true;
    state.menu_settings = false;
}

fn title_new(world: &mut World) {
    let keep: Vec<String> = game_kv(world)
        .keys()
        .filter(|key| {
            *key == "ui_scale"
                || *key == "lang"
                || key.starts_with("a11y.")
                || key.starts_with("vol.")
        })
        .cloned()
        .collect();
    let session = game_kv(world)
        .get("session_id")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let mut patch = BTreeMap::new();
    for key in game_kv(world).keys() {
        if !keep.contains(key) {
            patch.insert(key.clone(), Value::Null);
        }
    }
    patch.insert("session_id".to_string(), json!(session + 1));
    patch.insert("title_done".to_string(), json!(true));
    patch_game(world, patch);
    if let Some(writer) = world.get_resource::<UiWriter>() {
        writer.open("title", false);
        writer.open("credits", false);
        writer.set_visible("hud", true);
    }
    world.resource_mut::<VaelScreensState>().hud_visible = true;
    world.resource_mut::<VaelScreensState>().menu_settings = false;
    if let Err(err) = request_change(world, ChangeScene::new("main", Arrival::Entry(None))) {
        warn!("vael UI: Novo Jogo sem troca de cena: {err}");
    }
}

fn title_settings(world: &mut World) {
    if let Some(writer) = world.get_resource::<UiWriter>() {
        writer.open("title", false);
        writer.open("menu", true);
        writer.select_tab("menu", "system");
    }
    world.resource_mut::<VaelScreensState>().menu_settings = true;
}

fn flask_max(save: &VaelSave, rules: &VaelUiRules) -> u32 {
    let learned = learned_set(save);
    let mut state = gear_state(save);
    let mut engine = StatEngine::new(rules.attributes);
    let snapshot = engine.snapshot(&rules.gear, &mut state, &learned);
    crate::respawn::flasks_from_stat(snapshot.stat("flasks"))
}

fn death_retry(world: &mut World) {
    let save = world.resource::<VaelSave>().clone();
    let spawn = save.spawn.unwrap_or([0.0, 150.0, 0.0]);
    let flasks = world
        .get_resource::<VaelUiRules>()
        .map(|rules| flask_max(&save, rules))
        .unwrap_or(crate::respawn::FLASK_BASE);
    crate::respawn::respawn_now(world, Vec3::from_array(spawn), flasks);
}

/// Consome o pedido do fallback da autoridade (sem modal de morte). O patch
/// de vitais já foi escrito pela autoridade; aqui fecha-se o ciclo com o
/// teleporte, os frascos e a troca de cena se estiver num interior.
pub(crate) fn apply_respawn_request(world: &mut World) {
    let Some(request) = world
        .get_resource::<crate::respawn::RespawnRequest>()
        .copied()
    else {
        return;
    };
    if !request.active {
        return;
    }
    if let Some(mut request) = world.get_resource_mut::<crate::respawn::RespawnRequest>() {
        request.active = false;
    }
    let save = world.resource::<VaelSave>().clone();
    let flasks = world
        .get_resource::<VaelUiRules>()
        .map(|rules| flask_max(&save, rules))
        .unwrap_or(crate::respawn::FLASK_BASE);
    crate::respawn::respawn_now(world, Vec3::from_array(request.spawn), flasks);
}

fn death_load(world: &mut World) {
    let Some(best) = world
        .get_resource::<SaveSlotsCache>()
        .and_then(|cache| latest_slot(&cache.slots).cloned())
    else {
        toast(world, "Sem jogos guardados.");
        sfx(world, SfxClip::Error);
        return;
    };
    if let Some(mut request) = world.get_resource_mut::<SaveRequest>() {
        request.save = false;
        request.load = true;
        request.slot = if best.slot == 4 {
            None
        } else {
            Some(best.slot)
        };
    }
    if let Some(writer) = world.get_resource::<UiWriter>() {
        writer.open("death", false);
        writer.set_visible("hud", true);
    }
    let mut state = world.resource_mut::<VaelScreensState>();
    state.hud_visible = true;
    state.pending_title_close = 120;
}

fn death_to_title(world: &mut World) {
    let mut patch = BTreeMap::new();
    patch.insert("dying".to_string(), json!(0));
    patch.insert("death_pending".to_string(), json!(false));
    patch.insert("death_wait".to_string(), json!(0));
    patch_game(world, patch);
    if let Some(writer) = world.get_resource::<UiWriter>() {
        writer.open("death", false);
        writer.open("region-map", false);
        writer.open("travel", false);
        writer.set_visible("hud", false);
        writer.open("title", true);
    }
    world.resource_mut::<VaelScreensState>().hud_visible = false;
    let scene = world.get_resource::<CurrentScene>().cloned();
    if !in_menu_scene(scene.as_ref()) {
        let _ = request_change(world, ChangeScene::new("menu", Arrival::Entry(None)));
    }
}

// ── construção de linhas ────────────────────────────────────────────────

fn lang(save: &VaelSave) -> String {
    save.legacy
        .get("lang")
        .and_then(Value::as_str)
        .unwrap_or("pt")
        .to_string()
}

fn item_label(content: Option<&VaelContent>, id: &str) -> String {
    content
        .and_then(|content| content.items.items.iter().find(|item| item.id == id))
        .map(|item| {
            if item.label.is_empty() {
                id.to_string()
            } else {
                item.label.clone()
            }
        })
        .unwrap_or_else(|| id.to_string())
}

fn item_icon(id: &str) -> Option<String> {
    let icon = match id {
        "gold" => "hud_gold.png",
        "wood" | "dark-wood" => "hud_wood.png",
        "stone" => "hud_stone.png",
        "berry" => "berry.png",
        "korok-seed" => "crystal_blue.png",
        "potion" => "potion_health.png",
        "antidote" => "item_antidote.png",
        "bomb" => "item_bomb.png",
        "moss_potion" => "moss_potion.png",
        "iron_axe" => "iron_axe.png",
        "nature_amulet" => "nature_amulet.png",
        "blessed_rod" => "blessed_rod.png",
        "ancient_relic" => "ancient_relic.png",
        "cactus_fiber" => "cactus_fiber.png",
        "silk_cloth" => "silk_cloth.png",
        "wolf_pelt" => "wolf_pelt.png",
        "steel_sword" => "sword.png",
        "shield_wooden" => "shield_wooden.png",
        _ => return None,
    };
    Some(format!("assets/icons/{icon}"))
}

fn biome_label(biome: &str) -> String {
    match biome {
        "city" => "Discórdia",
        "dark_forest" | "dark-forest" => "Floresta Sombria",
        "desert" => "Ermo",
        "swamp" => "Pântano",
        "frozen_peaks" | "frozen-peaks" => "Picos Gelados",
        value => value,
    }
    .to_string()
}

fn target_label(target: &str) -> String {
    let label = match target {
        "wolf" => "lobos",
        "bandit" => "bandidos",
        "goblin" => "goblins",
        "shade" => "sombras",
        "slime" => "slimes",
        "scorpion" => "escorpiões",
        "bogling" => "boglings",
        "witch" => "a Bruxa do Bosque",
        "sand-worm" => "o Verme da Areia",
        "bog-warden" => "o Guardião do Lodo",
        "boss" => "o Ogro dos Picos",
        "stone" => "pedras",
        "wood" => "madeira",
        "dark-wood" => "madeira escura",
        "bog-moss" => "musgo do pântano",
        "nota" => "Notas",
        "enviado" => "o Enviado de Vidro",
        "sino" => "a Sineira Afogada",
        "defender" => "a Testemunha de Vidro",
        "goblin-brute" => "goblins brutos",
        "bandit-shield" => "bandidos de escudo",
        "bogling-hexer" => "boglings feiticeiros",
        "shade-night" => "sombras da noite",
        "elite" => "elites",
        "wolf_pelt" => "peles de lobo",
        value => return value.to_string(),
    };
    label.to_string()
}

fn objective_line(
    content: Option<&VaelContent>,
    def: &QuestDef,
    state: &str,
    view: &crate::quest::ObjectiveView,
) -> String {
    if state == "ready"
        && let Some(parley) = &def.parley
        && let Some(report) = &parley.report
        && !report.example.is_empty()
    {
        return format!("diz ao NPC: «{}»", report.example);
    }
    let prefix = if view.total > 1 {
        format!("etapa {}/{}: ", view.at, view.total)
    } else {
        String::new()
    };
    if !view.title.is_empty() {
        return format!("{prefix}{}", view.title);
    }
    let count = view.count;
    match view.objective_type.as_str() {
        "craft" => format!(
            "{prefix}forjar {count}× {}",
            item_label(content, &view.target)
        ),
        "kill" => format!("{prefix}derrotar {count}× {}", target_label(&view.target)),
        "collect" => format!("{prefix}reunir {count}× {}", target_label(&view.target)),
        "visit" => format!("{prefix}visitar {count} locais"),
        "decision" => format!("{prefix}convencer {}", target_label(&view.target)),
        other => format!("{prefix}{other}"),
    }
}

fn quest_rows(save: &VaelSave, content: Option<&VaelContent>, rules: &VaelUiRules) -> Vec<RowVm> {
    let kv = quest_kv(save);
    let mut rows = Vec::new();
    let empty = std::collections::BTreeMap::new();
    let quests = content.map(|content| &content.quests).unwrap_or(&empty);
    for (id, def) in quests {
        let id_string = id.to_string();
        let state = quest_state(&rules.quests, &kv, &id_string);
        if state == "not_taken" || state == "unknown" {
            continue;
        }
        let (have, need) = quest_progress(&rules.quests, &kv, &id_string);
        let (progress, progress_text) = if state == "done" {
            (1.0, format!("{need}/{need}"))
        } else {
            (
                if need > 0 {
                    (have as f64 / need as f64).clamp(0.0, 1.0)
                } else {
                    0.0
                },
                format!("{have}/{need}"),
            )
        };
        let view = quest_objective(&rules.quests, &kv, &id_string);
        let status_text = match state.as_str() {
            "ready" => "pronta a entregar",
            "active" => "em curso",
            "done" => "concluída",
            _ => "por aceitar",
        };
        let tracked = save.quests.tracked.as_deref() == Some(id_string.as_str())
            && (state == "active" || state == "ready");
        rows.push(
            RowVm::menu(
                format!("track-{id_string}"),
                if tracked {
                    format!("▶ {}", def.title)
                } else {
                    def.title.clone()
                },
                format!(
                    "{} · {}",
                    objective_line(content, def, &state, &view),
                    biome_label(&def.biome)
                ),
                status_text,
            )
            .with_progress(progress),
        );
        let _ = progress_text;
    }
    rows
}

/// Linhas da aba Talentos: atributos atuais e o que as stats derivadas valem.
///
/// O motor é o mesmo do jogo (`src/stats.rs`, `bevy_gauge`); o menu só lê o
/// snapshot do save, por isso não há uma tabela de conversão paralela.
fn stat_lines(save: &VaelSave, rules: &VaelUiRules, lang: &str) -> (String, String) {
    let learned = learned_set(save);
    let mut state = gear_state(save);
    let mut engine = StatEngine::new(rules.attributes);
    let snapshot = engine.snapshot(&rules.gear, &mut state, &learned);
    let round = |value: f64| value.round() as i64;
    let (keys, damage, health, speed, crit) = if lang == "en" {
        (
            ["STR", "AGI", "VIT", "PRE", "SPI"],
            "Damage",
            "Health",
            "Speed",
            "Crit",
        )
    } else {
        (
            ["FOR", "DES", "VIT", "PRE", "ESP"],
            "Dano",
            "Vida",
            "Vel",
            "Crít",
        )
    };
    let attributes = format!(
        "{} {} · {} {} · {} {} · {} {} · {} {}",
        keys[0],
        round(snapshot.attribute("strength")),
        keys[1],
        round(snapshot.attribute("agility")),
        keys[2],
        round(snapshot.attribute("vitality")),
        keys[3],
        round(snapshot.attribute("precision")),
        keys[4],
        round(snapshot.attribute("spirit")),
    );
    let derived = format!(
        "{damage} +{} · {health} +{} · {speed} +{}% · {crit} +{}%",
        round(snapshot.stat("damage")),
        round(snapshot.stat("max_hp")),
        round(snapshot.stat("speed") * 100.0),
        round(snapshot.stat("crit") * 100.0),
    );
    (attributes, derived)
}

fn bag_rows(save: &VaelSave, content: Option<&VaelContent>, rules: &VaelUiRules) -> Vec<RowVm> {
    let Some(content) = content else {
        return Vec::new();
    };
    let kv = inventory_kv(save);
    let learned = learned_set(save);
    let mut state = gear_state(save);
    // Um único motor de atributos para a lista inteira: `StatEngine::new`
    // cria um `World` e o interner global, não pode nascer por item/frame.
    let mut engine = StatEngine::new(rules.attributes);
    let _ = engine.snapshot(&rules.gear, &mut state, &learned);
    let mut items: Vec<&ItemDef> = content
        .items
        .items
        .iter()
        .filter(|item| vault_get(&rules.vault, &kv, &item.id) > 0 || item.kind == "resource")
        .collect();
    items.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.id.cmp(&b.id)));
    items
        .into_iter()
        .map(|item| {
            let count = vault_get(&rules.vault, &kv, &item.id);
            let icon = item_icon(&item.id);
            let slot = rules.gear.slot_of(&item.id);
            let mut tag = String::new();
            if slot.is_some() && count > 0 {
                if is_equipped(&rules.gear, &mut state, &item.id) {
                    tag = "equipado".to_string();
                } else {
                    let needs = rules
                        .gear
                        .items
                        .get(&item.id)
                        .map(|item| item.requires.clone())
                        .unwrap_or_default();
                    if !needs.trim().is_empty() && !engine.requirement_met(&needs) {
                        let text = rules
                            .gear
                            .items
                            .get(&item.id)
                            .map(|item| item.requires_text.clone())
                            .unwrap_or_default();
                        tag = format!("requer {text}");
                    } else {
                        tag = "equipar".to_string();
                    }
                }
            } else if item.use_item.is_some() && count > 0 {
                tag = "[3]".to_string();
            }
            let mut tip = format!("{} ×{count}", item.label);
            if !item.desc.is_empty() {
                tip.push_str(" — ");
                tip.push_str(&item.desc);
            }
            RowVm::menu(
                format!("bag-{}", item.id),
                item.label.clone(),
                if item.desc.is_empty() {
                    tip
                } else {
                    item.desc.clone()
                },
                tag,
            )
            .with_icon(icon)
        })
        .collect()
}

fn shop_list_rows(save: &VaelSave, rules: &VaelUiRules) -> Vec<RowVm> {
    let kv = inventory_kv(save);
    shop_rows(&rules.shop, &kv)
        .into_iter()
        .map(|row| {
            RowVm::menu(
                format!("shop-{}", row.id),
                if row.selling {
                    format!("{} ×{}", row.label, row.stock)
                } else {
                    row.label.clone()
                },
                String::new(),
                if row.selling {
                    format!("+{}", row.price)
                } else {
                    row.price.to_string()
                },
            )
        })
        .collect()
}

fn forge_rows(save: &VaelSave, content: Option<&VaelContent>, rules: &VaelUiRules) -> Vec<RowVm> {
    let kv = inventory_kv(save);
    rules
        .craft
        .recipes
        .iter()
        .map(|recipe| {
            let status = craft_status(&rules.craft, &kv, &recipe.id)
                .unwrap_or(crate::economy::CraftStatus::Missing);
            let count = recipe.output_count();
            let label = item_label(content, &recipe.out);
            let cost = recipe
                .cost
                .iter()
                .map(|(item, n)| {
                    format!(
                        "{n}× {} ({})",
                        item_label(content, item),
                        vault_get(&rules.vault, &kv, item)
                    )
                })
                .collect::<Vec<_>>()
                .join(" · ");
            let (status_text, note) = match status {
                crate::economy::CraftStatus::Ok => ("forjar", String::new()),
                crate::economy::CraftStatus::Missing => ("falta material", String::new()),
                crate::economy::CraftStatus::Full => ("mochila cheia", String::new()),
            };
            RowVm::menu(
                format!("forge-{}", recipe.id),
                if count > 1 {
                    format!("{label} ×{count}")
                } else {
                    label
                },
                if cost.is_empty() { note } else { cost },
                status_text,
            )
        })
        .collect()
}

fn controls_rows() -> Vec<RowVm> {
    [
        ("W A S D", "mover"),
        ("SHIFT", "correr"),
        ("ESPAÇO", "saltar"),
        ("RATO/J", "atacar"),
        ("C", "esquiva (dash)"),
        ("E", "curar"),
        ("R", "golpe forte"),
        ("L", "guarda / aparar"),
        ("B", "bomba"),
        ("V", "trocar de alvo"),
        ("1 / 2", "poção / antídoto"),
        ("E", "interagir e falar"),
        ("F", "assinar marco da Nota"),
        ("G", "viagem rápida (junto a uma fogueira)"),
        ("Q / ESC", "abrir e fechar este diário"),
        ("1 - 8", "separador do diário (com ele aberto)"),
        ("P", "profiler"),
    ]
    .into_iter()
    .map(|(key, action)| RowVm::menu(String::new(), key, action, String::new()).info())
    .collect()
}

fn a11y_rows(
    a11y: &A11y,
    scale: f32,
    locale: Option<&Locale>,
    mixer: &AudioMixerSettings,
) -> Vec<RowVm> {
    let on_off = |value: bool| if value { "ligado" } else { "desligado" };
    let lang = locale.map(|locale| locale.lang.as_str()).unwrap_or("pt");
    [
        (
            "shake",
            "Tremor da câmara".to_string(),
            on_off(a11y.shake).to_string(),
        ),
        (
            "hitstop",
            "Hit-stop no impacto".to_string(),
            on_off(a11y.hitstop).to_string(),
        ),
        (
            "interact_hold",
            "[E] por hold (0,35 s)".to_string(),
            on_off(a11y.interact_hold).to_string(),
        ),
        (
            "subtitle",
            "Tamanho das legendas".to_string(),
            format!("{:.2}×", a11y.subtitle_scale),
        ),
        (
            "ui_scale",
            "Escala do HUD".to_string(),
            format!("{scale:.2}×"),
        ),
        ("language", "Idioma".to_string(), lang.to_string()),
        (
            "vol-master",
            "Volume geral".to_string(),
            format!("{}%", (mixer.master * 100.0).round() as i32),
        ),
        (
            "vol-music",
            "Volume da música".to_string(),
            format!("{}%", (mixer.music * 100.0).round() as i32),
        ),
        (
            "vol-sfx",
            "Volume dos efeitos".to_string(),
            format!("{}%", (mixer.sfx * 100.0).round() as i32),
        ),
        (
            "vol-ambience",
            "Volume do ambiente".to_string(),
            format!("{}%", (mixer.ambience * 100.0).round() as i32),
        ),
    ]
    .into_iter()
    .map(|(id, label, value)| RowVm::menu(format!("a11y-{id}"), label, String::new(), value))
    .collect()
}

fn system_rows(save: &VaelSave, ui: Option<&UiData>, scene: Option<&CurrentScene>) -> Vec<RowVm> {
    let health = if save.vitals.max_hp > 0.0 {
        format!(
            "{}/{}",
            save.vitals.hp.round() as i64,
            save.vitals.max_hp.round() as i64
        )
    } else {
        "—".to_string()
    };
    let playtime = save
        .legacy
        .get("playtime_secs")
        .and_then(Value::as_f64)
        .and_then(|n| fmt_playtime(Some(n)))
        .unwrap_or_else(|| "—".to_string());
    let zone = ui.map(|ui| ui.zone_name.clone()).unwrap_or_default();
    [
        ("Motor".to_string(), "Viber".to_string()),
        ("Render".to_string(), "Bevy · Rapier".to_string()),
        ("Cena".to_string(), scene_id(scene)),
        (
            "Hora".to_string(),
            ui.map(|ui| ui.clock.clone()).unwrap_or_default(),
        ),
        ("Nível".to_string(), save.progression.level.to_string()),
        ("Vida".to_string(), health),
        ("Ouro".to_string(), save.inventory.gold.to_string()),
        ("Zona".to_string(), zone),
        ("Sessão".to_string(), playtime),
    ]
    .into_iter()
    .map(|(key, value)| RowVm::menu(String::new(), key, String::new(), value).info())
    .collect()
}

fn credits_rows() -> Vec<RowVm> {
    [
        "VAEL — O Silêncio de Vael",
        "Um RPG de mundo aberto feito com a engine Viber.",
        "Mundo, quests e regras: scripts Luau do Vael.",
        "Engine, ferramentas e editor: Viber.",
        "Assets: pipeline AiGameKit.",
    ]
    .into_iter()
    .map(|line| RowVm {
        title: line.to_string(),
        ..Default::default()
    })
    .collect()
}

// ── sistemas ────────────────────────────────────────────────────────────

/// Troca as páginas do diário. O `Visibility` bindado num `ScrollViewer`
/// não colapsa de forma fiável no layout WPF; o sistema escreve `Display`
/// directamente nas páginas nomeadas (oito nós, custo desprezável).
pub fn sync_menu_pages(
    vm: Option<Res<VaelScreensVm>>,
    ui: PfQuery,
    mut pages: Query<(&mut Node, &mut Visibility)>,
) {
    let Some(vm) = vm else {
        return;
    };
    let Some(tabs) = vm.0.read(|m: &ScreensVm| {
        [
            ("page-quests", m.tab_quests),
            ("page-bag", m.tab_bag),
            ("page-skills", m.tab_skills),
            ("page-shop", m.tab_shop),
            ("page-forge", m.tab_forge),
            ("page-controls", m.tab_controls),
            ("page-a11y", m.tab_a11y),
            ("page-system", m.tab_system),
        ]
    }) else {
        return;
    };
    for (name, active) in tabs {
        if let Some(entity) = ui.by_name(name)
            && let Ok((mut node, mut visibility)) = pages.get_mut(entity)
        {
            let wanted = if active { Display::Grid } else { Display::None };
            if node.display != wanted {
                node.display = wanted;
            }
            let wanted_vis = if active {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
            if *visibility != wanted_vis {
                *visibility = wanted_vis;
            }
        }
        let Some(tab) = name.strip_prefix("page-") else {
            continue;
        };
        for suffix in ["-active", "-line"] {
            let name = format!("tab-{tab}{suffix}");
            let Some(entity) = ui.by_name(&name) else {
                continue;
            };
            let Ok((_, mut visibility)) = pages.get_mut(entity) else {
                continue;
            };
            let wanted = if active {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
            if *visibility != wanted {
                *visibility = wanted;
            }
        }
    }
}

pub fn spawn_screens(mut commands: Commands, vm: Res<VaelScreensVm>) {
    let menu = commands.spawn_xaml_bound(include_xaml!("ui/menu.xaml"), vm.0.clone());
    let title = commands.spawn_xaml_bound(include_xaml!("ui/title.xaml"), vm.0.clone());
    let death = commands.spawn_xaml_bound(include_xaml!("ui/death.xaml"), vm.0.clone());
    commands.entity(menu).insert(GlobalZIndex(Z_MENU));
    commands.entity(title).insert(GlobalZIndex(Z_TITLE));
    commands.entity(death).insert(GlobalZIndex(Z_DEATH));
}

pub fn build_rules(
    mut commands: Commands,
    content: Option<Res<VaelContent>>,
    root: Option<Res<crate::VaelProjectRoot>>,
) {
    let Some(content) = content else {
        return;
    };
    commands.insert_resource(VaelUiRules {
        shop: ShopRules::from_items(&content.items),
        craft: CraftRules::from_content(&content.items, &content.recipes),
        gear: GearRules::from_content(&content),
        vault: VaultRules::from_items(&content.items),
        quests: QuestRules::from_content(&content),
        xp: XpRules::from_progression(&content.progression),
        attributes: AttributeRules::default(),
    });
    // Geometria da árvore de talentos: derivada uma vez do conteúdo; os
    // ícones presentes no pool são descobertos aqui para não pedir imagens
    // inexistentes ao AssetServer.
    let mut layout = VaelTalentLayout::build(&content.progression);
    if let Some(root) = root.as_deref() {
        layout.icons = crate::ui::talent_tree::discover_icons(&root.0);
    }
    commands.insert_resource(layout);
}

pub fn hide_legacy_roots(
    registry: Option<Res<viber::ui::UiRegistry>>,
    writer: Option<Res<UiWriter>>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let (Some(registry), Some(writer)) = (registry.as_deref(), writer.as_deref()) else {
        return;
    };
    let roots = ["menu-layer", "title-layer", "death-layer"];
    if roots.iter().all(|id| registry.get(id).is_some()) {
        for id in roots {
            writer.set_visible(id, false);
        }
        *done = true;
        info!("vael UI: raízes declarativas de menu/título/morte escondidas");
    }
}

pub fn refresh_slots(
    time: Res<Time<Real>>,
    key: Option<Res<WorldSaveKey>>,
    dir: Option<Res<SaveDir>>,
    mut cache: ResMut<SaveSlotsCache>,
) {
    cache.timer -= time.delta_secs();
    if cache.timer > 0.0 && !cache.slots.is_empty() {
        return;
    }
    cache.timer = 0.5;
    cache.slots = crate::save_native::list_slots(
        key.as_deref().and_then(|key| key.0.as_deref()),
        dir.as_deref().and_then(|dir| dir.0.as_deref()),
    );
}

/// Repõe escala/idioma/a11y/volumes guardados no `viber.game()` (o antigo
/// `menu.lua` fazia-o; sem ele o save carregado ficava com os defaults).
/// As chaves de preferência que o [`apply_preferences`] aplica; a assinatura
/// de mudança deriva da mesma lista, para a sig e o corpo não divergirem.
const PREF_KEYS: [&str; 10] = [
    "a11y.shake",
    "a11y.hitstop",
    "a11y.interact_hold",
    "a11y.subtitle_scale",
    "ui_scale",
    "lang",
    "vol.master",
    "vol.music",
    "vol.sfx",
    "vol.ambience",
];

pub fn apply_preferences(
    save: Res<VaelSave>,
    mut a11y: ResMut<A11y>,
    mut scale: ResMut<UiUserScale>,
    locale: Option<ResMut<Locale>>,
    mut mixer: ResMut<AudioMixerSettings>,
    mut last: Local<String>,
) {
    let sig = format!("{:?}", PREF_KEYS.map(|key| save.legacy.get(key)));
    if *last == sig {
        return;
    }
    *last = sig;
    if let Some(value) = save.legacy.get("a11y.shake").and_then(Value::as_bool) {
        a11y.shake = value;
    }
    if let Some(value) = save.legacy.get("a11y.hitstop").and_then(Value::as_bool) {
        a11y.hitstop = value;
    }
    if let Some(value) = save
        .legacy
        .get("a11y.interact_hold")
        .and_then(Value::as_bool)
    {
        a11y.interact_hold = value;
    }
    if let Some(value) = save
        .legacy
        .get("a11y.subtitle_scale")
        .and_then(Value::as_f64)
    {
        a11y.subtitle_scale = (value as f32).clamp(0.5, 3.0);
    }
    if let Some(value) = save.legacy.get("ui_scale").and_then(Value::as_f64) {
        scale.0 = (value as f32).clamp(0.5, 3.0);
    }
    if let Some(lang) = save.legacy.get("lang").and_then(Value::as_str)
        && let Some(mut locale) = locale
        && locale.lang != lang
    {
        locale.set_lang(lang);
    }
    if let Some(value) = save.legacy.get("vol.master").and_then(Value::as_f64) {
        mixer.master = (value as f32).clamp(0.0, 1.0);
    }
    if let Some(value) = save.legacy.get("vol.music").and_then(Value::as_f64) {
        mixer.music = (value as f32).clamp(0.0, 1.0);
    }
    if let Some(value) = save.legacy.get("vol.sfx").and_then(Value::as_f64) {
        mixer.sfx = (value as f32).clamp(0.0, 1.0);
    }
    if let Some(value) = save.legacy.get("vol.ambience").and_then(Value::as_f64) {
        mixer.ambience = (value as f32).clamp(0.0, 1.0);
    }
}

/// Abre o título no arranque (ou salta-o em `--entry dev`/QA/interiores),
/// fecha-o depois de um load e troca para a cena `menu` depois do autosave.
///
/// Exclusivo: a troca de cena (`request_change`) precisa de `&mut World`.
pub fn drive_title(world: &mut World) {
    let Some(writer) = world.get_resource::<UiWriter>().cloned() else {
        return;
    };
    if !world.resource::<VaelScreensState>().booted {
        let Some(current) = world.get_resource::<CurrentScene>().cloned() else {
            return;
        };
        world.resource_mut::<VaelScreensState>().booted = true;
        let id = current.id.as_deref().unwrap_or("");
        let has_title = id == "main" || id == "menu";
        let dev = current.entry.as_deref() == Some("dev");
        let open = has_title && !dev;
        writer.open("title", open);
        writer.set_visible("hud", !open);
        world.resource_mut::<VaelScreensState>().hud_visible = !open;
        let mut patch = BTreeMap::new();
        patch.insert("title_done".to_string(), json!(!open));
        patch_game(world, patch);
    }

    if world.resource::<VaelScreensState>().pending_title_close > 0 {
        let remaining = {
            let mut state = world.resource_mut::<VaelScreensState>();
            state.pending_title_close -= 1;
            state.pending_title_close
        };
        if remaining == 0 {
            writer.open("title", false);
            writer.open("credits", false);
            writer.set_visible("hud", true);
            world.resource_mut::<VaelScreensState>().hud_visible = true;
            let mut patch = BTreeMap::new();
            patch.insert("title_done".to_string(), json!(true));
            patch_game(world, patch);
        }
    }

    if world.resource::<VaelScreensState>().pending_menu_scene > 0 {
        let remaining = {
            let mut state = world.resource_mut::<VaelScreensState>();
            state.pending_menu_scene -= 1;
            state.pending_menu_scene
        };
        if remaining == 0 {
            let current = world.get_resource::<CurrentScene>().cloned();
            if !in_menu_scene(current.as_ref())
                && let Err(err) =
                    request_change(world, ChangeScene::new("menu", Arrival::Entry(None)))
            {
                warn!("vael UI: troca para a cena de menu falhou: {err}");
            }
        }
    }

    if world.resource::<VaelScreensState>().menu_settings {
        let menu_open = world
            .get_resource::<UiModalsOpen>()
            .is_some_and(|modals| modals.is_open("menu"));
        let title_open = world
            .get_resource::<UiModalsOpen>()
            .is_some_and(|modals| modals.is_open("title"));
        let hud_visible = world.resource::<VaelScreensState>().hud_visible;
        if !menu_open && !title_open && !hud_visible {
            writer.open("title", true);
            world.resource_mut::<VaelScreensState>().menu_settings = false;
        }
    }
}

/// O diário fecha mapa/viagem ao abrir (o `menu.lua` fazia-o; sem ele os
/// dois overlays podiam ficar empilhados).
pub fn close_map_on_menu(modals: Option<Res<UiModalsOpen>>, writer: Option<Res<UiWriter>>) {
    let (Some(modals), Some(writer)) = (modals.as_deref(), writer.as_deref()) else {
        return;
    };
    if !modals.is_open("menu") {
        return;
    }
    writer.open("region-map", false);
    writer.open("travel", false);
}

#[allow(clippy::too_many_arguments)]
pub fn sync_screens(
    save: Res<VaelSave>,
    content: Option<Res<VaelContent>>,
    rules: Option<Res<VaelUiRules>>,
    layout: Option<Res<VaelTalentLayout>>,
    ui: Option<Res<UiData>>,
    tabs: Option<Res<UiTabs>>,
    modals: Option<Res<UiModalsOpen>>,
    scene: Option<Res<CurrentScene>>,
    slots: Option<Res<SaveSlotsCache>>,
    a11y: Option<Res<A11y>>,
    scale: Option<Res<UiUserScale>>,
    locale: Option<Res<Locale>>,
    mixer: Option<Res<AudioMixerSettings>>,
    mut state: ResMut<VaelScreensState>,
    vm: Res<VaelScreensVm>,
) {
    let content = content.as_deref();
    let rules = rules.as_deref();
    let lang = lang(&save);
    let tab = tabs
        .as_deref()
        .and_then(|tabs| tabs.selected("menu"))
        .unwrap_or("quests")
        .to_string();
    let on_title = in_menu_scene(scene.as_deref());
    let menu_open = modals.as_deref().is_some_and(|m| m.is_open("menu"));

    // Com o diário fechado não há razão para reconstruir ~100 linhas (e o
    // `StatEngine` da mochila) todos os frames.
    let (quests, bag, shop, forge, controls, a11y, system) = if menu_open {
        let quests = rules
            .map(|rules| quest_rows(&save, content, rules))
            .unwrap_or_default();
        let bag = rules
            .map(|rules| bag_rows(&save, content, rules))
            .unwrap_or_default();
        let shop = rules
            .map(|rules| shop_list_rows(&save, rules))
            .unwrap_or_default();
        let forge = rules
            .map(|rules| forge_rows(&save, content, rules))
            .unwrap_or_default();
        let controls = controls_rows();
        let a11y = a11y_rows(
            a11y.as_deref().unwrap_or(&A11y::default()),
            scale.as_deref().map(|scale| scale.0).unwrap_or(1.0),
            locale.as_deref(),
            mixer.as_deref().unwrap_or(&AudioMixerSettings::default()),
        );
        let system = system_rows(&save, ui.as_deref(), scene.as_deref());
        (quests, bag, shop, forge, controls, a11y, system)
    } else {
        (
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    };
    // Só a aba Talentos com o menu aberto precisa do snapshot de atributos;
    // o `sync_screens` corre todos os frames e um `StatEngine` novo por frame
    // era trabalho morto com o menu fechado.
    let stats_visible = tab == "skills" && menu_open;
    let (attributes_line, derived_line) = if stats_visible {
        rules
            .map(|rules| stat_lines(&save, rules, &lang))
            .unwrap_or_default()
    } else {
        (String::new(), String::new())
    };

    // Árvore de talentos: o layout é estático; só os estados e o detalhe
    // mudam. A seleção inicial é a primeira raiz para o painel não nascer
    // vazio quando o jogador abre a aba.
    let learned = learned_set(&save);
    let layout_ref = layout.as_deref();
    if menu_open
        && let Some(layout) = layout_ref
        && !state
            .talent_selected
            .as_deref()
            .is_some_and(|id| layout.node(id).is_some())
    {
        state.talent_selected = layout.roots.first().cloned();
    }
    let selected = state.talent_selected.clone();
    let (talent_nodes_vm, talent_requires, talent_exclusions, talent_w, talent_h) = if menu_open {
        match layout_ref {
            Some(layout) => (
                talent_nodes(layout, &learned, selected.as_deref()),
                layout
                    .requires_edges
                    .iter()
                    .map(TalentEdgeVm::from)
                    .collect(),
                layout
                    .exclusion_edges
                    .iter()
                    .map(TalentEdgeVm::from)
                    .collect(),
                layout.canvas_width,
                layout.canvas_height,
            ),
            None => (Vec::new(), Vec::new(), Vec::new(), 0.0, 0.0),
        }
    } else {
        (Vec::new(), Vec::new(), Vec::new(), 0.0, 0.0)
    };
    let talent_detail_vm = if menu_open {
        content
            .map(|content| {
                let def = selected.as_deref().and_then(|id| {
                    content
                        .progression
                        .skills
                        .iter()
                        .find(|skill| skill.id == id)
                });
                let mut detail = talent_detail(
                    def,
                    &content.progression,
                    &learned,
                    save.progression.skill_points,
                    &lang,
                );
                detail.has_icon = selected
                    .as_deref()
                    .is_some_and(|id| layout_ref.is_some_and(|layout| layout.icons.contains(id)));
                if !detail.has_icon {
                    detail.icon.clear();
                }
                detail
            })
            .unwrap_or_default()
    } else {
        Default::default()
    };
    let title_open = modals.as_deref().is_some_and(|m| m.is_open("title"));
    let credits_open = modals.as_deref().is_some_and(|m| m.is_open("credits"));
    let death_open = modals.as_deref().is_some_and(|m| m.is_open("death"));
    let slots = slots
        .as_deref()
        .map(|cache| cache.slots.as_slice())
        .unwrap_or(&[]);
    let best = latest_slot(slots);
    let save_info = slot_summary(best);

    let live_quests = quests.len();
    let live_bag = bag.len();
    let live_shop = shop.len();
    let live_forge = forge.len();

    vm.0.update(|m: &mut ScreensVm| {
        m.menu_open = menu_open;
        m.menu_stats_visible = !on_title;
        m.menu_title = label(content, &lang, "menu.title", "DIÁRIO");
        m.menu_clock = ui.as_deref().map(|ui| ui.clock.clone()).unwrap_or_default();
        m.menu_hp_label = label(content, &lang, "menu.health", "VIDA");
        m.menu_hp = format!(
            "{}/{}",
            save.vitals.hp.round() as i64,
            save.vitals.max_hp.round() as i64
        );
        m.menu_level_label = label(content, &lang, "menu.level", "NÍVEL");
        m.menu_level = save.progression.level.to_string();
        m.menu_gold_label = label(content, &lang, "menu.gold", "OURO");
        m.menu_gold = save.inventory.gold.to_string();
        m.menu_foot = label(content, &lang, "menu.foot", "Q ou ESC fecha");
        m.menu_points_label = label(
            content,
            &lang,
            "menu.skills.points",
            "Pontos de talento por gastar",
        );
        m.menu_points = save.progression.skill_points.to_string();
        m.menu_attributes = attributes_line.clone();
        m.menu_derived = derived_line.clone();

        m.talent_nodes = talent_nodes_vm.clone();
        m.talent_requires_edges = talent_requires.clone();
        m.talent_exclusion_edges = talent_exclusions.clone();
        m.talent_canvas_width = talent_w;
        m.talent_canvas_height = talent_h;
        m.talent_detail_visible = talent_detail_vm.visible;
        m.talent_selected_icon = talent_detail_vm.icon.clone();
        m.talent_has_selected_icon = talent_detail_vm.has_icon;
        m.talent_selected_title = talent_detail_vm.title.clone();
        m.talent_selected_effects = talent_detail_vm.effects.clone();
        m.talent_selected_requirement = talent_detail_vm.requirement.clone();
        m.talent_selected_status = talent_detail_vm.status.clone();
        m.talent_learn_label = talent_detail_vm.learn_label.clone();
        m.talent_learn_enabled = talent_detail_vm.learn_enabled;

        m.tab_quests = tab == "quests";
        m.tab_bag = tab == "bag";
        m.tab_skills = tab == "skills";
        m.tab_shop = tab == "shop";
        m.tab_forge = tab == "forge";
        m.tab_controls = tab == "controls";
        m.tab_a11y = tab == "a11y";
        m.tab_system = tab == "system";

        m.tab_quests_label = label(content, &lang, "menu.tab.quests", "Missões");
        m.tab_bag_label = label(content, &lang, "menu.tab.bag", "Mochila");
        m.tab_skills_label = label(content, &lang, "menu.tab.skills", "Talentos");
        m.tab_shop_label = label(content, &lang, "menu.tab.shop", "Loja");
        m.tab_forge_label = label(content, &lang, "menu.tab.forge", "Forja");
        m.tab_controls_label = label(content, &lang, "menu.tab.controls", "Controlos");
        m.tab_a11y_label = label(content, &lang, "menu.tab.a11y", "Opções");
        m.tab_system_label = label(content, &lang, "menu.tab.system", "Sistema");

        m.quests_empty = live_quests == 0;
        m.bag_empty = live_bag == 0;
        m.shop_empty = live_shop == 0;
        m.forge_empty = live_forge == 0;
        m.quests_empty_text = label(
            content,
            &lang,
            "menu.quests.empty",
            "Nenhuma missão registada.",
        );
        m.bag_empty_text = label(content, &lang, "menu.bag.empty", "Mochila vazia.");
        m.shop_empty_text = "Sem ofertas.".to_string();
        m.forge_empty_text = "Sem receitas.".to_string();
        m.quests = quests.clone();
        m.bag = bag.clone();
        m.shop = shop.clone();
        m.forge = forge.clone();
        m.controls = controls.clone();
        m.a11y = a11y.clone();
        m.system = system.clone();

        m.save1_label = label(content, &lang, "menu.save1", "Guardar 1");
        m.save2_label = label(content, &lang, "menu.save2", "Guardar 2");
        m.save3_label = label(content, &lang, "menu.save3", "Guardar 3");
        m.load1_label = load_label(1, slots.iter().find(|s| s.slot == 1));
        m.load2_label = load_label(2, slots.iter().find(|s| s.slot == 2));
        m.load3_label = load_label(3, slots.iter().find(|s| s.slot == 3));
        m.save_enabled = !on_title && player_alive(&save);
        let load_ok = |n: u32| {
            slots
                .iter()
                .find(|s| s.slot == n)
                .is_some_and(|s| s.exists && s.scene.as_deref() != Some("menu"))
        };
        m.load1_enabled = load_ok(1);
        m.load2_enabled = load_ok(2);
        m.load3_enabled = load_ok(3);
        m.title_button_label = label(content, &lang, "menu.to_title", "Ecrã de título");
        m.profiler_label = label(content, &lang, "menu.profiler", "Profiler [P]");

        m.title_open = title_open;
        m.credits_open = credits_open;
        m.title_logo = "VAEL".to_string();
        m.title_kicker = label(content, &lang, "title.kicker", "CAPÍTULO I");
        m.title_sub = label(content, &lang, "title.sub", "O Silêncio de Vael");
        m.title_hint = label(content, &lang, "title.hint", "Setas/stick navegam");
        m.title_continue_label = label(content, &lang, "title.continue", "Continuar");
        m.title_new_label = label(content, &lang, "title.new", "Novo Jogo");
        m.title_settings_label = label(content, &lang, "title.settings", "Definições");
        m.title_credits_label = label(content, &lang, "title.credits", "Créditos");
        m.title_quit_label = label(content, &lang, "title.quit", "Sair");
        m.title_back_label = label(content, &lang, "title.back", "Voltar");
        m.title_save_info = save_info.clone();
        m.title_continue_enabled = best.is_some();
        m.credits_title = label(content, &lang, "title.credits.title", "CRÉDITOS");
        m.credits = credits_rows();

        m.death_open = death_open;
        m.death_kicker = "O VALE FICA À ESPERA".to_string();
        m.death_title = label(content, &lang, "death.title", "CAÍSTE");
        m.death_line = label(
            content,
            &lang,
            "death.line",
            "A última fogueira ainda te chama.",
        );
        m.death_save_info = format!("Último save: {save_info}");
        m.death_retry_label = label(content, &lang, "death.retry", "Tentar de novo");
        m.death_load_label = label(content, &lang, "death.load", "Carregar");
        m.death_title_label = label(content, &lang, "death.to_title", "Título");
        m.death_load_enabled = best.is_some();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn death_abre_so_com_death_pending() {
        let mut save = VaelSave::default();
        assert!(!death_should_open(&save, false));
        save.legacy
            .insert("death_pending".into(), Value::Bool(true));
        assert!(death_should_open(&save, false));
        assert!(!death_should_open(&save, true));
    }

    fn slot(n: u32, scene: Option<&str>, saved_at: f64, level: u32) -> SaveSlotInfo {
        SaveSlotInfo {
            slot: n,
            path: format!("slot{n}"),
            exists: true,
            bytes: 1,
            saved_at: Some(saved_at),
            level,
            gold: 0,
            scene: scene.map(str::to_string),
            playtime: Some(125.0),
            title: None,
        }
    }

    #[test]
    fn playtime_formata_horas_e_minutos() {
        assert_eq!(fmt_playtime(Some(0.0)).as_deref(), Some("0 min"));
        assert_eq!(fmt_playtime(Some(125.0)).as_deref(), Some("2 min"));
        assert_eq!(fmt_playtime(Some(3_725.0)).as_deref(), Some("1 h 02 min"));
        assert_eq!(fmt_playtime(Some(-1.0)), None);
        assert_eq!(fmt_playtime(None), None);
    }

    #[test]
    fn inventario_usa_as_chaves_do_cofre() {
        let mut save = VaelSave::default();
        save.inventory.gold = 30;
        save.inventory.items.insert("dark-wood".into(), 2);
        let kv = inventory_kv(&save);
        assert_eq!(kv.get("item.gold"), Some(&30));
        assert_eq!(kv.get("item.dark-wood"), Some(&2));
        assert!(
            !kv.contains_key("gold"),
            "as chaves do cofre vão prefixadas (`item.`); sem prefixo o `vault_get` lia 0"
        );
        let vault = VaultRules {
            stack_max: 99,
            resource_cap: 1_000_000,
            starting: BTreeMap::new(),
            items: BTreeMap::new(),
        };
        assert_eq!(vault_get(&vault, &kv, "gold"), 30);
    }

    #[test]
    fn latest_slot_ignora_a_cena_de_menu_e_escolhe_o_mais_recente() {
        let slots = vec![
            slot(0, Some("main"), 10.0, 3),
            slot(1, Some("menu"), 99.0, 9),
            slot(2, Some("main"), 20.0, 4),
        ];
        let best = latest_slot(&slots).expect("há saves de jogo");
        assert_eq!(best.slot, 2);
        assert_eq!(best.level, 4);
    }

    #[test]
    fn load_label_mostra_nivel_e_tempo_ou_sem_jogo() {
        assert_eq!(
            load_label(2, Some(&slot(2, None, 0.0, 7))),
            "Carregar 2 · Nv 7 · 2 min"
        );
        assert_eq!(
            load_label(1, Some(&slot(1, Some("menu"), 0.0, 3))),
            "Carregar 1 · sem jogo"
        );
        assert_eq!(load_label(3, None), "Carregar 3");
    }

    #[test]
    fn mensagens_do_sistema_de_slots() {
        assert_eq!(slot_summary(None), "Sem jogos guardados");
        assert_eq!(
            slot_summary(Some(&slot(1, None, 0.0, 0))),
            "Jogo guardado · 2 min"
        );
    }
}

#[cfg(test)]
mod xaml_tests {
    use super::*;
    use crate::ui::talent_tree::{TalentEdgeVm, TalentNodeVm};
    use bevy::asset::AssetPlugin;

    fn row(action: &str) -> RowVm {
        RowVm {
            action: action.into(),
            title: "Linha".into(),
            subtitle: "sub".into(),
            tag: "tag".into(),
            progress: 0.5,
            has_progress: true,
            clickable: true,
            icon: String::new(),
            has_icon: false,
        }
    }

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default(), PfUiPlugin));
        app.init_asset::<Image>();
        app.init_asset::<bevy::text::Font>();
        app
    }

    fn instantiate(app: &mut App, scene: XamlScene) -> Entity {
        let root = app.world_mut().spawn_empty().id();
        let doc = scene.document();
        let result = bevy_pf::instantiate_document(app.world_mut(), root, &doc)
            .expect("XAML tem de instanciar");
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        root
    }

    #[test]
    fn menu_xaml_instancia_sem_warnings() {
        let mut app = app();
        let mut model = ScreensVm::default();
        model.menu_open = true;
        model.quests = vec![row("track-a")];
        model.bag = vec![row("bag-gold")];
        model.talent_nodes = vec![TalentNodeVm {
            action: "talent-a".into(),
            size: 64.0,
            has_icon: false,
            ..Default::default()
        }];
        model.talent_requires_edges = vec![TalentEdgeVm {
            x2: 10.0,
            y2: 10.0,
            ..Default::default()
        }];
        model.talent_exclusion_edges = vec![TalentEdgeVm {
            x2: 10.0,
            ..Default::default()
        }];
        model.talent_canvas_width = 800.0;
        model.talent_canvas_height = 400.0;
        model.talent_detail_visible = true;
        model.talent_selected_title = "Talento".into();
        model.talent_selected_effects = "+2 Força".into();
        model.talent_selected_requirement = "Sem requisitos".into();
        model.talent_selected_status = "disponível".into();
        model.talent_learn_label = "Aprender".into();
        model.talent_learn_enabled = true;
        model.shop = vec![row("shop-a")];
        model.forge = vec![row("forge-a")];
        model.controls = vec![row("")];
        model.a11y = vec![row("a11y-shake")];
        model.system = vec![row("")];
        let vm = Bindable::new(model);
        let root = app.world_mut().spawn(DataContext(vm.clone())).id();
        let doc = include_xaml!("ui/menu.xaml").document();
        let result = bevy_pf::instantiate_document(app.world_mut(), root, &doc)
            .expect("menu XAML tem de instanciar");
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        app.world_mut().entity_mut(root).insert(DataContext(vm));
        for _ in 0..3 {
            app.update();
        }
    }

    #[test]
    fn tab_ativa_troca_de_pagina_e_destaque() {
        let mut app = app();
        let mut model = ScreensVm::default();
        model.tab_quests = true;
        let vm = Bindable::new(model);
        let root = app.world_mut().spawn(DataContext(vm.clone())).id();
        let doc = include_xaml!("ui/menu.xaml").document();
        let result = bevy_pf::instantiate_document(app.world_mut(), root, &doc)
            .expect("menu XAML tem de instanciar");
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        app.world_mut()
            .entity_mut(root)
            .insert(DataContext(vm.clone()));
        app.insert_resource(VaelScreensVm(vm.clone()))
            .add_systems(Update, sync_menu_pages);
        app.update();
        let (quests_page, bag_page, quests_active, bag_active) = {
            let names = app.world().get::<XamlNames>(root).expect("namescope");
            (
                names.get("page-quests").expect("page quests"),
                names.get("page-bag").expect("page bag"),
                names.get("tab-quests-active").expect("destaque quests"),
                names.get("tab-bag-active").expect("destaque bag"),
            )
        };
        {
            let quests = app
                .world()
                .get::<Visibility>(quests_page)
                .copied()
                .unwrap_or(Visibility::Inherited);
            let bag = app
                .world()
                .get::<Visibility>(bag_page)
                .copied()
                .unwrap_or(Visibility::Inherited);
            assert_ne!(quests, Visibility::Hidden);
            assert_eq!(bag, Visibility::Hidden);
        }

        vm.update(|m: &mut ScreensVm| {
            m.tab_quests = false;
            m.tab_bag = true;
        });
        for _ in 0..3 {
            app.update();
        }
        {
            let quests = app
                .world()
                .get::<Visibility>(quests_page)
                .copied()
                .unwrap_or(Visibility::Inherited);
            let bag = app
                .world()
                .get::<Visibility>(bag_page)
                .copied()
                .unwrap_or(Visibility::Inherited);
            assert_eq!(quests, Visibility::Hidden);
            assert_ne!(bag, Visibility::Hidden);
        }
        assert_eq!(
            app.world().get::<Visibility>(quests_active).copied(),
            Some(Visibility::Hidden)
        );
        assert_ne!(
            app.world().get::<Visibility>(bag_active).copied(),
            Some(Visibility::Hidden)
        );
    }

    #[test]
    fn title_e_creditos_xaml_instanciam_sem_warnings() {
        let mut app = app();
        let mut model = ScreensVm::default();
        model.title_open = true;
        model.credits_open = true;
        model.credits = vec![row("")];
        let vm = Bindable::new(model);
        let root = app.world_mut().spawn(DataContext(vm.clone())).id();
        let doc = include_xaml!("ui/title.xaml").document();
        let result = bevy_pf::instantiate_document(app.world_mut(), root, &doc)
            .expect("title XAML tem de instanciar");
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        app.world_mut().entity_mut(root).insert(DataContext(vm));
        app.update();
    }

    #[test]
    fn death_xaml_instancia_sem_warnings() {
        let mut app = app();
        let mut model = ScreensVm::default();
        model.death_open = true;
        let vm = Bindable::new(model);
        let root = app.world_mut().spawn(DataContext(vm.clone())).id();
        let doc = include_xaml!("ui/death.xaml").document();
        let result = bevy_pf::instantiate_document(app.world_mut(), root, &doc)
            .expect("death XAML tem de instanciar");
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        app.world_mut().entity_mut(root).insert(DataContext(vm));
        app.update();
    }

    #[test]
    fn instantiate_helper_compila() {
        let mut app = app();
        let _ = instantiate(&mut app, include_xaml!("ui/hud.xaml"));
    }
}

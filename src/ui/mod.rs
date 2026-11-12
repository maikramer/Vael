//! UI do Vael em `bevy_pf` (XAML/WPF sobre `bevy_ui`).
//!
//! O HUD é um documento XAML instanciado no ECS, com bindings por reflexão
//! para [`HudVm`]. A fonte dos valores continua a ser o estado do jogo: o
//! [`VaelSave`] espelhado do Lua, o [`VaelContent`] e os binds que o
//! `rules.lua`/`hud.lua` publicam; [`UiData`] traz relógio, zona, prompt,
//! toast e chuva.
//!
//! Gate: `VAEL_UI_PF=0` desliga o HUD/XAML (fica só a barra de fala da
//! engine; não há fallback declarativo).

use bevy::prelude::*;
use bevy::reflect::Reflect;
use bevy_pf::prelude::*;
use serde_json::Value;
use viber::ui::UiData;
use viber::ui::UiModalsOpen;
use viber::ui::script::UiScriptBinds;
use viber::ui::writer::UiWriter;

use crate::content::VaelContent;
use crate::hud::{OTHER_ROWS, best_quest, other_quest_rows, others_more_text};
use crate::save::VaelSave;

pub mod map;
pub mod screens;
pub mod talent_tree;
pub mod waypoint;

/// Variável de ambiente do gate (`1` = XAML ligado).
pub const UI_PF_ENV: &str = "VAEL_UI_PF";

/// Uma linha da lista "outras missões" do HUD.
#[derive(Reflect, Default, Clone, Debug, PartialEq, Eq)]
pub struct QuestRowVm {
    pub text: String,
    pub ready: bool,
}

/// View-model do HUD. Os campos `lbl_*` são os rótulos já localizados; os
/// restantes são os valores publicados pelo jogo.
#[derive(Reflect, Default, Clone)]
pub struct HudVm {
    pub hud_visible: bool,
    pub vitals_visible: bool,

    pub health: f64,
    pub health_text: String,
    pub health_low: bool,
    pub xp: f64,
    pub xp_text: String,
    pub level_text: String,
    pub talent_ready: bool,

    pub status_venom: bool,
    pub status_burn: bool,
    pub status_frost: bool,

    pub gold: String,
    pub wood: String,
    pub stone: String,
    pub berry: String,
    pub seeds: String,

    pub flask: String,
    pub potion: String,
    pub antidote: String,
    pub moss: String,
    pub bomb: String,

    pub quest_active: bool,
    pub quest_title: String,
    pub quest: f64,
    pub quest_text: String,
    pub quests_others: bool,
    pub quest_others: Vec<QuestRowVm>,
    pub quest_others_more: String,
    pub quest_others_more_visible: bool,

    pub target_alive: bool,
    pub target_name: String,
    pub target: f64,

    pub boss_active: bool,
    pub boss_name: String,
    pub boss: f64,
    pub boss_enraged: bool,

    pub combo_active: bool,
    pub combo_text: String,

    pub clock: String,
    pub day: f64,
    pub zone_name: String,
    pub wet: bool,
    pub rain: f64,
    pub net_on: bool,
    pub net_label: String,

    pub dash_cd: f64,
    pub dash_ready: bool,
    pub dash_opacity: f64,
    pub heal_cd: f64,
    pub heal_ready: bool,
    pub heal_opacity: f64,
    pub strike_cd: f64,
    pub strike_ready: bool,
    pub strike_opacity: f64,
    pub potion_opacity: f64,
    pub antidote_opacity: f64,
    pub moss_opacity: f64,
    pub bomb_opacity: f64,

    pub prompt_active: bool,
    pub prompt_key: String,
    pub prompt_label: String,
    pub toast_active: bool,
    pub toast_text: String,
    pub zone_active: bool,

    pub dmg_front: bool,
    pub dmg_back: bool,
    pub dmg_left: bool,
    pub dmg_right: bool,

    pub lbl_health: String,
    pub lbl_low: String,
    pub lbl_venom: String,
    pub lbl_venom_hint: String,
    pub lbl_burn: String,
    pub lbl_moss_hint: String,
    pub lbl_frost: String,
    pub lbl_gold: String,
    pub lbl_wood: String,
    pub lbl_stone: String,
    pub lbl_berry: String,
    pub lbl_seeds: String,
    pub lbl_journal: String,
    pub lbl_quest: String,
    pub lbl_boss: String,
    pub lbl_rage: String,
    pub lbl_clock: String,
    pub lbl_net: String,
    pub lbl_rain: String,
    pub lbl_attack: String,
    pub lbl_guard: String,
    pub lbl_dash: String,
    pub lbl_flask: String,
    pub lbl_strike: String,
    pub lbl_potion: String,
    pub lbl_antidote: String,
    pub lbl_moss: String,
    pub lbl_bomb: String,
    pub lbl_explore: String,

    pub open_journal: RelayCommand,
}

/// Handle partilhado do view-model do HUD.
#[derive(Resource, Clone)]
pub struct VaelHudVm(pub Bindable);

/// Plugin da UI do Vael. Tem de ser adicionado DEPOIS do `DefaultPlugins`
/// (main), porque o `PfUiPlugin` regista fonts/assets/UI widgets.
pub struct VaelUiPlugin;

impl Plugin for VaelUiPlugin {
    fn build(&self, app: &mut App) {
        if !enabled() {
            info!("vael UI: bevy_pf desligado por {UI_PF_ENV}");
            return;
        }
        let mut vm = HudVm::default();
        vm.open_journal = RelayCommand::new(|world, _| {
            if let Some(writer) = world.get_resource::<UiWriter>() {
                writer.open("menu", true);
                writer.select_tab("menu", "quests");
            }
        });
        let screens = screens::ScreensVm::with_commands();
        let map = map::MapVm::with_commands();
        app.add_plugins(PfUiPlugin)
            .insert_resource(VaelHudVm(Bindable::new(vm)))
            .insert_resource(screens::VaelScreensVm(Bindable::new(screens)))
            .insert_resource(map::VaelMapVm(Bindable::new(map)))
            .init_resource::<map::MapTravelState>()
            .init_resource::<screens::SaveSlotsCache>()
            .init_resource::<screens::VaelScreensState>()
            .add_systems(
                Startup,
                (
                    spawn_hud.after(load_skin_fonts),
                    screens::spawn_screens.after(load_skin_fonts),
                    screens::build_rules,
                    map::spawn_map.after(load_skin_fonts),
                    load_skin_fonts,
                ),
            )
            .add_systems(
                Update,
                (
                    sync_hud.before(PfBindingSystems),
                    screens::sync_screens.before(PfBindingSystems),
                    screens::sync_menu_pages,
                    screens::hide_legacy_roots,
                    screens::refresh_slots,
                    screens::apply_preferences,
                    screens::drive_title,
                    screens::close_map_on_menu,
                    map::map_input,
                    map::sync_map.before(PfBindingSystems),
                ),
            );
    }
}

/// Faces da skin. O `Assets<Font>` do Bevy 0.20 remove um asset cujo `Handle`
/// forte deixa de existir no update seguinte; guardar os handles aqui é o que
/// mantém a família `Noto Serif Display` viva para o `FontFamily` do XAML.
#[derive(Resource)]
struct VaelSkinFonts {
    _faces: Vec<Handle<bevy::text::Font>>,
}

/// Regista as fontes da skin (Noto Serif Display, OFL) em `Assets<Font>` de
/// forma síncrona — sem loader assíncrono — e segura os handles.
fn load_skin_fonts(
    mut commands: Commands,
    fonts: Option<ResMut<Assets<bevy::text::Font>>>,
) {
    let Some(mut fonts) = fonts else {
        return;
    };
    let faces = vec![
        fonts.add(bevy::text::Font::from_bytes(
            include_bytes!("../../ui/fonts/NotoSerifDisplay-Regular.ttf").to_vec(),
        )),
        fonts.add(bevy::text::Font::from_bytes(
            include_bytes!("../../ui/fonts/NotoSerifDisplay-Bold.ttf").to_vec(),
        )),
    ];
    commands.insert_resource(VaelSkinFonts { _faces: faces });
}

/// Gate do ambiente, relido uma vez no arranque.
pub fn enabled() -> bool {
    match std::env::var(UI_PF_ENV) {
        Ok(raw) => !matches!(
            raw.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off" | "no"
        ),
        Err(_) => true,
    }
}

fn hud_xaml() -> XamlScene {
    include_xaml!("ui/hud.xaml")
}

fn spawn_hud(mut commands: Commands, vm: Res<VaelHudVm>) {
    commands.spawn_xaml_bound(hud_xaml(), vm.0.clone());
}

/// `math.floor(v + 0.5)` do `rules.lua` (`%d` arredonda assim).
fn round(v: f64) -> i64 {
    (v + 0.5).floor() as i64
}

fn bind_text(binds: Option<&UiScriptBinds>, name: &str) -> String {
    binds
        .and_then(|b| b.get(name))
        .map(|v| v.text())
        .unwrap_or_default()
}

fn bind_number(binds: Option<&UiScriptBinds>, name: &str) -> f64 {
    binds
        .and_then(|b| b.get(name))
        .map(|v| v.number() as f64)
        .unwrap_or(0.0)
}

fn bind_flag(binds: Option<&UiScriptBinds>, name: &str) -> bool {
    binds
        .and_then(|b| b.get(name))
        .map(|v| v.flag())
        .unwrap_or(false)
}

fn legacy_number(save: &VaelSave, key: &str) -> f64 {
    save.legacy
        .get(key)
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite())
        .unwrap_or(0.0)
}

fn legacy_text(save: &VaelSave, key: &str) -> String {
    save.legacy
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn label(content: Option<&VaelContent>, lang: &str, key: &str, fallback: &str) -> String {
    content
        .and_then(|content| content.locale.get(lang))
        .and_then(|table| table.get(key))
        .cloned()
        .unwrap_or_else(|| fallback.to_string())
}

/// Sistema de sincronização: HUD lê o estado e publica no view-model.
#[allow(clippy::too_many_arguments)]
fn sync_hud(
    save: Res<VaelSave>,
    content: Option<Res<VaelContent>>,
    binds: Option<Res<UiScriptBinds>>,
    ui: Option<Res<UiData>>,
    modals: Option<Res<UiModalsOpen>>,
    time: Res<Time>,
    vm: Res<VaelHudVm>,
) {
    let content = content.as_deref();
    let binds = binds.as_deref();
    let ui = ui.as_deref();
    let lang = save
        .legacy
        .get("lang")
        .and_then(Value::as_str)
        .unwrap_or("pt");

    let hp = save.vitals.hp as f64;
    let max_hp = save.vitals.max_hp.max(1.0) as f64;
    let health = (hp / max_hp).clamp(0.0, 1.0);
    let xp_next = save.progression.xp_next.max(1) as f64;
    let xp = (save.progression.xp as f64 / xp_next).clamp(0.0, 1.0);

    let (quest_title, quest, quest_text) = if let Some(content) = content {
        best_quest(&save, content)
    } else {
        (String::new(), 0.0, String::new())
    };
    let rows = content
        .map(|content| other_quest_rows(&save, content, &quest_title))
        .unwrap_or_default();
    let visible_rows: Vec<QuestRowVm> = rows
        .iter()
        .take(OTHER_ROWS)
        .map(|row| QuestRowVm {
            text: row.text(),
            ready: row.ready,
        })
        .collect();
    let extra = rows.len().saturating_sub(OTHER_ROWS);

    let flasks = save
        .legacy
        .get("flasks")
        .and_then(Value::as_f64)
        .map(|n| n.floor().max(0.0))
        .unwrap_or(2.0);

    let potion = save.inventory.get("potion");
    let antidote = save.inventory.get("antidote");
    let moss = save.inventory.get("moss_potion");
    let bomb = save.inventory.get("bomb");

    let dash_ready = bind_flag(binds, "ab.dash.ready");
    let heal_ready = bind_flag(binds, "ab.heal.ready");
    let strike_ready = bind_flag(binds, "ab.strike.ready");

    let edge = legacy_text(&save, "dmg.edge");
    let edge_until = legacy_number(&save, "dmg.until");
    let edge_live = edge_until > time.elapsed_secs_f64();

    let now_modals = modals.as_deref();
    let hud_visible = !now_modals.is_some_and(|m| m.is_open("title") || m.is_open("death"));

    vm.0.update(|m: &mut HudVm| {
        m.hud_visible = hud_visible;
        m.vitals_visible = true;

        m.health = health;
        m.health_text = format!("{}/{}", round(hp), round(max_hp));
        m.health_low = health <= 0.3;
        m.xp = xp;
        m.xp_text = format!("{}/{}", save.progression.xp, save.progression.xp_next);
        m.level_text = save.progression.level.to_string();
        m.talent_ready = save.progression.skill_points > 0;

        m.status_venom = legacy_number(&save, "venom") > 0.0;
        m.status_burn = legacy_number(&save, "burn") > 0.0;
        m.status_frost = legacy_number(&save, "frost") > 0.0;

        m.gold = save.inventory.gold.to_string();
        m.wood = save.inventory.wood.to_string();
        m.stone = save.inventory.stone.to_string();
        m.berry = save.inventory.berry.to_string();
        m.seeds = save.inventory.get("korok_seed").to_string();

        m.flask = flasks.to_string();
        m.potion = potion.to_string();
        m.antidote = antidote.to_string();
        m.moss = moss.to_string();
        m.bomb = bomb.to_string();

        m.quest_active = !quest_title.is_empty();
        m.quest_title = quest_title.clone();
        m.quest = quest;
        m.quest_text = quest_text;
        m.quests_others = !visible_rows.is_empty();
        m.quest_others = visible_rows.clone();
        m.quest_others_more = others_more_text(extra);
        m.quest_others_more_visible = extra > 0;

        m.target_alive = bind_flag(binds, "target.alive");
        m.target_name = bind_text(binds, "target.name");
        m.target = bind_number(binds, "target").clamp(0.0, 1.0);

        m.boss_active = bind_flag(binds, "boss.active");
        m.boss_name = bind_text(binds, "boss.name");
        m.boss = bind_number(binds, "boss").clamp(0.0, 1.0);
        m.boss_enraged = bind_flag(binds, "boss.enraged");

        m.combo_active = bind_flag(binds, "combo");
        m.combo_text = bind_text(binds, "combo.text");

        if let Some(ui) = ui {
            m.clock = ui.clock.clone();
            m.day = ui.day_fraction.clamp(0.0, 1.0) as f64;
            m.zone_name = if ui.zone_name.is_empty() {
                label(content, lang, "hud.zone", "O Vale")
            } else {
                ui.zone_name.clone()
            };
            m.wet = ui.weather_rain > 0.05;
            m.rain = ui.weather_rain.clamp(0.0, 1.0) as f64;
            m.prompt_active = !ui.prompt_label.is_empty();
            m.prompt_key = ui.prompt_key.clone();
            m.prompt_label = ui.prompt_label.clone();
            m.toast_active = ui.toast_time > 0.0 && !ui.toast.is_empty();
            m.toast_text = ui.toast.clone();
            m.zone_active = ui.zone_reveal > 0.0 && !ui.zone_name.is_empty();
        }
        m.net_on = bind_flag(binds, "net.on");
        m.net_label = bind_text(binds, "net.label");

        m.dash_cd = bind_number(binds, "ab.dash").clamp(0.0, 1.0);
        m.heal_cd = bind_number(binds, "ab.heal").clamp(0.0, 1.0);
        m.strike_cd = bind_number(binds, "ab.strike").clamp(0.0, 1.0);
        m.dash_ready = dash_ready;
        m.heal_ready = heal_ready;
        m.strike_ready = strike_ready;
        m.dash_opacity = if dash_ready { 1.0 } else { 0.4 };
        m.heal_opacity = if heal_ready { 1.0 } else { 0.4 };
        m.strike_opacity = if strike_ready { 1.0 } else { 0.4 };
        m.potion_opacity = if potion > 0 { 1.0 } else { 0.35 };
        m.antidote_opacity = if antidote > 0 { 1.0 } else { 0.35 };
        m.moss_opacity = if moss > 0 { 1.0 } else { 0.35 };
        m.bomb_opacity = if bomb > 0 { 1.0 } else { 0.35 };

        m.dmg_front = edge_live && edge == "dmg-front";
        m.dmg_back = edge_live && edge == "dmg-back";
        m.dmg_left = edge_live && edge == "dmg-left";
        m.dmg_right = edge_live && edge == "dmg-right";

        m.lbl_health = label(content, lang, "hud.health", "VIDA");
        m.lbl_low = label(content, lang, "hud.low", "VIDA BAIXA");
        m.lbl_venom = label(content, lang, "hud.venom", "ENVENENADO");
        m.lbl_venom_hint = label(content, lang, "hud.venom.hint", "antídoto [2]");
        m.lbl_burn = label(content, lang, "hud.burn", "A ARDER");
        m.lbl_moss_hint = label(content, lang, "hud.moss.hint", "musgo [3]");
        m.lbl_frost = label(content, lang, "hud.frost", "GELADO");
        m.lbl_gold = label(content, lang, "hud.gold", "OURO");
        m.lbl_wood = label(content, lang, "hud.wood", "MAD.");
        m.lbl_stone = label(content, lang, "hud.stone", "PEDRA");
        m.lbl_berry = label(content, lang, "hud.berry", "FRUTAS");
        m.lbl_seeds = label(content, lang, "hud.seeds", "SEMENTES");
        m.lbl_journal = label(content, lang, "hud.journal", "DIÁRIO");
        m.lbl_quest = label(content, lang, "hud.quest", "MISSÃO ATIVA");
        m.lbl_boss = label(content, lang, "hud.boss", "CHEFE");
        m.lbl_rage = label(content, lang, "hud.rage", "FÚRIA");
        m.lbl_clock = label(content, lang, "hud.clock", "HORA LOCAL");
        m.lbl_net = label(content, lang, "hud.net", "REDE");
        m.lbl_rain = label(content, lang, "hud.rain", "CHUVA");
        m.lbl_attack = label(content, lang, "hud.attack", "J  ATACAR");
        m.lbl_guard = label(content, lang, "hud.guard", "L  DEFENDER");
        m.lbl_dash = label(content, lang, "hud.dash", "Esquiva");
        m.lbl_flask = label(content, lang, "hud.flask", "Cantil");
        m.lbl_strike = label(content, lang, "hud.strike", "Impacto");
        m.lbl_potion = label(content, lang, "hud.potion", "Poção");
        m.lbl_antidote = label(content, lang, "hud.antidote", "Antídoto");
        m.lbl_moss = label(content, lang, "hud.moss", "Musgo");
        m.lbl_bomb = label(content, lang, "hud.bomb", "Bomba");
        m.lbl_explore = label(content, lang, "hud.explore", "EXPLORAR");
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::AssetPlugin;

    #[test]
    fn skin_font_regista_a_familia_no_primeiro_update() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            bevy::text::TextPlugin,
        ));
        app.init_asset::<Image>();
        // Os handles têm de sobreviver ao update: o `Assets<Font>` remove um
        // font cujo handle forte morre (era o que apagava a skin antes de o
        // `FontCx` a registar).
        let faces: Vec<_> = [
            &include_bytes!("../../ui/fonts/NotoSerifDisplay-Regular.ttf")[..],
            &include_bytes!("../../ui/fonts/NotoSerifDisplay-Bold.ttf")[..],
        ]
        .into_iter()
        .map(|bytes| {
            app.world_mut()
                .resource_mut::<Assets<bevy::text::Font>>()
                .add(bevy::text::Font::from_bytes(bytes.to_vec()))
        })
        .collect();
        app.update();
        let world = app.world_mut();
        let mut font_cx = world.resource_mut::<bevy::text::FontCx>();
        let nomes: Vec<String> = font_cx
            .collection
            .family_names()
            .map(str::to_string)
            .collect();
        assert!(
            font_cx.collection.family_id("Noto Serif Display").is_some(),
            "a família da skin tem de ficar registada para o `FontFamily` do XAML resolver; registadas: {nomes:?}"
        );
        drop(faces);
    }

    #[test]
    fn hud_xaml_instancia_sem_warnings() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default(), PfUiPlugin));
        app.init_asset::<Image>();
        app.init_asset::<bevy::text::Font>();
        let doc = hud_xaml().document();
        let root = app.world_mut().spawn_empty().id();
        let result = bevy_pf::instantiate_document(app.world_mut(), root, &doc)
            .expect("o HUD XAML tem de instanciar");
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    }
}

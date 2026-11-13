//! Mapa regional e viagem rápida em `bevy_pf`, com o controlador em Rust.
//!
//! O input M/G/F/Esc, o pin, a assinatura dos marcos, a viagem e a UI são
//! deste módulo; o waypoint/marcadores do HUD vêm de [`super::waypoint`]. O
//! `map.lua`/`world/explore.xml` e o `travel.lua` foram removidos.

use bevy::prelude::*;
use bevy::ui::GlobalZIndex;
use bevy_pf::prelude::*;
use serde_json::{Value, json};
use viber::ambient::{SfxClip, SfxEvent};
use bevy_ext_core::HudMarkers;
use viber::luau::components::ScriptToast;
use viber::luau::host::LuaScriptHost;
use bevy_ext_core::MenusOpen;
use viber::player::{Player, TeleportSettle};
use viber::terrain::runtime::TerrainRuntime;
use viber::ui::UiModalsOpen;
use viber::ui::widgets::UiFocusedInput;

use crate::content::{VaelContent, label};
use crate::save::VaelSave;
use crate::travel::{
    TravelState, VaelTravelRules, available, distance_xz, is_revealed, is_surveyed,
};
use crate::ui::screens::{RowVm, VaelUiRules, quest_kv};

/// Z da camada do mapa (acima do HUD, abaixo do título/morte).
const Z_MAP: i32 = 150;
/// Lado do canvas do mapa em píxeis (a imagem é quadrada).
const CANVAS: f32 = 460.0;
/// Raio do marcador; o centro cai no ponto projetado.
const MARKER_R: f32 = 10.0;

/// Estado de abertura dos dois painéis.
#[derive(Resource, Default, Clone, Copy)]
pub struct MapTravelState {
    pub map_open: bool,
    pub travel_open: bool,
}

impl MapTravelState {
    pub fn any(&self) -> bool {
        self.map_open || self.travel_open
    }
}

/// View-model do mapa/viagem.
#[derive(Reflect, Default, Clone)]
pub struct MapVm {
    pub map_open: bool,
    pub travel_open: bool,
    pub map_title: String,
    pub map_sub: String,
    pub map_hint: String,
    pub map_empty_text: String,
    pub travel_title: String,
    pub travel_sub: String,
    pub travel_empty_text: String,
    pub map_rows: Vec<RowVm>,
    pub travel_rows: Vec<RowVm>,
    pub map_rows_empty: bool,
    pub travel_rows_empty: bool,
    pub player_margin: String,
    pub player_visible: bool,
    pub waypoint_margin: String,
    pub waypoint_label: String,
    pub waypoint_visible: bool,
    pub map_command: RelayCommand,
}

#[derive(Resource, Clone)]
pub struct VaelMapVm(pub Bindable);

impl MapVm {
    pub fn with_commands() -> Self {
        let mut vm = Self::default();
        vm.map_command = RelayCommand::new(|world, param| {
            if let Some(id) = param {
                handle_map_action(world, &id);
            }
        });
        vm
    }
}

fn toast(world: &mut World, text: impl Into<String>) {
    if world.contains_resource::<Messages<ScriptToast>>() {
        world.write_message(ScriptToast(text.into()));
    }
}

fn sfx(world: &mut World, clip: SfxClip, position: Option<Vec3>) {
    if world.contains_resource::<Messages<SfxEvent>>() {
        world.write_message(SfxEvent { clip, position });
    }
}

fn patch_game(world: &mut World, patch: std::collections::BTreeMap<String, Value>) {
    if let Some(host) = world.get_resource::<LuaScriptHost>() {
        viber::luau::game::apply_patch(&host.lua, &patch);
    }
}

fn player_position(world: &mut World) -> Option<[f32; 3]> {
    let mut query = world.query_filtered::<&GlobalTransform, With<Player>>();
    query.iter(world).next().map(|transform| {
        let p = transform.translation();
        [p.x, p.y, p.z]
    })
}

fn near_fire(world: &mut World, player: [f32; 3], range: f32) -> bool {
    let mut query = world.query::<(&Name, &GlobalTransform)>();
    query.iter(world).any(|(name, transform)| {
        name.as_str().contains("campfire")
            && transform.translation().distance(Vec3::from_array(player)) <= range
    })
}

fn mark_position(world: &World, name: &str) -> Option<[f32; 3]> {
    world
        .get_resource::<TravelState>()
        .and_then(|state| state.get().positions.get(name).copied())
}

fn sign_nearest(world: &mut World) {
    let Some(rules) = world.get_resource::<VaelTravelRules>().map(|r| r.0.clone()) else {
        return;
    };
    let Some(player) = player_position(world) else {
        return;
    };
    let save = world.resource::<VaelSave>().clone();
    let positions = world
        .get_resource::<TravelState>()
        .map(|state| state.get().positions)
        .unwrap_or_default();
    let mut best: Option<(String, f64)> = None;
    let mut locked: Option<(String, f64)> = None;
    for mark in &rules.landmarks {
        if is_surveyed(&save, &mark.name) {
            continue;
        }
        let Some(position) = positions.get(&mark.name) else {
            continue;
        };
        let dx = position[0] - player[0];
        let dy = position[1] - player[1];
        let dz = position[2] - player[2];
        let distance = ((dx * dx + dy * dy + dz * dz) as f64).sqrt();
        if mark.radius <= 0.0 || distance > mark.radius {
            continue;
        }
        let revealed = is_revealed(&save, &mark.biome);
        if available(mark, revealed) {
            if best.as_ref().is_none_or(|(_, d)| distance < *d) {
                best = Some((mark.name.clone(), distance));
            }
        } else if locked.as_ref().is_none_or(|(_, d)| distance < *d) {
            locked = Some((mark.name.clone(), distance));
        }
    }
    let Some((name, _)) = best else {
        if let Some((name, _)) = locked {
            let mark = rules.get(&name);
            let label = mark
                .and_then(|mark| rules.biome_label(&mark.biome))
                .unwrap_or("um bioma por cartografar");
            toast(
                world,
                format!("A terra aqui ainda não foi cartografada — procura a atalaia de {label}."),
            );
        }
        return;
    };
    let mut patch = std::collections::BTreeMap::new();
    patch.insert(format!("mark.{name}"), json!(true));
    patch_game(world, patch);
    // `report_visit` é a mesma função Rust que o scan de visitas usa.
    if let Some(ui_rules) = world.get_resource::<VaelUiRules>().cloned() {
        let mut kv = quest_kv(world.resource::<VaelSave>());
        let changed = crate::quest_events::report_visit(&ui_rules.quests, &mut kv, &name);
        if !changed.is_empty() {
            let patch = kv
                .into_iter()
                .filter(|(key, _)| key.starts_with("q."))
                .collect::<std::collections::BTreeMap<_, _>>();
            patch_game(world, patch);
        }
    }
    let mark = rules.get(&name);
    let label = mark
        .map(|mark| mark.label.clone())
        .unwrap_or_else(|| name.clone());
    let biome = mark.map(|mark| mark.biome.clone()).unwrap_or_default();
    if biome == "tower" {
        toast(
            world,
            format!("Atalaia cartografada: {label} — viaja com [G] na fogueira."),
        );
    } else {
        let biome_label = rules
            .biome_label(&biome)
            .unwrap_or(biome.as_str())
            .to_string();
        let left = rules
            .landmarks
            .iter()
            .filter(|mark| {
                mark.biome == biome
                    && available(mark, is_revealed(&save, &mark.biome))
                    && !is_surveyed(&save, &mark.name)
                    && mark.name != name
            })
            .count();
        let suffix = match left {
            0 => format!(" — {biome_label} sem marcos por assinar!"),
            1 => format!(" (falta 1 marco: {biome_label})"),
            n => format!(" (faltam {n} marcos: {biome_label})"),
        };
        toast(world, format!("Medido e assinado: {label}{suffix}"));
    }
    let position = mark_position(world, &name).map(Vec3::from_array);
    sfx(world, SfxClip::QuestDone, position);
}

fn pin_mark(world: &mut World, name: &str) {
    let Some(rules) = world.get_resource::<VaelTravelRules>().map(|r| r.0.clone()) else {
        return;
    };
    let Some(mark) = rules.get(name) else {
        toast(world, "Não conheço esse lugar.");
        return;
    };
    let scene = world
        .get_resource::<viber::scene_loader::CurrentScene>()
        .and_then(|scene| scene.id.clone());
    let in_main = scene.as_deref().is_none_or(|id| id == "main");
    if in_main && mark_position(world, name).is_none() {
        toast(world, "Esse destino já não existe.");
        return;
    }
    let mut patch = std::collections::BTreeMap::new();
    patch.insert("pin".to_string(), json!(name));
    patch_game(world, patch);
    toast(world, format!("Destino: {}", mark.label));
}

fn go_mark(world: &mut World, name: &str) {
    let Some(position) = mark_position(world, name) else {
        toast(world, "Esse marco ainda não está no mundo.");
        return;
    };
    // Exclui a entidade do próprio marco do raio de aterragem (o poço/torre
    // não é o chão onde o herói deve ficar).
    let mut exclude = Vec::new();
    {
        let mut query = world.query::<(Entity, &Name)>();
        for (entity, entity_name) in query.iter(world) {
            if entity_name.as_str() == name {
                exclude.push(entity);
                break;
            }
        }
    }
    let requested = Vec3::new(position[0], position[1] + 1.0, position[2]);
    let landing = viber::player::teleport_landing(world, requested, &exclude);
    if let Some(entity) = world
        .query_filtered::<Entity, With<Player>>()
        .iter(world)
        .next()
    {
        if let Some(mut transform) = world.get_mut::<Transform>(entity) {
            transform.translation = landing;
        }
        if let Some(mut player) = world.get_mut::<Player>(entity) {
            player.vel_x = 0.0;
            player.vel_y = 0.0;
            player.vel_z = 0.0;
        }
        world.entity_mut(entity).insert(TeleportSettle::default());
    }
    world.resource_mut::<MapTravelState>().travel_open = false;
    toast(world, "A fogueira leva-te até lá.");
    sfx(world, SfxClip::Travel, Some(landing));
}

fn handle_map_action(world: &mut World, id: &str) {
    if id == "map-close" {
        world.resource_mut::<MapTravelState>().map_open = false;
    } else if id == "travel-close" {
        world.resource_mut::<MapTravelState>().travel_open = false;
    } else if let Some(name) = id.strip_prefix("pin-") {
        pin_mark(world, name);
    } else if let Some(name) = id.strip_prefix("go-") {
        go_mark(world, name);
    }
}

/// Input M/G/F/Esc, exclusivo para poder teleportar e escrever no KV.
pub fn map_input(world: &mut World) {
    let keys = world
        .get_resource::<ButtonInput<KeyCode>>()
        .map(|keys| {
            (
                keys.just_pressed(KeyCode::KeyM),
                keys.just_pressed(KeyCode::KeyG),
                keys.just_pressed(KeyCode::KeyF),
                keys.just_pressed(KeyCode::Escape),
            )
        })
        .unwrap_or((false, false, false, false));
    let typing = world
        .get_resource::<UiFocusedInput>()
        .is_some_and(|focus| focus.0.is_some());
    let modals = world.get_resource::<UiModalsOpen>();
    let overlay = modals.is_some_and(|modals| {
        modals.is_open("menu")
            || modals.is_open("title")
            || modals.is_open("death")
            || modals.is_open("profiler-win")
    });
    let menu_blocked = world
        .get_resource::<MenusOpen>()
        .is_some_and(|menus| menus.modal || menus.shop || menus.cinematic);
    let save = world.resource::<VaelSave>();
    let downed = save.vitals.hp <= 0.0
        || save
            .legacy
            .get("death_pending")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    let blocked = typing || overlay || menu_blocked || downed;

    let mut state = *world.resource::<MapTravelState>();
    if overlay || menu_blocked || downed {
        state.map_open = false;
        state.travel_open = false;
    }
    if keys.3 {
        state.map_open = false;
        state.travel_open = false;
    }
    if keys.0 && !blocked {
        if state.map_open {
            state.map_open = false;
        } else if !state.travel_open {
            state.map_open = true;
        }
    }
    if keys.1 && !blocked {
        if state.travel_open {
            state.travel_open = false;
        } else if !state.map_open {
            let player = player_position(world);
            let range = world
                .get_resource::<VaelTravelRules>()
                .map(|rules| rules.0.campfire_range as f32)
                .unwrap_or(14.0);
            if player.is_some_and(|player| near_fire(world, player, range)) {
                state.travel_open = true;
            }
        }
    }
    if keys.2 && !blocked && !state.map_open && !state.travel_open {
        sign_nearest(world);
    }
    // Fecha a viagem se o herói sair da fogueira ou morrer.
    if state.travel_open {
        let player = player_position(world);
        let range = world
            .get_resource::<VaelTravelRules>()
            .map(|rules| rules.0.campfire_range as f32)
            .unwrap_or(14.0);
        if downed || player.is_none_or(|player| !near_fire(world, player, range)) {
            state.travel_open = false;
        }
    }
    let map_open = state.map_open;
    let travel_open = state.travel_open;
    *world.resource_mut::<MapTravelState>() = state;
    if let Some(mut menus) = world.get_resource_mut::<MenusOpen>() {
        menus.map = map_open;
        menus.travel = travel_open;
    }
}

fn project(position: [f32; 3], world_size: f32) -> (f64, f64) {
    let half = world_size.max(1.0) * 0.5;
    let left = ((position[0] + half) / world_size * CANVAS - MARKER_R).clamp(0.0, CANVAS - 12.0);
    let top = ((position[2] + half) / world_size * CANVAS - MARKER_R).clamp(0.0, CANVAS - 12.0);
    (left as f64, top as f64)
}

#[allow(clippy::too_many_arguments)]
pub fn sync_map(
    state: Res<MapTravelState>,
    save: Res<VaelSave>,
    content: Option<Res<VaelContent>>,
    rules: Option<Res<VaelTravelRules>>,
    travel: Option<Res<TravelState>>,
    markers: Option<Res<HudMarkers>>,
    terrain: Option<Res<TerrainRuntime>>,
    player: Query<&GlobalTransform, With<Player>>,
    vm: Res<VaelMapVm>,
) {
    let content = content.as_deref();
    let lang = save
        .legacy
        .get("lang")
        .and_then(Value::as_str)
        .unwrap_or("pt");
    let positions = travel
        .as_deref()
        .map(|state| state.get().positions)
        .unwrap_or_default();
    let player_pos = player
        .iter()
        .next()
        .map(|transform| {
            let p = transform.translation();
            [p.x, p.y, p.z]
        })
        .or_else(|| travel.as_deref().and_then(|state| state.get().player));
    let world_size = terrain
        .as_deref()
        .map(|terrain| terrain.spec.world_size)
        .filter(|size| *size > 0.0)
        .unwrap_or(4000.0);

    let mut map_rows = Vec::new();
    let mut travel_rows = Vec::new();
    if let Some(rules) = rules.as_deref() {
        for mark in &rules.0.landmarks {
            let Some(position) = positions.get(&mark.name) else {
                continue;
            };
            let revealed = is_revealed(&save, &mark.biome);
            if !available(mark, revealed) {
                continue;
            }
            let surveyed = is_surveyed(&save, &mark.name);
            let biome = rules
                .0
                .biome_label(&mark.biome)
                .unwrap_or(mark.biome.as_str());
            let distance = player_pos
                .map(|player| distance_xz(*position, player).floor() as i64)
                .unwrap_or(0);
            map_rows.push(RowVm {
                action: format!("pin-{}", mark.name),
                title: mark.label.clone(),
                subtitle: format!("{biome} · {distance} m"),
                tag: if surveyed {
                    "assinado".to_string()
                } else {
                    "por assinar".to_string()
                },
                progress: 0.0,
                has_progress: false,
                clickable: true,
                icon: String::new(),
                has_icon: false,
            });
            if surveyed {
                travel_rows.push(RowVm {
                    action: format!("go-{}", mark.name),
                    title: mark.label.clone(),
                    subtitle: String::new(),
                    tag: format!("{distance} m"),
                    progress: 0.0,
                    has_progress: false,
                    clickable: true,
                    icon: String::new(),
                    has_icon: false,
                });
            }
        }
    }

    let (player_left, player_top) = player_pos
        .map(|player| project(player, world_size))
        .unwrap_or((0.0, 0.0));
    let waypoint = markers
        .as_deref()
        .and_then(|markers| markers.waypoint.as_ref())
        .map(|point| {
            (
                project(point.position.to_array(), world_size),
                point.label.clone(),
            )
        });

    vm.0.update(|m: &mut MapVm| {
        m.map_open = state.map_open;
        m.travel_open = state.travel_open;
        m.map_title = label(content, lang, "map.title", "MAPA");
        m.map_sub = label(content, lang, "map.sub", "O Vale de Vael");
        m.map_hint = label(
            content,
            lang,
            "map.hint",
            "Clica num marco para fixar o destino.",
        );
        m.map_empty_text = label(content, lang, "map.empty", "Nenhum marco cartografado.");
        m.travel_title = label(content, lang, "map.travel.title", "VIAGEM");
        m.travel_sub = label(content, lang, "map.travel.sub", "Fogueiras assinadas");
        m.travel_empty_text = label(
            content,
            lang,
            "map.travel.empty",
            "Assina um marco com [F] para viajar até lá.",
        );
        m.map_rows_empty = map_rows.is_empty();
        m.travel_rows_empty = travel_rows.is_empty();
        m.map_rows = map_rows.clone();
        m.travel_rows = travel_rows.clone();
        m.player_visible = player_pos.is_some();
        m.player_margin = format!("{player_left:.1},{player_top:.1},0,0");
        if let Some(((left, top), label)) = waypoint {
            m.waypoint_margin = format!("{left:.1},{top:.1},0,0");
            m.waypoint_label = format!("Destino: {label}");
            m.waypoint_visible = true;
        } else {
            m.waypoint_label = String::new();
            m.waypoint_visible = false;
        }
    });
}

pub fn spawn_map(mut commands: Commands, vm: Res<VaelMapVm>) {
    let root = commands.spawn_xaml_bound(include_xaml!("ui/map.xaml"), vm.0.clone());
    commands.entity(root).insert(GlobalZIndex(Z_MAP));
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::AssetPlugin;

    #[test]
    fn map_xaml_instancia_sem_warnings() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default(), PfUiPlugin));
        app.init_asset::<Image>();
        app.init_asset::<bevy::text::Font>();
        let mut model = MapVm::default();
        model.player_margin = "224,224,0,0".into();
        model.waypoint_margin = "300,300,0,0".into();
        model.player_visible = true;
        model.waypoint_visible = true;
        let vm = Bindable::new(model);
        let root = app.world_mut().spawn(DataContext(vm.clone())).id();
        let doc = include_xaml!("ui/map.xaml").document();
        let result = bevy_pf::instantiate_document(app.world_mut(), root, &doc)
            .expect("o mapa XAML tem de instanciar");
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        app.world_mut().entity_mut(root).insert(DataContext(vm));
        for _ in 0..3 {
            app.update();
        }
        let names = app.world().get::<XamlNames>(root).expect("namescope");
        let marker = names.get("map-player").expect("marcador do jogador");
        let node = app.world().get::<Node>(marker).expect("Node do marcador");
        assert_eq!(node.margin.left, Val::Px(224.0));
        assert_eq!(node.margin.top, Val::Px(224.0));
    }

    #[test]
    fn projecao_centra_o_mundo_e_limita_as_bordas() {
        let (left, top) = project([0.0, 0.0, 0.0], 4000.0);
        assert!((left - (CANVAS / 2.0 - MARKER_R) as f64).abs() < 0.01);
        assert!((top - (CANVAS / 2.0 - MARKER_R) as f64).abs() < 0.01);
        let (left, top) = project([-4000.0, 0.0, 4000.0], 4000.0);
        assert_eq!(left, 0.0);
        assert_eq!(top, (CANVAS - 12.0) as f64);
    }
}

//! Waypoint/marcadores do HUD em Rust.
//!
//! Substitui o `scripts/game/travel.lua` como publicador de `HudMarkers`:
//! blips/beacons dos marcos disponíveis, o pin (destino do mapa) e a seta da
//! quest seguida (pronta → dador; ativa → objetivo; sem quest → dador novo
//! mais próximo). O mapa/viagem e o input já são de `crate::ui::map`.

use bevy::prelude::*;
use serde_json::Value;
use bevy_ext_core::{HudBlip, HudMarkers, HudPoint};
use viber::luau::host::LuaScriptHost;
use bevy_ext_core::Player;
use viber::scene_loader::CurrentScene;
use viber::vitals::{Health, Hostile};

use crate::attach::VaelScriptTags;
use crate::content::VaelContent;
use crate::harvest::Harvestable;
use crate::quest::{quest_available, quest_objective, quest_state};
use crate::quest_events::kind_matches;
use crate::save::VaelSave;
use crate::travel::{TravelState, VaelTravelRules, available, is_revealed, is_surveyed};
use crate::ui::screens::{VaelUiRules, quest_kv};

/// Período do publish (o `travel.lua` também era 2 Hz).
const PERIOD: f32 = 0.5;
/// Distância de chegada que limpa o pin (o mesmo raio da seta do HUD).
const PIN_ARRIVED_M: f32 = 4.0;
/// Beacons no HUD: os mais próximos, sem atalaias (as torres veem-se).
const BEACON_CAP: usize = 16;

pub struct VaelWaypointPlugin;

impl Plugin for VaelWaypointPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HudMarkers>()
            .add_systems(Update, publish_markers.after(viber::luau::luau_update));
    }
}

fn player_position(
    player: &Query<&GlobalTransform, With<Player>>,
    travel: Option<&TravelState>,
) -> Option<[f32; 3]> {
    player
        .iter()
        .next()
        .map(|transform| {
            let p = transform.translation();
            [p.x, p.y, p.z]
        })
        .or_else(|| travel.and_then(|state| state.get().player))
}

fn script_kind(path: &str) -> &str {
    path.strip_prefix("enemies/")
        .and_then(|path| path.strip_suffix(".lua"))
        .unwrap_or(path)
}

fn distance3(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    (dx * dx + dy * dy + dz * dz).sqrt()
}

fn quest_waypoint(
    save: &VaelSave,
    content: Option<&VaelContent>,
    rules: Option<&VaelUiRules>,
    travel: &TravelState,
    player: [f32; 3],
    npcs: &std::collections::BTreeMap<String, Vec3>,
    hostiles: &Query<(&GlobalTransform, &VaelScriptTags, Option<&Health>), With<Hostile>>,
    harvestables: &Query<(&GlobalTransform, &Harvestable)>,
) -> Option<HudPoint> {
    let rules = rules?;
    let content = content?;
    let kv = quest_kv(save);
    let positions = travel.get().positions;

    let npc_point = |def: &crate::content::QuestDef, prefix: &str| -> Option<HudPoint> {
        let position = *npcs.get(&def.npc)?;
        Some(HudPoint {
            position,
            label: format!(
                "{prefix} · {}",
                if def.title.is_empty() {
                    &def.id
                } else {
                    &def.title
                }
            ),
        })
    };

    let objective_point = |id: &str| -> Option<HudPoint> {
        let def = content.quests.get(id)?;
        let view = quest_objective(&rules.quests, &kv, id);
        let title = if def.title.is_empty() {
            &def.id
        } else {
            &def.title
        };
        match view.objective_type.as_str() {
            "kill" => {
                let mut best: Option<(HudPoint, f32)> = None;
                for (transform, tags, health) in hostiles.iter() {
                    if health.is_some_and(|health| health.current <= 0.0) {
                        continue;
                    }
                    if !kind_matches(script_kind(&tags.path), &view.target) {
                        continue;
                    }
                    let position = transform.translation();
                    let distance = distance3(position.to_array(), player);
                    if best.as_ref().is_none_or(|(_, best_d)| distance < *best_d) {
                        best = Some((
                            HudPoint {
                                position,
                                label: format!("Caçar · {title}"),
                            },
                            distance,
                        ));
                    }
                }
                best.map(|(point, _)| point)
            }
            "visit" => {
                let mut best: Option<(HudPoint, f32)> = None;
                for token in view.target.split_whitespace() {
                    let marker = format!("q.{id}.v.{token}");
                    if !matches!(kv.get(&marker), Some(Value::Bool(true))) {
                        let Some(position) = positions.get(token).copied() else {
                            continue;
                        };
                        let label = content
                            .landmarks
                            .landmarks
                            .iter()
                            .find(|mark| mark.name == token)
                            .map(|mark| {
                                if mark.label.is_empty() {
                                    token.to_string()
                                } else {
                                    mark.label.clone()
                                }
                            })
                            .unwrap_or_else(|| token.to_string());
                        let point = HudPoint {
                            position: Vec3::from_array(position),
                            label: format!("Visitar · {label}"),
                        };
                        let distance = distance3(position, player);
                        if best.as_ref().is_none_or(|(_, best_d)| distance < *best_d) {
                            best = Some((point, distance));
                        }
                    }
                }
                best.map(|(point, _)| point)
            }
            "collect" => {
                if view.target == "nota" {
                    let mut best: Option<(HudPoint, f32)> = None;
                    for (transform, tags, health) in hostiles.iter() {
                        if health.is_some_and(|health| health.current <= 0.0) {
                            continue;
                        }
                        let kind = script_kind(&tags.path);
                        if !matches!(kind, "witch" | "sand-worm" | "bog-warden" | "boss") {
                            continue;
                        }
                        if matches!(
                            save.legacy.get(format!("nota_{kind}").as_str()),
                            Some(Value::Bool(true))
                        ) {
                            continue;
                        }
                        let position = transform.translation();
                        let distance = distance3(position.to_array(), player);
                        if best.as_ref().is_none_or(|(_, best_d)| distance < *best_d) {
                            best = Some((
                                HudPoint {
                                    position,
                                    label: format!("Guardião · {title}"),
                                },
                                distance,
                            ));
                        }
                    }
                    return best.map(|(point, _)| point);
                }
                let mut best: Option<(HudPoint, f32)> = None;
                for (transform, harvestable) in harvestables.iter() {
                    if harvestable.kind != view.target {
                        continue;
                    }
                    let name = harvestable.name.as_str();
                    if !name.is_empty()
                        && (save.harvested.contains(name)
                            || matches!(
                                save.legacy.get(format!("harvested.{name}").as_str()),
                                Some(Value::Bool(true))
                            )
                            || matches!(
                                save.legacy.get(format!("once:{name}").as_str()),
                                Some(Value::Bool(true))
                            ))
                    {
                        continue;
                    }
                    let position = transform.translation();
                    let distance = distance3(position.to_array(), player);
                    if best.as_ref().is_none_or(|(_, best_d)| distance < *best_d) {
                        best = Some((
                            HudPoint {
                                position,
                                label: format!("Recolher · {title}"),
                            },
                            distance,
                        ));
                    }
                }
                best.map(|(point, _)| point)
            }
            _ => None,
        }
    };

    let tracked = save.quests.tracked.as_deref();
    if let Some(id) = tracked {
        let state = quest_state(&rules.quests, &kv, id);
        let def = content.quests.get(id);
        if let Some(def) = def {
            if state == "ready" {
                if let Some(point) = npc_point(def, "Entregar") {
                    return Some(point);
                }
            } else if state == "active"
                && let Some(point) = objective_point(id).or_else(|| npc_point(def, "Missão"))
            {
                return Some(point);
            }
        }
    }

    let mut ready = None;
    let mut active = None;
    let mut new_best: Option<(HudPoint, f32)> = None;
    for (id, def) in &content.quests {
        let state = quest_state(&rules.quests, &kv, id);
        if state == "ready" {
            if ready.is_none() {
                ready = npc_point(def, "Entregar");
            }
        } else if state == "active" {
            if active.is_none() {
                active = objective_point(id).or_else(|| npc_point(def, "Missão"));
            }
        } else if quest_available(&rules.quests, &kv, id) {
            // Sem quest: o dador novo mais próximo. `quest_available` fica
            // para a onda das quests; aqui o fallback é o cartão do HUD.
            if let Some(point) = npc_point(def, "Missão") {
                let distance = distance3(point.position.to_array(), player);
                if new_best
                    .as_ref()
                    .is_none_or(|(_, best_d)| distance < *best_d)
                {
                    new_best = Some((point, distance));
                }
            }
        }
    }
    ready
        .or(active)
        .or_else(|| new_best.map(|(point, _)| point))
}

#[allow(clippy::too_many_arguments)]
fn publish_markers(
    time: Res<Time>,
    mut timer: Local<f32>,
    mut markers: ResMut<HudMarkers>,
    travel: Option<Res<TravelState>>,
    rules: Option<Res<VaelTravelRules>>,
    save: Res<VaelSave>,
    content: Option<Res<VaelContent>>,
    ui_rules: Option<Res<VaelUiRules>>,
    scene: Option<Res<CurrentScene>>,
    host: Option<Res<LuaScriptHost>>,
    player: Query<&GlobalTransform, With<Player>>,
    names: Query<(&Name, &GlobalTransform)>,
    hostiles: Query<(&GlobalTransform, &VaelScriptTags, Option<&Health>), With<Hostile>>,
    harvestables: Query<(&GlobalTransform, &Harvestable)>,
) {
    *timer -= time.delta_secs();
    if *timer > 0.0 {
        return;
    }
    *timer = PERIOD;
    let Some(travel) = travel else {
        return;
    };
    let Some(rules) = rules else {
        return;
    };
    let player_pos = player_position(&player, Some(&travel));
    let positions = travel.get().positions;
    let npcs: std::collections::BTreeMap<String, Vec3> = names
        .iter()
        .map(|(name, transform)| (name.as_str().to_string(), transform.translation()))
        .collect();

    let mut blips = Vec::new();
    let mut beacons: Vec<(Vec3, f32)> = Vec::new();
    for mark in &rules.0.landmarks {
        let Some(position) = positions.get(&mark.name) else {
            continue;
        };
        if !available(mark, is_revealed(&save, &mark.biome)) {
            continue;
        }
        let position = Vec3::from_array(*position);
        blips.push(HudBlip {
            position,
            tower: mark.is_tower(),
        });
        if !mark.is_tower() && !is_surveyed(&save, &mark.name) {
            let distance = player_pos
                .map(|player| distance3(position.to_array(), player))
                .unwrap_or(f32::MAX);
            beacons.push((position, distance));
        }
    }
    beacons.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    let beacons = beacons
        .into_iter()
        .take(BEACON_CAP)
        .map(|(position, _)| position)
        .collect();

    let mut waypoint = None;
    let mut pin_patch = None;
    if let Some(pin) = save.legacy.get("pin").and_then(Value::as_str) {
        if let Some(position) = positions.get(pin) {
            let position = Vec3::from_array(*position);
            if player_pos
                .is_some_and(|player| distance3(position.to_array(), player) <= PIN_ARRIVED_M)
            {
                pin_patch = Some(("pin".to_string(), Value::Null));
            } else {
                waypoint = Some(HudPoint {
                    position,
                    label: rules.0.label(pin),
                });
            }
        } else if scene
            .as_deref()
            .and_then(|scene| scene.id.as_deref())
            .is_none_or(|id| id == "main")
        {
            pin_patch = Some(("pin".to_string(), Value::Null));
        }
    }
    if let Some((key, value)) = pin_patch
        && let Some(host) = host.as_deref()
    {
        let mut patch = std::collections::BTreeMap::new();
        patch.insert(key, value);
        viber::luau::game::apply_patch(&host.lua, &patch);
    }
    if waypoint.is_none()
        && let Some(player_pos) = player_pos
        && let Some(point) = quest_waypoint(
            &save,
            content.as_deref(),
            ui_rules.as_deref(),
            &travel,
            player_pos,
            &npcs,
            &hostiles,
            &harvestables,
        )
    {
        waypoint = Some(point);
    }

    markers.blips = blips;
    markers.beacons = beacons;
    markers.waypoint = waypoint;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_do_script_tira_o_prefixo_e_extensao() {
        assert_eq!(script_kind("enemies/wolf.lua"), "wolf");
        assert_eq!(script_kind("enemies/bandit-archer.lua"), "bandit-archer");
        assert_eq!(script_kind("outra/coisa.lua"), "outra/coisa.lua");
    }

    #[test]
    fn distancia_3d_nao_e_so_xz() {
        assert!((distance3([0.0, 0.0, 0.0], [3.0, 4.0, 0.0]) - 5.0).abs() < 1e-6);
        assert!(distance3([0.0, 20.0, 0.0], [0.0, 0.0, 0.0]) > 4.0);
    }
}

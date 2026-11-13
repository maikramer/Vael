//! Save/load do Vael sobre `moonshine-save`.
//!
//! O `SavePlugin` da engine deixa de correr no Vael (`external_save: true`):
//! os slots são ficheiros RON por slot, com um sidecar JSON para o menu. O
//! payload é um único recurso refletido (`VaelSaveFile`); o estado que não é
//! refletível (`VaelSave.legacy`, o `world_kv` do Lua e os rebinds) viaja
//! como JSON dentro desse recurso. A posição, a cena, os volumes, o idioma,
//! os recortes de terreno e os recortes de ação mantêm a paridade com o
//! envelope antigo.
//!
//! O `viber.save_slot(0)`/`viber.save()` do Lua continua a escrever
//! `SaveRequest`; este módulo consome-o no Update seguinte ao `luau_update`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use bevy::prelude::*;
use moonshine_save::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use viber::input_map::{Action, ActionMap};
use viber::locale::Locale;
use viber::luau::game::{game_to_json, json_to_game};
use viber::luau::host::LuaScriptHost;
use viber::music::AudioMixerSettings;
use bevy_ext_save::{SaveDir, SaveSlotInfo, WorldSaveKey};
use viber::save::SaveRequest;
use viber::scene_loader::{Arrival, ChangeScene, CurrentScene, request_change};
use viber::terrain::runtime::TerrainRuntime;
use viber::ui::UiWriter;

use crate::save::{VAEL_SAVE_VERSION, VaelSave};

/// Payload refletido do save. Tudo o que não é `Reflect` (legacy do
/// `VaelSave`, KV do Lua, rebinds) fica em strings JSON.
#[derive(Resource, Reflect, Clone, Debug, Default)]
#[reflect(Resource)]
#[type_path = "vael::save"]
pub struct VaelSaveFile {
    pub version: u32,
    /// Identidade da gravação; o sidecar confirma que o load aplicou este
    /// ficheiro (e não um payload velho que ficou no recurso).
    pub save_id: String,
    pub saved_at: f64,
    pub playtime: f64,
    pub title: String,
    pub position: Option<Vec3>,
    /// master, music, sfx.
    pub volumes: Vec3,
    /// voice, ambience.
    pub volumes_extra: Vec2,
    pub scene: String,
    pub came_from_scene: Option<String>,
    pub came_from_portal: Option<String>,
    pub locale: String,
    pub terrain_patches: Vec<viber::terrain::delta::DeltaPatch>,
    pub action_keys_json: String,
    pub world_kv_json: String,
    pub save_json: String,
}

/// Metadados do menu (lidos sem deserializar o RON).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct SlotMeta {
    save_id: String,
    saved_at: f64,
    playtime: f64,
    title: String,
    level: u32,
    gold: u32,
    scene: String,
}

/// Estado transitório entre o pedido, o observer do `moonshine-save` e a
/// aplicação no frame seguinte.
#[derive(Resource, Default)]
pub struct NativeSaveState {
    pending_save: Option<(PathBuf, SlotMeta)>,
    pending_load: Option<PathBuf>,
    save_done: bool,
    load_done: bool,
    load_legacy: Option<Box<VaelSaveFile>>,
}

pub struct VaelNativeSavePlugin;

/// A cadeia reflectida que o payload gravável precisa — registada pelo
/// plugin e pelos testes que montam `App`'s à mão.
fn register_save_types(app: &mut App) {
    app.register_type::<VaelSaveFile>()
        .register_type::<Vec3>()
        .register_type::<Vec2>()
        .register_type::<viber::terrain::delta::DeltaPatch>();
}

impl Plugin for VaelNativeSavePlugin {
    fn build(&self, app: &mut App) {
        register_save_types(app);
        app.init_resource::<VaelSaveFile>()
            .init_resource::<NativeSaveState>()
            .add_observer(save_on_default_event)
            .add_observer(load_on_default_event)
            .add_observer(on_native_saved)
            .add_observer(on_native_loaded)
            .add_systems(
                Update,
                process_save_requests.after(viber::luau::luau_update),
            )
            .add_systems(PostUpdate, (finish_native_save, finish_native_load));
    }
}

fn on_native_saved(_event: On<Saved>, mut state: ResMut<NativeSaveState>) {
    state.save_done = true;
}

fn on_native_loaded(_event: On<Loaded>, mut state: ResMut<NativeSaveState>) {
    state.load_done = true;
}

/// Consome `SaveRequest` (UI, menu e `viber.save*` do Lua) e dispara o
/// moonshine. O ficheiro é escrito/deserializado nos observers; o frame de
/// `PostUpdate` aplica o resultado.
fn process_save_requests(world: &mut World) {
    // Um load/save pedido a meio de uma troca de cena aplicaria o payload na
    // cena errada (o `request_change` de um load também é recusado enquanto
    // o loader está ocupado). O pedido fica em `SaveRequest` e volta a ser
    // tentado no frame seguinte.
    if world
        .get_resource::<viber::scene_loader::SceneLoader>()
        .is_some_and(|loader| loader.busy())
    {
        return;
    }
    let Some(mut request) = world.get_resource_mut::<SaveRequest>() else {
        return;
    };
    let (save, load, slot) = (request.save, request.load, request.slot);
    if !save && !load {
        return;
    }
    request.save = false;
    request.load = false;

    if save {
        let Some(payload) = capture(world) else {
            warn!("vael save: sem estado para gravar");
            return;
        };
        let path = native_path(world, slot);
        if let Some(parent) = path.parent()
            && let Err(err) = std::fs::create_dir_all(parent)
        {
            error!("vael save: não criei {}: {err}", parent.display());
            return;
        }
        let meta = meta_of(&payload);
        world.insert_resource(payload);
        if let Some(mut state) = world.get_resource_mut::<NativeSaveState>() {
            state.pending_save = Some((path.clone(), meta));
            state.save_done = false;
        }
        world.trigger_save(
            SaveWorld::default_into_file(path.clone()).include_resource::<VaelSaveFile>(),
        );
        info!(path = %path.display(), "vael save: gravado");
    } else {
        let path = native_path(world, slot);
        if path.is_file() {
            if let Some(mut state) = world.get_resource_mut::<NativeSaveState>() {
                state.pending_load = Some(path.clone());
                state.load_done = false;
            }
            world.trigger_load(LoadWorld::default_from_file(path.clone()));
            info!(path = %path.display(), "vael save: carregado");
        } else if let Some(payload) = legacy_payload(world, slot) {
            // Save anterior ao moonshine: importa o JSON e aplica já.
            if let Some(mut state) = world.get_resource_mut::<NativeSaveState>() {
                state.load_legacy = Some(Box::new(payload));
            }
        } else {
            warn!("vael save: slot {:?} sem ficheiro", slot);
        }
    }
}

fn finish_native_save(world: &mut World) {
    let Some((path, meta)) = world
        .get_resource_mut::<NativeSaveState>()
        .and_then(|mut state| {
            if !state.save_done {
                return None;
            }
            state.save_done = false;
            state.pending_save.take()
        })
    else {
        return;
    };
    if let Err(err) = write_meta(&path, &meta) {
        error!("vael save: sidecar {}: {err}", path.display());
    }
}

fn finish_native_load(world: &mut World) {
    let ready = world
        .get_resource::<NativeSaveState>()
        .is_some_and(|state| state.load_done || state.load_legacy.is_some());
    if !ready {
        return;
    }
    let (loaded, legacy) = {
        let Some(mut state) = world.get_resource_mut::<NativeSaveState>() else {
            return;
        };
        state.load_done = false;
        (state.pending_load.take(), state.load_legacy.take())
    };
    if let Some(payload) = legacy {
        apply_payload(world, &payload);
        return;
    }
    let Some(path) = loaded else {
        return;
    };
    // Confirma que o recurso é o ficheiro que pedimos (um load falhado deixa
    // o payload anterior e não pode ser aplicado por engano).
    let Some(meta) = read_meta(&path) else {
        warn!("vael save: sidecar em falta para {}", path.display());
        return;
    };
    let payload = world.resource::<VaelSaveFile>().clone();
    if payload.save_id != meta.save_id {
        warn!(
            "vael save: load de {} não aplicou o payload esperado",
            path.display()
        );
        return;
    }
    apply_payload(world, &payload);
}

fn capture(world: &mut World) -> Option<VaelSaveFile> {
    let save = world
        .get_resource::<VaelSave>()
        .cloned()
        .unwrap_or_default();
    let kv = world
        .get_resource::<LuaScriptHost>()
        .map(|host| game_to_json(&host.lua))
        .unwrap_or_else(|| save.to_world_kv());
    let world_kv_json = serde_json::to_string(&kv).unwrap_or_default();
    let save_json = serde_json::to_string(&save).unwrap_or_default();
    let position = world
        .query_filtered::<&GlobalTransform, With<bevy_ext_core::Player>>()
        .iter(world)
        .next()
        .map(|transform| transform.translation());
    let scene = world
        .get_resource::<CurrentScene>()
        .filter(|scene| scene.switchable())
        .cloned();
    let mixer = world
        .get_resource::<AudioMixerSettings>()
        .cloned()
        .unwrap_or_default();
    let locale = world
        .get_resource::<Locale>()
        .map(|locale| locale.lang.clone())
        .unwrap_or_else(|| "pt".to_string());
    let terrain_patches = world
        .get_resource::<TerrainRuntime>()
        .map(|terrain| terrain.deltas.export_patches())
        .unwrap_or_default();
    let action_keys = world
        .get_resource::<ActionMap>()
        .map(|map| {
            map.bindings
                .iter()
                .filter_map(|(action, binding)| {
                    binding
                        .key
                        .as_ref()
                        .map(|key| (action.as_str().to_string(), key.clone()))
                })
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0);
    Some(VaelSaveFile {
        version: VAEL_SAVE_VERSION,
        save_id: format!("{now:.6}"),
        saved_at: now,
        playtime: kv
            .get("playtime_secs")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        title: kv
            .get("save_title")
            .and_then(Value::as_str)
            .unwrap_or("Vael")
            .to_string(),
        position,
        volumes: Vec3::new(mixer.master, mixer.music, mixer.sfx),
        volumes_extra: Vec2::new(mixer.voice, mixer.ambience),
        scene: scene
            .as_ref()
            .and_then(|scene| scene.id.clone())
            .unwrap_or_else(|| "main".to_string()),
        came_from_scene: scene
            .as_ref()
            .and_then(|scene| scene.came_from.as_ref().map(|(scene, _)| scene.clone())),
        came_from_portal: scene
            .as_ref()
            .and_then(|scene| scene.came_from.as_ref().map(|(_, portal)| portal.clone())),
        locale,
        terrain_patches,
        action_keys_json: serde_json::to_string(&action_keys).unwrap_or_default(),
        world_kv_json,
        save_json,
    })
}

fn apply_payload(world: &mut World, payload: &VaelSaveFile) {
    if let Ok(save) = serde_json::from_str::<VaelSave>(&payload.save_json) {
        world.insert_resource(save);
    } else {
        warn!("vael save: save_json inválido");
    }
    if let Some(host) = world.get_resource::<LuaScriptHost>()
        && let Ok(kv) = serde_json::from_str::<BTreeMap<String, Value>>(&payload.world_kv_json)
    {
        json_to_game(&host.lua, &kv);
    }
    if let Some(mut mixer) = world.get_resource_mut::<AudioMixerSettings>() {
        mixer.master = payload.volumes.x.clamp(0.0, 1.0);
        mixer.music = payload.volumes.y.clamp(0.0, 1.0);
        mixer.sfx = payload.volumes.z.clamp(0.0, 1.0);
        mixer.voice = payload.volumes_extra.x.clamp(0.0, 1.0);
        mixer.ambience = payload.volumes_extra.y.clamp(0.0, 1.0);
    }
    if let Some(mut locale) = world.get_resource_mut::<Locale>() {
        locale.set_lang(&payload.locale);
    }
    if let Some(mut map) = world.get_resource_mut::<ActionMap>()
        && let Ok(keys) =
            serde_json::from_str::<BTreeMap<String, String>>(&payload.action_keys_json)
    {
        for (name, key) in keys {
            if let Some(action) = Action::parse(&name) {
                map.rebind_key(action, &key);
            }
        }
    }
    world.insert_resource(viber::save::PendingSavePatches(
        payload.terrain_patches.clone(),
    ));
    let current = world.get_resource::<CurrentScene>().cloned();
    let scene_matches = current
        .as_ref()
        .and_then(|scene| scene.id.as_deref())
        .is_some_and(|id| id == payload.scene);
    if scene_matches {
        if let Some(position) = payload.position {
            crate::respawn::teleport_player(world, position);
        }
    } else {
        let mut change = ChangeScene::new(
            payload.scene.clone(),
            payload
                .position
                .map(Arrival::Position)
                .unwrap_or(Arrival::Entry(None)),
        );
        change.came_from = payload
            .came_from_scene
            .clone()
            .zip(payload.came_from_portal.clone());
        if let Err(err) = request_change(world, change) {
            warn!("vael save: troca de cena no load falhou: {err}");
            if let Some(position) = payload.position {
                crate::respawn::teleport_player(world, position);
            }
        }
    }
    if let Some(writer) = world.get_resource::<UiWriter>() {
        writer.open("menu", false);
        writer.open("death", false);
        writer.set_visible("hud", true);
    }
}

// ── paths e slots ───────────────────────────────────────────────────────

fn default_save_dir() -> PathBuf {
    bevy_ext_save::save_path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn save_dir(world: &World) -> PathBuf {
    world
        .get_resource::<SaveDir>()
        .and_then(|dir| dir.0.clone())
        .unwrap_or_else(default_save_dir)
}

fn world_key(world: &World) -> String {
    world
        .get_resource::<WorldSaveKey>()
        .and_then(|key| key.0.clone())
        .unwrap_or_else(|| "vael".to_string())
}

fn native_path(world: &World, slot: Option<u32>) -> PathBuf {
    let suffix = match slot {
        None => "legacy".to_string(),
        Some(0) => "auto".to_string(),
        Some(slot) => format!("slot{slot}"),
    };
    save_dir(world).join(format!("{}.{}.vsave.ron", world_key(world), suffix))
}

fn meta_path(path: &Path) -> PathBuf {
    path.with_extension("meta.json")
}

fn meta_of(payload: &VaelSaveFile) -> SlotMeta {
    let save = serde_json::from_str::<VaelSave>(&payload.save_json).unwrap_or_default();
    SlotMeta {
        save_id: payload.save_id.clone(),
        saved_at: payload.saved_at,
        playtime: payload.playtime,
        title: payload.title.clone(),
        level: save.progression.level.max(0) as u32,
        gold: save.inventory.gold.max(0) as u32,
        scene: payload.scene.clone(),
    }
}

fn write_meta(path: &Path, meta: &SlotMeta) -> Result<(), String> {
    use std::io::Write as _;
    let text = serde_json::to_string_pretty(meta).map_err(|err| err.to_string())?;
    let tmp = meta_path(path).with_extension("meta.json.tmp");
    // tmp+fsync+rename (o padrão atómico do `bevy_ext_save`): um kill -9
    // entre a escrita e o rename não deixa a meta do slot meia-escrita.
    let mut file = std::fs::File::create(&tmp).map_err(|err| err.to_string())?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|err| err.to_string())?;
    drop(file);
    std::fs::rename(&tmp, meta_path(path)).map_err(|err| err.to_string())
}

fn read_meta(path: &Path) -> Option<SlotMeta> {
    let text = std::fs::read_to_string(meta_path(path)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Lista os quatro slots nativos; se um slot ainda só tiver o JSON antigo,
/// devolve o metadata antigo para o menu continuar a mostrá-lo.
pub fn list_slots(world_key: Option<&str>, save_dir: Option<&Path>) -> Vec<SaveSlotInfo> {
    let old = bevy_ext_save::list_save_slots(world_key, save_dir);
    let base = save_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(default_save_dir);
    let key = world_key.unwrap_or("vael");
    let mut rows = Vec::with_capacity(5);
    for slot in 0..=3u32 {
        let path = base.join(format!("{key}.{}.vsave.ron", slot_suffix(slot)));
        let meta = read_meta(&path);
        if let Some(meta) = meta {
            let file = std::fs::metadata(&path).ok();
            rows.push(SaveSlotInfo {
                slot,
                path: path.display().to_string(),
                exists: file.is_some(),
                bytes: file.as_ref().map_or(0, |meta| meta.len()),
                saved_at: Some(meta.saved_at),
                level: meta.level,
                gold: meta.gold,
                scene: Some(meta.scene),
                playtime: Some(meta.playtime),
                title: Some(meta.title),
            });
        } else if let Some(old) = old.iter().find(|row| row.slot == slot) {
            rows.push(old.clone());
        } else {
            rows.push(SaveSlotInfo {
                slot,
                path: path.display().to_string(),
                exists: false,
                bytes: 0,
                saved_at: None,
                level: 0,
                gold: 0,
                scene: None,
                playtime: None,
                title: None,
            });
        }
    }
    if let Some(legacy) = old.iter().find(|row| row.slot == 4) {
        rows.push(legacy.clone());
    }
    rows
}

fn slot_suffix(slot: u32) -> String {
    if slot == 0 {
        "auto".to_string()
    } else {
        format!("slot{slot}")
    }
}

/// Importa o envelope JSON antigo para o payload nativo.
fn legacy_payload(world: &mut World, slot: Option<u32>) -> Option<VaelSaveFile> {
    let key = world
        .get_resource::<WorldSaveKey>()
        .and_then(|key| key.0.clone());
    let dir = world
        .get_resource::<SaveDir>()
        .and_then(|dir| dir.0.clone());
    let old_path = bevy_ext_save::save_path_for_slot(key.as_deref(), dir.as_deref(), slot);
    let game = bevy_ext_save::load_from_disk(&old_path).ok()?;
    let save = game
        .game
        .as_ref()
        .and_then(|game| game.get("vael"))
        .and_then(|value| serde_json::from_value::<VaelSave>(value.clone()).ok())
        .unwrap_or_else(|| VaelSave::from_world_kv(&game.world_kv));
    let world_kv_json = serde_json::to_string(&game.world_kv).unwrap_or_default();
    let action_keys_json = serde_json::to_string(&game.action_keys).unwrap_or_default();
    Some(VaelSaveFile {
        version: game.version.max(VAEL_SAVE_VERSION),
        save_id: format!("legacy-{:?}", slot),
        saved_at: game.saved_at.unwrap_or(0.0),
        playtime: game.playtime.unwrap_or(0.0),
        title: game.title.clone().unwrap_or_else(|| "Vael".to_string()),
        position: game.position.map(Vec3::from_array),
        volumes: game
            .volumes
            .map(|(master, music, sfx)| Vec3::new(master, music, sfx))
            .unwrap_or(Vec3::ONE),
        volumes_extra: game
            .volumes_extra
            .map(|(voice, ambience)| Vec2::new(voice, ambience))
            .unwrap_or(Vec2::ONE),
        scene: game.scene.clone().unwrap_or_else(|| "main".to_string()),
        came_from_scene: game.came_from.as_ref().map(|(scene, _)| scene.clone()),
        came_from_portal: game.came_from.as_ref().map(|(_, portal)| portal.clone()),
        locale: game.locale.clone().unwrap_or_else(|| "pt".to_string()),
        terrain_patches: game.terrain_patches.clone(),
        action_keys_json,
        world_kv_json,
        save_json: serde_json::to_string(&save).unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::AssetPlugin;

    fn payload() -> VaelSaveFile {
        VaelSaveFile {
            version: VAEL_SAVE_VERSION,
            save_id: "test-id".into(),
            saved_at: 42.0,
            playtime: 7.0,
            title: "Vael teste".into(),
            position: Some(Vec3::new(1.0, 2.0, 3.0)),
            volumes: Vec3::new(1.0, 0.8, 0.7),
            volumes_extra: Vec2::new(0.9, 0.6),
            scene: "main".into(),
            came_from_scene: Some("interiors/chapel".into()),
            came_from_portal: Some("exit".into()),
            locale: "pt".into(),
            terrain_patches: Vec::new(),
            action_keys_json: "{\"jump\":\"space\"}".into(),
            world_kv_json: "{}".into(),
            save_json: serde_json::to_string(&VaelSave::default()).unwrap(),
        }
    }

    #[test]
    fn moonshine_grava_e_carrega_o_payload() {
        let dir = std::env::temp_dir().join(format!("vael-save-native-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("slot.vsave.ron");
        {
            let mut app = App::new();
            app.add_plugins((MinimalPlugins, AssetPlugin::default()));
            register_save_types(&mut app);
            app.add_observer(save_on_default_event);
            app.insert_resource(payload());
            app.world_mut().trigger_save(
                SaveWorld::default_into_file(path.clone()).include_resource::<VaelSaveFile>(),
            );
            app.update();
        }
        assert!(path.is_file(), "moonshine devia ter escrito o ficheiro");
        {
            let mut app = App::new();
            app.add_plugins((MinimalPlugins, AssetPlugin::default()));
            register_save_types(&mut app);
            app.init_resource::<VaelSaveFile>()
                .add_observer(load_on_default_event);
            app.world_mut()
                .trigger_load(LoadWorld::default_from_file(path.clone()));
            app.update();
            let loaded = app.world().resource::<VaelSaveFile>();
            assert_eq!(loaded.save_id, "test-id");
            assert_eq!(loaded.position, Some(Vec3::new(1.0, 2.0, 3.0)));
            assert_eq!(loaded.scene, "main");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sidecar_guarda_e_le_os_metadados() {
        let dir = std::env::temp_dir().join(format!("vael-save-meta-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("slot.vsave.ron");
        let meta = SlotMeta {
            save_id: "id".into(),
            saved_at: 1.0,
            playtime: 2.0,
            title: "t".into(),
            level: 3,
            gold: 4,
            scene: "main".into(),
        };
        write_meta(&path, &meta).expect("meta");
        let back = read_meta(&path).expect("meta de volta");
        assert_eq!(back.save_id, "id");
        assert_eq!(back.level, 3);
        assert_eq!(back.gold, 4);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

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
use std::sync::{Mutex, OnceLock};

use bevy::prelude::*;
use bevy_elements::{CivilDate, WorldClock};
use bevy_ext_save::{SaveDir, SaveSlotInfo, WorldSaveKey};
use moonshine_save::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use viber::input_map::{Action, ActionMap};
use viber::locale::Locale;
use viber::luau::game::{game_to_json_process, restore_game_json};
use viber::music::AudioMixerSettings;
use viber::save::SaveRequest;
use viber::scene_loader::{Arrival, ChangeScene, CurrentScene, request_change};
use viber::terrain::runtime::TerrainRuntime;
use viber::ui::UiWriter;

use crate::boat::BoatStates;
use crate::save::{SavedCalendar, VAEL_SAVE_VERSION, VaelSave};

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
///
/// `#[serde(default)]` e `level`/`gold` em `i64`: um sidecar de outra versão
/// (campo novo em falta) ou editado à mão com um negativo não pode falhar a
/// leitura — um `read_meta` a `None` escondia o slot do menu apesar do save
/// estar bom.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct SlotMeta {
    save_id: String,
    saved_at: f64,
    playtime: f64,
    title: String,
    level: i64,
    gold: i64,
    scene: String,
}

/// Estado transitório entre o pedido, o observer do `moonshine-save` e a
/// aplicação no frame seguinte.
#[derive(Resource, Default)]
pub struct NativeSaveState {
    /// `(temporário, destino, meta)`: o moonshine escreve no temporário (o
    /// `std::fs::write` dele trunca o destino) e o fim do frame renomeia.
    pending_save: Option<(PathBuf, PathBuf, SlotMeta)>,
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
                process_save_requests.after(viber::luau::bms_bridge::apply_bms_commands),
            )
            .add_systems(
                PostUpdate,
                (
                    finish_native_save,
                    finish_native_load,
                    report_failed_native_io,
                )
                    .chain(),
            );
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
    // O slot é do pedido, não um estado pegajoso: um `viber.save()` a seguir
    // a um save manual não pode cair no slot manual (o monólito limpa-o).
    request.slot = None;

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
        let tmp = tmp_path(&path);
        let meta = meta_of(&payload);
        world.insert_resource(payload);
        if let Some(mut state) = world.get_resource_mut::<NativeSaveState>() {
            state.pending_save = Some((tmp.clone(), path.clone(), meta));
            state.save_done = false;
        }
        world.trigger_save(SaveWorld::default_into_file(tmp).include_resource::<VaelSaveFile>());
        info!(path = %path.display(), "vael save: gravado");
    } else {
        let path = native_path(world, slot);
        if path.is_file() {
            // A identidade do payload é zerada ANTES do load: se o load falhar,
            // o recurso fica com o payload (e o id) anterior e um id vazio
            // denuncia-o — mesmo quando o sidecar se perdeu (kill entre a
            // gravação do RON e a da meta).
            if let Some(mut payload) = world.get_resource_mut::<VaelSaveFile>() {
                payload.save_id.clear();
            }
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
    let Some((tmp, path, meta)) =
        world
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
    // Só depois do rename é que o slot muda: um kill a meio da escrita deixa
    // o save ANTERIOR intacto (o moonshine escreve com `std::fs::write`, que
    // truncava o destino).
    if let Err(err) = commit_save(&tmp, &path) {
        error!("vael save: {} não substituído: {err}", path.display());
        return;
    }
    if let Err(err) = write_meta(&path, &meta) {
        error!("vael save: sidecar {}: {err}", path.display());
    }
}

/// `fsync` do temporário + rename atómico para o slot.
fn commit_save(tmp: &Path, path: &Path) -> Result<(), String> {
    std::fs::File::open(tmp)
        .and_then(|file| file.sync_all())
        .map_err(|err| format!("{}: {err}", tmp.display()))?;
    std::fs::rename(tmp, path).map_err(|err| format!("{}: {err}", path.display()))
}

/// Um save/load que o moonshine recusou (RON de outro jogo, ficheiro
/// corrompido, disco cheio) NÃO dispara `Saved`/`Loaded`: sem isto o pedido
/// ficava pendente para sempre e a operação falhava em silêncio (só a linha
/// `error!` do crate no log) — o jogador via "a carregar" e nada acontecia.
fn report_failed_native_io(world: &mut World) {
    let Some(mut state) = world.get_resource_mut::<NativeSaveState>() else {
        return;
    };
    if !state.save_done
        && let Some((tmp, path, _)) = state.pending_save.take()
    {
        warn!(
            "vael save: {} não gravou (o slot anterior ficou intacto)",
            path.display()
        );
        // O temporário meio escrito não serve para nada.
        let _ = std::fs::remove_file(&tmp);
    }
    if !state.load_done
        && state.load_legacy.is_none()
        && let Some(path) = state.pending_load.take()
    {
        warn!(
            "vael save: {} não carregou (ficheiro inválido ou de outro jogo)",
            path.display()
        );
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
    // O `save_id` é zerado antes do load, por isso um payload com id é prova
    // de que o ficheiro pedido É o que está no recurso (um load falhado
    // deixava-o vazio). O sidecar é só o cache do menu: se faltar ou estiver
    // atrasado (kill entre o rename do RON e a meta), o ficheiro manda.
    let payload = world.resource::<VaelSaveFile>().clone();
    if payload.save_id.is_empty() {
        warn!(
            "vael save: load de {} não aplicou o payload esperado",
            path.display()
        );
        return;
    }
    match read_meta(&path) {
        Some(meta) if payload.save_id != meta.save_id => warn!(
            "vael save: sidecar de {} desactualizado — a aplicar o ficheiro",
            path.display()
        ),
        Some(_) => {}
        None => warn!(
            "vael save: sidecar em falta para {} — a aplicar o payload do ficheiro",
            path.display()
        ),
    }
    apply_payload(world, &payload);
}

fn capture(world: &mut World) -> Option<VaelSaveFile> {
    let mut save = world
        .get_resource::<VaelSave>()
        .cloned()
        .unwrap_or_default();
    // Store do processo primeiro (o `viber.game()` dos contextos BMS); sem
    // chaves (arranque), o save tipado.
    let (kv, live_store) = {
        let kv = game_to_json_process();
        if kv.is_empty() {
            (save.to_world_kv(), false)
        } else {
            (kv, true)
        }
    };
    // O store é o espelho VIVO e o tipado só é sincronizado a 10 Hz: fundir
    // aqui deixa o `game.vael` e o `world_kv` do MESMO ficheiro a contar a
    // mesma história (hp/gear/quests), sem tocar no que é só-Rust.
    if live_store {
        save.merge_world_kv(&kv);
    }
    // Barcos: o [`BoatStates`] sobrevive aos `SceneLoad` (os barcos são
    // despawnados ao sair do `main`) e é atualizado a cada frame com a física
    // viva; guardar daqui é o que faz um save a bordo acordar o barco no sítio
    // onde ficou (e não no autorado).
    if let Some(bus) = world.get_resource::<crate::player::HeroMotionBus>() {
        save.stamina = bus.lock().stamina.clone();
    }
    if let Some(states) = world.get_resource::<BoatStates>() {
        save.boats = states.poses.clone();
        save.riding_boat = states.riding.clone();
    }
    if let Some(clock) = world.get_resource::<WorldClock>() {
        save.calendar = Some(SavedCalendar {
            year: clock.date.year,
            month: clock.date.month,
            day: clock.date.day,
            hour: clock.hour,
        });
    }
    let world_kv_json = serde_json::to_string(&kv).unwrap_or_default();
    let save_json = serde_json::to_string(&save).unwrap_or_default();
    // `Transform` e não `GlobalTransform`: o `Update` corre antes da
    // propagação da hierarquia (PostUpdate), por isso a global é sempre um
    // frame atrás do que o herói está a ver — a pose gravada não pode ficar
    // 1 frame para trás do sítio onde o jogador gravou.
    let position = world
        .query_filtered::<&Transform, With<bevy_ext_core::Player>>()
        .iter(world)
        .next()
        .map(|transform| transform.translation)
        .filter(|position| position.is_finite());
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
    let kv = serde_json::from_str::<BTreeMap<String, Value>>(&payload.world_kv_json).ok();
    // Um payload editado à mão (ou vindo de outra versão) pode trazer NaN/inf:
    // um `position` não finito punha a física do herói em NaN e um volume NaN
    // silenciava o áudio para sempre.
    let position = payload.position.filter(|position| position.is_finite());
    let mut boats = BoatStates::default();
    let mut stamina = crate::stamina::Stamina::default();
    match serde_json::from_str::<VaelSave>(&payload.save_json) {
        Ok(save) => {
            if let Some(at) = save.calendar
                && let Some(mut clock) = world.get_resource_mut::<WorldClock>()
                && (0.0..24.0).contains(&at.hour)
            {
                clock.date = CivilDate::new(at.year, at.month, at.day);
                clock.hour = at.hour;
            }
            stamina = save.stamina.clone();
            boats = BoatStates::from_save(save.boats.clone(), save.riding_boat.clone());
            world.insert_resource(save);
        }
        Err(err) => {
            // Sem o tipado (ficheiro de outra versão/truncado), migrar o
            // `world_kv` é melhor do que deixar o `VaelSave` da sessão
            // anterior a mandar no HUD/save seguinte.
            warn!("vael save: save_json inválido ({err}); a migrar world_kv");
            if let Some(kv) = &kv {
                world.insert_resource(VaelSave::from_world_kv(kv));
            }
        }
    }
    // As poses dos barcos vêm do save: o `drive_boat_states` aplica-as aos
    // barcos vivos (e o `spawn_boats` usa-as se o load trocar de cena).
    world.insert_resource(boats);
    if let Some(bus) = world.get_resource::<crate::player::HeroMotionBus>() {
        bus.lock().stamina = stamina;
    }
    // O embarque da sessão anterior não sobrevive ao load: sem isto o
    // `seat_riders` prendia o herói ao barco velho depois do teleporte. O
    // `BoatStates` do save volta a pô-lo a bordo se for esse o caso.
    crate::boat::leave_boat_now(world);
    // `restore_game_json` repõe o store do processo (o `viber.game()` de
    // todos os contextos BMS lê daqui — já não depende de haver host).
    if let Some(kv) = kv {
        restore_game_json(&kv);
    }
    if let Some(mut mixer) = world.get_resource_mut::<AudioMixerSettings>() {
        // Um volume não finito mantém o da sessão em vez de o estragar.
        let set = |value: f32| value.is_finite().then(|| value.clamp(0.0, 1.0));
        if let Some(value) = set(payload.volumes.x) {
            mixer.master = value;
        }
        if let Some(value) = set(payload.volumes.y) {
            mixer.music = value;
        }
        if let Some(value) = set(payload.volumes.z) {
            mixer.sfx = value;
        }
        if let Some(value) = set(payload.volumes_extra.x) {
            mixer.voice = value;
        }
        if let Some(value) = set(payload.volumes_extra.y) {
            mixer.ambience = value;
        }
    }
    // O idioma só troca se o ficheiro existir: um save com um `lang`
    // desconhecido deixava a tabela vazia e o HUD mostrava as chaves `@…`
    // cruas até o jogador escolher outro idioma.
    if let Some(mut locale) = world.get_resource_mut::<Locale>()
        && locale
            .dir
            .join(format!("{}.json", payload.locale))
            .is_file()
    {
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
        crate::harvest::reload_trees(world);
        crate::rocks::reload(world);
        if let Some(position) = position {
            crate::respawn::teleport_player(world, position);
        }
    } else {
        let arrival = position
            .map(Arrival::Position)
            .unwrap_or(Arrival::Entry(None));
        let mut change = ChangeScene::new(payload.scene.clone(), arrival.clone());
        change.came_from = payload
            .came_from_scene
            .clone()
            .zip(payload.came_from_portal.clone());
        if let Err(err) = request_change(world, change) {
            warn!("vael save: troca de cena no load falhou: {err}");
            // A cena guardada pode ter saído do projeto: cair no mundo
            // principal com a pose guardada é melhor do que ficar na cena
            // atual (o título) com o estado carregado. O `came_from` do save
            // pertence à cena que já não existe — não se repõe.
            let main = viber::project::MAIN_SCENE;
            let mut recovered = false;
            if payload.scene != main {
                let mut fallback = ChangeScene::new(main, arrival);
                fallback.came_from = None;
                recovered = request_change(world, fallback).is_ok();
            }
            if !recovered && let Some(position) = position {
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

/// Temporário da gravação (o `finish_native_save` renomeia-o para o slot).
fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_default();
    name.push(".tmp");
    path.with_file_name(name)
}

fn meta_of(payload: &VaelSaveFile) -> SlotMeta {
    let save = serde_json::from_str::<VaelSave>(&payload.save_json).unwrap_or_default();
    SlotMeta {
        save_id: payload.save_id.clone(),
        saved_at: payload.saved_at,
        playtime: payload.playtime,
        title: payload.title.clone(),
        level: save.progression.level,
        gold: save.inventory.gold,
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

/// `i64` do sidecar → `u32` do menu, sem negativos nem overflow.
fn u32_level(value: i64) -> u32 {
    value.clamp(0, u32::MAX as i64) as u32
}

fn read_meta(path: &Path) -> Option<SlotMeta> {
    let text = std::fs::read_to_string(meta_path(path)).ok()?;
    let mut meta: SlotMeta = serde_json::from_str(&text).ok()?;
    // Um sidecar editado à mão com NaN/inf rebentava a ordenação do menu
    // (`latest_slot` compara `saved_at`) e o texto do `playtime`.
    if !meta.saved_at.is_finite() {
        meta.saved_at = 0.0;
    }
    if !meta.playtime.is_finite() {
        meta.playtime = 0.0;
    }
    Some(meta)
}

/// Impressão digital de um ficheiro para a cache dos slots legados:
/// `(caminho, (mtime em nanos, tamanho))`.
fn file_stamp(path: &Path) -> (PathBuf, Option<(u128, u64)>) {
    let meta = std::fs::metadata(path).ok();
    let mtime = meta
        .as_ref()
        .and_then(|meta| meta.modified().ok())
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos());
    (
        path.to_path_buf(),
        mtime.map(|mtime| (mtime, meta.as_ref().map_or(0, std::fs::Metadata::len))),
    )
}

/// Cache dos slots do envelope JSON antigo.
///
/// O `bevy_ext_save::list_save_slots` abre e re-parseia até cinco JSON em
/// cada chamada, e o menu chama o [`list_slots`] a 2 Hz: sem mudança no disco
/// (caminho + mtime + tamanho de cada ficheiro), as linhas são as mesmas.
#[derive(Default)]
struct LegacySlotsCache {
    stamp: Vec<(PathBuf, Option<(u128, u64)>)>,
    rows: Vec<SaveSlotInfo>,
}

fn legacy_slots(world_key: Option<&str>, save_dir: Option<&Path>) -> Vec<SaveSlotInfo> {
    static CACHE: OnceLock<Mutex<Option<LegacySlotsCache>>> = OnceLock::new();

    let mut stamp = Vec::with_capacity(5);
    for slot in 0..=3u32 {
        stamp.push(file_stamp(&bevy_ext_save::save_path_for_slot(
            world_key,
            save_dir,
            Some(slot),
        )));
    }
    stamp.push(file_stamp(&bevy_ext_save::save_path_for(
        world_key, save_dir,
    )));

    let cache = CACHE.get_or_init(|| Mutex::new(None));
    let mut cached = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(rows) = cached.as_ref()
        && rows.stamp == stamp
    {
        return rows.rows.clone();
    }
    let rows = bevy_ext_save::list_save_slots(world_key, save_dir);
    *cached = Some(LegacySlotsCache {
        stamp,
        rows: rows.clone(),
    });
    rows
}

/// Slot do `viber.save()`/`viber.load()` (sem slot) gravado pela nativa:
/// `{key}.legacy.vsave.ron`. O envelope JSON antigo (`{key}.save.json`) é o
/// outro candidato a slot 4, tratado pelo [`bevy_ext_save::list_save_slots`].
fn native_legacy_slot(base: &Path, key: &str) -> Option<SaveSlotInfo> {
    let path = base.join(format!("{key}.legacy.vsave.ron"));
    let file = std::fs::metadata(&path).ok()?;
    let meta = read_meta(&path);
    let saved_at = meta
        .as_ref()
        .map(|meta| meta.saved_at)
        .filter(|saved_at| *saved_at > 0.0)
        .or_else(|| {
            file.modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs_f64())
        });
    Some(SaveSlotInfo {
        slot: 4,
        path: path.display().to_string(),
        exists: true,
        bytes: file.len(),
        saved_at,
        level: meta.as_ref().map_or(0, |meta| u32_level(meta.level)),
        gold: meta.as_ref().map_or(0, |meta| u32_level(meta.gold)),
        scene: meta.as_ref().map(|meta| meta.scene.clone()),
        playtime: meta.as_ref().map(|meta| meta.playtime),
        title: meta.as_ref().map(|meta| meta.title.clone()),
    })
}

/// Lista os quatro slots nativos; se um slot ainda só tiver o JSON antigo,
/// devolve o metadata antigo para o menu continuar a mostrá-lo.
///
/// O **slot 4** é o `viber.save()`/`viber.load()` (sem slot), como no
/// monólito: a UI pede-o ao load com `slot = None` e assim o save escrito por
/// `viber.save()` deixa de ser invisível no menu. Mapear `None` → slot 0
/// (autosave) foi rejeitado: uma gravação manual apagaria o autosave.
pub fn list_slots(world_key: Option<&str>, save_dir: Option<&Path>) -> Vec<SaveSlotInfo> {
    let old = legacy_slots(world_key, save_dir);
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
                level: u32_level(meta.level),
                gold: u32_level(meta.gold),
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
    // A nativa manda; o envelope JSON antigo só aparece se ela não existir.
    if let Some(legacy) =
        native_legacy_slot(&base, &key).or_else(|| old.iter().find(|row| row.slot == 4).cloned())
    {
        rows.push(legacy);
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
    let game = match bevy_ext_save::load_from_disk(&old_path) {
        Ok(game) => game,
        Err(err) => {
            // "slot sem ficheiro" para um ficheiro que EXISTE mas não abre é
            // uma mensagem que esconde a corrupção do envelope antigo.
            if old_path.is_file() {
                warn!("vael save: {} ilegível ({err})", old_path.display());
            }
            return None;
        }
    };
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
    use crate::boat::BoatPose;
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

    /// O moonshine escreve no temporário: só o rename mexe no slot, por isso
    /// uma gravação interrompida não perde o save anterior.
    #[test]
    fn grava_no_temporario_e_so_o_rename_muda_o_slot() {
        let dir = std::env::temp_dir().join(format!("vael-save-atomic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("vael.auto.vsave.ron");
        std::fs::write(&path, "save antigo").expect("antigo");
        let tmp = tmp_path(&path);
        std::fs::write(&tmp, "save novo").expect("novo");

        assert_eq!(
            std::fs::read_to_string(&path).expect("slot"),
            "save antigo",
            "o slot só muda no commit"
        );
        commit_save(&tmp, &path).expect("commit");
        assert_eq!(std::fs::read_to_string(&path).expect("slot"), "save novo");
        assert!(!tmp.exists(), "o temporário sai depois do rename");
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

    fn unique_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vael-save-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).expect("dir");
        dir
    }

    fn meta() -> SlotMeta {
        SlotMeta {
            save_id: "id".into(),
            saved_at: 12.0,
            playtime: 3.0,
            title: "Vael".into(),
            level: 5,
            gold: 7,
            scene: "main".into(),
        }
    }

    /// O `viber.save()` (sem slot) grava `{key}.legacy.vsave.ron`; o menu tem
    /// de o ver como slot 4 (a UI pede o 4 ao load como `slot = None`).
    #[test]
    fn o_save_sem_slot_aparece_como_slot_4() {
        let dir = unique_dir("legacy-slot");
        let path = dir.join("vael.legacy.vsave.ron");
        std::fs::write(&path, "dados").expect("ron");
        write_meta(&path, &meta()).expect("meta");

        let rows = list_slots(Some("vael"), Some(&dir));
        let row = rows.iter().find(|row| row.slot == 4).expect("slot 4");
        assert!(row.exists);
        assert_eq!(row.path, path.display().to_string());
        assert_eq!(row.level, 5);
        assert_eq!(row.gold, 7);
        assert_eq!(row.scene.as_deref(), Some("main"));
        assert_eq!(row.saved_at, Some(12.0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Sem sidecar (kill entre o rename e a meta) o slot continua listado: um
    /// save existente não pode ficar invisível no menu.
    #[test]
    fn o_save_sem_slot_sem_sidecar_ainda_aparece() {
        let dir = unique_dir("legacy-sem-meta");
        std::fs::write(dir.join("vael.legacy.vsave.ron"), "dados").expect("ron");

        let rows = list_slots(Some("vael"), Some(&dir));
        let row = rows.iter().find(|row| row.slot == 4).expect("slot 4");
        assert!(row.exists, "o ficheiro existe");
        assert!(row.saved_at.is_some(), "cai no mtime");
        assert_eq!(row.level, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Um sidecar de outra versão (sem título/nível/ouro) ou com valores
    /// estranhos não pode falhar a leitura e esconder um save que existe.
    #[test]
    fn sidecar_leniente_nao_esconde_o_slot() {
        let dir = unique_dir("meta-leniente");
        let path = dir.join("vael.slot1.vsave.ron");
        std::fs::write(&path, "dados").expect("ron");
        // Só os campos "antigos"; `level` fora do intervalo do `u32`.
        std::fs::write(
            meta_path(&path),
            r#"{"save_id":"x","saved_at":5.0,"playtime":1.0,"scene":"main","level":-4}"#,
        )
        .expect("meta");

        let rows = list_slots(Some("vael"), Some(&dir));
        let row = rows.iter().find(|row| row.slot == 1).expect("slot 1");
        assert!(row.exists);
        assert_eq!(row.level, 0, "negativo clampa em 0");
        assert_eq!(row.gold, 0);
        assert_eq!(row.scene.as_deref(), Some("main"));
        assert_eq!(row.saved_at, Some(5.0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A nativa manda: se os dois existirem, o slot 4 é o `.legacy.vsave.ron`
    /// (é o que o load lê primeiro, ver o `process_save_requests`).
    #[test]
    fn o_slot_4_prefere_a_nativa_ao_envelope_json() {
        let dir = unique_dir("legacy-prioridade");
        let native = dir.join("vael.legacy.vsave.ron");
        std::fs::write(&native, "dados").expect("ron");
        write_meta(&native, &meta()).expect("meta");
        let old = bevy_ext_save::save_path_for(Some("vael"), Some(&dir));
        let mut game = bevy_ext_save::SaveGame::default();
        game.world_kv
            .insert("save_level".into(), serde_json::json!(2));
        bevy_ext_save::save_to_disk(&old, &game).expect("json antigo");

        let rows = list_slots(Some("vael"), Some(&dir));
        let row = rows.iter().find(|row| row.slot == 4).expect("slot 4");
        assert_eq!(row.path, native.display().to_string());
        assert_eq!(row.level, 5);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// O envelope JSON antigo continua a aparecer no slot 4, e a cache por
    /// (caminho, mtime, tamanho) apanha um ficheiro escrito depois.
    #[test]
    fn o_envelope_json_antigo_aparece_e_a_cache_ve_o_ficheiro_novo() {
        #[rustversion::since(1.99)]
        fn set_mtime(path: &Path, time: std::time::SystemTime) -> std::io::Result<()> {
            std::fs::set_times(path, std::fs::FileTimes::new().set_modified(time))
        }

        #[rustversion::before(1.99)]
        fn set_mtime(path: &Path, time: std::time::SystemTime) -> std::io::Result<()> {
            std::fs::File::options()
                .write(true)
                .open(path)?
                .set_modified(time)
        }

        let dir = unique_dir("legacy-cache");
        let first = list_slots(Some("vael"), Some(&dir));
        assert!(first.iter().all(|row| row.slot != 4), "nada gravado ainda");

        let old = bevy_ext_save::save_path_for(Some("vael"), Some(&dir));
        let mut game = bevy_ext_save::SaveGame::default();
        game.world_kv
            .insert("save_level".into(), serde_json::json!(3));
        bevy_ext_save::save_to_disk(&old, &game).expect("json antigo");
        let first_mtime = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        set_mtime(&old, first_mtime).unwrap();
        let size = std::fs::metadata(&old).unwrap().len();

        let rows = list_slots(Some("vael"), Some(&dir));
        let row = rows.iter().find(|row| row.slot == 4).expect("slot 4");
        assert_eq!(row.level, 3);

        // Mesmo caminho e tamanho: só a data distingue a nova gravação.
        game.world_kv
            .insert("save_level".into(), serde_json::json!(9));
        bevy_ext_save::save_to_disk(&old, &game).expect("json antigo");
        set_mtime(&old, first_mtime + std::time::Duration::from_secs(10)).unwrap();
        assert_eq!(std::fs::metadata(&old).unwrap().len(), size);
        let rows = list_slots(Some("vael"), Some(&dir));
        let row = rows.iter().find(|row| row.slot == 4).expect("slot 4");
        assert_eq!(row.level, 9);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn autumn_date_and_hour_survive_loading_a_slot() {
        let _guard = viber::luau::game::test_support::PROCESS_STORE_TEST_LOCK
            .lock()
            .unwrap();
        let mut world = World::new();
        let date = CivilDate::new(2028, 11, 4);
        world.insert_resource(WorldClock::new(date).at_hour(16.75));
        let mut payload = capture(&mut world).unwrap();
        world.insert_resource(WorldClock::default().with_time_scale(42.0));
        apply_payload(&mut world, &payload);
        let loaded = world.resource::<WorldClock>();
        assert_eq!(
            (loaded.date, loaded.hour, loaded.time_scale),
            (date, 16.75, 42.0)
        );

        let mut old: Value = serde_json::from_str(&payload.save_json).unwrap();
        old.as_object_mut().unwrap().remove("calendar");
        payload.save_json = old.to_string();
        world.insert_resource(WorldClock::default());
        apply_payload(&mut world, &payload);
        assert_eq!(
            world.resource::<WorldClock>().date,
            WorldClock::default().date
        );
    }

    #[test]
    fn loading_mid_swim_restores_remaining_stamina_and_the_same_shore() {
        let _lock = viber::luau::game::test_support::PROCESS_STORE_TEST_LOCK.lock().unwrap();
        let mut world = World::new();
        world.init_resource::<VaelSave>();
        let bus = crate::player::HeroMotionBus::default();
        world.insert_resource(bus.clone());
        let shore = Vec3::new(15.0, 8.0, -42.0);
        bus.lock().stamina.shore = Some(shore);
        bus.lock().stamina.tick(9.0, true, false, false);
        let swimming = bus.lock().stamina.clone();
        let mut payload = capture(&mut world).unwrap();
        bus.lock().stamina = Default::default();
        apply_payload(&mut world, &payload);
        assert_eq!(bus.lock().stamina, swimming);

        let mut old: Value = serde_json::from_str(&payload.save_json).unwrap();
        old.as_object_mut().unwrap().remove("stamina");
        payload.save_json = old.to_string();
        apply_payload(&mut world, &payload);
        assert_eq!(bus.lock().stamina, Default::default());
    }

    /// O `capture` guarda a pose dos barcos e o `apply_payload` volta a pô-la
    /// por aplicar — o `drive_boat_states` é que a aplica às entidades.
    #[test]
    fn capture_e_apply_levam_as_poses_dos_barcos() {
        // O store de `viber.game()` é do processo e o `apply_payload` escreve
        // nele: o lock serializa com os outros testes que o tocam.
        let _store_lock = viber::luau::game::test_support::PROCESS_STORE_TEST_LOCK
            .lock()
            .expect("lock do store");
        let pose = BoatPose {
            name: "barco-a".into(),
            position: Vec3::new(4.0, 1.5, -8.0),
            yaw: 0.5,
            mooring: None,
        };
        let mut world = World::new();
        world.init_resource::<VaelSave>();
        world.insert_resource(BoatStates {
            poses: vec![pose.clone()],
            riding: Some("barco-a".into()),
            ..Default::default()
        });

        let payload = capture(&mut world).expect("payload");
        let save: VaelSave = serde_json::from_str(&payload.save_json).expect("save_json");
        assert_eq!(save.boats, vec![pose.clone()]);
        assert_eq!(save.riding_boat.as_deref(), Some("barco-a"));

        // O load repõe o estado, ainda por aplicar.
        apply_payload(&mut world, &payload);
        let states = world.resource::<BoatStates>();
        assert_eq!(states.poses, vec![pose]);
        assert_eq!(states.riding.as_deref(), Some("barco-a"));
        assert!(states.pending);
    }

    /// Um payload com `position`/volumes não finitos não pode estragar a
    /// física nem silenciar o áudio.
    #[test]
    fn apply_ignora_posicao_e_volumes_nao_finitos() {
        let _store_lock = viber::luau::game::test_support::PROCESS_STORE_TEST_LOCK
            .lock()
            .expect("lock do store");
        let mut world = World::new();
        world.init_resource::<VaelSave>();
        world.insert_resource(AudioMixerSettings::default());
        let mut payload = payload();
        payload.position = Some(Vec3::new(f32::NAN, 0.0, 1.0));
        payload.volumes = Vec3::new(f32::INFINITY, 0.5, f32::NAN);
        payload.volumes_extra = Vec2::new(0.25, f32::NAN);

        apply_payload(&mut world, &payload);

        let mixer = world.resource::<AudioMixerSettings>();
        assert_eq!(mixer.music, 0.5);
        assert_eq!(mixer.voice, 0.25);
        // Um NaN mantinha o valor anterior (aqui, o default) e não o estraga.
        assert!(mixer.master.is_finite());
        assert!(mixer.sfx.is_finite());
        assert!(mixer.ambience.is_finite());
    }
}

//! Marcos/viagem em Rust: o catálogo de `data/landmarks.json` e a seleção do
//! marco mais próximo.
//!
//! As funções puras — catálogo saneado, etiquetas, revelação/assinatura e
//! [`LandmarkRules::nearest`] — correm sem a VM. O sistema Bevy junta as
//! posições reais (`Name` + `GlobalTransform`) e publica-as em
//! [`TravelState`]; a bridge [`install`] expõe `viber.vael.travel_*` no
//! arranque, depois da economia.
//!
//! Ficam no Lua: o teleporte/troca de cena (`travel.go`), os marcadores
//! (`viber.markers`), o mapa/UI e a validação da viagem rápida. Com
//! `VAEL_TRAVEL_RUST=1` o `lib/landmarks.lua` lê o catálogo da bridge; sem o
//! gate (default) mantém o JSON e a lógica antiga como fallback.
//!
//! Chaves de estado (as mesmas do Lua):
//! - `biome.<bioma>` = `true` → bioma revelado (`rules.biome_revealed`);
//! - `mark.<marco>` = `true` → marco assinado (`landmarks.is_marked`).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use mlua::{Lua, Table, Value};
use serde_json::Value as JsonValue;
use viber::luau::host::LuaScriptHost;
use bevy_ext_core::Player;

use crate::components::VaelPlayer;
use crate::content::{LandmarkDef, LandmarksFile, VaelContent};
use crate::save::VaelSave;
use crate::vault::{vael_fn, vael_table};

/// Variável de ambiente que liga a autoridade dos marcos (`1`/`true`/`on`/`yes`).
pub const RUST_TRAVEL_ENV: &str = "VAEL_TRAVEL_RUST";
/// Chave do `viber.game()` que avisa o Lua de que a bridge é a fonte dos dados.
pub const RUST_GATE_KEY: &str = "vael_travel_rust";
/// `campfire_range` do Lua quando o conteúdo não traz um valor válido.
pub const DEFAULT_CAMPFIRE_RANGE: f64 = 14.0;
/// Prefixo do bioma revelado no `viber.game()`.
pub const BIOME_KEY_PREFIX: &str = "biome.";
/// Prefixo do marco assinado no `viber.game()`.
pub const MARK_KEY_PREFIX: &str = "mark.";
/// Período da fotografia: o `publish` do `travel.lua` também corre a 2 Hz.
pub const TRAVEL_PERIOD_SECS: f32 = 0.5;

// ── catálogo ────────────────────────────────────────────────────────────

/// Marco já saneado: o conteúdo editável não pode rebentar consumidores.
#[derive(Debug, Clone, PartialEq)]
pub struct Landmark {
    pub name: String,
    pub biome: String,
    pub label: String,
    pub radius: f64,
    pub survey: String,
    pub pilgrim: bool,
}

impl Landmark {
    /// `true` no bioma especial das atalaias (`biome == "tower"`).
    pub fn is_tower(&self) -> bool {
        self.biome == "tower"
    }

    /// Um marco sem bioma fica fechado até o conteúdo ser corrigido.
    pub fn has_biome(&self) -> bool {
        !self.biome.is_empty()
    }

    /// Saneia uma entrada do JSON; `None` para um nome vazio.
    fn from_def(def: &LandmarkDef) -> Option<Self> {
        if def.name.is_empty() {
            return None;
        }
        // Sem nome autoral, o id é o fallback (como o `label` do Lua).
        let label = if def.label.is_empty() {
            def.name.clone()
        } else {
            def.label.clone()
        };
        // `radius` ausente/NaN/negativo vale 0: 0 = não assina.
        let radius = if def.radius.is_finite() && def.radius > 0.0 {
            def.radius
        } else {
            0.0
        };
        Some(Self {
            name: def.name.clone(),
            biome: def.biome.clone(),
            label,
            radius,
            survey: def.survey.clone().unwrap_or_default(),
            pilgrim: def.pilgrim,
        })
    }
}

/// Catálogo de marcos derivado de `data/landmarks.json`.
#[derive(Debug, Clone, PartialEq)]
pub struct LandmarkRules {
    /// Raio do aviso de fogueira (o `CAMPFIRE_RANGE` do Lua).
    pub campfire_range: f64,
    /// Chave = bioma; valor = nome no mapa (o `BIOME_LABEL` do Lua).
    pub biome_labels: BTreeMap<String, String>,
    /// Marcos válidos, na ordem do conteúdo.
    pub landmarks: Vec<Landmark>,
    /// Nome → índice do PRIMEIRO marco com esse nome (o `label` do Lua).
    index: BTreeMap<String, usize>,
}

impl Default for LandmarkRules {
    fn default() -> Self {
        Self::from_file(&LandmarksFile::default())
    }
}

impl LandmarkRules {
    /// Saneia o catálogo carregado por [`VaelContent`](crate::content::VaelContent).
    pub fn from_file(file: &LandmarksFile) -> Self {
        let campfire_range = if file.campfire_range.is_finite() && file.campfire_range > 0.0 {
            file.campfire_range
        } else {
            DEFAULT_CAMPFIRE_RANGE
        };
        let mut biome_labels = BTreeMap::new();
        for (biome, label) in &file.biome_labels {
            if !biome.is_empty() {
                biome_labels.insert(biome.clone(), label.clone());
            }
        }
        let mut landmarks = Vec::with_capacity(file.landmarks.len());
        let mut index = BTreeMap::new();
        for def in &file.landmarks {
            let Some(mark) = Landmark::from_def(def) else {
                continue;
            };
            index.entry(mark.name.clone()).or_insert(landmarks.len());
            landmarks.push(mark);
        }
        Self {
            campfire_range,
            biome_labels,
            landmarks,
            index,
        }
    }

    /// Marco pelo nome (o primeiro, quando o conteúdo repete o nome).
    pub fn get(&self, name: &str) -> Option<&Landmark> {
        self.index
            .get(name)
            .and_then(|&index| self.landmarks.get(index))
    }

    /// Nome autoral do marco; um nome desconhecido devolve-se a si mesmo.
    pub fn label(&self, name: &str) -> String {
        match self.get(name) {
            Some(mark) => mark.label.clone(),
            None => name.to_string(),
        }
    }

    /// Nome do bioma no mapa; `None` quando o JSON não o traduz.
    pub fn biome_label(&self, biome: &str) -> Option<&str> {
        self.biome_labels.get(biome).map(String::as_str)
    }

    /// Marco mais próximo em XZ (o `Y` não entra: uma atalaia vive no topo da
    /// plataforma e não pode perder para o chão).
    ///
    /// Empate: fica o primeiro da lista (comparação estrita, como o Lua).
    /// Marcos sem posição conhecida não entram.
    pub fn nearest(
        &self,
        positions: &BTreeMap<String, [f32; 3]>,
        px: f32,
        pz: f32,
    ) -> Option<NearestLandmark> {
        let mut best: Option<NearestLandmark> = None;
        for (index, mark) in self.landmarks.iter().enumerate() {
            let Some(position) = positions.get(&mark.name) else {
                continue;
            };
            let distance = distance_xz(*position, [px, 0.0, pz]);
            if !distance.is_finite() {
                continue;
            }
            let better = match &best {
                None => true,
                Some(best) => distance < best.distance,
            };
            if better {
                best = Some(NearestLandmark {
                    index,
                    name: mark.name.clone(),
                    distance,
                });
            }
        }
        best
    }

    /// Foco completo (etiquetas + estado do save) do marco mais próximo.
    pub fn focus(
        &self,
        positions: &BTreeMap<String, [f32; 3]>,
        save: &VaelSave,
        px: f32,
        pz: f32,
    ) -> Option<TravelFocus> {
        let nearest = self.nearest(positions, px, pz)?;
        let mark = self.landmarks.get(nearest.index)?;
        let position = positions.get(&mark.name).copied()?;
        let revealed = mark.has_biome() && is_revealed(save, &mark.biome);
        Some(TravelFocus {
            name: mark.name.clone(),
            label: mark.label.clone(),
            biome: mark.biome.clone(),
            biome_label: self
                .biome_label(&mark.biome)
                .unwrap_or(mark.biome.as_str())
                .to_string(),
            position,
            distance: nearest.distance,
            surveyed: is_surveyed(save, &mark.name),
            revealed,
            available: available(mark, revealed),
        })
    }
}

/// Distância em XZ entre dois pontos.
pub fn distance_xz(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dz = a[2] - b[2];
    (dx * dx + dz * dz).sqrt()
}

/// Um marco está disponível? Atalaias e marcos da peregrinação não dependem
/// de bioma revelado; um marco sem bioma fica fechado (o `available` do Lua).
pub fn available(mark: &Landmark, revealed: bool) -> bool {
    if mark.is_tower() || mark.pilgrim {
        return true;
    }
    if !mark.has_biome() {
        return false;
    }
    revealed
}

/// Marco mais próximo (índice no catálogo + distância XZ).
#[derive(Debug, Clone, PartialEq)]
pub struct NearestLandmark {
    pub index: usize,
    pub name: String,
    pub distance: f32,
}

/// Foco de viagem: o marco mais próximo e o estado que o mapa/HUD mostram.
#[derive(Debug, Clone, PartialEq)]
pub struct TravelFocus {
    pub name: String,
    pub label: String,
    pub biome: String,
    pub biome_label: String,
    pub position: [f32; 3],
    /// Distância XZ ao jogador.
    pub distance: f32,
    /// `mark.<nome>` no save.
    pub surveyed: bool,
    /// `biome.<bioma>` no save.
    pub revealed: bool,
    /// Regra do `landmarks.available`.
    pub available: bool,
}

// ── revelação/assinatura (as chaves do Lua) ─────────────────────────────

/// `biome.<bioma> == true` no save (o `rules.biome_revealed`).
pub fn is_revealed(save: &VaelSave, biome: &str) -> bool {
    if biome.is_empty() {
        return false;
    }
    matches!(
        save.legacy.get(&format!("{BIOME_KEY_PREFIX}{biome}")),
        Some(JsonValue::Bool(true))
    )
}

/// Marca um bioma como revelado; devolve `false` para um nome vazio.
pub fn reveal_biome(save: &mut VaelSave, biome: &str) -> bool {
    if biome.is_empty() {
        return false;
    }
    save.legacy
        .insert(format!("{BIOME_KEY_PREFIX}{biome}"), JsonValue::Bool(true));
    true
}

/// `mark.<nome> == true` no save (o `landmarks.is_marked`).
pub fn is_surveyed(save: &VaelSave, name: &str) -> bool {
    !name.is_empty() && save.markers.contains(name)
}

/// Biomas revelados no save (só as chaves `biome.<nome> == true`).
pub fn revealed_biomes(save: &VaelSave) -> BTreeSet<String> {
    save.legacy
        .iter()
        .filter_map(|(key, value)| {
            if matches!(value, JsonValue::Bool(true)) {
                key.strip_prefix(BIOME_KEY_PREFIX)
                    .filter(|biome| !biome.is_empty())
                    .map(str::to_owned)
            } else {
                None
            }
        })
        .collect()
}

// ── fotografia do sistema Bevy ───────────────────────────────────────────

/// Posições e seleção do frame: o sistema escreve, a bridge lê.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TravelSnapshot {
    /// Posição do herói; `None` antes do spawn.
    pub player: Option<[f32; 3]>,
    /// Nome do marco → posição de mundo conhecida.
    pub positions: BTreeMap<String, [f32; 3]>,
    pub nearest: Option<TravelFocus>,
}

/// Handle partilhado da fotografia (`Arc<Mutex>`): sistema escreve, bridge lê.
#[derive(Resource, Debug, Clone, Default)]
pub struct TravelState(pub Arc<Mutex<TravelSnapshot>>);

impl TravelState {
    /// Cópia da fotografia (lock recuperado mesmo depois de poison).
    pub fn get(&self) -> TravelSnapshot {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Substitui a fotografia.
    pub fn set(&self, snapshot: TravelSnapshot) {
        *self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = snapshot;
    }

    /// Substitui só quando algo mudou (evita acordar leitores da bridge e
    /// reescrever um mapa de 22 marcos a 2 Hz sem necessidade).
    pub fn set_if_changed(&self, snapshot: TravelSnapshot) {
        let mut current = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if *current != snapshot {
            *current = snapshot;
        }
    }
}

/// Catálogo do sistema, construído no `Startup` a partir do [`VaelContent`].
#[derive(Resource, Debug, Clone, Default)]
pub struct VaelTravelRules(pub Arc<LandmarkRules>);

/// Gate `VAEL_TRAVEL_RUST` (opt-in: só com `1`/`true`/`on`/`yes`).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct VaelTravelConfig {
    pub rust: bool,
}

impl VaelTravelConfig {
    pub fn from_raw(raw: Option<&str>) -> Self {
        Self {
            rust: rust_enabled(raw),
        }
    }

    pub fn from_env() -> Self {
        Self::from_raw(std::env::var(RUST_TRAVEL_ENV).ok().as_deref())
    }
}

impl Default for VaelTravelConfig {
    fn default() -> Self {
        Self::from_env()
    }
}

fn rust_enabled(raw: Option<&str>) -> bool {
    matches!(
        raw.map(|value| value.trim().to_ascii_lowercase())
            .as_deref(),
        Some("1" | "true" | "on" | "yes")
    )
}

// ── sistemas ────────────────────────────────────────────────────────────

/// Recolhe as posições dos marcos e atualiza o mais próximo.
///
/// O `Y` do marco fica guardado (a viagem precisa dele) mas a distância é
/// XZ. Sem herói (menu/interior ainda a carregar) a fotografia fica vazia.
fn update_travel_state(
    rules: Option<Res<VaelTravelRules>>,
    save: Option<Res<VaelSave>>,
    state: Res<TravelState>,
    time: Res<Time>,
    mut last: Local<Option<f32>>,
    players: Query<&GlobalTransform, Or<(With<Player>, With<VaelPlayer>)>>,
    named: Query<(&Name, &GlobalTransform)>,
) {
    let Some(rules) = rules else {
        return;
    };
    // A varredura de nomes é cara (~79k entidades no mundo): 2 Hz chegam,
    // como o `publish` do Lua. O primeiro frame corre sempre.
    let now = time.elapsed_secs();
    if let Some(last) = *last
        && now - last < TRAVEL_PERIOD_SECS
    {
        return;
    }
    *last = Some(now);
    let rules = &rules.0;
    // `iter().next()` e não `single()`: com dois heróis (co-op) o `single`
    // falhava e a fotografia ficava presa.
    let Some(player) = players.iter().next() else {
        state.set_if_changed(TravelSnapshot::default());
        return;
    };
    let player_at = player.translation().to_array();
    let mut positions = BTreeMap::new();
    for (name, transform) in &named {
        let name = name.as_str();
        if rules.get(name).is_none() {
            continue;
        }
        // Nome repetido no mundo: fica a PRIMEIRA entidade, como o `viber.find`.
        positions
            .entry(name.to_string())
            .or_insert_with(|| transform.translation().to_array());
    }
    // Sem `VaelSave` (apps mínimas de teste) o estado é tudo fechado.
    let empty = VaelSave::default();
    let save = save.as_deref().unwrap_or(&empty);
    let nearest = rules.focus(&positions, save, player_at[0], player_at[2]);
    state.set_if_changed(TravelSnapshot {
        player: Some(player_at),
        positions,
        nearest,
    });
}

// ── bridge Luau ─────────────────────────────────────────────────────────

/// Plugin de viagem/marcos: instala `viber.vael.travel_*` depois da economia.
pub struct VaelTravelPlugin;

impl Plugin for VaelTravelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VaelTravelConfig>()
            .init_resource::<TravelState>()
            .add_systems(
                Startup,
                (
                    insert_travel_rules,
                    apply_travel_gate,
                    // A tabela `viber.vael` já existe: acrescenta as chaves.
                    install_travel_api
                        .after(crate::economy::install_economy_api)
                        .after(apply_travel_gate),
                ),
            )
            .add_systems(
                PreUpdate,
                (
                    // Um save carregado depois do `Startup` pode repor a chave
                    // do gate; corrige-se antes dos scripts do `Update`.
                    enforce_travel_gate,
                    update_travel_state.after(crate::state_sync::sync_state_from_lua),
                )
                    .run_if(viber::app::scripts_enabled),
            );
    }
}

/// Arranque: constrói o catálogo para o sistema.
fn insert_travel_rules(mut commands: Commands, content: Option<Res<VaelContent>>) {
    let Some(content) = content else {
        warn!("vael: viagem sem conteúdo carregado — catálogo de marcos vazio");
        return;
    };
    let rules = LandmarkRules::from_file(&content.landmarks);
    commands.insert_resource(VaelTravelRules(Arc::new(rules)));
}

/// Arranque: liga a chave do gate no `viber.game()` (default desligado).
fn apply_travel_gate(config: Res<VaelTravelConfig>, host: Option<Res<LuaScriptHost>>) {
    let Some(host) = host else {
        if config.rust {
            warn!("vael: VAEL_TRAVEL_RUST sem host Luau — gate inativo");
        }
        return;
    };
    write_gate(&host, config.rust);
}

/// Corrige o gate a cada frame (save carregado, restart de scripts).
fn enforce_travel_gate(config: Res<VaelTravelConfig>, host: Option<Res<LuaScriptHost>>) {
    let Some(host) = host else {
        return;
    };
    write_gate(&host, config.rust);
}

/// Escreve a chave só quando o valor guardado não corresponde ao modo.
fn write_gate(host: &LuaScriptHost, rust: bool) {
    let Ok(game) = host.lua.named_registry_value::<Table>("viber_game") else {
        return;
    };
    let on = matches!(
        game.raw_get::<Value>(RUST_GATE_KEY),
        Ok(Value::Boolean(true))
    );
    let result = if rust && !on {
        game.raw_set(RUST_GATE_KEY, true)
    } else if !rust && on {
        game.raw_set(RUST_GATE_KEY, Value::Nil)
    } else {
        Ok(())
    };
    if let Err(err) = result {
        error!("vael: falha a escrever `{RUST_GATE_KEY}`: {err}");
    }
}

/// Arranque: acrescenta as funções de viagem à tabela `viber.vael`.
fn install_travel_api(
    host: Option<Res<LuaScriptHost>>,
    content: Option<Res<VaelContent>>,
    state: Res<TravelState>,
) {
    let Some(host) = host else {
        warn!("vael: viagem sem host Luau — `viber.vael` não estendido");
        return;
    };
    let Some(content) = content else {
        warn!("vael: viagem sem conteúdo carregado — `viber.vael` não estendido");
        return;
    };
    let rules = LandmarkRules::from_file(&content.landmarks);
    let marcos = rules.landmarks.len();
    match install(&host.lua, Arc::new(rules), &state) {
        Ok(()) => info!(marcos, "vael: marcos/viagem em Rust ligados (`viber.vael`)"),
        Err(err) => error!("vael: falha a instalar viagem em `viber.vael`: {err}"),
    }
}

/// Acrescenta `travel_landmarks`, `travel_nearest`, `travel_reveal`,
/// `travel_is_revealed` e `travel_label` a `viber.vael` sem substituir as
/// funções que já lá estão.
pub(crate) fn install(
    lua: &Lua,
    rules: Arc<LandmarkRules>,
    state: &TravelState,
) -> mlua::Result<()> {
    let vael = vael_table(lua)?;

    vael_fn!(
        lua,
        vael,
        "travel_landmarks",
        [rules = Arc::clone(&rules), state = state.clone()],
        move |lua, ()| {
            let game: Table = lua.named_registry_value("viber_game")?;
            let snapshot = state.get();
            let rows = lua.create_table()?;
            for (index, mark) in rules.landmarks.iter().enumerate() {
                let position = snapshot.positions.get(&mark.name).copied();
                let distance = match (position, snapshot.player) {
                    (Some(position), Some(player)) => Some(distance_xz(position, player)),
                    _ => None,
                };
                let row = landmark_row(lua, &rules, &game, mark, position, distance)?;
                rows.set(index + 1, row)?;
            }
            Ok(rows)
        }
    );

    vael_fn!(
        lua,
        vael,
        "travel_nearest",
        [rules = Arc::clone(&rules), state = state.clone()],
        move |lua, (px, pz): (Option<f64>, Option<f64>)| {
            let snapshot = state.get();
            let (px, pz) = match (px, pz) {
                (Some(px), Some(pz)) => (px as f32, pz as f32),
                // Sem coordenadas: o herói da fotografia.
                _ => match snapshot.player {
                    Some(player) => (player[0], player[2]),
                    None => return Ok(Value::Nil),
                },
            };
            let Some(nearest) = rules.nearest(&snapshot.positions, px, pz) else {
                return Ok(Value::Nil);
            };
            let Some(mark) = rules.landmarks.get(nearest.index) else {
                return Ok(Value::Nil);
            };
            let game: Table = lua.named_registry_value("viber_game")?;
            let position = snapshot.positions.get(&mark.name).copied();
            let row = landmark_row(lua, &rules, &game, mark, position, Some(nearest.distance))?;
            Ok(Value::Table(row))
        }
    );

    vael_fn!(
        lua,
        vael,
        "travel_reveal",
        move |lua, biome: Option<String>| {
            let biome = biome.unwrap_or_default();
            if biome.is_empty() {
                return Ok(false);
            }
            let game: Table = lua.named_registry_value("viber_game")?;
            game.raw_set(format!("{BIOME_KEY_PREFIX}{biome}"), true)?;
            Ok(true)
        }
    );

    vael_fn!(lua, vael, "travel_is_revealed", move |lua,
                                                    biome: Option<
        String,
    >| {
        let biome = biome.unwrap_or_default();
        if biome.is_empty() {
            return Ok(false);
        }
        let game: Table = lua.named_registry_value("viber_game")?;
        let key = format!("{BIOME_KEY_PREFIX}{biome}");
        Ok(matches!(game.raw_get::<Value>(key)?, Value::Boolean(true)))
    });

    vael_fn!(
        lua,
        vael,
        "travel_label",
        [rules = Arc::clone(&rules)],
        move |lua, name: Option<String>| {
            let name = name.unwrap_or_default();
            if name.is_empty() {
                return Ok(Value::Nil);
            }
            let label = rules.label(&name);
            Ok(Value::String(lua.create_string(&label)?))
        }
    );

    // Substitui o `travel.pin` do Lua (diálogo "Perguntar por um lugar").
    vael_fn!(
        lua,
        vael,
        "travel_pin",
        [rules = Arc::clone(&rules)],
        move |lua, name: Option<String>| {
            let name = name.unwrap_or_default();
            if name.is_empty() || rules.get(&name).is_none() {
                return Ok(false);
            }
            let game: Table = lua.named_registry_value("viber_game")?;
            game.raw_set("pin", name)?;
            Ok(true)
        }
    );

    Ok(())
}

/// Tabela de um marco para o Lua (a mesma forma de `travel_landmarks`).
fn landmark_row(
    lua: &Lua,
    rules: &LandmarkRules,
    game: &Table,
    mark: &Landmark,
    position: Option<[f32; 3]>,
    distance: Option<f32>,
) -> mlua::Result<Table> {
    let row = lua.create_table()?;
    row.set("name", mark.name.as_str())?;
    row.set("label", mark.label.as_str())?;
    row.set("biome", mark.biome.as_str())?;
    row.set(
        "biome_label",
        rules
            .biome_label(&mark.biome)
            .unwrap_or(mark.biome.as_str()),
    )?;
    row.set("radius", mark.radius)?;
    row.set("survey", mark.survey.as_str())?;
    row.set("pilgrim", mark.pilgrim)?;
    let revealed = if mark.has_biome() {
        lua_true(game, &format!("{BIOME_KEY_PREFIX}{}", mark.biome))?
    } else {
        false
    };
    row.set("revealed", revealed)?;
    row.set(
        "surveyed",
        lua_true(game, &format!("{MARK_KEY_PREFIX}{}", mark.name))?,
    )?;
    row.set("available", available(mark, revealed))?;
    if let Some([x, y, z]) = position {
        row.set("x", x)?;
        row.set("y", y)?;
        row.set("z", z)?;
    }
    if let Some(distance) = distance {
        row.set("distance", distance)?;
    }
    Ok(row)
}

/// `true` só para um booleano `true` (o `== true` do Lua).
fn lua_true(game: &Table, key: &str) -> mlua::Result<bool> {
    Ok(matches!(game.raw_get::<Value>(key)?, Value::Boolean(true)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn def(name: &str, biome: &str, label: &str, radius: f64) -> LandmarkDef {
        LandmarkDef {
            name: name.into(),
            biome: biome.into(),
            label: label.into(),
            radius,
            ..Default::default()
        }
    }

    /// Torre (bosque), arco (deserto), atalaia (torre) e a pedra da romaria.
    /// O nome vazio fica de fora.
    fn regras() -> LandmarkRules {
        let mut pedra = def("pedra", "alto", "Pedra do Caminho", 14.0);
        pedra.pilgrim = true;
        let file = LandmarksFile {
            campfire_range: 12.0,
            biome_labels: BTreeMap::from([
                ("dark_forest".to_string(), "bosque".to_string()),
                ("desert".to_string(), "deserto".to_string()),
            ]),
            landmarks: vec![
                def("torre", "dark_forest", "Torre", 10.0),
                def("arco", "desert", "Arco", 12.0),
                def("atalaia", "tower", "Atalaia", 8.0),
                pedra,
                def("", "desert", "Sem nome", 5.0),
            ],
            ..Default::default()
        };
        LandmarkRules::from_file(&file)
    }

    fn posicoes() -> BTreeMap<String, [f32; 3]> {
        BTreeMap::from([
            ("torre".to_string(), [10.0, 50.0, 0.0]),
            ("arco".to_string(), [-10.0, 0.0, 0.0]),
            ("atalaia".to_string(), [0.0, 0.0, 30.0]),
        ])
    }

    // ── catálogo/etiquetas ──────────────────────────────────────────────

    #[test]
    fn campfire_e_etiquetas_do_conteudo() {
        let rules = regras();
        assert_eq!(rules.campfire_range, 12.0);
        assert_eq!(rules.landmarks.len(), 4, "o nome vazio fica de fora");
        assert_eq!(rules.label("torre"), "Torre");
        assert_eq!(rules.label("desconhecido"), "desconhecido");
        assert_eq!(rules.biome_label("dark_forest"), Some("bosque"));
        assert_eq!(rules.biome_label("desert"), Some("deserto"));
        assert_eq!(rules.biome_label("sem-bioma"), None);
    }

    #[test]
    fn etiqueta_vazia_cai_para_o_nome_e_raio_invalido_vale_zero() {
        let file = LandmarksFile {
            landmarks: vec![
                LandmarkDef {
                    name: "sem-label".into(),
                    biome: "desert".into(),
                    label: String::new(),
                    radius: -3.0,
                    ..Default::default()
                },
                LandmarkDef {
                    name: "nan".into(),
                    biome: "desert".into(),
                    label: "NaN".into(),
                    radius: f64::NAN,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let rules = LandmarkRules::from_file(&file);
        assert_eq!(rules.label("sem-label"), "sem-label");
        assert_eq!(rules.get("sem-label").expect("marco").radius, 0.0);
        assert_eq!(rules.get("nan").expect("marco").radius, 0.0);
    }

    #[test]
    fn conteudo_ausente_nao_rebenta() {
        let rules = LandmarkRules::default();
        assert_eq!(rules.campfire_range, DEFAULT_CAMPFIRE_RANGE);
        assert!(rules.landmarks.is_empty());
        assert!(rules.nearest(&BTreeMap::new(), 0.0, 0.0).is_none());
        assert!(
            rules
                .focus(&BTreeMap::new(), &VaelSave::default(), 0.0, 0.0)
                .is_none()
        );
        assert_eq!(rules.label("qualquer"), "qualquer");
        assert_eq!(rules.biome_label("desert"), None);
    }

    // ── seleção ─────────────────────────────────────────────────────────

    #[test]
    fn nearest_usa_xz_e_o_y_do_marco_nao_conta() {
        let rules = regras();
        let near = rules
            .nearest(&posicoes(), 0.0, 0.0)
            .expect("marco mais próximo");
        // Torre e arco empatam a 10; a torre vem primeiro na lista.
        assert_eq!(near.name, "torre");
        assert!((near.distance - 10.0).abs() < 1e-4);
        // O Y=50 da torre não a afasta: a distância é XZ.
        assert_eq!(near.index, 0);
    }

    #[test]
    fn nearest_desempata_pelo_primeiro_da_lista() {
        let rules = regras();
        let mut positions = BTreeMap::new();
        positions.insert("torre".to_string(), [5.0, 0.0, 0.0]);
        positions.insert("arco".to_string(), [-5.0, 0.0, 0.0]);
        let near = rules.nearest(&positions, 0.0, 0.0).expect("empate");
        assert_eq!(near.name, "torre", "o primeiro da lista ganha o empate");
    }

    #[test]
    fn nearest_ignora_marcos_sem_posicao() {
        let rules = regras();
        let mut positions = BTreeMap::new();
        positions.insert("arco".to_string(), [-10.0, 0.0, 0.0]);
        // A torre não tem posição conhecida: o arco fica, mesmo mais longe.
        let near = rules.nearest(&positions, 10.0, 0.0).expect("arco");
        assert_eq!(near.name, "arco");
        assert!((near.distance - 20.0).abs() < 1e-4);
        assert!(rules.nearest(&BTreeMap::new(), 0.0, 0.0).is_none());
    }

    #[test]
    fn nearest_escolhe_o_mais_perto_quando_o_jogador_se_aproxima() {
        let rules = regras();
        let near = rules
            .nearest(&posicoes(), -9.0, 0.0)
            .expect("arco mais próximo");
        assert_eq!(near.name, "arco");
        assert!((near.distance - 1.0).abs() < 1e-4);
    }

    // ── revelação/assinatura ────────────────────────────────────────────

    #[test]
    fn revelar_e_assinar_gravam_as_chaves_do_lua() {
        let mut save = VaelSave::default();
        assert!(!is_revealed(&save, "dark_forest"));
        assert!(reveal_biome(&mut save, "dark_forest"));
        assert!(is_revealed(&save, "dark_forest"));
        assert!(!reveal_biome(&mut save, ""));
        assert!(!is_revealed(&save, ""));

        save.markers.insert("torre".to_string());
        assert!(is_surveyed(&save, "torre"));
        assert!(!is_surveyed(&save, "arco"));
        assert!(!is_surveyed(&save, ""));

        let kv = save.to_world_kv();
        assert_eq!(kv.get("biome.dark_forest"), Some(&json!(true)));
        assert_eq!(kv.get("mark.torre"), Some(&json!(true)));
        assert_eq!(
            revealed_biomes(&save),
            BTreeSet::from(["dark_forest".to_string()])
        );
    }

    #[test]
    fn foco_marca_disponibilidade_pelo_bioma_revelado() {
        let rules = regras();
        let positions = posicoes();
        let save = VaelSave::default();

        let torre = rules.focus(&positions, &save, 0.0, 0.0).expect("torre");
        assert_eq!(torre.name, "torre");
        assert_eq!(torre.label, "Torre");
        assert_eq!(torre.biome_label, "bosque");
        assert!(!torre.revealed);
        assert!(!torre.available, "bioma por revelar");
        assert!(!torre.surveyed);

        let mut save = VaelSave::default();
        reveal_biome(&mut save, "dark_forest");
        save.markers.insert("torre".to_string());
        let torre = rules.focus(&positions, &save, 0.0, 0.0).expect("torre");
        assert!(torre.revealed);
        assert!(torre.available);
        assert!(torre.surveyed);
    }

    #[test]
    fn atalaia_e_peregrinacao_estao_sempre_disponiveis() {
        let rules = regras();
        let atalaia = rules.get("atalaia").expect("atalaia");
        assert!(available(atalaia, false), "atalaia não depende de bioma");
        let pedra = rules.get("pedra").expect("pedra");
        assert!(available(pedra, false), "romaria não depende de bioma");

        let orfao = Landmark::from_def(&def("orfao", "", "Órfão", 5.0)).expect("marco");
        assert!(!available(&orfao, true), "sem bioma fica fechado");
    }

    // ── fotografia/sistema ──────────────────────────────────────────────

    #[test]
    fn sistema_publica_o_marco_mais_proximo() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(VaelTravelRules(Arc::new(regras())))
            .init_resource::<TravelState>()
            .add_systems(PreUpdate, update_travel_state);
        app.world_mut()
            .spawn((Player::default(), GlobalTransform::from_xyz(0.0, 0.0, 0.0)));
        app.world_mut().spawn((
            Name::new("torre"),
            GlobalTransform::from_xyz(10.0, 50.0, 0.0),
        ));
        app.world_mut().spawn((
            Name::new("arco"),
            GlobalTransform::from_xyz(-10.0, 0.0, 0.0),
        ));
        app.world_mut().spawn((
            Name::new("nao-e-marco"),
            GlobalTransform::from_xyz(1.0, 0.0, 0.0),
        ));

        app.update();

        let state = app.world().resource::<TravelState>().get();
        assert_eq!(state.player, Some([0.0, 0.0, 0.0]));
        assert_eq!(state.positions.len(), 2, "só marcos do conteúdo");
        let near = state.nearest.expect("marco mais próximo");
        assert_eq!(near.name, "torre");
        assert_eq!(near.position, [10.0, 50.0, 0.0]);
        assert!((near.distance - 10.0).abs() < 1e-4);
        assert!(!near.available, "sem save o bioma está fechado");
    }

    // ── bridge ──────────────────────────────────────────────────────────

    fn lua_com_bridge() -> (Lua, Table, TravelState) {
        let lua = Lua::new();
        lua.globals()
            .set("viber", lua.create_table().expect("tabela viber"))
            .expect("global viber");
        let game = lua.create_table().expect("tabela do jogo");
        lua.set_named_registry_value("viber_game", game.clone())
            .expect("registar viber_game");
        let state = TravelState::default();
        install(&lua, Arc::new(regras()), &state).expect("instalar travel_*");
        (lua, game, state)
    }

    #[test]
    fn bridge_devolve_marcos_estado_e_etiquetas() {
        let (lua, game, state) = lua_com_bridge();
        game.raw_set("biome.dark_forest", true).expect("bioma");
        game.raw_set("mark.torre", true).expect("assinatura");

        let mut positions = posicoes();
        positions.remove("atalaia");
        state.set(TravelSnapshot {
            player: Some([0.0, 0.0, 0.0]),
            positions,
            nearest: None,
        });

        let rows: Table = lua
            .load("return viber.vael.travel_landmarks()")
            .eval()
            .expect("travel_landmarks");
        assert_eq!(rows.raw_len(), 4, "as quatro entradas válidas");
        let torre = rows
            .sequence_values::<Table>()
            .flatten()
            .find(|row| row.get::<String>("name").is_ok_and(|name| name == "torre"))
            .expect("linha da torre");
        assert_eq!(torre.get::<String>("label").expect("label"), "Torre");
        assert_eq!(torre.get::<String>("biome_label").expect("bioma"), "bosque");
        assert_eq!(torre.get::<String>("biome").expect("bioma"), "dark_forest");
        assert!((torre.get::<f64>("radius").expect("raio") - 10.0).abs() < 1e-6);
        assert!(torre.get::<bool>("revealed").expect("revealed"));
        assert!(torre.get::<bool>("surveyed").expect("surveyed"));
        assert!(torre.get::<bool>("available").expect("available"));
        assert!((torre.get::<f64>("distance").expect("distância") - 10.0).abs() < 1e-4);
        assert!((torre.get::<f64>("y").expect("y") - 50.0).abs() < 1e-6);

        let arco = rows
            .sequence_values::<Table>()
            .flatten()
            .find(|row| row.get::<String>("name").is_ok_and(|name| name == "arco"))
            .expect("linha do arco");
        assert!(!arco.get::<bool>("revealed").expect("revealed"));
        assert!(!arco.get::<bool>("available").expect("available"));
    }

    #[test]
    fn bridge_proximo_por_argumentos_e_pelo_jogador() {
        let (lua, _game, state) = lua_com_bridge();
        let mut positions = posicoes();
        positions.remove("atalaia");
        state.set(TravelSnapshot {
            player: Some([0.0, 0.0, 0.0]),
            positions,
            nearest: None,
        });

        let perto: Table = lua
            .load("return viber.vael.travel_nearest(9, 0)")
            .eval()
            .expect("nearest com argumentos");
        assert_eq!(perto.get::<String>("name").expect("name"), "torre");
        assert!((perto.get::<f64>("distance").expect("distância") - 1.0).abs() < 1e-4);

        // Sem argumentos usa o herói da fotografia (empate → primeira da lista).
        let jogador: Table = lua
            .load("return viber.vael.travel_nearest()")
            .eval()
            .expect("nearest do jogador");
        assert_eq!(jogador.get::<String>("name").expect("name"), "torre");
    }

    #[test]
    fn bridge_sem_posicoes_devolve_nil() {
        let (lua, _game, state) = lua_com_bridge();
        state.set(TravelSnapshot::default());
        assert!(
            lua.load("return viber.vael.travel_nearest() == nil")
                .eval::<bool>()
                .expect("sem jogador")
        );
        assert!(
            lua.load("return viber.vael.travel_nearest(0, 0) == nil")
                .eval::<bool>()
                .expect("sem posições")
        );
    }

    #[test]
    fn bridge_revela_e_le_a_chave_do_viber_game() {
        let (lua, game, _state) = lua_com_bridge();
        assert!(
            !lua.load("return viber.vael.travel_is_revealed('desert')")
                .eval::<bool>()
                .expect("por revelar")
        );
        assert!(
            lua.load("return viber.vael.travel_reveal('desert')")
                .eval::<bool>()
                .expect("revelar")
        );
        assert!(matches!(
            game.raw_get::<Value>("biome.desert").expect("chave"),
            Value::Boolean(true)
        ));
        assert!(
            lua.load("return viber.vael.travel_is_revealed('desert')")
                .eval::<bool>()
                .expect("revelado")
        );
        assert!(
            lua.load("return viber.vael.travel_reveal('') == false")
                .eval::<bool>()
                .expect("vazio")
        );
        assert!(
            lua.load("return viber.vael.travel_is_revealed(nil) == false")
                .eval::<bool>()
                .expect("nil")
        );
    }

    #[test]
    fn bridge_etiquetas_e_tabela_existente() {
        let (lua, _game, _state) = lua_com_bridge();
        let label: String = lua
            .load("return viber.vael.travel_label('torre')")
            .eval()
            .expect("label");
        assert_eq!(label, "Torre");
        assert!(
            lua.load("return viber.vael.travel_label('desconhecido') == 'desconhecido'")
                .eval::<bool>()
                .expect("fallback")
        );
        assert!(
            lua.load("return viber.vael.travel_label(nil) == nil")
                .eval::<bool>()
                .expect("nil")
        );

        // Instalar por cima de uma tabela existente não apaga as chaves.
        let lua = Lua::new();
        lua.globals()
            .set("viber", lua.create_table().expect("viber"))
            .expect("global");
        let game = lua.create_table().expect("jogo");
        lua.set_named_registry_value("viber_game", game)
            .expect("registar");
        lua.load("viber.vael = { vault_add = function() return 7 end }")
            .exec()
            .expect("tabela antiga");
        install(&lua, Arc::new(regras()), &TravelState::default()).expect("instalar");
        let valor: i64 = lua
            .load("return viber.vael.vault_add()")
            .eval()
            .expect("vault_add intacto");
        assert_eq!(valor, 7);
        assert!(
            lua.load("return type(viber.vael.travel_landmarks) == 'function'")
                .eval::<bool>()
                .expect("travel_landmarks")
        );
    }

    // ── gate ────────────────────────────────────────────────────────────

    #[test]
    fn gate_por_variavel_de_ambiente() {
        assert!(!rust_enabled(None));
        assert!(!rust_enabled(Some("")));
        assert!(!rust_enabled(Some("0")));
        for on in ["1", "true", "on", "yes", " TRUE ", "On"] {
            assert!(rust_enabled(Some(on)), "devia ligar: {on}");
        }
        for off in ["0", "false", "off", "no", "OFF", " False "] {
            assert!(!rust_enabled(Some(off)), "devia desligar: {off}");
        }
        assert!(!VaelTravelConfig::from_raw(None).rust);
        assert!(VaelTravelConfig::from_raw(Some("1")).rust);
    }

    #[test]
    fn default_le_a_variavel_de_ambiente() {
        let raw = std::env::var(RUST_TRAVEL_ENV).ok();
        assert_eq!(
            VaelTravelConfig::default().rust,
            rust_enabled(raw.as_deref()),
            "o default tem de ler VAEL_TRAVEL_RUST"
        );
        if std::env::var_os(RUST_TRAVEL_ENV).is_none() {
            assert!(
                !VaelTravelConfig::default().rust,
                "sem a variável a viagem fica no Lua"
            );
        }
    }
}

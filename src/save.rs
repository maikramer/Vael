//! Save do Vael.
//!
//! O save da engine é um envelope: pose, cena, volumes e `world_kv` (o
//! `viber.game()` do Lua) mais uma secção `game` opaca. O Vael usa as duas:
//! **`world_kv` continua a ser o espelho plano** que os scripts leem durante
//! a transição, e `game.vael` guarda o estado tipado (a fonte de verdade
//! quando as regras passarem para Rust).
//!
//! [`VaelSave::from_world_kv`] migra os saves antigos (chaves planas do
//! `rules.lua`, `quest_states`/`skill_learned` de versões anteriores) sem
//! perder nada: tudo o que não é reconhecido fica em `legacy`.

use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::boat::BoatPose;
use crate::compat::{json_bool, json_f32, json_i64};
use crate::components::{Inventory, PlayerVitals, QuestLog, XpState};

/// Versão do schema tipado [`VaelSave`].
pub const VAEL_SAVE_VERSION: u32 = 1;

/// Equipamento: `gear.weapon`, `gear.armor` e `gear.charm` no Lua.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, Reflect)]
#[serde(default, rename_all = "snake_case")]
pub struct Equipment {
    pub weapon: Option<String>,
    pub armor: Option<String>,
    pub charm: Option<String>,
}

impl Equipment {
    fn entries(&self) -> [(&'static str, &Option<String>); 3] {
        [
            ("weapon", &self.weapon),
            ("armor", &self.armor),
            ("charm", &self.charm),
        ]
    }
}

/// Marcas de história (`story_prologo`/`story_epilogo` no Lua).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, Reflect)]
#[serde(default, rename_all = "snake_case")]
pub struct StoryFlags {
    pub prologue: bool,
    pub epilogue: bool,
}

/// Estado completo do jogador. Serializa com os desconhecidos achatados em
/// `legacy`, por isso um save escrito por uma versão futura não perde campos.
#[derive(Debug, Clone, PartialEq, Resource, Serialize, Deserialize, Reflect)]
#[serde(default, rename_all = "snake_case")]
#[reflect(Resource)]
pub struct VaelSave {
    pub version: u32,
    pub vitals: PlayerVitals,
    pub progression: XpState,
    pub inventory: Inventory,
    pub quests: QuestLog,
    pub equipment: Equipment,
    pub story: StoryFlags,
    pub harvested: BTreeSet<String>,
    pub markers: BTreeSet<String>,
    pub spawn: Option<[f32; 3]>,
    /// Poses dos barcos do `main` ([`crate::boat::BoatStates`]) e o barco em
    /// que o herói seguia. Os barcos são despawnados a cada `SceneLoad`, por
    /// isso a pose tem de viajar no save: sem ela um save a bordo acordava o
    /// herói na coordenada do barco e o barco no sítio autorado.
    ///
    /// Vive no JSON tipado (e não no payload refletido do `save_native`) de
    /// propósito: o `#[serde(default)]` deixa um save antigo (sem os campos)
    /// carregar, enquanto um campo novo no payload refletido fazia falhar o
    /// `from_reflect` do moonshine e perdia o save INTEIRO.
    #[serde(default)]
    pub boats: Vec<BoatPose>,
    /// Nome do barco em que o herói seguia (`None` = a pé).
    #[serde(default)]
    pub riding_boat: Option<String>,
    /// Chaves que o schema tipado não conhece (progresso `q.<id>.n`,
    /// `dialogue.*`, `riddle.*`, `lang`, …). Nunca é descartado.
    #[serde(flatten, default)]
    #[reflect(ignore)]
    pub legacy: BTreeMap<String, Value>,
}

impl Default for VaelSave {
    fn default() -> Self {
        Self {
            version: VAEL_SAVE_VERSION,
            vitals: PlayerVitals::default(),
            progression: XpState::default(),
            inventory: Inventory::default(),
            quests: QuestLog::default(),
            equipment: Equipment::default(),
            story: StoryFlags::default(),
            harvested: BTreeSet::new(),
            markers: BTreeSet::new(),
            spawn: None,
            boats: Vec::new(),
            riding_boat: None,
            legacy: BTreeMap::new(),
        }
    }
}

impl VaelSave {
    /// Migra o `world_kv` plano do `viber.game()` para o schema tipado.
    ///
    /// As chaves conhecidas do `rules.lua` entram nos campos; todas as
    /// outras — incluindo progresso de quests (`q.<id>.n`, `.stage`, `.v.*`)
    /// — ficam em `legacy` e voltam no [`Self::to_world_kv`].
    pub fn from_world_kv(kv: &BTreeMap<String, Value>) -> Self {
        let mut save = Self::default();
        let mut spawn = [None::<f32>; 3];
        let mut spawn_antigo = None;
        let mut level_visto = false;
        let mut menu_level = None;
        let mut gold_visto = false;
        let mut menu_gold = None;

        for (raw_key, value) in kv {
            let key = raw_key.as_str();
            match key {
                "hp" => {
                    if let Some(n) = json_f32(value) {
                        save.vitals.hp = n;
                    }
                    continue;
                }
                "max_hp" => {
                    if let Some(n) = json_f32(value) {
                        save.vitals.max_hp = n;
                    }
                    continue;
                }
                "xp" => {
                    if let Some(n) = json_i64(value) {
                        save.progression.xp = n;
                    }
                    continue;
                }
                "xp_next" => {
                    if let Some(n) = json_i64(value) {
                        save.progression.xp_next = n;
                    }
                    continue;
                }
                "level" => {
                    if let Some(n) = json_i64(value) {
                        save.progression.level = n;
                        level_visto = true;
                    }
                    continue;
                }
                "skill_points" => {
                    if let Some(n) = json_i64(value) {
                        save.progression.skill_points = n;
                    }
                    continue;
                }
                "save_level" => {
                    menu_level = json_i64(value);
                    continue;
                }
                "save_gold" => {
                    menu_gold = json_i64(value);
                    continue;
                }
                "tracked_quest" => {
                    if let Some(id) = value.as_str() {
                        save.quests.tracked = Some(id.to_string());
                    } else if !value.is_null() {
                        save.legacy.insert(key.to_string(), value.clone());
                    }
                    continue;
                }
                "spawn_x" => {
                    spawn[0] = json_f32(value);
                    if spawn[0].is_none() {
                        keep_raw(&mut save.legacy, key, value);
                    }
                    continue;
                }
                "spawn_y" => {
                    spawn[1] = json_f32(value);
                    if spawn[1].is_none() {
                        keep_raw(&mut save.legacy, key, value);
                    }
                    continue;
                }
                "spawn_z" => {
                    spawn[2] = json_f32(value);
                    if spawn[2].is_none() {
                        keep_raw(&mut save.legacy, key, value);
                    }
                    continue;
                }
                "spawn_pos" => {
                    spawn_antigo = parse_spawn(value);
                    if spawn_antigo.is_none() && !value.is_null() {
                        keep_raw(&mut save.legacy, key, value);
                    }
                    continue;
                }
                "story_prologo" => {
                    save.story.prologue = json_bool(value).unwrap_or(false);
                    continue;
                }
                "story_epilogo" => {
                    save.story.epilogue = json_bool(value).unwrap_or(false);
                    continue;
                }
                "quest_states" => {
                    if let Some(states) = parse_states(value) {
                        for (id, state) in states {
                            if state == "done" {
                                save.quests.done.push(id.clone());
                            }
                            save.quests.states.insert(id, state);
                        }
                    } else {
                        save.legacy.insert(key.to_string(), value.clone());
                    }
                    continue;
                }
                "skill_learned" => {
                    if let Some(ids) = parse_skill_list(value) {
                        save.progression.skills.extend(ids);
                    } else {
                        save.legacy.insert(key.to_string(), value.clone());
                    }
                    continue;
                }
                "equipment" => {
                    if let Some(obj) = value.as_object() {
                        save.equipment.weapon =
                            obj.get("weapon").and_then(Value::as_str).map(str::to_owned);
                        save.equipment.armor =
                            obj.get("armor").and_then(Value::as_str).map(str::to_owned);
                        save.equipment.charm =
                            obj.get("charm").and_then(Value::as_str).map(str::to_owned);
                    } else if !value.is_null() {
                        save.legacy.insert(key.to_string(), value.clone());
                    }
                    continue;
                }
                "harvested" => {
                    if let Some(names) = truthy_names(value) {
                        save.harvested.extend(names);
                    } else {
                        save.legacy.insert(key.to_string(), value.clone());
                    }
                    continue;
                }
                "markers" => {
                    if let Some(names) = truthy_names(value) {
                        save.markers.extend(names);
                    } else {
                        save.legacy.insert(key.to_string(), value.clone());
                    }
                    continue;
                }
                _ => {}
            }

            if let Some(id) = key.strip_prefix("item.") {
                if let Some(n) = json_i64(value) {
                    if id == "gold" {
                        gold_visto = true;
                    }
                    save.inventory.set(id, n);
                } else {
                    save.legacy.insert(key.to_string(), value.clone());
                }
                continue;
            }
            if let Some(id) = key.strip_prefix("skill.") {
                match json_bool(value) {
                    Some(true) => {
                        save.progression.skills.insert(id.to_string());
                    }
                    Some(false) => {}
                    None => {
                        save.legacy.insert(key.to_string(), value.clone());
                    }
                }
                continue;
            }
            if let Some(rest) = key.strip_prefix("q.") {
                // `q.<id>` é o estado; `q.<id>.n`/`.stage`/`.v.*` são
                // progresso e ficam em legacy.
                if !rest.contains('.')
                    && let Some(state) = value.as_str()
                {
                    if state == "done" {
                        save.quests.done.push(rest.to_string());
                    }
                    save.quests
                        .states
                        .insert(rest.to_string(), state.to_string());
                    continue;
                }
                save.legacy.insert(key.to_string(), value.clone());
                continue;
            }
            if let Some(slot) = key.strip_prefix("gear.") {
                if let Some(id) = value.as_str() {
                    match slot {
                        "weapon" => save.equipment.weapon = Some(id.to_string()),
                        "armor" => save.equipment.armor = Some(id.to_string()),
                        "charm" => save.equipment.charm = Some(id.to_string()),
                        _ => {
                            save.legacy.insert(key.to_string(), value.clone());
                        }
                    }
                } else if !value.is_null() {
                    save.legacy.insert(key.to_string(), value.clone());
                }
                continue;
            }
            if let Some(name) = key.strip_prefix("harvested.") {
                match json_bool(value) {
                    Some(true) => {
                        save.harvested.insert(name.to_string());
                    }
                    Some(false) => {}
                    None => {
                        save.legacy.insert(key.to_string(), value.clone());
                    }
                }
                continue;
            }
            if let Some(name) = key.strip_prefix("mark.") {
                match json_bool(value) {
                    Some(true) => {
                        save.markers.insert(name.to_string());
                    }
                    Some(false) => {}
                    None => {
                        save.legacy.insert(key.to_string(), value.clone());
                    }
                }
                continue;
            }

            save.legacy.insert(key.to_string(), value.clone());
        }

        // `save_level`/`save_gold` são o espelho do menu; só valem quando a
        // chave principal do save não existe.
        if !level_visto && let Some(level) = menu_level {
            save.progression.level = level;
        }
        if !gold_visto && let Some(gold) = menu_gold {
            save.inventory.gold = gold;
        }

        if let [Some(x), Some(y), Some(z)] = spawn {
            save.spawn = Some([x, y, z]);
        } else if let Some(pos) = spawn_antigo {
            save.spawn = Some(pos);
        }

        save.quests.done.sort();
        save.quests.done.dedup();
        save
    }

    /// Aplica um snapshot do `viber.game()` **sem apagar** o que o Rust
    /// possui e o KV não trouxe.
    ///
    /// O snapshot de 10 Hz é normalmente completo, mas há campos que o Rust
    /// escreve à frente do espelho Lua (`spawn`, colheitas, marcadores) e
    /// chaves só-Rust em `legacy`. Substituir o `VaelSave` inteiro perdia-os
    /// até ao próximo `apply_patch`; o merge mantém-nos.
    pub fn merge_world_kv(&mut self, kv: &BTreeMap<String, Value>) {
        if kv.is_empty() {
            return;
        }
        let incoming = Self::from_world_kv(kv);
        let has = |key: &str| kv.contains_key(key);
        let has_prefix = |prefix: &str| kv.keys().any(|key| key.starts_with(prefix));
        // Marca de "snapshot do store vivo": são as chaves que o `rules.lua`
        // semeia no arranque. Quando estão lá, a AUSÊNCIA de uma secção é um
        // estado (equipamento removido) e não uma lacuna de um snapshot
        // parcial.
        let live = has("hp") || has("max_hp") || has("level") || has("xp");

        if has("hp") {
            self.vitals.hp = incoming.vitals.hp;
        }
        if has("max_hp") {
            self.vitals.max_hp = incoming.vitals.max_hp;
        }
        if has("xp") {
            self.progression.xp = incoming.progression.xp;
        }
        if has("xp_next") {
            self.progression.xp_next = incoming.progression.xp_next;
        }
        if has("level") {
            self.progression.level = incoming.progression.level;
        }
        if has("skill_points") {
            self.progression.skill_points = incoming.progression.skill_points;
        }
        if has("skill_learned") || has_prefix("skill.") {
            self.progression.skills = incoming.progression.skills.clone();
        }
        if has_prefix("item.") || has("save_gold") {
            self.inventory = incoming.inventory.clone();
        }
        // `q.<id>` cobre o destracking: o diário limpa `tracked_quest` mas a
        // quest fica com o seu estado, por isso o snapshot traz a chave `q.`.
        if has("quest_states") || has_prefix("q.") {
            self.quests = incoming.quests.clone();
        }
        // O equipamento NÃO é só-Rust: desequipar a última peça apaga
        // `gear.<slot>` do Lua, por isso a ausência das chaves também tem de
        // chegar ao tipado (senão o painel continuava a mostrar a peça que
        // saiu até ao próximo load).
        if has("equipment") || has_prefix("gear.") || live {
            self.equipment = incoming.equipment.clone();
        }
        if has("story_prologo") {
            self.story.prologue = incoming.story.prologue;
        }
        if has("story_epilogo") {
            self.story.epilogue = incoming.story.epilogue;
        }
        // O spawn só é substituído quando o snapshot traz os TRÊS eixos (o
        // parser exige-os): um KV parcial (`spawn_x` sem y/z) não pode
        // apagar o ponto de descanso que o Rust já tem.
        if let Some(spawn) = incoming.spawn {
            self.spawn = Some(spawn);
        }
        // Colheitas/marcadores são monotónicos e o Rust é o dono; o KV é
        // espelho. A união nunca desfaz uma colheita feita neste frame.
        self.harvested.extend(incoming.harvested.iter().cloned());
        self.markers.extend(incoming.markers.iter().cloned());

        // O que o Rust possui e o snapshot não trouxe fica: inclui chaves com
        // forma tipada que o schema não leu (estão em `legacy` e o
        // `to_world_kv` repõe-nas; descartá-las aqui perdia-as no merge).
        let mut legacy = incoming.legacy;
        for (key, value) in &self.legacy {
            if !kv.contains_key(key) {
                legacy.entry(key.clone()).or_insert_with(|| value.clone());
            }
        }
        self.legacy = legacy;
    }

    /// Espelho plano compatível com o `viber.game()` do Lua.
    pub fn to_world_kv(&self) -> BTreeMap<String, Value> {
        let mut kv = BTreeMap::new();

        // Legacy primeiro, SEM filtrar pela forma da chave: um valor tipado
        // que o schema não conseguiu ler (`item.x` textual, `gear.slot`
        // numérico, `tracked_quest` não-string, `spawn_*` textual, …) fica em
        // `legacy` e tem de voltar — filtrá-lo aqui perdia-o no round-trip.
        // As escritas dos campos tipados vêm depois e sobrepõem-se, por isso
        // uma chave regenerável nunca ressuscita com o valor velho.
        for (key, value) in &self.legacy {
            kv.insert(key.clone(), value.clone());
        }

        kv.insert("hp".into(), json!(self.vitals.hp));
        kv.insert("max_hp".into(), json!(self.vitals.max_hp));
        kv.insert("xp".into(), json!(self.progression.xp));
        kv.insert("xp_next".into(), json!(self.progression.xp_next));
        kv.insert("level".into(), json!(self.progression.level));
        kv.insert("skill_points".into(), json!(self.progression.skill_points));

        for (id, n) in self.inventory.entries() {
            kv.insert(format!("item.{id}"), json!(n));
        }
        for id in &self.progression.skills {
            kv.insert(format!("skill.{id}"), json!(true));
        }
        for (id, state) in &self.quests.states {
            kv.insert(format!("q.{id}"), json!(state));
        }
        for id in &self.quests.done {
            kv.entry(format!("q.{id}")).or_insert_with(|| json!("done"));
        }
        if let Some(tracked) = &self.quests.tracked {
            kv.insert("tracked_quest".into(), json!(tracked));
        }
        for (slot, id) in self.equipment.entries() {
            if let Some(id) = id {
                kv.insert(format!("gear.{slot}"), json!(id));
            }
        }
        for name in &self.harvested {
            kv.insert(format!("harvested.{name}"), json!(true));
        }
        for name in &self.markers {
            kv.insert(format!("mark.{name}"), json!(true));
        }
        if let Some([x, y, z]) = self.spawn {
            kv.insert("spawn_x".into(), json!(x));
            kv.insert("spawn_y".into(), json!(y));
            kv.insert("spawn_z".into(), json!(z));
        }
        kv.insert("story_prologo".into(), json!(self.story.prologue));
        kv.insert("story_epilogo".into(), json!(self.story.epilogue));

        // O menu de saves da engine lê estes dois do `world_kv`.
        kv.insert("save_level".into(), json!(self.progression.level));
        kv.insert("save_gold".into(), json!(self.inventory.gold));

        kv
    }

    /// Lê o estado tipado de um [`bevy_ext_save::SaveGame`]: a secção
    /// `game.vael` manda; sem ela (save antigo), migra o `world_kv`.
    pub fn from_save_game(save: &bevy_ext_save::SaveGame) -> Self {
        if let Some(typed) = save.game.as_ref().and_then(|game| game.get("vael")) {
            match serde_json::from_value::<Self>(typed.clone()) {
                Ok(parsed) => return parsed,
                Err(err) => {
                    error!("vael: game.vael inválido ({err}); a migrar world_kv");
                }
            }
        }
        Self::from_world_kv(&save.world_kv)
    }

    /// Escreve o envelope: `world_kv` (transição Lua) + `game.vael`
    /// (schema tipado). Não toca no resto do save da engine.
    pub fn write_to_save_game(&self, save: &mut bevy_ext_save::SaveGame) {
        save.world_kv = self.to_world_kv();
        let value = match serde_json::to_value(self) {
            Ok(value) => value,
            Err(err) => {
                error!("vael: save não serializa: {err}");
                return;
            }
        };
        let game = save
            .game
            .get_or_insert_with(|| Value::Object(Default::default()));
        if !game.is_object() {
            *game = Value::Object(Default::default());
        }
        game["vael"] = value;
    }
}

/// Plugin do save do Vael: recurso tipado. O disco é do
/// [`crate::save_native`] (`moonshine-save`); este plugin só garante o
/// `VaelSave` e os tipos registados.
pub struct VaelSavePlugin;

impl Plugin for VaelSavePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VaelSave>()
            .register_type::<VaelSave>()
            .register_type::<Equipment>()
            .register_type::<StoryFlags>()
            .register_type::<crate::boat::BoatPose>()
            .register_type::<PlayerVitals>()
            .register_type::<Inventory>()
            .register_type::<XpState>()
            .register_type::<QuestLog>();
    }
}

// ── conversões defensivas ───────────────────────────────────────────────
// Os leitores tolerantes (`as_i64`/`as_f32`/`as_bool`) vivem no
// `crate::compat` (o JSON legacy e o `viber_game` partilham as mesmas
// tolerâncias do Lua); aqui ficam só os parsers de estruturas do save.

fn parse_spawn(value: &Value) -> Option<[f32; 3]> {
    let obj = value.as_object()?;
    Some([
        json_f32(obj.get("x")?)?,
        json_f32(obj.get("y")?)?,
        json_f32(obj.get("z")?)?,
    ])
}

/// `quest_states` é um objecto `id → estado`, por vezes serializado como
/// string JSON. Qualquer entrada que não seja estado invalida a migração
/// (a chave inteira fica em `legacy`).
fn parse_states(value: &Value) -> Option<Vec<(String, String)>> {
    let map = match value {
        Value::Object(map) => map.clone(),
        Value::String(text) => match serde_json::from_str::<Value>(text) {
            Ok(Value::Object(map)) => map,
            _ => return None,
        },
        _ => return None,
    };
    let mut out = Vec::with_capacity(map.len());
    for (id, raw) in map {
        out.push((id, raw.as_str()?.to_string()));
    }
    Some(out)
}

/// `skill_learned`: lista de ids, objecto `id → true` ou lista numa string
/// JSON.
fn parse_skill_list(value: &Value) -> Option<Vec<String>> {
    match value {
        Value::Array(items) => {
            let ids: Option<Vec<&str>> = items.iter().map(Value::as_str).collect();
            Some(ids?.into_iter().map(str::to_owned).collect())
        }
        Value::Object(map) => Some(
            map.iter()
                .filter(|(_, raw)| json_bool(raw) == Some(true))
                .map(|(id, _)| id.clone())
                .collect(),
        ),
        Value::String(text) => {
            if let Ok(parsed) = serde_json::from_str::<Value>(text)
                && !parsed.is_string()
            {
                return parse_skill_list(&parsed);
            }
            Some(
                text.split(',')
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned)
                    .collect(),
            )
        }
        _ => None,
    }
}

/// Lista de nomes (`["a", "b"]`) ou objecto `nome → true`.
fn truthy_names(value: &Value) -> Option<Vec<String>> {
    match value {
        Value::Array(items) => {
            let names: Option<Vec<&str>> = items.iter().map(Value::as_str).collect();
            Some(names?.into_iter().map(str::to_owned).collect())
        }
        Value::Object(map) => Some(
            map.iter()
                .filter(|(_, raw)| json_bool(raw) == Some(true))
                .map(|(name, _)| name.clone())
                .collect(),
        ),
        _ => None,
    }
}

/// Guarda em `legacy` o valor de uma chave tipada que o schema não conseguiu
/// ler: o [`VaelSave::to_world_kv`] volta a escrevê-la e o campo tipado (se o
/// souber representar) sobrepõe-se no output.
fn keep_raw(legacy: &mut BTreeMap<String, Value>, key: &str, value: &Value) {
    legacy.insert(key.to_string(), value.clone());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kv_antigo() -> BTreeMap<String, Value> {
        let mut kv = BTreeMap::new();
        kv.insert("hp".into(), json!(73));
        kv.insert("max_hp".into(), json!(100));
        kv.insert("xp".into(), json!(20));
        kv.insert("xp_next".into(), json!(150));
        kv.insert("level".into(), json!(4));
        kv.insert("skill_points".into(), json!(2));
        kv.insert("item.gold".into(), json!(12));
        kv.insert("item.wood".into(), json!(7));
        kv.insert("item.bog-moss".into(), json!(3));
        kv.insert(
            "quest_states".into(),
            json!({"story_notas": "active", "city_intro": "done"}),
        );
        kv.insert("skill_learned".into(), json!(["vitality1", "strength1"]));
        kv.insert("skill.vitality1".into(), json!(true));
        kv.insert("q.story_notas.n".into(), json!(2));
        kv.insert("q.story_notas.need".into(), json!(4));
        kv.insert("spawn_x".into(), json!(1.5));
        kv.insert("spawn_y".into(), json!(2.5));
        kv.insert("spawn_z".into(), json!(3.5));
        kv.insert("story_prologo".into(), json!(true));
        kv.insert("harvested.old_tree".into(), json!(true));
        kv.insert("mark.atalaia-do-vale".into(), json!(true));
        kv.insert("gear.weapon".into(), json!("iron_axe"));
        kv.insert("tracked_quest".into(), json!("story_notas"));
        kv.insert("mystery".into(), json!({"a": 1}));
        kv
    }

    #[test]
    fn migra_world_kv_antigo() {
        let save = VaelSave::from_world_kv(&kv_antigo());

        assert_eq!(save.vitals.hp, 73.0);
        assert_eq!(save.vitals.max_hp, 100.0);
        assert_eq!(save.inventory.gold, 12);
        assert_eq!(save.inventory.wood, 7);
        assert_eq!(save.inventory.get("bog-moss"), 3);
        assert_eq!(save.progression.xp, 20);
        assert_eq!(save.progression.xp_next, 150);
        assert_eq!(save.progression.level, 4);
        assert_eq!(save.progression.skill_points, 2);
        assert!(save.progression.skills.contains("vitality1"));
        assert!(save.progression.skills.contains("strength1"));

        assert_eq!(
            save.quests.states.get("story_notas"),
            Some(&"active".to_string())
        );
        assert_eq!(
            save.quests.states.get("city_intro"),
            Some(&"done".to_string())
        );
        assert!(save.quests.done.contains(&"city_intro".to_string()));
        assert_eq!(save.quests.tracked.as_deref(), Some("story_notas"));

        assert_eq!(save.spawn, Some([1.5, 2.5, 3.5]));
        assert!(save.story.prologue);
        assert!(save.harvested.contains("old_tree"));
        assert!(save.markers.contains("atalaia-do-vale"));
        assert_eq!(save.equipment.weapon.as_deref(), Some("iron_axe"));

        // Progresso fino não é reconhecido pelo schema: viaja em legacy.
        assert_eq!(save.legacy.get("q.story_notas.n"), Some(&json!(2)));
        assert_eq!(save.legacy.get("q.story_notas.need"), Some(&json!(4)));
        assert_eq!(save.legacy.get("mystery"), Some(&json!({"a": 1})));
    }

    #[test]
    fn espelho_lua_volta_a_ter_as_chaves() {
        let save = VaelSave::from_world_kv(&kv_antigo());
        let kv = save.to_world_kv();

        assert_eq!(kv.get("hp"), Some(&json!(73.0)));
        assert_eq!(kv.get("item.gold"), Some(&json!(12)));
        assert_eq!(kv.get("item.bog-moss"), Some(&json!(3)));
        assert_eq!(kv.get("q.story_notas"), Some(&json!("active")));
        assert_eq!(kv.get("q.city_intro"), Some(&json!("done")));
        assert_eq!(kv.get("skill.vitality1"), Some(&json!(true)));
        assert_eq!(kv.get("gear.weapon"), Some(&json!("iron_axe")));
        assert_eq!(kv.get("tracked_quest"), Some(&json!("story_notas")));
        assert_eq!(kv.get("spawn_x"), Some(&json!(1.5)));
        assert_eq!(kv.get("save_level"), Some(&json!(4)));
        assert_eq!(kv.get("save_gold"), Some(&json!(12)));

        // Nada perdido: progresso e desconhecidos continuam lá.
        assert_eq!(kv.get("q.story_notas.n"), Some(&json!(2)));
        assert_eq!(kv.get("q.story_notas.need"), Some(&json!(4)));
        assert_eq!(kv.get("mystery"), Some(&json!({"a": 1})));
    }

    #[test]
    fn serde_round_trip() {
        let save = VaelSave::from_world_kv(&kv_antigo());
        let json = serde_json::to_string(&save).expect("serializa");
        let back: VaelSave = serde_json::from_str(&json).expect("deserializa");
        assert_eq!(save, back);
    }

    #[test]
    fn desconhecidos_sobrevivem_em_legacy() {
        let mut kv = BTreeMap::new();
        kv.insert("hp".into(), json!(50));
        kv.insert("flag_futura".into(), json!("x"));
        kv.insert("tabela_futura".into(), json!([1, 2, 3]));

        let save = VaelSave::from_world_kv(&kv);
        assert_eq!(save.legacy.get("flag_futura"), Some(&json!("x")));
        assert_eq!(save.legacy.get("tabela_futura"), Some(&json!([1, 2, 3])));

        let back = VaelSave::from_world_kv(&save.to_world_kv());
        assert_eq!(back.legacy.get("flag_futura"), Some(&json!("x")));
        assert_eq!(back.legacy.get("tabela_futura"), Some(&json!([1, 2, 3])));
    }

    #[test]
    fn save_game_ida_e_volta() {
        let save = VaelSave::from_world_kv(&kv_antigo());
        let mut envelope = bevy_ext_save::SaveGame::default();
        save.write_to_save_game(&mut envelope);

        assert!(envelope.game.as_ref().and_then(|g| g.get("vael")).is_some());
        assert_eq!(VaelSave::from_save_game(&envelope), save);
    }

    #[test]
    fn merge_preserva_campos_rust_ausentes_do_kv() {
        let mut save = VaelSave::default();
        save.vitals.hp = 90.0;
        save.vitals.max_hp = 140.0;
        save.spawn = Some([1.0, 2.0, 3.0]);
        save.harvested.insert("old_tree".into());
        save.markers.insert("atalaia-do-vale".into());
        save.legacy.insert("vael_god".into(), json!(true));

        let mut kv = BTreeMap::new();
        kv.insert("hp".into(), json!(50));

        save.merge_world_kv(&kv);

        assert_eq!(save.vitals.hp, 50.0);
        assert_eq!(save.vitals.max_hp, 140.0, "max_hp não veio no KV");
        assert_eq!(save.spawn, Some([1.0, 2.0, 3.0]));
        assert!(save.harvested.contains("old_tree"));
        assert!(save.markers.contains("atalaia-do-vale"));
        assert_eq!(save.legacy.get("vael_god"), Some(&json!(true)));
    }

    #[test]
    fn merge_snapshot_completo_sobrepoe_e_une_monotonicos() {
        let mut save = VaelSave::default();
        save.spawn = Some([1.0, 2.0, 3.0]);
        save.markers.insert("antigo".into());
        save.harvested.insert("old_tree".into());

        let mut kv = kv_antigo();
        kv.insert("spawn_x".into(), json!(9.0));
        kv.insert("spawn_y".into(), json!(8.0));
        kv.insert("spawn_z".into(), json!(7.0));
        kv.insert("mark.novo".into(), json!(true));
        kv.insert("harvested.new_tree".into(), json!(true));

        save.merge_world_kv(&kv);

        assert_eq!(save.spawn, Some([9.0, 8.0, 7.0]));
        assert!(save.markers.contains("antigo"));
        assert!(save.markers.contains("novo"));
        assert!(save.harvested.contains("old_tree"));
        assert!(save.harvested.contains("new_tree"));
        assert_eq!(save.vitals.hp, 73.0);
    }

    /// Desequipar a última peça apaga `gear.<slot>` do Lua: a ausência das
    /// chaves num snapshot vivo também é um estado.
    #[test]
    fn merge_limpa_o_equipamento_quando_nao_ha_gear_no_snapshot() {
        let mut save = VaelSave::default();
        save.equipment.weapon = Some("iron_axe".into());

        let mut kv = BTreeMap::new();
        kv.insert("hp".into(), json!(50));
        kv.insert("item.gold".into(), json!(1));

        save.merge_world_kv(&kv);

        assert_eq!(save.equipment.weapon, None);
    }

    /// Um KV com só `spawn_x` não pode apagar o ponto de descanso guardado.
    #[test]
    fn merge_nao_apaga_o_spawn_com_um_snapshot_parcial() {
        let mut save = VaelSave::default();
        save.spawn = Some([1.0, 2.0, 3.0]);

        let mut kv = BTreeMap::new();
        kv.insert("hp".into(), json!(50));
        kv.insert("spawn_x".into(), json!(9.0));

        save.merge_world_kv(&kv);

        assert_eq!(save.spawn, Some([1.0, 2.0, 3.0]));
    }

    #[test]
    fn aceita_numeros_fracionarios_e_strings() {
        let mut kv = BTreeMap::new();
        kv.insert("hp".into(), json!(73.8));
        kv.insert("level".into(), json!(4.0));
        kv.insert("item.gold".into(), json!("9"));

        let save = VaelSave::from_world_kv(&kv);
        assert_eq!(save.vitals.hp, 73.8);
        assert_eq!(save.progression.level, 4);
        assert_eq!(save.inventory.gold, 9);
    }

    /// Um valor de chave TIPADA que o schema não consegue ler não pode
    /// desaparecer: fica em `legacy` e volta ao espelho `world_kv` (os campos
    /// tipados só se sobrepõem quando têm um valor para escrever).
    #[test]
    fn chaves_tipadas_ilegiveis_sobrevivem_ao_round_trip() {
        let mut kv = BTreeMap::new();
        kv.insert("hp".into(), json!(50));
        kv.insert("item.gold".into(), json!(10));
        kv.insert("item.bog-moss".into(), json!("três"));
        kv.insert("tracked_quest".into(), json!(42));
        kv.insert("gear.weapon".into(), json!(7));
        kv.insert("q.city_intro".into(), json!(["junk"]));
        kv.insert("skill.vitality1".into(), json!("yes"));
        kv.insert("harvested.old_tree".into(), json!("talvez"));
        kv.insert("spawn_x".into(), json!("longe"));
        kv.insert("spawn_pos".into(), json!({"x": 1.0}));

        let save = VaelSave::from_world_kv(&kv);
        assert_eq!(save.legacy.get("item.bog-moss"), Some(&json!("três")));
        assert_eq!(save.legacy.get("tracked_quest"), Some(&json!(42)));
        assert_eq!(save.legacy.get("gear.weapon"), Some(&json!(7)));
        assert_eq!(save.legacy.get("q.city_intro"), Some(&json!(["junk"])));
        assert_eq!(save.legacy.get("skill.vitality1"), Some(&json!("yes")));
        assert_eq!(save.legacy.get("harvested.old_tree"), Some(&json!("talvez")));
        assert_eq!(save.legacy.get("spawn_x"), Some(&json!("longe")));
        assert_eq!(save.legacy.get("spawn_pos"), Some(&json!({"x": 1.0})));

        let back = save.to_world_kv();
        // O espelho volta a tê-las (o `is_typed_key` já não as filtra).
        assert_eq!(back.get("item.bog-moss"), Some(&json!("três")));
        assert_eq!(back.get("tracked_quest"), Some(&json!(42)));
        assert_eq!(back.get("gear.weapon"), Some(&json!(7)));
        assert_eq!(back.get("q.city_intro"), Some(&json!(["junk"])));
        assert_eq!(back.get("skill.vitality1"), Some(&json!("yes")));
        assert_eq!(back.get("harvested.old_tree"), Some(&json!("talvez")));
        assert_eq!(back.get("spawn_x"), Some(&json!("longe")));
        assert_eq!(back.get("spawn_pos"), Some(&json!({"x": 1.0})));
        // O campo tipado ganha às chaves que ele próprio representa.
        assert_eq!(back.get("hp"), Some(&json!(50.0)));
        assert_eq!(back.get("item.gold"), Some(&json!(10)));

        // E o ciclo é estável: recarregar o espelho não muda nada.
        let again = VaelSave::from_world_kv(&back);
        assert_eq!(again.legacy, save.legacy);
        assert_eq!(again.to_world_kv(), back);
    }

    /// O merge não pode apagar o que o KV não traz (incluindo as chaves
    /// tipadas ilegíveis guardadas em `legacy`).
    #[test]
    fn merge_preserva_chaves_tipadas_ilegiveis() {
        let mut save = VaelSave::default();
        save.legacy.insert("item.bog-moss".into(), json!("três"));

        let mut kv = BTreeMap::new();
        kv.insert("hp".into(), json!(50));

        save.merge_world_kv(&kv);

        assert_eq!(save.legacy.get("item.bog-moss"), Some(&json!("três")));
        assert_eq!(save.to_world_kv().get("item.bog-moss"), Some(&json!("três")));
    }

    /// Os barcos viajam no JSON tipado: posição e yaw por barco, mais o barco
    /// em que o herói seguia. Um save antigo (sem os campos) carrega.
    #[test]
    fn barcos_sobrevivem_ao_round_trip_do_save() {
        let mut save = VaelSave::default();
        save.boats.push(BoatPose {
            name: "barco-a".into(),
            position: Vec3::new(1.0, 2.0, 3.0),
            yaw: 1.25,
        });
        save.boats.push(BoatPose {
            name: "barco-b".into(),
            position: Vec3::new(-4.0, 0.5, 9.0),
            yaw: -0.5,
        });
        save.riding_boat = Some("barco-a".into());

        // VaelSave → JSON (o `save_json` do payload).
        let text = serde_json::to_string(&save).expect("serializa");
        let back: VaelSave = serde_json::from_str(&text).expect("deserializa");
        assert_eq!(back, save);

        // E o envelope completo (o caminho do save_native).
        let mut envelope = bevy_ext_save::SaveGame::default();
        save.write_to_save_game(&mut envelope);
        assert_eq!(VaelSave::from_save_game(&envelope), save);

        // Save antigo: os campos novos ficam com o default.
        let antigo: VaelSave = serde_json::from_value(json!({
            "version": 1,
            "vitals": {"hp": 40.0, "max_hp": 100.0},
        }))
        .expect("save antigo");
        assert!(antigo.boats.is_empty());
        assert_eq!(antigo.riding_boat, None);
    }
}

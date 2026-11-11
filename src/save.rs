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
                    if let Some(n) = as_f32(value) {
                        save.vitals.hp = n;
                    }
                    continue;
                }
                "max_hp" => {
                    if let Some(n) = as_f32(value) {
                        save.vitals.max_hp = n;
                    }
                    continue;
                }
                "xp" => {
                    if let Some(n) = as_i64(value) {
                        save.progression.xp = n;
                    }
                    continue;
                }
                "xp_next" => {
                    if let Some(n) = as_i64(value) {
                        save.progression.xp_next = n;
                    }
                    continue;
                }
                "level" => {
                    if let Some(n) = as_i64(value) {
                        save.progression.level = n;
                        level_visto = true;
                    }
                    continue;
                }
                "skill_points" => {
                    if let Some(n) = as_i64(value) {
                        save.progression.skill_points = n;
                    }
                    continue;
                }
                "save_level" => {
                    menu_level = as_i64(value);
                    continue;
                }
                "save_gold" => {
                    menu_gold = as_i64(value);
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
                    spawn[0] = as_f32(value);
                    continue;
                }
                "spawn_y" => {
                    spawn[1] = as_f32(value);
                    continue;
                }
                "spawn_z" => {
                    spawn[2] = as_f32(value);
                    continue;
                }
                "spawn_pos" => {
                    spawn_antigo = parse_spawn(value);
                    continue;
                }
                "story_prologo" => {
                    save.story.prologue = as_bool(value).unwrap_or(false);
                    continue;
                }
                "story_epilogo" => {
                    save.story.epilogue = as_bool(value).unwrap_or(false);
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
                if let Some(n) = as_i64(value) {
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
                match as_bool(value) {
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
                if !rest.contains('.') {
                    if let Some(state) = value.as_str() {
                        if state == "done" {
                            save.quests.done.push(rest.to_string());
                        }
                        save.quests
                            .states
                            .insert(rest.to_string(), state.to_string());
                        continue;
                    }
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
                match as_bool(value) {
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
                match as_bool(value) {
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

    /// Espelho plano compatível com o `viber.game()` do Lua.
    pub fn to_world_kv(&self) -> BTreeMap<String, Value> {
        let mut kv = BTreeMap::new();

        // Legacy primeiro; as chaves representadas pelo schema tipado são
        // reescritas a partir dos campos (e as que deixaram de existir não
        // ressuscitam).
        for (key, value) in &self.legacy {
            if !is_typed_key(key) {
                kv.insert(key.clone(), value.clone());
            }
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

    /// Lê o estado tipado de um [`viber::save::SaveGame`]: a secção
    /// `game.vael` manda; sem ela (save antigo), migra o `world_kv`.
    pub fn from_save_game(save: &viber::save::SaveGame) -> Self {
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
    pub fn write_to_save_game(&self, save: &mut viber::save::SaveGame) {
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

/// Plugin do save do Vael: recurso tipado (a ponte com o disco continua a
/// ser a do [`viber::save::SavePlugin`]).
pub struct VaelSavePlugin;

impl Plugin for VaelSavePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VaelSave>()
            .register_type::<VaelSave>()
            .register_type::<Equipment>()
            .register_type::<StoryFlags>()
            .register_type::<PlayerVitals>()
            .register_type::<Inventory>()
            .register_type::<XpState>()
            .register_type::<QuestLog>();
    }
}

// ── conversões defensivas ───────────────────────────────────────────────

fn as_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f.floor() as i64)),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn as_f32(value: &Value) -> Option<f32> {
    match value {
        Value::Number(n) => n.as_f64().map(|f| f as f32),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn as_bool(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => Some(n.as_f64().is_some_and(|f| f != 0.0)),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn parse_spawn(value: &Value) -> Option<[f32; 3]> {
    let obj = value.as_object()?;
    Some([
        as_f32(obj.get("x")?)?,
        as_f32(obj.get("y")?)?,
        as_f32(obj.get("z")?)?,
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
                .filter(|(_, raw)| as_bool(raw) == Some(true))
                .map(|(id, _)| id.clone())
                .collect(),
        ),
        Value::String(text) => {
            if let Ok(parsed) = serde_json::from_str::<Value>(text) {
                if !parsed.is_string() {
                    return parse_skill_list(&parsed);
                }
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
                .filter(|(_, raw)| as_bool(raw) == Some(true))
                .map(|(name, _)| name.clone())
                .collect(),
        ),
        _ => None,
    }
}

/// Chave do `world_kv` cujo valor é reescrito a partir dos campos tipados.
fn is_typed_key(key: &str) -> bool {
    matches!(
        key,
        "hp" | "max_hp"
            | "xp"
            | "xp_next"
            | "level"
            | "skill_points"
            | "save_level"
            | "save_gold"
            | "tracked_quest"
            | "spawn_x"
            | "spawn_y"
            | "spawn_z"
            | "spawn_pos"
            | "story_prologo"
            | "story_epilogo"
    ) || key.starts_with("item.")
        || key.starts_with("skill.")
        || key.starts_with("gear.")
        || key.starts_with("harvested.")
        || key.starts_with("mark.")
        || key
            .strip_prefix("q.")
            .is_some_and(|rest| !rest.contains('.'))
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
        let mut envelope = viber::save::SaveGame::default();
        save.write_to_save_game(&mut envelope);

        assert!(envelope.game.as_ref().and_then(|g| g.get("vael")).is_some());
        assert_eq!(VaelSave::from_save_game(&envelope), save);
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
}

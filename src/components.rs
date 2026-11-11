//! Componentes e recursos partilhados do Vael.
//!
//! A fonte de verdade durante a transição é o [`crate::save::VaelSave`];
//! estes tipos são as secções que o save usa e que as ondas seguintes vão
//! espelhar nas entidades.

use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// Marca do herói do Vael. Com `gameplay: false` a engine não cria o
/// `viber::player::Player`; quem spawnar o herói (Wave 1) usa esta marca.
#[derive(Component, Debug, Clone, Copy, Default, Reflect)]
#[reflect(Component)]
pub struct VaelPlayer;

/// Vida do herói. Espelha `hp`/`max_hp` do `viber.game()`.
#[derive(Debug, Clone, PartialEq, Resource, Serialize, Deserialize, Reflect)]
#[serde(default, rename_all = "snake_case")]
#[reflect(Resource)]
pub struct PlayerVitals {
    pub hp: f32,
    pub max_hp: f32,
}

impl Default for PlayerVitals {
    /// Os defaults do `rules.lua`: 100/100.
    fn default() -> Self {
        Self {
            hp: 100.0,
            max_hp: 100.0,
        }
    }
}

/// Progressão: XP, nível e pontos de talento. Espelha `xp`, `xp_next`,
/// `level`, `skill_points` e os `skill.<id> = true` do Lua.
#[derive(Debug, Clone, PartialEq, Resource, Serialize, Deserialize, Reflect)]
#[serde(default, rename_all = "snake_case")]
#[reflect(Resource)]
pub struct XpState {
    pub xp: i64,
    pub xp_next: i64,
    pub level: i64,
    pub skill_points: i64,
    /// Talentos aprendidos.
    pub skills: BTreeSet<String>,
}

impl Default for XpState {
    /// `ensure_xp` do `rules.lua`: nível 1 e a primeira meta de 100 XP.
    fn default() -> Self {
        Self {
            xp: 0,
            xp_next: 100,
            level: 1,
            skill_points: 0,
            skills: BTreeSet::new(),
        }
    }
}

/// Cofre/inventário. Os campos fixos são os que o HUD, a loja e o HUD de
/// talentos usam; `items` guarda o resto (`item.<id>` no Lua).
#[derive(Debug, Clone, PartialEq, Default, Resource, Serialize, Deserialize, Reflect)]
#[serde(default, rename_all = "snake_case")]
#[reflect(Resource)]
pub struct Inventory {
    pub gold: i64,
    pub wood: i64,
    pub stone: i64,
    pub berry: i64,
    pub korok_seed: i64,
    pub potion: i64,
    pub antidote: i64,
    pub bomb: i64,
    pub moss_potion: i64,
    pub items: BTreeMap<String, i64>,
}

impl Inventory {
    /// Quantidade de um item (0 quando não existe).
    pub fn get(&self, id: &str) -> i64 {
        match id {
            "gold" => self.gold,
            "wood" => self.wood,
            "stone" => self.stone,
            "berry" => self.berry,
            "korok_seed" => self.korok_seed,
            "potion" => self.potion,
            "antidote" => self.antidote,
            "bomb" => self.bomb,
            "moss_potion" => self.moss_potion,
            other => self.items.get(other).copied().unwrap_or(0),
        }
    }

    /// Escreve um item no campo fixo ou em `items`.
    pub fn set(&mut self, id: &str, n: i64) {
        match id {
            "gold" => self.gold = n,
            "wood" => self.wood = n,
            "stone" => self.stone = n,
            "berry" => self.berry = n,
            "korok_seed" => self.korok_seed = n,
            "potion" => self.potion = n,
            "antidote" => self.antidote = n,
            "bomb" => self.bomb = n,
            "moss_potion" => self.moss_potion = n,
            other => {
                self.items.insert(other.to_string(), n);
            }
        }
    }

    /// Todos os pares `(id, quantidade)` — os campos fixos primeiro, os
    /// itens arbitrários depois (um id repetido em `items` ganha).
    pub fn entries(&self) -> Vec<(&str, i64)> {
        let mut out = vec![
            ("gold", self.gold),
            ("wood", self.wood),
            ("stone", self.stone),
            ("berry", self.berry),
            ("korok_seed", self.korok_seed),
            ("potion", self.potion),
            ("antidote", self.antidote),
            ("bomb", self.bomb),
            ("moss_potion", self.moss_potion),
        ];
        out.extend(self.items.iter().map(|(id, n)| (id.as_str(), *n)));
        out
    }
}

/// Diário de quests. `states` é `q.<id>`, `done` é a lista derivada e
/// `tracked` é o `tracked_quest` que o HUD segue.
#[derive(Debug, Clone, PartialEq, Default, Resource, Serialize, Deserialize, Reflect)]
#[serde(default, rename_all = "snake_case")]
#[reflect(Resource)]
pub struct QuestLog {
    pub states: BTreeMap<String, String>,
    pub done: Vec<String>,
    pub tracked: Option<String>,
}

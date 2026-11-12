//! Atributos e stats derivados do herói — o grafo do `bevy_gauge`.
//!
//! O antigo [`crate::gear::Stats`] era um mapa plano: talentos e equipamento
//! somavam efeitos soltos (`damage`, `crit`, `speed`, …). Aqui os efeitos
//! passam a alimentar **atributos** (Força, Agilidade, Vitalidade, Precisão,
//! Espírito) e as stats derivadas são **expressões** do gauge:
//!
//! ```text
//! Força 7  ──►  Stat.damage = (Strength - 5) * 3  = +6 dano
//! Vida 7   ──►  Stat.max_hp = (Vitality - 5) * 10 = +20 vida
//! Agi 9    ──►  Stat.speed  = (Agility - 5) * 0.025 = +10% velocidade
//! ```
//!
//! O valor do gauge é a propagação: mudar `Strength` (talento novo, item
//! equipado) reavalia as stats dependentes sem ordem manual. As stats que o
//! jogo já consumia mantêm as chaves antigas (`damage`, `crit`, `speed`,
//! `max_hp`, `defense`, …), por isso [`crate::gear::player_stats`] e
//! `viber.vael.stat(key)` continuam a falar a mesma língua com o Lua.
//!
//! ## Porque há uma `World` própria
//!
//! A bridge `viber.vael` é chamada de dentro do `luau_update`, no meio de um
//! sistema da App principal: não dá para pedir `AttributesMut` daí. O
//! [`StatEngine`] guarda uma `World` mínima (só `DependencyGraph`,
//! `TagResolver` e a entidade do herói) e corre um sistema curto quando o
//! conjunto equipado/talentos muda. O resultado é um [`StatSnapshot`] que as
//! funções Lua leem — síncrono, testável sem a App e sem um frame de atraso.
//!
//! O interner de nomes do gauge é global (`Interner::set_global`), por isso a
//! App principal pode acrescentar o `AttributesPlugin` e partilhar os mesmos
//! `AttributeId` sem duplicar nomes.

use std::collections::{BTreeMap, BTreeSet, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};

use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use bevy_gauge::attribute_id::Interner;
use bevy_gauge::graph::DependencyGraph;
use bevy_gauge::prelude::*;
use bevy_gauge::requirements::AttributeRequirement;
use bevy_gauge::tags::TagResolver;

use crate::gear::{GearRules, GearState, SLOTS};

/// Nomes internos dos atributos no gauge.
pub const STRENGTH: &str = "Strength";
pub const AGILITY: &str = "Agility";
pub const VITALITY: &str = "Vitality";
pub const PRECISION: &str = "Precision";
pub const SPIRIT: &str = "Spirit";

/// Chave de `viber.vael.attribute`/`stat_sheet` → nome no gauge.
pub const ATTRIBUTE_KEYS: [(&str, &str); 5] = [
    ("strength", STRENGTH),
    ("agility", AGILITY),
    ("vitality", VITALITY),
    ("precision", PRECISION),
    ("spirit", SPIRIT),
];

/// Prefixo das stats derivadas no gauge (`Stat.damage`, `Stat.crit`, …).
pub const STAT_PREFIX: &str = "Stat.";

/// Base de cada atributo no nível 1 (sem talentos nem equipamento).
pub const ATTRIBUTE_BASE: f32 = 5.0;

/// Quanto cada ponto de atributo rende nas stats. É a tabela de balanceamento
/// do sistema; os talentos de atributo do `data/progression.json` mantêm a
/// paridade com os efeitos antigos a estes valores (ex.: +2 Força = +6 dano).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AttributeRules {
    /// Valor de todos os atributos sem modificadores.
    pub base: f32,
    /// Dano por ponto de Força acima da base.
    pub damage_per_strength: f32,
    /// Empurrão por ponto de Força acima da base (multiplicador, 1.0 = base).
    pub knockback_per_strength: f32,
    /// Velocidade de movimento por ponto de Agilidade acima da base.
    pub speed_per_agility: f32,
    /// Velocidade de ataque por ponto de Agilidade acima da base.
    pub attack_speed_per_agility: f32,
    /// Recarga da esquiva por ponto de Agilidade acima da base (delta).
    pub dash_cd_per_agility: f32,
    /// Vida máxima por ponto de Vitalidade acima da base.
    pub hp_per_vitality: f32,
    /// Defesa por ponto de Vitalidade acima da base.
    pub defense_per_vitality: f32,
    /// Crítico por ponto de Precisão acima da base.
    pub crit_per_precision: f32,
    /// Multiplicador de crítico por ponto de Precisão acima da base (delta).
    pub crit_mult_per_precision: f32,
    /// Roubo de vida por ponto de Espírito acima da base.
    pub lifesteal_per_spirit: f32,
    /// Execução por ponto de Espírito acima da base (delta).
    pub execute_per_spirit: f32,
    /// Cura (poções e cantil) por ponto de Espírito acima da base
    /// (multiplicador).
    pub heal_power_per_spirit: f32,
    /// Cargas de cantil por ponto de Espírito acima da base (fracionário; o
    /// Lua faz `floor` na soma com a base).
    pub flasks_per_spirit: f32,
}

impl Default for AttributeRules {
    fn default() -> Self {
        Self {
            base: ATTRIBUTE_BASE,
            damage_per_strength: 3.0,
            knockback_per_strength: 0.01,
            speed_per_agility: 0.025,
            attack_speed_per_agility: 0.0075,
            dash_cd_per_agility: -0.005,
            hp_per_vitality: 10.0,
            defense_per_vitality: 0.003,
            crit_per_precision: 0.02,
            crit_mult_per_precision: 0.005,
            lifesteal_per_spirit: 0.005,
            execute_per_spirit: 0.002,
            heal_power_per_spirit: 0.005,
            flasks_per_spirit: 0.25,
        }
    }
}

impl AttributeRules {
    /// Expressões das stats derivadas, na ordem em que entram no grafo.
    ///
    /// `Stat.speed`, `Stat.crit`, `Stat.max_hp` e `Stat.damage` são **deltas**
    /// (o `Stats`/Lua soma-os à base do jogo); `Stat.attack_speed`,
    /// `Stat.knockback` e `Stat.heal_power` são multiplicadores (1.0 = base).
    pub fn expressions(&self) -> Vec<(&'static str, String)> {
        let base = self.base;
        vec![
            (
                "damage",
                format!("(Strength - {base}) * {}", self.damage_per_strength),
            ),
            (
                "max_hp",
                format!("(Vitality - {base}) * {}", self.hp_per_vitality),
            ),
            (
                "speed",
                format!("(Agility - {base}) * {}", self.speed_per_agility),
            ),
            (
                "crit",
                format!("(Precision - {base}) * {}", self.crit_per_precision),
            ),
            (
                "defense",
                format!("(Vitality - {base}) * {}", self.defense_per_vitality),
            ),
            (
                "lifesteal",
                format!("(Spirit - {base}) * {}", self.lifesteal_per_spirit),
            ),
            (
                "execute",
                format!("(Spirit - {base}) * {}", self.execute_per_spirit),
            ),
            (
                "crit_mult",
                format!("(Precision - {base}) * {}", self.crit_mult_per_precision),
            ),
            (
                "dash_cd",
                if self.dash_cd_per_agility >= 0.0 {
                    format!("(Agility - {base}) * {}", self.dash_cd_per_agility)
                } else {
                    format!(
                        "0.0 - (Agility - {base}) * {}",
                        self.dash_cd_per_agility.abs()
                    )
                },
            ),
            (
                "flasks",
                format!("(Spirit - {base}) * {}", self.flasks_per_spirit),
            ),
            (
                "attack_speed",
                format!(
                    "1.0 + (Agility - {base}) * {}",
                    self.attack_speed_per_agility
                ),
            ),
            (
                "knockback",
                format!(
                    "1.0 + (Strength - {base}) * {}",
                    self.knockback_per_strength
                ),
            ),
            (
                "heal_power",
                format!("1.0 + (Spirit - {base}) * {}", self.heal_power_per_spirit),
            ),
        ]
    }

    /// Nome do atributo (gauge) para uma chave de efeito (`strength`, …).
    pub fn attribute_of(key: &str) -> Option<&'static str> {
        ATTRIBUTE_KEYS
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, name)| *name)
    }
}

/// Valores acabados de um rebuild: atributos e stats derivadas.
///
/// Os `f64` vêm arredondados a 4 casas — o gauge calcula em `f32` e sem isto
/// um `+10%` lia-se `1.100000023841858` no Lua.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StatSnapshot {
    /// Chave minúscula (`strength`, `agility`, …) → valor.
    pub attributes: BTreeMap<String, f64>,
    /// Chave da stat (`damage`, `crit`, `defense`, …) → valor.
    pub stats: BTreeMap<String, f64>,
}

impl StatSnapshot {
    /// Valor de um atributo (0 quando não existe).
    pub fn attribute(&self, key: &str) -> f64 {
        self.attributes.get(key).copied().unwrap_or(0.0)
    }

    /// Valor de uma stat. Identidades (`attack_speed`, `knockback`,
    /// `heal_power`) valem 1.0 quando o grafo não as criou; o resto vale 0.
    pub fn stat(&self, key: &str) -> f64 {
        self.stats.get(key).copied().unwrap_or(match key {
            "attack_speed" | "knockback" | "heal_power" => 1.0,
            _ => 0.0,
        })
    }

    /// Multiplicador de velocidade do jogador (`1 + Stat.speed`).
    pub fn speed_mult(&self) -> f64 {
        1.0 + self.stat("speed")
    }
}

/// Motor do stat sheet: uma `World` mínima com a entidade do herói.
pub struct StatEngine {
    world: World,
    entity: Entity,
    rules: AttributeRules,
    /// Último [`ModifierSet`] aplicado (para o remover antes de reconstruir).
    applied: Option<ModifierSet>,
    /// Assinatura do conjunto equipado/talentos do último rebuild.
    digest: Option<u64>,
    snapshot: StatSnapshot,
}

impl StatEngine {
    /// Cria a `World` e a entidade do herói. O interner global fica pronto
    /// mesmo que a App principal não tenha o `AttributesPlugin`.
    pub fn new(rules: AttributeRules) -> Self {
        // O interner é global e só precisa de ser publicado uma vez; criar um
        // `ThreadedRodeo` novo por motor (o menu cria um por frame) era lixo.
        static INTERNER: std::sync::Once = std::sync::Once::new();
        INTERNER.call_once(|| Interner::new().set_global());
        let mut world = World::new();
        world.insert_resource(DependencyGraph::default());
        world.insert_resource(TagResolver::new());
        let entity = world.spawn(Attributes::new()).id();
        Self {
            world,
            entity,
            rules,
            applied: None,
            digest: None,
            snapshot: StatSnapshot::default(),
        }
    }

    /// Devolve o snapshot atual, reconstruindo o grafo se os talentos ou o
    /// equipamento mudaram.
    ///
    /// `state` é mutável porque `crate::gear::effects` limpa marcas órfãs
    /// (item vendido/entregue) no mesmo passo.
    pub fn snapshot(
        &mut self,
        gear: &GearRules,
        state: &mut GearState,
        learned: &BTreeSet<String>,
    ) -> &StatSnapshot {
        let digest = digest_of(state, learned);
        if self.digest != Some(digest) {
            self.rebuild(gear, state, learned);
        }
        &self.snapshot
    }

    /// Verdade de uma expressão de requisito (`"Strength >= 7"`).
    ///
    /// Deve ser chamada depois de [`snapshot`](Self::snapshot) para o grafo
    /// refletir o equipamento/talentos atuais. Expressão vazia = satisfeita.
    pub fn requirement_met(&self, expression: &str) -> bool {
        let expression = expression.trim();
        if expression.is_empty() {
            return true;
        }
        let Some(attributes) = self.world.get::<Attributes>(self.entity) else {
            return false;
        };
        let mut requirement = AttributeRequirement::new(expression);
        requirement.compile();
        requirement.met(attributes)
    }

    /// Reconstrói removendo o set anterior e aplicando o novo no mesmo
    /// sistema (sem acumular modificadores entre chamadas).
    fn rebuild(&mut self, gear: &GearRules, state: &mut GearState, learned: &BTreeSet<String>) {
        let set = modifier_set(&self.rules, gear, state, learned);
        let previous = self.applied.clone();
        let applied = set.clone();
        let entity = self.entity;
        let rules = self.rules;

        let result =
            self.world
                .run_system_once(move |mut attributes: AttributesMut| -> StatSnapshot {
                    if let Some(previous) = &previous {
                        previous.remove(entity, &mut attributes);
                    }
                    applied.apply_all(entity, &mut attributes);
                    let attributes = attributes
                        .get_attributes(entity)
                        .expect("entidade do stat sheet existe desde o arranque");
                    gather(attributes, &rules)
                });

        match result {
            Ok(snapshot) => {
                self.snapshot = snapshot;
                self.applied = Some(set);
                // O `effects` dentro do `modifier_set` pode ter limpado uma
                // marca órfã: a assinatura guardada é a do estado já limpo.
                self.digest = Some(digest_of(state, learned));
            }
            Err(err) => {
                // Mantém o snapshot e o set anteriores; a próxima chamada
                // volta a tentar (o digest não avançou).
                warn!("vael: falha a reconstruir os atributos: {err}");
            }
        }
    }
}

/// Assinatura dos inputs que mudam o grafo: talentos aprendidos e ids
/// equipados. As contagens da mochila não entram (só validam o equipamento).
fn digest_of(state: &GearState, learned: &BTreeSet<String>) -> u64 {
    let mut hasher = DefaultHasher::new();
    for id in learned {
        id.hash(&mut hasher);
    }
    for slot in SLOTS {
        state.equipped.get(slot).hash(&mut hasher);
    }
    hasher.finish()
}

/// Constrói o set completo: base dos atributos, expressões derivadas e os
/// efeitos de talentos + equipamento.
pub fn modifier_set(
    rules: &AttributeRules,
    gear: &GearRules,
    state: &mut GearState,
    learned: &BTreeSet<String>,
) -> ModifierSet {
    let mut set = ModifierSet::new();

    for (_, name) in ATTRIBUTE_KEYS {
        set.add(name, rules.base);
    }
    for (key, expression) in rules.expressions() {
        set.add_expr(&format!("{STAT_PREFIX}{key}"), &expression);
    }

    for id in learned {
        if let Some(effects) = gear.skills.get(id) {
            add_effects(&mut set, effects);
        }
    }

    // `effects` valida os slots e soma as três peças equipadas.
    let gear_effects = crate::gear::effects(gear, state);
    add_effects(&mut set, &gear_effects);
    set
}

/// Um efeito de talento/item: atributo quando a chave é `strength` & cia.,
/// senão `Stat.<chave>` (é o que `viber.vael.stat(key)` lê). Não finitos são
/// ignorados, como no `add_effects` antigo.
fn add_effects(set: &mut ModifierSet, effects: &BTreeMap<String, f64>) {
    for (key, value) in effects {
        if key.is_empty() || !value.is_finite() {
            continue;
        }
        let value = *value as f32;
        if !value.is_finite() {
            continue;
        }
        match AttributeRules::attribute_of(key) {
            Some(name) => set.add(name, value),
            None => set.add(&format!("{STAT_PREFIX}{key}"), value),
        }
    }
}

/// Lê os valores do `Attributes` para o snapshot.
fn gather(attributes: &Attributes, _rules: &AttributeRules) -> StatSnapshot {
    let mut snapshot = StatSnapshot::default();
    for (key, name) in ATTRIBUTE_KEYS {
        snapshot
            .attributes
            .insert(key.to_string(), round(attributes.value(name)));
    }

    // As stats são todas as entidades `Stat.*` que o grafo criou (expressões
    // fixas + efeitos arbitrários de itens/talentos).
    let interner = Interner::global();
    for (id, value) in attributes.iter() {
        let name = interner.resolve(id);
        if let Some(key) = name.strip_prefix(STAT_PREFIX) {
            snapshot.stats.insert(key.to_string(), round(value));
        }
    }

    // Identidades: mesmo sem modificadores, o `stat` tem de devolver 1.0.
    for key in ["attack_speed", "knockback", "heal_power"] {
        snapshot.stats.entry(key.to_string()).or_insert(1.0);
    }
    snapshot
}

/// `f32` do gauge → `f64` do Lua, arredondado a 4 casas.
fn round(value: f32) -> f64 {
    let value = value as f64;
    if value.is_finite() {
        (value * 10_000.0).round() / 10_000.0
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gear::GearItem;

    fn efeito(pares: &[(&str, f64)]) -> BTreeMap<String, f64> {
        pares
            .iter()
            .map(|(key, value)| (key.to_string(), *value))
            .collect()
    }

    /// Regras de teste com os talentos de atributo e um item de cada slot.
    fn regras() -> GearRules {
        let mut gear = GearRules::default();
        gear.skills
            .insert("strength1".into(), efeito(&[("strength", 2.0)]));
        gear.skills
            .insert("vitality1".into(), efeito(&[("vitality", 2.0)]));
        gear.skills
            .insert("agility1".into(), efeito(&[("agility", 4.0)]));
        gear.skills
            .insert("precision1".into(), efeito(&[("precision", 4.0)]));
        gear.skills
            .insert("spirit1".into(), efeito(&[("spirit", 3.0)]));
        gear.skills
            .insert("forca".into(), efeito(&[("damage", 5.0)]));
        gear.skills.insert(
            "quebrado".into(),
            efeito(&[("damage", f64::NAN), ("speed", f64::INFINITY)]),
        );
        gear.items.insert(
            "amuleto".into(),
            GearItem {
                id: "amuleto".into(),
                label: "amuleto".into(),
                slot: "charm".into(),
                effects: efeito(&[("damage", 8.0), ("defense", 0.15), ("venom_resist", 0.5)]),
                weapon: None,
                ..Default::default()
            },
        );
        gear
    }

    fn estado(equipado: Option<&str>) -> GearState {
        let mut state = GearState::default();
        if let Some(id) = equipado {
            state.held.insert(id.to_string(), 1.0);
            state.equipped.insert("charm".into(), id.to_string());
        }
        state
    }

    #[test]
    fn atributos_base_sem_talentos() {
        let mut engine = StatEngine::new(AttributeRules::default());
        let mut state = estado(None);
        let snap = engine
            .snapshot(&regras(), &mut state, &BTreeSet::new())
            .clone();
        assert_eq!(snap.attribute("strength"), 5.0);
        assert_eq!(snap.attribute("spirit"), 5.0);
        assert_eq!(snap.stat("damage"), 0.0);
        assert_eq!(snap.stat("crit"), 0.0);
        assert_eq!(snap.stat("speed"), 0.0);
        assert_eq!(snap.stat("max_hp"), 0.0);
        assert_eq!(snap.speed_mult(), 1.0);
        assert_eq!(snap.stat("attack_speed"), 1.0);
        assert_eq!(snap.stat("knockback"), 1.0);
        assert_eq!(snap.stat("heal_power"), 1.0);
        assert_eq!(snap.stat("defense"), 0.0);
    }

    #[test]
    fn talentos_de_atributo_propagam_para_as_stats() {
        let mut engine = StatEngine::new(AttributeRules::default());
        let mut state = estado(None);
        let learned = BTreeSet::from([
            "strength1".to_string(),
            "vitality1".to_string(),
            "agility1".to_string(),
            "precision1".to_string(),
            "spirit1".to_string(),
        ]);
        let snap = engine.snapshot(&regras(), &mut state, &learned).clone();

        assert_eq!(snap.attribute("strength"), 7.0);
        assert_eq!(snap.attribute("vitality"), 7.0);
        assert_eq!(snap.attribute("agility"), 9.0);
        assert_eq!(snap.attribute("precision"), 9.0);
        assert_eq!(snap.attribute("spirit"), 8.0);

        // Paridade com os efeitos antigos dos talentos.
        assert_eq!(snap.stat("damage"), 6.0); // 2 Força × 3
        assert_eq!(snap.stat("max_hp"), 20.0); // 2 Vitalidade × 10
        assert_eq!(snap.stat("speed"), 0.1); // 4 Agilidade × 0.025
        assert_eq!(snap.speed_mult(), 1.1);
        assert_eq!(snap.stat("crit"), 0.08); // 4 Precisão × 0.02

        // Bónus cruzados: os pontos também mexem noutras stats.
        assert_eq!(snap.stat("attack_speed"), 1.03);
        assert_eq!(snap.stat("dash_cd"), -0.02);
        assert_eq!(snap.stat("knockback"), 1.02);
        assert_eq!(snap.stat("crit_mult"), 0.02);
        assert_eq!(snap.stat("defense"), 0.006);
        assert_eq!(snap.stat("lifesteal"), 0.015);
        assert_eq!(snap.stat("execute"), 0.006);
        assert_eq!(snap.stat("flasks"), 0.75);
        assert_eq!(snap.stat("heal_power"), 1.015);
    }

    #[test]
    fn equipamento_soma_stats_e_efeitos_arbitrarios() {
        let mut engine = StatEngine::new(AttributeRules::default());
        let mut state = estado(Some("amuleto"));
        let snap = engine
            .snapshot(&regras(), &mut state, &BTreeSet::new())
            .clone();
        assert_eq!(snap.stat("damage"), 8.0);
        assert_eq!(snap.stat("defense"), 0.15);
        // Efeito sem stat derivada própria: fica em `Stat.venom_resist`.
        assert_eq!(snap.stat("venom_resist"), 0.5);
    }

    #[test]
    fn equipamento_pode_dar_atributos() {
        let mut gear = regras();
        gear.items.insert(
            "manto_vital".into(),
            GearItem {
                id: "manto_vital".into(),
                label: "manto vital".into(),
                slot: "armor".into(),
                effects: efeito(&[("vitality", 2.0), ("agility", 1.0)]),
                weapon: None,
                ..Default::default()
            },
        );
        let mut state = GearState::default();
        state.held.insert("manto_vital".into(), 1.0);
        state.equipped.insert("armor".into(), "manto_vital".into());

        let mut engine = StatEngine::new(AttributeRules::default());
        let snap = engine.snapshot(&gear, &mut state, &BTreeSet::new()).clone();
        assert_eq!(snap.attribute("vitality"), 7.0);
        assert_eq!(snap.attribute("agility"), 6.0);
        assert_eq!(snap.stat("max_hp"), 20.0);
        assert_eq!(snap.stat("speed"), 0.025);
        assert_eq!(snap.stat("attack_speed"), 1.0075);
    }

    #[test]
    fn rebuild_nao_acumula_modificadores() {
        let mut engine = StatEngine::new(AttributeRules::default());
        let mut state = estado(None);
        let rules = regras();
        let um = BTreeSet::from(["strength1".to_string()]);
        let dois = BTreeSet::from(["strength1".to_string(), "vitality1".to_string()]);

        assert_eq!(engine.snapshot(&rules, &mut state, &um).stat("damage"), 6.0);
        let snap = engine.snapshot(&rules, &mut state, &dois).clone();
        assert_eq!(snap.stat("damage"), 6.0);
        assert_eq!(snap.stat("max_hp"), 20.0);

        // Voltar ao conjunto vazio tem de desfazer tudo (sem somar por cima).
        let snap = engine
            .snapshot(&rules, &mut state, &BTreeSet::new())
            .clone();
        assert_eq!(snap.stat("damage"), 0.0);
        assert_eq!(snap.stat("max_hp"), 0.0);
        assert_eq!(snap.attribute("strength"), 5.0);
    }

    #[test]
    fn efeitos_nao_finitos_sao_ignorados() {
        let mut engine = StatEngine::new(AttributeRules::default());
        let mut state = estado(None);
        let snap = engine
            .snapshot(
                &regras(),
                &mut state,
                &BTreeSet::from(["quebrado".to_string()]),
            )
            .clone();
        assert_eq!(snap.stat("damage"), 0.0);
        assert_eq!(snap.stat("speed"), 0.0);
    }

    #[test]
    fn requisito_le_os_atributos_do_grafo() {
        let mut engine = StatEngine::new(AttributeRules::default());
        let mut state = estado(None);
        let rules = regras();
        engine.snapshot(&rules, &mut state, &BTreeSet::new());
        assert!(!engine.requirement_met("Strength >= 7"));
        assert!(engine.requirement_met("Vitality >= 5"));
        assert!(engine.requirement_met(""));

        engine.snapshot(
            &rules,
            &mut state,
            &BTreeSet::from(["strength1".to_string()]),
        );
        assert!(engine.requirement_met("Strength >= 7"));
        assert!(!engine.requirement_met("Strength >= 8"));
        // Uma expressão inválida nunca é satisfeita (o gauge regista o erro).
        assert!(!engine.requirement_met("Strength >="));
    }

    #[test]
    fn regras_de_atributo_sao_configuraveis() {
        let rules = AttributeRules {
            damage_per_strength: 2.0,
            hp_per_vitality: 12.0,
            ..Default::default()
        };
        let mut engine = StatEngine::new(rules);
        let mut state = estado(None);
        let snap = engine
            .snapshot(
                &regras(),
                &mut state,
                &BTreeSet::from(["strength1".to_string(), "vitality1".to_string()]),
            )
            .clone();
        assert_eq!(snap.stat("damage"), 4.0);
        assert_eq!(snap.stat("max_hp"), 24.0);
    }
}

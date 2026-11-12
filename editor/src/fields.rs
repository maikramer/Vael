//! Commits dos campos de texto e combobox para o estado do editor.
//!
//! O formulário é reconstruído depois de cada commit; os valores que o
//! utilizador ainda não confirmou não são tocados.

use std::collections::BTreeMap;

use bevy::prelude::*;
use jackdaw_feathers::combobox::ComboBoxChangeEvent;
use jackdaw_feathers::text_edit::TextEditCommitEvent;
use vael_content::{DialogueFile, Expect, ItemShop, ItemUse, Objective, Parley, Report, WeaponDef};

use crate::state::VaelState;
use crate::ui::{ChoiceFieldBinding, FieldId, PanelId, PanelState, TextFieldBinding};

pub struct VaelFieldsPlugin;

impl Plugin for VaelFieldsPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(on_text_commit)
            .add_observer(on_choice_change);
    }
}

fn on_text_commit(
    event: On<TextEditCommitEvent>,
    bindings: Query<&TextFieldBinding>,
    parents: Query<&ChildOf>,
    mut state: ResMut<VaelState>,
    mut panel: ResMut<PanelState>,
) {
    let Some(binding) = std::iter::once(event.entity)
        .chain(parents.iter_ancestors(event.entity))
        .find_map(|entity| bindings.get(entity).ok().cloned())
    else {
        return;
    };
    match apply_text(&binding, &event.text, &mut state, &mut panel) {
        Ok(()) => panel.status.clear(),
        Err(err) => panel.status = format!("{:?}: {err}", binding.field),
    }
}

fn on_choice_change(
    event: On<ComboBoxChangeEvent>,
    bindings: Query<&ChoiceFieldBinding>,
    parents: Query<&ChildOf>,
    mut state: ResMut<VaelState>,
    mut panel: ResMut<PanelState>,
) {
    let Some(binding) = std::iter::once(event.entity)
        .chain(parents.iter_ancestors(event.entity))
        .find_map(|entity| bindings.get(entity).ok().cloned())
    else {
        return;
    };
    let value = event.value.clone().unwrap_or_else(|| event.label.clone());
    match apply_choice(&binding, &value, &mut state, &mut panel) {
        Ok(()) => panel.status.clear(),
        Err(err) => panel.status = format!("{:?}: {err}", binding.field),
    }
}

fn apply_text(
    binding: &TextFieldBinding,
    text: &str,
    state: &mut VaelState,
    panel: &mut PanelState,
) -> Result<(), String> {
    let value = text.trim();
    if binding.field == FieldId::QuestSearch {
        panel.quest_search = value.to_string();
        return Ok(());
    }
    match binding.panel {
        PanelId::Items => apply_item(binding, value, state),
        PanelId::Landmarks => apply_landmark(binding, value, state, panel),
        PanelId::Progression => apply_progression(binding, value, state),
        PanelId::Dialogues => apply_dialogue(binding, value, state, panel),
        PanelId::Quests => apply_quest(binding, value, state, panel),
    }
}

fn apply_item(
    binding: &TextFieldBinding,
    value: &str,
    state: &mut VaelState,
) -> Result<(), String> {
    let item = state
        .items
        .items
        .iter_mut()
        .find(|item| item.id == binding.key)
        .ok_or_else(|| format!("item `{}` não existe", binding.key))?;

    match binding.field {
        FieldId::Label => item.label = value.to_string(),
        FieldId::Order => item.order = parse_i64(value)?,
        FieldId::Stack => item.stack = parse_opt_positive(value)?,
        FieldId::Desc => item.desc = value.to_string(),
        FieldId::Slot => item.slot = value.to_string(),
        FieldId::ShopBuy => {
            let shop = item.shop.get_or_insert_with(ItemShop::default);
            shop.buy = parse_opt_positive(value)?;
        }
        FieldId::ShopSell => {
            let shop = item.shop.get_or_insert_with(ItemShop::default);
            shop.sell = parse_opt_positive(value)?;
        }
        FieldId::ShopLabel => {
            let shop = item.shop.get_or_insert_with(ItemShop::default);
            shop.label = value.to_string();
        }
        FieldId::UseHeal => {
            let use_item = item.use_item.get_or_insert_with(ItemUse::default);
            use_item.heal = if value.is_empty() {
                0.0
            } else {
                parse_f64(value)?
            };
        }
        FieldId::UseCure => {
            let use_item = item.use_item.get_or_insert_with(ItemUse::default);
            use_item.cure = parse_bool(value)?;
        }
        FieldId::WeaponDamage => {
            let weapon = item.weapon.get_or_insert_with(WeaponDef::default);
            weapon.damage = parse_f64(value)?;
        }
        FieldId::WeaponSpeed => {
            let weapon = item.weapon.get_or_insert_with(WeaponDef::default);
            weapon.speed = parse_f64(value)?;
        }
        FieldId::WeaponRange => {
            let weapon = item.weapon.get_or_insert_with(WeaponDef::default);
            weapon.range = parse_f64(value)?;
        }
        FieldId::WeaponGrip => {
            let weapon = item.weapon.get_or_insert_with(WeaponDef::default);
            weapon.grip = value.to_string();
        }
        FieldId::Requires => item.requires = value.to_string(),
        FieldId::RequiresText => item.requires_text = value.to_string(),
        FieldId::Effects => item.effects = parse_effects(value)?,
        other => return Err(format!("campo {other:?} não se aplica a itens")),
    }

    // Um `use`/`shop` sem conteúdo deixa de existir (como o plugin antigo).
    if item
        .use_item
        .as_ref()
        .is_some_and(|use_item| use_item.heal == 0.0 && !use_item.cure)
    {
        item.use_item = None;
    }
    if item
        .shop
        .as_ref()
        .is_some_and(|shop| shop.buy.is_none() && shop.sell.is_none() && shop.label.is_empty())
    {
        item.shop = None;
    }
    Ok(())
}

fn apply_landmark(
    binding: &TextFieldBinding,
    value: &str,
    state: &mut VaelState,
    panel: &mut PanelState,
) -> Result<(), String> {
    let landmarks = &mut state.landmarks.landmarks;
    let index = landmarks
        .iter()
        .position(|landmark| landmark.name == binding.key)
        .ok_or_else(|| format!("marco `{}` não existe", binding.key))?;

    match binding.field {
        FieldId::Name => {
            if value.is_empty() {
                return Err("nome vazio".to_string());
            }
            if value != binding.key && landmarks.iter().any(|landmark| landmark.name == value) {
                return Err("já existe um marco com esse nome".to_string());
            }
            landmarks[index].name = value.to_string();
            panel.landmark = Some(value.to_string());
        }
        FieldId::Label => landmarks[index].label = value.to_string(),
        FieldId::Radius => landmarks[index].radius = parse_f64(value)?,
        FieldId::Survey => landmarks[index].survey = parse_opt_string(value),
        FieldId::Pilgrim => landmarks[index].pilgrim = parse_bool(value)?,
        other => return Err(format!("campo {other:?} não se aplica a marcos")),
    }
    Ok(())
}

fn apply_progression(
    binding: &TextFieldBinding,
    value: &str,
    state: &mut VaelState,
) -> Result<(), String> {
    match binding.field {
        FieldId::XpBase => state.progression.xp.base = parse_i64(value)?,
        FieldId::XpGrowth => state.progression.xp.growth = parse_f64(value)?,
        FieldId::XpSkillPoints => {
            state.progression.xp.skill_points_per_level = parse_i64(value)?;
        }
        FieldId::SongSeedEvery => state.progression.song_seed_talent_every = parse_i64(value)?,
        FieldId::KillXpLine => {
            let (creature, xp) = value
                .split_once(':')
                .ok_or_else(|| "usa `criatura:xp`".to_string())?;
            let creature = creature.trim();
            if creature.is_empty() {
                return Err("criatura vazia".to_string());
            }
            let xp = parse_i64(xp.trim())?;
            if creature != binding.key && state.progression.kill_xp.contains_key(creature) {
                return Err(format!("`{creature}` já existe em kill_xp"));
            }
            state.progression.kill_xp.remove(&binding.key);
            state.progression.kill_xp.insert(creature.to_string(), xp);
        }
        FieldId::Label | FieldId::Requires | FieldId::Excludes | FieldId::Effects => {
            let skill = state
                .progression
                .skills
                .iter_mut()
                .find(|skill| skill.id == binding.key)
                .ok_or_else(|| format!("talento `{}` não existe", binding.key))?;
            match binding.field {
                FieldId::Label => skill.label = value.to_string(),
                FieldId::Requires => skill.requires = parse_opt_string(value),
                FieldId::Excludes => skill.excludes = parse_opt_string(value),
                FieldId::Effects => skill.effects = parse_effects(value)?,
                _ => unreachable!(),
            }
        }
        other => return Err(format!("campo {other:?} não se aplica à progressão")),
    }
    Ok(())
}

fn apply_dialogue(
    binding: &TextFieldBinding,
    value: &str,
    state: &mut VaelState,
    panel: &mut PanelState,
) -> Result<(), String> {
    let file_name = panel
        .dialogue_file
        .clone()
        .ok_or_else(|| "sem ficheiro de diálogo".to_string())?;
    let file: &mut DialogueFile = state
        .dialogues
        .get_mut(&file_name)
        .ok_or_else(|| format!("diálogo `{file_name}` não carregado"))?;

    match binding.field {
        FieldId::Start => file.start = value.to_string(),
        FieldId::NodeId => {
            if value.is_empty() {
                return Err("id de nó vazio".to_string());
            }
            if value != binding.key {
                if file.nodes.contains_key(value) {
                    return Err("já existe um nó com esse id".to_string());
                }
                let node = file
                    .nodes
                    .remove(&binding.key)
                    .ok_or_else(|| format!("nó `{}` não existe", binding.key))?;
                if file.start == binding.key {
                    file.start = value.to_string();
                }
                for other in file.nodes.values_mut() {
                    for choice in &mut other.choices {
                        if choice.goto == binding.key {
                            choice.goto = value.to_string();
                        }
                    }
                }
                file.nodes.insert(value.to_string(), node);
                panel.dialogue_node = Some(value.to_string());
            }
        }
        FieldId::NodeSpeaker | FieldId::NodeText | FieldId::NodeVoice | FieldId::NodePortrait => {
            let node = file
                .nodes
                .get_mut(&binding.key)
                .ok_or_else(|| format!("nó `{}` não existe", binding.key))?;
            match binding.field {
                FieldId::NodeSpeaker => node.speaker = value.to_string(),
                FieldId::NodeText => node.text = value.to_string(),
                FieldId::NodeVoice => node.voice = parse_opt_string(value),
                FieldId::NodePortrait => node.portrait = parse_opt_string(value),
                _ => unreachable!(),
            }
        }
        FieldId::ChoiceText | FieldId::ChoiceGoto => {
            let node = file
                .nodes
                .get_mut(&binding.key)
                .ok_or_else(|| format!("nó `{}` não existe", binding.key))?;
            let choice = node
                .choices
                .get_mut(binding.sub)
                .ok_or_else(|| format!("escolha {} não existe", binding.sub))?;
            match binding.field {
                FieldId::ChoiceText => choice.text = value.to_string(),
                FieldId::ChoiceGoto => choice.goto = value.to_string(),
                _ => unreachable!(),
            }
        }
        other => return Err(format!("campo {other:?} não se aplica a diálogos")),
    }
    Ok(())
}

fn apply_quest(
    binding: &TextFieldBinding,
    value: &str,
    state: &mut VaelState,
    panel: &mut PanelState,
) -> Result<(), String> {
    if binding.field == FieldId::QuestId {
        if value.is_empty() {
            return Err("id de quest vazio".to_string());
        }
        if value != binding.key {
            if state.quest(value).is_some() {
                return Err("já existe uma quest com esse id".to_string());
            }
            let quest = state
                .quest_mut(&binding.key)
                .ok_or_else(|| format!("quest `{}` não existe", binding.key))?;
            quest.id = value.to_string();
            panel.quest = Some(value.to_string());
        }
        return Ok(());
    }

    let quest = state
        .quest_mut(&binding.key)
        .ok_or_else(|| format!("quest `{}` não existe", binding.key))?;

    match binding.field {
        FieldId::Npc => quest.npc = value.to_string(),
        FieldId::Title => quest.title = value.to_string(),
        FieldId::QuestPortrait => quest.portrait = parse_opt_string(value),
        FieldId::QuestVoice => quest.voice = parse_opt_string(value),
        FieldId::ObjectiveTarget => quest.objective.target = value.to_string(),
        FieldId::ObjectiveCount => quest.objective.count = parse_i64(value)?,
        FieldId::ObjectiveRadius => quest.objective.radius = parse_f64(value)?,
        FieldId::ObjectiveTitle => quest.objective.title = value.to_string(),
        FieldId::RewardGold => quest.rewards.gold = parse_i64(value)?,
        FieldId::RewardXp => quest.rewards.xp = parse_i64(value)?,
        FieldId::RewardItems => quest.rewards.items = lines(value),
        FieldId::LinesIntro => quest.lines_intro = lines(value),
        FieldId::LinesProgress => quest.lines_progress = lines(value),
        FieldId::LinesComplete => quest.lines_complete = lines(value),
        FieldId::LinesLocked => quest.lines_locked = lines(value),
        FieldId::Next => quest.next = value.to_string(),
        FieldId::Requires => quest.requires = lines(value),
        FieldId::ParleyPrompt
        | FieldId::ParleyExample
        | FieldId::ParleyHint
        | FieldId::ParleyExpect
        | FieldId::ParleyPartial => {
            let parley = quest.parley.get_or_insert_with(Parley::default);
            match binding.field {
                FieldId::ParleyPartial => parley.partial = parse_bool(value)?,
                FieldId::ParleyExpect => {
                    let report = parley.report.get_or_insert_with(Report::default);
                    report.expect = parse_expects(value)?;
                }
                _ => {
                    let report = parley.report.get_or_insert_with(Report::default);
                    match binding.field {
                        FieldId::ParleyPrompt => report.prompt = value.to_string(),
                        FieldId::ParleyExample => report.example = value.to_string(),
                        FieldId::ParleyHint => report.hint = value.to_string(),
                        _ => unreachable!(),
                    }
                }
            }
        }
        FieldId::StagesJson => {
            if value.is_empty() {
                quest.stages.clear();
            } else {
                let stages: Vec<Objective> = serde_json::from_str(value)
                    .map_err(|err| format!("JSON de stages inválido: {err}"))?;
                quest.stages = stages;
            }
        }
        other => return Err(format!("campo {other:?} não se aplica a quests")),
    }
    Ok(())
}

fn apply_choice(
    binding: &ChoiceFieldBinding,
    value: &str,
    state: &mut VaelState,
    panel: &mut PanelState,
) -> Result<(), String> {
    match (binding.panel, binding.field) {
        (PanelId::Items, FieldId::Kind) => {
            let stack_max = state.items.stack_max;
            let resource_cap = state.items.resource_cap;
            let item = state
                .items
                .items
                .iter_mut()
                .find(|item| item.id == binding.key)
                .ok_or_else(|| format!("item `{}` não existe", binding.key))?;
            item.kind = value.to_string();
            match value {
                "resource" => item.stack = Some(resource_cap),
                "item" if item.stack.is_some_and(|stack| stack > stack_max) => {
                    item.stack = Some(stack_max);
                }
                _ => {}
            }
        }
        (PanelId::Landmarks, FieldId::Biome) => {
            let landmark = state
                .landmarks
                .landmarks
                .iter_mut()
                .find(|landmark| landmark.name == binding.key)
                .ok_or_else(|| format!("marco `{}` não existe", binding.key))?;
            landmark.biome = value.to_string();
        }
        (PanelId::Quests, FieldId::Biome) => {
            let quest = state
                .quest_mut(&binding.key)
                .ok_or_else(|| format!("quest `{}` não existe", binding.key))?;
            quest.biome = value.to_string();
        }
        (PanelId::Quests, FieldId::ObjectiveType) => {
            let quest = state
                .quest_mut(&binding.key)
                .ok_or_else(|| format!("quest `{}` não existe", binding.key))?;
            quest.objective.objective_type = value.to_string();
        }
        (PanelId::Dialogues, FieldId::File) => {
            if value.is_empty() {
                panel.dialogue_file = None;
                panel.dialogue_node = None;
            } else {
                panel.dialogue_file = Some(value.to_string());
                panel.dialogue_node = state
                    .dialogues
                    .get(value)
                    .and_then(|dialogue| dialogue.nodes.keys().next().cloned());
                panel.dialogue_choice = 0;
            }
        }
        (PanelId::Quests, FieldId::QuestFile) => {
            if value.is_empty() {
                panel.quest_file = None;
                panel.quest = None;
            } else {
                panel.quest_file = Some(value.to_string());
                panel.quest = state
                    .quest_files
                    .iter()
                    .find(|file| file.file == value)
                    .and_then(|file| file.quests.first().map(|quest| quest.id.clone()));
            }
        }
        _ => return Err("combobox sem efeito neste campo".to_string()),
    }
    Ok(())
}

fn parse_i64(value: &str) -> Result<i64, String> {
    value
        .parse()
        .map_err(|_| format!("`{value}` não é um inteiro"))
}

fn parse_f64(value: &str) -> Result<f64, String> {
    value
        .parse()
        .map_err(|_| format!("`{value}` não é um número"))
}

fn parse_opt_positive(value: &str) -> Result<Option<i64>, String> {
    if value.is_empty() {
        return Ok(None);
    }
    let number = parse_i64(value)?;
    if number <= 0 {
        Ok(None)
    } else {
        Ok(Some(number))
    }
}

fn parse_opt_string(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn parse_bool(value: &str) -> Result<bool, String> {
    match value.to_lowercase().as_str() {
        "" | "0" | "false" | "nao" | "não" => Ok(false),
        "1" | "true" | "sim" => Ok(true),
        other => Err(format!("`{other}` não é sim/não")),
    }
}

fn lines(value: &str) -> Vec<String> {
    value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn parse_effects(value: &str) -> Result<BTreeMap<String, f64>, String> {
    let mut effects = BTreeMap::new();
    for line in lines(value) {
        let (key, number) = line
            .split_once(':')
            .or_else(|| line.split_once('='))
            .ok_or_else(|| format!("`{line}`: usa `efeito:valor`"))?;
        let key = key.trim();
        if key.is_empty() {
            return Err(format!("`{line}`: efeito vazio"));
        }
        let number: f64 = number
            .trim()
            .parse()
            .map_err(|_| format!("`{line}`: valor não numérico"))?;
        effects.insert(key.to_string(), number);
    }
    Ok(effects)
}

/// `kind:id` por linha; `kind` é `criatura`, `item` ou `local`.
fn parse_expects(value: &str) -> Result<Vec<Expect>, String> {
    let mut expects = Vec::new();
    for line in lines(value) {
        let (kind, id) = line
            .split_once(':')
            .ok_or_else(|| format!("`{line}`: usa `kind:id`"))?;
        let kind = kind.trim();
        let id = id.trim();
        if kind.is_empty() || id.is_empty() {
            return Err(format!("`{line}`: kind/id vazio"));
        }
        expects.push(Expect {
            kind: kind.to_string(),
            id: id.to_string(),
            ..Default::default()
        });
    }
    Ok(expects)
}

//! Widgets e estado dos painéis do Vael.
//!
//! Cada painel é uma janela do dock com uma lista à esquerda e um formulário à
//! direita. O formulário é reconstruído quando o `PanelState` ou o `VaelState`
//! mudam; os commits de texto/combobox mutam o estado e disparam a
//! reconstrução.
//!
//! Os helpers recebem `&mut World` (não `Commands`): o build inicial da janela
//! e os rebuilds dos sistemas usam o mesmo caminho.

use bevy::prelude::*;
use jackdaw_feathers::button::{ButtonOperatorCall, ButtonProps, ButtonVariant, button};
use jackdaw_feathers::combobox::{ComboBoxOptionData, combobox_with_selected};
use jackdaw_feathers::scroll::{scroll_area, scrollbar};
use jackdaw_feathers::text_edit::{TextEditProps, text_edit};
use jackdaw_feathers::tokens;

/// Painel dono de um campo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelId {
    Items,
    Landmarks,
    Progression,
    Dialogues,
    Quests,
}

/// Campo editável. O mesmo `FieldId` pode aplicar-se a structs diferentes;
/// o `PanelId` desfaz a ambiguidade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldId {
    // Itens
    Label,
    Kind,
    Order,
    Stack,
    Desc,
    Slot,
    ShopBuy,
    ShopSell,
    ShopLabel,
    UseHeal,
    UseCure,
    WeaponDamage,
    WeaponSpeed,
    WeaponRange,
    WeaponGrip,
    Requires,
    RequiresText,
    Effects,
    // Marcos
    Name,
    Biome,
    Radius,
    Survey,
    Pilgrim,
    // Progressão
    XpBase,
    XpGrowth,
    XpSkillPoints,
    SongSeedEvery,
    Excludes,
    KillXpLine,
    // Diálogos
    Start,
    File,
    NodeId,
    NodeSpeaker,
    NodeText,
    NodeVoice,
    NodePortrait,
    ChoiceText,
    ChoiceGoto,
    // Quests
    QuestId,
    QuestFile,
    QuestSearch,
    Npc,
    Title,
    QuestPortrait,
    QuestVoice,
    ObjectiveType,
    ObjectiveTarget,
    ObjectiveCount,
    ObjectiveRadius,
    ObjectiveTitle,
    RewardGold,
    RewardXp,
    RewardItems,
    LinesIntro,
    LinesProgress,
    LinesComplete,
    LinesLocked,
    Next,
    ParleyPrompt,
    ParleyExample,
    ParleyHint,
    ParleyExpect,
    ParleyPartial,
    StagesJson,
}

/// Ligação de um campo de texto ao estado.
#[derive(Component, Clone, Debug)]
pub struct TextFieldBinding {
    pub panel: PanelId,
    pub field: FieldId,
    /// Chave do objeto (id do item/quest, nome do marco, nó do diálogo).
    pub key: String,
    /// Índice auxiliar (escolha, linha de kill_xp).
    pub sub: usize,
}

/// Ligação de um combobox ao estado.
#[derive(Component, Clone, Debug)]
pub struct ChoiceFieldBinding {
    pub panel: PanelId,
    pub field: FieldId,
    pub key: String,
    pub sub: usize,
}

/// Seleção e estado de UI de todos os painéis.
#[derive(Resource, Default, Clone)]
pub struct PanelState {
    pub item: Option<String>,
    pub landmark: Option<String>,
    pub skill: Option<String>,
    pub quest_file: Option<String>,
    pub quest: Option<String>,
    pub quest_search: String,
    pub dialogue_file: Option<String>,
    pub dialogue_node: Option<String>,
    pub dialogue_choice: usize,
    /// Objeto cujo botão "Apagar" já foi premido uma vez.
    pub delete_armed: Option<String>,
    pub status: String,
}

impl PanelState {
    pub fn selected(&self, panel: PanelId) -> Option<&String> {
        match panel {
            PanelId::Items => self.item.as_ref(),
            PanelId::Landmarks => self.landmark.as_ref(),
            PanelId::Progression => self.skill.as_ref(),
            PanelId::Dialogues => self.dialogue_node.as_ref(),
            PanelId::Quests => self.quest.as_ref(),
        }
    }

    pub fn select(&mut self, panel: PanelId, key: Option<String>) {
        match panel {
            PanelId::Items => self.item = key,
            PanelId::Landmarks => self.landmark = key,
            PanelId::Progression => self.skill = key,
            PanelId::Dialogues => {
                self.dialogue_node = key;
                self.dialogue_choice = 0;
            }
            PanelId::Quests => self.quest = key,
        }
        self.delete_armed = None;
    }
}

/// Marca uma raiz de painel já preenchida pelo menos uma vez.
#[derive(Component)]
pub struct PanelPopulated;

/// Raiz de um painel dentro da janela do dock.
pub fn panel_root(world: &mut World, parent: Entity, marker: impl Component) -> Entity {
    world
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: px(0),
                bottom: px(0),
                left: px(0),
                right: px(0),
                flex_direction: FlexDirection::Column,
                overflow: Overflow::clip(),
                ..default()
            },
            ChildOf(parent),
            marker,
        ))
        .id()
}

/// Linha de topo: título, ações e estado.
pub fn toolbar(world: &mut World, parent: Entity) -> Entity {
    world
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(tokens::SPACING_SM),
                row_gap: px(tokens::SPACING_XS),
                flex_wrap: FlexWrap::Wrap,
                padding: UiRect::axes(px(tokens::SPACING_MD), px(tokens::SPACING_XS)),
                flex_shrink: 0.0,
                ..default()
            },
            ChildOf(parent),
        ))
        .id()
}

/// Texto simples no painel.
pub fn label(world: &mut World, parent: Entity, text: impl Into<String>, muted: bool) {
    world.spawn((
        Text::new(text.into()),
        TextFont {
            font_size: tokens::TEXT_SIZE,
            ..default()
        },
        TextColor(if muted {
            tokens::TEXT_MUTED_COLOR.into()
        } else {
            tokens::TEXT_PRIMARY
        }),
        ChildOf(parent),
    ));
}

pub fn heading(world: &mut World, parent: Entity, text: impl Into<String>) {
    world.spawn((
        Text::new(text.into()),
        TextFont {
            font_size: tokens::TEXT_SIZE_LG,
            ..default()
        },
        TextColor(tokens::TEXT_PRIMARY),
        ChildOf(parent),
    ));
}

/// Corpo com lista à esquerda e formulário à direita.
pub fn body(world: &mut World, parent: Entity) -> (Entity, Entity) {
    let row = world
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                flex_grow: 1.0,
                min_width: px(0),
                min_height: px(0),
                ..default()
            },
            ChildOf(parent),
        ))
        .id();
    let list = scroll_column(world, row, px(250));
    let form = scroll_column(world, row, percent(100));
    (list, form)
}

/// Coluna com scroll vertical e barra.
pub fn scroll_column(world: &mut World, parent: Entity, width: Val) -> Entity {
    let grow = width == percent(100);
    let row = world
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                width,
                flex_grow: if grow { 1.0 } else { 0.0 },
                flex_shrink: if grow { 1.0 } else { 0.0 },
                min_width: px(0),
                min_height: px(0),
                ..default()
            },
            ChildOf(parent),
        ))
        .id();
    let list = world
        .spawn((
            scroll_area(),
            Node {
                flex_direction: FlexDirection::Column,
                flex_grow: 1.0,
                min_width: px(0),
                overflow: Overflow {
                    x: OverflowAxis::Clip,
                    y: OverflowAxis::Scroll,
                },
                row_gap: px(2),
                padding: UiRect::all(px(tokens::SPACING_XS)),
                ..default()
            },
            ChildOf(row),
        ))
        .id();
    world.spawn((scrollbar(list), ChildOf(row)));
    list
}

/// Botão de ação (op sem parâmetros ou com pares chave/valor).
pub fn action_button(
    world: &mut World,
    parent: Entity,
    text: impl Into<String>,
    operator: &'static str,
    params: &[(&'static str, String)],
) {
    let props = ButtonProps::new(text);
    if params.is_empty() {
        world.spawn((button(props.call_operator(operator)), ChildOf(parent)));
    } else {
        let mut call = ButtonOperatorCall::new(operator);
        for (key, value) in params {
            call = call.with_param(*key, value.clone());
        }
        world.spawn((button(props), call, ChildOf(parent)));
    }
}

/// Botão de lista; `selected` pinta o item atual.
pub fn list_button(
    world: &mut World,
    parent: Entity,
    text: impl Into<String>,
    selected: bool,
    operator: &'static str,
    id: &str,
) {
    let variant = if selected {
        ButtonVariant::Primary
    } else {
        ButtonVariant::Default
    };
    world.spawn((
        button(ButtonProps::new(text).align_left().with_variant(variant)),
        ButtonOperatorCall::new(operator).with_param("id", id.to_string()),
        ChildOf(parent),
    ));
}

/// Botão de lista por índice (escolhas de diálogo).
pub fn index_button(
    world: &mut World,
    parent: Entity,
    text: impl Into<String>,
    selected: bool,
    operator: &'static str,
    index: usize,
) {
    let variant = if selected {
        ButtonVariant::Primary
    } else {
        ButtonVariant::Default
    };
    world.spawn((
        button(ButtonProps::new(text).align_left().with_variant(variant)),
        ButtonOperatorCall::new(operator).with_param("index", index.to_string()),
        ChildOf(parent),
    ));
}

/// Linha etiqueta + controlo; devolve o slot do controlo.
pub fn field_row(world: &mut World, parent: Entity, text: impl Into<String>) -> Entity {
    let row = world
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(tokens::SPACING_SM),
                width: percent(100),
                min_width: px(0),
                ..default()
            },
            ChildOf(parent),
        ))
        .id();
    world.spawn((
        Node {
            width: px(150),
            min_width: px(150),
            ..default()
        },
        Text::new(text.into()),
        TextFont {
            font_size: tokens::TEXT_SIZE_SM,
            ..default()
        },
        TextColor(tokens::TEXT_MUTED_COLOR.into()),
        ChildOf(row),
    ));
    world
        .spawn((
            Node {
                flex_grow: 1.0,
                min_width: px(0),
                ..default()
            },
            ChildOf(row),
        ))
        .id()
}

/// Campo de texto ligado ao estado. `numeric` ativa o filtro de número.
pub fn text_field(
    world: &mut World,
    parent: Entity,
    label: impl Into<String>,
    binding: TextFieldBinding,
    value: impl Into<String>,
    numeric: bool,
    multiline: bool,
) {
    let control = field_row(world, parent, label);
    let mut props = TextEditProps::default()
        .with_default_value(value)
        .allow_empty();
    if numeric {
        props = props.numeric_int();
    }
    if multiline {
        props = props.multiline().grow();
    }
    world.spawn((text_edit(props), binding, ChildOf(control)));
}

/// Campo de texto para `f64` (aceita casas decimais).
pub fn float_field(
    world: &mut World,
    parent: Entity,
    label: impl Into<String>,
    binding: TextFieldBinding,
    value: impl Into<String>,
) {
    let control = field_row(world, parent, label);
    let props = TextEditProps::default()
        .with_default_value(value)
        .allow_empty()
        .numeric_f32();
    world.spawn((text_edit(props), binding, ChildOf(control)));
}

/// Campo de texto sem a linha de etiqueta, para composição em linhas próprias.
pub fn raw_text_field(
    world: &mut World,
    parent: Entity,
    binding: TextFieldBinding,
    value: impl Into<String>,
    numeric: bool,
    multiline: bool,
) {
    let mut props = TextEditProps::default()
        .with_default_value(value)
        .allow_empty();
    if numeric {
        props = props.numeric_int();
    }
    if multiline {
        props = props.multiline().grow();
    }
    world.spawn((text_edit(props), binding, ChildOf(parent)));
}

/// Combobox ligado ao estado. `options` é `(label, value)`.
pub fn choice_field(
    world: &mut World,
    parent: Entity,
    label: impl Into<String>,
    binding: ChoiceFieldBinding,
    options: Vec<(String, String)>,
    selected: usize,
) {
    let control = field_row(world, parent, label);
    let options: Vec<ComboBoxOptionData> = options
        .into_iter()
        .map(|(label, value)| ComboBoxOptionData::new(label).with_value(value))
        .collect();
    world.spawn((
        combobox_with_selected(options, selected),
        binding,
        ChildOf(control),
    ));
}

/// Separador visual entre secções.
pub fn separator(world: &mut World, parent: Entity) {
    world.spawn((
        Node {
            height: px(1),
            width: percent(100),
            margin: UiRect::vertical(px(tokens::SPACING_SM)),
            ..default()
        },
        BackgroundColor(tokens::BORDER_COLOR.into()),
        ChildOf(parent),
    ));
}

/// Mensagem de estado no topo do painel.
pub fn status_line(world: &mut World, parent: Entity, text: impl Into<String>) {
    world.spawn((
        Node {
            padding: UiRect::axes(px(tokens::SPACING_MD), px(2)),
            ..default()
        },
        Text::new(text.into()),
        TextFont {
            font_size: tokens::TEXT_SIZE_SM,
            ..default()
        },
        TextColor(tokens::TEXT_MUTED_COLOR.into()),
        ChildOf(parent),
    ));
}

/// Linha de botões.
pub fn button_row(world: &mut World, parent: Entity) -> Entity {
    world
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(tokens::SPACING_XS),
                flex_wrap: FlexWrap::Wrap,
                ..default()
            },
            ChildOf(parent),
        ))
        .id()
}

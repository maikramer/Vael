//! Painel de itens (`data/items.json`).

use bevy::prelude::*;

use vael_content::ItemDef;

use crate::state::VaelState;
use crate::ui::{
    FieldId, PanelId, PanelPopulated, PanelState, TextFieldBinding, action_button, body,
    button_row, choice_field, float_field, heading, label, list_button, separator, status_line,
    text_field, toolbar,
};

#[derive(Component)]
pub struct ItemsRoot;

pub fn rebuild(
    mut commands: Commands,
    roots: Query<Entity, With<ItemsRoot>>,
    populated: Query<(), With<PanelPopulated>>,
    state: Res<VaelState>,
    panel: Res<PanelState>,
) {
    let stale = state.is_changed() || panel.is_changed();
    super::rebuild_panel(
        &mut commands,
        roots.iter(),
        &populated,
        stale,
        &state,
        &panel,
        build,
    );
}

pub fn build(world: &mut World, root: Entity, state: &VaelState, panel: &PanelState) {
    let top = toolbar(world, root);
    heading(world, top, "Itens");
    action_button(
        world,
        top,
        "Novo",
        "vael.content.add",
        &[("panel", "items".to_string())],
    );
    action_button(
        world,
        top,
        "Gravar",
        "vael.content.save",
        &[("panel", "items".to_string())],
    );
    action_button(world, top, "Recarregar", "vael.content.reload", &[]);
    action_button(world, top, "Validar", "vael.content.validate", &[]);
    if !panel.status.is_empty() {
        status_line(world, root, panel.status.clone());
    }
    let (list, form) = body(world, root);

    let mut items: Vec<&ItemDef> = state.items.items.iter().collect();
    items.sort_by_key(|item| (item.order, item.id.clone()));
    let selected = panel
        .item
        .as_deref()
        .or_else(|| items.first().map(|item| item.id.as_str()))
        .map(str::to_string);
    for item in &items {
        list_button(
            world,
            list,
            format!("{} · {}", item.label, item.id),
            selected.as_deref() == Some(item.id.as_str()),
            "vael.content.select",
            &item.id,
        );
    }

    let Some(item) = items
        .iter()
        .find(|item| selected.as_deref() == Some(item.id.as_str()))
        .copied()
    else {
        label(world, form, "Seleciona um item.", true);
        return;
    };
    let key = item.id.clone();
    let bind = |field: FieldId| TextFieldBinding {
        panel: PanelId::Items,
        field,
        key: key.clone(),
        sub: 0,
    };

    read_id(world, form, "id", &item.id);
    text_field(
        world,
        form,
        "label",
        bind(FieldId::Label),
        &item.label,
        false,
        false,
    );

    let kinds = ["item", "resource"];
    choice_field(
        world,
        form,
        "kind",
        crate::ui::ChoiceFieldBinding {
            panel: PanelId::Items,
            field: FieldId::Kind,
            key: key.clone(),
            sub: 0,
        },
        super::options(&kinds),
        super::selected_index(&kinds, &item.kind),
    );
    text_field(
        world,
        form,
        "order",
        bind(FieldId::Order),
        item.order.to_string(),
        true,
        false,
    );
    text_field(
        world,
        form,
        "stack (vazio = stack_max)",
        bind(FieldId::Stack),
        item.stack
            .map(|value| value.to_string())
            .unwrap_or_default(),
        true,
        false,
    );
    text_field(
        world,
        form,
        "desc",
        bind(FieldId::Desc),
        &item.desc,
        false,
        false,
    );
    text_field(
        world,
        form,
        "slot",
        bind(FieldId::Slot),
        &item.slot,
        false,
        false,
    );

    separator(world, form);
    label(world, form, "Loja", false);
    let shop = item.shop.clone().unwrap_or_default();
    text_field(
        world,
        form,
        "buy (vazio = não vende)",
        bind(FieldId::ShopBuy),
        shop.buy.map(|value| value.to_string()).unwrap_or_default(),
        true,
        false,
    );
    text_field(
        world,
        form,
        "sell (vazio = não compra)",
        bind(FieldId::ShopSell),
        shop.sell.map(|value| value.to_string()).unwrap_or_default(),
        true,
        false,
    );
    text_field(
        world,
        form,
        "label da loja",
        bind(FieldId::ShopLabel),
        &shop.label,
        false,
        false,
    );

    separator(world, form);
    label(world, form, "Uso", false);
    let use_item = item.use_item.clone().unwrap_or_default();
    float_field(
        world,
        form,
        "heal",
        bind(FieldId::UseHeal),
        super::compact(use_item.heal),
    );
    action_button(
        world,
        form,
        if use_item.cure {
            "curar: sim"
        } else {
            "curar: não"
        },
        "vael.content.toggle",
        &[
            ("panel", "items".to_string()),
            ("id", key.clone()),
            ("field", "use_cure".to_string()),
        ],
    );

    separator(world, form);
    label(world, form, "Arma", false);
    let weapon = item.weapon.clone().unwrap_or_default();
    float_field(
        world,
        form,
        "damage",
        bind(FieldId::WeaponDamage),
        super::compact(weapon.damage),
    );
    float_field(
        world,
        form,
        "speed",
        bind(FieldId::WeaponSpeed),
        super::compact(weapon.speed),
    );
    float_field(
        world,
        form,
        "range",
        bind(FieldId::WeaponRange),
        super::compact(weapon.range),
    );
    text_field(
        world,
        form,
        "grip",
        bind(FieldId::WeaponGrip),
        &weapon.grip,
        false,
        false,
    );

    separator(world, form);
    label(world, form, "Requisito e efeitos", false);
    text_field(
        world,
        form,
        "requires",
        bind(FieldId::Requires),
        &item.requires,
        false,
        false,
    );
    text_field(
        world,
        form,
        "requires_text",
        bind(FieldId::RequiresText),
        &item.requires_text,
        false,
        false,
    );
    text_field(
        world,
        form,
        "effects (efeito:valor por linha)",
        bind(FieldId::Effects),
        super::effects_text(&item.effects),
        false,
        true,
    );

    separator(world, form);
    let row = button_row(world, form);
    action_button(
        world,
        row,
        if panel.delete_armed.as_deref() == Some(item.id.as_str()) {
            "Confirmar apagar"
        } else {
            "Apagar"
        },
        "vael.content.delete",
        &[("panel", "items".to_string()), ("id", key.clone())],
    );
}

fn read_id(world: &mut World, parent: Entity, label_text: &str, value: &str) {
    let row = crate::ui::field_row(world, parent, label_text);
    label(world, row, value.to_string(), false);
}

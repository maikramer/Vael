//! Vael: MCP operators — manipulação e input expostos pelo `jd mcp`.
//!
//! Uma extensão não pode acrescentar tools ao servidor MCP do Jackdaw, mas
//! qualquer operador registado aparece em `list_operators`/`call_operator`.
//! Estes operadores são a metade "editor" do MCP; os agentes usam-nos pelo
//! `jd mcp` ou pelo plugin MCP do projeto.

use std::path::PathBuf;

use bevy::prelude::*;
use jackdaw_api::prelude::DefaultArea;
use jackdaw_extension::op::{report_to_caller, warn_caller};
use jackdaw_extension::prelude::*;
use jackdaw_extension::selection::Selection;
use serde_json::{Value, json};
use bevy_ext_bridge_protocol::{
    BridgeClient, DEFAULT_TOOLS_PORT, METHOD_KEY, is_listening,
};

pub const EXTENSION_ID: &str = "vael_mcp_jackdaw";
pub const WINDOW_ID: &str = "vael.mcp";

fn port() -> u16 {
    std::env::var("VAEL_TOOLS_PORT")
        .ok()
        .and_then(|raw| raw.trim().parse().ok())
        .unwrap_or(DEFAULT_TOOLS_PORT)
}

fn call_game(world: &mut World, method: &str, params: Value) -> OperatorResult {
    let port = port();
    if !is_listening(port) {
        warn_caller(world, format!("vael.mcp: bridge desligado na porta {port}"));
        return OperatorResult::Cancelled;
    }
    match BridgeClient::localhost(port).call(method, params) {
        Ok(value) => {
            report_to_caller(world, value.to_string());
            OperatorResult::Finished
        }
        Err(error) => {
            warn_caller(world, format!("vael.mcp: {error:#}"));
            OperatorResult::Cancelled
        }
    }
}

#[operator(
    id = "vael.mcp.add_group",
    label = "Vael: add group MCP",
    allows_undo = true
)]
fn add_group(_params: In<OperatorParameters>, world: &mut World) -> OperatorResult {
    let name = "MCP".to_string();
    let Some(parent) = world
        .get_resource::<Selection>()
        .and_then(Selection::primary)
    else {
        warn_caller(world, "vael.mcp.add_group: seleciona primeiro um pai");
        return OperatorResult::Cancelled;
    };
    let settings = CallOperatorSettings {
        creates_history_entry: true,
        ..default()
    };
    match world
        .operator("entity.add.group")
        .param("name", name.clone())
        .param("parent", parent)
        .settings(settings)
        .call()
    {
        Ok(OperatorResult::Finished) => {
            report_to_caller(world, json!({ "group": name, "parent": parent.to_bits() }).to_string());
            OperatorResult::Finished
        }
        Ok(other) => other,
        Err(error) => {
            warn_caller(world, format!("vael.mcp.add_group: {error:#}"));
            OperatorResult::Cancelled
        }
    }
}

#[operator(
    id = "vael.mcp.input_f3",
    label = "Vael: profiler F3",
    allows_undo = false
)]
fn input_f3(_params: In<OperatorParameters>, world: &mut World) -> OperatorResult {
    call_game(
        world,
        METHOD_KEY,
        json!({ "key": "F3", "state": "click", "hold_ms": 120 }),
    )
}

#[operator(
    id = "vael.mcp.screenshot",
    label = "Vael: screenshot",
    allows_undo = false
)]
fn screenshot(_params: In<OperatorParameters>, world: &mut World) -> OperatorResult {
    let port = port();
    if !is_listening(port) {
        warn_caller(world, format!("vael.mcp.screenshot: bridge desligado na porta {port}"));
        return OperatorResult::Cancelled;
    }
    match BridgeClient::localhost(port)
        .screenshot(3000)
        .or_else(|_| BridgeClient::localhost(port).legacy_screenshot(3000))
    {
        Ok((png, _)) => {
            let dir = std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(".jackdaw");
            let _ = std::fs::create_dir_all(&dir);
            let path = dir.join("vael-mcp-shot.png");
            if let Err(error) = std::fs::write(&path, &png) {
                warn_caller(world, format!("vael.mcp.screenshot: {error}"));
                return OperatorResult::Cancelled;
            }
            report_to_caller(
                world,
                json!({ "path": path.display().to_string(), "bytes": png.len() }).to_string(),
            );
            OperatorResult::Finished
        }
        Err(error) => {
            warn_caller(world, format!("vael.mcp.screenshot: {error:#}"));
            OperatorResult::Cancelled
        }
    }
}

fn build(world: &mut World, parent: Entity) {
    world
        .spawn((
            ChildOf(parent),
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: px(6),
                padding: UiRect::all(px(8)),
                ..default()
            },
        ))
        .with_children(|root| {
            root.spawn((
                Text::new("Vael: MCP operators"),
                TextFont {
                    font_size: bevy::text::FontSize::Px(14.0),
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
            root.spawn((
                Text::new(
                    "Estes operadores aparecem no `jd mcp` (list_operators/call_operator).",
                ),
                TextFont {
                    font_size: bevy::text::FontSize::Px(11.0),
                    ..default()
                },
                TextColor(Color::srgb(0.7, 0.7, 0.7)),
            ));
            root.spawn(button(
                ButtonProps::new("Add group MCP").call_operator("vael.mcp.add_group"),
            ));
            root.spawn(button(
                ButtonProps::new("Screenshot").call_operator("vael.mcp.screenshot"),
            ));
            root.spawn(button(
                ButtonProps::new("Profiler F3").call_operator("vael.mcp.input_f3"),
            ));
        });
}

#[derive(Default)]
pub struct McpOperatorsExtension;

impl JackdawExtension for McpOperatorsExtension {
    fn id(&self) -> String {
        EXTENSION_ID.to_string()
    }

    fn label(&self) -> String {
        "Vael: MCP operators".to_string()
    }

    fn description(&self) -> String {
        "Operadores de seleção/manipulação/input do Vael para o jd mcp".to_string()
    }

    fn register(&self, ctx: &mut ExtensionRegistrar<'_>) {
        ctx.register_operator::<AddGroupOp>();
        ctx.register_operator::<InputF3Op>();
        ctx.register_operator::<ScreenshotOp>();
        ctx.register_menu_entry::<AddGroupOp>(TopLevelMenu::Tools);
        ctx.register_window(
            WindowDescriptor::new(WINDOW_ID)
                .with_name("Vael: MCP operators")
                .with_default_area(DefaultArea::RightSidebar)
                .with_priority(43)
                .with_build(|window| {
                    let parent = window.target_entity();
                    build(window.world_mut(), parent);
                }),
        );
    }
}

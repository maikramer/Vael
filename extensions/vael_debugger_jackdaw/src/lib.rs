//! Vael: Debugger — play/pause/step e atalhos de QA pelo bridge do jogo.
//!
//! O PIE do Jackdaw não expõe à extensão o canal do jogo; estes operadores
//! falam com o `bevy_ext_bridge_runtime` por BRP lateral. São finos de propósito:
//! a lógica de pausa/step vive no runtime, aqui só há botões/operadores.

use bevy::prelude::*;
use jackdaw_api::prelude::DefaultArea;
use jackdaw_extension::op::{report_to_caller, warn_caller};
use jackdaw_extension::prelude::*;
use serde_json::{Value, json};
use bevy_ext_bridge_protocol::{
    BridgeClient, DEFAULT_TOOLS_PORT, METHOD_KEY, METHOD_PAUSE, METHOD_PLAY, METHOD_STEP,
    is_listening,
};

pub const EXTENSION_ID: &str = "vael_debugger_jackdaw";
pub const WINDOW_ID: &str = "vael.debug";

fn port() -> u16 {
    std::env::var("VAEL_TOOLS_PORT")
        .ok()
        .and_then(|raw| raw.trim().parse().ok())
        .unwrap_or(DEFAULT_TOOLS_PORT)
}

fn call_game(world: &mut World, method: &str, params: Value) -> OperatorResult {
    let port = port();
    if !is_listening(port) {
        warn_caller(
            world,
            format!("vael.debug: bridge desligado na porta {port}"),
        );
        return OperatorResult::Cancelled;
    }
    let client = BridgeClient::localhost(port);
    match client.call(method, params) {
        Ok(value) => {
            report_to_caller(world, value.to_string());
            OperatorResult::Finished
        }
        Err(error) => {
            warn_caller(world, format!("vael.debug: {error:#}"));
            OperatorResult::Cancelled
        }
    }
}

#[operator(id = "vael.debug.play", label = "Vael: play", allows_undo = false)]
fn play(_params: In<OperatorParameters>, world: &mut World) -> OperatorResult {
    call_game(world, METHOD_PLAY, json!({}))
}

#[operator(id = "vael.debug.pause", label = "Vael: pause", allows_undo = false)]
fn pause(_params: In<OperatorParameters>, world: &mut World) -> OperatorResult {
    call_game(world, METHOD_PAUSE, json!({}))
}

#[operator(id = "vael.debug.step", label = "Vael: step", allows_undo = false)]
fn step(_params: In<OperatorParameters>, world: &mut World) -> OperatorResult {
    call_game(world, METHOD_STEP, json!({ "frames": 1 }))
}

#[operator(
    id = "vael.debug.toggle_profiler",
    label = "Vael: profiler F3",
    allows_undo = false
)]
fn toggle_profiler(_params: In<OperatorParameters>, world: &mut World) -> OperatorResult {
    call_game(
        world,
        METHOD_KEY,
        json!({ "key": "F3", "state": "click", "hold_ms": 120 }),
    )
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
                Text::new("Vael: Debugger"),
                TextFont {
                    font_size: bevy::text::FontSize::Px(14.0),
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
            root.spawn((
                Text::new(
                    "Fala com o runtime do jogo na porta 15702 (VAEL_TOOLS_PORT). \
                     Sem jogo, o operador avisa em warnings.",
                ),
                TextFont {
                    font_size: bevy::text::FontSize::Px(11.0),
                    ..default()
                },
                TextColor(Color::srgb(0.7, 0.7, 0.7)),
            ));
            for (caption, op) in [
                ("Play", "vael.debug.play"),
                ("Pause", "vael.debug.pause"),
                ("Step 1 frame", "vael.debug.step"),
                ("Profiler F3", "vael.debug.toggle_profiler"),
            ] {
                root.spawn(button(ButtonProps::new(caption).call_operator(op)));
            }
        });
}

#[derive(Default)]
pub struct DebuggerExtension;

impl JackdawExtension for DebuggerExtension {
    fn id(&self) -> String {
        EXTENSION_ID.to_string()
    }

    fn label(&self) -> String {
        "Vael: Debugger".to_string()
    }

    fn description(&self) -> String {
        "Play/pause/step e atalhos de QA do Vael via runtime".to_string()
    }

    fn register(&self, ctx: &mut ExtensionRegistrar<'_>) {
        ctx.register_operator::<PlayOp>();
        ctx.register_operator::<PauseOp>();
        ctx.register_operator::<StepOp>();
        ctx.register_operator::<ToggleProfilerOp>();
        ctx.register_menu_entry::<PlayOp>(TopLevelMenu::Tools);
        ctx.register_menu_entry::<StepOp>(TopLevelMenu::Tools);
        ctx.register_window(
            WindowDescriptor::new(WINDOW_ID)
                .with_name("Vael: Debugger")
                .with_default_area(DefaultArea::RightSidebar)
                .with_priority(42)
                .with_build(|window| {
                    let parent = window.target_entity();
                    build(window.world_mut(), parent);
                }),
        );
    }
}

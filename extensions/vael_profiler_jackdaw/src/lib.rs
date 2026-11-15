//! Vael: Profiler — painel do Jackdaw com os dados do `bevy_ext_bridge_runtime`.
//!
//! A extensão não bloqueia o editor: uma thread de background faz polling ao
//! bridge do jogo e publica um snapshot num `Arc<Mutex<_>>`; o sistema de
//! refresh só lê esse snapshot.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use jackdaw_api::prelude::DefaultArea;
use jackdaw_extension::AppState;
use jackdaw_extension::op::report_to_caller;
use jackdaw_extension::prelude::*;
use serde_json::{Value, json};
use bevy_ext_bridge_protocol::{
    BridgeClient, DEFAULT_TOOLS_PORT, METHOD_FRAME_STATS, METHOD_MEMORY, is_listening,
};

pub const EXTENSION_ID: &str = "vael_profiler_jackdaw";
pub const WINDOW_ID: &str = "vael.profiler";

#[derive(Clone, Default)]
struct Snapshot {
    frame: Option<Value>,
    memory: Option<Value>,
    error: Option<String>,
}

impl Snapshot {
    fn to_json(&self) -> String {
        json!({
            "frame": self.frame,
            "memory": self.memory,
            "error": self.error,
        })
        .to_string()
    }
}

#[derive(Resource, Clone)]
struct ProfilerState {
    snapshot: Arc<Mutex<Snapshot>>,
}

#[derive(Component)]
struct ProfilerStatus;

fn port() -> u16 {
    std::env::var("VAEL_TOOLS_PORT")
        .ok()
        .and_then(|raw| raw.trim().parse().ok())
        .unwrap_or(DEFAULT_TOOLS_PORT)
}

fn poll(stop: Arc<AtomicBool>, snapshot: Arc<Mutex<Snapshot>>) {
    let port = port();
    while !stop.load(Ordering::Relaxed) {
        let next = if is_listening(port) {
            let client = BridgeClient::localhost(port);
            match client.call(METHOD_FRAME_STATS, json!({})) {
                Ok(frame) => Snapshot {
                    frame: Some(frame),
                    memory: client.call(METHOD_MEMORY, json!({})).ok(),
                    error: None,
                },
                Err(error) => Snapshot {
                    frame: None,
                    memory: None,
                    error: Some(error.to_string()),
                },
            }
        } else {
            Snapshot {
                frame: None,
                memory: None,
                error: Some(format!("bridge desligado na porta {port}")),
            }
        };
        if let Ok(mut current) = snapshot.lock() {
            *current = next;
        }
        std::thread::sleep(std::time::Duration::from_millis(750));
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
                Text::new("Vael: Profiler"),
                TextFont {
                    font_size: bevy::text::FontSize::Px(14.0),
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
            root.spawn((
                Text::new("À espera do runtime do jogo…"),
                TextFont {
                    font_size: bevy::text::FontSize::Px(12.0),
                    ..default()
                },
                TextColor(Color::srgb(0.75, 0.75, 0.75)),
                ProfilerStatus,
            ));
            root.spawn(button(
                ButtonProps::new("Snapshot").call_operator("vael.profiler.snapshot"),
            ));
            root.spawn((
                Text::new("p95/hitches/RSS vêm de `vael-tools debug prof --tab frames`."),
                TextFont {
                    font_size: bevy::text::FontSize::Px(11.0),
                    ..default()
                },
                TextColor(Color::srgb(0.65, 0.65, 0.65)),
            ));
        });
}

fn refresh(state: Res<ProfilerState>, mut status: Query<&mut Text, With<ProfilerStatus>>) {
    let Ok(mut status) = status.single_mut() else {
        return;
    };
    let snapshot = state
        .snapshot
        .lock()
        .map(|snapshot| snapshot.clone())
        .unwrap_or_default();
    status.0 = if let Some(error) = snapshot.error {
        format!("sem sinal: {error}")
    } else {
        let frame = snapshot.frame.unwrap_or(Value::Null);
        let memory = snapshot.memory.unwrap_or(Value::Null);
        let p95 = frame.get("p95_ms").and_then(Value::as_f64).unwrap_or(0.0);
        let fps = frame.get("fps").and_then(Value::as_f64).unwrap_or(0.0);
        let hitches = frame
            .get("hitches")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        let rss = memory
            .get("rss_kb")
            .and_then(Value::as_u64)
            .map_or(0, |kb| kb / 1024);
        format!(
            "p95 {p95:.1} ms · {fps:.1} fps · {hitches} hitch(es) · RSS {rss} MB"
        )
    };
}

#[operator(
    id = "vael.profiler.snapshot",
    label = "Vael: profiler snapshot",
    allows_undo = false
)]
fn profiler_snapshot(_params: In<OperatorParameters>, world: &mut World) -> OperatorResult {
    let json = world
        .get_resource::<ProfilerState>()
        .and_then(|state| state.snapshot.lock().ok().map(|snapshot| snapshot.to_json()))
        .unwrap_or_else(|| "{\"error\":\"profiler sem estado\"}".to_string());
    report_to_caller(world, json);
    OperatorResult::Finished
}

pub struct ProfilerExtension {
    stop: Arc<AtomicBool>,
    snapshot: Arc<Mutex<Snapshot>>,
}

impl Default for ProfilerExtension {
    fn default() -> Self {
        Self {
            stop: Arc::new(AtomicBool::new(false)),
            snapshot: Arc::new(Mutex::new(Snapshot::default())),
        }
    }
}

impl JackdawExtension for ProfilerExtension {
    fn id(&self) -> String {
        EXTENSION_ID.to_string()
    }

    fn label(&self) -> String {
        "Vael: Profiler".to_string()
    }

    fn description(&self) -> String {
        "p95/FPS/hitches/RSS do runtime do Vael no editor".to_string()
    }

    fn register(&self, ctx: &mut ExtensionRegistrar<'_>) {
        ctx.register_operator::<ProfilerSnapshotOp>();
        ctx.register_menu_entry::<ProfilerSnapshotOp>(TopLevelMenu::Tools);
        ctx.register_window(
            WindowDescriptor::new(WINDOW_ID)
                .with_name("Vael: Profiler")
                .with_default_area(DefaultArea::RightSidebar)
                .with_priority(41)
                .with_build(|window| {
                    let parent = window.target_entity();
                    build(window.world_mut(), parent);
                }),
        );
    }

    fn app_plugin(&self) -> Option<Box<dyn Plugin>> {
        let stop = Arc::clone(&self.stop);
        let snapshot = Arc::clone(&self.snapshot);
        Some(Box::new(move |app: &mut App| {
            // O plugin closure tem de ser `Fn` (pode ser chamado mais de uma
            // vez); os Arcs são clonados dentro da chamada.
            let stop = Arc::clone(&stop);
            let snapshot = Arc::clone(&snapshot);
            app.insert_resource(ProfilerState {
                snapshot: Arc::clone(&snapshot),
            });
            app.add_systems(Update, refresh.run_if(in_state(AppState::Editor)));
            std::thread::spawn(move || poll(stop, snapshot));
        }))
    }

    fn unregister(&self, _world: &mut World, _entity: Entity) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

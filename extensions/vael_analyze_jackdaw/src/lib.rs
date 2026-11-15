//! Vael: Problems — painel do Jackdaw com o `analyze` do `vael_tools`.
//!
//! O Jackdaw não tem um registry de Problems público; a extensão tem a sua
//! dock window. O operador `vael.analyze.run` usa a mesma implementação do
//! `vael-tools analyze` e publica o JSON em `report_to_caller`, por isso o
//! `jd mcp` também o vê em `reports`.

use std::path::Path;

use bevy::prelude::*;
use jackdaw_api::prelude::DefaultArea;
use jackdaw_extension::AppState;
use jackdaw_extension::document::SceneFilePath;
use jackdaw_extension::op::{report_to_caller, warn_caller};
use jackdaw_extension::prelude::*;

use vael_tools::AnalyzeReport;

pub const EXTENSION_ID: &str = "vael_analyze_jackdaw";
pub const WINDOW_ID: &str = "vael.problems";

#[derive(Resource)]
struct Problems {
    report: Option<AnalyzeReport>,
}

impl Default for Problems {
    fn default() -> Self {
        Self { report: None }
    }
}

#[derive(Component)]
struct ProblemsStatus;

#[operator(id = "vael.analyze.run", label = "Vael: analyze", allows_undo = false)]
fn run_analyze(_params: In<OperatorParameters>, world: &mut World) -> OperatorResult {
    let path = world
        .get_resource::<SceneFilePath>()
        .and_then(|scene| scene.path.clone());
    let report = vael_tools::analyze(path.as_deref().map(Path::new));
    let json = serde_json::to_string(&report).unwrap_or_else(|error| {
        format!("{{\"error\":\"serializar analyze: {error}\"}}")
    });
    if let Some(error) = &report.error {
        warn_caller(world, format!("vael.analyze: {error}"));
    }
    if let Some(mut problems) = world.get_resource_mut::<Problems>() {
        problems.report = Some(report);
    }
    report_to_caller(world, json);
    OperatorResult::Finished
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
                Text::new("Vael: Problems"),
                TextFont {
                    font_size: bevy::text::FontSize::Px(14.0),
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
            root.spawn((
                Text::new("Sem análise. Corre `Analyze` ou `vael-tools analyze`."),
                TextFont {
                    font_size: bevy::text::FontSize::Px(12.0),
                    ..default()
                },
                TextColor(Color::srgb(0.75, 0.75, 0.75)),
                ProblemsStatus,
            ));
            root.spawn(button(
                ButtonProps::new("Analyze").call_operator("vael.analyze.run"),
            ));
            root.spawn((
                Text::new(
                    "O relatório completo vai para `reports` no jd mcp; \
                     a CLI headless usa o mesmo core.",
                ),
                TextFont {
                    font_size: bevy::text::FontSize::Px(11.0),
                    ..default()
                },
                TextColor(Color::srgb(0.65, 0.65, 0.65)),
            ));
        });
}

fn refresh(problems: Res<Problems>, mut status: Query<&mut Text, With<ProblemsStatus>>) {
    let Ok(mut status) = status.single_mut() else {
        return;
    };
    status.0 = match &problems.report {
        None => "Sem análise. Corre `Analyze` ou `vael-tools analyze`.".to_string(),
        Some(report) if report.error.is_some() => {
            format!("erro: {}", report.error.as_deref().unwrap_or("?"))
        }
        Some(report) => format!(
            "{} entidades, {} assets ({} em falta), {} scripts; {} warnings",
            report.entities.total,
            report.assets.references,
            report.assets.missing.len(),
            report.scripts.len(),
            report.warnings.len()
        ),
    };
}

#[derive(Default)]
pub struct AnalyzeExtension;

impl JackdawExtension for AnalyzeExtension {
    fn id(&self) -> String {
        EXTENSION_ID.to_string()
    }

    fn label(&self) -> String {
        "Vael: Problems".to_string()
    }

    fn description(&self) -> String {
        "Analyze headless do Vael no painel Problems do editor".to_string()
    }

    fn register(&self, ctx: &mut ExtensionRegistrar<'_>) {
        ctx.register_operator::<RunAnalyzeOp>();
        ctx.register_menu_entry::<RunAnalyzeOp>(TopLevelMenu::Tools);
        ctx.register_window(
            WindowDescriptor::new(WINDOW_ID)
                .with_name("Vael: Problems")
                .with_default_area(DefaultArea::BottomDock)
                .with_priority(40)
                .with_build(|window| {
                    let parent = window.target_entity();
                    build(window.world_mut(), parent);
                }),
        );
    }

    fn app_plugin(&self) -> Option<Box<dyn Plugin>> {
        // Closure, não um `impl Plugin`: o `jd` classificaria o crate como jogo.
        Some(Box::new(|app: &mut App| {
            app.init_resource::<Problems>();
            app.add_systems(Update, refresh.run_if(in_state(AppState::Editor)));
        }))
    }
}

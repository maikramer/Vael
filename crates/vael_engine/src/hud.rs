//! HUD do mundo — shim do monólito (exit5/G-2).
//!
//! Os widgets (compass/minimap/waypoint/elements/interact/menu/vitals/
//! markers/widgets) vivem no `bevy_ext_engine::hud` com os TypePaths
//! `viber::hud::…` congelados lá; a arte (`hud_assets`) já era do twin
//! desde a exit2/hubs. O caminho `viber::hud::*` resolve por aqui.

pub use bevy_ext_engine::hud::assets;
pub use bevy_ext_engine::hud::compass;
pub use bevy_ext_engine::hud::elements;
pub use bevy_ext_engine::hud::interact;
pub use bevy_ext_engine::hud::markers;
pub use bevy_ext_engine::hud::menu;
pub use bevy_ext_engine::hud::minimap;
pub use bevy_ext_engine::hud::vitals;
pub use bevy_ext_engine::hud::waypoint;
pub use bevy_ext_engine::hud::widgets;

pub use bevy_ext_engine::hud::*;

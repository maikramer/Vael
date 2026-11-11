//! Vael em Rust: fundação do jogo nativo sobre a engine Viber.
//!
//! O mundo e o conteúdo continuam a ser os ficheiros do projeto (`world.xml`,
//! JSON e, nesta fase, os scripts Lua). Este crate monta a App com o
//! comportamento de jogo da engine DESLIGADO (`gameplay: false`) e o host
//! Luau em `Compat`: as regras passam para Rust onda a onda, sem deixar o
//! jogo sem comportamento enquanto a transição decorre.

use std::path::PathBuf;

use bevy::prelude::*;

pub mod attach;
pub mod components;
pub mod content;
pub mod save;
pub mod ui_bind;

pub use attach::{VaelAttachPlugin, VaelScriptTags};
pub use components::{Inventory, PlayerVitals, QuestLog, VaelPlayer, XpState};
pub use content::{VaelContent, VaelContentPlugin};
pub use save::{VaelSave, VaelSavePlugin};
pub use ui_bind::{VaelHud, VaelUiPlugin};

/// Raiz do projeto Vael: a pasta com `config.yaml`, `data/`, `quests/`,
/// `locale/` e o `world.xml`. Resolvida no arranque; todos os loaders leem
/// daqui (nunca do cwd).
#[derive(Resource, Debug, Clone, Default, Reflect)]
#[reflect(Resource)]
pub struct VaelProjectRoot(pub PathBuf);

/// Plugin do jogo. A engine já montou a App ([`viber::app::build_app`]);
/// isto acrescenta apenas o que é do Vael.
pub struct VaelPlugin;

impl Plugin for VaelPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<VaelProjectRoot>()
            .init_resource::<VaelProjectRoot>()
            .add_plugins((
                VaelContentPlugin,
                VaelSavePlugin,
                VaelUiPlugin,
                VaelAttachPlugin,
            ));
    }
}

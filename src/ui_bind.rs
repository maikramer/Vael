//! HUD do Vael na UI declarativa da engine (Wave futura).
//!
//! O escritor de binds (`viber.ui.set`) ainda pertence a outra passagem.
//! Aqui fica só o recurso partilhado: quem escrever os binds publica o
//! estado do HUD em [`VaelHud`] e mantém-no barato enquanto o HUD declarativo
//! não substituir o do Lua.

use bevy::prelude::*;

/// Estado que o HUD do Vael vai publicar. Vazio de propósito nesta fundação.
#[derive(Debug, Clone, Copy, Default, Resource, Reflect)]
#[reflect(Resource)]
pub struct VaelHud;

/// Plugin de UI do Vael. Sem sistemas nesta passagem.
pub struct VaelUiPlugin;

impl Plugin for VaelUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VaelHud>().register_type::<VaelHud>();
    }
}

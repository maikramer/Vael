//! Texto localizado e balão do HUD — SEM intérprete de grafos.
//!
//! Os grafos de diálogo (`dialogues/*.json`) são CONTEÚDO e correm no jogo
//! (Lua, `scripts/townsfolk.lua`): o script lê o JSON com `viber.load_json`,
//! guarda o cursor em `viber.game()` e escreve com `viber.say`. A engine só
//! resolve chaves `@nome` (locale) e desenha o balão.
//!
//! Os contratos ([`ExternalSayPresenter`], [`localized`],
//! [`localized_lines`]) foram para o `bevy_ext_core` (onda exit2/luau) —
//! reexportados aqui para `vael_engine::dialogue::…` continuar a resolver. O
//! BALÃO nativo ([`show_balloon`]) cá fica: escreve no `HudBalloon` do HUD,
//! um tipo de serviço que o core não pode ter.

use bevy::prelude::*;

pub use bevy_engine_core::dialogue::{ExternalSayPresenter, localized, localized_lines};

/// Balão do HUD (mesma escrita de `viber.say`).
pub fn show_balloon(
    balloons: &mut Query<(&mut Visibility, &mut crate::hud::HudBalloon, &Children)>,
    texts: &mut Query<&mut Text>,
    body: &str,
    secs: f32,
) -> bool {
    let mut found = false;
    for (mut visibility, mut balloon, children) in balloons.iter_mut() {
        found = true;
        balloon.timer = secs;
        *visibility = Visibility::Visible;
        if let Some(child) = children.first()
            && let Ok(mut text) = texts.get_mut(*child)
        {
            text.0 = body.into();
        }
    }
    found
}

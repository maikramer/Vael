//! Pós-processamento — shim do monólito + o glue de integração.
//!
//! A LENTE vive no `bevy_ext_postfx::runtime` (exit5/G-2; o módulo inteiro
//! saiu — `PostFxPlugin`, drives, split-tone, key da lua, torch pool, aerial,
//! testes). O caminho `viber::postfx::*` reexporta-o; os TypePaths
//! `viber::postfx::` ficam congelados no crate.
//!
//! Glue que FICOU no monólito (o twin não vê os módulos de jogo):
//! - [`publish_cinematic_focus`] — copia o foco do playback de cinemáticas
//!   para [`PostFxCinematicFocus`], o recurso que o `drive_dof_focus` do twin
//!   lê (contrato documentado lá — "quem integrar copia o par por frame").

pub use bevy_ext_postfx::runtime::*;

use bevy::prelude::*;
use crate::cinematic::CinematicPlayback;

/// Copia o foco de DOF do playback de cinemáticas para o recurso do twin.
/// Sem cinemática a correr (ou sem foco), `None` — o `drive_dof_focus` volta
/// à distância câmara↔herói. Corre antes do [`drive_dof_focus`]? Não: o
/// registo é `.before(postfx::drive_dof_focus)` no `app.rs` do monólito.
pub fn publish_cinematic_focus(
    cinematic: Option<Res<CinematicPlayback>>,
    mut focus: ResMut<PostFxCinematicFocus>,
) {
    focus.0 = cinematic.as_deref().and_then(|c| c.focus());
}

/// Adaptador do `postfx:` do `config.yaml` (`bevy_ext_engine::config` — com
/// serde) para o contrato de dados crus do twin. Os campos são os MESMOS; o
/// twin não depende de serde.
pub fn resolve_profile(config: Option<&bevy_ext_engine::config::PostFxConfig>) {
    let converted = config.map(|c| PostFxConfig {
        disabled: c.disabled.clone(),
        ssao: c.ssao.clone(),
    });
    bevy_ext_postfx::runtime::resolve_profile(converted.as_ref());
}

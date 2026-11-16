//! UI declarativa — SHIM sobre o crate `bevy_ext_ui` (exit3/wave-c).
//!
//! A árvore inteira (`collect`/`runtime`/`style`/`script`/`tree`/`widgets`/
//! `modal`/`bind`… — ~13k lin) vive no crate; os caminhos `viber::ui::*` e os
//! TypePaths (`viber::ui::…`, congelados lá) continuam a resolver e os tipos
//! são OS MESMOS no jogo. O que aqui ficou é o GLUE que o crate não pode ter
//! (os TODO(viber-exit) dele nomeiam o anfitrião):
//!
//! * [`publish_ui_world_state`] — o retrato do mundo ([`host::UiWorldState`])
//!   que os binds do HUD leem (relógio/chuva/zona + `display-name`);
//! * [`publish_ui_font`] — a fonte do HUD (`UiFont`) para tooltips e listas;
//!
//! Os contratos que o crate definiu no `ui::host` (ScriptInteraction,
//! ScriptEventQueue, HudHidden, MinimapRange, SceneSource…) são reexportados
//! PELOS módulos donos do monólito (`luau`/`hud`/`scene`) — um só tipo no
//! jogo, o TypePath histórico fica congelado no crate.

pub use bevy_ext_ui::ui::*;

use bevy::prelude::*;

/// Retrato do MUNDO para os binds — publicado a cada frame ANTES do
/// `UiSet::Collect`.
///
/// Espelha o que o collect antigo lia directamente (`DayCycleState`,
/// `WeatherState`, `CurrentBiome`, `BiomeRegions`): o crate é consumidor e
/// não conhece estes tipos. `zone_display` é o `display-name` autoral DA
/// região activa (vazio = o título deriva do id, como sempre).
#[allow(clippy::type_complexity)]
pub fn publish_ui_world_state(
    day: Option<Res<crate::worldsys::DayCycleState>>,
    weather: Option<Res<crate::worldsys::WeatherState>>,
    biome: Option<Res<crate::ambient::CurrentBiome>>,
    biomes: Option<Res<crate::worldsys::BiomeRegions>>,
    mut world: ResMut<host::UiWorldState>,
) {
    world.minute_of_day = day.as_deref().map(|day| day.minute_of_day);
    world.rain = weather.as_deref().map(|w| w.rain).unwrap_or(0.0);
    world.zone = biome.as_deref().and_then(|b| b.id.clone());
    world.zone_display = world
        .zone
        .as_deref()
        .and_then(|id| {
            biomes.as_deref()?.list.iter().find(|r| r.id == *id).and_then(|r| {
                let name = r.display_name.trim();
                (!name.is_empty()).then(|| name.to_string())
            })
        })
        .unwrap_or_default();
}

/// Espelha a fonte do HUD ([`crate::hud::HudAssets`]) no contrato do crate.
///
/// Sem o recurso ainda (o HUD constrói-o preguiçosamente), a UI usa a fonte
/// omissa — o mesmo efeito do antigo `Option<Res<HudAssets>>`.
pub fn publish_ui_font(hud: Option<Res<crate::hud::HudAssets>>, mut commands: Commands) {
    if let Some(hud) = hud.as_deref() {
        commands.insert_resource(host::UiFont(hud.font.clone()));
    }
}



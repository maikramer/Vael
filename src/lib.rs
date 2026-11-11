//! Vael em Rust: fundação do jogo nativo sobre a engine Viber.
//!
//! O mundo e o conteúdo continuam a ser os ficheiros do projeto (`world.xml`,
//! JSON e, nesta fase, os scripts Lua). Este crate monta a App com o
//! comportamento de jogo da engine DESLIGADO (`gameplay: false`) e o host
//! Luau em `Compat`: as regras passam para Rust onda a onda, sem deixar o
//! jogo sem comportamento enquanto a transição decorre.
//!
//! Nesta passagem o estado corre em **shadow mode**: [`VaelStateSyncPlugin`]
//! espelha o `viber.game()`, [`VaelRulesShadowPlugin`] simula as regras vitais
//! ao lado do Lua e [`VaelHudPlugin`] publica os binds do HUD ao lado do Lua,
//! só a ler. A tabela Lua continua a ser a fonte de verdade.
//!
//! O comportamento de HUD que não é diálogo (`scripts/ui/hud.lua`) já é do
//! [`VaelHudUiPlugin`]: linhas de "outras missões", clique do diário, TTL da
//! barra de chefe e soco do combo.
//!
//! Com `VAEL_RULES_RUST=1`, o [`VaelRulesAuthorityPlugin`] passa a ser o dono
//! do tick de vitais/status/morte: lê o `viber.game()` depois do Luau, simula
//! com o mesmo `shadow_tick` e escreve de volta as chaves que possui.

use std::path::PathBuf;

use bevy::prelude::*;

pub mod attach;
pub mod components;
pub mod content;
pub mod gear;
pub mod hud;
pub mod hud_ui;
pub mod rules_authority;
pub mod rules_shadow;
pub mod save;
pub mod state_sync;
pub mod ui_bind;
pub mod vault;
pub mod xp;

pub use attach::{VaelAttachPlugin, VaelScriptTags};
pub use components::{Inventory, PlayerVitals, QuestLog, VaelPlayer, XpState};
pub use content::{VaelContent, VaelContentPlugin};
pub use gear::{GearRules, GearState, Stats, VaelGearPlugin, Weapon};
pub use hud::{VaelHudConfig, VaelHudPlugin};
pub use hud_ui::VaelHudUiPlugin;
pub use rules_authority::{VaelRulesAuthority, VaelRulesAuthorityPlugin, VaelRulesConfig};
pub use rules_shadow::{VaelRulesShadow, VaelRulesShadowPlugin};
pub use save::{VaelSave, VaelSavePlugin};
pub use state_sync::VaelStateSyncPlugin;
pub use ui_bind::{VaelHud, VaelUiPlugin};
pub use vault::VaelVaultPlugin;
pub use xp::{VaelXpPlugin, XpGain, XpRules};

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
                // Cofre em Rust: `viber.vael` instalado no `Startup` (o
                // `rules.lua` fica só com os embrulhos finos).
                VaelVaultPlugin,
                // XP/nível/pontos de talento: acrescenta funções à mesma
                // tabela `viber.vael`, depois do cofre.
                VaelXpPlugin,
                // Equipamento/passivas: `viber.vael.gear_*` e `player_stats`,
                // depois do cofre e do XP.
                VaelGearPlugin,
                VaelUiPlugin,
                // Shadow mode: espelho do `viber.game()` e HUD ao lado do Lua.
                VaelStateSyncPlugin,
                // Simulação paralela das regras vitais do `rules.lua`.
                VaelRulesShadowPlugin,
                // Autoridade (opt-in `VAEL_RULES_RUST=1`): o tick de
                // vitais/status/morte passa para Rust.
                VaelRulesAuthorityPlugin,
                VaelHudPlugin,
                // HUD comportamental que o `scripts/ui/hud.lua` deixou.
                VaelHudUiPlugin,
                VaelAttachPlugin,
            ));
    }
}

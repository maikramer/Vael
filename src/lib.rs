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

// Sistemas Bevy exprimem queries grandes e as VMs da UI são construídas campo
// a campo; estes três lints são ruído de estilo neste código, não contratos.
#![allow(
    clippy::field_reassign_with_default,
    clippy::too_many_arguments,
    clippy::type_complexity
)]

use std::path::PathBuf;

use bevy::prelude::*;

pub mod attach;
pub mod camera;
pub mod clock;
pub mod combat;
pub mod components;
pub mod content;
pub mod economy;
pub mod environment;
pub mod flora;
pub mod gear;
#[cfg(feature = "hanabi")]
pub mod hanabi_fx;
pub mod harvest;
pub mod hud;
pub mod hud_ui;
pub mod lighting;
pub mod player;
pub mod quest;
pub mod quest_events;
pub mod rules_authority;
pub mod rules_shadow;
pub mod save;
pub mod settings;
pub mod speech;
pub mod state_sync;
pub mod stats;
pub mod travel;
pub mod ui;
pub mod ui_bind;
pub mod vault;
pub mod world_text;
pub mod xp;

pub use attach::{VaelAttachPlugin, VaelScriptTags};
pub use clock::{ClockSnapshot, VaelClock, VaelClockPlugin};
pub use combat::{CombatKv, CombatRules, DamageResult, VaelCombatPlugin};
pub use components::{Inventory, PlayerVitals, QuestLog, VaelPlayer, XpState};
pub use content::{VaelContent, VaelContentPlugin};
pub use economy::{
    CraftRules, CraftStatus, ShopRules, ShopStatus, TradeAction, TradeReason, VaelEconomyPlugin,
};
pub use environment::{VaelEnvironmentPlugin, elements_enabled};
pub use flora::VaelFloraPlugin;
pub use gear::{GearRules, GearState, Stats, VaelGearPlugin, Weapon};
pub use harvest::VaelHarvestPlugin;
pub use hud::{VaelHudConfig, VaelHudPlugin};
pub use hud_ui::VaelHudUiPlugin;
pub use lighting::VaelShadowPlugin;
pub use quest::{QuestRules, VaelQuestPlugin};
pub use quest_events::VaelQuestEventsPlugin;
pub use rules_authority::{VaelRulesAuthority, VaelRulesAuthorityPlugin, VaelRulesConfig};
pub use rules_shadow::{VaelRulesShadow, VaelRulesShadowPlugin};
pub use save::{VaelSave, VaelSavePlugin};
pub use speech::VaelSpeechPlugin;
pub use state_sync::VaelStateSyncPlugin;
pub use stats::{AttributeRules, StatEngine, StatSnapshot};
pub use travel::VaelTravelPlugin;
pub use ui_bind::{VaelHud, VaelHudResourcePlugin};
pub use vault::VaelVaultPlugin;
pub use world_text::{VaelWorldTextPlugin, WorldTextEvent};
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
        // Um erro de COMANDO do ECS (entidade despawnada a meio de uma
        // transição de cena) não pode abortar o jogo: o `bevy_elements`
        // (nuvens) e as câmaras transitórias do carregamento disparam-no. O
        // handler `warn` loga e segue — um bug real continua visível no log,
        // mas um frame transitório deixa de matar o processo.
        app.set_error_handler(bevy::ecs::error::warn);
        app.add_plugins(jackdaw_runtime::JackdawPlugin);
        app.register_type::<VaelProjectRoot>()
            .init_resource::<VaelProjectRoot>()
            // Definições da cena (ambiente/PostFX/flora) antes dos plugins
            // que as leem no `build`.
            .add_plugins(settings::VaelSettingsPlugin)
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
                // Ciclo da quest (estado/progresso/aceitar/entregar/
                // recompensas): `viber.vael.quest_*`, depois do equipamento.
                VaelQuestPlugin,
                // Combate (guarda/parry, dano, cura, i-frames):
                // `viber.vael.guard`/`damage_player`/`heal_player`/
                // `grant_iframes`, depois das quests.
                VaelCombatPlugin,
                // Relógio/chuva: `WorldClock`/`Weather` do `bevy_elements`
                // (ou `DayCycleState`/`WeatherState` da engine, sem ele):
                // `viber.vael.clock_*`, depois do combate.
                VaelClockPlugin,
                VaelHudResourcePlugin,
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
        // Loja/forja (preços, tetos e negociação): `shop_*`/`craft_*`, depois
        // do relógio. Em chamada à parte: a tupla do Bevy pára nos 15.
        app.add_plugins((
            VaelEconomyPlugin,
            // Waypoint/marcadores do HUD (o `travel.lua` deixou de ser o
            // publicador): blips, beacons, pin e a seta da quest.
            ui::waypoint::VaelWaypointPlugin,
            // Colheita/interação, viagem/marcos e eventos de quest: cada
            // módulo instala o seu plugin sem tocar no `lib.rs`.
            VaelHarvestPlugin,
            VaelTravelPlugin,
            VaelQuestEventsPlugin,
            // Texto flutuante no mundo (`bevy_rich_text3d`): dano, cura,
            // coleta e combo. Substitui o pool 2D da engine — o `main.rs`
            // arranca com `damage_numbers_2d: false`.
            VaelWorldTextPlugin,
            // Falas no mundo (`bevy_speech_bubbles`): o `viber.say` deixa de
            // escrever no balão do HUD e passa a ser um balão sobre o NPC.
            VaelSpeechPlugin::<viber::recipes::spawn::OrbitCamera>::default(),
            // Árvores procedurais (`bevy_flora`) na entrada da cidade: teste
            // A/B contra os GLB, atrás do gate `VAEL_FLORA`.
            VaelFloraPlugin,
            // Ambiente vivo (`bevy_elements`): céu astronómico, clima,
            // nuvens, chuva/neve e relâmpagos no lugar do `<Sky>`/
            // `<DayCycle>`/`<Weather>` da engine. `VAEL_ELEMENTS=0` desliga.
            VaelEnvironmentPlugin,
            // Sombras do sol a 15 Hz no modo `day-night` (`shadow-refresh-hz`
            // do `<DayCycle>`); fora desse modo é um no-op.
            VaelShadowPlugin,
            // Player/câmara do Vael: substitui os sistemas da engine
            // (`player_camera:false`) e mantém o `Player` sincronizado só
            // para a animação enquanto essa camada não é portada.
            player::VaelPlayerPlugin,
            camera::VaelCameraPlugin,
        ));
        #[cfg(feature = "hanabi")]
        app.add_plugins(hanabi_fx::VaelHanabiPlugin);
    }
}

//! Vael em Rust: fundação do jogo nativo sobre a engine Viber.
//!
//! O mundo e o conteúdo são os ficheiros do projeto (`world.bsn`, JSON e os
//! scripts Lua transitórios). Este crate monta a App com o comportamento de
//! jogo da engine DESLIGADO (`gameplay: false`) e o host Luau em `Compat`:
//! as regras passam para Rust onda a onda, sem deixar o jogo sem
//! comportamento enquanto a transição decorre.
//!
//! O [`VaelStateSyncPlugin`] espelha o `viber.game()` e o [`VaelHudPlugin`]
//! publica os binds do HUD. A autoridade de vitais/status/morte é o
//! **default** desde a Onda 4a ([`VaelRulesAuthorityPlugin`];
//! `VAEL_RULES_RUST=0` devolve o tick ao Lua). O [`VaelRulesShadowPlugin`]
//! (`VAEL_RULES_SHADOW=1`) continua a ser a ferramenta de paridade.
//!
//! O comportamento de HUD que não é diálogo é do [`VaelHudUiPlugin`]: linhas
//! de "outras missões", clique do diário, TTL da barra de chefe e soco do
//! combo.

// Sistemas Bevy exprimem queries grandes e as VMs da UI são construídas campo
// a campo; estes três lints são ruído de estilo neste código, não contratos.
#![allow(
    clippy::field_reassign_with_default,
    clippy::too_many_arguments,
    clippy::type_complexity
)]

use std::path::PathBuf;

use bevy::prelude::*;

pub mod animation;
pub mod app;
pub mod boat;
pub mod bridge_ready;
pub mod camera;
pub mod cinematic_trigger;
pub mod clock;
pub mod combat;
mod compat;
pub mod components;
pub mod content;
pub mod economy;
pub mod editor_link;
pub mod environment;
pub mod flora;
pub mod rocks;
mod fauna;
pub mod gear;
pub mod grass;
pub mod ground;
pub mod snow_caps;
#[cfg(feature = "hanabi")]
pub mod hanabi_fx;
mod breakables;
pub mod harvest;
pub mod head_look;
pub mod hud;
pub mod hud_ui;
pub mod input;
pub mod jump_trace;
pub mod lights;
pub mod live_edit;
pub mod perf_diag;
pub mod player;
#[cfg(feature = "playtest")]
pub mod playtest_support;
pub mod primitives;
pub mod qa;
pub mod quest;
pub mod quest_events;
pub mod respawn;
pub mod rules_authority;
pub mod rules_shadow;
pub mod save;
pub mod save_native;
pub mod script_host;
pub mod settings;
pub mod speech;
pub mod spring;
pub mod state_sync;
pub mod stats;
pub mod travel;
pub mod ui;
pub mod vault;
pub mod world_text;
pub mod xp;

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
pub use input::VaelInputPlugin;
pub use lights::VaelPracticalLightsPlugin;
pub use primitives::VaelPrimitivesPlugin;
pub use quest::{QuestRules, VaelQuestPlugin};
pub use quest_events::VaelQuestEventsPlugin;
pub use rules_authority::{VaelRulesAuthority, VaelRulesAuthorityPlugin, VaelRulesConfig};
pub use rules_shadow::{VaelRulesShadow, VaelRulesShadowPlugin};
pub use save::{VaelSave, VaelSavePlugin};
pub use speech::VaelSpeechPlugin;
pub use state_sync::VaelStateSyncPlugin;
pub use stats::{AttributeRules, StatEngine, StatSnapshot};
pub use travel::VaelTravelPlugin;
pub use ui::waypoint::{VaelAttachPlugin, VaelScriptTags};
pub use vault::VaelVaultPlugin;
pub use world_text::{VaelWorldTextPlugin, WorldTextEvent};
pub use xp::{VaelXpPlugin, XpGain, XpRules};

/// Raiz do projeto Vael: a pasta com `config.yaml`, `data/`, `quests/`,
/// `locale/` e o `world.bsn`. Resolvida no arranque; todos os loaders leem
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
                // Espelho do `viber.game()`; o HUD tem o seu próprio plugin.
                VaelStateSyncPlugin,
                // Simulação paralela das regras vitais do `rules.lua`
                // (ferramenta de paridade, `VAEL_RULES_SHADOW=1`).
                VaelRulesShadowPlugin,
                // Autoridade (default; `VAEL_RULES_RUST=0` devolve o tick
                // de vitais/status/morte ao Lua).
                VaelRulesAuthorityPlugin,
                // Morte que nasce no tick da autoridade (DoT): larga a bolsa
                // e toca `game_over` como o `start_death` do Lua.
                respawn::VaelRespawnPlugin,
                VaelHudPlugin,
                // HUD comportamental que o `scripts/ui/hud.lua` deixou.
                VaelHudUiPlugin,
                VaelAttachPlugin,
            ));
        // Loja/forja (preços, tetos e negociação): `shop_*`/`craft_*`, depois
        // do relógio. Em chamada à parte: a tupla do Bevy pára nos 15.
        app.add_plugins((
            save_native::VaelNativeSavePlugin,
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
            // Objetos de primitivas autorais (`<Composition>`): troca o
            // fallback StandardMaterial do spawner pelo mesh/material/colisor
            // do `bevy_ext_gradmesh` (o mesmo crate do preview do Jackdaw).
            // `VAEL_PRIMITIVES=standard` desliga.
            VaelPrimitivesPlugin,
            // Ambiente vivo (`bevy_elements`): céu astronómico, clima,
            // nuvens, chuva/neve e relâmpagos no lugar do `<Sky>`/
            // `<DayCycle>`/`<Weather>` da engine. `VAEL_ELEMENTS=0` desliga.
            VaelEnvironmentPlugin,
            // Player/câmara do Vael: substitui os sistemas da engine
            // (`player_camera:false`) e mantém o `Player` sincronizado só
            // para a animação enquanto essa camada não é portada.
            // Input sobre o `jackdaw_input` (escreve o `ActionState`).
            VaelInputPlugin,
            player::VaelPlayerPlugin,
            jump_trace::JumpTracePlugin,
            camera::VaelCameraPlugin,
        ));
        // Gatilho de área das cinemáticas (`CinematicTrigger` autorado no
        // Jackdaw): substitui o `scripts/cinematic-trigger.lua`. Fora da tupla
        // (15 é o teto do Bevy).
        app.add_plugins(cinematic_trigger::VaelCinematicTriggerPlugin);
        // Relva GPU (`bevy_grass`): campos por bioma, bake das máscaras a
        // partir do `TerrainRuntime`, vento do `WeatherState` e trampling.
        // Vive aqui, não na engine. Fora da tupla (15 é o teto do Bevy).
        app.add_plugins(grass::VaelGrassPlugin);
        app.add_plugins(rocks::QuarryPlugin);
        app.add_plugins(breakables::BreakablesPlugin);
        app.add_plugins(fauna::FaunaPlugin);
        // Neve, rastos e sombra de nuvem no chão do `.ahgt`.
        app.add_plugins(ground::VaelGroundPlugin);
        // Tampas de neve volumétricas em telhados e props (+ abrigo do chão).
        app.add_plugins(snow_caps::VaelSnowCapsPlugin);
        // Barcos a remos: casco procedural, flutuação (engine), embarque/
        // saída com E e condução com WASD. Fora da tupla acima (15 é o teto).
        app.add_plugins(boat::VaelBoatPlugin);
        // Luzes práticas (`bevy_lights`): as luzes de fogo viram candidatas de
        // um pool orçamentado com flicker e chama, em vez da lavagem quente
        // uniforme do ganho nocturno global. `VAEL_PRACTICAL_LIGHTS=0`
        // devolve o comportamento anterior.
        app.add_plugins(lights::VaelPracticalLightsPlugin);
        // Play do Jackdaw: `SceneNodeId` nas entidades para as edições ao vivo.
        app.add_plugins(editor_link::VaelEditorLinkPlugin);
        // QA: cenários (`qa/scenarios/*.json`) aplicados no arranque e a
        // cada gravação do ficheiro; `viber.vael.qa_*` para a bridge.
        app.add_plugins(qa::VaelQaPlugin);
        // Edições de atributos/specs/props do Jackdaw → re-spawn/patch.
        app.add_plugins(live_edit::VaelLiveEditPlugin);
        // `AnimationSet` autorado → rig do `jackdaw_animation_runtime`.
        app.add_plugins(animation::VaelAnimationPlugin);
        // Look-at da cabeça: NPCs/criaturas viram-se para o herói no alcance.
        app.add_plugins(head_look::VaelHeadLookPlugin);
        app.add_plugins(perf_diag::VaelPerfDiagPlugin);
        // Spring bones: mola na crina dos lobos (demo do crate).
        app.add_plugins(spring::VaelSpringPlugin);
        // Invariantes do jogo para o harness `bevy_playtest`: só durante um
        // cenário (sem o env o plugin nem entra, incluindo no build normal).
        #[cfg(feature = "playtest")]
        if std::env::var_os("BEVY_PLAYTEST_SCENARIO").is_some() {
            app.add_plugins(playtest_support::VaelPlaytestPlugin);
        }
        #[cfg(feature = "hanabi")]
        app.add_plugins(hanabi_fx::VaelHanabiPlugin);
    }
}

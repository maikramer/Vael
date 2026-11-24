//! Colheita e interação com recursos do mundo.
//!
//! Porta o comportamento de `scripts/game/harvest.lua`: lê os props autorais
//! (`destructible`, `resource-kind`, `resource-yield`) de qualquer entidade
//! com [`ScriptProps`], escolhe o alvo mais próximo dentro do alcance
//! efetivo, publica o prompt `[J]` pela [`ScriptInteraction`] da engine e,
//! a cada golpe, desconta a durabilidade. No último golpe credita o cofre e o
//! XP pelas regras puras de [`crate::vault`]/[`crate::xp`], marca
//! `VaelSave::harvested` com uma chave estável (`Nome@proveniência`) e
//! derruba/substitui/despawna a entidade.
//!
//! Machado e picareta partilham foco, entrada e contacto nos dois modos.
//! O gate escolhe o controlador dos recursos colhidos à mão.
//!
//! - o substituto (`berry-bush-bare`) leva a escala autoral (`replace-scale`)
//!   aplicada depois do spawn do protótipo (o pedido da engine não transporta
//!   escala — ver [`ReplacementScale`]);
//! - o progresso de quests `collect` é pedido ao [`QuestEventQueue`]; sem
//!   ele, o `sync_collect` do `rules.tick` (Lua) apanha o cofre no frame
//!   seguinte.

mod chips;
mod mining;
mod regrowth;
pub(crate) mod timber;

use bevy::ecs::message::Messages;
use bevy::prelude::*;
use bevy_ext_core::{SfxClip, SfxEvent};
use bevy_ext_world::xml::values::parse_color;
use bevy_flora::{
    BuckTree, FellTree, Felled, PooledLook, PooledTree, TreeLanded, TreeLog, TreeShake, TreeShakes,
    TreeStump,
};
pub(crate) use chips::Fracture as MiningFracture;
use chips::{HitQueue, ToolHit};
use serde_json::{Value as Json, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};
use vael_engine::feedback::DamageNumberEvent;
use vael_engine::interact::{InteractionFocus, range_scale};
use vael_engine::luau::components::{ScriptInteraction, ScriptProps};
use vael_engine::menus::MenusOpen;
use vael_engine::physics_fx::Falling;
use vael_engine::player::Player;
use vael_engine::recipes::spawn::{PendingPrototypeSpawn, PendingScriptSpawns};
use vael_engine::scene::SceneSource;
use vael_engine::scene_loader::{CurrentScene, SceneEpoch};
use vael_engine::ui::ScriptBindValue;
use vael_engine::ui::script::UiScriptBinds;
use vael_engine::ui::widgets::UiFocusedInput;
use vael_engine::vitals::Health;

use crate::VaelProjectRoot;
use crate::compat::env_enabled;
use crate::components::{Inventory, PlayerVitals, XpState};
use crate::content::VaelContent;
use crate::quest_events::{QuestEvent, QuestEventQueue};
use crate::save::VaelSave;
use crate::vault::{VaultRules, item_key, vael_fn, vael_table, vault_add, vault_take};
use crate::xp::{
    KEY_LEVEL, KEY_SKILL_POINTS, KEY_XP, KEY_XP_NEXT, XP_KEYS, XpGain, XpMap, XpRules, add_xp,
};

/// Variável de ambiente que liga a colheita em Rust (`1`/`true`/`on`/`yes`).
pub const RUST_ENV: &str = "VAEL_HARVEST_RUST";
/// Chave do `vael.game()` que avisa o Lua de que a colheita é do Rust.
pub const RUST_GATE_KEY: &str = "vael_harvest_rust";
/// Período da verificação do gate guardado no save (s).
pub const GATE_PERIOD_SECS: f32 = 0.5;
/// Período da varredura de props já colhidos no save (s).
pub const REAP_PERIOD_SECS: f32 = 1.0;

/// Alcance autorado por omissão (`RANGE_DEFAULT` do Lua).
pub const RANGE_DEFAULT: f32 = 3.5;
/// Intervalo mínimo entre golpes (s).
pub const SWING_COOLDOWN: f32 = 0.4;
/// XP por prop partido (`HARVEST_XP` do Lua).
pub const HARVEST_XP: f64 = 30.0;
/// Cor do popup quando o prop não autora uma válida.
pub const DEFAULT_POPUP_COLOR: [f32; 3] = [1.0, 211.0 / 255.0, 90.0 / 255.0];
/// Cor do popup de "bolsa cheia" (`#ffb347`).
pub const FULL_POPUP_COLOR: [f32; 3] = [1.0, 179.0 / 255.0, 71.0 / 255.0];
/// O `pick` sem `resource-kind` é uma berry (fallback do Lua).
pub const PICK_FALLBACK_KIND: &str = "berry";
/// Frutas do template quando o `resource-yield` falta/inválido.
pub const PICK_FALLBACK_YIELD: i64 = 3;
/// Substitutos permitidos (lista FECHADA, como no Lua).
pub const REPLACEMENTS: &[&str] = &["berry-bush-bare"];

/// Comprimento de cada toro ao retalhar uma árvore deitada (m).
pub const LOG_LENGTH: f32 = 2.2;
/// Raio mínimo dos ramos que viram toro.
pub const LOG_MIN_RADIUS: f32 = 0.08;
/// Collider do toco depois do corte: raio e meia-altura (m). O tronco deitado
/// do `bevy_flora` não colide; o toco continua sólido e curto.
pub const STUMP_RADIUS: f32 = 0.45;
pub const STUMP_HALF_HEIGHT: f32 = 0.35;
/// Alcance do prompt "Cortar em toros" (m, autorado; escala do `interact`).
pub const BUCK_RANGE: f32 = 3.0;
/// Intervalo entre golpes de retalhar (s).
pub const BUCK_COOLDOWN: f32 = 0.5;
/// Distância horizontal alvo à superfície de contacto para mineração e toros (m).
pub const TOOL_STANDOFF: f32 = 0.88;
/// Distância horizontal alvo para derrubar árvores em pé (+10 cm para dar mais curso ao machado).
pub const TREE_CHOP_STANDOFF: f32 = TOOL_STANDOFF + 0.10;
/// Banda morta (m) em torno de [`TREE_CHOP_STANDOFF`] para o herói não oscilar entre golpes.
const TREE_CHOP_STANDOFF_DEADZONE: f32 = 0.03;
/// Ajuste radial máximo num único golpe (m).
const TOOL_MAX_LUNGE: f32 = 0.68;

/// Estilo de quebra autorado (`break-style`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Reflect, Default)]
pub enum BreakStyle {
    /// A árvore tomba (`break-style: fall`).
    Fall,
    /// Colher frutas (`break-style: pick`).
    Pick,
    /// Pedra/tronco que estilhaça.
    Shatter,
    /// Qualquer outro valor (o default do Lua).
    #[default]
    Burst,
}

impl BreakStyle {
    /// `(c["break-style"] or "burst"):lower()`.
    pub fn from_raw(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "fall" => Self::Fall,
            "pick" => Self::Pick,
            "shatter" => Self::Shatter,
            _ => Self::Burst,
        }
    }
}

/// Dados de colheita de uma entidade, já fora do parser de props.
#[derive(Debug, Clone, PartialEq, Component, Reflect)]
#[reflect(Component)]
pub struct Harvestable {
    /// Golpes autorados (`hits`), já saneados (>= 1).
    pub hits: i64,
    /// Golpes que faltam para partir.
    pub remaining: i64,
    pub style: BreakStyle,
    /// Recurso creditado no cofre (`resource-kind`; `berry` no fallback).
    pub kind: String,
    /// Quantidade creditada (`resource-yield`, inteira >= 1).
    pub amount: i64,
    /// Texto do popup de quebra (`popup-text`).
    pub popup: String,
    /// Cor do popup em RGB linear-ish (3 ou 6 dígitos hex).
    pub popup_color: [f32; 3],
    /// Alcance AUTORADO; o efetivo é `vael_engine::interact::scaled_range`.
    pub range: f32,
    /// Altura do corte acima da base (`cut-height`, m): o `bevy_flora` corta
    /// a árvore aí e o toco fica com essa altura.
    pub cut_height: f32,
    /// Substituto autorado (`replacement`), ainda por validar.
    pub replacement: String,
    /// Escala do substituto (`replace-scale`, 0,05..20).
    pub replace_scale: f32,
    /// `Name` da entidade (vazio quando não tem).
    pub name: String,
    /// Chave persistente: `Nome@proveniência` (ver [`stable_key`]).
    pub stable_key: Option<String>,
}

impl Harvestable {
    /// Lê os props autorais. `None` quando a entidade não é colhível.
    ///
    /// `stable_key` é calculado pelo chamador ([`stable_key`]) para manter a
    /// proveniência fora do parser.
    pub fn from_props(
        props: &[(String, String)],
        name: &str,
        stable_key: Option<String>,
    ) -> Option<Self> {
        let raw = prop(props, "destructible");
        let raw_kind = prop(props, "resource-kind");
        if raw.is_empty() && raw_kind.is_empty() {
            return None;
        }
        let components = parse_components(raw);
        let style = BreakStyle::from_raw(
            components
                .get("break-style")
                .map(String::as_str)
                .unwrap_or("burst"),
        );
        let replacement = components.get("replacement").cloned().unwrap_or_default();
        // As berry_bush do Scatter perderam o `resource-kind`; o estilo `pick`
        // (ou o substituto `berry-bush-bare`) diz o que são.
        let is_berry = style == BreakStyle::Pick || replacement == "berry-bush-bare";
        let kind = if raw_kind.is_empty() && is_berry {
            PICK_FALLBACK_KIND.to_string()
        } else {
            raw_kind.to_string()
        };
        let amount = match parse_number(prop(props, "resource-yield")) {
            Some(value) if value.is_finite() && value >= 1.0 => value.floor() as i64,
            _ => {
                if is_berry {
                    PICK_FALLBACK_YIELD
                } else {
                    1
                }
            }
        };
        // `hits` não finito nunca parte o prop; 0/negativo parte ao primeiro.
        let hits = match parse_number(components.get("hits").map(String::as_str).unwrap_or("")) {
            Some(value) if !value.is_nan() && value != f64::INFINITY => {
                value.floor().max(1.0) as i64
            }
            _ => 3,
        };
        let range = match parse_number(components.get("range").map(String::as_str).unwrap_or("")) {
            Some(value) if value.is_finite() && value > 0.0 => value as f32,
            _ => RANGE_DEFAULT,
        };
        let cut_height = match parse_number(
            components
                .get("cut-height")
                .map(String::as_str)
                .unwrap_or(""),
        ) {
            Some(value) if value.is_finite() && value > 0.0 => (value as f32).clamp(0.05, 3.0),
            _ => 0.5,
        };
        let replace_scale = match parse_number(
            components
                .get("replace-scale")
                .map(String::as_str)
                .unwrap_or(""),
        ) {
            Some(value) if value.is_finite() && (0.05..=20.0).contains(&value) => value as f32,
            _ => 1.0,
        };
        let popup = components.get("popup-text").cloned().unwrap_or_default();
        let popup_color = parse_popup_color(
            components
                .get("popup-color")
                .map(String::as_str)
                .unwrap_or(""),
        );
        Some(Self {
            hits,
            remaining: hits,
            style,
            kind,
            amount,
            popup,
            popup_color,
            range,
            cut_height,
            replacement,
            replace_scale,
            name: name.to_string(),
            stable_key,
        })
    }

    /// O mesmo que [`Self::from_props`], a partir dos componentes tipados do
    /// `.bsn` (`bevy_ext_scene`): sem strings do DSL pelo meio. Paridade com
    /// `from_props` garantida pelo teste `specs_e_props_dao_o_mesmo`.
    pub fn from_specs(
        destructible: Option<&bevy_ext_scene::Destructible>,
        harvest: Option<&bevy_ext_scene::Harvestable>,
        name: &str,
        stable_key: Option<String>,
    ) -> Option<Self> {
        if destructible.is_none() && harvest.is_none() {
            return None;
        }
        let style =
            destructible.map_or(BreakStyle::Burst, |d| BreakStyle::from_raw(&d.break_style));
        let replacement = destructible
            .and_then(|d| d.replacement.clone())
            .unwrap_or_default();
        let is_berry = style == BreakStyle::Pick || replacement == "berry-bush-bare";
        let raw_kind = harvest.map_or("", |h| h.kind.as_str());
        let kind = if raw_kind.is_empty() && is_berry {
            PICK_FALLBACK_KIND.to_string()
        } else {
            raw_kind.to_string()
        };
        let amount = match harvest.map(|h| h.amount) {
            Some(amount) if amount >= 1 => i64::from(amount),
            _ if is_berry => PICK_FALLBACK_YIELD,
            _ => 1,
        };
        let hits = destructible.map_or(3, |d| i64::from(d.hits.max(1)));
        let cut_height = match destructible.and_then(|d| d.cut_height) {
            Some(value) if value.is_finite() && value > 0.0 => value.clamp(0.05, 3.0),
            _ => 0.5,
        };
        let replace_scale = match destructible.and_then(|d| d.replace_scale) {
            Some(value) if value.is_finite() && (0.05..=20.0).contains(&value) => value,
            _ => 1.0,
        };
        Some(Self {
            hits,
            remaining: hits,
            style,
            kind,
            amount,
            popup: destructible
                .map(|d| d.popup_text.clone())
                .unwrap_or_default(),
            popup_color: parse_popup_color(destructible.map_or("", |d| d.popup_color.as_str())),
            range: RANGE_DEFAULT,
            cut_height,
            replacement,
            replace_scale,
            name: name.to_string(),
            stable_key,
        })
    }

    /// Ferramenta que o prop pede (`chop`/`mine`/`pick`).
    pub fn tool(&self) -> &'static str {
        tool_of(self.style, &self.kind)
    }

    /// Verbo do prompt (`Cortar`/`Colher frutas`/`Minerar`).
    pub fn label(&self) -> &'static str {
        label_of(self.style, &self.kind)
    }

    /// Substituto permitido; a lista é fechada para um typo não spawnar nada.
    pub fn known_replacement(&self) -> Option<&str> {
        REPLACEMENTS
            .contains(&self.replacement.as_str())
            .then_some(self.replacement.as_str())
    }
}

/// Chave estável da estação tombada com o `bevy_flora`
/// (`flora-fell.<chave>.*` no save). Vive na árvore deitada (raiz) até ser
/// retalhada.
#[derive(Component, Debug, Clone)]
struct FloraFellKey(String);

/// Na **estação**: marca que a árvore tombada já é uma entidade RAIZ. O
/// `bevy_flora` corta em espaço local (`Transform`), por isso a árvore que
/// tomba tem de ser uma entidade RAIZ com a pose global — a estação é filha do
/// grupo da vegetação (offset de +38,73 m) e um `Transform` local punha o toco
/// debaixo do chão. A limpeza por troca de cena encontra a raiz por
/// `FellTree`/`Felled`; guardar a `Entity` aqui só duplicava essa posse.
#[derive(Component, Debug, Clone, Copy)]
struct FloraFellTree;

/// Foco da interação "Cortar em toros" numa árvore já deitada.
#[derive(Debug, Clone, Default, Resource)]
pub(crate) struct BuckFocus {
    pub(crate) entity: Option<Entity>,
    cooldown: f32,
}

/// Geração de cena em que nasceu uma árvore tombada / toco / toro do
/// `bevy_flora`.
///
/// Os três são entidades RAIZ (o descarregamento da cena só leva o que o
/// spawner marcou), por isso são limpos quando a geração muda. Comparar com o
/// `CurrentScene.id` não serve: a troca de cena corre no `Last` e a limpeza no
/// `Update` do frame seguinte — uma árvore **restaurada do save** na cena nova
/// (spawn do `SceneLoad`, dentro dessa troca) era varrida como se fosse da
/// velha. O `SceneEpoch` sobe ANTES do `SceneLoad`, por isso a árvore reposta
/// já leva a geração nova e sobrevive.
#[derive(Component, Debug, Clone, Copy)]
struct FellEpoch(u64);

/// A geração de cena corrente (`0` sem o recurso — apps mínimas/testes).
fn fell_epoch(epoch: Option<&SceneEpoch>) -> u64 {
    epoch.map_or(0, |epoch| epoch.0)
}

/// Pedido de escala uniforme para o substituto (`replace-scale`) à espera do
/// spawn do `<Prototype>`.
///
/// O `PendingPrototypeSpawn` da engine **não transporta escala** (nome,
/// posição, cota, callback Lua) e o `on_spawned` é uma `mlua::Function` que o
/// caminho Rust não tem. O `apply_script_spawns` materializa a raiz com a
/// translação pedida; este sistema, no `PostUpdate` depois dele e antes da
/// propagação, encontra a raiz nova pelo XZ e escreve a escala. A posição vem
/// do mesmo pedido, por isso o casamento é exato (≈1 mm).
#[derive(Debug, Clone, Copy)]
struct ReplacementScale {
    x: f32,
    z: f32,
    scale: f32,
    /// Idade do pedido (s): um protótipo que não existe não deixa o pedido
    /// pendente para sempre.
    age: f32,
}

/// Pedidos de escala à espera do spawn (ver [`ReplacementScale`]).
#[derive(Resource, Default)]
struct PendingReplacementScales(Vec<ReplacementScale>);

/// Teto de espera de um pedido de escala (s).
const REPLACEMENT_SCALE_TIMEOUT: f32 = 2.0;

/// Serviços de movimento do herói e de spawn de substitutos usados pelo golpe.
#[derive(bevy::ecs::system::SystemParam)]
struct HarvestServices<'w, 's> {
    logs: Query<'w, 's, &'static timber::GroundLog>,
    pools: Option<Res<'w, Assets<bevy_flora::TreePool>>>,
    mining: Res<'w, mining::Mining>,
    mouse: Option<Res<'w, ButtonInput<MouseButton>>>,

    spawns: Option<ResMut<'w, PendingScriptSpawns>>,
    scales: Option<ResMut<'w, PendingReplacementScales>>,
    motion: Option<Res<'w, crate::player::HeroMotionBus>>,
    collect_fx: Option<Res<'w, crate::collect_fx::CollectBurstQueue>>,
    hit_flashes: Option<Res<'w, crate::collect_fx::HitFlashQueue>>,
    shake: Option<ResMut<'w, bevy_ext_engine::camera_glue::CameraShake>>,
    children: Query<'w, 's, &'static Children>,
    crystals: Query<'w, 's, (), With<crate::rocks::CrystalGrowth>>,
}

/// Pedidos de tombo vindos do Lua (`vael.vael.fell_tree`): a função só
/// enfileira (o closure não tem o ECS); o sistema processa no `Update`.
#[derive(Resource, Clone, Default)]
struct PendingFloraFell(std::sync::Arc<std::sync::Mutex<Vec<u64>>>);

/// Instala `vael.vael.fell_tree(id) -> true` — o `harvest.lua` usa-a no ramo
/// `break-style: fall` no lugar de `vael.topple`. Sem isto o caminho Lua
/// (default) tombava a ESTAÇÃO com o `Falling` da engine e perdia o toco e o
/// tronco deitado do `bevy_flora`.
fn install_fell_api(
    mut commands: Commands,
    chips: Res<HitQueue>,
    mining: Res<mining::Mining>,
    buttons: Res<HarvestToolInput>,
) {
    let reserved = buttons.reserved.clone();
    let mining = (*mining).clone();
    let hits = (*chips).clone();
    let queue = PendingFloraFell::default();
    let shared = std::sync::Arc::clone(&queue.0);
    crate::script_host::register_installer(move |lua| -> mlua::Result<()> {
        let vael = vael_table(lua)?;
        mining.install(lua, &vael)?;
        vael_fn!(
            lua,
            vael,
            "harvest_tool_focus",
            [reserved = reserved.clone()],
            move |_, ()| { Ok(reserved.load(std::sync::atomic::Ordering::Relaxed) != 0) }
        );
        vael_fn!(
            lua,
            vael,
            "wood_focus",
            [reserved = reserved.clone()],
            move |_, ()| { Ok(reserved.load(std::sync::atomic::Ordering::Relaxed) == 1) }
        );
        for (name, stone) in [("wood_hit", false), ("rock_hit", true)] {
            let mining = mining.clone();
            let hits = hits.clone();
            vael.set(
                name,
                lua.create_function(
                    move |_,
                          (bits, x, y, z, nx, nz, height, final_hit): (
                        i64,
                        f32,
                        f32,
                        f32,
                        f32,
                        f32,
                        f32,
                        bool,
                    )| {
                        let Some(target) = Entity::try_from_bits(bits as u64) else {
                            return Ok(());
                        };
                        hits.push(ToolHit {
                            surface: if stone { mining.surface(target) } else { None },
                            stone,
                            target,
                            base: Vec3::new(x, y, z),
                            outward: Vec2::new(nx, nz),
                            height,
                            final_hit,
                        });
                        Ok(())
                    },
                )?,
            )?;
        }
        vael_fn!(
            lua,
            vael,
            "fell_tree",
            [shared = std::sync::Arc::clone(&shared)],
            move |_, bits: i64| {
                shared
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(bits as u64);
                Ok(true)
            }
        );
        Ok(())
    });
    commands.insert_resource(queue);
    info!("vael: `vael.vael.fell_tree` registado nos contextos BMS (tombo do bevy_flora no Lua)");
}

/// Processa os pedidos do Lua: árvore procedural → `FellTree` numa raiz com a
/// pose global; prop antigo → o `Falling` da engine (o que o `vael.topple`
/// fazia).
fn process_pending_fell(
    mut commands: Commands,
    pending: Option<Res<PendingFloraFell>>,
    epoch: Option<Res<SceneEpoch>>,
    mut save: Option<ResMut<VaelSave>>,
    props: Query<(
        &GlobalTransform,
        Option<&PooledTree>,
        Option<&PooledLook>,
        Option<&Harvestable>,
        Option<&Transform>,
    )>,
    players: Query<&GlobalTransform, With<Player>>,
) {
    let Some(pending) = pending else {
        return;
    };
    let epoch = fell_epoch(epoch.as_deref());
    let requests = std::mem::take(
        &mut *pending
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    );
    if requests.is_empty() {
        return;
    }
    let player = players
        .iter()
        .next()
        .map(GlobalTransform::translation)
        .unwrap_or(Vec3::ZERO);
    for bits in requests {
        let Some(entity) = Entity::try_from_bits(bits) else {
            continue;
        };
        let Ok((global, pooled, look, Some(harvestable), local)) = props.get(entity) else {
            continue;
        };
        let position = global.translation();
        let direction = (position - player).with_y(0.).normalize_or(Vec3::X);
        let cut_height = harvestable.cut_height;
        let key = harvestable.stable_key.clone();
        if let Some(key) = key.as_deref() {
            mark_harvested(save.as_deref_mut(), key);
        }
        if let Some(pooled) = pooled {
            let tree = spawn_felled_tree(
                &mut commands,
                pooled,
                look,
                global,
                direction,
                cut_height,
                false,
                epoch,
            );
            if let Some(key) = key.as_deref() {
                commands.entity(tree).insert(FloraFellKey(key.to_string()));
                store_flora_fell(save.as_deref_mut(), key, direction, cut_height, false);
            }
            regrowth::leave_root(&mut commands, entity, tree, key);
        } else {
            let initial = local.map(|local| local.rotation).unwrap_or_default();
            commands
                .entity(entity)
                .try_remove::<(
                    Harvestable,
                    ScriptInteraction,
                    vael_engine::luau::components::LuaScriptRef,
                )>()
                .insert(Falling {
                    axis: Vec3::new(direction.z, 0.0, -direction.x),
                    timer: 0.0,
                    initial,
                });
        }
    }
}

/// Foco atual da colheita (o prop mais próximo dentro do alcance).
#[derive(Debug, Clone, Default, Resource, Reflect)]
#[reflect(Resource)]
pub struct HarvestFocus {
    /// Entidade que recebeu o `ScriptInteraction` deste módulo.
    pub entity: Option<Entity>,
    /// Última ferramenta publicada em `harvest.tool` ("" = mãos livres).
    pub tool: String,
    /// Cooldown restante entre golpes (s).
    pub cooldown: f32,
    /// Menu/título/morte: o prompt e o golpe estão desligados.
    pub blocked: bool,
    pending_swing: Option<(Entity, f32)>,
    grip_remaining: f32,
    held_target: Option<Entity>,
    contact: Vec3,
}

#[derive(Default, Resource)]
struct HarvestToolInput {
    reserved: std::sync::Arc<std::sync::atomic::AtomicU8>,
    held: bool,
}

impl HarvestToolInput {
    fn reserve(&self, tool: &str) {
        self.reserved.store(
            match tool {
                "chop" => 1,
                "mine" => 2,
                _ => 0,
            },
            std::sync::atomic::Ordering::Relaxed,
        );
    }
}

fn read_harvest_buttons(
    mut input: ResMut<HarvestToolInput>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mouse: Option<Res<ButtonInput<MouseButton>>>,
    map: Option<Res<vael_engine::input_map::ActionMap>>,
    pads: Query<&Gamepad>,
) {
    let button = map
        .as_ref()
        .and_then(|map| map.bindings.get(&vael_engine::input_map::Action::Attack))
        .and_then(|b| b.pad.as_deref())
        .and_then(crate::input::pad_of);
    input.held = keys.is_some_and(|k| k.pressed(attack_key(map.as_deref())))
        || mouse.is_some_and(|m| m.pressed(MouseButton::Left))
        || button.is_some_and(|b| pads.iter().any(|pad| pad.pressed(b)));
}

/// Gate da colheita em Rust.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Resource)]
pub struct VaelHarvestConfig {
    pub rust: bool,
}

impl VaelHarvestConfig {
    /// Constrói o gate a partir do valor bruto (`None` = variável ausente).
    pub fn from_raw(raw: Option<&str>) -> Self {
        Self {
            rust: env_enabled(raw),
        }
    }

    /// Lê `VAEL_HARVEST_RUST` do ambiente.
    pub fn from_env() -> Self {
        Self::from_raw(std::env::var(RUST_ENV).ok().as_deref())
    }
}

impl Default for VaelHarvestConfig {
    /// Opt-in: só liga com `1`/`true`/`on`/`yes`.
    fn default() -> Self {
        Self::from_env()
    }
}

/// Patch do gate: `Unchanged`, escrever `true` ou apagar a chave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatePatch {
    Unchanged,
    On,
    Clear,
}

/// Decide o patch do gate a partir do valor atual do `vael.game()`.
pub fn gate_patch(current: Option<bool>, rust: bool) -> GatePatch {
    if rust {
        if current != Some(true) {
            GatePatch::On
        } else {
            GatePatch::Unchanged
        }
    } else if current == Some(true) {
        // Save escrito com o gate ligado não pode calar o Lua numa sessão
        // normal (o mesmo tratamento do `vael_rust_vitals`).
        GatePatch::Clear
    } else {
        GatePatch::Unchanged
    }
}

/// Plugin da colheita/interação.
pub struct VaelHarvestPlugin;

impl Plugin for VaelHarvestPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VaelHarvestConfig>()
            .init_resource::<HarvestFocus>()
            .init_resource::<HarvestToolInput>()
            .add_plugins(timber::TimberPlugin)
            .init_resource::<HitQueue>()
            .init_resource::<mining::Mining>()
            .init_resource::<mining::ArmOverride>()
            .add_systems(
                PostUpdate,
                mining::clear_override.before(bevy::app::AnimationSystems),
            )
            .add_systems(
                PostUpdate,
                mining::constrain
                    .after(bevy::app::AnimationSystems)
                    .after(vael_engine::ik::bone_offsets_system)
                    .before(bevy::transform::TransformSystems::Propagate),
            )
            .init_resource::<PendingReplacementScales>()
            .register_type::<Harvestable>()
            .register_type::<HarvestFocus>()
            .register_type::<BreakStyle>()
            // O gate vive fora do `run_if`: quando OFF também tem de apagar
            // um `true` que venha de um save antigo.
            // Depois do cofre: o `vael_table` é ler-ou-criar, e dois
            // instaladores em paralelo criavam cada um a sua `vael.vael` —
            // a do cofre perdia-se (o `vault_add` nil do `rules.lua:919`).
            .add_systems(
                Startup,
                (publish_harvest_gate, install_fell_api)
                    .chain()
                    .after(crate::vault::install_vault_api),
            )
            .add_systems(Update, sync_harvest_gate)
            // `Harvestable` e o reap correm SEMPRE: o caminho Lua (default)
            // também precisa deles para o restauro das árvores tombadas
            // (`flora-fell.*`). O gate só controla os recursos colhidos à mão.
            .add_systems(PostStartup, (attach_harvestables, reap_on_load).chain())
            .add_systems(
                vael_engine::scene_loader::SceneLoad,
                (attach_harvestables, reap_on_load)
                    .chain()
                    .after(vael_engine::recipes::spawn::startup),
            )
            // Props que nascem DEPOIS do load (spawn de script, `live_edit` do
            // Jackdaw): o `PostStartup`/`SceneLoad` não os vê e o caminho Lua
            // (`vael.player_nearby`) vê-os — a colheita Rust ficava cega.
            .add_systems(Update, attach_late_harvestables)
            .add_systems(
                PreUpdate,
                (
                    sync_size_scaled_hits,
                    read_harvest_buttons,
                    update_harvest_focus,
                )
                    .chain()
                    .after(crate::input::VaelInputSystems)
                    .after(bevy::input::InputSystems),
            )
            .add_systems(Update, harvest_input)
            // O processamento do tombo do Lua, o reap periódico, o embate e
            // os toros valem para os DOIS caminhos de colheita.
            .add_systems(
                Update,
                (
                    process_pending_fell,
                    reap_periodic.after(process_pending_fell),
                    regrowth::fit_stumps.before(regrowth::grow),
                    regrowth::grow.after(reap_periodic).after(harvest_input),
                    chips::emit.after(harvest_input),
                    chips::fly,
                    tree_landed_feedback,
                    buck_felled_trees,
                ),
            )
            // As raízes do `bevy_flora` (árvores deitadas, tocos e toros) não
            // são da cena: ficam com a geração em que nasceram e morrem quando
            // ela muda.
            .add_systems(Update, (tag_fell_entities, despawn_fell_trees).chain())
            // A escala do substituto: o spawn do protótipo é exclusivo e corre
            // no `PostUpdate`; escrever depois dele e antes da propagação põe
            // a escala no `GlobalTransform` do MESMO frame.
            .add_systems(
                PostUpdate,
                apply_replacement_scales
                    .after(vael_engine::recipes::spawn::apply_script_spawns)
                    .before(bevy::transform::TransformSystems::Propagate),
            )
            .init_resource::<BuckFocus>();
    }
}

/// `true` quando `VAEL_HARVEST_RUST` liga a implementação Rust.
pub fn harvest_rust_enabled(config: Option<Res<VaelHarvestConfig>>) -> bool {
    config.is_some_and(|config| config.rust)
}

// ── parser de props ─────────────────────────────────────────────────────

/// `prop-<nome>` do `ScriptProps` (texto cru; "" quando ausente).
fn prop<'a>(props: &'a [(String, String)], key: &str) -> &'a str {
    props
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
        .unwrap_or("")
}

/// `destructible="hits: 3; break-style: fall; …"` → pares.
///
/// Reproduz o `gmatch("([%w%-_]+)%s*:%s*([^;]+)")` do Lua: a chave é ASCII
/// alfanumérica/`-`/`_`, o valor é tudo até ao `;` seguinte e é aparado. Uma
/// chave inválida no meio (`"foo bar: 1"`) ainda casa a partir de `bar`, como
/// no Lua.
pub fn parse_components(value: &str) -> BTreeMap<String, String> {
    let bytes = value.as_bytes();
    let mut out = BTreeMap::new();
    let mut i = 0;
    while i < bytes.len() {
        if !is_key_byte(bytes[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && is_key_byte(bytes[i]) {
            i += 1;
        }
        let key = &value[start..i];
        let mut cursor = i;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= bytes.len() || bytes[cursor] != b':' {
            i = start + 1;
            continue;
        }
        cursor += 1;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let value_start = cursor;
        while cursor < bytes.len() && bytes[cursor] != b';' {
            cursor += 1;
        }
        if cursor > value_start {
            out.insert(
                key.to_ascii_lowercase(),
                value[value_start..cursor].trim().to_string(),
            );
            i = cursor;
        } else {
            // `[^;]+` não casa vazio: o gmatch avança um carácter.
            i = start + 1;
        }
    }
    out
}

fn is_key_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'
}

/// `tonumber` tolerante (decimal ou hex inteiro não negativo).
fn parse_number(raw: &str) -> Option<f64> {
    let text = raw.trim();
    if text.is_empty() {
        return None;
    }
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X"))
        && let Ok(value) = i64::from_str_radix(hex, 16)
    {
        return Some(value as f64);
    }
    text.parse::<f64>().ok()
}

/// Cor `#rgb`/`#rrggbb`; inválida cai no amarelo do Lua.
///
/// O parse é o do formato de cena ([`parse_color`] do `bevy_ext_world`, a
/// mesma matemática `/255`), mas o contrato do prop é mais estreito: só
/// `#hex` conta — nomes de cor e `0x` do formato de cena continuam a cair
/// no amarelo, como no Lua.
pub fn parse_popup_color(raw: &str) -> [f32; 3] {
    if raw.starts_with('#')
        && let Ok(color) = parse_color(raw, "popup-color")
    {
        return color;
    }
    DEFAULT_POPUP_COLOR
}

/// Ferramenta do prop (o `tool_of` do Lua).
pub fn tool_of(style: BreakStyle, kind: &str) -> &'static str {
    if style == BreakStyle::Fall {
        return "chop";
    }
    if style == BreakStyle::Pick {
        return "pick";
    }
    if kind == "wood" || kind == "dark-wood" {
        return "chop";
    }
    "mine"
}

/// Verbo do prompt (o `label_of` do Lua).
pub fn label_of(style: BreakStyle, kind: &str) -> &'static str {
    if style == BreakStyle::Fall || kind == "wood" || kind == "dark-wood" {
        return "Cortar";
    }
    match style {
        BreakStyle::Pick => "Colher frutas",
        _ => "Minerar",
    }
}

// ── chave estável / persistência ───────────────────────────────────────

/// Chave persistente de um prop: `Nome@proveniência`.
///
/// A proveniência é o ficheiro XML (relativo à raiz do projeto quando
/// possível) e o caminho de índices do nó (`#3/4`), não a pose arredondada:
/// a mesma árvore continua a ser a mesma depois de mover o mundo. Sem nome,
/// a proveniência sozinha identifica a instância; sem nome nem proveniência
/// não há chave (o Lua também não persiste props anónimos).
pub fn stable_key(
    name: Option<&str>,
    source: Option<&SceneSource>,
    root: Option<&Path>,
) -> Option<String> {
    let name = name.unwrap_or("");
    let provenance = source.map(|source| provenance_label(source, root));
    match (name.is_empty(), provenance) {
        (false, Some(provenance)) => Some(format!("{name}@{provenance}")),
        (false, None) => Some(name.to_string()),
        (true, Some(provenance)) => Some(format!("@{provenance}")),
        (true, None) => None,
    }
}

/// Ficheiro + caminho do nó que declarou a entidade.
fn provenance_label(source: &SceneSource, root: Option<&Path>) -> String {
    let file = source.0.file.as_ref();
    let relative = root
        .and_then(|root| file.strip_prefix(root).ok())
        .unwrap_or(file);
    let parts: Vec<String> = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    // Últimos dois componentes (`vegetation/forest.xml`) chegam para
    // distinguir os ficheiros do projeto sem gravar caminhos absolutos.
    let mut label = if parts.len() > 2 {
        parts[parts.len() - 2..].join("/")
    } else {
        parts.join("/")
    };
    if label.is_empty() {
        label.push_str("mundo");
    }
    if !source.0.path.is_empty() {
        label.push('#');
        label.push_str(
            &source
                .0
                .path
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join("/"),
        );
    }
    label
}

/// Marca a chave em `VaelSave::harvested` e no espelho do jogo
/// (`harvested.<chave>` no store do processo — o `vael.game()` dos
/// contextos BMS lê daqui).
fn mark_harvested(save: Option<&mut VaelSave>, key: &str) {
    if key.is_empty() {
        return;
    }
    if let Some(save) = save {
        save.harvested.insert(key.to_string());
    }
    let mut patch = BTreeMap::new();
    patch.insert(format!("harvested.{key}"), Json::Bool(true));
    vael_engine::luau::game::apply_patch_json(&patch);
}

/// Guarda a pose/escala do substituto (`stump.<chave>.*`), como o Lua.
fn store_stump(save: Option<&mut VaelSave>, key: &str, proto: &str, position: Vec3, scale: f32) {
    if key.is_empty() {
        return;
    }
    let fields = [
        (
            format!("stump.{key}.proto"),
            Json::String(proto.to_string()),
        ),
        (format!("stump.{key}.x"), json!(position.x)),
        (format!("stump.{key}.y"), json!(position.y)),
        (format!("stump.{key}.z"), json!(position.z)),
        (format!("stump.{key}.scale"), json!(scale)),
    ];
    if let Some(save) = save {
        for (name, value) in &fields {
            save.legacy.insert(name.clone(), value.clone());
        }
    }
    vael_engine::luau::game::apply_patch_json(&fields.into_iter().collect::<BTreeMap<_, _>>());
}

/// Guarda o estado do tombo de uma árvore procedural (`flora-fell.<chave>.*`):
/// direcção da queda, altura do corte e se já foi retalhada em toros. É o que
/// o load usa para a deitar no sítio certo, sem a tombar outra vez.
fn store_flora_fell(
    save: Option<&mut VaelSave>,
    key: &str,
    direction: Vec3,
    cut_height: f32,
    bucked: bool,
) {
    if key.is_empty() {
        return;
    }
    let fields = [
        (format!("flora-fell.{key}.dx"), json!(direction.x)),
        (format!("flora-fell.{key}.dz"), json!(direction.z)),
        (format!("flora-fell.{key}.cut"), json!(cut_height)),
        (format!("flora-fell.{key}.bucked"), Json::Bool(bucked)),
    ];
    if let Some(save) = save {
        for (name, value) in &fields {
            save.legacy.insert(name.clone(), value.clone());
        }
    }
    vael_engine::luau::game::apply_patch_json(&fields.into_iter().collect::<BTreeMap<_, _>>());
}

/// Marca a árvore deitada como retalhada em toros (`flora-fell.<chave>.bucked`).
fn store_flora_bucked(save: Option<&mut VaelSave>, key: &str) {
    if key.is_empty() {
        return;
    }
    let name = format!("flora-fell.{key}.bucked");
    if let Some(save) = save {
        save.legacy.insert(name.clone(), Json::Bool(true));
    }
    let mut patch = BTreeMap::new();
    patch.insert(name, Json::Bool(true));
    vael_engine::luau::game::apply_patch_json(&patch);
}

/// Estado de uma árvore tombada com o `bevy_flora`, lido do save.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FloraFell {
    pub dx: f32,
    pub dz: f32,
    pub cut_height: f32,
    pub bucked: bool,
}

/// `flora-fell.<chave>.*` do snapshot → [`FloraFell`]; `None` se não houver
/// direcção guardada (save antigo, entidade sem chave).
pub fn flora_fell_from_kv(kv: &BTreeMap<String, Json>, key: &str) -> Option<FloraFell> {
    let get = |suffix: &str| kv.get(&format!("flora-fell.{key}.{suffix}"));
    let dx = get("dx")?.as_f64()? as f32;
    let dz = get("dz")?.as_f64()? as f32;
    if !dx.is_finite() || !dz.is_finite() {
        return None;
    }
    let cut_height = get("cut")
        .and_then(Json::as_f64)
        .filter(|value| value.is_finite() && *value > 0.0)
        .map(|value| (value as f32).clamp(0.05, 3.0))
        .unwrap_or(0.5);
    let bucked = get("bucked").and_then(Json::as_bool).unwrap_or(false);
    Some(FloraFell {
        dx,
        dz,
        cut_height,
        bucked,
    })
}

/// Dados do substituto guardados no save.
#[derive(Debug, Clone, PartialEq)]
pub struct Stump {
    pub proto: String,
    pub x: f32,
    /// `None` = sem Y autorado (o spawn assenta no terreno).
    pub y: Option<f32>,
    pub z: f32,
    /// Escala uniforme autorada (`replace-scale`); saves antigos não a têm.
    pub scale: f32,
}

/// Lê `stump.<chave>.*` de um snapshot do `vael.game()`/`legacy`.
pub fn stump_for(kv: &BTreeMap<String, Json>, key: &str) -> Option<Stump> {
    let field = |name: &str| format!("stump.{key}.{name}");
    let proto = kv.get(&field("proto"))?.as_str()?;
    if !REPLACEMENTS.contains(&proto) {
        return None;
    }
    let scale = json_f32(kv.get(&field("scale")))
        .filter(|scale| (0.05..=20.0).contains(scale))
        .unwrap_or(1.0);
    Some(Stump {
        proto: proto.to_string(),
        x: json_f32(kv.get(&field("x")))?,
        y: json_f32(kv.get(&field("y"))),
        z: json_f32(kv.get(&field("z")))?,
        scale,
    })
}

fn json_f32(value: Option<&Json>) -> Option<f32> {
    let Json::Number(number) = value? else {
        return None;
    };
    number
        .as_f64()
        .filter(|value| value.is_finite())
        .map(|value| value as f32)
}

/// `true` quando a entidade já foi colhida; devolve a chave que casou.
///
/// Chaves com `@` são as estáveis (só casam com o `stable_key` da entidade);
/// chaves sem `@` são nomes do save antigo do Lua e casam pelo `Name`.
pub fn reaped_key(harvestable: &Harvestable, harvested: &BTreeSet<String>) -> Option<String> {
    if let Some(key) = &harvestable.stable_key
        && harvested.contains(key)
    {
        return Some(key.clone());
    }
    if !harvestable.name.is_empty()
        && !harvestable.name.contains('@')
        && harvested.contains(&harvestable.name)
    {
        return Some(harvestable.name.clone());
    }
    None
}

// ── regras puras (testáveis sem a VM) ───────────────────────────────────

/// Aplica um golpe; `true` quando a durabilidade chega a zero.
pub fn apply_hit(harvestable: &mut Harvestable) -> bool {
    harvestable.remaining = (harvestable.remaining - 1).max(0);
    harvestable.remaining == 0
}

/// O que um golpe final rendeu.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BreakRewards {
    /// Unidades que entraram no cofre.
    pub added: i64,
    /// A bolsa estava cheia: nada entrou e o prop NÃO parte.
    pub full: bool,
    pub xp: XpGain,
}

/// Credita loot + XP pelas funções puras do cofre e do XP.
///
/// O crédito parcial é devolvido quando a bolsa não leva tudo: o prop fica no
/// mundo para depois de fazer espaço (o `break_prop` do Lua).
pub fn grant_break_rewards(
    inventory: &mut Inventory,
    progression: &mut XpState,
    vault: &VaultRules,
    xp_rules: &XpRules,
    kind: &str,
    amount: i64,
) -> BreakRewards {
    let mut added = 0;
    if !kind.is_empty() && amount > 0 {
        let key = item_key(kind);
        let mut kv = BTreeMap::new();
        kv.insert(key.clone(), inventory.get(kind).max(0));
        added = vault_add(vault, &mut kv, kind, amount as f64);
        if added < amount {
            if added > 0 {
                let _ = vault_take(vault, &mut kv, kind, added as f64);
            }
            return BreakRewards {
                added: 0,
                full: true,
                xp: XpGain::default(),
            };
        }
        if added > 0 {
            inventory.set(kind, kv.get(&key).copied().unwrap_or(0));
        }
    }
    let mut xp_kv = xp_map_from_state(progression);
    let xp = add_xp(&mut xp_kv, xp_rules, HARVEST_XP);
    apply_xp_map(&xp_kv, progression);
    BreakRewards {
        added,
        full: false,
        xp,
    }
}

fn xp_map_from_state(state: &XpState) -> XpMap {
    let mut kv = XpMap::new();
    kv.insert(KEY_XP.into(), state.xp as f64);
    kv.insert(KEY_XP_NEXT.into(), state.xp_next as f64);
    kv.insert(KEY_LEVEL.into(), state.level as f64);
    kv.insert(KEY_SKILL_POINTS.into(), state.skill_points as f64);
    kv
}

fn apply_xp_map(kv: &XpMap, state: &mut XpState) {
    state.xp = kv.get(KEY_XP).copied().unwrap_or(0.0) as i64;
    state.xp_next = kv.get(KEY_XP_NEXT).copied().unwrap_or(0.0) as i64;
    state.level = kv.get(KEY_LEVEL).copied().unwrap_or(0.0) as i64;
    state.skill_points = kv.get(KEY_SKILL_POINTS).copied().unwrap_or(0.0) as i64;
}

/// Prop mais próximo DENTRO do alcance efetivo; empates ficam no primeiro.
pub fn nearest_within<I>(origin: Vec3, candidates: I) -> Option<(Entity, f32)>
where
    I: IntoIterator<Item = (Entity, Vec3, f32)>,
{
    let mut best: Option<(Entity, f32)> = None;
    for (entity, position, range) in candidates {
        // Comparação em distância ao quadrado: o `sqrt` só no vencedor.
        let distance_sq = position.distance_squared(origin);
        let range = range.max(0.0);
        if distance_sq <= range * range
            && best.is_none_or(|(_, best_distance_sq)| distance_sq < best_distance_sq)
        {
            best = Some((entity, distance_sq));
        }
    }
    best.map(|(entity, distance_sq)| (entity, distance_sq.sqrt()))
}

// ── sistemas ────────────────────────────────────────────────────────────

/// Arranque: fixa o gate no `vael.game()`.
fn publish_harvest_gate(config: Res<VaelHarvestConfig>) {
    apply_gate(config.rust);
}

/// Corrige o gate enquanto o jogo corre (o load do menu pode repor o save).
fn sync_harvest_gate(
    time: Res<Time>,
    config: Res<VaelHarvestConfig>,
    mut timer: Local<Option<Timer>>,
) {
    let timer =
        timer.get_or_insert_with(|| Timer::from_seconds(GATE_PERIOD_SECS, TimerMode::Repeating));
    if !timer.tick(time.delta()).just_finished() {
        return;
    }
    apply_gate(config.rust);
}

fn apply_gate(rust: bool) {
    let current = read_gate();
    let mut patch = BTreeMap::new();
    match gate_patch(current, rust) {
        GatePatch::On => {
            patch.insert(RUST_GATE_KEY.to_string(), Json::Bool(true));
        }
        GatePatch::Clear => {
            patch.insert(RUST_GATE_KEY.to_string(), Json::Null);
        }
        GatePatch::Unchanged => {}
    }
    if !patch.is_empty() {
        vael_engine::luau::game::apply_patch_json(&patch);
    }
}

fn read_gate() -> Option<bool> {
    vael_engine::luau::game::game_get_json(RUST_GATE_KEY).map(|v| v.as_bool().unwrap_or(false))
}

/// Marca pedras e árvores cujo número de golpes escala proporcionalmente ao
/// tamanho físico da instância (escala do `Transform`, segmentos/dimensões de
/// `QuarryStone`, tronco do `TreePool` ou AABB do `Collider`).
#[derive(Component, Debug, Clone, Copy)]
pub struct SizeScaledHits {
    pub base_hits: i64,
}

fn effective_scale(transform: Option<&Transform>, global: Option<&GlobalTransform>) -> Vec3 {
    let local = transform.map_or(Vec3::ONE, |t| t.scale.abs());
    let world = global.map_or(Vec3::ONE, |g| g.scale().abs());
    if (world - Vec3::ONE).length_squared() > 1e-6 {
        world
    } else {
        local
    }
}

/// Golpes necessários para minerar uma pedra em função do tamanho (escala,
/// número de blocos da formação procedural e volume do collider).
pub fn rock_hits_for_size(
    base_hits: i64,
    scale: Vec3,
    segments: Option<u8>,
    collider: Option<&avian3d::prelude::Collider>,
) -> i64 {
    let base = base_hits.max(1) as f32;
    let scale = scale.abs();
    let mean_scale = (scale.x * scale.y * scale.z).cbrt();
    if !mean_scale.is_finite() || mean_scale <= 0.0 {
        return base_hits.max(1);
    }
    let extent_ratio = if let Some(collider) = collider {
        let aabb = collider
            .shape()
            .compute_aabb(&avian3d::parry::math::Pose::identity());
        let local_ext = Vec3::new(
            aabb.maxs.x - aabb.mins.x,
            aabb.maxs.y - aabb.mins.y,
            aabb.maxs.z - aabb.mins.z,
        )
        .abs();
        let world_ext = local_ext * scale;
        let geom_extent = (world_ext.x * world_ext.y * world_ext.z).cbrt();
        if geom_extent.is_finite() && geom_extent > 1e-3 {
            // Referência: pedra média de 1 segmento à escala 1 (~1.06 m de média geométrica).
            (geom_extent / 1.06).max(0.25)
        } else {
            mean_scale
        }
    } else {
        mean_scale
    };
    let segment_bonus = 1.0 + 0.12 * f32::from(segments.unwrap_or(1).saturating_sub(1));
    ((base * extent_ratio * segment_bonus).round() as i64).clamp(1, 10)
}

/// Golpes necessários para derrubar uma árvore em função do raio do tronco,
/// altura e escala da instância.
pub fn tree_hits_for_size(
    base_hits: i64,
    scale: Vec3,
    trunk_radius: Option<f32>,
    trunk_height: Option<f32>,
    collider: Option<&avian3d::prelude::Collider>,
) -> i64 {
    let base = base_hits.max(1) as f32;
    let scale = scale.abs();
    let mean_scale = (scale.x * scale.y * scale.z).cbrt();
    if !mean_scale.is_finite() || mean_scale <= 0.0 {
        return base_hits.max(1);
    }
    let scale_xz = (scale.x * scale.z).sqrt();
    let size_ratio = if let (Some(r), Some(h)) = (trunk_radius, trunk_height) {
        let world_r = (r * scale_xz).max(0.05);
        let world_h = (h * scale.y).max(0.5);
        // Referência (3 golpes): raio de tronco ~0.40 m e altura ~7.5 m.
        (world_r / 0.40) * 0.65 + (world_h / 7.5) * 0.35
    } else if let Some(collider) = collider {
        let aabb = collider
            .shape()
            .compute_aabb(&avian3d::parry::math::Pose::identity());
        let local_ext = Vec3::new(
            aabb.maxs.x - aabb.mins.x,
            aabb.maxs.y - aabb.mins.y,
            aabb.maxs.z - aabb.mins.z,
        )
        .abs();
        let world_r = ((local_ext.x + local_ext.z) * 0.25 * scale_xz).max(0.05);
        let world_h = (local_ext.y * scale.y).max(0.5);
        // Cilindro de estação padrão: raio ~0.45 m, altura 4.4 m.
        (world_r / 0.45) * 0.65 + (world_h / 4.4) * 0.35
    } else {
        mean_scale
    };
    ((base * size_ratio).round() as i64).clamp(1, 10)
}

/// Golpes necessários para processar um tronco caído (`GroundLog`) em função
/// do comprimento e do raio da base.
pub fn log_hits_for_size(length: f32, radius: f32) -> i64 {
    let length = length.max(0.25);
    let radius = radius.max(0.05);
    let ratio = (radius / 0.38) * 0.65 + (length / 5.5) * 0.35;
    ((3.0 * ratio).round() as i64).clamp(1, 8)
}

/// Copia os props autorais para [`Harvestable`] (PostStartup + SceneLoad).
fn attach_harvestables(
    mut commands: Commands,
    root: Option<Res<VaelProjectRoot>>,
    props: Query<
        (
            Entity,
            &ScriptProps,
            Option<&Name>,
            Option<&SceneSource>,
            Option<&Transform>,
            Option<&GlobalTransform>,
        ),
        Without<Harvestable>,
    >,
) {
    let root = root.as_deref().map(|root| root.0.as_path());
    attach_props(&mut commands, root, &props);
}

/// O mesmo, para os props que nascem **depois** do load (`spawn_prototype`
/// de um script, `live_edit` do Jackdaw a re-spawnar o nó). O caminho Lua lê
/// `vael.player_nearby` (vivo) e via-os; o Rust ficava cego até ao reload.
fn attach_late_harvestables(
    mut commands: Commands,
    root: Option<Res<VaelProjectRoot>>,
    props: Query<
        (
            Entity,
            &ScriptProps,
            Option<&Name>,
            Option<&SceneSource>,
            Option<&Transform>,
            Option<&GlobalTransform>,
        ),
        (Added<ScriptProps>, Without<Harvestable>),
    >,
) {
    let root = root.as_deref().map(|root| root.0.as_path());
    attach_props(&mut commands, root, &props);
}

fn attach_props<F: bevy::ecs::query::QueryFilter>(
    commands: &mut Commands,
    root: Option<&Path>,
    props: &Query<
        (
            Entity,
            &ScriptProps,
            Option<&Name>,
            Option<&SceneSource>,
            Option<&Transform>,
            Option<&GlobalTransform>,
        ),
        F,
    >,
) {
    for (entity, props, name, source, transform, global) in props.iter() {
        let name = name.map(Name::as_str).unwrap_or("");
        let key = stable_key(Some(name), source, root);
        // O `.bsn` tipado é a fonte; as props (strings do DSL) só para mundos
        // antigos/QA que ainda não foram migrados.
        let specs = source.and_then(|source| bevy_ext_world::bsn::specs::specs_for(&source.0));
        let typed = specs.as_deref().and_then(|specs| {
            Harvestable::from_specs(
                specs.destructible.as_ref(),
                specs.harvest.as_ref(),
                name,
                key.clone(),
            )
        });
        if let Some(mut harvestable) =
            typed.or_else(|| Harvestable::from_props(&props.0, name, key))
        {
            let tool = harvestable.tool();
            if matches!(tool, "chop" | "mine") {
                let base_hits = harvestable.hits;
                let scale = effective_scale(transform, global);
                let scaled = if tool == "mine" {
                    rock_hits_for_size(base_hits, scale, None, None)
                } else {
                    tree_hits_for_size(base_hits, scale, None, None, None)
                };
                harvestable.hits = scaled;
                harvestable.remaining = scaled;
                commands
                    .entity(entity)
                    .insert((harvestable, SizeScaledHits { base_hits }));
            } else {
                commands.entity(entity).insert(harvestable);
            }
        }
    }
}

/// Atualiza `hits`/`remaining` de pedras e árvores intactas assim que a sua
/// geometria final (`QuarryStone`, `PooledTree` ou `Collider`) fica disponível.
#[allow(clippy::type_complexity)]
fn sync_size_scaled_hits(
    pools: Option<Res<Assets<bevy_flora::TreePool>>>,
    mut targets: Query<
        (
            &mut Harvestable,
            &SizeScaledHits,
            Option<&Transform>,
            Option<&GlobalTransform>,
            Option<&crate::rocks::QuarryStone>,
            Option<&PooledTree>,
            Option<&avian3d::prelude::Collider>,
        ),
        Or<(
            Added<SizeScaledHits>,
            Added<crate::rocks::QuarryStone>,
            Added<PooledTree>,
            Added<avian3d::prelude::Collider>,
            Changed<Transform>,
        )>,
    >,
) {
    for (mut harvestable, size_scaled, transform, global, quarry, pooled, collider) in &mut targets
    {
        // Não altera a contagem de uma pedra ou árvore que já levou golpes.
        if harvestable.remaining != harvestable.hits {
            continue;
        }
        let scale = effective_scale(transform, global);
        let scaled = match harvestable.tool() {
            "mine" => rock_hits_for_size(
                size_scaled.base_hits,
                scale,
                quarry.map(|q| q.segments),
                collider,
            ),
            "chop" => {
                let (trunk_r, trunk_h) = pooled
                    .and_then(|p| pools.as_ref()?.get(&p.pool))
                    .map(|pool| {
                        (
                            Some(pool.params.trunk.radius),
                            Some(pool.params.trunk.height),
                        )
                    })
                    .unwrap_or((None, None));
                tree_hits_for_size(size_scaled.base_hits, scale, trunk_r, trunk_h, collider)
            }
            _ => continue,
        };
        if harvestable.hits != scaled {
            harvestable.hits = scaled;
            harvestable.remaining = scaled;
        }
    }
}

/// Atualiza o alvo/prompt da colheita a cada frame.
#[allow(clippy::too_many_arguments)]
fn update_harvest_focus(
    mut commands: Commands,
    time: Res<Time>,
    mut focus: ResMut<HarvestFocus>,
    players: Query<&GlobalTransform, (With<Player>, Without<crate::boat::RidingBoat>)>,
    harvestables: Query<
        (
            Entity,
            &GlobalTransform,
            &Harvestable,
            Option<&Health>,
            Option<&avian3d::prelude::Collider>,
            Has<timber::GroundLog>,
        ),
        (Without<Player>, Without<timber::Crown>),
    >,
    config: Res<VaelHarvestConfig>,
    buttons: Res<HarvestToolInput>,
    menus: Option<Res<MenusOpen>>,
    modals: Option<Res<vael_engine::ui::modal::UiModalsOpen>>,
    scene: Option<Res<CurrentScene>>,
    vitals: Option<Res<PlayerVitals>>,
    mut ui: Option<ResMut<UiScriptBinds>>,
    map: Option<Res<vael_engine::input_map::ActionMap>>,
) {
    focus.cooldown = (focus.cooldown - time.delta_secs()).max(0.0);
    focus.grip_remaining = (focus.grip_remaining - time.delta_secs()).max(0.0);
    let blocked = scene.as_deref().and_then(|scene| scene.id.as_deref()) == Some("menu")
        || modals
            .as_deref()
            .is_some_and(|modals| modals.is_open("title"))
        || menus.as_deref().is_some_and(MenusOpen::any)
        || vitals.as_deref().is_some_and(|vitals| vitals.hp <= 0.0);
    focus.blocked = blocked;
    if !buttons.held || blocked {
        focus.held_target = None;
    }
    buttons.reserve("");
    if blocked || players.is_empty() {
        focus.pending_swing = None;
        focus.grip_remaining = 0.;
        clear_focus(&mut commands, &mut focus, ui.as_deref_mut());
        return;
    }
    let Some(player) = players.iter().next() else {
        clear_focus(&mut commands, &mut focus, ui.as_deref_mut());
        return;
    };
    let origin = player.translation();
    // Uma leitura do env por frame, não uma por harvestable.
    let scale = range_scale();
    let hand = origin + Vec3::Y * 0.8;
    let nearest = harvestables
        .iter()
        .filter_map(|(e, tf, h, hp, collider, log)| {
            if h.remaining <= 0 || hp.is_some_and(|hp| hp.current <= 0.) {
                return None;
            }
            let pos = tf.translation();
            let reach = h.range * scale;
            let uses_tool = matches!(h.tool(), "chop" | "mine");
            // A rolling trunk can be reached anywhere along its length. Project at hand
            // height, so the hero need not stand next to the origin buried under the tree.
            let (contact, distance) = if uses_tool {
                if !log && pos.xz().distance_squared(origin.xz()) > (reach + 4.).powi(2) {
                    return None;
                }
                let contact =
                    collider.map_or(pos, |c| c.project_point(pos, tf.rotation(), hand, true).0);
                (contact, hand.distance(contact))
            } else {
                (pos, origin.distance(pos))
            };
            (distance <= reach).then_some((e, contact, distance, h, log, origin.distance(pos)))
        })
        .min_by(|a, b| {
            let locked = |e| {
                focus.held_target == Some(e)
                    || focus.pending_swing.is_some_and(|(target, _)| target == e)
            };
            locked(b.0)
                .cmp(&locked(a.0))
                .then_with(|| a.2.total_cmp(&b.2))
                .then_with(|| a.0.cmp(&b.0))
        });
    let Some((entity, contact, _, harvestable, log, center_distance)) =
        nearest.filter(|(_, _, _, h, _, _)| config.rust || matches!(h.tool(), "chop" | "mine"))
    else {
        clear_focus(&mut commands, &mut focus, ui.as_deref_mut());
        buttons.reserve(&focus.tool);
        return;
    };
    focus.contact = contact;
    let uses_tool = matches!(harvestable.tool(), "chop" | "mine");

    if focus.entity != Some(entity) {
        if let Some(previous) = focus.entity.replace(entity) {
            commands.entity(previous).try_remove::<ScriptInteraction>();
        }
        commands.entity(entity).try_insert(ScriptInteraction {
            label: if log {
                "Extrair madeira"
            } else {
                harvestable.label()
            }
            .to_string(),
            key: attack_key(map.as_deref()),
            range: if uses_tool {
                (harvestable.range * scale).max(center_distance + 1.)
            } else {
                harvestable.range * scale
            },
        });
    }
    set_tool(&mut focus, harvestable.tool(), ui.as_deref_mut());
    buttons.reserve(&focus.tool);
}

/// A tecla de "atacar" do `ActionMap` (J por defeito): a do prompt `[J]` e
/// a que o `InteractionFocus` arbitra entre alvos.
fn attack_key(map: Option<&vael_engine::input_map::ActionMap>) -> KeyCode {
    map.and_then(|map| crate::input::action_key(map, vael_engine::input_map::Action::Attack))
        .unwrap_or(KeyCode::KeyJ)
}

/// Remove o prompt do alvo anterior e devolve a ferramenta às mãos.
fn clear_focus(commands: &mut Commands, focus: &mut HarvestFocus, ui: Option<&mut UiScriptBinds>) {
    if let Some(entity) = focus.entity.take() {
        commands.entity(entity).try_remove::<ScriptInteraction>();
    }
    set_tool(focus, "", ui);
}

/// `harvest.tool` do `equipment.lua` ("" quando é `pick` ou não há alvo).
fn set_tool(focus: &mut HarvestFocus, tool: &str, ui: Option<&mut UiScriptBinds>) {
    if focus.grip_remaining > 0.0 {
        return;
    }
    let tool = if tool == "pick" { "" } else { tool };
    if focus.tool == tool {
        return;
    }
    focus.tool = tool.to_string();
    if let Some(ui) = ui {
        ui.0.insert(
            "harvest.tool".to_string(),
            ScriptBindValue::Text(tool.to_string()),
        );
    }
}

/// Spawn da árvore que tomba como entidade RAIZ, com a pose GLOBAL da
/// estação (o `bevy_flora` corta em espaço local e a estação é filha do grupo
/// da vegetação — um `Transform` local punha o toco 38 m abaixo). Herda o
/// pool, a variante e o look da estação.
fn spawn_felled_tree(
    commands: &mut Commands,
    pooled: &PooledTree,
    look: Option<&PooledLook>,
    global: &GlobalTransform,
    direction: Vec3,
    cut_height: f32,
    lying: bool,
    epoch: u64,
) -> Entity {
    let tree = commands
        .spawn((
            PooledTree {
                pool: pooled.pool.clone(),
                variant: pooled.variant,
            },
            Transform::from_translation(global.translation())
                .with_rotation(global.rotation())
                .with_scale(global.scale()),
            Visibility::Inherited,
            FellTree {
                direction: Vec2::new(direction.x, direction.z),
                cut_height,
                speed: 1.16,
                lying,
            },
            timber::FallingTrunk { restored: lying },
            FellEpoch(epoch),
        ))
        .id();
    if let Some(look) = look {
        commands.entity(tree).insert(*look);
    }
    tree
}

/// Rotação horária dinâmica (graus, vista de cima) aplicada ao herói ao derrubar uma árvore.
///
/// Troncos finos (~0,30 m) giram pouco (~4°), troncos médios (~0,41 m) ficam por volta de 5° e
/// troncos grossos (≥0,95 m, como carvalhos adultos) chegam a 10° para abrir o ombro direito e
/// dar mais curso ao golpe de machado.
pub fn tree_chop_yaw_deg(trunk_radius: Option<f32>) -> f32 {
    const MIN_DEG: f32 = 4.0;
    const DEFAULT_DEG: f32 = 5.0;
    const MAX_DEG: f32 = 10.0;
    const THIN_RADIUS: f32 = 0.30;
    const THICK_RADIUS: f32 = 0.95;

    let Some(radius) = trunk_radius.filter(|r| r.is_finite() && *r > 1e-3) else {
        return DEFAULT_DEG;
    };
    let t = ((radius - THIN_RADIUS) / (THICK_RADIUS - THIN_RADIUS)).clamp(0.0, 1.0);
    MIN_DEG + (MAX_DEG - MIN_DEG) * t
}

/// Roda uma direção horizontal XZ no sentido horário quando vista de cima (+Y para cima).
pub fn rotate_clockwise_xz(dir: Vec2, degrees: f32) -> Vec2 {
    (Quat::from_rotation_y(-degrees.to_radians()) * Vec3::new(dir.x, 0.0, dir.y)).xz()
}

/// Calcula posicionamento e orientação do herói para o início de um golpe de ferramenta.
fn tool_attack_motion(
    player: Vec3,
    target_center: Vec3,
    contact: Vec3,
    standing_tree: bool,
    flora_trunk_radius: Option<f32>,
    secs: f32,
) -> crate::player::AttackMotion {
    let toward = (contact - player)
        .xz()
        .try_normalize()
        .or_else(|| (target_center - player).xz().try_normalize());
    let dist = player.xz().distance(contact.xz());
    if !standing_tree {
        return crate::player::AttackMotion {
            secs,
            slow: 0.25,
            // Close the last half-step on approach. Once planted, subsequent blows pivot in place.
            lunge: (dist - TOOL_STANDOFF).clamp(0.0, TOOL_MAX_LUNGE),
            step: Vec2::ZERO,
            face: toward,
        };
    }
    let delta = dist - TREE_CHOP_STANDOFF;
    let step = if delta.abs() > TREE_CHOP_STANDOFF_DEADZONE {
        toward.map_or(Vec2::ZERO, |dir| {
            dir * delta.clamp(-TOOL_MAX_LUNGE, TOOL_MAX_LUNGE)
        })
    } else {
        Vec2::ZERO
    };
    let contact_radius = contact.xz().distance(target_center.xz());
    let trunk_radius = match flora_trunk_radius.filter(|r| r.is_finite() && *r > 1e-3) {
        Some(flora_r) => Some(flora_r.max(contact_radius)),
        None => (contact_radius > 1e-3).then_some(contact_radius),
    };
    let yaw_deg = tree_chop_yaw_deg(trunk_radius);
    let face = toward.map(|dir| rotate_clockwise_xz(dir, yaw_deg));
    crate::player::AttackMotion {
        secs,
        slow: 0.25,
        lunge: 0.0,
        step,
        face,
    }
}

/// J num alvo em foco: um golpe; no último, loot/XP/persistência/quebra.
#[allow(clippy::too_many_arguments)]
fn harvest_input(
    mut commands: Commands,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    actions: Option<Res<vael_engine::input_map::ActionState>>,
    map: Option<Res<vael_engine::input_map::ActionMap>>,
    mut focus: ResMut<HarvestFocus>,
    mut save: Option<ResMut<VaelSave>>,
    content: Option<Res<VaelContent>>,
    typing: Option<Res<UiFocusedInput>>,
    interaction: Option<Res<InteractionFocus>>,
    time: Res<Time>,
    epoch: Option<Res<SceneEpoch>>,
    hits: Res<HitQueue>,
    mut harvestables: Query<(
        &mut Harvestable,
        &GlobalTransform,
        Option<&Transform>,
        Option<&PooledTree>,
        Option<&PooledLook>,
    )>,
    mut services: HarvestServices,
    players: Query<
        (
            Entity,
            &GlobalTransform,
            Option<&crate::player::VaelController>,
        ),
        (With<Player>, Without<crate::boat::RidingBoat>),
    >,
    queue: Option<Res<QuestEventQueue>>,
) {
    // "Atacar" (J por defeito) pelo input do Vael: segue o rebind e o
    // contexto de jogo (parado com menus abertos).
    let Some((hero, player_pose, controller)) = players.iter().next() else {
        return;
    };
    let uses_tool = matches!(focus.tool.as_str(), "chop" | "mine");
    let touching = if let Some((target, delay)) = focus.pending_swing.take() {
        if focus.blocked
            || focus.entity != Some(target)
            || controller.is_some_and(|c| c.dodge.is_some())
        {
            return;
        }
        let left = delay - time.delta_secs();
        let waiting = if uses_tool {
            match services.mining.result(target) {
                mining::Contact::Touch(_) => false,
                mining::Contact::Unshaped => left > 0.0,
                mining::Contact::Pending if left > -0.65 => true,
                _ => return,
            }
        } else {
            left > 0.0
        };
        if waiting {
            focus.pending_swing = Some((target, left));
            return;
        }
        true
    } else {
        if focus.blocked
            || focus.cooldown > 0.0
            || !(crate::input::attack_just(actions.as_deref(), map.as_deref(), keys.as_deref())
                || uses_tool
                    && (services
                        .mouse
                        .as_ref()
                        .is_some_and(|m| m.just_pressed(MouseButton::Left))
                        || focus.entity.is_some() && focus.held_target == focus.entity))
        {
            return;
        }
        false
    };
    let attack_key = attack_key(map.as_deref());
    // Com um `UiInput` focado as teclas são texto (o `vael.input` da engine
    // também as bloqueia): o J do chat não pode partir uma árvore.
    if typing.as_deref().is_some_and(|typing| typing.0.is_some()) {
        return;
    }
    let Some(entity) = focus.entity else {
        return;
    };
    // Outro [J] nativo mais próximo é dono do prompt (o `other_j_prompt`).
    if let Some(interaction) = interaction.as_deref()
        && let Some(winner) = interaction.winner(attack_key)
        && winner != entity
    {
        return;
    }
    let Ok((mut harvestable, transform, local, pooled, look)) = harvestables.get_mut(entity) else {
        focus.entity = None;
        return;
    };
    let position = transform.translation();
    let player = player_pose.translation();
    let style = harvestable.style;
    let kind = harvestable.kind.clone();
    let tool = harvestable.tool();
    let stone = tool == "mine";
    let uses_tool = stone || tool == "chop";
    if !touching && focus.grip_remaining > 0.0 && focus.tool != tool {
        return;
    }
    if uses_tool && !touching {
        if controller.is_some_and(|c| c.dodge.is_some()) {
            return;
        }
        focus.held_target = Some(entity);
        let (duration, contact) = if stone {
            (0.98, 0.98 * 0.41)
        } else {
            (0.90, 0.90 * (0.43 / 1.5))
        };
        focus.cooldown = duration + 0.06;
        focus.grip_remaining = duration;
        focus.pending_swing = Some((entity, contact));
        services.mining.begin(entity);
        commands
            .entity(hero)
            .insert(vael_engine::animation::PendingGesture {
                name: tool.into(),
                duration: Some(duration),
                speed: 1.,
                blend: Some(0.065),
                age: 0.,
                lock: false,
            });
        if let Some(bus) = &services.motion {
            let standing_tree = !stone && style == BreakStyle::Fall;
            let flora_radius = standing_tree
                .then(|| {
                    let pool = services.pools.as_deref()?.get(&pooled?.pool)?;
                    let scale = effective_scale(local, Some(transform));
                    Some(pool.params.trunk.radius * (scale.x * scale.z).sqrt())
                })
                .flatten();
            bus.request_attack(tool_attack_motion(
                player,
                position,
                focus.contact,
                standing_tree,
                flora_radius,
                contact + 0.08,
            ));
        }
        return;
    }
    let timber_hit = services.logs.get(entity).ok();
    let broken = apply_hit(&mut harvestable);
    let height = harvestable.cut_height;
    let contact = focus.contact;
    let emit = |final_hit| {
        if uses_tool {
            hits.push(ToolHit {
                surface: services.mining.surface(entity).or_else(|| {
                    timber_hit.map(|_| mining::Surface {
                        collider: entity,
                        point: contact,
                        normal: (player + Vec3::Y * 0.8 - contact).normalize_or(Vec3::Y),
                    })
                }),
                stone,
                target: entity,
                base: position,
                height,
                outward: (player - position).xz(),
                final_hit,
            });
        }
    };
    if !uses_tool {
        focus.cooldown = SWING_COOLDOWN;
        commands
            .entity(hero)
            .insert(vael_engine::animation::PendingGesture {
                name: "gather".into(),
                duration: Some(0.55),
                speed: 1.0,
                blend: Some(0.06),
                age: 0.,
                lock: false,
            });
    }
    if !broken {
        emit(false);
        if let Some(flashes) = services.hit_flashes.as_deref() {
            // Árvores procedurais já têm `TreeShakes`; rochas/arbustos/troncos
            // recebem também um micro squash-and-stretch elástico.
            flashes.push(entity, pooled.is_none(), 1.0);
        }
        play_sfx(&mut commands, hit_clip(style, &kind), position);
        return;
    }

    // Loot e XP: o crédito é TENTADO antes de partir; com a bolsa cheia o
    // parcial é devolvido e o prop fica (uma mão-cheia de frutas não pode
    // pagar XP por nada).
    let mut added = 0;
    let mut full = false;
    if let (Some(content), Some(save)) = (content.as_deref(), save.as_deref_mut()) {
        let vault = VaultRules::from_items(&content.items);
        let xp_rules = XpRules::from_progression(&content.progression);
        seed_inventory(&mut save.inventory, &kind);
        seed_xp(&mut save.progression);
        let rewards = grant_break_rewards(
            &mut save.inventory,
            &mut save.progression,
            &vault,
            &xp_rules,
            &kind,
            harvestable.amount,
        );
        added = rewards.added;
        full = rewards.full;
        if !full {
            // Escreve também o XP quando não há recurso (`added == 0`): o
            // ganho continua a ser pago e não pode perder-se no snapshot.
            patch_rewards(&kind, &save.inventory, &save.progression);
            if added > 0
                && let Some(queue) = queue.as_deref()
            {
                queue.push(QuestEvent::SyncCollect);
            }
        }
    }
    emit(!full);
    if !full && let (Some(log), Some(save)) = (timber_hit, save.as_deref_mut()) {
        save.timber.remove(&log.id);
    }
    if !full {
        crate::collect_fx::subtle_camera_shake(
            services.shake.as_deref_mut(),
            if style == BreakStyle::Fall {
                0.14
            } else {
                0.09
            },
        );
        if let Some(bursts) = services.collect_fx.as_deref() {
            let has_crystal = services
                .children
                .get(entity)
                .is_ok_and(|kids| kids.iter().any(|k| services.crystals.contains(k)));
            let visual = if has_crystal {
                crate::collect_fx::CollectVisualKind::Crystal
            } else if !kind.is_empty() {
                crate::collect_fx::CollectVisualKind::from_resource_kind(&kind)
            } else if tool == "chop" {
                crate::collect_fx::CollectVisualKind::Wood
            } else if tool == "mine" {
                crate::collect_fx::CollectVisualKind::Stone
            } else {
                crate::collect_fx::CollectVisualKind::Berry
            };
            let count = (added.max(harvestable.amount).clamp(2, 6)) as u8;
            bursts.push(visual, position + Vec3::Y * 0.75, count);
        }
    }
    if full {
        // O Lua deixa o prop a um golpe de partir e tenta outra vez depois de
        // fazer espaço.
        harvestable.remaining = 1;
        play_sfx(&mut commands, SfxClip::Error, position);
        show_popup(
            &mut commands,
            position + Vec3::Y * 1.4,
            "Bolsa cheia".to_string(),
            FULL_POPUP_COLOR,
        );
        return;
    }

    // Persistência: chave estável + pose do substituto (para o reap).
    let key = harvestable.stable_key.clone();
    let replacement = harvestable.known_replacement().map(str::to_string);
    let replace_scale = harvestable.replace_scale;
    let popup = harvestable.popup.clone();
    let popup_color = harvestable.popup_color;
    if let Some(key) = key.as_deref() {
        mark_harvested(save.as_deref_mut(), key);
        if let Some(proto) = replacement.as_deref() {
            store_stump(save.as_deref_mut(), key, proto, position, replace_scale);
        }
    }
    // Quebra: substituto, topple ou despawn.
    if let Some(proto) = replacement.as_deref() {
        if let Some(spawns) = services.spawns.as_deref_mut() {
            spawns.0.push(PendingPrototypeSpawn {
                name: proto.to_string(),
                pos: position,
                // O Lua passa o Y autorado; `seat: false` mantém a pose.
                seat: false,
                on_spawned: None,
                caller_path: None,
            });
            // O pedido da engine não leva escala: aplica-se à raiz quando o
            // `apply_script_spawns` a materializar (ver [`ReplacementScale`]).
            if let Some(scales) = services.scales.as_deref_mut() {
                scales.0.push(ReplacementScale {
                    x: position.x,
                    z: position.z,
                    scale: replace_scale,
                    age: 0.0,
                });
            }
        }
        commands.entity(entity).try_despawn();
    } else if style == BreakStyle::Fall {
        let direction = (position - player).with_y(0.).normalize_or(Vec3::X);
        if let Some(pooled) = pooled {
            // Árvore procedural: o `bevy_flora` corta-a (toco, queda, faces de
            // corte). A árvore passa para uma entidade RAIZ com a pose GLOBAL
            // (ver `FloraFellTree`); a estação fica com os props e o toco.
            let cut_height = harvestable.cut_height;
            let tree = spawn_felled_tree(
                &mut commands,
                pooled,
                look,
                transform,
                direction,
                cut_height,
                false,
                fell_epoch(epoch.as_deref()),
            );
            regrowth::leave_root(&mut commands, entity, tree, key.clone());
            if let Some(key) = key.as_deref() {
                commands.entity(tree).insert(FloraFellKey(key.to_string()));
                store_flora_fell(save.as_deref_mut(), key, direction, cut_height, false);
            }
        } else {
            // Prop antigo (era GLB): tombo da engine, como antes.
            let initial = local.map(|local| local.rotation).unwrap_or_default();
            commands.entity(entity).try_remove::<(
                Harvestable,
                ScriptInteraction,
                vael_engine::luau::components::LuaScriptRef,
            )>();
            commands.entity(entity).try_insert(Falling {
                axis: Vec3::new(direction.z, 0.0, -direction.x),
                timer: 0.0,
                initial,
            });
        }
    } else {
        commands.entity(entity).try_despawn();
    }
    focus.entity = None;

    play_sfx(
        &mut commands,
        if style == BreakStyle::Fall && pooled.is_some() {
            SfxClip::ChopHit
        } else {
            break_clip(style, &kind)
        },
        position,
    );
    if !popup.is_empty() {
        let text = if added > 0 {
            format!("+{added} {popup}")
        } else {
            popup
        };
        show_popup(&mut commands, position + Vec3::Y * 1.4, text, popup_color);
    }
}

/// Lê o cofre (store do processo) para o estado tipado antes de somar (o
/// snapshot de 10 Hz pode estar atrasado).
fn seed_inventory(inventory: &mut Inventory, kind: &str) {
    if kind.is_empty() {
        return;
    }
    if let Some(count) = read_game_i64(&item_key(kind)) {
        inventory.set(kind, count);
    }
}

fn seed_xp(state: &mut XpState) {
    let mut kv = xp_map_from_state(state);
    for key in XP_KEYS {
        if let Some(value) = read_game_f64(key) {
            kv.insert(key.to_string(), value);
        }
    }
    apply_xp_map(&kv, state);
}

fn patch_rewards(kind: &str, inventory: &Inventory, xp: &XpState) {
    let mut patch = BTreeMap::new();
    if !kind.is_empty() {
        patch.insert(item_key(kind), json!(inventory.get(kind)));
    }
    for key in XP_KEYS {
        let value = match key {
            KEY_XP => xp.xp as f64,
            KEY_XP_NEXT => xp.xp_next as f64,
            KEY_LEVEL => xp.level as f64,
            KEY_SKILL_POINTS => xp.skill_points as f64,
            _ => continue,
        };
        patch.insert(key.to_string(), json!(value));
    }
    vael_engine::luau::game::apply_patch_json(&patch);
}

fn read_game_i64(key: &str) -> Option<i64> {
    vael_engine::luau::game::game_get_json(key)
        .and_then(|v| v.as_f64())
        .filter(|value| value.is_finite())
        .map(|value| value as i64)
}

fn read_game_f64(key: &str) -> Option<f64> {
    vael_engine::luau::game::game_get_json(key)
        .and_then(|v| v.as_f64())
        .filter(|value| value.is_finite())
}

fn hit_clip(style: BreakStyle, kind: &str) -> SfxClip {
    match tool_of(style, kind) {
        "chop" => SfxClip::ChopHit,
        "mine" => SfxClip::MineHit,
        _ => SfxClip::Harvest,
    }
}

fn break_clip(style: BreakStyle, kind: &str) -> SfxClip {
    match tool_of(style, kind) {
        "chop" => SfxClip::ChopBreak,
        "mine" => SfxClip::MineBreak,
        _ => SfxClip::Harvest,
    }
}

fn play_sfx(commands: &mut Commands, clip: SfxClip, position: Vec3) {
    commands.queue(move |world: &mut World| {
        if world.get_resource::<Messages<SfxEvent>>().is_some() {
            world.write_message(SfxEvent {
                clip,
                position: Some(position),
            });
        }
    });
}

fn show_popup(commands: &mut Commands, position: Vec3, text: String, color: [f32; 3]) {
    commands.queue(move |world: &mut World| {
        if world
            .get_resource::<Messages<DamageNumberEvent>>()
            .is_some()
        {
            world.write_message(DamageNumberEvent {
                position,
                text,
                color: Color::srgb(color[0], color[1], color[2]),
            });
        }
    });
}

// ── reap do save ────────────────────────────────────────────────────────

fn kv_snapshot(save: Option<&VaelSave>) -> BTreeMap<String, Json> {
    // Store do processo primeiro (o `vael.game()` dos contextos BMS); sem
    // chaves (arranque), o legacy do save.
    let kv = vael_engine::luau::game::game_to_json_process();
    if !kv.is_empty() {
        return kv;
    }
    save.map(|save| save.legacy.clone()).unwrap_or_default()
}

fn reap_saved(
    commands: &mut Commands,
    save: Option<&VaelSave>,
    mut spawns: Option<&mut PendingScriptSpawns>,
    mut scales: Option<&mut PendingReplacementScales>,
    epoch: u64,
    helper: &TransformHelper<'_, '_>,
    harvestables: &Query<(
        Entity,
        &Harvestable,
        Option<&PooledTree>,
        Option<&PooledLook>,
        Option<&FloraFellTree>,
        &GlobalTransform,
    )>,
) {
    // Um só snapshot por varredura: `harvested.*` e `stump.*` saem do mesmo
    // mapa (o snapshot clona o jogo inteiro).
    let kv = kv_snapshot(save);
    let mut harvested = save.map(|save| save.harvested.clone()).unwrap_or_default();
    for (key, value) in &kv {
        if let Some(name) = key.strip_prefix("harvested.")
            && matches!(value, Json::Bool(true))
        {
            harvested.insert(name.to_string());
        }
    }
    if harvested.is_empty() {
        return;
    }
    for (entity, harvestable, pooled, look, fell_tree, global) in harvestables.iter() {
        let Some(matched) = reaped_key(harvestable, &harvested) else {
            continue;
        };
        // Árvore procedural já tombada: o save diz COMO está deitada. Repõe-se
        // a árvore-raiz já deitada (`FellTree { lying }`), com toco e, se
        // `bucked`, os toros — em vez de despawnar. Uma só vez por estação.
        let fell_key = harvestable
            .stable_key
            .as_ref()
            .filter(|key| kv.contains_key(&format!("flora-fell.{key}.dx")))
            .unwrap_or(&matched);
        if let Some(fell) = flora_fell_from_kv(&kv, fell_key) {
            let Some(pooled) = pooled else { continue };
            if fell_tree.is_some() {
                continue;
            }
            // `GlobalTransform` ainda está no DEFAULT no `PostStartup`/no
            // `SceneLoad` (o `TransformPlugin` só propaga no `PostUpdate`): a
            // árvore reposta nascia na origem do mundo, à escala 1. A pose
            // global da estação vem do `TransformHelper`, que a compõe dos
            // `Transform` da hierarquia.
            let global = helper.compute_global_transform(entity).unwrap_or(*global);
            let tree = spawn_felled_tree(
                commands,
                pooled,
                look,
                &global,
                Vec3::new(fell.dx, 0.0, fell.dz),
                fell.cut_height,
                true,
                epoch,
            );
            commands.entity(tree).insert(FloraFellKey(fell_key.clone()));
            // Older slots only recorded whether the trunk had been sectioned.
            // That operation never paid out timber; restore its collectable wood once.
            regrowth::leave_root(commands, entity, tree, Some(fell_key.clone()));
            continue;
        }
        commands.entity(entity).try_despawn();
        let Some(stump) = stump_for(&kv, &matched) else {
            continue;
        };
        let Some(spawns) = spawns.as_deref_mut() else {
            continue;
        };
        spawns.0.push(PendingPrototypeSpawn {
            name: stump.proto,
            pos: Vec3::new(stump.x, stump.y.unwrap_or(0.0), stump.z),
            seat: stump.y.is_none(),
            on_spawned: None,
            caller_path: None,
        });
        // A escala autorada do substituto (`stump.<chave>.scale`) volta com
        // ele; sem isto o rebento ficava à escala do protótipo.
        if let Some(scales) = scales.as_deref_mut() {
            scales.0.push(ReplacementScale {
                x: stump.x,
                z: stump.z,
                scale: stump.scale,
                age: 0.0,
            });
        }
    }
}

/// A same-scene load keeps the ECS entities. Reconcile those trees against the new slot.
pub(crate) fn reload_trees(world: &mut World) {
    use bevy::ecs::system::RunSystemOnce;
    if world.contains_resource::<VaelHarvestConfig>() {
        if let Some(mut focus) = world.get_resource_mut::<HarvestFocus>() {
            focus.pending_swing = None;
            focus.held_target = None;
            focus.grip_remaining = 0.0;
        }
        if let Err(err) = world
            .run_system_once(timber::restore)
            .and_then(|()| world.run_system_once(regrowth::reset_on_load))
            .and_then(|()| world.run_system_once(reap_on_load))
        {
            warn!("harvest: erro ao restaurar as árvores: {err}");
        }
    }
}

/// PostStartup/SceneLoad: apaga já os props que o save diz estarem colhidos.
fn reap_on_load(
    mut commands: Commands,
    save: Option<Res<VaelSave>>,
    mut spawns: Option<ResMut<PendingScriptSpawns>>,
    mut scales: Option<ResMut<PendingReplacementScales>>,
    epoch: Option<Res<SceneEpoch>>,
    helper: TransformHelper,
    harvestables: Query<(
        Entity,
        &Harvestable,
        Option<&PooledTree>,
        Option<&PooledLook>,
        Option<&FloraFellTree>,
        &GlobalTransform,
    )>,
) {
    reap_saved(
        &mut commands,
        save.as_deref(),
        spawns.as_deref_mut(),
        scales.as_deref_mut(),
        fell_epoch(epoch.as_deref()),
        &helper,
        &harvestables,
    );
}

/// O botão Carregar do menu pode trazer o save DEPOIS do arranque (o
/// `reap_saved` do Lua também não é one-shot).
fn reap_periodic(
    time: Res<Time>,
    mut timer: Local<Option<Timer>>,
    mut commands: Commands,
    save: Option<Res<VaelSave>>,
    mut spawns: Option<ResMut<PendingScriptSpawns>>,
    mut scales: Option<ResMut<PendingReplacementScales>>,
    epoch: Option<Res<SceneEpoch>>,
    helper: TransformHelper,
    harvestables: Query<(
        Entity,
        &Harvestable,
        Option<&PooledTree>,
        Option<&PooledLook>,
        Option<&FloraFellTree>,
        &GlobalTransform,
    )>,
) {
    let timer =
        timer.get_or_insert_with(|| Timer::from_seconds(REAP_PERIOD_SECS, TimerMode::Repeating));
    if !timer.tick(time.delta()).just_finished() {
        return;
    }
    reap_saved(
        &mut commands,
        save.as_deref(),
        spawns.as_deref_mut(),
        scales.as_deref_mut(),
        fell_epoch(epoch.as_deref()),
        &helper,
        &harvestables,
    );
}

/// Marca os tocos e toros que o `bevy_flora` cria (entidades RAIZ suas) com a
/// geração da árvore de que nasceram: o toco guarda a árvore em `TreeStump` e
/// o toro o toco, por isso a geração propaga-se sem um `CurrentScene`.
fn tag_fell_entities(
    mut commands: Commands,
    epoch: Option<Res<SceneEpoch>>,
    epochs: Query<&FellEpoch>,
    stumps: Query<(Entity, &TreeStump), (Added<TreeStump>, Without<FellEpoch>)>,
    logs: Query<(Entity, &TreeLog), (Added<TreeLog>, Without<FellEpoch>)>,
) {
    if stumps.is_empty() && logs.is_empty() {
        return;
    }
    let current = fell_epoch(epoch.as_deref());
    for (entity, stump) in &stumps {
        let inherited = stump
            .tree
            .and_then(|tree| epochs.get(tree).ok())
            .map_or(current, |epoch| epoch.0);
        commands.entity(entity).insert(FellEpoch(inherited));
    }
    for (entity, log) in &logs {
        let inherited = epochs.get(log.stump).map_or(current, |epoch| epoch.0);
        commands.entity(entity).insert(FellEpoch(inherited));
    }
}

/// Limpa as raízes do `bevy_flora` da cena que saiu: árvores tombadas
/// (`FellTree`/`Felled`), os **tocos** (`TreeStump`, uma raiz criada pelo
/// crate — sem isto a cena seguinte herdava um toco com malha órfã) e os
/// **toros** (`TreeLog`, criados ao retalhar). O estado persiste no save e o
/// restauro (no `SceneLoad`, já com a geração nova) repõe-nos.
fn despawn_fell_trees(
    mut commands: Commands,
    epoch: Option<Res<SceneEpoch>>,
    trees: Query<
        (Entity, &FellEpoch),
        Or<(With<FellTree>, With<Felled>, With<TreeStump>, With<TreeLog>)>,
    >,
) {
    let current = fell_epoch(epoch.as_deref());
    for (entity, born) in &trees {
        if born.0 != current {
            commands.entity(entity).despawn();
        }
    }
}

/// Escreve a escala do substituto na raiz que o `apply_script_spawns`
/// materializou (ver [`ReplacementScale`]).
fn apply_replacement_scales(
    time: Res<Time>,
    mut pending: ResMut<PendingReplacementScales>,
    mut spawned: Query<(&mut Transform, Option<&ChildOf>), Added<Transform>>,
) {
    if pending.0.is_empty() {
        return;
    }
    let dt = time.delta_secs();
    pending.0.retain_mut(|request| {
        let mut matched = false;
        for (mut transform, parent) in &mut spawned {
            // A raiz do protótipo não tem pai; um filho local (0,0) não é o
            // substituto (e um casamento errado escalava a malha errada).
            if parent.is_some() {
                continue;
            }
            if (transform.translation.x - request.x).abs() <= 1e-3
                && (transform.translation.z - request.z).abs() <= 1e-3
            {
                transform.scale = Vec3::splat(request.scale);
                matched = true;
                break;
            }
        }
        if matched {
            return false;
        }
        request.age += dt;
        request.age <= REPLACEMENT_SCALE_TIMEOUT
    });
}

/// Embate da árvore cortada: o `bevy_flora` avisa com `TreeLanded` no primeiro
/// contacto com o chão. Som de queda é do jogo (o crate já larga as folhas).
fn tree_landed_feedback(
    mut landed: Option<MessageReader<TreeLanded>>,
    mut sfx: Option<MessageWriter<SfxEvent>>,
) {
    let Some(landed) = landed.as_mut() else {
        return;
    };
    let Some(sfx) = sfx.as_mut() else {
        // Consome as mensagens para o buffer não crescer.
        for _ in landed.read() {}
        return;
    };
    for message in landed.read() {
        sfx.write(SfxEvent {
            clip: SfxClip::ChopBreak,
            position: Some(message.at),
        });
    }
}

/// `[J]` numa árvore já deitada: publica "Cortar em toros" e retalha-a
/// (`BuckTree`) — o `bevy_flora` substitui-a por `TreeLog`s no chão.
#[allow(clippy::too_many_arguments)]
fn buck_felled_trees(
    mut commands: Commands,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    actions: Option<Res<vael_engine::input_map::ActionState>>,
    map: Option<Res<vael_engine::input_map::ActionMap>>,
    time: Res<Time>,
    mut focus: ResMut<BuckFocus>,
    interaction: Option<Res<InteractionFocus>>,
    mut save: Option<ResMut<VaelSave>>,
    collect_fx: Option<Res<crate::collect_fx::CollectBurstQueue>>,
    mut shake: Option<ResMut<bevy_ext_engine::camera_glue::CameraShake>>,
    players: Query<&GlobalTransform, (With<Player>, Without<crate::boat::RidingBoat>)>,
    felled: Query<
        (
            Entity,
            &GlobalTransform,
            Option<&BuckTree>,
            Option<&FloraFellKey>,
            Option<&Felled>,
            Option<&FellTree>,
        ),
        (
            Or<(With<Felled>, With<FellTree>)>,
            Without<timber::FallingTrunk>,
        ),
    >,
) {
    focus.cooldown = (focus.cooldown - time.delta_secs()).max(0.0);
    let attack_key = attack_key(map.as_deref());
    let Some(player) = players.iter().next().map(GlobalTransform::translation) else {
        return;
    };
    let reach = BUCK_RANGE * range_scale();
    let mut best: Option<(Entity, f32)> = None;
    for (entity, transform, bucked, _, state, request) in &felled {
        if bucked.is_some()
            || !(state.is_some_and(|s| s.at_rest) || request.is_some_and(|r| r.lying))
        {
            continue;
        }
        let distance = transform.translation().distance(player);
        if distance <= reach && best.is_none_or(|(_, best)| distance < best) {
            best = Some((entity, distance));
        }
    }
    let target = best.map(|(entity, _)| entity);
    if focus.entity != target {
        if let Some(previous) = focus.entity
            && Some(previous) != target
        {
            commands.entity(previous).try_remove::<ScriptInteraction>();
        }
        if let Some(target) = target {
            commands.entity(target).try_insert(ScriptInteraction {
                label: "Cortar em toros".to_string(),
                key: attack_key,
                range: reach,
            });
        }
        focus.entity = target;
    }
    let Some(target) = target else {
        return;
    };
    if !crate::input::attack_just(actions.as_deref(), map.as_deref(), keys.as_deref())
        || focus.cooldown > 0.0
    {
        return;
    }
    // Outro [J] nativo mais próximo (o prompt da colheita) tem prioridade.
    if let Some(interaction) = interaction.as_deref()
        && let Some(winner) = interaction.winner(attack_key)
        && winner != target
    {
        return;
    }
    commands.entity(target).try_insert(BuckTree {
        log_length: LOG_LENGTH,
        min_radius: LOG_MIN_RADIUS,
    });
    // Persistência ANTES: o crate pode despawnar a entidade no mesmo frame.
    if let Ok((_, tf, _, key, _, _)) = felled.get(target) {
        if let Some(key) = key {
            store_flora_bucked(save.as_deref_mut(), &key.0);
        }
        crate::collect_fx::subtle_camera_shake(shake.as_deref_mut(), 0.09);
        if let Some(bursts) = collect_fx.as_deref() {
            bursts.push(
                crate::collect_fx::CollectVisualKind::Wood,
                tf.translation() + Vec3::Y * 0.55,
                4,
            );
        }
    }
    focus.cooldown = BUCK_COOLDOWN;
    focus.entity = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cada nó colhível do mundo dá o mesmo `Harvestable` pelos componentes
    /// tipados e pelas strings do DSL (que o loader ainda reconstrói).
    #[test]
    fn specs_e_props_dao_o_mesmo() {
        let world = Path::new(env!("CARGO_MANIFEST_DIR")).join("world.bsn");
        let loaded = bevy_ext_world::bsn::load_bsn_world(&world).expect("world.bsn");
        fn walk(nodes: &[bevy_ext_world::xml::XmlNode], checked: &mut usize) {
            for node in nodes {
                if let Some(origin) = &node.origin
                    && let Some(specs) = bevy_ext_world::bsn::specs::specs_for(origin)
                    && (specs.destructible.is_some() || specs.harvest.is_some())
                {
                    let props: Vec<(String, String)> = node
                        .attrs
                        .iter()
                        .filter(|(k, _)| {
                            matches!(
                                k.as_str(),
                                "destructible" | "resource-kind" | "resource-yield"
                            )
                        })
                        .cloned()
                        .collect();
                    let by_props = Harvestable::from_props(&props, "n", None);
                    let by_specs = Harvestable::from_specs(
                        specs.destructible.as_ref(),
                        specs.harvest.as_ref(),
                        "n",
                        None,
                    );
                    assert_eq!(by_specs, by_props, "{:?}", node.attrs);
                    *checked += 1;
                }
                walk(&node.children, checked);
            }
        }
        let mut checked = 0;
        walk(&loaded.nodes, &mut checked);
        assert!(checked > 2000, "só {checked} nós colhíveis com specs");
    }
    use bevy::MinimalPlugins;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::input::ButtonInput;
    use std::path::PathBuf;
    use std::sync::Arc;
    use vael_engine::xml::NodeOrigin;

    fn props(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    fn source(file: &str, path: &[u32]) -> SceneSource {
        SceneSource(Arc::new(NodeOrigin {
            file: Arc::from(Path::new(file)),
            path: path.to_vec(),
        }))
    }

    fn harvestable(
        name: &str,
        key: Option<&str>,
        hits: i64,
        range: f32,
        kind: &str,
        amount: i64,
    ) -> Harvestable {
        Harvestable {
            hits,
            remaining: hits,
            style: BreakStyle::Shatter,
            kind: kind.to_string(),
            amount,
            popup: "Pedra".to_string(),
            popup_color: DEFAULT_POPUP_COLOR,
            range,
            cut_height: 0.5,
            replacement: String::new(),
            replace_scale: 1.0,
            name: name.to_string(),
            stable_key: key.map(str::to_string),
        }
    }

    fn app_rust() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(VaelHarvestConfig { rust: true })
            .init_resource::<VaelSave>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<QuestEventQueue>()
            .add_plugins(VaelHarvestPlugin);
        app
    }

    fn finish_chop(app: &mut App) {
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(100),
        ));
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear();
        for _ in 0..3 {
            app.update();
        }
    }

    fn spawn_player(app: &mut App) -> Entity {
        app.world_mut()
            .spawn((
                Player::default(),
                Transform::default(),
                GlobalTransform::default(),
            ))
            .id()
    }

    fn spawn_prop(app: &mut App, harvestable: Harvestable) -> Entity {
        app.world_mut()
            .spawn((
                harvestable,
                Transform::default(),
                GlobalTransform::default(),
            ))
            .id()
    }

    // ── parsing ─────────────────────────────────────────────────────────

    #[test]
    fn parse_components_le_o_component_string() {
        let parsed = parse_components(
            "hits: 3; break-style: fall; popup-text: Pedra; popup-color: #cccccc; \
             range: 2.5; replacement: berry-bush-bare; replace-scale: 1.5",
        );
        assert_eq!(parsed.get("hits").map(String::as_str), Some("3"));
        assert_eq!(parsed.get("break-style").map(String::as_str), Some("fall"));
        assert_eq!(parsed.get("popup-text").map(String::as_str), Some("Pedra"));
        assert_eq!(
            parsed.get("popup-color").map(String::as_str),
            Some("#cccccc")
        );
        assert_eq!(parsed.get("range").map(String::as_str), Some("2.5"));
        assert_eq!(
            parsed.get("replacement").map(String::as_str),
            Some("berry-bush-bare")
        );
        assert_eq!(parsed.get("replace-scale").map(String::as_str), Some("1.5"));
    }

    #[test]
    fn parse_components_malformado_nao_rebenta() {
        // Chave inválida no meio: o gmatch do Lua ainda casa `bar`.
        let parsed = parse_components("foo bar: 1; hits: ; =; break-style: pick");
        assert_eq!(parsed.get("bar").map(String::as_str), Some("1"));
        assert!(!parsed.contains_key("hits"), "valor vazio não casa");
        assert_eq!(parsed.get("break-style").map(String::as_str), Some("pick"));
        assert!(parse_components("").is_empty());
    }

    #[test]
    fn parse_props_valido() {
        let props = props(&[
            (
                "destructible",
                "hits: 3; break-style: fall; popup-text: Madeira; \
                 popup-color: #abcdef; range: 2.5; replacement: berry-bush-bare; \
                 replace-scale: 1.5",
            ),
            ("resource-kind", "wood"),
            ("resource-yield", "5"),
        ]);
        let harvestable =
            Harvestable::from_props(&props, "tree", Some("tree@x".into())).expect("colhível");
        assert_eq!(harvestable.hits, 3);
        assert_eq!(harvestable.remaining, 3);
        assert_eq!(harvestable.style, BreakStyle::Fall);
        assert_eq!(harvestable.kind, "wood");
        assert_eq!(harvestable.amount, 5);
        assert_eq!(harvestable.popup, "Madeira");
        assert_eq!(harvestable.range, 2.5);
        assert_eq!(harvestable.replace_scale, 1.5);
        assert_eq!(harvestable.replacement, "berry-bush-bare");
        assert_eq!(harvestable.stable_key.as_deref(), Some("tree@x"));
        assert_eq!(harvestable.label(), "Cortar");
        assert_eq!(harvestable.tool(), "chop");
        assert_eq!(
            parse_popup_color("#abcdef"),
            [171.0 / 255.0, 205.0 / 255.0, 239.0 / 255.0]
        );
    }

    #[test]
    fn parse_props_malformado_cai_nos_defaults() {
        let props = props(&[
            (
                "destructible",
                "hits: abc; range: -2; replace-scale: 99; popup-color: #12345",
            ),
            ("resource-kind", "stone"),
            ("resource-yield", "NaN"),
        ]);
        let harvestable = Harvestable::from_props(&props, "rock", None).expect("colhível");
        assert_eq!(harvestable.hits, 3, "hits inválido vale 3");
        assert_eq!(harvestable.amount, 1, "yield inválido vale 1");
        assert_eq!(harvestable.range, RANGE_DEFAULT);
        assert_eq!(harvestable.replace_scale, 1.0);
        assert_eq!(harvestable.popup_color, DEFAULT_POPUP_COLOR);
        assert_eq!(harvestable.style, BreakStyle::Burst);
        assert_eq!(harvestable.label(), "Minerar");
        assert_eq!(harvestable.tool(), "mine");
    }

    #[test]
    fn parse_props_ausente_nao_e_colhivel() {
        assert!(Harvestable::from_props(&[], "nada", None).is_none());
        let vazio = props(&[("destructible", ""), ("resource-kind", "")]);
        assert!(Harvestable::from_props(&vazio, "nada", None).is_none());
    }

    #[test]
    fn pick_sem_kind_vira_berry_com_o_yield_do_template() {
        let input = props(&[("destructible", "break-style: pick; hits: 2")]);
        let harvestable = Harvestable::from_props(&input, "bush", None).expect("colhível");
        assert_eq!(harvestable.kind, "berry");
        assert_eq!(harvestable.amount, PICK_FALLBACK_YIELD);
        assert_eq!(harvestable.style, BreakStyle::Pick);
        assert_eq!(harvestable.label(), "Colher frutas");
        assert_eq!(harvestable.tool(), "pick");

        // O substituto `berry-bush-bare` também identifica a berry.
        let input = props(&[(
            "destructible",
            "break-style: shatter; replacement: berry-bush-bare",
        )]);
        let harvestable = Harvestable::from_props(&input, "bush", None).expect("colhível");
        assert_eq!(harvestable.kind, "berry");
        assert_eq!(harvestable.amount, PICK_FALLBACK_YIELD);
    }

    #[test]
    fn numeros_saneados_como_no_lua() {
        assert_eq!(
            Harvestable::from_props(
                &props(&[("destructible", "hits: 0"), ("resource-kind", "stone")]),
                "r",
                None
            )
            .expect("colhível")
            .hits,
            1
        );
        assert_eq!(
            Harvestable::from_props(
                &props(&[("destructible", "hits: 2.9"), ("resource-kind", "stone")]),
                "r",
                None
            )
            .expect("colhível")
            .hits,
            2
        );
        let props = props(&[
            ("destructible", ""),
            ("resource-kind", "stone"),
            ("resource-yield", "2.9"),
        ]);
        assert_eq!(
            Harvestable::from_props(&props, "r", None)
                .expect("colhível")
                .amount,
            2
        );
    }

    // ── chave estável ───────────────────────────────────────────────────

    #[test]
    fn chave_estavel_usa_nome_e_proveniencia() {
        let scene = source("/proj/world/vegetation/forest.xml", &[3, 4]);
        let root = PathBuf::from("/proj");
        assert_eq!(
            stable_key(Some("tree"), Some(&scene), Some(&root)).as_deref(),
            Some("tree@vegetation/forest.xml#3/4")
        );
        // Sem nome a proveniência sozinha identifica a instância.
        assert_eq!(
            stable_key(None, Some(&scene), Some(&root)).as_deref(),
            Some("@vegetation/forest.xml#3/4")
        );
        // Sem proveniência fica o nome (compatível com o save do Lua).
        assert_eq!(
            stable_key(Some("tree"), None, Some(&root)).as_deref(),
            Some("tree")
        );
        assert_eq!(stable_key(None, None, Some(&root)), None);
    }

    #[test]
    fn chave_estavel_sem_raiz_nao_grava_o_caminho_absoluto() {
        let scene = source("/home/jogador/Vael/world/vegetation/forest.xml", &[1]);
        assert_eq!(
            stable_key(Some("tree"), Some(&scene), None).as_deref(),
            Some("tree@vegetation/forest.xml#1")
        );
    }

    #[test]
    fn reaped_key_distingue_estavel_de_nome_antigo() {
        let mut harvested = BTreeSet::new();
        harvested.insert("tree@forest.xml#1".to_string());
        let modern = harvestable("tree", Some("tree@forest.xml#1"), 3, 3.5, "wood", 1);
        assert_eq!(
            reaped_key(&modern, &harvested).as_deref(),
            Some("tree@forest.xml#1")
        );
        // O mesmo nome noutra cena não é apanhado pela chave estável.
        let other = harvestable("tree", Some("tree@swamp.xml#1"), 3, 3.5, "wood", 1);
        assert_eq!(reaped_key(&other, &harvested), None);

        // Save antigo com o nome puro: casa pelo `Name` (a ambiguidade entre
        // cenas é a mesma do `reap_saved` do Lua).
        let mut old = BTreeSet::new();
        old.insert("tree".to_string());
        assert_eq!(reaped_key(&modern, &old).as_deref(), Some("tree"));
        let legacy = harvestable("tree", None, 3, 3.5, "wood", 1);
        assert_eq!(reaped_key(&legacy, &old).as_deref(), Some("tree"));
    }

    #[test]
    fn stump_ida_e_volta() {
        let mut kv = BTreeMap::new();
        kv.insert(
            "stump.tree@forest.xml#1.proto".into(),
            json!("berry-bush-bare"),
        );
        kv.insert("stump.tree@forest.xml#1.x".into(), json!(1.5));
        kv.insert("stump.tree@forest.xml#1.y".into(), json!(2.0));
        kv.insert("stump.tree@forest.xml#1.z".into(), json!(-3.0));
        kv.insert("stump.tree@forest.xml#1.scale".into(), json!(1.35));
        let stump = stump_for(&kv, "tree@forest.xml#1").expect("stump");
        assert_eq!(stump.proto, "berry-bush-bare");
        assert_eq!((stump.x, stump.y, stump.z), (1.5, Some(2.0), -3.0));
        assert!((stump.scale - 1.35).abs() < 1e-6);
        // Sem `scale` (save antigo) o substituto fica à escala do protótipo.
        kv.remove("stump.tree@forest.xml#1.scale");
        assert_eq!(
            stump_for(&kv, "tree@forest.xml#1").map(|s| s.scale),
            Some(1.0)
        );
        // Fora do intervalo autorado (`replace-scale` 0,05..20) cai em 1.
        kv.insert("stump.tree@forest.xml#1.scale".into(), json!(99.0));
        assert_eq!(
            stump_for(&kv, "tree@forest.xml#1").map(|s| s.scale),
            Some(1.0)
        );
        // Sem protótipo conhecido não há stump.
        kv.insert("stump.tree@forest.xml#1.proto".into(), json!("nao-existe"));
        assert!(stump_for(&kv, "tree@forest.xml#1").is_none());
    }

    // ── foco/alcance ────────────────────────────────────────────────────

    #[test]
    fn nearest_within_escolhe_o_mais_perto_e_respeita_o_alcance() {
        let near = Entity::from_raw_u32(1).expect("id");
        let far = Entity::from_raw_u32(2).expect("id");
        let out = Entity::from_raw_u32(3).expect("id");
        let chosen = nearest_within(
            Vec3::ZERO,
            [
                (far, Vec3::new(3.0, 0.0, 0.0), 3.5),
                (near, Vec3::new(1.0, 0.0, 0.0), 3.5),
                (out, Vec3::new(10.0, 0.0, 0.0), 3.5),
            ],
        );
        assert_eq!(chosen.map(|(entity, _)| entity), Some(near));
        assert_eq!(
            nearest_within(Vec3::ZERO, [(out, Vec3::new(10.0, 0.0, 0.0), 3.5)]),
            None
        );
    }

    #[test]
    fn foco_publica_prompt_e_remove_ao_sair_do_alcance() {
        let mut app = app_rust();
        let _player = spawn_player(&mut app);
        let prop = spawn_prop(
            &mut app,
            harvestable("tree", Some("tree@test"), 3, 3.5, "wood", 1),
        );
        app.update();

        assert_eq!(
            app.world().resource::<HarvestFocus>().entity,
            Some(prop),
            "o prop no mesmo sítio do herói é o foco"
        );
        let interaction = app
            .world()
            .get::<ScriptInteraction>(prop)
            .expect("prompt publicado");
        assert_eq!(interaction.key, KeyCode::KeyJ);
        assert_eq!(interaction.label, "Cortar");
        assert!((interaction.range - vael_engine::interact::scaled_range(3.5)).abs() < 1e-5);

        // Move o prop para longe: o prompt tem de sair.
        if let Some(mut transform) = app.world_mut().get_mut::<GlobalTransform>(prop) {
            *transform = GlobalTransform::from_translation(Vec3::new(100.0, 0.0, 0.0));
        }
        app.update();
        assert_eq!(app.world().resource::<HarvestFocus>().entity, None);
        assert!(app.world().get::<ScriptInteraction>(prop).is_none());
    }

    #[test]
    fn segurar_o_clique_na_casca_repete_o_corte_e_soltar_encerra_o_ciclo() {
        let mut app = app_rust();
        app.insert_resource(VaelHarvestConfig { rust: false })
            .init_resource::<ButtonInput<MouseButton>>()
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(100),
            ));
        spawn_player(&mut app);
        let tree = spawn_prop(&mut app, harvestable("oak", None, 7, 3.5, "wood", 3));
        app.world_mut().entity_mut(tree).insert((
            avian3d::prelude::Collider::cylinder(0.9, 3.0),
            GlobalTransform::from_translation(Vec3::new(2.2, 0., 0.)),
        ));
        app.update();
        assert_eq!(app.world().resource::<HarvestFocus>().entity, Some(tree));
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .clear();
        let mut times = Vec::new();
        let mut previous = 7;
        for tick in 1..=24 {
            app.update();
            let left = app.world().get::<Harvestable>(tree).unwrap().remaining;
            if left != previous {
                assert_eq!(left, previous - 1);
                times.push(tick);
                previous = left;
            }
        }
        assert_eq!(times.len(), 3);
        assert!(times.windows(2).all(|t| (9..=11).contains(&(t[1] - t[0]))));
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        for _ in 0..15 {
            app.update();
        }
        assert_eq!(app.world().get::<Harvestable>(tree).unwrap().remaining, 4);
    }

    #[test]
    fn mining_prompt_owns_the_attack_and_held_j_repeats_on_the_same_rock() {
        let mut app = app_rust();
        app.insert_resource(VaelHarvestConfig { rust: false })
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(100),
            ));
        let hero = spawn_player(&mut app);
        let mut stone = harvestable("granite", None, 6, 3.5, "stone", 3);
        stone.style = BreakStyle::Shatter;
        let rock = spawn_prop(&mut app, stone);
        app.world_mut().entity_mut(rock).insert((
            avian3d::prelude::Collider::sphere(1.0),
            GlobalTransform::from_translation(Vec3::new(2.1, 0.5, 0.)),
        ));
        app.update();
        assert_eq!(app.world().resource::<HarvestFocus>().entity, Some(rock));
        assert_eq!(
            app.world().get::<ScriptInteraction>(rock).unwrap().label,
            "Minerar"
        );
        assert_eq!(
            app.world()
                .resource::<HarvestToolInput>()
                .reserved
                .load(std::sync::atomic::Ordering::Relaxed),
            2
        );
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyJ);
        app.update();
        assert_eq!(
            app.world()
                .get::<vael_engine::animation::PendingGesture>(hero)
                .unwrap()
                .name,
            "mine"
        );
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear();
        for _ in 0..21 {
            app.update();
        }
        assert_eq!(app.world().get::<Harvestable>(rock).unwrap().remaining, 4);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(KeyCode::KeyJ);
        for _ in 0..15 {
            app.update();
        }
        assert_eq!(app.world().get::<Harvestable>(rock).unwrap().remaining, 4);
    }

    #[test]
    fn sem_player_ou_props_nao_faz_nada() {
        let mut app = app_rust();
        app.update();
        assert_eq!(app.world().resource::<HarvestFocus>().entity, None);

        let _player = spawn_player(&mut app);
        app.update();
        assert_eq!(app.world().resource::<HarvestFocus>().entity, None);
        // Sem J nem player/props não há pânico nem foco.
    }

    #[test]
    fn attach_le_os_script_props() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let entity = app
            .world_mut()
            .spawn(ScriptProps(Arc::new(props(&[
                (
                    "destructible",
                    "hits: 2; break-style: shatter; popup-text: Pedra",
                ),
                ("resource-kind", "stone"),
                ("resource-yield", "4"),
            ]))))
            .id();
        app.world_mut()
            .run_system_once(attach_harvestables)
            .expect("attach");
        let harvestable = app.world().get::<Harvestable>(entity).expect("colhível");
        assert_eq!(harvestable.hits, 2);
        assert_eq!(harvestable.kind, "stone");
        assert_eq!(harvestable.amount, 4);
    }

    /// Um prop que nasce com o jogo a correr (`spawn_prototype` de um script,
    /// `live_edit`) também recebe `Harvestable`: o caminho Lua lê
    /// `vael.player_nearby` e via-o, o Rust ficava cego até ao reload.
    #[test]
    fn attach_le_props_nascidos_depois_do_load() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_systems(Update, attach_late_harvestables);
        app.update();
        let entity = app
            .world_mut()
            .spawn(ScriptProps(Arc::new(props(&[
                ("destructible", "hits: 1; break-style: shatter"),
                ("resource-kind", "stone"),
            ]))))
            .id();
        app.update();
        assert!(
            app.world().get::<Harvestable>(entity).is_some(),
            "o prop de runtime fica colhível"
        );
    }

    // ── durabilidade/loot/XP ────────────────────────────────────────────

    #[test]
    fn um_golpe_desconta_e_o_ultimo_parte() {
        let mut harvestable = harvestable("tree", None, 3, 3.5, "wood", 1);
        assert!(!apply_hit(&mut harvestable));
        assert_eq!(harvestable.remaining, 2);
        assert!(!apply_hit(&mut harvestable));
        assert_eq!(harvestable.remaining, 1);
        assert!(apply_hit(&mut harvestable));
        assert_eq!(harvestable.remaining, 0);
    }

    fn content_com(pairs: &[(&str, &str, Option<i64>)]) -> VaelContent {
        let items = pairs
            .iter()
            .map(|(id, kind, stack)| crate::content::ItemDef {
                id: id.to_string(),
                kind: kind.to_string(),
                stack: *stack,
                ..Default::default()
            })
            .collect();
        VaelContent {
            items: crate::content::ItemsFile {
                items,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn quebra_credita_loot_e_xp() {
        let content = content_com(&[("wood", "resource", None)]);
        let vault = VaultRules::from_items(&content.items);
        let xp_rules = XpRules::from_progression(&content.progression);
        let mut inventory = Inventory::default();
        let mut progression = XpState::default();

        let rewards = grant_break_rewards(
            &mut inventory,
            &mut progression,
            &vault,
            &xp_rules,
            "wood",
            5,
        );
        assert_eq!(rewards.added, 5);
        assert!(!rewards.full);
        assert_eq!(inventory.wood, 5);
        assert_eq!(progression.xp, HARVEST_XP as i64);
        assert_eq!(progression.level, 1);
    }

    #[test]
    fn xp_atravessa_o_nivel() {
        let content = content_com(&[("wood", "resource", None)]);
        let vault = VaultRules::from_items(&content.items);
        let xp_rules = XpRules::from_progression(&content.progression);
        let mut inventory = Inventory::default();
        let mut progression = XpState {
            xp: 90,
            xp_next: 100,
            ..Default::default()
        };

        let rewards = grant_break_rewards(
            &mut inventory,
            &mut progression,
            &vault,
            &xp_rules,
            "wood",
            1,
        );
        assert_eq!(rewards.xp.levels, 1);
        assert_eq!(progression.level, 2);
        assert_eq!(progression.xp, 20);
        assert_eq!(progression.xp_next, 150);
        assert_eq!(progression.skill_points, 1);
    }

    #[test]
    fn bolsa_cheia_nao_quebra_nem_paga_xp() {
        // Teto de pilha 1: com 1 fruta já não cabe nada.
        let content = content_com(&[("berry", "item", Some(1))]);
        let vault = VaultRules::from_items(&content.items);
        let xp_rules = XpRules::from_progression(&content.progression);
        let mut inventory = Inventory::default();
        inventory.set("berry", 1);
        let mut progression = XpState::default();

        let rewards = grant_break_rewards(
            &mut inventory,
            &mut progression,
            &vault,
            &xp_rules,
            "berry",
            3,
        );
        assert!(rewards.full);
        assert_eq!(rewards.added, 0);
        assert_eq!(inventory.berry, 1, "o parcial é devolvido");
        assert_eq!(progression.xp, 0);
    }

    #[test]
    fn golpe_final_quebra_marca_e_despawna() {
        let _store_lock = vael_engine::luau::game::test_support::PROCESS_STORE_TEST_LOCK
            .lock()
            .expect("lock de teste do store");
        vael_engine::luau::game::restore_game_json(&std::collections::BTreeMap::new());
        let mut app = app_rust();
        app.insert_resource(content_com(&[("wood", "resource", None)]));
        let _player = spawn_player(&mut app);
        let prop = spawn_prop(
            &mut app,
            harvestable("tree", Some("tree@world/forest.xml#1"), 1, 3.5, "wood", 5),
        );

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyJ);
        app.update();
        assert!(app.world().get::<Harvestable>(prop).is_some());
        assert_eq!(app.world().resource::<VaelSave>().inventory.wood, 0);
        finish_chop(&mut app);

        assert!(
            app.world().get::<Harvestable>(prop).is_none(),
            "o prop partido sai do mundo"
        );
        let save = app.world().resource::<VaelSave>();
        assert_eq!(save.inventory.wood, 5);
        assert_eq!(save.progression.xp, HARVEST_XP as i64);
        assert!(save.harvested.contains("tree@world/forest.xml#1"));
    }

    /// `cut-height` do `destructible` chega ao `Harvestable` (o `bevy_flora`
    /// corta a árvore a essa altura); ausente/inválido cai em 0,5 m.
    #[test]
    fn cut_height_vem_do_destructible() {
        let com = props(&[(
            "destructible",
            "hits: 2; break-style: fall; cut-height: 0.7",
        )]);
        let parsed = Harvestable::from_props(&com, "tree", None).expect("colhível");
        assert!((parsed.cut_height - 0.7).abs() < 1e-5);

        let sem = props(&[("destructible", "hits: 2; break-style: fall")]);
        let parsed = Harvestable::from_props(&sem, "tree", None).expect("colhível");
        assert!((parsed.cut_height - 0.5).abs() < 1e-5);

        let lixo = props(&[("destructible", "hits: 2; break-style: fall; cut-height: -3")]);
        let parsed = Harvestable::from_props(&lixo, "tree", None).expect("colhível");
        assert!((parsed.cut_height - 0.5).abs() < 1e-5);
    }

    /// `flora-fell.<chave>.*` ida e volta pelo save e pelo snapshot.
    #[test]
    fn flora_fell_ida_e_volta() {
        let mut save = VaelSave::default();
        store_flora_fell(
            Some(&mut save),
            "arvore@world/forest.xml#1",
            Vec3::new(0.6, 0.0, -0.8),
            0.7,
            true,
        );
        let fell =
            flora_fell_from_kv(&save.legacy, "arvore@world/forest.xml#1").expect("estado guardado");
        assert!((fell.dx - 0.6).abs() < 1e-5);
        assert!((fell.dz + 0.8).abs() < 1e-5);
        assert!((fell.cut_height - 0.7).abs() < 1e-5);
        assert!(fell.bucked);
        // Sem direcção guardada não há restauro.
        assert!(flora_fell_from_kv(&save.legacy, "outra").is_none());
    }

    /// O último golpe numa árvore procedural NÃO despawna: insere `FellTree`
    /// (o bevy_flora tomba e deixa o toco) e guarda a direcção no save.
    #[test]
    fn tombo_de_arvore_procedural_insere_fell_tree() {
        let _store_lock = vael_engine::luau::game::test_support::PROCESS_STORE_TEST_LOCK
            .lock()
            .expect("lock de teste do store");
        vael_engine::luau::game::restore_game_json(&std::collections::BTreeMap::new());
        let mut app = app_rust();
        app.insert_resource(content_com(&[("wood", "resource", None)]));
        app.insert_resource(bevy_elements::WorldClock::new(
            bevy_elements::CivilDate::new(2028, 11, 30),
        ));
        let _player = spawn_player(&mut app);
        let mut h = harvestable("tree", Some("tree@world/forest.xml#1"), 1, 3.5, "wood", 5);
        h.style = BreakStyle::Fall;
        h.cut_height = 0.7;
        let prop = spawn_prop(&mut app, h);
        app.world_mut().entity_mut(prop).insert(PooledTree {
            pool: Handle::default(),
            variant: 0,
        });

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyJ);
        app.update();
        assert!(app.world().get::<Harvestable>(prop).is_some());
        assert_eq!(app.world().resource::<VaelSave>().inventory.wood, 0);
        finish_chop(&mut app);

        assert!(
            app.world().get_entity(prop).is_ok(),
            "a estação (toco + props) fica no mundo"
        );
        assert!(
            app.world().get::<PooledTree>(prop).is_none(),
            "a árvore sai da estação (o corte é em espaço global)"
        );
        let tree = {
            let mut q = app.world_mut().query_filtered::<Entity, With<FellTree>>();
            q.single(app.world())
                .expect("a árvore tombada é uma raiz com FellTree")
        };
        assert!(
            app.world().get::<FellTree>(tree).is_some(),
            "o bevy_flora recebe o FellTree na árvore-raiz"
        );
        assert!(
            app.world().get::<Falling>(tree).is_none(),
            "o tombo antigo da engine não entra numa árvore procedural"
        );
        let save = app.world().resource::<VaelSave>();
        assert!(
            save.legacy
                .contains_key("flora-fell.tree@world/forest.xml#1.dx"),
            "a direcção da queda fica no save"
        );
        assert_eq!(
            app.world().get::<Visibility>(prop),
            Some(&Visibility::Hidden)
        );
        app.world_mut()
            .resource_mut::<bevy_elements::WorldClock>()
            .advance_hours(36.);
        app.update();
        assert!(app.world().get::<Harvestable>(prop).is_none());
        assert!(app.world().get::<Children>(prop).is_some_and(|kids| {
            kids.iter()
                .any(|kid| app.world().get::<PooledTree>(kid).is_some())
        }));
        app.world_mut()
            .resource_mut::<bevy_elements::WorldClock>()
            .advance_hours(36.);
        app.update();
        assert_eq!(app.world().get::<Harvestable>(prop).unwrap().remaining, 1);
        assert!(app.world().get::<PooledTree>(prop).is_some());
        assert!(app.world().get_entity(tree).is_err());
        assert!(app.world().resource::<VaelSave>().harvested.is_empty());
        assert!(
            app.world()
                .resource::<VaelSave>()
                .legacy
                .keys()
                .all(|k| !k.starts_with("flora-fell."))
        );
    }

    /// O restauro corre no `PostStartup`/`SceneLoad`, quando o `GlobalTransform`
    /// das entidades acabadas de spawnar ainda está no DEFAULT (o
    /// `TransformPlugin` propaga no `PostUpdate`): a pose global da estação
    /// tem de vir do `TransformHelper`, senão a árvore reposta nascia na
    /// origem do mundo, à escala 1.
    #[test]
    fn reap_restaura_a_arvore_com_a_pose_global_da_estacao() {
        let _store_lock = vael_engine::luau::game::test_support::PROCESS_STORE_TEST_LOCK
            .lock()
            .expect("lock de teste do store");
        vael_engine::luau::game::restore_game_json(&std::collections::BTreeMap::new());
        let mut app = app_rust();
        let _player = spawn_player(&mut app);
        let group = app
            .world_mut()
            .spawn(Transform::from_xyz(0.0, 38.7304, 0.0))
            .id();
        let mut h = harvestable("tree", Some("tree@world/forest.xml#1"), 3, 3.5, "wood", 3);
        h.style = BreakStyle::Fall;
        let prop = app
            .world_mut()
            .spawn((
                h,
                Transform::from_xyz(-102.0, 7.0, 176.0),
                ChildOf(group),
                PooledTree {
                    pool: Handle::default(),
                    variant: 0,
                },
            ))
            .id();
        {
            let mut save = app.world_mut().resource_mut::<VaelSave>();
            store_flora_fell(
                Some(&mut save),
                "tree@world/forest.xml#1",
                Vec3::new(0.6, 0.0, -0.8),
                0.7,
                false,
            );
            save.harvested.insert("tree@world/forest.xml#1".into());
        }

        app.update();

        let tree = {
            let mut q = app.world_mut().query_filtered::<Entity, With<FellTree>>();
            q.single(app.world())
                .expect("a árvore reposta é uma raiz com FellTree")
        };
        let pose = app
            .world()
            .get::<Transform>(tree)
            .expect("a árvore reposta tem pose");
        assert!(
            (pose.translation.y - 45.7304).abs() < 1e-3,
            "a cota soma o grupo da vegetação: {}",
            pose.translation.y
        );
        assert!((pose.translation.x + 102.0).abs() < 1e-3);
        assert!((pose.translation.z - 176.0).abs() < 1e-3);
        // A estação continua no sítio (o toco não se mudou para a origem).
        let station = app
            .world()
            .get::<Transform>(prop)
            .expect("a estação fica com o Transform");
        assert_eq!(station.translation, Vec3::new(-102.0, 7.0, 176.0));
    }

    /// `reap_saved` não despawna: repõe a árvore já deitada.
    #[test]
    fn reap_restaura_arvore_deitada_do_save() {
        let _store_lock = vael_engine::luau::game::test_support::PROCESS_STORE_TEST_LOCK
            .lock()
            .expect("lock de teste do store");
        vael_engine::luau::game::restore_game_json(&std::collections::BTreeMap::new());
        let mut app = app_rust();
        app.insert_resource(content_com(&[("wood", "resource", None)]));
        let _player = spawn_player(&mut app);
        let point = Vec2::new(-102.0, 176.0);
        app.insert_resource(bevy_flora::FloraGround::new(move |_| 45.0));
        let mut h = harvestable("tree", Some("tree@world/forest.xml#1"), 3, 3.5, "wood", 3);
        h.style = BreakStyle::Fall;
        let prop = spawn_prop(&mut app, h);
        app.world_mut().entity_mut(prop).insert(PooledTree {
            pool: Handle::default(),
            variant: 0,
        });
        {
            let mut save = app.world_mut().resource_mut::<VaelSave>();
            store_flora_fell(
                Some(&mut save),
                "tree@world/forest.xml#1",
                Vec3::new(0.6, 0.0, -0.8),
                0.7,
                false,
            );
            save.harvested.insert("tree@world/forest.xml#1".into());
        }

        app.update();

        assert!(
            app.world().get_entity(prop).is_ok(),
            "a estação tombada não é despawnada pelo reap"
        );
        let tree = {
            let mut q = app.world_mut().query_filtered::<Entity, With<FellTree>>();
            q.single(app.world())
                .expect("a árvore reposta é uma raiz com FellTree")
        };
        let fell = app
            .world()
            .get::<FellTree>(tree)
            .expect("o save repõe o FellTree lying");
        assert!(fell.lying, "restaurada já deitada, sem tombar outra vez");
        assert!((fell.direction.x - 0.6).abs() < 1e-5);
        assert!((fell.cut_height - 0.7).abs() < 1e-5);
        assert!(app.world().get::<Harvestable>(prop).is_none());
        let _ = point;
    }

    /// Uma troca de cena leva as árvores deitadas, os **tocos** e os toros: os
    /// três são RAIZES criadas pelo `bevy_flora` fora da hierarquia da cena.
    /// A árvore **restaurada do save** na cena nova (spawn do `SceneLoad`, já
    /// com a geração nova) NÃO pode ser varrida no frame seguinte — era o que
    /// a limpeza por `CurrentScene.id` fazia (a troca corre no `Last`).
    #[test]
    fn troca_de_cena_limpa_as_raizes_do_bevy_flora_da_geracao_antiga() {
        let capsule = || bevy_flora::WoodCapsule {
            a: Vec3::ZERO,
            b: Vec3::Y,
            radius: 0.1,
        };
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(SceneEpoch(7))
            .add_systems(Update, (tag_fell_entities, despawn_fell_trees).chain());

        let old_tree = app
            .world_mut()
            .spawn((FellTree::default(), FellEpoch(6)))
            .id();
        let old_stump = app
            .world_mut()
            .spawn((
                TreeStump {
                    tree: Some(old_tree),
                    capsule: capsule(),
                },
                FellEpoch(6),
            ))
            .id();
        let old_log = app
            .world_mut()
            .spawn((
                TreeLog {
                    stump: old_stump,
                    index: 0,
                    capsule: capsule(),
                },
                FellEpoch(6),
            ))
            .id();
        // A árvore reposta do save na cena NOVA (geração 7) fica.
        let new_tree = app
            .world_mut()
            .spawn((
                FellTree {
                    lying: true,
                    ..default()
                },
                FellEpoch(7),
            ))
            .id();
        // O toco que o crate acabou de criar herda a geração da árvore.
        let new_stump = app
            .world_mut()
            .spawn(TreeStump {
                tree: Some(new_tree),
                capsule: capsule(),
            })
            .id();

        app.update();
        assert!(
            app.world().get_entity(old_tree).is_err(),
            "a árvore deitada da cena anterior sai"
        );
        assert!(
            app.world().get_entity(old_stump).is_err(),
            "o toco do crate também é uma raiz e sai"
        );
        assert!(
            app.world().get_entity(old_log).is_err(),
            "os toros da cena anterior saem"
        );
        assert!(
            app.world().get_entity(new_tree).is_ok(),
            "a árvore restaurada na cena nova não é varrida"
        );
        assert_eq!(
            app.world().get::<FellEpoch>(new_stump).map(|epoch| epoch.0),
            Some(7),
            "o toco novo herda a geração da árvore"
        );
    }

    /// `[J]` numa árvore deitada retalha-a em toros e marca o save.
    #[test]
    fn j_na_arvore_deitada_insere_buck_tree() {
        let _store_lock = vael_engine::luau::game::test_support::PROCESS_STORE_TEST_LOCK
            .lock()
            .expect("lock de teste do store");
        vael_engine::luau::game::restore_game_json(&std::collections::BTreeMap::new());
        let mut app = app_rust();
        let _player = spawn_player(&mut app);
        // Uma árvore já deitada não é um `Harvestable` (foi cortada): sem isto
        // o `harvest_input` apanhava o J e despawnava-a.
        let lying = app
            .world_mut()
            .spawn((
                Transform::default(),
                GlobalTransform::default(),
                FellTree {
                    direction: Vec2::X,
                    cut_height: 0.5,
                    speed: 1.0,
                    lying: true,
                },
                FloraFellKey("tree@world/forest.xml#1".into()),
            ))
            .id();

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyJ);
        app.update();

        assert!(
            app.world().get::<BuckTree>(lying).is_some(),
            "o [J] na árvore deitada pede os toros"
        );
        let save = app.world().resource::<VaelSave>();
        assert!(
            matches!(
                save.legacy.get("flora-fell.tree@world/forest.xml#1.bucked"),
                Some(Json::Bool(true))
            ),
            "o estado retalhado fica no save"
        );
    }

    #[test]
    fn picareta_so_marca_o_contacto_se_o_alvo_continua_ao_alcance() {
        let mut app = app_rust();
        let hero = spawn_player(&mut app);
        let prop = spawn_prop(&mut app, harvestable("outcrop", None, 3, 3.5, "stone", 4));
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyJ);
        app.update();
        let gesture = app
            .world()
            .get::<vael_engine::animation::PendingGesture>(hero)
            .unwrap();
        assert_eq!(gesture.name, "mine");
        finish_chop(&mut app); // 300 ms, ainda antes do contacto da picareta.
        assert_eq!(app.world().get::<Harvestable>(prop).unwrap().remaining, 3);
        assert_eq!(app.world().resource::<HarvestFocus>().tool, "mine");
        app.world_mut()
            .entity_mut(hero)
            .insert(GlobalTransform::from_translation(Vec3::X * 10.));
        finish_chop(&mut app);
        assert_eq!(app.world().get::<Harvestable>(prop).unwrap().remaining, 3);
    }

    #[test]
    fn um_golpe_intermedio_nao_credita_loot() {
        let mut app = app_rust();
        app.insert_resource(content_com(&[("wood", "resource", None)]));
        let _player = spawn_player(&mut app);
        let prop = spawn_prop(
            &mut app,
            harvestable("tree", Some("tree@world/forest.xml#1"), 3, 3.5, "wood", 5),
        );

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyJ);
        app.update();
        assert!(app.world().get::<Harvestable>(prop).is_some());
        assert_eq!(app.world().resource::<VaelSave>().inventory.wood, 0);
        finish_chop(&mut app);

        assert_eq!(
            app.world().get::<Harvestable>(prop).map(|h| h.remaining),
            Some(2)
        );
        assert_eq!(app.world().resource::<VaelSave>().inventory.wood, 0);
        assert!(app.world().resource::<VaelSave>().harvested.is_empty());
    }

    #[test]
    fn reap_apaga_o_prop_do_save_e_preserva_os_outros() {
        let _store_lock = vael_engine::luau::game::test_support::PROCESS_STORE_TEST_LOCK
            .lock()
            .expect("lock de teste do store");
        vael_engine::luau::game::restore_game_json(&std::collections::BTreeMap::new());
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let harvested_key = "tree@world/forest.xml#1";
        app.insert_resource(VaelSave {
            harvested: [harvested_key.to_string()].into_iter().collect(),
            ..Default::default()
        });
        let reaped = spawn_prop(
            &mut app,
            harvestable("tree", Some(harvested_key), 3, 3.5, "wood", 1),
        );
        let kept = spawn_prop(
            &mut app,
            harvestable("tree", Some("tree@world/forest.xml#2"), 3, 3.5, "wood", 1),
        );
        app.world_mut().run_system_once(reap_on_load).expect("reap");
        assert!(app.world().get::<Harvestable>(reaped).is_none());
        assert!(app.world().get::<Harvestable>(kept).is_some());
    }

    // ── gate ────────────────────────────────────────────────────────────

    #[test]
    fn gate_so_liga_com_os_valores_aceites() {
        assert!(VaelHarvestConfig::from_raw(Some("1")).rust);
        assert!(VaelHarvestConfig::from_raw(Some("true")).rust);
        assert!(VaelHarvestConfig::from_raw(Some(" YES ")).rust);
        assert!(!VaelHarvestConfig::from_raw(Some("0")).rust);
        assert!(!VaelHarvestConfig::from_raw(None).rust);
    }

    #[test]
    fn patch_rewards_escreve_o_xp_quando_nao_ha_recurso() {
        let _store_lock = vael_engine::luau::game::test_support::PROCESS_STORE_TEST_LOCK
            .lock()
            .expect("lock de teste do store");
        // Isolar o store do processo: testes em paralelo partilham-no
        // (o lock serializa; o restore limpa o que ficou de outros).
        vael_engine::luau::game::restore_game_json(&std::collections::BTreeMap::new());
        let inventory = Inventory::default();
        let xp = XpState {
            xp: HARVEST_XP as i64,
            xp_next: 100,
            level: 1,
            skill_points: 0,
            ..Default::default()
        };
        patch_rewards("", &inventory, &xp);
        let kv = vael_engine::luau::game::game_to_json_process();
        assert_eq!(
            kv.get(KEY_XP).and_then(Json::as_f64),
            Some(HARVEST_XP),
            "integral sai como inteiro no JSON"
        );
    }

    #[test]
    fn store_stump_grava_a_pose_no_legacy() {
        let mut save = VaelSave::default();
        store_stump(
            Some(&mut save),
            "tree@forest.xml#1",
            "berry-bush-bare",
            Vec3::new(1.5, 2.0, -3.0),
            1.25,
        );
        let stump = stump_for(&save.legacy, "tree@forest.xml#1").expect("stump");
        assert_eq!(stump.proto, "berry-bush-bare");
        assert_eq!((stump.x, stump.y, stump.z), (1.5, Some(2.0), -3.0));
        assert!(
            (stump.scale - 1.25).abs() < 1e-6,
            "a escala do substituto ida e volta: {}",
            stump.scale
        );
    }

    /// O substituto spawnado pelo `apply_script_spawns` recebe a escala do
    /// `replace-scale`: o pedido da engine não a transporta, por isso é escrita
    /// na raiz nova (a que não tem pai) pela posição XZ do pedido.
    #[test]
    fn replace_scale_aplica_se_a_raiz_spawnada() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<PendingReplacementScales>()
            .add_systems(Update, apply_replacement_scales);
        app.world_mut()
            .resource_mut::<PendingReplacementScales>()
            .0
            .push(ReplacementScale {
                x: 4.0,
                z: -2.0,
                scale: 1.5,
                age: 0.0,
            });
        // Um FILHO na origem local (0,0) não pode ser o alvo.
        let parent = app
            .world_mut()
            .spawn(Transform::from_xyz(100.0, 0.0, 100.0))
            .id();
        let child = app
            .world_mut()
            .spawn((Transform::default(), ChildOf(parent)))
            .id();
        let bush = app
            .world_mut()
            .spawn(Transform::from_xyz(4.0, 40.0, -2.0))
            .id();

        app.update();

        assert_eq!(
            app.world().get::<Transform>(bush).map(|t| t.scale),
            Some(Vec3::splat(1.5))
        );
        assert_eq!(
            app.world().get::<Transform>(child).map(|t| t.scale),
            Some(Vec3::ONE),
            "um filho local não é a raiz do protótipo"
        );
        assert!(
            app.world()
                .resource::<PendingReplacementScales>()
                .0
                .is_empty(),
            "o pedido é consumido pelo spawn que casa"
        );
    }

    #[test]
    fn patch_do_gate_corrige_um_save_antigo() {
        assert_eq!(gate_patch(None, true), GatePatch::On);
        assert_eq!(gate_patch(Some(false), true), GatePatch::On);
        assert_eq!(gate_patch(Some(true), true), GatePatch::Unchanged);
        assert_eq!(gate_patch(Some(true), false), GatePatch::Clear);
        assert_eq!(gate_patch(None, false), GatePatch::Unchanged);
    }

    #[test]
    fn gate_escreve_e_apaga_a_chave_no_viber_game() {
        let _store_lock = vael_engine::luau::game::test_support::PROCESS_STORE_TEST_LOCK
            .lock()
            .expect("lock de teste do store");
        // Isolar o store do processo: testes em paralelo partilham-no
        // (o lock serializa; o restore limpa o que ficou de outros).
        vael_engine::luau::game::restore_game_json(&std::collections::BTreeMap::new());

        apply_gate(true);
        let kv = vael_engine::luau::game::game_to_json_process();
        assert_eq!(kv.get(RUST_GATE_KEY), Some(&Json::Bool(true)));

        apply_gate(false);
        let kv = vael_engine::luau::game::game_to_json_process();
        assert!(
            !kv.contains_key(RUST_GATE_KEY),
            "gate desligado sai do store"
        );
    }

    #[test]
    fn numero_de_golpes_escala_proporcionalmente_ao_tamanho_de_pedras_e_arvores() {
        // Pedras: pequena < média < grande (mais segmentos e maior volume/escala).
        let small_rock_collider = avian3d::prelude::Collider::cuboid(0.8, 0.45, 0.7);
        let medium_rock_collider = avian3d::prelude::Collider::cuboid(1.4, 0.7, 1.25);
        let large_rock_collider = avian3d::prelude::Collider::cuboid(2.2, 1.3, 2.0);
        let small_rock_hits =
            rock_hits_for_size(3, Vec3::splat(0.75), Some(1), Some(&small_rock_collider));
        let medium_rock_hits =
            rock_hits_for_size(3, Vec3::ONE, Some(1), Some(&medium_rock_collider));
        let multi_seg_rock_hits =
            rock_hits_for_size(3, Vec3::splat(1.1), Some(3), Some(&medium_rock_collider));
        let large_rock_hits =
            rock_hits_for_size(3, Vec3::splat(1.25), Some(3), Some(&large_rock_collider));
        assert!(
            small_rock_hits < medium_rock_hits
                && medium_rock_hits < multi_seg_rock_hits
                && multi_seg_rock_hits < large_rock_hits,
            "golpes de pedra devem crescer com o tamanho: small={small_rock_hits}, med={medium_rock_hits}, multi={multi_seg_rock_hits}, large={large_rock_hits}"
        );

        // Árvores em pé: cerejeira pequena < bétula média < pinheiro grande < carvalho gigante.
        let small_tree_hits = tree_hits_for_size(3, Vec3::splat(0.75), Some(0.22), Some(2.0), None);
        let medium_tree_hits = tree_hits_for_size(3, Vec3::ONE, Some(0.35), Some(7.5), None);
        let large_tree_hits = tree_hits_for_size(3, Vec3::splat(1.1), Some(0.60), Some(8.0), None);
        let giant_oak_hits = tree_hits_for_size(3, Vec3::splat(1.35), Some(0.80), Some(8.0), None);
        assert!(
            small_tree_hits < medium_tree_hits
                && medium_tree_hits < large_tree_hits
                && large_tree_hits < giant_oak_hits,
            "golpes de árvore devem crescer com o tamanho: small={small_tree_hits}, med={medium_tree_hits}, large={large_tree_hits}, giant={giant_oak_hits}"
        );

        // Troncos caídos: tronco fino/curto < médio < grosso/comprido.
        let small_log_hits = log_hits_for_size(2.2, 0.16);
        let medium_log_hits = log_hits_for_size(5.5, 0.38);
        let large_log_hits = log_hits_for_size(7.0, 0.72);
        assert!(
            small_log_hits < medium_log_hits && medium_log_hits < large_log_hits,
            "golpes de tronco caído devem crescer com o tamanho: small={small_log_hits}, med={medium_log_hits}, large={large_log_hits}"
        );
    }

    #[test]
    fn tree_chop_yaw_escala_com_grossura_do_tronco() {
        assert!((tree_chop_yaw_deg(None) - 5.0).abs() < 1e-5);
        assert!((tree_chop_yaw_deg(Some(0.0)) - 5.0).abs() < 1e-5);
        assert!((tree_chop_yaw_deg(Some(f32::NAN)) - 5.0).abs() < 1e-5);

        let thin = tree_chop_yaw_deg(Some(0.25));
        let medium = tree_chop_yaw_deg(Some(0.41));
        let wide = tree_chop_yaw_deg(Some(0.68));
        let thick = tree_chop_yaw_deg(Some(0.98));
        let giant = tree_chop_yaw_deg(Some(1.35));

        assert!((thin - 4.0).abs() < 1e-5, "thin={thin}");
        assert!((medium - 5.0).abs() < 0.1, "medium={medium}");
        assert!(thin < medium && medium < wide && wide < thick);
        assert!((thick - 10.0).abs() < 1e-5, "thick={thick}");
        assert!((giant - 10.0).abs() < 1e-5, "giant={giant}");
    }

    #[test]
    fn rotate_clockwise_xz_gira_no_sentido_horario() {
        // +Z (olhar padrão) girado 90° no sentido horário (vista de cima, +Y) aponta para -X (direita).
        let from_pos_z = rotate_clockwise_xz(Vec2::Y, 90.0);
        assert!(
            from_pos_z.distance(Vec2::NEG_X) < 1e-5,
            "got {from_pos_z:?}"
        );
        // -Z (norte) girado 90° no sentido horário aponta para +X (leste).
        let from_neg_z = rotate_clockwise_xz(Vec2::NEG_Y, 90.0);
        assert!(from_neg_z.distance(Vec2::X) < 1e-5, "got {from_neg_z:?}");

        // Qualquer direção girada 10° no sentido horário reduz o yaw (atan2(x, z)) em 10°.
        let dir = Vec2::new(0.6, 0.8);
        let rotated = rotate_clockwise_xz(dir, 10.0);
        let yaw0 = dir.x.atan2(dir.y);
        let yaw1 = rotated.x.atan2(rotated.y);
        assert!(
            ((yaw0 - yaw1).to_degrees() - 10.0).abs() < 1e-4,
            "delta={}",
            (yaw0 - yaw1).to_degrees()
        );
        assert!((rotated.length() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn tool_attack_motion_afasta_e_gira_so_para_arvores_em_pe() {
        let player = Vec3::ZERO;
        // Árvore média (r=0.41 m) à frente em +Z; contacto na casca a 0.88 m (10 cm perto demais).
        let tree_center = Vec3::new(0.0, 0.0, 0.88 + 0.41);
        let contact = Vec3::new(0.0, 0.8, 0.88);
        let m_close = tool_attack_motion(player, tree_center, contact, true, None, 0.34);
        assert_eq!(m_close.lunge, 0.0);
        assert!(
            (m_close.step.y + 0.10).abs() < 1e-4 && m_close.step.x.abs() < 1e-5,
            "deve recuar 10 cm na radial (-Z), got {:?}",
            m_close.step
        );
        let expected_med_face = rotate_clockwise_xz(Vec2::Y, tree_chop_yaw_deg(Some(0.41)));
        assert!(
            m_close.face.unwrap().distance(expected_med_face) < 1e-5,
            "face={:?} expected={expected_med_face:?}",
            m_close.face
        );

        // Já a 0.98 m da casca: não desliza nem oscila.
        let m_planted = tool_attack_motion(
            player,
            Vec3::new(0.0, 0.0, TREE_CHOP_STANDOFF + 0.41),
            Vec3::new(0.0, 0.8, TREE_CHOP_STANDOFF),
            true,
            None,
            0.34,
        );
        assert_eq!(m_planted.step, Vec2::ZERO);

        // Árvore grossa (carvalho r=1.05 m via flora) mais longe (1.38 m): avança 0.40 m até 0.98 m e gira 10°.
        let m_thick = tool_attack_motion(
            player,
            Vec3::new(0.0, 0.0, 1.38 + 0.55),
            Vec3::new(0.0, 0.8, 1.38),
            true,
            Some(1.05),
            0.34,
        );
        assert!(
            (m_thick.step.y - 0.40).abs() < 1e-4 && m_thick.step.x.abs() < 1e-5,
            "deve avançar 40 cm na radial (+Z), got {:?}",
            m_thick.step
        );
        let expected_thick_face = rotate_clockwise_xz(Vec2::Y, 10.0);
        assert!(
            m_thick.face.unwrap().distance(expected_thick_face) < 1e-5,
            "face={:?} expected={expected_thick_face:?}",
            m_thick.face
        );

        // Mineração / tronco caído mantém o standoff 0.88 m e o rosto reto sem giro.
        let m_rock = tool_attack_motion(
            player,
            Vec3::new(0.0, 0.0, 1.48),
            Vec3::new(0.0, 0.8, 1.18),
            false,
            None,
            0.48,
        );
        assert_eq!(m_rock.step, Vec2::ZERO);
        assert!((m_rock.lunge - 0.30).abs() < 1e-5);
        assert_eq!(m_rock.face, Some(Vec2::Y));
    }

    #[test]
    fn derrubar_arvore_publica_recuo_e_giro_horario_no_bus() {
        let mut app = app_rust();
        app.init_resource::<crate::player::HeroMotionBus>();
        spawn_player(&mut app);

        let mut h = harvestable("oak", None, 4, 3.5, "wood", 3);
        h.style = BreakStyle::Fall;
        let tree = spawn_prop(&mut app, h);
        // Árvore grossa (raio 1.0 m) com a casca a 0.88 m em +Z (centro em z = 1.88).
        app.world_mut().entity_mut(tree).insert((
            avian3d::prelude::Collider::cylinder(1.0, 3.0),
            GlobalTransform::from_translation(Vec3::new(0.0, 0.0, 1.88)),
        ));

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyJ);
        app.update();

        let attack = app
            .world()
            .resource::<crate::player::HeroMotionBus>()
            .lock()
            .attack
            .expect("golpe de machado publica movimento no bus");
        assert_eq!(attack.lunge, 0.0);
        assert!(
            (attack.step.y + 0.10).abs() < 1e-3 && attack.step.x.abs() < 1e-4,
            "step={:?}",
            attack.step
        );
        let expected_face = rotate_clockwise_xz(Vec2::Y, 10.0);
        assert!(
            attack.face.unwrap().distance(expected_face) < 1e-3,
            "face={:?} expected={expected_face:?}",
            attack.face
        );
    }
}

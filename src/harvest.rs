//! Colheita/interação em Rust (`VAEL_HARVEST_RUST=1`).
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
//! O gate é opt-in: sem `VAEL_HARVEST_RUST=1` (ou `true`/`on`/`yes`) nada
//! aqui corre e o `scripts/game/harvest.lua` continua a ser a implementação.
//! Com o gate ligado o plugin escreve `vael_harvest_rust = true` no
//! `viber.game()` (e o Lua faz `return` no `on_update`).
//!
//! Limites desta primeira versão:
//! - o golpe é imediato (sem o atraso de impacto/cooldown de animação do Lua);
//!   o cooldown de 0,4 s entre golpes mantém-se;
//! - o topple usa [`viber::physics_fx::Falling`] para `break-style: fall`;
//!   os bursts/FX de partículas ficam para uma passagem posterior;
//! - o substituto (`berry-bush-bare`) é spawnado sem a escala original (o
//!   pedido de spawn não transporta escala);
//! - o progresso de quests `collect` é pedido ao [`QuestEventQueue`]; sem
//!   ele, o `sync_collect` do `rules.tick` (Lua) apanha o cofre no frame
//!   seguinte.

use bevy::ecs::message::Messages;
use bevy::prelude::*;
use bevy_ext_world::xml::values::parse_color;
use bevy_flora::{
    BuckTree, FellTree, Felled, PooledLook, PooledTree, TreeLanded, TreeShake, TreeShakes,
};
use mlua::{Lua, Table, Value as LuaValue};
use serde_json::{Value as Json, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};
use viber::ambient::{SfxClip, SfxEvent};
use viber::feedback::DamageNumberEvent;
use viber::interact::{InteractionFocus, range_scale};
use viber::luau::components::{ScriptInteraction, ScriptProps};
use viber::luau::game::{apply_patch, game_to_json};
use viber::luau::host::LuaScriptHost;
use viber::menus::MenusOpen;
use viber::physics_fx::Falling;
use viber::player::Player;
use viber::recipes::spawn::{PendingPrototypeSpawn, PendingScriptSpawns};
use viber::scene::SceneSource;
use viber::scene_loader::CurrentScene;
use viber::ui::ScriptBindValue;
use viber::ui::script::UiScriptBinds;
use viber::ui::widgets::UiFocusedInput;
use viber::vitals::Health;

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
/// Chave do `viber.game()` que avisa o Lua de que a colheita é do Rust.
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
    /// Alcance AUTORADO; o efetivo é `viber::interact::scaled_range`.
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
struct BuckFocus {
    entity: Option<Entity>,
    cooldown: f32,
}

/// Pedidos de tombo vindos do Lua (`viber.vael.fell_tree`): a função só
/// enfileira (o closure não tem o ECS); o sistema processa no `Update`.
#[derive(Resource, Clone, Default)]
struct PendingFloraFell(std::sync::Arc<std::sync::Mutex<Vec<u64>>>);

/// Instala `viber.vael.fell_tree(id) -> true` — o `harvest.lua` usa-a no ramo
/// `break-style: fall` no lugar de `viber.topple`. Sem isto o caminho Lua
/// (default) tombava a ESTAÇÃO com o `Falling` da engine e perdia o toco e o
/// tronco deitado do `bevy_flora`.
fn install_fell_api(host: Option<Res<LuaScriptHost>>, mut commands: Commands) {
    let Some(host) = host else {
        return;
    };
    let queue = PendingFloraFell::default();
    let shared = std::sync::Arc::clone(&queue.0);
    let lua = &host.lua;
    let installed = (|| -> mlua::Result<()> {
        let vael = vael_table(lua)?;
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
    })();
    match installed {
        Ok(()) => {
            commands.insert_resource(queue);
            info!("vael: `viber.vael.fell_tree` ligado (tombo do bevy_flora no Lua)");
        }
        Err(err) => error!("vael: falha a instalar `viber.vael.fell_tree`: {err}"),
    }
}

/// Processa os pedidos do Lua: árvore procedural → `FellTree` numa raiz com a
/// pose global; prop antigo → o `Falling` da engine (o que o `viber.topple`
/// fazia).
fn process_pending_fell(
    mut commands: Commands,
    pending: Option<Res<PendingFloraFell>>,
    host: Option<Res<LuaScriptHost>>,
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
    let requests = std::mem::take(
        &mut *pending
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    );
    if requests.is_empty() {
        return;
    }
    let lua = host.as_deref().map(|host| &host.lua);
    let player = players
        .iter()
        .next()
        .map(GlobalTransform::translation)
        .unwrap_or(Vec3::ZERO);
    for bits in requests {
        let Some(entity) = Entity::try_from_bits(bits) else {
            continue;
        };
        let Ok((global, pooled, look, harvestable, local)) = props.get(entity) else {
            continue;
        };
        let position = global.translation();
        let direction = (position - player).normalize_or_zero();
        let cut_height = harvestable.map(|h| h.cut_height).unwrap_or(0.5);
        let key = harvestable.and_then(|h| h.stable_key.clone());
        if let Some(pooled) = pooled {
            let tree = spawn_felled_tree(
                &mut commands,
                pooled,
                look,
                global,
                direction,
                cut_height,
                false,
            );
            if let Some(key) = key.as_deref() {
                commands.entity(tree).insert(FloraFellKey(key.to_string()));
                store_flora_fell(lua, save.as_deref_mut(), key, direction, cut_height, false);
            }
            commands
                .entity(entity)
                .try_remove::<(
                    Harvestable,
                    ScriptInteraction,
                    viber::luau::components::LuaScriptRef,
                )>()
                .remove::<PooledTree>()
                .insert(FloraFellTree);
            viber::physics::replace_with_stump_collider(
                &mut commands,
                entity,
                STUMP_RADIUS,
                STUMP_HALF_HEIGHT,
            );
        } else {
            let initial = local.map(|local| local.rotation).unwrap_or_default();
            commands
                .entity(entity)
                .try_remove::<(
                    Harvestable,
                    ScriptInteraction,
                    viber::luau::components::LuaScriptRef,
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

/// Decide o patch do gate a partir do valor atual do `viber.game()`.
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
            .register_type::<Harvestable>()
            .register_type::<HarvestFocus>()
            .register_type::<BreakStyle>()
            // O gate vive fora do `run_if`: quando OFF também tem de apagar
            // um `true` que venha de um save antigo.
            .add_systems(Startup, (publish_harvest_gate, install_fell_api).chain())
            .add_systems(Update, sync_harvest_gate)
            // `Harvestable` e o reap correm SEMPRE: o caminho Lua (default)
            // também precisa deles para o restauro das árvores tombadas
            // (`flora-fell.*`). O foco/golpe Rust continuam atrás do gate.
            .add_systems(PostStartup, (attach_harvestables, reap_on_load).chain())
            .add_systems(
                viber::scene_loader::SceneLoad,
                (attach_harvestables, reap_on_load)
                    .chain()
                    .after(viber::recipes::spawn::startup),
            )
            .add_systems(
                Update,
                (update_harvest_focus, harvest_input)
                    .chain()
                    .run_if(harvest_rust_enabled),
            )
            // O processamento do tombo do Lua, o reap periódico, o embate e
            // os toros valem para os DOIS caminhos de colheita.
            .add_systems(
                Update,
                (
                    process_pending_fell,
                    reap_periodic,
                    tree_landed_feedback,
                    buck_felled_trees,
                    despawn_fell_trees,
                ),
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

/// Marca a chave em `VaelSave::harvested` e no espelho Lua (`harvested.<chave>`).
fn mark_harvested(lua: Option<&Lua>, save: Option<&mut VaelSave>, key: &str) {
    if key.is_empty() {
        return;
    }
    if let Some(save) = save {
        save.harvested.insert(key.to_string());
    }
    if let Some(lua) = lua {
        let mut patch = BTreeMap::new();
        patch.insert(format!("harvested.{key}"), Json::Bool(true));
        apply_patch(lua, &patch);
    }
}

/// Guarda a pose/escala do substituto (`stump.<chave>.*`), como o Lua.
fn store_stump(
    lua: Option<&Lua>,
    save: Option<&mut VaelSave>,
    key: &str,
    proto: &str,
    position: Vec3,
    scale: f32,
) {
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
    if let Some(lua) = lua {
        apply_patch(lua, &fields.into_iter().collect());
    }
}

/// Guarda o estado do tombo de uma árvore procedural (`flora-fell.<chave>.*`):
/// direcção da queda, altura do corte e se já foi retalhada em toros. É o que
/// o load usa para a deitar no sítio certo, sem a tombar outra vez.
fn store_flora_fell(
    lua: Option<&Lua>,
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
    if let Some(lua) = lua {
        apply_patch(lua, &fields.into_iter().collect());
    }
}

/// Marca a árvore deitada como retalhada em toros (`flora-fell.<chave>.bucked`).
fn store_flora_bucked(lua: Option<&Lua>, save: Option<&mut VaelSave>, key: &str) {
    if key.is_empty() {
        return;
    }
    let name = format!("flora-fell.{key}.bucked");
    if let Some(save) = save {
        save.legacy.insert(name.clone(), Json::Bool(true));
    }
    if let Some(lua) = lua {
        let mut patch = BTreeMap::new();
        patch.insert(name, Json::Bool(true));
        apply_patch(lua, &patch);
    }
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
}

/// Lê `stump.<chave>.*` de um snapshot do `viber.game()`/`legacy`.
pub fn stump_for(kv: &BTreeMap<String, Json>, key: &str) -> Option<Stump> {
    let field = |name: &str| format!("stump.{key}.{name}");
    let proto = kv.get(&field("proto"))?.as_str()?;
    if !REPLACEMENTS.contains(&proto) {
        return None;
    }
    Some(Stump {
        proto: proto.to_string(),
        x: json_f32(kv.get(&field("x")))?,
        y: json_f32(kv.get(&field("y"))),
        z: json_f32(kv.get(&field("z")))?,
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

/// Arranque: fixa o gate no `viber.game()`.
fn publish_harvest_gate(config: Res<VaelHarvestConfig>, host: Option<Res<LuaScriptHost>>) {
    let Some(host) = host else {
        return;
    };
    apply_gate(&host.lua, config.rust);
}

/// Corrige o gate enquanto o jogo corre (o load do menu pode repor o save).
fn sync_harvest_gate(
    time: Res<Time>,
    config: Res<VaelHarvestConfig>,
    host: Option<Res<LuaScriptHost>>,
    mut timer: Local<Option<Timer>>,
) {
    let Some(host) = host else {
        return;
    };
    let timer =
        timer.get_or_insert_with(|| Timer::from_seconds(GATE_PERIOD_SECS, TimerMode::Repeating));
    if !timer.tick(time.delta()).just_finished() {
        return;
    }
    apply_gate(&host.lua, config.rust);
}

fn apply_gate(lua: &Lua, rust: bool) {
    let current = read_gate(lua);
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
        apply_patch(lua, &patch);
    }
}

fn read_gate(lua: &Lua) -> Option<bool> {
    let game: Table = lua.named_registry_value("viber_game").ok()?;
    match game.raw_get::<LuaValue>(RUST_GATE_KEY).ok()? {
        LuaValue::Boolean(value) => Some(value),
        _ => None,
    }
}

/// Copia os props autorais para [`Harvestable`] (PostStartup + SceneLoad).
fn attach_harvestables(
    mut commands: Commands,
    root: Option<Res<VaelProjectRoot>>,
    props: Query<(Entity, &ScriptProps, Option<&Name>, Option<&SceneSource>), Without<Harvestable>>,
) {
    let root = root.as_deref().map(|root| root.0.as_path());
    for (entity, props, name, source) in &props {
        let name = name.map(Name::as_str).unwrap_or("");
        let key = stable_key(Some(name), source, root);
        if let Some(harvestable) = Harvestable::from_props(&props.0, name, key) {
            commands.entity(entity).insert(harvestable);
        }
    }
}

/// Atualiza o alvo/prompt da colheita a cada frame.
#[allow(clippy::too_many_arguments)]
fn update_harvest_focus(
    mut commands: Commands,
    time: Res<Time>,
    mut focus: ResMut<HarvestFocus>,
    players: Query<&GlobalTransform, With<Player>>,
    harvestables: Query<(Entity, &GlobalTransform, &Harvestable, Option<&Health>), Without<Player>>,
    menus: Option<Res<MenusOpen>>,
    modals: Option<Res<viber::ui::modal::UiModalsOpen>>,
    scene: Option<Res<CurrentScene>>,
    vitals: Option<Res<PlayerVitals>>,
    mut ui: Option<ResMut<UiScriptBinds>>,
) {
    focus.cooldown = (focus.cooldown - time.delta_secs()).max(0.0);
    let blocked = scene.as_deref().and_then(|scene| scene.id.as_deref()) == Some("menu")
        || modals
            .as_deref()
            .is_some_and(|modals| modals.is_open("title"))
        || menus.as_deref().is_some_and(MenusOpen::any)
        || vitals.as_deref().is_some_and(|vitals| vitals.hp <= 0.0);
    focus.blocked = blocked;
    if blocked {
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
    let nearest = nearest_within(
        origin,
        harvestables
            .iter()
            .filter(|(_, _, harvestable, health)| {
                harvestable.remaining > 0 && health.is_none_or(|health| health.current > 0.0)
            })
            .map(|(entity, transform, harvestable, _)| {
                (entity, transform.translation(), harvestable.range * scale)
            }),
    );
    let Some((entity, _)) = nearest else {
        clear_focus(&mut commands, &mut focus, ui.as_deref_mut());
        return;
    };
    let Ok((_, _, harvestable, _)) = harvestables.get(entity) else {
        clear_focus(&mut commands, &mut focus, ui.as_deref_mut());
        return;
    };
    if focus.entity != Some(entity) {
        if let Some(previous) = focus.entity.replace(entity) {
            commands.entity(previous).try_remove::<ScriptInteraction>();
        }
        commands.entity(entity).try_insert(ScriptInteraction {
            label: harvestable.label().to_string(),
            key: KeyCode::KeyJ,
            range: harvestable.range * scale,
        });
    }
    set_tool(&mut focus, harvestable.tool(), ui.as_deref_mut());
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
                speed: 1.0,
                lying,
            },
        ))
        .id();
    if let Some(look) = look {
        commands.entity(tree).insert(*look);
    }
    tree
}

/// J num alvo em foco: um golpe; no último, loot/XP/persistência/quebra.
#[allow(clippy::too_many_arguments)]
fn harvest_input(
    mut commands: Commands,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mut focus: ResMut<HarvestFocus>,
    mut save: Option<ResMut<VaelSave>>,
    content: Option<Res<VaelContent>>,
    host: Option<Res<LuaScriptHost>>,
    typing: Option<Res<UiFocusedInput>>,
    interaction: Option<Res<InteractionFocus>>,
    time: Res<Time>,
    mut shakes: Option<ResMut<TreeShakes>>,
    mut harvestables: Query<(
        &mut Harvestable,
        &GlobalTransform,
        Option<&Transform>,
        Option<&PooledTree>,
        Option<&PooledLook>,
    )>,
    mut spawns: Option<ResMut<PendingScriptSpawns>>,
    players: Query<&GlobalTransform, With<Player>>,
    queue: Option<Res<QuestEventQueue>>,
) {
    let Some(keys) = keys else {
        return;
    };
    if !keys.just_pressed(KeyCode::KeyJ) || focus.blocked || focus.cooldown > 0.0 {
        return;
    }
    // Com um `UiInput` focado as teclas são texto (o `viber.input` da engine
    // também as bloqueia): o J do chat não pode partir uma árvore.
    if typing.as_deref().is_some_and(|typing| typing.0.is_some()) {
        return;
    }
    let Some(entity) = focus.entity else {
        return;
    };
    // Outro [J] nativo mais próximo é dono do prompt (o `other_j_prompt`).
    if let Some(interaction) = interaction.as_deref()
        && let Some(winner) = interaction.winner(KeyCode::KeyJ)
        && winner != entity
    {
        return;
    }
    let Ok((mut harvestable, transform, local, pooled, look)) = harvestables.get_mut(entity) else {
        focus.entity = None;
        return;
    };
    let position = transform.translation();
    let player = players
        .iter()
        .next()
        .map(GlobalTransform::translation)
        .unwrap_or(Vec3::ZERO);
    let style = harvestable.style;
    let kind = harvestable.kind.clone();
    let broken = apply_hit(&mut harvestable);
    focus.cooldown = SWING_COOLDOWN;
    if !broken {
        play_sfx(&mut commands, hit_clip(style, &kind), position);
        // Árvore procedural: a copa abana no golpe e larga folhas
        // (`FallingLeaves::per_shake`). Prop antigo não tem de onde abanar.
        if pooled.is_some()
            && let Some(shakes) = shakes.as_mut()
        {
            let wave = (position - player).normalize_or_zero();
            shakes.shake(
                TreeShake {
                    at: Vec2::new(position.x, position.z),
                    radius: 1.8,
                    strength: 0.9,
                    direction: Vec2::new(wave.x, wave.z),
                    duration: 0.5,
                    start: 0.0,
                },
                &time,
            );
        }
        return;
    }

    // Loot e XP: o crédito é TENTADO antes de partir; com a bolsa cheia o
    // parcial é devolvido e o prop fica (uma mão-cheia de frutas não pode
    // pagar XP por nada).
    let lua = host.as_deref().map(|host| &host.lua);
    let mut added = 0;
    let mut full = false;
    if let (Some(content), Some(save)) = (content.as_deref(), save.as_deref_mut()) {
        let vault = VaultRules::from_items(&content.items);
        let xp_rules = XpRules::from_progression(&content.progression);
        if let Some(lua) = lua {
            seed_inventory(lua, &mut save.inventory, &kind);
            seed_xp(lua, &mut save.progression);
        }
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
            if let Some(lua) = lua {
                patch_rewards(lua, &kind, &save.inventory, &save.progression);
            }
            if added > 0
                && let Some(queue) = queue.as_deref()
            {
                queue.push(QuestEvent::SyncCollect);
            }
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
        mark_harvested(lua, save.as_deref_mut(), key);
        if let Some(proto) = replacement.as_deref() {
            store_stump(
                lua,
                save.as_deref_mut(),
                key,
                proto,
                position,
                replace_scale,
            );
        }
    }
    // Quebra: substituto, topple ou despawn.
    if let Some(proto) = replacement.as_deref() {
        if let Some(spawns) = spawns.as_deref_mut() {
            spawns.0.push(PendingPrototypeSpawn {
                name: proto.to_string(),
                pos: position,
                // O Lua passa o Y autorado; `seat: false` mantém a pose.
                seat: false,
                on_spawned: None,
                caller_path: None,
            });
        }
        commands.entity(entity).try_despawn();
    } else if style == BreakStyle::Fall {
        let direction = (position - player).normalize_or_zero();
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
            );
            commands
                .entity(entity)
                .try_remove::<(
                    Harvestable,
                    ScriptInteraction,
                    viber::luau::components::LuaScriptRef,
                )>()
                .remove::<PooledTree>()
                .insert(FloraFellTree);
            viber::physics::replace_with_stump_collider(
                &mut commands,
                entity,
                STUMP_RADIUS,
                STUMP_HALF_HEIGHT,
            );
            if let Some(key) = key.as_deref() {
                commands.entity(tree).insert(FloraFellKey(key.to_string()));
                store_flora_fell(lua, save.as_deref_mut(), key, direction, cut_height, false);
            }
        } else {
            // Prop antigo (era GLB): tombo da engine, como antes.
            let initial = local.map(|local| local.rotation).unwrap_or_default();
            commands.entity(entity).try_remove::<(
                Harvestable,
                ScriptInteraction,
                viber::luau::components::LuaScriptRef,
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

    play_sfx(&mut commands, break_clip(style, &kind), position);
    if !popup.is_empty() {
        let text = if added > 0 {
            format!("+{added} {popup}")
        } else {
            popup
        };
        show_popup(&mut commands, position + Vec3::Y * 1.4, text, popup_color);
    }
}

/// Lê o cofre Lua para o estado tipado antes de somar (o snapshot de 10 Hz
/// pode estar atrasado).
fn seed_inventory(lua: &Lua, inventory: &mut Inventory, kind: &str) {
    if kind.is_empty() {
        return;
    }
    if let Some(count) = read_game_i64(lua, &item_key(kind)) {
        inventory.set(kind, count);
    }
}

fn seed_xp(lua: &Lua, state: &mut XpState) {
    let mut kv = xp_map_from_state(state);
    for key in XP_KEYS {
        if let Some(value) = read_game_f64(lua, key) {
            kv.insert(key.to_string(), value);
        }
    }
    apply_xp_map(&kv, state);
}

fn patch_rewards(lua: &Lua, kind: &str, inventory: &Inventory, xp: &XpState) {
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
    apply_patch(lua, &patch);
}

fn read_game_i64(lua: &Lua, key: &str) -> Option<i64> {
    let game: Table = lua.named_registry_value("viber_game").ok()?;
    match game.raw_get::<LuaValue>(key).ok()? {
        LuaValue::Integer(value) => Some(value),
        LuaValue::Number(value) if value.is_finite() => Some(value as i64),
        _ => None,
    }
}

fn read_game_f64(lua: &Lua, key: &str) -> Option<f64> {
    let game: Table = lua.named_registry_value("viber_game").ok()?;
    match game.raw_get::<LuaValue>(key).ok()? {
        LuaValue::Integer(value) => Some(value as f64),
        LuaValue::Number(value) if value.is_finite() => Some(value),
        _ => None,
    }
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

fn kv_snapshot(lua: Option<&Lua>, save: Option<&VaelSave>) -> BTreeMap<String, Json> {
    if let Some(lua) = lua {
        let kv = game_to_json(lua);
        if !kv.is_empty() {
            return kv;
        }
    }
    save.map(|save| save.legacy.clone()).unwrap_or_default()
}

fn reap_saved(
    commands: &mut Commands,
    lua: Option<&Lua>,
    save: Option<&VaelSave>,
    mut spawns: Option<&mut PendingScriptSpawns>,
    harvestables: &Query<(
        Entity,
        &Harvestable,
        Option<&PooledTree>,
        Option<&PooledLook>,
        Option<&FloraFellTree>,
        &GlobalTransform,
    )>,
) {
    // Um só snapshot por varredura: `harvested.*` e `stump.*` saem da mesma
    // tabela (o `game_to_json` clona o jogo inteiro).
    let kv = kv_snapshot(lua, save);
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
        if let Some(fell) = flora_fell_from_kv(&kv, &matched) {
            let Some(pooled) = pooled else { continue };
            if fell_tree.is_some() {
                continue;
            }
            let tree = spawn_felled_tree(
                commands,
                pooled,
                look,
                global,
                Vec3::new(fell.dx, 0.0, fell.dz),
                fell.cut_height,
                true,
            );
            commands.entity(tree).insert(FloraFellKey(matched.clone()));
            if fell.bucked {
                commands.entity(tree).try_insert(BuckTree {
                    log_length: LOG_LENGTH,
                    min_radius: LOG_MIN_RADIUS,
                });
            }
            commands
                .entity(entity)
                .try_remove::<(Harvestable, ScriptInteraction)>()
                .remove::<PooledTree>()
                .insert(FloraFellTree);
            viber::physics::replace_with_stump_collider(
                commands,
                entity,
                STUMP_RADIUS,
                STUMP_HALF_HEIGHT,
            );
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
    }
}

/// PostStartup/SceneLoad: apaga já os props que o save diz estarem colhidos.
fn reap_on_load(
    mut commands: Commands,
    host: Option<Res<LuaScriptHost>>,
    save: Option<Res<VaelSave>>,
    mut spawns: Option<ResMut<PendingScriptSpawns>>,
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
        host.as_deref().map(|host| &host.lua),
        save.as_deref(),
        spawns.as_deref_mut(),
        &harvestables,
    );
}

/// O botão Carregar do menu pode trazer o save DEPOIS do arranque (o
/// `reap_saved` do Lua também não é one-shot).
fn reap_periodic(
    time: Res<Time>,
    mut timer: Local<Option<Timer>>,
    mut commands: Commands,
    host: Option<Res<LuaScriptHost>>,
    save: Option<Res<VaelSave>>,
    mut spawns: Option<ResMut<PendingScriptSpawns>>,
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
        host.as_deref().map(|host| &host.lua),
        save.as_deref(),
        spawns.as_deref_mut(),
        &harvestables,
    );
}

/// As árvores tombadas são raízes (fora da hierarquia da cena): numa troca de
/// cena ficariam a flutuar. O estado persiste no save e o restauro repõe-nas.
fn despawn_fell_trees(
    mut commands: Commands,
    current: Option<Res<CurrentScene>>,
    trees: Query<Entity, Or<(With<FellTree>, With<Felled>)>>,
    mut last: Local<Option<String>>,
) {
    let Some(current) = current else {
        return;
    };
    let id = current.id.clone().unwrap_or_default();
    if last.as_deref() == Some(id.as_str()) {
        return;
    }
    let first = last.is_none();
    *last = Some(id);
    if first {
        return;
    }
    for tree in &trees {
        commands.entity(tree).despawn();
    }
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
    time: Res<Time>,
    mut focus: ResMut<BuckFocus>,
    interaction: Option<Res<InteractionFocus>>,
    host: Option<Res<LuaScriptHost>>,
    mut save: Option<ResMut<VaelSave>>,
    players: Query<&GlobalTransform, With<Player>>,
    felled: Query<
        (
            Entity,
            &GlobalTransform,
            Option<&BuckTree>,
            Option<&FloraFellKey>,
        ),
        Or<(With<Felled>, With<FellTree>)>,
    >,
) {
    focus.cooldown = (focus.cooldown - time.delta_secs()).max(0.0);
    let Some(keys) = keys else {
        return;
    };
    let Some(player) = players.iter().next().map(GlobalTransform::translation) else {
        return;
    };
    let reach = BUCK_RANGE * range_scale();
    let mut best: Option<(Entity, f32)> = None;
    for (entity, transform, bucked, _) in &felled {
        if bucked.is_some() {
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
                key: KeyCode::KeyJ,
                range: reach,
            });
        }
        focus.entity = target;
    }
    let Some(target) = target else {
        return;
    };
    if !keys.just_pressed(KeyCode::KeyJ) || focus.cooldown > 0.0 {
        return;
    }
    // Outro [J] nativo mais próximo (o prompt da colheita) tem prioridade.
    if let Some(interaction) = interaction.as_deref()
        && let Some(winner) = interaction.winner(KeyCode::KeyJ)
        && winner != target
    {
        return;
    }
    commands.entity(target).try_insert(BuckTree {
        log_length: LOG_LENGTH,
        min_radius: LOG_MIN_RADIUS,
    });
    // Persistência ANTES: o crate pode despawnar a entidade no mesmo frame.
    if let Ok((_, _, _, Some(key))) = felled.get(target) {
        store_flora_bucked(
            host.as_deref().map(|host| &host.lua),
            save.as_deref_mut(),
            &key.0,
        );
    }
    focus.cooldown = BUCK_COOLDOWN;
    focus.entity = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::MinimalPlugins;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::input::ButtonInput;
    use std::path::PathBuf;
    use std::sync::Arc;
    use viber::xml::NodeOrigin;

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
        let stump = stump_for(&kv, "tree@forest.xml#1").expect("stump");
        assert_eq!(stump.proto, "berry-bush-bare");
        assert_eq!((stump.x, stump.y, stump.z), (1.5, Some(2.0), -3.0));
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
        assert!((interaction.range - viber::interact::scaled_range(3.5)).abs() < 1e-5);

        // Move o prop para longe: o prompt tem de sair.
        if let Some(mut transform) = app.world_mut().get_mut::<GlobalTransform>(prop) {
            *transform = GlobalTransform::from_translation(Vec3::new(100.0, 0.0, 0.0));
        }
        app.update();
        assert_eq!(app.world().resource::<HarvestFocus>().entity, None);
        assert!(app.world().get::<ScriptInteraction>(prop).is_none());
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
            None,
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
        let mut app = app_rust();
        app.insert_resource(content_com(&[("wood", "resource", None)]));
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
    }

    /// O `reap_saved` repõe a árvore TOMBADA do save (em vez de a despawnar):
    /// `FellTree { lying: true }` + chave `flora-fell` na entidade.
    #[test]
    fn reap_restaura_arvore_deitada_do_save() {
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
                None,
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

    /// `[J]` numa árvore deitada retalha-a em toros e marca o save.
    #[test]
    fn j_na_arvore_deitada_insere_buck_tree() {
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

        assert_eq!(
            app.world().get::<Harvestable>(prop).map(|h| h.remaining),
            Some(2)
        );
        assert_eq!(app.world().resource::<VaelSave>().inventory.wood, 0);
        assert!(app.world().resource::<VaelSave>().harvested.is_empty());
    }

    #[test]
    fn reap_apaga_o_prop_do_save_e_preserva_os_outros() {
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
        let lua = Lua::new();
        let game = lua.create_table().expect("tabela");
        lua.set_named_registry_value("viber_game", game.clone())
            .expect("registry");
        let inventory = Inventory::default();
        let xp = XpState {
            xp: HARVEST_XP as i64,
            xp_next: 100,
            level: 1,
            skill_points: 0,
            ..Default::default()
        };
        patch_rewards(&lua, "", &inventory, &xp);
        assert_eq!(game.raw_get::<f64>(KEY_XP).expect("xp"), HARVEST_XP);
    }

    #[test]
    fn store_stump_grava_a_pose_no_legacy() {
        let mut save = VaelSave::default();
        store_stump(
            None,
            Some(&mut save),
            "tree@forest.xml#1",
            "berry-bush-bare",
            Vec3::new(1.5, 2.0, -3.0),
            1.25,
        );
        let stump = stump_for(&save.legacy, "tree@forest.xml#1").expect("stump");
        assert_eq!(stump.proto, "berry-bush-bare");
        assert_eq!((stump.x, stump.y, stump.z), (1.5, Some(2.0), -3.0));
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
        let lua = Lua::new();
        let game = lua.create_table().expect("tabela");
        lua.set_named_registry_value("viber_game", game.clone())
            .expect("registry");

        apply_gate(&lua, true);
        assert_eq!(
            game.raw_get::<LuaValue>(RUST_GATE_KEY).expect("gate"),
            LuaValue::Boolean(true)
        );
        apply_gate(&lua, false);
        assert!(matches!(
            game.raw_get::<LuaValue>(RUST_GATE_KEY).expect("gate"),
            LuaValue::Nil
        ));
    }
}

//! Contornos (outlines) dinâmicos e vibrantes com `bevy_mod_outline` (jump-flood).
//!
//! Destaca visualmente tudo o que o jogador pode colher, quebrar ou apanhar:
//! - **Árvores e troncos** (`Harvestable` de `chop`, `GroundLog`, `Felled`):
//!   âmbar dourado quente ao alcance (nas `PooledTree` do `bevy_flora`, aplica-se
//!   aos filhos de madeira — tronco/ramos — para evitar desfasamento de vento nas
//!   folhas e manter a silhueta do tronco limpa).
//! - **Rochas, minérios e cristais** (`Harvestable` de `mine`, `QuarryStone`,
//!   `CrystalGrowth`): ciano cristalino luminoso (ou prismático turquesa-violeta
//!   quando a rocha carrega cristais).
//! - **Coletáveis botânicos** (`berry-bush`, `mushroom.lua`, `bog-moss.lua`):
//!   verde-esmeralda vivo.
//! - **Baús fechados** (`chest.lua` não vazio): dourado tesouro.
//! - **Props destrutíveis** (`Breakable` — barris, caixas, bancos): laranja-cobre
//!   quente ao alcance de golpe, mudando para coral incandescente quando rachados.
//! - **Itens largados por destrutíveis** (`PropPickup` — vida, ouro, madeira):
//!   outline vibrante contínuo enquanto estiverem no chão até `PICKUP_OUTLINE_RANGE`,
//!   intensificando quando o herói se aproxima.
//!
//! Gate: `VAEL_OUTLINE=0` desliga todo o sistema (A/B).

use std::collections::HashMap;
use std::f32::consts::TAU;

use avian3d::prelude::Collider;
use bevy::camera::visibility::VisibilityRange;
use bevy::prelude::*;
use bevy_flora::{BuckTree, FellTree, Felled, FloraMaterial, PooledTree};
use bevy_mod_outline::{
    InheritOutline, OutlineAlphaMask, OutlinePlugin, OutlineStencil, OutlineStencilEnabled,
    OutlineVolume, OutlineWarmUp, PropagateOutline, StopPropagateOutline, TextureChannel,
};
use vael_engine::interact::{InteractionFocus, range_scale};
use vael_engine::luau::components::{LuaScriptRef, ScriptInteraction};
use vael_engine::menus::MenusOpen;
use vael_engine::player::Player;
use vael_engine::scene_loader::CurrentScene;
use vael_engine::vitals::Health;

use crate::breakables::{Breakable, PickupKind, PropPickup};
use crate::collect_fx::{HitFlashQueue, IgnoreOutline, ImpactWobble, apply_impact_wobble};
use crate::components::PlayerVitals;
use crate::harvest::{BUCK_RANGE, BuckFocus, HarvestFocus, Harvestable, timber};
use crate::rocks::CrystalGrowth;

/// Variável de ambiente para desligar outlines (`VAEL_OUTLINE=0`).
pub const OUTLINE_ENV: &str = "VAEL_OUTLINE";

/// Alcance (m) em que props destrutíveis (barris/caixas/bancos) ganham outline.
pub const BREAKABLE_OUTLINE_RANGE: f32 = 3.4;
/// Raio (m) em que itens largados no chão mantêm outline vibrante visível.
pub const PICKUP_OUTLINE_RANGE: f32 = 24.0;
/// Raio (m) de proximidade em que o loot no chão ganha destaque extra.
const PICKUP_FOCUS_RANGE: f32 = 3.5;

const FADE_IN_SPEED: f32 = 7.5;
const FADE_OUT_SPEED: f32 = 5.2;
const FOCUS_LERP_SPEED: f32 = 12.0;
const FLASH_DECAY_SPEED: f32 = 5.5;

/// `true` quando o sistema de outlines está ativo (default ON; `VAEL_OUTLINE=0` desliga).
pub fn outline_enabled() -> bool {
    !matches!(
        std::env::var(OUTLINE_ENV)
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "0" | "false" | "off" | "no"
    )
}

/// Estilo visual do contorno conforme o tipo de coletável/destrutível.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutlineStyle {
    Wood,
    Stone,
    Crystal,
    Flora,
    Chest,
    Breakable,
    BreakableCracked,
    LootLife,
    LootGold,
    LootWood,
}

impl OutlineStyle {
    /// Cor base (alcance) e cor em foco (alvo prioritário) em sRGB.
    fn colors(self, now: f32, phase: f32) -> (Color, Color) {
        match self {
            Self::Wood => (Color::srgb(0.98, 0.74, 0.24), Color::srgb(1.0, 0.88, 0.44)),
            Self::Stone => (Color::srgb(0.32, 0.86, 0.98), Color::srgb(0.56, 0.96, 1.0)),
            Self::Crystal => {
                // Brilho prismático suave entre turquesa e violeta-cristal.
                let shimmer = (now * 2.6 + phase).sin() * 0.5 + 0.5;
                let base = Color::srgb(0.28, 0.94, 0.88)
                    .mix(&Color::srgb(0.68, 0.58, 1.0), shimmer * 0.42);
                let focus = Color::srgb(0.52, 1.0, 0.95)
                    .mix(&Color::srgb(0.84, 0.72, 1.0), shimmer * 0.35);
                (base, focus)
            }
            Self::Flora => (Color::srgb(0.34, 0.94, 0.52), Color::srgb(0.56, 1.0, 0.70)),
            Self::Chest => (Color::srgb(1.0, 0.84, 0.24), Color::srgb(1.0, 0.94, 0.52)),
            Self::Breakable => (Color::srgb(1.0, 0.58, 0.20), Color::srgb(1.0, 0.74, 0.34)),
            Self::BreakableCracked => (Color::srgb(1.0, 0.36, 0.20), Color::srgb(1.0, 0.54, 0.28)),
            Self::LootLife => (Color::srgb(1.0, 0.30, 0.56), Color::srgb(1.0, 0.52, 0.72)),
            Self::LootGold => (Color::srgb(1.0, 0.84, 0.18), Color::srgb(1.0, 0.95, 0.50)),
            Self::LootWood => (Color::srgb(0.98, 0.66, 0.26), Color::srgb(1.0, 0.82, 0.44)),
        }
    }

    /// Largura lógica base (px) ao alcance e em foco ativo.
    fn widths(self) -> (f32, f32) {
        match self {
            Self::LootLife | Self::LootGold | Self::LootWood => (2.8, 4.2),
            Self::Wood => (2.4, 3.8),
            Self::Stone | Self::Crystal => (2.5, 4.0),
            Self::Flora | Self::Chest => (2.4, 3.8),
            Self::Breakable | Self::BreakableCracked => (2.3, 3.7),
        }
    }

    /// Frequência (rad/s) e amplitude relativa da respiração orgânica.
    fn pulse_params(self) -> (f32, f32) {
        match self {
            Self::LootLife => (5.2, 0.24),
            Self::LootGold | Self::LootWood => (4.2, 0.20),
            Self::Crystal => (3.8, 0.18),
            Self::BreakableCracked => (6.0, 0.22),
            _ => (3.0, 0.14),
        }
    }

    fn is_loot(self) -> bool {
        matches!(self, Self::LootLife | Self::LootGold | Self::LootWood)
    }
}

/// Estado animado de contorno aplicado a um coletável/destrutível/pickup.
#[derive(Component, Debug, Clone)]
pub(crate) struct AnimatedOutline {
    pub style: OutlineStyle,
    pub weight: f32,
    pub focused: bool,
    pub focus_weight: f32,
    pub flash: f32,
    pub tree_wood_only: bool,
    pub phase: f32,
}

/// Flashes de impacto ativos por entidade (alimentados por [`HitFlashQueue`]).
#[derive(Resource, Default)]
struct ActiveHitFlashes(HashMap<Entity, f32>);

/// Pedido efémero calculado a cada frame para uma entidade ao alcance.
#[derive(Debug, Clone, Copy)]
struct DesiredOutline {
    style: OutlineStyle,
    focused: bool,
    tree_wood_only: bool,
}

/// Plugin que instala `bevy_mod_outline` (jump-flood) e conduz os contornos
/// dinâmicos dos coletáveis, árvores, rochas, destrutíveis e loot.
pub struct VaelOutlinePlugin;

impl Plugin for VaelOutlinePlugin {
    fn build(&self, app: &mut App) {
        if !outline_enabled() {
            return;
        }
        // Só instala o pipeline de render quando `RenderApp` existe (testes
        // headless com `MinimalPlugins` correm a lógica de estado sem panic).
        if app.get_sub_app(bevy::render::RenderApp).is_some() {
            app.add_plugins(OutlinePlugin::JUMP_FLOOD);
        }
        app.init_resource::<ActiveHitFlashes>().add_systems(
            Update,
            (
                suppress_ignored_outlines,
                drain_hit_flashes,
                update_collectible_outlines,
                sync_alpha_masks,
            )
                .chain(),
        );
    }
}

/// Impede que elementos decorativos (glints, anéis de chão, rachaduras) herdem
/// o outline do pai, sobrescrevendo o volume e o stencil para desligado.
fn suppress_ignored_outlines(
    mut commands: Commands,
    added: Query<Entity, Added<IgnoreOutline>>,
) {
    for entity in &added {
        commands.entity(entity).try_insert((
            StopPropagateOutline,
            OutlineVolume {
                visible: false,
                width: 0.0,
                colour: Color::NONE,
            },
            OutlineStencil {
                enabled: OutlineStencilEnabled::Never,
                offset: 0.0,
            },
        ));
    }
}

/// Aplica `OutlineAlphaMask` automático a descendentes herdados cujo material
/// `StandardMaterial` usa `AlphaMode::Mask` (ex: folhas de `berry-bush`).
fn sync_alpha_masks(
    mut commands: Commands,
    materials: Option<Res<Assets<StandardMaterial>>>,
    meshes: Query<
        (Entity, &MeshMaterial3d<StandardMaterial>),
        (With<InheritOutline>, Without<OutlineAlphaMask>, Without<IgnoreOutline>),
    >,
) {
    let Some(materials) = materials else {
        return;
    };
    for (entity, handle) in &meshes {
        let Some(mat) = materials.get(&handle.0) else {
            continue;
        };
        if let AlphaMode::Mask(threshold) = mat.alpha_mode
            && let Some(tex) = &mat.base_color_texture
        {
            commands.entity(entity).try_insert(OutlineAlphaMask {
                texture: Some(tex.clone()),
                channel: TextureChannel::A,
                threshold,
            });
        }
    }
}

/// Consome `HitFlashQueue`: regista o flash no outline e dispara `ImpactWobble`.
fn drain_hit_flashes(
    mut commands: Commands,
    queue: Option<Res<HitFlashQueue>>,
    mut active: ResMut<ActiveHitFlashes>,
    transforms: Query<(&Transform, Option<&ImpactWobble>, Has<PooledTree>)>,
) {
    let Some(queue) = queue else {
        return;
    };
    let requests = std::mem::take(&mut *queue.0.lock().unwrap_or_else(|e| e.into_inner()));
    for req in requests {
        active
            .0
            .entry(req.entity)
            .and_modify(|v| *v = v.max(req.strength))
            .or_insert(req.strength);
        if req.wobble
            && let Ok((tf, existing, is_pooled_tree)) = transforms.get(req.entity)
            && !is_pooled_tree
        {
            apply_impact_wobble(
                &mut commands,
                req.entity,
                tf.scale,
                existing,
                0.095 * req.strength,
            );
        }
    }
}

#[derive(bevy::ecs::system::SystemParam)]
struct OutlineGateCtx<'w> {
    harvest_focus: Option<Res<'w, HarvestFocus>>,
    buck_focus: Option<Res<'w, BuckFocus>>,
    interaction_focus: Option<Res<'w, InteractionFocus>>,
    menus: Option<Res<'w, MenusOpen>>,
    modals: Option<Res<'w, vael_engine::ui::modal::UiModalsOpen>>,
    scene: Option<Res<'w, CurrentScene>>,
    vitals: Option<Res<'w, PlayerVitals>>,
}

#[derive(bevy::ecs::system::SystemParam)]
struct TreeWoodLookup<'w, 's> {
    children_q: Query<'w, 's, &'static Children>,
    crystals: Query<'w, 's, (), With<CrystalGrowth>>,
    flora_mats: Option<Res<'w, Assets<FloraMaterial>>>,
    std_mats: Option<Res<'w, Assets<StandardMaterial>>>,
    wood_candidates: Query<
        'w,
        's,
        (
            Option<&'static MeshMaterial3d<FloraMaterial>>,
            Option<&'static MeshMaterial3d<StandardMaterial>>,
            Option<&'static VisibilityRange>,
            Has<InheritOutline>,
        ),
        With<Mesh3d>,
    >,
}

/// Avalia todos os coletáveis/destrutíveis/loot elegíveis e conduz o fade/pulso
/// suave dos `OutlineVolume` com limpeza automática fora de alcance.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update_collectible_outlines(
    mut commands: Commands,
    time: Res<Time>,
    mut flashes: ResMut<ActiveHitFlashes>,
    players: Query<&GlobalTransform, (With<Player>, Without<crate::boat::RidingBoat>)>,
    ctx: OutlineGateCtx,
    harvestables: Query<
        (
            Entity,
            &GlobalTransform,
            &Harvestable,
            Option<&Health>,
            Option<&Collider>,
            Has<PooledTree>,
            Has<timber::GroundLog>,
        ),
        (Without<Player>, Without<timber::Crown>),
    >,
    felled_trees: Query<
        (Entity, &GlobalTransform, Option<&Felled>, Option<&FellTree>),
        (
            Or<(With<Felled>, With<FellTree>)>,
            Without<BuckTree>,
            Without<timber::FallingTrunk>,
        ),
    >,
    breakables: Query<(Entity, &GlobalTransform, &Breakable), Without<Player>>,
    pickups: Query<(Entity, &GlobalTransform, &PropPickup), Without<Player>>,
    script_collectibles: Query<
        (
            Entity,
            &GlobalTransform,
            &ScriptInteraction,
            Option<&LuaScriptRef>,
            Option<&Health>,
        ),
        (
            Without<Harvestable>,
            Without<Breakable>,
            Without<PropPickup>,
            Without<Felled>,
            Without<FellTree>,
            Without<Player>,
        ),
    >,
    mut animated: Query<(Entity, &mut AnimatedOutline, &mut OutlineVolume)>,
    lookup: TreeWoodLookup,
) {
    let dt = time.delta_secs().min(0.05);
    let now = time.elapsed_secs();

    // Decaimento global dos flashes pendentes.
    flashes.0.retain(|_, v| {
        *v = (*v - dt * FLASH_DECAY_SPEED).max(0.0);
        *v > 0.001
    });

    let blocked = ctx.scene.as_deref().and_then(|s| s.id.as_deref()) == Some("menu")
        || ctx.modals.as_deref().is_some_and(|m| m.is_open("title"))
        || ctx.menus.as_deref().is_some_and(MenusOpen::any)
        || ctx.vitals.as_deref().is_some_and(|v| v.hp <= 0.0);

    let mut desired: HashMap<Entity, DesiredOutline> = HashMap::new();

    if !blocked
        && let Some(player_pose) = players.iter().next()
    {
        let origin = player_pose.translation();
        let hand = origin + Vec3::Y * 0.8;
        let facing = (player_pose.rotation() * Vec3::Z)
            .xz()
            .normalize_or(Vec2::Y);
        let scale = range_scale();
        let focused_harvest = ctx.harvest_focus.as_deref().and_then(|f| f.entity);
        let focused_buck = ctx.buck_focus.as_deref().and_then(|f| f.entity);
        let j_winner = ctx
            .interaction_focus
            .as_deref()
            .and_then(|f| f.winner(KeyCode::KeyJ));

        // 1. Árvores, rochas, troncos e arbustos (`Harvestable`).
        for (entity, tf, h, hp, collider, is_pooled_tree, is_log) in &harvestables {
            if h.remaining <= 0 || hp.is_some_and(|hp| hp.current <= 0.0) {
                continue;
            }
            let pos = tf.translation();
            let reach = h.range * scale;
            let tool = h.tool();
            let uses_tool = matches!(tool, "chop" | "mine");
            let in_range = if uses_tool {
                if !is_log && pos.xz().distance_squared(origin.xz()) > (reach + 4.5).powi(2) {
                    false
                } else {
                    let contact = collider
                        .map_or(pos, |c| c.project_point(pos, tf.rotation(), hand, true).0);
                    hand.distance_squared(contact) <= reach * reach
                }
            } else {
                origin.distance_squared(pos) <= reach * reach
            };
            if !in_range {
                continue;
            }
            let style = match tool {
                "chop" => OutlineStyle::Wood,
                "mine" => {
                    let has_crystal = lookup
                        .children_q
                        .get(entity)
                        .is_ok_and(|kids| kids.iter().any(|k| lookup.crystals.contains(k)));
                    if has_crystal {
                        OutlineStyle::Crystal
                    } else {
                        OutlineStyle::Stone
                    }
                }
                _ => OutlineStyle::Flora,
            };
            let focused = focused_harvest == Some(entity) || j_winner == Some(entity);
            desired.insert(
                entity,
                DesiredOutline {
                    style,
                    focused,
                    tree_wood_only: is_pooled_tree,
                },
            );
        }

        // 2. Árvores deitadas prontas para retalhar em toros.
        let buck_reach_sq = (BUCK_RANGE * scale).powi(2);
        for (entity, tf, state, req) in &felled_trees {
            if !(state.is_some_and(|s| s.at_rest) || req.is_some_and(|r| r.lying)) {
                continue;
            }
            if tf.translation().distance_squared(origin) <= buck_reach_sq {
                desired.entry(entity).or_insert(DesiredOutline {
                    style: OutlineStyle::Wood,
                    focused: focused_buck == Some(entity),
                    tree_wood_only: false,
                });
            }
        }

        // 3. Props destrutíveis (barris, caixas, bancos) ao alcance do golpe.
        let break_range_sq = BREAKABLE_OUTLINE_RANGE * BREAKABLE_OUTLINE_RANGE;
        let mut best_breakable: Option<(Entity, f32)> = None;
        for (entity, tf, prop) in &breakables {
            if prop.hits_left == 0 {
                continue;
            }
            let center = tf.translation() + Vec3::Y * 0.45;
            let delta = center - origin;
            if delta.y.abs() > 1.8 {
                continue;
            }
            let dist_sq = delta.xz().length_squared();
            if dist_sq > break_range_sq {
                continue;
            }
            let style = if prop.hits_left == 1 {
                OutlineStyle::BreakableCracked
            } else {
                OutlineStyle::Breakable
            };
            desired.entry(entity).or_insert(DesiredOutline {
                style,
                focused: false,
                tree_wood_only: false,
            });
            let flat = delta.xz();
            let in_front = flat.length_squared() < 0.01 || facing.dot(flat.normalize()) >= 0.2;
            if in_front && best_breakable.is_none_or(|(_, best)| dist_sq < best) {
                best_breakable = Some((entity, dist_sq));
            }
        }
        if let Some((entity, _)) = best_breakable
            && let Some(entry) = desired.get_mut(&entity)
        {
            entry.focused = true;
        }

        // 4. Coletáveis scriptados (cogumelos, musgo, arbustos com script, baús fechados).
        for (entity, tf, interaction, script, hp) in &script_collectibles {
            if hp.is_some_and(|h| h.current <= 0.0) {
                continue;
            }
            let dist_sq = tf.translation().distance_squared(origin);
            if dist_sq > interaction.range * interaction.range {
                continue;
            }
            let Some(style) = classify_script_collectible(interaction, script) else {
                continue;
            };
            let focused = ctx
                .interaction_focus
                .as_deref()
                .is_some_and(|f| f.wins(interaction.key, entity));
            desired.entry(entity).or_insert(DesiredOutline {
                style,
                focused,
                tree_wood_only: false,
            });
        }

        // 5. Itens que caem dos destrutíveis — outline vibrante contínuo no chão!
        let pickup_range_sq = PICKUP_OUTLINE_RANGE * PICKUP_OUTLINE_RANGE;
        let pickup_focus_sq = PICKUP_FOCUS_RANGE * PICKUP_FOCUS_RANGE;
        for (entity, tf, pickup) in &pickups {
            let dist_sq = tf.translation().distance_squared(origin);
            if dist_sq > pickup_range_sq {
                continue;
            }
            let style = match pickup.kind {
                PickupKind::Life => OutlineStyle::LootLife,
                PickupKind::Gold => OutlineStyle::LootGold,
                PickupKind::Wood => OutlineStyle::LootWood,
            };
            desired.insert(
                entity,
                DesiredOutline {
                    style,
                    focused: dist_sq <= pickup_focus_sq,
                    tree_wood_only: false,
                },
            );
        }
    }

    // Atualiza entidades que já têm contorno animado.
    for (entity, mut anim, mut volume) in &mut animated {
        if let Some(target) = desired.remove(&entity) {
            anim.style = target.style;
            anim.focused = target.focused;
            anim.tree_wood_only = target.tree_wood_only;
            anim.weight = (anim.weight + dt * FADE_IN_SPEED).min(1.0);
        } else {
            anim.focused = false;
            anim.weight = (anim.weight - dt * FADE_OUT_SPEED).max(0.0);
        }
        let focus_target = if anim.focused { 1.0 } else { 0.0 };
        anim.focus_weight += (focus_target - anim.focus_weight) * (dt * FOCUS_LERP_SPEED).min(1.0);
        if let Some(&hit_flash) = flashes.0.get(&entity) {
            anim.flash = anim.flash.max(hit_flash);
        } else {
            anim.flash = (anim.flash - dt * FLASH_DECAY_SPEED).max(0.0);
        }

        if anim.weight <= 0.001 && anim.flash <= 0.001 {
            remove_outline(
                &mut commands,
                entity,
                anim.tree_wood_only,
                &lookup.children_q,
                &lookup.wood_candidates,
            );
            continue;
        }

        apply_volume(&mut volume, &anim, now);
        if anim.tree_wood_only {
            mark_tree_wood_children(
                &mut commands,
                entity,
                &lookup.children_q,
                &lookup.wood_candidates,
                lookup.flora_mats.as_deref(),
                lookup.std_mats.as_deref(),
            );
        }
    }

    // Anexa contorno às entidades que acabaram de entrar em alcance (ou loot recém-criado).
    for (entity, target) in desired {
        let phase = (entity.index_u32().wrapping_mul(2_654_435_761) % 1024) as f32 / 1024.0 * TAU;
        let initial_weight = if target.style.is_loot() { 0.85 } else { 0.22 };
        let initial_flash = flashes.0.get(&entity).copied().unwrap_or(0.0);
        let anim = AnimatedOutline {
            style: target.style,
            weight: initial_weight,
            focused: target.focused,
            focus_weight: if target.focused { 1.0 } else { 0.0 },
            flash: initial_flash,
            tree_wood_only: target.tree_wood_only,
            phase,
        };
        let mut volume = OutlineVolume::default();
        apply_volume(&mut volume, &anim, now);
        let mut cmd = commands.entity(entity);
        cmd.try_insert((
            anim,
            volume,
            OutlineStencil {
                enabled: OutlineStencilEnabled::IfVolume,
                offset: 0.0,
            },
            OutlineWarmUp::default().with_transparency(true),
        ));
        if target.tree_wood_only {
            mark_tree_wood_children(
                &mut commands,
                entity,
                &lookup.children_q,
                &lookup.wood_candidates,
                lookup.flora_mats.as_deref(),
                lookup.std_mats.as_deref(),
            );
        } else {
            cmd.try_insert(PropagateOutline);
        }
    }
}

fn classify_script_collectible(
    interaction: &ScriptInteraction,
    script: Option<&LuaScriptRef>,
) -> Option<OutlineStyle> {
    let path = script.map(|s| s.path.as_str()).unwrap_or("");
    if path.ends_with("chest.lua") {
        let empty = interaction.label.trim().eq_ignore_ascii_case("vazio");
        return (!empty).then_some(OutlineStyle::Chest);
    }
    if path.ends_with("song-seed.lua") {
        return Some(OutlineStyle::Crystal);
    }
    if path.ends_with("mushroom.lua")
        || path.ends_with("bog-moss.lua")
        || path.ends_with("fruit-tree.lua")
        || interaction.key == KeyCode::KeyJ
    {
        return Some(OutlineStyle::Flora);
    }
    None
}

fn apply_volume(volume: &mut OutlineVolume, anim: &AnimatedOutline, now: f32) {
    let (base_col, focus_col) = anim.style.colors(now, anim.phase);
    let (base_w, focus_w) = anim.style.widths();
    let (freq, amp) = anim.style.pulse_params();

    let focus_t = anim.focus_weight.clamp(0.0, 1.0);
    let pulse = (now * freq + anim.phase).sin() * 0.5 + 0.5;
    let breath = 1.0 + (pulse - 0.5) * 2.0 * amp * 0.85f32.lerp(1.25, focus_t);

    // Smoothstep no peso de entrada/saída para fade sedoso.
    let w = anim.weight.clamp(0.0, 1.0);
    let smooth = w * w * (3.0 - 2.0 * w);

    let flash = anim.flash.clamp(0.0, 1.5);
    let mut colour = base_col.mix(&focus_col, focus_t);
    if flash > 0.01 {
        colour = colour.mix(&Color::srgb(1.0, 0.98, 0.88), (flash * 0.82).min(0.92));
    }
    let alpha = (0.72f32.lerp(0.96, focus_t) * smooth + flash * 0.25).clamp(0.0, 1.0);
    colour.set_alpha(alpha);

    // Mantém largura acima de 1 px durante o fade-in/fade-out enquanto o alpha
    // desvanece até 0: em jump-flood, largura < 1 px sumia de golpe.
    let width = base_w.lerp(focus_w, focus_t) * breath * (0.55 + 0.45 * smooth) + flash * 2.1;

    volume.visible = alpha > 0.01 && width > 0.1;
    volume.width = width.clamp(0.0, 8.5);
    volume.colour = colour;
}

/// Marca apenas os filhos de madeira LOD0 (tronco/ramos opacos próximos) de uma
/// `PooledTree` com `InheritOutline`, deixando folhas alpha-masked com vento,
/// LODs distantes (ocultos por `VisibilityRange`) e impostores de fora.
fn mark_tree_wood_children(
    commands: &mut Commands,
    tree: Entity,
    children_q: &Query<&Children>,
    candidates: &Query<
        (
            Option<&MeshMaterial3d<FloraMaterial>>,
            Option<&MeshMaterial3d<StandardMaterial>>,
            Option<&VisibilityRange>,
            Has<InheritOutline>,
        ),
        With<Mesh3d>,
    >,
    flora_mats: Option<&Assets<FloraMaterial>>,
    std_mats: Option<&Assets<StandardMaterial>>,
) {
    let Ok(children) = children_q.get(tree) else {
        return;
    };
    for child in children.iter() {
        let Ok((flora_handle, std_handle, vis_range, already)) = candidates.get(child) else {
            continue;
        };
        if already {
            continue;
        }
        // `bevy_flora` cria filhos LOD0/LOD1/LOD2 simultaneamente com `VisibilityRange`;
        // o `bevy_mod_outline` não avalia `VisibilityRange`, então só LOD0 (start == 0)
        // deve receber contorno ao alcance de corte.
        if vis_range.is_some_and(|r| r.start_margin.start > 0.01) {
            continue;
        }
        let is_opaque_wood = flora_handle
            .and_then(|h| flora_mats?.get(&h.0))
            .is_some_and(|m| m.base.alpha_mode == AlphaMode::Opaque)
            || std_handle
                .and_then(|h| std_mats?.get(&h.0))
                .is_some_and(|m| m.alpha_mode == AlphaMode::Opaque);
        if is_opaque_wood {
            commands.entity(child).try_insert(InheritOutline);
        }
    }
}

fn remove_outline(
    commands: &mut Commands,
    entity: Entity,
    tree_wood_only: bool,
    children_q: &Query<&Children>,
    candidates: &Query<
        (
            Option<&MeshMaterial3d<FloraMaterial>>,
            Option<&MeshMaterial3d<StandardMaterial>>,
            Option<&VisibilityRange>,
            Has<InheritOutline>,
        ),
        With<Mesh3d>,
    >,
) {
    commands.entity(entity).try_remove::<(
        AnimatedOutline,
        OutlineVolume,
        OutlineStencil,
        OutlineWarmUp,
        PropagateOutline,
    )>();
    if tree_wood_only
        && let Ok(children) = children_q.get(entity)
    {
        for child in children.iter() {
            if candidates.get(child).is_ok_and(|(_, _, _, has)| has) {
                commands.entity(child).try_remove::<InheritOutline>();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bevy::time::TimeUpdateStrategy;

    use super::*;
    use crate::breakables::{Breakable, Kind, PickupKind, PropPickup};
    use crate::harvest::BreakStyle;

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
                50,
            )))
            .init_resource::<HitFlashQueue>()
            .init_resource::<HarvestFocus>()
            .init_resource::<BuckFocus>()
            .init_resource::<InteractionFocus>()
            .add_plugins(VaelOutlinePlugin);
        app
    }

    #[test]
    fn harvestable_gains_outline_in_range_and_fades_out_when_far() {
        let mut app = test_app();
        let hero = app
            .world_mut()
            .spawn((
                Player::default(),
                Transform::IDENTITY,
                GlobalTransform::IDENTITY,
            ))
            .id();
        let rock = app
            .world_mut()
            .spawn((
                Transform::from_xyz(1.2, 0.0, 0.0),
                GlobalTransform::from_xyz(1.2, 0.0, 0.0),
                Harvestable {
                    hits: 3,
                    remaining: 3,
                    style: BreakStyle::Shatter,
                    kind: "stone".into(),
                    amount: 1,
                    popup: "Pedra".into(),
                    popup_color: [1.0; 3],
                    range: 3.5,
                    cut_height: 0.5,
                    replacement: String::new(),
                    replace_scale: 1.0,
                    name: "Rocha".into(),
                    stable_key: None,
                },
            ))
            .id();

        app.update();
        app.update();
        let vol = app
            .world()
            .get::<OutlineVolume>(rock)
            .expect("rocha ao alcance deve receber OutlineVolume");
        assert!(vol.visible && vol.width > 0.5);
        assert_eq!(
            app.world().get::<AnimatedOutline>(rock).unwrap().style,
            OutlineStyle::Stone
        );

        // Afasta o herói para fora do alcance: outline faz fade-out e é removido.
        app.world_mut()
            .entity_mut(hero)
            .insert(GlobalTransform::from_xyz(30.0, 0.0, 0.0));
        for _ in 0..12 {
            app.update();
        }
        assert!(
            app.world().get::<OutlineVolume>(rock).is_none(),
            "rocha fora de alcance deve limpar o OutlineVolume após o fade-out"
        );
    }

    #[test]
    fn breakable_and_dropped_pickup_get_distinct_outlines() {
        let mut app = test_app();
        app.world_mut().spawn((
            Player::default(),
            Transform::IDENTITY,
            GlobalTransform::IDENTITY,
        ));
        let barrel = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, 0.0, 2.0),
                GlobalTransform::from_xyz(0.0, 0.0, 2.0),
                Breakable {
                    kind: Kind::Barrel,
                    hits_left: 1,
                },
            ))
            .id();
        let loot = app
            .world_mut()
            .spawn((
                Transform::from_xyz(10.0, 0.0, 0.0),
                GlobalTransform::from_xyz(10.0, 0.0, 0.0),
                PropPickup {
                    kind: PickupKind::Life,
                },
            ))
            .id();

        app.update();
        app.update();
        let barrel_anim = app.world().get::<AnimatedOutline>(barrel).unwrap();
        assert_eq!(barrel_anim.style, OutlineStyle::BreakableCracked);
        assert!(barrel_anim.focused);

        let loot_anim = app.world().get::<AnimatedOutline>(loot).unwrap();
        assert_eq!(loot_anim.style, OutlineStyle::LootLife);
        assert!(app.world().get::<OutlineVolume>(loot).unwrap().visible);
    }

    #[test]
    fn closed_chest_gets_outline_and_empty_chest_does_not() {
        let closed = ScriptInteraction {
            label: "Abrir o baú".into(),
            key: KeyCode::KeyE,
            range: 2.0,
        };
        let empty = ScriptInteraction {
            label: "Vazio".into(),
            key: KeyCode::KeyE,
            range: 2.0,
        };
        let script = LuaScriptRef {
            path: "chest.lua".into(),
        };
        assert_eq!(
            classify_script_collectible(&closed, Some(&script)),
            Some(OutlineStyle::Chest)
        );
        assert_eq!(classify_script_collectible(&empty, Some(&script)), None);
    }

    #[test]
    fn pooled_tree_outlines_only_lod0_wood_and_skips_far_lods_and_leaves() {
        let mut app = test_app();
        app.init_resource::<Assets<StandardMaterial>>();
        let wood_mat = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                alpha_mode: AlphaMode::Opaque,
                ..default()
            });
        let leaf_mat = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                alpha_mode: AlphaMode::Mask(0.5),
                ..default()
            });

        app.world_mut().spawn((
            Player::default(),
            Transform::IDENTITY,
            GlobalTransform::IDENTITY,
        ));
        let tree = app
            .world_mut()
            .spawn((
                Transform::from_xyz(1.5, 0.0, 0.0),
                GlobalTransform::from_xyz(1.5, 0.0, 0.0),
                PooledTree {
                    pool: Handle::default(),
                    variant: 0,
                },
                Harvestable {
                    hits: 3,
                    remaining: 3,
                    style: BreakStyle::Fall,
                    kind: "wood".into(),
                    amount: 2,
                    popup: "Madeira".into(),
                    popup_color: [1.0; 3],
                    range: 3.5,
                    cut_height: 0.5,
                    replacement: String::new(),
                    replace_scale: 1.0,
                    name: "Pinheiro".into(),
                    stable_key: None,
                },
            ))
            .id();
        let lod0_wood = app
            .world_mut()
            .spawn((
                ChildOf(tree),
                Mesh3d::default(),
                MeshMaterial3d(wood_mat.clone()),
                VisibilityRange::abrupt(0.0, 45.0),
            ))
            .id();
        let lod0_leaves = app
            .world_mut()
            .spawn((
                ChildOf(tree),
                Mesh3d::default(),
                MeshMaterial3d(leaf_mat),
                VisibilityRange::abrupt(0.0, 45.0),
            ))
            .id();
        let lod1_wood = app
            .world_mut()
            .spawn((
                ChildOf(tree),
                Mesh3d::default(),
                MeshMaterial3d(wood_mat.clone()),
                VisibilityRange::abrupt(45.0, 110.0),
            ))
            .id();
        let lod2_wood = app
            .world_mut()
            .spawn((
                ChildOf(tree),
                Mesh3d::default(),
                MeshMaterial3d(wood_mat),
                VisibilityRange::abrupt(110.0, 260.0),
            ))
            .id();

        app.update();
        app.update();

        assert!(
            app.world().get::<InheritOutline>(lod0_wood).is_some(),
            "LOD0 opaco da árvore deve herdar outline"
        );
        assert!(
            app.world().get::<InheritOutline>(lod0_leaves).is_none(),
            "folhas alpha-masked da árvore não devem herdar outline"
        );
        assert!(
            app.world().get::<InheritOutline>(lod1_wood).is_none(),
            "LOD1 oculto por VisibilityRange não deve herdar outline"
        );
        assert!(
            app.world().get::<InheritOutline>(lod2_wood).is_none(),
            "LOD2 oculto por VisibilityRange não deve herdar outline"
        );
    }

    #[test]
    fn fade_out_keeps_width_above_one_pixel_until_transparent() {
        let mut volume = OutlineVolume::default();
        for w in [0.15f32, 0.35, 0.65, 1.0] {
            let anim = AnimatedOutline {
                style: OutlineStyle::Stone,
                weight: w,
                focused: false,
                focus_weight: 0.0,
                flash: 0.0,
                tree_wood_only: false,
                phase: 0.0,
            };
            apply_volume(&mut volume, &anim, 0.0);
            if volume.visible {
                assert!(
                    volume.width >= 1.0,
                    "largura ({}) não pode cair abaixo de 1 px durante o fade (w={w})",
                    volume.width
                );
            }
        }
    }

    #[test]
    fn focus_transitions_smoothly_without_abrupt_jump() {
        let mut app = test_app();
        app.world_mut().spawn((
            Player::default(),
            Transform::IDENTITY,
            GlobalTransform::IDENTITY,
        ));
        let rock = app
            .world_mut()
            .spawn((
                Transform::from_xyz(1.2, 0.0, 0.0),
                GlobalTransform::from_xyz(1.2, 0.0, 0.0),
                Harvestable {
                    hits: 3,
                    remaining: 3,
                    style: BreakStyle::Shatter,
                    kind: "stone".into(),
                    amount: 1,
                    popup: "Pedra".into(),
                    popup_color: [1.0; 3],
                    range: 3.5,
                    cut_height: 0.5,
                    replacement: String::new(),
                    replace_scale: 1.0,
                    name: "Rocha".into(),
                    stable_key: None,
                },
            ))
            .id();

        app.update();
        app.update();
        assert_eq!(app.world().get::<AnimatedOutline>(rock).unwrap().focus_weight, 0.0);

        // Ativa foco: focus_weight deve subir gradualmente (não saltar de 0 para 1 num frame).
        app.world_mut().resource_mut::<HarvestFocus>().entity = Some(rock);
        app.update();
        let mid = app.world().get::<AnimatedOutline>(rock).unwrap().focus_weight;
        assert!(
            mid > 0.0 && mid < 1.0,
            "focus_weight deve interpolar suavemente no primeiro frame, obteve {mid}"
        );
        for _ in 0..10 {
            app.update();
        }
        let settled = app.world().get::<AnimatedOutline>(rock).unwrap().focus_weight;
        assert!(settled > 0.99, "focus_weight deve convergir para 1.0, obteve {settled}");
    }
}

//! Spring bones: física de cadeia nos rigs (demo da migração para
//! `bevy_skeleton_modifiers`).
//!
//! Os lobos da floresta têm a pluma da crina pendurada do pescoço
//! (`HeadAccessory1 → 2 → 3 → 3_3 → 3_4`, filhos do `neck_03`). Hoje ela é
//! rígida e acompanha o clip; com uma [`SpringBoneChain`] do crate, a cadeia
//! atrasa e mola com o movimento — vida gratuita nos monstros, sem autoraria
//! no `.bsn`. Os presets dos outros personagens continuam intactos: só rigs
//! com script de lobo entram.
//!
//! Gate de A/B: `VAEL_SPRING_TAILS=0` não spawna as cadeias.

use bevy::prelude::*;
use bevy_ext_core::animation::normalize_clip_name;
use vael_engine::ik::{FootIkRigs, SpringBoneChain};
use vael_engine::luau::components::LuaScriptRef;

/// Prefixo normalizado do osso raiz da pluma (cadeia de crina da pipeline de
/// rigging — o mesmo nome em todos os quadrúpedes gerados).
const MANE_PREFIX: &str = "headaccessory";

/// Marca um rig com a cadeia já spawnada (evitar duplicar por frame). Guarda
/// a entidade da cadeia para o rebind poder matá-la: `FootIkRigs` sai e volta
/// (bind de tier/LOD) sem a subárvore morrer, e sem isto ficavam DUAS cadeias
/// de mola no mesmo osso (física a dobrar).
#[derive(Component)]
struct SpringBound {
    chain: bevy::ecs::entity::Entity,
}

pub struct VaelSpringPlugin;

impl Plugin for VaelSpringPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, spring_bind_system);
    }
}

fn springs_enabled() -> bool {
    std::env::var("VAEL_SPRING_TAILS").ok().as_deref() != Some("0")
}

/// Acha a cadeia da crina: o osso mais alto com o prefixo e o descendente
/// mais fundo pelo mesmo prefixo (caminho do primeiro filho que casa a cada
/// nível — na prática é 1:1, `HeadAccessory1→2→3→3_3→3_4`).
fn find_mane_chain(
    root: Entity,
    children: &Query<&Children>,
    names: &Query<&Name>,
) -> Option<(Entity, Entity)> {
    let mut all = Vec::new();
    walk(root, children, names, &mut all);
    let matches: Vec<Entity> = all
        .into_iter()
        .filter(|(_, n)| normalize_clip_name(n).starts_with(MANE_PREFIX))
        .map(|(e, _)| e)
        .collect();
    let chain_root = *matches.first()?;
    // Descer do raiz pelo primeiro filho que casa, até não haver mais.
    let mut end = chain_root;
    for _ in 0..32 {
        let next = children.get(end).ok()?.iter().find(|c| {
            names
                .get(*c)
                .map(|n| normalize_clip_name(n.as_str()).starts_with(MANE_PREFIX))
                .unwrap_or(false)
        });
        match next {
            Some(child) => end = child,
            None => break,
        }
    }
    (end != chain_root).then_some((chain_root, end))
}

fn walk(
    root: Entity,
    children: &Query<&Children>,
    names: &Query<&Name>,
    out: &mut Vec<(Entity, String)>,
) {
    let Ok(kids) = children.get(root) else {
        return;
    };
    for child in kids.iter() {
        if let Ok(name) = names.get(child) {
            out.push((child, name.as_str().to_string()));
        }
        walk(child, children, names, out);
    }
}

/// Spawna as cadeias de mola nos lobos (script contém "wolf") com rig já
/// ligado ao IK. Os donos das cadeias são filhos da raiz do rig — morrem com
/// a subárvore no rebind; o `SpringBound` do velho rig é limpo quando o
/// `FootIkRigs` desaparece, para o novo voltar a ligar.
fn spring_bind_system(
    mut commands: Commands,
    rigs: Query<(Entity, &FootIkRigs, &LuaScriptRef), Without<SpringBound>>,
    stale: Query<(Entity, &SpringBound), Without<FootIkRigs>>,
    names: Query<&Name>,
    children: Query<&Children>,
) {
    if !springs_enabled() {
        return;
    }
    for (entity, bound) in &stale {
        // Rebind de tier: as cadeias velhas morreram com a subárvore (ou não —
        // matar na mesma é um no-op); voltar ao pendente para o novo rig ligar.
        commands.entity(bound.chain).try_despawn();
        commands.entity(entity).remove::<SpringBound>();
    }
    for (entity, rig, script) in &rigs {
        if !script.path.contains("wolf") {
            continue;
        }
        // A cadeia primeiro, a marca depois: com a marca posta antes de a
        // encontrar, um rig cujos ossos ainda não instanciaram (ou cujo nome
        // de osso não casa) ficava marcado sem mola para sempre.
        let Some((root, end)) = find_mane_chain(rig.root, &children, &names) else {
            continue;
        };
        let chain = commands
            .spawn((
                Name::new("vael-spring-mane"),
                SpringBoneChain::new(root, end),
                ChildOf(rig.root),
            ))
            .id();
        commands.entity(entity).insert(SpringBound { chain });
        // Defaults do Godot 4.7.2; sem colliders na primeira passada. Dono
        // filho da raiz do rig (ciclo de vida da subárvore).
        debug!("vael: spring da crina no lobo {entity:?} ({root:?}→{end:?})");
    }
}

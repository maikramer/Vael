//! Ponte entre o host BMS ([`bevy_ext_luau`]) e o monólito.
//!
//! Duas metades, espelhos uma da outra:
//!
//! - **comandos**: [`apply_bms_commands`] (Update, depois do dreno do BMS)
//!   converte os `ScriptCommand` do `bevy_ext_luau` para os do monólito e
//!   aplica-os com [`apply_script_command`] — o MESMO caminho que o
//!   `luau_update` usa para os seus próprios comandos;
//! - **estado**: [`publish_host_state`] (PreUpdate, antes do publisher do
//!   BMS) preenche o [`bevy_ext_luau::ScriptHostState`] com o snapshot do
//!   frame extraído do `luau_update` (posições/nomes, HP, props, hostis,
//!   prototypes, superfícies, câmara…).
//!
//! O [`apply_script_command`] é a EXTRAÇÃO dos braços que viviam inline no
//! match de `commands.queue` do `luau_update` (behavior 1:1): os dois
//! consumidores partilham os mesmos recursos/writers (toasts, SFX,
//! `ProjectileQueue`, `TerrainEditQueue`, `UiPrompt`, marcadores do HUD,
//! `SaveRequest`…), e o mapa-sombra de HP criado por `Commands` + os avisos
//! 1× vivem agora no [`ScriptWarnState`] (recurso, partilhado pelos dois
//! caminhos).

use std::collections::{HashMap, HashSet};

use bevy::animation::graph::AnimationNodeIndex;
use bevy::animation::transition::AnimationTransitions;
use bevy::animation::AnimationPlayer;
use bevy::prelude::*;

use bevy_ext_luau::{
    CallbackLabel, IntoCallbackLabel, LuaScriptingPlugin, ScriptAsset, ScriptCallbackMessage,
    ScriptComponent, ScriptValue, event_handler,
};

use crate::player::Player;
use crate::vitals::Health;

use super::commands::ScriptCommand;
use super::components::{LuaScriptRef, ScriptInteraction, ScriptProps, ScriptToast};
use super::input::key_code_from_str;
use super::sfx::match_gesture_clip;

/// Estado por-frame partilhado pelo `luau_update` e pela ponte BMS:
/// - `once`: avisos 1× (plugin ausente, fila cheia, entidade sem vitais,
///   gesto sem clip…) — as chaves são as mesmas que os `Local`s antigos do
///   `luau_update` usavam (com prefixo para as keyed por entidade);
/// - `fresh_health`: sombra do HP criado por `Commands` NESTE frame — os
///   comandos seguintes do MESMO dreno (set_max_hp seguido de damage) leem
///   daqui; o `luau_update` limpa-o ao início de cada dreno (era o
///   `locals.fresh_health.clear()`).
#[derive(Resource, Default)]
pub struct ScriptWarnState {
    pub(crate) once: HashSet<String>,
    pub(crate) fresh_health: HashMap<Entity, (f32, f32)>,
}

/// Acesso ao estado de aviso/sombra, criando-o se faltar (apps mínimas que
/// montam `apply_script_command` sem nenhum plugin).
fn warn_state(world: &mut World) -> Mut<'_, ScriptWarnState> {
    if !world.contains_resource::<ScriptWarnState>() {
        world.init_resource::<ScriptWarnState>();
    }
    world.resource_mut::<ScriptWarnState>()
}

/// Aviso 1× com chave arbitrária (o formato das chaves é interno).
fn warn_once(world: &mut World, key: &str, message: &str) {
    let fresh = warn_state(world).once.insert(key.to_string());
    if fresh {
        warn!(target: "vael::luau", "{message}");
    }
}

/// A entidade do herói (o `players.single()` do `luau_update`).
fn player_of(world: &mut World) -> Option<Entity> {
    let mut query = world.query_filtered::<Entity, With<Player>>();
    query.iter(world).next()
}

/// Posição de MUNDO de uma entidade de script — o mesmo par de leituras do
/// `scripts` do `luau_update` (`GlobalTransform` preferido ao `Transform`
/// local; ver o comentário do repro da cidade). Exclui o herói, como a
/// query `Without<Player>` fazia: `move_towards`/`face_towards` no herói
/// continuam a ser ignorados pela engine. DIFERENÇA assumida: o `scripts`
/// do `luau_update` exigia `LuaScriptRef` — os comandos da ponte BMS
/// podem mirar entidades sem script (attach BMS não usa o componente),
/// e o comportamento dos comandos do monólito não muda (enfileirados só
/// por entidades com script).
fn scripted_pos(world: &World, entity: Entity) -> Option<Vec3> {
    if world.get::<Player>(entity).is_some() {
        return None;
    }
    world
        .get::<GlobalTransform>(entity)
        .map(|global| global.translation())
        .or_else(|| world.get::<Transform>(entity).map(|t| t.translation))
}

/// Locomoção de script: `drive_to`/`drive`/`look_at` na entidade, criando o
/// `AiLocomotion` + `LocomotionProfile` quando faltam (mesma regra do braço
/// do `MoveBy`: o perfil aprende a velocidade comandada no mesmo gesto).
enum Drive {
    /// Marcha para um ponto (`MoveTowards`): (posição actual, alvo, velocidade).
    To(Vec2, Vec2, f32),
    /// Velocidade desejada (`MoveBy`).
    By(Vec2),
    /// Pedido de olhar (`FaceTowards`).
    Look(Vec2),
}

fn drive_locomotion(world: &mut World, entity: Entity, drive: Drive, dt: f32) {
    // Velocidade a que o perfil de locomoção aprende (`Look` não observa:
    // o braço do `FaceTowards` também não tocava no perfil).
    let observed = match drive {
        Drive::To(_, _, speed) => Some(speed),
        Drive::By(velocity) => Some(velocity.length()),
        Drive::Look(_) => None,
    };
    let has_loco = world.get::<crate::ai::AiLocomotion>(entity).is_some();
    if has_loco {
        if let Some(mut loco) = world.get_mut::<crate::ai::AiLocomotion>(entity) {
            match drive {
                Drive::To(here, goal, speed) => loco.drive_to(here, goal, speed, dt),
                Drive::By(velocity) => loco.drive(velocity),
                Drive::Look(dir) => loco.look_at(dir),
            }
        }
        if let Some(speed) = observed
            && let Some(mut profile) =
                world.get_mut::<crate::animation::LocomotionProfile>(entity)
        {
            profile.observe(speed);
        }
    } else {
        let mut loco = crate::ai::AiLocomotion::default();
        match drive {
            Drive::To(here, goal, speed) => loco.drive_to(here, goal, speed, dt),
            Drive::By(velocity) => loco.drive(velocity),
            Drive::Look(dir) => loco.look_at(dir),
        }
        match observed {
            Some(speed) => {
                let mut profile = crate::animation::LocomotionProfile::default();
                profile.observe(speed);
                world.commands().entity(entity).try_insert((loco, profile));
            }
            None => {
                world.commands().entity(entity).try_insert(loco);
            }
        }
    }
}

/// One-shot com playback speed sobre `&mut World` — corpo 1:1 do
/// `bevy_ext_engine::animation::play_action_scaled` (o helper espera um
/// `Query`, inacessível fora de sistemas). `duration_of` vem pré-lido para
/// não reabrir o componente depois do borrow do player.
fn play_action_scaled_world(
    world: &mut World,
    entity: Entity,
    player_entity: Entity,
    locked: bool,
    duration: f32,
    node: AnimationNodeIndex,
    blend: std::time::Duration,
    terminal: bool,
    speed: f32,
) -> bool {
    if locked {
        return false;
    }
    let speed = if speed.is_finite() && speed > 0.05 {
        speed
    } else {
        1.0
    };
    {
        let mut players = world.query::<(
            &mut AnimationPlayer,
            &mut AnimationTransitions,
        )>();
        let Ok((mut player, mut transitions)) = players.get_mut(world, player_entity)
        else {
            return false;
        };
        transitions
            .play(&mut player, node, blend)
            .set_repeat(bevy::animation::RepeatAnimation::Never)
            .set_speed(speed);
    }
    if let Some(mut animator) = world.get_mut::<crate::animation::CharacterAnimator>(
        entity,
    ) {
        animator.current = Some(node);
        animator.locked = terminal;
        animator.action_time = if terminal {
            f32::INFINITY
        } else {
            // A clip com duração não mensurável trancava o rig para sempre;
            // cai num beat curto para o driver recuperar o rig sempre.
            let d = if duration > 1e-3 { duration } else { 0.25 };
            d / speed
        };
    }
    true
}

/// Destrutível com queda (`Topple`): física genérica; a direção sai do
/// herói ao alvo. Corpo 1:1 do closure que o braço do `Topple` enfileirava —
/// com `&mut World` direto já não precisa de comando diferido.
fn topple_entity(world: &mut World, entity: Entity, player_pos: Option<Vec3>) {
    let Some(player) = player_pos else {
        return;
    };
    let Some(global) = world.get::<GlobalTransform>(entity) else {
        return;
    };
    let target = global.translation();
    let dir = (target - player).normalize_or_zero();
    let initial = world
        .get::<Transform>(entity)
        .map(|t| t.rotation)
        .unwrap_or_default();
    let Ok(mut prop) = world.get_entity_mut(entity) else {
        return;
    };
    prop.insert(crate::physics_fx::Falling {
        axis: Vec3::new(dir.z, 0.0, -dir.x),
        timer: 0.0,
        initial,
    });
    prop.remove::<LuaScriptRef>();
}

/// Cura: um `amount` negativo não tira vida (isso é `entity_damage`) e o
/// resultado nunca sai de `[0, max]`. Sem o chão, `current + (-grande)`
/// ficava negativo e a entidade deixava de poder morrer.
fn apply_heal(current: f32, max: f32, amount: f32) -> f32 {
    (current + amount.max(0.0)).clamp(0.0, max.max(0.0))
}

/// Fallback de `viber.set_visible` quando a reflexão não tem `Visibility`
/// (app mínima de teste sem `ReflectPlugin`): o comportamento antigo de
/// inserir o componente. `Inherited` (e não `Visible`) para a entidade voltar
/// a respeitar o pai escondido quando é uma subárvore.
fn insert_visibility(world: &mut World, entity: Entity, visible: bool) {
    if let Ok(mut entity_mut) = world.get_entity_mut(entity) {
        entity_mut.insert(if visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
}

/// Balão do HUD (`viber.say` sem presenter externo) sobre `&mut World` —
/// corpo 1:1 do `crate::dialogue::show_balloon` (o helper espera `Query`s),
/// com as escritas de texto faseadas para respeitar os borrows.
fn show_balloon_world(world: &mut World, body: &str, secs: f32) -> bool {
    let mut found = false;
    let mut children: Vec<Entity> = Vec::new();
    {
        let mut balloons = world.query::<(
            &mut Visibility,
            &mut crate::hud::HudBalloon,
            &bevy::ecs::hierarchy::Children,
        )>();
        for (mut visibility, mut balloon, kids) in balloons.iter_mut(world) {
            found = true;
            balloon.timer = secs;
            *visibility = Visibility::Visible;
            if let Some(child) = kids.first() {
                children.push(*child);
            }
        }
    }
    let mut texts = world.query::<&mut Text>();
    for child in children {
        if let Ok(mut text) = texts.get_mut(world, child) {
            text.0 = body.into();
        }
    }
    found
}

/// Aplica UM comando enfileirado por script, com `&mut World`.
///
/// EXTRAÇÃO 1:1 dos braços que viviam inline no match de commands.queue do
/// `luau_update` (ver git): os consumidores são o próprio `luau_update` (via
/// `commands.queue`) e a ponte BMS ([`apply_bms_commands`]). Os avisos 1× e
/// o mapa-sombra de HP vivem em [`ScriptWarnState`] no World.
pub(crate) fn apply_script_command(world: &mut World, command: ScriptCommand) {
    let dt = world.resource::<Time>().delta_secs();
    let elapsed = world.resource::<Time>().elapsed_secs_f64();
    match command {
        ScriptCommand::MoveTowards { entity, goal, speed } => {
            // A posição actual vem do MUNDO (o alvo é mundo); sem ela não
            // há direcção nem chegada.
            let Some(here) = scripted_pos(world, entity) else {
                return;
            };
            let here = Vec2::new(here.x, here.z);
            drive_locomotion(world, entity, Drive::To(here, goal, speed), dt);
        }
        ScriptCommand::MoveBy(entity, velocity) => {
            // A velocidade desejada vai para o [`crate::ai::AiLocomotion`];
            // o passo, a rampa de aceleração, o snap de Y e a viragem são
            // do consumidor único (`apply_ai_locomotion`, PostUpdate).
            // Entidades ainda sem o componente recebem-no aqui — o perfil
            // de locomoção aprende a velocidade comandada no mesmo gesto,
            // que é o que faz os 17 scripts do exemplo acertarem o clip e
            // a cadência sem uma linha de autoria.
            drive_locomotion(world, entity, Drive::By(velocity), dt);
        }
        ScriptCommand::FaceTowards(entity, target) => {
            // Direção no MUNDO (o alvo é o herói, em mundo).
            let Some(here) = scripted_pos(world, entity) else {
                return;
            };
            let dir = Vec2::new(target.x - here.x, target.z - here.z);
            // Pedido de olhar, não ordem: `apply_ai_locomotion` só o honra
            // com a entidade praticamente parada. A andar, o corpo aponta
            // para onde VAI — é esta regra que mata o "andar de lado" dos
            // scripts que chamam `face_player()` e `move_towards()` no
            // mesmo frame.
            drive_locomotion(world, entity, Drive::Look(dir), dt);
        }
        ScriptCommand::SetLocomotion { entity, walk, run, turn_rate } => {
            let mut profile = crate::animation::LocomotionProfile::authored(walk, run);
            if let Some(turn_rate) = turn_rate {
                profile.turn_rate = turn_rate;
            }
            let has_loco = world.get::<crate::ai::AiLocomotion>(entity).is_some();
            let has_profile = world.get::<crate::animation::LocomotionProfile>(entity).is_some();
            if has_loco && has_profile {
                if let Some(mut existing) = world.get_mut::<crate::animation::LocomotionProfile>(entity) {
                    *existing = profile;
                }
            } else if has_loco {
                world
                    .commands()
                    .entity(entity)
                    .try_insert(profile);
            } else {
                world
                    .commands()
                    .entity(entity)
                    .try_insert((crate::ai::AiLocomotion::default(), profile));
            }
        }
        ScriptCommand::ChangeScene { scene, entry, via } => {
            let request = crate::scene_loader::ChangeScene {
                scene,
                arrival: crate::scene_loader::Arrival::from_entry(entry.as_deref()),
                via_portal: via,
                toast: None,
                sound: None,
                came_from: None,
                return_pose: None,
            };
            if let Err(error) = crate::scene_loader::request_change(world, request) {
                warn!(target: "vael::luau", "viber.change_scene: {error:#}");
            }
        }
        ScriptCommand::TeleportPlayer { pos, exclude } => {
            let Some(entity) = player_of(world) else {
                return;
            };
            // ASSENTA no destino em vez de largar o herói no ar: as
            // colunas do destino podem estar por assar (vinha de uma
            // bolsa de interior a 2,6 km) e, sem collider, o herói
            // caía pelo mundo fora antes de o streaming o apanhar.
            //
            // O Y vem de um RAIO DE FÍSICA (props/plataformas/pontes)
            // com o terreno analítico como plano B — o braço original
            // diferia num comando porque os SystemParams não expõem o
            // `&mut World` do Rapier; esta extração tem-no.
            //
            // Exclui o herói E o prop pedido (marco): o raio não
            // pode pousá-lo por cima do próprio destino.
            let mut skip = vec![entity];
            if let Some(prop) = exclude {
                skip.push(prop);
            }
            let landed = crate::player::teleport_landing(world, pos, &skip);
            if let Some(mut transform) = world.get_mut::<Transform>(entity) {
                transform.translation = landed;
            }
            if let Some(mut player) = world.get_mut::<Player>(entity) {
                player.vel_x = 0.0;
                player.vel_y = 0.0;
                player.vel_z = 0.0;
            }
            if let Ok(mut hero) = world.get_entity_mut(entity) {
                hero.remove::<crate::physics_fx::Knockback>();
                hero.insert(crate::player::TeleportSettle::default());
            }
        }
        ScriptCommand::PushPlayer { velocity } => {
            let Some(entity) = player_of(world) else {
                return;
            };
            world
                .commands()
                .entity(entity)
                .try_insert(crate::physics_fx::Knockback { velocity });
        }
        ScriptCommand::FireProjectile { template, origin, target, shooter } => {
            let Some(mut queue) = world.get_resource_mut::<crate::projectile::ProjectileQueue>()
            else {
                warn_once(
                    world,
                    "fire_projectile",
                    "viber.fire_projectile sem ProjectilePlugin — pedido ignorado",
                );
                return;
            };
            queue.requests.push(crate::projectile::ProjectileRequest {
                template,
                origin,
                target,
                shooter,
            });
        }
        ScriptCommand::HealPlayer(amount) => {
            let Some(entity) = player_of(world) else {
                return;
            };
            if let Some(mut health) = world.get_mut::<Health>(entity) {
                // O mesmo contrato de `entity_heal`: negativo não tira vida
                // e um NaN do produtor não envenena o HP para sempre.
                health.current = apply_heal(health.current, health.max, amount);
            }
        }
        ScriptCommand::Toast(msg) => {
            let msg = world
                .get_resource::<crate::locale::Locale>()
                .map(|locale| crate::dialogue::localized_lines(&msg, Some(&locale.table)))
                .unwrap_or(msg);
            info!(target: "vael::luau", "[toast] {msg}");
            world.write_message(ScriptToast(msg));
        }
        ScriptCommand::SetInteraction { entity, label, key, range } => {
            let Some(code) = key_code_from_str(&key) else {
                return;
            };
            // Label vazio = sem interação (o script quer esconder o
            // prompt). Sem o remover a entidade continuava a GANHAR o
            // foco da tecla (distância 0, sem prompt nenhum) e calava
            // o NPC real ao lado.
            if label.trim().is_empty() {
                world.commands().entity(entity).try_remove::<ScriptInteraction>();
            } else {
                world.commands().entity(entity).try_insert(ScriptInteraction {
                    label,
                    key: code,
                    // ÚNICO sítio onde o alcance autorado vira alcance
                    // efetivo: daqui para a frente (prompt, foco,
                    // `interacted`, colheita) todos leem o mesmo número.
                    range: crate::interact::scaled_range(range),
                });
            }
        }
        ScriptCommand::Despawn(entity) => {
            world.commands().entity(entity).try_despawn();
        }
        ScriptCommand::Gesture { entity, name, speed, lock, blend, duration: target_secs } => {
            // Leitura prévia do rig (o borrow do `Mut` não pode coexistir
            // com o do `AnimationPlayer`).
            let prep = world.get::<crate::animation::CharacterAnimator>(entity).map(
                |animator| {
                    (
                        animator.player,
                        animator.locked,
                        match_gesture_clip(&animator, &name)
                            .and_then(|index| animator.nodes.get(index).copied()),
                    )
                },
            );
            let Some((player_entity, locked, node)) = prep else {
                // Sem rig da engine: o jogo pode ter outro (estados de
                // animação autorados no Jackdaw) — fica com o pedido.
                world.commands().trigger(super::runtime::UnboundGesture {
                    entity,
                    name: name.clone(),
                    speed,
                });
                // Rig ainda a carregar: o gesto espera por ele (e é
                // largado em silêncio se não chegar — sem aviso por
                // NPC a cada arranque).
                world.commands().entity(entity).try_insert(
                    crate::animation::PendingGesture {
                        name,
                        speed,
                        lock,
                        blend,
                        age: 0.0,
                        duration: target_secs,
                    },
                );
                return;
            };
            let Some(node) = node else {
                let clip_names = world
                    .get::<crate::animation::CharacterAnimator>(entity)
                    .map(|animator| animator.clip_names.clone());
                if let Some(clip_names) = clip_names
                    && warn_state(world)
                        .once
                        .insert(format!("gesture:{}", entity.to_bits()))
                {
                    warn!(target: "vael::luau",
                        "viber.gesture('{name}'): nenhum clip do rig corresponde (clips: {clip_names:?})");
                }
                return;
            };
            if locked {
                return;
            }
            let duration = world
                .get::<crate::animation::CharacterAnimator>(entity)
                .map(|animator| animator.duration_of(node))
                .unwrap_or_default();
            // Blend de gesto ~250 ms como o npc_gesture_system; one-shot
            // devolve o rig ao driver no fim do clip. `speed` escala a
            // reprodução (`viber.play_clip`; 1.0 = nominal).
            let blend = blend
                .map(std::time::Duration::from_secs_f32)
                .unwrap_or(std::time::Duration::from_millis(250));
            let speed = crate::animation::gesture_speed(speed, duration, target_secs);
            play_action_scaled_world(
                world,
                entity,
                player_entity,
                false,
                duration,
                node,
                blend,
                lock,
                speed,
            );
        }
        ScriptCommand::TerrainEdit { edit } => {
            let Some(mut queue) = world.get_resource_mut::<crate::terrain::delta::TerrainEditQueue>()
            else {
                // Apps mínimas sem o plugin de terreno: warn 1× (a fila
                // nem existe — o pedido não pode ser aceite em silêncio).
                warn_once(
                    world,
                    "terrain_edit",
                    "viber.terrain.* sem TerrainEditQueue registada — pedido ignorado",
                );
                return;
            };
            if !queue.push(edit) {
                warn_once(
                    world,
                    "terrain_edit_full",
                    &format!(
                        "viber.terrain.*: fila de edições cheia ({} pendentes) — pedidos \
                         descartados até a fila escoar",
                        crate::terrain::delta::EDIT_QUEUE_CAP
                    ),
                );
            }
        }
        ScriptCommand::PlaySfx { clip, position } => {
            world.write_message(crate::ambient::SfxEvent { clip, position });
        }
        ScriptCommand::Topple { entity } => {
            let player_pos = player_of(world).and_then(|p| {
                world.get::<GlobalTransform>(p).map(|g| g.translation())
            });
            topple_entity(world, entity, player_pos);
        }
        ScriptCommand::PlayCinematic { id, anchor } => {
            world.write_message(crate::cinematic::PlayCinematic {
                id,
                anchor,
                inline: None,
            });
        }
        ScriptCommand::SetScale { entity, scale } => {
            // TODO(migration): o nome Lua mantém-se; a escrita passa pela
            // reflexão (`viber.ecs`), que já não depende de a entidade
            // estar na query `scripts`/`poses` — funciona em props, NPCs
            // sem script e no próprio herói.
            let patch = super::ecs::EcsValue::Map(vec![(
                "scale".to_string(),
                super::ecs::EcsValue::Vec3([scale, scale, scale]),
            )]);
            if let Err(error) =
                super::ecs::patch_component(world, entity, super::ecs::TRANSFORM_PATH, &patch)
            {
                // Sem `ReflectPlugin`/`Transform` no registo (app
                // mínima): mantém a escrita directa de sempre.
                warn!(target: "vael::luau", "viber.set_scale: {error}");
                if let Some(mut transform) = world.get_mut::<Transform>(entity) {
                    transform.scale = Vec3::splat(scale);
                }
            }
        }
        ScriptCommand::EntitySetMaxHp { entity, max } => {
            // O herói NÃO entra no dreno de `entity_*` (query
            // `Without<Player>`): o ramo de criação substituía o `Health`
            // inteiro e um level-up curava-o em vez de só subir o teto.
            if let Some(mut health) = world.get_mut::<Health>(entity) {
                health.max = max;
                health.current = health.current.min(max);
            } else {
                // Criação: também grava no mapa-sombra (comandos do
                // mesmo frame não vêem a query).
                world
                    .commands()
                    .entity(entity)
                    .try_insert(Health { current: max, max });
                warn_state(world).fresh_health.insert(entity, (max, max));
            }
        }
        ScriptCommand::EntityDamage { entity, amount } => {
            // Helper local: emitir Kill na TRANSIÇÃO vivo→morto. A ENGINE
            // NÃO decide o que a morte significa: sem cadáver, sem XP, sem
            // remover o script — o jogo ouve o evento e trata da morte, do
            // loot e das quests. Bater num cadáver não mata outra vez.
            // O nome segue o contrato do melee (`script_kind` do path); o
            // herói (fora da query do monólito) fica "creature".
            let kind = if world.get::<Player>(entity).is_some() {
                "creature".to_string()
            } else {
                world
                    .get::<LuaScriptRef>(entity)
                    .map(|lref| crate::script_kind::script_kind(&lref.path))
                    .unwrap_or_else(|| "creature".to_string())
            };
            let player_hit = player_of(world) == Some(entity);
            let mut killed: Option<i64> = None;
            let mut damaged = false;
            if let Some(mut health) = world.get_mut::<Health>(entity) {
                let before = health.current;
                crate::vitals::apply_damage(&mut health, amount);
                if before > 0.0 && health.current <= 0.0 && !player_hit {
                    killed = Some(entity.to_bits() as i64);
                }
                damaged = true;
            }
            if !damaged {
                // Sem `Health` no mundo: a sombra do HP criado por `Commands`
                // neste frame (o padrão set_max_hp → damage do MESMO dreno).
                let next = {
                    let mut warn = warn_state(world);
                    warn.fresh_health.get_mut(&entity).map(|(cur, max)| {
                        let before = *cur;
                        let next = (*cur - amount).clamp(0.0, (*max).max(0.0));
                        *cur = next;
                        if before > 0.0 && next <= 0.0 && !player_hit {
                            killed = Some(entity.to_bits() as i64);
                        }
                        (next, *max)
                    })
                };
                if let Some((next, max)) = next {
                    world
                        .commands()
                        .entity(entity)
                        .try_insert(Health { current: next, max });
                } else {
                    let bits = entity.to_bits();
                    let fresh = warn_state(world).once.insert(format!("vitals:{bits}"));
                    if fresh {
                        warn!(target: "vael::luau",
                            "viber.entity_damage: entidade sem Health — chama viber.entity_set_max_hp primeiro");
                    }
                }
            }
            if let Some(bits) = killed {
                if let Some(mut events) = world.get_resource_mut::<super::events::ScriptEventQueue>()
                {
                    events.push(super::events::ScriptGameEvent::Kill { name: kind, entity: bits });
                }
            }
        }
        ScriptCommand::EntityHeal { entity, amount } => {
            if let Some(mut health) = world.get_mut::<Health>(entity) {
                health.current = apply_heal(health.current, health.max, amount);
            } else {
                let healed = {
                    let mut warn = warn_state(world);
                    warn.fresh_health.get_mut(&entity).map(|(cur, max)| {
                        let next = apply_heal(*cur, *max, amount);
                        *cur = next;
                        (next, *max)
                    })
                };
                if let Some((next, max)) = healed {
                    world
                        .commands()
                        .entity(entity)
                        .try_insert(Health { current: next, max });
                }
            }
        }
        ScriptCommand::EntityKnockback { entity, dir, strength } => {
            if strength > 0.0 && dir.length_squared() > 1e-6 {
                world
                    .commands()
                    .entity(entity)
                    .try_insert(crate::physics_fx::knockback_after(dir, strength));
            }
        }
        ScriptCommand::HitFlash { entity, secs } => {
            world
                .commands()
                .entity(entity)
                .try_insert(crate::feedback::HitFlash { timer: secs });
        }
        ScriptCommand::Prompt { key, label } => {
            if let Some(mut prompt) = world.get_resource_mut::<crate::ui::UiPrompt>() {
                if label.trim().is_empty() {
                    prompt.clear();
                } else {
                    prompt.set_script(key, label);
                }
            }
        }
        ScriptCommand::SetPlayerSpeed(mult) => {
            let mut query = world.query_filtered::<&mut crate::player::Player, With<Player>>();
            for mut player in query.iter_mut(world) {
                player.speed = crate::player::BASE_SPEED * mult;
            }
        }
        ScriptCommand::SetLantern(intensity) => {
            if let Some(mut light) = world.get_resource_mut::<crate::ambient::TorchLightOverride>()
            {
                light.intensity = if intensity.is_finite() {
                    intensity.max(0.0)
                } else {
                    0.0
                };
            }
        }
        ScriptCommand::SetBoneOffset {
            entity,
            bone,
            rotation,
        } => {
            // Sem entidade (ponte BMS do editor): o herói, como sempre.
            let target = entity.or_else(|| {
                let mut query =
                    world.query_filtered::<Entity, With<crate::player::Player>>();
                query.iter(world).next()
            });
            let Some(target) = target else {
                return;
            };
            let Ok(mut target) = world.get_entity_mut(target) else {
                return;
            };
            if rotation == Quat::IDENTITY {
                if let Some(mut offsets) = target.get_mut::<crate::ik::BoneOffsets>() {
                    offsets.clear(&bone);
                }
            } else if let Some(mut offsets) = target.get_mut::<crate::ik::BoneOffsets>() {
                offsets.set(&bone, rotation);
            } else {
                let mut offsets = crate::ik::BoneOffsets::default();
                offsets.set(&bone, rotation);
                target.insert(offsets);
            }
        }
        ScriptCommand::Corpse { entity } => {
            // Só a marca: quem tem `RagdollDeath` (attr `ragdoll`) entra
            // em ragdoll; o despawn e o loot são do jogo.
            world.commands().entity(entity).try_insert(crate::corpse::Corpse {
                timer: crate::corpse::CORPSE_LIFETIME,
            });
        }
        ScriptCommand::Attach { owner, slot, url, bone, pos, rot, scale } => {
            let Some(mut queue) = world.get_resource_mut::<crate::attachment::AttachmentRequests>()
            else {
                warn_once(
                    world,
                    "attach",
                    "viber.attach sem AttachmentPlugin — pedido ignorado",
                );
                return;
            };
            queue.0.push(crate::attachment::AttachmentRequest {
                owner,
                slot,
                url,
                bone,
                pos,
                rot,
                scale,
            });
        }
        ScriptCommand::A11ySet { subtitle_scale, shake, hitstop, interact_hold } => {
            if let Some(mut a11y) = world.get_resource_mut::<crate::a11y::A11y>() {
                if let Some(scale) = subtitle_scale {
                    a11y.subtitle_scale = scale.clamp(0.5, 3.0);
                }
                if let Some(value) = shake {
                    a11y.shake = value;
                }
                if let Some(value) = hitstop {
                    a11y.hitstop = value;
                }
                if let Some(value) = interact_hold {
                    a11y.interact_hold = value;
                }
            }
        }
        ScriptCommand::OwnUiAction(name) => {
            if let Some(mut owners) = world.get_resource_mut::<crate::ui::actions::UiActionOwners>()
            {
                owners.0.insert(name);
            }
        }
        ScriptCommand::RadialDamage { x, z, radius, damage, knockback } => {
            // Falloff linear (skills::radial_damage) + knockback radial
            // (physics_fx) + evento Kill. Cadáver, XP e quests são do
            // JOGO: a engine só aplica o dano e conta a morte.
            apply_radial_damage(world, x, z, radius, damage, knockback, None);
        }
        ScriptCommand::Burst { preset, pos, count } => {
            if preset == "sparks" {
                let spec = crate::fx_specs::hit_sparks_spec();
                let mut commands = world.commands();
                crate::particles::spawn_burst(&mut commands, &spec, pos, count.clamp(1, 256));
            } else {
                let spec = crate::recipes::ParticleSpec {
                    preset,
                    emission_rate: None,
                    life: None,
                    speed: None,
                    size: None,
                    color: None,
                    shape_radius: None,
                    looping: false,
                    world_space: false,
                };
                let mut commands = world.commands();
                crate::particles::spawn_burst(&mut commands, &spec, pos, count.clamp(1, 256));
            }
        }
        ScriptCommand::Ring { x, z, radius, color } => {
            let y = world
                .get_resource::<crate::terrain::runtime::TerrainRuntime>()
                .map(|rt| rt.sample_mesh_surface(x, z))
                .unwrap_or(0.0);
            let color = color
                .map(|[r, g, b]| Color::srgb(r, g, b))
                .unwrap_or(Color::srgba(0.85, 0.9, 1.0, 0.9));
            if !world.contains_resource::<Assets<Mesh>>()
                || !world.contains_resource::<Assets<StandardMaterial>>()
            {
                return;
            }
            // O helper original espera `Commands` + dois `ResMut`
            // simultâneos — o `resource_scope` dá o mesmo trio a partir de
            // `&mut World` sem duplicar o corpo.
            world.resource_scope::<Assets<Mesh>, _>(|world, mut meshes| {
                world.resource_scope::<Assets<StandardMaterial>, _>(
                    |world, mut materials| {
                        let mut commands = world.commands();
                        crate::impact::spawn_impact_ring(
                            &mut commands,
                            &mut meshes,
                            &mut materials,
                            Vec3::new(x, y + 0.08, z),
                            radius,
                            color,
                        );
                    },
                );
            });
        }
        ScriptCommand::Shake(amount) => {
            if let Some(mut shake) = world.get_resource_mut::<crate::camera::CameraShake>() {
                crate::camera::add_camera_shake(&mut shake, amount);
            }
        }
        ScriptCommand::CameraKick(impulse) => {
            if let Some(mut kick) = world.get_resource_mut::<crate::camera::CameraKick>() {
                crate::camera::add_camera_kick(&mut kick, impulse);
            }
        }
        ScriptCommand::FovKick(deg) => {
            if let Some(mut fx) = world.get_resource_mut::<crate::camera::CameraFx>() {
                crate::camera::fov_kick(&mut fx, deg);
            }
        }
        ScriptCommand::Punch { stops, bloom } => {
            if let Some(mut postfx) = world.get_resource_mut::<crate::postfx::PostFxState>() {
                crate::postfx::punch_impact(&mut postfx, stops, bloom);
            }
        }
        ScriptCommand::HitStop(secs) => {
            if let Some(mut stop) = world.get_resource_mut::<crate::time_fx::HitStop>() {
                crate::time_fx::request_hit_stop(&mut stop, secs);
            }
        }
        ScriptCommand::DamageNumber { text, pos, color } => {
            world.write_message(crate::feedback::DamageNumberEvent {
                position: pos,
                text,
                color: color
                    .map(|[r, g, b]| Color::srgb(r, g, b))
                    .unwrap_or(Color::srgb(1.0, 0.9, 0.4)),
            });
        }
        ScriptCommand::OwnSystem(name) => {
            if let Some(mut owners) = world.get_resource_mut::<super::ownership::ScriptSystemOwners>()
            {
                owners.0.insert(name);
            }
        }
        ScriptCommand::ReleaseSystem(name) => {
            if let Some(mut owners) = world.get_resource_mut::<super::ownership::ScriptSystemOwners>()
            {
                owners.0.remove(&name);
            }
        }
        ScriptCommand::CameraFree(active) => {
            if let Some(mut state) = world.get_resource_mut::<crate::camera::ScriptCameraFree>() {
                state.active = active;
            }
        }
        ScriptCommand::CameraPose { pos, look } => {
            if let Some(mut state) = world.get_resource_mut::<crate::camera::ScriptCameraFree>() {
                state.pos = pos;
                state.look = look;
            }
        }
        ScriptCommand::SetVisible { entity, visible } => {
            // TODO(migration): shim Lua mantido; o caminho principal é a
            // reflexão (`Visibility` é um enum: a variante é o patch).
            let variant = if visible { "Inherited" } else { "Hidden" };
            let patch = super::ecs::EcsValue::Enum(variant.to_string());
            match super::ecs::patch_component(world, entity, super::ecs::VISIBILITY_PATH, &patch) {
                // Entidade sem `Visibility`, ou app mínima sem o
                // `ReflectPlugin`: mantém o comportamento antigo de a
                // inserir. `Inherited` (e não `Visible`) para a
                // entidade voltar a respeitar o pai escondido.
                Ok(false) => insert_visibility(world, entity, visible),
                Err(error) => {
                    warn!(target: "vael::luau", "viber.set_visible: {error}");
                    insert_visibility(world, entity, visible);
                }
                Ok(true) => {}
            }
        }
        ScriptCommand::Say { text, secs, speaker } => {
            let text = world
                .get_resource::<crate::locale::Locale>()
                .map(|locale| crate::dialogue::localized_lines(&text, Some(&locale.table)))
                .unwrap_or(text);
            let duration = if secs > 0.0 {
                secs
            } else {
                crate::hud::BALLOON_DURATION
            };
            // A linha resolvida chega ao jogo (com o orador) mesmo quando
            // o HUD nativo a desenha: é o contrato para um presenter
            // próprio e para legendas/QA.
            world.write_message(super::components::ScriptSay {
                speaker,
                text: text.clone(),
                seconds: duration,
            });
            // Fala no balão baixa a música (genérico: qualquer `viber.say`
            // é diálogo, não um toast de sistema). A voz, quando existir,
            // fica por cima; o SFX não é tocado.
            if let Some(mut duck) = world.get_resource_mut::<crate::music::AudioDuck>() {
                duck.apply(
                    crate::music::AudioBus::Music,
                    0.35,
                    duration + 0.5,
                    elapsed,
                );
            }
            if world
                .get_resource::<crate::dialogue::ExternalSayPresenter>()
                .is_none()
            {
                let shown = show_balloon_world(world, &text, duration);
                if !shown {
                    warn_once(
                        world,
                        "balloon",
                        "viber.say: o mundo não declara <DialogueBalloon> — o texto não aparece no HUD",
                    );
                }
            }
        }
        ScriptCommand::SaveNow => {
            if let Some(mut request) = world.get_resource_mut::<crate::save::SaveRequest>() {
                request.save = true;
                request.slot = None;
            }
        }
        ScriptCommand::LoadNow => {
            if let Some(mut request) = world.get_resource_mut::<crate::save::SaveRequest>() {
                request.load = true;
                request.slot = None;
            }
        }
        ScriptCommand::SaveSlot(slot) => {
            if let Some(mut request) = world.get_resource_mut::<crate::save::SaveRequest>() {
                request.save = true;
                request.slot = Some(slot);
            }
        }
        ScriptCommand::LoadSlot(slot) => {
            if let Some(mut request) = world.get_resource_mut::<crate::save::SaveRequest>() {
                request.load = true;
                request.slot = Some(slot);
            }
        }
        ScriptCommand::SetLocale(lang) => {
            // O publisher (`publish_host_state`) espelha o `Locale` para o
            // `ScriptHostState` a cada frame — os contextos BMS leem-no dos
            // globals no frame seguinte (o ctx da VM antiga morreu com ela).
            if let Some(mut locale) = world.get_resource_mut::<crate::locale::Locale>() {
                locale.set_lang(&lang);
            }
        }
        ScriptCommand::Quit => {
            world.write_message(AppExit::Success);
        }
        ScriptCommand::SpawnPrototype { name, pos, seat, on_spawned, caller_path } => {
            if let Some(mut spawns) =
                world.get_resource_mut::<crate::recipes::spawn::PendingScriptSpawns>()
            {
                spawns.0.push(crate::recipes::spawn::PendingPrototypeSpawn {
                    name,
                    pos,
                    seat,
                    on_spawned,
                    caller_path,
                });
            }
        }
        ScriptCommand::SetPose { entity, pos, yaw } => {
            // TODO(migration): shim Lua mantido; a escrita passa pela
            // reflexão. Um patch parcial preserva a escala e cobre
            // qualquer entidade com `Transform` (script, herói, prop).
            let patch = super::ecs::EcsValue::Map(vec![
                (
                    "translation".to_string(),
                    super::ecs::EcsValue::Vec3([pos.x, pos.y, pos.z]),
                ),
                (
                    "rotation".to_string(),
                    super::ecs::EcsValue::Quat(Quat::from_rotation_y(yaw).to_array()),
                ),
            ]);
            if let Err(error) =
                super::ecs::patch_component(world, entity, super::ecs::TRANSFORM_PATH, &patch)
            {
                // Sem reflexão (app mínima): escrita directa de
                // sempre, preservando a escala.
                warn!(target: "vael::luau", "viber.set_pose: {error}");
                if let Some(mut transform) = world.get_mut::<Transform>(entity) {
                    let scale = transform.scale;
                    transform.translation = pos;
                    transform.rotation = Quat::from_rotation_y(yaw);
                    transform.scale = scale;
                }
            }
        }
        ScriptCommand::SoundLoop { id, clip, pitch, volume, position, ambience, owner } => {
            if let Some(mut loops) = world.get_resource_mut::<crate::music::ScriptSoundLoops>() {
                loops.wanted.insert(
                    id,
                    crate::music::ScriptLoopParams {
                        clip,
                        pitch,
                        volume,
                        position,
                        ambience,
                        owner,
                    },
                );
            }
        }
        ScriptCommand::SoundStop(id) => {
            if let Some(mut loops) = world.get_resource_mut::<crate::music::ScriptSoundLoops>() {
                loops.wanted.remove(&id);
            }
        }
        ScriptCommand::MusicLayer(layer) => {
            if let Some(mut over) = world.get_resource_mut::<crate::music::MusicLayerOverride>() {
                over.0 = layer.filter(|name| !name.is_empty());
            }
        }
        ScriptCommand::MusicCombat { active, boss } => {
            if let Some(mut combat) = world.get_resource_mut::<crate::music::CombatMusicState>() {
                if active {
                    combat.engage(elapsed, boss);
                } else {
                    combat.clear();
                }
            }
        }
        ScriptCommand::AudioDuck { bus, gain, secs } => {
            if let Some(mut duck) = world.get_resource_mut::<crate::music::AudioDuck>()
                && let Some(bus) = crate::music::AudioBus::parse(&bus)
            {
                duck.apply(bus, gain, secs, elapsed);
            }
        }
        ScriptCommand::AudioReverb { preset, wet } => {
            if let Some(mut request) = world.get_resource_mut::<crate::music::AudioReverb>()
                && let Some(preset) = crate::music::ReverbPreset::parse(&preset)
            {
                request.preset = preset;
                request.wet = wet;
            }
        }
        ScriptCommand::Decal { url, position, normal, size, life, color } => {
            if let Some(mut queue) = world.get_resource_mut::<crate::decal_fx::DecalQueue>() {
                queue.0.push(crate::decal_fx::DecalRequest {
                    url,
                    position,
                    normal,
                    size,
                    life,
                    color: Color::srgb(color[0], color[1], color[2]),
                });
            }
        }
        ScriptCommand::SetMarkers { blips, beacons, waypoint } => {
            if let Some(mut markers) = world.get_resource_mut::<crate::hud::markers::HudMarkers>() {
                markers.blips = blips
                    .into_iter()
                    .map(|(position, tower)| crate::hud::markers::HudBlip { position, tower })
                    .collect();
                markers.beacons = beacons;
                markers.waypoint = waypoint
                    .map(|(position, label)| crate::hud::markers::HudPoint { position, label });
            }
        }
        ScriptCommand::Noise { pos, loudness } => {
            if let Some(mut bus) = world.get_resource_mut::<crate::senses::SenseBus>() {
                bus.events.push(crate::senses::SenseEvent {
                    kind: crate::senses::SenseKind::Hear,
                    pos,
                    loudness,
                    from: None,
                });
            }
        }
        // `viber.ecs.*` (a tabela Lua) morreu: a escrita genérica por
        // reflexão sobrevive só nas primitivas (`set_scale`/`set_visible`
        // /`set_position` via `patch_component`).
    }
}

/// Dano em ÁREA sobre `&mut World` — corpo 1:1 do braço `RadialDamage` do
/// `luau_update`, com o `skip` do BMS (exclui o emissor; o monólito não tem
/// o campo, passa `None`).
pub(crate) fn apply_radial_damage(
    world: &mut World,
    x: f32,
    z: f32,
    radius: f32,
    damage: f32,
    knockback: f32,
    skip: Option<Entity>,
) {
    // Acções recolhidas no loop (o `iter_mut` guarda o borrow mutável do
    // World) e aplicadas fora — mesma ordem de efeito do braço original.
    let mut knocks: Vec<(Entity, Vec3, f32)> = Vec::new();
    let mut kills: Vec<Entity> = Vec::new();
    {
        let mut query = world.query_filtered::<
            (Entity, &mut Health, &GlobalTransform),
            Without<Player>,
        >();
        for (entity, mut health, transform) in query.iter_mut(world) {
            if health.current <= 0.0 || skip == Some(entity) {
                continue;
            }
            let pos = transform.translation();
            let dist = Vec2::new(pos.x - x, pos.z - z).length();
            let Some(amount) = crate::vitals::radial_damage(dist, radius, damage) else {
                continue;
            };
            crate::vitals::apply_damage(&mut health, amount);
            if knockback > 0.0
                && let Some(strength) =
                    crate::physics_fx::radial_strength(dist, radius, knockback)
            {
                let dir = (pos - Vec3::new(x, pos.y, z)).normalize_or_zero();
                knocks.push((entity, dir, strength));
            }
            if health.current <= 0.0 {
                kills.push(entity);
            }
        }
    }
    for (entity, dir, strength) in knocks {
        world
            .commands()
            .entity(entity)
            .try_insert(crate::physics_fx::knockback_after(dir, strength));
    }
    let kills = kills
        .into_iter()
        .map(|entity| {
            let name = world
                .get::<LuaScriptRef>(entity)
                .map(|lref| crate::script_kind::script_kind(&lref.path))
                .unwrap_or_else(|| "creature".to_string());
            (entity, name)
        })
        .collect::<Vec<_>>();
    for (entity, name) in kills {
        if let Some(mut events) = world.get_resource_mut::<super::events::ScriptEventQueue>() {
            events.push(super::events::ScriptGameEvent::Kill {
                name,
                entity: entity.to_bits() as i64,
            });
        }
    }
}

// ═══ PONTE: comandos BMS → engine ═══════════════════════════════════════

/// Alcance dos projéteis crus do BMS (o ponto-alvo do disparo: linha reta
/// com `arc=false`, arco balístico baixo com `arc=true`).
const BMS_PROJECTILE_RANGE: f32 = 40.0;
/// Gravidade do arco balístico pedido com `arc = true` (m/s²).
const BMS_PROJECTILE_GRAVITY: f32 = 9.81;

/// Cursor de leitura das mensagens do BMS — um sistema exclusivo não tem o
/// `MessageReader` de sistema; o cursor vive num recurso para não reler
/// mensagens de frames anteriores (o `.after(drain_commands)` só ordena).
#[derive(Resource, Default, Clone)]
struct BmsCommandCursor(
    bevy::ecs::message::MessageCursor<bevy_ext_luau::commands::ScriptCommandMessage>,
);

/// Ponte entre o host BMS e o monólito:
/// - [`publish_host_state`] (PreUpdate, ANTES do publisher do BMS) preenche
///   o [`bevy_ext_luau::ScriptHostState`] com o snapshot do frame — as
///   extrações das queries do `luau_update` (posições/nomes incrementais,
///   HP, props, hostis, prototypes, superfícies, câmara, locale, a11y…);
/// - [`apply_bms_commands`] (Update, DEPOIS do dreno do BMS) converte os
///   `ScriptCommand` do `bevy_ext_luau` para os do monólito e aplica com o
///   MESMO [`apply_script_command`] do `luau_update` — um só caminho de
///   aplicação, os mesmos recursos.
///
/// O plugin é aditivo: sem o `VaelScriptingPlugin` do BMS no app, os dois
/// sistemas não fazem nada (as mensagens não existem; o `Option<ResMut>`
/// do estado falha em silêncio).
pub struct BmsBridgePlugin;

impl bevy::app::Plugin for BmsBridgePlugin {
    fn build(&self, app: &mut bevy::app::App) {
        // O host BMS não corre o build do `LuauScriptPlugin`: os recursos e
        // mensagens que o JOGO e os braços de aplicação consomem registam-se
        // aqui (o mesmo bloco do plugin antigo, sem a VM).
        app.init_resource::<Time>()
            .init_resource::<bevy::input::ButtonInput<bevy::input::keyboard::KeyCode>>()
            .init_resource::<crate::interact::InteractionFocus>()
            .init_resource::<crate::luau::ScriptEventQueue>()
            .init_resource::<crate::ui::UiPrompt>()
            .init_resource::<crate::attachment::AttachmentRequests>()
            .init_resource::<crate::attachment::AttachmentSlots>()
            .init_resource::<crate::luau::ScriptSystemOwners>()
            .init_resource::<crate::recipes::spawn::PendingScriptSpawns>()
            .add_message::<ScriptToast>()
            .add_message::<crate::luau::ScriptSay>()
            .add_message::<crate::ambient::SfxEvent>()
            .add_message::<crate::feedback::AttackAlert>()
            .add_message::<crate::feedback::DamageNumberEvent>();
        app.add_systems(
            bevy::app::PreUpdate,
            // O foco por tecla recalcula-se ANTES do publisher: o
            // `interaction_focus` publicado aos scripts é o DESTE frame.
            crate::interact::focus_interactions.before(publish_host_state),
        );
        app.init_resource::<ScriptWarnState>()
            .init_resource::<BmsCommandCursor>()
            .init_resource::<ScriptMirrorState>()
            .init_resource::<BmsScriptExports>()
            .add_systems(
                bevy::app::PreUpdate,
                (
                    // PASSO 1 da integração: espelhar `LuaScriptRef` →
                    // `ScriptComponent` (attach BMS). Sem isto nenhum
                    // contexto nasce — os scripts do jogo não correm.
                    mirror_script_components,
                    // Esquema `--@export` dos scripts já carregados (o
                    // `viber.prop`/`viber.props` lê-no do estado publicado).
                    parse_script_exports.before(publish_host_state),
                    publish_host_state
                        // ANTES do publisher do BMS: o `publish_globals` copia
                        // o recurso para os globals dos contextos; o tick de
                        // timers lê o relógio publicado.
                        .before(bevy_ext_luau::publish_globals)
                        .before(bevy_ext_luau::timers::tick_timers),
                ),
            )
            .add_systems(
                bevy::app::Update,
                // PASSO 3 da integração: despacho `on_update(dt)` — o
                // handler do BMS procura a GLOBAL `on_update` em cada
                // contexto. Antes do dreno: os comandos enfileirados pelos
                // scripts no frame corrente são drenados e aplicados no
                // mesmo frame (`apply_bms_commands` corre depois do dreno).
                (
                    probe_scripts_start, // `VAEL_PERF_DIAG`
                    send_on_update,
                    event_handler::<OnScriptUpdate, LuaScriptingPlugin>,
                    probe_scripts_end, // `VAEL_PERF_DIAG`
                )
                    .chain()
                    .in_set(VaelScripts)
                    .before(bevy_ext_luau::commands::drain_commands),
            )
            .add_systems(
                bevy::app::Update,
                // DEPOIS do dreno do BMS: os comandos do frame corrente
                // aplicam-se no mesmo frame.
                apply_bms_commands.after(bevy_ext_luau::commands::drain_commands),
            );
    }
}

// ═══ INTEGRAÇÃO DO HOST (espelho + despacho `on_update`) ═══════════════

/// Label da callback de frame do BMS: o handler do BMS procura a GLOBAL
/// `on_update` em cada contexto (o mesmo contrato do `luau_update` do
/// monólito — os scripts não mudam).
pub struct OnScriptUpdate;

/// Âncora de ordenação para o jogo: os sistemas que lêem o que os scripts
/// escreveram NO frame (store `viber.game()`, mensagens de comando) correm
/// DEPOIS deste set (`send_on_update` + handler `on_update`), e os que
/// aplicam comandos, depois do dreno (`apply_bms_commands`).
#[derive(bevy::ecs::schedule::SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub struct VaelScripts;

// `VAEL_PERF_DIAG`
static PROBE_SCRIPTS_T0: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
fn probe_scripts_start() {
    // Sem `VAEL_PERF_DIAG` não se toca no `Mutex` — o par start/end corre em
    // todo o frame e a medição não pode custar nada quando está desligada.
    if bevy_engine_core::perf_probe::enabled() {
        *PROBE_SCRIPTS_T0.lock().unwrap() = Some(std::time::Instant::now());
    }
}
fn probe_scripts_end() {
    if let Some(t0) = PROBE_SCRIPTS_T0.lock().unwrap().take() {
        bevy_engine_core::perf_probe::record("on_update(scripts)", t0.elapsed().as_secs_f64() * 1000.0);
    }
}

impl IntoCallbackLabel for OnScriptUpdate {
    fn into_callback_label() -> CallbackLabel {
        "on_update".into()
    }
}

/// Path de asset de um `LuaScriptRef` (`scripts/<path>` relativo ao
/// mundo): o `world_dir` é a PRIMEIRA asset root do jogo
/// (`config::asset_roots`), portanto o prefixo `scripts/` resolve no
/// `AssetServer` sem source extra.
fn script_asset_path(lref: &str) -> String {
    if lref.starts_with("scripts/") {
        lref.to_string()
    } else {
        format!("scripts/{lref}")
    }
}

/// Mapa mantido por [`mirror_script_components`]: entidade → (handle do
/// `ScriptAsset`, path do asset). Alimenta a entrega de sinais BMS (o
/// `EmitSignal`/`CallGroup` precisa do handle para mirar o contexto) e o
/// parse de exports.
#[derive(Resource, Default)]
pub struct ScriptMirrorState {
    pub by_entity: HashMap<Entity, (Handle<ScriptAsset>, String)>,
}

/// Esquema `--@export` por path, extraído dos assets de script já
/// carregados ([`parse_script_exports`]). Em modo BMS substitui o
/// `ScriptCtx::exports` do host antigo como fonte do `viber.prop`.
#[derive(Resource, Default)]
pub struct BmsScriptExports {
    map: std::sync::Arc<HashMap<String, std::sync::Arc<bevy_ext_luau::exports::ScriptExports>>>,
    parsed: HashSet<AssetId<ScriptAsset>>,
}

impl BmsScriptExports {
    /// Snapshot para o `ScriptHostState.exports` (o publisher copia).
    pub fn snapshot(&self) -> HashMap<String, std::sync::Arc<bevy_ext_luau::exports::ScriptExports>> {
        (*self.map).clone()
    }
}

/// PASSO 1 da integração: espelha `LuaScriptRef` → `ScriptComponent` com
/// `ScriptAttachment::EntityScript(entity, handle)`. Os hooks do
/// `ScriptComponent` emitem os `ScriptLifecycleMessage` que o pipeline do
/// BMS consome (cria/destrói contextos); a remoção do `LuaScriptRef`
/// (streaming unload, topple) remove o espelho e desliga o contexto.
#[allow(clippy::type_complexity)]
fn mirror_script_components(
    mut commands: Commands,
    server: Res<AssetServer>,
    added: Query<(Entity, &LuaScriptRef), Added<LuaScriptRef>>,
    mut removed: RemovedComponents<LuaScriptRef>,
    mut mirror: ResMut<ScriptMirrorState>,
) {
    let _probe = bevy_engine_core::perf_probe::Probe::new("luau.mirror");
    let _probe = bevy_engine_core::perf_probe::Probe::new("mirror_script_components"); // `VAEL_PERF_DIAG`
    for (entity, lref) in &added {
        let path = script_asset_path(&lref.path);
        let handle: Handle<ScriptAsset> = server.load(&path);
        commands
            .entity(entity)
            .try_insert(ScriptComponent(vec![handle.clone()]));
        mirror.by_entity.insert(entity, (handle, path));
    }
    for entity in removed.read() {
        if mirror.by_entity.remove(&entity).is_some() {
            // Só entidades que NÓS espelhamos: um `ScriptComponent` de
            // outra via (editor/Jackdaw) não é nosso para remover.
            commands.entity(entity).try_remove::<ScriptComponent>();
        }
    }
}

/// Parse do esquema `--@export` dos scripts cujos assets JÁ chegaram (o
/// load é assíncrono; tentar antes é leitura falhada). Corre uma vez por
/// (asset, path): o conteúdo de um asset com hot-reload muda sob o MESMO
/// id — aí o schema novo só entra com o re-attach do script.
fn parse_script_exports(
    scripts: Res<Assets<ScriptAsset>>,
    mirror: Res<ScriptMirrorState>,
    mut exports: ResMut<BmsScriptExports>,
) {
    let _probe = bevy_engine_core::perf_probe::Probe::new("parse_script_exports"); // `VAEL_PERF_DIAG`
    if mirror.by_entity.is_empty() {
        return;
    }
    for (handle, path) in mirror.by_entity.values() {
        let Some(asset) = scripts.get(handle) else {
            continue;
        };
        if !exports.parsed.insert(handle.id()) {
            continue;
        }
        let source = String::from_utf8_lossy(&asset.content);
        std::sync::Arc::make_mut(&mut exports.map)
            .insert(path.clone(), std::sync::Arc::new(bevy_ext_luau::exports::parse_exports(&source)));
    }
}

/// Despacho `on_update(dt)`: uma `ScriptCallbackMessage` por script de
/// entidade, com label `on_update` e o delta acumulado. O [`event_handler`]
/// do BMS (o sistema a seguir, ordenado) lê a fila e corre a global
/// `on_update` de cada script.
///
/// LOD por distância ao herói: os scripts globais (`scripts/game/`,
/// `scripts/ui/`) correm sempre; os do mundo (criaturas, aldeões, POIs)
/// correm a cada frame perto do herói e espaçados longe dele, com o `dt`
/// acumulado desde a última chamada (os relógios dos scripts não derivam).
/// Medido a 2026-11-15: 225 scripts activos custavam ~21 ms/frame — a maior
/// parte criaturas a centenas de metros.
fn send_on_update(
    time: Res<Time>,
    mirror: Res<ScriptMirrorState>,
    transforms: Query<&GlobalTransform>,
    players: Query<&GlobalTransform, With<Player>>,
    mut pending: Local<HashMap<Entity, f64>>,
    mut frame: Local<u64>,
    mut writer: MessageWriter<ScriptCallbackMessage>,
    mut counts: Local<HashMap<String, u32>>,
) {
    let _probe = bevy_engine_core::perf_probe::Probe::new("luau.send");
    if perf_count_scripts() && *frame % 600 == 599 {
        let mut v: Vec<_> = counts.drain().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        for (path, n) in v.iter().take(25) {
            info!("PERFSCRIPT {:6.1}/frame {path}", *n as f32 / 600.0);
        }
    }
    let dt = time.delta_secs_f64();
    *frame = frame.wrapping_add(1);
    let hero = players.iter().next().map(GlobalTransform::translation);
    pending.retain(|entity, _| mirror.by_entity.contains_key(entity));
    for (entity, (handle, path)) in &mirror.by_entity {
        let acc = pending.entry(*entity).or_insert(0.0);
        *acc += dt;
        if script_skipped(path) {
            continue;
        }
        let every = script_update_interval(path, hero, transforms.get(*entity).ok());
        if every > 1 && (*frame + u64::from(entity.index().index())) % every != 0 {
            continue;
        }
        // Teto: um script que esteve parado (cena a carregar, pausa) não
        // recebe um salto enorme de tempo de uma vez.
        let step = acc.min(0.5);
        *acc = 0.0;
        if perf_count_scripts() {
            *counts.entry(path.clone()).or_default() += 1;
        }
        writer.write(ScriptCallbackMessage::new_for_script_entity(
            OnScriptUpdate::into_callback_label(),
            vec![ScriptValue::Float(step)],
            handle.clone(),
            *entity,
        ));
    }
}

/// Diagnóstico: `VAEL_SCRIPT_SKIP=prefixo,…` não despacha o `on_update`
/// dos scripts cujo caminho começa por um dos prefixos (bisseção de custo).
fn script_skipped(path: &str) -> bool {
    static SKIP: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    SKIP.get_or_init(|| {
        std::env::var("VAEL_SCRIPT_SKIP")
            .ok()
            .map(|raw| {
                raw.split(',')
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    })
    .iter()
    .any(|prefix| path.starts_with(prefix.as_str()))
}

fn perf_count_scripts() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("VAEL_PERF_DIAG").ok().as_deref() == Some("1"))
}

/// Raio (m) até onde um script do mundo corre a cada frame (prompts de
/// interação, combate corpo a corpo).
const SCRIPT_LOD_NEAR_M: f32 = 24.0;
/// Raio (m) até onde corre a cada [`SCRIPT_LOD_CLOSE_EVERY`] frames.
const SCRIPT_LOD_CLOSE_M: f32 = 60.0;
const SCRIPT_LOD_CLOSE_EVERY: u64 = 2;
/// Raio (m) até onde corre a cada [`SCRIPT_LOD_MID_EVERY`] frames.
const SCRIPT_LOD_MID_M: f32 = 120.0;
/// A partir dos 120 m o script é invisível (o anel do véu são 150 m, mas a
/// câmara joga a 20-30 m): 6 frames em vez de 4 corta ~1/3 dos despachos
/// nessa banda sem nada notório. Medido a 2026-11-16: `on_update(scripts)`
/// custava 4,3 ms/frame.
const SCRIPT_LOD_MID_EVERY: u64 = 6;
/// Além do raio médio: a cada N frames (o `dt` acumula; 2,5 Hz a 60 fps —
/// o registo de hostis aceita entradas com menos de 1 s).
/// A >120 m. 48 frames (0,8 s a 60 fps) para a AI de longe: o jogador não
/// distingue um slime a 130 m a reagir em 0,4 s ou 0,8 s.
const SCRIPT_LOD_FAR_EVERY: u64 = 48;

/// De quantos em quantos frames corre o `on_update` deste script.
fn script_update_interval(path: &str, hero: Option<Vec3>, at: Option<&GlobalTransform>) -> u64 {
    let global = ["scripts/game/", "scripts/ui/", "scripts/lib/"]
        .iter()
        .any(|prefix| path.starts_with(prefix));
    let (Some(hero), Some(at), false) = (hero, at, global) else {
        return 1;
    };
    // `VAEL_SCRIPT_LOD_SCALE` escala os raios (A/B; 0 = tudo "longe").
    static SCALE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    let scale = *SCALE.get_or_init(|| {
        std::env::var("VAEL_SCRIPT_LOD_SCALE")
            .ok()
            .and_then(|v| v.trim().parse::<f32>().ok())
            .filter(|v| v.is_finite() && *v >= 0.0)
            .unwrap_or(1.0)
    });
    let d = at.translation().distance(hero) / scale.max(1e-3);
    if d <= SCRIPT_LOD_NEAR_M {
        1
    } else if d <= SCRIPT_LOD_CLOSE_M {
        SCRIPT_LOD_CLOSE_EVERY
    } else if d <= SCRIPT_LOD_MID_M {
        SCRIPT_LOD_MID_EVERY
    } else {
        SCRIPT_LOD_FAR_EVERY
    }
}

/// Sistema EXCLUSIVO: lê as `ScriptCommandMessage` escritas pelo
/// `drain_commands` do BMS, converte cada variante para o `ScriptCommand`
/// do monólito e aplica com [`apply_script_command`].
pub fn apply_bms_commands(world: &mut World) {
    let _probe = bevy_engine_core::perf_probe::Probe::new("luau.apply");
    let _probe = bevy_engine_core::perf_probe::Probe::new("apply_bms_commands"); // `VAEL_PERF_DIAG`
    // Lê e avança um cursor LOCAL (o recurso fica com o avançado — os
    // borrows do World são por fases).
    let (pending, cursor) = {
        let Some(messages) =
            world.get_resource::<Messages<bevy_ext_luau::commands::ScriptCommandMessage>>()
        else {
            return;
        };
        let mut cursor = world.resource::<BmsCommandCursor>().clone();
        let pending = cursor
            .0
            .read(messages)
            .map(|m| m.0.clone())
            .collect::<Vec<_>>();
        (pending, cursor)
    };
    world.resource_mut::<BmsCommandCursor>().0 = cursor.0;
    for command in pending {
        // A conversão pode aplicar inline (as variantes sem geméa no
        // monólito — `SetPosition`, `RadialDamage` com `skip`, sinais) ou
        // devolver o comando convertido para o caminho comum.
        if let Some(mapped) = convert_bms_command(world, command) {
            apply_script_command(world, mapped);
        }
    }
}

/// Converte um comando do BMS para o geméa do monólito. `None` = aplicado
/// inline (ou morto, com aviso 1×) — nunca perde-se em silêncio.
fn convert_bms_command(
    world: &mut World,
    command: bevy_ext_luau::commands::ScriptCommand,
) -> Option<ScriptCommand> {
    use bevy_ext_luau::commands::ScriptCommand as Bms;
    match command {
        // ── 1:1 (mesmos nomes e campos) ─────────────────────────────────
        Bms::PlaySfx { clip, position } => {
            Some(ScriptCommand::PlaySfx { clip, position })
        }
        Bms::Shake(amount) => Some(ScriptCommand::Shake(amount)),
        Bms::FovKick(deg) => Some(ScriptCommand::FovKick(deg)),
        Bms::HitStop(secs) => Some(ScriptCommand::HitStop(secs)),
        Bms::DamageNumber { text, pos, color } => {
            Some(ScriptCommand::DamageNumber { text, pos, color })
        }
        Bms::Ring { x, z, radius, color } => {
            Some(ScriptCommand::Ring { x, z, radius, color })
        }
        Bms::Topple { entity } => Some(ScriptCommand::Topple { entity }),
        Bms::OwnUiAction(name) => Some(ScriptCommand::OwnUiAction(name)),
        Bms::Despawn(entity) => Some(ScriptCommand::Despawn(entity)),
        Bms::OwnSystem(name) => Some(ScriptCommand::OwnSystem(name)),
        Bms::ReleaseSystem(name) => Some(ScriptCommand::ReleaseSystem(name)),
        Bms::SetVisible { entity, visible } => {
            Some(ScriptCommand::SetVisible { entity, visible })
        }
        Bms::SetScale { entity, scale } => {
            Some(ScriptCommand::SetScale { entity, scale })
        }
        Bms::SetPose { entity, pos, yaw } => {
            Some(ScriptCommand::SetPose { entity, pos, yaw })
        }
        Bms::Burst { preset, pos, count } => {
            Some(ScriptCommand::Burst { preset, pos, count })
        }
        Bms::Decal { url, position, normal, size, life, color } => Some(ScriptCommand::Decal {
            url,
            position,
            normal,
            size,
            life,
            color,
        }),
        Bms::SpawnPrototype { name, pos, seat, on_spawned, caller_path } => {
            Some(ScriptCommand::SpawnPrototype {
                name,
                pos,
                seat,
                on_spawned,
                caller_path,
            })
        }
        Bms::HitFlash { entity, secs } => {
            Some(ScriptCommand::HitFlash { entity, secs })
        }
        Bms::Corpse { entity } => Some(ScriptCommand::Corpse { entity }),
        Bms::Attach { owner, slot, url, bone, pos, rot, scale } => {
            Some(ScriptCommand::Attach {
                owner,
                slot,
                url,
                bone,
                pos,
                rot,
                scale,
            })
        }
        Bms::SoundLoop { id, clip, pitch, volume, position, ambience, owner } => {
            Some(ScriptCommand::SoundLoop {
                id,
                clip,
                pitch,
                volume,
                position,
                ambience,
                owner,
            })
        }
        Bms::SoundStop(id) => Some(ScriptCommand::SoundStop(id)),
        Bms::MusicLayer(layer) => Some(ScriptCommand::MusicLayer(layer)),
        Bms::MusicCombat { active, boss } => {
            Some(ScriptCommand::MusicCombat { active, boss })
        }
        Bms::AudioDuck { bus, gain, secs } => {
            Some(ScriptCommand::AudioDuck { bus, gain, secs })
        }
        Bms::AudioReverb { preset, wet } => {
            Some(ScriptCommand::AudioReverb { preset, wet })
        }
        Bms::Punch { stops, bloom } => Some(ScriptCommand::Punch { stops, bloom }),
        Bms::CameraKick(impulse) => Some(ScriptCommand::CameraKick(impulse)),
        Bms::SetLantern(intensity) => Some(ScriptCommand::SetLantern(intensity)),
        Bms::SetBoneOffset { bone, rotation } => {
            Some(ScriptCommand::SetBoneOffset {
                entity: None, // o editor fala com o herói
                bone,
                rotation,
            })
        }
        Bms::CameraFree(active) => Some(ScriptCommand::CameraFree(active)),
        Bms::CameraPose { pos, look } => Some(ScriptCommand::CameraPose { pos, look }),
        Bms::EntityDamage { entity, amount } => {
            Some(ScriptCommand::EntityDamage { entity, amount })
        }
        Bms::EntityHeal { entity, amount } => {
            Some(ScriptCommand::EntityHeal { entity, amount })
        }
        Bms::EntitySetMaxHp { entity, max } => {
            Some(ScriptCommand::EntitySetMaxHp { entity, max })
        }
        Bms::Noise { pos, loudness } => Some(ScriptCommand::Noise { pos, loudness }),
        Bms::SetPlayerSpeed(mps) => {
            // BMS fala m/s; o monólito recebe o MULTIPLICADOR da base
            // (`player.speed = BASE_SPEED * mult`), com o mesmo clamp
            // 0.1..=5.0 que a API Lua do monólito aplicava.
            let mult = (mps / crate::player::BASE_SPEED).clamp(0.1, 5.0);
            Some(ScriptCommand::SetPlayerSpeed(mult))
        }
        Bms::PlayCinematic { id, anchor } => {
            Some(ScriptCommand::PlayCinematic { id, anchor })
        }
        Bms::Say { text, secs, speaker } => {
            Some(ScriptCommand::Say { text, secs, speaker })
        }
        Bms::Quit => Some(ScriptCommand::Quit),
        Bms::A11ySet { subtitle_scale, shake, hitstop, interact_hold } => {
            Some(ScriptCommand::A11ySet {
                subtitle_scale,
                shake,
                hitstop,
                interact_hold,
            })
        }
        Bms::SetMarkers { blips, beacons, waypoint } => Some(ScriptCommand::SetMarkers {
            blips,
            beacons,
            waypoint,
        }),
        Bms::Prompt { key, label } => Some(ScriptCommand::Prompt { key, label }),
        Bms::SetLocale(lang) => Some(ScriptCommand::SetLocale(lang)),

        // ── adaptadas (diferenças de campos) ────────────────────────────
        Bms::Gesture { entity, name, speed, lock, blend, duration } => {
            // O BMS resolve a própria entidade à fila (`Some`); `None`
            // só chega de fora de um script (REPL) — sem alvo, no-op.
            let Some(entity) = entity else {
                warn_once(
                    world,
                    "bms:gesture-none",
                    "ponte BMS: `Gesture` sem entidade (fora de on_update) — ignorado",
                );
                return None;
            };
            Some(ScriptCommand::Gesture {
                entity,
                name,
                speed,
                lock,
                blend,
                duration,
            })
        }
        Bms::SetInteraction { entity, label, key, range } => {
            let Some(entity) = entity else {
                warn_once(
                    world,
                    "bms:interaction-none",
                    "ponte BMS: `SetInteraction` sem entidade (fora de on_update) — ignorado",
                );
                return None;
            };
            Some(ScriptCommand::SetInteraction {
                entity,
                label,
                key,
                range,
            })
        }
        Bms::FaceTowards { entity, target } => {
            Some(ScriptCommand::FaceTowards(entity, target))
        }
        Bms::MoveBy { entity, velocity } => Some(ScriptCommand::MoveBy(entity, velocity)),
        Bms::FireProjectileTemplate { template, target, shooter } => {
            // Boca: a pose do atirador neste frame + a altura do cano; alvo
            // omisso = o peito do herói (o contrato do monólito).
            let origin = shooter
                .and_then(|e| world.get::<GlobalTransform>(e))
                .map(|g| g.translation())?
                + Vec3::Y * crate::projectile::MUZZLE_HEIGHT;
            let target = match target {
                Some(target) => target,
                None => {
                    let mut players = world
                        .query_filtered::<&GlobalTransform, With<crate::player::Player>>();
                    players.iter(world).next()?.translation()
                        + Vec3::Y * crate::projectile::CHEST_HEIGHT
                }
            };
            Some(ScriptCommand::FireProjectile { template, origin, target, shooter })
        }
        Bms::SetLocomotion { entity, walk, run, turn_rate } => {
            Some(ScriptCommand::SetLocomotion { entity, walk, run, turn_rate })
        }
        Bms::TeleportPlayer { pos, exclude } => {
            Some(ScriptCommand::TeleportPlayer { pos, exclude })
        }
        Bms::MoveTowards { entity, target, speed } => {
            // BMS leva o alvo em 3D; a locomoção do monólito é planar (o
            // snap de Y é do `apply_ai_locomotion`).
            Some(ScriptCommand::MoveTowards {
                entity,
                goal: Vec2::new(target.x, target.z),
                speed,
            })
        }
        Bms::ChangeScene { scene, entry } => Some(ScriptCommand::ChangeScene {
            scene,
            entry,
            // O BMS não sabe quem pediu (sem `via`).
            via: None,
        }),
        Bms::PushPlayer { dir, force } => {
            // BMS: direção + força; o monólito leva a VELOCIDADE de
            // deslocamento (`push_velocity` zera o Y e escala pelo decay,
            // como o braço nativo).
            Some(ScriptCommand::PushPlayer {
                velocity: bevy_engine_core::push_velocity(dir.normalize_or_zero() * force),
            })
        }
        Bms::EntityKnockback { entity, dir, force } => Some(ScriptCommand::EntityKnockback {
            entity,
            dir,
            strength: force,
        }),
        Bms::SaveGame(slot) => Some(match slot {
            None => ScriptCommand::SaveNow,
            Some(slot) => ScriptCommand::SaveSlot(slot),
        }),
        Bms::LoadGame(slot) => Some(match slot {
            None => ScriptCommand::LoadNow,
            Some(slot) => ScriptCommand::LoadSlot(slot),
        }),
        Bms::TerrainEdit { kind, pos, radius, strength } => {
            // O BMS fala `kind` string; o monólito leva a variante tipada.
            // `smooth` não tem geméa no monólito (warn 1× e cai).
            let edit = match kind.as_str() {
                "lower" => crate::terrain::delta::TerrainEdit::Lower {
                    at: Vec2::new(pos.x, pos.z),
                    radius,
                    depth: strength,
                },
                "raise" => crate::terrain::delta::TerrainEdit::Raise {
                    at: Vec2::new(pos.x, pos.z),
                    radius,
                    height: strength,
                },
                "flatten" => crate::terrain::delta::TerrainEdit::Flatten {
                    at: Vec2::new(pos.x, pos.z),
                    radius,
                    height: None,
                },
                other => {
                    warn_once(
                        world,
                        "bms:terrain-smooth",
                        &format!("ponte BMS: terrain '{other}' sem geméa no monólito — ignorado"),
                    );
                    return None;
                }
            };
            Some(ScriptCommand::TerrainEdit { edit })
        }
        Bms::FireProjectile { from, dir, speed, damage, faction, arc, shooter } => {
            Some(bms_fire_projectile(world, from, dir, speed, damage, faction, arc, shooter))
        }

        // ── sem geméa: aplicadas inline (ou mortas) ─────────────────────
        Bms::SetPosition { entity, pos } => {
            apply_set_position(world, entity, pos);
            None
        }
        Bms::RadialDamage { pos, radius, max_damage, skip } => {
            // Dano planar (x, z), como o braço do monólito; sem knockback
            // (o BMS não traz o campo) e com o `skip` do emissor.
            apply_radial_damage(world, pos.x, pos.z, radius, max_damage, 0.0, skip);
            None
        }
        Bms::EmitSignal { emitter, signal, args } => {
            deliver_emit(world, emitter, &signal, &args);
            None
        }
        Bms::CallGroup { group, method, args, sender } => {
            deliver_group(
                world,
                &group,
                &method,
                &args,
                sender.map(|entity| entity.to_bits() as i64),
            );
            None
        }
        Bms::TimerFire { .. } => {
            // Morto: o tick do BMS (`timers::tick_timers`) corre as
            // callbacks diretamente — este comando nunca devia chegar à
            // ponte; um aviso 1× marca a regressão sem travar o frame.
            warn_once(
                world,
                "bms:timer-fire",
                "ponte BMS: `TimerFire` chegou ao monólito (o tick BMS devia correr a callback) — ignorado",
            );
            None
        }
    }
}

/// Projétil CRU do BMS (`from/dir/speed/damage/faction/arc`) — o monólito
/// só dispara de `<ProjectileTemplate>`. A ponte regista um template
/// sintético DETERMINÍSTICO por conjunto de parâmetros (`bms:{facção}:
/// {speed}:{damage}:{arc}`, recriado na troca de cena — o
/// `install_templates` limpa o registo) e dispara para
/// `from + dir·ALCANCE`: linha reta exata com `arc=false` (a
/// `launch_velocity` com gravidade 0 devolve `dir·speed`), arco balístico
/// baixo com `arc=true`.
fn bms_fire_projectile(
    world: &mut World,
    from: Vec3,
    dir: Vec3,
    speed: f32,
    damage: f32,
    faction: String,
    arc: bool,
    shooter: Option<Entity>,
) -> ScriptCommand {
    use crate::projectile::{Faction, ProjectileTemplate};
    let faction = match faction.as_str() {
        "enemy" | "hostile" => Faction::Enemy,
        "player" | "hero" | "ally" => Faction::Player,
        "neutral" | "none" => Faction::Neutral,
        other => {
            // O BMS não valida a facção à fila — o default do monólito é
            // enemy (mesma regra do `from_config`).
            warn_once(
                world,
                &format!("bms:projectile-faction:{other}"),
                &format!("ponte BMS: facção de projétil '{other}' desconhecida — enemy"),
            );
            Faction::Enemy
        }
    };
    let template_id = format!("bms:{faction:?}:{speed}:{damage}:{arc}");
    let speed = speed.max(0.1);
    if let Some(mut templates) =
        world.get_resource_mut::<crate::projectile::ProjectileTemplates>()
        && !templates.0.contains_key(&template_id)
    {
        templates.0.insert(
            template_id.clone(),
            ProjectileTemplate {
                id: template_id.clone(),
                speed,
                damage: damage.max(0.0),
                // Vida suficiente para atravessar o alcance do disparo.
                max_life: (BMS_PROJECTILE_RANGE / speed) + 0.5,
                sensor_radius: 0.3,
                faction,
                gravity: if arc { BMS_PROJECTILE_GRAVITY } else { 0.0 },
                color: match faction {
                    Faction::Enemy => [1.0, 0.35, 0.2],
                    Faction::Player => [0.35, 0.75, 1.0],
                    Faction::Neutral => [0.9, 0.9, 0.8],
                },
                size: 0.12,
            },
        );
    }
    ScriptCommand::FireProjectile {
        template: template_id,
        origin: from,
        target: from + dir * BMS_PROJECTILE_RANGE,
        shooter,
    }
}

/// `viber.set_position` do BMS — o mesmo algoritmo do compat
/// `host.take_pending` do `luau_update`: a posição é de MUNDO e o
/// `Transform` é local; a pose do pai infere-se do par
/// (`GlobalTransform`, `Transform`) da própria entidade.
fn apply_set_position(world: &mut World, entity: Entity, pos: Vec3) {
    let parent_world = {
        let local = world.get::<Transform>(entity).map(|t| t.compute_affine());
        let global = world
            .get::<GlobalTransform>(entity)
            .map(|global| global.affine());
        match (global, local) {
            (Some(global), Some(local)) => global * local.inverse(),
            _ => bevy::math::Affine3A::IDENTITY,
        }
    };
    if let Some(mut transform) = world.get_mut::<Transform>(entity) {
        transform.translation = parent_world.inverse().transform_point3(pos);
    }
}

// ═══ ENTREGA DE SINAIS (o `viber.emit`/`call_group` do BMS corre os
// handlers nos contextos do host do monólito — o mesmo caminho do loop de
// sinais do `luau_update`, aqui para UMA chamada por comando). ═══════════

/// Entrega `viber.emit`: os alvos vêm dos `connect-<sinal>` do emissor
/// (`self`/nome/`@grupo`), resolvidos pelos snapshots de nomes/grupos do
/// host. Handlers que emitem outra vez entram na fila do próprio host
/// (entregues pelo `luau_update` no frame seguinte).
fn deliver_emit(
    world: &mut World,
    emitter: Option<Entity>,
    signal: &str,
    args: &[serde_json::Value],
) {
    let Some(emitter) = emitter else {
        return; // sinal sem emissor: como no monólito, nada acontece
    };
    // Cópias locais: o borrow das conexões não pode atravessar o
    // `run_method`.
    let conns: Vec<(bevy_ext_luau::exports::ConnTarget, String)> = world
        .get::<super::components::SignalConnections>(emitter)
        .map(|connections| {
            connections
                .of(signal)
                .iter()
                .map(|conn| (conn.target.clone(), conn.method.clone()))
                .collect()
        })
        .unwrap_or_default();
    let sender = emitter.to_bits() as i64;
    for (target_spec, method) in conns {
        let targets: Vec<i64> = match &target_spec {
            bevy_ext_luau::exports::ConnTarget::Name(name) if name == "self" => {
                vec![sender]
            }
            bevy_ext_luau::exports::ConnTarget::Name(name) => {
                host_members(world, &bevy_ext_luau::exports::ConnTarget::Name(name.clone()))
            }
            bevy_ext_luau::exports::ConnTarget::Group(group) => host_members(
                world,
                &bevy_ext_luau::exports::ConnTarget::Group(group.clone()),
            ),
        };
        if targets.is_empty() {
            warn_once(
                world,
                &format!("signal-target:{signal}:{target_spec}"),
                &format!("[luau] sinal '{signal}': alvo '{target_spec}' sem entidades"),
            );
            continue;
        }
        for bits in targets {
            let Some(target) = Entity::try_from_bits(bits as u64) else {
                continue;
            };
            run_on_target(world, target, &method, args, Some(sender));
        }
    }
}

/// Entrega `viber.call_group`: o método corre em todos os membros
/// publicados do grupo.
fn deliver_group(
    world: &mut World,
    group: &str,
    method: &str,
    args: &[serde_json::Value],
    sender: Option<i64>,
) {
    let members = host_members(
        world,
        &bevy_ext_luau::exports::ConnTarget::Group(group.to_string()),
    );
    for bits in members {
        let Some(target) = Entity::try_from_bits(bits as u64) else {
            continue;
        };
        run_on_target(world, target, method, args, sender);
    }
}

/// Membros de um alvo (`nome` ou `@grupo`) segundo o snapshot do
/// `ScriptHostState` publicado (as MESMAS extrações de nomes/grupos que o
/// `luau_update` do host antigo semeava no ctx dele).
fn host_members(world: &World, target: &bevy_ext_luau::exports::ConnTarget) -> Vec<i64> {
    let Some(state) = world.get_resource::<bevy_ext_luau::ScriptHostState>() else {
        return Vec::new();
    };
    match target {
        bevy_ext_luau::exports::ConnTarget::Name(name) => state
            .named_entities
            .get(name)
            .cloned()
            .unwrap_or_default(),
        bevy_ext_luau::exports::ConnTarget::Group(group) => {
            state.groups.iter().filter_map(|(bits, groups)| {
                groups
                    .contains(group)
                    .then_some(*bits)
            }).collect()
        }
    }
}

/// Corre `method(args…)` no contexto do alvo — entrega BMS
/// (`ScriptCallbackMessage` com o método como label — o handler procura a
/// GLOBAL `method` no contexto do alvo; o host antigo do monólito, que
/// corria o método na VM partilhada com semente de contexto, morreu com ele).
fn run_on_target(
    world: &mut World,
    target: Entity,
    method: &str,
    args: &[serde_json::Value],
    _sender: Option<i64>,
) {
    let Some(_path) = world
        .get::<LuaScriptRef>(target)
        .map(|lref| lref.path.clone())
    else {
        warn_once(
            world,
            &format!("signal-noscript:{target}:{method}"),
            &format!("[luau] {method}: o alvo {target} não tem script — ignorado"),
        );
        return;
    };
    deliver_bms_callback(world, target, method, args);
}

/// Entrega BMS de um método de sinal: `ScriptCallbackMessage` mirando o
/// contexto `EntityScript` do alvo. O handler do BMS procura a GLOBAL
/// `method` — o mesmo contrato do `run_method` do host antigo (a função
/// global de topo do script é o método do sinal).
fn deliver_bms_callback(
    world: &mut World,
    target: Entity,
    method: &str,
    args: &[serde_json::Value],
) {
    let Some(handle) = world
        .get_resource::<ScriptMirrorState>()
        .and_then(|mirror| mirror.by_entity.get(&target))
        .map(|(handle, _)| handle.clone())
    else {
        warn_once(
            world,
            &format!("bms:signal-nomirror:{target}:{method}"),
            &format!("[luau] {method}: o alvo {target} não está espelhado no BMS — ignorado"),
        );
        return;
    };
    let values = args.iter().map(json_to_script_value).collect();
    world.write_message(ScriptCallbackMessage::new_for_script_entity(
        // CallbackLabel filtra caracteres inválidos; um método com nome
        // estranho cai em "" e o handler ignora (não há global "").
        CallbackLabel::new_lossy(method),
        values,
        handle,
        target,
    ));
}

/// JSON → `ScriptValue` para os args de sinais (os mesmos valores que o
/// `json_to_lua` do `game` produziria, na variante BMS).
fn json_to_script_value(value: &serde_json::Value) -> ScriptValue {
    match value {
        serde_json::Value::Null => ScriptValue::Unit,
        serde_json::Value::Bool(b) => ScriptValue::Bool(*b),
        serde_json::Value::Number(n) => n
            .as_f64()
            .map(ScriptValue::Float)
            .unwrap_or(ScriptValue::Unit),
        serde_json::Value::String(s) => ScriptValue::String(s.clone().into()),
        serde_json::Value::Array(items) => ScriptValue::List(
            items
                .iter()
                .map(json_to_script_value)
                .collect::<std::collections::VecDeque<_>>(),
        ),
        serde_json::Value::Object(map) => ScriptValue::Map(
            map.iter()
                .map(|(key, value)| (key.clone(), json_to_script_value(value)))
                .collect(),
        ),
    }
}

// ═══ PONTE: estado do monólito → globals do BMS ═════════════════════════

/// Consultas de superfície do monólito sobre o SEAM do BMS
/// (`bevy_ext_luau::SurfaceQueries`): o leitor do terreno (altura + voxel +
/// overlay de edições vivas — recriado por frame, as edições são mutáveis)
/// e os registos de estradas/água imutáveis pós-bootstrap.
struct BmsSurfaces {
    reader: crate::terrain::runtime::TerrainReader,
    registries: std::sync::Arc<super::ctx::SurfaceRegistries>,
    /// Metade do tamanho do mundo (`in_field` do runtime — fora da pegada
    /// não há terreno: as amostras saturariam na borda).
    field_half: f32,
}

impl bevy_ext_luau::SurfaceQueries for BmsSurfaces {
    fn ground_below(&self, x: f32, y: f32, z: f32) -> Option<f32> {
        self.reader.voxel.surface_below(&self.reader.base(), x, z, y)
    }

    fn ground_top(&self, x: f32, z: f32) -> Option<f32> {
        (x >= -self.field_half && x <= self.field_half && z >= -self.field_half && z <= self.field_half)
            .then(|| self.reader.sample(x, z))
    }

    fn on_road(&self, x: f32, z: f32) -> bool {
        self.registries.on_road(x, z)
    }

    fn in_water(&self, x: f32, z: f32) -> bool {
        self.registries.in_water(x, z)
    }

    fn nearest_road(&self, x: f32, z: f32) -> Option<bevy_ext_luau::RoadSample> {
        self.registries.nearest_road(x, z).map(road_sample_flat)
    }

    fn road_at(&self, s: f32) -> Option<bevy_ext_luau::RoadSample> {
        self.registries.road_at(s).map(road_sample_flat)
    }
}

/// Espelho plano do `RoadSample` (10 números, os MESMOS campos).
fn road_sample_flat(sample: crate::terrain::roads::RoadSample) -> bevy_ext_luau::RoadSample {
    bevy_ext_luau::RoadSample {
        s: sample.s,
        length: sample.length,
        half_width: sample.half_width,
        lateral: sample.lateral,
        deck: sample.deck,
        deck_y: sample.deck_y,
        heading: sample.heading,
        center_dist: sample.center_dist,
        cx: sample.cx,
        cz: sample.cz,
    }
}

/// Sobra do snapshot incremental de nomes (`forget_named` do `luau_update`):
/// uma entidade sai dos vetores por nome e dos mapas de posição/nome; o
/// nome morre com o último bits.
fn forget_named(
    named: &mut HashMap<String, Vec<i64>>,
    positions: &mut HashMap<i64, Vec3>,
    names: &mut HashMap<i64, String>,
    bits: i64,
) {
    positions.remove(&bits);
    // Só a lista do PRÓPRIO nome: o `retain` sobre o mapa inteiro era O(N)
    // por despawn (~80k nomes; os efeitos nascem e morrem a cada golpe).
    let Some(name) = names.remove(&bits) else {
        return;
    };
    if let Some(list) = named.get_mut(&name) {
        list.retain(|b| *b != bits);
        if list.is_empty() {
            named.remove(&name);
        }
    }
}

/// Queries do publisher (as MESMAS do `luau_update`) num `SystemParam` —
/// o sistema passava do teto de 16 parâmetros do Bevy.
#[derive(bevy::ecs::system::SystemParam)]
struct PublishStateParams<'w, 's> {
    players: Query<
        'w,
        's,
        (Entity, &'static GlobalTransform, Option<&'static Health>),
        With<Player>,
    >,
    animators: Query<'w, 's, &'static crate::animation::CharacterAnimator>,
    names: Query<'w, 's, (Entity, &'static Name, &'static GlobalTransform), Without<Player>>,
    names_changed: Query<
        'w,
        's,
        (Entity, &'static Name, &'static GlobalTransform),
        (Without<Player>, Or<(Changed<Name>, Changed<GlobalTransform>)>),
    >,
    names_removed: bevy::ecs::lifecycle::RemovedComponents<'w, 's, Name>,
    // Entidades com script SEM `Name` (as criaturas dos spawners): o
    // `viber.position()` delas lê o mesmo mapa. Sem isto viam-se todas na
    // ORIGEM — a praça da vila: todas perseguiam o herói de qualquer ponto
    // do mapa, empilhavam-se nele e nunca atacavam (a distância do golpe
    // era a do herói à origem). Visto 2026-11-15.
    unnamed_scripted: Query<
        'w,
        's,
        (Entity, &'static GlobalTransform),
        (
            With<ScriptComponent>,
            Without<Name>,
            Without<Player>,
            // `Added<ScriptComponent>`: o espelho põe o script DEPOIS do spawn
            // (o `Changed<GlobalTransform>` desse frame já passou) — uma
            // criatura parada nunca entrava no mapa e via-se na origem.
            Or<(Changed<GlobalTransform>, Added<ScriptComponent>)>,
        ),
    >,
    scripts_removed: bevy::ecs::lifecycle::RemovedComponents<'w, 's, ScriptComponent>,
    healths: Query<'w, 's, (Entity, &'static Health, &'static GlobalTransform), Without<Player>>,
    script_props: Query<'w, 's, (Entity, Ref<'static, ScriptProps>)>,
    props_removed: RemovedComponents<'w, 's, ScriptProps>,
    groups: Query<'w, 's, (Entity, Ref<'static, crate::luau::Groups>)>,
    groups_removed: RemovedComponents<'w, 's, crate::luau::Groups>,
    hostiles: Query<
        'w,
        's,
        (Entity, Option<&'static LuaScriptRef>, &'static GlobalTransform),
        (With<crate::vitals::Hostile>, Without<Player>),
    >,
}

/// Recursos do publisher — todos `Option`: apps mínimas de teste montam o
/// sistema sem os plugins de cena/áudio/save/rede.
#[derive(bevy::ecs::system::SystemParam)]
struct PublishStateResources<'w> {
    scene: Option<bevy::ecs::system::Res<'w, crate::scene_loader::CurrentScene>>,
    biome: Option<bevy::ecs::system::Res<'w, crate::ambient::CurrentBiome>>,
    prototypes: Option<bevy::ecs::system::Res<'w, crate::recipes::spawn::PrototypeLibrary>>,
    camera_snapshot: Option<bevy::ecs::system::Res<'w, crate::camera::CameraSnapshot>>,
    cinematic: Option<bevy::ecs::system::Res<'w, crate::cinematic::CinematicPlayback>>,
    terrain: Option<bevy::ecs::system::Res<'w, crate::terrain::runtime::TerrainRuntime>>,
    a11y: Option<bevy::ecs::system::Res<'w, crate::a11y::A11y>>,
    net: Option<bevy::ecs::system::Res<'w, crate::netplay::NetLink>>,
    locale: Option<bevy::ecs::system::Res<'w, crate::locale::Locale>>,
    save_key: Option<bevy::ecs::system::Res<'w, crate::save::WorldSaveKey>>,
    save_dir: Option<bevy::ecs::system::Res<'w, crate::save::SaveDir>>,
    // Modo BMS: foco de interação, gate de escrita de UI e o esquema
    // `--@export` extraído dos assets de script.
    focus: Option<bevy::ecs::system::Res<'w, crate::interact::InteractionFocus>>,
    typing: Option<bevy::ecs::system::Res<'w, crate::ui::widgets::UiFocusedInput>>,
    bms_exports: Option<bevy::ecs::system::Res<'w, BmsScriptExports>>,
}

/// Preenche o [`bevy_ext_luau::ScriptHostState`] a partir do World — as
/// MESMAS extrações que o `luau_update` semeava no `ScriptCtx`, com os
/// mesmos avisos de custo (nomes incrementais, prototypes só quando mudam).
#[allow(clippy::type_complexity)]
fn publish_host_state(
    time: Res<Time>,
    mut state: Option<ResMut<bevy_ext_luau::ScriptHostState>>,
    mut queries: PublishStateParams,
    res: PublishStateResources,
    mut names_seeded: Local<bool>,
    // Registos de estradas/água — imutáveis pós-bootstrap (reconstruídos
    // só quando muda o runtime do terreno).
    mut road_water: Local<Option<std::sync::Arc<super::ctx::SurfaceRegistries>>>,
    mut slots_timer: Local<Option<f64>>,
) {
    let _probe = bevy_engine_core::perf_probe::Probe::new("luau.publish_state");
    let _probe = bevy_engine_core::perf_probe::Probe::new("publish_host_state"); // `VAEL_PERF_DIAG`
    // Sem o plugin BMS no app não há estado a preencher (e o custo de
    // publicar seria lixo puro).
    let reseed = state.as_ref().is_some_and(|s| s.is_added());
    let Some(state) = state.as_deref_mut() else {
        return;
    };
    state.time = time.elapsed_secs_f64();

    // Herói: posição de MUNDO, forward (+Z), HP e bits — a mesma extração
    // da query `players` do `luau_update`.
    if let Ok((entity, global, health)) = queries.players.single() {
        let f = crate::player::hero_forward(global);
        state.player = Some(global.translation());
        state.player_forward = Vec2::new(f.x, f.z);
        state.player_hp = health.map(|h| (h.current, h.max));
        state.player_id = Some(entity.to_bits() as i64);
        // Duração de cada clip do rig do herói (`viber.clip_duration`).
        state.clip_durations = queries
            .animators
            .get(entity)
            .map(|animator| {
                animator
                    .clip_names
                    .iter()
                    .zip(animator.durations.iter())
                    .map(|(name, duration)| {
                        (crate::animation::normalize_clip_name(name), *duration)
                    })
                    .collect()
            })
            .unwrap_or_default();
    } else {
        state.player = None;
        state.player_forward = Vec2::ZERO;
        state.player_hp = None;
        state.player_id = None;
        state.clip_durations.clear();
    }

    // Cena/bioma (`viber.scene_name`/`scene_entry`/`biome`).
    state.scene = res.scene.as_ref().and_then(|s| s.id.clone());
    state.scene_entry = res.scene.as_ref().and_then(|s| s.entry.clone());
    state.biome = res.biome.as_ref().and_then(|b| b.id.clone());

    // Pasta de scripts (`viber.load`): semeada UMA vez no arranque (o
    // anfitrião chama `game::set_scripts_dir` no build); o publisher nunca
    // escreve `None` sobre ela. O static do `game` espelha-se por frame no
    // `publish_globals` do crate — só `Some` escreve.
    state.scripts_dir = None;

    // Esquema `--@export` por path — em modo BMS o [`parse_script_exports`]
    // extrai-o dos assets e o publisher copia-o.
    if let Some(bms) = res.bms_exports.as_deref() {
        state.exports.clone_from(&bms.map);
    } else if !state.exports.is_empty() {
        state.exports = Default::default();
    }

    // Foco de interação por tecla (o `viber.interacted` do BMS só aceita o
    // vencedor) e o gate de `UiInput` focado (teclas são texto).
    state.interaction_focus = res
        .focus
        .as_deref()
        .map(|focus| {
            focus
                .snapshot()
                .into_iter()
                .map(|(key, entity)| (key, entity.to_bits() as i64))
                .collect()
        })
        .unwrap_or_default();
    state.ui_typing = res
        .typing
        .as_deref()
        .is_some_and(|focused| focused.0.is_some());

    // Snapshot de nomes/posições — INCREMENTAL como no `luau_update`
    // (reconstruir 79k nomes por frame custava ~18 ms no simple-rpg) e
    // escrito DIRETO no recurso (sem clones por frame). Os vetores por
    // nome ficam ORDENADOS por bits (ordem estável).
    // Os mapas são `Arc` partilhados com os globals dos scripts: larga a
    // cópia publicada ANTES de os mutar, para o `make_mut` não clonar
    // ~80k entradas por frame (o `publish_globals` volta a partilhá-los já a
    // seguir; os scripts só correm no `Update`). Medido a 2026-11-15: os dois
    // publishers custavam ~37 ms/frame com os clones.
    bevy_ext_luau::release_snapshot_maps();
    let named_entities = std::sync::Arc::make_mut(&mut state.named_entities);
    let positions = std::sync::Arc::make_mut(&mut state.positions);
    let names = std::sync::Arc::make_mut(&mut state.names);
    if !*names_seeded {
        *names_seeded = true;
        queries.names_removed.clear();
        named_entities.clear();
        positions.clear();
        names.clear();
        for (entity, name, transform) in &queries.names {
            let bits = entity.to_bits() as i64;
            named_entities
                .entry(name.to_string())
                .or_default()
                .push(bits);
            positions.insert(bits, transform.translation());
            names.insert(bits, name.to_string());
        }
        for list in named_entities.values_mut() {
            list.sort_unstable();
        }
    } else {
        for entity in queries.names_removed.read() {
            forget_named(named_entities, positions, names, entity.to_bits() as i64);
        }
        for (entity, name, transform) in &queries.names_changed {
            let bits = entity.to_bits() as i64;
            match names.get(&bits) {
                // Só se mexeu (o caso de todos os ossos animados, por frame):
                // a posição chega — sem alocar o nome nem tocar nas listas.
                Some(old) if old == name.as_str() => {
                    positions.insert(bits, transform.translation());
                    continue;
                }
                Some(_) => forget_named(named_entities, positions, names, bits),
                None => {}
            }
            let list = named_entities.entry(name.to_string()).or_default();
            if let Err(at) = list.binary_search(&bits) {
                list.insert(at, bits);
            }
            positions.insert(bits, transform.translation());
            names.insert(bits, name.to_string());
        }
    }

    for (entity, transform) in &queries.unnamed_scripted {
        positions.insert(entity.to_bits() as i64, transform.translation());
    }
    for entity in queries.scripts_removed.read() {
        let bits = entity.to_bits() as i64;
        if !names.contains_key(&bits) {
            positions.remove(&bits);
        }
    }

    // HP por entidade COM vitais (sem o herói) e hostis vivos — o mesmo
    // par de loops do `luau_update` (hostis com HP ≤ 0 saem).
    {
        let hp_snapshot = std::sync::Arc::make_mut(&mut state.hp_snapshot);
        hp_snapshot.clear();
        for (entity, health, _) in &queries.healths {
            hp_snapshot.insert(entity.to_bits() as i64, (health.current, health.max));
        }
    }
    // `snap` empresta OUTRO campo do `state`: o `make_mut` dos hostis empresta
    // `state.hostiles` mut, e campos distintos são empréstimos disjuntos.
    let snap: &HashMap<i64, (f32, f32)> = &state.hp_snapshot;
    let hostiles = std::sync::Arc::make_mut(&mut state.hostiles);
    hostiles.clear();
    hostiles.extend(queries
        .hostiles
        .iter()
        .filter_map(|(entity, script, global)| {
            let Some(&(current, max)) = snap.get(&(entity.to_bits() as i64)) else {
                return None;
            };
            if current <= 0.0 {
                return None;
            }
            let kind = script
                .map(|script| crate::script_kind::script_kind(&script.path))
                .unwrap_or_else(|| "creature".to_string());
            let forward = global.rotation() * Vec3::Z;
            Some(bevy_ext_luau::HostileSnap {
                id: entity.to_bits() as i64,
                kind,
                pos: global.translation(),
                hp: current,
                max,
                forward: Vec2::new(forward.x, forward.z),
            })
        })
        .collect::<Vec<_>>());

    // Props autorais (`viber.nearby`/`viber.prop`) — dados, não regra.
    {
        let props = std::sync::Arc::make_mut(&mut state.props);
        if reseed {
            props.clear();
        }
        for entity in queries.props_removed.read() {
            props.remove(&(entity.to_bits() as i64));
        }
        for (entity, prop) in &queries.script_props {
            if reseed || prop.is_changed() {
                props.insert(entity.to_bits() as i64, prop.0.clone());
            }
        }
    }

    // Remove antes de aplicar mudanças: um componente pode voltar no mesmo frame.
    {
        let groups = std::sync::Arc::make_mut(&mut state.groups);
        if reseed {
            groups.clear();
        }
        for entity in queries.groups_removed.read() {
            groups.remove(&(entity.to_bits() as i64));
        }
        for (entity, member_of) in &queries.groups {
            if reseed || member_of.is_changed() {
                groups.insert(entity.to_bits() as i64, member_of.0.as_ref().clone());
            }
        }
    }

    // Biblioteca de prototypes: imutável pós-arranque — só re-semeia
    // quando muda (ou na 1.ª vez).
    if let Some(lib) = res.prototypes.as_ref()
        && (lib.is_changed() || state.prototype_names.is_empty())
    {
        state.prototype_names = std::sync::Arc::new(lib.0.keys().cloned().collect());
    }

    // Presets de partícula válidos (`viber.burst`).
    if state.particle_presets.is_empty() {
        state.particle_presets = std::sync::Arc::new(
            crate::particles::PRESET_NAMES
                .iter()
                .map(|name| (*name).to_string())
                .collect(),
        );
    }

    // Superfícies: o registo de estradas/água é construído UMA vez (o
    // mundo carvado não muda; troca de cena insere um runtime novo) e o
    // leitor de alturas é recriado por frame — as edições vivas do terreno
    // mutam o overlay e o `ground_below` tem de as ver (o monólito
    // recriava o leitor do ctx todos os frames).
    let rebuild = road_water.is_none() || res.terrain.as_ref().is_some_and(|rt| rt.is_added());
    if rebuild {
        *road_water = res.terrain.as_ref().map(|rt| {
            std::sync::Arc::new(super::ctx::SurfaceRegistries {
                roads: rt.roads.clone(),
                water: rt.water.clone(),
            })
        });
    }
    state.surfaces = road_water.as_ref().zip(res.terrain.as_ref()).map(
        |(registries, rt)| {
            let handle: bevy_ext_luau::SurfaceQueriesHandle = std::sync::Arc::new(BmsSurfaces {
                reader: rt.reader(),
                registries: registries.clone(),
                field_half: rt.spec.world_size * 0.5,
            });
            handle
        },
    );

    // Pose da câmara de jogo (`viber.camera_state`).
    state.camera = res
        .camera_snapshot
        .as_deref()
        .map(|snap| (snap.pos, snap.yaw_deg, snap.pitch_deg));

    // Cinemática a tocar (`viber.cinematic_playing`).
    state.cinematic = res
        .cinematic
        .as_deref()
        .and_then(|c| c.playing().map(str::to_owned));

    // Acessibilidade (`viber.a11y`): o jogo desenha; a engine aplica.
    state.a11y = res
        .a11y
        .as_deref()
        .map(|a11y| {
            (
                a11y.subtitle_scale,
                a11y.shake,
                a11y.hitstop,
                a11y.interact_hold,
            )
        })
        .unwrap_or((1.0, true, true, false));

    // Papel de rede (`viber.net_role`).
    state.net_role = Some(
        match res.net.as_deref().map(|net| &net.role) {
            Some(crate::netplay::NetRole::Host { .. }) => "host",
            Some(crate::netplay::NetRole::Join { .. }) => "join",
            _ => "off",
        }
        .to_string(),
    );

    // Locale: a tabela só clona quando o idioma muda (o monólito semeava
    // uma vez por run — troca em runtime via `SetLocale`).
    if let Some(locale) = res.locale.as_deref()
        && state.locale_lang.as_deref() != Some(locale.lang.as_str())
    {
        state.locale_lang = Some(locale.lang.clone());
        state.locale_table = Some(std::sync::Arc::new(locale.table.clone()));
    }

    // Metadados dos slots (`viber.save_slots`): ler 5 JSONs por frame é
    // lixo — refresca no 1.º frame e a cada 2 s (um save novo aparece no
    // menu de continuar no refresh seguinte).
    let now = time.elapsed_secs_f64();
    if slots_timer.is_none_or(|last| now - last >= 2.0) {
        *slots_timer = Some(now);
        let key = res.save_key.as_deref().and_then(|k| k.0.clone());
        let dir = res.save_dir.as_deref().and_then(|d| d.0.clone());
        state.save_slots = Some(
            crate::save::list_save_slots(key.as_deref(), dir.as_deref())
                .into_iter()
                .map(|slot| {
                    vec![
                        ("slot".to_string(), serde_json::json!(slot.slot)),
                        ("path".to_string(), serde_json::json!(slot.path)),
                        ("exists".to_string(), serde_json::json!(slot.exists)),
                        ("bytes".to_string(), serde_json::json!(slot.bytes)),
                        ("saved_at".to_string(), serde_json::json!(slot.saved_at)),
                        ("level".to_string(), serde_json::json!(slot.level)),
                        ("gold".to_string(), serde_json::json!(slot.gold)),
                        ("scene".to_string(), serde_json::json!(slot.scene)),
                        ("playtime".to_string(), serde_json::json!(slot.playtime)),
                        ("title".to_string(), serde_json::json!(slot.title)),
                    ]
                })
                .collect(),
        );
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    use crate::luau::Groups;
    use bevy_ext_luau::ScriptHostState;

    #[test]
    fn script_metadata_survives_idle_frames_and_tracks_removals() {
        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<ScriptHostState>()
            .add_systems(Update, publish_host_state);
        let props = || ScriptProps(vec![("kind".into(), "wood".into())].into());
        let entity = app.world_mut().spawn((props(), Groups(vec!["item".into()].into()))).id();
        let id = entity.to_bits() as i64;
        app.update();
        app.update();
        let state = app.world().resource::<ScriptHostState>();
        assert_eq!(state.props[&id][0].1, "wood");
        assert_eq!(state.groups[&id], ["item"]);

        app.world_mut().entity_mut(entity).remove::<(Groups, ScriptProps)>()
            .insert(Groups(vec!["prop".into()].into()));
        app.update();
        let state = app.world().resource::<ScriptHostState>();
        assert!(!state.props.contains_key(&id));
        assert_eq!(state.groups[&id], ["prop"]);

        app.world_mut().remove_resource::<ScriptHostState>();
        app.update();
        app.init_resource::<ScriptHostState>();
        app.update();
        assert_eq!(app.world().resource::<ScriptHostState>().groups[&id], ["prop"]);
        app.world_mut().despawn(entity);
        app.update();
        assert!(!app.world().resource::<ScriptHostState>().groups.contains_key(&id));
    }
}

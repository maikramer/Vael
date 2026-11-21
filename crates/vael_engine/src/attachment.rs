//! Anexar props a ossos de um rig — primitiva GENÉRICA.
//!
//! Um jogo de corrida usa isto para o volante, o capacete, uma bandeira; um
//! RPG para a espada na mão. A engine não conhece "arma" nem "ferramenta":
//! só encontra o osso pelo nome, instancia o GLB como filho e troca/remove
//! por SLOT. A regra (qual URL, qual osso, quando) é do script
//! (`vael.attach`/`vael.detach`).

use std::collections::HashMap;

use bevy::prelude::*;

use crate::recipes::spawn::GltfScenePending;

/// Pedido de attach/detach vindo de um script.
#[derive(Debug, Clone, Reflect)]
pub struct AttachmentRequest {
    /// Dono; `None` = herói.
    pub owner: Option<Entity>,
    /// Slot lógico (`"weapon"`, `"helmet"`…): substitui o que lá estiver.
    pub slot: String,
    /// URL do GLB; `None` = detach.
    pub url: Option<String>,
    /// Nome do osso (`"hand_r"`, `"head"`…).
    pub bone: String,
    pub pos: Vec3,
    pub rot: Vec3,
    pub scale: f32,
}

/// Fila de pedidos (o runtime Luau empurra; `apply_attachments` consome).
#[derive(Debug, Default, Resource, Reflect)]
#[reflect(Resource)]
pub struct AttachmentRequests(pub Vec<AttachmentRequest>);

/// `(dono, slot)` → entidade anexada (para trocar/remover sem órfãos).
#[derive(Debug, Default, Resource, Reflect)]
#[reflect(Resource)]
pub struct AttachmentSlots(pub HashMap<(Entity, String), Entity>);

/// Quanto tempo um pedido espera pelo osso (o GLB do dono ainda a carregar)
/// antes de desistir com um aviso.
pub const ATTACH_BONE_WAIT_SECS: f32 = 15.0;

/// Pedidos cujo osso ainda não existe: o glTF do dono (herói, criatura) chega
/// alguns frames — ou segundos, num disco lento — depois do primeiro
/// `vael.attach`. A engine guarda-os e tenta de novo em cada frame, em vez de
/// o script repetir o pedido (cada repetição despachava e recarregava o GLB).
#[derive(Debug, Default, Resource)]
pub struct PendingAttachments(pub Vec<(Entity, AttachmentRequest, f32)>);

/// Grip já pendurado. Se a entidade morre (o rig do herói troca de cena e
/// leva o osso), o pedido volta à fila — o script só pede uma vez.
#[derive(Debug, Default, Resource)]
pub struct MountedAttachments(HashMap<(Entity, String), (Entity, AttachmentRequest)>);

pub struct AttachmentPlugin;

impl Plugin for AttachmentPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AttachmentRequests>()
            .init_resource::<AttachmentSlots>()
            .init_resource::<PendingAttachments>()
            .init_resource::<MountedAttachments>()
            .add_systems(Update, apply_attachments);
    }
}

/// Aplica a fila. Um pedido `url = None` só remove o slot.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn apply_attachments(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    time: Res<Time>,
    mut requests: ResMut<AttachmentRequests>,
    mut pending: ResMut<PendingAttachments>,
    mut slots: ResMut<AttachmentSlots>,
    mut mounted: ResMut<MountedAttachments>,
    players: Query<Entity, With<crate::player::Player>>,
    names: Query<&Name>,
    children: Query<&Children>,
    entities: Query<Entity>,
    // O `WorldInstance` entra quando a cena é *pedida*, frames (ou segundos)
    // antes dos ossos. O relógio dos 15 s só corre depois de haver um nome
    // na subárvore — senão a espada desiste com o herói ainda na fila.
    scene: Query<
        (),
        Or<(
            With<GltfScenePending>,
            With<bevy::world_serialization::WorldAssetRoot>,
            With<bevy::world_serialization::WorldInstance>,
        )>,
    >,
    failed: Query<(), With<crate::gltf_residency::GltfSceneFailed>>,
) {
    let queued = std::mem::take(&mut requests.0);
    let mut revive = Vec::new();
    mounted.0.retain(|(owner, _), (child, request)| {
        if !entities.contains(*owner) {
            return false;
        }
        if entities.contains(*child) {
            return true;
        }
        revive.push((*owner, request.clone()));
        false
    });
    if queued.is_empty() && pending.0.is_empty() && revive.is_empty() {
        return;
    }
    // Donos (ou props) despawnados deixavam a entrada `(dono, slot) → entidade`
    // no mapa PARA SEMPRE — um clone por criatura equipada numa horda. A poda
    // corre só quando há pedidos (nunca por frame) e o `Query<Entity>` só
    // responde a `contains`, sem varrer o mundo.
    slots
        .0
        .retain(|(owner, _), child| entities.contains(*owner) && entities.contains(*child));
    // Pedidos à espera do osso vêm primeiro; um pedido novo para o mesmo
    // `(dono, slot)` substitui o que estava à espera (trocar de arma enquanto
    // o herói carrega não pode anexar a antiga depois da nova).
    let dt = time.delta_secs();
    let mut work: Vec<(Entity, AttachmentRequest, f32)> = Vec::new();
    for (owner, request, age) in std::mem::take(&mut pending.0) {
        if entities.contains(owner) {
            // O `dt` só entra quando o osso falta E o rig já instanciou.
            // Somá-lo aqui fazia a espera correr durante o load do GLB.
            work.push((owner, request, age));
        }
    }
    for request in queued {
        // `iter().next()` e não `single()`: com 2+ players (co-op) o
        // `single()` falhava e TODOS os pedidos sem `id` caíam em silêncio
        // — a espada/Tocha do herói nunca aparecia. O resto da engine já
        // usa o primeiro player nessa situação (`interact`, nav tile).
        let Some(owner) = request.owner.or_else(|| players.iter().next()) else {
            continue;
        };
        work.retain(|(o, r, _)| !(*o == owner && r.slot == request.slot));
        revive.retain(|(o, r)| !(*o == owner && r.slot == request.slot));
        work.push((owner, request, 0.0));
    }
    for (owner, request) in revive {
        work.retain(|(o, r, _)| !(*o == owner && r.slot == request.slot));
        work.push((owner, request, 0.0));
    }
    for (owner, request, age) in work {
        let key = (owner, request.slot.clone());
        // O osso resolve-se ANTES de trocar o slot: falhar o osso não pode
        // deixar o dono sem o attachment ANTIGO — remover antes deixava a
        // troca de "mão para cabeça" (osso inexistente no rig) a despir a
        // tocha viva e não pôr nada em vez dela. Um detach (`url = None`)
        // continua a remover só o slot.
        let Some(url) = request.url.clone() else {
            mounted.0.remove(&key);
            if let Some(old) = slots.0.remove(&key) {
                commands.entity(old).try_despawn();
            }
            continue;
        };
        let Some(bone) = find_bone(owner, &request.bone, &children, &names) else {
            let awaiting_rig = !failed.contains(owner)
                && scene.contains(owner)
                && !descendant_has_name(owner, &children, &names);
            if awaiting_rig || age < ATTACH_BONE_WAIT_SECS {
                let next_age = if awaiting_rig { age } else { age + dt };
                pending.0.push((owner, request, next_age));
            } else {
                warn!(
                    target: "vael::attachment",
                    "osso '{}' não encontrado em {:.0} s — '{}' não anexado (o anterior fica)",
                    request.bone,
                    ATTACH_BONE_WAIT_SECS,
                    url
                );
            }
            continue;
        };
        if let Some(old) = slots.0.remove(&key) {
            commands.entity(old).try_despawn();
        }
        let handle = crate::meshopt::load_gltf(&asset_server, url);
        let transform = Transform {
            translation: request.pos,
            rotation: Quat::from_euler(EulerRot::XYZ, request.rot.x, request.rot.y, request.rot.z),
            scale: Vec3::splat(request.scale.max(0.001)),
        };
        let spawned = commands
            .spawn((
                transform,
                Visibility::Inherited,
                GltfScenePending::single(handle),
            ))
            .id();
        commands.entity(bone).add_child(spawned);
        slots.0.insert(key.clone(), spawned);
        mounted.0.insert(key, (spawned, request));
    }
}

/// A subárvore já tem algum `Name`? O collider do herói não é filho; até o
/// GLB instanciar não há nomes, e o osso da mão ainda não pode falhar.
fn descendant_has_name(root: Entity, children: &Query<&Children>, names: &Query<&Name>) -> bool {
    fn walk(entity: Entity, children: &Query<&Children>, names: &Query<&Name>, depth: u8) -> bool {
        if depth > 24 {
            return false;
        }
        if names.get(entity).is_ok() {
            return true;
        }
        let Ok(kids) = children.get(entity) else {
            return false;
        };
        kids.iter()
            .any(|child| walk(child, children, names, depth + 1))
    }
    let Ok(kids) = children.get(root) else {
        return false;
    };
    kids.iter().any(|child| walk(child, children, names, 0))
}

/// Procura um osso por nome normalizado (exato ou substring), com os alias
/// comuns de mão. Genérico: não assume `hand_r` nem rig humanoide.
/// Público para quem precisa de saber se o osso já existe antes de pedir um
/// attach (o preview do editor espera a hierarquia do GLB carregar).
pub fn find_bone(
    root: Entity,
    wanted: &str,
    children: &Query<&Children>,
    names: &Query<&Name>,
) -> Option<Entity> {
    let wanted = wanted.trim().to_ascii_lowercase();
    let mut aliases = vec![wanted.clone()];
    match wanted.as_str() {
        "hand_r" | "righthand" | "right_hand" => {
            aliases.extend(["righthand".into(), "right_hand".into(), "hand_r".into()]);
        }
        "hand_l" | "lefthand" | "left_hand" => {
            aliases.extend(["lefthand".into(), "left_hand".into(), "hand_l".into()]);
        }
        _ => {}
    }
    fn walk(
        entity: Entity,
        aliases: &[String],
        children: &Query<&Children>,
        names: &Query<&Name>,
    ) -> Option<Entity> {
        if let Ok(name) = names.get(entity) {
            let name = name.as_str().to_ascii_lowercase();
            // "hand_r" não pode casar com "hand_r_index"/"finger".
            let is_finger =
                name.contains("finger") || name.contains("index") && name.contains("hand");
            if !is_finger
                && aliases
                    .iter()
                    .any(|alias| name == *alias || name.contains(alias))
            {
                return Some(entity);
            }
        }
        for child in children.get(entity).ok()?.iter() {
            if let Some(found) = walk(child, aliases, children, names) {
                return Some(found);
            }
        }
        None
    }
    walk(root, &aliases, children, names)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with_owner() -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()));
        app.init_asset::<bevy::gltf::Gltf>();
        app.add_plugins(AttachmentPlugin);
        let owner = app
            .world_mut()
            .spawn((Transform::default(), Name::new("hero")))
            .id();
        (app, owner)
    }

    fn request(owner: Entity, url: &str) -> AttachmentRequest {
        AttachmentRequest {
            owner: Some(owner),
            slot: "weapon".into(),
            url: Some(url.into()),
            bone: "hand_r".into(),
            pos: Vec3::ZERO,
            rot: Vec3::ZERO,
            scale: 1.0,
        }
    }

    #[test]
    fn a_request_waits_for_the_bone_and_keeps_the_old_attachment() {
        let (mut app, owner) = app_with_owner();
        let old = app.world_mut().spawn(Transform::default()).id();
        app.world_mut()
            .resource_mut::<AttachmentSlots>()
            .0
            .insert((owner, "weapon".into()), old);
        app.world_mut()
            .resource_mut::<AttachmentRequests>()
            .0
            .push(request(owner, "sword.glb"));
        app.update();
        // Sem osso: o pedido fica à espera e a arma antiga continua na mão.
        assert_eq!(app.world().resource::<PendingAttachments>().0.len(), 1);
        assert_eq!(
            app.world()
                .resource::<AttachmentSlots>()
                .0
                .get(&(owner, "weapon".into())),
            Some(&old)
        );
        // O rig chega (glTF carregado): o pedido aplica-se sem o script repetir.
        let bone = app.world_mut().spawn(Name::new("hand_r")).id();
        app.world_mut().entity_mut(owner).add_child(bone);
        app.update();
        assert!(app.world().resource::<PendingAttachments>().0.is_empty());
        let now = app.world().resource::<AttachmentSlots>().0[&(owner, "weapon".into())];
        assert_ne!(now, old);
        assert_eq!(
            app.world().get::<ChildOf>(now).map(|c| c.parent()),
            Some(bone)
        );
        assert!(app.world().get_entity(old).is_err());
    }

    #[test]
    fn a_newer_request_replaces_the_one_waiting_for_the_bone() {
        let (mut app, owner) = app_with_owner();
        app.world_mut()
            .resource_mut::<AttachmentRequests>()
            .0
            .push(request(owner, "sword.glb"));
        app.update();
        app.world_mut()
            .resource_mut::<AttachmentRequests>()
            .0
            .push(request(owner, "torch.glb"));
        app.update();
        let pending = &app.world().resource::<PendingAttachments>().0;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].1.url.as_deref(), Some("torch.glb"));
    }

    /// O véu deste mundo passa dos 15 s. Enquanto o GLB do dono não
    /// instancia, a espera não expira — senão a espada desiste em silêncio
    /// e o script não volta a pedir.
    #[test]
    fn the_bone_wait_pauses_while_the_owner_gltf_is_loading() {
        let (mut app, owner) = app_with_owner();
        app.world_mut()
            .entity_mut(owner)
            .insert(crate::recipes::spawn::GltfScenePending::single(Handle::<
                bevy::gltf::Gltf,
            >::default(
            )));
        app.world_mut()
            .resource_mut::<PendingAttachments>()
            .0
            .push((owner, request(owner, "sword.glb"), 100.0));
        app.update();
        let pending = &app.world().resource::<PendingAttachments>().0;
        assert_eq!(pending.len(), 1, "o load do rig não pode fazer desistir");
        assert_eq!(
            pending[0].2, 100.0,
            "a espera não avança enquanto o GLB carrega"
        );
    }

    /// Trocar a cena do rig despawna o osso e a espada que era filha dele.
    /// O script já pediu uma vez: a engine volta a pendurar sozinha.
    #[test]
    fn a_despawned_grip_is_attached_again() {
        let (mut app, owner) = app_with_owner();
        let bone = app.world_mut().spawn(Name::new("hand_r")).id();
        app.world_mut().entity_mut(owner).add_child(bone);
        app.world_mut()
            .resource_mut::<AttachmentRequests>()
            .0
            .push(request(owner, "sword.glb"));
        app.update();
        let first = app.world().resource::<AttachmentSlots>().0[&(owner, "weapon".into())];
        app.world_mut().entity_mut(first).despawn();
        app.update();
        let second = app.world().resource::<AttachmentSlots>().0[&(owner, "weapon".into())];
        assert_ne!(first, second);
        assert_eq!(
            app.world()
                .get::<ChildOf>(second)
                .map(|child| child.parent()),
            Some(bone)
        );
    }

    /// A cena já foi pedida (`WorldAssetRoot`) mas os ossos ainda não
    /// existem. Os 15 s não correm — o arranque deste mundo passa disso.
    #[test]
    fn the_bone_wait_pauses_until_the_scene_has_names() {
        let (mut app, owner) = app_with_owner();
        app.world_mut()
            .entity_mut(owner)
            .insert(bevy::world_serialization::WorldAssetRoot(Handle::<
                bevy::world_serialization::WorldAsset,
            >::default(
            )));
        app.world_mut()
            .resource_mut::<PendingAttachments>()
            .0
            .push((owner, request(owner, "sword.glb"), 100.0));
        app.update();
        let pending = &app.world().resource::<PendingAttachments>().0;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].2, 100.0);
    }

    /// Rig já na hierarquia, sem `hand_r`: aí sim os 15 s contam e expiram.
    #[test]
    fn a_spawned_rig_without_the_bone_expires() {
        let (mut app, owner) = app_with_owner();
        app.world_mut()
            .entity_mut(owner)
            .insert(bevy::world_serialization::WorldAssetRoot(Handle::<
                bevy::world_serialization::WorldAsset,
            >::default(
            )));
        let head = app.world_mut().spawn(Name::new("head")).id();
        app.world_mut().entity_mut(owner).add_child(head);
        app.world_mut()
            .resource_mut::<PendingAttachments>()
            .0
            .push((owner, request(owner, "sword.glb"), 100.0));
        app.update();
        assert!(app.world().resource::<PendingAttachments>().0.is_empty());
    }
}

//! Edições do Jackdaw no jogo a correr, para lá do `Transform`.
//!
//! No Play, o editor manda cada componente mudado de um nó
//! (`ControlEvent::NodeComponents`) e o `node_sync` do `jackdaw_runtime`
//! insere-o, por reflexão, nas entidades com o `SceneNodeId` desse nó
//! ([`crate::editor_link`]). O `Transform` basta inserir; o resto
//! (`WorldNode`, specs tipados do `bevy_ext_scene`, `CustomProperties`) só
//! tem efeito quando a engine volta a montar o nó a partir dos atributos:
//!
//! - **só props** (`CustomProperties`): patch do `ScriptProps` no sítio — o
//!   script continua com o seu estado (afinar o jogo vivo);
//! - **o resto**: os atributos são recompostos
//!   (`bevy_ext_world::bsn::specs::live_attrs`: os autorados que vieram do
//!   editor, os derivados do load, os specs e a pose actual) e o nó é
//!   re-spawnado pela engine (`recipes::parse_world` + `spawn_specs`), debaixo
//!   do mesmo pai. A nova entidade recebe outra vez o `SceneNodeId`.
//!
//! **Estrutura.** Apagar e mover nós já chega pelo `jackdaw_runtime` (as
//! nossas entidades não pertencem a nenhuma `JackdawSceneRoot`, e essas
//! passam o filtro de cena). Um nó **novo** debaixo de um nó nosso chega
//! como entidade "crua" (os componentes do documento, sem o que a engine
//! monta): [`adopt_new_nodes`] monta-a pela engine no mesmo pai, com uma
//! origem sintética, e passa-lhe o `SceneNodeId`.
//!
//! Um nó com filhos no mesmo ficheiro re-spawna com a subárvore registada
//! no load. Não muda a quente: filhos vindos de outro ficheiro (prefab) e
//! tags que a engine só monta no load (terreno, céu, UI…) — fica um aviso.
//! Os specs espelhados também notificam remoção.

use std::sync::Arc;

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy_ext_scene::{
    BodySpec, ColliderSpec, Destructible, FenceSpec, FloraStation, Harvestable, ModelLod, NodeSpecs,
    ParticleSpec, PortalSpec, ScriptRef, SpawnExclusionSpec,
};
use bevy_ext_world::bsn::WorldNode;
use bevy_ext_world::bsn::specs;
use jackdaw_scene_types::{CustomProperties, SceneNodeId};
use vael_engine::scene::SceneSource;

/// A entidade tem os specs/props do nó como componentes (espelho do load):
/// os componentes passam a ser a verdade, e removê-los conta.
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct SpecsMirrored;

/// Põe na entidade os specs e as props do nó de onde veio.
pub fn mirror_specs(
    commands: &mut Commands,
    entity: Entity,
    origin: &bevy_ext_world::xml::NodeOrigin,
) {
    let specs = specs::specs_for(origin);
    let mut target = commands.entity(entity);
    target.try_insert(SpecsMirrored);
    let Some(specs) = specs else {
        return;
    };
    macro_rules! put {
        ($field:ident) => {
            if let Some(value) = specs.$field.clone() {
                target.try_insert(value);
            }
        };
    }
    put!(collider);
    put!(body);
    put!(destructible);
    put!(harvest);
    put!(lod);
    put!(flora);
    put!(script);
    put!(particle);
    put!(exclusion);
    put!(portal);
    put!(camera);
    put!(interior);
    put!(mixer);
    put!(music_layer);
    put!(biome);
    put!(fence);
    if let Some(props) = &specs.props {
        let mut custom = CustomProperties::default();
        for prop in &props.props {
            custom.properties.insert(
                prop.key.clone(),
                jackdaw_scene_types::PropertyValue::String(prop.value.clone().into()),
            );
        }
        target.try_insert(custom);
    }
}

/// Entidades com edições por aplicar; `true` = só props mudaram.
#[derive(Resource, Default, Debug)]
pub struct PendingLiveEdits(pub HashMap<Entity, bool>);

pub struct VaelLiveEditPlugin;

impl Plugin for VaelLiveEditPlugin {
    fn build(&self, app: &mut App) {
        if !crate::editor_link::link_enabled() {
            return;
        }
        // O `node_sync` só insere tipos que o registo do jogo conhece.
        if !app.is_plugin_added::<bevy_ext_scene::SceneSpecPlugin>() {
            app.add_plugins(bevy_ext_scene::SceneSpecPlugin);
        }
        app.register_type::<WorldNode>()
            .register_type::<bevy_ext_world::bsn::WorldProp>()
            .init_resource::<PendingLiveEdits>();
        watch_edits::<WorldNode, false>(app);
        watch_edits::<ColliderSpec, false>(app);
        watch_edits::<BodySpec, false>(app);
        watch_edits::<Destructible, false>(app);
        watch_edits::<Harvestable, false>(app);
        watch_edits::<ModelLod, false>(app);
        watch_edits::<FloraStation, false>(app);
        watch_edits::<ScriptRef, false>(app);
        watch_edits::<ParticleSpec, false>(app);
        watch_edits::<SpawnExclusionSpec, false>(app);
        watch_edits::<PortalSpec, false>(app);
        watch_edits::<bevy_ext_scene::CameraSpec, false>(app);
        watch_edits::<bevy_ext_scene::InteriorSceneSpec, false>(app);
        watch_edits::<bevy_ext_scene::AudioMixerSpec, false>(app);
        watch_edits::<bevy_ext_scene::MusicLayerSpec, false>(app);
        watch_edits::<bevy_ext_scene::BiomeRegionSpec, false>(app);
        // A cerca nasce da linha (`fence-points`): mover um ponto no editor
        // refaz postes, travessas e tábuas.
        watch_edits::<FenceSpec, false>(app);
        watch_edits::<CustomProperties, true>(app);
        // Depois do `node_sync` (PreUpdate) e antes da física do frame.
        app.add_systems(Update, (adopt_new_nodes, apply_live_edits).chain());
    }
}

// Jackdaw updates mutable reflected components in place. Insert observers alone
// only catch the first edit on an entity that did not already carry the spec.
fn watch_edits<T: Component, const PROPS_ONLY: bool>(app: &mut App) {
    app.add_observer(mark::<T, PROPS_ONLY>)
        .add_observer(mark_removed::<T, PROPS_ONLY>)
        .add_systems(
            Update,
            mark_changed::<T, PROPS_ONLY>.before(apply_live_edits),
        );
}

fn mark_changed<T: Component, const PROPS_ONLY: bool>(
    changed: Query<(Entity, Ref<T>), (Changed<T>, With<SceneNodeId>, With<SceneSource>)>,
    mut pending: ResMut<PendingLiveEdits>,
) {
    for (entity, value) in &changed {
        // Initial mirroring happens before the node is tagged. Rebuilding that
        // initial value would re-spawn it and mirror it again on the next frame.
        if value.last_changed() != value.added() {
            *pending.0.entry(entity).or_insert(PROPS_ONLY) &= PROPS_ONLY;
        }
    }
}

fn mark<T: Component, const PROPS_ONLY: bool>(
    insert: On<Insert<T>>,
    tagged: Query<(), (With<SceneNodeId>, With<SceneSource>)>,
    mut pending: ResMut<PendingLiveEdits>,
) {
    if !tagged.contains(insert.entity) {
        return;
    }
    let only = pending.0.entry(insert.entity).or_insert(PROPS_ONLY);
    *only &= PROPS_ONLY;
}

fn mark_removed<T: Component, const PROPS_ONLY: bool>(
    remove: On<Remove<T>>,
    tagged: Query<(), (With<SceneNodeId>, With<SceneSource>, With<SpecsMirrored>)>,
    mut pending: ResMut<PendingLiveEdits>,
) {
    if !tagged.contains(remove.entity) {
        return;
    }
    let only = pending.0.entry(remove.entity).or_insert(PROPS_ONLY);
    *only &= PROPS_ONLY;
}

/// Os specs do nó: os do load, substituídos pelos que estão na entidade
/// (com o espelho, só os da entidade — um removido já não conta).
fn current_specs(
    world: &World,
    entity: Entity,
    origin: &bevy_ext_world::xml::NodeOrigin,
) -> NodeSpecs {
    let get = |e: Entity| world.get_entity(e).ok();
    let mirrored = get(entity).is_some_and(|e| e.contains::<SpecsMirrored>());
    let mut out = if mirrored {
        NodeSpecs::default()
    } else {
        specs::specs_for(origin)
            .map(|specs| (*specs).clone())
            .unwrap_or_default()
    };
    let Some(entity) = get(entity) else {
        return out;
    };
    macro_rules! take {
        ($field:ident, $ty:ty) => {
            if let Some(value) = entity.get::<$ty>() {
                out.$field = Some(value.clone());
            }
        };
    }
    take!(collider, ColliderSpec);
    take!(body, BodySpec);
    take!(destructible, Destructible);
    take!(harvest, Harvestable);
    take!(lod, ModelLod);
    take!(flora, FloraStation);
    take!(fence, FenceSpec);
    take!(script, ScriptRef);
    take!(particle, ParticleSpec);
    take!(exclusion, SpawnExclusionSpec);
    take!(portal, PortalSpec);
    take!(camera, bevy_ext_scene::CameraSpec);
    take!(interior, bevy_ext_scene::InteriorSceneSpec);
    take!(mixer, bevy_ext_scene::AudioMixerSpec);
    take!(music_layer, bevy_ext_scene::MusicLayerSpec);
    take!(biome, bevy_ext_scene::BiomeRegionSpec);
    if let Some(custom) = entity.get::<CustomProperties>() {
        out.props = specs::merged_props(None, Some(custom.clone()));
    }
    out
}

/// `prop-*` + os dados de colheita que o spawn também põe no `ScriptProps`
/// (repor só as props apagava-os e a árvore deixava de ser colhível).
pub fn script_props(attrs: &[(String, String)]) -> Vec<(String, String)> {
    attrs
        .iter()
        .filter_map(|(key, value)| {
            if let Some(name) = key.strip_prefix("prop-") {
                return Some((name.to_string(), value.clone()));
            }
            matches!(
                key.as_str(),
                "destructible" | "resource-kind" | "resource-yield"
            )
            .then(|| (key.clone(), value.clone()))
        })
        .collect()
}

/// Origem de um nó criado no editor durante o Play (não existe no disco).
pub fn play_origin(id: u64) -> Arc<bevy_ext_world::xml::NodeOrigin> {
    Arc::new(bevy_ext_world::xml::NodeOrigin {
        file: Arc::from(std::path::Path::new("jackdaw-play")),
        path: vec![(id >> 32) as u32, id as u32],
    })
}

/// Nós novos do editor (entidades cruas: `WorldNode` sem `SceneSource`)
/// montados pela engine. Um de cada vez de cima para baixo: os filhos crus
/// passam para a entidade montada e são adoptados nos frames seguintes.
pub fn adopt_new_nodes(world: &mut World) {
    let mut raw =
        world.query_filtered::<(Entity, &SceneNodeId, &WorldNode), Without<SceneSource>>();
    let candidates: Vec<(Entity, u64, WorldNode)> = raw
        .iter(world)
        .map(|(entity, id, node)| (entity, id.0, node.clone()))
        .collect();
    for (entity, id, node) in candidates {
        // Pai ainda cru: espera que ele seja adoptado primeiro.
        let parent = world.get::<ChildOf>(entity).map(ChildOf::parent);
        if parent.is_some_and(|parent| {
            world.get::<WorldNode>(parent).is_some() && world.get::<SceneSource>(parent).is_none()
        }) {
            continue;
        }
        if let Err(reason) = adopt_one(world, entity, id, node, parent) {
            warn!("vael: nó novo do Jackdaw não montado — {reason}");
            // Não volta a tentar todos os frames.
            world.entity_mut(entity).remove::<WorldNode>();
        }
    }
}

fn adopt_one(
    world: &mut World,
    entity: Entity,
    id: u64,
    node: WorldNode,
    parent: Option<Entity>,
) -> Result<(), String> {
    let origin = play_origin(id);
    let raw: Vec<(String, String)> = node
        .attrs
        .iter()
        .map(|prop| (prop.key.clone(), prop.value.clone()))
        .collect();
    let mut attrs = raw.clone();
    if let Some(name) = world.get::<Name>(entity)
        && !attrs.iter().any(|(k, _)| k == "name")
    {
        attrs.push(("name".to_string(), name.as_str().to_string()));
    }
    let recorded = specs::RecordedNode {
        tag: node.tag.clone(),
        raw_attrs: raw,
        attrs: attrs.clone(),
        text: node.text.clone(),
        has_children: false,
    };
    let pose = world
        .get::<Transform>(entity)
        .map(|t| bevy_ext_world::bsn::Pose {
            translation: t.translation,
            rotation: t.rotation,
            scale: t.scale,
        });
    let node_specs = current_specs(world, entity, &origin);
    let attrs = specs::live_attrs(&recorded, None, &node_specs, pose);
    let xml = bevy_ext_world::xml::XmlNode {
        tag: node.tag.clone(),
        attrs: attrs.clone(),
        text: node.text.clone(),
        children: Vec::new(),
        origin: Some(origin.clone()),
    };
    let parsed = vael_engine::recipes::parse_world(&[], std::slice::from_ref(&xml))
        .map_err(|err| format!("<{}>: {err:#}", node.tag))?;
    let Some(spec) = parsed.entities.first() else {
        return Err(format!("<{}> não gera entidade", node.tag));
    };
    if !vael_engine::recipes::spawn::spawns_incrementally(spec) {
        return Err(format!("<{}> só nasce ao recarregar o mundo", node.tag));
    }
    // Regista antes de montar: o observer da `SceneSource` põe o id.
    specs::record_node(&origin, specs::RecordedNode { attrs, ..recorded });
    specs::set_node_specs(&origin, node_specs);
    specs::record_node_id_pub(&origin, id);
    let spawned = vael_engine::recipes::spawn::spawn_specs(world, &parsed.entities, parent)
        .map_err(|err| format!("<{}>: {err}", node.tag))?;
    let Some(&adopted) = spawned.first() else {
        return Ok(());
    };
    // Os filhos crus mudam-se para a entidade montada; o cru sai.
    let kids: Vec<Entity> = world
        .get::<Children>(entity)
        .map(|kids| kids.iter().collect())
        .unwrap_or_default();
    for kid in kids {
        world.entity_mut(kid).insert(ChildOf(adopted));
    }
    world.entity_mut(entity).despawn();
    world.entity_mut(adopted).insert(SceneNodeId(id));
    info!(
        "vael: nó novo <{}> do Jackdaw montado pela engine",
        node.tag
    );
    Ok(())
}

pub fn apply_live_edits(world: &mut World) {
    let pending = std::mem::take(&mut world.resource_mut::<PendingLiveEdits>().0);
    for (entity, props_only) in pending {
        if let Err(reason) = apply_one(world, entity, props_only) {
            warn!("vael: edição ao vivo não aplicada — {reason}");
        }
    }
}

fn apply_one(world: &mut World, entity: Entity, props_only: bool) -> Result<(), String> {
    let Ok(view) = world.get_entity(entity) else {
        return Ok(()); // já re-spawnado neste frame
    };
    let Some(source) = view.get::<SceneSource>().map(|s| s.0.clone()) else {
        return Ok(());
    };
    let name = view
        .get::<Name>()
        .map(|n| n.as_str().to_string())
        .unwrap_or_else(|| format!("{entity}"));
    let Some(recorded) = specs::node_for(&source) else {
        return Err(format!("{name}: nó sem registo do load"));
    };
    let parent = view.get::<ChildOf>().map(ChildOf::parent);
    let pose = view.get::<Transform>().map(|t| bevy_ext_world::bsn::Pose {
        translation: t.translation,
        rotation: t.rotation,
        scale: t.scale,
    });
    let world_node = view.get::<WorldNode>().cloned();
    let raw: Option<Vec<(String, String)>> = world_node.as_ref().map(|node| {
        node.attrs
            .iter()
            .map(|prop| (prop.key.clone(), prop.value.clone()))
            .collect()
    });
    let node_specs = current_specs(world, entity, &source);
    let attrs = specs::live_attrs(&recorded, raw.as_deref(), &node_specs, pose);

    if props_only {
        let props = script_props(&attrs);
        let mut target = world.entity_mut(entity);
        if props.is_empty() {
            target.remove::<vael_engine::luau::ScriptProps>();
        } else {
            target.insert(vael_engine::luau::ScriptProps(Arc::new(props)));
        }
        specs::set_node_specs(&source, node_specs);
        info!("vael: props de `{name}` actualizadas ao vivo (Jackdaw)");
        return Ok(());
    }

    // Nó com filhos: a subárvore vem do registo do load (os filhos ficam
    // como estavam no disco, com as edições que já tinham recebido).
    let children = if recorded.has_children {
        match specs::recorded_subtree(&source) {
            Some(kids) if !kids.is_empty() => kids,
            _ => {
                return Err(format!(
                    "{name}: os filhos vêm de outro ficheiro (prefab) — muda ao recarregar o mundo"
                ));
            }
        }
    } else {
        Vec::new()
    };
    let has_children = !children.is_empty();
    let tag = world_node
        .as_ref()
        .map(|node| node.tag.clone())
        .unwrap_or_else(|| recorded.tag.clone());
    let text = world_node
        .as_ref()
        .map(|node| node.text.clone())
        .unwrap_or_else(|| recorded.text.clone());
    let xml = bevy_ext_world::xml::XmlNode {
        tag: tag.clone(),
        attrs: attrs.clone(),
        text: text.clone(),
        children,
        origin: Some(source.clone()),
    };
    let parsed = vael_engine::recipes::parse_world(&[], std::slice::from_ref(&xml))
        .map_err(|err| format!("{name}: {err:#}"))?;
    if parsed.entities.is_empty() {
        return Ok(());
    }
    if let Some(bad) = parsed
        .entities
        .iter()
        .find(|spec| !vael_engine::recipes::spawn::spawns_incrementally(spec))
    {
        return Err(format!(
            "{name}: <{}> só muda ao recarregar o mundo",
            bad.tag.as_deref().unwrap_or(&tag)
        ));
    }
    // Sai tudo o que nasceu deste nó e dos descendentes (mesma origem).
    let mut owned = world.query::<(Entity, &SceneSource)>();
    let doomed: Vec<Entity> = owned
        .iter(world)
        .filter(|(_, other)| other.0.file == source.file && other.0.path.starts_with(&source.path))
        .map(|(e, _)| e)
        .collect();
    for doomed in doomed {
        if let Ok(e) = world.get_entity_mut(doomed) {
            e.despawn();
        }
    }
    // Registo ANTES de montar: o espelho da entidade nova lê os specs
    // daqui (com o registo velho, um spec removido voltava).
    specs::set_node_specs(&source, node_specs);
    specs::record_node(
        &source,
        specs::RecordedNode {
            tag,
            raw_attrs: raw.unwrap_or_else(|| recorded.raw_attrs.clone()),
            attrs,
            text,
            has_children,
        },
    );
    vael_engine::recipes::spawn::spawn_specs(world, &parsed.entities, parent)
        .map_err(|err| format!("{name}: {err}"))?;
    info!("vael: `{name}` re-spawnado com a edição do Jackdaw");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn props_de_script_guardam_os_dados_de_colheita() {
        let attrs = vec![
            ("prop-cinematic".to_string(), "x".to_string()),
            ("resource-kind".to_string(), "wood".to_string()),
            ("url".to_string(), "/a.glb".to_string()),
        ];
        assert_eq!(
            script_props(&attrs),
            vec![
                ("cinematic".to_string(), "x".to_string()),
                ("resource-kind".to_string(), "wood".to_string()),
            ]
        );
    }

    #[test]
    fn jackdaw_pie_detects_repeated_in_place_edits_to_mirrored_specs() {
        use jackdaw_runtime::node_sync::{apply_node_components, install_node_index};
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(bevy_ext_scene::SceneSpecPlugin)
            .init_resource::<PendingLiveEdits>();
        install_node_index(&mut app);
        watch_edits::<ScriptRef, false>(&mut app);
        let entity = app
            .world_mut()
            .spawn((SceneSource(default()), ScriptRef::default()))
            .id();
        app.world_mut().entity_mut(entity).insert(SceneNodeId(44));
        app.update();
        assert!(app.world().resource::<PendingLiveEdits>().0.is_empty());

        for path in ["first.lua", "next.lua"] {
            apply_node_components(
                app.world_mut(),
                44,
                &[(
                    ScriptRef::type_path().into(),
                    serde_json::json!({ "path": path }),
                )],
                &[],
            );
            app.update();
            assert_eq!(app.world().get::<ScriptRef>(entity).unwrap().path, path);
            assert_eq!(
                app.world_mut()
                    .resource_mut::<PendingLiveEdits>()
                    .0
                    .remove(&entity),
                Some(false)
            );
            app.update();
            assert!(app.world().resource::<PendingLiveEdits>().0.is_empty());
        }
    }

    #[test]
    fn remover_um_spec_espelhado_pede_re_spawn() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<PendingLiveEdits>()
            .add_observer(mark::<ColliderSpec, false>)
            .add_observer(mark_removed::<ColliderSpec, false>);
        let origin = Arc::new(bevy_ext_world::xml::NodeOrigin::default());
        // Espelho antes do id: não marca nada.
        let e = app
            .world_mut()
            .spawn((SceneSource(origin), SpecsMirrored, ColliderSpec::default()))
            .id();
        app.world_mut().entity_mut(e).insert(SceneNodeId(1 << 33));
        assert!(app.world().resource::<PendingLiveEdits>().0.is_empty());
        app.world_mut().entity_mut(e).remove::<ColliderSpec>();
        assert_eq!(
            app.world().resource::<PendingLiveEdits>().0.get(&e),
            Some(&false)
        );
    }

    #[test]
    fn insercao_em_entidade_sem_id_e_ignorada() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<PendingLiveEdits>()
            .add_observer(mark::<ScriptRef, false>)
            .add_observer(mark::<CustomProperties, true>);
        let origin = Arc::new(bevy_ext_world::xml::NodeOrigin::default());
        let loose = app.world_mut().spawn(SceneSource(origin.clone())).id();
        let tagged = app
            .world_mut()
            .spawn((SceneSource(origin), SceneNodeId(1 << 33)))
            .id();
        app.world_mut()
            .entity_mut(loose)
            .insert(ScriptRef::default());
        app.world_mut()
            .entity_mut(tagged)
            .insert(CustomProperties::default());
        let pending = &app.world().resource::<PendingLiveEdits>().0;
        assert_eq!(pending.get(&tagged), Some(&true), "só props");
        assert!(!pending.contains_key(&loose));
        // Um spec a seguir no mesmo frame faz dele um re-spawn.
        app.world_mut()
            .entity_mut(tagged)
            .insert(ScriptRef::default());
        assert_eq!(
            app.world().resource::<PendingLiveEdits>().0.get(&tagged),
            Some(&false)
        );
    }
}

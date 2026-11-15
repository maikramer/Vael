//! Ligação ao Play do Jackdaw: edições do editor no jogo a correr.
//!
//! O editor endereça cada edição pelo `SceneNodeId` do nó
//! (`ControlEvent::NodeComponents`) e o `jackdaw_runtime` aplica-a, por
//! reflexão, às entidades que têm esse id (`node_sync::SceneNodeIndex`). O
//! mundo do Vael é montado pelo spawner da engine, que não sabe nada de ids:
//! este módulo põe o `SceneNodeId` em cada entidade cuja `SceneSource` (a
//! origem do nó no documento) o `bevy_ext_world` registou ao ler o `.bsn`.
//!
//! O que chega hoje: o `Transform` de qualquer nó com id (mover/rodar/escalar
//! no viewport move a coisa no jogo) e os componentes reflectidos que o jogo
//! conhece. Atributos do DSL (`WorldNode`) e specs tipados não re-spawnam o
//! nó — isso é o `scene::sync` da engine, ainda desligado para `.bsn`.
//!
//! A etiquetagem com ids só corre sob o Play (`JACKDAW_PIE`): fora dele
//! ninguém os lê.
//!
//! **Componentes nativos.** O loader traduz o `.bsn` para o DSL da engine e
//! o DSL só conhece os seus atributos; tudo o resto que se autora no editor
//! (`AnimationSet`, camera rig, binds de UI, componentes `vael::…`) ficava
//! pelo caminho. O `bevy_ext_world` guarda esses patches por nó e
//! [`apply_native_components`] aplica-os à entidade quando ela nasce (sempre,
//! não só no Play), para os tipos registados cujo crate está em
//! [`NATIVE_ALLOW`] (mais `VAEL_NATIVE_ALLOW=prefixo,…`).

use bevy::prelude::*;
use jackdaw_scene_types::SceneNodeId;
use viber::scene::SceneSource;

pub struct VaelEditorLinkPlugin;

/// Crates cujos componentes autorados passam para o jogo.
pub const NATIVE_ALLOW: &[&str] = &[
    "vael::",
    "jackdaw_animation_runtime::",
    "jackdaw_camera_rig::",
    "jackdaw_bind::",
    "jackdaw_widgets_runtime::",
];

impl Plugin for VaelEditorLinkPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(apply_native_components);
        if !link_enabled() {
            return;
        }
        app.add_observer(tag_scene_node);
    }
}

/// Play do Jackdaw (`JACKDAW_PIE`) ou `VAEL_EDITOR_LINK=1` (testes/QA): as
/// entidades levam o `SceneNodeId` e as edições ao vivo aplicam-se.
pub fn link_enabled() -> bool {
    std::env::var_os("JACKDAW_PIE").is_some() || forced()
}

/// `VAEL_EDITOR_LINK=1` liga a etiquetagem fora do Play (testes/QA).
fn forced() -> bool {
    matches!(
        std::env::var("VAEL_EDITOR_LINK").ok().as_deref(),
        Some("1" | "true" | "on")
    )
}

fn tag_scene_node(
    add: On<Add<SceneSource>>,
    sources: Query<&SceneSource, Without<SceneNodeId>>,
    mut commands: Commands,
) {
    let Ok(source) = sources.get(add.entity) else {
        return;
    };
    if let Some(id) = bevy_ext_world::bsn::specs::scene_node_id_for(&source.0) {
        // Primeiro os specs/props do nó como componentes (para uma remoção
        // no editor ter o que remover — `live_edit`), depois o id: os
        // observers de edição só reagem a entidades com id, por isso este
        // espelho inicial não dispara re-spawns.
        crate::live_edit::mirror_specs(&mut commands, add.entity, &source.0);
        commands.entity(add.entity).try_insert(SceneNodeId(id));
    }
}

fn allowed(type_path: &str) -> bool {
    NATIVE_ALLOW
        .iter()
        .any(|prefix| type_path.starts_with(prefix))
        || std::env::var("VAEL_NATIVE_ALLOW").is_ok_and(|extra| {
            extra
                .split(',')
                .map(str::trim)
                .any(|prefix| !prefix.is_empty() && type_path.starts_with(prefix))
        })
}

fn apply_native_components(
    add: On<Add<SceneSource>>,
    sources: Query<&SceneSource>,
    mut commands: Commands,
) {
    let Ok(source) = sources.get(add.entity) else {
        return;
    };
    let Some(patches) = bevy_ext_world::bsn::specs::native_components_for(&source.0) else {
        return;
    };
    let entity = add.entity;
    commands.queue(move |world: &mut World| {
        if world.get_entity(entity).is_err() {
            return;
        }
        for patch in patches.iter() {
            let Some(path) = jackdaw_bsn::patch_type_path(patch) else {
                continue;
            };
            if !allowed(path) {
                continue;
            }
            let registered = world
                .resource::<AppTypeRegistry>()
                .read()
                .get_with_type_path(path)
                .is_some_and(|reg| reg.data::<bevy::ecs::reflect::ReflectComponent>().is_some());
            if !registered {
                warn_once!("vael: componente autorado `{path}` não está registado no jogo");
                continue;
            }
            jackdaw_bsn::apply_component_patch(world, entity, patch);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sem_registo_nao_etiqueta() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_observer(tag_scene_node);
        let origin = bevy_ext_world::xml::NodeOrigin::default();
        let entity = app
            .world_mut()
            .spawn(SceneSource(std::sync::Arc::new(origin)))
            .id();
        app.update();
        assert!(app.world().get::<SceneNodeId>(entity).is_none());
    }

    #[derive(Component, Reflect, Default, Debug, PartialEq)]
    #[reflect(Component, Default)]
    struct Marca {
        forca: f32,
    }

    #[test]
    fn componente_autorado_chega_a_entidade() {
        let dir = std::env::temp_dir().join(format!("vael_native_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.yaml"), "assets:\n  roots: assets\n").unwrap();
        let path = dir.join("world.bsn");
        std::fs::write(
            &path,
            r#"#"world"
bevy_world_xml::bsn::WorldNode {
    tag: "world",
    attrs: [],
    text: "",
}
bevy_ecs::hierarchy::Children [
    #"poco"
    bevy_world_xml::bsn::WorldNode {
        tag: "Entity",
        attrs: [],
        text: "",
    }
    vael::editor_link::tests::Marca { forca: 2.5 }
    bevy_render::view::visibility::Visibility::Hidden
]
"#,
        )
        .unwrap();
        let loaded = bevy_ext_world::bsn::load::load_bsn_world(&path).expect("carrega");
        let origin = loaded
            .nodes
            .iter()
            .find(|n| n.tag == "Entity")
            .and_then(|n| n.origin.clone())
            .expect("origem");

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .register_type::<Marca>()
            .add_observer(apply_native_components);
        let entity = app.world_mut().spawn(SceneSource(origin)).id();
        app.update();
        assert_eq!(
            app.world().get::<Marca>(entity),
            Some(&Marca { forca: 2.5 })
        );
        // Fora da lista (render da engine): não é aplicado.
        assert!(app.world().get::<Visibility>(entity).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn so_crates_permitidos_passam() {
        assert!(allowed("jackdaw_animation_runtime::AnimationSet"));
        assert!(allowed("vael::player::VaelController"));
        assert!(!allowed("bevy_render::view::Visibility"));
    }
}

//! Definições do Vael autoradas na cena (`world/settings/*.bsn`).
//!
//! O ambiente (`ElementsSettings`), o pós-processamento (`PostFxSettings`)
//! e a flora (`FloraSpecies`/`FloraSettings`) são componentes `Reflect` em
//! entidades do `.bsn`: o Jackdaw edita-os no inspector (e nos painéis da
//! extensão `vael_jackdaw`) e o jogo lê-os daqui. Gravar no editor chega ao
//! jogo que está a correr: o `SceneSettingsPlugin` relê os documentos.
//!
//! Precedência: cena > `config.yaml`/constantes; as variáveis `VAEL_START_*`
//! e `VIBER_NO_*` continuam a ganhar (QA).

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use bevy::prelude::{
    App, Commands, Component, Entity, FromReflect, In, IntoSystem, Local, Plugin, PreUpdate, Query,
    Reflect, Res, ResMut, Resource, Transform, TypePath, default,
};
use bevy::reflect::GetTypeRegistration;
use bevy::reflect::PartialReflect;
use bevy::reflect::TypeRegistry;
use bevy_ext_elements::{ElementsSettings, ElementsSettingsPlugin};
use bevy_ext_flora::{FloraSettings, FloraSettingsPlugin, FloraSpecies};
use bevy_ext_grass::{GrassSettingsSpec, GrassSettingsSpecPlugin, GrassStyleSpec};
use bevy_ext_postfx::{PostFxSettings, PostFxSettingsPlugin, SsaoQuality};
use bevy_ext_world::bsn::settings::{
    SceneNatives, SceneSettingsApp, SceneSettingsPlugin, emit_native_entity, emit_native_group,
    native_value,
};
use jackdaw_scene_types::SceneNodeId;

use crate::VaelProjectRoot;

pub use bevy_ext_world::bsn::settings::{SceneSetting, SceneSettingList};

/// Pasta dos documentos de definições, relativa à raiz do projeto.
pub const SETTINGS_DIR: &str = "world/settings";

/// O mundo de onde vêm as definições: sempre o principal do projeto (o menu
/// e as salas usam as mesmas).
pub fn settings_world(root: &Path) -> PathBuf {
    root.join("world.bsn")
}

/// Lê as definições da cena e mantém-nas atualizadas.
pub struct VaelSettingsPlugin;

impl Plugin for VaelSettingsPlugin {
    fn build(&self, app: &mut App) {
        let root = app
            .world()
            .get_resource::<VaelProjectRoot>()
            .map(|root| root.0.clone())
            .unwrap_or_default();
        app.add_plugins((
            ElementsSettingsPlugin,
            PostFxSettingsPlugin,
            FloraSettingsPlugin,
            GrassSettingsSpecPlugin,
            // Colisor/corpo/destrutível/colheita tipados nos `.bsn`: o
            // schema do Jackdaw tem de os conhecer para os editar.
            bevy_ext_scene::SceneSpecPlugin,
            SceneSettingsPlugin::new(settings_world(&root)),
        ))
        .add_scene_setting::<FloraSettings>()
        .add_scene_setting_list::<FloraSpecies>()
        .add_scene_setting::<GrassSettingsSpec>()
        .add_scene_setting_list::<GrassStyleSpec>();
        // EnvironmentPlugin reads the initial value while building its systems.
        let elements =
            native_value::<ElementsSettings>(&app.world().resource::<SceneNatives>().natives)
                .unwrap_or_default();
        app.insert_resource(SettingsInPlay(crate::editor_link::link_enabled()))
            .insert_resource(SceneSetting(elements))
            .add_systems(
                PreUpdate,
                (
                    read_singleton::<ElementsSettings>.pipe(apply_elements),
                    read_singleton::<PostFxSettings>.pipe(apply_postfx_settings),
                ),
            );
    }
}

#[derive(Resource)]
struct SettingsInPlay(bool);

/// Keep the disk baseline separate from the current PIE value. Saving another
/// module leaves the preview intact. Saving a new value for this setting wins.
#[derive(Default)]
struct Singleton<T> {
    generation: Option<u64>,
    disk: Option<(Option<u64>, T)>,
    entity: Option<Entity>,
    published: Option<Option<T>>,
}

fn read_singleton<T>(
    natives: Res<SceneNatives>,
    play: Res<SettingsInPlay>,
    mut state: Local<Singleton<T>>,
    mut commands: Commands,
    components: Query<&T>,
) -> Option<Option<T>>
where
    T: Component
        + Reflect
        + FromReflect
        + GetTypeRegistration
        + TypePath
        + Default
        + Clone
        + PartialEq,
{
    if state.generation != Some(natives.generation) {
        state.generation = Some(natives.generation);
        let disk = natives.natives.iter().find_map(|node| {
            node.get(T::type_path())?;
            native_value::<T>(std::slice::from_ref(node)).map(|value| (node.node_id, value))
        });
        if state.disk != disk {
            if let Some(entity) = state.entity.take() {
                commands.entity(entity).try_despawn();
            }
            if play.0
                && let Some((Some(id), value)) = &disk
            {
                state.entity = Some(commands.spawn((SceneNodeId(*id), value.clone())).id());
            }
            state.disk = disk;
            // The new entity becomes queryable after commands are applied.
            let next = state.disk.as_ref().map(|(_, value)| value.clone());
            state.published = Some(next.clone());
            return Some(next);
        }
    }
    let next = match state.entity {
        Some(entity) => components.get(entity).ok(),
        None => state.disk.as_ref().map(|(_, value)| value),
    };
    if state.published.as_ref().map(Option::as_ref) == Some(next) {
        return None;
    }
    let next = next.cloned();
    state.published = Some(next.clone());
    Some(next)
}

fn apply_elements(
    In(change): In<Option<Option<ElementsSettings>>>,
    mut setting: ResMut<SceneSetting<ElementsSettings>>,
) {
    if let Some(value) = change {
        setting.0 = value.unwrap_or_default();
    }
}

fn apply_postfx_settings(In(change): In<Option<Option<PostFxSettings>>>) {
    let Some(value) = change else { return };
    match value {
        Some(settings) => viber::postfx::set_scene_profile(
            &settings.disabled_keys(),
            settings.ssao.then_some(settings.ssao_quality.name()),
        ),
        None => viber::postfx::clear_scene_profile(),
    }
}

/// A lente que o Vael pagava no `config.yaml` (`postfx.disabled` + `ssao`).
pub fn vael_postfx() -> PostFxSettings {
    PostFxSettings {
        volumetrics: false,
        depth_of_field: false,
        motion_blur: false,
        chromatic_aberration: false,
        vignette: false,
        split_tone: false,
        aerial: false,
        auto_exposure: false,
        ssao_quality: SsaoQuality::Medium,
        ..default()
    }
}

/// Escreve `world/settings/{environment,postfx,flora}.bsn` com os valores
/// atuais (as constantes de antes). Não sobrescreve sem `force`.
pub fn export_settings(root: &Path, force: bool) -> Result<Vec<PathBuf>> {
    let mut registry = TypeRegistry::default();
    registry.register::<ElementsSettings>();
    registry.register::<PostFxSettings>();
    registry.register::<FloraSpecies>();
    registry.register::<FloraSettings>();
    registry.register::<GrassSettingsSpec>();
    registry.register::<GrassStyleSpec>();
    // Sem `Transform` o Jackdaw não pendura a entidade na hierarquia.
    registry.register::<SceneNodeId>();
    registry.register::<Transform>();
    let transform = Transform::default();

    let elements = ElementsSettings::default();
    let postfx = vael_postfx();
    let flora = FloraSettings::default();
    let species = crate::flora::legacy_species();
    let grass = crate::grass::legacy_grass_settings();
    let grass_styles = crate::grass::legacy_grass_styles();

    // The default modules and their instances in world.bsn address the same PIE nodes.
    let environment_id = SceneNodeId(6852620019402782939);
    let postfx_id = SceneNodeId(6852620019402782940);
    let files = [
        (
            "environment.bsn",
            emit_native_entity(
                "Ambiente",
                &[
                    transform.as_partial_reflect(),
                    elements.as_partial_reflect(),
                    environment_id.as_partial_reflect(),
                ],
                &registry,
            ),
        ),
        (
            "postfx.bsn",
            emit_native_entity(
                "Pós-processamento",
                &[
                    transform.as_partial_reflect(),
                    postfx.as_partial_reflect(),
                    postfx_id.as_partial_reflect(),
                ],
                &registry,
            ),
        ),
        (
            "flora.bsn",
            emit_native_group(
                "Flora",
                &[transform.as_partial_reflect(), flora.as_partial_reflect()],
                &species
                    .iter()
                    .map(|species| {
                        (
                            species.key.clone(),
                            vec![transform.as_partial_reflect(), species.as_partial_reflect()],
                        )
                    })
                    .collect::<Vec<_>>(),
                &registry,
            ),
        ),
        (
            "grass.bsn",
            emit_native_group(
                "Relva",
                &[transform.as_partial_reflect(), grass.as_partial_reflect()],
                &grass_styles
                    .iter()
                    .map(|style| {
                        (
                            format!("Bioma {}", style.biome),
                            vec![transform.as_partial_reflect(), style.as_partial_reflect()],
                        )
                    })
                    .collect::<Vec<_>>(),
                &registry,
            ),
        ),
    ];

    let dir = root.join(SETTINGS_DIR);
    // Confere os quatro ANTES de escrever: um `--force` a menos não pode
    // deixar a pasta com metade dos ficheiros reescritos e a outra metade
    // intacta (o bail a meio de um export anterior fazia isso).
    if !force {
        for (name, _) in &files {
            let path = dir.join(name);
            if path.exists() {
                anyhow::bail!("{} já existe (usa --force)", path.display());
            }
        }
    }
    // Emite e VALIDA os quatro antes do primeiro `write`: uma falha de
    // validação a meio também não pode deixar a exportação a metade.
    let mut contents = Vec::with_capacity(files.len());
    for (name, body) in files {
        let path = dir.join(name);
        let text = format!(
            "// Definições do Vael ({name}). Editar no Jackdaw (`jd open .`):\n\
             // o inspector e os painéis da extensão vael_jackdaw gravam aqui.\n\n{body}"
        );
        jackdaw_bsn_check(&path, &text)?;
        contents.push((path, text));
    }
    std::fs::create_dir_all(&dir).with_context(|| format!("criar {}", dir.display()))?;
    let mut written = Vec::new();
    for (path, text) in contents {
        std::fs::write(&path, text).with_context(|| format!("gravar {}", path.display()))?;
        written.push(path);
    }
    Ok(written)
}

fn jackdaw_bsn_check(path: &Path, text: &str) -> Result<()> {
    bevy_ext_world::bsn::validate_emitted(path, text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::{AppTypeRegistry, MinimalPlugins};

    #[test]
    fn jackdaw_pie_environment_edits_use_the_authored_id_and_survive_unrelated_reloads() {
        use jackdaw_runtime::node_sync::{apply_node_components, install_node_index};
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("world/settings/environment.bsn");
        let (native, _) = bevy_ext_world::bsn::settings::read_natives(&path).unwrap();
        let id = native[0].node_id.expect("id do módulo");
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ElementsSettingsPlugin))
            .init_resource::<SceneNatives>()
            .insert_resource(SettingsInPlay(true))
            .init_resource::<SceneSetting<ElementsSettings>>()
            .add_systems(
                PreUpdate,
                read_singleton::<ElementsSettings>.pipe(apply_elements),
            );
        install_node_index(&mut app);
        app.world_mut().resource_mut::<SceneNatives>().natives = native;
        app.update();

        let path = ElementsSettings::type_path().to_string();
        let before = app
            .world()
            .resource::<SceneSetting<ElementsSettings>>()
            .0
            .clone();
        let mut value = before.clone();
        value.start_hour = 22.0;
        let registry = app.world().resource::<AppTypeRegistry>().read();
        let edit = serde_json::to_value(bevy::reflect::serde::TypedReflectSerializer::new(
            &value, &registry,
        ))
        .unwrap();
        drop(registry);
        apply_node_components(app.world_mut(), id, &[(path.clone(), edit)], &[]);
        app.update();
        assert_eq!(
            app.world().resource::<SceneSetting<ElementsSettings>>().0,
            value
        );
        app.world_mut().resource_mut::<SceneNatives>().generation += 1;
        app.update();
        assert_eq!(
            app.world().resource::<SceneSetting<ElementsSettings>>().0,
            value
        );
        apply_node_components(app.world_mut(), id, &[], &[path]);
        app.update();
        assert_eq!(
            app.world().resource::<SceneSetting<ElementsSettings>>().0,
            ElementsSettings::default()
        );
    }

    /// O que se exporta volta a ler-se igual (o round-trip do editor).
    #[test]
    fn export_le_se_de_volta() {
        let dir = tempfile_dir();
        let written = export_settings(&dir, true).expect("export");
        assert_eq!(written.len(), 4);
        for name in ["environment.bsn", "postfx.bsn"] {
            let authored_path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(SETTINGS_DIR)
                .join(name);
            let (authored, _) =
                bevy_ext_world::bsn::settings::read_natives(&authored_path).unwrap();
            let (exported, _) =
                bevy_ext_world::bsn::settings::read_natives(&dir.join(SETTINGS_DIR).join(name))
                    .unwrap();
            assert!(authored[0].node_id.is_some());
            assert_eq!(exported[0].node_id, authored[0].node_id);
        }
        let world = dir.join("world.bsn");
        std::fs::write(
            &world,
            "#\"world\"\nbevy_ecs::hierarchy::Children [\n    #\"Ambiente\"\n    jackdaw::prefab::components::IsA { source: \"world/settings/environment.bsn\", deleted: [] }\n    --\n    #\"Pós\"\n    jackdaw::prefab::components::IsA { source: \"world/settings/postfx.bsn\", deleted: [] }\n    --\n    #\"Flora\"\n    jackdaw::prefab::components::IsA { source: \"world/settings/flora.bsn\", deleted: [] }\n    --\n    #\"Relva\"\n    jackdaw::prefab::components::IsA { source: \"world/settings/grass.bsn\", deleted: [] }\n]\n",
        )
        .expect("world");
        let (natives, _) = bevy_ext_world::bsn::settings::read_natives(&world).expect("ler");
        assert_eq!(
            native_value::<ElementsSettings>(&natives),
            Some(ElementsSettings::default())
        );
        assert_eq!(
            native_value::<PostFxSettings>(&natives),
            Some(vael_postfx())
        );
        assert_eq!(
            native_value::<FloraSettings>(&natives),
            Some(FloraSettings::default())
        );
        let species = bevy_ext_world::bsn::settings::native_values::<FloraSpecies>(&natives);
        assert_eq!(species, crate::flora::legacy_species());
        assert_eq!(
            native_value::<GrassSettingsSpec>(&natives),
            Some(crate::grass::legacy_grass_settings())
        );
        let styles = bevy_ext_world::bsn::settings::native_values::<GrassStyleSpec>(&natives);
        assert_eq!(styles, crate::grass::legacy_grass_styles());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Um ficheiro já existente aborta o export ANTES de escrever os outros:
    /// nada de uma pasta com metade dos ficheiros reescritos.
    #[test]
    fn export_sem_force_nao_escreve_nada_a_meio() {
        let dir = tempfile_dir();
        let settings = dir.join(SETTINGS_DIR);
        std::fs::create_dir_all(&settings).expect("settings");
        std::fs::write(settings.join("postfx.bsn"), "// já existe").expect("postfx");

        let err = export_settings(&dir, false).expect_err("devia recusar");
        assert!(err.to_string().contains("já existe"), "{err}");
        assert!(
            !settings.join("environment.bsn").exists(),
            "nenhum ficheiro se perde nem se escreve a meio"
        );
        assert_eq!(
            std::fs::read_to_string(settings.join("postfx.bsn")).expect("postfx"),
            "// já existe"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    fn tempfile_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vael-settings-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).expect("tmp");
        dir
    }
}

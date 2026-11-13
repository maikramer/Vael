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
use bevy::prelude::{App, First, Local, Plugin, Res, Transform, default};
use bevy::reflect::PartialReflect;
use bevy::reflect::TypeRegistry;
use bevy_ext_elements::{ElementsSettings, ElementsSettingsPlugin};
use bevy_ext_flora::{FloraSettings, FloraSettingsPlugin, FloraSpecies};
use bevy_ext_postfx::{PostFxSettings, PostFxSettingsPlugin, SsaoQuality};
use bevy_ext_world::bsn::settings::{
    SceneNatives, SceneSettingsApp, SceneSettingsPlugin, emit_native_entity, emit_native_group,
    native_value,
};

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
            SceneSettingsPlugin::new(settings_world(&root)),
        ))
        .add_scene_setting::<ElementsSettings>()
        .add_scene_setting::<FloraSettings>()
        .add_scene_setting_list::<FloraSpecies>()
        .add_systems(First, apply_postfx_settings);
    }
}

/// O `PostFxSettings` da cena substitui o `postfx:` do `config.yaml` (e
/// volta a ele se a entidade desaparecer do documento).
fn apply_postfx_settings(natives: Option<Res<SceneNatives>>, mut seen: Local<Option<u64>>) {
    let Some(natives) = natives else {
        return;
    };
    if *seen == Some(natives.generation) {
        return;
    }
    *seen = Some(natives.generation);
    match native_value::<PostFxSettings>(&natives.natives) {
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
    // Sem `Transform` o Jackdaw não pendura a entidade na hierarquia.
    registry.register::<Transform>();
    let transform = Transform::default();

    let elements = ElementsSettings::default();
    let postfx = vael_postfx();
    let flora = FloraSettings::default();
    let species = crate::flora::legacy_species();

    let files = [
        (
            "environment.bsn",
            emit_native_entity(
                "Ambiente",
                &[
                    transform.as_partial_reflect(),
                    elements.as_partial_reflect(),
                ],
                &registry,
            ),
        ),
        (
            "postfx.bsn",
            emit_native_entity(
                "Pós-processamento",
                &[transform.as_partial_reflect(), postfx.as_partial_reflect()],
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
    ];

    let dir = root.join(SETTINGS_DIR);
    std::fs::create_dir_all(&dir).with_context(|| format!("criar {}", dir.display()))?;
    let mut written = Vec::new();
    for (name, body) in files {
        let path = dir.join(name);
        if path.exists() && !force {
            anyhow::bail!("{} já existe (usa --force)", path.display());
        }
        let text = format!(
            "// Definições do Vael ({name}). Editar no Jackdaw (`jd open .`):\n\
             // o inspector e os painéis da extensão vael_jackdaw gravam aqui.\n\n{body}"
        );
        jackdaw_bsn_check(&path, &text)?;
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

    /// O que se exporta volta a ler-se igual (o round-trip do editor).
    #[test]
    fn export_le_se_de_volta() {
        let dir = tempfile_dir();
        let written = export_settings(&dir, true).expect("export");
        assert_eq!(written.len(), 3);
        let world = dir.join("world.bsn");
        std::fs::write(
            &world,
            "#\"world\"\nbevy_ecs::hierarchy::Children [\n    #\"Ambiente\"\n    jackdaw::prefab::components::IsA { source: \"world/settings/environment.bsn\", deleted: [] }\n    --\n    #\"Pós\"\n    jackdaw::prefab::components::IsA { source: \"world/settings/postfx.bsn\", deleted: [] }\n    --\n    #\"Flora\"\n    jackdaw::prefab::components::IsA { source: \"world/settings/flora.bsn\", deleted: [] }\n]\n",
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

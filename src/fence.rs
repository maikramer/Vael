//! Cercas procedurais (`bevy_ext_fence`): o chão do Vael e as definições da cena.
//!
//! O mundo deixou de repetir um GLB de cerca: cada linha autorada é um
//! `<Entity>` com `FenceSpec` (`fence-points`, `fence-style`, …) e o
//! `FencePlugin` gera postes, travessas, tábuas, paliçada, varas entretecidas,
//! treliça, cruzetas, cordas ou portão (`docs/FENCES.md`). As predefinições são
//! `FenceStyle` em `world/settings/fences.bsn`, como as espécies da flora.
//!
//! Este módulo é só o adaptador:
//!
//! * **chão**: [`VaelFenceGround`] amostra o `TerrainRuntime` — dentro do
//!   heightmap é a cota do terreno; fora dele (interiores) não há chão e os
//!   postes ficam à cota autorada;
//! * **definições**: `SceneSetting<FenceSettings>`/`SceneSettingList<FenceStyle>`
//!   (recursos da cena) → `FenceSettingsValue`/`FenceStyleSet` do crate, só
//!   quando mudam, para não refazer as cercas a cada frame;
//! * **o pedido**: fora do Play do Jackdaw a entidade nasce sem o componente;
//!   `adopt_specs` põe-lhe o `FenceSpec` que o loader tirou do documento.
//!
//! Gate `VAEL_FENCE=0` (o `VIBER_FENCE` legado continua a valer).

use std::sync::Arc;

use bevy::prelude::*;
use bevy_ext_fence::{
    FenceGround, FenceGroundProvider, FencePlugin, FenceSettings, FenceSettingsValue, FenceStyle, FenceStyleSet,
};
use bevy_ext_scene::FenceSpec;
use bevy_ext_world::bsn::specs;
use vael_engine::scene::SceneSource;
use vael_engine::terrain::runtime::TerrainRuntime;

use crate::settings::{SceneSetting, SceneSettingList};

/// `VAEL_FENCE` ligado por omissão: `0`/`false`/`off`/`no` desliga.
fn fence_enabled(value: Option<&str>) -> bool {
    !matches!(
        value.map(|raw| raw.trim().to_ascii_lowercase()).as_deref(),
        Some("0" | "false" | "off" | "no")
    )
}

/// O terreno do Vael visto da cerca.
struct VaelFenceGround {
    sample: Box<dyn Fn(f32, f32) -> f32 + Send + Sync>,
    half: f32,
}

impl FenceGround for VaelFenceGround {
    fn height(&self, x: f32, z: f32) -> Option<f32> {
        // Fora do heightmap não há chão: as amostras saturam na orla e
        // devolviam a cota da borda como se fosse piso.
        (x >= -self.half && x <= self.half && z >= -self.half && z <= self.half)
            .then(|| (self.sample)(x, z))
    }
}

/// Publica o terreno ao crate: um `TerrainRuntime` novo (ou editado) troca o
/// chão; sem terreno (interiores) não há chão nenhum.
fn provide_ground(
    terrain: Option<Res<TerrainRuntime>>,
    mut provider: ResMut<FenceGroundProvider>,
    mut seen: Local<Option<(usize, u64)>>,
) {
    let Some(terrain) = terrain else {
        if provider.0.take().is_some() {
            *seen = None;
        }
        return;
    };
    let stamp = (Arc::as_ptr(&terrain.grid) as usize, terrain.deltas.revision());
    if *seen == Some(stamp) {
        return;
    }
    *seen = Some(stamp);
    // O leitor partilha os `Arc` da grelha: barato de clonar, e segue o
    // terreno sem prender o recurso.
    let reader = terrain.reader();
    provider.0 = Some(Arc::new(VaelFenceGround {
        sample: Box::new(move |x, z| reader.sample(x, z)),
        half: terrain.spec.world_size * 0.5,
    }));
}

/// As definições da cena (recursos) para o crate. Só escreve quando mudam: um
/// `ResMut` tocado todos os frames refazia as cercas a cada frame.
fn sync_definitions(
    settings: Res<SceneSetting<FenceSettings>>,
    styles: Res<SceneSettingList<FenceStyle>>,
    mut value: ResMut<FenceSettingsValue>,
    mut set: ResMut<FenceStyleSet>,
) {
    if value.0.as_ref() != Some(&settings.0) {
        value.0 = Some(settings.0.clone());
    }
    if set.0 != styles.0 {
        set.0 = styles.0.clone();
    }
}

/// Já olhámos se o nó tinha cerca.
#[derive(Component)]
struct FenceAdopted;

/// Põe nas entidades do mundo o `FenceSpec` que o loader leu do documento. No
/// Play do Jackdaw o `live_edit` já o faz (`mirror_specs`); aqui é para o jogo
/// normal, onde o componente não chega de outra forma.
fn adopt_specs(mut commands: Commands, nodes: Query<(Entity, &SceneSource), (Without<FenceSpec>, Without<FenceAdopted>)>) {
    for (entity, source) in &nodes {
        commands.entity(entity).try_insert(FenceAdopted);
        if let Some(spec) = specs::specs_for(&source.0).and_then(|specs| specs.fence.clone()) {
            commands.entity(entity).try_insert(spec);
        }
    }
}

pub struct VaelFencePlugin;

impl Plugin for VaelFencePlugin {
    fn build(&self, app: &mut App) {
        if !fence_enabled(bevy_ext_core::envs::var_lossy("FENCE").as_deref()) {
            return;
        }
        app.add_plugins(FencePlugin)
            .add_systems(PreUpdate, (provide_ground, sync_definitions, adopt_specs).chain());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn o_gate_liga_por_omissao() {
        assert!(fence_enabled(None));
        assert!(fence_enabled(Some("1")));
        for off in ["0", "false", "OFF", " no "] {
            assert!(!fence_enabled(Some(off)), "{off}");
        }
    }

    #[test]
    fn sem_terreno_nao_ha_chao() {
        let mut app = App::new();
        app.init_resource::<FenceGroundProvider>()
            .add_systems(Update, provide_ground);
        app.world_mut().resource_mut::<FenceGroundProvider>().0 = Some(Arc::new(VaelFenceGround {
            sample: Box::new(|_, _| 0.0),
            half: 32.0,
        }));
        app.update();
        assert!(app.world().resource::<FenceGroundProvider>().0.is_none(), "um interior não tem chão");
    }

    #[test]
    fn o_chao_e_o_terreno_dentro_do_campo() {
        let ground = VaelFenceGround {
            sample: Box::new(|x, z| x + 10.0 * z),
            half: 32.0,
        };
        assert_eq!(ground.height(2.0, 0.5), Some(7.0));
        assert_eq!(ground.height(32.0, -32.0), Some(-288.0), "a borda ainda é chão");
        assert!(ground.height(32.1, 0.0).is_none(), "fora do heightmap não há chão");
        assert!(ground.height(0.0, -33.0).is_none());
    }

    #[test]
    fn as_definicoes_so_se_escrevem_quando_mudam() {
        let mut app = App::new();
        app.insert_resource(SceneSetting::<FenceSettings>(FenceSettings::default()))
            .insert_resource(SceneSettingList::<FenceStyle>(bevy_ext_fence::presets()))
            .init_resource::<FenceSettingsValue>()
            .init_resource::<FenceStyleSet>()
            .add_systems(Update, sync_definitions);
        app.update();
        let world = app.world_mut();
        assert_eq!(world.resource::<FenceStyleSet>().0.len(), 10);
        assert_eq!(world.resource::<FenceSettingsValue>().0, Some(FenceSettings::default()));
        // Já iguais: um frame a mais não marca nada.
        world.resource_mut::<FenceStyleSet>().bypass_change_detection();
        app.update();
        assert!(!app.world().is_resource_changed::<FenceStyleSet>());
        assert!(!app.world().is_resource_changed::<FenceSettingsValue>());
        // Uma gravação no editor chega ao crate.
        app.world_mut().resource_mut::<SceneSetting<FenceSettings>>().0.batch = 3;
        app.update();
        assert_eq!(app.world().resource::<FenceSettingsValue>().0.as_ref().map(|s| s.batch), Some(3));
    }
}

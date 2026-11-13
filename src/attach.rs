//! Ligação entre as entidades com script Luau e o lado Rust.
//!
//! A engine marca cada nó com [`LuaScriptRef`] (o caminho do script) e, se o
//! nó declara `prop-*`, com [`ScriptProps`]. O Vael copia esses dados para um
//! [`VaelScriptTags`] estável: é o gancho que a Wave 1 vai usar para saber
//! qual é o comportamento de cada entidade sem voltar a consultar o XML.
//!
//! Sem comportamento nesta passagem — só a cópia.

use std::collections::BTreeMap;

use bevy::prelude::*;
use viber::luau::components::{LuaScriptRef, ScriptProps};

/// Script e props de uma entidade, já fora do host Luau.
#[derive(Component, Debug, Clone, PartialEq, Eq, Reflect)]
#[reflect(Component)]
pub struct VaelScriptTags {
    /// Caminho relativo a `scripts/` (ex.: `doors/gate.lua`).
    pub path: String,
    /// Props por instância (`prop-<nome>` do XML), texto cru.
    pub props: BTreeMap<String, String>,
}

/// Plugin de ligação: corre depois do spawn da cena e a cada troca de cena.
pub struct VaelAttachPlugin;

impl Plugin for VaelAttachPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<VaelScriptTags>()
            .add_systems(PostStartup, attach_script_tags)
            // O primeiro `spawn::startup` corre no `Startup`; as trocas de
            // cena voltam a correr `SceneLoad` — a ordem importa.
            .add_systems(
                viber::scene_loader::SceneLoad,
                attach_script_tags.after(viber::recipes::spawn::startup),
            );
    }
}

fn attach_script_tags(
    mut commands: Commands,
    scripts: Query<(Entity, &LuaScriptRef, Option<&ScriptProps>), Without<VaelScriptTags>>,
) {
    for (entity, script, props) in &scripts {
        let props = props
            .map(|props| props.0.iter().cloned().collect())
            .unwrap_or_default();
        commands.entity(entity).insert(VaelScriptTags {
            path: script.path.clone(),
            props,
        });
    }
}

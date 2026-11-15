//! Gatilho de área de uma cinemática, agora em Rust.
//!
//! O nó é uma `<Entity>` normal com o componente `CinematicTrigger`, autorado
//! no Jackdaw (inspector › Câmara) e entregue pela ponte de componentes
//! nativos do [`crate::editor_link`] — `vael::` está na lista de prefixos
//! permitidos, por isso o componente chega à entidade em qualquer execução,
//! não só no Play.
//!
//! Era o `scripts/cinematic-trigger.lua`: dispara na **entrada** do raio (não
//! enquanto o herói fica dentro), uma vez por save quando `once`, e não
//! consome a entrada se uma cena ou um menu estiver aberto (quando fechar, o
//! gatilho ainda vê o herói dentro e dispara).
//!
//! A marca "já tocou" vive em [`VaelSave::markers`] (`cine.<id>`), que é
//! persistido e o `merge_world_kv` nunca desfaz. Saves antigos tinham a marca
//! no `legacy` do Lua (`cine_<id>`): é lida para não repetir o filme.

use bevy::prelude::*;
use bevy_ext_core::{MenusOpen, Player};
use viber::cinematic::{CinematicPlayback, PlayCinematic};

use crate::save::VaelSave;

/// Raio por omissão quando o componente não o traz.
pub const DEFAULT_RADIUS: f32 = 14.0;
/// Marca no save de que a cinemática já tocou por este gatilho.
const MARK_PREFIX: &str = "cine.";

/// Componente autorado no Jackdaw: a cinemática e a área que a dispara.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component, Default, Debug, Clone)]
pub struct CinematicTrigger {
    /// `id` da `<Cinematic>` (o mesmo do painel Vael: Cinemáticas).
    pub cinematic: String,
    /// Raio de entrada (m).
    pub radius: f32,
    /// Só a primeira vez (por save).
    pub once: bool,
}

impl Default for CinematicTrigger {
    fn default() -> Self {
        Self {
            cinematic: String::new(),
            radius: DEFAULT_RADIUS,
            once: true,
        }
    }
}

/// Estado de dentro/fora do gatilho (vive na entidade, não no save).
#[derive(Component, Debug, Default, Clone, Copy)]
struct Inside(bool);

pub struct VaelCinematicTriggerPlugin;

impl Plugin for VaelCinematicTriggerPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<CinematicTrigger>()
            .add_systems(Update, drive_cinematic_triggers);
    }
}

#[allow(clippy::too_many_arguments)]
fn drive_cinematic_triggers(
    mut commands: Commands,
    mut triggers: Query<(Entity, &CinematicTrigger, &GlobalTransform, Option<&Inside>)>,
    players: Query<&GlobalTransform, With<Player>>,
    playback: Option<Res<CinematicPlayback>>,
    menus: Option<Res<MenusOpen>>,
    cinematics: Option<Res<viber::cinematic::CinematicLibrary>>,
    mut save: Option<ResMut<VaelSave>>,
    mut play: MessageWriter<PlayCinematic>,
) {
    let Some(player) = players.iter().next() else {
        return;
    };
    let hero = player.translation();
    // Uma cena a tocar (ou um menu aberto) não dispara nem consome a entrada:
    // quando acabar, o herói ainda está dentro e o bordo acontece então.
    let playing = playback.as_deref().and_then(CinematicPlayback::playing);
    if blocked(playing, menus.as_deref()) {
        return;
    }
    for (entity, trigger, transform, inside) in &mut triggers {
        let id = trigger.cinematic.trim();
        if id.is_empty() {
            continue;
        }
        let at = transform.translation();
        let radius = if trigger.radius.is_finite() {
            trigger.radius.clamp(0.1, 500.0)
        } else {
            DEFAULT_RADIUS
        };
        let dx = hero.x - at.x;
        let dz = hero.z - at.z;
        let current = dx * dx + dz * dz <= radius * radius;
        let entering = current && !inside.is_some_and(|inside| inside.0);
        if !inside.is_some_and(|inside| inside.0 == current)
            && let Ok(mut entity) = commands.get_entity(entity)
        {
            entity.try_insert(Inside(current));
        }
        if !entering {
            continue;
        }
        // Um id sem `<Cinematic>` no mundo não pode queimar o `once`: o
        // pedido é recusado e a marca ficava para sempre (o gatilho nunca
        // mais tocava, nem depois de o autor corrigir o id).
        if cinematics
            .as_deref()
            .is_some_and(|library| !library.0.contains_key(id))
        {
            warn_once!("vael: gatilho de cinemática `{id}` sem <Cinematic> — ignorado");
            continue;
        }
        if trigger.once && save.as_deref().is_some_and(|save| already_played(save, id)) {
            continue;
        }
        if trigger.once
            && let Some(save) = save.as_deref_mut()
        {
            save.markers.insert(format!("{MARK_PREFIX}{id}"));
        }
        play.write(PlayCinematic {
            id: id.to_string(),
            anchor: None,
            inline: None,
        });
    }
}

/// Uma cena a tocar ou um menu aberto bloqueiam o gatilho.
fn blocked(playing: Option<&str>, menus: Option<&MenusOpen>) -> bool {
    playing.is_some() || menus.is_some_and(MenusOpen::any)
}

/// `true` quando este gatilho já tocou neste save.
fn already_played(save: &VaelSave, id: &str) -> bool {
    if save.markers.contains(&format!("{MARK_PREFIX}{id}")) {
        return true;
    }
    // Compatibilidade: o `cinematic-trigger.lua` gravava `cine_<id>` no
    // `viber.game()`, que o save guarda no `legacy` (chave `cine_<id>`).
    save.legacy.get(&format!("cine_{id}")).is_some_and(truthy)
}

/// Um valor do `legacy` (JSON) conta como "feito" se for verdadeiro/não vazio.
fn truthy(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Bool(value) => *value,
        serde_json::Value::Number(value) => value.as_f64().is_some_and(|n| n != 0.0),
        serde_json::Value::String(value) => !value.is_empty(),
        serde_json::Value::Null => false,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save::VaelSave;

    /// Os ids que o sistema pediu para tocar, lidos como mensagens.
    #[derive(Resource, Default)]
    struct Seen(Vec<String>);

    fn collect(mut reader: MessageReader<PlayCinematic>, mut seen: ResMut<Seen>) {
        for message in reader.read() {
            seen.0.push(message.id.clone());
        }
    }

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<CinematicPlayback>()
            .init_resource::<VaelSave>()
            .init_resource::<MenusOpen>()
            .init_resource::<Seen>()
            .add_message::<PlayCinematic>()
            .add_systems(Update, (drive_cinematic_triggers, collect).chain());
        app
    }

    fn player(app: &mut App, at: Vec3) -> Entity {
        app.world_mut()
            .spawn((Player::default(), GlobalTransform::from_translation(at)))
            .id()
    }

    fn trigger(app: &mut App, cinematic: &str, at: Vec3, radius: f32, once: bool) -> Entity {
        app.world_mut()
            .spawn((
                CinematicTrigger {
                    cinematic: cinematic.into(),
                    radius,
                    once,
                },
                GlobalTransform::from_translation(at),
            ))
            .id()
    }

    /// As mensagens entregues até agora (e limpa a lista).
    fn played(app: &mut App) -> Vec<String> {
        std::mem::take(&mut app.world_mut().resource_mut::<Seen>().0)
    }

    fn move_to(app: &mut App, entity: Entity, at: Vec3) {
        app.world_mut()
            .entity_mut(entity)
            .insert(GlobalTransform::from_translation(at));
    }

    #[test]
    fn dispara_na_entrada_e_uma_vez() {
        let mut app = app();
        let hero = player(&mut app, Vec3::new(100.0, 0.0, 0.0));
        trigger(&mut app, "campo", Vec3::ZERO, 22.0, true);
        app.update();
        assert!(played(&mut app).is_empty(), "fora do raio não dispara");

        move_to(&mut app, hero, Vec3::new(10.0, 0.0, 0.0));
        app.update();
        assert_eq!(played(&mut app), ["campo"], "entrada dispara");
        app.update();
        assert!(played(&mut app).is_empty(), "ficar dentro não repete");

        // Sai e volta a entrar: `once` já marcou no save.
        move_to(&mut app, hero, Vec3::new(100.0, 0.0, 0.0));
        app.update();
        move_to(&mut app, hero, Vec3::ZERO);
        app.update();
        assert!(
            played(&mut app).is_empty(),
            "once: não repete no mesmo save"
        );
        assert!(
            app.world()
                .resource::<VaelSave>()
                .markers
                .contains("cine.campo"),
            "marca gravada"
        );
    }

    #[test]
    fn once_false_repete_apos_sair_e_voltar() {
        let mut app = app();
        let hero = player(&mut app, Vec3::ZERO);
        trigger(&mut app, "campo", Vec3::ZERO, 22.0, false);
        app.update();
        assert_eq!(played(&mut app), ["campo"]);
        move_to(&mut app, hero, Vec3::new(100.0, 0.0, 0.0));
        app.update();
        move_to(&mut app, hero, Vec3::ZERO);
        app.update();
        assert_eq!(played(&mut app), ["campo"], "sem once volta a tocar");
        assert!(app.world().resource::<VaelSave>().markers.is_empty());
    }

    #[test]
    fn cena_ou_menu_nao_disparam_nem_consomem() {
        // Sem cena nem menu, o gatilho não está bloqueado.
        assert!(!blocked(None, Some(&MenusOpen::default())));
        // Uma cena a tocar bloqueia.
        assert!(blocked(Some("outra"), Some(&MenusOpen::default())));
        // Um menu aberto bloqueia; fechado não.
        assert!(blocked(
            None,
            Some(&MenusOpen {
                map: true,
                ..Default::default()
            })
        ));
        assert!(blocked(
            None,
            Some(&MenusOpen {
                cinematic: true,
                ..Default::default()
            })
        ));

        // E o sistema, com o menu aberto, não dispara nem marca a entrada:
        // quando o menu fechar, o bordo ainda acontece.
        let mut app = app();
        player(&mut app, Vec3::ZERO);
        trigger(&mut app, "campo", Vec3::ZERO, 22.0, true);
        app.world_mut().resource_mut::<MenusOpen>().modal = true;
        app.update();
        assert!(played(&mut app).is_empty(), "menu aberto bloqueia");
        app.world_mut().resource_mut::<MenusOpen>().modal = false;
        app.update();
        assert_eq!(played(&mut app), ["campo"], "fechou: dispara");
    }

    #[test]
    fn marca_do_lua_antigo_conta() {
        let mut save = VaelSave::default();
        save.legacy
            .insert("cine_campo".into(), serde_json::json!(true));
        assert!(already_played(&save, "campo"));
        assert!(!already_played(&save, "outra"));
    }

    #[test]
    fn id_sem_cinematica_nao_dispara_nem_queima_o_once() {
        let mut app = app();
        // Biblioteca sem o id: o `<Cinematic>` não existe no mundo.
        app.insert_resource(viber::cinematic::CinematicLibrary(Default::default()));
        player(&mut app, Vec3::ZERO);
        trigger(&mut app, "campo", Vec3::ZERO, 22.0, true);
        app.update();
        assert!(played(&mut app).is_empty(), "sem <Cinematic> não toca");
        assert!(
            app.world().resource::<VaelSave>().markers.is_empty(),
            "e não grava a marca (o gatilho continua vivo)"
        );
    }

    #[test]
    fn id_vazio_nao_dispara() {
        let mut app = app();
        player(&mut app, Vec3::ZERO);
        trigger(&mut app, "  ", Vec3::ZERO, 22.0, true);
        app.update();
        assert!(played(&mut app).is_empty());
    }

    #[test]
    fn plugin_regista_o_tipo() {
        // O `editor_link` só aplica componentes registados; sem isto o
        // gatilho autorado era ignorado com um `warn_once`.
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(VaelCinematicTriggerPlugin);
        let registry = app.world().resource::<AppTypeRegistry>().read();
        assert!(
            registry
                .get_with_type_path("vael::cinematic_trigger::CinematicTrigger")
                .is_some()
        );
    }

    /// O nó do gatilho no `world.bsn` chega ao jogo como componente nativo (o
    /// `editor_link` aplica-o) e já não tem `script`/`prop-*`: o gatilho é
    /// Rust, não o `cinematic-trigger.lua`.
    #[test]
    fn o_mundo_traz_o_componente() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("world.bsn");
        let loaded = bevy_ext_world::bsn::load_bsn_world(&path).expect("world.bsn");
        fn walk(nodes: &[bevy_ext_world::xml::XmlNode], found: &mut bool) {
            for node in nodes {
                if node.attr("name") == Some("trigger.campo-de-batalha") {
                    let origin = node.origin.as_ref().expect("origem do nó");
                    let patches = bevy_ext_world::bsn::specs::native_components_for(origin)
                        .expect("componentes nativos do gatilho");
                    let paths: Vec<&str> = patches
                        .iter()
                        .filter_map(jackdaw_bsn::patch_type_path)
                        .collect();
                    assert_eq!(
                        paths,
                        ["vael::cinematic_trigger::CinematicTrigger"],
                        "o gatilho chega como componente nativo"
                    );
                    assert!(node.attr("script").is_none(), "sem script Lua");
                    assert!(
                        node.attr("prop-cinematic").is_none(),
                        "as props antigas saíram"
                    );
                    *found = true;
                }
                walk(&node.children, found);
            }
        }
        let mut found = false;
        walk(&loaded.nodes, &mut found);
        assert!(found, "o gatilho existe no mundo");
    }
}

//! Falas no MUNDO — [`bevy_speech_bubbles`].
//!
//! As linhas que os scripts emitem por `viber.say` deixaram de aparecer no
//! balão do HUD: a engine resolve o locale, baixa a música e publica
//! [`ScriptSay`]; este plugin desenha um balão no mundo sobre quem falou
//! (NPC, quadro de avisos, altar) e desliga o balão nativo com
//! [`ExternalSayPresenter`].
//!
//! O balão é um quad billboard com o texto rasterizado na CPU pela crate
//! (`BubbleAssets` + `Bubble` + `BubblePx`), por isso fica atrás de paredes e
//! acompanha o dono. Uma linha de callback que corre no HUD (a barra de fala
//! vive lá) não tem `Transform` de mundo: o orador cai para o alvo do [E] e,
//! sem ele, para um toast — o mesmo caminho do antigo balão de sistema.
//!
//! Cada balão compra uma `Image` e um `StandardMaterial`; o plugin apaga-os
//! no fim da vida (a crate não tem GC de assets) e substitui o balão anterior
//! do mesmo dono, para uma fala nova não empilhar texto velho.

use std::collections::HashMap;
use std::marker::PhantomData;

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::time::Real;
use bevy_speech_bubbles::{
    BalloonScale, Bubble, BubbleAssets, BubbleKind, BubblePx, BubbleStyle, DefaultBalloonSkin,
    Emotion, SpeechBubblesPlugin, build_bubble, dwell_secs, track_bubbles,
};
use viber::dialogue::ExternalSayPresenter;
use viber::interact::InteractionFocus;
use viber::luau::{ScriptSay, ScriptToast};
use viber::recipes::spawn::OrbitCamera;

/// A fonte de display da casa (Cinzel Bold, a mesma do HUD e do texto de
/// dano) — os balões falam com a voz tipográfica do jogo.
const FONT: &[u8] = viber::hud::assets::DISPLAY_FONT;

/// Mínimo/máximo de vida de um balão (s). O mínimo evita um flash ilegível;
/// o máximo impede que uma fala longa fique presa depois de a conversa mudar.
const MIN_SECONDS: f32 = 1.5;
const MAX_SECONDS: f32 = 12.0;

/// Tinta quente do HUD: pedra escura, ouro da casa e texto-papel.
pub const VAEL_BALLOON_SKIN: bevy_speech_bubbles::BalloonSkin = bevy_speech_bubbles::BalloonSkin {
    fill: [26, 22, 18, 242],
    border: [214, 185, 121, 255],
    text: [245, 238, 224, 255],
    pixel: false,
    thought_fill: [22, 20, 18, 236],
    thought_border: [150, 140, 120, 240],
    text_px: 40.0,
};

/// Vida útil de um balão: o pedido do script, mas nunca menos do que o tempo
/// de leitura do texto (a crate mede palavras a 200 wpm, com piso próprio).
pub fn bubble_seconds(text: &str, seconds: f32) -> f32 {
    seconds
        .max(dwell_secs(text))
        .clamp(MIN_SECONDS, MAX_SECONDS)
}

/// O balão de uma fala, com os handles para o GC manual.
#[derive(Component)]
struct SpeechFace {
    material: Handle<StandardMaterial>,
    image: Option<Handle<Image>>,
    expires_at: f32,
    /// Texto que o balão mostra (linhas do mesmo frame já juntadas). Só o
    /// teste de agrupamento o lê; o runtime não precisa de o manter.
    #[cfg(test)]
    text: String,
}

impl SpeechFace {
    fn remove_assets(&self, images: &mut Assets<Image>, materials: &mut Assets<StandardMaterial>) {
        materials.remove(&self.material);
        if let Some(image) = &self.image {
            images.remove(image);
        }
    }
}

/// Plugin das falas do Vael.
///
/// `C` é o marcador da câmara que os balões encaram (como na crate): o jogo
/// usa [`OrbitCamera`], um teste headless pode usar um marcador próprio.
pub struct VaelSpeechPlugin<C: Component = OrbitCamera>(PhantomData<fn() -> C>);

impl<C: Component> Default for VaelSpeechPlugin<C> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<C: Component> Plugin for VaelSpeechPlugin<C> {
    fn build(&self, app: &mut App) {
        app.add_plugins(SpeechBubblesPlugin::<C>::default())
            // A engine continua a resolver/duckar/publicar `ScriptSay`, mas o
            // balão nativo do HUD fica desligado.
            .insert_resource(ExternalSayPresenter)
            .insert_resource(BalloonScale {
                // Um balão ~2,4 m de largo no mundo (o default 1/210 dava 3 m
                // a esta distância de câmara).
                world_per_px: 1.0 / 260.0,
                // A ponta da cauda fica acima da cabeça de um humanoide.
                anchor_y: 2.15,
            })
            .insert_resource(DefaultBalloonSkin(VAEL_BALLOON_SKIN))
            .add_systems(Startup, setup_bubble_assets)
            .add_systems(
                Update,
                (
                    present_spoken_lines
                        .after(viber::luau::luau_update)
                        .before(track_bubbles::<C>),
                    // Antes do `track_bubbles`: um balão velho nunca é
                    // despawnado pela crate com os assets ainda vivos.
                    expire_speech_faces.before(track_bubbles::<C>),
                ),
            );
    }
}

/// A crate não lê ficheiros de fonte: o Vael empresta-lhe a Cinzel do HUD e
/// um quad unitário.
fn setup_bubble_assets(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>) {
    let quad = meshes.add(Rectangle::new(1.0, 1.0));
    match BubbleAssets::from_font_bytes(quad, FONT.to_vec()) {
        Ok(assets) => {
            commands.insert_resource(assets);
        }
        Err(error) => {
            error!("vael: balões de fala desligados — fonte inválida: {error}");
        }
    }
}

/// Onde a fala aparece: o orador do `ScriptSay` quando tem pose no mundo; o
/// alvo do [E] quando a chamada correu no HUD (callbacks da barra de fala);
/// sem nenhum, o toast do sistema.
fn speaker_in_world(
    line: &ScriptSay,
    speakers: &Query<(&Transform, &Visibility)>,
    focus: Option<&InteractionFocus>,
) -> Option<Entity> {
    line.speaker
        .filter(|entity| speakers.contains(*entity))
        .or_else(|| {
            focus
                .and_then(|focus| focus.winner(KeyCode::KeyE))
                .filter(|entity| speakers.contains(*entity))
        })
}

/// Publica um balão por fala, substituindo o anterior do mesmo dono.
fn present_spoken_lines(
    mut commands: Commands,
    mut spoken: MessageReader<ScriptSay>,
    time: Res<Time<Real>>,
    assets: Option<Res<BubbleAssets>>,
    skin: Res<DefaultBalloonSkin>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    speakers: Query<(&Transform, &Visibility)>,
    existing: Query<(Entity, &Bubble, &SpeechFace)>,
    focus: Option<Res<InteractionFocus>>,
    mut toasts: MessageWriter<ScriptToast>,
) {
    // Várias linhas para o mesmo dono no MESMO frame (a resposta de uma
    // pista seguida do "já tens o suficiente") juntam-se no mesmo balão: o
    // HUD antigo só mostrava a última e o jogador perdia texto.
    let mut latest: HashMap<Entity, (String, f32)> = HashMap::new();
    let focus = focus.as_deref();
    for line in spoken.read() {
        match speaker_in_world(line, &speakers, focus) {
            Some(owner) => {
                if let Some((text, seconds)) = latest.get_mut(&owner) {
                    text.push('\n');
                    text.push_str(&line.text);
                    *seconds = seconds.max(line.seconds);
                } else {
                    latest.insert(owner, (line.text.clone(), line.seconds));
                }
            }
            None => {
                // Sem dono no mundo: o texto não se perde, vira toast.
                toasts.write(ScriptToast(line.text.clone()));
            }
        }
    }
    if latest.is_empty() {
        return;
    }
    let Some(assets) = assets.as_deref() else {
        for (owner, (text, _)) in latest {
            debug!(
                ?owner,
                "vael: fala sem BubbleAssets — o texto não aparece ({text})"
            );
        }
        return;
    };

    for (owner, (text, seconds)) in latest {
        // O balão velho do mesmo dono sai (com os assets) antes do novo.
        for (entity, bubble, face) in &existing {
            if bubble.owner == owner {
                face.remove_assets(&mut images, &mut materials);
                commands.entity(entity).despawn();
            }
        }
        let rendered = build_bubble(
            assets,
            &mut images,
            &mut materials,
            &BubbleStyle {
                kind: BubbleKind::Speech,
                emotion: Emotion::Neutral,
                tail: true,
                skin: skin.0,
            },
            &text,
        );
        let image = materials
            .get(&rendered.material)
            .and_then(|material| material.base_color_texture.clone());
        commands.spawn((
            Bubble {
                owner,
                offset: Vec2::ZERO,
            },
            BubblePx(rendered.px),
            SpeechFace {
                material: rendered.material.clone(),
                image,
                expires_at: time.elapsed_secs() + bubble_seconds(&text, seconds),
                #[cfg(test)]
                text: text.clone(),
            },
            Mesh3d(assets.quad.clone()),
            MeshMaterial3d(rendered.material),
            NotShadowCaster,
            // Nasce escondido. Com o `apply_deferred` que o `.before(track)`
            // força, o track posiciona-o já neste frame; se um dia a ordem
            // mudar, o balão continua escondido em vez de aparecer na origem.
            Visibility::Hidden,
            Transform::from_scale(Vec3::new(rendered.size.x, rendered.size.y, 1.0)),
        ));
    }
}

/// Fecha o balão no fim da vida ou quando o dono desaparece (troca de cena),
/// devolvendo a textura e o material ao `Assets` antes do despawn.
fn expire_speech_faces(
    mut commands: Commands,
    time: Res<Time<Real>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    faces: Query<(Entity, &SpeechFace, &Bubble)>,
    owners: Query<(&Transform, &Visibility)>,
) {
    let now = time.elapsed_secs();
    for (entity, face, bubble) in &faces {
        if now >= face.expires_at || !owners.contains(bubble.owner) {
            face.remove_assets(&mut images, &mut materials);
            commands.entity(entity).despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Marcador de câmara de teste: o plugin é genérico e a crate só precisa
    /// de UM `GlobalTransform` com o marcador.
    #[derive(Component)]
    struct TestCamera;

    fn headless_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(bevy::asset::AssetPlugin::default())
            .init_asset::<Mesh>()
            .init_asset::<Image>()
            .init_asset::<StandardMaterial>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<InteractionFocus>()
            .add_message::<ScriptSay>()
            .add_message::<ScriptToast>()
            .add_plugins(VaelSpeechPlugin::<TestCamera>::default());
        app.world_mut()
            .spawn((TestCamera, Transform::default(), GlobalTransform::default()));
        app.update(); // Startup: `BubbleAssets`
        app
    }

    fn only_bubble(app: &mut App) -> Entity {
        let mut query = app.world_mut().query_filtered::<Entity, With<Bubble>>();
        query.single(app.world()).expect("um balão no mundo")
    }

    #[test]
    fn short_lines_keep_the_script_time() {
        assert_eq!(bubble_seconds("Olá.", 4.0), 4.0);
    }

    #[test]
    fn long_lines_wait_for_reading() {
        let text = "Uma linha longa com muitas palavras para o jogador ter tempo \
                    de ler antes de o balão desaparecer do mundo";
        assert!(bubble_seconds(text, 4.0) > 4.0);
        assert!(bubble_seconds(text, 0.0) > 1.0);
    }

    #[test]
    fn a_huge_lifetime_is_capped() {
        assert_eq!(bubble_seconds("curto", 60.0), MAX_SECONDS);
    }

    #[test]
    fn the_hud_font_is_a_usable_balloon_font() {
        let assets = BubbleAssets::from_font_bytes(Handle::<Mesh>::default(), FONT.to_vec());
        assert!(
            assets.is_ok(),
            "Cinzel (a fonte do HUD): {:?}",
            assets.err()
        );
    }

    /// Os scripts que falam passaram a usar `talk.say`/`say.say` e o
    /// `viber.entity_id()`: compila-os (sem executar) para o jogo não
    /// descobrir um erro de sintaxe só quando o NPC abre a boca.
    #[test]
    fn the_scripts_that_talk_compile() {
        let lua = mlua::Lua::new();
        for path in [
            "scripts/ui/say.lua",
            "scripts/npc/dialogue.lua",
            "scripts/npc/decision-quest.lua",
            "scripts/npc/forest-wolves.lua",
            "scripts/townsfolk.lua",
            "scripts/notice-board.lua",
            "scripts/lib/riddle.lua",
        ] {
            let source =
                std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{path}: {error}"));
            if let Err(error) = lua.load(&source).set_name(path.to_string()).into_function() {
                panic!("{path}: {error}");
            }
        }
    }

    #[test]
    fn a_spoken_line_becomes_a_world_bubble() {
        let mut app = headless_app();
        let speaker = app
            .world_mut()
            .spawn((Transform::default(), Visibility::Inherited))
            .id();
        app.world_mut().write_message(ScriptSay {
            speaker: Some(speaker),
            text: "Olá, viajante.".into(),
            seconds: 4.0,
        });
        app.update();
        let bubble = only_bubble(&mut app);
        assert_eq!(app.world().get::<Bubble>(bubble).unwrap().owner, speaker);
        assert!(app.world().get::<BubblePx>(bubble).is_some());
        assert!(app.world().get::<Mesh3d>(bubble).is_some());

        // O `.before(track_bubbles)` força o sync point: o balão já sai
        // ancorado e visível no MESMO frame (nada de um flash na origem).
        let transform = *app.world().get::<Transform>(bubble).unwrap();
        assert!(
            transform.translation.y > 1.0,
            "balão ancorado acima do NPC: {transform:?}"
        );
        assert!(transform.scale.x > 0.1, "escala do balão: {transform:?}");
        assert_eq!(
            *app.world().get::<Visibility>(bubble).unwrap(),
            Visibility::Inherited,
            "o track revela o balão do dono visível"
        );
    }

    #[test]
    fn two_lines_in_the_same_frame_share_one_bubble() {
        let mut app = headless_app();
        let speaker = app
            .world_mut()
            .spawn((Transform::default(), Visibility::Inherited))
            .id();
        for text in ["A pista é o sino.", "Já tens o suficiente."] {
            app.world_mut().write_message(ScriptSay {
                speaker: Some(speaker),
                text: text.into(),
                seconds: 4.0,
            });
        }
        app.update();
        let bubble = only_bubble(&mut app);
        let face = app.world().get::<SpeechFace>(bubble).unwrap();
        assert!(
            face.text.contains("A pista é o sino.") && face.text.contains("Já tens o suficiente."),
            "as duas linhas no mesmo balão: {:?}",
            face.text
        );
    }

    #[test]
    fn a_callback_line_uses_the_focused_npc() {
        let mut app = headless_app();
        let npc = app
            .world_mut()
            .spawn((Transform::default(), Visibility::Inherited))
            .id();
        app.world_mut()
            .resource_mut::<InteractionFocus>()
            .consider(KeyCode::KeyE, npc, 1.0);
        app.world_mut().write_message(ScriptSay {
            // O `say.lua` corre no HUD: o orador chega nulo/inválido.
            speaker: None,
            text: "Não te ouvi.".into(),
            seconds: 2.0,
        });
        app.update();
        let bubble = only_bubble(&mut app);
        assert_eq!(app.world().get::<Bubble>(bubble).unwrap().owner, npc);
    }

    #[test]
    fn a_line_without_a_world_speaker_becomes_a_toast() {
        let mut app = headless_app();
        app.world_mut().write_message(ScriptSay {
            speaker: None,
            text: "Não percebi. Tenta outra vez.".into(),
            seconds: 2.0,
        });
        app.update();
        let toasts: Vec<ScriptToast> = app
            .world()
            .resource::<bevy::ecs::message::Messages<ScriptToast>>()
            .iter_current_update_messages()
            .cloned()
            .collect();
        assert!(
            toasts
                .iter()
                .any(|toast| toast.0 == "Não percebi. Tenta outra vez."),
            "sem dono vira toast: {toasts:?}"
        );
        let mut query = app.world_mut().query_filtered::<Entity, With<Bubble>>();
        assert_eq!(query.iter(app.world()).count(), 0);
    }

    #[test]
    fn an_expired_bubble_gives_its_assets_back() {
        let mut app = headless_app();
        let speaker = app
            .world_mut()
            .spawn((Transform::default(), Visibility::Inherited))
            .id();
        app.world_mut().write_message(ScriptSay {
            speaker: Some(speaker),
            text: "Até à próxima.".into(),
            seconds: 4.0,
        });
        app.update();
        let bubble = only_bubble(&mut app);
        let face = app.world().get::<SpeechFace>(bubble).unwrap();
        let material = face.material.clone();
        let image = face.image.clone().unwrap();
        app.world_mut()
            .get_mut::<SpeechFace>(bubble)
            .unwrap()
            .expires_at = -1.0;

        app.update();
        assert!(
            app.world().get_entity(bubble).is_err(),
            "o balão expirado sai"
        );
        assert!(
            !app.world()
                .resource::<Assets<StandardMaterial>>()
                .contains(&material),
            "o material volta ao Assets"
        );
        assert!(
            !app.world().resource::<Assets<Image>>().contains(&image),
            "a textura volta ao Assets"
        );
    }
}

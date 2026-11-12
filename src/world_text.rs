//! Texto flutuante no MUNDO — dano, cura, coleta e combo — em
//! [`bevy_rich_text3d`].
//!
//! A engine continua a ser a única boca de entrada: os scripts pedem
//! `viber.damage_number` (dano, cura, esquiva, bloqueio, loot, colheita) e a
//! engine emite [`DamageNumberEvent`]; o `harvest.rs` escreve a mesma
//! mensagem. O pool 2D projectado da engine fica desligado
//! (`AppOptions::damage_numbers_2d = false`) e este plugin desenha o texto
//! como geometria no mundo: um quad por glifo, atlas partilhado, billboard
//! para a câmara, subida e fade com o mesmo tempo do feedback antigo.
//!
//! ## Porquê um pool e um material por slot
//!
//! Criar entidade/mesh/material por número não é grátis e os assets do Bevy
//! não têm GC: um `StandardMaterial` novo por popup ficava para sempre em
//! `Assets`. O pool fixo resolve os dois lados. O material é POR SLOT só por
//! causa do fade: a cor do texto é cozida nos vértices pelo `Text3d` e mudar
//! `Text3dStyle.color` a cada frame forçaria re-tesselação do texto; o alpha
//! do `base_color` do material é grátis e o atlas é o mesmo.
//!
//! O `Text3d` é re-gerado quando o texto muda (uma vez por popup) e o pop de
//! escala usa o `Transform`, não o `world_scale` do estilo — mudar o estilo
//! também re-tesselava.

use std::sync::Arc;

use bevy::color::Srgba;
use bevy::material::AlphaMode;
use bevy::prelude::*;
use bevy_rich_text3d::{
    LoadFonts, Text3d, Text3dPlugin, Text3dStyle, TextAlign, TextAnchor, TextAtlas,
};
use viber::feedback::{DamageNumberEvent, damage_number_scale, number_font_size};

/// Vida útil de um número (s), igual ao feedback 2D da engine.
pub const TEXT_LIFETIME: f32 = 0.9;
/// Slots do pool. A engine 2D tinha 14; o mundo aguenta mais (AOE, loot em
/// cadeia) sem alocar nada por popup.
pub const TEXT_POOL: usize = 32;
/// Subida em m/s. Igual ao antigo: 0,6 m/s no total de [`TEXT_LIFETIME`].
pub const TEXT_RISE: f32 = 0.6;
/// Altura de uma linha normal em metros de mundo (os 24 px do HUD antigo).
pub const TEXT_WORLD_HEIGHT: f32 = 0.45;
/// Tamanho usado para rasterizar os glifos no atlas. O `world_scale` reduz o
/// mesh para metros; rasterizar a 1 px daria texto serrilhado.
pub const GLYPH_SIZE: f32 = 64.0;
/// Fonte de display do HUD (Cinzel Bold, OFL) já embutida na engine — o texto
/// do mundo e a UI ficam com a mesma voz.
pub const DISPLAY_FONT: &[u8] = viber::hud::assets::DISPLAY_FONT;
/// Família da fonte carregada de [`DISPLAY_FONT`].
pub const FONT_FAMILY: &str = "Cinzel";

/// Texto flutuante pedido por código do jogo.
///
/// A ponte de [`DamageNumberEvent`] usa o construtor; código novo (quests,
/// avisos, marcos) pode escrever a mensagem directamente.
#[derive(Debug, Clone, bevy::ecs::message::Message)]
pub struct WorldTextEvent {
    pub position: Vec3,
    pub text: String,
    pub color: Color,
    /// Multiplicador sobre a altura base (`1.0` = dano normal).
    pub scale: f32,
    /// Vida útil em segundos (mínimo aplicado na activação).
    pub lifetime: f32,
    /// Subida em m/s.
    pub rise: f32,
}

impl WorldTextEvent {
    pub fn new(position: Vec3, text: impl Into<String>, color: Color) -> Self {
        Self {
            position,
            text: text.into(),
            color,
            scale: 1.0,
            lifetime: TEXT_LIFETIME,
            rise: TEXT_RISE,
        }
    }
}

/// Slot do pool. `active = false` é um slot livre; o cursor do pool
/// (`Local<usize>`) roda por todos, por isso um slot vivo só é reutilizado
/// depois de [`TEXT_POOL`] popups (≈ 0,9 s de combate intenso).
#[derive(Component, Debug)]
struct WorldTextSlot {
    active: bool,
    age: f32,
    lifetime: f32,
    rise: f32,
    origin: Vec3,
    /// Escala base antes do pop (crítico/remate maiores).
    scale: f32,
}

impl WorldTextSlot {
    fn inactive() -> Self {
        Self {
            active: false,
            age: 0.0,
            lifetime: TEXT_LIFETIME,
            rise: TEXT_RISE,
            origin: Vec3::ZERO,
            scale: 1.0,
        }
    }
}

/// Entidades do pool, na ordem em que o cursor as visita.
#[derive(Resource, Default)]
struct WorldTextPool {
    entities: Vec<Entity>,
}

// ── lógica pura (testada) ───────────────────────────────────────────────

/// Escala de um texto: o tamanho relativo do HUD 2D (24/28/32 px) vira
/// multiplicador da altura de mundo. `CRIT`/`!`/`x2` continuam maiores.
pub fn text_scale(text: &str) -> f32 {
    number_font_size(text) / 24.0
}

/// Subida (m) de um texto com `age` segundos.
pub fn text_offset(age: f32, rise: f32) -> f32 {
    age.max(0.0) * rise
}

/// Alpha do fade linear (1 → 0) ao longo da vida útil.
pub fn text_alpha(age: f32, lifetime: f32) -> f32 {
    if lifetime <= 0.0 {
        return 0.0;
    }
    (1.0 - age / lifetime).clamp(0.0, 1.0)
}

/// Escala final no instante `age`: base × pop com overshoot.
pub fn text_scale_at(base: f32, age: f32) -> f32 {
    base.max(0.0) * damage_number_scale(age)
}

// ── plugin ──────────────────────────────────────────────────────────────

/// Plugin do texto 3D do mundo. Requer o `DefaultPlugins` (ou pelo menos os
/// assets de imagem/mesh) — é adicionado depois do `build_app` da engine.
pub struct VaelWorldTextPlugin;

impl Plugin for VaelWorldTextPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<WorldTextEvent>()
            // A bridge lê a mensagem da engine; registá-la aqui mantém o
            // plugin testável sem o `FeedbackPlugin`.
            .add_message::<DamageNumberEvent>()
            .add_plugins(Text3dPlugin {
                default_atlas_dimension: (1024, 1024),
                load_system_fonts: false,
                // O estilo usa `Family::Name("Cinzel")`; o default "serif"
                // também resolve para a mesma face se alguém o omitir.
                serif_family: FONT_FAMILY.to_string(),
                ..Default::default()
            })
            .insert_resource(LoadFonts {
                font_embedded: vec![DISPLAY_FONT],
                ..Default::default()
            })
            .add_systems(Startup, spawn_world_text_pool)
            .add_systems(
                Update,
                (
                    // Os scripts escrevem `viber.damage_number` no
                    // `luau_update`; ler depois evita um frame de atraso.
                    queue_damage_numbers.after(viber::luau::luau_update),
                    activate_world_text,
                    animate_world_text,
                )
                    .chain(),
            );
    }
}

/// Cria o pool de textos e um `StandardMaterial` por slot (fade).
fn spawn_world_text_pool(mut commands: Commands, mut materials: ResMut<Assets<StandardMaterial>>) {
    let template = StandardMaterial {
        base_color_texture: Some(TextAtlas::DEFAULT_IMAGE.clone()),
        alpha_mode: AlphaMode::Blend,
        // Texto de HUD no mundo: não apanha luz, nem névoa, e vê-se dos dois
        // lados (o billboard pode ficar de costas num frame).
        unlit: true,
        fog_enabled: false,
        cull_mode: None,
        // Tira o texto de cima da pele/terreno quando o alvo está encostado.
        depth_bias: 0.5,
        ..Default::default()
    };
    let mut entities = Vec::with_capacity(TEXT_POOL);
    for i in 0..TEXT_POOL {
        let material = materials.add(template.clone());
        let entity = commands
            .spawn((
                Text3d::new(""),
                Text3dStyle {
                    size: GLYPH_SIZE,
                    world_scale: Some(Vec2::splat(TEXT_WORLD_HEIGHT)),
                    font: Arc::from(FONT_FAMILY),
                    align: TextAlign::Center,
                    anchor: TextAnchor::CENTER,
                    color: Srgba::WHITE,
                    ..Default::default()
                },
                Mesh3d::default(),
                MeshMaterial3d(material),
                Transform::default(),
                Visibility::Hidden,
                WorldTextSlot::inactive(),
                Name::new(format!("fx:world-text-{i}")),
            ))
            .id();
        entities.push(entity);
    }
    commands.insert_resource(WorldTextPool { entities });
}

/// Traduz o feedback genérico da engine (`DamageNumberEvent`) em texto do
/// mundo. O tamanho acompanha o tipo de golpe como no HUD 2D.
fn queue_damage_numbers(
    mut incoming: MessageReader<DamageNumberEvent>,
    mut out: MessageWriter<WorldTextEvent>,
) {
    for event in incoming.read() {
        out.write(WorldTextEvent {
            position: event.position,
            text: event.text.clone(),
            color: event.color,
            scale: text_scale(&event.text),
            lifetime: TEXT_LIFETIME,
            rise: TEXT_RISE,
        });
    }
}

/// Activa o próximo slot do cursor para cada texto pedido.
#[allow(clippy::type_complexity)]
fn activate_world_text(
    mut cursor: Local<usize>,
    pool: Res<WorldTextPool>,
    mut incoming: MessageReader<WorldTextEvent>,
    camera: Query<&GlobalTransform, With<Camera3d>>,
    mut slots: Query<(
        &mut WorldTextSlot,
        &mut Text3d,
        &mut Text3dStyle,
        &mut Transform,
        &mut Visibility,
        &MeshMaterial3d<StandardMaterial>,
    )>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if pool.entities.is_empty() {
        return;
    }
    // O billboard já nasce certo: sem isto o primeiro frame do texto ficava
    // de perfil.
    let rotation = camera
        .iter()
        .next()
        .map(|transform| transform.rotation())
        .unwrap_or(Quat::IDENTITY);
    for event in incoming.read() {
        if event.text.is_empty() {
            continue;
        }
        let entity = pool.entities[*cursor % pool.entities.len()];
        *cursor = (*cursor + 1) % pool.entities.len();
        let Ok((mut slot, mut text, mut style, mut transform, mut visibility, material)) =
            slots.get_mut(entity)
        else {
            continue;
        };

        slot.active = true;
        slot.age = 0.0;
        slot.lifetime = event.lifetime.max(0.05);
        slot.rise = event.rise;
        slot.origin = event.position;
        slot.scale = event.scale.max(0.05);

        if !text.set_single(&event.text) {
            *text = Text3d::new(&event.text);
        }
        // A cor vive nos vértices do mesh do texto (e é re-teselada uma vez
        // por popup); o fade é o alpha do material, por slot.
        style.color = event.color.to_srgba();

        transform.translation = event.position;
        transform.rotation = rotation;
        transform.scale = Vec3::splat(slot.scale);
        *visibility = Visibility::Inherited;
        if let Some(mut material) = materials.get_mut(&material.0) {
            material.base_color = Color::WHITE;
        }
    }
}

/// Sobe, faz billboard, aplica o pop de escala e desvanece.
#[allow(clippy::type_complexity)]
fn animate_world_text(
    time: Res<Time>,
    camera: Query<&GlobalTransform, With<Camera3d>>,
    mut slots: Query<(
        &mut WorldTextSlot,
        &mut Transform,
        &mut Visibility,
        &MeshMaterial3d<StandardMaterial>,
    )>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let rotation = camera
        .iter()
        .next()
        .map(|transform| transform.rotation())
        .unwrap_or(Quat::IDENTITY);
    let dt = time.delta_secs();
    for (mut slot, mut transform, mut visibility, material) in &mut slots {
        if !slot.active {
            continue;
        }
        slot.age += dt;
        if slot.age >= slot.lifetime {
            slot.active = false;
            *visibility = Visibility::Hidden;
            if let Some(mut material) = materials.get_mut(&material.0) {
                material.base_color.set_alpha(0.0);
            }
            continue;
        }
        transform.translation = slot.origin + Vec3::Y * text_offset(slot.age, slot.rise);
        transform.rotation = rotation;
        transform.scale = Vec3::splat(text_scale_at(slot.scale, slot.age));
        // Reafirma a visibilidade: um número que nasce atrás da câmara não
        // pode ficar escondido para o resto da vida.
        *visibility = Visibility::Inherited;
        if let Some(mut material) = materials.get_mut(&material.0) {
            material
                .base_color
                .set_alpha(text_alpha(slot.age, slot.lifetime));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_text_scale_ranks_hits() {
        assert!((text_scale("-25") - 1.0).abs() < 1e-6);
        assert!((text_scale("-50 x2") - 28.0 / 24.0).abs() < 1e-6);
        assert!((text_scale("-63 CRIT!") - 32.0 / 24.0).abs() < 1e-6);
    }

    #[test]
    fn test_rise_and_fade_curve() {
        assert_eq!(text_offset(0.0, TEXT_RISE), 0.0);
        assert!((text_offset(TEXT_LIFETIME, TEXT_RISE) - 0.54).abs() < 1e-5);
        assert_eq!(text_alpha(0.0, TEXT_LIFETIME), 1.0);
        assert_eq!(text_alpha(TEXT_LIFETIME, TEXT_LIFETIME), 0.0);
        assert_eq!(text_alpha(TEXT_LIFETIME * 2.0, TEXT_LIFETIME), 0.0);
        assert!((text_alpha(TEXT_LIFETIME * 0.5, TEXT_LIFETIME) - 0.5).abs() < 1e-6);
        // O pop de escala é o da engine: pico de 1,25× aos 40 ms.
        assert!((text_scale_at(1.0, 0.0) - 1.0).abs() < 1e-6);
        assert!((text_scale_at(1.0, 0.04) - 1.25).abs() < 1e-3);
        assert!((text_scale_at(2.0, 0.5) - 2.0).abs() < 1e-6);
    }

    #[test]
    fn test_slot_activation_sets_text_and_transform() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<WorldTextEvent>();
        app.init_resource::<Assets<StandardMaterial>>();
        let material = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial::default());
        let entity = app
            .world_mut()
            .spawn((
                Text3d::new(""),
                Text3dStyle::default(),
                Mesh3d::default(),
                MeshMaterial3d(material),
                Transform::default(),
                Visibility::Hidden,
                WorldTextSlot::inactive(),
            ))
            .id();
        app.insert_resource(WorldTextPool {
            entities: vec![entity],
        });
        app.add_systems(Update, activate_world_text);

        app.world_mut().write_message(WorldTextEvent::new(
            Vec3::new(1.0, 2.0, 3.0),
            "-10",
            Color::srgb(1.0, 0.45, 0.2),
        ));
        app.update();

        let world = app.world_mut();
        let slot = world.get::<WorldTextSlot>(entity).expect("slot");
        assert!(slot.active);
        assert_eq!(slot.origin, Vec3::new(1.0, 2.0, 3.0));
        let text = world.get::<Text3d>(entity).expect("texto");
        assert_eq!(text.get_single(), Some("-10"));
        let style = world.get::<Text3dStyle>(entity).expect("estilo");
        assert_eq!(style.color, Color::srgb(1.0, 0.45, 0.2).to_srgba());
        let transform = world.get::<Transform>(entity).expect("transform");
        assert_eq!(transform.translation, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(
            *world.get::<Visibility>(entity).expect("vis"),
            Visibility::Inherited
        );
    }
}

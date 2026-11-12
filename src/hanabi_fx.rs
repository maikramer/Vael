//! Protótipo de partículas GPU com `bevy_hanabi`.
//!
//! Não substitui o sistema CPU do Viber: acrescenta um efeito de brasas às
//! fogueiras/fornos nomeados, atrás da feature `hanabi` e de `VAEL_HANABI=1`.
//! O objetivo é medir/ver o efeito num ponto real do mundo antes de decidir
//! se vale migrar presets de fogo de alta contagem.

use bevy::prelude::*;
use bevy_hanabi::prelude::*;

/// Liga o protótipo só quando pedido explicitamente.
fn enabled_from_env() -> bool {
    matches!(
        std::env::var("VAEL_HANABI")
            .ok()
            .map(|raw| raw.trim().to_ascii_lowercase())
            .as_deref(),
        Some("1" | "true" | "on" | "yes")
    )
}

/// Handle do efeito construído no arranque.
#[derive(Resource, Debug, Clone)]
struct VaelEmberEffect(Handle<EffectAsset>);

/// Marca a entidade que já recebeu o emissor (evita duplicar no hot-reload).
#[derive(Component)]
struct HanabiEmbers;

/// Plugin do protótipo Hanabi.
pub struct VaelHanabiPlugin;

impl Plugin for VaelHanabiPlugin {
    fn build(&self, app: &mut App) {
        if !enabled_from_env() {
            return;
        }
        app.add_plugins(HanabiPlugin)
            .add_systems(Startup, create_ember_effect)
            .add_systems(Update, attach_ember_emitters);
    }
}

fn create_ember_effect(mut effects: ResMut<Assets<EffectAsset>>, mut commands: Commands) {
    let handle = effects.add(build_ember_effect());
    commands.insert_resource(VaelEmberEffect(handle));
}

/// Efeito simples: partículas pequenas, quentes, que sobem e apagam.
fn build_ember_effect() -> EffectAsset {
    let mut color = bevy_hanabi::Gradient::new();
    color.add_key(0.0, Vec4::new(1.0, 0.62, 0.18, 1.0));
    color.add_key(0.25, Vec4::new(1.0, 0.30, 0.04, 0.95));
    color.add_key(0.75, Vec4::new(0.45, 0.08, 0.01, 0.45));
    color.add_key(1.0, Vec4::new(0.05, 0.01, 0.0, 0.0));

    let mut size = bevy_hanabi::Gradient::new();
    size.add_key(0.0, Vec3::splat(0.10));
    size.add_key(0.5, Vec3::splat(0.07));
    size.add_key(1.0, Vec3::splat(0.01));

    let writer = ExprWriter::new();
    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    let init_lifetime = SetAttributeModifier::new(Attribute::LIFETIME, writer.lit(1.3).expr());
    let init_pos = SetPositionSphereModifier {
        center: writer.lit(Vec3::ZERO).expr(),
        radius: writer.lit(0.28).expr(),
        dimension: ShapeDimension::Volume,
    };
    let init_vel = SetVelocitySphereModifier {
        center: writer.lit(Vec3::ZERO).expr(),
        speed: writer.lit(0.85).expr(),
    };

    EffectAsset::new(256, SpawnerSettings::rate(22.0.into()), writer.finish())
        .with_name("vael_embers")
        .with_alpha_mode(bevy_hanabi::AlphaMode::Add)
        .init(init_pos)
        .init(init_vel)
        .init(init_age)
        .init(init_lifetime)
        .render(ColorOverLifetimeModifier::new(color))
        .render(SizeOverLifetimeModifier {
            gradient: size,
            screen_space_size: false,
        })
}

/// Pendura o emissor nas fogueiras/fornos que vão nascendo com o mundo.
fn attach_ember_emitters(
    mut commands: Commands,
    effect: Option<Res<VaelEmberEffect>>,
    named: Query<(Entity, &Name), (Added<Name>, Without<HanabiEmbers>)>,
) {
    let Some(effect) = effect else {
        return;
    };
    for (entity, name) in &named {
        let name = name.as_str().to_ascii_lowercase();
        if !name.contains("campfire") && !name.contains("furnace") {
            continue;
        }
        commands
            .spawn((
                Name::new("vael.embers"),
                ParticleEffect::new(effect.0.clone()),
                Transform::from_xyz(0.0, 0.45, 0.0),
                ChildOf(entity),
            ))
            .insert(HanabiEmbers);
        commands.entity(entity).insert(HanabiEmbers);
    }
}

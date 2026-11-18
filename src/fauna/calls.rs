//! Local wildlife calls on the ambience bus.
use super::birds::WoodlandBird;
use bevy::{camera::RenderTarget, prelude::*};
use bevy_kira_audio::{AudioChannel, AudioControl, AudioSource};
use viber::music::{AmbienceBus, linear_to_db};

#[derive(Resource)]
pub(super) struct Calls {
    birds: [Handle<AudioSource>; 2],
    fox: Handle<AudioSource>,
}

#[derive(Message)]
pub(super) struct FoxAlarm(pub Vec3);

type Listener<'w, 's> = Query<
    'w,
    's,
    (
        &'static GlobalTransform,
        &'static Camera,
        Option<&'static RenderTarget>,
    ),
    With<Camera3d>,
>;

fn ear<'a>(cameras: &'a Listener) -> Option<&'a GlobalTransform> {
    cameras
        .iter()
        .find(|(_, c, t)| bevy_ext_core::camera::is_window_camera(Some(c), *t))
        .map(|(tf, _, _)| tf)
}

fn play(
    bus: &AudioChannel<AmbienceBus>,
    clip: &Handle<AudioSource>,
    ear: &GlobalTransform,
    source: Vec3,
    gain: f32,
    rate: f64,
) {
    let delta = source - ear.translation();
    let attenuation = (1. - delta.length() / 32.).clamp(0., 1.);
    if attenuation > 0. {
        bus.play(clip.clone())
            .with_volume(linear_to_db(gain * attenuation))
            .with_panning(delta.normalize_or_zero().dot(*ear.right()) * 0.85)
            .with_playback_rate(rate);
    }
}

pub(super) fn load(
    mut commands: Commands,
    server: Res<AssetServer>,
    config: Option<Res<viber::config::GameConfig>>,
) {
    let dir = config
        .as_deref()
        .map(|c| c.assets.sfx_dir.as_path())
        .unwrap_or(std::path::Path::new("assets/audio/sfx"));
    let dir = dir.join("creatures");
    commands.insert_resource(Calls {
        birds: ["jay_call.ogg", "owl_hoot.ogg"].map(|file| server.load(dir.join(file))),
        fox: server.load(dir.join("fox_bark.ogg")),
    });
}

pub(super) fn listen(
    time: Res<Time>,
    clips: Res<Calls>,
    bus: Option<Res<AudioChannel<AmbienceBus>>>,
    cameras: Listener,
    birds: Query<(&WoodlandBird, &Transform)>,
    mut next: Local<f32>,
) {
    let now = time.elapsed_secs();
    if now < *next {
        return;
    }
    *next = now + 1.;
    let Some(bus) = bus else { return };
    let Some(ear) = ear(&cameras) else { return };
    let caller = birds
        .iter()
        .filter(|(b, t)| {
            b.resting() && t.translation.distance_squared(ear.translation()) < 32. * 32.
        })
        .max_by_key(|(b, _)| ((now * 0.47 + b.phase).sin() * 100.) as i32);
    let Some((bird, tf)) = caller else { return };
    play(
        &bus,
        &clips.birds[bird.owl as usize],
        ear,
        tf.translation,
        0.31,
        0.94 + bird.phase as f64 * 0.021,
    );
    *next = now + if bird.owl { 13. } else { 7.5 } + bird.phase * 0.85;
}

pub(super) fn fox_alarm(
    time: Res<Time>,
    mut alarms: MessageReader<FoxAlarm>,
    clips: Res<Calls>,
    bus: Option<Res<AudioChannel<AmbienceBus>>>,
    cameras: Listener,
    mut next: Local<f32>,
) {
    let (Some(bus), Some(ear)) = (bus, ear(&cameras)) else {
        alarms.clear();
        return;
    };
    let now = time.elapsed_secs();
    for FoxAlarm(pos) in alarms.read() {
        if now >= *next && pos.distance_squared(ear.translation()) < 32. * 32. {
            play(&bus, &clips.fox, ear, *pos, 0.38, 1.);
            *next = now + 12.;
        }
    }
}

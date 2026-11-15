//! TEMPORÁRIO: trace do salto/aterragem (`VAEL_JUMP_TRACE=1`).
use avian3d::prelude::*;
use bevy::prelude::*;
use bevy_tnua::prelude::TnuaController;
use viber::player::HeroScheme;

pub struct JumpTracePlugin;
impl Plugin for JumpTracePlugin {
    fn build(&self, app: &mut App) {
        if std::env::var("VAEL_JUMP_TRACE").is_err() {
            return;
        }
        app.add_systems(
            FixedPostUpdate,
            fixed_trace.after(PhysicsSystems::Writeback),
        )
        .add_systems(PostUpdate, frame_trace);
    }
}

fn fixed_trace(
    q: Query<
        (&Position, &LinearVelocity, &TnuaController<HeroScheme>),
        With<viber::player::Player>,
    >,
    sensors: Query<&bevy_tnua_avian3d::prelude::TnuaAvian3dSensorShape>,
    prox: Query<&bevy_tnua::TnuaProximitySensor>,
    time: Res<Time>,
) {
    let _ = sensors;
    for (p, v, c) in &q {
        if v.0.y.abs() < 0.05 && !c.is_airborne().unwrap_or(true) {
            continue;
        }
        let pr: Vec<String> = prox
            .iter()
            .map(|s| {
                s.output
                    .as_ref()
                    .map_or("-".into(), |o| format!("{:.3}", o.proximity))
            })
            .collect();
        eprintln!(
            "JT fix t={:.3} y={:+.3} vy={:+6.2} air={} prox={:?}",
            time.elapsed_secs(),
            p.0.y,
            v.0.y,
            c.is_airborne().unwrap_or(true) as u8,
            pr
        );
    }
}

fn frame_trace(
    q: Query<(
        &Transform,
        &viber::player::Player,
        Option<&viber::animation::CharacterAnimator>,
    )>,
    time: Res<Time>,
) {
    for (t, p, a) in &q {
        let state = a.and_then(|a| a.state);
        if p.grounded
            && p.vel_y.abs() < 0.05
            && !matches!(
                state,
                Some(viber::animation::AnimState::Fall | viber::animation::AnimState::Jump)
            )
        {
            continue;
        }
        eprintln!(
            "JT frm t={:.3} y={:+.3} g={} vy={:+6.2} anim={:?} busy={}",
            time.elapsed_secs(),
            t.translation.y,
            p.grounded as u8,
            p.vel_y,
            state,
            a.map(|a| a.is_busy()).unwrap_or(false)
        );
    }
}

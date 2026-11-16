//! Driver do mapa de ações: teclado, gamepad e rebind gravado no save.
//!
//! Os CONTRATOS ([`Action`], [`Binding`], [`ActionMap`], [`ActionState`],
//! TypePaths `viber::input_map` congelados) vivem no `bevy_ext_core` —
//! reexportados aqui para `viber::input_map::…` continuar a resolver para O
//! MESMO tipo (o netplay do `bevy_ext_engine` escreve o mesmo recurso).
//!
//! O PLUGIN cá fica: [`gather_actions`] respeita o foco de texto da UI
//! declarativa (`UiFocusedInput`) e resolve teclas pelo parser do host
//! Luau (`input_code_from_str`) — serviços do runtime que o core não pode
//! ter. Sem o recurso (apps de teste que chamam o movimento à mão) o
//! teclado antigo continua a valer.

use bevy::input::gamepad::{Gamepad, GamepadAxis, GamepadButton};
use bevy::prelude::*;

pub use bevy_engine_core::input_map::{Action, ActionMap, ActionState, Binding};

fn key_of(name: &str) -> Option<KeyCode> {
    crate::luau::input_code_from_str(name).and_then(|code| match code {
        crate::luau::InputCode::Key(key) => Some(key),
        _ => None,
    })
}

fn pad_of(name: &str) -> Option<GamepadButton> {
    match name {
        "south" => Some(GamepadButton::South),
        "east" => Some(GamepadButton::East),
        "west" => Some(GamepadButton::West),
        "north" => Some(GamepadButton::North),
        "start" => Some(GamepadButton::Start),
        "left_trigger" => Some(GamepadButton::LeftTrigger),
        "left_trigger2" => Some(GamepadButton::LeftTrigger2),
        _ => None,
    }
}

pub fn gather_actions(
    keys: Res<ButtonInput<KeyCode>>,
    map: Res<ActionMap>,
    pads: Query<&Gamepad>,
    // Com um `<UiInput>` focado (a barra de fala do NLU) as teclas são TEXTO:
    // o movimento, o `viber.input.*` e o `viber.interacted` já o respeitam —
    // sem este guarda, escrever "j" na barra disparava o ataque.
    typing: Option<Res<crate::ui::widgets::UiFocusedInput>>,
    mut state: ResMut<ActionState>,
) {
    if typing.as_deref().is_some_and(|f| f.0.is_some()) {
        *state = ActionState::default();
        return;
    }
    let mut move_x = 0.0;
    let mut forward = 0.0;
    if keys.pressed(KeyCode::KeyD) || keys.pressed(KeyCode::ArrowRight) {
        move_x += 1.0;
    }
    if keys.pressed(KeyCode::KeyA) || keys.pressed(KeyCode::ArrowLeft) {
        move_x -= 1.0;
    }
    if keys.pressed(KeyCode::KeyW) || keys.pressed(KeyCode::ArrowUp) {
        forward += 1.0;
    }
    if keys.pressed(KeyCode::KeyS) || keys.pressed(KeyCode::ArrowDown) {
        forward -= 1.0;
    }
    for pad in &pads {
        let x = pad.get(GamepadAxis::LeftStickX).unwrap_or(0.0);
        let y = pad.get(GamepadAxis::LeftStickY).unwrap_or(0.0);
        if x.abs() > 0.2 {
            move_x = x;
        }
        if y.abs() > 0.2 {
            forward = y;
        }
    }
    let just = |action: Action| -> bool {
        let Some(binding) = map.bindings.get(&action) else {
            return false;
        };
        let key = binding
            .key
            .as_deref()
            .and_then(key_of)
            .is_some_and(|k| keys.just_pressed(k));
        let pad = binding
            .pad
            .as_deref()
            .and_then(pad_of)
            .is_some_and(|button| pads.iter().any(|pad| pad.just_pressed(button)));
        key || pad
    };
    let held = |action: Action| -> bool {
        let Some(binding) = map.bindings.get(&action) else {
            return false;
        };
        let key = binding
            .key
            .as_deref()
            .and_then(key_of)
            .is_some_and(|k| keys.pressed(k));
        let pad = binding
            .pad
            .as_deref()
            .and_then(pad_of)
            .is_some_and(|button| pads.iter().any(|pad| pad.pressed(button)));
        key || pad
    };
    *state = ActionState {
        move_x,
        forward,
        jump_just: just(Action::Jump),
        sprint: held(Action::Sprint),
        attack_just: just(Action::Attack),
        interact_just: just(Action::Interact),
        lock_just: just(Action::Lock),
        menu_just: just(Action::Menu),
        crouch_held: held(Action::Crouch),
    };
}

/// Presente quando o jogo escreve o [`ActionState`] por conta própria (o
/// Vael avalia-o com o `jackdaw_input`): o [`gather_actions`] da engine
/// deixa de correr, em vez de calcular um estado que seria reescrito no
/// mesmo frame. O [`ActionMap`] continua a ser a fonte dos binds.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct ExternalActionState;

pub struct InputMapPlugin;

impl Plugin for InputMapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ActionMap>()
            .init_resource::<ActionState>()
            // `just_pressed`/`pressed` são reescritos pelo `InputPlugin` no
            // mesmo schedule; sem a ordem explícita este sistema podia ler o
            // estado do frame ANTERIOR (ação 1 frame atrasada, conforme a
            // ordem que o scheduler escolhesse).
            .add_systems(
                PreUpdate,
                gather_actions
                    .after(bevy::input::InputSystems)
                    .run_if(not(resource_exists::<ExternalActionState>)),
            );
    }
}

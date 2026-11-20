//! Input do Vael sobre o `jackdaw_input`: contextos, acções e bindings como
//! entidades, avaliados no `PreUpdate` pelo mesmo crate que o editor usa.
//!
//! O contrato com o resto do jogo (e com o Lua) não muda: os sistemas leem o
//! [`ActionState`] da engine. O que muda é quem o escreve:
//!
//! - o **teclado/rato** é avaliado pelo `jackdaw_input` em dois contextos:
//!   [`VaelUiInput`] (prioridade 10: mapa, viagem, placa, fechar e o menu) e
//!   [`VaelGameplayInput`] (andar, saltar, atacar…), que fica **inactivo**
//!   enquanto um menu, loja, mapa, viagem ou cinemática estiver aberto
//!   (`MenusOpen`). Enquanto se escreve numa `<UiInput>` o teclado sai dos
//!   dois (`InputSources`). As acções de UI chegam ao jogo por
//!   [`VaelUiActions`];
//! - o **gamepad** continua aqui (o `jackdaw_input` não tem bindings de
//!   comando), com os mesmos botões do [`ActionMap`];
//! - o [`ActionMap`] da engine continua a ser a fonte dos binds (rebind no
//!   menu, gravado no save): quando muda, os `Binding` são refeitos.
//!
//! O `gather_actions` da engine fica parado
//! (`viber::input_map::ExternalActionState`): [`write_action_state`] é o
//! único escritor do `ActionState`.
//!
//! No Play do Jackdaw o editor reencaminha o input como mensagens normais do
//! `bevy_input` para a janela virtual, por isso o mesmo caminho serve o jogo
//! embutido e a janela do SO.

use bevy::input::gamepad::{Gamepad, GamepadAxis, GamepadButton};
use bevy::prelude::*;
use bevy_ext_core::MenusOpen;
use jackdaw_input::prelude::{
    Action as InputAction, ActionOf, ActionSettings, ActionSystems, Binding, BindingOf, Condition,
    ContextActivity, ContextPriority, InputActionPlugin, InputContext, InputSources, TriggerState,
};
use viber::input_map::{Action, ActionMap, ActionState};

/// Contexto de jogo (andar, saltar, atacar, …).
#[derive(Component, Debug, Clone, Copy)]
#[require(InputContext)]
pub struct VaelGameplayInput;

/// Contexto de UI: avaliado antes do jogo e sempre activo (é ele que fecha
/// o que o jogo abriu).
#[derive(Component, Debug, Clone, Copy)]
#[require(InputContext)]
pub struct VaelUiInput;

/// Prioridade do [`VaelUiInput`] sobre o [`VaelGameplayInput`] (0).
pub const UI_PRIORITY: i32 = 10;

/// O que uma entidade de acção representa no Vael.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaelAction {
    /// Uma das acções com rebind do [`ActionMap`].
    Mapped(Action),
    /// Um dos quatro sentidos do movimento (fixos: WASD e setas).
    Move(MoveDir),
    /// Teclas fixas dos painéis.
    Ui(UiAction),
    /// X / DPadUp: acende ou apaga a lanterna na cintura.
    Lantern,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiAction {
    /// M: abre/fecha o mapa.
    Map,
    /// G: viagem rápida (à beira de uma fogueira).
    Travel,
    /// F: assinar a placa mais próxima.
    Sign,
    /// Esc: fecha o painel aberto.
    Close,
}

const UI_KEYS: [(UiAction, KeyCode); 4] = [
    (UiAction::Map, KeyCode::KeyM),
    (UiAction::Travel, KeyCode::KeyG),
    (UiAction::Sign, KeyCode::KeyF),
    (UiAction::Close, KeyCode::Escape),
];

/// As acções de UI deste frame (escritas no `PreUpdate`). Quem lia
/// `ButtonInput<KeyCode>` para M/G/F/Esc lê isto: o texto focado e a
/// prioridade dos contextos já estão aplicados.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct VaelUiActions {
    pub map: bool,
    pub travel: bool,
    pub sign: bool,
    pub close: bool,
}

/// Acções de jogo próprias do Vael (fora do `ActionMap` genérico da engine).
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct VaelGameplayActions {
    /// X / DPadUp: alterna a lanterna na cintura.
    pub lantern_just: bool,
}

/// Tecla do teclado ligada a uma acção do [`ActionMap`] (a do prompt
/// `[J]`/`[E]` e do `InteractionFocus`).
pub fn action_key(map: &ActionMap, action: Action) -> Option<KeyCode> {
    map.bindings
        .get(&action)
        .and_then(|binding| binding.key.as_deref())
        .and_then(key_of)
}

/// "Atacar" premido neste frame: o [`ActionState`] quando o input do Vael
/// está montado; sem ele (apps de teste), a tecla do bind directamente.
pub fn attack_just(
    actions: Option<&ActionState>,
    map: Option<&ActionMap>,
    keys: Option<&ButtonInput<KeyCode>>,
) -> bool {
    if let Some(actions) = actions {
        return actions.attack_just;
    }
    let key = map
        .and_then(|map| action_key(map, Action::Attack))
        .unwrap_or(KeyCode::KeyJ);
    keys.is_some_and(|keys| keys.just_pressed(key))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveDir {
    Forward,
    Back,
    Left,
    Right,
}

/// Acções com rebind e o momento em que disparam: as "just" só no frame da
/// tecla, a corrida e o agachar enquanto estiverem premidos.
const MAPPED: [(Action, Condition); 7] = [
    (Action::Jump, Condition::Press),
    (Action::Sprint, Condition::Down),
    (Action::Attack, Condition::Press),
    (Action::Interact, Condition::Press),
    (Action::Lock, Condition::Press),
    (Action::Menu, Condition::Press),
    (Action::Crouch, Condition::Down),
];

const MOVES: [(MoveDir, [KeyCode; 2]); 4] = [
    (MoveDir::Forward, [KeyCode::KeyW, KeyCode::ArrowUp]),
    (MoveDir::Back, [KeyCode::KeyS, KeyCode::ArrowDown]),
    (MoveDir::Left, [KeyCode::KeyA, KeyCode::ArrowLeft]),
    (MoveDir::Right, [KeyCode::KeyD, KeyCode::ArrowRight]),
];

/// Sistemas do input do Vael. O `ActionState` está pronto depois deste set.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VaelInputSystems;

pub struct VaelInputPlugin;

impl Plugin for VaelInputPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<InputActionPlugin>() {
            app.add_plugins(InputActionPlugin);
        }
        // O `gather_actions` da engine pára: quem escreve o `ActionState`
        // é este módulo.
        app.init_resource::<viber::input_map::ExternalActionState>()
            .init_resource::<ActionMap>()
            .init_resource::<ActionState>()
            .init_resource::<VaelUiActions>()
            .init_resource::<VaelGameplayActions>()
            .add_systems(Startup, spawn_contexts)
            .add_systems(
                PreUpdate,
                (
                    // Antes da avaliação: binds novos e teclado cortado
                    // enquanto se escreve.
                    (
                        sync_mapped_bindings,
                        gate_keyboard_while_typing,
                        gate_gameplay_while_menus,
                    )
                        .before(ActionSystems)
                        .after(bevy::input::InputSystems),
                    write_action_state
                        .in_set(VaelInputSystems)
                        .after(ActionSystems)
                        .after(viber::input_map::gather_actions),
                ),
            );
    }
}

fn key_of(name: &str) -> Option<KeyCode> {
    match viber::luau::input_code_from_str(name)? {
        viber::luau::InputCode::Key(key) => Some(key),
        _ => None,
    }
}

/// Nome do botão de comando (o que o [`ActionMap`] guarda) → botão do gamepad.
///
/// A lista acompanha a do `ActionMap::default` e os nomes históricos que um
/// save/manifesto antigo possa trazer: um nome em falta aqui é um bind ÓRFÃO
/// (o `pad_of` devolvia `None` e o botão nunca disparava, em silêncio).
pub(crate) fn pad_of(name: &str) -> Option<GamepadButton> {
    Some(match name.trim().to_ascii_lowercase().as_str() {
        "south" | "a" => GamepadButton::South,
        "east" | "b" => GamepadButton::East,
        "west" | "x" => GamepadButton::West,
        "north" | "y" => GamepadButton::North,
        "start" => GamepadButton::Start,
        "select" | "back" => GamepadButton::Select,
        "mode" | "guide" => GamepadButton::Mode,
        "left_trigger" | "left_shoulder" | "lb" | "l1" => GamepadButton::LeftTrigger,
        "right_trigger" | "right_shoulder" | "rb" | "r1" => GamepadButton::RightTrigger,
        // O bind por omissão do agachar (`ActionMap`) é `dpad_down`: sem esta
        // entrada o `pad_of` devolvia `None` e o botão nunca disparava.
        "dpad_down" | "down" => GamepadButton::DPadDown,
        "dpad_up" | "up" => GamepadButton::DPadUp,
        "dpad_left" | "left" => GamepadButton::DPadLeft,
        "dpad_right" | "right" => GamepadButton::DPadRight,
        "left_trigger2" | "lt" | "l2" => GamepadButton::LeftTrigger2,
        "right_trigger2" | "rt" | "r2" => GamepadButton::RightTrigger2,
        "left_stick" | "l3" => GamepadButton::LeftThumb,
        "right_stick" | "r3" => GamepadButton::RightThumb,
        _ => return None,
    })
}

/// "shift" é qualquer um dos lados (o `jackdaw_input` compara a tecla física).
fn keys_for(name: &str) -> Vec<KeyCode> {
    let Some(key) = key_of(name) else {
        return Vec::new();
    };
    match key {
        KeyCode::ShiftLeft | KeyCode::ShiftRight => vec![KeyCode::ShiftLeft, KeyCode::ShiftRight],
        KeyCode::ControlLeft | KeyCode::ControlRight => {
            vec![KeyCode::ControlLeft, KeyCode::ControlRight]
        }
        KeyCode::AltLeft | KeyCode::AltRight => vec![KeyCode::AltLeft, KeyCode::AltRight],
        other => vec![other],
    }
}

fn spawn_contexts(mut commands: Commands) {
    let context = commands
        .spawn((Name::new("Input: jogo"), VaelGameplayInput))
        .id();
    let ui = commands
        .spawn((
            Name::new("Input: UI"),
            VaelUiInput,
            ContextPriority(UI_PRIORITY),
        ))
        .id();
    for (kind, key) in UI_KEYS {
        let action = commands
            .spawn((
                Name::new(format!("Input: UI {kind:?}")),
                InputAction,
                ActionOf(ui),
                VaelAction::Ui(kind),
            ))
            .id();
        commands.spawn((Binding::from(key), Condition::Press, BindingOf(action)));
    }
    for (dir, keys) in MOVES {
        let action = commands
            .spawn((
                Name::new(format!("Input: mover {dir:?}")),
                InputAction,
                ActionOf(context),
                VaelAction::Move(dir),
            ))
            .id();
        for key in keys {
            commands.spawn((Binding::from(key), Condition::Down, BindingOf(action)));
        }
    }
    let lantern_action = commands
        .spawn((
            Name::new("Input: lanterna"),
            InputAction,
            ActionOf(context),
            VaelAction::Lantern,
        ))
        .id();
    commands.spawn((
        Binding::from(KeyCode::KeyX),
        Condition::Press,
        BindingOf(lantern_action),
    ));
    for (mapped, _) in MAPPED {
        // Os bindings destas acções nascem no `sync_mapped_bindings` (o
        // `ActionMap` acabado de criar conta como mudado).
        // O menu vive na UI: com o jogo inactivo (menu aberto) tem de
        // continuar a fechá-lo.
        let owner = if mapped == Action::Menu { ui } else { context };
        commands.spawn((
            Name::new(format!("Input: {}", mapped.as_str())),
            InputAction,
            ActionOf(owner),
            ActionSettings {
                // Uma tecla já premida quando o bind muda não dispara sozinha.
                require_reset: true,
                consume_input: false,
            },
            VaelAction::Mapped(mapped),
        ));
    }
}

/// Refaz os `Binding` das acções com rebind sempre que o [`ActionMap`] muda.
fn sync_mapped_bindings(
    map: Res<ActionMap>,
    mut commands: Commands,
    actions: Query<(Entity, &VaelAction, Option<&jackdaw_input::Bindings>)>,
    mut done_once: Local<bool>,
) {
    // O contexto nasce no `Startup` com comandos; no primeiro `PreUpdate` as
    // acções já existem, mesmo que o mapa não tenha mudado desde então.
    if *done_once && !map.is_changed() {
        return;
    }
    let mut any = false;
    for (entity, kind, bindings) in &actions {
        let VaelAction::Mapped(action) = *kind else {
            continue;
        };
        any = true;
        if let Some(bindings) = bindings {
            for binding in bindings {
                commands.entity(binding).despawn();
            }
        }
        let condition = MAPPED
            .iter()
            .find(|(a, _)| *a == action)
            .map_or(Condition::Press, |(_, c)| *c);
        let key = map.bindings.get(&action).and_then(|b| b.key.as_deref());
        for key in key.map(keys_for).unwrap_or_default() {
            commands.spawn((Binding::from(key), condition, BindingOf(entity)));
        }
    }
    *done_once |= any;
}

/// Com uma `<UiInput>` focada (barra de fala) as teclas são texto: o
/// teclado sai de todos os contextos, modificadores incluídos.
fn gate_keyboard_while_typing(
    typing: Option<Res<viber::ui::widgets::UiFocusedInput>>,
    mut sources: ResMut<InputSources>,
) {
    let keyboard = !typing.is_some_and(|focus| focus.0.is_some());
    if sources.keyboard != keyboard {
        sources.keyboard = keyboard;
    }
}

/// Menu, loja, mapa, viagem ou cinemática abertos: o contexto de jogo
/// pára (as suas acções leem "não premidas") e o de UI continua.
fn gate_gameplay_while_menus(
    menus: Option<Res<MenusOpen>>,
    mut contexts: Query<&mut ContextActivity, With<VaelGameplayInput>>,
) {
    let active = !menus.is_some_and(|menus| menus.any());
    for mut activity in &mut contexts {
        if activity.0 != active {
            *activity = ContextActivity(active);
        }
    }
}

/// Junta o teclado (acções avaliadas) e o gamepad no [`ActionState`].
fn write_action_state(
    map: Res<ActionMap>,
    pads: Query<&Gamepad>,
    sources: Res<InputSources>,
    gameplay: Query<&ContextActivity, With<VaelGameplayInput>>,
    actions: Query<(&VaelAction, &TriggerState)>,
    mut state: ResMut<ActionState>,
    mut ui: ResMut<VaelUiActions>,
    mut extra: ResMut<VaelGameplayActions>,
) {
    let fired = |want: VaelAction| {
        actions
            .iter()
            .any(|(kind, trigger)| *kind == want && *trigger == TriggerState::Fired)
    };
    let axis = |pos: MoveDir, neg: MoveDir| {
        fired(VaelAction::Move(pos)) as i32 as f32 - fired(VaelAction::Move(neg)) as i32 as f32
    };
    let mut move_x = axis(MoveDir::Right, MoveDir::Left);
    let mut forward = axis(MoveDir::Forward, MoveDir::Back);

    // A escrever, o comando também fica calado (como no `gather_actions`).
    // Com o jogo inactivo só o botão do menu passa.
    let typing = !sources.keyboard;
    let playing = gameplay.iter().all(|activity| activity.0);
    let pads: Vec<&Gamepad> = if typing {
        Vec::new()
    } else {
        pads.iter().collect()
    };
    let sticks: &[&Gamepad] = if playing { &pads } else { &[] };
    for pad in sticks {
        let x = pad.get(GamepadAxis::LeftStickX).unwrap_or(0.0);
        let y = pad.get(GamepadAxis::LeftStickY).unwrap_or(0.0);
        if x.abs() > 0.2 {
            move_x = x;
        }
        if y.abs() > 0.2 {
            forward = y;
        }
    }
    let pad = |action: Action, held: bool| {
        map.bindings
            .get(&action)
            .and_then(|b| b.pad.as_deref())
            .and_then(pad_of)
            .is_some_and(|button| {
                pads.iter().any(|pad| {
                    if held {
                        pad.pressed(button)
                    } else {
                        pad.just_pressed(button)
                    }
                })
            })
    };
    let get = |action: Action, held: bool| {
        fired(VaelAction::Mapped(action))
            || ((playing || action == Action::Menu) && pad(action, held))
    };
    *ui = VaelUiActions {
        map: fired(VaelAction::Ui(UiAction::Map)),
        travel: fired(VaelAction::Ui(UiAction::Travel)),
        sign: fired(VaelAction::Ui(UiAction::Sign)),
        close: fired(VaelAction::Ui(UiAction::Close)),
    };
    *extra = VaelGameplayActions {
        lantern_just: fired(VaelAction::Lantern)
            || (playing
                && pads
                    .iter()
                    .any(|pad| pad.just_pressed(GamepadButton::DPadUp))),
    };
    *state = ActionState {
        move_x,
        forward,
        jump_just: get(Action::Jump, false),
        sprint: get(Action::Sprint, true),
        attack_just: get(Action::Attack, false),
        interact_just: get(Action::Interact, false),
        lock_just: get(Action::Lock, false),
        menu_just: get(Action::Menu, false),
        crouch_held: get(Action::Crouch, true),
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::input::mouse::MouseWheel;

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<MouseWheel>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .add_plugins(VaelInputPlugin);
        // O `gather_actions` da engine não está nesta app: a ordem
        // `.after(gather_actions)` fica sem efeito, como deve.
        app.update();
        app
    }

    fn press(app: &mut App, key: KeyCode) {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(key);
    }

    fn release_all(app: &mut App) {
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.release_all();
        keys.clear();
    }

    #[test]
    fn wasd_e_setas_dao_o_eixo() {
        let mut app = app();
        press(&mut app, KeyCode::KeyW);
        press(&mut app, KeyCode::ArrowLeft);
        app.update();
        let state = app.world().resource::<ActionState>();
        assert_eq!((state.move_x, state.forward), (-1.0, 1.0));
    }

    #[test]
    fn salto_e_so_no_frame_da_tecla() {
        let mut app = app();
        press(&mut app, KeyCode::Space);
        app.update();
        assert!(app.world().resource::<ActionState>().jump_just);
        // Ainda premida, já não é "just".
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear();
        app.update();
        assert!(!app.world().resource::<ActionState>().jump_just);
    }

    #[test]
    fn corrida_aceita_os_dois_shifts() {
        let mut app = app();
        press(&mut app, KeyCode::ShiftRight);
        app.update();
        assert!(app.world().resource::<ActionState>().sprint);
    }

    #[test]
    fn rebind_no_action_map_refaz_o_binding() {
        let mut app = app();
        app.world_mut()
            .resource_mut::<ActionMap>()
            .rebind_key(Action::Jump, "f");
        app.update();
        release_all(&mut app);
        app.update();
        press(&mut app, KeyCode::Space);
        app.update();
        assert!(!app.world().resource::<ActionState>().jump_just);
        release_all(&mut app);
        app.update();
        press(&mut app, KeyCode::KeyF);
        app.update();
        assert!(app.world().resource::<ActionState>().jump_just);
    }

    #[test]
    fn menu_aberto_para_o_jogo_mas_nao_a_ui() {
        let mut app = app();
        app.insert_resource(MenusOpen {
            map: true,
            ..Default::default()
        });
        app.update();
        press(&mut app, KeyCode::KeyW);
        press(&mut app, KeyCode::KeyJ);
        press(&mut app, KeyCode::KeyM);
        press(&mut app, KeyCode::KeyQ);
        app.update();
        let state = app.world().resource::<ActionState>().clone();
        assert_eq!(state.forward, 0.0);
        assert!(!state.attack_just);
        assert!(state.menu_just, "o menu fecha-se com o jogo parado");
        assert!(app.world().resource::<VaelUiActions>().map);
    }

    #[test]
    fn mapa_fecha_e_o_jogo_volta() {
        let mut app = app();
        app.insert_resource(MenusOpen {
            map: true,
            ..Default::default()
        });
        app.update();
        app.world_mut().resource_mut::<MenusOpen>().map = false;
        app.update();
        press(&mut app, KeyCode::KeyW);
        app.update();
        assert_eq!(app.world().resource::<ActionState>().forward, 1.0);
    }

    #[test]
    fn atacar_sem_input_montado_le_a_tecla_do_bind() {
        let mut keys = ButtonInput::<KeyCode>::default();
        let mut map = ActionMap::default();
        map.rebind_key(Action::Attack, "k");
        keys.press(KeyCode::KeyK);
        assert!(attack_just(None, Some(&map), Some(&keys)));
        assert!(!attack_just(None, None, Some(&keys)));
    }

    #[test]
    fn a_escrever_nada_dispara() {
        let mut app = app();
        app.insert_resource(viber::ui::widgets::UiFocusedInput(Some(
            Entity::PLACEHOLDER,
        )));
        press(&mut app, KeyCode::KeyW);
        press(&mut app, KeyCode::KeyJ);
        press(&mut app, KeyCode::KeyM);
        app.update();
        let state = app.world().resource::<ActionState>();
        assert_eq!(state.forward, 0.0);
        assert!(!state.attack_just);
        assert!(!app.world().resource::<VaelUiActions>().map);
    }

    /// Nenhum bind do `ActionMap` pode ser ÓRFÃO no gamepad: um nome sem
    /// entrada no `pad_of` anulava o botão em silêncio (o agachar era o caso
    /// deste teste quando `dpad_down` faltava).
    #[test]
    fn todos_os_binds_de_gamepad_do_action_map_mapeiam() {
        let map = ActionMap::default();
        for (action, binding) in &map.bindings {
            let Some(pad) = binding.pad.as_deref() else {
                continue;
            };
            assert!(
                pad_of(pad).is_some(),
                "{action:?}: o bind de gamepad '{pad}' não tem botão"
            );
        }
        // Os dois lados do gatilho continuam distintos (sprint no shoulder).
        assert_eq!(pad_of("left_trigger"), Some(GamepadButton::LeftTrigger));
        assert_eq!(pad_of("left_trigger2"), Some(GamepadButton::LeftTrigger2));
        assert_eq!(pad_of(" RIGHT_TRIGGER "), Some(GamepadButton::RightTrigger));
    }

    #[test]
    fn tecla_x_dispara_lanterna_uma_vez_e_para_com_menu() {
        let mut app = app();
        press(&mut app, KeyCode::KeyX);
        app.update();
        assert!(app.world().resource::<VaelGameplayActions>().lantern_just);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear();
        app.update();
        assert!(!app.world().resource::<VaelGameplayActions>().lantern_just);

        // Com menu aberto o contexto de jogo desliga.
        release_all(&mut app);
        app.insert_resource(MenusOpen {
            map: true,
            ..Default::default()
        });
        app.update();
        press(&mut app, KeyCode::KeyX);
        app.update();
        assert!(!app.world().resource::<VaelGameplayActions>().lantern_just);
    }
}

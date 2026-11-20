//! Lanterna de cintura do herói: acende e apaga por botão (`X` / `DPadUp`),
//! substitui a luz automática noturna da engine e alimenta a furtividade da IA.
//!
//! # Comportamento
//!
//! * A luz noturna automática (`lantern_driver` em `bevy_ext_engine`) é
//!   desligada via [`ExternalLantern`] — à noite o herói fica genuinamente no
//!   escuro se a lanterna estiver apagada.
//! * O modelo 3D (`lantern_hanging_lod0.glb`) fica pendurado no osso
//!   `spine_01` (anca esquerda do cinto, livre da espada em `hand_r`), com
//!   balanço suave de pêndulo ao correr/virar e brilho emissivo próprio.
//! * Quando o herói equipa a tocha de mão (`V`, `TorchLightOverride > 0`), a
//!   lanterna da cintura esconde-se (passou para a mão) e a luz acompanha a
//!   mão; ao voltar à espada, regressa à cintura mantendo o estado do botão.
//! * `viber.vael.lantern_{on,lit,intensity,set}` expõe o estado aos scripts
//!   Lua: `lib/fsm.lua` reduz o alcance de visão dos inimigos à noite com a
//!   lanterna apagada e atrai-os quando acesa.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::transform::TransformSystems;
use bevy_ext_core::{MenusOpen, Player, SfxClip, SfxEvent};
use mlua::Lua;
use serde_json::Value;
use vael_engine::ambient::{
    ExternalLantern, LANTERN_MAX_INTENSITY, LANTERN_RANGE, TorchLightOverride, TravellerLantern,
};
use vael_engine::attachment::find_bone;
use vael_engine::luau::components::ScriptToast;
use vael_engine::recipes::spawn::GltfScenePending;
use vael_engine::ui::UiModalsOpen;

use crate::content::{VaelContent, label};
use crate::hud::save_lang;
use crate::input::VaelGameplayActions;
use crate::save::VaelSave;
use crate::vault::{vael_fn, vael_table};

/// Chave persistida no save (`viber.game().lantern_on`).
pub const LANTERN_SAVE_KEY: &str = "lantern_on";
/// Modelo 3D da lanterna (o mesmo do pool da tocha, escalado para o cinto).
pub const WAIST_LANTERN_GLB: &str = "assets/meshes/interiors/lantern_hanging_lod0.glb";
/// Osso da cintura no `hero_lod0.glb` (altura ~0,90 m, +X esquerda, +Y cima, +Z frente).
pub const WAIST_LANTERN_BONE: &str = "spine_01";
/// Escala do modelo da lanterna no cinto (~22,4 cm de altura × 9 cm de largura).
const WAIST_LANTERN_SCALE: f32 = 0.28;
/// Altura do aro de topo no GLB em escala 1 (`0.80 m` no precompute).
const LANTERN_MESH_TOP_Y: f32 = 0.80;
/// Posição do pivô (aro no cinto) no espaço local de `spine_01` (anca esquerda).
const WAIST_PIVOT_OFFSET: Vec3 = Vec3::new(0.20, -0.02, -0.03);
/// Offset do corpo da lanterna abaixo do pivô para o aro encostar ao cinto.
const WAIST_MESH_OFFSET: Vec3 = Vec3::new(0.0, -LANTERN_MESH_TOP_Y * WAIST_LANTERN_SCALE, 0.0);
/// Offset da chama interior (centro da gaiola de vidro) no espaço do pivô.
const WAIST_CORE_OFFSET: Vec3 = Vec3::new(0.0, -0.115, 0.0);
/// Alcance da luz da lanterna de cintura (m).
const WAIST_LIGHT_RANGE: f32 = LANTERN_RANGE + 1.0;
/// Constante de tempo (s) do fade suave ao acender/apagar.
const FADE_TAU: f32 = 0.16;
/// Limiar abaixo do qual a luz pontual é escondida (fração do máximo).
const HIDE_THRESHOLD: f32 = 0.02;
/// Cor quente da chama da lanterna (#ffb066).
const LANTERN_COLOR: Color = Color::srgb(1.0, 0.69, 0.40);
/// Emissivo da gaiola do modelo quando aceso (HDR suave para bloom).
const MESH_EMISSIVE_LIT: LinearRgba = LinearRgba::new(4.2, 1.85, 0.45, 1.0);
/// Emissivo da chama interior quando acesa.
const CORE_EMISSIVE_LIT: LinearRgba = LinearRgba::new(15.0, 7.2, 1.9, 1.0);
/// Inclinação lateral de repouso (rad) para afastar ligeiramente da coxa.
const REST_OUTWARD_ROLL: f32 = -0.07;
/// Limite de balanço do pêndulo (rad, ~22°).
const MAX_SWAY_RAD: f32 = 0.38;

/// Fotografia partilhada do estado da lanterna (Rust ↔ Lua ↔ HUD).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LanternSnapshot {
    /// Interruptor da lanterna de cintura (controlado pelo jogador / save).
    pub waist_on: bool,
    /// Tocha de mão equipada (`V`, `TorchLightOverride.intensity > 0`).
    pub torch_held: bool,
    /// `true` quando o herói emite luz (cintura acesa OU tocha de mão).
    pub lit: bool,
    /// Intensidade normalizada atual (0..1) após o fade suave.
    pub intensity: f32,
    /// Pedido pendente vindo do Lua (`viber.vael.lantern_set`).
    pending_set: Option<bool>,
}

/// Estado partilhado da lanterna do herói.
#[derive(Resource, Debug, Clone, Default)]
pub struct VaelLanternState(pub Arc<Mutex<LanternSnapshot>>);

impl VaelLanternState {
    fn lock(&self) -> std::sync::MutexGuard<'_, LanternSnapshot> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Snapshot atual.
    pub fn get(&self) -> LanternSnapshot {
        *self.lock()
    }

    /// Define o interruptor da lanterna de cintura diretamente.
    pub fn set_waist(&self, on: bool) {
        let mut s = self.lock();
        s.waist_on = on;
        s.lit = s.waist_on || s.torch_held;
        s.pending_set = None;
    }

    /// Enfileira uma alteração vinda do Lua (aplicada e persistida no `Update`).
    pub fn request_set(&self, on: bool) {
        let mut s = self.lock();
        s.pending_set = Some(on);
        s.waist_on = on;
        s.lit = s.waist_on || s.torch_held;
    }
}

/// Pivô da lanterna preso ao osso `spine_01` do herói (balança como pêndulo).
#[derive(Component, Debug)]
pub struct WaistLanternPivot {
    pub owner: Entity,
    last_pos: Option<Vec3>,
    last_vel: Vec2,
    angle: Vec2,
    ang_vel: Vec2,
}

/// Filho que recebe o `GltfScenePending` da lanterna de cintura.
#[derive(Component)]
struct WaistLanternMesh;

/// Chama emissiva interior dentro da lanterna.
#[derive(Component)]
struct WaistLanternCore {
    material: Handle<StandardMaterial>,
}

/// Material clonado de uma malha filha do GLB (evita mexer no asset partilhado
/// das lanternas do mercado).
#[derive(Component)]
struct WaistLanternClonedMaterial(Handle<StandardMaterial>);

/// Rastreio local do último valor observado em [`VaelSave`] (evita que um
/// snapshot antigo de 10 Hz do `state_sync` reverta um toggle acabado de fazer).
#[derive(Resource, Default)]
struct LanternSaveSync {
    last_save_raw: Option<Option<bool>>,
    suppress_until: f32,
}

/// Plugin da lanterna de cintura do Vael.
pub struct VaelLanternPlugin;

impl Plugin for VaelLanternPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ExternalLantern>()
            .init_resource::<VaelLanternState>()
            .init_resource::<LanternSaveSync>()
            .add_systems(
                Startup,
                install_lantern_api.after(crate::combat::install_combat_api),
            )
            .add_systems(
                Update,
                (
                    sync_lantern_from_save,
                    toggle_waist_lantern,
                    mount_waist_lantern,
                    prepare_waist_lantern_materials,
                )
                    .chain(),
            )
            .add_systems(
                PostUpdate,
                drive_waist_lantern.after(TransformSystems::Propagate),
            );
    }
}

/// Arranque: regista `viber.vael.lantern_{on,lit,intensity,set}` nos contextos BMS e REPL.
pub(crate) fn install_lantern_api(state: Res<VaelLanternState>) {
    let state = state.clone();
    crate::script_host::register_installer(move |lua| install(lua, &state));
    info!("vael: lanterna de cintura registada nos contextos BMS (`viber.vael`)");
}

/// Instala as funções da lanterna em `viber.vael`.
pub(crate) fn install(lua: &Lua, state: &VaelLanternState) -> mlua::Result<()> {
    let vael = vael_table(lua)?;

    vael_fn!(lua, vael, "lantern_on", [state = state.clone()], move |_, ()| {
        Ok(state.get().waist_on)
    });

    vael_fn!(lua, vael, "lantern_lit", [state = state.clone()], move |_, ()| {
        Ok(state.get().lit)
    });

    vael_fn!(
        lua,
        vael,
        "lantern_intensity",
        [state = state.clone()],
        move |_, ()| Ok(state.get().intensity as f64)
    );

    vael_fn!(
        lua,
        vael,
        "lantern_set",
        [state = state.clone()],
        move |_, on: bool| {
            state.request_set(on);
            Ok(())
        }
    );

    Ok(())
}

/// Flicker calmo e orgânico de lanterna (em torno de 1.0, ±5%).
pub fn lantern_flicker(elapsed: f32) -> f32 {
    1.0 + 0.032 * (elapsed * 7.3).sin()
        + 0.020 * (elapsed * 16.7 + 1.4).sin()
        + 0.010 * (elapsed * 29.1 + 0.6).sin()
}

/// Sincroniza o interruptor da lanterna com o save (load/novo jogo) e drena
/// pedidos vindos de `viber.vael.lantern_set`.
fn sync_lantern_from_save(
    time: Res<Time>,
    state: Res<VaelLanternState>,
    save: Option<Res<VaelSave>>,
    mut sync: ResMut<LanternSaveSync>,
) {
    let now = time.elapsed_secs();
    let pending = {
        let mut s = state.lock();
        s.pending_set.take()
    };
    if let Some(on) = pending {
        sync.suppress_until = now + 0.35;
        persist_lantern(on);
        return;
    }
    let Some(save) = save else {
        return;
    };
    let raw = save.legacy.get(LANTERN_SAVE_KEY).and_then(Value::as_bool);
    if sync.last_save_raw == Some(raw) {
        return;
    }
    sync.last_save_raw = Some(raw);
    if now < sync.suppress_until {
        return;
    }
    state.set_waist(raw.unwrap_or(false));
}

fn persist_lantern(on: bool) {
    let mut patch = BTreeMap::new();
    patch.insert(LANTERN_SAVE_KEY.to_string(), Value::Bool(on));
    vael_engine::luau::game::apply_patch_json(&patch);
}

/// Botão da lanterna (`X` / `DPadUp`): alterna a lanterna de cintura.
fn toggle_waist_lantern(
    time: Res<Time>,
    state: Res<VaelLanternState>,
    mut sync: ResMut<LanternSaveSync>,
    actions: Option<Res<VaelGameplayActions>>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    menus: Option<Res<MenusOpen>>,
    modals: Option<Res<UiModalsOpen>>,
    save: Option<Res<VaelSave>>,
    content: Option<Res<VaelContent>>,
    players: Query<&GlobalTransform, With<Player>>,
    mut sfx: Option<MessageWriter<SfxEvent>>,
    mut toast: Option<MessageWriter<ScriptToast>>,
) {
    let pressed = match actions.as_deref() {
        Some(actions) => actions.lantern_just,
        None => keys
            .as_deref()
            .is_some_and(|keys| keys.just_pressed(KeyCode::KeyX)),
    };
    if !pressed {
        return;
    }
    if menus.as_deref().is_some_and(MenusOpen::any) {
        return;
    }
    if modals
        .as_deref()
        .is_some_and(|m| m.is_open("title") || m.is_open("death"))
    {
        return;
    }
    if let Some(save) = save.as_deref()
        && save.vitals.max_hp > 0.0
        && save.vitals.hp <= 0.0
    {
        return;
    }
    let next = !state.get().waist_on;
    state.set_waist(next);
    sync.suppress_until = time.elapsed_secs() + 0.35;
    persist_lantern(next);

    let pos = players.iter().next().map(GlobalTransform::translation);
    if let Some(sfx) = sfx.as_mut() {
        sfx.write(SfxEvent {
            clip: if next {
                SfxClip::FireCrackle
            } else {
                SfxClip::ItemDrop
            },
            position: pos,
        });
    }
    if let Some(toast) = toast.as_mut() {
        let lang = save.as_deref().map_or("pt", save_lang);
        let msg = if next {
            label(content.as_deref(), lang, "toast.lantern.on", "Lanterna acesa")
        } else {
            label(
                content.as_deref(),
                lang,
                "toast.lantern.off",
                "Lanterna apagada",
            )
        };
        toast.write(ScriptToast(msg));
    }
}

/// Monta o modelo da lanterna de cintura no osso `spine_01` do herói assim que
/// o esqueleto estiver disponível (e remonta se o rig mudar de cena).
fn mount_waist_lantern(
    mut commands: Commands,
    asset_server: Option<Res<AssetServer>>,
    mut meshes: Option<ResMut<Assets<Mesh>>>,
    mut materials: Option<ResMut<Assets<StandardMaterial>>>,
    players: Query<Entity, With<Player>>,
    pivots: Query<(Entity, &WaistLanternPivot)>,
    children: Query<&Children>,
    names: Query<&Name>,
    entities: Query<Entity>,
) {
    // Limpa pivôs órfãos cujo dono já não existe.
    for (pivot_entity, pivot) in &pivots {
        if !entities.contains(pivot.owner) {
            commands.entity(pivot_entity).try_despawn();
        }
    }
    let Some(hero) = players.iter().next() else {
        return;
    };
    if pivots.iter().any(|(_, p)| p.owner == hero) {
        return;
    }
    let Some(bone) = find_bone(hero, WAIST_LANTERN_BONE, &children, &names) else {
        return;
    };

    let pivot = commands
        .spawn((
            Name::new("vael:lanterna-cintura"),
            WaistLanternPivot {
                owner: hero,
                last_pos: None,
                last_vel: Vec2::ZERO,
                angle: Vec2::ZERO,
                ang_vel: Vec2::ZERO,
            },
            Transform::from_translation(WAIST_PIVOT_OFFSET)
                .with_rotation(Quat::from_rotation_z(REST_OUTWARD_ROLL)),
            Visibility::Inherited,
        ))
        .id();
    commands.entity(bone).add_child(pivot);

    if let Some(asset_server) = asset_server.as_deref() {
        let handle = vael_engine::meshopt::load_gltf(asset_server, WAIST_LANTERN_GLB.to_string());
        let mesh_root = commands
            .spawn((
                Name::new("vael:lanterna-cintura-malha"),
                WaistLanternMesh,
                Transform {
                    translation: WAIST_MESH_OFFSET,
                    rotation: Quat::IDENTITY,
                    scale: Vec3::splat(WAIST_LANTERN_SCALE),
                },
                Visibility::Inherited,
                GltfScenePending::single(handle),
            ))
            .id();
        commands.entity(pivot).add_child(mesh_root);
    }

    if let (Some(meshes), Some(materials)) = (meshes.as_deref_mut(), materials.as_deref_mut()) {
        let sphere = meshes.add(Sphere::new(0.024));
        let mat = materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.85, 0.45),
            emissive: LinearRgba::BLACK,
            unlit: true,
            ..default()
        });
        let core = commands
            .spawn((
                Name::new("vael:lanterna-cintura-chama"),
                WaistLanternCore {
                    material: mat.clone(),
                },
                Mesh3d(sphere),
                MeshMaterial3d(mat),
                NotShadowCaster,
                Transform::from_translation(WAIST_CORE_OFFSET),
                Visibility::Hidden,
            ))
            .id();
        commands.entity(pivot).add_child(core);
    }
}

/// Clona os `StandardMaterial` das malhas instanciadas pelo GLB da lanterna de
/// cintura e marca-as com `NotShadowCaster` para que a gaiola não bloqueie a
/// própria luz pontual nem afete as lanternas partilhadas do mercado.
fn prepare_waist_lantern_materials(
    mut commands: Commands,
    mut materials: Option<ResMut<Assets<StandardMaterial>>>,
    roots: Query<Entity, With<WaistLanternMesh>>,
    children: Query<&Children>,
    mesh_mats: Query<
        &MeshMaterial3d<StandardMaterial>,
        Without<WaistLanternClonedMaterial>,
    >,
) {
    let Some(materials) = materials.as_deref_mut() else {
        return;
    };
    for root in &roots {
        let mut stack = vec![root];
        while let Some(entity) = stack.pop() {
            if let Ok(mat_comp) = mesh_mats.get(entity)
                && let Some(existing) = materials.get(&mat_comp.0).cloned()
            {
                let mut cloned = existing;
                cloned.emissive = LinearRgba::BLACK;
                let handle = materials.add(cloned);
                commands.entity(entity).insert((
                    MeshMaterial3d(handle.clone()),
                    WaistLanternClonedMaterial(handle),
                    NotShadowCaster,
                ));
            }
            if let Ok(kids) = children.get(entity) {
                for child in kids.iter() {
                    stack.push(child);
                }
            }
        }
    }
}

/// Atualiza pêndulo, visibilidade, emissivo e a `PointLight` [`TravellerLantern`].
fn drive_waist_lantern(
    time: Res<Time>,
    state: Res<VaelLanternState>,
    torch: Option<Res<TorchLightOverride>>,
    mut materials: Option<ResMut<Assets<StandardMaterial>>>,
    players: Query<&GlobalTransform, With<Player>>,
    mut pivots: Query<
        (&mut WaistLanternPivot, &mut Transform, &mut Visibility, &GlobalTransform),
        (Without<TravellerLantern>, Without<WaistLanternCore>),
    >,
    mut cores: Query<
        (&WaistLanternCore, &mut Visibility, &GlobalTransform),
        (Without<TravellerLantern>, Without<WaistLanternPivot>),
    >,
    cloned_mats: Query<&WaistLanternClonedMaterial>,
    mut lanterns: Query<
        (&mut Transform, &mut PointLight, &mut Visibility),
        (With<TravellerLantern>, Without<WaistLanternPivot>, Without<WaistLanternCore>),
    >,
    mut commands: Commands,
) {
    let torch_intensity = torch.map_or(0.0, |t| t.intensity.max(0.0));
    let torch_held = torch_intensity > 1.0;

    let waist_on = {
        let mut s = state.lock();
        s.torch_held = torch_held;
        s.lit = s.waist_on || torch_held;
        s.waist_on
    };

    let Some(hero_gt) = players.iter().next() else {
        return;
    };
    let hero_pos = hero_gt.translation();
    let dt = time.delta_secs().clamp(0.0, 0.1);
    let elapsed = time.elapsed_secs();
    let flicker = lantern_flicker(elapsed);

    // 1. Pêndulo + visibilidade do modelo na cintura (esconde quando a tocha
    // está na mão, pois o herói tirou a lanterna do cinto).
    let mut lantern_world_pos: Option<Vec3> = None;
    for (mut pivot, mut transform, mut visibility, pivot_gt) in &mut pivots {
        let wanted_vis = if torch_held {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        if *visibility != wanted_vis {
            *visibility = wanted_vis;
        }
        if dt > 1e-4 {
            if let Some(prev) = pivot.last_pos {
                let vel = Vec2::new(hero_pos.x - prev.x, hero_pos.z - prev.z) / dt;
                let accel = (vel - pivot.last_vel) / dt;
                pivot.last_vel = vel;
                let inv_rot = hero_gt.to_scale_rotation_translation().1.inverse();
                let local_acc = inv_rot * Vec3::new(accel.x, 0.0, accel.y);
                let local_vel = inv_rot * Vec3::new(vel.x, 0.0, vel.y);
                // Aceleração e arrasto do ar balançam a lanterna suavemente.
                let target_x = (-local_acc.z * 0.022 - local_vel.z * 0.026).clamp(-MAX_SWAY_RAD, MAX_SWAY_RAD);
                let target_z = (local_acc.x * 0.022 + local_vel.x * 0.022).clamp(-MAX_SWAY_RAD, MAX_SWAY_RAD);
                let target = Vec2::new(target_x, target_z);
                let spring_k = 28.0;
                let damping = 6.8;
                let a = (target - pivot.angle) * spring_k - pivot.ang_vel * damping;
                pivot.ang_vel += a * dt;
                pivot.angle = (pivot.angle + pivot.ang_vel * dt)
                    .clamp(Vec2::splat(-MAX_SWAY_RAD), Vec2::splat(MAX_SWAY_RAD));
                transform.rotation = Quat::from_euler(
                    EulerRot::XYZ,
                    pivot.angle.x,
                    0.0,
                    REST_OUTWARD_ROLL + pivot.angle.y,
                );
            }
            pivot.last_pos = Some(hero_pos);
        }
        lantern_world_pos = Some(pivot_gt.translation() + Vec3::Y * -0.10);
    }

    // Posição real da luz pontual: nasce junto da lanterna (com ligeiro
    // afastamento da anca para iluminar ambos os lados sem auto-sombra dura).
    let light_anchor = if torch_held {
        let (_, rot, _) = hero_gt.to_scale_rotation_translation();
        let fwd = rot * Vec3::Z;
        let right = rot * -Vec3::X;
        hero_pos + fwd * 0.45 + right * 0.25 + Vec3::Y * 1.45
    } else if let Some(core_pos) = cores.iter().next().map(|(_, _, gt)| gt.translation()) {
        let (_, rot, _) = hero_gt.to_scale_rotation_translation();
        let left = rot * Vec3::X;
        core_pos + left * 0.14 + Vec3::Y * 0.30
    } else if let Some(pos) = lantern_world_pos {
        pos + Vec3::Y * 0.30
    } else {
        hero_pos + Vec3::Y * 1.15
    };

    // 2. Intensidade alvo: ZERO quando apagada (mesmo à noite!).
    let target_intensity = if torch_held {
        torch_intensity.max(LANTERN_MAX_INTENSITY) * flicker
    } else if waist_on {
        LANTERN_MAX_INTENSITY * flicker
    } else {
        0.0
    };

    let Ok((mut light_tf, mut light, mut light_vis)) = lanterns.single_mut() else {
        commands.spawn((
            TravellerLantern,
            PointLight {
                color: LANTERN_COLOR,
                intensity: 0.0,
                range: WAIST_LIGHT_RANGE,
                radius: 0.24,
                ..default()
            },
            Transform::from_translation(light_anchor),
            Visibility::Hidden,
            Name::new("fx:lanterna-viajante"),
        ));
        return;
    };

    light_tf.translation = light_anchor;
    let blend = if dt > 0.0 {
        1.0 - (-dt / FADE_TAU).exp()
    } else {
        1.0
    };
    light.intensity += (target_intensity - light.intensity) * blend;
    if target_intensity == 0.0 && light.intensity < LANTERN_MAX_INTENSITY * 0.004 {
        light.intensity = 0.0;
    }
    let norm = (light.intensity / LANTERN_MAX_INTENSITY).clamp(0.0, 1.5);
    let visible = light.intensity >= LANTERN_MAX_INTENSITY * HIDE_THRESHOLD;
    let wanted_light_vis = if visible {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    if *light_vis != wanted_light_vis {
        *light_vis = wanted_light_vis;
    }

    // Publica a intensidade normalizada (usada pelos testes e regras Lua).
    {
        let mut s = state.lock();
        s.intensity = norm.clamp(0.0, 1.0);
    }

    // 3. Atualiza o brilho emissivo da lanterna da cintura (só quando está na
    // cintura e acesa).
    let waist_glow = if !torch_held && waist_on {
        norm.clamp(0.0, 1.2)
    } else {
        0.0
    };
    let core_vis = if waist_glow > 0.02 {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for (core, mut vis, _) in &mut cores {
        if *vis != core_vis {
            *vis = core_vis;
        }
        if let Some(materials) = materials.as_deref_mut()
            && let Some(mut mat) = materials.get_mut(&core.material)
        {
            mat.emissive = if waist_glow > 0.01 {
                CORE_EMISSIVE_LIT * waist_glow
            } else {
                LinearRgba::BLACK
            };
        }
    }
    if let Some(materials) = materials.as_deref_mut() {
        for cloned in &cloned_mats {
            if let Some(mut mat) = materials.get_mut(&cloned.0) {
                mat.emissive = if waist_glow > 0.01 {
                    MESH_EMISSIVE_LIT * waist_glow
                } else {
                    LinearRgba::BLACK
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn test_app() -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()));
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            Duration::from_secs_f32(0.1),
        ));
        app.init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .init_asset::<bevy::gltf::Gltf>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<VaelGameplayActions>()
            .init_resource::<TorchLightOverride>()
            .init_resource::<VaelSave>()
            .add_message::<SfxEvent>()
            .add_message::<ScriptToast>()
            .add_plugins(VaelLanternPlugin);

        let hero = app
            .world_mut()
            .spawn((
                Player::default(),
                Transform::from_xyz(1.0, 2.0, 3.0),
                GlobalTransform::from_xyz(1.0, 2.0, 3.0),
            ))
            .id();
        let spine = app
            .world_mut()
            .spawn((
                Name::new(WAIST_LANTERN_BONE),
                Transform::from_xyz(0.0, 0.9, 0.0),
                GlobalTransform::from_xyz(1.0, 2.9, 3.0),
            ))
            .id();
        app.world_mut().entity_mut(hero).add_child(spine);
        (app, hero)
    }

    fn step(app: &mut App, secs: f32) {
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            Duration::from_secs_f32(secs),
        ));
        app.update();
    }

    #[test]
    fn desliga_lanterna_automatica_da_engine_e_arranca_apagada() {
        let (mut app, _) = test_app();
        assert!(app.world().contains_resource::<ExternalLantern>());
        // Mesmo com noite a 100%, a lanterna começa apagada e escondida.
        app.insert_resource(vael_engine::worldsys::AtmosphereState {
            night: 1.0,
            ..Default::default()
        });
        for _ in 0..8 {
            step(&mut app, 0.1);
        }
        let snap = app.world().resource::<VaelLanternState>().get();
        assert!(!snap.waist_on);
        assert!(!snap.lit);
        assert_eq!(snap.intensity, 0.0);

        let mut q = app
            .world_mut()
            .query_filtered::<(&PointLight, &Visibility), With<TravellerLantern>>();
        let (light, vis) = q.single(app.world()).expect("TravellerLantern spawnada");
        assert_eq!(light.intensity, 0.0);
        assert_eq!(*vis, Visibility::Hidden);
    }

    #[test]
    fn botao_acende_e_apaga_com_som_toast_e_luz() {
        let (mut app, _) = test_app();
        step(&mut app, 0.05);

        // Acende com o botão.
        app.world_mut()
            .resource_mut::<VaelGameplayActions>()
            .lantern_just = true;
        step(&mut app, 0.05);
        app.world_mut()
            .resource_mut::<VaelGameplayActions>()
            .lantern_just = false;
        for _ in 0..10 {
            step(&mut app, 0.1);
        }

        let snap = app.world().resource::<VaelLanternState>().get();
        assert!(snap.waist_on);
        assert!(snap.lit);
        assert!(snap.intensity > 0.85, "intensidade={}", snap.intensity);

        let mut q = app
            .world_mut()
            .query_filtered::<(&PointLight, &Visibility), With<TravellerLantern>>();
        let (light, vis) = q.single(app.world()).expect("lanterna");
        assert!(light.intensity > LANTERN_MAX_INTENSITY * 0.8);
        assert_eq!(*vis, Visibility::Inherited);

        // Apaga com novo toque no botão.
        app.world_mut()
            .resource_mut::<VaelGameplayActions>()
            .lantern_just = true;
        step(&mut app, 0.05);
        app.world_mut()
            .resource_mut::<VaelGameplayActions>()
            .lantern_just = false;
        for _ in 0..15 {
            step(&mut app, 0.1);
        }
        let snap = app.world().resource::<VaelLanternState>().get();
        assert!(!snap.waist_on);
        assert!(!snap.lit);
        let (light, vis) = q.single(app.world()).expect("lanterna");
        assert_eq!(light.intensity, 0.0);
        assert_eq!(*vis, Visibility::Hidden);
    }

    #[test]
    fn monta_na_cintura_clona_material_e_esconde_com_tocha_na_mao() {
        let (mut app, _) = test_app();
        step(&mut app, 0.05);

        let mut pivot_q = app
            .world_mut()
            .query::<(Entity, &WaistLanternPivot, &Visibility)>();
        let (_, _, vis) = pivot_q.single(app.world()).expect("pivô montado em spine_01");
        assert_eq!(*vis, Visibility::Inherited);

        // Simula uma malha filha do GLB com material partilhado.
        let shared_mat = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial::default());
        let mesh_root = app
            .world_mut()
            .query_filtered::<Entity, With<WaistLanternMesh>>()
            .single(app.world())
            .expect("mesh root");
        let child_mesh = app
            .world_mut()
            .spawn(MeshMaterial3d(shared_mat.clone()))
            .id();
        app.world_mut().entity_mut(mesh_root).add_child(child_mesh);

        app.world()
            .resource::<VaelLanternState>()
            .request_set(true);
        for _ in 0..8 {
            step(&mut app, 0.1);
        }
        // O material partilhado NÃO pode ter sido alterado.
        let shared = app
            .world()
            .resource::<Assets<StandardMaterial>>()
            .get(&shared_mat)
            .unwrap();
        assert_eq!(shared.emissive, LinearRgba::BLACK);
        // A malha da lanterna ganhou NotShadowCaster + material clonado aceso.
        assert!(app.world().get::<NotShadowCaster>(child_mesh).is_some());
        let cloned_handle = app
            .world()
            .get::<WaistLanternClonedMaterial>(child_mesh)
            .expect("material clonado")
            .0
            .clone();
        let cloned = app
            .world()
            .resource::<Assets<StandardMaterial>>()
            .get(&cloned_handle)
            .unwrap();
        assert!(cloned.emissive.red > 2.0, "emissivo aceso: {:?}", cloned.emissive);

        // Ao equipar a tocha na mão, o modelo na cintura esconde-se e a luz segue acesa.
        app.world_mut()
            .resource_mut::<TorchLightOverride>()
            .intensity = LANTERN_MAX_INTENSITY;
        step(&mut app, 0.05);
        let (_, _, vis) = pivot_q.single(app.world()).expect("pivô");
        assert_eq!(*vis, Visibility::Hidden);
        assert!(app.world().resource::<VaelLanternState>().get().lit);
    }

    #[test]
    fn bridge_lua_le_e_comanda_a_lanterna() {
        let state = VaelLanternState::default();
        let lua = Lua::new();
        lua.globals()
            .set("viber", lua.create_table().unwrap())
            .unwrap();
        install(&lua, &state).unwrap();

        assert!(!lua.load("return viber.vael.lantern_on()").eval::<bool>().unwrap());
        assert!(!lua.load("return viber.vael.lantern_lit()").eval::<bool>().unwrap());
        lua.load("viber.vael.lantern_set(true)").exec().unwrap();
        assert!(lua.load("return viber.vael.lantern_on()").eval::<bool>().unwrap());
        assert!(lua.load("return viber.vael.lantern_lit()").eval::<bool>().unwrap());
        lua.load("viber.vael.lantern_set(false)").exec().unwrap();
        assert!(!lua.load("return viber.vael.lantern_on()").eval::<bool>().unwrap());
    }

    #[test]
    fn menu_aberto_e_morte_bloqueiam_o_botao() {
        let (mut app, _) = test_app();
        step(&mut app, 0.05);

        app.insert_resource(MenusOpen {
            map: true,
            ..Default::default()
        });
        app.world_mut()
            .resource_mut::<VaelGameplayActions>()
            .lantern_just = true;
        step(&mut app, 0.05);
        assert!(!app.world().resource::<VaelLanternState>().get().waist_on);

        app.world_mut().resource_mut::<MenusOpen>().map = false;
        app.world_mut().resource_mut::<VaelSave>().vitals.max_hp = 100.0;
        app.world_mut().resource_mut::<VaelSave>().vitals.hp = 0.0;
        step(&mut app, 0.05);
        assert!(!app.world().resource::<VaelLanternState>().get().waist_on);
    }
}

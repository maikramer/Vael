//! Cadência das sombras do sol.
//!
//! Com o céu/tempo do `bevy_elements` (`src/environment.rs`) o relógio é
//! contínuo e o sol anda ~0.3°/s; o passe de sombras corre a **15 Hz**
//! (`ELEMENTS_SHADOW_REFRESH_HZ`), com as cascatas congeladas entre refreshes.
//! Sem `<DayCycle>` nem `bevy_elements` (mundos QA antigos), o sistema é um
//! no-op.
//!
//! Com `<DayCycle mode="day-night" shadow-refresh-hz="15">` (A/B antigo), o
//! sol fica
//! parado numa âncora e o mapa de sombras direcional só precisa de ser
//! re-renderizado quando as cascatas se deslocam com a câmara ou quando um
//! caster dinâmico (herói, NPC, criatura) se mexe. Em vez de pagar as 4
//! vistas de sombra em TODOS os frames, o jogo salta o passe nos frames
//! intermédios e re-renderiza a 15 Hz.
//!
//! O mecanismo: `per_view_shadow_pass` do Bevy devolve cedo quando não há
//! uma `BinnedRenderPhase<Shadow>` para a view retida. Removemos a entrada
//! das views direcionais ANTES do passe; a textura de profundidade persiste
//! (o `prepare_lights` volta a pedi-la ao `TextureCache` todos os frames), e
//! o passe principal continua a amostrá-la. Nada de alterar o Bevy.
//!
//! Fora do modo `day-night` (ou durante a travessia dia↔noite, ou com
//! `shadow-refresh-hz="0"`) o sistema não salta nada — o comportamento é o
//! de sempre. Com o relógio contínuo do `bevy_elements` a cadência é
//! `VAEL_SHADOW_HZ` (default 15; 0 = todos os frames). Uma cena nova
//! (`SceneEpoch`) força um render imediato.

use bevy::core_pipeline::schedule::Core3d;
use bevy::light::Cascades;
use bevy::pbr::{EARLY_SHADOW_PASS, LightEntity, Shadow, ShadowView, per_view_shadow_pass};
use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_phase::ViewBinnedRenderPhases;
use bevy::render::view::ExtractedView;
use viber::scene_loader::SceneEpoch;
use viber::worldsys::DayCycleState;

/// Cadência das sombras com o relógio contínuo do `bevy_elements`.
///
/// O sol anda ~0.3°/s (72 s de jogo por segundo real): a 15 Hz o passo é
/// ~0.02°, invisível, e o passe de sombras deixa de correr em 3 de cada 4
/// frames. O céu/relógio continuam contínuos (sem saltos que realocam o
/// probe do céu).
const ELEMENTS_SHADOW_REFRESH_HZ: f32 = 15.0;

/// Cadência pedida por `VAEL_SHADOW_HZ` (default 15; 0 = todos os frames).
fn elements_shadow_hz() -> f32 {
    std::env::var("VAEL_SHADOW_HZ")
        .ok()
        .and_then(|raw| raw.trim().parse::<f32>().ok())
        .filter(|hz| hz.is_finite() && *hz >= 0.0)
        .unwrap_or(ELEMENTS_SHADOW_REFRESH_HZ)
}

/// Pedido main→render: este frame re-renderiza as sombras do sol?
///
/// `active` é só diagnóstico (o modo está ligado e assentado); `render` é o
/// que o system do render app lê. Fora do modo, `render` fica sempre `true`.
#[derive(Resource, Clone, Debug, ExtractResource)]
#[extract_app(RenderApp)]
pub struct VaelShadowRefresh {
    pub active: bool,
    pub render: bool,
}

impl Default for VaelShadowRefresh {
    fn default() -> Self {
        // Primeira extracção: renderiza, mesmo que o primeiro tick chegue
        // com `active` falso (a sombra ainda não existe).
        Self {
            active: false,
            render: true,
        }
    }
}

pub struct VaelShadowPlugin;

impl Plugin for VaelShadowPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VaelShadowRefresh>()
            .add_plugins(ExtractResourcePlugin::<VaelShadowRefresh>::default())
            .add_systems(
                Update,
                drive_shadow_refresh.after(viber::worldsys::daycycle_drive),
            )
            // As cascatas da directional são RELATIVAS À CÂMARA: congelá-las
            // nos frames saltados é obrigatório, senão o passe principal
            // amostrava o mapa antigo com matrizes novas e as sombras
            // deslizavam. Corre depois do construtor do Bevy.
            .add_systems(
                PostUpdate,
                freeze_cached_cascades
                    .after(bevy::light::cascade::build_directional_light_cascades)
                    // As frustas de visibilidade dos casters têm de nascer das
                    // cascatas CONGELADAS, senão o culling diverge do mapa.
                    .before(bevy::light::update_directional_light_frusta),
            );
        // O passe de sombras vive no sub-app de render. O sistema corre
        // depois do `prepare_lights`/queue (ambos no schedule `Render`) e
        // antes do passe direcional do `Core3d`.
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.add_systems(
                Core3d,
                skip_cached_directional_shadows.before(per_view_shadow_pass::<EARLY_SHADOW_PASS>),
            );
        }
    }
}

/// Decide, no mundo principal, se este frame re-renderiza as sombras.
///
/// O acumulador vive aqui (e não no render app) porque o `Time` não é
/// extraído para o mundo do render.
fn drive_shadow_refresh(
    time: Res<Time>,
    clock: Option<Res<DayCycleState>>,
    elements: Option<Res<bevy_elements::WorldClock>>,
    sun: Option<Res<bevy_elements::SunState>>,
    scene: Option<Res<SceneEpoch>>,
    camera: Query<&GlobalTransform, With<Camera3d>>,
    mut refresh: ResMut<VaelShadowRefresh>,
    mut accumulator: Local<f32>,
    mut last_scene: Local<u64>,
    mut initialized: Local<bool>,
    mut was_active: Local<bool>,
    mut last_visual: Local<f32>,
    mut last_camera: Local<Vec3>,
    mut last_light_switch: Local<Option<(bool, bool)>>,
) {
    let (hz, active, visual_changed) = match clock.as_deref() {
        Some(clock) => {
            // A travessia dia↔noite move o sol: aí o passe tem de correr em
            // todos os frames, senão o cache ficava com a sombra do estado
            // anterior.
            let hz = clock.shadow_refresh_hz;
            let active = clock.is_day_night() && clock.visual_transition >= 1.0 && hz > 0.0;
            // Uma edição ao vivo de `day-minute`/`night-minute` muda a âncora:
            // força um render para o novo sol não apanhar o mapa antigo.
            let visual = clock.visual_minute();
            let changed = (visual - *last_visual).abs() > 1e-6;
            *last_visual = visual;
            (hz, active, changed)
        }
        // Relógio contínuo do `bevy_elements`: cadência fixa de 15 Hz. Um
        // salto de minuto nunca acontece em jogo (o relógio avança sempre);
        // o QA arranca já na hora com `VAEL_START_HOUR`.
        None if elements.is_some() => {
            let hz = elements_shadow_hz();
            (hz, hz > 0.0, false)
        }
        None => {
            refresh.active = false;
            refresh.render = true;
            *accumulator = 0.0;
            return;
        }
    };

    // Uma cena nova invalida o mapa: as entidades e as cascatas mudaram.
    let scene_changed = scene.is_some_and(|epoch| {
        let changed = !*initialized || epoch.0 != *last_scene;
        *last_scene = epoch.0;
        changed
    });
    *initialized = true;

    let just_activated = active != *was_active;
    *was_active = active;
    refresh.active = active;

    // O crate troca as sombras do Sol pelas da Lua quando ela nasce: o mapa
    // novo tem de ser desenhado JÁ, não 66 ms depois.
    let light_switch = sun.as_deref().is_some_and(|sun| {
        let now = (sun.altitude_deg > -0.5, sun.moon_altitude_deg > -0.5);
        let switched = *last_light_switch != Some(now);
        *last_light_switch = Some(now);
        switched
    });

    // Teleporte/viagem rápida: as cascatas antigas podem já não cobrir o
    // sítio novo. 50 m num frame é sempre um salto, não corrida.
    let camera_jump = camera.iter().next().is_some_and(|transform| {
        let position = transform.translation();
        let jumped = (position - *last_camera).length_squared() > 50.0 * 50.0;
        *last_camera = position;
        jumped
    });

    if !active || scene_changed || just_activated || visual_changed || light_switch || camera_jump {
        refresh.render = true;
        *accumulator = 0.0;
        return;
    }

    refresh.render = refresh_due(active, hz, time.delta_secs(), &mut accumulator);
}

/// Deve este frame re-renderizar as sombras?
///
/// Separado do sistema para a cadência ser testável sem `App`/`Time`.
fn refresh_due(active: bool, hz: f32, dt: f32, accumulator: &mut f32) -> bool {
    if !active || hz <= 0.0 {
        *accumulator = 0.0;
        return true;
    }
    *accumulator += dt.max(0.0);
    let period = 1.0 / hz;
    if *accumulator >= period {
        // Sobra no acumulador, limitada a MEIO período: um frame longo
        // (stall) não pode deixar dívida que faça o frame seguinte
        // re-renderizar outra vez (o teste cobre 0.25 s seguido de 0.01 s).
        *accumulator = (*accumulator - period).min(period * 0.5);
        true
    } else {
        false
    }
}

/// Salta o passe das sombras direcionais nos frames em que o cache é válido.
///
/// Remover a fase faz `view_shadow_pass` sair antes de abrir o render pass,
/// portanto a depth view NÃO é limpa e o conteúdo do frame anterior fica.
fn skip_cached_directional_shadows(
    refresh: Option<Res<VaelShadowRefresh>>,
    phases: Option<ResMut<ViewBinnedRenderPhases<Shadow>>>,
    views: Query<(&ExtractedView, &LightEntity), With<ShadowView>>,
) {
    let Some(refresh) = refresh else {
        return;
    };
    if refresh.render {
        return;
    }
    let Some(mut phases) = phases else {
        return;
    };
    for (view, light) in &views {
        if matches!(light, LightEntity::Directional { .. }) {
            phases.0.remove(&view.retained_view_entity);
        }
    }
}

/// Mantém as cascatas do último render nos frames em que o passe é saltado.
///
/// Sem isto, o `build_directional_light_cascades` recalculava as matrizes com
/// a câmara atual e o passe principal amostrava o mapa antigo com matrizes
/// novas: a sombra do mundo deslizava a cada frame saltado.
fn freeze_cached_cascades(
    refresh: Res<VaelShadowRefresh>,
    mut lights: Query<(Entity, &mut Cascades)>,
    mut cache: Local<Vec<(Entity, Cascades)>>,
) {
    if refresh.render {
        cache.clear();
        for (entity, cascades) in &lights {
            cache.push((entity, (*cascades).clone()));
        }
        return;
    }
    for (entity, mut cascades) in &mut lights {
        if let Some((_, saved)) = cache.iter().find(|(cached, _)| *cached == entity) {
            *cascades = saved.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::refresh_due;

    /// 15 Hz a 60 fps: 3 frames saltados, o 4.º renderiza.
    #[test]
    fn fifteen_hz_renders_every_fourth_frame() {
        let mut acc = 0.0;
        let frames: Vec<bool> = (0..8)
            .map(|_| refresh_due(true, 15.0, 0.017, &mut acc))
            .collect();
        assert_eq!(
            frames,
            vec![false, false, false, true, false, false, false, true]
        );
    }

    /// Fora do modo (ou com hz=0) nunca salta — o comportamento antigo.
    #[test]
    fn inactive_or_zero_hz_always_renders() {
        let mut acc = 0.0;
        for _ in 0..4 {
            assert!(refresh_due(false, 15.0, 1.0 / 60.0, &mut acc));
            assert!(refresh_due(true, 0.0, 1.0 / 60.0, &mut acc));
        }
        assert_eq!(acc, 0.0);
    }

    /// Um frame mais longo do que o período renderiza e não acumula dívida
    /// (senão passava a renderizar em todos os frames seguintes).
    #[test]
    fn long_frame_renders_and_caps_the_debt() {
        let mut acc = 0.0;
        assert!(refresh_due(true, 15.0, 0.25, &mut acc));
        assert!(acc <= 1.0 / 15.0);
        assert!(!refresh_due(true, 15.0, 0.01, &mut acc));
    }
}

//! Prontidão do mundo: o ecrã de carregamento só sai quando o jogo está
//! inteiro.
//!
//! "Pronto" = o herói existe e assentou (sem [`SpawnGated`]/[`TeleportSettle`]),
//! nenhum `<GltfScene>` **do anel próximo** está à espera do GLB
//! ([`GltfScenePending`] — o resto do mundo carrega por distância, ver
//! [`crate::gltf_residency`]), os spawn groups fecharam e os albedos do
//! terreno estão residentes — e tudo isso
//! estável durante [`STABLE_FRAMES`] frames (as malhas/texturas sobem para a
//! GPU nos frames a seguir ao load, e um sistema pode criar pendentes novos
//! no frame seguinte).
//!
//! Um carregamento é uma *época*: o arranque e cada troca de cena
//! ([`crate::scene_loader::LoaderPhase::Swap`]) começam uma nova. O
//! pico de pendentes da época dá o denominador do progresso do véu, e a
//! transição para pronto escreve no log quanto demorou — a referência para
//! medir o tempo de carregamento.
//!
//! Subida para a GPU: sem [`RenderAssetBytesPerFrame`] o Bevy preparava
//! TODAS as texturas/malhas novas num só frame — ~2 GB de staging no Vael,
//! um frame de ~7 s preso em `vkAllocateMemory` (perfil 2026-11-13). O
//! orçamento por frame ([`loading_upload_budget`]) fatia a subida e
//! reaproveita o staging; a sonda [`GpuUploadProbe`] diz à prontidão se ainda
//! há fila (`gpu_uploading`). Só vale ATRÁS do véu: em jogo há assets grandes
//! re-enviados todos os frames, e o Bevy remove um asset modificado quando o
//! orçamento do frame já esgotou — com 48 MB o mundo ficava negro (medido
//! 2026-11-13). Depois de pronto, o limite sai (comportamento do Bevy).
//!
//! Agentes/QA: `viber.ping` e `viber.scene.current` devolvem `ready`
//! (+ `loading` com o progresso). `viber debug wait-ready` espera por ele.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bevy::prelude::*;
use bevy::render::render_asset::RenderAssetBytesPerFrame;
use serde::Serialize;

use crate::player::{Player, TeleportSettle};
use crate::recipes::spawn::GltfScenePending;
use crate::scene_loader::{LoaderPhase, SceneLoader};
use crate::spawn_gate::SpawnGated;

/// Frames seguidos com tudo pronto antes de declarar o mundo pronto.
pub const STABLE_FRAMES: u32 = 12;
/// Tempo mínimo de uma época (s, tempo real): o mundo nasce em `Startup`, mas
/// o terreno/spawners ainda criam pendentes nos primeiros frames.
pub const MIN_EPOCH_SECS: f32 = 0.5;

/// Faixa de `Camera::order` das câmaras de bake de impostores do
/// `bevy_flora` (`-10_000 + índice`).
const IMPOSTOR_BAKE_ORDER: isize = -10_000 + 4_096;
/// Rede de segurança (s): um GLB que nunca resolve não pode prender o jogo
/// atrás do véu para sempre. `VIBER_LOADING_MAX_S` substitui-o.
pub const DEFAULT_MAX_SECS: f32 = 240.0;

/// Orçamento de subida por frame durante o carregamento (MiB). Atrás do véu
/// o frame pode ser pesado; o que interessa é não alocar o mundo inteiro de
/// staging de uma vez. `VIBER_UPLOAD_MB` substitui-o.
pub const LOADING_UPLOAD_MB: usize = 256;

/// `VIBER_UPLOAD_MB=off|0`: desliga o orçamento (diagnóstico).
pub fn upload_budget_disabled() -> bool {
    bevy_engine_core::envs::var_lossy("UPLOAD_MB").is_some_and(|v| matches!(v.trim(), "off" | "0"))
}

/// `VAEL_WAIT_BAKES=0`: o véu deixa de esperar pelos bakes de impostores.
///
/// Por omissão **espera** — e a 2026-11-16 isso revelou-se o comportamento
/// honesto: sem esperar, o véu saía ~4 s mais cedo e o ecrã abria com terreno e
/// HUD mas **sem props** (capturado em série). A causa não eram as bakes: era a
/// condição de upload e a falta de instanciação das cenas (ver `instantiating` e
/// `gpu_uploading` em [`update_world_readiness`]). Com as duas corrigidas, o
/// `=0` também mostra a vista completa; esperar continua a ser o default porque
/// as bakes correm por cima da fila de upload e não custam tempo de parede
/// mensurável.
pub fn wait_for_bakes() -> bool {
    !std::env::var("VAEL_WAIT_BAKES").is_ok_and(|v| matches!(v.trim(), "0" | "false" | "off"))
}

/// Orçamento efetivo do carregamento (bytes).
pub fn loading_upload_budget() -> usize {
    bevy_engine_core::envs::var_lossy("UPLOAD_MB")
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(LOADING_UPLOAD_MB)
        * 1024
        * 1024
}

/// Bytes que a render world pediu para subir no último frame (o limitador
/// soma o pedido mesmo quando adia o asset): `>= orçamento` = há fila.
#[derive(Resource, Clone, Default)]
pub struct GpuUploadProbe(pub Arc<AtomicUsize>);

/// Pipelines de render ainda a compilar (render world → main world). Uma
/// malha cujo pipeline não está pronto NÃO desenha: sem esperar por isto o
/// véu abria com só o herói e as partículas visíveis — o resto (terreno,
/// acampamento, árvores) aparecia segundos depois sobre o "chão" do céu
/// (visto 2026-11-15 na cena de menu e no `main`).
#[derive(Resource, Clone, Default)]
pub struct PipelineProbe(pub Arc<AtomicUsize>);

/// Teto efetivo da espera (s).
pub fn max_wait_secs() -> f32 {
    bevy_engine_core::envs::var_lossy("LOADING_MAX_S")
        .and_then(|v| v.trim().parse::<f32>().ok())
        .filter(|v| *v > 0.0)
        .unwrap_or(DEFAULT_MAX_SECS)
}

/// Estado do carregamento da época atual.
#[derive(Resource, Debug, Clone, Default, Serialize, Reflect)]
#[reflect(Resource)]
pub struct WorldReadiness {
    /// O mundo está inteiro (ou a época esgotou o teto).
    pub ready: bool,
    /// `ready` por esgotar o teto, não por estar tudo carregado.
    pub timed_out: bool,
    /// `<GltfScene>` ainda à espera do GLB.
    pub pending_models: usize,
    /// Pico de pendentes desta época (denominador do progresso).
    pub total_models: usize,
    pub textures_ready: bool,
    pub player_ready: bool,
    pub spawn_groups_ready: bool,
    /// A render world ainda tem texturas/malhas em fila para a GPU.
    pub gpu_uploading: bool,
    /// Progresso mostrado no véu (0..1, só sobe dentro da época): etapas
    /// pesadas — modelos 50%, GPU 20%, texturas do terreno/spawn/herói 10%
    /// cada. Os GLB costumam acabar ainda no `Startup` (o pico de pendentes
    /// fica 1), por isso a barra não pode ser só modelos.
    pub shown_progress: f32,
    /// Segundos (reais) desde o início da época.
    pub elapsed_secs: f32,
    /// Duração da última época até ficar pronta (s).
    pub last_load_secs: Option<f32>,
    /// Quantas épocas já correram (0 = arranque).
    pub epoch: u32,
    #[serde(skip)]
    stable_frames: u32,
    #[serde(skip)]
    started_at: f64,
    #[serde(skip)]
    started: bool,
}

impl WorldReadiness {
    /// Progresso 0..1 dos modelos desta época.
    pub fn progress(&self) -> f32 {
        if self.total_models == 0 {
            return if self.ready { 1.0 } else { 0.0 };
        }
        let done = self.total_models.saturating_sub(self.pending_models);
        (done as f32 / self.total_models as f32).clamp(0.0, 1.0)
    }

    /// Fração de modelos resolvidos; sem pendentes conta como feita.
    fn progress_models_or_done(&self) -> f32 {
        if self.pending_models == 0 {
            1.0
        } else {
            self.progress()
        }
    }

    /// Recomeça a contagem (troca de cena).
    pub fn restart(&mut self, now: f64) {
        let epoch = self.epoch + u32::from(self.started);
        let last = self.last_load_secs;
        *self = Self {
            epoch,
            last_load_secs: last,
            started_at: now,
            started: true,
            ..Default::default()
        };
    }

    /// Linha curta para o véu.
    pub fn status_line(&self) -> String {
        if self.pending_models > 0 {
            let done = self.total_models.saturating_sub(self.pending_models);
            return format!(
                "a carregar modelos {done}/{} ({:.0}%)",
                self.total_models,
                self.progress() * 100.0
            );
        }
        if self.gpu_uploading {
            return "a enviar texturas para a placa...".into();
        }
        if !self.textures_ready {
            return "a carregar texturas do terreno...".into();
        }
        if !self.spawn_groups_ready {
            return "a povoar o mundo...".into();
        }
        if !self.player_ready {
            return "a assentar o herói...".into();
        }
        "quase pronto...".into()
    }
}

/// JSON da prontidão para a bridge (`viber.ping`/`viber.scene.current`).
/// Sem o recurso (apps mínimas) conta como pronto; no modo Edit é sempre.
pub fn readiness_json(world: &World) -> serde_json::Value {
    match world.get_resource::<WorldReadiness>() {
        Some(r) => {
            let mut value = serde_json::to_value(r).unwrap_or_default();
            value["progress"] = serde_json::json!(r.shown_progress);
            value["status"] = serde_json::json!(r.status_line());
            value
        }
        None => serde_json::json!({ "ready": true }),
    }
}

/// O carregamento está a decorrer (o mundo ainda não pode ser mostrado).
pub fn world_loading(readiness: Option<Res<WorldReadiness>>) -> bool {
    readiness.is_some_and(|r| !r.ready)
}

/// O véu pausou o `Time<Virtual>` (e só ele o despausa ao fechar a época).
#[derive(Resource, Default)]
pub struct LoadingTimePause(pub bool);

/// O loading é um momento CONGELADO: sem física, sem tempo a correr, sem
/// IA/animação — o mundo assenta (colliders, GLB, texturas) e só então o
/// jogo acorda, já com o herói colocado no chão pelo portão de spawn.
/// O `Time<Real>` continua (a prontidão mede-se nele), e o render também:
/// é o que permite carregar atrás do véu.
fn freeze_time_while_loading(
    readiness: Option<Res<WorldReadiness>>,
    mut paused_by_us: ResMut<LoadingTimePause>,
    mut time: ResMut<Time<bevy::time::Virtual>>,
) {
    let loading = readiness.is_some_and(|r| !r.ready);
    if loading {
        if !paused_by_us.0 {
            time.pause();
            paused_by_us.0 = true;
        }
    }
    // Só despausa o que NÓS pausámos: o `pause` do debugger (`FrameStepper`)
    // também mexe no `Time<Virtual>` e não pode ser desfeito pelo véu.
    else if paused_by_us.0 {
        time.unpause();
        paused_by_us.0 = false;
    }
}

pub struct WorldReadyPlugin;

impl Plugin for WorldReadyPlugin {
    fn build(&self, app: &mut App) {
        let probe = GpuUploadProbe::default();
        let pipelines = PipelineProbe::default();
        app.insert_resource(pipelines.clone());
        app.init_resource::<WorldReadiness>()
            .init_resource::<LoadingTimePause>()
            .register_type::<WorldReadiness>()
            .insert_resource(probe.clone())
            .add_systems(
                PostUpdate,
                (update_world_readiness, set_upload_budget).chain(),
            )
            .add_systems(bevy::app::Update, freeze_time_while_loading)
            .add_systems(
                PostUpdate,
                suspend_sun_shadows_while_baking
                    .before(bevy::light::SimulationLightSystems::UpdateDirectionalLightCascades),
            );
        if let Some(render_app) = app.get_sub_app_mut(bevy::render::RenderApp) {
            render_app.insert_resource(probe).insert_resource(pipelines).add_systems(
                bevy::render::Render,
                (
                    probe_pipelines.in_set(bevy::render::RenderSystems::Cleanup),
                    unthrottle_until_stbn.in_set(bevy::render::RenderSystems::ExtractCommands),
                    probe_uploads
                        .in_set(bevy::render::RenderSystems::Cleanup)
                        .before(bevy::render::render_asset::reset_render_asset_bytes_per_frame),
                ),
            );
        }
    }
}

/// Render world: frame SEM orçamento enquanto a textura STBN (blue noise) não
/// estiver na GPU. O `prepare_mesh_view_bind_groups` do Bevy 0.20 faz
/// `expect` nela ("STBN texture is added unconditionally"); com orçamento ela
/// podia ficar para o frame seguinte, o sistema entrava em pânico, envenenava
/// um mutex e a vista 3D ficava negra para sempre (medido 2026-11-13). As
/// outras LUTs (DFG/LTC) têm fallback.
fn unthrottle_until_stbn(
    bluenoise: Option<Res<bevy::pbr::Bluenoise>>,
    images: Res<bevy::render::render_asset::RenderAssets<bevy::render::texture::GpuImage>>,
    mut limiter: ResMut<bevy::render::render_asset::RenderAssetBytesPerFrameLimiter>,
) {
    if limiter.max_bytes.is_none() {
        return;
    }
    let ready = bluenoise.is_none_or(|b| images.get(&b.texture).is_some());
    if !ready {
        limiter.max_bytes = None;
    }
}

/// Render world: copia os bytes pedidos neste frame antes do reset.
fn probe_uploads(
    limiter: Res<bevy::render::render_asset::RenderAssetBytesPerFrameLimiter>,
    probe: Res<GpuUploadProbe>,
) {
    probe.0.store(
        limiter.bytes_written.load(Ordering::Relaxed),
        Ordering::Relaxed,
    );
}

/// Sombras do sol desligadas enquanto há bakes de impostores.
///
/// O Bevy constrói as cascatas da luz direccional para TODAS as câmaras
/// activas, sem olhar às render layers: cada uma das 64 câmaras de bake do
/// `bevy_flora` desenhava as 4 cascatas do mundo inteiro. Medido a
/// 2026-11-15: ~330 ms por frame só com os bakes (3 fps atrás do véu, 40 s de
/// carregamento na cena de menu). Os bakes correm atrás do véu (ver
/// [`update_world_readiness`]), por isso a vista do jogo não perde nada; o
/// dono da flag (o `bevy_elements`) repõe-na no frame seguinte ao último bake.
fn suspend_sun_shadows_while_baking(
    cameras: Query<(&Camera, &bevy::camera::RenderTarget)>,
    mut lights: Query<&mut DirectionalLight>,
) {
    let baking = cameras.iter().any(|(camera, target)| {
        camera.is_active
            && camera.order <= IMPOSTOR_BAKE_ORDER
            && matches!(target, bevy::camera::RenderTarget::Image(_))
    });
    if !baking {
        return;
    }
    for mut light in &mut lights {
        if light.shadow_maps_enabled {
            light.shadow_maps_enabled = false;
        }
    }
}

/// Render world: quantos pipelines estão na fila de compilação.
fn probe_pipelines(
    cache: Option<Res<bevy::render::render_resource::PipelineCache>>,
    probe: Res<PipelineProbe>,
) {
    let waiting = cache.map_or(0, |cache| cache.waiting_pipelines().count());
    probe.0.store(waiting, Ordering::Relaxed);
}

/// Orçamento de subida: só atrás do véu; em jogo, sem limite.
///
/// Publica também o `LoadingStreaming` que o terreno lê: com o véu em cima a
/// vaga de colunas a seguir ao bootstrap é trabalho de carga e ao orçamento
/// normal (4 caixas/frame) segurava o véu vários segundos.
fn set_upload_budget(
    readiness: Res<WorldReadiness>,
    budget: Option<ResMut<RenderAssetBytesPerFrame>>,
    mut loading: Option<ResMut<bevy_engine_core::cinematic::LoadingStreaming>>,
) {
    let world_loading = !readiness.ready;
    if let Some(loading) = loading.as_deref_mut() {
        loading.0 = world_loading;
    }
    let Some(mut budget) = budget else {
        return;
    };
    // `VIBER_UPLOAD_MB=off`: sem orçamento (o comportamento antigo do Bevy).
    if upload_budget_disabled() {
        return;
    }
    let max = world_loading.then(loading_upload_budget);
    if budget.max_bytes != max {
        budget.max_bytes = max;
    }
}

#[allow(clippy::too_many_arguments)]
fn update_world_readiness(
    time: Res<Time<Real>>,
    mut readiness: ResMut<WorldReadiness>,
    pending: Query<(&GltfScenePending, Option<&GlobalTransform>)>,
    // Âncoras do streaming (`gltf_residency`) que ainda NÃO pediram o GLB:
    // sem as contar, o véu via 0 pendentes nos frames antes de a residência
    // correr com o herói e abria sobre um mundo vazio (visto 2026-11-15: o
    // título abria sobre chão bege, sem acampamento; o jogo sobre preto).
    anchors: Query<
        (
            &GlobalTransform,
            Has<GltfScenePending>,
            Has<bevy::world_serialization::WorldAssetRoot>,
            Has<bevy::world_serialization::WorldInstance>,
            Has<crate::gltf_residency::GltfSceneFailed>,
        ),
        With<crate::gltf_residency::GltfSceneSpec>,
    >,
    terrain_lod: Option<Res<crate::terrain::plugin::ChunkLodState>>,
    // Bakes de impostores do `bevy_flora` (câmaras offscreen na faixa de
    // `order` ≤ −10 000). Enquanto correm, a vista do jogo sai sem geometria
    // — só céu, herói e partículas sobre o "chão" bege da atmosfera (medido
    // 2026-11-15: ~5 s depois do véu; `VAEL_FLORA_IMPOSTOR=0` fazia sumir).
    // São trabalho de carregamento: o véu espera por eles.
    bake_cameras: Query<(&Camera, &bevy::camera::RenderTarget)>,
    // `GlobalTransform` opcional: apps mínimas (e testes) spawnam o `Player`
    // sem transform — a prontidão não pode depender dele.
    players: Query<
        (Option<&GlobalTransform>, Has<SpawnGated>, Has<TeleportSettle>),
        With<Player>,
    >,
    server: Option<Res<AssetServer>>,
    gate: Option<Res<crate::terrain::runtime::TerrainTextureGate>>,
    groups: Option<Res<crate::spawner::PendingSpawnGroups>>,
    loader: Option<Res<SceneLoader>>,
    mode: Option<Res<crate::app::RunMode>>,
    probe: Option<Res<GpuUploadProbe>>,
    pipeline_probe: Option<Res<PipelineProbe>>,
    budget: Option<Res<RenderAssetBytesPerFrame>>,
    frame: Res<bevy::diagnostic::FrameCount>,
) {
    // No editor o mundo não "carrega" para jogar (sem véu, sem herói).
    if mode.as_deref() == Some(&crate::app::RunMode::Edit) {
        readiness.ready = true;
        return;
    }
    let now = time.elapsed_secs_f64();
    // Troca de cena: o varrimento + a cena nova são uma época nova. A fase
    // `Swap` dura exatamente um frame (o `Last` desse frame troca e passa a
    // `Settle`), já com o ecrã a preto; durante `FadeOut` o mundo antigo
    // continua pronto e visível. Só o `Swap` reabre: reabrir também no
    // `Settle` voltava a fechar a época que acabou de ficar pronta.
    let swapping = loader
        .as_deref()
        .is_some_and(|l| l.phase == LoaderPhase::Swap);
    if !readiness.started || (swapping && readiness.ready) {
        // Primeiro `PostUpdate`: tudo o que é síncrono (parse, spawn, terreno)
        // já correu, e a contagem do véu começa aqui. A marca fecha a metade
        // do arranque que o `mundo pronto em X s` não via.
        bevy_engine_core::boot::mark("first_frame");
        readiness.restart(now);
    }

    // Só o anel do véu conta: o mundo entra por distância
    // (`gltf_residency`) e o que está longe carrega em segundo plano — o véu
    // não pode esperar pelo mapa inteiro. Sem herói (ou com a residência
    // desligada, `ready_radius` infinito) conta tudo, como antes.
    let ready_radius = crate::gltf_residency::ready_radius();
    let hero = players
        .iter()
        .filter_map(|(transform, _, _)| transform)
        .next()
        .map(|t| t.translation());
    let pending_models = match hero {
        Some(hero) if ready_radius.is_finite() => pending
            .iter()
            .filter(|(_, transform)| {
                transform.is_none_or(|t| t.translation().distance(hero) <= ready_radius)
            })
            .count(),
        _ => pending.iter().count(),
    };
    // Âncoras do anel ainda por pedir. Sem herói ainda, o anel é
    // desconhecido: qualquer âncora não residente segura o véu (o herói
    // nasce no mesmo load; contar todas evita abrir antes de ele existir).
    let unrequested = anchors
        .iter()
        .filter(|(_, pending, root, instance, failed)| {
            !(*pending || *root || *instance || *failed)
        })
        .filter(|(transform, ..)| match hero {
            Some(hero) if ready_radius.is_finite() => {
                transform.translation().distance(hero) <= ready_radius
            }
            Some(_) => true,
            None => false,
        })
        .count();
    // Âncoras do anel com a raiz pedida mas SEM instância. O
    // `gltf_scene_spawner` tira o `GltfScenePending` no frame em que o handle
    // carrega e escreve o `WorldAssetRoot`; a subárvore só é montada pelo
    // `world_instance_spawner` no frame seguinte (e pode demorar mais com a
    // fila cheia). Entre os dois a âncora não era pendente NEM "por pedir":
    // era invisível, e o véu abria com o mundo por instanciar — ecrã só com
    // terreno e HUD, zero props (visto 2026-11-16; as bakes de impostores
    // tapavam-no por acaso, ao atrasar o véu ~4 s).
    let instantiating = anchors
        .iter()
        .filter(|(_, pending, root, instance, failed)| {
            *root && !*instance && !*pending && !*failed
        })
        .filter(|(transform, ..)| match hero {
            Some(hero) if ready_radius.is_finite() => {
                transform.translation().distance(hero) <= ready_radius
            }
            Some(_) => true,
            None => false,
        })
        .count();
    let pending_models = pending_models + unrequested + instantiating;
    // Colunas do terreno por meshar perto da câmara: sem isto o véu saía com
    // o chão ainda por desenhar.
    let bakes_active = bake_cameras
        .iter()
        .filter(|(camera, target)| {
            camera.is_active
                && camera.order <= IMPOSTOR_BAKE_ORDER
                && matches!(target, bevy::camera::RenderTarget::Image(_))
        })
        .count();
    // `VAEL_WAIT_BAKES=0` tira os bakes da condição do véu (A/B): são ~42
    // variantes × 16 câmaras. Com as condições de instanciação e de upload
    // honestas (ver abaixo) a vista sai completa nos dois braços.
    let baking = if wait_for_bakes() { bakes_active } else { 0 };
    let compiling = pipeline_probe
        .as_deref()
        .map_or(0, |probe| probe.0.load(Ordering::Relaxed));
    let terrain_busy = terrain_lod.as_deref().is_some_and(|lod| lod.busy);
    let textures_ready = match (&server, &gate) {
        (Some(server), Some(gate)) => gate.ready(server),
        _ => true,
    };
    // O portão de spawn segura o herói ATÉ a época fechar (e o véu espera
    // pelo portão), por isso um herói `gated` não é "ainda não pronto" —
    // exigi-lo aqui embrulhava o véu num impasse. O que não pode é um
    // `TeleportSettle` preso. Sem herói nenhum (a cena de menu por
    // exemplo) não bloqueia: não há a quem assentar.
    let player_ready = players.is_empty()
        || players.iter().any(|(_, _gated, settling)| !settling);
    let spawn_groups_ready = groups
        .as_deref()
        .is_none_or(|g| g.groups.iter().all(|g| g.done));

    // "Ainda a subir" = o render world ESCREVEU alguma coisa neste frame.
    //
    // Comparar com o teto (`>= max`) não servia: o limiter do Bevy clampa o
    // último asset que não cabe e deixa os seguintes para o frame a seguir, por
    // isso um frame com fila cheia pode escrever muito abaixo do teto (medido:
    // 380 KB e 29 MB com o mundo por subir). O véu contava esses frames como
    // parados e abria com os props invisíveis — as bakes de impostores tapavam
    // o buraco por acaso, ao atrasar o véu ~4 s. Com `bytes_written > 0` e os
    // `STABLE_FRAMES` já exigidos, o véu só sai depois de 12 frames sem
    // escrever nada (fila drenada).
    let gpu_uploading = probe
        .as_deref()
        .is_some_and(|probe| probe.0.load(Ordering::Relaxed) > 0);
    // Diagnóstico: os números crus da sonda (`bytes_written` do frame anterior
    // vs o teto) — sem eles não se distingue "fila a drenar" de "sonda a ler 0".
    let upload_debug = (
        probe
            .as_deref()
            .map_or(0, |p| p.0.load(Ordering::Relaxed)),
        budget.as_deref().and_then(|b| b.max_bytes).unwrap_or(0),
    );

    let r = readiness.as_mut();
    r.gpu_uploading = gpu_uploading;
    r.pending_models = pending_models;
    r.total_models = r.total_models.max(pending_models);
    r.textures_ready = textures_ready;
    r.player_ready = player_ready;
    r.spawn_groups_ready = spawn_groups_ready;
    r.elapsed_secs = (now - r.started_at) as f32;
    if r.ready {
        return;
    }
    // Rasto por segundo (`RUST_LOG=bevy_ext_viber_engine::world_ready=debug`):
    // é a medição do que segura o véu.
    if r.elapsed_secs.floor() != ((now - r.started_at) as f32 - time.delta_secs()).floor() {
        debug!(
            "carregamento t={:.1}s frame={} modelos {}/{} (por pedir {} instanciando {}) texturas={} herói={} spawn_groups={} gpu_fila={} [{}B/{}B] terreno_ocupado={} pipelines={} bakes={} (players={} settling={} gated={})",
            r.elapsed_secs,
            frame.0,
            r.total_models.saturating_sub(pending_models),
            r.total_models,
            unrequested,
            instantiating,
            textures_ready,
            player_ready,
            spawn_groups_ready,
            gpu_uploading,
            upload_debug.0,
            upload_debug.1,
            terrain_busy,
            compiling,
            baking,
            players.iter().count(),
            players.iter().filter(|(_, _, s)| *s).count(),
            players.iter().filter(|(_, g, _)| *g).count()
        );
    }
    let stage = 0.5 * r.progress_models_or_done()
        + 0.2 * f32::from(u8::from(pending_models == 0 && !gpu_uploading))
        + 0.1 * f32::from(u8::from(textures_ready))
        + 0.1 * f32::from(u8::from(spawn_groups_ready))
        + 0.1 * f32::from(u8::from(player_ready));
    r.shown_progress = r.shown_progress.max(stage.clamp(0.0, 1.0));
    let all = pending_models == 0
        && !terrain_busy
        && compiling == 0
        && baking == 0
        // Um `Player` sem `GlobalTransform` é uma app mínima/teste: não há
        // herói a assentar nem posição para o anel — não bloqueia (ver o
        // contrato do parâmetro `players`). Sem players nenhuns (menu) idem.
        && (hero.is_some() || players.iter().all(|(transform, ..)| transform.is_none()))
        && textures_ready
        && player_ready
        && spawn_groups_ready
        && !gpu_uploading;
    r.stable_frames = if all { r.stable_frames + 1 } else { 0 };
    let settled = r.stable_frames >= STABLE_FRAMES && r.elapsed_secs >= MIN_EPOCH_SECS;
    let timed_out = !settled && r.elapsed_secs >= max_wait_secs();
    if settled || timed_out {
        r.ready = true;
        r.timed_out = timed_out;
        r.shown_progress = 1.0;
        r.last_load_secs = Some(r.elapsed_secs);
        if timed_out {
            warn!(
                "mundo: carregamento esgotou {:.0}s — mostra-se incompleto \
                 ({} modelos pendentes, texturas={}, herói={}, spawn groups={})",
                r.elapsed_secs, pending_models, textures_ready, player_ready, spawn_groups_ready
            );
        } else {
            info!(
                "mundo pronto em {:.2}s ({} modelos, época {})",
                r.elapsed_secs, r.total_models, r.epoch
            );
        }
        // O tempo do véu começa no primeiro `PostUpdate`: o parse do `.bsn`, o
        // spawn do mundo, o bootstrap do terreno e as pools de flora já cá
        // estavam e não apareciam em lado nenhum. Uma linha, uma vez.
        bevy_engine_core::boot::mark("veil");
        if bevy_engine_core::boot::enabled() {
            info!("{}", bevy_engine_core::boot::report());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progresso_conta_feitos_sobre_o_pico() {
        let r = WorldReadiness {
            pending_models: 25,
            total_models: 100,
            ..Default::default()
        };
        assert!((r.progress() - 0.75).abs() < 1e-6);
        assert!(r.status_line().contains("75/100"));
    }

    #[test]
    fn restart_avanca_a_epoca_e_guarda_a_ultima_duracao() {
        let mut r = WorldReadiness::default();
        r.restart(1.0);
        assert_eq!(r.epoch, 0);
        r.ready = true;
        r.last_load_secs = Some(3.0);
        r.restart(10.0);
        assert_eq!(r.epoch, 1);
        assert!(!r.ready);
        assert_eq!(r.last_load_secs, Some(3.0));
    }

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, WorldReadyPlugin));
        app
    }

    /// Sem herói (cena de menu) não há assentamento a esperar — o
    /// `player_ready` fica verdadeiro. O que o véu não faz é abrir antes da
    /// época mínima (o mundo ainda está a nascer nos primeiros frames).
    #[test]
    fn sem_heroi_o_veu_espera_a_epoca_minima() {
        let mut app = app();
        for _ in 0..40 {
            app.update();
        }
        let r = app.world().resource::<WorldReadiness>();
        assert!(!r.ready);
        assert!(r.player_ready);
    }

    #[test]
    fn heroi_assente_e_sem_pendentes_fica_pronto() {
        let mut app = app();
        app.world_mut().spawn(Player::default());
        let start = std::time::Instant::now();
        while !app.world().resource::<WorldReadiness>().ready {
            app.update();
            assert!(start.elapsed().as_secs() < 5, "nunca ficou pronto");
        }
        let r = app.world().resource::<WorldReadiness>();
        assert!(!r.timed_out);
        assert!(r.elapsed_secs >= MIN_EPOCH_SECS);
    }
}

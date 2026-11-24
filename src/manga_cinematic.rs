//! Cinemáticas em pranchas de mangá (GDD §9 — *Pranchas — todas as cinemáticas*).
//!
//! Cada cinemática vive em `data/cutscenes/<id>.json` e compõe páginas de mangá
//! em tempo real sobre um lienzo 16:9:
//! - quadros poligonais arbitrários (cortes diagonais, lâminas, trapézios) com
//!   contorno de tinta de pena, sombra de calha, *Ken Burns* suave, linhas de
//!   velocidade radiais e revelação em lâmina (`manga_panel.wesl`);
//! - bolhas de diálogo (`speech` com cauda direccional contínua, `shout` em
//!   explosão dentada, `whisper` tracejado, `narration` em ficha de caderno de
//!   cartógrafo) e onomatopeias inclinadas (`sfx`) (`manga_bubble.wesl`);
//! - leitura compasso a compasso (*beat*), com avanço automático ou manual
//!   (`Espaço`/`Enter`/clique), recuo (`←`/`Backspace`) e salto (`Esc`).
//!
//! Integra com o contrato existente da engine (`PlayCinematic`, `StopCinematic`,
//! `CinematicPlayback::external`, `ExternalCinematicIds`, `MenusOpen::cinematic`,
//! `CinematicEnded` e `ScriptGameEvent::CinematicEnd`) para que `vael.cinematic`,
//! `CinematicTrigger` e `vael.cinematic.play` funcionem sem distinção.
//!
//! Gate: `VAEL_MANGA_CINEMATICS=0` desliga e devolve tudo aos planos 3D legados.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use bevy::asset::embedded_asset;
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use bevy::ui::{GlobalZIndex, UiTransform};
use bevy::ui_render::prelude::{MaterialNode, UiMaterial, UiMaterialPlugin};
use bevy_ext_core::{MenusOpen, SfxEvent};
use serde::{Deserialize, Serialize};
use vael_engine::a11y::A11y;
use vael_engine::cinematic::{
    CinematicEnded, CinematicPlayback, ExternalCinematicIds, PlayCinematic, StopCinematic,
};
use vael_engine::luau::{ScriptEventQueue, ScriptGameEvent, sfx_clip_from_str};

/// Variável de ambiente do gate (`0` desliga o leitor de mangá).
pub const MANGA_ENV: &str = "VAEL_MANGA_CINEMATICS";

/// Pasta canónica dos ficheiros de cinemáticas em pranchas.
pub const CUTSCENES_DIR: &str = "data/cutscenes";

/// Camada acima do HUD/menus/título (200) e abaixo do ecrã de morte (300).
const Z_MANGA: i32 = 250;

/// Tempo mínimo antes de aceitar `Esc` (evita saltar no mesmo frame que fechou o título).
const SKIP_GUARD_SECS: f32 = 0.35;

/// Duração da animação de entrada de um quadro (s).
const PANEL_REVEAL_SECS: f32 = 0.42;

/// Duração do pop-in de uma bolha (s).
const BUBBLE_POP_SECS: f32 = 0.24;

/// Duração do fade de viragem de página (s).
const PAGE_FADE_SECS: f32 = 0.28;

/// Intervalo de polling de hot-reload dos JSON (s).
const HOT_RELOAD_INTERVAL: f32 = 1.0;

#[must_use]
pub fn manga_enabled() -> bool {
    std::env::var(MANGA_ENV).as_deref() != Ok("0")
}

// ── Esquema JSON ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaperStyle {
    #[default]
    Parchment,
    Night,
    Charcoal,
}

impl PaperStyle {
    fn page_color(self) -> Color {
        match self {
            PaperStyle::Parchment => Color::srgb(0.918, 0.878, 0.788),
            PaperStyle::Night => Color::srgb(0.075, 0.072, 0.088),
            PaperStyle::Charcoal => Color::srgb(0.145, 0.135, 0.128),
        }
    }

    fn desk_color(self) -> Color {
        match self {
            PaperStyle::Parchment => Color::srgb(0.085, 0.078, 0.072),
            PaperStyle::Night => Color::srgb(0.032, 0.030, 0.038),
            PaperStyle::Charcoal => Color::srgb(0.055, 0.052, 0.050),
        }
    }

    fn ink_color(self) -> Vec4 {
        match self {
            PaperStyle::Parchment => Vec4::new(0.078, 0.070, 0.064, 1.0),
            PaperStyle::Night => Vec4::new(0.88, 0.85, 0.79, 1.0),
            PaperStyle::Charcoal => Vec4::new(0.05, 0.048, 0.045, 1.0),
        }
    }

    fn hud_text_color(self) -> Color {
        match self {
            PaperStyle::Parchment => Color::srgba(0.16, 0.14, 0.12, 0.82),
            PaperStyle::Night | PaperStyle::Charcoal => Color::srgba(0.91, 0.88, 0.80, 0.84),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevealKind {
    Fade,
    #[default]
    SlashRight,
    SlashLeft,
    SlashDown,
    Pop,
}

impl RevealKind {
    fn as_f32(self) -> f32 {
        match self {
            RevealKind::Fade => 0.0,
            RevealKind::SlashRight => 1.0,
            RevealKind::SlashLeft => 2.0,
            RevealKind::SlashDown => 3.0,
            RevealKind::Pop => 4.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BubbleKind {
    #[default]
    Speech,
    Shout,
    Whisper,
    Narration,
    Sfx,
}

impl BubbleKind {
    fn shader_kind(self) -> f32 {
        match self {
            BubbleKind::Speech => 0.0,
            BubbleKind::Shout => 1.0,
            BubbleKind::Whisper => 2.0,
            BubbleKind::Narration => 3.0,
            BubbleKind::Sfx => 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccentTone {
    #[default]
    None,
    Amber,
    Turquoise,
}

impl AccentTone {
    fn vec4(self) -> Vec4 {
        match self {
            AccentTone::None => Vec4::new(0.54, 0.48, 0.38, 1.0),
            AccentTone::Amber => Vec4::new(0.88, 0.54, 0.22, 1.0),
            AccentTone::Turquoise => Vec4::new(0.29, 0.72, 0.78, 1.0),
        }
    }

    fn color(self) -> Color {
        let v = self.vec4();
        Color::srgba(v.x, v.y, v.z, v.w)
    }
}

fn default_true() -> bool {
    true
}
fn default_duration() -> f32 {
    4.0
}
fn default_border_px() -> f32 {
    5.0
}
fn default_zoom() -> f32 {
    1.04
}
fn default_bubble_width() -> f32 {
    0.28
}
fn default_sfx_size() -> f32 {
    34.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TitleCardSpec {
    pub text: String,
    #[serde(default)]
    pub sub: String,
    #[serde(default)]
    pub beat: usize,
    #[serde(default = "default_title_pos")]
    pub pos: [f32; 2],
}

fn default_title_pos() -> [f32; 2] {
    [0.05, 0.05]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MangaPanel {
    pub id: String,
    /// Caminho relativo aos roots de assets (`assets/public/...` → `"cutscenes/prologo/p1.png"`).
    pub image: String,
    /// 4 vértices horários `[TL, TR, BR, BL]` em coordenadas normalizadas da página `[0..1]`.
    pub quad: [[f32; 2]; 4],
    #[serde(default)]
    pub beat: usize,
    #[serde(default = "default_duration")]
    pub duration: f32,
    #[serde(default)]
    pub reveal: RevealKind,
    #[serde(default = "default_border_px")]
    pub border_px: f32,
    #[serde(default)]
    pub pan_from: [f32; 2],
    #[serde(default)]
    pub pan_to: [f32; 2],
    #[serde(default = "default_zoom")]
    pub zoom_from: f32,
    #[serde(default = "default_zoom")]
    pub zoom_to: f32,
    #[serde(default)]
    pub speedlines: f32,
    #[serde(default)]
    pub shake: f32,
    #[serde(default)]
    pub flash: f32,
    #[serde(default)]
    pub sfx: Option<String>,
    #[serde(default)]
    pub accent: AccentTone,
    /// Metadados de geração (`tools/gen_cutscene.py`); ignorados pelo runtime.
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub seed: Option<u64>,
    #[serde(default)]
    pub reference: Option<String>,
}

impl MangaPanel {
    /// Caixa envolvente `[min_x, min_y, max_x, max_y]` do quad.
    #[must_use]
    pub fn bounds(&self) -> Vec4 {
        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for [x, y] in self.quad {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
        Vec4::new(min_x, min_y, max_x, max_y)
    }

    /// Verifica se o quad é estritamente convexo e orientado no sentido horário (UI +X direita, +Y baixo).
    #[must_use]
    pub fn is_valid_convex_cw(&self) -> bool {
        for i in 0..4 {
            let a = self.quad[i];
            let b = self.quad[(i + 1) % 4];
            let c = self.quad[(i + 2) % 4];
            let abx = b[0] - a[0];
            let aby = b[1] - a[1];
            let bcx = c[0] - b[0];
            let bcy = c[1] - b[1];
            // Em espaço UI (+X direita, +Y baixo), viragem à direita (horária) tem cross > 0.
            let cross = abx * bcy - aby * bcx;
            if cross <= 1e-5 {
                return false;
            }
        }
        true
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MangaBubble {
    #[serde(default)]
    pub kind: BubbleKind,
    #[serde(default)]
    pub beat: usize,
    #[serde(default)]
    pub delay: f32,
    /// Canto superior esquerdo da bolha em coordenadas normalizadas da página `[0..1]`.
    pub pos: [f32; 2],
    #[serde(default = "default_bubble_width")]
    pub width: f32,
    /// Alvo da cauda em coordenadas normalizadas da página `[0..1]` (só `speech`).
    #[serde(default)]
    pub tail: Option<[f32; 2]>,
    #[serde(default)]
    pub speaker: Option<String>,
    pub text: String,
    #[serde(default)]
    pub angle_deg: f32,
    #[serde(default = "default_sfx_size")]
    pub size: f32,
    #[serde(default)]
    pub accent: AccentTone,
    #[serde(default)]
    pub sfx: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MangaPage {
    pub id: String,
    #[serde(default)]
    pub paper: PaperStyle,
    #[serde(default)]
    pub title_card: Option<TitleCardSpec>,
    pub panels: Vec<MangaPanel>,
    #[serde(default)]
    pub bubbles: Vec<MangaBubble>,
    /// Duração de compassos extra (opcional) quando uma bolha fica num beat sem quadro novo.
    #[serde(default)]
    pub beat_durations: HashMap<usize, f32>,
}

impl MangaPage {
    /// Maior índice de `beat` presente na página (quadros, bolhas ou título).
    #[must_use]
    pub fn max_beat(&self) -> usize {
        let p = self.panels.iter().map(|p| p.beat).max().unwrap_or(0);
        let b = self.bubbles.iter().map(|b| b.beat).max().unwrap_or(0);
        let t = self.title_card.as_ref().map_or(0, |t| t.beat);
        p.max(b).max(t)
    }

    /// Duração de auto-avanço (s) para um dado `beat`.
    #[must_use]
    pub fn beat_duration(&self, beat: usize) -> f32 {
        if let Some(&explicit) = self.beat_durations.get(&beat) {
            return explicit.max(0.8);
        }
        let panel_dur = self
            .panels
            .iter()
            .filter(|p| p.beat == beat)
            .map(|p| p.duration)
            .fold(0.0f32, f32::max);
        let bubble_dur = self
            .bubbles
            .iter()
            .filter(|b| b.beat == beat)
            .map(|b| b.delay + 1.8 + (b.text.chars().count() as f32 * 0.042))
            .fold(0.0f32, f32::max);
        panel_dur.max(bubble_dur).max(2.6)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MangaCutscene {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub subtitle: String,
    #[serde(default = "default_true")]
    pub skippable: bool,
    #[serde(default = "default_true")]
    pub auto_advance: bool,
    pub pages: Vec<MangaPage>,
}

impl MangaCutscene {
    /// Valida estrutura básica (id, ≥1 página, quads convexos horários).
    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty() {
            return Err("cutscene sem id".into());
        }
        if self.pages.is_empty() {
            return Err(format!("cutscene `{}` sem páginas", self.id));
        }
        for (pi, page) in self.pages.iter().enumerate() {
            if page.panels.is_empty() {
                return Err(format!(
                    "cutscene `{}` página #{pi} (`{}`) sem quadros",
                    self.id, page.id
                ));
            }
            for panel in &page.panels {
                if !panel.is_valid_convex_cw() {
                    return Err(format!(
                        "cutscene `{}` quadro `{}`: quad inválido (precisa de 4 vértices convexos horários)",
                        self.id, panel.id
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Carrega todas as cinemáticas de `dir/*.json`.
pub fn load_cutscenes_from_dir(dir: &Path) -> HashMap<String, MangaCutscene> {
    let mut out = HashMap::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    for path in paths {
        let Ok(raw) = fs::read_to_string(&path) else {
            warn!("manga_cinematic: falha a ler {}", path.display());
            continue;
        };
        match serde_json::from_str::<MangaCutscene>(&raw) {
            Ok(spec) => match spec.validate() {
                Ok(()) => {
                    out.insert(spec.id.clone(), spec);
                }
                Err(err) => warn!("manga_cinematic: {}: {err}", path.display()),
            },
            Err(err) => warn!(
                "manga_cinematic: JSON inválido em {}: {err}",
                path.display()
            ),
        }
    }
    out
}

// ── Materiais UI (WESL) ─────────────────────────────────────────────────

#[derive(ShaderType, Clone, Debug, Default)]
pub struct MangaPanelUniform {
    pub p01: Vec4,
    pub p23: Vec4,
    pub bounds: Vec4,
    pub uv_ctrl: Vec4,
    pub fx: Vec4,
    pub fx2: Vec4,
    pub accent: Vec4,
    pub ink: Vec4,
}

#[derive(AsBindGroup, Asset, TypePath, Clone, Debug)]
pub struct MangaPanelMaterial {
    #[uniform(0)]
    pub data: MangaPanelUniform,
    #[texture(1)]
    #[sampler(2)]
    pub art_texture: Handle<Image>,
}

impl UiMaterial for MangaPanelMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://vael/manga_panel.wesl".into()
    }
}

#[derive(ShaderType, Clone, Debug, Default)]
pub struct MangaBubbleUniform {
    pub params: Vec4,
    pub tail: Vec4,
    pub fill: Vec4,
    pub ink: Vec4,
    pub accent: Vec4,
}

#[derive(AsBindGroup, Asset, TypePath, Clone, Debug)]
pub struct MangaBubbleMaterial {
    #[uniform(0)]
    pub data: MangaBubbleUniform,
}

impl UiMaterial for MangaBubbleMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://vael/manga_bubble.wesl".into()
    }
}

// ── Recursos & Estado de Reprodução ─────────────────────────────────────

#[derive(Resource, Debug, Default, Clone)]
pub struct MangaCutsceneLibrary {
    pub specs: HashMap<String, MangaCutscene>,
    pub dir: PathBuf,
    mtimes: HashMap<PathBuf, SystemTime>,
    reload_timer: f32,
}

impl MangaCutsceneLibrary {
    pub fn load(dir: PathBuf) -> Self {
        let specs = load_cutscenes_from_dir(&dir);
        let mtimes = collect_mtimes(&dir);
        Self {
            specs,
            dir,
            mtimes,
            reload_timer: 0.0,
        }
    }

    pub fn contains(&self, id: &str) -> bool {
        self.specs.contains_key(id)
    }

    pub fn ids(&self) -> HashSet<String> {
        self.specs.keys().cloned().collect()
    }
}

fn collect_mtimes(dir: &Path) -> HashMap<PathBuf, SystemTime> {
    let mut map = HashMap::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return map;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "json")
            && let Ok(meta) = entry.metadata()
            && let Ok(mtime) = meta.modified()
        {
            map.insert(path, mtime);
        }
    }
    map
}

#[derive(Debug, Clone)]
pub struct MangaActive {
    pub spec: MangaCutscene,
    pub page_idx: usize,
    pub beat: usize,
    pub total_elapsed: f32,
    pub page_elapsed: f32,
    pub beat_elapsed: f32,
    /// Quando o jogador recuou manualmente, segura o auto-avanço até novo clique/tecla.
    pub manual_hold: bool,
    pub skipped: bool,
    pub spawned_page: Option<usize>,
    pub fired_panel_sfx: HashSet<usize>,
    pub fired_bubble_sfx: HashSet<usize>,
    pub hidden_hud: Vec<(Entity, Visibility)>,
}

impl MangaActive {
    pub fn new(spec: MangaCutscene, hidden_hud: Vec<(Entity, Visibility)>) -> Self {
        Self {
            spec,
            page_idx: 0,
            beat: 0,
            total_elapsed: 0.0,
            page_elapsed: 0.0,
            beat_elapsed: 0.0,
            manual_hold: false,
            skipped: false,
            spawned_page: None,
            fired_panel_sfx: HashSet::new(),
            fired_bubble_sfx: HashSet::new(),
            hidden_hud,
        }
    }

    pub fn current_page(&self) -> Option<&MangaPage> {
        self.spec.pages.get(self.page_idx)
    }

    /// Avança um passo: se a animação de entrada do compasso ainda corre, assenta-a;
    /// caso contrário passa ao compasso/página seguinte. Devolve `true` se terminou.
    pub fn advance(&mut self) -> bool {
        self.manual_hold = false;
        let Some(page) = self.spec.pages.get(self.page_idx) else {
            return true;
        };
        let settle_t = self.beat_settle_time(page, self.beat);
        if self.beat_elapsed + 0.02 < settle_t {
            self.beat_elapsed = settle_t;
            return false;
        }
        let max_beat = page.max_beat();
        if self.beat < max_beat {
            self.beat += 1;
            self.beat_elapsed = 0.0;
            return false;
        }
        if self.page_idx + 1 < self.spec.pages.len() {
            self.page_idx += 1;
            self.beat = 0;
            self.page_elapsed = 0.0;
            self.beat_elapsed = 0.0;
            self.fired_panel_sfx.clear();
            self.fired_bubble_sfx.clear();
            return false;
        }
        true
    }

    /// Recua um compasso (ou para o último compasso da página anterior).
    pub fn step_back(&mut self) {
        self.manual_hold = true;
        if self.beat > 0 {
            self.beat -= 1;
            self.beat_elapsed = 0.6;
        } else if self.page_idx > 0 {
            self.page_idx -= 1;
            let last_beat = self
                .spec
                .pages
                .get(self.page_idx)
                .map_or(0, MangaPage::max_beat);
            self.beat = last_beat;
            self.page_elapsed = PAGE_FADE_SECS;
            self.beat_elapsed = 0.6;
            self.fired_panel_sfx.clear();
            self.fired_bubble_sfx.clear();
        }
    }

    fn beat_settle_time(&self, page: &MangaPage, beat: usize) -> f32 {
        let mut t = PANEL_REVEAL_SECS;
        for b in page.bubbles.iter().filter(|b| b.beat == beat) {
            t = t.max(b.delay + BUBBLE_POP_SECS);
        }
        t
    }
}

#[derive(Resource, Debug, Default)]
pub struct MangaPlayback {
    pub active: Option<MangaActive>,
}

impl MangaPlayback {
    #[must_use]
    pub fn playing(&self) -> Option<&str> {
        self.active.as_ref().map(|a| a.spec.id.as_str())
    }
}

/// Recursos de tipografia e placeholder 1×1 branco carregados uma vez.
#[derive(Resource, Clone)]
struct MangaArtAssets {
    font_bold: Handle<Font>,
    font_regular: Handle<Font>,
    fallback_tex: Handle<Image>,
}

// ── Marcadores ECS do Overlay ───────────────────────────────────────────

#[derive(Component)]
struct MangaOverlayRoot;

#[derive(Component)]
struct MangaPageStage;

#[derive(Component)]
struct MangaPanelView {
    panel_idx: usize,
    material: Handle<MangaPanelMaterial>,
    image: Handle<Image>,
}

#[derive(Component)]
struct MangaBubbleView {
    bubble_idx: usize,
    material: Option<Handle<MangaBubbleMaterial>>,
    base_angle_rad: f32,
}

#[derive(Component)]
struct MangaTitleView {
    beat: usize,
}

#[derive(Component)]
struct MangaProgressFill;

#[derive(Component)]
struct MangaPageDots;

#[derive(Component)]
struct MangaPageFadeVeil;

type HudRoots<'w, 's> = Query<
    'w,
    's,
    (Entity, &'static mut Visibility),
    (With<Node>, Without<ChildOf>, Without<MangaOverlayRoot>),
>;

// ── Plugin ──────────────────────────────────────────────────────────────

pub struct VaelMangaCinematicPlugin;

impl Plugin for VaelMangaCinematicPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "manga_panel.wesl");
        embedded_asset!(app, "manga_bubble.wesl");

        let library = if manga_enabled() {
            MangaCutsceneLibrary::load(PathBuf::from(CUTSCENES_DIR))
        } else {
            MangaCutsceneLibrary::default()
        };

        app.add_plugins((
            UiMaterialPlugin::<MangaPanelMaterial>::default(),
            UiMaterialPlugin::<MangaBubbleMaterial>::default(),
        ))
        .insert_resource(library)
        .init_resource::<MangaPlayback>()
        .add_systems(Startup, (setup_manga_assets, sync_external_ids))
        .add_systems(
            Update,
            (
                hot_reload_cutscenes,
                start_manga_cinematic,
                tick_manga_cinematic,
                rebuild_manga_page,
                drive_manga_visuals,
            )
                .chain(),
        );
    }
}

fn setup_manga_assets(
    mut commands: Commands,
    asset_server: Option<Res<AssetServer>>,
    mut images: Option<ResMut<Assets<Image>>>,
) {
    let (font_bold, font_regular) = match asset_server {
        Some(server) => (
            server.load("ui/fonts/NotoSerifDisplay-Bold.ttf"),
            server.load("ui/fonts/NotoSerifDisplay-Regular.ttf"),
        ),
        None => (Handle::default(), Handle::default()),
    };
    let fallback_tex = images
        .as_mut()
        .map(|imgs| {
            use bevy::asset::RenderAssetUsages;
            use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
            imgs.add(Image::new_fill(
                Extent3d {
                    width: 2,
                    height: 2,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                &[232, 222, 200, 255],
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::RENDER_WORLD,
            ))
        })
        .unwrap_or_default();
    commands.insert_resource(MangaArtAssets {
        font_bold,
        font_regular,
        fallback_tex,
    });
}

fn sync_external_ids(
    library: Res<MangaCutsceneLibrary>,
    mut external_ids: Option<ResMut<ExternalCinematicIds>>,
) {
    if let Some(ext) = external_ids.as_deref_mut() {
        ext.0 = library.ids();
    }
}

fn hot_reload_cutscenes(
    real: Res<Time<Real>>,
    mut library: ResMut<MangaCutsceneLibrary>,
    mut external_ids: Option<ResMut<ExternalCinematicIds>>,
) {
    if !manga_enabled() || library.dir.as_os_str().is_empty() {
        return;
    }
    library.reload_timer += real.delta_secs();
    if library.reload_timer < HOT_RELOAD_INTERVAL {
        return;
    }
    library.reload_timer = 0.0;
    let now = collect_mtimes(&library.dir);
    if now != library.mtimes {
        library.mtimes = now;
        library.specs = load_cutscenes_from_dir(&library.dir);
        if let Some(ext) = external_ids.as_deref_mut() {
            ext.0 = library.ids();
        }
        info!(
            "manga_cinematic: recarregadas {} pranchas de {}",
            library.specs.len(),
            library.dir.display()
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn start_manga_cinematic(
    mut requests: MessageReader<PlayCinematic>,
    library: Res<MangaCutsceneLibrary>,
    mut manga: ResMut<MangaPlayback>,
    mut engine_playback: Option<ResMut<CinematicPlayback>>,
    mut menus: Option<ResMut<MenusOpen>>,
    mut ui_nodes: HudRoots<'_, '_>,
    mut ended: MessageWriter<CinematicEnded>,
    mut events: Option<ResMut<ScriptEventQueue>>,
) {
    if !manga_enabled() {
        return;
    }
    let Some(req) = requests.read().last().cloned() else {
        return;
    };
    if req.inline.is_some() {
        return;
    }
    let Some(spec) = library.specs.get(&req.id).cloned() else {
        return;
    };
    if let Some(prev) = manga.active.take() {
        restore_hud(&mut ui_nodes, &prev.hidden_hud);
        emit_end(prev.spec.id, true, &mut ended, events.as_deref_mut());
    }
    // Se a engine tinha algo 3D activo, limpa-o.
    if let Some(pb) = engine_playback.as_deref_mut() {
        pb.active = None;
        pb.external = Some(spec.id.clone());
    }
    if let Some(m) = menus.as_deref_mut() {
        m.cinematic = true;
    }
    let mut hidden_hud = Vec::new();
    for (entity, mut vis) in &mut ui_nodes {
        hidden_hud.push((entity, *vis));
        *vis = Visibility::Hidden;
    }
    info!(
        "manga_cinematic: ▶ {} ({} páginas)",
        spec.id,
        spec.pages.len()
    );
    manga.active = Some(MangaActive::new(spec, hidden_hud));
}

#[allow(clippy::too_many_arguments)]
fn tick_manga_cinematic(
    real: Res<Time<Real>>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mouse: Option<Res<ButtonInput<MouseButton>>>,
    mut stops: MessageReader<StopCinematic>,
    mut manga: ResMut<MangaPlayback>,
    mut engine_playback: Option<ResMut<CinematicPlayback>>,
    mut menus: Option<ResMut<MenusOpen>>,
    mut sfx: MessageWriter<SfxEvent>,
    mut ended: MessageWriter<CinematicEnded>,
    mut events: Option<ResMut<ScriptEventQueue>>,
    mut ui_nodes: HudRoots<'_, '_>,
    mut commands: Commands,
    roots: Query<Entity, With<MangaOverlayRoot>>,
) {
    let stop_requested = stops.read().count() > 0;
    let dt = real.delta_secs().clamp(0.0, 0.1);

    let Some(active) = manga.active.as_mut() else {
        // Garante que o overlay é desmontado quando não há prancha activa.
        for e in &roots {
            commands.entity(e).despawn();
        }
        return;
    };

    active.total_elapsed += dt;
    active.page_elapsed += dt;
    active.beat_elapsed += dt;

    // Dispara SFX dos quadros e bolhas do compasso actual.
    if let Some(page) = active.spec.pages.get(active.page_idx) {
        for (idx, panel) in page.panels.iter().enumerate() {
            if panel.beat == active.beat
                && !active.fired_panel_sfx.contains(&idx)
                && let Some(name) = panel.sfx.as_deref()
            {
                active.fired_panel_sfx.insert(idx);
                if let Some(clip) = sfx_clip_from_str(name) {
                    sfx.write(SfxEvent {
                        clip,
                        position: None,
                    });
                }
            }
        }
        for (idx, bubble) in page.bubbles.iter().enumerate() {
            if bubble.beat == active.beat
                && active.beat_elapsed >= bubble.delay
                && !active.fired_bubble_sfx.contains(&idx)
                && let Some(name) = bubble.sfx.as_deref()
            {
                active.fired_bubble_sfx.insert(idx);
                if let Some(clip) = sfx_clip_from_str(name) {
                    sfx.write(SfxEvent {
                        clip,
                        position: None,
                    });
                }
            }
        }
    }

    let skip_key = keys
        .as_deref()
        .is_some_and(|k| k.just_pressed(KeyCode::Escape));
    let advance_key = keys
        .as_deref()
        .is_some_and(|k| k.any_just_pressed([KeyCode::Space, KeyCode::Enter, KeyCode::ArrowRight]))
        || mouse
            .as_deref()
            .is_some_and(|m| m.just_pressed(MouseButton::Left));
    let back_key = keys
        .as_deref()
        .is_some_and(|k| k.any_just_pressed([KeyCode::ArrowLeft, KeyCode::Backspace]));

    let mut finish = false;
    if stop_requested
        || (active.spec.skippable && active.total_elapsed > SKIP_GUARD_SECS && skip_key)
    {
        active.skipped = true;
        finish = true;
    } else if back_key && active.total_elapsed > 0.15 {
        active.step_back();
    } else if advance_key && active.total_elapsed > 0.15 {
        finish = active.advance();
    } else if active.spec.auto_advance && !active.manual_hold {
        let dur = active
            .current_page()
            .map_or(3.0, |p| p.beat_duration(active.beat));
        if active.beat_elapsed >= dur {
            finish = active.advance();
        }
    }

    if !finish {
        return;
    }

    let done = manga.active.take().expect("activa acima");
    if let Some(pb) = engine_playback.as_deref_mut()
        && pb.external.as_deref() == Some(done.spec.id.as_str())
    {
        pb.external = None;
    }
    if let Some(m) = menus.as_deref_mut() {
        m.cinematic = false;
    }
    restore_hud(&mut ui_nodes, &done.hidden_hud);
    for e in &roots {
        commands.entity(e).despawn();
    }
    info!(
        "manga_cinematic: ■ {}{}",
        done.spec.id,
        if done.skipped { " (saltada)" } else { "" }
    );
    emit_end(
        done.spec.id,
        done.skipped,
        &mut ended,
        events.as_deref_mut(),
    );
}

fn restore_hud(ui_nodes: &mut HudRoots<'_, '_>, hidden: &[(Entity, Visibility)]) {
    for (entity, previous) in hidden {
        if let Ok((_, mut vis)) = ui_nodes.get_mut(*entity) {
            *vis = *previous;
        }
    }
}

fn emit_end(
    id: String,
    skipped: bool,
    ended: &mut MessageWriter<CinematicEnded>,
    events: Option<&mut ScriptEventQueue>,
) {
    if let Some(q) = events {
        q.push(ScriptGameEvent::CinematicEnd {
            id: id.clone(),
            skipped,
        });
    }
    ended.write(CinematicEnded { id, skipped });
}

#[allow(clippy::too_many_arguments)]
fn rebuild_manga_page(
    mut commands: Commands,
    mut manga: ResMut<MangaPlayback>,
    art: Option<Res<MangaArtAssets>>,
    asset_server: Option<Res<AssetServer>>,
    mut panel_mats: Option<ResMut<Assets<MangaPanelMaterial>>>,
    mut bubble_mats: Option<ResMut<Assets<MangaBubbleMaterial>>>,
    a11y: Option<Res<A11y>>,
    roots: Query<Entity, With<MangaOverlayRoot>>,
) {
    let Some(active) = manga.active.as_mut() else {
        return;
    };
    if active.spawned_page == Some(active.page_idx) {
        return;
    }
    let (Some(art), Some(panel_mats), Some(bubble_mats)) =
        (art, panel_mats.as_mut(), bubble_mats.as_mut())
    else {
        return;
    };
    let Some(page) = active.spec.pages.get(active.page_idx).cloned() else {
        return;
    };

    for e in &roots {
        commands.entity(e).despawn();
    }
    active.spawned_page = Some(active.page_idx);

    let text_scale = a11y
        .as_deref()
        .map_or(1.0, |a| a.subtitle_scale.clamp(0.85, 1.5));
    let mk_font = |font: &Handle<Font>, size: f32| TextFont {
        font: font.clone().into(),
        font_size: (size * text_scale).into(),
        ..Default::default()
    };
    let total_pages = active.spec.pages.len();
    let current_page_idx = active.page_idx;

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                bottom: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..Default::default()
            },
            BackgroundColor(page.paper.desk_color()),
            GlobalZIndex(Z_MANGA),
            Name::new("manga_cinematic:overlay"),
            MangaOverlayRoot,
        ))
        .with_children(|root| {
            // Lienzo 16:9 fixo da prancha — mantém todos os cortes diagonais fiéis em qualquer ecrã.
            root.spawn((
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    max_width: Val::Vh(177.78),
                    max_height: Val::Vw(56.25),
                    aspect_ratio: Some(16.0 / 9.0),
                    overflow: Overflow::clip(),
                    ..Default::default()
                },
                BackgroundColor(page.paper.page_color()),
                Name::new("manga_cinematic:page"),
                MangaPageStage,
            ))
            .with_children(|stage| {
                // 1. Quadros poligonais (cada um no espaço completo da página para cortes diagonais).
                for (idx, panel) in page.panels.iter().enumerate() {
                    let img: Handle<Image> = match &asset_server {
                        Some(srv) if !panel.image.trim().is_empty() => {
                            srv.load(panel.image.clone())
                        }
                        _ => art.fallback_tex.clone(),
                    };
                    let bounds = panel.bounds();
                    let mat = panel_mats.add(MangaPanelMaterial {
                        data: MangaPanelUniform {
                            p01: Vec4::new(
                                panel.quad[0][0],
                                panel.quad[0][1],
                                panel.quad[1][0],
                                panel.quad[1][1],
                            ),
                            p23: Vec4::new(
                                panel.quad[2][0],
                                panel.quad[2][1],
                                panel.quad[3][0],
                                panel.quad[3][1],
                            ),
                            bounds,
                            uv_ctrl: Vec4::new(
                                panel.pan_from[0],
                                panel.pan_from[1],
                                panel.zoom_from,
                                1.0,
                            ),
                            fx: Vec4::new(0.0, panel.reveal.as_f32(), panel.border_px, 0.0),
                            fx2: Vec4::new(panel.speedlines, 0.0, 0.0, 0.0),
                            accent: panel.accent.vec4(),
                            ink: page.paper.ink_color(),
                        },
                        art_texture: img.clone(),
                    });
                    stage.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(0.0),
                            right: Val::Px(0.0),
                            top: Val::Px(0.0),
                            bottom: Val::Px(0.0),
                            ..Default::default()
                        },
                        MaterialNode(mat.clone()),
                        MangaPanelView {
                            panel_idx: idx,
                            material: mat,
                            image: img,
                        },
                    ));
                }

                // 2. Cartão de título opcional da página (ex.: "O Silêncio de Vael").
                if let Some(tc) = &page.title_card {
                    stage
                        .spawn((
                            Node {
                                position_type: PositionType::Absolute,
                                left: Val::Percent(tc.pos[0] * 100.0),
                                top: Val::Percent(tc.pos[1] * 100.0),
                                padding: UiRect::axes(Val::Px(20.0), Val::Px(12.0)),
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(2.0),
                                border: UiRect::all(Val::Px(3.0)),
                                ..Default::default()
                            },
                            BackgroundColor(Color::srgba(0.075, 0.070, 0.064, 0.92)),
                            BorderColor::all(Color::srgba(0.92, 0.87, 0.76, 0.9)),
                            Visibility::Hidden,
                            MangaTitleView { beat: tc.beat },
                        ))
                        .with_children(|card| {
                            if !tc.sub.is_empty() {
                                card.spawn((
                                    Text::new(tc.sub.clone()),
                                    mk_font(&art.font_regular, 14.0),
                                    TextColor(Color::srgb(0.88, 0.72, 0.44)),
                                ));
                            }
                            card.spawn((
                                Text::new(tc.text.clone()),
                                mk_font(&art.font_bold, 30.0),
                                TextColor(Color::srgb(0.96, 0.93, 0.86)),
                            ));
                        });
                }

                // 3. Bolhas de diálogo, narração e onomatopeias (`sfx`).
                for (idx, bubble) in page.bubbles.iter().enumerate() {
                    let angle_rad = bubble.angle_deg.to_radians();
                    if bubble.kind == BubbleKind::Sfx {
                        let accent = if bubble.accent == AccentTone::None {
                            Color::srgb(0.96, 0.92, 0.84)
                        } else {
                            bubble.accent.color()
                        };
                        stage
                            .spawn((
                                Node {
                                    position_type: PositionType::Absolute,
                                    left: Val::Percent(bubble.pos[0] * 100.0),
                                    top: Val::Percent(bubble.pos[1] * 100.0),
                                    ..Default::default()
                                },
                                UiTransform::from_rotation(Rot2::radians(angle_rad)),
                                Visibility::Hidden,
                                MangaBubbleView {
                                    bubble_idx: idx,
                                    material: None,
                                    base_angle_rad: angle_rad,
                                },
                            ))
                            .with_children(|sfx_node| {
                                // Contorno/sombra deslocada a tinta preta para leitura sobre qualquer arte.
                                for (ox, oy) in [(-2.5, -2.5), (3.0, 3.0), (2.5, -2.0), (-2.0, 2.5)]
                                {
                                    sfx_node.spawn((
                                        Node {
                                            position_type: PositionType::Absolute,
                                            left: Val::Px(ox),
                                            top: Val::Px(oy),
                                            ..Default::default()
                                        },
                                        Text::new(bubble.text.clone()),
                                        mk_font(&art.font_bold, bubble.size),
                                        TextColor(Color::srgb(0.06, 0.055, 0.05)),
                                    ));
                                }
                                sfx_node.spawn((
                                    Text::new(bubble.text.clone()),
                                    mk_font(&art.font_bold, bubble.size),
                                    TextColor(accent),
                                ));
                            });
                        continue;
                    }

                    let (pad_x, pad_y, inset_px, border_px, fill, text_col) = match bubble.kind {
                        BubbleKind::Speech => (
                            24.0,
                            18.0,
                            24.0,
                            3.2,
                            Vec4::new(0.972, 0.949, 0.898, 0.98),
                            Color::srgb(0.09, 0.08, 0.07),
                        ),
                        BubbleKind::Shout => (
                            30.0,
                            24.0,
                            28.0,
                            3.8,
                            Vec4::new(0.985, 0.968, 0.915, 0.99),
                            Color::srgb(0.07, 0.06, 0.055),
                        ),
                        BubbleKind::Whisper => (
                            20.0,
                            14.0,
                            14.0,
                            2.4,
                            Vec4::new(0.935, 0.905, 0.840, 0.94),
                            Color::srgb(0.20, 0.18, 0.16),
                        ),
                        BubbleKind::Narration => (
                            20.0,
                            13.0,
                            10.0,
                            2.8,
                            Vec4::new(0.938, 0.898, 0.808, 0.96),
                            Color::srgb(0.11, 0.10, 0.085),
                        ),
                        BubbleKind::Sfx => unreachable!(),
                    };

                    // Direcção da cauda a partir do centro aproximado da bolha até `tail`.
                    let (has_tail, tail_dir) = match bubble.tail {
                        Some([tx, ty]) if bubble.kind == BubbleKind::Speech => {
                            let cx = bubble.pos[0] + bubble.width * 0.5;
                            let cy = bubble.pos[1] + 0.075;
                            let d = Vec2::new((tx - cx) * (16.0 / 9.0), ty - cy);
                            if d.length_squared() > 1e-5 {
                                (1.0, d.normalize())
                            } else {
                                (0.0, Vec2::Y)
                            }
                        }
                        _ => (0.0, Vec2::Y),
                    };

                    let bmat = bubble_mats.add(MangaBubbleMaterial {
                        data: MangaBubbleUniform {
                            params: Vec4::new(bubble.kind.shader_kind(), border_px, inset_px, 0.0),
                            tail: Vec4::new(has_tail, tail_dir.x, tail_dir.y, inset_px * 0.92),
                            fill,
                            ink: Vec4::new(0.075, 0.068, 0.062, 1.0),
                            accent: bubble.accent.vec4(),
                        },
                    });

                    stage
                        .spawn((
                            Node {
                                position_type: PositionType::Absolute,
                                left: Val::Percent(bubble.pos[0] * 100.0),
                                top: Val::Percent(bubble.pos[1] * 100.0),
                                width: Val::Percent(bubble.width * 100.0),
                                padding: UiRect::axes(Val::Px(pad_x), Val::Px(pad_y)),
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(3.0),
                                ..Default::default()
                            },
                            UiTransform::from_rotation(Rot2::radians(angle_rad)),
                            Visibility::Hidden,
                            MangaBubbleView {
                                bubble_idx: idx,
                                material: Some(bmat.clone()),
                                base_angle_rad: angle_rad,
                            },
                        ))
                        .with_children(|bnode| {
                            // Fundo vetorial da bolha expandido em `inset_px` para a cauda/picos nunca cortarem o texto.
                            bnode.spawn((
                                Node {
                                    position_type: PositionType::Absolute,
                                    left: Val::Px(-inset_px),
                                    right: Val::Px(-inset_px),
                                    top: Val::Px(-inset_px),
                                    bottom: Val::Px(-inset_px),
                                    ..Default::default()
                                },
                                MaterialNode(bmat),
                            ));
                            if let Some(speaker) = bubble.speaker.as_deref()
                                && !speaker.trim().is_empty()
                            {
                                let sp_col = match bubble.accent {
                                    AccentTone::Amber => Color::srgb(0.62, 0.31, 0.09),
                                    AccentTone::Turquoise => Color::srgb(0.11, 0.42, 0.48),
                                    AccentTone::None => Color::srgb(0.36, 0.28, 0.19),
                                };
                                bnode.spawn((
                                    Text::new(speaker.to_uppercase()),
                                    mk_font(&art.font_bold, 11.5),
                                    TextColor(sp_col),
                                ));
                            }
                            let body_size = match bubble.kind {
                                BubbleKind::Shout => 18.0,
                                BubbleKind::Whisper => 14.5,
                                BubbleKind::Narration => 15.0,
                                BubbleKind::Speech | BubbleKind::Sfx => 15.5,
                            };
                            let font = if bubble.kind == BubbleKind::Shout {
                                &art.font_bold
                            } else {
                                &art.font_regular
                            };
                            bnode.spawn((
                                Text::new(bubble.text.clone()),
                                mk_font(font, body_size),
                                TextColor(text_col),
                            ));
                        });
                }

                // 4. Barra discreta de leitura no rodapé (pontos de página + controlos + progresso).
                stage
                    .spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Percent(3.0),
                            right: Val::Percent(3.0),
                            bottom: Val::Px(8.0),
                            justify_content: JustifyContent::SpaceBetween,
                            align_items: AlignItems::Center,
                            ..Default::default()
                        },
                        MangaPageDots,
                    ))
                    .with_children(|bar| {
                        let dots = (0..total_pages)
                            .map(|i| if i == current_page_idx { "●" } else { "○" })
                            .collect::<Vec<_>>()
                            .join(" ");
                        bar.spawn((
                            Text::new(format!(
                                "Prancha {}/{}  {dots}",
                                current_page_idx + 1,
                                total_pages
                            )),
                            mk_font(&art.font_bold, 12.5),
                            TextColor(page.paper.hud_text_color()),
                        ));
                        bar.spawn((
                            Text::new("[Espaço / Clique] Avançar  ·  [←] Voltar  ·  [Esc] Saltar"),
                            mk_font(&art.font_regular, 12.0),
                            TextColor(page.paper.hud_text_color()),
                        ));
                    });

                // Linha fina de progresso do compasso actual.
                stage
                    .spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(0.0),
                            right: Val::Px(0.0),
                            bottom: Val::Px(0.0),
                            height: Val::Px(3.0),
                            ..Default::default()
                        },
                        BackgroundColor(Color::srgba(0.08, 0.07, 0.06, 0.25)),
                    ))
                    .with_children(|track| {
                        track.spawn((
                            Node {
                                width: Val::Percent(0.0),
                                height: Val::Percent(100.0),
                                ..Default::default()
                            },
                            BackgroundColor(Color::srgba(0.84, 0.56, 0.24, 0.85)),
                            MangaProgressFill,
                        ));
                    });

                // Véu breve na viragem de página.
                stage.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(0.0),
                        right: Val::Px(0.0),
                        top: Val::Px(0.0),
                        bottom: Val::Px(0.0),
                        ..Default::default()
                    },
                    BackgroundColor(Color::srgba(0.05, 0.045, 0.04, 1.0)),
                    MangaPageFadeVeil,
                ));
            });
        });
}

#[allow(clippy::too_many_arguments)]
fn drive_manga_visuals(
    manga: Res<MangaPlayback>,
    a11y: Option<Res<A11y>>,
    images: Option<Res<Assets<Image>>>,
    mut panel_mats: Option<ResMut<Assets<MangaPanelMaterial>>>,
    mut bubble_mats: Option<ResMut<Assets<MangaBubbleMaterial>>>,
    panels: Query<&MangaPanelView>,
    mut bubbles: Query<(&MangaBubbleView, &mut Visibility, &mut UiTransform)>,
    mut titles: Query<(&MangaTitleView, &mut Visibility), Without<MangaBubbleView>>,
    mut progress: Query<&mut Node, With<MangaProgressFill>>,
    mut veil: Query<&mut BackgroundColor, With<MangaPageFadeVeil>>,
) {
    let Some(active) = manga.active.as_ref() else {
        return;
    };
    let Some(page) = active.current_page() else {
        return;
    };
    let allow_shake = a11y.as_deref().is_none_or(|a| a.shake);

    // 1. Véu de entrada da página.
    let fade_a = (1.0 - (active.page_elapsed / PAGE_FADE_SECS)).clamp(0.0, 1.0);
    for mut bg in &mut veil {
        bg.0 = Color::srgba(0.05, 0.045, 0.04, fade_a * fade_a);
    }

    // 2. Barra de progresso do compasso.
    let beat_dur = page.beat_duration(active.beat).max(0.1);
    let pct = if active.spec.auto_advance && !active.manual_hold {
        (active.beat_elapsed / beat_dur).clamp(0.0, 1.0) * 100.0
    } else {
        100.0
    };
    for mut node in &mut progress {
        node.width = Val::Percent(pct);
    }

    // 3. Quadros poligonais.
    if let Some(panel_mats) = panel_mats.as_mut() {
        for view in &panels {
            let Some(spec) = page.panels.get(view.panel_idx) else {
                continue;
            };
            let Some(mut mat) = panel_mats.get_mut(&view.material) else {
                continue;
            };

            let (reveal, local_t, dim) = if spec.beat > active.beat {
                (0.0, 0.0, 0.0)
            } else if spec.beat == active.beat {
                let r = ease_out_cubic((active.beat_elapsed / PANEL_REVEAL_SECS).clamp(0.0, 1.0));
                (r, active.beat_elapsed, 0.0)
            } else {
                // Quadro já revelado em compasso anterior: fica na página, levemente atenuado.
                (1.0, spec.duration, 0.22)
            };

            // Ken Burns suave dentro do corte diagonal.
            let kb = (local_t / spec.duration.max(0.5)).clamp(0.0, 1.0);
            let pan_x = spec.pan_from[0] + (spec.pan_to[0] - spec.pan_from[0]) * kb;
            let pan_y = spec.pan_from[1] + (spec.pan_to[1] - spec.pan_from[1]) * kb;
            let zoom = spec.zoom_from + (spec.zoom_to - spec.zoom_from) * kb;

            // Tremor breve no impacto (respeita A11y).
            let mut shake_off = Vec2::ZERO;
            if allow_shake && spec.beat == active.beat && spec.shake > 0.0 && local_t < 0.32 {
                let decay = (1.0 - local_t / 0.32).powi(2) * spec.shake * 0.012;
                shake_off = Vec2::new(
                    (local_t * 78.0).sin() * decay,
                    (local_t * 95.0).cos() * decay,
                );
            }

            let flash =
                if allow_shake && spec.beat == active.beat && spec.flash > 0.0 && local_t < 0.24 {
                    (1.0 - local_t / 0.24).powi(2) * spec.flash
                } else {
                    0.0
                };

            let (has_tex, tex_aspect) = images
                .as_deref()
                .and_then(|imgs| imgs.get(&view.image))
                .filter(|img| img.width() > 4 && img.height() > 4)
                .map(|img| (1.0, img.width() as f32 / img.height() as f32))
                .unwrap_or((0.0, 16.0 / 9.0));

            mat.data.p01 = Vec4::new(
                spec.quad[0][0] + shake_off.x,
                spec.quad[0][1] + shake_off.y,
                spec.quad[1][0] + shake_off.x,
                spec.quad[1][1] + shake_off.y,
            );
            mat.data.p23 = Vec4::new(
                spec.quad[2][0] + shake_off.x,
                spec.quad[2][1] + shake_off.y,
                spec.quad[3][0] + shake_off.x,
                spec.quad[3][1] + shake_off.y,
            );
            mat.data.uv_ctrl = Vec4::new(pan_x, pan_y, zoom, tex_aspect);
            mat.data.fx = Vec4::new(reveal, spec.reveal.as_f32(), spec.border_px, dim);
            mat.data.fx2 = Vec4::new(spec.speedlines, flash, has_tex, active.total_elapsed);
        }
    }

    // 4. Título da página.
    for (tv, mut vis) in &mut titles {
        let show = active.beat >= tv.beat;
        vis.set_if_neq(if show {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }

    // 5. Bolhas de diálogo e onomatopeias.
    for (view, mut vis, mut xform) in &mut bubbles {
        let Some(spec) = page.bubbles.get(view.bubble_idx) else {
            continue;
        };
        let (shown, pop) = if spec.beat > active.beat {
            (false, 0.0)
        } else if spec.beat == active.beat {
            let t = active.beat_elapsed - spec.delay;
            if t < 0.0 {
                (false, 0.0)
            } else {
                let k = (t / BUBBLE_POP_SECS).clamp(0.0, 1.0);
                (true, k)
            }
        } else {
            (true, 1.0)
        };

        vis.set_if_neq(if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        if !shown {
            continue;
        }

        // Pop com leve elástico (sem salto se A11y reduziu movimento).
        let scale = if allow_shake {
            ease_out_back(pop)
        } else {
            0.94 + 0.06 * ease_out_cubic(pop)
        };
        xform.scale = Vec2::splat(scale);
        xform.rotation = Rot2::radians(view.base_angle_rad);

        if let (Some(mat_h), Some(mats)) = (&view.material, bubble_mats.as_mut())
            && let Some(mut mat) = mats.get_mut(mat_h)
        {
            // Bolhas de compassos anteriores ficam ligeiramente discretas para o olho seguir a nova fala.
            let alpha = if spec.beat < active.beat {
                0.86
            } else {
                ease_out_cubic(pop)
            };
            mat.data.params.w = alpha;
        }
    }
}

fn ease_out_cubic(t: f32) -> f32 {
    let inv = 1.0 - t.clamp(0.0, 1.0);
    1.0 - inv * inv * inv
}

fn ease_out_back(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let c1 = 1.55;
    let c3 = c1 + 1.0;
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_cutscene() -> MangaCutscene {
        MangaCutscene {
            id: "teste".into(),
            title: "Teste".into(),
            subtitle: "Cap I".into(),
            skippable: true,
            auto_advance: true,
            pages: vec![
                MangaPage {
                    id: "p1".into(),
                    paper: PaperStyle::Parchment,
                    title_card: None,
                    panels: vec![
                        MangaPanel {
                            id: "a".into(),
                            image: "cutscenes/teste/a.png".into(),
                            quad: [[0.04, 0.05], [0.96, 0.05], [0.96, 0.44], [0.04, 0.52]],
                            beat: 0,
                            duration: 3.0,
                            reveal: RevealKind::SlashRight,
                            border_px: 5.0,
                            pan_from: [0.0, 0.0],
                            pan_to: [0.0, 0.0],
                            zoom_from: 1.0,
                            zoom_to: 1.05,
                            speedlines: 0.0,
                            shake: 0.0,
                            flash: 0.0,
                            sfx: None,
                            accent: AccentTone::Amber,
                            prompt: None,
                            width: None,
                            height: None,
                            seed: None,
                            reference: None,
                        },
                        MangaPanel {
                            id: "b".into(),
                            image: "cutscenes/teste/b.png".into(),
                            quad: [[0.04, 0.56], [0.52, 0.49], [0.43, 0.94], [0.04, 0.94]],
                            beat: 1,
                            duration: 3.0,
                            reveal: RevealKind::SlashDown,
                            border_px: 5.0,
                            pan_from: [0.0, 0.0],
                            pan_to: [0.0, 0.0],
                            zoom_from: 1.0,
                            zoom_to: 1.0,
                            speedlines: 0.5,
                            shake: 0.4,
                            flash: 0.3,
                            sfx: Some("crash".into()),
                            accent: AccentTone::Turquoise,
                            prompt: None,
                            width: None,
                            height: None,
                            seed: None,
                            reference: None,
                        },
                    ],
                    bubbles: vec![MangaBubble {
                        kind: BubbleKind::Speech,
                        beat: 1,
                        delay: 0.3,
                        pos: [0.1, 0.6],
                        width: 0.25,
                        tail: Some([0.3, 0.8]),
                        speaker: Some("Bram".into()),
                        text: "Lume aceso.".into(),
                        angle_deg: 0.0,
                        size: 30.0,
                        accent: AccentTone::Amber,
                        sfx: None,
                    }],
                    beat_durations: HashMap::new(),
                },
                MangaPage {
                    id: "p2".into(),
                    paper: PaperStyle::Night,
                    title_card: None,
                    panels: vec![MangaPanel {
                        id: "c".into(),
                        image: "cutscenes/teste/c.png".into(),
                        quad: [[0.05, 0.06], [0.95, 0.06], [0.95, 0.92], [0.05, 0.92]],
                        beat: 0,
                        duration: 3.0,
                        reveal: RevealKind::Fade,
                        border_px: 5.0,
                        pan_from: [0.0, 0.0],
                        pan_to: [0.0, 0.0],
                        zoom_from: 1.0,
                        zoom_to: 1.0,
                        speedlines: 0.0,
                        shake: 0.0,
                        flash: 0.0,
                        sfx: None,
                        accent: AccentTone::None,
                        prompt: None,
                        width: None,
                        height: None,
                        seed: None,
                        reference: None,
                    }],
                    bubbles: vec![],
                    beat_durations: HashMap::new(),
                },
            ],
        }
    }

    #[test]
    fn diagonal_quad_validation_accepts_cw_and_rejects_ccw() {
        let spec = sample_cutscene();
        assert!(spec.validate().is_ok());

        let mut bad = spec.clone();
        bad.pages[0].panels[0].quad.reverse();
        assert!(bad.validate().is_err());
    }

    #[test]
    fn advance_settles_first_then_moves_beats_and_pages() {
        let mut active = MangaActive::new(sample_cutscene(), vec![]);
        assert_eq!((active.page_idx, active.beat), (0, 0));

        // 1.º clique durante reveal assenta o compasso sem saltar.
        assert!(!active.advance());
        assert_eq!((active.page_idx, active.beat), (0, 0));
        assert!(active.beat_elapsed >= PANEL_REVEAL_SECS);

        // 2.º clique passa ao beat 1 da página 0.
        assert!(!active.advance());
        assert_eq!((active.page_idx, active.beat), (0, 1));

        // Assenta o beat 1 (com bolha), depois muda para página 1.
        assert!(!active.advance());
        assert!(!active.advance());
        assert_eq!((active.page_idx, active.beat), (1, 0));

        // Recuar volta à página 0, último compasso.
        active.step_back();
        assert_eq!((active.page_idx, active.beat), (0, 1));
        assert!(active.manual_hold);

        // Avançar até terminar.
        assert!(!active.advance());
        assert_eq!((active.page_idx, active.beat), (1, 0));
        assert!(!active.advance());
        assert!(active.advance());
    }

    #[test]
    fn prologo_json_on_disk_parses_and_validates() {
        let path = Path::new(CUTSCENES_DIR).join("prologo.json");
        if !path.exists() {
            return;
        }
        let raw = fs::read_to_string(&path).expect("ler prologo.json");
        let spec: MangaCutscene = serde_json::from_str(&raw).expect("parse prologo.json");
        spec.validate().expect("prologo.json válido");
        assert_eq!(spec.id, "prologo");
        assert!(spec.pages.len() >= 3);
    }
}

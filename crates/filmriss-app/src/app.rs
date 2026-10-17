//! Main app: state, interaction logic and layout.
//!
//! Layout (desktop / tablet landscape):
//! ┌──────────────────────────────┬───────────┐
//! │ Top bar                      │ Side      │
//! ├──────────────────────────────┤ panel     │
//! │ Live preview (GPU)           │ (Look,    │
//! │  ← swipe → simulation        │  Recipe,  │
//! │  hold = original             │  Camera,  │
//! ├──────────────────────────────┤  Help)    │
//! │ Film strip  ·  shutter       │           │
//! └──────────────────────────────┴───────────┘
//! In narrow windows (phone portrait) the side panel moves to the bottom.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use anyhow::Context as _;
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Key, Margin, Rect, RichText, ScrollArea, Sense, Stroke, StrokeKind,
    Ui, Vec2,
};
use filmriss_camera::{CameraDescriptor, CameraId, CameraStream, Notify, StreamState};
use filmriss_core::{FilmRecipe, Library, Origin, Paths};

use crate::capture::{self, CaptureJob, CaptureResult};
use crate::preview::{PreviewCallback, PreviewFrame, PreviewResources, Uniforms};
use crate::style::{self, ACCENT, BG, TEXT_DIM};
use crate::thumbnails::Thumbnails;

const PERMISSION_PENDING: u8 = 0;
const PERMISSION_GRANTED: u8 = 1;
const PERMISSION_DENIED: u8 = 2;

/// Swipe distance (points) after which a swipe switches the simulation.
const SWIPE_THRESHOLD: f32 = 60.0;
/// How long (s) the preview must be pressed to show the original.
const HOLD_TO_COMPARE: f64 = 0.25;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Settings {
    simulation: String,
    intensity: f32,
    mirror: bool,
    camera: Option<String>,
    show_info: bool,
    save_original: bool,
    panel_open: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            simulation: "Standard".into(),
            intensity: 1.0,
            mirror: true,
            camera: None,
            show_info: false,
            save_original: false,
            panel_open: true,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Look,
    Recipe,
    Camera,
    Help,
}

struct Toast {
    text: String,
    until: f64,
    error: bool,
}

/// Counts events within the last second (for FPS displays).
#[derive(Default)]
struct Rate(VecDeque<f64>);

impl Rate {
    fn tick(&mut self, now: f64) {
        self.0.push_back(now);
        while self.0.front().is_some_and(|&t| t < now - 1.0) {
            self.0.pop_front();
        }
    }
    fn per_sec(&self) -> usize {
        self.0.len()
    }
}

pub struct FilmrissApp {
    settings: Settings,
    paths: Option<Paths>,
    library: Library,
    /// Recipes as last loaded – for the "modified" marker and "Reset".
    pristine: Vec<FilmRecipe>,
    current: usize,
    tab: Tab,
    save_name: String,
    confirm_delete: bool,

    permission: Arc<AtomicU8>,
    camera_started: bool,
    cameras: Vec<CameraDescriptor>,
    stream: Option<CameraStream>,
    reported_failure: bool,
    frame: Option<PreviewFrame>,
    next_frame_id: u64,
    cam_rate: Rate,
    ui_rate: Rate,
    gpu_info: String,

    thumbs: Thumbnails,
    scroll_to_current: bool,
    switched_at: f64,

    capture_tx: mpsc::Sender<CaptureResult>,
    capture_rx: mpsc::Receiver<CaptureResult>,
    captures_pending: usize,
    flash_at: Option<f64>,

    toasts: Vec<Toast>,
    swipe: Vec2,
    scroll_accum: f32,
    last_scroll_switch: f64,
    hold_since: Option<f64>,
    compare_button_down: bool,
    split_mode: bool,
    split_pos: f32,
    immersive: bool,
}

impl FilmrissApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> anyhow::Result<Self> {
        style::apply(&cc.egui_ctx);

        let rs = cc
            .wgpu_render_state
            .as_ref()
            .context("wgpu is not available – Filmriss needs a GPU (Metal/Vulkan/DX12/GL)")?;
        PreviewResources::install(rs);
        let info = rs.adapter.get_info();
        let gpu_info = format!("{} ({:?})", info.name, info.backend);
        log::info!("GPU: {gpu_info}, Format {:?}", rs.target_format);

        let settings: Settings = cc
            .storage
            .and_then(|s| eframe::get_value(s, "settings"))
            .unwrap_or_default();

        let paths = Paths::discover();
        if let Some(p) = &paths {
            if let Err(e) = p.ensure_exist() {
                log::warn!("could not create folders: {e}");
            }
            log::info!(
                "Recipes: {}  LUTs: {}  Photos: {}",
                p.recipes.display(),
                p.luts.display(),
                p.captures.display()
            );
        }

        let permission = Arc::new(AtomicU8::new(PERMISSION_PENDING));
        {
            let permission = permission.clone();
            let ctx = cc.egui_ctx.clone();
            filmriss_camera::request_permission(move |granted| {
                permission.store(
                    if granted { PERMISSION_GRANTED } else { PERMISSION_DENIED },
                    Ordering::SeqCst,
                );
                ctx.request_repaint();
            });
        }

        let (capture_tx, capture_rx) = mpsc::channel();
        let mut app = Self {
            settings,
            paths,
            library: Library {
                simulations: Vec::new(),
                warnings: Vec::new(),
            },
            pristine: Vec::new(),
            current: 0,
            tab: Tab::Look,
            save_name: String::new(),
            confirm_delete: false,
            permission,
            camera_started: false,
            cameras: Vec::new(),
            stream: None,
            reported_failure: false,
            frame: None,
            next_frame_id: 0,
            cam_rate: Rate::default(),
            ui_rate: Rate::default(),
            gpu_info,
            thumbs: Thumbnails::default(),
            scroll_to_current: true,
            switched_at: f64::NEG_INFINITY,
            capture_tx,
            capture_rx,
            captures_pending: 0,
            flash_at: None,
            toasts: Vec::new(),
            swipe: Vec2::ZERO,
            scroll_accum: 0.0,
            last_scroll_switch: 0.0,
            hold_since: None,
            compare_button_down: false,
            split_mode: false,
            split_pos: 0.5,
            immersive: false,
        };
        let wanted = app.settings.simulation.clone();
        app.reload_library(Some(wanted), 0.0);
        Ok(app)
    }

    // ── Library ──────────────────────────────────────────────────────────────

    fn reload_library(&mut self, select: Option<String>, now: f64) {
        let wanted = select.or_else(|| self.library.simulations.get(self.current).map(|s| s.name().to_owned()));
        self.library = Library::load(self.paths.as_ref());
        self.pristine = self.library.simulations.iter().map(|s| s.recipe.clone()).collect();
        let index = wanted
            .and_then(|name| self.library.simulations.iter().position(|s| s.name() == name))
            .unwrap_or(0);
        self.current = index;
        self.settings.simulation = self.library.simulations[index].name().to_owned();
        self.save_name = self.settings.simulation.clone();
        self.thumbs.invalidate();
        self.scroll_to_current = true;
        let warnings = std::mem::take(&mut self.library.warnings);
        for w in warnings {
            self.toast(w, true, now);
        }
    }

    fn select(&mut self, index: usize, now: f64) {
        let n = self.library.simulations.len();
        self.current = index.min(n - 1);
        self.settings.simulation = self.library.simulations[self.current].name().to_owned();
        self.save_name = self.settings.simulation.clone();
        self.confirm_delete = false;
        self.scroll_to_current = true;
        self.switched_at = now;
    }

    fn step(&mut self, delta: isize, now: f64) {
        let n = self.library.simulations.len() as isize;
        self.select((self.current as isize + delta).rem_euclid(n) as usize, now);
    }

    fn is_modified(&self, i: usize) -> bool {
        self.pristine
            .get(i)
            .is_some_and(|p| *p != self.library.simulations[i].recipe)
    }

    fn save_recipe(&mut self, as_new: bool, now: f64) {
        let Some(paths) = self.paths.clone() else {
            self.toast("No storage location available".into(), true, now);
            return;
        };
        let sim = &self.library.simulations[self.current];
        let mut recipe = sim.recipe.clone();
        let existing = match (&sim.origin, as_new) {
            (Origin::UserRecipe(p), false) => Some(p.clone()),
            _ => None,
        };
        if as_new {
            let name = self.save_name.trim();
            recipe.name = if name.is_empty() || name == sim.name() {
                format!("{} (custom)", sim.name())
            } else {
                name.to_owned()
            };
        }
        match Library::save_recipe(&paths, &recipe, existing.as_deref()) {
            Ok(path) => {
                self.toast(format!("Saved: {}", path.display()), false, now);
                // Don't silently keep the edits on the template
                if as_new {
                    self.library.simulations[self.current].recipe = self.pristine[self.current].clone();
                }
                self.reload_library(Some(recipe.name), now);
            }
            Err(e) => self.toast(format!("Saving failed: {e}"), true, now),
        }
    }

    fn reset_recipe(&mut self) {
        let sim = &mut self.library.simulations[self.current];
        sim.recipe = self.pristine[self.current].clone();
        sim.rebake();
        self.thumbs.invalidate();
    }

    fn delete_recipe(&mut self, now: f64) {
        if let Origin::UserRecipe(path) = &self.library.simulations[self.current].origin {
            match std::fs::remove_file(path) {
                Ok(()) => {
                    self.toast("Recipe deleted".into(), false, now);
                    let fallback = self.library.simulations[0].name().to_owned();
                    self.reload_library(Some(fallback), now);
                }
                Err(e) => self.toast(format!("Deleting failed: {e}"), true, now),
            }
        }
        self.confirm_delete = false;
    }

    fn export_cube(&mut self, now: f64) {
        let Some(paths) = &self.paths else { return };
        match Library::export_cube(paths, &self.library.simulations[self.current]) {
            Ok(path) => self.toast(format!("LUT exported: {}", path.display()), false, now),
            Err(e) => self.toast(format!("Export failed: {e}"), true, now),
        }
    }

    // ── Camera ───────────────────────────────────────────────────────────────

    fn ensure_camera(&mut self, ctx: &egui::Context, now: f64) {
        if self.camera_started {
            return;
        }
        let permission = self.permission.load(Ordering::SeqCst);
        if permission == PERMISSION_PENDING {
            return;
        }
        self.camera_started = true;
        if permission == PERMISSION_DENIED {
            self.toast(
                "No camera access – test pattern active. Allow it under System Settings › Privacy & Security › Camera."
                    .into(),
                true,
                now,
            );
            self.cameras = vec![CameraDescriptor::test_pattern()];
        } else {
            self.cameras = filmriss_camera::list_cameras();
        }
        let saved = self.settings.camera.clone();
        let desc = self
            .cameras
            .iter()
            .find(|c| Some(c.id.key()) == saved)
            .or_else(|| self.cameras.first())
            .cloned()
            .unwrap_or_else(CameraDescriptor::test_pattern);
        self.open_camera(desc, ctx);
    }

    fn open_camera(&mut self, desc: CameraDescriptor, ctx: &egui::Context) {
        log::info!("Opening camera: {}", desc.name);
        // Close the old stream first so the device is released.
        self.stream = None;
        let ctx = ctx.clone();
        let notify: Notify = Arc::new(move || ctx.request_repaint());
        self.settings.camera = Some(desc.id.key());
        self.stream = Some(CameraStream::open(desc, notify));
        self.reported_failure = false;
    }

    fn poll_camera(&mut self, now: f64) {
        let Some(stream) = &self.stream else { return };
        if let Some(frame) = stream.take_frame() {
            self.next_frame_id += 1;
            let new = PreviewFrame {
                id: self.next_frame_id,
                frame: Arc::new(frame),
            };
            if let Some(old) = self.frame.replace(new) {
                // Return the buffer unless someone still holds it (e.g. a running capture).
                if let Ok(f) = Arc::try_unwrap(old.frame) {
                    stream.recycle(f.data);
                }
            }
            self.cam_rate.tick(now);
        }
        if let StreamState::Failed(msg) = stream.state()
            && !self.reported_failure
        {
            self.reported_failure = true;
            self.toast(msg, true, now);
        }
    }

    fn is_test_pattern(&self) -> bool {
        self.stream
            .as_ref()
            .is_none_or(|s| s.descriptor().id == CameraId::TestPattern)
    }

    // ── Capture ──────────────────────────────────────────────────────────────

    fn capture(&mut self, ctx: &egui::Context, now: f64) {
        let Some(frame) = &self.frame else {
            self.toast("No camera frame yet".into(), true, now);
            return;
        };
        let Some(paths) = &self.paths else {
            self.toast("No storage location available".into(), true, now);
            return;
        };
        let sim = &self.library.simulations[self.current];
        let job = CaptureJob {
            frame: frame.frame.clone(),
            lut: sim.lut.clone(),
            // Same seed as the preview → identical grain
            params: sim.look_params(self.settings.intensity, frame.id as u32),
            simulation: sim.name().to_owned(),
            dir: paths.captures.clone(),
            save_original: self.settings.save_original,
        };
        capture::spawn(job, self.capture_tx.clone(), ctx.clone());
        self.captures_pending += 1;
        self.flash_at = Some(now);
    }

    fn poll_captures(&mut self, now: f64) {
        while let Ok(result) = self.capture_rx.try_recv() {
            self.captures_pending = self.captures_pending.saturating_sub(1);
            match result {
                Ok(path) => {
                    let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                    self.toast(format!("📷 {name}"), false, now);
                }
                Err(e) => self.toast(format!("Photo failed: {e}"), true, now),
            }
        }
    }

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn toast(&mut self, text: String, error: bool, now: f64) {
        if error {
            log::warn!("{text}");
        }
        let secs = if error { 6.0 } else { 3.0 };
        self.toasts.push(Toast {
            text,
            until: now + secs,
            error,
        });
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context, now: f64) {
        if ctx.egui_wants_keyboard_input() {
            self.compare_button_down = false;
            return;
        }
        // Arrows/Space/Enter belong to the app: consume them so egui doesn't move focus
        // with them or trigger a focused button a second time.
        let (prev, next, shoot, info, split, mirror, hide, panel, full, compare) = ctx.input_mut(|i| {
            let none = egui::Modifiers::NONE;
            (
                i.consume_key(none, Key::ArrowLeft),
                i.consume_key(none, Key::ArrowRight),
                i.consume_key(none, Key::Space) | i.consume_key(none, Key::Enter),
                i.key_pressed(Key::I),
                i.key_pressed(Key::S),
                i.key_pressed(Key::M),
                i.key_pressed(Key::H),
                i.key_pressed(Key::P),
                i.key_pressed(Key::F),
                i.key_down(Key::C),
            )
        });
        if prev {
            self.step(-1, now);
        }
        if next {
            self.step(1, now);
        }
        if shoot {
            self.capture(ctx, now);
        }
        if info {
            self.settings.show_info ^= true;
        }
        if split {
            self.split_mode ^= true;
        }
        if mirror {
            self.settings.mirror ^= true;
        }
        if hide {
            self.immersive ^= true;
        }
        if panel {
            self.settings.panel_open ^= true;
        }
        if full {
            let is_full = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!is_full));
        }
        self.compare_button_down = compare;
    }
}

impl eframe::App for FilmrissApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        self.ui_rate.tick(now);

        self.ensure_camera(&ctx, now);
        self.poll_camera(now);
        self.poll_captures(now);
        self.handle_keys(&ctx, now);
        self.thumbs
            .update(&ctx, now, self.frame.as_ref(), &self.library.simulations);
        self.toasts.retain(|t| t.until > now);
        if let Some(t) = self.toasts.iter().map(|t| t.until).reduce(f64::min) {
            ctx.request_repaint_after(Duration::from_secs_f64((t - now).max(0.0)));
        }

        if !self.immersive {
            let narrow = ui.available_width() < 760.0;
            let mut open = self.settings.panel_open;
            let side = if narrow {
                egui::Panel::bottom("side-narrow")
                    .default_size(320.0)
                    .size_range(160.0..=600.0)
            } else {
                egui::Panel::right("side").default_size(340.0).size_range(280.0..=520.0)
            };
            side.resizable(true)
                .frame(egui::Frame::new().fill(style::PANEL).inner_margin(Margin::same(12)))
                .show_collapsible(ui, &mut open, |ui| self.side_panel(ui, now));
            self.settings.panel_open = open;

            egui::Panel::top("top")
                .frame(egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(12, 8)))
                .show(ui, |ui| self.top_bar(ui));
            egui::Panel::bottom("bottom")
                .frame(egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(12, 10)))
                .show(ui, |ui| self.bottom_bar(ui, now));
        }

        egui::CentralPanel::no_frame().show(ui, |ui| self.preview(ui, now));
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, "settings", &self.settings);
    }
}

// ── UI parts ─────────────────────────────────────────────────────────────────

impl FilmrissApp {
    fn top_bar(&mut self, ui: &mut Ui) {
        // Icons only on narrow displays so nothing overlaps
        let compact = ui.available_width() < 620.0;
        ui.horizontal(|ui| {
            ui.label(RichText::new("FILMRISS").color(ACCENT).strong().size(18.0));
            if !compact {
                ui.add_space(8.0);
                ui.label(RichText::new(self.library.simulations[self.current].name()).strong());
            }

            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                let pick = |full: &'static str, short: &'static str| if compact { short } else { full };
                let panel_label = if self.settings.panel_open {
                    pick("⚙ Hide", "⚙")
                } else {
                    pick("⚙ Settings", "⚙")
                };
                if ui
                    .selectable_label(self.settings.panel_open, panel_label)
                    .on_hover_text("Side panel (P)")
                    .clicked()
                {
                    self.settings.panel_open ^= true;
                }
                if ui
                    .selectable_label(self.split_mode, pick("◑ Compare", "◑"))
                    .on_hover_text("Split view (S)")
                    .clicked()
                {
                    self.split_mode ^= true;
                }
                if ui
                    .selectable_label(self.settings.show_info, pick("ℹ Info", "ℹ"))
                    .on_hover_text("Technical info (I)")
                    .clicked()
                {
                    self.settings.show_info ^= true;
                }
            });
        });
    }

    fn bottom_bar(&mut self, ui: &mut Ui, now: f64) {
        let mut clicked = None;
        ScrollArea::horizontal().id_salt("filmstrip").show(ui, |ui| {
            ui.horizontal(|ui| {
                for i in 0..self.library.simulations.len() {
                    let resp = self.sim_card(ui, i);
                    if i == self.current && self.scroll_to_current {
                        resp.scroll_to_me(Some(Align::Center));
                        self.scroll_to_current = false;
                    }
                    if resp.clicked() {
                        clicked = Some(i);
                    }
                }
            });
        });
        if let Some(i) = clicked {
            self.select(i, now);
        }

        ui.add_space(6.0);
        // Narrow (phone portrait): strength slider on its own row above the shutter
        let narrow = ui.available_width() < 560.0;
        if narrow {
            self.intensity_slider(ui);
        }
        ui.columns(3, |cols| {
            if !narrow {
                cols[0].with_layout(egui::Layout::left_to_right(Align::Center), |ui| {
                    self.intensity_slider(ui)
                });
            }
            cols[1].vertical_centered(|ui| {
                if shutter_button(ui, self.captures_pending > 0).clicked() {
                    let ctx = ui.ctx().clone();
                    self.capture(&ctx, now);
                }
            });
            cols[2].with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                let resp = ui.add(egui::Button::new("👁 Before").sense(Sense::click_and_drag()));
                let resp = resp.on_hover_text("Hold to show the original (key C)");
                if resp.is_pointer_button_down_on() {
                    self.compare_button_down = true;
                }
            });
        });
    }

    fn intensity_slider(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Strength").color(TEXT_DIM));
            let mut pct = self.settings.intensity * 100.0;
            ui.style_mut().spacing.slider_width = (ui.available_width() - 80.0).clamp(60.0, 220.0);
            let slider = egui::Slider::new(&mut pct, 0.0..=100.0).suffix(" %").fixed_decimals(0);
            let resp = ui.add(slider).on_hover_text("Double-click: 100 %");
            if resp.double_clicked() {
                self.settings.intensity = 1.0;
            } else if resp.changed() {
                self.settings.intensity = pct / 100.0;
            }
        });
    }

    fn sim_card(&self, ui: &mut Ui, i: usize) -> egui::Response {
        let size = Vec2::new(104.0, 84.0);
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        let painter = ui.painter_at(rect.expand(2.0));
        let img_rect = Rect::from_min_size(rect.min, Vec2::new(size.x, 58.0));
        let selected = i == self.current;

        match self.thumbs.get(i) {
            Some(tex) => {
                egui::Image::new(tex)
                    .corner_radius(6)
                    .fit_to_exact_size(img_rect.size())
                    .paint_at(ui, img_rect);
            }
            None => {
                painter.rect_filled(img_rect, 6, Color32::from_gray(40));
            }
        }
        if selected {
            painter.rect_stroke(img_rect, 6, Stroke::new(2.5, ACCENT), StrokeKind::Outside);
        } else if resp.hovered() {
            painter.rect_stroke(
                img_rect,
                6,
                Stroke::new(1.0, Color32::from_gray(160)),
                StrokeKind::Outside,
            );
        }

        let sim = &self.library.simulations[i];
        let mut name = sim.name().to_owned();
        if self.is_modified(i) {
            name.push_str(" •");
        }
        let color = if selected { ACCENT } else { Color32::from_gray(210) };
        let galley = painter.layout(name, FontId::proportional(12.5), color, size.x);
        let text_pos = egui::pos2(rect.center().x - galley.size().x / 2.0, img_rect.bottom() + 4.0);
        painter.galley(text_pos, galley, color);
        resp.on_hover_text(&sim.recipe.description)
    }

    fn preview(&mut self, ui: &mut Ui, now: f64) {
        let avail = ui.available_rect_before_wrap();
        let painter = ui.painter_at(avail);
        painter.rect_filled(avail, 0, Color32::BLACK);
        let resp = ui.allocate_rect(avail, Sense::click_and_drag());

        let Some(frame) = self.frame.clone() else {
            let msg = match (
                self.permission.load(Ordering::SeqCst),
                self.stream.as_ref().map(|s| s.state()),
            ) {
                (PERMISSION_PENDING, _) => "Waiting for camera permission …".to_owned(),
                (_, Some(StreamState::Failed(e))) => format!("Camera error:\n{e}"),
                _ => "Camera starting …".to_owned(),
            };
            painter.text(
                avail.center(),
                Align2::CENTER_CENTER,
                msg,
                FontId::proportional(18.0),
                TEXT_DIM,
            );
            if matches!(self.stream.as_ref().map(|s| s.state()), Some(StreamState::Failed(_))) {
                let btn = Rect::from_center_size(avail.center() + Vec2::new(0.0, 60.0), Vec2::new(200.0, 40.0));
                if ui.put(btn, egui::Button::new("Use test pattern")).clicked() {
                    let ctx = ui.ctx().clone();
                    self.open_camera(CameraDescriptor::test_pattern(), &ctx);
                }
            }
            self.draw_toasts(&painter, avail);
            return;
        };

        // Fit the image (keep aspect ratio)
        let aspect = frame.frame.width as f32 / frame.frame.height as f32;
        let mut size = avail.size();
        if size.x / size.y > aspect {
            size.x = size.y * aspect;
        } else {
            size.y = size.x / aspect;
        }
        let rect = Rect::from_center_size(avail.center(), size);

        // ── Gestures ──
        let mut split = 0.0;
        if self.split_mode {
            if let Some(pos) = resp.interact_pointer_pos()
                && (resp.dragged() || resp.clicked())
            {
                self.split_pos = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            }
            split = self.split_pos;
        } else {
            if resp.dragged() {
                self.swipe += resp.drag_delta();
            }
            if resp.drag_stopped() {
                if self.swipe.x.abs() > SWIPE_THRESHOLD && self.swipe.x.abs() > self.swipe.y.abs() {
                    self.step(if self.swipe.x < 0.0 { 1 } else { -1 }, now);
                }
                self.swipe = Vec2::ZERO;
            }
        }
        if resp.is_pointer_button_down_on() {
            self.hold_since.get_or_insert(now);
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        } else {
            self.hold_since = None;
            self.swipe = if resp.dragged() { self.swipe } else { Vec2::ZERO };
        }
        let holding = !self.split_mode
            && self.hold_since.is_some_and(|t| now - t > HOLD_TO_COMPARE)
            && self.swipe.length() < 12.0;
        let show_original = holding || self.compare_button_down;
        if show_original {
            split = 1.0;
        }
        if resp.double_clicked() {
            self.immersive ^= true;
        }
        // Trackpad: horizontal two-finger swipe switches the simulation
        if resp.hovered() {
            let dx = ui.ctx().input(|i| i.smooth_scroll_delta.x);
            if dx == 0.0 {
                self.scroll_accum *= 0.8;
            } else {
                self.scroll_accum += dx;
            }
            if self.scroll_accum.abs() > 120.0 && now - self.last_scroll_switch > 0.4 {
                self.step(if self.scroll_accum < 0.0 { 1 } else { -1 }, now);
                self.last_scroll_switch = now;
                self.scroll_accum = 0.0;
            }
        }

        // ── GPU preview ──
        let sim = &self.library.simulations[self.current];
        let uniforms = Uniforms {
            intensity: self.settings.intensity,
            grain: sim.recipe.grain,
            grain_size: sim.recipe.grain_size,
            vignette: sim.recipe.vignette,
            split,
            mirror: if self.settings.mirror && !self.is_test_pattern() {
                1.0
            } else {
                0.0
            },
            seed: frame.id as u32,
            ..Default::default()
        };
        painter.add(egui_wgpu::Callback::new_paint_callback(
            rect,
            PreviewCallback {
                frame: frame.clone(),
                lut: sim.lut.clone(),
                uniforms,
            },
        ));

        // ── Overlays ──
        if self.split_mode {
            let x = rect.left() + rect.width() * self.split_pos;
            painter.line_segment(
                [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                Stroke::new(2.0, Color32::WHITE),
            );
            painter.circle(
                egui::pos2(x, rect.center().y),
                14.0,
                Color32::from_black_alpha(160),
                Stroke::new(2.0, Color32::WHITE),
            );
            label(
                &painter,
                egui::pos2(rect.left() + 12.0, rect.bottom() - 12.0),
                Align2::LEFT_BOTTOM,
                "Original",
            );
            label(
                &painter,
                egui::pos2(rect.right() - 12.0, rect.bottom() - 12.0),
                Align2::RIGHT_BOTTOM,
                sim.name(),
            );
        } else if show_original {
            label(
                &painter,
                rect.center_top() + Vec2::new(0.0, 16.0),
                Align2::CENTER_TOP,
                "Original",
            );
        }

        let since_switch = now - self.switched_at;
        if since_switch < 1.4 {
            let alpha = (1.0 - ((since_switch - 0.9) / 0.5).clamp(0.0, 1.0)) as f32;
            let pos = rect.center_top() + Vec2::new(0.0, 28.0);
            let galley = painter.layout_no_wrap(
                sim.name().to_owned(),
                FontId::proportional(28.0),
                Color32::WHITE.gamma_multiply(alpha),
            );
            let bg = Rect::from_center_size(
                pos + Vec2::new(0.0, galley.size().y / 2.0),
                galley.size() + Vec2::new(28.0, 14.0),
            );
            painter.rect_filled(bg, 10, Color32::from_black_alpha((140.0 * alpha) as u8));
            painter.galley(bg.center() - galley.size() / 2.0, galley, Color32::WHITE);
            ui.ctx().request_repaint();
        }

        if self.settings.show_info {
            let f = &frame.frame;
            let cam = self
                .stream
                .as_ref()
                .map(|s| s.descriptor().name.clone())
                .unwrap_or_default();
            let fmt = match self.stream.as_ref().map(|s| s.state()) {
                Some(StreamState::Running { format, .. }) => format,
                _ => String::new(),
            };
            let text = format!(
                "{cam}  ·  {}×{}  {fmt}\nCamera {} fps  ·  UI {} fps\nGPU {}\nLUT {}³  ·  Frame #{}",
                f.width,
                f.height,
                self.cam_rate.per_sec(),
                self.ui_rate.per_sec(),
                self.gpu_info,
                sim.lut.size(),
                frame.id,
            );
            let galley = painter.layout_no_wrap(text, FontId::monospace(12.0), Color32::from_gray(230));
            let bg = Rect::from_min_size(rect.min + Vec2::splat(10.0), galley.size() + Vec2::splat(16.0));
            painter.rect_filled(bg, 8, Color32::from_black_alpha(150));
            painter.galley(bg.min + Vec2::splat(8.0), galley, Color32::WHITE);
        }

        if let Some(t) = self.flash_at {
            let age = (now - t) as f32;
            if age < 0.2 {
                painter.rect_filled(rect, 0, Color32::from_white_alpha((200.0 * (1.0 - age / 0.2)) as u8));
                ui.ctx().request_repaint();
            } else {
                self.flash_at = None;
            }
        }

        self.draw_toasts(&painter, avail);
    }

    fn draw_toasts(&self, painter: &egui::Painter, area: Rect) {
        let mut y = area.bottom() - 16.0;
        for t in self.toasts.iter().rev() {
            let color = if t.error {
                Color32::from_rgb(255, 170, 160)
            } else {
                Color32::WHITE
            };
            let galley = painter.layout(t.text.clone(), FontId::proportional(14.0), color, area.width() - 80.0);
            let size = galley.size() + Vec2::new(24.0, 14.0);
            let bg = Rect::from_min_size(egui::pos2(area.center().x - size.x / 2.0, y - size.y), size);
            painter.rect_filled(bg, 10, Color32::from_black_alpha(190));
            if t.error {
                painter.rect_stroke(
                    bg,
                    10,
                    Stroke::new(1.0, Color32::from_rgb(200, 80, 70)),
                    StrokeKind::Inside,
                );
            }
            painter.galley(bg.min + Vec2::new(12.0, 7.0), galley, color);
            y = bg.top() - 8.0;
        }
    }

    fn side_panel(&mut self, ui: &mut Ui, now: f64) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, Tab::Look, "Look");
            ui.selectable_value(&mut self.tab, Tab::Recipe, "Recipe");
            ui.selectable_value(&mut self.tab, Tab::Camera, "Camera");
            ui.selectable_value(&mut self.tab, Tab::Help, "Help");
        });
        ui.separator();
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| match self.tab {
            Tab::Look => self.tab_look(ui, now),
            Tab::Recipe => self.tab_recipe(ui, now),
            Tab::Camera => self.tab_camera(ui),
            Tab::Help => self.tab_help(ui),
        });
    }

    fn tab_look(&mut self, ui: &mut Ui, now: f64) {
        let sim = &self.library.simulations[self.current];
        ui.heading(sim.name());
        ui.label(RichText::new(origin_label(&sim.origin)).color(TEXT_DIM).small());
        if !sim.recipe.description.is_empty() {
            ui.label(&sim.recipe.description);
        }
        ui.add_space(8.0);

        let mut chosen = None;
        for (i, s) in self.library.simulations.iter().enumerate() {
            let mut text = RichText::new(s.name());
            if !s.is_builtin() {
                text = text.italics();
            }
            if ui
                .add_sized(
                    [ui.available_width(), 30.0],
                    egui::Button::selectable(i == self.current, text),
                )
                .clicked()
            {
                chosen = Some(i);
            }
        }
        if let Some(i) = chosen {
            self.select(i, now);
        }

        ui.add_space(8.0);
        ui.separator();
        ui.checkbox(&mut self.settings.mirror, "Mirror preview (selfie)");
        ui.checkbox(&mut self.settings.save_original, "Also save the original");
        ui.checkbox(&mut self.settings.show_info, "Show technical info");
        ui.add_space(8.0);
        if ui
            .button("↻ Reload library")
            .on_hover_text("Re-reads recipes and .cube files")
            .clicked()
        {
            self.reload_library(None, now);
        }
        if let Some(paths) = &self.paths {
            ui.horizontal_wrapped(|ui| {
                if ui.button("📁 LUTs").on_hover_text("Put .cube files here").clicked() {
                    open_folder(&paths.luts);
                }
                if ui.button("📁 Recipes").clicked() {
                    open_folder(&paths.recipes);
                }
                if ui.button("📁 Photos").clicked() {
                    open_folder(&paths.captures);
                }
            });
        }
    }

    fn tab_recipe(&mut self, ui: &mut Ui, now: f64) {
        let modified = self.is_modified(self.current);
        let sim = &mut self.library.simulations[self.current];
        ui.horizontal(|ui| {
            ui.heading(sim.recipe.name.clone());
            if modified {
                ui.label(RichText::new("• modified").color(ACCENT));
            }
        });
        ui.label(
            RichText::new("Changes apply live. Double-click a slider to reset it.")
                .color(TEXT_DIM)
                .small(),
        );
        ui.add_space(6.0);

        if crate::editor::show(ui, &mut sim.recipe) {
            sim.rebake();
            self.thumbs.invalidate();
        }

        ui.add_space(10.0);
        ui.separator();
        let is_user = matches!(self.library.simulations[self.current].origin, Origin::UserRecipe(_));
        ui.label(RichText::new("Save").strong());
        ui.horizontal(|ui| {
            ui.label("Name");
            ui.add(egui::TextEdit::singleline(&mut self.save_name).desired_width(f32::INFINITY));
        });
        ui.horizontal_wrapped(|ui| {
            if is_user && ui.add_enabled(modified, egui::Button::new("💾 Save")).clicked() {
                self.save_recipe(false, now);
            }
            if ui.button("➕ Save as new simulation").clicked() {
                self.save_recipe(true, now);
            }
            if ui.add_enabled(modified, egui::Button::new("↺ Reset")).clicked() {
                self.reset_recipe();
            }
        });
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("📤 Export as .cube")
                .on_hover_text("For Lightroom, Photoshop, Resolve, Final Cut …")
                .clicked()
            {
                self.export_cube(now);
            }
            if is_user {
                let label = if self.confirm_delete {
                    "Really delete?"
                } else {
                    "🗑 Delete"
                };
                if ui
                    .button(RichText::new(label).color(Color32::from_rgb(255, 140, 130)))
                    .clicked()
                {
                    if self.confirm_delete {
                        self.delete_recipe(now);
                    } else {
                        self.confirm_delete = true;
                    }
                }
            }
        });
    }

    fn tab_camera(&mut self, ui: &mut Ui) {
        let current = self.stream.as_ref().map(|s| s.descriptor().id.clone());
        let mut chosen = None;
        for cam in &self.cameras {
            if ui
                .add_sized(
                    [ui.available_width(), 30.0],
                    egui::Button::selectable(current.as_ref() == Some(&cam.id), &cam.name),
                )
                .clicked()
            {
                chosen = Some(cam.clone());
            }
        }
        if let Some(cam) = chosen
            && current.as_ref() != Some(&cam.id)
        {
            let ctx = ui.ctx().clone();
            self.open_camera(cam, &ctx);
        }
        if ui.button("↻ Rescan cameras").clicked() {
            self.camera_started = false;
            self.stream = None;
            let ctx = ui.ctx().clone();
            self.ensure_camera(&ctx, 0.0);
        }
        ui.add_space(8.0);
        if let Some(stream) = &self.stream {
            let state = match stream.state() {
                StreamState::Starting => "starting …".to_owned(),
                StreamState::Running { width, height, format } => format!("running – {width}×{height}, {format}"),
                StreamState::Failed(e) => format!("error: {e}"),
            };
            ui.label(RichText::new(format!("Status: {state}")).color(TEXT_DIM));
        }
        ui.checkbox(&mut self.settings.mirror, "Mirror preview (selfie)");
    }

    fn tab_help(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("Gestures").strong());
        for line in [
            "Swipe left/right – switch simulation",
            "Trackpad: two-finger sideways swipe – switch simulation",
            "Press and hold – show original",
            "Double-tap – hide/show controls",
        ] {
            ui.label(format!("• {line}"));
        }
        ui.add_space(8.0);
        ui.label(RichText::new("Keyboard").strong());
        egui::Grid::new("keys")
            .num_columns(2)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                for (k, d) in [
                    ("← / →", "Switch simulation"),
                    ("Space", "Take photo"),
                    ("C (hold)", "Show original"),
                    ("S", "Split view"),
                    ("I", "Technical info"),
                    ("M", "Mirror"),
                    ("P", "Side panel"),
                    ("H", "Hide everything"),
                    ("F", "Fullscreen"),
                ] {
                    ui.monospace(k);
                    ui.label(d);
                    ui.end_row();
                }
            });
        if let Some(p) = &self.paths {
            ui.add_space(8.0);
            ui.label(RichText::new("Folders").strong());
            for (name, path) in [
                ("Photos", &p.captures),
                ("Recipes", &p.recipes),
                ("LUTs", &p.luts),
                ("Exports", &p.exports),
            ] {
                ui.horizontal(|ui| {
                    if ui.small_button("open").clicked() {
                        open_folder(path);
                    }
                    ui.label(
                        RichText::new(format!("{name}: {}", path.display()))
                            .small()
                            .color(TEXT_DIM),
                    );
                });
            }
        }
    }
}

fn shutter_button(ui: &mut Ui, busy: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(68.0), Sense::click());
    let painter = ui.painter();
    let c = rect.center();
    let pressed = resp.is_pointer_button_down_on();
    painter.circle_stroke(c, 31.0, Stroke::new(4.0, Color32::WHITE));
    let inner = if pressed { 22.0 } else { 25.0 };
    let fill = if busy {
        ACCENT
    } else if resp.hovered() {
        Color32::from_gray(235)
    } else {
        Color32::WHITE
    };
    painter.circle_filled(c, inner, fill);
    resp.on_hover_text("Take photo (Space)")
}

fn label(painter: &egui::Painter, pos: egui::Pos2, align: Align2, text: &str) {
    let galley = painter.layout_no_wrap(text.to_owned(), FontId::proportional(14.0), Color32::WHITE);
    let rect = align.anchor_size(pos, galley.size() + Vec2::new(16.0, 8.0));
    painter.rect_filled(rect, CornerRadius::same(6), Color32::from_black_alpha(150));
    painter.galley(rect.min + Vec2::new(8.0, 4.0), galley, Color32::WHITE);
}

fn origin_label(origin: &Origin) -> String {
    match origin {
        Origin::BuiltIn => "Built-in simulation".into(),
        Origin::UserRecipe(p) => format!(
            "Custom recipe · {}",
            p.file_name().unwrap_or_default().to_string_lossy()
        ),
        Origin::CubeFile(p) => format!("3D-LUT · {}", p.file_name().unwrap_or_default().to_string_lossy()),
    }
}

fn open_folder(path: &Path) {
    let _ = std::fs::create_dir_all(path);
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(target_os = "windows")]
    let cmd = "explorer";
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let cmd = "xdg-open";
    if let Err(e) = std::process::Command::new(cmd).arg(path).spawn() {
        log::warn!("could not open folder: {e}");
    }
}

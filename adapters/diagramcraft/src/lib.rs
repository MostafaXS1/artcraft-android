//! Builder-owned Android shell for DiagramCraft.
//!
//! ARTCRAFT_DIAGRAMCRAFT_SAF_BRIDGE_V1

use android_activity::AndroidApp;
use diagramcraft_model::Document;
use diagramcraft_render::raster;
use diagramcraft_render::scene::{page, Options as RenderOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

mod system_fonts;
mod saf;

const SAF_BRIDGE_MARKER: &str = "ARTCRAFT_DIAGRAMCRAFT_SAF_BRIDGE_V1";
const ANDROID_SHELL_MARKER: &str = "ARTCRAFT_DIAGRAMCRAFT_ANDROID_SHELL_V1";

static ALIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

#[allow(unused_unsafe)]
fn install_persist_env(app: &AndroidApp) {
    let Some(base) = app.external_data_path().or_else(|| app.internal_data_path()) else {
        return;
    };
    let config = base.join("config");
    let cache = base.join("cache");
    let _ = std::fs::create_dir_all(&config);
    let _ = std::fs::create_dir_all(&cache);
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", &config);
        std::env::set_var("XDG_CACHE_HOME", &cache);
    }
}

fn ensure_screen_rect(app: &AndroidApp, ctx: &egui::Context, raw: &mut egui::RawInput) {
    const UNSET: f32 = 4096.0;
    let usable = raw
        .screen_rect
        .is_some_and(|r| r.width() > 0.0 && r.height() > 0.0 && r.width() < UNSET && r.height() < UNSET);
    if usable {
        return;
    }
    let (width, height) = if let Some(window) = app.native_window() {
        let (w, h) = (window.width(), window.height());
        if w > 0 && h > 0 {
            (w as f32, h as f32)
        } else {
            (1920.0, 1200.0)
        }
    } else {
        (1920.0, 1200.0)
    };
    let ppp = ctx.pixels_per_point().max(0.1);
    raw.screen_rect = Some(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(width / ppp, height / ppp),
    ));
}

fn apply_android_modifiers(state: &mut egui::Modifiers, raw: &mut egui::RawInput) {
    let before = *state;
    let mut kept: Vec<egui::Event> = Vec::with_capacity(raw.events.len());
    for mut event in raw.events.drain(..) {
        match &mut event {
            egui::Event::Key {
                key,
                physical_key,
                pressed,
                modifiers,
                ..
            } => {
                let tracked = physical_key.unwrap_or(*key);
                match tracked {
                    egui::Key::ShiftLeft | egui::Key::ShiftRight => state.shift = *pressed,
                    egui::Key::ControlLeft | egui::Key::ControlRight => state.ctrl = *pressed,
                    egui::Key::AltLeft | egui::Key::AltRight => state.alt = *pressed,
                    _ => {}
                }
                state.mac_cmd = false;
                state.command = state.ctrl;
                *modifiers = *state;
                kept.push(event);
            }
            egui::Event::PointerButton { modifiers, .. } => {
                *modifiers = *state;
                kept.push(event);
            }
            egui::Event::MouseWheel { modifiers, .. } => {
                *modifiers = *state;
                kept.push(event);
            }
            egui::Event::Text(_) if state.ctrl || state.command || state.mac_cmd => {}
            egui::Event::WindowFocused(false) => {
                *state = egui::Modifiers::default();
                kept.push(event);
            }
            _ => kept.push(event),
        }
    }
    raw.events = kept;
    if *state != before {
        raw.events.push(egui::Event::ModifiersChanged(*state));
    }
}

#[derive(Default)]
struct SafState {
    next_id: AtomicU64,
    mirror: saf::SaveMirror,
}

impl SafState {
    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed).wrapping_add(1)
    }
}

struct DiagramCraftApp {
    app: AndroidApp,
    bridge: saf::Bridge,
    state: Arc<SafState>,
    modifiers: egui::Modifiers,
    intent_warmup: i64,

    // Document state
    current_path: Option<PathBuf>,
    document: Option<Document>,
    current_page_idx: usize,
    page_texture: Option<egui::TextureHandle>,
    error_message: Option<String>,

    // View state
    zoom: f32,
    pan: egui::Vec2,
}

impl DiagramCraftApp {
    fn new(app: AndroidApp, bridge: saf::Bridge, state: Arc<SafState>) -> Self {
        Self {
            app,
            bridge,
            state,
            modifiers: egui::Modifiers::default(),
            intent_warmup: 2,
            current_path: None,
            document: None,
            current_page_idx: 0,
            page_texture: None,
            error_message: None,
            zoom: 1.0,
            pan: egui::Vec2::ZERO,
        }
    }

    fn load_file(&mut self, ctx: &egui::Context, path: &Path) {
        log::info!("Loading Visio document: {}", path.display());
        match diagramcraft_vsdx::read(path) {
            Ok(doc) => {
                self.current_path = Some(path.to_path_buf());
                self.current_page_idx = 0;
                self.error_message = None;
                self.zoom = 1.0;
                self.pan = egui::Vec2::ZERO;
                self.document = Some(doc);
                self.render_current_page(ctx);
            }
            Err(e) => {
                log::error!("Failed to load Visio document: {e}");
                self.error_message = Some(format!("Could not read Visio file: {e}"));
            }
        }
    }

    fn render_current_page(&mut self, ctx: &egui::Context) {
        self.page_texture = None;
        let Some(ref doc) = self.document else { return };
        if doc.pages.is_empty() {
            self.error_message = Some("Visio document contains no pages.".into());
            return;
        }
        let page_idx = self.current_page_idx.min(doc.pages.len() - 1);
        let opts = RenderOptions::default();
        match page(doc, page_idx, opts) {
            Ok(scene) => match raster::render(&scene, opts) {
                Ok(img) => {
                    let size = [img.width() as usize, img.height() as usize];
                    let color_img = egui::ColorImage::from_rgba_unmultiplied(size, &img.into_raw());
                    let texture = ctx.load_texture("visio-page", color_img, egui::TextureOptions::LINEAR);
                    self.page_texture = Some(texture);
                    self.error_message = None;
                }
                Err(e) => {
                    self.error_message = Some(format!("Rasterization failed: {e}"));
                }
            },
            Err(e) => {
                self.error_message = Some(format!("Failed to layout page {page_idx}: {e}"));
            }
        }
    }
}

impl Drop for DiagramCraftApp {
    fn drop(&mut self) {
        ALIVE.store(false, Ordering::Relaxed);
    }
}

impl eframe::App for DiagramCraftApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.state.mirror.sync(&self.bridge);
        watch_incoming(&self.bridge, ctx);
        if let Some(path) = drain_incoming(&self.bridge, &self.state, &mut self.intent_warmup) {
            self.load_file(ctx, &PathBuf::from(path));
        }
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        ensure_screen_rect(&self.app, ctx, raw_input);
        apply_android_modifiers(&mut self.modifiers, raw_input);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        system_fonts::install_system_fonts(ui.ctx());

        // Top bar
        egui::Panel::top("top_panel").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("DiagramCraft");
                ui.separator();

                if ui.button("📂 Open Visio…").clicked() {
                    let bridge = self.bridge.clone_ref();
                    let id = self.state.next_id();
                    let answer = bridge.pick_open_blocking(id, false, saf::PICK_TIMEOUT);
                    if let Ok(saf::PickAnswer::Paths(paths)) = answer {
                        if let Some(first) = paths.first() {
                            self.load_file(ui.ctx(), &PathBuf::from(first));
                        }
                    }
                }

                let mut page_change: Option<isize> = None;
                if let Some(ref doc) = self.document {
                    let num_pages = doc.pages.len();
                    if num_pages > 1 {
                        ui.separator();
                        if ui.add_enabled(self.current_page_idx > 0, egui::Button::new("◀ Prev")).clicked() {
                            page_change = Some(-1);
                        }

                        let current_name = doc.pages.get(self.current_page_idx)
                            .map(|p| p.name.as_str())
                            .filter(|n| !n.is_empty())
                            .unwrap_or("Untitled Page");

                        ui.label(format!("Page {} of {}: \"{}\"", self.current_page_idx + 1, num_pages, current_name));

                        if ui.add_enabled(self.current_page_idx + 1 < num_pages, egui::Button::new("Next ▶")).clicked() {
                            page_change = Some(1);
                        }
                    }

                    ui.separator();
                    if ui.button("🔍 -").clicked() {
                        self.zoom = (self.zoom * 0.8).max(0.1);
                    }
                    ui.label(format!("{:.0}%", self.zoom * 100.0));
                    if ui.button("🔍 +").clicked() {
                        self.zoom = (self.zoom * 1.25).min(10.0);
                    }
                    if ui.button("Reset").clicked() {
                        self.zoom = 1.0;
                        self.pan = egui::Vec2::ZERO;
                    }
                }

                if let Some(delta) = page_change {
                    if delta < 0 && self.current_page_idx > 0 {
                        self.current_page_idx -= 1;
                        self.render_current_page(ui.ctx());
                    } else if delta > 0 {
                        self.current_page_idx += 1;
                        self.render_current_page(ui.ctx());
                    }
                }

                if let Some(ref path) = self.current_path {
                    ui.separator();
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("document");
                    ui.label(egui::RichText::new(name).strong());
                }
            });
        });

        // Main canvas area
        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(ref err) = self.error_message {
                ui.colored_label(egui::Color32::RED, err);
            }

            if let Some(ref texture) = self.page_texture {
                let available = ui.available_size();
                let (rect, response) = ui.allocate_exact_size(available, egui::Sense::drag());

                if response.dragged() {
                    self.pan += response.drag_delta();
                }

                let tex_size = texture.size_vec2();
                let fit_scale = (available.x / tex_size.x).min(available.y / tex_size.y).min(1.0);
                let current_scale = fit_scale * self.zoom;
                let draw_size = tex_size * current_scale;

                let center = rect.center() + self.pan;
                let image_rect = egui::Rect::from_center_size(center, draw_size);

                let painter = ui.painter_at(rect);
                painter.image(texture.id(), image_rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
            } else if self.document.is_none() {
                ui.vertical_centered(|ui| {
                    ui.add_space(80.0);
                    ui.heading("Welcome to DiagramCraft");
                    ui.add_space(16.0);
                    ui.label("View Microsoft Visio drawings, stencils, and templates (.vsdx, .vsdm, .vsd).");
                    ui.add_space(24.0);
                    if ui.button(egui::RichText::new("📂 Open Visio Document…").size(18.0)).clicked() {
                        let bridge = self.bridge.clone_ref();
                        let id = self.state.next_id();
                        let answer = bridge.pick_open_blocking(id, false, saf::PICK_TIMEOUT);
                        if let Ok(saf::PickAnswer::Paths(paths)) = answer {
                            if let Some(first) = paths.first() {
                                self.load_file(ui.ctx(), &PathBuf::from(first));
                            }
                        }
                    }
                });
            }
        });
    }
}

fn watch_incoming(bridge: &saf::Bridge, ctx: &egui::Context) {
    static WATCHER: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    if WATCHER.set(()).is_err() {
        return;
    }
    let bridge = bridge.clone_ref();
    let ctx = ctx.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_millis(200));
        if !ALIVE.load(Ordering::Relaxed) {
            break;
        }
        match bridge.intent_pending() {
            Ok(pending) if pending > 0 => ctx.request_repaint(),
            Ok(_) => {}
            Err(_) => break,
        }
    });
}

fn drain_incoming(bridge: &saf::Bridge, state: &SafState, warmup: &mut i64) -> Option<String> {
    if *warmup > 0 {
        *warmup -= 1;
        return None;
    }
    if *warmup < 0 {
        return None;
    }
    for _ in 0..16 {
        match bridge.intent_poll() {
            Ok(Some(path)) => {
                log::info!("DiagramCraft incoming file: {path}");
                state.mirror.track(0, &path);
                return Some(path);
            }
            Ok(None) => break,
            Err(_) => {
                *warmup = -1;
                break;
            }
        }
    }
    None
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(android_app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("com.artcraft.diagramcraft"),
    );
    log::info!("{SAF_BRIDGE_MARKER} ready ({ANDROID_SHELL_MARKER})");

    egui_winit::android_activity::set_activity(android_app.activity_as_ptr());
    egui_winit::android_activity::set_vm(android_app.vm_as_ptr());
    install_persist_env(&android_app);
    system_fonts::install_font_env(&android_app);

    let bridge = saf::Bridge::new(&android_app);
    bridge.request_storage_permission();
    let state = Arc::new(SafState::default());

    let mut native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("DiagramCraft")
            .with_drag_and_drop(true),
        renderer: eframe::Renderer::Wgpu,
        android_app: Some(android_app.clone()),
        ..Default::default()
    };
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut native.wgpu_options.wgpu_setup {
        create.instance_descriptor.backends = eframe::wgpu::Backends::VULKAN | eframe::wgpu::Backends::GL;
    }

    let app_bridge = bridge.clone_ref();
    let result = eframe::run_native(
        "DiagramCraft",
        native,
        Box::new(move |_cc| {
            Ok(Box::new(DiagramCraftApp::new(
                app_bridge.android_app().clone(),
                app_bridge.clone_ref(),
                state,
            )))
        }),
    );
    if let Err(error) = result {
        log::error!("DiagramCraft Android exited with error: {error}");
    }
}

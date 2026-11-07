//! Builder-owned Android shell for DiagramCraft.
//!
//! ARTCRAFT_DIAGRAMCRAFT_SAF_BRIDGE_V1

use android_activity::AndroidApp;
use dc_ui::app::App as DcApp;
use std::path::Path;
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
    /// Pending file path to load when we have CreationContext
    pending_path: std::sync::Mutex<Option<String>>,
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
    dc_app: DcApp,
}

impl DiagramCraftApp {
    fn new(app: AndroidApp, bridge: saf::Bridge, state: Arc<SafState>, dc_app: DcApp) -> Self {
        Self {
            app,
            bridge,
            state,
            modifiers: egui::Modifiers::default(),
            intent_warmup: 2,
            dc_app,
        }
    }

    fn load_file(&mut self, path: &Path) {
        log::info!("Loading Visio document: {}", path.display());
        // Load into the running app so the theme, recents and stencils survive opening a file.
        self.dc_app.open_document(path.to_path_buf());
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
            // Store the path to load in the next ui() call when we have access to the frame
            *self.state.pending_path.lock().unwrap() = Some(path);
            ctx.request_repaint();
        }
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        ensure_screen_rect(&self.app, ctx, raw_input);
        apply_android_modifiers(&mut self.modifiers, raw_input);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        system_fonts::install_system_fonts(ui.ctx());

        // Check if there's a pending file to load
        if let Some(path) = {
            let mut guard = self.state.pending_path.lock().unwrap();
            guard.take()
        } {
            self.load_file(Path::new(&path));
        }

        // Delegate to the upstream dc-ui App (start screen when no document is open).
        self.dc_app.ui(ui, frame);
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

    // Use android-activity directly (from the vendored crate)
    android_activity::set_nonblocking_callbacks(true);
    install_persist_env(&android_app);
    system_fonts::install_font_env(&android_app);

    // Publish Activity and VM for egui-winit bridges
    #[cfg(target_os = "android")]
    {
        egui_winit::android_activity::set_activity(android_app.activity_as_ptr());
        egui_winit::android_activity::set_vm(android_app.vm_as_ptr());
    }

    let bridge = saf::Bridge::new(&android_app);
    bridge.request_storage_permission();
    let state = Arc::new(SafState::default());

    // Route dc-ui's blocking `rfd::FileDialog` calls (File→Open, Save As, Open Stencil, Insert
    // Picture, hyperlink browse) through Android's system Storage Access Framework pickers. dc-ui
    // runs the dialog on a worker thread, so the picker answer returns to that same thread. Do NOT
    // also drain picker answers in the frame loop: `wait_answer` owns the JNI result queue, and a
    // second reader would steal the answer.
    let launcher_bridge = bridge.clone_ref();
    let launcher_state = state.clone();
    rfd::artcraft_saf::install_blocking(Arc::new(move |kind| {
        let id = launcher_state.next_id();
        let answer = match kind {
            rfd::artcraft_saf::Kind::Open { multiple } => {
                launcher_bridge.pick_open_blocking(id, multiple, saf::PICK_TIMEOUT)
            }
            rfd::artcraft_saf::Kind::Save(name) => {
                launcher_bridge.pick_blocking(id, Some(&name), saf::PICK_TIMEOUT)
            }
            rfd::artcraft_saf::Kind::Folder => {
                launcher_bridge.pick_folder_blocking(id, saf::PICK_TIMEOUT)
            }
        };
        match answer {
            Ok(saf::PickAnswer::Paths(paths)) => {
                // Watch the staged destination for app writes and mirror them to the chosen SAF
                // document (SAF only grants access to its own URI, not the private staging file).
                for path in &paths {
                    launcher_state.mirror.track(id, path);
                }
                Some(paths.into_iter().map(std::path::PathBuf::from).collect())
            }
            Ok(saf::PickAnswer::Cancelled) => None,
            Err(error) => {
                log::error!("Android file picker: {error}");
                None
            }
        }
    }));

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
    let app_state = state.clone();
    let result = eframe::run_native(
        "DiagramCraft",
        native,
        Box::new(move |cc| {
            let dc_app = DcApp::new(cc, None);
            let app = DiagramCraftApp::new(
                app_bridge.android_app().clone(),
                app_bridge.clone_ref(),
                app_state,
                dc_app,
            );
            Ok(Box::new(app))
        }),
    );
    if let Err(error) = result {
        log::error!("DiagramCraft Android exited with error: {error}");
    }
}
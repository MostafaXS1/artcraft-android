//! Builder-owned Android shell for DiagramCraft.
//!
//! ARTCRAFT_DIAGRAMCRAFT_SAF_BRIDGE_V1

use android_activity::AndroidApp;
use dc_ui::app::App as DcApp;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

mod system_fonts;
mod gpu_fallback;
mod mobile_text_bar;
mod touch_gestures;
mod saf;
mod mcp_http;

const SAF_BRIDGE_MARKER: &str = "ARTCRAFT_DIAGRAMCRAFT_SAF_BRIDGE_V1";
const ANDROID_SHELL_MARKER: &str = "ARTCRAFT_DIAGRAMCRAFT_ANDROID_SHELL_V1";

static ALIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

fn migrate_dir_contents(src: &Path, dst: &Path) {
    if !src.is_dir() {
        return;
    }
    let _ = std::fs::create_dir_all(dst);
    if let Ok(entries) = std::fs::read_dir(src) {
        for entry in entries.flatten() {
            let src_path = entry.path();
            let file_name = entry.file_name();
            let dst_path = dst.join(file_name);
            if src_path.is_dir() {
                migrate_dir_contents(&src_path, &dst_path);
                let _ = std::fs::remove_dir(&src_path);
            } else if src_path.is_file() {
                if std::fs::rename(&src_path, &dst_path).is_err() {
                    if std::fs::copy(&src_path, &dst_path).is_ok() {
                        let _ = std::fs::remove_file(&src_path);
                    }
                }
            }
        }
    }
    let _ = std::fs::remove_dir(src);
}

fn resolve_media_dir(app: &AndroidApp) -> Option<PathBuf> {
    if let Some(path) = app.external_data_path() {
        let s = path.to_string_lossy();
        if s.contains("/Android/data/") {
            let replaced = s.replace("/Android/data/", "/Android/media/");
            let media_path = PathBuf::from(replaced);
            let _ = std::fs::create_dir_all(&media_path);
            if path.exists() && path != media_path {
                migrate_dir_contents(&path, &media_path);
            }
            return Some(media_path);
        }
        return Some(path);
    }
    app.internal_data_path()
}

#[allow(unused_unsafe)]
fn install_persist_env(app: &AndroidApp) {
    let Some(base) = resolve_media_dir(app) else {
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
            egui::Event::ModifiersChanged(m) => {
                state.shift = m.shift;
                state.ctrl = m.ctrl;
                state.alt = m.alt;
                state.mac_cmd = m.mac_cmd;
                state.command = m.command || m.ctrl;
                kept.push(event);
            }
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
                if matches!(*key, egui::Key::Backspace | egui::Key::Delete) {
                    *modifiers = egui::Modifiers::default();
                } else {
                    modifiers.shift |= state.shift;
                    modifiers.ctrl |= state.ctrl;
                    modifiers.alt |= state.alt;
                    modifiers.command |= state.command;
                }
                kept.push(event);
            }
            egui::Event::PointerButton { modifiers, .. } => {
                modifiers.shift |= state.shift;
                modifiers.ctrl |= state.ctrl;
                modifiers.alt |= state.alt;
                modifiers.command |= state.command;
                kept.push(event);
            }
            egui::Event::MouseWheel { modifiers, .. } => {
                modifiers.shift |= state.shift;
                modifiers.ctrl |= state.ctrl;
                modifiers.alt |= state.alt;
                modifiers.command |= state.command;
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
    _mcp: Option<mcp_http::Endpoint>,
}

impl DiagramCraftApp {
    fn new(app: AndroidApp, bridge: saf::Bridge, state: Arc<SafState>, dc_app: DcApp) -> Self {
        let _mcp = mcp_http::start_mcp_service(
            "DiagramCraft".to_string(),
            "0.1.0".to_string(),
            Arc::new(|| ALIVE.load(Ordering::Relaxed)),
        );
        Self {
            app,
            bridge,
            state,
            modifiers: egui::Modifiers::default(),
            intent_warmup: 2,
            dc_app,
            _mcp,
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
        mobile_text_bar::process_raw_input(ctx, raw_input);
        touch_gestures::process_raw_input(ctx, raw_input);
        ensure_screen_rect(&self.app, ctx, raw_input);
        apply_android_modifiers(&mut self.modifiers, raw_input);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        gpu_fallback::mark_ok();
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
        mobile_text_bar::show_text_bar(ui.ctx());
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


fn device_is_high_density(app: &AndroidApp) -> Option<bool> {
    use jni::{jni_sig, jni_str, JavaVM, objects::JObject};
    let jvm = unsafe { JavaVM::from_raw(app.vm_as_ptr() as *mut _) };
    let activity_raw = app.activity_as_ptr() as jni::sys::jobject;
    let result = jvm.attach_current_thread(|env| -> jni::errors::Result<bool> {
        let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
        Ok(env
            .call_method(&*activity, jni_str!("isHighDensityDevice"), jni_sig!("()Z"), &[])?
            .z()?)
    });
    match result {
        Ok(is_dense) => Some(is_dense),
        Err(error) => {
            log::warn!("Could not determine screen density: {error}");
            None
        }
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(android_app: AndroidApp) {
    #[allow(unused_unsafe)]
    unsafe {
        std::env::set_var("RUST_BACKTRACE", "1");
        if let Some(path) = android_app.internal_data_path().or_else(|| android_app.external_data_path()) {
            let tmp = path.join("tmp");
            let _ = std::fs::create_dir_all(&tmp);
            std::env::set_var("TMPDIR", &tmp);
        }
    }
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
    let gpu_state_dir = android_app.internal_data_path().or_else(|| android_app.external_data_path()).unwrap_or_else(|| std::path::PathBuf::from("."));

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
    native.wgpu_options.surface.desired_maximum_frame_latency = Some(2);
    let gpu_guard = gpu_fallback::configure_wgpu(&mut native, &gpu_state_dir);
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut native.wgpu_options.wgpu_setup {
        create.instance_descriptor.flags = eframe::wgpu::InstanceFlags::empty();
        create.power_preference = eframe::wgpu::PowerPreference::HighPerformance;
        create.device_descriptor = std::sync::Arc::new(|adapter: &eframe::wgpu::Adapter| {
            let supported = adapter.limits();
            let base = if adapter.get_info().backend == eframe::wgpu::Backend::Gl {
                eframe::wgpu::Limits::downlevel_webgl2_defaults()
            } else {
                eframe::wgpu::Limits::default()
            };
            let base = base.or_worse_values_from(&supported);
            eframe::wgpu::DeviceDescriptor {
                label: Some("wgpu device"),
                required_limits: eframe::wgpu::Limits {
                    max_texture_dimension_2d: supported.max_texture_dimension_2d.min(8192),
                    max_buffer_size: supported.max_buffer_size,
                    ..base
                },
                ..Default::default()
            }
        });
    }

    let app_bridge = bridge.clone_ref();
    let app_state = state.clone();
    let result = eframe::run_native(
        "DiagramCraft",
        native,
        Box::new(move |cc| {
            if device_is_high_density(app_bridge.android_app()) == Some(true) {
                cc.egui_ctx.set_zoom_factor(0.75);
            }
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
        gpu_guard.record_error();
    }
}
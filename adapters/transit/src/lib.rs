//! Builder-owned Android shell for Transit.
//!
//! ARTCRAFT_TRANSIT_SAF_BRIDGE_V1

use android_activity::AndroidApp;
use eframe::App as EframeApp;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use transit_ui_studio::{Services, StudioApp};

mod system_fonts;
mod saf;
mod mcp_http;

pub static CONTROL_TX: std::sync::OnceLock<std::sync::mpsc::Sender<transit_ui_studio::ControlRequest>> = std::sync::OnceLock::new();
pub static EGUI_CTX: std::sync::Mutex<Option<egui::Context>> = std::sync::Mutex::new(None);

const SAF_BRIDGE_MARKER: &str = "ARTCRAFT_TRANSIT_SAF_BRIDGE_V1";
const ANDROID_SHELL_MARKER: &str = "ARTCRAFT_TRANSIT_ANDROID_SHELL_V1";

static ALIVE: AtomicBool = AtomicBool::new(true);

#[allow(unused_unsafe)]
fn install_persist_env(app: &AndroidApp) -> Option<PathBuf> {
    let base = app.external_data_path().or_else(|| app.internal_data_path())?;
    let config = base.join("config");
    let cache = base.join("cache");
    let _ = std::fs::create_dir_all(&config);
    let _ = std::fs::create_dir_all(&cache);
    unsafe {
        std::env::set_var("HOME", &base);
        std::env::set_var("APPDATA", &config);
        std::env::set_var("XDG_DATA_HOME", &config);
        std::env::set_var("XDG_CONFIG_HOME", &config);
        std::env::set_var("XDG_CACHE_HOME", &cache);
    }
    Some(base)
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
}

#[derive(Default)]
pub struct SafState {
    next_id: AtomicU64,
    mirror: saf::SaveMirror,
}

impl SafState {
    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed).wrapping_add(1)
    }
}

fn saf_poll(bridge: &saf::Bridge, state: &SafState) {
    for _ in 0..16 {
        let raw = match bridge.poll() {
            Ok(Some(raw)) => raw,
            Ok(None) => break,
            Err(error) => {
                log::error!("Could not poll Android file picker: {error}");
                break;
            }
        };
        let value: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let Some(id) = value.get("id").and_then(serde_json::Value::as_u64) else { continue };
        let status = value.get("status").and_then(serde_json::Value::as_str).unwrap_or("error");
        let paths: Option<Vec<PathBuf>> = if status == "ok" {
            Some(saf::picker_paths(&value).into_iter().map(PathBuf::from).collect())
        } else {
            None
        };
        if let Some(ref paths) = paths {
            for path in paths {
                state.mirror.track(id, &path.to_string_lossy());
            }
        }
        rfd::artcraft_saf::deliver(id, paths);
    }
    state.mirror.sync(bridge);
}

fn install_saf_launcher(bridge: saf::Bridge, state: Arc<SafState>) {
    let launcher_bridge = bridge.clone_ref();
    rfd::artcraft_saf::install(Arc::new(move |id, kind| {
        match kind {
            rfd::artcraft_saf::Kind::Open { multiple } => {
                launcher_bridge.request_open(id, multiple).ok();
            }
            rfd::artcraft_saf::Kind::Save(ref name) => {
                launcher_bridge.request_save(id, name).ok();
            }
            rfd::artcraft_saf::Kind::Folder => {
                launcher_bridge.request_folder(id).ok();
            }
        }
    }));

    let blocking_bridge = bridge.clone_ref();
    let blocking_state = Arc::clone(&state);
    rfd::artcraft_saf::install_blocking(Arc::new(move |kind| {
        let id = blocking_state.next_id();
        let answer = match kind {
            rfd::artcraft_saf::Kind::Open { multiple } => {
                let answer = blocking_bridge.pick_open_blocking(id, multiple, saf::PICK_TIMEOUT);
                if let Ok(saf::PickAnswer::Paths(ref paths)) = answer {
                    for path in paths {
                        blocking_state.mirror.track(id, path);
                    }
                }
                answer
            }
            rfd::artcraft_saf::Kind::Save(ref name) => {
                let answer = blocking_bridge.pick_blocking(id, Some(name), saf::PICK_TIMEOUT);
                if let Ok(saf::PickAnswer::Paths(ref paths)) = answer {
                    if let Some(path) = paths.first() {
                        blocking_state.mirror.track(id, path);
                    }
                }
                answer
            }
            rfd::artcraft_saf::Kind::Folder => blocking_bridge.pick_folder_blocking(id, saf::PICK_TIMEOUT),
        };
        match answer {
            Ok(saf::PickAnswer::Paths(paths)) => Some(paths.into_iter().map(PathBuf::from).collect()),
            Ok(saf::PickAnswer::Cancelled) => None,
            Err(error) => {
                log::error!("Transit Android file picker: {error}");
                None
            }
        }
    }));
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

fn drain_incoming(bridge: &saf::Bridge, state: &SafState, warmup: &mut i64) {
    if *warmup > 0 {
        *warmup -= 1;
        return;
    }
    if *warmup < 0 {
        return;
    }
    for _ in 0..16 {
        match bridge.intent_poll() {
            Ok(Some(path)) => {
                log::info!("Transit received file: {path}");
                state.mirror.track(0, &path);
            }
            Ok(None) => break,
            Err(_) => {
                *warmup = -1;
                break;
            }
        }
    }
}

struct TransitAndroidApp {
    inner: StudioApp,
    app: AndroidApp,
    bridge: saf::Bridge,
    state: Arc<SafState>,
    modifiers: egui::Modifiers,
    intent_warmup: i64,
    _mcp: Option<mcp_http::Endpoint>,
}

impl Drop for TransitAndroidApp {
    fn drop(&mut self) {
        ALIVE.store(false, Ordering::Relaxed);
    }
}

impl EframeApp for TransitAndroidApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Ok(mut g) = EGUI_CTX.lock() {
            if g.is_none() {
                *g = Some(ctx.clone());
            }
        }
        saf_poll(&self.bridge, &self.state);
        watch_incoming(&self.bridge, ctx);
        drain_incoming(&self.bridge, &self.state, &mut self.intent_warmup);
        self.inner.logic(ctx);
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        ensure_screen_rect(&self.app, ctx, raw_input);
        apply_android_modifiers(&mut self.modifiers, raw_input);
        self.inner.raw_input_hook(raw_input);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        system_fonts::install_system_fonts(ui.ctx());
        self.inner.ui(ui);
    }

    fn on_exit(&mut self) {
        self.state.mirror.sync(&self.bridge);
        self.inner.shut_down();
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
    std::panic::set_hook(Box::new(|info| {
        log::error!("Transit panic: {info}");
    }));

    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("com.artcraft.transit"),
    );
    log::info!("{SAF_BRIDGE_MARKER} ready ({ANDROID_SHELL_MARKER})");

    egui_winit::android_activity::set_activity(android_app.activity_as_ptr());
    egui_winit::android_activity::set_vm(android_app.vm_as_ptr());
    let config_dir = install_persist_env(&android_app);
    system_fonts::install_font_env(&android_app);

    let bridge = saf::Bridge::new(&android_app);
    bridge.request_storage_permission();
    let state = Arc::new(SafState::default());
    install_saf_launcher(bridge.clone_ref(), Arc::clone(&state));

    let mut native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Transit")
            .with_drag_and_drop(true),
        renderer: eframe::Renderer::Wgpu,
        android_app: Some(android_app.clone()),
        ..Default::default()
    };
    native.wgpu_options.surface.desired_maximum_frame_latency = Some(2);
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut native.wgpu_options.wgpu_setup {
        create.instance_descriptor.backends = eframe::wgpu::Backends::VULKAN | eframe::wgpu::Backends::GL;
        create.power_preference = eframe::wgpu::PowerPreference::HighPerformance;
    }

    let open_bridge = bridge.clone_ref();
    let open_state = Arc::clone(&state);
    let save_bridge = bridge.clone_ref();
    let save_state = Arc::clone(&state);

    let services = Services {
        pick_open: Some(Box::new(move |_purpose: &str, _folder: Option<&Path>| {
            let id = open_state.next_id();
            match open_bridge.pick_blocking(id, None, saf::PICK_TIMEOUT) {
                Ok(saf::PickAnswer::Paths(paths)) => {
                    let path = paths.into_iter().next();
                    if let Some(path) = &path {
                        open_state.mirror.track(id, path);
                    }
                    path
                }
                _ => None,
            }
        })),
        pick_save: Some(Box::new(move |name: &str| {
            let id = save_state.next_id();
            match save_bridge.pick_blocking(id, Some(name), saf::PICK_TIMEOUT) {
                Ok(saf::PickAnswer::Paths(paths)) => {
                    let path = paths.into_iter().next();
                    if let Some(path) = &path {
                        save_state.mirror.track(id, path);
                    }
                    path
                }
                _ => None,
            }
        })),
        confirm: None,
        config_dir,
        open_audio: None,
        update_check: None,
        update_off_by_env: true,
        log: None,
    };

    let app_bridge = bridge.clone_ref();
    let result = eframe::run_native(
        "Transit",
        native,
        Box::new(move |_cc| {
            let (control_tx, control_rx) = std::sync::mpsc::channel();
            let _ = CONTROL_TX.set(control_tx);
            let app = StudioApp::new(transit_engine::Engine::default(), None, services).with_control(control_rx);
            let _mcp = mcp_http::start_mcp_service(
                "Transit".to_string(),
                "0.6.2".to_string(),
                Arc::new(|| ALIVE.load(Ordering::Relaxed)),
            );
            Ok(Box::new(TransitAndroidApp {
                inner: app,
                app: app_bridge.android_app().clone(),
                state: Arc::clone(&state),
                modifiers: egui::Modifiers::default(),
                intent_warmup: 2,
                bridge: app_bridge.clone_ref(),
                _mcp,
            }))
        }),
    );
    if let Err(error) = result {
        log::error!("Transit Android exited with error: {error}");
    }
}

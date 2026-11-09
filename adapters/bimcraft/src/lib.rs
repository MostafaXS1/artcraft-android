//! Builder-owned Android shell for BIMCraft.
//!
//! ARTCRAFT_BIMCRAFT_SAF_BRIDGE_V1

use android_activity::AndroidApp;
use bimcraft_core::engine::Session;
use bimcraft_ui_egui::BimApp;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

mod system_fonts;
mod saf;
mod mcp_http;

const SAF_BRIDGE_MARKER: &str = "ARTCRAFT_BIMCRAFT_SAF_BRIDGE_V1";
const ANDROID_SHELL_MARKER: &str = "ARTCRAFT_BIMCRAFT_ANDROID_SHELL_V1";

static ALIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

#[allow(unused_unsafe)]
fn install_persist_env(app: &AndroidApp) {
    let Some(base) = app.external_data_path().or_else(|| app.internal_data_path()) else {
        return;
    };
    let config = base.join("config");
    let cache = base.join("cache");
    let projects = base.join("projects");
    let _ = std::fs::create_dir_all(&config);
    let _ = std::fs::create_dir_all(&cache);
    let _ = std::fs::create_dir_all(&projects);
    unsafe {
        std::env::set_var("HOME", &base);
        std::env::set_var("XDG_CONFIG_HOME", &config);
        std::env::set_var("XDG_CACHE_HOME", &cache);
        std::env::set_var("BIMCRAFT_PROJECTS_DIR", &projects);
        std::env::set_var("BIMCRAFT_NO_UPDATE_CHECK", "1");
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
                log::error!("BIMCraft Android file picker: {error}");
                None
            }
        }
    }));
}

struct BIMCraftAndroidApp {
    inner: BimApp,
    app: AndroidApp,
    bridge: saf::Bridge,
    state: Arc<SafState>,
    modifiers: egui::Modifiers,
    intent_warmup: i64,
    _mcp: Option<mcp_http::Endpoint>,
}

impl Drop for BIMCraftAndroidApp {
    fn drop(&mut self) {
        ALIVE.store(false, Ordering::Relaxed);
    }
}

impl eframe::App for BIMCraftAndroidApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        saf_poll(&self.bridge, &self.state);
        watch_incoming(&self.bridge, ctx);
        drain_incoming(&self.bridge, &self.state, &mut self.inner, &mut self.intent_warmup);
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        ensure_screen_rect(&self.app, ctx, raw_input);
        apply_android_modifiers(&mut self.modifiers, raw_input);
        self.inner.raw_input_hook(ctx, raw_input);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        system_fonts::install_system_fonts(ui.ctx());
        self.inner.ui(ui, _frame);
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

fn drain_incoming(bridge: &saf::Bridge, state: &SafState, app: &mut BimApp, warmup: &mut i64) {
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
                log::info!("BIMCraft received file: {path}");
                state.mirror.track(0, &path);
                let p = PathBuf::from(&path);
                if bimcraft_ui_egui::revit::is_revit(&p) {
                    app.open_later(p);
                } else {
                    let _ = app.session.execute("open", &serde_json::json!({ "path": path }));
                }
            }
            Ok(None) => break,
            Err(_) => {
                *warmup = -1;
                break;
            }
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
    std::panic::set_hook(Box::new(|info| {
        log::error!("BIMCraft panic: {info}");
    }));

    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("com.artcraft.bimcraft"),
    );
    log::info!("{SAF_BRIDGE_MARKER} ready ({ANDROID_SHELL_MARKER})");

    egui_winit::android_activity::set_activity(android_app.activity_as_ptr());
    egui_winit::android_activity::set_vm(android_app.vm_as_ptr());
    install_persist_env(&android_app);
    system_fonts::install_font_env(&android_app);

    let bridge = saf::Bridge::new(&android_app);
    bridge.request_storage_permission();
    let state = Arc::new(SafState::default());
    install_saf_launcher(bridge.clone_ref(), Arc::clone(&state));

    let mut native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("BIMCraft")
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

    let app_bridge = bridge.clone_ref();
    let result = eframe::run_native(
        "BIMCraft",
        native,
        Box::new(move |_cc| {
            let mut session = Session::new();
            #[cfg(feature = "dwg")]
            {
                session.dwg = bimcraft_dwg::reader();
            }

            #[cfg(feature = "rvt")]
            let revit_reader: Option<bimcraft_ui_egui::revit::Importer> = Some(std::sync::Arc::new(|path: &Path| {
                let stem = path.file_stem().map_or_else(|| "Project".into(), |s| s.to_string_lossy().into_owned());
                bimcraft_rvt::import_path(path, &stem).map_err(|e| e.to_string())
            }));
            #[cfg(not(feature = "rvt"))]
            let revit_reader: Option<bimcraft_ui_egui::revit::Importer> = None;

            let mut app = BimApp::new(session);
            app.revit = revit_reader;
            app.show_start_page();

            let _mcp = mcp_http::start_mcp_service(
                "BIMCraft".to_string(),
                "0.11.0".to_string(),
                std::sync::Arc::new(|| ALIVE.load(Ordering::Relaxed)),
            );

            Ok(Box::new(BIMCraftAndroidApp {
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
        log::error!("BIMCraft Android exited with error: {error}");
    }
}

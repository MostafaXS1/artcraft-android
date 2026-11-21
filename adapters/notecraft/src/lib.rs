//! Builder-owned Android shell for NoteCraft.
//!
//! ARTCRAFT_NOTECRAFT_SAF_BRIDGE_V1

use android_activity::AndroidApp;
use notecraft_engine::Session;
use notecraft_ui_egui::NoteCraftApp;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

mod system_fonts;
mod mobile_text_bar;
mod touch_gestures;
mod saf;
mod mcp_http;

pub static CONTROL_TX: std::sync::Mutex<Option<std::sync::mpsc::Sender<notecraft_ui_egui::control::ControlRequest>>> = std::sync::Mutex::new(None);
pub static EGUI_CTX: std::sync::Mutex<Option<egui::Context>> = std::sync::Mutex::new(None);

const SAF_BRIDGE_MARKER: &str = "ARTCRAFT_NOTECRAFT_SAF_BRIDGE_V1";
const ANDROID_SHELL_MARKER: &str = "ARTCRAFT_NOTECRAFT_ANDROID_SHELL_V1";

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
    let notebooks = base.join("NoteCraft");
    let config = base.join("config");
    let cache = base.join("cache");
    let _ = std::fs::create_dir_all(&notebooks);
    let _ = std::fs::create_dir_all(&config);
    let _ = std::fs::create_dir_all(&cache);
    unsafe {
        std::env::set_var("NOTECRAFT_NOTEBOOKS", &notebooks);
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
}

impl SafState {
    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed).wrapping_add(1)
    }
}

fn install_saf_launcher(bridge: saf::Bridge, state: Arc<SafState>) {
    rfd::artcraft_saf::install_blocking(Arc::new(move |kind| {
        let id = state.next_id();
        let answer = match kind {
            rfd::artcraft_saf::Kind::Open { multiple } => {
                let answer = bridge.pick_open_blocking(id, multiple, saf::PICK_TIMEOUT);
                if let Ok(saf::PickAnswer::Paths(ref paths)) = answer {
                    for path in paths {
                        state.mirror.track(id, path);
                    }
                }
                answer
            }
            rfd::artcraft_saf::Kind::Save(ref name) => {
                let answer = bridge.pick_blocking(id, Some(name), saf::PICK_TIMEOUT);
                if let Ok(saf::PickAnswer::Paths(ref paths)) = answer {
                    if let Some(path) = paths.first() {
                        state.mirror.track(id, path);
                    }
                }
                answer
            }
            rfd::artcraft_saf::Kind::Folder => bridge.pick_folder_blocking(id, saf::PICK_TIMEOUT),
        };
        match answer {
            Ok(saf::PickAnswer::Paths(paths)) => Some(paths.into_iter().map(PathBuf::from).collect()),
            Ok(saf::PickAnswer::Cancelled) => None,
            Err(error) => {
                log::error!("NoteCraft Android file picker: {error}");
                None
            }
        }
    }));
}

struct NoteCraftAndroidApp {
    inner: NoteCraftApp,
    app: AndroidApp,
    bridge: saf::Bridge,
    state: Arc<SafState>,
    modifiers: egui::Modifiers,
    intent_warmup: i64,
    _mcp: Option<mcp_http::Endpoint>,
}

impl Drop for NoteCraftAndroidApp {
    fn drop(&mut self) {
        ALIVE.store(false, Ordering::Relaxed);
    }
}

impl eframe::App for NoteCraftAndroidApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Ok(mut g) = EGUI_CTX.lock() {
            *g = Some(ctx.clone());
        }
        self.state.mirror.sync(&self.bridge);
        watch_incoming(&self.bridge, ctx);
        drain_incoming(&mut self.inner, &self.bridge, &self.state, &mut self.intent_warmup);
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        mobile_text_bar::process_raw_input(ctx, raw_input);
        touch_gestures::process_raw_input(ctx, raw_input);
        ensure_screen_rect(&self.app, ctx, raw_input);
        apply_android_modifiers(&mut self.modifiers, raw_input);

        let mut keyboard_paste = false;
        raw_input.events.retain(|event| {
            let is_paste = matches!(
                event,
                egui::Event::Key { key: egui::Key::V, pressed: true, modifiers, .. }
                    if modifiers.ctrl || modifiers.command
            ) || matches!(event, egui::Event::PasteImage(_));
            keyboard_paste |= is_paste;
            !is_paste
        });
        if keyboard_paste {
            self.inner.queue_keyboard_paste();
        }
        raw_input.events.extend(self.inner.take_events());
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        system_fonts::install_system_fonts(ui.ctx());
        self.inner.show(ui);
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

fn drain_incoming(app: &mut NoteCraftApp, bridge: &saf::Bridge, state: &SafState, warmup: &mut i64) {
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
                log::info!("NoteCraft opening incoming file: {path}");
                state.mirror.track(0, &path);
                app.open_path(&path);
            }
            Ok(None) => break,
            Err(_) => {
                *warmup = -1;
                break;
            }
        }
    }
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
            .with_tag("com.artcraft.notecraft"),
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

    let clipboard: Box<dyn notecraft_ui_egui::clipboard::Board> = match notecraft_ui_egui::clipboard::open() {
        Ok(board) => board,
        Err(_) => Box::new(notecraft_ui_egui::clipboard::Unavailable),
    };

    let mut native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("NoteCraft")
            .with_drag_and_drop(true),
        renderer: eframe::Renderer::Wgpu,
        android_app: Some(android_app.clone()),
        ..Default::default()
    };
    native.wgpu_options.surface.desired_maximum_frame_latency = Some(2);
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut native.wgpu_options.wgpu_setup {
        create.instance_descriptor.backends = eframe::wgpu::Backends::VULKAN | eframe::wgpu::Backends::GL;
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
    let result = eframe::run_native(
        "NoteCraft",
        native,
        Box::new(move |cc| {
            if device_is_high_density(app_bridge.android_app()) == Some(true) {
                cc.egui_ctx.set_zoom_factor(0.75);
            }
            if let Ok(mut g) = EGUI_CTX.lock() { *g = Some(cc.egui_ctx.clone()); }
            let internal_dir = app_bridge.android_app().internal_data_path().unwrap_or_else(|| PathBuf::from("/data/data/com.artcraft.notecraft/files"));
            let notebooks = match std::env::var("NOTECRAFT_NOTEBOOKS") {
                Ok(p) => PathBuf::from(p),
                Err(_) => internal_dir.join("NoteCraft"),
            };
            let _ = std::fs::create_dir_all(&notebooks);
            let session = Session::new(&notebooks, notecraft_store::settings::Settings::default_location())
                .map_err(|e| Box::<dyn std::error::Error + Send + Sync>::from(format!("Failed to initialize NoteCraft session: {e}")))?;
            let (control_tx, control_rx) = std::sync::mpsc::channel();
            if let Ok(mut g) = CONTROL_TX.lock() { *g = Some(control_tx); }
            let control_server = notecraft_ui_egui::control::ControlServer {
                port: 0,
                key_file: PathBuf::new(),
                key: String::new(),
            };
            let inner = NoteCraftApp::new(session)
                .with_clipboard(clipboard)
                .with_control(control_server, control_rx);
            let _mcp = mcp_http::start_mcp_service(
                "NoteCraft".to_string(),
                "0.0.9".to_string(),
                Arc::new(|| ALIVE.load(Ordering::Relaxed)),
            );
            Ok(Box::new(NoteCraftAndroidApp {
                inner,
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
        log::error!("NoteCraft Android exited with error: {error}");
    }
}

//! Builder-owned Android shell for HumanMotionCraft.
//!
//! ARTCRAFT_HUMANMOTIONCRAFT_SAF_BRIDGE_V1

use android_activity::AndroidApp;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

mod system_fonts;
mod saf;
mod mcp_http;

const SAF_BRIDGE_MARKER: &str = "ARTCRAFT_HUMANMOTIONCRAFT_SAF_BRIDGE_V1";
const ANDROID_SHELL_MARKER: &str = "ARTCRAFT_HUMANMOTIONCRAFT_ANDROID_SHELL_V1";

static ALIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

#[allow(unused_unsafe)]
fn install_persist_env(app: &AndroidApp) {
    let Some(base) = app.external_data_path().or_else(|| app.internal_data_path()) else {
        return;
    };
    let projects = base.join("projects");
    let library = base.join("library");
    let config = base.join("config");
    let cache = base.join("cache");
    let _ = std::fs::create_dir_all(&projects);
    let _ = std::fs::create_dir_all(&library);
    let _ = std::fs::create_dir_all(&config);
    let _ = std::fs::create_dir_all(&cache);
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", &config);
        std::env::set_var("XDG_CACHE_HOME", &cache);
        std::env::set_var("AC_LIBRARY_DIR", &library);
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
                log::error!("HumanMotionCraft Android file picker error: {error}");
                None
            }
        }
    }));
}

struct HumanMotionCraftAndroidApp {
    inner: ac_app::App,
    app: AndroidApp,
    bridge: saf::Bridge,
    state: Arc<SafState>,
    modifiers: egui::Modifiers,
    _mcp: Option<mcp_http::Endpoint>,
}

impl Drop for HumanMotionCraftAndroidApp {
    fn drop(&mut self) {
        ALIVE.store(false, Ordering::Relaxed);
    }
}

impl eframe::App for HumanMotionCraftAndroidApp {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.state.mirror.sync(&self.bridge);
        self.inner.logic(ctx, frame);
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        ensure_screen_rect(&self.app, ctx, raw_input);
        apply_android_modifiers(&mut self.modifiers, raw_input);
        self.inner.raw_input_hook(ctx, raw_input);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        system_fonts::install_system_fonts(ui.ctx());
        self.inner.ui(ui, frame);
    }

    fn on_exit(&mut self) {
        self.inner.on_exit();
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(android_app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("com.artcraft.humanmotioncraft"),
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

    let internal_dir = android_app.internal_data_path().unwrap_or_else(|| PathBuf::from("/data/data/com.artcraft.humanmotioncraft/files"));
    let assets_dir = internal_dir.join("assets");
    let library_dir = internal_dir.join("library");
    let _ = std::fs::create_dir_all(&library_dir);

    let mut args = ac_app::Args::default();
    args.assets = Some(assets_dir);
    args.library = Some(library_dir);

    let mut native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(ac_core::product::NAME)
            .with_drag_and_drop(true),
        renderer: eframe::Renderer::Wgpu,
        depth_buffer: 0,
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
        ac_core::product::NAME,
        native,
        Box::new(move |cc| {
            let inner = ac_app::App::new(cc, args).map_err(|e| Box::<dyn std::error::Error + Send + Sync>::from(e))?;
            let _mcp = mcp_http::start_mcp_service(
                "HumanMotionCraft".to_string(),
                "0.3.0".to_string(),
                Arc::new(|| ALIVE.load(Ordering::Relaxed)),
            );
            Ok(Box::new(HumanMotionCraftAndroidApp {
                inner,
                app: app_bridge.android_app().clone(),
                bridge: app_bridge.clone_ref(),
                state: Arc::clone(&state),
                modifiers: egui::Modifiers::default(),
                _mcp,
            }))
        }),
    );
    if let Err(error) = result {
        log::error!("HumanMotionCraft Android exited with error: {error}");
    }
}

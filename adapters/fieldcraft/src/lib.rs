//! Builder-owned Android shell for FieldCraft.

use android_activity::AndroidApp;
use fieldcraft_engine::Session;
use fieldcraft_ui_egui::ModelApp;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

mod system_fonts;
mod mobile_text_bar;
mod touch_gestures;
mod mcp_http;
mod saf;

const SAF_BRIDGE_MARKER: &str = "ARTCRAFT_FIELDCRAFT_SAF_BRIDGE_V1";
const ANDROID_SHELL_MARKER: &str = "ARTCRAFT_FIELDCRAFT_ANDROID_SHELL_V1";

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

#[allow(unused_unsafe)]
fn install_persist_env(app: &AndroidApp) -> Option<PathBuf> {
    let base = if let Some(path) = app.external_data_path() {
        let s = path.to_string_lossy();
        if s.contains("/Android/data/") {
            let replaced = s.replace("/Android/data/", "/Android/media/");
            let media_path = PathBuf::from(replaced);
            let _ = std::fs::create_dir_all(&media_path);
            if path.exists() && path != media_path {
                migrate_dir_contents(&path, &media_path);
            }
            media_path
        } else {
            path
        }
    } else if let Some(path) = app.internal_data_path() {
        path
    } else {
        return None;
    };
    let config = base.join("config");
    let cache = base.join("cache");
    let autosave = base.join("autosave");
    let recovery = base.join("recovery");
    let _ = std::fs::create_dir_all(&config);
    let _ = std::fs::create_dir_all(&cache);
    let _ = std::fs::create_dir_all(&autosave);
    let _ = std::fs::create_dir_all(&recovery);
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", &config);
        std::env::set_var("XDG_CACHE_HOME", &cache);
    }
    log::info!("settings directory: {}", config.display());
    log::info!("autosave directory: {}", autosave.display());
    log::info!("recovery directory: {}", recovery.display());
    Some(config)
}

fn apply_immersive_fullscreen(app: &AndroidApp) {
    let app_clone = app.clone();
    app.run_on_java_main_thread(Box::new(move || {
        let jvm = unsafe { jni::JavaVM::from_raw(app_clone.vm_as_ptr() as *mut _) };
        let result = jvm.attach_current_thread(|env| -> jni::errors::Result<()> {
            let activity_raw = app_clone.activity_as_ptr() as jni::sys::jobject;
            let activity = unsafe { env.as_cast_raw::<jni::objects::JObject>(&activity_raw)? };
            let window = env
                .call_method(&*activity, jni::jni_str!("getWindow"), jni::jni_sig!("()Landroid/view/Window;"), &[])?
                .l()?;
            if window.is_null() {
                return Ok(());
            }
            let decor = env
                .call_method(&window, jni::jni_str!("getDecorView"), jni::jni_sig!("()Landroid/view/View;"), &[])?
                .l()?;
            if decor.is_null() {
                return Ok(());
            }
            const IMMERSIVE: i32 =
                0x0000_0100 | 0x0000_0002 | 0x0000_0004 | 0x0000_0200 | 0x0000_0400 | 0x0000_1000;
            const LIGHT_ICON_FLAGS: i32 = 0x2000 | 0x0010;
            let visibility = env
                .call_method(
                    &decor,
                    jni::jni_str!("getSystemUiVisibility"),
                    jni::jni_sig!("()I"),
                    &[],
                )?
                .i()?;
            env.call_method(
                &decor,
                jni::jni_str!("setSystemUiVisibility"),
                jni::jni_sig!("(I)V"),
                &[jni::objects::JValue::Int((visibility & LIGHT_ICON_FLAGS) | IMMERSIVE)],
            )?;
            Ok(())
        });
        if let Err(error) = result {
            log::error!("Could not apply immersive fullscreen: {error}");
        }
    }));
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

fn saf_services(bridge: saf::Bridge, state: Arc<SafState>) -> fieldcraft_ui_egui::Services {
    let open_bridge = bridge.clone_ref();
    let open_state = Arc::clone(&state);
    let save_bridge = bridge.clone_ref();
    let save_state = Arc::clone(&state);
    fieldcraft_ui_egui::Services {
        pick_open: Some(Box::new(move |_filters| {
            let id = open_state.next_id();
            match open_bridge.pick_open_blocking(id, false, saf::PICK_TIMEOUT) {
                Ok(saf::PickAnswer::Paths(paths)) => {
                    for path in &paths {
                        open_state.mirror.track(id, path);
                    }
                    paths.into_iter().next()
                }
                _ => None,
            }
        })),
        pick_save: Some(Box::new(move |name, _filters| {
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
        ..Default::default()
    }
}

struct FieldCraftAndroidApp {
    inner: ModelApp,
    app: AndroidApp,
    bridge: saf::Bridge,
    state: Arc<SafState>,
    modifiers: egui::Modifiers,
    intent_warmup: i64,
    _mcp: Option<mcp_http::Endpoint>,
}

impl Drop for FieldCraftAndroidApp {
    fn drop(&mut self) {
        ALIVE.store(false, Ordering::Relaxed);
    }
}

impl eframe::App for FieldCraftAndroidApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.state.mirror.sync(&self.bridge);
        if self.intent_warmup > 0 {
            self.intent_warmup -= 1;
        } else if self.intent_warmup == 0 {
            for _ in 0..16 {
                match self.bridge.intent_poll() {
                    Ok(Some(path)) => {
                        log::info!("Opening incoming file: {path}");
                        self.state.mirror.track(0, &path);
                        self.inner.open_path(&path);
                        ctx.request_repaint();
                    }
                    Ok(None) => break,
                    Err(error) => {
                        log::info!("Intent poll stopped: {error}");
                        self.intent_warmup = -1;
                        break;
                    }
                }
            }
        }
        self.inner.logic(ctx);
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        mobile_text_bar::process_raw_input(ctx, raw);
        touch_gestures::process_raw_input(ctx, raw);
        ensure_screen_rect(&self.app, ctx, raw);
        apply_android_modifiers(&mut self.modifiers, raw);
        if raw.events.iter().any(|event| matches!(event, egui::Event::WindowFocused(false))) {
            self.state.mirror.sync(&self.bridge);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        system_fonts::install_system_fonts(ui.ctx());
        self.inner.ui(ui);
        mobile_text_bar::show_text_bar(ui.ctx());
    }

    fn on_exit(&mut self) {
        self.state.mirror.sync(&self.bridge);
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
            .with_tag("com.artcraft.fieldcraft"),
    );
    log::info!("{ANDROID_SHELL_MARKER} ready ({})", saf::SAF_MARKER);
    apply_immersive_fullscreen(&android_app);
    egui_winit::android_activity::set_activity(android_app.activity_as_ptr());
    egui_winit::android_activity::set_vm(android_app.vm_as_ptr());
    install_persist_env(&android_app);
    system_fonts::install_font_env(&android_app);

    let saf_bridge = saf::Bridge::new(&android_app);
    saf_bridge.request_storage_permission();
    let saf_state = Arc::new(SafState::default());
    let platform_services = saf_services(saf_bridge.clone_ref(), Arc::clone(&saf_state));

    fieldcraft_lbm_gpu::register();
    fieldcraft_mesh_gpu::register();
    fieldcraft_ui_egui::register_picture_drawer();

    let mut options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_title("FieldCraft")
            .with_drag_and_drop(true),
        android_app: Some(android_app.clone()),
        depth_buffer: 24,
        ..Default::default()
    };
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup {
        create.instance_descriptor.backends = eframe::wgpu::Backends::VULKAN | eframe::wgpu::Backends::GL;
        create.instance_descriptor.flags = eframe::wgpu::InstanceFlags::empty();
        create.device_descriptor = std::sync::Arc::new(|adapter: &eframe::wgpu::Adapter| {
            let supported = adapter.limits();
            let base = if adapter.get_info().backend == eframe::wgpu::Backend::Gl {
                eframe::wgpu::Limits::downlevel_webgl2_defaults()
            } else {
                eframe::wgpu::Limits::default()
            };
            let base = base.or_worse_values_from(&supported);
            eframe::wgpu::DeviceDescriptor {
                label: Some("FieldCraft wgpu device"),
                required_limits: eframe::wgpu::Limits {
                    max_texture_dimension_2d: supported.max_texture_dimension_2d.min(8192),
                    max_buffer_size: supported.max_buffer_size,
                    ..base
                },
                ..Default::default()
            }
        });
    }

    let app_bridge = saf_bridge.clone_ref();
    let app_state = saf_state.clone();
    let result = eframe::run_native(
        "FieldCraft",
        options,
        Box::new(move |cc| {
            let session = Session::default();
            let mut inner = ModelApp::new(session).with_services(platform_services);
            inner.custom_titlebar = false;
            inner.integrated_titlebar = false;
            if let Some(rs) = &cc.wgpu_render_state {
                inner.set_wgpu(rs, 24, rs.msaa_samples, &cc.egui_ctx);
            }
            let _mcp = mcp_http::start_mcp_service(
                "FieldCraft".to_string(),
                "0.8.1".to_string(),
                Arc::new(|| ALIVE.load(Ordering::Relaxed)),
            );
            Ok(Box::new(FieldCraftAndroidApp {
                inner,
                app: app_bridge.android_app().clone(),
                bridge: app_bridge,
                state: app_state,
                modifiers: egui::Modifiers::default(),
                intent_warmup: 2,
                _mcp,
            }))
        }),
    );
    if let Err(error) = result {
        log::error!("FieldCraft Android exited with an error: {error}");
    }
}

//! Builder-owned Android shell for RustyPainter.

use android_activity::AndroidApp;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use winit::platform::android::EventLoopBuilderExtAndroid;

mod system_fonts;
mod mobile_text_bar;
mod touch_gestures;
mod mcp_http;
mod saf;

const SAF_BRIDGE_MARKER: &str = "ARTCRAFT_RUSTYPAINTER_SAF_BRIDGE_V1";
const ANDROID_SHELL_MARKER: &str = "ARTCRAFT_RUSTYPAINTER_ANDROID_SHELL_V1";

static ALIVE: AtomicBool = AtomicBool::new(true);

fn install_persist_env(app: &AndroidApp) -> PathBuf {
    let base = if let Some(path) = app.external_data_path() {
        let s = path.to_string_lossy();
        if s.contains("/Android/data/") {
            let replaced = s.replace("/Android/data/", "/Android/media/");
            let media_path = PathBuf::from(replaced);
            let _ = std::fs::create_dir_all(&media_path);
            media_path
        } else {
            path
        }
    } else if let Some(path) = app.internal_data_path() {
        path
    } else {
        PathBuf::from("/data/local/tmp")
    };
    let app_dir = base.join("RustyPainter");
    let config = app_dir.join("config");
    let cache = app_dir.join("cache");
    let autosave = app_dir.join("autosave");
    let recovery = app_dir.join("recovery");
    let _ = std::fs::create_dir_all(&app_dir);
    let _ = std::fs::create_dir_all(&config);
    let _ = std::fs::create_dir_all(&cache);
    let _ = std::fs::create_dir_all(&autosave);
    let _ = std::fs::create_dir_all(&recovery);
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", &config);
        std::env::set_var("XDG_CACHE_HOME", &cache);
        std::env::set_var("XDG_DATA_HOME", &app_dir);
    }
    log::info!("RustyPainter directory: {}", app_dir.display());
    app_dir
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
        if let Err(e) = result {
            log::warn!("apply_immersive_fullscreen failed: {e}");
        }
    }));
}

struct SafState {
    next_id: AtomicU64,
    mirror: saf::SaveMirror,
}

impl SafState {
    fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            mirror: saf::SaveMirror::default(),
        }
    }

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
                log::error!("RustyPainter Android file picker: {error}");
                None
            }
        }
    }));
}

struct RustyPainterAndroidApp {
    inner: Box<dyn eframe::App>,
    bridge: saf::Bridge,
    state: Arc<SafState>,
}

impl eframe::App for RustyPainterAndroidApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        system_fonts::install_system_fonts(ctx);
        saf_poll(&self.bridge, &self.state);
        self.inner.update(ctx, frame);
        mobile_text_bar::show_text_bar(ctx);
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        touch_gestures::process_raw_input(ctx, raw_input);
        mobile_text_bar::process_raw_input(ctx, raw_input);
        self.inner.raw_input_hook(ctx, raw_input);
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub fn android_main(android_app: AndroidApp) {
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
        log::error!("RustyPainter panic: {info}");
    }));

    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("com.artcraft.rustypainter"),
    );
    log::info!("{SAF_BRIDGE_MARKER} ready ({ANDROID_SHELL_MARKER})");

    install_persist_env(&android_app);
    system_fonts::install_font_env(&android_app);
    apply_immersive_fullscreen(&android_app);

    let bridge = saf::Bridge::new(&android_app);
    bridge.request_storage_permission();
    let saf_state = Arc::new(SafState::new());
    install_saf_launcher(bridge.clone_ref(), Arc::clone(&saf_state));

    let app_clone = android_app.clone();
    let mut options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_title("RustyPainter")
            .with_fullscreen(true),
        ..Default::default()
    };
    options.event_loop_builder = Some(Box::new(move |builder| {
        builder.with_android_app(app_clone);
    }));

    let bridge_for_app = bridge.clone();
    let state_for_app = Arc::clone(&saf_state);

    let _ = eframe::run_native(
        "RustyPainter",
        options,
        Box::new(move |cc| {
            if cc.egui_ctx.screen_rect().width() < 600.0 {
                cc.egui_ctx.set_zoom_factor(0.85);
            }
            system_fonts::install_system_fonts(&cc.egui_ctx);
            let inner = Box::new(rusty_painter::PainterApp::new(cc));
            Ok(Box::new(RustyPainterAndroidApp {
                inner,
                bridge: bridge_for_app,
                state: state_for_app,
            }))
        }),
    );
    std::process::exit(0);
}

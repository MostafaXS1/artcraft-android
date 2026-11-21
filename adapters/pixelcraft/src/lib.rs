//! Builder-owned Android shell for PixelCraft.

use android_activity::AndroidApp;
use pixelcraft::PixelCraft;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

mod system_fonts;
mod mobile_text_bar;
mod touch_gestures;
mod mcp_http;
mod saf;

const SAF_BRIDGE_MARKER: &str = "ARTCRAFT_PIXELCRAFT_SAF_BRIDGE_V1";
const ANDROID_SHELL_MARKER: &str = "ARTCRAFT_PIXELCRAFT_ANDROID_SHELL_V1";

static ALIVE: AtomicBool = AtomicBool::new(true);

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
        std::env::set_var("XDG_DATA_HOME", &base);
        std::env::set_var("PIXELCRAFT_HOME", &base);
    }
    log::info!("settings directory: {}", config.display());
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
                log::error!("PixelCraft Android file picker: {error}");
                None
            }
        }
    }));
}

struct PixelCraftAndroidApp {
    inner: PixelCraft,
    bridge: saf::Bridge,
    saf_state: Arc<SafState>,
    _mcp: Option<mcp_http::Endpoint>,
    intent_warmup: i32,
}

impl eframe::App for PixelCraftAndroidApp {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        system_fonts::install_system_fonts(ctx);
        saf_poll(&self.bridge, &self.saf_state);
        if self.intent_warmup > 0 {
            self.intent_warmup -= 1;
        } else if self.intent_warmup == 0 {
            for _ in 0..16 {
                match self.bridge.intent_poll() {
                    Ok(Some(path)) => {
                        log::info!("Opening incoming file: {path}");
                        self.inner.open_recent(PathBuf::from(path));
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
        self.inner.logic(ctx, frame);
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        touch_gestures::process_raw_input(ctx, raw_input);
        mobile_text_bar::process_raw_input(ctx, raw_input);
        self.inner.raw_input_hook(ctx, raw_input);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.inner.ui(ui, frame);
        mobile_text_bar::show_text_bar(ui.ctx());
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(android_app: AndroidApp) {
    #[allow(unused_unsafe)]
    unsafe {
        std::env::set_var("RUST_BACKTRACE", "1");
        let base = android_app.external_data_path().map(|p| {
            let s = p.to_string_lossy();
            if s.contains("/Android/data/") {
                PathBuf::from(s.replace("/Android/data/", "/Android/media/"))
            } else {
                p
            }
        }).or_else(|| android_app.internal_data_path());
        if let Some(path) = base {
            let tmp = path.join("tmp");
            let _ = std::fs::create_dir_all(&tmp);
            std::env::set_var("TMPDIR", &tmp);
        }
    }
    std::panic::set_hook(Box::new(|info| {
        log::error!("PixelCraft panic: {info}");
    }));

    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("com.artcraft.pixelcraft"),
    );
    log::info!("{SAF_BRIDGE_MARKER} ready ({ANDROID_SHELL_MARKER})");

    egui_winit::android_activity::set_activity(android_app.activity_as_ptr());
    egui_winit::android_activity::set_vm(android_app.vm_as_ptr());
    install_persist_env(&android_app);
    system_fonts::install_font_env(&android_app);
    apply_immersive_fullscreen(&android_app);

    let bridge = saf::Bridge::new(&android_app);
    bridge.request_storage_permission();

    let saf_state = Arc::new(SafState::new());
    install_saf_launcher(bridge.clone_ref(), Arc::clone(&saf_state));

    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("PixelCraft"),
        renderer: eframe::Renderer::Glow,
        android_app: Some(android_app.clone()),
        ..Default::default()
    };

    let app_bridge = bridge.clone_ref();
    let app_saf_state = Arc::clone(&saf_state);
    let result = eframe::run_native(
        "PixelCraft",
        native,
        Box::new(move |cc| {
            let inner = PixelCraft::new(cc, None);
            let _mcp = mcp_http::start_mcp_service(
                "PixelCraft".to_string(),
                "0.11.0".to_string(),
                std::sync::Arc::new(|| ALIVE.load(Ordering::Relaxed)),
            );
            Ok(Box::new(PixelCraftAndroidApp {
                inner,
                bridge: app_bridge,
                saf_state: app_saf_state,
                _mcp,
                intent_warmup: 2,
            }))
        }),
    );
    if let Err(error) = result {
        log::error!("PixelCraft Android exited with error: {error}");
    }
}

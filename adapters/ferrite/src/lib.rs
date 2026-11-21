//! Builder-owned Android shell for Ferrite.

use android_activity::AndroidApp;
use ferrite::FerriteApp;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

mod system_fonts;
mod mobile_text_bar;
mod mcp_http;
mod saf;

const SAF_BRIDGE_MARKER: &str = "ARTCRAFT_FERRITE_SAF_BRIDGE_V1";
const ANDROID_SHELL_MARKER: &str = "ARTCRAFT_FERRITE_ANDROID_SHELL_V1";

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

struct FerriteAndroidApp {
    inner: FerriteApp,
    _bridge: saf::Bridge,
    _mcp: Option<mcp_http::Endpoint>,
}

impl eframe::App for FerriteAndroidApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        system_fonts::install_system_fonts(ctx);
        self.inner.update(ctx, frame);
        mobile_text_bar::show_text_bar(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        system_fonts::install_system_fonts(ui.ctx());
        self.inner.ui(ui, frame);
        mobile_text_bar::show_text_bar(ui.ctx());
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        mobile_text_bar::process_raw_input(ctx, raw_input);
        self.inner.raw_input_hook(ctx, raw_input);
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
    std::panic::set_hook(Box::new(|info| {
        log::error!("Ferrite panic: {info}");
    }));

    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("com.artcraft.ferrite"),
    );
    log::info!("{SAF_BRIDGE_MARKER} ready ({ANDROID_SHELL_MARKER})");

    egui_winit::android_activity::set_activity(android_app.activity_as_ptr());
    egui_winit::android_activity::set_vm(android_app.vm_as_ptr());
    install_persist_env(&android_app);
    system_fonts::install_font_env(&android_app);
    apply_immersive_fullscreen(&android_app);

    let bridge = saf::Bridge::new(&android_app);
    bridge.request_storage_permission();

    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Ferrite"),
        renderer: eframe::Renderer::Glow,
        android_app: Some(android_app.clone()),
        ..Default::default()
    };

    let app_bridge = bridge.clone_ref();
    let result = eframe::run_native(
        "Ferrite",
        native,
        Box::new(move |cc| {
            if device_is_high_density(app_bridge.android_app()) == Some(true) {
                cc.egui_ctx.set_zoom_factor(0.75);
            }
            let inner = FerriteApp::new(cc, false);
            let _mcp = mcp_http::start_mcp_service(
                "Ferrite".to_string(),
                "0.3.1".to_string(),
                std::sync::Arc::new(|| ALIVE.load(Ordering::Relaxed)),
            );
            Ok(Box::new(FerriteAndroidApp {
                inner,
                _bridge: app_bridge,
                _mcp,
            }))
        }),
    );
    if let Err(error) = result {
        log::error!("Ferrite Android exited with error: {error}");
    }
}

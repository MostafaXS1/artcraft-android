//! Android eframe bootstrap.

#[cfg(target_os = "android")]
use crate::backend::AndroidBackend;
use crate::bridge::Bridge;
#[cfg(target_os = "android")]
use crate::paths::android_files_dir;
use crate::settings::{load, save};
#[cfg(target_os = "android")]
use android_activity::AndroidApp;
use artcraft_launcher_ui_egui::{Host, LauncherUi, TitleBar};
use eframe::egui;
#[cfg(target_os = "android")]
use jni::errors::Error as JniError;
#[cfg(target_os = "android")]
use jni::objects::JObject;
#[cfg(target_os = "android")]
use jni::sys::jobject;
use std::sync::{Arc, Mutex};

/// Configure wgpu to use GLES backend on Android to avoid Vulkan driver bugs on Adreno GPUs.
#[cfg(target_os = "android")]
fn wgpu_options() -> eframe::WgpuConfiguration {
    use eframe::wgpu::Backends;
    let mut config = eframe::WgpuConfiguration::default();
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut config.wgpu_setup {
        // Force GLES backend only (no Vulkan) to avoid driver bugs on Adreno GPUs
        setup.instance_descriptor.backends = Backends::GL;
    }
    config
}

/// Give egui a real screen size before winit has published one.
///
/// `egui-winit` leaves `RawInput::screen_rect` unset while `Window::inner_size()` is zero. egui
/// then uses its 10000-point default viewport, which can make the first layout enormous; the
/// native window already knows the real pixel size, so use that. The same helper keeps split
/// screen and freeform resizes correct when the platform hasn't delivered a fresh rect yet.
#[cfg(target_os = "android")]
fn ensure_screen_rect(android: &AndroidApp, ctx: &egui::Context, raw: &mut egui::RawInput) {
    const UNSET: f32 = 4096.0;
    let usable = raw.screen_rect.is_some_and(|r| r.width() > 0.0 && r.height() > 0.0 && r.width() < UNSET && r.height() < UNSET);
    if usable {
        return;
    }
    let (width, height) = if let Some(window) = android.native_window() {
        let (w, h) = (window.width(), window.height());
        if w > 0 && h > 0 { (w as f32, h as f32) } else { (1920.0, 1200.0) }
    } else {
        (1920.0, 1200.0)
    };
    let ppp = ctx.pixels_per_point().max(0.1);
    raw.screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width / ppp, height / ppp)));
}

const SETTINGS_FILE: &str = "settings.json";

/// Runs the eframe event loop on the GameActivity thread.
#[cfg(target_os = "android")]
#[allow(unsafe_code)]
pub fn run(android: AndroidApp) -> Result<(), String> {
    // The egui-winit IME bridge reads the Activity's `pollIme()` through JNI; publish the handles
    // it needs before the first frame.
    egui_winit::android_activity::set_activity(android.activity_as_ptr());
    egui_winit::android_activity::set_vm(android.vm_as_ptr());

    // Initialize rustls-platform-verifier for Android BEFORE any HTTP requests
    // This must happen before Http::new() is called
    #[cfg(target_os = "android")]
    {
        log::info!("Initializing rustls-platform-verifier for Android");
        // Get JNIEnv and context (activity) from AndroidApp
        let vm_ptr = android.vm_as_ptr() as *mut jni::sys::JavaVM;
        let activity_ptr = android.activity_as_ptr() as jobject;

        // SAFETY: android-activity guarantees these pointers are valid
        let vm = unsafe { jni::JavaVM::from_raw(vm_ptr) };

        // Use attach_current_thread which provides an Env in a closure
        vm.attach_current_thread(|env| -> Result<(), JniError> {
            let context = unsafe { JObject::from_raw(env, activity_ptr) };
            log::info!("Calling rustls_platform_verifier::android::init_with_env");
            rustls_platform_verifier::android::init_with_env(env, context)
        })
        .map_err(|e| format!("rustls-platform-verifier init failed: {e}"))?;
        log::info!("rustls-platform-verifier initialized successfully");
    }

    let files_dir = android_files_dir(&android)?;

    // Bridge for JNI calls (install, uninstall, detect version) — file-based like photocraft
    let bridge_dir = files_dir.join("bridge");
    let bridge = Bridge::new(bridge_dir);

    // Load persisted settings (JSON in app private storage)
    let settings_path = files_dir.join(SETTINGS_FILE);
    let settings = load(&settings_path).unwrap_or_default();

    // Waker for the event loop (signals from backend thread)
    let waker = Arc::new(Waker::new());

    // Create the backend and start its worker thread
    let mut backend = AndroidBackend::new(bridge.clone(), files_dir.clone(), waker.as_ref().clone());
    backend.start();
    let backend_box: Box<dyn artcraft_launcher_core::protocol::Backend> = Box::new(backend);

    // NativeOptions tuned for Android (from photocraft pattern)
    let mut native_options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        // eframe hands it to winit's event loop (with_android_app).
        android_app: Some(android.clone()),
        viewport: egui::ViewportBuilder::default().with_title("ArtCraft Launcher").with_app_id("ai.storyteller.artcraft-launcher"),
        persist_window: false,
        ..Default::default()
    };

    // Force GLES backend on Android to avoid Vulkan driver bugs on Adreno GPUs
    #[cfg(target_os = "android")]
    {
        native_options.wgpu_options = wgpu_options();
    }

    eframe::run_native(
        "ArtCraft Launcher",
        native_options,
        Box::new(move |cc| {
            // Install fonts (theme::install_fonts handles Archivo/Inter/Geist + CJK)
            artcraft_launcher_ui_egui::theme::install_fonts(&cc.egui_ctx, None);

            // The community apps' product pages, generated from about.json; the official apps'
            // pages live in core, and the details view finds both the same way.
            artcraft_launcher_core::details::set_extra_details(crate::about::ABOUT);

            // Create LauncherUi with the correct API: (ctx, backend, settings)
            let mut ui = LauncherUi::new(&cc.egui_ctx, backend_box, settings);

            // The shared UI builds its entries from the desktop suite; the Android launcher lists
            // every app the builder ports, including the community ones.
            ui.entries = crate::catalog::SUITE_ANDROID.iter().map(artcraft_launcher_ui_egui::model::AppEntry::new).collect();

            Ok(Box::new(Shell {
                ui,
                waker: waker.clone(),
                settings_path,
                host: AndroidHost { bridge: bridge.clone() },
                android: android.clone(),
                keyboard_shown: false,
                keyboard_synced: false,
                theme_dark: None,
                theme_synced: false,
            }))
        }),
    )
    .map_err(|e| format!("eframe error: {e}"))
}

/// Waker for the event loop (signals from backend thread)
#[derive(Clone)]
pub struct Waker {
    ctx: Arc<Mutex<Option<egui::Context>>>,
}

impl Waker {
    pub fn new() -> Self {
        Self { ctx: Arc::new(Mutex::new(None)) }
    }

    /// Set the egui context to wake.
    fn set_ctx(&self, ctx: egui::Context) {
        *self.ctx.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(ctx);
    }

    /// Wake the event loop to process events.
    pub fn wake(&self) {
        if let Some(ctx) = self.ctx.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref() {
            ctx.request_repaint();
        }
    }
}

/// Android-specific Host implementation (minimal)
struct AndroidHost {
    bridge: crate::bridge::Bridge,
}

impl Host for AndroidHost {
    fn title_bar(&self) -> TitleBar {
        TitleBar::Native
    }

    fn notify(&mut self, _title: &str, _body: &str) {
        // Android notifications handled by Java side if needed
    }

    fn has_tray(&self) -> bool {
        false
    }

    fn has_desktops(&self) -> bool {
        false
    }

    fn login_item(&self) -> Option<bool> {
        None
    }

    fn set_login_item(&mut self, _on: bool) -> Result<(), String> {
        Err("not supported".into())
    }

    fn pick_folder(&mut self, _initial: Option<&str>) -> Option<String> {
        None // No folder picker on Android
    }

    fn default_install_folder(&mut self, _kind: artcraft_launcher_core::PackageKind, _arch: Option<artcraft_launcher_core::Arch>) -> Option<String> {
        None
    }

    fn reduce_motion(&self) -> bool {
        // Could check Android's "Reduce motion" accessibility setting via JNI
        false
    }

    fn discord_url(&self) -> &'static str {
        "https://discord.gg/artcraft"
    }

    fn quit(&mut self) {
        // On Android, we don't quit programmatically; user swipes away the app
    }

    fn move_to_background(&mut self) {
        // Move task to background (like pressing Home button).
        // The bridge can be busy with an install; never block the UI thread on it.
        let bridge = self.bridge.clone();
        std::thread::spawn(move || bridge.move_to_background());
    }
}

/// Holds the running state between frames
struct Shell {
    ui: LauncherUi,
    host: AndroidHost,
    waker: Arc<Waker>,
    settings_path: std::path::PathBuf,
    /// The live activity handle: queried for the native window size while resizing.
    #[cfg(target_os = "android")]
    android: AndroidApp,
    /// Whether the soft keyboard was last asked to show.
    keyboard_shown: bool,
    /// Whether the keyboard state was written at least once this run.
    keyboard_synced: bool,
    /// Whether the system theme is currently dark.
    theme_dark: Option<bool>,
    /// Whether the theme was synced at least once this run.
    theme_synced: bool,
}

impl eframe::App for Shell {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Set the context on the waker on first frame
        self.waker.set_ctx(ctx.clone());

        // Drain events from backend and run logic
        self.ui.logic(ctx, &mut self.host);

        // The soft keyboard is shown and hidden by egui-winit itself (through
        // `set_ime_allowed`): the manual bridge used to fight it and leave a keyboard up with
        // nowhere to send its text.
        //
        // The Activity's hidden input view is the text source (the winit Android backend never
        // delivered committed IME text): show it exactly while a text field has focus, so the
        // IME types into the view `pollIme` drains. A tap re-asserts "show": egui keeps the
        // focus when the keyboard is dismissed with Back, and tapping the field again should
        // bring it back.
        let focused = ctx.text_edit_focused();
        let tapped = ctx.input(|i| i.pointer.any_pressed());
        if !self.keyboard_synced || focused != self.keyboard_shown || (focused && tapped) {
            self.keyboard_synced = true;
            self.keyboard_shown = focused;
            self.host.bridge.set_keyboard(focused);
        }

        // Keep the window's system bars (only visible in transient swipe mode) in step with the
        // theme egui actually resolved. `ThemeChoice::System` is left alone: the vendored
        // egui-winit feeds `RawInput::system_theme` from the Activity's configuration every frame,
        // so the whole interface follows Android light/dark live.
        let is_dark = ctx.theme() == egui::Theme::Dark;
        if !self.theme_synced || self.theme_dark != Some(is_dark) {
            self.theme_synced = true;
            self.theme_dark = Some(is_dark);
            self.host.bridge.set_theme(is_dark);
        }

        // Handle close request: on Android, move to background instead of quitting
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            // Move task to background (like pressing Home)
            self.host.move_to_background();
        }
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        // First frames and split-screen/freeform resizes can arrive before winit publishes a
        // usable screen rect; without this egui lays out against its 10000-point default.
        #[cfg(target_os = "android")]
        ensure_screen_rect(&self.android, _ctx, raw_input);
        // The keyboard's own state is egui's: `raw_input_hook` is only for the launcher's
        // reduce-motion wheel smoothing.
        self.ui.raw_input(_ctx, raw_input);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Draw the UI
        self.ui.ui(ui, &mut self.host);
    }

    fn on_exit(&mut self) {
        // Save on exit
        let _ = save(&self.settings_path, &self.ui.settings);
    }
}

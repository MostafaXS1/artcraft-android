//! Builder-owned Android shell for Coiman IDE.
//!
//! Coiman is an eframe/egui desktop IDE with private `app`/`ui` modules. The builder's prepare
//! script exposes those modules through a small `lib.rs` in `coiman-ide` and adds public wrappers
//! for the two file-opening methods. This adapter then provides everything Android needs around
//! that app:
//!
//! * the Java `CoimanIdActivity` runs the system open/save/folder pickers (SAF), stages a picked
//!   document inside app-private storage and mirrors saves back to the chosen document;
//! * the same Activity runs a small broker thread that forwards tool invocations and terminal
//!   sessions to Termux through its official `RUN_COMMAND` service;
//! * this crate installs the rfd launcher the vendored Android rfd backend calls, points
//!   `quiet_command` at the bundled Termux shim, and boots `CodeApp` inside an eframe shell that
//!   carries the builder's Android input, fullscreen and font handling.
//!
//! Upstream Coiman crates are never hand-edited; the changes prepare.sh makes are described there.
//!
//! ARTCRAFT_COIMANIDE_ANDROID_SHELL_V1

use android_activity::AndroidApp;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

mod coiman_text_bar;
mod saf;
mod system_fonts;
mod termux;
mod touch_gestures;

/// Emitted at startup so a device log (and the packaged `.so` strings) can confirm this shell is
/// the one running.
const ANDROID_SHELL_MARKER: &str = "ARTCRAFT_COIMANIDE_ANDROID_SHELL_V1";

/// Cleared when the app shuts down, so background watchers stop talking to the Activity.
static ALIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Point Coiman's settings and cache roots at app-private storage before it reads them.
///
/// `system_fonts::install_font_env` owns `HOME` (it needs it for the device-font symlinks) and
/// `XDG_DATA_HOME`; the config and cache roots are set separately so Coiman's session, recent list
/// and runtime files stay under the app's own directory.
#[allow(unused_unsafe)]
fn install_persist_env(app: &AndroidApp) -> Option<PathBuf> {
    let base = app.internal_data_path().or_else(|| app.external_data_path())?;
    let config = base.join("config");
    let cache = base.join("cache");
    let _ = std::fs::create_dir_all(&config);
    let _ = std::fs::create_dir_all(&cache);
    // SAFETY: `android_main` runs before the app starts any worker thread.
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", &config);
        std::env::set_var("XDG_CACHE_HOME", &cache);
        std::env::set_var("TMPDIR", &cache);
    }
    log::info!("config directory: {}", config.display());
    install_bundled_themes(&base, &config.join("coiman-ide").join("themes"));
    Some(config)
}

/// Copies the bundled VS Code themes into Coiman's config folder.
///
/// Coiman's theme picker looks for VS Code themes in the user's VS Code extensions folder and in
/// `<config>/themes`. There is no VS Code on a tablet, so the themes that ship with the app (the
/// Activity unpacks them next to the app's files) are copied there once.
fn install_bundled_themes(base: &Path, target: &Path) {
    let source = base.join("coiman-themes");
    let Ok(entries) = std::fs::read_dir(&source) else { return };
    if std::fs::create_dir_all(target).is_err() {
        return;
    }
    let mut copied = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|extension| extension == "json") {
            let name = path.file_name().unwrap_or_default();
            let to = target.join(name);
            if !to.exists() && std::fs::copy(&path, &to).is_ok() {
                copied += 1;
            }
        }
    }
    if copied > 0 {
        log::info!("installed {copied} bundled themes in {}", target.display());
    }
}

/// Give egui a real screen size on the first frame, before winit has published one.
///
/// `egui-winit` leaves `RawInput::screen_rect` unset while `Window::inner_size()` is zero. egui then
/// uses its 10000-point default viewport, which can make the first layout enormous; the native
/// window already knows the real pixel size, so use that.
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

/// The on-screen keyboard's height in pixels, as the Activity reports it (0 when it is hidden).
static IME_HEIGHT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// The egui context, so the Activity can ask for a repaint when the keyboard's height changes.
static IME_CONTEXT: std::sync::OnceLock<egui::Context> = std::sync::OnceLock::new();

/// Whether the app published an IME output in the last frame.
///
/// A real text field (the search popups) does, and so does nothing else: Coiman's editor and its
/// terminal draw their own text and deliberately do not, so the keyboard stays away until the
/// user asks for it. The back button uses this to tell "close the popup" from "toggle the
/// keyboard" apart.
static IME_PUBLISHED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Toggle the soft keyboard while there is something to type into.
///
/// Android hands the back button to the app as a key event (it never reaches the Activity's
/// `onBackPressed`), so this is where the tablet's "put the keyboard away / bring it back" lives:
/// with the keyboard up the back button hides it, with it down it shows it again. A tablet has
/// no other way to get the keyboard back after dismissing it.
fn toggle_keyboard(ctx: &egui::Context, bridge: &saf::Bridge) {
    // The IME output says what wants the keyboard: the editor and the terminal both draw their
    // own text, so neither of them is a TextEdit.
    let wants = ctx.egui_wants_keyboard_input();
    let height = IME_HEIGHT.load(Ordering::Relaxed);
    // A text field (a search popup) publishes an IME output; then the back button belongs to the
    // popup and not to the keyboard.
    let published = IME_PUBLISHED.load(Ordering::Relaxed);
    log::info!("back button: something wants the keyboard: {wants}, it is {height} px tall");
    if !wants || published {
        return;
    }
    if height > 0 {
        bridge.hide_soft_input();
    } else {
        bridge.show_soft_input();
    }
}

/// Called by the Activity when the keyboard's height changes (`nativeImeHeight`).
///
/// The keyboard opens and closes without the app asking for a repaint, so the Activity says so
/// from its layout listener; without this the app would keep drawing the old layout until
/// something else happened to repaint it.
#[no_mangle]
pub extern "system" fn Java_com_artcraft_coimanide_CoimanIdActivity_nativeImeHeight(
    _env: *mut std::ffi::c_void,
    _this: *mut std::ffi::c_void,
    pixels: i32,
) {
    IME_HEIGHT.store(pixels.max(0), Ordering::Relaxed);
    log::info!("the keyboard is {pixels} px tall");
    if let Some(ctx) = IME_CONTEXT.get() {
        ctx.request_repaint();
    }
}

/// Keep the interface clear of the soft keyboard.
///
/// Android 15 and later do not resize the window for the keyboard: the keyboard is drawn on top
/// of the app. Coiman keeps its terminal and its other panels at the bottom of the window, so
/// without this the keyboard would cover the terminal — the one part of an IDE that is useless
/// without a keyboard. The screen is shortened by the keyboard's height, and the whole layout
/// (the editor above it) moves up with it.
///
/// The height is read every frame: while a field has the keyboard, egui-winit keeps the app
/// repainting, so the frame that notices the keyboard is never late. The Activity also reports
/// the height when the window's layout changes, which wakes the app for the repaint.
fn keep_clear_of_the_keyboard(ctx: &egui::Context, raw: &mut egui::RawInput, bridge: &saf::Bridge) {
    let inset = bridge.ime_inset_bottom();
    IME_HEIGHT.store(inset, Ordering::Relaxed);
    let covered = inset as f32 / ctx.pixels_per_point().max(0.1);
    if covered <= 1.0 {
        return;
    }
    let Some(rect) = raw.screen_rect else { return };
    // Keep at least a third of the screen: a very tall keyboard must not leave nothing.
    let height = (rect.height() - covered).max(rect.height() * 0.3);
    raw.screen_rect = Some(egui::Rect::from_min_size(
        rect.min,
        egui::vec2(rect.width(), height),
    ));
}

/// Reconstruct egui's modifier state on Android.
///
/// winit's Android backend delivers `KeyboardInput` events but never emits `ModifiersChanged`, so
/// Ctrl/Alt shortcuts are ignored while plain keys work. Track the physical modifiers and stamp
/// them onto this frame's events, suppressing the text a shortcut would otherwise insert.
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
            egui::Event::Key { key, physical_key, pressed, modifiers, .. } => {
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

/// The picker requests in flight and the staged save destinations to mirror.
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

/// Install the launcher `rfd`'s Android backend calls for every (blocking) dialog.
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
                log::error!("Android file picker: {error}");
                None
            }
        }
    }));
}

struct CoimanAndroidApp {
    inner: coiman_ide::app::CodeApp,
    /// The live activity handle: queried for the native window size on the first frame.
    app: AndroidApp,
    bridge: saf::Bridge,
    state: Arc<SafState>,
    modifiers: egui::Modifiers,
    /// Files the system asked us to open are staged by the Activity; this is the shared queue the
    /// app's per-frame poll drains. `intent_warmup` skips the first couple of frames so Coiman has
    /// finished restoring its own session, and goes negative if the host has no intent bridge.
    intent_warmup: i64,
}

impl Drop for CoimanAndroidApp {
    fn drop(&mut self) {
        ALIVE.store(false, Ordering::Relaxed);
    }
}

impl eframe::App for CoimanAndroidApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.state.mirror.sync(&self.bridge);
        watch_incoming(&self.bridge, ctx);
        drain_incoming(
            &self.bridge,
            &self.state,
            &mut self.inner,
            &mut self.intent_warmup,
        );
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        coiman_text_bar::process_raw_input(ctx, raw);
        touch_gestures::process_raw_input(ctx, raw);
        ensure_screen_rect(&self.app, ctx, raw);
        keep_clear_of_the_keyboard(ctx, raw, &self.bridge);
        // The Android IME bridge sends the Enter key as a Key event *and* as a Text("\n"). The
        // terminal writes both, so the command ran twice; the key event is the one that counts.
        if raw.events.iter().any(|event| {
            matches!(event, egui::Event::Key { key: egui::Key::Enter, pressed: true, .. })
        }) {
            raw.events.retain(|event| !matches!(event, egui::Event::Text(text) if text == "\n"));
        }
        // The back button toggles the soft keyboard (Android delivers it as a key event).
        if raw.events.iter().any(|event| {
            matches!(
                event,
                egui::Event::Key { key: egui::Key::BrowserBack, pressed: true, .. }
            )
        }) {
            toggle_keyboard(ctx, &self.bridge);
        }
        apply_android_modifiers(&mut self.modifiers, raw);
        if raw.events.iter().any(|event| matches!(event, egui::Event::WindowFocused(false))) {
            self.state.mirror.sync(&self.bridge);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        system_fonts::install_system_fonts(ui.ctx());
        eframe::App::ui(&mut self.inner, ui, frame);
        // Inside the pass, the output says whether a text field asked for the keyboard.
        IME_PUBLISHED.store(ui.ctx().output(|o| o.ime.is_some()), Ordering::Relaxed);
        coiman_text_bar::show_text_bar(ui.ctx());
    }

    fn on_exit(&mut self) {
        self.state.mirror.sync(&self.bridge);
    }
}

/// Watch for files the system handed the app while it was idle.
fn watch_incoming(bridge: &saf::Bridge, ctx: &egui::Context) {
    static WATCHER: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    if WATCHER.set(()).is_err() {
        return;
    }
    let bridge = bridge.clone_ref();
    let ctx = ctx.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_millis(250));
        if !ALIVE.load(Ordering::Relaxed) {
            break;
        }
        match bridge.intent_pending() {
            Ok(pending) if pending > 0 => ctx.request_repaint(),
            Ok(_) => {}
            Err(error) => {
                log::info!("Launcher does not report incoming files: {error}");
                break;
            }
        }
    });
}

/// Deliver files the system handed the app (an intent it is registered for).
fn drain_incoming(
    bridge: &saf::Bridge,
    state: &SafState,
    inner: &mut coiman_ide::app::CodeApp,
    warmup: &mut i64,
) {
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
                log::info!("Opening the file the system handed the app: {path}");
                state.mirror.track(0, &path);
                coiman_ide::open_file(inner, Path::new(&path));
            }
            Ok(None) => break,
            Err(error) => {
                log::info!("Launcher does not report incoming files: {error}");
                *warmup = -1;
                break;
            }
        }
    }
}

#[cfg(target_os = "android")]
#[no_mangle]
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
            .with_tag("com.artcraft.coimanide"),
    );
    log::info!("{ANDROID_SHELL_MARKER} ready ({})", saf::SAF_MARKER);
    egui_winit::android_activity::set_activity(android_app.activity_as_ptr());
    egui_winit::android_activity::set_vm(android_app.vm_as_ptr());
    let _config_dir = install_persist_env(&android_app);
    system_fonts::install_font_env(&android_app);
    termux::install_env(&android_app);
    // Coiman's own log (what the app did, and why it stopped) lives in the app's config folder.
    // The desktop build starts it in main(); here the shell starts it instead, so a crash on the
    // device is reported the same way ("Developer: Open Crash Log" in the app).
    coiman_core::applog::start();
    let bridge = saf::Bridge::new(&android_app);
    bridge.request_storage_permission();
    if let Ok(dir) = bridge.files_dir() {
        log::info!("Coiman app-private storage: {}", dir.display());
    }
    let state = Arc::new(SafState::default());
    install_saf_launcher(bridge.clone_ref(), Arc::clone(&state));

    let mut native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Coiman IDE")
            .with_decorations(false)
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
    }
    let app_bridge = bridge.clone_ref();
    let result = eframe::run_native(
        "Coiman IDE",
        native,
        Box::new(move |cc| {
            let inner = coiman_ide::app::CodeApp::new(None, &cc.egui_ctx);
            let _ = IME_CONTEXT.set(cc.egui_ctx.clone());
            Ok(Box::new(CoimanAndroidApp {
                inner,
                app: app_bridge.android_app().clone(),
                state: Arc::clone(&state),
                modifiers: egui::Modifiers::default(),
                intent_warmup: 2,
                bridge: app_bridge.clone_ref(),
            }))
        }),
    );
    if let Err(error) = result {
        log::error!("Coiman Android exited with an error: {error}");
    }
}

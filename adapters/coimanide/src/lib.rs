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
use coiman_ide::android_keyboard::{self, Mode, Request};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;

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

    // Call Java extractThemes() to unpack bundled themes from assets to files/coiman-themes
    // This must run after the native activity is initialized but before the theme picker runs.
    unsafe {
        use jni::{JavaVM, jni_sig, jni_str};
        // SAFETY: AndroidApp owns the live VM pointer for the lifetime of the activity.
        let jvm = JavaVM::from_raw(app.vm_as_ptr() as *mut _);
        let activity_raw = app.activity_as_ptr() as jni::sys::jobject;
        let result = jvm.attach_current_thread(|env| -> jni::errors::Result<()> {
            let activity = unsafe { env.as_cast_raw::<jni::objects::JObject>(&activity_raw)? };
            env.call_method(
                &*activity,
                jni_str!("extractThemes"),
                jni_sig!("()V"),
                &[]
            )?;
            // Check for exceptions
            if env.exception_check() {
                env.exception_describe();
                env.exception_clear();
                log::warn!("Java extractThemes() threw an exception");
            } else {
                log::info!("Called Java extractThemes() from Rust");
            }
            Ok(())
        });
        if let Err(e) = result {
            log::warn!("Failed to call Java extractThemes(): {e}");
        }
    }

    install_bundled_themes(&base, &config.join("coiman-ide").join("themes"));
    Some(config)
}

/// Phone screens (smallest width below 600 dp) start at 95% zoom: Coiman's desktop default of
/// 125% crowds the workbench on a handset, leaving too little room for the editor. Tablets and
/// external displays keep the desktop default. Returns `None` when the Activity can't answer.
#[cfg(target_os = "android")]
fn device_is_phone(app: &AndroidApp) -> Option<bool> {
    use jni::{JavaVM, jni_sig, jni_str};
    // SAFETY: AndroidApp owns the live VM pointer for the lifetime of the activity.
    let jvm = unsafe { JavaVM::from_raw(app.vm_as_ptr() as *mut _) };
    let activity_raw = app.activity_as_ptr() as jni::sys::jobject;
    let result = jvm.attach_current_thread(|env| -> jni::errors::Result<bool> {
        let activity = unsafe { env.as_cast_raw::<jni::objects::JObject>(&activity_raw)? };
        Ok(env
            .call_method(&*activity, jni_str!("isHighDensityDevice"), jni_sig!("()Z"), &[])?
            .z()?)
    });
    match result {
        Ok(is_phone) => Some(is_phone),
        Err(error) => {
            log::warn!("Could not determine the screen size: {error}");
            None
        }
    }
}

/// Applies the 95% phone zoom to Coiman's user settings once per device.
///
/// `settings.toml` belongs to the user, so the choice is remembered with a `device_class` sidecar:
/// the first launch on a phone sets 95%, while a user who later changes the zoom by hand keeps it.
#[cfg(target_os = "android")]
fn apply_phone_zoom_once(app: &AndroidApp) {
    const PHONE_ZOOM: &str = "0.95";
    let Some(settings) = coiman_core::settings::user_settings_path() else { return };
    let Some(folder) = settings.parent() else { return };
    let marker = folder.join("device_class");
    if std::fs::read_to_string(&marker).ok().is_some_and(|saved| saved.trim() == "scaled") {
        return;
    }
    if device_is_phone(app) != Some(true) {
        return;
    }
    let text = std::fs::read_to_string(&settings).unwrap_or_default();
    let text = coiman_core::settings::set_value_in_text(&text, "workbench", "zoom", PHONE_ZOOM);
    let temporary = settings.with_extension("toml.tmp");
    let written = std::fs::create_dir_all(folder)
        .and_then(|()| std::fs::write(&temporary, text))
        .and_then(|()| std::fs::rename(&temporary, &settings));
    if let Err(error) = written {
        log::warn!("could not apply the phone zoom to {}: {error}", settings.display());
        return;
    }
    if let Err(error) = std::fs::write(&marker, b"scaled") {
        log::warn!("could not remember the phone zoom in {}: {error}", marker.display());
    }
    log::info!("phone screen: using {PHONE_ZOOM} zoom");
}

/// Copies the bundled VS Code themes into Coiman's config folder.
///
/// Coiman's theme picker looks for VS Code themes in the user's VS Code extensions folder and in
/// `<config>/themes`. There is no VS Code on a tablet, so the themes that ship with the app (the
/// Activity unpacks them next to the app's files) are copied there once.
fn install_bundled_themes(base: &Path, target: &Path) {
    // The Java Activity extracts themes to getFilesDir()/coiman-themes. internal_data_path() IS
    // the files dir on Android (e.g. /data/user/0/<pkg>/files), so that's base/coiman-themes.
    let source = base.join("coiman-themes");

    // Wait for Java extraction to complete (up to 10 seconds).
    // On first install, the Java onCreate() runs extractThemes() but it may not finish
    // before android_main() starts. The Java code creates the directory first, then writes
    // files - so we must wait for at least one .json file to appear, not just the directory.
    let mut attempts = 0;
    let mut has_json = false;
    let mut entries = std::fs::read_dir(&source);
    while attempts < 100 {
        if let Ok(e) = &mut entries {
            for entry in e {
                if let Ok(entry) = entry {
                    if entry.path().extension().is_some_and(|ext| ext == "json") {
                        has_json = true;
                        break;
                    }
                }
            }
            if has_json { break; }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        entries = std::fs::read_dir(&source);
        attempts += 1;
    }
    let Ok(entries) = entries else {
        log::warn!("bundled themes were not extracted: {} does not exist", source.display());
        return;
    };
    if !has_json {
        log::warn!("bundled themes were not extracted: no .json files in {}", source.display());
        return;
    }

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

/// Whether the app published an IME output in the last frame.
///
/// The editor and the terminal publish one while they have focus, and so do the search popups
/// (real text fields). The shell uses this to tell "a text surface has the keyboard" from "the
/// app is only looking at the code".
static IME_PUBLISHED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// What the user last asked the keyboard button to do.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Intent {
    /// Nobody said anything: the mode's own rule decides.
    Automatic,
    /// The user asked for the keyboard.
    Wanted,
    /// The user asked for the keyboard to go away.
    Unwanted,
}

/// The soft keyboard, as the keyboard button in the bottom bar sees it.
///
/// The button carries a mode — always on, never on its own, or as the app sees fit — and can
/// ask for the keyboard right away. This is the Android shell's side of that: the shell owns the
/// Activity, and with it the only way to show or hide the soft keyboard.
struct Keyboard {
    /// The mode the button is in.
    mode: Mode,
    /// What the user last asked for.
    intent: Intent,
    /// When the app last asked the platform for the keyboard: a request that does not take
    /// effect is not repeated every frame.
    last_ask: Option<std::time::Instant>,
    /// Show the keyboard once, right now. Auto and Off must not keep re-showing it after the
    /// user dismissed it; only an explicit "Show now", a tap in a text surface, or On do.
    force_show: bool,
}

impl Keyboard {
    fn new() -> Keyboard {
        Keyboard {
            mode: Mode::Auto,
            intent: Intent::Automatic,
            last_ask: None,
            force_show: false,
        }
    }

    /// Carries out what the button asked for, and what the mode says.
    fn apply(&mut self, ctx: &egui::Context, bridge: &saf::Bridge) {
        // What the button asked for right now.
        match android_keyboard::take_request() {
            Some(Request::Show) => {
                self.intent = Intent::Wanted;
                self.force_show = true;
            }
            Some(Request::Hide) => {
                self.intent = Intent::Unwanted;
                self.force_show = false;
            }
            Some(Request::TextTapped) => {
                // A tap in the editor or the terminal asks for the keyboard again, unless the
                // user chose Off: that mode never opens it on its own.
                if self.mode != Mode::Off {
                    self.intent = Intent::Automatic;
                    self.force_show = true;
                }
            }
            Some(Request::Automatic) => {
                // The user picked Auto: forget any earlier "Show now"/"Hide now".
                self.intent = Intent::Automatic;
                self.force_show = false;
            }
            None => {}
        }

        // A mode the user has just picked. The mode's rule below carries it out.
        let mode = android_keyboard::mode();
        if mode != self.mode {
            self.mode = mode;
            self.intent = Intent::Automatic;
            self.force_show = mode == Mode::On;
        }

        // What the mode wants this frame. `published` is egui's own IME output: the editor and
        // the terminal publish it while they have focus, and the search popups while they are
        // open.
        let published = IME_PUBLISHED.load(Ordering::Relaxed);
        let wanted = match self.mode {
            // On: up even without a text surface. "Hide now" is the one exception.
            Mode::On => self.intent != Intent::Unwanted,
            // Off: never on its own. "Show now" is the one exception.
            Mode::Off => self.intent == Intent::Wanted,
            // Auto: while a text surface has focus, and after a "Show now".
            Mode::Auto => {
                self.intent != Intent::Unwanted && (published || self.intent == Intent::Wanted)
            }
        };

        // egui-winit opens and closes the keyboard as the IME output comes and goes. Keep one
        // published while the mode wants the keyboard up (On has no text surface to publish
        // one), and take it away when the mode does not (Off must suppress a text field's own
        // output too).
        if wanted {
            if !published {
                let rect = ctx.content_rect();
                ctx.output_mut(|o| {
                    o.ime = Some(egui::output::IMEOutput {
                        owner: egui::Id::NULL,
                        context_generation: 0,
                        purpose: egui::IMEPurpose::Normal,
                        rect,
                        cursor_rect: egui::Rect::from_min_size(
                            rect.left_bottom(),
                            egui::vec2(1.0, 1.0),
                        ),
                        should_interrupt_composition: false,
                        reset_request: None,
                        change_cause: egui::output::ImeChangeCause::Other,
                        surrounding_text: egui::output::IMESurroundingText::Unavailable,
                    });
                });
            }
        } else {
            ctx.output_mut(|o| o.ime = None);
        }

        // The mode's own rule, every frame.
        let height = IME_HEIGHT.load(Ordering::Relaxed);
        if wanted {
            // The platform only shows the keyboard implicitly when a text field appears; On,
            // "Show now" and a tap in a text surface force it, which is also what makes it work
            // with a hardware keyboard attached. Auto does not force it on its own: the user
            // dismissing the keyboard must stick until a text surface is tapped again. The rate
            // limit keeps a refused request from looping.
            if height == 0 && (self.force_show || self.mode == Mode::On) {
                self.show(bridge);
            }
        } else if height > 0 {
            bridge.hide_soft_input();
        }
        self.force_show = false;
    }

    /// Asks the platform for the soft keyboard, at most once a second.
    fn show(&mut self, bridge: &saf::Bridge) {
        let now = std::time::Instant::now();
        let too_soon = self
            .last_ask
            .is_some_and(|last| now.duration_since(last) < std::time::Duration::from_millis(800));
        if too_soon {
            return;
        }
        self.last_ask = Some(now);
        log::info!("the keyboard button asks for the soft keyboard");
        bridge.show_soft_input();
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
    /// The soft keyboard, as the keyboard button in the bottom bar sees it.
    keyboard: Keyboard,
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
        // The back button belongs to the app: the keyboard is the button in the bottom bar's
        // business now.
        apply_android_modifiers(&mut self.modifiers, raw);
        if raw.events.iter().any(|event| matches!(event, egui::Event::WindowFocused(false))) {
            self.state.mirror.sync(&self.bridge);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        system_fonts::install_system_fonts(ui.ctx());
        eframe::App::ui(&mut self.inner, ui, frame);
        // Inside the pass, the output says whether a text surface asked for the keyboard.
        IME_PUBLISHED.store(ui.ctx().output(|o| o.ime.is_some()), Ordering::Relaxed);
        coiman_text_bar::show_text_bar(ui.ctx());
        // The keyboard button in the bottom bar has been drawn, and may have been tapped.
        self.keyboard.apply(ui.ctx(), &self.bridge);
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
    apply_phone_zoom_once(&android_app);
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
            Ok(Box::new(CoimanAndroidApp {
                inner,
                app: app_bridge.android_app().clone(),
                state: Arc::clone(&state),
                modifiers: egui::Modifiers::default(),
                keyboard: Keyboard::new(),
                intent_warmup: 2,
                bridge: app_bridge.clone_ref(),
            }))
        }),
    );
    if let Err(error) = result {
        log::error!("Coiman Android exited with an error: {error}");
    }
}

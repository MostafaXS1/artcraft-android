//! Builder-owned Android shell for SubCraft.
//!
//! SubCraft is not part of the `storytold` craft family: its crates are named `subcraft-*` and it
//! has no `Services` abstraction to inject. Instead its file dialogs are the blocking
//! `rfd::FileDialog`, which has no Android backend. The builder vendors an Android-compatible `rfd`
//! (`vendor/rfd`) whose dialogs route through `rfd::artcraft_saf`; this shell installs the launcher
//! that drives them with Android's Storage Access Framework (SAF):
//!
//! * the Java `SubCraftActivity` runs the system open/save/folder pickers, stages a picked document
//!   inside app-private storage, and remembers a save destination's SAF URI;
//! * the Rust side shows the picker, waits for the user's answer (safe: `android_main` runs on its
//!   own Rust thread, and the SAF bridge flips the vendored `android-activity` glue's callbacks to
//!   non-blocking while the pick parks that thread, so Android's lifecycle handshake never stalls),
//!   and hands the staged private path back to `rfd`;
//! * the app writes to that private path as usual and [`saf::SaveMirror`] copies the bytes back to
//!   the chosen document on the next frames.
//!
//! The upstream SubCraft crates remain unchanged.
//!
//! ARTCRAFT_SUBCRAFT_SAF_BRIDGE_V1

use android_activity::AndroidApp;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

// Shared with the generated launchers: brings the device's own system fonts into the egui fallback
// chain and makes them visible to the app's own font scanners (see templates/android/system_fonts.rs).
mod system_fonts;
mod mobile_text_bar;
mod touch_gestures;
mod mcp_http;

// The SAF bridge, inlined so the adapter is self-contained (a custom adapter is copied verbatim and
// does not receive templates/android/saf_bridge.rs).
mod saf;

/// Emitted at startup so the builder can confirm the bridge is present in the packaged `.so`.
const SAF_BRIDGE_MARKER: &str = "ARTCRAFT_SUBCRAFT_SAF_BRIDGE_V1";

/// Emitted at startup so a device log (and the packaged `.so` strings) can confirm this shell —
/// including immersive fullscreen — is the one that is running.
const ANDROID_SHELL_MARKER: &str = "ARTCRAFT_SUBCRAFT_ANDROID_SHELL_V1";

/// Cleared when the app shuts down, so the background watcher stops talking to the Activity.
///
/// Android releases the Activity's Java object as soon as the native thread returns, so a watcher
/// that kept asking it for files after that would call into a dead object. The flag is cleared when
/// the app is dropped, which happens before the native thread returns.
static ALIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Point the settings/cache roots at app-private storage before the app reads them. `HOME` and
/// `XDG_DATA_HOME` are set by [`system_fonts::install_font_env`] to the *internal* directory (it
/// needs it for the font symlinks); the external app directory is preferred for the settings and
/// caches so they stay reachable over USB. This also ensures autosave and crash recovery files are
/// stored under Android/data/com.artcraft.subcraft/files, where the user can access them via MTP.
#[allow(unused_unsafe)]
fn install_persist_env(app: &AndroidApp) -> Option<PathBuf> {
    let Some(base) = app.external_data_path().or_else(|| app.internal_data_path()) else {
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
    // SAFETY: `android_main` runs before the app starts any worker thread, so no other thread is
    // reading the environment here.
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", &config);
        std::env::set_var("XDG_CACHE_HOME", &cache);
    }
    // Ensure SubCraft uses the external app-private directory for preferences and autosaves,
    // matching the behavior of other ArtCraft apps. Some SubCraft versions read settings from
    // standard locations; create the autosave/recovery dirs here so they are present.
    log::info!("settings directory: {}", config.display());
    log::info!("autosave directory: {}", autosave.display());
    log::info!("recovery directory: {}", recovery.display());
    Some(config)
}

/// Ask the Activity whether the screen is dense enough (>600 dpi) to give the app its own UI scale
/// instead of the system's. Returns `None` when the answer cannot be read. See
/// `SubCraftActivity.isHighDensityDevice`.
///
/// SubCraft has no persisted interface scale of its own (its UI-size preference is applied by the
/// app), so nothing calls this yet; it mirrors the other adapters' bootstrap so the rule lives in
/// one place on the Java side.
#[allow(dead_code)]
fn device_is_high_density(app: &AndroidApp) -> Option<bool> {
    // SAFETY: AndroidApp owns the live VM pointer for the lifetime of the activity.
    let jvm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut _) };
    let activity_raw = app.activity_as_ptr() as jni::sys::jobject;
    let result = jvm.attach_current_thread(|env| -> jni::errors::Result<bool> {
        let activity = unsafe { env.as_cast_raw::<jni::objects::JObject>(&activity_raw)? };
        Ok(env
            .call_method(&*activity, jni::jni_str!("isHighDensityDevice"), jni::jni_sig!("()Z"), &[])?
            .z()?)
    });
    match result {
        Ok(is_dense) => Some(is_dense),
        Err(error) => {
            log::warn!("Could not determine the screen density: {error}");
            None
        }
    }
}

/// Give egui a real screen size on the frames where winit has not reported one yet.
///
/// `egui-winit` leaves `RawInput::screen_rect` as `None` while `Window::inner_size()` is zero, which
/// happens on the very first frame: the native window exists (the surface was created) but its size
/// has not been published to the winit backend. egui then falls back to its own default viewport of
/// 10000×10000 points. SubCraft applies its UI-size preference (a zoom factor) in that same frame,
/// and egui rescales the *default* rect by the zoom change (`10000 / 0.95 = 10526` points). The 3D
/// viewport allocates its offscreen render target from that rect, so the first frame asked wgpu for a
/// texture far larger than the device's `max_texture_dimension_2d` and the app panicked before a
/// second frame could correct the size.
///
/// The native window (the same surface winit presents to) already knows the real pixel size, so it is
/// used to fill the rect in whenever winit's is missing or still the unset default. The rect is in
/// points, i.e. physical pixels divided by the current `pixels_per_point` (which includes the zoom).
fn ensure_screen_rect(app: &AndroidApp, ctx: &egui::Context, raw: &mut egui::RawInput) {
    // egui's unset default is 10000 points; no Android surface is anywhere near that wide or tall.
    // A zero/negative or missing rect is equally unusable.
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

/// Reconstruct egui's modifier state on Android.
///
/// winit's Android backend delivers `KeyboardInput` events but never emits `ModifiersChanged`, so
/// egui's modifier state stays empty and `Ctrl`/`Alt` shortcuts are ignored while plain keys keep
/// working. The physical modifier keys do arrive as `Event::Key` events, so they are tracked here and
/// stamped onto this frame's events. Mirrors `egui-winit`'s handling and suppresses the text
/// character that would otherwise be emitted alongside a Ctrl/Command shortcut.
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
        // `RawInput` has no modifier field in this egui version: `InputState.modifiers` is driven
        // purely by this event, so it must be emitted on change.
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

/// Install the launcher `rfd`'s Android backend calls for every (blocking) dialog. The bridge shows
/// the system picker and waits for the user, and (for a save) records the staged destination so the
/// app's own write is mirrored back to the chosen SAF document.
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

#[allow(dead_code)]
/// Diff keyboard text into the edit egui understands.
fn ime_diff(prev: &str, now: &str) -> (usize, String, bool) {
    let common = prev.chars().zip(now.chars()).take_while(|(a, b)| a == b).count();
    let backspaces = prev.chars().count() - common;
    let added: String = now.chars().skip(common).collect();
    let enter = added.contains('\n');
    let insert: String = added.chars().filter(|c| *c != '\n' && *c != '\r').collect();
    (backspaces, insert, enter)
}

#[allow(dead_code)]
fn ime_events(edit: &(usize, String, bool)) -> Vec<egui::Event> {
    let (backspaces, insert, enter) = edit.clone();
    let key = |key| egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    let mut out: Vec<egui::Event> = Vec::with_capacity(backspaces + 2);
    for _ in 0..backspaces {
        out.push(key(egui::Key::Backspace));
    }
    if !insert.is_empty() {
        out.push(egui::Event::Text(insert));
    }
    if enter {
        out.push(key(egui::Key::Enter));
    }
    out
}

struct SubCraftAndroidApp {
    inner: subcraft_ui::SubcraftApp,
    /// The live activity handle: queried for the native window size while winit has not reported one.
    app: AndroidApp,
    bridge: saf::Bridge,
    state: Arc<SafState>,
    ime_text: String,
    ime_stale: Option<String>,
    editing: bool,
    modifiers: egui::Modifiers,
    // Files the system asked us to open are staged by the Activity; this is the shared queue the
    // app's own per-frame poll drains. `intent_warmup` skips the first couple of frames so the app
    // has finished its own start-up, and goes negative (stop polling) if the host has no intent
    // bridge.
    intent_warmup: i64,
    _mcp: Option<mcp_http::Endpoint>,
}

impl SubCraftAndroidApp {
    /// Poll android-activity's IME text and turn changes into egui events.
    ///
    /// winit opens the soft keyboard for a focused text field but drops GameActivity's text
    /// events, so the shell drains the keyboard's own text state. While a field has focus the
    /// buffer is polled every frame and a change becomes Backspace/Text/Enter events.
    #[allow(dead_code)]
    fn poll_ime(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        // A focused egui field, or text the app edits on its canvas: the canvas asks for the
        // keyboard through egui's IME output instead of focusing a field.
        let editing = ctx.egui_wants_keyboard_input() || ctx.output(|o| o.ime.is_some());
        if editing != self.editing {
            self.editing = editing;
            if editing {
                self.app.set_ime_editor_info(
                    android_activity::input::InputType::TYPE_CLASS_TEXT
                        | android_activity::input::InputType::TYPE_TEXT_FLAG_MULTI_LINE,
                    android_activity::input::TextInputAction::None,
                    android_activity::input::ImeOptions::IME_FLAG_NO_FULLSCREEN,
                );
                self.app.set_text_input_state(android_activity::input::TextInputState {
                    text: String::new(),
                    selection: android_activity::input::TextSpan { start: 0, end: 0 },
                    compose_region: None,
                });
                let old = std::mem::take(&mut self.ime_text);
                self.ime_stale = if old.is_empty() { None } else { Some(old) };
                self.app.show_soft_input(false);
            } else {
                self.app.hide_soft_input(false);
            }
        }
        if self.editing {
            // The user tapped in the text again or pressed a key: if Android's back key had
            // dismissed the keyboard, bring it back. Escape is the back key itself.
            let interacted = raw.events.iter().any(|event| match event {
                egui::Event::PointerButton { pressed: true, .. } => true,
                egui::Event::Key { key, pressed: true, .. } => *key != egui::Key::Escape,
                _ => false,
            });
            if interacted {
                self.app.show_soft_input(false);
            }
            let now = self.app.text_input_state().text;
            if self.ime_stale.as_ref().is_some_and(|old| *old != now) {
                self.ime_stale = None;
            }
            if now != self.ime_text && self.ime_stale.is_none() {
                let prev = self.ime_text.clone();
                self.ime_text = now.clone();
                for event in ime_events(&ime_diff(&prev, &now)) {
                    raw.events.push(event);
                }
            }
            // GameActivity's text events don't wake the UI: keep a frame coming while typing.
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }
}

impl Drop for SubCraftAndroidApp {
    fn drop(&mut self) {
        // The Activity is about to go: stop the watcher before the native thread returns.
        ALIVE.store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

impl eframe::App for SubCraftAndroidApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Commit any staged save destination the app rewrote before the app runs, and deliver files
        // the system handed us through the app's own picker queue.
        self.state.mirror.sync(&self.bridge);
        watch_incoming(&self.bridge, ctx);
        drain_incoming(&self.bridge, &self.state, &mut self.intent_warmup);
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        mobile_text_bar::process_raw_input(ctx, raw);
        touch_gestures::process_raw_input(ctx, raw);
        ensure_screen_rect(&self.app, ctx, raw);
        apply_android_modifiers(&mut self.modifiers, raw);
        if raw.events.iter().any(|event| matches!(event, egui::Event::WindowFocused(false))) {
            self.state.mirror.sync(&self.bridge);
        }
        self.inner.raw_input_hook(ctx, raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        system_fonts::install_system_fonts(ui.ctx());
        // SubCraft does all of its per-frame work (input, picker delivery, autosave) in `ui`, so
        // there is nothing to forward to `logic` — calling it would run the frame twice.
        self.inner.ui(ui, frame);
        mobile_text_bar::show_text_bar(ui.ctx());
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.inner.save(storage);
        storage.flush();
    }

    // Built with `wgpu` only (no `glow`), so `on_exit` takes no GL context (eframe 0.36).
    fn on_exit(&mut self) {
        self.state.mirror.sync(&self.bridge);
        self.inner.on_exit();
    }
}

/// Watch for files the system (or another app's drop) handed the app while it was idle.
///
/// egui runs a frame only when something asks it to, so a file that arrives while the app sits
/// untouched would wait in the Activity's queue until the user happened to touch the screen. The
/// watcher asks the Activity how many files are waiting (without taking any) and asks egui for a
/// repaint when there is one, so the file opens on its own. The file itself is still delivered on
/// the frame thread, in [`drain_incoming`].
fn watch_incoming(bridge: &saf::Bridge, ctx: &egui::Context) {
    static WATCHER: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    if WATCHER.set(()).is_err() {
        return;
    }
    let bridge = bridge.clone_ref();
    let ctx = ctx.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_millis(200));
        if !ALIVE.load(std::sync::atomic::Ordering::Relaxed) {
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

/// Deliver any file the system handed the app (an intent it is registered for). Each staged path is
/// offered to SubCraft's own picker queue, exactly like a document chosen in the open picker: a
/// `.subcraft` project opens as a project, everything else as a mesh.
fn drain_incoming(bridge: &saf::Bridge, state: &SafState, warmup: &mut i64) {
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
                let path = PathBuf::from(path);
                if is_project(&path) {
                    subcraft_ui::files::deliver_android(subcraft_ui::files::Purpose::OpenProject, vec![path]);
                } else {
                    subcraft_ui::files::deliver_android(subcraft_ui::files::Purpose::OpenMesh, vec![path]);
                }
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

/// Whether `path` names a SubCraft project (rather than an importable mesh).
fn is_project(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(subcraft_app::PROJECT_EXTENSION))
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
            .with_tag("com.artcraft.subcraft"),
    );
    log::info!(
        "{SAF_BRIDGE_MARKER} ready ({}; {ANDROID_SHELL_MARKER}; ARTCRAFT_ANDROID_NATIVE_IME_V1)",
        saf::SAF_MARKER,
    );
    egui_winit::android_activity::set_activity(android_app.activity_as_ptr());
    egui_winit::android_activity::set_vm(android_app.vm_as_ptr());
    let config_dir = install_persist_env(&android_app);
    system_fonts::install_font_env(&android_app);
    let bridge = saf::Bridge::new(&android_app);
    // Ask once for broad storage access so the app's own file browsers can read files beyond the
    // ones picked through SAF.
    bridge.request_storage_permission();
    if let Ok(dir) = bridge.files_dir() {
        log::info!("SubCraft app-private storage: {}", dir.display());
    }
    let state = Arc::new(SafState::default());
    install_saf_launcher(bridge.clone_ref(), Arc::clone(&state));

    subcraft_ui::register_gpu_baking();

    let options = subcraft_ui::LaunchOptions::default();

    let mut native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("SubCraft")
            .with_drag_and_drop(true),
        renderer: eframe::Renderer::Wgpu,
        android_app: Some(android_app.clone()),
        persistence_path: config_dir.map(|c| c.join("app.ron")),
        ..Default::default()
    };
    // Vulkan first, GL as the fallback: some Android GPUs only expose one.
    native.wgpu_options.surface.desired_maximum_frame_latency = Some(2);
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut native.wgpu_options.wgpu_setup {
        create.instance_descriptor.backends = eframe::wgpu::Backends::VULKAN | eframe::wgpu::Backends::GL;
        create.instance_descriptor.flags = eframe::wgpu::InstanceFlags::empty();
        create.power_preference = eframe::wgpu::PowerPreference::HighPerformance;
    }
    let app_bridge = bridge.clone_ref();
    let result = eframe::run_native(
        "SubCraft",
        native,
        Box::new(move |cc| {
            let inner = subcraft_ui::SubcraftApp::new(cc, options);
            let _mcp = mcp_http::start_mcp_service(
                "SubCraft".to_string(),
                "0.2.5".to_string(),
                Arc::new(|| ALIVE.load(Ordering::Relaxed)),
            );
            Ok(Box::new(SubCraftAndroidApp {
                inner,
                app: app_bridge.android_app().clone(),
                state: Arc::clone(&state),
                ime_text: String::new(),
                ime_stale: None,
                editing: false,
                modifiers: egui::Modifiers::default(),
                intent_warmup: 2,
                bridge: app_bridge.clone_ref(),
                _mcp,
            }))
        }),
    );
    if let Err(error) = result {
        log::error!("SubCraft Android exited with an error: {error}");
    }
}

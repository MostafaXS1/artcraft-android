//! Builder-owned Android shell for VectorCraft.
//!
//! Platform file operations use Android's Storage Access Framework (SAF). The Java
//! `VectorCraftActivity` runs the pickers, stages picked documents inside app-private storage and
//! remembers a save destination's URI; the Rust side reads staged paths with the standard file API,
//! writes bytes to them, and copies them back through ContentResolver. The upstream VectorCraft
//! crates remain unchanged.
//!
//! VectorCraft shows its file dialogs off the UI thread through [`Services::start_pick`]: a command
//! (or a dialog's OK) runs as an entry, asking returns no path for now, and when the picker answers
//! the entry runs again with the picked path. The picker is launched through JNI and its answer is
//! delivered into the receiver each frame by [`poll_answers`].
//!
//! ARTCRAFT_VECTORCRAFT_SAF_BRIDGE_V1

use android_activity::AndroidApp;
use jni::{
    JavaVM, jni_sig, jni_str,
    objects::{JObject, JString, JValue},
};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use vectorcraft_engine::Session;
use vectorcraft_ui_egui::picks::PickRequest;
use vectorcraft_ui_egui::{FilePick, Inbox, Services, VectorcraftApp};

// Shared with the generated launchers: brings the device's own system fonts into the egui fallback
// chain and makes them visible to the app's own font scanners (see templates/android/system_fonts.rs).
mod system_fonts;

// Shared with the generated launchers: turns Android's soft keyboard into egui IME events (see
// templates/android/ime_bridge.rs). The `mod ime_bridge;` marker also keeps the vendored
// android-activity, which forwards show/hide to this Activity's imeShow/imeHide, in step.
mod ime_bridge;

/// Emitted at startup so the builder can confirm the bridge is present in the packaged `.so`.
const SAF_BRIDGE_MARKER: &str = "ARTCRAFT_VECTORCRAFT_SAF_BRIDGE_V1";

/// Emitted at startup so a device log (and the packaged `.so` strings) can confirm this shell —
/// including the Android modifier-key fix and immersive fullscreen — is the one that is running.
const ANDROID_SHELL_MARKER: &str = "ARTCRAFT_VECTORCRAFT_ANDROID_SHELL_V2_MODIFIERS_FULLSCREEN";

/// Cleared when the app shuts down, so a background watcher stops talking to the Activity.
///
/// Android releases the Activity's Java object as soon as the native thread returns, so a
/// watcher that kept asking it for files after that would call into a dead object. The flag is
/// cleared when the app is dropped, which happens before the native thread returns.
static ALIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

struct AndroidFileBridge {
    app: AndroidApp,
    vm: Arc<JavaVM>,
}

impl AndroidFileBridge {
    fn new(app: AndroidApp) -> Result<Self, String> {
        // AndroidApp owns the live VM pointer for the lifetime of the activity.
        let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr() as *mut _) };
        Ok(Self {
            app,
            vm: Arc::new(vm),
        })
    }

    /// Keep the JavaVM and the AndroidApp cheap to clone into the platform-service closures.
    fn clone_for_services(&self) -> Self {
        Self {
            app: self.app.clone(),
            vm: self.vm.clone(),
        }
    }

    fn request_open(&self, id: u64, multiple: bool, mime: &str) -> Result<(), String> {
        self.vm
            .attach_current_thread(|env| -> jni::errors::Result<()> {
                // AndroidApp returns an unowned global ref. Do not delete it when this wrapper drops.
                let activity_raw = self.app.activity_as_ptr() as jni::sys::jobject;
                // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it
                // without taking ownership or attempting to delete the reference.
                let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
                let mime = env.new_string(mime)?;
                let mime_obj = JObject::from(mime);
                env.call_method(
                    &*activity,
                    jni_str!("vcRequestOpen"),
                    jni_sig!("(JZLjava/lang/String;)V"),
                    &[
                        JValue::Long(id as i64),
                        JValue::Bool(multiple),
                        JValue::Object(&mime_obj),
                    ],
                )?;
                Ok(())
            })
            .map_err(|e| e.to_string())
    }

    fn request_save(&self, id: u64, name: &str, mime: &str) -> Result<(), String> {
        self.vm
            .attach_current_thread(|env| -> jni::errors::Result<()> {
                let activity_raw = self.app.activity_as_ptr() as jni::sys::jobject;
                // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it
                // without taking ownership or attempting to delete the reference.
                let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
                let name = env.new_string(name)?;
                let name_obj = JObject::from(name);
                let mime = env.new_string(mime)?;
                let mime_obj = JObject::from(mime);
                env.call_method(
                    &*activity,
                    jni_str!("vcRequestSave"),
                    jni_sig!("(JLjava/lang/String;Ljava/lang/String;)V"),
                    &[
                        JValue::Long(id as i64),
                        JValue::Object(&name_obj),
                        JValue::Object(&mime_obj),
                    ],
                )?;
                Ok(())
            })
            .map_err(|e| e.to_string())
    }

    fn poll_result(&self) -> Result<Option<String>, String> {
        self.vm
            .attach_current_thread(|env| -> jni::errors::Result<Option<String>> {
                let activity_raw = self.app.activity_as_ptr() as jni::sys::jobject;
                // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it
                // without taking ownership or attempting to delete the reference.
                let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
                let value = env
                    .call_method(
                        &*activity,
                        jni_str!("vcPollPickerResult"),
                        jni_sig!("()Ljava/lang/String;"),
                        &[],
                    )?
                    .l()?;
                if value.is_null() {
                    return Ok(None);
                }
                let value = JString::cast_local(env, value)?;
                let value: String = value.try_to_string(env)?;
                Ok(Some(value))
            })
            .map_err(|e| e.to_string())
    }

    /// Hand back the private path of one file the system asked this app to open (an
    /// `ACTION_VIEW`/`ACTION_SEND` it is registered for), or `None` when nothing is pending. Returns
    /// an error when the Activity has no intent host; the caller then stops asking.
    fn intent_poll(&self) -> Result<Option<String>, String> {
        self.vm
            .attach_current_thread(|env| -> jni::errors::Result<Option<String>> {
                let activity_raw = self.app.activity_as_ptr() as jni::sys::jobject;
                // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it
                // without taking ownership or attempting to delete the reference.
                let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
                let value = match env.call_method(
                    &*activity,
                    jni_str!("intentPoll"),
                    jni_sig!("()Ljava/lang/String;"),
                    &[],
                ) {
                    Ok(value) => value.l()?,
                    Err(error) => {
                        // A missing `intentPoll` leaves a pending NoSuchMethodError on the shared
                        // JNI thread (the picker bridge uses it too); clear it before returning.
                        env.exception_clear();
                        return Err(error);
                    }
                };
                if value.is_null() {
                    return Ok(None);
                }
                let value = JString::cast_local(env, value)?;
                let value: String = value.try_to_string(env)?;
                Ok(Some(value))
            })
            .map_err(|e| e.to_string())
    }

    /// How many files the system (or a drop) handed us that the shell has not opened yet.
    /// Asked without taking anything off the queue, so the frame that opens the file still
    /// finds it there. The watcher's thread uses it to wake an idle app.
    fn intent_pending(&self) -> Result<usize, String> {
        self.vm
            .attach_current_thread(|env| -> jni::errors::Result<usize> {
                let activity_raw = self.app.activity_as_ptr() as jni::sys::jobject;
                // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it
                // without taking ownership or attempting to delete the reference.
                let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
                let value = match env.call_method(
                    &*activity,
                    jni_str!("intentPending"),
                    jni_sig!("()I"),
                    &[],
                ) {
                    Ok(value) => value.i()?,
                    Err(error) => {
                        // A missing `intentPending` leaves a pending NoSuchMethodError on the shared
                        // JNI thread (which the SAF bridge also uses); clear it before returning.
                        env.exception_clear();
                        return Err(error);
                    }
                };
                Ok(value.max(0) as usize)
            })
            .map_err(|e| e.to_string())
    }

    fn files_dir(&self) -> Result<String, String> {
        self.vm
            .attach_current_thread(|env| -> jni::errors::Result<String> {
                let activity_raw = self.app.activity_as_ptr() as jni::sys::jobject;
                // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it
                // without taking ownership or attempting to delete the reference.
                let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
                let value = env
                    .call_method(
                        &*activity,
                        jni_str!("vcFilesDir"),
                        jni_sig!("()Ljava/lang/String;"),
                        &[],
                    )?
                    .l()?;
                let value = JString::cast_local(env, value)?;
                let value: String = value.try_to_string(env)?;
                Ok(value)
            })
            .map_err(|e| e.to_string())
    }

    /// `NONE` for ordinary private paths, `OK` after committing a selected SAF destination, or
    /// `ERROR:<message>` when the Android provider refused the write.
    fn commit_if_mapped(&self, path: &str) -> Result<String, String> {
        self.vm
            .attach_current_thread(|env| -> jni::errors::Result<String> {
                let activity_raw = self.app.activity_as_ptr() as jni::sys::jobject;
                // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it
                // without taking ownership or attempting to delete the reference.
                let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
                let path = env.new_string(path)?;
                let path_obj = JObject::from(path);
                let value = env
                    .call_method(
                        &*activity,
                        jni_str!("vcCommitIfMapped"),
                        jni_sig!("(Ljava/lang/String;)Ljava/lang/String;"),
                        &[JValue::Object(&path_obj)],
                    )?
                    .l()?;
                let value = JString::cast_local(env, value)?;
                let value: String = value.try_to_string(env)?;
                Ok(value)
            })
            .map_err(|e| e.to_string())
    }
}

/// The picker requests in flight: their answer receivers, keyed by request id.
#[derive(Default)]
struct PickState {
    next_id: AtomicU64,
    senders: Mutex<HashMap<u64, Sender<Vec<String>>>>,
}

impl PickState {
    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed).wrapping_add(1)
    }

    fn insert(&self, id: u64, sender: Sender<Vec<String>>) {
        self.senders
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, sender);
    }

    fn remove(&self, id: u64) -> Option<Sender<Vec<String>>> {
        self.senders
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id)
    }

    fn is_empty(&self) -> bool {
        self.senders
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
    }
}

/// A MIME type for an open dialog, from its first file-type filter (every file when it has none).
fn mime_for_filters(pick: &FilePick) -> String {
    pick.filters
        .first()
        .and_then(|(_, exts)| exts.first().map(|s| s.as_str()))
        .map(mime_for_extension)
        .unwrap_or_else(|| "*/*".to_string())
}

fn mime_for_name(name: &str) -> String {
    let ext = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    mime_for_extension(ext)
}

fn mime_for_extension(ext: &str) -> String {
    match ext.to_ascii_lowercase().as_str() {
        "svg" | "svgz" => "image/svg+xml",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "tif" | "tiff" => "image/tiff",
        "bmp" => "image/bmp",
        _ => "*/*",
    }
    .to_string()
}

/// Show `request`'s Android picker off the UI thread; the answer arrives on the receiver. `None`
/// when this host can't (a folder picker): the request is then dropped as cancelled.
fn start_pick(
    state: &Arc<PickState>,
    bridge: &AndroidFileBridge,
    request: PickRequest,
) -> Option<Receiver<Vec<String>>> {
    let id = state.next_id();
    let (tx, rx) = mpsc::channel();
    let launch = match request {
        PickRequest::Open(pick) => {
            let mime = mime_for_filters(&pick);
            state.insert(id, tx);
            bridge.request_open(id, false, &mime)
        }
        PickRequest::OpenMany => {
            state.insert(id, tx);
            bridge.request_open(id, true, "*/*")
        }
        PickRequest::Save(pick) => {
            let name = if pick.name.trim().is_empty() {
                "Untitled.svg".to_string()
            } else {
                pick.name.clone()
            };
            let mime = mime_for_name(&name);
            state.insert(id, tx);
            bridge.request_save(id, &name, &mime)
        }
        PickRequest::Folder => return None,
    };
    if let Err(error) = launch {
        state.remove(id);
        log::error!("Could not start the Android file picker: {error}");
        return None;
    }
    Some(rx)
}

/// Each frame: read every picker answer Android queued and send it to the request that asked.
fn poll_answers(state: &Arc<PickState>, bridge: &AndroidFileBridge, ctx: &egui::Context) {
    // Each answer is one short JSON object queued by VectorCraftActivity after picker I/O ends.
    for _ in 0..8 {
        let raw = match bridge.poll_result() {
            Ok(Some(raw)) => raw,
            Ok(None) => break,
            Err(error) => {
                log::error!("Android file picker polling failed: {error}");
                break;
            }
        };
        let value: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(value) => value,
            Err(error) => {
                log::error!("Invalid Android picker result: {error}: {raw}");
                continue;
            }
        };
        let Some(id) = value.get("id").and_then(serde_json::Value::as_u64) else {
            log::error!("Android picker result has no request id: {raw}");
            continue;
        };
        let Some(sender) = state.remove(id) else {
            log::warn!("Ignoring stale Android picker reply {id}");
            continue;
        };
        let status = value
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("error");
        let paths: Vec<String> = match status {
            "cancel" => Vec::new(),
            "ok" => value
                .get("paths")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
                .collect(),
            _ => {
                log::error!(
                    "Android file picker failed: {}",
                    value
                        .get("message")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("unknown error")
                );
                Vec::new()
            }
        };
        let _ = sender.send(paths);
        ctx.request_repaint();
    }
    if !state.is_empty() {
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
}

/// Write `bytes` to `path`, then copy them to the selected SAF document when `path` is staged.
fn write_mobile(bridge: &AndroidFileBridge, path: &str, bytes: &[u8]) -> Result<(), String> {
    let target = Path::new(path);
    if let Some(parent) = target.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Could not create {}: {e}", parent.display()))?;
    }
    fs::write(target, bytes).map_err(|e| format!("Could not write {}: {e}", target.display()))?;
    match bridge.commit_if_mapped(path)? {
        state if state == "NONE" || state == "OK" => Ok(()),
        state if state.starts_with("ERROR:") => Err(state.trim_start_matches("ERROR:").to_owned()),
        state => Err(format!("Unexpected Android save result: {state}")),
    }
}

/// The app's private storage directory. Android exposes two: the internal one
/// (`/data/user/0/<pkg>/files`), which a file manager or a computer never sees, and the app-specific
/// external one (`/storage/emulated/0/Android/data/<pkg>/files`), which the user can reach over USB
/// or MTP without root. The external directory is preferred so the settings, autosaves and exported
/// documents are actually findable; the internal directory is the fallback for the rare device where
/// external storage is unmounted or unavailable.
fn app_data_dir(app: &AndroidApp) -> Option<std::path::PathBuf> {
    app.external_data_path().or_else(|| app.internal_data_path())
}

/// Where the app keeps its settings, preferences and recent-file list: the app's private files
/// directory plus a `config` subfolder, which survives restarts and updates. The desktop app keeps
/// these in its per-user config folder and the web app in the browser; the Android shell has
/// neither, so it writes them here.
fn persist_dir(app: &AndroidApp) -> Option<std::path::PathBuf> {
    app_data_dir(app).map(|base| base.join("config"))
}

/// Point the platform path helpers at app-private storage before the app reads them, so anything
/// the app writes itself (autosave, recovery copies, caches) also lands there.
///
/// Only the settings and caches are set here, and only to the external app directory, so they stay
/// reachable over USB. `HOME` and `XDG_DATA_HOME` are left to
/// [`system_fonts::install_font_env`], which keeps them on internal storage: they are the roots the
/// apps' font scanners read, and the symlinks to `/system/fonts` they need can only be created
/// there (`/storage/emulated/0` refuses `symlink`).
#[allow(unused_unsafe)]
fn install_persist_env(app: &AndroidApp) {
    let Some(base) = app_data_dir(app) else { return };
    let config = base.join("config");
    let cache = base.join("cache");
    let _ = fs::create_dir_all(config.join(".config"));
    let _ = fs::create_dir_all(&cache);
    // SAFETY: `android_main` runs before the app starts any worker thread, so no other thread is
    // reading the environment here.
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", &config);
        std::env::set_var("XDG_CACHE_HOME", &cache);
    }
    log::info!("settings directory: {}", config.display());
}

fn persist_read(dir: &Path, name: &str) -> Option<Vec<u8>> {
    fs::read(dir.join(name)).ok()
}

/// Ask the Activity whether the screen is below 600 dpi (phones) for the launch to apply the
/// app's own scale. Denser screens stay at Auto. Returns `None` when unreadable.
fn device_is_high_density(app: &AndroidApp) -> Option<bool> {
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
        Ok(is_dense) => Some(is_dense),
        Err(error) => {
            log::warn!("Could not determine the screen density: {error}");
            None
        }
    }
}

/// Write `bytes` to `dir/name` through a temporary file renamed over the old one, so a crash leaves
/// the old file or the new one whole, never a truncated one.
fn persist_write(dir: &Path, name: &str, bytes: &[u8]) {
    if let Err(error) = fs::create_dir_all(dir) {
        log::warn!("settings: {}: {error}", dir.display());
        return;
    }
    let path = dir.join(name);
    let temporary = path.with_extension("tmp");
    let written = fs::write(&temporary, bytes).and_then(|()| fs::rename(&temporary, &path));
    if let Err(error) = written {
        log::warn!("settings: {}: {error}", path.display());
    }
}

/// Whether the high-density UI scale should be applied to this launch, and the sidecar that
/// remembers that it was.
///
/// The scale belongs to the *device*, but `ui.json` belongs to the *user*: it is written on the
/// first launch and never revisited, so a screen that later stops being high-density (a foldable, an
/// external display, a display-size change) kept the old scale. The choice is remembered in its own
/// sidecar (`device_class`), so "first launch" means "the scale has not been set for this device
/// yet" and a user who changes the scale by hand keeps it: the sidecar already says it was applied,
/// so nothing is re-applied.
fn device_scale_first_time(dir: &Path) -> bool {
    let marker = dir.join("device_class");
    if fs::read_to_string(&marker).ok().is_some_and(|saved| saved.trim() == "scaled") {
        return false;
    }
    persist_write(dir, "device_class", b"scaled");
    true
}

/// Read a saved `ui.json` leniently: a docking layout or hidden-panel entry this version cannot
/// read is dropped instead of discarding every setting (as the desktop app does).
fn decode_saved_prefs(bytes: &[u8]) -> Option<vectorcraft_ui_egui::UiState> {
    use serde_json::{Map, Value, json};

    fn recover(object: &mut Map<String, Value>, hidden_key: &str) {
        if let Some(layout) = object.get("docking")
            && serde_json::from_value::<vectorcraft_ui_egui::UiState>(json!({"docking": layout})).is_err()
        {
            object.remove("docking");
        }
        if let Some(hidden) = object.get_mut(hidden_key) {
            if let Value::Object(entries) = hidden {
                entries.retain(|panel, location| {
                    serde_json::from_value::<vectorcraft_ui_egui::UiState>(json!({"docking_hidden": {panel: location}})).is_ok()
                });
            } else {
                object.remove(hidden_key);
            }
        }
    }

    let mut value: Value = serde_json::from_slice(bytes).ok()?;
    let object = value.as_object_mut()?;
    recover(object, "docking_hidden");
    if let Some(Value::Array(workspaces)) = object.get_mut("custom_workspaces") {
        for workspace in workspaces {
            if let Some(object) = workspace.as_object_mut() {
                recover(object, "dockingHidden");
            }
        }
    }
    serde_json::from_value(value).ok()
}

/// Load the saved UI state and engine preferences, as the desktop app does at startup.
fn persist_load(app: &mut VectorcraftApp, dir: &Path) {
    if let Some(bytes) = persist_read(dir, "ui.json")
        && let Some(ui) = decode_saved_prefs(&bytes)
    {
        app.ui = ui.sanitized();
    }
    vectorcraft_ui_egui::prefs_dialog::restore(app);
    let recovery = dir.join("Data Recovery").to_string_lossy().to_string();
    app.session.recovery.set_default_folder(Some(recovery));
}

/// Save the UI state and engine preferences, as the desktop app does on exit.
fn persist_save(app: &mut VectorcraftApp, dir: &Path) {
    let mut ui = app.ui.clone();
    ui.engine_prefs = app.session.prefs.to_json();
    if let Ok(bytes) = serde_json::to_vec_pretty(&ui) {
        persist_write(dir, "ui.json", &bytes);
    }
    if app.session.documents().iter().any(|doc| doc.is_dirty()) {
        vectorcraft_ui_egui::recovery::tick(app, 0.0);
        vectorcraft_ui_egui::recovery::tick(app, 1e9);
        vectorcraft_ui_egui::background::wait_all(app);
    }
}

/// Reconstruct egui's modifier state on Android.
///
/// winit's Android backend delivers `KeyboardInput` events but never emits `ModifiersChanged`
/// (it discards the `KeyEvent`'s meta state), so egui's modifier state stays empty and
/// `Ctrl`/`Alt` shortcuts are ignored while plain keys continue to work. Physical modifier keys
/// do arrive as `Event::Key` events (`Key::ControlLeft`, `Key::AltLeft`, ...), so we track them
/// and stamp the resulting state onto this frame's events. This mirrors `egui-winit`'s
/// `ModifiersChanged` handling and suppresses the text character that would otherwise be emitted
/// alongside a Ctrl/Command shortcut.
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
            egui::Event::Text(_) if state.ctrl || state.command || state.mac_cmd => {
                // Suppress text emitted with a modifier chord.
            }
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
        // purely by this event (plus its own per-event copies), so it must be emitted on change.
        raw.events.push(egui::Event::ModifiersChanged(*state));
    }
}

fn services(bridge: AndroidFileBridge, state: Arc<PickState>, inbox: Inbox) -> Services {
    let write_bridge = bridge.clone_for_services();
    let write_shared_bridge = bridge.clone_for_services();
    let pick_bridge = bridge.clone_for_services();
    let pick_state = state.clone();
    Services {
        // Files the system asked us to open (ACTION_VIEW/ACTION_SEND) are queued here and drained
        // by the app's own per-frame inbox, exactly like a document chosen in the picker.
        inbox: Some(inbox),
        // A staged document is an ordinary file in app-private storage.
        read: Some(Box::new(|path: &str| {
            fs::read(path).map_err(|e| format!("Could not read {path}: {e}"))
        })),
        write: Some(Box::new(move |path: &str, bytes: &[u8]| {
            write_mobile(&write_bridge, path, bytes)
        })),
        write_shared: Some(Arc::new(move |path: &str, bytes: &[u8]| {
            write_mobile(&write_shared_bridge, path, bytes)
        })),
        // Shown off the UI thread, so `ask` never calls these; they make the dialogs count as
        // available (`picks::can`).
        pick_open: Some(Box::new(|_pick: &FilePick| None)),
        pick_save: Some(Box::new(|_pick: &FilePick| None)),
        pick_open_multi: Some(Box::new(Vec::new)),
        pick_folder: Some(Box::new(|| None)),
        start_pick: Some(Box::new(move |request: PickRequest| {
            start_pick(&pick_state, &pick_bridge, request)
        })),
        ..Services::default()
    }
}

struct VectorCraftAndroidApp {
    inner: VectorcraftApp,
    bridge: AndroidFileBridge,
    picks: Arc<PickState>,
    ime: ime_bridge::ime::ImeBridge,
    modifiers: egui::Modifiers,
    persist: Option<std::path::PathBuf>,
    persist_last: Option<std::time::Instant>,
    // Files the system asked us to open are staged by the Activity; this is the shared queue the
    // app's own inbox drains. `intent_warmup` skips the first couple of frames so the app has
    // finished its own start-up, and goes negative (stop polling) if the host has no intent bridge.
    intent_inbox: Inbox,
    intent_warmup: i64,
    // Staged working copies under media/ the app still refers to (see `staged_prune_tick`), and
    // when the one-time orphan sweep of the directory is due.
    staged_seen: Vec<String>,
    staged_sweep_at: Option<std::time::Instant>,
}

/// Read the files the system asked this app to open (an `ACTION_VIEW`/`ACTION_SEND` it is registered
/// for) into the app's import inbox, exactly like a document chosen in the picker. Each staged path
/// is read once; a launcher without the intent host stops the poll after one error.
/// Watch for files the system (or another app's drop) handed the app while it was idle.
///
/// egui runs a frame only when something asks it to, so a file that arrives while the app sits
/// untouched would wait in the Activity's queue until the user happened to touch the screen. The
/// watcher asks the Activity how many files are waiting (without taking any of them) and asks
/// egui for a repaint when there is one, so the file opens on its own. The file itself is still
/// opened on the frame thread, in `drain_incoming`.
fn watch_incoming(bridge: &AndroidFileBridge, ctx: &egui::Context) {
    static WATCHER: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    if WATCHER.set(()).is_err() {
        return;
    }
    let bridge = bridge.clone_for_services();
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

fn drain_incoming(bridge: &AndroidFileBridge, inbox: &Inbox, warmup: &mut i64) {
    if *warmup > 0 {
        *warmup -= 1;
        return;
    }
    if *warmup < 0 {
        return;
    }
    for _ in 0..16 {
        match bridge.intent_poll() {
            Ok(Some(path)) => match fs::read(&path) {
                Ok(bytes) => {
                    let name = Path::new(&path)
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "imported-file".to_string());
                    log::info!("Opening the file the system handed the app: {path}");
                    inbox.lock().unwrap_or_else(|e| e.into_inner()).push((name, bytes));
                }
                Err(error) => log::error!("Could not read the incoming file {path}: {error}"),
            },
            Ok(None) => break,
            Err(error) => {
                log::info!("Launcher does not report incoming files: {error}");
                *warmup = -1;
                break;
            }
        }
    }
}

impl VectorCraftAndroidApp {
    /// The staged copy of a document, if `path` names one: a *direct* child of the app's own
    /// `media/` directory (see VectorCraftActivity.newStageFile). Anything else on the device is
    /// never touched.
    fn staged_media_file(&self, path: &str) -> Option<PathBuf> {
        let files = self.bridge.files_dir().ok()?;
        let media = Path::new(&files).join("media");
        let file = PathBuf::from(path);
        (file.parent() == Some(media.as_path())).then_some(file)
    }

    /// Delete every staged copy under `media/` that neither the recent list nor an open document
    /// refers to, sparing files written in the last minute (a document may be mid-import). This
    /// clears out copies orphaned before this pruning existed, or by an app killed mid-session.
    fn staged_sweep_orphans(bridge: &AndroidFileBridge, recent: &[String], open: &[String]) {
        let Ok(files) = bridge.files_dir() else { return };
        let media = Path::new(&files).join("media");
        let Ok(entries) = fs::read_dir(&media) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if recent.iter().any(|keep| Path::new(keep) == path.as_path())
                || open.iter().any(|keep| Path::new(keep) == path.as_path())
            {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }
            let fresh = meta
                .modified()
                .ok()
                .and_then(|at| at.elapsed().ok())
                .is_some_and(|age| age < std::time::Duration::from_secs(60));
            if fresh {
                continue;
            }
            let name = path.to_string_lossy().into_owned();
            match fs::remove_file(&path) {
                Ok(()) => log::info!("deleted an orphaned staged copy: {name}"),
                Err(error) => log::warn!("could not delete the staged copy {name}: {error}"),
            }
        }
    }

    /// When an entry leaves the recent-files list (the list is cleared, an entry is removed with
    /// its ×, or it ages out of the capped list) and no open document is showing that file, its
    /// staged copy under `media/` is deleted: the document's original still lives at its SAF
    /// location. The `uri:` mapping in SharedPreferences is deliberately kept, so re-opening the
    /// same document reuses the same path and a later save still reaches the original. A delayed
    /// sweep of the whole directory clears out copies this diff never saw (an older session, or a
    /// crash); it runs once, half a minute after start, to let session restore finish first.
    fn staged_prune_tick(&mut self) {
        let current: Vec<String> = self.inner.ui.recent_files.clone();
        let open: Vec<String> = self
            .inner
            .session
            .documents()
            .iter()
            .filter_map(|doc| doc.path.clone())
            .collect::<Vec<_>>();
        for stale in self
            .staged_seen
            .iter()
            .filter(|path| !current.contains(*path) && !open.contains(*path))
        {
            let Some(file) = self.staged_media_file(stale) else { continue };
            match fs::remove_file(&file) {
                Ok(()) => log::info!("deleted the staged copy of a recent file the app let go: {stale}"),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => log::warn!("could not delete the staged copy {stale}: {error}"),
            }
        }
        if self.staged_sweep_at.is_some_and(|at| std::time::Instant::now() >= at) {
            self.staged_sweep_at = None;
            Self::staged_sweep_orphans(&self.bridge, &current, &open);
        }
        self.staged_seen = current;
    }

    /// Save settings, preferences and the recent-files list, at most every two seconds. Android
    /// can kill a backgrounded app without a clean exit, so waiting for `on_exit` alone would lose
    /// whatever changed since the last write.
    fn persist_tick(&mut self) {
        let Some(dir) = self.persist.clone() else { return };
        let now = std::time::Instant::now();
        if self.persist_last.is_some_and(|last| now.duration_since(last) < std::time::Duration::from_secs(2)) {
            return;
        }
        self.persist_last = Some(now);
        persist_save(&mut self.inner, &dir);
    }

    /// Save now: the window lost focus (the app is being backgrounded) or it is exiting.
    fn persist_now(&mut self) {
        if let Some(dir) = self.persist.clone() {
            persist_save(&mut self.inner, &dir);
        }
    }
}

impl Drop for VectorCraftAndroidApp {
    fn drop(&mut self) {
        // The Activity is about to go: stop the watcher before the native thread returns.
        ALIVE.store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

impl eframe::App for VectorCraftAndroidApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Deliver picker answers before the app's own logic runs, so `picks::poll` resumes the
        // entry that asked in this same frame.
        poll_answers(&self.picks, &self.bridge, ctx);
        // Wake the app when a file the system handed us is waiting, then deliver it through the
        // same inbox.
        watch_incoming(&self.bridge, ctx);
        drain_incoming(&self.bridge, &self.intent_inbox, &mut self.intent_warmup);
        self.staged_prune_tick();
        self.inner.logic(ctx);
        self.persist_tick();
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.ime.poll(raw);
        apply_android_modifiers(&mut self.modifiers, raw);
        if raw.events.iter().any(|event| matches!(event, egui::Event::WindowFocused(false))) {
            self.persist_now();
        }
        self.inner.raw_input_hook(raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        system_fonts::install_system_fonts(ui.ctx());
        self.inner.ui(ui);
    }

    fn on_exit(&mut self) {
        self.persist_now();
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(android_app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("com.artcraft.vectorcraft"),
    );
    log::info!("{SAF_BRIDGE_MARKER} ready; {ANDROID_SHELL_MARKER}; {}", ime_bridge::ime::IME_MARKER);
    egui_winit::android_activity::set_activity(android_app.activity_as_ptr());
    install_persist_env(&android_app);
    system_fonts::install_font_env(&android_app);
    let bridge = match AndroidFileBridge::new(android_app.clone()) {
        Ok(bridge) => bridge,
        Err(error) => {
            log::error!("VectorCraft Android startup failed: {error}");
            return;
        }
    };
    if let Ok(dir) = bridge.files_dir() {
        log::info!("VectorCraft app-private storage: {dir}");
    }
    let picks = Arc::new(PickState::default());
    let intent_inbox: Inbox = Arc::new(Mutex::new(Vec::new()));
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("VectorCraft")
            .with_drag_and_drop(true),
        android_app: Some(android_app),
        ..Default::default()
    };
    let app_bridge = bridge.clone_for_services();
    let app_picks = picks.clone();
    let app_intent_inbox = intent_inbox.clone();
    let result = eframe::run_native(
        "VectorCraft",
        options,
        Box::new(move |_cc| {
            let mut app = VectorcraftApp::new(
                Session::new(),
                services(app_bridge.clone_for_services(), app_picks.clone(), app_intent_inbox.clone()),
            );
            let persist = persist_dir(&app_bridge.app);
            if let Some(dir) = &persist {
                // Load the saved settings *first*, so the device's default UI scale below is not
                // overwritten by `prefs_dialog::restore` (which rebuilds the prefs from the saved
                // state, or from the defaults when there is none).
                persist_load(&mut app, dir);
                // VectorCraft draws its work on a canvas, so a high-density screen gets a *smaller*
                // interface (75 %), not a larger one; every other screen keeps the app's own
                // default, Auto. Applied once (`device_scale_first_time`), which covers a first
                // launch and an upgrade from a build without this bootstrap; a user who sets the
                // scale by hand keeps it, since the sidecar already says it was applied.
                if device_is_high_density(&app_bridge.app) == Some(true) && device_scale_first_time(dir) {
                    let _ = app.session.execute("prefs.set", &serde_json::json!({"key": "uiScaling", "value": 0.75}));
                    persist_save(&mut app, dir);
                }
            }
            Ok(Box::new(VectorCraftAndroidApp {
                inner: app,
                bridge: app_bridge.clone_for_services(),
                picks: app_picks.clone(),
                ime: ime_bridge::ime::ImeBridge::new(&app_bridge.app),
                modifiers: egui::Modifiers::default(),
                persist,
                persist_last: None,
                intent_inbox: app_intent_inbox.clone(),
                intent_warmup: 2,
                staged_seen: Vec::new(),
                staged_sweep_at: Some(std::time::Instant::now() + std::time::Duration::from_secs(30)),
            }))
        }),
    );
    if let Err(error) = result {
        log::error!("VectorCraft Android exited with an error: {error}");
    }
}

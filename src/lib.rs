//! Builder-owned Android shell for PhotoCraft.
//! Platform file operations use Android's Storage Access Framework (SAF). Selected documents
//! are staged inside app-private storage, with a persistent URI mapping used to write saved bytes
//! back through ContentResolver. The upstream PhotoCraft crates remain unchanged.
//! ARTCRAFT_PHOTOCRAFT_SAF_JNI022_FIX_V1

use android_activity::AndroidApp;
use jni::{
    JavaVM, jni_sig, jni_str,
    objects::{JObject, JString, JValue},
};
use photocraft_codecs::{ChannelLayout, EncodeOptions, Image};
use photocraft_doc::Document;
use photocraft_engine::Session;
use photocraft_format::RecoveryStore;
use photocraft_ui_egui::{
    FileDialogAnswer, FileDialogReply, FileDialogRequest, Inbox, PhotocraftApp, Recoverable,
    Services,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

// Shared with the generated launchers: brings the device's own system fonts into the egui fallback
// chain and makes them visible to the app's own font scanners (see templates/android/system_fonts.rs).
mod system_fonts;

// Shared with the generated launchers: turns Android's soft keyboard into egui IME events (see
// templates/android/ime_bridge.rs). The `mod ime_bridge;` marker also keeps the vendored
// android-activity, which forwards show/hide to this Activity's imeShow/imeHide, in step.
mod ime_bridge;

/// Emitted at startup so a device log (and the packaged `.so` strings) can confirm this shell —
/// including the Android modifier-key fix and immersive fullscreen — is the one that is running.
const ANDROID_SHELL_MARKER: &str = "ARTCRAFT_PHOTOCRAFT_ANDROID_SHELL_V2_MODIFIERS_FULLSCREEN";

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

    fn request_open(&self, id: u64, multiple: bool) -> Result<(), String> {
        self.vm
            .attach_current_thread(|env| -> jni::errors::Result<()> {
                // AndroidApp returns an unowned global ref. Do not delete it when this wrapper drops.
                let activity_raw = self.app.activity_as_ptr() as jni::sys::jobject;
                // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it
                // without taking ownership or attempting to delete the reference.
                let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
                env.call_method(
                    &*activity,
                    jni_str!("pcRequestOpen"),
                    jni_sig!("(JZ)V"),
                    &[JValue::Long(id as i64), JValue::Bool(multiple)],
                )?;
                Ok(())
            })
            .map_err(|e| e.to_string())
    }

    fn request_save(&self, id: u64, name: &str) -> Result<(), String> {
        self.vm
            .attach_current_thread(|env| -> jni::errors::Result<()> {
                let activity_raw = self.app.activity_as_ptr() as jni::sys::jobject;
                // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it
                // without taking ownership or attempting to delete the reference.
                let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
                let name = env.new_string(name)?;
                let name_obj = JObject::from(name);
                env.call_method(
                    &*activity,
                    jni_str!("pcRequestSave"),
                    jni_sig!("(JLjava/lang/String;)V"),
                    &[JValue::Long(id as i64), JValue::Object(&name_obj)],
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
                        jni_str!("pcPollPickerResult"),
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

    fn files_dir(&self) -> Result<PathBuf, String> {
        self.vm
            .attach_current_thread(|env| -> jni::errors::Result<PathBuf> {
                let activity_raw = self.app.activity_as_ptr() as jni::sys::jobject;
                // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it
                // without taking ownership or attempting to delete the reference.
                let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
                let value = env
                    .call_method(
                        &*activity,
                        jni_str!("pcFilesDir"),
                        jni_sig!("()Ljava/lang/String;"),
                        &[],
                    )?
                    .l()?;
                let value = JString::cast_local(env, value)?;
                let value: String = value.try_to_string(env)?;
                Ok(PathBuf::from(value))
            })
            .map_err(|e| e.to_string())
    }

    /// Returns NONE for ordinary private paths, OK after committing a selected SAF destination,
    /// or ERROR:<message> if the Android provider refused the write.
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
                        jni_str!("pcCommitIfMapped"),
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

#[derive(Default)]
struct PickerState {
    next_id: AtomicU64,
    replies: Mutex<HashMap<u64, FileDialogReply>>,
}

impl PickerState {
    fn launch(
        &self,
        bridge: &AndroidFileBridge,
        request: FileDialogRequest,
        reply: FileDialogReply,
    ) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        let launch_result = match request {
            FileDialogRequest::Open { multiple, .. } => {
                self.replies
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(id, reply);
                bridge.request_open(id, multiple)
            }
            FileDialogRequest::Save { suggested } => {
                let name = Path::new(&suggested)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "Untitled.pcraft".to_string());
                self.replies
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(id, reply);
                bridge.request_save(id, &name)
            }
            FileDialogRequest::Folder { .. } => {
                reply.send(Some(FileDialogAnswer::Unavailable(
                    "Choosing folders is not supported by this Android file picker yet. Use Open or Save As for individual files.".into(),
                )));
                return;
            }
        };
        if let Err(error) = launch_result {
            if let Some(reply) = self
                .replies
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id)
            {
                reply.send(Some(FileDialogAnswer::Unavailable(format!(
                    "Could not start Android file picker: {error}"
                ))));
            }
        }
    }

    fn poll(&self, bridge: &AndroidFileBridge, ctx: &egui::Context) {
        // Each result is one short JSON object queued by PhotoCraftActivity after picker I/O ends.
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
            let Some(reply) = self
                .replies
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id)
            else {
                log::warn!("Ignoring stale Android picker reply {id}");
                continue;
            };
            let status = value
                .get("status")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("error");
            let kind = value
                .get("kind")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let answer = match status {
                "cancel" => None,
                "ok" if kind == "open" => {
                    let paths = value
                        .get("paths")
                        .and_then(serde_json::Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(serde_json::Value::as_str)
                        .map(str::to_owned)
                        .collect::<Vec<_>>();
                    (!paths.is_empty()).then_some(FileDialogAnswer::Paths(paths))
                }
                "ok" if kind == "save" => value
                    .get("path")
                    .and_then(serde_json::Value::as_str)
                    .map(|path| FileDialogAnswer::SaveTo(path.to_owned())),
                _ => Some(FileDialogAnswer::Unavailable(
                    value
                        .get("message")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("Android file operation failed")
                        .to_owned(),
                )),
            };
            reply.send(answer);
            ctx.request_repaint();
        }
        if !self.replies.lock().unwrap_or_else(|e| e.into_inner()).is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
}

fn write_mobile(bridge: &AndroidFileBridge, path: &str, bytes: &[u8]) -> Result<(), String> {
    let target = Path::new(path);
    if let Some(parent) = target.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Could not create {}: {e}", parent.display()))?;
    }
    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("photocraft-output");
    let parent = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let temporary = parent.join(format!(".{name}.tmp-{}", std::process::id()));
    fs::write(&temporary, bytes)
        .map_err(|e| format!("Could not write {}: {e}", temporary.display()))?;
    if let Err(error) = fs::rename(&temporary, target) {
        let _ = fs::remove_file(&temporary);
        return Err(format!(
            "Could not finish writing {}: {error}",
            target.display()
        ));
    }
    match bridge.commit_if_mapped(path)? {
        state if state == "NONE" || state == "OK" => Ok(()),
        state if state.starts_with("ERROR:") => Err(state.trim_start_matches("ERROR:").to_owned()),
        state => Err(format!("Unexpected Android save result: {state}")),
    }
}

type SharedRecovery = Rc<RefCell<Option<RecoveryStore>>>;

fn with_store<R>(store: &SharedRecovery, f: impl FnOnce(&mut RecoveryStore) -> R) -> Result<R, String> {
    let mut slot = store.try_borrow_mut().map_err(|_| "crash recovery is busy".to_string())?;
    Ok(f(slot.as_mut().ok_or("no config directory")?))
}

fn services(bridge: AndroidFileBridge, picker: Arc<PickerState>, prefs_path: PathBuf, inbox: Inbox) -> Services {
    let write_bridge = bridge.clone_for_services();
    let save_bridge = bridge.clone_for_services();
    let prefs_bridge = bridge.clone_for_services();
    let load_prefs_path = prefs_path.clone();

    let recovery_dir = prefs_path.parent().map(|p| p.join("recovery"));
    let recover_dir = recovery_dir.clone();
    let store: SharedRecovery = Rc::new(RefCell::new(recovery_dir.map(RecoveryStore::new)));
    let (s1, s2, s4, s5) = (store.clone(), store.clone(), store.clone(), store.clone());

    Services {
        // Files the system asked us to open (ACTION_VIEW/ACTION_SEND) are queued here and drained
        // by the app's own per-frame inbox, exactly like a document chosen in the picker.
        inbox: Some(inbox),
        import: Some(Box::new(
            |name: &str, bytes: &[u8], opts: &photocraft_io::ImportOptions| {
                photocraft_io::import_with_options(name, bytes, opts)
                    .map(|r| (r.document, r.warnings))
                    .map_err(|e| e.to_string())
            },
        )),
        export: Some(Box::new(
            |doc: &Document, path: &str, settings: &photocraft_ui_egui::ExportSettings| {
                let mut opts = photocraft_io::ExportOptions::default();
                if let Some(q) = settings.jpeg_quality {
                    opts.encode.jpeg_quality = q;
                }
                opts.encode.webp_lossless = settings.webp_lossless;
                if let Some(q) = settings.webp_quality {
                    opts.encode.webp_quality = q;
                }
                opts.tiff_layers = settings.tiff_layers;
                opts.xmp = if settings.xmp_all {
                    photocraft_io::XmpEmbed::All
                } else {
                    photocraft_io::XmpEmbed::None
                };
                opts.tga_bits = settings.tga_bits;
                opts.image_previews = settings.image_previews;
                opts.psd_maximize_compatibility = settings.psd_maximize_compatibility;
                photocraft_io::export(doc, path, &opts)
                    .map(|r| (r.bytes, r.warnings))
                    .map_err(|e| e.to_string())
            },
        )),
        // Native .pcraft Save uses save_file rather than export. Route it through the same SAF-aware writer.
        save_file: Some(std::sync::Arc::new(
            move |doc: &Document,
                  path: &str,
                  settings: &photocraft_ui_egui::ExportSettings,
                  ctl: &photocraft_ui_egui::jobs_ui::SaveCtl| {
                ctl.progress(0.0, "Encoding");
                let mut opts = photocraft_io::ExportOptions::default();
                if let Some(q) = settings.jpeg_quality {
                    opts.encode.jpeg_quality = q;
                }
                opts.encode.webp_lossless = settings.webp_lossless;
                if let Some(q) = settings.webp_quality {
                    opts.encode.webp_quality = q;
                }
                opts.tiff_layers = settings.tiff_layers;
                opts.xmp = if settings.xmp_all {
                    photocraft_io::XmpEmbed::All
                } else {
                    photocraft_io::XmpEmbed::None
                };
                opts.tga_bits = settings.tga_bits;
                opts.image_previews = settings.image_previews;
                opts.psd_maximize_compatibility = settings.psd_maximize_compatibility;
                let result = photocraft_io::export(doc, path, &opts).map_err(|e| e.to_string())?;
                ctl.commit()?;
                ctl.progress(0.85, "Writing");
                write_mobile(&save_bridge, path, &result.bytes)?;
                ctl.progress(1.0, "");
                Ok(result.warnings)
            },
        )),
        file_dialog: Some(Box::new(move |request, _parent, reply| {
            picker.launch(&bridge, request, reply);
        })),
        write: Some(Box::new(move |path: &str, bytes: &[u8]| {
            write_mobile(&write_bridge, path, bytes)
        })),
        load_prefs: Some(Box::new(move || fs::read_to_string(&load_prefs_path).ok())),
        save_prefs: Some(Box::new(move |text: &str| {
            write_mobile(
                &prefs_bridge,
                &prefs_path.to_string_lossy(),
                text.as_bytes(),
            )
        })),
        encode_png: Some(Box::new(|w, h, rgba| {
            let img = Image::from_u8(w, h, ChannelLayout::Rgba, rgba.to_vec())
                .map_err(|e| e.to_string())?;
            photocraft_codecs::encode(
                &img,
                photocraft_codecs::Format::Png,
                &EncodeOptions::default(),
            )
            .map_err(|e| e.to_string())
        })),
        autosave: Some(Box::new(move |doc: &Arc<Document>, revision: u64, path: Option<&str>| {
            with_store(&s1, |s| s.autosave_checked(doc, revision, path.map(str::to_string)))?
        })),
        autosave_results: Some(Box::new(move || with_store(&s4, RecoveryStore::take_completed).unwrap_or_default())),
        flush_autosaves: Some(Box::new(move || with_store(&s5, RecoveryStore::flush).unwrap_or_default())),
        discard_autosave: Some(Box::new(move |id: u64| {
            let _ = with_store(&s2, |s| s.discard(id));
        })),
        recover: Some(Box::new(move || {
            let Some(dir) = recover_dir.as_ref() else { return Vec::new() };
            photocraft_format::list_recovery(dir)
                .into_iter()
                .map(|e| Recoverable {
                    key: e.info.key.clone(),
                    name: e.info.document_name.clone(),
                    path: e.info.original_path.clone(),
                    load: Box::new(move || photocraft_format::recover(&e).map_err(|e| e.to_string())),
                })
                .collect()
        })),
        adopt_autosave: Some(Box::new(move |id: u64, key: &str| {
            let _ = with_store(&store, |s| s.adopt(id, key));
        })),
        ..Services::default()
    }
}

// Keep the JavaVM and the AndroidApp cheap to clone into platform-service closures.
impl AndroidFileBridge {
    fn clone_for_services(&self) -> Self {
        Self {
            app: self.app.clone(),
            vm: self.vm.clone(),
        }
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


struct PhotoCraftAndroidApp {
    inner: PhotocraftApp,
    bridge: AndroidFileBridge,
    picker: Arc<PickerState>,
    ime: ime_bridge::ime::ImeBridge,
    modifiers: egui::Modifiers,
    // Files the system asked us to open are staged by the Activity; this is the shared queue the
    // app's own inbox drains. `intent_warmup` skips the first couple of frames so the app has
    // finished its own start-up, and goes negative (stop polling) if the host has no intent bridge.
    intent_inbox: Inbox,
    intent_warmup: i64,
    // Staged working copies under media/ the app still refers to (see `staged_prune_tick`), and
    // when the one-time orphan sweep of the directory is due.
    staged_seen: Vec<String>,
    staged_sweep_at: Option<std::time::Instant>,
    persist_last: Option<std::time::Instant>,
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

impl PhotoCraftAndroidApp {
    /// The staged copy of a document, if `path` names one: a *direct* child of the app's own
    /// `media/` directory (see PhotoCraftActivity.newStageFile). Anything else on the device is
    /// never touched.
    fn staged_media_file(&self, path: &str) -> Option<PathBuf> {
        let files = self.bridge.files_dir().ok()?;
        let media = files.join("media");
        let file = PathBuf::from(path);
        (file.parent() == Some(media.as_path())).then_some(file)
    }

    /// Delete every staged copy under `media/` that neither the recent list nor an open document
    /// refers to, sparing files written in the last minute (a document may be mid-import). This
    /// clears out copies orphaned before this pruning existed, or by an app killed mid-session.
    fn staged_sweep_orphans(bridge: &AndroidFileBridge, recent: &[String], open: &[String]) {
        let Ok(files) = bridge.files_dir() else { return };
        let media = files.join("media");
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

    fn persist_tick(&mut self) {
        let now = std::time::Instant::now();
        if self.persist_last.is_some_and(|last| now.duration_since(last) < std::time::Duration::from_secs(2)) {
            return;
        }
        self.persist_last = Some(now);
        self.persist_now();
    }

    fn persist_now(&mut self) {
        if let Some(flush) = self.inner.services.flush_autosaves.as_mut() {
            let _ = flush();
        }
    }
}

impl Drop for PhotoCraftAndroidApp {
    fn drop(&mut self) {
        // The Activity is about to go: stop the watcher before the native thread returns.
        ALIVE.store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

impl eframe::App for PhotoCraftAndroidApp {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.picker.poll(&self.bridge, ctx);
        // Wake the app when a file the system handed us is waiting, then deliver it through the
        // app's own inbox.
        watch_incoming(&self.bridge, ctx);
        drain_incoming(&self.bridge, &self.intent_inbox, &mut self.intent_warmup);
        self.staged_prune_tick();
        self.persist_tick();
        self.inner.logic(ctx, frame);
        if self.inner.session.has_jobs() || !self.inner.jobs.opens.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        system_fonts::install_system_fonts(ui.ctx());
        self.inner.ui(ui, _frame);
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.ime.poll(raw);
        apply_android_modifiers(&mut self.modifiers, raw);
        if raw.events.iter().any(|event| matches!(event, egui::Event::WindowFocused(false))) {
            self.persist_now();
        }
        self.inner.raw_input_hook(ctx, raw);
    }

    fn on_exit(&mut self) {
        self.persist_now();
    }
}

/// Screen below 600 dpi (phones) applies the scale; denser screens stay at Auto.
#[cfg(target_os = "android")]
fn device_is_high_density(app: &AndroidApp) -> Option<bool> {
    // SAFETY: AndroidApp owns the live VM pointer for the lifetime of the activity.
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
            log::warn!("Could not determine the screen density: {error}");
            None
        }
    }
}

/// Whether the high-density UI scale should be applied to this launch, and the sidecar that
/// remembers that it was.
///
/// The scale belongs to the *device*, but `preferences.json` belongs to the *user*: it is written on
/// the first launch and never revisited, so a screen that later stops being high-density (a
/// foldable, an external display, a display-size change) kept the old scale. The choice is
/// remembered in its own sidecar (`device_class`), so "first launch" means "the scale has not been
/// set for this device yet" and a user who changes the scale by hand keeps it: the sidecar already
/// says it was applied, so nothing is re-applied.
#[cfg(target_os = "android")]
fn device_scale_first_time(dir: &Path) -> bool {
    let marker = dir.join("device_class");
    if fs::read_to_string(&marker).ok().is_some_and(|saved| saved.trim() == "scaled") {
        return false;
    }
    let path = dir.join("device_class");
    let temporary = path.with_extension("tmp");
    if let Err(error) = fs::create_dir_all(dir).and_then(|()| fs::write(&temporary, b"scaled")).and_then(|()| fs::rename(&temporary, &path)) {
        log::warn!("settings: {}: {error}", path.display());
    }
    true
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(android_app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("com.artcraft.photocraft"),
    );
    log::info!("{ANDROID_SHELL_MARKER} ready; {}", ime_bridge::ime::IME_MARKER);
    egui_winit::android_activity::set_activity(android_app.activity_as_ptr());
    system_fonts::install_font_env(&android_app);
    let bridge = match AndroidFileBridge::new(android_app.clone()) {
        Ok(bridge) => bridge,
        Err(error) => {
            log::error!("PhotoCraft Android startup failed: {error}");
            return;
        }
    };
    let files_dir = match bridge.files_dir() {
        Ok(path) => path,
        Err(error) => {
            log::error!("Could not locate PhotoCraft app-private storage: {error}");
            return;
        }
    };
    let prefs_path = files_dir.join("PhotoCraft").join("preferences.json");
    if let Some(parent) = prefs_path.parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            log::error!("Could not create preferences directory: {error}");
        }
    }
    let picker = Arc::new(PickerState::default());
    let intent_inbox: Inbox = Arc::new(Mutex::new(Vec::new()));
    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("PhotoCraft")
            .with_drag_and_drop(true),
        renderer: eframe::Renderer::Wgpu,
        android_app: Some(android_app),
        ..Default::default()
    };
    // The adapter's real texture and buffer limits, so large documents stay on the GPU canvas;
    // Vulkan first, GLES on devices without a usable Vulkan driver — the same setup the desktop
    // app and the generated launchers use.
    photocraft_ui_egui::gpu_canvas::use_adapter_limits(&mut options.wgpu_options.wgpu_setup);
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup {
        create.instance_descriptor.backends =
            eframe::wgpu::Backends::VULKAN | eframe::wgpu::Backends::GL;
    }
    let app_bridge = bridge.clone_for_services();
    let app_picker = picker.clone();
    let app_intent_inbox = intent_inbox.clone();
    let result = eframe::run_native(
        "PhotoCraft",
        options,
        Box::new(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, photocraft_ui_egui::theme::ThemeKind::Pro);
            let mut app = PhotocraftApp::new(
                Session::new(),
                services(
                    app_bridge.clone_for_services(),
                    app_picker.clone(),
                    prefs_path.clone(),
                    app_intent_inbox.clone(),
                ),
            );
            app.background_jobs = true;
            // A high-density screen gets the app's 150 % UI scale; every other screen keeps its own
            // default, Auto. Applied once (`device_scale_first_time`), which covers a first launch
            // and an upgrade from a build without this bootstrap; a user who sets the scale by hand
            // keeps it, since the sidecar already says it was applied. The app saves it through
            // `save_prefs` on its normal schedule.
            let class_dir = prefs_path.parent().unwrap_or_else(|| Path::new("."));
            if device_is_high_density(&app_bridge.app) == Some(true) && device_scale_first_time(class_dir) {
                let _ = app.session.execute("prefs.set", serde_json::json!({"values": {"interface.uiScale": "150"}}));
            }
            // Hand the canvas the live render state when the device is a real GPU, and record the
            // adapter for Help › System Info either way — without this the canvas stays on the CPU
            // compositor and System Info reports no adapter ("unknown").
            if let Some(rs) = cc.wgpu_render_state.clone() {
                let info = rs.adapter.get_info();
                log::info!("wgpu adapter: {} ({:?})", info.name, info.backend);
                app.perf.gpu_info.set_adapter(&info);
                if info.device_type != eframe::wgpu::DeviceType::Cpu {
                    app.set_wgpu(rs);
                }
            }
            Ok(Box::new(PhotoCraftAndroidApp {
                inner: app,
                bridge: app_bridge.clone_for_services(),
                picker: app_picker.clone(),
                ime: ime_bridge::ime::ImeBridge::new(&app_bridge.app),
                modifiers: egui::Modifiers::default(),
                intent_inbox: app_intent_inbox.clone(),
                intent_warmup: 2,
                staged_seen: Vec::new(),
                staged_sweep_at: Some(std::time::Instant::now() + std::time::Duration::from_secs(30)),
                persist_last: None,
            }))
        }),
    );
    if let Err(error) = result {
        log::error!("PhotoCraft Android exited with an error: {error}");
    }
}

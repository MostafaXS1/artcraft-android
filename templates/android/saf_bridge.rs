// Builder-generated Android Storage Access Framework bridge (shared module).
//
// The generated Activity (see templates/android/SafActivity.java) answers the system open/save
// pickers, stages the chosen file inside app-private storage and remembers the chosen SAF
// destination URI. This module is the Rust side: it drives the picker over JNI and polls answers
// once per frame. The upstream ArtCraft crates are never modified.
//
// ARTCRAFT_ANDROID_SAF_BRIDGE_V3
mod saf {
    use android_activity::AndroidApp;
    use jni::{
        JavaVM, jni_sig, jni_str,
        objects::{JObject, JString, JValue},
    };
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// Emitted at startup so a device log (and the packaged `.so` strings) can confirm this shell —
    /// including the SAF file bridge — is the one running.
    pub const SAF_MARKER: &str = "ARTCRAFT_ANDROID_SAF_BRIDGE_V3";

    /// How long a synchronous picker waits for the user before giving up (the app carries on as if
    /// the dialog was cancelled; the SAF picker itself has no timeout).
    pub const PICK_TIMEOUT: Duration = Duration::from_secs(180);

    /// The answer a blocking picker returned.
    pub enum PickAnswer {
        /// The chosen (staged, app-private) paths. A save dialog has exactly one.
        Paths(Vec<String>),
        /// The user dismissed the picker.
        Cancelled,
    }

    /// RAII gate for the duration of a blocking pick: while alive, JVM-thread lifecycle
    /// callbacks (`set_input`/`set_window`/`set_activity_state`, …) in the vendored
    /// `android-activity` glue no longer block waiting for the Rust main thread to pump
    /// events. A blocking pick *is* that parked main thread (or otherwise runs long
    /// enough that Android would ANR the ~5s wait), so the glue delivers its state
    /// through the command pipe instead and the main thread consumes it after the pick.
    /// Restored on every exit path — including panics — by `Drop`.
    struct NonblockingCallbacks;

    impl NonblockingCallbacks {
        fn enable() -> Self {
            android_activity::set_nonblocking_callbacks(true);
            Self
        }
    }

    impl Drop for NonblockingCallbacks {
        fn drop(&mut self) {
            android_activity::set_nonblocking_callbacks(false);
        }
    }

    /// Thin JNI wrapper around the generated `NativeActivity` subclass. `android_main` runs on its
    /// own Rust thread, so every picker request is asynchronous: this only asks Java to start the
    /// picker and polls the staged answers later.
    pub struct Bridge {
        app: AndroidApp,
        vm: Arc<JavaVM>,
    }

    impl Bridge {
        pub fn new(app: &AndroidApp) -> Self {
            // SAFETY: AndroidApp owns the live VM pointer for the lifetime of the activity.
            let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr() as *mut _) };
            Self {
                app: app.clone(),
                vm: Arc::new(vm),
            }
        }

        pub fn clone_ref(&self) -> Self {
            Self {
                app: self.app.clone(),
                vm: Arc::clone(&self.vm),
            }
        }

        /// Ask the Activity (once, remembered) for broad device file access, so the app's own file
        /// browsers can read and write files beyond the ones picked through SAF. Fire-and-forget:
        /// the user answers in a system screen and the app carries on either way.
        pub fn request_storage_permission(&self) {
            let result = self.vm.attach_current_thread(|env| -> jni::errors::Result<()> {
                // AndroidApp returns an unowned global ref; as_cast_raw borrows it.
                let activity_raw = self.app.activity_as_ptr() as jni::sys::jobject;
                // SAFETY: AndroidApp retains this global Activity reference.
                let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
                env.call_method(
                    &*activity,
                    jni_str!("safRequestStoragePermission"),
                    jni_sig!("()V"),
                    &[],
                )?;
                Ok(())
            });
            if let Err(error) = result {
                log::error!("Could not request Android file access: {error}");
            }
        }

        pub fn request_open(&self, id: u64, multiple: bool) -> Result<(), String> {
            self.vm
                .attach_current_thread(|env| -> jni::errors::Result<()> {
                    // AndroidApp returns an unowned global ref. Do not delete it when this wrapper drops.
                    let activity_raw = self.app.activity_as_ptr() as jni::sys::jobject;
                    // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it
                    // without taking ownership or attempting to delete the reference.
                    let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
                    env.call_method(
                        &*activity,
                        jni_str!("safRequestOpen"),
                        jni_sig!("(JZ)V"),
                        &[JValue::Long(id as i64), JValue::Bool(multiple)],
                    )?;
                    Ok(())
                })
                .map_err(|e| e.to_string())
        }

        pub fn request_save(&self, id: u64, name: &str) -> Result<(), String> {
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
                        jni_str!("safRequestSave"),
                        jni_sig!("(JLjava/lang/String;)V"),
                        &[JValue::Long(id as i64), JValue::Object(&name_obj)],
                    )?;
                    Ok(())
                })
                .map_err(|e| e.to_string())
        }

        pub fn request_folder(&self, id: u64) -> Result<(), String> {
            self.vm
                .attach_current_thread(|env| -> jni::errors::Result<()> {
                    let activity_raw = self.app.activity_as_ptr() as jni::sys::jobject;
                    // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it.
                    let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
                    env.call_method(&*activity, jni_str!("safRequestFolder"), jni_sig!("(J)V"), &[JValue::Long(id as i64)])?;
                    Ok(())
                })
                .map_err(|e| e.to_string())
        }

        /// Hand a generated PDF to Android's print system (the native print spooler). Java answers
        /// synchronously — `"OK"` once the print dialog has been queued on the main thread, or
        /// `"ERROR: …"` when no print service exists — so the caller gets its receipt (or failure)
        /// inside the same frame while the dialog itself opens asynchronously.
        pub fn print_document(&self, title: &str, pdf: &[u8]) -> Result<String, String> {
            self.vm
                .attach_current_thread(|env| -> jni::errors::Result<String> {
                    let activity_raw = self.app.activity_as_ptr() as jni::sys::jobject;
                    // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it.
                    let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
                    let title_obj = JObject::from(env.new_string(title)?);
                    let bytes_obj = JObject::from(env.byte_array_from_slice(pdf)?);
                    let value = env
                        .call_method(
                            &*activity,
                            jni_str!("printDocument"),
                            jni_sig!("(Ljava/lang/String;[B)Ljava/lang/String;"),
                            &[JValue::Object(&title_obj), JValue::Object(&bytes_obj)],
                        )?
                        .l()?;
                    if value.is_null() {
                        return Ok(String::new());
                    }
                    let value = JString::cast_local(env, value)?;
                    value.try_to_string(env)
                })
                .map_err(|e| e.to_string())
        }

        pub fn poll(&self) -> Result<Option<String>, String> {
            self.vm
                .attach_current_thread(|env| -> jni::errors::Result<Option<String>> {
                    let activity_raw = self.app.activity_as_ptr() as jni::sys::jobject;
                    // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it
                    // without taking ownership or attempting to delete the reference.
                    let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
                    let value = env
                        .call_method(&*activity, jni_str!("safPoll"), jni_sig!("()Ljava/lang/String;"), &[])?
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

        /// Hand back the private path of one file the system asked us to open (an `ACTION_VIEW` /
        /// `ACTION_SEND` the app is registered for), or `None` when nothing is pending. Each staged
        /// copy is consumed once. Returns an error if the Activity has no intent host (an older
        /// launcher); the caller treats that as "nothing pending" and stops asking.
        pub fn intent_poll(&self) -> Result<Option<String>, String> {
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
                            // JNI thread (which the SAF bridge also uses); clear it before returning.
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
        /// finds it there. The shell's watcher thread uses it to wake an idle app (egui runs a
        /// frame only when something asks it to, so a file that arrives while the app sits
        /// untouched would otherwise wait for the next touch).
        pub fn intent_pending(&self) -> Result<usize, String> {
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

        pub fn files_dir(&self) -> Result<PathBuf, String> {
            self.vm
                .attach_current_thread(|env| -> jni::errors::Result<PathBuf> {
                    let activity_raw = self.app.activity_as_ptr() as jni::sys::jobject;
                    // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it
                    // without taking ownership or attempting to delete the reference.
                    let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
                    let value = env
                        .call_method(&*activity, jni_str!("safFilesDir"), jni_sig!("()Ljava/lang/String;"), &[])?
                        .l()?;
                    let value = JString::cast_local(env, value)?;
                    let value: String = value.try_to_string(env)?;
                    Ok(PathBuf::from(value))
                })
                .map_err(|e| e.to_string())
        }

        /// Returns `NONE` for ordinary private paths, `OK` after committing a selected SAF
        /// destination, or `ERROR:<message>` if the Android provider refused the write.
        pub fn commit(&self, path: &str) -> Result<String, String> {
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
                            jni_str!("safCommit"),
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

        /// Drain every picker answer queued so far, as raw JSON strings (bounded, so a runaway
        /// picker cannot spin this loop forever).
        pub fn drain(&self) -> Result<Vec<String>, String> {
            let mut answers = Vec::new();
            for _ in 0..64 {
                match self.poll()? {
                    Some(raw) => answers.push(raw),
                    None => break,
                }
            }
            Ok(answers)
        }

        /// Blocking picker for apps whose file API is synchronous (the app expects the path at
        /// once). The system picker runs on the Android UI thread while `android_main` runs on its
        /// own thread, so waiting here does not freeze the UI; `timeout` stops a stuck picker from
        /// hanging the app forever. `save_name` selects the save picker, otherwise the open picker.
        pub fn pick_blocking(&self, id: u64, save_name: Option<&str>, timeout: Duration) -> Result<PickAnswer, String> {
            let _nonblocking = NonblockingCallbacks::enable();
            match save_name {
                Some(name) => self.request_save(id, name)?,
                None => self.request_open(id, false)?,
            }
            self.wait_answer(id, timeout)
        }

        /// Blocking open picker that lets the user select several files at once (apps that import a
        /// list of media ask for that). Like [`Self::pick_blocking`], but the picker allows
        /// multi-select.
        pub fn pick_open_blocking(&self, id: u64, multiple: bool, timeout: Duration) -> Result<PickAnswer, String> {
            let _nonblocking = NonblockingCallbacks::enable();
            self.request_open(id, multiple)?;
            self.wait_answer(id, timeout)
        }

        /// Blocking folder picker: the answer is a real directory path on the device's main
        /// storage (only such folders can be mapped back to a path), or a cancellation.
        pub fn pick_folder_blocking(&self, id: u64, timeout: Duration) -> Result<PickAnswer, String> {
            let _nonblocking = NonblockingCallbacks::enable();
            self.request_folder(id)?;
            self.wait_answer(id, timeout)
        }

        /// Blocking picker for a **worker thread**, while the frame thread keeps pumping the event
        /// loop. Unlike [`Self::pick_blocking`], nothing here parks the frame thread: the app's
        /// window/surface lifecycle (`Suspended`/`Resumed`) is delivered normally, so the renderer
        /// tears down and rebuilds its surface around the picker instead of keeping a stale one
        /// (which panics with `ERROR_NATIVE_WINDOW_IN_USE_KHR` on resume). Because the frame thread
        /// is alive, the glue's lifecycle callbacks complete on their own — `NonblockingCallbacks`
        /// must NOT be enabled here, and input is left to the frame thread's own iterator.
        pub fn pick_blocking_worker(&self, id: u64, save_name: Option<&str>, timeout: Duration) -> Result<PickAnswer, String> {
            match save_name {
                Some(name) => self.request_save(id, name)?,
                None => self.request_open(id, false)?,
            }
            self.wait_answer_worker(id, timeout)
        }

        fn wait_answer_worker(&self, id: u64, timeout: Duration) -> Result<PickAnswer, String> {
            let deadline = Instant::now() + timeout;
            loop {
                for raw in self.drain()? {
                    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else { continue };
                    if value.get("id").and_then(serde_json::Value::as_u64) != Some(id) {
                        continue;
                    }
                    return match value.get("status").and_then(serde_json::Value::as_str).unwrap_or("error") {
                        "ok" => Ok(PickAnswer::Paths(picker_paths(&value))),
                        "cancel" => Ok(PickAnswer::Cancelled),
                        _ => Err(value
                            .get("message")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("the Android file picker failed")
                            .to_owned()),
                    };
                }
                if Instant::now() >= deadline {
                    return Err("the Android file picker did not answer".to_owned());
                }
                std::thread::sleep(Duration::from_millis(15));
            }
        }

        fn wait_answer(&self, id: u64, timeout: Duration) -> Result<PickAnswer, String> {
            let deadline = Instant::now() + timeout;
            loop {
                // While the main thread parks here, any input events that still reach the
                // app's queue must be finished, or Android's input dispatcher waits on us
                // and ANRs the app (`finish_event` is called for each drained event).
                self.finish_pending_input();
                for raw in self.drain()? {
                    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else { continue };
                    if value.get("id").and_then(serde_json::Value::as_u64) != Some(id) {
                        continue;
                    }
                    return match value.get("status").and_then(serde_json::Value::as_str).unwrap_or("error") {
                        "ok" => Ok(PickAnswer::Paths(picker_paths(&value))),
                        "cancel" => Ok(PickAnswer::Cancelled),
                        _ => Err(value
                            .get("message")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("the Android file picker failed")
                            .to_owned()),
                    };
                }
                if Instant::now() >= deadline {
                    return Err("the Android file picker did not answer".to_owned());
                }
                std::thread::sleep(Duration::from_millis(15));
            }
        }

        /// Drains (and finishes) whatever input events are queued for the app while a blocking
        /// pick parks this thread. Bounded per call so the wait loop stays responsive; a bound
        /// miss is harmless because the next iteration tops the queue up again. Returns without
        /// touching anything if winit's own iterator is alive (only one iterator may exist).
        fn finish_pending_input(&self) {
            let Ok(mut events) = self.app.input_events_iter() else { return };
            for _ in 0..128 {
                if !events.next(|_| android_activity::InputStatus::Handled) {
                    break;
                }
            }
        }
    }

    /// The staged paths in a picker answer: `paths` (open, possibly several) or the single `path`
    /// (save).
    pub fn picker_paths(value: &serde_json::Value) -> Vec<String> {
        let mut paths: Vec<String> = value
            .get("paths")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.as_str().map(str::to_owned))
            .collect();
        if paths.is_empty() {
            if let Some(path) = value.get("path").and_then(serde_json::Value::as_str) {
                paths.push(path.to_owned());
            }
        }
        paths
    }

    /// The file name (without directory) of a staged path, for the app's display.
    pub fn picker_name(path: &str) -> String {
        std::path::Path::new(path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "imported-file".to_owned())
    }

    /// Mirrors a private file the app writes itself to a chosen SAF destination: tracks the file
    /// and pushes it to the destination URI whenever it changes on disk. Used by hosts whose file
    /// API is path-based (the app writes the file without a write hook to intercept). The file
    /// stays in app-private storage, so later saves to the same path sync too.
    ///
    /// A path-based app may not write the exact staged path it was handed: it can add or replace
    /// the extension after the pick (GridCraft saves "Book" as "Book.xlsx"). The whole family of
    /// files sharing the staged file's stem is watched, so that write is committed too — the Java
    /// side resolves such a sibling back to the chosen document. Files already there when the pick
    /// happened are remembered, so they are never pushed into a new document.
    #[derive(Default)]
    pub struct SaveMirror {
        entries: Mutex<HashMap<String, Tracked>>,
    }

    #[derive(Default)]
    struct Tracked {
        path: String,
        /// Last-known stamp of every staged path already seen for this pick.
        stamps: HashMap<String, (u64, u64)>,
        /// When the stem family was last listed. Listing every frame would read the whole staged
        /// directory 60 times a second; a sibling written by the app is still found within a frame
        /// or two.
        scanned: Option<Instant>,
    }

    /// How often the staged directory is listed for new files sharing the tracked stem.
    const SCAN_INTERVAL: Duration = Duration::from_millis(200);

    impl SaveMirror {
        /// Watch `path` (a staged save or open destination) for writes.
        pub fn track(&self, _id: u64, path: &str) {
            let mut entries = self.entries.lock().unwrap_or_else(|error| error.into_inner());
            if let Some(existing) = entries.get_mut(path) {
                for candidate in family(path) {
                    existing.stamps.entry(candidate.clone()).or_insert_with(|| file_stamp(&candidate));
                }
                return;
            }
            let mut tracked = Tracked { path: path.to_owned(), ..Tracked::default() };
            for candidate in family(path) {
                tracked.stamps.insert(candidate.clone(), file_stamp(&candidate));
            }
            tracked.scanned = Some(Instant::now());
            entries.insert(path.to_owned(), tracked);
        }

        /// Commit every tracked file whose contents changed since the last sync.
        pub fn sync(&self, bridge: &Bridge) {
            let mut entries = self.entries.lock().unwrap_or_else(|error| error.into_inner());
            for tracked in entries.values_mut() {
                if tracked.scanned.is_none_or(|at| at.elapsed() >= SCAN_INTERVAL) {
                    for candidate in family(&tracked.path) {
                        tracked.stamps.entry(candidate.clone()).or_insert_with(|| file_stamp(&candidate));
                    }
                    tracked.scanned = Some(Instant::now());
                }
                let candidates: Vec<String> = tracked.stamps.keys().cloned().collect();
                for candidate in candidates {
                    let stamp = file_stamp(&candidate);
                    if tracked.stamps.get(&candidate) == Some(&stamp) {
                        continue;
                    }
                    tracked.stamps.insert(candidate.clone(), stamp);
                    // An empty file is the staged copy the picker itself just created, not a write.
                    if stamp.1 == 0 {
                        continue;
                    }
                    match bridge.commit(&candidate) {
                        Ok(status) if status == "NONE" || status == "OK" => {}
                        Ok(status) => log::error!("Could not save {candidate}: {status}"),
                        Err(error) => log::error!("Could not save {candidate}: {error}"),
                    }
                }
            }
        }
    }

    /// The staged path plus any file beside it with the same stem ("Book.xlsx", "Book.csv", …).
    fn family(path: &str) -> Vec<String> {
        let mut paths = vec![path.to_owned()];
        let staged = std::path::Path::new(path);
        let (Some(parent), Some(stem)) = (staged.parent(), staged.file_stem()) else {
            return paths;
        };
        let Ok(entries) = std::fs::read_dir(parent) else { return paths };
        for entry in entries.flatten() {
            let candidate = entry.path();
            if candidate.file_stem().is_some_and(|other| other == stem)
                && let Some(candidate) = candidate.to_str()
                && candidate != path
            {
                paths.push(candidate.to_owned());
            }
        }
        paths
    }

    fn file_stamp(path: &str) -> (u64, u64) {
        std::fs::metadata(path)
            .map(|meta| {
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|since| since.as_nanos() as u64)
                    .unwrap_or(0);
                (mtime, meta.len())
            })
            .unwrap_or((0, 0))
    }
}

//! Generated Android launcher for WordCraft.
//! Starts the app's egui UI with Android Storage Access Framework (SAF) file services plus the
//! shared Android platform shell (immersive fullscreen and the Ctrl/Alt modifier-key fix).

mod system_fonts;
mod ime_bridge;
use wordcraft_engine::Session;
use wordcraft_ui_egui::{WordApp, Services};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use android_activity::AndroidApp;
use jni::{JavaVM, jni_sig, jni_str, objects::{JObject, JString, JValue}};

/// Emitted at startup so a device log (and the packaged `.so` strings) can confirm that this
/// shell — including the Android modifier-key fix and immersive fullscreen — is the one running.
const ARTCRAFT_ANDROID_SHELL_MARKER: &str = "ARTCRAFT_WORDCRAFT_ANDROID_SHELL_V32_SAF_FILEACCESS";

/// Hide the Android status and navigation bars so the egui canvas owns the whole screen.
///
/// The theme's `windowFullscreen` flag only hides the status bar; the navigation bar needs sticky
/// immersive mode. `android_main` runs on its own Rust thread, so the Java main (UI) thread is
/// free and these window calls can run on it without a Java activity subclass.
fn apply_immersive_fullscreen(app: &AndroidApp) {
    let app_clone = app.clone();
    app.run_on_java_main_thread(Box::new(move || {
        // SAFETY: AndroidApp owns the live VM pointer for the lifetime of the activity.
        let jvm = unsafe { JavaVM::from_raw(app_clone.vm_as_ptr() as *mut _) };
        let result = jvm.attach_current_thread(|env| -> jni::errors::Result<()> {
            let activity_raw = app_clone.activity_as_ptr() as jni::sys::jobject;
            // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it.
            let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
            let window = env
                .call_method(&*activity, jni_str!("getWindow"), jni_sig!("()Landroid/view/Window;"), &[])?
                .l()?;
            if window.is_null() {
                return Ok(());
            }
            let decor = env
                .call_method(&window, jni_str!("getDecorView"), jni_sig!("()Landroid/view/View;"), &[])?
                .l()?;
            if decor.is_null() {
                return Ok(());
            }
            // LAYOUT_STABLE | HIDE_NAVIGATION | FULLSCREEN | LAYOUT_HIDE_NAVIGATION
            // | LAYOUT_FULLSCREEN | IMMERSIVE_STICKY
            const IMMERSIVE: i32 =
                0x0000_0100 | 0x0000_0002 | 0x0000_0004 | 0x0000_0200 | 0x0000_0400 | 0x0000_1000;
            // SYSTEM_UI_FLAG_LIGHT_STATUS_BAR | SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR: this
            // replace-style set must not wipe the icon contrast that reflects the app's theme
            // (setBarIconAppearance) on Android 9/10, where the refresh can run repeatedly.
            const LIGHT_ICON_FLAGS: i32 = 0x2000 | 0x0010;
            let visibility = env
                .call_method(
                    &decor,
                    jni_str!("getSystemUiVisibility"),
                    jni_sig!("()I"),
                    &[],
                )?
                .i()?;
            env.call_method(
                &decor,
                jni_str!("setSystemUiVisibility"),
                jni_sig!("(I)V"),
                &[JValue::Int((visibility & LIGHT_ICON_FLAGS) | IMMERSIVE)],
            )?;
            Ok(())
        });
        if let Err(error) = result {
            log::error!("Could not apply immersive fullscreen: {error}");
        }
    }));
}

/// Keep the system-bar icon contrast on the app's active theme.
///
/// The bars are edge-to-edge (they float over the canvas when a swipe brings them in), so their
/// icon color must follow what egui is drawing: dark icons over a light theme, light icons over
/// a dark one. egui's choice arrives here as `light_theme` and the Activity applies
/// `APPEARANCE_LIGHT_*` on the Java main thread, just like the fullscreen calls — `android_main`
/// must not block waiting for it. Runs only when the theme actually changes.
fn apply_bar_appearance(app: &AndroidApp, light_theme: bool) {
    let app_clone = app.clone();
    app.run_on_java_main_thread(Box::new(move || {
        // SAFETY: AndroidApp owns the live VM pointer for the lifetime of the activity.
        let jvm = unsafe { JavaVM::from_raw(app_clone.vm_as_ptr() as *mut _) };
        let result = jvm.attach_current_thread(|env| -> jni::errors::Result<()> {
            let activity_raw = app_clone.activity_as_ptr() as jni::sys::jobject;
            // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it.
            let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
            env.call_method(
                &*activity,
                jni_str!("setBarIconAppearance"),
                jni_sig!("(Z)V"),
                &[JValue::Bool(light_theme)],
            )?;
            Ok(())
        });
        if let Err(error) = result {
            log::error!("Could not style the Android system bars: {error}");
        }
    }));
}

/// Reconstruct egui's modifier state on Android.
///
/// winit's Android backend delivers `KeyboardInput` events but never emits `ModifiersChanged`
/// (it discards the `KeyEvent`'s meta state), so egui's modifier state stays empty and
/// `Ctrl`/`Alt` shortcuts are ignored while plain keys continue to work. Physical modifier keys
/// do arrive as `Event::Key` events (`Key::ControlLeft`, `Key::AltLeft`, ...), so we track them
/// and stamp the resulting state onto this frame's events, suppressing the text character that
/// would otherwise be emitted alongside a Ctrl/Command shortcut.
fn apply_android_modifiers(state: &mut egui::Modifiers, raw: &mut egui::RawInput) {
    let before = *state;
    let mut kept: Vec<egui::Event> = Vec::with_capacity(raw.events.len());
    for mut event in raw.events.drain(..) {
        match &mut event {
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
        raw.events.push(egui::Event::ModifiersChanged(*state));
    }
}

/// Read the Android system clipboard's text (empty when there is none). Reaches
/// `ClipboardManager` through JNI using the live VM and Activity held by `AndroidApp`,
/// so it works from `android_main`'s own thread without a Java activity subclass.
fn android_clipboard_get(app: &AndroidApp) -> String {
    let app = app.clone();
    // SAFETY: AndroidApp owns the live VM pointer for the lifetime of the activity.
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr() as *mut _) };
    let activity_raw = app.activity_as_ptr() as jni::sys::jobject;
    let result = vm.attach_current_thread(|env| -> jni::errors::Result<String> {
        // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it
        // without taking ownership or attempting to delete the reference.
        let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
        let service = JObject::from(env.new_string("clipboard")?);
        let manager = env
            .call_method(
                &*activity,
                jni_str!("getSystemService"),
                jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"),
                &[JValue::Object(&service)],
            )?
            .l()?;
        if manager.is_null() {
            return Ok(String::new());
        }
        let clip = env
            .call_method(
                &manager,
                jni_str!("getPrimaryClip"),
                jni_sig!("()Landroid/content/ClipData;"),
                &[],
            )?
            .l()?;
        if clip.is_null() {
            return Ok(String::new());
        }
        let count = env
            .call_method(&clip, jni_str!("getItemCount"), jni_sig!("()I"), &[])?
            .i()?;
        if count <= 0 {
            return Ok(String::new());
        }
        let item = env
            .call_method(
                &clip,
                jni_str!("getItemAt"),
                jni_sig!("(I)Landroid/content/ClipData$Item;"),
                &[JValue::Int(0)],
            )?
            .l()?;
        if item.is_null() {
            return Ok(String::new());
        }
        let text = env
            .call_method(
                &item,
                jni_str!("getText"),
                jni_sig!("()Ljava/lang/CharSequence;"),
                &[],
            )?
            .l()?;
        if text.is_null() {
            return Ok(String::new());
        }
        let text = env
            .call_method(
                &text,
                jni_str!("toString"),
                jni_sig!("()Ljava/lang/String;"),
                &[],
            )?
            .l()?;
        if text.is_null() {
            return Ok(String::new());
        }
        let text = JString::cast_local(env, text)?;
        Ok(text.try_to_string(env)?)
    });
    result.unwrap_or_else(|error| {
        log::error!("Could not read the Android clipboard: {error}");
        String::new()
    })
}

/// Write text to the Android system clipboard (`ClipboardManager`). Best-effort:
/// a refusal is logged, never raised into the frame loop.
fn android_clipboard_set(app: &AndroidApp, text: &str) {
    let app = app.clone();
    let text = text.to_owned();
    // SAFETY: AndroidApp owns the live VM pointer for the lifetime of the activity.
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr() as *mut _) };
    let activity_raw = app.activity_as_ptr() as jni::sys::jobject;
    let result = vm.attach_current_thread(|env| -> jni::errors::Result<()> {
        // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it
        // without taking ownership or attempting to delete the reference.
        let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
        let service = JObject::from(env.new_string("clipboard")?);
        let manager = env
            .call_method(
                &*activity,
                jni_str!("getSystemService"),
                jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"),
                &[JValue::Object(&service)],
            )?
            .l()?;
        if manager.is_null() {
            return Ok(());
        }
        let text_obj = JObject::from(env.new_string(&text)?);
        let label = JObject::from(env.new_string("egui")?);
        let clip = env
            .call_static_method(
                jni_str!("android/content/ClipData"),
                jni_str!("newPlainText"),
                jni_sig!("(Ljava/lang/CharSequence;Ljava/lang/CharSequence;)Landroid/content/ClipData;"),
                &[JValue::Object(&label), JValue::Object(&text_obj)],
            )?
            .l()?;
        if clip.is_null() {
            return Ok(());
        }
        env.call_method(
            &manager,
            jni_str!("setPrimaryClip"),
            jni_sig!("(Landroid/content/ClipData;)V"),
            &[JValue::Object(&clip)],
        )?;
        Ok(())
    });
    if let Err(error) = result {
        log::error!("Could not write the Android clipboard: {error}");
    }
}
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
    #[derive(Default)]
    pub struct SaveMirror {
        entries: Mutex<HashMap<u64, (String, u64, u64)>>,
    }

    impl SaveMirror {
        /// Watch `path` (a staged save destination) for writes.
        pub fn track(&self, id: u64, path: &str) {
            let (mtime, size) = file_stamp(path);
            self.entries
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .insert(id, (path.to_owned(), mtime, size));
        }

        /// Commit every tracked file whose contents changed since the last sync.
        pub fn sync(&self, bridge: &Bridge) {
            let mut entries = self.entries.lock().unwrap_or_else(|error| error.into_inner());
            for (_id, (path, mtime, size)) in entries.iter_mut() {
                let (now_mtime, now_size) = file_stamp(path);
                if (now_mtime, now_size) == (*mtime, *size) {
                    continue;
                }
                *mtime = now_mtime;
                *size = now_size;
                match bridge.commit(path) {
                    Ok(status) if status == "NONE" || status == "OK" => {}
                    Ok(status) => log::error!("Could not save {path}: {status}"),
                    Err(error) => log::error!("Could not save {path}: {error}"),
                }
            }
        }
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


pub type SafInbox = wordcraft_ui_egui::Inbox;

#[derive(Default)]
pub struct SafState {
    next_id: AtomicU64,
    saves: Mutex<HashMap<u64, Vec<u8>>>,
}

pub fn saf_new_state() -> Arc<SafState> {
    Arc::new(SafState::default())
}

fn saf_next_id(state: &SafState) -> u64 {
    state.next_id.fetch_add(1, Ordering::Relaxed).wrapping_add(1)
}

/// Write to the staged private path, then push it through SAF when it was a chosen save
/// destination (a plain private path is a no-op commit).
fn saf_write_and_commit(bridge: &saf::Bridge, path: &str, bytes: &[u8]) -> Result<(), String> {
    let target = std::path::Path::new(path);
    if let Some(parent) = target.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
    }
    std::fs::write(target, bytes).map_err(|error| format!("Could not write {}: {error}", target.display()))?;
    match bridge.commit(path)? {
        status if status == "NONE" || status == "OK" => Ok(()),
        status if status.starts_with("ERROR:") => Err(status.trim_start_matches("ERROR:").trim().to_owned()),
        status => Err(format!("Unexpected Android save result: {status}")),
    }
}

fn saf_services(bridge: saf::Bridge, state: Arc<SafState>, inbox: SafInbox) -> Services {
    let open_bridge = bridge.clone_ref();
    let print_bridge = bridge.clone_ref();
    let save_bridge = bridge.clone_ref();
    let open_state = Arc::clone(&state);
    let save_state = Arc::clone(&state);
    Services {
        open_async: Some(Box::new(move |_purpose: &str| {
            let id = saf_next_id(&open_state);
            if let Err(error) = open_bridge.request_open(id, false) {
                log::error!("Could not start the Android open picker: {error}");
            }
        })),
        download: Some(Box::new(move |name: &str, bytes: &[u8]| {
            let id = saf_next_id(&save_state);
            save_state.saves.lock().unwrap_or_else(|error| error.into_inner()).insert(id, bytes.to_vec());
            if let Err(error) = save_bridge.request_save(id, name) {
                log::error!("Could not start the Android save picker: {error}");
                save_state.saves.lock().unwrap_or_else(|error| error.into_inner()).remove(&id);
                return Err(error);
            }
            Ok(())
        })),
        // Android keeps printer choice inside the system print dialog: it reaches every printer
        // the device knows (Wi-Fi, network, cloud print services), not just ones an in-app list
        // could discover itself. Offer exactly that one entry — named and defaulted so File >
        // Print's auto-selection settles on it and enables submission — and the dialog the
        // submit opens does the real printer/paper/duplex/color/page-range picking.
        printers: Some(Box::new(|| {
            let (tx, rx) = std::sync::mpsc::channel();
            let _ = tx.send(vec![wordcraft_ui_egui::printing::Printer {
                name: "Android print system".to_owned(),
                default: true,
            }]);
            rx
        })),
        // The document arrives as a PDF WordCraft already collated (range and settings baked
        // in), so handing it straight to the system spooler is the whole job; Java answers
        // inside the same call, letting us report success or the reason for failure right
        // where the desktop spooler would have.
        print_native: Some(Box::new(move |bytes: Vec<u8>, _settings: wordcraft_ui_egui::printing::Settings, _collate: bool, _options: Vec<(String, String)>| {
            let (tx, rx) = std::sync::mpsc::channel();
            let receipt = print_bridge
                .print_document("WordCraft", &bytes)
                .and_then(|answer| {
                    if answer == "OK" {
                        // Empty receipt renders as the clean "Print job submitted." message.
                        Ok(String::new())
                    } else {
                        Err(answer)
                    }
                });
            let _ = tx.send(receipt);
            rx
        })),
        inbox: Some(inbox),
        ..Services::default()
    }
}

fn saf_poll(bridge: &saf::Bridge, state: &SafState, inbox: &SafInbox) {
    for _ in 0..16 {
        let raw = match bridge.poll() {
            Ok(Some(raw)) => raw,
            Ok(None) => break,
            Err(error) => {
                log::error!("Could not poll the Android file picker: {error}");
                break;
            }
        };
        let value: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(value) => value,
            Err(error) => {
                log::error!("Ignoring a malformed Android picker result ({error}): {raw}");
                continue;
            }
        };
        let Some(id) = value.get("id").and_then(serde_json::Value::as_u64) else { continue };
        let kind = value.get("kind").and_then(serde_json::Value::as_str).unwrap_or_default();
        let status = value.get("status").and_then(serde_json::Value::as_str).unwrap_or("error");
        match (kind, status) {
            ("open", "ok") => {
                let paths: Vec<String> = value
                    .get("paths")
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|entry| entry.as_str().map(str::to_owned))
                    .collect();
                for path in paths {
                    match std::fs::read(&path) {
                        Ok(bytes) => {
                            let name = std::path::Path::new(&path)
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_else(|| "imported-file".to_owned());
                            inbox.lock().unwrap_or_else(|error| error.into_inner()).push((name, bytes));
                        }
                        Err(error) => log::error!("Could not read the picked file {path}: {error}"),
                    }
                }
            }
            ("open", "cancel") => {}
            ("save", "ok") => {
                let bytes = state.saves.lock().unwrap_or_else(|error| error.into_inner()).remove(&id);
                if let Some(bytes) = bytes {
                    match value.get("path").and_then(serde_json::Value::as_str) {
                        Some(path) => {
                            if let Err(error) = saf_write_and_commit(bridge, path, &bytes) {
                                log::error!("Could not write the saved file: {error}");
                            }
                        }
                        None => log::error!("Android save picker returned no destination path"),
                    }
                }
            }
            ("save", "cancel") => {
                state.saves.lock().unwrap_or_else(|error| error.into_inner()).remove(&id);
            }
            _ => {
                if let Some(message) = value.get("message").and_then(serde_json::Value::as_str) {
                    log::error!("Android file picker error: {message}");
                }
                state.saves.lock().unwrap_or_else(|error| error.into_inner()).remove(&id);
            }
        }
    }
}

/// On-device input diagnostics (reported through `adb logcat`).
///
/// Counts the egui events the platform backend produced since the last report: pointer
/// motion, mouse buttons, wheel scrolls and touches. The stock touch-only winit backend
/// turned a connected mouse into a finger: every mouse move/click arrived wrapped in
/// `Event::Touch`, so `touch` grew while hovering. With the builder-patched backend the
/// same mouse activity grows `pointer`/`buttons`/`wheel` and leaves `touch` at zero
/// (only real fingers grow it). Reports at most once per second, and only when a counter
/// actually changed.
#[derive(Default)]
struct InputProbe {
    pointer_moved: u64,
    buttons: u64,
    wheels: u64,
    touches: u64,
    reported: (u64, u64, u64, u64),
    last_report: Option<std::time::Instant>,
}

impl InputProbe {
    fn observe(&mut self, raw: &egui::RawInput) {
        for event in &raw.events {
            match event {
                egui::Event::PointerMoved(_) => self.pointer_moved += 1,
                egui::Event::PointerButton { .. } => self.buttons += 1,
                egui::Event::MouseWheel { .. } => self.wheels += 1,
                egui::Event::Touch { .. } => self.touches += 1,
                _ => {}
            }
        }
        let totals = (self.pointer_moved, self.buttons, self.wheels, self.touches);
        if totals == self.reported {
            return;
        }
        let now = std::time::Instant::now();
        if self
            .last_report
            .is_some_and(|last| now.duration_since(last) < std::time::Duration::from_secs(1))
        {
            return;
        }
        log::info!(
            "input-probe: pointer={} buttons={} wheel={} touch={} (event totals)",
            totals.0, totals.1, totals.2, totals.3
        );
        self.reported = totals;
        self.last_report = Some(now);
    }
}


/// The app's private storage directory. Android exposes two: the internal one
/// (`/data/user/0/<pkg>/files`), which a file manager or a computer never sees, and the app-specific
/// external one (`/storage/emulated/0/Android/data/<pkg>/files`), which the user can reach over USB
/// or MTP without root. The external directory is preferred so the settings, autosaves and exported
/// documents are actually findable; the internal directory is the fallback for the rare device where
/// external storage is unmounted or unavailable. Both survive restarts and app updates.
fn app_data_dir(app: &AndroidApp) -> Option<std::path::PathBuf> {
    app.external_data_path().or_else(|| app.internal_data_path())
}

/// The app's private files directory plus a `config` subfolder, where the settings, preferences and
/// recent-files list are kept. The folder survives restarts and app updates, which is what the
/// desktop versions get from their per-user config folder.
fn persist_dir(app: &AndroidApp) -> Option<std::path::PathBuf> {
    app_data_dir(app).map(|base| base.join("config"))
}

/// Point the platform path helpers at app-private storage before the app reads them, so anything
/// the app writes itself (autosave, recovery copies, caches) also lands there.
///
/// Only the settings and caches are set here, and only to the external app directory, so they stay
/// reachable over USB. `HOME` and `XDG_DATA_HOME` are left to
/// [`super::system_fonts::install_font_env`], which keeps them on internal storage: they are the
/// roots the apps' font scanners read, and the symlinks to `/system/fonts` they need can only be
/// created there (`/storage/emulated/0` refuses `symlink`).
#[allow(unused_unsafe)]
fn install_persist_env(app: &AndroidApp) {
    let Some(base) = app_data_dir(app) else { return };
    let config = base.join("config");
    let cache = base.join("cache");
    let _ = std::fs::create_dir_all(config.join(".config"));
    let _ = std::fs::create_dir_all(&cache);
    // SAFETY: `android_main` runs before the app starts any worker thread, so no other thread is
    // reading the environment here.
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", &config);
        std::env::set_var("XDG_CACHE_HOME", &cache);
    }
    log::info!("settings directory: {}", config.display());
}

#[allow(dead_code)]
fn persist_read(dir: &std::path::Path, name: &str) -> Option<Vec<u8>> {
    std::fs::read(dir.join(name)).ok()
}

/// Write `bytes` to `dir/name` through a temporary file renamed over the old one, so a crash leaves
/// the old file or the new one whole, never a truncated one.
#[allow(dead_code)]
fn persist_write(dir: &std::path::Path, name: &str, bytes: &[u8]) {
    if let Err(error) = std::fs::create_dir_all(dir) {
        log::warn!("settings: {}: {error}", dir.display());
        return;
    }
    let path = dir.join(name);
    let temporary = path.with_extension("tmp");
    let written = std::fs::write(&temporary, bytes).and_then(|()| std::fs::rename(&temporary, &path));
    if let Err(error) = written {
        log::warn!("settings: {}: {error}", path.display());
    }
}

fn persist_load(inner: &mut WordApp, dir: &std::path::Path) {
if let Some(bytes) = persist_read(dir, "ui.json") {
        if let Some(ui) = wordcraft_ui_egui::UiState::from_json(&bytes) {
            inner.apply_prefs(ui);
        }
    }
}

fn persist_save(inner: &mut WordApp, dir: &std::path::Path) {
match serde_json::to_vec_pretty(&inner.prefs()) {
        Ok(bytes) => persist_write(dir, "ui.json", &bytes),
        Err(error) => log::warn!("saving settings: {error}"),
    }
}

struct AndroidShell {
    inner: WordApp,
    app: AndroidApp,
    ime: ime_bridge::ime::ImeBridge,
    modifiers: egui::Modifiers,
    fullscreen_applied: bool,
    probe: InputProbe,
    bars_theme: Option<bool>,
    saf_bridge: saf::Bridge,
    saf_state: std::sync::Arc<SafState>,
    saf_inbox: SafInbox,
    intent_warmup: i64,
    persist: Option<std::path::PathBuf>,
    persist_last: Option<std::time::Instant>,
    margin_scroll: Option<egui::Pos2>,
}

impl AndroidShell {
    fn new(mut inner: WordApp, app: AndroidApp, saf_bridge: saf::Bridge, saf_state: std::sync::Arc<SafState>, saf_inbox: SafInbox) -> Self {
        let persist = persist_dir(&app);
        if let Some(dir) = &persist {
            persist_load(&mut inner, dir);
        }
        let ime = ime_bridge::ime::ImeBridge::new(&app);
        Self { inner, app, ime, modifiers: egui::Modifiers::default(), fullscreen_applied: false, probe: InputProbe::default(), bars_theme: None, saf_bridge, saf_state, saf_inbox, intent_warmup: 2, persist, persist_last: None, margin_scroll: None }
    }

    /// Drain any finished Android file-picker requests and hand the results to the app.
    fn poll_file_picker(&mut self) {
        saf_poll(&self.saf_bridge, &self.saf_state, &self.saf_inbox);
    }

    fn open_incoming(&mut self, path: &str) {
        log::info!("Opening the file the system handed the app: {path}");
        match std::fs::read(path) {
            Ok(bytes) => self.saf_inbox.lock().unwrap().push((saf::picker_name(path), bytes)),
            Err(error) => log::error!("Could not read the incoming file {path}: {error}"),
        }
    }

    /// Open any file the system handed the app (an intent it is registered for). Runs once per
    /// staged file and is a no-op afterwards; a launcher without the intent host stops the poll.
    fn poll_incoming_files(&mut self) {
        if self.intent_warmup > 0 {
            self.intent_warmup -= 1;
            return;
        }
        if self.intent_warmup < 0 {
            return;
        }
        for _ in 0..16 {
            match self.saf_bridge.intent_poll() {
                Ok(Some(path)) => self.open_incoming(&path),
                Ok(None) => break,
                Err(error) => {
                    log::info!("Launcher does not report incoming files: {error}");
                    self.intent_warmup = -1;
                    break;
                }
            }
        }
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

    /// Keep the system-bar icon contrast on the app's active theme: egui's light/dark choice is
    /// compared every frame and only a flip reaches Java (`apply_bar_appearance`), where the
    /// edge-to-edge bars get dark icons over a light canvas and light icons over a dark one.
    fn sync_bar_appearance(&mut self, ctx: &egui::Context) {
        // `Context::style()` is gone in egui 0.36; `theme()` is what selects the active
        // light/dark style, so it is exactly the choice the bars must mirror.
        let light_theme = ctx.theme() == egui::Theme::Light;
        if self.bars_theme == Some(light_theme) {
            return;
        }
        self.bars_theme = Some(light_theme);
        apply_bar_appearance(&self.app, light_theme);
    }

    /// Re-apply sticky immersive mode on the first frame and whenever the window regains focus
    /// (the system restores the bars after a gesture or when another activity closes).
    fn refresh_fullscreen(&mut self, raw: &egui::RawInput) {
        let refocused = raw
            .events
            .iter()
            .any(|event| matches!(event, egui::Event::WindowFocused(true)));
        if !self.fullscreen_applied || refocused {
            self.fullscreen_applied = true;
            apply_immersive_fullscreen(&self.app);
        }
    }

    /// Emit egui `Copy`/`Cut`/`Paste` events for Ctrl/Cmd+C/X/V key presses.
    ///
    /// winit's Android backend never emits `ModifiersChanged`, so egui-winit's own
    /// modifier-based clipboard-shortcut detection never fires there. The shell detects the
    /// shortcuts after [`apply_android_modifiers`] has reconstructed the modifier state and
    /// synthesizes the events egui's widgets (text edits, label selection) expect.

    /// Scroll the page when dragging in the empty margin outside the page instead of
    /// selecting text.
    fn handle_margin_touch_scroll(&mut self, raw: &mut egui::RawInput) {
        let canvas_rect = self.inner.canvas.canvas_rect;
        let page_rects = self.inner.canvas.page_rects.clone();

        let in_margin = |pos: egui::Pos2| -> bool {
            if let Some(c) = canvas_rect {
                pos.y >= c.min.y && pos.y <= c.max.y && !page_rects.iter().any(|r| r.contains(pos))
            } else {
                false
            }
        };

        let mut synthesized: Vec<egui::Event> = Vec::new();
        let mut new_events: Vec<egui::Event> = Vec::with_capacity(raw.events.len());

        for event in raw.events.drain(..) {
            match event {
                egui::Event::Touch { id, phase, pos, force, device_id } => {
                    match phase {
                        egui::TouchPhase::Start => {
                            if in_margin(pos) {
                                self.margin_scroll = Some(pos);
                            }
                            new_events.push(egui::Event::Touch { id, phase, pos, force, device_id });
                        }
                        egui::TouchPhase::Move => {
                            new_events.push(egui::Event::Touch { id, phase, pos, force, device_id });
                        }
                        egui::TouchPhase::End | egui::TouchPhase::Cancel => {
                            if self.margin_scroll.take().is_some() {
                                synthesized.push(egui::Event::MouseWheel {
                                    unit: egui::MouseWheelUnit::Point,
                                    delta: egui::Vec2::ZERO,
                                    phase: egui::TouchPhase::End,
                                    modifiers: egui::Modifiers::NONE,
                                });
                            }
                            new_events.push(egui::Event::Touch { id, phase, pos, force, device_id });
                        }
                    }
                }
                egui::Event::PointerButton { pos, button, pressed, modifiers } => {
                    if button == egui::PointerButton::Primary {
                        if pressed && in_margin(pos) {
                            self.margin_scroll = Some(pos);
                            // Drop the button press so WordCraft does not start text selection.
                            continue;
                        }
                        if self.margin_scroll.is_some() {
                            if !pressed {
                                self.margin_scroll = None;
                                synthesized.push(egui::Event::MouseWheel {
                                    unit: egui::MouseWheelUnit::Point,
                                    delta: egui::Vec2::ZERO,
                                    phase: egui::TouchPhase::End,
                                    modifiers: egui::Modifiers::NONE,
                                });
                            }
                            // Drop button release while margin scrolling.
                            continue;
                        }
                    }
                    new_events.push(egui::Event::PointerButton { pos, button, pressed, modifiers });
                }
                egui::Event::PointerMoved(pos) => {
                    if let Some(last_pos) = self.margin_scroll {
                        let delta = pos - last_pos;
                        self.margin_scroll = Some(pos);
                        if delta.x != 0.0 || delta.y != 0.0 {
                            synthesized.push(egui::Event::MouseWheel {
                                unit: egui::MouseWheelUnit::Point,
                                delta,
                                phase: egui::TouchPhase::Move,
                                modifiers: egui::Modifiers::NONE,
                            });
                        }
                        // Ensure pointer hover position is inside canvas_rect so WordCraft's
                        // wheel hover check passes and allows the scroll.
                        let pointer_pos = if let Some(c) = canvas_rect {
                            egui::pos2(
                                pos.x.clamp(c.min.x + 10.0, c.max.x - 10.0),
                                pos.y.clamp(c.min.y + 10.0, c.max.y - 10.0),
                            )
                        } else {
                            pos
                        };
                        new_events.push(egui::Event::PointerMoved(pointer_pos));
                        continue;
                    }
                    new_events.push(egui::Event::PointerMoved(pos));
                }
                other => {
                    new_events.push(other);
                }
            }
        }
        new_events.extend(synthesized);
        raw.events = new_events;
    }

    fn handle_clipboard_shortcuts(&mut self, raw: &mut egui::RawInput) {
        let mut synthesized: Vec<egui::Event> = Vec::new();
        for event in &raw.events {
            let egui::Event::Key { key, pressed: true, modifiers, .. } = event else { continue };
            if !modifiers.command && !modifiers.ctrl {
                continue;
            }
            match key {
                egui::Key::C => synthesized.push(egui::Event::Copy),
                egui::Key::X => synthesized.push(egui::Event::Cut),
                egui::Key::V => {
                    let text = android_clipboard_get(&self.app);
                    if !text.is_empty() {
                        synthesized.push(egui::Event::Paste(text));
                    }
                }
                _ => {}
            }
        }
        raw.events.extend(synthesized);
    }
}

impl eframe::App for AndroidShell {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.poll_incoming_files();
        self.poll_file_picker();
        self.persist_tick();
        let _ = frame;
        self.inner.logic(ctx);
        self.sync_bar_appearance(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        system_fonts::install_system_fonts(ui.ctx());
        let _ = frame;
        self.inner.ui(ui);
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.probe.observe(raw);
        self.ime.poll(raw);
        self.refresh_fullscreen(raw);
        apply_android_modifiers(&mut self.modifiers, raw);
        self.handle_clipboard_shortcuts(raw);
        self.handle_margin_touch_scroll(raw);
        if raw.events.iter().any(|event| matches!(event, egui::Event::WindowFocused(false))) {
            self.persist_now();
        }
        let _ = ctx;
        self.inner.raw_input_hook(raw);
    }

    fn on_exit(&mut self) {
        self.persist_now();
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(android_app: android_activity::AndroidApp) {
    log::info!("{ARTCRAFT_ANDROID_SHELL_MARKER} ready ({}, {})", saf::SAF_MARKER, ime_bridge::ime::IME_MARKER);
    apply_immersive_fullscreen(&android_app);
    egui_winit::android_activity::set_activity(android_app.activity_as_ptr());
    install_persist_env(&android_app);
    system_fonts::install_font_env(&android_app);
    let saf_bridge = saf::Bridge::new(&android_app);
    saf_bridge.request_storage_permission();
    let saf_state = saf_new_state();
    let saf_inbox = SafInbox::default();
    let platform_services = saf_services(saf_bridge.clone_ref(), std::sync::Arc::clone(&saf_state), saf_inbox.clone());
    let inner = WordApp::new(Session::new(wordcraft_engine::sample::sample_document()), platform_services);
    let shell = AndroidShell::new(inner, android_app.clone(), saf_bridge, saf_state, saf_inbox);
    let mut options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_title("WordCraft")
            .with_drag_and_drop(true),
        android_app: Some(android_app),
        ..Default::default()
    };
    // Vulkan first with a GLES fallback for devices without a usable Vulkan driver, and the
    // adapter's own texture/buffer sizes: egui's 8192 px default cap sends documents wider or
    // taller than that to the CPU compositor (the desktop apps do this in their gpu_canvas).
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup {
        create.instance_descriptor.backends = eframe::wgpu::Backends::VULKAN | eframe::wgpu::Backends::GL;
        create.device_descriptor = std::sync::Arc::new(|adapter: &eframe::wgpu::Adapter| {
            let supported = adapter.limits();
            let base = if adapter.get_info().backend == eframe::wgpu::Backend::Gl {
                eframe::wgpu::Limits::downlevel_webgl2_defaults()
            } else {
                eframe::wgpu::Limits::default()
            };
            let base = base.or_worse_values_from(&supported);
            eframe::wgpu::DeviceDescriptor {
                label: Some("WordCraft wgpu device"),
                required_limits: eframe::wgpu::Limits {
                    max_texture_dimension_2d: supported.max_texture_dimension_2d,
                    max_buffer_size: supported.max_buffer_size,
                    ..base
                },
                ..Default::default()
            }
        });
    }
    let result = eframe::run_native(
        "WordCraft",
        options,
        Box::new(move |_cc| Ok(Box::new(shell))),
    );
    if let Err(error) = result {
        log::error!("WordCraft Android exited with an error: {error}");
    }
}

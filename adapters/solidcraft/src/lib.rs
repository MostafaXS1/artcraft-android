//! Generated Android launcher for SolidCraft.
//! Starts the app's egui UI with Android Storage Access Framework (SAF) file services plus the
//! shared Android platform shell (immersive fullscreen and the Ctrl/Alt modifier-key fix).

mod system_fonts;
mod mobile_text_bar;
mod mcp_http;
use solidcraft_engine::Session;
use solidcraft_ui_egui::{SolveApp, Services};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use android_activity::AndroidApp;
use jni::{JavaVM, jni_sig, jni_str, objects::{JObject, JString, JValue}};

/// Emitted at startup so a device log (and the packaged `.so` strings) can confirm that this
/// shell — including the Android modifier-key fix and immersive fullscreen — is the one running.
const ARTCRAFT_ANDROID_SHELL_MARKER: &str = "ARTCRAFT_SOLIDCRAFT_ANDROID_SHELL_V72_SAF_FILEACCESS";

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

/// Finish the Activity, ending the app. The Activity's own `onDestroy` ends the process once it
/// is finishing (see templates/android/SafActivity.java). Runs on the Java main thread, like the
/// fullscreen and bar-appearance calls, because `Activity.finish()` must be called there.
fn finish_activity(app: &AndroidApp) {
    let app_clone = app.clone();
    app.run_on_java_main_thread(Box::new(move || {
        // SAFETY: AndroidApp owns the live VM pointer for the lifetime of the activity.
        let jvm = unsafe { JavaVM::from_raw(app_clone.vm_as_ptr() as *mut _) };
        let result = jvm.attach_current_thread(|env| -> jni::errors::Result<()> {
            let activity_raw = app_clone.activity_as_ptr() as jni::sys::jobject;
            // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it.
            let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
            env.call_method(&*activity, jni_str!("finish"), jni_sig!("()V"), &[])?;
            Ok(())
        });
        if let Err(error) = result {
            log::error!("Could not finish the activity: {error}");
        }
    }));
}

/// Ask the Activity whether the screen is dense enough (>600 dpi) to give the app its own UI scale
/// instead of the system's. Returns `None` when the answer cannot be read, in which case the caller
/// leaves the UI scale alone.
///
/// The Activity reads the screen's `densityDpi`; the test and its rationale live in
/// `SafActivity.isHighDensityDevice`. Called once, on the Java main thread, using the same
/// activity-via-JNI pattern as the fullscreen and bar-appearance helpers above.
fn device_is_high_density(app: &AndroidApp) -> Option<bool> {
    // SAFETY: AndroidApp owns the live VM pointer for the lifetime of the activity.
    let jvm = unsafe { JavaVM::from_raw(app.vm_as_ptr() as *mut _) };
    let activity_raw = app.activity_as_ptr() as jni::sys::jobject;
    let result = jvm.attach_current_thread(|env| -> jni::errors::Result<bool> {
        // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it.
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

        /// Ask the Activity for microphone / audio recording permission (SoundCraft).
        pub fn request_microphone_permission(&self) {
            let result = self.vm.attach_current_thread(|env| -> jni::errors::Result<()> {
                let activity_raw = self.app.activity_as_ptr() as jni::sys::jobject;
                let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
                env.call_method(
                    &*activity,
                    jni_str!("safRequestMicrophonePermission"),
                    jni_sig!("()V"),
                    &[],
                )?;
                Ok(())
            });
            if let Err(error) = result {
                log::error!("Could not request Android microphone access: {error}");
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


pub type SafInbox = ();

#[derive(Default)]
pub struct SafState {
    next_id: AtomicU64,
    mirror: saf::SaveMirror,
}

pub fn saf_new_state() -> Arc<SafState> {
    Arc::new(SafState::default())
}

fn saf_next_id(state: &SafState) -> u64 {
    state.next_id.fetch_add(1, Ordering::Relaxed).wrapping_add(1)
}

/// Open the Android picker and block until the user answers. The picker runs on the Java UI thread
/// while `android_main` runs on its own thread, so waiting here does not freeze the UI. Empty on
/// cancel or error; `pick_blocking` has its own timeout so a stuck picker cannot hang the app.
fn saf_open_paths(bridge: &saf::Bridge, state: &SafState) -> Vec<String> {
    let id = saf_next_id(state);
    match bridge.pick_blocking(id, None, saf::PICK_TIMEOUT) {
        Ok(saf::PickAnswer::Paths(paths)) => {
            for path in &paths {
                state.mirror.track(id, path);
            }
            paths
        }
        Ok(saf::PickAnswer::Cancelled) => Vec::new(),
        Err(error) => {
            log::error!("Android open picker: {error}");
            Vec::new()
        }
    }
}

/// Save dialog: the staged app-private destination, watched so the app's own write is mirrored to
/// the chosen SAF document on the next frames.
fn saf_save_path(bridge: &saf::Bridge, state: &SafState, name: &str) -> Option<String> {
    let id = saf_next_id(state);
    match bridge.pick_blocking(id, Some(name), saf::PICK_TIMEOUT) {
        Ok(saf::PickAnswer::Paths(paths)) => {
            let path = paths.into_iter().next();
            if let Some(path) = &path {
                state.mirror.track(id, path);
            }
            path
        }
        Ok(saf::PickAnswer::Cancelled) => None,
        Err(error) => {
            log::error!("Android save picker: {error}");
            None
        }
    }
}

/// Folder dialog: a real directory path on the device's main storage (SAF folders elsewhere cannot
/// be mapped to one), or `None` when cancelled.
fn saf_folder_path(bridge: &saf::Bridge, state: &SafState) -> Option<String> {
    let id = saf_next_id(state);
    match bridge.pick_folder_blocking(id, saf::PICK_TIMEOUT) {
        Ok(saf::PickAnswer::Paths(paths)) => paths.into_iter().next(),
        Ok(saf::PickAnswer::Cancelled) => None,
        Err(error) => {
            log::error!("Android folder picker: {error}");
            None
        }
    }
}

fn saf_services(bridge: saf::Bridge, state: Arc<SafState>, _inbox: SafInbox) -> Services {
    let open_bridge = bridge.clone_ref();
    let open_state = Arc::clone(&state);
    let save_bridge = bridge.clone_ref();
    let save_state = Arc::clone(&state);
    let file_bridge = bridge.clone_ref();
    let file_state = Arc::clone(&state);
    Services {
        pick_open: Some(Box::new(move |_exts: &[&str]| {
            let bridge = open_bridge.clone_ref();
            let state = Arc::clone(&open_state);
            Box::pin(async move {
                saf_open_paths(&bridge, &state).into_iter().next()
            })
        })),
        pick_save: Some(Box::new(move |name: &str, _exts: &[&str]| {
            let bridge = save_bridge.clone_ref();
            let state = Arc::clone(&save_state);
            let name = name.to_owned();
            Box::pin(async move {
                saf_save_path(&bridge, &state, &name)
            })
        })),
        pick_file: Some(Box::new(move |_label: &str, _exts: &[&str]| {
            let bridge = file_bridge.clone_ref();
            let state = Arc::clone(&file_state);
            Box::pin(async move {
                saf_open_paths(&bridge, &state).into_iter().next()
            })
        })),
        ..Services::default()
    }
}

fn saf_poll(bridge: &saf::Bridge, state: &SafState, _inbox: &SafInbox) {
    state.mirror.sync(bridge);
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
        let internal = app.internal_data_path().map(|p| p.join("models")).unwrap_or_else(|| base.join("models"));
        let external = base.join("models");
        if !internal.join("text-detection.onnx").is_file() || !internal.join("text-recognition.onnx").is_file() {
            let _ = std::fs::create_dir_all(&internal);
            for name in ["text-detection.onnx", "text-recognition.onnx"] {
                if let Ok(cname) = std::ffi::CString::new(format!("models/{name}")) {
                    if let Some(mut asset) = app.asset_manager().open(&cname) {
                        let dest = internal.join(name);
                        if let Ok(mut f) = std::fs::File::create(&dest) {
                            let _ = std::io::copy(&mut asset, &mut f);
                        }
                    }
                }
            }
        }
        if internal.join("text-detection.onnx").is_file() {
            std::env::set_var("PDFCRAFT_MODELS", &internal);
        } else {
            std::env::set_var("PDFCRAFT_MODELS", &external);
        }
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

/// Whether the high-density UI scale should be applied to this launch, and the sidecar that
/// remembers that it was.
///
/// The scale belongs to the *device*, but the settings file it lands in belongs to the *user*: it is
/// written on the first launch and never revisited, so
///   * a screen that later stops being high-density (a foldable, an external display, a display-size
///     change) kept the old scale, and
///   * an install that already had a settings file — an upgrade from a version before this
///     bootstrap existed — never got it at all, so an updated app and a freshly installed one
///     behaved differently.
///
/// The choice is remembered in its own sidecar (`device_class`), separate from the app's settings,
/// so "first launch" means "the scale has not been set for this device yet", not "first launch of
/// this file". A user who changes the scale by hand keeps it: the sidecar already says it was
/// applied, so nothing is re-applied.
#[allow(dead_code)]
fn device_scale_first_time(dir: &std::path::Path) -> bool {
    let marker = dir.join("device_class");
    if std::fs::read_to_string(&marker).ok().is_some_and(|saved| saved.trim() == "scaled") {
        return false;
    }
    persist_write(dir, "device_class", b"scaled");
    true
}

fn persist_load(inner: &mut SolveApp, dir: &std::path::Path) {
if let Some(bytes) = persist_read(dir, "prefs.json") {
        if let Ok(prefs) = std::str::from_utf8(&bytes) {
            inner.load_prefs(prefs);
        }
    }
}

fn persist_save(inner: &mut SolveApp, dir: &std::path::Path) {
persist_write(dir, "prefs.json", inner.prefs().as_bytes());
}

/// Cleared when the shell shuts down, so a background watcher stops talking to the Activity.
///
/// Android releases the Activity's Java object as soon as the native thread returns, so a
/// watcher that kept asking it for files after that would call into a dead object. The flag is
/// cleared when the shell is dropped, which happens before the native thread returns.
static ALIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub static CONTROL_TX: std::sync::OnceLock<std::sync::mpsc::Sender<solidcraft_ui_egui::ControlRequest>> = std::sync::OnceLock::new();
pub static EGUI_CTX: std::sync::Mutex<Option<egui::Context>> = std::sync::Mutex::new(None);

struct AndroidShell {
    inner: SolveApp,
    app: AndroidApp,
    modifiers: egui::Modifiers,
    fullscreen_applied: bool,
    probe: InputProbe,
    bars_theme: Option<bool>,
    back_pending: bool,
    closing: bool,
    close_asked: bool,
    saf_bridge: saf::Bridge,
    saf_state: std::sync::Arc<SafState>,
    saf_inbox: SafInbox,
    intent_warmup: i64,
    persist: Option<std::path::PathBuf>,
    persist_last: Option<std::time::Instant>,
}

impl AndroidShell {
    fn new(mut inner: SolveApp, app: AndroidApp, saf_bridge: saf::Bridge, saf_state: std::sync::Arc<SafState>, saf_inbox: SafInbox) -> Self {
        let persist = persist_dir(&app);
        if let Some(dir) = &persist {
            persist_load(&mut inner, dir);
        }
        Self { inner, app, modifiers: egui::Modifiers::default(), fullscreen_applied: false, probe: InputProbe::default(), bars_theme: None, back_pending: false, closing: false, close_asked: false, saf_bridge, saf_state, saf_inbox, intent_warmup: 2, persist, persist_last: None }
    }

    /// Drain any finished Android file-picker requests and hand the results to the app.
    fn poll_file_picker(&mut self) {
        saf_poll(&self.saf_bridge, &self.saf_state, &self.saf_inbox);
    }

    fn open_incoming(&mut self, path: &str) {
        log::info!("Opening the file the system handed the app: {path}");
        self.saf_state.mirror.track(0, path);
        self.inner.open_path_with(path, &Default::default());
    }

    /// Watch for files the system (or another app's drop) handed the app while it was idle.
    ///
    /// egui runs a frame only when something asks it to, so a file that arrives while the app
    /// sits untouched would wait in the Activity's queue until the user happened to touch the
    /// screen. The watcher asks the Activity how many files are waiting (without taking any of
    /// them) and asks egui for a repaint when there is one, so the file opens on its own. The
    /// file itself is still opened on the frame thread, in `poll_incoming_files`.
    fn watch_incoming(&mut self, ctx: &egui::Context) {
        static WATCHER: std::sync::OnceLock<()> = std::sync::OnceLock::new();
        if WATCHER.set(()).is_err() {
            return;
        }
        let bridge = self.saf_bridge.clone_ref();
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
        self.saf_state.mirror.sync(&self.saf_bridge);
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

    /// Map Android's system back button onto Escape.
    ///
    /// winit's Android backend consumes `KEYCODE_BACK` (it reports the key as handled), so the
    /// framework's own back handling never runs and the button only reaches egui as
    /// `Key::BrowserBack` — which almost no app listens for. Every app already treats Escape as
    /// "close the dialog / cancel / go back", so the back press is re-labelled as Escape, giving
    /// the button its usual meaning with no app change. The press is also remembered, so
    /// [`AndroidShell::ui`] can finish the app when nothing consumed it (the normal Android
    /// "back exits" behaviour).
    fn map_android_back(&mut self, raw: &mut egui::RawInput) {
        let mut pressed = false;
        for event in &mut raw.events {
            if let egui::Event::Key { key, pressed: is_pressed, .. } = event
                && *key == egui::Key::BrowserBack
            {
                *key = egui::Key::Escape;
                pressed |= *is_pressed;
            }
        }
        if pressed {
            self.back_pending = true;
            log::info!("Android: back press mapped to Escape");
        }
    }

    /// The user asked to leave (the Android back button). Let the app have the first word: the
    /// apps that guard against losing work ask first, and only a close they agree to ends the
    /// app. `close_requested` is the desktop's own check — the desktop runs it when its window
    /// closes — so the app opens the same "Do you want to quit without saving unsaved changes?"
    /// prompt it shows on the desktop instead of vanishing with unsaved work.
    fn request_close(&mut self) {
        if self.closing {
            return;
        }
        log::info!("Android: the app asked to close");
        self.closing = true;
        finish_activity(&self.app);
    }

    /// Emit egui `Copy`/`Cut`/`Paste` events for Ctrl/Cmd+C/X/V key presses.
    ///
    /// winit's Android backend never emits `ModifiersChanged`, so egui-winit's own
    /// modifier-based clipboard-shortcut detection never fires there. The shell detects the
    /// shortcuts after [`apply_android_modifiers`] has reconstructed the modifier state and
    /// synthesizes the events egui's widgets (text edits, label selection) expect.
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

impl Drop for AndroidShell {
    fn drop(&mut self) {
        // The Activity is about to go: stop the watcher before the native thread returns.
        ALIVE.store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

impl eframe::App for AndroidShell {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if let Ok(mut g) = EGUI_CTX.lock() {
            if g.is_none() {
                *g = Some(ctx.clone());
            }
        }
        self.watch_incoming(ctx);
        self.poll_incoming_files();
        self.poll_file_picker();
        self.persist_tick();
        let _ = frame;
        self.inner.logic(ctx);
        self.sync_bar_appearance(ctx);
        if !self.closing && self.inner.quit_requested {
            // The user confirmed the app's own prompt ("Don't save", or the save
            // finished): the app may go now.
            self.closing = true;
            finish_activity(&self.app);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        system_fonts::install_system_fonts(ui.ctx());
        let _ = frame;
        self.inner.ui(ui);
        mobile_text_bar::show_text_bar(ui.ctx());
        if self.back_pending {
            self.back_pending = false;
            // A back press was mapped onto Escape. If the app consumed that Escape (a dialog
            // closed, a panel went back), the button did its job; otherwise back means "leave",
            // as it does on Android — which the app may still refuse, by asking first.
            if ui.ctx().input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                if self.close_asked {
                    // The app had its say last time (it opened a prompt, which this press just
                    // dismissed); don't ask it again straight away.
                    self.close_asked = false;
                } else {
                    self.request_close();
                }
            }
        }
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.probe.observe(raw);
        self.refresh_fullscreen(raw);
        apply_android_modifiers(&mut self.modifiers, raw);
        self.map_android_back(raw);
        self.handle_clipboard_shortcuts(raw);
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
    // A panic's backtrace, to see where the Vulkan surface is created.
    #[allow(unused_unsafe)]
    unsafe {
        std::env::set_var("RUST_BACKTRACE", "1");
        if let Some(path) = android_app.internal_data_path().or_else(|| android_app.external_data_path()) {
            let tmp = path.join("tmp");
            let _ = std::fs::create_dir_all(&tmp);
            std::env::set_var("TMPDIR", &tmp);
        }
    }
    // Route Rust `log` records to logcat; without this a failed picker/save was invisible.
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("com.artcraft.solidcraft"),
    );
    log::info!("{ARTCRAFT_ANDROID_SHELL_MARKER} ready ({})", saf::SAF_MARKER);
    apply_immersive_fullscreen(&android_app);
    egui_winit::android_activity::set_activity(android_app.activity_as_ptr());
    egui_winit::android_activity::set_vm(android_app.vm_as_ptr());
    install_persist_env(&android_app);
    system_fonts::install_font_env(&android_app);
    let saf_bridge = saf::Bridge::new(&android_app);
    saf_bridge.request_storage_permission();
    let saf_state = saf_new_state();
    let saf_inbox = SafInbox::default();
    let platform_services = saf_services(saf_bridge.clone_ref(), std::sync::Arc::clone(&saf_state), saf_inbox.clone());
    let (control_tx, control_rx) = std::sync::mpsc::channel();
    let _ = CONTROL_TX.set(control_tx);
    let inner = SolveApp::new(Session::default(), platform_services).with_control(control_rx);
    let _mcp_endpoint = mcp_http::start_mcp_service("SolidCraft".into(), "0.40.0-rc.3".into(), Arc::new(|| ALIVE.load(Ordering::Relaxed)));
    let mut shell = AndroidShell::new(inner, android_app.clone(), saf_bridge, saf_state, saf_inbox);
    let mut options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_title("SolidCraft")
            .with_drag_and_drop(true),
        android_app: Some(android_app.clone()),
        ..Default::default()
    };
    // Initialize egui theme from Android system UI mode (light/dark preference)
    let dark_mode = {
        use jni::{JavaVM, jni_sig, jni_str, objects::JObject};
        let app_clone = android_app.clone();
        let vm = unsafe { JavaVM::from_raw(app_clone.vm_as_ptr() as *mut _) };
        vm.attach_current_thread(|env| -> jni::errors::Result<bool> {
            let activity_raw = app_clone.activity_as_ptr() as jni::sys::jobject;
            let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
            let resources = env.call_method(activity, jni_str!("getResources"), jni_sig!("()Landroid/content/res/Resources;"), &[])?
                .l()?;
            let config = env.call_method(&resources, jni_str!("getConfiguration"), jni_sig!("()Landroid/content/res/Configuration;"), &[])?
                .l()?;
            let ui_mode = env.get_field(&config, jni_str!("uiMode"), jni_sig!("I"))?.i()?;
            // UI_MODE_NIGHT_YES = 0x20, UI_MODE_NIGHT_MASK = 0x30
            let is_dark = (ui_mode & 0x30) == 0x20;
            Ok(is_dark)
        }).unwrap_or(false)
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
                label: Some("SolidCraft wgpu device"),
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
        "SolidCraft",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_theme(egui::ThemePreference::System);
            solidcraft_ui_egui::fonts::setup_bundled_fonts(&cc.egui_ctx);
            if let Some(rs) = &cc.wgpu_render_state {
                let supported = eframe::egui_wgpu::supported_msaa_samples(&rs.adapter, &rs.device, rs.target_format, None);
                let samples = if supported.contains(&4) { 4 } else if supported.contains(&2) { 2 } else { 1 };
                shell.inner.set_wgpu_split(rs, 24, 8, samples);
                solidcraft_ui_egui::gpu::install_picture_renderer(rs);
            }
            Ok(Box::new(shell))
        }),
    );
    if let Err(error) = result {
        log::error!("SolidCraft Android exited with an error: {error}");
    }
}

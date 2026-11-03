//! Android system light/dark feed.
//!
//! The web builds learn the OS appearance from `prefers-color-scheme`, and desktop winit
//! delivers it as `WindowEvent::ThemeChanged` (which fills `RawInput::system_theme`).
//! Android's winit backend emits neither, so an app following the system theme — the
//! upstream default, `Appearance::System` → `ThemePreference::System` — would sit on its
//! start-up theme forever, and the interface would never switch with the phone.
//!
//! This module closes that gap the same way the clipboard bridge crosses into Java: each
//! frame, `take_egui_input` reads the Activity's current `Configuration.uiMode` over JNI
//! and hands the result to egui as `RawInput::system_theme` — the very field the desktop
//! path fills. egui resolves `ThemePreference::System` against that value, so the whole
//! interface re-themes live, and the shell's per-frame system-bar sync follows it too.
//!
//! A quick-settings toggle never restarts the Activity (the generated manifest declares
//! `uiMode` in its `configChanges`); when the shade closes the window regains focus, that
//! focus event schedules a frame, and the frame picks the new value up — all before the
//! app is visible again.
#![cfg_attr(not(any(target_os = "android", test)), allow(dead_code))]

use egui::Theme;

/// `Configuration.UI_MODE_NIGHT_MASK` / `UI_MODE_NIGHT_YES`: the two bits of
/// `Configuration.uiMode` that say whether the OS is currently in night mode.
const NIGHT_MASK: i32 = 0x30;
const NIGHT_YES: i32 = 0x20;

/// The theme the `Configuration.uiMode` night bits ask for.
pub fn theme_from_ui_mode(ui_mode: i32) -> Theme {
    if ui_mode & NIGHT_MASK == NIGHT_YES {
        Theme::Dark
    } else {
        Theme::Light
    }
}

/// The OS's current light/dark theme, read from the Activity's `Configuration`.
///
/// Returns `None` when the value cannot be read (egui then keeps its fallback theme —
/// the same behavior as a device that never reports one).
#[cfg(target_os = "android")]
#[expect(unsafe_code)]
pub fn system_theme() -> Option<Theme> {
    use jni::{jni_sig, jni_str, objects::JObject, JavaVM};

    let ctx = ndk_context::android_context();
    // SAFETY: AndroidApp owns the live VM pointer; `ndk-context` initialized it before `android_main`.
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) };
    let res = vm.attach_current_thread(|env| -> jni::errors::Result<Option<Theme>> {
        let ctx_raw: jni::sys::jobject = ctx.context().cast();
        // SAFETY: the context is an Android global ref owned by ndk-context; borrow, never delete.
        let activity = unsafe { env.as_cast_raw::<JObject<'_>>(&ctx_raw)? };
        let resources = env
            .call_method(
                &*activity,
                jni_str!("getResources"),
                jni_sig!("()Landroid/content/res/Resources;"),
                &[],
            )?
            .l()?;
        if resources.is_null() {
            return Ok(None);
        }
        let config = env
            .call_method(
                &resources,
                jni_str!("getConfiguration"),
                jni_sig!("()Landroid/content/res/Configuration;"),
                &[],
            )?
            .l()?;
        if config.is_null() {
            return Ok(None);
        }
        // `Configuration.uiMode` is a public int field (not a byte); its night bits are all
        // we need. The JNI field signature must match the Java type exactly — a wrong one
        // makes GetFieldID fail with NoSuchFieldError and the read come back `None`.
        let ui_mode = env.get_field(&config, jni_str!("uiMode"), jni_sig!("I"))?.i()?;
        Ok(Some(theme_from_ui_mode(ui_mode)))
    });
    match res {
        Ok(theme) => theme,
        Err(err) => {
            log::error!("egui-winit Android system theme read failed: {err}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use egui::Theme;

    #[test]
    fn night_bits_select_the_theme() {
        // Configuration.UI_MODE_NIGHT_YES on a normal "normal" (non-car/television) UI mode.
        let normal_night = 0x21; // UI_MODE_TYPE_NORMAL (0x01) | UI_MODE_NIGHT_YES (0x20)
        let normal_day = 0x01; // UI_MODE_TYPE_NORMAL | UI_MODE_NIGHT_NO (0x00)
        assert_eq!(super::theme_from_ui_mode(normal_night), Theme::Dark);
        assert_eq!(super::theme_from_ui_mode(normal_day), Theme::Light);
        // The type bits (car, television, …) must not influence the night decision.
        assert_eq!(super::theme_from_ui_mode(0x02 | 0x20), Theme::Dark);
        assert_eq!(super::theme_from_ui_mode(0x04 | 0x00), Theme::Light);
    }
}

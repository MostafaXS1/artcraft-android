// Read the system theme (light/dark) from Android's Configuration.uiMode.
use jni::{jni_sig, jni_str, objects::JObject, JavaVM};
use egui::Theme;
use android_activity::AndroidApp;

const NIGHT_MASK: i32 = 0x30;
const NIGHT_YES: i32 = 0x20;

fn theme_from_ui_mode(ui_mode: i32) -> Theme {
    if ui_mode & NIGHT_MASK == NIGHT_YES {
        Theme::Dark
    } else {
        Theme::Light
    }
}

/// Reads the OS's current light/dark theme from the Activity's Configuration.
/// Returns None when the value cannot be read.
pub fn read_android_system_theme(app: &AndroidApp) -> Option<Theme> {
    // AndroidApp owns the live VM pointer for the lifetime of the activity.
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr() as *mut _) };
    
    let res = vm.attach_current_thread(|env| -> jni::errors::Result<Option<Theme>> {
        let activity_raw = app.activity_as_ptr() as jni::sys::jobject;
        let activity = unsafe { env.as_cast_raw::<JObject<'_>>(&activity_raw)? };
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
        let ui_mode = env.get_field(&config, jni_str!("uiMode"), jni_sig!("I"))?.i()?;
        Ok(Some(theme_from_ui_mode(ui_mode)))
    });
    match res {
        Ok(theme) => theme,
        Err(err) => {
            log::error!("Android system theme read failed: {err}");
            None
        }
    }
}

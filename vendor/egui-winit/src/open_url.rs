//! Open a URL in the system browser from Android.
//!
//! egui's link widgets (`Hyperlink`, `OpenUrl` commands) emit an [`egui::OutputCommand::OpenUrl`]
//! that `egui-winit` forwards to `webbrowser::open`. That crate has an Android backend, but it is
//! only compiled when the `links` feature is enabled, and the ArtCraft apps build with
//! `eframe`'s defaults off — so on Android the command is silently dropped and a clicked link does
//! nothing. The app's own `open_url` service is likewise unset on Android, so every link path ends
//! here.
//!
//! Rather than depend on an optional feature, this opens the URL the way Android wants it: an
//! `ACTION_VIEW` intent on the Activity, which hands the URL to whichever browser the device has.
//! The Activity handle is published by the shell ([`crate::android_activity`]).
#![cfg(target_os = "android")]

/// `Activity.startActivity(Intent)` → an `ACTION_VIEW` intent for `url`, or `false` if the Activity
/// is not published yet or the intent could not be started (in which case nothing should happen:
/// a device with no browser at all is not an error worth surfacing).
#[expect(unsafe_code)]
pub fn open_url(url: &str) -> bool {
    let ctx = ndk_context::android_context();
    // SAFETY: AndroidApp owns the live VM pointer; `ndk-context` initialized it before `android_main`.
    let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) };
    let Some(activity_ptr) = crate::android_activity::activity_raw() else {
        return false;
    };

    let result = vm.attach_current_thread(|env| -> jni::errors::Result<bool> {
        use jni::{jni_sig, jni_str, objects::{JObject, JValue}};

        // SAFETY: the shell stored an unowned JNI global reference owned by `android-activity`.
        let activity_raw = activity_ptr;
        let activity = unsafe { env.as_cast_raw::<JObject<'_>>(&activity_raw)? };

        // `Uri.parse(url)`
        let uri_class = env.find_class(jni_str!("android/net/Uri"))?;
        let url_obj = env.new_string(url)?;
        let url_ref: &JObject<'_> = url_obj.as_ref();
        let uri = env
            .call_static_method(
                &uri_class,
                jni_str!("parse"),
                jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"),
                &[JValue::Object(url_ref)],
            )?
            .l()?;
        if uri.is_null() {
            return Ok(false);
        }

        // `new Intent(Intent.ACTION_VIEW, uri)`
        let intent_class = env.find_class(jni_str!("android/content/Intent"))?;
        let action_view = env
            .get_static_field(
                &intent_class,
                jni_str!("ACTION_VIEW"),
                jni_sig!("Ljava/lang/String;"),
            )?
            .l()?;
        let intent = env.new_object(
            &intent_class,
            jni_sig!("(Ljava/lang/String;Landroid/net/Uri;)V"),
            &[JValue::Object(&action_view), JValue::Object(&uri)],
        )?;

        // `intent.addCategory(Intent.CATEGORY_BROWSABLE)` — keep it in the foreground browser
        // rather than whatever the system decides is a handler.
        let category = env
            .get_static_field(
                &intent_class,
                jni_str!("CATEGORY_BROWSABLE"),
                jni_sig!("Ljava/lang/String;"),
            )?
            .l()?;
        env.call_method(
            &intent,
            jni_str!("addCategory"),
            jni_sig!("(Ljava/lang/String;)Landroid/content/Intent;"),
            &[JValue::Object(&category)],
        )?;

        // `intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)` — starting from an Activity context does
        // not need it, but the extra task keeps a second tap from stacking on the current one.
        let flags = env
            .get_static_field(
                &intent_class,
                jni_str!("FLAG_ACTIVITY_NEW_TASK"),
                jni_sig!("I"),
            )?
            .i()?;
        env.call_method(
            &intent,
            jni_str!("addFlags"),
            jni_sig!("(I)Landroid/content/Intent;"),
            &[JValue::Int(flags)],
        )?;

        // `activity.startActivity(intent)`
        env.call_method(
            &*activity,
            jni_str!("startActivity"),
            jni_sig!("(Landroid/content/Intent;)V"),
            &[JValue::Object(&intent)],
        )?;
        Ok(true)
    });

    match result {
        Ok(opened) => opened,
        Err(err) => {
            log::error!("egui-winit Android open_url failed: {err}");
            false
        }
    }
}

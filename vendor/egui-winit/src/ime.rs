//! High-performance native Android IME text bridge.

#[cfg(target_os = "android")]
pub mod android {
    use jni::{jni_sig, jni_str, objects::{JObject, JString}, JavaVM};

    #[expect(unsafe_code)]
    pub fn poll_all_ime_events() -> Vec<String> {
        let activity_raw = crate::android_activity::activity_raw();
        let vm_ptr: Option<*mut jni::sys::JavaVM> = crate::android_activity::vm_raw().or_else(|| {
            let ctx = ndk_context::android_context();
            let p: *mut jni::sys::JavaVM = ctx.vm().cast();
            if p.is_null() {
                None
            } else {
                Some(p)
            }
        });
        let Some(vm_ptr) = vm_ptr else {
            return Vec::new();
        };
        let Some(activity_raw) = activity_raw else {
            return Vec::new();
        };
        if activity_raw.is_null() {
            return Vec::new();
        }
        let vm = unsafe { JavaVM::from_raw(vm_ptr) };
        let res = vm.attach_current_thread(|env| -> jni::errors::Result<Vec<String>> {
            let activity = unsafe { env.as_cast_raw::<JObject<'_>>(&activity_raw)? };
            let mut list = Vec::new();
            while let Ok(val) = env.call_method(
                &*activity,
                jni_str!("pollIme"),
                jni_sig!("()Ljava/lang/String;"),
                &[],
            ) {
                let obj = val.l()?;
                if obj.is_null() {
                    break;
                }
                let jstr = JString::cast_local(env, obj)?;
                list.push(jstr.try_to_string(env)?);
            }
            Ok(list)
        });
        res.unwrap_or_default()
    }
}

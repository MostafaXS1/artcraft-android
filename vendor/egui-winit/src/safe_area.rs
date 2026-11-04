#[cfg(target_os = "ios")]
pub use ios::get_safe_area_insets;

#[cfg(target_os = "android")]
#[allow(unused_imports)]
pub use android::{get_safe_area_insets, top_inset_pixels};

#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub fn get_safe_area_insets() -> egui::SafeAreaInsets {
    egui::SafeAreaInsets::default()
}

#[cfg(target_os = "android")]
mod android {
    use egui::{epaint::MarginF32, SafeAreaInsets};
    use jni::{jni_sig, jni_str, objects::JObject, JavaVM};

    #[expect(unsafe_code)]
    pub fn top_inset_pixels() -> f32 {
        let activity_raw = crate::android_activity::activity_raw();
        let vm_ptr: Option<*mut jni::sys::JavaVM> = crate::android_activity::vm_raw().or_else(|| {
            let ctx = ndk_context::android_context();
            let p: *mut jni::sys::JavaVM = ctx.vm().cast();
            if p.is_null() { None } else { Some(p) }
        });
        let Some(vm_ptr) = vm_ptr else {
            return 0.0;
        };
        let Some(activity_raw) = activity_raw else {
            return 0.0;
        };
        if activity_raw.is_null() {
            return 0.0;
        }
        let vm = unsafe { JavaVM::from_raw(vm_ptr) };
        let res = vm.attach_current_thread(|env| -> jni::errors::Result<f32> {
            let activity = unsafe { env.as_cast_raw::<JObject<'_>>(&activity_raw)? };
            match env.call_method(&*activity, jni_str!("getSafeInsetTop"), jni_sig!("()I"), &[]) {
                Ok(val) => Ok(val.i()? as f32),
                Err(_) => {
                    let _ = env.exception_clear();
                    Ok(0.0)
                }
            }
        });
        res.unwrap_or(0.0)
    }

    pub fn get_safe_area_insets() -> SafeAreaInsets {
        let top_px = top_inset_pixels();
        if top_px > 0.0 {
            SafeAreaInsets(MarginF32 {
                top: top_px,
                left: 0.0,
                right: 0.0,
                bottom: 0.0,
            })
        } else {
            SafeAreaInsets::default()
        }
    }
}

#[cfg(target_os = "ios")]
mod ios {
    use egui::{SafeAreaInsets, epaint::MarginF32};
    use objc2::{ClassType, rc::Retained};
    use objc2_foundation::{MainThreadMarker, NSObjectProtocol};
    use objc2_ui_kit::{UIApplication, UISceneActivationState, UIWindowScene};

    /// Gets the ios safe area insets.
    ///
    /// A safe area defines the area within a view that isn’t covered by a navigation bar, tab bar,
    /// toolbar, or other views a window might provide. Safe areas are essential for avoiding a
    /// device’s interactive and display features, like Dynamic Island on iPhone or the camera
    /// housing on some Mac models.
    ///
    /// Once winit v0.31 has been released this can be removed in favor of
    /// `winit::Window::safe_area`.
    pub fn get_safe_area_insets() -> SafeAreaInsets {
        let Some(main_thread_marker) = MainThreadMarker::new() else {
            log::error!("Getting safe area insets needs to be performed on the main thread");
            return SafeAreaInsets::default();
        };

        let app = UIApplication::sharedApplication(main_thread_marker);

        #[expect(unsafe_code)]
        unsafe {
            // Look for the first window scene that's in the foreground
            for scene in app.connectedScenes() {
                if scene.isKindOfClass(UIWindowScene::class())
                    && matches!(
                        scene.activationState(),
                        UISceneActivationState::ForegroundActive
                            | UISceneActivationState::ForegroundInactive
                    )
                {
                    // SAFETY: class kind was checked above with `isKindOfClass`
                    let window_scene = Retained::cast_unchecked::<UIWindowScene>(scene.clone());
                    if let Some(window) = window_scene.keyWindow() {
                        let insets = window.safeAreaInsets();
                        return SafeAreaInsets(MarginF32 {
                            top: insets.top as f32,
                            left: insets.left as f32,
                            right: insets.right as f32,
                            bottom: insets.bottom as f32,
                        });
                    }
                }
            }
        }

        SafeAreaInsets::default()
    }
}

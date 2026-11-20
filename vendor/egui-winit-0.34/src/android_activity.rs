//! Shared Android JNI plumbing for the `egui-winit` Android integrations.
//!
//! `ndk_context::android_context()` hands out the *Application*, not the Activity, and the
//! integrations here need `Activity`-only APIs (`getWindow()` for the pointer icon,
//! `startActivity()` for links). The app shell has the real handle at startup, so it publishes it
//! once here; the bridges read it back.

#![cfg(target_os = "android")]

use std::sync::atomic::{AtomicUsize, Ordering};

/// The Activity jobject, published once by the app shell (`AndroidApp::activity_as_ptr`).
///
/// The pointer is an unowned JNI global reference owned by `android-activity`, so it must never be
/// deleted by anything that reads it here.
static ACTIVITY: AtomicUsize = AtomicUsize::new(0);
static VM: AtomicUsize = AtomicUsize::new(0);

/// Publish the Activity jobject for the bridges. Call once at startup with
/// `app.activity_as_ptr() as usize`. Updates on activity recreation.
pub fn set_activity(activity_as_ptr: *mut std::ffi::c_void) {
    ACTIVITY.store(activity_as_ptr as usize, Ordering::Release);
}

/// Publish the JavaVM pointer for the bridges. Call once at startup with
/// `app.vm_as_ptr() as usize`. Updates on activity recreation.
pub fn set_vm(vm_as_ptr: *mut std::ffi::c_void) {
    VM.store(vm_as_ptr as usize, Ordering::Release);
}

/// The published Activity jobject, or `None` before the shell publishes it.
pub fn activity_raw() -> Option<jni::sys::jobject> {
    let ptr = ACTIVITY.load(Ordering::Acquire);
    if ptr != 0 {
        Some(ptr as jni::sys::jobject)
    } else {
        None
    }
}

/// The published JavaVM pointer, or `None` before the shell publishes it.
pub fn vm_raw() -> Option<*mut jni::sys::JavaVM> {
    let ptr = VM.load(Ordering::Acquire);
    if ptr != 0 {
        Some(ptr as *mut jni::sys::JavaVM)
    } else {
        None
    }
}

//! A host-side test of the Android input classification.
//!
//! The backend's Android code cannot be built or run on the host — it is behind
//! `target_os = "android"` — but the part that works out what a device is needs nothing
//! but arithmetic, so it is included here and tested with `rustc` alone:
//!
//! ```sh
//! rustc --edition 2021 --test tests/android_device_kind.rs -o /tmp/device_kind_test
//! /tmp/device_kind_test
//! ```

#[path = "../vendor/winit/src/platform_impl/android/device_kind.rs"]
mod device_kind;

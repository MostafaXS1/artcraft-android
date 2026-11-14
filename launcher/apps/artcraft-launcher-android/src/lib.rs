//! ArtCraft Launcher Android shell.
//!
//! GameActivity loads this cdylib and calls `android_main`. The UI is `artcraft-launcher-ui-egui`,
//! driven by an `AndroidBackend` that implements the same OS seam the desktop and web shells use.

// The host build exists only for the catalog and bridge tests: the UI, backend and their helpers
// are Android-only, so on other targets they look unused.
#![cfg_attr(not(target_os = "android"), allow(dead_code))]

mod about;
#[cfg(target_os = "android")]
mod app;
#[cfg(target_os = "android")]
mod backend;
mod bridge;
mod catalog;
mod paths;
#[cfg(target_os = "android")]
mod settings;

#[cfg(target_os = "android")]
use crate::app::run;
#[cfg(target_os = "android")]
use android_activity::AndroidApp;

/// Entry point called by GameActivity.
// android-activity looks the entry point up by name, hence `no_mangle` (the crate's only
// `unsafe`; see `Cargo.toml`).
#[allow(unsafe_code)]
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("ArtCraftLauncher"));
    std::panic::set_hook(Box::new(|info| log::error!("panic: {info}")));
    if let Err(e) = run(app) {
        log::error!("ArtCraft Launcher stopped: {e}");
    }
}

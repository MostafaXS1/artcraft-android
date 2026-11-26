//! Runtime Android fullscreen (immersive mode) preference toggle.
//!
//! Zero external dependencies beyond `std` and `egui`, so it compiles cleanly in shared
//! desktop/Android `ui-egui` crates. On Android, the launcher registers the JNI handler via
//! `set_handler` and initializes the saved preference via `set_initial`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

static FULLSCREEN: AtomicBool = AtomicBool::new(true);
static HANDLER: Mutex<Option<Box<dyn Fn(bool) + Send + Sync>>> = Mutex::new(None);

pub fn is_enabled() -> bool {
    FULLSCREEN.load(Ordering::Relaxed)
}

pub fn set_initial(enabled: bool) {
    FULLSCREEN.store(enabled, Ordering::Relaxed);
}

pub fn set_handler(handler: impl Fn(bool) + Send + Sync + 'static) {
    if let Ok(mut guard) = HANDLER.lock() {
        *guard = Some(Box::new(handler));
    }
}

pub fn set_enabled(enabled: bool) {
    FULLSCREEN.store(enabled, Ordering::Relaxed);
    if let Ok(guard) = HANDLER.lock() {
        if let Some(h) = guard.as_ref() {
            h(enabled);
        }
    }
}

/// Renders the "Fullscreen (Immersive Mode)" checkbox under Theme settings.
pub fn ui(ui: &mut egui::Ui) {
    let mut val = is_enabled();
    if ui.checkbox(&mut val, "Fullscreen (Immersive Mode)").changed() {
        set_enabled(val);
    }
}

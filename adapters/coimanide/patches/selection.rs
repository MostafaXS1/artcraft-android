//! Where the selected text is, for the Android text bar.
//!
//! Coiman's editor and terminal draw their own text, so neither is an egui `TextEdit` and the
//! platform's text bar cannot find their selection through egui's own state. They publish the
//! selection's rectangle here instead, and the bar places itself next to it.
//!
//! The rectangle is `[min.x, min.y, max.x, max.y]` in egui points, or `None` when nothing is
//! selected. It is written and read on the UI thread; the lock keeps the static well-defined
//! between frames.

use std::sync::RwLock;

static RECT: RwLock<Option<[f32; 4]>> = RwLock::new(None);

/// Publishes where the selected text is, or `None` when there is no selection.
pub fn set(rect: Option<[f32; 4]>) {
    if let Ok(mut slot) = RECT.write() {
        *slot = rect;
    }
}

/// Where the selected text is, if there is one.
pub fn get() -> Option<[f32; 4]> {
    RECT.read().ok().and_then(|slot| *slot)
}

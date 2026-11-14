//! macOS only: which desktop (Space) the launcher's window appears on when it's shown.
//!
//! By default macOS switches to the desktop the window was last on, which is jarring when the
//! launcher is opened from the menu bar on another desktop. With "move to active space" set, the
//! window comes to the desktop you're on instead (and leaves the one it was on). Other platforms
//! show a hidden window on the current desktop anyway, so this module only exists on macOS.

use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSWindowCollectionBehavior};

/// Makes the launcher's windows follow the current desktop when shown (`true`), or stay on their
/// own desktop and switch to it (`false`, macOS's default). Must run on the main thread, as the
/// UI does; elsewhere it does nothing.
pub fn follow_current_desktop(on: bool) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    for window in app.windows().iter() {
        let mut behavior = window.collectionBehavior();
        behavior.set(NSWindowCollectionBehavior::MoveToActiveSpace, on);
        window.setCollectionBehavior(behavior);
    }
}

//! Bounded hand-off from Android background workers to FigCraft's live UI thread.
//!
//! Producers must never mutate FigApp or Session directly. They enqueue the upstream
//! ControlRequest here; AndroidShell drains it immediately before FigApp::ui, where
//! FigCraft's own control::drain performs validation and execution.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

use figcraft_ui_egui::{ControlRequest, FigApp};

const MAX_PENDING_REQUESTS: usize = 64;

fn queue() -> &'static Mutex<VecDeque<ControlRequest>> {
    static QUEUE: OnceLock<Mutex<VecDeque<ControlRequest>>> = OnceLock::new();
    QUEUE.get_or_init(|| Mutex::new(VecDeque::new()))
}

/// Enqueue one request from a background worker.
///
/// Returns the original request when the queue is full, allowing the transport to
/// return an explicit overload error instead of silently dropping work.
pub fn enqueue(request: ControlRequest) -> Result<(), ControlRequest> {
    let mut pending = queue()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if pending.len() >= MAX_PENDING_REQUESTS {
        return Err(request);
    }
    pending.push_back(request);
    Ok(())
}

/// Transfer pending requests to FigCraft on the UI thread.
///
/// The app's own control::drain then handles cancellation, execution and replies.
pub fn drain(app: &mut FigApp) {
    let requests: Vec<_> = {
        let mut pending = queue()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        pending.drain(..).collect()
    };
    app.control_queue.extend(requests);
}

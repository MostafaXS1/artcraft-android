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
fn push_bounded<T>(pending: &mut VecDeque<T>, item: T, limit: usize) -> Result<(), T> {
    if pending.len() >= limit {
        return Err(item);
    }
    pending.push_back(item);
    Ok(())
}

pub fn enqueue(request: ControlRequest) -> Result<(), ControlRequest> {
    let mut pending = queue()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    push_bounded(&mut pending, request, MAX_PENDING_REQUESTS)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_rejects_overflow_without_dropping_the_item() {
        let mut pending = VecDeque::new();
        for value in 0..MAX_PENDING_REQUESTS {
            assert!(push_bounded(&mut pending, value, MAX_PENDING_REQUESTS).is_ok());
        }
        assert_eq!(pending.len(), MAX_PENDING_REQUESTS);
        assert_eq!(push_bounded(&mut pending, 99, MAX_PENDING_REQUESTS), Err(99));
        assert_eq!(pending.len(), MAX_PENDING_REQUESTS);
    }
}

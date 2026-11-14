//! Request-bound IME resets. Calling the OS is not an acknowledgement.

use egui::{Event, ImeResetId, ViewportId, output::IMEOutput};
use winit::event::WindowEvent;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ContextKey {
    owner: egui::Id,
    generation: u64,
    purpose: egui::IMEPurpose,
}

impl From<&IMEOutput> for ContextKey {
    fn from(ime: &IMEOutput) -> Self {
        Self {
            owner: ime.owner,
            generation: ime.context_generation,
            purpose: ime.purpose,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Pending {
    id: ImeResetId,
    context: ContextKey,
    dispatched: bool,
}

/// One State's actual native window. Serial identity is process-unique across Context replacement.
pub(crate) struct ResetBridge<W> {
    window: Option<W>,
    latest: Option<ImeResetId>,
    pending: Option<Pending>,
}

impl<W> Default for ResetBridge<W> {
    fn default() -> Self {
        Self {
            window: None,
            latest: None,
            pending: None,
        }
    }
}

impl<W: Copy + Eq> ResetBridge<W> {
    /// Invalidating the target never authorizes an old completion to release a later guard.
    pub(crate) fn invalidate(&mut self) {
        self.pending = None;
    }

    /// The command dispatcher changes native purpose without requiring another settled output.
    /// Identical commands retain their context; changed purpose retires the outstanding fence.
    pub(crate) fn apply_purpose_command(
        &mut self,
        old: &mut egui::IMEPurpose,
        new: egui::IMEPurpose,
    ) -> bool {
        if *old == new {
            return false;
        }
        self.invalidate();
        *old = new;
        true
    }

    /// A command-side disable retires a dispatched native fence. An undispatched request can
    /// still wait for a later allowed descriptor; no completion exists for it to accept yet.
    pub(crate) fn external_allowed(&mut self, allowed: bool) {
        if !allowed && self.pending.is_some_and(|pending| pending.dispatched) {
            self.invalidate();
        }
    }

    /// Called only for settled platform output. Returns one actual native call to make.
    pub(crate) fn prepare(
        &mut self,
        window: W,
        viewport: ViewportId,
        ime: Option<&IMEOutput>,
        allowed: bool,
    ) -> Option<ImeResetId> {
        if self.window != Some(window) {
            self.window = Some(window);
            self.pending = None;
            // Retain latest so old output cannot replay a request on a replacement window.
        }
        let Some(ime) = ime else {
            self.invalidate();
            return None;
        };
        let context = ContextKey::from(ime);
        if self
            .pending
            .is_some_and(|pending| pending.context != context)
        {
            self.invalidate();
        }
        if let Some(id) = ime.reset_request
            && id.viewport_id() == viewport
            && self
                .latest
                .is_none_or(|latest| id.as_u64() > latest.as_u64())
        {
            self.latest = Some(id);
            self.pending = Some(Pending {
                id,
                context,
                dispatched: false,
            });
        }
        let pending = self.pending.as_mut()?;
        if allowed && !pending.dispatched {
            pending.dispatched = true;
            Some(pending.id)
        } else {
            None
        }
    }

    /// The real `WindowEvent` is translated exactly here, in the normal raw event stream.
    /// Other/wrong-window/stale/duplicate control events never change the input vector.
    pub(crate) fn on_event(
        &mut self,
        window: W,
        viewport: ViewportId,
        event: &WindowEvent,
        events: &mut Vec<Event>,
    ) -> bool {
        let WindowEvent::ImeResetCompleted { request_id } = event else {
            return false;
        };
        let Some(pending) = self.pending else {
            return false;
        };
        if self.window != Some(window)
            || pending.id.viewport_id() != viewport
            || pending.id.as_u64() != *request_id
            || !pending.dispatched
        {
            return false;
        }
        self.pending = None;
        events.push(Event::ImeResetCompleted {
            request_id: pending.id,
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Context, IMEPurpose, Id, Rect};

    fn output(ctx: &Context, viewport: ViewportId) -> IMEOutput {
        let mut ime = IMEOutput::new(
            Id::unique("editor"),
            1,
            IMEPurpose::Normal,
            Rect::ZERO,
            Rect::ZERO,
        );
        ime.reset_request = ctx.request_ime_reset(viewport);
        assert!(ime.reset_request.is_some());
        ime.should_interrupt_composition = true;
        ime
    }

    #[test]
    fn actual_window_marker_is_in_order_without_call_return_acknowledgement() {
        let ctx = Context::default();
        let viewport = ViewportId::ROOT;
        let ime = output(&ctx, viewport);
        let id = ime.reset_request.unwrap();
        let mut bridge = ResetBridge::<u64>::default();
        let mut events = vec![Event::Ime(egui::ImeEvent::Commit("old".into()))];
        assert_eq!(bridge.prepare(11, viewport, Some(&ime), true), Some(id));
        assert_eq!(
            events.len(),
            1,
            "native call preparation must not fabricate completion"
        );
        assert!(!bridge.on_event(
            12,
            viewport,
            &WindowEvent::ImeResetCompleted {
                request_id: id.as_u64()
            },
            &mut events
        ));
        assert_eq!(events.len(), 1);
        assert!(bridge.on_event(
            11,
            viewport,
            &WindowEvent::ImeResetCompleted {
                request_id: id.as_u64()
            },
            &mut events
        ));
        events.push(Event::Ime(egui::ImeEvent::Preedit {
            text: "new".into(),
            active_range_chars: None,
        }));
        assert!(
            matches!(&events[..], [Event::Ime(egui::ImeEvent::Commit(old)), Event::ImeResetCompleted { request_id }, Event::Ime(egui::ImeEvent::Preedit { text, .. })] if old == "old" && *request_id == id && text == "new")
        );
        assert!(!bridge.on_event(
            11,
            viewport,
            &WindowEvent::ImeResetCompleted {
                request_id: id.as_u64()
            },
            &mut events
        ));
        assert_eq!(
            bridge.prepare(11, viewport, Some(&ime), true),
            None,
            "replayed settled output stays one-shot"
        );
    }

    #[test]
    fn same_owner_caret_requests_supersede_and_old_output_cannot_revive_them() {
        let ctx = Context::default();
        let viewport = ViewportId::ROOT;
        let first = output(&ctx, viewport);
        let newer = output(&ctx, viewport);
        let old = first.reset_request.unwrap();
        let new = newer.reset_request.unwrap();
        let mut bridge = ResetBridge::<u64>::default();
        let mut events = Vec::new();
        assert_eq!(bridge.prepare(1, viewport, Some(&first), true), Some(old));
        assert_eq!(bridge.prepare(1, viewport, Some(&newer), true), Some(new));
        assert_eq!(bridge.prepare(1, viewport, Some(&first), true), None);
        assert!(!bridge.on_event(
            1,
            viewport,
            &WindowEvent::ImeResetCompleted {
                request_id: old.as_u64()
            },
            &mut events
        ));
        assert!(bridge.on_event(
            1,
            viewport,
            &WindowEvent::ImeResetCompleted {
                request_id: new.as_u64()
            },
            &mut events
        ));
        assert_eq!(events, vec![Event::ImeResetCompleted { request_id: new }]);
    }

    #[test]
    fn disallowed_context_defers_without_synthetic_ack_then_dispatches_once() {
        let ctx = Context::default();
        let viewport = ViewportId::ROOT;
        let mut ime = output(&ctx, viewport);
        let id = ime.reset_request.unwrap();
        let mut bridge = ResetBridge::<u64>::default();
        let mut events = Vec::new();
        assert_eq!(bridge.prepare(1, viewport, Some(&ime), false), None);
        assert!(!bridge.on_event(
            1,
            viewport,
            &WindowEvent::ImeResetCompleted {
                request_id: id.as_u64()
            },
            &mut events
        ));
        ime.reset_request = None;
        assert_eq!(bridge.prepare(1, viewport, Some(&ime), true), Some(id));
        assert_eq!(bridge.prepare(1, viewport, Some(&ime), true), None);
        assert!(bridge.on_event(
            1,
            viewport,
            &WindowEvent::ImeResetCompleted {
                request_id: id.as_u64()
            },
            &mut events
        ));
    }

    #[test]
    fn generic_lifecycle_wrong_token_and_invalidated_context_are_not_acknowledgements() {
        let ctx = Context::default();
        let viewport = ViewportId::ROOT;
        let ime = output(&ctx, viewport);
        let id = ime.reset_request.unwrap();
        let mut bridge = ResetBridge::<u64>::default();
        let mut events = Vec::new();
        assert_eq!(bridge.prepare(1, viewport, Some(&ime), true), Some(id));
        for event in [
            WindowEvent::Ime(winit::event::Ime::Disabled),
            WindowEvent::Ime(winit::event::Ime::Enabled),
            WindowEvent::ImeResetCompleted {
                request_id: u64::MAX,
            },
        ] {
            assert!(!bridge.on_event(1, viewport, &event, &mut events));
        }
        assert!(events.is_empty());
        bridge.invalidate();
        assert!(!bridge.on_event(
            1,
            viewport,
            &WindowEvent::ImeResetCompleted {
                request_id: id.as_u64()
            },
            &mut events
        ));
        assert_eq!(
            bridge.prepare(1, viewport, Some(&ime), true),
            None,
            "focus/destroy invalidation cannot replay old output"
        );
    }

    #[test]
    fn replacement_window_context_and_wrong_viewport_never_ack_old_request() {
        let ctx = Context::default();
        let viewport = ViewportId::ROOT;
        let mut ime = output(&ctx, viewport);
        let id = ime.reset_request.unwrap();
        let other = ViewportId::from_hash_of("other");
        let mut bridge = ResetBridge::<u64>::default();
        let mut events = Vec::new();
        assert_eq!(bridge.prepare(1, other, Some(&ime), true), None);
        assert_eq!(bridge.prepare(1, viewport, Some(&ime), true), Some(id));
        assert_eq!(bridge.prepare(2, viewport, Some(&ime), true), None);
        assert!(!bridge.on_event(
            1,
            viewport,
            &WindowEvent::ImeResetCompleted {
                request_id: id.as_u64()
            },
            &mut events
        ));
        ime.reset_request = ctx.request_ime_reset(viewport);
        let current = ime.reset_request.unwrap();
        assert_eq!(bridge.prepare(2, viewport, Some(&ime), true), Some(current));
        ime.context_generation += 1;
        ime.reset_request = None;
        bridge.prepare(2, viewport, Some(&ime), true);
        assert!(!bridge.on_event(
            2,
            viewport,
            &WindowEvent::ImeResetCompleted {
                request_id: current.as_u64()
            },
            &mut events
        ));
        assert!(events.is_empty());
    }
    #[test]
    fn actual_command_purpose_change_retires_marker_but_identical_command_keeps_it() {
        let ctx = Context::default();
        let viewport = ViewportId::ROOT;
        let mut ime = output(&ctx, viewport);
        let mut purpose = IMEPurpose::Normal;
        let mut bridge = ResetBridge::<u64>::default();
        let mut events = Vec::new();
        let first = ime.reset_request.unwrap();
        assert_eq!(bridge.prepare(1, viewport, Some(&ime), true), Some(first));
        assert!(!bridge.apply_purpose_command(&mut purpose, IMEPurpose::Normal));
        assert!(bridge.on_event(
            1,
            viewport,
            &WindowEvent::ImeResetCompleted {
                request_id: first.as_u64()
            },
            &mut events
        ));
        ime.reset_request = ctx.request_ime_reset(viewport);
        let old = ime.reset_request.unwrap();
        bridge.prepare(1, viewport, Some(&ime), true);
        assert!(bridge.apply_purpose_command(&mut purpose, IMEPurpose::Password));
        assert_eq!(purpose, IMEPurpose::Password);
        assert!(!bridge.on_event(
            1,
            viewport,
            &WindowEvent::ImeResetCompleted {
                request_id: old.as_u64()
            },
            &mut events
        ));
        ime.purpose = IMEPurpose::Password;
        ime.reset_request = ctx.request_ime_reset(viewport);
        let new = ime.reset_request.unwrap();
        assert_eq!(bridge.prepare(1, viewport, Some(&ime), true), Some(new));
        assert!(bridge.on_event(
            1,
            viewport,
            &WindowEvent::ImeResetCompleted {
                request_id: new.as_u64()
            },
            &mut events
        ));
        assert_eq!(
            events,
            vec![
                Event::ImeResetCompleted { request_id: first },
                Event::ImeResetCompleted { request_id: new }
            ]
        );
    }

    #[test]
    fn external_disable_rejects_dispatched_marker_but_preserves_undispatched_deferral() {
        let ctx = Context::default();
        let viewport = ViewportId::ROOT;
        let mut ime = output(&ctx, viewport);
        let old = ime.reset_request.unwrap();
        let mut bridge = ResetBridge::<u64>::default();
        let mut events = Vec::new();
        bridge.prepare(1, viewport, Some(&ime), true);
        bridge.external_allowed(false);
        assert!(!bridge.on_event(
            1,
            viewport,
            &WindowEvent::ImeResetCompleted {
                request_id: old.as_u64()
            },
            &mut events
        ));
        ime.reset_request = ctx.request_ime_reset(viewport);
        let next = ime.reset_request.unwrap();
        assert_eq!(bridge.prepare(1, viewport, Some(&ime), false), None);
        bridge.external_allowed(false);
        bridge.external_allowed(true);
        ime.reset_request = None;
        assert_eq!(bridge.prepare(1, viewport, Some(&ime), true), Some(next));
        assert!(bridge.on_event(
            1,
            viewport,
            &WindowEvent::ImeResetCompleted {
                request_id: next.as_u64()
            },
            &mut events
        ));
    }
}

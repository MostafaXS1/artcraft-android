//! Outgoing IME surrounding-text synchronization for the Wayland backend.
//!
//! [`egui::output::IMEOutput`] is produced once per pass by the widget that owns IME input.
//! This module turns that transient descriptor into ordered window operations:
//!
//! * ordinary text/selection edits for the same owner and context generation only update the
//!   surrounding text, so an ongoing composition is preserved;
//! * a new owner, context generation or IME purpose first clears the old protocol context so a
//!   composition from the previous editor is never carried into the new one;
//! * an explicit [`egui::output::IMESurroundingText::Unavailable`] or a password purpose clears
//!   the old text rather than publishing a placeholder or stale text;
//! * a valid, explicitly empty payload is sent as `set_surrounding_text("", 0, 0)` and is
//!   therefore distinct from an unavailable context.
//!
//! The planner is deliberately independent of winit. It only mutates its cache after every sink
//! operation for the pass reported [`SinkOutcome::Applied`]; a locally suppressed write is never
//! remembered as published. The production sink is the small Wayland adapter at the bottom of
//! this file.
//!
//! # Supported targets
//!
//! The module is compiled exactly where winit's Wayland backend exists: `feature = "wayland"` on
//! free Unix targets outside Apple, Android, Emscripten and Redox. That covers Linux, the BSDs and
//! any other target winit's `wayland_platform` cfg accepts; it is not available on macOS/Windows.
//! X11 windows in the same winit build are rejected at runtime through
//! `xdg_toplevel().is_none()` and keep their pre-existing IME behavior.
//!
//! # Command ownership
//!
//! External [`egui::ViewportCommand::IMEAllowed`] commands are applied by the owning
//! [`crate::State`] through `process_viewport_commands`. A disable commits synchronously. An
//! enable only records a pending request and is committed by a later platform-output pass that
//! carries an IME descriptor, so stale descriptor state cannot enable an editor that is gone.
//! eframe pre-applies these IME commands before platform output; `egui_glow` already runs commands
//! first. An immediate viewport's own command is returned with that child pass, so its renderer
//! can apply it before publishing the child's platform output; the parent keeps its own and any
//! sibling-directed commands. There is no process-global pending map, window-id key, birth stamp
//! or destruction-time cleanup registry. Integrations must process commands only when both the
//! window and its `State` exist; eframe defers them until then. `WindowEvent::Destroyed` resets
//! the per-window IME latches on every platform for a potentially reused `State`.
//!
//! # Required winit capability (E21b)
//!
//! Context clears go through the dedicated `WindowExtWayland::clear_ime_surrounding_text()`. It
//! forgets winit's cached text, leaves the window's allowed state untouched, never enables a
//! disabled window, and sends a committed disable/enable reset only when text was published or a
//! composition was active. The consumer must not emulate it with `set_ime_allowed(false)`
//! followed by `set_ime_allowed(true)`: those commits re-enable a window that the integration,
//! the focus latch or an external `ViewportCommand::IMEAllowed(false)` deliberately disabled.
//!
//! The workspace pins the reviewed `storytold/winit` revision
//! `ae48c756a3b81125eb796c0cc6dccf60990d1181`, which provides this API. `Cargo.lock` records
//! that same immutable canonical Git source. A local path override is not required by this
//! consumer; the source capability is checked by compiling the calls below. `xdg_toplevel()`
//! identifies a Wayland window and does not by itself prove compositor text-input support.

use core::fmt;

use egui::{
    IMEPurpose, Id, ImeChangeCause,
    output::{IMEOutput, IMESurroundingTextPayload, MAX_IME_SURROUNDING_TEXT_BYTES},
};
use winit::window::Window;

/// Whether a sink could carry out one requested operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SinkOutcome {
    /// The operation was handed to the platform, or the requested state was already in effect.
    Applied,

    /// The sink dropped the operation because of its local state, for example because the
    /// window's IME is disabled. The caller must not record this write as published.
    Suppressed,
}

/// The protocol-facing operation for one egui pass.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ImeSurroundingAction {
    /// Nothing changed, or there is no Wayland surrounding-text support.
    None,

    /// Publish this bounded snapshot without resetting the composition.
    Set(SurroundingText),

    /// Forget the old owner/context and cancel its composition.
    Clear,

    /// Clear the old owner/context, then publish the new owner's snapshot.
    ///
    /// The clear must be applied before the new text. The old preedit is cancelled, never
    /// inserted into the new editor.
    ClearThenSet(SurroundingText),
}

/// A validated, bounded surrounding-text snapshot.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct SurroundingText {
    text: String,
    cursor: usize,
    anchor: usize,
    cause: ImeChangeCause,
}

impl SurroundingText {
    fn new(text: &str, cursor: usize, anchor: usize) -> Self {
        Self {
            text: text.to_owned(),
            cursor,
            anchor,
            cause: ImeChangeCause::Other,
        }
    }

    fn with_cause(mut self, cause: ImeChangeCause) -> Self {
        self.cause = cause;
        self
    }

    fn matches(&self, text: &str, cursor: usize, anchor: usize) -> bool {
        self.cursor == cursor && self.anchor == anchor && self.text == text
    }
}

impl fmt::Debug for SurroundingText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never put editor or password text into diagnostics.
        f.debug_struct("SurroundingText")
            .field("text", &"<redacted>")
            .field("text_len_bytes", &self.text.len())
            .field("cursor", &self.cursor)
            .field("anchor", &self.anchor)
            .field("cause", &self.cause)
            .finish()
    }
}

/// Identity of one IME-owning editor context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ContextKey {
    owner: Id,
    generation: u64,
    purpose: IMEPurpose,
}

/// What the backend currently has for one context.
#[derive(Clone, Debug, PartialEq, Eq)]
struct BackendContext {
    key: ContextKey,
    /// `None` means the context is known to the backend but has no surrounding text.
    published: Option<SurroundingText>,
}

/// The state to keep only after the planned operation has been applied.
#[derive(Clone, Debug, PartialEq, Eq)]
enum NextState {
    Keep,
    Forget,
    Publish(BackendContext),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Transition {
    action: ImeSurroundingAction,
    next: NextState,
}

/// A sink for the platform operations.
///
/// The production implementation is [`WinitImeSurroundingSink`]; tests use a recording fake that
/// models a locally disabled IME, so a suppressed write is observable.
pub(crate) trait ImeSurroundingSink {
    /// Is this a Wayland window with text-input-v3 surrounding-text support?
    fn supports_ime_surrounding_text(&self) -> bool;

    /// Forget the backend's cached surrounding text and invalidate the old IME context.
    ///
    /// The operation must preserve the window's allowed state and must never enable a window
    /// whose IME is disabled. Forgetting the cache of a disabled window is a harmless no-op and
    /// must still report [`SinkOutcome::Applied`]; [`SinkOutcome::Suppressed`] is reserved for a
    /// sink that could not hand the operation to the platform at all (for example an unsupported
    /// backend).
    fn clear_ime_context(&mut self) -> SinkOutcome;

    /// Publish bounded committed text and UTF-8 byte offsets.
    fn set_ime_surrounding_text(
        &mut self,
        text: &str,
        cursor: usize,
        anchor: usize,
        cause: ImeChangeCause,
    ) -> SinkOutcome;
}

/// Per-window outgoing IME surrounding-text state.
///
/// This is intentionally separate from the integration-level enable/disable latch
/// ([`ImeGate`] and `State::allow_ime`). This structure only remembers what the backend was last
/// told after every platform operation for the pass was applied.
#[derive(Default)]
pub(crate) struct ImeSurroundingSync {
    backend: Option<BackendContext>,
}

impl ImeSurroundingSync {
    /// Forget what the backend has cached.
    ///
    /// Call this after the integration or winit has reset/disabled IME without going through a
    /// planned action, for example when `set_ime_allowed(false)` was called, a text input was
    /// enabled/disabled (`Ime::Enabled` / `Ime::Disabled`), the composition was interrupted, or
    /// an external `ViewportCommand::IMEAllowed` caused a real platform transition. A redundant
    /// command with no platform call keeps the cache so a later owner/generation change can still
    /// plan its clear. The next available descriptor after a reset is sent in full.
    pub(crate) fn forget_backend(&mut self) {
        self.backend = None;
    }

    /// Plan, apply, and then commit one pass.
    ///
    /// The returned action is useful for redacted tracing. The cache is only updated after every
    /// sink operation for this action reported [`SinkOutcome::Applied`].
    pub(crate) fn update(
        &mut self,
        sink: &mut impl ImeSurroundingSink,
        ime: Option<&IMEOutput>,
        ime_allowed: bool,
    ) -> ImeSurroundingAction {
        if !sink.supports_ime_surrounding_text() {
            // X11 and non-Wayland integrations do not receive surrounding text from us. Do not
            // keep a cache that could suppress a later operation on a different backend.
            self.backend = None;
            return ImeSurroundingAction::None;
        }

        let transition = self.plan(ime, ime_allowed);
        let applied = apply_action(sink, &transition.action);

        if applied {
            match transition.next {
                NextState::Keep => {}
                NextState::Forget => self.backend = None,
                NextState::Publish(context) => self.backend = Some(context),
            }
        } else {
            // A locally suppressed write did not reach the compositor. Never remember it as
            // published; the next pass must send the full snapshot again.
            self.backend = None;
            log::trace!("IME surrounding-text operation was suppressed locally; cache forgotten");
        }

        transition.action
    }

    /// Pure transition planner. Does not mutate `self`.
    fn plan(&self, ime: Option<&IMEOutput>, ime_allowed: bool) -> Transition {
        if !ime_allowed {
            // The integration (or the focus latch, or an external IMEAllowed command) disabled
            // IME already. Winit forgets its cached text on disable, so we only drop our own
            // knowledge. Do not plan a clear: issuing disable/enable here would re-enable a
            // window that is deliberately disabled and can fight the focus latch.
            return Transition {
                action: ImeSurroundingAction::None,
                next: NextState::Forget,
            };
        }

        let Some(normalized) = NormalizedIme::new(ime) else {
            return Transition {
                action: if self.backend.is_some() {
                    ImeSurroundingAction::Clear
                } else {
                    ImeSurroundingAction::None
                },
                next: NextState::Forget,
            };
        };

        let key = normalized.key;
        let available = normalized.available();

        if let Some(backend) = &self.backend {
            if backend.key == key {
                return match available {
                    Some((text, cursor, anchor)) => {
                        if backend
                            .published
                            .as_ref()
                            .is_some_and(|old| old.matches(text, cursor, anchor))
                        {
                            Transition {
                                action: ImeSurroundingAction::None,
                                next: NextState::Keep,
                            }
                        } else {
                            let cause = if backend.published.is_some() {
                                normalized.cause
                            } else {
                                ImeChangeCause::Other
                            };
                            let text = SurroundingText::new(text, cursor, anchor).with_cause(cause);
                            Transition {
                                action: ImeSurroundingAction::Set(text.clone()),
                                next: NextState::Publish(BackendContext {
                                    key,
                                    published: Some(text),
                                }),
                            }
                        }
                    }
                    None => {
                        if backend.published.is_none() {
                            Transition {
                                action: ImeSurroundingAction::None,
                                next: NextState::Keep,
                            }
                        } else {
                            Transition {
                                action: ImeSurroundingAction::Clear,
                                next: NextState::Publish(BackendContext {
                                    key,
                                    published: None,
                                }),
                            }
                        }
                    }
                };
            }

            // A new owner, focus generation or purpose must not inherit the old composition.
            return match available {
                Some((text, cursor, anchor)) => {
                    let text = SurroundingText::new(text, cursor, anchor);
                    Transition {
                        action: ImeSurroundingAction::ClearThenSet(text.clone()),
                        next: NextState::Publish(BackendContext {
                            key,
                            published: Some(text),
                        }),
                    }
                }
                None => Transition {
                    action: ImeSurroundingAction::Clear,
                    next: NextState::Publish(BackendContext {
                        key,
                        published: None,
                    }),
                },
            };
        }

        // No known previous context: there is nothing to clear. The integration's initial
        // enable (or winit's reset) already provides the clean boundary.
        match available {
            Some((text, cursor, anchor)) => {
                let text = SurroundingText::new(text, cursor, anchor);
                Transition {
                    action: ImeSurroundingAction::Set(text.clone()),
                    next: NextState::Publish(BackendContext {
                        key,
                        published: Some(text),
                    }),
                }
            }
            None => Transition {
                action: ImeSurroundingAction::None,
                next: NextState::Publish(BackendContext {
                    key,
                    published: None,
                }),
            },
        }
    }
}

/// Apply one planned action through ordered sink operations.
///
/// Returns `false` as soon as an operation is suppressed, so a clear never proceeds to publishing
/// new text when the backend could not forget the old context.
fn apply_action(sink: &mut impl ImeSurroundingSink, action: &ImeSurroundingAction) -> bool {
    match action {
        ImeSurroundingAction::None => true,
        ImeSurroundingAction::Set(text) => {
            sink.set_ime_surrounding_text(&text.text, text.cursor, text.anchor, text.cause)
                == SinkOutcome::Applied
        }
        ImeSurroundingAction::Clear => sink.clear_ime_context() == SinkOutcome::Applied,
        ImeSurroundingAction::ClearThenSet(text) => {
            if sink.clear_ime_context() != SinkOutcome::Applied {
                return false;
            }
            sink.set_ime_surrounding_text(
                &text.text,
                text.cursor,
                text.anchor,
                ImeChangeCause::Other,
            ) == SinkOutcome::Applied
        }
    }
}

/// A validated view of one outgoing descriptor.
struct NormalizedIme<'a> {
    key: ContextKey,
    payload: Option<&'a IMESurroundingTextPayload>,
    cause: ImeChangeCause,
}

impl<'a> NormalizedIme<'a> {
    fn new(ime: Option<&'a IMEOutput>) -> Option<Self> {
        let ime = ime?;
        let key = ContextKey {
            owner: ime.owner,
            generation: ime.context_generation,
            purpose: ime.purpose,
        };

        // Password fields never publish text, even if a caller built an `Available` payload by
        // hand. This is the last line of defense against stale password text in public output.
        let payload = if ime.purpose == IMEPurpose::Password {
            None
        } else {
            ime.surrounding_text
                .available()
                .filter(|payload| valid_payload(payload))
        };

        Some(Self {
            key,
            payload,
            cause: ime.change_cause,
        })
    }

    fn available(&self) -> Option<(&'a str, usize, usize)> {
        self.payload
            .map(|payload| (payload.text.as_str(), payload.cursor, payload.anchor))
    }
}

/// Validate payloads that any caller can construct because the fields are public.
fn valid_payload(payload: &IMESurroundingTextPayload) -> bool {
    let text = payload.text.as_str();
    text.len() <= MAX_IME_SURROUNDING_TEXT_BYTES
        && payload.cursor <= text.len()
        && payload.anchor <= text.len()
        && text.is_char_boundary(payload.cursor)
        && text.is_char_boundary(payload.anchor)
}

/// Durable window-focus and external-command latch for IME enabling.
///
/// egui does not clear widget focus when the *window* loses focus (`WindowEvent::Focused(false)`
/// only reaches [`egui::Event::WindowFocused`]), so a focused `TextEdit` keeps producing
/// `PlatformOutput::ime = Some(..)` on later passes. Without a latch the integration would treat
/// that stale descriptor as a fresh enable on the very next pass and undo the focus-loss clear.
///
/// The latch is event-driven, not polled: it is updated from `WindowEvent::Focused` and from an
/// externally applied `ViewportCommand::IMEAllowed`, and it only answers whether a pass may ask
/// the platform to enable IME. Nothing in this type calls back into the window, so an
/// asynchronous `Ime::Enabled`/`Ime::Disabled` cannot create an event-driven enable/disable loop.
///
/// External enables use one explicit pending policy: `IMEAllowed(true)` clears the external veto,
/// but an enable is only committed by a later platform-output pass that actually carries an IME
/// descriptor. This avoids enabling the window from the stale `ime_requested` bit of a previous
/// pass when the editor has already disappeared. External disables are not deferred: a
/// `IMEAllowed(false)` commits synchronously so that the same frame cannot publish surrounding
/// text first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ImeGate {
    /// Last window focus state delivered by winit.
    ///
    /// Starts `false`: IME is not enabled before winit has reported focus. Winit delivers
    /// `Focused(true)` when the window gains keyboard focus, which also requests a repaint.
    focused: bool,

    /// Set by `ViewportCommand::IMEAllowed(false)`, cleared by `IMEAllowed(true)`.
    external_ime_disabled: bool,

    /// Whether `IMEAllowed(true)` is waiting for a pass with an actual IME descriptor.
    ///
    /// This is deliberately not a second permission source: the descriptor and focus latch still
    /// decide whether IME may be enabled. The flag records that an external enable must not be
    /// satisfied by a stale descriptor, and is consumed by the next pass that has one.
    pending_external_enable: bool,
}

impl ImeGate {
    /// Record `WindowEvent::Focused`.
    pub(crate) fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    /// Clear every per-window latch after `WindowEvent::Destroyed`.
    ///
    /// eframe normally drops the whole `State` with its viewport, but a reused `State` must not
    /// retain focus, external-disable or pending-enable state from the destroyed window.
    pub(crate) fn forget_window(&mut self) {
        *self = Self::default();
    }

    /// Record the value of an externally applied `ViewportCommand::IMEAllowed`.
    ///
    /// `false` is a durable veto for later passes even while egui still reports an IME-owning
    /// widget and takes effect synchronously. `true` clears that veto and records a pending
    /// enable, but cannot enable IME on its own: the next platform-output pass carrying an IME
    /// owner and window focus commits the transition. Neither value overrides the focus latch,
    /// and the egui descriptor still decides whether IME is needed at all.
    pub(crate) fn set_external_ime_allowed(&mut self, allowed: bool) {
        self.external_ime_disabled = !allowed;
        self.pending_external_enable = allowed;
    }

    /// May this pass ask winit to enable IME and publish surrounding text?
    ///
    /// `ime_requested` is `PlatformOutput::ime.is_some()`. A stale descriptor while the window is
    /// unfocused, or after the application disabled IME through `ViewportCommand::IMEAllowed`,
    /// must not enable or republish anything.
    pub(crate) fn permits_ime(&self, ime_requested: bool) -> bool {
        ime_requested && self.focused && !self.external_ime_disabled
    }

    /// Decide the integration-level `allow_ime` value for one pass.
    ///
    /// `current_allow` is the last value passed to `Window::set_ime_allowed`. `changed` means the
    /// integration must commit the transition and invalidate the surrounding-text cache.
    pub(crate) fn transition(
        &mut self,
        ime_requested: bool,
        current_allow: bool,
    ) -> ImeAllowTransition {
        // A real descriptor satisfies a pending external enable, consuming the latch.
        // A `None` pass must not.
        self.pending_external_enable &= !ime_requested;
        let allow = self.permits_ime(ime_requested);
        ImeAllowTransition {
            allow,
            changed: allow != current_allow,
        }
    }

    /// External permission changes cannot create an editor descriptor.
    ///
    /// A disable is applied immediately. An enable is pending and leaves `current_allow`
    /// unchanged; only [`Self::transition`] with a descriptor may raise it. This is the single
    /// ordering policy used by both eframe (which has to pre-apply the command before platform
    /// output) and `egui_glow` (which already runs commands before platform output).
    pub(crate) fn external_transition(&self, current_allow: bool) -> ImeAllowTransition {
        let allow = if self.external_ime_disabled || !self.focused {
            false
        } else {
            current_allow
        };
        ImeAllowTransition {
            allow,
            changed: allow != current_allow,
        }
    }

    /// Whether an external enable is still waiting for a descriptor-carrying pass.
    #[cfg(test)]
    pub(crate) fn has_pending_external_enable(&self) -> bool {
        self.pending_external_enable
    }
}

/// Result of [`ImeGate::transition`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ImeAllowTransition {
    /// Value to keep in `State::allow_ime` after this pass.
    pub(crate) allow: bool,

    /// Whether `set_ime_allowed(allow)` must be committed and the cache invalidated.
    pub(crate) changed: bool,
}

/// True for a Wayland toplevel, including in a winit build that also creates X11 windows.
///
/// The bridge compiles for a free-Unix winit build that may run either backend, so the module
/// cfg alone cannot tell the two windows apart.
pub(crate) fn is_wayland_window(window: &Window) -> bool {
    use winit::platform::wayland::WindowExtWayland as _;
    // `xdg_toplevel` is `None` for an X11 window in the same winit build.
    window.xdg_toplevel().is_some()
}

/// Wayland window adapter for [`ImeSurroundingSink`].
///
/// `ime_allowed` is a read-only snapshot of the integration-level `State::allow_ime`, the value
/// last committed to `Window::set_ime_allowed`. The dedicated clear preserves that state, so the
/// snapshot never changes during a pass; it only decides whether publishing text would actually
/// reach the compositor.
pub(crate) struct WinitImeSurroundingSink<'a> {
    window: &'a Window,
    ime_allowed: bool,
}

impl<'a> WinitImeSurroundingSink<'a> {
    pub(crate) fn new(window: &'a Window, ime_allowed: bool) -> Self {
        Self {
            window,
            ime_allowed,
        }
    }
}

impl ImeSurroundingSink for WinitImeSurroundingSink<'_> {
    fn supports_ime_surrounding_text(&self) -> bool {
        // This proves the window is Wayland; it does not prove that the compositor implements
        // `zwp_text_input_v3` or that a text input is currently entered. Winit keeps the snapshot
        // for the next entering text input, which is why publishing is still correct there. Nor
        // does it prove which winit API surface is available: the clear below needs the E21b
        // `clear_ime_surrounding_text`, which the pinned revision does not have.
        is_wayland_window(self.window)
    }

    fn clear_ime_context(&mut self) -> SinkOutcome {
        use winit::platform::wayland::WindowExtWayland as _;
        // One dedicated commit: winit forgets its cached text, keeps `ime_allowed` as it is,
        // never enables a disabled window, and resets the protocol context only when text was
        // published or a composition was active. It is a no-op for a disabled window apart from
        // forgetting the cache, which is why no local allowed check is needed here.
        self.window.clear_ime_surrounding_text();
        SinkOutcome::Applied
    }

    fn set_ime_surrounding_text(
        &mut self,
        text: &str,
        cursor: usize,
        anchor: usize,
        cause: ImeChangeCause,
    ) -> SinkOutcome {
        if !self.ime_allowed {
            // winit would cache the snapshot without committing it while disabled. Report the
            // suppression so the planner forgets instead of assuming the text is published.
            return SinkOutcome::Suppressed;
        }

        use winit::platform::wayland::WindowExtWayland as _;
        let cause = match cause {
            ImeChangeCause::InputMethod => {
                winit::platform::wayland::ImeTextChangeCause::InputMethod
            }
            ImeChangeCause::Other => winit::platform::wayland::ImeTextChangeCause::Other,
        };
        self.window
            .set_ime_surrounding_text_with_change_cause(text, cursor, anchor, cause);
        SinkOutcome::Applied
    }
}

// Notes on the E21b clear:
//
// Do not substitute `Window::reset_ime()`: it re-applies the cached surrounding state on enable,
// so it is not a clear. Do not reconstruct the clear from `set_ime_allowed(false)` plus
// `set_ime_allowed(true)`: that re-enables a window the integration disabled and produces
// `Ime::Disabled`/`Ime::Enabled` events the caller never asked for.

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{
        IMEPurpose, Id, Rect,
        output::{IMESurroundingText, IMESurroundingTextPayload, MAX_IME_SURROUNDING_TEXT_BYTES},
    };

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Call {
        /// `State` committed `window.set_ime_allowed(false)` directly (focus loss or a normal
        /// `PlatformOutput::ime = None` pass).
        WindowDisable,

        /// `State` committed `window.set_ime_allowed(true)` directly (normal pass or focus
        /// regain).
        WindowEnable,

        /// One dedicated `clear_ime_surrounding_text` commit through the sink. It preserves the
        /// window's allowed state and never emits `WindowEnable`/`WindowDisable`.
        SinkClear,

        Set(String, usize, usize),

        /// The sink refused to publish because its window was disabled.
        SuppressedSet(String, usize, usize),
    }

    #[derive(Debug)]
    struct FakeSink {
        calls: Vec<Call>,
        causes: Vec<ImeChangeCause>,
        supported: bool,
        /// Models winit's `ime_allowed` plus its cached surrounding-text snapshot.
        allowed: bool,
        published: Option<(String, usize, usize)>,
    }

    impl FakeSink {
        fn new() -> Self {
            Self {
                calls: Vec::new(),
                causes: Vec::new(),
                supported: true,
                allowed: true,
                published: None,
            }
        }

        fn unsupported() -> Self {
            Self {
                supported: false,
                ..Self::new()
            }
        }
    }

    impl ImeSurroundingSink for FakeSink {
        fn supports_ime_surrounding_text(&self) -> bool {
            self.supported
        }

        fn clear_ime_context(&mut self) -> SinkOutcome {
            if !self.supported {
                return SinkOutcome::Suppressed;
            }

            // The dedicated clear forgets the cached text and preserves the allowed state. It is
            // applied even for a disabled window: forgetting there is a no-op, never an enable.
            self.published = None;
            self.calls.push(Call::SinkClear);
            SinkOutcome::Applied
        }

        fn set_ime_surrounding_text(
            &mut self,
            text: &str,
            cursor: usize,
            anchor: usize,
            cause: ImeChangeCause,
        ) -> SinkOutcome {
            if !self.supported || !self.allowed {
                self.calls
                    .push(Call::SuppressedSet(text.to_owned(), cursor, anchor));
                return SinkOutcome::Suppressed;
            }

            self.published = Some((text.to_owned(), cursor, anchor));
            self.causes.push(cause);
            self.calls.push(Call::Set(text.to_owned(), cursor, anchor));
            SinkOutcome::Applied
        }
    }

    #[test]
    fn change_cause_tracks_the_snapshot_but_does_not_trigger_a_commit() {
        let mut sync = ImeSurroundingSync::default();
        let mut sink = FakeSink::new();
        let mut first = output(1, 1, IMEPurpose::Normal, "a");
        first.change_cause = ImeChangeCause::InputMethod;
        sync.update(&mut sink, Some(&first), true);
        assert_eq!(
            sink.causes,
            [ImeChangeCause::Other],
            "initial context is application-origin"
        );

        let mut edited = output(1, 1, IMEPurpose::Normal, "ab");
        edited.change_cause = ImeChangeCause::InputMethod;
        sync.update(&mut sink, Some(&edited), true);
        assert_eq!(sink.causes.last(), Some(&ImeChangeCause::InputMethod));
        let calls = sink.calls.len();
        edited.change_cause = ImeChangeCause::Other;
        assert_eq!(
            sync.update(&mut sink, Some(&edited), true),
            ImeSurroundingAction::None
        );
        assert_eq!(
            sink.calls.len(),
            calls,
            "cause alone does not republish identical text"
        );
    }

    #[test]
    fn change_cause_is_other_after_owner_generation_or_availability_changes() {
        let mut sync = ImeSurroundingSync::default();
        let mut sink = FakeSink::new();
        sync.update(
            &mut sink,
            Some(&output(1, 1, IMEPurpose::Normal, "a")),
            true,
        );
        for (owner, generation) in [(2, 1), (2, 2)] {
            let mut next = output(owner, generation, IMEPurpose::Normal, "b");
            next.change_cause = ImeChangeCause::InputMethod;
            assert!(matches!(
                sync.update(&mut sink, Some(&next), true),
                ImeSurroundingAction::ClearThenSet(_)
            ));
            assert_eq!(sink.causes.last(), Some(&ImeChangeCause::Other));
        }
        sync.update(
            &mut sink,
            Some(&output_unavailable(2, 2, IMEPurpose::Normal)),
            true,
        );
        let mut next = output(2, 2, IMEPurpose::Normal, "c");
        next.change_cause = ImeChangeCause::InputMethod;
        sync.update(&mut sink, Some(&next), true);
        assert_eq!(sink.causes.last(), Some(&ImeChangeCause::Other));
    }

    #[test]
    fn suppressed_change_does_not_cache_cause_or_text_as_published() {
        let mut sync = ImeSurroundingSync::default();
        let mut sink = FakeSink::new();
        sync.update(
            &mut sink,
            Some(&output(1, 1, IMEPurpose::Normal, "a")),
            true,
        );
        sink.allowed = false;
        let mut edited = output(1, 1, IMEPurpose::Normal, "ab");
        edited.change_cause = ImeChangeCause::InputMethod;
        sync.update(&mut sink, Some(&edited), true);
        assert_eq!(sink.causes, [ImeChangeCause::Other]);
        sink.allowed = true;
        assert!(matches!(
            sync.update(&mut sink, Some(&edited), true),
            ImeSurroundingAction::Set(_)
        ));
        assert_eq!(sink.published, Some(("ab".into(), 2, 2)));
        assert_eq!(
            sink.causes.last(),
            Some(&ImeChangeCause::Other),
            "restored context is a fresh boundary"
        );
    }

    fn output(owner: u64, generation: u64, purpose: IMEPurpose, text: &str) -> IMEOutput {
        let mut ime = IMEOutput::new(
            Id::unique(owner),
            generation,
            purpose,
            Rect::ZERO,
            Rect::ZERO,
        );
        ime.surrounding_text = if purpose == IMEPurpose::Password {
            // Deliberately provide text to prove the consumer rejects it for password fields.
            IMESurroundingText::Available(IMESurroundingTextPayload {
                text: text.to_owned(),
                cursor: text.len(),
                anchor: text.len(),
            })
        } else {
            IMESurroundingText::from_text(text, text.len(), text.len(), None)
        };
        ime
    }

    fn output_with_payload(
        owner: u64,
        generation: u64,
        purpose: IMEPurpose,
        payload: IMESurroundingTextPayload,
    ) -> IMEOutput {
        let mut ime = IMEOutput::new(
            Id::unique(owner),
            generation,
            purpose,
            Rect::ZERO,
            Rect::ZERO,
        );
        ime.surrounding_text = IMESurroundingText::Available(payload);
        ime
    }

    fn output_unavailable(owner: u64, generation: u64, purpose: IMEPurpose) -> IMEOutput {
        let mut ime = IMEOutput::new(
            Id::unique(owner),
            generation,
            purpose,
            Rect::ZERO,
            Rect::ZERO,
        );
        ime.surrounding_text = IMESurroundingText::Unavailable;
        ime
    }

    fn update(
        sync: &mut ImeSurroundingSync,
        sink: &mut FakeSink,
        ime: Option<&IMEOutput>,
    ) -> ImeSurroundingAction {
        sync.update(sink, ime, true)
    }

    /// Models the production sequence in `State::handle_platform_output_inner` and
    /// `WindowEvent::Focused` / `ViewportCommand::IMEAllowed` using the real [`ImeGate`],
    /// [`ImeSurroundingSync`] and sink trait.
    struct PassHarness {
        gate: ImeGate,
        sync: ImeSurroundingSync,
        sink: FakeSink,
        allow_ime: bool,
        old_ime_purpose: IMEPurpose,
    }

    impl PassHarness {
        fn new() -> Self {
            let mut sink = FakeSink::new();
            sink.allowed = false;
            Self {
                gate: ImeGate::default(),
                sync: ImeSurroundingSync::default(),
                sink,
                allow_ime: false,
                old_ime_purpose: IMEPurpose::Normal,
            }
        }

        fn pass(&mut self, ime: Option<&IMEOutput>) -> ImeSurroundingAction {
            let transition = self.gate.transition(ime.is_some(), self.allow_ime);
            if transition.changed {
                self.allow_ime = transition.allow;
                self.sink.allowed = transition.allow;
                self.sink.calls.push(if transition.allow {
                    Call::WindowEnable
                } else {
                    Call::WindowDisable
                });
                // winit forgets its cached text at every enable/disable boundary.
                self.sink.published = None;
                self.sync.forget_backend();
            }

            if let Some(ime) = ime
                && ime.purpose != self.old_ime_purpose
            {
                self.old_ime_purpose = ime.purpose;
            }

            self.sync.update(&mut self.sink, ime, self.allow_ime)
        }

        /// `WindowEvent::Focused(false)` while a text widget still owns IME focus.
        fn window_focus_lost(&mut self) {
            self.gate.set_focused(false);
            // `clear_ime_surrounding_on_focus_loss`
            if self.allow_ime {
                self.allow_ime = false;
                self.sink.allowed = false;
                self.sink.published = None;
                self.sink.calls.push(Call::WindowDisable);
            }
            self.sync.forget_backend();
        }

        /// `WindowEvent::Focused(true)`; the enable happens on the next platform-output pass.
        fn window_focus_gained(&mut self) {
            self.gate.set_focused(true);
        }

        /// `State::handle_external_ime_allowed`, called by `process_viewport_commands` for
        /// `ViewportCommand::IMEAllowed`. A disable commits here; an enable stays pending until
        /// `pass` sees a descriptor.
        fn external_ime_allowed(&mut self, allowed: bool) {
            self.gate.set_external_ime_allowed(allowed);
            let transition = self.gate.external_transition(self.allow_ime);
            self.allow_ime = transition.allow;
            if transition.changed {
                self.sink.allowed = transition.allow;
                self.sink.published = None;
                self.sink.calls.push(if transition.allow {
                    Call::WindowEnable
                } else {
                    Call::WindowDisable
                });
                // Only a real platform transition resets winit's cached text.
                self.sync.forget_backend();
            }
        }

        /// `process_viewport_commands` for `ViewportCommand::IMEPurpose`. An unchanged purpose is
        /// a no-op; a changed purpose is a context boundary and invalidates the bridge cache.
        fn external_ime_purpose(&mut self, purpose: IMEPurpose) {
            if purpose != self.old_ime_purpose {
                self.old_ime_purpose = purpose;
                self.sync.forget_backend();
            }
        }

        /// eframe ordering: an `IMEAllowed` command for this viewport is pre-applied before the
        /// platform-output pass, then the pass publishes surrounding text.
        fn eframe_pass(
            &mut self,
            ime: Option<&IMEOutput>,
            external_allowed: Option<bool>,
        ) -> ImeSurroundingAction {
            if let Some(allowed) = external_allowed {
                self.external_ime_allowed(allowed);
            }
            self.pass(ime)
        }

        /// `WindowEvent::Destroyed`: clear the per-window latches without touching a dead window.
        fn window_destroyed(&mut self) {
            self.gate.forget_window();
            self.allow_ime = false;
            self.old_ime_purpose = IMEPurpose::Normal;
            self.sink.allowed = false;
            self.sink.published = None;
            self.sync.forget_backend();
        }

        /// Model winit's asynchronous `Ime::Disabled`, which only invalidates the bridge cache;
        /// the integration must not toggle IME from it.
        fn ime_disabled_event(&mut self) {
            self.sync.forget_backend();
        }
    }

    #[test]
    fn first_available_payload_is_sent_without_a_clear() {
        let mut sync = ImeSurroundingSync::default();
        let mut sink = FakeSink::new();
        let ime = output(1, 1, IMEPurpose::Normal, "hello");

        update(&mut sync, &mut sink, Some(&ime));

        assert_eq!(sink.calls, vec![Call::Set("hello".to_owned(), 5, 5)]);
    }

    #[test]
    fn unchanged_repeated_passes_do_not_touch_the_window() {
        let mut sync = ImeSurroundingSync::default();
        let mut sink = FakeSink::new();
        let ime = output(1, 1, IMEPurpose::Normal, "hello");

        update(&mut sync, &mut sink, Some(&ime));
        for _ in 0..5 {
            update(&mut sync, &mut sink, Some(&ime));
        }

        assert_eq!(sink.calls, vec![Call::Set("hello".to_owned(), 5, 5)]);
    }

    #[test]
    fn ordinary_edits_do_not_clear_but_purpose_changes_do() {
        let mut sync = ImeSurroundingSync::default();
        let mut sink = FakeSink::new();

        let ime = output(1, 1, IMEPurpose::Normal, "hello");
        update(&mut sync, &mut sink, Some(&ime));
        let ime = output(1, 1, IMEPurpose::Normal, "hello!");
        update(&mut sync, &mut sink, Some(&ime));
        let ime = output(1, 1, IMEPurpose::Terminal, "hello!");
        update(&mut sync, &mut sink, Some(&ime));

        assert_eq!(
            sink.calls,
            vec![
                Call::Set("hello".to_owned(), 5, 5),
                Call::Set("hello!".to_owned(), 6, 6),
                // A purpose change is a context transition: one dedicated clear, then the new
                // snapshot.
                Call::SinkClear,
                Call::Set("hello!".to_owned(), 6, 6),
            ]
        );
    }

    #[test]
    fn cursor_and_selection_changes_are_published() {
        let mut sync = ImeSurroundingSync::default();
        let mut sink = FakeSink::new();
        let text = "abcd";

        let ime = output(1, 1, IMEPurpose::Normal, text);
        update(&mut sync, &mut sink, Some(&ime));
        let mut ime = output(1, 1, IMEPurpose::Normal, text);
        ime.surrounding_text = IMESurroundingText::from_text(text, 1, 3, None);
        update(&mut sync, &mut sink, Some(&ime));

        assert_eq!(
            sink.calls,
            vec![
                Call::Set("abcd".to_owned(), 4, 4),
                Call::Set("abcd".to_owned(), 1, 3),
            ]
        );
    }

    #[test]
    fn available_empty_is_distinct_from_unavailable() {
        let mut sync = ImeSurroundingSync::default();
        let mut sink = FakeSink::new();

        let empty = output(1, 1, IMEPurpose::Normal, "");
        update(&mut sync, &mut sink, Some(&empty));
        assert_eq!(sink.calls, vec![Call::Set(String::new(), 0, 0)]);

        let unavailable = output_unavailable(1, 1, IMEPurpose::Normal);
        update(&mut sync, &mut sink, Some(&unavailable));
        update(&mut sync, &mut sink, Some(&unavailable));
        assert_eq!(
            sink.calls,
            vec![Call::Set(String::new(), 0, 0), Call::SinkClear],
            "unavailable clears once; repeated unavailable passes are quiet"
        );
    }

    #[test]
    fn unavailable_then_available_publishes_the_full_payload() {
        let mut sync = ImeSurroundingSync::default();
        let mut sink = FakeSink::new();

        let unavailable = output_unavailable(1, 1, IMEPurpose::Normal);
        update(&mut sync, &mut sink, Some(&unavailable));
        assert!(
            sink.calls.is_empty(),
            "an unavailable first context has nothing to clear"
        );

        let available = output(1, 1, IMEPurpose::Normal, "hello");
        update(&mut sync, &mut sink, Some(&available));
        assert_eq!(sink.calls, vec![Call::Set("hello".to_owned(), 5, 5)]);
    }

    #[test]
    fn owner_and_generation_transitions_clear_before_publishing() {
        let mut sync = ImeSurroundingSync::default();
        let mut sink = FakeSink::new();

        let first = output(1, 1, IMEPurpose::Normal, "old editor");
        update(&mut sync, &mut sink, Some(&first));

        let second = output(2, 1, IMEPurpose::Normal, "new editor");
        update(&mut sync, &mut sink, Some(&second));

        let third = output(2, 2, IMEPurpose::Normal, "refocused");
        update(&mut sync, &mut sink, Some(&third));

        assert_eq!(
            sink.calls,
            vec![
                Call::Set("old editor".to_owned(), 10, 10),
                Call::SinkClear,
                Call::Set("new editor".to_owned(), 10, 10),
                Call::SinkClear,
                Call::Set("refocused".to_owned(), 9, 9),
            ]
        );
    }

    #[test]
    fn owner_change_to_unavailable_clears_without_leaking_old_text() {
        let mut sync = ImeSurroundingSync::default();
        let mut sink = FakeSink::new();

        let old = output(1, 1, IMEPurpose::Normal, "secret old text");
        update(&mut sync, &mut sink, Some(&old));

        let password = output_unavailable(2, 1, IMEPurpose::Password);
        update(&mut sync, &mut sink, Some(&password));

        assert_eq!(
            sink.calls,
            vec![
                Call::Set("secret old text".to_owned(), 15, 15),
                Call::SinkClear,
            ],
            "the old text must not be republished for the new owner"
        );
    }

    #[test]
    fn password_purpose_rejects_publicly_constructed_text() {
        let mut sync = ImeSurroundingSync::default();
        let mut sink = FakeSink::new();

        let normal = output(1, 1, IMEPurpose::Normal, "visible");
        update(&mut sync, &mut sink, Some(&normal));

        let password = output(1, 1, IMEPurpose::Password, "hunter2");
        update(&mut sync, &mut sink, Some(&password));

        assert_eq!(
            sink.calls,
            vec![Call::Set("visible".to_owned(), 7, 7), Call::SinkClear],
            "password text is never sent"
        );
    }

    #[test]
    fn invalid_public_payloads_never_reach_the_sink() {
        let cases = [
            IMESurroundingTextPayload {
                text: "abc".to_owned(),
                cursor: 4,
                anchor: 4,
            },
            IMESurroundingTextPayload {
                text: "abc".to_owned(),
                cursor: 0,
                anchor: 4,
            },
            IMESurroundingTextPayload {
                text: "é".to_owned(),
                cursor: 1,
                anchor: 1,
            },
            IMESurroundingTextPayload {
                text: "x".repeat(MAX_IME_SURROUNDING_TEXT_BYTES + 1),
                cursor: MAX_IME_SURROUNDING_TEXT_BYTES + 1,
                anchor: MAX_IME_SURROUNDING_TEXT_BYTES + 1,
            },
        ];

        for payload in cases {
            let mut sync = ImeSurroundingSync::default();
            let mut sink = FakeSink::new();

            let valid = output(1, 1, IMEPurpose::Normal, "old text");
            update(&mut sync, &mut sink, Some(&valid));

            let invalid = output_with_payload(1, 1, IMEPurpose::Normal, payload);
            update(&mut sync, &mut sink, Some(&invalid));

            assert_eq!(
                sink.calls,
                vec![Call::Set("old text".to_owned(), 8, 8), Call::SinkClear],
                "invalid payload must clear old text and send nothing"
            );
        }
    }

    #[test]
    fn maximum_sized_payload_is_accepted() {
        let mut sync = ImeSurroundingSync::default();
        let mut sink = FakeSink::new();
        let text = "a".repeat(MAX_IME_SURROUNDING_TEXT_BYTES);
        let ime = output(1, 1, IMEPurpose::Normal, &text);

        update(&mut sync, &mut sink, Some(&ime));

        assert_eq!(
            sink.calls,
            vec![Call::Set(
                text,
                MAX_IME_SURROUNDING_TEXT_BYTES,
                MAX_IME_SURROUNDING_TEXT_BYTES
            )]
        );
    }

    #[test]
    fn ime_none_clears_and_forgets_the_cached_text() {
        let mut sync = ImeSurroundingSync::default();
        let mut sink = FakeSink::new();

        let ime = output(1, 1, IMEPurpose::Normal, "hello");
        update(&mut sync, &mut sink, Some(&ime));
        update(&mut sync, &mut sink, None);
        update(&mut sync, &mut sink, None);

        assert_eq!(
            sink.calls,
            vec![Call::Set("hello".to_owned(), 5, 5), Call::SinkClear,]
        );
    }

    #[test]
    fn ime_none_while_already_disabled_does_not_reenable_the_window() {
        let mut sync = ImeSurroundingSync::default();
        let mut sink = FakeSink::new();
        let ime = output(1, 1, IMEPurpose::Normal, "hello");

        update(&mut sync, &mut sink, Some(&ime));
        // The integration disabled IME (focus loss / external command) and winit forgot its text.
        sink.allowed = false;
        sink.published = None;
        sync.forget_backend();

        let action = sync.update(&mut sink, None, false);
        assert_eq!(action, ImeSurroundingAction::None);
        assert_eq!(
            sink.calls,
            vec![Call::Set("hello".to_owned(), 5, 5)],
            "an already-disabled window must not receive a clear or an enable"
        );
    }

    #[test]
    fn focus_loss_and_interrupt_forget_force_a_full_resend() {
        let mut sync = ImeSurroundingSync::default();
        let mut sink = FakeSink::new();

        let ime = output(1, 1, IMEPurpose::Normal, "hello");
        update(&mut sync, &mut sink, Some(&ime));
        sync.forget_backend();
        update(&mut sync, &mut sink, Some(&ime));

        assert_eq!(
            sink.calls,
            vec![
                Call::Set("hello".to_owned(), 5, 5),
                Call::Set("hello".to_owned(), 5, 5),
            ],
            "a reset cache must not suppress the next full payload"
        );
    }

    #[test]
    fn unsupported_backend_never_caches_or_touches_text() {
        let mut sync = ImeSurroundingSync::default();
        let mut unsupported = FakeSink::unsupported();
        let ime = output(1, 1, IMEPurpose::Normal, "hello");

        update(&mut sync, &mut unsupported, Some(&ime));
        assert!(
            unsupported.calls.is_empty(),
            "unsupported backend must not touch the sink"
        );

        let mut supported = FakeSink::new();
        update(&mut sync, &mut supported, Some(&ime));
        assert_eq!(supported.calls, vec![Call::Set("hello".to_owned(), 5, 5)]);
    }

    #[test]
    fn disabled_ime_does_not_publish_text() {
        let mut sync = ImeSurroundingSync::default();
        let mut sink = FakeSink::new();
        let ime = output(1, 1, IMEPurpose::Normal, "hello");

        let action = sync.update(&mut sink, Some(&ime), false);

        assert_eq!(action, ImeSurroundingAction::None);
        assert!(
            sink.calls.is_empty(),
            "a disabled pass must not publish text"
        );
    }

    #[test]
    fn suppressed_set_is_not_cached_and_is_resent_when_allowed() {
        let mut harness = PassHarness::new();
        harness.window_focus_gained();
        let ime = output(1, 1, IMEPurpose::Normal, "hello");
        harness.pass(Some(&ime));

        // Model a local disable the bridge has not observed yet: winit would cache the next
        // snapshot silently instead of committing it.
        harness.sink.allowed = false;
        harness.sink.published = None;

        let edited = output(1, 1, IMEPurpose::Normal, "hello!");
        harness.pass(Some(&edited));

        harness.sink.allowed = true;
        harness.pass(Some(&edited));

        assert_eq!(
            harness.sink.calls,
            vec![
                Call::WindowEnable,
                Call::Set("hello".to_owned(), 5, 5),
                Call::SuppressedSet("hello!".to_owned(), 6, 6),
                // The suppressed write was not cached, so the full snapshot is sent again.
                Call::Set("hello!".to_owned(), 6, 6),
            ]
        );
    }

    #[test]
    fn clear_on_a_locally_disabled_window_never_enables_it() {
        let mut sync = ImeSurroundingSync::default();
        let mut sink = FakeSink::new();
        let first = output(1, 1, IMEPurpose::Normal, "old");
        update(&mut sync, &mut sink, Some(&first));

        // The window is disabled locally, but the planner still believes IME is allowed.
        sink.allowed = false;
        sink.published = None;

        let second = output(2, 1, IMEPurpose::Normal, "new");
        sync.update(&mut sink, Some(&second), true);

        assert_eq!(
            sink.calls,
            vec![
                Call::Set("old".to_owned(), 3, 3),
                // The clear forgets winit's cache and preserves the disabled state...
                Call::SinkClear,
                // ...but the new snapshot is refused while disabled.
                Call::SuppressedSet("new".to_owned(), 3, 3),
            ],
            "a clear must not enable a window the integration disabled"
        );
        assert!(
            !sink.allowed,
            "the dedicated clear must preserve the allowed state"
        );

        sink.allowed = true;
        sync.update(&mut sink, Some(&second), true);
        assert_eq!(
            sink.calls,
            vec![
                Call::Set("old".to_owned(), 3, 3),
                Call::SinkClear,
                Call::SuppressedSet("new".to_owned(), 3, 3),
                // The suppressed write was not cached, so this is a plain full publish: the
                // backend already forgot the old context and the window is allowed again.
                Call::Set("new".to_owned(), 3, 3),
            ]
        );
    }

    #[test]
    fn context_clear_preserves_allowed_state_and_emits_no_toggle() {
        let mut sync = ImeSurroundingSync::default();
        let mut sink = FakeSink::new();

        let first = output(1, 1, IMEPurpose::Normal, "old editor");
        update(&mut sync, &mut sink, Some(&first));
        let second = output(2, 1, IMEPurpose::Normal, "new editor");
        update(&mut sync, &mut sink, Some(&second));

        assert_eq!(
            sink.calls,
            vec![
                Call::Set("old editor".to_owned(), 10, 10),
                Call::SinkClear,
                Call::Set("new editor".to_owned(), 10, 10),
            ],
            "a context clear is one dedicated commit, not a disable/enable pair"
        );
        assert!(sink.allowed, "the clear must not disable the window");
        assert_eq!(sink.published, Some(("new editor".to_owned(), 10, 10)));
    }

    #[test]
    fn every_context_boundary_uses_one_clear_and_never_publishes_old_text() {
        // Password and unavailable payloads have no text of their own; the previous owner's text
        // must be forgotten with one dedicated clear, without an enable/disable cycle.
        let cases = [
            output_unavailable(2, 1, IMEPurpose::Normal),
            output(2, 1, IMEPurpose::Password, "hunter2"),
        ];

        for boundary in cases {
            let mut sync = ImeSurroundingSync::default();
            let mut sink = FakeSink::new();

            let old = output(1, 1, IMEPurpose::Normal, "secret old text");
            update(&mut sync, &mut sink, Some(&old));
            update(&mut sync, &mut sink, Some(&boundary));
            update(&mut sync, &mut sink, Some(&boundary));

            assert_eq!(
                sink.calls,
                vec![
                    Call::Set("secret old text".to_owned(), 15, 15),
                    Call::SinkClear
                ],
                "one clear for the boundary, then quiet passes"
            );
            assert!(sink.allowed);
        }
    }

    #[test]
    fn focus_loss_latch_blocks_stale_ime_output_until_focus_regain() {
        let mut harness = PassHarness::new();
        harness.window_focus_gained();
        let ime = output(1, 1, IMEPurpose::Normal, "hello");

        harness.pass(Some(&ime));
        assert_eq!(
            harness.sink.calls,
            vec![Call::WindowEnable, Call::Set("hello".to_owned(), 5, 5),]
        );

        harness.window_focus_lost();
        // egui still reports the same IME-owning widget; these passes must stay silent.
        harness.pass(Some(&ime));
        harness.pass(Some(&ime));
        assert_eq!(
            harness.sink.calls,
            vec![
                Call::WindowEnable,
                Call::Set("hello".to_owned(), 5, 5),
                Call::WindowDisable,
            ],
            "an unfocused window must not be re-enabled or republished"
        );
        assert!(
            !harness.sink.allowed,
            "the window must stay disabled while unfocused"
        );

        harness.window_focus_gained();
        harness.pass(Some(&ime));
        assert_eq!(
            harness.sink.calls,
            vec![
                Call::WindowEnable,
                Call::Set("hello".to_owned(), 5, 5),
                Call::WindowDisable,
                // Focus regain: one clean enable plus the full payload.
                Call::WindowEnable,
                Call::Set("hello".to_owned(), 5, 5),
            ]
        );
        assert_eq!(harness.sink.published, Some(("hello".to_owned(), 5, 5)));
    }

    #[test]
    fn external_permission_without_an_editor_does_not_enable_ime() {
        let mut harness = PassHarness::new();
        harness.window_focus_gained();
        harness.external_ime_allowed(true);
        assert!(harness.sink.calls.is_empty());
        assert!(!harness.allow_ime);
        let ime = output(1, 1, IMEPurpose::Normal, "hello");
        harness.pass(Some(&ime));
        assert!(harness.allow_ime);
        harness.pass(None);
        let calls = harness.sink.calls.len();
        harness.external_ime_allowed(true);
        assert!(!harness.allow_ime);
        assert_eq!(harness.sink.calls.len(), calls);
    }

    #[test]
    fn external_purpose_boundary_requires_full_other_resend() {
        let mut harness = PassHarness::new();
        harness.window_focus_gained();
        let ime = output(1, 1, IMEPurpose::Normal, "hello");
        harness.pass(Some(&ime));
        // The external IMEPurpose command invalidates the same cache, even if the
        // next editor descriptor has not changed yet.
        harness.sync.forget_backend();
        harness.pass(Some(&ime));
        assert_eq!(
            harness
                .sink
                .calls
                .iter()
                .filter(|call| matches!(call, Call::Set(..)))
                .count(),
            2
        );
        assert_eq!(
            harness.sink.causes.last(),
            Some(&egui::output::ImeChangeCause::Other)
        );
    }

    #[test]
    fn external_disable_then_enable_resends_unchanged_text() {
        let mut harness = PassHarness::new();
        harness.window_focus_gained();
        let ime = output(1, 1, IMEPurpose::Normal, "hello");
        harness.pass(Some(&ime));

        harness.external_ime_allowed(false);
        // The synchronous command disables the window; the asynchronous `Ime::Disabled` from
        // winit must not toggle IME again.
        harness.ime_disabled_event();
        harness.pass(Some(&ime));
        assert_eq!(
            harness.sink.calls,
            vec![
                Call::WindowEnable,
                Call::Set("hello".to_owned(), 5, 5),
                Call::WindowDisable,
            ],
            "IMEAllowed(false) must disable and suppress the stale descriptor"
        );
        assert!(
            !harness.sink.allowed,
            "external disable must leave the window disabled"
        );

        harness.external_ime_allowed(true);
        harness.pass(Some(&ime));
        assert_eq!(
            harness.sink.calls,
            vec![
                Call::WindowEnable,
                Call::Set("hello".to_owned(), 5, 5),
                Call::WindowDisable,
                // The external command re-enabled winit synchronously; unchanged editor text
                // still gets a full surrounding-text resend because the cache was invalidated.
                Call::WindowEnable,
                Call::Set("hello".to_owned(), 5, 5),
            ]
        );
    }

    #[test]
    fn external_enable_waits_for_a_descriptor_pass() {
        let mut harness = PassHarness::new();
        harness.window_focus_gained();
        let ime = output(1, 1, IMEPurpose::Normal, "hello");
        harness.pass(Some(&ime));
        assert!(harness.allow_ime);

        harness.external_ime_allowed(false);
        harness.external_ime_allowed(true);
        assert!(!harness.allow_ime, "the enable is pending, not synchronous");
        assert!(harness.gate.has_pending_external_enable());

        // The next pass has no editor descriptor. It must not satisfy the pending enable from
        // the stale `ime_requested` bit of the previous pass.
        harness.pass(None);
        assert!(!harness.allow_ime);
        assert_eq!(
            harness.sink.calls,
            vec![
                Call::WindowEnable,
                Call::Set("hello".to_owned(), 5, 5),
                Call::WindowDisable,
            ],
            "the stale descriptor must not cause a second WindowEnable"
        );

        // A real descriptor commits exactly one enable and republishes the full snapshot.
        harness.pass(Some(&ime));
        assert!(harness.allow_ime);
        assert_eq!(
            harness.sink.calls,
            vec![
                Call::WindowEnable,
                Call::Set("hello".to_owned(), 5, 5),
                Call::WindowDisable,
                Call::WindowEnable,
                Call::Set("hello".to_owned(), 5, 5),
            ]
        );
    }

    #[test]
    fn eframe_ordering_disables_before_publishing_surrounding_text() {
        let mut harness = PassHarness::new();
        harness.window_focus_gained();
        let old = output(1, 1, IMEPurpose::Normal, "hello");
        harness.pass(Some(&old));
        assert!(harness.allow_ime);

        // eframe pre-applies the viewport's IMEAllowed(false) before handling platform output.
        // The changed descriptor in the same frame must not reach the sink.
        let changed = output(1, 1, IMEPurpose::Normal, "hello!");
        harness.eframe_pass(Some(&changed), Some(false));

        assert_eq!(
            harness.sink.calls,
            vec![
                Call::WindowEnable,
                Call::Set("hello".to_owned(), 5, 5),
                Call::WindowDisable,
            ],
            "WindowDisable must precede any surrounding-text Set for this pass"
        );
        assert!(!harness.allow_ime);
        assert_eq!(harness.sink.published, None);
    }

    #[test]
    fn redundant_external_enable_preserves_cache_for_owner_change() {
        let mut harness = PassHarness::new();
        harness.window_focus_gained();
        let first = output(1, 1, IMEPurpose::Normal, "same text");
        harness.pass(Some(&first));

        // The window is already enabled, so this command makes no platform transition. It must
        // not erase the bridge's knowledge of the old owner/context.
        harness.external_ime_allowed(true);
        assert!(harness.allow_ime);

        let second = output(2, 2, IMEPurpose::Normal, "same text");
        assert!(matches!(
            harness.pass(Some(&second)),
            ImeSurroundingAction::ClearThenSet(_)
        ));
        assert_eq!(
            harness.sink.calls,
            vec![
                Call::WindowEnable,
                Call::Set("same text".to_owned(), 9, 9),
                Call::SinkClear,
                Call::Set("same text".to_owned(), 9, 9),
            ],
            "a redundant external enable must not turn the new owner into an initial context"
        );
    }

    #[test]
    fn unchanged_external_purpose_preserves_the_bridge_cache() {
        let mut harness = PassHarness::new();
        harness.window_focus_gained();
        let ime = output(1, 1, IMEPurpose::Normal, "same text");
        harness.pass(Some(&ime));

        harness.external_ime_purpose(IMEPurpose::Normal);
        assert!(matches!(
            harness.pass(Some(&ime)),
            ImeSurroundingAction::None
        ));
        assert_eq!(
            harness
                .sink
                .calls
                .iter()
                .filter(|call| matches!(call, Call::Set(..)))
                .count(),
            1,
            "an unchanged external purpose must be a no-op"
        );

        let second = output(2, 2, IMEPurpose::Normal, "same text");
        assert!(matches!(
            harness.pass(Some(&second)),
            ImeSurroundingAction::ClearThenSet(_)
        ));
        assert_eq!(
            harness.sink.calls,
            vec![
                Call::WindowEnable,
                Call::Set("same text".to_owned(), 9, 9),
                Call::SinkClear,
                Call::Set("same text".to_owned(), 9, 9),
            ],
            "the unchanged purpose must not hide the later owner transition"
        );
    }

    #[test]
    fn destroyed_window_clears_external_and_focus_latches() {
        let mut harness = PassHarness::new();
        harness.window_focus_gained();
        let ime = output(1, 1, IMEPurpose::Normal, "hello");
        harness.pass(Some(&ime));
        harness.external_ime_allowed(false);

        harness.window_destroyed();
        assert!(!harness.allow_ime);
        assert!(!harness.gate.has_pending_external_enable());

        // A reused State must not keep the dead window's external veto.
        harness.window_focus_gained();
        harness.pass(Some(&ime));
        assert!(harness.allow_ime);
        assert_eq!(
            harness.sink.calls.last(),
            Some(&Call::Set("hello".to_owned(), 5, 5))
        );
    }

    #[test]
    fn multiple_states_isolate_external_commands_and_caches() {
        let mut a = PassHarness::new();
        let mut b = PassHarness::new();
        a.window_focus_gained();
        b.window_focus_gained();
        let a_ime = output(1, 1, IMEPurpose::Normal, "alpha");
        let b_ime = output(2, 1, IMEPurpose::Normal, "beta");

        a.pass(Some(&a_ime));
        b.pass(Some(&b_ime));
        a.external_ime_allowed(false);
        a.pass(Some(&a_ime));
        b.pass(Some(&b_ime));

        assert_eq!(
            a.sink.calls,
            vec![
                Call::WindowEnable,
                Call::Set("alpha".to_owned(), 5, 5),
                Call::WindowDisable,
            ],
            "the external disable belongs only to state A"
        );
        assert_eq!(
            b.sink.calls,
            vec![Call::WindowEnable, Call::Set("beta".to_owned(), 4, 4),],
            "state B keeps its own allow latch and published cache"
        );

        a.external_ime_allowed(true);
        a.pass(Some(&a_ime));
        assert_eq!(
            a.sink.calls,
            vec![
                Call::WindowEnable,
                Call::Set("alpha".to_owned(), 5, 5),
                Call::WindowDisable,
                Call::WindowEnable,
                Call::Set("alpha".to_owned(), 5, 5),
            ]
        );
        assert_eq!(
            b.sink.calls,
            vec![Call::WindowEnable, Call::Set("beta".to_owned(), 4, 4),],
            "re-enabling state A must not invalidate or republish state B"
        );
    }

    #[test]
    fn dropping_one_state_does_not_leak_into_a_fresh_state() {
        let ime = output(1, 1, IMEPurpose::Normal, "hello");
        let mut dropped = PassHarness::new();
        dropped.window_focus_gained();
        dropped.pass(Some(&ime));
        dropped.external_ime_allowed(false);
        drop(dropped);

        // There is no process-global pending entry or cache to inherit. A fresh state must still
        // enable/focus and publish the unchanged text from scratch.
        let mut fresh = PassHarness::new();
        fresh.window_focus_gained();
        fresh.pass(Some(&ime));
        assert_eq!(
            fresh.sink.calls,
            vec![Call::WindowEnable, Call::Set("hello".to_owned(), 5, 5),]
        );
        assert!(fresh.sink.allowed);
    }

    #[test]
    fn external_command_then_platform_output_preserves_focus_veto() {
        let mut harness = PassHarness::new();
        harness.window_focus_gained();
        let ime = output(1, 1, IMEPurpose::Normal, "hello");
        harness.pass(Some(&ime));

        // eframe processes viewport commands before/with the platform output for that pass. The
        // synchronous disable must not be undone by the same pass's stale IME descriptor.
        harness.external_ime_allowed(false);
        harness.pass(Some(&ime));
        assert_eq!(
            harness.sink.calls,
            vec![
                Call::WindowEnable,
                Call::Set("hello".to_owned(), 5, 5),
                Call::WindowDisable,
            ]
        );

        // An external enable while unfocused latches but does not transiently enable the window,
        // even when the same pass still has an IME descriptor.
        harness.window_focus_lost();
        harness.external_ime_allowed(true);
        harness.pass(Some(&ime));
        assert_eq!(
            harness.sink.calls,
            vec![
                Call::WindowEnable,
                Call::Set("hello".to_owned(), 5, 5),
                Call::WindowDisable,
            ],
            "the focus veto must survive an external enable and platform output"
        );
        assert!(!harness.sink.allowed);

        // Focus regain consumes the latched external enable and republishes unchanged text.
        harness.window_focus_gained();
        harness.pass(Some(&ime));
        assert_eq!(
            harness.sink.calls,
            vec![
                Call::WindowEnable,
                Call::Set("hello".to_owned(), 5, 5),
                Call::WindowDisable,
                Call::WindowEnable,
                Call::Set("hello".to_owned(), 5, 5),
            ]
        );
    }

    #[test]
    fn multiple_windows_keep_independent_caches_and_latches() {
        let mut sync_a = ImeSurroundingSync::default();
        let mut sync_b = ImeSurroundingSync::default();
        let mut sink_a = FakeSink::new();
        let mut sink_b = FakeSink::new();

        let a_first = output(1, 1, IMEPurpose::Normal, "alpha");
        let b_first = output(2, 1, IMEPurpose::Normal, "beta");
        update(&mut sync_a, &mut sink_a, Some(&a_first));
        update(&mut sync_b, &mut sink_b, Some(&b_first));

        // Window A becomes disabled and forgets; window B must still suppress its unchanged
        // pass and only republish when its own text changes.
        sink_a.allowed = false;
        let a_same = output(1, 1, IMEPurpose::Normal, "alpha");
        sync_a.update(&mut sink_a, Some(&a_same), false);
        update(&mut sync_b, &mut sink_b, Some(&b_first));

        assert_eq!(sink_a.calls, vec![Call::Set("alpha".to_owned(), 5, 5)]);
        assert_eq!(sink_b.calls, vec![Call::Set("beta".to_owned(), 4, 4)]);

        let b_edited = output(2, 1, IMEPurpose::Normal, "beta!");
        update(&mut sync_b, &mut sink_b, Some(&b_edited));
        assert_eq!(
            sink_b.calls,
            vec![
                Call::Set("beta".to_owned(), 4, 4),
                Call::Set("beta!".to_owned(), 5, 5),
            ]
        );
    }

    #[test]
    fn ime_gate_requires_focus_and_no_external_disable() {
        let mut gate = ImeGate::default();
        assert_eq!(
            gate.transition(true, false),
            ImeAllowTransition {
                allow: false,
                changed: false,
            },
            "no IME is enabled before winit reports focus"
        );

        gate.set_focused(true);
        assert_eq!(
            gate.transition(true, false),
            ImeAllowTransition {
                allow: true,
                changed: true,
            }
        );
        assert!(
            gate.transition(false, true).changed,
            "dropping the IME request must disable the window"
        );

        gate.set_external_ime_allowed(false);
        assert_eq!(
            gate.transition(true, true),
            ImeAllowTransition {
                allow: false,
                changed: true,
            }
        );

        gate.set_external_ime_allowed(true);
        assert!(
            gate.transition(true, false).allow,
            "clearing the external disable must permit IME again"
        );

        gate.set_focused(false);
        assert!(
            !gate.transition(true, true).allow,
            "focus loss must veto IME even when an external enable is latched"
        );
    }

    #[test]
    fn async_ime_disabled_event_forgets_once_without_toggling() {
        let mut harness = PassHarness::new();
        harness.window_focus_gained();
        let ime = output(1, 1, IMEPurpose::Normal, "hello");
        harness.pass(Some(&ime));

        // An asynchronous disable from winit only invalidates our cache.
        harness.ime_disabled_event();
        harness.pass(Some(&ime));
        harness.pass(Some(&ime));

        assert_eq!(
            harness.sink.calls,
            vec![
                Call::WindowEnable,
                Call::Set("hello".to_owned(), 5, 5),
                Call::Set("hello".to_owned(), 5, 5),
            ],
            "the lifecycle event must not toggle IME and must cause at most one resend"
        );
    }

    #[test]
    fn diagnostics_redact_surrounding_text() {
        let mut sync = ImeSurroundingSync::default();
        let mut sink = FakeSink::new();
        let ime = output(1, 1, IMEPurpose::Normal, "correct horse battery staple");
        let action = update(&mut sync, &mut sink, Some(&ime));

        let diagnostic = format!("{action:?}");
        assert!(!diagnostic.contains("horse"), "{diagnostic}");
        assert!(diagnostic.contains("<redacted>"), "{diagnostic}");
        assert!(diagnostic.contains("28"), "{diagnostic}");
    }
}

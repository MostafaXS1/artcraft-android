//! Bounded asynchronous clipboard requests for the egui event path.
//!
//! Winit's Wayland clipboard performs every regular and PRIMARY transfer through a caller-owned
//! transfer handle. This module keeps those handles in the per-window [`crate::State`], records the
//! viewport, focus owner and input generation that requested the paste, and drains a completed
//! transfer once on that state's next input preparation. It never waits for a clipboard peer.
//!
//! The transport is a small trait so the request routing can be tested headlessly with delayed
//! mock handles. Production Wayland routing is implemented in [`crate::clipboard`] on top of
//! winit's validated `begin_load_*`, `input_origin` and `claim_*` APIs.

use core::time::Duration;
use std::{collections::BTreeMap, io, sync::Arc, time::Instant};

use egui::mutex::{Mutex, MutexGuard};
use egui::{Id, LayerId, Pos2, ViewportId};
use winit::window::WindowId;

/// A callback invoked once when a transfer has a result.
pub(crate) type CompletionWaker = Arc<dyn Fn() + Send + Sync + 'static>;

/// Total regular-paste delivery budget: 2 s transfer plus decode headroom, never reset.
pub(crate) const DELIVERY_BUDGET: Duration = Duration::from_secs(3);

/// A cfg-independent clipboard bridge failure.
///
/// Winit's Wayland-specific `ClipboardError` is mapped to this type at the adapter edge, so the
/// bridge and its tests never name a Wayland concrete type. Variants mirror the transport
/// classification plus the bounded PNG codec's own failures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PasteError {
    kind: PasteErrorKind,
    message: String,
}

/// The category of a [`PasteError`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(clippy::allow_attributes, dead_code)] // Codec-only variants exist without the image feature.
pub(crate) enum PasteErrorKind {
    /// No offer exists on the selected seat/family.
    Absent,
    /// The offer exists but does not advertise the requested type.
    NotOffered,
    /// The offer belongs to a stale source of this clipboard.
    StaleSelfSource,
    /// The owner, seat, focus, window or offer changed during the transfer.
    Interrupted,
    /// A transfer, delivery or claim deadline expired.
    TimedOut,
    /// The protocol family or backend is unavailable.
    Unsupported,
    /// A queue or admission budget is full.
    Busy,
    /// The payload exceeds a byte or pixel cap.
    TooLarge,
    /// The caller supplied invalid dimensions, limits or state.
    InvalidInput,
    /// The event loop or clipboard connection has ended.
    Disconnected,
    /// The encoded payload is not a valid accepted PNG.
    Malformed,
    /// PNG encoding failed.
    Encode,
    /// An internal codec invariant was violated.
    Internal,
    /// A backend failure that is not classified more specifically.
    Backend,
}

#[allow(clippy::allow_attributes, dead_code)] // Some compatibility helpers are cfg-specific.
impl PasteError {
    pub(crate) fn new(kind: PasteErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub(crate) fn from_io(error: &io::Error) -> Self {
        let kind = match error.kind() {
            io::ErrorKind::NotFound => PasteErrorKind::Absent,
            io::ErrorKind::InvalidData => PasteErrorKind::Malformed,
            io::ErrorKind::Interrupted => PasteErrorKind::Interrupted,
            io::ErrorKind::TimedOut => PasteErrorKind::TimedOut,
            io::ErrorKind::Unsupported => PasteErrorKind::Unsupported,
            io::ErrorKind::WouldBlock => PasteErrorKind::Busy,
            io::ErrorKind::InvalidInput => PasteErrorKind::InvalidInput,
            io::ErrorKind::BrokenPipe | io::ErrorKind::NotConnected => PasteErrorKind::Disconnected,
            _ => PasteErrorKind::Backend,
        };
        Self::new(kind, error.to_string())
    }

    pub(crate) fn kind(&self) -> PasteErrorKind {
        self.kind
    }

    pub(crate) fn io_kind(&self) -> io::ErrorKind {
        match self.kind {
            PasteErrorKind::Absent => io::ErrorKind::NotFound,
            PasteErrorKind::NotOffered
            | PasteErrorKind::StaleSelfSource
            | PasteErrorKind::TooLarge
            | PasteErrorKind::Malformed => io::ErrorKind::InvalidData,
            PasteErrorKind::Interrupted => io::ErrorKind::Interrupted,
            PasteErrorKind::TimedOut => io::ErrorKind::TimedOut,
            PasteErrorKind::Unsupported => io::ErrorKind::Unsupported,
            PasteErrorKind::Busy => io::ErrorKind::WouldBlock,
            PasteErrorKind::InvalidInput => io::ErrorKind::InvalidInput,
            PasteErrorKind::Disconnected => io::ErrorKind::BrokenPipe,
            PasteErrorKind::Encode | PasteErrorKind::Internal | PasteErrorKind::Backend => {
                io::ErrorKind::Other
            }
        }
    }

    pub(crate) fn to_io_error(&self) -> io::Error {
        io::Error::new(self.io_kind(), self.message.clone())
    }
}

impl core::fmt::Display for PasteError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.message)
    }
}

impl core::error::Error for PasteError {}

/// Which selection family a transfer or claim addresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum PasteKind {
    /// The regular clipboard (`wl_data_device` selection).
    Regular,
    /// The PRIMARY selection (`zwp_primary_selection_device_v1`).
    Primary,
}

/// One encoded PNG payload kept alive by its transport permit.
///
/// The bridge never inspects or detaches transport-owned accounting. Production wraps winit's
/// `PngBytes`; tests can wrap an ordinary `Arc<[u8]>`-like payload.
pub(crate) trait EncodedPng: Send + Sync {
    /// The exact encoded PNG bytes.
    #[allow(clippy::allow_attributes, dead_code)] // Only the optional image codec reads these bytes.
    fn as_bytes(&self) -> &[u8];

    /// The encoded payload length in bytes.
    fn len(&self) -> usize;
}

/// One combined regular-read result.
#[derive(Clone)]
#[allow(clippy::allow_attributes, dead_code)] // Constructed by the optional Wayland transport and tests.
pub(crate) enum PasteContent {
    /// Normalized UTF-8 text.
    Text(Arc<str>),
    /// Exact `image/png` bytes with their transport permit still held.
    ImagePng(Arc<dyn EncodedPng>),
}

impl core::fmt::Debug for PasteContent {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Text(text) => f.debug_tuple("Text").field(text).finish(),
            Self::ImagePng(png) => f.debug_tuple("ImagePng").field(&png.len()).finish(),
        }
    }
}

/// One owned asynchronous read.
///
/// Production is winit's combined or PRIMARY transfer handle; tests use a mock with the same
/// once-only contract. `try_take` consumes the result at most once.
pub(crate) trait PasteTransfer: Send + Sync {
    /// Take the completed result, if any. A cancelled handle never yields a result.
    fn try_take(&self) -> Option<Result<PasteContent, PasteError>>;

    /// Cancel the transfer. Idempotent; a cancelled transfer never wakes the caller.
    fn cancel(&self);
}

/// A decoded image plus the codec admission that must stay alive until handoff.
pub(crate) struct DecodedPasteImage {
    /// The decoded, premultiplied image.
    pub image: egui::ColorImage,
    /// Codec-owned accounting retained until the application owns `image`.
    pub(crate) retention: Box<dyn Send + Sync>,
}

impl core::fmt::Debug for DecodedPasteImage {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DecodedPasteImage")
            .field("size", &self.image.size)
            .finish_non_exhaustive()
    }
}

impl DecodedPasteImage {
    #[allow(clippy::allow_attributes, dead_code)] // Called by the optional image codec and tests.
    pub(crate) fn new(image: egui::ColorImage, retention: Box<dyn Send + Sync>) -> Self {
        Self { image, retention }
    }
}

/// A handle that removes or marks queued codec work.
///
/// Cancelling a queued job drops its payload and permit promptly. Running work may finish but
/// remains budgeted until it completes. Cancellation never wakes the caller.
pub(crate) trait JobCancellation: Send + Sync {
    fn cancel(&self);
}

/// The production bounded PNG decoder used by the composed paste state.
pub(crate) trait ImagePasteDecoder: Send + Sync {
    /// Submit one decode. The completion runs exactly once, off the event-loop thread, and must
    /// not be called while the composed state's lock is held.
    ///
    /// Returns a cancellation handle when the decode was accepted into a cancellable queue.
    fn decode(
        &self,
        png: Arc<dyn EncodedPng>,
        deadline: Instant,
        completion: Box<dyn FnOnce(Result<DecodedPasteImage, PasteError>) + Send + 'static>,
    ) -> Option<Arc<dyn JobCancellation>>;
}

/// The clipboard operations the bridge needs.
///
/// The associated claim token is opaque to this module and is produced by the same transport
/// that validates it. That keeps winit's `InputOriginToken` single-use rules in winit.
pub(crate) trait ClipboardTransport {
    /// The transport's opaque, single-use input-origin token.
    ///
    /// Cloning is allowed so one captured batch origin can be offered independently to the
    /// regular and PRIMARY families; each family may consume its clone only once.
    type ClaimToken: Clone;

    /// Start a bounded asynchronous read for `target_window`.
    fn begin_load(
        &self,
        kind: PasteKind,
        target_window: WindowId,
        caller_generation: u64,
        waker: CompletionWaker,
    ) -> Result<Box<dyn PasteTransfer>, PasteError>;

    /// Get a fresh trigger token for the current input turn on `target_window`.
    fn input_origin(&self, target_window: WindowId) -> Result<Self::ClaimToken, PasteError>;

    /// Claim selection ownership synchronously in the triggering input turn.
    fn claim(
        &self,
        kind: PasteKind,
        text: Arc<str>,
        token: Self::ClaimToken,
        target_window: WindowId,
        caller_generation: u64,
    ) -> Result<(), PasteError>;

    /// Invalidate pending regular image reservations for a new winning regular output.
    ///
    /// Non-image transports keep the no-op default.
    #[allow(clippy::allow_attributes, dead_code)] // Called by the image-enabled Wayland output path.
    fn invalidate_image_reservations(&self, _target_window: WindowId) -> Result<(), PasteError> {
        Ok(())
    }
}

/// Claim a selection synchronously with an already-resolved exact origin token.
///
/// Origin lookup is intentionally not performed here: output processing resolves the batch
/// capture once and passes a clone to each winning family. This helper never substitutes a
/// later serial. Ownership is set before it returns; pipe serving happens later on winit's
/// bounded workers.
#[allow(clippy::allow_attributes, dead_code)] // Only the Wayland event path has an origin-token transport.
pub(crate) fn claim<T: ClipboardTransport>(
    transport: &T,
    kind: PasteKind,
    text: &str,
    token: T::ClaimToken,
    target_window: WindowId,
    caller_generation: u64,
) -> Result<(), PasteError> {
    transport.claim(
        kind,
        Arc::from(text),
        token,
        target_window,
        caller_generation,
    )
}

/// One batch's captured input-origin state.
///
/// `Absent` is not the same as a failed or stale capture: only `Absent` may use the existing
/// latest-fresh compatibility fallback for regular text or PRIMARY. A captured token belongs to
/// the input generation in which it was obtained; output in another generation fails instead
/// of silently substituting a later serial. A failed capture likewise never falls back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[allow(clippy::allow_attributes, dead_code)] // Only the Wayland event path captures an origin.
pub(crate) enum CapturedClaimOrigin<Token> {
    /// No claimable input event was captured for this batch.
    #[default]
    Absent,
    /// A claimable input event was observed and yielded this exact token.
    Captured {
        /// The paste/input generation at capture time.
        generation: u64,
        /// The transport's single-use-per-family token.
        token: Token,
    },
    /// A claimable input event was observed but its token could not be obtained.
    Failed {
        /// The paste/input generation at capture time.
        generation: u64,
        /// The transport error kind, retained for diagnostics.
        kind: io::ErrorKind,
    },
}

/// Capture the current transport origin for an input generation.
///
/// The error is retained as an explicit `Failed` state, rather than collapsed into `Absent`, so
/// output processing can reject the batch without asking for a later origin.
#[allow(clippy::allow_attributes, dead_code)] // Only the Wayland event path captures an origin.
pub(crate) fn capture_claim_origin<T: ClipboardTransport>(
    transport: &T,
    target_window: WindowId,
    generation: u64,
) -> CapturedClaimOrigin<T::ClaimToken> {
    match transport.input_origin(target_window) {
        Ok(token) => CapturedClaimOrigin::Captured { generation, token },
        Err(error) => CapturedClaimOrigin::Failed {
            generation,
            kind: error.io_kind(),
        },
    }
}

/// Per-`State` preparation of one input batch's claim origin.
///
/// Claimable key/pointer events and `RequestCopy`/`RequestCut` only mark the batch as pending
/// while events are dispatched. The single transport lookup happens once in
/// [`crate::State::take_egui_input`], after any matching focus gain has been processed. Output
/// handling always takes this slot, even when no clipboard command wins, so an unused token can
/// never be associated with a later batch.
#[allow(clippy::allow_attributes, dead_code)] // Only the Wayland event path captures an origin.
pub(crate) struct ClaimBatch<Token> {
    pending: bool,
    prepared: CapturedClaimOrigin<Token>,
}

impl<Token> Default for ClaimBatch<Token> {
    fn default() -> Self {
        Self {
            pending: false,
            prepared: CapturedClaimOrigin::Absent,
        }
    }
}

#[allow(clippy::allow_attributes, dead_code)] // Only the Wayland event path uses the batch.
impl<Token> ClaimBatch<Token> {
    /// Record a claimable trigger, `RequestCopy` or `RequestCut` for the current input batch.
    pub(crate) fn note_claimable_input(&mut self) {
        self.pending = true;
    }

    /// Prepare the batch's single origin lookup.
    ///
    /// `transport` is `None` when this build/display has no Wayland clipboard transport, or when
    /// preparation happens on a non-Wayland window; the prepared state is then `Absent`.
    /// A pending request is consumed exactly once, and an older prepared value can never survive
    /// into another batch even when output handling was skipped.
    pub(crate) fn prepare<T: ClipboardTransport<ClaimToken = Token>>(
        &mut self,
        transport: Option<&T>,
        target_window: WindowId,
        generation: u64,
    ) {
        let pending = core::mem::take(&mut self.pending);
        self.prepared = match (pending, transport) {
            (true, Some(transport)) => capture_claim_origin(transport, target_window, generation),
            _ => CapturedClaimOrigin::Absent,
        };
    }

    /// Take the prepared origin for this batch. Output handling calls this unconditionally.
    pub(crate) fn take_prepared(&mut self) -> CapturedClaimOrigin<Token> {
        core::mem::take(&mut self.prepared)
    }

    /// Drop both a pending request and any prepared token.
    pub(crate) fn invalidate(&mut self) {
        self.pending = false;
        self.prepared = CapturedClaimOrigin::Absent;
    }

    /// Whether a claimable event is waiting for input preparation.
    #[cfg(test)]
    pub(crate) fn has_pending_input(&self) -> bool {
        self.pending
    }

    /// The prepared state, for tests. It does not consume the slot.
    #[cfg(test)]
    pub(crate) fn prepared(&self) -> &CapturedClaimOrigin<Token> {
        &self.prepared
    }
}

/// Whether a regular-clipboard winner may use the legacy latest-fresh fallback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RegularClaimOriginPolicy {
    /// `CopyText`: a captured token or, when `Absent`, one synchronous fallback.
    AllowFreshFallback,
    /// `CopyImage`: the exact captured token only; never a fallback.
    RequireCapturedOrigin,
}

/// A refused claim origin.
///
/// These errors are explicit so a stale/failed capture can never turn into a different serial.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ClaimOriginError {
    /// A captured token belongs to a different input generation.
    StaleCapture {
        /// Generation requested by output processing.
        expected_generation: u64,
        /// Generation at which the token was captured.
        captured_generation: u64,
    },
    /// A claimable input event was seen, but token capture failed.
    CaptureFailed {
        /// Generation requested by output processing.
        expected_generation: u64,
        /// Generation at which capture failed.
        captured_generation: u64,
        /// The transport error kind, retained for diagnostics.
        kind: io::ErrorKind,
    },
    /// An image winner requires a captured origin and none is available.
    ImageRequiresCapturedOrigin {
        /// Generation requested by output processing.
        expected_generation: u64,
    },
    /// The permitted one-shot fallback lookup failed.
    FreshOriginFailed {
        /// Transport error kind.
        kind: io::ErrorKind,
        /// Transport error text.
        message: String,
    },
}

impl ClaimOriginError {
    fn kind(&self) -> io::ErrorKind {
        match self {
            Self::StaleCapture { .. } | Self::ImageRequiresCapturedOrigin { .. } => {
                io::ErrorKind::InvalidInput
            }
            Self::CaptureFailed { kind, .. } | Self::FreshOriginFailed { kind, .. } => *kind,
        }
    }
}

impl core::fmt::Display for ClaimOriginError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::StaleCapture {
                expected_generation,
                captured_generation,
            } => write!(
                f,
                "captured clipboard origin is stale (captured generation {captured_generation}, output generation {expected_generation})"
            ),
            Self::CaptureFailed {
                expected_generation,
                captured_generation,
                kind,
            } => write!(
                f,
                "clipboard origin capture failed ({kind:?}; captured generation {captured_generation}, output generation {expected_generation})"
            ),
            Self::ImageRequiresCapturedOrigin {
                expected_generation,
            } => write!(
                f,
                "image copy requires a captured origin (output generation {expected_generation}); fallback is not allowed"
            ),
            Self::FreshOriginFailed { kind, message } => {
                write!(
                    f,
                    "latest clipboard origin lookup failed ({kind:?}): {message}"
                )
            }
        }
    }
}

impl core::error::Error for ClaimOriginError {}

impl From<ClaimOriginError> for io::Error {
    fn from(error: ClaimOriginError) -> Self {
        Self::new(error.kind(), error.to_string())
    }
}

/// Resolved origins for the clipboard families selected from one output batch.
///
/// `None` means that family has no winner. A `Some(Err(..))` winner must fail explicitly and
/// leave the previous clipboard source unchanged.
#[derive(Debug)]
#[allow(clippy::allow_attributes, dead_code)] // Read by the Wayland output path and tests.
pub(crate) struct BatchClaimOrigins<Token> {
    /// Origin for the winning regular text or image command.
    pub regular: Option<Result<Token, ClaimOriginError>>,
    /// Origin for the winning PRIMARY command.
    pub primary: Option<Result<Token, ClaimOriginError>>,
}

impl<Token> Default for BatchClaimOrigins<Token> {
    fn default() -> Self {
        Self {
            regular: None,
            primary: None,
        }
    }
}

/// Resolve the captured batch origin once, cloning it for each winning family.
///
/// This is the only place in output handling that may call `input_origin`. It does so at most
/// once, and only when the capture is `Absent` and at least one family permits the legacy
/// fallback (regular text or PRIMARY). A captured stale/failed origin and an image winner never
/// trigger the lookup. No winner means no origin work at all.
#[allow(clippy::allow_attributes, dead_code)] // Only the Wayland event path resolves origins.
pub(crate) fn resolve_batch_claim_origins<T: ClipboardTransport>(
    transport: &T,
    target_window: WindowId,
    expected_generation: u64,
    captured: CapturedClaimOrigin<T::ClaimToken>,
    regular: Option<RegularClaimOriginPolicy>,
    primary: bool,
) -> BatchClaimOrigins<T::ClaimToken> {
    if regular.is_none() && !primary {
        return BatchClaimOrigins::default();
    }

    let fallback_permitted =
        primary || regular == Some(RegularClaimOriginPolicy::AllowFreshFallback);
    let fallback = if matches!(&captured, CapturedClaimOrigin::Absent) && fallback_permitted {
        Some(transport.input_origin(target_window).map_err(|error| {
            ClaimOriginError::FreshOriginFailed {
                kind: error.io_kind(),
                message: error.to_string(),
            }
        }))
    } else {
        None
    };

    let resolve = |policy: RegularClaimOriginPolicy| {
        let allow_fallback = policy == RegularClaimOriginPolicy::AllowFreshFallback;
        match &captured {
            CapturedClaimOrigin::Captured { generation, token }
                if *generation == expected_generation =>
            {
                Ok(token.clone())
            }
            CapturedClaimOrigin::Captured { generation, .. } => {
                Err(ClaimOriginError::StaleCapture {
                    expected_generation,
                    captured_generation: *generation,
                })
            }
            CapturedClaimOrigin::Failed { generation, kind } => {
                Err(ClaimOriginError::CaptureFailed {
                    expected_generation,
                    captured_generation: *generation,
                    kind: *kind,
                })
            }
            CapturedClaimOrigin::Absent if allow_fallback => fallback
                .as_ref()
                .expect("fallback is requested when a family permits it")
                .clone(),
            CapturedClaimOrigin::Absent => Err(ClaimOriginError::ImageRequiresCapturedOrigin {
                expected_generation,
            }),
        }
    };

    let resolve_primary = || match &captured {
        CapturedClaimOrigin::Captured { generation, token }
            if *generation == expected_generation =>
        {
            Ok(token.clone())
        }
        CapturedClaimOrigin::Captured { generation, .. } => Err(ClaimOriginError::StaleCapture {
            expected_generation,
            captured_generation: *generation,
        }),
        CapturedClaimOrigin::Failed { generation, kind } => Err(ClaimOriginError::CaptureFailed {
            expected_generation,
            captured_generation: *generation,
            kind: *kind,
        }),
        CapturedClaimOrigin::Absent => fallback
            .as_ref()
            .expect("PRIMARY requests the fallback when visible")
            .clone(),
    };

    let origins = BatchClaimOrigins {
        regular: regular.map(resolve),
        primary: primary.then(resolve_primary),
    };
    drop(captured); // Consume this prepared batch even when no family retains a token.
    origins
}

/// An input action that may invalidate a pending paste.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InputKind {
    /// Another key press (an edit, shortcut or selection-changing key).
    KeyPress,
    /// A pointer press starts a click, selection or focus change.
    PointerPress,
    /// An IME commit edits the focused text.
    Edit,
    /// Window focus was lost.
    Focus,
    /// Window focus was gained. A click-to-focus compositor may deliver this immediately after
    /// the middle-click press that started a PRIMARY read; it must not cancel that same read.
    FocusGained,
    /// The window is closing or was destroyed.
    Close,
    /// The window was hidden, minimized or occluded.
    Hide,
    /// A wheel scroll can move content under a pending middle-click position.
    Scroll,
    /// A key/pointer release or a repaint. These deliberately do not cancel a pending paste.
    NonInvalidating,
}

/// A completed transfer, revalidated against the requesting state.
#[derive(Debug)]
pub(crate) enum PasteOutcome {
    /// A completed regular-clipboard text read, or a transfer/decode failure.
    Regular(Result<String, PasteError>),
    /// A completed regular-clipboard PNG read after bounded decode.
    RegularImage(DecodedPasteImage),
    /// A completed PRIMARY read, with the middle-click position captured at request time.
    Primary {
        pos: Pos2,
        result: Result<String, PasteError>,
    },
}

/// Fields shared by every pending request and used for delivery fences.
struct PendingCommon {
    viewport_id: ViewportId,
    target_window: WindowId,
    generation: u64,
    focused_widget: Option<Id>,
    /// How many focus-owner changes this request may tolerate before it becomes stale.
    allowed_focus_changes: u8,
    /// The layer under the middle-click position at request time.
    target_layer: Option<LayerId>,
    pos: Option<Pos2>,
}

struct PendingPaste {
    common: PendingCommon,
    handle: Box<dyn PasteTransfer>,
}

struct PendingRegular {
    common: PendingCommon,
    composed: ComposedRegular,
}

enum PendingRequest {
    Raw(PendingPaste),
    Regular(PendingRegular),
}

impl PendingRequest {
    fn common(&self) -> &PendingCommon {
        match self {
            Self::Raw(request) => &request.common,
            Self::Regular(request) => &request.common,
        }
    }

    fn common_mut(&mut self) -> &mut PendingCommon {
        match self {
            Self::Raw(request) => &mut request.common,
            Self::Regular(request) => &mut request.common,
        }
    }

    fn kind(&self) -> PasteKind {
        match self {
            Self::Raw(_) => PasteKind::Primary,
            Self::Regular(_) => PasteKind::Regular,
        }
    }

    fn cancel(self) {
        match self {
            Self::Raw(request) => request.handle.cancel(),
            Self::Regular(request) => request.composed.cancel(),
        }
    }
}

/// A PRIMARY read that was rejected because the active seat has not yet delivered focus to the
/// target window. Click-to-focus compositors may dispatch the pointer press before
/// `wl_keyboard.enter`; the request is retried once the matching `Focused(true)` is observed.
struct DeferredPrimary {
    pos: Pos2,
    focused_widget: Option<Id>,
    target_layer: Option<LayerId>,
    generation: u64,
    waker: CompletionWaker,
}

/// A composed regular request: raw transfer, bounded PNG decode and one terminal delivery.
///
/// The raw completion waker drives decode itself, so suppressing the viewport repaint cannot
/// deadlock a paste. The caller terminal waker is invoked exactly once, outside all locks, on
/// the terminal transition only. Cancellation never wakes.
struct ComposedRegular {
    inner: Arc<ComposedRegularInner>,
}

struct ComposedRegularInner {
    state: Mutex<ComposedState>,
    decoder: Option<Arc<dyn ImagePasteDecoder>>,
    terminal_waker: CompletionWaker,
    deadline: Instant,
    /// Deterministic deadline clock for tests.
    #[cfg(test)]
    test_now: Mutex<Option<Instant>>,
}

struct ComposedState {
    phase: ComposedPhase,
    raw: Option<Box<dyn PasteTransfer>>,
    decode_epoch: u64,
    decode_cancel: Option<Arc<dyn JobCancellation>>,
    result: Option<Result<RegularPasteResult, PasteError>>,
    terminal_woken: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ComposedPhase {
    Loading,
    Decoding,
    Ready,
    Failed,
    Cancelled,
    Consumed,
}

enum RegularPasteResult {
    Text(Arc<str>),
    Image(DecodedPasteImage),
}

struct DecodeRequest {
    epoch: u64,
    png: Arc<dyn EncodedPng>,
    decoder: Arc<dyn ImagePasteDecoder>,
    deadline: Instant,
}

impl ComposedRegular {
    fn new(
        decoder: Option<Arc<dyn ImagePasteDecoder>>,
        terminal_waker: CompletionWaker,
        deadline: Instant,
    ) -> Self {
        Self {
            inner: Arc::new(ComposedRegularInner {
                state: Mutex::new(ComposedState {
                    phase: ComposedPhase::Loading,
                    raw: None,
                    decode_epoch: 0,
                    decode_cancel: None,
                    result: None,
                    terminal_woken: false,
                }),
                decoder,
                terminal_waker,
                deadline,
                #[cfg(test)]
                test_now: Mutex::new(None),
            }),
        }
    }

    /// The clock used for absolute deadline checks.
    #[cfg_attr(
        not(test),
        expect(
            clippy::unused_self,
            reason = "tests supply this request’s deterministic clock"
        )
    )]
    fn now(&self) -> Instant {
        #[cfg(test)]
        {
            if let Some(now) = *self.inner.test_now.lock() {
                return now;
            }
        }
        Instant::now()
    }

    #[cfg(test)]
    fn set_test_now(&self, now: Instant) {
        *self.inner.test_now.lock() = Some(now);
    }

    fn lock(&self) -> MutexGuard<'_, ComposedState> {
        self.inner.state.lock()
    }

    fn raw_waker(&self) -> CompletionWaker {
        let weak = Arc::downgrade(&self.inner);
        Arc::new(move || {
            if let Some(inner) = weak.upgrade() {
                Self { inner }.advance_raw();
            }
        })
    }

    fn attach(&self, handle: Box<dyn PasteTransfer>) {
        let attached = {
            let mut state = self.lock();
            if state.phase == ComposedPhase::Loading {
                state.raw = Some(handle);
                true
            } else {
                false
            }
        };
        if attached {
            // Also picks up a self-paste that completed before the handle was returned.
            self.advance_raw();
        }
    }

    /// Take a raw result and, for PNG, submit decode. Safe to call from a winit worker waker and
    /// from `attach` after an immediate self-paste wake.
    fn advance_raw(&self) {
        let mut decode = None;
        let mut wake = false;
        {
            let mut state = self.lock();
            if state.phase != ComposedPhase::Loading {
                return;
            }
            let Some(raw) = state.raw.as_ref() else {
                // The handle is attached after `begin_load`; `attach` re-runs this method.
                return;
            };
            let Some(result) = raw.try_take() else {
                return;
            };
            state.raw = None;
            match result {
                Ok(PasteContent::Text(text)) => {
                    let result = if self.now() >= self.inner.deadline {
                        Err(PasteError::new(
                            PasteErrorKind::TimedOut,
                            "the composed paste delivery deadline expired",
                        ))
                    } else {
                        Ok(RegularPasteResult::Text(text))
                    };
                    state.phase = if result.is_ok() {
                        ComposedPhase::Ready
                    } else {
                        ComposedPhase::Failed
                    };
                    state.result = Some(result);
                    if !state.terminal_woken {
                        state.terminal_woken = true;
                        wake = true;
                    }
                }
                Ok(PasteContent::ImagePng(png)) => {
                    if self.now() >= self.inner.deadline {
                        state.phase = ComposedPhase::Failed;
                        state.result = Some(Err(PasteError::new(
                            PasteErrorKind::TimedOut,
                            "the composed paste delivery deadline expired",
                        )));
                        if !state.terminal_woken {
                            state.terminal_woken = true;
                            wake = true;
                        }
                    } else if let Some(decoder) = &self.inner.decoder {
                        state.phase = ComposedPhase::Decoding;
                        state.decode_epoch = state.decode_epoch.wrapping_add(1);
                        decode = Some(DecodeRequest {
                            epoch: state.decode_epoch,
                            png,
                            decoder: Arc::clone(decoder),
                            deadline: self.inner.deadline,
                        });
                    } else {
                        state.phase = ComposedPhase::Failed;
                        state.result = Some(Err(PasteError::new(
                            PasteErrorKind::NotOffered,
                            "PNG decode is unavailable in this build",
                        )));
                        if !state.terminal_woken {
                            state.terminal_woken = true;
                            wake = true;
                        }
                    }
                }
                Err(error) => {
                    state.phase = ComposedPhase::Failed;
                    state.result = Some(Err(error));
                    if !state.terminal_woken {
                        state.terminal_woken = true;
                        wake = true;
                    }
                }
            }
        }

        if wake {
            (self.inner.terminal_waker)();
        }

        if let Some(request) = decode {
            let request = Arc::new(request);
            let weak = Arc::downgrade(&self.inner);
            let completion_request = Arc::clone(&request);
            let completion: Box<
                dyn FnOnce(Result<DecodedPasteImage, PasteError>) + Send + 'static,
            > = Box::new(move |result| {
                let Some(inner) = weak.upgrade() else {
                    return;
                };
                let composed = Self { inner };
                composed.decode_finished(completion_request.epoch, result);
            });
            let cancel =
                request
                    .decoder
                    .decode(Arc::clone(&request.png), request.deadline, completion);
            if let Some(cancel) = cancel {
                // Publish the handle for `cancel`/deadline handling. If the request became
                // terminal or was cancelled while `decode` was starting, cancel the job instead
                // of leaving it queued.
                let cancel_now = {
                    let mut state = self.lock();
                    if state.phase == ComposedPhase::Decoding && state.decode_epoch == request.epoch
                    {
                        state.decode_cancel = Some(Arc::clone(&cancel));
                        false
                    } else {
                        true
                    }
                };
                if cancel_now {
                    cancel.cancel();
                }
            }
        }
    }

    fn decode_finished(&self, epoch: u64, result: Result<DecodedPasteImage, PasteError>) {
        let mut wake = false;
        {
            let mut state = self.lock();
            if state.phase != ComposedPhase::Decoding || state.decode_epoch != epoch {
                // Cancelled, superseded or already terminal: the decode permit/image is dropped
                // here and no caller wake is produced.
                return;
            }
            let result = if self.now() >= self.inner.deadline {
                Err(PasteError::new(
                    PasteErrorKind::TimedOut,
                    "the composed paste delivery deadline expired during decode",
                ))
            } else {
                result
            };
            state.decode_cancel = None;
            state.phase = if result.is_ok() {
                ComposedPhase::Ready
            } else {
                ComposedPhase::Failed
            };
            state.result = Some(result.map(RegularPasteResult::Image));
            if !state.terminal_woken {
                state.terminal_woken = true;
                wake = true;
            }
        }
        if wake {
            (self.inner.terminal_waker)();
        }
    }

    /// Take the terminal result exactly once. The absolute deadline is rechecked here for
    /// in-progress work and for an already-`Ready` result: a value ready before the deadline but
    /// drained afterwards becomes `TimedOut` and never turns into a late paste.
    fn try_take_terminal(&self) -> Option<Result<RegularPasteResult, PasteError>> {
        let mut cancel = None;
        let mut state = self.lock();
        if matches!(
            state.phase,
            ComposedPhase::Loading | ComposedPhase::Decoding | ComposedPhase::Ready
        ) {
            if self.now() >= self.inner.deadline {
                state.phase = ComposedPhase::Failed;
                state.result = Some(Err(PasteError::new(
                    PasteErrorKind::TimedOut,
                    "the composed paste delivery deadline expired",
                )));
                state.decode_epoch = state.decode_epoch.wrapping_add(1);
                state.raw = None;
                cancel = state.decode_cancel.take();
            } else if !matches!(state.phase, ComposedPhase::Ready) {
                return None;
            }
        }
        let result = match state.phase {
            ComposedPhase::Ready | ComposedPhase::Failed => {
                state.phase = ComposedPhase::Consumed;
                state.result.take()
            }
            ComposedPhase::Cancelled
            | ComposedPhase::Consumed
            | ComposedPhase::Loading
            | ComposedPhase::Decoding => None,
        };
        drop(state);
        if let Some(cancel) = cancel {
            cancel.cancel();
        }
        result
    }

    /// Cancel without waking. Queued decode work is removed and its payload/permit released;
    /// running decode may finish, but its result cannot escape.
    fn cancel(&self) {
        let (raw, cancel) = {
            let mut state = self.lock();
            if matches!(
                state.phase,
                ComposedPhase::Cancelled | ComposedPhase::Consumed
            ) {
                return;
            }
            state.phase = ComposedPhase::Cancelled;
            state.decode_epoch = state.decode_epoch.wrapping_add(1);
            state.result = None;
            (state.raw.take(), state.decode_cancel.take())
        };
        if let Some(raw) = raw {
            raw.cancel();
        }
        if let Some(cancel) = cancel {
            cancel.cancel();
        }
    }
}

/// The per-window pending paste table.
pub(crate) struct PasteBridge {
    viewport_id: ViewportId,
    target_window: Option<WindowId>,
    generation: u64,
    next_request_id: u64,
    pending: BTreeMap<u64, PendingRequest>,
    deferred_primary: Option<DeferredPrimary>,
    image_decoder: Option<Arc<dyn ImagePasteDecoder>>,
}

impl PasteBridge {
    pub(crate) fn new(viewport_id: ViewportId) -> Self {
        Self {
            viewport_id,
            target_window: None,
            generation: 0,
            next_request_id: 0,
            pending: BTreeMap::new(),
            deferred_primary: None,
            image_decoder: None,
        }
    }

    /// Install the bounded PNG decoder used for combined regular reads.
    #[allow(clippy::allow_attributes, dead_code)] // Only image-enabled Wayland builds decode PNG.
    pub(crate) fn set_image_decoder(&mut self, decoder: Option<Arc<dyn ImagePasteDecoder>>) {
        self.image_decoder = decoder;
    }

    /// The current input generation. Requests record it and only complete while it is unchanged.
    #[allow(clippy::allow_attributes, dead_code)] // Read only by the Wayland event path.
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    /// Bind this state to its native window. A replaced target cancels every pending transfer:
    /// a closed or reused window must never receive another window's paste.
    /// Returns whether the target changed, so the caller can invalidate its prepared claim.
    pub(crate) fn set_target_window(&mut self, target_window: WindowId) -> bool {
        if self.target_window == Some(target_window) {
            return false;
        }
        self.cancel_all();
        self.target_window = Some(target_window);
        self.generation = self.generation.wrapping_add(1);
        true
    }

    /// Record an input action. Invalidating actions advance the generation and cancel in-flight
    /// transfers; releases and repaints leave the pending transfer alone.
    pub(crate) fn note_input(&mut self, kind: InputKind) {
        match kind {
            InputKind::NonInvalidating => {}
            InputKind::FocusGained => self.note_focus_gained(),
            InputKind::Scroll => {
                self.cancel_kind(PasteKind::Primary);
                self.deferred_primary = None;
            }
            InputKind::KeyPress
            | InputKind::PointerPress
            | InputKind::Edit
            | InputKind::Focus
            | InputKind::Close
            | InputKind::Hide => {
                self.cancel_all();
                self.generation = self.generation.wrapping_add(1);
            }
        }
    }

    fn note_focus_gained(&mut self) {
        let keeps_primary = self.pending.values().any(|request| {
            request.kind() == PasteKind::Primary && request.common().generation == self.generation
        }) || self
            .deferred_primary
            .as_ref()
            .is_some_and(|deferred| deferred.generation == self.generation);

        if keeps_primary {
            self.cancel_kind(PasteKind::Regular);
        } else {
            self.cancel_all();
            self.generation = self.generation.wrapping_add(1);
        }
    }

    /// Start a regular-clipboard paste for the focused widget.
    ///
    /// The request is bound to this state's viewport, current window and input generation. Its
    /// waker must request a repaint of that viewport; workers never inspect egui state. Regular
    /// reads are composed: a raw completion may submit PNG decode without waiting for a UI poll.
    #[allow(clippy::allow_attributes, dead_code)] // Non-Wayland platforms keep their synchronous clipboard path.
    pub(crate) fn begin_regular<T: ClipboardTransport>(
        &mut self,
        transport: &T,
        focused_widget: Option<Id>,
        waker: CompletionWaker,
    ) -> Result<(), PasteError> {
        let target_window = self.target_window.ok_or_else(|| {
            PasteError::new(
                PasteErrorKind::Disconnected,
                "the state has no native window for this clipboard request",
            )
        })?;
        let generation = self.generation;
        let common = PendingCommon {
            viewport_id: self.viewport_id,
            target_window,
            generation,
            focused_widget,
            allowed_focus_changes: 0,
            target_layer: None,
            pos: None,
        };
        let deadline = Instant::now() + DELIVERY_BUDGET;
        let composed =
            ComposedRegular::new(self.image_decoder.as_ref().map(Arc::clone), waker, deadline);
        let raw_waker = composed.raw_waker();
        let handle =
            transport.begin_load(PasteKind::Regular, target_window, generation, raw_waker)?;
        composed.attach(handle);
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1);
        self.pending.insert(
            request_id,
            PendingRequest::Regular(PendingRegular { common, composed }),
        );
        Ok(())
    }

    /// Start a PRIMARY paste for a middle-click at `pos`.
    ///
    /// `target_layer` is the egui layer under `pos` at request time; a change is treated as
    /// target churn. If the active seat has not yet focused this window (the pointer press may be
    /// delivered before `wl_keyboard.enter`), the request is deferred and retried after focus.
    #[allow(clippy::allow_attributes, dead_code)] // Non-Wayland platforms keep their synchronous PRIMARY path.
    pub(crate) fn begin_primary<T: ClipboardTransport>(
        &mut self,
        transport: &T,
        focused_widget: Option<Id>,
        pos: Pos2,
        target_layer: Option<LayerId>,
        waker: CompletionWaker,
    ) -> Result<(), PasteError> {
        let retry_waker = Arc::clone(&waker);
        self.deferred_primary = None;
        match self.begin_raw(
            PasteKind::Primary,
            transport,
            focused_widget,
            Some(pos),
            target_layer,
            waker,
        ) {
            Ok(()) => {
                self.deferred_primary = None;
                Ok(())
            }
            Err(error) if error.io_kind() == io::ErrorKind::InvalidInput => {
                self.deferred_primary = Some(DeferredPrimary {
                    pos,
                    focused_widget,
                    target_layer,
                    generation: self.generation,
                    waker: retry_waker,
                });
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    /// Retry a focus-deferred PRIMARY read. Safe to call on every focus gain; a still-unfocused
    /// seat keeps the same single deferred request.
    #[allow(clippy::allow_attributes, dead_code)] // Only the Wayland event path defers reads.
    pub(crate) fn retry_deferred_primary<T: ClipboardTransport>(
        &mut self,
        transport: &T,
    ) -> Result<(), PasteError> {
        let Some(deferred) = self.deferred_primary.take() else {
            return Ok(());
        };
        if deferred.generation != self.generation || self.target_window.is_none() {
            return Ok(());
        }

        match self.begin_raw(
            PasteKind::Primary,
            transport,
            deferred.focused_widget,
            Some(deferred.pos),
            deferred.target_layer,
            Arc::clone(&deferred.waker),
        ) {
            Ok(()) => Ok(()),
            Err(error) if error.io_kind() == io::ErrorKind::InvalidInput => {
                self.deferred_primary = Some(deferred);
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    fn begin_raw<T: ClipboardTransport>(
        &mut self,
        kind: PasteKind,
        transport: &T,
        focused_widget: Option<Id>,
        pos: Option<Pos2>,
        target_layer: Option<LayerId>,
        waker: CompletionWaker,
    ) -> Result<(), PasteError> {
        let target_window = self.target_window.ok_or_else(|| {
            PasteError::new(
                PasteErrorKind::Disconnected,
                "the state has no native window for this clipboard request",
            )
        })?;
        let generation = self.generation;
        let handle = transport.begin_load(kind, target_window, generation, waker)?;
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1);
        self.pending.insert(
            request_id,
            PendingRequest::Raw(PendingPaste {
                common: PendingCommon {
                    viewport_id: self.viewport_id,
                    target_window,
                    generation,
                    focused_widget,
                    allowed_focus_changes: u8::from(kind == PasteKind::Primary),
                    target_layer,
                    pos,
                },
                handle,
            }),
        );
        Ok(())
    }

    /// Take every completed request exactly once, in request order.
    ///
    /// A request is delivered only if this is still its window and its input generation is
    /// current. A keyboard paste also requires its focused widget to be unchanged; a
    /// middle-click PRIMARY paste tolerates its own click's focus move exactly once and requires
    /// the captured position to still resolve to the same egui layer. Stale requests are
    /// cancelled and dropped without an outcome. This must run only from the owning state's
    /// input preparation.
    pub(crate) fn drain_with_layers(
        &mut self,
        current_window: WindowId,
        current_focused_widget: Option<Id>,
        current_layer_at: impl Fn(Pos2) -> Option<LayerId>,
    ) -> Vec<PasteOutcome> {
        let mut outcomes = Vec::new();
        let request_ids: Vec<u64> = self.pending.keys().copied().collect();

        for request_id in request_ids {
            let Some(request) = self.pending.get_mut(&request_id) else {
                continue;
            };

            let is_current = Self::request_is_current(
                request.common_mut(),
                self.viewport_id,
                self.target_window,
                self.generation,
                current_window,
                current_focused_widget,
                &current_layer_at,
            );

            if !is_current {
                if let Some(request) = self.pending.remove(&request_id) {
                    request.cancel();
                }
                continue;
            }

            match request {
                PendingRequest::Raw(request) => {
                    let Some(result) = request.handle.try_take() else {
                        continue;
                    };
                    let pos = request.common.pos;
                    let Some(_request) = self.pending.remove(&request_id) else {
                        continue;
                    };
                    let result = result.and_then(|content| match content {
                        PasteContent::Text(text) => Ok(text.to_string()),
                        PasteContent::ImagePng(_) => Err(PasteError::new(
                            PasteErrorKind::NotOffered,
                            "PRIMARY does not carry images",
                        )),
                    });
                    outcomes.push(match pos {
                        Some(pos) => PasteOutcome::Primary { pos, result },
                        None => PasteOutcome::Regular(result),
                    });
                }
                PendingRequest::Regular(request) => {
                    let Some(result) = request.composed.try_take_terminal() else {
                        continue;
                    };
                    let Some(_request) = self.pending.remove(&request_id) else {
                        continue;
                    };
                    outcomes.push(match result {
                        Ok(RegularPasteResult::Text(text)) => {
                            PasteOutcome::Regular(Ok(text.to_string()))
                        }
                        Ok(RegularPasteResult::Image(image)) => PasteOutcome::RegularImage(image),
                        Err(error) => PasteOutcome::Regular(Err(error)),
                    });
                }
            }
        }

        outcomes
    }

    fn request_is_current(
        common: &mut PendingCommon,
        viewport_id: ViewportId,
        bridge_target: Option<WindowId>,
        bridge_generation: u64,
        current_window: WindowId,
        current_focused_widget: Option<Id>,
        current_layer_at: &impl Fn(Pos2) -> Option<LayerId>,
    ) -> bool {
        let mut is_current = common.viewport_id == viewport_id
            && common.target_window == current_window
            && bridge_target == Some(current_window)
            && common.generation == bridge_generation;

        if is_current && common.focused_widget != current_focused_widget {
            if common.allowed_focus_changes == 0 {
                is_current = false;
            } else {
                common.allowed_focus_changes -= 1;
                common.focused_widget = current_focused_widget;
            }
        }

        if is_current
            && let Some(pos) = common.pos
            && current_layer_at(pos) != common.target_layer
        {
            is_current = false;
        }

        is_current
    }

    /// Test-facing convenience wrapper: no egui layers are consulted.
    #[cfg(test)]
    pub(crate) fn drain(
        &mut self,
        current_window: WindowId,
        current_focused_widget: Option<Id>,
    ) -> Vec<PasteOutcome> {
        self.drain_with_layers(current_window, current_focused_widget, |_| None)
    }

    fn cancel_kind(&mut self, kind: PasteKind) {
        let request_ids: Vec<u64> = self
            .pending
            .iter()
            .filter(|(_, request)| request.kind() == kind)
            .map(|(request_id, _)| *request_id)
            .collect();
        for request_id in request_ids {
            if let Some(request) = self.pending.remove(&request_id) {
                request.cancel();
            }
        }
    }

    fn cancel_all(&mut self) {
        self.deferred_primary = None;
        for request in core::mem::take(&mut self.pending).into_values() {
            request.cancel();
        }
    }
}

impl Drop for PasteBridge {
    fn drop(&mut self) {
        // Raw handles and composed requests are cancelled without waking the caller.
        self.cancel_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use egui::mutex::Mutex;
    use std::collections::HashSet;

    #[derive(Default)]
    struct MockSlot {
        result: Mutex<Option<Result<PasteContent, PasteError>>>,
        waker: Mutex<Option<CompletionWaker>>,
        cancelled: AtomicUsize,
        completed: AtomicUsize,
    }

    impl MockSlot {
        fn complete(&self, result: Result<PasteContent, PasteError>) {
            if self.cancelled.load(Ordering::Acquire) != 0
                || self.completed.load(Ordering::Acquire) != 0
            {
                return;
            }
            self.completed.store(1, Ordering::Release);
            *self.result.lock() = Some(result);
            let waker = self.waker.lock().take();
            if let Some(waker) = waker {
                waker();
            }
        }
    }

    struct MockTransfer(Arc<MockSlot>);

    impl PasteTransfer for MockTransfer {
        fn try_take(&self) -> Option<Result<PasteContent, PasteError>> {
            if self.0.cancelled.load(Ordering::Acquire) != 0 {
                return None;
            }
            self.0.result.lock().take()
        }

        fn cancel(&self) {
            self.0.cancelled.fetch_add(1, Ordering::AcqRel);
            self.0.result.lock().take();
            self.0.waker.lock().take();
        }
    }

    struct MockPng(Vec<u8>);

    impl EncodedPng for MockPng {
        fn as_bytes(&self) -> &[u8] {
            &self.0
        }

        fn len(&self) -> usize {
            self.0.len()
        }
    }

    type MockCompletion = Box<dyn FnOnce(Result<DecodedPasteImage, PasteError>) + Send>;

    #[derive(Default)]
    struct MockDecoder {
        calls: AtomicUsize,
        cancellations: Arc<AtomicUsize>,
        pending: Mutex<Vec<MockCompletion>>,
    }

    impl MockDecoder {
        fn calls(&self) -> usize {
            self.calls.load(Ordering::Acquire)
        }

        fn cancellations(&self) -> usize {
            self.cancellations.load(Ordering::Acquire)
        }

        fn complete(&self, result: Result<egui::ColorImage, PasteError>) {
            let completions = core::mem::take(&mut *self.pending.lock());
            let result = result.map(Arc::new);
            for completion in completions {
                completion(result.as_ref().map_or_else(
                    |error| Err(error.clone()),
                    |image| Ok(DecodedPasteImage::new(image.as_ref().clone(), Box::new(()))),
                ));
            }
        }
    }

    struct MockCancellation(Arc<AtomicUsize>);

    impl JobCancellation for MockCancellation {
        fn cancel(&self) {
            self.0.fetch_add(1, Ordering::AcqRel);
        }
    }

    impl ImagePasteDecoder for MockDecoder {
        fn decode(
            &self,
            _png: Arc<dyn EncodedPng>,
            _deadline: Instant,
            completion: Box<dyn FnOnce(Result<DecodedPasteImage, PasteError>) + Send + 'static>,
        ) -> Option<Arc<dyn JobCancellation>> {
            self.calls.fetch_add(1, Ordering::AcqRel);
            self.pending.lock().push(completion);
            Some(Arc::new(MockCancellation(Arc::clone(&self.cancellations))))
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    struct RecordedClaim {
        kind: PasteKind,
        text: String,
        token: u64,
        target_window: WindowId,
        generation: u64,
    }

    struct MockTransport {
        loads: Mutex<Vec<Arc<MockSlot>>>,
        current_token: AtomicU64,
        consumed: Mutex<HashSet<(u64, PasteKind)>>,
        claims: Mutex<Vec<RecordedClaim>>,
        deny_primary: core::sync::atomic::AtomicBool,
        origin_calls: AtomicUsize,
        deny_origin: core::sync::atomic::AtomicBool,
        /// Result delivered synchronously from `begin_load`, before the handle is returned.
        immediate: Mutex<Option<Result<PasteContent, PasteError>>>,
    }

    impl Default for MockTransport {
        fn default() -> Self {
            Self {
                loads: Mutex::new(Vec::new()),
                // `input_origin` floors a never-bumped counter at 1; start there so the first
                // explicit `new_input_turn()` really produces a different serial.
                current_token: AtomicU64::new(1),
                consumed: Mutex::new(HashSet::new()),
                claims: Mutex::new(Vec::new()),
                deny_primary: core::sync::atomic::AtomicBool::new(false),
                origin_calls: AtomicUsize::new(0),
                deny_origin: core::sync::atomic::AtomicBool::new(false),
                immediate: Mutex::new(None),
            }
        }
    }

    impl MockTransport {
        fn new_input_turn(&self) {
            self.current_token.fetch_add(1, Ordering::AcqRel);
        }

        fn slot(&self, index: usize) -> Arc<MockSlot> {
            Arc::clone(&self.loads.lock()[index])
        }

        fn claims(&self) -> Vec<RecordedClaim> {
            core::mem::take(&mut *self.claims.lock())
        }

        fn deny_primary(&self, deny: bool) {
            self.deny_primary.store(deny, Ordering::Release);
        }

        fn deny_origin(&self, deny: bool) {
            self.deny_origin.store(deny, Ordering::Release);
        }

        fn origin_calls(&self) -> usize {
            self.origin_calls.load(Ordering::Acquire)
        }
    }

    impl ClipboardTransport for MockTransport {
        type ClaimToken = u64;

        fn begin_load(
            &self,
            kind: PasteKind,
            _target_window: WindowId,
            _caller_generation: u64,
            waker: CompletionWaker,
        ) -> Result<Box<dyn PasteTransfer>, PasteError> {
            if kind == PasteKind::Primary && self.deny_primary.load(Ordering::Acquire) {
                return Err(PasteError::new(
                    PasteErrorKind::InvalidInput,
                    "the active seat has not focused the target yet",
                ));
            }
            let slot = Arc::new(MockSlot::default());
            *slot.waker.lock() = Some(Arc::clone(&waker));
            self.loads.lock().push(Arc::clone(&slot));
            if let Some(result) = self.immediate.lock().take() {
                slot.complete(result);
            }
            Ok(Box::new(MockTransfer(slot)))
        }

        fn input_origin(&self, _target_window: WindowId) -> Result<Self::ClaimToken, PasteError> {
            self.origin_calls.fetch_add(1, Ordering::AcqRel);
            if self.deny_origin.load(Ordering::Acquire) {
                return Err(PasteError::new(
                    PasteErrorKind::Busy,
                    "no fresh input origin",
                ));
            }
            Ok(self.current_token.load(Ordering::Acquire).max(1))
        }

        fn claim(
            &self,
            kind: PasteKind,
            text: Arc<str>,
            token: Self::ClaimToken,
            target_window: WindowId,
            caller_generation: u64,
        ) -> Result<(), PasteError> {
            if !self.consumed.lock().insert((token, kind)) {
                return Err(PasteError::new(
                    PasteErrorKind::InvalidInput,
                    "the input origin was already consumed in this selection family",
                ));
            }
            self.claims.lock().push(RecordedClaim {
                kind,
                text: text.to_string(),
                token,
                target_window,
                generation: caller_generation,
            });
            Ok(())
        }
    }

    fn bridge(viewport: u64, window: u64) -> PasteBridge {
        let mut bridge = PasteBridge::new(ViewportId::from_hash_of(("test-viewport", viewport)));
        bridge.set_target_window(WindowId::from(window));
        bridge
    }

    fn noop_waker() -> CompletionWaker {
        Arc::new(|| {})
    }

    fn counting_waker(count: &Arc<AtomicUsize>) -> CompletionWaker {
        let count = Arc::clone(count);
        Arc::new(move || {
            count.fetch_add(1, Ordering::AcqRel);
        })
    }

    #[test]
    fn delayed_regular_completion_is_delivered_once_after_the_wake() {
        let transport = MockTransport::default();
        let mut bridge = bridge(1, 1);
        let focused = Some(Id::unique("field"));
        let wakes = Arc::new(AtomicUsize::new(0));
        bridge
            .begin_regular(&transport, focused, counting_waker(&wakes))
            .unwrap();

        assert!(bridge.drain(WindowId::from(1), focused).is_empty());
        assert_eq!(wakes.load(Ordering::Acquire), 0);

        transport
            .slot(0)
            .complete(Ok(PasteContent::Text(Arc::from("hello"))));
        assert_eq!(wakes.load(Ordering::Acquire), 1);

        let outcomes = bridge.drain(WindowId::from(1), focused);
        assert_eq!(outcomes.len(), 1);
        match &outcomes[0] {
            PasteOutcome::Regular(Ok(text)) => assert_eq!(text, "hello"),
            other => panic!("unexpected outcome: {other:?}"),
        }
        assert!(bridge.drain(WindowId::from(1), focused).is_empty());
        assert_eq!(wakes.load(Ordering::Acquire), 1);

        transport
            .slot(0)
            .complete(Ok(PasteContent::Text(Arc::from("late"))));
        assert_eq!(wakes.load(Ordering::Acquire), 1);
    }

    #[test]
    fn delayed_primary_completion_keeps_the_requested_position() {
        let transport = MockTransport::default();
        let mut bridge = bridge(1, 1);
        let focused = Some(Id::unique("field"));
        let wakes = Arc::new(AtomicUsize::new(0));
        bridge
            .begin_primary(
                &transport,
                focused,
                Pos2::new(12.0, 34.0),
                None,
                counting_waker(&wakes),
            )
            .unwrap();

        transport
            .slot(0)
            .complete(Ok(PasteContent::Text(Arc::from("primary"))));
        let outcomes = bridge.drain(WindowId::from(1), focused);
        assert_eq!(outcomes.len(), 1);
        match &outcomes[0] {
            PasteOutcome::Primary { pos, result } => {
                assert_eq!(*pos, Pos2::new(12.0, 34.0));
                assert_eq!(result.as_ref().unwrap(), "primary");
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
    }

    #[test]
    fn two_targets_route_delayed_completions_to_their_own_state() {
        let transport = MockTransport::default();
        let mut first = bridge(1, 1);
        let mut second = bridge(2, 2);
        let first_focus = Some(Id::unique("first"));
        let second_focus = Some(Id::unique("second"));
        let first_wakes = Arc::new(AtomicUsize::new(0));
        let second_wakes = Arc::new(AtomicUsize::new(0));

        first
            .begin_regular(&transport, first_focus, counting_waker(&first_wakes))
            .unwrap();
        second
            .begin_primary(
                &transport,
                second_focus,
                Pos2::new(1.0, 2.0),
                None,
                counting_waker(&second_wakes),
            )
            .unwrap();

        transport
            .slot(0)
            .complete(Ok(PasteContent::Text(Arc::from("first"))));
        let first_outcomes = first.drain(WindowId::from(1), first_focus);
        assert_eq!(first_outcomes.len(), 1);
        assert!(second.drain(WindowId::from(2), second_focus).is_empty());

        transport
            .slot(1)
            .complete(Ok(PasteContent::Text(Arc::from("second"))));
        assert!(first.drain(WindowId::from(1), first_focus).is_empty());
        let second_outcomes = second.drain(WindowId::from(2), second_focus);
        assert_eq!(second_outcomes.len(), 1);
        assert_eq!(first_wakes.load(Ordering::Acquire), 1);
        assert_eq!(second_wakes.load(Ordering::Acquire), 1);
    }

    #[test]
    fn edit_generation_and_focus_change_drop_a_completed_paste() {
        for invalidation in [InputKind::KeyPress, InputKind::Edit, InputKind::Focus] {
            let transport = MockTransport::default();
            let mut bridge = bridge(1, 1);
            let focused = Some(Id::unique("field"));
            bridge
                .begin_regular(&transport, focused, noop_waker())
                .unwrap();

            bridge.note_input(invalidation);
            transport
                .slot(0)
                .complete(Ok(PasteContent::Text(Arc::from("stale"))));
            assert!(bridge.drain(WindowId::from(1), focused).is_empty());
        }

        let transport = MockTransport::default();
        let mut bridge = bridge(1, 1);
        let focused = Some(Id::unique("field"));
        bridge
            .begin_regular(&transport, focused, noop_waker())
            .unwrap();
        transport
            .slot(0)
            .complete(Ok(PasteContent::Text(Arc::from("stale focus"))));
        assert!(
            bridge
                .drain(WindowId::from(1), Some(Id::unique("other")))
                .is_empty()
        );
    }

    #[test]
    fn primary_paste_survives_the_focus_change_of_its_own_middle_click() {
        let transport = MockTransport::default();
        let mut bridge = bridge(1, 1);
        let before_click = Some(Id::unique("before"));
        let after_click = Some(Id::unique("clicked"));
        bridge
            .begin_primary(
                &transport,
                before_click,
                Pos2::new(3.0, 4.0),
                None,
                noop_waker(),
            )
            .unwrap();

        // The same pointer press can move widget focus before the transfer completes.
        transport
            .slot(0)
            .complete(Ok(PasteContent::Text(Arc::from("middle"))));
        let outcomes = bridge.drain(WindowId::from(1), after_click);
        assert_eq!(outcomes.len(), 1);
    }

    #[test]
    fn close_and_target_replacement_cancel_pending_reads() {
        let transport = MockTransport::default();
        let mut bridge = bridge(1, 1);
        let focused = Some(Id::unique("field"));
        let wakes = Arc::new(AtomicUsize::new(0));
        bridge
            .begin_regular(&transport, focused, counting_waker(&wakes))
            .unwrap();

        bridge.note_input(InputKind::Close);
        transport
            .slot(0)
            .complete(Ok(PasteContent::Text(Arc::from("after close"))));
        assert!(bridge.drain(WindowId::from(1), focused).is_empty());
        assert_eq!(wakes.load(Ordering::Acquire), 0);

        bridge
            .begin_regular(&transport, focused, counting_waker(&wakes))
            .unwrap();
        bridge.set_target_window(WindowId::from(2));
        transport
            .slot(1)
            .complete(Ok(PasteContent::Text(Arc::from("after replacement"))));
        assert!(bridge.drain(WindowId::from(2), focused).is_empty());
        assert_eq!(wakes.load(Ordering::Acquire), 0);
    }

    #[test]
    fn key_or_pointer_release_and_repaint_do_not_cancel_a_paste() {
        let transport = MockTransport::default();
        let mut bridge = bridge(1, 1);
        let focused = Some(Id::unique("field"));
        bridge
            .begin_regular(&transport, focused, noop_waker())
            .unwrap();

        bridge.note_input(InputKind::NonInvalidating);
        transport
            .slot(0)
            .complete(Ok(PasteContent::Text(Arc::from("release is fine"))));
        let outcomes = bridge.drain(WindowId::from(1), focused);
        assert_eq!(outcomes.len(), 1);
    }

    #[test]
    fn present_empty_success_is_distinct_from_absent_and_failed_transfers() {
        let transport = MockTransport::default();
        let mut bridge = bridge(1, 1);
        let focused = Some(Id::unique("field"));
        for _ in 0..3 {
            bridge
                .begin_regular(&transport, focused, noop_waker())
                .unwrap();
        }

        transport
            .slot(0)
            .complete(Ok(PasteContent::Text(Arc::from(""))));
        transport
            .slot(1)
            .complete(Err(PasteError::new(PasteErrorKind::Absent, "no offer")));
        transport.slot(2).complete(Err(PasteError::new(
            PasteErrorKind::Interrupted,
            "owner changed",
        )));

        let outcomes = bridge.drain(WindowId::from(1), focused);
        assert_eq!(outcomes.len(), 3);
        match &outcomes[0] {
            PasteOutcome::Regular(Ok(text)) => assert!(text.is_empty()),
            other => panic!("present empty must stay a successful text: {other:?}"),
        }
        match &outcomes[1] {
            PasteOutcome::Regular(Err(error)) => {
                assert_eq!(error.kind(), PasteErrorKind::Absent);
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
        match &outcomes[2] {
            PasteOutcome::Regular(Err(error)) => {
                assert_eq!(error.kind(), PasteErrorKind::Interrupted);
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
    }

    #[test]
    fn captured_claim_token_is_used_instead_of_a_later_origin() {
        let transport = MockTransport::default();
        let target = WindowId::from(1);
        transport.new_input_turn(); // token 2: the input that asked for the copy
        let captured = transport.input_origin(target).unwrap();
        transport.new_input_turn(); // token 3: a later, unrelated input turn

        claim(
            &transport,
            PasteKind::Regular,
            "captured",
            captured,
            target,
            7,
        )
        .unwrap();
        let claims = transport.claims();
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].token, 2, "the triggering input token must win");
    }

    #[test]
    fn captured_origin_is_cloned_for_regular_and_primary_without_fallback() {
        let transport = MockTransport::default();
        let target = WindowId::from(1);
        transport.new_input_turn();
        let captured = capture_claim_origin(&transport, target, 4);
        assert_eq!(transport.origin_calls(), 1);

        let origins = resolve_batch_claim_origins(
            &transport,
            target,
            4,
            captured,
            Some(RegularClaimOriginPolicy::AllowFreshFallback),
            true,
        );
        assert_eq!(transport.origin_calls(), 1, "no fallback after a capture");
        assert_eq!(origins.regular.unwrap().unwrap(), 2);
        assert_eq!(origins.primary.unwrap().unwrap(), 2);
    }

    #[test]
    fn stale_capture_fails_both_families_without_latest_origin_lookup() {
        let transport = MockTransport::default();
        let target = WindowId::from(1);
        transport.new_input_turn();
        let captured = capture_claim_origin(&transport, target, 4);
        assert_eq!(transport.origin_calls(), 1);

        // IME/focus or another invalidating event advanced the batch generation without a
        // new claimable pointer/keyboard capture.
        let origins = resolve_batch_claim_origins(
            &transport,
            target,
            5,
            captured,
            Some(RegularClaimOriginPolicy::AllowFreshFallback),
            true,
        );
        assert_eq!(transport.origin_calls(), 1, "stale must not fall back");
        assert!(matches!(
            origins.regular.unwrap(),
            Err(ClaimOriginError::StaleCapture {
                expected_generation: 5,
                captured_generation: 4,
            })
        ));
        assert!(matches!(
            origins.primary.unwrap(),
            Err(ClaimOriginError::StaleCapture {
                expected_generation: 5,
                captured_generation: 4,
            })
        ));
    }

    #[test]
    fn failed_capture_fails_both_families_without_latest_origin_lookup() {
        let transport = MockTransport::default();
        transport.deny_origin(true);
        let target = WindowId::from(1);
        let captured = capture_claim_origin(&transport, target, 9);
        assert_eq!(transport.origin_calls(), 1);
        assert_eq!(
            captured,
            CapturedClaimOrigin::<u64>::Failed {
                generation: 9,
                kind: io::ErrorKind::WouldBlock,
            }
        );

        let origins = resolve_batch_claim_origins(
            &transport,
            target,
            9,
            captured,
            Some(RegularClaimOriginPolicy::AllowFreshFallback),
            true,
        );
        assert_eq!(transport.origin_calls(), 1, "failed must not fall back");
        let regular = origins.regular.unwrap().unwrap_err();
        assert!(matches!(
            &regular,
            ClaimOriginError::CaptureFailed {
                expected_generation: 9,
                captured_generation: 9,
                kind: io::ErrorKind::WouldBlock,
            }
        ));
        assert_eq!(
            io::Error::from(regular).kind(),
            io::ErrorKind::WouldBlock,
            "the capture error kind is preserved for diagnostics"
        );
        assert!(matches!(
            origins.primary.unwrap(),
            Err(ClaimOriginError::CaptureFailed {
                expected_generation: 9,
                captured_generation: 9,
                kind: io::ErrorKind::WouldBlock,
            })
        ));
    }

    #[test]
    fn absent_text_and_primary_share_one_synchronous_fallback_token() {
        let transport = MockTransport::default();
        let target = WindowId::from(1);
        let origins = resolve_batch_claim_origins(
            &transport,
            target,
            7,
            CapturedClaimOrigin::Absent,
            Some(RegularClaimOriginPolicy::AllowFreshFallback),
            true,
        );
        assert_eq!(transport.origin_calls(), 1, "one lookup for both families");
        assert_eq!(origins.regular.unwrap().unwrap(), 1);
        assert_eq!(origins.primary.unwrap().unwrap(), 1);
    }

    #[test]
    fn absent_image_refuses_while_primary_may_use_one_fallback() {
        let transport = MockTransport::default();
        let target = WindowId::from(1);
        let origins = resolve_batch_claim_origins(
            &transport,
            target,
            7,
            CapturedClaimOrigin::Absent,
            Some(RegularClaimOriginPolicy::RequireCapturedOrigin),
            true,
        );
        assert_eq!(transport.origin_calls(), 1, "only PRIMARY may fall back");
        assert!(matches!(
            origins.regular.unwrap(),
            Err(ClaimOriginError::ImageRequiresCapturedOrigin {
                expected_generation: 7,
            })
        ));
        assert_eq!(origins.primary.unwrap().unwrap(), 1);
    }

    #[test]
    fn absent_image_never_uses_a_fallback() {
        let transport = MockTransport::default();
        let target = WindowId::from(1);
        let origins = resolve_batch_claim_origins(
            &transport,
            target,
            7,
            CapturedClaimOrigin::Absent,
            Some(RegularClaimOriginPolicy::RequireCapturedOrigin),
            false,
        );
        assert_eq!(transport.origin_calls(), 0);
        assert!(matches!(
            origins.regular.unwrap(),
            Err(ClaimOriginError::ImageRequiresCapturedOrigin {
                expected_generation: 7,
            })
        ));
        assert!(origins.primary.is_none());
    }

    #[test]
    fn no_clipboard_winner_consumes_no_origin() {
        let transport = MockTransport::default();
        let origins = resolve_batch_claim_origins(
            &transport,
            WindowId::from(1),
            3,
            CapturedClaimOrigin::Absent,
            None,
            false,
        );
        assert_eq!(transport.origin_calls(), 0);
        assert!(origins.regular.is_none());
        assert!(origins.primary.is_none());
    }

    #[test]
    fn fallback_failure_is_explicit_and_uses_no_other_token() {
        let transport = MockTransport::default();
        transport.deny_origin(true);
        let origins = resolve_batch_claim_origins(
            &transport,
            WindowId::from(1),
            3,
            CapturedClaimOrigin::Absent,
            Some(RegularClaimOriginPolicy::AllowFreshFallback),
            true,
        );
        assert_eq!(transport.origin_calls(), 1);
        assert!(matches!(
            origins.regular.unwrap(),
            Err(ClaimOriginError::FreshOriginFailed {
                kind: io::ErrorKind::WouldBlock,
                ..
            })
        ));
        assert!(matches!(
            origins.primary.unwrap(),
            Err(ClaimOriginError::FreshOriginFailed {
                kind: io::ErrorKind::WouldBlock,
                ..
            })
        ));
    }

    #[test]
    fn cloned_batch_token_remains_single_use_per_family() {
        let transport = MockTransport::default();
        let target = WindowId::from(1);
        let mut origins = resolve_batch_claim_origins(
            &transport,
            target,
            3,
            CapturedClaimOrigin::Absent,
            Some(RegularClaimOriginPolicy::AllowFreshFallback),
            true,
        );
        let regular = origins.regular.take().unwrap().unwrap();
        let primary = origins.primary.take().unwrap().unwrap();
        assert_eq!(regular, primary);
        claim(&transport, PasteKind::Regular, "copy", regular, target, 3).unwrap();
        claim(
            &transport,
            PasteKind::Primary,
            "primary",
            primary,
            target,
            3,
        )
        .unwrap();
        let claims = transport.claims();
        assert_eq!(claims.len(), 2);
        assert_eq!(claims[0].token, claims[1].token);
        assert_eq!(claims[0].kind, PasteKind::Regular);
        assert_eq!(claims[1].kind, PasteKind::Primary);
    }

    #[test]
    fn focus_gain_keeps_a_pending_primary_but_cancels_a_regular_paste() {
        let transport = MockTransport::default();
        let mut bridge = bridge(1, 1);
        let focused = Some(Id::unique("field"));
        bridge
            .begin_regular(&transport, focused, noop_waker())
            .unwrap();
        bridge
            .begin_primary(&transport, focused, Pos2::new(5.0, 6.0), None, noop_waker())
            .unwrap();

        // Click-to-focus compositors may deliver the keyboard focus gain after the pointer press.
        bridge.note_input(InputKind::FocusGained);

        transport
            .slot(0)
            .complete(Ok(PasteContent::Text(Arc::from("keyboard"))));
        transport
            .slot(1)
            .complete(Ok(PasteContent::Text(Arc::from("middle"))));
        let outcomes = bridge.drain(WindowId::from(1), Some(Id::unique("clicked")));
        assert_eq!(outcomes.len(), 1, "only the PRIMARY read survives");
        match &outcomes[0] {
            PasteOutcome::Primary { result, .. } => {
                assert_eq!(result.as_ref().unwrap(), "middle");
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
    }

    #[test]
    fn primary_tolerates_exactly_one_focus_move() {
        let transport = MockTransport::default();
        let mut bridge = bridge(1, 1);
        let before_click = Some(Id::unique("before"));
        let after_click = Some(Id::unique("clicked"));
        bridge
            .begin_primary(
                &transport,
                before_click,
                Pos2::new(1.0, 1.0),
                None,
                noop_waker(),
            )
            .unwrap();

        // The click's own focus move is accepted and consumes the single allowance.
        assert!(bridge.drain(WindowId::from(1), after_click).is_empty());
        // A later focus change in the same generation is target churn and drops the read.
        transport
            .slot(0)
            .complete(Ok(PasteContent::Text(Arc::from("late"))));
        assert!(
            bridge
                .drain(WindowId::from(1), Some(Id::unique("modal")))
                .is_empty()
        );
    }

    #[test]
    fn scroll_and_layer_change_invalidate_only_the_pending_primary() {
        let transport = MockTransport::default();
        let mut bridge = bridge(1, 1);
        let focused = Some(Id::unique("field"));
        let editor = LayerId::new(egui::Order::Middle, Id::unique("editor"));
        let modal = LayerId::new(egui::Order::Foreground, Id::unique("modal"));
        bridge
            .begin_regular(&transport, focused, noop_waker())
            .unwrap();
        bridge
            .begin_primary(
                &transport,
                focused,
                Pos2::new(2.0, 3.0),
                Some(editor),
                noop_waker(),
            )
            .unwrap();

        // A wheel scroll can move the text under a captured middle-click position.
        bridge.note_input(InputKind::Scroll);
        transport
            .slot(0)
            .complete(Ok(PasteContent::Text(Arc::from("regular survives"))));
        transport
            .slot(1)
            .complete(Ok(PasteContent::Text(Arc::from("primary must not land"))));
        let outcomes = bridge.drain_with_layers(WindowId::from(1), focused, |_| Some(editor));
        assert_eq!(outcomes.len(), 1);
        assert!(matches!(outcomes[0], PasteOutcome::Regular(_)));

        // A modal layer under the click position also invalidates a pending PRIMARY.
        bridge
            .begin_primary(
                &transport,
                focused,
                Pos2::new(2.0, 3.0),
                Some(editor),
                noop_waker(),
            )
            .unwrap();
        transport
            .slot(2)
            .complete(Ok(PasteContent::Text(Arc::from("behind a modal"))));
        assert!(
            bridge
                .drain_with_layers(WindowId::from(1), focused, |_| Some(modal))
                .is_empty()
        );
    }

    #[test]
    fn focused_middle_click_is_deferred_then_retried_after_focus_arrives() {
        let transport = MockTransport::default();
        transport.deny_primary(true);
        let mut bridge = bridge(1, 1);
        let focused = Some(Id::unique("field"));
        bridge
            .begin_primary(&transport, focused, Pos2::new(4.0, 5.0), None, noop_waker())
            .unwrap();
        assert!(transport.loads.lock().is_empty(), "no read before focus");

        // The focus event triggers exactly one retry of the same request.
        transport.deny_primary(false);
        bridge.note_input(InputKind::FocusGained);
        bridge.retry_deferred_primary(&transport).unwrap();
        assert_eq!(transport.loads.lock().len(), 1);

        transport
            .slot(0)
            .complete(Ok(PasteContent::Text(Arc::from("middle"))));
        let outcomes = bridge.drain(WindowId::from(1), focused);
        assert_eq!(outcomes.len(), 1);
    }

    #[test]
    fn claims_use_the_current_origin_once_per_selection_family() {
        let transport = MockTransport::default();
        let target = WindowId::from(1);

        // One input turn may set the regular clipboard and PRIMARY once each.
        let regular = transport.input_origin(target).unwrap();
        let primary = transport.input_origin(target).unwrap();
        claim(&transport, PasteKind::Regular, "copy", regular, target, 7).unwrap();
        claim(
            &transport,
            PasteKind::Primary,
            "primary",
            primary,
            target,
            7,
        )
        .unwrap();
        let another = transport.input_origin(target).unwrap();
        let error = claim(&transport, PasteKind::Regular, "again", another, target, 7).unwrap_err();
        assert_eq!(error.kind(), PasteErrorKind::InvalidInput);

        let claims = transport.claims();
        assert_eq!(claims.len(), 2);
        assert_eq!(claims[0].kind, PasteKind::Regular);
        assert_eq!(claims[0].text, "copy");
        assert_eq!(claims[0].target_window, target);
        assert_eq!(claims[0].generation, 7);
        assert_eq!(claims[1].kind, PasteKind::Primary);

        // A fresh triggering input may claim the regular family again.
        transport.new_input_turn();
        let second = transport.input_origin(target).unwrap();
        claim(
            &transport,
            PasteKind::Regular,
            "second turn",
            second,
            target,
            8,
        )
        .unwrap();
        let claims = transport.claims();
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].text, "second turn");
        assert_eq!(claims[0].generation, 8);
    }

    #[test]
    fn claim_batch_captures_once_at_preparation_with_the_current_generation() {
        let transport = MockTransport::default();
        let target = WindowId::from(1);
        let mut batch = ClaimBatch::<u64>::default();

        // Dispatch only records intent; no transport lookup happens yet.
        batch.note_claimable_input();
        assert!(batch.has_pending_input());
        assert_eq!(transport.origin_calls(), 0);

        // A click-to-focus keyboard enter may advance the paste generation before preparation.
        transport.new_input_turn(); // token 2
        let generation = 8;
        batch.prepare(Some(&transport), target, generation);
        assert_eq!(transport.origin_calls(), 1);
        assert!(!batch.has_pending_input());
        assert_eq!(
            batch.prepared(),
            &CapturedClaimOrigin::Captured {
                generation: 8,
                token: 2,
            }
        );

        let origins = resolve_batch_claim_origins(
            &transport,
            target,
            generation,
            batch.take_prepared(),
            Some(RegularClaimOriginPolicy::RequireCapturedOrigin),
            true,
        );
        assert_eq!(transport.origin_calls(), 1);
        assert_eq!(origins.regular.unwrap().unwrap(), 2);
        assert_eq!(origins.primary.unwrap().unwrap(), 2);
    }

    #[test]
    fn empty_output_consumes_the_prepared_token_and_the_next_batch_has_no_origin() {
        let transport = MockTransport::default();
        let target = WindowId::from(1);
        let mut batch = ClaimBatch::<u64>::default();

        batch.note_claimable_input();
        batch.prepare(Some(&transport), target, 4);
        assert!(matches!(
            batch.take_prepared(),
            CapturedClaimOrigin::Captured {
                generation: 4,
                token: 1
            }
        ));
        assert_eq!(
            batch.take_prepared(),
            CapturedClaimOrigin::Absent,
            "the prepared slot is single-use even without a clipboard winner"
        );

        // The output had no clipboard winner. A later programmatic image in a fresh batch must
        // see `Absent`, not the unused token from the key press.
        batch.prepare(Some(&transport), target, 5);
        let origins = resolve_batch_claim_origins(
            &transport,
            target,
            5,
            batch.take_prepared(),
            Some(RegularClaimOriginPolicy::RequireCapturedOrigin),
            false,
        );
        assert_eq!(transport.origin_calls(), 1, "no lookup for an unused batch");
        assert!(matches!(
            origins.regular.unwrap(),
            Err(ClaimOriginError::ImageRequiresCapturedOrigin {
                expected_generation: 5,
            })
        ));
    }

    #[test]
    fn failed_capture_is_not_carried_into_the_next_batch() {
        let transport = MockTransport::default();
        transport.deny_origin(true);
        let target = WindowId::from(1);
        let mut batch = ClaimBatch::<u64>::default();

        batch.note_claimable_input();
        batch.prepare(Some(&transport), target, 9);
        assert_eq!(
            batch.take_prepared(),
            CapturedClaimOrigin::Failed {
                generation: 9,
                kind: io::ErrorKind::WouldBlock,
            }
        );

        // No output consumed the failed state. A new batch without claimable input is `Absent`.
        batch.prepare(Some(&transport), target, 10);
        assert_eq!(batch.take_prepared(), CapturedClaimOrigin::Absent);
    }

    #[test]
    fn claim_batch_has_no_transport_fallback_when_prepared_without_one() {
        let transport = MockTransport::default();
        let target = WindowId::from(1);
        let mut batch = ClaimBatch::<u64>::default();

        batch.note_claimable_input();
        batch.prepare(None::<&MockTransport>, target, 3);
        assert_eq!(transport.origin_calls(), 0);
        assert_eq!(batch.take_prepared(), CapturedClaimOrigin::Absent);
    }

    #[test]
    fn rebind_and_hide_invalidate_pending_and_prepared_claims() {
        let transport = MockTransport::default();
        let target = WindowId::from(1);
        let mut batch = ClaimBatch::<u64>::default();

        batch.note_claimable_input();
        batch.prepare(Some(&transport), target, 1);
        batch.invalidate();
        assert!(!batch.has_pending_input());
        assert_eq!(batch.take_prepared(), CapturedClaimOrigin::Absent);

        batch.note_claimable_input();
        batch.invalidate();
        batch.prepare(Some(&transport), target, 2);
        assert_eq!(
            batch.take_prepared(),
            CapturedClaimOrigin::Absent,
            "invalidation also drops an unprepared pending trigger"
        );
        assert_eq!(
            transport.origin_calls(),
            1,
            "only the prepared batch looked up"
        );
    }

    #[test]
    fn a_cancelled_transfer_never_wakes_or_yields_a_result() {
        let transport = MockTransport::default();
        let mut bridge = bridge(1, 1);
        let focused = Some(Id::unique("field"));
        let wakes = Arc::new(AtomicUsize::new(0));
        bridge
            .begin_regular(&transport, focused, counting_waker(&wakes))
            .unwrap();

        bridge.note_input(InputKind::Close);
        transport
            .slot(0)
            .complete(Ok(PasteContent::Text(Arc::from("late"))));
        assert_eq!(wakes.load(Ordering::Acquire), 0);
        assert!(bridge.drain(WindowId::from(1), focused).is_empty());
    }
    fn sample_image() -> egui::ColorImage {
        egui::ColorImage::new([1, 1], vec![egui::Color32::RED])
    }

    #[test]
    fn raw_image_completion_submits_decode_without_ui_poll_and_wakes_once() {
        let transport = MockTransport::default();
        let decoder = Arc::new(MockDecoder::default());
        let mut bridge = bridge(1, 1);
        bridge.set_image_decoder(Some(Arc::clone(&decoder) as Arc<dyn ImagePasteDecoder>));
        let focused = Some(Id::unique("field"));
        let wakes = Arc::new(AtomicUsize::new(0));
        bridge
            .begin_regular(&transport, focused, counting_waker(&wakes))
            .unwrap();

        transport
            .slot(0)
            .complete(Ok(PasteContent::ImagePng(Arc::new(MockPng(vec![
                0x89, b'P', b'N', b'G',
            ])))));
        assert_eq!(decoder.calls(), 1, "raw completion schedules decode");
        assert_eq!(
            wakes.load(Ordering::Acquire),
            0,
            "decode is not terminal yet"
        );
        assert!(bridge.drain(WindowId::from(1), focused).is_empty());

        decoder.complete(Ok(sample_image()));
        assert_eq!(
            wakes.load(Ordering::Acquire),
            1,
            "exactly one terminal wake"
        );

        let outcomes = bridge.drain(WindowId::from(1), focused);
        assert_eq!(outcomes.len(), 1);
        match &outcomes[0] {
            PasteOutcome::RegularImage(decoded) => {
                assert_eq!(decoded.image.size, [1, 1]);
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
        assert!(bridge.drain(WindowId::from(1), focused).is_empty());
        assert_eq!(wakes.load(Ordering::Acquire), 1, "one-shot delivery");
    }

    #[test]
    fn immediate_self_paste_wake_before_handle_attach_still_delivers_text() {
        let transport = MockTransport::default();
        *transport.immediate.lock() = Some(Ok(PasteContent::Text(Arc::from("immediate"))));
        let mut bridge = bridge(1, 1);
        let focused = Some(Id::unique("field"));
        let wakes = Arc::new(AtomicUsize::new(0));
        bridge
            .begin_regular(&transport, focused, counting_waker(&wakes))
            .expect("immediate completion does not fail admission");
        assert_eq!(wakes.load(Ordering::Acquire), 1);

        let outcomes = bridge.drain(WindowId::from(1), focused);
        assert_eq!(outcomes.len(), 1);
        match &outcomes[0] {
            PasteOutcome::Regular(Ok(text)) => assert_eq!(text, "immediate"),
            other => panic!("unexpected outcome: {other:?}"),
        }
    }

    #[test]
    fn cancel_after_raw_image_completion_suppresses_decode_wake_and_result() {
        let transport = MockTransport::default();
        let decoder = Arc::new(MockDecoder::default());
        let mut bridge = bridge(1, 1);
        bridge.set_image_decoder(Some(Arc::clone(&decoder) as Arc<dyn ImagePasteDecoder>));
        let focused = Some(Id::unique("field"));
        let wakes = Arc::new(AtomicUsize::new(0));
        bridge
            .begin_regular(&transport, focused, counting_waker(&wakes))
            .unwrap();

        transport
            .slot(0)
            .complete(Ok(PasteContent::ImagePng(Arc::new(MockPng(vec![1, 2, 3])))));
        assert_eq!(decoder.calls(), 1);
        bridge.note_input(InputKind::Close);
        assert_eq!(
            decoder.cancellations(),
            1,
            "queued decode cancellation propagates through the decoder trait"
        );
        decoder.complete(Ok(sample_image()));

        assert_eq!(wakes.load(Ordering::Acquire), 0, "cancel never wakes");
        assert!(bridge.drain(WindowId::from(1), focused).is_empty());
    }

    #[test]
    fn text_completion_after_delivery_deadline_is_timed_out() {
        let composed = ComposedRegular::new(
            None,
            noop_waker(),
            Instant::now()
                .checked_sub(Duration::from_millis(1))
                .expect("instant is representable"),
        );
        let slot = Arc::new(MockSlot::default());
        *slot.result.lock() = Some(Ok(PasteContent::Text(Arc::from("late"))));
        composed.attach(Box::new(MockTransfer(slot)));

        let result = composed.try_take_terminal().expect("terminal timeout");
        match result {
            Err(error) => assert_eq!(error.kind(), PasteErrorKind::TimedOut),
            Ok(_) => panic!("a late text result must not be delivered"),
        }
        assert!(composed.try_take_terminal().is_none(), "one-shot failure");
    }

    fn ready_text_composed(deadline: Instant) -> ComposedRegular {
        let composed = ComposedRegular::new(None, noop_waker(), deadline);
        let mut state = composed.lock();
        state.phase = ComposedPhase::Ready;
        state.result = Some(Ok(RegularPasteResult::Text(Arc::from("ready"))));
        state.terminal_woken = true;
        drop(state);
        composed
    }

    #[test]
    fn ready_result_drained_before_delivery_deadline_is_delivered() {
        let deadline = Instant::now() + Duration::from_secs(1);
        let composed = ready_text_composed(deadline);
        composed.set_test_now(
            deadline
                .checked_sub(Duration::from_millis(1))
                .expect("instant is representable"),
        );
        match composed.try_take_terminal().expect("ready terminal result") {
            Ok(RegularPasteResult::Text(text)) => assert_eq!(text.as_ref(), "ready"),
            Ok(RegularPasteResult::Image(_)) => panic!("unexpected image result"),
            Err(error) => panic!("unexpected error: {error:?}"),
        }
        assert!(composed.try_take_terminal().is_none(), "one-shot delivery");
    }

    #[test]
    fn ready_result_drained_after_delivery_deadline_is_timed_out() {
        let deadline = Instant::now() + Duration::from_secs(1);
        let composed = ready_text_composed(deadline);
        composed.set_test_now(
            deadline
                .checked_sub(Duration::from_millis(1))
                .expect("instant is representable"),
        );
        assert_eq!(
            composed.lock().phase,
            ComposedPhase::Ready,
            "the result was ready before the deadline"
        );
        composed.set_test_now(deadline + Duration::from_millis(1));
        match composed.try_take_terminal().expect("terminal timeout") {
            Err(error) => assert_eq!(error.kind(), PasteErrorKind::TimedOut),
            Ok(_) => panic!("a late ready result must not be delivered"),
        }
        assert!(composed.try_take_terminal().is_none(), "one-shot failure");
    }

    #[test]
    fn terminal_waker_runs_outside_the_composed_state_lock() {
        let holder: Arc<std::sync::OnceLock<std::sync::Weak<ComposedRegular>>> =
            Arc::new(std::sync::OnceLock::new());
        let observed = Arc::new(AtomicUsize::new(0));
        let waker = {
            let holder = Arc::clone(&holder);
            let observed = Arc::clone(&observed);
            let waker: CompletionWaker = Arc::new(move || {
                if let Some(request) = holder.get().and_then(std::sync::Weak::upgrade) {
                    // This must acquire the same lock. A wake under the lock would deadlock.
                    let result = request.try_take_terminal();
                    observed.store(usize::from(result.is_some()), Ordering::Release);
                }
            });
            waker
        };
        let composed = Arc::new(ComposedRegular::new(
            None,
            waker,
            Instant::now() + Duration::from_mins(1),
        ));
        holder.set(Arc::downgrade(&composed)).ok();

        let slot = Arc::new(MockSlot::default());
        *slot.result.lock() = Some(Ok(PasteContent::Text(Arc::from("no lock"))));
        composed.attach(Box::new(MockTransfer(slot)));
        assert_eq!(
            observed.load(Ordering::Acquire),
            1,
            "the terminal waker observed a delivered result"
        );
    }

    #[test]
    fn image_without_a_decoder_fails_terminal_not_offered_once() {
        let transport = MockTransport::default();
        let mut bridge = bridge(1, 1);
        let focused = Some(Id::unique("field"));
        let wakes = Arc::new(AtomicUsize::new(0));
        bridge
            .begin_regular(&transport, focused, counting_waker(&wakes))
            .unwrap();

        transport
            .slot(0)
            .complete(Ok(PasteContent::ImagePng(Arc::new(MockPng(vec![1, 2, 3])))));
        assert_eq!(wakes.load(Ordering::Acquire), 1);

        let outcomes = bridge.drain(WindowId::from(1), focused);
        assert_eq!(outcomes.len(), 1);
        match &outcomes[0] {
            PasteOutcome::Regular(Err(error)) => {
                assert_eq!(error.kind(), PasteErrorKind::NotOffered);
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
        assert!(bridge.drain(WindowId::from(1), focused).is_empty());
    }
}

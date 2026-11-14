//! [`egui`] bindings for [`winit`](https://github.com/rust-windowing/winit).
//!
//! The library translates winit events to egui, handled copy/paste,
//! updates the cursor, open links clicked in egui, etc.
//!
//! ## Feature flags
#![cfg_attr(feature = "document-features", doc = document_features::document_features!())]
//!

#![expect(clippy::manual_range_contains)]

#[cfg(target_os = "windows")]
use std::collections::HashSet;

#[cfg(feature = "accesskit")]
pub use accesskit_winit;
pub use egui;
#[cfg(feature = "accesskit")]
use egui::accesskit;
use egui::{Pos2, Rect, Theme, Vec2, ViewportBuilder, ViewportCommand, ViewportId, ViewportInfo};
pub use winit;

// TODO(emilk): remove once we are on a winit with https://github.com/rust-windowing/winit/pull/4732
#[cfg(target_os = "macos")]
mod macos_scroll_momentum;

pub mod clipboard;
#[cfg(target_os = "android")]
pub mod android_activity;
#[cfg(target_os = "android")]
pub mod ime;
#[cfg(target_os = "android")]
pub mod system_theme;
mod clipboard_bridge;
#[cfg(all(
    feature = "clipboard-image",
    feature = "clipboard",
    feature = "wayland",
    any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    )
))]
mod clipboard_image;
// Wayland surrounding-text bridge. Winit compiles its Wayland backend for free Unix
// targets (Linux, the BSDs and the rest of winit's `wayland_platform` set), not only Linux.
#[cfg(not(target_arch = "wasm32"))]
mod dropped_file;
mod held_keys;
mod ime_reset;
#[cfg(all(
    feature = "wayland",
    unix,
    not(target_vendor = "apple"),
    not(target_os = "android"),
    not(target_os = "emscripten"),
    not(target_os = "redox"),
))]
mod ime_surrounding;
mod output_selector;
mod pointer_icon;
mod safe_area;
mod window_settings;

pub use window_settings::{WindowGeometry, WindowSettings};

use raw_window_handle::HasDisplayHandle;

#[cfg(not(target_arch = "wasm32"))]
use dropped_file::NativeFile;

use winit::{
    dpi::{PhysicalPosition, PhysicalSize},
    event::ElementState,
    event_loop::ActiveEventLoop,
    window::{CursorGrabMode, CustomCursor, Window, WindowButtons, WindowLevel},
};

pub fn screen_size_in_pixels(window: &Window) -> egui::Vec2 {
    let size = if cfg!(target_os = "ios") {
        // `outer_size` Includes the area behind the "dynamic island".
        // It is up to the eframe user to make sure the dynamic island doesn't cover anything important.
        // That will be easier once https://github.com/rust-windowing/winit/pull/3890 lands
        window.outer_size()
    } else {
        window.inner_size()
    };
    egui::vec2(size.width as f32, size.height as f32)
}

/// Calculate the `pixels_per_point` for a given window, given the current egui zoom factor
pub fn pixels_per_point(egui_ctx: &egui::Context, window: &Window) -> f32 {
    let native_pixels_per_point = window.scale_factor() as f32;
    let egui_zoom_factor = egui_ctx.zoom_factor();
    egui_zoom_factor * native_pixels_per_point
}

// ----------------------------------------------------------------------------

#[must_use]
#[derive(Clone, Copy, Debug, Default)]
pub struct EventResponse {
    /// If true, egui consumed this event, i.e. wants exclusive use of this event
    /// (e.g. a mouse click on an egui window, or entering text into a text field).
    ///
    /// For instance, if you use egui for a game, you should only
    /// pass on the events to your game when [`Self::consumed`] is `false`.
    ///
    /// Note that egui uses `tab` to move focus between elements, so this will always be `true` for tabs.
    pub consumed: bool,

    /// Do we need an egui refresh because of this event?
    pub repaint: bool,
}

// ----------------------------------------------------------------------------

/// Handles the integration between egui and a winit Window.
///
/// Instantiate one of these per viewport/window.
pub struct State {
    /// Shared clone.
    egui_ctx: egui::Context,

    viewport_id: ViewportId,
    start_time: web_time::Instant,
    egui_input: egui::RawInput,

    /// The current modifier state.
    ///
    /// We keep a copy so we can stamp
    /// it onto per-event `modifiers` fields and emit [`egui::Event::ModifiersChanged`].
    modifiers: egui::Modifiers,

    held_keys: held_keys::HeldKeys,

    pointer_pos_in_points: Option<egui::Pos2>,
    any_pointer_button_down: bool,
    current_cursor_icon: Option<egui::CursorIcon>,

    /// Cached `CustomCursor` for the last RGBA bitmap pushed through
    /// `PlatformOutput::cursor_image`. We dedupe by `Arc::as_ptr` so the
    /// integration only re-uploads the bitmap to the OS when the app
    /// switches sprite, not every frame the cursor moves. `usize` is the
    /// raw pointer of the source `Arc<[u8]>` — opaque, only used as a
    /// cache key.
    current_custom_cursor: Option<(usize, CustomCursor)>,

    clipboard: clipboard::Clipboard,

    /// Pending asynchronous clipboard reads for this window.
    clipboard_paste: clipboard_bridge::PasteBridge,

    /// Wakes this viewport when a clipboard transfer completes.
    #[cfg(all(
        feature = "clipboard",
        feature = "wayland",
        any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ),
    ))]
    clipboard_repaint_waker: clipboard_bridge::CompletionWaker,

    /// The last PRIMARY text claimed for one input generation, so repeated output commands do
    /// not re-claim an unchanged selection.
    #[cfg(all(
        feature = "clipboard",
        feature = "wayland",
        any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ),
    ))]
    last_primary_claim: Option<(u64, String)>,

    /// Per-input-batch claim-origin lifecycle.
    ///
    /// Claimable events and explicit copy/cut requests only mark the batch as pending during
    /// dispatch. The final input origin is captured once during [`Self::take_egui_input`], after
    /// any click-to-focus keyboard enter has been processed. Platform output always consumes the
    /// prepared slot, even without a winning clipboard command, so no token survives into a later
    /// batch. This is a batch association, not per-command provenance: when both clipboard
    /// families win from a merged `PlatformOutput`, both validate independent clones of this
    /// token. A stale or failed capture is an explicit error; only `Absent` may use the legacy
    /// latest-fresh fallback, and images never do.
    #[cfg(all(
        feature = "clipboard",
        feature = "wayland",
        any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ),
    ))]
    clipboard_claim_batch: clipboard_bridge::ClaimBatch<winit::platform::wayland::InputOriginToken>,

    /// Clipboard requests or ownership claims rejected since this state was created.
    ///
    /// A rejected claim leaves the previous selection content in place; this counter makes the
    /// otherwise invisible failure observable to the application.
    #[cfg(all(
        feature = "clipboard",
        feature = "wayland",
        any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ),
    ))]
    clipboard_failures: u64,

    /// Dedicated regular-copy epoch. Unrelated typing, IME, pointer release or another
    /// state's PRIMARY claim must not cancel a frozen image copy.
    #[cfg(all(
        feature = "clipboard-image",
        feature = "clipboard",
        feature = "wayland",
        any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ),
    ))]
    copy_epoch: u64,

    /// One in-flight PNG copy encode and its exact winit reservation, if any.
    #[cfg(all(
        feature = "clipboard-image",
        feature = "clipboard",
        feature = "wayland",
        any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ),
    ))]
    pending_image_copy: Option<ImageCopyPending>,

    /// If `true`, mouse inputs will be treated as touches.
    /// Useful for debugging touch support in egui.
    ///
    /// Creates duplicate touches, if real touch inputs are coming.
    simulate_touch_screen: bool,

    /// Is Some(…) when a touch is being translated to a pointer.
    ///
    /// Only one touch will be interpreted as pointer at any time.
    pointer_touch_id: Option<u64>,

    #[cfg(feature = "accesskit")]
    pub accesskit: Option<accesskit_winit::Adapter>,

    allow_ime: bool,
    ime_reset: ime_reset::ResetBridge<winit::window::WindowId>,

    /// See [`Self::set_clipboard_shortcuts`].
    clipboard_shortcuts: bool,

    ime_cursor_rect_px: Option<egui::Rect>,
    old_ime_purpose: egui::IMEPurpose,

    /// Outgoing text-input-v3 surrounding text, cached per window and IME owner.
    #[cfg(all(
        feature = "wayland",
        unix,
        not(target_vendor = "apple"),
        not(target_os = "android"),
        not(target_os = "emscripten"),
        not(target_os = "redox"),
    ))]
    ime_surrounding: ime_surrounding::ImeSurroundingSync,

    /// Durable window-focus and external `IMEAllowed` latch for enabling IME.
    ///
    /// egui keeps widget focus when the window loses focus, so a stale `PlatformOutput::ime`
    /// must not re-enable IME or republish surrounding text while unfocused.
    #[cfg(all(
        feature = "wayland",
        unix,
        not(target_vendor = "apple"),
        not(target_os = "android"),
        not(target_os = "emscripten"),
        not(target_os = "redox"),
    ))]
    ime_gate: ime_surrounding::ImeGate,

    // TODO(emilk): remove once we are on a winit with https://github.com/rust-windowing/winit/pull/4732
    #[cfg(target_os = "macos")]
    scroll_momentum_monitor: Option<macos_scroll_momentum::ScrollMomentumMonitor>,

    /// Used by [`State::try_on_ime_processed_keyboard_input`] to track key
    /// release events that should be filtered out. See comments in that method
    /// for details.
    #[cfg(target_os = "windows")]
    pressed_processed_physical_keys: HashSet<winit::keyboard::PhysicalKey>,
}

/// Classify a winit event for pending-paste invalidation.
///
/// A later edit or focus/selection-changing action cancels a paste in flight. A release of the
/// initiating shortcut, pointer release, repaint and unrelated events do not. A focus gain is
/// special: on a click-to-focus compositor it can be the second half of the same middle-click
/// that started a PRIMARY read.
fn clipboard_input_kind(event: &winit::event::WindowEvent) -> clipboard_bridge::InputKind {
    use crate::clipboard_bridge::InputKind;
    use winit::event::WindowEvent;

    match event {
        WindowEvent::KeyboardInput {
            event,
            is_synthetic,
            ..
        } => clipboard_key_input_kind(event.state, *is_synthetic),
        WindowEvent::MouseInput {
            state: ElementState::Pressed,
            ..
        } => InputKind::PointerPress,
        WindowEvent::Touch(touch) if touch.phase == winit::event::TouchPhase::Started => {
            InputKind::PointerPress
        }
        WindowEvent::Ime(winit::event::Ime::Commit(_)) => InputKind::Edit,
        WindowEvent::Focused(false) => InputKind::Focus,
        WindowEvent::Focused(true) => InputKind::FocusGained,
        WindowEvent::CloseRequested | WindowEvent::Destroyed => InputKind::Close,
        WindowEvent::Occluded(true) => InputKind::Hide,
        WindowEvent::MouseWheel { .. } => InputKind::Scroll,
        _ => InputKind::NonInvalidating,
    }
}

/// Classify a keyboard event without constructing a `winit::event::KeyEvent` (its
/// platform-specific field is private to winit, so this is also the testable seam).
fn clipboard_key_input_kind(
    state: winit::event::ElementState,
    is_synthetic: bool,
) -> clipboard_bridge::InputKind {
    use crate::clipboard_bridge::InputKind;

    if is_synthetic || state != winit::event::ElementState::Pressed {
        // Releasing the paste/copy shortcut must not cancel the request it started.
        InputKind::NonInvalidating
    } else {
        InputKind::KeyPress
    }
}

/// Whether an input action must discard this batch's pending/prepared claim origin.
///
/// A click-to-focus `Focused(true)` is deliberately not invalidating: it can be the second half
/// of the same pointer press that made the batch claimable.
fn clipboard_input_invalidates_claim_batch(kind: clipboard_bridge::InputKind) -> bool {
    matches!(
        kind,
        clipboard_bridge::InputKind::Focus
            | clipboard_bridge::InputKind::Close
            | clipboard_bridge::InputKind::Hide
    )
}

/// Whether this event is an input turn that winit may accept as an ownership claim trigger.
///
/// Keyboard presses and pointer presses/releases make the current input batch claimable. The
/// token is looked up once during input preparation, not at dispatch or platform-output time.
fn clipboard_input_claims_origin(event: &winit::event::WindowEvent) -> bool {
    use winit::event::{ElementState, WindowEvent};

    match event {
        WindowEvent::KeyboardInput {
            event,
            is_synthetic,
            ..
        } => !*is_synthetic && event.state == ElementState::Pressed,
        WindowEvent::MouseInput { .. } => true,
        _ => false,
    }
}

/// Batch origins resolved for output, or a marker that this build/display has no Wayland
/// origin transport. Keeping this type available on every platform avoids cfg-dependent
/// arguments to the command-processing helpers.
#[derive(Default)]
enum OutputClaimOrigins {
    /// No winit Wayland transport is available for this output.
    #[default]
    NotWayland,
    /// One captured-or-fallback origin per selected clipboard family.
    #[cfg(all(
        feature = "clipboard",
        feature = "wayland",
        any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ),
    ))]
    Wayland(clipboard_bridge::BatchClaimOrigins<winit::platform::wayland::InputOriginToken>),
}

/// One family's resolved origin after selection and batch-level resolution.
#[derive(Default)]
enum OutputClaimOrigin {
    /// No winit Wayland transport is available for this output.
    #[default]
    NotWayland,
    /// The exact token or permitted fallback to use for this family.
    #[cfg(all(
        feature = "clipboard",
        feature = "wayland",
        any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ),
    ))]
    Ready(winit::platform::wayland::InputOriginToken),
    /// The family must fail explicitly without consulting another origin.
    #[cfg(all(
        feature = "clipboard",
        feature = "wayland",
        any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ),
    ))]
    Rejected(clipboard_bridge::ClaimOriginError),
}

/// Existing PRIMARY behavior: suppress an unchanged `(input generation, text)` claim.
///
/// This is the explicit exception to the no-cross-batch-deduplication rule. It is deliberately
/// keyed by the input generation, not by a logical batch id; a new input generation may claim the
/// same text again.
fn unchanged_primary_is_suppressed(
    last_claim: Option<&(u64, String)>,
    generation: u64,
    text: &str,
) -> bool {
    last_claim.is_some_and(|(last_generation, last_text)| {
        *last_generation == generation && last_text == text
    })
}

/// Clear the per-window IME latches when a window is destroyed.
///
/// This is deliberately independent of the Wayland-only bridge state: a reused [`State`] on
/// Windows, macOS or X11 must also ask the new window for its cursor area and purpose, and must
/// not skip the first `set_ime_allowed(true)`.
fn reset_ime_window_latches(
    allow_ime: &mut bool,
    ime_cursor_rect_px: &mut Option<egui::Rect>,
    old_ime_purpose: &mut egui::IMEPurpose,
) {
    *allow_ime = false;
    *ime_cursor_rect_px = None;
    *old_ime_purpose = egui::IMEPurpose::Normal;
}

/// One pending asynchronous PNG copy, its exact reserved input origin and delivery deadline.
#[cfg(all(
    feature = "clipboard-image",
    feature = "clipboard",
    feature = "wayland",
    any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    )
))]
struct ImageCopyPending {
    epoch: u64,
    target_window: winit::window::WindowId,
    deadline: std::time::Instant,
    reservation: Option<winit::platform::wayland::ImageClaimReservation>,
    request: clipboard_image::ImageCopyRequest,
}

#[cfg(all(
    feature = "clipboard-image",
    feature = "clipboard",
    feature = "wayland",
    any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    )
))]
impl Drop for ImageCopyPending {
    fn drop(&mut self) {
        // Dropping the state must not let a late encode install ownership.
        self.request.cancel();
        self.reservation.take();
    }
}

impl State {
    /// Construct a new instance
    pub fn new(
        egui_ctx: egui::Context,
        viewport_id: ViewportId,
        display_target: &dyn HasDisplayHandle,
        native_pixels_per_point: Option<f32>,
        theme: Option<winit::window::Theme>,
        max_texture_side: Option<usize>,
    ) -> Self {
        profiling::function_scope!();

        let egui_input = egui::RawInput {
            focused: false, // winit will tell us when we have focus
            ..Default::default()
        };

        // SAFETY: The display handle is obtained from `display_target` which the caller
        // is responsible for keeping alive. Winit display handles remain valid for the
        // duration of the event loop, which outlives any `State` instance.
        #[expect(unsafe_code)]
        let clipboard = unsafe {
            clipboard::Clipboard::new(display_target.display_handle().ok().map(|h| h.as_raw()))
        };

        #[cfg(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        ))]
        let clipboard_repaint_waker = {
            let egui_ctx = egui_ctx.clone();
            std::sync::Arc::new(move || egui_ctx.request_repaint_of(viewport_id))
        };

        let mut slf = Self {
            viewport_id,
            start_time: web_time::Instant::now()
                .checked_sub(web_time::Duration::from_secs_f64(egui_ctx.time()))
                .unwrap_or_else(web_time::Instant::now),
            egui_ctx,
            egui_input,
            modifiers: egui::Modifiers::default(),
            held_keys: held_keys::HeldKeys::default(),
            pointer_pos_in_points: None,
            any_pointer_button_down: false,
            current_cursor_icon: None,
            current_custom_cursor: None,

            clipboard,
            clipboard_paste: clipboard_bridge::PasteBridge::new(viewport_id),
            #[cfg(all(
                feature = "clipboard",
                feature = "wayland",
                any(
                    target_os = "linux",
                    target_os = "dragonfly",
                    target_os = "freebsd",
                    target_os = "netbsd",
                    target_os = "openbsd"
                ),
            ))]
            clipboard_repaint_waker,
            #[cfg(all(
                feature = "clipboard",
                feature = "wayland",
                any(
                    target_os = "linux",
                    target_os = "dragonfly",
                    target_os = "freebsd",
                    target_os = "netbsd",
                    target_os = "openbsd"
                ),
            ))]
            last_primary_claim: None,
            #[cfg(all(
                feature = "clipboard",
                feature = "wayland",
                any(
                    target_os = "linux",
                    target_os = "dragonfly",
                    target_os = "freebsd",
                    target_os = "netbsd",
                    target_os = "openbsd"
                ),
            ))]
            clipboard_claim_batch: clipboard_bridge::ClaimBatch::default(),
            #[cfg(all(
                feature = "clipboard",
                feature = "wayland",
                any(
                    target_os = "linux",
                    target_os = "dragonfly",
                    target_os = "freebsd",
                    target_os = "netbsd",
                    target_os = "openbsd"
                ),
            ))]
            clipboard_failures: 0,
            #[cfg(all(
                feature = "clipboard-image",
                feature = "clipboard",
                feature = "wayland",
                any(
                    target_os = "linux",
                    target_os = "dragonfly",
                    target_os = "freebsd",
                    target_os = "netbsd",
                    target_os = "openbsd"
                ),
            ))]
            copy_epoch: 0,
            #[cfg(all(
                feature = "clipboard-image",
                feature = "clipboard",
                feature = "wayland",
                any(
                    target_os = "linux",
                    target_os = "dragonfly",
                    target_os = "freebsd",
                    target_os = "netbsd",
                    target_os = "openbsd"
                ),
            ))]
            pending_image_copy: None,

            simulate_touch_screen: false,
            pointer_touch_id: None,

            #[cfg(feature = "accesskit")]
            accesskit: None,

            allow_ime: false,
            ime_reset: Default::default(),
            clipboard_shortcuts: true,
            ime_cursor_rect_px: None,
            old_ime_purpose: egui::IMEPurpose::Normal,
            #[cfg(all(
                feature = "wayland",
                unix,
                not(target_vendor = "apple"),
                not(target_os = "android"),
                not(target_os = "emscripten"),
                not(target_os = "redox"),
            ))]
            ime_surrounding: ime_surrounding::ImeSurroundingSync::default(),
            #[cfg(all(
                feature = "wayland",
                unix,
                not(target_vendor = "apple"),
                not(target_os = "android"),
                not(target_os = "emscripten"),
                not(target_os = "redox"),
            ))]
            ime_gate: ime_surrounding::ImeGate::default(),
            #[cfg(target_os = "macos")]
            scroll_momentum_monitor: macos_scroll_momentum::ScrollMomentumMonitor::install(),
            #[cfg(target_os = "windows")]
            pressed_processed_physical_keys: HashSet::new(),
        };

        #[cfg(all(
            feature = "clipboard-image",
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        ))]
        slf.clipboard_paste
            .set_image_decoder(Some(clipboard_image::paste_decoder()));

        slf.egui_input
            .viewports
            .entry(ViewportId::ROOT)
            .or_default()
            .native_pixels_per_point = native_pixels_per_point;
        slf.egui_input.system_theme = theme.map(to_egui_theme);

        if let Some(max_texture_side) = max_texture_side {
            slf.set_max_texture_side(max_texture_side);
        }
        slf
    }

    #[cfg(feature = "accesskit")]
    pub fn init_accesskit<T: From<accesskit_winit::Event> + Send>(
        &mut self,
        event_loop: &ActiveEventLoop,
        window: &Window,
        event_loop_proxy: winit::event_loop::EventLoopProxy<T>,
    ) {
        profiling::function_scope!();

        self.accesskit = Some(accesskit_winit::Adapter::with_event_loop_proxy(
            event_loop,
            window,
            event_loop_proxy,
        ));
    }

    /// Call this once a graphics context has been created to update the maximum texture dimensions
    /// that egui will use.
    pub fn set_max_texture_side(&mut self, max_texture_side: usize) {
        self.egui_input.max_texture_side = Some(max_texture_side);
    }

    /// Fetches text from the clipboard and returns it.
    ///
    /// This is a bounded synchronous compatibility API; on Wayland it can block for up to
    /// winit's transfer deadline. Input handling uses the asynchronous paste bridge instead.
    pub fn clipboard_text(&mut self) -> Option<String> {
        self.clipboard.get()
    }

    /// Fetches an image from the clipboard and returns it, if there is one and the platform
    /// backend supports it. Mirrors [`Self::clipboard_text`] for images.
    ///
    /// This is a synchronous compatibility API for non-Wayland backends. On Wayland authoritative
    /// clipboard ownership belongs to winit, so this method explicitly refuses instead of
    /// opening arboard/smithay or blocking the event loop. The supported Wayland image path is
    /// the regular paste event (`Ctrl+V` or [`Self::request_paste`]), which performs a bounded
    /// combined read and off-thread PNG decode.
    pub fn clipboard_image(&mut self) -> Option<egui::ColorImage> {
        self.clipboard.get_image()
    }

    /// Places the text onto the clipboard.
    ///
    /// This is the synchronous compatibility API without an input-origin token. The egui event
    /// path uses a validated claim on the triggering input turn instead.
    pub fn set_clipboard_text(&mut self, text: String) {
        self.clipboard.set_text(text);
    }

    fn focused_widget_id(&self) -> Option<egui::Id> {
        self.egui_ctx.memory(|memory| memory.focused())
    }

    /// Request a regular-clipboard paste from [`egui::ViewportCommand::RequestPaste`].
    ///
    /// On Wayland this starts the same bounded asynchronous bridge as the paste shortcut. It
    /// never blocks the event path and never performs a synchronous arboard image read. Other
    /// platforms keep their synchronous compatibility behavior.
    pub fn request_paste(&mut self, window: &Window) {
        self.rebind_clipboard_target(window);
        self.begin_regular_clipboard_paste();
    }

    /// Forward an app-requested cut (`ViewportCommand::RequestCut`) through the normal egui path.
    pub fn request_cut(&mut self, _window: &Window) {
        self.note_claimable_clipboard_input();
        self.egui_input.events.push(egui::Event::Cut);
    }

    /// Forward an app-requested copy (`ViewportCommand::RequestCopy`) through the normal egui path.
    pub fn request_copy(&mut self, _window: &Window) {
        self.note_claimable_clipboard_input();
        self.egui_input.events.push(egui::Event::Copy);
    }

    /// Bind the paste bridge and claim batch to this native window.
    ///
    /// A target replacement cancels pending reads and discards any pending/prepared claim origin:
    /// a token captured for one window must never be used by a reused `State` bound elsewhere.
    fn rebind_clipboard_target(&mut self, window: &Window) {
        let rebound = self.clipboard_paste.set_target_window(window.id());
        #[cfg(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        ))]
        if rebound {
            self.clipboard_claim_batch.invalidate();
            self.cancel_pending_image_copy();
        }
        #[cfg(not(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        )))]
        let _ = rebound;
    }

    /// Mark a claimable trigger, `RequestCopy` or `RequestCut` for this input batch.
    ///
    /// The token itself is captured once during [`Self::take_egui_input`], not at dispatch, so
    /// click-to-focus keyboard-enter ordering and later events in the same batch are reflected.
    #[cfg_attr(
        not(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        )),
        expect(clippy::needless_pass_by_ref_mut, clippy::unused_self)
    )]
    fn note_claimable_clipboard_input(&mut self) {
        #[cfg(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        ))]
        self.clipboard_claim_batch.note_claimable_input();
    }

    /// Drop claim-origin state that belongs to a hidden, unfocused or otherwise invalid batch.
    #[cfg_attr(
        not(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        )),
        expect(clippy::needless_pass_by_ref_mut, clippy::unused_self)
    )]
    fn invalidate_clipboard_claim_batch(&mut self) {
        #[cfg(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        ))]
        self.clipboard_claim_batch.invalidate();
    }

    /// Capture this input batch's claim origin once, at input preparation.
    ///
    /// Preparation runs after all events for the batch (including a click-to-focus
    /// `Focused(true)`) have been dispatched, and uses the then-current paste generation. The
    /// prepared slot is independent of output selection and is always consumed by
    /// `handle_platform_output`, even when no clipboard command wins.
    #[cfg_attr(
        not(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        )),
        expect(clippy::needless_pass_by_ref_mut, clippy::unused_self)
    )]
    fn prepare_clipboard_claim_origin(&mut self, _window: &Window) {
        #[cfg(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        ))]
        {
            let transport = self.clipboard.wayland_transport();
            let generation = self.clipboard_paste.generation();
            self.clipboard_claim_batch
                .prepare(transport.as_ref(), _window.id(), generation);
        }
    }

    /// Consume the prepared batch origin, even when this output has no clipboard winner.
    #[cfg(all(
        feature = "clipboard",
        feature = "wayland",
        any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ),
    ))]
    fn take_clipboard_claim_origin(
        &mut self,
    ) -> clipboard_bridge::CapturedClaimOrigin<winit::platform::wayland::InputOriginToken> {
        self.clipboard_claim_batch.take_prepared()
    }

    #[cfg(not(all(
        feature = "clipboard",
        feature = "wayland",
        any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ),
    )))]
    #[cfg_attr(
        not(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        )),
        expect(clippy::needless_pass_by_ref_mut, clippy::unused_self)
    )]
    fn take_clipboard_claim_origin(&mut self) {}

    /// Resolve the captured batch origin once for the selected clipboard families.
    ///
    /// Only `Absent` regular text or PRIMARY may perform the legacy fallback lookup, and they
    /// share one token. Images and stale/failed captures never call `input_origin`. The prepared
    /// origin was already consumed by the caller, so a no-winner batch cannot leak it.
    #[cfg(all(
        feature = "clipboard",
        feature = "wayland",
        any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ),
    ))]
    fn resolve_output_claim_origins(
        &self,
        _window: &Window,
        generation: u64,
        captured: clipboard_bridge::CapturedClaimOrigin<winit::platform::wayland::InputOriginToken>,
        regular: Option<clipboard_bridge::RegularClaimOriginPolicy>,
        primary: bool,
    ) -> OutputClaimOrigins {
        if (regular.is_some() || primary)
            && let Some(transport) = self.clipboard.wayland_transport()
        {
            return OutputClaimOrigins::Wayland(clipboard_bridge::resolve_batch_claim_origins(
                &transport,
                _window.id(),
                generation,
                captured,
                regular,
                primary,
            ));
        }

        OutputClaimOrigins::NotWayland
    }

    #[cfg(not(all(
        feature = "clipboard",
        feature = "wayland",
        any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ),
    )))]
    #[cfg_attr(
        not(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        )),
        expect(clippy::needless_pass_by_ref_mut, clippy::unused_self)
    )]
    fn resolve_output_claim_origins(
        &mut self,
        _window: &Window,
        _generation: u64,
        _captured: (),
        _regular: Option<clipboard_bridge::RegularClaimOriginPolicy>,
        _primary: bool,
    ) -> OutputClaimOrigins {
        OutputClaimOrigins::NotWayland
    }

    #[cfg_attr(
        not(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        )),
        expect(clippy::needless_pass_by_ref_mut)
    )]
    fn take_regular_output_origin(origins: &mut OutputClaimOrigins) -> OutputClaimOrigin {
        #[cfg(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        ))]
        if let OutputClaimOrigins::Wayland(origins) = origins {
            return match origins.regular.take() {
                Some(Ok(token)) => OutputClaimOrigin::Ready(token),
                Some(Err(error)) => OutputClaimOrigin::Rejected(error),
                None => OutputClaimOrigin::NotWayland,
            };
        }

        #[cfg(not(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        )))]
        let _ = origins;

        OutputClaimOrigin::NotWayland
    }

    #[cfg_attr(
        not(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        )),
        expect(clippy::needless_pass_by_ref_mut)
    )]
    fn take_primary_output_origin(origins: &mut OutputClaimOrigins) -> OutputClaimOrigin {
        #[cfg(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        ))]
        if let OutputClaimOrigins::Wayland(origins) = origins {
            return match origins.primary.take() {
                Some(Ok(token)) => OutputClaimOrigin::Ready(token),
                Some(Err(error)) => OutputClaimOrigin::Rejected(error),
                None => OutputClaimOrigin::NotWayland,
            };
        }

        #[cfg(not(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        )))]
        let _ = origins;

        OutputClaimOrigin::NotWayland
    }

    /// Existing PRIMARY behavior: do not re-claim the same text in one input generation.
    ///
    /// This is the explicit exception to the no-cross-batch-dedup rule. A new input generation
    /// may claim the same text again.
    #[cfg_attr(
        not(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        )),
        expect(clippy::unused_self)
    )]
    fn primary_claim_is_suppressed(&self, generation: u64, text: &str) -> bool {
        #[cfg(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        ))]
        let last_claim = self.last_primary_claim.as_ref();
        #[cfg(not(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        )))]
        let last_claim: Option<&(u64, String)> = None;

        unchanged_primary_is_suppressed(last_claim, generation, text)
    }

    /// Retry a middle-click PRIMARY read that lost the click-to-focus ordering race.
    // The Wayland implementation mutates the per-window protocol state.
    #[cfg_attr(
        not(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        )),
        expect(clippy::needless_pass_by_ref_mut, clippy::unused_self)
    )]
    fn retry_deferred_primary(&mut self) {
        #[cfg(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        ))]
        if let Some(transport) = self.clipboard.wayland_transport()
            && let Err(err) = self.clipboard_paste.retry_deferred_primary(&transport)
        {
            self.record_clipboard_failure("PRIMARY retry", &err.to_io_error());
        }
    }

    /// Record a rejected request or claim and make non-routine failures visible in the log.
    // The Wayland implementation mutates the per-window protocol state.
    #[cfg_attr(
        not(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        )),
        expect(clippy::needless_pass_by_ref_mut, clippy::unused_self)
    )]
    fn record_clipboard_failure(&mut self, action: &str, error: &std::io::Error) {
        #[cfg(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        ))]
        {
            if error.kind() == std::io::ErrorKind::NotFound {
                // Empty/non-text content is a mundane no-op, not a transport fault.
                log::debug!("winit Wayland clipboard {action} failed: {error}");
            } else {
                self.clipboard_failures = self.clipboard_failures.wrapping_add(1);
                log::warn!(
                    "winit Wayland clipboard {action} failed ({} failures this session): {error}",
                    self.clipboard_failures
                );
            }
        }
        #[cfg(not(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        )))]
        let _ = (action, error);
    }

    /// Record an explicit stale/failed/missing origin rejection and count it as a clipboard
    /// failure. Unlike a transport "not found" result, an origin rejection is never routine.
    #[cfg(all(
        feature = "clipboard",
        feature = "wayland",
        any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ),
    ))]
    fn record_claim_origin_failure(
        &mut self,
        action: &str,
        error: &clipboard_bridge::ClaimOriginError,
    ) {
        self.clipboard_failures = self.clipboard_failures.wrapping_add(1);
        log::warn!(
            "winit Wayland clipboard {action} rejected the output origin ({} failures this session): {error}",
            self.clipboard_failures
        );
    }

    /// Number of Wayland clipboard requests or ownership claims rejected since construction.
    ///
    /// This includes admission failures (a full worker queue, a missing PRIMARY protocol or a
    /// disconnected event loop) and rejected ownership claims. The previous selection content is
    /// left in place when a claim is rejected.
    #[cfg(all(
        feature = "clipboard",
        feature = "wayland",
        any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ),
    ))]
    pub fn clipboard_failure_count(&self) -> u64 {
        self.clipboard_failures
    }

    /// Cancel pending clipboard reads because the window was hidden, minimized or destroyed.
    fn cancel_pending_clipboard_pastes(&mut self) {
        self.clipboard_paste
            .note_input(clipboard_bridge::InputKind::Hide);
        self.invalidate_clipboard_claim_batch();
        self.cancel_pending_image_copy();
    }

    /// Cancel one pending PNG copy and drop its exact winit reservation without waking.
    #[cfg(all(
        feature = "clipboard-image",
        feature = "clipboard",
        feature = "wayland",
        any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ),
    ))]
    fn cancel_pending_image_copy(&mut self) {
        if let Some(mut pending) = self.pending_image_copy.take() {
            pending.request.cancel();
            pending.reservation.take();
        }
    }

    #[cfg(not(all(
        feature = "clipboard-image",
        feature = "clipboard",
        feature = "wayland",
        any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ),
    )))]
    #[expect(clippy::needless_pass_by_ref_mut)]
    fn cancel_pending_image_copy(&mut self) {
        let _ = self;
    }

    /// A new winning regular output supersedes any pending image encode before origin
    /// resolution, even when that new attempt later fails.
    #[cfg(all(
        feature = "clipboard-image",
        feature = "clipboard",
        feature = "wayland",
        any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ),
    ))]
    fn invalidate_image_copy_for_regular_output(&mut self, window: &Window) {
        self.copy_epoch = self.copy_epoch.wrapping_add(1);
        self.cancel_pending_image_copy();
        if let Some(transport) = self.clipboard.wayland_transport()
            && let Err(error) = clipboard_bridge::ClipboardTransport::invalidate_image_reservations(
                &transport,
                window.id(),
            )
        {
            self.record_clipboard_failure("image reservation invalidation", &error.to_io_error());
        }
    }

    #[cfg(not(all(
        feature = "clipboard-image",
        feature = "clipboard",
        feature = "wayland",
        any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ),
    )))]
    #[expect(clippy::needless_pass_by_ref_mut)]
    fn invalidate_image_copy_for_regular_output(&mut self, _window: &Window) {
        let _ = self;
    }

    /// Install a completed PNG encode exactly once, on the event-loop thread, using its original
    /// reserved input serial. Focus, window, visibility, hide and deadline fences are rechecked
    /// before winit validates the remaining seat/ownership epochs.
    #[cfg(all(
        feature = "clipboard-image",
        feature = "clipboard",
        feature = "wayland",
        any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ),
    ))]
    fn install_pending_image_copy(&mut self, window: &Window) {
        let Some(pending) = self.pending_image_copy.as_ref() else {
            return;
        };
        if pending.epoch != self.copy_epoch
            || pending.target_window != window.id()
            || std::time::Instant::now() >= pending.deadline
        {
            self.cancel_pending_image_copy();
            self.clipboard_failures = self.clipboard_failures.wrapping_add(1);
            log::warn!("winit Wayland image copy expired or was superseded before installation");
            return;
        }
        let Some(result) = pending.request.try_take() else {
            return;
        };
        let Some(mut pending) = self.pending_image_copy.take() else {
            return;
        };
        if std::time::Instant::now() >= pending.deadline {
            self.clipboard_failures = self.clipboard_failures.wrapping_add(1);
            log::warn!("winit Wayland image copy expired before installation");
            return;
        }
        let reservation = pending.reservation.take();
        let png = match result {
            Ok(png) => png,
            Err(error) => {
                self.record_clipboard_failure("image encode", &error.to_io_error());
                return;
            }
        };
        let Some(reservation) = reservation else {
            self.record_clipboard_failure(
                "image claim",
                &std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "the image reservation was already consumed",
                ),
            );
            return;
        };
        if !window.has_focus()
            || window.is_visible() == Some(false)
            || window.is_minimized() == Some(true)
        {
            drop(png);
            drop(reservation);
            self.clipboard_failures = self.clipboard_failures.wrapping_add(1);
            log::warn!("winit Wayland image copy rejected after a focus/window/hide fence");
            return;
        }
        let Some(transport) = self.clipboard.wayland_transport() else {
            drop(png);
            drop(reservation);
            self.clipboard_failures = self.clipboard_failures.wrapping_add(1);
            log::warn!("winit Wayland clipboard is unavailable; encoded image was not installed");
            return;
        };
        if let Err(error) = transport.claim_reserved_image_png(png, reservation) {
            self.record_clipboard_failure("image claim", &error.to_io_error());
        }
    }

    #[cfg(not(all(
        feature = "clipboard-image",
        feature = "clipboard",
        feature = "wayland",
        any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ),
    )))]
    #[expect(clippy::needless_pass_by_ref_mut)]
    fn install_pending_image_copy(&mut self, _window: &Window) {
        let _ = self;
    }

    /// Start a non-blocking regular-clipboard paste on Wayland, or use the platform's
    /// synchronous compatibility path elsewhere.
    fn begin_regular_clipboard_paste(&mut self) {
        #[cfg(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        ))]
        if let Some(transport) = self.clipboard.wayland_transport() {
            let focused = self.focused_widget_id();
            match self.clipboard_paste.begin_regular(
                &transport,
                focused,
                std::sync::Arc::clone(&self.clipboard_repaint_waker),
            ) {
                Ok(()) => {}
                Err(err) => {
                    // Wayland image fallback is deliberately disabled until winit can carry an
                    // image offer through a validated, bounded transfer. An absent or non-text
                    // regular offer (including `InvalidData` from a stale self-source) is a
                    // failed paste, never a synchronous arboard read on the event path.
                    self.record_clipboard_failure("regular paste request", &err.to_io_error());
                }
            }
            return;
        }

        #[cfg(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        ))]
        if self.clipboard.is_wayland() {
            // No winit clipboard means no validated asynchronous transport. Do not substitute a
            // blocking arboard/data-control read on the Wayland event path.
            self.clipboard_failures = self.clipboard_failures.wrapping_add(1);
            log::warn!("winit Wayland clipboard is unavailable; ignoring paste request");
            return;
        }

        // Non-Wayland platforms retain their existing synchronous clipboard behavior.
        if let Some(contents) = self.clipboard.get() {
            let contents = contents.replace("\r\n", "\n");
            if !contents.is_empty() {
                self.egui_input.events.push(egui::Event::Paste(contents));
            }
        } else if let Some(image) = self.clipboard.get_image() {
            // No usable text on the clipboard (e.g. an image was copied with
            // mspaint/Snipping Tool, which never puts a text representation
            // alongside it) — fall back to an image paste rather than doing
            // nothing, mirroring `Event::Copy`/`OutputCommand::CopyImage`.
            self.egui_input
                .events
                .push(egui::Event::PasteImage(std::sync::Arc::new(image)));
        }
    }

    /// Start a non-blocking PRIMARY paste for a middle-click on Wayland.
    fn begin_primary_clipboard_paste(&mut self, pos: egui::Pos2) {
        #[cfg(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        ))]
        if let Some(transport) = self.clipboard.wayland_transport() {
            let focused = self.focused_widget_id();
            let target_layer = self.egui_ctx.layer_id_at(pos);
            if let Err(err) = self.clipboard_paste.begin_primary(
                &transport,
                focused,
                pos,
                target_layer,
                std::sync::Arc::clone(&self.clipboard_repaint_waker),
            ) {
                self.record_clipboard_failure("PRIMARY request", &err.to_io_error());
            }
            return;
        }

        #[cfg(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        ))]
        if self.clipboard.is_wayland() {
            self.clipboard_failures = self.clipboard_failures.wrapping_add(1);
            log::warn!("winit Wayland clipboard is unavailable; ignoring PRIMARY request");
            return;
        }

        if let Some(text) = self.clipboard.get_primary_text()
            && !text.is_empty()
        {
            self.egui_input
                .events
                .push(egui::Event::MiddleClickPaste { pos, text });
        }
    }

    /// Revalidate and emit completed clipboard reads exactly once.
    fn drain_clipboard_pastes(&mut self, window: &Window) {
        let focused = self.focused_widget_id();
        let outcomes = {
            // The bridge only mutably borrows its own field; the hit-test reads egui memory.
            let egui_ctx = &self.egui_ctx;
            self.clipboard_paste
                .drain_with_layers(window.id(), focused, |pos| egui_ctx.layer_id_at(pos))
        };
        for outcome in outcomes {
            match outcome {
                clipboard_bridge::PasteOutcome::Regular(Ok(text)) => {
                    // A present empty text is a successful read. Preserve egui's existing
                    // no-op-for-empty paste behavior without treating it as absent content.
                    let text = text.replace("\r\n", "\n");
                    if !text.is_empty() {
                        self.egui_input.events.push(egui::Event::Paste(text));
                    }
                }
                clipboard_bridge::PasteOutcome::Regular(Err(err)) => {
                    // No image fallback here: see `begin_regular_clipboard_paste`.
                    self.record_clipboard_failure("regular paste", &err.to_io_error());
                }
                clipboard_bridge::PasteOutcome::RegularImage(decoded) => {
                    let clipboard_bridge::DecodedPasteImage {
                        image,
                        retention: _retention,
                    } = decoded;
                    // The image moves into the event. `_retention` is dropped after handoff, so
                    // the codec budget no longer claims to bound application-retained pastes.
                    self.egui_input
                        .events
                        .push(egui::Event::PasteImage(std::sync::Arc::new(image)));
                }
                clipboard_bridge::PasteOutcome::Primary { pos, result } => match result {
                    Ok(text) if !text.is_empty() => {
                        self.egui_input
                            .events
                            .push(egui::Event::MiddleClickPaste { pos, text });
                    }
                    // PRESENT empty PRIMARY text is a success, not an absent offer.
                    Ok(_) => {}
                    Err(err) => {
                        self.record_clipboard_failure("PRIMARY paste", &err.to_io_error());
                    }
                },
            }
        }
    }

    /// Put a selected regular text winner on the clipboard.
    ///
    /// `origin` was resolved once for the whole output batch. A rejected stale/failed capture
    /// fails here without asking for another origin; the previous clipboard source is preserved.
    #[cfg_attr(
        not(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        )),
        expect(clippy::needless_pass_by_value)
    )]
    fn copy_text_output(&mut self, _window: &Window, text: String, origin: OutputClaimOrigin) {
        #[cfg(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        ))]
        if let Some(transport) = self.clipboard.wayland_transport() {
            let generation = self.clipboard_paste.generation();
            match origin {
                OutputClaimOrigin::Ready(token) => {
                    log::trace!("Wayland copy claim uses the captured batch origin");
                    if let Err(err) = clipboard_bridge::claim(
                        &transport,
                        clipboard_bridge::PasteKind::Regular,
                        &text,
                        token,
                        _window.id(),
                        generation,
                    ) {
                        self.record_clipboard_failure("copy claim", &err.to_io_error());
                    }
                }
                OutputClaimOrigin::Rejected(error) => {
                    self.record_claim_origin_failure("copy claim", &error);
                }
                OutputClaimOrigin::NotWayland => {
                    self.clipboard_failures = self.clipboard_failures.wrapping_add(1);
                    log::warn!(
                        "winit Wayland clipboard origin is unavailable; copy was not claimed"
                    );
                }
            }
            return;
        }

        #[cfg(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        ))]
        if self.clipboard.is_wayland() {
            self.clipboard_failures = self.clipboard_failures.wrapping_add(1);
            log::warn!("winit Wayland clipboard is unavailable; copy was not claimed");
            return;
        }

        let _ = origin;
        self.clipboard.set_text(text);
    }

    /// Put a selected image winner on the clipboard.
    ///
    /// The exact captured token is reserved synchronously, the caller-owned image is moved into
    /// the bounded encode worker without cloning, and the reserved PNG is installed on the
    /// event-loop thread after the encode completion repaint. A rejected origin, reservation,
    /// encode or claim leaves the previously installed clipboard source unchanged.
    #[cfg_attr(
        not(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        )),
        expect(clippy::needless_pass_by_value)
    )]
    fn copy_image_output(
        &mut self,
        _window: &Window,
        image: egui::ColorImage,
        origin: OutputClaimOrigin,
    ) {
        #[cfg(all(
            feature = "clipboard-image",
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        ))]
        if self.clipboard.is_wayland() {
            match origin {
                OutputClaimOrigin::Ready(token) => {
                    if let Err(error) = clipboard_image::validate_color_image(&image) {
                        self.record_clipboard_failure("image copy admission", &error.to_io_error());
                        return;
                    }
                    let deadline =
                        std::time::Instant::now() + clipboard_image::COPY_DELIVERY_BUDGET;
                    let Some(transport) = self.clipboard.wayland_transport() else {
                        self.clipboard_failures = self.clipboard_failures.wrapping_add(1);
                        log::warn!(
                            "winit Wayland clipboard is unavailable; image copy was not admitted"
                        );
                        return;
                    };
                    let reservation = match transport.reserve_image_claim(
                        winit::platform::wayland::SelectionClaim {
                            target_window: _window.id(),
                            caller_generation: self.clipboard_paste.generation(),
                            trigger: token,
                        },
                    ) {
                        Ok(reservation) => reservation,
                        Err(error) => {
                            self.record_clipboard_failure(
                                "image reservation",
                                &error.to_io_error(),
                            );
                            return;
                        }
                    };
                    // Defensively supersede any request that survived a prior output path.
                    self.cancel_pending_image_copy();
                    let request = match clipboard_image::ImageCopyRequest::start(
                        image,
                        std::sync::Arc::clone(&self.clipboard_repaint_waker),
                    ) {
                        Ok(request) => request,
                        Err(error) => {
                            drop(reservation);
                            self.record_clipboard_failure(
                                "image encode request",
                                &error.to_io_error(),
                            );
                            return;
                        }
                    };
                    self.pending_image_copy = Some(ImageCopyPending {
                        epoch: self.copy_epoch,
                        target_window: _window.id(),
                        deadline,
                        reservation: Some(reservation),
                        request,
                    });
                }
                OutputClaimOrigin::Rejected(error) => {
                    self.record_claim_origin_failure("image copy", &error);
                }
                OutputClaimOrigin::NotWayland => {
                    self.clipboard_failures = self.clipboard_failures.wrapping_add(1);
                    log::warn!(
                        "winit Wayland clipboard origin is unavailable; image copy was not admitted"
                    );
                }
            }
            return;
        }

        #[cfg(all(
            feature = "clipboard",
            feature = "wayland",
            not(feature = "clipboard-image"),
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        ))]
        if self.clipboard.is_wayland() {
            match origin {
                OutputClaimOrigin::Rejected(error) => {
                    self.record_claim_origin_failure("image copy", &error);
                }
                OutputClaimOrigin::Ready(_) | OutputClaimOrigin::NotWayland => {
                    self.clipboard_failures = self.clipboard_failures.wrapping_add(1);
                    log::warn!(
                        "Wayland image copy requires the 'clipboard-image' feature; nothing was written"
                    );
                }
            }
            drop(image);
            return;
        }

        let _ = origin;
        self.clipboard.set_image(&image);
        drop(image);
    }

    /// Put a selected settled selection on PRIMARY.
    ///
    /// Existing unchanged-selection suppression is handled before origin resolution in
    /// [`Self::handle_platform_output_inner`], so this method consumes no origin when suppressed.
    #[cfg_attr(
        not(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        )),
        expect(clippy::needless_pass_by_value)
    )]
    fn primary_selection_output(
        &mut self,
        _window: &Window,
        text: String,
        origin: OutputClaimOrigin,
    ) {
        #[cfg(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        ))]
        if let Some(transport) = self.clipboard.wayland_transport() {
            let generation = self.clipboard_paste.generation();
            match origin {
                OutputClaimOrigin::Ready(token) => {
                    log::trace!("Wayland PRIMARY claim uses the captured batch origin");
                    match clipboard_bridge::claim(
                        &transport,
                        clipboard_bridge::PasteKind::Primary,
                        &text,
                        token,
                        _window.id(),
                        generation,
                    ) {
                        Ok(()) => {
                            self.last_primary_claim = Some((generation, text));
                        }
                        Err(err) => {
                            self.record_clipboard_failure("PRIMARY claim", &err.to_io_error());
                        }
                    }
                }
                OutputClaimOrigin::Rejected(error) => {
                    self.record_claim_origin_failure("PRIMARY claim", &error);
                }
                OutputClaimOrigin::NotWayland => {
                    self.clipboard_failures = self.clipboard_failures.wrapping_add(1);
                    log::warn!(
                        "winit Wayland clipboard origin is unavailable; PRIMARY was not claimed"
                    );
                }
            }
            return;
        }

        #[cfg(all(
            feature = "clipboard",
            feature = "wayland",
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
        ))]
        if self.clipboard.is_wayland() {
            self.clipboard_failures = self.clipboard_failures.wrapping_add(1);
            log::warn!("winit Wayland clipboard is unavailable; PRIMARY was not claimed");
            return;
        }

        let _ = origin;
        self.clipboard.set_primary_text(text);
    }

    /// Returns [`false`] or the last value that [`Window::set_ime_allowed()`] was called with, used for debouncing.
    pub fn allow_ime(&self) -> bool {
        self.allow_ime
    }

    /// Set the last value that [`Window::set_ime_allowed()`] was called with.
    pub fn set_allow_ime(&mut self, allow: bool) {
        self.allow_ime = allow;
        if !allow {
            self.ime_reset.invalidate();
        }
    }

    /// Are clipboard keyboard shortcuts translated into clipboard events?
    ///
    /// See [`Self::set_clipboard_shortcuts`].
    #[inline]
    pub fn clipboard_shortcuts(&self) -> bool {
        self.clipboard_shortcuts
    }

    /// Should the platform clipboard keyboard shortcuts be translated into
    /// [`egui::Event::Cut`], [`egui::Event::Copy`] and [`egui::Event::Paste`]?
    ///
    /// The shortcuts are <kbd>Cmd/Ctrl</kbd>+<kbd>X</kbd>/<kbd>C</kbd>/<kbd>V</kbd>,
    /// the dedicated Cut/Copy/Paste keys, and on Windows also
    /// <kbd>Shift</kbd>+<kbd>Delete</kbd>, <kbd>Ctrl</kbd>+<kbd>Insert</kbd> and <kbd>Shift</kbd>+<kbd>Insert</kbd>.
    ///
    /// If `false`, these key presses are instead passed on as ordinary [`egui::Event::Key`] events,
    /// so your app can bind them to whatever it wants.
    /// Note that this means built-in widgets such as [`egui::TextEdit`] will no longer
    /// respond to these shortcuts, since they listen for the clipboard events.
    /// You can still push [`egui::Event::Cut`] etc. yourself.
    ///
    /// Default: `true`.
    #[inline]
    pub fn set_clipboard_shortcuts(&mut self, enabled: bool) {
        self.clipboard_shortcuts = enabled;
    }

    #[inline]
    pub fn egui_ctx(&self) -> &egui::Context {
        &self.egui_ctx
    }

    /// The current input state.
    /// This is changed by [`Self::on_window_event`] and cleared by [`Self::take_egui_input`].
    #[inline]
    pub fn egui_input(&self) -> &egui::RawInput {
        &self.egui_input
    }

    /// The current input state.
    /// This is changed by [`Self::on_window_event`] and cleared by [`Self::take_egui_input`].
    #[inline]
    pub fn egui_input_mut(&mut self) -> &mut egui::RawInput {
        &mut self.egui_input
    }

    /// Prepare for a new frame by extracting the accumulated input,
    ///
    /// as well as setting [the time](egui::RawInput::time) and [screen rectangle](egui::RawInput::screen_rect).
    ///
    /// You need to set [`egui::RawInput::viewports`] yourself though.
    /// Use [`update_viewport_info`] to update the info for each
    /// viewport.
    pub fn take_egui_input(&mut self, window: &Window) -> egui::RawInput {
        profiling::function_scope!();

        // Completed clipboard reads are revalidated and emitted exactly once here, before the
        // input is handed to egui. A worker only wakes this viewport; it never inspects egui.
        self.rebind_clipboard_target(window);
        self.install_pending_image_copy(window);
        self.drain_clipboard_pastes(window);

        // All events for this batch have been dispatched by now, including any click-to-focus
        // keyboard enter. Capture the final claimable input origin exactly once, using the
        // generation current at preparation. The slot is consumed by platform output below.
        self.prepare_clipboard_claim_origin(window);

        self.egui_input.time = Some(self.start_time.elapsed().as_secs_f64());

        // Android's winit backend never emits `ThemeChanged`, so the OS light/dark state
        // reaches egui through a direct read of the Activity's configuration instead —
        // `RawInput::system_theme`, the same field the desktop path fills. Set every frame;
        // egui resolves `ThemePreference::System` against it and only re-themes when the
        // value actually changes.
        #[cfg(target_os = "android")]
        {
            self.egui_input.system_theme = crate::system_theme::system_theme();
        }

        // Android: egui's TextEdit needs actual text events, and winit's Android backend
        // never delivers them. The Activity keeps a queue of committed text, backspace and
        // enter (see `ime`), which is drained into egui here.
        #[cfg(target_os = "android")]
        {
            for ev in crate::ime::android::poll_all_ime_events() {
                if ev == "\x08" {
                    self.egui_input.events.push(egui::Event::Key {
                        key: egui::Key::Backspace,
                        physical_key: Some(egui::Key::Backspace),
                        pressed: true,
                        repeat: false,
                        modifiers: self.modifiers,
                    });
                    self.egui_input.events.push(egui::Event::Key {
                        key: egui::Key::Backspace,
                        physical_key: Some(egui::Key::Backspace),
                        pressed: false,
                        repeat: false,
                        modifiers: self.modifiers,
                    });
                } else if ev == "\n" {
                    self.egui_input.events.push(egui::Event::Key {
                        key: egui::Key::Enter,
                        physical_key: Some(egui::Key::Enter),
                        pressed: true,
                        repeat: false,
                        modifiers: self.modifiers,
                    });
                    self.egui_input.events.push(egui::Event::Key {
                        key: egui::Key::Enter,
                        physical_key: Some(egui::Key::Enter),
                        pressed: false,
                        repeat: false,
                        modifiers: self.modifiers,
                    });
                    self.egui_input.events.push(egui::Event::Text("\n".to_string()));
                } else {
                    self.egui_input.events.push(egui::Event::Text(ev));
                }
            }
        }

        // On Windows, a minimized window will have 0 width and height.
        // See: https://github.com/rust-windowing/winit/issues/208
        // This solves an issue where egui window positions would be changed when minimizing on Windows.
        let screen_size_in_pixels = screen_size_in_pixels(window);
        let screen_size_in_points =
            screen_size_in_pixels / pixels_per_point(&self.egui_ctx, window);

        self.egui_input.screen_rect = (screen_size_in_points.x > 0.0
            && screen_size_in_points.y > 0.0)
            .then(|| Rect::from_min_size(Pos2::ZERO, screen_size_in_points));

        // Tell egui which viewport is now active:
        self.egui_input.viewport_id = self.viewport_id;

        self.egui_input
            .viewports
            .entry(self.viewport_id)
            .or_default()
            .native_pixels_per_point = Some(window.scale_factor() as f32);

        self.egui_input.take()
    }

    /// Call this when there is a new event.
    ///
    /// The result can be found in [`Self::egui_input`] and be extracted with [`Self::take_egui_input`].
    pub fn on_window_event(
        &mut self,
        window: &Window,
        event: &winit::event::WindowEvent,
    ) -> EventResponse {
        profiling::function_scope!(short_window_event_description(event));

        #[cfg(feature = "accesskit")]
        if let Some(accesskit) = self.accesskit.as_mut() {
            accesskit.process_event(window, event);
        }

        // A new input generation invalidates a pending paste on an edit, focus/selection change
        // or close. Releases and repaints deliberately leave it in flight.
        self.rebind_clipboard_target(window);
        let clipboard_input = clipboard_input_kind(event);
        self.clipboard_paste.note_input(clipboard_input);
        if clipboard_input_claims_origin(event) {
            // Record the intent now; the token is captured once in `take_egui_input` so a
            // click-to-focus keyboard-enter and any later event in the same batch are reflected.
            self.note_claimable_clipboard_input();
        }
        if clipboard_input_invalidates_claim_batch(clipboard_input) {
            // A token from an unfocused, hidden or destroyed window must not be used for output.
            self.invalidate_clipboard_claim_batch();
            // An in-flight image encode must not install ownership after focus/window loss.
            self.cancel_pending_image_copy();
        }
        if matches!(event, winit::event::WindowEvent::Focused(true)) {
            // A click-to-focus compositor may dispatch the middle-click press before
            // `wl_keyboard.enter`. Retry the one deferred PRIMARY read once focus arrives.
            self.retry_deferred_primary();
        }

        use winit::event::WindowEvent;

        #[cfg(target_os = "ios")]
        match &event {
            WindowEvent::Resized(_)
            | WindowEvent::ScaleFactorChanged { .. }
            | WindowEvent::Focused(true)
            | WindowEvent::Occluded(false) => {
                // Once winit v0.31 has been released this can be reworked to get the safe area from
                // `Window::safe_area`, and updated from a new event which is being discussed in
                // https://github.com/rust-windowing/winit/issues/3911.
                self.egui_input_mut().safe_area_insets = Some(safe_area::get_safe_area_insets());
            }
            _ => {}
        }

        match event {
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                let native_pixels_per_point = *scale_factor as f32;

                self.egui_input
                    .viewports
                    .entry(self.viewport_id)
                    .or_default()
                    .native_pixels_per_point = Some(native_pixels_per_point);

                EventResponse {
                    repaint: true,
                    consumed: false,
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.on_mouse_button_input(*state, *button);
                EventResponse {
                    repaint: true,
                    consumed: self.egui_ctx.egui_wants_pointer_input(),
                }
            }
            WindowEvent::MouseWheel { delta, phase, .. } => {
                self.on_mouse_wheel(window, *delta, *phase);
                EventResponse {
                    repaint: true,
                    consumed: self.egui_ctx.egui_wants_pointer_input(),
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.on_cursor_moved(window, *position);
                EventResponse {
                    repaint: true,
                    consumed: self.egui_ctx.egui_is_using_pointer(),
                }
            }
            WindowEvent::CursorLeft { .. } => {
                self.pointer_pos_in_points = None;
                self.egui_input.events.push(egui::Event::PointerGone);
                EventResponse {
                    repaint: true,
                    consumed: false,
                }
            }
            // WindowEvent::TouchpadPressure {device_id, pressure, stage, ..  } => {} // TODO(emilk)
            WindowEvent::Touch(touch) => {
                self.on_touch(window, touch);
                let consumed = match touch.phase {
                    winit::event::TouchPhase::Started
                    | winit::event::TouchPhase::Ended
                    | winit::event::TouchPhase::Cancelled => {
                        self.egui_ctx.egui_wants_pointer_input()
                    }
                    winit::event::TouchPhase::Moved => self.egui_ctx.egui_is_using_pointer(),
                };
                EventResponse {
                    repaint: true,
                    consumed,
                }
            }

            WindowEvent::ImeResetCompleted { .. } => {
                let forwarded = self.ime_reset.on_event(
                    window.id(),
                    self.viewport_id,
                    event,
                    &mut self.egui_input.events,
                );
                EventResponse {
                    repaint: forwarded,
                    consumed: false,
                }
            }
            WindowEvent::Ime(ime) => {
                if matches!(ime, winit::event::Ime::Commit(_))
                    && let Some((before_bytes, after_bytes)) = ime_delete_surrounding(window)
                {
                    self.egui_input.events.push(egui::Event::Ime(
                        egui::ImeEvent::DeleteSurroundingBytes {
                            before_bytes,
                            after_bytes,
                        },
                    ));
                }
                self.on_ime(ime);

                EventResponse {
                    repaint: true,
                    consumed: self.egui_ctx.egui_wants_keyboard_input(),
                }
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } => {
                if *is_synthetic && event.state == ElementState::Pressed {
                    // Winit generates fake "synthetic" KeyboardInput events when the focus
                    // is changed to the window, or away from it. Synthetic key presses
                    // represent no real key presses and should be ignored.
                    // See https://github.com/rust-windowing/winit/issues/3543
                    EventResponse {
                        repaint: true,
                        consumed: false,
                    }
                } else {
                    let egui_wants_keyboard_input = self.egui_ctx.egui_wants_keyboard_input();

                    if let Some(response) =
                        self.try_on_ime_processed_keyboard_input(event, egui_wants_keyboard_input)
                    {
                        response
                    } else {
                        self.on_keyboard_input(event);

                        // When pressing the Tab key, egui focuses the first focusable element, hence Tab always consumes.
                        let consumed = egui_wants_keyboard_input
                            || event.logical_key
                                == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Tab);
                        EventResponse {
                            repaint: true,
                            consumed,
                        }
                    }
                }
            }
            WindowEvent::Focused(focused) => {
                let focused = if cfg!(target_os = "macos") {
                    // TODO(emilk): remove this work-around once we update winit
                    // https://github.com/rust-windowing/winit/issues/4371
                    // https://github.com/emilk/egui/issues/7588
                    window.has_focus()
                } else {
                    *focused
                };

                self.egui_input.focused = focused;
                #[cfg(all(
                    feature = "wayland",
                    unix,
                    not(target_vendor = "apple"),
                    not(target_os = "android"),
                    not(target_os = "emscripten"),
                    not(target_os = "redox"),
                ))]
                self.ime_gate.set_focused(focused);
                if !focused {
                    self.ime_reset.invalidate();
                    // Avoid sticky modifiers when focus is lost (egui clears its own copy too).
                    self.modifiers = egui::Modifiers::default();
                    self.held_keys.clear();
                    #[cfg(target_os = "windows")]
                    self.pressed_processed_physical_keys.clear();
                    #[cfg(all(
                        feature = "wayland",
                        unix,
                        not(target_vendor = "apple"),
                        not(target_os = "android"),
                        not(target_os = "emscripten"),
                        not(target_os = "redox"),
                    ))]
                    self.clear_ime_surrounding_on_focus_loss(window);
                }
                self.egui_input
                    .events
                    .push(egui::Event::WindowFocused(focused));
                EventResponse {
                    repaint: true,
                    consumed: false,
                }
            }
            WindowEvent::ThemeChanged(winit_theme) => {
                self.egui_input.system_theme = Some(to_egui_theme(*winit_theme));
                EventResponse {
                    repaint: true,
                    consumed: false,
                }
            }
            WindowEvent::HoveredFile(path) => {
                self.egui_input.hovered_files.push(egui::HoveredFile {
                    path: Some(path.clone()),
                    ..Default::default()
                });
                EventResponse {
                    repaint: true,
                    consumed: false,
                }
            }
            WindowEvent::HoveredFileCancelled => {
                self.egui_input.hovered_files.clear();
                EventResponse {
                    repaint: true,
                    consumed: false,
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            WindowEvent::DroppedFile(path) => {
                self.egui_input.hovered_files.clear();
                self.egui_input
                    .dropped_files
                    .push(std::sync::Arc::new(NativeFile::from(path.clone())));
                EventResponse {
                    repaint: true,
                    consumed: false,
                }
            }
            // Winit's web backend does not emit file-drop events. Browser file reads
            // require a browser file handle, which this path-only event cannot provide.
            #[cfg(target_arch = "wasm32")]
            WindowEvent::DroppedFile(_) => EventResponse {
                repaint: false,
                consumed: false,
            },
            WindowEvent::ModifiersChanged(state) => {
                let state = state.state();

                let alt = state.alt_key();
                let ctrl = state.control_key();
                let shift = state.shift_key();
                let super_ = state.super_key();

                self.modifiers.alt = alt;
                self.modifiers.ctrl = ctrl;
                self.modifiers.shift = shift;
                // Use `Context::os()` rather than `target_os`, so that this also works on the web,
                // where the OS is only known at runtime (web integrations should set it using
                // `Context::set_os()` and `egui::os::OperatingSystem::from_user_agent()`).
                let is_mac = self.egui_ctx.os().is_mac();
                self.modifiers.mac_cmd = is_mac && super_;
                self.modifiers.command = if is_mac { super_ } else { ctrl };

                self.egui_input
                    .events
                    .push(egui::Event::ModifiersChanged(self.modifiers));

                EventResponse {
                    repaint: true,
                    consumed: false,
                }
            }

            WindowEvent::Destroyed => {
                self.ime_reset.invalidate();
                self.held_keys.clear();
                self.modifiers = egui::Modifiers::default();
                self.egui_input.focused = false;
                self.egui_input
                    .events
                    .push(egui::Event::WindowFocused(false));
                #[cfg(target_os = "windows")]
                self.pressed_processed_physical_keys.clear();
                // These latches describe the destroyed window, not just its Wayland protocol
                // state. Reset them on every platform so a reused `State` cannot skip the new
                // window's first cursor-area or purpose call, or believe IME is already enabled.
                reset_ime_window_latches(
                    &mut self.allow_ime,
                    &mut self.ime_cursor_rect_px,
                    &mut self.old_ime_purpose,
                );
                #[cfg(all(
                    feature = "wayland",
                    unix,
                    not(target_vendor = "apple"),
                    not(target_os = "android"),
                    not(target_os = "emscripten"),
                    not(target_os = "redox"),
                ))]
                {
                    // A reused `State` must not retain this window's focus, external-veto or
                    // pending-enable latches, nor believe that its surrounding text is still
                    // cached. The window is gone, so there is nothing to disable on the platform.
                    self.ime_gate.forget_window();
                    self.ime_surrounding.forget_backend();
                }
                EventResponse {
                    repaint: true,
                    consumed: false,
                }
            }

            // Things that may require repaint:
            WindowEvent::RedrawRequested
            | WindowEvent::CursorEntered { .. }
            | WindowEvent::Occluded(_)
            | WindowEvent::Resized(_)
            | WindowEvent::Moved(_)
            | WindowEvent::TouchpadPressure { .. }
            | WindowEvent::CloseRequested => EventResponse {
                repaint: true,
                consumed: false,
            },

            // Things we completely ignore:
            WindowEvent::ActivationTokenDone { .. }
            | WindowEvent::AxisMotion { .. }
            | WindowEvent::DoubleTapGesture { .. } => EventResponse {
                repaint: false,
                consumed: false,
            },

            WindowEvent::PinchGesture { delta, .. } => {
                // Positive delta values indicate magnification (zooming in).
                // Negative delta values indicate shrinking (zooming out).
                let zoom_factor = (*delta as f32).exp();
                self.egui_input.events.push(egui::Event::Zoom(zoom_factor));
                EventResponse {
                    repaint: true,
                    consumed: self.egui_ctx.egui_wants_pointer_input(),
                }
            }

            WindowEvent::RotationGesture { delta, .. } => {
                // Positive delta values indicate counterclockwise rotation
                // Negative delta values indicate clockwise rotation
                // This is opposite of egui's sign convention for angles
                self.egui_input
                    .events
                    .push(egui::Event::Rotate(-delta.to_radians()));
                EventResponse {
                    repaint: true,
                    consumed: self.egui_ctx.egui_wants_pointer_input(),
                }
            }

            WindowEvent::PanGesture { delta, phase, .. } => {
                let pixels_per_point = pixels_per_point(&self.egui_ctx, window);

                self.egui_input.events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: Vec2::new(delta.x, delta.y) / pixels_per_point,
                    phase: to_egui_touch_phase(*phase),
                    source: egui::MouseWheelSource::Trackpad,
                    modifiers: self.modifiers,
                });
                EventResponse {
                    repaint: true,
                    consumed: self.egui_ctx.egui_wants_pointer_input(),
                }
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    #[expect(clippy::unused_self, clippy::needless_pass_by_ref_mut)]
    #[inline(always)]
    fn try_on_ime_processed_keyboard_input(
        &mut self,
        _event: &winit::event::KeyEvent,
        _egui_wants_keyboard_input: bool,
    ) -> Option<EventResponse> {
        // `KeyboardInput` events processed by the IME are not emitted by
        // `winit` on non-Windows platforms, so we don't need to do anything
        // here.

        None
    }

    #[cfg(target_os = "windows")]
    #[inline(always)]
    fn try_on_ime_processed_keyboard_input(
        &mut self,
        event: &winit::event::KeyEvent,
        egui_wants_keyboard_input: bool,
    ) -> Option<EventResponse> {
        if !self.allow_ime {
            None
        } else if event.state == ElementState::Pressed
            && event.logical_key == winit::keyboard::NamedKey::Process
        {
            // On Windows, some `winit` versions have a bug where
            // `KeyboardInput` events processed by the IME are still emitted. [^1]
            //
            // The pinned `craft/0.30.13` winit already drops `VK_PROCESSKEY`
            // presses before they reach `egui-winit` (see
            // `platform_impl/windows/keyboard.rs`), so this branch is now
            // defensive compatibility for winit builds that still deliver
            // `NamedKey::Process`. It only acts on presses: a Process release
            // must not be mistaken for a fresh interception.
            //
            // As a workaround, we detect these events by checking whether their
            // `logical_key` is `winit::keyboard::NamedKey::Process`, and filter
            // them out to keep behavior consistent with other platforms.
            //
            // `winit::keyboard::NamedKey::Process` is not documented in
            // `winit`. Reading through its source code, we find that it is
            // mapped from `VK_PROCESSKEY` on Windows [^2]. (On an unrelated
            // note, Web is the only other platform that also uses it [^3].)
            // According to Microsoft, “the IME sets the virtual key value
            // to `VK_PROCESSKEY` after processing a key input message” [^4].
            // See also [^5].
            // (I can't find a documentation page dedicated to this value.)
            //
            // Keep this defensive code while any supported `winit` build can
            // still emit IME-processed presses. If a pin drops that behaviour
            // entirely, `pressed_processed_physical_keys` and this method can
            // be removed.
            //
            // [^1]: https://github.com/rust-windowing/winit/issues/4508
            // [^2]: https://github.com/rust-windowing/winit/blob/e9809ef54b18499bb4f2cac945719ecc2a61061b/src/platform_impl/windows/keyboard_layout.rs#L946
            // [^3]: https://developer.mozilla.org/en-US/docs/Web/API/UI_Events/Keyboard_event_key_values
            // [^4]: https://learn.microsoft.com/en-us/windows/win32/api/imm/nf-imm-immgetvirtualkey#remarks
            // [^5]: https://learn.microsoft.com/en-us/windows/win32/learnwin32/keyboard-input#character-messages

            // The IME may take over a key which already emitted an ordinary
            // press. Retire that exact identity before filtering its release.
            let before = self.egui_input.events.len();
            self.release_keyboard_input(event.physical_key);
            self.pressed_processed_physical_keys
                .insert(event.physical_key);

            Some(EventResponse {
                repaint: self.egui_input.events.len() != before,
                consumed: egui_wants_keyboard_input,
            })
        } else if event.state == ElementState::Released
            && self
                .pressed_processed_physical_keys
                .remove(&event.physical_key)
        {
            // If an ordinary press was emitted after the filtered Process press
            // (for example a layout repeat), retire its exact identity as well.
            // This is a no-op for the normal filtered press, which retired its
            // identity when the marker was inserted.
            let before = self.egui_input.events.len();
            self.release_keyboard_input(event.physical_key);
            // Unlike key-presses, we can not tell whether a key-release event
            // is processed by the IME or not by looking at its `logical_key`,
            // because their `logical_key` is the original value (e.g.
            // `winit::keyboard::Key::Character(…)`) rather than
            // `winit::keyboard::Key::Named(winit::keyboard::NamedKey::Process)`.
            // (See the screencast for Windows in [^1].)
            // So we track the physical keys of processed key-presses and
            // filter out the corresponding key-releases.
            //
            // [^1]: https://github.com/rust-windowing/winit/issues/4508

            Some(EventResponse {
                repaint: self.egui_input.events.len() != before,
                consumed: egui_wants_keyboard_input,
            })
        } else {
            None
        }
    }

    /// ## NOTE
    ///
    /// on Mac even Cmd-C is pressed during ime, a `c` is pushed to Preedit.
    /// So no need to check `is_mac_cmd`.
    ///
    /// ### How events are emitted by [`winit`] across different setups in various situations
    ///
    /// This is done by uncommenting the code block at the top of this method
    /// and checking console outputs.
    ///
    /// winit version: 0.30.12.
    ///
    /// #### Setups
    ///
    /// - `a-macos15-apple_shuangpin`: macOS 15.7.3 `aarch64`, IME: builtin Chinese Shuangpin - Simplified. (Demo app shows: renderer: `wgpu`, backend: `Metal`.)
    /// - `b-debian13_gnome48_wayland-fcitx5_shuangpin`: Debian 13 `aarch64`, Gnome 48, Wayland, IME: Fcitx5 with fcitx5-chinese-addons's Shuangpin. (Demo app shows: renderer: `wgpu`, backend: `Gl`.)
    /// - `c-windows11-ms_pinyin`: Windows11 23H2 `x86_64`, IME: builtin Microsoft Pinyin. (Demo app shows: renderer: `wgpu`, backend: `Vulkan` & `Dx12`, others: `Dx12` & `Gl`.)
    ///
    /// #### Situation: pressed space to select the first candidate "测试"
    ///
    /// | Setup                                       | Events in Order                                                                                                                  |
    /// | ------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
    /// | a-macos15-apple_shuangpin                   | `Preedit("", None)` -> `Commit("测试")`                                                                                          |
    /// | b-debian13_gnome48_wayland-fcitx5_shuangpin | `Preedit("", None)` -> `Commit("测试")` -> `Preedit("", Some(0, 0))` -> `Preedit("", None)` (duplicate until `TextEdit` blurred) |
    /// | c-windows11-ms_pinyin                       | `Preedit("测试", Some(…))` -> `Preedit("", None)` -> `Commit("测试")` -> `Disabled`                                              |
    ///
    /// #### Situation: pressed backspace to delete the last character in the composition
    ///
    /// | Setup                                       | Events in Order                                                                       |
    /// | a-macos15-apple_shuangpin                   | `Preedit("", None)`                                                                   |
    /// | b-debian13_gnome48_wayland-fcitx5_shuangpin | `Preedit("", Some(0, 0))` -> `Preedit("", None)` (duplicate until `TextEdit` blurred) |
    /// | c-windows11-ms_pinyin                       | `Preedit("", Some(0, 0))` -> `Preedit("", None)` -> `Commit("")` -> `Disabled`        |
    ///
    /// #### Situation: clicked somewhere else while there is an active composition with the pre-edit text "ce"
    ///
    /// | Setup                                       | Events in Order                                                                                   |
    /// | ------------------------------------------- | ------------------------------------------------------------------------------------------------- |
    /// | a-macos15-apple_shuangpin                   | nothing emitted                                                                                   |
    /// | b-debian13_gnome48_wayland-fcitx5_shuangpin | `Preedit("", Some(0, 0))` (duplicate) -> `Preedit("", None)` (duplicate until `TextEdit` blurred) |
    /// | c-windows11-ms_pinyin                       | nothing emitted                                                                                   |
    fn on_ime(&mut self, ime: &winit::event::Ime) {
        // // code for inspecting ime events emitted by winit:
        // {
        //     static LAST_IME: std::sync::Mutex<Option<winit::event::Ime>> =
        //         std::sync::Mutex::new(None);
        //     static IS_LAST_DUPLICATE: std::sync::atomic::AtomicBool =
        //         std::sync::atomic::AtomicBool::new(false);
        //     let mut last_ime_guard = LAST_IME.lock().unwrap();
        //     if { last_ime_guard.as_ref().cloned() }.as_ref() != Some(ime) {
        //         println!("IME={ime:?}");
        //         *last_ime_guard = Some(ime.clone());
        //         IS_LAST_DUPLICATE.store(false, std::sync::atomic::Ordering::Relaxed);
        //     } else if !IS_LAST_DUPLICATE.load(std::sync::atomic::Ordering::Relaxed) {
        //         println!("IME=(duplicate)");
        //         IS_LAST_DUPLICATE.store(true, std::sync::atomic::Ordering::Relaxed);
        //     }
        // }

        match ime {
            // [`winit::event::Ime::Enabled`] means different things in X11 and
            // Wayland, but it does not affect event forwarding. Only the Wayland cache
            // invalidation below is backend-specific.
            // See <https://github.com/rust-windowing/winit/issues/2498>
            winit::event::Ime::Enabled => {
                // Enabling re-applies winit's cached snapshot, so our cache stays authoritative.
            }
            winit::event::Ime::Disabled => {
                // A disable is the one lifecycle boundary that can drop winit's cached text
                // without a platform-output pass of ours (for example an external
                // `set_ime_allowed(false)`). Only invalidate the cache here; never call
                // `set_ime_allowed` from this asynchronous event, which would create an
                // event-driven enable/disable loop. The next platform-output pass republishes
                // the authoritative snapshot once, if the window is still allowed and focused.
                #[cfg(all(
                    feature = "wayland",
                    unix,
                    not(target_vendor = "apple"),
                    not(target_os = "android"),
                    not(target_os = "emscripten"),
                    not(target_os = "redox"),
                ))]
                self.ime_surrounding.forget_backend();
            }
            winit::event::Ime::Preedit(text, active_range_bytes) => {
                let active_range_chars = match *active_range_bytes {
                    Some((start_bytes, end_bytes)) => {
                        if let (Some(start_chars), Some(middle_chars)) = (
                            text.get(..start_bytes).map(|s| s.chars().count()),
                            text.get(start_bytes..end_bytes).map(|s| s.chars().count()),
                        ) {
                            if cfg!(target_os = "windows") && start_chars == 0 && middle_chars == 0
                            {
                                // Workaround for a bug on Windows where `winit`
                                // incorrectly reports the cursor position at
                                // the start of the preedit text during
                                // composition with the builtin Korean IME.
                                // See: https://github.com/emilk/egui/pull/8083#issuecomment-4206742668
                                // TODO(umajho): Remove this workaround once the
                                // `winit` bug is fixed and we've updated to a
                                // version that includes the fix.
                                None
                            } else {
                                Some(start_chars..start_chars + middle_chars)
                            }
                        } else {
                            log::warn!("ignoring {ime:?}'s range because it is invalid");
                            None
                        }
                    }
                    None => None,
                };

                self.egui_input
                    .events
                    .push(egui::Event::Ime(egui::ImeEvent::Preedit {
                        text: text.clone(),
                        active_range_chars,
                    }));
            }
            winit::event::Ime::Commit(text) => {
                self.egui_input
                    .events
                    .push(egui::Event::Ime(egui::ImeEvent::Commit(text.clone())));
            }
        }
    }

    /// Clear and forget outgoing surrounding text when this Wayland window loses focus.
    ///
    /// Winit forgets its cached text when IME is disallowed, so disabling here is the committed
    /// clear boundary. The durable [`ime_surrounding::ImeGate`] latch keeps later stale
    /// `PlatformOutput::ime` passes from re-enabling IME or republishing text while the window
    /// stays unfocused; focus regain commits one clean enable and a full resend.
    #[cfg(all(
        feature = "wayland",
        unix,
        not(target_vendor = "apple"),
        not(target_os = "android"),
        not(target_os = "emscripten"),
        not(target_os = "redox"),
    ))]
    fn clear_ime_surrounding_on_focus_loss(&mut self, window: &Window) {
        // An X11 window in the same winit build has no Wayland toplevel and no surrounding-text
        // protocol state. Preserve its pre-existing IME behavior.
        if self.allow_ime && ime_surrounding::is_wayland_window(window) {
            self.allow_ime = false;
            window.set_ime_allowed(false);
        }
        self.ime_surrounding.forget_backend();
    }

    /// Apply an external `ViewportCommand::IMEAllowed` to this window synchronously.
    ///
    /// This is the command-side owner of the same IME gate transition used by
    /// [`Self::handle_platform_output_inner`]. `IMEAllowed(false)` commits immediately, so a
    /// later platform output in the same frame cannot publish surrounding text first.
    /// `IMEAllowed(true)` only clears the external veto and records a pending enable; the next
    /// platform-output pass with a real IME descriptor and window focus commits the enable. That
    /// single policy prevents a stale descriptor from enabling a window after its editor is gone.
    /// Only a real platform transition forgets the surrounding-text cache. A redundant command
    /// keeps it so a same-pass owner/generation change can still plan `ClearThenSet`.
    ///
    /// X11 windows in a Wayland-capable winit build, and all non-Wayland targets, keep the
    /// pre-existing raw `set_ime_allowed` behavior.
    // Reset admission changes on every platform; Wayland also updates protocol/cache state.
    fn handle_external_ime_allowed(&mut self, window: &Window, allowed: bool) {
        self.ime_reset.external_allowed(allowed);
        #[cfg(all(
            feature = "wayland",
            unix,
            not(target_vendor = "apple"),
            not(target_os = "android"),
            not(target_os = "emscripten"),
            not(target_os = "redox"),
        ))]
        if ime_surrounding::is_wayland_window(window) {
            self.ime_gate.set_external_ime_allowed(allowed);
            let transition = self.ime_gate.external_transition(self.allow_ime);
            self.allow_ime = transition.allow;
            if transition.changed {
                window.set_ime_allowed(transition.allow);
                // Only a real platform transition invalidates winit's cached text. A redundant
                // `IMEAllowed(true)` or `false` must preserve the bridge cache; otherwise an
                // owner/generation change later in the same pass would look like an initial
                // context and could skip the clear that cancels the previous composition.
                self.ime_surrounding.forget_backend();
            }
            return;
        }

        window.set_ime_allowed(allowed);
    }

    /// Publish this pass's authoritative outgoing IME surrounding text on Wayland.
    ///
    /// The planner is pure and the window adapter is a sink. The cache is only committed after
    /// every sink operation for the pass was applied; a locally suppressed write is forgotten,
    /// not cached. Diagnostics only contain redacted lengths.
    #[cfg(all(
        feature = "wayland",
        unix,
        not(target_vendor = "apple"),
        not(target_os = "android"),
        not(target_os = "emscripten"),
        not(target_os = "redox"),
    ))]
    fn sync_ime_surrounding_text(
        &mut self,
        window: &Window,
        ime: Option<&egui::output::IMEOutput>,
    ) {
        let mut sink = ime_surrounding::WinitImeSurroundingSink::new(window, self.allow_ime);
        let action = self.ime_surrounding.update(&mut sink, ime, self.allow_ime);

        if action != ime_surrounding::ImeSurroundingAction::None {
            if let Some(ime) = ime {
                log::trace!(
                    "IME surrounding text for owner {:?}, generation {}: {action:?}",
                    ime.owner,
                    ime.context_generation,
                );
            } else {
                log::trace!("IME surrounding text cleared: {action:?}");
            }
        }
    }

    /// Forward raw mouse motion (e.g. `winit::event::DeviceEvent::MouseMotion`) to egui
    /// as [`egui::Event::MouseMoved`].
    ///
    /// The motion is ignored unless the pointer is inside the window or a pointer button is held down,
    /// so that mouse motion elsewhere on the screen doesn't cause repaints.
    ///
    /// Returns `true` if the event was sent to egui.
    pub fn on_mouse_motion(&mut self, delta: (f64, f64)) -> bool {
        if !self.is_pointer_in_window() && !self.any_pointer_button_down {
            return false;
        }

        self.egui_input.events.push(egui::Event::MouseMoved(Vec2 {
            x: delta.0 as f32,
            y: delta.1 as f32,
        }));
        true
    }

    /// Returns `true` when the pointer is currently inside the window.
    pub fn is_pointer_in_window(&self) -> bool {
        self.pointer_pos_in_points.is_some()
    }

    /// Returns `true` if any pointer button is currently held down.
    pub fn is_any_pointer_button_down(&self) -> bool {
        self.any_pointer_button_down
    }

    /// Call this when there is a new [`accesskit::ActionRequest`].
    ///
    /// The result can be found in [`Self::egui_input`] and be extracted with [`Self::take_egui_input`].
    #[cfg(feature = "accesskit")]
    pub fn on_accesskit_action_request(&mut self, request: accesskit::ActionRequest) {
        self.egui_input
            .events
            .push(egui::Event::AccessKitActionRequest(request));
    }

    fn on_mouse_button_input(
        &mut self,
        state: winit::event::ElementState,
        button: winit::event::MouseButton,
    ) {
        if let Some(pos) = self.pointer_pos_in_points
            && let Some(button) = translate_mouse_button(button)
        {
            let pressed = state == winit::event::ElementState::Pressed;

            self.egui_input.events.push(egui::Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers: self.modifiers,
            });

            // Middle-click pastes the PRIMARY selection on X11 and Wayland. On Wayland this
            // starts a bounded asynchronous read; the paste event arrives when it completes.
            if pressed && button == egui::PointerButton::Middle {
                self.begin_primary_clipboard_paste(pos);
            }

            if self.simulate_touch_screen {
                if pressed {
                    self.any_pointer_button_down = true;

                    self.egui_input.events.push(egui::Event::Touch {
                        device_id: egui::TouchDeviceId(0),
                        id: egui::TouchId(0),
                        phase: egui::TouchPhase::Start,
                        pos,
                        force: None,
                    });
                } else {
                    self.any_pointer_button_down = false;

                    self.egui_input.events.push(egui::Event::PointerGone);

                    self.egui_input.events.push(egui::Event::Touch {
                        device_id: egui::TouchDeviceId(0),
                        id: egui::TouchId(0),
                        phase: egui::TouchPhase::End,
                        pos,
                        force: None,
                    });
                }
            }
        }
    }

    fn on_cursor_moved(
        &mut self,
        window: &Window,
        pos_in_pixels: winit::dpi::PhysicalPosition<f64>,
    ) {
        let pixels_per_point = pixels_per_point(&self.egui_ctx, window);

        let pos_in_points = egui::pos2(
            pos_in_pixels.x as f32 / pixels_per_point,
            pos_in_pixels.y as f32 / pixels_per_point,
        );
        self.pointer_pos_in_points = Some(pos_in_points);

        if self.simulate_touch_screen {
            if self.any_pointer_button_down {
                self.egui_input
                    .events
                    .push(egui::Event::PointerMoved(pos_in_points));

                self.egui_input.events.push(egui::Event::Touch {
                    device_id: egui::TouchDeviceId(0),
                    id: egui::TouchId(0),
                    phase: egui::TouchPhase::Move,
                    pos: pos_in_points,
                    force: None,
                });
            }
        } else {
            self.egui_input
                .events
                .push(egui::Event::PointerMoved(pos_in_points));
        }
    }

    fn on_touch(&mut self, window: &Window, touch: &winit::event::Touch) {
        let pixels_per_point = pixels_per_point(&self.egui_ctx, window);

        // Emit touch event
        self.egui_input.events.push(egui::Event::Touch {
            device_id: egui::TouchDeviceId(egui::epaint::util::hash(touch.device_id)),
            id: egui::TouchId::from(touch.id),
            phase: to_egui_touch_phase(touch.phase),
            pos: egui::pos2(
                touch.location.x as f32 / pixels_per_point,
                touch.location.y as f32 / pixels_per_point,
            ),
            force: match touch.force {
                Some(winit::event::Force::Normalized(force)) => Some(force as f32),
                Some(winit::event::Force::Calibrated {
                    force,
                    max_possible_force,
                    ..
                }) => Some((force / max_possible_force) as f32),
                None => None,
            },
        });
        // If we're not yet translating a touch or we're translating this very
        // touch …
        if self.pointer_touch_id.is_none() || self.pointer_touch_id.unwrap_or_default() == touch.id
        {
            // … emit PointerButton resp. PointerMoved events to emulate mouse
            match touch.phase {
                winit::event::TouchPhase::Started => {
                    self.pointer_touch_id = Some(touch.id);
                    // First move the pointer to the right location
                    self.on_cursor_moved(window, touch.location);
                    self.on_mouse_button_input(
                        winit::event::ElementState::Pressed,
                        winit::event::MouseButton::Left,
                    );
                }
                winit::event::TouchPhase::Moved => {
                    self.on_cursor_moved(window, touch.location);
                }
                winit::event::TouchPhase::Ended => {
                    self.pointer_touch_id = None;
                    self.on_mouse_button_input(
                        winit::event::ElementState::Released,
                        winit::event::MouseButton::Left,
                    );
                    // The pointer should vanish completely to not get any
                    // hover effects
                    self.pointer_pos_in_points = None;
                    self.egui_input.events.push(egui::Event::PointerGone);
                }
                winit::event::TouchPhase::Cancelled => {
                    self.pointer_touch_id = None;
                    self.pointer_pos_in_points = None;
                    self.egui_input.events.push(egui::Event::PointerGone);
                }
            }
        }
    }

    fn on_mouse_wheel(
        &mut self,
        window: &Window,
        delta: winit::event::MouseScrollDelta,
        phase: winit::event::TouchPhase,
    ) {
        let pixels_per_point = pixels_per_point(&self.egui_ctx, window);

        {
            let (unit, delta) = match delta {
                winit::event::MouseScrollDelta::LineDelta(x, y) => {
                    (egui::MouseWheelUnit::Line, egui::vec2(x, y))
                }
                winit::event::MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition {
                    x,
                    y,
                }) => (
                    egui::MouseWheelUnit::Point,
                    egui::vec2(x as f32, y as f32) / pixels_per_point,
                ),
            };
            let phase = to_egui_touch_phase(phase);
            let modifiers = self.modifiers;

            // winit doesn't tell us what is driving the scroll, and collapses the macOS
            // momentum phase into the regular `TouchPhase`, so we detect that ourselves.
            // TODO(emilk): use `WindowEvent::MouseWheel::source` once we are on a winit
            // with https://github.com/rust-windowing/winit/pull/4732
            let source = cfg_select! {
                target_os = "macos" => {
                    if self
                        .scroll_momentum_monitor
                        .as_ref()
                        .is_some_and(|monitor| monitor.latest_scroll_event_is_momentum())
                    {
                        egui::MouseWheelSource::Momentum
                    } else {
                        // On macOS, only trackpads (and the Magic Mouse) report precise deltas:
                        match unit {
                            egui::MouseWheelUnit::Point => egui::MouseWheelSource::Trackpad,
                            egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => {
                                egui::MouseWheelSource::Wheel
                            }
                        }
                    }
                }
                _ => egui::MouseWheelSource::Unknown,
            };

            self.egui_input.events.push(egui::Event::MouseWheel {
                unit,
                delta,
                phase,
                source,
                modifiers,
            });
        }
    }

    /// Release the egui key that was actually emitted when this physical key
    /// went down (if any), stamping the current modifiers and physical mapping.
    fn release_keyboard_input(&mut self, physical: winit::keyboard::PhysicalKey) {
        let physical_key = match physical {
            winit::keyboard::PhysicalKey::Code(code) => key_from_key_code(code),
            winit::keyboard::PhysicalKey::Unidentified(_) => None,
        };
        for key in self.held_keys.release(physical) {
            self.egui_input.events.push(egui::Event::Key {
                key,
                physical_key,
                pressed: false,
                repeat: false,
                modifiers: self.modifiers,
            });
        }
    }

    /// Emit an ordinary key press for a physical key, using the identity that
    /// egui already received for it if it is still held (a repeat).
    fn press_keyboard_input(
        &mut self,
        physical: winit::keyboard::PhysicalKey,
        physical_key: Option<egui::Key>,
        translated: Option<egui::Key>,
    ) -> Option<egui::Key> {
        let active_key = self.held_keys.press(physical, translated, true)?;
        self.egui_input.events.push(egui::Event::Key {
            key: active_key,
            physical_key,
            pressed: true,
            repeat: false, // egui will fill this in for us!
            modifiers: self.modifiers,
        });
        Some(active_key)
    }

    /// Translate a pressed key into a cut/copy/paste event if it is a clipboard
    /// shortcut. The clipboard event is pushed before retiring any prior
    /// identity for the physical key, and a `None` marker is recorded so the
    /// matching release cannot clear another physical alias.
    ///
    /// Returns `true` if the event was a clipboard shortcut (and was consumed).
    fn try_clipboard_shortcut(
        &mut self,
        physical: winit::keyboard::PhysicalKey,
        translated: Option<egui::Key>,
    ) -> bool {
        let Some(active_key) = translated else {
            return false;
        };
        if !self.clipboard_shortcuts {
            return false;
        }

        if is_cut_command(self.modifiers, active_key) {
            self.egui_input.events.push(egui::Event::Cut);
        } else if is_copy_command(self.modifiers, active_key) {
            self.egui_input.events.push(egui::Event::Copy);
        } else if is_paste_command(self.modifiers, active_key) {
            // On Wayland this only starts a transfer and requests a viewport repaint on
            // completion; it never blocks the event path on the clipboard peer.
            self.begin_regular_clipboard_paste();
        } else {
            return false;
        }

        self.release_keyboard_input(physical);
        let _ = self.held_keys.press(physical, translated, false);
        true
    }

    fn on_keyboard_input(&mut self, event: &winit::event::KeyEvent) {
        let winit::event::KeyEvent {
            // Represents the position of a key independent of the currently active layout.
            //
            // It also uniquely identifies the physical key (i.e. it's mostly synonymous with a scancode).
            // The most prevalent use case for this is games. For example the default keys for the player
            // to move around might be the W, A, S, and D keys on a US layout. The position of these keys
            // is more important than their label, so they should map to Z, Q, S, and D on an "AZERTY"
            // layout. (This value is `KeyCode::KeyW` for the Z key on an AZERTY layout.)
            physical_key,

            // Represents the results of a keymap, i.e. what character a certain key press represents.
            // When telling users "Press Ctrl-F to find", this is where we should
            // look for the "F" key, because they may have a dvorak layout on
            // a qwerty keyboard, and so the logical "F" character may not be located on the physical `KeyCode::KeyF` position.
            logical_key: winit_logical_key,

            text,

            state,

            location: _, // e.g. is it on the numpad?
            repeat: _,   // egui will figure this out for us
            ..
        } = event;

        let pressed = *state == winit::event::ElementState::Pressed;

        let physical_key = if let winit::keyboard::PhysicalKey::Code(keycode) = *physical_key {
            key_from_key_code(keycode)
        } else {
            None
        };

        let (latin_key, latin_char, prefer_latin) = latin_key(event, self.modifiers);
        if prefer_latin && let (Some(physical_key), Some(latin_char)) = (physical_key, latin_char) {
            self.egui_input
                .latin_layout_chars
                .push((physical_key, latin_char));
        }
        // The platforms winit reports Latin layouts on: X11 and Wayland.
        #[cfg(all(
            unix,
            not(target_vendor = "apple"),
            not(target_os = "android"),
            not(target_os = "emscripten"),
            not(target_os = "redox"),
            any(feature = "x11", feature = "wayland")
        ))]
        {
            use winit::platform::latin_layout::KeyEventExtLatinLayout as _;
            self.egui_input.keyboard_layout_is_latin = Some(event.is_latin_layout());
        }
        let logical_key = letter_key(
            key_from_winit_key(winit_logical_key),
            latin_key,
            prefer_latin,
        );

        // Helpful logging to enable when adding new key support
        log::trace!(
            "logical {:?} -> {:?},  physical {:?} -> {:?}",
            event.logical_key,
            logical_key,
            event.physical_key,
            physical_key
        );

        // "Logical OR physical key" is a fallback mechanism for keyboard layouts without Latin characters: it lets them
        // emit events as if the corresponding keys from the Latin layout were pressed. In this case, clipboard shortcuts
        // are mapped to the physical keys that normally contain C, X, V, etc.
        // See also: https://github.com/emilk/egui/issues/3653
        let translated = logical_key.or(physical_key);
        if pressed {
            if self.try_clipboard_shortcut(event.physical_key, translated) {
                return;
            }

            let _ = self.press_keyboard_input(event.physical_key, physical_key, translated);
        } else {
            self.release_keyboard_input(event.physical_key);
        }

        if let Some(text) = text
            .as_ref()
            .map(|t| t.as_str())
            .or_else(|| winit_logical_key.to_text())
        {
            // Make sure there is text, and that it is not control characters
            // (e.g. delete is sent as "\u{f728}" on macOS).
            if !text.is_empty() && text.chars().all(is_printable_char) {
                // On some platforms we get here when the user presses Cmd-C (copy), ctrl-W, etc.
                // We need to ignore these characters that are side-effects of commands.
                // Also make sure the key is pressed (not released). On Linux, text might
                // contain some data even when the key is released.
                let is_cmd =
                    self.modifiers.ctrl || self.modifiers.command || self.modifiers.mac_cmd;
                if pressed && !is_cmd {
                    self.egui_input
                        .events
                        .push(egui::Event::Text(text.to_owned()));
                }
            }
        }
    }

    /// Call with the output given by `egui`.
    ///
    /// Call this exactly once per accumulated logical batch (the `PlatformOutput` returned by
    /// one `Context::run`/`run_ui` call, including all of its layout passes). Rust cannot enforce
    /// that caller contract; submitting the same output twice legitimately duplicates an action.
    ///
    /// Within the batch, egui-winit executes only the last regular `CopyText`/`CopyImage` and,
    /// independently, the last `TextSelectionSettled`. This is an explicit last-emitted-command
    /// policy, not per-widget focus arbitration or exact per-command raw-input provenance. The
    /// final claimable input event captured before preparation is associated with both winning
    /// families; stale or failed captures fail explicitly.
    ///
    /// This will, if needed:
    /// * update the cursor
    /// * copy text to the clipboard
    /// * open any clicked urls
    /// * update the IME
    /// *
    pub fn handle_platform_output(
        &mut self,
        window: &Window,
        platform_output: egui::PlatformOutput,
    ) {
        self.handle_platform_output_inner(window, None, platform_output);
    }

    /// Same as [`Self::handle_platform_output`] but threads the
    /// `ActiveEventLoop` so we can register a `winit::CustomCursor` from
    /// `PlatformOutput::cursor_image`. Integration paths that don't have
    /// access to the event loop (e.g. immediate viewports) should call
    /// [`Self::handle_platform_output`] instead — any custom cursor
    /// request is silently dropped there and the standard `cursor_icon`
    /// path still runs.
    pub fn handle_platform_output_with_event_loop(
        &mut self,
        window: &Window,
        event_loop: &ActiveEventLoop,
        platform_output: egui::PlatformOutput,
    ) {
        self.handle_platform_output_inner(window, Some(event_loop), platform_output);
    }

    fn handle_platform_output_inner(
        &mut self,
        window: &Window,
        event_loop: Option<&ActiveEventLoop>,
        platform_output: egui::PlatformOutput,
    ) {
        profiling::function_scope!();

        let egui::PlatformOutput {
            commands,
            cursor_icon,
            cursor_image,
            events: _,                    // handled elsewhere
            mutable_text_under_cursor: _, // only used in eframe web
            ime,
            accesskit_update,
            num_completed_passes: _,    // `egui::Context::run` handles this
            request_discard_reasons: _, // `egui::Context::run` handles this
        } = platform_output;

        let selection = output_selector::select_clipboard_commands(&commands);
        let generation = self.clipboard_paste.generation();
        let primary_suppressed = selection.primary.is_some_and(|index| {
            let egui::OutputCommand::TextSelectionSettled(text) = &commands[index] else {
                return false;
            };
            self.primary_claim_is_suppressed(generation, text)
        });
        let regular_origin_policy = selection.regular.map(|regular| match regular.kind {
            output_selector::RegularClipboardKind::Text => {
                clipboard_bridge::RegularClaimOriginPolicy::AllowFreshFallback
            }
            output_selector::RegularClipboardKind::Image => {
                clipboard_bridge::RegularClaimOriginPolicy::RequireCapturedOrigin
            }
        });
        // A winning regular output supersedes any older pending image encode before origin
        // resolution. This happens even when the new attempt later fails. PRIMARY is independent.
        if selection.regular.is_some() {
            self.invalidate_image_copy_for_regular_output(window);
        }
        // Consume the prepared batch slot even if no clipboard command wins. Otherwise an unused
        // token from this input batch could be mistaken for a later programmatic copy's origin.
        #[cfg_attr(
            not(all(
                feature = "clipboard",
                feature = "wayland",
                any(
                    target_os = "linux",
                    target_os = "dragonfly",
                    target_os = "freebsd",
                    target_os = "netbsd",
                    target_os = "openbsd"
                ),
            )),
            expect(clippy::let_unit_value)
        )]
        let captured_claim_origin = self.take_clipboard_claim_origin();
        let mut claim_origins = self.resolve_output_claim_origins(
            window,
            generation,
            captured_claim_origin,
            regular_origin_policy,
            selection.primary.is_some() && !primary_suppressed,
        );

        for (index, command) in commands.into_iter().enumerate() {
            match command {
                egui::OutputCommand::CopyText(text)
                    if selection
                        .regular
                        .is_some_and(|regular| regular.index == index) =>
                {
                    let origin = Self::take_regular_output_origin(&mut claim_origins);
                    self.copy_text_output(window, text, origin);
                }
                egui::OutputCommand::TextSelectionSettled(text)
                    if selection.primary == Some(index) && !primary_suppressed =>
                {
                    let origin = Self::take_primary_output_origin(&mut claim_origins);
                    self.primary_selection_output(window, text, origin);
                }
                egui::OutputCommand::CopyImage(image)
                    if selection
                        .regular
                        .is_some_and(|regular| regular.index == index) =>
                {
                    let origin = Self::take_regular_output_origin(&mut claim_origins);
                    self.copy_image_output(window, image, origin);
                }
                // Superseded commands retain no payload or origin as a fallback.
                egui::OutputCommand::CopyText(_)
                | egui::OutputCommand::TextSelectionSettled(_)
                | egui::OutputCommand::CopyImage(_) => {}
                egui::OutputCommand::OpenUrl(open_url) => {
                    open_url_in_browser(&open_url.url);
                }
            }
        }

        self.apply_cursor(window, event_loop, cursor_icon, cursor_image.as_ref());

        // The bridge cfg compiles for both Wayland and X11 windows in the same winit build.
        // Only Wayland windows take part in the focus/external latch and surrounding-text cache.
        #[cfg(all(
            feature = "wayland",
            unix,
            not(target_vendor = "apple"),
            not(target_os = "android"),
            not(target_os = "emscripten"),
            not(target_os = "redox"),
        ))]
        let wayland_ime_window = ime_surrounding::is_wayland_window(window);

        let ime_requested = ime.is_some();
        #[cfg(all(
            feature = "wayland",
            unix,
            not(target_vendor = "apple"),
            not(target_os = "android"),
            not(target_os = "emscripten"),
            not(target_os = "redox"),
        ))]
        let (allow_ime, is_toggling_ime) = if wayland_ime_window {
            let transition = self.ime_gate.transition(ime_requested, self.allow_ime);
            (transition.allow, transition.changed)
        } else {
            (ime_requested, self.allow_ime != ime_requested)
        };
        #[cfg(not(all(
            feature = "wayland",
            unix,
            not(target_vendor = "apple"),
            not(target_os = "android"),
            not(target_os = "emscripten"),
            not(target_os = "redox"),
        )))]
        let (allow_ime, is_toggling_ime) = (ime_requested, self.allow_ime != ime_requested);
        #[cfg(all(
            feature = "wayland",
            unix,
            not(target_vendor = "apple"),
            not(target_os = "android"),
            not(target_os = "emscripten"),
            not(target_os = "redox"),
        ))]
        let mut ime_context_was_reset = is_toggling_ime;
        if is_toggling_ime {
            self.allow_ime = allow_ime;
            #[cfg(target_os = "windows")]
            if !self.allow_ime {
                // Defensively clear the set to avoid unexpected behavior.
                //
                // We don't do the same in `ime_event_disable` because the key
                // release events for IME confirmation keys arrive after
                // `winit::event::Ime::Disabled`.
                self.pressed_processed_physical_keys.clear();
            }

            profiling::scope!("set_ime_allowed");
            window.set_ime_allowed(allow_ime);
        }

        // This is settled output. Preparing a call does not synthesize an input marker.
        let reset_request =
            self.ime_reset
                .prepare(window.id(), self.viewport_id, ime.as_ref(), self.allow_ime);
        let stock_interrupt = reset_request.is_none()
            && self.allow_ime
            && !is_toggling_ime
            && ime
                .as_ref()
                .is_some_and(|ime| ime.should_interrupt_composition && ime.reset_request.is_none());
        if reset_request.is_some() || stock_interrupt {
            if let Some(request) = reset_request {
                // The stock winit 0.30 `Window` has no `reset_ime_with_request`; on Android the
                // IME is driven by the Activity's input view, so toggling is the reset.
                #[cfg(target_os = "android")]
                let _ = request;
                #[cfg(not(target_os = "android"))]
                window.reset_ime_with_request(request.as_u64());
            } else {
                // Preserve the stock untokened interruption operation and timing.
                window.set_ime_allowed(false);
                window.set_ime_allowed(true);
            }
            #[cfg(all(
                feature = "wayland",
                unix,
                not(target_vendor = "apple"),
                not(target_os = "android"),
                not(target_os = "emscripten"),
                not(target_os = "redox"),
            ))]
            {
                ime_context_was_reset = true;
            }
        }

        if let Some(ime) = &ime {
            if ime.purpose != self.old_ime_purpose {
                self.old_ime_purpose = ime.purpose;
                window.set_ime_purpose(to_winit_ime_purpose(ime.purpose));
            }

            let pixels_per_point = pixels_per_point(&self.egui_ctx, window);
            let ime_cursor_rect_px = pixels_per_point * ime.cursor_rect;
            if self.ime_cursor_rect_px != Some(ime_cursor_rect_px)
                || self.egui_ctx.input(|i| !i.events.is_empty())
            {
                self.ime_cursor_rect_px = Some(ime_cursor_rect_px);
                profiling::scope!("set_ime_cursor_area");
                window.set_ime_cursor_area(
                    winit::dpi::PhysicalPosition {
                        x: ime_cursor_rect_px.min.x,
                        y: ime_cursor_rect_px.min.y,
                    },
                    winit::dpi::PhysicalSize {
                        width: ime_cursor_rect_px.width(),
                        height: ime_cursor_rect_px.height(),
                    },
                );
            }
        } else {
            self.ime_cursor_rect_px = None;
        }

        #[cfg(all(
            feature = "wayland",
            unix,
            not(target_vendor = "apple"),
            not(target_os = "android"),
            not(target_os = "emscripten"),
            not(target_os = "redox"),
        ))]
        {
            if ime_context_was_reset {
                // Winit forgets its cached text on enable/disable and composition interrupt.
                self.ime_surrounding.forget_backend();
            }
            self.sync_ime_surrounding_text(window, ime.as_ref());
        }

        #[cfg(feature = "accesskit")]
        if let Some(accesskit) = self.accesskit.as_mut()
            && let Some(update) = accesskit_update
        {
            profiling::scope!("accesskit");
            accesskit.update_if_active(|| update);
        }

        #[cfg(not(feature = "accesskit"))]
        let _ = accesskit_update;
    }

    /// Apply either a bitmap cursor (preferred when both `cursor_image`
    /// and `event_loop` are `Some`) or the standard `cursor_icon` to the
    /// window. Mirrors the no-flicker dedupe the old `set_cursor_icon`
    /// did, on the appropriate cache key for whichever path is active.
    fn apply_cursor(
        &mut self,
        window: &Window,
        event_loop: Option<&ActiveEventLoop>,
        cursor_icon: egui::CursorIcon,
        cursor_image: Option<&egui::CustomCursorImage>,
    ) {
        let is_pointer_in_window = self.pointer_pos_in_points.is_some();
        if !is_pointer_in_window {
            // Drop both caches so the cursor gets re-applied (and the
            // bitmap re-checked for staleness) once the pointer comes
            // back. Same contract the old `set_cursor_icon` followed.
            self.current_cursor_icon = None;
            self.current_custom_cursor = None;
            return;
        }

        // Bitmap cursor wins over CursorIcon when both are present and we
        // have an event loop to register it with. Otherwise the bitmap is
        // dropped and we fall through to the icon path — this is the
        // documented fallback for integrations that didn't opt in.
        if let (Some(image), Some(event_loop)) = (cursor_image, event_loop) {
            let key = std::sync::Arc::as_ptr(&image.rgba).cast::<u8>() as usize;
            let cached = self
                .current_custom_cursor
                .as_ref()
                .filter(|(k, _)| *k == key)
                .map(|(_, c)| c.clone());

            let custom = match cached {
                Some(c) => c,
                None => match winit::window::CustomCursor::from_rgba(
                    image.rgba.to_vec(),
                    image.size[0],
                    image.size[1],
                    image.hotspot[0],
                    image.hotspot[1],
                ) {
                    Ok(source) => {
                        let c = event_loop.create_custom_cursor(source);
                        self.current_custom_cursor = Some((key, c.clone()));
                        c
                    }
                    Err(err) => {
                        log::warn!(
                            "egui-winit: invalid cursor bitmap, falling back to cursor_icon: {err:?}"
                        );
                        self.current_custom_cursor = None;
                        self.set_cursor_icon_inner(window, cursor_icon);
                        return;
                    }
                },
            };

            window.set_cursor_visible(true);
            window.set_cursor(custom);
            // Resync `current_cursor_icon` so the next icon-only path
            // notices a real change rather than dedupe-skipping it.
            self.current_cursor_icon = None;
            return;
        }

        self.current_custom_cursor = None;
        self.set_cursor_icon_inner(window, cursor_icon);
    }

    /// Icon-only path, factored out so `apply_cursor` can fall back to it
    /// when the bitmap path bails. Preserves the original dedupe.
    fn set_cursor_icon_inner(&mut self, window: &Window, cursor_icon: egui::CursorIcon) {
        if self.current_cursor_icon == Some(cursor_icon) {
            // Prevent flickering near frame boundary when Windows OS tries to control cursor icon for window resizing.
            // On other platforms: just early-out to save CPU.
            return;
        }

        self.current_cursor_icon = Some(cursor_icon);

        if let Some(winit_cursor_icon) = translate_cursor(cursor_icon) {
            window.set_cursor_visible(true);
            window.set_cursor(winit_cursor_icon);
        } else {
            window.set_cursor_visible(false);
        }

        // winit's Android `set_cursor` is a no-op: the icon the interface resolved never
        // reaches the screen. Forward it to the Activity's surface view as a system pointer
        // icon instead (I-beam, pointing hand, resize arrows, …). Only on icon change.
        #[cfg(target_os = "android")]
        crate::pointer_icon::apply(cursor_icon);
    }
}

fn to_egui_touch_phase(phase: winit::event::TouchPhase) -> egui::TouchPhase {
    match phase {
        winit::event::TouchPhase::Started => egui::TouchPhase::Start,
        winit::event::TouchPhase::Moved => egui::TouchPhase::Move,
        winit::event::TouchPhase::Ended => egui::TouchPhase::End,
        winit::event::TouchPhase::Cancelled => egui::TouchPhase::Cancel,
    }
}

fn to_egui_theme(theme: winit::window::Theme) -> Theme {
    match theme {
        winit::window::Theme::Dark => Theme::Dark,
        winit::window::Theme::Light => Theme::Light,
    }
}

pub fn inner_rect_in_points(window: &Window, pixels_per_point: f32) -> Option<Rect> {
    let inner_pos_px = window.inner_position().ok()?;
    let inner_pos_px = egui::pos2(inner_pos_px.x as f32, inner_pos_px.y as f32);

    let inner_size_px = window.inner_size();
    let inner_size_px = egui::vec2(inner_size_px.width as f32, inner_size_px.height as f32);

    let inner_rect_px = egui::Rect::from_min_size(inner_pos_px, inner_size_px);

    Some(inner_rect_px / pixels_per_point)
}

pub fn outer_rect_in_points(window: &Window, pixels_per_point: f32) -> Option<Rect> {
    let outer_pos_px = window.outer_position().ok()?;
    let outer_pos_px = egui::pos2(outer_pos_px.x as f32, outer_pos_px.y as f32);

    let outer_size_px = window.outer_size();
    let outer_size_px = egui::vec2(outer_size_px.width as f32, outer_size_px.height as f32);

    let outer_rect_px = egui::Rect::from_min_size(outer_pos_px, outer_size_px);

    Some(outer_rect_px / pixels_per_point)
}

/// Update the given viewport info with the current state of the window.
///
/// Call before [`State::take_egui_input`].
///
/// If this is called right after window creation, `is_init` should be `true`, otherwise `false`.
pub fn update_viewport_info(
    viewport_info: &mut ViewportInfo,
    egui_ctx: &egui::Context,
    window: &Window,
    is_init: bool,
) {
    profiling::function_scope!();
    let pixels_per_point = pixels_per_point(egui_ctx, window);

    let has_a_position = match window.is_minimized() {
        Some(true) => false,
        Some(false) | None => true,
    };

    let inner_rect = if has_a_position {
        inner_rect_in_points(window, pixels_per_point)
    } else {
        None
    };

    let outer_rect = if has_a_position {
        outer_rect_in_points(window, pixels_per_point)
    } else {
        None
    };

    let monitor_size = {
        profiling::scope!("monitor_size");
        if let Some(monitor) = window.current_monitor() {
            let size = monitor.size().to_logical::<f32>(pixels_per_point.into());
            Some(egui::vec2(size.width, size.height))
        } else {
            None
        }
    };

    viewport_info.title = Some(window.title());
    viewport_info.native_pixels_per_point = Some(window.scale_factor() as f32);

    viewport_info.monitor_size = monitor_size;
    viewport_info.inner_rect = inner_rect;
    viewport_info.outer_rect = outer_rect;

    if is_init || !cfg!(target_os = "macos") {
        // Asking for minimized/maximized state at runtime leads to a deadlock on Mac when running
        // `cargo run -p custom_window_frame`.
        // See https://github.com/emilk/egui/issues/3494
        viewport_info.maximized = Some(window.is_maximized());
        viewport_info.minimized = Some(window.is_minimized().unwrap_or(false));
    }

    viewport_info.fullscreen = Some(window.fullscreen().is_some());
    viewport_info.focused = Some(window.has_focus());
}

fn open_url_in_browser(_url: &str) {
    #[cfg(feature = "webbrowser")]
    if let Err(err) = webbrowser::open(_url) {
        log::warn!("Failed to open url: {err}");
    }

    #[cfg(not(feature = "webbrowser"))]
    {
        log::warn!("Cannot open url - feature \"links\" not enabled.");
    }
}

/// Winit sends special keys (backspace, delete, F1, …) as characters.
/// Ignore those.
/// We also ignore '\r', '\n', '\t'.
/// Newlines are handled by the `Key::Enter` event.
fn is_printable_char(chr: char) -> bool {
    let is_in_private_use_area = '\u{e000}' <= chr && chr <= '\u{f8ff}'
        || '\u{f0000}' <= chr && chr <= '\u{ffffd}'
        || '\u{100000}' <= chr && chr <= '\u{10fffd}';

    !is_in_private_use_area && !chr.is_ascii_control()
}

fn is_cut_command(modifiers: egui::Modifiers, keycode: egui::Key) -> bool {
    keycode == egui::Key::Cut
        || (modifiers.command_only() && keycode == egui::Key::X)
        || (cfg!(target_os = "windows") && modifiers.shift && keycode == egui::Key::Delete)
}

fn is_copy_command(modifiers: egui::Modifiers, keycode: egui::Key) -> bool {
    keycode == egui::Key::Copy
        || (modifiers.command_only() && keycode == egui::Key::C)
        || (cfg!(target_os = "windows") && modifiers.ctrl && keycode == egui::Key::Insert)
}

fn is_paste_command(modifiers: egui::Modifiers, keycode: egui::Key) -> bool {
    keycode == egui::Key::Paste
        || (modifiers.command_only() && keycode == egui::Key::V)
        || (cfg!(target_os = "windows") && modifiers.shift && keycode == egui::Key::Insert)
}

fn translate_mouse_button(button: winit::event::MouseButton) -> Option<egui::PointerButton> {
    match button {
        winit::event::MouseButton::Left => Some(egui::PointerButton::Primary),
        winit::event::MouseButton::Right => Some(egui::PointerButton::Secondary),
        winit::event::MouseButton::Middle => Some(egui::PointerButton::Middle),
        winit::event::MouseButton::Back => Some(egui::PointerButton::Extra1),
        winit::event::MouseButton::Forward => Some(egui::PointerButton::Extra2),
        winit::event::MouseButton::Other(_) => None,
    }
}

fn key_from_winit_key(key: &winit::keyboard::Key) -> Option<egui::Key> {
    match key {
        winit::keyboard::Key::Named(named_key) => key_from_named_key(*named_key),
        winit::keyboard::Key::Character(str) => egui::Key::from_name(str.as_str()),
        winit::keyboard::Key::Unidentified(_) | winit::keyboard::Key::Dead(_) => None,
    }
}

/// A letter key's egui key, for shortcuts.
///
/// `latin_key` is the Latin key the platform gives this key: the Windows virtual-key letter,
/// the macOS ⌘-table key, or the key on the first Latin layout on Linux. Its letter replaces
/// what the layout types when `prefer_latin`, and when the layout doesn't type an A–Z letter
/// there. Otherwise Latin layouts (Dvorak, AZERTY) keep the letters they type.
///
/// On layouts without Latin letters (Russian, Greek, Arabic, Hebrew, Thai) this makes Ctrl+C
/// match the key the platform calls C, as it does in native apps. When `prefer_latin` and the
/// Latin key is no letter, it stands in for a character egui has no key for: with Dvorak and
/// Russian, the key that types `й` is Dvorak's `'`, not the Q of its position.
fn letter_key(
    logical_key: Option<egui::Key>,
    latin_key: Option<egui::Key>,
    prefer_latin: bool,
) -> Option<egui::Key> {
    let is_letter = |key: &egui::Key| {
        let name = key.name();
        name.len() == 1 && name.as_bytes()[0].is_ascii_alphabetic()
    };
    let latin_letter = latin_key.filter(is_letter);
    if prefer_latin {
        latin_letter.or(logical_key).or(latin_key)
    } else {
        logical_key
            .filter(is_letter)
            .or(latin_letter)
            .or(logical_key)
    }
}

/// The platform's Latin key for this key, the character it stands for when shortcuts follow it
/// ([`egui::RawInput::latin_layout_chars`]), and whether it takes precedence over the letter the
/// layout types (see [`letter_key`]).
#[cfg(target_os = "windows")]
fn latin_key(
    event: &winit::event::KeyEvent,
    _modifiers: egui::Modifiers,
) -> (Option<egui::Key>, Option<char>, bool) {
    (
        windows_virtual_letter(windows_virtual_key(event.physical_key)),
        None,
        false,
    )
}

/// The text around the cursor an input method deletes before the [`winit::event::Ime::Commit`]
/// being handled, in bytes before and after the selection or pre-edit text.
fn ime_delete_surrounding(window: &Window) -> Option<(usize, usize)> {
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::WindowExtMacOS as _;
        window
            .take_ime_delete_surrounding()
            .map(|delete| (delete.before_bytes, delete.after_bytes))
    }
    #[cfg(all(
        feature = "wayland",
        unix,
        not(target_vendor = "apple"),
        not(target_os = "android"),
        not(target_os = "emscripten"),
        not(target_os = "redox"),
    ))]
    {
        use winit::platform::wayland::WindowExtWayland as _;
        window
            .take_ime_delete_surrounding()
            .map(|delete| (delete.before_bytes, delete.after_bytes))
    }
    #[cfg(not(any(
        target_os = "macos",
        all(
            feature = "wayland",
            unix,
            not(target_vendor = "apple"),
            not(target_os = "android"),
            not(target_os = "emscripten"),
            not(target_os = "redox"),
        )
    )))]
    {
        let _ = window;
        None
    }
}

/// With ⌘ held, letters come from the layout's ⌘ table, which differs from what the layout
/// types on non-Latin layouts and on layouts like "Dvorak – QWERTY ⌘".
#[cfg(target_os = "macos")]
fn latin_key(
    event: &winit::event::KeyEvent,
    modifiers: egui::Modifiers,
) -> (Option<egui::Key>, Option<char>, bool) {
    use winit::platform::macos::KeyEventExtMacOS as _;
    if modifiers.mac_cmd {
        let key = event.key_with_command();
        (key_from_winit_key(&key), key_char(&key), true)
    } else {
        (None, None, false)
    }
}

/// On a non-Latin layout, letters come from the first Latin layout the user has configured.
#[cfg(all(
    unix,
    not(target_vendor = "apple"),
    not(target_os = "android"),
    not(target_os = "emscripten"),
    not(target_os = "redox"),
    any(feature = "x11", feature = "wayland")
))]
fn latin_key(
    event: &winit::event::KeyEvent,
    _modifiers: egui::Modifiers,
) -> (Option<egui::Key>, Option<char>, bool) {
    use winit::platform::latin_layout::KeyEventExtLatinLayout as _;
    let key = event.latin_key();
    (
        key.as_ref().and_then(key_from_winit_key),
        key.as_ref().and_then(key_char),
        !event.is_latin_layout(),
    )
}

#[cfg(not(any(
    target_os = "windows",
    target_os = "macos",
    all(
        unix,
        not(target_vendor = "apple"),
        not(target_os = "android"),
        not(target_os = "emscripten"),
        not(target_os = "redox"),
        any(feature = "x11", feature = "wayland")
    )
)))]
fn latin_key(
    _event: &winit::event::KeyEvent,
    _modifiers: egui::Modifiers,
) -> (Option<egui::Key>, Option<char>, bool) {
    (None, None, false)
}

/// The character of a key that types exactly one.
#[cfg_attr(
    not(any(
        target_os = "macos",
        all(
            unix,
            not(target_vendor = "apple"),
            not(target_os = "android"),
            not(target_os = "emscripten"),
            not(target_os = "redox"),
            any(feature = "x11", feature = "wayland")
        )
    )),
    expect(dead_code)
)]
fn key_char(key: &winit::keyboard::Key) -> Option<char> {
    let mut chars = key.to_text()?.chars();
    let c = chars.next()?;
    chars.next().is_none().then_some(c)
}

/// The letter of a Windows virtual-key code: `VK_A` to `VK_Z` are the ASCII codes of 'A' to 'Z'.
#[cfg(any(target_os = "windows", test))]
fn windows_virtual_letter(virtual_key: Option<u32>) -> Option<egui::Key> {
    let c = char::from_u32(virtual_key?).filter(char::is_ascii_uppercase)?;
    egui::Key::from_name(c.encode_utf8(&mut [0; 4]))
}

/// The virtual-key code the current keyboard layout gives this physical key.
#[cfg(target_os = "windows")]
fn windows_virtual_key(physical_key: winit::keyboard::PhysicalKey) -> Option<u32> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyboardLayout, MAPVK_VSC_TO_VK_EX, MapVirtualKeyExW,
    };
    use winit::platform::scancode::PhysicalKeyExtScancode as _;

    let scancode = physical_key.to_scancode()?;
    // SAFETY: neither call takes pointers. Keyboard events arrive on the window's
    // thread, so `GetKeyboardLayout(0)` is the layout the key was typed with.
    #[expect(unsafe_code)]
    let virtual_key =
        unsafe { MapVirtualKeyExW(scancode, MAPVK_VSC_TO_VK_EX, GetKeyboardLayout(0)) };
    (virtual_key != 0).then_some(virtual_key)
}

fn key_from_named_key(named_key: winit::keyboard::NamedKey) -> Option<egui::Key> {
    use egui::Key;
    use winit::keyboard::NamedKey;

    Some(match named_key {
        NamedKey::Enter => Key::Enter,
        NamedKey::Tab => Key::Tab,
        NamedKey::ArrowDown => Key::ArrowDown,
        NamedKey::ArrowLeft => Key::ArrowLeft,
        NamedKey::ArrowRight => Key::ArrowRight,
        NamedKey::ArrowUp => Key::ArrowUp,
        NamedKey::End => Key::End,
        NamedKey::Home => Key::Home,
        NamedKey::PageDown => Key::PageDown,
        NamedKey::PageUp => Key::PageUp,
        NamedKey::Backspace => Key::Backspace,
        NamedKey::Delete => Key::Delete,
        NamedKey::Insert => Key::Insert,
        NamedKey::Escape => Key::Escape,
        NamedKey::Cut => Key::Cut,
        NamedKey::Copy => Key::Copy,
        NamedKey::Paste => Key::Paste,

        NamedKey::Space => Key::Space,

        NamedKey::F1 => Key::F1,
        NamedKey::F2 => Key::F2,
        NamedKey::F3 => Key::F3,
        NamedKey::F4 => Key::F4,
        NamedKey::F5 => Key::F5,
        NamedKey::F6 => Key::F6,
        NamedKey::F7 => Key::F7,
        NamedKey::F8 => Key::F8,
        NamedKey::F9 => Key::F9,
        NamedKey::F10 => Key::F10,
        NamedKey::F11 => Key::F11,
        NamedKey::F12 => Key::F12,
        NamedKey::F13 => Key::F13,
        NamedKey::F14 => Key::F14,
        NamedKey::F15 => Key::F15,
        NamedKey::F16 => Key::F16,
        NamedKey::F17 => Key::F17,
        NamedKey::F18 => Key::F18,
        NamedKey::F19 => Key::F19,
        NamedKey::F20 => Key::F20,
        NamedKey::F21 => Key::F21,
        NamedKey::F22 => Key::F22,
        NamedKey::F23 => Key::F23,
        NamedKey::F24 => Key::F24,
        NamedKey::F25 => Key::F25,
        NamedKey::F26 => Key::F26,
        NamedKey::F27 => Key::F27,
        NamedKey::F28 => Key::F28,
        NamedKey::F29 => Key::F29,
        NamedKey::F30 => Key::F30,
        NamedKey::F31 => Key::F31,
        NamedKey::F32 => Key::F32,
        NamedKey::F33 => Key::F33,
        NamedKey::F34 => Key::F34,
        NamedKey::F35 => Key::F35,

        NamedKey::BrowserBack => Key::BrowserBack,
        NamedKey::BrowserForward => Key::BrowserForward,
        NamedKey::BrowserRefresh => Key::BrowserRefresh,
        NamedKey::BrowserSearch => Key::BrowserSearch,
        NamedKey::BrowserHome => Key::BrowserHome,
        NamedKey::BrowserFavorites => Key::BrowserFavorites,
        NamedKey::BrowserStop => Key::BrowserStop,

        NamedKey::MediaPlayPause => Key::MediaPlayPause,
        NamedKey::MediaTrackNext => Key::MediaTrackNext,
        NamedKey::MediaTrackPrevious => Key::MediaTrackPrevious,
        NamedKey::MediaStop => Key::MediaStop,
        NamedKey::AudioVolumeMute => Key::AudioVolumeMute,
        NamedKey::AudioVolumeDown => Key::AudioVolumeDown,
        NamedKey::AudioVolumeUp => Key::AudioVolumeUp,

        NamedKey::LaunchMail => Key::LaunchMail,
        NamedKey::LaunchApplication1 => Key::LaunchApp1,
        NamedKey::LaunchApplication2 => Key::LaunchApp2,

        NamedKey::CapsLock => Key::CapsLock,
        NamedKey::NumLock => Key::NumLock,
        NamedKey::ScrollLock => Key::ScrollLock,
        NamedKey::PrintScreen => Key::PrintScreen,
        NamedKey::Pause => Key::Pause,
        NamedKey::ContextMenu => Key::Menu,

        NamedKey::Fn => Key::Fn,
        NamedKey::Eject => Key::Eject,
        NamedKey::Help => Key::Help,
        NamedKey::Power => Key::Power,
        NamedKey::Standby => Key::Sleep,
        NamedKey::Clear => Key::Clear,

        _ => {
            log::trace!("Unknown key: {named_key:?}");
            return None;
        }
    })
}

fn key_from_key_code(key: winit::keyboard::KeyCode) -> Option<egui::Key> {
    use egui::Key;
    use winit::keyboard::KeyCode;

    Some(match key {
        KeyCode::ArrowDown => Key::ArrowDown,
        KeyCode::ArrowLeft => Key::ArrowLeft,
        KeyCode::ArrowRight => Key::ArrowRight,
        KeyCode::ArrowUp => Key::ArrowUp,

        KeyCode::Escape => Key::Escape,
        KeyCode::Tab => Key::Tab,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Enter | KeyCode::NumpadEnter => Key::Enter,

        KeyCode::Insert => Key::Insert,
        KeyCode::Delete => Key::Delete,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,

        // Punctuation
        KeyCode::Space => Key::Space,
        KeyCode::Comma => Key::Comma,
        KeyCode::Period => Key::Period,
        // KeyCode::Colon => Key::Colon, // NOTE: there is no physical colon key on an american keyboard
        KeyCode::Semicolon => Key::Semicolon,
        KeyCode::Backslash => Key::Backslash,
        KeyCode::Slash | KeyCode::NumpadDivide => Key::Slash,
        KeyCode::BracketLeft => Key::OpenBracket,
        KeyCode::BracketRight => Key::CloseBracket,
        KeyCode::Backquote => Key::Backtick,
        KeyCode::Quote => Key::Quote,

        KeyCode::Cut => Key::Cut,
        KeyCode::Copy => Key::Copy,
        KeyCode::Paste => Key::Paste,
        KeyCode::Minus | KeyCode::NumpadSubtract => Key::Minus,
        KeyCode::NumpadAdd => Key::Plus,
        KeyCode::Equal => Key::Equals,

        KeyCode::Digit0 | KeyCode::Numpad0 => Key::Num0,
        KeyCode::Digit1 | KeyCode::Numpad1 => Key::Num1,
        KeyCode::Digit2 | KeyCode::Numpad2 => Key::Num2,
        KeyCode::Digit3 | KeyCode::Numpad3 => Key::Num3,
        KeyCode::Digit4 | KeyCode::Numpad4 => Key::Num4,
        KeyCode::Digit5 | KeyCode::Numpad5 => Key::Num5,
        KeyCode::Digit6 | KeyCode::Numpad6 => Key::Num6,
        KeyCode::Digit7 | KeyCode::Numpad7 => Key::Num7,
        KeyCode::Digit8 | KeyCode::Numpad8 => Key::Num8,
        KeyCode::Digit9 | KeyCode::Numpad9 => Key::Num9,

        KeyCode::KeyA => Key::A,
        KeyCode::KeyB => Key::B,
        KeyCode::KeyC => Key::C,
        KeyCode::KeyD => Key::D,
        KeyCode::KeyE => Key::E,
        KeyCode::KeyF => Key::F,
        KeyCode::KeyG => Key::G,
        KeyCode::KeyH => Key::H,
        KeyCode::KeyI => Key::I,
        KeyCode::KeyJ => Key::J,
        KeyCode::KeyK => Key::K,
        KeyCode::KeyL => Key::L,
        KeyCode::KeyM => Key::M,
        KeyCode::KeyN => Key::N,
        KeyCode::KeyO => Key::O,
        KeyCode::KeyP => Key::P,
        KeyCode::KeyQ => Key::Q,
        KeyCode::KeyR => Key::R,
        KeyCode::KeyS => Key::S,
        KeyCode::KeyT => Key::T,
        KeyCode::KeyU => Key::U,
        KeyCode::KeyV => Key::V,
        KeyCode::KeyW => Key::W,
        KeyCode::KeyX => Key::X,
        KeyCode::KeyY => Key::Y,
        KeyCode::KeyZ => Key::Z,

        KeyCode::F1 => Key::F1,
        KeyCode::F2 => Key::F2,
        KeyCode::F3 => Key::F3,
        KeyCode::F4 => Key::F4,
        KeyCode::F5 => Key::F5,
        KeyCode::F6 => Key::F6,
        KeyCode::F7 => Key::F7,
        KeyCode::F8 => Key::F8,
        KeyCode::F9 => Key::F9,
        KeyCode::F10 => Key::F10,
        KeyCode::F11 => Key::F11,
        KeyCode::F12 => Key::F12,
        KeyCode::F13 => Key::F13,
        KeyCode::F14 => Key::F14,
        KeyCode::F15 => Key::F15,
        KeyCode::F16 => Key::F16,
        KeyCode::F17 => Key::F17,
        KeyCode::F18 => Key::F18,
        KeyCode::F19 => Key::F19,
        KeyCode::F20 => Key::F20,
        KeyCode::F21 => Key::F21,
        KeyCode::F22 => Key::F22,
        KeyCode::F23 => Key::F23,
        KeyCode::F24 => Key::F24,
        KeyCode::F25 => Key::F25,
        KeyCode::F26 => Key::F26,
        KeyCode::F27 => Key::F27,
        KeyCode::F28 => Key::F28,
        KeyCode::F29 => Key::F29,
        KeyCode::F30 => Key::F30,
        KeyCode::F31 => Key::F31,
        KeyCode::F32 => Key::F32,
        KeyCode::F33 => Key::F33,
        KeyCode::F34 => Key::F34,
        KeyCode::F35 => Key::F35,

        // Modifier keys — egui now surfaces them as distinct physical
        // variants so games / capture UIs can bind them independently.
        // The collapsed `Modifiers.shift/ctrl/alt/command` booleans still
        // track just the "any side is pressed" state for shortcut matching.
        KeyCode::ShiftLeft => Key::ShiftLeft,
        KeyCode::ShiftRight => Key::ShiftRight,
        KeyCode::ControlLeft => Key::ControlLeft,
        KeyCode::ControlRight => Key::ControlRight,
        KeyCode::AltLeft => Key::AltLeft,
        KeyCode::AltRight => Key::AltRight,
        KeyCode::SuperLeft => Key::SuperLeft,
        KeyCode::SuperRight => Key::SuperRight,

        // ISO 102nd key — `<>|` on French AZERTY, `\|` on UK QWERTY.
        KeyCode::IntlBackslash => Key::IntlBackslash,

        // Lock / System keys:
        KeyCode::CapsLock => Key::CapsLock,
        KeyCode::NumLock => Key::NumLock,
        KeyCode::ScrollLock => Key::ScrollLock,
        KeyCode::PrintScreen => Key::PrintScreen,
        KeyCode::Pause => Key::Pause,
        KeyCode::ContextMenu => Key::Menu,

        // Browser keys:
        KeyCode::BrowserBack => Key::BrowserBack,
        KeyCode::BrowserForward => Key::BrowserForward,
        KeyCode::BrowserRefresh => Key::BrowserRefresh,
        KeyCode::BrowserSearch => Key::BrowserSearch,
        KeyCode::BrowserHome => Key::BrowserHome,
        KeyCode::BrowserFavorites => Key::BrowserFavorites,
        KeyCode::BrowserStop => Key::BrowserStop,

        // Media keys:
        KeyCode::MediaPlayPause => Key::MediaPlayPause,
        KeyCode::MediaTrackNext => Key::MediaTrackNext,
        KeyCode::MediaTrackPrevious => Key::MediaTrackPrevious,
        KeyCode::MediaStop => Key::MediaStop,
        KeyCode::AudioVolumeMute => Key::AudioVolumeMute,
        KeyCode::AudioVolumeDown => Key::AudioVolumeDown,
        KeyCode::AudioVolumeUp => Key::AudioVolumeUp,

        // Launch keys:
        KeyCode::LaunchMail => Key::LaunchMail,
        KeyCode::LaunchApp1 => Key::LaunchApp1,
        KeyCode::LaunchApp2 => Key::LaunchApp2,

        // Mac / other system keys:
        KeyCode::Fn => Key::Fn,
        KeyCode::Eject => Key::Eject,
        KeyCode::Help => Key::Help,
        KeyCode::Power => Key::Power,
        KeyCode::Sleep => Key::Sleep,

        _ => {
            return None;
        }
    })
}

fn translate_cursor(cursor_icon: egui::CursorIcon) -> Option<winit::window::CursorIcon> {
    match cursor_icon {
        egui::CursorIcon::None => None,

        egui::CursorIcon::Alias => Some(winit::window::CursorIcon::Alias),
        egui::CursorIcon::AllScroll => Some(winit::window::CursorIcon::AllScroll),
        egui::CursorIcon::Cell => Some(winit::window::CursorIcon::Cell),
        egui::CursorIcon::ContextMenu => Some(winit::window::CursorIcon::ContextMenu),
        egui::CursorIcon::Copy => Some(winit::window::CursorIcon::Copy),
        egui::CursorIcon::Crosshair => Some(winit::window::CursorIcon::Crosshair),
        egui::CursorIcon::Default => Some(winit::window::CursorIcon::Default),
        egui::CursorIcon::Grab => Some(winit::window::CursorIcon::Grab),
        egui::CursorIcon::Grabbing => Some(winit::window::CursorIcon::Grabbing),
        egui::CursorIcon::Help => Some(winit::window::CursorIcon::Help),
        egui::CursorIcon::Move => Some(winit::window::CursorIcon::Move),
        egui::CursorIcon::NoDrop => Some(winit::window::CursorIcon::NoDrop),
        egui::CursorIcon::NotAllowed => Some(winit::window::CursorIcon::NotAllowed),
        egui::CursorIcon::PointingHand => Some(winit::window::CursorIcon::Pointer),
        egui::CursorIcon::Progress => Some(winit::window::CursorIcon::Progress),

        egui::CursorIcon::ResizeHorizontal => Some(winit::window::CursorIcon::EwResize),
        egui::CursorIcon::ResizeNeSw => Some(winit::window::CursorIcon::NeswResize),
        egui::CursorIcon::ResizeNwSe => Some(winit::window::CursorIcon::NwseResize),
        egui::CursorIcon::ResizeVertical => Some(winit::window::CursorIcon::NsResize),

        egui::CursorIcon::ResizeEast => Some(winit::window::CursorIcon::EResize),
        egui::CursorIcon::ResizeSouthEast => Some(winit::window::CursorIcon::SeResize),
        egui::CursorIcon::ResizeSouth => Some(winit::window::CursorIcon::SResize),
        egui::CursorIcon::ResizeSouthWest => Some(winit::window::CursorIcon::SwResize),
        egui::CursorIcon::ResizeWest => Some(winit::window::CursorIcon::WResize),
        egui::CursorIcon::ResizeNorthWest => Some(winit::window::CursorIcon::NwResize),
        egui::CursorIcon::ResizeNorth => Some(winit::window::CursorIcon::NResize),
        egui::CursorIcon::ResizeNorthEast => Some(winit::window::CursorIcon::NeResize),
        egui::CursorIcon::ResizeColumn => Some(winit::window::CursorIcon::ColResize),
        egui::CursorIcon::ResizeRow => Some(winit::window::CursorIcon::RowResize),

        egui::CursorIcon::Text => Some(winit::window::CursorIcon::Text),
        egui::CursorIcon::VerticalText => Some(winit::window::CursorIcon::VerticalText),
        egui::CursorIcon::Wait => Some(winit::window::CursorIcon::Wait),
        egui::CursorIcon::ZoomIn => Some(winit::window::CursorIcon::ZoomIn),
        egui::CursorIcon::ZoomOut => Some(winit::window::CursorIcon::ZoomOut),
    }
}

// Helpers for egui Viewports
// ---------------------------------------------------------------------------
#[derive(Debug)]
pub enum ActionRequested {
    Screenshot(egui::ScreenshotCallback),
    Cut,
    Copy,
    Paste,

    /// Run `App::ui` and paint a frame even while the window is hidden (minimized or occluded).
    ///
    /// This is useful when the UI needs to keep running without anything currently visible on
    /// screen. For example, a tool can use this to drive an app in the background:
    /// `egui_inspection` includes it with every request so that a hidden app still runs its UI,
    /// paints the screenshot, rebuilds the widget tree, and applies injected input.
    ///
    /// See [`egui::ViewportCommand::RequestPaintWhileHidden`].
    PaintWhileHidden,
}

impl ActionRequested {
    /// Does this need a painted frame, even from a window that is hidden?
    ///
    /// A screenshot of a hidden window is still a screenshot of something, and painting is
    /// the only way to produce it.
    pub fn wants_paint(&self) -> bool {
        matches!(self, Self::Screenshot(_) | Self::PaintWhileHidden)
    }
}

/// Apply viewport commands to a window and its owning [`State`].
///
/// # Fork API difference
///
/// Upstream egui's helper takes only `window`. This fork requires `state` so an external
/// [`ViewportCommand::IMEAllowed`] can update that window's IME gate and surrounding-text cache
/// synchronously, without a process-global pending-command map or a later reconciliation pass.
///
/// # Migration
///
/// Pass the [`State`] created for this window after `window`, for example:
/// `process_viewport_commands(ctx, &mut info, commands, &window, &mut state, &mut actions)`.
/// Integrations must not process commands before both the window and its [`State`] exist. In
/// particular, eframe keeps commands in `Viewport::deferred_commands` until both are present.
pub fn process_viewport_commands(
    egui_ctx: &egui::Context,
    info: &mut ViewportInfo,
    commands: impl IntoIterator<Item = ViewportCommand>,
    window: &Window,
    state: &mut State,
    actions_requested: &mut Vec<ActionRequested>,
) {
    for command in commands {
        process_viewport_command(egui_ctx, window, state, command, info, actions_requested);
    }
}

fn process_viewport_command(
    egui_ctx: &egui::Context,
    window: &Window,
    state: &mut State,
    command: ViewportCommand,
    info: &mut ViewportInfo,
    actions_requested: &mut Vec<ActionRequested>,
) {
    profiling::function_scope!(&format!("{command:?}"));

    use winit::window::ResizeDirection;

    log::trace!("Processing ViewportCommand::{command:?}");

    let pixels_per_point = pixels_per_point(egui_ctx, window);

    match command {
        ViewportCommand::Close => {
            info.events.push(egui::ViewportEvent::Close);
        }
        ViewportCommand::CancelClose => {
            // Need to be handled elsewhere
        }
        ViewportCommand::StartDrag => {
            // If `.has_focus()` is not checked on x11 the input will be permanently taken until the app is killed!
            if window.has_focus()
                && let Err(err) = window.drag_window()
            {
                log::warn!("{command:?}: {err}");
            }
        }
        ViewportCommand::InnerSize(size) => {
            let width_px = pixels_per_point * size.x.max(1.0);
            let height_px = pixels_per_point * size.y.max(1.0);
            let requested_size = PhysicalSize::new(width_px, height_px);
            if let Some(_returned_inner_size) = window.request_inner_size(requested_size) {
                // On platforms where the size is entirely controlled by the user the
                // applied size will be returned immediately, resize event in such case
                // may not be generated.
                // e.g. Linux

                // On platforms where resizing is disallowed by the windowing system, the current
                // inner size is returned immediately, and the user one is ignored.
                // e.g. Android, iOS, …

                // However, comparing the results is prone to numerical errors
                // because the linux backend converts physical to logical and back again.
                // So let's just assume it worked:

                info.inner_rect = inner_rect_in_points(window, pixels_per_point);
                info.outer_rect = outer_rect_in_points(window, pixels_per_point);
            } else {
                // e.g. macOS, Windows
                // The request went to the display system,
                // and the actual size will be delivered later with the [`WindowEvent::Resized`].
            }
        }
        ViewportCommand::BeginResize(direction) => {
            if let Err(err) = window.drag_resize_window(match direction {
                egui::viewport::ResizeDirection::North => ResizeDirection::North,
                egui::viewport::ResizeDirection::South => ResizeDirection::South,
                egui::viewport::ResizeDirection::East => ResizeDirection::East,
                egui::viewport::ResizeDirection::West => ResizeDirection::West,
                egui::viewport::ResizeDirection::NorthEast => ResizeDirection::NorthEast,
                egui::viewport::ResizeDirection::SouthEast => ResizeDirection::SouthEast,
                egui::viewport::ResizeDirection::NorthWest => ResizeDirection::NorthWest,
                egui::viewport::ResizeDirection::SouthWest => ResizeDirection::SouthWest,
            }) {
                log::warn!("{command:?}: {err}");
            }
        }
        ViewportCommand::Title(title) => {
            window.set_title(&title);
        }
        ViewportCommand::Transparent(v) => window.set_transparent(v),
        ViewportCommand::Visible(v) => {
            window.set_visible(v);
            if !v {
                // A hidden window must not apply a paste that completes later.
                state.cancel_pending_clipboard_pastes();
            }
        }
        ViewportCommand::OuterPosition(pos) => {
            window.set_outer_position(PhysicalPosition::new(
                pixels_per_point * pos.x,
                pixels_per_point * pos.y,
            ));
        }
        ViewportCommand::MinInnerSize(s) => {
            window.set_min_inner_size((s.is_finite() && s != Vec2::ZERO).then_some(
                PhysicalSize::new(pixels_per_point * s.x, pixels_per_point * s.y),
            ));
        }
        ViewportCommand::MaxInnerSize(s) => {
            window.set_max_inner_size((s.is_finite() && s != Vec2::INFINITY).then_some(
                PhysicalSize::new(pixels_per_point * s.x, pixels_per_point * s.y),
            ));
        }
        ViewportCommand::ResizeIncrements(s) => {
            window.set_resize_increments(
                s.map(|s| PhysicalSize::new(pixels_per_point * s.x, pixels_per_point * s.y)),
            );
        }
        ViewportCommand::Resizable(v) => window.set_resizable(v),
        ViewportCommand::EnableButtons {
            close,
            minimized,
            maximize,
        } => window.set_enabled_buttons(
            if close {
                WindowButtons::CLOSE
            } else {
                WindowButtons::empty()
            } | if minimized {
                WindowButtons::MINIMIZE
            } else {
                WindowButtons::empty()
            } | if maximize {
                WindowButtons::MAXIMIZE
            } else {
                WindowButtons::empty()
            },
        ),
        ViewportCommand::Minimized(v) => {
            window.set_minimized(v);
            info.minimized = Some(v);
            if v {
                // A minimized window must not apply a paste that completes later.
                state.cancel_pending_clipboard_pastes();
            }
        }
        ViewportCommand::Maximized(v) => {
            window.set_maximized(v);
            info.maximized = Some(v);
        }
        ViewportCommand::Fullscreen(v) => {
            window.set_fullscreen(v.then_some(winit::window::Fullscreen::Borderless(None)));
        }
        ViewportCommand::SetMonitor(idx) => {
            if let Some(monitor) = window.available_monitors().nth(idx) {
                window.set_fullscreen(Some(winit::window::Fullscreen::Borderless(Some(monitor))));
            } else {
                log::warn!(
                    "ViewportCommand::SetMonitor({idx}): index out of range ({} monitors available)",
                    window.available_monitors().count()
                );
            }
        }
        ViewportCommand::Decorations(v) => {
            window.set_decorations(v);
            #[cfg(target_os = "windows")]
            {
                use winit::platform::windows::WindowExtWindows as _;

                // don't request the undecorated-window drop shadow in fullscreen (#8399)
                window.set_undecorated_shadow(!v && window.fullscreen().is_none());
            }
        }
        ViewportCommand::WindowLevel(l) => window.set_window_level(match l {
            egui::viewport::WindowLevel::AlwaysOnBottom => WindowLevel::AlwaysOnBottom,
            egui::viewport::WindowLevel::AlwaysOnTop => WindowLevel::AlwaysOnTop,
            egui::viewport::WindowLevel::Normal => WindowLevel::Normal,
        }),
        ViewportCommand::Icon(icon) => {
            let winit_icon = icon.and_then(|icon| to_winit_icon(&icon));
            window.set_window_icon(winit_icon);
        }
        ViewportCommand::IMERect(rect) => {
            window.set_ime_cursor_area(
                PhysicalPosition::new(pixels_per_point * rect.min.x, pixels_per_point * rect.min.y),
                PhysicalSize::new(
                    pixels_per_point * rect.size().x,
                    pixels_per_point * rect.size().y,
                ),
            );
        }
        ViewportCommand::IMEAllowed(v) => {
            state.handle_external_ime_allowed(window, v);
        }
        ViewportCommand::IMEPurpose(p) => {
            if state
                .ime_reset
                .apply_purpose_command(&mut state.old_ime_purpose, p)
            {
                window.set_ime_purpose(to_winit_ime_purpose(p));
                #[cfg(all(
                    feature = "wayland",
                    unix,
                    not(target_vendor = "apple"),
                    not(target_os = "android"),
                    not(target_os = "emscripten"),
                    not(target_os = "redox"),
                ))]
                if ime_surrounding::is_wayland_window(window) {
                    // A changed purpose is a real context boundary. An unchanged command above
                    // is a no-op and keeps the bridge cache, including a later owner/generation
                    // transition's ability to plan `ClearThenSet`.
                    state.ime_surrounding.forget_backend();
                }
            }
        }
        ViewportCommand::Focus => {
            if !window.has_focus() {
                window.focus_window();
            }
        }
        ViewportCommand::RequestUserAttention(a) => {
            window.request_user_attention(match a {
                egui::UserAttentionType::Reset => None,
                egui::UserAttentionType::Critical => {
                    Some(winit::window::UserAttentionType::Critical)
                }
                egui::UserAttentionType::Informational => {
                    Some(winit::window::UserAttentionType::Informational)
                }
            });
        }
        ViewportCommand::SetTheme(t) => {
            window.set_theme(match t {
                egui::SystemTheme::Light => Some(winit::window::Theme::Light),
                egui::SystemTheme::Dark => Some(winit::window::Theme::Dark),
                egui::SystemTheme::SystemDefault => None,
            });

            #[cfg(target_os = "windows")]
            {
                refresh_windows_non_client_activation(window);
            }
        }
        ViewportCommand::ContentProtected(v) => window.set_content_protected(v),
        ViewportCommand::CursorPosition(pos) => {
            if let Err(err) = window.set_cursor_position(PhysicalPosition::new(
                pixels_per_point * pos.x,
                pixels_per_point * pos.y,
            )) {
                log::warn!("{command:?}: {err}");
            }
        }
        ViewportCommand::CursorGrab(o) => {
            if let Err(err) = window.set_cursor_grab(match o {
                egui::viewport::CursorGrab::None => CursorGrabMode::None,
                egui::viewport::CursorGrab::Confined => CursorGrabMode::Confined,
                egui::viewport::CursorGrab::Locked => CursorGrabMode::Locked,
            }) {
                log::warn!("{command:?}: {err}");
            }
        }
        ViewportCommand::CursorVisible(v) => window.set_cursor_visible(v),
        ViewportCommand::MousePassthrough(passthrough) => {
            if let Err(err) = window.set_cursor_hittest(!passthrough) {
                log::warn!("{command:?}: {err}");
            }
        }
        ViewportCommand::Screenshot(callback) => {
            actions_requested.push(ActionRequested::Screenshot(callback));
        }
        ViewportCommand::RequestPaintWhileHidden => {
            actions_requested.push(ActionRequested::PaintWhileHidden);
        }
        ViewportCommand::RequestCut => {
            actions_requested.push(ActionRequested::Cut);
        }
        ViewportCommand::RequestCopy => {
            actions_requested.push(ActionRequested::Copy);
        }
        ViewportCommand::RequestPaste => {
            actions_requested.push(ActionRequested::Paste);
        }
    }
}

fn to_winit_ime_purpose(purpose: egui::IMEPurpose) -> winit::window::ImePurpose {
    match purpose {
        egui::IMEPurpose::Password => winit::window::ImePurpose::Password,
        egui::IMEPurpose::Terminal => winit::window::ImePurpose::Terminal,
        egui::IMEPurpose::Normal => winit::window::ImePurpose::Normal,
    }
}

/// Build and intitlaize a window.
///
/// Wrapper around `create_winit_window_builder` and `apply_viewport_builder_to_window`.
///
/// # Errors
/// Possible causes of error include denied permission, incompatible system, and lack of memory.
pub fn create_window(
    egui_ctx: &egui::Context,
    event_loop: &ActiveEventLoop,
    viewport_builder: &ViewportBuilder,
) -> Result<Window, winit::error::OsError> {
    profiling::function_scope!();

    let window_attributes = apply_monitor_to_window_attributes(
        create_winit_window_attributes(egui_ctx, viewport_builder.clone()),
        viewport_builder,
        event_loop,
    );

    let window = event_loop.create_window(window_attributes)?;
    apply_viewport_builder_to_window(egui_ctx, &window, viewport_builder);
    Ok(window)
}

/// Apply [`ViewportBuilder::with_monitor`] to freshly-built [`winit::window::WindowAttributes`].
///
/// Resolve the target monitor index → `MonitorHandle` and request borderless
/// fullscreen on that output, so the window is created directly on the right
/// monitor. This is the only reliable way to target a specific monitor under
/// Wayland, and also avoids the Mutter race where `OuterPosition` is ignored
/// pre-mapping.
///
/// Must be called by every backend that builds its own window from
/// [`create_winit_window_attributes`] (the glow backend and per-viewport window
/// creation do this) — otherwise `with_monitor` silently does nothing there.
pub fn apply_monitor_to_window_attributes(
    mut window_attributes: winit::window::WindowAttributes,
    viewport_builder: &ViewportBuilder,
    event_loop: &ActiveEventLoop,
) -> winit::window::WindowAttributes {
    if let Some(idx) = viewport_builder.monitor {
        if let Some(monitor) = event_loop.available_monitors().nth(idx) {
            window_attributes = window_attributes
                .with_fullscreen(Some(winit::window::Fullscreen::Borderless(Some(monitor))));
        } else {
            log::warn!(
                "ViewportBuilder::with_monitor({idx}): index out of range ({} monitors available)",
                event_loop.available_monitors().count()
            );
        }
    }
    window_attributes
}

pub fn create_winit_window_attributes(
    egui_ctx: &egui::Context,
    viewport_builder: ViewportBuilder,
) -> winit::window::WindowAttributes {
    profiling::function_scope!();

    let ViewportBuilder {
        title,
        position,
        inner_size,
        min_inner_size,
        max_inner_size,
        fullscreen,
        maximized,
        resizable,
        transparent,
        decorations,
        icon,
        active,
        visible,
        close_button,
        minimize_button,
        maximize_button,
        window_level,

        // macOS:
        fullsize_content_view: _fullsize_content_view,
        movable_by_window_background: _movable_by_window_background,
        title_shown: _title_shown,
        titlebar_buttons_shown: _titlebar_buttons_shown,
        titlebar_shown: _titlebar_shown,
        has_shadow: _has_shadow,

        // Windows:
        drag_and_drop: _drag_and_drop,
        taskbar: _taskbar,

        // wayland:
        app_id: _app_id,

        // x11
        window_type: _window_type,
        override_redirect: _override_redirect,

        mouse_passthrough: _, // handled in `apply_viewport_builder_to_window`
        clamp_size_to_monitor_size: _, // Handled in `viewport_builder` in `epi_integration.rs`
        monitor: _, // Handled in `create_window` (needs ActiveEventLoop for monitor handle)
    } = viewport_builder;

    let mut window_attributes = winit::window::WindowAttributes::default()
        .with_title(title.unwrap_or_else(|| "egui window".to_owned()))
        .with_transparent(transparent.unwrap_or(false))
        .with_decorations(decorations.unwrap_or(true))
        .with_resizable(resizable.unwrap_or(true))
        .with_visible(visible.unwrap_or(true))
        .with_maximized(if cfg!(target_os = "ios") {
            true
        } else if cfg!(target_os = "windows") && visible == Some(false) {
            // Maximizing a hidden window on Windows briefly shows it.
            // Deferred to `apply_viewport_builder_to_window`, which hides that flash.
            false
        } else {
            maximized.unwrap_or(false)
        })
        .with_window_level(match window_level.unwrap_or_default() {
            egui::viewport::WindowLevel::AlwaysOnBottom => WindowLevel::AlwaysOnBottom,
            egui::viewport::WindowLevel::AlwaysOnTop => WindowLevel::AlwaysOnTop,
            egui::viewport::WindowLevel::Normal => WindowLevel::Normal,
        })
        .with_fullscreen(
            fullscreen.and_then(|e| e.then_some(winit::window::Fullscreen::Borderless(None))),
        )
        .with_enabled_buttons({
            let mut buttons = WindowButtons::empty();
            if minimize_button.unwrap_or(true) {
                buttons |= WindowButtons::MINIMIZE;
            }
            if maximize_button.unwrap_or(true) {
                buttons |= WindowButtons::MAXIMIZE;
            }
            if close_button.unwrap_or(true) {
                buttons |= WindowButtons::CLOSE;
            }
            buttons
        })
        .with_active(active.unwrap_or(true));

    // Here and below: we create `LogicalSize` / `LogicalPosition` taking
    // zoom factor into account. We don't have a good way to get physical size here,
    // and trying to do it anyway leads to weird bugs on Wayland, see:
    // https://github.com/emilk/egui/issues/7095#issuecomment-2920545377
    // https://github.com/rust-windowing/winit/issues/4266
    #[expect(
        clippy::disallowed_types,
        reason = "zoom factor is manually accounted for"
    )]
    #[cfg(not(target_os = "ios"))]
    {
        use winit::dpi::{LogicalPosition, LogicalSize};
        let zoom_factor = egui_ctx.zoom_factor();

        if let Some(size) = inner_size {
            window_attributes = window_attributes
                .with_inner_size(LogicalSize::new(zoom_factor * size.x, zoom_factor * size.y));
        }

        if let Some(size) = min_inner_size {
            window_attributes = window_attributes
                .with_min_inner_size(LogicalSize::new(zoom_factor * size.x, zoom_factor * size.y));
        }

        if let Some(size) = max_inner_size {
            window_attributes = window_attributes
                .with_max_inner_size(LogicalSize::new(zoom_factor * size.x, zoom_factor * size.y));
        }

        if let Some(pos) = position {
            window_attributes = window_attributes.with_position(LogicalPosition::new(
                zoom_factor * pos.x,
                zoom_factor * pos.y,
            ));
        }
    }
    #[cfg(target_os = "ios")]
    {
        // Unused:
        _ = egui_ctx;
        _ = pixels_per_point;
        _ = position;
        _ = inner_size;
        _ = min_inner_size;
        _ = max_inner_size;
    }

    if let Some(icon) = icon {
        let winit_icon = to_winit_icon(&icon);
        window_attributes = window_attributes.with_window_icon(winit_icon);
    }

    #[cfg(all(feature = "wayland", target_os = "linux"))]
    if let Some(app_id) = _app_id {
        use winit::platform::wayland::WindowAttributesExtWayland as _;
        window_attributes = window_attributes.with_name(app_id, "");
    }

    // Consume the activation token our launcher handed us, so the first
    // window actually gets the focus.
    //
    // A desktop entry with `StartupNotify=true` passes a token through
    // `XDG_ACTIVATION_TOKEN` (Wayland) or `DESKTOP_STARTUP_ID` (X11), and
    // winit can only apply it at window creation. eframe never read it, so
    // under a compositor that enforces focus-stealing prevention the window
    // opened unfocused and stayed that way: `ViewportCommand::Focus` is
    // exactly the request such a compositor refuses, so nothing could
    // recover it and the user had to click the window themselves.
    //
    // The variables are cleared once read, per the startup-notification
    // spec: a token is single-use, and leaving it in the environment would
    // have every later window — and every child process — replay it.
    #[cfg(all(target_os = "linux", any(feature = "wayland", feature = "x11")))]
    {
        use winit::platform::startup_notify::{
            WindowAttributesExtStartupNotify as _, reset_activation_token_env,
        };
        let token = std::env::var("XDG_ACTIVATION_TOKEN")
            .or_else(|_| std::env::var("DESKTOP_STARTUP_ID"))
            .ok()
            .filter(|t| !t.is_empty());
        if let Some(token) = token {
            log::debug!("using the activation token from the environment to focus the window");
            reset_activation_token_env();
            window_attributes = window_attributes
                .with_activation_token(winit::window::ActivationToken::from_raw(token));
        }
    }

    #[cfg(all(feature = "x11", target_os = "linux"))]
    {
        use winit::platform::x11::WindowAttributesExtX11 as _;
        if let Some(window_type) = _window_type {
            use winit::platform::x11::WindowType;
            window_attributes = window_attributes.with_x11_window_type(vec![match window_type {
                egui::X11WindowType::Normal => WindowType::Normal,
                egui::X11WindowType::Utility => WindowType::Utility,
                egui::X11WindowType::Dock => WindowType::Dock,
                egui::X11WindowType::Desktop => WindowType::Desktop,
                egui::X11WindowType::Toolbar => WindowType::Toolbar,
                egui::X11WindowType::Menu => WindowType::Menu,
                egui::X11WindowType::Splash => WindowType::Splash,
                egui::X11WindowType::Dialog => WindowType::Dialog,
                egui::X11WindowType::DropdownMenu => WindowType::DropdownMenu,
                egui::X11WindowType::PopupMenu => WindowType::PopupMenu,
                egui::X11WindowType::Tooltip => WindowType::Tooltip,
                egui::X11WindowType::Notification => WindowType::Notification,
                egui::X11WindowType::Combo => WindowType::Combo,
                egui::X11WindowType::Dnd => WindowType::Dnd,
            }]);
        }
        if let Some(override_redirect) = _override_redirect {
            window_attributes = window_attributes.with_override_redirect(override_redirect);
        }
    }

    #[cfg(target_os = "windows")]
    {
        use winit::platform::windows::WindowAttributesExtWindows as _;
        if let Some(enable) = _drag_and_drop {
            window_attributes = window_attributes.with_drag_and_drop(enable);
        }
        if let Some(show) = _taskbar {
            window_attributes = window_attributes.with_skip_taskbar(!show);
        }

        // don't request the undecorated-window drop shadow in fullscreen (#8399)
        let want_undecorated_shadow = !decorations.unwrap_or(true) && !fullscreen.unwrap_or(false);
        window_attributes = window_attributes.with_undecorated_shadow(want_undecorated_shadow);
    }

    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::WindowAttributesExtMacOS as _;
        window_attributes = window_attributes
            .with_title_hidden(!_title_shown.unwrap_or(true))
            .with_titlebar_buttons_hidden(!_titlebar_buttons_shown.unwrap_or(true))
            .with_titlebar_transparent(!_titlebar_shown.unwrap_or(true))
            .with_fullsize_content_view(_fullsize_content_view.unwrap_or(false))
            .with_movable_by_window_background(_movable_by_window_background.unwrap_or(false))
            .with_has_shadow(_has_shadow.unwrap_or(true));
    }

    window_attributes
}

fn to_winit_icon(icon: &egui::IconData) -> Option<winit::window::Icon> {
    if icon.is_empty() {
        None
    } else {
        profiling::function_scope!();
        match winit::window::Icon::from_rgba(icon.rgba.clone(), icon.width, icon.height) {
            Ok(winit_icon) => Some(winit_icon),
            Err(err) => {
                log::warn!("Invalid IconData: {err}");
                None
            }
        }
    }
}

/// Applies what `create_winit_window_builder` couldn't
pub fn apply_viewport_builder_to_window(
    egui_ctx: &egui::Context,
    window: &Window,
    builder: &ViewportBuilder,
) {
    if let Some(mouse_passthrough) = builder.mouse_passthrough
        && let Err(err) = window.set_cursor_hittest(!mouse_passthrough)
    {
        log::warn!("set_cursor_hittest failed: {err}");
    }

    {
        // In `create_winit_window_builder` we didn't know
        // on what monitor the window would appear, so we didn't know
        // how to translate egui ui point to native physical pixels.
        // Now we do know:

        let pixels_per_point = pixels_per_point(egui_ctx, window);

        if let Some(size) = builder.inner_size
            && window
                .request_inner_size(PhysicalSize::new(
                    pixels_per_point * size.x,
                    pixels_per_point * size.y,
                ))
                .is_some()
        {
            log::debug!("Failed to set window size");
        }
        if let Some(size) = builder.min_inner_size {
            window.set_min_inner_size(Some(PhysicalSize::new(
                pixels_per_point * size.x,
                pixels_per_point * size.y,
            )));
        }
        if let Some(size) = builder.max_inner_size {
            window.set_max_inner_size(Some(PhysicalSize::new(
                pixels_per_point * size.x,
                pixels_per_point * size.y,
            )));
        }
        if let Some(pos) = builder.position {
            let pos = PhysicalPosition::new(pixels_per_point * pos.x, pixels_per_point * pos.y);
            window.set_outer_position(pos);
        }
        if let Some(maximized) = builder.maximized {
            set_maximized(window, maximized);
        }
    }
}

/// Like [`winit::window::Window::set_maximized`], but doesn't flash a hidden window on Windows.
pub fn set_maximized(window: &winit::window::Window, maximized: bool) {
    #[cfg(target_os = "windows")]
    if maximized && window.is_visible() == Some(false) {
        set_maximized_while_hidden(window);
        return;
    }

    window.set_maximized(maximized);
}

/// Maximize a hidden window without it flashing on screen.
///
/// winit implements maximize with `ShowWindow(SW_MAXIMIZE)`, which also shows the window,
/// and then hides it again with `SW_HIDE` if it is supposed to be invisible.
/// We cloak the window during that so the brief show is never composited.
/// The window stays hidden but maximized, so the first frame is rendered at the right size.
// TODO(rust-windowing/winit#4575): remove this (and the `with_maximized` special case in
// `create_winit_window_attributes`) once we are on a winit with that fix (or rust-windowing/winit#4587).
#[cfg(target_os = "windows")]
fn set_maximized_while_hidden(window: &winit::window::Window) {
    use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    use windows_sys::Win32::Graphics::Dwm::{DWMWA_CLOAK, DwmSetWindowAttribute};

    let Ok(RawWindowHandle::Win32(handle)) = window.window_handle().map(|h| h.as_raw()) else {
        window.set_maximized(true);
        return;
    };
    let hwnd = handle.hwnd.get() as _;

    let set_cloaked = |cloaked: bool| {
        let value = i32::from(cloaked);
        // SAFETY: `hwnd` is a valid window handle and `value` is a BOOL, as DWMWA_CLOAK expects.
        #[expect(unsafe_code)]
        unsafe {
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_CLOAK as u32,
                (&raw const value).cast(),
                size_of::<i32>() as u32,
            )
        }
    };

    let cloaked = set_cloaked(true) >= 0;
    // winit runs this synchronously, since we're on the event loop thread:
    window.set_maximized(true);
    if cloaked {
        set_cloaked(false);
    }
}

#[cfg(target_os = "windows")]
fn refresh_windows_non_client_activation(window: &winit::window::Window) {
    use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    use windows_sys::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_NCACTIVATE};

    let Ok(window_handle) = window.window_handle() else {
        return;
    };

    let RawWindowHandle::Win32(handle) = window_handle.as_raw() else {
        return;
    };

    let hwnd = handle.hwnd.get() as _;

    // SAFETY:
    // On Windows, changing the window theme updates egui immediately, but the
    // native title bar may keep its previous colors until the activation state
    // changes. Send WM_NCACTIVATE to refresh the non-client title bar state
    // without actually changing the real focus.
    #[expect(unsafe_code)]
    unsafe {
        #[expect(clippy::branches_sharing_code)]
        if window.has_focus() {
            // The window is active already, so send inactive -> active to force
            // Windows to recalculate/repaint the title bar state.
            SendMessageW(hwnd, WM_NCACTIVATE, 0, 0);
            SendMessageW(hwnd, WM_NCACTIVATE, 1, 0);
        } else {
            // Keep the visual state inactive if the window does not have focus.
            SendMessageW(hwnd, WM_NCACTIVATE, 0, 0);
        }
    }
}

// ---------------------------------------------------------------------------

/// Short and fast description of a device event.
/// Useful for logging and profiling.
pub fn short_device_event_description(event: &winit::event::DeviceEvent) -> &'static str {
    use winit::event::DeviceEvent;

    match event {
        DeviceEvent::Added => "DeviceEvent::Added",
        DeviceEvent::Removed => "DeviceEvent::Removed",
        DeviceEvent::MouseMotion { .. } => "DeviceEvent::MouseMotion",
        DeviceEvent::MouseWheel { .. } => "DeviceEvent::MouseWheel",
        DeviceEvent::Motion { .. } => "DeviceEvent::Motion",
        DeviceEvent::Button { .. } => "DeviceEvent::Button",
        DeviceEvent::Key { .. } => "DeviceEvent::Key",
    }
}

/// Short and fast description of a window event.
/// Useful for logging and profiling.
pub fn short_window_event_description(event: &winit::event::WindowEvent) -> &'static str {
    use winit::event::WindowEvent;

    match event {
        WindowEvent::ActivationTokenDone { .. } => "WindowEvent::ActivationTokenDone",
        WindowEvent::Resized { .. } => "WindowEvent::Resized",
        WindowEvent::Moved { .. } => "WindowEvent::Moved",
        WindowEvent::CloseRequested => "WindowEvent::CloseRequested",
        WindowEvent::Destroyed => "WindowEvent::Destroyed",
        WindowEvent::DroppedFile { .. } => "WindowEvent::DroppedFile",
        WindowEvent::HoveredFile { .. } => "WindowEvent::HoveredFile",
        WindowEvent::HoveredFileCancelled => "WindowEvent::HoveredFileCancelled",
        WindowEvent::Focused { .. } => "WindowEvent::Focused",
        WindowEvent::KeyboardInput { .. } => "WindowEvent::KeyboardInput",
        WindowEvent::ModifiersChanged { .. } => "WindowEvent::ModifiersChanged",
        WindowEvent::Ime { .. } => "WindowEvent::Ime",
        WindowEvent::ImeResetCompleted { .. } => "WindowEvent::ImeResetCompleted",
        WindowEvent::CursorMoved { .. } => "WindowEvent::CursorMoved",
        WindowEvent::CursorEntered { .. } => "WindowEvent::CursorEntered",
        WindowEvent::CursorLeft { .. } => "WindowEvent::CursorLeft",
        WindowEvent::MouseWheel { .. } => "WindowEvent::MouseWheel",
        WindowEvent::MouseInput { .. } => "WindowEvent::MouseInput",
        WindowEvent::PinchGesture { .. } => "WindowEvent::PinchGesture",
        WindowEvent::RedrawRequested => "WindowEvent::RedrawRequested",
        WindowEvent::DoubleTapGesture { .. } => "WindowEvent::DoubleTapGesture",
        WindowEvent::RotationGesture { .. } => "WindowEvent::RotationGesture",
        WindowEvent::TouchpadPressure { .. } => "WindowEvent::TouchpadPressure",
        WindowEvent::AxisMotion { .. } => "WindowEvent::AxisMotion",
        WindowEvent::Touch { .. } => "WindowEvent::Touch",
        WindowEvent::ScaleFactorChanged { .. } => "WindowEvent::ScaleFactorChanged",
        WindowEvent::ThemeChanged { .. } => "WindowEvent::ThemeChanged",
        WindowEvent::Occluded { .. } => "WindowEvent::Occluded",
        WindowEvent::PanGesture { .. } => "WindowEvent::PanGesture",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destroyed_window_resets_ime_cursor_purpose_and_allow_on_every_platform() {
        let mut allow_ime = true;
        let mut cursor_rect_px = Some(egui::Rect::from_min_size(
            egui::pos2(10.0, 20.0),
            egui::vec2(30.0, 40.0),
        ));
        let mut old_ime_purpose = egui::IMEPurpose::Password;

        reset_ime_window_latches(&mut allow_ime, &mut cursor_rect_px, &mut old_ime_purpose);

        assert!(!allow_ime, "a reused State must enable the new window");
        assert_eq!(
            cursor_rect_px, None,
            "the new window needs its first cursor area"
        );
        assert_eq!(
            old_ime_purpose,
            egui::IMEPurpose::Normal,
            "the new window starts with the default purpose"
        );
    }

    #[test]
    fn unchanged_primary_suppression_is_generation_scoped() {
        let claim = (7, "selected".to_owned());
        assert!(unchanged_primary_is_suppressed(Some(&claim), 7, "selected"));
        assert!(!unchanged_primary_is_suppressed(Some(&claim), 7, "other"));
        assert!(
            !unchanged_primary_is_suppressed(Some(&claim), 8, "selected"),
            "a new input generation may claim the same PRIMARY text again"
        );
        assert!(!unchanged_primary_is_suppressed(None, 7, "selected"));
    }

    #[test]
    fn clipboard_input_classification_ignores_release_and_repaint() {
        use crate::clipboard_bridge::InputKind;
        use winit::event::{
            DeviceId, ElementState, Ime, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent,
        };

        let mouse = |state| WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state,
            button: MouseButton::Middle,
        };
        assert_eq!(
            clipboard_input_kind(&mouse(ElementState::Pressed)),
            InputKind::PointerPress
        );
        assert_eq!(
            clipboard_input_kind(&mouse(ElementState::Released)),
            InputKind::NonInvalidating,
            "the pointer release after middle-click must not cancel its paste"
        );
        assert_eq!(
            clipboard_input_kind(&WindowEvent::RedrawRequested),
            InputKind::NonInvalidating
        );
        assert_eq!(
            clipboard_input_kind(&WindowEvent::CloseRequested),
            InputKind::Close
        );
        assert_eq!(
            clipboard_input_kind(&WindowEvent::Destroyed),
            InputKind::Close
        );
        assert_eq!(
            clipboard_input_kind(&WindowEvent::Focused(false)),
            InputKind::Focus
        );
        assert_eq!(
            clipboard_input_kind(&WindowEvent::Focused(true)),
            InputKind::FocusGained,
            "click-to-focus must not cancel the middle-click it completes"
        );
        assert_eq!(
            clipboard_input_kind(&WindowEvent::Occluded(true)),
            InputKind::Hide
        );
        assert_eq!(
            clipboard_input_kind(&WindowEvent::Occluded(false)),
            InputKind::NonInvalidating
        );
        assert_eq!(
            clipboard_input_kind(&WindowEvent::MouseWheel {
                device_id: DeviceId::dummy(),
                delta: MouseScrollDelta::LineDelta(0.0, 1.0),
                phase: TouchPhase::Moved,
            }),
            InputKind::Scroll
        );
        assert_eq!(
            clipboard_input_kind(&WindowEvent::Ime(Ime::Commit("x".into()))),
            InputKind::Edit
        );

        // The released initiating shortcut must not cancel the read it started. KeyEvent itself
        // has a winit-private platform field, so the testable seam is the pure classifier.
        assert_eq!(
            clipboard_key_input_kind(ElementState::Released, false),
            InputKind::NonInvalidating
        );
        assert_eq!(
            clipboard_key_input_kind(ElementState::Pressed, false),
            InputKind::KeyPress
        );
        assert_eq!(
            clipboard_key_input_kind(ElementState::Pressed, true),
            InputKind::NonInvalidating,
            "synthetic key presses are not ownership or invalidation triggers"
        );

        assert!(clipboard_input_claims_origin(&mouse(ElementState::Pressed)));
        assert!(clipboard_input_claims_origin(&mouse(
            ElementState::Released
        )));
        assert!(!clipboard_input_claims_origin(
            &WindowEvent::RedrawRequested
        ));
    }

    #[test]
    fn claim_batch_invalidation_follows_focus_loss_hide_and_close() {
        use crate::clipboard_bridge::InputKind;

        for kind in [InputKind::Focus, InputKind::Close, InputKind::Hide] {
            assert!(clipboard_input_invalidates_claim_batch(kind));
        }
        for kind in [
            InputKind::FocusGained,
            InputKind::PointerPress,
            InputKind::KeyPress,
            InputKind::Edit,
            InputKind::Scroll,
            InputKind::NonInvalidating,
        ] {
            assert!(
                !clipboard_input_invalidates_claim_batch(kind),
                "must not invalidate the batch for {kind:?}"
            );
        }
    }

    #[test]
    fn letter_keys() {
        use egui::Key;
        const VK_RETURN: u32 = 0x0D;
        const VK_OEM_2: u32 = 0xBF;
        const VK_OEM_3: u32 = 0xC0;
        let vk = |c: char| Some(c as u32);
        let key = |text: &str| key_from_winit_key(&winit::keyboard::Key::Character(text.into()));

        // Windows: (layout, typed, virtual-key code, expected key)
        let windows = [
            ("US", "c", vk('C'), Some(Key::C)),
            ("Dvorak", "c", vk('C'), Some(Key::C)),
            ("AZERTY", "a", vk('A'), Some(Key::A)),
            ("German", "ö", Some(VK_OEM_3), None),
            ("Russian", "с", vk('C'), Some(Key::C)),
            ("Russian", "Ф", vk('A'), Some(Key::A)),
            ("Russian", ".", Some(VK_OEM_2), Some(Key::Period)),
            ("Russian", "1", vk('1'), Some(Key::Num1)),
            ("Greek", "ψ", vk('C'), Some(Key::C)),
            ("Hebrew", "ב", vk('C'), Some(Key::C)),
            ("Arabic", "ؤ", vk('C'), Some(Key::C)),
            ("Arabic AZERTY", "ض", vk('A'), Some(Key::A)),
            ("Thai", "แ", vk('C'), Some(Key::C)),
            ("Thai", "ฟ", None, None),
        ];
        for (layout, typed, virtual_key, expected) in windows {
            let got = letter_key(key(typed), windows_virtual_letter(virtual_key), false);
            assert_eq!(got, expected, "Windows {layout} {typed:?}");
        }

        // Linux: (layout, typed, `latin_key()`, `is_latin_layout()`, expected key)
        let linux = [
            ("US", "c", None, true, Some(Key::C)),
            ("Dvorak", "j", None, true, Some(Key::J)),
            ("AZERTY", "a", None, true, Some(Key::A)),
            ("Russian", "с", Some("c"), false, Some(Key::C)),
            ("Russian", "Ф", Some("a"), false, Some(Key::A)),
            ("Russian", ".", None, false, Some(Key::Period)),
            ("Russian, no Latin layout", "с", None, false, None),
            ("Greek", "ψ", Some("c"), false, Some(Key::C)),
            ("Hebrew", "ב", Some("c"), false, Some(Key::C)),
            ("Arabic", "ؤ", Some("c"), false, Some(Key::C)),
            ("Thai", "แ", Some("c"), false, Some(Key::C)),
            ("Russian with Dvorak", "с", Some("j"), false, Some(Key::J)),
            ("Dvorak, Russian", "й", Some("'"), false, Some(Key::Quote)),
            ("Dvorak, Russian", "ч", Some("q"), false, Some(Key::Q)),
            ("Dvorak, Russian", "1", Some("1"), false, Some(Key::Num1)),
            ("Russian", ".", Some("/"), false, Some(Key::Period)),
        ];
        for (layout, typed, latin, is_latin_layout, expected) in linux {
            let got = letter_key(key(typed), latin.and_then(key), !is_latin_layout);
            assert_eq!(got, expected, "Linux {layout} {typed:?}");
        }

        // macOS: (layout, typed, `key_with_command()`, ⌘ held, expected key)
        let macos = [
            ("US", "c", "c", true, Some(Key::C)),
            ("Dvorak", "j", "j", true, Some(Key::J)),
            ("AZERTY", "a", "a", true, Some(Key::A)),
            ("Dvorak – QWERTY ⌘", "j", "c", true, Some(Key::C)),
            ("Dvorak – QWERTY ⌘", "j", "c", false, Some(Key::J)),
            ("Russian", "с", "c", true, Some(Key::C)),
            ("Russian", "с", "c", false, None),
            ("Russian", "/", "/", true, Some(Key::Slash)),
            ("Greek", "ψ", "c", true, Some(Key::C)),
            ("Hebrew", "ב", "c", true, Some(Key::C)),
            ("Arabic", "ؤ", "c", true, Some(Key::C)),
            ("Thai", "แ", "c", true, Some(Key::C)),
        ];
        for (layout, typed, with_command, command, expected) in macos {
            // Mirrors the macOS `latin_key`.
            let (latin, prefer_latin) = if command {
                (key(with_command), true)
            } else {
                (None, false)
            };
            let got = letter_key(key(typed), latin, prefer_latin);
            assert_eq!(got, expected, "macOS {layout} {typed:?} ⌘ {command}");
        }

        // What shortcuts on Linux and macOS record per physical key.
        let winit_key = |text: &str| winit::keyboard::Key::Character(text.into());
        assert_eq!(key_char(&winit_key("'")), Some('\''));
        assert_eq!(key_char(&winit_key("j")), Some('j'));
        assert_eq!(key_char(&winit_key("ae")), None);

        let enter = key_from_winit_key(&winit::keyboard::Key::Named(
            winit::keyboard::NamedKey::Enter,
        ));
        let enter_vk = windows_virtual_letter(Some(VK_RETURN));
        assert_eq!(letter_key(enter, enter_vk, false), Some(Key::Enter));
        assert_eq!(letter_key(enter, None, true), Some(Key::Enter));
    }
}

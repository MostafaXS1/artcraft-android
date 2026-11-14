use raw_window_handle::RawDisplayHandle;

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ),
    feature = "wayland",
    feature = "clipboard",
    feature = "clipboard-image",
))]
use crate::clipboard_bridge::EncodedPng;
#[cfg(all(
    any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ),
    feature = "wayland",
    feature = "clipboard",
))]
use crate::clipboard_bridge::{
    ClipboardTransport, CompletionWaker, PasteContent, PasteError, PasteErrorKind, PasteKind,
    PasteTransfer,
};

/// Handles interfacing with the OS clipboard.
///
/// If the "clipboard" feature is off, or we cannot connect to the OS clipboard,
/// then a fallback clipboard that just works within the same app is used instead.
///
/// On Wayland, regular text and PRIMARY ownership go through winit's clipboard (W15/W15P).
/// The synchronous `get`/`get_primary_text` methods remain as bounded compatibility APIs for
/// non-event-path callers; [`crate::State`] uses the clipboard bridge's asynchronous
/// `begin_load_*` handles instead so a slow peer cannot block the event loop.
pub struct Clipboard {
    #[cfg(all(
        not(any(target_os = "android", target_os = "ios")),
        feature = "arboard",
    ))]
    arboard: Option<arboard::Clipboard>,

    #[cfg(all(
        any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ),
        feature = "smithay-clipboard"
    ))]
    smithay: Option<smithay_clipboard::Clipboard>,

    /// The clipboard of winit's Wayland event loop, which uses winit's own data device.
    /// When it is set, smithay-clipboard is not started.
    #[cfg(all(
        any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ),
        feature = "wayland",
        feature = "clipboard",
    ))]
    winit_wayland: Option<winit::platform::wayland::Clipboard>,

    /// Whether the display handle was Wayland even if winit's clipboard was unavailable.
    #[cfg(all(
        any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ),
        feature = "wayland",
        feature = "clipboard",
    ))]
    wayland_display: bool,

    /// Fallback manual clipboard.
    clipboard: String,
}

impl Clipboard {
    /// Construct a new instance.
    ///
    /// # Safety
    ///
    /// If `raw_display_handle` is `Some`, the display handle must remain valid for the
    /// entire lifetime of the returned `Clipboard` instance.
    #[expect(unsafe_code)]
    pub unsafe fn new(_raw_display_handle: Option<RawDisplayHandle>) -> Self {
        #[cfg(all(
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
            feature = "wayland",
            feature = "clipboard",
        ))]
        let wayland_display = matches!(&_raw_display_handle, Some(RawDisplayHandle::Wayland(_)));

        #[cfg(all(
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
            feature = "wayland",
            feature = "clipboard",
        ))]
        if wayland_display {
            // The winit event-loop clipboard is the only regular/PRIMARY owner on Wayland.
            // Even when winit did not expose a transport, starting arboard or smithay would
            // create a second ownership model and can steal the selection.
            let winit_wayland = init_winit_wayland_clipboard(_raw_display_handle);
            return Self {
                #[cfg(all(
                    not(any(target_os = "android", target_os = "ios")),
                    feature = "arboard",
                ))]
                arboard: None,
                #[cfg(feature = "smithay-clipboard")]
                smithay: None,
                winit_wayland,
                wayland_display: true,
                clipboard: Default::default(),
            };
        }

        Self {
            #[cfg(all(
                not(any(target_os = "android", target_os = "ios")),
                feature = "arboard",
            ))]
            arboard: init_arboard(),

            #[cfg(all(
                any(
                    target_os = "linux",
                    target_os = "dragonfly",
                    target_os = "freebsd",
                    target_os = "netbsd",
                    target_os = "openbsd"
                ),
                feature = "smithay-clipboard"
            ))]
            // SAFETY: The caller guarantees that the display handle remains valid.
            smithay: unsafe { init_smithay_clipboard(_raw_display_handle) },

            #[cfg(all(
                any(
                    target_os = "linux",
                    target_os = "dragonfly",
                    target_os = "freebsd",
                    target_os = "netbsd",
                    target_os = "openbsd"
                ),
                feature = "wayland",
                feature = "clipboard",
            ))]
            winit_wayland: None,
            #[cfg(all(
                any(
                    target_os = "linux",
                    target_os = "dragonfly",
                    target_os = "freebsd",
                    target_os = "netbsd",
                    target_os = "openbsd"
                ),
                feature = "wayland",
                feature = "clipboard",
            ))]
            wayland_display,

            clipboard: Default::default(),
        }
    }

    /// A clone of the winit Wayland transport, if winit initialized one.
    ///
    /// The event path uses this to start non-blocking transfers and to make synchronous
    /// ownership claims on the triggering input turn. Other platforms keep their existing
    /// synchronous clipboard path.
    #[cfg(all(
        any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ),
        feature = "wayland",
        feature = "clipboard",
    ))]
    pub(crate) fn wayland_transport(&self) -> Option<WaylandPasteTransport> {
        self.winit_wayland
            .clone()
            .map(|clipboard| WaylandPasteTransport { clipboard })
    }

    /// Whether the window's display handle is Wayland.
    ///
    /// If winit did not initialize its clipboard for a Wayland display, the event path must fail
    /// closed instead of calling a blocking arboard/data-control read as a substitute.
    #[cfg(all(
        any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ),
        feature = "wayland",
        feature = "clipboard",
    ))]
    pub(crate) fn is_wayland(&self) -> bool {
        self.wayland_display
    }

    /// Read the regular clipboard text.
    ///
    /// This is the bounded synchronous compatibility API. On Wayland it blocks the calling
    /// thread up to winit's transfer deadline, so [`crate::State`] uses the asynchronous
    /// clipboard bridge instead when handling input. A present empty text is `Some("")`; an
    /// absent or non-text regular selection may be represented by `None`, which lets the caller
    /// consider an image paste.
    #[cfg_attr(
        not(feature = "clipboard"),
        expect(clippy::needless_pass_by_ref_mut, clippy::unnecessary_wraps)
    )]
    pub fn get(&mut self) -> Option<String> {
        #[cfg(all(
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
            feature = "wayland",
            feature = "clipboard",
        ))]
        if self.is_wayland() {
            let Some(clipboard) = &self.winit_wayland else {
                // Missing winit transport must fail visibly, not start another ownership model.
                return None;
            };
            return match clipboard.load_text() {
                Ok(text) => Some(text),
                // A present empty text is `Some("")`. Absent and non-text regular offers are
                // `None`; malformed/interrupted transfers never fall back to arboard/smithay.
                Err(err) => {
                    log::debug!("winit Wayland paste error: {err}");
                    None
                }
            };
        }

        #[cfg(all(
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
            feature = "smithay-clipboard"
        ))]
        if let Some(clipboard) = &mut self.smithay {
            match clipboard.load() {
                Ok(text) => return Some(text),
                // Smithay uses NotFound when no supported text MIME type is offered.
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => {
                    // Not fatal: we fall back to arboard below.
                    log::debug!("smithay paste error: {err}");
                }
            }
        }

        #[cfg(all(
            not(any(target_os = "android", target_os = "ios")),
            feature = "arboard",
        ))]
        if let Some(clipboard) = &mut self.arboard {
            return match clipboard.get_text() {
                Ok(text) => Some(text),
                Err(err) => {
                    // Expected whenever the clipboard holds something other than text (e.g.
                    // an image copied with a screenshot tool) — the caller falls back to
                    // `Self::get_image` in that case, so this is not an error worth
                    // alarming the user/log about.
                    if !is_expected_content_absence(&err) {
                        log::error!("arboard paste error: {err}");
                    }
                    None
                }
            };
        }

        Some(self.clipboard.clone())
    }

    /// Put text on the regular clipboard through the compatibility API.
    ///
    /// This does not have an input-origin token. The egui event path claims ownership with
    /// winit's `claim_text` on the triggering input serial instead.
    pub fn set_text(&mut self, text: String) {
        #[cfg(all(
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
            feature = "wayland",
            feature = "clipboard",
        ))]
        if self.is_wayland() {
            let Some(clipboard) = &self.winit_wayland else {
                log::error!("winit Wayland clipboard is unavailable; text was not stored");
                return;
            };
            if let Err(err) = clipboard.store_text(text) {
                log::error!("Wayland copy/cut error: {err}");
            }
            return;
        }

        #[cfg(all(
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
            feature = "smithay-clipboard"
        ))]
        if let Some(clipboard) = &mut self.smithay {
            clipboard.store(text);
            return;
        }

        #[cfg(all(
            not(any(target_os = "android", target_os = "ios")),
            feature = "arboard",
        ))]
        if let Some(clipboard) = &mut self.arboard {
            if let Err(err) = clipboard.set_text(text) {
                log::error!("arboard copy/cut error: {err}");
            }
            return;
        }

        self.clipboard = text;
    }

    /// Get an image from the clipboard, if there is one and the platform backend supports it.
    ///
    /// This mirrors [`Self::set_image`] for the opposite direction, so that a Ctrl+V/Cmd+V
    /// paste can carry an image (e.g. a screenshot or a copied image) instead of text — see
    /// [`egui::Event::PasteImage`].
    ///
    /// This is an `arboard`-only synchronous compatibility path for non-Wayland displays.
    /// On Wayland authoritative ownership belongs to winit, so this method explicitly refuses
    /// instead of opening another transport or blocking the event loop. The supported image
    /// path is the `State` regular paste event, which uses a bounded combined read and the
    /// off-thread PNG codec. Non-Wayland synchronous paste keeps its existing fallback.
    #[cfg_attr(
        not(feature = "clipboard"),
        expect(clippy::needless_pass_by_ref_mut, clippy::unused_self)
    )]
    pub fn get_image(&mut self) -> Option<egui::ColorImage> {
        #[cfg(all(
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
            feature = "wayland",
            feature = "clipboard",
        ))]
        if self.is_wayland() {
            log::debug!(
                "refusing the synchronous Wayland image read; use the State paste event path"
            );
            return None;
        }

        #[cfg(all(
            not(any(target_os = "android", target_os = "ios")),
            feature = "arboard",
        ))]
        if let Some(clipboard) = &mut self.arboard {
            return match clipboard.get_image() {
                Ok(image) => Some(color_image_from_arboard(&image)),
                Err(err) => {
                    // Expected whenever the clipboard holds neither text nor an image (e.g.
                    // it's simply empty) — `Self::get` was already tried first and came up
                    // empty too, so this is the mundane "nothing to paste" case, not an error.
                    if !is_expected_content_absence(&err) {
                        log::error!("arboard paste-image error: {err}");
                    }
                    None
                }
            };
        }

        None
    }

    #[cfg_attr(
        not(feature = "clipboard"),
        expect(clippy::needless_pass_by_ref_mut, clippy::unused_self)
    )]
    pub fn set_image(&mut self, image: &egui::ColorImage) {
        #[cfg(all(
            any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ),
            feature = "wayland",
            feature = "clipboard",
        ))]
        if self.is_wayland() {
            log::error!(
                "refusing the synchronous Wayland image write; image copies use the State event path"
            );
            _ = image;
            return;
        }

        #[cfg(all(
            not(any(target_os = "android", target_os = "ios")),
            feature = "arboard",
        ))]
        if let Some(clipboard) = &mut self.arboard {
            if let Err(err) = clipboard.set_image(arboard_from_color_image(image)) {
                log::error!("arboard copy/cut error: {err}");
            }
            log::debug!("Copied image to clipboard");
            return;
        }

        log::error!(
            "Copying images is not supported. Enable the 'clipboard' feature of `egui-winit` to enable it."
        );
        _ = image;
    }
}

// The X11/Wayland PRIMARY selection: filled in by selecting text, pasted with the middle mouse button.
cfg_select! {
    any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ) => {
        #[cfg_attr(
            not(any(feature = "arboard", feature = "smithay-clipboard")),
            expect(
                clippy::unused_self,
                clippy::needless_pass_by_ref_mut,
                clippy::needless_pass_by_value,
                reason = "no backend to use"
            )
        )]
        impl Clipboard {
            /// Read the PRIMARY selection, if there is one.
            ///
            /// This is the bounded synchronous compatibility API. [`crate::State`] uses the
            /// asynchronous bridge for middle-click paste on Wayland.
            pub fn get_primary_text(&mut self) -> Option<String> {
                #[cfg(all(feature = "wayland", feature = "clipboard"))]
                if self.is_wayland() {
                    let Some(clipboard) = &self.winit_wayland else {
                        return None;
                    };
                    return match clipboard.load_primary_text() {
                        Ok(text) => Some(text),
                        Err(err) => {
                            // Do not fall through to arboard/smithay: winit owns the Wayland
                            // PRIMARY protocol and an interrupted or unsupported transfer is
                            // not an absent offer.
                            log::debug!("winit Wayland primary paste error: {err}");
                            None
                        }
                    };
                }

                #[cfg(feature = "smithay-clipboard")]
                if let Some(clipboard) = &mut self.smithay {
                    return clipboard
                        .load_primary()
                        .inspect_err(|err| log::debug!("smithay primary paste error: {err}"))
                        .ok();
                }

                #[cfg(feature = "arboard")]
                if let Some(clipboard) = &mut self.arboard {
                    use arboard::GetExtLinux as _;
                    return clipboard
                        .get()
                        .clipboard(arboard::LinuxClipboardKind::Primary)
                        .text()
                        .inspect_err(|err| log::debug!("arboard primary paste error: {err}"))
                        .ok();
                }

                None
            }

            /// Set the PRIMARY selection.
            ///
            /// This does not have an input-origin token. The egui event path claims PRIMARY with
            /// winit's `claim_primary_text` on the triggering pointer or keyboard serial instead.
            pub fn set_primary_text(&mut self, text: String) {
                #[cfg(all(feature = "wayland", feature = "clipboard"))]
                if self.is_wayland() {
                    let Some(clipboard) = &self.winit_wayland else {
                        log::error!(
                            "winit Wayland clipboard is unavailable; PRIMARY text was not stored"
                        );
                        return;
                    };
                    if let Err(err) = clipboard.store_primary_text(text) {
                        log::error!("Wayland PRIMARY copy error: {err}");
                    }
                    return;
                }

                #[cfg(feature = "smithay-clipboard")]
                if let Some(clipboard) = &mut self.smithay {
                    clipboard.store_primary(text);
                    return;
                }

                #[cfg(feature = "arboard")]
                if let Some(clipboard) = &mut self.arboard {
                    use arboard::SetExtLinux as _;
                    if let Err(err) = clipboard
                        .set()
                        .clipboard(arboard::LinuxClipboardKind::Primary)
                        .text(text)
                    {
                        log::error!("arboard primary copy error: {err}");
                    }
                    return;
                }

                _ = text;
            }
        }
    }
    _ => {
        #[expect(
            clippy::unused_self,
            clippy::needless_pass_by_ref_mut,
            reason = "there is no PRIMARY selection on this platform"
        )]
        impl Clipboard {
            /// Read the PRIMARY selection. Always `None` on this platform.
            pub fn get_primary_text(&mut self) -> Option<String> {
                None
            }

            /// Set the PRIMARY selection. Does nothing on this platform.
            pub fn set_primary_text(&mut self, _text: String) {}
        }
    }
}

/// Whether an `arboard::Error` from reading the clipboard is the expected, mundane outcome
/// of the clipboard simply not holding the requested content type (e.g. text was asked for
/// but the clipboard holds an image, or vice versa, or it's just empty) — as opposed to a
/// genuine failure (permissions, a locked clipboard, a conversion error) worth an `error!` log.
///
/// Pulled out as its own pure function (rather than inlined in the two `match`es above) so it
/// can be unit-tested without touching the real OS clipboard, which CI can't rely on.
#[cfg(all(
    not(any(target_os = "android", target_os = "ios")),
    feature = "arboard",
))]
fn is_expected_content_absence(err: &arboard::Error) -> bool {
    matches!(err, arboard::Error::ContentNotAvailable)
}

#[cfg(all(
    not(any(target_os = "android", target_os = "ios")),
    feature = "arboard",
))]
fn color_image_from_arboard(image: &arboard::ImageData<'_>) -> egui::ColorImage {
    egui::ColorImage::from_rgba_unmultiplied([image.width, image.height], &image.bytes)
}

/// arboard expects unmultiplied alpha, but [`egui::Color32`] is premultiplied.
#[cfg(all(
    not(any(target_os = "android", target_os = "ios")),
    feature = "arboard",
))]
fn arboard_from_color_image(image: &egui::ColorImage) -> arboard::ImageData<'static> {
    let bytes: Vec<u8> = image
        .pixels
        .iter()
        .flat_map(|color| color.to_srgba_unmultiplied())
        .collect();
    arboard::ImageData {
        width: image.width(),
        height: image.height(),
        bytes: bytes.into(),
    }
}

/// The production Wayland transport used by the asynchronous clipboard bridge.
///
/// It holds a clone of winit's event-loop clipboard, so the bridge and the compatibility
/// `Clipboard` methods share the same per-seat data device and PRIMARY device. It is only
/// constructed when winit initialized a Wayland clipboard.
#[cfg(all(
    any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ),
    feature = "wayland",
    feature = "clipboard",
))]
#[derive(Clone)]
pub(crate) struct WaylandPasteTransport {
    clipboard: winit::platform::wayland::Clipboard,
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ),
    feature = "wayland",
    feature = "clipboard",
))]
impl ClipboardTransport for WaylandPasteTransport {
    type ClaimToken = winit::platform::wayland::InputOriginToken;

    fn begin_load(
        &self,
        kind: PasteKind,
        target_window: winit::window::WindowId,
        caller_generation: u64,
        waker: CompletionWaker,
    ) -> Result<Box<dyn PasteTransfer>, PasteError> {
        // Winit owns the total transfer deadline (2 s by default), cumulative 16 MiB byte cap
        // and bounded worker admission. The completion waker is called at most once.
        let options =
            winit::platform::wayland::TransferOptions::new(target_window, caller_generation, waker);
        match kind {
            PasteKind::Regular => self
                .clipboard
                .begin_load_regular(options)
                .map(|handle| {
                    Box::new(WaylandPasteTransfer::Regular(handle)) as Box<dyn PasteTransfer>
                })
                .map_err(|error| paste_error_from_clipboard(&error)),
            PasteKind::Primary => self
                .clipboard
                .begin_load_primary_text(options)
                .map(|handle| {
                    Box::new(WaylandPasteTransfer::Primary(handle)) as Box<dyn PasteTransfer>
                })
                .map_err(|error| PasteError::from_io(&error)),
        }
    }

    fn input_origin(
        &self,
        target_window: winit::window::WindowId,
    ) -> Result<Self::ClaimToken, PasteError> {
        self.clipboard
            .input_origin(target_window)
            .map_err(|error| PasteError::from_io(&error))
    }

    fn claim(
        &self,
        kind: PasteKind,
        text: std::sync::Arc<str>,
        token: Self::ClaimToken,
        target_window: winit::window::WindowId,
        caller_generation: u64,
    ) -> Result<(), PasteError> {
        let claim = winit::platform::wayland::SelectionClaim {
            target_window,
            caller_generation,
            trigger: token,
        };
        match kind {
            PasteKind::Regular => self
                .clipboard
                .claim_text(text, claim)
                .map_err(|error| PasteError::from_io(&error)),
            PasteKind::Primary => self
                .clipboard
                .claim_primary_text(text, claim)
                .map_err(|error| PasteError::from_io(&error)),
        }
    }

    fn invalidate_image_reservations(
        &self,
        target_window: winit::window::WindowId,
    ) -> Result<(), PasteError> {
        self.clipboard
            .invalidate_image_reservations(target_window)
            .map_err(|error| paste_error_from_clipboard(&error))
    }
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ),
    feature = "wayland",
    feature = "clipboard",
    feature = "clipboard-image",
))]
impl WaylandPasteTransport {
    /// Reserve the exact captured regular-family token for an asynchronous PNG encode.
    pub(crate) fn reserve_image_claim(
        &self,
        claim: winit::platform::wayland::SelectionClaim,
    ) -> Result<winit::platform::wayland::ImageClaimReservation, PasteError> {
        self.clipboard
            .reserve_image_claim(claim)
            .map_err(|error| paste_error_from_clipboard(&error))
    }

    /// Install a reserved PNG using the original triggering serial.
    pub(crate) fn claim_reserved_image_png(
        &self,
        png: winit::platform::wayland::PngBytes,
        reservation: winit::platform::wayland::ImageClaimReservation,
    ) -> Result<(), PasteError> {
        self.clipboard
            .claim_reserved_image_png(png, reservation)
            .map_err(|error| paste_error_from_clipboard(&error))
    }
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ),
    feature = "wayland",
    feature = "clipboard",
))]
fn paste_error_from_clipboard(error: &winit::platform::wayland::ClipboardError) -> PasteError {
    use winit::platform::wayland::ClipboardError;
    let kind = match error {
        ClipboardError::Absent => PasteErrorKind::Absent,
        ClipboardError::NotOffered => PasteErrorKind::NotOffered,
        ClipboardError::StaleSelfSource => PasteErrorKind::StaleSelfSource,
        ClipboardError::Interrupted => PasteErrorKind::Interrupted,
        ClipboardError::TimedOut => PasteErrorKind::TimedOut,
        ClipboardError::Unsupported => PasteErrorKind::Unsupported,
        ClipboardError::Busy => PasteErrorKind::Busy,
        ClipboardError::TooLarge => PasteErrorKind::TooLarge,
        ClipboardError::InvalidInput => PasteErrorKind::InvalidInput,
        ClipboardError::Disconnected => PasteErrorKind::Disconnected,
        ClipboardError::Backend(_) => PasteErrorKind::Backend,
    };
    PasteError::new(kind, error.to_string())
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ),
    feature = "wayland",
    feature = "clipboard",
))]
enum WaylandPasteTransfer {
    Regular(winit::platform::wayland::RegularTransferHandle),
    Primary(winit::platform::wayland::TransferHandle),
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ),
    feature = "wayland",
    feature = "clipboard",
))]
#[cfg(feature = "clipboard-image")]
struct WinitEncodedPng(winit::platform::wayland::PngBytes);

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ),
    feature = "wayland",
    feature = "clipboard",
    feature = "clipboard-image"
))]
impl EncodedPng for WinitEncodedPng {
    fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    fn len(&self) -> usize {
        self.0.len()
    }
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ),
    feature = "wayland",
    feature = "clipboard",
    feature = "clipboard-image"
))]
fn paste_content_from_winit(content: winit::platform::wayland::ClipboardContent) -> PasteContent {
    match content {
        winit::platform::wayland::ClipboardContent::Text(text) => PasteContent::Text(text),
        winit::platform::wayland::ClipboardContent::ImagePng(png) => {
            PasteContent::ImagePng(std::sync::Arc::new(WinitEncodedPng(png)))
        }
    }
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ),
    feature = "wayland",
    feature = "clipboard",
    not(feature = "clipboard-image")
))]
fn paste_content_from_winit(
    content: winit::platform::wayland::ClipboardContent,
) -> Result<PasteContent, PasteError> {
    match content {
        winit::platform::wayland::ClipboardContent::Text(text) => Ok(PasteContent::Text(text)),
        winit::platform::wayland::ClipboardContent::ImagePng(_) => Err(PasteError::new(
            PasteErrorKind::NotOffered,
            "PNG paste requires the 'clipboard-image' feature",
        )),
    }
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ),
    feature = "wayland",
    feature = "clipboard",
    feature = "clipboard-image"
))]
fn finish_paste_content(
    content: Result<winit::platform::wayland::ClipboardContent, PasteError>,
) -> Result<PasteContent, PasteError> {
    content.map(paste_content_from_winit)
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ),
    feature = "wayland",
    feature = "clipboard",
    not(feature = "clipboard-image")
))]
fn finish_paste_content(
    content: Result<winit::platform::wayland::ClipboardContent, PasteError>,
) -> Result<PasteContent, PasteError> {
    content.and_then(paste_content_from_winit)
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ),
    feature = "wayland",
    feature = "clipboard",
))]
impl PasteTransfer for WaylandPasteTransfer {
    fn try_take(&self) -> Option<Result<PasteContent, PasteError>> {
        match self {
            Self::Regular(handle) => handle.try_take().map(|result| {
                finish_paste_content(result.map_err(|error| paste_error_from_clipboard(&error)))
            }),
            Self::Primary(handle) => handle.try_take().map(|result| {
                result
                    .map(PasteContent::Text)
                    .map_err(|error| PasteError::from_io(&error))
            }),
        }
    }

    fn cancel(&self) {
        match self {
            Self::Regular(handle) => handle.cancel(),
            Self::Primary(handle) => handle.cancel(),
        }
    }
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ),
    feature = "wayland",
    feature = "clipboard",
))]
fn init_winit_wayland_clipboard(
    raw_display_handle: Option<RawDisplayHandle>,
) -> Option<winit::platform::wayland::Clipboard> {
    let Some(RawDisplayHandle::Wayland(display)) = raw_display_handle else {
        return None;
    };
    let clipboard = winit::platform::wayland::Clipboard::for_display(display.display);
    if clipboard.is_some() {
        log::trace!("Using the clipboard of winit's Wayland event loop");
    }
    clipboard
}

#[cfg(all(
    not(any(target_os = "android", target_os = "ios")),
    feature = "arboard",
))]
fn init_arboard() -> Option<arboard::Clipboard> {
    profiling::function_scope!();

    log::trace!("Initializing arboard clipboard…");
    match arboard::Clipboard::new() {
        Ok(clipboard) => Some(clipboard),
        Err(err) => {
            log::warn!("Failed to initialize arboard clipboard: {err}");
            None
        }
    }
}

/// # Safety
///
/// The display handle in `raw_display_handle` must remain valid for the
/// lifetime of the returned `Clipboard`.
#[cfg(all(
    any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ),
    feature = "smithay-clipboard"
))]
#[expect(unsafe_code)]
unsafe fn init_smithay_clipboard(
    raw_display_handle: Option<RawDisplayHandle>,
) -> Option<smithay_clipboard::Clipboard> {
    profiling::function_scope!();

    if let Some(RawDisplayHandle::Wayland(display)) = raw_display_handle {
        log::trace!("Initializing smithay clipboard…");
        // SAFETY: The caller guarantees that the display handle remains valid.
        Some(unsafe { smithay_clipboard::Clipboard::new(display.display.as_ptr()) })
    } else {
        #[cfg(feature = "wayland")]
        log::debug!("Cannot init smithay clipboard without a Wayland display handle");
        #[cfg(not(feature = "wayland"))]
        log::debug!(
            "Cannot init smithay clipboard: the 'wayland' feature of 'egui-winit' is not enabled"
        );
        None
    }
}

#[cfg(all(
    not(any(target_os = "android", target_os = "ios")),
    feature = "arboard",
))]
#[cfg(test)]
mod tests {
    use super::{arboard_from_color_image, color_image_from_arboard, is_expected_content_absence};

    /// Regression test for the spurious `error!`-level log a maintainer caught by manually
    /// testing an image paste (nothing had exercised this distinction before): only
    /// `ContentNotAvailable` — clipboard simply doesn't hold the requested content type — is
    /// expected and should stay silent; every other `arboard::Error` variant is a real failure
    /// and must still be logged.
    #[test]
    fn only_content_not_available_is_treated_as_expected() {
        assert!(is_expected_content_absence(
            &arboard::Error::ContentNotAvailable
        ));

        assert!(!is_expected_content_absence(
            &arboard::Error::ClipboardNotSupported
        ));
        assert!(!is_expected_content_absence(
            &arboard::Error::ClipboardOccupied
        ));
        assert!(!is_expected_content_absence(
            &arboard::Error::ConversionFailure
        ));
        assert!(!is_expected_content_absence(&arboard::Error::Unknown {
            description: "anything".to_owned(),
        }));
    }

    #[test]
    fn color_image_from_arboard_converts_straight_to_premultiplied_alpha() {
        // 2x1 image: opaque red, then half-transparent white — straight (unmultiplied) alpha,
        // as arboard/the OS clipboard would hand it to us.
        let image = arboard::ImageData {
            width: 2,
            height: 1,
            bytes: std::borrow::Cow::Borrowed(&[255, 0, 0, 255, 255, 255, 255, 128]),
        };
        let color_image = color_image_from_arboard(&image);
        assert_eq!(color_image.size, [2, 1]);
        assert_eq!(
            color_image.pixels[0],
            egui::Color32::from_rgba_unmultiplied(255, 0, 0, 255)
        );
        assert_eq!(
            color_image.pixels[1],
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 128)
        );
    }

    #[test]
    fn arboard_from_color_image_converts_premultiplied_to_straight_alpha() {
        let color_image = egui::ColorImage::new(
            [3, 1],
            vec![
                egui::Color32::from_rgba_unmultiplied(255, 0, 0, 255),
                egui::Color32::from_rgba_unmultiplied(255, 255, 255, 128),
                egui::Color32::TRANSPARENT,
            ],
        );
        let image = arboard_from_color_image(&color_image);
        assert_eq!([image.width, image.height], [3, 1]);
        assert_eq!(
            &*image.bytes,
            &[255, 0, 0, 255, 255, 255, 255, 128, 0, 0, 0, 0]
        );
    }
}

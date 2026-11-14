//! Bounded PNG codec and composed delivery state for the Wayland clipboard bridge.
//!
//! The codec is process-global: one encode worker, one decode worker and one queued job per
//! direction in a cancellable bounded mailbox. Every queued or running job holds a conservative
//! permit from a shared 192 MiB admission budget. Cancelling queued work removes its envelope
//! and releases the payload/permit promptly; running work may finish while remaining budgeted.
//! Admission is not an RSS ceiling: allocator overhead and decoder-internal accounting that
//! cannot be made strict can add memory. Native peak-RSS qualification is not part of this module
//! or its tests.
//!
//! Decoded [`egui::ColorImage`] allocations are retained by [`DecodedPasteImage`] until the
//! composed paste hands the image to the application. Only the handoff releases the permit;
//! an application retaining pasted images is outside the codec budget by design.
//!
//! This module is only compiled for the authoritative Wayland transport with the
//! `clipboard-image` feature. It never opens arboard or smithay.

use core::time::Duration;
use std::{
    collections::VecDeque,
    io::{self, Cursor, Write},
    sync::Arc,
    time::Instant,
};

use egui::ColorImage;
use egui::mutex::Mutex;
use image::{ExtendedColorType, ImageEncoder as _, ImageFormat, ImageReader, Limits};
use parking_lot::Condvar;
use winit::platform::wayland::{ClipboardError, PngBytes, PngBytesWriter};

use crate::clipboard_bridge::{
    CompletionWaker, DecodedPasteImage, EncodedPng, ImagePasteDecoder, JobCancellation, PasteError,
    PasteErrorKind,
};

/// Largest accepted source or decoded image, in pixels.
pub(crate) const MAX_PIXELS: usize = 8_000_000;
/// Largest accepted width or height.
pub(crate) const MAX_DIMENSION: u32 = 8192;
/// Existing winit transfer cap, also the maximum encoded PNG this codec will produce.
pub(crate) const MAX_ENCODED_BYTES: usize = 16 * 1024 * 1024;
/// Shared admission budget for codec-owned queued/running/ready work.
pub(crate) const CODEC_BUDGET_BYTES: usize = 192 * 1024 * 1024;
/// Decoder allowance passed to `image` as `Limits::max_alloc`. The limit is non-strict.
pub(crate) const DECODE_LIMIT_BYTES: u64 = 64 * 1024 * 1024;
/// Copy encode and reservation delivery budget; winit additionally bounds the reservation
/// itself by the triggering input's five-second freshness.
pub(crate) const COPY_DELIVERY_BUDGET: Duration = Duration::from_secs(3);

/// The PNG signature every accepted encoded payload starts with.
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// One premultiplied source/conversion pixel. This is the `Color32` storage unit, not an RGBA
/// encoding assumption.
const COLOR_BYTES: usize = core::mem::size_of::<egui::Color32>();

fn codec_error(kind: PasteErrorKind, message: impl Into<String>) -> PasteError {
    PasteError::new(kind, message.into())
}

fn map_clipboard_error(error: &ClipboardError) -> PasteError {
    use ClipboardError as E;
    let kind = match error {
        E::Absent => PasteErrorKind::Absent,
        E::NotOffered => PasteErrorKind::NotOffered,
        E::StaleSelfSource => PasteErrorKind::StaleSelfSource,
        E::Interrupted => PasteErrorKind::Interrupted,
        E::TimedOut => PasteErrorKind::TimedOut,
        E::Unsupported => PasteErrorKind::Unsupported,
        E::Busy => PasteErrorKind::Busy,
        E::TooLarge => PasteErrorKind::TooLarge,
        E::InvalidInput => PasteErrorKind::InvalidInput,
        E::Disconnected => PasteErrorKind::Disconnected,
        E::Backend(_) => PasteErrorKind::Backend,
    };
    PasteError::new(kind, error.to_string())
}

/// Validate a caller-owned image against the bounded codec limits.
pub(crate) fn validate_image_size(size: [usize; 2]) -> Result<usize, PasteError> {
    let [width, height] = size;
    if width == 0 || height == 0 {
        return Err(codec_error(
            PasteErrorKind::InvalidInput,
            "an empty image cannot be encoded",
        ));
    }
    if width > MAX_DIMENSION as usize || height > MAX_DIMENSION as usize {
        return Err(codec_error(
            PasteErrorKind::TooLarge,
            format!("image dimension {width}x{height} exceeds {MAX_DIMENSION}"),
        ));
    }
    let pixels = width
        .checked_mul(height)
        .ok_or_else(|| codec_error(PasteErrorKind::TooLarge, "image pixel count overflowed"))?;
    if pixels > MAX_PIXELS {
        return Err(codec_error(
            PasteErrorKind::TooLarge,
            format!("image has {pixels} pixels, above {MAX_PIXELS}"),
        ));
    }
    Ok(pixels)
}

/// One validated caller image and the storage its encode admission must cover.
struct ImageAdmission {
    /// Logical pixel count from the declared dimensions; equal to `pixels.len()`.
    pixels: usize,
    /// Straight-alpha RGBA conversion buffer bytes.
    converted_bytes: usize,
    /// Retained source capacity charged when it exceeds the logical pixel count.
    retained_capacity_bytes: usize,
}

/// Validate the declared size and actual pixel buffer together before any admission or codec
/// work. A `ColorImage` has public fields, so dimensions alone are not a trustworthy description
/// of the vector that the worker will iterate.
fn validate_color_image_admission(image: &ColorImage) -> Result<ImageAdmission, PasteError> {
    let pixels = validate_image_size(image.size)?;
    if image.pixels.len() != pixels {
        return Err(codec_error(
            PasteErrorKind::InvalidInput,
            format!(
                "image size {}x{} requires {pixels} pixels, but the buffer has {}",
                image.size[0],
                image.size[1],
                image.pixels.len()
            ),
        ));
    }
    let logical_bytes = pixels
        .checked_mul(COLOR_BYTES)
        .ok_or_else(|| codec_error(PasteErrorKind::TooLarge, "image byte count overflowed"))?;
    let retained_bytes = image
        .pixels
        .capacity()
        .checked_mul(COLOR_BYTES)
        .ok_or_else(|| codec_error(PasteErrorKind::TooLarge, "image capacity overflowed"))?;
    Ok(ImageAdmission {
        pixels,
        converted_bytes: logical_bytes,
        retained_capacity_bytes: if image.pixels.capacity() > pixels {
            retained_bytes
        } else {
            0
        },
    })
}

/// Validate a caller-owned image against the declared dimensions and its actual pixel buffer.
pub(crate) fn validate_color_image(image: &ColorImage) -> Result<(), PasteError> {
    validate_color_image_admission(image).map(|_| ())
}

fn validate_decoded_dimensions(width: u32, height: u32) -> Result<usize, PasteError> {
    if width == 0 || height == 0 {
        return Err(codec_error(
            PasteErrorKind::Malformed,
            "PNG has a zero dimension",
        ));
    }
    if width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(codec_error(
            PasteErrorKind::TooLarge,
            format!("PNG dimension {width}x{height} exceeds {MAX_DIMENSION}"),
        ));
    }
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| codec_error(PasteErrorKind::TooLarge, "PNG pixel count overflowed"))?;
    if pixels > MAX_PIXELS {
        return Err(codec_error(
            PasteErrorKind::TooLarge,
            format!("PNG has {pixels} pixels, above {MAX_PIXELS}"),
        ));
    }
    Ok(pixels)
}

/// One admission budget shared by encode and decode work.
struct CodecBudget {
    used: Mutex<usize>,
    limit: usize,
}

impl CodecBudget {
    fn new(limit: usize) -> Self {
        Self {
            used: Mutex::new(0),
            limit,
        }
    }

    fn try_acquire(self: &Arc<Self>, bytes: usize) -> Result<CodecPermit, PasteError> {
        let mut used = self.used.lock();
        let next = used
            .checked_add(bytes)
            .ok_or_else(|| codec_error(PasteErrorKind::Busy, "codec budget overflowed"))?;
        if next > self.limit {
            return Err(codec_error(
                PasteErrorKind::Busy,
                "the PNG codec admission budget is full",
            ));
        }
        *used = next;
        Ok(CodecPermit {
            budget: Arc::clone(self),
            bytes,
        })
    }

    #[cfg(test)]
    fn used(&self) -> usize {
        *self.used.lock()
    }
}

/// A permit for codec-owned retained storage. Dropping it releases the admission.
pub(crate) struct CodecPermit {
    budget: Arc<CodecBudget>,
    bytes: usize,
}

impl Drop for CodecPermit {
    fn drop(&mut self) {
        let mut used = self.budget.used.lock();
        *used = used.saturating_sub(self.bytes);
    }
}

impl core::fmt::Debug for CodecPermit {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CodecPermit")
            .field("bytes", &self.bytes)
            .finish_non_exhaustive()
    }
}

fn codec_budget() -> &'static Arc<CodecBudget> {
    static BUDGET: std::sync::OnceLock<Arc<CodecBudget>> = std::sync::OnceLock::new();
    BUDGET.get_or_init(|| Arc::new(CodecBudget::new(CODEC_BUDGET_BYTES)))
}

type EncodeCompletion = Box<dyn FnOnce(Result<PngBytes, PasteError>) + Send + 'static>;
type DecodeCompletion = Box<dyn FnOnce(Result<DecodedPasteImage, PasteError>) + Send + 'static>;

struct EncodeJob {
    image: ColorImage,
    /// The validated logical pixel count, equal to `image.pixels.len()`.
    pixels: usize,
    permit: CodecPermit,
    completion: EncodeCompletion,
}

struct DecodeJob {
    png: Arc<dyn EncodedPng>,
    permit: CodecPermit,
    deadline: Instant,
    completion: DecodeCompletion,
}

/// A queued job plus its cancellation state.
struct JobSlot<T> {
    state: Mutex<SlotState<T>>,
}

enum SlotState<T> {
    Queued(T),
    Running,
    Cancelled,
    Finished,
}

impl<T> JobSlot<T> {
    fn new(job: T) -> Self {
        Self {
            state: Mutex::new(SlotState::Queued(job)),
        }
    }

    /// Take a still-queued payload. A cancelled slot yields nothing, so the worker skips it.
    fn take_queued(&self) -> Option<T> {
        let mut state = self.state.lock();
        match core::mem::replace(&mut *state, SlotState::Running) {
            SlotState::Queued(job) => Some(job),
            other => {
                *state = other;
                None
            }
        }
    }

    /// Cancel queued work and return its payload for dropping outside all queue locks. Running
    /// work is only marked cancelled; its payload and permit stay budgeted until it completes.
    fn cancel_queued(&self) -> Option<T> {
        let mut state = self.state.lock();
        match core::mem::replace(&mut *state, SlotState::Cancelled) {
            SlotState::Queued(job) => Some(job),
            SlotState::Running => {
                *state = SlotState::Running;
                None
            }
            other => {
                *state = other;
                None
            }
        }
    }

    fn mark_finished(&self) {
        let mut state = self.state.lock();
        *state = SlotState::Finished;
    }
}

struct MailboxInner<T> {
    queue: VecDeque<Arc<JobSlot<T>>>,
    closed: bool,
}

/// A bounded job mailbox. Unlike `sync_channel`, cancelling a queued job removes its envelope
/// and immediately releases the payload and any permit it holds.
struct JobMailbox<T> {
    inner: Mutex<MailboxInner<T>>,
    wake: Condvar,
    capacity: usize,
}

impl<T> JobMailbox<T> {
    fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(MailboxInner {
                queue: VecDeque::new(),
                closed: false,
            }),
            wake: Condvar::new(),
            capacity,
        }
    }

    fn try_push(self: &Arc<Self>, job: T) -> Result<JobHandle<T>, T> {
        let mut inner = self.inner.lock();
        if inner.closed || inner.queue.len() >= self.capacity {
            return Err(job);
        }
        let slot = Arc::new(JobSlot::new(job));
        inner.queue.push_back(Arc::clone(&slot));
        drop(inner);
        self.wake.notify_one();
        Ok(JobHandle {
            mailbox: Arc::clone(self),
            slot,
        })
    }

    /// Remove `slot` from the queue if it has not been dequeued, then cancel it. The returned
    /// payload must be dropped by the caller after all mailbox and slot locks are released.
    fn cancel(&self, slot: &Arc<JobSlot<T>>) -> Option<T> {
        {
            let mut inner = self.inner.lock();
            if let Some(position) = inner
                .queue
                .iter()
                .position(|queued| Arc::ptr_eq(queued, slot))
            {
                inner.queue.remove(position);
            }
        }
        slot.cancel_queued()
    }

    fn pop_blocking(&self) -> Option<Arc<JobSlot<T>>> {
        let mut inner = self.inner.lock();
        loop {
            if let Some(slot) = inner.queue.pop_front() {
                return Some(slot);
            }
            if inner.closed {
                return None;
            }
            self.wake.wait(&mut inner);
        }
    }

    fn close(&self) {
        let mut inner = self.inner.lock();
        inner.closed = true;
        drop(inner);
        self.wake.notify_all();
    }

    #[cfg(test)]
    fn try_pop(&self) -> Option<Arc<JobSlot<T>>> {
        let mut inner = self.inner.lock();
        inner.queue.pop_front()
    }

    #[cfg(test)]
    fn queue_len(&self) -> usize {
        let inner = self.inner.lock();
        inner.queue.len()
    }
}

/// The caller-side cancellation handle for one queued/running job.
struct JobHandle<T> {
    mailbox: Arc<JobMailbox<T>>,
    slot: Arc<JobSlot<T>>,
}

impl<T> JobHandle<T> {
    /// Cancel without waking anybody. Queued work is removed immediately; running work may
    /// finish and remains budgeted until its worker drops it.
    fn cancel(&self) {
        if let Some(job) = self.mailbox.cancel(&self.slot) {
            drop(job);
        }
    }
}

impl<T: Send + 'static> JobCancellation for JobHandle<T> {
    fn cancel(&self) {
        Self::cancel(self);
    }
}

/// The process-global, bounded codec workers.
struct CodecEngine {
    encode_mailbox: Arc<JobMailbox<EncodeJob>>,
    decode_mailbox: Arc<JobMailbox<DecodeJob>>,
    budget: Arc<CodecBudget>,
}

impl Drop for CodecEngine {
    fn drop(&mut self) {
        self.encode_mailbox.close();
        self.decode_mailbox.close();
    }
}

fn engine() -> Result<&'static CodecEngine, PasteError> {
    static ENGINE: std::sync::OnceLock<Result<CodecEngine, String>> = std::sync::OnceLock::new();
    ENGINE
        .get_or_init(CodecEngine::start)
        .as_ref()
        .map_err(|message| {
            codec_error(
                PasteErrorKind::Disconnected,
                format!("the PNG codec workers are unavailable: {message}"),
            )
        })
}

impl CodecEngine {
    fn start() -> Result<Self, String> {
        Self::start_with_budget(Arc::clone(codec_budget()))
    }

    fn start_with_budget(budget: Arc<CodecBudget>) -> Result<Self, String> {
        let encode_mailbox = Arc::new(JobMailbox::new(1));
        let decode_mailbox = Arc::new(JobMailbox::new(1));

        let encode_thread_mailbox = Arc::clone(&encode_mailbox);
        if let Err(error) = std::thread::Builder::new()
            .name("egui-winit-png-encode".to_owned())
            .spawn(move || encode_worker(&encode_thread_mailbox))
        {
            encode_mailbox.close();
            return Err(error.to_string());
        }
        let decode_thread_mailbox = Arc::clone(&decode_mailbox);
        if let Err(error) = std::thread::Builder::new()
            .name("egui-winit-png-decode".to_owned())
            .spawn(move || decode_worker(&decode_thread_mailbox))
        {
            encode_mailbox.close();
            decode_mailbox.close();
            return Err(error.to_string());
        }

        Ok(Self {
            encode_mailbox,
            decode_mailbox,
            budget,
        })
    }

    fn submit_encode(
        &self,
        image: ColorImage,
        completion: EncodeCompletion,
    ) -> Result<Arc<dyn JobCancellation>, PasteError> {
        // Validate the public size fields and actual pixel buffer together before admission.
        // The permit covers the conversion buffer, the encoded output and any retained capacity
        // beyond the logical pixel count. The encoder's bounded writer acquires winit's PNG
        // permit before allocating its output.
        let admission = validate_color_image_admission(&image)?;
        let transient = admission
            .converted_bytes
            .checked_add(admission.retained_capacity_bytes)
            .and_then(|bytes| bytes.checked_add(MAX_ENCODED_BYTES))
            .ok_or_else(|| codec_error(PasteErrorKind::TooLarge, "encode budget overflowed"))?;
        let permit = self.budget.try_acquire(transient)?;
        let job = EncodeJob {
            image,
            pixels: admission.pixels,
            permit,
            completion,
        };
        match self.encode_mailbox.try_push(job) {
            Ok(handle) => Ok(Arc::new(handle) as Arc<dyn JobCancellation>),
            Err(job) => {
                // Dropping outside the mailbox lock releases the permit promptly.
                drop(job);
                Err(codec_error(
                    PasteErrorKind::Busy,
                    "the PNG codec queue is full",
                ))
            }
        }
    }

    fn submit_decode(
        &self,
        png: Arc<dyn EncodedPng>,
        deadline: Instant,
        completion: DecodeCompletion,
    ) -> Result<Arc<dyn JobCancellation>, PasteError> {
        // Reserve the encoder/reader transient allowance, both pixel buffers and the encoded
        // input before the worker allocates anything.
        let pixels = MAX_PIXELS
            .checked_mul(8)
            .ok_or_else(|| codec_error(PasteErrorKind::TooLarge, "decode budget overflowed"))?;
        let transient = pixels
            .checked_add(DECODE_LIMIT_BYTES as usize)
            .and_then(|bytes| bytes.checked_add(png.len()))
            .ok_or_else(|| codec_error(PasteErrorKind::TooLarge, "decode budget overflowed"))?;
        let permit = match self.budget.try_acquire(transient) {
            Ok(permit) => permit,
            Err(error) => {
                completion(Err(error.clone()));
                return Err(error);
            }
        };
        let job = DecodeJob {
            png,
            permit,
            deadline,
            completion,
        };
        match self.decode_mailbox.try_push(job) {
            Ok(handle) => Ok(Arc::new(handle) as Arc<dyn JobCancellation>),
            Err(job) => {
                let DecodeJob {
                    completion, permit, ..
                } = job;
                drop(permit);
                let error = codec_error(PasteErrorKind::Busy, "the PNG codec queue is full");
                completion(Err(error.clone()));
                Err(error)
            }
        }
    }
}

fn encode_worker(mailbox: &JobMailbox<EncodeJob>) {
    while let Some(slot) = mailbox.pop_blocking() {
        if let Some(job) = slot.take_queued() {
            run_encode(job);
        }
        slot.mark_finished();
    }
}

fn decode_worker(mailbox: &JobMailbox<DecodeJob>) {
    while let Some(slot) = mailbox.pop_blocking() {
        if let Some(job) = slot.take_queued() {
            run_decode(job);
        }
        slot.mark_finished();
    }
}

/// A `Write` handle whose permit-bearing [`PngBytesWriter`] can be recovered after the image
/// encoder consumes its writer value.
#[derive(Clone)]
struct SharedPngWriter {
    writer: Arc<Mutex<Option<PngBytesWriter>>>,
}

impl SharedPngWriter {
    fn new(writer: PngBytesWriter) -> Self {
        Self {
            writer: Arc::new(Mutex::new(Some(writer))),
        }
    }

    fn finish(&self) -> Result<PngBytes, PasteError> {
        let writer =
            self.writer.lock().take().ok_or_else(|| {
                codec_error(PasteErrorKind::Internal, "PNG writer was already taken")
            })?;
        writer.finish().map_err(|error| map_clipboard_error(&error))
    }
}

impl Write for SharedPngWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let mut writer = self.writer.lock();
        let writer = writer
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "PNG writer is closed"))?;
        writer.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut writer = self.writer.lock();
        match writer.as_mut() {
            Some(writer) => writer.flush(),
            None => Ok(()),
        }
    }
}

fn run_encode(job: EncodeJob) {
    let EncodeJob {
        image,
        pixels,
        permit: _permit,
        completion,
    } = job;
    let [width, height] = image.size;
    // Recheck the invariant that made admission and the conversion reserve exact. This is a
    // cheap defensive check, not a second admission path.
    let converted_bytes = match pixels.checked_mul(COLOR_BYTES) {
        Some(bytes) if image.pixels.len() == pixels => bytes,
        _ => {
            completion(Err(codec_error(
                PasteErrorKind::Internal,
                "the queued image no longer matches its admitted pixel count",
            )));
            return;
        }
    };

    let mut rgba = Vec::new();
    if rgba.try_reserve_exact(converted_bytes).is_err() {
        completion(Err(codec_error(
            PasteErrorKind::Busy,
            "the PNG conversion buffer could not be allocated",
        )));
        return;
    }
    if rgba.capacity() < converted_bytes {
        completion(Err(codec_error(
            PasteErrorKind::Internal,
            "the PNG conversion buffer was not reserved exactly",
        )));
        return;
    }
    for pixel in &image.pixels {
        // The validated length and exact reserve guarantee this cannot grow the vector.
        rgba.extend_from_slice(&pixel.to_srgba_unmultiplied());
    }
    if rgba.len() != converted_bytes {
        completion(Err(codec_error(
            PasteErrorKind::Internal,
            "the PNG conversion buffer length changed during conversion",
        )));
        return;
    }

    let (Ok(width_u32), Ok(height_u32)) = (u32::try_from(width), u32::try_from(height)) else {
        completion(Err(codec_error(
            PasteErrorKind::TooLarge,
            "image dimensions do not fit in the PNG format",
        )));
        return;
    };

    let writer = match PngBytesWriter::new() {
        Ok(writer) => SharedPngWriter::new(writer),
        Err(error) => {
            completion(Err(map_clipboard_error(&error)));
            return;
        }
    };

    let encoded = image::codecs::png::PngEncoder::new(writer.clone()).write_image(
        &rgba,
        width_u32,
        height_u32,
        ExtendedColorType::Rgba8,
    );
    let result = match encoded {
        Ok(()) => writer.finish(),
        Err(error) => Err(codec_error(
            PasteErrorKind::Encode,
            format!("PNG encoding failed: {error}"),
        )),
    };
    drop(rgba);
    completion(result);
}

fn run_decode(job: DecodeJob) {
    let DecodeJob {
        png,
        permit,
        deadline,
        completion,
    } = job;

    if Instant::now() >= deadline {
        completion(Err(codec_error(
            PasteErrorKind::TimedOut,
            "the composed paste delivery deadline expired before decode",
        )));
        return;
    }

    let bytes = png.as_bytes();
    if bytes.len() < PNG_SIGNATURE.len() || bytes[..PNG_SIGNATURE.len()] != PNG_SIGNATURE {
        completion(Err(codec_error(
            PasteErrorKind::Malformed,
            "the clipboard image is not a PNG payload",
        )));
        return;
    }

    let result = decode_png(bytes);
    match result {
        Ok(image) => completion(Ok(DecodedPasteImage::new(image, Box::new(permit)))),
        Err(error) => completion(Err(error)),
    }
}

fn png_limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(DECODE_LIMIT_BYTES);
    limits
}

/// Decode one PNG payload with explicit format, strict dimension limits and a checked pixel
/// product before allocating the final [`ColorImage`].
pub(crate) fn decode_png(bytes: &[u8]) -> Result<ColorImage, PasteError> {
    if bytes.len() < PNG_SIGNATURE.len() || bytes[..PNG_SIGNATURE.len()] != PNG_SIGNATURE {
        return Err(codec_error(
            PasteErrorKind::Malformed,
            "the clipboard image is not a PNG payload",
        ));
    }

    let mut header = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Png);
    header.limits(png_limits());
    let (width, height) = header.into_dimensions().map_err(|error| {
        codec_error(
            PasteErrorKind::Malformed,
            format!("PNG header could not be decoded: {error}"),
        )
    })?;
    let pixels = validate_decoded_dimensions(width, height)?;

    let mut reader = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Png);
    reader.limits(png_limits());
    let dynamic = reader.decode().map_err(|error| {
        codec_error(
            PasteErrorKind::Malformed,
            format!("PNG payload could not be decoded: {error}"),
        )
    })?;
    let rgba = dynamic.into_rgba8();
    if rgba.width() as usize != width as usize || rgba.height() as usize != height as usize {
        return Err(codec_error(
            PasteErrorKind::Malformed,
            "decoded PNG dimensions changed between header and payload",
        ));
    }
    if rgba.as_raw().len() != pixels.saturating_mul(4) {
        return Err(codec_error(
            PasteErrorKind::Malformed,
            "decoded PNG buffer length does not match its dimensions",
        ));
    }
    color_image_from_rgba([width as usize, height as usize], rgba.as_raw(), pixels)
}

/// Convert straight-alpha RGBA bytes to premultiplied [`egui::Color32`] pixels with a fallible
/// final allocation.
fn color_image_from_rgba(
    size: [usize; 2],
    rgba: &[u8],
    pixels: usize,
) -> Result<ColorImage, PasteError> {
    let mut colors = Vec::new();
    colors.try_reserve_exact(pixels).map_err(|_error| {
        codec_error(
            PasteErrorKind::Busy,
            "the decoded ColorImage could not be allocated",
        )
    })?;
    for pixel in rgba.chunks_exact(4) {
        colors.push(egui::Color32::from_rgba_unmultiplied(
            pixel[0], pixel[1], pixel[2], pixel[3],
        ));
    }
    Ok(ColorImage::new(size, colors))
}

/// A ready-to-install copy encode. The event loop owns the winit reservation separately.
pub(crate) struct ImageCopyRequest {
    shared: Arc<CopyShared>,
}

struct CopyShared {
    state: Mutex<CopyState>,
    terminal_waker: CompletionWaker,
}

struct CopyState {
    result: Option<Result<PngBytes, PasteError>>,
    terminal_woken: bool,
    cancelled: bool,
    /// Cancellation handle for the queued encode, if it has not completed.
    job: Option<Arc<dyn JobCancellation>>,
}

impl Clone for ImageCopyRequest {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl ImageCopyRequest {
    /// Move `image` into the bounded encode worker. No clone or pixel copy happens on the
    /// calling (event-loop) thread.
    pub(crate) fn start(
        image: ColorImage,
        terminal_waker: CompletionWaker,
    ) -> Result<Self, PasteError> {
        let request = Self {
            shared: Arc::new(CopyShared {
                state: Mutex::new(CopyState {
                    result: None,
                    terminal_woken: false,
                    cancelled: false,
                    job: None,
                }),
                terminal_waker,
            }),
        };
        let completion_request = request.clone();
        let job = engine()?.submit_encode(
            image,
            Box::new(move |result| completion_request.complete(result)),
        )?;
        {
            let mut state = request.shared.state.lock();
            if !state.cancelled && state.result.is_none() {
                state.job = Some(job);
            } else {
                drop(state);
                job.cancel();
            }
        }
        Ok(request)
    }

    fn complete(&self, result: Result<PngBytes, PasteError>) {
        let should_wake = {
            let mut state = self.shared.state.lock();
            if state.cancelled || state.result.is_some() {
                false
            } else {
                state.result = Some(result);
                state.job = None;
                if state.terminal_woken {
                    false
                } else {
                    state.terminal_woken = true;
                    true
                }
            }
        };
        if should_wake {
            (self.shared.terminal_waker)();
        }
    }

    /// Take the completed encode exactly once. A cancelled request never yields a result.
    pub(crate) fn try_take(&self) -> Option<Result<PngBytes, PasteError>> {
        self.shared.state.lock().result.take()
    }

    /// Cancel without waking the caller. Queued encode work is removed and its payload/permit
    /// released promptly; a running encode may finish, but its result cannot be delivered.
    pub(crate) fn cancel(&self) {
        let job = {
            let mut state = self.shared.state.lock();
            state.cancelled = true;
            state.result = None;
            state.job.take()
        };
        if let Some(job) = job {
            job.cancel();
        }
    }
}

/// The production decoder used by the composed regular-paste state.
pub(crate) struct CodecPasteDecoder;

impl ImagePasteDecoder for CodecPasteDecoder {
    fn decode(
        &self,
        png: Arc<dyn EncodedPng>,
        deadline: Instant,
        completion: Box<dyn FnOnce(Result<DecodedPasteImage, PasteError>) + Send + 'static>,
    ) -> Option<Arc<dyn JobCancellation>> {
        if Instant::now() >= deadline {
            completion(Err(codec_error(
                PasteErrorKind::TimedOut,
                "the composed paste delivery deadline expired before decode admission",
            )));
            return None;
        }
        // `submit_decode` invokes the completion itself on admission failure.
        match engine() {
            Ok(engine) => match engine.submit_decode(png, deadline, completion) {
                Ok(job) => Some(job),
                Err(error) => {
                    log::debug!("PNG decode admission failed: {error}");
                    None
                }
            },
            Err(error) => {
                completion(Err(error));
                None
            }
        }
    }
}

/// Construct the production paste decoder.
pub(crate) fn paste_decoder() -> Arc<dyn ImagePasteDecoder> {
    Arc::new(CodecPasteDecoder)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicUsize, Ordering};
    use egui::Color32;
    use std::sync::mpsc::sync_channel;

    struct BytesPng(Vec<u8>);

    impl EncodedPng for BytesPng {
        fn as_bytes(&self) -> &[u8] {
            &self.0
        }

        fn len(&self) -> usize {
            self.0.len()
        }
    }

    fn encoded_png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(rgba, width, height, ExtendedColorType::Rgba8)
            .expect("test PNG encodes");
        bytes
    }

    #[test]
    fn codec_budget_rejects_before_exceeding_its_limit() {
        let budget = Arc::new(CodecBudget::new(10));
        let permit = budget.try_acquire(8).expect("first admission fits");
        assert_eq!(budget.used(), 8);
        assert!(budget.try_acquire(3).is_err());
        drop(permit);
        assert_eq!(budget.used(), 0);
        assert!(budget.try_acquire(10).is_ok());
    }

    #[test]
    fn queue_full_is_reported_as_busy_and_cancelled_slot_is_reusable() {
        let mailbox = Arc::new(JobMailbox::<u8>::new(1));
        let first = mailbox.try_push(1).expect("first job fits");
        assert!(mailbox.try_push(2).is_err(), "second job must be refused");
        assert_eq!(mailbox.queue_len(), 1);
        first.cancel();
        assert_eq!(mailbox.queue_len(), 0, "cancelled queued job is removed");
        let _second = mailbox.try_push(3).expect("cancelled slot is reusable");
        assert_eq!(mailbox.queue_len(), 1);
    }

    #[test]
    fn cancelled_queued_job_releases_its_permit_for_followup_admission() {
        let budget = Arc::new(CodecBudget::new(CODEC_BUDGET_BYTES));
        let mailbox = Arc::new(JobMailbox::<CodecPermit>::new(1));
        let first_permit = budget.try_acquire(4096).expect("first permit fits");
        let first = mailbox.try_push(first_permit).expect("first job queued");
        assert_eq!(budget.used(), 4096);

        let second_permit = budget.try_acquire(512).expect("second permit fits");
        let Err(rejected) = mailbox.try_push(second_permit) else {
            panic!("a full mailbox must reject the second job");
        };
        drop(rejected);
        assert_eq!(budget.used(), 4096, "rejected job released its permit");

        first.cancel();
        assert_eq!(budget.used(), 0, "queued cancellation released its permit");
        let followup_permit = budget.try_acquire(8192).expect("follow-up permit fits");
        let _followup = mailbox
            .try_push(followup_permit)
            .expect("cancelled queue slot accepts follow-up work");
    }

    #[test]
    fn cancelling_a_running_job_keeps_its_permit_until_completion() {
        let budget = Arc::new(CodecBudget::new(CODEC_BUDGET_BYTES));
        let mailbox = Arc::new(JobMailbox::<CodecPermit>::new(1));
        let permit = budget.try_acquire(2048).expect("permit fits");
        let handle = mailbox.try_push(permit).expect("job queued");
        let slot = mailbox.try_pop().expect("worker dequeues the job");
        let running = slot.take_queued().expect("worker takes the payload");
        assert_eq!(budget.used(), 2048);

        handle.cancel();
        assert_eq!(
            budget.used(),
            2048,
            "running work stays budgeted until the worker drops it"
        );
        drop(running);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn malformed_color_image_is_rejected_before_codec_admission() {
        let budget = Arc::new(CodecBudget::new(CODEC_BUDGET_BYTES));
        let engine = CodecEngine::start_with_budget(Arc::clone(&budget)).expect("engine starts");
        let mut image = ColorImage::new([2, 2], vec![Color32::RED; 4]);
        image.pixels.truncate(3);
        let mut extra_pixels = ColorImage::new([1, 1], vec![Color32::RED]);
        extra_pixels.pixels.push(Color32::RED);
        assert_eq!(
            validate_color_image(&extra_pixels).unwrap_err().kind(),
            PasteErrorKind::InvalidInput,
            "a buffer longer than its dimensions is malformed too"
        );
        let (sender, _receiver) = sync_channel(1);
        let result = engine.submit_encode(
            image,
            Box::new(move |result| {
                let _ = sender.send(result);
            }),
        );
        let Err(error) = result else {
            panic!("a size/pixels mismatch must be rejected");
        };
        assert_eq!(error.kind(), PasteErrorKind::InvalidInput);
        assert_eq!(budget.used(), 0, "no codec permit was acquired");
        assert_eq!(
            engine.encode_mailbox.queue_len(),
            0,
            "no job reached the mailbox"
        );
    }

    #[test]
    fn retained_pixel_capacity_beyond_logical_size_is_charged() {
        let mut pixels = Vec::with_capacity(1024);
        pixels.push(Color32::RED);
        let mut image = ColorImage::new([1, 1], vec![Color32::RED]);
        image.pixels = pixels;
        let admission = validate_color_image_admission(&image).expect("valid image");
        assert_eq!(admission.pixels, 1);
        assert_eq!(admission.converted_bytes, COLOR_BYTES);
        assert!(
            admission.retained_capacity_bytes >= 1024 * COLOR_BYTES,
            "retained capacity beyond the logical buffer is part of admission"
        );
        assert!(validate_color_image(&image).is_ok());
    }

    #[test]
    fn dimensions_are_checked_before_encode_or_conversion() {
        assert_eq!(validate_image_size([8192, 976]).unwrap(), 7_995_392);
        assert_eq!(
            validate_image_size([8193, 1]).unwrap_err().kind(),
            PasteErrorKind::TooLarge
        );
        assert_eq!(
            validate_image_size([4000, 4000]).unwrap_err().kind(),
            PasteErrorKind::TooLarge
        );
        assert_eq!(
            validate_decoded_dimensions(8193, 1).unwrap_err().kind(),
            PasteErrorKind::TooLarge
        );
        assert_eq!(
            validate_decoded_dimensions(1, 1).unwrap(),
            1,
            "checked product is small"
        );
    }

    #[test]
    fn malformed_png_signature_is_rejected_without_decoding() {
        assert_eq!(
            decode_png(b"not a png").unwrap_err().kind(),
            PasteErrorKind::Malformed
        );
        assert_eq!(
            decode_png(&[]).unwrap_err().kind(),
            PasteErrorKind::Malformed
        );
    }

    #[test]
    fn oversized_png_header_is_rejected_before_pixel_conversion() {
        let mut bytes = PNG_SIGNATURE.to_vec();
        bytes.extend_from_slice(&[0, 0, 0, 13, b'I', b'H', b'D', b'R']);
        bytes.extend_from_slice(&8193u32.to_be_bytes());
        bytes.extend_from_slice(&1u32.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        // A CRC is not needed: the limit is checked while reading the header. Even if the
        // decoder rejects the truncated file first, it must not allocate a pixel buffer.
        let error = decode_png(&bytes).expect_err("oversized header must fail");
        assert!(matches!(
            error.kind(),
            PasteErrorKind::TooLarge | PasteErrorKind::Malformed
        ));
    }

    #[test]
    fn bounded_writer_refuses_oversized_output_before_extending() {
        let mut writer = PngBytesWriter::with_limit(4).expect("small writer");
        writer.write_all(&[1, 2, 3, 4]).expect("exact limit fits");
        let error = writer
            .write_all(&[5])
            .expect_err("growth beyond limit fails");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(writer.len(), 4);
    }

    #[test]
    fn decode_converts_straight_alpha_to_premultiplied() {
        let png = encoded_png(2, 1, &[255, 0, 0, 255, 255, 255, 255, 128]);
        let image = decode_png(&png).expect("valid PNG decodes");
        assert_eq!(image.size, [2, 1]);
        assert_eq!(
            image.pixels[0],
            Color32::from_rgba_unmultiplied(255, 0, 0, 255)
        );
        assert_eq!(
            image.pixels[1],
            Color32::from_rgba_unmultiplied(255, 255, 255, 128)
        );
    }

    #[test]
    fn encode_roundtrip_preserves_opaque_and_alpha_pixels() {
        let image = ColorImage::new(
            [2, 1],
            vec![
                Color32::from_rgba_unmultiplied(12, 34, 56, 255),
                Color32::from_rgba_unmultiplied(200, 100, 50, 128),
            ],
        );
        let mut rgba = Vec::new();
        for pixel in &image.pixels {
            rgba.extend_from_slice(&pixel.to_srgba_unmultiplied());
        }
        let png = encoded_png(2, 1, &rgba);
        let decoded = decode_png(&png).expect("roundtrip decodes");
        assert_eq!(decoded.pixels, image.pixels);
    }
    #[test]
    fn expired_decode_deadline_fails_before_allocation() {
        let (sender, receiver) = sync_channel(1);
        let permit = codec_budget().try_acquire(1).expect("small permit");
        let job = DecodeJob {
            png: Arc::new(BytesPng(vec![0; 8])),
            permit,
            deadline: Instant::now()
                .checked_sub(Duration::from_millis(1))
                .expect("instant is representable"),
            completion: Box::new(move |result| {
                sender.send(result).expect("test receiver is alive");
            }),
        };
        run_decode(job);
        let result = receiver.recv().expect("completion runs");
        assert_eq!(
            result.expect_err("late decode must fail").kind(),
            PasteErrorKind::TimedOut
        );
    }

    fn copy_request(wakes: Arc<AtomicUsize>) -> ImageCopyRequest {
        ImageCopyRequest {
            shared: Arc::new(CopyShared {
                state: Mutex::new(CopyState {
                    result: None,
                    terminal_woken: false,
                    cancelled: false,
                    job: None,
                }),
                terminal_waker: Arc::new(move || {
                    wakes.fetch_add(1, Ordering::AcqRel);
                }),
            }),
        }
    }

    #[test]
    fn cancelled_copy_request_never_wakes_or_returns_a_result() {
        let wakes = Arc::new(AtomicUsize::new(0));
        let request = copy_request(Arc::clone(&wakes));
        request.cancel();
        request.complete(Err(codec_error(PasteErrorKind::TimedOut, "late")));
        assert_eq!(wakes.load(Ordering::Acquire), 0);
        assert!(request.try_take().is_none());
    }

    struct RecordingCancellation(Arc<AtomicUsize>);

    impl JobCancellation for RecordingCancellation {
        fn cancel(&self) {
            self.0.fetch_add(1, Ordering::AcqRel);
        }
    }

    #[test]
    fn cancelled_copy_request_cancels_its_queued_job() {
        let wakes = Arc::new(AtomicUsize::new(0));
        let request = copy_request(Arc::clone(&wakes));
        let cancels = Arc::new(AtomicUsize::new(0));
        request.shared.state.lock().job =
            Some(Arc::new(RecordingCancellation(Arc::clone(&cancels))));
        request.cancel();
        assert_eq!(cancels.load(Ordering::Acquire), 1);
        assert_eq!(wakes.load(Ordering::Acquire), 0, "cancel never wakes");
        assert!(request.shared.state.lock().job.is_none());
    }

    #[test]
    fn copy_request_wakes_exactly_once_and_is_one_shot() {
        let wakes = Arc::new(AtomicUsize::new(0));
        let request = copy_request(Arc::clone(&wakes));
        request.complete(Err(codec_error(PasteErrorKind::Encode, "first")));
        request.complete(Err(codec_error(PasteErrorKind::Encode, "second")));
        assert_eq!(wakes.load(Ordering::Acquire), 1);
        assert!(request.try_take().is_some());
        assert!(request.try_take().is_none());
    }
}

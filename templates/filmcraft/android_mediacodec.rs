//! Android `AMediaCodec` hardware H.264 / HEVC video decoder and H.264 video encoder.
//!
//! Uses Android's NDK `libmediandk` to decode and encode video using the device's fixed-function
//! hardware video processing unit (Qualcomm Venus/Adreno, MediaTek, Exynos, Google Tensor).
//!
//! Output frames for export are formatted as length-prefixed NAL units (AVCC) for ISO BMFF (MP4).
//! When the hardware encoder or decoder cannot be initialized, is a slow software emulation, or declines
//! the requested settings (e.g. non-square pixels, odd dimensions, HDR), the factory returns `None`
//! and FilmCraft automatically falls back to software processing.

#![allow(unsafe_code)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use filmcraft_codecs::hw::{NalCodec, NalStreamInfo};
use filmcraft_codecs::{CodecError, DecodedFrame, VideoDecoder};
use filmcraft_export::{
    BitrateMode, EncodedPacket, EncoderFrame, ExportError, ExportSettings, Format,
    HardwareEncoding, PrepareAhead, Result, VideoEncoder,
};
use filmcraft_isobmff::{AvcConfig, SampleEntry};
use filmcraft_time::FrameRate;
use ndk::hardware_buffer_format::HardwareBufferFormat;
use ndk::media::media_codec::{
    DequeuedInputBufferResult, DequeuedOutputBufferInfoResult, MediaCodec, MediaCodecDirection,
};
use ndk::media::media_format::MediaFormat;
use ndk::native_window::NativeWindow;

/// Maximum concurrent active hardware video decoders allowed on Android.
/// Qualcomm Venus/Adreno allocates ~250MB of contiguous DMA buffers per 4K decoder session.
/// Limiting to 1 concurrent decoder guarantees the driver never exhausts ION/dma-buf memory,
/// preventing driver MMAP failures and avoiding kernel low-memory-killer (LMK) eviction
/// of com.miui.home and home-screen widgets.
const MAX_CONCURRENT_HW_DECODERS: usize = 1;

static ACTIVE_HW_DECODERS: AtomicUsize = AtomicUsize::new(0);

fn try_acquire_hw_decoder() -> bool {
    let mut current = ACTIVE_HW_DECODERS.load(Ordering::Relaxed);
    loop {
        if current >= MAX_CONCURRENT_HW_DECODERS {
            return false;
        }
        match ACTIVE_HW_DECODERS.compare_exchange_weak(
            current,
            current + 1,
            Ordering::SeqCst,
            Ordering::Relaxed,
        ) {
            Ok(_) => return true,
            Err(actual) => current = actual,
        }
    }
}

fn release_hw_decoder() {
    let _ = ACTIVE_HW_DECODERS.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |val| {
        Some(val.saturating_sub(1))
    });
}

#[cfg(target_os = "android")]
use jni::{
    jni_sig, jni_str,
    objects::{Global, JObject, JValue},
    JavaVM,
};

use crate::biplanar;
use crate::hybrid::HybridDecoder;

/// The NAL units of an Annex B byte stream (start codes `00 00 01` / `00 00 00 01`).
pub fn annex_b_nals(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0usize;
    while let Some(w) = data.get(i..i.saturating_add(3)) {
        if w == [0, 0, 1] {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut out = Vec::with_capacity(starts.len());
    for (k, &s) in starts.iter().enumerate() {
        let e = match starts.get(k + 1) {
            Some(&next_s) => {
                // If the next start code was 00 00 00 01 (4 bytes), exactly one zero byte precedes [0, 0, 1]
                let start_of_sc = next_s.saturating_sub(3);
                if start_of_sc > s && data.get(start_of_sc - 1) == Some(&0) {
                    start_of_sc - 1
                } else {
                    start_of_sc
                }
            }
            None => {
                // Trailing zero padding at the very end of the buffer
                let mut end = data.len();
                while end > s && data.get(end - 1) == Some(&0) {
                    end -= 1;
                }
                end
            }
        };
        if let Some(n) = data.get(s..e).filter(|n| !n.is_empty()) {
            out.push(n);
        }
    }
    out
}

/// Convert Annex B NAL units to 4-byte big-endian length-prefixed NAL units (AVCC).
/// Drops parameter sets (SPS / PPS) and access unit delimiters (AUD), which live in the container header.
pub fn annex_b_to_length_prefixed(data: &[u8]) -> std::result::Result<Vec<u8>, String> {
    let nals = annex_b_nals(data);
    if nals.is_empty() {
        return Err("the encoder produced no NAL units".into());
    }
    let mut out = Vec::with_capacity(data.len());
    for n in nals {
        let nal_type = n.first().map(|b| b & 0x1f);
        // Drop SPS (7), PPS (8), AUD (9)
        if matches!(nal_type, Some(7..=9) | None) {
            continue;
        }
        let len = u32::try_from(n.len()).map_err(|_| "a NAL unit longer than 4 GiB".to_string())?;
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(n);
    }
    if out.is_empty() {
        return Err("the encoder produced no picture data".into());
    }
    Ok(out)
}

/// The hardware encoder factory registered in FilmCraft's export registry.
pub fn factory(
    format: Format,
    w: u32,
    h: u32,
    rate: FrameRate,
    s: &ExportSettings,
) -> Option<Result<Box<dyn VideoEncoder>>> {
    if format != Format::H264 {
        return None;
    }
    if s.hardware_encoding == HardwareEncoding::Off {
        log::info!("MediaCodec hardware encoding declined: hardware encoding is off");
        filmcraft_export::note_hw_encode_declined();
        return None;
    }
    if s.format.is_mxf() {
        log::info!("MediaCodec hardware encoding declined: MXF carries an Annex B stream");
        filmcraft_export::note_hw_encode_declined();
        return None;
    }
    if s.signal.is_hdr() {
        log::info!("MediaCodec hardware encoding declined: HDR not supported in 8-bit AVC");
        filmcraft_export::note_hw_encode_declined();
        return None;
    }
    if w % 2 != 0 || h % 2 != 0 || w == 0 || h == 0 {
        log::info!("MediaCodec hardware encoding declined: odd or zero picture size ({w}x{h})");
        filmcraft_export::note_hw_encode_declined();
        return None;
    }
    if s.pixel_aspect.is_some_and(|(n, d)| n != d) {
        log::info!("MediaCodec hardware encoding declined: non-square pixels");
        filmcraft_export::note_hw_encode_declined();
        return None;
    }

    match MediaCodecEncoder::new(w, h, rate, s) {
        Ok(enc) => {
            log::info!("MediaCodec hardware encoding session started for {w}x{h}");
            filmcraft_export::note_hw_encode_session();
            Some(Ok(Box::new(enc)))
        }
        Err(why) => {
            log::warn!("MediaCodec hardware encoding declined: {why}");
            filmcraft_export::note_hw_encode_declined();
            None
        }
    }
}

type Nv12Frame = Vec<u8>;

/// Direct, high-performance BT.709 RGBA to NV12 (YUV420 semi-planar) integer conversion.
/// Writes directly into contiguous NV12 buffer:
/// [0..w*h]: Y plane
/// [w*h..w*h + (w*h)/2]: Interleaved UV plane (U, V, U, V...)
pub fn rgba_to_nv12(rgba: &[u8], w: usize, h: usize, out: &mut Vec<u8>) {
    let y_len = w * h;
    let uv_len = (w.div_ceil(2)) * (h.div_ceil(2)) * 2;
    let total = y_len + uv_len;
    if out.capacity() < total {
        out.reserve(total - out.len());
    }
    // SAFETY: Every byte of y_plane and uv_plane is written by the loop below.
    unsafe {
        out.set_len(total);
    }

    let (y_plane, uv_plane) = out.split_at_mut(y_len);
    let half_h = h / 2;
    let _half_w = w / 2;

    for y in 0..half_h {
        let row0 = y * 2;
        let row1 = row0 + 1;
        let row0_src = match rgba.get(row0 * w * 4..(row0 + 1) * w * 4) {
            Some(s) => s,
            None => continue,
        };
        let row1_src = if row1 < h {
            rgba.get(row1 * w * 4..(row1 + 1) * w * 4).unwrap_or(row0_src)
        } else {
            row0_src
        };

        let y_rows = match y_plane.get_mut(row0 * w..(row1 + 1).min(h) * w) {
            Some(s) => s,
            None => continue,
        };
        let (y_row0, y_row1) = if row1 < h {
            y_rows.split_at_mut(w)
        } else {
            (y_rows, &mut [][..])
        };
        let uv_row = match uv_plane.get_mut(y * w..(y + 1) * w) {
            Some(s) => s,
            None => continue,
        };

        let chunk0_iter = row0_src.chunks_exact(8);
        let chunk1_iter = row1_src.chunks_exact(8);
        let y0_iter = y_row0.chunks_exact_mut(2);
        let uv_iter = uv_row.chunks_exact_mut(2);

        if !y_row1.is_empty() {
            let y1_iter = y_row1.chunks_exact_mut(2);
            for ((((c0, c1), y0), y1), uv) in chunk0_iter.zip(chunk1_iter).zip(y0_iter).zip(y1_iter).zip(uv_iter) {
                let r00 = c0[0] as i32;
                let g00 = c0[1] as i32;
                let b00 = c0[2] as i32;

                let r01 = c0[4] as i32;
                let g01 = c0[5] as i32;
                let b01 = c0[6] as i32;

                let r10 = c1[0] as i32;
                let g10 = c1[1] as i32;
                let b10 = c1[2] as i32;

                let r11 = c1[4] as i32;
                let g11 = c1[5] as i32;
                let b11 = c1[6] as i32;

                // BT.709 integer formula scaled by 256
                y0[0] = (16 + ((54 * r00 + 183 * g00 + 18 * b00 + 128) >> 8)).clamp(16, 235) as u8;
                y0[1] = (16 + ((54 * r01 + 183 * g01 + 18 * b01 + 128) >> 8)).clamp(16, 235) as u8;

                y1[0] = (16 + ((54 * r10 + 183 * g10 + 18 * b10 + 128) >> 8)).clamp(16, 235) as u8;
                y1[1] = (16 + ((54 * r11 + 183 * g11 + 18 * b11 + 128) >> 8)).clamp(16, 235) as u8;

                let r_avg = (r00 + r01 + r10 + r11 + 2) >> 2;
                let g_avg = (g00 + g01 + g10 + g11 + 2) >> 2;
                let b_avg = (b00 + b01 + b10 + b11 + 2) >> 2;

                uv[0] = (128 + ((-29 * r_avg - 99 * g_avg + 128 * b_avg + 128) >> 8)).clamp(16, 240) as u8;
                uv[1] = (128 + ((128 * r_avg - 116 * g_avg - 12 * b_avg + 128) >> 8)).clamp(16, 240) as u8;
            }
        } else {
            for (((c0, c1), y0), uv) in chunk0_iter.zip(chunk1_iter).zip(y0_iter).zip(uv_iter) {
                let r00 = c0[0] as i32;
                let g00 = c0[1] as i32;
                let b00 = c0[2] as i32;

                let r01 = c0[4] as i32;
                let g01 = c0[5] as i32;
                let b01 = c0[6] as i32;

                let r10 = c1[0] as i32;
                let g10 = c1[1] as i32;
                let b10 = c1[2] as i32;

                let r11 = c1[4] as i32;
                let g11 = c1[5] as i32;
                let b11 = c1[6] as i32;

                y0[0] = (16 + ((54 * r00 + 183 * g00 + 18 * b00 + 128) >> 8)).clamp(16, 235) as u8;
                y0[1] = (16 + ((54 * r01 + 183 * g01 + 18 * b01 + 128) >> 8)).clamp(16, 235) as u8;

                let r_avg = (r00 + r01 + r10 + r11 + 2) >> 2;
                let g_avg = (g00 + g01 + g10 + g11 + 2) >> 2;
                let b_avg = (b00 + b01 + b10 + b11 + 2) >> 2;

                uv[0] = (128 + ((-29 * r_avg - 99 * g_avg + 128 * b_avg + 128) >> 8)).clamp(16, 240) as u8;
                uv[1] = (128 + ((128 * r_avg - 116 * g_avg - 12 * b_avg + 128) >> 8)).clamp(16, 240) as u8;
            }
        }
    }
}

pub struct MediaCodecEncoder {
    codec: MediaCodec,
    surface: Option<NativeWindow>,
    w: u32,
    h: u32,
    stride: usize,
    slice_height: usize,
    rate: FrameRate,
    timescale: u32,
    duration: u32,
    bitrate_kbps: u32,
    key_interval_sec: i32,
    cbr: bool,
    nv12: Vec<u8>,
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
    eos_seen: bool,
    ready: Arc<Mutex<HashMap<u64, Nv12Frame>>>,
    in_flight: usize,
}

impl MediaCodecEncoder {
    pub fn new(w: u32, h: u32, rate: FrameRate, s: &ExportSettings) -> std::result::Result<Self, String> {
        let timescale = u32::try_from(rate.num).ok().filter(|n| *n > 0).ok_or("invalid frame rate numerator")?;
        let duration = u32::try_from(rate.den).ok().filter(|d| *d > 0).ok_or("invalid frame rate denominator")?;
        let fps = f64::from(timescale) / f64::from(duration);
        let auto_bitrate_kbps = {
            let pixels = (w as u64) * (h as u64);
            let target_fps = fps.max(1.0);
            let calculated = (pixels as f64 * target_fps * 0.15 / 1000.0) as u32;
            if pixels >= 3840 * 2160 {
                calculated.clamp(45_000, 55_000)
            } else if pixels >= 1920 * 1080 {
                calculated.clamp(16_000, 24_000)
            } else if pixels >= 1280 * 720 {
                calculated.clamp(8_000, 14_000)
            } else {
                calculated.clamp(4_000, 8_000)
            }
        };
        // Enforce high-fidelity bitrate floors to eliminate gradient banding on smooth surfaces
        // (45-55 Mbps for 4K 60fps, 16-24 Mbps for 1080p).
        let (min_kbps, max_kbps) = if (w as u64) * (h as u64) >= 3840 * 2160 {
            (45_000, 55_000)
        } else if (w as u64) * (h as u64) >= 1920 * 1080 {
            (16_000, 24_000)
        } else if (w as u64) * (h as u64) >= 1280 * 720 {
            (8_000, 14_000)
        } else {
            (4_000, 8_000)
        };
        let user_kbps = if s.bitrate_kbps >= 1000 {
            s.bitrate_kbps
        } else {
            auto_bitrate_kbps
        };
        let kbps = user_kbps.clamp(min_kbps, max_kbps);
        let key_interval_sec = s.keyframe_distance
            .filter(|k| *k > 0)
            .map(|k| (f64::from(k) / fps).round().max(1.0) as i32)
            .unwrap_or(2);
        let cbr = s.bitrate_mode == BitrateMode::Cbr;

        let make_format = |surface_mode: bool, with_operating_rate: bool, profile_opt: Option<(i32, i32)>| {
            let mut format = MediaFormat::new();
            format.set_str("mime", "video/avc");
            format.set_i32("width", w as i32);
            format.set_i32("height", h as i32);
            // COLOR_FormatSurface = 0x7F000789 (2130708361) or COLOR_FormatYUV420SemiPlanar = 21
            let color_fmt = if surface_mode { 0x7F000789 } else { 21 };
            format.set_i32("color-format", color_fmt);
            format.set_i32("bitrate", (kbps.saturating_mul(1000)) as i32);
            format.set_i32("max-bitrate", (max_kbps.saturating_mul(1000)) as i32);
            format.set_i32("frame-rate", fps.round().max(1.0) as i32);
            format.set_i32("i-frame-interval", key_interval_sec);
            format.set_i32("bitrate-mode", if cbr { 2 } else { 1 });
            if let Some((profile, level)) = profile_opt {
                format.set_i32("profile", profile);
                format.set_i32("level", level);
            }
            // Standard VUI color description for SDR video (BT.709, Full Range for GL Surface / Limited for NV12 buffer)
            // Surface rendering via OpenGL ES produces Full-Range [0, 255] RGB values.
            // Tagging with COLOR_RANGE_FULL (1) prevents video players from expanding
            // 16-235 to 0-255, eliminating gradient banding, crushed shadows, and clipped highlights.
            format.set_i32("color-standard", 1); // COLOR_STANDARD_BT709
            format.set_i32("color-range", if surface_mode { 1 } else { 2 });
            format.set_i32("color-transfer", 3); // COLOR_TRANSFER_SDR_VIDEO
            if with_operating_rate {
                format.set_i32("priority", 0);
                format.set_f32("operating-rate", 1000.0);
            }
            format
        };

        let try_init = |surface_mode: bool, with_operating_rate: bool, profile_opt: Option<(i32, i32)>| -> std::result::Result<(MediaCodec, Option<NativeWindow>), String> {
            let codec = MediaCodec::from_encoder_type("video/avc")
                .ok_or_else(|| "MediaCodec::from_encoder_type(\"video/avc\") returned None".to_string())?;

            if let Ok(name) = codec.name() {
                log::info!("MediaCodec encoder: {name} (surface_mode={surface_mode}, profile={profile_opt:?})");
                let lower = name.to_ascii_lowercase();
                if lower.starts_with("c2.android.") || lower.starts_with("omx.google.") || lower.contains(".sw.") {
                    return Err(format!("Rejecting software MediaCodec {name} (slower than native CPU encoder)"));
                }
            }

            let format = make_format(surface_mode, with_operating_rate, profile_opt);
            codec.configure(&format, None, MediaCodecDirection::Encoder)
                .map_err(|e| format!("AMediaCodec_configure failed: {e:?}"))?;

            let native_win = if surface_mode {
                let win = codec.create_input_surface()
                    .map_err(|e| format!("AMediaCodec_createInputSurface failed: {e:?}"))?;
                if let Err(e) = win.set_buffers_geometry(
                    w as i32,
                    h as i32,
                    Some(HardwareBufferFormat::R8G8B8A8_UNORM),
                ) {
                    log::warn!("win.set_buffers_geometry error: {e:?}");
                } else {
                    log::info!("win.set_buffers_geometry({w}x{h}, RGBA_8888) succeeded");
                }
                #[cfg(target_os = "android")]
                {
                    #[link(name = "android")]
                    unsafe extern "C" {
                        fn ANativeWindow_setBuffersDataSpace(window: *mut std::ffi::c_void, data_space: i32) -> i32;
                    }
                    unsafe {
                        // ADATASPACE_BT709 = 281083904 (STANDARD_BT709 | TRANSFER_SMPTE_170M | RANGE_LIMITED)
                        let res = ANativeWindow_setBuffersDataSpace(win.ptr().as_ptr() as *mut std::ffi::c_void, 281083904);
                        log::info!("ANativeWindow_setBuffersDataSpace(BT709=281083904) = {res}");
                    }
                }
                Some(win)
            } else {
                None
            };

            codec.start()
                .map_err(|e| format!("AMediaCodec_start failed: {e:?}"))?;
            Ok((codec, native_win))
        };

        let high_level = if (w as u64) * (h as u64) >= 3840 * 2160 {
            if fps > 30.0 { 0x10000 } else { 0x8000 } // Level 5.2 for 4K 60fps, Level 5.1 for <=30fps
        } else {
            0x2000 // AVCLevel42
        };

        // Try surface mode first with High Profile (AVCProfileHigh = 8, Level 5.2 for 4K 60fps),
        // falling back through profiles and to buffer mode.
        let (codec, surface) = try_init(true, true, Some((8, high_level)))
            .or_else(|e| {
                log::warn!("Surface mode High Profile Level 5.1/5.2 failed ({e}), trying Level 4.1");
                try_init(true, true, Some((8, 0x1000)))
            })
            .or_else(|e| {
                log::warn!("Surface mode High Profile failed ({e}), trying Main Profile");
                try_init(true, true, Some((2, 0x1000)))
            })
            .or_else(|e| {
                log::warn!("Surface mode Main Profile failed ({e}), trying unconstrained surface format");
                try_init(true, false, None)
            })
            .or_else(|e| {
                log::warn!("Surface mode completely failed ({e}), falling back to buffer mode High Profile");
                try_init(false, true, Some((8, high_level)))
            })
            .or_else(|e| {
                log::warn!("Buffer mode High Profile failed ({e}), trying basic buffer mode");
                try_init(false, false, None)
            })
            .map_err(|e| format!("Failed to initialize MediaCodec encoder: {e}"))?;

        let (stride, slice_height) = {
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| codec.input_format())) {
                Ok(infmt) => {
                    let s = infmt.i32("stride").filter(|&s| s > 0).map(|s| s as usize).unwrap_or(w as usize);
                    let sh = infmt.i32("slice-height").filter(|&h| h > 0).map(|h| h as usize).unwrap_or(h as usize);
                    log::info!("MediaCodec input format: width={w}, height={h}, stride={s}, slice-height={sh}");
                    (s, sh)
                }
                Err(_) => (w as usize, h as usize),
            }
        };

        Ok(Self {
            codec,
            surface,
            w,
            h,
            stride,
            slice_height,
            rate,
            timescale,
            duration,
            bitrate_kbps: kbps,
            key_interval_sec,
            cbr,
            nv12: Vec::new(),
            sps: None,
            pps: None,
            eos_seen: false,
            ready: Arc::new(Mutex::new(HashMap::new())),
            in_flight: 0,
        })
    }

    /// Reinitialize the hardware encoder in buffer mode (Phase A) if surface mode binding fails
    /// or if byte buffers need to be encoded.
    pub fn fallback_to_buffer(&mut self) -> bool {
        if self.surface.is_none() {
            return true;
        }
        log::warn!("Switching MediaCodec encoder from surface mode to buffer mode fallback");
        let _ = self.codec.stop();
        self.surface = None;
        let fps = f64::from(self.timescale) / f64::from(self.duration);
        let kbps = self.bitrate_kbps;
        let cbr = self.cbr;
        let key_interval_sec = self.key_interval_sec;
        let (w, h) = (self.w, self.h);
        let make_format = |with_operating_rate: bool, profile_opt: Option<(i32, i32)>| {
            let mut format = MediaFormat::new();
            format.set_str("mime", "video/avc");
            format.set_i32("width", w as i32);
            format.set_i32("height", h as i32);
            format.set_i32("color-format", 21); // COLOR_FormatYUV420SemiPlanar
            format.set_i32("bitrate", (kbps.saturating_mul(1000)) as i32);
            format.set_i32("frame-rate", fps.round().max(1.0) as i32);
            format.set_i32("i-frame-interval", key_interval_sec);
            format.set_i32("bitrate-mode", if cbr { 2 } else { 1 });
            if let Some((profile, level)) = profile_opt {
                format.set_i32("profile", profile);
                format.set_i32("level", level);
            }
            format.set_i32("color-standard", 1); // COLOR_STANDARD_BT709
            format.set_i32("color-range", 2);    // COLOR_RANGE_LIMITED
            format.set_i32("color-transfer", 3); // COLOR_TRANSFER_SDR_VIDEO
            if with_operating_rate {
                format.set_i32("priority", 0);
                format.set_f32("operating-rate", 1000.0);
            }
            format
        };
        let try_buffer = |with_op: bool, profile_opt: Option<(i32, i32)>| -> std::result::Result<MediaCodec, String> {
            let codec = MediaCodec::from_encoder_type("video/avc")
                .ok_or_else(|| "MediaCodec::from_encoder_type failed".to_string())?;
            let fmt = make_format(with_op, profile_opt);
            codec.configure(&fmt, None, MediaCodecDirection::Encoder)
                .map_err(|e| format!("configure failed: {e:?}"))?;
            codec.start()
                .map_err(|e| format!("start failed: {e:?}"))?;
            Ok(codec)
        };
        let high_level = if (w as u64) * (h as u64) >= 3840 * 2160 {
            if fps > 30.0 { 0x10000 } else { 0x8000 }
        } else {
            0x2000
        };
        let new_codec = match try_buffer(true, Some((8, high_level)))
            .or_else(|_| try_buffer(false, Some((8, high_level))))
            .or_else(|_| try_buffer(true, None))
            .or_else(|_| try_buffer(false, None)) {
            Ok(c) => c,
            Err(e) => {
                log::error!("MediaCodec failed to reinit in buffer mode: {e}");
                return false;
            }
        };
        if let Ok(infmt) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| new_codec.input_format())) {
            self.stride = infmt.i32("stride").filter(|&s| s > 0).map(|s| s as usize).unwrap_or(self.w as usize);
            self.slice_height = infmt.i32("slice-height").filter(|&h| h > 0).map(|h| h as usize).unwrap_or(self.h as usize);
        }
        self.codec = new_codec;
        self.in_flight = 0;
        log::info!("MediaCodec successfully fell back to buffer mode (stride={}, slice_height={})", self.stride, self.slice_height);
        true
    }

    fn fallback_sps(w: u32, h: u32) -> Vec<u8> {
        let _ = (w, h);
        vec![0x67, 0x64, 0x00, 0x28, 0xAC, 0x2B, 0x40, 0x3C, 0x01, 0x13, 0xF2, 0xCD, 0xC0, 0x40, 0x40, 0x50]
    }

    fn fallback_pps() -> Vec<u8> {
        vec![0x68, 0xEE, 0x3C, 0x80]
    }

    fn parse_param_set(data: &[u8], sps: &mut Option<Vec<u8>>, pps: &mut Option<Vec<u8>>) {
        let nals = annex_b_nals(data);
        if !nals.is_empty() {
            for nal in nals {
                match nal.first().map(|b| b & 0x1f) {
                    Some(7) => {
                        log::info!("Extracted SPS ({} bytes, profile {})", nal.len(), nal.get(1).copied().unwrap_or(0));
                        *sps = Some(nal.to_vec());
                    }
                    Some(8) => {
                        log::info!("Extracted PPS ({} bytes)", nal.len());
                        *pps = Some(nal.to_vec());
                    }
                    _ => {}
                }
            }
        } else if !data.is_empty() {
            match data.first().map(|b| b & 0x1f) {
                Some(7) => {
                    log::info!("Raw SPS ({} bytes, profile {})", data.len(), data.get(1).copied().unwrap_or(0));
                    *sps = Some(data.to_vec());
                }
                Some(8) => {
                    log::info!("Raw PPS ({} bytes)", data.len());
                    *pps = Some(data.to_vec());
                }
                _ => {}
            }
        }
    }

    fn drain_output(&mut self, initial_timeout: Duration, packets: &mut Vec<EncodedPacket>) -> Result<()> {
        let mut timeout = initial_timeout;
        loop {
            match self.codec.dequeue_output_buffer(timeout) {
                Ok(DequeuedOutputBufferInfoResult::Buffer(output_buffer)) => {
                    let info = *output_buffer.info();
                    let flags = info.flags();
                    let offset = info.offset().max(0) as usize;
                    let size = info.size().max(0) as usize;

                    if size > 0 {
                        let raw = output_buffer.buffer();
                        if let Some(slice) = raw.get(offset..offset.saturating_add(size)) {
                            if (flags & 2) != 0 {
                                // BUFFER_FLAG_CODEC_CONFIG = 2
                                Self::parse_param_set(slice, &mut self.sps, &mut self.pps);
                            } else {
                                if self.sps.is_none() || self.pps.is_none() {
                                    Self::parse_param_set(slice, &mut self.sps, &mut self.pps);
                                }
                                match annex_b_to_length_prefixed(slice) {
                                    Ok(data) => {
                                        let key = (flags & 1) != 0;
                                        packets.push(EncodedPacket {
                                            data,
                                            key,
                                            duration: self.duration,
                                            composition_offset: 0,
                                        });
                                        self.in_flight = self.in_flight.saturating_sub(1);
                                    }
                                    Err(e) => {
                                        log::debug!("skipping packet: {e}");
                                    }
                                }
                            }
                        }
                    }

                    let _ = self.codec.release_output_buffer(output_buffer, false);
                    if (flags & 4) != 0 {
                        // BUFFER_FLAG_END_OF_STREAM = 4
                        self.eos_seen = true;
                        break;
                    }
                    // For subsequent buffers in this drain, do NOT wait: only take what is already ready!
                    timeout = Duration::ZERO;
                }
                Ok(DequeuedOutputBufferInfoResult::OutputFormatChanged) => {
                    let fmt = self.codec.output_format();
                    if let Some(csd0) = fmt.buffer("csd-0") {
                        Self::parse_param_set(csd0, &mut self.sps, &mut self.pps);
                    }
                    if let Some(csd1) = fmt.buffer("csd-1") {
                        Self::parse_param_set(csd1, &mut self.sps, &mut self.pps);
                    }
                    timeout = Duration::ZERO;
                }
                Ok(DequeuedOutputBufferInfoResult::TryAgainLater)
                | Ok(DequeuedOutputBufferInfoResult::OutputBuffersChanged) => {
                    break;
                }
                Err(e) => {
                    return Err(ExportError::Encode(format!("dequeue_output_buffer error: {e:?}")));
                }
            }
        }
        Ok(())
    }
}

impl VideoEncoder for MediaCodecEncoder {
    fn input_surface(&self) -> Option<*mut std::ffi::c_void> {
        self.surface.as_ref().map(|w| w.ptr().as_ptr() as *mut std::ffi::c_void)
    }

    fn fallback_to_buffer(&mut self) -> bool {
        self.fallback_to_buffer()
    }

    fn sample_entry(&self) -> SampleEntry {
        let (w, h) = (self.w.min(u32::from(u16::MAX)) as u16, self.h.min(u32::from(u16::MAX)) as u16);
        let sps_list = match &self.sps {
            Some(s) if s.len() >= 4 => vec![s.clone()],
            _ => {
                log::warn!("MediaCodecEncoder::sample_entry: missing or incomplete SPS, using fallback");
                vec![Self::fallback_sps(self.w, self.h)]
            }
        };
        let pps_list = match &self.pps {
            Some(p) if !p.is_empty() => vec![p.clone()],
            _ => {
                log::warn!("MediaCodecEncoder::sample_entry: missing PPS, using fallback");
                vec![Self::fallback_pps()]
            }
        };
        let mut avc_config = AvcConfig::new(sps_list, pps_list, 4);
        if matches!(avc_config.profile, 100 | 110 | 122 | 144) && avc_config.ext.is_empty() {
            // ISO/IEC 14496-15 §5.3.3.1.2: High profile requires chroma_format (4:2:0 -> 1),
            // bit_depth_luma_minus8 (0), bit_depth_chroma_minus8 (0), numOfSequenceParameterSetExt (0).
            avc_config.ext = vec![0xFD, 0xF8, 0xF8, 0x00];
        }
        SampleEntry::avc(avc_config, w, h)
    }

    fn timescale(&self) -> u32 {
        self.timescale
    }

    fn prepare_ahead(&self) -> Option<PrepareAhead> {
        if self.surface.is_some() {
            // In zero-copy surface mode, GPU renders directly into the hardware surface.
            return None;
        }
        let ready = self.ready.clone();
        Some(Arc::new(move |f: &EncoderFrame| {
            let (w, h) = (f.width as usize, f.height as usize);
            let mut nv12 = Vec::new();
            filmcraft_export::timed(filmcraft_export::Stage::Convert, || {
                rgba_to_nv12(f.rgba, w, h, &mut nv12);
            });
            ready.lock().unwrap_or_else(PoisonError::into_inner).insert(f.index, nv12);
        }))
    }

    fn encode(&mut self, f: &EncoderFrame) -> Result<Vec<EncodedPacket>> {
        // Zero-copy surface mode: frame was rendered directly to the encoder's InputSurface on the GPU
        if self.surface.is_some() && f.rgba.is_empty() {
            let mut packets = Vec::new();
            self.in_flight += 1;

            // Drain any pending output packets immediately without waiting
            self.drain_output(Duration::ZERO, &mut packets)?;

            // If the encoder has 4 or more frames in-flight (Qualcomm hardware limit is 8 buffer slots),
            // wait for the hardware encoder to finish compressing earlier frames to prevent buffer overflow.
            while self.in_flight >= 4 {
                let prev = packets.len();
                self.drain_output(Duration::from_millis(15), &mut packets)?;
                if packets.len() == prev {
                    // Codec took >15ms and produced no packet, break to avoid blocking forever
                    break;
                }
            }

            if self.sps.is_none() || self.pps.is_none() {
                for _ in 0..100 {
                    if self.sps.is_some() && self.pps.is_some() {
                        break;
                    }
                    self.drain_output(Duration::from_millis(20), &mut packets)?;
                }
            }

            filmcraft_export::note_hw_encode_frame();
            return Ok(packets);
        }

        if self.surface.is_some() {
            if !self.fallback_to_buffer() {
                return Err(ExportError::Encode("Hardware encoder configured for surface rendering cannot accept CPU byte buffers".into()));
            }
        }

        if f.hdr.is_some() {
            return Err(ExportError::Unsupported("hardware encoding does not take HDR pictures".into()));
        }
        if (f.width, f.height) != (self.w, self.h) {
            return Err(ExportError::Encode(format!(
                "picture is {}x{}, the encoder was created for {}x{}",
                f.width, f.height, self.w, self.h
            )));
        }
        let (w, h) = (f.width as usize, f.height as usize);
        if f.rgba.len() < w.saturating_mul(h).saturating_mul(4) {
            return Err(ExportError::Encode("the RGBA picture is smaller than its size".into()));
        }

        // 1. Color conversion: retrieve pre-converted NV12 frame or convert if missing
        let ready_frame = {
            let mut ready = self.ready.lock().unwrap_or_else(PoisonError::into_inner);
            let mine = ready.remove(&f.index);
            ready.retain(|&i, _| i > f.index);
            mine
        };
        let expected_len = w * h + (w.div_ceil(2) * h.div_ceil(2) * 2);
        match ready_frame.filter(|buf| buf.len() == expected_len) {
            Some(buf) => {
                self.nv12 = buf;
            }
            None => {
                filmcraft_export::timed(filmcraft_export::Stage::Convert, || {
                    rgba_to_nv12(f.rgba, w, h, &mut self.nv12);
                });
            }
        }

        let mut packets = Vec::new();

        // 2. Drain any pending output buffers immediately without waiting
        self.drain_output(Duration::ZERO, &mut packets)?;

        // 3. Dequeue an input buffer: try non-blocking first.
        // If the encoder's input queue is full, drain output buffers in a loop with a generous timeout.
        let mut input_buf = 'outer: loop {
            for _ in 0..100 {
                match self.codec.dequeue_input_buffer(Duration::ZERO) {
                    Ok(DequeuedInputBufferResult::Buffer(b)) => break 'outer b,
                    _ => {
                        self.drain_output(Duration::from_millis(50), &mut packets)?;
                    }
                }
            }
            return Err(ExportError::Encode("MediaCodec input buffer dequeue timed out: hardware encoder did not accept new frame within 5s".into()));
        };

        // 4. Format directly into NV12 inside the hardware input buffer
        let stride = self.stride.max(w);
        let slice_height = self.slice_height.max(h);
        let y_size = stride.saturating_mul(slice_height);
        let uv_height = h.div_ceil(2);
        let uv_size = stride.saturating_mul(uv_height);
        let total_needed = y_size.saturating_add(uv_size);

        let dst = input_buf.buffer_mut();
        if dst.len() < total_needed {
            return Err(ExportError::Encode(format!(
                "MediaCodec input buffer too small: {} bytes, expected at least {}",
                dst.len(),
                total_needed
            )));
        }

        if stride == w && slice_height == h {
            // Fast contiguous copy for tightly packed buffers (common case)
            if dst.len() >= total_needed && self.nv12.len() >= total_needed {
                // SAFETY: dst is valid for total_needed bytes, non-null, and does not overlap with self.nv12.
                unsafe {
                    std::ptr::copy_nonoverlapping(self.nv12.as_ptr(), dst.as_mut_ptr() as *mut u8, total_needed);
                }
            }
        } else {
            // Strided copy for chips with alignment requirements (e.g. 16/32-byte alignment)
            let y_plane = &self.nv12[..w * h];
            let uv_plane = &self.nv12[w * h..];
            for row in 0..h {
                let src_offset = row * w;
                let dst_offset = row * stride;
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        y_plane.as_ptr().add(src_offset),
                        dst.as_mut_ptr().add(dst_offset) as *mut u8,
                        w,
                    );
                }
            }
            let uv_base = y_size;
            let uv_w = w.div_ceil(2) * 2;
            for row in 0..uv_height {
                let src_offset = row * uv_w;
                let dst_row = uv_base + row * stride;
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        uv_plane.as_ptr().add(src_offset),
                        dst.as_mut_ptr().add(dst_row) as *mut u8,
                        uv_w,
                    );
                }
            }
        }

        // 5. Calculate PTS in microseconds
        let pts_us = (f.index as u128)
            .saturating_mul(1_000_000)
            .saturating_mul(self.rate.den as u128)
            .checked_div(self.rate.num as u128)
            .unwrap_or(0) as u64;

        self.codec
            .queue_input_buffer(input_buf, 0, total_needed, pts_us, 0)
            .map_err(|e| ExportError::Encode(format!("queue_input_buffer failed: {e:?}")))?;

        // 6. Drain any output buffers that finished (non-blocking)
        self.drain_output(Duration::ZERO, &mut packets)?;

        // 7. If this is the start of encoding and SPS/PPS are not yet known, wait for codec config
        if self.sps.is_none() || self.pps.is_none() {
            for _ in 0..40 { // up to 2 seconds
                if self.sps.is_some() && self.pps.is_some() {
                    break;
                }
                self.drain_output(Duration::from_millis(50), &mut packets)?;
            }
        }

        filmcraft_export::note_hw_encode_frame();

        Ok(packets)
    }

    fn flush(&mut self) -> Result<Vec<EncodedPacket>> {
        let mut packets = Vec::new();

        if self.surface.is_some() {
            // Surface mode: signal EOS via AMediaCodec_signalEndOfInputStream
            if let Err(e) = self.codec.set_signal_end_of_input_stream() {
                log::warn!("AMediaCodec_signalEndOfInputStream failed: {e:?}");
            }
            for _ in 0..200 {
                if self.eos_seen {
                    break;
                }
                self.drain_output(Duration::from_millis(50), &mut packets)?;
            }
            self.in_flight = 0;
            let _ = self.codec.stop();
            return Ok(packets);
        }

        // Buffer mode: drain and queue EOS buffer
        self.drain_output(Duration::from_millis(50), &mut packets)?;

        // 2. Queue EOS buffer (size = 0, flag = 4 = BUFFER_FLAG_END_OF_STREAM)
        // Wait in a retry loop to ensure an input buffer becomes available for EOS
        let mut eos_queued = false;
        for _ in 0..50 {
            match self.codec.dequeue_input_buffer(Duration::from_millis(20)) {
                Ok(DequeuedInputBufferResult::Buffer(b)) => {
                    if self.codec.queue_input_buffer(b, 0, 0, 0, 4).is_ok() {
                        eos_queued = true;
                        break;
                    }
                }
                _ => {
                    self.drain_output(Duration::from_millis(20), &mut packets)?;
                }
            }
        }

        if !eos_queued {
            log::warn!("Could not queue EOS to AMediaCodec input buffer");
        }

        // 3. Drain until EOS seen or timeout loop ends
        for _ in 0..100 {
            if self.eos_seen {
                break;
            }
            self.drain_output(Duration::from_millis(50), &mut packets)?;
        }

        let _ = self.codec.stop();

        Ok(packets)
    }
}

// SAFETY: Android AMediaCodec handle can be safely sent to another thread; MediaCodecEncoder owns the codec and accesses it exclusively via &mut self.
unsafe impl Send for MediaCodecEncoder {}

/// Convert AVCC / HVCC length-prefixed NAL units to Annex B start codes (`00 00 00 01`).
pub fn avcc_to_annex_b(sample: &[u8], length_size: usize, out: &mut Vec<u8>) {
    out.clear();
    avcc_to_annex_b_append(sample, length_size, out);
}

/// Append AVCC / HVCC length-prefixed NAL units as Annex B start codes (`00 00 00 01`) into `out`.
pub fn avcc_to_annex_b_append(sample: &[u8], length_size: usize, out: &mut Vec<u8>) {
    let mut pos = 0usize;
    while pos + length_size <= sample.len() {
        let len = match length_size {
            1 => sample[pos] as usize,
            2 => u16::from_be_bytes([sample[pos], sample[pos + 1]]) as usize,
            4 => u32::from_be_bytes([sample[pos], sample[pos + 1], sample[pos + 2], sample[pos + 3]]) as usize,
            _ => {
                let mut acc = 0usize;
                for &b in &sample[pos..pos + length_size] {
                    acc = (acc << 8) | (b as usize);
                }
                acc
            }
        };
        pos += length_size;
        if len == 0 || pos + len > sample.len() {
            break;
        }
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(&sample[pos..pos + len]);
        pos += len;
    }
}

/// The hardware decoder factory registered in FilmCraft's codec registry.
pub fn decoder_factory(
    entry: &SampleEntry,
) -> Option<filmcraft_codecs::Result<Box<dyn VideoDecoder>>> {
    log::info!("MediaCodec decoder_factory evaluating codec: {}", entry.codec.name());
    if !filmcraft_codecs::hw::hardware_decoding() {
        log::info!("MediaCodec hardware decoding skipped: disabled in settings");
        return None;
    }
    if filmcraft_media::cancel::background() {
        log::info!("MediaCodec hardware decoding skipped: background task (thumbnail), using software decoder");
        return None;
    }
    let info = match NalStreamInfo::from_entry(entry) {
        Some(Ok(info)) => info,
        Some(Err(why)) => {
            log::warn!("MediaCodec decoder_factory: NalStreamInfo parse error for {}: {why}", entry.codec.name());
            return None;
        }
        None => {
            log::info!("MediaCodec decoder_factory: stream is not NAL-based (codec {})", entry.codec.name());
            return None;
        }
    };
    if !decoder_for(&info) {
        return None;
    }
    if !try_acquire_hw_decoder() {
        log::info!(
            "MediaCodec hardware decoding skipped: max concurrent decoders ({MAX_CONCURRENT_HW_DECODERS}) reached, falling back to software"
        );
        return None;
    }
    match MediaCodecDecoder::new(info.clone()) {
        Ok(dec) => {
            log::info!(
                "MediaCodec hardware decoder activated for {} video ({}) [active hw decoders: {}]",
                entry.codec.name(),
                dec.name(),
                ACTIVE_HW_DECODERS.load(Ordering::Relaxed)
            );
            Some(Ok(Box::new(HybridDecoder::new(
                Box::new(dec),
                entry.clone(),
                info,
            ))))
        }
        Err(why) => {
            release_hw_decoder();
            log::warn!("MediaCodec hardware decoding declined for {} video: {why}", entry.codec.name());
            filmcraft_codecs::hw::note_hw_declined();
            None
        }
    }
}

/// Whether MediaCodec decoder supports this stream configuration.
pub fn decoder_for(info: &NalStreamInfo) -> bool {
    if info.interlaced || info.bit_depth_luma != 8 || info.chroma_format_idc != 1 {
        return false;
    }
    matches!(info.codec, NalCodec::H264 | NalCodec::Hevc)
}

/// Android `AMediaCodec` hardware H.264 / HEVC video decoder.
pub struct MediaCodecDecoder {
    codec: MediaCodec,
    info: NalStreamInfo,
    name: String,
    crop: (u32, u32, u32, u32),
    stride: usize,
    slice_height: usize,
    color_format: i32,
    fed: u64,
    eos_seen: bool,
    need_param_sets: bool,
    input_annex_b: Vec<u8>,
    is_direct_surface: bool,
    last_render_pts_ns: i64,
}

impl MediaCodecDecoder {
    pub fn new(info: NalStreamInfo) -> std::result::Result<Self, String> {
        if info.interlaced {
            return Err("hardware decoding declined: interlaced stream".into());
        }
        if info.bit_depth_luma != 8 || info.chroma_format_idc != 1 {
            return Err(format!(
                "hardware decoding declined: unsupported bit depth ({}) or chroma format ({})",
                info.bit_depth_luma, info.chroma_format_idc
            ));
        }

        let (mime, default_name) = match info.codec {
            NalCodec::H264 => ("video/avc", "MediaCodec H.264"),
            NalCodec::Hevc => ("video/hevc", "MediaCodec HEVC"),
        };

        let (w, h) = info.coded;

        let direct_window = filmcraft_codecs::hw::direct_surface_for(w, h).and_then(|ptr| {
            let non_null = std::ptr::NonNull::new(ptr as *mut _)?;
            Some(unsafe { NativeWindow::clone_from_ptr(non_null) })
        });
        let has_direct = direct_window.is_some();

        let make_format = |with_operating_rate: bool, with_color_format: bool, with_csd: bool| {
            let mut format = MediaFormat::new();
            format.set_str("mime", mime);
            format.set_i32("width", w as i32);
            format.set_i32("height", h as i32);
            if with_color_format {
                // COLOR_FormatYUV420SemiPlanar (NV12) = 21
                format.set_i32("color-format", 21);
            }
            if with_operating_rate {
                format.set_i32("priority", 0);
                format.set_f32("operating-rate", 1000.0);
            }

            if with_csd {
                match info.codec {
                    NalCodec::H264 => {
                        let mut csd0 = Vec::new();
                        let mut csd1 = Vec::new();
                        for nal in &info.parameter_sets {
                            if let Some(&hdr) = nal.first() {
                                let nal_type = hdr & 0x1f;
                                if nal_type == 7 {
                                    csd0.extend_from_slice(&[0, 0, 0, 1]);
                                    csd0.extend_from_slice(nal);
                                } else if nal_type == 8 {
                                    csd1.extend_from_slice(&[0, 0, 0, 1]);
                                    csd1.extend_from_slice(nal);
                                }
                            }
                        }
                        if !csd0.is_empty() {
                            format.set_buffer("csd-0", &csd0);
                        }
                        if !csd1.is_empty() {
                            format.set_buffer("csd-1", &csd1);
                        }
                    }
                    NalCodec::Hevc => {
                        let mut csd0 = Vec::new();
                        for nal in &info.parameter_sets {
                            csd0.extend_from_slice(&[0, 0, 0, 1]);
                            csd0.extend_from_slice(nal);
                        }
                        if !csd0.is_empty() {
                            format.set_buffer("csd-0", &csd0);
                        }
                    }
                }
            }

            format
        };

        let try_init = |with_operating_rate: bool, with_color_format: bool, with_csd: bool, surface: Option<&NativeWindow>| -> std::result::Result<MediaCodec, String> {
            let codec = MediaCodec::from_decoder_type(mime)
                .ok_or_else(|| format!("MediaCodec::from_decoder_type(\"{mime}\") returned None"))?;

            if let Ok(name) = codec.name() {
                log::info!("MediaCodec decoder candidate: {name}");
                let lower = name.to_ascii_lowercase();
                if lower.starts_with("c2.android.") || lower.starts_with("omx.google.") || lower.contains(".sw.") {
                    return Err(format!("Rejecting software MediaCodec {name} (slower than native CPU decoder)"));
                }
            }

            let format = make_format(with_operating_rate, with_color_format, with_csd);
            codec.configure(&format, surface, MediaCodecDirection::Decoder)
                .map_err(|e| format!("AMediaCodec_configure decoder failed: {e:?}"))?;
            codec.start()
                .map_err(|e| format!("AMediaCodec_start decoder failed: {e:?}"))?;
            Ok(codec)
        };

        let (codec, is_direct_surface) = if has_direct {
            log::info!("Attempting to initialize MediaCodec decoder with direct surface bridge...");
            match try_init(true, false, true, direct_window.as_ref()) {
                Ok(c) => {
                    log::info!("MediaCodec decoder initialized with direct hardware surface bridge!");
                    (c, true)
                }
                Err(e) => {
                    log::warn!("Direct surface decoder init failed ({e}), falling back to buffer mode");
                    filmcraft_codecs::hw::clear_direct_surface();
                    let c = try_init(true, true, true, None)
                        .or_else(|e1| {
                            log::warn!("MediaCodec decoder fast init failed ({e1}), retrying minimal with color-format");
                            try_init(false, true, true, None)
                        })
                        .or_else(|e2| {
                            log::warn!("MediaCodec decoder color-format init failed ({e2}), retrying with default format");
                            try_init(false, false, true, None)
                        })
                        .or_else(|e3| {
                            log::warn!("MediaCodec decoder csd init failed ({e3}), retrying with in-band parameter sets");
                            try_init(false, false, false, None)
                        })
                        .map_err(|e4| format!("Failed to initialize MediaCodec decoder: {e4}"))?;
                    (c, false)
                }
            }
        } else {
            let c = try_init(true, true, true, None)
                .or_else(|e1| {
                    log::warn!("MediaCodec decoder fast init failed ({e1}), retrying minimal with color-format");
                    try_init(false, true, true, None)
                })
                .or_else(|e2| {
                    log::warn!("MediaCodec decoder color-format init failed ({e2}), retrying with default format");
                    try_init(false, false, true, None)
                })
                .or_else(|e3| {
                    log::warn!("MediaCodec decoder csd init failed ({e3}), retrying with in-band parameter sets");
                    try_init(false, false, false, None)
                })
                .map_err(|e4| format!("Failed to initialize MediaCodec decoder: {e4}"))?;
            (c, false)
        };

        let name = codec.name().unwrap_or_else(|_| default_name.to_string());
        log::info!("MediaCodec decoder initialized successfully: {name} (direct_surface={is_direct_surface})");
        let stride = w as usize;
        let slice_height = h as usize;
        let crop = info.crop;

        let mut dec = Self {
            codec,
            info,
            name,
            crop,
            stride,
            slice_height,
            color_format: 21,
            fed: 0,
            eos_seen: false,
            need_param_sets: true,
            input_annex_b: Vec::new(),
            is_direct_surface,
            last_render_pts_ns: 0,
        };

        let out_fmt = dec.codec.output_format();
        dec.update_format(&out_fmt);

        Ok(dec)
    }

    fn update_format(&mut self, fmt: &MediaFormat) {
        if let Some(cf) = fmt.i32("color-format") {
            self.color_format = cf;
        }
        if let Some(s) = fmt.i32("stride").filter(|&s| s > 0) {
            self.stride = s as usize;
        } else if let Some(w) = fmt.i32("width").filter(|&w| w > 0) {
            self.stride = self.stride.max(w as usize);
        }
        if let Some(sh) = fmt.i32("slice-height").filter(|&h| h > 0) {
            self.slice_height = sh as usize;
        } else if let Some(h) = fmt.i32("height").filter(|&h| h > 0) {
            self.slice_height = self.slice_height.max(h as usize);
        }

        if let (Some(l), Some(t), Some(r), Some(b)) = (
            fmt.i32("crop-left"),
            fmt.i32("crop-top"),
            fmt.i32("crop-right"),
            fmt.i32("crop-bottom"),
        ) {
            if r >= l && b >= t {
                let cw = (r - l + 1) as u32;
                let ch = (b - t + 1) as u32;
                if cw > 0 && ch > 0 {
                    self.crop = (l as u32, t as u32, cw, ch);
                }
            }
        }
        log::info!(
            "MediaCodecDecoder format updated: color_format={}, stride={}, slice_height={}, crop={:?}",
            self.color_format,
            self.stride,
            self.slice_height,
            self.crop
        );
    }

    fn drain_output(&mut self, timeout: Duration, frames: &mut Vec<DecodedFrame>) -> filmcraft_codecs::Result<()> {
        self.drain_output_inner(timeout, frames, true)
    }

    fn drain_output_inner(&mut self, mut timeout: Duration, frames: &mut Vec<DecodedFrame>, render_surface: bool) -> filmcraft_codecs::Result<()> {
        for _ in 0..100 {
            match self.codec.dequeue_output_buffer(timeout) {
                Ok(DequeuedOutputBufferInfoResult::Buffer(output_buffer)) => {
                    let info = output_buffer.info();
                    let flags = info.flags();
                    let pts = info.presentation_time_us();

                    if (flags & 2) == 0 {
                        if self.is_direct_surface {
                            // ZERO-COPY DIRECT SURFACE BRIDGE:
                            // Commit the decoded frame directly to the encoder's input surface if render_surface is true.
                            // If render_surface is false (e.g. during flush), discard without rendering to surface.
                            if let Err(e) = self.codec.release_output_buffer(output_buffer, render_surface) {
                                log::warn!("MediaCodecDecoder release_output_buffer failed: {e:?}");
                            }

                            frames.push(DecodedFrame {
                                pts,
                                frame: filmcraft_frame::VideoFrame {
                                    width: self.crop.2,
                                    height: self.crop.3,
                                    data: filmcraft_frame::PixelData::Rgba8(Arc::new(Vec::new())),
                                    color: self.info.color,
                                    par: self.info.par,
                                    pts: filmcraft_time::Tick::ZERO,
                                },
                                draft: false,
                            });

                            if (flags & 4) != 0 {
                                self.eos_seen = true;
                                break;
                            }
                            timeout = Duration::ZERO;
                            continue;
                        }

                        if info.size() > 0 {
                        let buf_data = output_buffer.buffer();
                        let offset = info.offset() as usize;
                        let size = info.size() as usize;
                        if let Some(slice) = buf_data.get(offset..offset.saturating_add(size)) {
                            let stride = self.stride.max(self.info.coded.0 as usize);
                            let slice_height = self.slice_height.max(self.info.coded.1 as usize);

                            if self.color_format == 19 {
                                // COLOR_FormatYUV420Planar (I420)
                                let y_size = stride * slice_height;
                                let uv_stride = stride / 2;
                                let uv_rows = slice_height / 2;
                                let u_size = uv_stride * uv_rows;
                                if slice.len() >= y_size + u_size * 2 {
                                    let (ox, oy, cw, ch) = (self.crop.0 as usize, self.crop.1 as usize, self.crop.2 as usize, self.crop.3 as usize);
                                    let mut y_plane = filmcraft_frame::pool::take_u8(cw * ch);
                                    for r in 0..ch {
                                        let src_start = (oy + r) * stride + ox;
                                        if let Some(row) = slice.get(src_start..src_start + cw) {
                                            y_plane.extend_from_slice(row);
                                        }
                                    }
                                    let u_start = y_size;
                                    let v_start = y_size + u_size;
                                    let (cox, coy, cuw, cuh) = (ox / 2, oy / 2, cw.div_ceil(2), ch.div_ceil(2));
                                    let mut u_plane = filmcraft_frame::pool::take_u8(cuw * cuh);
                                    let mut v_plane = filmcraft_frame::pool::take_u8(cuw * cuh);
                                    for r in 0..cuh {
                                        let u_row = u_start + (coy + r) * uv_stride + cox;
                                        let v_row = v_start + (coy + r) * uv_stride + cox;
                                        if let Some(urow) = slice.get(u_row..u_row + cuw) {
                                            u_plane.extend_from_slice(urow);
                                        }
                                        if let Some(vrow) = slice.get(v_row..v_row + cuw) {
                                            v_plane.extend_from_slice(vrow);
                                        }
                                    }
                                    let data = filmcraft_frame::PixelData::Yuv8 {
                                        planes: [Arc::new(y_plane), Arc::new(u_plane), Arc::new(v_plane)],
                                        chroma: filmcraft_frame::Chroma::C420,
                                        alpha: None,
                                    };
                                    frames.push(DecodedFrame {
                                        pts,
                                        frame: filmcraft_frame::VideoFrame {
                                            width: self.crop.2,
                                            height: self.crop.3,
                                            data,
                                            color: self.info.color,
                                            par: self.info.par,
                                            pts: filmcraft_time::Tick::ZERO,
                                        },
                                        draft: false,
                                    });
                                }
                            } else {
                                // Default: COLOR_FormatYUV420SemiPlanar (NV12)
                                let luma_size = stride.saturating_mul(slice_height);
                                let total_size = luma_size.saturating_add(stride.saturating_mul(slice_height.div_ceil(2)));

                                let (luma, chroma, rows) = if slice.len() >= total_size {
                                    (&slice[..luma_size], &slice[luma_size..], slice_height)
                                } else {
                                    let actual_rows = if stride > 0 {
                                        (slice.len() * 2) / (stride * 3)
                                    } else {
                                        self.info.coded.1 as usize
                                    };
                                    let luma_bytes = stride.saturating_mul(actual_rows);
                                    if slice.len() >= luma_bytes {
                                        (&slice[..luma_bytes], &slice[luma_bytes..], actual_rows)
                                    } else {
                                        let half = slice.len() * 2 / 3;
                                        (&slice[..half], &slice[half..], actual_rows)
                                    }
                                };

                                let biplanar = biplanar::Biplanar {
                                    luma,
                                    chroma,
                                    stride,
                                    rows,
                                };
                                let geometry = biplanar::Geometry {
                                    crop: self.crop,
                                    bits: 8,
                                    color: self.info.color,
                                    par: self.info.par,
                                };

                                match biplanar::to_frame(&biplanar, &geometry) {
                                    Ok(frame) => {
                                        frames.push(DecodedFrame {
                                            pts,
                                            frame,
                                            draft: false,
                                        });
                                    }
                                    Err(e) => {
                                        log::warn!("MediaCodecDecoder biplanar::to_frame failed: {e}");
                                    }
                                }
                            }
                        }
                        }
                    }

                    let _ = self.codec.release_output_buffer(output_buffer, false);
                    if (flags & 4) != 0 {
                        self.eos_seen = true;
                        break;
                    }
                    timeout = Duration::ZERO;
                }
                Ok(DequeuedOutputBufferInfoResult::OutputFormatChanged) => {
                    let fmt = self.codec.output_format();
                    self.update_format(&fmt);
                }
                Ok(DequeuedOutputBufferInfoResult::TryAgainLater)
                | Ok(DequeuedOutputBufferInfoResult::OutputBuffersChanged) => {
                    break;
                }
                Err(e) => {
                    return Err(CodecError::Decode(format!("dequeue_output_buffer error: {e:?}")));
                }
            }
        }
        Ok(())
    }
}

impl VideoDecoder for MediaCodecDecoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> filmcraft_codecs::Result<Vec<DecodedFrame>> {
        self.fed += 1;
        let mut frames = Vec::new();

        // 1. Convert AVCC / HVCC sample to Annex B, prepending parameter sets on first frame or seek
        self.input_annex_b.clear();
        if self.need_param_sets {
            for nal in &self.info.parameter_sets {
                self.input_annex_b.extend_from_slice(&[0, 0, 0, 1]);
                self.input_annex_b.extend_from_slice(nal);
            }
            self.need_param_sets = false;
        }
        avcc_to_annex_b_append(sample, self.info.length_size, &mut self.input_annex_b);
        if self.input_annex_b.is_empty() {
            return Ok(frames);
        }

        // 2. Dequeue input buffer with zero-latency fast-path & backoff
        let mut input_buf = 'outer: loop {
            if let Ok(DequeuedInputBufferResult::Buffer(b)) = self.codec.dequeue_input_buffer(Duration::ZERO) {
                break 'outer b;
            }
            for _ in 0..300 {
                match self.codec.dequeue_input_buffer(Duration::from_millis(2)) {
                    Ok(DequeuedInputBufferResult::Buffer(b)) => break 'outer b,
                    _ => {
                        self.drain_output(Duration::from_millis(10), &mut frames)?;
                    }
                }
            }
            return Err(CodecError::Decode(
                "MediaCodec input buffer dequeue timed out: hardware decoder did not accept new sample within 3s".into(),
            ));
        };

        // 3. Copy Annex-B data into input buffer
        let total_bytes = self.input_annex_b.len();
        let dst = input_buf.buffer_mut();
        if total_bytes > dst.len() {
            return Err(CodecError::Decode(format!(
                "input sample ({} bytes) exceeds MediaCodec input buffer capacity ({} bytes)",
                total_bytes,
                dst.len()
            )));
        }
        // SAFETY: dst is valid for total_bytes bytes, non-null, and does not overlap with self.input_annex_b.
        unsafe {
            std::ptr::copy_nonoverlapping(
                self.input_annex_b.as_ptr(),
                dst.as_mut_ptr() as *mut u8,
                total_bytes,
            );
        }

        // 4. Queue input buffer with pts
        self.codec
            .queue_input_buffer(input_buf, 0, total_bytes, pts as u64, 0)
            .map_err(|e| CodecError::Decode(format!("queue_input_buffer failed: {e:?}")))?;

        // 5. Drain ready output frames
        self.drain_output(Duration::ZERO, &mut frames)?;
        if frames.is_empty() {
            // Poll hardware decoder in 10ms slices up to 200ms so 4K frames never time out
            for _ in 0..20 {
                self.drain_output(Duration::from_millis(10), &mut frames)?;
                if !frames.is_empty() {
                    break;
                }
            }
        }

        Ok(frames)
    }

    fn flush(&mut self) -> Vec<DecodedFrame> {
        if self.is_direct_surface {
            drain_surface_scaler_texture();
        }
        let mut frames = Vec::new();

        // 1. Drain pending frames first without rendering to surface so SurfaceTexture is never clogged
        let _ = self.drain_output_inner(Duration::from_millis(20), &mut frames, false);

        if self.is_direct_surface {
            // Direct surface mode (export): queue EOS buffer (flags = 4) to signal EOF to surface/encoder
            let mut eos_queued = false;
            for _ in 0..50 {
                match self.codec.dequeue_input_buffer(Duration::from_millis(10)) {
                    Ok(DequeuedInputBufferResult::Buffer(b)) => {
                        if self.codec.queue_input_buffer(b, 0, 0, 0, 4).is_ok() {
                            eos_queued = true;
                            break;
                        }
                    }
                    _ => {
                        let _ = self.drain_output_inner(Duration::from_millis(10), &mut frames, false);
                    }
                }
            }

            if !eos_queued {
                log::warn!("Could not queue EOS to AMediaCodec decoder input buffer");
            }

            // Drain remaining buffers until EOS seen without rendering to surface
            for _ in 0..50 {
                if self.eos_seen {
                    break;
                }
                if self.drain_output_inner(Duration::from_millis(20), &mut frames, false).is_err() {
                    break;
                }
            }

            drain_surface_scaler_texture();
        } else {
            // Buffer mode (preview): drain pending in-flight frames without queuing EOS
            for _ in 0..15 {
                let count_before = frames.len();
                if self.drain_output_inner(Duration::from_millis(15), &mut frames, false).is_err() {
                    break;
                }
                if frames.len() == count_before {
                    break;
                }
            }
        }

        self.eos_seen = false;
        self.last_render_pts_ns = 0;
        frames
    }

    fn reset(&mut self) {
        if self.is_direct_surface {
            drain_surface_scaler_texture();
        }
        if let Err(e) = self.codec.flush() {
            log::warn!("MediaCodecDecoder reset flush failed: {e:?}");
        }
        if self.is_direct_surface {
            drain_surface_scaler_texture();
        }
        self.eos_seen = false;
        self.need_param_sets = true;
        self.last_render_pts_ns = 0;
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn is_random_access(&self, sample: &[u8]) -> Option<bool> {
        self.info.is_random_access(sample)
    }

    fn is_disposable(&self, sample: &[u8]) -> bool {
        self.info.is_disposable(sample)
    }
}

#[cfg(target_os = "android")]
impl Drop for MediaCodecDecoder {
    fn drop(&mut self) {
        if self.is_direct_surface {
            drain_surface_scaler_texture();
        }
        let _ = self.codec.stop();
        release_hw_decoder();
        log::info!(
            "MediaCodec hardware decoder released (remaining active: {})",
            ACTIVE_HW_DECODERS.load(Ordering::Relaxed)
        );
    }
}

#[cfg(not(target_os = "android"))]
impl Drop for MediaCodecDecoder {
    fn drop(&mut self) {
        release_hw_decoder();
    }
}

// SAFETY: Android AMediaCodec handle can be safely sent to another thread; MediaCodecDecoder owns the codec and accesses it exclusively via &mut self.
unsafe impl Send for MediaCodecDecoder {}

// ============================================================================
// Phase B3: Zero-Copy Hardware Surface Scaler Bridge
// Enables 30-second export for downscaled resolutions (4K -> 1080p, 720p)
// by blitting directly from ASurfaceTexture to encoder surface via Adreno 650.
// ============================================================================

#[cfg(target_os = "android")]
static ACTIVE_SCALER: Mutex<Option<HardwareSurfaceScaler>> = Mutex::new(None);

#[cfg(target_os = "android")]
const DEFAULT_LUMETRI: [f32; 26] = [
    1.0, 1.0, 1.0, 0.0, // u_p0: ge.r, ge.g, ge.b, b0
    1.0, 0.0, 0.0, 0.0, // u_p1: w0, sh_k, hl_k, contrast
    0.0, 1.0, 0.0, 0.0, // u_p2: faded, sat, vib, va
    0.0, 0.0, 1.0, 0.0, // u_p3: vmid, vfeather, vround_aspect, 0.0
    0.0, 0.0, 0.0, 0.0, // u_p4: st_k.r, st_k.g, st_k.b, 0.0
    0.0, 0.0, 0.0, 0.0, // u_p5: ht_k.r, ht_k.g, ht_k.b, 0.0
    0.0, 0.0,           // u_i1: creative_on, vignette_on
];

#[cfg(target_os = "android")]
pub fn register_scaler() {
    filmcraft_codecs::hw::register_surface_scaler(
        scaler_init,
        scaler_blit,
        scaler_check,
        scaler_clear,
        scaler_set_lumetri,
    );
    log::info!("Phase B3: HardwareSurfaceScaler registered with filmcraft_codecs");
}

#[cfg(not(target_os = "android"))]
pub fn register_scaler() {}

#[cfg(target_os = "android")]
fn scaler_set_lumetri(p: &[f32; 26]) {
    let mut guard = ACTIVE_SCALER.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(scaler) = guard.as_mut() {
        scaler.set_lumetri(p);
    }
}

#[cfg(target_os = "android")]
fn scaler_init(
    encoder_window: *mut std::ffi::c_void,
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
) -> Option<*mut std::ffi::c_void> {
    log::info!("Phase B3: Attempting HardwareSurfaceScaler initialization: {src_w}x{src_h} -> {dst_w}x{dst_h}...");
    let scaler = match HardwareSurfaceScaler::new(encoder_window, src_w, src_h, dst_w, dst_h) {
        Some(s) => s,
        None => {
            log::warn!("Phase B3: HardwareSurfaceScaler failed to initialize, falling back to Vulkan ExportRenderer");
            return None;
        }
    };
    let decoder_win = scaler.decoder_window();
    let mut guard = ACTIVE_SCALER.lock().unwrap_or_else(PoisonError::into_inner);
    *guard = Some(scaler);
    log::info!("Phase B3: HardwareSurfaceScaler active! Decoder window = {decoder_win:?}");
    Some(decoder_win)
}

#[cfg(target_os = "android")]
fn scaler_blit(pts_ns: i64) -> bool {
    let mut guard = ACTIVE_SCALER.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(scaler) = guard.as_mut() {
        scaler.blit(pts_ns)
    } else {
        false
    }
}

#[cfg(target_os = "android")]
pub fn drain_surface_scaler_texture() {
    let mut guard = ACTIVE_SCALER.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(scaler) = guard.as_mut() {
        unsafe {
            if (scaler.fn_eglMakeCurrent)(scaler.display, scaler.surface, scaler.surface, scaler.context) != 0 {
                for _ in 0..16 {
                    let res = (scaler.fn_ASurfaceTexture_updateTexImage)(scaler.asurface_texture);
                    if res != 0 {
                        break;
                    }
                }
                (scaler.fn_eglMakeCurrent)(scaler.display, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
            }
        }
    }
}

#[cfg(not(target_os = "android"))]
pub fn drain_surface_scaler_texture() {}

#[cfg(target_os = "android")]
fn scaler_check() -> bool {
    let guard = ACTIVE_SCALER.lock().unwrap_or_else(PoisonError::into_inner);
    guard.is_some()
}

#[cfg(target_os = "android")]
fn scaler_clear() {
    let mut guard = ACTIVE_SCALER.lock().unwrap_or_else(PoisonError::into_inner);
    *guard = None;
}

#[cfg(target_os = "android")]
unsafe extern "C" {
    fn dlopen(filename: *const std::os::raw::c_char, flag: std::os::raw::c_int) -> *mut std::ffi::c_void;
    fn dlsym(handle: *mut std::ffi::c_void, symbol: *const std::os::raw::c_char) -> *mut std::ffi::c_void;
    fn dlclose(handle: *mut std::ffi::c_void) -> std::os::raw::c_int;
}

#[cfg(target_os = "android")]
const EGL_DEFAULT_DISPLAY: *mut std::ffi::c_void = std::ptr::null_mut();
#[cfg(target_os = "android")]
const EGL_NO_CONTEXT: *mut std::ffi::c_void = std::ptr::null_mut();
#[cfg(target_os = "android")]
const EGL_NO_SURFACE: *mut std::ffi::c_void = std::ptr::null_mut();
#[cfg(target_os = "android")]
const EGL_NONE: i32 = 0x3038;
#[cfg(target_os = "android")]
const EGL_RED_SIZE: i32 = 0x3024;
#[cfg(target_os = "android")]
const EGL_GREEN_SIZE: i32 = 0x3023;
#[cfg(target_os = "android")]
const EGL_BLUE_SIZE: i32 = 0x3022;
#[cfg(target_os = "android")]
const EGL_ALPHA_SIZE: i32 = 0x3021;
#[cfg(target_os = "android")]
const EGL_RENDERABLE_TYPE: i32 = 0x3040;
#[cfg(target_os = "android")]
const EGL_OPENGL_ES2_BIT: i32 = 0x0004;
#[cfg(target_os = "android")]
const EGL_SURFACE_TYPE: i32 = 0x3033;
#[cfg(target_os = "android")]
const EGL_WINDOW_BIT: i32 = 0x0004;
#[cfg(target_os = "android")]
const EGL_CONTEXT_CLIENT_VERSION: i32 = 0x3098;

#[cfg(target_os = "android")]
const GL_TEXTURE_EXTERNAL_OES: u32 = 0x8D65;
#[cfg(target_os = "android")]
const GL_TEXTURE_MIN_FILTER: u32 = 0x2801;
#[cfg(target_os = "android")]
const GL_TEXTURE_MAG_FILTER: u32 = 0x2800;
#[cfg(target_os = "android")]
const GL_TEXTURE_WRAP_S: u32 = 0x2802;
#[cfg(target_os = "android")]
const GL_TEXTURE_WRAP_T: u32 = 0x2803;
#[cfg(target_os = "android")]
const GL_CLAMP_TO_EDGE: i32 = 0x812F;
#[cfg(target_os = "android")]
const GL_LINEAR: i32 = 0x2601;
#[cfg(target_os = "android")]
const GL_VERTEX_SHADER: u32 = 0x8B31;
#[cfg(target_os = "android")]
const GL_FRAGMENT_SHADER: u32 = 0x8B30;
#[cfg(target_os = "android")]
const GL_COMPILE_STATUS: u32 = 0x8B81;
#[cfg(target_os = "android")]
const GL_LINK_STATUS: u32 = 0x8B82;
#[cfg(target_os = "android")]
const GL_FLOAT: u32 = 0x1406;
#[cfg(target_os = "android")]
const GL_TRIANGLE_STRIP: u32 = 0x0005;

#[cfg(target_os = "android")]
const VS_SRC: &[u8] = b"attribute vec4 aPosition;\n\
attribute vec4 aTexCoord;\n\
uniform mat4 uTexMatrix;\n\
varying vec2 vTexCoord;\n\
void main() {\n\
    gl_Position = aPosition;\n\
    vTexCoord = (uTexMatrix * aTexCoord).xy;\n\
}\0";

#[cfg(target_os = "android")]
const FS_SRC: &[u8] = b"#extension GL_OES_EGL_image_external : require\n\
precision highp float;\n\
uniform samplerExternalOES sTexture;\n\
varying vec2 vTexCoord;\n\
uniform vec4 u_p0;\n\
uniform vec4 u_p1;\n\
uniform vec4 u_p2;\n\
uniform vec4 u_p3;\n\
uniform vec4 u_p4;\n\
uniform vec4 u_p5;\n\
uniform vec2 u_i1;\n\
uniform vec2 u_resolution;\n\
\n\
float linear_to_srgb1(float v) {\n\
    if (v <= 0.0031308) {\n\
        return 12.92 * v;\n\
    }\n\
    return 1.055 * pow(max(v, 0.0), 0.41666667) - 0.055;\n\
}\n\
\n\
float srgb_to_linear1(float v) {\n\
    if (v <= 0.04045) {\n\
        return v / 12.92;\n\
    }\n\
    return pow(max((v + 0.055) / 1.055, 0.0), 2.4);\n\
}\n\
\n\
vec3 enc(vec3 c) {\n\
    return vec3(linear_to_srgb1(c.r), linear_to_srgb1(c.g), linear_to_srgb1(c.b));\n\
}\n\
\n\
vec3 dec(vec3 c) {\n\
    return vec3(srgb_to_linear1(c.r), srgb_to_linear1(c.g), srgb_to_linear1(c.b));\n\
}\n\
\n\
float luma709(vec3 c) {\n\
    return dot(c, vec3(0.2126, 0.7152, 0.0722));\n\
}\n\
\n\
void main() {\n\
    vec4 src_sample = texture2D(sTexture, vTexCoord);\n\
    vec3 c = dec(src_sample.rgb);\n\
    vec3 ge = u_p0.rgb;\n\
    vec3 lin = c * ge;\n\
    vec3 v = enc(lin);\n\
    float b0 = u_p0.a;\n\
    float w0 = u_p1.r;\n\
    float denom = max(w0 - b0, 0.001);\n\
    v = (v - vec3(b0)) / denom;\n\
    float sh_k = u_p1.g;\n\
    float hl_k = u_p1.b;\n\
    float l = luma709(v);\n\
    float ws_val = clamp(1.0 - l, 0.0, 1.0);\n\
    float ws = ws_val * ws_val * ws_val;\n\
    float whl_val = clamp(l, 0.0, 1.0);\n\
    float whl = whl_val * whl_val * whl_val;\n\
    float nl = max(l + sh_k * ws + hl_k * whl, 0.0);\n\
    float k = 1.0;\n\
    if (l > 0.00001) {\n\
        k = min(nl / l, 1.0 + max(nl - l, 0.0) / 0.02);\n\
    }\n\
    float add = nl - max(l, 0.0) * k;\n\
    v = v * k + vec3(add);\n\
    float contrast = u_p1.a;\n\
    if (abs(contrast) > 0.0001) {\n\
        float kc = 1.0 + contrast;\n\
        vec3 q = clamp(v, 0.0, 1.0);\n\
        vec3 sc = q * q * (vec3(3.0) - 2.0 * q);\n\
        if (kc >= 1.0) {\n\
            v = q + (sc - q) * (kc - 1.0);\n\
        } else {\n\
            v = vec3(0.5) + (q - vec3(0.5)) * kc;\n\
        }\n\
    }\n\
    float faded = u_p2.r;\n\
    if (faded > 0.0) {\n\
        v = v * (1.0 - 0.25 * faded) + vec3(0.12 * faded);\n\
    }\n\
    if (u_i1.x > 0.5) {\n\
        vec3 st_k = u_p4.rgb;\n\
        vec3 ht_k = u_p5.rgb;\n\
        float l2 = clamp(luma709(v), 0.0, 1.0);\n\
        v = v + st_k * (1.0 - l2) + ht_k * l2;\n\
    }\n\
    float sat = u_p2.g;\n\
    float vib = u_p2.b;\n\
    float l3 = luma709(v);\n\
    float cur_sat = max(max(v.r, v.g), v.b) - min(min(v.r, v.g), v.b);\n\
    float sat_mult = sat * (1.0 + vib * (1.0 - clamp(cur_sat, 0.0, 1.0)));\n\
    v = vec3(l3) + (v - vec3(l3)) * sat_mult;\n\
    float va = u_p2.a;\n\
    if (u_i1.y > 0.5 && abs(va) > 0.0001) {\n\
        float vmid = u_p3.r;\n\
        float vfeather = u_p3.g;\n\
        float vround_aspect = u_p3.b;\n\
        vec2 p = vTexCoord * u_resolution;\n\
        float nx = (p.x / u_resolution.x - 0.5) * 2.0 * vround_aspect;\n\
        float ny = (p.y / u_resolution.y - 0.5) * 2.0;\n\
        float d = sqrt(nx * nx + ny * ny) / 1.41421356;\n\
        float edge = clamp((d - vmid * 0.9) / (max(vfeather, 0.01) * 0.9), 0.0, 1.0);\n\
        float e2 = edge * edge * (3.0 - 2.0 * edge);\n\
        float kv = 1.0 + va * 0.2 * e2;\n\
        if (va < 0.0) {\n\
            v = v * max(kv, 0.0);\n\
        } else {\n\
            v = v + (vec3(1.0) - v) * (kv - 1.0);\n\
        }\n\
    }\n\
    gl_FragColor = vec4(clamp(v, 0.0, 1.0), 1.0);\n\
}\0";

#[cfg(target_os = "android")]
fn compile_shader(
    gl_create_shader: unsafe extern "C" fn(u32) -> u32,
    gl_shader_source: unsafe extern "C" fn(u32, i32, *const *const std::os::raw::c_char, *const i32),
    gl_compile_shader: unsafe extern "C" fn(u32),
    gl_get_shader_iv: unsafe extern "C" fn(u32, u32, *mut i32),
    gl_get_shader_info_log: Option<unsafe extern "C" fn(u32, i32, *mut i32, *mut std::os::raw::c_char)>,
    shader_type: u32,
    src: &[u8],
) -> Option<u32> {
    unsafe {
        let shader = gl_create_shader(shader_type);
        if shader == 0 {
            return None;
        }
        let src_ptr = src.as_ptr() as *const std::os::raw::c_char;
        let len = src.len().saturating_sub(1) as i32;
        gl_shader_source(shader, 1, &src_ptr, &len);
        gl_compile_shader(shader);
        let mut status = 0i32;
        gl_get_shader_iv(shader, GL_COMPILE_STATUS, &mut status);
        if status == 0 {
            if let Some(get_log) = gl_get_shader_info_log {
                let mut buf = [0u8; 1024];
                let mut len = 0i32;
                get_log(shader, 1024, &mut len, buf.as_mut_ptr() as *mut _);
                let log_str = String::from_utf8_lossy(&buf[..len.max(0) as usize]);
                log::warn!("HardwareSurfaceScaler shader compile failed (type {shader_type}): {log_str}");
            }
            return None;
        }
        Some(shader)
    }
}

#[cfg(target_os = "android")]
pub struct HardwareSurfaceScaler {
    egl_lib: *mut std::ffi::c_void,
    gles_lib: *mut std::ffi::c_void,
    android_lib: *mut std::ffi::c_void,
    display: *mut std::ffi::c_void,
    context: *mut std::ffi::c_void,
    surface: *mut std::ffi::c_void,
    program: u32,
    tex_id: u32,
    u_texture_loc: i32,
    u_tex_matrix_loc: i32,
    a_position_loc: i32,
    a_tex_coord_loc: i32,
    u_p0_loc: i32,
    u_p1_loc: i32,
    u_p2_loc: i32,
    u_p3_loc: i32,
    u_p4_loc: i32,
    u_p5_loc: i32,
    u_i1_loc: i32,
    u_resolution_loc: i32,
    lumetri_params: [f32; 26],
    asurface_texture: *mut std::ffi::c_void,
    decoder_window: *mut std::ffi::c_void,
    _java_st_ref: Global<JObject<'static>>,
    // EGL function pointers
    fn_eglGetCurrentContext: unsafe extern "C" fn() -> *mut std::ffi::c_void,
    fn_eglMakeCurrent: unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void, *mut std::ffi::c_void, *mut std::ffi::c_void) -> u32,
    fn_eglSwapBuffers: unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void) -> u32,
    fn_eglDestroySurface: unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void) -> u32,
    fn_eglDestroyContext: unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void) -> u32,
    fn_eglTerminate: unsafe extern "C" fn(*mut std::ffi::c_void) -> u32,
    fn_eglPresentationTimeANDROID: Option<unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void, i64) -> u32>,
    // GLES function pointers
    fn_glUseProgram: unsafe extern "C" fn(u32),
    fn_glBindTexture: unsafe extern "C" fn(u32, u32),
    fn_glUniform1i: unsafe extern "C" fn(i32, i32),
    fn_glUniform2f: unsafe extern "C" fn(i32, f32, f32),
    fn_glUniform4fv: unsafe extern "C" fn(i32, i32, *const f32),
    fn_glUniformMatrix4fv: unsafe extern "C" fn(i32, i32, u8, *const f32),
    fn_glViewport: unsafe extern "C" fn(i32, i32, i32, i32),
    fn_glEnableVertexAttribArray: unsafe extern "C" fn(u32),
    fn_glVertexAttribPointer: unsafe extern "C" fn(u32, i32, u32, u8, i32, *const std::ffi::c_void),
    fn_glDrawArrays: unsafe extern "C" fn(u32, i32, i32),
    fn_glDeleteTextures: unsafe extern "C" fn(i32, *const u32),
    fn_glDeleteProgram: unsafe extern "C" fn(u32),
    // NDK function pointers
    fn_ASurfaceTexture_updateTexImage: unsafe extern "C" fn(*mut std::ffi::c_void) -> i32,
    fn_ASurfaceTexture_getTransformMatrix: unsafe extern "C" fn(*mut std::ffi::c_void, *mut f32),
    fn_ASurfaceTexture_getTimestamp: unsafe extern "C" fn(*mut std::ffi::c_void) -> i64,
    fn_ASurfaceTexture_release: unsafe extern "C" fn(*mut std::ffi::c_void),
    fn_ANativeWindow_release: unsafe extern "C" fn(*mut std::ffi::c_void),
    dst_w: u32,
    dst_h: u32,
}

#[cfg(target_os = "android")]
unsafe impl Send for HardwareSurfaceScaler {}

#[cfg(target_os = "android")]
impl HardwareSurfaceScaler {
    pub fn decoder_window(&self) -> *mut std::ffi::c_void {
        self.decoder_window
    }

    pub fn new(
        encoder_window: *mut std::ffi::c_void,
        src_w: u32,
        src_h: u32,
        dst_w: u32,
        dst_h: u32,
    ) -> Option<Self> {
        if encoder_window.is_null() || src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
            log::warn!("HardwareSurfaceScaler: invalid arguments");
            return None;
        }

        unsafe {
            // 1. Dynamic load libraries
            let egl_lib = dlopen(b"libEGL.so\0".as_ptr() as *const _, 2);
            if egl_lib.is_null() {
                log::warn!("HardwareSurfaceScaler: failed to dlopen libEGL.so");
                return None;
            }

            let gles_lib = dlopen(b"libGLESv2.so\0".as_ptr() as *const _, 2);
            if gles_lib.is_null() {
                log::warn!("HardwareSurfaceScaler: failed to dlopen libGLESv2.so");
                dlclose(egl_lib);
                return None;
            }

            let android_lib = dlopen(b"libandroid.so\0".as_ptr() as *const _, 2);
            if android_lib.is_null() {
                log::warn!("HardwareSurfaceScaler: failed to dlopen libandroid.so");
                dlclose(gles_lib);
                dlclose(egl_lib);
                return None;
            }

            let load = |lib: *mut std::ffi::c_void, name: &[u8]| -> Option<*mut std::ffi::c_void> {
                let p = dlsym(lib, name.as_ptr() as *const _);
                if p.is_null() {
                    log::warn!("HardwareSurfaceScaler: missing symbol {}", std::str::from_utf8(name).unwrap_or("?"));
                    None
                } else {
                    Some(p)
                }
            };

            // EGL symbols
            let fn_eglGetCurrentContext: unsafe extern "C" fn() -> *mut std::ffi::c_void = std::mem::transmute(load(egl_lib, b"eglGetCurrentContext\0")?);
            let fn_eglGetDisplay: unsafe extern "C" fn(*mut std::ffi::c_void) -> *mut std::ffi::c_void = std::mem::transmute(load(egl_lib, b"eglGetDisplay\0")?);
            let fn_eglInitialize: unsafe extern "C" fn(*mut std::ffi::c_void, *mut i32, *mut i32) -> u32 = std::mem::transmute(load(egl_lib, b"eglInitialize\0")?);
            let fn_eglChooseConfig: unsafe extern "C" fn(*mut std::ffi::c_void, *const i32, *mut *mut std::ffi::c_void, i32, *mut i32) -> u32 = std::mem::transmute(load(egl_lib, b"eglChooseConfig\0")?);
            let fn_eglCreateContext: unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void, *mut std::ffi::c_void, *const i32) -> *mut std::ffi::c_void = std::mem::transmute(load(egl_lib, b"eglCreateContext\0")?);
            let fn_eglCreateWindowSurface: unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void, *mut std::ffi::c_void, *const i32) -> *mut std::ffi::c_void = std::mem::transmute(load(egl_lib, b"eglCreateWindowSurface\0")?);
            let fn_eglMakeCurrent: unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void, *mut std::ffi::c_void, *mut std::ffi::c_void) -> u32 = std::mem::transmute(load(egl_lib, b"eglMakeCurrent\0")?);
            let fn_eglSwapBuffers: unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void) -> u32 = std::mem::transmute(load(egl_lib, b"eglSwapBuffers\0")?);
            let fn_eglDestroySurface: unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void) -> u32 = std::mem::transmute(load(egl_lib, b"eglDestroySurface\0")?);
            let fn_eglDestroyContext: unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void) -> u32 = std::mem::transmute(load(egl_lib, b"eglDestroyContext\0")?);
            let fn_eglTerminate: unsafe extern "C" fn(*mut std::ffi::c_void) -> u32 = std::mem::transmute(load(egl_lib, b"eglTerminate\0")?);
            let fn_eglGetProcAddress: unsafe extern "C" fn(*const std::os::raw::c_char) -> *mut std::ffi::c_void = std::mem::transmute(load(egl_lib, b"eglGetProcAddress\0")?);

            let fn_eglPresentationTimeANDROID: Option<unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void, i64) -> u32> = {
                let p = fn_eglGetProcAddress(b"eglPresentationTimeANDROID\0".as_ptr() as *const _);
                if !p.is_null() {
                    Some(std::mem::transmute(p))
                } else {
                    None
                }
            };

            // GLES symbols
            let fn_glGenTextures: unsafe extern "C" fn(i32, *mut u32) = std::mem::transmute(load(gles_lib, b"glGenTextures\0")?);
            let fn_glBindTexture: unsafe extern "C" fn(u32, u32) = std::mem::transmute(load(gles_lib, b"glBindTexture\0")?);
            let fn_glTexParameteri: unsafe extern "C" fn(u32, u32, i32) = std::mem::transmute(load(gles_lib, b"glTexParameteri\0")?);
            let fn_glCreateShader: unsafe extern "C" fn(u32) -> u32 = std::mem::transmute(load(gles_lib, b"glCreateShader\0")?);
            let fn_glShaderSource: unsafe extern "C" fn(u32, i32, *const *const std::os::raw::c_char, *const i32) = std::mem::transmute(load(gles_lib, b"glShaderSource\0")?);
            let fn_glCompileShader: unsafe extern "C" fn(u32) = std::mem::transmute(load(gles_lib, b"glCompileShader\0")?);
            let fn_glGetShaderiv: unsafe extern "C" fn(u32, u32, *mut i32) = std::mem::transmute(load(gles_lib, b"glGetShaderiv\0")?);
            let fn_glCreateProgram: unsafe extern "C" fn() -> u32 = std::mem::transmute(load(gles_lib, b"glCreateProgram\0")?);
            let fn_glAttachShader: unsafe extern "C" fn(u32, u32) = std::mem::transmute(load(gles_lib, b"glAttachShader\0")?);
            let fn_glLinkProgram: unsafe extern "C" fn(u32) = std::mem::transmute(load(gles_lib, b"glLinkProgram\0")?);
            let fn_glGetProgramiv: unsafe extern "C" fn(u32, u32, *mut i32) = std::mem::transmute(load(gles_lib, b"glGetProgramiv\0")?);
            let fn_glUseProgram: unsafe extern "C" fn(u32) = std::mem::transmute(load(gles_lib, b"glUseProgram\0")?);
            let fn_glGetAttribLocation: unsafe extern "C" fn(u32, *const std::os::raw::c_char) -> i32 = std::mem::transmute(load(gles_lib, b"glGetAttribLocation\0")?);
            let fn_glGetUniformLocation: unsafe extern "C" fn(u32, *const std::os::raw::c_char) -> i32 = std::mem::transmute(load(gles_lib, b"glGetUniformLocation\0")?);
            let fn_glEnableVertexAttribArray: unsafe extern "C" fn(u32) = std::mem::transmute(load(gles_lib, b"glEnableVertexAttribArray\0")?);
            let fn_glVertexAttribPointer: unsafe extern "C" fn(u32, i32, u32, u8, i32, *const std::ffi::c_void) = std::mem::transmute(load(gles_lib, b"glVertexAttribPointer\0")?);
            let fn_glUniformMatrix4fv: unsafe extern "C" fn(i32, i32, u8, *const f32) = std::mem::transmute(load(gles_lib, b"glUniformMatrix4fv\0")?);
            let fn_glUniform1i: unsafe extern "C" fn(i32, i32) = std::mem::transmute(load(gles_lib, b"glUniform1i\0")?);
            let fn_glUniform2f: unsafe extern "C" fn(i32, f32, f32) = std::mem::transmute(load(gles_lib, b"glUniform2f\0")?);
            let fn_glUniform4fv: unsafe extern "C" fn(i32, i32, *const f32) = std::mem::transmute(load(gles_lib, b"glUniform4fv\0")?);
            let fn_glViewport: unsafe extern "C" fn(i32, i32, i32, i32) = std::mem::transmute(load(gles_lib, b"glViewport\0")?);
            let fn_glDrawArrays: unsafe extern "C" fn(u32, i32, i32) = std::mem::transmute(load(gles_lib, b"glDrawArrays\0")?);
            let fn_glDeleteTextures: unsafe extern "C" fn(i32, *const u32) = std::mem::transmute(load(gles_lib, b"glDeleteTextures\0")?);
            let fn_glDeleteProgram: unsafe extern "C" fn(u32) = std::mem::transmute(load(gles_lib, b"glDeleteProgram\0")?);
            let fn_glDeleteShader: unsafe extern "C" fn(u32) = std::mem::transmute(load(gles_lib, b"glDeleteShader\0")?);
            let fn_glGetShaderInfoLog: Option<unsafe extern "C" fn(u32, i32, *mut i32, *mut std::os::raw::c_char)> = {
                let p = dlsym(gles_lib, b"glGetShaderInfoLog\0".as_ptr() as *const _);
                if !p.is_null() { Some(std::mem::transmute(p)) } else { None }
            };

            // NDK symbols
            let fn_ASurfaceTexture_fromSurfaceTexture: unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void) -> *mut std::ffi::c_void = std::mem::transmute(load(android_lib, b"ASurfaceTexture_fromSurfaceTexture\0")?);
            let fn_ASurfaceTexture_acquireANativeWindow: unsafe extern "C" fn(*mut std::ffi::c_void) -> *mut std::ffi::c_void = std::mem::transmute(load(android_lib, b"ASurfaceTexture_acquireANativeWindow\0")?);
            let fn_ASurfaceTexture_updateTexImage: unsafe extern "C" fn(*mut std::ffi::c_void) -> i32 = std::mem::transmute(load(android_lib, b"ASurfaceTexture_updateTexImage\0")?);
            let fn_ASurfaceTexture_getTransformMatrix: unsafe extern "C" fn(*mut std::ffi::c_void, *mut f32) = std::mem::transmute(load(android_lib, b"ASurfaceTexture_getTransformMatrix\0")?);
            let fn_ASurfaceTexture_getTimestamp: unsafe extern "C" fn(*mut std::ffi::c_void) -> i64 = std::mem::transmute(load(android_lib, b"ASurfaceTexture_getTimestamp\0")?);
            let fn_ASurfaceTexture_release: unsafe extern "C" fn(*mut std::ffi::c_void) = std::mem::transmute(load(android_lib, b"ASurfaceTexture_release\0")?);
            let fn_ANativeWindow_release: unsafe extern "C" fn(*mut std::ffi::c_void) = std::mem::transmute(load(android_lib, b"ANativeWindow_release\0")?);

            // 2. Setup EGL display and context
            let display = fn_eglGetDisplay(EGL_DEFAULT_DISPLAY);
            if display.is_null() {
                log::warn!("HardwareSurfaceScaler: eglGetDisplay failed");
                dlclose(android_lib); dlclose(gles_lib); dlclose(egl_lib);
                return None;
            }

            let mut major = 0i32;
            let mut minor = 0i32;
            if fn_eglInitialize(display, &mut major, &mut minor) == 0 {
                log::warn!("HardwareSurfaceScaler: eglInitialize failed");
                dlclose(android_lib); dlclose(gles_lib); dlclose(egl_lib);
                return None;
            }

            let config_attribs = [
                EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT,
                EGL_SURFACE_TYPE, EGL_WINDOW_BIT,
                EGL_RED_SIZE, 8,
                EGL_GREEN_SIZE, 8,
                EGL_BLUE_SIZE, 8,
                EGL_ALPHA_SIZE, 8,
                EGL_NONE,
            ];

            let mut config: *mut std::ffi::c_void = std::ptr::null_mut();
            let mut num_configs = 0i32;
            if fn_eglChooseConfig(display, config_attribs.as_ptr(), &mut config, 1, &mut num_configs) == 0 || num_configs == 0 {
                log::warn!("HardwareSurfaceScaler: eglChooseConfig failed");
                fn_eglTerminate(display);
                dlclose(android_lib); dlclose(gles_lib); dlclose(egl_lib);
                return None;
            }

            let ctx_attribs = [
                EGL_CONTEXT_CLIENT_VERSION, 2,
                EGL_NONE,
            ];

            let context = fn_eglCreateContext(display, config, EGL_NO_CONTEXT, ctx_attribs.as_ptr());
            if context.is_null() {
                log::warn!("HardwareSurfaceScaler: eglCreateContext failed");
                fn_eglTerminate(display);
                dlclose(android_lib); dlclose(gles_lib); dlclose(egl_lib);
                return None;
            }

            let surface_attribs = [EGL_NONE];
            let fn_ANativeWindow_setBuffersDataSpace: Option<unsafe extern "C" fn(*mut std::ffi::c_void, i32) -> i32> = {
                let sym = dlsym(android_lib, b"ANativeWindow_setBuffersDataSpace\0".as_ptr() as *const _);
                if !sym.is_null() {
                    Some(std::mem::transmute(sym))
                } else {
                    None
                }
            };
            if let Some(set_ds) = fn_ANativeWindow_setBuffersDataSpace {
                let res = set_ds(encoder_window, 281083904);
                log::info!("HardwareSurfaceScaler: ANativeWindow_setBuffersDataSpace(encoder_window, BT709) = {res}");
            }
            let surface = fn_eglCreateWindowSurface(display, config, encoder_window, surface_attribs.as_ptr());
            if surface.is_null() {
                log::warn!("HardwareSurfaceScaler: eglCreateWindowSurface failed");
                fn_eglDestroyContext(display, context);
                fn_eglTerminate(display);
                dlclose(android_lib); dlclose(gles_lib); dlclose(egl_lib);
                return None;
            }

            if fn_eglMakeCurrent(display, surface, surface, context) == 0 {
                log::warn!("HardwareSurfaceScaler: initial eglMakeCurrent failed");
                fn_eglDestroySurface(display, surface);
                fn_eglDestroyContext(display, context);
                fn_eglTerminate(display);
                dlclose(android_lib); dlclose(gles_lib); dlclose(egl_lib);
                return None;
            }

            // 3. Compile Shaders and Program
            let vs = compile_shader(fn_glCreateShader, fn_glShaderSource, fn_glCompileShader, fn_glGetShaderiv, fn_glGetShaderInfoLog, GL_VERTEX_SHADER, VS_SRC);
            let fs = compile_shader(fn_glCreateShader, fn_glShaderSource, fn_glCompileShader, fn_glGetShaderiv, fn_glGetShaderInfoLog, GL_FRAGMENT_SHADER, FS_SRC);
            let (vs, fs) = match (vs, fs) {
                (Some(v), Some(f)) => (v, f),
                _ => {
                    log::warn!("HardwareSurfaceScaler: shader compilation failed");
                    fn_eglMakeCurrent(display, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
                    fn_eglDestroySurface(display, surface);
                    fn_eglDestroyContext(display, context);
                    fn_eglTerminate(display);
                    dlclose(android_lib); dlclose(gles_lib); dlclose(egl_lib);
                    return None;
                }
            };

            let program = fn_glCreateProgram();
            fn_glAttachShader(program, vs);
            fn_glAttachShader(program, fs);
            fn_glLinkProgram(program);
            let mut link_status = 0i32;
            fn_glGetProgramiv(program, GL_LINK_STATUS, &mut link_status);
            fn_glDeleteShader(vs);
            fn_glDeleteShader(fs);

            if link_status == 0 {
                log::warn!("HardwareSurfaceScaler: shader link failed");
                fn_glDeleteProgram(program);
                fn_eglMakeCurrent(display, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
                fn_eglDestroySurface(display, surface);
                fn_eglDestroyContext(display, context);
                fn_eglTerminate(display);
                dlclose(android_lib); dlclose(gles_lib); dlclose(egl_lib);
                return None;
            }

            let a_position_loc = fn_glGetAttribLocation(program, b"aPosition\0".as_ptr() as *const _);
            let a_tex_coord_loc = fn_glGetAttribLocation(program, b"aTexCoord\0".as_ptr() as *const _);
            let u_texture_loc = fn_glGetUniformLocation(program, b"sTexture\0".as_ptr() as *const _);
            let u_tex_matrix_loc = fn_glGetUniformLocation(program, b"uTexMatrix\0".as_ptr() as *const _);
            let u_p0_loc = fn_glGetUniformLocation(program, b"u_p0\0".as_ptr() as *const _);
            let u_p1_loc = fn_glGetUniformLocation(program, b"u_p1\0".as_ptr() as *const _);
            let u_p2_loc = fn_glGetUniformLocation(program, b"u_p2\0".as_ptr() as *const _);
            let u_p3_loc = fn_glGetUniformLocation(program, b"u_p3\0".as_ptr() as *const _);
            let u_p4_loc = fn_glGetUniformLocation(program, b"u_p4\0".as_ptr() as *const _);
            let u_p5_loc = fn_glGetUniformLocation(program, b"u_p5\0".as_ptr() as *const _);
            let u_i1_loc = fn_glGetUniformLocation(program, b"u_i1\0".as_ptr() as *const _);
            let u_resolution_loc = fn_glGetUniformLocation(program, b"u_resolution\0".as_ptr() as *const _);

            // 4. Create External Texture
            let mut tex_id = 0u32;
            fn_glGenTextures(1, &mut tex_id);
            fn_glBindTexture(GL_TEXTURE_EXTERNAL_OES, tex_id);
            fn_glTexParameteri(GL_TEXTURE_EXTERNAL_OES, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
            fn_glTexParameteri(GL_TEXTURE_EXTERNAL_OES, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
            fn_glTexParameteri(GL_TEXTURE_EXTERNAL_OES, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
            fn_glTexParameteri(GL_TEXTURE_EXTERNAL_OES, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);

            // 5. Create SurfaceTexture via JNI
            let ctx = ndk_context::android_context();
            let vm_ptr: *mut jni::sys::JavaVM = ctx.vm().cast();
            if vm_ptr.is_null() {
                log::warn!("HardwareSurfaceScaler: ndk_context JavaVM is null");
                fn_glDeleteTextures(1, &tex_id);
                fn_glDeleteProgram(program);
                fn_eglMakeCurrent(display, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
                fn_eglDestroySurface(display, surface);
                fn_eglDestroyContext(display, context);
                fn_eglTerminate(display);
                dlclose(android_lib); dlclose(gles_lib); dlclose(egl_lib);
                return None;
            }

            let vm = JavaVM::from_raw(vm_ptr);
            let jni_res = vm.attach_current_thread(|env| -> jni::errors::Result<(Global<JObject<'static>>, *mut std::ffi::c_void, *mut std::ffi::c_void)> {
                let st_class = env.find_class(jni_str!("android/graphics/SurfaceTexture"))?;
                let st_obj = env.new_object(
                    &st_class,
                    jni_sig!("(I)V"),
                    &[JValue::Int(tex_id as i32)],
                )?;
                env.call_method(
                    &st_obj,
                    jni_str!("setDefaultBufferSize"),
                    jni_sig!("(II)V"),
                    &[JValue::Int(src_w as i32), JValue::Int(src_h as i32)],
                )?;
                let env_ptr = env.get_raw();
                let st_raw = st_obj.as_raw();
                let asurface_texture = fn_ASurfaceTexture_fromSurfaceTexture(env_ptr as *mut _, st_raw as *mut _);
                if asurface_texture.is_null() {
                    return Err(jni::errors::Error::NullPtr("ASurfaceTexture_fromSurfaceTexture returned null"));
                }
                let decoder_window = fn_ASurfaceTexture_acquireANativeWindow(asurface_texture);
                if decoder_window.is_null() {
                    fn_ASurfaceTexture_release(asurface_texture);
                    return Err(jni::errors::Error::NullPtr("ASurfaceTexture_acquireANativeWindow returned null"));
                }
                let global_ref = env.new_global_ref(st_obj)?;
                Ok((global_ref, asurface_texture, decoder_window))
            });

            let (_java_st_ref, asurface_texture, decoder_window) = match jni_res {
                Ok(tuple) => tuple,
                Err(e) => {
                    log::warn!("HardwareSurfaceScaler: JNI SurfaceTexture setup failed: {e:?}");
                    fn_glDeleteTextures(1, &tex_id);
                    fn_glDeleteProgram(program);
                    fn_eglMakeCurrent(display, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
                    fn_eglDestroySurface(display, surface);
                    fn_eglDestroyContext(display, context);
                    fn_eglTerminate(display);
                    dlclose(android_lib); dlclose(gles_lib); dlclose(egl_lib);
                    return None;
                }
            };

            let fn_ANativeWindow_setDequeueTimeout: Option<unsafe extern "C" fn(*mut std::ffi::c_void, i64) -> i32> = {
                let p = dlsym(android_lib, b"ANativeWindow_setDequeueTimeout\0".as_ptr() as *const _);
                if !p.is_null() {
                    Some(std::mem::transmute(p))
                } else {
                    None
                }
            };
            if let Some(set_timeout) = fn_ANativeWindow_setDequeueTimeout {
                let res = set_timeout(decoder_window, 100_000_000);
                log::info!("HardwareSurfaceScaler: ANativeWindow_setDequeueTimeout(100ms) = {res}");
            }

            // Always unbind context after initialization so other threads can access it
            fn_eglMakeCurrent(display, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);

            log::info!("HardwareSurfaceScaler successfully configured: {src_w}x{src_h} -> {dst_w}x{dst_h}");

            Some(Self {
                egl_lib,
                gles_lib,
                android_lib,
                display,
                context,
                surface,
                program,
                tex_id,
                u_texture_loc,
                u_tex_matrix_loc,
                a_position_loc,
                a_tex_coord_loc,
                u_p0_loc,
                u_p1_loc,
                u_p2_loc,
                u_p3_loc,
                u_p4_loc,
                u_p5_loc,
                u_i1_loc,
                u_resolution_loc,
                lumetri_params: DEFAULT_LUMETRI,
                asurface_texture,
                decoder_window,
                _java_st_ref,
                fn_eglGetCurrentContext,
                fn_eglMakeCurrent,
                fn_eglSwapBuffers,
                fn_eglDestroySurface,
                fn_eglDestroyContext,
                fn_eglTerminate,
                fn_eglPresentationTimeANDROID,
                fn_glUseProgram,
                fn_glBindTexture,
                fn_glUniform1i,
                fn_glUniform2f,
                fn_glUniform4fv,
                fn_glUniformMatrix4fv,
                fn_glViewport,
                fn_glEnableVertexAttribArray,
                fn_glVertexAttribPointer,
                fn_glDrawArrays,
                fn_glDeleteTextures,
                fn_glDeleteProgram,
                fn_ASurfaceTexture_updateTexImage,
                fn_ASurfaceTexture_getTransformMatrix,
                fn_ASurfaceTexture_getTimestamp,
                fn_ASurfaceTexture_release,
                fn_ANativeWindow_release,
                dst_w,
                dst_h,
            })
        }
    }

    pub fn set_lumetri(&mut self, p: &[f32; 26]) {
        self.lumetri_params = *p;
    }

    pub fn blit(&mut self, pts_ns: i64) -> bool {
        unsafe {
            if (self.fn_eglGetCurrentContext)() != self.context {
                if (self.fn_eglMakeCurrent)(self.display, self.surface, self.surface, self.context) == 0 {
                    log::warn!("HardwareSurfaceScaler: eglMakeCurrent failed");
                    return false;
                }
            }

            let _ = (self.fn_ASurfaceTexture_updateTexImage)(self.asurface_texture);

            let mut mtx = [0.0f32; 16];
            (self.fn_ASurfaceTexture_getTransformMatrix)(self.asurface_texture, mtx.as_mut_ptr());

            if let Some(set_time) = self.fn_eglPresentationTimeANDROID {
                if pts_ns >= 0 {
                    set_time(self.display, self.surface, pts_ns);
                }
            }

            (self.fn_glViewport)(0, 0, self.dst_w as i32, self.dst_h as i32);
            (self.fn_glUseProgram)(self.program);

            (self.fn_glBindTexture)(GL_TEXTURE_EXTERNAL_OES, self.tex_id);
            (self.fn_glUniform1i)(self.u_texture_loc, 0);
            (self.fn_glUniformMatrix4fv)(self.u_tex_matrix_loc, 1, 0, mtx.as_ptr());

            (self.fn_glUniform4fv)(self.u_p0_loc, 1, self.lumetri_params[0..4].as_ptr());
            (self.fn_glUniform4fv)(self.u_p1_loc, 1, self.lumetri_params[4..8].as_ptr());
            (self.fn_glUniform4fv)(self.u_p2_loc, 1, self.lumetri_params[8..12].as_ptr());
            (self.fn_glUniform4fv)(self.u_p3_loc, 1, self.lumetri_params[12..16].as_ptr());
            (self.fn_glUniform4fv)(self.u_p4_loc, 1, self.lumetri_params[16..20].as_ptr());
            (self.fn_glUniform4fv)(self.u_p5_loc, 1, self.lumetri_params[20..24].as_ptr());
            (self.fn_glUniform2f)(self.u_i1_loc, self.lumetri_params[24], self.lumetri_params[25]);
            (self.fn_glUniform2f)(self.u_resolution_loc, self.dst_w as f32, self.dst_h as f32);

            const POS: [f32; 8] = [-1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0];
            const UV: [f32; 8] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0];

            (self.fn_glEnableVertexAttribArray)(self.a_position_loc as u32);
            (self.fn_glVertexAttribPointer)(
                self.a_position_loc as u32,
                2,
                GL_FLOAT,
                0,
                0,
                POS.as_ptr() as *const _,
            );

            (self.fn_glEnableVertexAttribArray)(self.a_tex_coord_loc as u32);
            (self.fn_glVertexAttribPointer)(
                self.a_tex_coord_loc as u32,
                2,
                GL_FLOAT,
                0,
                0,
                UV.as_ptr() as *const _,
            );

            (self.fn_glDrawArrays)(GL_TRIANGLE_STRIP, 0, 4);

            let swap_res = (self.fn_eglSwapBuffers)(self.display, self.surface);

            swap_res != 0
        }
    }
}

#[cfg(target_os = "android")]
impl Drop for HardwareSurfaceScaler {
    fn drop(&mut self) {
        unsafe {
            if !self.decoder_window.is_null() {
                (self.fn_ANativeWindow_release)(self.decoder_window);
            }
            if !self.asurface_texture.is_null() {
                (self.fn_ASurfaceTexture_release)(self.asurface_texture);
            }
            if !self.display.is_null() {
                if !self.surface.is_null() && !self.context.is_null() {
                    (self.fn_eglMakeCurrent)(self.display, self.surface, self.surface, self.context);
                    if self.program != 0 {
                        (self.fn_glDeleteProgram)(self.program);
                    }
                    if self.tex_id != 0 {
                        (self.fn_glDeleteTextures)(1, &self.tex_id);
                    }
                }
                (self.fn_eglMakeCurrent)(self.display, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
                if !self.surface.is_null() {
                    (self.fn_eglDestroySurface)(self.display, self.surface);
                }
                if !self.context.is_null() {
                    (self.fn_eglDestroyContext)(self.display, self.context);
                }
                (self.fn_eglTerminate)(self.display);
            }
            if !self.egl_lib.is_null() {
                dlclose(self.egl_lib);
            }
            if !self.gles_lib.is_null() {
                dlclose(self.gles_lib);
            }
            if !self.android_lib.is_null() {
                dlclose(self.android_lib);
            }
        }
        log::info!("HardwareSurfaceScaler dropped and all resources released");
    }
}

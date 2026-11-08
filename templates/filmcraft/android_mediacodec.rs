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
        let mut e = match starts.get(k + 1) {
            Some(&next_s) => next_s.saturating_sub(3),
            None => data.len(),
        };
        // Trailing zero bytes belong to the next start code (4-byte form 00 00 00 01) or are trailing padding
        while e > s && data.get(e.saturating_sub(1)) == Some(&0) {
            e -= 1;
        }
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
}

impl MediaCodecEncoder {
    pub fn new(w: u32, h: u32, rate: FrameRate, s: &ExportSettings) -> std::result::Result<Self, String> {
        let timescale = u32::try_from(rate.num).ok().filter(|n| *n > 0).ok_or("invalid frame rate numerator")?;
        let duration = u32::try_from(rate.den).ok().filter(|d| *d > 0).ok_or("invalid frame rate denominator")?;
        let fps = f64::from(timescale) / f64::from(duration);
        let kbps = s.bitrate_kbps.max(100);
        let key_interval_sec = s.keyframe_distance
            .filter(|k| *k > 0)
            .map(|k| (f64::from(k) / fps).round().max(1.0) as i32)
            .unwrap_or(2);
        let cbr = s.bitrate_mode == BitrateMode::Cbr;

        let make_format = |surface_mode: bool, with_operating_rate: bool| {
            let mut format = MediaFormat::new();
            format.set_str("mime", "video/avc");
            format.set_i32("width", w as i32);
            format.set_i32("height", h as i32);
            // COLOR_FormatSurface = 0x7F000789 (2130708361) or COLOR_FormatYUV420SemiPlanar = 21
            let color_fmt = if surface_mode { 0x7F000789 } else { 21 };
            format.set_i32("color-format", color_fmt);
            format.set_i32("bitrate", (kbps.saturating_mul(1000)) as i32);
            format.set_i32("frame-rate", fps.round().max(1.0) as i32);
            format.set_i32("i-frame-interval", key_interval_sec);
            format.set_i32("bitrate-mode", if cbr { 2 } else { 1 });
            if with_operating_rate {
                format.set_i32("priority", 0);
                format.set_f32("operating-rate", 1000.0);
            }
            format
        };

        let try_init = |surface_mode: bool, with_operating_rate: bool| -> std::result::Result<(MediaCodec, Option<NativeWindow>), String> {
            let codec = MediaCodec::from_encoder_type("video/avc")
                .ok_or_else(|| "MediaCodec::from_encoder_type(\"video/avc\") returned None".to_string())?;

            if let Ok(name) = codec.name() {
                log::info!("MediaCodec encoder: {name} (surface_mode={surface_mode})");
                let lower = name.to_ascii_lowercase();
                if lower.starts_with("c2.android.") || lower.starts_with("omx.google.") || lower.contains(".sw.") {
                    return Err(format!("Rejecting software MediaCodec {name} (slower than native CPU encoder)"));
                }
            }

            let format = make_format(surface_mode, with_operating_rate);
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
                Some(win)
            } else {
                None
            };

            codec.start()
                .map_err(|e| format!("AMediaCodec_start failed: {e:?}"))?;
            Ok((codec, native_win))
        };

        // Try surface mode first for zero-copy GPU encoding, falling back to buffer mode if needed
        let (codec, surface) = match try_init(true, true) {
            Ok(res) => {
                log::info!("MediaCodec initialized in zero-copy surface mode with operating rate");
                res
            }
            Err(e1) => {
                log::warn!("MediaCodec surface mode fast init failed ({e1}), retrying with minimal format");
                match try_init(true, false) {
                    Ok(res) => {
                        log::info!("MediaCodec initialized in zero-copy surface mode (minimal format)");
                        res
                    }
                    Err(e2) => {
                        log::warn!("MediaCodec surface mode failed ({e2}), falling back to buffer mode");
                        match try_init(false, true) {
                            Ok(res) => res,
                            Err(e3) => {
                                log::warn!("MediaCodec buffer fast init failed ({e3}), retrying minimal format");
                                try_init(false, false).map_err(|e4| format!("Failed to initialize MediaCodec: {e4}"))?
                            }
                        }
                    }
                }
            }
        };

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
        let make_format = |with_operating_rate: bool| {
            let mut format = MediaFormat::new();
            format.set_str("mime", "video/avc");
            format.set_i32("width", w as i32);
            format.set_i32("height", h as i32);
            format.set_i32("color-format", 21); // COLOR_FormatYUV420SemiPlanar
            format.set_i32("bitrate", (kbps.saturating_mul(1000)) as i32);
            format.set_i32("frame-rate", fps.round().max(1.0) as i32);
            format.set_i32("i-frame-interval", key_interval_sec);
            format.set_i32("bitrate-mode", if cbr { 2 } else { 1 });
            if with_operating_rate {
                format.set_i32("priority", 0);
                format.set_f32("operating-rate", 1000.0);
            }
            format
        };
        let try_buffer = |with_op: bool| -> std::result::Result<MediaCodec, String> {
            let codec = MediaCodec::from_encoder_type("video/avc")
                .ok_or_else(|| "MediaCodec::from_encoder_type failed".to_string())?;
            let fmt = make_format(with_op);
            codec.configure(&fmt, None, MediaCodecDirection::Encoder)
                .map_err(|e| format!("configure failed: {e:?}"))?;
            codec.start()
                .map_err(|e| format!("start failed: {e:?}"))?;
            Ok(codec)
        };
        let new_codec = match try_buffer(true).or_else(|_| try_buffer(false)) {
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
        SampleEntry::avc(AvcConfig::new(sps_list, pps_list, 4), w, h)
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
            self.drain_output(Duration::ZERO, &mut packets)?;

            if self.sps.is_none() || self.pps.is_none() {
                for _ in 0..40 {
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
            for _ in 0..100 {
                if self.eos_seen {
                    break;
                }
                self.drain_output(Duration::from_millis(30), &mut packets)?;
            }
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
    match MediaCodecDecoder::new(info.clone()) {
        Ok(dec) => {
            log::info!("MediaCodec hardware decoder activated for {} video ({})", entry.codec.name(), dec.name());
            Some(Ok(Box::new(HybridDecoder::new(
                Box::new(dec),
                entry.clone(),
                info,
            ))))
        }
        Err(why) => {
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

        let direct_window = filmcraft_codecs::hw::direct_surface().and_then(|ptr| {
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

    fn drain_output(&mut self, mut timeout: Duration, frames: &mut Vec<DecodedFrame>) -> filmcraft_codecs::Result<()> {
        for _ in 0..100 {
            match self.codec.dequeue_output_buffer(timeout) {
                Ok(DequeuedOutputBufferInfoResult::Buffer(output_buffer)) => {
                    let info = output_buffer.info();
                    let flags = info.flags();
                    let pts = info.presentation_time_us();

                    if (flags & 2) == 0 {
                        if self.is_direct_surface {
                            // ZERO-COPY DIRECT SURFACE BRIDGE:
                            // Commit the decoded frame directly to the encoder's input surface.
                            let pts_ns = (pts.max(0) as u64).saturating_mul(1000) as i64;
                            let render_pts_ns = if pts_ns > self.last_render_pts_ns {
                                pts_ns
                            } else {
                                self.last_render_pts_ns + 1_000
                            };
                            self.last_render_pts_ns = render_pts_ns;
                            if let Err(e) = self.codec.release_output_buffer_at_time(output_buffer, render_pts_ns) {
                                log::warn!("MediaCodecDecoder release_output_buffer_at_time failed: {e:?}");
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
                    timeout = Duration::ZERO;
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
            for _ in 0..150 {
                match self.codec.dequeue_input_buffer(Duration::from_millis(2)) {
                    Ok(DequeuedInputBufferResult::Buffer(b)) => break 'outer b,
                    _ => {
                        self.drain_output(Duration::from_millis(5), &mut frames)?;
                    }
                }
            }
            return Err(CodecError::Decode(
                "MediaCodec input buffer dequeue timed out: hardware decoder did not accept new sample within 1.5s".into(),
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

        // 5. Drain any ready output frames non-blocking
        self.drain_output(Duration::ZERO, &mut frames)?;

        Ok(frames)
    }

    fn flush(&mut self) -> Vec<DecodedFrame> {
        let mut frames = Vec::new();

        // 1. Drain pending frames first
        let _ = self.drain_output(Duration::from_millis(20), &mut frames);

        // 2. Queue EOS buffer (flags = 4)
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
                    let _ = self.drain_output(Duration::from_millis(10), &mut frames);
                }
            }
        }

        if !eos_queued {
            log::warn!("Could not queue EOS to AMediaCodec decoder input buffer");
        }

        // 3. Drain until EOS seen or timeout
        for _ in 0..50 {
            if self.eos_seen {
                break;
            }
            let _ = self.drain_output(Duration::from_millis(20), &mut frames);
        }

        self.eos_seen = false;
        self.last_render_pts_ns = 0;
        frames
    }

    fn reset(&mut self) {
        if let Err(e) = self.codec.flush() {
            log::warn!("MediaCodecDecoder reset flush failed: {e:?}");
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

// SAFETY: Android AMediaCodec handle can be safely sent to another thread; MediaCodecDecoder owns the codec and accesses it exclusively via &mut self.
unsafe impl Send for MediaCodecDecoder {}

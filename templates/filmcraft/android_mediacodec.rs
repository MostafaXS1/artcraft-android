//! Android `AMediaCodec` hardware H.264 video encoder behind `filmcraft_export::VideoEncoder`.
//!
//! Uses Android's NDK `libmediandk` to encode video frames using the device's fixed-function
//! hardware video encoder (e.g. Qualcomm Venus/Adreno, MediaTek, Exynos, Google Tensor).
//!
//! Output frames are formatted as length-prefixed NAL units (AVCC) for ISO BMFF (MP4).
//! When the hardware encoder cannot be initialized, is a slow software emulation, or declines
//! the requested settings (e.g. non-square pixels, odd dimensions, HDR), the factory returns `None`
//! and FilmCraft automatically falls back to software encoding.

#![allow(unsafe_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use filmcraft_export::{
    BitrateMode, EncodedPacket, EncoderFrame, ExportError, ExportSettings, Format,
    HardwareEncoding, PrepareAhead, Result, VideoEncoder,
};
use filmcraft_isobmff::{AvcConfig, SampleEntry};
use filmcraft_time::FrameRate;
use ndk::media::media_codec::{
    DequeuedInputBufferResult, DequeuedOutputBufferInfoResult, MediaCodec, MediaCodecDirection,
};
use ndk::media::media_format::MediaFormat;

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

type YuvPlanes = (Vec<u8>, Vec<u8>, Vec<u8>);

pub struct MediaCodecEncoder {
    codec: MediaCodec,
    w: u32,
    h: u32,
    stride: usize,
    slice_height: usize,
    rate: FrameRate,
    timescale: u32,
    duration: u32,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
    eos_seen: bool,
    ready: Arc<Mutex<HashMap<u64, YuvPlanes>>>,
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

        let make_format = |with_operating_rate: bool| {
            let mut format = MediaFormat::new();
            format.set_str("mime", "video/avc");
            format.set_i32("width", w as i32);
            format.set_i32("height", h as i32);
            // COLOR_FormatYUV420SemiPlanar (NV12) = 21
            format.set_i32("color-format", 21);
            format.set_i32("bitrate", (kbps.saturating_mul(1000)) as i32);
            format.set_i32("frame-rate", fps.round().max(1.0) as i32);
            format.set_i32("i-frame-interval", key_interval_sec);
            format.set_i32("bitrate-mode", if s.bitrate_mode == BitrateMode::Cbr { 2 } else { 1 });
            if with_operating_rate {
                format.set_i32("priority", 0);
                format.set_f32("operating-rate", 1000.0);
            }
            format
        };

        let try_init = |with_operating_rate: bool| -> std::result::Result<MediaCodec, String> {
            let codec = MediaCodec::from_encoder_type("video/avc")
                .ok_or_else(|| "MediaCodec::from_encoder_type(\"video/avc\") returned None".to_string())?;

            #[cfg(feature = "api-level-28")]
            if let Ok(name) = codec.name() {
                log::info!("MediaCodec encoder: {name}");
                let lower = name.to_ascii_lowercase();
                if lower.starts_with("c2.android.") || lower.starts_with("omx.google.") || lower.contains(".sw.") {
                    return Err(format!("Rejecting software MediaCodec {name} (slower than native CPU encoder)"));
                }
            }

            let format = make_format(with_operating_rate);
            codec.configure(&format, None, MediaCodecDirection::Encoder)
                .map_err(|e| format!("AMediaCodec_configure failed: {e:?}"))?;
            codec.start()
                .map_err(|e| format!("AMediaCodec_start failed: {e:?}"))?;
            Ok(codec)
        };

        let codec = match try_init(true) {
            Ok(c) => c,
            Err(e1) => {
                log::warn!("MediaCodec fast init failed ({e1}), retrying with minimal format on fresh instance");
                try_init(false).map_err(|e2| format!("Failed to initialize MediaCodec: {e2}"))?
            }
        };

        #[cfg(feature = "api-level-28")]
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
        #[cfg(not(feature = "api-level-28"))]
        let (stride, slice_height) = (w as usize, h as usize);

        Ok(Self {
            codec,
            w,
            h,
            stride,
            slice_height,
            rate,
            timescale,
            duration,
            y: Vec::new(),
            u: Vec::new(),
            v: Vec::new(),
            sps: None,
            pps: None,
            eos_seen: false,
            ready: Arc::new(Mutex::new(HashMap::new())),
        })
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
        let ready = self.ready.clone();
        Some(Arc::new(move |f: &EncoderFrame| {
            let (w, h) = (f.width as usize, f.height as usize);
            let mut y = Vec::new();
            let mut u = Vec::new();
            let mut v = Vec::new();
            filmcraft_export::timed(filmcraft_export::Stage::Convert, || {
                filmcraft_export::rgba_to_yuv420_8(f.rgba, w, h, &mut y, &mut u, &mut v);
            });
            ready.lock().unwrap_or_else(PoisonError::into_inner).insert(f.index, (y, u, v));
        }))
    }

    fn encode(&mut self, f: &EncoderFrame) -> Result<Vec<EncodedPacket>> {
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

        // 1. Color conversion: retrieve pre-converted YUV planes or convert if missing
        let ready_planes = {
            let mut ready = self.ready.lock().unwrap_or_else(PoisonError::into_inner);
            let mine = ready.remove(&f.index);
            ready.retain(|&i, _| i > f.index);
            mine
        };
        let chroma = w.div_ceil(2) * h.div_ceil(2);
        match ready_planes.filter(|(y, u, v)| y.len() == w * h && u.len() == chroma && v.len() == chroma) {
            Some((y, u, v)) => {
                self.y = y;
                self.u = u;
                self.v = v;
            }
            None => {
                filmcraft_export::timed(filmcraft_export::Stage::Convert, || {
                    filmcraft_export::rgba_to_yuv420_8(f.rgba, w, h, &mut self.y, &mut self.u, &mut self.v);
                });
            }
        }

        let mut packets = Vec::new();

        // 2. Drain any pending output buffers immediately without waiting
        self.drain_output(Duration::ZERO, &mut packets)?;

        // 3. Dequeue an input buffer: try non-blocking first.
        // If the encoder's input queue is full, drain output buffers in a loop with a generous timeout.
        let mut input_buf = match self.codec.dequeue_input_buffer(Duration::ZERO) {
            Ok(DequeuedInputBufferResult::Buffer(b)) => b,
            _ => {
                let mut acquired = None;
                // Wait up to 5 seconds (100 iterations of 50ms), draining output continuously
                for _ in 0..100 {
                    self.drain_output(Duration::from_millis(50), &mut packets)?;
                    match self.codec.dequeue_input_buffer(Duration::ZERO) {
                        Ok(DequeuedInputBufferResult::Buffer(b)) => {
                            acquired = Some(b);
                            break;
                        }
                        _ => continue,
                    }
                }
                match acquired {
                    Some(b) => b,
                    None => return Err(ExportError::Encode("MediaCodec input buffer dequeue timed out: hardware encoder did not accept new frame within 5s".into())),
                }
            }
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
            let y_len = w * h;
            let (y_dst, uv_dst) = dst.split_at_mut(y_len);
            unsafe {
                std::ptr::copy_nonoverlapping(self.y.as_ptr(), y_dst.as_mut_ptr() as *mut u8, y_len);
            }
            for ((pair, &u_val), &v_val) in uv_dst.as_chunks_mut::<2>().0.iter_mut().zip(self.u.iter()).zip(self.v.iter()) {
                pair[0].write(u_val);
                pair[1].write(v_val);
            }
        } else {
            // Strided copy for chips with alignment requirements (e.g. 16/32-byte alignment)
            for row in 0..h {
                let src_offset = row * w;
                let dst_offset = row * stride;
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        self.y.as_ptr().add(src_offset),
                        dst.as_mut_ptr().add(dst_offset) as *mut u8,
                        w,
                    );
                }
            }
            let uv_base = y_size;
            let uv_w = w.div_ceil(2);
            for row in 0..uv_height {
                let src_offset = row * uv_w;
                let dst_row = uv_base + row * stride;
                let row_slice = &mut dst[dst_row..dst_row + uv_w * 2];
                for ((pair, &u_val), &v_val) in row_slice.as_chunks_mut::<2>().0.iter_mut().zip(&self.u[src_offset..src_offset + uv_w]).zip(&self.v[src_offset..src_offset + uv_w]) {
                    pair[0].write(u_val);
                    pair[1].write(v_val);
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

        // 1. Drain pending frames first so an input buffer is freed
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

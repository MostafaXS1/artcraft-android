//! Android `AMediaCodec` hardware H.264 video encoder behind `filmcraft_export::VideoEncoder`.
//!
//! Uses Android's NDK `libmediandk` to encode video frames using the device's fixed-function
//! hardware video encoder (e.g. Qualcomm Venus/Adreno, MediaTek, Exynos, Google Tensor).
//!
//! Output frames are formatted as length-prefixed NAL units (AVCC) for ISO BMFF (MP4).
//! When the hardware encoder cannot be initialized or declines the requested settings
//! (e.g. non-square pixels, odd dimensions, HDR), the factory returns `None` and FilmCraft
//! automatically falls back to software encoding.

use std::time::Duration;

use filmcraft_export::{
    BitrateMode, EncodedPacket, EncoderFrame, ExportError, ExportSettings, Format,
    H264Profile, HardwareEncoding, Result, VideoEncoder,
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
        let mut e = starts.get(k + 1).copied().unwrap_or(data.len());
        if e >= 3 && e > s {
            e -= 3;
        }
        // Trailing zero bytes belong to the next start code (4-byte form) or are padding
        while e > s && data.get(e - 1) == Some(&0) {
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
            filmcraft_export::note_hw_encode_session();
            Some(Ok(Box::new(enc)))
        }
        Err(why) => {
            log::info!("MediaCodec hardware encoding declined: {why}");
            filmcraft_export::note_hw_encode_declined();
            None
        }
    }
}

pub struct MediaCodecEncoder {
    codec: MediaCodec,
    w: u32,
    h: u32,
    rate: FrameRate,
    timescale: u32,
    duration: u32,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
    nv12: Vec<u8>,
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
    eos_seen: bool,
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

        let codec = MediaCodec::from_encoder_type("video/avc")
            .ok_or_else(|| "MediaCodec::from_encoder_type(\"video/avc\") returned None".to_string())?;

        let mut format = MediaFormat::new();
        format.set_str("mime", "video/avc");
        format.set_i32("width", w as i32);
        format.set_i32("height", h as i32);
        // COLOR_FormatYUV420SemiPlanar (NV12) = 21
        format.set_i32("color-format", 21);
        format.set_i32("bitrate", (kbps.saturating_mul(1000)) as i32);
        format.set_i32("frame-rate", fps.round().max(1.0) as i32);
        format.set_i32("i-frame-interval", key_interval_sec);
        // BitrateMode: 1 = VBR, 2 = CBR
        format.set_i32("bitrate-mode", if s.bitrate_mode == BitrateMode::Cbr { 2 } else { 1 });
        // AVC Profile: Baseline = 1, Main = 2, High = 8
        let profile = match s.h264_profile {
            H264Profile::Baseline => 1,
            H264Profile::Main => 2,
            H264Profile::High => 8,
        };
        format.set_i32("profile", profile);

        codec.configure(&format, None, MediaCodecDirection::Encoder)
            .map_err(|e| format!("AMediaCodec_configure failed: {e:?}"))?;
        codec.start()
            .map_err(|e| format!("AMediaCodec_start failed: {e:?}"))?;

        let total_yuv = (w as usize).saturating_mul(h as usize).saturating_mul(3) / 2;

        Ok(Self {
            codec,
            w,
            h,
            rate,
            timescale,
            duration,
            y: Vec::new(),
            u: Vec::new(),
            v: Vec::new(),
            nv12: vec![0u8; total_yuv],
            sps: None,
            pps: None,
            eos_seen: false,
        })
    }

    fn extract_sps_pps(&mut self, data: &[u8]) {
        for nal in annex_b_nals(data) {
            match nal.first().map(|b| b & 0x1f) {
                Some(7) if self.sps.is_none() => self.sps = Some(nal.to_vec()),
                Some(8) if self.pps.is_none() => self.pps = Some(nal.to_vec()),
                _ => {}
            }
        }
    }

    fn drain_output(&mut self, timeout: Duration, packets: &mut Vec<EncodedPacket>) -> Result<()> {
        loop {
            match self.codec.dequeue_output_buffer(timeout) {
                Ok(DequeuedOutputBufferInfoResult::Buffer(output_buffer)) => {
                    let info = *output_buffer.info();
                    let idx = output_buffer.index;
                    drop(output_buffer);

                    let flags = info.flags();
                    let offset = info.offset().max(0) as usize;
                    let size = info.size().max(0) as usize;

                    if size > 0 {
                        if let Some(raw) = self.codec.output_buffer(idx) {
                            if let Some(slice) = raw.get(offset..offset.saturating_add(size)) {
                                if (flags & 2) != 0 {
                                    // BUFFER_FLAG_CODEC_CONFIG = 2
                                    self.extract_sps_pps(slice);
                                } else {
                                    if self.sps.is_none() || self.pps.is_none() {
                                        self.extract_sps_pps(slice);
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
                    }

                    let _ = self.codec.release_output_buffer_by_index(idx, false);
                    if (flags & 4) != 0 {
                        // BUFFER_FLAG_END_OF_STREAM = 4
                        self.eos_seen = true;
                        break;
                    }
                }
                Ok(DequeuedOutputBufferInfoResult::OutputFormatChanged) => {
                    let fmt = self.codec.output_format();
                    if self.sps.is_none() {
                        if let Some(sps) = fmt.buffer("csd-0") {
                            self.sps = Some(sps.to_vec());
                        }
                    }
                    if self.pps.is_none() {
                        if let Some(pps) = fmt.buffer("csd-1") {
                            self.pps = Some(pps.to_vec());
                        }
                    }
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
        let sps = self.sps.clone().unwrap_or_default();
        let pps = self.pps.clone().unwrap_or_default();
        SampleEntry::avc(AvcConfig::new(vec![sps], vec![pps], 4), w, h)
    }

    fn timescale(&self) -> u32 {
        self.timescale
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

        // 1. Color conversion: RGBA -> Planar YUV420
        filmcraft_export::timed(filmcraft_export::Stage::Convert, || {
            filmcraft_export::rgba_to_yuv420_8(f.rgba, w, h, &mut self.y, &mut self.u, &mut self.v);
        });

        // 2. Interleave Planar YUV420 into NV12 (Y plane, then interleaved UV)
        let y_len = w.saturating_mul(h);
        let uv_len = y_len / 2;
        let total_len = y_len.saturating_add(uv_len);
        if self.nv12.len() != total_len {
            self.nv12.resize(total_len, 0);
        }
        self.nv12[..y_len].copy_from_slice(&self.y);
        let uv_dst = &mut self.nv12[y_len..];
        for (i, (&u_val, &v_val)) in self.u.iter().zip(self.v.iter()).enumerate() {
            if let Some(pair) = uv_dst.get_mut(i * 2..i * 2 + 2) {
                pair[0] = u_val;
                pair[1] = v_val;
            }
        }

        let mut packets = Vec::new();

        // 3. Drain any pending output buffers without waiting
        self.drain_output(Duration::ZERO, &mut packets)?;

        // 4. Dequeue an input buffer (with short retry if needed)
        let buf_idx = match self.codec.dequeue_input_buffer(Duration::from_millis(50)) {
            Ok(DequeuedInputBufferResult::Buffer(b)) => b.index,
            _ => {
                self.drain_output(Duration::from_millis(50), &mut packets)?;
                match self.codec.dequeue_input_buffer(Duration::from_millis(100)) {
                    Ok(DequeuedInputBufferResult::Buffer(b)) => b.index,
                    _ => return Err(ExportError::Encode("MediaCodec input buffer dequeue timed out".into())),
                }
            }
        };

        // 5. Copy NV12 bytes into the input buffer safely
        if let Some(dst) = self.codec.input_buffer(buf_idx) {
            for (d, s) in dst.iter_mut().zip(self.nv12.iter()) {
                d.write(*s);
            }
        } else {
            return Err(ExportError::Encode("MediaCodec input_buffer returned NULL".into()));
        }

        // 6. Calculate PTS in microseconds
        let pts_us = (f.index as u128)
            .saturating_mul(1_000_000)
            .saturating_mul(self.rate.den as u128)
            .checked_div(self.rate.num as u128)
            .unwrap_or(0) as u64;

        self.codec
            .queue_input_buffer_by_index(buf_idx, 0, total_len, pts_us, 0)
            .map_err(|e| ExportError::Encode(format!("queue_input_buffer failed: {e:?}")))?;

        // 7. Drain output buffers ready from this or previous frames
        self.drain_output(Duration::ZERO, &mut packets)?;
        filmcraft_export::note_hw_encode_frame();

        Ok(packets)
    }

    fn flush(&mut self) -> Result<Vec<EncodedPacket>> {
        let mut packets = Vec::new();

        // Queue EOS buffer (size = 0, flag = 4)
        if let Ok(DequeuedInputBufferResult::Buffer(b)) = self.codec.dequeue_input_buffer(Duration::from_millis(50)) {
            let idx = b.index;
            let _ = self.codec.queue_input_buffer_by_index(idx, 0, 0, 0, 4);
        }

        // Drain until EOS seen or timeout loop ends (50 * 50ms = 2.5s maximum drain window)
        for _ in 0..50 {
            if self.eos_seen {
                break;
            }
            self.drain_output(Duration::from_millis(50), &mut packets)?;
        }

        Ok(packets)
    }
}

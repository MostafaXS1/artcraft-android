//! Android `AMediaCodec` hardware H.264 video encoder for EffectCraft.
#![allow(unsafe_code)]

use std::time::Duration;

use filmcraft_isobmff::{AvcConfig, SampleEntry};
use effectcraft_time::FrameRate;
use ndk::media::media_codec::{
    DequeuedInputBufferResult, DequeuedOutputBufferInfoResult, MediaCodec, MediaCodecDirection,
};
use ndk::media::media_format::MediaFormat;

use crate::encode::{Codes, Packet, VideoEncoder, tag_nclx_limited};
use crate::{ExportError, Result};

pub struct AndroidH264 {
    codec: MediaCodec,
    codes: Codes,
    w: u32,
    h: u32,
    stride: usize,
    slice_height: usize,
    rate: FrameRate,
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
    eos_seen: bool,
    nv12: Vec<u8>,
}

fn annex_b_to_avcc(annex_b: &[u8], out: &mut Vec<u8>) -> std::result::Result<(), String> {
    out.clear();
    let mut nals = Vec::new();
    let mut i = 0;
    while i < annex_b.len() {
        if annex_b[i..].starts_with(&[0, 0, 0, 1]) {
            let start = i + 4;
            let next = annex_b[start..]
                .windows(4)
                .position(|w| w == [0, 0, 0, 1])
                .map(|p| start + p)
                .unwrap_or(annex_b.len());
            nals.push(&annex_b[start..next]);
            i = next;
        } else if annex_b[i..].starts_with(&[0, 0, 1]) {
            let start = i + 3;
            let next = annex_b[start..]
                .windows(3)
                .position(|w| w == [0, 0, 1])
                .map(|p| start + p)
                .unwrap_or(annex_b.len());
            nals.push(&annex_b[start..next]);
            i = next;
        } else {
            i += 1;
        }
    }
    for n in nals {
        let nal_type = n.first().map(|b| b & 0x1f);
        if matches!(nal_type, Some(7..=9) | None) {
            continue;
        }
        let len = u32::try_from(n.len()).map_err(|_| "NAL unit too large".to_string())?;
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(n);
    }
    Ok(())
}

impl AndroidH264 {
    pub fn new(w: u32, h: u32, rate: FrameRate, kbps: u32, codes: Codes) -> Result<Self> {
        let timescale = u32::try_from(rate.num).ok().filter(|n| *n > 0).ok_or_else(|| ExportError::Encode("invalid frame rate numerator".into()))?;
        let duration = u32::try_from(rate.den).ok().filter(|d| *d > 0).ok_or_else(|| ExportError::Encode("invalid frame rate denominator".into()))?;
        let fps = f64::from(timescale) / f64::from(duration);
        let kbps = kbps.max(100);
        let key_interval_sec = (f64::from(rate.as_f64() * 2.0) / fps).round().max(1.0) as i32;

        let mut format = MediaFormat::new();
        format.set_str("mime", "video/avc");
        format.set_i32("width", w as i32);
        format.set_i32("height", h as i32);
        format.set_i32("color-format", 21); // COLOR_FormatYUV420SemiPlanar (NV12)
        format.set_i32("bitrate", (kbps.saturating_mul(1000)) as i32);
        format.set_i32("frame-rate", fps.round().max(1.0) as i32);
        format.set_i32("i-frame-interval", key_interval_sec);
        format.set_i32("bitrate-mode", 1); // VBR = 1

        let codec = MediaCodec::from_encoder_type("video/avc")
            .ok_or_else(|| ExportError::Encode("MediaCodec::from_encoder_type(\"video/avc\") returned None".into()))?;

        codec.configure(&format, None, MediaCodecDirection::Encoder)
            .map_err(|e| ExportError::Encode(format!("AMediaCodec_configure encoder failed: {e:?}")))?;

        codec.start()
            .map_err(|e| ExportError::Encode(format!("AMediaCodec_start encoder failed: {e:?}")))?;

        let mut enc = Self {
            codec,
            codes,
            w,
            h,
            stride: w as usize,
            slice_height: h as usize,
            rate,
            sps: None,
            pps: None,
            eos_seen: false,
            nv12: Vec::new(),
        };

        let out_fmt = enc.codec.output_format();
        enc.update_format(&out_fmt);
        Ok(enc)
    }

    fn update_format(&mut self, fmt: &MediaFormat) {
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
        if let Some(csd0) = fmt.buffer("csd-0") {
            let mut i = 0;
            while i + 4 <= csd0.len() {
                if csd0[i..].starts_with(&[0, 0, 0, 1]) {
                    let start = i + 4;
                    let next = csd0[start..]
                        .windows(4)
                        .position(|w| w == [0, 0, 0, 1])
                        .map(|p| start + p)
                        .unwrap_or(csd0.len());
                    let nal = &csd0[start..next];
                    if let Some(&hdr) = nal.first() {
                        let nal_type = hdr & 0x1f;
                        if nal_type == 7 && self.sps.is_none() {
                            self.sps = Some(nal.to_vec());
                        } else if nal_type == 8 && self.pps.is_none() {
                            self.pps = Some(nal.to_vec());
                        }
                    }
                    i = next;
                } else {
                    i += 1;
                }
            }
        }
        if let Some(csd1) = fmt.buffer("csd-1") {
            if self.pps.is_none() {
                let mut nal = csd1;
                if nal.starts_with(&[0, 0, 0, 1]) {
                    nal = &nal[4..];
                }
                self.pps = Some(nal.to_vec());
            }
        }
    }

    fn drain_output(&mut self, mut timeout: Duration, packets: &mut Vec<Packet>) -> Result<()> {
        let mut avcc_buf = Vec::new();
        for _ in 0..100 {
            match self.codec.dequeue_output_buffer(timeout) {
                Ok(DequeuedOutputBufferInfoResult::Buffer(output_buffer)) => {
                    let info = output_buffer.info();
                    let flags = info.flags();

                    if (flags & 2) != 0 {
                        let slice = output_buffer.buffer();
                        let mut i = 0;
                        while i + 4 <= slice.len() {
                            if slice[i..].starts_with(&[0, 0, 0, 1]) {
                                let start = i + 4;
                                let next = slice[start..]
                                    .windows(4)
                                    .position(|w| w == [0, 0, 0, 1])
                                    .map(|p| start + p)
                                    .unwrap_or(slice.len());
                                let nal = &slice[start..next];
                                if let Some(&hdr) = nal.first() {
                                    let nal_type = hdr & 0x1f;
                                    if nal_type == 7 && self.sps.is_none() {
                                        self.sps = Some(nal.to_vec());
                                    } else if nal_type == 8 && self.pps.is_none() {
                                        self.pps = Some(nal.to_vec());
                                    }
                                }
                                i = next;
                            } else {
                                i += 1;
                            }
                        }
                    } else if info.size() > 0 {
                        let is_key = (flags & 1) != 0;
                        let slice = output_buffer.buffer();
                        if annex_b_to_avcc(slice, &mut avcc_buf).is_ok() && !avcc_buf.is_empty() {
                            packets.push(Packet {
                                data: avcc_buf.clone(),
                                key: is_key,
                                cto: 0,
                            });
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
                    return Err(ExportError::Encode(format!("dequeue_output_buffer error: {e:?}")));
                }
            }
        }
        Ok(())
    }

    fn rgba_to_nv12_into(&mut self, rgba: &[u8]) {
        let (w, h) = (self.w as usize, self.h as usize);
        let stride = self.stride.max(w);
        let slice_h = self.slice_height.max(h);
        let y_len = stride * slice_h;
        let uv_len = stride * (h.div_ceil(2));
        let total = y_len + uv_len;

        if self.nv12.len() < total {
            self.nv12.resize(total, 0);
        }

        let (y_plane, uv_plane) = self.nv12.split_at_mut(y_len);

        for row in 0..h {
            let row_src = &rgba[row * w * 4..(row + 1) * w * 4];
            let row_dst = &mut y_plane[row * stride..row * stride + w];
            for (px, y) in row_src.chunks_exact(4).zip(row_dst.iter_mut()) {
                let r = px[0] as i32;
                let g = px[1] as i32;
                let b = px[2] as i32;
                *y = (16 + ((54 * r + 183 * g + 18 * b + 128) >> 8)).clamp(16, 235) as u8;
            }
        }

        let half_h = h / 2;
        let half_w = w / 2;
        for row in 0..half_h {
            let row0_src = &rgba[row * 2 * w * 4..(row * 2 + 1) * w * 4];
            let row1_src = &rgba[(row * 2 + 1) * w * 4..(row * 2 + 2) * w * 4];
            let row_dst = &mut uv_plane[row * stride..row * stride + w];

            for (col, uv) in row_dst.chunks_exact_mut(2).enumerate().take(half_w) {
                let px00 = &row0_src[col * 8..col * 8 + 4];
                let px01 = &row0_src[col * 8 + 4..col * 8 + 8];
                let px10 = &row1_src[col * 8..col * 8 + 4];
                let px11 = &row1_src[col * 8 + 4..col * 8 + 8];

                let r_avg = (px00[0] as i32 + px01[0] as i32 + px10[0] as i32 + px11[0] as i32 + 2) >> 2;
                let g_avg = (px00[1] as i32 + px01[1] as i32 + px10[1] as i32 + px11[1] as i32 + 2) >> 2;
                let b_avg = (px00[2] as i32 + px01[2] as i32 + px10[2] as i32 + px11[2] as i32 + 2) >> 2;

                uv[0] = (128 + ((-29 * r_avg - 99 * g_avg + 128 * b_avg + 128) >> 8)).clamp(16, 240) as u8;
                uv[1] = (128 + ((128 * r_avg - 116 * g_avg - 12 * b_avg + 128) >> 8)).clamp(16, 240) as u8;
            }
        }
    }
}

impl VideoEncoder for AndroidH264 {
    fn sample_entry(&self) -> SampleEntry {
        let sps = self.sps.clone().unwrap_or_else(|| vec![0x67, 0x42, 0xc0, 0x1e, 0xd9, 0x01, 0x41, 0xfb, 0x01, 0x10, 0x00, 0x00, 0x03, 0x00, 0x10, 0x00, 0x00, 0x03, 0x03, 0x20, 0xf1, 0x62, 0xe4, 0x80]);
        let pps = self.pps.clone().unwrap_or_else(|| vec![0x68, 0xce, 0x38, 0x80]);
        let cfg = AvcConfig::new(vec![sps], vec![pps], 4);
        let mut entry = SampleEntry::avc(cfg, self.w as u16, self.h as u16);
        tag_nclx_limited(&mut entry, self.codes);
        entry
    }

    fn encode(&mut self, rgba: &[u8], index: u64) -> Result<Vec<Packet>> {
        self.rgba_to_nv12_into(rgba);
        let mut packets = Vec::new();
        self.drain_output(Duration::ZERO, &mut packets)?;

        let mut input_buf = loop {
            if let Ok(DequeuedInputBufferResult::Buffer(b)) = self.codec.dequeue_input_buffer(Duration::ZERO) {
                break b;
            }
            match self.codec.dequeue_input_buffer(Duration::from_millis(2)) {
                Ok(DequeuedInputBufferResult::Buffer(b)) => break b,
                _ => {
                    self.drain_output(Duration::from_millis(2), &mut packets)?;
                }
            }
        };

        let dst_raw = input_buf.buffer_mut().as_mut_ptr() as *mut u8;
        let dst_len = input_buf.buffer_mut().len();
        let copy_len = self.nv12.len().min(dst_len);
        unsafe {
            std::ptr::copy_nonoverlapping(self.nv12.as_ptr(), dst_raw, copy_len);
        }

        let pts_us = (index as u128)
            .saturating_mul(1_000_000)
            .saturating_mul(self.rate.den as u128)
            .checked_div(self.rate.num as u128)
            .unwrap_or(0) as u64;
        let _ = self.codec.queue_input_buffer(input_buf, 0, copy_len, pts_us, 0);

        self.drain_output(Duration::ZERO, &mut packets)?;
        Ok(packets)
    }

    fn flush(&mut self) -> Result<Vec<Packet>> {
        let mut packets = Vec::new();
        if !self.eos_seen {
            if let Ok(DequeuedInputBufferResult::Buffer(b)) = self.codec.dequeue_input_buffer(Duration::from_millis(10)) {
                let _ = self.codec.queue_input_buffer(b, 0, 0, 0, 4); // EOS
            }
            let _ = self.drain_output(Duration::from_millis(50), &mut packets);
        }
        Ok(packets)
    }
}

unsafe impl Send for AndroidH264 {}

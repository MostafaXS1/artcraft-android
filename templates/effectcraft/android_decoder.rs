//! Android `AMediaCodec` hardware H.264 / HEVC video decoder for EffectCraft.
#![allow(unsafe_code)]

use std::sync::Arc;
use std::time::Duration;

use filmcraft_codecs::hw::{NalCodec, NalStreamInfo};
use filmcraft_codecs::{CodecError, DecodedFrame, VideoDecoder};
use filmcraft_frame::{Chroma, PixelData, VideoFrame};
use filmcraft_isobmff::SampleEntry;
use ndk::media::media_codec::{
    DequeuedInputBufferResult, DequeuedOutputBufferInfoResult, MediaCodec, MediaCodecDirection,
};
use ndk::media::media_format::MediaFormat;

fn avcc_to_annex_b_append(avcc: &[u8], length_size: usize, out: &mut Vec<u8>) {
    let len_sz = length_size;
    if len_sz == 0 || len_sz > 4 {
        return;
    }
    let mut i = 0;
    while i + len_sz <= avcc.len() {
        let mut nal_len = 0usize;
        for &b in &avcc[i..i + len_sz] {
            nal_len = (nal_len << 8) | (b as usize);
        }
        i += len_sz;
        if i + nal_len > avcc.len() {
            break;
        }
        let nal = &avcc[i..i + nal_len];
        i += nal_len;
        if nal.is_empty() {
            continue;
        }
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(nal);
    }
}

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
}

impl MediaCodecDecoder {
    pub fn new(info: NalStreamInfo) -> Result<Self, String> {
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

        let make_format = |with_operating_rate: bool, with_color_format: bool, with_csd: bool| {
            let mut format = MediaFormat::new();
            format.set_str("mime", mime);
            format.set_i32("width", w as i32);
            format.set_i32("height", h as i32);
            if with_color_format {
                format.set_i32("color-format", 21); // COLOR_FormatYUV420SemiPlanar (NV12)
            }
            if with_operating_rate {
                format.set_i32("priority", 0);
                format.set_f32("operating-rate", 1000.0);
                format.set_i32("low-latency", 1);
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

        let try_init = |with_operating_rate: bool, with_color_format: bool, with_csd: bool| -> Result<MediaCodec, String> {
            let codec = MediaCodec::from_decoder_type(mime)
                .ok_or_else(|| format!("MediaCodec::from_decoder_type(\"{mime}\") returned None"))?;

            let format = make_format(with_operating_rate, with_color_format, with_csd);
            codec.configure(&format, None, MediaCodecDirection::Decoder)
                .map_err(|e| format!("AMediaCodec_configure decoder failed: {e:?}"))?;
            codec.start()
                .map_err(|e| format!("AMediaCodec_start decoder failed: {e:?}"))?;
            Ok(codec)
        };

        let codec = try_init(true, true, true)
            .or_else(|_| try_init(false, true, true))
            .or_else(|_| try_init(false, false, true))
            .or_else(|_| try_init(false, false, false))
            .map_err(|e| format!("Failed to initialize MediaCodec decoder: {e}"))?;

        let name = default_name.to_string();
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
    }

    fn drain_output(&mut self, mut timeout: Duration, frames: &mut Vec<DecodedFrame>) -> filmcraft_codecs::Result<()> {
        for _ in 0..100 {
            match self.codec.dequeue_output_buffer(timeout) {
                Ok(DequeuedOutputBufferInfoResult::Buffer(output_buffer)) => {
                    let info = output_buffer.info();
                    let flags = info.flags();
                    let pts = info.presentation_time_us();

                    if (flags & 2) == 0 {
                        let out_slice = output_buffer.buffer();
                        let (cw, ch) = (self.crop.2 as usize, self.crop.3 as usize);
                        let (ox, oy) = (self.crop.0 as usize, self.crop.1 as usize);
                        let stride = self.stride.max(cw + ox);
                        let slice_h = self.slice_height.max(ch + oy);

                        let mut y_plane = Vec::with_capacity(cw * ch);
                        let mut u_plane = Vec::with_capacity((cw.div_ceil(2)) * (ch.div_ceil(2)));
                        let mut v_plane = Vec::with_capacity((cw.div_ceil(2)) * (ch.div_ceil(2)));

                        let luma_start = oy * stride + ox;
                        for row in 0..ch {
                            let r_start = luma_start + row * stride;
                            if let Some(src) = out_slice.get(r_start..r_start + cw) {
                                y_plane.extend_from_slice(src);
                            } else {
                                y_plane.resize(cw * ch, 16);
                                break;
                            }
                        }

                        let uv_base = stride * slice_h;
                        let half_cw = cw.div_ceil(2);
                        let half_ch = ch.div_ceil(2);
                        let (half_ox, half_oy) = (ox / 2, oy / 2);

                        for r in 0..half_ch {
                            let uv_row_start = uv_base + (half_oy + r) * stride + (half_ox * 2);
                            for c in 0..half_cw {
                                let idx = uv_row_start + c * 2;
                                if idx + 1 < out_slice.len() {
                                    u_plane.push(out_slice[idx]);
                                    v_plane.push(out_slice[idx + 1]);
                                } else {
                                    u_plane.push(128);
                                    v_plane.push(128);
                                }
                            }
                        }

                        let data = PixelData::Yuv8 {
                            planes: [Arc::new(y_plane), Arc::new(u_plane), Arc::new(v_plane)],
                            chroma: Chroma::C420,
                            alpha: None,
                        };

                        frames.push(DecodedFrame {
                            pts,
                            frame: VideoFrame {
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

        let mut input_buf = 'outer: loop {
            if let Ok(DequeuedInputBufferResult::Buffer(b)) = self.codec.dequeue_input_buffer(Duration::ZERO) {
                break 'outer b;
            }
            match self.codec.dequeue_input_buffer(Duration::from_millis(4)) {
                Ok(DequeuedInputBufferResult::Buffer(b)) => break 'outer b,
                _ => {
                    self.drain_output(Duration::from_millis(2), &mut frames)?;
                }
            }
        };

        let dst_raw = input_buf.buffer_mut().as_mut_ptr() as *mut u8;
        let dst_len = input_buf.buffer_mut().len();
        let copy_len = self.input_annex_b.len().min(dst_len);
        unsafe {
            std::ptr::copy_nonoverlapping(self.input_annex_b.as_ptr(), dst_raw, copy_len);
        }

        let pts_us = (pts.max(0)) as u64;
        let _ = self.codec.queue_input_buffer(input_buf, 0, copy_len, pts_us, 0);

        self.drain_output(Duration::ZERO, &mut frames)?;
        filmcraft_codecs::hw::note_hw_frames(frames.len());
        Ok(frames)
    }

    fn flush(&mut self) -> Vec<DecodedFrame> {
        let mut frames = Vec::new();
        if !self.eos_seen {
            if let Ok(DequeuedInputBufferResult::Buffer(b)) = self.codec.dequeue_input_buffer(Duration::from_millis(10)) {
                let _ = self.codec.queue_input_buffer(b, 0, 0, 0, 4); // BUFFER_FLAG_END_OF_STREAM = 4
            }
            let _ = self.drain_output(Duration::from_millis(20), &mut frames);
            filmcraft_codecs::hw::note_hw_frames(frames.len());
        }
        frames
    }

    fn reset(&mut self) {
        let _ = self.codec.flush();
        self.eos_seen = false;
        self.need_param_sets = true;
    }

    fn name(&self) -> &str {
        &self.name
    }
}

unsafe impl Send for MediaCodecDecoder {}

pub fn decoder_factory(
    entry: &SampleEntry,
) -> Option<filmcraft_codecs::Result<Box<dyn VideoDecoder>>> {
    if !filmcraft_codecs::hw::hardware_decoding() {
        return None;
    }
    let info = match NalStreamInfo::from_entry(entry) {
        Some(Ok(info)) => info,
        _ => return None,
    };
    match MediaCodecDecoder::new(info) {
        Ok(dec) => {
            filmcraft_codecs::hw::note_hw_session();
            Some(Ok(Box::new(dec)))
        }
        Err(why) => {
            log::warn!("MediaCodec hardware decoding declined: {why}");
            filmcraft_codecs::hw::note_hw_declined();
            None
        }
    }
}

pub fn register() {
    filmcraft_codecs::register_video_decoder(decoder_factory);
    filmcraft_codecs::hw::set_hw_backend("MediaCodec");
}

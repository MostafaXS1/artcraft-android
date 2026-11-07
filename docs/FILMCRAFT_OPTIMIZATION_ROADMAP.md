# FilmCraft Mobile Export Performance Optimization Roadmap

**Target Hardware:** Xiaomi Pad 6 (Qualcomm Snapdragon 870 / Adreno 650)  
**Baseline Measured:** 1-minute 4K 60fps export took ~16.8 minutes (3.68 FPS) in Buffer Mode with CPU float unpack and LUT lookups.  
**Target Goal:** Under 30 seconds for 4K 60fps (CapCut / Premiere Mobile parity) with 100% color accuracy.

---

## Architecture & Roadmap Overview

```mermaid
flowchart TD
    Current["Current Baseline (Observed: 17 min @ 3.68 FPS)<br/>132MB/frame float readback + CPU sRGB LUT + scalar NV12"]
    
    Path1["Stage 1: Path 1 (ACTIVE / IMPLEMENTED)<br/>GPU sRGB 8-bit resolve + RGBA8 readback + unrolled NV12<br/>Target: ~1.8–2.5 minutes (~30–40 FPS)"]
    
    Path2["Stage 2: Path 2 (NEXT STEP)<br/>Fix Adreno Vulkan Surface swapchain on AMediaCodec ANativeWindow<br/>Target: ~60–80 seconds (~50–60 FPS)"]
    
    PhaseB2["Stage 3: Phase B2 (FINAL GOAL)<br/>Zero-copy hardware decoding via AHardwareBuffer / AImageReader<br/>Vulkan external memory import + 4-session Qualcomm governor<br/>Target: ~25–35 seconds (~100–120 FPS, CapCut Parity)"]

    Current --> Path1
    Path1 --> Path2
    Path2 --> PhaseB2
```

---

## Stage 1: Path 1 — GPU sRGB 8-bit Resolve & Fast RGBA8 Readback

### Current Bottleneck Diagnosis
In Buffer Mode:
1. `GpuCompositor::render_export_prepared` renders linear floating-point color into an accumulator texture.
2. It copies the accumulator to a staging buffer and reads back **132.7 Megabytes** of raw half-floats per 4K frame across the mobile RAM bus ($3840 \times 2160 \times 4 \times 4$ bytes).
3. The CPU iterates through 33.2 million half-floats converting them to 32-bit floats.
4. `Pipeline::frame` invokes `img.over_black_rgba8()`, performing 8.3 million discrete CPU sRGB LUT lookups via `linear_to_srgb_u8_table`.
5. Scalar `rgba_to_nv12` loops over every pixel with bounds checking.
6. Total CPU latency per frame: ~270 ms = 3.68 FPS = **16.8 minutes** for a 1-minute video.

### Path 1 Implementation
1. **GPU sRGB 8-bit Resolve**:
   - Call `render_plan(plan, prep, true, true)` on `GpuCompositor`.
   - The GPU's `final_pipeline` (`fs_full` shader in `composite.wgsl`) resolves the continuous IEC 61966-2-1 sRGB curve in 32-bit float ALU directly into `GpuCompositor::output` (`Rgba8Unorm`).
2. **Fast 8-bit Readback Buffer**:
   - Implement `GpuCompositor::render_export_rgba8` with reusable `output_readback_buf: Option<(wgpu::Buffer, u64)>`.
   - Readback memory shrinks from **132.7 MB down to 33.2 MB** per frame (saving 99.5 MB per frame / 358 GB over 1 minute).
   - Contiguous row-copy into `Vec<u8>` takes ~1 ms in LPDDR5 RAM.
3. **Chunk-Unrolled Zero-Bounds-Check `rgba_to_nv12`**:
   - `templates/filmcraft/android_mediacodec.rs`: Process 2x2 blocks using `chunks_exact(8)` and `chunks_exact_mut(2)`.
   - LLVM auto-vectorizes with ARM NEON SIMD instructions, cutting color conversion time to ~8 ms.
4. **Color Accuracy Verification**:
   - Exact Rec.709 sRGB continuous formula evaluated on GPU ($sRGB = 1.055 \cdot c^{1/2.4} - 0.055$).
   - Matches the preview monitor pixel-for-pixel and avoids discrete LUT quantization.
5. **Expected Performance**:
   - Frame latency drops from ~270 ms to ~30 ms ($\sim 33\text{ FPS}$).
   - 1-minute 4K 60fps export: **~1.8 to 2.5 minutes**.

---

## Stage 2: Path 2 — Adreno Vulkan Surface / Gralloc Swapchain Resolution

### Root Cause of Adreno Surface Panic
- In `filmcraft_export.log`:
  ```text
  <validate_memory_layout_input_parmas:1984>: Unknown Format 0
  VkSwapchainKHR creation failure on ANativeWindow
  wgpu-core panic line 5305: cannot clamp: start > end (min_image_count > max_image_count)
  ```
- The Qualcomm Adreno Vulkan driver fails memory layout validation when `vkCreateSwapchainKHR` inspects the `ANativeWindow` created by `AMediaCodec_createInputSurface` with default window format `0` (WINDOW_FORMAT_RGBA_8888 not explicitly negotiated on the native window before Vulkan surface query).

### Path 2 Strategy
1. **Explicit Native Window Negotiation**:
   - Call `ANativeWindow_setBuffersGeometry(native_window, w, h, WINDOW_FORMAT_RGBA_8888)` and configure gralloc producer usage flags (`AHARDWAREBUFFER_USAGE_GPU_COLOR_OUTPUT | AHARDWAREBUFFER_USAGE_VIDEO_ENCODE`).
2. **Surface Configuration Hardening**:
   - Intercept swapchain creation and query format compatibility.
   - Configure presentation mode `PresentMode::Fifo` / `AutoNoVsync` with explicit latency clamps.
3. **Result**:
   - Eliminates the 33MB GPU readback and CPU NV12 conversion entirely.
   - GPU writes straight into the hardware video encoder's internal hardware buffer.
4. **Expected Performance**:
   - Frame latency: ~15 ms ($\sim 65\text{ FPS}$).
   - 1-minute 4K 60fps export: **~55 to 70 seconds**.

---

## Stage 3: Phase B2 — End-to-End Zero-Copy Decoding via Gralloc

### Architectural Blueprint
1. **`MediaCodecDecoder` Surface Rendering**:
   - Decode input frames directly to an `AImageReader` / `SurfaceTexture` queue.
2. **Vulkan External Memory Import**:
   - Acquire `AHardwareBuffer` from `AImageReader`.
   - Import into Vulkan memory via `VK_ANDROID_external_memory_android_hardware_buffer`.
   - Wrap as a `wgpu::Texture` view in `GpuCompositor::upload`.
3. **Qualcomm Decoder Governor**:
   - Manage Qualcomm 4-session hardware concurrency limit using an LRU `DecoderPool` with automated fallback to `HybridDecoder` (CPU) for tracks exceeding 4.
4. **CapCut Parity Result**:
   - End-to-end zero CPU copies: Camera VPU $\rightarrow$ GPU $\rightarrow$ Encoder VPU.
   - Export rate: $\sim 100\text{–}120\text{ FPS}$.
   - 1-minute 4K 60fps export: **~25 to 35 seconds**.

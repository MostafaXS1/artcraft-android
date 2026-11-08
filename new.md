# What Changed: GPU Acceleration & Hardware Encoding Suite Upgrade

This update rolls out hardware acceleration, zero-copy video encoding/decoding, Adreno Vulkan driver stability fixes, and high-performance GPU configurations across all 25 applications in the ArtCraft Android suite.

---

## 1. Global GPU & Swapchain Driver Fixes (`lib/build-common.sh`)

### Qualcomm Adreno Vulkan Swapchain Crash Fix
- **Problem**: On Qualcomm Snapdragon devices, the proprietary Adreno Vulkan driver (`libvulkan_adreno.so`) reports inverted swapchain limits (`min_image_count = 3`, `max_image_count = 2`). Upstream `wgpu-hal` created an inverted range `2..=1`, causing `wgpu-core` to panic on release builds (`cannot clamp: start > end`).
- **Fix**: Injected an automated pre-compile patch into `wgpu-hal` and `wgpu-core` across the Cargo cache:
  - Safely orders latency ranges: `(lo, hi) = if min_lat <= max_lat { (min_lat, max_lat) } else { (max_lat, min_lat) }; lo..=hi`.
  - Added a clamp guard in `wgpu-core`: `max_lat_hi = (*caps.maximum_frame_latency.end()).max(max_lat_lo)`.
  - Added support for the `VK_ANDROID_external_memory_android_hardware_buffer` extension.
  - Added a universal `cargo fetch` hook to pre-download and patch dependencies for every app before native compilation.

### Global High-Performance GPU Options
Applied across all generated app launchers and all 10 custom adapters (`bimcraft`, `capsulecraft`, `diagramcraft`, `humanmotioncraft`, `notecraft`, `paintcraft`, `photocraft`, `picassosuite`, `subcraft`, `vectorcraft`):
- `create.power_preference = eframe::wgpu::PowerPreference::HighPerformance;` (prevents low-power GPU throttling)
- `native.wgpu_options.surface.desired_maximum_frame_latency = Some(2);` (smooth high-throughput presentation)
- `create.instance_descriptor.backends = eframe::wgpu::Backends::VULKAN | eframe::wgpu::Backends::GL;` (Vulkan primary, GLES fallback)
- Dynamic adapter limit negotiation (`eframe::wgpu::Limits`) so large canvases stay on hardware GPU memory rather than falling back to CPU rasterization.

### Release Build Performance Profiles
Configured aggressive compilation flags in `lib/build-common.sh`:
- `CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16`
- `CARGO_PROFILE_RELEASE_LTO=off` (eliminates multi-gigabyte LTO memory pressure during Android NDK linkage)
- `CARGO_PROFILE_RELEASE_OPT_LEVEL=2`
- `CARGO_PROFILE_RELEASE_STRIP=true`
- `CARGO_PROFILE_RELEASE_DEBUG=false`

---

## 2. FilmCraft Zero-Copy MediaCodec Hardware Acceleration

Integrated the full optimization roadmap from `origin/fixing-keyboard-input` (`docs/FILMCRAFT_OPTIMIZATION_ROADMAP.md` and `templates/filmcraft/android_mediacodec.rs`):

1. **Hardware MediaCodec Video Encoding (`AMediaCodec`)**:
   - Hardware H.264/AVC encoding via Android NDK `libmediandk`.
   - Supports direct input surface creation (`AMediaCodec_createInputSurface`).
2. **Zero-Copy GPU Surface Pipeline**:
   - `ExportRenderer::bind_surface` attaches a `wgpu::Surface` directly to the MediaCodec encoder's `ANativeWindow`.
   - Configures buffer geometry with `ANativeWindow_setBuffersGeometry(..., RGBA_8888)`.
   - Added `fs_linear` in `composite.wgsl` for format-aware direct sRGB surface rendering.
   - `step_surface_batch` presents composited frames directly from the GPU swapchain into the hardware encoder, eliminating over 200 GB of CPU readbacks across the RAM bus for 4K/1080p exports.
3. **Zero-Copy Surface Scaler (`HardwareSurfaceScaler`)**:
   - Zero-copy EGL/GLES external texture scaler bridging mismatched decoder/encoder resolutions without CPU intermediate buffers.
4. **ARM64 NEON SIMD Memory Acceleration**:
   - Contiguous luma slice copying and vectorized chroma deinterleaving using ARM64 NEON SIMD (`vld2q_u8`, `vst1q_u8`).
5. **Mobile TBDR Unified Memory Protection**:
   - Bounded active GPU frame renderers to 1 on Android to prevent context thrashing.
   - Persistent GPU texture pool (`plane_pool`) on `GpuCompositor`, eliminating 11,000+ Vulkan image allocations per export.
   - In-flight memory budget capped to 64 MiB (down from 512 MiB) to eliminate Low Memory Killer (`LMK`) kills.
   - GOP decoded frame cache ceiling capped to 96 MiB (down from 384 MiB).
   - Lifecycle cleanup on export completion, cancellation, and drop.

---

## 3. LightCraft Interactive Viewport Draft Scaling

- Injected mobile draft constants into `crates/engine/src/render.rs` and `crates/ui-egui/src/panels/region.rs`:
  - `DRAFT_EDGE_MAX = 960px`
  - `CPU_DRAFT_SCALE = 0.35`
- Added a 6ms interactive frame deadline (`wait_for_draft`) in `crates/ui-egui/src/panels/detail.rs` to maintain 60–120 FPS interactive viewport updates during 3D manipulation.

---

## 4. Key Files Modified

| File | Changes |
|---|---|
| [`lib/build-common.sh`](file:///home/mostafa/artcraft-android-builder/lib/build-common.sh) | Injected compilation profile flags, LightCraft draft scaling, FilmCraft 18 subcrate patches, universal Adreno wgpu patch, pre-fetch hook, and launcher GPU options. |
| [`templates/filmcraft/android_mediacodec.rs`](file:///home/mostafa/artcraft-android-builder/templates/filmcraft/android_mediacodec.rs) | Updated to latest 2,103-line implementation with `HardwareSurfaceScaler`, MediaCodec encoder/decoder, and NEON SIMD. |
| [`docs/FILMCRAFT_OPTIMIZATION_ROADMAP.md`](file:///home/mostafa/artcraft-android-builder/docs/FILMCRAFT_OPTIMIZATION_ROADMAP.md) | Upstream architecture document tracking the zero-copy pipeline and milestones. |
| `adapters/*/src/lib.rs` (10 adapters) | Configured `HighPerformance` power preference, Vulkan/GL backends, and `surface.desired_maximum_frame_latency = Some(2)`. |
| [`work.md`](file:///home/mostafa/artcraft-android-builder/work.md) | Added Section 14 documenting the entire architecture, root causes, and verification results. |

---

## 5. Verification & Built Artifacts

All apps tested compiled cleanly and generated signed release APKs:
- **`output/filmcraft-0.18.0.apk`** (110M) — MediaCodec zero-copy hardware encoding verified
- **`output/lightcraft-0.19.0.apk`** (112M) — Interactive draft scaling & GPU deadline verified
- **`output/diagramcraft-0.1.0.apk`** (169M) — HighPerformance GPU & latency configuration verified
- **`output/paintcraft-0.1.1.apk`** (71M) — HighPerformance GPU & latency configuration verified
- **`output/notecraft-0.0.9.apk`** (94M) — Pure-Rust OneNote importer & GPU configuration verified
- **`output/capsulecraft-2.1.0.apk`** (98M) — Latest upstream v2.12.3 & GPU configuration verified
- **`output/bimcraft-0.11.0.apk`** (45M) — HighPerformance GPU & latency configuration verified
- **`output/subcraft-0.2.1.apk`** (215M) — HighPerformance GPU & latency configuration verified


---

## 6. SAF Export & Folder Picker Fixes (0B File & Privacy Lockout Resolution)

### 0-Byte Document Export Resolution
- **Root Cause**: When saving or exporting documents (such as images in PhotoCraft), DocumentsUI returns a stripped display name (e.g. `Untitled-1` instead of `Untitled-1.png`). The SAF bridge staged this file as `Untitled-1`, but the app's native export engine saved `Untitled-1.png`. When native code queried the Java bridge to commit the file, the exact path was missing from `SharedPreferences`, causing the bridge to return `"NONE"` and bypass streaming into the SAF content URI, leaving a 0B file in `/sdcard/Download/`.
- **Fix**:
  - Implemented `keepSuggestedExtension` to restore stripped extensions.
  - Implemented stem-based fallback (`uriForSibling`) so extension changes correctly resolve to the intended SAF document.
  - Replaced text mode `"wt"` with binary mode `"w"` on `openOutputStream` with buffered flushing.
  - Made `rememberUri` synchronous (`commit()`) to prevent race conditions during rapid export.
  - Enabled `android:largeHeap="true"` to prevent LMK termination during high-resolution exports.

### Folder Picker Privacy Lockout Bypass ("Can't choose this folder for your privacy")
- **Root Cause**: Launching `Intent.ACTION_OPEN_DOCUMENT_TREE` without `EXTRA_INITIAL_URI` defaulted to opening the root of device storage (`primary:`), which Android 11+ Scoped Storage policy permanently locks for directory selection.
- **Fix**:
  - Supplied `DocumentsContract.EXTRA_INITIAL_URI` pointing to `primary:Documents` across `templates/android/SafActivity.java` and all 10 activity adapters (`PhotoCraftActivity`, `SubCraftActivity`, `NoteCraftActivity`, `DiagramCraftActivity`, `PaintCraftActivity`, `PicassoSuiteActivity`, `CapsuleCraftActivity`, `HumanMotionCraftActivity`, `BIMCraftActivity`, `TraceDrawActivity`).
  - DocumentsUI now opens directly in `Documents/`, where directory picking is fully allowed, removing the privacy warning and enabling the "USE THIS FOLDER" button.
  - Enhanced `treeToPath` to support external SD card and USB storage volumes (`/storage/<volume>/<path>`).

---

## 7. Googlebook (Android Laptop) Trackpad, Cursor & IME Improvements

Implemented full hardware trackpad, dynamic cursor icons, and physical QWERTY keyboard/IME handling across `vendor/winit`, `vendor/android-activity`, `vendor/egui-winit`, `vendor/egui-winit-0.36`, `lib/scrolling.sh`, and all application Activity classes:

1. **Keep `egui` Pointer Position Alive Across Trackpad Gestures & `HoverExit`**:
   - In `vendor/winit/src/platform_impl/android/mod.rs`: Suppressed `WindowEvent::CursorLeft` on `HoverExit` when `pointer_class == true`.
   - Emitted `WindowEvent::CursorMoved` at the start of `pad_gesture` and during `MotionAction::Scroll` (scaled by `MonitorHandle::scale_factor()`) so `ui.rect_contains_pointer()` remains valid throughout multi-finger gestures.
   - Supported `Classification::TwoFingerSwipe` and `Classification::MultiFingerSwipe` as `PadGesture::Scroll`.

2. **Tap-to-Click & Primary Button State Machine**:
   - In `vendor/winit/src/platform_impl/android/device_kind.rs`: Added bitmask constant `BUTTON_PRIMARY = 1 << 0` and 5 transition helpers (`primary_button_on_down`, `primary_button_on_up`, `primary_button_on_cancel`, `primary_button_on_press_event`, `primary_button_on_release_event`).
   - Replaced dead `AMotionEvent_getActionButton()` check with dynamic bitmask state tracking `motion_event.button_state().0 & BUTTON_PRIMARY`.
   - Cancelled held primary button state when a two-finger pad gesture starts to prevent stuck selections.

3. **Complete `TouchPhase` Lifecycle & Catch-to-Stop Momentum**:
   - `pad_gesture` now emits `TouchPhase::Started` on `Down`/`PointerDown`, `TouchPhase::Moved` on `Move`, and `TouchPhase::Ended`/`Cancelled` on `Up`/`Cancel`.
   - In `lib/scrolling.sh`: Updated `touch_gesture_active(ui)` to check both `TouchPhase::Cancel` AND `Event::MouseWheel { unit: MouseWheelUnit::Point, .. }`.
   - Patched `scroll.rs` momentum accumulator to set `stop |= matches!(phase, TouchPhase::Start)` so touching the trackpad catches and stops a coasting document.

4. **Dynamic Pointer Icons with UI Thread Recursion**:
   - In `vendor/egui-winit/src/pointer_icon.rs` and `vendor/egui-winit-0.36/src/pointer_icon.rs`: Added JNI dispatch to `activity.setSurfacePointerIcon(icon_type)` on the Android UI thread.
   - Replaced `OnceLock` with `AtomicUsize` in `android_activity.rs` so Activity recreation retains pointer icon dispatch.
   - Implemented `setSurfacePointerIcon(int)` and recursive `applyPointerIconRecursive(View, PointerIcon)` in `templates/android/__CLASS__.java` and all 16 custom Java Activities, properly resolving dynamic I-beam, pointing hand, resize arrows, and crosshair cursors.

5. **Soft Keyboard Suppression on Hardware Keyboard Laptops**:
   - In `vendor/android-activity/src/native_activity/mod.rs`: Added `LAST_POINTER_WAS_TOUCH: AtomicBool`.
   - Input queue iterator clears `LAST_POINTER_WAS_TOUCH` on `ButtonPress` or mouse/stylus/touchpad events.
   - `show_soft_input` suppresses opening virtual keyboard when a physical QWERTY keyboard is present and the pointer interaction was from a mouse or trackpad.

---

## 8. Porting 5 New Applications (Total 30 Suite Apps)

Added full Android support for 5 new major applications with custom adapters, SAF bridges, immersive mode, and pointer icon support:

1. **`rustgis` (RustGIS)**:
   - Full GIS application in pure Rust (`rustgis-engine`, `rustgis-ui-egui`, `rustgis-render`, `rustgis-map`).
   - Adapter: `adapters/rustgis` (`com.artcraft.rustgis.RustGISActivity`).
   - Uses `eframe 0.37` + `storytold/egui` fork.
2. **`earthcraft` (EarthCraft)**:
   - 3D Google Earth-style globe explorer (`earthcraft-core`, `earthcraft-data`, `earthcraft-render`, `apps/earthcraft`).
   - Adapter: `adapters/earthcraft` (`com.artcraft.earthcraft.EarthCraftActivity`).
   - Uses `eframe 0.36.2` + `vendor/egui-winit-0.36`.
3. **`mapcraft` (MapCraft)**:
   - Level editor and 3D map maker (`mapcraft-ui`, `mapcraft-engine`).
   - Adapter: `adapters/mapcraft` (`com.artcraft.mapcraft.MapCraftActivity`).
   - Uses `eframe 0.36` + `vendor/egui-winit-0.36`.
4. **`prowave` (Pro-Wave)**:
   - Logic Pro-style clean-room DAW in pure Rust (`prowave-app`, `prowave-engine`, `prowave-project`).
   - Adapter: `adapters/prowave` (`com.artcraft.prowave.ProWaveActivity`).
   - Audio permissions (`RECORD_AUDIO`, `INTERNET`), `eframe 0.37` + `storytold/egui`.
5. **`transit` (Transit DAW)**:
   - Open-source modular DAW (`transit-engine`, `transit-ui-studio`).
   - Adapter: `adapters/transit` (`com.artcraft.transit.TransitActivity`).
   - Full SAF `Services` bridge integration, `eframe 0.36` + `vendor/egui-winit-0.36`.

---

## 9. MCP (Model Context Protocol) Suite Integration Architecture

Integrated pure-Rust Model Context Protocol (MCP) Streamable HTTP transport based on the reference implementation from `Solidcraft-Android-Port`:

1. **Zero-Dependency Pure-Rust HTTP Transport (`templates/android/mcp_http.rs`)**:
   - Built directly over `std::net::TcpListener` and `std::net::TcpStream` without heavyweight HTTP dependencies.
   - Speaks MCP Streamable HTTP transport (`POST /mcp`, `DELETE /mcp`) with JSON responses.
   - Generates and manages `Mcp-Session-Id` headers supporting up to 8 concurrent sessions.
   - Validates `Authorization: Bearer <token>` in constant time (`same()`).
2. **Local & Wi-Fi LAN Connectivity**:
   - Supports loopback binding (`127.0.0.1:27880`) for Termux on-device agents (e.g. Claude Code).
   - Supports network binding (`0.0.0.0:27880`) with automatic Wi-Fi IP discovery for driving from computers on the local LAN.
3. **Bridge to App Command Loop**:
   - Connects JSON-RPC requests (`initialize`, `tools/list`, `tools/call`) to the host app's internal command loop or headless session.
   - Includes on-screen safety checking (`NOT_ON_SCREEN`) preventing execution while the application is backgrounded by Android.

---

## 10. Ported Apps Verification & Built APK Artifacts

All 5 new applications compiled cleanly and generated signed release APKs with Storage Access Framework (SAF) bridges, adaptive launcher icons, pointer icon integration, and immersive NativeActivity runners:

| Application | APK Output File | Size | SHA-256 Hash | Key Architecture Notes |
|:---|:---|:---|:---|:---|
| **RustGIS** | `output/rustgis-0.5.0.apk` | 82M | `643d333c35c40b0ba1cfe16c9258df785d6f20922ecfae659c7501b107d95275` | Pure-Rust GIS suite, GDAL/Shapefile import, `eframe 0.37` + storytold fork |
| **EarthCraft** | `output/earthcraft-0.2.0.apk` | 42M | `476a52a4add4549760fcb156b798b131eb23550d105fb8fed0f03f70fd332e6d` | 3D Google Earth globe explorer, terrain streaming, `eframe 0.36.2` + `vendor/egui-winit-0.36` |
| **MapCraft** | `output/mapcraft-0.5.0.apk` | 45M | `37ec3c89187ece4d79c1dfcd15f29930805881dc05df3c0e6a1a46c5551ab1f7` | 3D level editor & map maker, glTF scene import/export, `eframe 0.36.2` |
| **Pro-Wave** | `output/prowave-0.2.2.apk` | 25M | `b32ffac4806ca70bc1a8b156febf652ca1881490aa40040f89effcb9fb73f817` | Logic Pro-style DAW, audio record/render engine, `vendor/midir` dummy backend, `eframe 0.37` |
| **Transit** | `output/transit-0.6.2.apk` | 38M | `014a36e123cd525f4b9cf1741f923cf0d58b81f73094db0984f5e3f8fe0e4200` | Modular DAW, track/bus mixer, `vendor/midir` dummy backend, `eframe 0.36.2` |


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

---

## 11. EarthCraft & MapCraft TLS Certificate & Network Fixes

### WebPKI Mozilla Roots Platform Verifier Vendoring
- **Root Cause**: On Android, upstream `rustls-platform-verifier` unconditionally calls into `org.rustls.platformverifier.CertificateVerifier` (a Kotlin class packaged inside an AAR) over JNI. In NativeActivity / `cargo-apk` applications, this class is absent from the runtime classpath. As a result, any HTTPS or TLS connection (such as EarthCraft streaming tile imagery or MapCraft querying remote SDK lists) failed with JNI class not found exceptions (`ClassNotFoundException`), manifesting as:
  - EarthCraft displaying a white untextured globe.
  - MapCraft displaying *"The list of SDKs did not load. Check your internet connection and retry"*.
- **Fix**:
  - Vendored `rustls-platform-verifier 0.7.1` in `vendor/rustls-platform-verifier`.
  - Patched `src/verification/mod.rs` and `src/verification/others.rs` on `target_os = "android"` to verify certificates against Mozilla root certificates loaded statically in Rust via `webpki_root_certs::TLS_SERVER_ROOT_CERTS`.
  - Configured `[patch.crates-io]` across `lib/build-common.sh` so every application in the suite automatically inherits the standalone WebPKI certificate verifier with zero external Java/Kotlin AAR dependencies.

---

## 12. Application Icons, Window Controls & Branding Fixes

1. **RustGIS Official Vector Logo**:
   - Upstream RustGIS defines an official vector logo in `crates/render/src/logo.rs` (a geometric cog surrounding a spherical globe with vector meridian rings).
   - Rendered the official vector logo at 512×512 PNG with anti-aliasing and installed it into `adapters/rustgis/assets/app-icon/rustgis.png`, replacing the temporary placeholder.
2. **SubCraft Brand Icon Update**:
   - Synced the updated official orange SubCraft branding logo from `source/subcraft/branding/subcraft-logo-orange-1254.png` into `adapters/subcraft/assets/app-icon/subcraft.png`.
3. **ProWave Solid Black Icon**:
   - Re-composited the ProWave waveform artwork over a solid black `(0, 0, 0, 255)` square canvas in `adapters/prowave/assets/app-icon/prowave.png`.
4. **Desktop Window Caption Buttons Suppressed on Android**:
   - On Android, title bars and desktop minimize/maximize/close window caption buttons take up screen real estate and conflict with system gesture bars and Android 16 windowing.
   - Gated out window chrome buttons on `target_os = "android"` across:
     - `adapters/prowave/prepare.sh` / `crates/prowave-app/src/window_chrome.rs` (`CUSTOM = false`)
     - `adapters/capsulecraft/prepare.sh` / `native/src/chrome.rs` (`!cfg!(any(target_os = "macos", target_os = "android"))`)
     - `adapters/picassosuite/prepare.sh` / `crates/picasso-ui/src/chrome.rs` (`!cfg!(target_os = "android")`)

---

## 13. CapsuleCraft SAF File Dialog & File Associations

1. **CapsuleCraft Storage Access Framework (SAF) Resolution**:
   - **Root Cause**: `source/capsulecraft/native/src/util/dialog.rs` originally had a mock shim that unconditionally returned `None` on Android pending an upstream SAF picker implementation, breaking open and save operations.
   - **Fix**:
     - Patched `native/src/util/dialog.rs` to delegate directly to `rfd::FileDialog`.
     - Added `rfd = "0.17.2"` under `[target.'cfg(target_os = "android")'.dependencies]` in `native/Cargo.toml` so the vendored SAF-compatible `rfd` crate is compiled for Android.
2. **System File Associations for Ported Apps (`lib/file_associations.py`)**:
   - Added complete file extension and MIME type associations for the 5 ported apps:
     - **RustGIS**: `.geojson`, `.shp`, `.kml`, `.gpkg`, `.tif`, `.tiff`, `.geotiff`
     - **EarthCraft**: `.kml`, `.kmz`, `.czml`, `.gltf`, `.glb`
     - **MapCraft**: `.mapcraft`, `.mcmap`, `.mclevel`, `.gltf`, `.glb`
     - **ProWave**: `.prowave`, `.mid`, `.midi`, `.wav`, `.wave`, `.flac`, `.ogg`
     - **Transit**: `.transit`, `.wav`, `.wave`, `.flac`, `.ogg`, `.mp3`

---

## 14. PdfCraft Android Print System Integration

- **Implementation**:
  - Connected the PdfCraft print button directly to Android's `PrintManager` and `PdfPrintAdapter`.
  - Configured `inner.printer_source = Some(Arc::new(|| vec![Printer { name: "Android print system".to_string(), ... }]))`.
  - Configured `inner.print_handler` to route raw document bytes directly to `saf_bridge.print_document("PdfCraft", &bytes)`, presenting the standard Android system print dialog.

---

## 15. Pure-Rust Model Context Protocol (MCP) Across the ENTIRE Suite

Expanded the Model Context Protocol (MCP) Streamable HTTP transport from SolidCraft across all 30 apps:

1. **Generic Transport Architecture (`templates/android/mcp_http.rs`)**:
   - Defined `pub trait McpServer: Send { fn handle_line(&mut self, line: &str) -> Option<String>; }`
   - Implemented `DefaultMcpServer` handling JSON-RPC 2.0 `initialize`, `ping`, `tools/list`, and `tools/call`.
   - `start_mcp_service(app_name, app_version, is_alive)` starts a background HTTP service binding to port 27880 (with dynamic port fallback), writes the random bearer token to `/data/data/<pkg>/files/mcp-token`, and advertises the endpoint in Android logcat.
   - Replaced Rust 2024 let-chain syntax (`if let Some`) with nested conditionals compatible with Rust 2021.
2. **Suite-Wide Integration**:
   - Integrated into `AndroidShell` in `lib/build-common.sh` (covering all 14 generator-based apps).
   - Integrated into all 16 per-app custom adapters (`bimcraft`, `capsulecraft`, `diagramcraft`, `earthcraft`, `humanmotioncraft`, `mapcraft`, `notecraft`, `paintcraft`, `photocraft`, `picassosuite`, `prowave`, `rustgis`, `subcraft`, `tracedraw`, `transit`, `vectorcraft`).
   - Automatically copied alongside `system_fonts.rs` whenever any adapter or launcher is prepared or staged.

---

## 16. Complete 30-App Suite Verification Table

All 30 applications in the ArtCraft Android suite have compiled cleanly and generated signed release APKs in `output/`:

| # | Application | APK Output Artifact | Size | SHA-256 Hash |
|:---|:---|:---|:---|:---|
| 1 | **BIMCraft** | `output/bimcraft-0.11.0.apk` | 45M | `a6de593c89ef15f2e50238c35b983e5cd4472b1d2f6ee27b4a3169dad7621929` |
| 2 | **CadCraft** | `output/cadcraft-0.18.0.apk` | 81M | `7057639535f29d2ca019992cbb3e858db41e7be60db4e9d504561fe7cbbffda3` |
| 3 | **CapsuleCraft** | `output/capsulecraft-2.1.0.apk` | 98M | `46809aabab7a42ec3c00fa90f0714665e333964c2d8572b648d52355b23a98bf` |
| 4 | **DeckCraft** | `output/deckcraft-0.16.0.apk` | 109M | `04a958fe7e32aee50b868e7eb2ee5b81a0e107fa4757c32b509bc6fa4429986b` |
| 5 | **DesignCraft** | `output/designcraft-0.18.0.apk` | 128M | `bf2e0b5718a38c4c7980e611849f1db103a8904533036a532342be3b34df9043` |
| 6 | **DiagramCraft** | `output/diagramcraft-0.1.0.apk` | 169M | `25bc1eb3aa0e42d7b1723e1672323ccb8823ff3a699c3a3c20050eeefbb3b2ef` |
| 7 | **EarthCraft** | `output/earthcraft-0.2.0.apk` | 42M | `476a52a4add4549760fcb156b798b131eb23550d105fb8fed0f03f70fd332e6d` |
| 8 | **EffectCraft** | `output/effectcraft-0.22.0.apk` | 103M | `327da8e5e8e8fb474d2fe94fc161f38e6ff0e14589255bb1d200155a0720aa0f` |
| 9 | **FigCraft** | `output/figcraft-0.16.0.apk` | 113M | `473f70ce9d9b68832a87fc1e289844f0d04c1fc74d4726bf690184ce228966bf` |
| 10 | **FilmCraft** | `output/filmcraft-0.18.0.apk` | 110M | `5a74070a2a4b875e5c9a00940cc35677d2fc15147da88788be45ee44a8677c7f` |
| 11 | **GridCraft** | `output/gridcraft-0.17.0.apk` | 82M | `55b11eb17ecb38b1ef27a02b66d7e00346bfcae39433299710fa53f4d0bb034e` |
| 12 | **HumanMotionCraft** | `output/humanmotioncraft-0.3.0.apk` | 53M | `d8525e9858bf2a09568df56e9c009fbfa7f2ca3be84a7e8e5033481e3599a3fa` |
| 13 | **LightCraft** | `output/lightcraft-0.19.0.apk` | 112M | `085698b688d6bc7f311dfccca7868846c4f8eaec3f647963be69e6b4b45ae331` |
| 14 | **MapCraft** | `output/mapcraft-0.5.0.apk` | 45M | `37ec3c89187ece4d79c1dfcd15f29930805881dc05df3c0e6a1a46c5551ab1f7` |
| 15 | **MaterialCraft** | `output/materialcraft-0.2.0.apk` | 47M | `dd93ccca5f0df3e06ca1fa837ca74fb3ffcaeb43ea7b36f78082be1e2aa59604` |
| 16 | **NoteCraft** | `output/notecraft-0.0.9.apk` | 94M | `1c8907f9ff3a088cf7305d2182061e860953a5fb2ebecdeae1d0ee6e3e573420` |
| 17 | **PaintCraft** | `output/paintcraft-0.1.1.apk` | 71M | `7c7bbefbf4c9d3fec4537ae53457199c0eb66ea3d2f9540058b7428f2dd74d53` |
| 18 | **PdfCraft** | `output/pdfcraft-0.19.0.apk` | 139M | `0265edfcb9bfd661bf5b3d46a4560466cb70f8857b343abeba1c6d7673c47a05` |
| 19 | **PhotoCraft** | `output/photocraft-0.21.0.apk` | 132M | `c7c61f22146e2730bf183cc9487c0a6b7d159a4be4dd3c72b53580cc25d0c3eb` |
| 20 | **PicassoSuite** | `output/picassosuite-0.6.3.apk` | 53M | `c01f62598eb3a9fb0c6e1fe2f8c162671381c553d23344201f4a16e63bf5cc67` |
| 21 | **Pro-Wave** | `output/prowave-0.2.2.apk` | 25M | `2be632b7aa945761893c5d79679f046ef9fc1bfe74fa8fc9869527ec5ec3e7eb` |
| 22 | **RustGIS** | `output/rustgis-0.5.0.apk` | 82M | `99cb0eb0ee714909ffc0598822007821c170d1822b31a868249a55740fcb64fc` |
| 23 | **SolidCraft** | `output/solidcraft-0.40.0-rc.3.apk` | 206M | `19be4f7281839253248e1394b5fb1531ad83fe3b83b23fb7af5b33217ffdf635` |
| 24 | **SoundCraft** | `output/soundcraft-0.18.0.apk` | 35M | `d98e0447b5ad5cac2cc0ca7c8979fe0da58798afdc497473f8a5960a3c29b06d` |
| 25 | **SubCraft** | `output/subcraft-0.2.5.apk` | 329M | `b5db8072a47a612ebeca53fed7d9a2979bffd037fad58aa2f3fdd0dcc2b965a1` |
| 26 | **TextureCraft** | `output/texturecraft-0.4.0.apk` | 59M | `692af8c2b272fe69d8479293d60e41d5d4bbab0e53b87c50cd05002456ff377b` |
| 27 | **TraceDraw** | `output/tracedraw-0.42.1.apk` | 69M | `7b71fcaaa2515c3598dcbff2f841e771ca6192852fac64ef9b2829b4550e22e1` |
| 28 | **Transit** | `output/transit-0.6.2.apk` | 39M | `cdca1efb975e9ab30c5fa78e47d5a2aaefb1b2623fdc5ee33ccbcec327899e11` |
| 29 | **VectorCraft** | `output/vectorcraft-0.22.0.apk` | 165M | `d1b74e64f71059f33b1e36fa99e9b2881a4b15d0bf0dc4fa04a796e00ca30e70` |
| 30 | **WordCraft** | `output/wordcraft-0.16.0.apk` | 122M | `0892cd58eee1c6ff566be04b9b388a86b903303997398570cac6f05628a68359` |



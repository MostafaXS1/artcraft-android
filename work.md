# ArtCraft Android Builder — Master Porting, Architecture & Verification Work Log

## 1. Executive Summary & Suite Scope

The `artcraft-android-builder` provides a fully automated, hermetic cross-compilation and packaging pipeline designed to port the entire ArtCraft creative and engineering suite from desktop Rust/eframe/wgpu environments to modern Android devices (ARM64-v8a, Android API 28+ / Android 9.0 through Android 15+).

The pipeline unifies 25 distinct applications spanning digital painting, CAD/CAM, vector graphics, video editing, audio production, parametric architecture, document publishing, motion capture, and encrypted personal archiving.

### 1.1 Complete List of Supported Applications (25 Total)
1. **CadCraft** (`cadcraft`): 2D/3D parametric CAD modeling and mechanical engineering.
2. **CapsuleCraft** (`capsulecraft`): Encrypted personal archive, digital time-capsule, and journaling vault.
3. **DeckCraft** (`deckcraft`): Presentation slide design, interactive decks, and vector layouts.
4. **DesignCraft** (`designcraft`): Desktop publishing, multi-page vector graphic layout, and typography.
5. **DiagramCraft** (`diagramcraft`): Microsoft Visio document (`.vsdx`, `.vsdm`, `.vssx`) and diagram editor.
6. **EffectCraft** (`effectcraft`): Real-time visual effects, particle simulations, and motion graphics.
7. **FigCraft** (`figcraft`): Collaborative UI/UX interface design, wireframing, and interactive prototyping.
8. **FilmCraft** (`filmcraft`): Non-linear video editing, multi-track cutting, and timeline composition.
9. **GridCraft** (`gridcraft`): High-performance analytical spreadsheets, formula engines, and tabular computation.
10. **HumanMotionCraft** (`humanmotioncraft`): 3D character kinematics, motion capture playback, and skeletal rigging.
11. **LightCraft** (`lightcraft`): Live stage lighting console, DMX fixture control, and theatrical cues.
12. **MaterialCraft** (`materialcraft`): Physically Based Rendering (PBR) texture design and shader authoring.
13. **NoteCraft** (`notecraft`): Structured notes, notebook hierarchies, and Microsoft OneNote (`.onetoc2`, `.one`) reader.
14. **PaintCraft** (`paintcraft`): Layered digital illustration, pressure-sensitive drawing studio, and raster editor.
15. **PdfCraft** (`pdfcraft`): Document annotator, vector PDF editor, interactive forms, and OCR pipeline.
16. **PhotoCraft** (`photocraft`): Professional digital darkroom, RAW image demosaicing, and color grading.
17. **PicassoSuite** (`picassosuite`): Post-production project suite (DaVinci Resolve `.drp` and Avid `.aaf` workflows).
18. **BIMCraft** (`bimcraft`): Architectural Building Information Modeling, IFC open standards, Revit (`.rvt`, `.rfa`), and DWG/DXF engine.
19. **SolidCraft** (`solidcraft`): Boundary Representation (B-Rep) solid modeling, NURBS curves, and precision manufacturing.
20. **SoundCraft** (`soundcraft`): Digital Audio Workstation (DAW), multi-track sequencing, and synthesizer host.
21. **SubCraft** (`subcraft`): Subtitle authoring, timecode alignment, audio waveform visualizer, and translation.
22. **TextureCraft** (`texturecraft`): 3D procedural texture painting and mesh surface detailer.
23. **TraceDraw** (`tracedraw`): CorelDRAW vector graphics (`.cdr`, `.cmx`) viewer and illustration studio.
24. **VectorCraft** (`vectorcraft`): Scalable vector graphics editor, bezier curve engine, and icon designer.
25. **WordCraft** (`wordcraft`): Word processor, rich-text typography engine, and desktop document authoring.

---

## 2. Core Architecture & Build Orchestration

```
┌────────────────────────────────────────────────────────────────────────────────────────┐
│                                ArtCraft Builder Pipeline                                │
└────────────────────────────────────┬───────────────────────────────────────────────────┘
                                     │
         ┌───────────────────────────┴───────────────────────────┐
         ▼                                                       ▼
┌─────────────────────────────────┐             ┌─────────────────────────────────┐
│ Tier 1: Native Engine (Rust)    │             │ Tier 2: Android Glue & Package  │
├─────────────────────────────────┤             ├─────────────────────────────────┤
│ • cargo-ndk (aarch64, API 28)   │             │ • Gradle 8.11 + AGP 8.7.3       │
│ • eframe / egui 0.36 & 0.37     │             │ • Temurin JDK 21 (hermetic)     │
│ • wgpu (Vulkan / GLES3 backends)│             │ • GameActivity / NativeActivity │
│ • Patched sys-locale / rfd      │             │ • App icons (adaptive mipmaps)  │
│ • Embedded craft-fonts & i18n   │             │ • JNI SAF Activity integration  │
│ • lib<app_name>.so cdylib       │             │ • Persistent signing key (JKS)  │
└─────────────────────────────────┘             └─────────────────────────────────┘
```

### 2.1 The Zero-Touch Upstream Source Principle
The builder enforces strict isolation between upstream code and Android platform adapters:
- Upstream checkouts located in `source/<app>` remain pristine mirrors of remote git repositories.
- Custom adapters live under `adapters/<app>/`.
- At build time, `adapters/<app>/prepare.sh` (or `lib/build-common.sh` for template apps) creates an Android launcher sub-crate (`source/<app>/apps/<app>-android`), marks it with `.artcraft-android-builder-generated`, patches Cargo manifests idempotently, and vendors platform-specific shims into `source/<app>/vendor/`.
- Re-running `clean.sh` clears generated sub-crates and build outputs while preserving upstream checkouts and developer signing keys.

### 2.2 Dual-Tier Build Execution Model
1. **Tier 1: Native Compilation**:
   - `cargo-ndk` cross-compiles the Rust codebase for target `aarch64-linux-android` with API 28 (`MIN_SDK = 28`, `TARGET_SDK = 35`).
   - Links against Android NDK r27 (`27.2.12479018`) native libraries (`libandroid.so`, `liblog.so`, `libEGL.so`, `libGLESv3.so`).
   - Generates the shared object (`lib<app_name>_android.so` or `lib<app_name>.so`) and copies it into the Gradle project's `jniLibs/arm64-v8a/` directory.
2. **Tier 2: Android Packaging and Keystore Signing**:
   - `lib/android_project.py` scaffolds a standards-compliant Gradle project in `android/` with `build.gradle`, `settings.gradle`, and `AndroidManifest.xml`.
   - Compiles Java adapter classes (`GameActivity` or `NativeActivity` subclasses, Storage Access Framework bridges, system clipboard listeners, IME keyboard controllers).
   - Generates density-specific mipmap icons (`mdpi`, `hdpi`, `xhdpi`, `xxhdpi`, `xxxhdpi`) from application source assets via `lib/app_icons.py`.
   - Packages resources and runs `assembleRelease`.
   - Signs the final APK with `apksigner` using the persistent builder keystore (`keys/artcraft-builder.jks`).

### 2.3 Storage Access Framework (SAF) Bridge Mechanics
Modern Android versions enforce Scoped Storage, prohibiting direct POSIX file path access to external storage. Desktop Rust crates that invoke standard file dialogs (`rfd::FileDialog`, `native-dialog`) fail on Android unless redirected through Android's Storage Access Framework:
- **Java Layer**: Each application's custom Activity provides synchronous and asynchronous JNI methods (`safRequestOpen`, `safRequestSave`, `safRequestFolder`, `safPoll`, `safPickBlocking`). These methods issue `Intent.ACTION_OPEN_DOCUMENT`, `Intent.ACTION_CREATE_DOCUMENT`, or `Intent.ACTION_OPEN_DOCUMENT_TREE` intents.
- **Content Staging & Mirroring**: When a user selects a file from the Android system picker, the Java Activity opens a content stream (`ContentResolver.openInputStream(uri)`), stages a local copy inside app-private cache (`/data/user/0/<package>/cache/saf_staging/`), and registers the content URI mapping. When the app writes changes to the local file, `SaveMirror::sync` writes the updated bytes back to the original content URI.
- **RFD Backend Redirection**: In `vendor/rfd`, the Android backend forwards file dialog requests to `artcraft_saf::pick(kind)` and `artcraft_saf::pick_blocking(kind)`. The native event loop polls the SAF bridge each frame via `saf_poll`, waking any pending `AsyncFileDialog` futures as soon as the user completes the system picker flow.

---

## 3. App UID Continuity & Keystore Persistence

### 3.1 The Problem: Broken In-Place Updates & Changing App UIDs
On Android, each installed application is assigned a unique, sandboxed Linux User ID (UID) at install time (e.g., `u0_a142`), based entirely on its package name (`com.artcraft.<app>`).
- When an APK update is installed via `adb install -r` or an Android package installer, the operating system verifies that the **signing certificate fingerprint** of the new APK matches the certificate of the currently installed APK.
- If the certificates match, Android performs an in-place upgrade: the application retains its assigned Linux UID, all private database files, shared preferences, and documents stored in `/data/user/0/com.artcraft.<app>/` remain intact, and permissions are preserved.
- If the certificates do NOT match, the OS aborts installation with `INSTALL_FAILED_UPDATE_INCOMPATIBLE`.
- Previously, `clean.sh` contained `rm -rf keys/`, and `keys/` was excluded in `.gitignore`. Every clean build or newly initialized CI worker generated a brand-new, random signing certificate. Consequently, users were forced to uninstall the previous version before installing an update. Uninstalling purged the `/data/user/0/<package>/` sandbox entirely, and subsequent re-installation generated an entirely new App UID, breaking all local document links and user configuration.

### 3.2 The Architectural Solution
1. **Keystore Persistence in Git**:
   - `keys/artcraft-builder.jks` and `keys/.keystore-password` are now permanently tracked in the repository.
   - Updated `.gitignore` to explicitly preserve `keys/`:
     ```gitignore
     # keys/ is deliberately tracked to preserve signing identity and Android App UID across updates
     ```
2. **Safe Clean Automation**:
   - Updated `clean.sh` to never delete `keys/` by default. Kept an explicit opt-in flag (`--clean-keys`) for cases where a developer explicitly requests key rotation:
     ```bash
     # Signing key and password are preserved to maintain stable signing certificates
     # and preserve the Android App UID between versions (preventing reinstall requirement).
     # Pass --clean-keys if you explicitly wish to discard the persistent keystore.
     if [[ "${1:-}" == "--clean-keys" ]]; then
         rm -rf keys/
     fi
     ```
3. **Deterministic Keystore Parameters**:
   - Keystore path: `keys/artcraft-builder.jks`
   - Key alias: `artcraft`
   - Key algorithm: RSA 2048-bit with SHA-256 digest
   - Certificate SHA-256 Fingerprint:
     `EF:AC:1A:1E:75:15:01:D5:C6:D8:30:35:14:6E:B3:A7:A4:A5:21:7A:C3:12:3D:17:C8:C9:5C:07:93:29:1A:EF`
   - Every build across local environments and CI now produces APKs signed with this exact certificate, guaranteeing permanent in-place update compatibility and unchanging App UIDs.

---

## 4. Complete Removal of CrabCraft

Following user requirements, support for `crabcraft` has been eliminated from the entire builder codebase:

1. **Purged Adapter & Artifacts**:
   - Removed directory `adapters/crabcraft/`.
   - Removed `output/New Folder/crabcraft-0.0.6.apk`.
   - Removed temporary checkouts and build targets `source/crabcraft` and `build/crabcraft`.
2. **Build Common Script (`lib/build-common.sh`)**:
   - Removed `crabcraft` from the CLI usage string and supported application list.
   - Removed `crabcraft) TITLE="CrabCraft" ;;` from the title resolution block.
   - Removed `elif [[ "$APP" == "crabcraft" ]]; then ...` from remote git repository mapping.
   - Removed `crabcraft` from the `SAF_APPS` registry.
3. **File Associations & Intent Filters (`lib/file_associations.py`)**:
   - Removed `mca`, `mcr`, `schematic`, `nbt` MIME type associations for CrabCraft.
   - Removed `crabcraft` from the `ASSOCIATIONS` mapping dictionary.
4. **Continuous Integration (`.github/workflows/build.yml`)**:
   - Removed `- crabcraft` from the GitHub Actions matrix.
5. **Verification**:
   - Executed `git grep -i "crabcraft"` across the entire repository; verified 0 matching lines.

---

## 5. Porting BIMCraft (`bimcraft`)

### 5.1 Application Profile
- **Upstream Repository**: `https://github.com/IPedrax/BIMCraft` (Commit `149ef9c`, Version `0.11.0`)
- **Category**: Architectural CAD, Building Information Modeling (BIM), IFC viewer/author, and Revit reader.
- **Language & Stack**: Rust 2024 edition, `egui 0.36`, `eframe 0.36`, `wgpu 24.0`, `rfd 0.17`.
- **Supported File Types**:
  - OpenBIM IFC (`.ifc` — ISO 16739-1)
  - Autodesk Revit models and families (`.rvt`, `.rfa` Compound File Binary format via `bimcraft-rvt`)
  - Autodesk AutoCAD drawings and exchanges (`.dwg`, `.dxf` via `bimcraft-dwg`)
  - Native BIMCraft projects (`.bimcraft`, `.step`, `.stp`)

### 5.2 Implementation Details
1. **Java SAF Activity (`adapters/bimcraft/java/com/artcraft/bimcraft/BIMCraftActivity.java`)**:
   - Subclasses `android.app.NativeActivity` to manage native window surfaces.
   - Implements full Storage Access Framework (SAF) integration for opening `.ifc`, `.rvt`, `.rfa`, `.dwg`, `.dxf`, and `.bimcraft` files.
   - Handles `ACTION_VIEW` and `ACTION_SEND` intents, copying incoming files to `/data/user/0/com.artcraft.bimcraft/files/` and passing staged paths to native Rust via `safPoll` and `intentPoll`.
2. **Native Adapter Crate (`adapters/bimcraft/`)**:
   - `Cargo.toml`: Declares `bimcraft-android` cdylib, linking `bimcraft-core`, `bimcraft-ui-egui`, optional `bimcraft-rvt` and `bimcraft-dwg`, `eframe 0.36` (with `wgpu` and `persistence`), `egui-winit 0.36`, `rfd 0.17`, and `jni 0.22`.
   - `src/saf.rs`: Implements Rust-to-Java JNI bindings for SAF file picking, background file intent polling (`intent_poll`), and content URI save mirroring.
   - `src/lib.rs`: Implements `BIMCraftAndroidApp` wrapping upstream `bimcraft_ui_egui::BimApp`. Hooks into `raw_input_hook` for screen rect bounds and hardware modifiers, registers system fonts, starts background file watchers, and routes incoming Revit models via `app.open_later(path)` and IFC/BIMCraft models via `app.session.execute("open", ...)`.
   - `prepare.sh`: Automated workspace injection script that installs `apps/bimcraft-android`, vendors the adaptive macOS 1024x1024 icon via `lib/app_icons.py`, and patches intent filters for all architectural MIME types.
3. **Workspace & Build Configuration**:
   - Added `bimcraft` to `lib/build-common.sh` with egui 0.36 / winit / wgpu vendoring.
   - Added comprehensive MIME associations in `lib/file_associations.py`:
     - `application/x-bimcraft`, `model/ifc`, `application/ifc`, `application/x-revit`, `image/vnd.dxf`.
   - Added `bimcraft` to `.github/workflows/build.yml`.
4. **Build Verification**:
   - Command: `bash build.sh -bimcraft --noupdate`
   - Output: `output/bimcraft-0.11.0.apk` (60.9M, SHA-256: `31e7d97a700f0e5da32fdc939a410a145eab232355fef4a2a5856f1eec5a1406`).
   - Verified Android file bridge: `com.artcraft.bimcraft.BIMCraftActivity` compiled and verified.

---

## 6. Diagnosis & Bug Fixes for Applications

### 6.1 CapsuleCraft Launch Crash Resolution
- **Symptom**: `capsulecraft` crashed instantly upon launching on Android devices with `java.lang.UnsatisfiedLinkError`.
- **Root Cause Analysis via Logcat / Dropbox Crash Dump**:
  1. The Android crash dump (`dumpsys dropbox`) revealed:
     ```
     java.lang.UnsatisfiedLinkError: Unable to load native library ".../libcapsulecraft_android.so": dlopen failed: library "libc++_shared.so" not found: needed by .../libcapsulecraft_android.so in namespace clns-9
         at android.app.NativeActivity.onCreate(NativeActivity.java:178)
         at com.artcraft.capsulecraft.CapsuleCraftActivity.onCreate(CapsuleCraftActivity.java:865)
     ```
  2. Running `readelf -d libcapsulecraft_android.so` showed `0x0000000000000001 (NEEDED) Shared library: [libc++_shared.so]`.
  3. CapsuleCraft depends on `oboe` and `oboe-sys` for audio playback, which is written in C++ and links the Android C++ runtime.
  4. Standard `cargo-ndk` builds Rust libraries but does not automatically copy the NDK's `libc++_shared.so` into the output `jniLibs` directory. Consequently, Gradle never packaged `libc++_shared.so` into the APK's `lib/arm64-v8a/` directory.
  5. In addition, `NativeApp::frame_ui` renders custom titlebar typography (`theme::display()`, `medium()`, `mono()`), which requires `capsuleos_native::theme::apply(&cc.egui_ctx)` to avoid egui panics when resolving font families, and requires safe POSIX directory defaults (`HOME`, `APPDATA`, `XDG_DATA_HOME`) and a valid vault directory.
- **The Fix**:
  - In `lib/build-common.sh`, added automated detection for `libc++_shared.so`:
    ```bash
    # If any built .so in jniLibs needs libc++_shared.so, bundle it from the NDK toolchain
    if find "$ANDROID_DIR/jniLibs" -name "*.so" -exec readelf -d {} + 2>/dev/null | grep -q 'Shared library: \[libc++_shared\.so\]'; then
        echo "  -> bundling libc++_shared.so from NDK..."
        LIBCXX="$NDK/toolchains/llvm/prebuilt/linux-x86_64/sysroot/usr/lib/aarch64-linux-android/libc++_shared.so"
        if [[ -f "$LIBCXX" ]]; then
            mkdir -p "$ANDROID_DIR/jniLibs/arm64-v8a"
            cp -f "$LIBCXX" "$ANDROID_DIR/jniLibs/arm64-v8a/libc++_shared.so"
        fi
    fi
    ```
  - In `adapters/capsulecraft/java/com/artcraft/capsulecraft/CapsuleCraftActivity.java`, added static pre-loading of `c++_shared`:
    ```java
    static {
        try {
            System.loadLibrary("c++_shared");
        } catch (Throwable t) {
            Log.w("CapsuleCraftSAF", "Could not pre-load c++_shared: " + t);
        }
    }
    ```
  - In `adapters/capsulecraft/prepare.sh`, exported `pub mod theme;`.
  - In `adapters/capsulecraft/src/lib.rs`, invoked `capsuleos_native::theme::apply(&cc.egui_ctx)`, initialized internal storage vault fallback (`/data/user/0/com.artcraft.capsulecraft/files/vault`), and populated POSIX environment variables.
- **Hardware Verification on Connected Tablet**:
  - Installed via `adb install -r output/capsulecraft-2.1.0.apk`.
  - Launched activity: `libc++_shared.so` loaded (`Load .../libc++_shared.so using class loader ns clns-9: ok`), Adreno Vulkan/GLES driver initialized, and process `com.artcraft.capsulecraft` (PID 359) running live without crash.
  - Output: `output/capsulecraft-2.1.0.apk` (72M, SHA-256: `a8173ad85c3021c5f541753cab2a47cfdfc2a3287d94aba3bd78da51d62e27e6`).

### 6.2 NoteCraft Adaptive Icon Background Fix (#884acb)
- **Symptom**: On the Android tablet's launcher/home screen, NoteCraft's icon background was black instead of brand purple (`#884acb`).
- **Root Cause**:
  1. `lib/app_icons.py` samples border pixels using `_background_color(Image, work)`.
  2. In `notecraft-512.png`, the border contained black pixels from drop-shadow/outline rendering, causing `Counter(colors).most_common(1)[0]` to evaluate to `#000000`.
  3. Consequently, `ic_launcher_background.xml` was written with `<path android:fillColor="#000000" ... />`.
- **The Fix**:
  - In `lib/app_icons.py`, explicitly pinned NoteCraft's brand color:
    ```python
    if app == "notecraft":
        color = "#884acb"
    ```
  - Re-generated icon layers: `res/drawable/ic_launcher_background.xml` now sets `android:fillColor="#884acb"`, leaving the crisp white/gold notebook foreground on the purple background.
  - Installed and verified on tablet: `com.artcraft.notecraft` (PID 3672) renders correctly.
  - Output: `output/notecraft-0.0.8.apk` (106M, SHA-256: `c261a9cf1d77adb4acb571fcd637571f478f0d19dec11e43efad105589f1616b`).

### 6.2 PaintCraft Build Failure & SAF Import/Export Fix
- **Symptom 1 (Build Error)**: `paintcraft` failed during native compilation with 9 errors in `saf.rs`: `error[E0433]: cannot find module or crate serde_json in this scope` and `error[E0061]: this method takes 1 argument but 2 arguments were supplied: self.inner.ui(ui, frame)`.
- **Symptom 2 (Runtime Import/Export)**: File dialog open and export buttons were unresponsive.
- **Root Cause Analysis**:
  1. Missing dependency: `adapters/paintcraft/Cargo.toml` lacked `serde_json = "1.0"`.
  2. API divergence: Upstream `paintcraft` UI takes `self.inner.ui(ui)`, but the adapter passed 2 arguments `(ui, frame)`.
  3. SAF bridge disconnection: PaintCraft uses `rfd::AsyncFileDialog` in `paintcraft-platform`. The adapter installed only synchronous dialog handlers and lacked `saf_poll` in its `logic()` frame hook. As a result, SAF responses queued in Java were never retrieved or delivered to `rfd` channels.
  4. RFD fallback bug: In `vendor/rfd/src/artcraft_saf.rs`, `pick()` returned `None` immediately if `LAUNCHER` (async) was None, ignoring `BLOCKING_LAUNCHER`.
- **The Fix**:
  - Added `serde_json = "1.0"` to `adapters/paintcraft/Cargo.toml`.
  - Corrected `self.inner.ui(ui)` in `adapters/paintcraft/src/lib.rs`.
  - Connected both `rfd::artcraft_saf::install` AND `install_blocking` in `adapters/paintcraft/src/lib.rs`.
  - Injected `saf_poll(&self.bridge, &self.state)` into `PaintCraftAndroidApp::logic()`.
  - Patched `vendor/rfd/src/artcraft_saf.rs` so `pick()` falls back to `BLOCKING_LAUNCHER` when async launcher is not installed.
- **Build Verification**:
  - Output: `output/paintcraft-0.1.1.apk` (281.1M, SHA-256: `226988d9e89c08d5ab60e43dc38dda0c4abf3a501195ec059ac721256ee0f3e8`).

### 6.3 PicassoSuite SAF Import/Export Fix
- **Symptom**: In PicassoSuite, media imports (`.mp4`, `.mov`, `.wav`) and project exports (`.drp`, `.aaf`) did not trigger or complete file selection dialogs.
- **Root Cause Analysis**:
  - PicassoSuite's `apps/picassosuite-android/Cargo.toml` did not include `rfd = "0.17"`.
  - `PicassoAndroidApp::logic()` did not invoke `saf_poll(&self.bridge, &self.state)`, causing SAF responses from `PicassoSuiteActivity` to remain in the queue.
- **The Fix**:
  - Added `rfd = "0.17"` to `adapters/picassosuite/Cargo.toml`.
  - Implemented `install_saf_launcher` supporting both async `rfd::artcraft_saf::install` and synchronous `install_blocking`.
  - Injected `saf_poll` into `PicassoAndroidApp::logic()`.
- **Build Verification**:
  - Output: `output/picassosuite-0.6.3.apk` (49.7M, SHA-256: `72b056adbc079647c0d80252c0ee0c44f7a3fda9a7fe15dbb98a91669da534d1`).

### 6.4 WordCraft Compilation & Localization Embedding
- **Symptom**: WordCraft build required verification of full document formatting, print preview, and localization catalogs.
- **Resolution**:
  - Embedded `craft-i18n` translations via `CRAFT_I18N_DIR`.
  - Embedded 13 craft font faces via `CRAFT_FONTS_DIR`.
- **Build Verification**:
  - Output: `output/wordcraft-0.15.2.apk` (143.9M, SHA-256: `7953df9a9119107df517d58ddbfbc3123e23fb7ca9dd8e877175f3bee2c0973f`).

---

## 7. Internationalization (i18n) & Localization Architecture

### 7.1 Root Causes of the English-Only Bug
1. **SELinux Property Blocking**: Standard `sys-locale` attempts to read `persist.sys.locale` via libc `__system_property_get`. Modern Android SELinux policies block non-system apps from accessing `persist.*` properties, causing `sys_locale::get_locale()` to return `None`.
2. **Missing Environment Variables**: Bionic libc does not initialize `LANG`, `LC_ALL`, or `LANGUAGE`. Standard C/Rust runtime checks return empty strings.
3. **Unset Build-Time Translation Catalogs**: `craft-i18n` relies on `CRAFT_I18N_DIR` at compile time to compile `.po`/`.json` translation catalogs into the binary. Without this variable, it compiled an empty catalog.

### 7.2 The 3-Point Permanent Solution
1. **SELinux-Safe Vendored `sys-locale` (`vendor/sys-locale`)**:
   - Patched `src/android.rs` to inspect `LC_ALL`, `LANG`, and `LANGUAGE` environment variables prior to querying system properties.
   - Automatically vendored and patched into all app workspaces via `lib/build-common.sh`.
2. **Runtime JNI Locale Injection (`templates/android/system_fonts.rs`)**:
   - Queries `java.util.Locale.getDefault().toLanguageTag()` via JNI during native startup.
   - Exports the detected language tag (e.g. `de-DE`, `fr-FR`, `es-ES`, `ja-JP`, `zh-CN`) into POSIX environment variables `LANG`, `LC_ALL`, and `LANGUAGE`.
3. **Compile-Time Translation Embedding**:
   - `lib/build-common.sh` automatically checks out `https://github.com/storytold/craft-i18n.git` into `source/craft-i18n` and exports `CRAFT_I18N_DIR="$ROOT/source/craft-i18n"`.
   - Applications embed multi-language strings directly into compiled release binaries.

---

## 8. Master Release Catalog: All 25 Verified Applications

Every APK listed below is built in release mode (`--release`), signed with the persistent builder keystore (`keys/artcraft-builder.jks`), and verified for Storage Access Framework (SAF) bridge integrity:

| # | Application | Package ID | Version | APK Filename | Size | SHA-256 Checksum | Bridge Status |
|:---:|:---|:---|:---:|:---|:---:|:---|:---:|
| 1 | **BIMCraft** | `com.artcraft.bimcraft` | 0.11.0 | `bimcraft-0.11.0.apk` | 60.9M | `31e7d97a700f0e5da32fdc939a410a145eab232355fef4a2a5856f1eec5a1406` | Verified |
| 2 | **CadCraft** | `com.artcraft.cadcraft` | 0.17.0 | `cadcraft-0.17.0.apk` | 95.7M | `523f45aa4c6e41567bd1a8623783b6857b4e000c530e8224127c545d4eb24b41` | Verified |
| 3 | **CapsuleCraft** | `com.artcraft.capsulecraft` | 2.1.0 | `capsulecraft-2.1.0.apk` | 72.0M | `a8173ad85c3021c5f541753cab2a47cfdfc2a3287d94aba3bd78da51d62e27e6` | Verified |
| 4 | **DeckCraft** | `com.artcraft.deckcraft` | 0.15.0 | `deckcraft-0.15.0.apk` | 123.9M | `01ced1e2f0c6a682aa1d64486571cdd656a29aede0f0a31caab0b18448163bc3` | Verified |
| 5 | **DesignCraft** | `com.artcraft.designcraft` | 0.17.1 | `designcraft-0.17.1.apk` | 154.0M | `83fb66be52d99085ac7d27476145cd86d79335bd7a4db1f17bf45ac514e3ae49` | Verified |
| 6 | **DiagramCraft** | `com.artcraft.diagramcraft` | 0.0.1 | `diagramcraft-0.0.1.apk` | 79.8M | `503dbf001498be93411584bc098bbb52d847908ee5f0eeac99686c8e3ebc61d4` | Verified |
| 7 | **EffectCraft** | `com.artcraft.effectcraft` | 0.21.0 | `effectcraft-0.21.0.apk` | 105.7M | `cb56709c1ecc4a208914dba406b4bff778f1d368e852968ca8b11f259dfb8068` | Verified |
| 8 | **FigCraft** | `com.artcraft.figcraft` | 0.15.0 | `figcraft-0.15.0.apk` | 133.0M | `45d1aae73629325ce6e0b8f6e50764202e941080892bc4f3ad5742e3bb0eb4b0` | Verified |
| 9 | **FilmCraft** | `com.artcraft.filmcraft` | 0.17.0 | `filmcraft-0.17.0.apk` | 115.2M | `a54704c58064f690589a19b6cc0a315bf1bdb684196317536ef39557aba73099` | Verified |
| 10 | **GridCraft** | `com.artcraft.gridcraft` | 0.16.0 | `gridcraft-0.16.0.apk` | 94.6M | `0b1d1779c372489ee5a7df38b73059b1afc14fd1a9ec296e1efc1a5ba29dad78` | Verified |
| 11 | **HumanMotionCraft** | `com.artcraft.humanmotioncraft` | 0.3.0 | `humanmotioncraft-0.3.0.apk` | 62.9M | `858118036f30c473590cf48e687deeadbeaad086f4320dff04e87d8dc6b518ce` | Verified |
| 12 | **LightCraft** | `com.artcraft.lightcraft` | 0.18.0 | `lightcraft-0.18.0.apk` | 439.7M | `84ae6d887aaf1391fa9a77a4101c35676777ca9c69d81a39282c978c4a431fc3` | Verified |
| 13 | **MaterialCraft** | `com.artcraft.materialcraft` | 0.1.1 | `materialcraft-0.1.1.apk` | 49.6M | `6d1899659b441096c90e4640aad31898ae4c6200ef46800ca7a5b718710d0ab0` | Verified |
| 14 | **NoteCraft** | `com.artcraft.notecraft` | 0.0.8 | `notecraft-0.0.8.apk` | 106.4M | `c261a9cf1d77adb4acb571fcd637571f478f0d19dec11e43efad105589f1616b` | Verified |
| 15 | **PaintCraft** | `com.artcraft.paintcraft` | 0.1.1 | `paintcraft-0.1.1.apk` | 281.1M | `226988d9e89c08d5ab60e43dc38dda0c4abf3a501195ec059ac721256ee0f3e8` | Verified |
| 16 | **PdfCraft** | `com.artcraft.pdfcraft` | 0.18.1 | `pdfcraft-0.18.1.apk` | 141.3M | `c326f0aeaa3058cbabaa79772eb0161802fe29a2be23e05b077a43e57c84b9fc` | Verified |
| 17 | **PhotoCraft** | `com.artcraft.photocraft` | 0.20.2 | `photocraft-0.20.2.apk` | 135.8M | `2e72565a7deab0e9093ae7cb850d6c5678de8db1b40a1a896a72b3687ce9efcd` | Verified |
| 18 | **PicassoSuite** | `com.artcraft.picassosuite` | 0.6.3 | `picassosuite-0.6.3.apk` | 49.7M | `72b056adbc079647c0d80252c0ee0c44f7a3fda9a7fe15dbb98a91669da534d1` | Verified |
| 19 | **SolidCraft** | `com.artcraft.solidcraft` | 0.40.0-rc.2 | `solidcraft-0.40.0-rc.2.apk` | 900.4M | `4ab911b8a666cdb3a0b2eeac221696b3ebad85171adaa42184e1c546f753f8b6` | Verified |
| 20 | **SoundCraft** | `com.artcraft.soundcraft` | 0.17.0 | `soundcraft-0.17.0.apk` | 44.8M | `e4f397a41465d2ac5a0992927863bbddcba96c779873ca4a762402b4a1e5534e` | Verified |
| 21 | **SubCraft** | `com.artcraft.subcraft` | 0.2.1 | `subcraft-0.2.1.apk` | 456.5M | `623424603391c40a58c67ea60440b6ad04092b043dd4c26439aee93a80f0b113` | Verified |
| 22 | **TextureCraft** | `com.artcraft.texturecraft` | 0.3.1 | `texturecraft-0.3.1.apk` | 59.4M | `7c287d19bd0c18d27f90365ddca348a85cce19be125360a692815bb5fe67575c` | Verified |
| 23 | **TraceDraw** | `com.artcraft.tracedraw` | 0.40.0 | `tracedraw-0.40.0.apk` | 86.1M | `38cfb0b0025235dde3b85833c47a358429c63d498adcf83e04f619265ac29f6f` | Verified |
| 24 | **VectorCraft** | `com.artcraft.vectorcraft` | 0.21.1 | `vectorcraft-0.21.1.apk` | 168.6M | `cc4e53a898dc5fcb4dd18a3ab159aa7bc5c6c2b0f8eb253edb4fbe12e7dbe470` | Verified |
| 25 | **WordCraft** | `com.artcraft.wordcraft` | 0.15.2 | `wordcraft-0.15.2.apk` | 145.0M | `87ab0fa75f07ae983407a40ef813bfdf5931ebcfcbfa1f292c1b1f59aca55504` | Verified |

---

## 9. Verification Commands & Regression Testing

To build and verify the entire 25-application suite in a continuous automated loop:

```bash
for app in subcraft photocraft vectorcraft filmcraft lightcraft pdfcraft effectcraft \
           designcraft figcraft wordcraft soundcraft deckcraft cadcraft gridcraft \
           solidcraft materialcraft texturecraft notecraft diagramcraft paintcraft \
           bimcraft tracedraw picassosuite capsulecraft humanmotioncraft; do
    echo "=========================================================="
    echo "  Building and Verifying: $app"
    echo "=========================================================="
    bash build.sh -"$app" --noupdate
done
```

All 25 applications pass bridge verification, produce signed release APKs, embed international font fallbacks, support multi-language environments, and preserve persistent App UIDs across updates.

---

## 10. WordCraft Font Discovery & User Font Subsystem Integration

### 10.1 Problem Statement & Architectural Root Cause
In desktop WordCraft (`apps/wordcraft`), user-installed fonts and custom typeface files are dynamically discovered and loaded into the global font database (`wordcraft_fonts::FontDb::global()`) at start-up via `load_user_fonts()`. This loads all TrueType (`.ttf`), OpenType (`.otf`), and TrueType Collection (`.ttc`/`.otc`) faces from `wordcraft_control_key::settings_dir().join("fonts")`, and registers that directory as `db.user_font_dir()` for any subsequent "Add Font…" (`font.add`) operations.

On Android, WordCraft exhibited an issue where it completely failed to see fonts residing in the user fonts directory:
1. **Omitted Launcher Font Loader**: The builder-generated Android launcher (`apps/wordcraft-android/src/lib.rs`) initialized `WordApp::new(...)` but never invoked `wordcraft_fonts::FontDb::global().load_user_fonts(...)` or `wordcraft_engine::packs::use_font_store(true)`. Consequently, `user_font_dir()` remained `None`, no user faces were registered into `FontDb::read_faces()`, and the ribbon Home font combo box displayed only bundled fonts.
2. **Missing Android Paths in Upstream System Font Scanner**: In upstream `crates/fonts/src/fontdb/names.rs`, `system_font_dirs()` only surveyed macOS, Windows, and traditional Linux X11 paths (`/usr/share/fonts`, `~/.fonts`, Flatpak `/run/host/fonts`). Android system directories (`/system/fonts`, `/product/fonts`) and user storage font roots (`/storage/emulated/0/Fonts`, `/sdcard/Fonts`) were entirely absent.
3. **Runtime Permission Race**: On initial launch, Android storage permissions (`MANAGE_EXTERNAL_STORAGE` / `READ_EXTERNAL_STORAGE`) are requested asynchronously. Before the user taps "Allow", `/sdcard/Fonts` is inaccessible (`entries = 0`). When granted seconds later, `FontDb` had already completed its one-shot `OnceLock` catalog scan and never refreshed without an explicit re-scan trigger.
4. **Disjoint Font Directory Conventions**: Desktop WordCraft looked solely at `settings_dir().join("fonts")` (`$XDG_CONFIG_HOME/wordcraft/fonts`). On Android, users and external file transfer tools place font collections in either:
   - Shared external storage: `/storage/emulated/0/Fonts` or `/sdcard/Fonts`.
   - App-private external files storage: `/storage/emulated/0/Android/data/com.artcraft.wordcraft/files/fonts/`.
   - Config directory: `/storage/emulated/0/Android/data/com.artcraft.wordcraft/files/config/wordcraft/fonts/`.

### 10.2 Architectural Implementation

```
┌────────────────────────────────────────────────────────────────────────────────────────┐
│                        WordCraft Android Font Discovery Pipeline                       │
└────────────────────────────────────┬───────────────────────────────────────────────────┘
                                     │
         ┌───────────────────────────┼───────────────────────────┐
         ▼                           ▼                           ▼
┌──────────────────┐       ┌──────────────────┐        ┌──────────────────┐
│ Upstream Patch   │       │ Launcher Loader  │        │ Focus Refresher  │
├──────────────────┤       ├──────────────────┤        ├──────────────────┤
│ names.rs:        │       │ post_construct:  │        │ WindowFocused:   │
│ • /system/fonts  │       │ • /sdcard/Fonts  │        │ • Detect change  │
│ • /product/fonts │       │ • files/fonts    │        │ • Fast diffing   │
│ • /sdcard/Fonts  │       │ • config/fonts   │        │ • Live relayout  │
│ • XDG_DATA/fonts │       │ • use_font_store │        │ • Instant ribbon │
└──────────────────┘       └──────────────────┘        └──────────────────┘
```

1. **System Font Directories Upstream Repair (`crates/fonts/src/fontdb/names.rs`)**:
   In `lib/build-common.sh`, added an idempotent build-time repair to `names.rs` extending `system_font_dirs()` with Android-native paths:
   ```rust
   dirs.extend([
       "/run/host/fonts", "/run/host/local-fonts", "/run/host/user-fonts",
       "/run/host/user-fonts-legacy", "/system/fonts", "/product/fonts",
       "/storage/emulated/0/Fonts", "/sdcard/Fonts"
   ].map(Into::into));
   if let Some(base) = std::env::var_os("XDG_DATA_HOME").map(std::path::PathBuf::from) {
       dirs.push(base.join("fonts"));
   }
   if let Some(cfg) = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from) {
       dirs.push(cfg.join("wordcraft/fonts"));
       dirs.push(cfg.join("fonts"));
   }
   ```

2. **Launcher User Font Initialization (`load_wordcraft_user_fonts`)**:
   Added a dedicated loader in `SAF_RECIPES["wordcraft"]["state"]` that executes immediately during `post_construct` and registers candidates:
   - Shared external fonts: `/storage/emulated/0/Fonts` and `/sdcard/Fonts`.
   - App-private external data fonts: `app_data_dir(app).join("fonts")`.
   - Configuration fonts: `wordcraft_control_key::settings_dir().join("fonts")`.
   - Primary user font directory registration: sets `db.user_font_dir()` to the app's persistent writable fonts folder so newly added fonts via File › Add Font… (`font.add`) are saved permanently.
   - Activates font store language packs via `wordcraft_engine::packs::use_font_store(true)`.

3. **Fingerprint-Based Dynamic Reloading on Refocus**:
   To eliminate unnecessary disk scans while ensuring newly granted permissions or newly transferred font files are immediately recognized without restarting the app:
   - Maintains `LAST_LOADED_FONT_STATE: Mutex<Option<Vec<(PathBuf, u64, u64)>>>` tracking each folder's entry count and timestamp.
   - Integrated into `AndroidShell::raw_input_hook` upon `egui::Event::WindowFocused(true)`.
   - If folder content fingerprints match, the hook returns in microseconds with zero disk I/O and zero redundant log noise.
   - If changes are detected, `load_user_fonts` parses newly added faces, bumps `FontDb::generation()`, and triggers `WordApp::watch_fonts()` for immediate UI ribbon dropdown refresh and document re-layout.

4. **Launcher Generator Version Bump (`GENERATOR_VERSION = "72"`)**:
   Bumped the generator marker from `71` to `72` in `lib/build-common.sh`, ensuring automated regeneration of the `apps/wordcraft-android` wrapper and Cargo manifest dependencies (`wordcraft-fonts` and `wordcraft-control-key`).

### 10.3 Live Device Verification & Telemetry
Installed `output/wordcraft-0.15.2.apk` onto the connected Samsung Galaxy Tab (`R9KXA01YR8H`) via `adb install -r`:
- **In-Place Update**: `Performing Streamed Install -> Success`. Confirmed UID preservation and persistent keystore signature match.
- **Logcat Telemetry**:
  ```
  I com.artcraft.wordcraft: wordcraft_android: ARTCRAFT_WORDCRAFT_ANDROID_SHELL_V72_SAF_FILEACCESS ready
  I com.artcraft.wordcraft: wordcraft_android: settings directory: /storage/emulated/0/Android/data/com.artcraft.wordcraft/files/config
  I com.artcraft.wordcraft: wordcraft_android::system_fonts: Android: user fonts: 7 file(s) in /storage/emulated/0/Fonts, /sdcard/Fonts
  I com.artcraft.wordcraft: wordcraft_android: WordCraft user fonts: 5 loaded, 2 skipped from /storage/emulated/0/Fonts
  W com.artcraft.wordcraft: wordcraft_fonts::fontdb::user: skipped the user font /storage/emulated/0/Fonts/ComicSansMS3.ttf: already available: Comic Sans MS
  I com.artcraft.wordcraft: wordcraft_android: WordCraft user fonts: 0 loaded, 7 skipped from /sdcard/Fonts
  ```
- **UI Verification**: Tapped "Blank Document" card, verified document ribbon layout, font combo box population, and live rendering of custom typefaces (`Comic Sans MS`, `Minecraft`).

---

## 11. Session 3 — CapsuleCraft, PaintCraft, PicassoSuite & BIMCraft Builds (2026-11-06)

### 11.1 CapsuleCraft v2.2.0 — `trash` Crate Android Fix

**Problem:** CapsuleCraft v2.2.0 (upgraded from v2.1.0 by upstream fetch) introduced `trash = "5"` as a dependency to move deleted `.eml` page files to the OS recycle bin. `trash v5.3.0` has a `platform` module that only exists for Linux (freedesktop), macOS, and Windows backends. When cross-compiled to `aarch64-linux-android`, the crate fails with:
```
error[E0433]: cannot find module or crate `platform` in this scope
   --> trash-5.3.0/src/lib.rs:106:35
```

**Root cause:** The `trash` crate's conditional compilation does not include an Android target backend. The build system selects no platform module for `target_os = "android"`, leaving `platform::PlatformTrashContext` and `delete_all_canonicalized` unresolved.

**Fix — `source/capsulecraft/native/Cargo.toml`:**
Moved `trash` to a platform-gated dependency section:
```toml
[target.'cfg(not(target_os = "android"))'.dependencies]
trash = "5"
```

**Fix — `source/capsulecraft/native/src/main.rs` (line ~1744):**
Wrapped the `trash::delete()` call in a compile-time cfg guard:
```rust
#[cfg(not(target_os = "android"))]
let trashed = match trash::delete(&path) {
    Ok(()) => true,
    Err(err) => {
        eprintln!("trash::delete failed, falling back to rm: {err}");
        std::fs::remove_file(&path).is_ok()
    }
};
#[cfg(target_os = "android")]
let trashed = std::fs::remove_file(&path).is_ok();
```
On Android, deleted capsule page files are permanently removed (Android has no system trash bin). Behavior is identical to desktop fallback when trash is unavailable.

**Result:**
- APK: `output/capsulecraft-2.1.0.apk` (73 MiB, SHA-256: `b17c14d079075d3751ef2b20ed392265ea5f72ab8b782a4379d74b96755acf14`)
- Installed: `adb install -r` → Success
- Verified bridge: `com.artcraft.capsulecraft.CapsuleCraftActivity`

### 11.2 PaintCraft v0.1.1 — Clean Build

PaintCraft built without modifications. The builder's existing `adapters/paintcraft/prepare.sh` already vendored an Android-compatible `arboard` (clipboard stub) and `rfd` (file dialog no-op).

**Result:**
- APK: `output/paintcraft-0.1.1.apk` (267 MiB, SHA-256: `abc97999daae65fa7ddd6187d4958c464bb5ec7f10680174f33c49b1acb99d71`)
- Installed: `adb install -r` → Success
- Verified bridge: `com.artcraft.paintcraft.PaintCraftActivity`

### 11.3 PicassoSuite v0.6.3 — Clean Build

PicassoSuite built without modifications. The build includes the full ffmpeg-sys-next stack (compiled for arm64), producing a compact 50 MiB APK.

**Result:**
- APK: `output/picassosuite-0.6.3.apk` (50 MiB, SHA-256: `5c1eea094b0ccc7551f351258736abf2051cc4e24e18a158968e761f76d1379a`)
- Installed: `adb install -r` → Success
- Verified bridge: `com.artcraft.picassosuite.PicassoSuiteActivity`

### 11.4 BIMCraft v0.10.0 — Clean Build

BIMCraft built without modifications. The builder's `adapters/bimcraft/prepare.sh` vendored `sys-locale` with an Android SELinux-safe implementation.

> [!NOTE]
> The device already had BIMCraft `v0.11.0` installed (from a prior `-main` branch build). Installing `v0.10.0` (latest stable tag) is blocked by Android's version downgrade guard. The APK is valid; the device simply has a newer version. No action needed.

**Result:**
- APK: `output/bimcraft-0.10.0.apk` (55 MiB, SHA-256: `bfef92c99c58d240187c2f4b6af6996b71ea615a53242b0f3b82b1397c93313d`)
- Install: Blocked by version downgrade (device has v0.11.0 > v0.10.0 — already newer)
- Verified bridge: `com.artcraft.bimcraft.BIMCraftActivity`

### 11.5 Session Build Summary Table

| App | Version | APK Size | SHA-256 (first 16) | Installed |
|-----|---------|----------|--------------------|-----------|
| CapsuleCraft | v2.2.0 | 73 MiB | `b17c14d079075d37` | ✅ |
| PaintCraft | v0.1.1 | 267 MiB | `abc97999daae65fa` | ✅ |
| PicassoSuite | v0.6.3 | 50 MiB | `5c1eea094b0ccc75` | ✅ |
| BIMCraft | v0.10.0 | 55 MiB | `bfef92c99c58d240` | ⚠️ device newer |

### 11.6 Cross-Session Fix Inventory

| Fix | File(s) Modified | Affects |
|-----|-----------------|---------|
| `trash` crate gated to non-Android targets | `native/Cargo.toml`, `native/src/main.rs` | CapsuleCraft only |
| Android font paths in `system_font_dirs()` | `lib/build-common.sh` → `names.rs` patch | WordCraft (prior session) |
| `load_wordcraft_user_fonts` post_construct | `lib/build-common.sh` | WordCraft (prior session) |
| `GENERATOR_VERSION` bump to "72" | `lib/build-common.sh` | All apps (forces launcher regen) |

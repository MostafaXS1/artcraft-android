# AGENTS.MD - ArtCraft Android Builder Technical Reference

## Table of Contents

- [1. Project Overview](#1-project-overview)
- [2. Supported Apps](#2-supported-apps)
- [3. Repository Structure](#3-repository-structure)
- [4. Build System Architecture](#4-build-system-architecture)
- [5. Entry Points and Orchestration](#5-entry-points-and-orchestration)
- [6. Build Pipeline Stages](#6-build-pipeline-stages)
- [7. Source Acquisition and Management](#7-source-acquisition-and-management)
- [8. Upstream Fixes and Patches](#8-upstream-fixes-and-patches)
- [9. Launcher Generation vs Adapters](#9-launcher-generation-vs-adapters)
- [10. Android Launcher Generation (Generator Details)](#10-android-launcher-generation-generator-details)
- [11. Cargo Workspace Patching and Vendoring](#11-cargo-workspace-patching-and-vendoring)
- [12. Android Project Generation](#12-android-project-generation)
- [13. Shared Runtime Modules](#13-shared-runtime-modules)
- [14. Storage Access Framework (SAF)](#14-storage-access-framework-saf)
- [15. File Associations and Intent Filters](#15-file-associations-and-intent-filters)
- [16. Soft Keyboard and IME Integration](#16-soft-keyboard-and-ime-integration)
- [17. System Fonts](#17-system-fonts)
- [18. Persistence and UI Scale](#18-persistence-and-ui-scale)
- [19. Staged Files and Cleanup](#19-staged-files-and-cleanup)
- [20. Printing](#20-printing)
- [21. Clipboard](#21-clipboard)
- [22. Updates](#22-updates)
- [23. Icons and Theming](#23-icons-and-theming)
- [24. Input: Mouse, Touch, Trackpad](#24-input-mouse-touch-trackpad)
- [25. Back Button and Lifecycle](#25-back-button-and-lifecycle)
- [26. Fullscreen and Edge-to-Edge](#26-fullscreen-and-edge-to-edge)
- [27. Signing and Build Output](#27-signing-and-build-output)
- [28. Verification](#28-verification)
- [29. Adapters (Custom Ports)](#29-adapters-custom-ports)
- [30. Vendored Crates](#30-vendored-crates)
- [31. CI/CD Pipeline](#31-cicd-pipeline)
- [32. Logging and Debugging](#32-logging-and-debugging)
- [33. Operational Notes and Failure Modes](#33-operational-notes-and-failure-modes)
- [34. Key Files Reference](#34-key-files-reference)
- [35. Agent Guidelines](#35-agent-guidelines)
- [36. Build Environment & Toolchain Deep Dive](#36-build-environment--toolchain-deep-dive)
- [37. Cargo Workspace Patching Details](#37-cargo-workspace-patching-details)
- [38. Android Manifest & Metadata Generation](#38-android-manifest--metadata-generation)
- [39. SAF Bridge Implementation Details](#39-saf-bridge-implementation-details)
- [40. IME Bridge & Soft Keyboard](#40-ime-bridge--soft-keyboard)
- [41. System Fonts Integration](#41-system-fonts-integration)
- [42. Persistence & Data Storage](#42-persistence--data-storage)
- [43. Input Handling](#43-input-handling)
- [44. Fullscreen, Edge-to-Edge, Theming](#44-fullscreen-edge-to-edge-theming)
- [45. Update Checker](#45-update-checker)
- [46. Icon Generation Details](#46-icon-generation-details)
- [47. Printing Support](#47-printing-support)
- [48. Debugging & Troubleshooting](#48-debugging--troubleshooting)
- [49. Performance Considerations](#49-performance-considerations)
- [50. Security & Permissions](#50-security--permissions)
- [51. Version History & Changelog Notes](#51-version-history--changelog-notes)
- [52. Contributing Guidelines](#52-contributing-guidelines)
- [53. Appendix](#53-appendix)
- [54. Model Context Protocol (MCP) Suite Architecture](#54-model-context-protocol-mcp-suite-architecture)
- [55. Stylus & Digitizer Subsystem](#55-stylus--digitizer-subsystem)
- [56. Gesture Engine & Touch Rejection](#56-gesture-engine--touch-rejection)
- [57. Mobile Text Selection Context Bar](#57-mobile-text-selection-context-bar)
- [58. Integrated Topbar Modifier Controls](#58-integrated-topbar-modifier-controls)
- [59. Platform Hardening & Build Reliability](#59-platform-hardening--build-reliability)

## 1. Project Overview

ArtCraft Android Builder produces locally-signed, 64-bit ARM (`arm64-v8a`) Android APKs from official upstream repositories of the ArtCraft suite. It uses a hybrid approach where desktop-native egui/eframe applications are wrapped with Android-specific launchers without modifying the upstream source (except for minor automated patches).

**Core Principles:**
- **Zero-touch app migration**: Upstream source remains desktop-agnostic
- **Template-based generation**: Shared Java/Rust templates provide common Android functionality
- **App-specific adapters**: Complex apps can have custom ports
- **Vendor patching**: Critical crates (winit, egui-winit, rfd, arboard, egui-wgpu, android-activity) are patched for Android support
- **Single orchestration**: `lib/build-common.sh` coordinates the entire build process

## 2. Supported Apps

From `lib/build-common.sh` and `README.md`:
```text
photocraft vectorcraft filmcraft lightcraft pdfcraft effectcraft
designcraft figcraft wordcraft soundcraft deckcraft cadcraft gridcraft
subcraft solidcraft materialcraft texturecraft
anicraft bimcraft capsulecraft diagramcraft earthcraft
humanmotioncraft mapcraft notecraft paintcraft picassosuite
prowave rustgis tracedraw transit
```
Total: 28 apps. CI matrix builds all of them.

## 3. Repository Structure

```
artcraft-android-builder/
├── build.sh                # Release build entry point
├── build-debug.sh          # Debug build entry point
├── clean.sh                # Cleanup script
├── architecture.md         # Architecture documentation
├── README.md               # User documentation
├── CHANGELOG.md            # Change history
├── lib/
│   ├── build-common.sh     # Main orchestrator (~4110 lines)
│   ├── android_project.py  # Generates AndroidManifest.xml, Gradle project, themes
│   ├── app_icons.py        # Generates adaptive/legacy launcher icons
│   ├── file_associations.py # Central intent filter registry
│   ├── scrolling.sh        # Two-finger scroll/momentum patches
│   ├── ime.sh              # WordCraft Backspace-at-paragraph-start patch
│   └── subcraft.sh         # SubCraft file handoff patch
├── templates/
│   ├── android/            # Shared Java/Rust modules
│   │   ├── SafActivity.java
│   │   ├── saf_bridge.rs
│   │   ├── system_fonts.rs
│   │   └── UpdateCheck.java
│   └── gradle/             # Gradle wrapper template
├── adapters/               # Per-app custom adapters
├── vendor/                 # Vendored patched crates
├── .github/workflows/      # CI configuration
├── docs/                   # Documentation
├── source/                 # Upstream checkouts (per-app, preserved)
├── build/                  # Isolated Cargo target dirs per app
├── output/                 # Built APKs
├── logs/                   # Build logs
└── keys/                   # Signing keystore and password
```

**Notes:**
- `source/` checkouts are preserved across builds (not reset)
- `build/<app>/target/` is isolated per app
- `android-sdk/` may be created by builder if missing
- `.cache/jdk/` for downloaded Temurin JDK if needed

## 4. Build System Architecture

### 4.1 Hybrid Orchestration Model
The system uses a hybrid approach:
- **Bash orchestration** (`lib/build-common.sh`): Toolchain verification, environment setup, git operations, build stage management, locking
- **Python logic** (embedded via heredocs): Cargo.toml parsing, manifest generation, template instantiation, dependency patching

### 4.2 Build Configuration
**Constants in `lib/build-common.sh`:**
```bash
BUILDER_VERSION="0.2.0"
UPDATE_API_URL="https://api.github.com/repos/MostafaXS1/artcraft-android/releases/latest"
UPDATE_PAGE_URL="https://github.com/MostafaXS1/artcraft-android/releases/latest"
GENERATOR_VERSION="55"  # Bumped when generated launcher template changes
```

**Repository mappings:**
```bash
REPO="https://github.com/storytold/$APP.git"  # Default for most apps
# SubCraft and SolidCraft exceptions
APP_ORIGIN="https://github.com/Aalzard/subcraft"
REPO="https://github.com/Aalzard/subcraft.git"
APP_ORIGIN="https://github.com/tangency-labs/solidcraft"
REPO="https://github.com/tangency-labs/solidcraft.git"
```

**Locking mechanism:**
- Uses `flock` on `.artcraft-build.lock` to ensure only one build runs at a time
- Records PID in lock file
- Prevents concurrent builds that would corrupt the launcher crate

## 5. Entry Points and Orchestration

### 5.1 `build.sh` / `build-debug.sh`
Simple wrappers that:
1. Set `MODE="release"` or `MODE="debug"`
2. Call `lib/build-common.sh "$MODE" "$@"`

### 5.2 `lib/build-common.sh` Main Flow
Key responsibilities in order:
1. Argument parsing and validation
2. App metadata resolution (TITLE, REPO, origins)
3. Environment setup and locking
4. Tool validation (git, cargo, python3 with tomllib)
5. SDK/JDK/NDK discovery and bootstrap
6. Source fetch/reuse
7. Upstream fixes and patches
8. Launcher preparation (adapter or generator)
9. Vendoring and Cargo.toml patching
10. Android project generation
11. Signing key management
12. Build execution (cargo-ndk + Gradle)
13. Verification
14. Output copy

## 6. Build Pipeline Stages

As indicated by `say 1` through `say 8` in the build process:

1. **Checking build tools...** - Verify git, cargo, python3 with tomllib, cargo-ndk present or installable
2. **Finding Android SDK and NDK...** - Discover/bootstrap SDK/NDK/JDK for Gradle
3. **Fetching official source for $TITLE...** - Clone or reuse source with integrity checks
4. **Preparing the GameActivity launcher...** - Run adapter prepare.sh or inline generator, apply patches
5. **Preparing a builder-owned signing key...** - Create or reuse JKS keystore
6. **Building $TITLE ($MODE)...** - Execute `cargo ndk` build followed by Gradle assemble
7. **Verifying the Android file bridge in the built APK...** - Run `verify_android_bridge()` function
8. **Build succeeded.** - Copy APK to output/, report details

## 7. Source Acquisition and Management

### 7.1 Reuse Logic
If `source/$APP` exists:
- Must be a git checkout (`.git` directory) - else fail
- Origin must match expected origin (`APP_ORIGIN` or `REPO` trimmed) - else fail
- `source_tree_is_complete()` checks tracked files in `crates` and `apps` exist on disk; if missing (interrupted clone) fail and ask to move aside

### 7.2 Clone Logic
If missing:
1. Check disk space: >= 1GB free in `source/`
2. Clone to scratch `source/.clone-$APP.$$`, verify complete, move into place
3. Clone with `--depth 1`
4. Ref selection based on flags:
   - `-stable` (default): find latest tag matching `v*` via `git ls-remote --tags --sort=-v:refname`, checkout tag if found else default branch
   - `-main|-latest`: default branch
   - `-tag=<tag>` or `-branch=<branch>`: use specified ref

### 7.3 Craft Fonts (Optional)
For apps embedding shared fonts (WordCraft, PdfCraft):
- Looks for `source/craft-fonts/fonts/manifest.txt`
- Clones from `https://github.com/storytold/craft-fonts.git` if missing (depth 1)
- Sets `CRAFT_FONTS_DIR` if present
- Used for Arabic, Thai, Georgian, Japanese fonts in PDFs

## 8. Upstream Fixes and Patches

### 8.1 Upstream Compile Fixes (Python in build-common.sh)
Guarded fixes applied only if broken text present and not already fixed:

**PhotoCraft:**
- Adds missing `export_presets: take(&mut m, "export_presets")` in `crates/engine/src/presets/mod.rs`

**CADCraft:**
- Downgrades dependencies due to egui 0.37 breakage:
  - egui/eframe/egui-wgpu to 0.36
  - harfrust to 0.13.3
  - quick-xml to 0.41
  - rstar to 0.12

### 8.2 App-Side Patches (Shell Scripts)

**`lib/scrolling.sh`** - Applied to all apps
- Patches app source for two-finger scrolling/momentum
- Works with vendored winit/egui-winit that recognize gestures
- Prevents selection on first finger
- Adds momentum (Android sends no momentum natively)

**`lib/subcraft.sh`** - Applied only to SubCraft
- Adds `files::deliver_android(purpose, paths)` to SubCraft
- Enables incoming files (intents/drops) to go into same queue as picked files

**`lib/ime.sh`** - Applied only to WordCraft
- Fixes Backspace at the start of a paragraph for IMEs that deliver `DeleteSurrounding` instead of text-state changes
- The Activity and Rust shell use android-activity's native IME; no Java host is patched

All patches are guarded/idempotent.

## 9. Launcher Generation vs Adapters

### 9.1 Detection Logic
```bash
if [[ -x "adapters/$APP/prepare.sh" ]]; then
    echo "Using app-specific adapter: adapters/$APP/prepare.sh"
    bash "adapters/$APP/prepare.sh" "$APP" "$TITLE" "$SRC" "$MODE"
else
    # Run inline Python generator
    python3 - "$ROOT" "$SRC" "$APP" "$TITLE" "$TARGET_SDK" "$APPNAME" "$PACKAGENAME" "$APPICON" <<'PY'
    ...
    PY
fi
```

### 9.2 Adapter Overview
Apps using custom adapters: `photocraft`, `vectorcraft`, `subcraft`
- Have complete control over Android shell
- May have custom JNI SAF bridges
- Copied into `source/$APP/apps/$APP-android/` by `prepare.sh`
- Builder does not rewrite adapter sources

### 9.3 Generated Launcher Overview
All other apps use the inline Python generator in `lib/build-common.sh` (starting around line ~613).
- Creates Android crate directory structure
- Generates Rust launcher code
- Configures Cargo.toml with dependencies
- Sets up shared modules

## 10. Android Launcher Generation (Generator Details)

### 10.1 Generator Constants and Flow
```python
GENERATOR_VERSION = "55"
# Marker file: .artcraft-android-builder-generated contains version
```

Generator process:
1. **Check for custom adapter** - if `adapters/$APP/Cargo.toml` and `adapters/$APP/src/lib.rs` exist, copy adapter
2. **Check for existing generated launcher** - if exists with matching marker, preserve and refresh shared modules
3. **Analyze upstream workspace** - parse root `Cargo.toml`, find crates, detect UI crate type
4. **Determine app structure** - detect Session, Services/Hooks patterns, constructors
5. **Generate launcher files** - create directory structure, copy shared modules, generate `src/lib.rs` and `Cargo.toml`

### 10.2 Shared Modules Copied
Always copied to generated launchers:
- `templates/android/system_fonts.rs` → `src/system_fonts.rs`

Soft-keyboard input does not use a shared module: the generated shell calls android-activity's native IME directly (see §16).

Conditionally copied if SAF needed:
- `templates/android/saf_bridge.rs` → included in generated code

### 10.3 Generated `src/lib.rs` Structure
The generator creates `android_main` function that wires:
- **Logging**: `android_logger` with app tag
- **Fullscreen**: `apply_immersive_fullscreen()`
- **Activity setup**: `egui_winit::android_activity::set_activity()`
- **Fonts**: `system_fonts::install_font_env()`
- **SAF bridge** (if needed): `saf::Bridge::new()`, request permissions
- **eframe setup**: Create `NativeOptions` with `Wgpu` renderer, `GameActivity`
- **App construction**: Call app constructor with appropriate arguments
- **Event handling**: Input modifiers, clipboard shortcuts (Ctrl/Cmd+C/X/V), back button mapping
- **Lifecycle**: Close guard for unsaved changes, native IME show/hide (WordCraft)
- **Persistence/UI scale**: Load prefs, apply device scale on first phone launch, save periodically

### 10.4 SAF Apps Configuration
```python
SAF_APPS = {"designcraft", "deckcraft", "gridcraft", "wordcraft", "lightcraft", 
           "effectcraft", "filmcraft", "cadcraft", "soundcraft", "pdfcraft", 
           "figcraft", "subcraft", "solidcraft", "materialcraft", "texturecraft"}
```
Note: PhotoCraft and VectorCraft use adapters (custom SAF), so not in SAF_APPS list.

### 10.5 Audio Modules
```python
AUDIO_MODULES = {
    "soundcraft": ("cpal", "soundcraft_android_audio.rs", "soundcraft audio"),
    # Others may be added
}
```
For SoundCraft, `cpal = "0.16"` is injected into launcher `Cargo.toml` and audio module copied.

### 10.6 Dependency Injection
Generated launcher's `Cargo.toml` gets:
```toml
[dependencies]
eframe = { version = "...", features = ["wgpu", "persistence", "android-game-activity"] }
egui = { version = "..." }
egui-winit = "..."
android-activity = { version = "0.6", default-features = false, features = ["game-activity"] }
log = "..."
android_logger = "0.15"
jni = "0.22"
serde_json = "..." # if needed
# Plus app-specific dependencies
```

## 11. Cargo Workspace Patching and Vendoring

For each build, `lib/build-common.sh` applies patches to `source/$APP/Cargo.toml` via Python regex replacements in `[patch.crates-io]`:

### 11.1 Vendored Crate Replacements
| Crate | Vendor Path | Purpose |
|-------|-------------|---------|
| `egui-winit` | `vendor/egui-winit` | Android clipboard via JNI, native IME integration |
| `rfd` (if used) | `vendor/rfd` | Android stub/backend routing through `rfd::artcraft_saf` |
| `arboard` (if used) | `vendor/arboard` | Android stub backend |
| `winit` | `vendor/winit` | Android backend patched for mouse/trackpad/wheel/hover, momentum scrolling |
| `egui-wgpu` | `vendor/egui-wgpu` | Surface recovery fix (release old surface before creating new) |
| `android-activity` | `vendor/android-activity` | ANR-safe callbacks, native GameActivity IME show/hide, exported `set_nonblocking_callbacks` |

### 11.2 Vendoring Process
1. Copy from `ROOT/vendor/*` to `SRC/vendor/*`
2. Remove `vendor/*/target` directories
3. Apply regex replacements in root `Cargo.toml` `[patch.crates-io]` section
4. For apps that already vendor winit, only overwrite `src/platform_impl/android`

## 12. Android Project Generation

`lib/android_project.py` reads launcher crate `[package.metadata.android]` and generates the Android Gradle project.

### 12.1 Generated Files
| File | Purpose |
|------|---------|
| `AndroidManifest.xml` | Permissions, application/activity entries, intent filters, meta-data |
| `res/values/themes.xml` | Theme.Artcraft style |
| `android/settings.gradle` | Gradle settings |
| `android/build.gradle` | AGP configuration, dependencies, signing |
| `android/gradle.properties` | Gradle properties |
| `android/gradlew` | Gradle wrapper (copied from `templates/gradle/`) |
| `.artcraft-gradle.properties` | Contains `MIN_SDK` for cargo-ndk |

### 12.2 Manifest Generation Details
- **No `<package>` attribute** - AGP sets namespace
- **No `<uses-sdk>`** - handled by defaultConfig
- **Permissions**: From `[[package.metadata.android.uses_permission]]` entries
- **Activity**: Generated from `[[package.metadata.android.application.activity]]`
- **Intent filters**: Includes MAIN/LAUNCHER, plus file associations
- **Meta-data**: `android.app.lib_name` set to crate name

### 12.3 Gradle Configuration
```groovy
// android/build.gradle (generated)
plugins {
    id 'com.android.application'
}

android {
    namespace 'com.artcraft.<app>'
    compileSdk 35
    
    defaultConfig {
        applicationId 'com.artcraft.<app>'
        minSdk <from min_sdk_version>
        targetSdk 35
        versionCode <calculated>
        versionName '<version>'
    }
    
    signingConfigs {
        release {
            storeFile file('../../keys/artcraft-builder.jks')
            storePassword System.getenv('ARTCRAFT_KEYSTORE_PASSWORD')
            keyAlias 'artcraftbuilder'
            keyPassword System.getenv('ARTCRAFT_KEY_PASSWORD')
        }
    }
    
    buildTypes {
        release {
            signingConfig signingConfigs.release
            minifyEnabled false
        }
    }
    
    sourceSets {
        main {
            manifest.srcFile '../AndroidManifest.xml'
            java.srcDirs = ['../java']
            res.srcDirs = ['../res']
            jniLibs.srcDirs = ['../jniLibs']
        }
    }
}
```

**Dependencies:**
- `androidx.appcompat:appcompat`
- `androidx.core:core`
- `androidx.games:games-activity`
- `org.jetbrains.kotlin:kotlin-bom`

### 12.4 Version Code Calculation
```python
versionCode = (1 << 24) | (major << 16) | (minor << 8) | patch
```
From app semver (ndk-build2 compatible).

### 12.5 Theme
`res/values/themes.xml`:
```xml
<style name="Theme.Artcraft" parent="Theme.AppCompat.NoActionBar">
    <item name="android:windowFullscreen">true</item>
    <item name="android:windowBackground">@android:color/black</item>
</style>
```

## 13. Shared Runtime Modules

### 13.1 `templates/android/SafActivity.java`
**Purpose:** Shared Activity base class extending `GameActivity` (package `__PACKAGE__`, class `__CLASS__`)

**Key Features:**

**Permissions:**
- `safRequestStoragePermission()` - Requests MANAGE_EXTERNAL_STORAGE (API 30+) or READ/WRITE_EXTERNAL_STORAGE (API < 30), plus `RECORD_AUDIO` on API 23+
- `safRequestMicrophonePermission()` - Separate method for mic permission

**SAF Pickers:**
- `safRequestOpen(long id, boolean multiple)` - `ACTION_OPEN_DOCUMENT`
- `safRequestSave(long id, String suggestedName)` - `ACTION_CREATE_DOCUMENT`
- `safRequestFolder(long id)` - `ACTION_OPEN_DOCUMENT_TREE`
- Results queued in `ConcurrentLinkedQueue<String> results`, polled by `safPoll()`

**File Staging:**
- Copies picked documents to app-private external dir: `/storage/emulated/0/Android/data/<pkg>/files/media/`
- Collision handling: adds ` (2)`, ` (3)` etc if name taken
- Name sanitization
- Reuses existing staged file if name matches
- `SharedPreferences PREFS = "<pkg>_saf_uri_map"` maps `uri:<stagedPath>` → SAF `content://` URI
- `safCommit(String path)` writes staged file to remembered SAF URI via `ContentResolver.openOutputStream`
- Falls back to matching by stem if exact path not found

**Intent Handling:**
- `handleIncomingFiles(Intent)` processes ACTION_VIEW, ACTION_SEND, ACTION_SEND_MULTIPLE
- Copies URIs off UI thread to staging
- Queued in `ConcurrentLinkedQueue<String> intentPaths`, polled by `intentPoll()`
- Supports drag-drop via `View.OnDragListener`

**IME (Soft Keyboard):**
- No Java host: the Activity has no `EditText`, `InputConnection`, or `imeShow`/`imeHide`/`imePoll`
- android-activity/GameActivity owns the native IME; the Rust shell sets the editor info, resets and polls `text_input_state`, and calls `show_soft_input`/`hide_soft_input`

**Printing:**
- `printDocument(String title, byte[] pdfBytes)` creates temp PDF, uses `PrintManager` with `PdfPrintAdapter`
- Returns `"OK"` or error string

**UI/Display:**
- `applyImmersiveFullscreen()` - Hides system bars (sticky immersive)
- `applyEdgeToEdge()` - Draws under system bars
- `installCaptionBarInsets()` - Handles desktop window title bars (Samsung DeX)
- `setBarIconAppearance(boolean lightTheme)` - Syncs status/nav bar icons with theme
- `suppressSelectionHandles()` - Disables text selection handles over native surface
- `isHighDensityDevice()` - Returns true if `smallestScreenWidthDp < 600` (phone-class)
- `onDestroy()` - Kills process on finish to avoid winit ANR issues

**Drag and Drop:**
- `installDropTarget()` sets `OnDragListener`
- Accepts file drags, persists permissions, stages files

### 13.2 `templates/android/saf_bridge.rs`
**Purpose:** Rust JNI bridge for SAF operations

**Constants:**
```rust
pub const SAF_MARKER: &str = "ARTCRAFT_ANDROID_SAF_BRIDGE_V3";
const PICK_TIMEOUT: Duration = Duration::from_secs(180);
```

**NonblockingCallbacks RAII:**
```rust
struct NonblockingCallbacks;
impl NonblockingCallbacks {
    fn set() -> Self { android_activity::set_nonblocking_callbacks(true); NonblockingCallbacks }
}
impl Drop for NonblockingCallbacks { fn drop(&mut self) { android_activity::set_nonblocking_callbacks(false); } }
```
Prevents ANR during blocking SAF picker calls.

**Bridge Structure:**
```rust
pub struct Bridge {
    app: AndroidApp,
    vm: Arc<JavaVM>,
}
```
Methods:
- `new(app: &AndroidApp) -> Self`
- `pick_open_blocking(id: u64, multiple: bool, timeout: Duration) -> Result<PickAnswer>`
- `pick_save_blocking(id: u64, name: &str, timeout: Duration) -> Result<PickAnswer>`
- `pick_folder_blocking(id: u64, timeout: Duration) -> Result<PickAnswer>`
- `commit(path: &str) -> Result<()>`
- `files_dir() -> Result<PathBuf>`
- `intent_pending() -> Result<u32>`
- `intent_poll() -> Result<Option<String>>`
- `print_document(title: &str, pdf: &[u8]) -> Result<String>`
- `request_storage_permission()`
- `poll() -> Option<String>` - Get SAF results

**PickAnswer:**
```rust
pub enum PickAnswer {
    Paths(Vec<String>),
    Cancelled,
}
```

**SaveMirror:**
Tracks staged save paths by ID, watches for writes, calls `commit()` to mirror to SAF URI. Scans periodically (~200ms) to detect writes. A file that appears *beside* the staged path after the pick (the app adds or replaces the extension then: GridCraft's "Book" becomes "Book.xlsx") is the app's write and is committed as well — it is adopted with an "unseen" baseline, so it is committed even when the app had already finished writing it (otherwise the document the user chose stays 0 B). A sibling that already existed when the pick happened keeps its own stamp and is never pushed into the new document.

**JSON Protocol:** Results passed as JSON strings over JNI queue.

### 13.3 Soft keyboard (native IME)

**Purpose:** turn Android's native IME text state into egui events. There is no `ime_bridge.rs` module.

```rust
// In the generated AndroidShell / adapter:
let editing = ctx.egui_wants_keyboard_input() || ctx.output(|o| o.ime.is_some());
if editing {
    let now = android_app.text_input_state().text;
    // diff prev vs now -> Backspace keys, Event::Text, Enter key
    ctx.request_repaint_after(Duration::from_millis(16));
}
```

See §16 for the full flow.

### 13.4 `templates/android/system_fonts.rs`
**Purpose:** System font integration

**Functions:**
```rust
pub fn install_font_env(app: &AndroidApp) // Sets HOME, XDG_DATA_HOME to internal dir
pub fn install_system_fonts(ctx: &egui::Context) // Adds system fonts to egui families
```

**Behavior:**
- Scans `/system/fonts`, `/product/fonts` each frame
- Links to internal dirs: `$HOME/.fonts`, `$HOME/.local/share/fonts`, `$XDG_DATA_HOME/fonts`
- Creates minimal `fonts.conf`
- Discovers user fonts in `/storage/emulated/0/Fonts` after MANAGE_EXTERNAL_STORAGE granted
- Deduplicates via single real folder with symlinks
- Migrates old single-link setups
- Refreshes every few seconds
- HOME/XDG_DATA_HOME point to internal (needed for symlinks, external doesn't allow)

### 13.5 `templates/android/UpdateCheck.java`
**Purpose:** Update checker

```java
public class UpdateCheck {
    public static void start(Activity activity) {
        // Baked builder version: __BUILDER_VERSION__
        // Checks GitHub API after 5s delay
        // Notifies once per version if newer
    }
}
```
Needs `INTERNET` permission. Compares against `https://api.github.com/repos/MostafaXS1/artcraft-android/releases/latest`

## 14. Storage Access Framework (SAF)

### 14.1 Design Principles
- **Staging approach**: Documents opened via SAF are copied to app-private storage
- **Write-back**: Apps write to staged paths; SaveMirror detects writes and calls safCommit
- **URI persistence**: SharedPreferences maps staged paths to SAF URIs
- **Blocking with ANR prevention**: Uses NonblockingCallbacks during picker operations
- **Timeout**: 180 seconds for blocking picks

### 14.2 Storage Locations
**App-private external directory (preferred):**
`/storage/emulated/0/Android/data/<pkg>/files/`

**Subdirectories:**
- `files/media/` - Staged documents from SAF
- `files/config/` - App preferences/config
- `files/cache/` - App cache
- `files/autosave/`, `files/recovery/` - For apps with autosave (e.g., PhotoCraft)

**Internal directory (fallback):**
`/data/user/0/<pkg>/files/` - Used when external unavailable, and for font environment

### 14.3 SAF Flow

**Opening files:**
1. Rust calls `bridge.pick_open_blocking(id, multiple, timeout)`
2. Java shows `ACTION_OPEN_DOCUMENT` picker
3. User selects files, results returned via `onActivityResult`
4. Files copied off UI thread to `files/media/` with collision handling
5. URIs persisted in SharedPreferences
6. Results queued as JSON, polled by Rust

**Saving files:**
1. Rust calls `bridge.pick_save_blocking(id, name, timeout)`
2. Java shows `ACTION_CREATE_DOCUMENT` with suggested name
3. User chooses location, URI persisted
4. Empty staged file created
5. SaveMirror watches for writes to staged path
6. On write detection, calls `safCommit()` to copy to SAF URI

**safCommit logic:**
1. Look up URI by exact path in SharedPreferences
2. If not found, try `uriForSibling()` - match by stem (handles extension changes)
3. Open output stream to SAF URI (`"wt"` or `"w"`)
4. Copy staged file bytes to SAF URI
5. Return `"OK"` or error

### 14.4 SAF Apps List
```python
SAF_APPS = {"designcraft", "deckcraft", "gridcraft", "wordcraft", "lightcraft", 
           "effectcraft", "filmcraft", "cadcraft", "soundcraft", "pdfcraft", 
           "figcraft", "subcraft", "solidcraft", "materialcraft", "texturecraft"}
```
PhotoCraft and VectorCraft use custom adapters with similar SAF patterns.

### 14.5 Intent Opening
Apps can receive files via:
- `ACTION_VIEW` - Open with
- `ACTION_SEND` - Share single file
- `ACTION_SEND_MULTIPLE` - Share multiple files
- Drag and drop

**Flow:**
1. `handleIncomingFiles()` in Activity copies URIs to staging off UI thread
2. Queued in `intentPaths`
3. Apps call `bridge.intent_pending()` periodically (watcher thread every 200ms)
4. Apps call `bridge.intent_poll()` to get paths
5. Apps route to appropriate handlers (e.g., `files::deliver_android()` in SubCraft)

**Note:** `intentPending()` is only defined in adapters (PhotoCraft/VectorCraft/SubCraft). Generated launchers may have issues if not handled properly.

## 15. File Associations and Intent Filters

`lib/file_associations.py` is the single source of truth.

### 15.1 Structure
```python
EXT_MIME = {
    ".pdf": ["application/pdf"],
    ".psd": ["image/vnd.adobe.photoshop", "image/x-photoshop"],
    ".png": ["image/png"],
    # ...
}

ASSOCIATIONS = {
    "pdfcraft": [".pdf"],
    "wordcraft": [".docx", ".doc", ".rtf", ".odt", ".txt"],
    "photocraft": [".psd", ".png", ".jpg", ".jpeg", ".tif", ".tiff", ".webp", ".raw", ...],
    # ...
}
```

### 15.2 Intent Filter Generation
`intent_filter_toml(app)` builds full intent filter array:

Always includes:
- MAIN/LAUNCHER

For apps with MIME types:
- VIEW with mimes, scheme content/file, categories DEFAULT/BROWSABLE
- VIEW http/https with mimes (browsable)
- SEND with mimes
- SEND_MULTIPLE with mimes

For apps with extensions:
- VIEW by extension (no MIME): scheme content/file, host "*", pathPattern `.*\.<ext>` (properly escaped)
- VIEW by extension with mimeType="*/*": For octet-stream cases where MIME not reported

**Important notes:**
- pathPattern requires host="*" to be evaluated
- Duplicate extensions/mimes handled
- Extensions matched by filename even if provider reports generic MIME type

### 15.3 Patching
- Generator patches intent filters into generated Cargo.toml
- Adapters call `file_associations.patch_cargo_toml()` in their prepare.sh
- Applied during build process

## 16. Soft Keyboard and IME Integration

### 16.1 Architecture
- No Java host: android-activity/GameActivity owns the native soft keyboard
- The Rust shell sets the editor info and text state when editing starts
- Each frame it polls `AndroidApp::text_input_state().text` and diffs the buffer
- Changes become egui `Backspace`/`Text`/`Enter` events pushed into `raw.events`

### 16.2 Event Flow
1. egui reports that a field wants the keyboard (`egui_wants_keyboard_input()`), or the canvas asks through egui's IME output (`ctx.output(|o| o.ime.is_some())`)
2. On the editing edge the shell calls `set_ime_editor_info`, resets `set_text_input_state`, and calls `show_soft_input(false)`
3. Every frame the shell reads `text_input_state().text`
4. `ime_diff` counts removed characters (Backspace) and collects added text/Enter
5. The events are pushed into the frame's `RawInput`, and a 16 ms repaint keeps frames coming

### 16.3 Backspace Handling
- Removed characters are emitted as `egui::Event::Key { key: Backspace }`
- WordCraft's `lib/ime.sh` additionally fixes IMEs that still deliver `DeleteSurrounding` at a paragraph start
- No duplicate-pair guard is needed: the native text state is authoritative

### 16.4 Wake Mechanism
- GameActivity text events do not wake winit's event loop
- While editing the shell calls `ctx.request_repaint_after(Duration::from_millis(16))`, so a key appears in the next frame
- A pointer press or a non-Escape key re-shows the keyboard if Android's Back key dismissed it

### 16.5 Configuration
```rust
app.set_ime_editor_info(
    InputType::TYPE_CLASS_TEXT | InputType::TYPE_TEXT_FLAG_MULTI_LINE,
    TextInputAction::None,
    ImeOptions::IME_FLAG_NO_FULLSCREEN,
);
app.set_text_input_state(TextInputState { text: String::new(), selection: TextSpan { start: 0, end: 0 }, compose_region: None });
app.show_soft_input(false);
```
Keyboard overlays surface, doesn't resize it.

## 17. System Fonts

### 17.1 Overview
Loads Android system fonts as fallback for all egui font families. Supports CJK, Arabic, Hebrew, Indic, Thai, Khmer, Myanmar etc.

### 17.2 Font Loading
**Sources:**
- `/system/fonts/`
- `/product/fonts/`

Added as **lowest priority** in every egui font family (re-checked each frame).

### 17.3 Font Linking
**Internal storage (required for symlinks):**
- `$HOME/.fonts`
- `$HOME/.local/share/fonts`
- `$XDG_DATA_HOME/fonts`
All point to same real directory with symlinks

**Setup:**
- `install_font_env()` sets `HOME` and `XDG_DATA_HOME` to internal dir
- Creates minimal `fonts.conf`
- Deduplicates fonts
- Migrates old single-link setups

### 17.4 User Fonts
**Locations scanned:**
- `/storage/emulated/0/Fonts/`
- `/sdcard/Fonts/`
- Subdirectories

Requires `MANAGE_EXTERNAL_STORAGE` permission (granted via launcher prompt).

### 17.5 Refresh Mechanism
- Links checked every few seconds
- New fonts picked up without relaunch
- If granted permission after start, fonts become available

### 17.6 PDF Font Configuration
PdfCraft uses fontconfig pointing to device fonts by desktop-compatible names (e.g., NotoSansArabic-Regular.ttf).

## 18. Persistence and UI Scale

### 18.1 Persistence
**Storage location:** App-private external directory
`/storage/emulated/0/Android/data/<pkg>/files/`

**Environment variables set:**
```rust
std::env::set_var("XDG_CONFIG_HOME", &config);
std::env::set_var("XDG_CACHE_HOME", &cache);
// HOME and XDG_DATA_HOME set to internal dir for fonts
```

**Paths:**
- Config: `files/config/`
- Cache: `files/cache/`
- Preferences saved/restored at startup
- Autosave/recovery in `files/autosave/`, `files/recovery/` (PhotoCraft)

Apps use path helpers through `XDG_CONFIG_HOME`/`XDG_CACHE_HOME`.

### 18.2 UI Scale
**First launch on phone-class devices only:**
- Device class determined by `isHighDensityDevice()`: `smallestScreenWidthDp < 600` = phone-class
- Scale applied once via `device_scale_first_time(dir)` using `device_class` sidecar file
- Scale written back via `persist_save`
- Tablets left unchanged

**Default scales:**
- Document apps (PhotoCraft): 150%
- Canvas apps (VectorCraft): 75%
- Others: 85% (per recipe)
- Applied after `persist_load`

**Control:** Uses sidecar file independent of settings so upgrades get correct scale.

## 19. Staged Files and Cleanup

### 19.1 Staged Files
Location: `files/media/`

**Tracking:**
- SAF-opened documents copied here
- Recent paths tracked by app
- `uri:` mappings in SharedPreferences kept for re-opening

### 19.2 Cleanup Rules
- When files leave recent list and not open → staged copy deleted (original SAF URI remains)
- Delayed sweep after 30s clears orphaned staged copies not referenced and older than 60s
- Name collision cleared when staged copy deleted (can reuse suffix)
- Pruning runs periodically

## 20. Printing

**Supported apps:** WordCraft, GridCraft

**Flow:**
1. Rust calls `bridge.print_document(title, pdf_bytes)`
2. Java creates temp PDF file
3. Uses `PrintManager.print()` with custom `PdfPrintAdapter`
4. System print dialog shown (user selects printer, options)
5. Returns `"OK"` or error string

**Notes:** Printer enumeration done by Android system, not app.

## 21. Clipboard

- **Patched egui-winit** provides system clipboard via JNI `ClipboardManager`
- **Shell shortcuts** handle Ctrl/Cmd+C/X/V, synthesizing egui Copy/Cut/Paste events
- Works across apps

## 22. Updates

**UpdateCheck.java:**
- Runs 5 seconds after launch in background thread
- Checks `https://api.github.com/repos/MostafaXS1/artcraft-android/releases/latest`
- Compares baked `BUILDER_VERSION` with latest release tag
- Shows notification once per version if newer
- Links to `UPDATE_PAGE_URL`
- Fails silently (no network, rate limit, etc.)
- Needs `INTERNET` permission

**BUILDER_VERSION:** Set in `lib/build-common.sh` (currently 0.2.0)

## 23. Icons and Theming

### 23.1 Icon Generation
**`lib/app_icons.py`:**
- Prefers explicit `-appicon` flag
- Then app's vector icon
- Then largest PNG in app
- Never uses other ArtCraft app branding

**Outputs:**
- Legacy mipmaps (all densities)
- Adaptive icons (API 26+):
  - Background: solid brand color vector
  - Foreground: artwork on transparent, inside 108dp safe zone
  - Monochrome: same artwork for themed icons (Android 13+)
- Same treatment for adapters

### 23.2 Theming
- Follows system light/dark (`prefers-color-scheme`), live updates
- In-app Appearance setting can override (system default)
- Status/nav bar icons adjusted per theme
- Theme: `Theme.Artcraft` (AppCompat.NoActionBar), fullscreen, black background
- `setBarIconAppearance()` syncs icons with app theme

## 24. Input: Mouse, Touch, Trackpad

### 24.1 Mouse Support (Patched winit)
- Hover, buttons, wheel, trackpad
- Mouse wheel turns in notches (no coasting)
- Secondary click for context menus
- **Release Event Fix (`primary_button_on_up`)**: In stock winit Android implementations, `primary_button_on_up` checked `if *primary_down && !(button_state & BUTTON_PRIMARY != 0)`. On many Android mice/touchpads, `MotionEvent.getButtonState()` during `ACTION_UP` still contains `BUTTON_PRIMARY` (identifying the button that triggered the action), causing `primary_still_held` to evaluate as `true`. This suppressed the `Released` event, permanently leaving `*primary_down = true`, which dropped all subsequent clicks across the entire application. The vendored winit in `vendor/winit/src/platform_impl/android/device_kind.rs` clears `*primary_down = false` and emits `Released` whenever an `ACTION_UP` occurs, guaranteeing reliable click delivery.

### 24.2 Trackpad
- Smooth scrolling with coasting
- Two-finger slide scrolls 1:1 in pixels
- Two-finger pinch zooms
- Device classification detects trackpad vs mouse

### 24.3 Touch
- Two-finger slide scrolls document
- Two-finger pinch zooms
- Gesture recognition with deadzone:
  - Slide > finger width → scroll
  - Pinch distance change → zoom
  - Otherwise ignored (prevents accidental zoom during scroll)
- Flick coasting
- Never starts text selection
- Patched scrolling prevents selection on first finger

### 24.4 App-Side Patches
`lib/scrolling.sh` applied to all apps for enhanced gesture handling.

## 25. Back Button and Lifecycle

### 25.1 Back Button Handling
- winit consumes back key, reports as `Key::BrowserBack`
- Shell maps `BrowserBack` to `Key::Escape`
- If app consumes Escape → handled
- If not consumed and app has close guard (`close_requested()`) → show app's quit prompt
- If not consumed and no guard → finish activity

### 25.2 Close Guard
Apps with unsaved changes:
- `close_requested()` returns false → shell sets `close_asked=true`, doesn't finish
- App shows its own "quit without saving?" prompt
- User confirms → app sets `quit_requested=true`
- Shell detects `quit_requested` → calls `finish_activity()`

### 25.3 Lifecycle
- `onCreate()`: Setup immersive fullscreen, edge-to-edge, caption insets, selection suppression, drop target, IME, handle incoming files
- `onNewIntent()`: Handle files opened while app running (singleTask)
- `onConfigurationChanged()`: Refresh window insets
- `onWindowFocusChanged()`: Re-assert insets on focus
- `onDestroy()`: If finishing, shutdown executor, wait 20s for staged copies, kill process (prevents winit ANR)

## 26. Fullscreen and Edge-to-Edge

### 26.1 Immersive Fullscreen
- Sticky immersive mode
- Bars hidden until swipe
- Re-asserted on focus/config changes

### 26.2 Edge-to-Edge
- `setDecorFitsSystemWindows(false)` on API 30+
- Layout flags on older Android
- Transparent status and navigation bars
- Content draws behind bars

### 26.3 Bar Icon Appearance
- Synced to app theme (light/dark)
- Light theme → dark icons (APPEARANCE_LIGHT_STATUS_BARS)
- Dark theme → light icons
- Updated per frame via `setBarIconAppearance()`

### 26.4 Caption Bar Insets (Desktop Mode)
Handles Samsung DeX and freeform windows:
- Reserves space for window title bar
- Padding recomputed on every inset dispatch
- Strip painted in app theme color
- Forwarded to keyboard patch listener

### 26.5 Selection Suppression
Disables text selection handles and action modes over native surface (long press blocked).

## 27. Signing and Build Output

### 27.1 Signing
**Keystore:** `keys/artcraft-builder.jks` (JKS, RSA 2048, 10000 days validity)
**Created if missing:** `keytool -genkeypair`
**Password:** `keys/.keystore-password` (random if creating new, 'android' for compatibility if exists from older builds)
**Alias:** `artcraftbuilder`

**Environment variables passed to Gradle:**
```bash
ARTCRAFT_KEYSTORE="$ROOT/keys/artcraft-builder.jks"
ARTCRAFT_KEYSTORE_PASSWORD="$(cat "$ROOT/keys/.keystore-password")"
ARTCRAFT_KEY_ALIAS="artcraftbuilder"
ARTCRAFT_KEY_PASSWORD="$ARTCRAFT_KEYSTORE_PASSWORD"
```

### 27.2 Output Paths
| Path | Contents |
|------|----------|
| `output/<app>-<version>.apk` | Release APK |
| `output/<app>-<version>-debug.apk` | Debug APK |
| `source/<app>/` | Source checkout + generated GameActivity crate |
| `build/<app>/target/` | Isolated Cargo build output |
| `logs/<app>-<mode>-<timestamp>.log` | Build logs |
| `keys/artcraft-builder.jks` | Persistent signing keystore |

**APK naming:** Version from app's Cargo.toml
**Target:** `arm64-v8a` only

**Note:** Keystore is for local builds only. Do not use for public distribution.

## 28. Verification

### 28.1 Build-Time Verification
`verify_android_bridge(app, apk)` function:
1. Checks if app needs APK2/bridge (in `SAF_APPS` or adapter has java_sources)
2. Reads manifest from adapter or generated launcher to get package/activity and MAIN/LAUNCHER
3. If using aapt2, checks launchable-activity
4. Checks APK contains Activity class in dex and has .so files in `lib/`
5. Fails build if bridge missing

### 28.2 CI Verification
- Checks APK size >= 1MB
- Runs `unzip -tq` on APK
- Validates all artifacts before bundling/release

## 29. Adapters (Custom Ports)

Located in `adapters/`:

### 29.1 PhotoCraft
- **Files:** `adapters/photocraft/Cargo.toml`, `src/lib.rs`, `java/`, `res/`
- **Activity:** `PhotoCraftActivity`
- **Features:** Custom JNI SAF bridge (`pcRequestOpen/Save`, `pcPollPickerResult`), custom persistence/intent handling
- **Autosave:** Implements full crash recovery with 6 Services callbacks (see PhotoCraft crash recovery report)

### 29.2 VectorCraft
- **Files:** `adapters/vectorcraft/Cargo.toml`, `src/lib.rs`, `java/`, `res/`
- **Activity:** `VectorCraftActivity`
- **Features:** Custom JNI SAF bridge (`vcRequestOpen/Save`, `vcPollPickerResult`), uses `start_pick` pattern

### 29.3 SubCraft
- **Files:** `adapters/subcraft/Cargo.toml`, `src/lib.rs`, `java/`, `assets/`
- **Activity:** `SubCraftActivity`
- **Features:** Inlines SAF bridge (`src/saf.rs`), uses vendored rfd with Android backend, has `files::deliver_android` support via `subcraft.sh`
- **File handoff:** Handles incoming files via intents/drops

**Adapter structure:**
- `Cargo.toml` - Android launcher crate config
- `src/lib.rs` - Launcher code
- `res/` - Resources (icons, themes)
- `java/` - Java sources
- `prepare.sh` (optional) - Custom preparation script

**Adapter preparation:**
```bash
# If adapters/$APP/prepare.sh exists and executable
bash "adapters/$APP/prepare.sh" "$APP" "$TITLE" "$SRC" "$MODE"
```
prepare.sh copies adapter into `source/$APP/apps/$APP-android/`, removes prepare.sh/README.md, copies shared modules (system_fonts.rs).

## 30. Vendored Crates

Located in `vendor/` with Android-specific patches:

| Crate | Purpose | Patches |
|-------|---------|---------|
| `android-activity` | Activity integration | ANR-safe callbacks, IME view preference for show/hide, exported `set_nonblocking_callbacks`, IME view preference |
| `rfd` | File dialogs | Android backend routing through `rfd::artcraft_saf`, integrates with SAF bridge |
| `arboard` | Clipboard | Android stub backend using patched egui-winit JNI |
| `winit` | Windowing/input | Android backend patched for mouse/trackpad/wheel/hover, proper motion events, gesture support |
| `egui-winit` | egui/winit integration | Android clipboard backend via JNI ClipboardManager |
| `egui-wgpu` | egui/wgpu renderer | Surface recovery fix (release old surface before creating new) |

**Patching applied during build:** See Section 11.

## 31. CI/CD Pipeline

**File:** `.github/workflows/build.yml`

### 31.1 Triggers
- Push to branches/tags
- Manual dispatch (mode: release/debug, source_version: stable/main)

### 31.2 Configuration
- **Concurrency:** Cancels in-progress runs for same ref
- **Matrix:** Builds all 28 apps in parallel
- **Runner:** Ubuntu (GitHub Actions)
- **Timeout:** 35 minutes per-job limit

### 31.3 Toolchain & Robustness Constraints
- Python 3.12
- Java 21 (linked to /opt/jdk/temurin-21)
- Rust stable with `aarch64-linux-android` target
- cargo-ndk from prebuilt binary
- sccache for Rust compilation caching
- **NDK 27 Preference**: Runners with pre-installed NDK 30 preview break bindgen during clang header parsing (`Unversioned target triples are not supported!`). CI explicitly prefers stable NDK 27 (`27.*`) and exports `BINDGEN_EXTRA_CLANG_ARGS="--target=aarch64-linux-android28"`.
- **Git LFS Smudge Avoidance**: `GIT_LFS_SKIP_SMUDGE=1` is exported globally to prevent clone failures when upstream repos have broken LFS pointers (e.g. NoteCraft's 404 on `Large Desktop.one`).
- **Toolchain Hijack Prevention**: Upstream `rust-toolchain` and `rust-toolchain.toml` files are stripped immediately after checkout so `rustup` does not switch toolchains to versions lacking the Android target.
- **Source Branch Strategy**: Repositories with irregular non-version tags (such as `earthcraft` matching ancient `versions`) or whose Android support is actively authored against upstream `main` (`earthcraft`, `prowave`, `transit`, `rustgis`, `mapcraft`, `anicraft`) automatically build against `main` when `BUILD_SOURCE=stable`.

### 31.4 Caching
- sccache (GHA backend)
- Cargo registry and git
- Gradle caches

### 31.5 Build Process
1. Checkout repository
2. Setup toolchain
3. Run `build.sh` or `build-debug.sh` for each app
4. Verify APK integrity
5. Upload artifacts per app
6. On tag push: create GitHub release with all APKs

**See:** `docs/ci-workflow.md` for detailed architecture.

## 32. Logging and Debugging

### 32.1 Build Logs
- Location: `logs/<app>-<mode>-<timestamp>.log`
- Shell uses `say N "message"` for stage indicators
- Failures go through `fail()` which reports line number
- Trap on ERR reports failing line and command

### 32.2 Runtime Logs
- Rust logs go to logcat via `android_logger`
- Java logs use `android.util.Log`
- Tags: app package name or class name

### 32.3 Markers in .so
For verification:
- `ARTCRAFT_*_ANDROID_SHELL_V*` - Shell version markers
- `ARTCRAFT_ANDROID_SAF_BRIDGE_V3` - SAF bridge marker
- `ARTCRAFT_ANDROID_NATIVE_IME_V1` - Native IME marker

### 32.4 Debugging Tips
```bash
# View logcat
adb logcat -s com.artcraft.soundcraft

# Check installed permissions
adb shell dumpsys package com.artcraft.soundcraft | grep -A 5 "requested permissions"

# Check APK contents
aapt dump badging output/soundcraft-0.12.1.apk | grep package
aapt dump xmltree output/soundcraft-0.12.1.apk AndroidManifest.xml

# Install and test
adb install -r output/soundcraft-0.12.1.apk
```

## 33. Operational Notes and Failure Modes

### 33.1 Common Issues
| Issue | Cause | Solution |
|-------|-------|----------|
| Another build running | Lock file held | Wait or remove `.artcraft-build.lock` if process gone |
| Incomplete checkout | Interrupted clone | Move aside `source/<app>` and rebuild |
| Origin mismatch | Checkout from different repo | Move aside and rebuild |
| Permissions not in manifest | Stale launcher/Cargo.toml | Delete `source/<app>/apps/<app>-android/` and rebuild |
| Permission dialog doesn't appear | Runtime request issue | Check SafActivity.java, ensure requested in safRequestStoragePermission() |
| Generator version mismatch | Template changed | Delete launcher dir or bump GENERATOR_VERSION |
| Build fails with Python syntax error | Bad escaping in templates | Check f-string escaping in lib/build-common.sh |
| ANR on file picker | NonblockingCallbacks not used | Check android-activity vendor patch |
| Keyboard doesn't work | Native IME not polled | Check generated `poll_ime` sets editor info and polls `text_input_state`, and that the launcher was regenerated (GENERATOR_VERSION) |
| Fonts missing | Symlink issues | Must use internal storage for HOME/XDG_DATA_HOME |

### 33.2 Important Notes
- **Single build at a time**: Enforced by flock
- **Incomplete checkout detection**: Checks tracked files exist
- **Disk space**: >=1GB required before cloning
- **Origin safety**: Refuses to reuse unexpected origin
- **JDK version**: Needs 17-23, downloads Temurin 21 if missing
- **SDK/NDK**: Searched in common locations, downloaded if missing
- **Upstream fixes**: Guarded, no-op if already fixed
- **Patches**: Idempotent where possible
- **ANR prevention**: NonblockingCallbacks for blocking SAF picks, async dialogs for some apps
- **Surface loss**: egui-wgpu fix, WordCraft uses async dialogs
- **Font links**: Must be in internal storage (external can't symlink)
- **Intent pending**: Only adapters define it; generated launchers may need handling
- **isHighDensityDevice naming**: Returns true for small screens (width < 600dp), not high DPI
- **Docs say 13 apps** in some places, code supports 14

### 33.3 Testing Permissions
```bash
# Install APK
adb install -r output/<app>-<version>.apk

# Check requested permissions
adb shell dumpsys package com.artcraft.<app> | grep -A 10 "requested permissions"

# Grant permissions manually if needed
adb shell pm grant com.artcraft.<app> android.permission.RECORD_AUDIO
adb shell pm grant com.artcraft.<app> android.permission.READ_EXTERNAL_STORAGE

# For MANAGE_EXTERNAL_STORAGE
adb shell appops set com.artcraft.<app> MANAGE_EXTERNAL_STORAGE allow
```

## 34. Key Files Reference

| File | Purpose |
|------|---------|
| `lib/build-common.sh` | Main orchestrator, launcher generator, SAF recipes, vendoring, patches |
| `lib/android_project.py` | Manifest/Gradle/theme generation from Cargo metadata |
| `lib/file_associations.py` | Intent filter registry (EXT_MIME, ASSOCIATIONS) |
| `lib/app_icons.py` | Adaptive icon generation (mipmaps + monochrome) |
| `lib/scrolling.sh` | Two-finger scroll/momentum patches for all apps |
| `lib/ime.sh` | WordCraft Backspace-at-paragraph-start patch |
| `lib/subcraft.sh` | SubCraft file handoff patch adding `files::deliver_android` |
| `templates/android/SafActivity.java` | Shared Activity base (SAF, IME, permissions, fullscreen, etc.) |
| `templates/android/saf_bridge.rs` | Rust SAF JNI bridge with SaveMirror, NonblockingCallbacks |
| `templates/android/system_fonts.rs` | System font integration with linking |
| `templates/android/UpdateCheck.java` | GitHub update checker |
| `adapters/*/prepare.sh` | Adapter preparation scripts |
| `adapters/*/Cargo.toml` | Adapter launcher configurations |
| `adapters/*/src/lib.rs` | Adapter launcher implementations |
| `vendor/*` | Patched dependencies (android-activity, rfd, arboard, winit, egui-winit, egui-wgpu) |
| `.github/workflows/build.yml` | CI/CD pipeline |
| `docs/ci-workflow.md` | CI documentation |
| `README.md` | User documentation |
| `architecture.md` | Architecture documentation |

## 35. Agent Guidelines

### 35.1 General Principles
- **Read before modifying**: Always understand existing code patterns
- **Preserve upstream**: Don't modify upstream source in `source/`, use patches/adapters
- **Developer Privacy & Relative Paths**: NEVER include absolute local file paths (e.g., `/home/<user>/...`, `C:\Users\<user>\...`, `/Users/<user>/...`, or machine-specific user directory paths) in markdown files, documentation, commit messages, code comments, or PR descriptions. Always use workspace-relative paths (e.g., `lib/build-common.sh`, `agents.md`, `source/<app>/`) to protect developer privacy and prevent accidental exposure of local usernames and filesystem structure.
- **Test on device**: Use connected tablet via ADB for validation
- **Follow patterns**: Match existing code style and architecture
- **Be defensive**: Check for nulls, handle errors gracefully

### 35.2 Common Tasks

**Adding permissions:**
1. Add to `[[package.metadata.android.uses_permission]]` in `lib/build-common.sh` manifest template
2. Request at runtime in `templates/android/SafActivity.java` if needed
3. Regenerate launcher: delete `source/<app>/apps/<app>-android/` or bump GENERATOR_VERSION

**Fixing build issues:**
1. Check logs in `logs/`
2. Delete launcher dir to force regeneration
3. Check vendor patches are applied
4. Verify Python syntax in embedded scripts

**Testing:**
```bash
# Build
bash build.sh -<app>

# Install
adb install -r output/<app>-<version>.apk

# Check permissions
adb shell dumpsys package com.artcraft.<app> | grep -A 15 "requested permissions"

# Monitor logs
adb logcat -s com.artcraft.<app> | tail -50
```

**Regenerating launcher:**
```bash
rm -rf source/<app>/apps/<app>-android/
bash build.sh -<app>
```

### 35.3 PhotoCraft Crash Recovery Pattern
When implementing autosave for other apps, follow the PhotoCraft pattern:
1. Add `photocraft-format` dependency (or equivalent recovery crate)
2. Define `SharedRecovery` with `RecoveryStore`
3. Wire 6 Services callbacks: `autosave`, `autosave_results`, `flush_autosaves`, `discard_autosave`, `recover`, `adopt_autosave`
4. Call `flush_autosaves()` on app pause
5. Set `app.background_jobs = true` for better performance
6. Use zero-copy NIO for file staging

### 35.4 SubCraft Persistence
SubCraft uses external app-private storage via `install_persist_env()`:
- Sets `XDG_CONFIG_HOME`, `XDG_CACHE_HOME` to `files/config`, `files/cache`
- Creates `files/autosave/`, `files/recovery/` directories
- `eframe::App::save()` forwards to `inner.save()`
- Files stored in `/storage/emulated/0/Android/data/com.artcraft.subcraft/files/`

### 35.5 SoundCraft Specifics
- Uses workspace `soundcraft-playback` crate via cpal
- `AUDIO_MODULES` injects `cpal = "0.16"` for SoundCraft
- No copied `audio.rs` module needed
- Requires `RECORD_AUDIO` permission (manifest + runtime)
- Runtime permission requested via `safRequestStoragePermission()` in `SafActivity.java`

### 35.6 Developer Privacy & Path Hygiene
- **Never Include Local Machine Paths**: Do not expose machine-specific usernames, home directory paths, or absolute paths (such as `/home/username/...`, `C:\Users\username\...`, or `/Users/username/...`) in markdown files, documentation, commit messages, PR descriptions, or source code comments.
- **Always Use Relative Paths**: Always use workspace-relative paths (e.g., `lib/build-common.sh`, `adapters/photocraft/src/lib.rs`, `agents.md`).
- **Privacy in Error Logs & Debugging**: When quoting local debug commands, stack traces, or error logs in markdown, documentation, or issue reports, sanitize and scrub personal usernames or local filesystem hierarchies, replacing them with generic relative paths or placeholders.

---

**Document Version:** 56  
**Last Updated:** November 3, 2026  
**Maintained by:** ArtCraft Android Builder Team


## 36. Build Environment & Toolchain Deep Dive

### 36.1 Required Tools
| Tool | Purpose | Discovery/Installation |
|------|---------|------------------------|
| `git` | Source checkout/patching | Must be installed |
| `python3` with `tomllib` | Cargo.toml parsing, template generation | Python 3.11+ required |
| `cargo` | Rust toolchain | Must be installed |
| `rustc` | Rust compiler | Must be installed |
| `cargo-ndk` | Android NDK cargo wrapper | Installed if missing via `cargo install cargo-ndk` |
| `ndk-build`/`cmake` | NDK components | Via SDK/NDK |
| `jdk`/`javac` | Java compilation for Android | OpenJDK 17-23, downloaded from Adoptium if missing |
| `aapt2`/`zipalign`/`apksigner` | APK packaging/signing | From Android SDK build-tools |
| `gradle`/`gradlew` | Android build system | Wrapper copied from templates/gradle/ |
| `adb` | Device testing | For on-device verification |

### 36.2 SDK/NDK/JDK Discovery
**Search paths:**
- Environment: `ANDROID_HOME`, `ANDROID_SDK_ROOT`
- Common locations: `$HOME/Android/Sdk`, `/opt/android-sdk`, `/usr/local/android-sdk`
- NDK: `ndk/` or `ndk-bundle/` under SDK, or latest in `ndk/`
- JDK: `.cache/jdk/temurin-21*`, system Java, or downloaded from Adoptium API

**Bootstrap behavior:**
- If SDK missing, may attempt to download cmdline-tools
- If JDK missing, downloads Temurin JDK to `.cache/jdk/`
- Links JDK to `/opt/jdk/temurin-21` in CI

### 36.3 Cross-compilation Setup
- Target: `aarch64-linux-android` (64-bit ARM only)
- `CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER` set by cargo-ndk
- `NDK_HOME`/`ANDROID_NDK_HOME` configured
- `MIN_SDK_VERSION` written to `.artcraft-gradle.properties` for cargo-ndk
- Rust flags and link args configured by cargo-ndk

### 36.4 Build Directories
```
.artcraft-build.lock          # Global build lock (flock)
.cache/jdk/                   # Downloaded JDKs
keys/                         # Signing keystore
  ├── artcraft-builder.jks
  └── .keystore-password
source/                       # Upstream source checkouts (preserved)
  ├── <app>/                  # Main app source
  └── craft-fonts/            # Shared fonts (optional)
build/                        # Isolated build dirs
  └── <app>/
      └── target/             # Cargo target for this app
output/                       # Built APKs
logs/                         # Build logs with timestamps
android-sdk/                  # SDK if bootstrapped
```

## 37. Cargo Workspace Patching Details

### 37.1 Patch Mechanism
The build system uses Python to perform regex replacements in the root `Cargo.toml` `[patch.crates-io]` section. Patches are applied to `source/$APP/Cargo.toml`.

**Replacement patterns:**
```python
# Example patterns
r'egui-winit\s*=\s*[^\n]+'
# becomes
'egui-winit = { path = "vendor/egui-winit" }'
```

### 37.2 Vendor Directory Structure
```
vendor/
├── android-activity/   # Patched android-activity 0.6
├── arboard/            # Patched arboard
├── egui-wgpu/          # Patched egui-wgpu
├── egui-winit/         # Patched egui-winit
├── rfd/                # Patched rfd
└── winit/              # Patched winit
```

Each vendor crate is copied from `ROOT/vendor/` to `SRC/vendor/` during build. `target/` dirs removed to force recompilation.

### 37.3 Special Cases
- **Apps that already vendor winit**: Only `src/platform_impl/android` is overwritten from our vendor
- **rfd usage detection**: If app uses rfd, it's patched to use Android SAF backend
- **arboard usage**: Patched for Android clipboard

## 38. Android Manifest & Metadata Generation

### 38.1 Cargo Metadata Structure
In `Cargo.toml` `[package.metadata.android]`:
```toml
[package.metadata.android]
package = "com.artcraft.app"
apk_name = "app-1.0.0"
build_targets = ["aarch64-linux-android"]
resources = "res"
java_sources = "java"

[package.metadata.android.sdk]
min_sdk_version = 28
target_sdk_version = 35

[[package.metadata.android.uses_permission]]
name = "android.permission.INTERNET"

[[package.metadata.android.application]]
label = "App Name"
theme = "@style/Theme.Artcraft"
icon = "@mipmap/ic_launcher"

[[package.metadata.android.application.activity]]
name = ".MainActivity"
config_changes = "orientation|keyboard|keyboardHidden|..."
exported = true
launch_mode = "singleTask"
intent_filter = [...]
```

### 38.2 Generated AndroidManifest.xml
The `lib/android_project.py` script converts Cargo metadata to AndroidManifest.xml:
- No package attribute (AGP namespace)
- Permissions from uses_permission entries
- Application with label/theme/icon
- Activity with all attributes
- Intent filters from activity.intent_filter
- meta-data entries
- No uses-sdk (handled by defaultConfig)

### 38.3 File Associations in Manifest
File associations are added via `file_associations.py` which patches:
- `intent_filter` in Cargo.toml metadata
- Generates VIEW/SEND/SEND_MULTIPLE filters with proper MIME types and extensions
- Handles pathPattern escaping for extension-based matching

## 39. SAF Bridge Implementation Details

### 39.1 Java Side (SafActivity)
**Key methods:**
- `safRequestOpen(long id, boolean multiple)` - Launches ACTION_OPEN_DOCUMENT
- `safRequestSave(long id, String suggestedName)` - Launches ACTION_CREATE_DOCUMENT
- `safRequestFolder(long id)` - Launches ACTION_OPEN_DOCUMENT_TREE (API 21+)
- `safPoll()` - Returns queued JSON results as string
- `safCommit(String path)` - Writes staged file to SAF URI
- `intentPoll()` - Returns queued incoming file paths
- `intentPending()` - Returns count of pending intents

**Staging logic:**
```java
// Stage to app-private external dir
File mediaDir = new File(getExternalFilesDir(null), "media");
File stage = new File(mediaDir, uniqueName(displayName));
copyUriToFile(uri, stage);
rememberUri(stage, uri); // SharedPreferences
```

**URI mapping:**
- Key: `uri:` + absolute staged path
- Value: SAF content URI string
- Also tries to find by stem if exact path not found

### 39.2 Rust Side (saf_bridge.rs)
**Bridge API:**
```rust
impl Bridge {
    pub fn new(app: &AndroidApp) -> Self
    pub fn pick_open_blocking(id: u64, multiple: bool, timeout: Duration) -> Result<PickAnswer>
    pub fn pick_save_blocking(id: u64, name: &str, timeout: Duration) -> Result<PickAnswer>
    pub fn pick_folder_blocking(id: u64, timeout: Duration) -> Result<PickAnswer>
    pub fn commit(path: &str) -> Result<()>
    pub fn files_dir() -> Result<PathBuf>
    pub fn intent_pending() -> Result<u32>
    pub fn intent_poll() -> Result<Option<String>>
    pub fn print_document(title: &str, pdf: &[u8]) -> Result<String>
    pub fn request_storage_permission()
    pub fn poll() -> Option<String>
}
```

**SaveMirror:**
- Tracks saves by ID
- Watches staged files for modifications
- On write, calls commit() to mirror to SAF URI
- Periodic polling (~200ms)

**NonblockingCallbacks RAII:**
Prevents ANR by setting `android_activity::set_nonblocking_callbacks(true)` during blocking picker calls.

### 39.3 JSON Protocol
Results passed as JSON strings:
```json
{"id": 123, "kind": "open", "status": "ok", "paths": ["/path1", "/path2"]}
{"id": 124, "kind": "save", "status": "ok", "path": "/path"}
{"id": 125, "kind": "folder", "status": "ok", "path": "/path"}
{"status": "cancel"}
{"status": "error", "message": "..."}
```

## 40. IME Bridge & Soft Keyboard

### 40.1 Java Side (imeSetup)
Creates invisible EditText:
```java
imeView = new EditText(this) {
    @Override
    public InputConnection onCreateInputConnection(EditorInfo outAttrs) {
        outAttrs.inputType = InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_MULTI_LINE;
        outAttrs.imeOptions = EditorInfo.IME_FLAG_NO_FULLSCREEN;
        InputConnection base = super.onCreateInputConnection(outAttrs);
        return new ImeInputConnection(base);
    }
};
imeView.setFocusable(true);
imeView.setFocusableInTouchMode(true);
imeView.setVisibility(View.INVISIBLE);
```

### 40.2 IME Event Queue
Events queued with prefixes:
- `c<text>` - Commit text
- `p<text>` - Pre-edit/composing text
- `f` - Finish composing
- `d<before>,<after>` - Delete surrounding text
- `a<actionCode>` - Editor action

### 40.3 Backspace Pair Guard
Prevents double-deletion:
```java
private static final long IME_DELETE_PAIR_MS = 30L;
private volatile long lastImeDeleteSurroundingAt = 0L;
private volatile long lastImeDeleteKeyAt = 0L;
// If different methods within 30ms, drop second
```

### 40.4 Wake Mechanism
After queuing IME events, sends `KEYCODE_UNKNOWN` key down to wake native event loop:
```java
private void wakeNativeLoop() {
    dispatchKeyEvent(new KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_UNKNOWN));
    dispatchKeyEvent(new KeyEvent(KeyEvent.ACTION_UP, KeyEvent.KEYCODE_UNKNOWN));
}
```

### 40.5 Rust Side (native IME polling)
Native IME polling happens directly in the generated Android launcher's `poll_ime` method (or the app adapter's equivalent). The shell calls `android_activity::input::text_input_state().text` each frame while editing, diffs the result against the previous text, and emits egui `Backspace`/`Text`/`Enter` events. Editor info is set via `set_ime_editor_info` when a text field gains focus, and `show_soft_input(false)`/`hide_soft_input(false)` toggle the keyboard. There is no `ime_bridge.rs` module.

## 41. System Fonts Integration

### 41.1 Font Discovery
Scans:
- `/system/fonts/`
- `/product/fonts/`
- `/storage/emulated/0/Fonts/` (user fonts, requires MANAGE_EXTERNAL_STORAGE)
- `/sdcard/Fonts/`

### 41.2 Font Linking Strategy
Creates symlinks in internal storage (required because external storage doesn't allow symlinks reliably):
```rust
// HOME and XDG_DATA_HOME point to internal dir
std::env::set_var("HOME", &internal_home);
std::env::set_var("XDG_DATA_HOME", &xdg_data);
// Symlink all fonts to $XDG_DATA_HOME/fonts
```

### 41.3 Font Installation
Adds fonts to egui as fallback in all font families:
```rust
pub fn install_system_fonts(ctx: &egui::Context) {
    // Scan and link fonts
    // Add as lowest priority in every family
    // Refresh every few seconds
}
```

### 41.4 PDF Font Support
PdfCraft uses fontconfig with device fonts mapped by desktop-compatible names. System fonts linked to locations fontconfig searches.

## 42. Persistence & Data Storage

### 42.1 Directory Structure
```
/storage/emulated/0/Android/data/<pkg>/files/
├── config/      # App configuration (XDG_CONFIG_HOME)
├── cache/       # App cache (XDG_CACHE_HOME)
├── media/       # Staged SAF documents
├── autosave/    # Autosave files (PhotoCraft, etc.)
└── recovery/    # Crash recovery (PhotoCraft, etc.)
```

### 42.2 Environment Setup
In generated launchers:
```rust
fn install_persist_env(app: &AndroidApp) {
    let files = app.external_data_path();
    std::env::set_var("XDG_CONFIG_HOME", files.join("config"));
    std::env::set_var("XDG_CACHE_HOME", files.join("cache"));
    // HOME/XDG_DATA_HOME for fonts use internal dir
}
```

### 42.3 Autosave Pattern (PhotoCraft Reference)
See `adapters/photocraft/src/lib.rs` for full implementation:
- Uses `photocraft-format::RecoveryStore`
- 6 Services callbacks wired
- `flush_autosaves()` called on pause
- `app.background_jobs = true`
- Zero-copy NIO staging
- Crash recovery on startup

### 42.4 Scratch Storage and Sandbox TMPDIR Setup
On Android, the Rust standard library `std::env::temp_dir()` defaults to `/data/local/tmp` if the `TMPDIR` environment variable is unset. Non-rooted Android applications running in their sandbox lack read/write permissions to `/data/local/tmp`, triggering `Permission denied (os error 13)` whenever an engine allocates temporary files or spills memory to disk (e.g. PhotoCraft raster tile spilling: `"could not create a scratch file in /data/local/tmp: Permission denied (os error 13); spilling is paused"`).

**System-Wide Solution**:
All 18 custom adapters and the generated launcher templates in `lib/build-common.sh` set `TMPDIR` at the very start of `android_main` before initializing any engine services or preferences:
```rust
#[allow(unused_unsafe)]
unsafe {
    if let Some(path) = android_app.internal_data_path().or_else(|| android_app.external_data_path()) {
        let tmp = path.join("tmp");
        let _ = std::fs::create_dir_all(&tmp);
        std::env::set_var("TMPDIR", &tmp);
    }
}
```
This guarantees that `std::env::temp_dir()`, `tempfile::tempfile()`, and PhotoCraft's `(system temp)` scratch disk resolution always point to the application's private, writable sandbox storage.

## 43. Input Handling

### 43.1 Modifier Key Fix
Android/winit doesn't emit ModifiersChanged events. Shell tracks modifier keys:
```rust
fn apply_android_modifiers(state: &mut egui::Modifiers, raw: &mut egui::RawInput) {
    // Track Shift/Ctrl/Alt from Key events
    // Set state.command = state.ctrl
    // Suppress Text events when Ctrl/Command held
    // Emit ModifiersChanged if changed
}
```

### 43.2 Clipboard Shortcuts
Shell synthesizes Copy/Cut/Paste events for Ctrl/Cmd+C/X/V since winit doesn't handle them properly on Android.

### 43.3 Back Button Mapping
BrowserBack key mapped to Escape, then handled by app close logic:
1. If app consumes Escape via input → handled
2. If app has close guard and not asked → request_close() shows app prompt
3. If app agrees to close → finish_activity() called

### 43.4 Touch/Trackpad Gestures
Patched winit/egui-winit provides:
- Two-finger scroll with momentum
- Pinch zoom
- Proper hover/mouse events
- Selection suppression

## 44. Fullscreen, Edge-to-Edge, Theming

### 44.1 Immersive Fullscreen
```java
const IMMERSIVE = 0x0000_0100 | 0x0000_0002 | 0x0000_0004 | 0x0000_0200 | 0x0000_0400 | 0x0000_1000;
// STABLE | HIDE_NAVIGATION | FULLSCREEN | LAYOUT_HIDE_NAVIGATION | LAYOUT_FULLSCREEN | IMMERSIVE_STICKY
```
Re-applied on focus/config changes.

### 44.2 Edge-to-Edge
```java
WindowCompat.setDecorFitsSystemWindows(window, false);
// Or set flags for older APIs
window.setFlags(WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS, ...);
```

### 44.3 Bar Icon Appearance
Synced to egui theme (light/dark):
```java
public void setBarIconAppearance(boolean lightTheme) {
    WindowInsetsController controller = getWindow().getInsetsController();
    if (lightTheme) {
        controller.setSystemBarsAppearance(APPEARANCE_LIGHT_STATUS_BARS | APPEARANCE_LIGHT_NAVIGATION_BARS, 
            APPEARANCE_LIGHT_STATUS_BARS | APPEARANCE_LIGHT_NAVIGATION_BARS);
    } else {
        controller.setSystemBarsAppearance(0, APPEARANCE_LIGHT_STATUS_BARS | APPEARANCE_LIGHT_NAVIGATION_BARS);
    }
}
```

### 44.4 Caption Bar Insets (Desktop Mode)
For Samsung DeX/freeform windows, reserves space for window title bar with app-themed padding.

## 45. Update Checker

### 45.1 Implementation
`templates/android/UpdateCheck.java`:
```java
public class UpdateCheck {
    public static void start(Activity activity) {
        // Baked version: __BUILDER_VERSION__
        // Checks GitHub API after 5s delay
        // Shows notification if newer version available
    }
}
```

### 45.2 API Endpoint
`https://api.github.com/repos/MostafaXS1/artcraft-android/releases/latest`
Compares tag name with baked BUILDER_VERSION.

### 45.3 Behavior
- Runs in background thread after 5s delay
- Silent failure on network errors
- Notifies once per version
- Links to releases page

## 46. Icon Generation Details

### 46.1 Process (app_icons.py)
1. Determine source icon (explicit flag > app icon > largest PNG)
2. Generate legacy mipmaps for all densities (mdpi, hdpi, xhdpi, xxhdpi, xxxhdpi)
3. Generate adaptive icon:
   - Background: solid brand color
   - Foreground: icon artwork inside safe zone (66% of 108dp)
   - Monochrome: same artwork for themed icons (Android 13+)
4. Generate round icons if needed

### 46.2 Densities
| Density | Scale | Size |
|---------|-------|------|
| mdpi | 1.0x | 48x48 |
| hdpi | 1.5x | 72x72 |
| xhdpi | 2.0x | 96x96 |
| xxhdpi | 3.0x | 144x144 |
| xxxhdpi | 4.0x | 192x192 |

Adaptive icons: 108x108dp with 66dp safe zone.

## 47. Printing Support

### 47.1 Flow
1. Rust calls `bridge.print_document(title, pdf_bytes)`
2. Java creates temp PDF in cache dir
3. Uses PrintManager with PdfPrintAdapter
4. System print dialog shown
5. Returns "OK"

### 47.2 PdfPrintAdapter
Custom PrintDocumentAdapter:
- `onLayout()` - Reports page count
- `onWrite()` - Streams PDF to spooler

Supported apps: WordCraft, GridCraft

## 48. Debugging & Troubleshooting

### 48.1 Logcat
```bash
# All logs for app
adb logcat -s com.artcraft.soundcraft

# With timestamps
adb logcat -v time -s com.artcraft.soundcraft

# Clear and watch
adb logcat -c && adb logcat -s com.artcraft.soundcraft
```

### 48.2 APK Inspection
```bash
# Badging
aapt dump badging output/app.apk

# Manifest
aapt dump xmltree output/app.apk AndroidManifest.xml

# Permissions
aapt dump permissions output/app.apk

# Native libs
unzip -l output/app.apk | grep "\.so"
```

### 48.3 Common Debug Commands
```bash
# Check installed version
adb shell dumpsys package com.artcraft.app | grep versionName

# Grant all permissions
adb shell pm grant com.artcraft.app android.permission.RECORD_AUDIO
adb shell pm grant com.artcraft.app android.permission.READ_EXTERNAL_STORAGE

# MANAGE_EXTERNAL_STORAGE
adb shell appops set com.artcraft.app MANAGE_EXTERNAL_STORAGE allow

# Check storage
adb shell ls -la /storage/emulated/0/Android/data/com.artcraft.app/files/

# Force stop
adb shell am force-stop com.artcraft.app
```

### 48.4 Build Debugging
```bash
# Verbose build
bash build.sh -app 2>&1 | tee debug.log

# Check logs
tail -100 logs/app-release-*.log

# Regenerate everything
rm -rf source/app/apps/app-android build/app .artcraft-build.lock
bash build.sh -app
```

## 49. Performance Considerations

### 49.1 Build Performance
- sccache enabled in CI
- Isolated target dirs per app reduce cache pollution
- Cargo incremental compilation where possible
- Vendored deps cached

### 49.2 Runtime Performance
- WGPU rendering
- Two threads for SAF I/O
- NonblockingCallbacks prevent ANR
- Background jobs enabled for apps with autosave
- Zero-copy NIO where possible

### 49.3 Memory
- Staged files in app-private storage
- Old sessions dropped on UI thread
- Plugin instances garbage collected
- Periodic cleanup of orphaned staged files

## 50. Security & Permissions

### 50.1 Permissions
**Normal permissions:**
- INTERNET

**Dangerous permissions:**
- READ_EXTERNAL_STORAGE (maxSdkVersion 32)
- WRITE_EXTERNAL_STORAGE (maxSdkVersion 29)
- RECORD_AUDIO (runtime)
- MODIFY_AUDIO_SETTINGS

**Special permissions:**
- MANAGE_EXTERNAL_STORAGE (Android 11+, requires settings screen)

### 50.2 SAF Security
- All file access goes through SAF or app-private storage
- URI permissions persisted
- Staged copies isolated in app-private dir
- No direct file system access to external storage without permissions

### 50.3 Signing
- Uses local keystore for development
- Release builds signed with artcraft-builder.jks
- Keystore password stored in keys/.keystore-password
- Do not distribute with development keystore

## 51. Version History & Changelog Notes

### 51.1 Generator Version
`GENERATOR_VERSION = "55"` in lib/build-common.sh
Bump when launcher template changes significantly.

### 51.2 Builder Version
`BUILDER_VERSION = "0.4.1"` (current)
Shown in UpdateCheck, included in APK metadata.

### 51.3 Key Milestones
- v0.2.0: Previous version
- v0.4.0: Current version with expanded agents.md, SoundCraft audio routing fixes

## 52. Contributing Guidelines

### 52.1 Code Style
- Follow existing Rust/Java/Bash patterns
- Use 4 spaces for indentation in most files
- Keep functions focused and documented
- Add comments for non-obvious logic

### 52.2 Testing Requirements
- Test builds for affected apps
- Verify on connected Android device when possible
- Check permissions in manifest and runtime
- Ensure APK installs and launches

### 52.3 Patch Guidelines
- Make patches idempotent where possible
- Guard with checks for already-applied fixes
- Document in code comments
- Add to appropriate patch script

### 52.4 Commit Messages
Use clear, descriptive commit messages. Follow project style. Never include local system paths, home directories, or personal usernames in commit messages, code comments, or documentation; always use workspace-relative paths.

## 53. Appendix

### 53.1 Environment Variables
| Variable | Purpose |
|----------|---------|
| `ANDROID_HOME`/`ANDROID_SDK_ROOT` | SDK location |
| `ANDROID_NDK_HOME`/`NDK_HOME` | NDK location |
| `JAVA_HOME` | JDK location |
| `ARTCRAFT_KEYSTORE` | Keystore path |
| `ARTCRAFT_KEYSTORE_PASSWORD` | Keystore password |
| `ARTCRAFT_KEY_ALIAS` | Key alias |
| `ARTCRAFT_KEY_PASSWORD` | Key password |
| `CARGO_TARGET_DIR` | Cargo build output dir |
| `RUST_BACKTRACE` | Enable backtraces |

### 53.2 Useful Commands Reference
```bash
# Build specific app
bash build.sh -<app>

# Debug build
bash build-debug.sh -<app>

# Clean
bash clean.sh

# List devices
adb devices

# Install
adb install -r output/<app>-*.apk

# Logcat
adb logcat -s com.artcraft.<app>

# Uninstall
adb uninstall com.artcraft.<app>
```

### 53.3 File Patterns
- App names: lowercase (photocraft, soundcraft, etc.)
- Package names: com.artcraft.<app>
- Activity names: <AppName>Activity or .<AppName>Activity
- APK names: <app>-<version>.apk

### 53.4 Quick Reference Links
- [GitHub Repo](https://github.com/MostafaXS1/artcraft-android)
- [Releases](https://github.com/MostafaXS1/artcraft-android/releases)
- [ArtCraft Website](https://getartcraft.com/)
- [Storytold GitHub](https://github.com/storytold)

## 54. Model Context Protocol (MCP) Suite Architecture

ArtCraft Android applications expose an embedded, zero-configuration Model Context Protocol (MCP) server over HTTP and Server-Sent Events (SSE) bound to loopback (`127.0.0.1`) and LAN interfaces. This allows local or remote AI agents to automate document creation, layer manipulation, parametric CAD modeling, BIM scene construction, audio editing, and video generation directly on device.

### 54.1 Dual Operating Modes

The suite employs two distinct MCP operational architectures depending on whether an application is an interactive visual canvas or a batch automation engine:

1. **Live UI Control Architecture (Interactive Apps)**:
   - **Apps**: PhotoCraft, CADCraft, BIMCraft, SoundCraft, SolidCraft, DiagramCraft, PaintCraft, NoteCraft, VectorCraft, PicassoSuite, AniCraft, EarthCraft, MapCraft, ProWave, RustGIS, TraceDraw, Transit.
   - **Mechanism**: Tool execution cannot run on a detached headless engine session because the user expects to see layers, paths, walls, windows, and waveforms updated on the active screen.
   - **Dispatch Pipeline**:
     - The MCP HTTP server receives JSON-RPC 2.0 `tools/call`.
     - In PhotoCraft: Requests route over a local TCP loopback bridge to the app's internal command listener, which applies commands to the active document and requests an egui repaint.
     - In CADCraft, BIMCraft, SoundCraft, etc.: The MCP server packages the request into a `ControlRequest` and enqueues it into `CONTROL_TX` or `CONTROL_QUEUE`.
     - On each frame of the `egui` render loop: `with_control` drains pending requests, executes the tool closure against `&mut self.inner`, and sends the JSON result across a oneshot channel back to the MCP HTTP thread.
     - `ctx.request_repaint()` is triggered immediately, refreshing the GPU canvas.

2. **Headless Engine Mode (Batch / Render Engines)**:
   - **Apps**: FilmCraft, MaterialCraft, TextureCraft, PDFCraft.
   - **Mechanism**: Uses the official `rmcp` SDK or engine automation handlers with an isolated headless session (`Headless::default_session()`, `Filesystem::TrustedLocal`, `FilmcraftMcp::headless()`).
   - Requests execute asynchronously without requiring an active window or eframe rendering context.

### 54.2 Protocol & HTTP SSE Endpoint Specification
- **Endpoints**:
  - `POST /`: JSON-RPC 2.0 tool execution (`tools/list`, `tools/call`, `resources/list`, `initialize`).
  - `GET /sse`: Server-Sent Events stream for asynchronous progress and notifications.
  - `POST /message`: Message transport companion for SSE sessions.
  - `GET /status`: Diagnostic endpoint reporting server state, package name, version, and tool count.
- **Service Advertising**:
  On startup, the app logs:
  `MCP HTTP service started on http://127.0.0.1:27880/ (LAN: http://<IP>:27880/)`
  allowing discovery via logcat or adb port forwarding.

## 55. Stylus & Digitizer Subsystem

ArtCraft Android provides hardware-accelerated, high-fidelity active and passive stylus input across all 28 applications, matching desktop drawing tablet precision and latency.

### 55.1 Unbuffered Touch & Digitizer Sampling
- **Latency Elimination**: Standard Android view pipelines buffer and batch touch events to match display VSYNC (16ms or 8ms frame boundaries), introducing perceptible stroke drag during rapid drawing or sketching.
- **Java Layer Dispatch**: In `SafActivity.java`, `__CLASS__.java`, and custom adapter activities (`adapters/*/java/com/artcraft/*/*.java`), unbuffered dispatch is explicitly enabled during window attachment:
  ```java
  public void setupStylusDispatch() {
      View decor = getWindow().getDecorView();
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
          decor.requestUnbufferedDispatch(
              MotionEvent.TOOL_TYPE_STYLUS | MotionEvent.TOOL_TYPE_ERASER
          );
      }
  }
  ```
- This forces the Linux kernel and `InputReader` to bypass intermediate window manager batching and deliver raw digitizer coordinates directly to the native `android_main` event loop as soon as digitizer hardware reports them.

### 55.2 Samsung S-Pen & Vendor Action Code Normalization
- **Proprietary Action Codes**: Samsung S-Pen devices frequently emit vendor-specific action codes (e.g. `211` for pen down, `212` for pen up, `213` for pen hover move, `214` for pen move with button depressed) that standard Android NDK `AMOTION_EVENT_ACTION` enums do not map, leading standard GameActivity/winit apps to drop stylus strokes.
- **Winit Translation**: In `vendor/winit/src/platform_impl/android/mod.rs`, incoming motion events are intercepted and normalized:
  ```rust
  let normalized_action = match raw_action {
      211 => AMOTION_EVENT_ACTION_DOWN,
      212 => AMOTION_EVENT_ACTION_UP,
      213 | 214 => AMOTION_EVENT_ACTION_MOVE,
      other => other,
  };
  ```

### 55.3 Barrel Button & Secondary Action Mapping
- **Stylus Button Right-Click**: When the barrel button on an active pen (Apple Pencil double-tap on supported bridges, Samsung S-Pen button, or universal USI stylus side button) is pressed (`BUTTON_STYLUS_PRIMARY = 0x20` or `BUTTON_SECONDARY = 0x40`), `winit` synthesizes a right-click mouse input event (`MouseButton::Right`):
  ```rust
  let is_barrel_pressed = (button_state & (AMOTION_EVENT_BUTTON_STYLUS_PRIMARY | AMOTION_EVENT_BUTTON_SECONDARY)) != 0;
  if is_barrel_pressed {
      // Synthesize right-click down for tool context menus, eraser switching, or canvas panning
  }
  ```
- This gives mobile and tablet users instant access to contextual menus, secondary brush tools, and pan/zoom toggles without needing to hunt for desktop-style menu bars.

### 55.4 Palm Rejection & Tool Identity
- **Active Palm Suppression**: Resting a palm on the tablet screen while sketching with a stylus commonly generates stray touch events. The vendored winit implementation maintains a sliding timestamp (`LAST_STYLUS_MS`). When an active stylus event is processed:
  1. Any finger touch (`TOOL_TYPE_FINGER`) occurring within 500ms of active stylus input is discarded at the NDK event queue level.
  2. Stylus touch IDs are marked with bit 32 set (`(pointer_id as u64) | (1 << 32)`), ensuring downstream UI frameworks immediately identify the pointer as a precision instrument rather than a capacitive finger contact.

---

## 56. Gesture Engine & Touch Rejection

### 56.1 Multi-Finger Tap Gesture Synthesis (Undo / Redo)
To match modern touch-first digital art workflows, ArtCraft apps support multi-finger tap gestures for instantaneous undo and redo:
- **Two-Finger Tap -> Undo (`Ctrl + Z`)**:
  - Detected when exactly two fingers contact the screen, remain stationary within a small tolerance (24 logical pixels), and release within 350 milliseconds without initiating a pinch or scroll.
  - Synthesizes `VirtualKeyCode::LControl` + `VirtualKeyCode::Z` key press and release events directly into the window event stream.
- **Three-Finger Tap -> Redo (`Ctrl + Shift + Z`)**:
  - Detected when exactly three fingers tap simultaneously within 350 milliseconds.
  - Synthesizes `VirtualKeyCode::LControl` + `VirtualKeyCode::LShift` + `VirtualKeyCode::Z`.
- **Global App Coverage**: Because this gesture recognizer operates inside the vendored `winit` platform implementation, it functions consistently across all 28 applications without requiring per-app canvas modifications.

### 56.2 Pinch-to-Zoom Brush Dab Rejection
- **The Problem**: In desktop egui apps running on touchscreen Android devices, a multi-touch pinch-to-zoom gesture typically begins with one finger touching the screen slightly before the second finger (e.g. 10–30ms difference). Standard event dispatchers immediately fire a `PointerButton::Primary` down event for the first touch, causing painting and drawing apps (PhotoCraft, VectorCraft, DesignCraft, PaintCraft, etc.) to paint unwanted dots, dabs, or brush strokes right where the user began zooming.
- **The Solution (`PendingPointerDown`)**:
  - Implemented in `vendor/egui-winit/src/lib.rs` and `vendor/egui-winit-0.36/src/lib.rs`.
  - Precision stylus input (`TOOL_TYPE_STYLUS` / bit 32 set) dispatches immediately with zero delay.
  - Capacitive finger touches (`TOOL_TYPE_FINGER`) enter a pending state:
    ```rust
    struct PendingPointerDown {
        touch_id: u64,
        location: winit::dpi::PhysicalPosition<f64>,
    }
    ```
  - **Movement Threshold**: The pointer down event is held until the finger moves beyond `POINTER_DOWN_SLOP_PX` (20 physical pixels). The press is then anchored at the touch-down point, so a drag still starts on the widget the finger went down on.
  - **Cancellation on Multi-Touch**: If a second finger touches the screen, or if a pinch/zoom gesture is detected, the pending pointer down event is aborted cleanly. The canvas never receives a button-down event, preventing accidental brush strokes during zoom and pan operations.
  - **Atomic UI Click Synthesis**: If the finger is lifted without having moved (a regular single-finger tap on a UI button, checkbox, or text field), both pointer-down and pointer-up events are emitted atomically on release, ensuring crisp, responsive UI interaction.

### 56.3 Hold-to-Right-Click vs. Left-Click
- **The Problem**: egui turns a pointer press plus its release into a *click* whenever the press is younger than `InputOptions::max_click_duration` (0.8 s by default). The hold-to-right-click fired after 500 ms, so its left-button release was still inside that window: the long press activated the widget underneath the finger **and** opened the context menu. A long press could therefore paint a dot, press a ribbon button, or start a drag as well.
- **The Solution**: a finger's primary press is **deferred** until the gesture is known to be something else, so a hold never has a left press to cancel:
  - The finger **moves** past the slop → a drag: the press is sent, anchored at the touch-down point, and the hold is dropped.
  - The finger **lifts** → a tap: press and release are emitted atomically (see 56.2).
  - The finger **stays still** for `HoldToRightClick::HOLD_DURATION_MS` (500 ms) → a right-click: the pending press is dropped and only the secondary press/release is sent, so nothing under the finger is activated.
- A stylus still presses right away, so its hold only fires once that press is older than egui's click window (`hold_may_fire`), keeping the release from registering as a click there too.

---

## 57. Mobile Text Selection Context Bar

Desktop text editing widgets in `egui::TextEdit` assume the presence of a hardware keyboard or a right-click mouse for clipboard interactions. On Android, users require an immediate floating context bar above or below focused text selections.

### 57.1 Module Architecture (`templates/android/mobile_text_bar.rs`)
- **Inspection**: On each frame of the application UI loop, `mobile_text_bar::show_text_bar(ctx)` queries `ctx.memory(|m| m.focus())` to determine if a text editing widget holds keyboard focus.
- **Selection State**: Retrieves `egui::text_edit::CCursorRange` via `TextEdit::load_state(ctx, focused_id)`.
- **Dynamic Action Visibility**:
  - **Select all**: Selects the full range `[0, text.len()]`.
  - **Cut**: Copies selection to system clipboard via `ctx.copy_text(...)` and removes selected characters (only visible when a selection range is active).
  - **Copy**: Copies selection to system clipboard (only visible when text is selected).
  - **Paste**: Reads clipboard text from `ctx.input(|i| i.clipboard_text())` and inserts at the cursor position.
  - **Delete**: Clears selected characters immediately (dynamically hidden when no text is selected; appears only when text is actively highlighted).
- **Native UI Styling**: The context bar is rendered via `egui::Area` at `Order::Tooltip` using the app's native `egui::Visuals` (matching active window frames, corner rounding, drop shadows, and typography).
- **Focus Preservation**: Button clicks within the context bar explicitly restore and preserve widget focus on the parent `TextEdit`, preventing Android's virtual keyboard from dismissing unexpectedly.

---

## 58. Integrated Topbar Modifier Controls

Desktop creative tools rely heavily on keyboard modifier keys (`Shift` for aspect-ratio locking and multi-selection, `Ctrl` for duplication and grid snapping, `Alt` for eyedropper sampling and alternate tool modes).

### 58.1 Native Topbar Integration vs. Screen Overlays
Rather than employing floating button overlays that obscure canvas artwork, ArtCraft integrates toggleable modifier buttons directly into the primary application topbar or ribbon header next to native menus:
- **Target Applications**:
  1. `photocraft`: Top menu bar (`crates/ui-egui/src/menus/bar.rs`)
  2. `vectorcraft`: Top menu bar (`crates/ui-egui/src/menus/bar.rs`)
  3. `designcraft`: Top menu bar (`crates/ui-egui/src/menus/bar.rs`)
  4. `wordcraft`: Ribbon header bar (`crates/ui-egui/src/ribbon.rs`)
  5. `gridcraft`: Ribbon header bar (`crates/ui-egui/src/ribbon.rs`)
  6. `deckcraft`: Ribbon header bar (`crates/ui-egui/src/ribbon.rs`)

### 58.2 Shared Implementation (`templates/android/topbar_modifiers.rs`)
- **Visual Design**: Renders compact, high-contrast toggle buttons with standard Unicode modifier glyphs:
  - `⇧ Shift`
  - `⌃ Ctrl`
  - `⌥ Alt`
- **Toggle State Persistence**: Modifiers can be latched on or off with a tap. When active, buttons display an accented highlight fill.
- **Event Injection (`apply_to_raw_input`)**: Prior to passing `egui::RawInput` to the application UI framework, active modifier flags are OR-ed into `raw_input.modifiers`, allowing native tool logic, canvas shortcuts, and transform handles to respond exactly as if physical keys were held down.
- **Automated Build Integration (`lib/topbar_modifiers.sh`)**: Automatically invoked during launcher generation and adapter preparation to safely insert modifier UI calls into the upstream topbar render hierarchy.

---

## 59. Platform Hardening & Build Reliability

### 59.1 Upstream Git LFS Smudge Bypass
- **The Issue**: Certain upstream repositories (notably NoteCraft) introduce binary test fixtures tracked under Git LFS whose remote storage pointers may become unavailable (returning HTTP 404 during clone), aborting CI checkout pipelines with fatal git exit code 128.
- **The Mitigation**: `export GIT_LFS_SKIP_SMUDGE=1` is enforced in `lib/build-common.sh` and `.github/workflows/build.yml`. This allows the repository to clone cleanly without failing on orphaned third-party binary fixtures while preserving all application source and build assets.

### 59.2 Non-Rooted Scratch Disk & Tile Cache Configuration
- **The Issue**: Desktop image engines (such as GEGL or custom tile managers) default scratch file allocation to `/data/local/tmp` on POSIX systems. On standard Android devices without root privileges, application sandboxes are strictly prohibited from writing to `/data/local/tmp`, triggering:
  `could not create a scratch file in /data/local/tmp: Permission denied (os error 13); spilling is paused (tiles stay in memory)`
- **The Mitigation**: The Android build harness configures `TMPDIR`, `TEMP`, and `GEGL_SWAP_DIR` environment variables to point exclusively to the application's private internal cache directory (`context.getCacheDir().getAbsolutePath()`, e.g. `/data/data/<package>/cache`), ensuring compliant, high-speed tile spilling without permission violations.

### 59.3 AniCraft Brand Identity Refresh
- **Upstream Asset Sync**: AniCraft upstream introduced an updated official engraved rabbit mascot icon across high-resolution tile formats.
- **Asset Pipeline**: Master vector and raster assets (`adapters/anicraft/assets/app-icon/`) are processed via `lib/app_icons.py` to generate full-bleed adaptive vector layers (`res/mipmap-*/ic_launcher.png`, `ic_launcher_foreground.xml`) and legacy mipmaps, ensuring consistent presentation across modern Android launcher grids.


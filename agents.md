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
subcraft
```
Total: 14 apps. CI matrix builds all of them.

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
│   ├── ime.sh              # Soft keyboard return patches (WordCraft)
│   └── subcraft.sh         # SubCraft file handoff patch
├── templates/
│   ├── android/            # Shared Java/Rust modules
│   │   ├── SafActivity.java
│   │   ├── saf_bridge.rs
│   │   ├── ime_bridge.rs
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
# SubCraft exception
APP_ORIGIN="https://github.com/Aalzard/subcraft"
REPO="https://github.com/Aalzard/subcraft.git"
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
- Patches for soft keyboard return after back-dismiss
- Installs IME handling in Activity/shared bridge

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
- `templates/android/ime_bridge.rs` → `src/ime_bridge.rs`

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
- **Lifecycle**: Close guard for unsaved changes, IME hide on scroll (WordCraft)
- **Persistence/UI scale**: Load prefs, apply device scale on first phone launch, save periodically

### 10.4 SAF Apps Configuration
```python
SAF_APPS = {"designcraft", "deckcraft", "gridcraft", "wordcraft", "lightcraft", 
           "effectcraft", "filmcraft", "cadcraft", "soundcraft", "pdfcraft", 
           "figcraft", "subcraft"}
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
| `egui-winit` | `vendor/egui-winit` | Android clipboard via JNI, IME integration |
| `rfd` (if used) | `vendor/rfd` | Android stub/backend routing through `rfd::artcraft_saf` |
| `arboard` (if used) | `vendor/arboard` | Android stub backend |
| `winit` | `vendor/winit` | Android backend patched for mouse/trackpad/wheel/hover, momentum scrolling |
| `egui-wgpu` | `vendor/egui-wgpu` | Surface recovery fix (release old surface before creating new) |
| `android-activity` | `vendor/android-activity` | ANR-safe callbacks, IME view preference for show/hide, exported `set_nonblocking_callbacks` |

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
- Creates invisible focusable `EditText` (`imeView`)
- Custom `InputConnectionWrapper` queues events:
  - `"c"` + text - commit
  - `"p"` + text - composing/pre-edit
  - `"f"` - finish composing
  - `"d"<before>,<after>` - delete surrounding
  - `"a"<actionCode>` - editor action
- `imeShow()` / `imeHide()` control keyboard
- `imePoll()` returns queued events
- Backspace pair guard (30ms) prevents double-delete from IME
- Wakes native loop via `onKeyDown(KEYCODE_UNKNOWN)` for immediate processing

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
Tracks staged save paths by ID, watches for writes, calls `commit()` to mirror to SAF URI. Scans periodically (~200ms) to detect writes.

**JSON Protocol:** Results passed as JSON strings over JNI queue.

### 13.3 `templates/android/ime_bridge.rs`
**Purpose:** IME event bridge

```rust
pub const IME_MARKER: &str = "ARTCRAFT_ANDROID_IME_BRIDGE_V1";

pub struct ImeBridge { ... }

impl ImeBridge {
    pub fn new(app: &AndroidApp) -> Self
    pub fn poll(&self, raw: &mut egui::RawInput) // Converts Java events to egui IME events
    pub fn hide_ime(&self) -> Result<()>
}
```

Converts queued events from Java:
- `"c<text>"` → `egui::Event::Text(text)`
- `"p<text>"` → Composition event
- `"f"` → Finish composition
- `"d<before>,<after>"` → Delete events
- `"a<action>"` → Editor action

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
           "figcraft", "subcraft"}
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
- Invisible focusable `EditText` in Activity
- Custom `InputConnectionWrapper` intercepts IME events
- Events queued and polled by Rust
- Converted to egui IME events

### 16.2 Event Flow
1. User types on soft keyboard
2. IME calls InputConnection methods
3. `SafActivity` queues events with prefixes ("c", "p", "f", "d", "a")
4. `queueImeEvent()` also wakes native loop via `onKeyDown(KEYCODE_UNKNOWN)`
5. Rust calls `ime_poll()` each frame via `ime_bridge`
6. Converted to egui events and passed to `raw_input_hook`

### 16.3 Backspace Handling
- Two methods can report same backspace: `deleteSurroundingText(1,0)` and `sendKeyEvent(KEYCODE_DEL)`
- Arrive within microseconds (same dispatch)
- `claimDelete()` checks if other method fired within 30ms
- If pair detected, second call dropped
- Double-tap still works (different timestamps)

### 16.4 Wake Mechanism
- Native loop sleeps in `ALooper_pollOnce` until events
- Queued keyboard events don't wake loop automatically
- `KEYCODE_UNKNOWN` key events go through GameActivity's native queue
- Winit reports as `Key::Unidentified`, no widget acts on it
- Forces egui to process IME queue

### 16.5 Configuration
```java
window.setSoftInputMode(
    WindowManager.LayoutParams.SOFT_INPUT_ADJUST_NOTHING |
    WindowManager.LayoutParams.SOFT_INPUT_STATE_UNCHANGED
);
imeView.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_MULTI_LINE);
imeView.setImeOptions(EditorInfo.IME_FLAG_NO_FULLSCREEN);
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
prepare.sh copies adapter into `source/$APP/apps/$APP-android/`, removes prepare.sh/README.md, copies shared modules (system_fonts.rs, ime_bridge.rs).

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
- **Matrix:** Builds all 14 apps in parallel
- **Runner:** Ubuntu (GitHub Actions)
- **Timeout:** Reasonable per-job limits

### 31.3 Toolchain
- Python 3.12
- Java 21 (linked to /opt/jdk/temurin-21)
- Rust stable with `aarch64-linux-android` target
- cargo-ndk from prebuilt binary
- sccache for Rust compilation caching

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
- `ARTCRAFT_ANDROID_IME_BRIDGE_V1` - IME bridge marker

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
| Keyboard doesn't work | IME bridge missing | Check ime_bridge.rs copied, imeShow/imeHide methods exist |
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
| `lib/ime.sh` | Soft keyboard return patches (WordCraft) |
| `lib/subcraft.sh` | SubCraft file handoff patch adding `files::deliver_android` |
| `templates/android/SafActivity.java` | Shared Activity base (SAF, IME, permissions, fullscreen, etc.) |
| `templates/android/saf_bridge.rs` | Rust SAF JNI bridge with SaveMirror, NonblockingCallbacks |
| `templates/android/ime_bridge.rs` | IME event bridge to egui |
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

---

**Document Version:** 55  
**Last Updated:** October 25, 2026  
**Maintained by:** ArtCraft Android Builder Team

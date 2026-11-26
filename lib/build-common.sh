#!/usr/bin/env bash
# Core orchestrator for ArtCraft Android builds.
# Reduced from 8,322 lines to a concise modular driver (issue #73).
set -Eeuo pipefail

MODE="${1:-release}"
shift || true
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
export GIT_LFS_SKIP_SMUDGE=1
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"
cd "$ROOT"

usage() {
    cat <<'USAGE'
Usage:
  bash build.sh -pdfcraft [options]
  bash build-debug.sh -photocraft [options]

Required:
  -<app>              App to build (leading hyphen)

Options:
  -stable             Build against latest stable release tag (default)
  -main, -latest      Build against upstream main branch
  -tag=<tag>          Build against a specific git tag
  -branch=<branch>    Build against a specific git branch
  -appname=<name>     Override the display name (default: app's built-in title)
  -packagename=<pkg>  Override the Android package (default: com.artcraft.<app>)
  -appicon=<path>     Use a custom launcher icon PNG (default: app's own icon)
  --no-buttons        Build without virtual Shift/Ctrl/Alt and Cut/Copy/Paste/Delete buttons
  --noupdate          Skip checking for and downloading newer source
  -h, --help          Show this help

App names:
  photocraft vectorcraft filmcraft lightcraft pdfcraft effectcraft
  designcraft figcraft wordcraft soundcraft deckcraft cadcraft gridcraft
  subcraft solidcraft materialcraft texturecraft
  notecraft diagramcraft paintcraft bimcraft tracedraw picassosuite capsulecraft humanmotioncraft
  rustgis earthcraft mapcraft prowave transit anicraft coimanide pcbcraft fieldcraft papercraft ferrite pixelcraft clothcraft

Examples:
  bash build.sh -photocraft
  bash build.sh -photocraft -appname=notPhotoshop
  bash build.sh -photocraft -packagename=com.adobe.photoshow
  bash build.sh -photocraft -appicon=/path/to/icon.png
  bash build.sh -photocraft -appname=MyApp -packagename=com.example.myapp -appicon=icon.png

Files created by the builder stay under this directory:
  source/<app>/     checked-out source plus generated Android launcher
  build/<app>/      Cargo build output
  output/           final APKs
  logs/             build logs
  keys/             persistent local signing key (keep it safe)
USAGE
}

if [[ "$MODE" != "release" && "$MODE" != "debug" ]]; then
    echo "Internal error: build mode must be release or debug." >&2
    exit 2
fi

# Parse optional arguments
APPNAME=""
PACKAGENAME=""
APPICON=""
NO_BUTTONS="${NO_BUTTONS:-false}"
BUILD_SOURCE="${BUILD_SOURCE:-stable}"
UPDATE_SOURCE="${UPDATE_SOURCE:-true}"
RAW_APP=""
while [[ "$#" -gt 0 ]]; do
    case "$1" in
        --no-buttons|-no-buttons)
            NO_BUTTONS="true"
            export ARTCRAFT_NO_BUTTONS="1"
            shift
            ;;
        --noupdate|-noupdate)
            UPDATE_SOURCE="false"
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        -stable)
            BUILD_SOURCE="stable"
            shift
            ;;
        -main|-latest)
            BUILD_SOURCE="main"
            shift
            ;;
        -tag=*)
            BUILD_SOURCE="${1#-tag=}"
            shift
            ;;
        -branch=*)
            BUILD_SOURCE="${1#-branch=}"
            shift
            ;;
        -source=*)
            BUILD_SOURCE="${1#-source=}"
            shift
            ;;
        -appname=*)
            APPNAME="${1#-appname=}"
            shift
            ;;
        -packagename=*)
            PACKAGENAME="${1#-packagename=}"
            shift
            ;;
        -appicon=*)
            APPICON="${1#-appicon=}"
            shift
            ;;
        -*)
            if [[ -n "$RAW_APP" ]]; then
                echo "Multiple app targets specified: $RAW_APP and $1" >&2
                usage
                exit 2
            fi
            RAW_APP="$1"
            shift
            ;;
        *)
            echo "Unexpected argument: $1" >&2
            usage
            exit 2
            ;;
    esac
done

if [[ -z "$RAW_APP" ]]; then
    echo "Missing required app argument." >&2
    usage
    exit 2
fi
[[ "$RAW_APP" == -* ]] || { echo "Use a leading hyphen, for example: bash build.sh -pdfcraft" >&2; usage; exit 2; }
APP="${RAW_APP#-}"
[[ "$APP" != -* && "$APP" =~ ^[a-z]+$ ]] || { echo "Invalid app name: $RAW_APP" >&2; usage; exit 2; }

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
BUILDER_VERSION="0.7.1"
UPDATE_API_URL="https://api.github.com/repos/MostafaXS1/artcraft-android/releases/latest"
UPDATE_PAGE_URL="https://github.com/MostafaXS1/artcraft-android/releases/latest"

mkdir -p "$ROOT/source" "$ROOT/build" "$ROOT/output" "$ROOT/logs" "$ROOT/keys" "$ROOT/adapters"

# Name the failing line if a command fails unexpectedly
trap 'rc=$?; echo >&2; echo "ERROR: the build stopped at line $LINENO: $BASH_COMMAND (exit $rc)" >&2; echo "No source checkout or existing project files were deleted." >&2; exit $rc' ERR

say() {
    local step="$1"; shift
    printf '\n[%s/8] %s\n' "$step" "$*"
}

fail() {
    echo >&2
    echo "ERROR: $*" >&2
    echo "No source checkout or existing project files were deleted." >&2
    exit 1
}

# Single build lock: prevent concurrent builds that replace each other's launcher crate
BUILD_LOCK="$ROOT/.artcraft-build.lock"
if command -v flock >/dev/null 2>&1; then
    exec {BUILD_LOCK_FD}>>"$BUILD_LOCK"
    if ! flock -n "$BUILD_LOCK_FD"; then
        BUILD_LOCK_HOLDER="$(cat "$BUILD_LOCK" 2>/dev/null || true)"
        fail "another build is already running${BUILD_LOCK_HOLDER:+ (pid $BUILD_LOCK_HOLDER)}. Run builds one after another: two at once replace each other's launcher crate and both can fail. If that process is gone, just run this again. ($BUILD_LOCK)"
    fi
    : >"$BUILD_LOCK"
    printf '%s\n' "$$" >&"$BUILD_LOCK_FD"
fi

command -v cargo >/dev/null 2>&1 || fail "Rust/Cargo is required."
command -v python3 >/dev/null 2>&1 || fail "Python 3.11+ is required (tomllib is used to inspect manifests)."
python3 -c 'import tomllib' >/dev/null 2>&1 || fail "Python 3.11+ with tomllib is required."

# Load modular helper libraries
source "$ROOT/lib/catalog.sh"
source "$ROOT/lib/toolchain.sh"
source "$ROOT/lib/source.sh"
source "$ROOT/lib/vendoring.sh"
source "$ROOT/lib/signing.sh"
source "$ROOT/lib/packaging.sh"

# 1. Resolve app catalog metadata
resolve_app_catalog "$APP"
if [[ -n "$APPNAME" ]]; then TITLE="$APPNAME"; fi

# 2. Toolchain and SDK/NDK bootstrap
init_toolchain

# 3. Source tree fetch and checkout
checkout_app_source

# 4. Source repairs and modifications
python3 "$ROOT/lib/upstream_patches.py" "$ROOT/$SRC" "$APP" "$TITLE"
bash "$ROOT/lib/scrolling.sh" "$ROOT/$SRC" "$APP" "$TITLE"
bash "$ROOT/lib/subcraft.sh" "$ROOT/$SRC" "$APP" "$TITLE"
bash "$ROOT/lib/topbar_modifiers.sh" "$ROOT/$SRC" "$APP" "$TITLE" "$NO_BUTTONS"
bash "$ROOT/lib/about_dialog.sh" "$ROOT/$SRC" "$APP" "$TITLE"
bash "$ROOT/lib/fullscreen_toggle.sh" --patch-src "$ROOT/$SRC" "$APP" "$TITLE"

# 5. Generate or prepare NativeActivity launcher
ANDROID_DIR="$ROOT/$SRC/apps/$APP-android"
say 4 "Preparing the NativeActivity launcher..."
if [[ -x "$ROOT/adapters/$APP/prepare.sh" ]]; then
    echo "Using app-specific adapter: adapters/$APP/prepare.sh"
    bash "$ROOT/adapters/$APP/prepare.sh" "$APP" "$TITLE" "$SRC" "$MODE"
    if [[ -d "$ROOT/$SRC/apps/$APP-android/src" ]]; then
        text_bar_src="$ROOT/templates/android/mobile_text_bar.rs"
        if [[ "$NO_BUTTONS" == "true" ]]; then
            text_bar_src="$ROOT/templates/android/mobile_text_bar_disabled.rs"
        fi
        [[ -f "$text_bar_src" ]] && cp -f "$text_bar_src" "$ROOT/$SRC/apps/$APP-android/src/mobile_text_bar.rs"
        [[ -f "$ROOT/templates/android/touch_gestures.rs" ]] && cp -f "$ROOT/templates/android/touch_gestures.rs" "$ROOT/$SRC/apps/$APP-android/src/touch_gestures.rs"
        if [[ "$APP" == "subcraft" || "$APP" == "tracedraw" || "$APP" == "picassosuite" || "$APP" == "bimcraft" || "$APP" == "earthcraft" || "$APP" == "mapcraft" || "$APP" == "transit" || "$APP" == "papercraft" || "$APP" == "ferrite" || "$APP" == "pixelcraft" || "$APP" == "clothcraft" ]]; then
            sed -i '/source: egui::MouseWheelSource::/d' "$ROOT/$SRC/apps/$APP-android/src/touch_gestures.rs" 2>/dev/null || true
        fi
    fi
else
    python3 "$ROOT/lib/launcher_generator.py" "$ROOT" "$ROOT/$SRC" "$APP" "$TITLE" "$TARGET_SDK" "$APPNAME" "$PACKAGENAME" "$APPICON" "$NO_BUTTONS"
fi

# Install UpdateChecker and ArtCraftIme
UPDATER_DIR="$ANDROID_DIR/java/com/artcraft/updater"
if [[ -d "$ANDROID_DIR/java" ]]; then
    mkdir -p "$UPDATER_DIR"
    python3 "$ROOT/lib/cargo_patches.py" install-updater "$ROOT/templates/android/UpdateCheck.java" "$UPDATER_DIR/UpdateCheck.java" \
        "$BUILDER_VERSION" "$UPDATE_API_URL" "$UPDATE_PAGE_URL"
    echo "  -> installed the update checker (builder version $BUILDER_VERSION)"
fi

IME_DIR="$ANDROID_DIR/java/com/artcraft/ime"
if [[ -d "$ANDROID_DIR/java" ]]; then
    mkdir -p "$IME_DIR"
    cp "$ROOT/templates/android/ArtCraftIme.java" "$IME_DIR/ArtCraftIme.java"
fi

[[ -f "$ANDROID_DIR/Cargo.toml" && -f "$ANDROID_DIR/src/lib.rs" ]] || fail "Android wrapper is incomplete at $ANDROID_DIR. See the build log and adapters/README.md."
bash "$ROOT/lib/ime.sh" "$SRC" "$APP" "$TITLE"
bash "$ROOT/lib/fullscreen_toggle.sh" --wire-launcher "$ROOT/$SRC" "$APP" "$ANDROID_DIR"

# 6. Vendor dependencies and apply workspace patches
vendor_dependencies

# 7. Signing key preparation
prepare_signing_key

# 8. Build preparation
say 6 "Building $TITLE ($MODE)..."
mkdir -p "build/$APP/target" output logs
export CARGO_TARGET_DIR="$ROOT/build/$APP/target"
STAMP="$(date +%Y%m%d-%H%M%S)"
LOG="logs/$APP-$MODE-$STAMP.log"
cd "$ANDROID_DIR"

APP_VERSION="$(python3 "$ROOT/lib/cargo_patches.py" resolve-version "$ROOT/$SRC" "$APP")"

# Embed assets (craft-fonts, craft-i18n, models, contributors)
embed_craft_assets

# Compile native library with cargo-ndk
build_native_library

# Package and sign APK with Gradle, verify file bridge
package_and_sign_apk

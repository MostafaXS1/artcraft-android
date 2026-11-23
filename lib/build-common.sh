#!/usr/bin/env bash
set -Eeuo pipefail

MODE="${1:-}"
shift || true
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
export GIT_LFS_SKIP_SMUDGE=1
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
  rustgis earthcraft mapcraft prowave transit anicraft coimanide pcbcraft fieldcraft papercraft ferrite pixelcraft

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

case "$APP" in
    photocraft) TITLE="PhotoCraft" ;;
    vectorcraft) TITLE="VectorCraft" ;;
    filmcraft) TITLE="FilmCraft" ;;
    lightcraft) TITLE="LightCraft" ;;
    pdfcraft) TITLE="PdfCraft" ;;
    effectcraft) TITLE="EffectCraft" ;;
    designcraft) TITLE="DesignCraft" ;;
    figcraft) TITLE="FigCraft" ;;
    wordcraft) TITLE="WordCraft" ;;
    soundcraft) TITLE="SoundCraft" ;;
    deckcraft) TITLE="DeckCraft" ;;
    cadcraft) TITLE="CADCraft" ;;
    gridcraft) TITLE="GridCraft" ;;
    subcraft) TITLE="SubCraft" ;;
    solidcraft) TITLE="SolidCraft" ;;
    materialcraft) TITLE="MaterialCraft" ;;
    texturecraft) TITLE="TextureCraft" ;;
    notecraft) TITLE="NoteCraft" ;;
    diagramcraft) TITLE="DiagramCraft" ;;
    paintcraft) TITLE="PaintCraft" ;;
    bimcraft) TITLE="BIMCraft" ;;
    tracedraw) TITLE="TraceDraw" ;;
    picassosuite) TITLE="PicassoSuite" ;;
    capsulecraft) TITLE="CapsuleCraft" ;;
    humanmotioncraft) TITLE="HumanMotionCraft" ;;
    rustgis) TITLE="RustGIS" ;;
    earthcraft) TITLE="EarthCraft" ;;
    mapcraft) TITLE="MapCraft" ;;
    prowave) TITLE="ProWave" ;;
    transit) TITLE="Transit" ;;
    anicraft) TITLE="AniCraft" ;;
    coimanide) TITLE="Coiman IDE" ;;
    pcbcraft) TITLE="PCBCraft" ;;
    fieldcraft) TITLE="FieldCraft" ;;
    papercraft) TITLE="PaperCraft" ;;
    ferrite) TITLE="Ferrite" ;;
    pixelcraft) TITLE="PixelCraft" ;;
    *) echo "Unsupported app: $APP" >&2; usage; exit 2 ;;
esac
if [[ -n "$APPNAME" ]]; then TITLE="$APPNAME"; fi

SRC="source/$APP"
REPO="https://github.com/storytold/$APP.git"
# SubCraft, SolidCraft and the newly added apps are community apps that live at different origins.
APP_ORIGIN="https://github.com/storytold/$APP"
if [[ "$APP" == "subcraft" ]]; then
    REPO="https://github.com/Aalzard/subcraft.git"
    APP_ORIGIN="https://github.com/Aalzard/subcraft"
elif [[ "$APP" == "solidcraft" ]]; then
    REPO="https://github.com/tangency-labs/solidcraft.git"
    APP_ORIGIN="https://github.com/tangency-labs/solidcraft"
elif [[ "$APP" == "notecraft" ]]; then
    REPO="https://github.com/AlienTechSoftware/notecraft.git"
    APP_ORIGIN="https://github.com/AlienTechSoftware/notecraft"
elif [[ "$APP" == "diagramcraft" ]]; then
    REPO="https://github.com/baradhili/DiagramCraft.git"
    APP_ORIGIN="https://github.com/baradhili/DiagramCraft"
elif [[ "$APP" == "paintcraft" ]]; then
    REPO="https://github.com/replicraft/PaintCraft.git"
    APP_ORIGIN="https://github.com/replicraft/PaintCraft"
elif [[ "$APP" == "bimcraft" ]]; then
    REPO="https://github.com/IPedrax/BIMCraft.git"
    APP_ORIGIN="https://github.com/IPedrax/BIMCraft"
elif [[ "$APP" == "tracedraw" ]]; then
    REPO="https://github.com/tracedraw/tracedraw.git"
    APP_ORIGIN="https://github.com/tracedraw/tracedraw"
elif [[ "$APP" == "picassosuite" ]]; then
    REPO="https://github.com/fabox-os/Picasso-Suite.git"
    APP_ORIGIN="https://github.com/fabox-os/Picasso-Suite"
elif [[ "$APP" == "capsulecraft" ]]; then
    REPO="https://github.com/kostyk348/CapsuleCraft.git"
    APP_ORIGIN="https://github.com/kostyk348/CapsuleCraft"
elif [[ "$APP" == "humanmotioncraft" ]]; then
    REPO="https://github.com/YalKouassi/HumanMotionCraft.git"
    APP_ORIGIN="https://github.com/YalKouassi/HumanMotionCraft"
elif [[ "$APP" == "rustgis" ]]; then
    REPO="https://github.com/opengeos/RustGIS.git"
    APP_ORIGIN="https://github.com/opengeos/RustGIS"
elif [[ "$APP" == "earthcraft" ]]; then
    REPO="https://github.com/therichronicles/EarthCraft.git"
    APP_ORIGIN="https://github.com/therichronicles/EarthCraft"
elif [[ "$APP" == "mapcraft" ]]; then
    REPO="https://github.com/bearlikelion/MapCraft.git"
    APP_ORIGIN="https://github.com/bearlikelion/MapCraft"
elif [[ "$APP" == "prowave" ]]; then
    REPO="https://github.com/fabox-os/Pro-Wave.git"
    APP_ORIGIN="https://github.com/fabox-os/Pro-Wave"
elif [[ "$APP" == "transit" ]]; then
    REPO="https://github.com/transit-audio/transit.git"
    APP_ORIGIN="https://github.com/transit-audio/transit"
elif [[ "$APP" == "anicraft" ]]; then
    REPO="https://github.com/storytold/AniCraft.git"
    APP_ORIGIN="https://github.com/storytold/AniCraft"
elif [[ "$APP" == "coimanide" ]]; then
    REPO="https://github.com/ManuelCoimbra/coiman-ide.git"
    APP_ORIGIN="https://github.com/ManuelCoimbra/coiman-ide"
elif [[ "$APP" == "pcbcraft" ]]; then
    REPO="https://github.com/tangency-labs/pcbcraft.git"
    APP_ORIGIN="https://github.com/tangency-labs/pcbcraft"
elif [[ "$APP" == "fieldcraft" ]]; then
    REPO="https://github.com/tangency-labs/fieldcraft.git"
    APP_ORIGIN="https://github.com/tangency-labs/fieldcraft"
elif [[ "$APP" == "papercraft" ]]; then
    REPO="https://github.com/slyim/papercraft.git"
    APP_ORIGIN="https://github.com/slyim/papercraft"
elif [[ "$APP" == "ferrite" ]]; then
    REPO="https://github.com/OlaProeis/Ferrite.git"
    APP_ORIGIN="https://github.com/OlaProeis/Ferrite"
elif [[ "$APP" == "pixelcraft" ]]; then
    REPO="https://github.com/asad-albadi/PixelCraft.git"
    APP_ORIGIN="https://github.com/asad-albadi/PixelCraft"
fi

# The builder's own version, baked into every APK. The app compares it with the latest GitHub
# release of this repository on launch and offers the download when GitHub has a newer one
# (see templates/android/UpdateCheck.java). Bump it when a release is published.
BUILDER_VERSION="0.7.0"
UPDATE_API_URL="https://api.github.com/repos/MostafaXS1/artcraft-android/releases/latest"
UPDATE_PAGE_URL="https://github.com/MostafaXS1/artcraft-android/releases/latest"

# Absolute: the build later `cd`s into the launcher directory, and cargo-ndk/Gradle paths
# derived from $ANDROID_DIR must keep resolving from there.
ANDROID_DIR="$ROOT/$SRC/apps/$APP-android"
mkdir -p source build output logs keys adapters

say() { printf '\n[%s] %s\n' "$1" "$2"; }

# A command that fails on its own (not one the script handles with `|| true`, `if`, or a `case`)
# would otherwise stop the build without a word, leaving only the last progress line in the log.
# Name the failing line instead, so the log always says what went wrong.
trap 'rc=$?; echo >&2; echo "ERROR: the build stopped at line $LINENO: $BASH_COMMAND (exit $rc)" >&2; echo "No source checkout or existing project files were deleted." >&2; exit $rc' ERR

fail() {
    echo >&2
    echo "ERROR: $*" >&2
    echo "No source checkout or existing project files were deleted." >&2
    exit 1
}

# Only one build may run at a time, and the runs are not independent. `prepare.sh` replaces the
# launcher crate under source/<app>/apps/<app>-android, and this script later builds from that
# very directory (`cd "$ANDROID_DIR"`), which is where cargo-ndk's rustc and Gradle's javac
# inherit their working directories. A second build that wipes it mid-compile kills the first one
# with "Error occurred during initialization of VM ... Could not determine current working
# directory", which points nowhere near the real cause. Refuse up front instead, naming the
# process that holds the lock.
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

say 1 "Checking build tools..."
command -v git >/dev/null 2>&1 || fail "git is required."
command -v cargo >/dev/null 2>&1 || fail "Rust/Cargo is required."
command -v python3 >/dev/null 2>&1 || fail "Python 3.11+ is required (tomllib is used to inspect manifests)."
python3 -c 'import tomllib' >/dev/null 2>&1 || fail "Python 3.11+ with tomllib is required."

# Apps whose launcher (adapter or builder-generated) ships a Java Storage Access Framework Activity.
# Kept here in addition to the manifest check below so bridge verification covers the very first
# build too, before the launcher has been generated. Keep in sync with SAF_RECIPES in this file.
SAF_APPS=" designcraft deckcraft gridcraft wordcraft lightcraft effectcraft filmcraft cadcraft soundcraft pdfcraft figcraft subcraft solidcraft materialcraft texturecraft notecraft diagramcraft paintcraft bimcraft tracedraw picassosuite capsulecraft humanmotioncraft rustgis earthcraft mapcraft prowave transit anicraft coimanide pcbcraft fieldcraft papercraft ferrite pixelcraft "
needs_saf() {
    [[ " $SAF_APPS " == *" $1 "* ]]
}

# An adapter (or generated launcher) that ships Java sources (the Storage Access Framework file
# pickers) needs its Activity compiled into the APK, which the Gradle Android Plugin does below.
# `needs_apk2` still marks those apps for bridge verification after the build.
needs_apk2() {
    needs_saf "$1" && return 0
    local manifest="$ROOT/adapters/$1/Cargo.toml"
    [[ -f "$manifest" ]] && grep -q '^[[:space:]]*java_sources' "$manifest"
}

# The build compiles the native library with cargo-ndk (a jniLibs tree) and packages that tree,
# the Java Activity and the generated manifest with Gradle (see below). cargo-apk/cargo-apk2 are
# no longer invoked.
if ! cargo ndk --help >/dev/null 2>&1; then
    echo "cargo-ndk is needed to build $TITLE's native Android library. Installing it..."
    cargo install cargo-ndk --locked || fail "Could not install cargo-ndk. Install it manually with: cargo install cargo-ndk --locked"
fi
cargo ndk -V >/dev/null 2>&1 || fail "cargo-ndk is required but not working. Install it with: cargo install cargo-ndk --locked"
export PERL5LIB="$ROOT/lib/perl5:${PERL5LIB:-}"

# Gradle 8.11.1 with AGP 8.7.3 (see lib/android_project.py) cannot run on the newest JDKs —
# Java 25 dies in the Groovy build scripts with "Unsupported class file major version 69" —
# while keytool above is happiest on one. Use a detected JDK 17-23 for Gradle, or fetch a
# Temurin 21 into the builder's cache once when there is none.
find_gradle_jdk() {
    local dir major
    for dir in /usr/lib/jvm/* /usr/lib*/jvm/* /opt/java/* /opt/jdk/* "$HOME"/.sdkman/candidates/java/*; do
        [[ -x "$dir/bin/java" && -x "$dir/bin/javac" ]] || continue
        major="$("$dir/bin/java" -version 2>&1 | head -n1 | sed -nE 's/.*version "([0-9]+).*/\1/p')"
        [[ "$major" =~ ^[0-9]+$ ]] || continue
        if ((major >= 17 && major <= 23)); then
            printf '%s\n' "$dir"
            return 0
        fi
    done
    return 1
}

bootstrap_gradle_jdk() {
    local cache="$ROOT/.cache/jdk" dir="$ROOT/.cache/jdk/temurin-21" arch extracted tmp
    if [[ -x "$dir/bin/java" && -x "$dir/bin/javac" ]]; then
        printf '%s\n' "$dir"
        return 0
    fi
    case "$(uname -m)" in
        x86_64) arch="x64" ;;
        aarch64 | arm64) arch="aarch64" ;;
        *) fail "No Temurin JDK 21 build is known for architecture $(uname -m). Install a JDK 17-23 manually." ;;
    esac
    mkdir -p "$cache"
    tmp="$(mktemp -d "$cache/jdk-download.XXXXXX")"
    local jdk_url="https://api.adoptium.net/v3/binary/latest/21/ga/linux/$arch/jdk/hotspot/normal/eclipse"
    curl -fL --retry 3 -o "$tmp/jdk.tar.gz" "$jdk_url" \
        || { rm -rf "$tmp"; fail "Could not download the Temurin JDK 21 Gradle needs. Install a JDK 17-23 manually."; }
    local expected_sha256="${JDK_SHA256:-}"
    if [[ -z "$expected_sha256" ]]; then
        local loc
        loc="$(curl -fsSL --retry 3 "$jdk_url" -I 2>/dev/null | grep -i '^location:' | head -n1 | tr -d '\r' | awk '{print $2}')"
        if [[ -n "$loc" && "$loc" == http* ]]; then
            expected_sha256="$(curl -fsSL --retry 3 "${loc}.sha256.txt" 2>/dev/null | awk '{print $1}')"
        fi
    fi
    if [[ -n "$expected_sha256" ]]; then
        echo "$expected_sha256  $tmp/jdk.tar.gz" | sha256sum -c - \
            || { rm -rf "$tmp"; fail "Cryptographic SHA-256 checksum verification failed for downloaded JDK."; }
    fi
    tar -xzf "$tmp/jdk.tar.gz" -C "$tmp" || { rm -rf "$tmp"; fail "The downloaded JDK archive could not be extracted."; }
    extracted="$(find "$tmp" -maxdepth 1 -mindepth 1 -type d -name 'jdk-*' | head -n1)"
    [[ -n "$extracted" && -x "$extracted/bin/javac" ]] || { rm -rf "$tmp"; fail "The downloaded JDK archive is missing bin/javac."; }
    rm -rf "$dir"
    mv "$extracted" "$dir"
    rm -rf "$tmp"
    printf '%s\n' "$dir"
}

# Prove the packaged APK really carries the Android file bridge the adapter declares. The
# generated manifest mirrors the adapter's Cargo metadata one to one, so an activity that
# omits its MAIN/LAUNCHER intent-filter would still build into an APK with no launcher icon
# (unstartable from the home screen); a green build alone would not catch that.
verify_android_bridge() {
    local app="$1" apk="$2"
    needs_apk2 "$app" || return 0
    local manifest="$ROOT/adapters/$app/Cargo.toml"
    [[ -f "$manifest" ]] || manifest="$ROOT/source/$app/apps/$app-android/Cargo.toml"
    [[ -f "$manifest" ]] || { echo "WARNING: no manifest found for '$app'; skipping file-bridge verification." >&2; return 0; }
    local pkg activity launcher
    read -r pkg activity launcher < <(python3 - "$manifest" <<'PY'
import sys, tomllib
from pathlib import Path
data = tomllib.loads(Path(sys.argv[1]).read_text())
android = data["package"]["metadata"]["android"]
acts = android.get("application", {}).get("activity", [])
if isinstance(acts, dict):
    acts = [acts]
name, launches = "", False
for a in acts:
    name = a.get("name") or "android.app.NativeActivity"
    for f in a.get("intent_filter", []):
        if "android.intent.action.MAIN" in f.get("actions", []):
            launches = True
    if name:
        break
print(android["package"], name.lstrip("."), "1" if launches else "0")
PY
)
    [[ -n "$pkg" && -n "$activity" ]] || { echo "WARNING: could not read the manifest for '$app'; skipping file-bridge verification." >&2; return 0; }
    if [[ "$launcher" != "1" ]]; then
        fail "$manifest declares activity '$activity' without a MAIN intent-filter, so the built APK would have no launcher icon. Add [[package.metadata.android.application.activity.intent_filter]]."
    fi
    if command -v aapt2 >/dev/null 2>&1; then
        local badging
        badging="$(aapt2 dump badging "$apk" 2>/dev/null || true)"
        if ! grep -q "launchable-activity: name='$pkg.$activity'" <<<"$badging"; then
            fail "The built APK has no launcher activity '$pkg.$activity' (its manifest is missing the MAIN/LAUNCHER intent-filter), so it could not be started from the home screen."
        fi
    fi
    if ! python3 - "$apk" "$activity" <<'PY'
import sys, zipfile
apk, activity = sys.argv[1], sys.argv[2]
needle = activity.encode()
with zipfile.ZipFile(apk) as z:
    names = z.namelist()
    in_dex = any(n.endswith(".dex") and needle in z.read(n) for n in names)
    libs = [n for n in names if n.startswith("lib/") and n.endswith(".so")]
sys.exit(0 if (in_dex and libs) else 1)
PY
    then
        fail "The built APK is missing the compiled Activity '$activity' or its native library, so the Java SAF file picker is not in the APK."
    fi
    echo "Verified the Android file bridge: $pkg.$activity launches the app and is compiled into the APK."
}

# ---- The Android SDK -------------------------------------------------------------------------
# The builder keeps its own SDK in `android-sdk/`, beside the other build state, and fetches it
# when the machine has none: the command-line tools (which carry `sdkmanager`), the compile
# platform, the build tools, and the NDK. An SDK that is already on the machine is used where it
# lies, and an NDK that is already installed is never fetched twice — it is a gigabyte unpacked.
#
# The NDK is not tied to an SDK root (it is named through `ANDROID_NDK_HOME`), so the search for
# one reaches every SDK on the machine, not only the chosen root.

# The SDK root to use: the first candidate that holds the compile platform and some build tools.
# The builder's own `android-sdk/` is where a missing one is fetched to, and the machine's own
# SDKs are searched as well, so a machine that already has one does not download a second.
find_android_sdk() {
    local dir
    for dir in "${ARTCRAFT_SDK_DIR:-}" "${ANDROID_HOME:-}" "${ANDROID_SDK_ROOT:-}" \
               "$ROOT/android-sdk" "$HOME"/Android*/Sdk /opt/android-sdk; do
        [[ -n "$dir" && -d "$dir/platforms/android-$TARGET_SDK" && -d "$dir/build-tools" ]] || continue
        printf '%s\n' "$dir"
        return 0
    done
    printf '%s\n' "$ROOT/android-sdk"
}

# An NDK on the machine, wherever it lies.
find_android_ndk() {
    local dir
    for dir in "${ANDROID_NDK_HOME:-}" "${ANDROID_NDK_ROOT:-}" "${NDK_HOME:-}" \
               "$ROOT/android-sdk/ndk"/* /opt/android-sdk/ndk/* "$HOME"/Android*/Sdk/ndk/*; do
        [[ -n "$dir" && -d "$dir/toolchains/llvm/prebuilt" ]] || continue
        printf '%s\n' "$dir"
        return 0
    done
    return 1
}

# The newest command-line tools package for Linux, from Google's own repository index. The
# archive carries `sdkmanager`, which installs the platform, the build tools and the NDK.
android_cmdline_tools_url() {
    python3 - <<'PY'
import re
import urllib.request

best = 0
for index in ("repository2-3.xml", "repository2-1.xml"):
    try:
        with urllib.request.urlopen(
            f"https://dl.google.com/android/repository/{index}", timeout=60
        ) as response:
            data = response.read().decode("utf-8", "replace")
    except Exception:
        continue
    for number in re.findall(r"commandlinetools-linux-(\d+)_latest\.zip", data):
        best = max(best, int(number))
if best:
    print(f"https://dl.google.com/android/repository/commandlinetools-linux-{best}_latest.zip")
PY
}

# Fetch what the SDK is missing: the command-line tools, the compile platform, the build tools,
# and — when the machine has no NDK at all — the NDK as well.
ensure_android_sdk() {
    local sdk="$1" ndk_package="${2:-}" java_home tmp url
    mkdir -p "$sdk"
    if [[ ! -x "$sdk/cmdline-tools/latest/bin/sdkmanager" ]]; then
        say 2 "  -> fetching the Android command-line tools into $sdk"
        tmp="$(mktemp -d "$sdk/.cmdline-tools.XXXXXX")"
        url="$(android_cmdline_tools_url)"
        [[ -n "$url" ]] || { rm -rf "$tmp"; fail "Could not find the Android command-line tools. Set ANDROID_HOME to an SDK with them."; }
        curl -fL --retry 3 -o "$tmp/tools.zip" "$url" \
            || { rm -rf "$tmp"; fail "Could not download the Android command-line tools from $url."; }
        python3 - "$tmp/tools.zip" "$tmp" <<'PY' || { rm -rf "$tmp"; fail "The downloaded command-line tools archive could not be extracted."; }
import sys
import zipfile

with zipfile.ZipFile(sys.argv[1]) as archive:
    archive.extractall(sys.argv[2])
PY
        [[ -d "$tmp/cmdline-tools" ]] || { rm -rf "$tmp"; fail "The downloaded command-line tools archive is missing the cmdline-tools folder."; }
        mkdir -p "$sdk/cmdline-tools"
        rm -rf "$sdk/cmdline-tools/latest"
        mv "$tmp/cmdline-tools" "$sdk/cmdline-tools/latest"
        rm -rf "$tmp"
        # The archive is unpacked with python, which keeps no permissions: `sdkmanager` and its
        # companions have to be made executable again.
        chmod +x "$sdk/cmdline-tools/latest/bin/"* 2>/dev/null || true
    fi
    # `sdkmanager` is a Java program, so it needs a JDK: the one Gradle uses, or a fetched one.
    java_home="$(find_gradle_jdk || bootstrap_gradle_jdk)"
    local sdkmanager="$sdk/cmdline-tools/latest/bin/sdkmanager"
    # The licences are asked for once: without them the packages are refused.
    yes | JAVA_HOME="$java_home" "$sdkmanager" --sdk_root="$sdk" --licenses >/dev/null 2>&1 || true
    say 2 "  -> installing the Android platform, the build tools${ndk_package:+ and the NDK}"
    JAVA_HOME="$java_home" "$sdkmanager" --sdk_root="$sdk" \
        "platform-tools" "platforms;android-$TARGET_SDK" "build-tools;$BUILD_TOOLS_VERSION" $ndk_package \
        || fail "Could not install the Android SDK packages into $sdk."
}

say 2 "Finding Android SDK and NDK (the builder's own SDK lives in $ROOT/android-sdk)..."
TARGET_SDK="${ARTCRAFT_TARGET_SDK:-35}"
BUILD_TOOLS_VERSION="${ARTCRAFT_BUILD_TOOLS_VERSION:-37.0.0}"
NDK_VERSION="${ARTCRAFT_NDK_VERSION:-27.2.12479018}"
SDK="$(find_android_sdk)"
NDK="$(find_android_ndk || true)"
if [[ ! -d "$SDK/platforms/android-$TARGET_SDK" || ! -d "$SDK/build-tools" || -z "$NDK" ]]; then
    # Something is missing. The builder's own folder is where it is fetched to, so a machine
    # without an SDK ends up with one that needs no setup at all.
    SDK="$ROOT/android-sdk"
    NDK_PACKAGE=""
    [[ -n "$NDK" ]] || NDK_PACKAGE="ndk;$NDK_VERSION"
    ensure_android_sdk "$SDK" "$NDK_PACKAGE"
    [[ -n "$NDK" ]] || NDK="$SDK/ndk/$NDK_VERSION"
fi
[[ -d "$SDK/platforms/android-$TARGET_SDK" ]] || fail "Android SDK platform android-$TARGET_SDK not found under $SDK/platforms. Install it with sdkmanager or set ARTCRAFT_TARGET_SDK to an installed platform."
[[ -d "$SDK/build-tools" ]] || fail "Android build-tools directory missing under $SDK."
BUILD_TOOLS="${ARTCRAFT_BUILD_TOOLS_VERSION:-$(find "$SDK/build-tools" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' 2>/dev/null | sort -V | tail -n 1)}"
[[ -n "$BUILD_TOOLS" && -d "$SDK/build-tools/$BUILD_TOOLS" ]] || fail "No usable Android Build Tools installation found."
[[ -n "$NDK" && -d "$NDK" ]] || fail "No Android NDK found. Install one using Android SDK Manager."
export ANDROID_HOME="$SDK" ANDROID_SDK_ROOT="$SDK"
export ANDROID_NDK_HOME="$NDK" ANDROID_NDK_ROOT="$NDK" NDK_HOME="$NDK"
NDK_TOOLCHAIN="$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin"
[[ -d "$NDK_TOOLCHAIN" ]] || NDK_TOOLCHAIN="$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin"
export PATH="$NDK_TOOLCHAIN:$SDK/platform-tools:$SDK/build-tools/$BUILD_TOOLS:$PATH"
echo "SDK: $SDK"
echo "Platform: android-$TARGET_SDK"
echo "Build Tools: $BUILD_TOOLS"
echo "NDK: $NDK"
if command -v rustup >/dev/null 2>&1 && ! rustup target list --installed | grep -qx 'aarch64-linux-android'; then
    fail "Rust target is missing. Install it once with: rustup target add aarch64-linux-android"
fi

# Compilation profile optimizations: fast builds, low memory consumption on 2-vCPU / 4GB RAM machines,
# and stripping leftover debug symbols to keep APK sizes minimal without slow link-time optimization (ThinLTO).
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS="${CARGO_PROFILE_RELEASE_CODEGEN_UNITS:-16}"
export CARGO_PROFILE_RELEASE_LTO="${CARGO_PROFILE_RELEASE_LTO:-off}"
export CARGO_PROFILE_RELEASE_OPT_LEVEL="${CARGO_PROFILE_RELEASE_OPT_LEVEL:-2}"
export CARGO_PROFILE_RELEASE_STRIP="${CARGO_PROFILE_RELEASE_STRIP:-true}"
export CARGO_PROFILE_RELEASE_DEBUG="${CARGO_PROFILE_RELEASE_DEBUG:-false}"

# Avoid downloading large LFS assets that may be missing upstream (e.g. NoteCraft's 404 Large Desktop.one).
export GIT_LFS_SKIP_SMUDGE=1
git config --global core.quotePath false 2>/dev/null || true
# Ensure bindgen and clang use versioned target triples on NDK 27+ / 30+.
export BINDGEN_EXTRA_CLANG_ARGS="--target=aarch64-linux-android28 ${BINDGEN_EXTRA_CLANG_ARGS:-}"

# Repositories whose releases either lack tags, have irregular non-version tags (e.g. earthcraft 'versions'),
# or whose Android integrations require upstream main:
case "$APP" in
    earthcraft|prowave|transit|rustgis|mapcraft|anicraft|solidcraft|pcbcraft|ferrite)
        if [[ "$BUILD_SOURCE" == "stable" ]]; then
            BUILD_SOURCE="main"
        fi
        ;;
    fieldcraft)
        if [[ "$BUILD_SOURCE" == "stable" ]]; then
            BUILD_SOURCE="v0.8.1"
        fi
        ;;
    papercraft)
        if [[ "$BUILD_SOURCE" == "stable" ]]; then
            BUILD_SOURCE="v0.4.0"
        fi
        ;;
    pixelcraft)
        if [[ "$BUILD_SOURCE" == "stable" ]]; then
            BUILD_SOURCE="v0.11.0"
        fi
        ;;
esac

say 3 "Fetching official source for $TITLE..."

# A checkout is only usable when every tracked file of the workspace is actually on disk.
# `git clone` can be interrupted (a killed build, a full disk) after it has written the commit
# and the origin but before it has written the whole tree. Git then reports a perfectly normal
# origin and commit while the working tree is missing most of the crates, and reusing such a
# checkout fails much later with a misleading "this app needs an app-specific adapter" message.
# The tree is therefore verified here, where the real cause is still obvious.
#
# Only `crates/` and `apps/` are checked: those are the directories the workspace is made of,
# and the builder never removes files from them (it only replaces `vendor/`, which Git tracks
# but the build overwrites anyway).
source_tree_is_complete() {
    local dir="$1" missing
    [[ -d "$dir/.git" ]] || return 1
    git -C "$dir" rev-parse --verify HEAD >/dev/null 2>&1 || return 1
    missing="$(git -c core.quotePath=false -C "$dir" ls-tree -r --name-only HEAD -- crates apps 2>/dev/null | while IFS= read -r f; do
        [[ -e "$dir/$f" ]] || printf '%s\n' "$f"
    done | head -5)"
    [[ -z "$missing" ]]
}

# Query the latest published release tag for a repository.
# Prioritizes GitHub's published release redirect (/releases/latest), which ensures we only pick
# official, published releases — avoiding stranded tags from failed/cancelled CI runs (like NoteCraft v0.0.16),
# correctly picking up releases without a 'v' prefix (like CapsuleCraft 2.12.5), and ignoring pre-releases
# and release candidates (like SolidCraft v0.45.0-rc.4).
# Falls back to git ls-remote filtered for clean semver tags if the GitHub redirect cannot be reached.
resolve_latest_stable_tag() {
    local origin="$1" repo="$2"
    local tag=""

    # 1. Check GitHub's /releases/latest redirect
    if [[ "$origin" =~ ^https://github\.com/([^/]+)/([^/]+)/?$ ]]; then
        local gh_org="${BASH_REMATCH[1]}"
        local gh_repo="${BASH_REMATCH[2]%.git}"
        local latest_url="https://github.com/$gh_org/$gh_repo/releases/latest"
        local effective_url
        effective_url="$(curl -sIL -o /dev/null -w '%{url_effective}' --connect-timeout 5 --max-time 10 "$latest_url" 2>/dev/null || true)"
        if [[ "$effective_url" =~ /releases/tag/([^/?#]+) ]]; then
            tag="${BASH_REMATCH[1]}"
        fi
    fi

    # 2. Fallback: query git remote tags, excluding pre-releases (-rc, -alpha, -beta, -nightly, etc.)
    # and matching standard semver tags
    if [[ -z "$tag" ]]; then
        tag="$(git ls-remote --tags --sort=-v:refname "$repo" 2>/dev/null | \
            grep -v '\^{}' | \
            awk '{print $2}' | \
            sed 's|refs/tags/||' | \
            grep -E '^v?[0-9]+(\.[0-9]+)+$' | \
            head -n 1 || true)"
    fi

    # 3. Final fallback: traditional v* glob
    if [[ -z "$tag" ]]; then
        tag="$(git ls-remote --tags --sort=-v:refname "$repo" 'v*' 2>/dev/null | \
            grep -v '\^{}' | \
            head -n 1 | \
            awk '{print $2}' | \
            sed 's|refs/tags/||' || true)"
    fi

    printf '%s' "$tag"
}

if [[ -e "$SRC" ]]; then
    if [[ -d "$SRC" && ! -d "$SRC/.git" && -z "$(ls -A "$SRC" 2>/dev/null)" ]]; then
        rmdir "$SRC" 2>/dev/null || true
    fi
fi

if [[ -e "$SRC" ]]; then
    [[ -d "$SRC/.git" ]] || fail "$SRC exists but is not a Git checkout. Move it aside manually before retrying."
    ORIGIN="$(git -C "$SRC" remote get-url origin 2>/dev/null || true)"
    case "${ORIGIN%.git}" in
        "$APP_ORIGIN") ;;
        *) fail "$SRC has unexpected origin '$ORIGIN'; refusing to reuse or overwrite it." ;;
    esac
    if ! source_tree_is_complete "$SRC"; then
        fail "$SRC is an incomplete checkout: some files of the workspace are missing on disk. That happens when a clone is interrupted (a killed build, or a full disk). Move $SRC aside and run the build again to re-clone it."
    fi
    if [[ "$UPDATE_SOURCE" == "true" ]]; then
        echo "Checking for upstream source updates for $TITLE ($REPO)..."
        LATEST_REF=""
        if [[ "$BUILD_SOURCE" == "stable" ]]; then
            LATEST_TAG="$(resolve_latest_stable_tag "$APP_ORIGIN" "$REPO")"
            if [[ -n "$LATEST_TAG" ]]; then
                LATEST_REF="$LATEST_TAG"
            fi
        elif [[ "$BUILD_SOURCE" != "main" && "$BUILD_SOURCE" != "master" && "$BUILD_SOURCE" != "latest" && -n "$BUILD_SOURCE" ]]; then
            LATEST_REF="$BUILD_SOURCE"
        fi

        CURRENT_TAG="$(git -C "$SRC" describe --tags --exact-match 2>/dev/null || true)"
        CURRENT_COMMIT="$(git -C "$SRC" rev-parse HEAD 2>/dev/null || true)"
        IS_OUTDATED=false

        if [[ -n "$LATEST_REF" ]]; then
            if [[ "$CURRENT_TAG" != "$LATEST_REF" ]]; then
                IS_OUTDATED=true
            fi
        else
            REMOTE_HEAD="$(git ls-remote "$REPO" HEAD 2>/dev/null | awk '{print $1}')"
            if [[ -n "$REMOTE_HEAD" && "$CURRENT_COMMIT" != "$REMOTE_HEAD" ]]; then
                IS_OUTDATED=true
                LATEST_REF="$REMOTE_HEAD"
            fi
        fi

        if [[ "$IS_OUTDATED" == "true" ]]; then
            echo "Current source (${CURRENT_TAG:-${CURRENT_COMMIT:0:7}}) is outdated compared to upstream ($LATEST_REF). Updating..."
            git -c filter.lfs.smudge= -c filter.lfs.required=false -c filter.lfs.clean= -C "$SRC" fetch --tags origin >/dev/null 2>&1 || git -c filter.lfs.smudge= -c filter.lfs.required=false -c filter.lfs.clean= -C "$SRC" fetch origin >/dev/null 2>&1
            if git -C "$SRC" checkout -f "$LATEST_REF" >/dev/null 2>&1; then
                rm -rf "$SRC/apps/$APP-android"
                echo "Successfully updated $TITLE source to $LATEST_REF ($(git -C "$SRC" rev-parse --short HEAD 2>/dev/null || true))."
            else
                echo "Warning: Could not checkout $LATEST_REF; continuing with existing source."
            fi
        else
            echo "Current source is up to date (${CURRENT_TAG:-${CURRENT_COMMIT:0:7}})."
        fi
    else
        echo "Skipping update check (--noupdate passed)."
        echo "Reusing existing source checkout."
    fi
    echo "Ref/Commit: $(git -C "$SRC" describe --tags --always 2>/dev/null || git -C "$SRC" rev-parse --short HEAD 2>/dev/null || echo unknown)"
else
    # The clone goes into a scratch directory first and is only moved into place once the tree
    # has been verified, so an interrupted clone can never become the checkout a later build
    # reuses. `df` is checked up front as well: a clone that runs out of disk is the usual
    # reason for an incomplete tree in the first place.
    SRC_TMP="$ROOT/source/.clone-$APP.$$"
    rm -rf "$SRC_TMP"
    FREE_BYTES="$(df -k "$ROOT/source" 2>/dev/null | awk 'NR==2 {print $4 * 1024}')"
    if [[ "$FREE_BYTES" =~ ^[0-9]+$ ]] && (( FREE_BYTES < 1048576000 )); then
        rm -rf "$SRC_TMP"
        fail "Only $((FREE_BYTES / 1048576)) MiB are free on the disk holding $ROOT/source, but cloning $TITLE needs about 1 GiB. Free some space (build/ holds the Cargo output of every app) and run the build again."
    fi

    TARGET_REF=""
    if [[ "$BUILD_SOURCE" == "stable" ]]; then
        echo "Looking up latest stable release tag for $TITLE ($REPO)..."
        LATEST_TAG="$(resolve_latest_stable_tag "$APP_ORIGIN" "$REPO")"
        if [[ -n "$LATEST_TAG" ]]; then
            TARGET_REF="$LATEST_TAG"
            echo "  -> Found latest stable release tag: $TARGET_REF"
        else
            echo "  -> No stable release tag found; falling back to default branch."
        fi
    elif [[ "$BUILD_SOURCE" != "main" && "$BUILD_SOURCE" != "master" && "$BUILD_SOURCE" != "latest" && -n "$BUILD_SOURCE" ]]; then
        TARGET_REF="$BUILD_SOURCE"
        echo "Using target ref: $TARGET_REF"
    else
        echo "Building against upstream main branch."
    fi

    CLONE_ARGS=(--depth 1)
    if [[ -n "$TARGET_REF" ]]; then
        CLONE_ARGS+=(--branch "$TARGET_REF")
    fi

    if ! git -c filter.lfs.smudge= -c filter.lfs.required=false -c filter.lfs.clean= clone "${CLONE_ARGS[@]}" "$REPO" "$SRC_TMP"; then
        rm -rf "$SRC_TMP"
        fail "Cloning $REPO (ref: ${TARGET_REF:-default}) failed. Check the network connection and the free disk space, then run the build again."
    fi
    if ! source_tree_is_complete "$SRC_TMP"; then
        rm -rf "$SRC_TMP"
        fail "The clone of $REPO came back incomplete: some files of the workspace are missing on disk. That usually means the disk filled up during the clone. Free some space and run the build again."
    fi
    mv "$SRC_TMP" "$SRC"
fi

# Export upstream release commit SHA and build date for the app being built.
# Upstream ArtCraft apps use <APP>_BUILD_SHA and <APP>_BUILD_DATE in their About dialog and CLI
# (e.g. WORDCRAFT_BUILD_SHA, PHOTOCRAFT_BUILD_SHA, FILMCRAFT_BUILD_SHA). When absent, they fall back
# to "(development build)" or "dev". Setting these ensures Android builds display the exact release commit.
APP_COMMIT_SHA="$(git -C "$SRC" rev-parse HEAD 2>/dev/null || true)"
BUILD_DATE="$(date -u +%Y-%m-%d)"
APP_UPPER="$(echo "$APP" | tr '[:lower:]' '[:upper:]')"
if [[ "$MODE" == "release" && -n "$APP_COMMIT_SHA" ]]; then
    export "${APP_UPPER}_BUILD_SHA"="$APP_COMMIT_SHA"
    export "${APP_UPPER}_BUILD_DATE"="$BUILD_DATE"
    export BUILD_SHA="$APP_COMMIT_SHA"
    export BUILD_DATE="$BUILD_DATE"
    export GIT_HASH="$APP_COMMIT_SHA"
    export GIT_COMMIT="$APP_COMMIT_SHA"
    echo "  -> Baked release build commit: ${APP_COMMIT_SHA:0:8} (${APP_UPPER}_BUILD_SHA)"
else
    echo "  -> Development/debug build mode: leaving ${APP_UPPER}_BUILD_SHA unset (will show development build)"
fi

# Remove pinned upstream toolchains that might lack Android targets in CI runners
rm -f "$SRC/rust-toolchain" "$SRC/rust-toolchain.toml"

# Upstream `main` is normally green, but a regression can sit there for a while (the upstream
# preset-loader fix landed broken on 2026-10-17 and took its CI red). Each fix-up below is
# guarded: it edits the checkout only while the broken text is still present, and is a silent
# no-op once upstream repairs it, so nothing is frozen or permanently forked. Keep this list tiny
# and delete entries once upstream has shipped the fix.
python3 "$ROOT/lib/upstream_patches.py" "$SRC" "$APP" "$TITLE"

# Two-finger scrolling (the app-side gesture guard and the momentum hand-off) is applied by its
# own script, so it can be turned off for testing by commenting out the line below. It patches the
# app's own source at build time; the scroll synthesis itself is the vendored winit/egui-winit
# backends above.
bash "$ROOT/lib/scrolling.sh" "$SRC" "$APP" "$TITLE"

# SubCraft's Android file hand-off (a public `files::deliver_android`) is applied by its own script,
# so it can be turned off for testing by commenting out the line below. It patches the app's own
# source at build time; only SubCraft is touched.
bash "$ROOT/lib/subcraft.sh" "$SRC" "$APP" "$TITLE"

# Topbar modifiers (Shift, Ctrl, Alt toggle buttons integrated into topbar/ribbon for 6 apps).
bash "$ROOT/lib/topbar_modifiers.sh" "$SRC" "$APP" "$TITLE" "$NO_BUTTONS"

# Unofficial Android port disclaimer in About dialogs.
bash "$ROOT/lib/about_dialog.sh" "$SRC" "$APP" "$TITLE"

say 4 "Preparing the NativeActivity launcher..."
if [[ -x "adapters/$APP/prepare.sh" ]]; then
    echo "Using app-specific adapter: adapters/$APP/prepare.sh"
    bash "adapters/$APP/prepare.sh" "$APP" "$TITLE" "$SRC" "$MODE"
    if [[ -d "$SRC/apps/$APP-android/src" ]]; then
        text_bar_src="$ROOT/templates/android/mobile_text_bar.rs"
        if [[ "$NO_BUTTONS" == "true" ]]; then
            text_bar_src="$ROOT/templates/android/mobile_text_bar_disabled.rs"
        fi
        [[ -f "$text_bar_src" ]] && cp -f "$text_bar_src" "$SRC/apps/$APP-android/src/mobile_text_bar.rs"
        [[ -f "$ROOT/templates/android/touch_gestures.rs" ]] && cp -f "$ROOT/templates/android/touch_gestures.rs" "$SRC/apps/$APP-android/src/touch_gestures.rs"
        if [[ "$APP" == "subcraft" || "$APP" == "tracedraw" || "$APP" == "picassosuite" || "$APP" == "bimcraft" || "$APP" == "earthcraft" || "$APP" == "mapcraft" || "$APP" == "transit" || "$APP" == "anicraft" || "$APP" == "papercraft" || "$APP" == "ferrite" || "$APP" == "pixelcraft" ]]; then
            sed -i '/source: egui::MouseWheelSource::Unknown,/d' "$SRC/apps/$APP-android/src/touch_gestures.rs" 2>/dev/null || true
        fi
    fi
else
    python3 "$ROOT/lib/launcher_generator.py" "$ROOT" "$SRC" "$APP" "$TITLE" "$TARGET_SDK" "$APPNAME" "$PACKAGENAME" "$APPICON" "$NO_BUTTONS"
fi

# The shared update checker (templates/android/UpdateCheck.java) is installed next to every
# launcher's Activity, whichever path produced that launcher: the generated one and the
# per-app adapters alike. It compares the builder version baked into the APK with the latest
# GitHub release and offers the download when GitHub has a newer one.
UPDATER_DIR="$ANDROID_DIR/java/com/artcraft/updater"
if [[ -d "$ANDROID_DIR/java" ]]; then
    mkdir -p "$UPDATER_DIR"
    python3 - "$ROOT/templates/android/UpdateCheck.java" "$UPDATER_DIR/UpdateCheck.java" \
        "$BUILDER_VERSION" "$UPDATE_API_URL" "$UPDATE_PAGE_URL" <<'PY'
from pathlib import Path
import sys

source, target, version, api_url, page_url = sys.argv[1:6]
text = Path(source).read_text()
text = (
    text.replace("__BUILDER_VERSION__", version)
    .replace("__UPDATE_API_URL__", api_url)
    .replace("__UPDATE_PAGE_URL__", page_url)
)
Path(target).write_text(text)
PY
    echo "  -> installed the update checker (builder version $BUILDER_VERSION)"
fi

IME_DIR="$ANDROID_DIR/java/com/artcraft/ime"
if [[ -d "$ANDROID_DIR/java" ]]; then
    mkdir -p "$IME_DIR"
    cp "$ROOT/templates/android/ArtCraftIme.java" "$IME_DIR/ArtCraftIme.java"
fi


[[ -f "$ANDROID_DIR/Cargo.toml" && -f "$ANDROID_DIR/src/lib.rs" ]] || fail "Android wrapper is incomplete at $ANDROID_DIR. See the build log and adapters/README.md."

# WordCraft's soft-keyboard Backspace-at-the-start-of-a-paragraph fix is applied by its own
# script, so it can be turned off for testing by commenting out the line below. It patches the
# app's own keys.rs at build time (an IME may still deliver DeleteSurrounding instead of text
# state); only WordCraft is touched. The Activity/bridge IME patches are gone: the launchers and
# adapters use android-activity's native IME.
bash "$ROOT/lib/ime.sh" "$SRC" "$APP" "$TITLE"

# egui-winit's stock Android build gates its `arboard` clipboard backend behind
# `cfg(not(target_os = "android"))`, so on Android its `Clipboard` is only an in-app
# `String` — copy/paste never reaches the system clipboard. Vendor the builder's
# patched egui-winit (a self-contained JNI `ClipboardManager` backend) and patch it
# into the workspace so every app's eframe event loop uses it. The patch is
# platform-gated, so desktop builds are unaffected.
echo "Vendoring the Android-compatible egui-winit for $TITLE (system clipboard via JNI)..."
mkdir -p "$SRC/vendor"
if [[ "$APP" == "subcraft" || "$APP" == "tracedraw" || "$APP" == "picassosuite" || "$APP" == "bimcraft" || "$APP" == "earthcraft" || "$APP" == "mapcraft" || "$APP" == "transit" || "$APP" == "anicraft" || "$APP" == "papercraft" || "$APP" == "pixelcraft" ]]; then
    rm -rf "$SRC/vendor/egui-winit"
    cp -a "$ROOT/vendor/egui-winit-0.36" "$SRC/vendor/egui-winit"
elif [[ "$APP" == "ferrite" ]]; then
    rm -rf "$SRC/vendor/egui-winit"
    cp -a "$ROOT/vendor/egui-winit-0.34" "$SRC/vendor/egui-winit"
elif [[ "$APP" == "solidcraft" || "$APP" == "pcbcraft" || "$APP" == "fieldcraft" ]]; then
    # SolidCraft, PCBCraft and FieldCraft prepare.sh handle their own egui/winit setup
    echo "Skipping builder egui-winit copy for $APP (handled by prepare.sh)"
else
    rm -rf "$SRC/vendor/egui-winit"
    cp -a "$ROOT/vendor/egui-winit" "$SRC/vendor/egui-winit"
fi
if [[ "$APP" == "solidcraft" || "$APP" == "pcbcraft" || "$APP" == "fieldcraft" ]]; then
    # SolidCraft, PCBCraft and FieldCraft prepare.sh handle all egui/egui-winit/egui-wgpu patching
    echo "Skipping egui patching for $APP (handled by prepare.sh)"
else
python3 "$ROOT/lib/cargo_patches.py" patch-crates-io "$SRC/Cargo.toml" egui-winit auto "https://github.com/storytold/egui.git"
fi

# Some apps call `rfd` directly from a crate that ends up in the launcher's dependency graph
# (PDFCraft does, in ~18 files), and rfd 0.17 ships no Android backend, so it fails to compile for
# `aarch64-linux-android`. Vendor the builder's Android-compatible copy (a stub backend that
# resolves dialogs to "cancelled") and patch it into the workspace so the launcher builds. Real
# open/save on Android needs a platform adapter (see adapters/README.md).
if grep -rqE '\brfd::' "$SRC"/crates/*/src "$SRC"/apps/*/src 2>/dev/null || grep -qE '^\s*rfd\s*=' "$SRC/Cargo.toml" 2>/dev/null || grep -rqE '\brfd\b' "$ROOT/adapters/$APP" 2>/dev/null; then
    echo "Vendoring an Android-compatible rfd for $TITLE (native file dialogs become no-ops)..."
    mkdir -p "$SRC/vendor"
    rm -rf "$SRC/vendor/rfd"
    cp -a "$ROOT/vendor/rfd" "$SRC/vendor/rfd"
    python3 "$ROOT/lib/cargo_patches.py" patch-crates-io "$SRC/Cargo.toml" rfd "vendor/rfd"
fi

# Some apps also use arboard (the clipboard) from a crate in the launcher's dependency graph
# (PDFCraft does). arboard ships no Android backend and its Unix backend needs X11/Wayland, so it
# cannot compile for `aarch64-linux-android`. Vendor the builder's Android-compatible stub (a
# no-op clipboard) and patch it in. If crates use `arboard = { workspace = true }`, we must
# replace the workspace dependency; otherwise we add it via `[patch.crates-io]`.
if grep -rqE '\barboard\b' "$SRC"/crates "$SRC"/apps "$SRC"/src "$SRC"/Cargo.lock "$SRC"/Cargo.toml 2>/dev/null; then
    echo "Vendoring an Android-compatible arboard for $TITLE (clipboard access becomes a no-op)..."
    mkdir -p "$SRC/vendor"
    rm -rf "$SRC/vendor/arboard"
    cp -a "$ROOT/vendor/arboard" "$SRC/vendor/arboard"
    python3 "$ROOT/lib/cargo_patches.py" patch-arboard "$SRC/Cargo.toml" "$SRC"
fi

# midir on Android needs a platform adapter or dummy backend. Vendor the builder's Android-compatible
# midir stub (with dummy MIDI backend) and patch it into the workspace when midir is in dependencies.
if [[ -d "$ROOT/vendor/midir" ]] && (grep -rqE '\bmidir\b' "$SRC"/crates "$SRC"/apps "$SRC"/src 2>/dev/null || grep -qE '^\s*midir\s*=' "$SRC/Cargo.toml" 2>/dev/null); then
    echo "Vendoring an Android-compatible midir for $TITLE (dummy MIDI backend)..."
    mkdir -p "$SRC/vendor"
    rm -rf "$SRC/vendor/midir"
    cp -a "$ROOT/vendor/midir" "$SRC/vendor/midir"
    python3 "$ROOT/lib/cargo_patches.py" patch-crates-io "$SRC/Cargo.toml" midir "vendor/midir"
fi

# rustls-platform-verifier on Android delegates to Kotlin `org.rustls.platformverifier.CertificateVerifier`.
# In native Activity APKs, this Kotlin class is missing and JNI is uninitialized, breaking all TLS connections.
# Vendor the builder's WebPKI-backed rustls-platform-verifier (Mozilla root certs directly in Rust) and patch it in.
if [[ -d "$ROOT/vendor/rustls-platform-verifier" ]] && (grep -rqE '\brustls-platform-verifier\b' "$SRC/Cargo.lock" "$SRC"/Cargo.toml 2>/dev/null); then
    echo "Vendoring an Android-compatible rustls-platform-verifier for $TITLE (WebPKI Mozilla root certs)..."
    mkdir -p "$SRC/vendor"
    rm -rf "$SRC/vendor/rustls-platform-verifier"
    cp -a "$ROOT/vendor/rustls-platform-verifier" "$SRC/vendor/rustls-platform-verifier"
    python3 "$ROOT/lib/cargo_patches.py" patch-crates-io "$SRC/Cargo.toml" rustls-platform-verifier "vendor/rustls-platform-verifier"
fi

# winit's stock Android backend only translates touch actions: `handle_input_event` maps
# MotionAction::{Down,Move,Up,Cancel} to egui's `Touch` events and drops everything else
# (`_ => None // TODO mouse events`), so a connected mouse degenerated into finger taps —
# no right click, no wheel scrolling, no hover. Vendor the builder's patched winit (mouse
# buttons, wheel -> MouseWheel, hover -> CursorMoved/Entered/Left, mouse-source moves no
# longer masquerade as touch) and patch it into the workspace. PDFCraft already ships a
# forked winit on the same upstream base: only its Android backend directory is
# overwritten, so its non-Android customizations and its existing patch entry survive.
echo "Vendoring the mouse/scroll-aware winit for $TITLE..."
mkdir -p "$SRC/vendor"
if [[ -d "$SRC/vendor/winit" ]]; then
    # Upstream ships a storytold winit fork that re-adds `WindowEvent::ImeResetCompleted` /
    # `reset_ime_with_request` (removed from stock winit 0.30), which the fork's egui-winit 0.37
    # requires. Keep that fork intact so egui-winit stays compatible, and only refresh its Android
    # backend with the builder's patched mouse/scroll-aware backend.
    rm -rf "$SRC/vendor/winit/src/platform_impl/android"
    cp -a "$ROOT/vendor/winit/src/platform_impl/android" "$SRC/vendor/winit/src/platform_impl/android"
else
    rm -rf "$SRC/vendor/winit"
    cp -a "$ROOT/vendor/winit" "$SRC/vendor/winit"
fi

# The storytold winit fork re-adds `WindowEvent::ImeResetCompleted` / `reset_ime_with_request`
# (removed from stock winit 0.30) and its egui-winit calls `window.reset_ime_with_request()`. Keep
# winit and egui-winit consistent: ensure the vendored winit declares `ImeResetCompleted` + its
# `reset_ime_with_request` method, and add a no-op `reset_ime_with_request` to the Android `Window`
# impl (Android drives its own IME pipeline).
python3 "$ROOT/lib/cargo_patches.py" patch-winit-ime "$SRC"

# egui_dock (docking system for egui) vendors egui itself. Subcraft depends on egui_dock 0.21
# which pulls crates.io egui 0.36 into the graph alongside the builder's vendored egui 0.37,
# causing every `egui_dock::egui::Id` / `egui::Id` mismatch. Vendor egui_dock and point its
# egui dependency at the vendored egui 0.37 so both egui_dock and the app share one egui.
if grep -rqE '\begui_dock\b' "$SRC"/crates/*/src "$SRC"/apps/*/src 2>/dev/null; then
    echo "Vendoring egui_dock against the vendored egui for $TITLE..."
    mkdir -p "$SRC/vendor"
    rm -rf "$SRC/vendor/egui_dock"
    cp -a "$ROOT/vendor/egui_dock" "$SRC/vendor/egui_dock"
    python3 "$ROOT/lib/cargo_patches.py" patch-egui-dock "$SRC"
fi

# subcraft's fork egui-winit references accesskit_winit via a relative path that resolves to a
# non-existent nested `vendor/egui/vendor/winit`. Point it at the top-level vendored winit which
# the builder keeps in sync.
python3 "$ROOT/lib/cargo_patches.py" patch-subcraft-accesskit "$SRC"

python3 "$ROOT/lib/cargo_patches.py" patch-crates-io "$SRC/Cargo.toml" winit "vendor/winit"

# egui-wgpu's lost-surface recovery kept the old surface alive while it created the new one: the
# old `SurfaceState` owns the old `wgpu::Surface`, and it lived until the end of the function, so
# the new surface was created for a window that still had the old one. Android's Vulkan refuses
# that (ERROR_NATIVE_WINDOW_IN_USE_KHR) and wgpu panics on the error, so an app whose surface was
# lost — the app coming back from a file picker, for instance — died with it. Vendor the builder's
# patched egui-wgpu, which releases the old surface first, and patch it into the workspace.
echo "Vendoring the surface-fixing egui-wgpu for $TITLE..."
if [[ ("$APP" == "solidcraft" || "$APP" == "fieldcraft" || "$APP" == "pcbcraft") && (-d "$SRC/vendor/egui/crates/egui-wgpu" || -d "$SRC/vendor/egui-wgpu") ]]; then
    # Upstream already vendors egui-wgpu with its own custom additions (e.g. SolidCraft's CAD
    # pipelines, blur shaders, FrameCompositor, and DeviceLost). Keep its custom APIs intact
    # and patch the Android surface-loss recovery in-place in its winit.rs.
    wgpu_target="$SRC/vendor/egui/crates/egui-wgpu"
    [[ -d "$wgpu_target" ]] || wgpu_target="$SRC/vendor/egui-wgpu"
    python3 "$ROOT/lib/cargo_patches.py" patch-egui-wgpu-surface "$wgpu_target"
elif [[ "$APP" == "solidcraft" || "$APP" == "fieldcraft" || "$APP" == "pcbcraft" || "$APP" == "ferrite" ]]; then
    # SolidCraft, FieldCraft, PCBCraft and Ferrite handle their own graphics backend
    echo "Skipping builder egui-wgpu copy for $APP (handled by prepare.sh)"
elif [[ "$APP" == "subcraft" || "$APP" == "tracedraw" || "$APP" == "picassosuite" || "$APP" == "bimcraft" || "$APP" == "earthcraft" || "$APP" == "mapcraft" || "$APP" == "transit" || "$APP" == "anicraft" || "$APP" == "papercraft" || "$APP" == "pixelcraft" ]]; then
    rm -rf "$SRC/vendor/egui-wgpu"
    cp -a "$ROOT/vendor/egui-wgpu-0.36" "$SRC/vendor/egui-wgpu"
else
    rm -rf "$SRC/vendor/egui-wgpu"
    cp -a "$ROOT/vendor/egui-wgpu" "$SRC/vendor/egui-wgpu"
fi

# Skip egui-wgpu patching for SolidCraft, FieldCraft, PCBCraft and Ferrite - prepare.sh handles it
if [[ "$APP" != "solidcraft" && "$APP" != "fieldcraft" && "$APP" != "pcbcraft" && "$APP" != "ferrite" ]]; then
python3 "$ROOT/lib/cargo_patches.py" patch-crates-io "$SRC/Cargo.toml" egui-wgpu "vendor/egui-wgpu" "https://github.com/storytold/egui.git"
fi

# Every launcher uses the builder's vendored android-activity: the native-activity backend now
# carries three builder patches the crates.io copy lacks. (1) The sync-picker ANR fix: while a
# blocking pick parks the Rust main thread, the JVM-thread lifecycle, save-state and window
# callbacks must not wait on it (they would be ANR'd by Android after ~5s); the SAF bridge flips
# `set_nonblocking_callbacks` around each pick and the C glue then only writes the command pipe.
# (2) `show_soft_input`/`hide_soft_input` use NativeActivity's native IME calls: egui's TextEdit
# widgets request the soft keyboard automatically through the standard input method framework.
# (3) `set_nonblocking_callbacks` is exported for native-activity too (the SAF bridge calls it).
echo "Vendoring the ANR-safe android-activity glue for $TITLE..."
rm -rf "$SRC/vendor/android-activity"
cp -a "$ROOT/vendor/android-activity" "$SRC/vendor/android-activity"
python3 "$ROOT/lib/cargo_patches.py" patch-crates-io "$SRC/Cargo.toml" android-activity "vendor/android-activity"

# sys-locale on Android reads persist.sys.locale via libc, which modern Android SELinux rejects.
# Vendor the builder's patched sys-locale (which inspects LC_ALL, LANG, LANGUAGE first).
if grep -rqE '\bsys_locale\b' "$SRC"/crates/*/src "$SRC"/apps/*/src 2>/dev/null || grep -qE '^\s*sys-locale\s*=' "$SRC/Cargo.toml" 2>/dev/null; then
    echo "Vendoring an Android SELinux-safe sys-locale for $TITLE..."
    mkdir -p "$SRC/vendor"
    rm -rf "$SRC/vendor/sys-locale"
    cp -a "$ROOT/vendor/sys-locale" "$SRC/vendor/sys-locale"
    python3 "$ROOT/lib/cargo_patches.py" patch-crates-io "$SRC/Cargo.toml" sys-locale "vendor/sys-locale"
fi

# A `target/` directory left behind by running cargo inside a vendored crate must never be
# copied into the app tree: it is hundreds of megabytes and cargo ignores nested target dirs
# of path dependencies anyway. `rm -rf` is a no-op when the glob matches nothing.
rm -rf "$SRC/vendor"/*/target

# The Android adapter (adapters/<app>/) supplies both the complete eframe shell and the SAF
# file bridge, so the builder no longer rewrites apps/vectorcraft-android/src/lib.rs here. The
# hardcoded eframe-bridge repairs that used to live at this point were removed: they installed
# Services::default() (no file services) and could clobber the adapter.



say 5 "Preparing a builder-owned signing key..."
KEY="keys/artcraft-builder.jks"
KEY_PASSWORD_FILE="keys/.keystore-password"
mkdir -p keys
chmod 700 keys

if [[ -n "${ARTCRAFT_KEYSTORE_BASE64:-}" && ! -f "$KEY" ]]; then
    echo "$ARTCRAFT_KEYSTORE_BASE64" | base64 -d > "$KEY"
    chmod 600 "$KEY"
fi

if [[ ! -f "$KEY_PASSWORD_FILE" ]]; then
    if [[ -n "${ARTCRAFT_KEYSTORE_PASSWORD:-}" ]]; then
        printf '%s\n' "$ARTCRAFT_KEYSTORE_PASSWORD" > "$KEY_PASSWORD_FILE"
    elif [[ -f "$KEY" ]]; then
        # Compatibility with earlier builder versions, which used this local-only password.
        printf 'android\n' > "$KEY_PASSWORD_FILE"
    else
        python3 -c 'import secrets; print(secrets.token_hex(24))' > "$KEY_PASSWORD_FILE"
    fi
    chmod 600 "$KEY_PASSWORD_FILE"
fi
KEY_PASSWORD="$(<"$KEY_PASSWORD_FILE")"
if [[ ! -f "$KEY" ]]; then
    if [[ -n "${CI:-}" && "$MODE" == "release" && "${GITHUB_REF:-}" =~ ^refs/tags/ ]]; then
        fail "Release tag builds in CI require a persistent release keystore (set ARTCRAFT_KEYSTORE_BASE64 secret) to maintain Android update signature continuity."
    fi
    if [[ -n "${CI:-}" ]]; then
        echo "WARNING: Generating an ephemeral signing key for CI non-tag build. Generated APKs will not be update-compatible with official releases." >&2
    fi
    command -v keytool >/dev/null 2>&1 || fail "keytool is required. Install a JDK (on Fedora: sudo dnf install java-25-openjdk-devel)."
    keytool -genkeypair -noprompt -v \
        -keystore "$KEY" -storetype JKS \
        -storepass "$KEY_PASSWORD" -keypass "$KEY_PASSWORD" \
        -alias artcraftbuilder -keyalg RSA -keysize 2048 -validity 10000 \
        -dname "CN=ArtCraft Android Builder, OU=Local Builds, O=ArtCraft, C=US"
    chmod 600 "$KEY"
fi

# The same key reaches Gradle at build time through ARTCRAFT_KEYSTORE* environment variables
# (wired to the signingConfigs in the generated project below); the launcher's Cargo.toml no
# longer carries a signing section.

# Some upstream releases dropped `egui-wgpu`/`egui-winit` from [workspace.dependencies] while
# egui_kittest still inherits them via `workspace = true`, which breaks `cargo metadata` for
# the whole workspace. Re-add them (vendored egui fork) when missing. Run here, after vendoring
# (which rewrites Cargo.toml), so the addition survives to the cargo metadata call below.
python3 "$ROOT/lib/cargo_patches.py" ensure-workspace-deps "$SRC" "$APP"

# The storytold winit fork enables desktop wayland by default, which pulls sctk-adwaita ->
# crossfont -> fontconfig-sys v5 into the resolution graph. That collides with
# egui_system_font_provider -> fontique -> fontconfig-sys v6 (both `links = "fontconfig"`), and
# cargo refuses to build. Wayland is never used on Android, so empty the wayland feature family
# (drop from default + empty the definitions) so crossfont is never pulled. The ImeReset variant
# that egui-winit 0.37 needs lives in winit's source (untouched here), so fork egui-winit keeps
# compiling against fork winit.
python3 "$ROOT/lib/cargo_patches.py" strip-winit-wayland "$SRC"

say 6 "Building $TITLE ($MODE)..."
mkdir -p "build/$APP/target" output logs
export CARGO_TARGET_DIR="$ROOT/build/$APP/target"
STAMP="$(date +%Y%m%d-%H%M%S)"
LOG="logs/$APP-$MODE-$STAMP.log"
cd "$ANDROID_DIR"

# The APK is named after the version the app's own source declares, so a release built from
# `vectorcraft` v0.12.0 lands as `output/vectorcraft-0.12.0.apk` instead of a version-less
# `vectorcraft-release.apk`. Read the workspace `[workspace.package] version` (the value cargo bakes
# into the APK's own `versionName`) and fall back to the launcher crate's version for sources that do
# not pin one at the workspace level.
APP_VERSION="$(python3 - "$ROOT/$SRC" "$APP" <<'PY'
import pathlib, re, sys, tomllib

root, app = pathlib.Path(sys.argv[1]), sys.argv[2]
normalize = lambda value: re.sub(r"[^a-z0-9]", "", value.lower())
app_key = normalize(app)

def read_version(path):
    try:
        data = tomllib.loads(path.read_text())
    except (OSError, tomllib.TOMLDecodeError):
        return None
    version = data.get("workspace", {}).get("package", {}).get("version")
    if not version:
        version = data.get("package", {}).get("version")
    return version if isinstance(version, str) and version else None

# Prefer the source's own version: the workspace version, then the app's crate, then a crate
# named like the app (coiman-ide for coimanide), and only then the generated launcher's port
# version. This keeps APKs named after the upstream release they were built from.
for path in (root / "Cargo.toml", root / "apps" / app / "Cargo.toml"):
    version = read_version(path)
    if version:
        print(version)
        break
else:
    version = None
    for path in sorted(root.glob("crates/*/Cargo.toml")):
        try:
            name = tomllib.loads(path.read_text()).get("package", {}).get("name", "")
        except (OSError, tomllib.TOMLDecodeError):
            continue
        if name and (normalize(name) == app_key or normalize(name).startswith(app_key)):
            version = read_version(path)
            if version:
                break
    if not version:
        version = read_version(root / "apps" / (app + "-android") / "Cargo.toml")
    print(version or "0.0.0")
PY
)"

# The shared craft-fonts (https://github.com/storytold/craft-fonts) are an optional build input
# for the apps that embed them (WordCraft, PdfCraft): with `CRAFT_FONTS_DIR` pointing at a
# checkout, their fonts crate embeds the faces listed in that checkout's `fonts/manifest.txt`.
#
# Without it they build with no face of their own for a script the device may not have, and the
# PDF text writer then refuses the text instead of writing it: "The PDF text fonts can't draw
# “ا” (U+0627)" — which is what the Android build did, because it only made an empty directory
# where the fonts were expected.
#
# The checkout is kept, like the apps' own sources, so a local pin is not overwritten; a checkout
# that is there but has no `fonts/` directory is left alone and the apps build without the faces.
CRAFT_FONTS_DIR="$ROOT/source/craft-fonts"
if [[ -f "$CRAFT_FONTS_DIR/fonts/manifest.txt" ]]; then
    echo "Reusing the craft-fonts checkout in $CRAFT_FONTS_DIR"
elif [[ -e "$CRAFT_FONTS_DIR" ]]; then
    if [[ -d "$CRAFT_FONTS_DIR" && -z "$(ls -A "$CRAFT_FONTS_DIR" 2>/dev/null)" ]]; then
        rmdir "$CRAFT_FONTS_DIR" 2>/dev/null || true
    fi
    if [[ -e "$CRAFT_FONTS_DIR" ]]; then
        echo "The craft-fonts checkout at $CRAFT_FONTS_DIR has no fonts/manifest.txt; leaving it alone." >&2
    fi
fi
if [[ ! -e "$CRAFT_FONTS_DIR" ]]; then
    echo "Fetching craft-fonts (the shared faces the apps embed)..."
    git clone --depth 1 https://github.com/storytold/craft-fonts.git "$CRAFT_FONTS_DIR" || {
        echo "Could not fetch craft-fonts; the apps will build without the shared faces." >&2
    }
fi
if [[ -f "$CRAFT_FONTS_DIR/fonts/manifest.txt" ]]; then
    # The apps' build scripts read the faces from the checkout and embed them; the variable is
    # absolute, so it does not matter where the build runs from.
    export CRAFT_FONTS_DIR
    echo "  -> embedding the craft-fonts faces ($(grep -cve '^#' -e '^$' "$CRAFT_FONTS_DIR/fonts/manifest.txt") of them)"
fi

# The shared craft-i18n translations (https://github.com/storytold/craft-i18n) are embedded
# at compile time by apps like WordCraft and PdfCraft via CRAFT_I18N_DIR.
CRAFT_I18N_DIR="$ROOT/source/craft-i18n"
if [[ -d "$CRAFT_I18N_DIR/locales" ]]; then
    echo "Reusing the craft-i18n checkout in $CRAFT_I18N_DIR"
elif [[ -e "$CRAFT_I18N_DIR" ]]; then
    if [[ -d "$CRAFT_I18N_DIR" && -z "$(ls -A "$CRAFT_I18N_DIR" 2>/dev/null)" ]]; then
        rmdir "$CRAFT_I18N_DIR" 2>/dev/null || true
    fi
    if [[ -e "$CRAFT_I18N_DIR" ]]; then
        echo "The craft-i18n checkout at $CRAFT_I18N_DIR has no locales/; leaving it alone." >&2
    fi
fi
if [[ ! -e "$CRAFT_I18N_DIR" ]]; then
    echo "Fetching craft-i18n (the shared translations the apps embed)..."
    git clone --depth 1 https://github.com/storytold/craft-i18n.git "$CRAFT_I18N_DIR" || {
        echo "Could not fetch craft-i18n; the apps will build without the shared translations." >&2
    }
fi
if [[ -d "$CRAFT_I18N_DIR/locales" ]]; then
    export CRAFT_I18N_DIR
    echo "  -> embedding the craft-i18n catalogs"
fi

if [[ "$APP" == "pdfcraft" ]]; then
    MODELS_DIR="$ROOT/$SRC/assets/models"
    if [[ ! -f "$MODELS_DIR/text-detection.onnx" || ! -f "$MODELS_DIR/text-recognition.onnx" ]]; then
        echo "Fetching OCR models for PdfCraft..."
        mkdir -p "$MODELS_DIR"
        curl -sSL "https://ocrs-models.s3-accelerate.amazonaws.com/text-detection.onnx" -o "$MODELS_DIR/text-detection.onnx" || true
        curl -sSL "https://ocrs-models.s3-accelerate.amazonaws.com/text-recognition.onnx" -o "$MODELS_DIR/text-recognition.onnx" || true
    fi
    if [[ -f "$MODELS_DIR/text-detection.onnx" && -f "$MODELS_DIR/text-recognition.onnx" ]]; then
        echo "  -> packaging OCR models for PdfCraft"
    fi
fi

# Contributor credits: if a refreshed snapshot is available in data/contributors/<app>.json,
# copy it into the app source tree so the About dialog bakes full attribution.
if [[ -f "$ROOT/data/contributors/$APP.json" && -d "$ROOT/$SRC/contributors" ]]; then
    echo "  -> embedding refreshed contributor snapshot from data/contributors/$APP.json"
    cp "$ROOT/data/contributors/$APP.json" "$ROOT/$SRC/contributors/contributors.json"
fi

# The Gradle project, AndroidManifest.xml and theme resource are regenerated from the launcher's
# Cargo metadata on every build (see lib/android_project.py).
echo "  -> generating AndroidManifest.xml, the theme resource and the Gradle project"
python3 "$ROOT/lib/android_project.py" "$ROOT" "$ANDROID_DIR" "$APP" "$APP_VERSION"
# MIN_SDK feeds cargo-ndk's --platform below and must match the APK's minSdk.
# shellcheck source=/dev/null
source "$ANDROID_DIR/.artcraft-gradle.properties"

# SolidCraft uses upstream vendored egui - prepare.sh handles all egui patches
# No additional patching needed here

echo "  -> pre-fetching Cargo dependencies"
cargo fetch --manifest-path "$ANDROID_DIR/Cargo.toml" || true

echo "  -> patching wgpu for Adreno Vulkan swapchain support across the suite"
python3 "$ROOT/lib/cargo_patches.py" patch-wgpu-registry

if [[ "$MODE" == "release" ]]; then
    VAR_SHA="${APP_UPPER}_BUILD_SHA"
    if [[ -z "${!VAR_SHA:-}" && -d "$ROOT/$SRC/.git" ]]; then
        APP_COMMIT_SHA="$(git -C "$ROOT/$SRC" rev-parse HEAD 2>/dev/null || true)"
        if [[ -n "$APP_COMMIT_SHA" ]]; then
            export "${APP_UPPER}_BUILD_SHA"="$APP_COMMIT_SHA"
            export "${APP_UPPER}_BUILD_DATE"="$(date -u +%Y-%m-%d)"
            export BUILD_SHA="$APP_COMMIT_SHA"
            export BUILD_DATE="$(date -u +%Y-%m-%d)"
        fi
    fi
fi

export RUST_FONTCONFIG_DLOPEN=1
echo "  -> building the native library with cargo-ndk (arm64-v8a, API $MIN_SDK)"
rm -rf "$ANDROID_DIR/jniLibs"

# For SolidCraft, PCBCraft, and FieldCraft, cargo-ndk must run from the workspace root
# because it uses path dependencies (../../crates/engine) that
# don't resolve when cargo-ndk runs from the android app subdirectory.
if [[ "$APP" == "solidcraft" || "$APP" == "pcbcraft" || "$APP" == "fieldcraft" || "$APP" == "ferrite" || "$APP" == "papercraft" || "$APP" == "pixelcraft" ]]; then
    NDK_PACKAGE="$APP-android"
    NDK_MANIFEST="$ROOT/$SRC/Cargo.toml"
    if [[ "$MODE" == "release" ]]; then
        if cargo ndk -t arm64-v8a -P "$MIN_SDK" -o "$ANDROID_DIR/jniLibs" --manifest-path "$NDK_MANIFEST" build --release -p "$NDK_PACKAGE" 2>&1 | tee "$ROOT/$LOG"; then
            :
        else
            status=$?
            fail "Release native build failed (exit $status). Log: $ROOT/$LOG"
        fi
    else
        if cargo ndk -t arm64-v8a -P "$MIN_SDK" -o "$ANDROID_DIR/jniLibs" --manifest-path "$NDK_MANIFEST" build -p "$NDK_PACKAGE" 2>&1 | tee "$ROOT/$LOG"; then
            :
        else
            status=$?
            fail "Debug native build failed (exit $status). Log: $ROOT/$LOG"
        fi
    fi
else
    # Standard build from android app directory for other apps
    if [[ "$MODE" == "release" ]]; then
        if cargo ndk -t arm64-v8a -P "$MIN_SDK" -o "$ANDROID_DIR/jniLibs" build --release 2>&1 | tee "$ROOT/$LOG"; then
            :
        else
            status=$?
            fail "Release native build failed (exit $status). Log: $ROOT/$LOG"
        fi
    else
        if cargo ndk -t arm64-v8a -P "$MIN_SDK" -o "$ANDROID_DIR/jniLibs" build 2>&1 | tee "$ROOT/$LOG"; then
            :
        else
            status=$?
            fail "Debug native build failed (exit $status). Log: $ROOT/$LOG"
        fi
    fi
fi

# If any built .so in jniLibs needs libc++_shared.so, bundle it from the NDK toolchain.
#
# The check reads the whole output instead of grepping for the first match: `grep -q` exits at the
# first hit, readelf then dies of SIGPIPE, and with `set -o pipefail` that made the pipeline fail
# (exit 141) — so the C++ runtime was left out of the APK whenever readelf had more output
# pending, and the app died at launch with "library libc++_shared.so not found".
NEEDS_LIBCXX="$(find "$ANDROID_DIR/jniLibs" -name "*.so" -exec readelf -d {} + 2>/dev/null | grep 'Shared library: \[libc++_shared\.so\]' || true)"
if [[ -n "$NEEDS_LIBCXX" ]]; then
    echo "  -> bundling libc++_shared.so from NDK..."
    LIBCXX="$NDK/toolchains/llvm/prebuilt/linux-x86_64/sysroot/usr/lib/aarch64-linux-android/libc++_shared.so"
    if [[ -f "$LIBCXX" ]]; then
        mkdir -p "$ANDROID_DIR/jniLibs/arm64-v8a"
        cp -f "$LIBCXX" "$ANDROID_DIR/jniLibs/arm64-v8a/libc++_shared.so"
    else
        echo "  -> WARNING: the NDK has no libc++_shared.so; the app will not start" >&2
    fi
fi

GRADLE_JDK="$(find_gradle_jdk || true)"
if [[ -z "$GRADLE_JDK" ]]; then
    echo "  -> no JDK 17-23 found; downloading Temurin JDK 21 into .cache/jdk"
    GRADLE_JDK="$(bootstrap_gradle_jdk)"
fi
echo "  -> Gradle JDK: $GRADLE_JDK"

if [[ "$MODE" == "release" ]]; then
    GRADLE_TASK="assembleRelease"
    GRADLE_VARIANT="release"
    FINAL_APK="$ROOT/output/$APP-$APP_VERSION.apk"
else
    GRADLE_TASK="assembleDebug"
    GRADLE_VARIANT="debug"
    FINAL_APK="$ROOT/output/$APP-$APP_VERSION-debug.apk"
fi

echo "  -> packaging and signing with Gradle ($GRADLE_TASK)"
(
    cd "$ANDROID_DIR/android" && \
    JAVA_HOME="$GRADLE_JDK" \
    ARTCRAFT_KEYSTORE="$ROOT/$KEY" \
    ARTCRAFT_KEYSTORE_PASSWORD="$KEY_PASSWORD" \
    ARTCRAFT_KEY_ALIAS="artcraftbuilder" \
    ARTCRAFT_KEY_PASSWORD="$KEY_PASSWORD" \
    ./gradlew --console=plain "$GRADLE_TASK"
) 2>&1 | tee -a "$ROOT/$LOG"
gradle_status="${PIPESTATUS[0]}"
if [[ "$gradle_status" -eq 0 ]]; then
    :
else
    fail "Gradle $GRADLE_TASK failed (exit $gradle_status). Log: $ROOT/$LOG"
fi

# A single-module root project writes build/outputs/apk/<variant>/<rootProject.name>-<variant>.apk;
# an APK whose buildType got no signingConfig lands as -unsigned, which the check below rejects.
APK_DIR="$ANDROID_DIR/android/build/outputs/apk/$GRADLE_VARIANT"
BUILT_APK="$(find "$APK_DIR" -maxdepth 1 -type f -name '*.apk' ! -name '*-unsigned.apk' 2>/dev/null | head -n1)"
[[ -n "$BUILT_APK" && -f "$BUILT_APK" ]] || {
    echo "Expected a Gradle APK under $APK_DIR" >&2
    echo "Searching generated artifacts:" >&2
    find "$ANDROID_DIR/android/build/outputs" -type f -name '*.apk' -print >&2 || true
    fail "Gradle reported success but the APK artifact was not found. Log: $ROOT/$LOG"
}
cp -f "$BUILT_APK" "${FINAL_APK}.tmp" && mv -f "${FINAL_APK}.tmp" "$FINAL_APK"

say 7 "Verifying the Android file bridge in the built APK..."
verify_android_bridge "$APP" "$BUILT_APK"

say 8 "Build succeeded."
echo "App: $TITLE"
echo "Mode: $MODE"
echo "APK: $FINAL_APK"
echo "Size: $(du -h "$FINAL_APK" | awk '{print $1}')"
echo "SHA-256: $(sha256sum "$FINAL_APK" | awk '{print $1}')"
echo "Log: $ROOT/$LOG"
echo
echo "This is a locally signed build. Keep keys/artcraft-builder.jks for compatible future updates."
echo "Public distribution should use a separately managed signing key."

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
python3 - "$SRC" "$APP" "$TITLE" <<'PY'
from pathlib import Path
import re
import sys

src, app, title = Path(sys.argv[1]), sys.argv[2], sys.argv[3]


def fix(rel: str, old: str, new: str) -> bool:
    """Replace the single `old` in `rel` with `new`, unless it is already gone or already fixed."""
    path = src / rel
    try:
        text = path.read_text()
    except OSError:
        return False
    if old not in text or new in text or text.count(old) != 1:
        return False
    path.write_text(text.replace(old, new, 1))
    return True


def fix_all(rel: str, old: str, new: str) -> bool:
    """Replace all occurrences of `old` in `rel` with `new`, unless it is already gone."""
    path = src / rel
    try:
        text = path.read_text()
    except OSError:
        return False
    if old not in text:
        return False
    path.write_text(text.replace(old, new))
    return True


fixed = []
if app == "photocraft":
    # "load each persisted preset set independently" added `export_presets` to the serialised
    # `Persisted` set and to its write path, but not to the read initialiser, so the crate no
    # longer compiles. Fill in the missing field; the code below it already reads it back.
    if fix(
        "crates/engine/src/presets/mod.rs",
        '            recent_documents: take(&mut m, "recent_documents"),\n',
        '            recent_documents: take(&mut m, "recent_documents"),\n'
        '            export_presets: take(&mut m, "export_presets"),\n',
    ):
        fixed.append("crates/engine/src/presets/mod.rs")
elif app == "cadcraft":
    # cadcraft's own crates/fonts/src/complex.rs was written against harfrust 0.14
    # (FontRef::from_index, ShaperData, UnicodeBuffer::push(u32, u32), shape()), but the
    # workspace pinned harfrust 0.12.0. Bump it to 0.14 so the fonts crate compiles.
    # egui/eframe/egui-wgpu stay at 0.37 (they are path deps to the vendored fork, the
    # "0.37" -> "0.36" patterns below never match path deps and are left as no-ops).
    for rel, old, new in [
        ("Cargo.toml", 'harfrust = "0.12.0"', 'harfrust = "0.12.0"'),
        ("Cargo.toml", 'harfrust = "0.13.3"', 'harfrust = "0.12.0"'),
        ("Cargo.toml", 'harfrust = "0.14.0"', 'harfrust = "0.12.0"'),
        ("Cargo.toml", 'harfrust = "0.15.0"', 'harfrust = "0.12.0"'),
        ("Cargo.toml", 'quick-xml = "0.42"', 'quick-xml = "0.41"'),
        ("crates/engine/Cargo.toml", 'rstar = { version = "0.13", default-features = false }', 'rstar = { version = "0.12", default-features = false }'),
    ]:
        if fix(rel, old, new):
            fixed.append(f"{rel} ({old} -> {new})")

    # Fix rstar 0.12 API (locate_in_envelope_intersecting takes &T::Envelope)
    if fix(
        "crates/engine/src/spatial.rs",
        'self.tree.locate_in_envelope_intersecting(env)',
        'self.tree.locate_in_envelope_intersecting(&env)',
    ):
        fixed.append("crates/engine/src/spatial.rs (rstar API)")

    # Fix quick-xml into_inner() API (returns &[u8], which does not have .as_bytes())
    if fix_all(
        "crates/io/src/svg_read.rs",
        ".into_inner().as_bytes()",
        ".into_inner()",
    ):
        fixed.append("crates/io/src/svg_read.rs (quick-xml into_inner)")

elif app == "wordcraft":
    # rustc 1.99 rejects exhaustive struct patterns that name a field which is private to another
    # module ("pattern requires `..` due to inaccessible fields"). The `swap_fields!` macro
    # destructures `WindowUi` exhaustively by name, so add `..` to keep it compiling against the
    # updated struct. Guarded like the other fixes; a no-op once upstream repairs it.
    if fix(
        "crates/ui-egui/src/window_ui.rs",
        '        let WindowUi { $($(#[$attr])* $field: _),* } = &*$w;',
        '        let WindowUi { $($(#[$attr])* $field: _),*, .. } = &*$w;',
    ):
        fixed.append("crates/ui-egui/src/window_ui.rs (swap_fields pattern)")

    # Ensure system_font_dirs discovers Android system fonts, shared storage Fonts folder,
    # and app-private fonts folders.
    if fix(
        "crates/fonts/src/fontdb/names.rs",
        '        dirs.extend(["/run/host/fonts", "/run/host/local-fonts", "/run/host/user-fonts", "/run/host/user-fonts-legacy"].map(Into::into));',
        '        dirs.extend(["/run/host/fonts", "/run/host/local-fonts", "/run/host/user-fonts", "/run/host/user-fonts-legacy", "/system/fonts", "/product/fonts", "/storage/emulated/0/Fonts", "/sdcard/Fonts"].map(Into::into));\n        if let Some(base) = std::env::var_os("XDG_DATA_HOME").map(std::path::PathBuf::from) {\n            dirs.push(base.join("fonts"));\n        }\n        if let Some(cfg) = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from) {\n            dirs.push(cfg.join("wordcraft/fonts"));\n            dirs.push(cfg.join("fonts"));\n        }',
    ):
        fixed.append("crates/fonts/src/fontdb/names.rs (system and user font directories)")

elif app == "lightcraft":
    # Interactive slider / curve performance on Android:
    # 1. Whole-frame drafts on high-DPI Android screens rendered at full 2560px canvas resolution,
    #    consuming huge memory bandwidth and 80-150ms per draft on mobile GPUs/CPUs. Cap mobile
    #    interactive drafts to 960px (and 0.35 CPU draft scale) for fluid 60-120 FPS slider updates.
    if fix(
        "crates/ui-egui/src/region.rs",
        '/// Size of a slider drag\'s whole-frame draft when full size would be too slow (the CPU renders\n'
        '/// it, or full-size drafts of this drag went over [`DRAFT_BUDGET_MS`]), against the render at rest.\n'
        'pub const CPU_DRAFT_SCALE: f32 = 0.6;\n',
        '/// Size of a slider drag\'s whole-frame draft when full size would be too slow (the CPU renders\n'
        '/// it, or full-size drafts of this drag went over [`DRAFT_BUDGET_MS`]), against the render at rest.\n'
        '#[cfg(target_os = "android")]\n'
        'pub const CPU_DRAFT_SCALE: f32 = 0.35;\n'
        '#[cfg(not(target_os = "android"))]\n'
        'pub const CPU_DRAFT_SCALE: f32 = 0.6;\n'
        '\n'
        '/// The maximum long edge for whole-frame drafts during interactive dragging on mobile.\n'
        '/// Keeps draft rendering within ~2-5 ms on mobile GPUs and drastically reduces memory bandwidth.\n'
        '#[cfg(target_os = "android")]\n'
        'pub const DRAFT_EDGE_MAX: usize = 960;\n'
        '#[cfg(not(target_os = "android"))]\n'
        'pub const DRAFT_EDGE_MAX: usize = usize::MAX;\n',
    ):
        fixed.append("crates/ui-egui/src/region.rs (mobile draft constants)")

    if fix(
        "crates/ui-egui/src/region.rs",
        '    if !v.windows {\n'
        '        // the whole frame at the size it is drawn (a whole-frame diagnostic has no\n'
        '        // window); while a slider drags, a canvas-sized draft as with windows: a drag at 1:1 must\n'
        '        // not render the photo\'s every pixel each frame\n'
        '        let long = if v.draft.is_some() { drawn_long.min(canvas_long) } else { drawn_long };\n'
        '        let main_edge = settings.loupe_edge(long * scale, v.native_long, v.texture_side);\n'
        '        return LoupePlan { main_edge, window_edge: None };\n'
        '    }\n'
        '    // a user who chose a size above the preview source level (3840, 5120) means it; otherwise the\n'
        '    // original is not decoded for the whole-frame render\n'
        '    let source_cap = if settings.preview_limit == 0 { lightcraft_engine::SourceLevel::Preview.max_edge() } else { settings.preview_limit as usize };\n'
        '    let edge_at = |scale: f32| settings.loupe_edge(drawn_long.min(canvas_long) * scale, v.native_long, v.texture_side).min(source_cap);\n'
        '    let frame_edge = settings.window_frame_edge(drawn_long, v.native_long);\n'
        '    // a user who capped the preview size has chosen speed over sharpness at fit\n'
        '    let fit = drawn_long <= canvas_long * 1.05;\n'
        '    // (against the undrafted whole-frame render: a drag must not switch the window off and on)\n'
        '    let window_edge = (frame_edge as f32 > edge_at(1.0) as f32 * WINDOW_RATIO && !(fit && settings.preview_limit != 0)).then_some(frame_edge);\n'
        '    // with a window over it, the whole-frame draft is hidden (it feeds the histogram and stands in\n'
        '    // where the window has no pixels yet): the small draft does, and the window keeps the frame time\n'
        '    let scale = if window_edge.is_some() && v.draft.is_some() { scale.min(CPU_DRAFT_SCALE) } else { scale };\n'
        '    LoupePlan { main_edge: edge_at(scale), window_edge }',
        '    if !v.windows {\n'
        '        // the whole frame at the size it is drawn (a whole-frame diagnostic has no\n'
        '        // window); while a slider drags, a canvas-sized draft as with windows: a drag at 1:1 must\n'
        '        // not render the photo\'s every pixel each frame\n'
        '        let long = if v.draft.is_some() { drawn_long.min(canvas_long) } else { drawn_long };\n'
        '        let mut main_edge = settings.loupe_edge(long * scale, v.native_long, v.texture_side);\n'
        '        if v.draft.is_some() {\n'
        '            main_edge = main_edge.min(DRAFT_EDGE_MAX);\n'
        '        }\n'
        '        return LoupePlan { main_edge, window_edge: None };\n'
        '    }\n'
        '    // a user who chose a size above the preview source level (3840, 5120) means it; otherwise the\n'
        '    // original is not decoded for the whole-frame render\n'
        '    let source_cap = if settings.preview_limit == 0 { lightcraft_engine::SourceLevel::Preview.max_edge() } else { settings.preview_limit as usize };\n'
        '    let edge_at = |scale: f32| settings.loupe_edge(drawn_long.min(canvas_long) * scale, v.native_long, v.texture_side).min(source_cap);\n'
        '    let frame_edge = settings.window_frame_edge(drawn_long, v.native_long);\n'
        '    // a user who capped the preview size has chosen speed over sharpness at fit\n'
        '    let fit = drawn_long <= canvas_long * 1.05;\n'
        '    // (against the undrafted whole-frame render: a drag must not switch the window off and on)\n'
        '    let window_edge = (frame_edge as f32 > edge_at(1.0) as f32 * WINDOW_RATIO && !(fit && settings.preview_limit != 0)).then_some(frame_edge);\n'
        '    // with a window over it, the whole-frame draft is hidden (it feeds the histogram and stands in\n'
        '    // where the window has no pixels yet): the small draft does, and the window keeps the frame time\n'
        '    let scale = if window_edge.is_some() && v.draft.is_some() { scale.min(CPU_DRAFT_SCALE) } else { scale };\n'
        '    let mut main_edge = edge_at(scale);\n'
        '    if v.draft.is_some() {\n'
        '        main_edge = main_edge.min(DRAFT_EDGE_MAX);\n'
        '    }\n'
        '    LoupePlan { main_edge, window_edge }',
    ):
        fixed.append("crates/ui-egui/src/region.rs (clamp mobile draft to DRAFT_EDGE_MAX)")

    # 2. `wait_for_draft` lockout: previous render at rest (25-50ms) made `last_main_ms * 1.5 >= budget`
    #    fail on the first draft frame, so wait_for_draft never waited during drags. On Android, wait
    #    when GPU is available so fast 960px drafts show in the very same frame without backlog lag.
    if fix(
        "crates/ui-egui/src/render.rs",
        '    #[cfg(not(target_arch = "wasm32"))]\n'
        '    pub fn wait_for_draft(&mut self, ctx: &egui::Context, session: &mut Session, budget: std::time::Duration) {\n'
        '        if self.offload.is_some() || self.last_main_ms * 1.5 >= budget.as_secs_f64() * 1e3 {\n'
        '            return;\n'
        '        }',
        '    #[cfg(not(target_arch = "wasm32"))]\n'
        '    pub fn wait_for_draft(&mut self, ctx: &egui::Context, session: &mut Session, budget: std::time::Duration) {\n'
        '        #[cfg(target_os = "android")]\n'
        '        let skip_wait = self.offload.is_some() || (!lightcraft_engine::gpu::available() && self.last_main_ms > 80.0);\n'
        '        #[cfg(not(target_os = "android"))]\n'
        '        let skip_wait = self.offload.is_some() || self.last_main_ms * 1.5 >= budget.as_secs_f64() * 1e3;\n'
        '        if skip_wait {\n'
        '            return;\n'
        '        }',
    ):
        fixed.append("crates/ui-egui/src/render.rs (wait_for_draft Android GPU support)")

    # 3. Provide sufficient budget (~10ms) on Android for fast drafts to complete synchronously.
    if fix(
        "crates/ui-egui/src/panels/detail.rs",
        '        #[cfg(not(target_arch = "wasm32"))]\n'
        '        if interacting && whole_draft {\n'
        '            let frame = ui.input(|i| i.predicted_dt).clamp(1.0 / 240.0, 1.0 / 30.0);\n'
        '            // (a frame time that is not a number waits for nothing)\n'
        '            let budget = std::time::Duration::try_from_secs_f32(frame * 0.4).unwrap_or_default();\n'
        '            app.renderer.wait_for_draft(ui.ctx(), &mut app.session, budget);\n'
        '        }',
        '        #[cfg(not(target_arch = "wasm32"))]\n'
        '        if interacting && whole_draft {\n'
        '            let frame = ui.input(|i| i.predicted_dt).clamp(1.0 / 240.0, 1.0 / 30.0);\n'
        '            // (a frame time that is not a number waits for nothing)\n'
        '            #[cfg(target_os = "android")]\n'
        '            let budget = std::time::Duration::try_from_secs_f32(frame.max(1.0 / 60.0) * 0.6).unwrap_or(std::time::Duration::from_millis(10));\n'
        '            #[cfg(not(target_os = "android"))]\n'
        '            let budget = std::time::Duration::try_from_secs_f32(frame * 0.4).unwrap_or_default();\n'
        '            app.renderer.wait_for_draft(ui.ctx(), &mut app.session, budget);\n'
        '        }',
    ):
        fixed.append("crates/ui-egui/src/panels/detail.rs (Android wait_for_draft budget)")

elif app == "filmcraft":
    # 0. Allow unsafe code in workspace for hardware surface interop.
    if fix(
        "Cargo.toml",
        'unsafe_code = "forbid"\n',
        'unsafe_code = "allow"\n',
    ):
        fixed.append("Cargo.toml (unsafe_code = allow)")

    # 1. Add ndk dependency to crates/platform/Cargo.toml for AMediaCodec.
    if fix(
        "crates/platform/Cargo.toml",
        '[target.\'cfg(target_os = "linux")\'.dependencies]\nlibloading = "0.8"\n',
        '[target.\'cfg(target_os = "linux")\'.dependencies]\nlibloading = "0.8"\n\n'
        '# Android: hardware H.264 video encoding through Android\'s NDK MediaCodec API (libmediandk).\n'
        '[target.\'cfg(target_os = "android")\'.dependencies]\nndk = { version = "0.9", features = ["media", "nativewindow", "api-level-28", "rwh_06"], default-features = false }\nndk-sys = "0.6"\njni = "0.22.4"\nndk-context = "0.1.1"\n',
    ):
        fixed.append("crates/platform/Cargo.toml (ndk dependency)")

    # 1a. Allow unsafe code in crates/platform/Cargo.toml for NEON SIMD optimizations.
    if fix(
        "crates/platform/Cargo.toml",
        '[lints.rust]\nunsafe_code = "deny"\n',
        '[lints.rust]\nunsafe_code = "allow"\n',
    ):
        fixed.append("crates/platform/Cargo.toml (unsafe_code = allow)")

    # 1b. Add log dependency to crates/export/Cargo.toml for export logging.
    if fix(
        "crates/export/Cargo.toml",
        'rayon = { workspace = true }\n',
        'rayon = { workspace = true }\nlog = { workspace = true }\n',
    ):
        fixed.append("crates/export/Cargo.toml (log dependency)")

    # 2. Add android_mediacodec module declaration in crates/platform/src/lib.rs.
    if fix(
        "crates/platform/src/lib.rs",
        'mod annexb;\n',
        'mod annexb;\n'
        '#[cfg(target_os = "android")]\n'
        'pub mod android_mediacodec;\n',
    ):
        fixed.append("crates/platform/src/lib.rs (android_mediacodec module)")

    # 3. Register android_mediacodec factory in crates/platform/src/lib.rs.
    if fix(
        "crates/platform/src/lib.rs",
        '    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]\n'
        '    {\n'
        '        Availability::Unavailable("no hardware video decoder for this system yet")\n'
        '    }\n'
        '}\n'
        '\n'
        '/// Whether [`register`] has put a hardware decoder factory in front of our decoders.\n'
        'pub fn registered() -> bool {\n'
        '    #[cfg(target_os = "macos")]\n'
        '    {\n'
        '        filmcraft_codecs::video_decoder_registered(videotoolbox_factory)\n'
        '    }\n'
        '    #[cfg(target_os = "windows")]\n'
        '    {\n'
        '        filmcraft_codecs::video_decoder_registered(media_foundation_factory)\n'
        '    }\n'
        '    #[cfg(target_os = "linux")]\n'
        '    {\n'
        '        #[cfg(target_pointer_width = "64")]\n'
        '        if filmcraft_codecs::video_decoder_registered(nvdec_factory) {\n'
        '            return true;\n'
        '        }\n'
        '        filmcraft_codecs::video_decoder_registered(vaapi_factory)\n'
        '    }\n'
        '    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]\n',
        '    #[cfg(target_os = "android")]\n'
        '    {\n'
        '        static ONCE: std::sync::Once = std::sync::Once::new();\n'
        '        ONCE.call_once(|| {\n'
        '            filmcraft_codecs::register_video_decoder(android_mediacodec::decoder_factory);\n'
        '            filmcraft_export::register_encoder(android_mediacodec::factory);\n'
        '            filmcraft_codecs::hw::set_hw_backend("MediaCodec");\n'
        '            android_mediacodec::register_scaler();\n'
        '        });\n'
        '        Availability::Available("MediaCodec")\n'
        '    }\n'
        '    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux", target_os = "android")))]\n'
        '    {\n'
        '        Availability::Unavailable("no hardware video decoder for this system yet")\n'
        '    }\n'
        '}\n'
        '\n'
        '/// Whether [`register`] has put a hardware decoder factory in front of our decoders.\n'
        'pub fn registered() -> bool {\n'
        '    #[cfg(target_os = "macos")]\n'
        '    {\n'
        '        filmcraft_codecs::video_decoder_registered(videotoolbox_factory)\n'
        '    }\n'
        '    #[cfg(target_os = "windows")]\n'
        '    {\n'
        '        filmcraft_codecs::video_decoder_registered(media_foundation_factory)\n'
        '    }\n'
        '    #[cfg(target_os = "linux")]\n'
        '    {\n'
        '        #[cfg(target_pointer_width = "64")]\n'
        '        if filmcraft_codecs::video_decoder_registered(nvdec_factory) {\n'
        '            return true;\n'
        '        }\n'
        '        filmcraft_codecs::video_decoder_registered(vaapi_factory)\n'
        '    }\n'
        '    #[cfg(target_os = "android")]\n'
        '    {\n'
        '        filmcraft_codecs::video_decoder_registered(android_mediacodec::decoder_factory)\n'
        '            && filmcraft_export::encoder_registered(android_mediacodec::factory)\n'
        '    }\n'
        '    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux", target_os = "android")))]\n',
    ):
        fixed.append("crates/platform/src/lib.rs (MediaCodec registration)")

    # 3b. Support hardware_decoder_for on Android in crates/platform/src/lib.rs.
    if fix(
        "crates/platform/src/lib.rs",
        '    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]\n'
        '    {\n'
        '        let _ = entry;\n'
        '        false\n'
        '    }\n'
        '}\n',
        '    #[cfg(target_os = "android")]\n'
        '    {\n'
        '        filmcraft_codecs::hw::NalStreamInfo::from_entry(entry).and_then(|r| r.ok()).is_some_and(|info| {\n'
        '            android_mediacodec::decoder_for(&info)\n'
        '        })\n'
        '    }\n'
        '    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux", target_os = "android")))]\n'
        '    {\n'
        '        let _ = entry;\n'
        '        false\n'
        '    }\n'
        '}\n',
    ):
        fixed.append("crates/platform/src/lib.rs (MediaCodec decoder_for)")

    # 3c. Support direct surface bridge in crates/codecs/src/hw.rs.
    if fix(
        "crates/codecs/src/hw.rs",
        'static ENABLED: AtomicBool = AtomicBool::new(true);\n',
        'static ENABLED: AtomicBool = AtomicBool::new(true);\n\n'
        'static DIRECT_SURFACE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);\n\n'
        'static DIRECT_SURFACE_DIMS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);\n\n'
        'pub fn set_direct_surface(surface: *mut std::ffi::c_void, w: u32, h: u32) {\n'
        '    let dims = ((w as u64) << 32) | (h as u64);\n'
        '    DIRECT_SURFACE_DIMS.store(dims, Ordering::SeqCst);\n'
        '    DIRECT_SURFACE.store(surface as usize, Ordering::SeqCst);\n'
        '}\n\n'
        'pub fn clear_direct_surface() {\n'
        '    DIRECT_SURFACE.store(0, Ordering::SeqCst);\n'
        '    DIRECT_SURFACE_DIMS.store(0, Ordering::SeqCst);\n'
        '    clear_surface_scaler();\n'
        '}\n\n'
        'pub fn has_direct_surface() -> bool {\n'
        '    DIRECT_SURFACE.load(Ordering::SeqCst) != 0\n'
        '}\n\n'
        'pub fn direct_surface() -> Option<*mut std::ffi::c_void> {\n'
        '    let p = DIRECT_SURFACE.load(Ordering::SeqCst);\n'
        '    if p != 0 {\n'
        '        Some(p as *mut std::ffi::c_void)\n'
        '    } else {\n'
        '        None\n'
        '    }\n'
        '}\n\n'
        'pub fn direct_surface_for(w: u32, h: u32) -> Option<*mut std::ffi::c_void> {\n'
        '    let dims = DIRECT_SURFACE_DIMS.load(Ordering::SeqCst);\n'
        '    let expected = ((w as u64) << 32) | (h as u64);\n'
        '    if dims != 0 && dims == expected {\n'
        '        direct_surface()\n'
        '    } else {\n'
        '        None\n'
        '    }\n'
        '}\n\n'
        'pub type ScalerInitFn = fn(*mut std::ffi::c_void, u32, u32, u32, u32) -> Option<*mut std::ffi::c_void>;\n'
        'pub type ScalerBlitFn = fn() -> bool;\n'
        'pub type ScalerCheckFn = fn() -> bool;\n'
        'pub type ScalerClearFn = fn();\n\n'
        'static SCALER_INIT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);\n'
        'static SCALER_BLIT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);\n'
        'static SCALER_CHECK: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);\n'
        'static SCALER_CLEAR: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);\n\n'
        'pub fn register_surface_scaler(init: ScalerInitFn, blit: ScalerBlitFn, check: ScalerCheckFn, clear: ScalerClearFn) {\n'
        '    SCALER_INIT.store(init as usize, Ordering::SeqCst);\n'
        '    SCALER_BLIT.store(blit as usize, Ordering::SeqCst);\n'
        '    SCALER_CHECK.store(check as usize, Ordering::SeqCst);\n'
        '    SCALER_CLEAR.store(clear as usize, Ordering::SeqCst);\n'
        '}\n\n'
        'pub fn init_surface_scaler(window: *mut std::ffi::c_void, src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Option<*mut std::ffi::c_void> {\n'
        '    let p = SCALER_INIT.load(Ordering::SeqCst);\n'
        '    if p != 0 {\n'
        '        let f: ScalerInitFn = unsafe { std::mem::transmute(p) };\n'
        '        f(window, src_w, src_h, dst_w, dst_h)\n'
        '    } else {\n'
        '        None\n'
        '    }\n'
        '}\n\n'
        'pub fn blit_surface_scaler() -> bool {\n'
        '    let p = SCALER_BLIT.load(Ordering::SeqCst);\n'
        '    if p != 0 {\n'
        '        let f: ScalerBlitFn = unsafe { std::mem::transmute(p) };\n'
        '        f()\n'
        '    } else {\n'
        '        false\n'
        '    }\n'
        '}\n\n'
        'pub fn has_surface_scaler() -> bool {\n'
        '    let p = SCALER_CHECK.load(Ordering::SeqCst);\n'
        '    if p != 0 {\n'
        '        let f: ScalerCheckFn = unsafe { std::mem::transmute(p) };\n'
        '        f()\n'
        '    } else {\n'
        '        false\n'
        '    }\n'
        '}\n\n'
        'pub fn clear_surface_scaler() {\n'
        '    let p = SCALER_CLEAR.load(Ordering::SeqCst);\n'
        '    if p != 0 {\n'
        '        let f: ScalerClearFn = unsafe { std::mem::transmute(p) };\n'
        '        f();\n'
        '    }\n'
        '}\n',
    ):
        fixed.append("crates/codecs/src/hw.rs (direct surface bridge tracking)")

    # 4. Copy android_mediacodec.rs template to checkout.
    mediacodec_target = src / "crates/platform/src/android_mediacodec.rs"
    template_file = Path.cwd() / "templates/filmcraft/android_mediacodec.rs"
    if template_file.exists():
        content = template_file.read_text()
        if not mediacodec_target.exists() or mediacodec_target.read_text() != content:
            mediacodec_target.write_text(content)
            fixed.append("crates/platform/src/android_mediacodec.rs (updated from template)")

    # 5. Show hardware encoding status in export completion message and write export report log.
    if fix(
        "crates/export/src/job.rs",
        '            progress.set_status(format!("Done in {secs:.1}s"));\n',
        '            let enc = crate::hw_encode_stats();\n'
        '            let dec = filmcraft_codecs::hw::hw_stats();\n'
        '            let gop = filmcraft_codecs::gop_stats();\n'
        '            let gpu = crate::gpu_render_stats();\n'
        '            let hw_info = if gop.frames == 0 {\n'
        '                format!(" (Still/Gen, GPU: {}, Enc: {} HW)", gpu.frames, enc.frames)\n'
        '            } else {\n'
        '                format!(" (Dec: {}/{} HW, GPU: {}, Enc: {} HW)", dec.frames, gop.frames, gpu.frames, enc.frames)\n'
        '            };\n'
        '            let st = format!("Done in {secs:.1}s{hw_info}");\n'
        '            progress.set_status(st.clone());\n'
        '            let log_path = format!("{}.log", self.settings.path);\n'
        '            let mut diag = String::new();\n'
        '            diag.push_str("=== FILMCRAFT EXPORT REPORT ===\\n");\n'
        '            diag.push_str(&format!("Output: {}\\n", self.settings.path));\n'
        '            diag.push_str(&format!("Status: {st}\\n"));\n'
        '            diag.push_str(&format!("Frames: {nframes}\\n"));\n'
        '            diag.push_str(&format!("Duration: {secs:.2}s\\n"));\n'
        '            diag.push_str(&format!("Render FPS: {:.2}\\n", nframes as f64 / secs.max(1e-6)));\n'
        '            diag.push_str(&format!("File size: {bytes} bytes\\n\\n"));\n'
        '            diag.push_str("--- Pipeline Statistics ---\\n");\n'
        '            diag.push_str(&format!("GOP total decoded pictures: {}\\n", gop.frames));\n'
        '            diag.push_str(&format!("Hardware decoded pictures: {}\\n", dec.frames));\n'
        '            diag.push_str(&format!("Software decoded pictures: {}\\n", gop.frames.saturating_sub(dec.frames)));\n'
        '            diag.push_str(&format!("Hardware decoder sessions: {}, declined: {}, fallbacks: {}\\n", dec.sessions, dec.declined, dec.fallbacks));\n'
        '            diag.push_str(&format!("GPU composited frames: {}\\n", gpu.frames));\n'
        '            diag.push_str(&format!("Hardware encoded frames: {}\\n\\n", enc.frames));\n'
        '            diag.push_str("--- Stage Timings ---\\n");\n'
        '            for (stage, ns) in crate::stage_times() {\n'
        '                let ms = ns as f64 / 1_000_000.0;\n'
        '                diag.push_str(&format!("  {:?}: {:.1} ms ({:.2}s)\\n", stage, ms, ms / 1000.0));\n'
        '            }\n'
        '            log::info!("{diag}");\n'
        '            let _ = std::fs::write(&log_path, &diag);\n',
    ):
        fixed.append("crates/export/src/job.rs (export status HW indicator and log)")

    # 6. Display export status on the on-screen status bar when complete and show live progress.
    if fix(
        "crates/ui-egui/src/app/frame.rs",
        '        let job = running?;\n',
        '        for j in self.session.jobs.iter().rev() {\n'
        '            if j.progress.finished.load(Ordering::Relaxed) {\n'
        '                let st = j.progress.status.lock().unwrap_or_else(|e| e.into_inner()).clone();\n'
        '                if (st.starts_with("Done in ") || st.starts_with("Export failed")) && self.status_seen.0 != st {\n'
        '                    log::info!("Export complete: {st}");\n'
        '                    self.ui.status = st.clone();\n'
        '                    self.status_seen = (st, ctx.input(|i| i.time));\n'
        '                    ctx.request_repaint();\n'
        '                }\n'
        '                break;\n'
        '            }\n'
        '        }\n'
        '        let job = running?;\n'
        '        let done = job.progress.done.load(Ordering::Relaxed);\n'
        '        let total = job.progress.total.load(Ordering::Relaxed);\n'
        '        if total > 0 && !job.progress.finished.load(Ordering::Relaxed) {\n'
        '            let pct = (done as f32 / total as f32 * 100.0).clamp(0.0, 100.0);\n'
        '            self.ui.status = format!("Exporting: {done}/{total} frames ({pct:.0}%) [GPU Zero-Copy Enc]");\n'
        '        }\n',
    ):
        fixed.append("crates/ui-egui/src/app/frame.rs (export completion status and progress in UI)")

    # 7. Add diagnostics for make_video_decoder in crates/codecs/src/lib.rs.
    if fix(
        "crates/codecs/src/lib.rs",
        'pub fn make_video_decoder(entry: &filmcraft_isobmff::SampleEntry) -> Result<Box<dyn VideoDecoder>> {\n',
        'pub fn make_video_decoder(entry: &filmcraft_isobmff::SampleEntry) -> Result<Box<dyn VideoDecoder>> {\n'
        '    log::info!("make_video_decoder called for codec: {}", entry.codec.name());\n',
    ):
        fixed.append("crates/codecs/src/lib.rs (make_video_decoder diagnostic logging)")

    # 8. Single GPU frame renderer on Android to conserve unified memory and avoid TBDR context contention.
    if fix(
        "crates/export/src/pipeline.rs",
        '            let count = rayon::current_num_threads().clamp(2, 4);\n',
        '            let count = if cfg!(target_os = "android") { 1 } else { rayon::current_num_threads().clamp(2, 4) };\n',
    ):
        fixed.append("crates/export/src/pipeline.rs (bound GPU renderers on Android)")

    # 9. Bound GPU texture upload cache on Android and recycle textures into plane_pool.
    if fix(
        "crates/gpu/src/lib.rs",
        '        if self.uploads.len() > 24 {\n'
        '            let mut v: Vec<(u64, (usize, u32, u32))> = self.uploads.iter().map(|(k, u)| (u.last_used, *k)).collect();\n'
        '            v.sort_unstable();\n'
        '            for (_, k) in v.into_iter().take(self.uploads.len() - 24) {\n'
        '                self.uploads.remove(&k);\n'
        '            }\n'
        '        }\n',
        '        let max_uploads = if cfg!(target_os = "android") { 4 } else { 24 };\n'
        '        if self.uploads.len() > max_uploads {\n'
        '            let mut v: Vec<(u64, (usize, u32, u32))> = self.uploads.iter().map(|(k, u)| (u.last_used, *k)).collect();\n'
        '            v.sort_unstable();\n'
        '            for (_, k) in v.into_iter().take(self.uploads.len() - max_uploads) {\n'
        '                if let Some(removed) = self.uploads.remove(&k) {\n'
        '                    for (opt, view) in removed.textures.into_iter().zip(removed.views.into_iter()) {\n'
        '                        if let Some((w, h, fmt, tex)) = opt {\n'
        '                            if self.plane_pool.len() < 32 {\n'
        '                                self.plane_pool.push((w, h, fmt, tex, view));\n'
        '                            }\n'
        '                        }\n'
        '                    }\n'
        '                }\n'
        '            }\n'
        '        }\n',
    ):
        fixed.append("crates/gpu/src/lib.rs (bound texture upload cache on Android and recycle textures into plane_pool)")

    # 10. Bound in-flight export memory budget and batch size on Android to prevent OOM/LMK during 4K exports.
    if fix(
        "crates/export/src/job.rs",
        'pub const IN_FLIGHT_BUDGET: u64 = 1 << 29;\n',
        'pub const IN_FLIGHT_BUDGET: u64 = if cfg!(target_os = "android") { 1 << 26 } else { 1 << 29 };\n',
    ):
        fixed.append("crates/export/src/job.rs (bound in-flight memory budget on Android)")

    if fix(
        "crates/export/src/job.rs",
        '            batch: rayon::current_num_threads().clamp(2, 16) as i64,\n',
        '            batch: if cfg!(target_os = "android") { 2 } else { rayon::current_num_threads().clamp(2, 16) as i64 },\n',
    ):
        fixed.append("crates/export/src/job.rs (bound default batch size on Android)")

    # 11. Bound GOP decoded frame cache ceiling on Android (96 MiB instead of 384 MiB).
    if fix(
        "crates/codecs/src/gop.rs",
        'const FRAME_CACHE_CEILING: usize = 384 << 20;\n',
        'const FRAME_CACHE_CEILING: usize = if cfg!(target_os = "android") { 96 << 20 } else { 384 << 20 };\n',
    ):
        fixed.append("crates/codecs/src/gop.rs (bound GOP frame cache ceiling on Android)")

    # 12. Fast contiguous luma and ARM64 NEON SIMD chroma de-interleave in biplanar::to_frame.
    if fix(
        "crates/platform/src/biplanar.rs",
        'fn row(plane: &[u8], stride: usize, y: usize, x: usize, len: usize) -> Result<&[u8], String> {\n'
        '    let start = y.checked_mul(stride).and_then(|s| s.checked_add(x)).ok_or("plane offset overflows")?;\n'
        '    plane.get(start..start.checked_add(len).ok_or("plane offset overflows")?).ok_or_else(|| "plane row out of range".to_string())\n'
        '}\n',
        'fn row(plane: &[u8], stride: usize, y: usize, x: usize, len: usize) -> Result<&[u8], String> {\n'
        '    let start = y.checked_mul(stride).and_then(|s| s.checked_add(x)).ok_or("plane offset overflows")?;\n'
        '    plane.get(start..start.checked_add(len).ok_or("plane offset overflows")?).ok_or_else(|| "plane row out of range".to_string())\n'
        '}\n\n'
        '#[allow(unsafe_code)]\n'
        '#[inline(always)]\n'
        'fn deinterleave_uv(pairs: &[u8], u: &mut [u8], v: &mut [u8]) {\n'
        '    let mut i = 0;\n'
        '    let mut out = 0;\n'
        '    let len = u.len().min(v.len()).min(pairs.len() / 2);\n'
        '    #[cfg(target_arch = "aarch64")]\n'
        '    // SAFETY: out + 16 <= len <= pairs.len() / 2 ensures i + 32 <= pairs.len(), and out + 16 <= u.len() and v.len(). All pointers are within bounds.\n'
        '    unsafe {\n'
        '        while out + 16 <= len {\n'
        '            let uv = std::arch::aarch64::vld2q_u8(pairs.as_ptr().add(i));\n'
        '            std::arch::aarch64::vst1q_u8(u.as_mut_ptr().add(out), uv.0);\n'
        '            std::arch::aarch64::vst1q_u8(v.as_mut_ptr().add(out), uv.1);\n'
        '            i += 32;\n'
        '            out += 16;\n'
        '        }\n'
        '    }\n'
        '    while out < len {\n'
        '        if let (Some(&u_val), Some(&v_val)) = (pairs.get(i), pairs.get(i + 1)) {\n'
        '            if let Some(u_slot) = u.get_mut(out) {\n'
        '                *u_slot = u_val;\n'
        '            }\n'
        '            if let Some(v_slot) = v.get_mut(out) {\n'
        '                *v_slot = v_val;\n'
        '            }\n'
        '        }\n'
        '        i += 2;\n'
        '        out += 1;\n'
        '    }\n'
        '}\n',
    ):
        fixed.append("crates/platform/src/biplanar.rs (NEON deinterleave_uv helper)")

    if fix(
        "crates/platform/src/biplanar.rs",
        '    let data = if bps == 1 {\n'
        '        let mut yp = pool::take_u8(w * h);\n'
        '        for y in 0..h {\n'
        '            yp.extend_from_slice(row(src.luma, src.stride, oy + y, ox, w)?);\n'
        '        }\n'
        '        let (mut u, mut v) = (pool::take_u8(cw * ch), pool::take_u8(cw * ch));\n'
        '        for y in 0..ch {\n'
        '            // two passes over the row: each is a simple strided copy the compiler vectorises\n'
        '            let pairs = row(src.chroma, src.stride, coy + y, cox * 2, cw * 2)?.as_chunks::<2>().0;\n'
        '            u.extend(pairs.iter().map(|p| p[0]));\n'
        '            v.extend(pairs.iter().map(|p| p[1]));\n'
        '        }\n'
        '        PixelData::Yuv8 { planes: [Arc::new(yp), Arc::new(u), Arc::new(v)], chroma: Chroma::C420, alpha: None }\n',
        '    let data = if bps == 1 {\n'
        '        let mut yp = pool::take_u8(w * h);\n'
        '        if ox == 0 && oy == 0 && src.stride == w && src.rows >= h {\n'
        '            if let Some(luma_slice) = src.luma.get(..w * h) {\n'
        '                yp.extend_from_slice(luma_slice);\n'
        '            }\n'
        '        }\n'
        '        if yp.len() < w * h {\n'
        '            yp.clear();\n'
        '            for y in 0..h {\n'
        '                yp.extend_from_slice(row(src.luma, src.stride, oy + y, ox, w)?);\n'
        '            }\n'
        '        }\n'
        '        let mut u = pool::take_u8(cw * ch);\n'
        '        let mut v = pool::take_u8(cw * ch);\n'
        '        u.resize(cw * ch, 0);\n'
        '        v.resize(cw * ch, 0);\n'
        '        let total_chroma_needed = cw.saturating_mul(ch).saturating_mul(2);\n'
        '        let mut done = false;\n'
        '        if cox == 0 && coy == 0 && src.stride == w && src.rows >= h {\n'
        '            if let Some(chroma_slice) = src.chroma.get(..total_chroma_needed) {\n'
        '                deinterleave_uv(chroma_slice, &mut u, &mut v);\n'
        '                done = true;\n'
        '            }\n'
        '        }\n'
        '        if !done {\n'
        '            for y in 0..ch {\n'
        '                let row_bytes = row(src.chroma, src.stride, coy + y, cox * 2, cw * 2)?;\n'
        '                let row_start = y * cw;\n'
        '                let row_end = row_start + cw;\n'
        '                if let (Some(row_u), Some(row_v)) = (u.get_mut(row_start..row_end), v.get_mut(row_start..row_end)) {\n'
        '                    deinterleave_uv(row_bytes, row_u, row_v);\n'
        '                }\n'
        '            }\n'
        '        }\n'
        '        PixelData::Yuv8 { planes: [Arc::new(yp), Arc::new(u), Arc::new(v)], chroma: Chroma::C420, alpha: None }\n',
    ):
        fixed.append("crates/platform/src/biplanar.rs (fast contiguous luma and NEON SIMD chroma copy)")

    # 13. Support input_surface and surface rendering in VideoEncoder and FrameRenderer.
    if fix(
        "crates/export/src/registry.rs",
        '    fn prepare_ahead(&self) -> Option<PrepareAhead> {\n'
        '        None\n'
        '    }\n'
        '}\n',
        '    fn prepare_ahead(&self) -> Option<PrepareAhead> {\n'
        '        None\n'
        '    }\n'
        '    /// On platforms supporting direct GPU-to-hardware-encoder surface rendering (Android AMediaCodec),\n'
        '    /// returns the native window pointer (*mut std::ffi::c_void) of the input surface.\n'
        '    fn input_surface(&self) -> Option<*mut std::ffi::c_void> {\n'
        '        None\n'
        '    }\n'
        '    /// Switch from surface mode to buffer mode if surface binding fails.\n'
        '    fn fallback_to_buffer(&mut self) -> bool {\n'
        '        false\n'
        '    }\n'
        '}\n',
    ):
        fixed.append("crates/export/src/registry.rs (VideoEncoder input_surface)")

    if fix(
        "crates/export/src/registry.rs",
        '    fn caps(&self) -> filmcraft_render::plan::DeviceCaps;\n'
        '}\n',
        '    fn caps(&self) -> filmcraft_render::plan::DeviceCaps;\n'
        '    fn bind_surface(&mut self, _window: *mut std::ffi::c_void, _w: u32, _h: u32) -> bool {\n'
        '        false\n'
        '    }\n'
        '    fn render_to_surface(\n'
        '        &mut self,\n'
        '        _project: &Project,\n'
        '        _seq: ItemId,\n'
        '        _t: Tick,\n'
        '        _opts: filmcraft_render::RenderOptions,\n'
        '        _sources: &dyn SourceProvider,\n'
        '    ) -> Option<bool> {\n'
        '        None\n'
        '    }\n'
        '    fn render_plan_to_surface(&mut self, _plan: &filmcraft_render::plan::FramePlan) -> Option<bool> {\n'
        '        None\n'
        '    }\n'
        '    fn render_rgba8(\n'
        '        &mut self,\n'
        '        _project: &Project,\n'
        '        _seq: ItemId,\n'
        '        _t: Tick,\n'
        '        _opts: filmcraft_render::RenderOptions,\n'
        '        _sources: &dyn SourceProvider,\n'
        '    ) -> Option<Vec<u8>> {\n'
        '        None\n'
        '    }\n'
        '}\n',
    ):
        fixed.append("crates/export/src/registry.rs (FrameRenderer surface and rgba8 methods)")

    # 13.5 Add fs_linear to composite.wgsl for direct hardware sRGB surface rendering.
    if fix(
        "crates/gpu/src/composite.wgsl",
        '    return vec4(select(hi, lo, c <= vec3(0.0031308)), 1.0);\n'
        '}\n',
        '    return vec4(select(hi, lo, c <= vec3(0.0031308)), 1.0);\n'
        '}\n\n'
        '@fragment\n'
        'fn fs_linear(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {\n'
        '    let c = clamp(textureLoad(accum, vec2<i32>(pos.xy), 0).rgb, vec3(0.0), vec3(1.0));\n'
        '    return vec4(c, 1.0);\n'
        '}\n',
    ):
        fixed.append("crates/gpu/src/composite.wgsl (fs_linear for sRGB surface)")

    # 14. Add render_to_view and render_export_rgba8 on GpuCompositor.
    if fix(
        "crates/gpu/src/lib.rs",
        '    accum_readback_buf: Option<(wgpu::Buffer, u64)>,\n',
        '    accum_readback_buf: Option<(wgpu::Buffer, u64)>,\n'
        '    output_readback_buf: Option<(wgpu::Buffer, u64)>,\n'
        '    surface_pipeline: Option<(wgpu::TextureFormat, wgpu::RenderPipeline)>,\n'
        '    plane_pool: Vec<(u32, u32, wgpu::TextureFormat, wgpu::Texture, wgpu::TextureView)>,\n',
    ):
        fixed.append("crates/gpu/src/lib.rs (GpuCompositor output_readback_buf and surface_pipeline fields)")

    if fix(
        "crates/gpu/src/lib.rs",
        '            accum_readback_buf: None,\n',
        '            accum_readback_buf: None,\n'
        '            output_readback_buf: None,\n'
        '            surface_pipeline: None,\n'
        '            plane_pool: Vec::new(),\n',
    ):
        fixed.append("crates/gpu/src/lib.rs (GpuCompositor output_readback_buf and surface_pipeline init)")

    if fix(
        "crates/gpu/src/lib.rs",
        '    pub fn render_export_prepared(&mut self, plan: &FramePlan, prep: Option<&PreparedPlan>) -> Option<(u32, u32, Vec<f32>)> {\n',
        '    /// Direct GPU render into a target view (surface texture) using format-matched pipeline.\n'
        '    pub fn render_to_view(&mut self, plan: &FramePlan, prep: Option<&PreparedPlan>, target_view: &wgpu::TextureView, target_format: wgpu::TextureFormat) -> Option<()> {\n'
        '        let t = std::time::Instant::now();\n'
        '        let (accum_view, _) = self.render_plan(plan, prep, true, false);\n'
        '        let final_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {\n'
        '            label: Some("final-to-view"),\n'
        '            layout: &self.final_bgl,\n'
        '            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&accum_view) }],\n'
        '        });\n'
        '        if target_format != OUTPUT_FORMAT\n'
        '            && self.surface_pipeline.as_ref().map_or(true, |(fmt, _)| *fmt != target_format)\n'
        '        {\n'
        '            let shader = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {\n'
        '                label: Some("composite-surface"),\n'
        '                source: wgpu::ShaderSource::Wgsl(include_str!("composite.wgsl").into()),\n'
        '            });\n'
        '            let fpl = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {\n'
        '                label: Some("final-surface"),\n'
        '                bind_group_layouts: &[Some(&self.final_bgl)],\n'
        '                immediate_size: 0,\n'
        '            });\n'
        '            let entry_point = if target_format.is_srgb() { "fs_linear" } else { "fs_full" };\n'
        '            let p = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {\n'
        '                label: Some("final-surface"),\n'
        '                layout: Some(&fpl),\n'
        '                vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },\n'
        '                primitive: wgpu::PrimitiveState::default(),\n'
        '                depth_stencil: None,\n'
        '                multisample: wgpu::MultisampleState::default(),\n'
        '                fragment: Some(wgpu::FragmentState {\n'
        '                    module: &shader,\n'
        '                    entry_point: Some(entry_point),\n'
        '                    compilation_options: Default::default(),\n'
        '                    targets: &[Some(wgpu::ColorTargetState { format: target_format, blend: None, write_mask: wgpu::ColorWrites::ALL })],\n'
        '                }),\n'
        '                multiview_mask: None,\n'
        '                cache: None,\n'
        '            });\n'
        '            self.surface_pipeline = Some((target_format, p));\n'
        '        }\n'
        '        let pipeline = if target_format == OUTPUT_FORMAT {\n'
        '            &self.final_pipeline\n'
        '        } else {\n'
        '            &self.surface_pipeline.as_ref()?.1\n'
        '        };\n'
        '        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("filmcraft-composite-to-view") });\n'
        '        {\n'
        '            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {\n'
        '                label: Some("resolve-to-view"),\n'
        '                color_attachments: &[Some(wgpu::RenderPassColorAttachment {\n'
        '                    view: target_view,\n'
        '                    depth_slice: None,\n'
        '                    resolve_target: None,\n'
        '                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },\n'
        '                })],\n'
        '                depth_stencil_attachment: None,\n'
        '                timestamp_writes: None,\n'
        '                occlusion_query_set: None,\n'
        '                multiview_mask: None,\n'
        '            });\n'
        '            pass.set_pipeline(pipeline);\n'
        '            pass.set_bind_group(0, &final_bg, &[]);\n'
        '            pass.draw(0..3, 0..1);\n'
        '        }\n'
        '        self.queue.submit([enc.finish()]);\n'
        '        export_renderer::timing::add_submit(t.elapsed());\n'
        '        Some(())\n'
        '    }\n\n'
        '    pub fn render_export_prepared(&mut self, plan: &FramePlan, prep: Option<&PreparedPlan>) -> Option<(u32, u32, Vec<f32>)> {\n',
    ):
        fixed.append("crates/gpu/src/lib.rs (GpuCompositor render_to_view)")

    if fix(
        "crates/gpu/src/lib.rs",
        '        self.queue.submit([enc.finish()]);\n'
        '        export_renderer::timing::add_submit(t.elapsed());\n'
        '        self.map_accumulator(size)\n'
        '    }\n\n'
        '    /// Run the effect stage of a layer alone: `frame` decoded into its working image and `fx`\n',
        '        self.queue.submit([enc.finish()]);\n'
        '        export_renderer::timing::add_submit(t.elapsed());\n'
        '        self.map_accumulator(size)\n'
        '    }\n\n'
        '    pub fn render_export_rgba8(&mut self, plan: &FramePlan, prep: Option<&PreparedPlan>) -> Option<(u32, u32, Vec<u8>)> {\n'
        '        let t = std::time::Instant::now();\n'
        '        self.render_plan(plan, prep, true, true);\n'
        '        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("filmcraft-composite-rgba8") });\n'
        '        let size = self.copy_output(&mut enc)?;\n'
        '        self.queue.submit([enc.finish()]);\n'
        '        export_renderer::timing::add_submit(t.elapsed());\n'
        '        self.map_output(size)\n'
        '    }\n\n'
        '    fn copy_output(&mut self, enc: &mut wgpu::CommandEncoder) -> Option<(u32, u32, u32)> {\n'
        '        let (_, _, (w, h)) = self.output.as_ref()?;\n'
        '        let (w, h) = (*w, *h);\n'
        '        let row = w.checked_mul(4)?.div_ceil(256).checked_mul(256)?;\n'
        '        let needed = u64::from(row) * u64::from(h);\n'
        '        if self.output_readback_buf.as_ref().is_none_or(|(_, size)| *size < needed) {\n'
        '            let b = self.device.create_buffer(&wgpu::BufferDescriptor {\n'
        '                label: Some("output-readback"),\n'
        '                size: needed,\n'
        '                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,\n'
        '                mapped_at_creation: false,\n'
        '            });\n'
        '            self.output_readback_buf = Some((b, needed));\n'
        '        }\n'
        '        let (tex, _, _) = self.output.as_ref()?;\n'
        '        let (buf, _) = self.output_readback_buf.as_ref()?;\n'
        '        enc.copy_texture_to_buffer(\n'
        '            wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },\n'
        '            wgpu::TexelCopyBufferInfo { buffer: buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },\n'
        '            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },\n'
        '        );\n'
        '        Some((w, h, row))\n'
        '    }\n\n'
        '    fn map_output(&mut self, (w, h, row): (u32, u32, u32)) -> Option<(u32, u32, Vec<u8>)> {\n'
        '        if w == 0 || h == 0 {\n'
        '            return None;\n'
        '        }\n'
        '        let t = std::time::Instant::now();\n'
        '        let (buf, capacity) = self.output_readback_buf.take()?;\n'
        '        let slice = buf.slice(..);\n'
        '        let (tx, rx) = std::sync::mpsc::channel();\n'
        '        slice.map_async(wgpu::MapMode::Read, move |r| {\n'
        '            let _ = tx.send(r);\n'
        '        });\n'
        '        wait_mapped(&self.device, &rx)?;\n'
        '        let data = slice.get_mapped_range().ok()?;\n'
        '        let mut out = vec![0u8; (w * h * 4) as usize];\n'
        '        let width_bytes = (w * 4) as usize;\n'
        '        let row_bytes = row as usize;\n'
        '        if row_bytes == width_bytes {\n'
        '            out.copy_from_slice(&data[..width_bytes * h as usize]);\n'
        '        } else {\n'
        '            for (y, dst) in out.chunks_exact_mut(width_bytes).enumerate() {\n'
        '                let src = &data[y * row_bytes..y * row_bytes + width_bytes];\n'
        '                dst.copy_from_slice(src);\n'
        '            }\n'
        '        }\n'
        '        drop(data);\n'
        '        buf.unmap();\n'
        '        self.output_readback_buf = Some((buf, capacity));\n'
        '        export_renderer::timing::add_convert(t.elapsed());\n'
        '        Some((w, h, out))\n'
        '    }\n\n'
        '    /// Run the effect stage of a layer alone: `frame` decoded into its working image and `fx`\n',
    ):
        fixed.append("crates/gpu/src/lib.rs (GpuCompositor render_export_rgba8)")

    # 14.5 Add persistent texture pool on GpuCompositor to eliminate 11,000 Vulkan image allocations during export.
    if fix(
        "crates/gpu/src/lib.rs",
        'struct Uploaded {\n'
        '    /// Y, Cb, Cr (or the RGBA texture) and the alpha plane (the dummy texture when there is none).\n'
        '    views: [wgpu::TextureView; 4],\n',
        'struct Uploaded {\n'
        '    textures: [Option<(u32, u32, wgpu::TextureFormat, wgpu::Texture)>; 4],\n'
        '    /// Y, Cb, Cr (or the RGBA texture) and the alpha plane (the dummy texture when there is none).\n'
        '    views: [wgpu::TextureView; 4],\n',
    ):
        fixed.append("crates/gpu/src/lib.rs (struct Uploaded textures field)")

    if fix(
        "crates/gpu/src/lib.rs",
        '    fn plane_texture(&mut self, w: u32, h: u32, format: wgpu::TextureFormat, bytes: &[u8], bpp: u32) -> wgpu::TextureView {\n'
        '        let t = self.device.create_texture(&wgpu::TextureDescriptor {\n'
        '            label: Some("filmcraft-plane"),\n'
        '            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },\n'
        '            mip_level_count: 1,\n'
        '            sample_count: 1,\n'
        '            dimension: wgpu::TextureDimension::D2,\n'
        '            format,\n'
        '            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,\n'
        '            view_formats: &[],\n'
        '        });\n'
        '        self.queue.write_texture(\n'
        '            wgpu::TexelCopyTextureInfo { texture: &t, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },\n'
        '            bytes,\n'
        '            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * bpp), rows_per_image: Some(h) },\n'
        '            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },\n'
        '        );\n'
        '        self.uploaded_bytes += bytes.len() as u64;\n'
        '        t.create_view(&Default::default())\n'
        '    }\n',
        '    fn plane_texture(&mut self, w: u32, h: u32, format: wgpu::TextureFormat, bytes: &[u8], bpp: u32) -> (wgpu::Texture, wgpu::TextureView) {\n'
        '        let (t, v) = if let Some(idx) = self.plane_pool.iter().position(|(pw, ph, pfmt, _, _)| *pw == w && *ph == h && *pfmt == format) {\n'
        '            let (_, _, _, t, v) = self.plane_pool.swap_remove(idx);\n'
        '            (t, v)\n'
        '        } else {\n'
        '            let t = self.device.create_texture(&wgpu::TextureDescriptor {\n'
        '                label: Some("filmcraft-plane"),\n'
        '                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },\n'
        '                mip_level_count: 1,\n'
        '                sample_count: 1,\n'
        '                dimension: wgpu::TextureDimension::D2,\n'
        '                format,\n'
        '                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,\n'
        '                view_formats: &[],\n'
        '            });\n'
        '            let v = t.create_view(&Default::default());\n'
        '            (t, v)\n'
        '        };\n'
        '        self.queue.write_texture(\n'
        '            wgpu::TexelCopyTextureInfo { texture: &t, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },\n'
        '            bytes,\n'
        '            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * bpp), rows_per_image: Some(h) },\n'
        '            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },\n'
        '        );\n'
        '        self.uploaded_bytes += bytes.len() as u64;\n'
        '        (t, v)\n'
        '    }\n',
    ):
        fixed.append("crates/gpu/src/lib.rs (GpuCompositor plane_texture reuse from pool)")

    if fix(
        "crates/gpu/src/lib.rs",
        '        let up = match &f.data {\n'
        '            PixelData::Rgba8(d) => {\n'
        '                let v = self.plane_texture(w, h, wgpu::TextureFormat::Rgba8UnormSrgb, d, 4);\n'
        '                Uploaded {\n'
        '                    views: [v, dummy(), dummy(), dummy()],\n'
        '                    kind: 0,\n'
        '                    code_scale: 1.0,\n'
        '                    alpha_scale: 0.0,\n'
        '                    chroma: (w, h),\n'
        '                    last_used: self.clock,\n'
        '                    _pixels: f.data.clone(),\n'
        '                }\n'
        '            }\n'
        '            PixelData::RgbaF32(d) => {\n'
        '                let owned;\n'
        '                let half = match prep {\n'
        '                    Some(p) => &p.planes[0],\n'
        '                    None => {\n'
        '                        owned = f32_to_f16_bytes(d);\n'
        '                        &owned\n'
        '                    }\n'
        '                };\n'
        '                let v = self.plane_texture(w, h, wgpu::TextureFormat::Rgba16Float, half, 8);\n'
        '                Uploaded {\n'
        '                    views: [v, dummy(), dummy(), dummy()],\n'
        '                    kind: 1,\n'
        '                    code_scale: 1.0,\n'
        '                    alpha_scale: 0.0,\n'
        '                    chroma: (w, h),\n'
        '                    last_used: self.clock,\n'
        '                    _pixels: f.data.clone(),\n'
        '                }\n'
        '            }\n'
        '            PixelData::Yuv8 { planes, chroma, alpha } => {\n'
        '                let (sx, sy) = chroma.shifts();\n'
        '                let (cw, ch) = (w.div_ceil(1 << sx), h.div_ceil(1 << sy));\n'
        '                let y = self.plane_texture(w, h, wgpu::TextureFormat::R8Unorm, &planes[0], 1);\n'
        '                let u = self.plane_texture(cw, ch, wgpu::TextureFormat::R8Unorm, &planes[1], 1);\n'
        '                let v = self.plane_texture(cw, ch, wgpu::TextureFormat::R8Unorm, &planes[2], 1);\n'
        '                let a = alpha.as_ref().map(|a| self.plane_texture(w, h, wgpu::TextureFormat::R8Unorm, a, 1));\n'
        '                let alpha_scale = if a.is_some() { 1.0 } else { 0.0 };\n'
        '                Uploaded {\n'
        '                    views: [y, u, v, a.unwrap_or_else(dummy)],\n'
        '                    kind: 2,\n'
        '                    code_scale: 255.0,\n'
        '                    alpha_scale,\n'
        '                    chroma: (cw, ch),\n'
        '                    last_used: self.clock,\n'
        '                    _pixels: f.data.clone(),\n'
        '                }\n'
        '            }\n'
        '            PixelData::Yuv16 { planes, chroma, bits, alpha } => {\n'
        '                let (sx, sy) = chroma.shifts();\n'
        '                let (cw, ch) = (w.div_ceil(1 << sx), h.div_ceil(1 << sy));\n'
        '                let scale = (1u32 << bits) as f32;\n'
        '                let owned;\n'
        '                let p = match prep {\n'
        '                    Some(p) => p,\n'
        '                    None => {\n'
        '                        owned = Prepared { planes: planes.iter().chain(alpha.as_ref()).map(|p| codes_to_f16_bytes(p, *bits)).collect() };\n'
        '                        &owned\n'
        '                    }\n'
        '                };\n'
        '                let y = self.plane_texture(w, h, wgpu::TextureFormat::R16Float, &p.planes[0], 2);\n'
        '                let u = self.plane_texture(cw, ch, wgpu::TextureFormat::R16Float, &p.planes[1], 2);\n'
        '                let v = self.plane_texture(cw, ch, wgpu::TextureFormat::R16Float, &p.planes[2], 2);\n'
        '                // (full-resolution alpha plane: its codes span 0..2^bits - 1, hence the scale)\n'
        '                let a = alpha.as_ref().and_then(|_| p.planes.get(3)).map(|b| self.plane_texture(w, h, wgpu::TextureFormat::R16Float, b, 2));\n'
        '                let alpha_scale = if a.is_some() { scale / (scale - 1.0) } else { 0.0 };\n'
        '                let _ = Chroma::C420;\n'
        '                Uploaded {\n'
        '                    views: [y, u, v, a.unwrap_or_else(dummy)],\n'
        '                    kind: 2,\n'
        '                    code_scale: scale,\n'
        '                    alpha_scale,\n'
        '                    chroma: (cw, ch),\n'
        '                    last_used: self.clock,\n'
        '                    _pixels: f.data.clone(),\n'
        '                }\n'
        '            }\n'
        '        };\n'
        '        self.uploads.insert(key, up);\n',
        '        let up = match &f.data {\n'
        '            PixelData::Rgba8(d) => {\n'
        '                let (t, v) = self.plane_texture(w, h, wgpu::TextureFormat::Rgba8UnormSrgb, d, 4);\n'
        '                Uploaded {\n'
        '                    textures: [Some((w, h, wgpu::TextureFormat::Rgba8UnormSrgb, t)), None, None, None],\n'
        '                    views: [v, dummy(), dummy(), dummy()],\n'
        '                    kind: 0,\n'
        '                    code_scale: 1.0,\n'
        '                    alpha_scale: 0.0,\n'
        '                    chroma: (w, h),\n'
        '                    last_used: self.clock,\n'
        '                    _pixels: f.data.clone(),\n'
        '                }\n'
        '            }\n'
        '            PixelData::RgbaF32(d) => {\n'
        '                let owned;\n'
        '                let half = match prep {\n'
        '                    Some(p) => &p.planes[0],\n'
        '                    None => {\n'
        '                        owned = f32_to_f16_bytes(d);\n'
        '                        &owned\n'
        '                    }\n'
        '                };\n'
        '                let (t, v) = self.plane_texture(w, h, wgpu::TextureFormat::Rgba16Float, half, 8);\n'
        '                Uploaded {\n'
        '                    textures: [Some((w, h, wgpu::TextureFormat::Rgba16Float, t)), None, None, None],\n'
        '                    views: [v, dummy(), dummy(), dummy()],\n'
        '                    kind: 1,\n'
        '                    code_scale: 1.0,\n'
        '                    alpha_scale: 0.0,\n'
        '                    chroma: (w, h),\n'
        '                    last_used: self.clock,\n'
        '                    _pixels: f.data.clone(),\n'
        '                }\n'
        '            }\n'
        '            PixelData::Yuv8 { planes, chroma, alpha } => {\n'
        '                let (sx, sy) = chroma.shifts();\n'
        '                let (cw, ch) = (w.div_ceil(1 << sx), h.div_ceil(1 << sy));\n'
        '                let (ty, y) = self.plane_texture(w, h, wgpu::TextureFormat::R8Unorm, &planes[0], 1);\n'
        '                let (tu, u) = self.plane_texture(cw, ch, wgpu::TextureFormat::R8Unorm, &planes[1], 1);\n'
        '                let (tv, v) = self.plane_texture(cw, ch, wgpu::TextureFormat::R8Unorm, &planes[2], 1);\n'
        '                let (ta, a) = match alpha.as_ref() {\n'
        '                    Some(ab) => {\n'
        '                        let (t, v) = self.plane_texture(w, h, wgpu::TextureFormat::R8Unorm, ab, 1);\n'
        '                        (Some((w, h, wgpu::TextureFormat::R8Unorm, t)), Some(v))\n'
        '                    }\n'
        '                    None => (None, None),\n'
        '                };\n'
        '                let alpha_scale = if a.is_some() { 1.0 } else { 0.0 };\n'
        '                Uploaded {\n'
        '                    textures: [\n'
        '                        Some((w, h, wgpu::TextureFormat::R8Unorm, ty)),\n'
        '                        Some((cw, ch, wgpu::TextureFormat::R8Unorm, tu)),\n'
        '                        Some((cw, ch, wgpu::TextureFormat::R8Unorm, tv)),\n'
        '                        ta,\n'
        '                    ],\n'
        '                    views: [y, u, v, a.unwrap_or_else(dummy)],\n'
        '                    kind: 2,\n'
        '                    code_scale: 255.0,\n'
        '                    alpha_scale,\n'
        '                    chroma: (cw, ch),\n'
        '                    last_used: self.clock,\n'
        '                    _pixels: f.data.clone(),\n'
        '                }\n'
        '            }\n'
        '            PixelData::Yuv16 { planes, chroma, bits, alpha } => {\n'
        '                let (sx, sy) = chroma.shifts();\n'
        '                let (cw, ch) = (w.div_ceil(1 << sx), h.div_ceil(1 << sy));\n'
        '                let scale = (1u32 << bits) as f32;\n'
        '                let owned;\n'
        '                let p = match prep {\n'
        '                    Some(p) => p,\n'
        '                    None => {\n'
        '                        owned = Prepared { planes: planes.iter().chain(alpha.as_ref()).map(|p| codes_to_f16_bytes(p, *bits)).collect() };\n'
        '                        &owned\n'
        '                    }\n'
        '                };\n'
        '                let (ty, y) = self.plane_texture(w, h, wgpu::TextureFormat::R16Float, &p.planes[0], 2);\n'
        '                let (tu, u) = self.plane_texture(cw, ch, wgpu::TextureFormat::R16Float, &p.planes[1], 2);\n'
        '                let (tv, v) = self.plane_texture(cw, ch, wgpu::TextureFormat::R16Float, &p.planes[2], 2);\n'
        '                let (ta, a) = match alpha.as_ref().and_then(|_| p.planes.get(3)) {\n'
        '                    Some(ab) => {\n'
        '                        let (t, v) = self.plane_texture(w, h, wgpu::TextureFormat::R16Float, ab, 2);\n'
        '                        (Some((w, h, wgpu::TextureFormat::R16Float, t)), Some(v))\n'
        '                    }\n'
        '                    None => (None, None),\n'
        '                };\n'
        '                let alpha_scale = if a.is_some() { scale / (scale - 1.0) } else { 0.0 };\n'
        '                let _ = Chroma::C420;\n'
        '                Uploaded {\n'
        '                    textures: [\n'
        '                        Some((w, h, wgpu::TextureFormat::R16Float, ty)),\n'
        '                        Some((cw, ch, wgpu::TextureFormat::R16Float, tu)),\n'
        '                        Some((cw, ch, wgpu::TextureFormat::R16Float, tv)),\n'
        '                        ta,\n'
        '                    ],\n'
        '                    views: [y, u, v, a.unwrap_or_else(dummy)],\n'
        '                    kind: 2,\n'
        '                    code_scale: scale,\n'
        '                    alpha_scale,\n'
        '                    chroma: (cw, ch),\n'
        '                    last_used: self.clock,\n'
        '                    _pixels: f.data.clone(),\n'
        '                }\n'
        '            }\n'
        '        };\n'
        '        self.uploads.insert(key, up);\n',
    ):
        fixed.append("crates/gpu/src/lib.rs (GpuCompositor upload texture pooling and eviction recycle)")

    # 15. Add hardware surface binding and direct surface rendering on ExportRenderer.
    if fix(
        "crates/gpu/src/export_renderer.rs",
        'pub struct ExportRenderer {\n'
        '    _instance: wgpu::Instance,\n'
        '    #[expect(dead_code, reason = "kept for future frame-accuracy flows (readback fences)")]\n'
        '    device: wgpu::Device,\n'
        '    #[expect(dead_code, reason = "kept for future frame-accuracy flows (readback fences)")]\n'
        '    queue: wgpu::Queue,\n'
        '    compositor: GpuCompositor,\n'
        '}\n',
        'pub struct ExportRenderer {\n'
        '    instance: wgpu::Instance,\n'
        '    adapter: wgpu::Adapter,\n'
        '    #[expect(dead_code, reason = "kept for future frame-accuracy flows (readback fences)")]\n'
        '    device: wgpu::Device,\n'
        '    #[expect(dead_code, reason = "kept for future frame-accuracy flows (readback fences)")]\n'
        '    queue: wgpu::Queue,\n'
        '    compositor: GpuCompositor,\n'
        '    surface: Option<wgpu::Surface<\'static>>,\n'
        '    surface_format: Option<wgpu::TextureFormat>,\n'
        '}\n',
    ):
        fixed.append("crates/gpu/src/export_renderer.rs (ExportRenderer surface field)")

    if fix(
        "crates/gpu/src/export_renderer.rs",
        '        static SHARED: std::sync::OnceLock<Option<(wgpu::Instance, wgpu::Device, wgpu::Queue)>> = std::sync::OnceLock::new();\n'
        '        let (instance, device, queue) = SHARED\n',
        '        static SHARED: std::sync::OnceLock<Option<(wgpu::Instance, wgpu::Adapter, wgpu::Device, wgpu::Queue)>> = std::sync::OnceLock::new();\n'
        '        let (instance, adapter, device, queue) = SHARED\n',
    ):
        fixed.append("crates/gpu/src/export_renderer.rs (ExportRenderer shared adapter)")

    if fix(
        "crates/gpu/src/export_renderer.rs",
        '                Some((instance, device, queue))\n'
        '            })\n',
        '                Some((instance, adapter, device, queue))\n'
        '            })\n',
    ):
        fixed.append("crates/gpu/src/export_renderer.rs (ExportRenderer shared return adapter)")

    if fix(
        "crates/gpu/src/export_renderer.rs",
        '        Some(Self { _instance: instance, device, queue, compositor })\n'
        '    }\n',
        '        Some(Self { instance, adapter, device, queue, compositor, surface: None, surface_format: None })\n'
        '    }\n',
    ):
        fixed.append("crates/gpu/src/export_renderer.rs (ExportRenderer new with adapter)")

    if fix(
        "crates/gpu/src/export_renderer.rs",
        '    pub fn render(&mut self, project: &Project, seq: ItemId, t: Tick, opts: RenderOptions, sources: &dyn SourceProvider) -> Option<Image> {\n',
        '    pub fn bind_surface(&mut self, window_ptr: *mut std::ffi::c_void, w: u32, h: u32) -> bool {\n'
        '        if window_ptr.is_null() {\n'
        '            return false;\n'
        '        }\n'
        '        #[cfg(target_os = "android")]\n'
        '        {\n'
        '            #[link(name = "android")]\n'
        '            unsafe extern "C" {\n'
        '                fn ANativeWindow_setBuffersGeometry(window: *mut std::ffi::c_void, width: i32, height: i32, format: i32) -> i32;\n'
        '            }\n'
        '            unsafe {\n'
        '                // 1 = WINDOW_FORMAT_RGBA_8888 in android/native_window.h\n'
        '                let res = ANativeWindow_setBuffersGeometry(window_ptr, w as i32, h as i32, 1);\n'
        '                log::info!("ANativeWindow_setBuffersGeometry({w}x{h}, format=RGBA_8888) = {res}");\n'
        '            }\n'
        '        }\n'
        '        let non_null = match std::ptr::NonNull::new(window_ptr) {\n'
        '            Some(p) => p,\n'
        '            None => return false,\n'
        '        };\n'
        '        let handle = wgpu::rwh::AndroidNdkWindowHandle::new(non_null);\n'
        '        let target = wgpu::SurfaceTargetUnsafe::RawHandle {\n'
        '            raw_display_handle: Some(wgpu::rwh::RawDisplayHandle::Android(wgpu::rwh::AndroidDisplayHandle::new())),\n'
        '            raw_window_handle: wgpu::rwh::RawWindowHandle::AndroidNdk(handle),\n'
        '        };\n'
        '        let surface: wgpu::Surface<\'static> = match unsafe { self.instance.create_surface_unsafe(target) } {\n'
        '            Ok(s) => s,\n'
        '            Err(e) => {\n'
        '                log::warn!("Failed to create wgpu::Surface on native window: {e:?}");\n'
        '                return false;\n'
        '            }\n'
        '        };\n'
        '        let caps = surface.get_capabilities(&self.adapter);\n'
        '        log::info!("Surface capabilities: formats={:?}, present_modes={:?}, alpha={:?}", caps.formats, caps.present_modes, caps.alpha_modes);\n'
        '        if caps.formats.is_empty() || caps.present_modes.is_empty() {\n'
        '            log::warn!("Surface has no supported formats or present modes with adapter; declining surface mode");\n'
        '            return false;\n'
        '        }\n'
        '        let format = if caps.formats.contains(&crate::OUTPUT_FORMAT) { crate::OUTPUT_FORMAT } else { caps.formats[0] };\n'
        '        let present_mode = if caps.present_modes.contains(&wgpu::PresentMode::AutoNoVsync) { wgpu::PresentMode::AutoNoVsync } else { caps.present_modes[0] };\n'
        '        let alpha_mode = caps.alpha_modes[0];\n'
        '        let config = wgpu::SurfaceConfiguration {\n'
        '            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,\n'
        '            format,\n'
        '            color_space: wgpu::SurfaceColorSpace::Auto,\n'
        '            width: w,\n'
        '            height: h,\n'
        '            present_mode,\n'
        '            desired_maximum_frame_latency: 2,\n'
        '            alpha_mode,\n'
        '            view_formats: vec![],\n'
        '        };\n'
        '        let configure_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {\n'
        '            surface.configure(&self.device, &config);\n'
        '        }));\n'
        '        if configure_res.is_err() {\n'
        '            log::warn!("surface.configure panicked on this hardware window; declining surface mode");\n'
        '            return false;\n'
        '        }\n'
        '        log::info!("ExportRenderer successfully configured hardware surface: {w}x{h} ({format:?})");\n'
        '        self.surface = Some(surface);\n'
        '        self.surface_format = Some(format);\n'
        '        true\n'
        '    }\n\n'
        '    pub fn render_plan_to_surface(&mut self, plan: &filmcraft_render::plan::FramePlan) -> Option<bool> {\n'
        '        let surface = self.surface.as_ref()?;\n'
        '        let format = self.surface_format?;\n'
        '        match plan {\n'
        '            FramePlan::Layers { .. } | FramePlan::Composite { .. } => {\n'
        '                if device_failed() {\n'
        '                    return None;\n'
        '                }\n'
        '                let prep = prepare(plan);\n'
        '                let surface_tex = match surface.get_current_texture() {\n'
        '                    wgpu::CurrentSurfaceTexture::Success(tex) | wgpu::CurrentSurfaceTexture::Suboptimal(tex) => tex,\n'
        '                    e => {\n'
        '                        log::warn!("Surface get_current_texture failed: {e:?}");\n'
        '                        return None;\n'
        '                    }\n'
        '                };\n'
        '                let view = surface_tex.texture.create_view(&wgpu::TextureViewDescriptor::default());\n'
        '                self.compositor.render_to_view(plan, Some(&prep), &view, format)?;\n'
        '                self.queue.present(surface_tex);\n'
        '                if device_failed() {\n'
        '                    return None;\n'
        '                }\n'
        '                timing::add_frame();\n'
        '                Some(true)\n'
        '            }\n'
        '            FramePlan::Image(_) => None,\n'
        '        }\n'
        '    }\n\n'
        '    pub fn render_to_surface(\n'
        '        &mut self,\n'
        '        project: &Project,\n'
        '        seq: ItemId,\n'
        '        t: Tick,\n'
        '        opts: RenderOptions,\n'
        '        sources: &dyn SourceProvider,\n'
        '    ) -> Option<bool> {\n'
        '        let plan = plan_frame(project, seq, t, opts, self.compositor.caps(), sources);\n'
        '        self.render_plan_to_surface(&plan)\n'
        '    }\n\n'
        '    pub fn render_rgba8(\n'
        '        &mut self,\n'
        '        project: &Project,\n'
        '        seq: ItemId,\n'
        '        t: Tick,\n'
        '        opts: RenderOptions,\n'
        '        sources: &dyn SourceProvider,\n'
        '    ) -> Option<Vec<u8>> {\n'
        '        let plan = plan_frame(project, seq, t, opts, self.compositor.caps(), sources);\n'
        '        match &plan {\n'
        '            FramePlan::Layers { .. } | FramePlan::Composite { .. } => {\n'
        '                if device_failed() {\n'
        '                    return None;\n'
        '                }\n'
        '                let prep = prepare(&plan);\n'
        '                let (_w, _h, px) = self.compositor.render_export_rgba8(&plan, Some(&prep))?;\n'
        '                if device_failed() {\n'
        '                    return None;\n'
        '                }\n'
        '                timing::add_frame();\n'
        '                Some(px)\n'
        '            }\n'
        '            FramePlan::Image(_) => None,\n'
        '        }\n'
        '    }\n\n'
        '    pub fn render(&mut self, project: &Project, seq: ItemId, t: Tick, opts: RenderOptions, sources: &dyn SourceProvider) -> Option<Image> {\n',
    ):
        fixed.append("crates/gpu/src/export_renderer.rs (ExportRenderer bind_surface, render_to_surface and render_rgba8)")

    # 16. Support surface rendering and fast GPU RGBA8 dispatch in Pipeline.
    if fix(
        "crates/export/src/pipeline.rs",
        '        let pool = self.renderer.as_ref().filter(|pool| !filmcraft_render::plan::is_cpu_frame(&self.project, self.seq, t, self.opts, pool.caps));\n'
        '        let gpu = match pool {\n',
        '        let pool = self.renderer.as_ref().filter(|pool| !filmcraft_render::plan::is_cpu_frame(&self.project, self.seq, t, self.opts, pool.caps));\n'
        '        let is_fullscreen = match self.geom.content {\n'
        '            None => true,\n'
        '            Some((cw, ch)) => cw == self.w as usize && ch == self.h as usize,\n'
        '        };\n'
        '        let can_fast_rgba8 = !self.hdr_out\n'
        '            && !self.alpha\n'
        '            && is_fullscreen\n'
        '            && self.geom.offset == (0, 0)\n'
        '            && !self.effects.image_overlay.enabled\n'
        '            && !self.effects.name_overlay.enabled\n'
        '            && !self.effects.timecode_overlay.enabled\n'
        '            && !self.effects.video_limiter.enabled;\n'
        '        if can_fast_rgba8 {\n'
        '            if let Some(pool) = pool {\n'
        '                let asked = web_time::Instant::now();\n'
        '                let rgba = pool.with(|r| {\n'
        '                    crate::note_lock_wait(asked.elapsed());\n'
        '                    r.render_rgba8(&self.project, self.seq, t, self.opts, sources)\n'
        '                }).flatten();\n'
        '                if let Some(rgba) = rgba {\n'
        '                    note_gpu_frame();\n'
        '                    return (rgba, Vec::new());\n'
        '                }\n'
        '            }\n'
        '        }\n'
        '        let gpu = match pool {\n',
    ):
        fixed.append("crates/export/src/pipeline.rs (Pipeline fast rgba8 dispatch)")

    # 16. Support surface rendering in Pipeline.
    if fix(
        "crates/export/src/pipeline.rs",
        '    /// Free the pool\'s float images when the export ends: a standalone export, not one part of a\n'
        '    /// batch (`ExportSettings::part_of_batch`).\n'
        '    trim_pool: bool,\n'
        '}\n',
        '    /// Free the pool\'s float images when the export ends: a standalone export, not one part of a\n'
        '    /// batch (`ExportSettings::part_of_batch`).\n'
        '    trim_pool: bool,\n'
        '    pub src_w: u32,\n'
        '    pub src_h: u32,\n'
        '    surface_renderer: std::sync::Mutex<Option<Box<dyn FrameRenderer>>>,\n'
        '    surface_bound: std::sync::atomic::AtomicBool,\n'
        '    surface_caps: std::sync::atomic::AtomicU32,\n'
        '}\n',
    ):
        fixed.append("crates/export/src/pipeline.rs (Pipeline surface_bound field)")

    if fix(
        "crates/export/src/pipeline.rs",
        '            trim_pool: !settings.part_of_batch,\n'
        '            project: project.clone(),\n',
        '            trim_pool: !settings.part_of_batch,\n'
        '            src_w: q.settings.width,\n'
        '            src_h: q.settings.height,\n'
        '            surface_renderer: std::sync::Mutex::new(None),\n'
        '            surface_bound: std::sync::atomic::AtomicBool::new(false),\n'
        '            surface_caps: std::sync::atomic::AtomicU32::new(0),\n'
        '            project: project.clone(),\n',
    ):
        fixed.append("crates/export/src/pipeline.rs (Pipeline init surface_bound)")

    if fix(
        "crates/export/src/pipeline.rs",
        '    /// Output frame `f` as straight sRGB RGBA8 at the output size, or for HDR exports the encoded\n'
        '    /// R\'G\'B\' floats (3 per pixel).\n'
        '    pub fn frame(&self, f: i64, sources: &dyn SourceProvider) -> (Vec<u8>, Vec<f32>) {\n',
        '    pub fn bind_surface(&self, window_ptr: *mut std::ffi::c_void) -> bool {\n'
        '        if !self.can_surface_render() {\n'
        '            return false;\n'
        '        }\n'
        '        let is_1to1 = (self.opts.scale - 1.0).abs() < 1e-4;\n'
        '        if cfg!(target_os = "android") {\n'
        '            if is_1to1 {\n'
        '                filmcraft_codecs::hw::set_direct_surface(window_ptr, self.w, self.h);\n'
        '                self.surface_bound.store(true, std::sync::atomic::Ordering::Relaxed);\n'
        '                log::info!("Pipeline configured direct 1:1 decoder-to-encoder hardware surface bridge for {}x{}", self.w, self.h);\n'
        '                return true;\n'
        '            } else if let Some(decoder_win) = filmcraft_codecs::hw::init_surface_scaler(window_ptr, self.src_w, self.src_h, self.w, self.h) {\n'
        '                filmcraft_codecs::hw::set_direct_surface(decoder_win, self.src_w, self.src_h);\n'
        '                self.surface_bound.store(true, std::sync::atomic::Ordering::Relaxed);\n'
        '                log::info!("Pipeline configured zero-copy HardwareSurfaceScaler: {}x{} -> {}x{}", self.src_w, self.src_h, self.w, self.h);\n'
        '                return true;\n'
        '            }\n'
        '        }\n'
        '        if let Some(pool) = &self.renderer {\n'
        '            let (w, h) = (self.w, self.h);\n'
        '            let mut free = pool.renderers.lock().unwrap_or_else(std::sync::PoisonError::into_inner);\n'
        '            if let Some(mut r) = free.pop() {\n'
        '                let ok = r.bind_surface(window_ptr, w, h);\n'
        '                if ok {\n'
        '                    let caps = r.caps();\n'
        '                    self.surface_caps.store(caps.adjust_side, std::sync::atomic::Ordering::Relaxed);\n'
        '                    *self.surface_renderer.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(r);\n'
        '                    self.surface_bound.store(true, std::sync::atomic::Ordering::Relaxed);\n'
        '                    log::info!("Pipeline configured GPU surface renderer for {}x{} (scale={:.3})", w, h, self.opts.scale);\n'
        '                    return true;\n'
        '                } else {\n'
        '                    free.push(r);\n'
        '                }\n'
        '            }\n'
        '        }\n'
        '        false\n'
        '    }\n\n'
        '    pub fn has_surface(&self) -> bool {\n'
        '        self.surface_bound.load(std::sync::atomic::Ordering::Relaxed)\n'
        '    }\n\n'
        '    pub fn can_surface_render(&self) -> bool {\n'
        '        let is_fullscreen = match self.geom.content {\n'
        '            None => true,\n'
        '            Some((cw, ch)) => cw == self.w as usize && ch == self.h as usize,\n'
        '        };\n'
        '        !self.hdr_out\n'
        '            && is_fullscreen\n'
        '            && self.geom.offset == (0, 0)\n'
        '            && !self.effects.image_overlay.enabled\n'
        '            && !self.effects.name_overlay.enabled\n'
        '            && !self.effects.timecode_overlay.enabled\n'
        '            && !self.effects.video_limiter.enabled\n'
        '    }\n\n'
        '    pub fn surface_caps(&self) -> filmcraft_render::plan::DeviceCaps {\n'
        '        filmcraft_render::plan::DeviceCaps {\n'
        '            adjust_side: self.surface_caps.load(std::sync::atomic::Ordering::Relaxed),\n'
        '        }\n'
        '    }\n\n'
        '    pub fn plan_frame(&self, t: filmcraft_time::Tick, sources: &dyn SourceProvider) -> filmcraft_render::plan::FramePlan {\n'
        '        filmcraft_render::plan::plan_frame(&self.project, self.seq, t, self.opts, self.surface_caps(), sources)\n'
        '    }\n\n'
        '    pub fn render_plan_to_surface(&self, plan: &filmcraft_render::plan::FramePlan) -> Option<bool> {\n'
        '        if filmcraft_codecs::hw::has_direct_surface() {\n'
        '            if filmcraft_codecs::hw::has_surface_scaler() {\n'
        '                return Some(filmcraft_codecs::hw::blit_surface_scaler());\n'
        '            }\n'
        '            return Some(true);\n'
        '        }\n'
        '        let mut guard = self.surface_renderer.lock().unwrap_or_else(std::sync::PoisonError::into_inner);\n'
        '        let r = guard.as_mut()?;\n'
        '        r.render_plan_to_surface(plan)\n'
        '    }\n\n'
        '    pub fn render_to_surface(&self, t: filmcraft_time::Tick, sources: &dyn SourceProvider) -> Option<bool> {\n'
        '        let plan = self.plan_frame(t, sources);\n'
        '        self.render_plan_to_surface(&plan)\n'
        '    }\n\n'
        '    /// Output frame `f` as straight sRGB RGBA8 at the output size, or for HDR exports the encoded\n'
        '    /// R\'G\'B\' floats (3 per pixel).\n'
        '    pub fn frame(&self, f: i64, sources: &dyn SourceProvider) -> (Vec<u8>, Vec<f32>) {\n',
    ):
        fixed.append("crates/export/src/pipeline.rs (Pipeline bind_surface and render_to_surface)")

    # 17. Bind surface in Exporter::set_up and use step_surface_batch for zero-copy hardware encoding.
    if fix(
        "crates/export/src/job.rs",
        '        let venc = make_venc(&settings, pipe.w, pipe.h, pipe.rate)?;\n',
        '        let mut venc = make_venc(&settings, pipe.w, pipe.h, pipe.rate)?;\n',
    ):
        fixed.append("crates/export/src/job.rs (Exporter let mut venc in set_up)")

    if fix(
        "crates/export/src/job.rs",
        '        let measured = !settings.effects.loudness.enabled || audio.is_none();\n'
        '        Ok(Self {\n',
        '        let measured = !settings.effects.loudness.enabled || audio.is_none();\n'
        '        if pipe.can_surface_render() {\n'
        '            if let Some(surface_ptr) = venc.input_surface() {\n'
        '                filmcraft_codecs::release_idle();\n'
        '                if pipe.bind_surface(surface_ptr) {\n'
        '                    log::info!("Exporter enabled zero-copy hardware surface pipeline for {}x{}", pipe.w, pipe.h);\n'
        '                } else {\n'
        '                    log::warn!("Pipeline failed to bind hardware surface; falling back to buffer mode");\n'
        '                    let _ = venc.fallback_to_buffer();\n'
        '                }\n'
        '            }\n'
        '        }\n'
        '        Ok(Self {\n',
    ):
        fixed.append("crates/export/src/job.rs (Exporter bind_surface in set_up)")

    if fix(
        "crates/export/src/job.rs",
        '        if self.next < self.f1 {\n'
        '            return self.step_batch(sources, progress);\n'
        '        }\n',
        '        if self.next < self.f1 {\n'
        '            if self.pipe.has_surface() && self.venc.input_surface().is_some() {\n'
        '                return self.step_surface_batch(sources, progress);\n'
        '            }\n'
        '            return self.step_batch(sources, progress);\n'
        '        }\n',
    ):
        fixed.append("crates/export/src/job.rs (Exporter step_surface_batch dispatch)")

    if fix(
        "crates/export/src/job.rs",
        '    /// Encode (and mux) the batch starting at `self.next`, rendering the one after it meanwhile.\n'
        '    fn step_batch(&mut self, sources: &dyn SourceProvider, progress: &Progress) -> Result<Step> {\n',
        '    fn step_surface_batch(&mut self, sources: &dyn SourceProvider, progress: &Progress) -> Result<Step> {\n'
        '        let f = self.next;\n'
        '        let batch_size = if cfg!(target_os = "android") { 64 } else { self.batch_len() };\n'
        '        let end = self.next.saturating_add(batch_size).min(self.f1);\n'
        '        if end <= f {\n'
        '            return Err(ExportError::Encode("internal: an empty batch was rendered".into()));\n'
        '        }\n'
        '        let cuts: Vec<i64> = (f + 1..=end).filter(|&g| (g - self.f0) % INTERLEAVE == 0).collect();\n'
        '        let mut mixed = Vec::with_capacity(cuts.len());\n'
        '        if let Some(sr) = self.audio.as_ref().map(|a| a.sr) {\n'
        '            let t = web_time::Instant::now();\n'
        '            for &g in &cuts {\n'
        '                let until = self.sample_at_frame(g, sr);\n'
        '                mixed.push(self.audio.as_mut().and_then(|a| a.pull(until, sources)));\n'
        '            }\n'
        '            note_stage(Stage::Audio, t.elapsed());\n'
        '        }\n'
        '        if filmcraft_media::pending::take() {\n'
        '            return Ok(Step::Pending);\n'
        '        }\n'
        '        let mut mixed = mixed.into_iter();\n'
        '        let pipe = self.pipe.clone();\n'
        '        let f0 = self.f0;\n'
        '        let pipe_w = pipe.w;\n'
        '        let pipe_h = pipe.h;\n'
        '        for fi in f..end {\n'
        '            if progress.cancel.load(Ordering::Relaxed) {\n'
        '                filmcraft_codecs::hw::clear_direct_surface();\n'
        '                filmcraft_codecs::release_idle();\n'
        '                return Err(ExportError::Cancelled);\n'
        '            }\n'
        '            let t = pipe.rate.tick_of(fi);\n'
        '            let plan = pipe.plan_frame(t, sources);\n'
        '            let rendered = timed(Stage::Render, || pipe.render_plan_to_surface(&plan));\n'
        '            if rendered != Some(true) {\n'
        '                log::warn!("Hardware surface render fallback at frame {fi}; switching to buffer batch");\n'
        '                filmcraft_codecs::hw::clear_direct_surface();\n'
        '                filmcraft_codecs::release_idle();\n'
        '                let _ = self.venc.fallback_to_buffer();\n'
        '                return self.step_batch(sources, progress);\n'
        '            }\n'
        '            let fr = EncoderFrame {\n'
        '                width: pipe_w,\n'
        '                height: pipe_h,\n'
        '                rgba: &[],\n'
        '                hdr: None,\n'
        '                index: (fi - f0) as u64,\n'
        '            };\n'
        '            let packets = match timed(Stage::Encode, || self.venc.encode(&fr)) {\n'
        '                Ok(p) => p,\n'
        '                Err(e) => return Err(e),\n'
        '            };\n'
        '            self.queued.extend(packets);\n'
        '            if cuts.contains(&(fi + 1)) {\n'
        '                let mux_res = timed(Stage::Mux, || -> Result<()> {\n'
        '                    if self.mux.is_none() && self.mxf.is_none() {\n'
        '                        self.open_mux()?;\n'
        '                    }\n'
        '                    let packets = std::mem::take(&mut self.queued);\n'
        '                    self.write_video(packets)\n'
        '                });\n'
        '                mux_res?;\n'
        '                let pcm = mixed.next().flatten();\n'
        '                timed(Stage::Audio, || self.write_audio(pcm))?;\n'
        '            }\n'
        '        }\n'
        '        progress.done.fetch_add((end - f) as u64, Ordering::Relaxed);\n'
        '        self.next = end;\n'
        '        Ok(Step::Progress)\n'
        '    }\n\n'
        '    /// Encode (and mux) the batch starting at `self.next`, rendering the one after it meanwhile.\n'
        '    fn step_batch(&mut self, sources: &dyn SourceProvider, progress: &Progress) -> Result<Step> {\n',
    ):
        fixed.append("crates/export/src/job.rs (Exporter step_surface_batch implementation)")

    if fix(
        "crates/export/src/job.rs",
        '        note_stage(Stage::Finish, finishing.elapsed());\n'
        '        let secs = self.t0.elapsed().as_secs_f64();\n',
        '        note_stage(Stage::Finish, finishing.elapsed());\n'
        '        filmcraft_codecs::hw::clear_direct_surface();\n'
        '        filmcraft_codecs::release_idle();\n'
        '        let secs = self.t0.elapsed().as_secs_f64();\n',
    ):
        fixed.append("crates/export/src/job.rs (Exporter finish direct surface cleanup)")

    if fix(
        "crates/export/src/job.rs",
        '        if progress.cancel.load(Ordering::Relaxed) {\n'
        '            self.ahead = None;\n'
        '            return Err(ExportError::Cancelled);\n'
        '        }\n',
        '        if progress.cancel.load(Ordering::Relaxed) {\n'
        '            self.ahead = None;\n'
        '            filmcraft_codecs::hw::clear_direct_surface();\n'
        '            filmcraft_codecs::release_idle();\n'
        '            return Err(ExportError::Cancelled);\n'
        '        }\n',
    ):
        fixed.append("crates/export/src/job.rs (Exporter cancel direct surface cleanup)")

    if fix(
        "crates/export/src/job.rs",
        'impl Exporter {\n',
        'impl Drop for Exporter {\n'
        '    fn drop(&mut self) {\n'
        '        filmcraft_codecs::hw::clear_direct_surface();\n'
        '        filmcraft_codecs::release_idle();\n'
        '    }\n'
        '}\n\n'
        'impl Exporter {\n',
    ):
        fixed.append("crates/export/src/job.rs (Exporter Drop cleanup)")
elif app == "photocraft":
    if fix("crates/raster/src/memory/system.rs", '#[cfg(target_os = "linux")]', '#[cfg(any(target_os = "linux", target_os = "android"))]'):
        fixed.append("crates/raster/src/memory/system.rs (Android meminfo)")
    fix("crates/raster/src/memory/system.rs", '#[cfg(not(target_os = "linux"))]', '#[cfg(not(any(target_os = "linux", target_os = "android")))]')
    fix("crates/raster/src/memory/system.rs", '#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]', '#[cfg(not(any(target_os = "linux", target_os = "android", windows, target_os = "macos")))]')
elif app == "vectorcraft":
    if fix("crates/render/src/images.rs", '#[cfg(target_os = "linux")]', '#[cfg(any(target_os = "linux", target_os = "android"))]'):
        fixed.append("crates/render/src/images.rs (Android meminfo)")
    fix_all("crates/render/src/images.rs", '#[cfg(any(target_os = "linux", test))]', '#[cfg(any(target_os = "linux", target_os = "android", test))]')
    fix("crates/render/src/images.rs", '#[cfg(not(any(target_os = "linux", windows, target_os = "macos", target_os = "freebsd")))]', '#[cfg(not(any(target_os = "linux", target_os = "android", windows, target_os = "macos", target_os = "freebsd")))]')
elif app == "lightcraft":
    if fix("crates/engine/src/memory.rs", '#[cfg(target_os = "linux")]', '#[cfg(any(target_os = "linux", target_os = "android"))]'):
        fixed.append("crates/engine/src/memory.rs (Android meminfo)")
    fix("crates/engine/src/memory.rs", '#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "freebsd")))]', '#[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos", target_os = "freebsd")))]')
elif app == "pdfcraft":
    if fix(
        "crates/ui-egui/src/app.rs",
        "            printer_source: None,\n",
        "            printer_source: None,\n            print_handler: None,\n",
    ):
        fixed.append("crates/ui-egui/src/app.rs (print_handler)")
    if fix(
        "crates/ui-egui/src/lib.rs",
        "    pub printer_source: Option<PrinterSource>,\n",
        "    pub printer_source: Option<PrinterSource>,\n    pub print_handler: Option<std::sync::Arc<dyn Fn(&str, &[u8]) -> Result<String, String> + Send + Sync>>,\n",
    ):
        fixed.append("crates/ui-egui/src/lib.rs (print_handler)")
    if fix(
        "crates/ui-egui/src/workflows/print_ui.rs",
        "        let spooler = self.spooler.clone();\n        std::thread::spawn(move || {\n",
        "        let spooler = self.spooler.clone();\n        let handler = self.print_handler.clone();\n        std::thread::spawn(move || {\n",
    ) or fix(
        "crates/ui-egui/src/workflows/print_ui.rs",
        "        let ctx = self.ctx.clone();\n        std::thread::spawn(move || {\n",
        "        let ctx = self.ctx.clone();\n        let handler = self.print_handler.clone();\n        std::thread::spawn(move || {\n",
    ):
        fixed.append("crates/ui-egui/src/workflows/print_ui.rs (handler closure)")
    old_print_spooler = """            let result = match &spooler {
                Some(submit) => submit(&bytes, &job),
                None => spool::submit_with_progress(&bytes, &job, &mut |sent, sheets| {
                    let _ = tx.send(JobNews::Sent(sent, sheets));
                    repaint();
                })
                .map_err(|e| e.to_string()),
            };"""
    new_print_spooler = """            let result = if let Some(handler) = &handler {
                let name = if job.title.is_empty() { "PdfCraft".to_string() } else { job.title.clone() };
                handler(&name, &bytes)
            } else if let Some(submit) = &spooler {
                submit(&bytes, &job)
            } else {
                spool::submit_with_progress(&bytes, &job, &mut |sent, sheets| {
                    let _ = tx.send(JobNews::Sent(sent, sheets));
                    repaint();
                }).map_err(|e| e.to_string())
            };"""
    if fix("crates/ui-egui/src/workflows/print_ui.rs", old_print_spooler, new_print_spooler):
        fixed.append("crates/ui-egui/src/workflows/print_ui.rs (send_to_printer spooler)")
    old_print = """            // The receiver may be gone (the app quit): nothing to report to then.
            let result = spool::submit_with_progress(&bytes, &job, &mut |sent, sheets| {
                let _ = tx.send(JobNews::Sent(sent, sheets));
                repaint();
            });
            let _ = tx.send(JobNews::Done(result.map_err(|e| e.to_string())));"""
    new_print = """            let result = if let Some(handler) = handler {
                let name = if job.title.is_empty() { "PdfCraft".to_string() } else { job.title };
                handler(&name, &bytes)
            } else {
                // The receiver may be gone (the app quit): nothing to report to then.
                spool::submit_with_progress(&bytes, &job, &mut |sent, sheets| {
                    let _ = tx.send(JobNews::Sent(sent, sheets));
                    repaint();
                }).map_err(|e| e.to_string())
            };
            let _ = tx.send(JobNews::Done(result));"""
    if fix("crates/ui-egui/src/workflows/print_ui.rs", old_print, new_print):
        fixed.append("crates/ui-egui/src/workflows/print_ui.rs (send_to_printer)")


if fixed:
    print(f"  Repaired upstream compile errors still present in {title}: {', '.join(fixed)}")
PY

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
python3 - "$SRC/Cargo.toml" <<'PY'
from pathlib import Path
import sys, tomllib
p = Path(sys.argv[1])
s = p.read_text()
# The apps now ship the storytold egui 0.37 fork, whose egui-winit (vendored under
# vendor/egui/crates/egui-winit or vendor/egui-winit) is the matching winit 0.30 build. Patch crate-io
# so egui-winit is used. Keep the builder's egui-winit vendored as a harmless fallback.
winit_rel = "vendor/egui/crates/egui-winit" if (p.parent / "vendor/egui/crates/egui-winit").is_dir() else "vendor/egui-winit"
entry = f'egui-winit = {{ path = "{winit_rel}" }}'
import re
s = re.sub(r'^\s*egui-winit\s*=\s*\{.*?\}\s*(?:#.*)?\n?', '', s, flags=re.M)
header = "[patch.crates-io]"
if entry not in s:
    s = (s.replace(header, header + "\n" + entry, 1) if header in s
         else s.rstrip() + "\n\n" + header + "\n" + entry + "\n")
git_header = '[patch."https://github.com/storytold/egui.git"]'
if "https://github.com/storytold/egui.git" in s:
    if git_header in s:
        section = s.split(git_header)[1].split("\n[")[0]
        if entry not in section:
            s = s.replace(git_header, git_header + "\n" + entry, 1)
    else:
        s = s.rstrip() + "\n\n" + git_header + "\n" + entry + "\n"
tomllib.loads(s)
p.write_text(s)
PY
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
    python3 - "$SRC/Cargo.toml" <<'PY'
from pathlib import Path
import re, sys, tomllib
p = Path(sys.argv[1])
s = p.read_text()
entry = 'rfd = { path = "vendor/rfd" }'
header = "[patch.crates-io]"
if header in s:
    parts = s.split(header, 1)
    next_sec = re.search(r'\n\[', parts[1])
    patch_sec = parts[1][:next_sec.start()] if next_sec else parts[1]
    rest = parts[1][next_sec.start():] if next_sec else ""
    patch_sec = re.sub(r'^\s*rfd\s*=\s*\{.*?\}\s*(?:#.*)?\n?', '', patch_sec, flags=re.M)
    s = parts[0] + header + "\n" + entry + "\n" + patch_sec.lstrip('\n') + rest
else:
    s = s.rstrip() + "\n\n" + header + "\n" + entry + "\n"
tomllib.loads(s)
p.write_text(s)
PY
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
    python3 "$ROOT/fix_arboard.py" "$SRC/Cargo.toml" "$SRC"
fi

# midir on Android needs a platform adapter or dummy backend. Vendor the builder's Android-compatible
# midir stub (with dummy MIDI backend) and patch it into the workspace when midir is in dependencies.
if [[ -d "$ROOT/vendor/midir" ]] && (grep -rqE '\bmidir\b' "$SRC"/crates "$SRC"/apps "$SRC"/src 2>/dev/null || grep -qE '^\s*midir\s*=' "$SRC/Cargo.toml" 2>/dev/null); then
    echo "Vendoring an Android-compatible midir for $TITLE (dummy MIDI backend)..."
    mkdir -p "$SRC/vendor"
    rm -rf "$SRC/vendor/midir"
    cp -a "$ROOT/vendor/midir" "$SRC/vendor/midir"
    python3 - "$SRC/Cargo.toml" <<'PY'
from pathlib import Path
import re, sys, tomllib
p = Path(sys.argv[1])
s = p.read_text()
entry = 'midir = { path = "vendor/midir" }'
header = "[patch.crates-io]"
if header in s:
    parts = s.split(header, 1)
    next_sec = re.search(r'\n\[', parts[1])
    patch_sec = parts[1][:next_sec.start()] if next_sec else parts[1]
    rest = parts[1][next_sec.start():] if next_sec else ""
    patch_sec = re.sub(r'^\s*midir\s*=\s*\{.*?\}\s*(?:#.*)?\n?', '', patch_sec, flags=re.M)
    s = parts[0] + header + "\n" + entry + "\n" + patch_sec.lstrip('\n') + rest
else:
    s = s.rstrip() + "\n\n" + header + "\n" + entry + "\n"
tomllib.loads(s)
p.write_text(s)
PY
fi

# rustls-platform-verifier on Android delegates to Kotlin `org.rustls.platformverifier.CertificateVerifier`.
# In native Activity APKs, this Kotlin class is missing and JNI is uninitialized, breaking all TLS connections.
# Vendor the builder's WebPKI-backed rustls-platform-verifier (Mozilla root certs directly in Rust) and patch it in.
if [[ -d "$ROOT/vendor/rustls-platform-verifier" ]] && (grep -rqE '\brustls-platform-verifier\b' "$SRC/Cargo.lock" "$SRC"/Cargo.toml 2>/dev/null); then
    echo "Vendoring an Android-compatible rustls-platform-verifier for $TITLE (WebPKI Mozilla root certs)..."
    mkdir -p "$SRC/vendor"
    rm -rf "$SRC/vendor/rustls-platform-verifier"
    cp -a "$ROOT/vendor/rustls-platform-verifier" "$SRC/vendor/rustls-platform-verifier"
    python3 - "$SRC/Cargo.toml" <<'PY'
from pathlib import Path
import re, sys, tomllib
p = Path(sys.argv[1])
s = p.read_text()
entry = 'rustls-platform-verifier = { path = "vendor/rustls-platform-verifier" }'
header = "[patch.crates-io]"
if header in s:
    parts = s.split(header, 1)
    next_sec = re.search(r'\n\[', parts[1])
    patch_sec = parts[1][:next_sec.start()] if next_sec else parts[1]
    rest = parts[1][next_sec.start():] if next_sec else ""
    patch_sec = re.sub(r'^\s*rustls-platform-verifier\s*=\s*\{.*?\}\s*(?:#.*)?\n?', '', patch_sec, flags=re.M)
    s = parts[0] + header + "\n" + entry + "\n" + patch_sec.lstrip('\n') + rest
else:
    s = s.rstrip() + "\n\n" + header + "\n" + entry + "\n"
tomllib.loads(s)
p.write_text(s)
PY
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
python3 - "$SRC" <<'PY'
from pathlib import Path
import re, sys
src = Path(sys.argv[1])
event_rs = src / "vendor" / "winit" / "src" / "event.rs"
window_rs = src / "vendor" / "winit" / "src" / "window.rs"
mod_rs = src / "vendor" / "winit" / "src" / "platform_impl" / "android" / "mod.rs"
if not event_rs.is_file() or not window_rs.is_file() or not mod_rs.is_file():
    sys.exit(0)

changed = False

# Ensure ImeResetCompleted is in event.rs
ev = event_rs.read_text()
if "ImeResetCompleted" not in ev:
    anchor = "    Ime(Ime),\n"
    if anchor in ev:
        ev = ev.replace(anchor, anchor + '''\
    /// The ordered cancellation fence requested by [`Window::reset_ime_with_request`].
    ///
    /// ## Platform-specific
    ///
    /// - **Android:** No-op; Android drives its own input-method pipeline.
    ImeResetCompleted { request_id: u64 },
''')
        event_rs.write_text(ev)
        changed = True

# Ensure reset_ime_with_request is in window.rs
wn = window_rs.read_text()
if "fn reset_ime_with_request" not in wn:
    import re as _re
    m = _re.search(r"set_ime_purpose\(purpose\)\)\s*\n    \}\n", wn)
    if m:
        end = m.end()
        wn = wn[:end] + '''
    /// Requests an IME reset, identifying the request with `request_id`. No-op on Android.
    #[inline]
    pub fn reset_ime_with_request(&self, _request_id: u64) {}
''' + wn[end:]
        window_rs.write_text(wn)
        changed = True

# Ensure reset_ime_with_request is in Android mod.rs
text = mod_rs.read_text()
text = re.sub(r'    pub fn reset_ime_with_request\(&self, _request_id: u64\) \{\}\n', '', text)
text = text.replace(
    '    pub fn set_ime_purpose(&self, _purpose: ImePurpose) {}\n'
    '    pub fn set_ime_purpose(&self, _purpose: ImePurpose) {}\n',
    '    pub fn set_ime_purpose(&self, _purpose: ImePurpose) {}\n',
)
if "fn reset_ime_with_request" not in text:
    marker = "    pub fn set_ime_purpose(&self, _purpose: ImePurpose) {}\n"
    idx = text.find(marker)
    if idx >= 0:
        insert = '''
    /// No-op on Android: the platform drives its own input-method pipeline and does not use the
    /// winit IME-reset protocol.
    pub fn reset_ime_with_request(&self, _request_id: u64) {}
'''
        text = text[:idx+len(marker)] + insert + text[idx+len(marker):]
        mod_rs.write_text(text)
        changed = True

# Also ensure any egui-winit copy present covers ImeResetCompleted in its match statements
for ew_path in (
    src / "vendor" / "egui" / "crates" / "egui-winit" / "src" / "lib.rs",
    src / "vendor" / "egui-winit" / "src" / "lib.rs",
):
    if ew_path.is_file():
        ew = ew_path.read_text()
        if "ImeResetCompleted" not in ew:
            target1 = "WindowEvent::DoubleTapGesture { .. } => EventResponse {"
            repl1 = "WindowEvent::DoubleTapGesture { .. }\n            | WindowEvent::ImeResetCompleted { .. } => EventResponse {"
            if target1 in ew:
                ew = ew.replace(target1, repl1, 1)
                changed = True

            target2 = 'WindowEvent::ActivationTokenDone { .. } => "WindowEvent::ActivationTokenDone",\n'
            repl2 = target2 + '        WindowEvent::ImeResetCompleted { .. } => "WindowEvent::ImeResetCompleted",\n'
            if target2 in ew:
                ew = ew.replace(target2, repl2, 1)
                changed = True

            if changed:
                ew_path.write_text(ew)

if changed:
    print("  Synced winit IME-reset support with fork egui-winit")
PY

# egui_dock (docking system for egui) vendors egui itself. Subcraft depends on egui_dock 0.21
# which pulls crates.io egui 0.36 into the graph alongside the builder's vendored egui 0.37,
# causing every `egui_dock::egui::Id` / `egui::Id` mismatch. Vendor egui_dock and point its
# egui dependency at the vendored egui 0.37 so both egui_dock and the app share one egui.
if grep -rqE '\begui_dock\b' "$SRC"/crates/*/src "$SRC"/apps/*/src 2>/dev/null; then
    echo "Vendoring egui_dock against the vendored egui for $TITLE..."
    mkdir -p "$SRC/vendor"
    rm -rf "$SRC/vendor/egui_dock"
    cp -a "$ROOT/vendor/egui_dock" "$SRC/vendor/egui_dock"
    python3 - "$SRC" <<'PY'
from pathlib import Path
import re, sys
src = Path(sys.argv[1])
ws_toml = src / "Cargo.toml"
ws_text = ws_toml.read_text()
new_ws, n = re.subn(
    r'^\s*egui_dock\s*=\s*\{[^}]*\}\s*(?:#.*)?\n?',
    'egui_dock = { path = "vendor/egui_dock", features = ["serde"] }\n',
    ws_text,
    flags=re.M,
)
if n > 0:
    ws_toml.write_text(new_ws)
    print("  Replaced egui_dock with vendored path in [workspace.dependencies]")
elif 'egui_dock = { path = "vendor/egui_dock"' not in ws_text:
    marker = "[workspace.dependencies]\n"
    idx = ws_text.find(marker)
    if idx >= 0:
        ws_text = (ws_text[:idx + len(marker)]
                   + 'egui_dock = { path = "vendor/egui_dock", features = ["serde"] }\n'
                   + ws_text[idx + len(marker):])
        ws_toml.write_text(ws_text)
        print("  Added egui_dock to [workspace.dependencies]")
PY
fi

# subcraft's fork egui-winit references accesskit_winit via a relative path that resolves to a
# non-existent nested `vendor/egui/vendor/winit`. Point it at the top-level vendored winit which
# the builder keeps in sync.
python3 - "$SRC" <<'PY'
from pathlib import Path
import re, sys
src = Path(sys.argv[1])
efw = src / "vendor" / "egui" / "crates" / "egui-winit" / "Cargo.toml"
if not efw.is_file():
    sys.exit(0)
text = efw.read_text()
# The storytold fork sometimes stores a broken relative path (`../../vendor/winit` or a mangled
# `../..../winit`) that resolves outside the crate tree. Point it at the top-level vendored winit
# (`../../../winit` from `vendor/egui/crates/egui-winit/`).
new = re.sub(r'path = "[^"]*/winit"', 'path = "../../../winit"', text)
# subcraft's fork egui-winit points accesskit_winit at the winit crate directory (wrong -
# winit is not accesskit_winit). Use the workspace's crates.io accesskit_winit instead.
new = new.replace('accesskit_winit = { path = "../../../winit", optional = true }',
                  'accesskit_winit = { workspace = true, optional = true }')
if new != text:
    efw.write_text(new)
    print("  Fixed subcraft fork egui-winit accesskit_winit path")
PY

python3 - "$SRC/Cargo.toml" <<'PY'
from pathlib import Path
import sys, tomllib
p = Path(sys.argv[1])
s = p.read_text()
entry = 'winit = { path = "vendor/winit" }'
# Remove any existing winit patch entry to avoid duplicate keys
import re
# Remove lines starting with winit = (possibly with whitespace) within patch.crates-io section
# Simple approach: remove all occurrences of lines that look like winit patch
s = re.sub(r'^\s*winit\s*=\s*\{.*?\}\s*(?:#.*)?\n?', '', s, flags=re.M)
if entry not in s:
    header = "[patch.crates-io]"
    s = (s.replace(header, header + "\n" + entry, 1) if header in s
         else s.rstrip() + "\n\n" + header + "\n" + entry + "\n")
    tomllib.loads(s)
    p.write_text(s)
PY

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
    python3 - "$wgpu_target" <<'PY'
from pathlib import Path
import sys
winit_rs = Path(sys.argv[1]) / "src" / "winit.rs"
if not winit_rs.is_file():
    sys.exit(0)
text = winit_rs.read_text()
if 'cfg!(target_os = "android")' not in text:
    target = """        let Some(old_state) = self.surfaces.remove(&viewport_id) else {
            return Ok(());
        };"""
    replacement = """        let Some(old_state) = self.surfaces.remove(&viewport_id) else {
            return Ok(());
        };

        if cfg!(target_os = "android") {
            log::info!("egui-wgpu: dropping the lost surface of {viewport_id:?} (Android)");
            return Ok(());
        }"""
    if target in text:
        text = text.replace(target, replacement, 1)
        winit_rs.write_text(text)
        print("  Patched existing vendored egui-wgpu recreate_surface for Android surface handling")
else:
    buggy = """        let (requested, resizing, mut retry) = (old_state.requested, old_state.resizing, old_state.retry);

        if cfg!(target_os = "android") {
            log::info!("egui-wgpu: dropping the lost surface of {viewport_id:?} (Android)");
            return Ok(());
        }

        self.instance.poll_all(true);

        drop(old_state);"""
    clean = """        if cfg!(target_os = "android") {
            log::info!("egui-wgpu: dropping the lost surface of {viewport_id:?} (Android)");
            return Ok(());
        }"""
    if buggy in text:
        text = text.replace(buggy, clean)
    if "let mut retry = old_state.retry" not in text and "retry.recreated();" in text:
        text = text.replace("retry.recreated();", "let mut retry = old_state.retry;\n        retry.recreated();", 1)
    if "requested.0" in text:
        text = text.replace("requested.0,\n            requested.1,\n            resizing,\n            retry,", "old_state.requested.0,\n            old_state.requested.1,\n            old_state.resizing,\n            retry,")
    winit_rs.write_text(text)
    print("  Cleaned up egui-wgpu recreate_surface patch for Android surface handling")
PY
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
python3 - "$SRC/Cargo.toml" <<'PY'
from pathlib import Path
import sys, tomllib
p = Path(sys.argv[1])
s = p.read_text()
entry = 'egui-wgpu = { path = "vendor/egui-wgpu" }'
import re
s = re.sub(r'^\s*egui-wgpu\s*=\s*\{.*?\}\s*(?:#.*)?\n?', '', s, flags=re.M)
header = "[patch.crates-io]"
if entry not in s:
    s = (s.replace(header, header + "\n" + entry, 1) if header in s
         else s.rstrip() + "\n\n" + header + "\n" + entry + "\n")
git_header = '[patch."https://github.com/storytold/egui.git"]'
if "https://github.com/storytold/egui.git" in s:
    if git_header in s:
        section = s.split(git_header)[1].split("\n[")[0]
        if entry not in section:
            s = s.replace(git_header, git_header + "\n" + entry, 1)
    else:
        s = s.rstrip() + "\n\n" + git_header + "\n" + entry + "\n"
tomllib.loads(s)
p.write_text(s)
PY
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
python3 - "$SRC/Cargo.toml" <<'PY'
from pathlib import Path
import sys, tomllib
p = Path(sys.argv[1])
s = p.read_text()
entry = 'android-activity = { path = "vendor/android-activity" }'
import re
s = re.sub(r'^\s*android-activity\s*=\s*\{.*?\}\s*(?:#.*)?\n?', '', s, flags=re.M)
if entry not in s:
    header = "[patch.crates-io]"
    s = (s.replace(header, header + "\n" + entry, 1) if header in s
         else s.rstrip() + "\n\n" + header + "\n" + entry + "\n")
    tomllib.loads(s)
    p.write_text(s)
PY

# sys-locale on Android reads persist.sys.locale via libc, which modern Android SELinux rejects.
# Vendor the builder's patched sys-locale (which inspects LC_ALL, LANG, LANGUAGE first).
if grep -rqE '\bsys_locale\b' "$SRC"/crates/*/src "$SRC"/apps/*/src 2>/dev/null || grep -qE '^\s*sys-locale\s*=' "$SRC/Cargo.toml" 2>/dev/null; then
    echo "Vendoring an Android SELinux-safe sys-locale for $TITLE..."
    mkdir -p "$SRC/vendor"
    rm -rf "$SRC/vendor/sys-locale"
    cp -a "$ROOT/vendor/sys-locale" "$SRC/vendor/sys-locale"
    python3 - "$SRC/Cargo.toml" <<'PY'
from pathlib import Path
import re, sys, tomllib
p = Path(sys.argv[1])
s = p.read_text()
entry = 'sys-locale = { path = "vendor/sys-locale" }'
header = "[patch.crates-io]"
if header in s:
    parts = s.split(header, 1)
    next_sec = re.search(r'\n\[', parts[1])
    patch_sec = parts[1][:next_sec.start()] if next_sec else parts[1]
    rest = parts[1][next_sec.start():] if next_sec else ""
    patch_sec = re.sub(r'^\s*sys-locale\s*=\s*\{.*?\}\s*(?:#.*)?\n?', '', patch_sec, flags=re.M)
    s = parts[0] + header + "\n" + entry + "\n" + patch_sec.lstrip('\n') + rest
else:
    s = s.rstrip() + "\n\n" + header + "\n" + entry + "\n"
tomllib.loads(s)
p.write_text(s)
PY
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
python3 - "$SRC" "$APP" <<'PY'
from pathlib import Path
import re, sys
src, app = Path(sys.argv[1]), sys.argv[2]
for key, relpaths in [
    ("egui-wgpu", ["vendor/egui-wgpu", "vendor/egui/crates/egui-wgpu"]),
    ("egui-winit", ["vendor/egui-winit", "vendor/egui/crates/egui-winit"]),
    ("winit", ["vendor/winit"]),
    ("arboard", ["vendor/arboard"]),
]:
    try:
        text = (src / "Cargo.toml").read_text()
    except OSError:
        continue
    if "[workspace.dependencies]" not in text:
        continue
    section = text.split("[workspace.dependencies]", 1)[1].split("\n[", 1)[0]
    if re.search(rf"^\s*{re.escape(key)}\s*=", section, flags=re.M):
        continue
    relpath = next((p for p in relpaths if (src / p).is_dir()), None)
    if not relpath:
        continue
    marker = "[workspace.dependencies]\n"
    idx = text.find(marker)
    if idx >= 0:
        text = (text[:idx + len(marker)]
                + f'{key} = {{ path = "{relpath}" }}\n'
                + text[idx + len(marker):])
        (src / "Cargo.toml").write_text(text)
        print(f"  Added {key} to [workspace.dependencies]")
PY

# The storytold winit fork enables desktop wayland by default, which pulls sctk-adwaita ->
# crossfont -> fontconfig-sys v5 into the resolution graph. That collides with
# egui_system_font_provider -> fontique -> fontconfig-sys v6 (both `links = "fontconfig"`), and
# cargo refuses to build. Wayland is never used on Android, so empty the wayland feature family
# (drop from default + empty the definitions) so crossfont is never pulled. The ImeReset variant
# that egui-winit 0.37 needs lives in winit's source (untouched here), so fork egui-winit keeps
# compiling against fork winit.
python3 - "$SRC" <<'PY'
from pathlib import Path
import re, sys
src = Path(sys.argv[1])
winit_toml = src / "vendor" / "winit" / "Cargo.toml"
if not winit_toml.is_file():
    sys.exit(0)
text = winit_toml.read_text()
for name in ("wayland", "wayland-dlopen", "wayland-csd-adwaita"):
    text = re.sub(rf'(?m)^\s*"{re.escape(name)}",\s*\n', '', text)
for name in ("wayland", "wayland-dlopen", "wayland-csd-adwaita", "wayland-csd-adwaita-crossfont", "wayland-csd-adwaita-notitle"):
    text = re.sub(
        rf'(?ms)^(\s*{re.escape(name)}\s*=\s*\[\s*\n)(.*?)^(\s*\]\s*\n)',
        r'\1\3',
        text,
    )
for name in ("wayland-dlopen", "wayland-csd-adwaita-notitle"):
    text = re.sub(
        rf'(?m)^(\s*{re.escape(name)}\s*=\s*\[)[^\]]*(\]\s*)$',
        r'\1\2',
        text,
    )
winit_toml.write_text(text)
print("  Emptied wayland features in vendor/winit/Cargo.toml (fontconfig links conflict)")
PY

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
python3 - <<'WGPUPY'
from pathlib import Path
import os, stat

cargo_home = Path(os.environ.get("CARGO_HOME", os.path.expanduser("~/.cargo")))
search_dirs = [cargo_home / "registry" / "src", Path.home() / ".cargo" / "registry" / "src"]

for base in search_dirs:
    if not base.exists():
        continue
    # 1. Patch wgpu-hal swapchain latency calculation
    for p in base.glob("**/wgpu-hal-*/src/vulkan/swapchain/native.rs"):
        try:
            p.chmod(stat.S_IRUSR | stat.S_IWUSR | stat.S_IRGRP | stat.S_IROTH)
            txt = p.read_text()
            old = "maximum_frame_latency: (caps.min_image_count - 1)..=(max_image_count - 1)"
            if old in txt:
                new = (
                    "maximum_frame_latency: {\n"
                    "                let min_lat = caps.min_image_count.saturating_sub(1);\n"
                    "                let max_lat = if max_image_count == 0 { min_lat.max(2) } else { max_image_count.saturating_sub(1) };\n"
                    "                let (lo, hi) = if min_lat <= max_lat { (min_lat, max_lat) } else { (max_lat, min_lat) };\n"
                    "                lo..=hi\n"
                    "            }"
                )
                p.write_text(txt.replace(old, new))
                print(f"  -> patched wgpu-hal swapchain latency in {p.name}")
        except Exception as e:
            pass

    # 2. Patch wgpu-core clamp panic guard
    for p in base.glob("**/wgpu-core-*/src/device/resource.rs"):
        try:
            p.chmod(stat.S_IRUSR | stat.S_IWUSR | stat.S_IRGRP | stat.S_IROTH)
            txt = p.read_text()
            old = (
                "                let maximum_frame_latency = config.desired_maximum_frame_latency.clamp(\n"
                "                    *caps.maximum_frame_latency.start(),\n"
                "                    *caps.maximum_frame_latency.end(),\n"
                "                );"
            )
            if old in txt:
                new = (
                    "                let max_lat_lo = *caps.maximum_frame_latency.start();\n"
                    "                let max_lat_hi = (*caps.maximum_frame_latency.end()).max(max_lat_lo);\n"
                    "                let maximum_frame_latency = config.desired_maximum_frame_latency.clamp(\n"
                    "                    max_lat_lo,\n"
                    "                    max_lat_hi,\n"
                    "                );"
                )
                p.write_text(txt.replace(old, new))
                print(f"  -> patched wgpu-core clamp guard in {p.name}")
        except Exception as e:
            pass

    # 3. Patch wgpu-hal to enable VK_ANDROID_external_memory_android_hardware_buffer
    for p in base.glob("**/wgpu-hal-*/src/vulkan/adapter.rs"):
        try:
            p.chmod(stat.S_IRUSR | stat.S_IWUSR | stat.S_IRGRP | stat.S_IROTH)
            txt = p.read_text()
            old = "if self.supports_extension(ext::external_memory_dma_buf::NAME) {"
            if old in txt and "VK_ANDROID_external_memory_android_hardware_buffer" not in txt:
                new = (
                    'const ANDROID_HWB_EXT: &std::ffi::CStr = c"VK_ANDROID_external_memory_android_hardware_buffer";\n'
                    '        if self.supports_extension(ANDROID_HWB_EXT) {\n'
                    '            extensions.push(ANDROID_HWB_EXT);\n'
                    '        }\n'
                    '        if self.supports_extension(ext::external_memory_dma_buf::NAME) {'
                )
                p.write_text(txt.replace(old, new, 1))
                print(f"  -> patched wgpu-hal android_hardware_buffer extension in {p.name}")
        except Exception as e:
            pass
WGPUPY

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

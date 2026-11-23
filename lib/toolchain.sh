#!/usr/bin/env bash
# Android SDK, NDK, JDK and toolchain discovery and bootstrap helpers.
# Extracted from lib/build-common.sh as part of modularization (issue #73).

command -v fail >/dev/null 2>&1 || fail() { echo "ERROR: $*" >&2; exit 1; }
command -v say >/dev/null 2>&1 || say() { echo "==> $*"; }

# Apps whose launcher (adapter or builder-generated) ships a Java Storage Access Framework Activity.
# Kept here in addition to the manifest check below so bridge verification covers the very first
# build too, before the launcher has been generated. Keep in sync with SAF_RECIPES in this file.
SAF_APPS=" designcraft deckcraft gridcraft wordcraft lightcraft effectcraft filmcraft cadcraft soundcraft pdfcraft figcraft subcraft solidcraft materialcraft texturecraft notecraft diagramcraft paintcraft bimcraft tracedraw picassosuite capsulecraft humanmotioncraft rustgis earthcraft mapcraft prowave transit anicraft coimanide pcbcraft fieldcraft papercraft ferrite pixelcraft clothcraft "
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

check_cargo_ndk() {
    # The build compiles the native library with cargo-ndk (a jniLibs tree) and packages that tree,
    # the Java Activity and the generated manifest with Gradle (see below). cargo-apk/cargo-apk2 are
    # no longer invoked.
    if ! cargo ndk --help >/dev/null 2>&1; then
        echo "cargo-ndk is needed to build $TITLE's native Android library. Installing it..."
        cargo install cargo-ndk --locked || fail "Could not install cargo-ndk. Install it manually with: cargo install cargo-ndk --locked"
    fi
    cargo ndk -V >/dev/null 2>&1 || fail "cargo-ndk is required but not working. Install it with: cargo install cargo-ndk --locked"
    export PERL5LIB="$ROOT/lib/perl5:${PERL5LIB:-}"
}

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


init_toolchain() {
    check_cargo_ndk
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
}

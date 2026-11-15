#!/usr/bin/env bash
# Build ArtCraft Launcher's Android APK (arm64-v8a): the Rust library with cargo-ndk, then the APK
# with Gradle. Needs ANDROID_HOME (SDK with platform 35 and build-tools), ANDROID_NDK_HOME (r27+),
# JDK 17, `rustup target add aarch64-linux-android` and `cargo install cargo-ndk`.
#
#   apps/artcraft-launcher-android/build.sh            # release APK
#   PROFILE=dev apps/artcraft-launcher-android/build.sh  # unoptimised library, faster to build
#   RELEASE_BUILD=3 apps/artcraft-launcher-android/build.sh  # the 3rd Android release of this version
#
# The APK lands in target/android/artcraft-launcher-<version>[-<build>]-arm64-v8a.apk. A release of it
# is tagged android-v<version>[-<build>] with the APK attached: installed copies offer to update
# to it (android/app/src/main/java/.../Updater.java).

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
PROFILE="${PROFILE:-release}"
API="${ANDROID_API:-24}"

: "${ANDROID_HOME:?set ANDROID_HOME to the Android SDK}"
: "${ANDROID_NDK_HOME:?set ANDROID_NDK_HOME to the Android NDK (r27 or newer)}"
command -v cargo-ndk >/dev/null || { echo "error: cargo-ndk not found (cargo install cargo-ndk)" >&2; exit 1; }

FEATURE_ARGS=()
if [[ -n "${FEATURES:-}" ]]; then
    FEATURE_ARGS=(--features "${FEATURES}")
fi

# `cargo build` has no `--dev`: the dev profile is the default, only release needs a flag.
PROFILE_ARGS=()
if [[ "$PROFILE" == "release" ]]; then
    PROFILE_ARGS=(--release)
fi

# Read workspace version from root Cargo.toml
VERSION=$(grep '^version' "$ROOT/Cargo.toml" | head -1 | sed -E 's/.*"([^"]+)".*/\1/')
RELEASE_BUILD="${RELEASE_BUILD:-0}"
if [[ "$RELEASE_BUILD" -gt 0 ]]; then
    APK_VERSION="${VERSION}-${RELEASE_BUILD}"
    TAG="android-v${VERSION}-${RELEASE_BUILD}"
else
    APK_VERSION="${VERSION}"
    TAG="android-v${VERSION}"
fi

echo "Building ArtCraft Launcher ${APK_VERSION} (${PROFILE}) for arm64-v8a"

# ---- 1. Build the Rust library with cargo-ndk -------------------------------

cd "$HERE"

# Keep src/catalog.rs in sync with the single-file app catalog before compiling.
if [[ -f "$HERE/app-catalog.json" ]]; then
    "$HERE/generate-catalog.sh"
    "$HERE/generate-about.sh"
fi

# Build for Android arm64
cargo ndk -t arm64-v8a -o "$HERE/android/app/src/main/jniLibs" \
    --platform "$API" --manifest-path "$HERE/Cargo.toml" build ${FEATURE_ARGS[@]} ${PROFILE_ARGS[@]}

# The cdylib ends up in jniLibs/arm64-v8a/libartcraft_launcher_android.so
# Verify it exists
if [[ ! -f "$HERE/android/app/src/main/jniLibs/arm64-v8a/libartcraft_launcher_android.so" ]]; then
    echo "error: Rust library not found at jniLibs/arm64-v8a/libartcraft_launcher_android.so" >&2
    exit 1
fi

# Strip debug symbols for release builds (smaller APK)
if [[ "$PROFILE" == "release" ]]; then
    "$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-strip" \
        "$HERE/android/app/src/main/jniLibs/arm64-v8a/libartcraft_launcher_android.so" \
        || true
fi

# ---- 2. Build the APK with Gradle ------------------------------------------

cd "$HERE/android"

# Set the version for Gradle
export APP_VERSION="${VERSION}"
export RELEASE_BUILD="${RELEASE_BUILD}"

./gradlew assembleRelease --no-daemon

# ---- 3. Copy APK to target/android -----------------------------------------

OUT_DIR="$ROOT/target/android"
mkdir -p "$OUT_DIR"

APK_SOURCE="$HERE/android/app/build/outputs/apk/release/app-release.apk"
APK_DEST="$OUT_DIR/artcraft-launcher-${APK_VERSION}-arm64-v8a.apk"

cp "$APK_SOURCE" "$APK_DEST"
echo "APK ready: $APK_DEST"

# Print SHA256
sha256sum "$APK_DEST"
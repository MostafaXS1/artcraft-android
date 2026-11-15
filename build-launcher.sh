# Build ArtCraft Launcher's Android APK and output to output/Artcraft-launcher.apk
#
# Usage:
#   ./build-launcher.sh              # release APK
#   PROFILE=dev ./build-launcher.sh  # dev APK (faster, unoptimized)
#   RELEASE_BUILD=3 ./build-launcher.sh  # 3rd Android release of this version

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LAUNCHER_DIR="$SCRIPT_DIR/launcher/apps/artcraft-launcher-android"
OUT_DIR="$SCRIPT_DIR/output"
OUT_APK="$OUT_DIR/Artcraft-launcher.apk"

# Export environment for the build script
export ANDROID_HOME="${ANDROID_HOME:-$SCRIPT_DIR/android-sdk}"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$ANDROID_HOME/ndk/27.2.12479018}"

# Run the launcher's build script
cd "$LAUNCHER_DIR"
PROFILE="${PROFILE:-release}" RELEASE_BUILD="${RELEASE_BUILD:-0}" ./build.sh

# Find the built APK
VERSION=$(grep '^version' "$SCRIPT_DIR/launcher/Cargo.toml" | head -1 | sed -E 's/.*"([^"]+)".*/\1/')
RELEASE_BUILD="${RELEASE_BUILD:-0}"
if [[ "$RELEASE_BUILD" -gt 0 ]]; then
    APK_VERSION="${VERSION}-${RELEASE_BUILD}"
else
    APK_VERSION="${VERSION}"
fi

BUILT_APK="$SCRIPT_DIR/launcher/target/android/artcraft-launcher-${APK_VERSION}-arm64-v8a.apk"

if [[ ! -f "$BUILT_APK" ]]; then
    echo "error: built APK not found at $BUILT_APK" >&2
    exit 1
fi

# Copy to output folder as Artcraft-launcher.apk
mkdir -p "$OUT_DIR"
cp "$BUILT_APK" "$OUT_APK"

echo "✓ APK ready: $OUT_APK"
sha256sum "$OUT_APK"
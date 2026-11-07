#!/usr/bin/env bash
set -Eeuo pipefail
APP="${1:-picassosuite}"
TITLE="${2:-Picasso Suite}"
SRC="${3:-source/picassosuite}"
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
SRC_ABS="$ROOT/$SRC"
ADAPTER="$ROOT/adapters/picassosuite"
DEST="$SRC_ABS/apps/$APP-android"

# Build directory for external encoder libraries
FFMPEG_DEPS_PREFIX="$SRC_ABS/ffmpeg-deps"
mkdir -p "$FFMPEG_DEPS_PREFIX"

# Android NDK toolchain setup
NDK_PATH="$ROOT/android-sdk/ndk/27.2.12479018"
TOOLCHAIN="$NDK_PATH/toolchains/llvm/prebuilt/linux-x86_64"
TARGET_ARCH="aarch64-linux-android"
API_LEVEL="28"
CC="$TOOLCHAIN/bin/${TARGET_ARCH}${API_LEVEL}-clang"
CXX="$TOOLCHAIN/bin/${TARGET_ARCH}${API_LEVEL}-clang++"
AR="$TOOLCHAIN/bin/llvm-ar"
RANLIB="$TOOLCHAIN/bin/llvm-ranlib"
STRIP="$TOOLCHAIN/bin/llvm-strip"
STRINGS="$TOOLCHAIN/bin/llvm-strings"
export PATH="$TOOLCHAIN/bin:$PATH"
# Create symlinks for binutils that x264 configure expects
for tool in ar ranlib strip strings nm objdump objcopy; do
    if [[ ! -f "$TOOLCHAIN/bin/${TARGET_ARCH}-$tool" ]]; then
        ln -sf "llvm-$tool" "$TOOLCHAIN/bin/${TARGET_ARCH}-$tool" 2>/dev/null || true
    fi
done
export PKG_CONFIG_PATH="${FFMPEG_DEPS_PREFIX}/lib/pkgconfig:${PKG_CONFIG_PATH:-}"
export PKG_CONFIG_LIBDIR="${FFMPEG_DEPS_PREFIX}/lib/pkgconfig"

# Function to build a library for Android
build_lib() {
    local name="$1"
    local url="$2"
    local version="$3"
    local configure_args="${4:-}"
    local build_dir="$SRC_ABS/build-$name"

    echo "Building $name $version..."
    rm -rf "$build_dir"
    mkdir -p "$build_dir"
    cd "$build_dir"

    # Download and extract
    if [[ "$url" == *.git ]]; then
        git clone --depth 1 --branch "$version" "$url" .
    else
        curl -L "$url" | tar xz --strip-components=1
    fi

    # Set up cross-compilation environment
    export CC="$TOOLCHAIN/bin/${TARGET_ARCH}${API_LEVEL}-clang"
    export CXX="$TOOLCHAIN/bin/${TARGET_ARCH}${API_LEVEL}-clang++"
    export AR="$TOOLCHAIN/bin/llvm-ar"
    export RANLIB="$TOOLCHAIN/bin/llvm-ranlib"
    export STRIP="$TOOLCHAIN/bin/llvm-strip"
    export CFLAGS="-fPIC"
    export CXXFLAGS="-fPIC"
    export LDFLAGS=""

    # Configure and build - each library has different configure options
    case "$name" in
        x264)
            ./configure \
                --prefix="$FFMPEG_DEPS_PREFIX" \
                --host="$TARGET_ARCH" \
                --cross-prefix="${TARGET_ARCH}-" \
                --enable-static \
                --disable-cli \
                --enable-pic \
                --disable-asm \
                $configure_args
            ;;
        libvpx)
            # libvpx uses a different configure system
            export AS="$CC"
            ./configure \
                --prefix="$FFMPEG_DEPS_PREFIX" \
                --target=arm64-android-gcc \
                --disable-examples \
                --disable-tools \
                --disable-docs \
                --enable-vp9-highbitdepth \
                --as=auto \
                --enable-pic \
                --disable-shared \
                --enable-static \
                $configure_args
            ;;
        fdk-aac)
            # fdk-aac uses autogen.sh + configure
            if [[ -f "autogen.sh" ]]; then
                ./autogen.sh
            fi
            # Patch fdk-aac to use android/log.h instead of log/log.h (Android NDK difference)
            find . -name "*.cpp" -o -name "*.c" -o -name "*.h" | xargs sed -i 's|#include "log/log.h"|#include <android/log.h>|g' 2>/dev/null || true
            find . -name "*.cpp" -o -name "*.c" -o -name "*.h" | xargs sed -i 's|#include <log/log.h>|#include <android/log.h>|g' 2>/dev/null || true
            if [[ -f "autogen.sh" ]]; then
                ./autogen.sh
            fi
            ./configure \
                --prefix="$FFMPEG_DEPS_PREFIX" \
                --host="$TARGET_ARCH" \
                --enable-static \
                --disable-shared \
                $configure_args
            ;;
        mp3lame)
            ./configure \
                --prefix="$FFMPEG_DEPS_PREFIX" \
                --host="$TARGET_ARCH" \
                --enable-static \
                --disable-shared \
                --disable-frontend \
                $configure_args
            ;;
        opus)
            ./configure \
                --prefix="$FFMPEG_DEPS_PREFIX" \
                --host="$TARGET_ARCH" \
                --enable-static \
                --disable-shared \
                $configure_args
            ;;
        *)
            if [[ -f "configure" ]]; then
                ./configure \
                    --prefix="$FFMPEG_DEPS_PREFIX" \
                    --host="$TARGET_ARCH" \
                    --enable-static \
                    --disable-shared \
                    $configure_args
            elif [[ -f "CMakeLists.txt" ]]; then
                cmake -DCMAKE_INSTALL_PREFIX="$FFMPEG_DEPS_PREFIX" \
                    -DCMAKE_TOOLCHAIN_FILE="$NDK_PATH/build/cmake/android.toolchain.cmake" \
                    -DANDROID_ABI=arm64-v8a \
                    -DANDROID_PLATFORM=android-$API_LEVEL \
                    -DBUILD_SHARED_LIBS=OFF \
                    $configure_args .
            fi
            ;;
    esac

    make -j$(nproc)
    make install
    cd "$ROOT"
}

# Build x264 (essential for H.264 encoding)
if [[ ! -f "$FFMPEG_DEPS_PREFIX/lib/pkgconfig/x264.pc" ]]; then
    build_lib "x264" "https://code.videolan.org/videolan/x264/-/archive/stable/x264-stable.tar.gz" "stable" \
        ""
fi

# Build libvpx (for VP9 encoding)
if [[ ! -f "$FFMPEG_DEPS_PREFIX/lib/pkgconfig/vpx.pc" ]]; then
    build_lib "libvpx" "https://github.com/webmproject/libvpx/archive/refs/tags/v1.14.1.tar.gz" "v1.14.1" \
        "--disable-examples --disable-tools --disable-docs --enable-vp9-highbitdepth --as=auto"
fi

# Skip fdk-aac (uses Android internal APIs not in NDK), use built-in FFmpeg AAC encoder instead
# Skip mp3lame and opus for now - can be added later if needed
# Build x265 (for H.265 encoding) - optional, skipped for now

# Vendor patched ffmpeg-sys-next that handles Android NDK toolchain path correctly
mkdir -p "$SRC_ABS/vendor"
rm -rf "$SRC_ABS/vendor/ffmpeg-sys-next"
cp -a "$ROOT/vendor/ffmpeg-sys-next" "$SRC_ABS/vendor/ffmpeg-sys-next"

# Patch ffmpeg-sys-next build.rs to set PKG_CONFIG_PATH for the configure command
sed -i '/let mut configure = Command::new(&configure_path);/a\
    configure.env("PKG_CONFIG_PATH", "'"$FFMPEG_DEPS_PREFIX"'/lib/pkgconfig");' "$SRC_ABS/vendor/ffmpeg-sys-next/build.rs"

# Enable ffmpeg-next "build" feature and external encoder features for Android cross-compilation and patch ffmpeg-sys-next
python3 - "$SRC_ABS/Cargo.toml" <<'PY'
from pathlib import Path
import sys, tomllib, re
p = Path(sys.argv[1])
s = p.read_text()

# The upstream features don't include "build" or the external encoder features.
# We need "build" to compile FFmpeg from source, and specific encoder features
# like libx264, libmp3lame, libopus, libvpx, etc. for exporting to work.
# Note: ffmpeg-next 7.1.0 uses specific feature names:
# - "build-lib-freebidi" (not "fribidi")
# - "build-zlib" (not "build-lib-zlib")
# Find the ffmpeg-next line and replace its features array.
pattern = r'(ffmpeg-next\s*=\s*\{[^}]*features\s*=\s*\[)([^\]]*)(\])'
# Only include encoder features for libraries we actually built (x264, libvpx)
encoder_features = '"codec", "format", "software-scaling", "software-resampling", "build", "build-lib-x264", "build-lib-vpx", "build-zlib", "build-license-gpl", "build-license-version3", "build-license-nonfree"'
replacement = r'\1' + encoder_features + r'\3'
new_s, count = re.subn(pattern, replacement, s)
if count > 0:
    s = new_s
    print("Updated ffmpeg-next features with build and encoder features")
else:
    print("Could not find ffmpeg-next features to update")
    for i, line in enumerate(s.split('\n')):
        if 'ffmpeg-next' in line and 'features' in line:
            print(f"  Line {i}: {line}")

patch_entry = 'ffmpeg-sys-next = { path = "vendor/ffmpeg-sys-next" }'
if "vendor/ffmpeg-sys-next" not in s:
    header = "[patch.crates-io]"
    s = (s.replace(header, header + "\n" + patch_entry, 1) if header in s
         else s.rstrip() + "\n\n" + header + "\n" + patch_entry + "\n")

tomllib.loads(s)
p.write_text(s)
PY

mkdir -p "$DEST"
cp -a "$ADAPTER/." "$DEST/"
rm -f "$DEST/prepare.sh" "$DEST/README.md"
cp -f "$ROOT/templates/android/system_fonts.rs" "$DEST/src/system_fonts.rs"

# Write .cargo/config.toml to pass PKG_CONFIG_PATH to the build
mkdir -p "$SRC_ABS/.cargo"
cat > "$SRC_ABS/.cargo/config.toml" <<EOF
[build]
# Pass PKG_CONFIG_PATH so ffmpeg-sys-next can find our custom libraries (x264, libvpx)
rustflags = ["-L", "$FFMPEG_DEPS_PREFIX/lib"]
env = { PKG_CONFIG_PATH = "$FFMPEG_DEPS_PREFIX/lib/pkgconfig" }
EOF

python3 "$ROOT/lib/app_icons.py" --src "$SRC_ABS" --app "$APP" --dest "$DEST/res" || true
printf 'Generated by artcraft-android-builder; custom adapter sourced from adapters/picassosuite.\n' > "$DEST/.artcraft-android-builder-generated"

python3 - "$DEST/Cargo.toml" "$ROOT" "$APP" <<'PY'
from pathlib import Path
import sys, tomllib
path, root, app = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]
sys.path.insert(0, str(root / "lib"))
import file_associations
file_associations.patch_cargo_toml(path, app)
tomllib.loads(path.read_text())
PY
echo "Installed Picasso Suite adapter in $DEST"

#!/usr/bin/env bash
# Build a self-contained ffmpeg for Android (arm64), for the apps that shell out to it.
#
# AniCraft's video export runs an ffmpeg program (the `ANICRAFT_FFMPEG` environment
# variable, else `ffmpeg` from the PATH). Android has neither, so without a bundled ffmpeg
# the export cannot work at all.
#
# The build is shared and cached: the result lands in `build/ffmpeg-android` and every app
# that bundles it reuses it, so the (slow) cross-compile is paid once for the whole suite.
#
# Usage: bash lib/ffmpeg_android.sh [install-dir] [install-name]
#   With an install dir, the built program is copied there under `install-name`.
#
# The encoders are the ones AniCraft asks for:
#   .mp4 -> libx264 (H.264) + aac
#   .webm -> libvpx-vp9 (VP9) + libopus
set -Eeuo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
# The cache lives on the workspace filesystem, not on `build/` (a removable drive that cannot
# set file permissions, which the libraries' `make install` needs).
PREFIX="${FFMPEG_ANDROID_PREFIX:-$ROOT/.cache/ffmpeg-android}"
INSTALL_DIR="${1:-}"
INSTALL_NAME="${2:-ffmpeg}"
JOBS="$(nproc 2>/dev/null || echo 4)"

FFMPEG_VERSION="${FFMPEG_ANDROID_VERSION:-7.1.1}"
X264_URL="https://code.videolan.org/videolan/x264/-/archive/stable/x264-stable.tar.gz"
VPX_URL="https://github.com/webmproject/libvpx/archive/refs/tags/v1.14.1.tar.gz"
OPUS_URL="https://github.com/xiph/opus/releases/download/v1.5.2/opus-1.5.2.tar.gz"
FFMPEG_URL="https://ffmpeg.org/releases/ffmpeg-${FFMPEG_VERSION}.tar.xz"

if [[ -x "$PREFIX/bin/ffmpeg" ]]; then
    echo "ffmpeg for Android is already built: $PREFIX/bin/ffmpeg"
else
    # --- The NDK toolchain -------------------------------------------------
    if [[ -n "${ANDROID_NDK_HOME:-}" && -d "$ANDROID_NDK_HOME" ]]; then
        NDK_PATH="$ANDROID_NDK_HOME"
    elif [[ -n "${NDK_HOME:-}" && -d "$NDK_HOME" ]]; then
        NDK_PATH="$NDK_HOME"
    elif [[ -n "${ANDROID_HOME:-}" && -d "$ANDROID_HOME/ndk" ]]; then
        NDK_PATH=$(find "$ANDROID_HOME/ndk" -maxdepth 1 -mindepth 1 -name "27.*" -type d 2>/dev/null | sort -V | tail -n 1)
        if [[ -z "$NDK_PATH" ]]; then
            NDK_PATH=$(find "$ANDROID_HOME/ndk" -maxdepth 1 -mindepth 1 -type d | grep -v "/30\." | sort -V | tail -n 1)
        fi
    elif [[ -d "$ROOT/android-sdk/ndk/27.2.12479018" ]]; then
        NDK_PATH="$ROOT/android-sdk/ndk/27.2.12479018"
    elif [[ -d "$ROOT/android-sdk/ndk" ]]; then
        NDK_PATH=$(find "$ROOT/android-sdk/ndk" -maxdepth 1 -mindepth 1 -type d | sort -V | tail -n 1)
    else
        echo "ERROR: Could not find the Android NDK" >&2
        exit 1
    fi
    TOOLCHAIN="$NDK_PATH/toolchains/llvm/prebuilt/linux-x86_64"
    TARGET_ARCH="aarch64-linux-android"
    API_LEVEL="28"
    export CC="$TOOLCHAIN/bin/${TARGET_ARCH}${API_LEVEL}-clang"
    export CXX="$TOOLCHAIN/bin/${TARGET_ARCH}${API_LEVEL}-clang++"
    export AR="$TOOLCHAIN/bin/llvm-ar"
    export RANLIB="$TOOLCHAIN/bin/llvm-ranlib"
    export STRIP="$TOOLCHAIN/bin/llvm-strip"
    export NM="$TOOLCHAIN/bin/llvm-nm"
    export OBJDUMP="$TOOLCHAIN/bin/llvm-objdump"
    export OBJCOPY="$TOOLCHAIN/bin/llvm-objcopy"
    export PATH="$TOOLCHAIN/bin:$PATH"
    export PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig:${PKG_CONFIG_PATH:-}"
    export PKG_CONFIG_LIBDIR="$PREFIX/lib/pkgconfig"
    # x264's configure looks for ${host}-ar and friends; the NDK ships them as llvm-*.
    for tool in ar ranlib strip strings nm objdump objcopy; do
        if [[ ! -f "$TOOLCHAIN/bin/${TARGET_ARCH}-$tool" ]]; then
            ln -sf "llvm-$tool" "$TOOLCHAIN/bin/${TARGET_ARCH}-$tool" 2>/dev/null || true
        fi
    done

    WORK="${FFMPEG_ANDROID_WORK:-$ROOT/.cache/ffmpeg-android-work}"
    mkdir -p "$WORK" "$PREFIX"

    # --- One library at a time ----------------------------------------------
    build_lib() {
        local name="$1" url="$2" version="$3" configure_args="${4:-}"
        local build_dir="$WORK/$name"
        echo "Building $name for Android..."
        rm -rf "$build_dir"
        mkdir -p "$build_dir"
        cd "$build_dir"
        if [[ "$url" == *.git ]]; then
            git clone --depth 1 --branch "$version" "$url" .
        elif [[ "$url" == *.tar.xz ]]; then
            curl -fsSL "$url" | tar xJ --strip-components=1
        else
            curl -fsSL "$url" | tar xz --strip-components=1
        fi
        export CFLAGS="-fPIC -O2"
        export CXXFLAGS="-fPIC -O2"
        export LDFLAGS=""
        case "$name" in
            x264)
                ./configure \
                    --prefix="$PREFIX" \
                    --host="$TARGET_ARCH" \
                    --cross-prefix="${TARGET_ARCH}-" \
                    --enable-static \
                    --disable-cli \
                    --enable-pic \
                    --disable-asm \
                    $configure_args
                ;;
            libvpx)
                export AS="$CC"
                ./configure \
                    --prefix="$PREFIX" \
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
            opus)
                ./configure \
                    --prefix="$PREFIX" \
                    --host="$TARGET_ARCH" \
                    --enable-static \
                    --disable-shared \
                    --disable-extra-programs \
                    --disable-doc \
                    $configure_args
                ;;
        esac
        make -j"$JOBS"
        make install
        cd "$ROOT"
    }

    build_lib "x264" "$X264_URL" "stable" ""
    build_lib "libvpx" "$VPX_URL" "v1.14.1" ""
    build_lib "opus" "$OPUS_URL" "v1.5.2" ""

    # --- ffmpeg itself, with the program ------------------------------------
    echo "Building ffmpeg $FFMPEG_VERSION for Android..."
    FFMPEG_DIR="$WORK/ffmpeg"
    rm -rf "$FFMPEG_DIR"
    mkdir -p "$FFMPEG_DIR"
    cd "$FFMPEG_DIR"
    curl -fsSL "$FFMPEG_URL" | tar xJ --strip-components=1
    export CFLAGS="-fPIC -O2"
    export CXXFLAGS="-fPIC -O2"
    export LDFLAGS=""
    ./configure \
        --prefix="$PREFIX" \
        --enable-cross-compile \
        --cross-prefix="${TARGET_ARCH}-" \
        --target-os=android \
        --arch=aarch64 \
        --cc="$CC" \
        --cxx="$CXX" \
        --ar="$AR" \
        --ranlib="$RANLIB" \
        --nm="$NM" \
        --strip="$STRIP" \
        --pkg-config=pkg-config \
        --pkg-config-flags=--static \
        --extra-cflags="-fPIC -I$PREFIX/include" \
        --extra-ldflags="-L$PREFIX/lib -static-libstdc++" \
        --enable-static \
        --disable-shared \
        --enable-pic \
        --enable-gpl \
        --enable-libx264 \
        --enable-libvpx \
        --enable-libopus \
        --disable-doc \
        --disable-debug \
        --disable-x86asm \
        --disable-programs \
        --enable-ffmpeg
    # The program is what AniCraft runs, so it is built after the libraries.
    make -j"$JOBS" ffmpeg
    make install
    cd "$ROOT"
    # The sources and the object files are large; only the installed program is kept.
    rm -rf "$WORK"
fi

if [[ -n "$INSTALL_DIR" ]]; then
    mkdir -p "$INSTALL_DIR"
    cp -f "$PREFIX/bin/ffmpeg" "$INSTALL_DIR/$INSTALL_NAME"
    chmod 0755 "$INSTALL_DIR/$INSTALL_NAME"
    echo "Installed the Android ffmpeg as $INSTALL_DIR/$INSTALL_NAME"
fi
echo "ffmpeg for Android: $PREFIX/bin/ffmpeg"
"$PREFIX/bin/ffmpeg" -version 2>/dev/null | head -1 || true

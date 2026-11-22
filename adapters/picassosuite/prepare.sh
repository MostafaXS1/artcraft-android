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
    echo "ERROR: Could not find Android NDK" >&2
    exit 1
fi
TOOLCHAIN="$NDK_PATH/toolchains/llvm/prebuilt/linux-x86_64"
TARGET_ARCH="aarch64-linux-android"
API_LEVEL="28"
export BINDGEN_EXTRA_CLANG_ARGS="--target=aarch64-linux-android${API_LEVEL} ${BINDGEN_EXTRA_CLANG_ARGS:-}"
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

python3 - "$SRC_ABS/Cargo.toml" <<'PY'
from pathlib import Path
import re, sys
p = Path(sys.argv[1])
s = p.read_text()
m = re.search(r'(?ms)^members\s*=\s*\[(.*?)\]', s)
if m:
    inner = m.group(1)
    if '"apps/picassosuite-android"' not in inner:
        new_inner = inner.rstrip()
        if not new_inner.endswith(','):
            new_inner += ','
        new_inner += ' "apps/picassosuite-android"'
        s = s[:m.start(1)] + new_inner + s[m.end(1):]
        p.write_text(s)
PY

# Disable desktop window controls on Android
python3 - "$SRC_ABS/crates/picasso-ui/src/chrome.rs" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
s = p.read_text()
s = s.replace(
    '#[cfg(not(target_arch = "wasm32"))]\n        self.window_buttons(ui, ctx, s);\n        #[cfg(target_arch = "wasm32")]\n        let _ = ctx;',
    '#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]\n        self.window_buttons(ui, ctx, s);\n        #[cfg(any(target_arch = "wasm32", target_os = "android"))]\n        let _ = ctx;'
)
s = s.replace(
    '#[cfg(not(target_arch = "wasm32"))]\n    fn window_buttons',
    '#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]\n    fn window_buttons'
)
p.write_text(s)
PY

mkdir -p "$DEST"
cp -a "$ADAPTER/." "$DEST/"
rm -f "$DEST/prepare.sh" "$DEST/README.md"
cp -f "$ROOT/templates/android/system_fonts.rs" "$DEST/src/system_fonts.rs"
cp -f "$ROOT/templates/android/mobile_text_bar.rs" "$DEST/src/mobile_text_bar.rs"
cp -f "$ROOT/templates/android/touch_gestures.rs" "$DEST/src/touch_gestures.rs"
sed -i '/source: egui::MouseWheelSource::/d' "$DEST/src/touch_gestures.rs" 2>/dev/null || true
[[ -f "$ADAPTER/src/mcp_http.rs" ]] || cp -f "$ROOT/templates/android/mcp_http.rs" "$DEST/src/mcp_http.rs"

# Enable MediaCodec hardware decode backend in picasso-decode
python3 - "$SRC_ABS/crates/picasso-decode/src/hw.rs" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
s = p.read_text()
old = '''    } else if cfg!(target_os = "macos") {
        &[AV_HWDEVICE_TYPE_VIDEOTOOLBOX]'''
new = '''    } else if cfg!(target_os = "macos") {
        &[AV_HWDEVICE_TYPE_VIDEOTOOLBOX]
    } else if cfg!(target_os = "android") {
        &[AV_HWDEVICE_TYPE_MEDIACODEC]'''
if old in s and 'target_os = "android"' not in s:
    s = s.replace(old, new, 1)

helper = '''
pub(crate) fn open_video_decoder(
    params: &ff::codec::Parameters,
    time_base: ff::Rational,
    on_card: bool,
) -> Result<(ff::decoder::Video, bool), ff::Error> {
    #[cfg(target_os = "android")]
    if on_card {
        let mc_name = match params.id() {
            ff::codec::Id::H264 => Some("h264_mediacodec"),
            ff::codec::Id::HEVC => Some("hevc_mediacodec"),
            ff::codec::Id::VP9 => Some("vp9_mediacodec"),
            ff::codec::Id::AV1 => Some("av1_mediacodec"),
            _ => None,
        };
        if let Some(name) = mc_name {
            if let Some(dec) = ff::decoder::find_by_name(name) {
                if let Ok(mut ctx) = ff::codec::context::Context::from_parameters(params.clone()) {
                    ctx.set_threading(ff::threading::Config {
                        kind: ff::threading::Type::Frame,
                        count: 1,
                    });
                    unsafe {
                        (*ctx.as_mut_ptr()).pkt_timebase = time_base.into();
                    }
                    if let Ok(opened) = ctx.decoder().open_as(dec) {
                        if let Ok(video) = opened.video() {
                            return Ok((video, true));
                        }
                    }
                }
            }
        }
    }
    let mut ctx = ff::codec::context::Context::from_parameters(params.clone())?;
    let attached = on_card && attach(&mut ctx);
    ctx.set_threading(ff::threading::Config {
        kind: ff::threading::Type::Frame,
        count: if attached { 1 } else { 0 },
    });
    let video = ctx.decoder().video()?;
    Ok((video, attached))
}
'''
if 'pub(crate) fn open_video_decoder' not in s:
    s = s + helper

p.write_text(s)
PY

# Bind named MediaCodec decoders on Android in picasso-decode
python3 - "$SRC_ABS/crates/picasso-decode/src/lib.rs" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
s = p.read_text()

old_card = '''fn card_decoder_exists(params: &ff::codec::parameters::Parameters) -> bool {
    let Ok(mut probe) = ff::codec::context::Context::from_parameters(params.clone()) else {
        return false;
    };
    hw::attach(&mut probe)
}'''

new_card = '''fn card_decoder_exists(params: &ff::codec::parameters::Parameters) -> bool {
    #[cfg(target_os = "android")]
    {
        match params.id() {
            ff::codec::Id::H264 => ff::decoder::find_by_name("h264_mediacodec").is_some(),
            ff::codec::Id::HEVC => ff::decoder::find_by_name("hevc_mediacodec").is_some(),
            ff::codec::Id::VP9 => ff::decoder::find_by_name("vp9_mediacodec").is_some(),
            ff::codec::Id::AV1 => ff::decoder::find_by_name("av1_mediacodec").is_some(),
            _ => false,
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        let Ok(mut probe) = ff::codec::context::Context::from_parameters(params.clone()) else {
            return false;
        };
        hw::attach(&mut probe)
    }
}'''

old_dec = '''        let mut context = ff::codec::context::Context::from_parameters(params)?;
        let on_card = hardware && learnt != Some(false) && hw::attach(&mut context);
        // Decode on every core: pictures of a 1080p or 4K file do not arrive in time on one. The
        // card needs no threads, and each would hold more of its memory.
        context.set_threading(ff::threading::Config {
            kind: ff::threading::Type::Frame,
            count: if on_card { 1 } else { 0 },
        });
        let decoder = context.decoder().video()?;'''

new_dec = '''        let (decoder, on_card) = hw::open_video_decoder(&params, time_base, hardware && learnt != Some(false))?;'''

old_faster = '''fn faster_way(path: &Path) -> Option<bool> {
    let known = FASTER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    known.as_ref()?.get(path).copied()
}'''

new_faster = '''fn faster_way(path: &Path) -> Option<bool> {
    #[cfg(target_os = "android")]
    {
        let _ = path;
        return Some(true);
    }
    #[cfg(not(target_os = "android"))]
    {
        let known = FASTER
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        known.as_ref()?.get(path).copied()
    }
}'''

old_trial_rec = '''    fn record(&mut self, ms: f64, on_card: bool) -> TrialStep {'''
new_trial_rec = '''    fn record(&mut self, ms: f64, on_card: bool) -> TrialStep {
        #[cfg(target_os = "android")]
        {
            let _ = (ms, on_card);
            return TrialStep::Use(true);
        }'''

if old_card in s:
    s = s.replace(old_card, new_card, 1)
if old_dec in s:
    s = s.replace(old_dec, new_dec, 1)
if old_faster in s:
    s = s.replace(old_faster, new_faster, 1)
if old_trial_rec in s:
    s = s.replace(old_trial_rec, new_trial_rec, 1)
s = s.replace(
    'trial: (on_card && learnt.is_none()).then(Trial::default),',
    'trial: (!cfg!(target_os = "android") && on_card && learnt.is_none()).then(Trial::default),'
)
p.write_text(s)
PY

# Add MediaCodec hardware encoder to picasso-export
python3 - "$SRC_ABS/crates/picasso-export/src/lib.rs" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
s = p.read_text()

old_hw = '''pub enum Hardware {
    /// The processor (x264, x265, SVT-AV1 ...): the best picture for the size, the slowest.
    #[default]
    Native,
    /// NVIDIA's NVENC.
    Nvidia,
    /// Intel's Quick Sync.
    Intel,
    /// AMD's AMF (Windows).
    Amd,
    /// Apple's VideoToolbox (macOS).
    Apple,
    /// VA-API (Linux, Intel and AMD cards).
    Vaapi,
}'''

new_hw = '''pub enum Hardware {
    #[default]
    Native,
    MediaCodec,
    Nvidia,
    Intel,
    Amd,
    Apple,
    Vaapi,
}'''

old_all = '''    pub const ALL: [Self; 6] = [
        Self::Native,
        Self::Nvidia,
        Self::Intel,
        Self::Amd,
        Self::Apple,
        Self::Vaapi,
    ];'''

new_all = '''    pub const ALL: [Self; 7] = [
        Self::Native,
        Self::MediaCodec,
        Self::Nvidia,
        Self::Intel,
        Self::Amd,
        Self::Apple,
        Self::Vaapi,
    ];'''

old_label = '''    pub fn label(self) -> &'static str {
        match self {
            Self::Native => "Native",'''

new_label = '''    pub fn label(self) -> &'static str {
        match self {
            Self::MediaCodec => "MediaCodec (Android HW)",
            Self::Native => "Native",'''

old_enc = '''        Some(match (self, codec) {
            (Self::Nvidia, "h264") => "h264_nvenc",'''

new_enc = '''        Some(match (self, codec) {
            (Self::MediaCodec, "h264") => "h264_mediacodec",
            (Self::MediaCodec, "hevc") => "hevc_mediacodec",
            (Self::Nvidia, "h264") => "h264_nvenc",'''

old_spec_default = '''    pub fn default_spec(preset: Preset) -> ExportSpec {
        ExportSpec {
            preset,
            hardware: Hardware::Native,'''

new_spec_default = '''    pub fn default_spec(preset: Preset) -> ExportSpec {
        ExportSpec {
            preset,
            #[cfg(target_os = "android")]
            hardware: Hardware::MediaCodec,
            #[cfg(not(target_os = "android"))]
            hardware: Hardware::Native,'''

if old_hw in s:
    s = s.replace(old_hw, new_hw, 1)
if old_all in s:
    s = s.replace(old_all, new_all, 1)
if old_label in s:
    s = s.replace(old_label, new_label, 1)
if old_enc in s:
    s = s.replace(old_enc, new_enc, 1)
if old_spec_default in s:
    s = s.replace(old_spec_default, new_spec_default, 1)

old_opt_default = '''            hardware: Hardware::Native,'''
new_opt_default = '''            #[cfg(target_os = "android")]
            hardware: Hardware::MediaCodec,
            #[cfg(not(target_os = "android"))]
            hardware: Hardware::Native,'''
if old_opt_default in s:
    s = s.replace(old_opt_default, new_opt_default, 1)

p.write_text(s)
PY

python3 - "$SRC_ABS/crates/picasso-export/src/video.rs" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
s = p.read_text()

old_spec = '''    let mut spec = VideoSpec {
        encoder,
        pixel: if ten { pixel_10 } else { pixel_8 },
        hardware: hardware != Hardware::Native,
        vaapi: hardware == Hardware::Vaapi,
        options: Vec::new(),
        bit_rate,
        global_quality: None,
        qscale: false,
        intra: false,
    };'''

new_spec = '''    let is_mc = encoder.ends_with("_mediacodec");
    let mut spec = VideoSpec {
        encoder,
        pixel: if is_mc { Pixel::NV12 } else if ten { pixel_10 } else { pixel_8 },
        hardware: hardware != Hardware::Native || is_mc,
        vaapi: hardware == Hardware::Vaapi,
        options: Vec::new(),
        bit_rate: if is_mc && bit_rate == 0 { 16_000_000 } else { bit_rate },
        global_quality: None,
        qscale: false,
        intra: false,
    };'''

old_hdr = '''        if global_header {
            flags |= codec::Flags::GLOBAL_HEADER;
        }'''
new_hdr = '''        if global_header && !spec.encoder.ends_with("_mediacodec") {
            flags |= codec::Flags::GLOBAL_HEADER;
        }'''

old_b_frames = '''        if !o.frame_reordering || o.profile == crate::Profile::Baseline {
            enc.set_max_b_frames(0);
        }'''
new_b_frames = '''        if !o.frame_reordering || o.profile == crate::Profile::Baseline || spec.encoder.ends_with("_mediacodec") {
            enc.set_max_b_frames(0);
        }'''

if old_spec in s:
    s = s.replace(old_spec, new_spec, 1)
if old_hdr in s:
    s = s.replace(old_hdr, new_hdr, 1)
if old_b_frames in s:
    s = s.replace(old_b_frames, new_b_frames, 1)
p.write_text(s)
PY

python3 - "$SRC_ABS/crates/picasso-ui/src/deliver_form.rs" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
s = p.read_text()

old_deliv_hw = '''            let hardware = if choices.contains(&self.deliver.options.hardware) {
                self.deliver.options.hardware
            } else {
                if choices.len() > 1 {
                    self.deliver.options.hardware = Hardware::Native;
                }
                Hardware::Native
            };'''

new_deliv_hw = '''            let default_hw = if cfg!(target_os = "android") && choices.contains(&Hardware::MediaCodec) {
                Hardware::MediaCodec
            } else {
                Hardware::Native
            };
            let hardware = if choices.contains(&self.deliver.options.hardware) {
                self.deliver.options.hardware
            } else {
                if choices.len() > 1 {
                    self.deliver.options.hardware = default_hw;
                }
                default_hw
            };'''

if old_deliv_hw in s:
    s = s.replace(old_deliv_hw, new_deliv_hw, 1)
p.write_text(s)
PY

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

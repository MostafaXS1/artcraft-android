#!/usr/bin/env bash
set -Eeuo pipefail
APP="${1:-vidcut}"
TITLE="${2:-VidCut}"
SRC="${3:-source/vidcut}"
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
SRC_ABS="$ROOT/$SRC"
ADAPTER="$ROOT/adapters/vidcut"
DEST="$SRC_ABS/apps/$APP-android"

# Update root Cargo.toml to add workspace member
if [[ -f "$SRC_ABS/.cargo/config.toml" ]]; then
    python3 - "$SRC_ABS/.cargo/config.toml" <<'PY'
from pathlib import Path
import sys, re
p = Path(sys.argv[1])
s = p.read_text()
s = re.sub(r'(?m)^\s*target-dir\s*=\s*"[^"]*"\s*$', '', s)
p.write_text(s)
PY
fi

python3 - "$SRC_ABS/Cargo.toml" <<'PY'
from pathlib import Path
import re, sys
p = Path(sys.argv[1])
s = p.read_text()
m = re.search(r'(?ms)^members\s*=\s*\[(.*?)\]', s)
if m:
    inner = m.group(1)
    if '"apps/vidcut-android"' not in inner:
        new_inner = inner.rstrip()
        if not new_inner.endswith(','):
            new_inner += ','
        new_inner += ' "apps/vidcut-android"'
        s = s[:m.start(1)] + new_inner + s[m.end(1):]
s = re.sub(r'rfd\s*=\s*"0\.15"', 'rfd = { path = "vendor/rfd", default-features = false }', s)
p.write_text(s)
PY

# Add [lib] to crates/vidcut-app/Cargo.toml and create src/lib.rs if needed
python3 - "$SRC_ABS/crates/vidcut-app" <<'PY'
from pathlib import Path
import sys
app_dir = Path(sys.argv[1])
manifest = app_dir / "Cargo.toml"
s = manifest.read_text()
if "[lib]" not in s:
    s += '\n\n[lib]\nname = "vidcut_app"\npath = "src/lib.rs"\ncrate-type = ["rlib"]\n'
s = s.replace('rfd.workspace = true', 'rfd = { path = "../../vendor/rfd", default-features = false }')
manifest.write_text(s)

lib_rs = app_dir / "src" / "lib.rs"
if not lib_rs.is_file():
    lib_rs.write_text("pub mod app;\npub mod panels;\n")
PY

mkdir -p "$DEST"
cp -a "$ADAPTER/." "$DEST/"
rm -f "$DEST/prepare.sh" "$DEST/README.md"

echo "Bundling ffmpeg and ffprobe for VidCut..."
FFMPEG_DIR="$DEST/extra-jniLibs/arm64-v8a"
mkdir -p "$FFMPEG_DIR"
bash "$ROOT/lib/ffmpeg_android.sh" "$FFMPEG_DIR" "libvidcut_ffmpeg.so" "libvidcut_ffprobe.so"

python3 - "$SRC_ABS/crates/vidcut-media/src/ffmpeg_manager.rs" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
s = p.read_text()
s = s.replace(
    'let is_present = sidecar_path()\n        .map(|p| p.exists())\n        .unwrap_or(false);',
    'let is_present = sidecar_path().map(|p| p.exists()).unwrap_or(false) || std::env::var("VIDCUT_FFMPEG").map(|p| std::path::Path::new(&p).exists()).unwrap_or(false);'
)
s = s.replace(
    'sidecar_path()\n        .map(|p| p.exists())\n        .unwrap_or(false)',
    'sidecar_path().map(|p| p.exists()).unwrap_or(false) || std::env::var("VIDCUT_FFMPEG").map(|p| std::path::Path::new(&p).exists()).unwrap_or(false)'
)
p.write_text(s)
PY

python3 - "$SRC_ABS/crates/vidcut-media/src/decoder.rs" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
s = p.read_text()
if 'VIDCUT_FFPROBE' not in s:
    s = s.replace(
        'fn ffprobe_bin() -> String {',
        'fn ffprobe_bin() -> String {\n    if let Ok(p) = std::env::var("VIDCUT_FFPROBE") {\n        if std::path::Path::new(&p).exists() {\n            return p;\n        }\n    }'
    )
    p.write_text(s)
PY

python3 - "$SRC_ABS/crates/vidcut-media/src/encoder.rs" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
s = p.read_text()
if 'VIDCUT_FFMPEG' not in s:
    s = s.replace(
        'let ffmpeg_bin = ffmpeg_sidecar::paths::ffmpeg_path();',
        'let ffmpeg_bin = std::env::var("VIDCUT_FFMPEG").map(std::path::PathBuf::from).unwrap_or_else(|_| ffmpeg_sidecar::paths::ffmpeg_path());'
    )
    s = s.replace(
        'pub fn ffmpeg_available() -> bool {',
        'pub fn ffmpeg_available() -> bool {\n    if let Ok(p) = std::env::var("VIDCUT_FFMPEG") {\n        if std::path::Path::new(&p).exists() {\n            return true;\n        }\n    }'
    )

# Hardware accelerated encoding on Android with MediaCodec
single_old = '''        if seg.has_video {
            cmd.args(["-c:v", "libx264"]);
            cmd.args(["-crf", &job.quality.crf().to_string()]);
            cmd.args(["-preset", "medium"]);'''

single_new = '''        if seg.has_video {
            if std::env::var("VIDCUT_FFMPEG").is_ok() || cfg!(target_os = "android") {
                let bitrate = if job.width * job.height > 1920 * 1088 { "24M" } else if job.width * job.height > 1280 * 720 { "12M" } else { "6M" };
                cmd.args(["-c:v", "h264_mediacodec", "-b:v", bitrate, "-pix_fmt", "nv12"]);
            } else {
                cmd.args(["-c:v", "libx264"]);
                cmd.args(["-crf", &job.quality.crf().to_string()]);
                cmd.args(["-preset", "medium"]);
            }'''

multi_old = '''        if has_any_video {
            cmd.args(["-map", "[outv]"]);
            cmd.args(["-c:v", "libx264"]);
            cmd.args(["-crf", &job.quality.crf().to_string()]);
            cmd.args(["-preset", "medium"]);
        }'''

multi_new = '''        if has_any_video {
            cmd.args(["-map", "[outv]"]);
            if std::env::var("VIDCUT_FFMPEG").is_ok() || cfg!(target_os = "android") {
                let bitrate = if job.width * job.height > 1920 * 1088 { "24M" } else if job.width * job.height > 1280 * 720 { "12M" } else { "6M" };
                cmd.args(["-c:v", "h264_mediacodec", "-b:v", bitrate, "-pix_fmt", "nv12"]);
            } else {
                cmd.args(["-c:v", "libx264"]);
                cmd.args(["-crf", &job.quality.crf().to_string()]);
                cmd.args(["-preset", "medium"]);
            }
        }'''

if single_old in s:
    s = s.replace(single_old, single_new, 1)
if multi_old in s:
    s = s.replace(multi_old, multi_new, 1)

p.write_text(s)
PY

python3 - "$SRC_ABS/crates/vidcut-media/src/frame_extractor.rs" <<'PY'
from pathlib import Path
import sys, re
p = Path(sys.argv[1])
s = p.read_text()
if 'VIDCUT_FFMPEG' not in s:
    s = s.replace(
        'let ffmpeg_bin = ffmpeg_sidecar::paths::ffmpeg_path();',
        'let ffmpeg_bin = std::env::var("VIDCUT_FFMPEG").map(std::path::PathBuf::from).unwrap_or_else(|_| ffmpeg_sidecar::paths::ffmpeg_path());'
    )

# Accelerated hardware video decoding via named MediaCodec wrappers (with clean software fallback)
old_cmd = '''    let output = Command::new(&ffmpeg_bin)
        .args(["-ss", &format!("{:.6}", timestamp_secs)])
        .arg("-i")
        .arg(path.as_os_str())
        .args(["-vframes", "1"])
        .args(["-vf", &format!("scale={}:-2", out_width)])
        .args(["-f", "rawvideo", "-pix_fmt", "rgba"])
        .args(["-v", "error"])
        .arg("pipe:1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("Failed to run ffmpeg for frame extraction")?;'''

new_cmd = '''    let hw_codec = match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("webm") => "vp9_mediacodec",
        _ => "h264_mediacodec",
    };
    let hw_output = Command::new(&ffmpeg_bin)
        .args(["-ss", &format!("{:.6}", timestamp_secs)])
        .args(["-c:v", hw_codec])
        .arg("-i")
        .arg(path.as_os_str())
        .args(["-vframes", "1"])
        .args(["-vf", &format!("scale={}:-2", out_width)])
        .args(["-f", "rawvideo", "-pix_fmt", "rgba"])
        .args(["-v", "error"])
        .arg("pipe:1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    let output = match hw_output {
        Ok(out) if out.status.success() && !out.stdout.is_empty() => out,
        _ => {
            let hevc_output = Command::new(&ffmpeg_bin)
                .args(["-ss", &format!("{:.6}", timestamp_secs)])
                .args(["-c:v", "hevc_mediacodec"])
                .arg("-i")
                .arg(path.as_os_str())
                .args(["-vframes", "1"])
                .args(["-vf", &format!("scale={}:-2", out_width)])
                .args(["-f", "rawvideo", "-pix_fmt", "rgba"])
                .args(["-v", "error"])
                .arg("pipe:1")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output();
            match hevc_output {
                Ok(out) if out.status.success() && !out.stdout.is_empty() => out,
                _ => {
                    Command::new(&ffmpeg_bin)
                        .args(["-ss", &format!("{:.6}", timestamp_secs)])
                        .arg("-i")
                        .arg(path.as_os_str())
                        .args(["-vframes", "1"])
                        .args(["-vf", &format!("scale={}:-2", out_width)])
                        .args(["-f", "rawvideo", "-pix_fmt", "rgba"])
                        .args(["-v", "error"])
                        .arg("pipe:1")
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .output()
                        .context("Failed to run ffmpeg for frame extraction")?
                }
            }
        }
    };'''

if old_cmd in s:
    s = s.replace(old_cmd, new_cmd)
p.write_text(s)
PY
cp -f "$ROOT/templates/android/system_fonts.rs" "$DEST/src/system_fonts.rs"
python3 - "$DEST/src/system_fonts.rs" <<'PY'
from pathlib import Path
import sys, re
p = Path(sys.argv[1])
s = p.read_text()
s = re.sub(r'(?ms)pub fn install_system_fonts\(ctx: &egui::Context\) \{.*?^\}', 'pub fn install_system_fonts(_ctx: &egui::Context) {}', s)
p.write_text(s)
PY

cp -f "$ROOT/templates/android/mobile_text_bar.rs" "$DEST/src/mobile_text_bar.rs"
python3 - "$DEST/src/mobile_text_bar.rs" <<'PY'
from pathlib import Path
import sys, re
p = Path(sys.argv[1])
s = p.read_text()
s = s.replace("CornerRadius", "Rounding")
s = s.replace("Rounding::same(8)", "Rounding::same(8.0)")
s = s.replace("Rounding::same(5)", "Rounding::same(5.0)")
s = s.replace("ctx.content_rect()", "ctx.input(|i| i.raw.screen_rect).unwrap_or(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 1000.0)))")
s = re.sub(r'ui\.painter\(\)\.rect\(\s*bar_rect,\s*Rounding::same\(8\.0\),\s*bg_color,\s*stroke,\s*egui::StrokeKind::Inside,\s*\);',
           'ui.painter().rect_filled(bar_rect, Rounding::same(8.0), bg_color); ui.painter().rect_stroke(bar_rect, Rounding::same(8.0), stroke);', s)
s = re.sub(r'ui\.painter\(\)\.rect\(\s*r,\s*Rounding::same\(5\.0\),\s*fill,\s*btn_stroke,\s*egui::StrokeKind::Inside,\s*\);',
           'ui.painter().rect_filled(r, Rounding::same(5.0), fill); ui.painter().rect_stroke(r, Rounding::same(5.0), btn_stroke);', s)
p.write_text(s)
PY

cp -f "$ROOT/templates/android/touch_gestures.rs" "$DEST/src/touch_gestures.rs"
sed -i -e '/source: egui::MouseWheelSource::/d' -e '/phase: TouchPhase::/d' "$DEST/src/touch_gestures.rs" 2>/dev/null || true
[[ -f "$ADAPTER/src/mcp_http.rs" ]] || cp -f "$ROOT/templates/android/mcp_http.rs" "$DEST/src/mcp_http.rs"

python3 "$ROOT/lib/app_icons.py" --src "$SRC_ABS" --app "$APP" --dest "$DEST/res" || true
printf 'Generated by artcraft-android-builder; custom adapter sourced from adapters/vidcut.\n' > "$DEST/.artcraft-android-builder-generated"

python3 - "$DEST/Cargo.toml" "$ROOT" "$APP" <<'PY'
from pathlib import Path
import sys, tomllib
path, root, app = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]
sys.path.insert(0, str(root / "lib"))
import file_associations
file_associations.patch_cargo_toml(path, app)
tomllib.loads(path.read_text())
PY
echo "Installed VidCut adapter in $DEST"

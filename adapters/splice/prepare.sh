#!/usr/bin/env bash
set -Eeuo pipefail
APP="${1:-splice}"
TITLE="${2:-Splice}"
SRC="${3:-source/splice}"
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
SRC_ABS="$ROOT/$SRC"
ADAPTER="$ROOT/adapters/splice"
DEST="$SRC_ABS/apps/$APP-android"

# Update root Cargo.toml to add workspace members and [lib]
python3 - "$SRC_ABS/Cargo.toml" <<'PY'
from pathlib import Path
import re, sys
p = Path(sys.argv[1])
s = p.read_text()
if "[workspace]" not in s:
    s = '[workspace]\nmembers = [".", "apps/splice-android"]\n\n' + s
else:
    m = re.search(r'(?ms)^members\s*=\s*\[(.*?)\]', s)
    if m:
        inner = m.group(1)
        if '"apps/splice-android"' not in inner:
            new_inner = inner.rstrip()
            if not new_inner.endswith(','):
                new_inner += ','
            new_inner += ' "apps/splice-android"'
            s = s[:m.start(1)] + new_inner + s[m.end(1):]
if "[lib]" not in s:
    s += '\n\n[lib]\nname = "splice"\npath = "src/lib.rs"\ncrate-type = ["rlib"]\n'
p.write_text(s)
PY

mkdir -p "$DEST"
cp -a "$ADAPTER/." "$DEST/"
rm -f "$DEST/prepare.sh" "$DEST/README.md"

echo "Bundling ffmpeg and ffprobe for Splice..."
FFMPEG_DIR="$DEST/extra-jniLibs/arm64-v8a"
mkdir -p "$FFMPEG_DIR"
bash "$ROOT/lib/ffmpeg_android.sh" "$FFMPEG_DIR" "libsplice_ffmpeg.so" "libsplice_ffprobe.so"

python3 - "$SRC_ABS/src/ffmpeg.rs" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
s = p.read_text()
old_which = 'fn which(binary: &str) -> Result<PathBuf> {'
new_which = '''fn which(binary: &str) -> Result<PathBuf> {
    if binary == "ffmpeg" {
        if let Ok(var) = std::env::var("SPLICE_FFMPEG") {
            let p = PathBuf::from(var);
            if p.is_file() {
                return Ok(p);
            }
        }
    }
    if binary == "ffprobe" {
        if let Ok(var) = std::env::var("SPLICE_FFPROBE") {
            let p = PathBuf::from(var);
            if p.is_file() {
                return Ok(p);
            }
        }
    }'''
if old_which in s and 'SPLICE_FFMPEG' not in s:
    s = s.replace(old_which, new_which, 1)
    p.write_text(s)
PY

python3 - "$SRC_ABS/src/preview.rs" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
s = p.read_text()

# 1. Hardware accelerated scrubbing
old_scrub = '''        std::thread::spawn(move || {
            let mut command = Command::new(&ffmpeg);
            command.args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-ss",
                &format!("{seek:.4}"),
                "-i",
            ]);
            command.arg(&proxy);
            command.args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgba", "-"]);
            let output = command.output();'''

new_scrub = '''        std::thread::spawn(move || {
            let mut hw_command = Command::new(&ffmpeg);
            hw_command.args([
                "-hwaccel",
                "mediacodec",
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-ss",
                &format!("{seek:.4}"),
                "-i",
            ]);
            hw_command.arg(&proxy);
            hw_command.args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgba", "-"]);
            let output = match hw_command.output() {
                Ok(out) if out.status.success() && out.stdout.len() >= width as usize * height as usize * 4 => Ok(out),
                _ => {
                    let mut sw_command = Command::new(&ffmpeg);
                    sw_command.args([
                        "-hide_banner",
                        "-loglevel",
                        "error",
                        "-nostdin",
                        "-ss",
                        &format!("{seek:.4}"),
                        "-i",
                    ]);
                    sw_command.arg(&proxy);
                    sw_command.args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgba", "-"]);
                    sw_command.output()
                }
            };'''

if old_scrub in s:
    s = s.replace(old_scrub, new_scrub, 1)

# 2. Hardware accelerated playback
old_play = '''        std::thread::spawn(move || {
            let mut command = Command::new(&ffmpeg);
            command.args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-ss",
                &format!("{start:.4}"),
                "-i",
            ]);
            command.arg(&proxy);
            command.args([
                "-an", "-vf", "fps=30", "-f", "rawvideo", "-pix_fmt", "rgba", "-",
            ]);
            command.stdout(Stdio::piped()).stderr(Stdio::null());
            let mut child = match command.spawn() {
                Ok(child) => child,
                Err(_) => return,
            };
            let mut stdout = match child.stdout.take() {
                Some(stdout) => stdout,
                None => return,
            };
            *child_slot.lock().expect("child slot") = Some(child);'''

new_play = '''        std::thread::spawn(move || {
            let mut hw_command = Command::new(&ffmpeg);
            hw_command.args([
                "-hwaccel",
                "mediacodec",
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-ss",
                &format!("{start:.4}"),
                "-i",
            ]);
            hw_command.arg(&proxy);
            hw_command.args([
                "-an", "-vf", "fps=30", "-f", "rawvideo", "-pix_fmt", "rgba", "-",
            ]);
            hw_command.stdout(Stdio::piped()).stderr(Stdio::null());
            let (mut child, mut stdout) = match hw_command.spawn().and_then(|mut c| {
                let out = c.stdout.take().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "no stdout"));
                out.map(|o| (c, o))
            }) {
                Ok(pair) => pair,
                Err(_) => {
                    let mut sw_command = Command::new(&ffmpeg);
                    sw_command.args([
                        "-hide_banner",
                        "-loglevel",
                        "error",
                        "-nostdin",
                        "-ss",
                        &format!("{start:.4}"),
                        "-i",
                    ]);
                    sw_command.arg(&proxy);
                    sw_command.args([
                        "-an", "-vf", "fps=30", "-f", "rawvideo", "-pix_fmt", "rgba", "-",
                    ]);
                    sw_command.stdout(Stdio::piped()).stderr(Stdio::null());
                    match sw_command.spawn().and_then(|mut c| {
                        let out = c.stdout.take().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "no stdout"));
                        out.map(|o| (c, o))
                    }) {
                        Ok(pair) => pair,
                        Err(_) => return,
                    }
                }
            };
            *child_slot.lock().expect("child slot") = Some(child);'''

if old_play in s:
    s = s.replace(old_play, new_play, 1)

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
cp -f "$ROOT/templates/android/touch_gestures.rs" "$DEST/src/touch_gestures.rs"
sed -i -e '/source: egui::MouseWheelSource::/d' -e '/phase: TouchPhase::/d' "$DEST/src/touch_gestures.rs" 2>/dev/null || true
[[ -f "$ADAPTER/src/mcp_http.rs" ]] || cp -f "$ROOT/templates/android/mcp_http.rs" "$DEST/src/mcp_http.rs"

python3 "$ROOT/lib/app_icons.py" --src "$SRC_ABS" --app "$APP" --dest "$DEST/res" || true
printf 'Generated by artcraft-android-builder; custom adapter sourced from adapters/splice.\n' > "$DEST/.artcraft-android-builder-generated"

python3 - "$DEST/Cargo.toml" "$ROOT" "$APP" <<'PY'
from pathlib import Path
import sys, tomllib
path, root, app = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]
sys.path.insert(0, str(root / "lib"))
import file_associations
file_associations.patch_cargo_toml(path, app)
tomllib.loads(path.read_text())
PY
echo "Installed Splice adapter in $DEST"

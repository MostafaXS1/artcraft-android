#!/usr/bin/env bash
set -Eeuo pipefail
APP="${1:-prowave}"
TITLE="${2:-ProWave}"
SRC="${3:-source/prowave}"
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
SRC_ABS="$ROOT/$SRC"
ADAPTER="$ROOT/adapters/prowave"
DEST="$SRC_ABS/apps/$APP-android"

# Add storytold egui 0.37 fork patches
python3 - "$SRC_ABS/Cargo.toml" <<'PY'
from pathlib import Path
import sys, tomllib
p = Path(sys.argv[1])
s = p.read_text()
entries = '''
ecolor = { git = "https://github.com/storytold/egui.git", rev = "6d518fc799681571963842f95b55a6b68431b15a" }
emath = { git = "https://github.com/storytold/egui.git", rev = "6d518fc799681571963842f95b55a6b68431b15a" }
epaint = { git = "https://github.com/storytold/egui.git", rev = "6d518fc799681571963842f95b55a6b68431b15a" }
epaint_default_fonts = { git = "https://github.com/storytold/egui.git", rev = "6d518fc799681571963842f95b55a6b68431b15a" }
egui = { git = "https://github.com/storytold/egui.git", rev = "6d518fc799681571963842f95b55a6b68431b15a" }
egui_glow = { git = "https://github.com/storytold/egui.git", rev = "6d518fc799681571963842f95b55a6b68431b15a" }
egui_extras = { git = "https://github.com/storytold/egui.git", rev = "6d518fc799681571963842f95b55a6b68431b15a" }
egui_kittest = { git = "https://github.com/storytold/egui.git", rev = "6d518fc799681571963842f95b55a6b68431b15a" }
egui_system_font_provider = { git = "https://github.com/storytold/egui.git", rev = "6d518fc799681571963842f95b55a6b68431b15a" }
eframe = { git = "https://github.com/storytold/egui.git", rev = "6d518fc799681571963842f95b55a6b68431b15a" }
'''
if "https://github.com/storytold/egui.git" not in s:
    header = "[patch.crates-io]"
    s = (s.replace(header, header + "\n" + entries.strip(), 1) if header in s
         else s.rstrip() + "\n\n" + header + "\n" + entries.strip() + "\n")
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
    if '"apps/prowave-android"' not in inner:
        new_inner = inner.rstrip()
        if not new_inner.endswith(','):
            new_inner += ','
        new_inner += ' "apps/prowave-android"'
        s = s[:m.start(1)] + new_inner + s[m.end(1):]
        p.write_text(s)
PY

# Disable accesskit on eframe in crates/prowave-app for Android native-activity compatibility
sed -i 's/^eframe = "0.37.0"/eframe = { version = "0.37.0", default-features = false, features = ["default_fonts", "wgpu"] }/' "$SRC_ABS/crates/prowave-app/Cargo.toml" || true
# Downgrade midir to 0.10.4 to use vendored dummy backend (avoids libamidi linking error on API 28)
sed -i 's/^midir = "0.11.1"/midir = "0.10.4"/' "$SRC_ABS/crates/prowave-app/Cargo.toml" || true

# Ensure crates/prowave-app has a [lib] entry
if ! grep -q '^\[lib\]' "$SRC_ABS/crates/prowave-app/Cargo.toml"; then
    cat >> "$SRC_ABS/crates/prowave-app/Cargo.toml" <<'EOF'

[lib]
path = "src/lib.rs"
crate-type = ["rlib"]
EOF
fi

# Generate crates/prowave-app/src/lib.rs from src/main.rs
python3 - "$SRC_ABS/crates/prowave-app/src/main.rs" "$SRC_ABS/crates/prowave-app/src/lib.rs" <<'PY'
from pathlib import Path
import sys, re
src = Path(sys.argv[1])
dst = Path(sys.argv[2])
text = src.read_text()

# Strip inner attributes
lines = text.splitlines(True)
filtered = []
for line in lines:
    if line.startswith("#!["):
        continue
    filtered.append(line)
text = "".join(filtered)

# Make struct App and UiState public
text = re.sub(r'(?m)^struct App\b', 'pub struct App', text)
text = re.sub(r'(?m)^struct UiState\b', 'pub struct UiState', text)

# Add the create_app helper at the end.
#
# The Android shell needs an `App`, and upstream's own construction of one is the truth: the
# adapter used to spell out every field itself, so each release that added a field broke the
# Android build ("error[E0063]: missing fields `shot` and `shot_exit` in initializer of `App`").
# The literal is copied over instead, and the values main() takes from its command line are
# supplied empty: `bench_out`, `shots_dir` and `shot_out` are the paths the desktop app reads
# from its arguments (there are none on Android), `measuring` is what upstream uses to switch
# the MIDI ports off (Android has none), and `shot_exit` is the code a failed `--screenshot`
# leaves behind (`Default::default()` is the same fresh counter).
literal = None
for marker in ("let mut app = App {", "let app = App {"):
    at = text.find(marker)
    if at < 0:
        continue
    start_brace = text.index("{", at + len(marker) - 1)
    depth = 0
    for i in range(start_brace, len(text)):
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            depth -= 1
            if depth == 0:
                literal = text[start_brace + 1 : i]
                break
    break
if literal is None:
    raise SystemExit("prowave-app/src/main.rs no longer constructs an `App` the way the adapter reads it")

helper = """

/// The Android shell's app: upstream's own construction, with the desktop-only bits empty.
pub fn create_app(cc: &eframe::CreationContext<'_>) -> App {
    theme::apply(&cc.egui_ctx);
    cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
    let project = model::Project::empty();
    let sync = match prowave_engine::Engine::start() {
        Ok(e) => Some(engine_sync::EngineSync::new(e, &project)),
        Err(err) => {
            tracing::warn!("audio disabled on android: {err}");
            None
        }
    };
    let bench_out = bench::requested(&[]);
    let shots_dir = bench::shots_requested(&[]);
    let shot_out = bench::screenshot_requested(&[]);
    let measuring = true;
    let shot_exit: std::sync::Arc<std::sync::atomic::AtomicI32> = Default::default();
    let midi_out = None;
    App {""" + literal + """}
}
"""
dst.write_text(text + helper)
PY

# Disable desktop window controls on Android
sed -i 's/pub const CUSTOM: bool = !cfg!(target_os = "macos");/pub const CUSTOM: bool = !cfg!(any(target_os = "macos", target_os = "android"));/' "$SRC_ABS/crates/prowave-app/src/window_chrome.rs" || true

mkdir -p "$DEST"
cp -a "$ADAPTER/." "$DEST/"
rm -f "$DEST/prepare.sh" "$DEST/README.md"
cp -f "$ROOT/templates/android/system_fonts.rs" "$DEST/src/system_fonts.rs"
cp -f "$ROOT/templates/android/mobile_text_bar.rs" "$DEST/src/mobile_text_bar.rs"
cp -f "$ROOT/templates/android/touch_gestures.rs" "$DEST/src/touch_gestures.rs"
[[ -f "$ADAPTER/src/mcp_http.rs" ]] || cp -f "$ROOT/templates/android/mcp_http.rs" "$DEST/src/mcp_http.rs"

python3 "$ROOT/lib/app_icons.py" --src "$SRC_ABS" --app "$APP" --dest "$DEST/res" || true
printf 'Generated by artcraft-android-builder; custom adapter sourced from adapters/prowave.\n' > "$DEST/.artcraft-android-builder-generated"

python3 - "$DEST/Cargo.toml" "$ROOT" "$APP" <<'PY'
from pathlib import Path
import sys, tomllib
path, root, app = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]
sys.path.insert(0, str(root / "lib"))
import file_associations
file_associations.patch_cargo_toml(path, app)
tomllib.loads(path.read_text())
PY
echo "Installed ProWave adapter in $DEST"

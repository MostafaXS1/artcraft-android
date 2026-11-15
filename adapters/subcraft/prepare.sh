#!/usr/bin/env bash
set -Eeuo pipefail
APP="${1:-subcraft}"
TITLE="${2:-SubCraft}"
SRC="${3:-source/subcraft}"
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
SRC_ABS="$ROOT/$SRC"
ADAPTER="$ROOT/adapters/subcraft"
DEST="$SRC_ABS/apps/$APP-android"

# Disable desktop window caption buttons on Android
# The caption_buttons are drawn in the header when frameless() is true.
# On Android, we don't want these minimize/maximize/close buttons since
# Android handles window management differently (immersive fullscreen).
python3 - "$SRC_ABS/crates/ui/src/app.rs" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
s = p.read_text()

# Replace the caption_buttons call to be conditional on not being Android
old = '''            if frameless {
                // Minimize / maximize / close flush with the window's right edge, beside Export.
                let gap = ui.spacing().item_spacing.x;
                ui.spacing_mut().item_spacing.x = 0.0;
                crate::window_frame::caption_buttons(ui);
                ui.spacing_mut().item_spacing.x = gap;
                ui.add_space(8.0);
            }'''

new = '''            if frameless {
                // Minimize / maximize / close flush with the window's right edge, beside Export.
                // Disabled on Android: immersive fullscreen handles window controls.
                #[cfg(not(target_os = "android"))]
                {
                    let gap = ui.spacing().item_spacing.x;
                    ui.spacing_mut().item_spacing.x = 0.0;
                    crate::window_frame::caption_buttons(ui);
                    ui.spacing_mut().item_spacing.x = gap;
                    ui.add_space(8.0);
                }
                #[cfg(target_os = "android")]
                ui.add_space(8.0);
            }'''

if new in s:
    print("caption_buttons already conditional for Android")
elif old in s:
    s = s.replace(old, new)
    p.write_text(s)
    print("Patched caption_buttons to be disabled on Android")
else:
    print("Could not find caption_buttons block to patch")
    # Debug: show context around the expected location
    for i, line in enumerate(s.split('\n')):
        if 'caption_buttons' in line:
            print(f"  Line {i}: {line}")
            for j in range(max(0, i-5), min(len(s.split('\n')), i+5)):
                print(f"  Line {j}: {s.split('\n')[j]}")
            break
PY

# Also disable title_bar_background (window dragging) on Android
python3 - "$SRC_ABS/crates/ui/src/app.rs" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
s = p.read_text()

# Replace the title_bar_background call to be conditional on not being Android
old = '''        // Empty parts of the header move the window (registered first so widgets stay on top).
        if frameless {
            crate::window_frame::title_bar_background(ui, bar);
        }'''

new = '''        // Empty parts of the header move the window (registered first so widgets stay on top).
        // Disabled on Android: immersive fullscreen handles window position.
        #[cfg(not(target_os = "android"))]
        if frameless {
            crate::window_frame::title_bar_background(ui, bar);
        }'''

if new in s:
    print("title_bar_background already conditional for Android")
elif old in s:
    s = s.replace(old, new)
    p.write_text(s)
    print("Patched title_bar_background to be disabled on Android")
else:
    print("Could not find title_bar_background block to patch")
    for i, line in enumerate(s.split('\n')):
        if 'title_bar_background' in line:
            print(f"  Line {i}: {line}")
            for j in range(max(0, i-5), min(len(s.split('\n')), i+5)):
                print(f"  Line {j}: {s.split('\n')[j]}")
            break
PY

# Also disable resize_edges on Android
python3 - "$SRC_ABS/crates/ui/src/app.rs" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
s = p.read_text()

# Replace the resize_edges call to be conditional on not being Android
old = '''        if crate::window_frame::frameless() { crate::window_frame::resize_edges(&ctx); }'''

new = '''        // Disabled on Android: immersive fullscreen windows don't have resize handles.
        #[cfg(not(target_os = "android"))]
        if crate::window_frame::frameless() { crate::window_frame::resize_edges(&ctx); }'''

if new in s:
    print("resize_edges already conditional for Android")
elif old in s:
    s = s.replace(old, new)
    p.write_text(s)
    print("Patched resize_edges to be disabled on Android")
else:
    print("Could not find resize_edges block to patch")
    for i, line in enumerate(s.split('\n')):
        if 'resize_edges' in line:
            print(f"  Line {i}: {line}")
            for j in range(max(0, i-5), min(len(s.split('\n')), i+5)):
                print(f"  Line {j}: {s.split('\n')[j]}")
            break
PY

mkdir -p "$DEST"
cp -a "$ADAPTER/." "$DEST/"
rm -f "$DEST/prepare.sh" "$DEST/README.md"
cp -f "$ROOT/templates/android/system_fonts.rs" "$DEST/src/system_fonts.rs"
cp -f "$ROOT/templates/android/mobile_text_bar.rs" "$DEST/src/mobile_text_bar.rs"
cp -f "$ROOT/templates/android/touch_gestures.rs" "$DEST/src/touch_gestures.rs"
[[ -f "$ADAPTER/src/mcp_http.rs" ]] || cp -f "$ROOT/templates/android/mcp_http.rs" "$DEST/src/mcp_http.rs"

python3 "$ROOT/lib/app_icons.py" --src "$SRC_ABS" --app "$APP" --dest "$DEST/res" || true
printf 'Generated by artcraft-android-builder; custom adapter sourced from adapters/subcraft.\n' > "$DEST/.artcraft-android-builder-generated"

python3 - "$DEST/Cargo.toml" "$ROOT" "$APP" <<'PY'
from pathlib import Path
import sys, tomllib
path, root, app = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]
sys.path.insert(0, str(root / "lib"))
import file_associations
file_associations.patch_cargo_toml(path, app)
tomllib.loads(path.read_text())
PY
echo "Installed SubCraft adapter in $DEST"
#!/usr/bin/env bash
set -Eeuo pipefail
APP="${1:-diagramcraft}"
TITLE="${2:-DiagramCraft}"
SRC="${3:-source/diagramcraft}"
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
SRC_ABS="$ROOT/$SRC"
ADAPTER="$ROOT/adapters/diagramcraft"
DEST="$SRC_ABS/apps/$APP-android"

# Inject storytold/egui 0.37 patches so eframe matches vendored egui-winit and egui-wgpu
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

# Patch dc-ui app.rs to handle Android (no std::process::Command on Android)
python3 - "$SRC_ABS/crates/dc-ui/src/app.rs" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
s = p.read_text()

# Add Android case to the hyperlink opening logic
old = '''#[cfg(target_os = "linux")]
        let open = "xdg-open";
        #[cfg(target_os = "macos")]
        let open = "open";
        #[cfg(target_os = "windows")]
        let open = "cmd";
        #[cfg(target_os = "windows")]
        let flag = "/C";
        #[cfg(not(target_os = "windows"))]
        let flag = "";
        let mut cmd = std::process::Command::new(open);
        if !flag.is_empty() {
            cmd.arg(flag);
        }
        match cmd.arg(&address).spawn() {
            Ok(_) => self.status = format!("opened {address}"),
            Err(e) => self.status = format!("could not open {address}: {e}"),
        }'''

new = '''#[cfg(target_os = "linux")]
        let open = "xdg-open";
        #[cfg(target_os = "macos")]
        let open = "open";
        #[cfg(target_os = "windows")]
        let open = "cmd";
        #[cfg(target_os = "windows")]
        let flag = "/C";
        #[cfg(not(target_os = "windows"))]
        let flag = "";
        #[cfg(target_os = "android")]
        {
            // On Android, we can't use std::process::Command; hyperlink handling is a no-op.
            self.status = format!("hyperlink {address} not opened (Android)");
            return;
        }
        #[cfg(not(target_os = "android"))]
        {
            let mut cmd = std::process::Command::new(open);
            if !flag.is_empty() {
                cmd.arg(flag);
            }
            match cmd.arg(&address).spawn() {
                Ok(_) => self.status = format!("opened {address}"),
                Err(e) => self.status = format!("could not open {address}: {e}"),
            }
        }'''

# Check if already patched
if 'target_os = "android"' in s and 'hyperlink' in s and 'not opened (Android)' in s:
    print("dc-ui app.rs already patched for Android")
elif old in s:
    s = s.replace(old, new)
    p.write_text(s)
    print("Patched dc-ui app.rs for Android")
else:
    # Debug: show what's around the target area
    idx = s.find('let mut cmd = std::process::Command::new(open);')
    if idx >= 0:
        print("Found target area:")
        print(repr(s[max(0, idx-300):idx+400]))
    else:
        print("Could not find target pattern")
    sys.exit(1)
PY

# Expose a public "open this file into the running app" seam for the Android shell's incoming
# VIEW/SEND intents. `load` is private upstream, and replacing the whole App would discard the
# user's settings, recents and loaded stencils.
python3 - "$SRC_ABS/crates/dc-ui/src/app.rs" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
s = p.read_text()

anchor = '''    /// Open a document at `path` (headless tests, `dc-ui <file>`).
    pub fn with_doc(path: &std::path::Path) -> App {
        App::with_doc_opt(Some(path.to_path_buf()))
    }
'''

addition = '''
    /// Open `path` into this running app (the Android shell's incoming VIEW/SEND intents).
    /// Unlike `with_doc`, this preserves the app's settings, recents and loaded stencils.
    pub fn open_document(&mut self, path: PathBuf) {
        self.load(path);
    }
'''

if "pub fn open_document" in s:
    print("dc-ui app.rs already exposes open_document")
elif anchor in s:
    s = s.replace(anchor, anchor + addition)
    p.write_text(s)
    print("Patched dc-ui app.rs with open_document")
else:
    raise SystemExit("Could not find dc-ui with_doc anchor for the open_document patch")
PY

mkdir -p "$DEST"
cp -r "$ADAPTER/." "$DEST/"
rm -f "$DEST/prepare.sh" "$DEST/README.md"
rm -rf "$DEST/assets"
if [[ -d "$ADAPTER/assets" && ! -d "$SRC_ABS/assets" ]]; then
    cp -a "$ADAPTER/assets" "$SRC_ABS/"
fi
cp -f "$ROOT/templates/android/system_fonts.rs" "$DEST/src/system_fonts.rs"
cp -f "$ROOT/templates/android/mobile_text_bar.rs" "$DEST/src/mobile_text_bar.rs"
[[ -f "$ADAPTER/src/mcp_http.rs" ]] || cp -f "$ROOT/templates/android/mcp_http.rs" "$DEST/src/mcp_http.rs"

python3 "$ROOT/lib/app_icons.py" --src "$SRC_ABS" --app "$APP" --dest "$DEST/res" || true
printf 'Generated by artcraft-android-builder; custom adapter sourced from adapters/diagramcraft.\n' > "$DEST/.artcraft-android-builder-generated"

python3 - "$DEST/Cargo.toml" "$ROOT" "$APP" <<'PY'
from pathlib import Path
import sys, tomllib
path, root, app = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]
sys.path.insert(0, str(root / "lib"))
import file_associations
file_associations.patch_cargo_toml(path, app)
tomllib.loads(path.read_text())
PY
echo "Installed DiagramCraft adapter in $DEST"

# Make the launcher a member of the upstream workspace so its [patch.crates-io] and
# [patch."https://github.com/storytold/egui.git"] tables apply to the Android build. Without
# this the launcher is its own workspace, so the vendored winit never reaches egui-winit and
# the build fails with a stock winit that lacks the storytold IME extensions.
python3 - "$SRC_ABS/Cargo.toml" <<'PY'
from pathlib import Path
import re
import sys
p = Path(sys.argv[1])
s = p.read_text()
m = re.search(r'(?ms)^members\s*=\s*\[(.*?)\]', s)
if not m:
    raise SystemExit("DiagramCraft workspace has no members list; update the adapter prepare.sh")
inner = m.group(1)
if '"apps/diagramcraft-android"' not in inner:
    new_inner = inner.rstrip()
    if not new_inner.endswith(','):
        new_inner += ','
    new_inner += ' "apps/diagramcraft-android"'
    s = s[:m.start(1)] + new_inner + s[m.end(1):]
    p.write_text(s)
    print("Added apps/diagramcraft-android to the DiagramCraft workspace members")
PY
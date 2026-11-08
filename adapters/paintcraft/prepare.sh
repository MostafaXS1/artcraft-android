#!/usr/bin/env bash
set -Eeuo pipefail
APP="${1:-paintcraft}"
TITLE="${2:-PaintCraft}"
SRC="${3:-source/paintcraft}"
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
SRC_ABS="$ROOT/$SRC"
ADAPTER="$ROOT/adapters/paintcraft"
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

python3 - "$SRC_ABS/Cargo.toml" <<'PY'
from pathlib import Path
import re, sys
p = Path(sys.argv[1])
s = p.read_text()
m = re.search(r'(?ms)^members\s*=\s*\[(.*?)\]', s)
if m:
    inner = m.group(1)
    if '"apps/paintcraft-android"' not in inner:
        new_inner = inner.rstrip()
        if not new_inner.endswith(','):
            new_inner += ','
        new_inner += ' "apps/paintcraft-android"'
        s = s[:m.start(1)] + new_inner + s[m.end(1):]
        p.write_text(s)
PY

# Guard against egui DeleteSurroundingBytes match exhaustion
python3 - "$SRC_ABS/crates/presentation/paintcraft-ui/src/composition/reduce.rs" <<'PY'
import sys
from pathlib import Path
p = Path(sys.argv[1])
if p.exists():
    s = p.read_text()
    if "_ => {}" not in s and "DeleteSurroundingBytes" not in s:
        target = ".delete_surrounding_chars(self.cursor, *before_chars, *after_chars);\n            }"
        replacement = target + "\n            _ => {}"
        if target in s:
            p.write_text(s.replace(target, replacement, 1))
PY

# Guard against egui IMEOutput extra fields in storytold fork
python3 - "$SRC_ABS/apps/paintcraft/src/app/canvas.rs" <<'PY'
import sys
from pathlib import Path
p = Path(sys.argv[1])
if p.exists():
    s = p.read_text()
    old = '''                            painter.ctx().output_mut(|output| {
                                output.ime = Some(egui::output::IMEOutput {
                                    purpose: egui::IMEPurpose::Normal,
                                    rect: self
                                        .canvas_rect
                                        .unwrap_or(cursor_rect)
                                        .intersect(painter.clip_rect()),
                                    cursor_rect,
                                    should_interrupt_composition: interrupt,
                                })
                            });'''
    new = '''                            painter.ctx().output_mut(|output| {
                                let rect = self
                                    .canvas_rect
                                    .unwrap_or(cursor_rect)
                                    .intersect(painter.clip_rect());
                                let mut ime = egui::output::IMEOutput::new(
                                    egui::Id::new("paintcraft_canvas_ime"),
                                    1,
                                    egui::IMEPurpose::Normal,
                                    rect,
                                    cursor_rect,
                                );
                                ime.should_interrupt_composition = interrupt;
                                output.ime = Some(ime);
                            });'''
    if old in s:
        p.write_text(s.replace(old, new, 1))
PY

mkdir -p "$DEST"
cp -a "$ADAPTER/." "$DEST/"
rm -f "$DEST/prepare.sh" "$DEST/README.md"
cp -f "$ROOT/templates/android/system_fonts.rs" "$DEST/src/system_fonts.rs"

python3 "$ROOT/lib/app_icons.py" --src "$SRC_ABS" --app "$APP" --dest "$DEST/res" || true
printf 'Generated by artcraft-android-builder; custom adapter sourced from adapters/paintcraft.\n' > "$DEST/.artcraft-android-builder-generated"

python3 - "$DEST/Cargo.toml" "$ROOT" "$APP" <<'PY'
from pathlib import Path
import sys, tomllib
path, root, app = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]
sys.path.insert(0, str(root / "lib"))
import file_associations
file_associations.patch_cargo_toml(path, app)
tomllib.loads(path.read_text())
PY
echo "Installed PaintCraft adapter in $DEST"

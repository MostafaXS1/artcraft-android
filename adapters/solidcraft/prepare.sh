#!/usr/bin/env bash
set -Eeuo pipefail
APP="${1:-solidcraft}"
TITLE="${2:-SolidCraft}"
SRC="${3:-source/solidcraft}"
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
SRC_ABS="$ROOT/$SRC"
ADAPTER="$ROOT/adapters/solidcraft"
DEST="$SRC_ABS/apps/$APP-android"

# Vendor the storytold egui fork so egui-winit has clipboard_image() and correct process_viewport_commands()
if [[ ! -d "$SRC_ABS/vendor/egui/.git" ]]; then
    echo "Vendoring storytold egui fork..."
    rm -rf "$SRC_ABS/vendor/egui"
    mkdir -p "$SRC_ABS/vendor/egui"
    cd "$SRC_ABS/vendor/egui"
    git init
    git remote add origin https://github.com/storytold/egui.git
    git fetch --depth 1 origin 6d518fc799681571963842f95b55a6b68431b15a
    git checkout FETCH_HEAD
    cd "$SRC_ABS"
fi

# Inject storytold/egui 0.37 patches so eframe matches vendored egui-winit and egui-wgpu
# Pattern from diagramcraft: 
# - [patch.crates-io] contains ALL egui crates pointing to git (with hyphens: egui-wgpu, egui-winit)
#   EXCEPT egui-wgpu which points to local vendor/egui-wgpu (SolidCraft's custom version with FrameCompositor, DeviceLost)
# - [patch."https://github.com/storytold/egui.git"] contains local path overrides for egui-wgpu (vendor/egui-wgpu) and egui-winit (vendor/egui/crates/egui-winit)
# - workspace.dependencies points to local vendor/egui-wgpu for egui-wgpu, and git for egui-winit
python3 - "$SRC_ABS/Cargo.toml" <<'PY'
from pathlib import Path
import sys, re
p = Path(sys.argv[1])
s = p.read_text()

# Remove any existing [patch."https://github.com/storytold/egui.git"] section completely
git_header = '[patch."https://github.com/storytold/egui.git"]'
if git_header in s:
    parts = s.split(git_header)
    before = parts[0]
    after = parts[1]
    next_section = re.search(r'\n\[', after)
    if next_section:
        after = after[next_section.start():]
    else:
        after = ''
    s = before + after

# Fix workspace.dependencies
# - egui-wgpu should point to local vendor/egui-wgpu (SolidCraft's custom version)
# - egui-winit should point to git fork (has clipboard_image())
s = re.sub(
    r'^\s*egui-winit\s*=\s*\{[^}]*git\s*=\s*"https://github.com/storytold/egui.git"[^}]*\}',
    'egui-winit = { git = "https://github.com/storytold/egui.git", rev = "6d518fc799681571963842f95b55a6b68431b15a" }',
    s,
    flags=re.M
)
s = re.sub(
    r'^\s*egui-wgpu\s*=\s*\{[^}]*git\s*=\s*"https://github.com/storytold/egui.git"[^}]*\}',
    'egui-wgpu = { path = "vendor/egui-wgpu" }',
    s,
    flags=re.M
)
# Also fix egui-wgpu pointing to wrong local path (from fork)
s = re.sub(
    r'^\s*egui-wgpu\s*=\s*\{[^}]*path\s*=\s*"vendor/egui/crates/egui-wgpu"[^}]*\}',
    'egui-wgpu = { path = "vendor/egui-wgpu" }',
    s,
    flags=re.M
)

# Remove any existing egui git entries from [patch.crates-io] (we'll re-add them)
patch_crates_io = '[patch.crates-io]'
egui_crates = ['ecolor', 'emath', 'epaint', 'epaint_default_fonts', 'egui', 'egui_glow', 'egui_extras', 'egui-wgpu', 'egui-winit', 'eframe']
if patch_crates_io in s:
    parts = s.split(patch_crates_io)
    before = parts[0]
    after = parts[1]
    # Find end of patch.crates-io section
    next_section = re.search(r'\n\[', after)
    if next_section:
        patch_section = after[:next_section.start()]
        rest = after[next_section.start():]
    else:
        patch_section = after
        rest = ''
    # Remove existing egui git entries
    for crate in egui_crates:
        pattern = r'^\s*' + re.escape(crate) + r'\s*=\s*\{[^}]*git\s*=\s*"https://github.com/storytold/egui.git"[^}]*\}\s*\n?'
        patch_section = re.sub(pattern, '', patch_section, flags=re.M)
    # Also remove any local path entries for egui, epaint, egui-wgpu and egui-winit from patch.crates-io
    patch_section = re.sub(r'^\s*egui\s*=\s*\{[^}]*path\s*=\s*"[^"]*"[^}]*\}\s*\n?', '', patch_section, flags=re.M)
    patch_section = re.sub(r'^\s*epaint\s*=\s*\{[^}]*path\s*=\s*"[^"]*"[^}]*\}\s*\n?', '', patch_section, flags=re.M)
    patch_section = re.sub(r'^\s*egui-wgpu\s*=\s*\{[^}]*path\s*=\s*"[^"]*"[^}]*\}\s*\n?', '', patch_section, flags=re.M)
    patch_section = re.sub(r'^\s*egui-winit\s*=\s*\{[^}]*path\s*=\s*"[^"]*"[^}]*\}\s*\n?', '', patch_section, flags=re.M)
    # Add all egui git entries (using hyphens for egui-wgpu and egui-winit as they appear in the git repo)
    # BUT egui-wgpu, egui, epaint point to local vendor (SolidCraft's custom version)
    new_entries = '''
ecolor = { git = "https://github.com/storytold/egui.git", rev = "6d518fc799681571963842f95b55a6b68431b15a" }
emath = { git = "https://github.com/storytold/egui.git", rev = "6d518fc799681571963842f95b55a6b68431b15a" }
epaint = { path = "vendor/egui/crates/epaint" }
epaint_default_fonts = { git = "https://github.com/storytold/egui.git", rev = "6d518fc799681571963842f95b55a6b68431b15a" }
egui = { path = "vendor/egui/crates/egui" }
egui_glow = { git = "https://github.com/storytold/egui.git", rev = "6d518fc799681571963842f95b55a6b68431b15a" }
egui_extras = { git = "https://github.com/storytold/egui.git", rev = "6d518fc799681571963842f95b55a6b68431b15a" }
egui-wgpu = { path = "vendor/egui-wgpu" }
egui-winit = { git = "https://github.com/storytold/egui.git", rev = "6d518fc799681571963842f95b55a6b68431b15a" }
eframe = { git = "https://github.com/storytold/egui.git", rev = "6d518fc799681571963842f95b55a6b68431b15a" }'''
    s = before + patch_crates_io + '\n' + patch_section.lstrip('\n') + new_entries.strip() + '\n' + rest
else:
    new_entries = '''ecolor = { git = "https://github.com/storytold/egui.git", rev = "6d518fc799681571963842f95b55a6b68431b15a" }
emath = { git = "https://github.com/storytold/egui.git", rev = "6d518fc799681571963842f95b55a6b68431b15a" }
epaint = { path = "vendor/egui/crates/epaint" }
epaint_default_fonts = { git = "https://github.com/storytold/egui.git", rev = "6d518fc799681571963842f95b55a6b68431b15a" }
egui = { path = "vendor/egui/crates/egui" }
egui_glow = { git = "https://github.com/storytold/egui.git", rev = "6d518fc799681571963842f95b55a6b68431b15a" }
egui_extras = { git = "https://github.com/storytold/egui.git", rev = "6d518fc799681571963842f95b55a6b68431b15a" }
egui-wgpu = { path = "vendor/egui-wgpu" }
egui-winit = { git = "https://github.com/storytold/egui.git", rev = "6d518fc799681571963842f95b55a6b68431b15a" }
eframe = { git = "https://github.com/storytold/egui.git", rev = "6d518fc799681571963842f95b55a6b68431b15a" }'''
    s = s.rstrip() + "\n\n" + patch_crates_io + '\n' + new_entries.strip() + "\n"

# Now add the correct local path overrides to [patch."https://github.com/storytold/egui.git"]
# This section contains path overrides for egui-wgpu (vendor/egui-wgpu) and egui-winit (vendor/egui/crates/egui-winit)
# And also egui and epaint themselves so the git repo's internal deps use the local copy
git_entries = '''egui = { path = "vendor/egui/crates/egui" }
epaint = { path = "vendor/egui/crates/epaint" }
ecolor = { path = "vendor/egui/crates/ecolor" }
emath = { path = "vendor/egui/crates/emath" }
egui-wgpu = { path = "vendor/egui-wgpu" }
egui-winit = { path = "vendor/egui/crates/egui-winit" }'''
s = s.rstrip() + "\n\n" + git_header + "\n" + git_entries.strip() + "\n"

p.write_text(s)
print("Added storytold egui fork patches to Cargo.toml")
PY

python3 - "$SRC_ABS/Cargo.toml" <<'PY'
from pathlib import Path
import re, sys
p = Path(sys.argv[1])
s = p.read_text()
m = re.search(r'(?ms)^members\s*=\s*\[(.*?)\]', s)
if m:
    inner = m.group(1)
    if '"apps/solidcraft-android"' not in inner:
        new_inner = inner.rstrip()
        if not new_inner.endswith(','):
            new_inner += ','
        new_inner += ' "apps/solidcraft-android"'
        s = s[:m.start(1)] + new_inner + s[m.end(1):]
        p.write_text(s)
PY

mkdir -p "$DEST"
cp -a "$ADAPTER/." "$DEST/"
rm -f "$DEST/prepare.sh" "$DEST/README.md"
cp -f "$ROOT/templates/android/system_fonts.rs" "$DEST/src/system_fonts.rs"
[[ -f "$ADAPTER/src/mcp_http.rs" ]] || cp -f "$ROOT/templates/android/mcp_http.rs" "$DEST/src/mcp_http.rs"

python3 "$ROOT/lib/app_icons.py" --src "$SRC_ABS" --app "$APP" --dest "$DEST/res" || true
printf 'Generated by artcraft-android-builder; custom adapter sourced from adapters/solidcraft.\n' > "$DEST/.artcraft-android-builder-generated"

python3 - "$DEST/Cargo.toml" "$ROOT" "$APP" <<'PY'
from pathlib import Path
import sys, tomllib
path, root, app = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]
sys.path.insert(0, str(root / "lib"))
import file_associations
file_associations.patch_cargo_toml(path, app)
tomllib.loads(path.read_text())
PY

echo "SolidCraft prepare.sh complete"

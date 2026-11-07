#!/usr/bin/env bash
set -Eeuo pipefail
APP="${1:-capsulecraft}"
TITLE="${2:-CapsuleCraft}"
SRC="${3:-source/capsulecraft}"
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
SRC_ABS="$ROOT/$SRC"
ADAPTER="$ROOT/adapters/capsulecraft"
DEST="$SRC_ABS/apps/$APP-android"

# Create root workspace if missing
if [[ ! -f "$SRC_ABS/Cargo.toml" ]]; then
    cat > "$SRC_ABS/Cargo.toml" <<'EOF'
[workspace]
resolver = "2"
members = ["native", "apps/*"]

[workspace.package]
version = "2.1.0"
edition = "2021"

[patch.crates-io]
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
EOF
elif ! grep -q "storytold/egui.git" "$SRC_ABS/Cargo.toml"; then
    cat >> "$SRC_ABS/Cargo.toml" <<'EOF'

[patch.crates-io]
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
EOF
fi

# Ensure native has a [lib] entry
if ! grep -q '^\[lib\]' "$SRC_ABS/native/Cargo.toml"; then
    cat >> "$SRC_ABS/native/Cargo.toml" <<'EOF'

[lib]
name = "capsuleos_native"
path = "src/main.rs"
EOF
fi

# Make NativeApp public so the Android launcher can instantiate it
sed -i 's/^struct NativeApp/pub struct NativeApp/' "$SRC_ABS/native/src/main.rs"
sed -i 's/^    fn new(launch: cli::Launch)/    pub fn new(launch: cli::Launch)/' "$SRC_ABS/native/src/main.rs"
sed -i 's/^mod cli;/pub mod cli;/' "$SRC_ABS/native/src/main.rs"
sed -i 's/^mod vault;/pub mod vault;/' "$SRC_ABS/native/src/main.rs"
sed -i 's/^mod theme;/pub mod theme;/' "$SRC_ABS/native/src/main.rs"

# Make trash usage conditional on non-Android (trash crate doesn't support Android)
python3 - "$SRC_ABS/native/src/main.rs" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
s = p.read_text()

# Add conditional import for trash
old_import = 'use trash;'
new_import = '#[cfg(not(target_os = "android"))]\nuse trash;'
if new_import in s:
    print("trash import already conditional")
elif old_import in s:
    s = s.replace(old_import, new_import)
    p.write_text(s)
    print("Made trash import conditional")
else:
    print("Could not find trash import")

# Make trash usage conditional
old_usage = 'let trashed = match trash::delete(&path) {'
new_usage = '#[cfg(not(target_os = "android"))]\n        let trashed = match trash::delete(&path) {'
if new_usage in s:
    print("trash usage already conditional")
elif old_usage in s:
    s = s.replace(old_usage, new_usage)
    p.write_text(s)
    print("Made trash usage conditional")
else:
    print("Could not find trash usage")

# Also need to handle the fallback for Android
# The whole block needs to be wrapped, let's find the complete block and wrap it
old_block = '''let trashed = match trash::delete(&path) {
            Ok(()) => true,
            Err(err) => {
                eprintln!("trash::delete failed, falling back to rm: {err}");
                std::fs::remove_file(&path).is_ok()
            }
        };'''
new_block = '''#[cfg(not(target_os = "android"))]
        let trashed = match trash::delete(&path) {
            Ok(()) => true,
            Err(err) => {
                eprintln!("trash::delete failed, falling back to rm: {err}");
                std::fs::remove_file(&path).is_ok()
            }
        };
#[cfg(target_os = "android")]
        let trashed = std::fs::remove_file(&path).is_ok();'''
if new_block in s:
    print("trash block already conditional")
elif old_block in s:
    s = s.replace(old_block, new_block)
    p.write_text(s)
    print("Made trash block conditional with Android fallback")
else:
    print("Could not find trash block to patch")
PY

# Make `trash` crate optional on Android (it doesn't support Android targets)
python3 - "$SRC_ABS/native/Cargo.toml" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
s = p.read_text()
# Replace trash = "5" with a target-specific dependency
old = 'trash = "5"'
new = '# trash = "5"  # Android: moved to target-specific dependencies below'
if new in s:
    print("trash crate already patched for Android")
elif old in s:
    s = s.replace(old, new)
    # Add target-specific dependencies section if not present
    target_section = '\n[target.\'cfg(not(target_os = "android"))\'.dependencies]\ntrash = "5"\n'
    if '[target.\'cfg(not(target_os = "android"))\'.dependencies]' not in s:
        s += target_section
        print("Added target-specific dependencies for trash crate")
    p.write_text(s)
    print("Patched trash crate for Android")
else:
    print("Could not find trash dependency to patch")
PY

mkdir -p "$DEST"
cp -a "$ADAPTER/." "$DEST/"
rm -f "$DEST/prepare.sh" "$DEST/README.md"
cp -f "$ROOT/templates/android/system_fonts.rs" "$DEST/src/system_fonts.rs"

python3 "$ROOT/lib/app_icons.py" --src "$SRC_ABS" --app "$APP" --dest "$DEST/res" || true
printf 'Generated by artcraft-android-builder; custom adapter sourced from adapters/capsulecraft.\n' > "$DEST/.artcraft-android-builder-generated"

python3 - "$DEST/Cargo.toml" "$ROOT" "$APP" <<'PY'
from pathlib import Path
import sys, tomllib
path, root, app = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]
sys.path.insert(0, str(root / "lib"))
import file_associations
file_associations.patch_cargo_toml(path, app)
tomllib.loads(path.read_text())
PY
echo "Installed CapsuleCraft adapter in $DEST"

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

# Vendor the storytold egui fork and inject its Android IME bridge. Android's winit backend
# never emits IME text events: the soft keyboard's letters reach the Activity's input view
# (`onImeCommit` -> `imeQueue`) and nothing polls them, while digits/Enter/Backspace also
# arrive as ordinary key events. Without this bridge egui only receives numbers, so every
# text field ignores letters (same fix as SolidCraft, commit 24c06b1).
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

cat > "$SRC_ABS/vendor/egui/crates/egui-winit/src/ime.rs" <<'EOF'
//! High-performance native Android IME text bridge.

#[cfg(target_os = "android")]
pub mod android {
    use jni::{jni_sig, jni_str, objects::{JObject, JString}, JavaVM};

    #[expect(unsafe_code)]
    pub fn poll_all_ime_events() -> Vec<String> {
        let activity_raw = crate::android_activity::activity_raw();
        let vm_ptr: Option<*mut jni::sys::JavaVM> = crate::android_activity::vm_raw().or_else(|| {
            let ctx = ndk_context::android_context();
            let p: *mut jni::sys::JavaVM = ctx.vm().cast();
            if p.is_null() {
                None
            } else {
                Some(p)
            }
        });
        let Some(vm_ptr) = vm_ptr else {
            return Vec::new();
        };
        let Some(activity_raw) = activity_raw else {
            return Vec::new();
        };
        if activity_raw.is_null() {
            return Vec::new();
        }
        let vm = unsafe { JavaVM::from_raw(vm_ptr) };
        let res = vm.attach_current_thread(|env| -> jni::errors::Result<Vec<String>> {
            let activity = unsafe { env.as_cast_raw::<JObject<'_>>(&activity_raw)? };
            let mut list = Vec::new();
            while let Ok(val) = env.call_method(
                &*activity,
                jni_str!("pollIme"),
                jni_sig!("()Ljava/lang/String;"),
                &[],
            ) {
                let obj = val.l()?;
                if obj.is_null() {
                    break;
                }
                let jstr = JString::cast_local(env, obj)?;
                list.push(jstr.try_to_string(env)?);
            }
            Ok(list)
        });
        res.unwrap_or_default()
    }
}
EOF

python3 - "$SRC_ABS" <<'PY'
import sys
from pathlib import Path
src = Path(sys.argv[1])

# The android dependencies are already declared in the fork's egui-winit manifest; add them
# only if a fresh vendor ever lacks them (idempotent).
egui_winit_toml = src / "vendor/egui/crates/egui-winit/Cargo.toml"
if egui_winit_toml.is_file():
    txt = egui_winit_toml.read_text()
    if "target.'cfg(target_os = \"android\")'.dependencies" not in txt:
        txt = txt.rstrip() + "\n\n[target.'cfg(target_os = \"android\")'.dependencies]\njni = { version = \"0.22\", default-features = false }\nndk-context = { version = \"0.1\", default-features = false }\n"
        egui_winit_toml.write_text(txt)

# Declare the module next to the existing Android activity plumbing.
lib_rs = src / "vendor/egui/crates/egui-winit/src/lib.rs"
if lib_rs.is_file():
    txt = lib_rs.read_text()
    if "pub mod ime;" not in txt:
        target = "#[cfg(target_os = \"android\")]\npub mod android_activity;\n"
        if target in txt:
            txt = txt.replace(target, target + "#[cfg(target_os = \"android\")]\npub mod ime;\n", 1)

    # Drain the Activity's IME queue into egui input every frame. Backspace and Enter also
    # arrive as key events from winit, so emit matching key events here; everything else is
    # text (this is what makes letters work).
    ime_poll = """        #[cfg(target_os = "android")]
        {
            if self.allow_ime {
                self.egui_ctx.request_repaint_after(std::time::Duration::from_millis(16));
            }
            for ev in crate::ime::android::poll_all_ime_events() {
                if ev == "\\x08" {
                    self.egui_input.events.push(egui::Event::Key {
                        key: egui::Key::Backspace,
                        physical_key: Some(egui::Key::Backspace),
                        pressed: true,
                        repeat: false,
                        modifiers: self.modifiers,
                    });
                    self.egui_input.events.push(egui::Event::Key {
                        key: egui::Key::Backspace,
                        physical_key: Some(egui::Key::Backspace),
                        pressed: false,
                        repeat: false,
                        modifiers: self.modifiers,
                    });
                } else if ev == "\\n" {
                    self.egui_input.events.push(egui::Event::Key {
                        key: egui::Key::Enter,
                        physical_key: Some(egui::Key::Enter),
                        pressed: true,
                        repeat: false,
                        modifiers: self.modifiers,
                    });
                    self.egui_input.events.push(egui::Event::Key {
                        key: egui::Key::Enter,
                        physical_key: Some(egui::Key::Enter),
                        pressed: false,
                        repeat: false,
                        modifiers: self.modifiers,
                    });
                    self.egui_input.events.push(egui::Event::Text("\\n".to_string()));
                } else {
                    self.egui_input.events.push(egui::Event::Text(ev));
                }
            }
        }
"""
    time_stmt = "self.egui_input.time = Some(self.start_time.elapsed().as_secs_f64());\n"
    if ime_poll.strip() not in txt and time_stmt in txt:
        txt = txt.replace(time_stmt, time_stmt + "\n" + ime_poll, 1)
    lib_rs.write_text(txt)
PY

# Ensure native has a [lib] entry with rlib support
if ! grep -q '^\[lib\]' "$SRC_ABS/native/Cargo.toml"; then
    cat >> "$SRC_ABS/native/Cargo.toml" <<'EOF'

[lib]
name = "capsuleos_native"
path = "src/main.rs"
crate-type = ["cdylib", "rlib"]
EOF
else
    sed -i 's/crate-type = \["cdylib"\]/crate-type = ["cdylib", "rlib"]/' "$SRC_ABS/native/Cargo.toml"
fi

# Make NativeApp public so the Android launcher can instantiate it
sed -i 's/^struct NativeApp/pub struct NativeApp/' "$SRC_ABS/native/src/main.rs"
sed -i 's/^    fn new(launch: cli::Launch)/    pub fn new(launch: cli::Launch)/' "$SRC_ABS/native/src/main.rs"
sed -i 's/^mod cli;/pub mod cli;/' "$SRC_ABS/native/src/main.rs"
sed -i 's/^mod vault;/pub mod vault;/' "$SRC_ABS/native/src/main.rs"
sed -i 's/^mod theme;/pub mod theme;/' "$SRC_ABS/native/src/main.rs"
sed -i 's/^mod mcp;/pub mod mcp;/' "$SRC_ABS/native/src/main.rs"
sed -i 's/fn android_main(app: android_activity::AndroidApp)/fn upstream_android_main(app: android_activity::AndroidApp)/' "$SRC_ABS/native/src/main.rs"

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

android_target = '[target.\'cfg(target_os = "android")\'.dependencies]'
if android_target in s and 'rfd =' not in s.split(android_target)[1]:
    s = s.replace(android_target, android_target + '\nrfd = "0.17.2"')
    p.write_text(s)
    print("Added rfd to Android dependencies")
PY

# Disable desktop window caption buttons on Android
sed -i 's/!cfg!(target_os = "macos")/!cfg!(any(target_os = "macos", target_os = "android"))/' "$SRC_ABS/native/src/chrome.rs" || true

# Restore SAF file dialog in native/src/util/dialog.rs
cat > "$SRC_ABS/native/src/util/dialog.rs" <<'EOF'
//! File-dialog shim: delegates directly to rfd::FileDialog.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

pub struct FileDialog {
    inner: rfd::FileDialog,
}

impl Default for FileDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl FileDialog {
    pub fn new() -> Self {
        Self {
            inner: rfd::FileDialog::new(),
        }
    }

    pub fn set_title(self, title: impl Into<String>) -> Self {
        Self {
            inner: self.inner.set_title(title.into()),
        }
    }

    pub fn set_directory(self, dir: &Path) -> Self {
        Self {
            inner: self.inner.set_directory(dir),
        }
    }

    pub fn add_filter(self, name: impl Into<String>, extensions: &[&str]) -> Self {
        Self {
            inner: self.inner.add_filter(name, extensions),
        }
    }

    pub fn set_file_name(self, file_name: impl Into<String>) -> Self {
        Self {
            inner: self.inner.set_file_name(file_name),
        }
    }

    pub fn pick_file(self) -> Option<PathBuf> {
        self.inner.pick_file()
    }

    pub fn pick_files(self) -> Option<Vec<PathBuf>> {
        self.inner.pick_files()
    }

    pub fn pick_folder(self) -> Option<PathBuf> {
        self.inner.pick_folder()
    }

    pub fn save_file(self) -> Option<PathBuf> {
        self.inner.save_file()
    }
}
EOF

mkdir -p "$DEST"
cp -a "$ADAPTER/." "$DEST/"
rm -f "$DEST/prepare.sh" "$DEST/README.md"
cp -f "$ROOT/templates/android/system_fonts.rs" "$DEST/src/system_fonts.rs"
cp -f "$ROOT/templates/android/mobile_text_bar.rs" "$DEST/src/mobile_text_bar.rs"
cp -f "$ROOT/templates/android/touch_gestures.rs" "$DEST/src/touch_gestures.rs"
[[ -f "$ADAPTER/src/mcp_http.rs" ]] || cp -f "$ROOT/templates/android/mcp_http.rs" "$DEST/src/mcp_http.rs"

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

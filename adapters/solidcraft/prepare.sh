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

# Inject Android activity, IME bridge, and dependencies into vendored egui-winit
cat > "$SRC_ABS/vendor/egui/crates/egui-winit/src/android_activity.rs" <<'EOF'
//! Shared Android JNI plumbing for the `egui-winit` Android integrations.

#![cfg(target_os = "android")]

static ACTIVITY: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
static VM: std::sync::OnceLock<usize> = std::sync::OnceLock::new();

pub fn set_activity(activity_as_ptr: *mut std::ffi::c_void) {
    let _ = ACTIVITY.set(activity_as_ptr as usize);
}

pub fn set_vm(vm_as_ptr: *mut std::ffi::c_void) {
    let _ = VM.set(vm_as_ptr as usize);
}

pub fn activity_raw() -> Option<jni::sys::jobject> {
    ACTIVITY.get().copied().map(|ptr| ptr as jni::sys::jobject)
}

pub fn vm_raw() -> Option<*mut jni::sys::JavaVM> {
    VM.get().copied().map(|ptr| ptr as *mut jni::sys::JavaVM)
}
EOF

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

# Patch vendor/egui/crates/egui-winit/Cargo.toml
egui_winit_toml = src / "vendor/egui/crates/egui-winit/Cargo.toml"
if egui_winit_toml.is_file():
    txt = egui_winit_toml.read_text()
    if "target.'cfg(target_os = \"android\")'.dependencies" not in txt:
        txt = txt.rstrip() + "\n\n[target.'cfg(target_os = \"android\")'.dependencies]\njni = { version = \"0.22\", default-features = false }\nndk-context = { version = \"0.1\", default-features = false }\n"
        egui_winit_toml.write_text(txt)

# Patch vendor/egui/crates/egui-winit/src/lib.rs
lib_rs = src / "vendor/egui/crates/egui-winit/src/lib.rs"
if lib_rs.is_file():
    txt = lib_rs.read_text()
    if "pub mod android_activity;" not in txt:
        target = "mod window_settings;\n"
        replacement = target + "#[cfg(target_os = \"android\")]\npub mod android_activity;\n#[cfg(target_os = \"android\")]\npub mod ime;\n"
        if target in txt:
            txt = txt.replace(target, replacement, 1)

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

# Patch vendor/egui-wgpu/src/winit.rs
wgpu_winit = src / "vendor/egui-wgpu/src/winit.rs"
if wgpu_winit.is_file():
    txt = wgpu_winit.read_text()
    if 'cfg!(target_os = "android")' not in txt:
        target = "let Some(old_state) = self.surfaces.remove(&viewport_id) else {\n            return Ok(());\n        };\n"
        replacement = target + """
        let (requested, resizing, mut retry) = (old_state.requested, old_state.resizing, old_state.retry);

        if cfg!(target_os = "android") {
            log::info!("egui-wgpu: dropping the lost surface of {viewport_id:?} (Android)");
            return Ok(());
        }

        self.instance.poll_all(true);

        drop(old_state);
"""
        if target in txt:
            txt = txt.replace(target, replacement, 1)
            old_retry = "let mut retry = old_state.retry;\n        retry.recreated();\n        self.install_surface(\n            surface,\n            viewport_id,\n            old_state.requested.0,\n            old_state.requested.1,\n            old_state.resizing,\n            retry,"
            new_retry = "retry.recreated();\n        self.install_surface(\n            surface,\n            viewport_id,\n            requested.0,\n            requested.1,\n            resizing,\n            retry,"
            if old_retry in txt:
                txt = txt.replace(old_retry, new_retry, 1)
            wgpu_winit.write_text(txt)
PY

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
cp -f "$ROOT/templates/android/mobile_text_bar.rs" "$DEST/src/mobile_text_bar.rs"
cp -f "$ROOT/templates/android/touch_gestures.rs" "$DEST/src/touch_gestures.rs"
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

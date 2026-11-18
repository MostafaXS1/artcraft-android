#!/usr/bin/env bash
set -Eeuo pipefail
APP="${1:-solidcraft}"
TITLE="${2:-SolidCraft}"
SRC="${3:-source/solidcraft}"
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
SRC_ABS="$ROOT/$SRC"
ADAPTER="$ROOT/adapters/solidcraft"
DEST="$SRC_ABS/apps/$APP-android"

echo "Preserving SolidCraft upstream vendored egui crates..."

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

    if "ImeResetCompleted" not in txt:
        target1 = "WindowEvent::DoubleTapGesture { .. } => EventResponse {"
        repl1 = "WindowEvent::DoubleTapGesture { .. }\n            | WindowEvent::ImeResetCompleted { .. } => EventResponse {"
        if target1 in txt:
            txt = txt.replace(target1, repl1, 1)

        target2 = 'WindowEvent::ActivationTokenDone { .. } => "WindowEvent::ActivationTokenDone",\n'
        repl2 = target2 + '        WindowEvent::ImeResetCompleted { .. } => "WindowEvent::ImeResetCompleted",\n'
        if target2 in txt:
            txt = txt.replace(target2, repl2, 1)

    lib_rs.write_text(txt)

# Patch vendor/egui/crates/egui-wgpu/src/winit.rs
wgpu_winit = src / "vendor/egui/crates/egui-wgpu/src/winit.rs"
if not wgpu_winit.is_file():
    wgpu_winit = src / "vendor/egui-wgpu/src/winit.rs"
if wgpu_winit.is_file():
    txt = wgpu_winit.read_text()
    if 'cfg!(target_os = "android")' not in txt:
        target = "let Some(old_state) = self.surfaces.remove(&viewport_id) else {\n            return Ok(());\n        };\n"
        replacement = target + """
        if cfg!(target_os = "android") {
            log::info!("egui-wgpu: dropping the lost surface of {viewport_id:?} (Android)");
            return Ok(());
        }
"""
        if target in txt:
            txt = txt.replace(target, replacement, 1)
            wgpu_winit.write_text(txt)
PY

# Update SolidCraft workspace Cargo.toml
python3 - "$SRC_ABS/Cargo.toml" <<'PY'
from pathlib import Path
import sys, re, tomllib
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

# Ensure egui-winit is in [workspace.dependencies] so apps/solidcraft-android can use egui-winit = { workspace = true }
doc = tomllib.loads(s)
if "egui-winit" not in doc.get("workspace", {}).get("dependencies", {}):
    s = re.sub(
        r'(^\s*egui-wgpu\s*=\s*"0.37")',
        r'\1\negui-winit = "0.37"',
        s,
        count=1,
        flags=re.M
    )

# Ensure apps/solidcraft-android is in members
m = re.search(r'(?ms)^members\s*=\s*\[(.*?)\]', s)
if m:
    inner = m.group(1)
    if '"apps/solidcraft-android"' not in inner:
        new_inner = inner.rstrip()
        if not new_inner.endswith(','):
            new_inner += ','
        new_inner += ' "apps/solidcraft-android"'
        s = s[:m.start(1)] + new_inner + s[m.end(1):]

tomllib.loads(s)
p.write_text(s)
print("Updated SolidCraft Cargo.toml")
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

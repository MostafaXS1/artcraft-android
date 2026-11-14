#!/usr/bin/env bash
# Builder adapter for Coiman IDE (https://github.com/ManuelCoimbra/coiman-ide).
#
# Coiman is a desktop eframe/egui IDE whose `app` and `ui` modules live in a binary crate, and
# whose Git/Run/terminal features execute programs that only exist inside Termux. This script
# turns the checkout into a library plus an Android shell without hand-editing upstream code:
#
#   * coiman-ide gets a `lib.rs` exposing `app`/`ui` and the two file-opening wrappers;
#   * eframe uses wgpu and the native Activity, `portable-pty` and desktop-only pty code are
#     compiled out on Android, and the `trash` crate (which has no Android platform) is replaced
#     by a small stub that deletes permanently;
#   * `quiet_command` (every tool Coiman runs) and `which` (its installed-tool probe) are routed
#     to the Termux shim bundled as a native executable;
#   * the Activity template gets a TermuxBroker that forwards requests to Termux's official
#     RUN_COMMAND service, and the shim is compiled for arm64 with the NDK.
#
# Run by lib/build-common.sh as: prepare.sh <app> <title> <source-dir> <mode>

set -Eeuo pipefail
APP="${1:-coimanide}"
TITLE="${2:-Coiman IDE}"
SRC="${3:-source/coimanide}"
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
SRC_ABS="$ROOT/$SRC"
ADAPTER="$ROOT/adapters/coimanide"
DEST="$SRC_ABS/apps/$APP-android"

echo "Preparing $TITLE ($APP) from $SRC_ABS"

# --------------------------------------------------------------------------------------------------
# Workspace: the Android shell crate, and the Android-compatible trash replacement.
# --------------------------------------------------------------------------------------------------
python3 - "$SRC_ABS/Cargo.toml" <<'PY'
from pathlib import Path
import re, sys, tomllib

p = Path(sys.argv[1])
s = p.read_text()
m = re.search(r'(?ms)^members\s*=\s*\[(.*?)\]', s)
if not m:
    raise SystemExit("Coiman's workspace Cargo.toml has no members list")
inner = m.group(1)
if '"apps/coimanide-android"' not in inner:
    new_inner = inner.rstrip()
    if not new_inner.endswith(","):
        new_inner += ","
    new_inner += ' "apps/coimanide-android"'
    s = s[: m.start(1)] + new_inner + s[m.end(1):]

entry = 'trash = { path = "vendor/trash" }'
header = "[patch.crates-io]"
if header in s:
    parts = s.split(header, 1)
    next_sec = re.search(r"\n\[", parts[1])
    patch_sec = parts[1][: next_sec.start()] if next_sec else parts[1]
    rest = parts[1][next_sec.start():] if next_sec else ""
    patch_sec = re.sub(r"^\s*trash\s*=\s*\{.*?\}\s*(?:#.*)?\n?", "", patch_sec, flags=re.M)
    s = parts[0] + header + "\n" + entry + "\n" + patch_sec.lstrip("\n") + rest
else:
    s = s.rstrip() + "\n\n" + header + "\n" + entry + "\n"

# The builder's Android egui-winit (patched by lib/build-common.sh into this section) and the
# Android clipboard/IME/touch handling it carries are the storytold egui fork's build. Coiman
# depends on the crates.io egui/eframe 0.37, whose eframe calls the 5-argument
# `process_viewport_commands` and has no `clipboard_image`, so it cannot compile against that
# egui-winit. Patch the whole egui stack to the same fork revision every other builder app uses;
# build-common.sh then redirects egui-winit/egui-wgpu to its vendored copies (and creates the
# matching `[patch."https://github.com/storytold/egui.git"]` section from the URL above).
egui_rev = "6d518fc799681571963842f95b55a6b68431b15a"
# epaint/ecolor/emath too: the vendored egui-wgpu refers to `epaint::ColorImage`, and without the
# fork's epaint it would be a second, crates.io copy of the type.
for crate in ("egui", "eframe", "epaint", "ecolor", "emath"):
    patch_line = f'{crate} = {{ git = "https://github.com/storytold/egui.git", rev = "{egui_rev}" }}'
    existing = re.search(rf"(?m)^\s*{re.escape(crate)}\s*=\s*\{{.*?\}}\s*$", s)
    if existing:
        s = s[: existing.start()] + patch_line + s[existing.end():]
    else:
        s = s.replace(header, header + "\n" + patch_line, 1)
tomllib.loads(s)
p.write_text(s)
print("workspace: added apps/coimanide-android, the trash patch and the egui fork")
PY

mkdir -p "$SRC_ABS/vendor"
rm -rf "$SRC_ABS/vendor/trash"
cp -a "$ADAPTER/vendor/trash" "$SRC_ABS/vendor/trash"

# --------------------------------------------------------------------------------------------------
# coiman-ide: library target, Android eframe features.
# --------------------------------------------------------------------------------------------------
python3 - "$SRC_ABS/crates/coiman-ide/Cargo.toml" <<'PY'
from pathlib import Path
import re, sys, tomllib

p = Path(sys.argv[1])
s = p.read_text()

# eframe: the desktop build uses glow; Android uses wgpu and the native Activity. `accesskit`
# adds a second accessibility event loop (and an Android touch-exploration service) that the
# touch-first shell does not need; the other adapter launchers leave it out too.
line = ('eframe = { version = "0.37.0", default-features = false, features = ["wgpu", '
        '"clipboard", "default_fonts", "links", "android-native-activity"] }')
if re.search(r"(?m)^eframe\s*=", s):
    s = re.sub(r"(?m)^eframe\s*=.*$", line, s, count=1)
else:
    s = s.replace("[dependencies]\n", "[dependencies]\n" + line + "\n", 1)

if not re.search(r"(?m)^\[lib\]", s):
    s = s.rstrip() + """

[lib]
name = "coiman_ide"
path = "src/lib.rs"
crate-type = ["rlib"]
"""
tomllib.loads(s)
p.write_text(s)
print("coiman-ide: wgpu + native Activity, library target")
PY

# A library form of the binary: the same modules and core re-exports, plus the two entry points
# the Android shell needs (the app's own open_file/open_folder are crate-private).
python3 - "$SRC_ABS/crates/coiman-ide/src/main.rs" "$SRC_ABS/crates/coiman-ide/src/lib.rs" <<'PY'
from pathlib import Path
import re, sys

main = Path(sys.argv[1]).read_text()
match = re.search(r"(?ms)^pub\(crate\) use coiman_core::\{.*?\};", main)
if not match:
    raise SystemExit("main.rs no longer has the coiman_core re-export line")
reexport = match.group(0)

lib = '''//! Library form of Coiman IDE, used by the Android shell in `apps/coimanide-android`.
//!
//! The desktop binary keeps its own `main.rs`; this exposes the very same modules so the Android
//! launcher can draw `app::CodeApp` inside its own eframe shell and hand it files the system
//! opened. It also re-exports the core modules under the names the app's `crate::buffer`,
//! `crate::document`, ... paths expect, exactly as `main.rs` does.

pub mod android_keyboard;
pub mod app;
pub mod ui;

''' + reexport + '''

use std::path::{Path, PathBuf};

/// Opens a file the system handed the app (SAF staged a copy in app storage).
pub fn open_file(app: &mut app::CodeApp, path: &Path) {
    app.open_file(path);
}

/// Opens a project folder (the real path behind a SAF tree).
pub fn open_folder(app: &mut app::CodeApp, path: PathBuf) {
    app.open_folder(path);
}
'''
Path(sys.argv[2]).write_text(lib)
print("coiman-ide: src/lib.rs written")
PY

# --------------------------------------------------------------------------------------------------
# coiman-core: Termux tool routing, Android pty, portable-pty off Android.
# --------------------------------------------------------------------------------------------------
python3 - "$SRC_ABS/crates/coiman-core/Cargo.toml" <<'PY'
from pathlib import Path
import re, sys, tomllib

p = Path(sys.argv[1])
s = p.read_text()

# Remove the desktop-only pty crate from `[dependencies]` (not the target section this script
# appends below).
deps = re.search(r"(?ms)^\[dependencies\]\n(.*?)(?=^\[|\Z)", s)
if deps and re.search(r"(?m)^\s*portable-pty\s*=", deps.group(1)):
    block = re.sub(r"(?m)^\s*portable-pty\s*=.*\n?", "", deps.group(1), count=1)
    s = s[: deps.start(1)] + block + s[deps.end(1):]

desktop_pty = "[target.'cfg(not(target_os = \"android\"))'.dependencies]"
android_libc = "[target.'cfg(target_os = \"android\")'.dependencies]"


def ensure_section(header_line, content_line):
    global s
    if header_line not in s:
        s = s.rstrip() + "\n\n" + header_line + "\n" + content_line + "\n"
        return
    match = re.search(rf"(?ms)^{re.escape(header_line)}\n(.*?)(?=^\[|\Z)", s)
    if match and content_line not in match.group(1):
        s = s[: match.start(1)] + content_line + "\n" + match.group(1) + s[match.end(1):]


# The desktop terminal opens a pty; Android has no fork/pty for another app's binaries and the
# terminal is relayed through Termux instead (see src/terminal/pty_android.rs). The Android
# terminal session signals its child (the Termux shim) with libc::kill.
ensure_section(desktop_pty, 'portable-pty = "0.9.0"')
ensure_section(android_libc, 'libc = "0.2"')
tomllib.loads(s)
p.write_text(s)
print("coiman-core: portable-pty is desktop-only, libc for Android")
PY

cp -f "$ADAPTER/patches/android_termux.rs" "$SRC_ABS/crates/coiman-core/src/android_termux.rs"
cp -f "$ADAPTER/patches/pty_android.rs" "$SRC_ABS/crates/coiman-core/src/terminal/pty_android.rs"
# The editor and the terminal publish their selection through this module; upstream does not have
# it, so the adapter owns the file.
cp -f "$ADAPTER/patches/selection.rs" "$SRC_ABS/crates/coiman-core/src/selection.rs"

python3 - "$SRC_ABS/crates/coiman-core/src/lib.rs" "$SRC_ABS/crates/coiman-core/src/terminal/pty.rs" \
    "$SRC_ABS/crates/coiman-core/src/language/lsp/discovery.rs" "$SRC_ABS/crates/coiman-core/src/language/lsp/servers.rs" <<'PY'
from pathlib import Path
import sys

root, pty, discovery, servers = (Path(arg) for arg in sys.argv[1:])

lib = root.read_text()
changed = False

# --- the module that tells the text bar where the selection is --------------------------------
if "pub mod selection;" not in lib:
    anchor = "pub mod analysis;"
    if anchor not in lib:
        raise SystemExit("coiman-core/src/lib.rs no longer contains 'pub mod analysis;'; cannot register selection")
    lib = lib.replace(anchor, anchor + "\npub mod selection;", 1)
    changed = True

# --- the module that knows which programs Termux provides -------------------------------------
if "pub mod android_termux;" not in lib:
    anchor = "pub mod analysis;"
    if anchor not in lib:
        raise SystemExit("coiman-core/src/lib.rs no longer contains 'pub mod analysis;'; cannot register android_termux")
    lib = lib.replace(
        anchor,
        anchor + "\n#[cfg(target_os = \"android\")]\npub mod android_termux;",
        1,
    )
    changed = True

if changed:
    root.write_text(lib)
    print("coiman-core: selection and android_termux modules registered")

# --- pty.rs: the desktop Session is compiled out on Android -----------------------------------
text = pty.read_text()


def gate(old, new, count=1):
    global text
    if new in text:
        return
    if old not in text:
        raise SystemExit(f"pty.rs no longer contains: {old[:60]!r}")
    text = text.replace(old, new, count)


gate(
    "use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};",
    '#[cfg(not(target_os = "android"))]\n'
    "use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};",
)
gate("impl Size {\n    fn pty(self) -> PtySize {",
     '#[cfg(not(target_os = "android"))]\nimpl Size {\n    fn pty(self) -> PtySize {')
# `Read` and `mpsc::channel` are only used by the desktop session.
gate("use std::io::{Read, Write};",
     '#[cfg(not(target_os = "android"))]\nuse std::io::Read;\nuse std::io::Write;')
gate("use std::sync::mpsc::{self, Receiver, Sender};",
     '#[cfg(not(target_os = "android"))]\nuse std::sync::mpsc;\nuse std::sync::mpsc::{Receiver, Sender};')
gate("fn merged_path(", '#[cfg(not(target_os = "android"))]\nfn merged_path(')
gate("fn split_command(", '#[cfg(not(target_os = "android"))]\nfn split_command(')
gate("pub struct Session {", '#[cfg(not(target_os = "android"))]\npub struct Session {')
gate("impl Session {", '#[cfg(not(target_os = "android"))]\nimpl Session {')
gate("impl Drop for Session {", '#[cfg(not(target_os = "android"))]\nimpl Drop for Session {')
gate("fn kill_tree(pid: u32) {", '#[cfg(not(target_os = "android"))]\nfn kill_tree(pid: u32) {')
gate("#[cfg(test)]\nmod tests {", '#[cfg(all(test, not(target_os = "android")))]\nmod tests {')

tail = '''
#[cfg(target_os = "android")]
#[path = "pty_android.rs"]
mod android_impl;
#[cfg(target_os = "android")]
pub use android_impl::*;
'''
if "mod android_impl;" not in text:
    text = text.rstrip() + "\n" + tail
pty.write_text(text)
print("pty.rs: desktop pty gated, Android session module attached")

# --- discovery.rs: quiet_command and which know about Termux ----------------------------------
text = discovery.read_text()
if "crate::android_termux::tool_for" not in text:
    old = """pub fn quiet_command(program: &Path) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);"""
    new = """pub fn quiet_command(program: &Path) -> Command {
    // Android has none of Coiman's tools in its own sandbox: Termux has them. The bundled shim
    // turns one Termux RUN_COMMAND invocation into a normal child process (stdin/stdout/stderr),
    // so every caller keeps working unchanged.
    #[cfg(target_os = "android")]
    if let Some(tool) = crate::android_termux::tool_for(program) {
        if let Some(shim) = crate::android_termux::shim() {
            let mut forwarded = Command::new(shim);
            forwarded.env("COIMAN_TERMUX_TOOL", tool);
            return forwarded;
        }
    }
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);"""
    if old not in text:
        raise SystemExit("quiet_command no longer has the expected shape")
    text = text.replace(old, new, 1)
    discovery.write_text(text)
    print("discovery.rs: quiet_command forwards Termux tools")

text = servers.read_text()
if "crate::android_termux::tool_for" not in text:
    old = """pub fn which(name: &str) -> Option<PathBuf> {
    let exts: &[&str] = if cfg!(windows) { &[".exe", ".cmd", ".bat", ""] } else { &[""] };"""
    new = """pub fn which(name: &str) -> Option<PathBuf> {
    // Termux's bin directory belongs to another app (mode 700), so `is_file` can never see its
    // tools. Treat the programs the shim forwards as installed: quiet_command routes them to
    // Termux, and a missing package then produces Termux's "pkg install ..." hint.
    #[cfg(target_os = "android")]
    if let Some(tool) = crate::android_termux::tool_for(Path::new(name)) {
        return Some(PathBuf::from(tool));
    }
    let exts: &[&str] = if cfg!(windows) { &[".exe", ".cmd", ".bat", ""] } else { &[""] };"""
    if old not in text:
        raise SystemExit("which no longer has the expected shape")
    text = text.replace(old, new, 1)
    servers.write_text(text)
    print("servers.rs: which sees Termux tools")
PY

# Coiman draws desktop minimize/maximize/close buttons in its title bar. Android owns the window
# (and has no desktop decoration buttons), so leave them out.
python3 - "$SRC_ABS/crates/coiman-ide/src/app/shell/workbench.rs" <<'PY'
from pathlib import Path
import sys

p = Path(sys.argv[1])
s = p.read_text()
old = "fn window_buttons(ui: &mut egui::Ui, bar: egui::Rect, maximized: bool) {\n"
new = ("#[allow(unreachable_code, unused_variables)]\n"
       "fn window_buttons(ui: &mut egui::Ui, bar: egui::Rect, maximized: bool) {\n"
       "    // Android has no desktop caption buttons: the system owns the window.\n"
       "    #[cfg(target_os = \"android\")]\n"
       "    return;\n")
if "#[allow(unreachable_code, unused_variables)]\nfn window_buttons" in s:
    print("workbench.rs: window buttons already hidden on Android")
elif "Android has no desktop caption buttons" in s:
    # An earlier, simpler patch: bring it up to the current form.
    s = s.replace(
        "fn window_buttons(ui: &mut egui::Ui, bar: egui::Rect, maximized: bool) {\n",
        "#[allow(unreachable_code, unused_variables)]\n"
        "fn window_buttons(ui: &mut egui::Ui, bar: egui::Rect, maximized: bool) {\n",
        1,
    )
    p.write_text(s)
    print("workbench.rs: window buttons hidden on Android")
elif old in s:
    p.write_text(s.replace(old, new, 1))
    print("workbench.rs: window buttons hidden on Android")
else:
    raise SystemExit("workbench.rs no longer has window_buttons")
PY

# The soft-keyboard button in the bottom bar.
#
# Coiman's editor draws its own text and never asks Android for the soft keyboard: a keyboard
# that opens under every tap in the text area is in the way. On a tablet the user still needs
# the keyboard at hand, so the bottom bar (the status bar) gets a button that shows it and says
# what the app does with it: always on, never on its own, or as the app sees fit.
#
# The button itself is the builder's own module: it lives in Coiman's crate because that is
# where the status bar is drawn, and the Android shell reads the mode and the user's requests
# from it (only the shell can talk to the platform).
cp -f "$ADAPTER/patches/android_keyboard.rs" "$SRC_ABS/crates/coiman-ide/src/android_keyboard.rs"

python3 - "$SRC_ABS/crates/coiman-ide/src/app/shell/workbench.rs" <<'PY'
from pathlib import Path
import sys

p = Path(sys.argv[1])
s = p.read_text()

if "crate::android_keyboard::button(ui);" in s:
    print("workbench.rs: the keyboard button is already in the status bar")
else:
    # The button goes at the right end of the status bar: the bar's right-hand group is laid
    # out right to left, so the first widget in it is the one furthest right. Anchor on the
    # status-bar function rather than an exact block, which upstream keeps changing.
    start = s.find("pub(crate) fn status_bar(")
    layout = "ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {"
    idx = s.find(layout, start) if start >= 0 else -1
    if idx < 0:
        raise SystemExit("workbench.rs no longer has the status bar's right-hand group")
    insert_at = idx + len(layout)
    button = (
        "\n                // The soft keyboard is an Android thing; the desktop has no such button.\n"
        "                #[cfg(target_os = \"android\")]\n"
        "                crate::android_keyboard::button(ui);\n"
    )
    p.write_text(s[:insert_at] + button + s[insert_at:])
    print("workbench.rs: the keyboard button is in the status bar")
PY

# The command palette closes on a press outside its own area. On Android a tap arrives as a
# press and a release in the same frame, so the tap on Explorer's "New File"/"New Folder"/
# "Rename" opens the field and that very press then reads as "outside", closing it again before
# it can be seen. The palette has to ignore presses for the frame it opened in.
python3 - "$SRC_ABS/crates/coiman-ide/src/ui/palette.rs" <<'PY'
from pathlib import Path
import sys

p = Path(sys.argv[1])
s = p.read_text()

# The input field clears `just_opened` while placing the caret, so remember it first.
input_fn = """    fn input_ui(&mut self, ctx: &egui::Context, kind: InputKind, placeholder: &str) -> Option<PaletteResult> {
        let enter = ctx.input(|i| i.key_pressed(Key::Enter));
"""
input_fn_patched = """    fn input_ui(&mut self, ctx: &egui::Context, kind: InputKind, placeholder: &str) -> Option<PaletteResult> {
        // Whether this is the frame the field opened in, before the focus block below clears it.
        let just_opened = self.just_opened;
        let enter = ctx.input(|i| i.key_pressed(Key::Enter));
"""
if input_fn_patched in s:
    print("palette.rs: the input already remembers the frame it opened in")
elif input_fn in s:
    s = s.replace(input_fn, input_fn_patched, 1)
    print("palette.rs: the input remembers the frame it opened in")
else:
    raise SystemExit("palette.rs no longer has input_ui's opening")

input_close = """        self.close_on_click_outside(ctx, area.response.rect);
        if enter && !self.query.trim().is_empty() {
"""
input_close_patched = """        // On Android the tap that opened the field is a press in this same frame, and it is
        // outside the just-drawn area: it must not close the field.
        if !just_opened {
            self.close_on_click_outside(ctx, area.response.rect);
        }
        if enter && !self.query.trim().is_empty() {
"""
if input_close_patched in s:
    print("palette.rs: the input already ignores its opening press")
elif input_close in s:
    s = s.replace(input_close, input_close_patched, 1)
    print("palette.rs: the input ignores its opening press")
else:
    raise SystemExit("palette.rs no longer closes the input on an outside press")

# Pickers opened from a tap (a bookmark, a recent file) are shown in the same frame too.
ui_open = """        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
            self.open = false;
            return None;
        }

        if let Some((kind, placeholder)) = self.input {
"""
ui_open_patched = """        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
            self.open = false;
            return None;
        }
        // Before the list's focus block clears it: whether this is the frame it opened in.
        let just_opened = self.just_opened;

        if let Some((kind, placeholder)) = self.input {
"""
if "let just_opened = self.just_opened;\n\n        if let Some((kind, placeholder)) = self.input" in s:
    print("palette.rs: the list already remembers the frame it opened in")
elif ui_open in s:
    s = s.replace(ui_open, ui_open_patched, 1)
    print("palette.rs: the list remembers the frame it opened in")
else:
    raise SystemExit("palette.rs no longer has the palette's opening")

list_close = """        self.close_on_click_outside(ctx, area.response.rect);
        let picked = result.map(|i| items.swap_remove(i).2);
"""
list_close_patched = """        // The tap that opened a picker can be a press outside its area in this same frame.
        if !just_opened {
            self.close_on_click_outside(ctx, area.response.rect);
        }
        let picked = result.map(|i| items.swap_remove(i).2);
"""
if list_close_patched in s:
    print("palette.rs: the list already ignores its opening press")
elif list_close in s:
    s = s.replace(list_close, list_close_patched, 1)
    print("palette.rs: the list ignores its opening press")
else:
    raise SystemExit("palette.rs no longer closes the list on an outside press")

p.write_text(s)
PY

# The editor on Android: a tap has to place the cursor, and the platform's text bar
# (Cut/Copy/Paste/Delete) has to know about the selection.
#
# The editor publishes its IME output while it has focus: that is what tells Android where the
# text is and lets the soft keyboard follow it in Auto mode. The shell decides whether the
# keyboard is allowed (Off suppresses this output; On keeps one published even without focus).
python3 - "$SRC_ABS/crates/coiman-ide/src/ui/editor_view.rs" <<'PY'
from pathlib import Path
import sys

p = Path(sys.argv[1])
s = p.read_text()

# The upstream block uses crates.io egui 0.37's IMEOutput; the build uses the storytold fork,
# whose IMEOutput carries the owner/generation fields too.
ime_block_pristine = """    if has_focus {
        let to_global = ui.ctx().layer_transform_to_global(ui.layer_id()).unwrap_or_default();
        ui.output_mut(|o| {
            o.ime = Some(egui::output::IMEOutput {
                purpose: egui::IMEPurpose::Normal,
                rect: to_global * rect,
                cursor_rect: to_global * cursor_rect,
                should_interrupt_composition: false,
            });
        });
    }
"""
ime_block_patched = """    if has_focus {
        let to_global = ui.ctx().layer_transform_to_global(ui.layer_id()).unwrap_or_default();
        ui.output_mut(|o| {
            o.ime = Some(egui::output::IMEOutput {
                owner: egui::Id::NULL,
                context_generation: 0,
                purpose: egui::IMEPurpose::Normal,
                rect: to_global * rect,
                cursor_rect: to_global * cursor_rect,
                should_interrupt_composition: false,
                reset_request: None,
                change_cause: egui::output::ImeChangeCause::Other,
                surrounding_text: egui::output::IMESurroundingText::Unavailable,
            });
        });
    }
"""
widget_info = '    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Editor"));'
if ime_block_patched in s:
    print("editor_view.rs: the editor asks for the soft keyboard when it has focus")
elif ime_block_pristine in s:
    s = s.replace(ime_block_pristine, ime_block_patched, 1)
    print("editor_view.rs: the editor asks for the soft keyboard when it has focus")
elif widget_info in s:
    # An earlier build removed the block; put the fork's version back in the same place.
    s = s.replace(widget_info, ime_block_patched + widget_info, 1)
    print("editor_view.rs: the editor asks for the soft keyboard again when it has focus")
else:
    raise SystemExit("the editor no longer has the IME output's anchor")

# A tap on Android is a press and a release in the same frame: the builder's touch handling only
# knows it is a tap once the finger lifts, and egui clears "the button is down on this widget" on
# the release. The editor placed the cursor on the press, so on a tablet every tap was ignored and
# the cursor stayed where it was. `clicked` is the flag that survives the same-frame release.
old_press = "    let pressed = response.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_pressed());"
new_press = (
    "    // A tap on Android presses and releases in the same frame, and egui clears \"the button is\n"
    "    // down on this widget\" on the release: `clicked` is what is still true afterwards.\n"
    "    let pressed =\n"
    "        (response.is_pointer_button_down_on() || response.clicked()) && ui.input(|i| i.pointer.primary_pressed());"
)
if new_press in s:
    print("editor_view.rs: a tap already counts as a press")
elif old_press in s:
    s = s.replace(old_press, new_press, 1)
    print("editor_view.rs: a tap counts as a press")
else:
    raise SystemExit("editor_view.rs no longer finds the pointer press")

# A tap in the editor is the user saying "type here": in Auto mode the shell brings the
# keyboard back when the user dismissed it.
tap_anchor = """    if response.clicked() || response.drag_started() {
        response.request_focus();
    }
"""
tap_patched = """    if response.clicked() || response.drag_started() {
        response.request_focus();
        // A tap is the user saying "type here": in Auto, the keyboard comes back if it was
        // dismissed.
        #[cfg(target_os = "android")]
        crate::android_keyboard::text_tapped();
    }
"""
if "crate::android_keyboard::text_tapped();" in s:
    print("editor_view.rs: a tap already asks for the soft keyboard again")
elif tap_anchor in s:
    s = s.replace(tap_anchor, tap_patched, 1)
    print("editor_view.rs: a tap asks for the soft keyboard again")
else:
    raise SystemExit("editor_view.rs no longer focuses the editor on a tap")


# The platform's text bar looks for a TextEdit's selection to decide whether to offer
# Cut/Copy/Paste/Delete. This editor is not a TextEdit, so it tells the bar about its own
# selection: an empty range when nothing is selected, the selection otherwise.
old_selection = """    Output {
        hover,
        ctrl_click: view.ctrl_click.take(),"""
new_selection = """    // The Android text bar (Cut/Copy/Paste/Delete) looks for a TextEdit's selection; this editor
    // is not a TextEdit, so it tells the bar about the selection itself: the selection itself,
    // and where it is, so the bar sits next to the text instead of at the top of the screen.
    let selection = doc.buffer.primary();
    let mut state = egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
        egui::text::CCursor::new(selection.anchor),
        egui::text::CCursor::new(selection.head),
    )));
    state.store(ui.ctx(), id);
    // Where the selection is: the cursor's cell, so the platform's text bar sits right by the
    // text that is selected (the whole line would put it at the far end of the editor).
    let row_y = text_rect.top() + cursor_row as f32 * row_h - view.scroll.y;
    let cursor_x = text_rect.left() + cursor_x - view.scroll.x;
    coiman_core::selection::set(if has_focus && selection.anchor != selection.head {
        Some([cursor_x, row_y, cursor_x + char_w * 6.0, row_y + row_h])
    } else {
        None
    });

    Output {
        hover,
        ctrl_click: view.ctrl_click.take(),"""
if new_selection in s:
    print("editor_view.rs: the selection already reaches the text bar")
elif old_selection in s:
    s = s.replace(old_selection, new_selection, 1)
    print("editor_view.rs: the selection reaches the text bar")
else:
    raise SystemExit("editor_view.rs no longer returns an Output")

p.write_text(s)
PY

# The terminal on Android: the platform's text bar (Cut/Copy/Paste/Delete) has to know about the
# selection, and the terminal is a text surface, so it publishes its IME output while it has
# focus. The shell decides whether the keyboard is allowed (Off suppresses the output; On keeps
# one published even without focus).
python3 - "$SRC_ABS/crates/coiman-ide/src/ui/terminal.rs" <<'PY'
from pathlib import Path
import re, sys

p = Path(sys.argv[1])
s = p.read_text()

block = """        // The Android text bar (Cut/Copy/Paste/Delete) looks for a TextEdit's selection; the
        // terminal is not a TextEdit, so it tells the bar about the selection itself: whether
        // there is one, and where it is, so the bar sits next to the text.
        {
            let selected = {
                let term = tab.session.term();
                term.selection.is_some()
            };
            let mut state = egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
            state.cursor.set_char_range(Some(if selected {
                egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(1),
                )
            } else {
                egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(0),
                )
            }));
            state.store(ui.ctx(), id);
            // Where the selection is: the pointer is where the selection ended (the terminal's
            // selection is made with the mouse), so the bar goes right above it.
            coiman_core::selection::set(if selected && focused {
                ui.input(|i| i.pointer.interact_pos())
                    .map(|p| [p.x - 1.0, p.y - 10.0, p.x + 1.0, p.y + 10.0])
            } else {
                None
            });
        }
"""

# Any earlier version of this block goes: it is written once, in one shape.
s = re.sub(r"\n        // The Android text bar \(Cut/Copy/Paste/Delete\).*?\n        \}\n", "\n", s, flags=re.S)

anchor = "        paint(ui, &tab.session, &grid, &font, focused, rect);\n"
if anchor not in s:
    raise SystemExit("terminal.rs no longer paints the terminal grid")
if "coiman_core::selection::set" not in s:
    s = s.replace(anchor, anchor + block, 1)

# The IME output opens the soft keyboard in Auto mode and gives the platform the cursor's cell;
# without it, egui-winit would close the keyboard again while the terminal is being typed in.
marker = "// The terminal is a text surface: publish its IME output while it has focus."
# Replace any earlier version of the block: it is written once, in one shape.
s = re.sub(
    r"\n        // The terminal is a text surface: publish its IME output while it has focus\."
    r".*?\n        \}\n",
    "\n",
    s,
    flags=re.S,
)
ime_block = """        // The terminal is a text surface: publish its IME output while it has focus.
        if focused {
            let term = tab.session.term();
            let content = term.renderable_content();
            let point = content.cursor.point;
            let row = point.line.0 + content.display_offset as i32;
            let to_global = ui.ctx().layer_transform_to_global(ui.layer_id()).unwrap_or_default();
            let cursor_pos = egui::pos2(
                text_rect.left() + point.column.0 as f32 * char_w,
                text_rect.top() + row as f32 * row_h,
            );
            let cursor_rect = egui::Rect::from_min_size(cursor_pos, egui::vec2(char_w, row_h));
            ui.output_mut(|o| {
                o.ime = Some(egui::output::IMEOutput {
                    owner: id,
                    context_generation: 0,
                    purpose: egui::IMEPurpose::Terminal,
                    rect: to_global * rect,
                    cursor_rect: to_global * cursor_rect,
                    should_interrupt_composition: false,
                    reset_request: None,
                    change_cause: egui::output::ImeChangeCause::Other,
                    surrounding_text: egui::output::IMESurroundingText::Unavailable,
                });
            });
        }
"""
s = s.replace(anchor, anchor + ime_block, 1)

# A tap in the terminal is the user saying "type here": in Auto mode the shell brings the
# keyboard back when the user dismissed it.
tap_anchor = """        if focus_requested || response.clicked() || response.drag_started() {
            response.request_focus();
        }
"""
tap_patched = """        if focus_requested || response.clicked() || response.drag_started() {
            response.request_focus();
            // A tap is the user saying "type here": in Auto, the keyboard comes back if it was
            // dismissed.
            #[cfg(target_os = "android")]
            crate::android_keyboard::text_tapped();
        }
"""
if "crate::android_keyboard::text_tapped();" not in s:
    if tap_anchor not in s:
        raise SystemExit("terminal.rs no longer focuses the terminal on a tap")
    s = s.replace(tap_anchor, tap_patched, 1)
    print("terminal.rs: a tap asks for the soft keyboard again")
p.write_text(s)
print("terminal.rs: the selection reaches the text bar, and the terminal asks for the soft keyboard")
PY

# egui's text cursor lives in `egui::text` (the `cursor` module is epaint's and is not
# re-exported under it), so the selection the editor and the terminal publish uses that path.
python3 - "$SRC_ABS/crates/coiman-ide/src/ui/editor_view.rs" "$SRC_ABS/crates/coiman-ide/src/ui/terminal.rs" <<'PY'
from pathlib import Path
import sys

for name in sys.argv[1:]:
    p = Path(name)
    s = p.read_text()
    if "egui::text::cursor::CCursor" in s:
        p.write_text(s.replace("egui::text::cursor::CCursor", "egui::text::CCursor"))
        print(f"{p.name}: the text cursor path is fixed")
PY

# The caption buttons (minimize, maximize, close) are desktop-only; Android owns the window. The
# bar still reserved the space they would take, which left an empty strip at the right of the
# title bar. Give that space back.
python3 - "$SRC_ABS/crates/coiman-ide/src/app/shell/workbench.rs" <<'PY'
from pathlib import Path
import sys

p = Path(sys.argv[1])
s = p.read_text()
if "fn window_buttons_space() -> f32 {" in s:
    print("workbench.rs: the caption buttons' space is already given back")
    raise SystemExit(0)

count = s.count("3.0 * WINDOW_BUTTON")
if count == 0:
    raise SystemExit("workbench.rs no longer reserves the caption buttons' space")
s = s.replace("3.0 * WINDOW_BUTTON", "window_buttons_space()")

helper = """/// The width the caption buttons take at the right of the bar.
///
/// Android has none: the system owns the window, so the space goes back to the bar.
fn window_buttons_space() -> f32 {
    if cfg!(target_os = "android") {
        0.0
    } else {
        3.0 * WINDOW_BUTTON
    }
}

/// Size for secondary workbench text (breadcrumbs, status bar): 12/13 of the UI font.
const WINDOW_BUTTON: f32 = 46.0;"""
old_const = """/// Size for secondary workbench text (breadcrumbs, status bar): 12/13 of the UI font.
const WINDOW_BUTTON: f32 = 46.0;"""
if old_const not in s:
    raise SystemExit("workbench.rs no longer has the WINDOW_BUTTON constant")
s = s.replace(old_const, helper, 1)
p.write_text(s)
print(f"workbench.rs: {count} places now leave the caption buttons' space to the bar")
PY

# The themes the user has in VS Code, bundled with the app.
#
# Coiman's theme picker reads VS Code color themes (it has its own VS Code theme loader), and it
# finds them in the user's VS Code extensions folder or in `<config>/themes`. On a tablet there
# is no VS Code, so the themes are bundled as assets, unpacked by the Activity and copied into
# Coiman's config folder, where the picker lists them.
# Theme injection comes from the builder (adapter), not the source checkout.
THEMES_DIR="$ADAPTER/themes"
ADAPTER_ASSETS_DIR="$ADAPTER/assets"
if [[ -d "$THEMES_DIR" ]] && [[ -n "$(ls -A "$THEMES_DIR" 2>/dev/null)" ]]; then
    rm -rf "$ADAPTER_ASSETS_DIR/coiman-themes"
    mkdir -p "$ADAPTER_ASSETS_DIR/coiman-themes"
    cp "$THEMES_DIR"/*.json "$ADAPTER_ASSETS_DIR/coiman-themes/"
    echo "Bundled $(ls -1 "$ADAPTER_ASSETS_DIR/coiman-themes" | wc -l) themes with the app."
fi

# The Android shell crate itself.
# --------------------------------------------------------------------------------------------------
mkdir -p "$DEST"
# The staged launcher is rebuilt from scratch every time: a leftover file from an earlier build
# (or from the builder's own generated launcher) must not survive into the APK.
rm -rf "$DEST"
mkdir -p "$DEST"
cp -a "$ADAPTER/." "$DEST/"
rm -f "$DEST/prepare.sh" "$DEST/README.md"
# The shim sources and patches are only inputs to this script; keep the staged launcher small.
rm -rf "$DEST/shim" "$DEST/patches" "$DEST/vendor"
cp -f "$ROOT/templates/android/system_fonts.rs" "$DEST/src/system_fonts.rs"
cp -f "$ROOT/templates/android/mobile_text_bar.rs" "$DEST/src/mobile_text_bar.rs"
cp -f "$ROOT/templates/android/touch_gestures.rs" "$DEST/src/touch_gestures.rs"

# Coiman's text bar is the builder's, with one change: it sits next to the selected text instead
# of at the top of the screen (Coiman's editor and terminal draw their own text, so the widget's
# own rectangle is the whole area).
cp -f "$ROOT/templates/android/mobile_text_bar.rs" "$DEST/src/coiman_text_bar.rs"
python3 - "$DEST/src/coiman_text_bar.rs" <<'PY'
from pathlib import Path
import sys

p = Path(sys.argv[1])
s = p.read_text()
old = (
    "    // Determine field geometry (prefer IME output rect, fallback to response rect)\n"
    "    let field_rect = ctx\n"
    "        .output(|o| o.ime.as_ref().map(|i| i.rect))\n"
    "        .or_else(|| ctx.read_response(field_id).map(|r| r.rect));"
)
new = (
    "    // Determine field geometry. Coiman's editor and its terminal draw their own text, so they\n"
    "    // are not TextEdits: their own rectangle is the whole area, which would put the bar at the\n"
    "    // top of the screen. They publish where the selection is (coiman_core::selection), and the\n"
    "    // bar goes there: next to the text that is selected.\n"
    "    let field_rect = coiman_core::selection::get()\n"
    "        .map(|r| egui::Rect::from_min_max(egui::pos2(r[0], r[1]), egui::pos2(r[2], r[3])))\n"
    "        .or_else(|| ctx.output(|o| o.ime.as_ref().map(|i| i.rect)))\n"
    "        .or_else(|| ctx.read_response(field_id).map(|r| r.rect));"
)
if new in s:
    print("coiman_text_bar.rs: already sits next to the selection")
elif old in s:
    p.write_text(s.replace(old, new, 1))
    print("coiman_text_bar.rs: it sits next to the selection")
else:
    raise SystemExit("the text bar no longer finds the field geometry")
PY

# The Java Activity: the builder's SAF template plus the Termux broker.
mkdir -p "$DEST/java/com/artcraft/coimanide"
python3 - "$ROOT/templates/android/SafActivity.java" "$DEST/java/com/artcraft/coimanide/CoimanIdActivity.java" <<'PY'
from pathlib import Path
import sys

src = Path(sys.argv[1]).read_text()
text = (
    src.replace("__PACKAGE__", "com.artcraft.coimanide")
    .replace("__CLASS__", "CoimanIdActivity")
    .replace("__TAG__", "CoimanIdActivity")
)

field_anchor = '    private static final String TAG = "CoimanIdActivity";'

# The system's NativeActivity loads the native library for its own class loader; the app's
# classes (ArtCraftIme, whose JNI wake the keyboard listener calls) need it in their own. Loading
# it here as well is harmless when it is already loaded, and makes the JNI symbol resolve.
lib_loader = field_anchor + """

    static {
        try {
            System.loadLibrary("coimanide_android");
        } catch (Throwable ignored) {
        }
    }
"""
if 'System.loadLibrary("coimanide_android")' not in text:
    text = text.replace(field_anchor, lib_loader, 1)
    print("CoimanIdActivity.java: the app class loader can resolve the JNI wake")

if "private TermuxBroker termuxBroker;" not in text:
    text = text.replace(
        field_anchor,
        field_anchor + "\n    /** Forwards Coiman's tool/terminal requests to Termux. */\n"
        "    private TermuxBroker termuxBroker;",
        1,
    )

# The broker starts with the Activity; the anchors are single lines, and a missing one is an
# error: a silently skipped injection leaves the app without its Termux bridge.
start_anchor = "        super.onCreate(savedInstanceState);\n"
if "termuxBroker = new TermuxBroker(this);" not in text:
    if start_anchor not in text:
        raise SystemExit("the SAF Activity template no longer has the onCreate to start the broker in")
    text = text.replace(
        start_anchor,
        start_anchor
        + "        termuxBroker = new TermuxBroker(this);\n"
        + "        termuxBroker.start();\n",
        1,
    )

if "watchKeyboardHeight();" not in text:
    if "        installInputView();\n" not in text:
        raise SystemExit("the SAF Activity template no longer installs the input view")
    text = text.replace(
        "        installInputView();\n",
        "        installInputView();\n        watchKeyboardHeight();\n        extractThemes();\n",
        1,
    )

destroy_anchor = "    protected void onDestroy() {\n        try {"
if "termuxBroker.stop();" not in text:
    if destroy_anchor not in text:
        raise SystemExit("the SAF Activity template no longer has the onDestroy to stop the broker in")
    text = text.replace(
        destroy_anchor,
        "    protected void onDestroy() {\n"
        "        if (termuxBroker != null) {\n"
        "            termuxBroker.stop();\n"
        "        }\n"
        "        try {",
        1,
    )

if "public String termuxStatus()" not in text:
    text = text.rstrip()
    if not text.endswith("}"):
        raise SystemExit("the SAF Activity template no longer ends with a class brace")
    text = text[:-1] + """
    /** One-line Termux bridge state (the shared status file carries it to the shim too). */
    public String termuxStatus() {
        return termuxBroker == null ? "" : termuxBroker.status();
    }

    /**
     * Unpacks the bundled themes into the app's files folder. The shell copies them into Coiman's
     * config folder, where its theme picker lists them.
     */
    private void extractThemes() {
        try {
            String[] names = getAssets().list("coiman-themes");
            if (names == null || names.length == 0) {
                Log.e(TAG, "extractThemes: assets/coiman-themes is missing or empty");
                return;
            }
            File target = new File(getFilesDir(), "coiman-themes");
            target.mkdirs();
            int copied = 0;
            for (String name : names) {
                if (!name.endsWith(".json")) continue;
                File file = new File(target, name);
                if (file.exists() && file.length() > 0) continue;
                try (InputStream in = getAssets().open("coiman-themes/" + name);
                     OutputStream out = new FileOutputStream(file)) {
                    byte[] buf = new byte[64 * 1024];
                    int read;
                    while ((read = in.read(buf)) != -1) out.write(buf, 0, read);
                    copied++;
                }
            }
            Log.i(TAG, "extractThemes: " + copied + " theme(s) added, " + names.length + " asset(s) found");
        } catch (Throwable t) {
            Log.e(TAG, "extractThemes failed", t);
        }
    }

    /**
     * How much of the window the soft keyboard covers, in pixels (0 when it is hidden).
     *
     * The value is read on the UI thread by {@link #watchKeyboardHeight} and kept here: the
     * shell asks for it from the render thread every frame, and window insets may only be read
     * from the UI thread.
     */
    public int imeInsetBottom() {
        return imeInset;
    }

    /** The keyboard inset the layout listener saw last, in pixels. */
    private volatile int imeInset = 0;

    /** The height reported last; -1 when it has never been reported. */
    private int lastImeHeight = -1;

    /** Reads the keyboard's inset; UI thread only. */
    private int computeImeInset() {
        try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                android.view.WindowInsets insets =
                        getWindow() == null ? null : getWindow().getDecorView().getRootWindowInsets();
                if (insets != null) {
                    return insets.getInsets(android.view.WindowInsets.Type.ime()).bottom;
                }
            }
        } catch (Throwable ignored) {
        }
        return 0;
    }

    /**
     * Reports the keyboard's height whenever the window's layout changes (the keyboard opens or
     * closes). The shell needs a repaint to draw the new layout, and nothing else would ask for
     * one at that moment: {@link #wakeNativeLoop()} sends it an input event.
     */
    private void watchKeyboardHeight() {
        android.view.View decor = getWindow() == null ? null : getWindow().getDecorView();
        if (decor == null) return;
        decor.getViewTreeObserver().addOnGlobalLayoutListener(() -> {
            int height = computeImeInset();
            imeInset = height;
            if (height == lastImeHeight) return;
            lastImeHeight = height;
            Log.i(TAG, "keyboard height: " + height + " px");
            wakeNativeLoop();
        });
    }
}
"""
Path(sys.argv[2]).write_text(text)
print("CoimanIdActivity.java: SAF Activity + TermuxBroker")
PY

python3 "$ROOT/lib/app_icons.py" --src "$SRC_ABS" --app "$APP" --dest "$DEST/res" || true

# The file-association filters for code and text documents.
python3 - "$DEST/Cargo.toml" "$ROOT" "$APP" <<'PY'
from pathlib import Path
import sys, tomllib
path, root, app = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]
sys.path.insert(0, str(root / "lib"))
import file_associations
if not file_associations.patch_cargo_toml(path, app):
    print("warning: no intent_filter line to patch in the launcher manifest")
tomllib.loads(path.read_text())
PY

# --------------------------------------------------------------------------------------------------
# The Termux shim, compiled twice: tools (quiet_command) and the interactive terminal.
# --------------------------------------------------------------------------------------------------
find_ndk() {
    local candidate
    for candidate in "${ANDROID_NDK_HOME:-}" "${ANDROID_NDK_ROOT:-}" "${NDK_HOME:-}"; do
        [[ -n "$candidate" && -d "$candidate" ]] && { echo "$candidate"; return; }
    done
    candidate="$(find "$ROOT/android-sdk/ndk" -maxdepth 1 -mindepth 1 -type d 2>/dev/null | sort -V | tail -n 1)"
    [[ -n "$candidate" ]] && { echo "$candidate"; return; }
    return 1
}
NDK_PATH="$(find_ndk || true)"
[[ -n "$NDK_PATH" ]] || { echo "ERROR: could not find the Android NDK to build the Termux shim" >&2; exit 1; }
TOOLCHAIN="$NDK_PATH/toolchains/llvm/prebuilt/linux-x86_64"
CC="$TOOLCHAIN/bin/aarch64-linux-android28-clang"
if [[ ! -x "$CC" ]]; then
    CC="$(find "$TOOLCHAIN/bin" -maxdepth 1 -name 'aarch64-linux-android*-clang' -type f | sort -V | tail -n 1)"
fi
[[ -n "$CC" && -x "$CC" ]] || { echo "ERROR: no arm64 clang in $TOOLCHAIN/bin" >&2; exit 1; }

SHIM_DIR="$DEST/extra-jniLibs/arm64-v8a"
mkdir -p "$SHIM_DIR"
echo "Compiling the Termux shim with $CC"
"$CC" -O2 -fPIE -pie -pthread -o "$SHIM_DIR/libcoiman_termux_shim.so" "$ADAPTER/shim/termux_shim.c"
cp -f "$SHIM_DIR/libcoiman_termux_shim.so" "$SHIM_DIR/libcoiman_termux_shell.so"
chmod 0755 "$SHIM_DIR/libcoiman_termux_shim.so" "$SHIM_DIR/libcoiman_termux_shell.so"

# The emulator ABI is not a supported build target here (Termux is an arm64 device application),
# but Gradle packages a multi-ABI jniLibs tree as-is. An x86_64 emulator build is opt-in.
if [[ "${ARTCRAFT_TERMUX_X86_64:-}" == "1" ]]; then
    mkdir -p "$DEST/extra-jniLibs/x86_64"
    CC64="$TOOLCHAIN/bin/x86_64-linux-android28-clang"
    [[ -x "$CC64" ]] || CC64="$(find "$TOOLCHAIN/bin" -maxdepth 1 -name 'x86_64-linux-android*-clang' -type f | sort -V | tail -n 1)"
    "$CC64" -O2 -fPIE -pie -pthread -o "$DEST/extra-jniLibs/x86_64/libcoiman_termux_shim.so" "$ADAPTER/shim/termux_shim.c"
    cp -f "$DEST/extra-jniLibs/x86_64/libcoiman_termux_shim.so" "$DEST/extra-jniLibs/x86_64/libcoiman_termux_shell.so"
fi

printf 'Generated by artcraft-android-builder; custom adapter sourced from adapters/coimanide.\n' \
    > "$DEST/.artcraft-android-builder-generated"

echo "Installed $TITLE adapter in $DEST"

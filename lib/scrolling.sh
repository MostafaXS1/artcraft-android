#!/usr/bin/env bash
# Two-finger scrolling — the app-side patches.
#
# A phone or tablet has no wheel, so a two-finger drag is the only way to scroll. The
# builder's vendored winit backend (see lib/build-common.sh) recognises the gesture the way
# a browser does — a two-finger slide scrolls, a two-finger pinch zooms — and takes it over
# from the touch path: the fingers' touches are withheld, and the first finger's touch is
# cancelled so the gesture cannot start a text selection.
#
# Two things are left for the app:
#
#  * the press that arrives with the *first* finger must not start a selection: the canvas
#    has to know a gesture is in progress, which it learns from the touch the backend
#    cancelled;
#  * the momentum. The web build leaves the flick's coast to the browser (Chromium
#    synthesises it and sends it as more wheel events), and so do macOS and Windows; Android
#    sends no momentum of its own for a two-finger pan, so the app has to make the coast
#    itself, exactly as it does on Linux.
#
# Every edit is guarded and idempotent: it applies only while the original text is still
# present, so it no-ops once upstream handles it, and is re-applied on a fresh checkout.
#
# The scroll synthesis itself (the vendored winit/egui-winit backends) lives in
# lib/build-common.sh and runs regardless of this script; this script adds the app side.
#
# To test WITHOUT the patches, comment out the call to this script in lib/build-common.sh.
#
# Usage: scrolling.sh <source-dir> <app> <title>
set -euo pipefail

SRC="${1:?source dir}"
APP="${2:?app}"
TITLE="${3:?title}"

python3 - "$SRC" "$APP" "$TITLE" <<'PY'
from pathlib import Path
import re
import sys

src, app, title = Path(sys.argv[1]), sys.argv[2], sys.argv[3]


def fix(rel: str, old: str, new: str) -> bool:
    """Replace the single `old` in `rel` with `new`, unless it is already gone or already fixed."""
    path = src / rel
    try:
        text = path.read_text()
    except OSError:
        return False
    if old not in text or new in text or text.count(old) != 1:
        return False
    path.write_text(text.replace(old, new, 1))
    return True


def sub(rel: str, pattern: str, new: str) -> bool:
    """Replace the first match of `pattern` in `rel` with `new`, unless it is already fixed."""
    path = src / rel
    try:
        text = path.read_text()
    except OSError:
        return False
    if new in text:
        return False
    updated, count = re.subn(pattern, new, text, count=1, flags=re.S)
    if count != 1:
        return False
    path.write_text(updated)
    return True


fixed = []
if app == "wordcraft":
    # A two-finger gesture scrolls the document (see the builder's patched winit backend).
    # The backend takes the gesture over from the touch path and cancels the first finger's
    # touch when the second finger lands; that cancellation is how the canvas learns a
    # gesture is in progress, so a press that belongs to the gesture does not start a text
    # selection, and a selection the first finger already began is dropped.
    #
    # egui's own multi-touch state cannot be used for this: the backend keeps egui's touch
    # state clear (it reports the gesture itself, so egui must not read the same two
    # fingers as a pinch of its own and zoom the document by accident).
    guard = (
        "/// A two-finger gesture (scroll or pinch) is in progress: a touch press during it is part\n"
        "/// of that gesture, not a click, and any text-selection drag it began must be dropped.\n"
        "///\n"
        "/// The backend takes the gesture over from the touch path and says so by cancelling the\n"
        "/// first finger's touch (see the builder's patched winit backend), so the gesture is\n"
        "/// recognised by that cancellation and not by egui's own multi-touch state (which the\n"
        "/// backend keeps clear, so that egui does not read the same fingers as a pinch).\n"
        "fn touch_gesture_active(ui: &Ui) -> bool {\n"
        "    ui.input(|i| {\n"
        "        i.events\n"
        "            .iter()\n"
        "            .any(|e| matches!(e, egui::Event::Touch { phase: egui::TouchPhase::Cancel, .. }))\n"
        "    })\n"
        "}\n"
    )
    mouse_line = (
        "fn mouse(app: &mut WordApp, ui: &Ui, resp: &egui::Response, rects: &[Rect], "
        "layout: &DocLayout, scale: f32) {"
    )
    # A source that has never been patched: add the guard. A source that already has one
    # (an earlier builder's, or an upstream that grew its own) gets it updated instead.
    if "fn touch_gesture_active" not in (src / "crates/ui-egui/src/canvas.rs").read_text():
        if fix(
            "crates/ui-egui/src/canvas.rs",
            mouse_line,
            guard + "\n" + mouse_line,
        ):
            fixed.append("crates/ui-egui/src/canvas.rs (touch gestures start no selection)")
    elif sub(
        "crates/ui-egui/src/canvas.rs",
        r"/// A two-finger gesture \(scroll or pinch\) is in progress.*?\nfn touch_gesture_active\(ui: &Ui\) -> bool \{\n.*?\n\}\n",
        guard,
    ):
        fixed.append("crates/ui-egui/src/canvas.rs (touch gestures start no selection)")

    # Do not begin a selection on a press that is part of a two-finger gesture.
    if fix(
        "crates/ui-egui/src/canvas.rs",
        "    let pressed = ui.input(|i| i.pointer.primary_pressed()) && resp.contains_pointer() && !over_balloon(app, p);\n",
        "    let pressed = ui.input(|i| i.pointer.primary_pressed())\n"
        "        && !touch_gesture_active(ui)\n"
        "        && resp.contains_pointer()\n"
        "        && !over_balloon(app, p);\n",
    ):
        fixed.append("crates/ui-egui/src/canvas.rs (two-finger press does not select)")

    # A second finger arriving after the press already started a drag: drop it, so the scroll
    # never carries a selection along. Runs before the drag-extension arms below.
    if fix(
        "crates/ui-egui/src/canvas.rs",
        "    } else if app.canvas.dragging\n"
        "        && ui.input(|i| i.pointer.primary_down())\n"
        "        && let Some(d) = app.canvas.unit_drag.as_mut()\n",
        "    } else if app.canvas.dragging && touch_gesture_active(ui) {\n"
        "        // A two-finger scroll took over: stop selecting and forget the drag.\n"
        "        app.canvas.cancel_text_drag();\n"
        "    } else if app.canvas.dragging\n"
        "        && ui.input(|i| i.pointer.primary_down())\n"
        "        && let Some(d) = app.canvas.unit_drag.as_mut()\n",
    ):
        fixed.append("crates/ui-egui/src/canvas.rs (scroll cancels a live selection drag)")

    # The soft keyboard during a scroll or a drag. Android shows the keyboard whenever the
    # focused text field asks for it — and the canvas asks on every frame while it holds the
    # caret. A two-finger pan or a draw gesture therefore raised the keyboard over the very
    # document the user was scrolling. The canvas has to give the keyboard up the moment a
    # gesture starts, and stop asking for it while the gesture lasts; the shell then tells the
    # Activity to put the keyboard away (see the generated launcher template in
    # lib/build-common.sh and lib/ime.sh).
    #
    # `surrendered_focus` is the canvas's own memory of that decision for the frame: it is
    # cleared at the top of `show()` and set when a gesture is under way, so the IME output —
    # and the blinking caret — stand down for that frame without tearing focus away from the
    # document (the gesture ends with the caret still in the text, ready for the next keystroke).

    # The flags themselves, beside the canvas's other per-frame state.
    if fix(
        "crates/ui-egui/src/canvas.rs",
        "    pub want_focus: bool,\n",
        "    pub want_focus: bool,\n"
        "    /// The canvas was explicitly focused by a user tap — used to suppress auto-focus on load.\n"
        "    pub user_focused: bool,\n"
        "    /// Set when focus is surrendered this frame to suppress IME output immediately.\n"
        "    pub surrendered_focus: bool,\n",
    ):
        fixed.append("crates/ui-egui/src/canvas.rs (the frame's keyboard surrender)")

    # No keyboard on load: the canvas is built wanting focus so a tap lands the caret, but on
    # Android that focus is what raises the keyboard, and the first frame would show it over a
    # document the user has not touched. It starts surrendered, and remembers whether the user
    # ever tapped it.
    if fix(
        "crates/ui-egui/src/canvas.rs",
        "            want_focus: true,\n",
        "            want_focus: false,\n"
        "            user_focused: false,\n"
        "            surrendered_focus: false,\n",
    ):
        fixed.append("crates/ui-egui/src/canvas.rs (no keyboard before the first tap)")

    # Cleared each frame: a surrender lasts exactly the frame it is made in.
    if fix(
        "crates/ui-egui/src/canvas.rs",
        "    let full = ui.available_rect_before_wrap();\n",
        "    let full = ui.available_rect_before_wrap();\n"
        "    app.canvas.surrendered_focus = false; // reset each frame\n",
    ):
        fixed.append("crates/ui-egui/src/canvas.rs (a surrender lasts one frame)")

    # The gesture itself. A scroll (the middle-button pan or the cancelled two-finger touch),
    # a draw, or any other non-text drag is not text editing: the canvas gives the keyboard up
    # and asks the shell to hide it, and the frame's IME output stands down.
    if fix(
        "crates/ui-egui/src/canvas.rs",
        '        let resp = ui.interact(content, egui::Id::new("wordcraft_canvas"), Sense::click_and_drag());\n',
        '        let resp = ui.interact(content, egui::Id::new("wordcraft_canvas"), Sense::click_and_drag());\n'
        "        // On Android: if there's an active touch scroll/drag (not text selection),\n"
        "        // surrender focus immediately so the soft keyboard doesn't show during scroll.\n"
        '        if cfg!(target_os = "android") {\n'
        "            let middle_gesture = app.canvas.auto_scroll.blocks_input();\n"
        "            let is_drawing = app.canvas.ink.is_some()\n"
        "                || app.session.view.draw.mode != wordcraft_engine::cmd::draw::DrawMode::Select\n"
        "                || app.canvas.table_tool.is_some();\n"
        "            let is_text_drag = resp.drag_started()\n"
        "                && !app.canvas.auto_scroll.blocks_input()\n"
        "                && !is_drawing\n"
        "                && !ui.input(|i| i.pointer.button_down(egui::PointerButton::Middle));\n"
        "            let is_scrolling = app.canvas.auto_scroll.blocks_input();\n"
        "            let is_drawing_action = is_drawing || app.canvas.obj_drag.is_some() || app.canvas.crop_drag.is_some();\n"
        "            let has_gesture = (is_scrolling || is_drawing_action || crate::canvas::touch_gesture_active(ui))\n"
        "                && !is_text_drag;\n"
        "            if has_gesture {\n"
        "                resp.surrender_focus();\n"
        "                app.canvas.surrendered_focus = true; // suppress IME output this frame\n"
        "                app.request_ime_hide = true; // tell shell to hide IME\n"
        "            }\n"
        "        }\n",
    ):
        fixed.append("crates/ui-egui/src/canvas.rs (a scroll hides the keyboard)")

    # The IME output for a caret inside a slot (a table cell or a text box): only ask for the
    # keyboard when there is a real text caret, and never while the frame surrendered focus.
    if sub(
        "crates/ui-egui/src/canvas.rs",
        r"                if focused \{\n"
        r"                    ui\.ctx\(\)\.request_repaint_after\(std::time::Duration::from_millis\(530 - \(since as u64 % 530\)\)\);\n"
        r"                    let cr = Rect::from_min_max\(pos2\(x, y0\), pos2\(x \+ 1\.0, y1\)\);\n"
        r"                    ui\.ctx\(\)\.output_mut\(\|o\| \{\n"
        r"                        o\.ime = Some\(egui::output::IMEOutput \{\n"
        r"                            rect: cr,\n"
        r"                            cursor_rect: cr,\n"
        r"                            purpose: Default::default\(\),\n"
        r"                            should_interrupt_composition: ime_reset,\n"
        r"                        \}\);\n"
        r"                    \}\);\n"
        r"                \}\n",
        "                // A real text caret (a collapsed selection), not just canvas focus, and\n"
        "                // never while this frame surrendered the keyboard (a scroll or a draw).\n"
        "                let has_text_caret = app.session.sel.is_collapsed() && focused;\n"
        "                if has_text_caret && !app.canvas.surrendered_focus {\n"
        "                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(530 - (since as u64 % 530)));\n"
        "                    let cr = Rect::from_min_max(pos2(x, y0), pos2(x + 1.0, y1));\n"
        "                    ui.ctx().output_mut(|o| {\n"
        "                        o.ime = Some(egui::output::IMEOutput {\n"
        "                            rect: cr,\n"
        "                            cursor_rect: cr,\n"
        "                            purpose: Default::default(),\n"
        "                            should_interrupt_composition: ime_reset,\n"
        "                        });\n"
        "                    });\n"
        "                }\n",
    ):
        fixed.append("crates/ui-egui/src/canvas.rs (the slot caret asks for no keyboard mid-gesture)")

    # The body caret: the same stand-down, and the caret itself stops blinking while the
    # keyboard is away (the two `if focused` blocks below it are the other IME request).
    if fix(
        "crates/ui-egui/src/canvas.rs",
        "            let shown = app.session.sel.is_collapsed() && focused;\n",
        "            let shown = app.session.sel.is_collapsed() && focused && !app.canvas.surrendered_focus;\n",
    ):
        fixed.append("crates/ui-egui/src/canvas.rs (the caret stands down mid-gesture)")

    # The body caret's own IME request and its placement block, both gated on `shown`.
    if sub(
        "crates/ui-egui/src/canvas.rs",
        r"            if focused \{\n"
        r"                ui\.ctx\(\)\.request_repaint_after\(std::time::Duration::from_millis\(530 - \(since as u64 % 530\)\)\);\n"
        r"            \}\n"
        r"            // IME candidate window placement\.\n"
        r"            let cr = Rect::from_min_max\(pos2\(x, y0\), pos2\(x \+ 1\.0, y1\)\);\n"
        r"            if focused \{\n",
        "            if shown {\n"
        "                ui.ctx().request_repaint_after(std::time::Duration::from_millis(530 - (since as u64 % 530)));\n"
        "            }\n"
        "            // IME candidate window placement.\n"
        "            let cr = Rect::from_min_max(pos2(x, y0), pos2(x + 1.0, y1));\n"
        "            if shown {\n",
    ):
        fixed.append("crates/ui-egui/src/canvas.rs (the caret's IME request stands down mid-gesture)")

    # What takes focus. A primary click, an accessibility click, or a text-selection drag still
    # takes it; a drag that belongs to a scroll, a pan or a drawing tool must not — taking focus
    # there is what showed the keyboard mid-gesture. On Android only an explicit tap takes focus
    # at all: the programmatic `want_focus` (a dialog closing, say) must not raise the keyboard.
    if fix(
        "crates/ui-egui/src/canvas.rs",
        "    // click, an accessibility click, or a drag still takes focus, including a click on\n"
        "    // a canvas that already has it, so a composition commits. `want_focus` focuses only\n"
        "    // while the canvas does not, and does not interrupt an editor that already has it.\n"
        "    let keyboard_click = ui.memory(|m| m.has_focus(resp.id)) && ui.input(|i| i.key_pressed(egui::Key::Space) || i.key_pressed(egui::Key::Enter));\n"
        "    let pointer_click = resp.clicked_by(egui::PointerButton::Primary);\n"
        "    let accesskit_click = resp.clicked() && !pointer_click && !keyboard_click;\n"
        "    let focus_click = pointer_click || accesskit_click || resp.drag_started();\n"
        "    if focus_click || (app.canvas.want_focus && !resp.has_focus()) {\n"
        "        resp.request_focus();\n"
        "    }\n"
        "    if focus_click || app.canvas.want_focus {\n"
        "        app.canvas.want_focus = false;\n"
        "    }\n",
        "    // click, an accessibility click, or a text-selection drag takes focus. But drags for\n"
        "    // scrolling (middle-button), drawing (table pen), or panning must NOT take focus —\n"
        "    // they would show the soft keyboard and interrupt composition.\n"
        "    let keyboard_click = ui.memory(|m| m.has_focus(resp.id)) && ui.input(|i| i.key_pressed(egui::Key::Space) || i.key_pressed(egui::Key::Enter));\n"
        "    let pointer_click = resp.clicked_by(egui::PointerButton::Primary);\n"
        "    let accesskit_click = resp.clicked() && !pointer_click && !keyboard_click;\n"
        "    // Only a primary click, accessibility click, or TEXT-SELECTION drag should focus.\n"
        "    // Exclude middle-button scroll, table pen drawing, ink/pen/eraser drawing,\n"
        "    // and other non-text drags.\n"
        "    let middle_gesture = app.canvas.auto_scroll.blocks_input();\n"
        "    let is_drawing = app.canvas.ink.is_some()\n"
        "        || app.session.view.draw.mode != wordcraft_engine::cmd::draw::DrawMode::Select\n"
        "        || app.canvas.table_tool.is_some();\n"
        "    let is_text_drag = resp.drag_started()\n"
        "        && !middle_gesture\n"
        "        && !is_drawing\n"
        "        && !ui.input(|i| i.pointer.button_down(egui::PointerButton::Middle));\n"
        "    // When drawing tools are active (pen, eraser, lasso), a click should NOT take focus\n"
        "    // — it's a drawing action, not text editing. So exclude pointer_click in that case.\n"
        "    let focus_click = (pointer_click && !is_drawing) || accesskit_click || is_text_drag;\n"
        "\n"
        "    // On Android, the canvas should ONLY get keyboard focus from an explicit user\n"
        "    // tap (focus_click). Programmatic want_focus (e.g. closing a dialog) must not\n"
        "    // show the soft keyboard — it would appear during scroll/draw/transitions.\n"
        "    let want_focus_honored = if cfg!(target_os = \"android\") {\n"
        "        false // Only focus_click takes focus on Android\n"
        "    } else {\n"
        "        app.canvas.want_focus\n"
        "    };\n"
        "    if focus_click || (want_focus_honored && !resp.has_focus()) {\n"
        "        resp.request_focus();\n"
        "        app.canvas.user_focused = true;\n"
        "    }\n"
        "    if focus_click || app.canvas.want_focus {\n"
        "        app.canvas.want_focus = false;\n"
        "    }\n"
        "    // If the canvas somehow got focus without a user action (e.g. initial frame),\n"
        "    // surrender it so the soft keyboard doesn't appear prematurely.\n"
        "    let initial_surrender = !app.canvas.user_focused && resp.has_focus();\n"
        "    if initial_surrender {\n"
        "        resp.surrender_focus();\n"
        "    }\n",
    ):
        fixed.append("crates/ui-egui/src/canvas.rs (a scroll or a draw takes no focus)")

    # The canvas's own struct: the flag the shell reads to put the keyboard away.
    if fix(
        "crates/ui-egui/src/lib.rs",
        "    /// Comment and reply text typed but not yet written to the document (#1348).\n"
        "    pub(crate) comment_drafts: panes::CommentDrafts,\n",
        "    /// Comment and reply text typed but not yet written to the document (#1348).\n"
        "    pub(crate) comment_drafts: panes::CommentDrafts,\n"
        "    /// Set by the canvas on Android when a scroll or a draw surrenders the keyboard, so\n"
        "    /// the shell can put it away at once (see the shell's `logic`).\n"
        "    pub request_ime_hide: bool,\n",
    ):
        fixed.append("crates/ui-egui/src/lib.rs (the keyboard-hide request)")

    if fix(
        "crates/ui-egui/src/lib.rs",
        "            comment_drafts: Default::default(),\n",
        "            comment_drafts: Default::default(),\n"
        "            request_ime_hide: false,\n",
    ):
        fixed.append("crates/ui-egui/src/lib.rs (the keyboard-hide request, at rest)")

    # The flick's coast. The upstream constant leaves the momentum to the platform on the
    # platforms that send their own: macOS and Windows (they report the lift, and a touchpad
    # there as line deltas) and the web (Chromium synthesises the coast and sends it as more
    # wheel events, which is why the web build must not make one as well).
    #
    # Android sends no momentum of its own for a two-finger pan — the touchscreen's OS layer
    # has no inertia — so the app has to make the coast, exactly as it does on Linux. The
    # earlier builder patch excluded Android, which left a flick with no coast at all.
    #
    # A mouse's notches are line deltas and never reach the momentum, so they cannot coast:
    # only the smooth (point) deltas of a pan or a trackpad do.
    without_android = (
        "const SYNTH_MOMENTUM: bool = !cfg!(any(\n"
        '    target_os = "macos",\n'
        '    target_os = "ios",\n'
        '    target_os = "windows",\n'
        '    target_arch = "wasm32",\n'
        "));"
    )
    if fix(
        "crates/ui-egui/src/scroll.rs",
        'const SYNTH_MOMENTUM: bool = !cfg!(any(target_os = "macos", target_os = "ios", target_os = "windows", target_arch = "wasm32"));',
        without_android,
    ):
        fixed.append("crates/ui-egui/src/scroll.rs (the app makes the coast Android does not send)")
    elif sub(
        "crates/ui-egui/src/scroll.rs",
        r"const SYNTH_MOMENTUM: bool = !cfg!\(any\(.*?\n\)\);",
        without_android,
    ):
        fixed.append("crates/ui-egui/src/scroll.rs (the app makes the coast Android does not send)")

if fixed:
    print(f"  Added two-finger-scroll patches to {title}: {', '.join(fixed)}")
PY

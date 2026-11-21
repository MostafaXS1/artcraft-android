#!/usr/bin/env bash
set -Eeuo pipefail
APP="${1:-photocraft}"
TITLE="${2:-PhotoCraft}"
SRC="${3:-source/photocraft}"
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
SRC_ABS="$ROOT/$SRC"
ADAPTER="$ROOT/adapters/photocraft"
DEST="$SRC_ABS/apps/$APP-android"

[[ -f "$SRC_ABS/Cargo.toml" ]] || { echo "PhotoCraft workspace not found at $SRC_ABS" >&2; exit 1; }
if [[ -e "$DEST" && ! -f "$DEST/.artcraft-android-builder-generated" ]]; then
  echo "Refusing to replace unmarked Android directory: $DEST" >&2
  exit 1
fi
if [[ -d "$DEST" ]]; then
  rm -rf -- "$DEST"
fi
mkdir -p "$DEST"
cp -a "$ADAPTER/." "$DEST/"
rm -f "$DEST/prepare.sh" "$DEST/README.md"
# The Android launcher runs the same control server as the desktop app, so it needs the same
# file. Upstream v0.13's `control_server.rs` is the source of truth: it is copied here and
# adapted for Android (the desktop's `eprintln!` becomes the Android logger, and the test module
# is dropped because it needs the desktop's test dependencies). Copying it per build — rather than
# freezing one version in the adapter — keeps it in step with the upstream release being built:
# upstream v0.23 replaced `security::authentication_reply` with the `ServerHandshake` state
# machine, and a frozen copy no longer compiled.
python3 - "$SRC_ABS" "$DEST" <<'PY'
from pathlib import Path
import sys

src, dest = Path(sys.argv[1]), Path(sys.argv[2])
upstream = src / "apps" / "photocraft" / "src" / "control_server.rs"
target = dest / "src" / "control_server.rs"

if not upstream.is_file():
    raise SystemExit(
        f"photocraft: upstream no longer has {upstream}; the Android control server has to be "
        "adapted by hand"
    )

text = upstream.read_text()

# The desktop logs to stderr; on Android the records go to logcat through the `log` crate.
text = text.replace("eprintln!(", "log::info!(")

# The tests need the desktop crate's test-only dependencies; the Android launcher has none.
marker = "#[cfg(test)]\nmod tests {"
if marker in text:
    text = text[: text.index(marker)].rstrip() + "\n"

target.write_text(text)
print(f"photocraft: the control server follows upstream ({upstream.name})")
PY

# Apply mobile dialog and popup scaling adaptations to upstream ui-egui crates
python3 - "$SRC_ABS" <<'PY'
from pathlib import Path
import sys

src = Path(sys.argv[1])
ui_lib = src / "crates" / "ui-egui" / "src" / "lib.rs"
ui_app = src / "crates" / "ui-egui" / "src" / "app.rs"
dialogs = src / "crates" / "ui-egui" / "src" / "dialogs.rs"
new_doc = src / "crates" / "ui-egui" / "src" / "dialogs" / "new_doc_ui.rs"

if ui_lib.is_file():
    s = ui_lib.read_text()
    if "pub compact_layout: bool," not in s:
        s = s.replace("pub background_jobs: bool,", "pub background_jobs: bool,\n    pub compact_layout: bool,")
        ui_lib.write_text(s)

if ui_app.is_file():
    s = ui_app.read_text()
    if "compact_layout: false," not in s:
        s = s.replace("background_jobs: false,", "background_jobs: false,\n            compact_layout: false,")
        ui_app.write_text(s)

if dialogs.is_file():
    s = dialogs.read_text()
    old_show_modal = """fn show_modal<T>(
    modal: egui::Modal,
    ctx: &egui::Context,
    id: egui::Id,
    resizable: Option<(egui::Vec2, egui::Vec2, egui::Vec2)>,
    fixed: Option<egui::Vec2>,
    interactive: bool,
    contents: impl FnOnce(&mut egui::Ui) -> T,
) -> egui::ModalResponse<T> {"""
    new_show_modal = """#[allow(clippy::too_many_arguments)]
fn show_modal<T>(
    modal: egui::Modal,
    ctx: &egui::Context,
    id: egui::Id,
    resizable: Option<(egui::Vec2, egui::Vec2, egui::Vec2)>,
    fixed: Option<egui::Vec2>,
    interactive: bool,
    compact: Option<f32>,
    contents: impl FnOnce(&mut egui::Ui) -> T,
) -> egui::ModalResponse<T> {"""
    if old_show_modal in s:
        s = s.replace(old_show_modal, new_show_modal)
    
    old_modal_inner = """        if let Some(size) = fixed {
            egui::Resize::default().id(id.with("resize")).with_stroke(false).fixed_size(size).show(ui, |ui| {
                ui.set_clip_rect(ui.clip_rect().expand(8.0));
                contents(ui)
            })
        } else if let Some((size, min, max)) = resizable {
            // No stroke: Resize outlines its contents 2 points out, an inner box inside the
            // dialog's frame that left the title and buttons flush against it (#8199).
            egui::Resize::default().id(id.with("resize")).with_stroke(false).default_size(size.min(max)).min_size(min).max_size(max).show(ui, contents)
        } else {
            contents(ui)
        }"""
    new_modal_inner = """        if let Some(size) = fixed.filter(|_| compact.is_none()) {
            egui::Resize::default().id(id.with("resize")).with_stroke(false).fixed_size(size).show(ui, |ui| {
                ui.set_clip_rect(ui.clip_rect().expand(8.0));
                contents(ui)
            })
        } else if let Some((size, min, max)) = resizable.filter(|_| compact.is_none()) {
            // No stroke: Resize outlines its contents 2 points out, an inner box inside the
            // dialog's frame that left the title and buttons flush against it (#8199).
            egui::Resize::default().id(id.with("resize")).with_stroke(false).default_size(size.min(max)).min_size(min).max_size(max).show(ui, contents)
        } else if let Some(h) = compact {
            egui::ScrollArea::vertical().id_salt(id.with("compact-scroll")).max_height(h).min_scrolled_height(h).show(ui, contents).inner
        } else {
            contents(ui)
        }"""
    if old_modal_inner in s:
        s = s.replace(old_modal_inner, new_modal_inner)

    old_vars = """        let mut drag = egui::Vec2::ZERO;
        let mut close_clicked = false;
        let mut sizing = false;"""
    new_vars = """        let mut drag = egui::Vec2::ZERO;
        let mut close_clicked = false;
        let mut sizing = false;
        let phone = app.compact_layout;
        let on_canvas = phone && (d.kind == DialogKind::LayerStyle || crate::blur_pins::owns(&d.fields) || crate::adjust_dialog::owns(&d.fields) || d.fields.contains_key("__filter"));
        let screen_h = ctx.content_rect().height();
        let phone_height = phone.then(|| if on_canvas { (screen_h * 0.4).max(160.0) } else { (screen_h - 40.0).max(120.0) });"""
    if old_vars in s and "let phone = app.compact_layout;" not in s:
        s = s.replace(old_vars, new_vars)

    old_call = "let modal = show_modal(modal, ctx, id, resizable, fixed, interactive, |ui| {"
    new_call = "let modal = show_modal(modal, ctx, id, resizable, fixed, interactive, phone_height, |ui| {"
    if old_call in s:
        s = s.replace(old_call, new_call)

    old_widths = """            if !compact {
                ui.set_min_width(if curves { 300.0 } else { 380.0 });
            }
            // A resizable dialog's contents fill its resizable area (#7420), the others the window.
            let room = if resizable.is_some() { available.x } else { (ctx.content_rect().width() - 48.0).max(380.0) };
            let wide = crate::prefs_ui::width(&d.fields, room);
            if let Some(w) = wide {
                ui.set_min_width(w.min(460.0));
            }
            if d.kind == DialogKind::NewDocument {
                ui.set_min_width(800.0);
            }
            // The About window: room for the contributor table, the same width on every tab.
            let about_tabs = d.kind == DialogKind::About && d.fields.get("systemInfo").and_then(Value::as_bool) != Some(true);
            if about_tabs {
                ui.set_min_width(700.0);
            }"""
    new_widths = """            let fit = (ctx.content_rect().width() - 24.0).max(240.0);
            if !compact {
                ui.set_min_width((if curves { 300.0 } else { 380.0_f32 }).min(fit));
            }
            let room = if resizable.is_some() { available.x } else { (ctx.content_rect().width() - 48.0).max(240.0) };
            let wide = crate::prefs_ui::width(&d.fields, room);
            if let Some(w) = wide {
                ui.set_min_width(w.min(460.0).min(fit));
            }
            if d.kind == DialogKind::NewDocument {
                ui.set_min_width(800.0_f32.min(fit));
            }
            let about_tabs = d.kind == DialogKind::About && d.fields.get("systemInfo").and_then(Value::as_bool) != Some(true);
            if about_tabs {
                ui.set_min_width(700.0_f32.min(fit));
            }"""
    if old_widths in s:
        s = s.replace(old_widths, new_widths)

    old_color_picker = """            if color_picker {
                ui.set_width(crate::color_picker_ui::content_width(ui));
            }"""
    new_color_picker = """            if color_picker {
                ui.set_width(crate::color_picker_ui::content_width(ui));
            } else if phone && !web {
                let full = (ctx.content_rect().width() - 48.0).max(240.0);
                ui.set_min_width(full);
                ui.set_max_width(full);
            }"""
    if old_color_picker in s and "let full = (ctx.content_rect().width() - 48.0).max(240.0);" not in s:
        s = s.replace(old_color_picker, new_color_picker)

    old_body = """            if let Some(done) = dialog_body(app, ui, &d, &mut fields, id, interactive) {
                outcome = Some(done);
            }"""
    new_body = """            let body_room = phone_height.filter(|_| on_canvas).map(|h| (h - 110.0).max(80.0));
            outcome = match body_room {
                Some(h) => egui::ScrollArea::vertical()
                    .id_salt(id.with("phone-body"))
                    .max_height(h)
                    .show(ui, |ui| dialog_body(app, ui, &d, &mut fields, id, interactive))
                    .inner,
                None => dialog_body(app, ui, &d, &mut fields, id, interactive),
            };"""
    if old_body in s:
        s = s.replace(old_body, new_body)

    dialogs.write_text(s)

if new_doc.is_file():
    s = new_doc.read_text()
    if "let narrow = ui.available_width() < 790.0 || app.compact_layout;" not in s:
        s = s.replace("let cat = get_s(f, \"__category\", \"Recent\");\n    let mut create = false;\n    // Category tabs.\n    ui.horizontal(|ui| {",
                      "let cat = get_s(f, \"__category\", \"Recent\");\n    let mut create = false;\n    let narrow = ui.available_width() < 790.0 || app.compact_layout;\n    let full_width = ui.available_width();\n    // Category tabs.\n    ui.horizontal_wrapped(|ui| {")
        
        old_cols = """    let chosen = get_s(f, "__preset", "");
    ui.horizontal_top(|ui| {
        // Left: preset grid.
        ui.vertical(|ui| {
            ui.set_width(520.0);"""
        new_cols = """    let chosen = get_s(f, "__preset", "");
    let columns = if narrow { egui::Layout::top_down(egui::Align::Min) } else { egui::Layout::left_to_right(egui::Align::Min) };
    ui.with_layout(columns, |ui| {
        // Left: preset grid.
        ui.vertical(|ui| {
            ui.set_width(if narrow { full_width } else { 520.0 });"""
        s = s.replace(old_cols, new_cols)

        old_chunks = """            let card = vec2(164.0, 112.0);
            // Up to 20 recent documents: scroll past three rows rather than grow the dialog.
            egui::ScrollArea::vertical().id_salt("new-document-presets").max_height(3.0 * (card.y + 8.0)).show(ui, |ui| {
                for row in presets.chunks(3) {
                    ui.horizontal(|ui| {"""
        new_chunks = """            let card = vec2(164.0, 112.0);
            let per_row = if narrow { (((ui.available_width() + 8.0) / (card.x + 8.0)).floor() as usize).max(1) } else { 3 };
            // Up to 20 recent documents: scroll past three rows rather than grow the dialog.
            egui::ScrollArea::vertical().id_salt("new-document-presets").max_height(3.0 * (card.y + 8.0)).show(ui, |ui| {
                for row in presets.chunks(per_row) {
                    ui.horizontal(|ui| {"""
        s = s.replace(old_chunks, new_chunks)

        old_focus = """            if !f.contains_key("__focused") {
                f.insert("__focused".into(), json!(true));
                crate::field_tab::focus(ui.ctx(), r.id);
            }"""
        new_focus = """            if !f.contains_key("__focused") {
                f.insert("__focused".into(), json!(true));
                if !app.compact_layout {
                    crate::field_tab::focus(ui.ctx(), r.id);
                }
            }"""
        s = s.replace(old_focus, new_focus)
        new_doc.write_text(s)
print("photocraft: dialog scaling patches applied")
PY

# Ship the shared Android system-font module next to the adapter sources; its launcher references
# `mod system_fonts;` (see templates/android/system_fonts.rs, shared with the generated launchers).
cp -f "$ROOT/templates/android/system_fonts.rs" "$DEST/src/system_fonts.rs"
cp -f "$ROOT/templates/android/mobile_text_bar.rs" "$DEST/src/mobile_text_bar.rs"
cp -f "$ROOT/templates/android/touch_gestures.rs" "$DEST/src/touch_gestures.rs"
# The soft keyboard uses android-activity's native IME (the launcher polls `text_input_state`);
# there is no ime_bridge module to copy.
# Launcher icons: density mipmaps plus the API 26+ adaptive layers, from the same shared module
# the generated launchers use (see lib/app_icons.py).
if ! python3 "$ROOT/lib/app_icons.py" --src "$SRC_ABS" --app "$APP" --dest "$DEST/res"; then
  echo "No PhotoCraft icon could be generated; using a generic Android icon." >&2
  sed -i '/^icon = /d' "$DEST/Cargo.toml"
fi
printf 'Generated by artcraft-android-builder; custom SAF adapter sourced from adapters/photocraft.\n' > "$DEST/.artcraft-android-builder-generated"
# Advertise the app's file associations (Android open/share intent filters) in the copied manifest
# from the shared table, exactly like the generated launchers do.
python3 - "$DEST/Cargo.toml" "$ROOT" "$APP" <<'PY'
from pathlib import Path
import sys, tomllib
path, root, app = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]
sys.path.insert(0, str(root / "lib"))
import file_associations
file_associations.patch_cargo_toml(path, app)
tomllib.loads(path.read_text())
PY
echo "Installed PhotoCraft SAF adapter in $DEST (upstream PhotoCraft crates were not edited)."

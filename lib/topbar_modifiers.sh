#!/usr/bin/env bash
# Integrated toggleable Shift / Ctrl / Alt buttons for topbar / ribbon.
#
# Adds real UI toggle buttons (⇧ Shift, ⌃ Ctrl, ⌥ Alt) into the topbar
# next to menu/tab buttons for 6 apps:
#   photo (photocraft), vector (vectorcraft), design (designcraft),
#   word (wordcraft), grid (gridcraft), deck (deckcraft).
#
# Usage: topbar_modifiers.sh <source-dir> <app> <title>
set -euo pipefail

SRC="${1:?source dir}"
APP="${2:?app}"
TITLE="${3:?title}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

case "$APP" in
    photocraft|vectorcraft|designcraft|wordcraft|gridcraft|deckcraft)
        ;;
    *)
        exit 0
        ;;
esac

python3 - "$ROOT" "$SRC" "$APP" "$TITLE" <<'PY'
from pathlib import Path
import re
import shutil
import sys

root, src, app, title = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3], sys.argv[4]

ui_egui_src = src / "crates" / "ui-egui" / "src"
if not ui_egui_src.is_dir():
    sys.exit(0)

# 1. Copy topbar_modifiers.rs into crates/ui-egui/src/topbar_modifiers.rs
shutil.copy2(root / "templates" / "android" / "topbar_modifiers.rs", ui_egui_src / "topbar_modifiers.rs")

# 2. Register pub mod topbar_modifiers; in crates/ui-egui/src/lib.rs
lib_rs = ui_egui_src / "lib.rs"
if lib_rs.is_file():
    text = lib_rs.read_text(encoding="utf-8")
    if "pub mod topbar_modifiers;" not in text:
        text = text + "\npub mod topbar_modifiers;\n"
        lib_rs.write_text(text, encoding="utf-8")
        print(f"  Registered topbar_modifiers module in {app} ui-egui lib.rs")

# 3. Patch the UI bar / ribbon in the app
if app == "photocraft":
    # Clean up any legacy injection in menus/bar.rs
    bar_rs = ui_egui_src / "menus" / "bar.rs"
    if bar_rs.is_file():
        text = bar_rs.read_text(encoding="utf-8")
        for bad in (
            "crate::topbar_modifiers::render_topbar_modifiers(ui);\n",
            "crate::topbar_modifiers::render_topbar_modifiers_right(ui);\n",
            "crate::topbar_modifiers::render_topbar_modifiers(ui);",
            "crate::topbar_modifiers::render_topbar_modifiers_right(ui);",
        ):
            if bad in text:
                text = text.replace(bad, "")
                bar_rs.write_text(text, encoding="utf-8")
    # Dock to the right side inside group_ui in panels/title_bar.rs
    title_bar_rs = ui_egui_src / "panels" / "title_bar.rs"
    if title_bar_rs.is_file():
        text = title_bar_rs.read_text(encoding="utf-8")
        if "topbar_modifiers" not in text:
            target = "let mut group_ui = ui.new_child(egui::UiBuilder::new().max_rect(group).layout(crate::direction::trailing()));\n                let controls_end = {\n                    let ui = &mut group_ui;"
            replacement = "let mut group_ui = ui.new_child(egui::UiBuilder::new().max_rect(group).layout(crate::direction::trailing()));\n                let controls_end = {\n                    let ui = &mut group_ui;\n                    crate::topbar_modifiers::render_topbar_modifiers(ui);"
            if target in text:
                text = text.replace(target, replacement, 1)
                title_bar_rs.write_text(text, encoding="utf-8")
                print(f"  Integrated topbar modifiers into {title} right-side title bar")

elif app == "vectorcraft":
    # Clean up any legacy injection in menus/bar.rs
    bar_rs = ui_egui_src / "menus" / "bar.rs"
    if bar_rs.is_file():
        text = bar_rs.read_text(encoding="utf-8")
        for bad in (
            "crate::topbar_modifiers::render_topbar_modifiers(ui);\n",
            "crate::topbar_modifiers::render_topbar_modifiers_right(ui);\n",
            "crate::topbar_modifiers::render_topbar_modifiers(ui);",
            "crate::topbar_modifiers::render_topbar_modifiers_right(ui);",
        ):
            if bad in text:
                text = text.replace(bad, "")
                bar_rs.write_text(text, encoding="utf-8")
    # Dock to the right side inside rui in chrome.rs
    chrome_rs = ui_egui_src / "chrome.rs"
    if chrome_rs.is_file():
        text = chrome_rs.read_text(encoding="utf-8")
        if "topbar_modifiers" not in text:
            target = "let mut rui = ui.new_child(egui::UiBuilder::new().max_rect(right).layout(egui::Layout::right_to_left(egui::Align::Center)));\n            let ui = &mut rui;"
            replacement = "let mut rui = ui.new_child(egui::UiBuilder::new().max_rect(right).layout(egui::Layout::right_to_left(egui::Align::Center)));\n            let ui = &mut rui;\n            crate::topbar_modifiers::render_topbar_modifiers(ui);"
            if target in text:
                text = text.replace(target, replacement, 1)
                chrome_rs.write_text(text, encoding="utf-8")
                print(f"  Integrated topbar modifiers into {title} right-side chrome")

elif app == "designcraft":
    # Clean up any legacy injection in menus/bar.rs
    bar_rs = ui_egui_src / "menus" / "bar.rs"
    if bar_rs.is_file():
        text = bar_rs.read_text(encoding="utf-8")
        for bad in (
            "crate::topbar_modifiers::render_topbar_modifiers(ui);\n",
            "crate::topbar_modifiers::render_topbar_modifiers_right(ui);\n",
            "crate::topbar_modifiers::render_topbar_modifiers(ui);",
            "crate::topbar_modifiers::render_topbar_modifiers_right(ui);",
        ):
            if bad in text:
                text = text.replace(bad, "")
                bar_rs.write_text(text, encoding="utf-8")
    # Dock to right side inside tools in chrome/app_bar.rs
    app_bar_rs = ui_egui_src / "chrome" / "app_bar.rs"
    if app_bar_rs.is_file():
        text = app_bar_rs.read_text(encoding="utf-8")
        if "topbar_modifiers" not in text:
            target = "let tools = ui.allocate_ui_with_layout(size, egui::Layout::right_to_left(egui::Align::Center), |ui| {\n                            ui.add_space(4.0);"
            replacement = "let tools = ui.allocate_ui_with_layout(size, egui::Layout::right_to_left(egui::Align::Center), |ui| {\n                            ui.add_space(4.0);\n                            crate::topbar_modifiers::render_topbar_modifiers(ui);"
            if target in text:
                text = text.replace(target, replacement, 1)
                app_bar_rs.write_text(text, encoding="utf-8")
                print(f"  Integrated topbar modifiers into {title} right-side app bar")

elif app in ("wordcraft", "gridcraft", "deckcraft"):
    ribbon_rs = ui_egui_src / "ribbon.rs"
    if ribbon_rs.is_file():
        text = ribbon_rs.read_text(encoding="utf-8")
        if "topbar_modifiers" not in text:
            if app == "wordcraft":
                target = "if comments_button.clicked() {\n                                let _ = app.run(\"view.commentsPane\", json!({}));\n                            }\n                        });\n                        ui.add_space(8.0);"
                replacement = "if comments_button.clicked() {\n                                let _ = app.run(\"view.commentsPane\", json!({}));\n                            }\n                        });\n                        ui.add_space(8.0);\n                        crate::topbar_modifiers::render_topbar_modifiers(ui);\n                        ui.add_space(8.0);"
                if target in text:
                    text = text.replace(target, replacement, 1)
                    ribbon_rs.write_text(text, encoding="utf-8")
                    print(f"  Integrated topbar modifiers into {title} ribbon")
            elif app == "gridcraft":
                target = "keytip(app, ui, sequence, pos2(r.center().x, r.bottom() - 1.0));\n                        }\n                    }"
                replacement = "keytip(app, ui, sequence, pos2(r.center().x, r.bottom() - 1.0));\n                        }\n                    }\n                    crate::topbar_modifiers::render_topbar_modifiers_right(ui);"
                if target in text:
                    text = text.replace(target, replacement, 1)
                    ribbon_rs.write_text(text, encoding="utf-8")
                    print(f"  Integrated topbar modifiers into {title} ribbon")
            elif app == "deckcraft":
                target = "x += w + 2.0;\n        }\n        if app.ui.ribbon_collapsed {"
                replacement = "x += w + 2.0;\n        }\n        crate::topbar_modifiers::render_topbar_modifiers_at(ui, egui::pos2(ui.max_rect().max.x - 220.0, y - 1.0));\n        if app.ui.ribbon_collapsed {"
                if target in text:
                    text = text.replace(target, replacement, 1)
                    ribbon_rs.write_text(text, encoding="utf-8")
                    print(f"  Integrated topbar modifiers into {title} ribbon")
        elif app == "deckcraft" and "crate::topbar_modifiers::render_topbar_modifiers_at" in text:
            text = re.sub(
                r"crate::topbar_modifiers::render_topbar_modifiers_at\(ui, egui::pos2\(ui\.max_rect\(\)\.max\.x - \d+\.0, y - 1\.0\)\);",
                "crate::topbar_modifiers::render_topbar_modifiers_at(ui, egui::pos2(ui.max_rect().max.x - 220.0, y - 1.0));",
                text,
            )
            ribbon_rs.write_text(text, encoding="utf-8")
PY

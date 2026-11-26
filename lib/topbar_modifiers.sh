#!/usr/bin/env bash
# Integrated toggleable Shift / Ctrl / Alt buttons for topbar / ribbon.
#
# Adds real UI toggle buttons (`⇧ Shift`, `⌃ Ctrl`, `⌥ Alt`) into the topbar
# next to menu/tab buttons for 7 apps:
#   photocraft, vectorcraft, designcraft, wordcraft, gridcraft, deckcraft, tracedraw.
# By default, layout and left menus/title remain unchanged with icon+text buttons (`⇧ Shift`,
# `⌃ Ctrl`, `⌥ Alt`). Only when the topbar runs out of horizontal space do modifier buttons
# become square icon-only buttons (`⇧`, `⌃`, `⌥`) and the left menu/tab strip becomes a
# horizontally scrollable bar.
#
# Usage: topbar_modifiers.sh <source-dir> <app> <title>
set -euo pipefail

SRC="${1:?source dir}"
APP="${2:?app}"
TITLE="${3:?title}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

case "$APP" in
    photocraft|vectorcraft|designcraft|wordcraft|gridcraft|deckcraft|tracedraw)
        ;;
    *)
        exit 0
        ;;
esac

NO_BUTTONS="${4:-${ARTCRAFT_NO_BUTTONS:-false}}"

python3 - "$ROOT" "$SRC" "$APP" "$TITLE" "$NO_BUTTONS" <<'PY'
from pathlib import Path
import shutil
import sys

root, src, app, title, no_buttons_str = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3], sys.argv[4], sys.argv[5]
no_buttons = no_buttons_str.lower() in ("true", "1", "yes")

ui_egui_src = src / "crates" / "ui-egui" / "src"
if not ui_egui_src.is_dir():
    ui_egui_src = src / "apps" / "tracedraw" / "src"
if not ui_egui_src.is_dir():
    sys.exit(0)

# 1. Copy topbar_modifiers.rs into crates/ui-egui/src/topbar_modifiers.rs
template_file = "topbar_modifiers_disabled.rs" if no_buttons else "topbar_modifiers.rs"
shutil.copy2(root / "templates" / "android" / template_file, ui_egui_src / "topbar_modifiers.rs")
font_file = root / "templates" / "android" / "modifier_symbols.ttf"
if font_file.is_file():
    shutil.copy2(font_file, ui_egui_src / "modifier_symbols.ttf")

# 2. Register pub mod topbar_modifiers; in crates/ui-egui/src/lib.rs
lib_rs = ui_egui_src / "lib.rs"
if lib_rs.is_file():
    text = lib_rs.read_text(encoding="utf-8")
    if "pub mod topbar_modifiers;" not in text:
        text = text + "\npub mod topbar_modifiers;\n"
        lib_rs.write_text(text, encoding="utf-8")
        print(f"  Registered topbar_modifiers module in {app} ui-egui lib.rs")

# 3. Patch the UI bar / ribbon in each app
if app == "photocraft":
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

    title_bar_rs = ui_egui_src / "panels" / "title_bar.rs"
    if title_bar_rs.is_file():
        text = title_bar_rs.read_text(encoding="utf-8")
        changed = False

        # Modern TitleBar::new().show_titled
        if "TitleBar::new().show_titled" in text and "render_topbar_modifiers" not in text:
            old_left = "            if app.services.native_menu.is_none() {\n                crate::menus::menu_bar(app, ui);\n            }"
            new_left = (
                "            if app.services.native_menu.is_none() {\n"
                "                let tight = crate::topbar_modifiers::is_tight(ui.ctx(), \"photocraft\");\n"
                "                let row_w = ui.ctx().content_rect().width();\n"
                "                if tight {\n"
                "                    let avail = (ui.available_width() - crate::topbar_modifiers::modifier_bar_width_compact() - 104.0).max(80.0);\n"
                "                    ui.allocate_ui_with_layout(\n"
                "                        egui::vec2(avail, ui.available_height()),\n"
                "                        egui::Layout::left_to_right(egui::Align::Center),\n"
                "                        |ui| {\n"
                "                            egui::ScrollArea::horizontal()\n"
                "                                .id_salt(\"photocraft_topbar_scroll\")\n"
                "                                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)\n"
                "                                .show(ui, |ui| {\n"
                "                                    crate::menus::menu_bar(app, ui);\n"
                "                                });\n"
                "                        },\n"
                "                    );\n"
                "                    crate::topbar_modifiers::update_tight_measured(ui.ctx(), \"photocraft\", row_w, None, 180.0);\n"
                "                } else {\n"
                "                    let edge = crate::menus::menu_bar(app, ui);\n"
                "                    let used = (edge - ui.max_rect().left()).abs();\n"
                "                    crate::topbar_modifiers::update_tight_measured(ui.ctx(), \"photocraft\", row_w, Some(used), 180.0);\n"
                "                }\n"
                "            }"
            )
            old_right = "        |_, _| {},\n    );"
            new_right = (
                "        |ui, _| {\n"
                "            let tight = crate::topbar_modifiers::is_tight(ui.ctx(), \"photocraft\");\n"
                "            crate::topbar_modifiers::render_topbar_modifiers_for(ui, tight);\n"
                "        },\n    );"
            )
            if old_right in text:
                text = text.replace(old_right, new_right, 1)
                if old_left in text:
                    text = text.replace(old_left, new_left, 1)
                changed = True

        # v0.23.1 / legacy group_ui layout
        old_v23_menus = (
            "                let menus_end = if app.services.native_menu.is_some() {\n"
            "                    if rtl { ui.cursor().right() } else { ui.cursor().left() }\n"
            "                } else {\n"
            "                    crate::menus::menu_bar(app, ui)\n"
            "                };"
        )
        new_v23_menus = (
            "                let tight = crate::topbar_modifiers::is_tight(ui.ctx(), \"photocraft\");\n"
            "                let lead_used = if rtl { (full.right() - ui.cursor().right()).max(0.0) } else { (ui.cursor().left() - full.left()).max(0.0) };\n"
            "                let cap_tail = if rtl { caption_left } else { caption_right };\n"
            "                let row_w = (full.width() - lead_used - cap_tail - TITLE_GAP).max(80.0);\n"
            "                let menus_end = if app.services.native_menu.is_some() {\n"
            "                    if rtl { ui.cursor().right() } else { ui.cursor().left() }\n"
            "                } else if tight {\n"
            "                    let mod_w = crate::topbar_modifiers::modifier_bar_width_compact();\n"
            "                    let avail = (row_w - mod_w - 110.0).max(80.0);\n"
            "                    let resp = ui.allocate_ui_with_layout(\n"
            "                        egui::vec2(avail, full.height()),\n"
            "                        if rtl { egui::Layout::right_to_left(egui::Align::Center) } else { egui::Layout::left_to_right(egui::Align::Center) },\n"
            "                        |ui| {\n"
            "                            egui::ScrollArea::horizontal()\n"
            "                                .id_salt(\"photocraft_topbar_scroll\")\n"
            "                                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)\n"
            "                                .show(ui, |ui| {\n"
            "                                    crate::menus::menu_bar(app, ui);\n"
            "                                });\n"
            "                        },\n"
            "                    );\n"
            "                    crate::topbar_modifiers::update_tight_measured(ui.ctx(), \"photocraft\", row_w, None, 180.0);\n"
            "                    if rtl { resp.response.rect.left() } else { resp.response.rect.right() }\n"
            "                } else {\n"
            "                    let edge = crate::menus::menu_bar(app, ui);\n"
            "                    let menus_w = if rtl { (full.right() - lead_used - edge).max(0.0) } else { (edge - full.left() - lead_used).max(0.0) };\n"
            "                    crate::topbar_modifiers::update_tight_measured(ui.ctx(), \"photocraft\", row_w, Some(menus_w), 180.0);\n"
            "                    edge\n"
            "                };"
        )
        if old_v23_menus in text:
            text = text.replace(old_v23_menus, new_v23_menus, 1)
            changed = True

        if "render_topbar_modifiers" not in text:
            target = "let mut group_ui = ui.new_child(egui::UiBuilder::new().max_rect(group).layout(crate::direction::trailing()));\n                let controls_end = {\n                    let ui = &mut group_ui;"
            replacement = (
                "let mut group_ui = ui.new_child(egui::UiBuilder::new().max_rect(group).layout(crate::direction::trailing()));\n"
                "                let controls_end = {\n"
                "                    let ui = &mut group_ui;\n"
                "                    crate::topbar_modifiers::render_topbar_modifiers_for(ui, crate::topbar_modifiers::is_tight(ui.ctx(), \"photocraft\"));"
            )
            if target in text:
                text = text.replace(target, replacement, 1)
                changed = True

        if changed:
            title_bar_rs.write_text(text, encoding="utf-8")
            print(f"  Integrated topbar modifiers and responsive scrollable menu bar into {title}")

elif app == "vectorcraft":
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

    chrome_rs = ui_egui_src / "chrome.rs"
    if chrome_rs.is_file():
        text = chrome_rs.read_text(encoding="utf-8")
        changed = False

        # v0.24.1 TitleBar::new().show_fitted layout (leading + trailing)
        old_leading = (
            "    // With the macOS menu bar the menus are at the top of the screen.\n"
            "    if app.services.native_menu.is_none() && MenuBarUi::new(&app.menus.laid).show(ui).opened {\n"
            "        // A menu opened: File › Scripts lists the Scripts folder as it is now.\n"
            "        crate::scripts_menu::refresh(app, ui.ctx());\n"
            "    }"
        )
        old_leading_alt = (
            "    // With the macOS menu bar the menus are at the top of the screen instead.\n"
            "    if app.services.native_menu.is_none() && MenuBarUi::new(&app.menus.laid).show(ui).opened {\n"
            "        // A menu opened: File › Scripts lists the Scripts folder as it is now.\n"
            "        crate::scripts_menu::refresh(app, ui.ctx());\n"
            "    }"
        )
        new_leading = (
            "    // With the macOS menu bar the menus are at the top of the screen instead.\n"
            "    if app.services.native_menu.is_none() {\n"
            "        let tight = crate::topbar_modifiers::is_tight(ui.ctx(), \"vectorcraft\");\n"
            "        let row_w = ui.ctx().content_rect().width();\n"
            "        let mut opened = false;\n"
            "        if tight {\n"
            "            let mod_w = crate::topbar_modifiers::modifier_bar_width_compact();\n"
            "            let avail = (row_w - mod_w - 200.0).max(80.0);\n"
            "            let mut scroll_w = 0.0f32;\n"
            "            ui.allocate_ui_with_layout(\n"
            "                egui::vec2(avail, ui.available_height().max(24.0)),\n"
            "                egui::Layout::left_to_right(egui::Align::Center),\n"
            "                |ui| {\n"
            "                    let out = egui::ScrollArea::horizontal()\n"
            "                        .id_salt(\"vectorcraft_topbar_scroll\")\n"
            "                        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)\n"
            "                        .show(ui, |ui| {\n"
            "                            opened = MenuBarUi::new(&app.menus.laid).show(ui).opened;\n"
            "                        });\n"
            "                    scroll_w = out.content_size.x;\n"
            "                },\n"
            "            );\n"
            "            crate::topbar_modifiers::update_tight_measured(ui.ctx(), \"vectorcraft\", row_w, None, 200.0);\n"
            "        } else {\n"
            "            let x0 = ui.cursor().min.x;\n"
            "            opened = MenuBarUi::new(&app.menus.laid).show(ui).opened;\n"
            "            let left_w = (ui.min_rect().max.x - x0).max(0.0);\n"
            "            crate::topbar_modifiers::update_tight_measured(ui.ctx(), \"vectorcraft\", row_w, Some(left_w), 200.0);\n"
            "        }\n"
            "        if opened {\n"
            "            // A menu opened: File › Scripts lists the Scripts folder as it is now.\n"
            "            crate::scripts_menu::refresh(app, ui.ctx());\n"
            "        }\n"
            "    }"
        )
        if old_leading in text:
            text = text.replace(old_leading, new_leading, 1)
            changed = True
        elif old_leading_alt in text:
            text = text.replace(old_leading_alt, new_leading, 1)
            changed = True

        old_trailing = (
            "fn trailing(ui: &mut Ui, app: &mut VectorcraftApp) {\n"
            "    let safe = vectorcraft_plugins::safe_mode::active();"
        )
        new_trailing = (
            "fn trailing(ui: &mut Ui, app: &mut VectorcraftApp) {\n"
            "    let tight = crate::topbar_modifiers::is_tight(ui.ctx(), \"vectorcraft\");\n"
            "    crate::topbar_modifiers::render_topbar_modifiers_for(ui, tight);\n"
            "    let safe = vectorcraft_plugins::safe_mode::active();"
        )
        if old_trailing in text and "render_topbar_modifiers" not in text:
            text = text.replace(old_trailing, new_trailing, 1)
            changed = True

        # Legacy chrome.rs fallback
        old_menus = "let menus_end = if app.services.native_menu.is_some() { ui.cursor().min.x } else { menus::menu_bar(app, ui) };"
        new_menus = (
            "let tight = crate::topbar_modifiers::is_tight(ui.ctx(), \"vectorcraft\");\n"
            "            let full_bar = ui.max_rect();\n"
            "            let lead_used = (ui.cursor().min.x - full_bar.left()).max(0.0);\n"
            "            let right_trim = if custom { titlebar::WIDTH + 10.0 } else { 0.0 };\n"
            "            let row_w = (full_bar.width() - lead_used - right_trim).max(80.0);\n"
            "            let menus_end = if app.services.native_menu.is_some() {\n"
            "                ui.cursor().min.x\n"
            "            } else if tight {\n"
            "                let mod_w = crate::topbar_modifiers::modifier_bar_width_compact() + 12.0;\n"
            "                let avail = (row_w - mod_w - 8.0).max(80.0);\n"
            "                let resp = ui.allocate_ui_with_layout(\n"
            "                    egui::vec2(avail, APP_BAR_HEIGHT),\n"
            "                    egui::Layout::left_to_right(egui::Align::Center),\n"
            "                    |ui| {\n"
            "                        egui::ScrollArea::horizontal()\n"
            "                            .id_salt(\"vectorcraft_topbar_scroll\")\n"
            "                            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)\n"
            "                            .show(ui, |ui| {\n"
            "                                menus::menu_bar(app, ui);\n"
            "                            });\n"
            "                    },\n"
            "                );\n"
            "                crate::topbar_modifiers::update_tight_measured(ui.ctx(), \"vectorcraft\", row_w, None, 140.0);\n"
            "                resp.response.rect.right()\n"
            "            } else {\n"
            "                let edge = menus::menu_bar(app, ui);\n"
            "                let menus_w = (edge - full_bar.left() - lead_used).max(0.0);\n"
            "                crate::topbar_modifiers::update_tight_measured(ui.ctx(), \"vectorcraft\", row_w, Some(menus_w), 140.0);\n"
            "                edge\n"
            "            };"
        )
        if old_menus in text:
            text = text.replace(old_menus, new_menus, 1)
            changed = True
        if "render_topbar_modifiers" not in text:
            target = "let mut rui = ui.new_child(egui::UiBuilder::new().max_rect(right).layout(egui::Layout::right_to_left(egui::Align::Center)));\n            let ui = &mut rui;"
            replacement = (
                "let mut rui = ui.new_child(egui::UiBuilder::new().max_rect(right).layout(egui::Layout::right_to_left(egui::Align::Center)));\n"
                "            let ui = &mut rui;\n"
                "            crate::topbar_modifiers::render_topbar_modifiers_for(ui, crate::topbar_modifiers::is_tight(ui.ctx(), \"vectorcraft\"));"
            )
            if target in text:
                text = text.replace(target, replacement, 1)
                changed = True
        if changed:
            chrome_rs.write_text(text, encoding="utf-8")
            print(f"  Integrated topbar modifiers and responsive scrollable menu bar into {title}")

elif app == "designcraft":
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

    app_bar_rs = ui_egui_src / "chrome" / "app_bar.rs"
    if app_bar_rs.is_file():
        text = app_bar_rs.read_text(encoding="utf-8")
        changed = False
        old_scrolled = "let scrolled = egui::Rect::from_min_max(full.min, egui::pos2(full.max.x - caption_w, full.max.y));"
        new_scrolled = (
            "let tight_mod = crate::topbar_modifiers::is_tight(ui.ctx(), \"designcraft\");\n"
            "            let pinned_mod_w = if tight_mod { crate::topbar_modifiers::modifier_bar_width_compact() } else { 0.0 };\n"
            "            if pinned_mod_w > 0.0 {\n"
            "                let mod_rect = egui::Rect::from_min_max(egui::pos2((full.max.x - caption_w - pinned_mod_w).max(full.min.x), full.min.y), egui::pos2(full.max.x - caption_w, full.max.y));\n"
            "                ui.scope_builder(egui::UiBuilder::new().max_rect(mod_rect).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {\n"
            "                    crate::topbar_modifiers::render_topbar_modifiers_for(ui, true);\n"
            "                });\n"
            "            }\n"
            "            let scrolled = egui::Rect::from_min_max(full.min, egui::pos2((full.max.x - caption_w - pinned_mod_w).max(full.min.x + 80.0), full.max.y));"
        )
        old_tools = "let tools = ui.allocate_ui_with_layout(size, egui::Layout::right_to_left(egui::Align::Center), |ui| {\n                            ui.add_space(4.0);"
        new_tools = (
            "let tools = ui.allocate_ui_with_layout(size, egui::Layout::right_to_left(egui::Align::Center), |ui| {\n"
            "                            ui.add_space(4.0);\n"
            "                            if !tight_mod {\n"
            "                                crate::topbar_modifiers::render_topbar_modifiers_for(ui, false);\n"
            "                            }"
        )
        old_tools_end = (
            "                        if (tools.inner - tools_w).abs() > 0.5 {\n"
            "                            ui.data_mut(|d| d.insert_temp(tools_w_id, tools.inner));\n"
            "                            ui.ctx().request_discard(\"app bar: the right-hand controls changed width\");\n"
            "                        }"
        )
        new_tools_end = (
            "                        if (tools.inner - tools_w).abs() > 0.5 {\n"
            "                            ui.data_mut(|d| d.insert_temp(tools_w_id, tools.inner));\n"
            "                            ui.ctx().request_discard(\"app bar: the right-hand controls changed width\");\n"
            "                        }\n"
            "                        let base_tools = if !tight_mod { (tools.inner - crate::topbar_modifiers::modifier_bar_width_full()).max(0.0) } else { tools.inner };\n"
            "                        let left_measured = if !tight_mod { Some((menus_end - full.min.x).max(0.0)) } else { None };\n"
            "                        crate::topbar_modifiers::update_tight_measured(ui.ctx(), \"designcraft\", full.width() - caption_w, left_measured, base_tools + 16.0);"
        )
        if old_scrolled in text and "render_topbar_modifiers_for" not in text:
            text = text.replace(old_scrolled, new_scrolled, 1)
            if old_tools in text:
                text = text.replace(old_tools, new_tools, 1)
            if old_tools_end in text:
                text = text.replace(old_tools_end, new_tools_end, 1)
            changed = True
        if changed:
            app_bar_rs.write_text(text, encoding="utf-8")
            print(f"  Integrated topbar modifiers and compact pinned layout into {title}")

elif app == "wordcraft":
    ribbon_rs = ui_egui_src / "ribbon.rs"
    if ribbon_rs.is_file():
        text = ribbon_rs.read_text(encoding="utf-8")
        changed = False
        old_plan = "let plan = plan_strip(&specs, button_levels, ui.available_width(), MORE_TABS_W + 2.0);"
        new_plan = (
            "let all_tabs_w: f32 = specs.iter().map(|sp| sp.text_w + 18.0).sum();\n"
            "                        let compact_scroll = crate::topbar_modifiers::update_tight(ui.ctx(), \"wordcraft\", ui.available_width(), all_tabs_w, 165.0);\n"
            "                        let mod_w_cur = if compact_scroll { crate::topbar_modifiers::modifier_bar_width_compact() } else { crate::topbar_modifiers::modifier_bar_width_full() };\n"
            "                        let plan = plan_strip(&specs, button_levels, (ui.available_width() - mod_w_cur - 16.0).max(120.0), MORE_TABS_W + 2.0);"
        )
        if old_plan in text:
            text = text.replace(old_plan, new_plan, 1)
            changed = True

        old_tabs = "                        let tabs_layout = if crate::rtl::on() {\n                            egui::Layout::right_to_left(egui::Align::Center)\n                        } else {\n                            egui::Layout::left_to_right(egui::Align::Center)\n                        };\n                        ui.with_layout(tabs_layout, |ui| {"
        old_tabs_mid = (
            "                            for (i, ((tab, custom), _)) in tabs\n"
            "                                .iter()\n"
            "                                .zip(&plan.fits)\n"
            "                                .enumerate()\n"
            "                                .filter(|(_, ((tab, _), fits))| **fits || ghosts.iter().any(|(n, ..)| n == tab))"
        )
        new_tabs_mid = (
            "                            for (i, ((tab, custom), _)) in tabs\n"
            "                                .iter()\n"
            "                                .zip(&plan.fits)\n"
            "                                .enumerate()\n"
            "                                .filter(|(_, ((tab, _), fits))| compact_scroll || **fits || ghosts.iter().any(|(n, ..)| n == tab))"
        )
        old_w = "let w = plan.widths.get(i).copied().unwrap_or(0.0) - 2.0;"
        new_w = "let w = if compact_scroll { specs.get(i).map_or(48.0, |sp| sp.text_w + plan.pad) } else { plan.widths.get(i).copied().unwrap_or(0.0) - 2.0 };"
        old_more_cond = "if !left_out.is_empty() {"
        new_more_cond = "if !compact_scroll && !left_out.is_empty() {"

        if old_tabs in text and "wordcraft_topbar_scroll" not in text:
            text = text.replace(
                old_tabs,
                "                        let tabs_layout = if crate::rtl::on() {\n"
                "                            egui::Layout::right_to_left(egui::Align::Center)\n"
                "                        } else {\n"
                "                            egui::Layout::left_to_right(egui::Align::Center)\n"
                "                        };\n"
                "                        egui::ScrollArea::horizontal()\n"
                "                            .id_salt(\"wordcraft_topbar_scroll\")\n"
                "                            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)\n"
                "                            .hscroll(compact_scroll)\n"
                "                            .show(ui, |ui| {\n"
                "                        ui.with_layout(tabs_layout, |ui| {",
                1,
            )
            target_close = "                                    }\n                                });\n                            }\n                        });"
            if target_close in text:
                text = text.replace(
                    target_close,
                    "                                    }\n                                });\n                            }\n                        });\n                        });",
                    1,
                )
            text = text.replace(old_tabs_mid, new_tabs_mid, 1)
            text = text.replace(old_w, new_w, 1)
            text = text.replace(old_more_cond, new_more_cond, 1)
            changed = True

        if "render_topbar_modifiers" not in text:
            target = "if comments_button.clicked() {\n                                let _ = app.run(\"view.commentsPane\", json!({}));\n                            }\n                        });\n                        ui.add_space(8.0);"
            replacement = (
                "if comments_button.clicked() {\n"
                "                                let _ = app.run(\"view.commentsPane\", json!({}));\n"
                "                            }\n"
                "                        });\n"
                "                        ui.add_space(8.0);\n"
                "                        crate::topbar_modifiers::render_topbar_modifiers_for(ui, crate::topbar_modifiers::is_tight(ui.ctx(), \"wordcraft\"));\n"
                "                        ui.add_space(8.0);"
            )
            if target in text:
                text = text.replace(target, replacement, 1)
                changed = True
        if changed:
            ribbon_rs.write_text(text, encoding="utf-8")
            print(f"  Integrated topbar modifiers and responsive scrollable ribbon tabs into {title}")

elif app == "gridcraft":
    ribbon_rs = ui_egui_src / "ribbon.rs"
    if ribbon_rs.is_file():
        text = ribbon_rs.read_text(encoding="utf-8")
        changed = False
        old_block = (
            "            ui.scope_builder(egui::UiBuilder::new().max_rect(tabs_rect), |ui| {\n"
            "                ui.horizontal(|ui| {\n"
            "                    ui.spacing_mut().item_spacing.x = 2.0;\n"
            "                    ui.add_space(8.0);"
        )
        new_block = (
            "            ui.scope_builder(egui::UiBuilder::new().max_rect(tabs_rect), |ui| {\n"
            "                ui.horizontal(|ui| {\n"
            "                    ui.spacing_mut().item_spacing.x = 2.0;\n"
            "                    let compact = crate::topbar_modifiers::is_tight(ui.ctx(), \"gridcraft\");\n"
            "                    let mod_w_cur = if compact { crate::topbar_modifiers::modifier_bar_width_compact() } else { crate::topbar_modifiers::modifier_bar_width_full() };\n"
            "                    let scroll_w = (tabs_rect.width() - mod_w_cur - 12.0).max(80.0);\n"
            "                    let mut content_w = 0.0f32;\n"
            "                    ui.allocate_ui_with_layout(vec2(scroll_w, 32.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {\n"
            "                        let out = egui::ScrollArea::horizontal()\n"
            "                            .id_salt(\"gridcraft_topbar_scroll\")\n"
            "                            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)\n"
            "                            .hscroll(compact)\n"
            "                            .show(ui, |ui| {\n"
            "                                ui.horizontal(|ui| {\n"
            "                                    ui.spacing_mut().item_spacing.x = 2.0;\n"
            "                                    ui.add_space(8.0);"
        )
        old_end = "keytip(app, ui, sequence, pos2(r.center().x, r.bottom() - 1.0));\n                        }\n                    }"
        new_end = (
            "keytip(app, ui, sequence, pos2(r.center().x, r.bottom() - 1.0));\n"
            "                        }\n"
            "                    }\n"
            "                                });\n"
            "                            });\n"
            "                        content_w = out.content_size.x;\n"
            "                    });\n"
            "                    crate::topbar_modifiers::update_tight(ui.ctx(), \"gridcraft\", tabs_rect.width(), content_w, 16.0);\n"
            "                    crate::topbar_modifiers::render_topbar_modifiers_for_right(ui, compact);"
        )
        if old_block in text and "gridcraft_topbar_scroll" not in text:
            text = text.replace(old_block, new_block, 1)
            text = text.replace(old_end, new_end, 1)
            changed = True
        elif "render_topbar_modifiers" not in text and old_end in text:
            text = text.replace(old_end, old_end + "\n                    crate::topbar_modifiers::render_topbar_modifiers_for_right(ui, crate::topbar_modifiers::is_tight(ui.ctx(), \"gridcraft\"));", 1)
            changed = True
        if changed:
            ribbon_rs.write_text(text, encoding="utf-8")
            print(f"  Integrated topbar modifiers and responsive scrollable ribbon tabs into {title}")

elif app == "deckcraft":
    ribbon_rs = ui_egui_src / "ribbon.rs"
    if ribbon_rs.is_file():
        text = ribbon_rs.read_text(encoding="utf-8")
        changed = False
        old_section = (
            "        // Tabs. File comes first and opens the File tab's full-window view (Backstage), as in\n"
            "        // PowerPoint.\n"
            "        let mut x = rect.min.x + 12.0;\n"
            "        let y = rect.min.y + 2.0;\n"
        )
        old_tail = "            x += w + 2.0;\n        }\n        if app.ui.ribbon_collapsed {"
        if old_section in text and "deckcraft_topbar_scroll" not in text and old_tail in text:
            start_i = text.index(old_section)
            end_i = text.index(old_tail) + len("            x += w + 2.0;\n        }")
            compact_tabs_block = """        // Tabs. File comes first and opens the File tab's full-window view (Backstage), as in PowerPoint.
        let y = rect.min.y + 2.0;
        let compact_mod = crate::topbar_modifiers::is_tight(ui.ctx(), "deckcraft");
        let mod_w = if compact_mod { crate::topbar_modifiers::modifier_bar_width_compact() } else { crate::topbar_modifiers::modifier_bar_width_full() };
        let scroll_rect = Rect::from_min_size(pos2(rect.min.x + 8.0, y), vec2((rect.width() - mod_w - 16.0).max(80.0), 26.0));
        let mut content_w = 0.0f32;
        ui.scope_builder(egui::UiBuilder::new().max_rect(scroll_rect), |ui| {
            let out = egui::ScrollArea::horizontal()
                .id_salt("deckcraft_topbar_scroll")
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .hscroll(compact_mod)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        {
                            let w = ui.fonts_mut(|f| f.layout_no_wrap("File".to_string(), theme::font(13.0), t.text).size().x) + 18.0;
                            let (r, resp) = ui.allocate_exact_size(vec2(w, 24.0), Sense::click());
                            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "File"));
                            if resp.hovered() {
                                ui.painter().rect_filled(r.shrink2(vec2(2.0, 2.0)), CornerRadius::same(4), t.hover);
                            }
                            ui.painter().text(r.center(), Align2::CENTER_CENTER, "File", theme::font(13.0), t.text);
                            if resp.on_hover_text("File (Alt+F)").clicked() {
                                app.open_backstage(crate::backstage::Page::Home);
                            }
                        }
                        let all: Vec<(&str, bool)> = TABS.iter().map(|s| (*s, false)).chain(ctx_tabs.iter().map(|s| (*s, true))).collect();
                        for (name, contextual) in all {
                            let label = tab_label(name);
                            let font = theme::font(13.0);
                            let w = ui.fonts_mut(|f| f.layout_no_wrap(label.to_string(), font.clone(), t.text).size().x) + 18.0;
                            let (r, resp) = ui.allocate_exact_size(vec2(w, 24.0), Sense::click());
                            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
                            let active = app.ui.tab == name;
                            if resp.hovered() && !active {
                                ui.painter().rect_filled(r.shrink2(vec2(2.0, 2.0)), CornerRadius::same(4), t.hover);
                            }
                            let col = if contextual { t.contextual } else { t.text };
                            let f = if active { theme::bold(13.0) } else { font };
                            ui.painter().text(r.center(), Align2::CENTER_CENTER, label, f, col);
                            if active {
                                let uw = (w - 20.0).max(16.0);
                                ui.painter().rect_filled(Rect::from_center_size(pos2(r.center().x, r.max.y - 1.0), vec2(uw, 3.0)), CornerRadius::same(1), t.accent);
                            }
                            if resp.clicked() {
                                if active {
                                    app.ui.ribbon_collapsed = !app.ui.ribbon_collapsed;
                                } else {
                                    app.ui.tab = name.to_string();
                                    app.ui.ribbon_collapsed = false;
                                }
                            }
                            if resp.double_clicked() {
                                app.ui.ribbon_collapsed = !app.ui.ribbon_collapsed;
                            }
                        }
                    });
                });
            content_w = out.content_size.x;
        });
        crate::topbar_modifiers::update_tight(ui.ctx(), "deckcraft", rect.width(), content_w, 24.0);
        crate::topbar_modifiers::render_topbar_modifiers_at_for(ui, egui::pos2(ui.max_rect().max.x - mod_w - 8.0, y - 1.0), compact_mod);"""
            text = text[:start_i] + compact_tabs_block + text[end_i:]
            changed = True
        if changed:
            ribbon_rs.write_text(text, encoding="utf-8")
            print(f"  Integrated topbar modifiers and responsive scrollable ribbon tabs into {title}")

elif app == "tracedraw":
    mod_rs = ui_egui_src / "ui" / "mod.rs"
    if mod_rs.is_file():
        text = mod_rs.read_text(encoding="utf-8")
        bad = "menus::menu_bar(app, ui);\n                crate::topbar_modifiers::render_topbar_modifiers_right(ui);"
        if bad in text:
            text = text.replace(bad, "menus::menu_bar(app, ui);")
            mod_rs.write_text(text, encoding="utf-8")
            print(f"  Cleaned up legacy topbar modifiers call in {title} mod.rs")

    menus_rs = ui_egui_src / "ui" / "menus.rs"
    if menus_rs.is_file():
        text = menus_rs.read_text(encoding="utf-8")
        changed = False
        old_room = "let all = widths.iter().map(|w| w + gap).sum::<f32>();\n            let room = if all > ui.available_width() {\n                ui.available_width() - MORE_W\n            } else {\n                f32::INFINITY\n            };"
        new_room = (
            "let all = widths.iter().map(|w| w + gap).sum::<f32>();\n"
            "            let row_avail_w = ui.available_width();\n"
            "            let compact_scroll = crate::topbar_modifiers::update_tight(ui.ctx(), \"tracedraw\", row_avail_w, all, 16.0);\n"
            "            let mod_w = if compact_scroll { crate::topbar_modifiers::modifier_bar_width_compact() } else { crate::topbar_modifiers::modifier_bar_width_full() };\n"
            "            let room = if compact_scroll {\n"
            "                f32::INFINITY\n"
            "            } else if all > row_avail_w - mod_w {\n"
            "                (row_avail_w - mod_w - MORE_W).max(MORE_W)\n"
            "            } else {\n"
            "                f32::INFINITY\n"
            "            };"
        )
        if old_room in text:
            text = text.replace(old_room, new_room, 1)
            changed = True

        old_loop_start = "            for ((key, body), width) in menus.into_iter().zip(widths) {"
        new_loop_start = (
            "            let scroll_avail = (row_avail_w - mod_w - 8.0).max(80.0);\n"
            "            ui.allocate_ui_with_layout(egui::vec2(scroll_avail, ui.available_height()), egui::Layout::left_to_right(egui::Align::Center), |ui| {\n"
            "                egui::ScrollArea::horizontal()\n"
            "                    .id_salt(\"tracedraw_topbar_scroll\")\n"
            "                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)\n"
            "                    .hscroll(compact_scroll)\n"
            "                    .show(ui, |ui| {\n"
            "                        ui.horizontal(|ui| {\n"
            "            for ((key, body), width) in menus.into_iter().zip(widths) {"
        )
        old_loop_end = (
            "                titles.push(r);\n"
            "            }\n"
            "            // The menus under More"
        )
        new_loop_end = (
            "                titles.push(r);\n"
            "            }\n"
            "                        });\n"
            "                    });\n"
            "            });\n"
            "            // The menus under More"
        )
        if old_loop_start in text and old_loop_end in text and "tracedraw_topbar_scroll" not in text:
            text = text.replace(old_loop_start, new_loop_start, 1)
            text = text.replace(old_loop_end, new_loop_end, 1)
            changed = True

        target_render = "crate::ui::keynav::note_titles(\n                ui.ctx(),\n                crate::ui::keynav::Titles {\n                    popups: titles\n                        .iter()\n                        .map(egui::Popup::default_response_id)\n                        .collect(),\n                    keys: keys.clone(),\n                },\n            );"
        replacement_render = (
            "crate::ui::keynav::note_titles(\n"
            "                ui.ctx(),\n"
            "                crate::ui::keynav::Titles {\n"
            "                    popups: titles\n"
            "                        .iter()\n"
            "                        .map(egui::Popup::default_response_id)\n"
            "                        .collect(),\n"
            "                    keys: keys.clone(),\n"
            "                },\n"
            "            );\n"
            "            if !crate::topbar_modifiers::hardware_keyboard_connected() {\n"
            "                let row_rect = ui.max_rect();\n"
            "                let compact_mod = crate::topbar_modifiers::is_tight(ui.ctx(), \"tracedraw\");\n"
            "                let mod_w = if compact_mod { crate::topbar_modifiers::modifier_bar_width_compact() } else { crate::topbar_modifiers::modifier_bar_width_full() };\n"
            "                if row_rect.width() > mod_w + 10.0 {\n"
            "                    let right_rect = egui::Rect::from_min_max(\n"
            "                        egui::pos2((row_rect.right() - mod_w).max(row_rect.left()), row_rect.top()),\n"
            "                        row_rect.max,\n"
            "                    );\n"
            "                    ui.scope_builder(\n"
            "                        egui::UiBuilder::new()\n"
            "                            .max_rect(right_rect)\n"
            "                            .layout(egui::Layout::right_to_left(egui::Align::Center)),\n"
            "                        |ui| {\n"
            "                            crate::topbar_modifiers::render_topbar_modifiers_for(ui, compact_mod);\n"
            "                        },\n"
            "                    );\n"
            "                }\n"
            "            }"
        )
        if target_render in text and "render_topbar_modifiers" not in text:
            text = text.replace(target_render, replacement_render, 1)
            changed = True
        if changed:
            menus_rs.write_text(text, encoding="utf-8")
            print(f"  Integrated topbar modifiers and responsive scrollable menu bar into {title}")

    lib_rs = ui_egui_src / "lib.rs"
    if lib_rs.is_file():
        text = lib_rs.read_text(encoding="utf-8")
        if "topbar_modifiers::set_hardware_keyboard_connected" not in text:
            target = "crate::topbar_modifiers::apply_to_raw_input(&egui::Modifiers::NONE, raw_input);"
            replacement = "#[cfg(target_os = \"android\")]\n        crate::topbar_modifiers::set_hardware_keyboard_connected(android_activity::hardware_keyboard_connected());\n        crate::topbar_modifiers::apply_to_raw_input(&egui::Modifiers::NONE, raw_input);"
            if target in text:
                text = text.replace(target, replacement, 1)
                lib_rs.write_text(text, encoding="utf-8")
            elif "pen::filter_raw_input(ctx, raw_input);" in text:
                target = "pen::filter_raw_input(ctx, raw_input);"
                replacement = "pen::filter_raw_input(ctx, raw_input);\n        #[cfg(target_os = \"android\")]\n        crate::topbar_modifiers::set_hardware_keyboard_connected(android_activity::hardware_keyboard_connected());\n        crate::topbar_modifiers::apply_to_raw_input(&egui::Modifiers::NONE, raw_input);"
                text = text.replace(target, replacement, 1)
                lib_rs.write_text(text, encoding="utf-8")
        elif "&raw_input.modifiers" in text:
            text = text.replace("&raw_input.modifiers", "&egui::Modifiers::NONE")
            lib_rs.write_text(text, encoding="utf-8")
PY

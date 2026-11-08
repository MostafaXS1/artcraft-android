#!/usr/bin/env python3
"""Android-only FigCraft touch and viewport adaptations applied after source checkout."""
from pathlib import Path
import sys

src = Path(sys.argv[1])
ui = src / "crates/ui-egui/src"
changed = []

def replace(path, old, new, label):
    p = ui / path
    text = p.read_text()
    if new in text:
        return
    if old not in text:
        raise SystemExit(f"FigCraft mobile patch could not find expected code in {p}: {label}")
    p.write_text(text.replace(old, new, 1))
    changed.append(label)

replace(
    "menus/palette.rs",
    """    let area = Rect::from_min_max(pos2(bar.min.x, bar.min.y - 8.0 - (PALETTE_H + 16.0)), pos2(bar.max.x, bar.min.y - 8.0));""",
    """    let screen = ctx.content_rect();
    let panel_h = PALETTE_H.min((screen.height() - 32.0).max(160.0));
    let panel_w = bar_w.min((screen.width() - 24.0).max(220.0));
    let panel_x = (bar.center().x - panel_w * 0.5).clamp(
        screen.min.x + 12.0,
        (screen.max.x - 12.0 - panel_w).max(screen.min.x + 12.0),
    );
    let panel_y = (bar.min.y - 8.0 - panel_h - 16.0).max(screen.min.y + 12.0);
    let area = Rect::from_min_size(pos2(panel_x, panel_y), vec2(panel_w, panel_h));""",
    "viewport-clamped actions palette",
)
replace(
    "menus/palette.rs",
    "                let inner = bar_w - 16.0;\n                ui.set_width(inner);\n                ui.set_height(PALETTE_H);",
    "                let inner = area.width() - 16.0;\n                ui.set_width(inner);\n                ui.set_height(panel_h);",
    "responsive actions palette width and height",
)
replace(
    "menus/palette.rs",
    "                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {",
    "                egui::ScrollArea::vertical().max_height((panel_h - 100.0).max(72.0)).show(ui, |ui| {",
    "actions palette content scroll height",
)
replace(
    "menus/tree.rs",
    "        .constrain(constrain || cramped)",
    "        .constrain(true)",
    "keep submenu panels inside the mobile viewport",
)
replace(
    "menus/tree.rs",
    "    let SubmenuOptions { area, key, label, gap, constrain, tokens: t } = options;",
    "    let SubmenuOptions { area, key, label, gap, constrain: _, tokens: t } = options;",
    "discard desktop-only submenu constraint flag",
)
replace(
    "menus/tree.rs",
    """            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            let key = at_key(ui);
            ui.data_mut(|d| d.insert_temp(key, at));
            add(ui)""",
    """            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            let key = at_key(ui);
            ui.data_mut(|d| d.insert_temp(key, at));
            let max_menu_height = (ui.ctx().content_rect().height() - 24.0).max(120.0);
            egui::ScrollArea::vertical()
                .max_height(max_menu_height)
                .show(ui, add)
                .inner""",
    "vertically scrollable context and submenu menus",
)
replace(
    "toolbar.rs",
    "const GLYPH: f32 = 18.0;",
    "const GLYPH: f32 = 20.0;",
    "larger mobile tool glyphs",
)
replace(
    "toolbar.rs",
    "const FLYOUT_W: f32 = 16.0;",
    "const FLYOUT_W: f32 = 22.0;",
    "larger mobile flyout hit targets",
)
replace(
    "toolbar.rs",
    "const TOOL_CELL: f32 = 32.0;",
    "const TOOL_CELL: f32 = 40.0;",
    "larger touch targets for direct tool selection",
)
replace(
    "toolbar.rs",
    "const PAD: f32 = 8.0;",
    "const PAD: f32 = 10.0;",
    "more spacing around mobile toolbar controls",
)

if changed:
    print("Applied FigCraft Android adaptations: " + ", ".join(changed))
else:
    print("FigCraft Android adaptations already present.")

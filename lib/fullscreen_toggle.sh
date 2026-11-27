#!/usr/bin/env bash
# Injects a "Fullscreen (Immersive Mode)" toggle (on by default) under each app's Theme settings
# and wires it to the Android Activity's SharedPreferences + WindowInsetsController/SystemUiVisibility.
#
# Usage:
#   fullscreen_toggle.sh --patch-src <source-dir> <app> <title>
#   fullscreen_toggle.sh --wire-launcher <source-dir> <app> <android-dir>
set -euo pipefail

MODE="${1:?mode (--patch-src or --wire-launcher)}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [[ "$MODE" == "--patch-src" ]]; then
    SRC="${2:?source dir}"
    APP="${3:?app}"
    TITLE="${4:?title}"
    python3 - "$ROOT" "$SRC" "$APP" "$TITLE" <<'PY'
from pathlib import Path
import re
import shutil
import sys

root, src, app, title = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3], sys.argv[4]

candidates = [
    src / "crates" / "ui-egui" / "src",
    src / "crates" / "coiman-ide" / "src",
    src / "apps" / "tracedraw" / "src",
    src / "apps" / "paintcraft" / "src",
    src / "apps" / "earthcraft" / "src",
    src / "crates" / "ac-app" / "src",
    src / "crates" / "picasso-ui" / "src",
    src / "crates" / "photonic-gui" / "src",
    src / "crates" / "dc-ui" / "src",
    src / "crates" / "ui" / "src",
    src / "crates" / "ui-studio" / "src",
    src / "crates" / "prowave-app" / "src",
    src / "crates" / "presentation" / "src",
    src / "crates" / "egui-app" / "src",
    src / "crates" / "app" / "src",
    src / "native" / "src",
    src / "src",
]
ui_src = next((c for c in candidates if (c / "lib.rs").is_file() or (c / "main.rs").is_file()), None)
if ui_src is None:
    sys.exit(0)

# 1. Copy android_fullscreen.rs and register module
fs_dst = ui_src / "android_fullscreen.rs"
fs_text = (root / "templates" / "android" / "android_fullscreen.rs").read_text(encoding="utf-8")
ui_cargo = ui_src.parent / "Cargo.toml"
if ui_cargo.is_file():
    ctext = ui_cargo.read_text(encoding="utf-8", errors="ignore")
    if not re.search(r'(?m)^egui\s*[.=]', ctext) and re.search(r'(?m)^eframe\s*[.=]', ctext):
        fs_text = fs_text.replace("use std::sync::Mutex;\n", "use std::sync::Mutex;\nuse eframe::egui;\n", 1)
fs_dst.write_text(fs_text, encoding="utf-8")

def append_mod(rs_path: Path) -> None:
    if rs_path.is_file():
        t = rs_path.read_text(encoding="utf-8")
        if "mod android_fullscreen;" not in t:
            rs_path.write_text(t.rstrip() + "\npub mod android_fullscreen;\n", encoding="utf-8")

lib_rs = ui_src / "lib.rs"
main_rs = ui_src / "main.rs"
if lib_rs.is_file():
    append_mod(lib_rs)
elif main_rs.is_file():
    append_mod(main_rs)

# 2. Check if already injected anywhere in ui_src
def already_injected() -> bool:
    for rs in ui_src.rglob("*.rs"):
        if rs.name == "android_fullscreen.rs":
            continue
        if "android_fullscreen::ui(" in rs.read_text(encoding="utf-8", errors="ignore"):
            return True
    return False

if already_injected():
    sys.exit(0)

injected = []

def try_replace(rel_path: str, old: str, new: str) -> bool:
    p = src / rel_path
    if not p.is_file():
        return False
    text = p.read_text(encoding="utf-8", errors="ignore")
    if old in text and "android_fullscreen::ui" not in text:
        p.write_text(text.replace(old, new, 1), encoding="utf-8")
        injected.append(rel_path)
        return True
    return False

# --- Explicit app-specific Theme / Appearance settings injections ---
if app == "notecraft":
    try_replace(
        "crates/ui-egui/src/app.rs",
        "    ui.add_space(4.0);\n    let accent = crate::tl!(current.accent.label());",
        "    ui.add_space(4.0);\n    crate::android_fullscreen::ui(ui);\n    let accent = crate::tl!(current.accent.label());",
    )
elif app == "coimanide":
    try_replace(
        "crates/coiman-ide/src/app/shell/settings_ui.rs",
        "            if ui.button(label).clicked() {\n                *out = Some(Action::Theme);\n            }\n        }",
        "            if ui.button(label).clicked() {\n                *out = Some(Action::Theme);\n            }\n            crate::android_fullscreen::ui(ui);\n        }",
    )
elif app == "vectorcraft":
    try_replace(
        "crates/ui-egui/src/prefs_dialog.rs",
        "            PrefKind::Choice(opts) => {\n                choice_row(ui, d, sp.key, sp.label, opts, &v);",
        "            PrefKind::Choice(opts) => {\n                choice_row(ui, d, sp.key, sp.label, opts, &v);\n                if sp.key == \"uiColorScheme\" {\n                    crate::android_fullscreen::ui(ui);\n                }",
    )
elif app == "designcraft":
    try_replace(
        "crates/ui-egui/src/dialogs.rs",
        "                    });\n                    ui.add_space(8.0);\n                    crate::rtl::label(ui, egui::RichText::new(crate::i18n::tr(&lang, \"Panels\"))",
        "                    });\n                    crate::android_fullscreen::ui(ui);\n                    ui.add_space(8.0);\n                    crate::rtl::label(ui, egui::RichText::new(crate::i18n::tr(&lang, \"Panels\"))",
    )
elif app == "gridcraft":
    try_replace(
        "crates/ui-egui/src/backstage/options.rs",
        "            .response\n            .labelled_by(label.id);\n        ui.end_row();\n        let label = ui.label(lang.tr(\"Interface language\"));",
        "            .response\n            .labelled_by(label.id);\n        ui.end_row();\n        ui.label(\"\");\n        crate::android_fullscreen::ui(ui);\n        ui.end_row();\n        let label = ui.label(lang.tr(\"Interface language\"));",
    )
elif app == "deckcraft":
    try_replace(
        "crates/ui-egui/src/backstage/info.rs",
        "                .response\n                .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, \"Appearance\"));\n        });",
        "                .response\n                .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, \"Appearance\"));\n            ui.add_space(6.0);\n            crate::android_fullscreen::ui(ui);\n        });",
    )
elif app == "tracedraw":
    try_replace(
        "apps/tracedraw/src/ui/options.rs",
        "fn appearance_page(app: &mut App, ui: &mut Ui) {\n",
        "fn appearance_page(app: &mut App, ui: &mut Ui) {\n    crate::android_fullscreen::ui(ui);\n",
    )
elif app == "anicraft":
    try_replace(
        "src/dialogs.rs",
        "                    form_row(ui, LW, tr!(\"Theme\"), |ui| {\n                        ui.vertical(|ui| {\n",
        "                    form_row(ui, LW, tr!(\"Theme\"), |ui| {\n                        ui.vertical(|ui| {\n                            crate::android_fullscreen::ui(ui);\n",
    )
elif app == "bimcraft":
    try_replace(
        "crates/ui-egui/src/settings.rs",
        "        heading(ui, msg!(\"Theme\"));\n        self.theme_choices(ui, false);\n",
        "        heading(ui, msg!(\"Theme\"));\n        self.theme_choices(ui, false);\n        crate::android_fullscreen::ui(ui);\n",
    )
elif app == "clothcraft":
    try_replace(
        "src/atelier/settings.rs",
        "            ui.heading(\"Appearance\");\n",
        "            ui.heading(\"Appearance\");\n            crate::android_fullscreen::ui(ui);\n",
    )
elif app == "figcraft":
    try_replace(
        "crates/ui-egui/src/dialogs.rs",
        "    ui.label(tr(\"Theme\"));\n",
        "    ui.label(tr(\"Theme\"));\n    crate::android_fullscreen::ui(ui);\n",
    )
elif app == "pixelcraft":
    try_replace(
        "src/productivity_ui/settings.rs",
        "\"Theme\"",
        "\"Theme\"",
    )
    # Try after Theme section/label
    p_pix = src / "src/productivity_ui/settings.rs"
    if p_pix.is_file() and not injected:
        t_pix = p_pix.read_text(encoding="utf-8", errors="ignore")
        new_t, n = re.subn(r'(ui\.label\([^\n]*"Theme"[^\n]*\);)', r'\1\n            crate::android_fullscreen::ui(ui);', t_pix, count=1)
        if n == 0:
            new_t, n = re.subn(r'("Appearance",[\s\S]{0,120}?=>\s*\{)', r'\1\n                crate::android_fullscreen::ui(ui);', t_pix, count=1)
        if n > 0:
            p_pix.write_text(new_t, encoding="utf-8")
            injected.append("src/productivity_ui/settings.rs")
elif app == "humanmotioncraft":
    try_replace(
        "crates/ac-app/src/app/shell/settings.rs",
        "ui.label(\"Theme\");\n",
        "ui.label(\"Theme\");\n            crate::android_fullscreen::ui(ui);\n",
    )
elif app == "paintcraft":
    try_replace(
        "apps/paintcraft/src/app/preferences.rs",
        "Page::Appearance => appearance(ui, &mut draft.values),",
        "Page::Appearance => {\n                appearance(ui, &mut draft.values);\n                crate::android_fullscreen::ui(ui);\n            }",
    )
elif app == "cadcraft":
    try_replace(
        "crates/ui-egui/src/options/dialog.rs",
        "        let theme_combo = craft_ui::ComboBox::from_id_salt(\"options_theme\")",
        "        crate::android_fullscreen::ui(ui);\n        ui.end_row();\n        let theme_combo = craft_ui::ComboBox::from_id_salt(\"options_theme\")",
    )
elif app == "soundcraft":
    try_replace(
        "crates/ui-egui/src/panels/extra_windows.rs",
        "            if let Some(mode) = chosen {\n                let _ = app.run(\"ui.theme\", json!({\"mode\": mode.id()}));\n            }\n",
        "            if let Some(mode) = chosen {\n                let _ = app.run(\"ui.theme\", json!({\"mode\": mode.id()}));\n            }\n            crate::android_fullscreen::ui(ui);\n",
    )
elif app == "capsulecraft":
    try_replace(
        "native/src/modals/appearance.rs",
        "            body.horizontal(|ui| {\n",
        "            crate::android_fullscreen::ui(&mut body);\n            body.horizontal(|ui| {\n",
    )
elif app == "earthcraft":
    try_replace(
        "apps/earthcraft/src/app/ui/settings.rs",
        "                                    Category::Appearance => self.settings_appearance(ui),\n",
        "                                    Category::Appearance => {\n                                        self.settings_appearance(ui);\n                                        crate::android_fullscreen::ui(ui);\n                                    }\n",
    )
elif app == "ferrite":
    try_replace(
        "src/ui/settings/appearance.rs",
        "    changed\n}\n\n/// System title bar toggle",
        "    ui.add_space(4.0);\n    crate::android_fullscreen::ui(ui);\n    changed\n}\n\n/// System title bar toggle",
    )
elif app == "materialcraft":
    try_replace(
        "crates/ui-egui/src/windows.rs",
        "        crate::widgets::combo(ui, tr(\"Brightness\"), tr(p.brightness.label()), |ui| {\n            for b in Brightness::ALL {\n                ui.selectable_value(&mut p.brightness, b, tr(b.label()));\n            }\n        });\n",
        "        crate::widgets::combo(ui, tr(\"Brightness\"), tr(p.brightness.label()), |ui| {\n            for b in Brightness::ALL {\n                ui.selectable_value(&mut p.brightness, b, tr(b.label()));\n            }\n        });\n        crate::android_fullscreen::ui(ui);\n",
    )
elif app == "texturecraft":
    try_replace(
        "crates/ui-egui/src/windows.rs",
        "            ui.label(tr(\"Brightness\"));\n            egui::ComboBox::from_id_salt(\"brightness\").width(140.0).selected_text(tr(p.brightness.label())).show_ui(ui, |ui| {\n                for b in Brightness::ALL {\n                    ui.selectable_value(&mut p.brightness, b, tr(b.label()));\n                }\n            });\n            ui.end_row();\n",
        "            ui.label(tr(\"Brightness\"));\n            egui::ComboBox::from_id_salt(\"brightness\").width(140.0).selected_text(tr(p.brightness.label())).show_ui(ui, |ui| {\n                for b in Brightness::ALL {\n                    ui.selectable_value(&mut p.brightness, b, tr(b.label()));\n                }\n            });\n            ui.end_row();\n            ui.label(\"\");\n            crate::android_fullscreen::ui(ui);\n            ui.end_row();\n",
    )
elif app == "pcbcraft":
    try_replace(
        "crates/app/src/preferences.rs",
        "        settings::hint(ui, \"System follows your desktop or browser.\");\n    });\n",
        "        settings::hint(ui, \"System follows your desktop or browser.\");\n        crate::android_fullscreen::ui(ui);\n    });\n",
    )
elif app == "fieldcraft":
    try_replace(
        "crates/ui-egui/src/settings.rs",
        "            check(ui, \"settings.status_bar\", \"settings.status_bar_hint\", &mut app.ui.status_bar);\n        });",
        "            check(ui, \"settings.status_bar\", \"settings.status_bar_hint\", &mut app.ui.status_bar);\n            crate::android_fullscreen::ui(ui);\n        });",
    )
elif app == "picassosuite":
    try_replace(
        "crates/picasso-ui/src/menu_extra.rs",
        "                ui.separator();\n                ui.label(format!(\"Picasso Suite {}\", env!(\"CARGO_PKG_VERSION\")));",
        "                ui.separator();\n                crate::android_fullscreen::ui(ui);\n                ui.separator();\n                ui.label(format!(\"Picasso Suite {}\", env!(\"CARGO_PKG_VERSION\")));",
    )
elif app == "mapcraft":
    try_replace(
        "crates/ui/src/dialogs.rs",
        "        egui::Window::new(\"Preferences\").open(&mut open).default_width(460.0).show(ctx, |ui| {\n",
        "        egui::Window::new(\"Preferences\").open(&mut open).default_width(460.0).show(ctx, |ui| {\n            crate::android_fullscreen::ui(ui);\n",
    )
elif app == "effectcraft":
    settings_rs = ui_src / "panels" / "settings.rs"
    if settings_rs.is_file():
        text = settings_rs.read_text(encoding="utf-8", errors="ignore")
        if "theme_card(" in text and "ui.columns(2, |cols| {" in text:
            new_text, n = re.subn(
                r'(ui\.columns\(2,\s*\|cols\|\s*\{[\s\S]*?theme_card\([\s\S]*?\);\s*\}\);)',
                r'\1\n    crate::android_fullscreen::ui(ui);',
                text,
                count=1,
            )
            if n > 0:
                settings_rs.write_text(new_text, encoding="utf-8")
                injected.append("crates/ui-egui/src/panels/settings.rs")
elif app == "lightcraft":
    try_replace(
        "crates/ui-egui/src/dialogs/settings/appearance.rs",
        "    } else {\n        ui.horizontal_top(|ui| cards(ui, ((available - 12.0) / 2.0).floor()));\n    }\n    ui.add_space(2.0);",
        "    } else {\n        ui.horizontal_top(|ui| cards(ui, ((available - 12.0) / 2.0).floor()));\n    }\n    ui.add_space(2.0);\n    crate::android_fullscreen::ui(ui);",
    )
elif app == "filmcraft":
    try_replace(
        "crates/ui-egui/src/panels/settings.rs",
        "    ui.horizontal_top(|ui| {\n        theme_card(app, ui, d, \"appearance.lightTheme\", \"Light Theme\", settings::LIGHT_THEMES, light_active, width);\n        ui.add_space(4.0);\n        theme_card(app, ui, d, \"appearance.darkTheme\", \"Dark Theme\", settings::DARK_THEMES, !light_active, width);\n    });\n    ui.add_space(6.0);",
        "    ui.horizontal_top(|ui| {\n        theme_card(app, ui, d, \"appearance.lightTheme\", \"Light Theme\", settings::LIGHT_THEMES, light_active, width);\n        ui.add_space(4.0);\n        theme_card(app, ui, d, \"appearance.darkTheme\", \"Dark Theme\", settings::DARK_THEMES, !light_active, width);\n    });\n    ui.add_space(4.0);\n    crate::android_fullscreen::ui(ui);\n    ui.add_space(6.0);",
    )
elif app == "pdfcraft":
    if not try_replace(
        "crates/ui-egui/src/dialogs/preferences.rs",
        "    crate::appearance::preferences_section_with_context(ui, app, t, ctx);",
        "    crate::appearance::preferences_section_with_context(ui, app, t, ctx);\n    crate::android_fullscreen::ui(ui);",
    ):
        try_replace(
            "crates/ui-egui/src/dialogs/preferences.rs",
            "        AppearancePage::new(&suite, Bind::bound(&mut *self.selection))\n            .can_favorite(false)\n            .size(false)\n            .text_size(false)\n            .contrast(false)\n            .system_dark(system_dark)\n            .show(ui);",
            "        AppearancePage::new(&suite, Bind::bound(&mut *self.selection))\n            .can_favorite(false)\n            .size(false)\n            .text_size(false)\n            .contrast(false)\n            .system_dark(system_dark)\n            .show(ui);\n        crate::android_fullscreen::ui(ui);",
        )
elif app == "solidcraft":
    try_replace(
        "crates/ui-egui/src/prefs/interface.rs",
        "        ui_kit::settings_section_count(ui, \"theme\", app.tr(\"ui.theme\"), Some(&count), |ui| {\n            theme_row(app, ui);\n            theme_options_row(app, ui);\n        });",
        "        ui_kit::settings_section_count(ui, \"theme\", app.tr(\"ui.theme\"), Some(&count), |ui| {\n            theme_row(app, ui);\n            theme_options_row(app, ui);\n            crate::android_fullscreen::ui(ui);\n        });",
    )
elif app == "rustgis":
    try_replace(
        "crates/ui-egui/src/ribbon.rs",
        "            if folder {\n                ui.separator();\n                if ui.add(egui::Button::selectable(false, tl!(\"Load a Theme from File…\"))).clicked() {\n                    run_notified(app, \"ui.themeLoad\", Value::Null);\n                    ui.close();\n                }\n                if ui.add(egui::Button::selectable(false, tl!(\"Reload Themes\"))).clicked() {\n                    run_notified(app, \"ui.themeReload\", Value::Null);\n                    ui.close();\n                }\n            }",
        "            if folder {\n                ui.separator();\n                if ui.add(egui::Button::selectable(false, tl!(\"Load a Theme from File…\"))).clicked() {\n                    run_notified(app, \"ui.themeLoad\", Value::Null);\n                    ui.close();\n                }\n                if ui.add(egui::Button::selectable(false, tl!(\"Reload Themes\"))).clicked() {\n                    run_notified(app, \"ui.themeReload\", Value::Null);\n                    ui.close();\n                }\n            }\n            ui.separator();\n            crate::android_fullscreen::ui(ui);",
    )
elif app == "diagramcraft":
    try_replace(
        "crates/dc-ui/src/options.rs",
        "    r.widget_info(|| {\n        egui::WidgetInfo::labeled(egui::Role::TextInput, true, &initials_label)\n    });",
        "    r.widget_info(|| {\n        egui::WidgetInfo::labeled(egui::Role::TextInput, true, &initials_label)\n    });\n    ui.add_space(10.0);\n    crate::android_fullscreen::ui(ui);",
    )
elif app == "subcraft":
    try_replace(
        "crates/ui/src/recovery.rs",
        "egui::ComboBox::from_id_salt(\"ui_theme\")",
        "crate::android_fullscreen::ui(ui);\n    egui::ComboBox::from_id_salt(\"ui_theme\")",
    )
elif app == "photocraft":
    try_replace(
        "crates/ui-egui/src/prefs/prefs_ui/appearance.rs",
        "    if rows.dark {\n        theme_row(ui, \"Dark Theme\", !light_active, &mut dark, &mut favorites, DARK_THEME_OPTIONS);\n    }\n",
        "    if rows.dark {\n        theme_row(ui, \"Dark Theme\", !light_active, &mut dark, &mut favorites, DARK_THEME_OPTIONS);\n    }\n    ui.label(\"\");\n    crate::android_fullscreen::ui(ui);\n    ui.end_row();\n",
    )
elif app == "photonic":
    try_replace(
        "crates/photonic-gui/src/app/menu_drawer.rs",
        "                                        .on_hover_text(\n                                            \"System follows the desktop's light/dark setting; \\\n                                             Dark and Light pin the palette regardless.\",\n                                        );",
        "                                        .on_hover_text(\n                                            \"System follows the desktop's light/dark setting; \\\n                                             Dark and Light pin the palette regardless.\",\n                                        );\n                                        crate::android_fullscreen::ui(ui);",
    )
elif app == "transit":
    try_replace(
        "crates/ui-studio/src/menus.rs",
        "                        zoom.response.on_hover_text(\"Plug-in editor windows follow the display scale, not this zoom.\");",
        "                        zoom.response.on_hover_text(\"Plug-in editor windows follow the display scale, not this zoom.\");\n                        ui.separator();\n                        crate::android_fullscreen::ui(ui);",
    )
elif app == "prowave":
    ok1 = try_replace(
        "crates/prowave-app/src/project_settings.rs",
        "fn general(ui: &mut egui::Ui, p: &mut Project) {\n",
        "fn general(ui: &mut egui::Ui, p: &mut Project) {\n    crate::android_fullscreen::ui(ui);\n",
    )
    ok2 = try_replace(
        "crates/prowave-app/src/menus.rs",
        "        if row(ui, tr(it), on, check, key, false, look).clicked() {\n            apply(it, p, s);\n            ui.close();\n        }\n    }",
        "        if row(ui, tr(it), on, check, key, false, look).clicked() {\n            apply(it, p, s);\n            ui.close();\n        }\n    }\n    if items.contains(&\"Enter Full Screen\") {\n        separator(ui, look);\n        crate::android_fullscreen::ui(ui);\n    }",
    )
elif app == "papercraft":
    try_replace(
        "src/app.rs",
        "        if self.theme {\n            self.status_footer(ui);\n            self.theme_panel(ui);",
        "        if self.theme {\n            self.status_footer(ui);\n            crate::android_fullscreen::ui(ui);\n            self.theme_panel(ui);",
    )

# --- General fallbacks for any other app (or Storytold AppearancePage apps) ---
if not injected:
    for rs in sorted(ui_src.rglob("*.rs")):
        if "test" in rs.name or rs.name == "android_fullscreen.rs":
            continue
        text = rs.read_text(encoding="utf-8", errors="ignore")
        if "AppearancePage::new(" in text:
            new_text, n = re.subn(
                r"(AppearancePage::new\([^\n;]*\.show\(ui\);)",
                r"\1\n        crate::android_fullscreen::ui(ui);",
                text,
                count=1,
            )
            if n > 0:
                rs.write_text(new_text, encoding="utf-8")
                injected.append(str(rs.relative_to(src)))
                break

if not injected:
    for rs in sorted(ui_src.rglob("*.rs")):
        if "test" in rs.name or rs.name == "android_fullscreen.rs":
            continue
        text = rs.read_text(encoding="utf-8", errors="ignore")
        if "fn theme_picker(" in text and "combo.response.labelled_by(label.id);\n    });\n}" in text:
            text = text.replace(
                "combo.response.labelled_by(label.id);\n    });\n}",
                "combo.response.labelled_by(label.id);\n    });\n    crate::android_fullscreen::ui(ui);\n}",
                1,
            )
            rs.write_text(text, encoding="utf-8")
            injected.append(str(rs.relative_to(src)))
            break

if not injected:
    for rs in sorted(ui_src.rglob("*.rs")):
        if "test" in rs.name or rs.name == "android_fullscreen.rs":
            continue
        text = rs.read_text(encoding="utf-8", errors="ignore")
        for pat in (
            r"(ThemeMenu::new\([^\n;]*\.show\([^\)]*\);)",
            r"(ui\.selectable_value\([^\n;]*(?:Theme|Appearance)::[^\n;]*\);)",
            r"(egui::ComboBox::from_id_salt\(\"[^\"]*theme[^\"]*\"\)[^\n;]*\.show_ui\(ui,[\s\S]{0,600}?\}\);)",
            r"(ui\.heading\([^\)]*\"(?:Theme|Appearance)\"[^\)]*\);)",
            r"(egui::Window::new\([^\)]*\"(?:Preferences|Settings)\"[^\)]*\)[\s\S]{0,120}?\.show\(ctx,\s*\|ui\|\s*\{)",
        ):
            m = re.search(pat, text, re.IGNORECASE)
            if m:
                idx = m.end()
                new_text = text[:idx] + "\n    crate::android_fullscreen::ui(ui);" + text[idx:]
                rs.write_text(new_text, encoding="utf-8")
                injected.append(str(rs.relative_to(src)))
                break
        if injected:
            break

if injected:
    print(f"  Added Fullscreen (Immersive Mode) toggle under Theme settings in {title}: {', '.join(injected)}")
PY
elif [[ "$MODE" == "--wire-launcher" ]]; then
    SRC="${2:?source dir}"
    APP="${3:?app}"
    ANDROID_DIR="${4:?android dir}"
    python3 - "$ROOT" "$SRC" "$APP" "$ANDROID_DIR" <<'PY'
from pathlib import Path
import re
import sys

root, src, app, android_dir = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3], Path(sys.argv[4])

# 1. Ensure Java Activity has isImmersiveFullscreen / setImmersiveFullscreen & getTopbarButtonsMask
java_snippet = """    public int getTopbarButtonsMask() {
        return getSharedPreferences("artcraft_prefs", MODE_PRIVATE)
                .getInt("topbar_buttons_mask", 0x700);
    }

    public void setTopbarButtonsMask(final int mask) {
        getSharedPreferences("artcraft_prefs", MODE_PRIVATE)
                .edit()
                .putInt("topbar_buttons_mask", mask)
                .apply();
    }

    public boolean isImmersiveFullscreen() {
        return getSharedPreferences("artcraft_prefs", MODE_PRIVATE)
                .getBoolean("immersive_fullscreen", true);
    }

    public void setImmersiveFullscreen(final boolean enabled) {
        getSharedPreferences("artcraft_prefs", MODE_PRIVATE)
                .edit()
                .putBoolean("immersive_fullscreen", enabled)
                .apply();
        runOnUiThread(new Runnable() {
            @Override
            public void run() {
                lastInsetsHash = -1;
                applyImmersiveFullscreen();
                applyCaptionBarInsets(null);
                Window window = getWindow();
                if (window != null) {
                    window.getDecorView().requestApplyInsets();
                }
                try {
                    dispatchKeyEvent(new android.view.KeyEvent(android.view.KeyEvent.ACTION_DOWN, android.view.KeyEvent.KEYCODE_UNKNOWN));
                    dispatchKeyEvent(new android.view.KeyEvent(android.view.KeyEvent.ACTION_UP, android.view.KeyEvent.KEYCODE_UNKNOWN));
                } catch (Throwable ignored) {}
            }
        });
    }

    public void applyImmersiveFullscreen() {
        Window window = getWindow();
        if (window == null) {
            return;
        }
        if (isDesktopOrFreeform()) {
            return;
        }
        if (!isImmersiveFullscreen()) {
            WindowCompat.setDecorFitsSystemWindows(window, false);
            int barBg = lightTheme ? 0xFFF5F5F5 : 0xFF1E1E1E;
            try {
                window.setStatusBarColor(barBg);
                window.setNavigationBarColor(barBg);
            } catch (Throwable ignored) {}
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                WindowInsetsController controller = window.getInsetsController();
                if (controller != null) {
                    controller.show(WindowInsets.Type.systemBars());
                }
            } else {
                int visibility = window.getDecorView().getSystemUiVisibility();
                window.getDecorView().setSystemUiVisibility(
                        (visibility & (View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR
                                | View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR))
                                | View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                                | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
                                | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN);
            }
            return;
        }
        WindowCompat.setDecorFitsSystemWindows(window, false);
        try {
            window.setStatusBarColor(0x00000000);
            window.setNavigationBarColor(0x00000000);
        } catch (Throwable ignored) {}"""

old_java = """    private void applyImmersiveFullscreen() {
        Window window = getWindow();
        if (window == null) {
            return;
        }
        if (isDesktopOrFreeform()) {
            return;
        }"""

old_cap_java_re = re.compile(
    r"""            int top = 0;\n"""
    r"""            if \(Build\.VERSION\.SDK_INT >= Build\.VERSION_CODES\.R\) \{\n"""
    r"""                WindowInsets rootInsets = insets != null \? insets : decor\.getRootWindowInsets\(\);\n"""
    r"""                if \(rootInsets != null\) \{\n"""
    r"""                    top = rootInsets\.getInsets\(WindowInsets\.Type\.captionBar\(\)\)\.top;\n"""
    r"""                    if \(top == 0 && isDesktopOrFreeform\(\)\) \{\n"""
    r"""                        top = rootInsets\.getInsetsIgnoringVisibility\(\n"""
    r"""                                WindowInsets\.Type\.captionBar\(\) \| WindowInsets\.Type\.statusBars\(\)\)\.top;\n"""
    r"""                    \}\n"""
    r"""                \}\n"""
    r"""            \} else if \(insets != null\) \{\n"""
    r"""                top = insets\.getSystemWindowInsetTop\(\);\n"""
    r"""            \}\n"""
    r"""            if \(top == captionBarPadding\) \{\n"""
    r"""                return;\n"""
    r"""            \}\n"""
    r"""            captionBarPadding = top;\n"""
    r"""(?:            //[^\n]*\n)*"""
    r"""            content\.setBackgroundColor\(top > 0 \? \(lightTheme \? 0xFFF5F5F5 : 0xFF1E1E1E\) : 0x00000000\);\n"""
    r"""            content\.setPadding\(0, top, 0, 0\);\n"""
    r"""            if \(content\.getLayoutParams\(\) instanceof ViewGroup\.MarginLayoutParams\) \{\n"""
    r"""                ViewGroup\.MarginLayoutParams mlp = \(ViewGroup\.MarginLayoutParams\) content\.getLayoutParams\(\);\n"""
    r"""                if \(mlp\.topMargin != top\) \{\n"""
    r"""                    mlp\.topMargin = top;\n"""
    r"""                    content\.setLayoutParams\(mlp\);\n"""
    r"""                    content\.requestLayout\(\);\n"""
    r"""                \}\n"""
    r"""            \}"""
)

new_cap_java = """            int top = 0;
            int bottom = 0;
            int left = 0;
            int right = 0;
            boolean nonImmersive = !isImmersiveFullscreen() && !isDesktopOrFreeform();
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                WindowInsets rootInsets = insets != null ? insets : decor.getRootWindowInsets();
                if (rootInsets != null) {
                    if (nonImmersive) {
                        android.graphics.Insets sys = rootInsets.getInsetsIgnoringVisibility(
                                WindowInsets.Type.systemBars() | WindowInsets.Type.displayCutout());
                        left = sys.left;
                        top = sys.top;
                        right = sys.right;
                        bottom = sys.bottom;
                    } else {
                        top = rootInsets.getInsets(WindowInsets.Type.captionBar()).top;
                        if (top == 0 && isDesktopOrFreeform()) {
                            top = rootInsets.getInsetsIgnoringVisibility(
                                    WindowInsets.Type.captionBar() | WindowInsets.Type.statusBars()).top;
                        }
                    }
                }
            } else if (insets != null) {
                if (nonImmersive) {
                    left = insets.getSystemWindowInsetLeft();
                    top = insets.getSystemWindowInsetTop();
                    right = insets.getSystemWindowInsetRight();
                    bottom = insets.getSystemWindowInsetBottom();
                } else {
                    top = insets.getSystemWindowInsetTop();
                }
            }
            int hash = (top & 0xFF) ^ ((bottom & 0xFF) << 8) ^ ((left & 0xFF) << 16) ^ ((right & 0xFF) << 24);
            if (hash == lastInsetsHash && top == captionBarPadding && bottom == safeInsetBottom && left == safeInsetLeft && right == safeInsetRight) {
                return;
            }
            lastInsetsHash = hash;
            captionBarPadding = top;
            safeInsetBottom = bottom;
            safeInsetLeft = left;
            safeInsetRight = right;
            decor.setBackgroundColor((nonImmersive || top > 0) ? (lightTheme ? 0xFFF5F5F5 : 0xFF1E1E1E) : 0xFF000000);
            content.setBackgroundColor(0x00000000);
            content.setPadding(0, 0, 0, 0);
            if (content.getLayoutParams() instanceof ViewGroup.MarginLayoutParams) {
                ViewGroup.MarginLayoutParams mlp = (ViewGroup.MarginLayoutParams) content.getLayoutParams();
                if (mlp.leftMargin != 0 || mlp.topMargin != 0 || mlp.rightMargin != 0 || mlp.bottomMargin != 0) {
                    mlp.leftMargin = 0;
                    mlp.topMargin = 0;
                    mlp.rightMargin = 0;
                    mlp.bottomMargin = 0;
                    content.setLayoutParams(mlp);
                    content.requestLayout();
                }
            }
            try {
                dispatchKeyEvent(new android.view.KeyEvent(android.view.KeyEvent.ACTION_DOWN, android.view.KeyEvent.KEYCODE_UNKNOWN));
                dispatchKeyEvent(new android.view.KeyEvent(android.view.KeyEvent.ACTION_UP, android.view.KeyEvent.KEYCODE_UNKNOWN));
            } catch (Throwable ignored) {}"""

old_safe_top_java = """    public int getSafeInsetTop() {
        return captionBarPadding;
    }"""

new_safe_insets_java = """    private int safeInsetBottom = 0;
    private int safeInsetLeft = 0;
    private int safeInsetRight = 0;
    private int lastInsetsHash = -1;

    public int getSafeInsetTop() {
        return Math.max(0, captionBarPadding);
    }

    public int getSafeInsetBottom() {
        return safeInsetBottom;
    }

    public int getSafeInsetLeft() {
        return safeInsetLeft;
    }

    public int getSafeInsetRight() {
        return safeInsetRight;
    }"""

old_java_nodesktop = """    private void applyImmersiveFullscreen() {
        Window window = getWindow();
        if (window == null) {
            return;
        }"""

java_snippet_nodesktop = """    public int getTopbarButtonsMask() {
        return getSharedPreferences("artcraft_prefs", MODE_PRIVATE)
                .getInt("topbar_buttons_mask", 0x700);
    }

    public void setTopbarButtonsMask(final int mask) {
        getSharedPreferences("artcraft_prefs", MODE_PRIVATE)
                .edit()
                .putInt("topbar_buttons_mask", mask)
                .apply();
    }

    public boolean isImmersiveFullscreen() {
        return getSharedPreferences("artcraft_prefs", MODE_PRIVATE)
                .getBoolean("immersive_fullscreen", true);
    }

    public void setImmersiveFullscreen(final boolean enabled) {
        getSharedPreferences("artcraft_prefs", MODE_PRIVATE)
                .edit()
                .putBoolean("immersive_fullscreen", enabled)
                .apply();
        runOnUiThread(new Runnable() {
            @Override
            public void run() {
                applyImmersiveFullscreen();
                applyNonImmersiveInsets(null);
                Window window = getWindow();
                if (window != null) {
                    window.getDecorView().requestApplyInsets();
                    try {
                        dispatchKeyEvent(new android.view.KeyEvent(android.view.KeyEvent.ACTION_DOWN, android.view.KeyEvent.KEYCODE_UNKNOWN));
                        dispatchKeyEvent(new android.view.KeyEvent(android.view.KeyEvent.ACTION_UP, android.view.KeyEvent.KEYCODE_UNKNOWN));
                    } catch (Throwable ignored) {}
                }
            }
        });
    }

    private void applyNonImmersiveInsets(WindowInsets insets) {
        Window window = getWindow();
        if (window == null) {
            return;
        }
        View decor = window.getDecorView();
        View content = decor.findViewById(android.R.id.content);
        if (content == null) {
            return;
        }
        int left = 0, top = 0, right = 0, bottom = 0;
        if (!isImmersiveFullscreen()) {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                WindowInsets rootInsets = insets != null ? insets : decor.getRootWindowInsets();
                if (rootInsets != null) {
                    android.graphics.Insets sys = rootInsets.getInsetsIgnoringVisibility(
                            WindowInsets.Type.systemBars() | WindowInsets.Type.displayCutout());
                    left = sys.left;
                    top = sys.top;
                    right = sys.right;
                    bottom = sys.bottom;
                }
            } else if (insets != null) {
                left = insets.getSystemWindowInsetLeft();
                top = insets.getSystemWindowInsetTop();
                right = insets.getSystemWindowInsetRight();
                bottom = insets.getSystemWindowInsetBottom();
            }
        }
        decor.setBackgroundColor(!isImmersiveFullscreen() ? 0xFF1E1E1E : 0xFF000000);
        if (content.getLayoutParams() instanceof ViewGroup.MarginLayoutParams) {
            ViewGroup.MarginLayoutParams mlp = (ViewGroup.MarginLayoutParams) content.getLayoutParams();
            if (mlp.leftMargin != left || mlp.topMargin != top || mlp.rightMargin != right || mlp.bottomMargin != bottom) {
                mlp.leftMargin = left;
                mlp.topMargin = top;
                mlp.rightMargin = right;
                mlp.bottomMargin = bottom;
                content.setLayoutParams(mlp);
                content.requestLayout();
            }
        }
    }

    public void applyImmersiveFullscreen() {
        Window window = getWindow();
        if (window == null) {
            return;
        }
        if (!isImmersiveFullscreen()) {
            window.clearFlags(android.view.WindowManager.LayoutParams.FLAG_FULLSCREEN);
            try {
                window.setStatusBarColor(0xFF1E1E1E);
                window.setNavigationBarColor(0xFF1E1E1E);
            } catch (Throwable ignored) {}
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                window.setDecorFitsSystemWindows(false);
                WindowInsetsController controller = window.getInsetsController();
                if (controller != null) {
                    controller.show(WindowInsets.Type.systemBars());
                }
            } else {
                int visibility = window.getDecorView().getSystemUiVisibility();
                window.getDecorView().setSystemUiVisibility(
                        (visibility & (View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR
                                | View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR))
                                | View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                                | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
                                | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN);
            }
            applyNonImmersiveInsets(null);
            return;
        }
        window.addFlags(android.view.WindowManager.LayoutParams.FLAG_FULLSCREEN);
        try {
            window.setStatusBarColor(0x00000000);
            window.setNavigationBarColor(0x00000000);
        } catch (Throwable ignored) {}
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            window.setDecorFitsSystemWindows(false);
        }
        applyNonImmersiveInsets(null);"""

for java_file in (android_dir / "java").rglob("*.java"):
    jtext = java_file.read_text(encoding="utf-8", errors="ignore")
    modified = False
    if "getSafeInsetBottom()" not in jtext and old_safe_top_java in jtext:
        jtext = jtext.replace(old_safe_top_java, new_safe_insets_java, 1)
        modified = True
    if "applyImmersiveFullscreen()" in jtext and "isImmersiveFullscreen()" not in jtext:
        if old_java in jtext:
            jtext = jtext.replace(old_java, java_snippet, 1)
            modified = True
        elif old_java_nodesktop in jtext:
            jtext = jtext.replace(old_java_nodesktop, java_snippet_nodesktop, 1)
            jtext = jtext.replace("readScreenInsets(decor);", "applyNonImmersiveInsets(null);\n                    readScreenInsets(decor);", 1)
            jtext = jtext.replace(
                "if ((visibility & View.SYSTEM_UI_FLAG_FULLSCREEN) == 0) {",
                "if (isImmersiveFullscreen() && (visibility & View.SYSTEM_UI_FLAG_FULLSCREEN) == 0) {",
                1,
            )
            modified = True
    if old_cap_java_re.search(jtext):
        jtext = old_cap_java_re.sub(new_cap_java, jtext, count=1)
        modified = True
    old_bar_bg = """                if (captionBarPadding > 0) {
                    View content = getWindow() != null
                            ? getWindow().getDecorView().findViewById(android.R.id.content) : null;
                    if (content != null) {
                        content.setBackgroundColor(lightTheme ? 0xFFF5F5F5 : 0xFF1E1E1E);
                    }
                }"""
    new_bar_bg = """                int barBg = lightTheme ? 0xFFF5F5F5 : 0xFF1E1E1E;
                View decor = getWindow() != null ? getWindow().getDecorView() : null;
                if (decor != null && (!isImmersiveFullscreen() || captionBarPadding > 0)) {
                    decor.setBackgroundColor(barBg);
                }
                if (getWindow() != null && !isImmersiveFullscreen()) {
                    try {
                        getWindow().setStatusBarColor(barBg);
                        getWindow().setNavigationBarColor(barBg);
                    } catch (Throwable ignored) {}
                }"""
    if old_bar_bg in jtext:
        jtext = jtext.replace(old_bar_bg, new_bar_bg, 1)
        modified = True
    if modified:
        java_file.write_text(jtext, encoding="utf-8")
    elif "getTopbarButtonsMask()" not in jtext and "public boolean isImmersiveFullscreen() {" in jtext:
        topbar_java = """    public int getTopbarButtonsMask() {
        return getSharedPreferences("artcraft_prefs", MODE_PRIVATE)
                .getInt("topbar_buttons_mask", 0x700);
    }

    public void setTopbarButtonsMask(final int mask) {
        getSharedPreferences("artcraft_prefs", MODE_PRIVATE)
                .edit()
                .putInt("topbar_buttons_mask", mask)
                .apply();
    }

    public boolean isImmersiveFullscreen() {"""
        java_file.write_text(jtext.replace("    public boolean isImmersiveFullscreen() {", topbar_java, 1), encoding="utf-8")

# 2. Find UI crate ident
candidates = [
    src / "crates" / "ui-egui",
    src / "crates" / "coiman-ide",
    src / "apps" / "tracedraw",
    src / "apps" / "paintcraft",
    src / "apps" / "earthcraft",
    src / "crates" / "ac-app",
    src / "crates" / "picasso-ui",
    src / "crates" / "photonic-gui",
    src / "crates" / "dc-ui",
    src / "crates" / "ui",
    src / "crates" / "ui-studio",
    src / "crates" / "prowave-app",
    src / "crates" / "presentation",
    src / "crates" / "egui-app",
    src / "crates" / "app",
    src / "native",
    src,
]
ui_crate_dir = next((c for c in candidates if (c / "src" / "android_fullscreen.rs").is_file()), None)
if ui_crate_dir is None or not (ui_crate_dir / "Cargo.toml").is_file():
    sys.exit(0)

cargo_text = (ui_crate_dir / "Cargo.toml").read_text(encoding="utf-8", errors="ignore")
m = re.search(r'(?m)^name\s*=\s*"([^"]+)"', cargo_text)
if not m:
    sys.exit(0)
ui_ident = m.group(1).replace("-", "_")

# Ensure lib.rs generated during prepare.sh (e.g. ferrite/anicraft) declares android_fullscreen
ui_lib_rs = ui_crate_dir / "src" / "lib.rs"
if ui_lib_rs.is_file():
    utext = ui_lib_rs.read_text(encoding="utf-8")
    if "mod android_fullscreen;" not in utext:
        ui_lib_rs.write_text(utext.rstrip() + "\npub mod android_fullscreen;\n", encoding="utf-8")

launcher_cargo = (android_dir / "Cargo.toml").read_text(encoding="utf-8", errors="ignore")
if m.group(1) not in launcher_cargo and ui_ident not in launcher_cargo:
    sys.exit(0)

lib_rs = android_dir / "src" / "lib.rs"
if not lib_rs.is_file():
    sys.exit(0)

text = lib_rs.read_text(encoding="utf-8")

# 3. Guard Rust's apply_immersive_fullscreen if present
if not re.search(r"android_fullscreen::is_enabled", text):
    m_apply = re.search(r"fn apply_immersive_fullscreen\s*\(\s*app\s*:\s*&(?:android_activity::)?AndroidApp\s*\)\s*\{", text)
    if m_apply:
        pos_a = m_apply.end()
        text = text[:pos_a] + f"\n    if !{ui_ident}::android_fullscreen::is_enabled() {{\n        return;\n    }}" + text[pos_a:]

# 4. Inject init_fullscreen_toggle(&android_app) (and topbar button persistence if topbar_modifiers exists)
has_topbar = (ui_crate_dir / "src" / "topbar_modifiers.rs").is_file()
redo_ctrl_y = "true" if app in ("wordcraft", "gridcraft", "deckcraft") else "false"
topbar_init = ""
if has_topbar:
    topbar_init = f"""
    {ui_ident}::topbar_modifiers::set_redo_uses_ctrl_y({redo_ctrl_y});
    if let Ok(mask) = jvm.attach_current_thread(|env| -> jni::errors::Result<i32> {{
        let activity = unsafe {{ env.as_cast_raw::<JObject>(&activity_raw)? }};
        env.call_method(&*activity, jni_str!("getTopbarButtonsMask"), jni_sig!("()I"), &[])?.i()
    }}) {{
        {ui_ident}::topbar_modifiers::set_initial_mask(mask as u32);
    }}
    let app_for_tb = app.clone();
    {ui_ident}::topbar_modifiers::set_mask_handler(move |mask| {{
        let jvm = unsafe {{ JavaVM::from_raw(app_for_tb.vm_as_ptr() as *mut _) }};
        let activity_raw = app_for_tb.activity_as_ptr() as jni::sys::jobject;
        let _ = jvm.attach_current_thread(|env| -> jni::errors::Result<()> {{
            let activity = unsafe {{ env.as_cast_raw::<JObject>(&activity_raw)? }};
            env.call_method(
                &*activity,
                jni_str!("setTopbarButtonsMask"),
                jni_sig!("(I)V"),
                &[JValue::Int(mask as i32)],
            )?;
            Ok(())
        }});
    }});"""

if "init_fullscreen_toggle" not in text:
    helper = f"""
fn init_fullscreen_toggle(app: &android_activity::AndroidApp) {{
    use jni::{{jni_sig, jni_str, JavaVM, objects::{{JObject, JValue}}}};
    let jvm = unsafe {{ JavaVM::from_raw(app.vm_as_ptr() as *mut _) }};
    let activity_raw = app.activity_as_ptr() as jni::sys::jobject;
    if let Ok(enabled) = jvm.attach_current_thread(|env| -> jni::errors::Result<bool> {{
        let activity = unsafe {{ env.as_cast_raw::<JObject>(&activity_raw)? }};
        env.call_method(&*activity, jni_str!("isImmersiveFullscreen"), jni_sig!("()Z"), &[])?.z()
    }}) {{
        {ui_ident}::android_fullscreen::set_initial(enabled);
    }}
    let app_for_cb = app.clone();
    {ui_ident}::android_fullscreen::set_handler(move |enabled| {{
        let jvm = unsafe {{ JavaVM::from_raw(app_for_cb.vm_as_ptr() as *mut _) }};
        let activity_raw = app_for_cb.activity_as_ptr() as jni::sys::jobject;
        let _ = jvm.attach_current_thread(|env| -> jni::errors::Result<()> {{
            let activity = unsafe {{ env.as_cast_raw::<JObject>(&activity_raw)? }};
            env.call_method(
                &*activity,
                jni_str!("setImmersiveFullscreen"),
                jni_sig!("(Z)V"),
                &[JValue::Bool(enabled)],
            )?;
            Ok(())
        }});
    }});{topbar_init}
}}
"""
    text = text + "\n" + helper
    m_main = re.search(r"(?:pub\s+)?fn android_main\s*\(\s*([a-zA-Z0-9_]+)\s*:\s*(?:android_activity::)?AndroidApp\s*\)\s*\{", text)
    if m_main:
        arg_name = m_main.group(1)
        pos = m_main.end()
        text = text[:pos] + f"\n    init_fullscreen_toggle(&{arg_name});" + text[pos:]
    lib_rs.write_text(text, encoding="utf-8")
    print(f"  Wired fullscreen toggle JNI bridge in {android_dir.name}/src/lib.rs ({ui_ident})")
PY
fi

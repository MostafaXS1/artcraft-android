#!/usr/bin/env bash
# Integrated Unofficial Android Port Disclaimer in About dialogs across apps.
#
# Usage: about_dialog.sh <source-dir> <app> <title>
set -euo pipefail

SRC="${1:?source dir}"
APP="${2:?app}"
TITLE="${3:?title}"

python3 - "$SRC" "$APP" "$TITLE" <<'PY'
from pathlib import Path
import sys

src = Path(sys.argv[1])
app = sys.argv[2]
title = sys.argv[3]

DISCLAIMER_TEXT = (
    "We are an independent group of people who love ArtCraft very much "
    "and want to make them available for android. We are not "
    "affiliated with related to or endorsed by ArtCraft / Storytold "
    "and all our ports are completely unofficial"
)
DISCLAIMER_URL = "https://github.com/MostafaXS1/artcraft-android"
DISCLAIMER_LINK_TEXT = "Source code: github.com/MostafaXS1/artcraft-android"

MARKER = "You are using an unofficial android port of"


def patch_file(path: Path, target: str, replacement: str) -> bool:
    if not path.is_file():
        return False
    content = path.read_text(encoding="utf-8")
    if MARKER in content:
        return False
    if target in content:
        new_content = content.replace(target, replacement, 1)
        path.write_text(new_content, encoding="utf-8")
        print(f"  Applied unofficial disclaimer to {title} in {path.relative_to(src)}")
        return True
    return False


patched = False

# App-specific accurate injection points
if app == "wordcraft":
    dispatch_rs = src / "crates" / "ui-egui" / "src" / "dialogs" / "dispatch.rs"
    target = """\
    match *tab {
        1 => crate::credits::contributors_ui(ui),
        2 => crate::credits::models_ui(ui),
        _ => {"""
    replacement = f"""\
    match *tab {{
        1 => crate::credits::contributors_ui(ui),
        2 => crate::credits::models_ui(ui),
        _ => {{
            ui.label(egui::RichText::new("You are using an unofficial android port of {title}").font(semibold(18.0)));
            ui.add_space(4.0);
            ui.label("{DISCLAIMER_TEXT}");
            ui.add_space(4.0);
            ui.hyperlink_to("{DISCLAIMER_LINK_TEXT}", "{DISCLAIMER_URL}");
            ui.add_space(12.0);"""
    patched = patch_file(dispatch_rs, target, replacement)

elif app == "soundcraft":
    about_rs = src / "crates" / "ui-egui" / "src" / "panels" / "about.rs"
    target = """\
        match tab {
            1 => crate::credits::contributors_ui(ui),
            2 => crate::credits::models_ui(ui),
            _ => {"""
    replacement = f"""\
        match tab {{
            1 => crate::credits::contributors_ui(ui),
            2 => crate::credits::models_ui(ui),
            _ => {{
                ui.label(egui::RichText::new("You are using an unofficial android port of {title}").font(bold(18.0)));
                ui.add_space(4.0);
                ui.label("{DISCLAIMER_TEXT}");
                ui.add_space(4.0);
                ui.hyperlink_to("{DISCLAIMER_LINK_TEXT}", "{DISCLAIMER_URL}");
                ui.add_space(12.0);"""
    patched = patch_file(about_rs, target, replacement)

elif app == "lightcraft":
    dialogs_rs = src / "crates" / "ui-egui" / "src" / "dialogs.rs"
    target = """\
    match tab {
        1 => crate::credits::contributors_ui(app, ui),
        2 => crate::credits::models_ui(ui),
        _ => {"""
    replacement = f"""\
    match tab {{
        1 => crate::credits::contributors_ui(app, ui),
        2 => crate::credits::models_ui(ui),
        _ => {{
            ui.label(egui::RichText::new("You are using an unofficial android port of {title}").font(t.semibold(18.0)).color(t.text));
            ui.add_space(4.0);
            ui.label("{DISCLAIMER_TEXT}");
            ui.add_space(4.0);
            ui.hyperlink_to("{DISCLAIMER_LINK_TEXT}", "{DISCLAIMER_URL}");
            ui.add_space(12.0);"""
    patched = patch_file(dialogs_rs, target, replacement)

elif app == "vectorcraft":
    about_rs = src / "crates" / "ui-egui" / "src" / "dialogs" / "about.rs"
    target = """fn about_tab(app: &mut VectorcraftApp, ui: &mut egui::Ui) {\n    ui.set_width(380.0);"""
    replacement = f"""fn about_tab(app: &mut VectorcraftApp, ui: &mut egui::Ui) {{
    ui.set_width(520.0);
    ui.label(egui::RichText::new("You are using an unofficial android port of {title}").font(theme::semibold(18.0)));
    ui.add_space(4.0);
    ui.label("{DISCLAIMER_TEXT}");
    ui.add_space(4.0);
    ui.hyperlink_to("{DISCLAIMER_LINK_TEXT}", "{DISCLAIMER_URL}");
    ui.add_space(12.0);"""
    patched = patch_file(about_rs, target, replacement)

elif app == "filmcraft":
    dialogs_rs = src / "crates" / "ui-egui" / "src" / "panels" / "dialogs.rs"
    target = """fn about_tab(app: &mut FilmcraftApp, ui: &mut egui::Ui) {\n    use crate::icons::{self, Icon};\n    use crate::links;\n    let t = app.tokens;\n    ui.set_width(380.0);"""
    replacement = f"""fn about_tab(app: &mut FilmcraftApp, ui: &mut egui::Ui) {{
    use crate::icons::{{self, Icon}};
    use crate::links;
    let t = app.tokens;
    ui.set_width(520.0);
    ui.heading("You are using an unofficial android port of {title}");
    ui.add_space(4.0);
    ui.label("{DISCLAIMER_TEXT}");
    ui.add_space(4.0);
    ui.hyperlink_to("{DISCLAIMER_LINK_TEXT}", "{DISCLAIMER_URL}");
    ui.add_space(12.0);"""
    patched = patch_file(dialogs_rs, target, replacement)

elif app == "photocraft":
    dialogs_rs = src / "crates" / "ui-egui" / "src" / "dialogs.rs"
    target = """\
    match tab {
        "contributors" => crate::credits::contributors_ui(app, ui),
        "models" => crate::credits::models_ui(ui),
        _ => {"""
    replacement = f"""\
    match tab {{
        "contributors" => crate::credits::contributors_ui(app, ui),
        "models" => crate::credits::models_ui(ui),
        _ => {{
            ui.heading("You are using an unofficial android port of {title}");
            ui.add_space(4.0);
            ui.label("{DISCLAIMER_TEXT}");
            ui.add_space(4.0);
            ui.hyperlink_to("{DISCLAIMER_LINK_TEXT}", "{DISCLAIMER_URL}");
            ui.add_space(12.0);"""
    patched = patch_file(dialogs_rs, target, replacement)

elif app == "effectcraft":
    dialogs_rs = src / "crates" / "ui-egui" / "src" / "panels" / "dialogs.rs"
    target = """fn about_main(app: &mut EffectcraftApp, ui: &mut egui::Ui, t: &Tokens, cmd: &mut Option<&'static str>) {"""
    replacement = f"""fn about_main(app: &mut EffectcraftApp, ui: &mut egui::Ui, t: &Tokens, cmd: &mut Option<&'static str>) {{
    ui.heading("You are using an unofficial android port of {title}");
    ui.add_space(4.0);
    ui.label("{DISCLAIMER_TEXT}");
    ui.add_space(4.0);
    ui.hyperlink_to("{DISCLAIMER_LINK_TEXT}", "{DISCLAIMER_URL}");
    ui.add_space(12.0);"""
    patched = patch_file(dialogs_rs, target, replacement)

elif app == "cadcraft":
    dialogs_rs = src / "crates" / "ui-egui" / "src" / "dialogs.rs"
    target = """\
            match tab {
                1 => crate::credits::contributors_ui(ui),
                2 => crate::credits::models_ui(ui),
                _ => {
                    ui.heading(crate::tl!("CADCraft"));"""
    replacement = f"""\
            match tab {{
                1 => crate::credits::contributors_ui(ui),
                2 => crate::credits::models_ui(ui),
                _ => {{
                    ui.heading("You are using an unofficial android port of {title}");
                    ui.add_space(4.0);
                    ui.label("{DISCLAIMER_TEXT}");
                    ui.add_space(4.0);
                    ui.hyperlink_to("{DISCLAIMER_LINK_TEXT}", "{DISCLAIMER_URL}");
                    ui.add_space(12.0);
                    ui.heading(crate::tl!("CADCraft"));"""
    patched = patch_file(dialogs_rs, target, replacement)

elif app == "designcraft":
    about_rs = src / "crates" / "ui-egui" / "src" / "about.rs"
    target = """\
        ui.vertical_centered(|ui| {
            ui.add_space(8.0);
            let (r, _) = ui.allocate_exact_size(vec2(72.0, 72.0), Sense::hover());"""
    replacement = f"""\
        ui.vertical_centered(|ui| {{
            ui.label(egui::RichText::new("You are using an unofficial android port of {title}").font(semibold(18.0)).color(t.text_strong));
            ui.add_space(4.0);
            ui.label("{DISCLAIMER_TEXT}");
            ui.add_space(4.0);
            ui.hyperlink_to("{DISCLAIMER_LINK_TEXT}", "{DISCLAIMER_URL}");
            ui.add_space(12.0);
            ui.add_space(8.0);
            let (r, _) = ui.allocate_exact_size(vec2(72.0, 72.0), Sense::hover());"""
    patched = patch_file(about_rs, target, replacement)

# Generic fallback for any other app with crates/ui-egui
if not patched:
    ui_src = src / "crates" / "ui-egui" / "src"
    if ui_src.is_dir():
        for rs in ui_src.rglob("*.rs"):
            content = rs.read_text(encoding="utf-8")
            if MARKER in content:
                break
            if "crate::credits::contributors_ui" in content and "match " in content and "_ => {" in content:
                target = "_ => {"
                replacement = f"""_ => {{
            ui.heading("You are using an unofficial android port of {title}");
            ui.add_space(4.0);
            ui.label("{DISCLAIMER_TEXT}");
            ui.add_space(4.0);
            ui.hyperlink_to("{DISCLAIMER_LINK_TEXT}", "{DISCLAIMER_URL}");
            ui.add_space(12.0);"""
                if patch_file(rs, target, replacement):
                    break
PY

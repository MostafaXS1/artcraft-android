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
import re
import sys

src = Path(sys.argv[1])
app = sys.argv[2]
title = sys.argv[3]

DISCLAIMER_P1 = (
    "We are an independent group of people who love ArtCraft very much "
    "and want to make them available for android. We are not "
    "affiliated with, related to or endorsed by ArtCraft / Storytold "
    "and all our ports are completely unofficial"
)
DISCLAIMER_P2 = (
    "Please direct any bugs/issues related to Android porting to our GitHub. "
    "Do NOT go asking for help in the official ArtCraft discord server, "
    "Mailing list, or GitHub. They don't maintain mobile apps. Don't waste their time"
)
DISCLAIMER_URL = "https://github.com/MostafaXS1/artcraft-android"
DISCLAIMER_LINK_TEXT = "Source code: github.com/MostafaXS1/artcraft-android"

MARKER = "You are using an unofficial android port of"
MARKER_P2 = "Do NOT go asking for help in the official ArtCraft discord server"


def patch_file(path: Path, target: str, replacement: str) -> bool:
    if not path.is_file():
        return False
    content = path.read_text(encoding="utf-8")
    if MARKER in content:
        changed = False
        new_content = content

        # Check if missing paragraph 2 completely
        if MARKER_P2 not in new_content:
            old_p1_pattern = r'ui\.label\("We are an independent group of people who love ArtCraft very much[^"]+"\);'
            replacement_labels = (
                f'ui.add(egui::Label::new("{DISCLAIMER_P1}").wrap());\n'
                f'            ui.add_space(4.0);\n'
                f'            ui.add(egui::Label::new("{DISCLAIMER_P2}").wrap());'
            )
            new_content = re.sub(old_p1_pattern, replacement_labels, new_content)
            changed = True
        else:
            # Upgrade paragraph 2 wording if needed
            if "They don't maintain mobile apps. Don't waste their time" not in new_content:
                new_content = re.sub(
                    r"They don't maintain mobile apps[^\"]*\"",
                    "They don't maintain mobile apps. Don't waste their time\"",
                    new_content,
                )
                changed = True

        # Ensure paragraph 1 wraps
        p1_unwrapped = re.compile(r'ui\.label\("We are an independent group of people who love ArtCraft very much[^"]+"\);')
        if p1_unwrapped.search(new_content):
            new_content = p1_unwrapped.sub(f'ui.add(egui::Label::new("{DISCLAIMER_P1}").wrap());', new_content)
            changed = True

        # Ensure paragraph 2 wraps
        p2_unwrapped = re.compile(r'ui\.label\("Please direct any bugs/issues related to Android porting to our GitHub[^"]+"\);')
        if p2_unwrapped.search(new_content):
            new_content = p2_unwrapped.sub(f'ui.add(egui::Label::new("{DISCLAIMER_P2}").wrap());', new_content)
            changed = True

        # Ensure headings wrap
        heading_plain = re.compile(r'ui\.heading\("(You are using an unofficial android port of [^"]+)"\);')
        if heading_plain.search(new_content):
            new_content = heading_plain.sub(r'ui.add(egui::Label::new(egui::RichText::new("\1").heading()).wrap());', new_content)
            changed = True

        heading_rich = re.compile(r'ui\.label\((egui::RichText::new\("You are using an unofficial android port of [^"]+"\)[^;]+)\);')
        if heading_rich.search(new_content):
            new_content = heading_rich.sub(r'ui.add(egui::Label::new(\1).wrap());', new_content)
            changed = True

        # Ensure divider separator follows hyperlink_to before native about content
        sep_pattern = re.compile(
            r'([ \t]*)(ui\.hyperlink_to\("[^"]*artcraft-android"[^)]*\);)\r?\n[ \t]*ui\.add_space\(12\.0\);'
        )
        if sep_pattern.search(new_content):
            def replace_sep(m):
                indent = m.group(1)
                return (
                    f"{indent}{m.group(2)}\n"
                    f"{indent}ui.add_space(8.0);\n"
                    f"{indent}ui.separator();\n"
                    f"{indent}ui.add_space(8.0);"
                )
            new_content = sep_pattern.sub(replace_sep, new_content)
            changed = True

        if changed and new_content != content:
            path.write_text(new_content, encoding="utf-8")
            print(f"  Upgraded unofficial disclaimer for {title} in {path.relative_to(src)}")
            return True
        return False

    if target in content:
        new_content = content.replace(target, replacement, 1)
        path.write_text(new_content, encoding="utf-8")
        print(f"  Applied unofficial disclaimer to {title} in {path.relative_to(src)}")
        return True
    return False


code_block = f"""ui.add(egui::Label::new(egui::RichText::new("You are using an unofficial android port of {title}").heading()).wrap());
            ui.add_space(4.0);
            ui.add(egui::Label::new("{DISCLAIMER_P1}").wrap());
            ui.add_space(4.0);
            ui.add(egui::Label::new("{DISCLAIMER_P2}").wrap());
            ui.add_space(4.0);
            ui.hyperlink_to("{DISCLAIMER_LINK_TEXT}", "{DISCLAIMER_URL}");
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);"""

patched = False

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
            ui.add(egui::Label::new(egui::RichText::new("You are using an unofficial android port of {title}").font(semibold(18.0))).wrap());
            ui.add_space(4.0);
            ui.add(egui::Label::new("{DISCLAIMER_P1}").wrap());
            ui.add_space(4.0);
            ui.add(egui::Label::new("{DISCLAIMER_P2}").wrap());
            ui.add_space(4.0);
            ui.hyperlink_to("{DISCLAIMER_LINK_TEXT}", "{DISCLAIMER_URL}");
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);"""
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
                ui.add(egui::Label::new(egui::RichText::new("You are using an unofficial android port of {title}").font(bold(18.0))).wrap());
                ui.add_space(4.0);
                ui.add(egui::Label::new("{DISCLAIMER_P1}").wrap());
                ui.add_space(4.0);
                ui.add(egui::Label::new("{DISCLAIMER_P2}").wrap());
                ui.add_space(4.0);
                ui.hyperlink_to("{DISCLAIMER_LINK_TEXT}", "{DISCLAIMER_URL}");
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);"""
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
            ui.add(egui::Label::new(egui::RichText::new("You are using an unofficial android port of {title}").font(t.semibold(18.0)).color(t.text)).wrap());
            ui.add_space(4.0);
            ui.add(egui::Label::new("{DISCLAIMER_P1}").wrap());
            ui.add_space(4.0);
            ui.add(egui::Label::new("{DISCLAIMER_P2}").wrap());
            ui.add_space(4.0);
            ui.hyperlink_to("{DISCLAIMER_LINK_TEXT}", "{DISCLAIMER_URL}");
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);"""
    patched = patch_file(dialogs_rs, target, replacement)

elif app == "vectorcraft":
    about_rs = src / "crates" / "ui-egui" / "src" / "dialogs" / "about.rs"
    target = """fn about_tab(app: &mut VectorcraftApp, ui: &mut egui::Ui) {\n    ui.set_width(380.0);"""
    replacement = f"""fn about_tab(app: &mut VectorcraftApp, ui: &mut egui::Ui) {{
    ui.set_width(520.0);
    ui.add(egui::Label::new(egui::RichText::new("You are using an unofficial android port of {title}").font(theme::semibold(18.0))).wrap());
    ui.add_space(4.0);
    ui.add(egui::Label::new("{DISCLAIMER_P1}").wrap());
    ui.add_space(4.0);
    ui.add(egui::Label::new("{DISCLAIMER_P2}").wrap());
    ui.add_space(4.0);
    ui.hyperlink_to("{DISCLAIMER_LINK_TEXT}", "{DISCLAIMER_URL}");
    ui.add_space(8.0);
    ui.separator();
    ui.add_space(8.0);"""
    patched = patch_file(about_rs, target, replacement)

elif app == "filmcraft":
    dialogs_rs = src / "crates" / "ui-egui" / "src" / "panels" / "dialogs.rs"
    target = """fn about_tab(app: &mut FilmcraftApp, ui: &mut egui::Ui) {\n    use crate::icons::{self, Icon};\n    use crate::links;\n    let t = app.tokens;\n    ui.set_width(380.0);"""
    replacement = f"""fn about_tab(app: &mut FilmcraftApp, ui: &mut egui::Ui) {{
    use crate::icons::{{self, Icon}};
    use crate::links;
    let t = app.tokens;
    ui.set_width(520.0);
    ui.add(egui::Label::new(egui::RichText::new("You are using an unofficial android port of {title}").heading()).wrap());
    ui.add_space(4.0);
    ui.add(egui::Label::new("{DISCLAIMER_P1}").wrap());
    ui.add_space(4.0);
    ui.add(egui::Label::new("{DISCLAIMER_P2}").wrap());
    ui.add_space(4.0);
    ui.hyperlink_to("{DISCLAIMER_LINK_TEXT}", "{DISCLAIMER_URL}");
    ui.add_space(8.0);
    ui.separator();
    ui.add_space(8.0);"""
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
            ui.add(egui::Label::new(egui::RichText::new("You are using an unofficial android port of {title}").heading()).wrap());
            ui.add_space(4.0);
            ui.add(egui::Label::new("{DISCLAIMER_P1}").wrap());
            ui.add_space(4.0);
            ui.add(egui::Label::new("{DISCLAIMER_P2}").wrap());
            ui.add_space(4.0);
            ui.hyperlink_to("{DISCLAIMER_LINK_TEXT}", "{DISCLAIMER_URL}");
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);"""
    patched = patch_file(dialogs_rs, target, replacement)

elif app == "effectcraft":
    dialogs_rs = src / "crates" / "ui-egui" / "src" / "panels" / "dialogs.rs"
    target = """fn about_main(app: &mut EffectcraftApp, ui: &mut egui::Ui, t: &Tokens, cmd: &mut Option<&'static str>) {"""
    replacement = f"""fn about_main(app: &mut EffectcraftApp, ui: &mut egui::Ui, t: &Tokens, cmd: &mut Option<&'static str>) {{
    ui.add(egui::Label::new(egui::RichText::new("You are using an unofficial android port of {title}").heading()).wrap());
    ui.add_space(4.0);
    ui.add(egui::Label::new("{DISCLAIMER_P1}").wrap());
    ui.add_space(4.0);
    ui.add(egui::Label::new("{DISCLAIMER_P2}").wrap());
    ui.add_space(4.0);
    ui.hyperlink_to("{DISCLAIMER_LINK_TEXT}", "{DISCLAIMER_URL}");
    ui.add_space(8.0);
    ui.separator();
    ui.add_space(8.0);"""
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
                    ui.add(egui::Label::new(egui::RichText::new("You are using an unofficial android port of {title}").heading()).wrap());
                    ui.add_space(4.0);
                    ui.add(egui::Label::new("{DISCLAIMER_P1}").wrap());
                    ui.add_space(4.0);
                    ui.add(egui::Label::new("{DISCLAIMER_P2}").wrap());
                    ui.add_space(4.0);
                    ui.hyperlink_to("{DISCLAIMER_LINK_TEXT}", "{DISCLAIMER_URL}");
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(8.0);
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
            ui.add(egui::Label::new(egui::RichText::new("You are using an unofficial android port of {title}").font(semibold(18.0)).color(t.text_strong)).wrap());
            ui.add_space(4.0);
            ui.add(egui::Label::new("{DISCLAIMER_P1}").wrap());
            ui.add_space(4.0);
            ui.add(egui::Label::new("{DISCLAIMER_P2}").wrap());
            ui.add_space(4.0);
            ui.hyperlink_to("{DISCLAIMER_LINK_TEXT}", "{DISCLAIMER_URL}");
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);
            let (r, _) = ui.allocate_exact_size(vec2(72.0, 72.0), Sense::hover());"""
    patched = patch_file(about_rs, target, replacement)

elif app == "figcraft":
    dialogs_rs = src / "crates" / "ui-egui" / "src" / "dialogs.rs"
    target = "fn about_body(ui: &mut egui::Ui, t: &Tokens) {"
    replacement = f"""fn about_body(ui: &mut egui::Ui, t: &Tokens) {{
            {code_block}"""
    patched = patch_file(dialogs_rs, target, replacement)

elif app == "deckcraft":
    about_rs = src / "crates" / "ui-egui" / "src" / "dialogs" / "about.rs"
    target = "pub fn about(_app: &mut SlideApp, ui: &mut Ui) {\n    let t = Tokens::get(ui.ctx());\n    ui.vertical_centered(|ui| {"
    replacement = f"""pub fn about(_app: &mut SlideApp, ui: &mut Ui) {{
    let t = Tokens::get(ui.ctx());
    ui.vertical_centered(|ui| {{
        {code_block}"""
    patched = patch_file(about_rs, target, replacement)

elif app == "pdfcraft":
    dialogs_rs = src / "crates" / "ui-egui" / "src" / "dialogs.rs"
    target = "match tab {\n        1 => crate::credits::contributors_ui(ui),\n        2 => crate::credits::models_ui(ui),\n        _ => {"
    replacement = f"""match tab {{
        1 => crate::credits::contributors_ui(ui),
        2 => crate::credits::models_ui(ui),
        _ => {{
            {code_block}"""
    patched = patch_file(dialogs_rs, target, replacement)

elif app == "solidcraft":
    help_rs = src / "crates" / "ui-egui" / "src" / "help.rs"
    if help_rs.is_file():
        content = help_rs.read_text(encoding="utf-8")
        marker_target = 'ui_kit::Window::new(egui::Id::unique("sc_about")'
        if marker_target in content and MARKER not in content:
            idx = content.find(marker_target)
            idx2 = content.find('show(ctx, true, |ui| {', idx)
            if idx2 != -1:
                target_str = 'show(ctx, true, |ui| {'
                replacement = f"""show(ctx, true, |ui| {{
            {code_block}"""
                new_content = content[:idx2] + replacement + content[idx2 + len(target_str):]
                help_rs.write_text(new_content, encoding="utf-8")
                print(f"  Applied unofficial disclaimer to {title} in {help_rs.relative_to(src)}")
                patched = True
        elif MARKER in content:
            patch_file(help_rs, "", "")
            patched = True

elif app == "materialcraft":
    windows_rs = src / "crates" / "ui-egui" / "src" / "windows.rs"
    target = 'modal(ctx, "about", tr("About MaterialCraft"), &mut open, vec2(400.0, 260.0), |ui| {\n        let tk = Tokens::get(ui.ctx());\n        ui.vertical_centered(|ui| {'
    replacement = f"""modal(ctx, "about", tr("About MaterialCraft"), &mut open, vec2(540.0, 420.0), |ui| {{
        let tk = Tokens::get(ui.ctx());
        ui.vertical_centered(|ui| {{
            {code_block}"""
    patched = patch_file(windows_rs, target, replacement)

elif app == "texturecraft":
    windows_rs = src / "crates" / "ui-egui" / "src" / "windows.rs"
    target = 'dialog("about", tr("About TextureCraft"), &mut open).show(ctx, |ui| {\n        ui.vertical_centered(|ui| {'
    replacement = f"""dialog("about", tr("About TextureCraft"), &mut open).show(ctx, |ui| {{
        ui.set_width(520.0);
        ui.vertical_centered(|ui| {{
            {code_block}"""
    patched = patch_file(windows_rs, target, replacement)

elif app == "gridcraft":
    about_rs = src / "crates" / "ui-egui" / "src" / "dialogs" / "about.rs"
    target = 'match d.tab.as_str() {\n        "Contributors" => crate::credits::contributors_ui(ui),\n        "Models" => crate::credits::models_ui(ui),\n        _ => {'
    replacement = f"""match d.tab.as_str() {{
        "Contributors" => crate::credits::contributors_ui(ui),
        "Models" => crate::credits::models_ui(ui),
        _ => {{
            {code_block}"""
    patched = patch_file(about_rs, target, replacement)

# Generic fallback for any other app with crates/ui-egui
if not patched:
    ui_src = src / "crates" / "ui-egui" / "src"
    if ui_src.is_dir():
        for rs in sorted(ui_src.rglob("*.rs")):
            if not rs.is_file():
                continue
            content = rs.read_text(encoding="utf-8")
            if MARKER in content:
                break
            if "crate::credits::contributors_ui" in content and "match " in content and "_ => {" in content:
                target = "_ => {"
                replacement = f"""_ => {{
            {code_block}"""
                if patch_file(rs, target, replacement):
                    break
PY

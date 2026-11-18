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


FIRST_RUN_HELPER = r"""
#[allow(dead_code)]
fn is_first_run(app_name: &str) -> bool {
    static CHECKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    static IS_FIRST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    if CHECKED.load(std::sync::atomic::Ordering::Relaxed) {
        return IS_FIRST.swap(false, std::sync::atomic::Ordering::Relaxed);
    }

    let base_dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(std::path::PathBuf::from))
        .or_else(|| {
            std::env::var_os("TMPDIR").and_then(|p| {
                let pb = std::path::PathBuf::from(p);
                pb.parent().map(|p| p.to_path_buf())
            })
        })
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    let marker = base_dir.join(format!(".artcraft_{}_about_seen", app_name));
    if marker.exists() {
        CHECKED.store(true, std::sync::atomic::Ordering::Relaxed);
        IS_FIRST.store(false, std::sync::atomic::Ordering::Relaxed);
        false
    } else {
        let _ = std::fs::create_dir_all(&base_dir);
        let _ = std::fs::write(&marker, b"seen\n");
        CHECKED.store(true, std::sync::atomic::Ordering::Relaxed);
        IS_FIRST.store(false, std::sync::atomic::Ordering::Relaxed);
        true
    }
}
"""


def inject_first_run(path: Path, target: str, replacement: str) -> bool:
    if not path.is_file():
        return False
    content = path.read_text(encoding="utf-8")
    if "is_first_run(" in content:
        return False
    if target not in content:
        return False
    new_content = content.replace(target, replacement, 1)
    if "fn is_first_run(" not in new_content:
        new_content += "\n" + FIRST_RUN_HELPER
    path.write_text(new_content, encoding="utf-8")
    print(f"  Added first-run About dialog open for {title} in {path.relative_to(src)}")
    return True


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
    target = "_ => {\n            ui.label(egui::RichText::new(tl!(\"WordCraft\")).font(semibold(22.0)));"
    replacement = f"""_ => {{
            ui.add(egui::Label::new(egui::RichText::new("You are using an unofficial android port of {title}").font(semibold(18.0))).wrap());
            ui.add_space(4.0);
            ui.add(egui::Label::new("{DISCLAIMER_P1}").wrap());
            ui.add_space(4.0);
            ui.add(egui::Label::new("{DISCLAIMER_P2}").wrap());
            ui.add_space(4.0);
            ui.hyperlink_to("{DISCLAIMER_LINK_TEXT}", "{DISCLAIMER_URL}");
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);
            ui.label(egui::RichText::new(tl!("WordCraft")).font(semibold(22.0)));"""
    patched = patch_file(dispatch_rs, target, replacement)

    chrome_rs = src / "crates" / "ui-egui" / "src" / "dialogs" / "chrome.rs"
    fr_target = "pub fn show(app: &mut WordApp, ctx: &egui::Context) {\n    let Some(mut d) = app.dialog.take() else {"
    fr_repl = 'pub fn show(app: &mut WordApp, ctx: &egui::Context) {\n    if app.dialog.is_none() && is_first_run("wordcraft") {\n        app.dialog = Some(Dialog::About { tab: 0 });\n    }\n    let Some(mut d) = app.dialog.take() else {'
    inject_first_run(chrome_rs, fr_target, fr_repl)

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

    fr_target = "pub(super) fn about(app: &mut SoundApp, ctx: &egui::Context) {\n    let mut open = app.ui.show_about;\n    if !open {\n        return;\n    }"
    fr_repl = 'pub(super) fn about(app: &mut SoundApp, ctx: &egui::Context) {\n    if !app.ui.show_about && is_first_run("soundcraft") {\n        app.ui.show_about = true;\n    }\n    let mut open = app.ui.show_about;\n    if !open {\n        return;\n    }'
    inject_first_run(about_rs, fr_target, fr_repl)

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

    fr_target = "pub fn show(app: &mut LightcraftApp, ctx: &egui::Context, blocked: bool) {\n    let Some(mut dlg) = app.ui.dialog.clone() else { return };"
    fr_repl = 'pub fn show(app: &mut LightcraftApp, ctx: &egui::Context, blocked: bool) {\n    if app.ui.dialog.is_none() && is_first_run("lightcraft") {\n        app.ui.dialog = Some(crate::state::Dialog::About);\n    }\n    let Some(mut dlg) = app.ui.dialog.clone() else { return };'
    inject_first_run(dialogs_rs, fr_target, fr_repl)

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

    fr_target = "pub(super) fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {\n    if !app.ui.about {\n        return;\n    }"
    fr_repl = 'pub(super) fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {\n    if !app.ui.about && is_first_run("vectorcraft") {\n        app.ui.about = true;\n    }\n    if !app.ui.about {\n        return;\n    }'
    inject_first_run(about_rs, fr_target, fr_repl)

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

    fr_target = "    crate::panels::misc::history_dialogs(app, ctx);\n    let Some(d) = app.dialog else { return };"
    fr_repl = '    crate::panels::misc::history_dialogs(app, ctx);\n    if app.dialog.is_none() && is_first_run("filmcraft") {\n        app.dialog = Some(Dialog::About);\n    }\n    let Some(d) = app.dialog else { return };'
    inject_first_run(dialogs_rs, fr_target, fr_repl)

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

    fr_target = "pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {\n"
    fr_repl = 'pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {\n    if app.ui.dialogs.is_empty() && is_first_run("photocraft") {\n        app.ui.open_dialog(crate::state::DialogKind::About, Default::default());\n    }\n'
    inject_first_run(dialogs_rs, fr_target, fr_repl)

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

    fr_target = "pub fn show(app: &mut EffectcraftApp, ctx: &egui::Context) {\n    let Some(d) = app.dialog else { return };"
    fr_repl = 'pub fn show(app: &mut EffectcraftApp, ctx: &egui::Context) {\n    if app.dialog.is_none() && is_first_run("effectcraft") {\n        app.dialog = Some(Dialog::About);\n    }\n    let Some(d) = app.dialog else { return };'
    inject_first_run(dialogs_rs, fr_target, fr_repl)

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

    fr_target = "pub fn show(app: &mut CadApp, ctx: &egui::Context) {\n    crate::mtext_editor::show(app, ctx);\n    crate::quick::quick_properties(app, ctx);\n    let Some(d) = app.ui.dialog.clone() else { return };"
    fr_repl = 'pub fn show(app: &mut CadApp, ctx: &egui::Context) {\n    crate::mtext_editor::show(app, ctx);\n    crate::quick::quick_properties(app, ctx);\n    if app.ui.dialog.is_none() && is_first_run("cadcraft") {\n        app.ui.dialog = Some("about".into());\n    }\n    let Some(d) = app.ui.dialog.clone() else { return };'
    inject_first_run(dialogs_rs, fr_target, fr_repl)

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

    fr_target = "pub fn show(app: &mut DesignApp, ctx: &egui::Context) {\n    if !app.ui.about {\n        return;\n    }"
    fr_repl = 'pub fn show(app: &mut DesignApp, ctx: &egui::Context) {\n    if !app.ui.about && is_first_run("designcraft") {\n        app.ui.about = true;\n    }\n    if !app.ui.about {\n        return;\n    }'
    inject_first_run(about_rs, fr_target, fr_repl)

elif app == "figcraft":
    dialogs_rs = src / "crates" / "ui-egui" / "src" / "dialogs.rs"
    target = "fn about_body(ui: &mut egui::Ui, t: &Tokens) {"
    replacement = f"""fn about_body(ui: &mut egui::Ui, t: &Tokens) {{
            {code_block}"""
    patched = patch_file(dialogs_rs, target, replacement)

    fr_target = "    let t = Tokens::get(ctx);\n    if app.ui.about {\n        let mut open = true;"
    fr_repl = '    let t = Tokens::get(ctx);\n    if !app.ui.about && is_first_run("figcraft") {\n        app.ui.about = true;\n    }\n    if app.ui.about {\n        let mut open = true;'
    inject_first_run(dialogs_rs, fr_target, fr_repl)

elif app == "deckcraft":
    about_rs = src / "crates" / "ui-egui" / "src" / "dialogs" / "about.rs"
    target = "pub fn about(_app: &mut SlideApp, ui: &mut Ui) {\n    let t = Tokens::get(ui.ctx());\n    ui.vertical_centered(|ui| {"
    replacement = f"""pub fn about(_app: &mut SlideApp, ui: &mut Ui) {{
    let t = Tokens::get(ui.ctx());
    ui.vertical_centered(|ui| {{
        {code_block}"""
    patched = patch_file(about_rs, target, replacement)

    dialogs_rs = src / "crates" / "ui-egui" / "src" / "dialogs.rs"
    fr_target = "pub fn show(app: &mut SlideApp, ctx: &egui::Context) {\n    let Some(mut d) = app.dialog.take() else { return };"
    fr_repl = 'pub fn show(app: &mut SlideApp, ctx: &egui::Context) {\n    if app.dialog.is_none() && is_first_run("deckcraft") {\n        app.dialog = Some(Dialog::new("about"));\n    }\n    let Some(mut d) = app.dialog.take() else { return };'
    inject_first_run(dialogs_rs, fr_target, fr_repl)

elif app == "pdfcraft":
    dialogs_rs = src / "crates" / "ui-egui" / "src" / "dialogs.rs"
    target = "match tab {\n        1 => crate::credits::contributors_ui(ui),\n        2 => crate::credits::models_ui(ui),\n        _ => {"
    replacement = f"""match tab {{
        1 => crate::credits::contributors_ui(ui),
        2 => crate::credits::models_ui(ui),
        _ => {{
            {code_block}"""
    patched = patch_file(dialogs_rs, target, replacement)

    fr_target = "fn dialogs(app: &mut PdfCraftApp, ctx: &egui::Context) {\n    password(app, ctx);"
    fr_repl = 'fn dialogs(app: &mut PdfCraftApp, ctx: &egui::Context) {\n    if app.dialog.is_none() && is_first_run("pdfcraft") {\n        app.dialog = Some(Dialog::About);\n    }\n    password(app, ctx);'
    inject_first_run(dialogs_rs, fr_target, fr_repl)

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

        fr_target = "fn about(app: &mut SolveApp, ctx: &egui::Context) {\n    if !app.help.about {\n        return;\n    }"
        fr_repl = 'fn about(app: &mut SolveApp, ctx: &egui::Context) {\n    if !app.help.about {\n        if is_first_run("solidcraft") {\n            app.help.about = true;\n        } else {\n            return;\n        }\n    }'
        inject_first_run(help_rs, fr_target, fr_repl)

elif app == "materialcraft":
    windows_rs = src / "crates" / "ui-egui" / "src" / "windows.rs"
    target = 'modal(ctx, "about", tr("About MaterialCraft"), &mut open, vec2(400.0, 260.0), |ui| {\n        let tk = Tokens::get(ui.ctx());\n        ui.vertical_centered(|ui| {'
    replacement = f"""modal(ctx, "about", tr("About MaterialCraft"), &mut open, vec2(540.0, 420.0), |ui| {{
        let tk = Tokens::get(ui.ctx());
        ui.vertical_centered(|ui| {{
            {code_block}"""
    patched = patch_file(windows_rs, target, replacement)

    fr_target = "    if app.ui.windows.preferences {\n        preferences(app, ctx);\n    }\n    if app.ui.windows.about {\n        about(app, ctx);\n    }"
    fr_repl = '    if app.ui.windows.preferences {\n        preferences(app, ctx);\n    }\n    if !app.ui.windows.about && is_first_run("materialcraft") {\n        app.ui.windows.about = true;\n    }\n    if app.ui.windows.about {\n        about(app, ctx);\n    }'
    inject_first_run(windows_rs, fr_target, fr_repl)

elif app == "texturecraft":
    windows_rs = src / "crates" / "ui-egui" / "src" / "windows.rs"
    target = 'dialog("about", tr("About TextureCraft"), &mut open).show(ctx, |ui| {\n        ui.vertical_centered(|ui| {'
    replacement = f"""dialog("about", tr("About TextureCraft"), &mut open).show(ctx, |ui| {{
        ui.set_width(520.0);
        ui.vertical_centered(|ui| {{
            {code_block}"""
    patched = patch_file(windows_rs, target, replacement)

    fr_target = "fn about(app: &mut TextureCraftApp, ctx: &egui::Context) {\n    let tk = Tokens::get(ctx);\n    let mut open = app.ui.windows.about;"
    fr_repl = 'fn about(app: &mut TextureCraftApp, ctx: &egui::Context) {\n    if !app.ui.windows.about && is_first_run("texturecraft") {\n        app.ui.windows.about = true;\n    }\n    let tk = Tokens::get(ctx);\n    let mut open = app.ui.windows.about;'
    inject_first_run(windows_rs, fr_target, fr_repl)

elif app == "gridcraft":
    about_rs = src / "crates" / "ui-egui" / "src" / "dialogs" / "about.rs"
    target = 'match d.tab.as_str() {\n        "Contributors" => crate::credits::contributors_ui(ui),\n        "Models" => crate::credits::models_ui(ui),\n        _ => {'
    replacement = f"""match d.tab.as_str() {{
        "Contributors" => crate::credits::contributors_ui(ui),
        "Models" => crate::credits::models_ui(ui),
        _ => {{
            {code_block}"""
    patched = patch_file(about_rs, target, replacement)

    show_rs = src / "crates" / "ui-egui" / "src" / "dialogs" / "show.rs"
    fr_target = "pub fn show(app: &mut SheetApp, ctx: &egui::Context) {\n    let Some(mut d) = app.dialog.take() else { return };"
    fr_repl = 'pub fn show(app: &mut SheetApp, ctx: &egui::Context) {\n    if app.dialog.is_none() && is_first_run("gridcraft") {\n        app.open_dialog("about", json!({}));\n    }\n    let Some(mut d) = app.dialog.take() else { return };'
    inject_first_run(show_rs, fr_target, fr_repl)

elif app in ("launcher", "artcraft-launcher"):
    settings_dialog_rs = src / "crates" / "ui-egui" / "src" / "settings_dialog.rs"
    target = "    fn settings_contents(&mut self, ui: &mut Ui, host: &mut dyn Host, wide: bool) {\n"
    replacement = f"""    fn settings_disclaimer(&self, ui: &mut Ui) {{
        {code_block}
    }}

    fn settings_contents(&mut self, ui: &mut Ui, host: &mut dyn Host, wide: bool) {{
        self.settings_disclaimer(ui);
"""
    patched = patch_file(settings_dialog_rs, target, replacement)

    dialogs_rs = src / "crates" / "ui-egui" / "src" / "dialogs.rs"
    fr_target = "    pub(crate) fn dialogs(&mut self, ctx: &egui::Context, host: &mut dyn Host) {\n"
    fr_repl = '    pub(crate) fn dialogs(&mut self, ctx: &egui::Context, host: &mut dyn Host) {\n        if self.dialog.is_none() && is_first_run("launcher") {\n            self.dialog = Some(Dialog::Settings);\n        }\n'
    inject_first_run(dialogs_rs, fr_target, fr_repl)

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

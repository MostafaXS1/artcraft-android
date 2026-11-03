import sys
from pathlib import Path

def patch_line(file_path, after_line, insertion):
    path = Path(file_path)
    lines = path.read_text().splitlines()
    new_lines = []
    patched = False
    
    for line in lines:
        new_lines.append(line)
        if after_line in line and not patched:
            new_lines.append(insertion)
            patched = True
            
    path.write_text("\n".join(new_lines) + "\n")
    print(f"Patched {file_path}")

src = Path(sys.argv[1])

# lib.rs
# Find: `pub struct WordApp {`
# Insert after `pub struct WordApp {`
patch_line(src / "crates/ui-egui/src/lib.rs", "pub struct WordApp {", "    pub request_ime_hide: bool,")

# app.rs: WordApp::new initializer (inside impl WordApp, find WordApp {)
# This is tricky because there are many `WordApp {`
# I'll look for `comment_drafts: Default::default(),`
patch_line(src / "crates/ui-egui/src/app.rs", "comment_drafts: Default::default(),", "            request_ime_hide: false,")

# window_ui.rs: struct
patch_line(src / "crates/ui-egui/src/window_ui.rs", "pub(crate) struct WindowUi {", "    pub(crate) request_ime_hide: bool,")

# window_ui.rs: new()
patch_line(src / "crates/ui-egui/src/window_ui.rs", "shown_document: document,", "            request_ime_hide: false,")

# window_ui.rs: swap_fields
# I need to match the macro call, which is unique.
# It starts with `swap_fields! {`
patch_line(src / "crates/ui-egui/src/window_ui.rs", "shown_document,", "            request_ime_hide,")

# app.rs: WindowUi pattern match
# Look for `alt_modifier_seen,`
patch_line(src / "crates/ui-egui/src/app.rs", "alt_modifier_seen,", "            request_ime_hide,")

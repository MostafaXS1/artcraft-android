import sys
from pathlib import Path

def patch(file_path, target_line_substr, insertion, check_string):
    path = Path(file_path)
    lines = path.read_text().splitlines()
    
    if any(check_string in line for line in lines):
        print(f"Skipping {file_path}: {check_string} already present")
        return
        
    for i, line in enumerate(lines):
        if target_line_substr in line:
            # Insert after this line
            lines.insert(i + 1, insertion)
            path.write_text("\n".join(lines) + "\n")
            print(f"Patched {file_path}")
            return
            
    print(f"Error: {target_line_substr} not found in {file_path}")
    sys.exit(1)

src = Path(sys.argv[1])

# lib.rs: Field in WordApp struct (find the struct definition start)
patch(src / "crates/ui-egui/src/lib.rs",
      "pub struct WordApp {",
      "    pub request_ime_hide: bool,",
      "    pub request_ime_hide: bool")

# app.rs: WordApp::new field init (find WordApp {)
# This one is tricky, it's inside `pub fn new`.
# Let's find the `WordApp {` inside `new`.
path = src / "crates/ui-egui/src/app.rs"
lines = path.read_text().splitlines()
patched = False
for i, line in enumerate(lines):
    if "WordApp {" in line and "let window_ui" not in line: # Avoid the WindowUi init
        lines.insert(i + 1, "            request_ime_hide: false,")
        path.write_text("\n".join(lines) + "\n")
        print(f"Patched app.rs")
        patched = True
        break
if not patched:
    print("Error: Could not patch app.rs initializer")
    sys.exit(1)

# window_ui.rs: Struct field
patch(src / "crates/ui-egui/src/window_ui.rs",
      "pub(crate) struct WindowUi {",
      "    pub(crate) request_ime_hide: bool,",
      "request_ime_hide: bool")

# window_ui.rs: new() init
# This one is tricky again, WindowUi::new
# I'll just look for `shown_document: document,`
patch(src / "crates/ui-egui/src/window_ui.rs",
      "            shown_document: document,",
      "            request_ime_hide: false,",
      "request_ime_hide: false")

# window_ui.rs: swap_fields! macro
# Find `shown_document,`
patch(src / "crates/ui-egui/src/window_ui.rs",
      "            shown_document,",
      "            request_ime_hide,",
      "request_ime_hide,")

# app.rs: WindowUi pattern match
# Find `alt_modifier_seen,`
patch(src / "crates/ui-egui/src/app.rs",
      "            alt_modifier_seen,",
      "            request_ime_hide,",
      "request_ime_hide,")

import sys
from pathlib import Path

# Use explicit check to ensure we only apply if the field is NOT present
def add_field(file_path, search_pattern, new_line, check_string):
    path = Path(file_path)
    content = path.read_text()
    if check_string in content:
        print(f"Skipping {file_path}: {check_string} already present")
        return
    
    if search_pattern not in content:
        print(f"Error: {search_pattern} not found in {file_path}")
        return
        
    content = content.replace(search_pattern, search_pattern + "\n" + new_line)
    path.write_text(content)
    print(f"Patched {file_path}")

src = Path(sys.argv[1])

# 1. lib.rs (Struct and Initializer)
add_field(src / "crates/ui-egui/src/lib.rs",
          "    pub(crate) comment_drafts: panes::CommentDrafts,",
          "    pub request_ime_hide: bool,",
          "pub request_ime_hide: bool")

add_field(src / "crates/ui-egui/src/app.rs",
          "            comment_drafts: Default::default(),",
          "            request_ime_hide: false,",
          "request_ime_hide: false")

# 2. window_ui.rs
add_field(src / "crates/ui-egui/src/window_ui.rs",
          "    pub(crate) shown_document: u64,",
          "    pub(crate) request_ime_hide: bool,",
          "pub(crate) request_ime_hide: bool")

add_field(src / "crates/ui-egui/src/window_ui.rs",
          "            shown_document: document,",
          "            request_ime_hide: false,",
          "request_ime_hide: false")

add_field(src / "crates/ui-egui/src/window_ui.rs",
          "            shown_document,",
          "            request_ime_hide,",
          "request_ime_hide,")

# 3. app.rs - WindowUi pattern match
# This is tricky, let's just find the closing brace of the pattern match 
# and add `request_ime_hide,`
app_rs = src / "crates/ui-egui/src/app.rs"
content = app_rs.read_text()
if "request_ime_hide" not in content and "alt_modifier_seen," in content:
    content = content.replace("            alt_modifier_seen,", "            alt_modifier_seen,\n            request_ime_hide,")
    app_rs.write_text(content)
    print("Patched app.rs WindowUi pattern match")


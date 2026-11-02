import sys
from pathlib import Path

def patch(file_path, find_pattern, new_lines):
    path = Path(file_path)
    content = path.read_text()
    
    # If the new lines are already there, do nothing.
    if all(line.strip() in content for line in new_lines.split('\n') if line.strip()):
        print(f"Already patched {file_path}")
        return

    # Use a unique anchor.
    if find_pattern not in content:
        print(f"Error: {find_pattern} not found in {file_path}")
        sys.exit(1)
        
    # Inject.
    new_content = content.replace(find_pattern, find_pattern + "\n" + new_lines)
    path.write_text(new_content)
    print(f"Patched {file_path}")

src = Path(sys.argv[1])

# lib.rs: Field in WordApp struct
patch(src / "crates/ui-egui/src/lib.rs",
      "    pub(crate) comment_drafts: panes::CommentDrafts,",
      "    pub request_ime_hide: bool,")

# app.rs: WordApp::new field init
patch(src / "crates/ui-egui/src/app.rs",
      "            comment_drafts: Default::default(),",
      "            request_ime_hide: false,")

# window_ui.rs: Struct field
patch(src / "crates/ui-egui/src/window_ui.rs",
      "    pub(crate) shown_document: u64,",
      "    pub(crate) request_ime_hide: bool,")

# window_ui.rs: new() init
patch(src / "crates/ui-egui/src/window_ui.rs",
      "            shown_document: document,",
      "            request_ime_hide: false,")

# window_ui.rs: swap_fields!
patch(src / "crates/ui-egui/src/window_ui.rs",
      "            shown_document,",
      "            request_ime_hide,")

# app.rs: WindowUi pattern match
# This one I will use a more robust replacement.
app_rs = src / "crates/ui-egui/src/app.rs"
content = app_rs.read_text()
if "request_ime_hide" not in content:
    content = content.replace("            alt_modifier_seen,", "            alt_modifier_seen,\n            request_ime_hide,")
    app_rs.write_text(content)
    print("Patched app.rs pattern match")


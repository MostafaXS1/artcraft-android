import sys
from pathlib import Path

def apply_patch(file_path, find_pattern, new_lines):
    path = Path(file_path)
    content = path.read_text()
    
    # If already patched, skip
    if new_lines.strip() in content:
        print(f"Skipping {file_path}: already patched.")
        return
        
    if find_pattern not in content:
        print(f"Error: pattern not found in {file_path}")
        sys.exit(1)
        
    # Replace the *first* occurrence
    new_content = content.replace(find_pattern, find_pattern + "\n" + new_lines, 1)
    path.write_text(new_content)
    print(f"Patched {file_path}")

src = Path(sys.argv[1])

# 1. lib.rs - Struct field
apply_patch(src / "crates/ui-egui/src/lib.rs",
            "    pub(crate) comment_drafts: panes::CommentDrafts,",
            "    pub request_ime_hide: bool,")

# 2. window_ui.rs - Struct field
apply_patch(src / "crates/ui-egui/src/window_ui.rs",
            "    pub(crate) shown_document: u64,",
            "    pub(crate) request_ime_hide: bool,")

# 3. window_ui.rs - Initializer
apply_patch(src / "crates/ui-egui/src/window_ui.rs",
            "            shown_document: document,",
            "            request_ime_hide: false,")

# 4. window_ui.rs - swap_fields macro - This macro is tricky. 
# The build error said: `pattern requires .. due to inaccessible fields`.
# This is for the `let WindowUi { ... } = &*$w;` line inside the macro.
# I must add `..` to this pattern.
path = src / "crates/ui-egui/src/window_ui.rs"
content = path.read_text()
# Anchor: let WindowUi { $($(#[$attr])* $field: _),* } = &*$w;
content = content.replace("let WindowUi { $($(#[$attr])* $field: _),* } = &*$w;", 
                          "let WindowUi { $($(#[$attr])* $field: _),* , .. } = &*$w;")
path.write_text(content)
print("Patched window_ui.rs macro")

# 5. app.rs - WordApp initializer
apply_patch(src / "crates/ui-egui/src/app.rs",
            "            printing: Default::default(),",
            "            request_ime_hide: false,")

# 6. app.rs - WindowUi pattern match (WordApp::new)
# This one needs `..` as well.
# Pattern: let window_ui::WindowUi { ... } = ...
content = path.read_text() # reload
# Actually, I should use re for this to be safe, but simple replace might work if unique.
content = content.replace("            alt_modifier_seen,\n        } = window_ui::WindowUi::new(session.document_id());",
                          "            alt_modifier_seen,\n            request_ime_hide,\n        } = window_ui::WindowUi::new(session.document_id());")
(src / "crates/ui-egui/src/app.rs").write_text(content)                
print("Patched app.rs pattern")


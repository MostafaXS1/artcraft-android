from pathlib import Path
import sys

def patch_file(file_path, find_pattern, new_lines):
    path = Path(file_path)
    content = path.read_text()
    
    # Idempotency: if new_lines already in file, skip
    if new_lines.strip() in content:
        print(f"Already patched {file_path}")
        return
        
    # Inject
    if find_pattern not in content:
        print(f"Error: {find_pattern} not found in {file_path}")
        sys.exit(1)
        
    new_content = content.replace(find_pattern, find_pattern + "\n" + new_lines)
    path.write_text(new_content)
    print(f"Patched {file_path}")

src = Path(sys.argv[1])

# 1. Add request_ime_hide to WordApp struct definition in lib.rs
patch_file(src / "crates/ui-egui/src/lib.rs",
           "    pub(crate) comment_drafts: panes::CommentDrafts,",
           "    pub request_ime_hide: bool,")

# 2. Add request_ime_hide to WordApp initializer in app.rs
# Anchor: printing: Default::default(),
patch_file(src / "crates/ui-egui/src/app.rs",
           "            printing: Default::default(),",
           "            request_ime_hide: false,")

# 3. Add request_ime_hide to WindowUi struct in window_ui.rs
patch_file(src / "crates/ui-egui/src/window_ui.rs",
           "    pub(crate) shown_document: u64,",
           "    pub(crate) request_ime_hide: bool,")

# 4. Add request_ime_hide to WindowUi initializer in window_ui.rs
patch_file(src / "crates/ui-egui/src/window_ui.rs",
           "            shown_document: document,",
           "            request_ime_hide: false,")


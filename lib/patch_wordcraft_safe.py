import sys
from pathlib import Path

def patch_file(file_path, old_line, new_line, idempotent_check=None):
    path = Path(file_path)
    if not path.exists(): return
    content = path.read_text()
    
    if idempotent_check and idempotent_check in content:
        return
        
    if new_line in content:
        return
        
    if old_line not in content:
        return

    content = content.replace(old_line, new_line)
    path.write_text(content)
    print(f"Patched {file_path}")

src = Path(sys.argv[1])

# 1. lib.rs - Struct and Initializer
patch_file(src / "crates/ui-egui/src/lib.rs",
           "    pub(crate) comment_drafts: panes::CommentDrafts,",
           "    pub(crate) comment_drafts: panes::CommentDrafts,\n    pub request_ime_hide: bool,",
           idempotent_check="pub request_ime_hide: bool")

# 2. window_ui.rs - Struct, new, swap_fields
patch_file(src / "crates/ui-egui/src/window_ui.rs",
           "    pub(crate) shown_document: u64,",
           "    pub(crate) shown_document: u64,\n    pub(crate) request_ime_hide: bool,",
           idempotent_check="pub(crate) request_ime_hide: bool")

patch_file(src / "crates/ui-egui/src/window_ui.rs",
           "            shown_document: document,",
           "            shown_document: document,\n            request_ime_hide: false,",
           idempotent_check="request_ime_hide: false")

patch_file(src / "crates/ui-egui/src/window_ui.rs",
           "            shown_document,",
           "            shown_document,\n            request_ime_hide,",
           idempotent_check="request_ime_hide,")

# 3. app.rs - Initializer and Pattern Match
# The initializer in `app.rs` for `WordApp` has `comment_drafts: Default::default(),`
patch_file(src / "crates/ui-egui/src/app.rs",
           "            comment_drafts: Default::default(),",
           "            comment_drafts: Default::default(),\n            request_ime_hide: false,",
           idempotent_check="request_ime_hide: false")

# The pattern match in `app.rs` has `alt_modifier_seen,`
patch_file(src / "crates/ui-egui/src/app.rs",
           "            alt_modifier_seen,",
           "            alt_modifier_seen,\n            request_ime_hide,",
           idempotent_check="request_ime_hide")


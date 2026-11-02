import sys
from pathlib import Path

def patch_file(file_path, search_pattern, new_line, check_string):
    path = Path(file_path)
    if not path.exists(): return
    content = path.read_text()
    
    # Idempotency check: if the change is already there, skip.
    if check_string in content:
        return

    # Use a more targeted replacement: search for the specific line and append after it.
    if search_pattern not in content:
        # Some patterns might span multiple lines, let's use a simpler search if needed.
        # But for this case, the anchor should be unique.
        print(f"Pattern {search_pattern[:20]}... not found in {file_path}")
        return
    
    # Replace the first occurrence only
    new_content = content.replace(search_pattern, f"{search_pattern}\n{new_line}", 1)
    path.write_text(new_content)
    print(f"Patched {file_path}")

src = Path(sys.argv[1])

# --- Patches ---

# 1. lib.rs: Field in WordApp struct
patch_file(src / "crates/ui-egui/src/lib.rs",
           "    pub(crate) comment_drafts: panes::CommentDrafts,",
           "    pub request_ime_hide: bool,",
           "pub request_ime_hide: bool")

# 2. window_ui.rs: Struct field
patch_file(src / "crates/ui-egui/src/window_ui.rs",
           "    pub(crate) shown_document: u64,",
           "    pub(crate) request_ime_hide: bool,",
           "pub(crate) request_ime_hide: bool")

# 3. window_ui.rs: new() init
# This one needs to be very specific to not break the struct.
patch_file(src / "crates/ui-egui/src/window_ui.rs",
           "            shown_document: document,",
           "            request_ime_hide: false,",
           "request_ime_hide: false")

# 4. window_ui.rs: swap_fields! macro
# The macro looks like: swap_fields! { shown_document, ... }
# Look for `shown_document,`
patch_file(src / "crates/ui-egui/src/window_ui.rs",
           "            shown_document,",
           "            request_ime_hide,",
           "request_ime_hide,")

# 5. app.rs: WordApp::new initializer
# This is inside `WordApp {` which is at the end of `new`.
# Anchor on `comment_drafts: Default::default(),`
patch_file(src / "crates/ui-egui/src/app.rs",
           "            comment_drafts: Default::default(),",
           "            request_ime_hide: false,",
           "request_ime_hide: false")

# 6. app.rs: WindowUi pattern match
# Look for `alt_modifier_seen,`
patch_file(src / "crates/ui-egui/src/app.rs",
           "            alt_modifier_seen,",
           "            request_ime_hide,",
           "request_ime_hide,")

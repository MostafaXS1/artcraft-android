from pathlib import Path
import sys

def patch_wordcraft(src_dir):
    src = Path(src_dir)
    
    # 1. Add request_ime_hide to WordApp struct definition in lib.rs
    lib_rs = src / "crates/ui-egui/src/lib.rs"
    content = lib_rs.read_text()
    if "pub request_ime_hide: bool" not in content:
        content = content.replace("    pub(crate) comment_drafts: panes::CommentDrafts,", "    pub(crate) comment_drafts: panes::CommentDrafts,\n    pub request_ime_hide: bool,")
        lib_rs.write_text(content)
        
    # 2. Initialize request_ime_hide in WordApp::new in app.rs
    app_rs = src / "crates/ui-egui/src/app.rs"
    content = app_rs.read_text()
    # Check if init already exists. If not, add it.
    if "request_ime_hide: false," not in content:
        # Need to find the WordApp initializer.
        # It's usually `WordApp { ... }`.
        # Be careful not to match multiple times.
        if "comment_drafts: Default::default()," in content:
            content = content.replace("            comment_drafts: Default::default(),", "            comment_drafts: Default::default(),\n            request_ime_hide: false,")
            app_rs.write_text(content)
        
    # 3. Add request_ime_hide to WindowUi pattern match in window_ui.rs
    win_ui = src / "crates/ui-egui/src/window_ui.rs"
    content = win_ui.read_text()
    if "request_ime_hide" not in content:
        # This is tricky because of the multiple uses.
        # The build error suggested it's bound multiple times.
        # Let's see the file content first.
        pass

if __name__ == "__main__":
    patch_wordcraft(sys.argv[1])

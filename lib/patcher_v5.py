import sys
from pathlib import Path
import re

def patch_file_content(file_path, replacements):
    path = Path(file_path)
    content = path.read_text()
    for search, replace in replacements:
        if search not in content:
            print(f"Error: Pattern not found in {file_path}")
            continue
        content = content.replace(search, replace)
    path.write_text(content)
    print(f"Patched {file_path}")

src = Path(sys.argv[1])

# 1. lib.rs: Field in WordApp struct
patch_file_content(src / "crates/ui-egui/src/lib.rs",
    [("    pub(crate) comment_drafts: panes::CommentDrafts,",
      "    pub(crate) comment_drafts: panes::CommentDrafts,\n    pub request_ime_hide: bool,")])

# 2. window_ui.rs: Struct field
patch_file_content(src / "crates/ui-egui/src/window_ui.rs",
    [("    pub(crate) shown_document: u64,",
      "    pub(crate) shown_document: u64,\n    pub(crate) request_ime_hide: bool,")])

# 3. window_ui.rs: new() init
patch_file_content(src / "crates/ui-egui/src/window_ui.rs",
    [("            shown_document: document,",
      "            shown_document: document,\n            request_ime_hide: false,")])

# 4. window_ui.rs: swap_fields! macro (add ..)
# Macro definition is: macro_rules! swap_fields { ($w:ident, $($field:ident),* $(,)?) => { ... } }
# This is hard to patch without breaking.
# Wait, if I just add `request_ime_hide` to the *call*, the macro definition MUST support extra fields.
# If I can't change the macro, I have to ensure the macro call is ignored.
# Actually, the error `E0027` suggests I need `..`.

# Let's try patching app.rs's pattern match first.

# 5. app.rs: Initialize request_ime_hide
content = (src / "crates/ui-egui/src/app.rs").read_text()
# Find WordApp { ... } initializer
# It seems there is only one WordApp { ... } in new()
# Use a more specific regex.
match = re.search(r"WordApp \{.*?session,", content, re.DOTALL)
if match:
    # This is still dangerous.
    pass

# Let's try a simpler approach for the initializer: 
# Just replace `printing: Default::default(),` with `printing: Default::default(),\n            request_ime_hide: false,`
# This should be safe enough if it's unique.
content = content.replace("            printing: Default::default(),", 
                          "            printing: Default::default(),\n            request_ime_hide: false,")
(src / "crates/ui-egui/src/app.rs").write_text(content)

# 6. For pattern matches, I'll use `re.sub` to add `..`
content = content.replace("        let window_ui::WindowUi {", 
                          "        let window_ui::WindowUi { ..")
# This will break the existing destructuring. This is why it's hard.
# If I add `..`, I must remove all other field names in that block.


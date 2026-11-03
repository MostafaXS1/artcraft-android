from pathlib import Path

# This is a bit complex for a simple script, but I'll try to find the structure 
# "inbox: Some(inbox)," or similar and inject the system_theme block.

build_common = Path("/home/mostafa/artcraft-android-builder/lib/build-common.sh")
content = build_common.read_text()

theme_block = """
        system_theme: Some(Box::new(move |_ctx: &egui::Context| {
            system_theme::read_android_system_theme(&bridge.app)
        })),"""

# DeckCraft (1241)
content = content.replace("        inbox: Some(inbox),", "        inbox: Some(inbox)," + theme_block)

# GridCraft (1439) - it doesn't have an `inbox` field but ends with `..Services::default()`
# Wait, gridcraft has `printers` and `print_native`
# Let's check GridCraft again. It ends with `print_native` block and then...
# Actually, I'll just look for `..Services::default()` in those specific blocks.

# For gridcraft, the block ends with the print_native closure.
# For wordcraft, the block ends with the printer closure.

# This is getting dangerous. I'll just use the edit tool with very specific context for each.

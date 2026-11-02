from pathlib import Path
import sys
import re

src = Path(sys.argv[1])
app = sys.argv[2]

services_file = src / "crates/ui-egui/src/services.rs"
if not services_file.exists():
    sys.exit(0)

content = services_file.read_text()
if "SystemThemeFn" in content:
    sys.exit(0)

# Ensure egui::Theme is imported
if "use egui::Theme;" not in content:
    content = "use egui::Theme;\n" + content

# Add SystemThemeFn type alias first, separately
content = "pub type SystemThemeFn = Box<dyn Fn(&egui::Context) -> Option<egui::Theme>>;\n\n" + content

# Now find the Services struct and add derive(Default) if not present
if "#[derive(Default)]" not in content:
    content = content.replace("pub struct Services {", "#[derive(Default)]\npub struct Services {")

# Add system_theme field to Services
if "pub system_theme: Option<SystemThemeFn>," not in content:
    content = content.replace(
        "    pub clipboard_text: Option<ClipboardFn>,",
        "    pub clipboard_text: Option<ClipboardFn>,\n    #[default]\n    pub system_theme: Option<SystemThemeFn>,"
    )

services_file.write_text(content)
print(f"Patched {services_file}")

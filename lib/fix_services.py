from pathlib import Path
import re

for app in ["designcraft", "wordcraft", "gridcraft", "deckcraft"]:
    services_file = Path(f"/home/mostafa/artcraft-android-builder/source/{app}/crates/ui-egui/src/services.rs")
    if not services_file.exists():
        continue
    
    content = services_file.read_text()
    
    # Remove all definitions of SystemThemeFn
    content = re.sub(r"pub type SystemThemeFn = Box<dyn Fn\(&egui::Context\) -> Option<egui::Theme>>;\n*", "", content)
    
    # Remove all "use egui::Theme;"
    content = content.replace("use egui::Theme;", "")
    
    # Add one definition at the top
    content = "use egui::Theme;\npub type SystemThemeFn = Box<dyn Fn(&egui::Context) -> Option<egui::Theme>>;\n\n" + content.lstrip()
    
    # Ensure system_theme field is in Services struct (only once)
    # The current content might have multiple system_theme: Option<SystemThemeFn>,
    # Let's clean that up too.
    lines = content.splitlines()
    new_lines = []
    seen_system_theme = False
    for line in lines:
        if "pub system_theme: Option<SystemThemeFn>," in line:
            if not seen_system_theme:
                new_lines.append(line)
                seen_system_theme = True
        else:
            new_lines.append(line)
    
    services_file.write_text("\n".join(new_lines))
    print(f"Fixed {services_file}")

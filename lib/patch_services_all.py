from pathlib import Path

for app in ["designcraft", "wordcraft", "gridcraft", "deckcraft"]:
    services_file = Path(f"/home/mostafa/artcraft-android-builder/source/{app}/crates/ui-egui/src/services.rs")
    if not services_file.exists():
        continue
    content = services_file.read_text()
    if "system_theme: Option<SystemThemeFn>" in content:
        continue
    
    # Ensure SystemThemeFn type exists
    if "type SystemThemeFn =" not in content:
        content = content.replace("use ", "use egui::Theme;\npub type SystemThemeFn = Box<dyn Fn(&egui::Context) -> Option<egui::Theme>>;\n\nuse ")
        if "use egui::Theme;" not in content: # Fallback if no 'use ' found
            content = "use egui::Theme;\npub type SystemThemeFn = Box<dyn Fn(&egui::Context) -> Option<egui::Theme>>;\n\n" + content
    
    # Add field to Services
    if "pub system_theme" not in content:
        content = content.replace("pub struct Services {", "pub struct Services {\n    pub system_theme: Option<SystemThemeFn>,")
    
    services_file.write_text(content)
    print(f"Patched {services_file}")

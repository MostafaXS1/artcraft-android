from pathlib import Path
import re

for app in ["designcraft", "wordcraft", "gridcraft", "deckcraft"]:
    services_file = Path(f"/home/mostafa/artcraft-android-builder/source/{app}/crates/ui-egui/src/services.rs")
    if not services_file.exists():
        continue
    
    content = services_file.read_text()
    
    # Repair the specific corruption: "beca" + "use a paste only lands in the document when the page is the edit" + "target (#1349), where a dropped or opened file always lands."
    # The fix should be to join "beca" and "use" into "because", etc.
    
    # Re-apply the patch in a safer way.
    # 1. Clean everything.
    
    # 2. Add SystemThemeFn and field correctly.
    
    # ... This is too risky with a script again.
    # I'll just restore from a backup if I had one. 
    # Since I don't have a backup, I have to fix it manually.
    
    # Actually, I can use `git checkout` to restore the file if this is a git repo.
    # The environment says `Is directory a git repo: yes`.
    
    # Let me try to use git checkout to restore services.rs for each app.
    import subprocess
    subprocess.run(["git", "checkout", str(services_file)], cwd=services_file.parent)
    print(f"Restored {services_file}")

//! Settings persistence for Android (JSON-based)

use artcraft_launcher_ui_egui::Settings;
use std::path::Path;

/// Load settings from a JSON file
pub fn load(path: &Path) -> Option<Settings> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Save settings to a JSON file
pub fn save(path: &Path, value: &Settings) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| e.to_string())
}

//! Android paths: app-specific storage (no external storage permissions needed).

#[cfg(target_os = "android")]
use android_activity::AndroidApp;
#[cfg(target_os = "android")]
use std::path::PathBuf;

/// Returns the app's internal files directory: `/data/user/0/ai.storyteller.artcraft-launcher/files`
#[cfg(target_os = "android")]
pub fn android_files_dir(android: &AndroidApp) -> Result<PathBuf, String> {
    android.internal_data_path().ok_or_else(|| "Android internal data path not available".into())
}

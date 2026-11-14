//! Bridge between Rust and Android (Java).
//!
//! Uses a file-based protocol in the app's private storage to avoid JNI/unsafe:
//! - Request file: `bridge/request.json` — Rust writes, Java reads and deletes
//! - Response file: `bridge/answer.json` — Java writes atomically, Rust reads and deletes
//!
//! Java polls the request file while the Activity is resumed (every 150 ms), so every call here
//! waits for its answer. One request is outstanding at a time: a second one written before Java
//! read the first would replace it.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use log::{debug, info, warn};
use serde::{Deserialize, Serialize};

/// Bridge for async calls to Java via the file protocol.
#[derive(Clone)]
pub struct Bridge {
    base: PathBuf,
    next_id: Arc<AtomicU64>,
    /// Held from writing a request until its answer is read, so Java never sees a request
    /// overwritten before it read it.
    lock: Arc<Mutex<()>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum RequestKind {
    GetInstalledVersions { packages: Vec<String> },
    Uninstall { package: String },
    OpenApp { package: String },
    AppInfo { package: String },
    InstallApk { package: String, apk_path: String },
    MoveToBackground,
}

/// What Java answered. Every field is optional: the answer to each request kind fills in a
/// different one.
#[derive(Debug, Default, Deserialize)]
struct Answer {
    #[serde(default)]
    versions: Option<HashMap<String, Option<String>>>,
    #[serde(default)]
    success: Option<bool>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Serialize, Debug)]
struct BridgeRequest {
    id: u64,
    #[serde(flatten)]
    kind: RequestKind,
}

/// One notification for the Activity to post (or update in place, when the id repeats).
#[derive(Serialize)]
struct Notification<'a> {
    id: &'a str,
    title: &'a str,
    text: &'a str,
    channel: &'a str,
    /// `0..=100`; omitted for a notification without a progress bar.
    #[serde(skip_serializing_if = "Option::is_none")]
    progress: Option<u8>,
    /// Keep it up until it is replaced or cancelled (downloads in flight).
    ongoing: bool,
    auto_cancel: bool,
}

impl Bridge {
    pub fn new(base: PathBuf) -> Self {
        // base is already the bridge directory (e.g. files/bridge)
        fs::create_dir_all(&base).ok();
        Self { base, next_id: Arc::new(AtomicU64::new(1)), lock: Arc::new(Mutex::new(())) }
    }

    /// Which of `packages` are installed, and at what `versionName` (`None` when installed but the
    /// version couldn't be read). One round trip for the whole suite.
    pub fn installed_versions(&self, packages: &[String]) -> Result<HashMap<String, Option<String>>, String> {
        self.ask(RequestKind::GetInstalledVersions { packages: packages.to_vec() }, Duration::from_secs(10))
            .ok_or_else(|| "the bridge didn't answer".to_string())?
            .versions
            .ok_or_else(|| "the bridge didn't answer with installed versions".to_string())
    }

    /// Installs `apk` through a PackageInstaller session. Answers only once the user has accepted
    /// or dismissed the system prompt, so the result is the real one.
    pub fn install_apk(&self, package: &str, apk: &Path, timeout: Duration) -> Result<(), String> {
        let answer = self
            .ask(RequestKind::InstallApk { package: package.to_string(), apk_path: apk.to_string_lossy().to_string() }, timeout)
            .ok_or_else(|| "the system installer didn't answer".to_string())?;
        match answer.success {
            Some(true) => Ok(()),
            Some(false) => Err(match answer.error.as_deref() {
                // The user has to allow installs from this app first; Java has opened that page.
                Some("permission") => "Allow ArtCraft Launcher to install apps, then try again.".into(),
                Some("aborted") => artcraft_launcher_core::protocol::INSTALL_CANCELLED.into(),
                Some(why) => why.to_string(),
                None => "the install failed".into(),
            }),
            None => Err("the system installer didn't answer".into()),
        }
    }

    /// Opens the system uninstall prompt. Answers only once the user has accepted or dismissed it,
    /// so the result reflects what actually happened: `true` means the app is gone.
    pub fn uninstall(&self, package: &str, timeout: Duration) -> Result<bool, String> {
        let answer = self
            .ask(RequestKind::Uninstall { package: package.to_string() }, timeout)
            .ok_or_else(|| "the system uninstaller didn't answer".to_string())?;
        match answer.success {
            Some(removed) => Ok(removed),
            None => Err("the system uninstaller didn't answer".to_string()),
        }
    }

    /// Opens an installed app; `false` when it has no launchable activity or isn't installed.
    pub fn open_app(&self, package: &str) -> bool {
        self.ask(RequestKind::OpenApp { package: package.to_string() }, Duration::from_secs(5)).and_then(|answer| answer.success).unwrap_or(false)
    }

    /// Opens the system App info page for the package.
    pub fn app_info(&self, package: &str) {
        let _ = self.ask(RequestKind::AppInfo { package: package.to_string() }, Duration::from_secs(5));
    }

    /// Sends the task to the background, like pressing Home.
    pub fn move_to_background(&self) {
        let _ = self.ask(RequestKind::MoveToBackground, Duration::from_secs(5));
    }

    /// Asks Java to show or hide the soft keyboard. This is a plain file the Activity polls, not
    /// a request: the bridge may be busy waiting minutes for an install prompt, and the keyboard
    /// must still follow focus.
    pub fn set_keyboard(&self, show: bool) {
        let path = self.base.join("keyboard");
        let tmp = self.base.join("keyboard.tmp");
        let text = if show { "show" } else { "hide" };
        if let Err(e) = fs::write(&tmp, text) {
            warn!("Bridge: couldn't write {}: {e}", tmp.display());
            return;
        }
        if let Err(e) = fs::rename(&tmp, &path) {
            warn!("Bridge: couldn't publish {}: {e}", path.display());
        }
    }

    /// Writes the desired theme ("light" or "dark") to `bridge/theme` so the Java side can apply it.
    pub fn set_theme(&self, dark: bool) {
        let path = self.base.join("theme");
        let tmp = self.base.join("theme.tmp");
        let text = if dark { "dark" } else { "light" };
        if let Err(e) = fs::write(&tmp, text) {
            warn!("Bridge: couldn't write {}: {e}", tmp.display());
            return;
        }
        if let Err(e) = fs::rename(&tmp, &path) {
            warn!("Bridge: couldn't publish {}: {e}", path.display());
        }
    }

    /// Posts or updates an Android notification. Java polls `bridge/notifications/` and reads
    /// one JSON file per notification; a repeated `id` updates the same notification.
    ///
    /// `progress` is a percentage, or `None` for a notification without a progress bar.
    pub fn notify(&self, id: &str, title: &str, text: &str, channel: &str, progress: Option<u8>, ongoing: bool) {
        let notification = Notification { id, title, text, channel, progress: progress.map(|p| p.min(100)), ongoing, auto_cancel: !ongoing };
        self.publish_notification(id, &notification);
    }

    /// Removes a notification previously posted with [`Self::notify`].
    pub fn cancel_notification(&self, id: &str) {
        let notification = serde_json::json!({ "id": id, "cancel": true });
        self.publish_notification(id, &notification);
    }

    fn publish_notification(&self, id: &str, value: &impl Serialize) {
        let dir = self.base.join("notifications");
        if let Err(e) = fs::create_dir_all(&dir) {
            warn!("Bridge: couldn't create {}: {e}", dir.display());
            return;
        }
        let name = notification_file_name(id);
        let path = dir.join(&name);
        let tmp = dir.join(format!("{name}.tmp"));
        match serde_json::to_vec(value) {
            Ok(json) => {
                if let Err(e) = fs::write(&tmp, json) {
                    warn!("Bridge: couldn't write {}: {e}", tmp.display());
                    return;
                }
                if let Err(e) = fs::rename(&tmp, &path) {
                    warn!("Bridge: couldn't publish {}: {e}", path.display());
                }
            }
            Err(e) => warn!("Bridge: couldn't encode a notification: {e}"),
        }
    }

    /// The generation Java last wrote when the Activity's configuration changed (the system
    /// light/dark switch). The worker compares it and wakes egui so the next frame re-reads the
    /// configuration; without this a theme switch with no other input would wait for a touch.
    pub fn system_theme_generation(&self) -> Option<String> {
        fs::read_to_string(self.base.join("system_theme")).ok().map(|text| text.trim().to_string())
    }

    fn ask(&self, kind: RequestKind, timeout: Duration) -> Option<Answer> {
        let _guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = BridgeRequest { id, kind };
        self.write_request(&request);
        self.wait_for_response(id, timeout)
    }

    fn write_request(&self, request: &BridgeRequest) {
        let path = self.base.join("request.json");
        debug!("Bridge: writing request {request:?}");
        match serde_json::to_string(request) {
            Ok(json) => {
                if let Err(e) = fs::write(&path, json) {
                    warn!("Bridge: couldn't write {}: {e}", path.display());
                }
            }
            Err(e) => warn!("Bridge: couldn't encode request: {e}"),
        }
    }

    fn wait_for_response(&self, expected_id: u64, timeout: Duration) -> Option<Answer> {
        let start = Instant::now();
        let response_path = self.base.join("answer.json");
        while start.elapsed() < timeout {
            if let Ok(content) = fs::read_to_string(&response_path) {
                let _ = fs::remove_file(&response_path);
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) {
                    if value.get("id").and_then(|v| v.as_u64()) == Some(expected_id) {
                        return serde_json::from_value(value).ok();
                    }
                    warn!("Bridge: answer id {:?} isn't the expected {expected_id}", value.get("id"));
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        info!("Bridge: request {expected_id} timed out");
        None
    }
}

/// A file name Java can read for a notification id: ASCII letters, digits, `-` and `_`, capped.
fn notification_file_name(id: &str) -> String {
    let mut name = String::with_capacity(id.len() + 5);
    for c in id.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
            name.push(c);
        } else {
            name.push('_');
        }
    }
    if name.is_empty() {
        name.push_str("note");
    }
    name.truncate(96);
    name.push_str(".json");
    name
}

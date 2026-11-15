//! Android backend: finds every app's latest Android build in the builder's single GitHub
//! release, downloads the chosen APK over HTTPS (verified against the digest GitHub publishes),
//! and hands it to the system installer through the Java bridge.
//!
//! All work happens on one worker thread so a 150 MB download never stalls a frame; the UI sends
//! [`Command`]s and drains [`Event`]s.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use artcraft_launcher_core::protocol::{
    Backend, CANCELLED, Capabilities, Command, Event, INSTALL_CANCELLED, JobPhase, LatestRelease, Origin, PAUSED, PausedDownload, Source,
};
use artcraft_launcher_core::release::{self, Asset, Release};
use artcraft_launcher_core::status::{InstallSource, Installed};
use artcraft_launcher_core::version;
use artcraft_launcher_core::{Arch, Os, Platform};
use artcraft_launcher_native::http::{self, Fetched, Http, Stop, Transfer};
use artcraft_launcher_native::paths::Paths;
use artcraft_launcher_native::screenshots;

use crate::bridge::Bridge;
use crate::catalog::{self, SUITE_ANDROID};

/// The platform the launcher runs on.
const PLATFORM: Platform = Platform { os: Os::Android, arch: Arch::Arm64 };

/// Android can install, update, remove and open apps; "reveal" opens the system App info page.
const CAPABILITIES: Capabilities = Capabilities { manage_apps: true, reveal_label: "App info", self_update: false };

/// The exit the system installer needs for the user to read the confirmation prompt.
const INSTALL_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Work handed from the UI thread to the worker.
enum Job {
    Detect,
    Check { force: bool },
    Install(String),
    Uninstall(String),
}

/// The answer to "what Android builds are published?".
enum Outcome {
    Release(Release),
    /// The repository has no published release.
    NoRelease,
    /// Couldn't find out (offline, rate-limited, unreadable answer).
    Failed(String),
}

struct Answer {
    outcome: Outcome,
    source: Source,
    /// The API refused until this time, even if the release pages answered instead.
    api_limited_until: Option<i64>,
    /// Shown in the UI when no source could fetch anything.
    rate_limited_until: Option<i64>,
}

/// State shared between the UI thread and the worker.
struct Shared {
    events: Mutex<Vec<Event>>,
    waker: crate::app::Waker,
    /// The per-app release the install command uses, as last shown to the UI.
    latest: Mutex<HashMap<String, Release>>,
    installed: Mutex<HashMap<String, Installed>>,
    /// Pause/cancel flags of the running download, per app.
    stops: Mutex<HashMap<String, Arc<Stop>>>,
    files_dir: PathBuf,
    /// Posts Android notifications (download progress, available updates).
    bridge: Bridge,
    /// Releases whose update notification has already been posted, newest last. Persisted so a
    /// check every twelve minutes doesn't repeat it.
    notified_updates: Mutex<Vec<String>>,
    /// Detection is due (after an install, an uninstall or a focus change).
    redetect: AtomicBool,
    /// GitHub API rate limit reset time (Unix seconds), if we've been rate limited.
    rate_limited_until: Mutex<Option<i64>>,
}

impl Shared {
    fn emit(&self, event: Event) {
        self.emit_all([event]);
    }

    fn emit_all(&self, events: impl IntoIterator<Item = Event>) {
        self.events.lock().unwrap_or_else(PoisonError::into_inner).extend(events);
        self.waker.wake();
    }

    /// Posts one update notification for a release, no matter how many of its apps have a newer
    /// build installed. A release is announced once: the key (`repo@tag`) is persisted, so the
    /// twelve-minute check doesn't repeat it.
    fn notify_updates(&self, repo: &str, tag: &str, version: &str, names: &[&str]) {
        if names.is_empty() {
            return;
        }
        let key = format!("{repo}@{tag}");
        {
            let mut keys = self.notified_updates.lock().unwrap_or_else(PoisonError::into_inner);
            if keys.iter().any(|known| known == &key) {
                return;
            }
            keys.push(key.clone());
            let excess = keys.len().saturating_sub(MAX_NOTIFIED_UPDATES);
            if excess > 0 {
                keys.drain(0..excess);
            }
            save_notified_updates(&self.files_dir, &keys);
        }
        let title =
            if names.len() == 1 { format!("{} {} is available", names[0], version) } else { format!("{} updates are available", names.len()) };
        let text = update_summary(names);
        self.bridge.notify(&format!("update-{key}"), &title, &text, "updates", None, false);
    }
}

/// How many release keys the notification history keeps (older releases are never announced
/// again anyway once installed).
const MAX_NOTIFIED_UPDATES: usize = 200;

fn load_notified_updates(files_dir: &std::path::Path) -> Vec<String> {
    let path = files_dir.join("notified-updates.json");
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    let mut keys: Vec<String> = serde_json::from_str(&text).unwrap_or_default();
    let excess = keys.len().saturating_sub(MAX_NOTIFIED_UPDATES);
    if excess > 0 {
        keys.drain(0..excess);
    }
    keys
}

fn save_notified_updates(files_dir: &std::path::Path, keys: &[String]) {
    let path = files_dir.join("notified-updates.json");
    let tmp = files_dir.join("notified-updates.json.tmp");
    match serde_json::to_vec(keys) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&tmp, json) {
                log::warn!("couldn't save {}: {e}", tmp.display());
                return;
            }
            if let Err(e) = std::fs::rename(&tmp, &path) {
                log::warn!("couldn't publish {}: {e}", path.display());
            }
        }
        Err(e) => log::warn!("couldn't encode the notification history: {e}"),
    }
}

/// A notification body that names a few apps and counts the rest.
fn update_summary(names: &[&str]) -> String {
    match names {
        [] => "Open ArtCraft Launcher to see the new version.".to_string(),
        [only] => format!("Open ArtCraft Launcher to update {only}."),
        [a, b] => format!("{a} and {b} are ready to update."),
        [a, b, c] => format!("{a}, {b} and {c} are ready to update."),
        [a, b, rest @ ..] => format!("{a}, {b} and {} more are ready to update.", rest.len()),
    }
}

/// The Android backend: shared state, the bridge to Java, and the worker that does the network and
/// installer work.
pub struct AndroidBackend {
    shared: Arc<Shared>,
    bridge: Bridge,
    http: Option<Http>,
    jobs: Sender<Job>,
    worker: Option<thread::JoinHandle<()>>,
}

impl AndroidBackend {
    pub fn new(bridge: Bridge, files_dir: PathBuf, waker: crate::app::Waker) -> Self {
        let http = match Http::new() {
            Ok(http) => Some(http),
            Err(e) => {
                log::error!("HTTPS isn't available: {e}");
                None
            }
        };
        let (jobs, _) = mpsc::channel();
        let notified_updates = load_notified_updates(&files_dir);
        Self {
            shared: Arc::new(Shared {
                events: Mutex::new(Vec::new()),
                waker,
                latest: Mutex::new(HashMap::new()),
                installed: Mutex::new(HashMap::new()),
                stops: Mutex::new(HashMap::new()),
                files_dir,
                bridge: bridge.clone(),
                notified_updates: Mutex::new(notified_updates),
                redetect: AtomicBool::new(false),
                rate_limited_until: Mutex::new(None),
            }),
            bridge,
            http,
            jobs,
            worker: None,
        }
    }

    /// Starts the worker thread. The UI keeps polling [`Backend::poll`] for its events.
    pub fn start(&mut self) {
        let (jobs, work) = mpsc::channel();
        self.jobs = jobs;
        let shared = self.shared.clone();
        let bridge = self.bridge.clone();
        let http = self.http.clone();
        let files_dir = self.shared.files_dir.clone();
        self.worker = Some(thread::spawn(move || worker_main(shared, bridge, http, files_dir, work)));
    }

    /// The path a release's APK is downloaded to (finished or partial).
    fn download_path(files_dir: &std::path::Path, asset: &Asset) -> PathBuf {
        Paths::under(files_dir).downloads().join(&asset.name)
    }
}

impl Backend for AndroidBackend {
    fn send(&mut self, command: Command) {
        match command {
            Command::CheckForUpdates { force } => {
                // The UI asks on open and every twelve minutes (`CheckEvery`); the worker only
                // checks when asked, so "Only when I ask" stays quiet.
                log::info!("AndroidBackend: sending CheckForUpdates force={}", force);
                let _ = self.jobs.send(Job::Check { force });
            }
            Command::Detect => {
                // Detection is a bridge round trip: let the worker do it, not the UI thread.
                let _ = self.jobs.send(Job::Detect);
            }
            Command::Install { app } => {
                let _ = self.jobs.send(Job::Install(app));
            }
            Command::Uninstall { app } => {
                let _ = self.jobs.send(Job::Uninstall(app));
            }
            Command::Launch { app, args: _ } => {
                let Some(def) = catalog::app_by_id(&app) else { return };
                let package = catalog::package_for(&app);
                let name = def.name;
                let bridge = self.bridge.clone();
                let shared = self.shared.clone();
                // Opening an app goes through the same bridge as installs; a few hundred
                // milliseconds off the UI thread.
                thread::spawn(move || {
                    if !bridge.open_app(&package) {
                        shared.emit(Event::JobFinished { app: app.clone(), result: Err(format!("Couldn't open {name}. Is it installed?")) });
                    }
                });
            }
            Command::Reveal { app } => {
                let package = catalog::package_for(&app);
                let bridge = self.bridge.clone();
                thread::spawn(move || bridge.app_info(&package));
            }
            Command::Pause { app } => {
                if let Some(stop) = self.shared.stops.lock().unwrap_or_else(PoisonError::into_inner).get(&app) {
                    stop.pause();
                }
            }
            Command::Cancel { app } => {
                let running = self.shared.stops.lock().unwrap_or_else(PoisonError::into_inner).get(&app).cloned();
                match running {
                    Some(stop) => stop.cancel(),
                    // No job: a paused or kept download. Throw the file away.
                    None => {
                        let dest =
                            self.shared.latest.lock().unwrap_or_else(PoisonError::into_inner).get(&app).and_then(|release| {
                                release.assets.first().map(|asset| (asset.size, Self::download_path(&self.shared.files_dir, asset)))
                            });
                        if let Some((size, dest)) = dest {
                            http::discard(&dest, size);
                        }
                        self.shared.emit(Event::Paused { app, download: None });
                    }
                }
            }
            Command::ListVersions { app } => {
                let known = catalog::app_by_id(&app).is_some();
                let release = self.shared.latest.lock().unwrap_or_else(PoisonError::into_inner).get(&app).cloned();
                let result = if !known {
                    Err(format!("{app} isn't part of the Android suite"))
                } else {
                    match release {
                        Some(release) => Ok(vec![release]),
                        None => Err("the release list hasn't been fetched yet".into()),
                    }
                };
                self.shared.emit(Event::Versions { app, result });
            }
            Command::FetchScreenshots { app } => {
                let Some(details) = artcraft_launcher_core::details::AppDetails::by_id(&app) else { return };
                let http = self.http.clone();
                let paths = Paths::under(&self.shared.files_dir);
                let shared = self.shared.clone();
                thread::spawn(move || {
                    screenshots::fetch_all(http.as_ref(), &paths, details, |key, result| {
                        shared.emit(Event::Screenshot { app: app.clone(), key: key.to_string(), result });
                    });
                });
            }
            // No folders to replace, no in-place self-update, no release feed options.
            Command::Replace { .. } | Command::UpdateLauncher | Command::Configure(_) => {}
        }
    }

    fn poll(&mut self) -> Vec<Event> {
        std::mem::take(&mut *self.shared.events.lock().unwrap_or_else(PoisonError::into_inner))
    }

    fn capabilities(&self) -> Capabilities {
        CAPABILITIES
    }

    fn platform(&self) -> Option<Platform> {
        Some(PLATFORM)
    }

    fn now(&self) -> i64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
    }
}

fn worker_main(shared: Arc<Shared>, bridge: Bridge, http: Option<Http>, files_dir: PathBuf, jobs: Receiver<Job>) -> ! {
    let paths = Paths::under(&files_dir);
    log::info!("worker started");
    // Detect first, so installed apps don't sit on "Checking" while the release lookup runs. The
    // bridge only answers once the Activity is polling, which can be after this thread starts;
    // an unanswered first call is retried below rather than being taken as "nothing installed".
    let mut detection_ready = detect(&shared, &bridge);
    if !detection_ready {
        log::info!("the bridge isn't answering yet; will retry detection");
    }
    // Java bumps this file on every configuration change (light/dark, resize); waking egui
    // makes the next frame re-read the Activity before any touch arrives.
    let mut theme_generation = bridge.system_theme_generation();
    loop {
        while let Ok(job) = jobs.try_recv() {
            match job {
                Job::Detect => detection_ready = detect(&shared, &bridge) || detection_ready,
                Job::Check { force } => {
                    log::info!("Worker: processing CheckForUpdates force={}", force);
                    check_for_updates(&shared, http.as_ref(), &paths, force);
                }
                Job::Install(app) => install(&shared, &bridge, http.as_ref(), &app),
                Job::Uninstall(app) => uninstall(shared.clone(), &bridge, &app),
            }
        }

        if shared.redetect.swap(false, Ordering::SeqCst) {
            detection_ready = detect(&shared, &bridge);
        }

        let generation = bridge.system_theme_generation();
        if generation != theme_generation {
            theme_generation = generation;
            shared.waker.wake();
        }

        // Keep trying until the Activity answers once, so installed apps never show Install just
        // because the first request arrived before Java started polling.
        if !detection_ready {
            detection_ready = detect(&shared, &bridge);
            if detection_ready {
                shared.waker.wake();
            }
        }

        thread::sleep(Duration::from_millis(250));
    }
}

/// Asks Java which of the suite's packages are installed and at what version, then tells the UI.
/// Returns whether the bridge answered; a timeout leaves the cards on "Checking" for a retry.
fn detect(shared: &Shared, bridge: &Bridge) -> bool {
    let packages: Vec<String> = SUITE_ANDROID.iter().map(|app| catalog::package_for(app.id)).collect();
    let found = match bridge.installed_versions(&packages) {
        Ok(found) => found,
        Err(e) => {
            log::warn!("couldn't detect installed apps: {e}");
            return false;
        }
    };
    let latest = shared.latest.lock().unwrap_or_else(PoisonError::into_inner).clone();
    let mut events = Vec::with_capacity(SUITE_ANDROID.len() * 2);
    {
        let mut installed_map = shared.installed.lock().unwrap_or_else(PoisonError::into_inner);
        installed_map.clear();
        for app in &SUITE_ANDROID {
            let package = catalog::package_for(app.id);
            let installed = found.get(&package).cloned().flatten().map(|raw| Installed {
                version: version::parse(&raw),
                source: InstallSource::Launcher,
                location: package.clone(),
                package: Some(package),
                externally_managed: false,
                origin: None,
                installation: None,
            });
            if let Some(copy) = &installed {
                installed_map.insert(app.id.to_string(), copy.clone());
            }
            events.push(Event::Detected { app: app.id.to_string(), installed: installed.clone() });
            // `Installable` only travels while a copy is installed: it says whether installing the
            // latest release over this copy would go ahead (an update).
            if let (Some(copy), Some(release)) = (&installed, latest.get(app.id))
                && let (Some(installed_version), Some(latest_version)) = (copy.version.as_ref(), release.version.as_ref())
            {
                events.push(Event::Installable { app: app.id.to_string(), installable: version::is_update(latest_version, installed_version) });
            }
        }
    }
    shared.emit_all(events);
    true
}

/// The cache stem for a repository's last good answer. The shared builder release keeps the
/// existing `android-release` name so cached answers survive the upgrade.
fn cache_key(repo: &str) -> String {
    if repo == catalog::ANDROID_REPO {
        "android-release".to_string()
    } else {
        format!("{}-release", repo.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect::<String>())
    }
}

/// Every repository the suite needs to ask about, in catalog order and without duplicates.
fn suite_repos() -> Vec<&'static str> {
    let mut repos = Vec::new();
    for app in &SUITE_ANDROID {
        let repo = catalog::release_source_for(app).repo;
        if !repos.contains(&repo) {
            repos.push(repo);
        }
    }
    repos
}

/// Asks one repository for its latest release.
fn ask_github(http: &Http, paths: &Paths, force: bool, repo: &str) -> Answer {
    let key = cache_key(repo);
    // "Check now" skips the ETag, so GitHub answers with the current release even when the saved
    // answer says nothing changed.
    let etag = if force {
        None
    } else {
        let path = paths.cache.join(format!("{key}.etag"));
        std::fs::read_to_string(&path).ok().map(|text| text.trim().to_string()).filter(|tag| !tag.is_empty())
    };
    let api_url = format!("https://api.github.com/repos/{repo}/releases/latest");
    match http.get_api(&api_url, etag.as_deref()) {
        Ok(Fetched::Body { text, etag }) => match release::parse_github_release(&text, repo) {
            Ok(release) => {
                save_api_release(paths, repo, &text, etag.as_deref());
                Answer { outcome: Outcome::Release(release), source: Source::GitHubApi, api_limited_until: None, rate_limited_until: None }
            }
            Err(e) => ask_pages(http, paths, repo, e, None),
        },
        Ok(Fetched::NotModified) => {
            if let Some(release) = saved_release(paths, repo) {
                return Answer { outcome: Outcome::Release(release), source: Source::Cache, api_limited_until: None, rate_limited_until: None };
            }
            // The saved answer vanished between the two calls: ask again from scratch.
            match http.get_api(&api_url, None) {
                Ok(Fetched::Body { text, etag }) => match release::parse_github_release(&text, repo) {
                    Ok(release) => {
                        save_api_release(paths, repo, &text, etag.as_deref());
                        Answer { outcome: Outcome::Release(release), source: Source::GitHubApi, api_limited_until: None, rate_limited_until: None }
                    }
                    Err(e) => ask_pages(http, paths, repo, e, None),
                },
                Ok(Fetched::NotFound) => {
                    Answer { outcome: Outcome::NoRelease, source: Source::GitHubApi, api_limited_until: None, rate_limited_until: None }
                }
                Ok(Fetched::RateLimited { reset }) => ask_pages(http, paths, repo, "GitHub's API is out of quota right now".into(), reset),
                Ok(_) => ask_pages(http, paths, repo, "GitHub's answer didn't include the release".into(), None),
                Err(e) => ask_pages(http, paths, repo, e, None),
            }
        }
        Ok(Fetched::NotFound) => Answer { outcome: Outcome::NoRelease, source: Source::GitHubApi, api_limited_until: None, rate_limited_until: None },
        Ok(Fetched::RateLimited { reset }) => ask_pages(http, paths, repo, "GitHub's API is out of quota right now".into(), reset),
        Err(e) => ask_pages(http, paths, repo, e, None),
    }
}

/// Asks github.com's release pages, which have no API quota. This is tried before showing a
/// repository as offline (and before the saved answer), so a spent API quota no longer hides
/// releases, sizes, notes or updates.
fn ask_pages(http: &Http, paths: &Paths, repo: &str, why: String, reset: Option<i64>) -> Answer {
    match fetch_pages_release(http, repo) {
        Ok(Some(release)) => {
            save_pages_release(paths, repo, &release);
            Answer { outcome: Outcome::Release(release), source: Source::GitHubPages, api_limited_until: reset, rate_limited_until: None }
        }
        Ok(None) => Answer { outcome: Outcome::NoRelease, source: Source::GitHubPages, api_limited_until: reset, rate_limited_until: None },
        Err(e) => {
            log::warn!("GitHub's release pages for {repo} failed: {e}");
            match saved_release(paths, repo) {
                Some(release) => {
                    Answer { outcome: Outcome::Release(release), source: Source::Cache, api_limited_until: reset, rate_limited_until: None }
                }
                None => Answer { outcome: Outcome::Failed(why), source: Source::GitHubApi, api_limited_until: reset, rate_limited_until: reset },
            }
        }
    }
}

/// Reads the latest release from `releases/latest` (the tag), `releases.atom` (name, date, notes)
/// and `releases/expanded_assets/<tag>` (files and SHA-256 digests), without touching the API.
fn fetch_pages_release(http: &Http, repo: &str) -> Result<Option<Release>, String> {
    let latest = format!("https://github.com/{repo}/releases/latest");
    let Some(location) = http.redirect_target(&latest)? else { return Ok(None) };
    let Some(tag) = release::tag_from_redirect(&location, repo) else {
        return Err("GitHub's release page led somewhere unexpected".into());
    };
    let atom = match http.get_text(&format!("https://github.com/{repo}/releases.atom")) {
        Ok(atom) => atom,
        Err(e) => {
            log::warn!("couldn't read the release feed for {repo}: {e}");
            None
        }
    };
    let assets_url = format!("https://github.com/{repo}/releases/expanded_assets/{tag}");
    let Some(assets) = http.get_text(&assets_url)? else { return Err("the release page didn't list its files".into()) };
    let release = release::release_from_pages(repo, &tag, atom.as_deref(), &assets);
    if release.assets.is_empty() {
        return Err("the release page listed no files".into());
    }
    Ok(Some(release))
}

/// Saves the API's answer and its ETag for the next check.
fn save_api_release(paths: &Paths, repo: &str, text: &str, etag: Option<&str>) {
    let key = cache_key(repo);
    if let Err(e) = std::fs::create_dir_all(&paths.cache) {
        log::warn!("couldn't create {}: {e}", paths.cache.display());
    }
    if let Err(e) = std::fs::write(paths.cache.join(format!("{key}.json")), text) {
        log::warn!("couldn't save the release answer: {e}");
    }
    if let Some(etag) = etag
        && let Err(e) = std::fs::write(paths.cache.join(format!("{key}.etag")), etag)
    {
        log::warn!("couldn't save the release etag: {e}");
    }
}

/// Saves a release read from GitHub's pages, for when the machine is next offline.
fn save_pages_release(paths: &Paths, repo: &str, release: &Release) {
    let key = cache_key(repo);
    if let Err(e) = std::fs::create_dir_all(&paths.cache) {
        log::warn!("couldn't create {}: {e}", paths.cache.display());
    }
    match serde_json::to_vec(release) {
        Ok(json) => {
            if let Err(e) = std::fs::write(paths.cache.join(format!("{key}-pages.json")), json) {
                log::warn!("couldn't save the release pages answer: {e}");
            }
        }
        Err(e) => log::warn!("couldn't serialize the release pages answer: {e}"),
    }
}

/// The last good answer for a repository, when it is still on disk: the API's answer first, then
/// the copy saved from the release pages.
fn saved_release(paths: &Paths, repo: &str) -> Option<Release> {
    let key = cache_key(repo);
    if let Ok(text) = std::fs::read_to_string(paths.cache.join(format!("{key}.json")))
        && let Ok(release) = release::parse_github_release(&text, repo)
    {
        return Some(release);
    }
    let text = std::fs::read_to_string(paths.cache.join(format!("{key}-pages.json"))).ok()?;
    serde_json::from_str(&text).ok()
}

/// A network failure keeps the last good answer for that repository, if there is one.
fn fallback(paths: &Paths, repo: &str, why: String) -> Answer {
    match saved_release(paths, repo) {
        Some(release) => Answer { outcome: Outcome::Release(release), source: Source::Cache, api_limited_until: None, rate_limited_until: None },
        None => Answer { outcome: Outcome::Failed(why), source: Source::GitHubApi, api_limited_until: None, rate_limited_until: None },
    }
}

/// Checks every repository the suite uses. Each repository has its own ETag/cache file, so a
/// rate limit or outage on one doesn't hide the releases of the other.
fn check_for_updates(shared: &Shared, http: Option<&Http>, paths: &Paths, force: bool) {
    shared.emit(Event::CheckStarted);
    let repos = suite_repos();

    let Some(http) = http else {
        let why = "secure connections aren't available on this system".to_string();
        for repo in &repos {
            publish(shared, fallback(paths, repo, why.clone()), repo);
        }
        shared.emit(Event::CheckFinished { at: now_seconds(), source: Source::GitHubApi, rate_limited_until: None });
        return;
    };

    let mut api_limited = {
        let until = *shared.rate_limited_until.lock().unwrap_or_else(PoisonError::into_inner);
        until.filter(|until| now_seconds() < *until)
    };
    let mut last_source = Source::GitHubApi;
    let mut rate_limited_until = None;

    for repo in &repos {
        let answer = if let Some(until) = api_limited {
            // The API's quota is spent, but GitHub's release pages are not limited. Read those
            // instead of showing the saved (possibly old) answer as if nothing were available.
            log::info!("GitHub's API is rate limited; reading the release pages for {repo}");
            ask_pages(http, paths, repo, "GitHub's API is out of quota right now".into(), Some(until))
        } else {
            ask_github(http, paths, force, repo)
        };
        // Remember the refusal even when the release pages answered: it saves asking the API
        // again on every other repository and on the next check.
        if let Some(until) = answer.api_limited_until {
            api_limited = Some(until);
            *shared.rate_limited_until.lock().unwrap_or_else(PoisonError::into_inner) = Some(until);
        }
        if let Some(until) = answer.rate_limited_until {
            rate_limited_until = Some(until);
        }
        last_source = answer.source;
        publish(shared, answer, repo);
    }

    shared.emit(Event::CheckFinished { at: now_seconds(), source: last_source, rate_limited_until });
}

/// Turns one repository's answer into the per-app answers the UI shows. Only apps whose
/// `release_source_for` names this repository are touched, so every repository can publish in
/// turn without wiping the others' latest releases.
fn publish(shared: &Shared, answer: Answer, repo: &str) {
    let installed = shared.installed.lock().unwrap_or_else(PoisonError::into_inner).clone();
    let mut events = Vec::with_capacity(SUITE_ANDROID.len() + 8);
    let mut updated_apps: Vec<&'static str> = Vec::new();
    let mut update_release: Option<(String, String)> = None;
    {
        let mut latest = shared.latest.lock().unwrap_or_else(PoisonError::into_inner);
        match &answer.outcome {
            Outcome::Release(release) => {
                update_release = Some((release.tag.clone(), release.version_label()));
                for app in &SUITE_ANDROID {
                    let source = catalog::release_source_for(app);
                    if source.repo != repo {
                        continue;
                    }
                    let app_id = app.id.to_string();
                    let asset = match source.asset_pattern {
                        Some(pattern) => release.assets.iter().find(|asset| catalog::asset_matches(pattern, &asset.name)),
                        None => release.assets.iter().find(|asset| catalog::parse_apk_name(&asset.name).is_some_and(|(id, _)| id == app.id)),
                    };
                    match asset {
                        Some(asset) => {
                            let built = catalog::android_release(
                                app,
                                &release.tag,
                                release.version.clone(),
                                release.published_at,
                                release.html_url.clone(),
                                release.notes.clone(),
                                asset.clone(),
                            );
                            if let (Some(copy), Some(latest_version)) = (installed.get(app.id), built.version.as_ref())
                                && let Some(installed_version) = copy.version.as_ref()
                            {
                                let installable = version::is_update(latest_version, installed_version);
                                if installable {
                                    updated_apps.push(app.name);
                                }
                                events.push(Event::Installable { app: app_id.clone(), installable });
                            }
                            latest.insert(app_id.clone(), built.clone());
                            events.push(Event::Latest { app: app_id, latest: LatestRelease::Found(built), from: Origin::GitHub });
                        }
                        // Published releases exist, but none for this app (yet).
                        None => {
                            latest.remove(&app_id);
                            events.push(Event::Latest { app: app_id, latest: LatestRelease::None, from: Origin::GitHub });
                        }
                    }
                }
            }
            Outcome::NoRelease => {
                for app in &SUITE_ANDROID {
                    if catalog::release_source_for(app).repo != repo {
                        continue;
                    }
                    latest.remove(app.id);
                    events.push(Event::Latest { app: app.id.to_string(), latest: LatestRelease::None, from: Origin::GitHub });
                }
            }
            Outcome::Failed(why) => {
                // Keep the release we already know about; the UI shows the failure in the tooltip.
                for app in &SUITE_ANDROID {
                    if catalog::release_source_for(app).repo != repo {
                        continue;
                    }
                    events.push(Event::Latest { app: app.id.to_string(), latest: LatestRelease::Failed(why.clone()), from: Origin::GitHub });
                }
            }
        }
    }
    if let Some((tag, version)) = update_release
        && !updated_apps.is_empty()
    {
        shared.notify_updates(repo, &tag, &version, &updated_apps);
    }
    shared.emit_all(events);
}

/// Downloads and installs (or updates) one app's latest APK.
fn install(shared: &Shared, bridge: &Bridge, http: Option<&Http>, app_id: &str) {
    let Some(app) = catalog::app_by_id(app_id) else {
        shared.emit(Event::JobFinished { app: app_id.to_string(), result: Err("this app isn't part of the Android suite".into()) });
        return;
    };
    let app_name = app.name;
    let release = shared.latest.lock().unwrap_or_else(PoisonError::into_inner).get(app_id).cloned();
    let version_label = release.as_ref().map_or_else(|| "the latest version".to_string(), Release::version_label);
    let notify_id = format!("download-{app_id}");
    let Some(asset) = release.and_then(|release| release.assets.first().cloned()) else {
        shared.emit(Event::JobFinished { app: app_id.to_string(), result: Err(format!("{app_name} has no Android build yet.")) });
        return;
    };
    let Some(http) = http else {
        shared.emit(Event::JobFinished { app: app_id.to_string(), result: Err("secure connections aren't available on this system".into()) });
        return;
    };
    // GitHub publishes a SHA-256 digest for every release asset; without one there is nothing to
    // verify the download against, so it isn't installed.
    let Some(sha256) = asset.sha256.as_deref().filter(|hash| !hash.is_empty()) else {
        shared.emit(Event::JobFinished {
            app: app_id.to_string(),
            result: Err("the release doesn't publish a checksum for this build, so it wasn't installed".into()),
        });
        return;
    };

    let dest = AndroidBackend::download_path(&shared.files_dir, &asset);
    let stop = Arc::new(Stop::default());
    shared.stops.lock().unwrap_or_else(PoisonError::into_inner).insert(app_id.to_string(), stop.clone());
    let done = http::partial_bytes(&dest, asset.size).unwrap_or(0);
    shared.emit(Event::Progress { app: app_id.to_string(), phase: JobPhase::Downloading { done, total: asset.size } });
    bridge.notify(
        &notify_id,
        &format!("Downloading {app_name}"),
        &format!("Version {version_label}"),
        "downloads",
        Some(percent(done, asset.size)),
        true,
    );
    let result = {
        let mut last_notice = Instant::now();
        let mut last_percent = percent(done, asset.size);
        let mut report = |transfer: Transfer| {
            let (phase, done, total) = match transfer {
                Transfer::Bytes { done, total } => (JobPhase::Downloading { done, total }, done, total),
                Transfer::Reconnecting { done, total } => (JobPhase::Reconnecting { done, total }, done, total),
            };
            shared.emit(Event::Progress { app: app_id.to_string(), phase });
            // The notification only says something new once a second (or at the end): the
            // transfer reports every chunk, and Java polls its notification files every 150 ms.
            let now_percent = percent(done, total);
            if now_percent != last_percent && (now_percent == 100 || last_notice.elapsed() >= Duration::from_secs(1)) {
                last_percent = now_percent;
                last_notice = Instant::now();
                bridge.notify(
                    &notify_id,
                    &format!("Downloading {app_name}"),
                    &format!("Version {version_label}"),
                    "downloads",
                    Some(now_percent),
                    true,
                );
            }
        };
        http.download(&asset.url, &dest, asset.size, sha256, &stop, &mut report)
    };
    shared.stops.lock().unwrap_or_else(PoisonError::into_inner).remove(app_id);

    match result {
        Ok(()) => {
            shared.emit(Event::Progress { app: app_id.to_string(), phase: JobPhase::Installing });
            bridge.notify(&notify_id, &format!("Installing {app_name}"), "Confirm the system prompt to finish.", "downloads", None, true);
            let package = catalog::package_for(app_id);
            match bridge.install_apk(&package, &dest, INSTALL_TIMEOUT) {
                Ok(()) => {
                    let _ = std::fs::remove_file(&dest);
                    shared.redetect.store(true, Ordering::SeqCst);
                    bridge.notify(&notify_id, &format!("{app_name} is installed"), &format!("Version {version_label}"), "downloads", None, false);
                    shared.emit(Event::JobFinished { app: app_id.to_string(), result: Ok(format!("{app_name} is installed.")) });
                }
                Err(why) if why == INSTALL_CANCELLED => {
                    // The user dismissed the system prompt: the downloaded APK stays for next time.
                    bridge.cancel_notification(&notify_id);
                    shared.emit(Event::JobFinished { app: app_id.to_string(), result: Err(INSTALL_CANCELLED.into()) });
                }
                Err(why) => {
                    bridge.notify(&notify_id, &format!("Couldn't install {app_name}"), &why, "downloads", None, false);
                    shared.emit(Event::JobFinished { app: app_id.to_string(), result: Err(why) });
                }
            }
        }
        Err(why) if why == PAUSED => {
            let done = http::partial_bytes(&dest, asset.size).unwrap_or(0);
            bridge.cancel_notification(&notify_id);
            shared.emit(Event::Paused { app: app_id.to_string(), download: Some(PausedDownload { done, total: asset.size }) });
            shared.emit(Event::JobFinished { app: app_id.to_string(), result: Err(PAUSED.into()) });
        }
        Err(why) if why == CANCELLED => {
            http::discard(&dest, asset.size);
            bridge.cancel_notification(&notify_id);
            shared.emit(Event::Paused { app: app_id.to_string(), download: None });
            shared.emit(Event::JobFinished { app: app_id.to_string(), result: Err(CANCELLED.into()) });
        }
        Err(why) => {
            bridge.notify(&notify_id, &format!("Couldn't download {app_name}"), &why, "downloads", None, false);
            shared.emit(Event::Paused { app: app_id.to_string(), download: None });
            shared.emit(Event::JobFinished { app: app_id.to_string(), result: Err(why) });
        }
    }
}

/// A download's progress as a whole percentage (`0` when the size is unknown).
fn percent(done: u64, total: u64) -> u8 {
    if total == 0 {
        return 0;
    }
    (done.saturating_mul(100) / total).min(100) as u8
}

/// Asks the system to uninstall the app. The worker waits for the user's decision in the system
/// prompt (like an install waits for it), then re-detects and reports what happened.
fn uninstall(shared: Arc<Shared>, bridge: &Bridge, app_id: &str) {
    let package = catalog::package_for(app_id);
    let name = catalog::app_by_id(app_id).map(|def| def.name).unwrap_or(app_id);
    shared.emit(Event::Progress { app: app_id.to_string(), phase: JobPhase::Removing });
    match bridge.uninstall(&package, INSTALL_TIMEOUT) {
        Ok(true) => {
            shared.redetect.store(true, Ordering::SeqCst);
            shared.waker.wake();
            shared.emit(Event::JobFinished { app: app_id.to_string(), result: Ok(format!("{name} is uninstalled.")) });
        }
        // The user dismissed the prompt: the app is still there.
        Ok(false) => {
            shared.redetect.store(true, Ordering::SeqCst);
            shared.waker.wake();
            shared.emit(Event::JobFinished { app: app_id.to_string(), result: Err(CANCELLED.into()) });
        }
        Err(why) => {
            shared.emit(Event::JobFinished { app: app_id.to_string(), result: Err(why) });
        }
    }
}

fn now_seconds() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

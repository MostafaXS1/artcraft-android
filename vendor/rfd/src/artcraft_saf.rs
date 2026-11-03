//! ArtCraft Android builder bridge: route the Android file pickers through the generated Storage
//! Access Framework (SAF) Activity.
//!
//! Upstream rfd 0.17 has no Android backend; the builder vendors a compile-only copy whose dialogs
//! all resolve to "cancelled". On top of that, this module lets the generated launcher start the
//! system open/save/folder pickers and deliver their answers back to the rfd futures.
//!
//! An app drives its rfd dialog on a worker thread (`pollster::block_on`), so a picker future
//! returns `Pending` here until the launcher's per-frame poll delivers the answer — the frame loop
//! keeps running on its own thread, so nothing deadlocks.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::Waker;

/// Which system picker to show for a request.
#[derive(Clone, Debug)]
pub enum Kind {
    /// One or more files to open. `multiple` allows the picker to return several at once.
    Open { multiple: bool },
    /// Where to save one file, with the dialog's suggested file name.
    Save(String),
    /// A folder.
    Folder,
}

struct Pending {
    result: Option<Option<Vec<PathBuf>>>,
    waker: Option<Waker>,
}

static LAUNCHER: std::sync::OnceLock<Arc<dyn Fn(u64, Kind) + Send + Sync>> = std::sync::OnceLock::new();
/// Launcher for the *synchronous* dialog API (`FileDialog::save_file`, `pick_file`, `pick_folder`),
/// which cannot await a future. The generated/adapted Activity supplies a closure that shows the
/// picker and waits for its answer before returning. It runs on the Rust main thread, whose
/// callbacks the SAF bridge makes non-blocking for the duration (see `saf_bridge.rs`), so parking
/// it does not stall Android's lifecycle handshake.
static BLOCKING_LAUNCHER: std::sync::OnceLock<Arc<dyn Fn(Kind) -> Option<Vec<PathBuf>> + Send + Sync>> =
    std::sync::OnceLock::new();
static PENDING: std::sync::LazyLock<Mutex<HashMap<u64, Pending>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn lock_pending() -> std::sync::MutexGuard<'static, HashMap<u64, Pending>> {
    PENDING.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Install the launcher the generated Activity provides. It starts the system picker for a request
/// and returns at once; the answer arrives later through [`deliver`]. Only one launcher is needed;
/// the first install wins.
pub fn install(launcher: Arc<dyn Fn(u64, Kind) + Send + Sync>) {
    let _ = LAUNCHER.set(launcher);
}

/// Install the launcher for the synchronous dialog API: it shows the picker for `kind` and returns
/// the chosen paths (`None` = cancelled) before returning. Used by apps whose file API is the
/// blocking `rfd::FileDialog` (SubCraft), which has no future to await.
pub fn install_blocking(launcher: Arc<dyn Fn(Kind) -> Option<Vec<PathBuf>> + Send + Sync>) {
    let _ = BLOCKING_LAUNCHER.set(launcher);
}

/// Deliver a picker's answer (`paths`; `None` = cancelled) and wake the waiting future.
pub fn deliver(id: u64, paths: Option<Vec<PathBuf>>) {
    let mut pending = lock_pending();
    if let Some(p) = pending.get_mut(&id) {
        p.result = Some(paths);
        if let Some(w) = p.waker.take() {
            w.wake();
        }
    }
}

/// Start the picker for `kind`; `None` when no launcher is installed (the dialog then resolves to
/// "cancelled", exactly as the plain stub did).
fn start(kind: Kind) -> Option<u64> {
    let launcher = LAUNCHER.get()?;
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    lock_pending().insert(id, Pending { result: None, waker: None });
    launcher(id, kind);
    Some(id)
}

/// A future that resolves with the system picker's chosen paths (`None` = cancelled).
struct SafFuture {
    id: u64,
}

impl std::future::Future for SafFuture {
    type Output = Option<Vec<PathBuf>>;
    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let mut pending = lock_pending();
        if let Some(p) = pending.get_mut(&self.id) {
            if let Some(result) = p.result.take() {
                pending.remove(&self.id);
                return std::task::Poll::Ready(result);
            }
            p.waker = Some(cx.waker().clone());
        }
        std::task::Poll::Pending
    }
}

/// The Android backend's entry point: run the SAF picker for `kind` and resolve with its paths.
pub(crate) async fn pick(kind: Kind) -> Option<Vec<PathBuf>> {
    let id = start(kind)?;
    SafFuture { id }.await
}

/// The synchronous dialog API's entry point: run the blocking SAF picker for `kind` and return its
/// paths (`None` when cancelled, or when no blocking launcher is installed — the dialog then
/// resolves to "cancelled", exactly as the plain stub did).
pub(crate) fn pick_sync(kind: Kind) -> Option<Vec<PathBuf>> {
    BLOCKING_LAUNCHER.get().and_then(|launcher| launcher(kind))
}

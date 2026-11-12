//! The Termux side of the Android shell.
//!
//! Coiman's Git panel, Run button and terminal all use programs that only exist inside Termux
//! (git, cargo, python, …). Android gives an app no way to execute another app's binaries, so the
//! shell talks to Termux through its official `RUN_COMMAND` service instead:
//!
//! * `prepare.sh` bundles `termux_shim.c` twice, as `libcoiman_termux_shim.so` (tools) and
//!   `libcoiman_termux_shell.so` (the interactive terminal);
//! * a tool command is routed to the shim by `quiet_command` (patched in `coiman-core`); the shim
//!   writes a tiny request into app-private cache, the Activity's `TermuxBroker` turns it into a
//!   `RUN_COMMAND` intent, and Termux runs the tool in shared storage;
//! * the interactive terminal starts the same shim in terminal mode, where Termux runs `script`
//!   around `bash -i` and the shim relays bytes both ways.
//!
//! Everything the two apps exchange lives in `/sdcard/Documents/CoimanTermux`: Termux cannot read
//! an app's own external files directory (`Android/data/<pkg>`), so shared storage is the only
//! place both sides can see.
//!
//! The Rust side only publishes the locations (environment variables read by the shim and the
//! patched `coiman-core`); the protocol itself lives in the shim and `TermuxBroker.java`.

use android_activity::AndroidApp;
use std::path::{Path, PathBuf};

/// The Termux application package the `RUN_COMMAND` service belongs to.
pub const TERMUX_PACKAGE: &str = "com.termux";

/// Termux's shell, the program the terminal tab starts.
pub const TERMUX_BASH: &str = "/data/data/com.termux/files/usr/bin/bash";

/// Termux's home directory (shown in setup hints).
pub const TERMUX_HOME: &str = "/data/data/com.termux/files/home";

/// The shared folder both apps can read and write.
///
/// `/sdcard` is the app's view of the primary shared storage; Termux sees the same files through
/// the same path once `termux-setup-storage` has been run.
pub const PUBLIC_DIR: &str = "/sdcard/Documents/CoimanTermux";

/// The request directory below the app's own files directory (`getFilesDir()/coiman-termux`).
pub const REQUEST_DIR_NAME: &str = "coiman-termux";

/// The name `prepare.sh` bundles the tool shim under.
pub const SHIM_NAME: &str = "libcoiman_termux_shim.so";

/// The name `prepare.sh` bundles the terminal shim under.
pub const SHELL_NAME: &str = "libcoiman_termux_shell.so";

/// Point every Termux-facing component at the bundled shim and the shared folder.
///
/// Called before `CodeApp::new`, so the patched `quiet_command` sees `COIMAN_TERMUX_SHIM` from the
/// first probe onward. `system_fonts::install_font_env` runs first and owns `HOME`, which does not
/// matter here: the shim and Termux keep their own environments.
#[allow(unused_unsafe)]
pub fn install_env(app: &AndroidApp) {
    let Some(native_dir) = native_library_dir() else {
        log::warn!("Coiman IDE: the Termux shim was not found next to the native libraries");
        return;
    };
    let shim = native_dir.join(SHIM_NAME);
    let shell = native_dir.join(SHELL_NAME);
    if !shim.is_file() {
        log::warn!("Coiman IDE: the Termux shim is missing at {}", shim.display());
        return;
    }
    let request_dir = app
        .internal_data_path()
        .map(|base| base.join(REQUEST_DIR_NAME))
        .unwrap_or_else(|| PathBuf::from("/data/user/0/com.artcraft.coimanide/files").join(REQUEST_DIR_NAME));
    let _ = std::fs::create_dir_all(&request_dir);
    let _ = std::fs::create_dir_all(PUBLIC_DIR);
    let public = Path::new(PUBLIC_DIR);

    // SAFETY: `android_main` runs before Coiman starts a worker thread, and nothing else writes
    // these variables afterwards.
    unsafe {
        std::env::set_var("COIMAN_TERMUX_SHIM", &shim);
        std::env::set_var(
            "COIMAN_TERMUX_SHELL",
            if shell.is_file() { shell.as_os_str() } else { shim.as_os_str() },
        );
        std::env::set_var("COIMAN_TERMUX_REQ", &request_dir);
        std::env::set_var("COIMAN_TERMUX_PUBLIC", public);
        std::env::set_var("COIMAN_TERMUX_HOME", TERMUX_HOME);
        std::env::set_var("SHELL", TERMUX_BASH);
    }
    log::info!(
        "Termux bridge: shim {}, requests {}, shared {}",
        shim.display(),
        request_dir.display(),
        public.display()
    );
}

/// The directory Android unpacked the app's native libraries into.
///
/// `android-activity` does not expose `ApplicationInfo.nativeLibraryDir`, and the bundled shims
/// are never `dlopen`ed (so they do not appear in `/proc/self/maps` themselves). The app's own
/// launcher library does: one of the mapped `.so` files lives in that directory, and the shims sit
/// next to it.
pub fn native_library_dir() -> Option<PathBuf> {
    let maps = std::fs::read_to_string("/proc/self/maps").ok()?;
    for line in maps.lines() {
        let Some(path) = line.split_whitespace().last() else { continue };
        if !path.ends_with(".so") {
            continue;
        }
        if !(path.contains("/lib/") || path.contains("/lib64/")) {
            continue;
        }
        let Some(dir) = Path::new(path).parent() else { continue };
        if dir.join(SHIM_NAME).is_file() {
            return Some(dir.to_path_buf());
        }
    }
    None
}

/// The bundled tool shim, if this build has one.
pub fn shim() -> Option<PathBuf> {
    std::env::var_os("COIMAN_TERMUX_SHIM").map(PathBuf::from).filter(|path| path.is_file())
}

/// The bundled terminal shim (falls back to the tool shim).
pub fn shell_shim() -> Option<PathBuf> {
    std::env::var_os("COIMAN_TERMUX_SHELL")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .or_else(shim)
}

/// The shared request/result folder.
pub fn public_dir() -> PathBuf {
    std::env::var_os("COIMAN_TERMUX_PUBLIC")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(PUBLIC_DIR))
}

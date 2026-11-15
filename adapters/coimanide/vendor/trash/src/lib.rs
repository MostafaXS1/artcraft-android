//! Builder stub of the `trash` crate for Android.
//!
//! The real crate has no Android platform module, so it cannot compile for
//! `aarch64-linux-android`. Android has no desktop trash can either: a file deleted from the
//! system file manager is gone. Coiman's "Delete" therefore removes the path permanently and
//! reports success exactly like `trash::delete` does on the desktop. `prepare.sh` patches this
//! crate into the workspace with `[patch.crates-io]`, so upstream code keeps its `trash::delete`
//! call unchanged.

use std::fmt;
use std::path::Path;

/// The error type `trash::delete` returns. Only `Display`/`Error` are used by Coiman.
#[derive(Debug)]
pub struct Error(String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// Removes a file or directory permanently.
pub fn delete<T: AsRef<Path>>(path: T) -> Result<(), Error> {
    let path = path.as_ref();
    let result = if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    result.map_err(|error| Error(format!("could not delete {}: {error}", path.display())))
}

/// Compatibility with code that names the context type.
#[derive(Clone, Debug, Default)]
pub struct TrashContext;

impl TrashContext {
    pub const fn new() -> Self {
        Self
    }

    pub fn delete<T: AsRef<Path>>(&self, path: T) -> Result<(), Error> {
        delete(path)
    }
}

/// The real crate's convenience constant.
pub const DEFAULT_TRASH_CTX: TrashContext = TrashContext;

//! Termux tool forwarding for Android.
//!
//! Coiman calls `quiet_command` for every external program it runs: Git, the Run button, the
//! language servers, tests, formatters and the plugin tools. On Android those programs do not
//! exist in the app's own sandbox; they live in Termux. The builder's shell bundles a small shim
//! (`libcoiman_termux_shim.so`) and points this module at it through `COIMAN_TERMUX_SHIM`.
//!
//! `quiet_command` asks [`tool_for`] whether a program is one of the tools Termux provides. If it
//! is, the command is started as the shim with `COIMAN_TERMUX_TOOL=<name>`; the shim sends the
//! invocation to Termux's `RUN_COMMAND` service and relays stdin/stdout/stderr as if the program
//! had run locally. Everything else (the app's own bundled programs, URLs opened as commands, …)
//! keeps the desktop behaviour.
//!
//! The allowlist matters: only short-lived tools and language servers that speak a pipe protocol
//! are forwarded. Android's own `/system/bin` tools are deliberately absent — a `pkill` in Termux
//! cannot signal this app's processes.
//!
//! ARTCRAFT_COIMANIDE_TERMUX_TOOLS_V1

use std::path::{Path, PathBuf};

/// The shim `prepare.sh` bundles next to the app's native libraries.
pub fn shim() -> Option<PathBuf> {
    std::env::var_os("COIMAN_TERMUX_SHIM")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
}

/// The terminal shim (`libcoiman_termux_shell.so`), falling back to the tool shim when the build
/// bundled only one binary.
pub fn shell_shim() -> Option<PathBuf> {
    std::env::var_os("COIMAN_TERMUX_SHELL")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .or_else(shim)
}

/// The Termux tool name for `program`, or `None` when it should run locally.
pub fn tool_for(program: &Path) -> Option<String> {
    let name = program.file_name()?.to_str()?.to_ascii_lowercase();
    let name = name.strip_suffix(".exe").unwrap_or(&name);
    TOOLS.contains(&name).then(|| name.to_owned())
}

/// Programs Termux is expected to provide. Keep this list to tools that finish on their own or
/// speak a line/pipe protocol (the language servers); a terminal is started through the shell
/// shim, not through this path.
const TOOLS: &[&str] = &[
    // Source control and project hosts.
    "git", "gh", "git-lfs", "ssh", "scp", "rsync",
    // Downloads and archives.
    "curl", "wget", "tar", "gzip", "gunzip", "xz", "unxz", "zstd", "unzip", "zip", "jq",
    // Run buttons, tests and builds.
    "cargo", "rustc", "rustup", "rustfmt", "cargo-clippy", "clippy-driver", "python", "python3",
    "pip", "pip3", "uv", "poetry", "pytest", "node", "npm", "npx", "yarn", "pnpm", "tsc", "go",
    "gofmt", "java", "javac", "jar", "mvn", "gradle", "gradlew", "gcc", "g++", "cc", "c++",
    "clang", "clang++", "make", "cmake", "ninja", "ruby", "perl", "php", "lua", "sqlite3",
    "ffmpeg", "ffprobe", "openssl", "bash", "sh", "env",
    // Language servers (they stream JSON-RPC over the same relay).
    "rust-analyzer", "gopls", "pyright", "pyright-langserver", "clangd", "clangd-18",
    "typescript-language-server", "bash-language-server", "yaml-language-server",
    "vscode-json-language-server", "taplo", "jdtls", "lua-language-server", "intelephense",
];

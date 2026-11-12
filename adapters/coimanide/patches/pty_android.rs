//! Android terminal sessions: a Termux shell, relayed through the bundled shim.
//!
//! Included from `pty.rs` with `#[path = "pty_android.rs"]` only on Android. It reuses the desktop
//! module's public types (`Size`, `Notice`, `Listener`, the alacritty re-export and `screen_text`
//! shape) but never touches `portable-pty`: Android apps cannot fork a shell that has git, cargo or
//! python, and the tools live inside Termux. The shim (`termux_shim.c`) starts a `bash -i` session
//! inside Termux, relays this process's stdin/stdout through shared files, and exits with the
//! remote status. `Session` is therefore a normal child process from the app's point of view.
//!
//! ARTCRAFT_COIMANIDE_TERMUX_TERMINAL_V1

use super::*;

use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

/// Appends a line to the bridge's log in the shared folder.
///
/// The bridge lives in another app, so a failure there (Termux not installed, a missing package,
/// a refused permission) is invisible to Coiman except as the command's own output. The log makes
/// it readable from a computer (`adb shell cat /sdcard/Documents/CoimanTermux/coiman-termux.log`)
/// and from the device itself.
fn note(message: &str) {
    let Some(public) = std::env::var_os("COIMAN_TERMUX_PUBLIC") else { return };
    let path = std::path::PathBuf::from(public).join("coiman-termux.log");
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "{stamp} {message}");
    }
}

pub struct Session {
    term: Arc<FairMutex<Term<Listener>>>,
    writer: Writer,
    child: Arc<Mutex<Option<Child>>>,
    pid: Option<u32>,
    ended: Arc<AtomicBool>,
    notices: Receiver<Notice>,
    size: Size,
    shared_size: Arc<Mutex<Size>>,
    /// Program name, e.g. "bash".
    pub name: String,
    exited: bool,
}

impl Session {
    /// Starts the Termux shell in `cwd`; `command` is the configured shell name and is kept for
    /// signature compatibility (the remote side always runs Termux's own bash).
    pub fn spawn(
        _command: &str,
        cwd: &Path,
        env: &[(String, String)],
        size: Size,
        scrollback: usize,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, String> {
        Self::start(None, &[], cwd, env, size, scrollback, wake)
    }

    /// Runs a program inside Termux (a Run-button command, a test, a task terminal).
    pub fn spawn_args(
        program: &str,
        args: &[String],
        cwd: &Path,
        env: &[(String, String)],
        size: Size,
        scrollback: usize,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, String> {
        Self::start(Some(program), args, cwd, env, size, scrollback, wake)
    }

    fn start(
        program: Option<&str>,
        args: &[String],
        cwd: &Path,
        env: &[(String, String)],
        size: Size,
        scrollback: usize,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, String> {
        let (shim, tool) = match program {
            None => (
                crate::android_termux::shell_shim(),
                "__terminal__".to_owned(),
            ),
            Some(program) => {
                // Task programs come from launch settings and may be absolute desktop paths.
                // Termux resolves the bare name against its own PATH.
                let name = Path::new(program)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(program)
                    .to_owned();
                (crate::android_termux::shim(), name)
            }
        };
        let shim = shim.ok_or_else(|| {
            "Coiman IDE: the Termux bridge is missing from this build; reinstall the app.".to_owned()
        })?;

        let mut cmd = Command::new(&shim);
        cmd.env("COIMAN_TERMUX_TOOL", &tool);
        if program.is_some() {
            cmd.args(args);
        }
        cmd.env("COIMAN_TERMUX_SIZE", format!("{}x{}", size.columns, size.lines));
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERM_PROGRAM", "coiman-ide");
        for (key, value) in env {
            cmd.env(key, value);
        }
        cmd.current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        note(&format!(
            "session started: tool {tool}, cwd {}, args {:?}",
            cwd.display(),
            args
        ));
        let mut child = cmd
            .spawn()
            .map_err(|error| format!("could not start the Termux session: {error}"))?;
        let stdout = child.stdout.take().ok_or("the Termux session has no output")?;
        let stderr = child.stderr.take();
        let stdin = child.stdin.take().ok_or("the Termux session has no input")?;
        let writer: Writer = Arc::new(Mutex::new(Box::new(stdin) as Box<dyn Write + Send>));

        let (tx, notices) = mpsc::channel();
        let shared_size = Arc::new(Mutex::new(size));
        let listener = Listener {
            tx: tx.clone(),
            writer: Arc::clone(&writer),
            size: Arc::clone(&shared_size),
        };
        let config = Config { scrolling_history: scrollback, ..Config::default() };
        let term = Arc::new(FairMutex::new(Term::new(config, &size, listener)));
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);

        spawn_reader(Arc::clone(&term), Arc::clone(&wake), stdout, "stdout");
        if let Some(stderr) = stderr {
            spawn_reader(Arc::clone(&term), Arc::clone(&wake), stderr, "stderr");
        }

        let pid = child.id();
        let child = Arc::new(Mutex::new(Some(child)));
        let ended = Arc::new(AtomicBool::new(false));
        {
            let child = Arc::clone(&child);
            let ended = Arc::clone(&ended);
            let wake = Arc::clone(&wake);
            let tx: Sender<Notice> = tx.clone();
            std::thread::Builder::new()
                .name("termux-wait".into())
                .spawn(move || loop {
                    let status = {
                        let mut guard = child.lock().unwrap_or_else(|error| error.into_inner());
                        match guard.as_mut() {
                            Some(child) => match child.try_wait() {
                                Ok(Some(status)) => Some(status.code().map(|code| code as u32)),
                                Ok(None) => None,
                                Err(_) => Some(None),
                            },
                            None => Some(None),
                        }
                    };
                    match status {
                        Some(code) => {
                            ended.store(true, Ordering::Relaxed);
                            note(&format!("session ended: exit code {code:?}"));
                            let _ = tx.send(Notice::Exited(code));
                            wake();
                            break;
                        }
                        None => std::thread::sleep(std::time::Duration::from_millis(50)),
                    }
                })
                .map_err(|error| error.to_string())?;
        }

        let name = if program.is_none() {
            "bash".to_owned()
        } else {
            tool.clone()
        };
        Ok(Self {
            term,
            writer,
            child,
            pid: Some(pid),
            ended,
            notices,
            size,
            shared_size,
            name,
            exited: false,
        })
    }

    /// The emulator state (grid, cursor, modes); hold the lock briefly.
    pub fn term(&self) -> impl std::ops::DerefMut<Target = Term<Listener>> + '_ {
        self.term.lock()
    }

    /// Sends input (typed text or escape sequences) to the remote shell.
    pub fn write(&self, bytes: &[u8]) {
        if !self.exited {
            write_all(&self.writer, bytes);
        }
    }

    pub fn size(&self) -> Size {
        self.size
    }

    /// Updates the local emulator. The remote pty keeps the size the session started with: a FIFO
    /// has no window size to change, and Termux's `script` session is created once.
    pub fn resize(&mut self, size: Size) {
        if size == self.size || size.columns == 0 || size.lines == 0 {
            return;
        }
        self.size = size;
        *self.shared_size.lock().unwrap_or_else(|error| error.into_inner()) = size;
        self.term.lock().resize(size);
    }

    /// Everything reported since the last call.
    pub fn poll(&mut self) -> Vec<Notice> {
        let notices: Vec<Notice> = self.notices.try_iter().collect();
        if notices.iter().any(|notice| matches!(notice, Notice::Exited(_))) {
            self.exited = true;
        }
        notices
    }

    pub fn has_exited(&self) -> bool {
        self.exited
    }

    /// The visible screen as text, one line per row (trailing spaces trimmed).
    pub fn screen_text(&self) -> String {
        let term = self.term.lock();
        let grid = term.grid();
        let mut out = String::new();
        for line in 0..grid.screen_lines() {
            let row = &grid[Line(line as i32)];
            let text: String = (0..grid.columns()).map(|column| row[Column(column)].c).collect();
            out.push_str(text.trim_end());
            out.push('\n');
        }
        out
    }
}

fn spawn_reader(
    term: Arc<FairMutex<Term<Listener>>>,
    wake: Arc<dyn Fn() + Send + Sync>,
    mut reader: impl Read + Send + 'static,
    label: &'static str,
) {
    let _ = std::thread::Builder::new()
        .name("termux-reader".into())
        .spawn(move || {
            let mut processor: Processor = Processor::new();
            let mut buffer = vec![0u8; 64 * 1024];
            let mut logged = 0usize;
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => {
                        processor.advance(&mut *term.lock(), &buffer[..count]);
                        // The bridge's own messages (a missing package, a refused permission) come
                        // on stderr; keep the first ones in the log.
                        if logged < 400 {
                            let chunk = &buffer[..count.min(400 - logged)];
                            let text = String::from_utf8_lossy(chunk);
                            note(&format!("{label}: {}", text.trim_end()));
                            logged += chunk.len();
                        }
                        wake();
                    }
                }
            }
        });
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.ended.load(Ordering::Relaxed) {
            return;
        }
        // The shim writes the remote `kill` marker on SIGTERM, which is how the Termux watchdog
        // learns to stop the shell it started. Give it a moment before the hard kill.
        if let Some(pid) = self.pid {
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGTERM);
            }
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
        if let Some(child) = self.child.lock().unwrap_or_else(|error| error.into_inner()).as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

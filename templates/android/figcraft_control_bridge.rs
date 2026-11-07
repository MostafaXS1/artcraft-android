//! Bounded hand-off from Android background workers to FigCraft's live UI thread.
//!
//! Producers must never mutate FigApp or Session directly. They enqueue the upstream
//! ControlRequest here; AndroidShell drains it immediately before FigApp::ui, where
//! FigCraft's own control::drain performs validation and execution.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

use figcraft_ui_egui::{ControlRequest, FigApp};

const MAX_PENDING_REQUESTS: usize = 64;

fn queue() -> &'static Mutex<VecDeque<ControlRequest>> {
    static QUEUE: OnceLock<Mutex<VecDeque<ControlRequest>>> = OnceLock::new();
    QUEUE.get_or_init(|| Mutex::new(VecDeque::new()))
}

/// Enqueue one request from a background worker.
///
/// Returns the original request when the queue is full, allowing the transport to
/// return an explicit overload error instead of silently dropping work.
fn push_bounded<T>(pending: &mut VecDeque<T>, item: T, limit: usize) -> Result<(), T> {
    if pending.len() >= limit {
        return Err(item);
    }
    pending.push_back(item);
    Ok(())
}

pub fn enqueue(request: ControlRequest) -> Result<(), ControlRequest> {
    let mut pending = queue()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    push_bounded(&mut pending, request, MAX_PENDING_REQUESTS)
}

/// Transfer pending requests to FigCraft on the UI thread.
///
/// The app's own control::drain then handles cancellation, execution and replies.
pub fn drain(app: &mut FigApp) {
    let requests: Vec<_> = {
        let mut pending = queue()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        pending.drain(..).collect()
    };
    app.control_queue.extend(requests);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_rejects_overflow_without_dropping_the_item() {
        let mut pending = VecDeque::new();
        for value in 0..MAX_PENDING_REQUESTS {
            assert!(push_bounded(&mut pending, value, MAX_PENDING_REQUESTS).is_ok());
        }
        assert_eq!(pending.len(), MAX_PENDING_REQUESTS);
        assert_eq!(push_bounded(&mut pending, 99, MAX_PENDING_REQUESTS), Err(99));
        assert_eq!(pending.len(), MAX_PENDING_REQUESTS);
    }
}

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use figcraft_mcp::{Backend, Server};
use serde_json::Value;

static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);
const MCP_PORT: u16 = 38_761;
const MAX_HTTP_HEADER_BYTES: usize = 16 * 1024;
const MAX_HTTP_BODY_BYTES: usize = 32 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

struct LiveBackend;

impl Backend for LiveBackend {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        let id = Value::from(NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed));
        let state = RequestGate::new();
        let request = ControlRequest {
            id: id.clone(),
            method: method.to_owned(),
            params,
            reply: reply_tx,
            state: state.clone(),
        };
        enqueue(request).map_err(|_| "FigCraft control queue is full; retry shortly".to_owned())?;
        match reply_rx.recv_timeout(REQUEST_TIMEOUT) {
            Ok(reply) if reply.id == id => reply.result,
            Ok(_) => Err("FigCraft returned a mismatched control reply id".into()),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                let state_after_timeout = state.cancel_if_open();
                Err(match state_after_timeout {
                    figcraft_ui_egui::RequestState::Cancelled => "FigCraft UI did not process the request before the 120-second deadline; it was cancelled".into(),
                    figcraft_ui_egui::RequestState::Running => "FigCraft started the request but did not finish within 120 seconds; inspect the document before retrying".into(),
                    figcraft_ui_egui::RequestState::Replied => "FigCraft completed the request but its reply was delayed".into(),
                    figcraft_ui_egui::RequestState::Open => "FigCraft request timed out".into(),
                })
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                Err("FigCraft UI dropped the control reply channel".into())
            }
        }
    }

    fn has_ui(&self) -> bool { true }

    fn describe(&self) -> String {
        format!("attached to the live FigCraft Android canvas on 127.0.0.1:{MCP_PORT}")
    }
}

fn write_http(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8]) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()
}

fn handle_http(stream: &mut TcpStream, server: &mut Server, token: &str) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    stream.set_write_timeout(Some(Duration::from_secs(130)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 { return Ok(()); }
    let mut headers = std::collections::HashMap::<String, String>::new();
    let mut header_bytes = request_line.len();
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 { return Ok(()); }
        header_bytes += n;
        if header_bytes > MAX_HTTP_HEADER_BYTES {
            return write_http(stream, "431 Request Header Fields Too Large", "text/plain", b"headers too large");
        }
        if line == "\r\n" || line == "\n" { break; }
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");
    if method != "POST" || path != "/mcp" {
        return write_http(stream, "405 Method Not Allowed", "text/plain", b"POST /mcp only");
    }
    let auth_ok = headers.get("authorization")
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|provided| constant_time_eq(provided.as_bytes(), token.as_bytes()));
    if !auth_ok {
        return write_http(stream, "401 Unauthorized", "text/plain", b"bearer token required");
    }
    let content_type_ok = headers.get("content-type")
        .is_some_and(|v| v.to_ascii_lowercase().starts_with("application/json"));
    let accept_ok = headers.get("accept").is_some_and(|v| {
        v.contains("application/json") || v.contains("text/event-stream") || v.trim() == "*/*"
    });
    if !content_type_ok || !accept_ok {
        return write_http(stream, "415 Unsupported Media Type", "text/plain", b"expected JSON MCP request and JSON/SSE accept");
    }
    if headers.contains_key("transfer-encoding") {
        return write_http(stream, "400 Bad Request", "text/plain", b"chunked transfer encoding is not supported");
    }
    let length = headers.get("content-length")
        .and_then(|v| v.parse::<usize>().ok())
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "missing content-length"))?;
    if length == 0 || length > MAX_HTTP_BODY_BYTES {
        return write_http(stream, "413 Content Too Large", "text/plain", b"request body size is invalid");
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    let body_text = std::str::from_utf8(&body)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    match server.handle_line(body_text) {
        Some(response) => write_http(stream, "200 OK", "application/json", response.as_bytes()),
        None => write_http(stream, "202 Accepted", "text/plain", b""),
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let mut diff = a.len() ^ b.len();
    let max = a.len().max(b.len());
    for i in 0..max {
        diff |= usize::from(*a.get(i).unwrap_or(&0) ^ *b.get(i).unwrap_or(&0));
    }
    diff == 0
}

/// Starts the optional local MCP Streamable HTTP endpoint only when the user has
/// explicitly enabled it by creating files/mcp-server.enabled in app-private storage.
/// The bearer token is stored in files/mcp-server.token and never written to logcat.
pub fn start_mcp_if_enabled(private_dir: &Path) {
    let enabled = private_dir.join("mcp-server.enabled");
    if !enabled.is_file() { return; }
    let token_path = private_dir.join("mcp-server.token");
    let token = match load_or_create_token(&token_path) {
        Ok(token) => token,
        Err(error) => {
            log::error!("MCP server could not load/create its local access token: {error}");
            return;
        }
    };
    std::thread::Builder::new()
        .name("figcraft-mcp-http".into())
        .spawn(move || {
            let listener = match TcpListener::bind(("127.0.0.1", MCP_PORT)) {
                Ok(listener) => listener,
                Err(error) => {
                    log::error!("MCP server could not bind 127.0.0.1:{MCP_PORT}: {error}");
                    return;
                }
            };
            log::info!("Local MCP Streamable HTTP endpoint enabled at http://127.0.0.1:{MCP_PORT}/mcp");
            let mut server = Server::new(Box::new(LiveBackend));
            for connection in listener.incoming() {
                match connection {
                    Ok(mut stream) => {
                        if let Err(error) = handle_http(&mut stream, &mut server, &token) {
                            log::warn!("MCP HTTP request failed: {error}");
                        }
                    }
                    Err(error) => log::warn!("MCP listener error: {error}"),
                }
            }
        })
        .unwrap_or_else(|error| log::error!("MCP server thread could not start: {error}"));
}

fn load_or_create_token(path: &Path) -> std::io::Result<String> {
    if path.is_file() {
        let token = std::fs::read_to_string(path)?;
        let token = token.trim();
        if token.len() >= 32 && token.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(token.to_owned());
        }
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "existing token file is malformed; remove it to rotate the token"));
    }
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    let token = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    let mut file = options.open(path)?;
    file.write_all(token.as_bytes())?;
    file.sync_all()?;
    Ok(token)
}

#[cfg(test)]
mod http_tests {
    use super::*;

    #[test]
    fn token_comparison_is_constant_time_shape_and_checks_length() {
        assert!(constant_time_eq(b"abcdef", b"abcdef"));
        assert!(!constant_time_eq(b"abcdef", b"abcdeg"));
        assert!(!constant_time_eq(b"abc", b"abcdef"));
    }
}

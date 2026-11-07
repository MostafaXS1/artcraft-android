//! Bounded hand-off from Android background workers to FigCraft's live UI thread.
//!
//! Producers must never mutate FigApp or Session directly. They enqueue the upstream
//! ControlRequest here; AndroidShell drains it immediately before FigApp::ui, where
//! FigCraft's own control::drain performs validation and execution.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use egui::Context;

use figcraft_ui_egui::{ControlRequest, FigApp};

const MAX_PENDING_REQUESTS: usize = 64;

fn queue() -> &'static Mutex<VecDeque<ControlRequest>> {
    static QUEUE: OnceLock<Mutex<VecDeque<ControlRequest>>> = OnceLock::new();
    QUEUE.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn ui_context() -> &'static Mutex<Option<Context>> {
    static CONTEXT: OnceLock<Mutex<Option<Context>>> = OnceLock::new();
    CONTEXT.get_or_init(|| Mutex::new(None))
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
    let result = {
        let mut pending = queue()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        push_bounded(&mut pending, request, MAX_PENDING_REQUESTS)
    };
    if result.is_ok() {
        if let Some(ctx) = ui_context()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
        {
            ctx.request_repaint();
        }
    }
    result
}

/// Transfer pending requests to FigCraft on the UI thread.
///
/// The app's own control::drain then handles cancellation, execution and replies.
pub fn drain(app: &mut FigApp, ctx: &Context) {
    *ui_context()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(ctx.clone());
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
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use figcraft_mcp::{Backend, Server};
use serde_json::Value;

static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);
const MCP_PORT: u16 = 38_761;
const MAX_HTTP_HEADER_BYTES: usize = 16 * 1024;
const MAX_HTTP_BODY_BYTES: usize = 32 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

struct LiveBackend;

fn is_read_only_method(method: &str) -> bool {
    matches!(method,
        "engine.commands" | "document.inspect" | "node.get" | "ui.inspect" |
        "ui.render" | "ui.screenshot" | "window.list" | "comment.list")
}

struct ApprovalRequest {
    id: u64,
    command: String,
    params: Value,
    deadline: Instant,
    decision: std::sync::mpsc::Sender<bool>,
}

#[derive(Default)]
struct ApprovalState {
    queued: VecDeque<ApprovalRequest>,
    active: Option<ApprovalRequest>,
}

fn approvals() -> &'static Mutex<ApprovalState> {
    static APPROVALS: OnceLock<Mutex<ApprovalState>> = OnceLock::new();
    APPROVALS.get_or_init(|| Mutex::new(ApprovalState::default()))
}

static NEXT_APPROVAL_ID: AtomicU64 = AtomicU64::new(1);
const MAX_PENDING_APPROVALS: usize = 8;

fn request_mutation_approval(command: &str, params: Value) -> Result<(), String> {
    let (decision_tx, decision_rx) = std::sync::mpsc::channel();
    let mut state = approvals()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if state.queued.len() + (if state.active.is_some() { 1 } else { 0 }) >= MAX_PENDING_APPROVALS {
        return Err("too many pending MCP approvals; retry after resolving the current requests".into());
    }
    state.queued.push_back(ApprovalRequest {
        id: NEXT_APPROVAL_ID.fetch_add(1, Ordering::Relaxed),
        command: command.to_owned(),
        params,
        deadline: Instant::now() + REQUEST_TIMEOUT,
        decision: decision_tx,
    });
    drop(state);
    if let Some(ctx) = ui_context()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
    {
        ctx.request_repaint();
    }
    match decision_rx.recv_timeout(REQUEST_TIMEOUT) {
        Ok(true) => Ok(()),
        Ok(false) => Err("the user rejected this MCP command".into()),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err("MCP command approval timed out".into()),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err("MCP approval UI was closed".into()),
    }
}

/// Draws a blocking approval sheet over the live canvas. A mutation is not enqueued
/// into FigCraft's control queue until the user explicitly approves it.
pub fn approval_ui(ctx: &Context) {
    let view = {
        let mut state = approvals()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.active.is_none() {
            state.active = state.queued.pop_front();
        }
        state.active.as_ref().map(|request| (
            request.id,
            request.command.clone(),
            request.params.to_string(),
            request.deadline,
        ))
    };
    let Some((id, command, params, deadline)) = view else { return; };
    if Instant::now() >= deadline {
        let mut state = approvals()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.active.as_ref().is_some_and(|request| request.id == id) {
            if let Some(request) = state.active.take() {
                let _ = request.decision.send(false);
            }
        }
        return;
    }

    let mut decision = None;
    egui::Window::new("MCP command approval")
        .collapsible(false)
        .resizable(true)
        .default_width(420.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            ui.label("An external MCP client wants to change this document.");
            ui.separator();
            ui.label(format!("Command: {command}"));
            egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                ui.monospace(params.chars().take(6000).collect::<String>());
            });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Reject").clicked() { decision = Some(false); }
                if ui.button("Approve and run").clicked() { decision = Some(true); }
            });
        });

    if let Some(approved) = decision {
        let mut state = approvals()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.active.as_ref().is_some_and(|request| request.id == id) {
            if let Some(request) = state.active.take() {
                let _ = request.decision.send(approved);
            }
        }
    }
}

impl Backend for LiveBackend {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        if method == "engine.execute" {
            let command = params.get("command").and_then(Value::as_str)
                .ok_or("MCP engine.execute is missing a command name")?;
            let command_params = params.get("params").cloned().unwrap_or(Value::Null);
            request_mutation_approval(command, command_params)?;
        } else if !is_read_only_method(method) {
            return Err(format!("MCP method is not permitted by the Android bridge: {method}"));
        }
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

fn read_http_header_line(
    reader: &mut BufReader<TcpStream>,
    total_bytes: &mut usize,
) -> std::io::Result<Option<String>> {
    let mut bytes = Vec::new();
    let limit = (MAX_HTTP_HEADER_BYTES + 1) as u64;
    let read = (&mut *reader).take(limit).read_until(b'\n', &mut bytes)?;
    if read == 0 {
        return Ok(None);
    }
    *total_bytes += read;
    if *total_bytes > MAX_HTTP_HEADER_BYTES {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "HTTP headers too large"));
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

fn handle_http(stream: &mut TcpStream, server: &mut Server, token: &str) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    stream.set_write_timeout(Some(Duration::from_secs(130)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut header_bytes = 0;
    let request_line = match read_http_header_line(&mut reader, &mut header_bytes) {
        Ok(Some(line)) => line,
        Ok(None) => return Ok(()),
        Err(_) => return write_http(stream, "431 Request Header Fields Too Large", "text/plain", b"headers too large"),
    };
    let mut headers = std::collections::HashMap::<String, String>::new();
    loop {
        let line = match read_http_header_line(&mut reader, &mut header_bytes) {
            Ok(Some(line)) => line,
            Ok(None) => return Ok(()),
            Err(_) => return write_http(stream, "431 Request Header Fields Too Large", "text/plain", b"headers too large"),
        };
        if line == "\r\n" || line == "\n" { break; }
        let Some((name, value)) = line.split_once(':') else {
            return write_http(stream, "400 Bad Request", "text/plain", b"malformed HTTP header");
        };
        if headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned()).is_some() {
            return write_http(stream, "400 Bad Request", "text/plain", b"duplicate HTTP header");
        }
    }

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");
    let version = parts.next().unwrap_or("");
    if version != "HTTP/1.1" && version != "HTTP/1.0" {
        return write_http(stream, "400 Bad Request", "text/plain", b"invalid HTTP request line");
    }
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
        v.contains("application/json") || v.trim() == "*/*"
    });
    if !content_type_ok || !accept_ok {
        return write_http(stream, "415 Unsupported Media Type", "text/plain", b"expected JSON MCP request and application/json accept");
    }
    if headers.contains_key("transfer-encoding") {
        return write_http(stream, "400 Bad Request", "text/plain", b"chunked transfer encoding is not supported");
    }
    let length = headers.get("content-length")
        .and_then(|v| v.parse::<usize>().ok())
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "missing or invalid content-length"))?;
    if length == 0 || length > MAX_HTTP_BODY_BYTES {
        return write_http(stream, "413 Content Too Large", "text/plain", b"request body size is invalid");
    }
    if let Some(version) = headers.get("mcp-protocol-version") {
        if !figcraft_mcp::SUPPORTED_VERSIONS.contains(&version.as_str()) {
            return write_http(stream, "400 Bad Request", "text/plain", b"unsupported MCP protocol version");
        }
    } else if server.is_initialized() {
        return write_http(stream, "400 Bad Request", "text/plain", b"MCP-Protocol-Version header required after initialization");
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    let body_text = std::str::from_utf8(&body)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
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

#[cfg(test)]
mod access_tests {
    use super::*;

    #[test]
    fn backend_allows_only_read_only_control_methods() {
        assert!(is_read_only_method("document.inspect"));
        assert!(is_read_only_method("engine.commands"));
        assert!(is_read_only_method("ui.screenshot"));
        assert!(!is_read_only_method("engine.execute"));
        assert!(!is_read_only_method("ui.pointer"));
        assert!(!is_read_only_method("file.open"));
    }
}

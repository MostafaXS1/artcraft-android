//! Bounded hand-off from Android background workers to FigCraft's live UI thread.
//!
//! Producers must never mutate FigApp or Session directly. They enqueue the upstream
//! ControlRequest here; AndroidShell drains it immediately before FigApp::ui, where
//! FigCraft's own control::drain performs validation and execution.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use egui::Context;

use figcraft_ui_egui::{ControlRequest, FigApp, RequestGate};

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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use figcraft_mcp::{Backend, Server};
use serde_json::Value;

static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);
static MCP_GENERATION: AtomicU64 = AtomicU64::new(0);
static MCP_RUNNING: AtomicBool = AtomicBool::new(false);
static MCP_PRIVATE_DIR: OnceLock<std::path::PathBuf> = OnceLock::new();
static MCP_SETTINGS_OPEN: AtomicBool = AtomicBool::new(false);
static MCP_TEST_RUNNING: AtomicBool = AtomicBool::new(false);
static MCP_TEST_RESULT: OnceLock<Mutex<String>> = OnceLock::new();
const MCP_PORT: u16 = 38_761;
const MAX_HTTP_HEADER_BYTES: usize = 16 * 1024;
const MAX_HTTP_BODY_BYTES: usize = 32 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

struct LiveBackend;

fn is_read_only_method(method: &str) -> bool {
    matches!(method,
        "engine.commands" | "document.inspect" | "node.get" | "ui.inspect" |
        "ui.render" | "ui.screenshot" | "window.list")
}

fn command_uses_file_access(command: &str, params: &Value, depth: usize) -> bool {
    if command.starts_with("file.") {
        return true;
    }
    if command != "command.batch" {
        return false;
    }
    if depth >= 8 {
        return true;
    }
    let Some(commands) = params.get("commands").and_then(Value::as_array) else {
        return true;
    };
    commands.iter().any(|step| {
        let Some(command) = step.get("command").and_then(Value::as_str) else {
            return true;
        };
        let params = step.get("params").cloned().unwrap_or(Value::Null);
        command_uses_file_access(command, &params, depth + 1)
    })
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
const MAX_APPROVAL_PARAMS_BYTES: usize = 64 * 1024;

fn request_mutation_approval(command: &str, params: Value) -> Result<(), String> {
    if command.is_empty() || command.len() > 128 {
        return Err("MCP command name is empty or exceeds 128 bytes".into());
    }
    if !params.is_object() && !params.is_null() {
        return Err("MCP command parameters must be a JSON object or null".into());
    }
    if params.to_string().len() > MAX_APPROVAL_PARAMS_BYTES {
        return Err("MCP command parameters exceed the 64 KiB approval preview limit".into());
    }
    if command == "command.batch" {
        let count = params.get("commands").and_then(Value::as_array).map_or(0, |items| items.len());
        if count == 0 || count > 32 {
            return Err("MCP batches must contain between 1 and 32 commands".into());
        }
    }
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
    mcp_settings_ui(ctx);
    gemini_copilot_ui(ctx);
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
            if command_uses_file_access(command, &command_params, 0) {
                return Err("MCP file access is disabled in the Android bridge".into());
            }
            if command != "comment.list" {
                request_mutation_approval(command, command_params)?;
            }
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

fn filter_tool_list(response: &str) -> String {
    let Ok(mut value) = serde_json::from_str::<Value>(response) else {
        return response.to_owned();
    };
    if let Some(tools) = value.pointer_mut("/result/tools").and_then(Value::as_array_mut) {
        tools.retain(|tool| {
            !matches!(
                tool.get("name").and_then(Value::as_str).unwrap_or(""),
                "open_file" | "save_file" | "pointer" | "key" | "type_text" | "wait_for_changes"
            )
        });
    }
    value.to_string()
}

fn write_http_with_session(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
    session_id: &str,
) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nMcp-Session-Id: {session_id}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()
}

fn random_hex_token() -> std::io::Result<String> {
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn handle_http(
    stream: &mut TcpStream,
    server: &mut Server,
    token: &str,
    session_id: &mut Option<String>,
) -> std::io::Result<()> {
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
    if path != "/mcp" {
        return write_http(stream, "404 Not Found", "text/plain", b"unknown MCP endpoint");
    }
    // This endpoint is intentionally for native/local MCP clients only. Reject every
    // browser Origin rather than allowing DNS-rebinding requests from an untrusted site.
    if headers.contains_key("origin") {
        return write_http(stream, "403 Forbidden", "text/plain", b"browser-origin requests are not supported");
    }
    let auth_ok = headers.get("authorization")
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|provided| constant_time_eq(provided.as_bytes(), token.as_bytes()));
    if !auth_ok {
        return write_http(stream, "401 Unauthorized", "text/plain", b"bearer token required");
    }

    if method == "DELETE" {
        let matches_session = session_id.as_ref().is_some_and(|active| {
            headers.get("mcp-session-id").is_some_and(|provided| provided == active)
        });
        if !matches_session {
            return write_http(stream, "404 Not Found", "text/plain", b"unknown MCP session");
        }
        *server = Server::new(Box::new(LiveBackend));
        *session_id = None;
        return write_http(stream, "204 No Content", "text/plain", b"");
    }
    if method != "POST" {
        return write_http(stream, "405 Method Not Allowed", "text/plain", b"POST or DELETE /mcp only");
    }

    let content_type_ok = headers.get("content-type")
        .is_some_and(|value| value.to_ascii_lowercase().starts_with("application/json"));
    let accept_ok = headers.get("accept").is_some_and(|value| {
        value.contains("application/json") || value.trim() == "*/*"
    });
    if !content_type_ok || !accept_ok {
        return write_http(stream, "415 Unsupported Media Type", "text/plain", b"expected JSON MCP request and application/json accept");
    }
    if headers.contains_key("transfer-encoding") {
        return write_http(stream, "400 Bad Request", "text/plain", b"chunked transfer encoding is not supported");
    }
    let Some(raw_length) = headers.get("content-length") else {
        return write_http(stream, "411 Length Required", "text/plain", b"content-length is required");
    };
    let Ok(length) = raw_length.parse::<usize>() else {
        return write_http(stream, "400 Bad Request", "text/plain", b"invalid content-length");
    };
    if length == 0 || length > MAX_HTTP_BODY_BYTES {
        return write_http(stream, "413 Content Too Large", "text/plain", b"request body size is invalid");
    }
    let mut body = vec![0; length];
    if reader.read_exact(&mut body).is_err() {
        return write_http(stream, "400 Bad Request", "text/plain", b"request body is incomplete");
    }
    let body_text = match std::str::from_utf8(&body) {
        Ok(text) => text,
        Err(_) => return write_http(stream, "400 Bad Request", "text/plain", b"request body must be UTF-8"),
    };
    let request: Value = match serde_json::from_str(body_text) {
        Ok(request) => request,
        Err(_) => return write_http(stream, "400 Bad Request", "text/plain", b"invalid JSON-RPC request"),
    };
    let rpc_method = request.get("method").and_then(Value::as_str).unwrap_or("");
    if rpc_method.is_empty() {
        return write_http(stream, "400 Bad Request", "text/plain", b"JSON-RPC batch and response messages are not accepted");
    }

    let is_initialize = rpc_method == "initialize";
    match session_id.as_deref() {
        None if !is_initialize => {
            return write_http(stream, "400 Bad Request", "text/plain", b"send initialize before other MCP requests");
        }
        Some(active) => {
            if !headers.get("mcp-session-id").is_some_and(|provided| provided == active) {
                return write_http(stream, "404 Not Found", "text/plain", b"unknown or missing MCP session");
            }
            if !is_initialize && !headers.contains_key("mcp-protocol-version") {
                return write_http(stream, "400 Bad Request", "text/plain", b"MCP-Protocol-Version header required after initialize");
            }
        }
        None => {}
    }
    if let Some(version) = headers.get("mcp-protocol-version") {
        if !figcraft_mcp::SUPPORTED_VERSIONS.contains(&version.as_str()) {
            return write_http(stream, "400 Bad Request", "text/plain", b"unsupported MCP protocol version");
        }
    }

    match server.handle_line(body_text) {
        Some(response) => {
            let response_value = serde_json::from_str::<Value>(&response).unwrap_or(Value::Null);
            let initialize_succeeded = is_initialize && response_value.pointer("/result/protocolVersion").is_some();
            if initialize_succeeded && session_id.is_none() {
                match random_hex_token() {
                    Ok(id) => *session_id = Some(id),
                    Err(error) => {
                        log::error!("MCP session id generation failed: {error}");
                        return write_http(stream, "500 Internal Server Error", "text/plain", b"could not create MCP session");
                    }
                }
            }
            let response = if rpc_method == "tools/list" {
                filter_tool_list(&response)
            } else {
                response
            };
            if let Some(active) = session_id.as_deref() {
                write_http_with_session(stream, "200 OK", "application/json", response.as_bytes(), active)
            } else {
                write_http(stream, "200 OK", "application/json", response.as_bytes())
            }
        }
        None => {
            if let Some(active) = session_id.as_deref() {
                write_http_with_session(stream, "202 Accepted", "text/plain", b"", active)
            } else {
                write_http(stream, "202 Accepted", "text/plain", b"")
            }
        }
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
    // The settings launcher must know the app-private directory even while the server
    // is disabled. AndroidShell calls this once during startup.
    let _ = MCP_PRIVATE_DIR.set(private_dir.to_path_buf());
    let enabled = private_dir.join("mcp-server.enabled");
    if !enabled.is_file() { return; }
    // Prevent duplicate listener threads when startup and the settings toggle race.
    if MCP_RUNNING.swap(true, Ordering::SeqCst) { return; }
    let token_path = private_dir.join("mcp-server.token");
    let token = match load_or_create_token(&token_path) {
        Ok(token) => token,
        Err(error) => {
            MCP_RUNNING.store(false, Ordering::SeqCst);
            log::error!("MCP server could not load/create its local access token: {error}");
            return;
        }
    };
    let generation = MCP_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    if let Err(error) = std::thread::Builder::new()
        .name("figcraft-mcp-http".into())
        .spawn(move || {
            let listener = match TcpListener::bind(("127.0.0.1", MCP_PORT)) {
                Ok(listener) => listener,
                Err(error) => {
                    MCP_RUNNING.store(false, Ordering::SeqCst);
                    log::error!("MCP server could not bind 127.0.0.1:{MCP_PORT}: {error}");
                    return;
                }
            };
            if let Err(error) = listener.set_nonblocking(true) {
                MCP_RUNNING.store(false, Ordering::SeqCst);
                log::error!("MCP server could not enable nonblocking accept: {error}");
                return;
            }
            MCP_RUNNING.store(true, Ordering::SeqCst);
            log::info!("Local MCP Streamable HTTP endpoint enabled at http://127.0.0.1:{MCP_PORT}/mcp");
            let mut server = Server::new(Box::new(LiveBackend));
            let mut session_id = None;
            while MCP_GENERATION.load(Ordering::SeqCst) == generation {
                match listener.accept() {
                    Ok((mut stream, _peer)) => {
                        if let Err(error) = handle_http(&mut stream, &mut server, &token, &mut session_id) {
                            log::warn!("MCP HTTP request failed: {error}");
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    Err(error) => {
                        log::warn!("MCP listener error: {error}");
                        break;
                    }
                }
            }
            MCP_RUNNING.store(false, Ordering::SeqCst);
            log::info!("Local MCP server stopped");
        })
    {
        MCP_RUNNING.store(false, Ordering::SeqCst);
        log::error!("MCP server thread could not start: {error}");
    }
}


/// In-app MCP controls. The endpoint remains loopback-only and disabled until the user
/// explicitly enables it here. The token is shown only in this private app UI.
fn run_mcp_connection_test() {
    if MCP_TEST_RUNNING.swap(true, Ordering::SeqCst) { return; }
    let _ = std::thread::Builder::new().name("figcraft-mcp-self-test".into()).spawn(|| {
        let result = (|| -> Result<String, String> {
            if !MCP_RUNNING.load(Ordering::SeqCst) {
                return Err("MCP server is not running".into());
            }
            // Exercise the same MCP protocol dispatcher used by external clients.
            let mut protocol = Server::new(Box::new(LiveBackend));
            let init = protocol.handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"figcraft-self-test","version":"1.0"}}}"#)
                .ok_or("MCP initialize returned no response")?;
            let init_value: Value = serde_json::from_str(&init).map_err(|e| format!("Invalid initialize response: {e}"))?;
            if init_value.get("error").is_some() { return Err(format!("Initialize failed: {}", init_value["error"])); }
            let tools = protocol.handle_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#)
                .ok_or("MCP tools/list returned no response")?;
            let tools_value: Value = serde_json::from_str(&tools).map_err(|e| format!("Invalid tools/list response: {e}"))?;
            if tools_value.get("error").is_some() { return Err(format!("tools/list failed: {}", tools_value["error"])); }
            let count = tools_value.pointer("/result/tools").and_then(Value::as_array)
                .ok_or("tools/list response did not contain a tools array")?.len();
            if count == 0 { return Err("MCP connected but exposed zero tools".into()); }

            let mut backend = LiveBackend;
            backend.call("engine.commands", serde_json::json!({}))
                .map_err(|e| format!("Canvas command discovery failed: {e}"))?;
            backend.call("document.inspect", serde_json::json!({}))
                .map_err(|e| format!("Canvas inspection failed: {e}"))?;
            Ok(format!("PASS: initialize, tools/list ({count} tools), engine.commands, document.inspect"))
        })();
        let message = result.unwrap_or_else(|error| format!("FAIL: {error}"));
        *MCP_TEST_RESULT.get_or_init(|| Mutex::new(String::new())).lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = message;
        MCP_TEST_RUNNING.store(false, Ordering::SeqCst);
    });
}

fn mcp_settings_ui(ctx: &Context) {
    let Some(private_dir) = MCP_PRIVATE_DIR.get().cloned() else { return; };
    let marker = private_dir.join("mcp-server.enabled");
    let token_path = private_dir.join("mcp-server.token");
    let mut enabled = marker.is_file();
    let mut open = MCP_SETTINGS_OPEN.load(Ordering::Relaxed);
    egui::Area::new(egui::Id::new("figcraft_mcp_settings_launcher"))
        .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-16.0, -16.0))
        .show(ctx, |ui| {
            if ui.button(if open { "Close MCP" } else { "MCP Settings" }).clicked() {
                open = !open;
            }
        });
    egui::Window::new("MCP Server Settings")
        .open(&mut open)
        .default_width(360.0)
        .resizable(false)
        .show(ctx, |ui| {
            ui.label("Expose the current FigCraft canvas to a local MCP client.");
            ui.label("Only 127.0.0.1 is used. This server is not reachable over Wi-Fi/LAN.");
            ui.separator();
            if ui.checkbox(&mut enabled, "Enable local MCP server").changed() {
                if enabled {
                    match std::fs::OpenOptions::new().write(true).create(true).truncate(false).open(&marker) {
                        Ok(_) => start_mcp_if_enabled(&private_dir),
                        Err(error) => log::error!("Could not enable MCP server: {error}"),
                    }
                } else {
                    let _ = std::fs::remove_file(&marker);
                    stop_mcp_server();
                }
            }
            ui.label(format!("Status: {}", if MCP_RUNNING.load(Ordering::SeqCst) { "Running" } else if enabled { "Starting / unavailable" } else { "Disabled" }));
            ui.monospace(format!("http://127.0.0.1:{MCP_PORT}/mcp"));
            if ui.add_enabled(!MCP_TEST_RUNNING.load(Ordering::SeqCst), egui::Button::new("Test MCP Connection")).clicked() {
                *MCP_TEST_RESULT.get_or_init(|| Mutex::new(String::new())).lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = "Testing MCP handshake and live canvas access…".into();
                run_mcp_connection_test();
            }
            if MCP_TEST_RUNNING.load(Ordering::SeqCst) {
                ui.label("Test running… keep the document open.");
            }
            if let Some(result) = MCP_TEST_RESULT.get() {
                let result = result.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                if !result.is_empty() { ui.label(result.as_str()); }
            }
            if let Ok(token) = std::fs::read_to_string(&token_path) {
                ui.label("Bearer token (private to this device):");
                ui.label("•••••••••••••••• (hidden)");
                if ui.button("Copy token").clicked() { ctx.copy_text(token.trim().to_owned()); }
                if ui.button("Rotate token").clicked() {
                    stop_mcp_server();
                    let _ = std::fs::remove_file(&token_path);
                    if enabled { start_mcp_if_enabled(&private_dir); }
                }
            } else {
                ui.label("Enable the server to generate its access token.");
            }
            ui.small("Mutating MCP commands still require approval inside FigCraft.");
        });
    MCP_SETTINGS_OPEN.store(open, Ordering::Relaxed);
}

/// Stop accepting new MCP requests when the Android shell is dropped and cancel
/// requests that have not yet reached FigCraft's UI queue.
pub fn stop_mcp_server() {
    MCP_GENERATION.fetch_add(1, Ordering::SeqCst);
    let mut pending = queue()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for request in pending.drain(..) {
        request.state.cancel_if_open();
    }
    drop(pending);

    let mut state = approvals()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for request in state.queued.drain(..) {
        let _ = request.decision.send(false);
    }
    if let Some(request) = state.active.take() {
        let _ = request.decision.send(false);
    }
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
    let token = random_hex_token()?;
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

#[cfg(test)]
mod tool_filter_tests {
    use super::*;

    #[test]
    fn tool_list_hides_control_methods_not_supported_on_android() {
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {"tools": [
                {"name": "inspect_document"},
                {"name": "run_command"},
                {"name": "open_file"},
                {"name": "pointer"},
                {"name": "type_text"},
                {"name": "wait_for_changes"}
            ]}
        }).to_string();
        let filtered: Value = serde_json::from_str(&filter_tool_list(&response)).unwrap_or(Value::Null);
        let names: Vec<_> = filtered.pointer("/result/tools").and_then(Value::as_array)
            .map(|tools| tools.iter().filter_map(|tool| tool.get("name").and_then(Value::as_str)).collect())
            .unwrap_or_default();
        assert_eq!(names, vec!["inspect_document", "run_command"]);
    }
}

#[cfg(test)]
mod file_policy_tests {
    use super::*;

    #[test]
    fn blocks_file_commands_even_when_nested_in_a_batch() {
        assert!(command_uses_file_access("file.open", &Value::Null, 0));
        assert!(command_uses_file_access(
            "command.batch",
            &serde_json::json!({"commands": [
                {"command": "node.create", "params": {}},
                {"command": "file.save", "params": {"path": "/tmp/out.figcraft"}}
            ]}),
            0
        ));
        assert!(!command_uses_file_access("node.create", &Value::Null, 0));
    }
}


// -----------------------------------------------------------------------------
// Gemini Design Copilot (text-only MVP).
// API credentials are memory-only for this app session; they are never written
// to disk or logged. Keystore-backed persistence is intentionally a later step.
// -----------------------------------------------------------------------------

#[derive(Default)]
struct GeminiCopilotState {
    api_key: String,
    role: String,
    desired_output: String,
    context: String,
    prompt: String,
    response: String,
    status: String,
    busy: bool,
    open: bool,
}

fn gemini_copilot_state() -> &'static Mutex<GeminiCopilotState> {
    static STATE: OnceLock<Mutex<GeminiCopilotState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(GeminiCopilotState {
        role: "Graphic Designer, UI/UX Designer, Out-of-the-box Creative Consultant".into(),
        desired_output: "Critique and improve my design with practical color, layout, typography, asset, and UX recommendations".into(),
        status: "Ready. Add your Gemini API key to begin.".into(),
        ..GeminiCopilotState::default()
    }))
}

static GEMINI_COPILOT_OPEN: AtomicBool = AtomicBool::new(false);
static GEMINI_COPILOT_BUSY: AtomicBool = AtomicBool::new(false);

fn gemini_system_instruction(role: &str, desired_output: &str, context: &str) -> String {
    format!(r#"You are a professional AI copilot.
ROLE:
{role}

DESIRED DELIVERABLE:
{desired_output}

ADDITIONAL CONTEXT:
{context}

Adapt your expertise, terminology, workflow, depth, and output format to the selected role and deliverable. Be specific and actionable. Separate observed facts, assumptions, subjective preferences, and recommendations. If essential information is missing, state reasonable assumptions instead of inventing facts.

For visual design tasks, assess composition, hierarchy, alignment, spacing, contrast, consistency, readability, accessibility, and usability. Recommend contextual color palettes with exact HEX values and semantic roles where useful. Explain color psychology as a design heuristic, not a universal scientific guarantee. Use the Golden Ratio (1:1.618) only when it improves proportions or hierarchy; calculate dimensions from supplied canvas dimensions and never force it where usability or platform conventions matter. Give practical typography, asset dimensions, aspect ratios, and responsive guidance when relevant. Offer a coherent creative alternative when useful. Critique weak choices constructively; do not flatter by default.

IMPORTANT FIGCRAFT LIMITATION:
You are currently a text-only assistant. You cannot inspect the live canvas, read document layers, or apply edits. Never claim you have seen or changed the canvas. If the user asks for canvas edits, provide a conceptual, step-by-step recommendation only. Do not invent FigCraft command IDs or output executable code, shell commands, or file operations.
Return the format best suited to the requested deliverable. Avoid generic filler."#)
}

fn run_gemini_request(api_key: String, role: String, desired_output: String, context: String, prompt: String) {
    if GEMINI_COPILOT_BUSY.swap(true, Ordering::SeqCst) { return; }
    {
        let mut state = gemini_copilot_state().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        state.busy = true;
        state.status = "Connecting to Gemini…".into();
        state.response.clear();
    }
    let spawned = std::thread::Builder::new().name("figcraft-gemini-copilot".into()).spawn(move || {
        let result = (|| -> Result<String, String> {
            let client = reqwest::blocking::Client::builder()
                .connect_timeout(Duration::from_secs(15))
                .timeout(Duration::from_secs(90))
                .build()
                .map_err(|_| "Could not initialize the Gemini network client".to_owned())?;
            let instruction = gemini_system_instruction(&role, &desired_output, &context);
            let body = serde_json::json!({
                "systemInstruction": { "parts": [{ "text": instruction }] },
                "contents": [{ "role": "user", "parts": [{ "text": prompt }] }],
                "generationConfig": { "temperature": 0.7, "maxOutputTokens": 4096 }
            });
            let response = client.post("https://generativelanguage.googleapis.com/v1beta/models/gemini-3.8-flash:generateContent")
                .header("x-goog-api-key", api_key)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .json(&body)
                .send()
                .map_err(|error| {
                    if error.is_connect() {
                        format!("Could not connect to Gemini (DNS/TLS/network): {error}")
                    } else if error.is_timeout() {
                        format!("Gemini request timed out after the configured deadline: {error}")
                    } else if error.is_request() {
                        format!("Gemini request could not be sent: {error}")
                    } else {
                        format!("Gemini network error: {error}")
                    }
                })?;
            let status = response.status();
            if !status.is_success() {
                let detail = response.text().unwrap_or_default();
                let detail = detail.chars().take(700).collect::<String>();
                let reason = match status.as_u16() {
                    400 => "Gemini rejected the request. Check the request format.",
                    401 | 403 => "Gemini API key is invalid or does not have access.",
                    404 => "Gemini model or endpoint was not found. Check model availability for this API key.",
                    429 => "Gemini rate limit or quota reached. Try again later.",
                    500..=599 => "Gemini service is temporarily unavailable.",
                    _ => "Gemini request failed.",
                };
                return Err(if detail.trim().is_empty() {
                    format!("{reason} (HTTP {}).", status.as_u16())
                } else {
                    format!("{reason} (HTTP {}): {detail}", status.as_u16())
                });
            }
            let value: Value = response.json()
                .map_err(|_| "Gemini returned an unreadable response.".to_owned())?;
            let parts = value.pointer("/candidates/0/content/parts")
                .and_then(Value::as_array)
                .ok_or_else(|| "Gemini returned no text candidate. The request may have been blocked.".to_owned())?;
            let text = parts.iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>().join("\n");
            if text.trim().is_empty() { return Err("Gemini returned an empty response.".into()); }
            Ok(text)
        })();
        let mut state = gemini_copilot_state().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        match result {
            Ok(answer) => { state.response = answer; state.status = "Response received.".into(); }
            Err(error) => { state.status = error; }
        }
        state.busy = false;
        GEMINI_COPILOT_BUSY.store(false, Ordering::SeqCst);
        if let Some(ctx) = ui_context().lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref() {
            ctx.request_repaint();
        }
    });
    if spawned.is_err() {
        GEMINI_COPILOT_BUSY.store(false, Ordering::SeqCst);
        let mut state = gemini_copilot_state().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        state.busy = false;
        state.status = "Could not start the Gemini background worker.".into();
    }
}

/// Compact Android touch controls for common editing actions. The native desktop toolbar
/// remains available; this floating strip removes the need for keyboard shortcuts on phones.
pub fn mobile_controls_ui(app: &mut FigApp, ctx: &Context) {
    let screen = ctx.content_rect();
    if screen.width() < 520.0 || screen.height() < 520.0 {
        egui::Area::new(egui::Id::new("figcraft_android_touch_actions"))
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -104.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    egui::ScrollArea::horizontal()
                        .max_width((screen.width() - 24.0).max(220.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let has_selection = !app.session.selection().is_empty();
                                for (label, command, needs_selection) in [
                                    ("Undo", "edit.undo", false),
                                    ("Redo", "edit.redo", false),
                                    ("Copy", "edit.copy", true),
                                    ("Cut", "edit.cut", true),
                                    ("Duplicate", "edit.duplicate", true),
                                    ("Delete", "edit.delete", true),
                                    ("Group", "object.group", true),
                                    ("Ungroup", "object.ungroup", true),
                                ] {
                                    if ui.add_enabled(!needs_selection || has_selection, egui::Button::new(label).min_size(egui::vec2(58.0, 40.0))).clicked() {
                                        figcraft_ui_egui::menus::invoke(app, command);
                                    }
                                }
                                if ui.button("Actions").min_size(egui::vec2(68.0, 40.0)).clicked() {
                                    figcraft_ui_egui::menus::toggle_palette(&mut app.ui);
                                }
                            });
                        });
                });
            });
    }
}

fn gemini_copilot_ui(ctx: &Context) {
    let mut state = gemini_copilot_state().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut open = state.open || GEMINI_COPILOT_OPEN.load(Ordering::Relaxed);
    let mut pending_request: Option<(String, String, String, String, String)> = None;
    egui::Area::new(egui::Id::new("figcraft_gemini_copilot_launcher"))
        .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-16.0, -58.0))
        .show(ctx, |ui| {
            if ui.button(if open { "Close Copilot" } else { "Gemini Copilot" }).clicked() {
                open = !open;
            }
        });
    if open {
        let screen = ctx.content_rect();
        let panel_width = (screen.width() * 0.46).clamp(300.0, 480.0).min((screen.width() - 24.0).max(240.0));
        let panel_height = (screen.height() - 24.0).max(180.0);
        let panel_pos = egui::pos2(
            (screen.max.x - panel_width - 12.0).max(screen.min.x + 12.0),
            screen.min.y + 12.0,
        );
        egui::Window::new("Gemini Design Copilot")
            .open(&mut open)
            .default_pos(panel_pos)
            .default_width(panel_width)
            .default_height(panel_height.min(560.0))
            .max_size(egui::vec2((screen.width() - 24.0).max(240.0), panel_height))
            .resizable(true)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height((panel_height - 32.0).max(140.0))
                    .show(ui, |ui| {
                ui.label("Configure Gemini for the task. Role and deliverable are required.");
                ui.label("Gemini API key (kept in memory for this session only)");
                ui.add(egui::TextEdit::singleline(&mut state.api_key).password(true).hint_text("Paste API key"));
                ui.separator();
                ui.label("Sebagai apa? *");
                ui.add(egui::TextEdit::singleline(&mut state.role).hint_text("Contoh: Senior UI/UX Designer"));
                ui.label("Ingin hasilkan apa? *");
                ui.add(egui::TextEdit::multiline(&mut state.desired_output).desired_rows(2).hint_text("Deliverable yang ingin dibuat"));
                ui.label("Konteks tambahan (opsional)");
                ui.add(egui::TextEdit::multiline(&mut state.context).desired_rows(2).hint_text("Platform, audiens, ukuran canvas, brand, batasan…"));
                ui.separator();
                ui.label("Instruksi / brief");
                ui.add(egui::TextEdit::multiline(&mut state.prompt).desired_rows(4).hint_text("Jelaskan pekerjaan yang ingin dibantu Gemini…"));
                let ready = !state.api_key.trim().is_empty()
                    && !state.role.trim().is_empty()
                    && !state.desired_output.trim().is_empty()
                    && !state.prompt.trim().is_empty()
                    && !state.busy;
                if ui.add_enabled(ready, egui::Button::new(if state.busy { "Generating…" } else { "Generate with Gemini" })).clicked() {
                    let values = (
                        state.api_key.trim().to_owned(), state.role.trim().to_owned(),
                        state.desired_output.trim().to_owned(), state.context.trim().to_owned(),
                        state.prompt.trim().to_owned()
                    );
                    pending_request = Some(values);
                }
                ui.label(&state.status);
                if !state.response.is_empty() {
                    ui.separator();
                    ui.label("Gemini response");
                    egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                        ui.label(egui::RichText::new(&state.response).text_style(egui::TextStyle::Body));
                    });
                    if ui.button("Copy response").clicked() { ctx.copy_text(state.response.clone()); }
                }
                ui.separator();
                ui.small("MVP: text recommendations only. Canvas inspection, action-plan validation, preview, Apply, and undo integration are not implemented yet. API key is not saved and will be lost when the app closes.");
                    });
            });
    }
    state.open = open;
    GEMINI_COPILOT_OPEN.store(open, Ordering::Relaxed);
    // Never start a worker while holding the UI state mutex: the worker launcher
    // also acquires that mutex to set its busy/status fields.
    drop(state);
    if let Some((key, role, output, context, prompt)) = pending_request {
        run_gemini_request(key, role, output, context, prompt);
    }
}

//! Local loopback HTTP dashboard over real Aegis Platform APIs.
//!
//! The server is deliberately small and dependency-free: a bounded HTTP/1.1
//! parser over `std::net`, exact-match routes, and no path parameters.
//! Responses never include the owner token. The session token is a local
//! anti-CSRF capability shown once in the page and required for mutations.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::actions;
use crate::error::UiError;
use crate::html::INDEX_HTML;
use crate::model;

const MAX_HEADER_BYTES: usize = 16 * 1024;
const MAX_BODY_BYTES: usize = 64 * 1024;
const MAX_HEADERS: usize = 100;
const SOCKET_TIMEOUT: Duration = Duration::from_secs(5);
const ACCEPT_POLL: Duration = Duration::from_millis(10);

const SECURITY_HEADERS: &str = "Cache-Control: no-store\r\n\
X-Content-Type-Options: nosniff\r\n\
Referrer-Policy: no-referrer\r\n\
X-Frame-Options: DENY\r\n\
Content-Security-Policy: default-src 'none'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'\r\n\
Connection: close\r\n";

/// Options for the local dashboard server.
#[derive(Debug, Clone)]
pub struct DashboardOptions {
    /// Loopback bind address. Defaults to `127.0.0.1:0`.
    pub bind_addr: SocketAddr,
    /// When false, mutation actions are rejected and hidden in the page.
    pub allow_mutations: bool,
}

impl Default for DashboardOptions {
    fn default() -> Self {
        Self {
            bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            allow_mutations: false,
        }
    }
}

/// Marker type for the local GUI dashboard.
#[derive(Debug, Clone, Copy, Default)]
pub struct DashboardServer;

impl DashboardServer {
    pub fn bind(root: impl AsRef<Path>, options: DashboardOptions) -> Result<DashboardHandle> {
        bind(root, options)
    }
}

/// Bind a loopback HTTP dashboard for `root`.
pub fn bind(root: impl AsRef<Path>, options: DashboardOptions) -> Result<DashboardHandle> {
    if !options.bind_addr.ip().is_loopback() {
        return Err(UiError::NonLoopback(options.bind_addr));
    }
    let listener = TcpListener::bind(options.bind_addr)?;
    let addr = listener.local_addr()?;
    Ok(DashboardHandle {
        listener,
        root: root.as_ref().to_path_buf(),
        allow_mutations: options.allow_mutations,
        session_token: generate_session_token(),
        addr,
    })
}

/// A bound dashboard server.
pub struct DashboardHandle {
    listener: TcpListener,
    root: PathBuf,
    allow_mutations: bool,
    session_token: String,
    addr: SocketAddr,
}

impl DashboardHandle {
    /// Actual bound address, including the chosen ephemeral port.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Local session token. Display-only; never log it.
    pub fn session_token(&self) -> &str {
        &self.session_token
    }

    /// Serve until `shutdown` becomes true.
    pub fn serve_until(&self, shutdown: Arc<AtomicBool>) -> Result<()> {
        self.listener.set_nonblocking(true)?;
        while !shutdown.load(Ordering::SeqCst) {
            match self.listener.accept() {
                Ok((stream, _peer)) => {
                    let _ = self.handle_connection(stream);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(ACCEPT_POLL);
                }
                Err(error) => return Err(UiError::Io(error)),
            }
        }
        Ok(())
    }

    /// Serve forever, or until the listener fails.
    pub fn run(&self) -> Result<()> {
        self.serve_until(Arc::new(AtomicBool::new(false)))
    }

    fn handle_connection(&self, mut stream: TcpStream) -> std::io::Result<()> {
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(SOCKET_TIMEOUT))?;
        stream.set_write_timeout(Some(SOCKET_TIMEOUT))?;
        let response = match read_request(&mut stream) {
            Ok(request) => self.route(&request),
            Err(ParseError::TooLarge) => {
                text_response(413, "Payload Too Large", "request body too large")
            }
            Err(ParseError::BadRequest) => text_response(400, "Bad Request", "malformed request"),
        };
        write_response(&mut stream, &response)
    }

    fn route(&self, request: &HttpRequest) -> HttpResponse {
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/") => HttpResponse::new(200, "OK", "text/html; charset=utf-8", {
                self.index_html().into_bytes()
            }),
            ("GET", "/api/health") => json_response(200, "OK", health_value()),
            ("GET", "/api/status") => match model::collect_status(&self.root) {
                Ok(status) => match serde_json::to_value(status) {
                    Ok(value) => json_response(200, "OK", value),
                    Err(_) => api_error(500, "Internal Server Error", "status_unavailable"),
                },
                Err(_) => api_error(503, "Service Unavailable", "status_unavailable"),
            },
            ("GET", "/api/cores") => {
                let cores = model::collect_cores(&self.root);
                let values: Vec<Value> = cores.iter().map(|core| core.to_json()).collect();
                json_response(200, "OK", Value::Array(values))
            }
            ("GET", "/api/diagnostics") => match model::collect_diagnostics(&self.root) {
                Ok(report) => match serde_json::to_value(report) {
                    Ok(value) => json_response(200, "OK", value),
                    Err(_) => api_error(500, "Internal Server Error", "diagnostics_unavailable"),
                },
                Err(_) => api_error(503, "Service Unavailable", "diagnostics_unavailable"),
            },
            ("GET", "/api/modules") => {
                json_response(200, "OK", model::collect_modules(&self.root).to_json())
            }
            ("POST", "/api/actions") => {
                let (status, value) = actions::handle_action(
                    &self.root,
                    self.allow_mutations,
                    &self.session_token,
                    &request.headers,
                    &request.body,
                );
                json_response(status, reason_for(status), value)
            }
            ("GET", _) | ("POST", _) => text_response(404, "Not Found", "not found"),
            _ => text_response(405, "Method Not Allowed", "method not allowed"),
        }
    }

    fn index_html(&self) -> String {
        let token_meta = if self.allow_mutations {
            format!(
                "<meta name=\"aegis-session-token\" content=\"{}\">",
                self.session_token
            )
        } else {
            String::new()
        };
        let mutations = if self.allow_mutations {
            "true"
        } else {
            "false"
        };
        INDEX_HTML
            .replace("__AEGIS_TOKEN_META__", &token_meta)
            .replace("__AEGIS_MUTATIONS__", mutations)
    }
}

type Result<T> = std::result::Result<T, UiError>;

struct HttpRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

enum ParseError {
    BadRequest,
    TooLarge,
}

#[derive(Debug)]
struct HttpResponse {
    status: u16,
    reason: &'static str,
    content_type: &'static str,
    body: Vec<u8>,
}

impl HttpResponse {
    fn new(status: u16, reason: &'static str, content_type: &'static str, body: Vec<u8>) -> Self {
        Self {
            status,
            reason,
            content_type,
            body,
        }
    }
}

fn json_response(status: u16, reason: &'static str, value: Value) -> HttpResponse {
    match serde_json::to_vec(&value) {
        Ok(body) => HttpResponse::new(status, reason, "application/json", body),
        Err(_) => text_response(500, "Internal Server Error", "serialization failed"),
    }
}

fn api_error(status: u16, reason: &'static str, error: &'static str) -> HttpResponse {
    json_response(status, reason, json!({"ok": false, "error": error}))
}

fn text_response(status: u16, reason: &'static str, message: &'static str) -> HttpResponse {
    HttpResponse::new(status, reason, "text/plain; charset=utf-8", {
        message.as_bytes().to_vec()
    })
}

fn reason_for(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        409 => "Conflict",
        503 => "Service Unavailable",
        _ => "Internal Server Error",
    }
}

fn write_response(stream: &mut TcpStream, response: &HttpResponse) -> std::io::Result<()> {
    let header = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\n{}\r\n",
        response.status,
        response.reason,
        response.content_type,
        response.body.len(),
        SECURITY_HEADERS
    );
    stream.write_all(header.as_bytes())?;
    stream.write_all(&response.body)?;
    stream.flush()
}

fn read_request(stream: &mut TcpStream) -> std::result::Result<HttpRequest, ParseError> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        if let Some(position) = find_subsequence(&buffer, b"\r\n\r\n") {
            break position;
        }
        if buffer.len() > MAX_HEADER_BYTES {
            return Err(ParseError::BadRequest);
        }
        match stream.read(&mut chunk) {
            Ok(0) => return Err(ParseError::BadRequest),
            Ok(count) => buffer.extend_from_slice(&chunk[..count]),
            Err(_) => return Err(ParseError::BadRequest),
        }
    };
    if header_end > MAX_HEADER_BYTES {
        return Err(ParseError::BadRequest);
    }

    let header_text =
        std::str::from_utf8(&buffer[..header_end]).map_err(|_| ParseError::BadRequest)?;
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().ok_or(ParseError::BadRequest)?;
    let mut parts = request_line.split(' ');
    let method = parts.next().ok_or(ParseError::BadRequest)?.to_owned();
    let target = parts.next().ok_or(ParseError::BadRequest)?;
    let version = parts.next().ok_or(ParseError::BadRequest)?;
    if parts.next().is_some() {
        return Err(ParseError::BadRequest);
    }
    if method != "GET" && method != "POST" {
        return Err(ParseError::BadRequest);
    }
    if !version.starts_with("HTTP/1.") {
        return Err(ParseError::BadRequest);
    }
    if !target.starts_with('/') {
        return Err(ParseError::BadRequest);
    }
    let path = target.split('?').next().unwrap_or(target).to_owned();

    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (name, value) = line.split_once(':').ok_or(ParseError::BadRequest)?;
        if headers.len() >= MAX_HEADERS {
            return Err(ParseError::BadRequest);
        }
        headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
    }
    if headers.iter().any(|(name, _)| name == "transfer-encoding") {
        return Err(ParseError::BadRequest);
    }
    let content_length = match headers.iter().find(|(name, _)| name == "content-length") {
        Some((_, value)) => value.parse::<usize>().map_err(|_| ParseError::BadRequest)?,
        None => 0,
    };
    if content_length > MAX_BODY_BYTES {
        return Err(ParseError::TooLarge);
    }

    let body_start = header_end + 4;
    let already_read = buffer.len().saturating_sub(body_start);
    let take = already_read.min(content_length);
    let mut body = Vec::with_capacity(content_length);
    body.extend_from_slice(&buffer[body_start..body_start + take]);
    while body.len() < content_length {
        let remaining = content_length - body.len();
        let mut read_buffer = [0u8; 4096];
        let want = remaining.min(read_buffer.len());
        match stream.read(&mut read_buffer[..want]) {
            Ok(0) => return Err(ParseError::BadRequest),
            Ok(count) => body.extend_from_slice(&read_buffer[..count]),
            Err(_) => return Err(ParseError::BadRequest),
        }
    }

    Ok(HttpRequest {
        method,
        path,
        headers,
        body,
    })
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn health_value() -> Value {
    json!({
        "ok": true,
        "schema": "aegis.ui.health.v1",
        "generated_at": chrono::Utc::now().to_rfc3339(),
    })
}

fn generate_session_token() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let pid = u64::from(std::process::id());
    let thread = {
        let mut hasher = DefaultHasher::new();
        std::thread::current().id().hash(&mut hasher);
        hasher.finish()
    };
    let marker = Box::new(0u8);
    let address = &*marker as *const u8 as usize as u64;

    let mut token = String::with_capacity(32);
    for seed in [0u64, 0x9e37_79b9_7f4a_7c15u64] {
        let mut hasher = DefaultHasher::new();
        seed.hash(&mut hasher);
        now.hash(&mut hasher);
        pid.hash(&mut hasher);
        counter.hash(&mut hasher);
        thread.hash(&mut hasher);
        address.hash(&mut hasher);
        let value = hasher.finish();
        token.push_str(&format!("{value:016x}"));
    }
    token
}

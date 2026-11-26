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

use aegis_contracts::core::validate_core_id;
use aegis_contracts::personal::OutfitDecisionRecord;
use aegis_core_install::CoreInstaller;
use aegis_core_registry::CoreRegistry;
use aegis_core_runtime::CoreRuntime;
use aegis_diagnostics::Diagnostics;
use aegis_modules::ModuleRegistry;
use aegis_storage::StoragePaths;
use serde_json::{json, Value};

use crate::actions;
use crate::error::UiError;
use crate::html::{
    render_core_workspace, render_core_workspace_error, render_sections, INDEX_HTML,
};
use crate::model;

const MAX_HEADER_BYTES: usize = 16 * 1024;
const MAX_BODY_BYTES: usize = 64 * 1024;
const MAX_HEADERS: usize = 100;
const MAX_TARGET_BYTES: usize = 2 * 1024;
const MAX_ERROR_DETAIL_CHARS: usize = 512;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SectionStatus {
    Available,
    Partial,
    Stale,
    Unavailable,
}

impl SectionStatus {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Partial => "partial",
            Self::Stale => "stale",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct WorkspaceSection {
    pub(crate) title: &'static str,
    pub(crate) status: SectionStatus,
    pub(crate) note: Option<String>,
    pub(crate) rows: Vec<(String, String)>,
    pub(crate) items: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct CoreWorkspaceView {
    pub(crate) core_uuid: String,
    pub(crate) core_number: u8,
    pub(crate) display_name: String,
    pub(crate) lifecycle_state: String,
    pub(crate) generated_at: String,
    pub(crate) sections: Vec<WorkspaceSection>,
}

impl CoreWorkspaceView {
    /// JSON parity for the server-rendered Core workspace.
    ///
    /// Mirrors exactly the fields and section content rendered as HTML, so both
    /// surfaces expose the same real backend data and the same UNAVAILABLE
    /// notes. No values are synthesized here.
    pub(crate) fn to_json(&self) -> Value {
        let sections: Vec<Value> = self
            .sections
            .iter()
            .map(|section| {
                let rows: Vec<Value> = section
                    .rows
                    .iter()
                    .map(|(label, value)| json!({"label": label, "value": value}))
                    .collect();
                json!({
                    "title": section.title,
                    "status": section.status.as_str(),
                    "note": section.note,
                    "rows": rows,
                    "items": section.items,
                })
            })
            .collect();
        json!({
            "ok": true,
            "schema": "aegis.ui.core_workspace.v1",
            "core_uuid": self.core_uuid,
            "core_number": self.core_number,
            "display_name": self.display_name,
            "lifecycle_state": self.lifecycle_state,
            "generated_at": self.generated_at,
            "sections": sections,
        })
    }
}

enum WorkspaceError {
    InvalidCoreId,
    NotFound,
    Backend(String),
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
            ("GET", "/api/status") => {
                let dashboard = model::DashboardModel::collect(&self.root);
                let Some(platform) = dashboard.platform.as_ref() else {
                    return api_error(503, "Service Unavailable", "status_unavailable");
                };
                let mut value = match serde_json::to_value(platform) {
                    Ok(value) => value,
                    Err(_) => return api_error(500, "Internal Server Error", "status_unavailable"),
                };
                if let Value::Object(map) = &mut value {
                    map.insert("snapshots".to_owned(), snapshot_states_json(&dashboard));
                }
                json_response(200, "OK", value)
            }
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
            ("GET", "/api/core") | ("GET", "/api/core/") => {
                core_api_error(400, "Bad Request", "invalid_core_uuid", None)
            }
            ("GET", path) if path.starts_with("/api/core/") => {
                let raw_core_uuid = path.trim_start_matches("/api/core/").trim_end_matches('/');
                match build_core_workspace(&self.root, raw_core_uuid) {
                    Ok(view) => json_response(200, "OK", view.to_json()),
                    Err(WorkspaceError::InvalidCoreId) => {
                        core_api_error(400, "Bad Request", "invalid_core_uuid", None)
                    }
                    Err(WorkspaceError::NotFound) => {
                        core_api_error(404, "Not Found", "core_not_found", None)
                    }
                    Err(WorkspaceError::Backend(message)) => core_api_error(
                        503,
                        "Service Unavailable",
                        "core_workspace_unavailable",
                        Some(&message),
                    ),
                }
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
            ("GET", path) if path.starts_with("/core/") => {
                let raw_core_uuid = path.trim_start_matches("/core/").trim_end_matches('/');
                match build_core_workspace(&self.root, raw_core_uuid) {
                    Ok(view) => html_response(200, "OK", render_core_workspace(&view)),
                    Err(WorkspaceError::InvalidCoreId) => html_response(
                        400,
                        "Bad Request",
                        render_core_workspace_error(
                            "Invalid Core identifier",
                            "The requested identifier is not a valid Core UUID.",
                        ),
                    ),
                    Err(WorkspaceError::NotFound) => html_response(
                        404,
                        "Not Found",
                        render_core_workspace_error(
                            "Core not found",
                            "No registered Core matches this identifier.",
                        ),
                    ),
                    Err(WorkspaceError::Backend(message)) => html_response(
                        503,
                        "Service Unavailable",
                        render_core_workspace_error(
                            "Core workspace unavailable",
                            &format!("The backend could not serve this Core workspace: {message}"),
                        ),
                    ),
                }
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
        let dashboard = model::DashboardModel::collect(&self.root);
        let mut status_sections = snapshot_sections(&dashboard);
        status_sections.extend(world_and_personal_sections(&self.root, &dashboard));
        INDEX_HTML
            .replace("__AEGIS_TOKEN_META__", &token_meta)
            .replace("__AEGIS_MUTATIONS__", mutations)
            .replace(
                "__AEGIS_STATUS_SECTIONS__",
                &render_sections(&status_sections),
            )
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

fn html_response(status: u16, reason: &'static str, body: String) -> HttpResponse {
    HttpResponse::new(
        status,
        reason,
        "text/html; charset=utf-8",
        body.into_bytes(),
    )
}

fn api_error(status: u16, reason: &'static str, error: &'static str) -> HttpResponse {
    json_response(status, reason, json!({"ok": false, "error": error}))
}

/// Typed JSON error for the Core workspace API.
///
/// `code` is a stable machine-readable identifier. `detail`, when present, is
/// a real backend message, length-bounded and safely JSON-serialized. It never
/// contains owner or session tokens.
fn core_api_error(
    status: u16,
    reason: &'static str,
    code: &'static str,
    detail: Option<&str>,
) -> HttpResponse {
    let mut value = json!({"ok": false, "error": code});
    if let Some(detail) = detail {
        value["detail"] = Value::String(bound_detail(detail));
    }
    json_response(status, reason, value)
}

fn bound_detail(detail: &str) -> String {
    if detail.chars().count() > MAX_ERROR_DETAIL_CHARS {
        detail.chars().take(MAX_ERROR_DETAIL_CHARS).collect()
    } else {
        detail.to_owned()
    }
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
    if target.len() > MAX_TARGET_BYTES {
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

fn build_core_workspace(
    root: &Path,
    raw_core_uuid: &str,
) -> std::result::Result<CoreWorkspaceView, WorkspaceError> {
    let core_uuid = validate_core_id(raw_core_uuid)
        .map(|uuid| uuid.hyphenated().to_string())
        .map_err(|_| WorkspaceError::InvalidCoreId)?;
    let paths =
        StoragePaths::new(root).map_err(|error| WorkspaceError::Backend(error.to_string()))?;
    let registry = CoreRegistry::new(paths.clone());
    let entry = registry
        .get(&core_uuid)
        .map_err(|error| WorkspaceError::Backend(error.to_string()))?
        .ok_or(WorkspaceError::NotFound)?;

    let inspection = CoreInstaller::new(root)
        .map_err(|error| error.to_string())
        .and_then(|installer| {
            installer
                .inspect(&core_uuid)
                .map_err(|error| error.to_string())
        });
    let runtime = CoreRuntime::new(paths.clone()).status(&core_uuid).ok();
    let diagnostics = Diagnostics::new(root)
        .ok()
        .map(|diagnostics| diagnostics.collect());
    let core_report = diagnostics.as_ref().and_then(|report| {
        report
            .cores
            .entries
            .iter()
            .find(|core| core.core_uuid == core_uuid)
    });
    let (module_records, modules_error) = match ModuleRegistry::new(paths).list() {
        Ok(records) => (records, None),
        Err(error) => (Vec::new(), Some(error.to_string())),
    };
    let activity = registry.lifecycle_history(&core_uuid);

    let mut identity_rows = vec![
        ("Core UUID".to_owned(), entry.core_uuid.clone()),
        ("Core number".to_owned(), entry.core_number.to_string()),
        ("Display name".to_owned(), entry.display_name.clone()),
        (
            "Lifecycle state".to_owned(),
            entry.lifecycle_state.as_str().to_owned(),
        ),
        ("Registry schema".to_owned(), entry.schema.clone()),
        (
            "Storage path".to_owned(),
            entry.storage_relative_path.clone(),
        ),
        ("Initialized at".to_owned(), entry.initialized_at.clone()),
        ("Updated at".to_owned(), entry.updated_at.clone()),
    ];
    let mut identity_notes = Vec::new();
    match inspection {
        Ok(inspection) => {
            identity_rows.push((
                "Identity schema".to_owned(),
                inspection.identity.schema.clone(),
            ));
            identity_rows.push((
                "Identity revision".to_owned(),
                inspection.identity.identity_revision.to_string(),
            ));
            identity_rows.push((
                "Identity created at".to_owned(),
                inspection.identity.created_at.clone(),
            ));
            identity_rows.push((
                "Database instance".to_owned(),
                inspection.metadata.database_instance_uuid.clone(),
            ));
            identity_rows.push((
                "Schema revision".to_owned(),
                inspection.metadata.schema_revision.to_string(),
            ));
            identity_rows.push((
                "Initialization mode".to_owned(),
                inspection.metadata.initialization_mode.clone(),
            ));
            identity_rows.push((
                "Integrity status".to_owned(),
                inspection.integrity.status.as_str().to_owned(),
            ));
            if let Some(detail) = &inspection.integrity.detail {
                identity_rows.push(("Integrity detail".to_owned(), detail.clone()));
            }
        }
        Err(error) => {
            identity_notes.push(format!("Core identity inspection failed: {error}"));
        }
    }

    let memory_section = unavailable_section(
        "Memory",
        "UNAVAILABLE: no memory data source is linked to aegis-ui. Core memory tables and aegis-cognition are not dependencies of this crate.",
    );
    let personality_section = unavailable_section(
        "Personality",
        "UNAVAILABLE: no personality data source is linked to aegis-ui. aegis-core-profiles is not a dependency of this crate.",
    );
    let voice_section = unavailable_section(
        "Voice",
        "UNAVAILABLE: no voice data source is linked to aegis-ui. aegis-embodiment and aegis-core-profiles are not dependencies of this crate.",
    );
    let appearance_section = WorkspaceSection {
        title: "Appearance",
        status: SectionStatus::Unavailable,
        note: Some(
            "Per-Core theme data is UNAVAILABLE: no appearance data source (aegis-embodiment or aegis-core-profiles) is linked. The standard Aegis palette below is applied to this dashboard only."
                .to_owned(),
        ),
        rows: vec![
            ("Primary".to_owned(), "#9400D3".to_owned()),
            ("Accent".to_owned(), "#00C2FF".to_owned()),
            ("Tertiary".to_owned(), "#39FF14".to_owned()),
            ("Background".to_owned(), "#000000".to_owned()),
        ],
        items: Vec::new(),
    };

    let modules_section = match &modules_error {
        Some(error) => unavailable_section(
            "Modules",
            &format!("Module registry is UNAVAILABLE: {error}"),
        ),
        None => {
            let items = module_records
                .iter()
                .map(|module| {
                    let permissions = if module.permissions.is_empty() {
                        "no permissions declared".to_owned()
                    } else {
                        format!("permissions: {}", module.permissions.join(", "))
                    };
                    format!(
                        "{} {} - {} ({permissions})",
                        module.name,
                        module.version,
                        module.lifecycle_state.as_str()
                    )
                })
                .collect::<Vec<_>>();
            let note = if items.is_empty() {
                "Platform-wide module registry is available; no modules are installed.".to_owned()
            } else {
                "Platform-wide module registry; per-Core module bindings are not provided by the linked APIs."
                    .to_owned()
            };
            WorkspaceSection {
                title: "Modules",
                status: SectionStatus::Available,
                note: Some(note),
                rows: Vec::new(),
                items,
            }
        }
    };

    let permission_items = module_records
        .iter()
        .flat_map(|module| {
            module
                .permissions
                .iter()
                .map(move |permission| format!("{}: {permission}", module.name))
        })
        .collect::<Vec<_>>();
    let permissions_section = match &modules_error {
        Some(error) => unavailable_section(
            "Permissions",
            &format!("Module permission registry is UNAVAILABLE: {error}"),
        ),
        None => WorkspaceSection {
            title: "Permissions",
            status: SectionStatus::Partial,
            note: Some(if permission_items.is_empty() {
                "Per-Core permission grants are UNAVAILABLE (no Core permission data source is linked). No platform-wide module permissions are declared."
                    .to_owned()
            } else {
                "Platform-wide declared module permissions; per-Core permission grants are UNAVAILABLE (no Core permission data source is linked)."
                    .to_owned()
            }),
            rows: Vec::new(),
            items: permission_items,
        },
    };

    let mut diagnostics_rows = Vec::new();
    let mut diagnostics_notes = Vec::new();
    let mut diagnostics_available = false;
    match &diagnostics {
        Some(report) => {
            diagnostics_available = true;
            diagnostics_rows.push((
                "Platform diagnostics status".to_owned(),
                report.status.as_str().to_owned(),
            ));
            diagnostics_rows.push((
                "Report generated at".to_owned(),
                report.generated_at.clone(),
            ));
        }
        None => {
            diagnostics_notes.push("Diagnostics report is UNAVAILABLE for this Core.".to_owned())
        }
    }
    match core_report {
        Some(core) => {
            diagnostics_available = true;
            diagnostics_rows.push(("Core database path".to_owned(), core.database_path.clone()));
            if let Some(bytes) = core.database_bytes {
                diagnostics_rows.push(("Core database bytes".to_owned(), bytes.to_string()));
            }
            diagnostics_rows.push((
                "Diagnostics integrity".to_owned(),
                core.integrity.status.clone(),
            ));
            if let Some(detail) = &core.integrity.detail {
                diagnostics_rows.push(("Diagnostics integrity detail".to_owned(), detail.clone()));
            }
        }
        None if diagnostics.is_some() => {
            diagnostics_notes
                .push("This Core is not present in the diagnostics report.".to_owned());
        }
        None => {}
    }
    match &runtime {
        Some(runtime) => {
            diagnostics_available = true;
            diagnostics_rows.push((
                "Runtime state".to_owned(),
                runtime.state.as_str().to_owned(),
            ));
            diagnostics_rows.push((
                "Heartbeat fresh".to_owned(),
                runtime.heartbeat_fresh.to_string(),
            ));
            if let Some(at) = &runtime.heartbeat_at {
                diagnostics_rows.push(("Heartbeat at".to_owned(), at.clone()));
            }
            if let Some(age) = runtime.heartbeat_age_ms {
                diagnostics_rows.push(("Heartbeat age (ms)".to_owned(), age.to_string()));
            }
            if let Some(pid) = runtime.pid {
                diagnostics_rows.push(("Runtime PID".to_owned(), pid.to_string()));
            }
            diagnostics_rows.push((
                "Runtime generation".to_owned(),
                runtime.generation.to_string(),
            ));
            if let Some(at) = &runtime.started_at {
                diagnostics_rows.push(("Runtime started at".to_owned(), at.clone()));
            }
            diagnostics_rows.push(("Runtime updated at".to_owned(), runtime.updated_at.clone()));
            if let Some(executable) = &runtime.host_executable {
                diagnostics_rows.push(("Host executable".to_owned(), executable.clone()));
            }
            if let Some(error) = &runtime.last_error {
                diagnostics_rows.push(("Runtime last error".to_owned(), error.clone()));
            }
        }
        None => diagnostics_notes.push("Runtime status is UNAVAILABLE for this Core.".to_owned()),
    }
    let diagnostics_status = if diagnostics_available {
        if diagnostics_notes.is_empty() {
            SectionStatus::Available
        } else {
            SectionStatus::Partial
        }
    } else {
        SectionStatus::Unavailable
    };
    let diagnostics_section = WorkspaceSection {
        title: "Diagnostics",
        status: diagnostics_status,
        note: if diagnostics_notes.is_empty() {
            None
        } else {
            Some(diagnostics_notes.join(" "))
        },
        rows: diagnostics_rows,
        items: Vec::new(),
    };

    let activity_section = match activity {
        Ok(changes) => {
            let items = changes
                .iter()
                .map(|change| {
                    format!(
                        "{} -> {} at {} (reason: {})",
                        change.from.as_str(),
                        change.to.as_str(),
                        change.changed_at,
                        change.reason
                    )
                })
                .collect::<Vec<_>>();
            WorkspaceSection {
                title: "Activity",
                status: SectionStatus::Available,
                note: Some(if items.is_empty() {
                    "No lifecycle changes are recorded for this Core.".to_owned()
                } else {
                    "Append-only Core lifecycle history from the Platform registry.".to_owned()
                }),
                rows: Vec::new(),
                items,
            }
        }
        Err(error) => unavailable_section(
            "Activity",
            &format!("Lifecycle history is UNAVAILABLE: {error}"),
        ),
    };

    let identity_section = WorkspaceSection {
        title: "Identity",
        status: if identity_notes.is_empty() {
            SectionStatus::Available
        } else {
            SectionStatus::Partial
        },
        note: if identity_notes.is_empty() {
            None
        } else {
            Some(identity_notes.join(" "))
        },
        rows: identity_rows,
        items: Vec::new(),
    };

    let dashboard = model::DashboardModel::collect(root);
    let mut sections = vec![
        identity_section,
        memory_section,
        personality_section,
        voice_section,
        appearance_section,
        modules_section,
        permissions_section,
        diagnostics_section,
        activity_section,
    ];
    sections.extend(snapshot_sections(&dashboard));
    sections.extend(world_and_personal_sections(root, &dashboard));

    Ok(CoreWorkspaceView {
        core_uuid,
        core_number: entry.core_number,
        display_name: entry.display_name,
        lifecycle_state: entry.lifecycle_state.as_str().to_owned(),
        generated_at: chrono::Utc::now().to_rfc3339(),
        sections,
    })
}

fn unavailable_section(title: &'static str, note: &str) -> WorkspaceSection {
    WorkspaceSection {
        title,
        status: SectionStatus::Unavailable,
        note: Some(note.to_owned()),
        rows: Vec::new(),
        items: Vec::new(),
    }
}

/// Render the four platform-status snapshot sections shared by the dashboard
/// index and the per-Core workspace.
///
/// A section only renders snapshot values when the loader reports a fresh
/// snapshot. Missing, malformed, and stale snapshots render an explicit
/// `UNAVAILABLE`/`STALE` reason; no success is inferred from absent data.
fn snapshot_sections(dashboard: &model::DashboardModel) -> Vec<WorkspaceSection> {
    vec![
        fleet_section(dashboard),
        gpu_section(dashboard),
        inference_section(dashboard),
        service_section(dashboard),
    ]
}

fn snapshot_state_name(state: &model::SnapshotState) -> &'static str {
    match state {
        model::SnapshotState::Fresh => "fresh",
        model::SnapshotState::Stale(_) => "stale",
        model::SnapshotState::Missing(_) | model::SnapshotState::Malformed(_) => "unavailable",
    }
}

fn snapshot_section_status(state: &model::SnapshotState) -> SectionStatus {
    match state {
        model::SnapshotState::Fresh => SectionStatus::Available,
        model::SnapshotState::Stale(_) => SectionStatus::Stale,
        model::SnapshotState::Missing(_) | model::SnapshotState::Malformed(_) => {
            SectionStatus::Unavailable
        }
    }
}

/// Machine-readable snapshot states with explicit reasons for `/api/status`.
fn snapshot_states_json(dashboard: &model::DashboardModel) -> Value {
    let state = |snapshot: &model::SnapshotState| {
        json!({
            "state": snapshot_state_name(snapshot),
            "reason": snapshot.reason(),
        })
    };
    json!({
        "schema": "aegis.ui.status_snapshots.v1",
        "fleet": state(&dashboard.fleet_snapshot),
        "gpu": state(&dashboard.gpu_snapshot),
        "inference": state(&dashboard.inference_snapshot),
        "service": state(&dashboard.service_snapshot),
        "world": state(&dashboard.world_snapshot),
        "personal": state(&dashboard.personal_snapshot),
    })
}

fn snapshot_fallback_section(
    title: &'static str,
    state: &model::SnapshotState,
) -> WorkspaceSection {
    let status = snapshot_section_status(state);
    let note = match state {
        model::SnapshotState::Fresh => {
            "UNAVAILABLE: snapshot is fresh but no data view was produced".to_owned()
        }
        model::SnapshotState::Stale(reason) => format!("STALE: {reason}"),
        model::SnapshotState::Missing(reason) | model::SnapshotState::Malformed(reason) => {
            format!("UNAVAILABLE: {reason}")
        }
    };
    WorkspaceSection {
        title,
        status,
        note: Some(note),
        rows: Vec::new(),
        items: Vec::new(),
    }
}

fn fleet_section(dashboard: &model::DashboardModel) -> WorkspaceSection {
    let Some(fleet) = &dashboard.fleet else {
        return snapshot_fallback_section("Fleet", &dashboard.fleet_snapshot);
    };
    let reachable = fleet.hosts.iter().filter(|host| host.reachable).count();
    let items = fleet
        .hosts
        .iter()
        .map(|host| {
            let mut line = format!(
                "{}: {} (models: {}, latency: {} ms)",
                host.host_id,
                if host.reachable {
                    "reachable"
                } else {
                    "unreachable"
                },
                host.model_count,
                host.latency_ms
            );
            if let Some(error) = &host.error {
                line.push_str(&format!("; error: {error}"));
            }
            line
        })
        .collect::<Vec<_>>();
    WorkspaceSection {
        title: "Fleet",
        status: SectionStatus::Available,
        note: Some(format!(
            "Live fleet status snapshot: {} host(s), {reachable} reachable.",
            fleet.hosts.len()
        )),
        rows: Vec::new(),
        items,
    }
}

fn gpu_section(dashboard: &model::DashboardModel) -> WorkspaceSection {
    let Some(gpu) = &dashboard.gpu else {
        return snapshot_fallback_section("GPU", &dashboard.gpu_snapshot);
    };
    if !gpu.available {
        return WorkspaceSection {
            title: "GPU",
            status: SectionStatus::Unavailable,
            note: Some(format!(
                "UNAVAILABLE: {}",
                gpu.error
                    .clone()
                    .unwrap_or_else(|| "GPU reported unavailable by status snapshot".to_owned())
            )),
            rows: Vec::new(),
            items: Vec::new(),
        };
    }
    let mut rows = Vec::new();
    if let Some(name) = &gpu.name {
        rows.push(("GPU".to_owned(), name.clone()));
    }
    if let Some(utilization) = gpu.utilization_percent {
        rows.push(("Utilization (%)".to_owned(), utilization.to_string()));
    }
    if let Some(free_vram) = gpu.free_vram_mb {
        rows.push(("Free VRAM (MB)".to_owned(), free_vram.to_string()));
    }
    if let Some(temperature) = gpu.temperature_c {
        rows.push(("Temperature (C)".to_owned(), temperature.to_string()));
    }
    WorkspaceSection {
        title: "GPU",
        status: SectionStatus::Available,
        note: Some("Live GPU telemetry snapshot.".to_owned()),
        rows,
        items: Vec::new(),
    }
}

fn inference_section(dashboard: &model::DashboardModel) -> WorkspaceSection {
    let Some(inference) = &dashboard.inference else {
        return snapshot_fallback_section("Inference", &dashboard.inference_snapshot);
    };
    let mut rows = vec![("Active".to_owned(), inference.active.to_string())];
    if let Some(model_name) = &inference.model {
        rows.push(("Model".to_owned(), model_name.clone()));
    }
    if let Some(host_id) = &inference.host_id {
        rows.push(("Host".to_owned(), host_id.clone()));
    }
    if let Some(detail) = &inference.detail {
        rows.push(("Detail".to_owned(), detail.clone()));
    }
    WorkspaceSection {
        title: "Inference",
        status: SectionStatus::Available,
        note: Some(if inference.active {
            "Live inference status snapshot.".to_owned()
        } else {
            "Snapshot reports no active inference.".to_owned()
        }),
        rows,
        items: Vec::new(),
    }
}

fn service_section(dashboard: &model::DashboardModel) -> WorkspaceSection {
    let Some(service) = &dashboard.service else {
        return snapshot_fallback_section("Service", &dashboard.service_snapshot);
    };
    let mut rows = vec![
        ("Running".to_owned(), service.running.to_string()),
        ("PID alive".to_owned(), service.pid_alive.to_string()),
        (
            "Socket reachable".to_owned(),
            service.socket_reachable.to_string(),
        ),
    ];
    if let Some(pid) = service.pid {
        rows.insert(1, ("PID".to_owned(), pid.to_string()));
    }
    if let Some(error) = &service.error {
        rows.push(("Error".to_owned(), error.clone()));
    }
    WorkspaceSection {
        title: "Service",
        status: if service.error.is_some() {
            SectionStatus::Partial
        } else {
            SectionStatus::Available
        },
        note: Some("Live platform service status snapshot.".to_owned()),
        rows,
        items: Vec::new(),
    }
}

/// World plus Personal/Emotion/Continuity/Outfit sections from optional
/// operator/CLI snapshots, reusing the shared model loader.
fn world_and_personal_sections(
    root: &Path,
    dashboard: &model::DashboardModel,
) -> Vec<WorkspaceSection> {
    vec![
        world_section(dashboard),
        personal_section(dashboard),
        emotion_section(dashboard),
        continuity_section(dashboard),
        outfit_section(root, dashboard),
    ]
}

fn world_section(dashboard: &model::DashboardModel) -> WorkspaceSection {
    let Some(world) = &dashboard.world else {
        return snapshot_fallback_section("World", &dashboard.world_snapshot);
    };
    let mut rows = Vec::new();
    if let Some(local_time) = &world.local_time {
        rows.push(("Local time".to_owned(), local_time.clone()));
    }
    if let Some(timezone) = &world.timezone {
        rows.push(("Timezone".to_owned(), timezone.clone()));
    }
    if let Some(time_of_day) = &world.time_of_day {
        rows.push(("Time of day".to_owned(), time_of_day.clone()));
    }
    if let Some(season) = &world.season {
        rows.push(("Season".to_owned(), season.clone()));
    }
    if let Some(condition) = &world.condition {
        rows.push(("Condition".to_owned(), condition.clone()));
    }
    if let Some(temperature) = world.temperature_celsius {
        rows.push(("Temperature (C)".to_owned(), temperature.to_string()));
    }
    let (status, note) = if rows.is_empty() {
        (
            SectionStatus::Partial,
            "World status snapshot is fresh but reports no world fields.".to_owned(),
        )
    } else {
        (
            SectionStatus::Available,
            "World snapshot from the operator/CLI status channel.".to_owned(),
        )
    };
    WorkspaceSection {
        title: "World",
        status,
        note: Some(note),
        rows,
        items: Vec::new(),
    }
}

fn personal_section(dashboard: &model::DashboardModel) -> WorkspaceSection {
    let Some(personal) = &dashboard.personal else {
        return snapshot_fallback_section("Personal", &dashboard.personal_snapshot);
    };
    let mut rows = Vec::new();
    if let Some(core_uuid) = &personal.core_uuid {
        rows.push(("Core UUID".to_owned(), core_uuid.clone()));
    }
    if let Some(count) = personal.activity_count {
        rows.push(("Activities".to_owned(), count.to_string()));
    }
    if let Some(count) = personal.routine_count {
        rows.push(("Routines".to_owned(), count.to_string()));
    }
    let (status, note) = if rows.is_empty() {
        (
            SectionStatus::Partial,
            "Personal status snapshot is fresh but reports no personal fields.".to_owned(),
        )
    } else {
        (
            SectionStatus::Available,
            "Personal state from the operator/CLI status channel.".to_owned(),
        )
    };
    WorkspaceSection {
        title: "Personal",
        status,
        note: Some(note),
        rows,
        items: Vec::new(),
    }
}

fn emotion_section(dashboard: &model::DashboardModel) -> WorkspaceSection {
    let Some(personal) = &dashboard.personal else {
        return snapshot_fallback_section("Emotion", &dashboard.personal_snapshot);
    };
    let Some(emotion) = &personal.emotion else {
        return invalid_snapshot_section(
            "Emotion",
            "Personal status snapshot reports no emotion data",
        );
    };
    WorkspaceSection {
        title: "Emotion",
        status: SectionStatus::Available,
        note: Some("Emotion dimensions from the operator/CLI personal snapshot.".to_owned()),
        rows: vec![("Emotion".to_owned(), emotion.clone())],
        items: Vec::new(),
    }
}

fn continuity_section(dashboard: &model::DashboardModel) -> WorkspaceSection {
    let Some(personal) = &dashboard.personal else {
        return snapshot_fallback_section("Continuity", &dashboard.personal_snapshot);
    };
    let Some(continuity) = &personal.continuity else {
        return invalid_snapshot_section(
            "Continuity",
            "Personal status snapshot reports no continuity data",
        );
    };
    WorkspaceSection {
        title: "Continuity",
        status: SectionStatus::Available,
        note: Some("Continuity summary from the operator/CLI personal snapshot.".to_owned()),
        rows: vec![("Continuity".to_owned(), continuity.clone())],
        items: Vec::new(),
    }
}

/// Outfit decisions are not part of the shared model view, so they are read
/// directly from the same fresh personal snapshot. The snapshot state still
/// comes from the model loader; a stale or missing file never becomes fresh.
fn outfit_section(root: &Path, dashboard: &model::DashboardModel) -> WorkspaceSection {
    if !dashboard.personal_snapshot.is_fresh() {
        return snapshot_fallback_section("Outfit", &dashboard.personal_snapshot);
    }
    let path = root.join(model::PERSONAL_STATUS_SNAPSHOT_PATH);
    let value: Value = match std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    {
        Some(value) => value,
        None => {
            return invalid_snapshot_section(
                "Outfit",
                "Personal status snapshot could not be re-read for outfit data",
            )
        }
    };
    let Some(outfit) = value.get("outfit") else {
        return invalid_snapshot_section(
            "Outfit",
            "Personal status snapshot does not include an 'outfit' object",
        );
    };
    if let Some(summary) = outfit.as_str() {
        return WorkspaceSection {
            title: "Outfit",
            status: SectionStatus::Available,
            note: Some("Outfit summary from the operator/CLI personal snapshot.".to_owned()),
            rows: vec![("Outfit".to_owned(), summary.to_owned())],
            items: Vec::new(),
        };
    }
    let record: OutfitDecisionRecord = match serde_json::from_value(outfit.clone()) {
        Ok(record) => record,
        Err(error) => {
            return invalid_snapshot_section(
                "Outfit",
                format!("Outfit decision failed to deserialize: {error}"),
            )
        }
    };
    if let Err(error) = record.validate() {
        return invalid_snapshot_section(
            "Outfit",
            format!("Outfit decision failed validation: {error}"),
        );
    }
    let mut rows = vec![
        ("Decision id".to_owned(), record.id.clone()),
        ("Decided at".to_owned(), record.decided_at.clone()),
    ];
    if !record.rationale.is_empty() {
        rows.push(("Rationale".to_owned(), record.rationale.clone()));
    }
    WorkspaceSection {
        title: "Outfit",
        status: SectionStatus::Available,
        note: Some("Latest outfit decision from the operator/CLI personal snapshot.".to_owned()),
        rows,
        items: record.asset_ids,
    }
}

fn invalid_snapshot_section(title: &'static str, reason: impl Into<String>) -> WorkspaceSection {
    WorkspaceSection {
        title,
        status: SectionStatus::Unavailable,
        note: Some(format!("UNAVAILABLE: {}", reason.into())),
        rows: Vec::new(),
        items: Vec::new(),
    }
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

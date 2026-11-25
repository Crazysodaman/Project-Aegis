use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_core_install::CoreInstaller;
use aegis_core_runtime::CoreRuntime;
use aegis_diagnostics::Diagnostics;
use aegis_platform::PlatformService;
use aegis_safe::SafeStore;
use aegis_storage::{now_rfc3339, StoragePaths};
use aegis_ui::{bind, render_dashboard, DashboardHandle, DashboardOptions};
use serde_json::Value;
use tempfile::TempDir;

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_B: &str = "11111111-1111-4111-8111-111111111111";

struct Fixture {
    root: TempDir,
    owner_token: String,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("tempdir");
        let initialized = PlatformService::initialize(root.path()).expect("platform init");
        Self {
            root,
            owner_token: initialized.owner_token,
        }
    }

    fn path(&self) -> &Path {
        self.root.path()
    }

    fn installer(&self) -> CoreInstaller {
        CoreInstaller::new(self.path()).expect("installer")
    }

    fn add_core(&self, core_uuid: &str, core_number: u8, display_name: &str) {
        let request = CoreInitializationRequest {
            schema: CORE_INIT_REQUEST_SCHEMA.to_owned(),
            core_uuid: core_uuid.to_owned(),
            core_number,
            display_name: display_name.to_owned(),
            requested_at: now_rfc3339().expect("timestamp"),
        };
        self.installer()
            .initialize(&request, &self.owner_token)
            .expect("initialize core");
    }
}

struct Server {
    handle: Arc<DashboardHandle>,
    shutdown: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    fn start(root: &Path, allow_mutations: bool) -> Self {
        let options = DashboardOptions {
            bind_addr: "127.0.0.1:0".parse().expect("loopback address"),
            allow_mutations,
        };
        let handle = Arc::new(bind(root, options).expect("bind dashboard"));
        let shutdown = Arc::new(AtomicBool::new(false));
        let serving = Arc::clone(&handle);
        let stop = Arc::clone(&shutdown);
        let thread = std::thread::spawn(move || {
            serving.serve_until(stop).expect("serve dashboard");
        });
        Self {
            handle,
            shutdown,
            thread: Some(thread),
        }
    }

    fn addr(&self) -> SocketAddr {
        self.handle.addr()
    }

    fn token(&self) -> String {
        self.handle.session_token().to_owned()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Response {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Response {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).expect("JSON body")
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header == name)
            .map(|(_, value)| value.as_str())
    }
}

fn parse_response(bytes: &[u8]) -> Response {
    let split = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("HTTP header terminator");
    let head = std::str::from_utf8(&bytes[..split]).expect("HTTP header UTF-8");
    let body = String::from_utf8_lossy(&bytes[split + 4..]).into_owned();
    let mut lines = head.split("\r\n");
    let status_line = lines.next().expect("status line");
    let status = status_line
        .split_whitespace()
        .nth(1)
        .expect("status code")
        .parse::<u16>()
        .expect("numeric status");
    let headers = lines
        .filter_map(|line| {
            line.split_once(':')
                .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        })
        .collect();
    Response {
        status,
        headers,
        body,
    }
}

fn raw_request(addr: SocketAddr, raw: &[u8]) -> Response {
    let mut stream = TcpStream::connect(addr).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("read timeout");
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .expect("write timeout");
    stream.write_all(raw).expect("write request");
    stream.flush().expect("flush request");
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).expect("read response");
    parse_response(&bytes)
}

fn get(addr: SocketAddr, path: &str) -> Response {
    let raw = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    raw_request(addr, raw.as_bytes())
}

fn post(addr: SocketAddr, path: &str, token: Option<&str>, body: &str) -> Response {
    let mut head = format!(
        "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if let Some(token) = token {
        head.push_str(&format!("X-Aegis-Token: {token}\r\n"));
    }
    head.push_str("\r\n");
    let mut raw = head.into_bytes();
    raw.extend_from_slice(body.as_bytes());
    raw_request(addr, &raw)
}

#[test]
fn root_page_is_self_contained_and_never_leaks_owner_token() {
    let fixture = Fixture::new();
    let server = Server::start(fixture.path(), false);
    let response = get(server.addr(), "/");
    assert_eq!(response.status, 200);
    assert_eq!(
        response.header("content-type"),
        Some("text/html; charset=utf-8")
    );
    for color in ["#9400D3", "#00C2FF", "#39FF14"] {
        assert!(response.body.contains(color), "missing color {color}");
    }
    assert!(!response.body.contains(&fixture.owner_token));
    assert!(!response.body.contains(&server.token()));
    assert_eq!(response.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(response.header("cache-control"), Some("no-store"));
    assert!(response.header("content-security-policy").is_some());
    assert!(response.header("access-control-allow-origin").is_none());
}

#[test]
fn session_token_is_embedded_only_when_mutations_are_enabled() {
    let fixture = Fixture::new();
    let readonly = Server::start(fixture.path(), false);
    let readonly_token = readonly.token();
    let readonly_body = get(readonly.addr(), "/").body;
    assert!(!readonly_body.contains(&readonly_token));
    drop(readonly);

    let mutable = Server::start(fixture.path(), true);
    let mutable_token = mutable.token();
    let body = get(mutable.addr(), "/").body;
    assert!(body.contains("aegis-session-token"));
    assert!(body.contains(&mutable_token));
    assert!(!body.contains(&fixture.owner_token));
}

#[test]
fn status_reports_zero_core_then_two_real_cores() {
    let zero = Fixture::new();
    let server = Server::start(zero.path(), false);
    let status = get(server.addr(), "/api/status").json();
    assert_eq!(status["zero_core"].as_bool(), Some(true));
    assert_eq!(status["core_count"].as_u64(), Some(0));
    drop(server);

    let two = Fixture::new();
    two.add_core(CORE_A, 1, "Core A");
    two.add_core(CORE_B, 2, "Core B");
    let server = Server::start(two.path(), false);
    let status = get(server.addr(), "/api/status").json();
    assert_eq!(status["zero_core"].as_bool(), Some(false));
    assert_eq!(status["core_count"].as_u64(), Some(2));
}

#[test]
fn cores_endpoint_returns_two_real_cores_with_runtime_and_integrity() {
    let fixture = Fixture::new();
    fixture.add_core(CORE_A, 1, "Core A");
    fixture.add_core(CORE_B, 2, "Core B");
    let server = Server::start(fixture.path(), false);
    let cores = get(server.addr(), "/api/cores").json();
    let array = cores.as_array().expect("cores array");
    assert_eq!(array.len(), 2);
    let uuids: Vec<&str> = array
        .iter()
        .filter_map(|core| core["core_uuid"].as_str())
        .collect();
    assert!(uuids.contains(&CORE_A));
    assert!(uuids.contains(&CORE_B));
    for core in array {
        assert_eq!(core["lifecycle_state"], "installed");
        assert!(core["runtime"]["state"].is_string());
        assert!(core["runtime"]["heartbeat_fresh"].is_boolean());
        assert_eq!(core["integrity"]["status"], "verified");
    }
    assert!(!cores.to_string().contains(&fixture.owner_token));
}

#[test]
fn diagnostics_endpoint_matches_direct_collect() {
    let fixture = Fixture::new();
    fixture.add_core(CORE_A, 1, "Core A");
    fixture.add_core(CORE_B, 2, "Core B");
    let server = Server::start(fixture.path(), false);
    let http = get(server.addr(), "/api/diagnostics").json();
    let direct = Diagnostics::new(fixture.path())
        .expect("diagnostics")
        .collect();
    assert_eq!(http["schema"], direct.schema);
    assert_eq!(http["status"], direct.status.as_str());
    assert_eq!(
        http["platform"]["zero_core"].as_bool(),
        direct.platform.zero_core
    );
    assert_eq!(
        http["platform"]["core_count"].as_u64(),
        direct.platform.core_count.map(u64::from)
    );
    assert_eq!(
        http["cores"]["count"].as_u64(),
        Some(direct.cores.count as u64)
    );
    assert_eq!(
        http["reasons"],
        serde_json::to_value(&direct.reasons).expect("reasons")
    );
}

#[test]
fn modules_endpoint_returns_json() {
    let fixture = Fixture::new();
    let server = Server::start(fixture.path(), false);
    let response = get(server.addr(), "/api/modules");
    assert_eq!(response.status, 200);
    let value = response.json();
    assert!(value["modules"].is_array());
    assert!(value["status"].is_string());
}

#[test]
fn action_without_session_token_is_forbidden_and_has_no_effect() {
    let fixture = Fixture::new();
    fixture.add_core(CORE_A, 1, "Core A");
    let server = Server::start(fixture.path(), true);
    let body = serde_json::json!({
        "action": "core.deactivate",
        "core_uuid": CORE_A,
        "reason": "must not apply"
    })
    .to_string();
    let response = post(server.addr(), "/api/actions", None, &body);
    assert_eq!(response.status, 403);
    assert_eq!(response.json()["ok"].as_bool(), Some(false));
    let entry = fixture
        .installer()
        .get(CORE_A)
        .expect("registry get")
        .expect("registered core");
    assert_eq!(entry.lifecycle_state.as_str(), "installed");
}

#[test]
fn token_with_mutations_disabled_is_forbidden() {
    let fixture = Fixture::new();
    let server = Server::start(fixture.path(), false);
    let token = server.token();
    let body = serde_json::json!({
        "action": "safe.stop.set",
        "reason": "must not apply"
    })
    .to_string();
    let response = post(server.addr(), "/api/actions", Some(&token), &body);
    assert_eq!(response.status, 403);
    let safe = SafeStore::new(StoragePaths::new(fixture.path()).expect("storage paths"));
    assert!(!safe.stop_status().expect("stop status").active);
}

#[test]
fn authorized_safe_stop_set_actually_engages_stop() {
    let fixture = Fixture::new();
    let server = Server::start(fixture.path(), true);
    let token = server.token();
    let body = serde_json::json!({
        "action": "safe.stop.set",
        "reason": "dashboard integration test"
    })
    .to_string();
    let response = post(server.addr(), "/api/actions", Some(&token), &body);
    assert_eq!(response.status, 200);
    let value = response.json();
    assert_eq!(value["ok"].as_bool(), Some(true));
    assert_eq!(value["result"]["active"].as_bool(), Some(true));
    let safe = SafeStore::new(StoragePaths::new(fixture.path()).expect("storage paths"));
    let stop = safe.stop_status().expect("stop status");
    assert!(stop.active);
    assert_eq!(stop.updated_by, "aegis-dashboard");
    assert!(!response.body.contains(&fixture.owner_token));
}

#[test]
fn authorized_core_start_never_fakes_success() {
    let fixture = Fixture::new();
    fixture.add_core(CORE_A, 1, "Core A");
    let server = Server::start(fixture.path(), true);
    let token = server.token();
    let body = serde_json::json!({
        "action": "core.start",
        "core_uuid": CORE_A
    })
    .to_string();
    let response = post(server.addr(), "/api/actions", Some(&token), &body);
    assert!(response.status == 200 || response.status == 409);
    assert!(!response.body.contains(&fixture.owner_token));
    let value = response.json();
    let ok = value["ok"].as_bool().expect("ok flag");
    let paths = StoragePaths::new(fixture.path()).expect("storage paths");
    let runtime = CoreRuntime::new(paths);
    if ok {
        assert_eq!(value["result"]["state"], "running");
        let status = runtime.status(CORE_A).expect("runtime status");
        assert!(status.heartbeat_fresh);
        assert!(status.pid.is_some());
        let _ = runtime.stop(CORE_A, &fixture.owner_token);
    } else {
        assert!(value["result"].is_null());
        assert!(value["error"]
            .as_str()
            .is_some_and(|error| !error.is_empty()));
    }
}

#[test]
fn oversized_request_body_is_rejected() {
    let fixture = Fixture::new();
    let server = Server::start(fixture.path(), true);
    let raw = format!(
        "POST /api/actions HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        70_000
    );
    let response = raw_request(server.addr(), raw.as_bytes());
    assert!(response.status == 413 || response.status == 400);
}

#[test]
fn malformed_request_is_rejected() {
    let fixture = Fixture::new();
    let server = Server::start(fixture.path(), false);
    let response = raw_request(server.addr(), b"GARBAGE\r\n\r\n");
    assert_eq!(response.status, 400);
}

#[test]
fn non_loopback_bind_is_rejected() {
    let fixture = Fixture::new();
    let options = DashboardOptions {
        bind_addr: "0.0.0.0:0".parse().expect("non-loopback address"),
        allow_mutations: false,
    };
    assert!(matches!(
        bind(fixture.path(), options),
        Err(aegis_ui::UiError::NonLoopback(_))
    ));
}

#[test]
fn terminal_dashboard_contains_real_ids_and_ansi_colors() {
    let fixture = Fixture::new();
    fixture.add_core(CORE_A, 1, "Core A");
    fixture.add_core(CORE_B, 2, "Core B");
    let rendered = render_dashboard(fixture.path());
    assert!(rendered.contains(CORE_A));
    assert!(rendered.contains(CORE_B));
    assert!(rendered.contains("\x1b[2J"));
    assert!(rendered.contains("\x1b[38;2;148;0;211m"));
    assert!(rendered.contains("\x1b[38;2;0;194;255m"));
    assert!(rendered.contains("\x1b[38;2;57;255;20m"));
    assert!(rendered.contains("STOP"));
    assert!(!rendered.contains(&fixture.owner_token));
}

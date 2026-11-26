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

#[test]
fn core_workspace_renders_required_sections_with_real_backend_data() {
    let fixture = Fixture::new();
    fixture.add_core(CORE_A, 1, "Core A");
    let server = Server::start(fixture.path(), false);
    let response = get(server.addr(), &format!("/core/{CORE_A}"));
    assert_eq!(response.status, 200);
    assert_eq!(
        response.header("content-type"),
        Some("text/html; charset=utf-8")
    );
    for section in [
        "Identity",
        "Memory",
        "Personality",
        "Voice",
        "Appearance",
        "Modules",
        "Permissions",
        "Diagnostics",
        "Activity",
        "Fleet",
        "GPU",
        "Inference",
        "Service",
    ] {
        assert!(response.body.contains(section), "missing section {section}");
    }
    assert!(response.body.contains(CORE_A));
    assert!(response.body.contains("Core A"));
    assert!(response.body.contains("installed"));
    assert!(response.body.contains("verified"));
    assert!(response.body.contains("UNAVAILABLE"));
    assert!(!response.body.contains(&fixture.owner_token));
    assert!(!response.body.contains(&server.token()));
}

#[test]
fn unknown_core_workspace_returns_clear_html_error() {
    let fixture = Fixture::new();
    let server = Server::start(fixture.path(), false);
    let missing = "22222222-2222-4222-8222-222222222222";
    let response = get(server.addr(), &format!("/core/{missing}"));
    assert_eq!(response.status, 404);
    assert_eq!(
        response.header("content-type"),
        Some("text/html; charset=utf-8")
    );
    assert!(response.body.contains("Core not found"));
    assert!(response.body.contains("No registered Core"));

    let invalid = get(server.addr(), "/core/not-a-uuid");
    assert_eq!(invalid.status, 400);
    assert_eq!(
        invalid.header("content-type"),
        Some("text/html; charset=utf-8")
    );
    assert!(invalid.body.contains("Invalid Core identifier"));
    assert!(!invalid.body.contains("panic"));
}

#[test]
fn core_workspace_escapes_user_controlled_text() {
    let fixture = Fixture::new();
    let display_name = "<script>alert(\"x\")</script>&";
    fixture.add_core(CORE_A, 1, display_name);
    let server = Server::start(fixture.path(), false);
    let body = get(server.addr(), &format!("/core/{CORE_A}")).body;
    assert!(!body.contains("<script>alert"));
    assert!(body.contains("&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt;&amp;"));
}

#[test]
fn core_workspace_applies_core_theme_palette() {
    let fixture = Fixture::new();
    fixture.add_core(CORE_A, 1, "Core A");
    let server = Server::start(fixture.path(), false);
    let body = get(server.addr(), &format!("/core/{CORE_A}")).body;
    assert!(body.contains("--primary: #9400D3"));
    assert!(body.contains("--accent: #00C2FF"));
    assert!(body.contains("--tertiary: #39FF14"));
    assert!(body.contains("class=\"core-theme\""));
    assert!(body.contains("Appearance"));
    assert!(body.contains("Per-Core theme data is UNAVAILABLE"));
}

#[test]
fn core_workspace_reports_backend_failure_without_panic() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root_file = temp.path().join("not-a-directory");
    std::fs::write(&root_file, b"x").expect("write root file");
    let server = Server::start(&root_file, false);
    let response = get(server.addr(), &format!("/core/{CORE_A}"));
    assert_eq!(response.status, 503);
    assert_eq!(
        response.header("content-type"),
        Some("text/html; charset=utf-8")
    );
    assert!(response.body.contains("Core workspace unavailable"));
    assert!(!response.body.contains("panic"));
}

#[test]
fn core_workspace_json_api_matches_html_workspace() {
    let fixture = Fixture::new();
    fixture.add_core(CORE_A, 1, "Core A");
    let server = Server::start(fixture.path(), false);
    let response = get(server.addr(), &format!("/api/core/{CORE_A}"));
    assert_eq!(response.status, 200);
    assert_eq!(response.header("content-type"), Some("application/json"));
    assert_eq!(response.header("cache-control"), Some("no-store"));
    assert!(!response.body.contains(&fixture.owner_token));
    assert!(!response.body.contains(&server.token()));

    let value = response.json();
    assert_eq!(value["ok"].as_bool(), Some(true));
    assert_eq!(value["schema"], "aegis.ui.core_workspace.v1");
    assert_eq!(value["core_uuid"], CORE_A);
    assert_eq!(value["core_number"].as_u64(), Some(1));
    assert_eq!(value["display_name"], "Core A");
    assert_eq!(value["lifecycle_state"], "installed");
    assert!(value["generated_at"].is_string());

    let sections = value["sections"].as_array().expect("sections array");
    let titles: Vec<&str> = sections
        .iter()
        .filter_map(|section| section["title"].as_str())
        .collect();
    assert_eq!(
        titles,
        vec![
            "Identity",
            "Memory",
            "Personality",
            "Voice",
            "Appearance",
            "Modules",
            "Permissions",
            "Diagnostics",
            "Activity",
            "Fleet",
            "GPU",
            "Inference",
            "Service",
            "World",
            "Personal",
            "Emotion",
            "Continuity",
            "Outfit",
        ]
    );
    let statuses: Vec<&str> = sections
        .iter()
        .filter_map(|section| section["status"].as_str())
        .collect();
    assert!(statuses.contains(&"available"));
    assert!(statuses.contains(&"unavailable"));

    let identity = sections
        .iter()
        .find(|section| section["title"] == "Identity")
        .expect("identity section");
    let rows = identity["rows"].as_array().expect("identity rows");
    assert!(rows
        .iter()
        .any(|row| row["label"] == "Core UUID" && row["value"] == CORE_A));
    assert!(rows
        .iter()
        .any(|row| row["label"] == "Display name" && row["value"] == "Core A"));
    assert!(rows
        .iter()
        .any(|row| row["label"] == "Integrity status" && row["value"] == "verified"));

    let html = get(server.addr(), &format!("/core/{CORE_A}")).body;
    for title in &titles {
        assert!(html.contains(*title), "HTML missing section {title}");
    }
    assert!(html.contains(CORE_A));
    assert!(html.contains("Core A"));
    assert!(html.contains("installed"));
}

#[test]
fn core_workspace_json_api_returns_typed_errors() {
    let fixture = Fixture::new();
    let server = Server::start(fixture.path(), false);
    let missing = "22222222-2222-4222-8222-222222222222";

    let not_found = get(server.addr(), &format!("/api/core/{missing}"));
    assert_eq!(not_found.status, 404);
    assert_eq!(not_found.header("content-type"), Some("application/json"));
    let value = not_found.json();
    assert_eq!(value["ok"].as_bool(), Some(false));
    assert_eq!(value["error"], "core_not_found");
    assert!(!value.to_string().contains("panic"));

    let invalid = get(server.addr(), "/api/core/not-a-uuid");
    assert_eq!(invalid.status, 400);
    assert_eq!(invalid.json()["error"], "invalid_core_uuid");

    let empty = get(server.addr(), "/api/core");
    assert_eq!(empty.status, 400);
    assert_eq!(empty.json()["error"], "invalid_core_uuid");

    let trailing_missing = get(server.addr(), &format!("/api/core/{missing}/"));
    assert_eq!(trailing_missing.status, 404);
    assert_eq!(trailing_missing.json()["error"], "core_not_found");
}

#[test]
fn core_workspace_json_api_reports_backend_failure_without_panic() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root_file = temp.path().join("not-a-directory");
    std::fs::write(&root_file, b"x").expect("write root file");
    let server = Server::start(&root_file, false);
    let response = get(server.addr(), &format!("/api/core/{CORE_A}"));
    assert_eq!(response.status, 503);
    assert_eq!(response.header("content-type"), Some("application/json"));
    let value = response.json();
    assert_eq!(value["ok"].as_bool(), Some(false));
    assert_eq!(value["error"], "core_workspace_unavailable");
    assert!(value["detail"]
        .as_str()
        .is_some_and(|detail| !detail.is_empty()));
    assert!(!response.body.contains("panic"));
}

#[test]
fn core_workspace_has_breadcrumb_json_link_and_roster_workspace_links() {
    let fixture = Fixture::new();
    fixture.add_core(CORE_A, 1, "Core A");
    let server = Server::start(fixture.path(), false);

    let root = get(server.addr(), "/").body;
    assert!(root.contains("/core/"));
    assert!(root.contains("encodeURIComponent"));
    assert!(root.contains("href=\"/api/status\""));

    let workspace = get(server.addr(), &format!("/core/{CORE_A}")).body;
    assert!(workspace.contains("class=\"crumbs\""));
    assert!(workspace.contains("aria-label=\"Breadcrumb\""));
    assert!(workspace.contains("href=\"/\""));
    assert!(workspace.contains(&format!("href=\"/api/core/{CORE_A}\"")));
    assert!(workspace.contains("href=\"/api/status\""));
    assert!(workspace.contains("JSON API"));
    assert!(workspace.contains("dashboard"));
}

#[test]
fn core_workspace_paths_tolerate_trailing_slash() {
    let fixture = Fixture::new();
    fixture.add_core(CORE_A, 1, "Core A");
    let server = Server::start(fixture.path(), false);

    let html = get(server.addr(), &format!("/core/{CORE_A}/"));
    assert_eq!(html.status, 200);
    assert!(html.body.contains("Identity"));

    let json = get(server.addr(), &format!("/api/core/{CORE_A}/"));
    assert_eq!(json.status, 200);
    assert_eq!(json.json()["core_uuid"], CORE_A);
}

#[test]
fn core_workspace_json_api_serializes_user_text_safely() {
    let fixture = Fixture::new();
    let display_name = "<script>alert(\"x\")</script>&";
    fixture.add_core(CORE_A, 1, display_name);
    let server = Server::start(fixture.path(), false);

    let response = get(server.addr(), &format!("/api/core/{CORE_A}"));
    assert_eq!(response.status, 200);
    let value = response.json();
    assert_eq!(value["display_name"].as_str(), Some(display_name));

    let html = get(server.addr(), &format!("/core/{CORE_A}")).body;
    assert!(!html.contains("<script>alert"));
    assert!(html.contains("&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt;&amp;"));
}

#[test]
fn overlong_request_target_is_rejected_without_panic() {
    let fixture = Fixture::new();
    let server = Server::start(fixture.path(), false);
    let path = format!("/{}", "a".repeat(4096));
    let raw = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    let response = raw_request(server.addr(), raw.as_bytes());
    assert_eq!(response.status, 400);
    assert!(!response.body.contains("panic"));
}

fn write_snapshot(root: &Path, name: &str, contents: &str) {
    let directory = root.join("run").join("status");
    std::fs::create_dir_all(&directory).expect("snapshot directory");
    std::fs::write(directory.join(name), contents).expect("write snapshot");
}

fn fresh_timestamp() -> String {
    now_rfc3339().expect("timestamp")
}

fn write_fresh_status_snapshots(root: &Path) {
    let updated_at = fresh_timestamp();
    write_snapshot(
        root,
        "fleet.json",
        &format!(
            r#"{{"updated_at":"{updated_at}","hosts":[{{"host_id":"host-alpha","reachable":true,"model_count":2,"latency_ms":7,"error":null}}]}}"#
        ),
    );
    write_snapshot(
        root,
        "gpu.json",
        &format!(
            r#"{{"updated_at":"{updated_at}","available":true,"name":"NVIDIA RTX","utilization_percent":12,"free_vram_mb":6144,"temperature_c":55,"error":null}}"#
        ),
    );
    write_snapshot(
        root,
        "inference.json",
        &format!(
            r#"{{"updated_at":"{updated_at}","active":true,"model":"aegis-small","host_id":"host-alpha","detail":null}}"#
        ),
    );
    write_snapshot(
        root,
        "service.json",
        &format!(
            r#"{{"updated_at":"{updated_at}","running":true,"pid":4242,"pid_alive":true,"socket_reachable":true,"error":null}}"#
        ),
    );
}

#[test]
fn dashboard_renders_fresh_status_snapshot_sections() {
    let fixture = Fixture::new();
    fixture.add_core(CORE_A, 1, "Core A");
    write_fresh_status_snapshots(fixture.path());
    let server = Server::start(fixture.path(), false);

    let root = get(server.addr(), "/");
    assert_eq!(root.status, 200);
    for marker in [
        "Fleet",
        "host-alpha",
        "GPU",
        "NVIDIA RTX",
        "Inference",
        "aegis-small",
        "Service",
        "Live platform service status snapshot",
    ] {
        assert!(root.body.contains(marker), "dashboard missing {marker}");
    }
    assert!(!root.body.contains("STALE:"));

    let workspace = get(server.addr(), &format!("/core/{CORE_A}")).body;
    for marker in [
        "Fleet",
        "host-alpha",
        "GPU",
        "NVIDIA RTX",
        "Inference",
        "aegis-small",
        "Service",
    ] {
        assert!(workspace.contains(marker), "workspace missing {marker}");
    }
}

#[test]
fn stale_status_snapshot_renders_stale_not_values() {
    let fixture = Fixture::new();
    write_snapshot(
        fixture.path(),
        "service.json",
        r#"{"updated_at":"2000-01-01T00:00:00Z","running":true,"pid":4242,"pid_alive":true,"socket_reachable":true,"error":null}"#,
    );
    let server = Server::start(fixture.path(), false);

    let root = get(server.addr(), "/").body;
    assert!(root.contains("STALE"), "missing STALE badge");
    assert!(root.contains("is stale"), "missing stale reason");
    assert!(!root.contains("Live platform service status snapshot"));

    let status = get(server.addr(), "/api/status").json();
    assert_eq!(status["snapshots"]["service"]["state"], "stale");
    assert!(status["snapshots"]["service"]["reason"]
        .as_str()
        .is_some_and(|reason| reason.contains("stale")));
}

#[test]
fn malformed_status_snapshot_is_unavailable_without_panic() {
    let fixture = Fixture::new();
    let directory = fixture.path().join("run").join("status");
    std::fs::create_dir_all(&directory).expect("snapshot directory");
    std::fs::write(directory.join("gpu.json"), b"{not-json").expect("write malformed snapshot");
    let server = Server::start(fixture.path(), false);

    let root = get(server.addr(), "/");
    assert_eq!(root.status, 200);
    assert!(root.body.contains("UNAVAILABLE"));
    assert!(root.body.contains("GPU"));
    assert!(!root.body.contains("panic"));

    let status = get(server.addr(), "/api/status").json();
    assert_eq!(status["snapshots"]["gpu"]["state"], "unavailable");
    assert!(status["snapshots"]["gpu"]["reason"]
        .as_str()
        .is_some_and(|reason| reason.contains("malformed")));
}

#[test]
fn missing_status_snapshots_are_unavailable() {
    let fixture = Fixture::new();
    let server = Server::start(fixture.path(), false);

    let root = get(server.addr(), "/").body;
    assert!(root.contains("UNAVAILABLE"));
    assert!(root.contains("Fleet"));
    assert!(root.contains("GPU"));
    assert!(root.contains("Inference"));
    assert!(root.contains("Service"));
    assert!(!root.contains("Live fleet status snapshot"));

    let status = get(server.addr(), "/api/status").json();
    for section in ["fleet", "gpu", "inference", "service"] {
        assert_eq!(
            status["snapshots"][section]["state"], "unavailable",
            "{section} should be unavailable"
        );
        assert!(
            status["snapshots"][section]["reason"].is_string(),
            "{section} should carry a reason"
        );
    }
}

#[test]
fn status_api_exposes_snapshot_states_and_preserves_platform_fields() {
    let fixture = Fixture::new();
    fixture.add_core(CORE_A, 1, "Core A");
    write_fresh_status_snapshots(fixture.path());
    let server = Server::start(fixture.path(), false);

    let status = get(server.addr(), "/api/status").json();
    assert_eq!(status["core_count"].as_u64(), Some(1));
    assert!(status["zero_core"].is_boolean());

    let snapshots = &status["snapshots"];
    assert_eq!(snapshots["schema"], "aegis.ui.status_snapshots.v1");
    for section in ["fleet", "gpu", "inference", "service"] {
        assert_eq!(snapshots[section]["state"], "fresh", "{section} state");
        assert!(
            snapshots[section]["reason"].is_null(),
            "{section} fresh reason should be null"
        );
    }
}

fn world_snapshot_json(updated_at: &str, condition: &str, temperature: f64) -> String {
    serde_json::json!({
        "updated_at": updated_at,
        "local_time": "12:00",
        "timezone": "UTC",
        "time_of_day": "midday",
        "season": "autumn",
        "weather": {
            "condition": condition,
            "temperature_celsius": temperature
        }
    })
    .to_string()
}

fn personal_snapshot_json(updated_at: &str, continuity: &str) -> String {
    serde_json::json!({
        "updated_at": updated_at,
        "core_uuid": CORE_A,
        "emotion": {
            "valence": 0.4,
            "calm": 0.8
        },
        "continuity": {
            "summary": continuity
        },
        "activities": [{ "title": "Reading" }],
        "routines": [{ "name": "Morning check-in" }],
        "outfit": {
            "id": "outfit-1",
            "decided_at": updated_at,
            "asset_ids": ["asset-1", "asset-2"],
            "rationale": "warm and simple"
        }
    })
    .to_string()
}

#[test]
fn dashboard_renders_fresh_world_and_personal_snapshots() {
    let fixture = Fixture::new();
    fixture.add_core(CORE_A, 1, "Core A");
    let now = fresh_timestamp();
    write_snapshot(
        fixture.path(),
        "world.json",
        &world_snapshot_json(&now, "clear skies", 12.5),
    );
    write_snapshot(
        fixture.path(),
        "personal.json",
        &personal_snapshot_json(&now, "kept a journal"),
    );
    let server = Server::start(fixture.path(), false);

    let root = get(server.addr(), "/");
    assert_eq!(root.status, 200);
    for marker in [
        "World",
        "Personal",
        "Emotion",
        "Continuity",
        "Outfit",
        "UTC",
        "12:00",
        "midday",
        "autumn",
        "clear skies",
        "12.5",
        "valence=0.40",
        "calm=0.80",
        "kept a journal",
        "warm and simple",
        "asset-1",
    ] {
        assert!(root.body.contains(marker), "dashboard missing {marker}");
    }
    assert!(!root.body.contains("STALE:"));

    let workspace = get(server.addr(), &format!("/core/{CORE_A}")).body;
    for marker in [
        "World",
        "Personal",
        "Emotion",
        "Continuity",
        "Outfit",
        "clear skies",
        "kept a journal",
        "warm and simple",
    ] {
        assert!(workspace.contains(marker), "workspace missing {marker}");
    }

    let status = get(server.addr(), "/api/status").json();
    assert_eq!(status["snapshots"]["world"]["state"], "fresh");
    assert_eq!(status["snapshots"]["personal"]["state"], "fresh");
    assert!(status["snapshots"]["world"]["reason"].is_null());
}

#[test]
fn stale_world_and_personal_snapshots_render_stale_not_values() {
    let fixture = Fixture::new();
    let old = "2000-01-01T00:00:00Z";
    write_snapshot(
        fixture.path(),
        "world.json",
        &world_snapshot_json(old, "impossible-weather", 99.0),
    );
    write_snapshot(
        fixture.path(),
        "personal.json",
        &personal_snapshot_json(old, "ancient-summary"),
    );
    let server = Server::start(fixture.path(), false);

    let root = get(server.addr(), "/").body;
    assert!(root.contains("STALE"), "missing STALE badge");
    assert!(root.contains("is stale"), "missing stale reason");
    assert!(!root.contains("impossible-weather"));
    assert!(!root.contains("ancient-summary"));
    assert!(!root.contains("warm and simple"));
    for title in ["World", "Personal", "Emotion", "Continuity", "Outfit"] {
        assert!(root.contains(title), "missing section {title}");
    }

    let status = get(server.addr(), "/api/status").json();
    for section in ["world", "personal"] {
        assert_eq!(
            status["snapshots"][section]["state"], "stale",
            "{section} should be stale"
        );
        assert!(status["snapshots"][section]["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("stale")));
    }
}

#[test]
fn missing_world_and_personal_snapshots_are_unavailable() {
    let fixture = Fixture::new();
    let server = Server::start(fixture.path(), false);

    let root = get(server.addr(), "/").body;
    for title in ["World", "Personal", "Emotion", "Continuity", "Outfit"] {
        assert!(root.contains(title), "missing section {title}");
    }
    assert!(root.contains("no world status snapshot is available"));
    assert!(root.contains("no personal status snapshot is available"));

    let status = get(server.addr(), "/api/status").json();
    for section in ["world", "personal"] {
        assert_eq!(
            status["snapshots"][section]["state"], "unavailable",
            "{section} should be unavailable"
        );
        assert!(status["snapshots"][section]["reason"].is_string());
    }
}

#[test]
fn malformed_world_and_personal_snapshots_are_unavailable_without_panic() {
    let fixture = Fixture::new();
    write_snapshot(fixture.path(), "world.json", "{not-json");
    write_snapshot(fixture.path(), "personal.json", "{not-json");
    let server = Server::start(fixture.path(), false);

    let root = get(server.addr(), "/");
    assert_eq!(root.status, 200);
    assert!(root.body.contains("UNAVAILABLE"));
    assert!(root.body.contains("malformed JSON"));
    assert!(!root.body.contains("panic"));
    for title in ["World", "Personal", "Emotion", "Continuity", "Outfit"] {
        assert!(root.body.contains(title), "missing section {title}");
    }

    let status = get(server.addr(), "/api/status").json();
    for section in ["world", "personal"] {
        assert_eq!(status["snapshots"][section]["state"], "unavailable");
        assert!(status["snapshots"][section]["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("malformed")));
    }
}

#[test]
fn fresh_personal_snapshot_missing_emotion_is_not_inferred_available() {
    let fixture = Fixture::new();
    let now = fresh_timestamp();
    let mut value: Value = serde_json::from_str(&personal_snapshot_json(&now, "kept a journal"))
        .expect("personal JSON");
    value
        .as_object_mut()
        .expect("personal object")
        .remove("emotion");
    write_snapshot(fixture.path(), "personal.json", &value.to_string());
    let server = Server::start(fixture.path(), false);

    let body = get(server.addr(), "/").body;
    assert!(body.contains("Emotion"));
    assert!(body.contains("reports no emotion data"));
    assert!(body.contains("kept a journal"));
    assert!(body.contains("warm and simple"));
}

#[test]
fn world_and_personal_snapshot_text_is_html_escaped() {
    let fixture = Fixture::new();
    let now = fresh_timestamp();
    write_snapshot(
        fixture.path(),
        "world.json",
        &world_snapshot_json(&now, "<script>alert(\"x\")</script>", 1.0),
    );
    write_snapshot(
        fixture.path(),
        "personal.json",
        &personal_snapshot_json(&now, "<script>alert('y')</script>&"),
    );
    let server = Server::start(fixture.path(), false);

    let body = get(server.addr(), "/").body;
    assert!(!body.contains("<script>alert"));
    assert!(body.contains("&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt;"));
    assert!(body.contains("&lt;script&gt;alert(&#39;y&#39;)&lt;/script&gt;&amp;"));
}

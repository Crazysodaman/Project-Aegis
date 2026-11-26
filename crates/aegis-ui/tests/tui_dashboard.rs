use std::path::Path;

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_core_install::CoreInstaller;
use aegis_platform::PlatformService;
use aegis_storage::now_rfc3339;
use aegis_ui::{render_dashboard, render_summary_json};
use serde_json::{json, Value};
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

    fn add_core(&self, core_uuid: &str, core_number: u8, display_name: &str) {
        let request = CoreInitializationRequest {
            schema: CORE_INIT_REQUEST_SCHEMA.to_owned(),
            core_uuid: core_uuid.to_owned(),
            core_number,
            display_name: display_name.to_owned(),
            requested_at: now_rfc3339().expect("timestamp"),
        };
        CoreInstaller::new(self.path())
            .expect("installer")
            .initialize(&request, &self.owner_token)
            .expect("initialize core");
    }
}

#[test]
fn snapshot_renders_required_sections_with_real_data() {
    let fixture = Fixture::new();
    fixture.add_core(CORE_A, 1, "Core A");
    fixture.add_core(CORE_B, 2, "Core B");

    let rendered = render_dashboard(fixture.path());

    for section in [
        "Platform",
        "World",
        "Personal",
        "Installed Cores",
        "Fleet hosts",
        "GPU",
        "Active inference",
        "Memory",
        "Database",
        "Service",
        "Modules",
        "Errors and warnings",
    ] {
        assert!(rendered.contains(section), "missing section {section}");
    }
    assert!(rendered.contains(CORE_A));
    assert!(rendered.contains(CORE_B));
    assert!(rendered.contains("integrity="));
    assert!(rendered.contains("lifecycle=installed"));
    assert!(rendered.contains("\x1b[2J"));
    assert!(!rendered.contains(&fixture.owner_token));
}

#[test]
fn snapshot_labels_unlinked_data_sources_as_unavailable() {
    let fixture = Fixture::new();
    let rendered = render_dashboard(fixture.path());

    assert!(rendered.contains("UNAVAILABLE: aegis-fleet is not a dependency"));
    assert!(rendered.contains("UNAVAILABLE: aegis-fleet GPU telemetry is not linked"));
    assert!(rendered.contains("UNAVAILABLE: aegis-cognition is not a dependency"));
    assert!(rendered.contains("UNAVAILABLE: no memory telemetry data source"));
    assert!(rendered.contains("UNAVAILABLE: aegis-service is not a dependency"));
    assert!(rendered.contains("UNAVAILABLE: aegis-fleet"));
}

#[test]
fn snapshot_is_honest_for_uninitialized_root() {
    let root = tempfile::tempdir().expect("tempdir");
    let rendered = render_dashboard(root.path());

    assert!(rendered.contains("UNAVAILABLE"));
    assert!(rendered.contains("NONE: no registered Cores"));
    assert!(rendered.contains("NONE: no modules installed"));
    assert!(rendered.contains("platform.db : "));
    assert!(rendered.contains("core dbs    : "));
}

#[test]
fn summary_json_exposes_shared_values_and_unavailable_reasons() {
    let fixture = Fixture::new();
    fixture.add_core(CORE_A, 1, "Core A");

    let summary = render_summary_json(fixture.path());

    assert_eq!(summary["schema"], "aegis.ui.summary.v1");
    assert!(summary["generated_at"].is_string());
    assert_eq!(summary["platform"]["status"], "available");
    assert!(summary["overall_health"].is_string());
    assert_eq!(summary["cores"]["count"], 1);
    assert_eq!(summary["cores"]["items"][0]["core_uuid"], CORE_A);
    assert_eq!(
        summary["cores"]["items"][0]["integrity"]["status"],
        "verified"
    );

    for (key, needle) in [
        ("fleet", "aegis-fleet"),
        ("gpu", "GPU telemetry"),
        ("inference", "aegis-cognition"),
        ("service", "aegis-service"),
        ("memory", "memory telemetry"),
    ] {
        assert_eq!(summary[key]["status"], "unavailable", "{key}");
        assert!(
            summary[key]["unavailable_reason"]
                .as_str()
                .unwrap()
                .contains(needle),
            "{key}"
        );
    }
    assert!(summary["issues"]["errors"].is_array());
    assert!(summary["issues"]["warnings"].is_array());
    assert!(summary["consistency"].is_array());
}

#[test]
fn summary_json_flags_uninitialized_root_as_unavailable_with_consistency() {
    let root = tempfile::tempdir().expect("tempdir");
    let summary = render_summary_json(root.path());

    assert_eq!(summary["platform"]["status"], "unavailable");
    assert!(summary["platform"]["unavailable_reason"].is_string());
    assert_eq!(summary["cores"]["count"], 0);
    let consistency = summary["consistency"].as_array().expect("consistency");
    assert!(consistency.iter().any(|issue| issue["code"] == "no_cores"));
}

fn write_status_snapshot(root: &Path, name: &str, value: &Value) {
    let dir = root.join("run/status");
    std::fs::create_dir_all(&dir).expect("status dir");
    let path = dir.join(name);
    std::fs::write(path, serde_json::to_vec(value).expect("serialize")).expect("write snapshot");
}

fn fresh_fleet() -> Value {
    json!({
        "updated_at": now_rfc3339().expect("timestamp"),
        "hosts": [
            {
                "host_id": "host-snapshot",
                "reachable": true,
                "model_count": 3,
                "latency_ms": 7,
                "error": null
            }
        ]
    })
}

fn fresh_gpu() -> Value {
    json!({
        "updated_at": now_rfc3339().expect("timestamp"),
        "available": true,
        "name": "RTX Snapshot",
        "utilization_percent": 12,
        "free_vram_mb": 6144,
        "temperature_c": 55,
        "error": null
    })
}

fn fresh_inference() -> Value {
    json!({
        "updated_at": now_rfc3339().expect("timestamp"),
        "active": true,
        "model": "aegis-snapshot",
        "host_id": "host-snapshot",
        "detail": null
    })
}

fn fresh_service() -> Value {
    json!({
        "updated_at": now_rfc3339().expect("timestamp"),
        "running": true,
        "pid": 4242,
        "pid_alive": true,
        "socket_reachable": true,
        "error": null
    })
}

fn fresh_world() -> Value {
    json!({
        "updated_at": now_rfc3339().expect("timestamp"),
        "local_time": "08:30",
        "timezone": "UTC",
        "time_of_day": "morning",
        "season": "autumn",
        "weather": {
            "condition": "clear sky",
            "temperature_celsius": 12.5
        }
    })
}

fn fresh_personal() -> Value {
    json!({
        "updated_at": now_rfc3339().expect("timestamp"),
        "core_uuid": CORE_A,
        "emotion": {
            "valence": 0.4,
            "calm": 0.8
        },
        "continuity": {
            "summary": "kept a journal"
        },
        "activities": [{ "title": "walk" }],
        "routines": [{ "name": "morning tea" }]
    })
}

#[test]
fn fresh_snapshot_files_populate_all_sections() {
    let fixture = Fixture::new();
    write_status_snapshot(fixture.path(), "fleet.json", &fresh_fleet());
    write_status_snapshot(fixture.path(), "gpu.json", &fresh_gpu());
    write_status_snapshot(fixture.path(), "inference.json", &fresh_inference());
    write_status_snapshot(fixture.path(), "service.json", &fresh_service());

    let rendered = render_dashboard(fixture.path());
    assert!(rendered.contains("host-snapshot"));
    assert!(rendered.contains("RTX Snapshot"));
    assert!(rendered.contains("aegis-snapshot"));
    assert!(rendered.contains("pid     : 4242"));
    assert!(!rendered.contains("STALE"));

    let summary = render_summary_json(fixture.path());
    assert_eq!(summary["fleet"]["status"], "available");
    assert_eq!(summary["fleet"]["hosts"][0]["host_id"], "host-snapshot");
    assert_eq!(summary["gpu"]["status"], "available");
    assert_eq!(summary["gpu"]["name"], "RTX Snapshot");
    assert_eq!(summary["inference"]["status"], "available");
    assert_eq!(summary["inference"]["model"], "aegis-snapshot");
    assert_eq!(summary["service"]["status"], "available");
    assert_eq!(summary["service"]["pid"], 4242);
}

#[test]
fn missing_snapshot_files_keep_sections_unavailable() {
    let fixture = Fixture::new();
    let summary = render_summary_json(fixture.path());

    for (key, needle) in [
        ("fleet", "aegis-fleet"),
        ("gpu", "GPU telemetry"),
        ("inference", "aegis-cognition"),
        ("service", "aegis-service"),
    ] {
        assert_eq!(summary[key]["status"], "unavailable", "{key}");
        assert!(
            summary[key]["unavailable_reason"]
                .as_str()
                .expect("unavailable reason")
                .contains(needle),
            "{key}"
        );
    }
}

#[test]
fn fresh_world_and_personal_snapshots_render_real_values() {
    let fixture = Fixture::new();
    write_status_snapshot(fixture.path(), "world.json", &fresh_world());
    write_status_snapshot(fixture.path(), "personal.json", &fresh_personal());

    let rendered = render_dashboard(fixture.path());
    assert!(rendered.contains("World"));
    assert!(rendered.contains("08:30"));
    assert!(rendered.contains("clear sky"));
    assert!(rendered.contains("12.5 C"));
    assert!(rendered.contains("Personal"));
    assert!(rendered.contains("valence=0.40"));
    assert!(rendered.contains("kept a journal"));
    assert!(rendered.contains("activities  : 1"));
    assert!(rendered.contains("routines    : 1"));
    assert!(!rendered.contains("STALE"));
}

#[test]
fn missing_world_and_personal_snapshots_render_unavailable() {
    let fixture = Fixture::new();
    let rendered = render_dashboard(fixture.path());

    assert!(rendered.contains("UNAVAILABLE: no world status snapshot is available"));
    assert!(rendered.contains("UNAVAILABLE: no personal status snapshot is available"));
}

#[test]
fn stale_world_and_personal_snapshots_render_stale_without_values() {
    let fixture = Fixture::new();
    let stale_world = json!({
        "updated_at": "2020-01-01T00:00:00Z",
        "local_time": "03:00",
        "weather": { "condition": "old weather" }
    });
    let stale_personal = json!({
        "updated_at": "2020-01-01T00:00:00Z",
        "core_uuid": CORE_A,
        "continuity": { "summary": "old continuity" }
    });
    write_status_snapshot(fixture.path(), "world.json", &stale_world);
    write_status_snapshot(fixture.path(), "personal.json", &stale_personal);

    let rendered = render_dashboard(fixture.path());
    assert!(rendered.contains("STALE:"));
    assert!(!rendered.contains("old weather"));
    assert!(!rendered.contains("old continuity"));
}

#[test]
fn malformed_world_and_personal_snapshots_are_honest() {
    let fixture = Fixture::new();
    let dir = fixture.path().join("run/status");
    std::fs::create_dir_all(&dir).expect("status dir");
    std::fs::write(dir.join("world.json"), b"{ not valid json").expect("write malformed");
    std::fs::write(
        dir.join("personal.json"),
        serde_json::to_vec(&json!({
            "updated_at": now_rfc3339().expect("timestamp"),
            "emotion": 7
        }))
        .expect("serialize"),
    )
    .expect("write malformed");

    let rendered = render_dashboard(fixture.path());
    assert!(rendered.contains("malformed JSON"));
    assert!(rendered.contains("failed validation"));
    assert!(!rendered.contains("emotion     : 7"));
}

#[test]
fn stale_snapshot_files_render_stale_and_are_not_trusted() {
    let fixture = Fixture::new();
    let stale = json!({
        "updated_at": "2020-01-01T00:00:00Z",
        "hosts": [{ "host_id": "old-host", "reachable": true }]
    });
    write_status_snapshot(fixture.path(), "fleet.json", &stale);
    let stale_service = json!({
        "updated_at": "2020-01-01T00:00:00Z",
        "running": true,
        "pid": 1,
        "pid_alive": true,
        "socket_reachable": true
    });
    write_status_snapshot(fixture.path(), "service.json", &stale_service);

    let rendered = render_dashboard(fixture.path());
    assert!(rendered.contains("STALE:"));
    assert!(!rendered.contains("old-host"));

    let summary = render_summary_json(fixture.path());
    assert_eq!(summary["fleet"]["status"], "stale");
    assert_eq!(summary["service"]["status"], "stale");
    assert!(summary["fleet"]["unavailable_reason"]
        .as_str()
        .expect("reason")
        .contains("stale"));
}

#[test]
fn malformed_snapshot_file_does_not_panic_and_reports_unavailable() {
    let fixture = Fixture::new();
    let dir = fixture.path().join("run/status");
    std::fs::create_dir_all(&dir).expect("status dir");
    std::fs::write(dir.join("fleet.json"), b"{ not valid json").expect("write malformed");

    let rendered = render_dashboard(fixture.path());
    assert!(rendered.contains("UNAVAILABLE:"));
    assert!(rendered.contains("malformed JSON"));

    let summary = render_summary_json(fixture.path());
    assert_eq!(summary["fleet"]["status"], "unavailable");
    assert!(summary["fleet"]["unavailable_reason"]
        .as_str()
        .expect("reason")
        .contains("malformed JSON"));
}

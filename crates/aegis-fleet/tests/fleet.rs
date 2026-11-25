use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use aegis_fleet::{
    fleet_store, FleetError, FleetStore, GpuPolicy, GpuProbe, GpuTelemetry, HostRole,
    NvidiaSmiProbe, GPU_TELEMETRY_SCHEMA,
};
use aegis_platform::PlatformService;

fn setup() -> (tempfile::TempDir, PathBuf, String) {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("aegis-root");
    let initialized = PlatformService::initialize(&root).expect("platform");
    (temp, root, initialized.owner_token)
}

fn store(root: &std::path::Path) -> FleetStore {
    fleet_store(root).expect("fleet store")
}

fn mock_ollama(accepts: usize) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
    let address = listener.local_addr().expect("addr");
    let handle = thread::spawn(move || {
        for _ in 0..accepts {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buffer = [0u8; 4096];
            let read = stream.read(&mut buffer).unwrap_or(0);
            let request = String::from_utf8_lossy(&buffer[..read]);
            let body = if request.contains("/api/version") {
                r#"{"version":"0.0-test"}"#
            } else if request.contains("/api/tags") {
                r#"{"models":[{"name":"test-model","size":1,"details":{"family":"test"}}]}"#
            } else {
                r#"{}"#
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
            thread::sleep(Duration::from_millis(5));
        }
    });
    (format!("http://{address}"), handle)
}

#[test]
fn hosts_require_owner_proof_and_start_untrusted() {
    let (_temp, root, token) = setup();
    let store = store(&root);
    let host = store
        .add_host(
            "Venus",
            "http://127.0.0.1:11434",
            HostRole::Inference,
            serde_json::json!({"gpu":"RTX 3080 Ti"}),
            90,
            &token,
        )
        .expect("add host");
    assert!(!host.enrolled);
    assert!(!host.trusted);
    assert_eq!(host.role, HostRole::Inference);
    assert!(matches!(
        store.add_host(
            "Venus",
            "http://127.0.0.1:11434",
            HostRole::Inference,
            serde_json::Value::Null,
            50,
            &token,
        ),
        Err(FleetError::HostExists(_))
    ));
    assert!(matches!(
        store.add_host(
            "Artemis",
            "http://127.0.0.1:11434",
            HostRole::General,
            serde_json::Value::Null,
            50,
            "not-the-owner",
        ),
        Err(FleetError::Unauthorized)
    ));
}

#[test]
fn enrollment_probe_and_revocation_control_placement() {
    let (_temp, root, token) = setup();
    let store = store(&root);
    let (endpoint, server) = mock_ollama(2);
    let host = store
        .add_host(
            "Venus",
            &endpoint,
            HostRole::Inference,
            serde_json::Value::Null,
            80,
            &token,
        )
        .expect("host");
    assert!(store.placements().expect("placement").is_empty());
    store.enroll(&host.host_id, &token).expect("enroll");
    let probed = store.probe(&host.host_id).expect("probe");
    let health = probed.health.expect("health");
    assert!(health.reachable);
    assert_eq!(health.model_count, 1);
    assert_eq!(store.placements().expect("placement").len(), 1);
    store.revoke(&host.host_id, &token).expect("revoke");
    assert!(store.placements().expect("placement").is_empty());
    server.join().expect("server");
}

#[test]
fn inference_routing_requires_an_enrolled_host_with_the_model() {
    let (_temp, root, token) = setup();
    let store = store(&root);
    let (endpoint, server) = mock_ollama(4);
    let host = store
        .add_host(
            "Venus",
            &endpoint,
            HostRole::Inference,
            serde_json::Value::Null,
            80,
            &token,
        )
        .expect("host");
    store.enroll(&host.host_id, &token).expect("enroll");
    store.probe(&host.host_id).expect("probe");
    let candidate = store.route_inference("test-model").expect("route");
    assert_eq!(candidate.endpoint, endpoint);
    assert!(matches!(
        store.route_inference("missing-model"),
        Err(FleetError::NoHealthyHost(_))
    ));
    server.join().expect("server");
}

#[test]
fn unreachable_host_is_reported_and_platform_survives() {
    let (_temp, root, token) = setup();
    let store = store(&root);
    let host = store
        .add_host(
            "Offline",
            "http://127.0.0.1:1",
            HostRole::Inference,
            serde_json::Value::Null,
            50,
            &token,
        )
        .expect("host");
    store.enroll(&host.host_id, &token).expect("enroll");
    let probed = store.probe(&host.host_id).expect("probe");
    let health = probed.health.expect("health");
    assert!(!health.reachable);
    assert!(health.error.is_some());
    assert!(store.placements().expect("placement").is_empty());
    assert!(PlatformService::start(&root)
        .expect("platform")
        .status()
        .is_ok());
}

#[test]
fn policy_and_host_records_persist() {
    let (_temp, root, token) = setup();
    {
        let store = store(&root);
        store
            .add_host(
                "Venus",
                "http://127.0.0.1:11434",
                HostRole::Inference,
                serde_json::Value::Null,
                80,
                &token,
            )
            .expect("host");
        let policy = store.policy().expect("policy");
        assert!(policy.gaming_yield);
        let updated = store.set_policy(false, &token).expect("set policy");
        assert!(!updated.gaming_yield);
        assert!(store.set_policy(true, "bad").is_err());
    }
    let reopened = store(&root);
    assert_eq!(reopened.list_hosts().expect("hosts").len(), 1);
    assert!(!reopened.policy().expect("policy").gaming_yield);
}

#[test]
fn remove_requires_owner_and_deletes_the_record() {
    let (_temp, root, token) = setup();
    let store = store(&root);
    let host = store
        .add_host(
            "Temp",
            "http://127.0.0.1:11434",
            HostRole::Storage,
            serde_json::Value::Null,
            10,
            &token,
        )
        .expect("host");
    assert!(store.remove(&host.host_id, "bad").is_err());
    store.remove(&host.host_id, &token).expect("remove");
    assert!(store.list_hosts().expect("hosts").is_empty());
}

#[test]
fn endpoint_validation_rejects_unsafe_inputs() {
    let (_temp, root, token) = setup();
    let store = store(&root);
    assert!(matches!(
        store.add_host(
            "Bad",
            "https://example.test",
            HostRole::Inference,
            serde_json::Value::Null,
            50,
            &token,
        ),
        Err(FleetError::InvalidEndpoint(_))
    ));
    assert!(matches!(
        store.add_host(
            "Bad",
            "http://user:pass@example.test",
            HostRole::Inference,
            serde_json::Value::Null,
            50,
            &token,
        ),
        Err(FleetError::InvalidEndpoint(_))
    ));
}

#[test]
fn gpu_policy_and_admission_are_owner_gated() {
    let (_temp, root, token) = setup();
    let store = store(&root);
    let policy = store.policy().expect("policy");
    assert_eq!(policy.max_utilization_percent, 25);
    assert_eq!(policy.min_free_vram_mb, 2048);
    assert_eq!(policy.max_temperature_c, 80);

    let telemetry = GpuTelemetry {
        schema: GPU_TELEMETRY_SCHEMA.to_owned(),
        available: true,
        name: Some("Test GPU".to_owned()),
        utilization_percent: Some(70),
        memory_used_mb: Some(6000),
        memory_total_mb: Some(12288),
        temperature_c: Some(60),
        source: "test".to_owned(),
        checked_at: "2026-01-01T00:00:00Z".to_owned(),
        error: None,
    };
    let busy = store.admission(&telemetry).expect("admission");
    assert!(!busy.admit);
    assert!(busy.reasons.contains(&"gpu_busy".to_owned()));

    let relaxed = GpuPolicy {
        max_utilization_percent: 90,
        min_free_vram_mb: 1024,
        ..policy.gpu_policy()
    };
    assert!(store.set_gpu_policy(relaxed, "bad").is_err());
    let updated = store.set_gpu_policy(relaxed, &token).expect("set");
    assert_eq!(updated.max_utilization_percent, 90);
    let admit = store.admission(&telemetry).expect("admission");
    assert!(admit.admit, "reasons: {:?}", admit.reasons);
}

#[test]
fn live_gpu_telemetry_when_enabled() {
    if std::env::var("AEGIS_GPU_LIVE").ok().as_deref() != Some("1") {
        eprintln!("skipping live GPU test: set AEGIS_GPU_LIVE=1");
        return;
    }
    let telemetry = NvidiaSmiProbe::new().probe();
    assert!(
        telemetry.available,
        "GPU telemetry unavailable: {:?}",
        telemetry.error
    );
    assert!(telemetry
        .name
        .as_deref()
        .is_some_and(|name| !name.is_empty()));
    assert!(telemetry.memory_total_mb.unwrap_or(0) > 0);
    eprintln!("live GPU telemetry: {telemetry:?}");
}

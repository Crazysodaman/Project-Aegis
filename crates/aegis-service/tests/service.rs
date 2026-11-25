use std::time::Duration;

use aegis_platform::PlatformService;
use aegis_service::{ServiceError, ServiceManager};

fn setup() -> (tempfile::TempDir, String) {
    let temp = tempfile::tempdir().expect("tempdir");
    let token = PlatformService::initialize(temp.path())
        .expect("platform initialize")
        .owner_token;
    (temp, token)
}

fn manager(temp: &tempfile::TempDir) -> ServiceManager {
    ServiceManager::new(temp.path())
        .expect("service manager")
        .with_host_executable(env!("CARGO_BIN_EXE_aegis-service-host"))
        .with_socket_name(unique_socket(temp))
}

fn unique_socket(temp: &tempfile::TempDir) -> String {
    let name = temp
        .path()
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "aegis-service-test".to_owned());
    format!("aegis-service-{}-{name}", std::process::id())
}

#[test]
fn service_starts_answers_ipc_and_stops() {
    let (temp, token) = setup();
    let manager = manager(&temp);

    let state = manager.start(&token).expect("start service");
    assert!(state.pid > 0);
    let status = manager.status();
    assert!(status.running, "{status:?}");
    assert!(status.pid_alive);
    assert!(status.socket_reachable, "{status:?}");

    let report = manager.stop(&token).expect("stop service");
    assert!(report.stopped);
    assert!(!aegis_service::process_is_alive(state.pid));
    assert!(!manager.status().running);

    let again = manager.stop(&token).expect("idempotent stop");
    assert!(again.stopped);
    assert_eq!(again.pid, None);
}

#[test]
fn service_restarts_after_stop() {
    let (temp, token) = setup();
    let manager = manager(&temp);

    let first = manager.start(&token).expect("first start");
    manager.stop(&token).expect("first stop");
    let second = manager.start(&token).expect("second start");
    assert_ne!(first.pid, second.pid);
    manager.stop(&token).expect("second stop");
}

#[test]
fn service_start_requires_owner_proof() {
    let (temp, token) = setup();
    let manager = manager(&temp);

    let error = manager.start("wrong-token").expect_err("denied");
    assert!(matches!(error, ServiceError::Unauthorized));
    assert!(!manager.status().running);

    manager.start(&token).expect("authorized start");
    manager.stop(&token).expect("cleanup");
}

#[test]
fn service_start_requires_initialized_platform() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manager = ServiceManager::new(temp.path())
        .expect("manager")
        .with_host_executable(env!("CARGO_BIN_EXE_aegis-service-host"))
        .with_socket_name(unique_socket(&temp));
    let error = manager.start("any-token").expect_err("not initialized");
    assert!(matches!(
        error,
        ServiceError::Safe(_) | ServiceError::NotInitialized(_)
    ));
}

#[test]
fn stale_state_file_does_not_block_start() {
    let (temp, token) = setup();
    let manager = manager(&temp);
    let state = manager.start(&token).expect("start");
    manager.stop(&token).expect("stop");

    let run_dir = temp.path().join("run");
    std::fs::create_dir_all(&run_dir).expect("run dir");
    std::fs::write(
        run_dir.join("service.json"),
        format!(
            "{{\"schema\":\"aegis.service.state.v1\",\"pid\":999999,\"root\":\"{}\",\"socket\":\"aegis-platform\",\"started_at\":\"2026-01-01T00:00:00Z\",\"executable\":\"none\",\"protocol\":\"1.0\"}}",
            temp.path().display().to_string().replace('\\', "\\\\")
        ),
    )
    .expect("stale state");
    let status = manager.status();
    assert!(!status.running);

    let restarted = manager.start(&token).expect("restart over stale state");
    assert_ne!(restarted.pid, state.pid);
    manager.stop(&token).expect("cleanup");
}

#[test]
fn forced_stop_path_is_bounded() {
    let (temp, token) = setup();
    let manager = manager(&temp);
    let state = manager.start(&token).expect("start");
    let report = manager.stop(&token).expect("stop");
    assert!(report.stopped);
    assert!(!aegis_service::process_is_alive(state.pid));
    std::thread::sleep(Duration::from_millis(50));
    assert!(!aegis_service::process_is_alive(state.pid));
}

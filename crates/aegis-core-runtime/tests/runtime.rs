use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_contracts::roster::{CoreRegistrationRequest, CORE_REGISTRY_SCHEMA};
use aegis_core_install::CoreInstaller;
use aegis_core_integrity::compute_file_sha256;
use aegis_core_registry::CoreRegistry;
use aegis_core_runtime::{
    heartbeat_path, stop_sentinel_path, CoreRuntime, CoreRuntimeState, RuntimeError,
};
use aegis_platform::PlatformService;
use aegis_storage::{now_rfc3339, open_initialized, StoragePaths, PLATFORM_APPLICATION_ID};

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_B: &str = "b9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_C: &str = "c9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const UNKNOWN: &str = "33333333-3333-4333-8333-333333333333";

fn setup() -> (tempfile::TempDir, StoragePaths, CoreInstaller, String) {
    let temp = tempfile::tempdir().expect("tempdir");
    let initialized = PlatformService::initialize(temp.path()).expect("platform init");
    let installer = CoreInstaller::new(temp.path()).expect("installer");
    let paths = StoragePaths::new(temp.path()).expect("storage paths");
    (temp, paths, installer, initialized.owner_token)
}

fn request(core_uuid: &str, core_number: u8, display_name: &str) -> CoreInitializationRequest {
    CoreInitializationRequest {
        schema: CORE_INIT_REQUEST_SCHEMA.to_owned(),
        core_uuid: core_uuid.to_owned(),
        core_number,
        display_name: display_name.to_owned(),
        requested_at: now_rfc3339().expect("timestamp"),
    }
}

fn install(installer: &CoreInstaller, token: &str, core_uuid: &str, core_number: u8) {
    installer
        .initialize(&request(core_uuid, core_number, "Runtime Core"), token)
        .expect("initialize Core");
}

#[test]
fn starts_and_stops_a_real_child_process() {
    let (_temp, paths, installer, token) = setup();
    install(&installer, &token, CORE_A, 1);
    let runtime = CoreRuntime::new(paths.clone());

    let before = runtime.status(CORE_A).expect("status before start");
    assert_eq!(before.state, CoreRuntimeState::Stopped);
    assert!(!before.heartbeat_fresh);

    let started = runtime.start(CORE_A, &token).expect("start");
    assert_eq!(started.state, CoreRuntimeState::Running);
    assert!(started.is_running());
    assert!(started.pid.is_some());
    let heartbeat = heartbeat_path(paths.root(), CORE_A);
    assert!(heartbeat.is_file());

    thread::sleep(Duration::from_millis(600));
    let live = runtime.status(CORE_A).expect("status while running");
    assert_eq!(live.state, CoreRuntimeState::Running);
    assert!(live.heartbeat_fresh);
    assert_eq!(live.recorded_state, CoreRuntimeState::Running);

    let stopped = runtime.stop(CORE_A, &token).expect("stop");
    assert_eq!(stopped.state, CoreRuntimeState::Stopped);
    assert!(!stopped.heartbeat_fresh);
    assert!(!heartbeat.exists());
    assert!(!stop_sentinel_path(paths.root(), CORE_A).exists());
}

#[test]
fn unauthorized_mutations_are_rejected() {
    let (_temp, paths, installer, token) = setup();
    install(&installer, &token, CORE_A, 1);
    let runtime = CoreRuntime::new(paths.clone());

    let error = runtime.start(CORE_A, "wrong-token").expect_err("bad token");
    assert!(matches!(error, RuntimeError::Unauthorized));
    assert!(!heartbeat_path(paths.root(), CORE_A).exists());
    assert_eq!(
        runtime.status(CORE_A).expect("status").state,
        CoreRuntimeState::Stopped
    );

    runtime.start(CORE_A, &token).expect("authorized start");
    let error = runtime.stop(CORE_A, "wrong-token").expect_err("bad token");
    assert!(matches!(error, RuntimeError::Unauthorized));
    assert_eq!(
        runtime.status(CORE_A).expect("still running").state,
        CoreRuntimeState::Running
    );

    runtime.stop(CORE_A, &token).expect("authorized stop");
}

#[test]
fn unknown_cores_are_rejected() {
    let (_temp, paths, _installer, token) = setup();
    let runtime = CoreRuntime::new(paths);

    assert!(matches!(
        runtime.status(UNKNOWN),
        Err(RuntimeError::UnknownCore(core)) if core == UNKNOWN
    ));
    assert!(matches!(
        runtime.start(UNKNOWN, &token),
        Err(RuntimeError::UnknownCore(core)) if core == UNKNOWN
    ));
    assert!(matches!(
        runtime.stop(UNKNOWN, &token),
        Err(RuntimeError::UnknownCore(core)) if core == UNKNOWN
    ));
}

#[test]
fn stop_preserves_core_database_and_manifest_hashes() {
    let (_temp, paths, installer, token) = setup();
    install(&installer, &token, CORE_A, 1);
    let runtime = CoreRuntime::new(paths.clone());
    let database = installer.layout().database_path(CORE_A).expect("database");
    let manifest = installer.layout().integrity_path(CORE_A).expect("manifest");
    let database_hash = compute_file_sha256(&database).expect("database hash");
    let manifest_hash = compute_file_sha256(&manifest).expect("manifest hash");

    runtime.start(CORE_A, &token).expect("start");
    runtime.stop(CORE_A, &token).expect("stop");

    assert_eq!(
        compute_file_sha256(&database).expect("database hash after stop"),
        database_hash
    );
    assert_eq!(
        compute_file_sha256(&manifest).expect("manifest hash after stop"),
        manifest_hash
    );
    assert!(installer.verify(CORE_A).is_ok());
}

#[test]
fn missing_heartbeat_is_reported_stale_not_running() {
    let (_temp, paths, installer, token) = setup();
    install(&installer, &token, CORE_A, 1);
    let runtime = CoreRuntime::new(paths.clone());
    runtime.start(CORE_A, &token).expect("start");

    let sentinel = stop_sentinel_path(paths.root(), CORE_A);
    fs::write(&sentinel, b"stop").expect("write sentinel");
    let heartbeat = heartbeat_path(paths.root(), CORE_A);
    let deadline = Instant::now() + Duration::from_secs(2);
    while heartbeat.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(25));
    }
    assert!(!heartbeat.exists(), "host must remove its heartbeat");

    let stale = runtime.status(CORE_A).expect("status");
    assert_eq!(stale.state, CoreRuntimeState::Stale);
    assert_eq!(stale.recorded_state, CoreRuntimeState::Running);
    assert!(!stale.heartbeat_fresh);
    assert!(!stale.is_running());

    runtime.stop(CORE_A, &token).expect("stop stale Core");
    assert_eq!(
        runtime.status(CORE_A).expect("stopped").state,
        CoreRuntimeState::Stopped
    );
}

#[test]
fn one_core_runtime_failure_does_not_affect_another() {
    let (_temp, paths, installer, token) = setup();
    install(&installer, &token, CORE_A, 1);
    install(&installer, &token, CORE_B, 2);
    let runtime = CoreRuntime::new(paths.clone());
    runtime.start(CORE_A, &token).expect("start A");

    let registry = CoreRegistry::new(paths.clone());
    registry.initialize().expect("registry");
    registry
        .register(CoreRegistrationRequest {
            schema: CORE_REGISTRY_SCHEMA.to_owned(),
            core_uuid: CORE_C.to_owned(),
            core_number: 3,
            display_name: "Phantom Core".to_owned(),
            storage_relative_path: format!("cores/{CORE_C}/core.db"),
            initialized_at: now_rfc3339().expect("timestamp"),
        })
        .expect("register phantom Core");

    let failure = runtime.start(CORE_C, &token).expect_err("missing database");
    assert!(matches!(failure, RuntimeError::SpawnFailure(_)));

    let a = runtime.status(CORE_A).expect("A status");
    assert_eq!(a.state, CoreRuntimeState::Running);
    assert!(a.heartbeat_fresh);
    let c = runtime.status(CORE_C).expect("C status");
    assert_eq!(c.state, CoreRuntimeState::Failed);

    runtime.stop(CORE_A, &token).expect("stop A");
    runtime.stop(CORE_C, &token).expect("stop C");
}

#[test]
fn retired_core_is_rejected_without_affecting_running_core() {
    let (_temp, paths, installer, token) = setup();
    install(&installer, &token, CORE_A, 1);
    install(&installer, &token, CORE_B, 2);
    let runtime = CoreRuntime::new(paths.clone());
    runtime.start(CORE_B, &token).expect("start B");

    let connection = open_initialized(
        &paths.platform_db(),
        PLATFORM_APPLICATION_ID,
        "core_registry",
    )
    .expect("platform db");
    connection
        .execute(
            "UPDATE core_registry SET lifecycle_state = 'retired' WHERE core_uuid = ?1",
            [CORE_A],
        )
        .expect("retire A");

    let error = runtime.start(CORE_A, &token).expect_err("retired");
    assert!(matches!(
        error,
        RuntimeError::IllegalLifecycle { ref from, ref to }
            if from == "retired" && to == "running"
    ));

    let b = runtime.status(CORE_B).expect("B status");
    assert_eq!(b.state, CoreRuntimeState::Running);
    assert!(b.heartbeat_fresh);
    assert_eq!(runtime.list().expect("list").len(), 2);

    runtime.stop(CORE_B, &token).expect("stop B");
}

#[test]
fn runtime_start_and_stop_keep_registry_lifecycle_consistent() {
    let (_temp, paths, installer, token) = setup();
    install(&installer, &token, CORE_A, 1);
    let runtime = CoreRuntime::new(paths.clone());
    let registry = CoreRegistry::new(paths.clone());

    runtime.start(CORE_A, &token).expect("start");
    assert_eq!(
        registry
            .get(CORE_A)
            .expect("get")
            .expect("entry")
            .lifecycle_state,
        aegis_contracts::lifecycle::CoreLifecycleState::Active
    );

    runtime.stop(CORE_A, &token).expect("stop");
    assert_eq!(
        registry
            .get(CORE_A)
            .expect("get")
            .expect("entry")
            .lifecycle_state,
        aegis_contracts::lifecycle::CoreLifecycleState::Suspended
    );

    runtime.start(CORE_A, &token).expect("restart after stop");
    assert_eq!(
        runtime.status(CORE_A).expect("status").state,
        CoreRuntimeState::Running
    );
    runtime.stop(CORE_A, &token).expect("final stop");
}

#[test]
fn stop_blocks_start_and_halts_a_running_core() {
    let (temp, paths, installer, token) = setup();
    install(&installer, &token, CORE_A, 1);
    let runtime = CoreRuntime::new(paths.clone());
    let service = PlatformService::start(temp.path()).expect("platform service");

    service.set_stop("runtime stop test").expect("set stop");
    let error = runtime
        .start(CORE_A, &token)
        .expect_err("STOP must block start");
    assert!(matches!(error, RuntimeError::StopActive));
    assert!(!heartbeat_path(paths.root(), CORE_A).exists());

    service.clear_stop().expect("clear stop");
    runtime.start(CORE_A, &token).expect("start after clear");

    service
        .set_stop("stop running core")
        .expect("set stop while running");
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let status = runtime.status(CORE_A).expect("status");
        if status.state != CoreRuntimeState::Running {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Core host must observe STOP and stop heartbeating"
        );
        thread::sleep(Duration::from_millis(50));
    }

    let stopped = runtime.stop(CORE_A, &token).expect("stop after STOP");
    assert_eq!(stopped.state, CoreRuntimeState::Stopped);
    service.clear_stop().expect("final clear");
}

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_contracts::lifecycle::CoreLifecycleState;
use aegis_core_install::CoreInstaller;
use aegis_core_registry::CoreRegistry;
use aegis_core_runtime::{heartbeat_path, CoreRuntime, CoreRuntimeState, RuntimeError};
use aegis_platform::PlatformService;
use aegis_safe::SafeStore;
use aegis_storage::{now_rfc3339, StoragePaths};

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";

fn setup() -> (tempfile::TempDir, StoragePaths, CoreInstaller, String) {
    let temp = tempfile::tempdir().expect("tempdir");
    let initialized = PlatformService::initialize(temp.path()).expect("platform init");
    let installer = CoreInstaller::new(temp.path()).expect("installer");
    let paths = StoragePaths::new(temp.path()).expect("paths");
    (temp, paths, installer, initialized.owner_token)
}

fn request(core_uuid: &str) -> CoreInitializationRequest {
    CoreInitializationRequest {
        schema: CORE_INIT_REQUEST_SCHEMA.to_owned(),
        core_uuid: core_uuid.to_owned(),
        core_number: 1,
        display_name: "Stop Guard".to_owned(),
        requested_at: now_rfc3339().expect("timestamp"),
    }
}

#[test]
fn active_stop_blocks_runtime_start_and_leaves_no_effect() {
    let (_temp, paths, installer, token) = setup();
    installer
        .initialize(&request(CORE_A), &token)
        .expect("install Core");

    let runtime = CoreRuntime::new(paths.clone());
    let safe = SafeStore::new(paths.clone());
    safe.set_stop("tester", "security acceptance", &token)
        .expect("set stop");

    let heartbeat = heartbeat_path(paths.root(), CORE_A);
    let error = runtime
        .start(CORE_A, &token)
        .expect_err("STOP must block runtime start");
    assert!(matches!(error, RuntimeError::StopActive));
    assert!(!heartbeat.exists(), "blocked start must not spawn a host");
    assert_eq!(
        runtime.status(CORE_A).expect("status").state,
        CoreRuntimeState::Stopped
    );

    safe.clear_stop("tester", &token).expect("clear stop");
    let started = runtime.start(CORE_A, &token).expect("start after clear");
    assert_eq!(started.state, CoreRuntimeState::Running);
    runtime.stop(CORE_A, &token).expect("stop");
}

#[test]
fn stop_survives_reopen_and_still_blocks_start() {
    let (_temp, paths, installer, token) = setup();
    installer
        .initialize(&request(CORE_A), &token)
        .expect("install Core");

    {
        let safe = SafeStore::new(paths.clone());
        safe.set_stop("tester", "persisted stop", &token)
            .expect("set stop");
    }

    let reopened_safe = SafeStore::new(paths.clone());
    assert!(reopened_safe.stop_status().expect("status").active);
    let runtime = CoreRuntime::new(paths.clone());
    assert!(matches!(
        runtime.start(CORE_A, &token),
        Err(RuntimeError::StopActive)
    ));
}

#[test]
fn stop_revision_is_monotonic_across_reopens() {
    let (_temp, paths, installer, token) = setup();
    installer
        .initialize(&request(CORE_A), &token)
        .expect("install Core");

    let first = SafeStore::new(paths.clone())
        .set_stop("tester", "persisted stop", &token)
        .expect("set stop");
    assert_eq!(first.revision, 1);

    for _ in 0..2 {
        let reopened = SafeStore::new(paths.clone());
        let status = reopened.stop_status().expect("status");
        assert!(status.active);
        assert_eq!(status.revision, 1, "reopen must not bump the revision");
    }

    let cleared = SafeStore::new(paths.clone())
        .clear_stop("tester", &token)
        .expect("clear stop");
    assert!(!cleared.active);
    assert_eq!(cleared.revision, 2);

    let reopened = SafeStore::new(paths.clone());
    assert!(!reopened.stop_status().expect("status").active);
    assert!(reopened.audit_chain_valid().expect("audit chain"));
}

#[test]
fn stop_blocks_runtime_start_without_reactivating_suspended_core() {
    let (_temp, paths, installer, token) = setup();
    installer
        .initialize(&request(CORE_A), &token)
        .expect("install Core");
    let registry = CoreRegistry::new(paths.clone());
    let suspended = installer
        .deactivate(CORE_A, "maintenance", &token)
        .expect("deactivate");
    assert_eq!(suspended.lifecycle_state, CoreLifecycleState::Suspended);

    let safe = SafeStore::new(paths.clone());
    safe.set_stop("tester", "operator halt", &token)
        .expect("set stop");

    let runtime = CoreRuntime::new(paths.clone());
    let error = runtime
        .start(CORE_A, &token)
        .expect_err("STOP must block runtime start");
    assert!(matches!(error, RuntimeError::StopActive));
    assert!(
        !heartbeat_path(paths.root(), CORE_A).exists(),
        "blocked start must not spawn a host"
    );
    assert_eq!(
        registry
            .get(CORE_A)
            .expect("registry get")
            .expect("entry")
            .lifecycle_state,
        CoreLifecycleState::Suspended,
        "a blocked start must not reactivate the Core"
    );

    safe.clear_stop("tester", &token).expect("clear stop");
    let started = runtime.start(CORE_A, &token).expect("start after clear");
    assert_eq!(started.state, CoreRuntimeState::Running);
    assert_eq!(
        registry
            .get(CORE_A)
            .expect("registry get")
            .expect("entry")
            .lifecycle_state,
        CoreLifecycleState::Active
    );
    runtime.stop(CORE_A, &token).expect("stop");
}

#[test]
fn unauthorized_runtime_error_does_not_leak_token_or_paths() {
    let (_temp, paths, installer, token) = setup();
    installer
        .initialize(&request(CORE_A), &token)
        .expect("install Core");

    let root = paths.root().to_string_lossy().to_string();
    let bogus = "not-the-owner-token";
    let runtime = CoreRuntime::new(paths.clone());
    let error = runtime
        .start(CORE_A, bogus)
        .expect_err("wrong token must be rejected");
    assert!(matches!(error, RuntimeError::Unauthorized));
    let text = error.to_string();
    assert!(!text.contains(bogus), "candidate token leaked: {text}");
    assert!(!text.contains(&token), "owner token leaked: {text}");
    assert!(!text.contains("owner.proof"), "proof file leaked: {text}");
    assert!(!text.contains(&root), "storage root leaked: {text}");
    assert!(!heartbeat_path(paths.root(), CORE_A).exists());
}

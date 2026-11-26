use std::path::Path;

use aegis_contracts::ipc::{
    IPC_PROTOCOL_VERSION, METHOD_PLATFORM_SHUTDOWN, METHOD_PLATFORM_STATUS, METHOD_SAFE_STOP_SET,
};
use aegis_contracts::roster::CORE_REGISTRY_SCHEMA;
use aegis_contracts::status::PLATFORM_STATUS_SCHEMA;
use aegis_platform::{PlatformService, ServiceError};
use aegis_storage::{open_initialized, StoragePaths, PLATFORM_APPLICATION_ID};
use rusqlite::params;
use serde_json::json;

const CORE_REGISTRY_DDL: &str = "CREATE TABLE IF NOT EXISTS core_registry (
    core_uuid TEXT PRIMARY KEY,
    core_number INTEGER NOT NULL UNIQUE CHECK(core_number IN (1, 2, 3, 6, 7)),
    display_name TEXT NOT NULL,
    lifecycle_state TEXT NOT NULL CHECK(lifecycle_state IN (
        'design_only', 'approved', 'installed', 'active', 'suspended', 'retired'
    )),
    storage_relative_path TEXT NOT NULL UNIQUE,
    initialized_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    schema TEXT NOT NULL
);";

fn platform_connection(root: &Path) -> rusqlite::Connection {
    let paths = StoragePaths::new(root).expect("storage paths");
    open_initialized(&paths.platform_db(), PLATFORM_APPLICATION_ID, "platform")
        .expect("platform database")
}

fn seed_legacy_cores(root: &Path, cores: &[(u8, &str)]) {
    let connection = platform_connection(root);
    for (number, uuid) in cores {
        connection
            .execute(
                "INSERT INTO platform_core(core_uuid, core_number, lifecycle_state, created_at)
                 VALUES(?1, ?2, 'installed', ?3)",
                params![uuid, number, "2026-11-26T00:00:00+00:00"],
            )
            .expect("legacy core row");
    }
}

fn seed_registry_cores(root: &Path, cores: &[(u8, &str)]) {
    let connection = platform_connection(root);
    connection
        .execute_batch(CORE_REGISTRY_DDL)
        .expect("core registry schema");
    for (number, uuid) in cores {
        connection
            .execute(
                "INSERT INTO core_registry(
                    schema, core_uuid, core_number, display_name, lifecycle_state,
                    storage_relative_path, initialized_at, updated_at
                 ) VALUES(?1, ?2, ?3, ?4, 'installed', ?5, ?6, ?6)",
                params![
                    CORE_REGISTRY_SCHEMA,
                    uuid,
                    number,
                    format!("Core {number}"),
                    format!("cores/{uuid}/core.db"),
                    "2026-11-26T00:00:00+00:00",
                ],
            )
            .expect("registry core row");
    }
}

#[test]
fn zero_core_start_status_shutdown_and_reopen_keep_host_identity() {
    let temp = tempfile::tempdir().expect("tempdir");
    let initialized = PlatformService::initialize(temp.path()).expect("initialize");
    let service = PlatformService::start(temp.path()).expect("start");

    let status = service.status().expect("status");
    assert_eq!(status.schema, PLATFORM_STATUS_SCHEMA);
    assert_eq!(status.protocol, IPC_PROTOCOL_VERSION);
    assert_eq!(status.platform_id, initialized.platform_id);
    assert_eq!(status.callsign, initialized.callsign);
    assert!(status.callsign.starts_with("AEGIS-PLATFORM-"));
    assert!(status.zero_core);
    assert_eq!(status.core_count, 0);
    assert!(!status.stop_active);
    assert_eq!(status.authority, "local_single");
    assert!(!status.generated_at.is_empty());

    let token = service.owner_token().expect("owner token");
    assert_eq!(token, initialized.owner_token);

    let dispatched = service
        .dispatch(METHOD_PLATFORM_STATUS, &json!({}), &token)
        .expect("status dispatch");
    assert_eq!(dispatched["schema"].as_str(), Some(PLATFORM_STATUS_SCHEMA));
    assert_eq!(
        dispatched["platform_id"].as_str(),
        Some(initialized.platform_id.as_str())
    );
    assert_eq!(dispatched["zero_core"].as_bool(), Some(true));
    assert_eq!(dispatched["core_count"].as_u64(), Some(0));

    let shutdown = service
        .dispatch(METHOD_PLATFORM_SHUTDOWN, &json!({}), &token)
        .expect("shutdown dispatch");
    assert_eq!(shutdown["ok"].as_bool(), Some(true));
    assert_eq!(shutdown["shutdown"].as_bool(), Some(true));
    assert!(service.shutdown_requested());
    service.request_shutdown();
    assert!(service.shutdown_requested());
    drop(service);

    let reopened = PlatformService::start(temp.path()).expect("reopen");
    assert!(!reopened.shutdown_requested());
    assert_eq!(
        reopened.identity().expect("identity").platform_id,
        initialized.platform_id
    );
    assert_eq!(
        reopened.owner_token().expect("owner token"),
        initialized.owner_token
    );
    let reopened_status = reopened.status().expect("status after reopen");
    assert!(reopened_status.zero_core);
    assert_eq!(reopened_status.core_count, 0);
    assert_eq!(reopened_status.platform_id, initialized.platform_id);
}

#[test]
fn multi_core_discovery_prefers_core_registry_and_survives_reopen() {
    let temp = tempfile::tempdir().expect("tempdir");
    PlatformService::initialize(temp.path()).expect("initialize");
    let service = PlatformService::start(temp.path()).expect("start");

    let empty = service.status().expect("zero-core status");
    assert!(empty.zero_core);
    assert_eq!(empty.core_count, 0);

    seed_legacy_cores(
        temp.path(),
        &[
            (1, "11111111-1111-4111-8111-111111111111"),
            (2, "22222222-2222-4222-8222-222222222222"),
        ],
    );
    let legacy = service.status().expect("legacy registry status");
    assert!(!legacy.zero_core);
    assert_eq!(legacy.core_count, 2);

    seed_registry_cores(
        temp.path(),
        &[
            (1, "11111111-1111-4111-8111-111111111111"),
            (2, "22222222-2222-4222-8222-222222222222"),
            (3, "33333333-3333-4333-8333-333333333333"),
        ],
    );
    let registry = service.status().expect("core registry status");
    assert!(!registry.zero_core);
    assert_eq!(registry.core_count, 3);
    drop(service);

    let reopened = PlatformService::start(temp.path()).expect("reopen");
    let reopened_status = reopened.status().expect("status after reopen");
    assert!(!reopened_status.zero_core);
    assert_eq!(reopened_status.core_count, 3);
}

#[test]
fn shutdown_does_not_clear_persisted_stop_state_across_reopen() {
    let temp = tempfile::tempdir().expect("tempdir");
    let initialized = PlatformService::initialize(temp.path()).expect("initialize");
    let service = PlatformService::start(temp.path()).expect("start");
    let token = service.owner_token().expect("owner token");

    let stopped = service
        .dispatch(
            METHOD_SAFE_STOP_SET,
            &json!({"reason": "lifecycle test", "updated_by": "owner"}),
            &token,
        )
        .expect("set stop");
    assert_eq!(stopped["active"].as_bool(), Some(true));
    assert!(service.status().expect("status").stop_active);
    assert!(matches!(
        service.effect_ping(),
        Err(ServiceError::StopActive)
    ));

    let shutdown = service
        .dispatch(METHOD_PLATFORM_SHUTDOWN, &json!({}), &token)
        .expect("shutdown dispatch");
    assert_eq!(shutdown["shutdown"].as_bool(), Some(true));
    drop(service);

    let reopened = PlatformService::start(temp.path()).expect("reopen");
    assert!(!reopened.shutdown_requested());
    let status = reopened.status().expect("status after reopen");
    assert_eq!(status.platform_id, initialized.platform_id);
    assert!(status.stop_active);
    assert!(matches!(
        reopened.effect_ping(),
        Err(ServiceError::StopActive)
    ));

    let cleared = reopened.clear_stop().expect("clear stop");
    assert!(!cleared.active);
    assert!(!reopened.status().expect("status after clear").stop_active);
    assert!(reopened.effect_ping().expect("effect ping").ok);
}

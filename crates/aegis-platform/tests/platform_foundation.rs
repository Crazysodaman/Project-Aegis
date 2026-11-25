use aegis_contracts::ipc::{METHOD_PLATFORM_EFFECT_PING, METHOD_PLATFORM_STATUS};
use aegis_contracts::status::EffectPingResult;
use aegis_platform::{PlatformService, ServiceError};
use aegis_safe::SafeStore;
use aegis_storage::{open_read_only, StoragePaths, PLATFORM_APPLICATION_ID, SAFE_APPLICATION_ID};
use serde_json::json;

#[test]
fn platform_starts_with_zero_cores_and_separate_databases() {
    let temp = tempfile::tempdir().expect("tempdir");
    let initialized = PlatformService::initialize(temp.path()).expect("initialize");
    assert!(!initialized.platform_id.is_empty());
    assert!(initialized.callsign.starts_with("AEGIS-PLATFORM-"));
    assert!(!initialized.callsign.to_lowercase().contains("sofia"));
    assert!(!initialized.owner_token.is_empty());

    let service = PlatformService::start(temp.path()).expect("start");
    let status = service.status().expect("status");
    assert!(status.zero_core);
    assert_eq!(status.core_count, 0);
    assert!(!status.stop_active);
    assert_eq!(status.authority, "local_single");
    assert_eq!(status.protocol, "1.0");

    let paths = StoragePaths::new(temp.path()).expect("paths");
    let platform =
        open_read_only(&paths.platform_db(), PLATFORM_APPLICATION_ID).expect("platform read-only");
    let safe = open_read_only(&paths.safe_db(), SAFE_APPLICATION_ID).expect("safe read-only");

    let platform_tables: Vec<String> = platform
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .expect("prepare")
        .query_map([], |row| row.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("tables");
    assert!(platform_tables
        .iter()
        .any(|name| name == "platform_identity"));
    assert!(!platform_tables.iter().any(|name| name == "safe_stop"));

    let safe_tables: Vec<String> = safe
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .expect("prepare")
        .query_map([], |row| row.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("tables");
    assert!(safe_tables.iter().any(|name| name == "safe_stop"));
    assert!(!safe_tables.iter().any(|name| name == "platform_identity"));

    let core_count: i64 = platform
        .query_row("SELECT COUNT(*) FROM platform_core", [], |row| row.get(0))
        .expect("core count");
    assert_eq!(core_count, 0);
}

#[test]
fn platform_identity_survives_restart() {
    let temp = tempfile::tempdir().expect("tempdir");
    let first = PlatformService::initialize(temp.path()).expect("initialize");
    drop(first);
    let restarted = PlatformService::start(temp.path()).expect("restart");
    assert_eq!(
        restarted.identity().expect("identity").platform_id,
        PlatformService::start(temp.path())
            .expect("second restart")
            .identity()
            .expect("identity")
            .platform_id
    );
}

#[test]
fn stop_blocks_effects_and_can_be_cleared_by_owner() {
    let temp = tempfile::tempdir().expect("tempdir");
    PlatformService::initialize(temp.path()).expect("initialize");
    let service = PlatformService::start(temp.path()).expect("start");
    assert_eq!(service.effect_ping().expect("ping"), EffectPingResult::ok());

    let stopped = service.set_stop("batch1 test").expect("set stop");
    assert!(stopped.active);
    assert!(service.status().expect("status").stop_active);
    assert!(matches!(
        service.effect_ping(),
        Err(ServiceError::StopActive)
    ));

    let cleared = service.clear_stop().expect("clear stop");
    assert!(!cleared.active);
    assert_eq!(
        service.effect_ping().expect("ping after clear"),
        EffectPingResult::ok()
    );

    let paths = StoragePaths::new(temp.path()).expect("paths");
    let safe = SafeStore::new(paths);
    assert!(safe.audit_chain_valid().expect("audit chain"));
    assert!(safe.audit_count().expect("audit count") >= 4);
}

#[test]
fn unauthorized_token_is_denied() {
    let temp = tempfile::tempdir().expect("tempdir");
    PlatformService::initialize(temp.path()).expect("initialize");
    let service = PlatformService::start(temp.path()).expect("start");
    assert!(matches!(
        service.dispatch(METHOD_PLATFORM_EFFECT_PING, &json!({}), "wrong-token"),
        Err(ServiceError::Unauthorized)
    ));
    assert!(matches!(
        service.dispatch(METHOD_PLATFORM_STATUS, &json!({}), "wrong-token"),
        Err(ServiceError::Unauthorized)
    ));
}

#[test]
fn second_initialize_reuses_identity_and_owner() {
    let temp = tempfile::tempdir().expect("tempdir");
    let first = PlatformService::initialize(temp.path()).expect("initialize");
    let second = PlatformService::initialize(temp.path()).expect("initialize again");
    assert_eq!(first.platform_id, second.platform_id);
    assert_eq!(first.callsign, second.callsign);
    assert_eq!(first.owner_token, second.owner_token);
}

#[test]
fn platform_sources_do_not_reference_sofia_identity() {
    use std::fs;
    use std::path::Path;
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for entry in fs::read_dir(source_root).expect("source dir") {
        let entry = entry.expect("entry");
        if entry.path().extension().and_then(|value| value.to_str()) != Some("rs") {
            continue;
        }
        let source = fs::read_to_string(entry.path()).expect("source");
        assert!(
            !source.to_lowercase().contains("sofia"),
            "Platform source must remain Core-neutral: {}",
            entry.path().display()
        );
    }
}

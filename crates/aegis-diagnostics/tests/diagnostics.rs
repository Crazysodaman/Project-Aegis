use std::fs;
use std::io::Write;
use std::path::Path;

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_core_install::CoreInstaller;
use aegis_diagnostics::{Diagnostics, ModuleStatus, OverallStatus, MAX_REPORTED_REASON_CHARS};
use aegis_platform::PlatformService;
use aegis_storage::now_rfc3339;
use rusqlite::Connection;

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_B: &str = "11111111-1111-4111-8111-111111111111";

fn request(core_uuid: &str, core_number: u8, display_name: &str) -> CoreInitializationRequest {
    CoreInitializationRequest {
        schema: CORE_INIT_REQUEST_SCHEMA.to_owned(),
        core_uuid: core_uuid.to_owned(),
        core_number,
        display_name: display_name.to_owned(),
        requested_at: now_rfc3339().expect("timestamp"),
    }
}

fn setup_platform(root: &Path) -> (CoreInstaller, String) {
    let initialized = PlatformService::initialize(root).expect("platform initialize");
    let installer = CoreInstaller::new(root).expect("installer");
    (installer, initialized.owner_token)
}

fn install(installer: &CoreInstaller, token: &str, core_uuid: &str, number: u8, name: &str) {
    installer
        .initialize(&request(core_uuid, number, name), token)
        .expect("install core");
}

fn collect(root: &Path) -> aegis_diagnostics::DiagnosticsReport {
    Diagnostics::new(root).expect("diagnostics").collect()
}

#[test]
fn zero_core_platform_is_healthy() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (_installer, _token) = setup_platform(temp.path());

    let report = collect(temp.path());

    assert_eq!(report.status, OverallStatus::Healthy);
    assert!(report.reasons.is_empty(), "reasons: {:?}", report.reasons);
    assert!(report.platform.available);
    assert_eq!(report.platform.zero_core, Some(true));
    assert_eq!(report.platform.core_count, Some(0));
    assert_eq!(report.platform.stop_active, Some(false));
    assert_eq!(report.platform.authority.as_deref(), Some("local_single"));
    assert!(report.safe.available);
    assert!(report.safe.owner_proof_present);
    assert_eq!(report.safe.stop_active, Some(false));
    assert_eq!(report.cores.count, 0);
    assert!(report.cores.entries.is_empty());
    assert_eq!(report.modules.status, ModuleStatus::Available);
    assert_eq!(report.modules.count, Some(0));
}

#[test]
fn two_verified_cores_are_healthy() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (installer, token) = setup_platform(temp.path());
    install(&installer, &token, CORE_A, 1, "Core A");
    install(&installer, &token, CORE_B, 2, "Core B");

    let report = collect(temp.path());

    assert_eq!(report.status, OverallStatus::Healthy);
    assert!(report.reasons.is_empty(), "reasons: {:?}", report.reasons);
    assert_eq!(report.platform.core_count, Some(2));
    assert_eq!(report.cores.count, 2);
    assert_eq!(report.cores.entries.len(), 2);
    for core in &report.cores.entries {
        assert_eq!(core.integrity.status, "verified");
        assert!(core.database_bytes.is_some_and(|bytes| bytes > 0));
        assert!(core.database_path.ends_with("core.db"));
    }
}

#[test]
fn tampered_core_degrades_report_with_detail() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (installer, token) = setup_platform(temp.path());
    install(&installer, &token, CORE_A, 1, "Core A");
    install(&installer, &token, CORE_B, 2, "Core B");

    let a_path = installer.layout().database_path(CORE_A).expect("A path");
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&a_path)
        .expect("open A");
    file.write_all(b"tamper").expect("tamper");
    file.sync_all().expect("sync");
    drop(file);

    let report = collect(temp.path());

    assert_eq!(report.status, OverallStatus::Degraded);
    assert!(report.reasons.iter().any(|reason| reason.contains(CORE_A)));
    let a = report
        .cores
        .entries
        .iter()
        .find(|core| core.core_uuid == CORE_A)
        .expect("A entry");
    assert_eq!(a.integrity.status, "digest_mismatch");
    assert_ne!(a.integrity.actual_sha256, a.integrity.expected_sha256);
    let b = report
        .cores
        .entries
        .iter()
        .find(|core| core.core_uuid == CORE_B)
        .expect("B entry");
    assert_eq!(b.integrity.status, "verified");
}

#[test]
fn missing_core_database_is_reported_per_core() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (installer, token) = setup_platform(temp.path());
    install(&installer, &token, CORE_A, 1, "Core A");

    let a_path = installer.layout().database_path(CORE_A).expect("A path");
    fs::remove_file(&a_path).expect("remove core database");

    let report = collect(temp.path());

    assert_eq!(report.status, OverallStatus::Degraded);
    let a = &report.cores.entries[0];
    assert_eq!(a.core_uuid, CORE_A);
    assert_eq!(a.integrity.status, "missing");
    assert_eq!(a.database_bytes, None);
}

#[test]
fn missing_manifest_is_reported_per_core() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (installer, token) = setup_platform(temp.path());
    install(&installer, &token, CORE_A, 1, "Core A");

    let manifest = installer
        .layout()
        .integrity_path(CORE_A)
        .expect("A manifest path");
    fs::remove_file(&manifest).expect("remove manifest");

    let report = collect(temp.path());

    assert_eq!(report.status, OverallStatus::Degraded);
    let a = &report.cores.entries[0];
    assert_eq!(a.integrity.status, "missing_manifest");
    assert!(a.database_bytes.is_some());
}

#[test]
fn active_stop_is_included() {
    let temp = tempfile::tempdir().expect("tempdir");
    setup_platform(temp.path());
    PlatformService::start(temp.path())
        .expect("start")
        .set_stop("diagnostics test stop")
        .expect("set stop");

    let report = collect(temp.path());

    assert_eq!(report.platform.stop_active, Some(true));
    assert_eq!(report.safe.stop_active, Some(true));
    assert!(report
        .safe
        .stop_revision
        .is_some_and(|revision| revision >= 1));
}

#[test]
fn absent_module_table_degrades_with_reason() {
    let temp = tempfile::tempdir().expect("tempdir");
    setup_platform(temp.path());

    let platform_db = temp.path().join("platform.db");
    let connection = Connection::open(&platform_db).expect("open platform db");
    connection
        .execute_batch(
            "DROP TABLE aegis_module_events;
             DROP TABLE aegis_modules;",
        )
        .expect("drop module tables");
    drop(connection);

    let report = collect(temp.path());

    assert_eq!(report.modules.status, ModuleStatus::Unavailable);
    assert_eq!(report.modules.count, None);
    assert_eq!(report.status, OverallStatus::Degraded);
    assert!(report
        .reasons
        .iter()
        .any(|reason| reason.contains("Module registry is unavailable")));
}

#[test]
fn present_module_table_is_counted_read_only() {
    let temp = tempfile::tempdir().expect("tempdir");
    setup_platform(temp.path());

    let platform_db = temp.path().join("platform.db");
    let connection = Connection::open(&platform_db).expect("open platform db");
    connection
        .execute_batch(
            "DROP TABLE aegis_module_events;
             DROP TABLE aegis_modules;
             CREATE TABLE aegis_modules (
                module_id TEXT PRIMARY KEY,
                display_name TEXT NOT NULL
            );
            INSERT INTO aegis_modules(module_id, display_name) VALUES('m1', 'Module One');
            INSERT INTO aegis_modules(module_id, display_name) VALUES('m2', 'Module Two');",
        )
        .expect("create module table");
    drop(connection);

    let report = collect(temp.path());

    assert_eq!(report.modules.status, ModuleStatus::Available);
    assert_eq!(report.modules.count, Some(2));
    assert_eq!(report.status, OverallStatus::Healthy);
}

#[test]
fn diagnostics_does_not_create_or_modify_files() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (installer, token) = setup_platform(temp.path());
    install(&installer, &token, CORE_A, 1, "Core A");

    let before = snapshot_tree(temp.path());
    let _ = collect(temp.path());
    let after = snapshot_tree(temp.path());

    assert_eq!(before, after);
}

#[test]
fn diagnostics_does_not_create_a_missing_root() {
    let temp = tempfile::tempdir().expect("tempdir");
    let missing = temp.path().join("does-not-exist");
    let report = Diagnostics::new(&missing)
        .expect("absolute missing root is accepted")
        .collect();

    assert!(!missing.exists(), "diagnostics must not create the root");
    assert_eq!(report.status, OverallStatus::Critical);
    assert!(!report.platform.available);
    assert!(!report.safe.available);
}

#[test]
fn long_stop_reason_is_bounded_in_report() {
    let temp = tempfile::tempdir().expect("tempdir");
    setup_platform(temp.path());
    let reason = "r".repeat(400);
    PlatformService::start(temp.path())
        .expect("start")
        .set_stop(&reason)
        .expect("set stop");

    let report = collect(temp.path());

    let reported = report.safe.stop_reason.expect("reason");
    assert!(reported.chars().count() <= MAX_REPORTED_REASON_CHARS + 3);
    assert!(reported.ends_with("..."));
}

fn snapshot_tree(root: &Path) -> std::collections::BTreeMap<String, (u64, String)> {
    let mut snapshot = std::collections::BTreeMap::new();
    collect_files(root, root, &mut snapshot);
    snapshot
}

fn collect_files(
    root: &Path,
    directory: &Path,
    snapshot: &mut std::collections::BTreeMap<String, (u64, String)>,
) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, snapshot);
            continue;
        }
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let bytes = fs::read(&path).unwrap_or_default();
        let digest = aegis_core_integrity::compute_bytes_sha256(&bytes);
        snapshot.insert(
            relative.to_string_lossy().replace('\\', "/"),
            (bytes.len() as u64, digest),
        );
    }
}

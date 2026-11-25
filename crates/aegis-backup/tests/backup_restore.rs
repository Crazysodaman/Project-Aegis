use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use aegis_backup::{
    backup, list_backups, read_manifest, restore, verify_backup, BackupError,
    BackupVerificationStatus, RestoreOptions, VerificationIssueKind, BACKUP_MANIFEST_FILE,
};
use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_core_install::CoreInstaller;
use aegis_platform::PlatformService;
use aegis_storage::now_rfc3339;
use serde_json::json;

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_B: &str = "b9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_C: &str = "c9a0a604-a4e7-4a88-a985-3f071e2c8e7d";

fn init_platform() -> (tempfile::TempDir, String) {
    let temp = tempfile::tempdir().expect("tempdir");
    let initialized = PlatformService::initialize(temp.path()).expect("platform init");
    (temp, initialized.owner_token)
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

fn install(root: &Path, token: &str, core_uuid: &str, core_number: u8, name: &str) {
    let installer = CoreInstaller::new(root).expect("installer");
    installer
        .initialize(&request(core_uuid, core_number, name), token)
        .expect("initialize Core");
}

fn setup_two_cores() -> (tempfile::TempDir, String) {
    let (temp, token) = init_platform();
    install(temp.path(), &token, CORE_A, 1, "Core A");
    install(temp.path(), &token, CORE_B, 2, "Core B");
    (temp, token)
}

fn backup_path() -> (tempfile::TempDir, PathBuf) {
    let parent = tempfile::tempdir().expect("backup parent");
    let destination = parent.path().join("snapshot");
    (parent, destination)
}

fn target_path() -> (tempfile::TempDir, PathBuf) {
    let parent = tempfile::tempdir().expect("target parent");
    let target = parent.path().join("restored");
    (parent, target)
}

#[test]
fn backs_up_zero_core_platform_and_restores_it() {
    let (source, _token) = init_platform();
    let (_backup_parent, destination) = backup_path();

    let report = backup(source.path(), &destination).expect("backup");
    assert_eq!(report.file_count, 3);
    assert_eq!(
        report.total_bytes,
        report.manifest.files.iter().map(|f| f.bytes).sum::<u64>()
    );
    assert!(report.manifest.cores.is_empty());

    let manifest = read_manifest(&destination).expect("read manifest");
    assert_eq!(manifest.schema, aegis_backup::BACKUP_MANIFEST_SCHEMA);
    assert_eq!(manifest.format_version, aegis_backup::BACKUP_FORMAT_VERSION);
    assert!(!manifest.platform_id.is_empty());
    assert!(!manifest.platform_callsign.is_empty());

    let listed = list_backups(_backup_parent.path()).expect("list backups");
    assert_eq!(listed, vec![destination.clone()]);

    let verification = verify_backup(&destination).expect("verify");
    assert_eq!(verification.status, BackupVerificationStatus::Verified);
    assert!(verification.issues.is_empty());

    let (_target_parent, target) = target_path();
    let restored = restore(&destination, &target, RestoreOptions::default()).expect("restore");
    assert!(restored.restored_cores.is_empty());
    assert!(restored.preserved_source.is_none());
    assert!(target.join("platform.db").is_file());
    assert!(target.join("safe.db").is_file());
    assert!(target.join("owner.proof").is_file());
    assert_eq!(
        fs::read(source.path().join("owner.proof")).expect("source proof"),
        fs::read(target.join("owner.proof")).expect("restored proof")
    );
    assert_eq!(
        verify_backup(&destination)
            .expect("verify after restore")
            .status,
        BackupVerificationStatus::Verified,
        "restore must not modify the backup"
    );
}

#[test]
fn backs_up_two_cores_with_verified_integrity_and_restores_them() {
    let (source, _token) = setup_two_cores();
    let (_backup_parent, destination) = backup_path();

    let report = backup(source.path(), &destination).expect("backup");
    assert_eq!(report.manifest.cores.len(), 2);
    assert!(report
        .manifest
        .cores
        .iter()
        .all(|core| core.integrity_status == "verified"));
    assert!(report
        .manifest
        .cores
        .iter()
        .any(|core| core.core_uuid == CORE_A && core.core_number == 1));
    assert!(report
        .manifest
        .cores
        .iter()
        .any(|core| core.core_uuid == CORE_B && core.core_number == 2));

    let verification = verify_backup(&destination).expect("verify");
    assert_eq!(verification.status, BackupVerificationStatus::Verified);
    assert!(verification.issues.is_empty());

    let (_target_parent, target) = target_path();
    let restored = restore(&destination, &target, RestoreOptions::default()).expect("restore");
    assert_eq!(restored.restored_cores.len(), 2);
    assert!(restored.restored_cores.contains(&CORE_A.to_owned()));
    assert!(restored.restored_cores.contains(&CORE_B.to_owned()));

    let installer = CoreInstaller::new(&target).expect("restored installer");
    assert_eq!(
        installer.verify(CORE_A).expect("verify A").status.as_str(),
        "verified"
    );
    assert_eq!(
        installer.verify(CORE_B).expect("verify B").status.as_str(),
        "verified"
    );
}

#[test]
fn verify_detects_tampered_backup_file() {
    let (source, _token) = setup_two_cores();
    let (_backup_parent, destination) = backup_path();
    backup(source.path(), &destination).expect("backup");

    let tampered = destination.join("cores").join(CORE_A).join("core.db");
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&tampered)
        .expect("open tampered file");
    file.write_all(b"tampered").expect("tamper");
    file.sync_all().expect("sync");
    drop(file);

    let verification = verify_backup(&destination).expect("verify");
    assert_eq!(verification.status, BackupVerificationStatus::Invalid);
    assert!(verification
        .issues
        .iter()
        .any(|issue| issue.kind == VerificationIssueKind::Tampered));
}

#[test]
fn verify_detects_missing_file_and_extra_file() {
    let (source, _token) = setup_two_cores();
    let (_backup_parent, destination) = backup_path();
    backup(source.path(), &destination).expect("backup");

    fs::remove_file(destination.join("safe.db")).expect("remove safe.db");
    let verification = verify_backup(&destination).expect("verify missing");
    assert_eq!(verification.status, BackupVerificationStatus::Invalid);
    assert!(verification
        .issues
        .iter()
        .any(|issue| issue.kind == VerificationIssueKind::Missing));

    let extra = destination.join("cores").join("stray.txt");
    fs::write(&extra, b"unexpected").expect("write extra");
    let verification = verify_backup(&destination).expect("verify extra");
    assert!(verification
        .issues
        .iter()
        .any(|issue| issue.kind == VerificationIssueKind::Extra));
}

#[test]
fn existing_destination_is_refused_without_data_loss() {
    let (source, _token) = setup_two_cores();
    let (_parent, destination) = backup_path();
    fs::create_dir(&destination).expect("create destination");
    let sentinel = destination.join("keep-me.txt");
    fs::write(&sentinel, b"important").expect("write sentinel");

    let error = backup(source.path(), &destination).expect_err("destination exists");
    assert!(matches!(error, BackupError::DestinationExists(_)));
    assert_eq!(fs::read(&sentinel).expect("sentinel"), b"important");
}

#[test]
fn destination_inside_source_is_refused() {
    let (source, _token) = setup_two_cores();
    let destination = source.path().join("nested-backup");

    let error = backup(source.path(), &destination).expect_err("inside source");
    assert!(matches!(error, BackupError::DestinationInsideSource));
    assert!(!destination.exists());
}

#[test]
fn tampered_source_core_fails_closed_without_manifest() {
    let (source, _token) = setup_two_cores();
    let core_db = source.path().join("cores").join(CORE_A).join("core.db");
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&core_db)
        .expect("open core db");
    file.write_all(b"tampered").expect("tamper source");
    file.sync_all().expect("sync");
    drop(file);

    let (_backup_parent, destination) = backup_path();
    let error = backup(source.path(), &destination).expect_err("tampered source");
    assert!(matches!(error, BackupError::IntegrityFailed { .. }));
    assert!(
        !destination.join(BACKUP_MANIFEST_FILE).exists(),
        "a failed backup must not write a manifest"
    );
}

#[test]
fn restore_refuses_non_empty_target_and_preserves_it() {
    let (source, _token) = setup_two_cores();
    let (_backup_parent, destination) = backup_path();
    backup(source.path(), &destination).expect("backup");

    let (_target_parent, target) = target_path();
    fs::create_dir(&target).expect("create target");
    let sentinel = target.join("keep-me.txt");
    fs::write(&sentinel, b"do not delete").expect("write sentinel");

    let error =
        restore(&destination, &target, RestoreOptions::default()).expect_err("non-empty target");
    assert!(matches!(error, BackupError::RestoreTargetNotEmpty(_)));
    assert_eq!(fs::read(&sentinel).expect("sentinel"), b"do not delete");
}

#[test]
fn restore_with_allow_existing_preserves_original_data() {
    let (source, _token) = setup_two_cores();
    let (_backup_parent, destination) = backup_path();
    backup(source.path(), &destination).expect("backup source");

    let (target_temp, target_token) = init_platform();
    install(target_temp.path(), &target_token, CORE_C, 3, "Core C");
    let target = target_temp.path();

    let preserved_parent = tempfile::tempdir().expect("preserved parent");
    let preserved = preserved_parent.path().join("pre-existing");

    let report = restore(
        &destination,
        target,
        RestoreOptions {
            allow_existing: true,
            backup_existing_to: Some(preserved.clone()),
        },
    )
    .expect("allow existing restore");

    assert_eq!(report.preserved_source, Some(preserved.clone()));
    assert!(preserved.join(BACKUP_MANIFEST_FILE).is_file());

    let preserved_installer = CoreInstaller::new(&preserved).expect("preserved installer");
    assert_eq!(
        preserved_installer
            .verify(CORE_C)
            .expect("verify original Core C")
            .status
            .as_str(),
        "verified",
        "original data must survive in the rollback backup"
    );

    let target_installer = CoreInstaller::new(target).expect("target installer");
    assert_eq!(
        target_installer
            .verify(CORE_A)
            .expect("verify restored A")
            .status
            .as_str(),
        "verified"
    );
    assert_eq!(
        target_installer
            .verify(CORE_B)
            .expect("verify restored B")
            .status
            .as_str(),
        "verified"
    );
}

#[test]
fn fresh_heartbeat_refuses_restore() {
    let (source, _token) = setup_two_cores();
    let (_backup_parent, destination) = backup_path();
    backup(source.path(), &destination).expect("backup");

    let run_dir = destination.join("run");
    fs::create_dir_all(&run_dir).expect("run dir");
    let heartbeat = run_dir.join(format!("{CORE_A}.heartbeat"));
    let payload = json!({
        "core_uuid": CORE_A,
        "pid": 4242,
        "timestamp_ms": chrono::Utc::now().timestamp_millis() + 60_000,
    });
    fs::write(&heartbeat, serde_json::to_vec(&payload).expect("heartbeat"))
        .expect("write heartbeat");

    let (_target_parent, target) = target_path();
    let error =
        restore(&destination, &target, RestoreOptions::default()).expect_err("running core");
    assert!(matches!(error, BackupError::RunningCore(core) if core == CORE_A));
    assert!(!target.exists());
}

#[test]
fn running_runtime_row_refuses_allow_existing_restore() {
    let (source, _token) = setup_two_cores();
    let (_backup_parent, destination) = backup_path();
    backup(source.path(), &destination).expect("backup source");

    let (target_temp, target_token) = init_platform();
    install(target_temp.path(), &target_token, CORE_C, 3, "Core C");
    let target = target_temp.path();
    let sentinel = target.join("keep-me.txt");
    fs::write(&sentinel, b"original").expect("write sentinel");

    let platform_db = target.join("platform.db");
    let connection = rusqlite::Connection::open(&platform_db).expect("open target platform");
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS core_runtime_state (
                core_uuid TEXT PRIMARY KEY,
                state TEXT NOT NULL
            );",
        )
        .expect("runtime table");
    connection
        .execute(
            "INSERT INTO core_runtime_state(core_uuid, state) VALUES(?1, 'running')",
            [CORE_C],
        )
        .expect("runtime row");
    drop(connection);

    let preserved_parent = tempfile::tempdir().expect("preserved parent");
    let preserved = preserved_parent.path().join("pre-existing");
    let error = restore(
        &destination,
        target,
        RestoreOptions {
            allow_existing: true,
            backup_existing_to: Some(preserved.clone()),
        },
    )
    .expect_err("running runtime row");
    assert!(matches!(error, BackupError::RunningCore(core) if core == CORE_C));
    assert_eq!(fs::read(&sentinel).expect("sentinel"), b"original");
    assert!(!preserved.exists());
}

#[test]
fn owner_proof_is_copied_byte_for_byte_and_not_embedded() {
    let (source, _token) = setup_two_cores();
    let (_backup_parent, destination) = backup_path();
    backup(source.path(), &destination).expect("backup");

    let source_proof = fs::read(source.path().join("owner.proof")).expect("source proof");
    let copied_proof = fs::read(destination.join("owner.proof")).expect("copied proof");
    assert_eq!(source_proof, copied_proof);

    let manifest_bytes = fs::read(destination.join(BACKUP_MANIFEST_FILE)).expect("manifest");
    assert!(
        !manifest_bytes
            .windows(source_proof.len())
            .any(|window| window == source_proof.as_slice()),
        "the manifest must not contain owner proof bytes"
    );
}

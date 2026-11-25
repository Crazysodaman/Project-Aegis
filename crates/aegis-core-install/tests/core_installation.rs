use std::fs;
use std::path::PathBuf;

use aegis_contracts::initialization::{
    CoreInitializationRequest, InitializationCheckpoint, InitializationStage,
    CORE_INIT_CHECKPOINT_SCHEMA, CORE_INIT_REQUEST_SCHEMA,
};
use aegis_core_identity::write_checkpoint;
use aegis_core_install::{CoreInstaller, InstallError};
use aegis_core_integrity::compute_file_sha256;
use aegis_platform::PlatformService;
use aegis_storage::now_rfc3339;

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_B: &str = "11111111-1111-4111-8111-111111111111";
const CORE_C: &str = "22222222-2222-4222-8222-222222222222";

fn setup() -> (tempfile::TempDir, CoreInstaller, String) {
    let temp = tempfile::tempdir().expect("tempdir");
    let initialized = PlatformService::initialize(temp.path()).expect("platform init");
    let token = initialized.owner_token;
    let installer = CoreInstaller::new(temp.path()).expect("installer");
    (temp, installer, token)
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

#[test]
fn initializes_multiple_independent_cores_and_platform_sees_them() {
    let (temp, installer, token) = setup();
    let a = installer
        .initialize(&request(CORE_A, 1, "Core A"), &token)
        .expect("init A");
    let b = installer
        .initialize(&request(CORE_B, 2, "Core B"), &token)
        .expect("init B");

    assert_ne!(
        a.metadata.database_instance_uuid,
        b.metadata.database_instance_uuid
    );
    assert_ne!(
        a.metadata.database_instance_uuid, a.metadata.core_uuid,
        "database instance UUID must not equal the Core UUID"
    );
    assert!(a.metadata.database_instance_uuid.len() > 30);
    assert!(b.metadata.database_instance_uuid.len() > 30);

    let a_path = installer.layout().database_path(CORE_A).expect("A path");
    let b_path = installer.layout().database_path(CORE_B).expect("B path");
    assert_ne!(a_path, b_path);
    assert!(a_path.is_file());
    assert!(b_path.is_file());
    assert_eq!(
        compute_file_sha256(&a_path).unwrap(),
        a.integrity.actual_sha256.unwrap()
    );
    assert_eq!(
        compute_file_sha256(&b_path).unwrap(),
        b.integrity.actual_sha256.unwrap()
    );

    let entries = installer.list().expect("list");
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().any(|entry| entry.core_uuid == CORE_A));
    assert!(entries.iter().any(|entry| entry.core_uuid == CORE_B));

    let platform = PlatformService::start(temp.path()).expect("platform start");
    let status = platform.status().expect("status");
    assert!(!status.zero_core);
    assert_eq!(status.core_count, 2);

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
fn repeated_initialization_is_rejected_without_changing_files() {
    let (_temp, installer, token) = setup();
    let first = installer
        .initialize(&request(CORE_A, 1, "Core A"), &token)
        .expect("init");
    let path = installer.layout().database_path(CORE_A).expect("path");
    let before = compute_file_sha256(&path).expect("hash before");

    let error = installer
        .initialize(&request(CORE_A, 1, "Core A"), &token)
        .expect_err("duplicate init");
    assert!(matches!(error, InstallError::AlreadyInitialized));
    assert_eq!(compute_file_sha256(&path).expect("hash after"), before);
    assert_eq!(first.identity.core_uuid, CORE_A);
}

#[test]
fn unauthorized_owner_cannot_initialize() {
    let (temp, installer, _token) = setup();
    let error = installer
        .initialize(&request(CORE_A, 1, "Core A"), "wrong-token")
        .expect_err("unauthorized init");
    assert!(matches!(error, InstallError::Unauthorized));
    assert!(!installer.layout().database_path(CORE_A).unwrap().exists());
    let platform = PlatformService::start(temp.path()).expect("platform start");
    assert!(platform.status().expect("status").zero_core);
}

#[test]
fn interrupted_checkpoint_blocks_silent_recovery() {
    let (_temp, installer, token) = setup();
    let marker = installer
        .layout()
        .initialization_marker_path(CORE_A)
        .expect("marker");
    let checkpoint = InitializationCheckpoint {
        schema: CORE_INIT_CHECKPOINT_SCHEMA.to_owned(),
        core_uuid: CORE_A.to_owned(),
        stage: InitializationStage::StoragePrepared,
        updated_at: now_rfc3339().expect("timestamp"),
        failure_code: None,
    };
    write_checkpoint(&marker, &checkpoint).expect("write checkpoint");
    let error = installer
        .initialize(&request(CORE_A, 1, "Core A"), &token)
        .expect_err("interrupted");
    assert!(matches!(error, InstallError::Interrupted));
    assert!(!installer.layout().database_path(CORE_A).unwrap().exists());
}

#[test]
fn tampered_database_fails_integrity_and_isolates_other_cores() {
    let (_temp, installer, token) = setup();
    installer
        .initialize(&request(CORE_A, 1, "Core A"), &token)
        .expect("init A");
    installer
        .initialize(&request(CORE_B, 2, "Core B"), &token)
        .expect("init B");

    let a_path = installer.layout().database_path(CORE_A).expect("A path");
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&a_path)
        .expect("open A");
    use std::io::Write;
    file.write_all(b"tamper").expect("tamper");
    file.sync_all().expect("sync");
    drop(file);

    let error = installer.verify(CORE_A).expect_err("tampered A");
    assert!(matches!(
        error,
        InstallError::VerificationFailed(
            aegis_contracts::integrity::IntegrityStatus::DigestMismatch
        )
    ));
    assert!(installer.verify(CORE_B).is_ok(), "B must remain healthy");
    assert_eq!(installer.list().expect("list").len(), 2);
}

#[test]
fn rename_preserves_uuid_and_reseals_integrity() {
    let (_temp, installer, token) = setup();
    let first = installer
        .initialize(&request(CORE_A, 1, "Core A"), &token)
        .expect("init");
    let renamed = installer
        .rename_display_name(CORE_A, "Core A Renamed", &token)
        .expect("rename");
    assert_eq!(renamed.identity.core_uuid, first.identity.core_uuid);
    assert_eq!(renamed.identity.core_number, first.identity.core_number);
    assert_ne!(renamed.identity.display_name, first.identity.display_name);
    assert_eq!(renamed.registry_entry.display_name, "Core A Renamed");
    assert_eq!(renamed.integrity.status.as_str(), "verified");
    assert_eq!(
        installer.verify(CORE_A).expect("verify").status.as_str(),
        "verified"
    );
}

#[test]
fn deactivation_preserves_database_and_manifest() {
    let (_temp, installer, token) = setup();
    installer
        .initialize(&request(CORE_A, 1, "Core A"), &token)
        .expect("init");
    let db_path = installer.layout().database_path(CORE_A).expect("db path");
    let manifest_path = installer
        .layout()
        .integrity_path(CORE_A)
        .expect("manifest path");
    let db_hash = compute_file_sha256(&db_path).expect("db hash");
    let manifest_hash = compute_file_sha256(&manifest_path).expect("manifest hash");

    let entry = installer
        .deactivate(CORE_A, "owner requested suspend", &token)
        .expect("deactivate");
    assert_eq!(entry.lifecycle_state.as_str(), "suspended");
    assert_eq!(
        compute_file_sha256(&db_path).expect("db hash after"),
        db_hash
    );
    assert_eq!(
        compute_file_sha256(&manifest_path).expect("manifest hash after"),
        manifest_hash
    );
    assert!(installer.verify(CORE_A).is_ok());
}

#[test]
fn prohibited_core_numbers_are_rejected() {
    let (_temp, installer, token) = setup();
    for number in [4u8, 5u8] {
        let error = installer
            .initialize(&request(CORE_C, number, "Forbidden"), &token)
            .expect_err("prohibited");
        assert!(matches!(
            error,
            InstallError::Identity(aegis_core_identity::IdentityError::Core(
                aegis_contracts::core::CoreContractError::ProhibitedCoreNumber
            ))
        ));
    }
}

#[test]
fn read_snapshot_does_not_invalidate_digest() {
    let (_temp, installer, token) = setup();
    installer
        .initialize(&request(CORE_A, 1, "Core A"), &token)
        .expect("init");
    let before = installer.verify(CORE_A).expect("verify before");
    let inspection = installer.inspect(CORE_A).expect("inspect");
    assert_eq!(inspection.integrity.status.as_str(), "verified");
    let after = installer.verify(CORE_A).expect("verify after");
    assert_eq!(before.actual_sha256, after.actual_sha256);
}

#[test]
fn database_paths_are_inside_the_cores_root() {
    let (_temp, installer, token) = setup();
    installer
        .initialize(&request(CORE_A, 1, "Core A"), &token)
        .expect("init");
    let path = installer.layout().database_path(CORE_A).expect("path");
    assert!(path.starts_with(installer.layout().cores_root()));
    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some("core.db")
    );
    let _ = PathBuf::from(&path);
}

#[test]
fn uppercase_uuid_is_canonicalized_and_does_not_leave_partial_state() {
    let (_temp, installer, token) = setup();
    let uppercase = CORE_A.to_uppercase();
    let installation = installer
        .initialize(&request(&uppercase, 1, "Core A"), &token)
        .expect("uppercase UUID must canonicalize");
    assert_eq!(installation.identity.core_uuid, CORE_A);
    let path = installer.layout().database_path(CORE_A).expect("path");
    assert!(path.is_file());
    assert_eq!(
        installer.verify(CORE_A).expect("verify").status.as_str(),
        "verified"
    );
}

#[test]
fn failed_checkpoint_with_existing_database_can_be_recovered() {
    let (_temp, installer, token) = setup();
    let identity =
        aegis_core_identity::validate_initialization_request(&request(CORE_A, 1, "Core A"))
            .expect("identity");
    let database = aegis_core_storage::CoreDatabase::create_fresh(installer.layout(), &identity)
        .expect("create database");
    database.close().expect("close");
    let marker = installer
        .layout()
        .initialization_marker_path(CORE_A)
        .expect("marker");
    let failed = InitializationCheckpoint {
        schema: CORE_INIT_CHECKPOINT_SCHEMA.to_owned(),
        core_uuid: CORE_A.to_owned(),
        stage: InitializationStage::Failed,
        updated_at: now_rfc3339().expect("timestamp"),
        failure_code: Some(aegis_contracts::initialization::InitializationFailureCode::Internal),
    };
    write_checkpoint(&marker, &failed).expect("failed checkpoint");
    let recovered = installer
        .initialize(&request(CORE_A, 1, "Core A"), &token)
        .expect("recover failed initialization");
    assert_eq!(recovered.identity.core_uuid, CORE_A);
    assert_eq!(
        installer.verify(CORE_A).expect("verify").status.as_str(),
        "verified"
    );
}

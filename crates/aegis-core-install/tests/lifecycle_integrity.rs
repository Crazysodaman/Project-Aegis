//! End-to-end Core lifecycle, identity, and integrity tests (Batch 5 / Agent 10).
//!
//! These tests exercise the composed installer across identity, storage,
//! integrity, and registry domains using disposable temporary roots only.

use std::fs;

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_contracts::integrity::IntegrityStatus;
use aegis_contracts::lifecycle::CoreLifecycleState;
use aegis_core_install::{CoreInstallation, CoreInstaller};
use aegis_core_integrity::compute_file_sha256;
use aegis_platform::PlatformService;
use aegis_storage::now_rfc3339;

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_B: &str = "b1b1b1b1-2222-4333-8444-555566667777";

fn setup() -> (tempfile::TempDir, CoreInstaller, String) {
    let temp = tempfile::tempdir().expect("tempdir");
    let initialized = PlatformService::initialize(temp.path()).expect("platform init");
    let installer = CoreInstaller::new(temp.path()).expect("installer");
    (temp, installer, initialized.owner_token)
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

fn install(
    installer: &CoreInstaller,
    token: &str,
    core_uuid: &str,
    core_number: u8,
    display_name: &str,
) -> CoreInstallation {
    installer
        .initialize(&request(core_uuid, core_number, display_name), token)
        .expect("initialize Core")
}

#[test]
fn install_verify_and_inspect_report_the_same_identity() {
    let (_temp, installer, token) = setup();
    let installed = install(&installer, &token, CORE_A, 1, "Sofía Ada Lyra");

    assert_eq!(installed.identity.core_uuid, CORE_A);
    assert_eq!(installed.identity.core_number, 1);
    assert_eq!(
        installed.registry_entry.lifecycle_state,
        CoreLifecycleState::Installed
    );

    let report = installer.verify(CORE_A).expect("verify integrity");
    assert_eq!(report.status, IntegrityStatus::Verified);

    let inspected = installer.inspect(CORE_A).expect("inspect identity");
    assert_eq!(inspected.identity, installed.identity);
    assert_eq!(inspected.metadata, installed.metadata);
    assert_eq!(inspected.integrity.status, IntegrityStatus::Verified);

    let entry = inspected.registry_entry.expect("Core must be registered");
    assert_eq!(entry.core_uuid, installed.registry_entry.core_uuid);
    assert_eq!(entry.core_number, 1);
    assert_eq!(entry.lifecycle_state, CoreLifecycleState::Installed);
}

#[test]
fn rename_preserves_uuid_core_number_and_database_instance() {
    let (_temp, installer, token) = setup();
    let before = install(&installer, &token, CORE_A, 1, "Sofía Ada Lyra");
    let database_path = installer.layout().database_path(CORE_A).expect("db path");

    let renamed = installer
        .rename_display_name(CORE_A, "Sofía Ada Lyra Prime", &token)
        .expect("rename");

    assert_eq!(renamed.identity.core_uuid, before.identity.core_uuid);
    assert_eq!(renamed.identity.core_number, before.identity.core_number);
    assert_eq!(renamed.identity.created_at, before.identity.created_at);
    assert_ne!(renamed.identity.display_name, before.identity.display_name);
    assert_eq!(
        renamed.identity.identity_revision,
        before.identity.identity_revision + 1
    );
    assert_eq!(
        renamed.metadata.database_instance_uuid, before.metadata.database_instance_uuid,
        "rename must not change the database instance identity"
    );
    assert_eq!(renamed.registry_entry.display_name, "Sofía Ada Lyra Prime");
    assert_eq!(renamed.integrity.status, IntegrityStatus::Verified);
    assert_eq!(
        compute_file_sha256(&database_path).expect("digest after rename"),
        renamed
            .integrity
            .actual_sha256
            .clone()
            .expect("verified digest"),
        "the resealed manifest must match the current database bytes"
    );

    let inspected = installer.inspect(CORE_A).expect("inspect after rename");
    assert_eq!(inspected.identity.core_uuid, CORE_A);
    assert_eq!(inspected.identity.core_number, 1);
    assert_eq!(
        inspected.metadata.database_instance_uuid,
        before.metadata.database_instance_uuid
    );
    assert_eq!(
        installer.verify(CORE_A).expect("verify").status,
        IntegrityStatus::Verified
    );
}

#[test]
fn deactivate_then_reactivate_preserves_database_instance_and_data() {
    let (_temp, installer, token) = setup();
    let installed = install(&installer, &token, CORE_A, 1, "Sofía Ada Lyra");
    let database_path = installer.layout().database_path(CORE_A).expect("db path");
    let manifest_path = installer
        .layout()
        .integrity_path(CORE_A)
        .expect("manifest path");
    let core_dir = installer.layout().core_dir(CORE_A).expect("core dir");
    let database_digest = compute_file_sha256(&database_path).expect("db digest");
    let manifest_digest = compute_file_sha256(&manifest_path).expect("manifest digest");

    let retained = core_dir.join("retained-data.bin");
    fs::write(&retained, b"core-private-data").expect("write retained data");

    let suspended = installer
        .deactivate(CORE_A, "scheduled maintenance", &token)
        .expect("deactivate");
    assert_eq!(suspended.lifecycle_state, CoreLifecycleState::Suspended);
    let after_deactivate = installer.inspect(CORE_A).expect("inspect suspended");
    assert_eq!(
        after_deactivate.metadata.database_instance_uuid,
        installed.metadata.database_instance_uuid
    );

    let active = installer
        .reactivate(CORE_A, "maintenance complete", &token)
        .expect("reactivate");
    assert_eq!(active.lifecycle_state, CoreLifecycleState::Active);
    let after_reactivate = installer.inspect(CORE_A).expect("inspect active");
    assert_eq!(
        after_reactivate.metadata.database_instance_uuid,
        installed.metadata.database_instance_uuid
    );
    assert_eq!(after_reactivate.identity, installed.identity);

    assert_eq!(
        compute_file_sha256(&database_path).expect("db digest after"),
        database_digest,
        "lifecycle changes must not mutate the Core database"
    );
    assert_eq!(
        compute_file_sha256(&manifest_path).expect("manifest digest after"),
        manifest_digest,
        "lifecycle changes must not mutate the integrity manifest"
    );
    assert_eq!(
        fs::read(&retained).expect("retained data present"),
        b"core-private-data"
    );
    assert_eq!(
        installer.verify(CORE_A).expect("verify").status,
        IntegrityStatus::Verified
    );
}

#[test]
fn uninstall_preserve_retains_core_directory_and_data() {
    let (_temp, installer, token) = setup();
    let installed = install(&installer, &token, CORE_A, 1, "Sofía Ada Lyra");
    let database_path = installer.layout().database_path(CORE_A).expect("db path");
    let manifest_path = installer
        .layout()
        .integrity_path(CORE_A)
        .expect("manifest path");
    let core_dir = installer.layout().core_dir(CORE_A).expect("core dir");
    let database_digest = compute_file_sha256(&database_path).expect("db digest");
    let manifest_digest = compute_file_sha256(&manifest_path).expect("manifest digest");

    let retained = core_dir.join("retained-data.bin");
    fs::write(&retained, b"core-private-data").expect("write retained data");

    let retired = installer
        .uninstall_preserve(CORE_A, "owner uninstall preserve", &token)
        .expect("uninstall preserve");
    assert_eq!(retired.lifecycle_state, CoreLifecycleState::Retired);

    assert!(core_dir.is_dir(), "Core directory must be retained");
    assert!(database_path.is_file(), "Core database must be retained");
    assert!(
        manifest_path.is_file(),
        "integrity manifest must be retained"
    );
    assert_eq!(
        compute_file_sha256(&database_path).expect("db digest after"),
        database_digest
    );
    assert_eq!(
        compute_file_sha256(&manifest_path).expect("manifest digest after"),
        manifest_digest
    );
    assert_eq!(
        fs::read(&retained).expect("retained data present"),
        b"core-private-data"
    );

    let inspected = installer.inspect(CORE_A).expect("inspect retired");
    assert_eq!(inspected.identity, installed.identity);
    assert_eq!(
        inspected.metadata.database_instance_uuid,
        installed.metadata.database_instance_uuid
    );
    assert_eq!(
        inspected.registry_entry.expect("entry").lifecycle_state,
        CoreLifecycleState::Retired
    );
    assert_eq!(
        installer.verify(CORE_A).expect("verify").status,
        IntegrityStatus::Verified
    );
}

#[test]
fn two_cores_have_distinct_databases_and_instance_uuids() {
    let (_temp, installer, token) = setup();
    let alpha = install(&installer, &token, CORE_A, 1, "Sofía Ada Lyra");
    let beta = install(&installer, &token, CORE_B, 2, "Mack Fuller");

    let alpha_path = installer
        .layout()
        .database_path(CORE_A)
        .expect("alpha path");
    let beta_path = installer.layout().database_path(CORE_B).expect("beta path");

    assert_ne!(alpha_path, beta_path);
    assert_ne!(alpha_path.parent(), beta_path.parent());
    assert!(alpha_path.is_file());
    assert!(beta_path.is_file());

    assert_ne!(
        alpha.metadata.database_instance_uuid, beta.metadata.database_instance_uuid,
        "each Core database must have its own instance UUID"
    );
    assert_ne!(
        alpha.metadata.database_instance_uuid, CORE_A,
        "database instance UUID must not equal the Core UUID"
    );
    assert_ne!(
        beta.metadata.database_instance_uuid, CORE_B,
        "database instance UUID must not equal the Core UUID"
    );
    assert_ne!(alpha.identity.core_uuid, beta.identity.core_uuid);

    assert_eq!(
        installer.verify(CORE_A).expect("verify alpha").status,
        IntegrityStatus::Verified
    );
    assert_eq!(
        installer.verify(CORE_B).expect("verify beta").status,
        IntegrityStatus::Verified
    );
    assert_eq!(installer.list().expect("list").len(), 2);
}

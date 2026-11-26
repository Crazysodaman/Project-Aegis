//! Backup ownership/contamination and secret-leak tests (Final / Agent 16).
//!
//! Complements `backup_restore.rs`, `batch8_inventory.rs`, and `batch8_qa.rs`.
//! These tests attack a finished backup: a cross-Core sidecar with a forged
//! manifest, swapped Platform/Safe databases, and a tampered owner proof.

use std::fs;
use std::path::{Path, PathBuf};

use aegis_backup::{
    backup, restore, verify_backup, BackupError, BackupVerificationStatus, RestoreOptions,
    VerificationIssueKind,
};
use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_core_install::CoreInstaller;
use aegis_core_integrity::compute_file_sha256;
use aegis_platform::PlatformService;
use aegis_storage::now_rfc3339;
use rusqlite::{params, Connection};
use serde_json::Value;

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_B: &str = "b9a0a604-a4e7-4a88-a985-3f071e2c8e7d";

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
    CoreInstaller::new(root)
        .expect("installer")
        .initialize(&request(core_uuid, core_number, name), token)
        .expect("initialize Core");
}

fn core_dir(root: &Path, core_uuid: &str) -> PathBuf {
    root.join("cores").join(core_uuid)
}

fn write_memory_sidecar(root: &Path, core_uuid: &str, marker: &str) {
    let directory = core_dir(root, core_uuid);
    fs::create_dir_all(&directory).expect("core dir");
    let connection = Connection::open(directory.join("memory.db")).expect("open memory db");
    connection
        .execute_batch(
            "CREATE TABLE core_memory_ownership (
                singleton INTEGER NOT NULL PRIMARY KEY CHECK(singleton = 1),
                core_uuid TEXT NOT NULL,
                schema TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE TABLE memory_entries (
                id TEXT NOT NULL PRIMARY KEY,
                core_uuid TEXT NOT NULL,
                content TEXT NOT NULL
            );
            PRAGMA journal_mode = DELETE;",
        )
        .expect("memory schema");
    connection
        .execute(
            "INSERT INTO core_memory_ownership(singleton, core_uuid, schema, created_at)
             VALUES(1, ?1, 'aegis.core.memory.v1', '2026-01-01T00:00:00Z')",
            [core_uuid],
        )
        .expect("memory ownership");
    connection
        .execute(
            "INSERT INTO memory_entries(id, core_uuid, content) VALUES(?1, ?2, ?3)",
            params![format!("memory-{core_uuid}"), core_uuid, marker],
        )
        .expect("memory entry");
}

fn fresh_path(parent: &Path, name: &str) -> PathBuf {
    parent.join(name)
}

fn rewrite_file_entry(manifest: &mut Value, relative: &str, sha256: &str, bytes: u64) {
    let files = manifest["files"].as_array_mut().expect("files array");
    let entry = files
        .iter_mut()
        .find(|entry| entry["relative_path"].as_str() == Some(relative))
        .expect("file entry");
    entry["sha256"] = Value::String(sha256.to_owned());
    entry["bytes"] = Value::from(bytes);
}

fn rewrite_core_database_entry(
    manifest: &mut Value,
    core_uuid: &str,
    file_name: &str,
    sha256: &str,
    bytes: u64,
) {
    let cores = manifest["cores"].as_array_mut().expect("cores array");
    let core = cores
        .iter_mut()
        .find(|core| core["core_uuid"].as_str() == Some(core_uuid))
        .expect("core entry");
    let databases = core["databases"].as_array_mut().expect("databases array");
    let database = databases
        .iter_mut()
        .find(|database| database["file_name"].as_str() == Some(file_name))
        .expect("database entry");
    database["sha256"] = Value::String(sha256.to_owned());
    database["bytes"] = Value::from(bytes);
}

fn digest_and_len(path: &Path) -> (String, u64) {
    let sha256 = compute_file_sha256(path).expect("digest");
    let bytes = fs::metadata(path).expect("metadata").len();
    (sha256, bytes)
}

#[test]
fn cross_core_sidecar_swap_with_forged_manifest_is_caught_by_ownership() {
    let source = tempfile::tempdir().expect("source");
    let initialized = PlatformService::initialize(source.path()).expect("platform init");
    install(source.path(), &initialized.owner_token, CORE_A, 1, "Core A");
    install(source.path(), &initialized.owner_token, CORE_B, 2, "Core B");
    write_memory_sidecar(source.path(), CORE_A, "A-memory");
    write_memory_sidecar(source.path(), CORE_B, "B-memory");

    let backup_parent = tempfile::tempdir().expect("backup parent");
    let destination = fresh_path(backup_parent.path(), "snapshot");
    backup(source.path(), &destination).expect("backup");
    assert_eq!(
        verify_backup(&destination).expect("verify").status,
        BackupVerificationStatus::Verified
    );

    // Replace A's memory database with B's, then rewrite the manifest digests
    // so a pure hash check can no longer notice the substitution.
    let a_memory = destination.join(format!("cores/{CORE_A}/memory.db"));
    let b_memory = destination.join(format!("cores/{CORE_B}/memory.db"));
    fs::copy(&b_memory, &a_memory).expect("swap memory database");
    let (sha256, bytes) = digest_and_len(&a_memory);

    let manifest_path = destination.join(aegis_backup::BACKUP_MANIFEST_FILE);
    let mut manifest: Value = serde_json::from_slice(&fs::read(&manifest_path).expect("manifest"))
        .expect("parse manifest");
    rewrite_file_entry(
        &mut manifest,
        &format!("cores/{CORE_A}/memory.db"),
        &sha256,
        bytes,
    );
    rewrite_core_database_entry(&mut manifest, CORE_A, "memory.db", &sha256, bytes);
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).expect("serialize manifest"),
    )
    .expect("write manifest");

    let a_memory_relative = format!("cores/{CORE_A}/memory.db");
    let verification = verify_backup(&destination).expect("verify forged backup");
    assert_eq!(verification.status, BackupVerificationStatus::Invalid);
    assert!(
        verification.issues.iter().any(|issue| {
            issue.kind == VerificationIssueKind::Integrity
                && issue.relative_path.as_deref() == Some(a_memory_relative.as_str())
                && issue.detail.contains("another Core")
        }),
        "ownership check must reject the cross-Core swap: {:?}",
        verification.issues
    );

    let target_parent = tempfile::tempdir().expect("target parent");
    let target = fresh_path(target_parent.path(), "restored");
    let error = restore(&destination, &target, RestoreOptions::default())
        .expect_err("contaminated backup must not restore");
    assert!(matches!(error, BackupError::InvalidBackup(_)));
    assert!(!target.exists(), "no partial target may be created");
}

#[test]
fn swapped_platform_and_safe_databases_escape_verify_but_restore_fails_closed() {
    let source = tempfile::tempdir().expect("source");
    let _initialized = PlatformService::initialize(source.path()).expect("platform init");

    let backup_parent = tempfile::tempdir().expect("backup parent");
    let destination = fresh_path(backup_parent.path(), "snapshot");
    backup(source.path(), &destination).expect("backup");
    assert_eq!(
        verify_backup(&destination).expect("verify").status,
        BackupVerificationStatus::Verified
    );

    let platform_path = destination.join("platform.db");
    let safe_path = destination.join("safe.db");
    let platform = fs::read(&platform_path).expect("platform bytes");
    let safe = fs::read(&safe_path).expect("safe bytes");
    fs::write(&platform_path, &safe).expect("swap platform");
    fs::write(&safe_path, &platform).expect("swap safe");
    let (platform_sha, platform_bytes) = digest_and_len(&platform_path);
    let (safe_sha, safe_bytes) = digest_and_len(&safe_path);

    let manifest_path = destination.join(aegis_backup::BACKUP_MANIFEST_FILE);
    let mut manifest: Value = serde_json::from_slice(&fs::read(&manifest_path).expect("manifest"))
        .expect("parse manifest");
    rewrite_file_entry(&mut manifest, "platform.db", &platform_sha, platform_bytes);
    rewrite_file_entry(&mut manifest, "safe.db", &safe_sha, safe_bytes);
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).expect("serialize manifest"),
    )
    .expect("write manifest");

    // KNOWN GAP (Agent 16): verify_backup hashes platform.db/safe.db but does not
    // check their SQLite application markers, so a self-consistent swap still
    // reports Verified. Restore re-checks the markers and fails closed.
    let verification = verify_backup(&destination).expect("verify swapped backup");
    assert_eq!(
        verification.status,
        BackupVerificationStatus::Verified,
        "documented gap: verification alone does not police Platform/Safe markers"
    );

    let target_parent = tempfile::tempdir().expect("target parent");
    let target = fresh_path(target_parent.path(), "restored");
    let error = restore(&destination, &target, RestoreOptions::default())
        .expect_err("swapped markers must block restore");
    assert!(matches!(
        error,
        BackupError::Storage(aegis_storage::StorageError::WrongApplicationId { .. })
    ));
    assert!(!target.exists(), "failed restore must leave no target");
}

#[test]
fn tampered_owner_proof_is_reported_without_echoing_its_bytes() {
    let source = tempfile::tempdir().expect("source");
    let _initialized = PlatformService::initialize(source.path()).expect("platform init");

    let backup_parent = tempfile::tempdir().expect("backup parent");
    let destination = fresh_path(backup_parent.path(), "snapshot");
    backup(source.path(), &destination).expect("backup");

    let sentinel = b"OWNER-PROOF-SECRET-SENTINEL-0123456789";
    let proof_path = destination.join("owner.proof");
    fs::write(&proof_path, sentinel).expect("tamper owner proof");

    let verification = verify_backup(&destination).expect("verify tampered proof");
    assert_eq!(verification.status, BackupVerificationStatus::Invalid);
    let proof_issue = verification
        .issues
        .iter()
        .find(|issue| {
            issue.relative_path.as_deref() == Some("owner.proof")
                && issue.kind == VerificationIssueKind::Tampered
        })
        .expect("owner proof tamper issue");
    let sentinel_text = String::from_utf8_lossy(sentinel);
    assert!(
        !proof_issue.detail.contains(sentinel_text.as_ref()),
        "proof bytes leaked into verification detail: {}",
        proof_issue.detail
    );

    let target_parent = tempfile::tempdir().expect("target parent");
    let target = fresh_path(target_parent.path(), "restored");
    let error = restore(&destination, &target, RestoreOptions::default())
        .expect_err("tampered proof must block restore");
    let text = error.to_string();
    assert!(
        !text.contains(sentinel_text.as_ref()),
        "proof bytes leaked: {text}"
    );
    assert!(
        !text.contains("owner.proof"),
        "proof path leaked through restore error: {text}"
    );
    assert!(!target.exists());
}

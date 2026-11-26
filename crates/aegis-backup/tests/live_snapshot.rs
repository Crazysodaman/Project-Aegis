//! Final / Agent 17: live-write consistency and archive-refusal tests.
//!
//! Proves that databases holding committed WAL content are snapshotted through
//! SQLite (`VACUUM INTO`) while they are still open, that no `-wal` sidecar is
//! copied, and that the v2 manifest, ownership, digest, and target-refusal
//! guarantees still hold.

use std::fs;
use std::path::{Path, PathBuf};

use aegis_backup::{
    backup, read_manifest, restore, verify_backup, BackupError, BackupVerificationStatus,
    RestoreOptions, VerificationIssueKind,
};
use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_core_install::CoreInstaller;
use aegis_core_integrity::seal_manifest_with_metadata;
use aegis_core_storage::{CoreDatabase, CoreStorageLayout};
use aegis_platform::PlatformService;
use aegis_storage::now_rfc3339;
use rusqlite::{params, Connection};
use serde_json::Value;

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_B: &str = "b9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CREATED: &str = "2026-01-01T00:00:00Z";

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

fn init_platform() -> (tempfile::TempDir, String) {
    let temp = tempfile::tempdir().expect("tempdir");
    let initialized = PlatformService::initialize(temp.path()).expect("platform init");
    (temp, initialized.owner_token)
}

fn setup_two_cores() -> (tempfile::TempDir, String) {
    let (temp, token) = init_platform();
    install(temp.path(), &token, CORE_A, 1, "Core A");
    install(temp.path(), &token, CORE_B, 2, "Core B");
    (temp, token)
}

fn write_memory_sidecar(root: &Path, core_uuid: &str, marker: &str) {
    let dir = core_dir(root, core_uuid);
    fs::create_dir_all(&dir).expect("core dir");
    let connection = Connection::open(dir.join("memory.db")).expect("open memory db");
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
             VALUES(1, ?1, 'aegis.core.memory.v1', ?2)",
            params![core_uuid, CREATED],
        )
        .expect("memory ownership");
    connection
        .execute(
            "INSERT INTO memory_entries(id, core_uuid, content) VALUES(?1, ?2, ?3)",
            params![format!("seed-{core_uuid}"), core_uuid, marker],
        )
        .expect("memory entry");
}

fn memory_contents(path: &Path) -> Vec<String> {
    let connection = Connection::open(path).expect("open memory db");
    let mut statement = connection
        .prepare("SELECT content FROM memory_entries ORDER BY id ASC")
        .expect("prepare memory query");
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .expect("memory rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect memory rows")
}

fn contains_sidecar(root: &Path) -> bool {
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(&directory).expect("read backup directory") {
            let entry = entry.expect("backup entry");
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if name.ends_with("-wal") || name.ends_with("-shm") || name.ends_with("-journal") {
                return true;
            }
        }
    }
    false
}

#[test]
fn committed_wal_sidecar_is_snapshotted_while_open_and_restores() {
    let (source, _token) = init_platform();
    install(source.path(), &_token, CORE_A, 1, "Core A");
    write_memory_sidecar(source.path(), CORE_A, "seed-before-wal");

    let memory_path = core_dir(source.path(), CORE_A).join("memory.db");
    let writer = Connection::open(&memory_path).expect("open writer");
    let mode: String = writer
        .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
        .expect("enable WAL");
    assert_eq!(mode.to_ascii_lowercase(), "wal");
    writer
        .execute(
            "INSERT INTO memory_entries(id, core_uuid, content) VALUES(?1, ?2, ?3)",
            params![format!("wal-{CORE_A}"), CORE_A, "committed-in-wal"],
        )
        .expect("insert committed WAL row");
    let wal_path = memory_path.with_extension("db-wal");
    assert!(wal_path.exists(), "writer must leave a WAL sidecar");

    // A quote in the destination exercises safe SQL path quoting for the
    // internal VACUUM INTO target.
    let backup_parent = tempfile::tempdir().expect("backup parent");
    let destination = backup_parent.path().join("snap-'quoted'");
    let report = backup(source.path(), &destination).expect("backup while writer open");
    assert!(
        report
            .manifest
            .files
            .iter()
            .any(|file| file.relative_path == format!("cores/{CORE_A}/memory.db")),
        "the sidecar database must be backed up"
    );
    assert!(
        !report
            .manifest
            .files
            .iter()
            .any(|file| file.relative_path.contains("-wal")),
        "no WAL sidecar may be recorded in the manifest"
    );
    assert!(
        !contains_sidecar(&destination),
        "no WAL/SHM/journal sidecar may be copied into the backup"
    );

    let snapshot = destination.join(format!("cores/{CORE_A}/memory.db"));
    assert_eq!(
        memory_contents(&snapshot),
        vec!["seed-before-wal".to_owned(), "committed-in-wal".to_owned()],
        "the SQLite snapshot must contain both sealed and committed WAL rows"
    );
    assert_eq!(
        verify_backup(&destination).expect("verify").status,
        BackupVerificationStatus::Verified
    );

    drop(writer);

    let target_parent = tempfile::tempdir().expect("target parent");
    let target = target_parent.path().join("restored");
    restore(&destination, &target, RestoreOptions::default()).expect("restore");
    assert_eq!(
        memory_contents(&core_dir(&target, CORE_A).join("memory.db")),
        vec!["seed-before-wal".to_owned(), "committed-in-wal".to_owned()],
        "restored database must verify the committed WAL row"
    );
    let ownership: String = Connection::open(core_dir(&target, CORE_A).join("memory.db"))
        .expect("open restored memory")
        .query_row("SELECT core_uuid FROM core_memory_ownership", [], |row| {
            row.get(0)
        })
        .expect("ownership");
    assert_eq!(ownership, CORE_A);
}

#[test]
fn committed_wal_core_database_is_resealed_and_restores() {
    let (source, _token) = init_platform();
    install(source.path(), &_token, CORE_A, 1, "Core A");

    let core_path = core_dir(source.path(), CORE_A).join("core.db");
    {
        let connection = Connection::open(&core_path).expect("open core db");
        let mode: String = connection
            .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
            .expect("enable WAL");
        assert_eq!(mode.to_ascii_lowercase(), "wal");
        connection
            .execute_batch(
                "CREATE TABLE live_probe(value TEXT NOT NULL);
                 INSERT INTO live_probe(value) VALUES('sealed');",
            )
            .expect("create probe");
    }

    // Re-seal the source manifest for the schema change made above, then commit
    // a further row to the WAL and keep the writer open.
    let layout = CoreStorageLayout::new(source.path()).expect("source layout");
    let snapshot = CoreDatabase::read_snapshot(&layout, CORE_A).expect("source snapshot");
    let source_manifest = layout.integrity_path(CORE_A).expect("integrity path");
    seal_manifest_with_metadata(
        CORE_A,
        &snapshot.database_path,
        &source_manifest,
        snapshot.metadata.schema_revision,
        &snapshot.identity,
        Some(&snapshot.metadata.database_instance_uuid),
        Some(&snapshot.metadata.initialization_mode),
        Some(snapshot.metadata.core_number),
    )
    .expect("reseal source manifest");

    let writer = Connection::open(&core_path).expect("open live writer");
    writer
        .execute(
            "INSERT INTO live_probe(value) VALUES('committed-in-wal')",
            [],
        )
        .expect("insert committed WAL row");
    assert!(
        core_path.with_extension("db-wal").exists(),
        "writer must leave a WAL sidecar"
    );

    let backup_parent = tempfile::tempdir().expect("backup parent");
    let destination = backup_parent.path().join("snapshot");
    let report = backup(source.path(), &destination).expect("backup live core database");
    assert!(report
        .manifest
        .cores
        .iter()
        .any(|core| core.core_uuid == CORE_A && core.integrity_status == "verified"));
    assert!(!contains_sidecar(&destination));
    assert_eq!(
        verify_backup(&destination).expect("verify").status,
        BackupVerificationStatus::Verified
    );

    drop(writer);

    let target_parent = tempfile::tempdir().expect("target parent");
    let target = target_parent.path().join("restored");
    restore(&destination, &target, RestoreOptions::default()).expect("restore");

    let restored = core_dir(&target, CORE_A).join("core.db");
    let values: Vec<String> = {
        let connection = Connection::open(&restored).expect("open restored core");
        let mut statement = connection
            .prepare("SELECT value FROM live_probe ORDER BY rowid ASC")
            .expect("prepare probe query");
        statement
            .query_map([], |row| row.get::<_, String>(0))
            .expect("probe rows")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect probe rows")
    };
    assert_eq!(
        values,
        vec!["sealed".to_owned(), "committed-in-wal".to_owned()],
        "restored core must include committed WAL content"
    );
    assert_eq!(
        CoreInstaller::new(&target)
            .expect("restored installer")
            .verify(CORE_A)
            .expect("verify restored core")
            .status
            .as_str(),
        "verified",
        "the re-sealed manifest must validate the snapshot"
    );
}

#[test]
fn corrupted_archive_is_refused_without_writing_target() {
    let (source, _token) = setup_two_cores();
    let backup_parent = tempfile::tempdir().expect("backup parent");
    let destination = backup_parent.path().join("snapshot");
    backup(source.path(), &destination).expect("backup");

    let corrupted = destination.join(format!("cores/{CORE_A}/core.db"));
    fs::write(&corrupted, b"not a sqlite database").expect("corrupt archive");

    let verification = verify_backup(&destination).expect("verify corrupted archive");
    assert_eq!(verification.status, BackupVerificationStatus::Invalid);
    assert!(verification.issues.iter().any(|issue| matches!(
        issue.kind,
        VerificationIssueKind::Tampered | VerificationIssueKind::SizeMismatch
    )));

    let target_parent = tempfile::tempdir().expect("target parent");
    let target = target_parent.path().join("restored");
    let error = restore(&destination, &target, RestoreOptions::default())
        .expect_err("corrupted archive must be refused");
    assert!(matches!(error, BackupError::InvalidBackup(_)));
    assert!(!target.exists());
}

#[test]
fn incompatible_manifest_version_is_refused() {
    let (source, _token) = init_platform();
    let backup_parent = tempfile::tempdir().expect("backup parent");
    let destination = backup_parent.path().join("snapshot");
    backup(source.path(), &destination).expect("backup");

    let manifest_path = destination.join(aegis_backup::BACKUP_MANIFEST_FILE);
    let mut value: Value = serde_json::from_slice(&fs::read(&manifest_path).expect("manifest"))
        .expect("parse manifest");
    value["format_version"] = Value::from(aegis_backup::BACKUP_FORMAT_VERSION + 1);
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&value).expect("serialize manifest"),
    )
    .expect("write incompatible manifest");

    assert!(matches!(
        read_manifest(&destination),
        Err(BackupError::InvalidBackup(_))
    ));
    assert!(matches!(
        verify_backup(&destination),
        Err(BackupError::InvalidBackup(_))
    ));

    let target_parent = tempfile::tempdir().expect("target parent");
    let target = target_parent.path().join("restored");
    assert!(matches!(
        restore(&destination, &target, RestoreOptions::default()),
        Err(BackupError::InvalidBackup(_))
    ));
    assert!(!target.exists());
}

#[test]
fn missing_optional_databases_are_not_fatal() {
    let (source, _token) = setup_two_cores();
    let backup_parent = tempfile::tempdir().expect("backup parent");
    let destination = backup_parent.path().join("snapshot");
    let report = backup(source.path(), &destination).expect("backup without sidecars");

    assert!(report
        .missing_databases
        .contains(&format!("cores/{CORE_A}/memory.db")));
    for core in &report.manifest.cores {
        assert_eq!(core.databases.len(), 1);
        assert!(!core.missing_optional_databases.is_empty());
    }
    assert_eq!(
        verify_backup(&destination).expect("verify").status,
        BackupVerificationStatus::Verified
    );

    let target_parent = tempfile::tempdir().expect("target parent");
    let target = target_parent.path().join("restored");
    restore(&destination, &target, RestoreOptions::default()).expect("restore without sidecars");
    assert!(core_dir(&target, CORE_A).join("core.db").is_file());
    assert!(!core_dir(&target, CORE_A).join("memory.db").exists());
}

#[test]
fn non_empty_target_is_refused_and_preserved() {
    let (source, _token) = setup_two_cores();
    let backup_parent = tempfile::tempdir().expect("backup parent");
    let destination = backup_parent.path().join("snapshot");
    backup(source.path(), &destination).expect("backup");

    let target_parent = tempfile::tempdir().expect("target parent");
    let target = target_parent.path().join("restored");
    fs::create_dir(&target).expect("target dir");
    let sentinel = target.join("keep-me.txt");
    fs::write(&sentinel, b"do not delete").expect("sentinel");

    let error = restore(&destination, &target, RestoreOptions::default())
        .expect_err("non-empty target must be refused");
    assert!(matches!(error, BackupError::RestoreTargetNotEmpty(_)));
    assert_eq!(fs::read(&sentinel).expect("sentinel"), b"do not delete");
    assert!(!target.join("platform.db").exists());
}

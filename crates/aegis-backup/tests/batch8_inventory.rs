use std::fs;
use std::path::{Path, PathBuf};

use aegis_backup::{
    backup, read_manifest, restore, verify_backup, BackupError, BackupFileKind,
    BackupVerificationStatus, RestoreOptions, BACKUP_FORMAT_VERSION, BACKUP_FORMAT_VERSION_V1,
    BACKUP_MANIFEST_SCHEMA, BACKUP_MANIFEST_SCHEMA_V1, CORE_DATABASE_FILE,
};
use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_core_install::CoreInstaller;
use aegis_platform::PlatformService;
use aegis_storage::now_rfc3339;
use serde_json::Value;

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_B: &str = "b9a0a604-a4e7-4a88-a985-3f071e2c8e7d";

const STANDARD_SIDECARS: &[(&str, &str)] = &[
    ("memory.db", "core_memory_ownership"),
    ("conversation.db", "conversation_ownership"),
    ("assets.db", "asset_ownership"),
    ("social.db", "relationship_ownership"),
    ("embeddings.db", "embedding_ownership"),
];

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

fn core_dir(root: &Path, core_uuid: &str) -> PathBuf {
    root.join("cores").join(core_uuid)
}

fn database_instance_uuid(root: &Path, core_uuid: &str) -> String {
    let database = core_dir(root, core_uuid).join(CORE_DATABASE_FILE);
    let connection = rusqlite::Connection::open(database).expect("open core db");
    connection
        .query_row(
            "SELECT database_instance_uuid FROM core_identity WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .expect("instance uuid")
}

fn write_sidecar(
    root: &Path,
    core_uuid: &str,
    file_name: &str,
    ownership_table: &str,
    marker: &str,
) {
    let path = core_dir(root, core_uuid).join(file_name);
    let connection = rusqlite::Connection::open(&path).expect("open sidecar");
    if file_name == "memory.db" {
        connection
            .execute_batch(&format!(
                "CREATE TABLE IF NOT EXISTS {ownership_table} (
                    singleton INTEGER NOT NULL PRIMARY KEY CHECK (singleton = 1),
                    core_uuid TEXT NOT NULL,
                    schema TEXT NOT NULL,
                    created_at TEXT NOT NULL
                );"
            ))
            .expect("memory ownership table");
        connection
            .execute(
                &format!(
                    "INSERT INTO {ownership_table}(singleton, core_uuid, schema, created_at)
                     VALUES(1, ?1, 'aegis.memory.v1', '2026-01-01T00:00:00Z')"
                ),
                [core_uuid],
            )
            .expect("memory ownership row");
    } else {
        let instance = database_instance_uuid(root, core_uuid);
        connection
            .execute_batch(&format!(
                "CREATE TABLE IF NOT EXISTS {ownership_table} (
                    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                    core_uuid TEXT NOT NULL UNIQUE,
                    database_instance_uuid TEXT NOT NULL,
                    created_at TEXT NOT NULL
                );"
            ))
            .expect("sidecar ownership table");
        connection
            .execute(
                &format!(
                    "INSERT INTO {ownership_table}(singleton, core_uuid, database_instance_uuid, created_at)
                     VALUES(1, ?1, ?2, '2026-01-01T00:00:00Z')"
                ),
                rusqlite::params![core_uuid, instance],
            )
            .expect("sidecar ownership row");
    }
    connection
        .execute_batch("CREATE TABLE IF NOT EXISTS batch8_marker(value TEXT NOT NULL);")
        .expect("marker table");
    connection
        .execute("INSERT INTO batch8_marker(value) VALUES(?1)", [marker])
        .expect("marker row");
    connection
        .execute_batch("PRAGMA journal_mode = DELETE;")
        .expect("journal mode");
}

fn write_asset(root: &Path, core_uuid: &str, asset_id: &str, name: &str, bytes: &[u8]) -> PathBuf {
    let path = core_dir(root, core_uuid)
        .join("assets")
        .join(asset_id)
        .join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("asset dir");
    }
    fs::write(&path, bytes).expect("asset bytes");
    path
}

fn fabricate_full_root() -> tempfile::TempDir {
    let (temp, token) = init_platform();
    install(temp.path(), &token, CORE_A, 1, "Core A");
    install(temp.path(), &token, CORE_B, 2, "Core B");

    for (file_name, table) in STANDARD_SIDECARS {
        write_sidecar(
            temp.path(),
            CORE_A,
            file_name,
            table,
            &format!("A-{file_name}"),
        );
        write_sidecar(
            temp.path(),
            CORE_B,
            file_name,
            table,
            &format!("B-{file_name}"),
        );
    }
    // personal.db is optional; only Core A has it.
    write_sidecar(
        temp.path(),
        CORE_A,
        "personal.db",
        "personal_ownership",
        "A-personal.db",
    );

    write_asset(
        temp.path(),
        CORE_A,
        "11111111-1111-4111-8111-111111111111",
        "avatar.bin",
        b"core-a-avatar-bytes",
    );
    write_asset(
        temp.path(),
        CORE_A,
        "11111111-1111-4111-8111-111111111111",
        "nested/voice.wav",
        b"core-a-nested-voice",
    );
    write_asset(
        temp.path(),
        CORE_B,
        "22222222-2222-4222-8222-222222222222",
        "theme.bin",
        b"core-b-theme-bytes",
    );
    // An empty asset directory must not break the walk.
    fs::create_dir_all(core_dir(temp.path(), CORE_B).join("assets/empty")).expect("empty dir");

    temp
}

fn marker_value(database_path: &Path) -> String {
    let connection = rusqlite::Connection::open(database_path).expect("open marker db");
    connection
        .query_row("SELECT value FROM batch8_marker LIMIT 1", [], |row| {
            row.get(0)
        })
        .expect("marker value")
}

fn ownership_core_uuid(database_path: &Path, table: &str) -> String {
    let connection = rusqlite::Connection::open(database_path).expect("open ownership db");
    connection
        .query_row(
            &format!("SELECT core_uuid FROM {table} LIMIT 1"),
            [],
            |row| row.get(0),
        )
        .expect("ownership core uuid")
}

#[test]
fn full_inventory_backup_and_restore_is_isolated() {
    let source = fabricate_full_root();
    let (_backup_parent, destination) = backup_path();

    let report = backup(source.path(), &destination).expect("backup");
    assert_eq!(report.manifest.format_version, BACKUP_FORMAT_VERSION);
    assert_eq!(report.manifest.schema, BACKUP_MANIFEST_SCHEMA);
    assert_eq!(report.manifest.cores.len(), 2);

    let core_a = report
        .manifest
        .cores
        .iter()
        .find(|core| core.core_uuid == CORE_A)
        .expect("Core A metadata");
    let core_b = report
        .manifest
        .cores
        .iter()
        .find(|core| core.core_uuid == CORE_B)
        .expect("Core B metadata");

    // Core A has core.db plus six sidecars; Core B lacks personal.db.
    assert_eq!(core_a.databases.len(), 7);
    assert_eq!(core_b.databases.len(), 6);
    assert!(core_a
        .databases
        .iter()
        .any(|db| db.file_name == "personal.db"));
    assert!(!core_b
        .databases
        .iter()
        .any(|db| db.file_name == "personal.db"));
    assert!(core_b
        .missing_optional_databases
        .contains(&"personal.db".to_owned()));
    assert!(report
        .missing_databases
        .contains(&format!("cores/{CORE_B}/personal.db")));
    assert!(!report
        .missing_databases
        .iter()
        .any(|path| path.starts_with(&format!("cores/{CORE_A}/"))));

    // Every database is recorded with a kind, digest, and byte count.
    for database in core_a.databases.iter().chain(core_b.databases.iter()) {
        assert_eq!(database.sha256.len(), 64);
        assert!(database.bytes > 0);
        assert!(!database.ownership_table.is_empty());
        let file = report
            .manifest
            .file(&database.relative_path)
            .expect("database file entry");
        assert_eq!(file.sha256, database.sha256);
        assert_eq!(file.bytes, database.bytes);
    }

    let asset_files: Vec<_> = report
        .manifest
        .files
        .iter()
        .filter(|file| file.kind == BackupFileKind::CoreAssetFile)
        .collect();
    assert_eq!(asset_files.len(), 3);
    assert!(report.manifest.files.iter().any(|file| file.relative_path
        == format!("cores/{CORE_A}/assets/11111111-1111-4111-8111-111111111111/avatar.bin")));

    let verification = verify_backup(&destination).expect("verify");
    assert_eq!(verification.status, BackupVerificationStatus::Verified);
    assert!(verification.issues.is_empty());

    let (_target_parent, target) = target_path();
    let restored = restore(&destination, &target, RestoreOptions::default()).expect("restore");
    assert_eq!(restored.restored_cores.len(), 2);
    assert_eq!(restored.file_count, report.file_count);
    assert_eq!(restored.total_bytes, report.total_bytes);

    // Identity is preserved for both Cores.
    for core in [CORE_A, CORE_B] {
        let installer = CoreInstaller::new(&target).expect("restored installer");
        assert_eq!(
            installer.verify(core).expect("verify core").status.as_str(),
            "verified"
        );
    }

    // Ownership rows point at the owning Core after restore.
    for (file_name, table) in STANDARD_SIDECARS {
        let path = core_dir(&target, CORE_A).join(file_name);
        assert_eq!(ownership_core_uuid(&path, table), CORE_A);
        let path = core_dir(&target, CORE_B).join(file_name);
        assert_eq!(ownership_core_uuid(&path, table), CORE_B);
    }
    assert_eq!(
        ownership_core_uuid(
            &core_dir(&target, CORE_A).join("personal.db"),
            "personal_ownership"
        ),
        CORE_A
    );
    assert!(!core_dir(&target, CORE_B).join("personal.db").exists());

    // Marker rows prove no Core inherited another Core's content.
    assert_eq!(
        marker_value(&core_dir(&target, CORE_A).join("memory.db")),
        "A-memory.db"
    );
    assert_eq!(
        marker_value(&core_dir(&target, CORE_B).join("memory.db")),
        "B-memory.db"
    );
    assert_eq!(
        marker_value(&core_dir(&target, CORE_A).join("conversation.db")),
        "A-conversation.db"
    );
    assert_eq!(
        marker_value(&core_dir(&target, CORE_B).join("conversation.db")),
        "B-conversation.db"
    );

    // Asset bytes and relative paths survive exactly.
    for relative in [
        format!("cores/{CORE_A}/assets/11111111-1111-4111-8111-111111111111/avatar.bin"),
        format!("cores/{CORE_A}/assets/11111111-1111-4111-8111-111111111111/nested/voice.wav"),
        format!("cores/{CORE_B}/assets/22222222-2222-4222-8222-222222222222/theme.bin"),
    ] {
        let source_bytes = fs::read(source.path().join(&relative)).expect("source asset");
        let restored_bytes = fs::read(target.join(&relative)).expect("restored asset");
        assert_eq!(source_bytes, restored_bytes);
    }
}

#[test]
fn cross_core_database_ownership_is_rejected() {
    let source = fabricate_full_root();
    // Replace Core A's memory database with Core B's: same schema, wrong owner.
    let a_db = core_dir(source.path(), CORE_A).join("memory.db");
    let b_db = core_dir(source.path(), CORE_B).join("memory.db");
    fs::copy(&b_db, &a_db).expect("swap memory database");

    let (_backup_parent, destination) = backup_path();
    let error = backup(source.path(), &destination).expect_err("ownership mismatch");
    assert!(matches!(error, BackupError::IntegrityFailed { .. }));
    assert!(!destination.join("backup.manifest.json").exists());
}

#[test]
fn missing_optional_databases_are_reported_not_fatal() {
    let (source, _token) = setup_two_cores();
    let (_backup_parent, destination) = backup_path();
    let report = backup(source.path(), &destination).expect("backup");

    assert!(!report.missing_databases.is_empty());
    assert!(report
        .missing_databases
        .contains(&format!("cores/{CORE_A}/memory.db")));
    for core in &report.manifest.cores {
        assert!(core
            .databases
            .iter()
            .any(|db| db.file_name == CORE_DATABASE_FILE));
        assert_eq!(core.databases.len(), 1);
        assert_eq!(core.missing_optional_databases.len(), 6);
    }

    let (_target_parent, target) = target_path();
    restore(&destination, &target, RestoreOptions::default()).expect("restore without sidecars");
    assert!(target.join("cores").join(CORE_A).join("core.db").is_file());
}

#[test]
fn legacy_v1_manifest_without_database_list_is_readable_and_restorable() {
    let (source, _token) = setup_two_cores();
    let (_backup_parent, destination) = backup_path();
    backup(source.path(), &destination).expect("backup");

    let manifest_path = destination.join("backup.manifest.json");
    let bytes = fs::read(&manifest_path).expect("read manifest");
    let mut value: Value = serde_json::from_slice(&bytes).expect("parse manifest");
    value["schema"] = Value::String(BACKUP_MANIFEST_SCHEMA_V1.to_owned());
    value["format_version"] = Value::from(BACKUP_FORMAT_VERSION_V1);
    for core in value["cores"].as_array_mut().expect("cores array") {
        let object = core.as_object_mut().expect("core object");
        object.remove("databases");
        object.remove("missing_optional_databases");
    }
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&value).expect("serialize v1 manifest"),
    )
    .expect("write v1 manifest");

    let manifest = read_manifest(&destination).expect("read v1 manifest");
    assert_eq!(manifest.schema, BACKUP_MANIFEST_SCHEMA_V1);
    assert_eq!(manifest.format_version, BACKUP_FORMAT_VERSION_V1);
    assert!(manifest.cores.iter().all(|core| core.databases.is_empty()));

    let verification = verify_backup(&destination).expect("verify v1");
    assert_eq!(verification.status, BackupVerificationStatus::Verified);

    let (_target_parent, target) = target_path();
    let restored = restore(&destination, &target, RestoreOptions::default()).expect("restore v1");
    assert_eq!(restored.restored_cores.len(), 2);
    let installer = CoreInstaller::new(&target).expect("installer");
    assert_eq!(
        installer.verify(CORE_A).expect("verify A").status.as_str(),
        "verified"
    );
}

#[test]
fn corrupted_partial_backup_is_refused_without_writing_target() {
    let source = fabricate_full_root();
    let (_backup_parent, destination) = backup_path();
    backup(source.path(), &destination).expect("backup");

    // Simulate an interrupted/incomplete copy: truncate one asset.
    let asset = destination.join(format!(
        "cores/{CORE_A}/assets/11111111-1111-4111-8111-111111111111/avatar.bin"
    ));
    fs::write(&asset, b"partial").expect("truncate asset");

    let (_target_parent, target) = target_path();
    let error = restore(&destination, &target, RestoreOptions::default())
        .expect_err("corrupted backup must be refused");
    assert!(matches!(error, BackupError::InvalidBackup(_)));
    assert!(!target.exists(), "no partial target may be created");
}

#[test]
fn interrupted_partial_target_is_refused_and_preserved() {
    let source = fabricate_full_root();
    let (_backup_parent, destination) = backup_path();
    backup(source.path(), &destination).expect("backup");

    // Simulate an interrupted restore that left only one file behind.
    let (_target_parent, target) = target_path();
    fs::create_dir_all(core_dir(&target, CORE_A)).expect("partial target");
    let leftover = core_dir(&target, CORE_A).join("core.db");
    fs::copy(core_dir(source.path(), CORE_A).join("core.db"), &leftover).expect("leftover core.db");
    let leftover_bytes = fs::read(&leftover).expect("leftover bytes");

    let error = restore(&destination, &target, RestoreOptions::default())
        .expect_err("partial target must be refused");
    assert!(matches!(error, BackupError::RestoreTargetNotEmpty(_)));
    assert_eq!(fs::read(&leftover).expect("leftover after"), leftover_bytes);
    assert!(!target.join("platform.db").exists());
}

//! Batch 8 / Milestone 7 independent internal QA for end-to-end backup,
//! verification, restore, and per-Core isolation.
//!
//! The backup crate deliberately has no dependency on the SDK/conversation
//! crates, so this suite seeds product-shaped SQLite databases (memory,
//! personal, conversation) with the exact ownership/schema tables those
//! products use, then proves the bytes, rows, ownership, and identity survive a
//! full backup -> verify -> restore cycle.

use std::fs;
use std::path::{Path, PathBuf};

use aegis_backup::{
    backup, restore, verify_backup, BackupError, BackupVerificationStatus, RestoreOptions,
    CORE_DATABASE_FILE,
};
use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_core_install::CoreInstaller;
use aegis_core_storage::{CoreDatabase, CoreStorageLayout};
use aegis_platform::PlatformService;
use aegis_storage::now_rfc3339;
use rusqlite::{params, Connection};

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
    let installer = CoreInstaller::new(root).expect("installer");
    installer
        .initialize(&request(core_uuid, core_number, name), token)
        .expect("initialize Core");
}

fn setup_root() -> (tempfile::TempDir, String) {
    let temp = tempfile::tempdir().expect("tempdir");
    let initialized = PlatformService::initialize(temp.path()).expect("platform init");
    install(temp.path(), &initialized.owner_token, CORE_A, 1, "Core A");
    install(temp.path(), &initialized.owner_token, CORE_B, 2, "Core B");
    (temp, initialized.owner_token)
}

fn core_dir(root: &Path, core_uuid: &str) -> PathBuf {
    root.join("cores").join(core_uuid)
}

fn instance_uuid(root: &Path, core_uuid: &str) -> String {
    let connection =
        Connection::open(core_dir(root, core_uuid).join(CORE_DATABASE_FILE)).expect("open core db");
    connection
        .query_row(
            "SELECT database_instance_uuid FROM core_identity WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .expect("instance uuid")
}

fn seed_memory(root: &Path, core_uuid: &str, marker: &str) {
    let connection =
        Connection::open(core_dir(root, core_uuid).join("memory.db")).expect("open memory db");
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
                kind TEXT NOT NULL,
                content TEXT NOT NULL,
                source TEXT NOT NULL,
                provenance TEXT NOT NULL,
                confidence REAL NOT NULL CHECK(confidence >= 0.0 AND confidence <= 1.0),
                tags_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                archived INTEGER NOT NULL CHECK(archived IN (0, 1)),
                deleted INTEGER NOT NULL CHECK(deleted IN (0, 1)),
                revision INTEGER NOT NULL CHECK(revision >= 1)
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
            "INSERT INTO memory_entries(
                id, core_uuid, kind, content, source, provenance, confidence,
                tags_json, created_at, updated_at, archived, deleted, revision
             ) VALUES(?1, ?2, 'fact', ?3, 'qa', 'qa', 0.9, '[]', ?4, ?4, 0, 0, 1)",
            params![format!("memory-{core_uuid}"), core_uuid, marker, CREATED],
        )
        .expect("memory entry");
}

fn seed_personal(root: &Path, core_uuid: &str, marker: &str, joy: f64) {
    let instance = instance_uuid(root, core_uuid);
    let connection =
        Connection::open(core_dir(root, core_uuid).join("personal.db")).expect("open personal db");
    connection
        .execute_batch(
            "CREATE TABLE personal_ownership (
                singleton INTEGER NOT NULL PRIMARY KEY CHECK(singleton = 1),
                core_uuid TEXT NOT NULL,
                database_instance_uuid TEXT NOT NULL,
                schema TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE TABLE personal_schema (
                singleton INTEGER NOT NULL PRIMARY KEY CHECK(singleton = 1),
                schema TEXT NOT NULL,
                revision INTEGER NOT NULL CHECK(revision >= 1),
                created_at TEXT NOT NULL
            );
            CREATE TABLE emotion_state (
                core_uuid TEXT NOT NULL PRIMARY KEY,
                schema TEXT NOT NULL,
                valence REAL NOT NULL,
                arousal REAL NOT NULL,
                joy REAL NOT NULL,
                calm REAL NOT NULL,
                curiosity REAL NOT NULL,
                tension REAL NOT NULL,
                intensity REAL NOT NULL,
                updated_at TEXT NOT NULL,
                revision INTEGER NOT NULL CHECK(revision >= 1)
            );
            CREATE TABLE continuity (
                core_uuid TEXT NOT NULL PRIMARY KEY,
                last_active_at TEXT NOT NULL,
                last_session_id TEXT,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE personal_events (
                sequence INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
                core_uuid TEXT NOT NULL,
                kind TEXT NOT NULL,
                detail TEXT NOT NULL,
                session_id TEXT,
                created_at TEXT NOT NULL
            );
            PRAGMA journal_mode = DELETE;",
        )
        .expect("personal schema");
    connection
        .execute(
            "INSERT INTO personal_ownership(
                singleton, core_uuid, database_instance_uuid, schema, created_at
             ) VALUES(1, ?1, ?2, 'aegis.core.personal.v1', ?3)",
            params![core_uuid, instance, CREATED],
        )
        .expect("personal ownership");
    connection
        .execute(
            "INSERT INTO personal_schema(singleton, schema, revision, created_at)
             VALUES(1, 'aegis.core.personal.v1', 1, ?1)",
            params![CREATED],
        )
        .expect("personal schema marker");
    connection
        .execute(
            "INSERT INTO emotion_state(
                core_uuid, schema, valence, arousal, joy, calm, curiosity,
                tension, intensity, updated_at, revision
             ) VALUES(?1, 'aegis.core.personal.emotion.v1', 0.5, 0.4, ?2, 0.2, 0.3, 0.1, 0.5, ?3, 2)",
            params![core_uuid, joy, CREATED],
        )
        .expect("emotion state");
    connection
        .execute(
            "INSERT INTO continuity(core_uuid, last_active_at, last_session_id, updated_at)
             VALUES(?1, ?2, ?3, ?2)",
            params![core_uuid, CREATED, format!("session-{marker}")],
        )
        .expect("continuity");
    connection
        .execute(
            "INSERT INTO personal_events(core_uuid, kind, detail, session_id, created_at)
             VALUES(?1, 'experience', ?2, NULL, ?3)",
            params![core_uuid, marker, CREATED],
        )
        .expect("personal event");
}

fn seed_conversation(root: &Path, core_uuid: &str, marker: &str) {
    let instance = instance_uuid(root, core_uuid);
    let connection = Connection::open(core_dir(root, core_uuid).join("conversation.db"))
        .expect("open conversation db");
    connection
        .execute_batch(
            "CREATE TABLE conversation_ownership (
                singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                core_uuid TEXT NOT NULL UNIQUE,
                database_instance_uuid TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE TABLE conversation_sessions (
                session_id TEXT PRIMARY KEY,
                core_uuid TEXT NOT NULL,
                title TEXT NOT NULL,
                state TEXT NOT NULL CHECK(state IN ('active', 'archived', 'deleted')),
                provider_id TEXT,
                model TEXT,
                message_count INTEGER NOT NULL CHECK(message_count >= 0),
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                revision INTEGER NOT NULL CHECK(revision >= 1)
            );
            CREATE TABLE conversation_messages (
                message_id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL CHECK(role IN ('system', 'user', 'assistant', 'tool')),
                content TEXT NOT NULL,
                provider_id TEXT,
                model TEXT,
                prompt_tokens INTEGER,
                completion_tokens INTEGER,
                metadata_json TEXT,
                created_at TEXT NOT NULL
            );
            PRAGMA journal_mode = DELETE;",
        )
        .expect("conversation schema");
    connection
        .execute(
            "INSERT INTO conversation_ownership(
                singleton, core_uuid, database_instance_uuid, created_at
             ) VALUES(1, ?1, ?2, ?3)",
            params![core_uuid, instance, CREATED],
        )
        .expect("conversation ownership");
    let session_id = format!("session-{core_uuid}");
    connection
        .execute(
            "INSERT INTO conversation_sessions(
                session_id, core_uuid, title, state, provider_id, model,
                message_count, created_at, updated_at, revision
             ) VALUES(?1, ?2, ?3, 'active', 'qa', 'test-model', 2, ?4, ?4, 1)",
            params![
                session_id,
                core_uuid,
                format!("conversation-{marker}"),
                CREATED
            ],
        )
        .expect("conversation session");
    for (role, content) in [
        ("user", format!("user-{marker}")),
        ("assistant", format!("assistant-{marker}")),
    ] {
        connection
            .execute(
                "INSERT INTO conversation_messages(
                    session_id, role, content, provider_id, model, created_at
                 ) VALUES(?1, ?2, ?3, 'qa', 'test-model', ?4)",
                params![session_id, role, content, CREATED],
            )
            .expect("conversation message");
    }
}

fn seed_all(root: &Path) {
    seed_memory(root, CORE_A, "A-memory");
    seed_personal(root, CORE_A, "A-experience", 0.25);
    seed_conversation(root, CORE_A, "A-conversation");

    seed_memory(root, CORE_B, "B-memory");
    seed_personal(root, CORE_B, "B-experience", 0.75);
    seed_conversation(root, CORE_B, "B-conversation");
}

fn read_memory_content(root: &Path, core_uuid: &str) -> String {
    let connection =
        Connection::open(core_dir(root, core_uuid).join("memory.db")).expect("open memory db");
    connection
        .query_row(
            "SELECT content FROM memory_entries WHERE core_uuid = ?1",
            [core_uuid],
            |row| row.get(0),
        )
        .expect("memory content")
}

fn read_emotion_joy(root: &Path, core_uuid: &str) -> f64 {
    let connection =
        Connection::open(core_dir(root, core_uuid).join("personal.db")).expect("open personal db");
    connection
        .query_row(
            "SELECT joy FROM emotion_state WHERE core_uuid = ?1",
            [core_uuid],
            |row| row.get(0),
        )
        .expect("emotion joy")
}

fn read_last_session(root: &Path, core_uuid: &str) -> String {
    let connection =
        Connection::open(core_dir(root, core_uuid).join("personal.db")).expect("open personal db");
    connection
        .query_row(
            "SELECT last_session_id FROM continuity WHERE core_uuid = ?1",
            [core_uuid],
            |row| row.get(0),
        )
        .expect("last session")
}

fn read_personal_event(root: &Path, core_uuid: &str) -> String {
    let connection =
        Connection::open(core_dir(root, core_uuid).join("personal.db")).expect("open personal db");
    connection
        .query_row(
            "SELECT detail FROM personal_events WHERE core_uuid = ?1",
            [core_uuid],
            |row| row.get(0),
        )
        .expect("personal event")
}

fn read_session_title(root: &Path, core_uuid: &str) -> String {
    let connection = Connection::open(core_dir(root, core_uuid).join("conversation.db"))
        .expect("open conversation db");
    connection
        .query_row(
            "SELECT title FROM conversation_sessions WHERE core_uuid = ?1",
            [core_uuid],
            |row| row.get(0),
        )
        .expect("session title")
}

fn read_first_message(root: &Path, core_uuid: &str, role: &str) -> String {
    let connection = Connection::open(core_dir(root, core_uuid).join("conversation.db"))
        .expect("open conversation db");
    connection
        .query_row(
            "SELECT content FROM conversation_messages WHERE role = ?1 ORDER BY message_id ASC LIMIT 1",
            [role],
            |row| row.get(0),
        )
        .expect("message content")
}

fn ownership_core(root: &Path, core_uuid: &str, file_name: &str, table: &str) -> String {
    let connection =
        Connection::open(core_dir(root, core_uuid).join(file_name)).expect("open ownership db");
    connection
        .query_row(&format!("SELECT core_uuid FROM {table}"), [], |row| {
            row.get(0)
        })
        .expect("ownership core uuid")
}

fn fresh_destination(parent: &Path, name: &str) -> PathBuf {
    parent.join(name)
}

#[test]
fn end_to_end_backup_restore_preserves_data_and_isolates_cores() {
    let (source, _token) = setup_root();
    seed_all(source.path());

    let backup_parent = tempfile::tempdir().expect("backup parent");
    let destination = fresh_destination(backup_parent.path(), "snapshot");
    let report = backup(source.path(), &destination).expect("backup");
    assert_eq!(report.manifest.cores.len(), 2);
    assert!(report
        .manifest
        .files
        .iter()
        .any(|file| file.relative_path == format!("cores/{CORE_A}/personal.db")));
    assert!(report
        .manifest
        .files
        .iter()
        .any(|file| file.relative_path == format!("cores/{CORE_B}/conversation.db")));

    let verification = verify_backup(&destination).expect("verify backup");
    assert_eq!(verification.status, BackupVerificationStatus::Verified);
    assert!(verification.issues.is_empty());

    let target_parent = tempfile::tempdir().expect("target parent");
    let target = fresh_destination(target_parent.path(), "restored");
    let restored = restore(&destination, &target, RestoreOptions::default()).expect("restore");
    assert_eq!(restored.restored_cores.len(), 2);
    assert_eq!(restored.file_count, report.file_count);

    let installer = CoreInstaller::new(&target).expect("restored installer");
    for core in [CORE_A, CORE_B] {
        assert_eq!(
            installer.verify(core).expect("verify core").status.as_str(),
            "verified"
        );
    }

    // Real memory, personal, and conversation rows survive byte-for-byte.
    assert_eq!(read_memory_content(&target, CORE_A), "A-memory");
    assert_eq!(read_memory_content(&target, CORE_B), "B-memory");
    assert_eq!(read_emotion_joy(&target, CORE_A), 0.25);
    assert_eq!(read_emotion_joy(&target, CORE_B), 0.75);
    assert_eq!(read_last_session(&target, CORE_A), "session-A-experience");
    assert_eq!(read_personal_event(&target, CORE_A), "A-experience");
    assert_eq!(
        read_session_title(&target, CORE_A),
        "conversation-A-conversation"
    );
    assert_eq!(
        read_first_message(&target, CORE_A, "user"),
        "user-A-conversation"
    );
    assert_eq!(
        read_first_message(&target, CORE_A, "assistant"),
        "assistant-A-conversation"
    );

    // Ownership rows point at the owning Core after restore.
    for (file_name, table) in [
        ("memory.db", "core_memory_ownership"),
        ("personal.db", "personal_ownership"),
        ("conversation.db", "conversation_ownership"),
    ] {
        assert_eq!(ownership_core(&target, CORE_A, file_name, table), CORE_A);
        assert_eq!(ownership_core(&target, CORE_B, file_name, table), CORE_B);
    }

    // Core A's data cannot be interpreted as Core B's.
    assert_ne!(
        read_memory_content(&target, CORE_A),
        read_memory_content(&target, CORE_B)
    );
    let a_owned_in_b: i64 = {
        let connection = Connection::open(core_dir(&target, CORE_B).join("personal.db"))
            .expect("open B personal db");
        connection
            .query_row(
                "SELECT COUNT(*) FROM emotion_state WHERE core_uuid = ?1",
                [CORE_A],
                |row| row.get(0),
            )
            .expect("foreign row count")
    };
    assert_eq!(a_owned_in_b, 0);

    // Swapping A's personal database into B is rejected by a later backup.
    fs::copy(
        core_dir(&target, CORE_A).join("personal.db"),
        core_dir(&target, CORE_B).join("personal.db"),
    )
    .expect("swap personal database");
    let second_parent = tempfile::tempdir().expect("second parent");
    let second_destination = fresh_destination(second_parent.path(), "tainted");
    let error = backup(&target, &second_destination).expect_err("contamination is rejected");
    assert!(matches!(error, BackupError::IntegrityFailed { .. }));
    assert!(!second_destination.join("backup.manifest.json").exists());
}

#[test]
fn restored_identity_is_unchanged_and_backup_is_reproducible() {
    let (source, _token) = setup_root();
    seed_memory(source.path(), CORE_A, "A-memory");
    seed_personal(source.path(), CORE_A, "A-experience", 0.25);
    seed_conversation(source.path(), CORE_A, "A-conversation");

    let source_layout = CoreStorageLayout::new(source.path()).expect("source layout");
    let before = CoreDatabase::read_snapshot(&source_layout, CORE_A).expect("identity before");

    let backup_parent = tempfile::tempdir().expect("backup parent");
    let first_destination = fresh_destination(backup_parent.path(), "first");
    let first = backup(source.path(), &first_destination).expect("first backup");
    assert_eq!(
        verify_backup(&first_destination)
            .expect("verify first")
            .status,
        BackupVerificationStatus::Verified
    );

    let target_parent = tempfile::tempdir().expect("target parent");
    let target = fresh_destination(target_parent.path(), "restored");
    restore(&first_destination, &target, RestoreOptions::default()).expect("restore");
    CoreInstaller::new(&target)
        .expect("restored installer")
        .verify(CORE_A)
        .expect("verify restored core");

    let restored_layout = CoreStorageLayout::new(&target).expect("restored layout");
    let after = CoreDatabase::read_snapshot(&restored_layout, CORE_A).expect("identity after");
    assert_eq!(after.identity.core_uuid, before.identity.core_uuid);
    assert_eq!(after.identity.core_uuid, CORE_A);
    assert_eq!(after.identity.core_number, 1);
    assert_eq!(after.identity.display_name, "Core A");
    assert_eq!(
        after.metadata.database_instance_uuid,
        before.metadata.database_instance_uuid
    );
    assert_eq!(after.identity.created_at, before.identity.created_at);

    // The restored root can itself be backed up and verifies identically.
    let second_parent = tempfile::tempdir().expect("second parent");
    let second_destination = fresh_destination(second_parent.path(), "second");
    let second = backup(&target, &second_destination).expect("second backup");
    assert_eq!(
        verify_backup(&second_destination)
            .expect("verify second")
            .status,
        BackupVerificationStatus::Verified
    );
    assert_eq!(second.manifest.cores.len(), first.manifest.cores.len());
    assert_eq!(second.file_count, first.file_count);
    assert_eq!(second.total_bytes, first.total_bytes);
    assert_eq!(read_emotion_joy(&target, CORE_A), 0.25);
    assert_eq!(read_last_session(&target, CORE_A), "session-A-experience");
}

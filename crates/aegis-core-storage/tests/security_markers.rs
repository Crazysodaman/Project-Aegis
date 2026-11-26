use std::fs;

use aegis_contracts::core::CoreIdentity;
use aegis_core_storage::{CoreDatabase, CoreStorageError, CoreStorageLayout, CORE_APPLICATION_ID};
use aegis_storage::{PLATFORM_APPLICATION_ID, SAFE_APPLICATION_ID};
use rusqlite::Connection;

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_B: &str = "b1b1b1b1-2222-4333-8444-555566667777";
const MEMORY_APPLICATION_ID: i64 = 0x4145_474d;

fn identity() -> CoreIdentity {
    CoreIdentity::new(CORE_A, 1, "Alpha", "2026-11-25T00:00:00Z").expect("identity")
}

#[test]
fn foreign_application_markers_are_rejected_without_path_leak() {
    let temp = tempfile::tempdir().expect("tempdir");
    let layout = CoreStorageLayout::new(temp.path()).expect("layout");
    let root = layout.root().to_string_lossy().to_string();

    let identity = identity();
    let database = CoreDatabase::create_fresh(&layout, &identity).expect("create");
    let database_path = database.database_path().to_path_buf();
    database.close().expect("close");

    for marker in [
        PLATFORM_APPLICATION_ID,
        SAFE_APPLICATION_ID,
        MEMORY_APPLICATION_ID,
    ] {
        assert_ne!(
            marker, CORE_APPLICATION_ID,
            "test marker must differ from the Core marker"
        );
        {
            let connection = Connection::open(&database_path).expect("open database");
            connection
                .execute_batch(&format!("PRAGMA application_id = {marker};"))
                .expect("set foreign marker");
        }

        let error = CoreDatabase::open(&layout, CORE_A).expect_err("foreign marker must fail");
        let text = error.to_string();
        assert!(matches!(error, CoreStorageError::ForeignOwnership));
        assert!(!text.contains("core.db"), "database file leaked: {text}");
        assert!(!text.contains(&root), "storage root leaked: {text}");

        {
            let connection = Connection::open(&database_path).expect("reopen database");
            connection
                .execute_batch(&format!("PRAGMA application_id = {CORE_APPLICATION_ID};"))
                .expect("restore marker");
        }
    }

    let reopened = CoreDatabase::open(&layout, CORE_A).expect("reopen with Core marker");
    assert_eq!(reopened.identity().expect("identity"), identity);
}

#[test]
fn core_marker_is_distinct_from_platform_and_safe() {
    assert_ne!(CORE_APPLICATION_ID, PLATFORM_APPLICATION_ID);
    assert_ne!(CORE_APPLICATION_ID, SAFE_APPLICATION_ID);
    assert_ne!(PLATFORM_APPLICATION_ID, SAFE_APPLICATION_ID);
}

#[test]
fn corrupt_core_database_bytes_are_rejected_without_path_leak() {
    let temp = tempfile::tempdir().expect("tempdir");
    let layout = CoreStorageLayout::new(temp.path()).expect("layout");
    let root = layout.root().to_string_lossy().to_string();
    let identity = identity();
    let database = CoreDatabase::create_fresh(&layout, &identity).expect("create");
    let database_path = database.database_path().to_path_buf();
    database.close().expect("close");

    fs::write(&database_path, b"this is not a sqlite database").expect("corrupt database");

    let error = CoreDatabase::open(&layout, CORE_A).expect_err("corrupt database must fail");
    let text = error.to_string();
    assert!(!text.contains("core.db"), "database file leaked: {text}");
    assert!(!text.contains(&root), "storage root leaked: {text}");
}

#[test]
fn core_database_is_bound_to_its_owning_core_uuid() {
    let temp = tempfile::tempdir().expect("tempdir");
    let layout = CoreStorageLayout::new(temp.path()).expect("layout");
    let root = layout.root().to_string_lossy().to_string();
    let owner = identity();

    let database = CoreDatabase::create_fresh(&layout, &owner).expect("create");
    let source = database.database_path().to_path_buf();
    database.close().expect("close");

    let other_dir = layout.core_dir(CORE_B).expect("other core dir");
    fs::create_dir_all(&other_dir).expect("create other core dir");
    let other_db = other_dir.join("core.db");
    fs::copy(&source, &other_db).expect("copy owning database");

    let error = CoreDatabase::open(&layout, CORE_B).expect_err("foreign identity must fail");
    assert!(matches!(error, CoreStorageError::ForeignOwnership));
    let text = error.to_string();
    assert!(!text.contains("core.db"), "database file leaked: {text}");
    assert!(!text.contains(&root), "storage root leaked: {text}");

    let snapshot =
        CoreDatabase::read_snapshot(&layout, CORE_B).expect_err("foreign snapshot must fail");
    assert!(matches!(snapshot, CoreStorageError::ForeignOwnership));
}

//! Per-Core storage isolation and ownership tests (Batch 2 / Wave 1b).

use std::fs;

use aegis_contracts::core::{CoreContractError, CoreIdentity, CORE_STORAGE_SCHEMA};
use aegis_contracts::initialization::FRESH_INITIALIZATION_MODE;
use aegis_core_storage::{
    CoreDatabase, CoreStorageError, CoreStorageLayout, CORE_APPLICATION_ID, CORE_DB_FILE,
    CORE_INITIALIZATION_MARKER_FILE, CORE_INTEGRITY_FILE,
};
use aegis_storage::{
    open_initialized, verify_application_id, StoragePaths, PLATFORM_APPLICATION_ID,
    SAFE_APPLICATION_ID, SCHEMA_REVISION,
};
use rusqlite::Connection;

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_B: &str = "b1b1b1b1-2222-4333-8444-555566667777";
const CORE_C: &str = "c2c2c2c2-3333-4444-8555-666677778888";

fn identity(core_uuid: &str, core_number: u8, display_name: &str) -> CoreIdentity {
    CoreIdentity::new(core_uuid, core_number, display_name, "2026-11-22T00:00:00Z")
        .expect("valid identity")
}

fn layout(root: &std::path::Path) -> CoreStorageLayout {
    CoreStorageLayout::new(root).expect("layout")
}

#[test]
fn multiple_cores_are_independent_and_do_not_leak() {
    let temp = tempfile::tempdir().expect("tempdir");
    let layout = layout(temp.path());

    let alpha = identity(CORE_A, 1, "Alpha");
    let beta = identity(CORE_B, 2, "Beta");
    let gamma = identity(CORE_C, 3, "Gamma");

    let alpha_db = CoreDatabase::create_fresh(&layout, &alpha).expect("alpha");
    let beta_db = CoreDatabase::create_fresh(&layout, &beta).expect("beta");
    let gamma_db = CoreDatabase::create_fresh(&layout, &gamma).expect("gamma");

    assert_ne!(alpha_db.database_path(), beta_db.database_path());
    assert_ne!(beta_db.database_path(), gamma_db.database_path());
    assert_ne!(alpha_db.database_path(), gamma_db.database_path());

    assert_eq!(alpha_db.identity().expect("alpha id"), alpha);
    assert_eq!(beta_db.identity().expect("beta id"), beta);
    assert_eq!(gamma_db.identity().expect("gamma id"), gamma);

    let alpha_renamed = alpha_db
        .rename_display_name("Alpha Prime")
        .expect("rename alpha");
    assert_eq!(alpha_renamed.core_uuid, CORE_A);
    assert_eq!(
        alpha_db.identity().expect("alpha id").display_name,
        "Alpha Prime"
    );
    assert_eq!(beta_db.identity().expect("beta id"), beta);
    assert_eq!(gamma_db.identity().expect("gamma id"), gamma);

    let alpha_path = alpha_db.database_path().to_path_buf();
    let beta_path = beta_db.database_path().to_path_buf();
    let gamma_path = gamma_db.database_path().to_path_buf();

    alpha_db.close().expect("close alpha");
    beta_db.close().expect("close beta");
    gamma_db.close().expect("close gamma");

    assert_eq!(
        CoreDatabase::open(&layout, CORE_A)
            .expect("reopen alpha")
            .identity()
            .expect("alpha identity"),
        alpha_renamed
    );
    assert_eq!(
        CoreDatabase::open(&layout, CORE_B)
            .expect("reopen beta")
            .identity()
            .expect("beta identity"),
        beta
    );
    assert_eq!(
        CoreDatabase::open(&layout, CORE_C)
            .expect("reopen gamma")
            .identity()
            .expect("gamma identity"),
        gamma
    );

    for (path, expected_uuid) in [
        (alpha_path, CORE_A),
        (beta_path, CORE_B),
        (gamma_path, CORE_C),
    ] {
        let connection = Connection::open(&path).expect("direct open");
        let rows: Vec<String> = {
            let mut statement = connection
                .prepare("SELECT core_uuid FROM core_identity")
                .expect("prepare");
            statement
                .query_map([], |row| row.get(0))
                .expect("query")
                .collect::<Result<_, _>>()
                .expect("rows")
        };
        assert_eq!(rows, vec![expected_uuid.to_owned()]);
    }
}

#[test]
fn identical_display_names_do_not_collide() {
    let temp = tempfile::tempdir().expect("tempdir");
    let layout = layout(temp.path());

    let alpha = identity(CORE_A, 1, "Shared Name");
    let beta = identity(CORE_B, 2, "Shared Name");

    let alpha_db = CoreDatabase::create_fresh(&layout, &alpha).expect("alpha");
    let beta_db = CoreDatabase::create_fresh(&layout, &beta).expect("beta");

    assert_eq!(
        alpha_db.identity().expect("alpha").display_name,
        "Shared Name"
    );
    assert_eq!(
        beta_db.identity().expect("beta").display_name,
        "Shared Name"
    );
    assert_ne!(alpha_db.database_path(), beta_db.database_path());
    assert_eq!(alpha_db.database_path().file_name().unwrap(), CORE_DB_FILE);
    assert_eq!(beta_db.database_path().file_name().unwrap(), CORE_DB_FILE);

    alpha_db.close().expect("close alpha");
    beta_db.close().expect("close beta");
}

#[test]
fn path_traversal_and_foreign_identifiers_are_rejected() {
    let temp = tempfile::tempdir().expect("tempdir");
    let layout = layout(temp.path());

    for bad in [
        "",
        "..",
        "../core.db",
        "../../outside",
        "cores/../../outside",
        "a/b",
        "/absolute",
        "C:\\absolute",
        "00000000-0000-0000-0000-000000000000",
        "not-a-uuid",
    ] {
        assert!(
            layout.core_dir(bad).is_err(),
            "core_dir must reject {bad:?}"
        );
        assert!(
            layout.database_path(bad).is_err(),
            "database_path must reject {bad:?}"
        );
        assert!(
            layout.integrity_path(bad).is_err(),
            "integrity_path must reject {bad:?}"
        );
        assert!(
            layout.initialization_marker_path(bad).is_err(),
            "marker path must reject {bad:?}"
        );
    }

    let traversal_uuid = format!("../{CORE_A}");
    assert!(matches!(
        layout.database_path(&traversal_uuid),
        Err(CoreStorageError::InvalidCoreId)
    ));
}

#[test]
fn approved_root_enforcement_and_sidecar_paths() {
    let temp = tempfile::tempdir().expect("tempdir");
    let layout = layout(temp.path());

    let cores_root = layout.cores_root().to_path_buf();
    let core_dir = layout.core_dir(CORE_A).expect("core dir");
    let database_path = layout.database_path(CORE_A).expect("db path");
    let integrity_path = layout.integrity_path(CORE_A).expect("integrity path");
    let marker_path = layout
        .initialization_marker_path(CORE_A)
        .expect("marker path");

    assert!(cores_root.ends_with("cores"));
    assert!(core_dir.starts_with(&cores_root));
    assert!(database_path.starts_with(&cores_root));
    assert!(integrity_path.starts_with(&cores_root));
    assert!(marker_path.starts_with(layout.root()));
    assert_eq!(core_dir.parent(), Some(cores_root.as_path()));
    assert_eq!(database_path.parent(), Some(core_dir.as_path()));
    assert_eq!(integrity_path.file_name().unwrap(), CORE_INTEGRITY_FILE);
    assert!(marker_path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(CORE_INITIALIZATION_MARKER_FILE)));

    assert!(matches!(
        CoreStorageLayout::new("relative-root"),
        Err(CoreStorageError::RelativeRoot)
    ));

    let root_file = temp.path().join("not-a-directory");
    fs::write(&root_file, b"x").expect("write file");
    assert!(CoreStorageLayout::new(&root_file).is_err());
}

#[test]
fn platform_and_safe_databases_are_never_touched() {
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = StoragePaths::new(temp.path()).expect("storage paths");

    {
        let platform = open_initialized(&paths.platform_db(), PLATFORM_APPLICATION_ID, "platform")
            .expect("platform");
        verify_application_id(&platform, PLATFORM_APPLICATION_ID).expect("platform marker");
    }
    {
        let safe = open_initialized(&paths.safe_db(), SAFE_APPLICATION_ID, "safe").expect("safe");
        verify_application_id(&safe, SAFE_APPLICATION_ID).expect("safe marker");
    }

    let platform_before = fs::read(paths.platform_db()).expect("platform bytes");
    let safe_before = fs::read(paths.safe_db()).expect("safe bytes");

    let layout = CoreStorageLayout::new(temp.path()).expect("layout");
    let db = CoreDatabase::create_fresh(&layout, &identity(CORE_A, 1, "Alpha")).expect("core");
    let core_path = db.database_path().to_path_buf();
    db.close().expect("close");

    assert_ne!(core_path, paths.platform_db());
    assert_ne!(core_path, paths.safe_db());
    assert_eq!(
        fs::read(paths.platform_db()).expect("platform bytes"),
        platform_before
    );
    assert_eq!(fs::read(paths.safe_db()).expect("safe bytes"), safe_before);

    let platform = open_initialized(&paths.platform_db(), PLATFORM_APPLICATION_ID, "platform")
        .expect("platform reopen");
    verify_application_id(&platform, PLATFORM_APPLICATION_ID).expect("platform marker");
    drop(platform);

    let safe =
        open_initialized(&paths.safe_db(), SAFE_APPLICATION_ID, "safe").expect("safe reopen");
    verify_application_id(&safe, SAFE_APPLICATION_ID).expect("safe marker");
}

#[test]
fn existing_same_core_is_reused_but_conflicts_are_rejected() {
    let temp = tempfile::tempdir().expect("tempdir");
    let layout = layout(temp.path());

    let original = identity(CORE_A, 1, "Alpha");
    let first = CoreDatabase::create_fresh(&layout, &original).expect("first");
    let first_path = first.database_path().to_path_buf();
    let first_identity = first.identity().expect("identity");
    first.close().expect("close");

    let second = CoreDatabase::create_fresh(&layout, &original).expect("idempotent recreate");
    assert_eq!(second.database_path(), first_path.as_path());
    assert_eq!(second.identity().expect("identity"), first_identity);
    second.close().expect("close");

    let conflict = identity(CORE_A, 2, "Different Core Number");
    let error = CoreDatabase::create_fresh(&layout, &conflict).expect_err("number conflict");
    assert!(matches!(error, CoreStorageError::ForeignOwnership));
}

#[test]
fn partial_interrupted_directory_state_is_rejected() {
    let temp = tempfile::tempdir().expect("tempdir");
    let layout = layout(temp.path());

    let directory = layout.core_dir(CORE_A).expect("core dir");
    fs::create_dir_all(&directory).expect("create dir");
    fs::write(directory.join("stray.tmp"), b"partial").expect("write stray");

    let error = CoreDatabase::create_fresh(&layout, &identity(CORE_A, 1, "Alpha"))
        .expect_err("partial state must fail");
    assert!(matches!(error, CoreStorageError::PartialState));

    let interrupted_directory = layout.core_dir(CORE_B).expect("interrupted dir");
    fs::create_dir_all(&interrupted_directory).expect("create interrupted dir");
    let interrupted_path = layout.database_path(CORE_B).expect("interrupted db path");
    drop(
        open_initialized(&interrupted_path, CORE_APPLICATION_ID, "core")
            .expect("database without identity"),
    );

    let error = CoreDatabase::create_fresh(&layout, &identity(CORE_B, 2, "Beta"))
        .expect_err("interrupted database must fail");
    assert!(matches!(error, CoreStorageError::PartialState));
}

#[test]
fn ownership_substitution_and_foreign_markers_are_rejected() {
    let temp = tempfile::tempdir().expect("tempdir");
    let layout = layout(temp.path());

    let alpha = identity(CORE_A, 1, "Alpha");
    let alpha_db = CoreDatabase::create_fresh(&layout, &alpha).expect("alpha");
    let alpha_path = alpha_db.database_path().to_path_buf();
    alpha_db.close().expect("close");

    let beta_directory = layout.core_dir(CORE_B).expect("beta dir");
    fs::create_dir_all(&beta_directory).expect("beta directory");
    fs::copy(&alpha_path, beta_directory.join(CORE_DB_FILE)).expect("copy database");

    let substituted = CoreDatabase::open(&layout, CORE_B).expect_err("substitution must fail");
    assert!(matches!(substituted, CoreStorageError::ForeignOwnership));

    {
        let connection = Connection::open(&alpha_path).expect("direct open");
        connection
            .execute(
                "UPDATE core_identity SET core_uuid = ?1 WHERE singleton = 1",
                [CORE_C],
            )
            .expect("substitute row");
    }
    let tampered = CoreDatabase::open(&layout, CORE_A).expect_err("tampered row must fail");
    assert!(matches!(tampered, CoreStorageError::ForeignOwnership));

    let gamma = identity(CORE_C, 3, "Gamma");
    let gamma_db = CoreDatabase::create_fresh(&layout, &gamma).expect("gamma");
    let gamma_path = gamma_db.database_path().to_path_buf();
    gamma_db.close().expect("close gamma");

    {
        let connection = Connection::open(&gamma_path).expect("direct open");
        connection
            .execute_batch(&format!(
                "PRAGMA application_id = {PLATFORM_APPLICATION_ID};"
            ))
            .expect("foreign marker");
    }
    let foreign = CoreDatabase::open(&layout, CORE_C).expect_err("foreign marker must fail");
    assert!(matches!(foreign, CoreStorageError::ForeignOwnership));
    assert_ne!(
        CORE_APPLICATION_ID, PLATFORM_APPLICATION_ID,
        "Core marker must not match the Platform marker"
    );
    assert_ne!(
        CORE_APPLICATION_ID, SAFE_APPLICATION_ID,
        "Core marker must not match the Safe marker"
    );
}

#[test]
fn missing_database_is_rejected() {
    let temp = tempfile::tempdir().expect("tempdir");
    let layout = layout(temp.path());

    let error = CoreDatabase::open(&layout, CORE_A).expect_err("missing database");
    assert!(matches!(error, CoreStorageError::DatabaseMissing));
}

#[test]
fn forbidden_core_numbers_are_rejected() {
    let temp = tempfile::tempdir().expect("tempdir");
    let layout = layout(temp.path());

    for number in [4u8, 5u8] {
        let mut forbidden = identity(CORE_A, 1, "Forbidden");
        forbidden.core_number = number;
        let error = CoreDatabase::create_fresh(&layout, &forbidden)
            .expect_err("prohibited number must fail");
        assert!(matches!(
            error,
            CoreStorageError::Contract(CoreContractError::ProhibitedCoreNumber)
        ));
    }
}

#[test]
fn display_name_mutation_preserves_uuid_and_increments_revision() {
    let temp = tempfile::tempdir().expect("tempdir");
    let layout = layout(temp.path());

    let original = identity(CORE_A, 1, "Alpha");
    let db = CoreDatabase::create_fresh(&layout, &original).expect("create");
    let before = db.identity().expect("before");

    let after = db
        .rename_display_name("Alpha Ada Lyra")
        .expect("rename display name");
    assert_eq!(after.core_uuid, before.core_uuid);
    assert_eq!(after.core_number, before.core_number);
    assert_eq!(after.created_at, before.created_at);
    assert_eq!(after.identity_revision, before.identity_revision + 1);
    assert_eq!(after.display_name, "Alpha Ada Lyra");
    assert_eq!(db.identity().expect("current"), after);

    let metadata = db.metadata().expect("metadata");
    assert_eq!(metadata.schema, CORE_STORAGE_SCHEMA);
    assert_eq!(metadata.core_uuid, CORE_A);
    assert_eq!(metadata.core_number, 1);
    assert_eq!(metadata.display_name, "Alpha Ada Lyra");
    assert_eq!(metadata.initialization_mode, FRESH_INITIALIZATION_MODE);
    assert_eq!(metadata.schema_revision, SCHEMA_REVISION as u64);
    assert_eq!(metadata.created_at, original.created_at);

    assert!(db.rename_display_name("   ").is_err());
    assert_eq!(db.identity().expect("unchanged"), after);

    let path = db.database_path().to_path_buf();
    db.close().expect("close");

    let reopened = CoreDatabase::open(&layout, CORE_A).expect("reopen");
    assert_eq!(reopened.database_path(), path.as_path());
    assert_eq!(reopened.identity().expect("reopened"), after);
    reopened.verify_sqlite_integrity().expect("integrity");
}

#[test]
fn integrity_check_passes_for_fresh_database() {
    let temp = tempfile::tempdir().expect("tempdir");
    let layout = layout(temp.path());

    let db = CoreDatabase::create_fresh(&layout, &identity(CORE_A, 1, "Alpha")).expect("create");
    db.verify_sqlite_integrity().expect("integrity");
    db.close().expect("close");
}

#[cfg(unix)]
#[test]
fn redirected_root_and_core_directory_are_rejected() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("tempdir");
    let real_root = temp.path().join("real");
    fs::create_dir_all(&real_root).expect("real root");
    let linked_root = temp.path().join("linked");
    symlink(&real_root, &linked_root).expect("root symlink");

    assert!(matches!(
        CoreStorageLayout::new(&linked_root),
        Err(CoreStorageError::RedirectedRoot)
    ));

    let layout = layout(&real_root);
    let cores_root = layout.cores_root().to_path_buf();
    let outside = temp.path().join("outside");
    fs::create_dir_all(&outside).expect("outside");
    symlink(&outside, cores_root.join(CORE_A)).expect("core symlink");

    let error = CoreDatabase::create_fresh(&layout, &identity(CORE_A, 1, "Alpha"))
        .expect_err("redirected core dir must fail");
    assert!(matches!(error, CoreStorageError::RedirectedComponent));
}

#[test]
fn delete_journal_policy_leaves_no_wal_or_shm_sidecars() {
    let temp = tempfile::tempdir().expect("tempdir");
    let layout = layout(temp.path());
    let database = CoreDatabase::create_fresh(&layout, &identity(CORE_A, 1, "Alpha"))
        .expect("create fresh core database");
    let database_path = database.database_path().to_path_buf();
    database.verify_sqlite_integrity().expect("integrity");
    database.close().expect("close");

    let journal_mode: String = open_initialized(&database_path, CORE_APPLICATION_ID, "core")
        .expect("open core db")
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .expect("journal mode");
    assert_eq!(journal_mode.to_ascii_lowercase(), "delete");

    for suffix in ["-wal", "-shm"] {
        let mut name = database_path.as_os_str().to_os_string();
        name.push(suffix);
        assert!(
            !std::path::Path::new(&name).exists(),
            "unexpected SQLite sidecar {}",
            std::path::Path::new(&name).display()
        );
    }
}

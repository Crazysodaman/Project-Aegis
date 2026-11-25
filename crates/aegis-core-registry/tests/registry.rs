use aegis_contracts::lifecycle::CoreLifecycleState;
use aegis_contracts::roster::{CoreRegistrationRequest, CORE_REGISTRY_SCHEMA};
use aegis_core_registry::{CoreRegistry, RegistryError};
use aegis_storage::{
    open_initialized, open_read_only, StoragePaths, PLATFORM_APPLICATION_ID, SAFE_APPLICATION_ID,
};
use rusqlite::params;

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_B: &str = "b9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_C: &str = "c9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_APPLICATION_ID: i64 = 0x4145_4743; // "AEGC" marker owned by the Core agent.

fn temp_paths() -> (tempfile::TempDir, StoragePaths) {
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = StoragePaths::new(temp.path()).expect("storage paths");
    (temp, paths)
}

fn request(
    core_uuid: &str,
    core_number: u8,
    display_name: &str,
    storage_relative_path: &str,
) -> CoreRegistrationRequest {
    CoreRegistrationRequest {
        schema: CORE_REGISTRY_SCHEMA.to_owned(),
        core_uuid: core_uuid.to_owned(),
        core_number,
        display_name: display_name.to_owned(),
        storage_relative_path: storage_relative_path.to_owned(),
        initialized_at: "2026-11-22T00:00:00Z".to_owned(),
    }
}

fn core_path(core_uuid: &str) -> String {
    format!("cores/{core_uuid}/core.db")
}

fn registry(paths: &StoragePaths) -> CoreRegistry {
    let registry = CoreRegistry::new(paths.clone());
    registry.initialize().expect("initialize registry");
    registry
}

fn initialize_legacy_platform(paths: &StoragePaths) -> rusqlite::Connection {
    let connection = open_initialized(&paths.platform_db(), PLATFORM_APPLICATION_ID, "platform")
        .expect("legacy platform db");
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS platform_core (
                core_uuid TEXT PRIMARY KEY,
                core_number INTEGER NOT NULL CHECK(core_number IN (1, 2, 3, 6, 7)),
                lifecycle_state TEXT NOT NULL CHECK(lifecycle_state IN (
                    'design_only', 'approved', 'installed', 'active', 'suspended', 'retired'
                )),
                created_at TEXT NOT NULL
            );",
        )
        .expect("legacy platform_core table");
    connection
}

fn insert_legacy_core(
    connection: &rusqlite::Connection,
    core_uuid: &str,
    core_number: i64,
    lifecycle_state: &str,
    created_at: &str,
) {
    connection
        .execute(
            "INSERT INTO platform_core(core_uuid, core_number, lifecycle_state, created_at)
             VALUES(?1, ?2, ?3, ?4)",
            params![core_uuid, core_number, lifecycle_state, created_at],
        )
        .expect("insert legacy core");
}

#[test]
fn register_list_and_get() {
    let (_temp, paths) = temp_paths();
    let registry = registry(&paths);

    let entry = registry
        .register(request(CORE_A, 1, "Sofía", &core_path(CORE_A)))
        .expect("register A");
    assert_eq!(entry.core_uuid, CORE_A);
    assert_eq!(entry.core_number, 1);
    assert_eq!(entry.display_name, "Sofía");
    assert_eq!(entry.lifecycle_state, CoreLifecycleState::Installed);
    assert_eq!(entry.schema, CORE_REGISTRY_SCHEMA);

    let fetched = registry.get(CORE_A).expect("get").expect("present");
    assert_eq!(fetched, entry);

    registry
        .register(request(CORE_B, 6, "Ada", &core_path(CORE_B)))
        .expect("register B");

    let listed = registry.list().expect("list");
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].core_number, 1);
    assert_eq!(listed[1].core_number, 6);
}

#[test]
fn duplicate_uuid_and_number_are_rejected() {
    let (_temp, paths) = temp_paths();
    let registry = registry(&paths);
    registry
        .register(request(CORE_A, 1, "Sofía", &core_path(CORE_A)))
        .expect("register A");

    let duplicate_uuid = registry
        .register(request(CORE_A, 1, "Sofía", &core_path(CORE_A)))
        .expect_err("duplicate UUID");
    assert!(matches!(
        duplicate_uuid,
        RegistryError::DuplicateCoreUuid(ref uuid) if uuid == CORE_A
    ));

    let duplicate_number = registry
        .register(request(CORE_B, 1, "Ada", &core_path(CORE_B)))
        .expect_err("duplicate number");
    assert!(matches!(
        duplicate_number,
        RegistryError::DuplicateCoreNumber(1)
    ));

    let duplicate_path = registry
        .register(request(CORE_B, 6, "Ada", &core_path(CORE_A)))
        .expect_err("foreign storage path");
    assert!(matches!(
        duplicate_path,
        RegistryError::ForeignEntry(ref owner) if owner == CORE_A
    ));

    assert_eq!(registry.list().expect("list").len(), 1);
}

#[test]
fn display_name_change_preserves_immutable_identity() {
    let (_temp, paths) = temp_paths();
    let registry = registry(&paths);
    let original = registry
        .register(request(CORE_A, 1, "Sofía", &core_path(CORE_A)))
        .expect("register");

    let renamed = registry
        .update_display_name(CORE_A, "Sofía Ada Lyra")
        .expect("rename");
    assert_eq!(renamed.core_uuid, original.core_uuid);
    assert_eq!(renamed.core_number, original.core_number);
    assert_eq!(
        renamed.storage_relative_path,
        original.storage_relative_path
    );
    assert_eq!(renamed.initialized_at, original.initialized_at);
    assert_eq!(renamed.display_name, "Sofía Ada Lyra");
    assert_ne!(renamed.display_name, original.display_name);

    assert!(matches!(
        registry.update_display_name(CORE_A, "   "),
        Err(RegistryError::InvalidDisplayName)
    ));
    assert!(matches!(
        registry.update_display_name(CORE_B, "Unknown"),
        Err(RegistryError::CoreNotFound(_))
    ));
}

#[test]
fn deactivate_preserves_entry_data_and_history() {
    let (_temp, paths) = temp_paths();
    let registry = registry(&paths);
    let entry = registry
        .register(request(CORE_A, 1, "Sofía", &core_path(CORE_A)))
        .expect("register");

    let data_file = paths.root().join(&entry.storage_relative_path);
    std::fs::create_dir_all(data_file.parent().expect("parent")).expect("core dir");
    std::fs::write(&data_file, b"core-owned-data").expect("core data");

    let suspended = registry
        .deactivate(CORE_A, "scheduled maintenance")
        .expect("deactivate");
    assert_eq!(suspended.lifecycle_state, CoreLifecycleState::Suspended);
    assert_eq!(suspended.storage_relative_path, entry.storage_relative_path);
    assert_eq!(suspended.core_uuid, entry.core_uuid);
    assert_eq!(
        std::fs::read(&data_file).expect("data still present"),
        b"core-owned-data"
    );

    let history = registry.lifecycle_history(CORE_A).expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].from, CoreLifecycleState::Installed);
    assert_eq!(history[0].to, CoreLifecycleState::Suspended);
    assert_eq!(history[0].reason, "scheduled maintenance");

    let illegal = registry
        .deactivate(CORE_A, "again")
        .expect_err("suspended cannot suspend again");
    assert!(matches!(
        illegal,
        RegistryError::IllegalTransition { ref from, ref to }
            if from == "suspended" && to == "suspended"
    ));

    assert!(matches!(
        registry.deactivate(CORE_B, "unknown"),
        Err(RegistryError::CoreNotFound(_))
    ));
    assert!(matches!(
        registry.lifecycle_history(CORE_B),
        Err(RegistryError::CoreNotFound(_))
    ));
}

#[test]
fn active_core_can_be_deactivated() {
    let (_temp, paths) = temp_paths();
    let registry = registry(&paths);
    registry
        .register(request(CORE_A, 1, "Sofía", &core_path(CORE_A)))
        .expect("register");

    {
        let connection = open_initialized(
            &paths.platform_db(),
            PLATFORM_APPLICATION_ID,
            "core_registry",
        )
        .expect("platform db");
        connection
            .execute(
                "UPDATE core_registry SET lifecycle_state = 'active' WHERE core_uuid = ?1",
                [CORE_A],
            )
            .expect("mark active");
    }

    let suspended = registry
        .deactivate(CORE_A, "operator request")
        .expect("deactivate active");
    assert_eq!(suspended.lifecycle_state, CoreLifecycleState::Suspended);

    let history = registry.lifecycle_history(CORE_A).expect("history");
    assert_eq!(
        history.last().expect("change").from,
        CoreLifecycleState::Active
    );
}

#[test]
fn prohibited_invalid_and_reserved_requests_are_rejected() {
    let (_temp, paths) = temp_paths();
    let registry = registry(&paths);

    for number in [4u8, 5u8] {
        let error = registry
            .register(request(CORE_A, number, "Prohibited", &core_path(CORE_A)))
            .expect_err("prohibited number");
        assert!(matches!(error, RegistryError::InvalidRequest(_)));
    }

    for path in ["", "../escape/core.db", "/absolute/core.db", "C:/core.db"] {
        let error = registry
            .register(request(CORE_A, 1, "Sofía", path))
            .expect_err("invalid path");
        assert!(matches!(error, RegistryError::InvalidRequest(_)));
    }

    for path in ["platform.db", "safe.db"] {
        let error = registry
            .register(request(CORE_A, 1, "Sofía", path))
            .expect_err("reserved path");
        assert!(matches!(error, RegistryError::ReservedStoragePath));
    }

    assert!(matches!(
        registry.get("not-a-uuid"),
        Err(RegistryError::InvalidCoreUuid)
    ));
    assert!(matches!(
        registry.deactivate(CORE_A, ""),
        Err(RegistryError::InvalidCoreUuid) | Err(RegistryError::MissingReason)
    ));
    assert!(registry.list().expect("list").is_empty());
}

#[test]
fn zero_core_platform_is_valid() {
    let (_temp, paths) = temp_paths();
    let registry = registry(&paths);
    registry.initialize().expect("initialize is idempotent");

    assert!(registry.list().expect("list").is_empty());
    assert!(registry.get(CORE_A).expect("get").is_none());
    assert!(matches!(
        registry.lifecycle_history(CORE_A),
        Err(RegistryError::CoreNotFound(_))
    ));
}

#[test]
fn one_core_failure_does_not_affect_other_entries() {
    let (_temp, paths) = temp_paths();
    let registry = registry(&paths);
    registry
        .register(request(CORE_A, 1, "Sofía", &core_path(CORE_A)))
        .expect("register A");
    registry
        .register(request(CORE_B, 6, "Ada", &core_path(CORE_B)))
        .expect("register B");

    let failure = registry
        .register(request(CORE_A, 6, "Sofía Clone", &core_path(CORE_C)))
        .expect_err("duplicate UUID must fail");
    assert!(matches!(failure, RegistryError::DuplicateCoreUuid(_)));

    let suspended = registry
        .deactivate(CORE_A, "isolated failure")
        .expect("deactivate A");
    assert_eq!(suspended.lifecycle_state, CoreLifecycleState::Suspended);

    let other = registry
        .get(CORE_B)
        .expect("get B")
        .expect("B remains registered");
    assert_eq!(other.lifecycle_state, CoreLifecycleState::Installed);
    assert_eq!(registry.list().expect("list").len(), 2);
}

#[test]
fn platform_safe_and_core_databases_remain_separate() {
    let (_temp, paths) = temp_paths();
    let registry = registry(&paths);

    let core_relative = core_path(CORE_A);
    let core_db = paths.root().join(&core_relative);
    std::fs::create_dir_all(core_db.parent().expect("parent")).expect("core dir");
    open_initialized(&paths.safe_db(), SAFE_APPLICATION_ID, "safe").expect("safe db");
    open_initialized(&core_db, CORE_APPLICATION_ID, "core").expect("core db");

    registry
        .register(request(CORE_A, 1, "Sofía", &core_relative))
        .expect("register core");

    assert_ne!(paths.platform_db(), paths.safe_db());
    assert_ne!(paths.platform_db(), core_db);
    assert_ne!(paths.safe_db(), core_db);

    let platform =
        open_read_only(&paths.platform_db(), PLATFORM_APPLICATION_ID).expect("platform marker");
    let safe = open_read_only(&paths.safe_db(), SAFE_APPLICATION_ID).expect("safe marker");
    let core = open_read_only(&core_db, CORE_APPLICATION_ID).expect("core marker");

    for (connection, expected) in [
        (&platform, PLATFORM_APPLICATION_ID),
        (&safe, SAFE_APPLICATION_ID),
        (&core, CORE_APPLICATION_ID),
    ] {
        let actual: i64 = connection
            .query_row("PRAGMA application_id", [], |row| row.get(0))
            .expect("read marker");
        assert_eq!(actual, expected);
    }

    let platform_tables: i64 = platform
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'core_registry'",
            [],
            |row| row.get(0),
        )
        .expect("platform table count");
    assert_eq!(platform_tables, 1);

    for connection in [&safe, &core] {
        let tables: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'core_registry'",
                [],
                |row| row.get(0),
            )
            .expect("table count");
        assert_eq!(tables, 0);
    }
}

#[test]
fn reactivate_suspended_core_returns_active_and_preserves_data() {
    let (_temp, paths) = temp_paths();
    let registry = registry(&paths);
    let entry = registry
        .register(request(CORE_A, 1, "SofA-a", &core_path(CORE_A)))
        .expect("register");

    let data_file = paths.root().join(&entry.storage_relative_path);
    std::fs::create_dir_all(data_file.parent().expect("parent")).expect("core dir");
    std::fs::write(&data_file, b"core-owned-data").expect("core data");
    let manifest = paths
        .root()
        .join("cores")
        .join(CORE_A)
        .join("manifest.json");
    std::fs::write(&manifest, b"core-manifest").expect("manifest");

    registry
        .deactivate(CORE_A, "scheduled maintenance")
        .expect("deactivate");

    let reactivated = registry
        .reactivate(CORE_A, "maintenance complete")
        .expect("reactivate");
    assert_eq!(reactivated.lifecycle_state, CoreLifecycleState::Active);
    assert_eq!(reactivated.core_uuid, entry.core_uuid);
    assert_eq!(
        reactivated.storage_relative_path,
        entry.storage_relative_path
    );
    assert_eq!(
        std::fs::read(&data_file).expect("data still present"),
        b"core-owned-data"
    );
    assert_eq!(
        std::fs::read(&manifest).expect("manifest still present"),
        b"core-manifest"
    );

    let history = registry.lifecycle_history(CORE_A).expect("history");
    assert_eq!(history.len(), 2);
    assert_eq!(history[1].from, CoreLifecycleState::Suspended);
    assert_eq!(history[1].to, CoreLifecycleState::Active);
    assert_eq!(history[1].reason, "maintenance complete");
}

#[test]
fn reactivate_installed_or_retired_core_is_rejected() {
    let (_temp, paths) = temp_paths();
    let registry = registry(&paths);
    registry
        .register(request(CORE_A, 1, "SofA-a", &core_path(CORE_A)))
        .expect("register A");
    registry
        .register(request(CORE_B, 6, "Ada", &core_path(CORE_B)))
        .expect("register B");

    let installed = registry
        .reactivate(CORE_A, "premature")
        .expect_err("installed cannot reactivate");
    assert!(matches!(
        installed,
        RegistryError::IllegalTransition { ref from, ref to }
            if from == "installed" && to == "active"
    ));
    assert_eq!(
        registry
            .get(CORE_A)
            .expect("get A")
            .expect("present")
            .lifecycle_state,
        CoreLifecycleState::Installed
    );
    assert!(registry
        .lifecycle_history(CORE_A)
        .expect("history")
        .is_empty());

    registry
        .uninstall_preserve(CORE_B, "retired by owner")
        .expect("retire B");
    let retired = registry
        .reactivate(CORE_B, "not allowed")
        .expect_err("retired cannot reactivate");
    assert!(matches!(
        retired,
        RegistryError::IllegalTransition { ref from, ref to }
            if from == "retired" && to == "active"
    ));
    assert_eq!(
        registry
            .get(CORE_B)
            .expect("get B")
            .expect("present")
            .lifecycle_state,
        CoreLifecycleState::Retired
    );
}

#[test]
fn uninstall_preserve_retires_and_leaves_core_files_untouched() {
    let (_temp, paths) = temp_paths();
    let registry = registry(&paths);
    let entry = registry
        .register(request(CORE_A, 1, "SofA-a", &core_path(CORE_A)))
        .expect("register");

    let core_dir = paths.root().join("cores").join(CORE_A);
    std::fs::create_dir_all(&core_dir).expect("core dir");
    let database = paths.root().join(&entry.storage_relative_path);
    std::fs::write(&database, b"sentinel-core-database-bytes").expect("core db");
    let manifest = core_dir.join("manifest.json");
    std::fs::write(&manifest, b"sentinel-manifest").expect("manifest");

    let retired = registry
        .uninstall_preserve(CORE_A, "owner uninstall")
        .expect("retire");
    assert_eq!(retired.lifecycle_state, CoreLifecycleState::Retired);
    assert_eq!(retired.storage_relative_path, entry.storage_relative_path);
    assert!(core_dir.is_dir());
    assert_eq!(
        std::fs::read(&database).expect("database untouched"),
        b"sentinel-core-database-bytes"
    );
    assert_eq!(
        std::fs::read(&manifest).expect("manifest untouched"),
        b"sentinel-manifest"
    );

    let history = registry.lifecycle_history(CORE_A).expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].from, CoreLifecycleState::Installed);
    assert_eq!(history[0].to, CoreLifecycleState::Retired);
    assert_eq!(history[0].reason, "owner uninstall");

    let repeated = registry
        .uninstall_preserve(CORE_A, "again")
        .expect_err("retired cannot retire again");
    assert!(matches!(
        repeated,
        RegistryError::IllegalTransition { ref from, ref to }
            if from == "retired" && to == "retired"
    ));
    assert_eq!(
        std::fs::read(&database).expect("database still untouched"),
        b"sentinel-core-database-bytes"
    );
}

#[test]
fn uninstall_preserve_accepts_installed_suspended_and_active() {
    let (_temp, paths) = temp_paths();
    let registry = registry(&paths);
    registry
        .register(request(CORE_A, 1, "SofA-a", &core_path(CORE_A)))
        .expect("register A");
    registry
        .register(request(CORE_B, 6, "Ada", &core_path(CORE_B)))
        .expect("register B");
    registry
        .register(request(CORE_C, 7, "Lyra", &core_path(CORE_C)))
        .expect("register C");

    registry
        .deactivate(CORE_B, "suspend before retire")
        .expect("suspend B");
    {
        let connection = open_initialized(
            &paths.platform_db(),
            PLATFORM_APPLICATION_ID,
            "core_registry",
        )
        .expect("platform db");
        connection
            .execute(
                "UPDATE core_registry SET lifecycle_state = 'active' WHERE core_uuid = ?1",
                [CORE_C],
            )
            .expect("mark C active");
    }

    let retired_a = registry
        .uninstall_preserve(CORE_A, "retire installed")
        .expect("retire A");
    let retired_b = registry
        .uninstall_preserve(CORE_B, "retire suspended")
        .expect("retire B");
    let retired_c = registry
        .uninstall_preserve(CORE_C, "retire active")
        .expect("retire C");

    for (retired, expected_from) in [
        (retired_a, CoreLifecycleState::Installed),
        (retired_b, CoreLifecycleState::Suspended),
        (retired_c, CoreLifecycleState::Active),
    ] {
        assert_eq!(retired.lifecycle_state, CoreLifecycleState::Retired);
        let history = registry
            .lifecycle_history(&retired.core_uuid)
            .expect("history");
        assert_eq!(history.last().expect("last change").from, expected_from);
        assert_eq!(
            history.last().expect("last change").to,
            CoreLifecycleState::Retired
        );
    }
}

#[test]
fn destructive_deletion_is_not_implemented() {
    let source = include_str!("../src/lib.rs");
    for forbidden in ["remove_file(", "remove_dir(", "remove_dir_all("] {
        assert!(
            !source.contains(forbidden),
            "registry must not implement destructive deletion: {forbidden}"
        );
    }
}

#[test]
fn lifecycle_failure_is_isolated_to_one_core() {
    let (_temp, paths) = temp_paths();
    let registry = registry(&paths);
    registry
        .register(request(CORE_A, 1, "SofA-a", &core_path(CORE_A)))
        .expect("register A");
    registry
        .register(request(CORE_B, 6, "Ada", &core_path(CORE_B)))
        .expect("register B");

    registry
        .deactivate(CORE_A, "maintenance")
        .expect("suspend A");
    let failure = registry
        .reactivate(CORE_B, "not suspended")
        .expect_err("installed cannot reactivate");
    assert!(matches!(failure, RegistryError::IllegalTransition { .. }));

    assert_eq!(
        registry
            .get(CORE_A)
            .expect("get A")
            .expect("present")
            .lifecycle_state,
        CoreLifecycleState::Suspended
    );
    assert_eq!(
        registry
            .get(CORE_B)
            .expect("get B")
            .expect("present")
            .lifecycle_state,
        CoreLifecycleState::Installed
    );
    assert_eq!(
        registry.lifecycle_history(CORE_A).expect("history A").len(),
        1
    );
    assert!(registry
        .lifecycle_history(CORE_B)
        .expect("history B")
        .is_empty());
}

#[test]
fn legacy_platform_core_rows_are_migrated_when_registry_is_empty() {
    let (_temp, paths) = temp_paths();
    {
        let legacy = initialize_legacy_platform(&paths);
        insert_legacy_core(&legacy, CORE_A, 1, "suspended", "2026-11-20T00:00:00Z");
        insert_legacy_core(&legacy, CORE_B, 6, "active", "2026-11-21T00:00:00Z");
    }

    let registry = CoreRegistry::new(paths.clone());
    let report = registry.reconcile().expect("reconcile");
    assert!(report.platform_core_present);
    assert!(report.registry_was_empty);
    assert_eq!(report.imported, 2);
    assert_eq!(report.skipped, 0);

    let migrated_a = registry.get(CORE_A).expect("get A").expect("migrated A");
    assert_eq!(migrated_a.schema, CORE_REGISTRY_SCHEMA);
    assert_eq!(migrated_a.core_number, 1);
    assert_eq!(migrated_a.display_name, "Legacy Core 1");
    assert_eq!(migrated_a.lifecycle_state, CoreLifecycleState::Suspended);
    assert_eq!(migrated_a.storage_relative_path, core_path(CORE_A));
    assert_eq!(migrated_a.initialized_at, "2026-11-20T00:00:00Z");
    assert_eq!(migrated_a.updated_at, migrated_a.initialized_at);

    let migrated_b = registry.get(CORE_B).expect("get B").expect("migrated B");
    assert_eq!(migrated_b.display_name, "Legacy Core 6");
    assert_eq!(migrated_b.lifecycle_state, CoreLifecycleState::Active);
    assert_eq!(migrated_b.storage_relative_path, core_path(CORE_B));
    assert_eq!(migrated_b.initialized_at, "2026-11-21T00:00:00Z");

    let again = registry
        .migrate_legacy_platform_core()
        .expect("reconcile again");
    assert!(again.platform_core_present);
    assert!(!again.registry_was_empty);
    assert_eq!(again.imported, 0);
    assert_eq!(again.skipped, 0);

    let connection = open_read_only(&paths.platform_db(), PLATFORM_APPLICATION_ID)
        .expect("legacy platform readable");
    let legacy_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM platform_core", [], |row| row.get(0))
        .expect("legacy count");
    assert_eq!(legacy_count, 2);
}

#[test]
fn reconcile_is_noop_when_registry_already_has_rows() {
    let (_temp, paths) = temp_paths();
    {
        let legacy = initialize_legacy_platform(&paths);
        insert_legacy_core(&legacy, CORE_A, 1, "installed", "2026-11-20T00:00:00Z");
    }

    let registry = registry(&paths);
    registry
        .register(request(CORE_C, 7, "Existing", &core_path(CORE_C)))
        .expect("register existing");

    let report = registry.reconcile().expect("reconcile");
    assert!(report.platform_core_present);
    assert!(!report.registry_was_empty);
    assert_eq!(report.imported, 0);
    assert_eq!(report.skipped, 0);
    assert!(registry.get(CORE_A).expect("get A").is_none());
    assert_eq!(registry.list().expect("list").len(), 1);
}

#[test]
fn reconcile_is_noop_without_legacy_platform_core_table() {
    let (_temp, paths) = temp_paths();
    let registry = registry(&paths);

    let report = registry.reconcile().expect("reconcile");
    assert!(!report.platform_core_present);
    assert!(report.registry_was_empty);
    assert_eq!(report.imported, 0);
    assert_eq!(report.skipped, 0);
    assert!(registry.list().expect("list").is_empty());
}

#[test]
fn legacy_migration_isolates_invalid_rows() {
    let (_temp, paths) = temp_paths();
    {
        let legacy = initialize_legacy_platform(&paths);
        insert_legacy_core(
            &legacy,
            "not-a-uuid",
            1,
            "installed",
            "2026-11-20T00:00:00Z",
        );
        insert_legacy_core(&legacy, CORE_A, 6, "installed", "2026-11-20T00:00:00Z");
        insert_legacy_core(&legacy, CORE_B, 6, "installed", "2026-11-20T00:00:00Z");
    }

    let registry = CoreRegistry::new(paths.clone());
    let report = registry.reconcile().expect("reconcile");
    assert_eq!(report.imported, 1);
    assert_eq!(report.skipped, 2);
    assert!(registry.get(CORE_A).expect("get A").is_some());
    assert!(registry.get(CORE_B).expect("get B").is_none());
    assert_eq!(registry.list().expect("list").len(), 1);

    let connection = open_read_only(&paths.platform_db(), PLATFORM_APPLICATION_ID)
        .expect("legacy platform readable");
    let legacy_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM platform_core", [], |row| row.get(0))
        .expect("legacy count");
    assert_eq!(legacy_count, 3);
}

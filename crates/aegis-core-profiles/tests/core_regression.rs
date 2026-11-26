use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_contracts::lifecycle::CoreLifecycleState;
use aegis_core_install::CoreInstaller;
use aegis_core_profiles::{profile, profiles, validate_profiles, IdentitySource, ProfileStatus};
use aegis_core_sdk::{CoreMemoryStore, ImportMode, MemoryKind, MemoryQuery, NewMemory};
use aegis_platform::PlatformService;
use aegis_storage::{now_rfc3339, StoragePaths};

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_B: &str = "b1b1b1b1-2222-4333-8444-555566667777";

fn request(core_uuid: &str, core_number: u8, display_name: &str) -> CoreInitializationRequest {
    CoreInitializationRequest {
        schema: CORE_INIT_REQUEST_SCHEMA.to_owned(),
        core_uuid: core_uuid.to_owned(),
        core_number,
        display_name: display_name.to_owned(),
        requested_at: now_rfc3339().expect("timestamp"),
    }
}

fn setup_cores() -> (tempfile::TempDir, std::path::PathBuf) {
    let temp = tempfile::tempdir().expect("tempdir");
    let initialized = PlatformService::initialize(temp.path()).expect("platform init");
    let installer = CoreInstaller::new(temp.path()).expect("installer");
    installer
        .initialize(&request(CORE_A, 2, "Mack Fuller"), &initialized.owner_token)
        .expect("install A");
    installer
        .initialize(&request(CORE_B, 6, "Vera"), &initialized.owner_token)
        .expect("install B");
    let root = temp.path().to_path_buf();
    (temp, root)
}

#[test]
fn approved_profile_identities_and_statuses_match_the_roster() {
    validate_profiles().expect("valid profiles");
    assert_eq!(profiles().len(), 7);

    let expected = [
        (
            1u8,
            "Sofía Ada Lyra",
            ProfileStatus::Approved,
            IdentitySource::PreservedLegacy,
        ),
        (
            2u8,
            "Mack Fuller",
            ProfileStatus::Approved,
            IdentitySource::ApprovedDefinition,
        ),
        (
            3u8,
            "Nova Arden",
            ProfileStatus::Approved,
            IdentitySource::ApprovedDefinition,
        ),
        (
            6u8,
            "Vera",
            ProfileStatus::Approved,
            IdentitySource::ApprovedExternalSpecification,
        ),
    ];
    for (number, name, status, source) in expected {
        let approved = profile(number).expect("approved profile");
        assert_eq!(approved.status, status);
        assert_eq!(approved.identity_source, source);
        assert!(approved.implementation_authorized);
        assert!(!approved.is_prohibited());
        assert_eq!(approved.display_name.as_deref(), Some(name));
    }

    for number in [4u8, 5u8] {
        let twin = profile(number).expect("twin profile");
        assert_eq!(twin.status, ProfileStatus::DesignOnly);
        assert_eq!(twin.identity_source, IdentitySource::Prohibited);
        assert!(twin.is_prohibited());
        assert!(twin.display_name.is_none());
    }

    let blank = profile(7).expect("blank profile");
    assert_eq!(blank.status, ProfileStatus::BlankFoundation);
    assert_eq!(blank.identity_source, IdentitySource::OwnerSupplied);
    assert!(blank.display_name.is_none());
    assert!(blank.identity.gender.is_none());
    assert!(blank.identity.species.is_none());
}

#[test]
fn two_core_memory_stores_cannot_read_each_other() {
    let (_temp, root) = setup_cores();

    let store_a = CoreMemoryStore::open(&root, CORE_A).expect("A memory store");
    let store_b = CoreMemoryStore::open(&root, CORE_B).expect("B memory store");
    assert_ne!(store_a.database_path(), store_b.database_path());

    let entry = store_a
        .create(NewMemory::new(MemoryKind::Fact, "Only Mack may read this"))
        .expect("create A memory");

    assert!(store_b.get(&entry.id).expect("B get").is_none());
    assert!(store_b
        .search(&MemoryQuery::new().with_text("Only Mack may read this"))
        .expect("B search")
        .is_empty());
    assert!(store_b.export().expect("B export").entries.is_empty());
    assert_eq!(
        store_a
            .search(&MemoryQuery::new().with_text("Only Mack may read this"))
            .expect("A search")
            .len(),
        1
    );

    let store_b_path = store_b.database_path().to_path_buf();
    drop(store_b);
    drop(store_a);

    let store_a_path = StoragePaths::new(&root)
        .expect("paths")
        .root()
        .join("cores")
        .join(CORE_A)
        .join("memory.db");
    assert_ne!(store_a_path, store_b_path);
    std::fs::copy(&store_a_path, &store_b_path).expect("copy A memory over B");

    let error = CoreMemoryStore::open(&root, CORE_B).expect_err("foreign ownership");
    assert!(matches!(error, aegis_core_sdk::SdkError::OwnershipMismatch));
}

#[test]
fn foreign_memory_database_error_does_not_leak_paths() {
    let (_temp, root) = setup_cores();
    let root_text = root.to_string_lossy().to_string();

    let store_a = CoreMemoryStore::open(&root, CORE_A).expect("A memory store");
    store_a
        .create(NewMemory::new(MemoryKind::Fact, "A only"))
        .expect("create A memory");
    let a_path = store_a.database_path().to_path_buf();
    drop(store_a);

    let store_b = CoreMemoryStore::open(&root, CORE_B).expect("B memory store");
    let b_path = store_b.database_path().to_path_buf();
    drop(store_b);

    assert_ne!(a_path, b_path);
    std::fs::copy(&a_path, &b_path).expect("copy A memory over B");

    let error = CoreMemoryStore::open(&root, CORE_B).expect_err("foreign ownership");
    assert!(matches!(error, aegis_core_sdk::SdkError::OwnershipMismatch));
    let text = error.to_string();
    assert!(!text.contains("memory.db"), "database file leaked: {text}");
    assert!(!text.contains(&root_text), "storage root leaked: {text}");
}

#[test]
fn cross_core_memory_import_is_rejected_without_leaking_data() {
    let (_temp, root) = setup_cores();
    let store_a = CoreMemoryStore::open(&root, CORE_A).expect("A memory store");
    let store_b = CoreMemoryStore::open(&root, CORE_B).expect("B memory store");

    let entry = store_a
        .create(NewMemory::new(MemoryKind::Fact, "Mack-only import payload"))
        .expect("create A memory");
    let export = store_a.export().expect("export A memory");

    let error = store_b
        .import(&export, ImportMode::RejectExisting)
        .expect_err("foreign import must be rejected");
    assert!(matches!(error, aegis_core_sdk::SdkError::ForeignCore));

    assert!(store_b.get(&entry.id).expect("B get").is_none());
    assert!(store_b.export().expect("B export").entries.is_empty());
    let intact = store_a.get(&entry.id).expect("A get").expect("entry");
    assert_eq!(intact.id, entry.id);
}

#[test]
fn one_core_cannot_mutate_another_cores_memory_entries() {
    let (_temp, root) = setup_cores();
    let store_a = CoreMemoryStore::open(&root, CORE_A).expect("A memory store");
    let store_b = CoreMemoryStore::open(&root, CORE_B).expect("B memory store");
    let entry = store_a
        .create(NewMemory::new(MemoryKind::Episode, "private episode"))
        .expect("create A memory");

    assert!(matches!(
        store_b.update_content(&entry.id, "hijacked"),
        Err(aegis_core_sdk::SdkError::NotFound)
    ));
    assert!(matches!(
        store_b.set_tags(&entry.id, vec!["hijack".to_owned()]),
        Err(aegis_core_sdk::SdkError::NotFound)
    ));
    assert!(matches!(
        store_b.archive(&entry.id),
        Err(aegis_core_sdk::SdkError::NotFound)
    ));
    assert!(matches!(
        store_b.soft_delete(&entry.id),
        Err(aegis_core_sdk::SdkError::NotFound)
    ));
    assert!(matches!(
        store_b.purge(&entry.id),
        Err(aegis_core_sdk::SdkError::NotFound)
    ));

    let intact = store_a.get(&entry.id).expect("A get").expect("entry");
    assert_eq!(intact.content, "private episode");
    assert!(!intact.archived);
    assert!(!intact.deleted);
    assert_eq!(intact.revision, 1);
    assert_eq!(store_a.export().expect("A export").entries.len(), 1);
}

#[test]
fn suspend_and_reactivate_preserve_core_memory_and_identity() {
    let temp = tempfile::tempdir().expect("tempdir");
    let initialized = PlatformService::initialize(temp.path()).expect("platform init");
    let installer = CoreInstaller::new(temp.path()).expect("installer");
    let installed = installer
        .initialize(&request(CORE_A, 2, "Mack Fuller"), &initialized.owner_token)
        .expect("install A");
    installer
        .initialize(&request(CORE_B, 6, "Vera"), &initialized.owner_token)
        .expect("install B");

    let entry = {
        let store = CoreMemoryStore::open(temp.path(), CORE_A).expect("A memory store");
        store
            .create(NewMemory::new(MemoryKind::Fact, "survives suspension"))
            .expect("create A memory")
    };

    let suspended = installer
        .deactivate(CORE_A, "maintenance", &initialized.owner_token)
        .expect("deactivate");
    assert_eq!(suspended.lifecycle_state, CoreLifecycleState::Suspended);

    {
        let during = CoreMemoryStore::open(temp.path(), CORE_A).expect("open during suspension");
        let memory = during.get(&entry.id).expect("get").expect("entry");
        assert_eq!(memory.content, "survives suspension");
    }

    let active = installer
        .reactivate(CORE_A, "maintenance complete", &initialized.owner_token)
        .expect("reactivate");
    assert_eq!(active.lifecycle_state, CoreLifecycleState::Active);

    let inspected = installer
        .inspect(CORE_A)
        .expect("inspect after reactivation");
    assert_eq!(inspected.identity, installed.identity);
    assert_eq!(
        inspected.metadata.database_instance_uuid,
        installed.metadata.database_instance_uuid
    );
    installer.verify(CORE_A).expect("verify after reactivation");

    let after = CoreMemoryStore::open(temp.path(), CORE_A).expect("A memory after");
    assert_eq!(
        after.get(&entry.id).expect("get").expect("entry").content,
        "survives suspension"
    );

    let store_b = CoreMemoryStore::open(temp.path(), CORE_B).expect("B memory store");
    assert!(store_b.get(&entry.id).expect("B get").is_none());
    assert!(store_b.export().expect("B export").entries.is_empty());
}

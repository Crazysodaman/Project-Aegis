use aegis_contracts::core::CoreContractError;
use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_contracts::lifecycle::CoreLifecycleState;
use aegis_core_identity::IdentityError;
use aegis_core_install::{CoreInstaller, InstallError};
use aegis_platform::PlatformService;
use aegis_storage::now_rfc3339;

const SOFIA: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const MACK: &str = "12345678-1234-4234-8234-123456789abc";
const VERA: &str = "b1b1b1b1-2222-4333-8444-555566667777";
const TWIN_FOUR: &str = "c2c2c2c2-3333-4444-8555-666677778888";
const TWIN_FIVE: &str = "d3d3d3d3-4444-4555-8666-777788889999";
const BLANK: &str = "e4e4e4e4-5555-4666-8777-88889999aaaa";

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

#[test]
fn two_installed_cores_have_separate_databases_and_instance_uuids() {
    let (_temp, installer, token) = setup();

    let mack = installer
        .initialize(&request(MACK, 2, "Mack Fuller"), &token)
        .expect("install Mack");
    let vera = installer
        .initialize(&request(VERA, 6, "Vera"), &token)
        .expect("install Vera");

    assert_eq!(mack.identity.core_number, 2);
    assert_eq!(mack.identity.display_name, "Mack Fuller");
    assert_eq!(vera.identity.core_number, 6);
    assert_eq!(vera.identity.display_name, "Vera");
    assert_eq!(mack.identity.core_uuid, MACK);
    assert_eq!(vera.identity.core_uuid, VERA);
    assert_eq!(
        mack.registry_entry.lifecycle_state,
        CoreLifecycleState::Installed
    );
    assert_eq!(
        vera.registry_entry.lifecycle_state,
        CoreLifecycleState::Installed
    );

    let mack_path = installer.layout().database_path(MACK).expect("mack path");
    let vera_path = installer.layout().database_path(VERA).expect("vera path");
    assert_ne!(mack_path, vera_path);
    assert_ne!(mack_path, installer.paths().platform_db());
    assert_ne!(vera_path, installer.paths().safe_db());
    assert!(mack_path.is_file());
    assert!(vera_path.is_file());

    let mack_instance = &mack.metadata.database_instance_uuid;
    let vera_instance = &vera.metadata.database_instance_uuid;
    assert_ne!(
        mack_instance, vera_instance,
        "each Core must have its own database instance UUID"
    );
    assert_ne!(mack_instance, MACK);
    assert_ne!(vera_instance, VERA);
    assert_ne!(mack_instance, &vera.identity.core_uuid);

    let mack_inspection = installer.inspect(MACK).expect("inspect Mack");
    let vera_inspection = installer.inspect(VERA).expect("inspect Vera");
    assert_eq!(mack_inspection.identity.core_uuid, MACK);
    assert_eq!(vera_inspection.identity.core_uuid, VERA);
    assert_eq!(mack_inspection.identity.display_name, "Mack Fuller");
    assert_eq!(vera_inspection.identity.display_name, "Vera");
    assert_ne!(
        mack_inspection.metadata.database_instance_uuid,
        vera_inspection.metadata.database_instance_uuid
    );
    installer.verify(MACK).expect("verify Mack");
    installer.verify(VERA).expect("verify Vera");
}

#[test]
fn prohibited_twins_cannot_be_installed() {
    let (_temp, installer, token) = setup();

    for (core_uuid, number) in [(TWIN_FOUR, 4u8), (TWIN_FIVE, 5u8)] {
        let error = installer
            .initialize(&request(core_uuid, number, "Twin"), &token)
            .expect_err("prohibited Core must be rejected");
        assert!(matches!(
            error,
            InstallError::Identity(IdentityError::Core(CoreContractError::ProhibitedCoreNumber))
        ));
        assert!(
            !installer
                .layout()
                .database_path(core_uuid)
                .expect("prohibited path")
                .exists(),
            "prohibited Core must not create a database"
        );
    }
}

#[test]
fn blank_core_requires_an_owner_supplied_name() {
    let (_temp, installer, token) = setup();

    let blank_error = installer
        .initialize(&request(BLANK, 7, "   "), &token)
        .expect_err("blank display name must be rejected");
    assert!(matches!(
        blank_error,
        InstallError::Identity(IdentityError::Core(CoreContractError::InvalidDisplayName))
    ));

    let installed = installer
        .initialize(&request(BLANK, 7, "Owner Chosen"), &token)
        .expect("owner-supplied name");
    assert_eq!(installed.identity.core_number, 7);
    assert_eq!(installed.identity.display_name, "Owner Chosen");
    assert_eq!(
        installed.registry_entry.lifecycle_state,
        CoreLifecycleState::Installed
    );
    installer.verify(BLANK).expect("verify blank Core");
}

#[test]
fn three_installed_cores_keep_distinct_stable_instances_across_reopen() {
    let (temp, installer, token) = setup();
    let sofia = installer
        .initialize(&request(SOFIA, 1, "Sofía Ada Lyra"), &token)
        .expect("install Sofía");
    let mack = installer
        .initialize(&request(MACK, 2, "Mack Fuller"), &token)
        .expect("install Mack");
    let vera = installer
        .initialize(&request(VERA, 6, "Vera"), &token)
        .expect("install Vera");

    let instances = [
        sofia.metadata.database_instance_uuid.clone(),
        mack.metadata.database_instance_uuid.clone(),
        vera.metadata.database_instance_uuid.clone(),
    ];
    assert_ne!(instances[0], instances[1]);
    assert_ne!(instances[0], instances[2]);
    assert_ne!(instances[1], instances[2]);
    for instance in &instances {
        assert_ne!(instance, SOFIA);
        assert_ne!(instance, MACK);
        assert_ne!(instance, VERA);
    }

    let paths = [
        installer.layout().database_path(SOFIA).expect("sofia path"),
        installer.layout().database_path(MACK).expect("mack path"),
        installer.layout().database_path(VERA).expect("vera path"),
    ];
    assert_ne!(paths[0], paths[1]);
    assert_ne!(paths[0], paths[2]);
    assert_ne!(paths[1], paths[2]);
    for path in &paths {
        assert!(path.is_file(), "installed Core database must exist");
    }

    drop(installer);
    let reopened = CoreInstaller::new(temp.path()).expect("reopened installer");
    for (core_uuid, expected) in [
        (SOFIA, &instances[0]),
        (MACK, &instances[1]),
        (VERA, &instances[2]),
    ] {
        let inspected = reopened.inspect(core_uuid).expect("inspect after reopen");
        assert_eq!(
            &inspected.metadata.database_instance_uuid, expected,
            "instance UUID must be stable across reopen"
        );
        reopened.verify(core_uuid).expect("verify after reopen");
    }
}

#[test]
fn suspend_and_reactivate_preserve_identity_and_record_history() {
    let (_temp, installer, token) = setup();
    let installed = installer
        .initialize(&request(MACK, 2, "Mack Fuller"), &token)
        .expect("install Mack");
    let database_path = installer.layout().database_path(MACK).expect("mack path");
    let before = std::fs::read(&database_path).expect("database bytes before lifecycle");

    let suspended = installer
        .deactivate(MACK, "scheduled maintenance", &token)
        .expect("deactivate");
    assert_eq!(suspended.lifecycle_state, CoreLifecycleState::Suspended);
    let active = installer
        .reactivate(MACK, "maintenance complete", &token)
        .expect("reactivate");
    assert_eq!(active.lifecycle_state, CoreLifecycleState::Active);

    let registry = aegis_core_registry::CoreRegistry::new(installer.paths().clone());
    let history = registry.lifecycle_history(MACK).expect("lifecycle history");
    assert_eq!(history.len(), 2, "suspend and reactivate must be recorded");
    assert_eq!(history[0].from, CoreLifecycleState::Installed);
    assert_eq!(history[0].to, CoreLifecycleState::Suspended);
    assert_eq!(history[1].from, CoreLifecycleState::Suspended);
    assert_eq!(history[1].to, CoreLifecycleState::Active);

    let inspected = installer.inspect(MACK).expect("inspect after reactivation");
    assert_eq!(inspected.identity, installed.identity);
    assert_eq!(
        inspected.metadata.database_instance_uuid,
        installed.metadata.database_instance_uuid
    );
    assert_eq!(
        std::fs::read(&database_path).expect("database bytes after lifecycle"),
        before,
        "lifecycle transitions must not mutate the Core database"
    );
    installer.verify(MACK).expect("verify after reactivation");
}

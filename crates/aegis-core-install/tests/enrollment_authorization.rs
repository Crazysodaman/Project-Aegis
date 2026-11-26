//! Enrollment/authorization trust tests for Core installation (Final / Agent 16).
//!
//! These complement the existing installation and lifecycle suites by proving
//! that every lifecycle mutation is owner-gated, that prohibited Core numbers
//! leave no enrollment trace, and that a missing owner proof blocks enrollment.

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_contracts::lifecycle::CoreLifecycleState;
use aegis_core_install::{CoreInstaller, InstallError};
use aegis_core_integrity::compute_file_sha256;
use aegis_core_registry::CoreRegistry;
use aegis_platform::PlatformService;
use aegis_safe::SafeError;
use aegis_storage::now_rfc3339;

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const TWIN_FOUR: &str = "c2c2c2c2-3333-4444-8555-666677778888";
const TWIN_FIVE: &str = "d3d3d3d3-4444-4555-8666-777788889999";

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
fn lifecycle_mutations_require_owner_and_leave_state_untouched() {
    let (_temp, installer, token) = setup();
    let installed = installer
        .initialize(&request(CORE_A, 1, "Core A"), &token)
        .expect("install Core");
    let database_path = installer.layout().database_path(CORE_A).expect("db path");
    let manifest_path = installer.layout().integrity_path(CORE_A).expect("manifest");
    let database_digest = compute_file_sha256(&database_path).expect("db digest");
    let manifest_digest = compute_file_sha256(&manifest_path).expect("manifest digest");

    let prefix = &token[..token.len() - 1];
    for candidate in ["", "wrong-token", prefix] {
        assert!(matches!(
            installer.rename_display_name(CORE_A, "Hijacked", candidate),
            Err(InstallError::Unauthorized)
        ));
        assert!(matches!(
            installer.deactivate(CORE_A, "hijack", candidate),
            Err(InstallError::Unauthorized)
        ));
        assert!(matches!(
            installer.reactivate(CORE_A, "hijack", candidate),
            Err(InstallError::Unauthorized)
        ));
        assert!(matches!(
            installer.uninstall_preserve(CORE_A, "hijack", candidate),
            Err(InstallError::Unauthorized)
        ));
    }

    let entry = installer
        .get(CORE_A)
        .expect("get Core")
        .expect("registered Core");
    assert_eq!(entry.lifecycle_state, CoreLifecycleState::Installed);
    assert_eq!(entry.display_name, installed.registry_entry.display_name);
    assert_eq!(entry.core_number, 1);
    assert_eq!(
        compute_file_sha256(&database_path).expect("db digest after"),
        database_digest,
        "rejected lifecycle mutations must not touch the database"
    );
    assert_eq!(
        compute_file_sha256(&manifest_path).expect("manifest digest after"),
        manifest_digest
    );
    assert_eq!(
        installer.verify(CORE_A).expect("verify").status.as_str(),
        "verified"
    );

    let registry = CoreRegistry::new(installer.paths().clone());
    assert!(
        registry
            .lifecycle_history(CORE_A)
            .expect("lifecycle history")
            .is_empty(),
        "rejected mutations must not record lifecycle history"
    );
}

#[test]
fn prohibited_cores_cannot_enter_enrollment_and_leave_no_trace() {
    let (_temp, installer, token) = setup();

    for (core_uuid, number) in [(TWIN_FOUR, 4u8), (TWIN_FIVE, 5u8)] {
        let error = installer
            .initialize(&request(core_uuid, number, "Twin"), &token)
            .expect_err("prohibited Core must be rejected");
        assert!(matches!(
            error,
            InstallError::Identity(aegis_core_identity::IdentityError::Core(
                aegis_contracts::core::CoreContractError::ProhibitedCoreNumber
            ))
        ));

        assert!(
            !installer
                .layout()
                .database_path(core_uuid)
                .expect("prohibited db path")
                .exists(),
            "prohibited Core must not create a database"
        );
        assert!(
            !installer
                .layout()
                .initialization_marker_path(core_uuid)
                .expect("prohibited marker path")
                .exists(),
            "prohibited Core must not create an initialization marker"
        );
        assert!(
            installer.get(core_uuid).expect("get prohibited").is_none(),
            "prohibited Core must not be registered"
        );
    }

    assert!(installer.list().expect("list").is_empty());
}

#[test]
fn missing_owner_proof_blocks_enrollment_without_leaking_paths() {
    let (_temp, installer, _token) = setup();
    let paths = installer.paths().clone();
    let root = paths.root().to_string_lossy().to_string();
    std::fs::remove_file(paths.owner_proof()).expect("remove owner proof");

    let error = installer
        .initialize(&request(CORE_A, 1, "Core A"), "any-token")
        .expect_err("enrollment without owner proof must fail");
    assert!(matches!(
        error,
        InstallError::Safe(SafeError::OwnerNotInitialized)
    ));
    let text = error.to_string();
    assert!(!text.contains("owner.proof"), "proof file leaked: {text}");
    assert!(!text.contains("safe.db"), "database file leaked: {text}");
    assert!(!text.contains(&root), "storage root leaked: {text}");
    assert!(!installer
        .layout()
        .database_path(CORE_A)
        .expect("db path")
        .exists());
    assert!(installer.get(CORE_A).expect("get Core").is_none());
}

//! Phase 1 identity invariants enforced at the installation boundary.
//!
//! The canonical Core #1 UUID and the prohibition on Cores #4/#5, plus the
//! owner-supplied-name requirement for Core #7, must be enforced before any
//! database, marker, or registry write.

use aegis_contracts::core::CoreContractError;
use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_core_identity::IdentityError;
use aegis_core_install::{CoreInstaller, InstallError};
use aegis_platform::PlatformService;
use aegis_roster::{RosterError, SOFIA_CANONICAL_UUID};
use aegis_storage::now_rfc3339;

const WRONG_UUID_FOR_ONE: &str = "11111111-1111-4111-8111-111111111111";
const TWIN_FOUR: &str = "c2c2c2c2-3333-4444-8555-666677778888";
const TWIN_FIVE: &str = "d3d3d3d3-4444-4555-8666-777788889999";
const BLANK_UUID: &str = "e4e4e4e4-5555-4666-8777-88889999aaaa";

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

fn assert_no_core_files(installer: &CoreInstaller, core_uuid: &str) {
    assert!(
        !installer
            .layout()
            .database_path(core_uuid)
            .expect("database path")
            .exists(),
        "rejected initialization must not create a database"
    );
    assert!(
        !installer
            .layout()
            .initialization_marker_path(core_uuid)
            .expect("marker path")
            .exists(),
        "rejected initialization must not create an initialization marker"
    );
    assert!(
        installer.get(core_uuid).expect("get Core").is_none(),
        "rejected initialization must not register the Core"
    );
}

#[test]
fn wrong_core_one_uuid_is_rejected_with_no_side_effects() {
    let (_temp, installer, token) = setup();

    let error = installer
        .initialize_approved(&request(WRONG_UUID_FOR_ONE, 1, "Sofía Ada Lyra"), &token)
        .expect_err("wrong Core #1 UUID must be rejected");
    assert!(matches!(
        error,
        InstallError::Roster(RosterError::CanonicalUuidMismatch(1))
    ));

    assert_no_core_files(&installer, WRONG_UUID_FOR_ONE);
    assert_no_core_files(&installer, SOFIA_CANONICAL_UUID);
    assert!(installer.list().expect("list").is_empty());
}

#[test]
fn correct_core_one_uuid_is_accepted() {
    let (_temp, installer, token) = setup();

    let installed = installer
        .initialize_approved(&request(SOFIA_CANONICAL_UUID, 1, "Sofía Ada Lyra"), &token)
        .expect("canonical Core #1 UUID must be accepted");
    assert_eq!(installed.identity.core_uuid, SOFIA_CANONICAL_UUID);
    assert_eq!(installed.identity.core_number, 1);
    assert_eq!(
        installer
            .verify(SOFIA_CANONICAL_UUID)
            .expect("verify")
            .status
            .as_str(),
        "verified"
    );
}

#[test]
fn prohibited_twins_are_rejected_with_no_side_effects() {
    let (_temp, installer, token) = setup();

    for (core_uuid, number) in [(TWIN_FOUR, 4u8), (TWIN_FIVE, 5u8)] {
        let error = installer
            .initialize_approved(&request(core_uuid, number, "Twin"), &token)
            .expect_err("prohibited Core must be rejected");
        assert!(matches!(
            error,
            InstallError::Identity(IdentityError::Core(CoreContractError::ProhibitedCoreNumber))
        ));
        assert_no_core_files(&installer, core_uuid);
    }
    assert!(installer.list().expect("list").is_empty());
}

#[test]
fn blank_core_seven_name_is_rejected_with_no_side_effects() {
    let (_temp, installer, token) = setup();

    let error = installer
        .initialize_approved(&request(BLANK_UUID, 7, "   "), &token)
        .expect_err("blank owner-supplied name must be rejected");
    assert!(matches!(
        error,
        InstallError::Identity(IdentityError::Core(CoreContractError::InvalidDisplayName))
    ));
    assert_no_core_files(&installer, BLANK_UUID);
    assert!(installer.list().expect("list").is_empty());
}

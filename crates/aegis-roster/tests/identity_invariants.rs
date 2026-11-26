//! Phase 1 identity and Constitution invariants for the approved roster.
//!
//! Core #1 carries the only owner-supplied canonical UUID and Constitution
//! digest. Cores #2, #3, and #6 deliberately leave those values unknown rather
//! than inventing them. These tests lock in the catalog data and the
//! pre-install identity gate.

use aegis_roster::{
    entry, validate_core_identity, validate_roster, RosterError, SOFIA_CANONICAL_UUID,
    SOFIA_CONSTITUTION_SHA256,
};

const OTHER_UUID: &str = "11111111-1111-4111-8111-111111111111";

#[test]
fn catalog_carries_only_authoritative_sofia_identity() {
    validate_roster().expect("roster must be valid");

    let sofia = entry(1).expect("core 1");
    assert_eq!(sofia.canonical_uuid, Some(SOFIA_CANONICAL_UUID));
    assert_eq!(sofia.constitution_sha256, Some(SOFIA_CONSTITUTION_SHA256));

    for number in [2u8, 3, 6] {
        let design = entry(number).expect("approved Core");
        assert_eq!(
            design.canonical_uuid, None,
            "Core #{number} canonical UUID is unknown and must not be invented"
        );
        assert_eq!(
            design.constitution_sha256, None,
            "Core #{number} Constitution digest is unknown and must not be invented"
        );
    }

    for number in [4u8, 5, 7] {
        let design = entry(number).expect("Core");
        assert_eq!(design.canonical_uuid, None);
        assert_eq!(design.constitution_sha256, None);
    }
}

#[test]
fn core_one_accepts_its_canonical_uuid_and_returns_the_canonical_name() {
    assert_eq!(
        validate_core_identity(1, SOFIA_CANONICAL_UUID, None).expect("canonical"),
        "Sofía Ada Lyra"
    );
    assert_eq!(
        validate_core_identity(1, SOFIA_CANONICAL_UUID, Some("Sofía Ada Lyra"))
            .expect("approved name"),
        "Sofía Ada Lyra"
    );
    assert_eq!(
        validate_core_identity(1, &SOFIA_CANONICAL_UUID.to_uppercase(), None)
            .expect("uppercase canonical UUID"),
        "Sofía Ada Lyra"
    );
}

#[test]
fn core_one_rejects_any_non_canonical_uuid() {
    assert!(matches!(
        validate_core_identity(1, OTHER_UUID, None),
        Err(RosterError::CanonicalUuidMismatch(1))
    ));
}

#[test]
fn malformed_uuid_is_rejected() {
    for candidate in ["", "not-a-uuid", "a9a0a604a4e74a88a9853f071e2c8e7d", "zzzz"] {
        assert!(
            matches!(
                validate_core_identity(2, candidate, None),
                Err(RosterError::InvalidCoreUuid)
            ),
            "candidate {candidate:?} must be rejected as an invalid UUID"
        );
    }
    assert!(matches!(
        validate_core_identity(2, "00000000-0000-0000-0000-000000000000", None),
        Err(RosterError::InvalidCoreUuid)
    ));
}

#[test]
fn prohibited_twins_are_rejected_before_uuid_checks() {
    for number in [4u8, 5u8] {
        assert!(matches!(
            validate_core_identity(number, OTHER_UUID, Some("Twin")),
            Err(RosterError::ProhibitedCore(n)) if n == number
        ));
    }
}

#[test]
fn blank_core_requires_an_owner_supplied_name() {
    assert!(matches!(
        validate_core_identity(7, OTHER_UUID, None),
        Err(RosterError::DisplayNameRequired(7))
    ));
    assert!(matches!(
        validate_core_identity(7, OTHER_UUID, Some("   ")),
        Err(RosterError::DisplayNameRequired(7))
    ));
    assert_eq!(
        validate_core_identity(7, OTHER_UUID, Some("  Owner Chosen  "))
            .expect("owner-supplied name"),
        "Owner Chosen"
    );
}

#[test]
fn cores_without_canonical_uuid_accept_any_valid_uuid() {
    for number in [2u8, 3, 6] {
        let name = validate_core_identity(number, OTHER_UUID, None).expect("no canonical UUID");
        assert!(!name.is_empty());
    }
}

#[test]
fn unknown_core_numbers_are_rejected() {
    assert!(matches!(
        validate_core_identity(8, OTHER_UUID, Some("X")),
        Err(RosterError::UnknownCoreNumber(8))
    ));
}

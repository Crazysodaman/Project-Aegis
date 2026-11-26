//! Approved Core roster policy tests (Batch 5 / Agent 10).
//!
//! The roster is data, not authority. These tests lock in the owner-approved
//! expectations: #4/#5 are prohibited design-only twins and #7 is a blank
//! identity whose display name must be supplied by the owner.

use aegis_roster::{
    entry, resolve_display_name, roster, validate_roster, RosterDesignStatus, RosterError,
};

#[test]
fn prohibited_twins_are_design_only_and_cannot_install() {
    for number in [4u8, 5u8] {
        let design = entry(number).expect("twin entry");
        assert_eq!(design.status, RosterDesignStatus::Prohibited);
        assert!(!design.identity_defined);
        assert!(!design.implementation_authorized);
        assert!(!design.can_install());
        assert!(design.default_display_name.is_none());

        assert!(matches!(
            resolve_display_name(number, None),
            Err(RosterError::ProhibitedCore(n)) if n == number
        ));
        assert!(matches!(
            resolve_display_name(number, Some("Twin")),
            Err(RosterError::ProhibitedCore(n)) if n == number
        ));
    }
}

#[test]
fn blank_core_requires_owner_supplied_name_without_invented_identity() {
    let blank = entry(7).expect("core 7");
    assert_eq!(blank.status, RosterDesignStatus::Implementable);
    assert!(!blank.identity_defined);
    assert!(blank.implementation_authorized);
    assert!(blank.can_install());
    assert!(blank.default_display_name.is_none());

    assert!(matches!(
        resolve_display_name(7, None),
        Err(RosterError::DisplayNameRequired(7))
    ));
    assert!(matches!(
        resolve_display_name(7, Some("   ")),
        Err(RosterError::DisplayNameRequired(7))
    ));
    assert_eq!(
        resolve_display_name(7, Some("Owner Chosen")).expect("owner-supplied name"),
        "Owner Chosen"
    );
    assert_eq!(
        resolve_display_name(7, Some("  Owner Chosen  ")).expect("trimmed owner name"),
        "Owner Chosen"
    );
}

#[test]
fn catalog_matches_expected_roster_identities() {
    validate_roster().expect("valid roster");

    let numbers: Vec<u8> = roster().iter().map(|design| design.core_number).collect();
    assert_eq!(numbers, vec![1, 2, 3, 4, 5, 6, 7]);

    assert_eq!(
        entry(1).expect("core 1").default_display_name.as_deref(),
        Some("Sofía Ada Lyra")
    );
    assert!(
        entry(1).expect("core 1").preserve_legacy_identity,
        "Core #1 must preserve the legacy Sofía identity"
    );
    assert_eq!(
        entry(2).expect("core 2").default_display_name.as_deref(),
        Some("Mack Fuller")
    );
    assert_eq!(
        entry(3).expect("core 3").default_display_name.as_deref(),
        Some("Nova Arden")
    );
    assert_eq!(
        entry(6).expect("core 6").default_display_name.as_deref(),
        Some("Vera")
    );

    for number in [1u8, 2, 3, 6] {
        assert!(
            entry(number).expect("approved Core").can_install(),
            "Core #{number} must be installable"
        );
    }
}

#[test]
fn unknown_core_numbers_are_rejected() {
    for number in [0u8, 8, 9, 255] {
        assert!(entry(number).is_none());
        assert!(matches!(
            resolve_display_name(number, Some("X")),
            Err(RosterError::UnknownCoreNumber(n)) if n == number
        ));
    }
}

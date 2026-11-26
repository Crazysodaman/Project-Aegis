use aegis_roster::{
    entry, resolve_display_name, roster, validate_roster, RosterDesignStatus, RosterError,
};

#[test]
fn approved_roster_identities_and_statuses_are_locked() {
    validate_roster().expect("valid roster");
    assert_eq!(roster().len(), 7);

    let expected = [
        (1u8, "Sofía Ada Lyra", true),
        (2u8, "Mack Fuller", false),
        (3u8, "Nova Arden", false),
        (6u8, "Vera", false),
    ];
    for (number, name, preserve_legacy) in expected {
        let design = entry(number).expect("approved Core");
        assert_eq!(design.status, RosterDesignStatus::Implementable);
        assert!(design.identity_defined);
        assert!(design.implementation_authorized);
        assert!(design.can_install());
        assert_eq!(design.default_display_name.as_deref(), Some(name));
        assert_eq!(design.preserve_legacy_identity, preserve_legacy);
        assert_eq!(
            resolve_display_name(number, None).expect("catalog name"),
            name
        );
    }

    assert_eq!(
        entry(6).expect("vera").designation,
        "Vera / S-039",
        "S-039 is the catalog designation and Vera the display name"
    );
}

#[test]
fn design_only_twins_remain_prohibited() {
    for number in [4u8, 5u8] {
        let design = entry(number).expect("twin");
        assert_eq!(design.status, RosterDesignStatus::Prohibited);
        assert!(!design.identity_defined);
        assert!(!design.implementation_authorized);
        assert!(!design.can_install());
        assert!(design.default_display_name.is_none());
        assert!(!design.preserve_legacy_identity);
    }
}

#[test]
fn core_seven_stays_blank_and_unwritten() {
    let blank = entry(7).expect("blank Core");
    assert_eq!(blank.status, RosterDesignStatus::Implementable);
    assert_eq!(blank.designation, "Unwritten");
    assert!(!blank.identity_defined);
    assert!(blank.implementation_authorized);
    assert!(blank.can_install());
    assert!(blank.default_display_name.is_none());
    assert!(!blank.preserve_legacy_identity);
}

#[test]
fn approved_cores_reject_impostor_names_and_keep_catalog_designations() {
    let expected = [
        (1u8, "Sofía Ada Lyra", "Sofía Ada Lyra"),
        (2u8, "Mack Fuller", "Mack Fuller"),
        (3u8, "Nova Arden", "Nova Arden"),
        (6u8, "Vera", "Vera / S-039"),
    ];
    for (number, approved_name, designation) in expected {
        assert_eq!(
            resolve_display_name(number, Some(approved_name)).expect("approved name"),
            approved_name
        );
        assert!(matches!(
            resolve_display_name(number, Some("Impostor")),
            Err(RosterError::DisplayNameMismatch(n, ref name))
                if n == number && name == approved_name
        ));
        assert_eq!(
            entry(number).expect("approved entry").designation,
            designation
        );
    }
}

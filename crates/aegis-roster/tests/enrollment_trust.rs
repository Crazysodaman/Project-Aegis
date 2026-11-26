//! Enrollment/install trust invariants for the approved roster (Final / Agent 16).
//!
//! The roster is policy data, not an authority. These tests prove that hostile
//! or impersonating display-name requests cannot be reflected back through
//! errors and that only the owner-approved Core numbers can be resolved for
//! installation.

use aegis_roster::{entry, resolve_display_name, validate_roster, RosterDesignStatus, RosterError};

#[test]
fn hostile_requested_names_are_not_reflected_in_errors() {
    let hostile = "Impostor/../../SECRET-OWNER-TOKEN-0123456789";

    for number in [1u8, 2, 3, 6] {
        let approved = entry(number)
            .expect("approved Core")
            .default_display_name
            .clone()
            .expect("approved name");
        let error = resolve_display_name(number, Some(hostile))
            .expect_err("impostor name must be rejected");
        assert!(
            matches!(
                &error,
                RosterError::DisplayNameMismatch(n, name)
                    if *n == number && name == &approved
            ),
            "unexpected error: {error:?}"
        );
        let text = error.to_string();
        assert!(!text.contains(hostile), "candidate leaked: {text}");
        assert!(!text.contains("SECRET-OWNER-TOKEN"));
        assert!(
            text.contains(&approved),
            "error should name the approved identity, not the input: {text}"
        );
    }

    for number in [4u8, 5u8] {
        let error = resolve_display_name(number, Some(hostile))
            .expect_err("prohibited Core must be rejected");
        assert!(matches!(error, RosterError::ProhibitedCore(n) if n == number));
        assert!(!error.to_string().contains(hostile));
    }

    let control = "bad\u{0007}name";
    let error = resolve_display_name(7, Some(control)).expect_err("control characters rejected");
    assert!(matches!(error, RosterError::InvalidDisplayName));
    assert!(!error.to_string().contains(hostile));
}

#[test]
fn only_approved_core_numbers_can_be_resolved_for_install() {
    validate_roster().expect("valid roster");

    let installable = [1u8, 2, 3, 6, 7];
    let prohibited = [4u8, 5];

    for number in 0u8..=255 {
        let can_install = entry(number).map(|design| design.can_install());
        let allowed = installable.contains(&number);
        assert_eq!(
            can_install,
            if entry(number).is_some() {
                Some(allowed)
            } else {
                None
            },
            "unexpected install authorization for Core #{number}"
        );

        if allowed {
            continue;
        }
        let error = resolve_display_name(number, Some("Anything"))
            .expect_err("non-installable Core must be rejected");
        match (entry(number), prohibited.contains(&number)) {
            (Some(_), true) => {
                assert!(matches!(error, RosterError::ProhibitedCore(n) if n == number))
            }
            (Some(design), false) => {
                assert_eq!(design.status, RosterDesignStatus::DesignOnly);
                assert!(matches!(error, RosterError::DesignOnlyCore(n) if n == number));
            }
            (None, _) => assert!(matches!(error, RosterError::UnknownCoreNumber(n) if n == number)),
        }
    }
}

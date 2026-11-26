//! Phase 1 identity invariants shared between profiles and the approved roster.
//!
//! Profiles describe approved identity attributes. They must not invent the
//! canonical UUID or Constitution digest for any Core. Only Core #1 carries the
//! owner-supplied authoritative values, which live in the roster.

use aegis_core_profiles::{profile, validate_profiles};
use aegis_roster::{entry, SOFIA_CANONICAL_UUID, SOFIA_CONSTITUTION_SHA256};

#[test]
fn profiles_do_not_invent_canonical_identity() {
    validate_profiles().expect("profiles must be valid");

    let sofia = entry(1).expect("core 1 roster entry");
    assert_eq!(sofia.canonical_uuid, Some(SOFIA_CANONICAL_UUID));
    assert_eq!(sofia.constitution_sha256, Some(SOFIA_CONSTITUTION_SHA256));

    for number in [2u8, 3, 6] {
        let roster = entry(number).expect("roster entry");
        let profile = profile(number).expect("profile");
        assert_eq!(roster.canonical_uuid, None);
        assert_eq!(roster.constitution_sha256, None);
        assert_eq!(
            profile.display_name.as_deref(),
            roster.default_display_name.as_deref()
        );
    }
}

#[test]
fn prohibited_and_blank_profiles_stay_unwritten() {
    for number in [4u8, 5u8] {
        let roster = entry(number).expect("twin roster entry");
        let profile = profile(number).expect("twin profile");
        assert_eq!(roster.canonical_uuid, None);
        assert_eq!(roster.constitution_sha256, None);
        assert!(profile.is_prohibited());
    }

    let blank_roster = entry(7).expect("blank roster entry");
    let blank_profile = profile(7).expect("blank profile");
    assert_eq!(blank_roster.canonical_uuid, None);
    assert_eq!(blank_roster.constitution_sha256, None);
    assert!(blank_roster.default_display_name.is_none());
    assert!(blank_profile.display_name.is_none());
}

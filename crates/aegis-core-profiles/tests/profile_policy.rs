//! Approved Core profile policy tests (Batch 5 / Agent 10).
//!
//! Profiles must not invent identity for prohibited or blank Cores. These tests
//! mirror the roster expectations for #4/#5 and #7 at the profile layer.

use aegis_core_profiles::{
    profile, profiles, resolve_display_name, validate_profiles, IdentitySource, ProfileError,
    ProfileStatus,
};

const CORE_UUID: &str = "11111111-1111-4111-8111-111111111111";

#[test]
fn prohibited_twins_have_no_invented_identity_or_personality() {
    for number in [4u8, 5u8] {
        let twin = profile(number).expect("twin profile");
        assert!(twin.is_prohibited());
        assert_eq!(twin.status, ProfileStatus::DesignOnly);
        assert_eq!(twin.identity_source, IdentitySource::Prohibited);
        assert!(!twin.implementation_authorized);
        assert!(twin.display_name.is_none());

        assert!(twin.identity.gender.is_none());
        assert!(twin.identity.age_years.is_none());
        assert!(twin.identity.species.is_none());
        assert!(twin.personality.traits.is_empty());
        assert!(twin.personality.values.is_empty());
        assert!(twin.personality.flaws.is_empty());
        assert!(twin.appearance.hair.is_none());
        assert!(twin.appearance.eyes.is_none());
        assert!(twin.appearance.build.is_none());
        assert!(twin.appearance.aesthetic.is_none());
        assert!(twin.appearance.wardrobe_tags.is_empty());
        assert!(twin.voice.profile_id.is_none());
        assert!(twin.voice.description.is_none());

        assert!(twin
            .personality_for(CORE_UUID)
            .expect("personality lookup")
            .is_none());
        assert!(matches!(
            resolve_display_name(number, Some("Twin")),
            Err(ProfileError::ProhibitedCore(n)) if n == number
        ));
    }
}

#[test]
fn blank_core_identity_stays_uninvented_and_needs_owner_name() {
    let blank = profile(7).expect("blank profile");
    assert_eq!(blank.status, ProfileStatus::BlankFoundation);
    assert_eq!(blank.identity_source, IdentitySource::OwnerSupplied);
    assert!(blank.implementation_authorized);
    assert!(!blank.is_prohibited());
    assert!(blank.display_name.is_none());

    assert!(blank.identity.gender.is_none());
    assert!(blank.identity.age_years.is_none());
    assert!(blank.identity.species.is_none());
    assert!(blank.personality.traits.is_empty());
    assert!(blank.personality.values.is_empty());
    assert!(blank.appearance.hair.is_none());
    assert!(blank.appearance.eyes.is_none());
    assert!(blank.appearance.aesthetic.is_none());
    assert!(blank.voice.profile_id.is_none());

    assert!(blank
        .personality_for(CORE_UUID)
        .expect("personality lookup")
        .is_none());

    assert!(matches!(
        resolve_display_name(7, None),
        Err(ProfileError::DisplayNameRequired(7))
    ));
    assert_eq!(
        resolve_display_name(7, Some("Owner Chosen")).expect("owner-supplied name"),
        "Owner Chosen"
    );
}

#[test]
fn catalog_and_approved_identities_match_the_roster() {
    validate_profiles().expect("valid profiles");
    assert_eq!(profiles().len(), 7);

    let sofia = profile(1).expect("sofia profile");
    assert_eq!(sofia.display_name.as_deref(), Some("Sofía Ada Lyra"));
    assert_eq!(sofia.identity_source, IdentitySource::PreservedLegacy);

    assert_eq!(
        profile(2).expect("mack profile").display_name.as_deref(),
        Some("Mack Fuller")
    );
    assert_eq!(
        profile(3).expect("nova profile").display_name.as_deref(),
        Some("Nova Arden")
    );

    let vera = profile(6).expect("vera profile");
    assert_eq!(vera.display_name.as_deref(), Some("Vera"));
    assert_eq!(
        vera.identity_source,
        IdentitySource::ApprovedExternalSpecification
    );
}

//! Approved Core profiles.
//!
//! This crate is the data source for the owner-approved Core roster. It encodes
//! only explicitly approved attributes. It never invents data for a blank or
//! externally specified Core, and it refuses to overwrite an existing
//! personality when applying a profile.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use aegis_core_sdk::{CoreMemoryStore, PersonalityProfile};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Schema identifier for a Core profile.
pub const CORE_PROFILE_SCHEMA: &str = "aegis.core.profile.v1";
/// Approved Sofía voice reference.
pub const SOFIA_VOICE_REFERENCE: &str = "sofia_r9_bluehour_REFERENCE_AIR_medium";
/// Maximum number of personality traits in a profile.
pub const MAX_PROFILE_TRAITS: usize = 32;

#[derive(Debug, Error)]
pub enum ProfileError {
    #[error("Core number {0} is not in the approved roster")]
    UnknownCore(u8),
    #[error("Core number {0} is prohibited from implementation")]
    ProhibitedCore(u8),
    #[error("Core number {0} is blank; the owner must supply a display name")]
    DisplayNameRequired(u8),
    #[error("Core number {0} must use its approved display name: {1}")]
    DisplayNameMismatch(u8, String),
    #[error("approved profile catalog is invalid: {0}")]
    InvalidCatalog(&'static str),
    #[error("existing personality must be preserved and was not overwritten")]
    PersonalityAlreadyPresent,
    #[error("Core SDK failed: {0}")]
    Sdk(#[from] aegis_core_sdk::SdkError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileStatus {
    Approved,
    DesignOnly,
    BlankFoundation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentitySource {
    PreservedLegacy,
    ApprovedDefinition,
    ApprovedExternalSpecification,
    OwnerSupplied,
    Prohibited,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdentityAttributes {
    pub gender: Option<String>,
    pub age_years: Option<u8>,
    pub species: Option<String>,
    pub notes: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersonalityAttributes {
    pub traits: BTreeMap<String, f64>,
    pub values: Vec<String>,
    pub flaws: Vec<String>,
    pub relationship_style: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppearanceAttributes {
    pub hair: Option<String>,
    pub eyes: Option<String>,
    pub build: Option<String>,
    pub aesthetic: Option<String>,
    pub wardrobe_tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceAttributes {
    pub profile_id: Option<String>,
    pub description: Option<String>,
    pub preserve_existing: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoreProfile {
    pub schema: String,
    pub core_number: u8,
    pub designation: String,
    pub status: ProfileStatus,
    pub implementation_authorized: bool,
    pub identity_source: IdentitySource,
    pub display_name: Option<String>,
    pub identity: IdentityAttributes,
    pub personality: PersonalityAttributes,
    pub appearance: AppearanceAttributes,
    pub voice: VoiceAttributes,
    pub specification_reference: Option<String>,
    pub notes: String,
}

impl CoreProfile {
    pub fn is_prohibited(&self) -> bool {
        self.status == ProfileStatus::DesignOnly || !self.implementation_authorized
    }

    /// Build a personality profile for this Core, if one is approved.
    pub fn personality_for(
        &self,
        core_uuid: &str,
    ) -> Result<Option<PersonalityProfile>, ProfileError> {
        if self.is_prohibited() || self.personality.traits.is_empty() {
            return Ok(None);
        }
        let mut profile = PersonalityProfile::neutral(core_uuid)?;
        profile.traits = self
            .personality
            .traits
            .iter()
            .map(|(name, value)| (name.clone(), *value))
            .collect();
        profile.values = self.personality.values.clone();
        profile.tone = "approved profile".to_owned();
        profile.growth_policy = "append_only".to_owned();
        profile.revision = 1;
        profile.validate()?;
        Ok(Some(profile))
    }

    /// Apply the approved personality to an installed Core without overwriting
    /// any existing personality.
    pub fn apply_personality(
        &self,
        memory: &CoreMemoryStore,
        core_uuid: &str,
    ) -> Result<ProfileApplyOutcome, ProfileError> {
        if memory.load_personality()?.is_some() {
            return Err(ProfileError::PersonalityAlreadyPresent);
        }
        match self.personality_for(core_uuid)? {
            Some(profile) => {
                memory.save_personality(&profile)?;
                Ok(ProfileApplyOutcome {
                    applied: true,
                    reason: "approved personality applied".to_owned(),
                })
            }
            None => Ok(ProfileApplyOutcome {
                applied: false,
                reason: match self.identity_source {
                    IdentitySource::OwnerSupplied => {
                        "blank identity; no personality is invented".to_owned()
                    }
                    IdentitySource::ApprovedExternalSpecification => {
                        "personality is owned by the approved external specification".to_owned()
                    }
                    _ => "existing identity is preserved; no profile personality is defined"
                        .to_owned(),
                },
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileApplyOutcome {
    pub applied: bool,
    pub reason: String,
}

static PROFILES: LazyLock<Vec<CoreProfile>> = LazyLock::new(build_profiles);

fn base(
    core_number: u8,
    designation: &str,
    status: ProfileStatus,
    implementation_authorized: bool,
    identity_source: IdentitySource,
) -> CoreProfile {
    CoreProfile {
        schema: CORE_PROFILE_SCHEMA.to_owned(),
        core_number,
        designation: designation.to_owned(),
        status,
        implementation_authorized,
        identity_source,
        display_name: None,
        identity: IdentityAttributes {
            gender: None,
            age_years: None,
            species: None,
            notes: String::new(),
        },
        personality: PersonalityAttributes {
            traits: BTreeMap::new(),
            values: Vec::new(),
            flaws: Vec::new(),
            relationship_style: String::new(),
        },
        appearance: AppearanceAttributes {
            hair: None,
            eyes: None,
            build: None,
            aesthetic: None,
            wardrobe_tags: Vec::new(),
        },
        voice: VoiceAttributes {
            profile_id: None,
            description: None,
            preserve_existing: false,
        },
        specification_reference: None,
        notes: String::new(),
    }
}

fn traits(entries: &[(&str, f64)]) -> BTreeMap<String, f64> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), *value))
        .collect()
}

fn build_profiles() -> Vec<CoreProfile> {
    let mut sofia = base(
        1,
        "Sofía Ada Lyra",
        ProfileStatus::Approved,
        true,
        IdentitySource::PreservedLegacy,
    );
    sofia.display_name = Some("Sofía Ada Lyra".to_owned());
    sofia.voice.profile_id = Some(SOFIA_VOICE_REFERENCE.to_owned());
    sofia.voice.preserve_existing = true;
    sofia.specification_reference =
        Some("D:\\SofiaDev\\SofiaAdaLyra legacy specifications (read-only)".to_owned());
    sofia.notes =
        "Preserve the established Sofía identity, personality, assets, and behavior. This profile deliberately does not duplicate or replace them."
            .to_owned();

    let mut mack = base(
        2,
        "Mack Fuller",
        ProfileStatus::Approved,
        true,
        IdentitySource::ApprovedDefinition,
    );
    mack.display_name = Some("Mack Fuller".to_owned());
    mack.identity.gender = Some("male".to_owned());
    mack.identity.age_years = Some(34);
    mack.personality.traits = traits(&[
        ("independence", 0.8),
        ("exploration", 0.85),
        ("individuality", 0.8),
        ("warmth", 0.6),
        ("stubbornness", 0.7),
    ]);
    mack.personality.values = vec!["autonomy".to_owned(), "honesty".to_owned()];
    mack.personality.flaws = vec!["stubbornness".to_owned()];
    mack.personality.relationship_style = "gradual and earned".to_owned();
    mack.appearance.hair = Some("dark hair with stubble".to_owned());
    mack.appearance.build = Some("sturdy practical build".to_owned());
    mack.appearance.aesthetic = Some("practical workwear".to_owned());
    mack.appearance.wardrobe_tags = vec!["practical workwear".to_owned()];
    mack.voice.description = Some("warm".to_owned());
    mack.notes = "Approved independent, exploratory, individualistic identity.".to_owned();

    let mut nova = base(
        3,
        "Nova Arden",
        ProfileStatus::Approved,
        true,
        IdentitySource::ApprovedDefinition,
    );
    nova.display_name = Some("Nova Arden".to_owned());
    nova.identity.gender = Some("female".to_owned());
    nova.identity.species = Some("half-elf".to_owned());
    nova.personality.traits = traits(&[
        ("rebelliousness", 0.8),
        ("playfulness", 0.85),
        ("independence", 0.8),
    ]);
    nova.personality.values = vec!["freedom".to_owned(), "honesty".to_owned()];
    nova.personality.relationship_style = "playful and independent".to_owned();
    nova.appearance.hair = Some("medium rose-colored hair".to_owned());
    nova.appearance.eyes = Some("amber".to_owned());
    nova.appearance.aesthetic = Some("half-elf fantasy".to_owned());
    nova.notes = "Approved rebellious, playful, independent identity.".to_owned();

    let mut twins = Vec::new();
    for number in [4u8, 5u8] {
        let mut twin = base(
            number,
            if number == 4 {
                "Core #4 (design-only twin)"
            } else {
                "Core #5 (design-only twin)"
            },
            ProfileStatus::DesignOnly,
            false,
            IdentitySource::Prohibited,
        );
        twin.notes =
            "Identical twin design only. Implementation, packaging, installation, and activation are prohibited."
                .to_owned();
        twins.push(twin);
    }

    let mut vera = base(
        6,
        "Vera / S-039",
        ProfileStatus::Approved,
        true,
        IdentitySource::ApprovedExternalSpecification,
    );
    vera.display_name = Some("Vera".to_owned());
    vera.specification_reference = Some(
        "Vera / S-039 approved specification (external source of truth; not duplicated here)"
            .to_owned(),
    );
    vera.notes =
        "Spartan-inspired approved identity. Measurements, hair, armor, colors, voice, and personality must be taken from the approved external specification and must not be invented here."
            .to_owned();

    let mut blank = base(
        7,
        "Unwritten",
        ProfileStatus::BlankFoundation,
        true,
        IdentitySource::OwnerSupplied,
    );
    blank.notes =
        "Blank identity foundation. The owner must supply name, gender, species, appearance, voice, and personality through the identity workflow."
            .to_owned();

    let mut profiles = vec![sofia, mack, nova];
    profiles.extend(twins);
    profiles.push(vera);
    profiles.push(blank);
    profiles
}

pub fn profiles() -> &'static [CoreProfile] {
    &PROFILES
}

pub fn profile(core_number: u8) -> Option<&'static CoreProfile> {
    PROFILES
        .iter()
        .find(|profile| profile.core_number == core_number)
}

/// Validate the catalog's internal invariants and roster-name consistency.
pub fn validate_profiles() -> Result<(), ProfileError> {
    if PROFILES.len() != 7 {
        return Err(ProfileError::InvalidCatalog("expected seven Core numbers"));
    }
    let mut seen = std::collections::BTreeSet::new();
    for profile in PROFILES.iter() {
        if !(1..=7).contains(&profile.core_number) {
            return Err(ProfileError::InvalidCatalog("Core number out of range"));
        }
        if !seen.insert(profile.core_number) {
            return Err(ProfileError::InvalidCatalog("duplicate Core number"));
        }
        if profile.schema != CORE_PROFILE_SCHEMA {
            return Err(ProfileError::InvalidCatalog("schema mismatch"));
        }
        if profile.personality.traits.len() > MAX_PROFILE_TRAITS {
            return Err(ProfileError::InvalidCatalog("too many traits"));
        }
        for (name, value) in &profile.personality.traits {
            if name.trim().is_empty()
                || name.chars().count() > 64
                || name.chars().any(char::is_control)
            {
                return Err(ProfileError::InvalidCatalog("trait name"));
            }
            if !value.is_finite() || !(-1.0..=1.0).contains(value) {
                return Err(ProfileError::InvalidCatalog("trait value"));
            }
        }
        let roster = aegis_roster::entry(profile.core_number)
            .ok_or(ProfileError::InvalidCatalog("missing roster entry"))?;
        if roster.default_display_name != profile.display_name {
            return Err(ProfileError::InvalidCatalog(
                "profile display name disagrees with roster",
            ));
        }
        if profile.display_name.is_some()
            && !profile.is_prohibited()
            && profile
                .display_name
                .as_deref()
                .is_some_and(|name| name.trim().is_empty() || name.chars().any(char::is_control))
        {
            return Err(ProfileError::InvalidCatalog("display name"));
        }
    }

    for number in [4u8, 5u8] {
        let profile = profile(number).ok_or(ProfileError::InvalidCatalog("missing twin"))?;
        if !profile.is_prohibited()
            || profile.status != ProfileStatus::DesignOnly
            || profile.identity_source != IdentitySource::Prohibited
            || profile.display_name.is_some()
            || !profile.personality.traits.is_empty()
            || profile.appearance.hair.is_some()
        {
            return Err(ProfileError::InvalidCatalog("prohibited twin profile"));
        }
    }

    let blank = profile(7).ok_or(ProfileError::InvalidCatalog("missing blank Core"))?;
    if blank.status != ProfileStatus::BlankFoundation
        || !blank.implementation_authorized
        || blank.identity_source != IdentitySource::OwnerSupplied
        || blank.display_name.is_some()
        || blank.identity.gender.is_some()
        || blank.identity.species.is_some()
        || !blank.personality.traits.is_empty()
        || blank.appearance.hair.is_some()
        || blank.voice.profile_id.is_some()
    {
        return Err(ProfileError::InvalidCatalog(
            "Core #7 must remain a blank identity",
        ));
    }

    let sofia = profile(1).ok_or(ProfileError::InvalidCatalog("missing Sofía"))?;
    if !sofia.implementation_authorized
        || sofia.identity_source != IdentitySource::PreservedLegacy
        || sofia.voice.profile_id.as_deref() != Some(SOFIA_VOICE_REFERENCE)
        || !sofia.voice.preserve_existing
        || !sofia.personality.traits.is_empty()
    {
        return Err(ProfileError::InvalidCatalog("Sofía preservation profile"));
    }

    for number in [2u8, 3u8] {
        let profile = profile(number).ok_or(ProfileError::InvalidCatalog("missing Core"))?;
        if profile.is_prohibited()
            || profile.identity_source != IdentitySource::ApprovedDefinition
            || profile.display_name.is_none()
            || profile.identity.gender.is_none()
            || profile.personality.traits.is_empty()
        {
            return Err(ProfileError::InvalidCatalog("approved Core profile"));
        }
    }

    let vera = profile(6).ok_or(ProfileError::InvalidCatalog("missing Vera"))?;
    if vera.is_prohibited()
        || vera.identity_source != IdentitySource::ApprovedExternalSpecification
        || vera.display_name.as_deref() != Some("Vera")
        || vera.specification_reference.is_none()
        || !vera.personality.traits.is_empty()
    {
        return Err(ProfileError::InvalidCatalog("Vera profile"));
    }

    Ok(())
}

/// Resolve the display name for an install request using approved profiles.
pub fn resolve_display_name(
    core_number: u8,
    requested: Option<&str>,
) -> Result<String, ProfileError> {
    let profile = profile(core_number).ok_or(ProfileError::UnknownCore(core_number))?;
    if profile.is_prohibited() {
        return Err(ProfileError::ProhibitedCore(core_number));
    }
    let requested = requested.map(str::trim).filter(|value| !value.is_empty());
    match profile.display_name.as_deref() {
        Some(approved) => {
            if requested.is_some_and(|name| name != approved) {
                return Err(ProfileError::DisplayNameMismatch(
                    core_number,
                    approved.to_owned(),
                ));
            }
            Ok(approved.to_owned())
        }
        None => requested
            .map(str::to_owned)
            .ok_or(ProfileError::DisplayNameRequired(core_number)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_valid_and_matches_roster() {
        validate_profiles().expect("profiles");
    }

    #[test]
    fn forbidden_and_blank_profiles_cannot_be_invented() {
        assert!(matches!(
            resolve_display_name(4, Some("Twin")),
            Err(ProfileError::ProhibitedCore(4))
        ));
        assert!(matches!(
            resolve_display_name(7, None),
            Err(ProfileError::DisplayNameRequired(7))
        ));
        assert_eq!(
            resolve_display_name(7, Some("Owner Chosen")).expect("owner name"),
            "Owner Chosen"
        );
    }

    #[test]
    fn approved_personality_is_defined_only_where_approved() {
        let mack = profile(2).expect("mack");
        let personality = mack
            .personality_for("11111111-1111-4111-8111-111111111111")
            .expect("personality")
            .expect("defined");
        assert_eq!(personality.traits.get("exploration"), Some(&0.85));
        for number in [1u8, 6u8, 7u8] {
            assert!(profile(number)
                .expect("profile")
                .personality_for("11111111-1111-4111-8111-111111111111")
                .expect("none")
                .is_none());
        }
    }
}

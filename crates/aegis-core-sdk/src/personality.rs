//! Personality profile contract.
//!
//! Personality is expressive data only. It never grants permission and is never
//! consulted for authorization decisions.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{canonical_core_id, chars_within, now_rfc3339, SdkError};

/// Schema identifier for a personality profile.
pub const PERSONALITY_SCHEMA: &str = "aegis.core.personality.v1";

/// Maximum number of personality traits.
pub const MAX_TRAITS: usize = 128;
/// Maximum character length of a trait name.
pub const MAX_TRAIT_NAME_CHARS: usize = 64;
/// Maximum character length of the tone description.
pub const MAX_TONE_CHARS: usize = 512;
/// Maximum number of declared values.
pub const MAX_VALUES: usize = 64;
/// Maximum character length of a single value.
pub const MAX_VALUE_CHARS: usize = 256;
/// Maximum character length of the growth policy.
pub const MAX_GROWTH_POLICY_CHARS: usize = 512;

const RESERVED_TRAIT_TERMS: [&str; 5] =
    ["permission", "authorization", "role", "admin", "privilege"];

/// Versioned personality profile for one Core.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersonalityProfile {
    pub schema: String,
    pub core_uuid: String,
    pub traits: BTreeMap<String, f64>,
    pub tone: String,
    pub values: Vec<String>,
    pub growth_policy: String,
    pub updated_at: String,
    pub revision: u64,
}

impl PersonalityProfile {
    /// Build a neutral profile that can be edited by the caller.
    pub fn neutral(core_uuid: &str) -> Result<Self, SdkError> {
        let core_uuid = canonical_core_id(core_uuid)?;
        Ok(Self {
            schema: PERSONALITY_SCHEMA.to_owned(),
            core_uuid,
            traits: BTreeMap::new(),
            tone: "neutral".to_owned(),
            values: Vec::new(),
            growth_policy: "append_only".to_owned(),
            updated_at: now_rfc3339()?,
            revision: 1,
        })
    }

    /// Validate schema, bounds, trait values, and reserved trait names.
    ///
    /// Trait names that describe authorization (`permission`, `authorization`,
    /// `role`, `admin`, `privilege`) are rejected case-insensitively so a
    /// profile can never be mistaken for an authority document.
    pub fn validate(&self) -> Result<(), SdkError> {
        if self.schema != PERSONALITY_SCHEMA {
            return Err(SdkError::UnsupportedPersonalitySchema);
        }
        canonical_core_id(&self.core_uuid)?;
        if self.revision == 0 {
            return Err(SdkError::InvalidPersonality("revision must be at least 1"));
        }
        if self.updated_at.trim().is_empty() {
            return Err(SdkError::InvalidPersonality("updated_at is required"));
        }
        if !chars_within(&self.tone, MAX_TONE_CHARS) {
            return Err(SdkError::InvalidPersonality("tone is too long"));
        }
        if self.growth_policy.trim().is_empty()
            || !chars_within(&self.growth_policy, MAX_GROWTH_POLICY_CHARS)
        {
            return Err(SdkError::InvalidPersonality("growth_policy is invalid"));
        }
        if self.values.len() > MAX_VALUES {
            return Err(SdkError::InvalidPersonality("too many values"));
        }
        for value in &self.values {
            if value.trim().is_empty() || !chars_within(value, MAX_VALUE_CHARS) {
                return Err(SdkError::InvalidPersonality("value is invalid"));
            }
        }
        if self.traits.len() > MAX_TRAITS {
            return Err(SdkError::InvalidPersonality("too many traits"));
        }
        for (name, value) in &self.traits {
            let trimmed = name.trim();
            if trimmed.is_empty()
                || !chars_within(trimmed, MAX_TRAIT_NAME_CHARS)
                || trimmed.chars().any(char::is_control)
            {
                return Err(SdkError::InvalidPersonality("trait name is invalid"));
            }
            let lowered = trimmed.to_lowercase();
            if RESERVED_TRAIT_TERMS
                .iter()
                .any(|term| lowered == *term || lowered.contains(term))
            {
                return Err(SdkError::InvalidPersonality(
                    "trait names must not describe authorization",
                ));
            }
            if !value.is_finite() || !(-1.0..=1.0).contains(value) {
                return Err(SdkError::InvalidPersonality(
                    "trait value must be between -1.0 and 1.0",
                ));
            }
        }
        Ok(())
    }
}

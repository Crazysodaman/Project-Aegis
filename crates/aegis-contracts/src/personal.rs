//! Versioned, authority-free personal-state contracts.
//!
//! These types describe a single Core's subjective state: emotions, recent
//! continuity, activities, routines, and outfit decisions. They are expressive
//! data only. Nothing in this module grants permission, encodes a role, or is
//! consulted for authorization.
//!
//! All collections and strings are bounded so a caller can never turn personal
//! state into an unbounded document. Floating-point values must be finite and
//! inside the declared ranges.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::core::validate_core_id;

/// Contract schema for personal-state ownership and storage.
pub const PERSONAL_SCHEMA: &str = "aegis.core.personal.v1";
/// Schema identifier for an emotion state snapshot.
pub const EMOTION_STATE_SCHEMA: &str = "aegis.core.personal.emotion.v1";
/// Schema identifier for an expressed-emotion projection.
pub const EXPRESSED_EMOTION_SCHEMA: &str = "aegis.core.personal.expressed.v1";
/// Schema identifier for a continuity summary.
pub const CONTINUITY_SUMMARY_SCHEMA: &str = "aegis.core.personal.continuity.v1";
/// Schema identifier for an activity record.
pub const ACTIVITY_SCHEMA: &str = "aegis.core.personal.activity.v1";
/// Schema identifier for a routine record.
pub const ROUTINE_SCHEMA: &str = "aegis.core.personal.routine.v1";
/// Schema identifier for an outfit decision record.
pub const OUTFIT_DECISION_SCHEMA: &str = "aegis.core.personal.outfit.v1";

/// Maximum character length of a timestamp string.
pub const MAX_TIMESTAMP_CHARS: usize = 64;
/// Maximum character length of an opaque record identifier.
pub const MAX_ID_CHARS: usize = 128;
/// Maximum character length of an activity title.
pub const MAX_ACTIVITY_TITLE_CHARS: usize = 160;
/// Maximum character length of activity notes.
pub const MAX_ACTIVITY_NOTES_CHARS: usize = 2048;
/// Maximum character length of a routine name.
pub const MAX_ROUTINE_NAME_CHARS: usize = 160;
/// Maximum character length of a routine time-of-day label.
pub const MAX_TIME_OF_DAY_CHARS: usize = 64;
/// Maximum character length of routine notes.
pub const MAX_ROUTINE_NOTES_CHARS: usize = 1024;
/// Maximum character length of a free-text experience summary.
pub const MAX_EXPERIENCE_CHARS: usize = 512;
/// Maximum character length of an open-thread summary.
pub const MAX_OPEN_THREAD_CHARS: usize = 512;
/// Maximum number of recent experiences in a continuity summary.
pub const MAX_RECENT_EXPERIENCES: usize = 8;
/// Maximum number of open threads in a continuity summary.
pub const MAX_OPEN_THREADS: usize = 8;
/// Maximum number of activities in a continuity summary.
pub const MAX_CONTINUITY_ACTIVITIES: usize = 8;
/// Maximum number of asset identifiers on one outfit decision.
pub const MAX_OUTFIT_ASSET_IDS: usize = 32;
/// Maximum character length of one asset identifier.
pub const MAX_ASSET_ID_CHARS: usize = 128;
/// Maximum character length of an outfit rationale.
pub const MAX_OUTFIT_RATIONALE_CHARS: usize = 1024;
/// Maximum character length of a session identifier.
pub const MAX_SESSION_ID_CHARS: usize = 128;
/// Maximum character length of an emotion influence source.
pub const MAX_INFLUENCE_SOURCE_CHARS: usize = 128;
/// Maximum character length of a relationship event type.
pub const MAX_RELATIONSHIP_EVENT_TYPE_CHARS: usize = 64;
/// Upper bound of the explicit relationship trust scale (0 ..= 100).
pub const MAX_RELATIONSHIP_TRUST: u8 = 100;
/// Fraction of the internal emotion that is outwardly expressed (0 ..= 1).
///
/// The projection is a fixed, deterministic attenuation so the same internal
/// state always yields the same expressed state.
pub const EXPRESSED_EMOTION_GAIN: f32 = 0.8;
/// Hard upper bound for a continuity elapsed-offline value (ten years).
pub const MAX_ELAPSED_OFFLINE_SECONDS: u64 = 10 * 365 * 24 * 60 * 60;

/// Validation failures for personal-state contracts.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PersonalContractError {
    #[error("personal schema is unsupported")]
    InvalidSchema,
    #[error("personal Core UUID is invalid")]
    InvalidCoreUuid,
    #[error("personal identifier is invalid")]
    InvalidId,
    #[error("personal timestamp is invalid")]
    InvalidTimestamp,
    #[error("personal string field is invalid: {0}")]
    InvalidString(&'static str),
    #[error("personal number field is out of range: {0}")]
    InvalidNumber(&'static str),
    #[error("personal collection is too large: {0}")]
    TooManyEntries(&'static str),
}

/// Six-dimensional emotion reading for one Core.
///
/// `valence` is signed (-1 negative ..= 1 positive). The remaining dimensions
/// are unsigned intensities (0 ..= 1). A neutral state is all zeros.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EmotionDimensions {
    pub valence: f32,
    pub arousal: f32,
    pub joy: f32,
    pub calm: f32,
    pub curiosity: f32,
    pub tension: f32,
}

impl EmotionDimensions {
    /// Fully neutral dimensions: no valence, no activation.
    pub const fn neutral() -> Self {
        Self {
            valence: 0.0,
            arousal: 0.0,
            joy: 0.0,
            calm: 0.0,
            curiosity: 0.0,
            tension: 0.0,
        }
    }

    /// Validate finiteness and ranges.
    pub fn validate(&self) -> Result<(), PersonalContractError> {
        validate_signed_unit(self.valence, "valence")?;
        for (field, value) in [
            ("arousal", self.arousal),
            ("joy", self.joy),
            ("calm", self.calm),
            ("curiosity", self.curiosity),
            ("tension", self.tension),
        ] {
            validate_unit(value, field)?;
        }
        Ok(())
    }

    /// Clamp every dimension into its declared range.
    ///
    /// Callers must validate finiteness first; `f32::clamp` propagates NaN.
    pub fn clamped(self) -> Self {
        Self {
            valence: self.valence.clamp(-1.0, 1.0),
            arousal: self.arousal.clamp(0.0, 1.0),
            joy: self.joy.clamp(0.0, 1.0),
            calm: self.calm.clamp(0.0, 1.0),
            curiosity: self.curiosity.clamp(0.0, 1.0),
            tension: self.tension.clamp(0.0, 1.0),
        }
    }

    /// Scale every dimension by `gain` and clamp the result into range.
    ///
    /// Callers must validate finiteness first; a non-finite `gain` propagates.
    pub fn scaled(self, gain: f32) -> Self {
        Self {
            valence: self.valence * gain,
            arousal: self.arousal * gain,
            joy: self.joy * gain,
            calm: self.calm * gain,
            curiosity: self.curiosity * gain,
            tension: self.tension * gain,
        }
        .clamped()
    }
}

/// A single deterministic emotion adjustment.
///
/// Each field is a signed delta (-1 ..= 1) applied to the matching dimension.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EmotionDelta {
    pub valence: f32,
    pub arousal: f32,
    pub joy: f32,
    pub calm: f32,
    pub curiosity: f32,
    pub tension: f32,
}

impl EmotionDelta {
    /// No adjustment.
    pub const fn zero() -> Self {
        Self {
            valence: 0.0,
            arousal: 0.0,
            joy: 0.0,
            calm: 0.0,
            curiosity: 0.0,
            tension: 0.0,
        }
    }

    /// Validate finiteness and the signed unit range.
    pub fn validate(&self) -> Result<(), PersonalContractError> {
        for (field, value) in [
            ("valence", self.valence),
            ("arousal", self.arousal),
            ("joy", self.joy),
            ("calm", self.calm),
            ("curiosity", self.curiosity),
            ("tension", self.tension),
        ] {
            validate_signed_unit(value, field)?;
        }
        Ok(())
    }
}

/// One weighted input to the deterministic emotion engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmotionInfluence {
    pub source: String,
    pub weight: f32,
    pub delta: EmotionDelta,
}

impl EmotionInfluence {
    /// Validate source bounds, weight range, and delta ranges.
    pub fn validate(&self) -> Result<(), PersonalContractError> {
        validate_text(&self.source, MAX_INFLUENCE_SOURCE_CHARS, "influence.source")?;
        validate_unit(self.weight, "influence.weight")?;
        self.delta.validate()
    }

    /// Build an influence from an explicit relationship event.
    ///
    /// The weight is derived only from the event's explicit `trust` value; no
    /// trust is defaulted, guessed, or read from a relationship record. The
    /// event's directional `delta` is preserved verbatim.
    pub fn from_relationship_event(
        event: &RelationshipEmotionEvent,
    ) -> Result<Self, PersonalContractError> {
        event.validate()?;
        let source = truncate_chars(
            &format!(
                "relationship:{}:{}",
                event.relationship_id, event.event_type
            ),
            MAX_INFLUENCE_SOURCE_CHARS,
        );
        let influence = Self {
            source,
            weight: f32::from(event.trust) / f32::from(MAX_RELATIONSHIP_TRUST),
            delta: event.delta,
        };
        influence.validate()?;
        Ok(influence)
    }
}

/// An explicit relationship event that may influence emotion.
///
/// `trust` is the event's own explicit value on a 0 ..= 100 scale. Callers must
/// supply it; there is no default and no value is inferred from a relationship
/// store. `delta` carries the event's directional emotional effect.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelationshipEmotionEvent {
    pub relationship_id: String,
    pub event_type: String,
    pub trust: u8,
    pub delta: EmotionDelta,
}

impl RelationshipEmotionEvent {
    /// Validate identifier, event type, explicit trust, and delta ranges.
    pub fn validate(&self) -> Result<(), PersonalContractError> {
        validate_id(&self.relationship_id)?;
        validate_text(
            &self.event_type,
            MAX_RELATIONSHIP_EVENT_TYPE_CHARS,
            "relationship.event_type",
        )?;
        if self.trust > MAX_RELATIONSHIP_TRUST {
            return Err(PersonalContractError::InvalidNumber("relationship.trust"));
        }
        self.delta.validate()
    }
}

/// Persisted emotion state for one Core.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmotionState {
    pub schema: String,
    pub core_uuid: String,
    pub dimensions: EmotionDimensions,
    pub intensity: f32,
    pub updated_at: String,
    pub revision: u64,
}

impl EmotionState {
    /// Build a neutral snapshot with revision 1.
    pub fn neutral(
        core_uuid: &str,
        updated_at: impl Into<String>,
    ) -> Result<Self, PersonalContractError> {
        let parsed =
            validate_core_id(core_uuid).map_err(|_| PersonalContractError::InvalidCoreUuid)?;
        let state = Self {
            schema: EMOTION_STATE_SCHEMA.to_owned(),
            core_uuid: parsed.hyphenated().to_string(),
            dimensions: EmotionDimensions::neutral(),
            intensity: 0.0,
            updated_at: updated_at.into(),
            revision: 1,
        };
        state.validate()?;
        Ok(state)
    }

    /// Validate schema, identity, dimensions, intensity, timestamp, revision.
    pub fn validate(&self) -> Result<(), PersonalContractError> {
        if self.schema != EMOTION_STATE_SCHEMA {
            return Err(PersonalContractError::InvalidSchema);
        }
        validate_core_id(&self.core_uuid).map_err(|_| PersonalContractError::InvalidCoreUuid)?;
        self.dimensions.validate()?;
        validate_unit(self.intensity, "intensity")?;
        validate_timestamp(&self.updated_at)?;
        if self.revision == 0 {
            return Err(PersonalContractError::InvalidNumber("revision"));
        }
        Ok(())
    }

    /// Project the internal state into a bounded, deterministic outward
    /// expression without mutating the internal state.
    pub fn expressed(&self) -> Result<ExpressedEmotion, PersonalContractError> {
        self.expressed_with_gain(EXPRESSED_EMOTION_GAIN)
    }

    /// Project the internal state using an explicit attenuation `gain`
    /// (0 ..= 1). The result is clamped into range.
    pub fn expressed_with_gain(
        &self,
        gain: f32,
    ) -> Result<ExpressedEmotion, PersonalContractError> {
        self.validate()?;
        validate_unit(gain, "expression_gain")?;
        let expressed = ExpressedEmotion {
            schema: EXPRESSED_EMOTION_SCHEMA.to_owned(),
            core_uuid: self.core_uuid.clone(),
            dimensions: self.dimensions.scaled(gain),
            intensity: (self.intensity * gain).clamp(0.0, 1.0),
            internal_revision: self.revision,
        };
        expressed.validate()?;
        Ok(expressed)
    }
}

/// Outward expression of an internal emotion state.
///
/// This is a pure projection: internal dimensions are attenuated and clamped,
/// and `intensity` never exceeds the internal intensity. `internal_revision`
/// links the projection back to the snapshot it was derived from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExpressedEmotion {
    pub schema: String,
    pub core_uuid: String,
    pub dimensions: EmotionDimensions,
    pub intensity: f32,
    pub internal_revision: u64,
}

impl ExpressedEmotion {
    /// Validate schema, identity, dimensions, intensity, and revision.
    pub fn validate(&self) -> Result<(), PersonalContractError> {
        if self.schema != EXPRESSED_EMOTION_SCHEMA {
            return Err(PersonalContractError::InvalidSchema);
        }
        validate_core_id(&self.core_uuid).map_err(|_| PersonalContractError::InvalidCoreUuid)?;
        self.dimensions.validate()?;
        validate_unit(self.intensity, "expressed.intensity")?;
        if self.internal_revision == 0 {
            return Err(PersonalContractError::InvalidNumber(
                "expressed.internal_revision",
            ));
        }
        Ok(())
    }
}

/// Lifecycle status for a personal activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityStatus {
    Planned,
    Active,
    Paused,
    Completed,
    Cancelled,
}

impl ActivityStatus {
    /// Stable storage identifier.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
        }
    }

    /// Parse a storage identifier.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "planned" => Some(Self::Planned),
            "active" => Some(Self::Active),
            "paused" => Some(Self::Paused),
            "completed" => Some(Self::Completed),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

/// An activity the Core is planning, performing, or has finished.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Activity {
    pub id: String,
    pub title: String,
    pub status: ActivityStatus,
    pub started_at: String,
    pub updated_at: String,
    pub notes: String,
}

impl Activity {
    /// Validate identifier, title, timestamps, and notes bounds.
    pub fn validate(&self) -> Result<(), PersonalContractError> {
        validate_id(&self.id)?;
        validate_text(&self.title, MAX_ACTIVITY_TITLE_CHARS, "activity.title")?;
        validate_timestamp(&self.started_at)?;
        validate_timestamp(&self.updated_at)?;
        validate_optional_text(&self.notes, MAX_ACTIVITY_NOTES_CHARS, "activity.notes")
    }
}

/// A recurring or preferred routine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Routine {
    pub id: String,
    pub name: String,
    pub time_of_day: Option<String>,
    pub notes: String,
}

impl Routine {
    /// Validate identifier, name, optional time-of-day, and notes bounds.
    pub fn validate(&self) -> Result<(), PersonalContractError> {
        validate_id(&self.id)?;
        validate_text(&self.name, MAX_ROUTINE_NAME_CHARS, "routine.name")?;
        if let Some(time_of_day) = &self.time_of_day {
            validate_text(time_of_day, MAX_TIME_OF_DAY_CHARS, "routine.time_of_day")?;
        }
        validate_optional_text(&self.notes, MAX_ROUTINE_NOTES_CHARS, "routine.notes")
    }
}

/// Bounded continuity snapshot computed on demand.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContinuitySummary {
    pub schema: String,
    pub core_uuid: String,
    pub last_active_at: String,
    pub last_session_id: Option<String>,
    pub elapsed_offline_seconds: u64,
    pub recent_experiences: Vec<String>,
    pub open_threads: Vec<String>,
    pub activities: Vec<Activity>,
    pub updated_at: String,
}

impl ContinuitySummary {
    /// Validate schema, identity, bounds, and every nested item.
    pub fn validate(&self) -> Result<(), PersonalContractError> {
        if self.schema != CONTINUITY_SUMMARY_SCHEMA {
            return Err(PersonalContractError::InvalidSchema);
        }
        validate_core_id(&self.core_uuid).map_err(|_| PersonalContractError::InvalidCoreUuid)?;
        validate_timestamp(&self.last_active_at)?;
        if let Some(session_id) = &self.last_session_id {
            validate_text(session_id, MAX_SESSION_ID_CHARS, "last_session_id")?;
        }
        if self.elapsed_offline_seconds > MAX_ELAPSED_OFFLINE_SECONDS {
            return Err(PersonalContractError::InvalidNumber(
                "elapsed_offline_seconds",
            ));
        }
        if self.recent_experiences.len() > MAX_RECENT_EXPERIENCES {
            return Err(PersonalContractError::TooManyEntries("recent_experiences"));
        }
        for experience in &self.recent_experiences {
            validate_text(experience, MAX_EXPERIENCE_CHARS, "recent_experience")?;
        }
        if self.open_threads.len() > MAX_OPEN_THREADS {
            return Err(PersonalContractError::TooManyEntries("open_threads"));
        }
        for thread in &self.open_threads {
            validate_text(thread, MAX_OPEN_THREAD_CHARS, "open_thread")?;
        }
        if self.activities.len() > MAX_CONTINUITY_ACTIVITIES {
            return Err(PersonalContractError::TooManyEntries("activities"));
        }
        for activity in &self.activities {
            activity.validate()?;
        }
        validate_timestamp(&self.updated_at)
    }
}

/// Append-only record of a wardrobe decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutfitDecisionRecord {
    pub id: String,
    pub decided_at: String,
    pub asset_ids: Vec<String>,
    pub rationale: String,
}

impl OutfitDecisionRecord {
    /// Validate identifier, timestamp, asset identifiers, and rationale bounds.
    pub fn validate(&self) -> Result<(), PersonalContractError> {
        validate_id(&self.id)?;
        validate_timestamp(&self.decided_at)?;
        if self.asset_ids.len() > MAX_OUTFIT_ASSET_IDS {
            return Err(PersonalContractError::TooManyEntries("asset_ids"));
        }
        for asset_id in &self.asset_ids {
            validate_text(asset_id, MAX_ASSET_ID_CHARS, "asset_id")?;
        }
        validate_optional_text(
            &self.rationale,
            MAX_OUTFIT_RATIONALE_CHARS,
            "outfit.rationale",
        )
    }
}

fn validate_id(value: &str) -> Result<(), PersonalContractError> {
    if value.trim().is_empty()
        || value.chars().count() > MAX_ID_CHARS
        || value.chars().any(char::is_control)
    {
        return Err(PersonalContractError::InvalidId);
    }
    Ok(())
}

fn validate_timestamp(value: &str) -> Result<(), PersonalContractError> {
    if value.trim().is_empty()
        || value.chars().count() > MAX_TIMESTAMP_CHARS
        || value.chars().any(char::is_control)
    {
        return Err(PersonalContractError::InvalidTimestamp);
    }
    Ok(())
}

fn validate_text(
    value: &str,
    maximum: usize,
    field: &'static str,
) -> Result<(), PersonalContractError> {
    if value.trim().is_empty()
        || value.chars().count() > maximum
        || value.chars().any(char::is_control)
    {
        return Err(PersonalContractError::InvalidString(field));
    }
    Ok(())
}

fn validate_optional_text(
    value: &str,
    maximum: usize,
    field: &'static str,
) -> Result<(), PersonalContractError> {
    if value.chars().count() > maximum || value.chars().any(char::is_control) {
        return Err(PersonalContractError::InvalidString(field));
    }
    Ok(())
}

fn validate_unit(value: f32, field: &'static str) -> Result<(), PersonalContractError> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(PersonalContractError::InvalidNumber(field));
    }
    Ok(())
}

fn validate_signed_unit(value: f32, field: &'static str) -> Result<(), PersonalContractError> {
    if !value.is_finite() || !(-1.0..=1.0).contains(&value) {
        return Err(PersonalContractError::InvalidNumber(field));
    }
    Ok(())
}

fn truncate_chars(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CORE: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";

    #[test]
    fn neutral_state_is_valid() {
        let state = EmotionState::neutral(CORE, "2026-11-26T00:00:00Z").expect("neutral");
        state.validate().expect("valid");
        assert_eq!(state.dimensions, EmotionDimensions::neutral());
        assert_eq!(state.intensity, 0.0);
    }

    #[test]
    fn out_of_range_dimensions_are_rejected() {
        let mut dimensions = EmotionDimensions::neutral();
        dimensions.valence = 1.5;
        assert_eq!(
            dimensions.validate(),
            Err(PersonalContractError::InvalidNumber("valence"))
        );

        dimensions.valence = 0.0;
        dimensions.arousal = -0.1;
        assert_eq!(
            dimensions.validate(),
            Err(PersonalContractError::InvalidNumber("arousal"))
        );

        dimensions.arousal = f32::NAN;
        assert!(dimensions.validate().is_err());
    }

    #[test]
    fn continuity_counts_and_strings_are_bounded() {
        let summary = ContinuitySummary {
            schema: CONTINUITY_SUMMARY_SCHEMA.to_owned(),
            core_uuid: CORE.to_owned(),
            last_active_at: "2026-11-26T00:00:00Z".to_owned(),
            last_session_id: Some("session".to_owned()),
            elapsed_offline_seconds: 10,
            recent_experiences: vec!["one".to_owned()],
            open_threads: vec!["thread".to_owned()],
            activities: Vec::new(),
            updated_at: "2026-11-26T00:10:00Z".to_owned(),
        };
        summary.validate().expect("valid summary");

        let mut too_many = summary.clone();
        too_many.recent_experiences = (0..=MAX_RECENT_EXPERIENCES)
            .map(|index| format!("experience {index}"))
            .collect();
        assert_eq!(
            too_many.validate(),
            Err(PersonalContractError::TooManyEntries("recent_experiences"))
        );

        let mut oversized = summary;
        oversized.open_threads = vec!["x".repeat(MAX_OPEN_THREAD_CHARS + 1)];
        assert_eq!(
            oversized.validate(),
            Err(PersonalContractError::InvalidString("open_thread"))
        );
    }

    #[test]
    fn outfit_decisions_are_bounded() {
        let record = OutfitDecisionRecord {
            id: "outfit-1".to_owned(),
            decided_at: "2026-11-26T00:00:00Z".to_owned(),
            asset_ids: vec!["asset-1".to_owned()],
            rationale: "warm and simple".to_owned(),
        };
        record.validate().expect("valid outfit");

        let mut too_many = record.clone();
        too_many.asset_ids = (0..=MAX_OUTFIT_ASSET_IDS)
            .map(|index| format!("asset-{index}"))
            .collect();
        assert_eq!(
            too_many.validate(),
            Err(PersonalContractError::TooManyEntries("asset_ids"))
        );
    }

    #[test]
    fn activity_status_round_trips() {
        for status in [
            ActivityStatus::Planned,
            ActivityStatus::Active,
            ActivityStatus::Paused,
            ActivityStatus::Completed,
            ActivityStatus::Cancelled,
        ] {
            assert_eq!(ActivityStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(ActivityStatus::parse("unknown"), None);
    }

    #[test]
    fn expressed_projection_is_bounded_and_deterministic() {
        let mut state = EmotionState::neutral(CORE, "2026-11-26T00:00:00Z").expect("neutral");
        state.dimensions = EmotionDimensions {
            valence: 1.0,
            arousal: 1.0,
            joy: 1.0,
            calm: 1.0,
            curiosity: 1.0,
            tension: 1.0,
        };
        state.intensity = 1.0;
        state.revision = 7;

        let first = state.expressed().expect("expressed");
        let second = state.expressed().expect("expressed again");

        assert_eq!(first, second);
        assert_eq!(first.schema, EXPRESSED_EMOTION_SCHEMA);
        assert_eq!(first.core_uuid, CORE);
        assert_eq!(first.internal_revision, 7);
        first.validate().expect("valid expressed");
        assert!(first.intensity <= state.intensity);
        assert!((first.dimensions.valence - EXPRESSED_EMOTION_GAIN).abs() < 1e-6);
        assert!((first.dimensions.tension - EXPRESSED_EMOTION_GAIN).abs() < 1e-6);
        assert_eq!(
            state.dimensions,
            EmotionDimensions {
                valence: 1.0,
                arousal: 1.0,
                joy: 1.0,
                calm: 1.0,
                curiosity: 1.0,
                tension: 1.0,
            }
        );
    }

    #[test]
    fn expressed_projection_rejects_invalid_gain() {
        let state = EmotionState::neutral(CORE, "2026-11-26T00:00:00Z").expect("neutral");
        assert_eq!(
            state.expressed_with_gain(1.5),
            Err(PersonalContractError::InvalidNumber("expression_gain"))
        );
        assert!(state.expressed_with_gain(f32::NAN).is_err());
        let zero = state.expressed_with_gain(0.0).expect("zero gain");
        assert_eq!(zero.dimensions, EmotionDimensions::neutral());
        assert_eq!(zero.intensity, 0.0);
    }

    #[test]
    fn relationship_event_influence_uses_explicit_trust() {
        let event = RelationshipEmotionEvent {
            relationship_id: "rel-1".to_owned(),
            event_type: "kindness".to_owned(),
            trust: 40,
            delta: EmotionDelta {
                valence: 0.5,
                arousal: 0.0,
                joy: 0.4,
                calm: 0.1,
                curiosity: 0.0,
                tension: 0.0,
            },
        };
        event.validate().expect("valid event");
        let influence = EmotionInfluence::from_relationship_event(&event).expect("influence");
        influence.validate().expect("valid influence");
        assert!((influence.weight - 0.4).abs() < 1e-6);
        assert_eq!(influence.delta, event.delta);
        assert!(influence.source.contains("rel-1"));
        assert!(influence.source.contains("kindness"));
    }

    #[test]
    fn relationship_event_rejects_invalid_trust_and_fields() {
        let base = RelationshipEmotionEvent {
            relationship_id: "rel-1".to_owned(),
            event_type: "kindness".to_owned(),
            trust: 40,
            delta: EmotionDelta::zero(),
        };

        let mut too_trusting = base.clone();
        too_trusting.trust = MAX_RELATIONSHIP_TRUST + 1;
        assert_eq!(
            EmotionInfluence::from_relationship_event(&too_trusting),
            Err(PersonalContractError::InvalidNumber("relationship.trust"))
        );

        let mut empty_id = base.clone();
        empty_id.relationship_id = String::new();
        assert_eq!(empty_id.validate(), Err(PersonalContractError::InvalidId));

        let mut empty_type = base;
        empty_type.event_type = String::new();
        assert_eq!(
            empty_type.validate(),
            Err(PersonalContractError::InvalidString(
                "relationship.event_type"
            ))
        );
    }

    #[test]
    fn relationship_event_source_is_bounded() {
        let event = RelationshipEmotionEvent {
            relationship_id: "r".repeat(MAX_ID_CHARS),
            event_type: "e".repeat(MAX_RELATIONSHIP_EVENT_TYPE_CHARS),
            trust: 100,
            delta: EmotionDelta::zero(),
        };
        let influence = EmotionInfluence::from_relationship_event(&event).expect("influence");
        assert!(influence.source.chars().count() <= MAX_INFLUENCE_SOURCE_CHARS);
        assert!((influence.weight - 1.0).abs() < 1e-6);
    }
}

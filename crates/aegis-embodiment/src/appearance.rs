//! Appearance, expression, gesture, wardrobe, and voice interface contracts.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::EmbodimentError;

/// Schema identifier for a Core appearance configuration.
pub const APPEARANCE_SCHEMA: &str = "aegis.embodiment.appearance.v1";
/// Approved Aegis primary color.
pub const AEGIS_PRIMARY: &str = "#9400D3";
/// Approved Aegis accent color.
pub const AEGIS_ACCENT: &str = "#00C2FF";
/// Approved Aegis tertiary color.
pub const AEGIS_TERTIARY: &str = "#39FF14";
/// Maximum named expressions.
pub const MAX_EXPRESSIONS: usize = 128;
/// Maximum named gestures.
pub const MAX_GESTURES: usize = 256;
/// Maximum wardrobe entries.
pub const MAX_WARDROBE: usize = 128;

/// A Core's theme colors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Theme {
    pub primary: String,
    pub accent: String,
    pub tertiary: String,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            primary: AEGIS_PRIMARY.to_owned(),
            accent: AEGIS_ACCENT.to_owned(),
            tertiary: AEGIS_TERTIARY.to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceProfile {
    pub voice_id: String,
    pub display_name: String,
    pub sample_asset_id: Option<String>,
}

/// Persisted appearance configuration for one Core.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Appearance {
    pub schema: String,
    pub core_uuid: String,
    pub avatar_asset_id: Option<String>,
    pub theme: Theme,
    pub expressions: BTreeMap<String, String>,
    pub gestures: BTreeMap<String, String>,
    pub wardrobe: Vec<String>,
    pub voice: Option<VoiceProfile>,
    pub updated_at: String,
    pub revision: u64,
}

impl Appearance {
    pub fn new(core_uuid: &str, updated_at: &str) -> Self {
        Self {
            schema: APPEARANCE_SCHEMA.to_owned(),
            core_uuid: core_uuid.to_owned(),
            avatar_asset_id: None,
            theme: Theme::default(),
            expressions: BTreeMap::new(),
            gestures: BTreeMap::new(),
            wardrobe: Vec::new(),
            voice: None,
            updated_at: updated_at.to_owned(),
            revision: 1,
        }
    }

    pub fn validate(&self, expected_core: &str) -> Result<(), EmbodimentError> {
        if self.schema != APPEARANCE_SCHEMA || self.core_uuid != expected_core {
            return Err(EmbodimentError::InvalidInput("appearance identity"));
        }
        if self.revision < 1 || self.updated_at.trim().is_empty() {
            return Err(EmbodimentError::CorruptData("appearance revision"));
        }
        validate_theme(&self.theme)?;
        if self.expressions.len() > MAX_EXPRESSIONS
            || self.gestures.len() > MAX_GESTURES
            || self.wardrobe.len() > MAX_WARDROBE
        {
            return Err(EmbodimentError::InvalidInput("appearance bounds"));
        }
        for name in self.expressions.keys() {
            validate_animation_name(name)?;
        }
        for name in self.gestures.keys() {
            validate_animation_name(name)?;
        }
        if let Some(voice) = &self.voice {
            if voice.voice_id.trim().is_empty() || voice.voice_id.chars().count() > 128 {
                return Err(EmbodimentError::InvalidInput("voice id"));
            }
            if voice.display_name.trim().is_empty() || voice.display_name.chars().count() > 128 {
                return Err(EmbodimentError::InvalidInput("voice display name"));
            }
        }
        Ok(())
    }
}

pub fn validate_theme(theme: &Theme) -> Result<(), EmbodimentError> {
    for color in [&theme.primary, &theme.accent, &theme.tertiary] {
        if !is_hex_color(color) {
            return Err(EmbodimentError::InvalidInput("theme color"));
        }
    }
    Ok(())
}

pub fn is_hex_color(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 7 && bytes[0] == b'#' && bytes[1..].iter().all(|byte| byte.is_ascii_hexdigit())
}

pub fn validate_animation_name(value: &str) -> Result<(), EmbodimentError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 64 || trimmed.chars().any(char::is_control) {
        return Err(EmbodimentError::InvalidInput("animation name"));
    }
    Ok(())
}

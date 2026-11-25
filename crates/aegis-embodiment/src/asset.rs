//! Asset library contracts.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::EmbodimentError;

/// Schema identifier for a stored asset record.
pub const ASSET_SCHEMA: &str = "aegis.embodiment.asset.v1";
/// Maximum asset size accepted by the library (256 MiB).
pub const MAX_ASSET_BYTES: u64 = 256 * 1024 * 1024;
/// Maximum asset name length in characters.
pub const MAX_ASSET_NAME_CHARS: usize = 128;
/// Maximum media-type length.
pub const MAX_MEDIA_TYPE_CHARS: usize = 128;
/// Maximum number of tags per asset.
pub const MAX_ASSET_TAGS: usize = 32;
/// Maximum metadata size in bytes.
pub const MAX_ASSET_METADATA_BYTES: usize = 64 * 1024;

/// Classification of a stored embodiment asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Avatar,
    Expression,
    Gesture,
    Wardrobe,
    VoiceSample,
    Theme,
    Texture,
    Other,
}

impl AssetKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Avatar => "avatar",
            Self::Expression => "expression",
            Self::Gesture => "gesture",
            Self::Wardrobe => "wardrobe",
            Self::VoiceSample => "voice_sample",
            Self::Theme => "theme",
            Self::Texture => "texture",
            Self::Other => "other",
        }
    }

    pub fn parse(value: &str) -> Result<Self, EmbodimentError> {
        match value {
            "avatar" => Ok(Self::Avatar),
            "expression" => Ok(Self::Expression),
            "gesture" => Ok(Self::Gesture),
            "wardrobe" => Ok(Self::Wardrobe),
            "voice_sample" => Ok(Self::VoiceSample),
            "theme" => Ok(Self::Theme),
            "texture" => Ok(Self::Texture),
            "other" => Ok(Self::Other),
            _ => Err(EmbodimentError::InvalidAssetKind),
        }
    }
}

/// One stored asset record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Asset {
    pub schema: String,
    pub asset_id: String,
    pub core_uuid: String,
    pub kind: AssetKind,
    pub name: String,
    pub media_type: String,
    pub relative_path: String,
    pub sha256: String,
    pub bytes: u64,
    pub tags: Vec<String>,
    pub metadata: Value,
    pub created_at: String,
    pub updated_at: String,
    pub deleted: bool,
    pub revision: u64,
}

/// Input used to import a new asset.
#[derive(Debug, Clone)]
pub struct NewAsset<'a> {
    pub kind: AssetKind,
    pub name: &'a str,
    pub media_type: &'a str,
    pub bytes: &'a [u8],
    pub tags: Vec<String>,
    pub metadata: Option<&'a Value>,
}

impl<'a> NewAsset<'a> {
    pub fn new(kind: AssetKind, name: &'a str, media_type: &'a str, bytes: &'a [u8]) -> Self {
        Self {
            kind,
            name,
            media_type,
            bytes,
            tags: Vec::new(),
            metadata: None,
        }
    }

    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    pub fn with_metadata(mut self, metadata: &'a Value) -> Self {
        self.metadata = Some(metadata);
        self
    }
}

pub(crate) fn validate_new_asset(asset: &NewAsset<'_>) -> Result<(), EmbodimentError> {
    validate_name(asset.name)?;
    if asset.media_type.trim().is_empty()
        || asset.media_type.chars().count() > MAX_MEDIA_TYPE_CHARS
        || asset.media_type.chars().any(char::is_control)
    {
        return Err(EmbodimentError::InvalidInput("asset media type"));
    }
    if asset.bytes.is_empty() || asset.bytes.len() as u64 > MAX_ASSET_BYTES {
        return Err(EmbodimentError::InvalidInput("asset bytes"));
    }
    if asset.tags.len() > MAX_ASSET_TAGS {
        return Err(EmbodimentError::InvalidInput("asset tags"));
    }
    for tag in &asset.tags {
        if tag.trim().is_empty() || tag.chars().count() > 64 || tag.chars().any(char::is_control) {
            return Err(EmbodimentError::InvalidInput("asset tag"));
        }
    }
    if let Some(metadata) = asset.metadata {
        let encoded = serde_json::to_vec(metadata)?;
        if encoded.len() > MAX_ASSET_METADATA_BYTES {
            return Err(EmbodimentError::InvalidInput("asset metadata"));
        }
    }
    Ok(())
}

pub(crate) fn validate_name(name: &str) -> Result<(), EmbodimentError> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || name.chars().count() > MAX_ASSET_NAME_CHARS
        || name.chars().any(char::is_control)
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains("..")
    {
        return Err(EmbodimentError::InvalidInput("asset name"));
    }
    Ok(())
}

pub fn safe_file_name(name: &str) -> String {
    let mut output = String::with_capacity(name.len());
    for character in name.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
            output.push(character);
        } else {
            output.push('_');
        }
    }
    if output.is_empty() || output == "." || output == ".." {
        output.push_str("asset");
    }
    output
}

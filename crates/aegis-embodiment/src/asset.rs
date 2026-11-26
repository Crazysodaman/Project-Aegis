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
/// Schema identifier for an asset-library audit report.
pub const ASSET_AUDIT_SCHEMA: &str = "aegis.embodiment.audit.v1";
/// Schema identifier for a recovery manifest.
pub const ASSET_MANIFEST_SCHEMA: &str = "aegis.embodiment.manifest.v1";
/// Maximum number of assets returned by one filtered search.
pub const MAX_ASSET_SEARCH_LIMIT: usize = 1000;

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

/// Lifecycle state reported for an asset in audits and manifests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetState {
    Live,
    Deleted,
}

impl AssetState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Deleted => "deleted",
        }
    }
}

/// Per-asset result produced by an integrity audit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditOutcome {
    Verified,
    Missing,
    Corrupt,
}

impl AuditOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Missing => "missing",
            Self::Corrupt => "corrupt",
        }
    }
}

/// Query used to filter and bound asset listings. Filtering is additive: an
/// empty filter returns every asset in the Core (subject to the result limit).
#[derive(Debug, Clone, Default)]
pub struct AssetFilter {
    /// Restrict to one asset kind.
    pub kind: Option<AssetKind>,
    /// Case-insensitive substring match against the asset name.
    pub name_contains: Option<String>,
    /// Require every listed tag to be present (case-insensitive).
    pub tags: Vec<String>,
    /// Case-insensitive exact match against the asset media type.
    pub media_type: Option<String>,
    /// Include soft-deleted assets in the result.
    pub include_deleted: bool,
    /// Maximum number of results; clamped to [`MAX_ASSET_SEARCH_LIMIT`].
    pub limit: Option<usize>,
}

impl AssetFilter {
    /// Return true when `asset` satisfies every configured predicate.
    pub fn matches(&self, asset: &Asset) -> bool {
        if let Some(kind) = self.kind {
            if asset.kind != kind {
                return false;
            }
        }
        if let Some(name) = &self.name_contains {
            let needle = name.trim();
            if !needle.is_empty() && !asset.name.to_lowercase().contains(&needle.to_lowercase()) {
                return false;
            }
        }
        if let Some(media_type) = &self.media_type {
            let needle = media_type.trim();
            if !needle.is_empty() && !asset.media_type.eq_ignore_ascii_case(needle) {
                return false;
            }
        }
        for tag in &self.tags {
            let tag = tag.trim();
            if tag.is_empty() {
                continue;
            }
            if !asset
                .tags
                .iter()
                .any(|asset_tag| asset_tag.eq_ignore_ascii_case(tag))
            {
                return false;
            }
        }
        true
    }
}

/// One asset entry in an integrity audit report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetAuditEntry {
    pub asset_id: String,
    pub kind: AssetKind,
    pub name: String,
    pub sha256: String,
    pub bytes: u64,
    pub state: AssetState,
    pub relative_path: String,
    pub outcome: AuditOutcome,
}

/// Result of re-hashing every live asset in one Core.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetAudit {
    pub schema: String,
    pub core_uuid: String,
    pub audited_at: String,
    pub entries: Vec<AssetAuditEntry>,
    pub verified: usize,
    pub missing: usize,
    pub corrupt: usize,
}

impl AssetAudit {
    /// True when every audited asset verified and none were missing or corrupt.
    pub fn is_clean(&self) -> bool {
        self.missing == 0 && self.corrupt == 0
    }
}

/// One asset entry in a recovery manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetManifestEntry {
    pub asset_id: String,
    pub kind: AssetKind,
    pub name: String,
    pub media_type: String,
    pub sha256: String,
    pub bytes: u64,
    pub state: AssetState,
    /// Path relative to the owning Core's root directory.
    pub relative_path: String,
}

/// Backup/recovery description of every asset owned by one Core.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetManifest {
    pub schema: String,
    pub core_uuid: String,
    pub generated_at: String,
    pub entries: Vec<AssetManifestEntry>,
}

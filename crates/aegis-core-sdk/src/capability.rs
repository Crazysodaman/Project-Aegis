//! Capability vocabulary and manifest.
//!
//! Capabilities are descriptive only. A manifest never grants authorization.

use serde::{Deserialize, Serialize};

use crate::{canonical_core_id, SdkError, CORE_SDK_NAME, CORE_SDK_VERSION};

/// Schema identifier for a capability manifest.
pub const CAPABILITY_MANIFEST_SCHEMA: &str = "aegis.core.capabilities.v1";

/// Stable capability identifiers exposed by the Universal Core SDK.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreCapability {
    Identity,
    Personality,
    Constitution,
    Memory,
    GrowthHistory,
    Preferences,
    Voice,
    Appearance,
    Avatar,
    Emotions,
    Expressions,
    Gestures,
    Wardrobe,
    Assets,
    Activities,
    Modules,
    Permissions,
    Diagnostics,
    Relationships,
    Social,
    LifecycleEvents,
    Ui,
    ToolModel,
}

/// Every capability currently known to the SDK.
pub const ALL_CAPABILITIES: [CoreCapability; 23] = [
    CoreCapability::Identity,
    CoreCapability::Personality,
    CoreCapability::Constitution,
    CoreCapability::Memory,
    CoreCapability::GrowthHistory,
    CoreCapability::Preferences,
    CoreCapability::Voice,
    CoreCapability::Appearance,
    CoreCapability::Avatar,
    CoreCapability::Emotions,
    CoreCapability::Expressions,
    CoreCapability::Gestures,
    CoreCapability::Wardrobe,
    CoreCapability::Assets,
    CoreCapability::Activities,
    CoreCapability::Modules,
    CoreCapability::Permissions,
    CoreCapability::Diagnostics,
    CoreCapability::Relationships,
    CoreCapability::Social,
    CoreCapability::LifecycleEvents,
    CoreCapability::Ui,
    CoreCapability::ToolModel,
];

/// Capabilities every conforming manifest should advertise.
pub const REQUIRED_CAPABILITIES: [CoreCapability; 3] = [
    CoreCapability::Identity,
    CoreCapability::Personality,
    CoreCapability::Memory,
];

impl CoreCapability {
    /// Stable snake_case identifier.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::Personality => "personality",
            Self::Constitution => "constitution",
            Self::Memory => "memory",
            Self::GrowthHistory => "growth_history",
            Self::Preferences => "preferences",
            Self::Voice => "voice",
            Self::Appearance => "appearance",
            Self::Avatar => "avatar",
            Self::Emotions => "emotions",
            Self::Expressions => "expressions",
            Self::Gestures => "gestures",
            Self::Wardrobe => "wardrobe",
            Self::Assets => "assets",
            Self::Activities => "activities",
            Self::Modules => "modules",
            Self::Permissions => "permissions",
            Self::Diagnostics => "diagnostics",
            Self::Relationships => "relationships",
            Self::Social => "social",
            Self::LifecycleEvents => "lifecycle_events",
            Self::Ui => "ui",
            Self::ToolModel => "tool_model",
        }
    }

    /// Parse a stable identifier.
    pub fn parse(value: &str) -> Option<Self> {
        ALL_CAPABILITIES
            .iter()
            .copied()
            .find(|capability| capability.as_str() == value)
    }

    /// Every capability currently known to the SDK.
    pub fn all() -> &'static [CoreCapability] {
        &ALL_CAPABILITIES
    }
}

/// Versioned description of the capabilities a Core advertises.
///
/// This is descriptive metadata, never an authorization decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityManifest {
    pub schema: String,
    pub sdk_name: String,
    pub sdk_version: String,
    pub core_uuid: String,
    pub capabilities: Vec<CoreCapability>,
}

impl CapabilityManifest {
    /// Build a manifest advertising every known capability for a Core.
    pub fn full(core_uuid: &str) -> Result<Self, SdkError> {
        let core_uuid = canonical_core_id(core_uuid)?;
        Ok(Self {
            schema: CAPABILITY_MANIFEST_SCHEMA.to_owned(),
            sdk_name: CORE_SDK_NAME.to_owned(),
            sdk_version: CORE_SDK_VERSION.to_owned(),
            core_uuid,
            capabilities: CoreCapability::all().to_vec(),
        })
    }

    /// Capabilities every conforming manifest should advertise.
    pub fn required_capabilities() -> &'static [CoreCapability] {
        &REQUIRED_CAPABILITIES
    }

    /// Whether the manifest advertises a capability.
    pub fn has(&self, capability: CoreCapability) -> bool {
        self.capabilities.contains(&capability)
    }

    /// Validate the manifest structure and uniqueness of capability identifiers.
    ///
    /// Validation is purely structural: a valid manifest is not an
    /// authorization grant.
    pub fn validate(&self) -> Result<(), SdkError> {
        if self.schema != CAPABILITY_MANIFEST_SCHEMA
            || self.sdk_name != CORE_SDK_NAME
            || self.sdk_version.trim().is_empty()
        {
            return Err(SdkError::InvalidCapabilityManifest);
        }
        canonical_core_id(&self.core_uuid)?;
        if self.capabilities.len() > ALL_CAPABILITIES.len() {
            return Err(SdkError::InvalidCapabilityManifest);
        }
        let mut seen = std::collections::BTreeSet::new();
        for capability in &self.capabilities {
            if !seen.insert(*capability) {
                return Err(SdkError::InvalidCapabilityManifest);
            }
        }
        Ok(())
    }
}

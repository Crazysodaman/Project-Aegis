//! Validated Fleet host capability declarations.
//!
//! A host's `capabilities` column is free-form JSON, so this module provides a
//! small, validated declaration that can round-trip through that column without
//! weakening the existing API. The declaration is explicit: the serialized
//! `inference` flag is derived only from the declared roles/capabilities, so a
//! host that does not advertise inference can never be recorded as an inference
//! peer by this helper.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{FleetError, HostRole};

/// Schema identifier for a structured host capability declaration.
pub const HOST_CAPABILITIES_SCHEMA: &str = "aegis.fleet.host_capabilities.v1";
/// The only capability name reserved for inference advertisement.
pub const INFERENCE_CAPABILITY: &str = "inference";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostCapabilities {
    pub schema: String,
    pub roles: Vec<HostRole>,
    pub capabilities: Vec<String>,
    /// Derived: true only when the host declares the inference role.
    pub inference: bool,
}

#[derive(Deserialize)]
struct RawCapabilities {
    #[serde(default)]
    roles: Vec<HostRole>,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    inference: bool,
}

impl HostCapabilities {
    /// Maximum number of distinct roles in one declaration.
    pub const MAX_ROLES: usize = 8;
    /// Maximum number of capability labels in one declaration.
    pub const MAX_CAPABILITIES: usize = 32;
    /// Maximum length of a single capability label.
    pub const MAX_CAPABILITY_LEN: usize = 48;

    /// Build and validate a declaration.
    pub fn new<R, C, S>(roles: R, capabilities: C) -> Result<Self, FleetError>
    where
        R: IntoIterator<Item = HostRole>,
        C: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut declared_roles = Vec::new();
        for role in roles {
            if declared_roles.contains(&role) {
                return Err(FleetError::InvalidInput("host roles"));
            }
            declared_roles.push(role);
        }
        if declared_roles.is_empty() || declared_roles.len() > Self::MAX_ROLES {
            return Err(FleetError::InvalidInput("host roles"));
        }

        let mut declared_capabilities = Vec::new();
        for capability in capabilities {
            let capability = capability.as_ref().trim().to_ascii_lowercase();
            if capability.is_empty()
                || capability.chars().count() > Self::MAX_CAPABILITY_LEN
                || capability.chars().any(char::is_control)
                || !capability
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
            {
                return Err(FleetError::InvalidInput("host capabilities"));
            }
            if declared_capabilities.contains(&capability) {
                return Err(FleetError::InvalidInput("host capabilities"));
            }
            declared_capabilities.push(capability);
        }
        if declared_capabilities.len() > Self::MAX_CAPABILITIES {
            return Err(FleetError::InvalidInput("host capabilities"));
        }

        let inference = declared_roles.contains(&HostRole::Inference);
        if declared_capabilities
            .iter()
            .any(|name| name == INFERENCE_CAPABILITY)
            && !inference
        {
            return Err(FleetError::InvalidInput("inference capability"));
        }

        Ok(Self {
            schema: HOST_CAPABILITIES_SCHEMA.to_owned(),
            roles: declared_roles,
            capabilities: declared_capabilities,
            inference,
        })
    }

    /// True only when this declaration advertises inference.
    pub fn supports_inference(&self) -> bool {
        self.inference
    }

    pub fn has_capability(&self, capability: &str) -> bool {
        let capability = capability.trim().to_ascii_lowercase();
        self.capabilities
            .iter()
            .any(|declared| declared == &capability)
    }

    /// Serialize into the free-form `FleetHost.capabilities` value.
    pub fn to_value(&self) -> Result<Value, FleetError> {
        Ok(serde_json::to_value(self)?)
    }

    /// Parse and re-validate a previously serialized declaration.
    ///
    /// Values that advertise inference without an inference role are rejected.
    pub fn from_value(value: &Value) -> Result<Self, FleetError> {
        let raw: RawCapabilities = serde_json::from_value(value.clone())?;
        let RawCapabilities {
            roles,
            capabilities,
            inference,
        } = raw;
        let declared = Self::new(roles, capabilities)?;
        if inference && !declared.inference {
            return Err(FleetError::InvalidInput("inference capability"));
        }
        Ok(declared)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inference_is_declared_explicitly() {
        let caps = HostCapabilities::new(
            [HostRole::Inference],
            [INFERENCE_CAPABILITY, "ollama", "gpu"],
        )
        .expect("capabilities");
        assert!(caps.supports_inference());
        assert!(caps.has_capability("OLLAMA"));

        let general = HostCapabilities::new([HostRole::General], ["smb", "winrm"])
            .expect("general capabilities");
        assert!(!general.supports_inference());
        let value = general.to_value().expect("value");
        assert_eq!(value["inference"], serde_json::json!(false));
    }

    #[test]
    fn declarations_round_trip_through_json() {
        let caps = HostCapabilities::new([HostRole::Inference, HostRole::Storage], ["gpu"])
            .expect("capabilities");
        let value = caps.to_value().expect("value");
        assert_eq!(value["schema"], serde_json::json!(HOST_CAPABILITIES_SCHEMA));
        let parsed = HostCapabilities::from_value(&value).expect("from value");
        assert_eq!(parsed, caps);
    }

    #[test]
    fn invalid_declarations_are_rejected() {
        assert!(HostCapabilities::new([], ["gpu"]).is_err());
        assert!(HostCapabilities::new([HostRole::General, HostRole::General], ["gpu"]).is_err());
        assert!(HostCapabilities::new([HostRole::General], [""]).is_err());
        assert!(HostCapabilities::new([HostRole::General], ["bad capability"]).is_err());
        assert!(HostCapabilities::new([HostRole::General], ["bad\nvalue"]).is_err());
        assert!(HostCapabilities::new([HostRole::General], ["gpu", "gpu"]).is_err());
        assert!(HostCapabilities::new([HostRole::General], ["x".repeat(49)]).is_err());
        assert!(
            HostCapabilities::new([HostRole::General], [INFERENCE_CAPABILITY]).is_err(),
            "inference capability requires the inference role"
        );
    }

    #[test]
    fn from_value_rejects_borrowed_inference_advertisement() {
        let value = serde_json::json!({
            "schema": HOST_CAPABILITIES_SCHEMA,
            "roles": ["general"],
            "capabilities": ["smb"],
            "inference": true,
        });
        assert!(HostCapabilities::from_value(&value).is_err());
    }
}

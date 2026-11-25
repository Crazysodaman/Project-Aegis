//! Platform Core registry and lifecycle contracts.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::core::{validate_core_id, validate_core_number, validate_display_name};
use crate::lifecycle::CoreLifecycleState;

pub const CORE_REGISTRY_SCHEMA: &str = "aegis.core.registry.v1";
pub const CORE_LIFECYCLE_CHANGE_SCHEMA: &str = "aegis.core.lifecycle_change.v1";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RosterContractError {
    #[error("Core registry identity is invalid")]
    InvalidIdentity,
    #[error("Core registry lifecycle state is invalid")]
    InvalidLifecycleState,
    #[error("Core registry storage path is invalid")]
    InvalidStoragePath,
    #[error("Core registry timestamp is invalid")]
    InvalidTimestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreRegistrationRequest {
    pub schema: String,
    pub core_uuid: String,
    pub core_number: u8,
    pub display_name: String,
    pub storage_relative_path: String,
    pub initialized_at: String,
}

impl CoreRegistrationRequest {
    pub fn validate(&self) -> Result<(), RosterContractError> {
        if self.schema != CORE_REGISTRY_SCHEMA {
            return Err(RosterContractError::InvalidIdentity);
        }
        validate_core_id(&self.core_uuid)
            .and_then(|_| validate_core_number(self.core_number))
            .and_then(|_| validate_display_name(&self.display_name))
            .map_err(|_| RosterContractError::InvalidIdentity)?;
        validate_storage_path(&self.storage_relative_path)?;
        if self.initialized_at.trim().is_empty() {
            return Err(RosterContractError::InvalidTimestamp);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreRegistryEntry {
    pub schema: String,
    pub core_uuid: String,
    pub core_number: u8,
    pub display_name: String,
    pub lifecycle_state: CoreLifecycleState,
    pub storage_relative_path: String,
    pub initialized_at: String,
    pub updated_at: String,
}

impl CoreRegistryEntry {
    pub fn validate(&self) -> Result<(), RosterContractError> {
        if self.schema != CORE_REGISTRY_SCHEMA {
            return Err(RosterContractError::InvalidIdentity);
        }
        validate_core_id(&self.core_uuid)
            .and_then(|_| validate_core_number(self.core_number))
            .and_then(|_| validate_display_name(&self.display_name))
            .map_err(|_| RosterContractError::InvalidIdentity)?;
        validate_storage_path(&self.storage_relative_path)?;
        if self.initialized_at.trim().is_empty() || self.updated_at.trim().is_empty() {
            return Err(RosterContractError::InvalidTimestamp);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreLifecycleChange {
    pub schema: String,
    pub core_uuid: String,
    pub from: CoreLifecycleState,
    pub to: CoreLifecycleState,
    pub reason: String,
    pub changed_at: String,
}

impl CoreLifecycleChange {
    pub fn validate(&self) -> Result<(), RosterContractError> {
        if self.schema != CORE_LIFECYCLE_CHANGE_SCHEMA {
            return Err(RosterContractError::InvalidLifecycleState);
        }
        validate_core_id(&self.core_uuid).map_err(|_| RosterContractError::InvalidIdentity)?;
        if self.reason.trim().is_empty() || self.changed_at.trim().is_empty() {
            return Err(RosterContractError::InvalidTimestamp);
        }
        Ok(())
    }
}

pub fn validate_storage_path(path: &str) -> Result<(), RosterContractError> {
    if path.trim().is_empty()
        || path.starts_with('/')
        || path.starts_with('\\')
        || path.contains(':')
        || path.contains("..")
        || path.contains('\\')
    {
        return Err(RosterContractError::InvalidStoragePath);
    }
    if path.split('/').any(|component| component.is_empty()) {
        return Err(RosterContractError::InvalidStoragePath);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_paths_must_be_relative_and_bounded() {
        assert!(validate_storage_path("cores/a9a0a604/core.db").is_ok());
        assert!(validate_storage_path("../core.db").is_err());
        assert!(validate_storage_path("C:/core.db").is_err());
        assert!(validate_storage_path("/core.db").is_err());
        assert!(validate_storage_path("cores//core.db").is_err());
    }
}

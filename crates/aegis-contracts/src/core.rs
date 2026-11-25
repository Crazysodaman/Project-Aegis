//! Core identity contracts.
//!
//! Identity is immutable by Core UUID. Display names are mutable presentation
//! metadata and never replace the UUID as the primary key.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::lifecycle::is_approved_core_number;

pub const CORE_IDENTITY_SCHEMA: &str = "aegis.core.identity.v1";
pub const CORE_STORAGE_SCHEMA: &str = "aegis.core.storage.v1";
pub const CORE_IDENTITY_TABLE: &str = "core_identity";
pub const MAX_DISPLAY_NAME_CHARS: usize = 160;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CoreContractError {
    #[error("Core UUID is invalid")]
    InvalidCoreId,
    #[error("Core number is prohibited")]
    ProhibitedCoreNumber,
    #[error("Core number is not approved")]
    UnapprovedCoreNumber,
    #[error("Core display name is invalid")]
    InvalidDisplayName,
    #[error("Core identity schema is unsupported")]
    InvalidSchema,
    #[error("Core identity revision is invalid")]
    InvalidRevision,
    #[error("Core database instance UUID is invalid")]
    InvalidDatabaseInstance,
    #[error("Core timestamp is invalid")]
    InvalidTimestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreIdentity {
    pub schema: String,
    pub core_uuid: String,
    pub core_number: u8,
    pub display_name: String,
    pub created_at: String,
    pub identity_revision: u64,
}

impl CoreIdentity {
    pub fn new(
        core_uuid: impl Into<String>,
        core_number: u8,
        display_name: impl Into<String>,
        created_at: impl Into<String>,
    ) -> Result<Self, CoreContractError> {
        let core_uuid = canonicalize_core_id(&core_uuid.into())?;
        let identity = Self {
            schema: CORE_IDENTITY_SCHEMA.to_owned(),
            core_uuid,
            core_number,
            display_name: display_name.into(),
            created_at: created_at.into(),
            identity_revision: 1,
        };
        identity.validate()?;
        Ok(identity)
    }

    pub fn validate(&self) -> Result<(), CoreContractError> {
        if self.schema != CORE_IDENTITY_SCHEMA {
            return Err(CoreContractError::InvalidSchema);
        }
        validate_core_id(&self.core_uuid)?;
        validate_core_number(self.core_number)?;
        validate_display_name(&self.display_name)?;
        if self.created_at.trim().is_empty() {
            return Err(CoreContractError::InvalidTimestamp);
        }
        if self.identity_revision < 1 {
            return Err(CoreContractError::InvalidRevision);
        }
        Ok(())
    }

    /// Display-name change only: UUID and Core number are immutable.
    pub fn with_display_name(
        &self,
        display_name: impl Into<String>,
    ) -> Result<Self, CoreContractError> {
        let renamed = Self {
            schema: self.schema.clone(),
            core_uuid: canonicalize_core_id(&self.core_uuid)?,
            core_number: self.core_number,
            display_name: display_name.into(),
            created_at: self.created_at.clone(),
            identity_revision: self.identity_revision.saturating_add(1),
        };
        renamed.validate()?;
        Ok(renamed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreStorageMetadata {
    pub schema: String,
    pub core_uuid: String,
    pub database_instance_uuid: String,
    pub core_number: u8,
    pub display_name: String,
    pub schema_revision: u64,
    pub initialization_mode: String,
    pub created_at: String,
}

impl CoreStorageMetadata {
    pub fn validate(&self) -> Result<(), CoreContractError> {
        if self.schema != CORE_STORAGE_SCHEMA {
            return Err(CoreContractError::InvalidSchema);
        }
        let core = validate_core_id(&self.core_uuid)?;
        let database = validate_core_id(&self.database_instance_uuid)
            .map_err(|_| CoreContractError::InvalidDatabaseInstance)?;
        if core == database {
            return Err(CoreContractError::InvalidDatabaseInstance);
        }
        validate_core_number(self.core_number)?;
        validate_display_name(&self.display_name)?;
        if self.schema_revision < 1 {
            return Err(CoreContractError::InvalidRevision);
        }
        if self.initialization_mode.trim().is_empty() || self.created_at.trim().is_empty() {
            return Err(CoreContractError::InvalidTimestamp);
        }
        Ok(())
    }
}

pub fn canonicalize_core_id(core_uuid: &str) -> Result<String, CoreContractError> {
    Ok(validate_core_id(core_uuid)?.hyphenated().to_string())
}

pub fn validate_core_id(core_uuid: &str) -> Result<Uuid, CoreContractError> {
    let parsed = Uuid::parse_str(core_uuid).map_err(|_| CoreContractError::InvalidCoreId)?;
    if parsed.is_nil() {
        return Err(CoreContractError::InvalidCoreId);
    }
    Ok(parsed)
}

pub fn validate_core_number(core_number: u8) -> Result<(), CoreContractError> {
    if crate::lifecycle::is_prohibited_core_number(core_number) {
        return Err(CoreContractError::ProhibitedCoreNumber);
    }
    if !is_approved_core_number(core_number) {
        return Err(CoreContractError::UnapprovedCoreNumber);
    }
    Ok(())
}

pub fn validate_display_name(display_name: &str) -> Result<(), CoreContractError> {
    let trimmed = display_name.trim();
    if trimmed.is_empty() || trimmed.chars().count() > MAX_DISPLAY_NAME_CHARS {
        return Err(CoreContractError::InvalidDisplayName);
    }
    if display_name.chars().any(|character| character.is_control()) {
        return Err(CoreContractError::InvalidDisplayName);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CORE: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";

    #[test]
    fn uuid_is_immutable_and_name_is_mutable() {
        let identity =
            CoreIdentity::new(CORE, 1, "Sofía", "2026-11-22T00:00:00Z").expect("identity");
        let renamed = identity
            .with_display_name("Sofía Ada Lyra")
            .expect("rename");
        assert_eq!(identity.core_uuid, renamed.core_uuid);
        assert_eq!(identity.core_number, renamed.core_number);
        assert_ne!(identity.display_name, renamed.display_name);
        assert_eq!(renamed.identity_revision, 2);
    }

    #[test]
    fn prohibited_core_numbers_are_rejected() {
        for number in [4u8, 5u8] {
            assert!(matches!(
                CoreIdentity::new(CORE, number, "Test", "2026-11-22T00:00:00Z"),
                Err(CoreContractError::ProhibitedCoreNumber)
            ));
        }
    }

    #[test]
    fn nil_uuid_is_rejected() {
        assert!(matches!(
            CoreIdentity::new(
                "00000000-0000-0000-0000-000000000000",
                1,
                "Test",
                "2026-11-22T00:00:00Z"
            ),
            Err(CoreContractError::InvalidCoreId)
        ));
    }

    #[test]
    fn uuid_is_canonicalized_without_changing_identity() {
        let upper = CoreIdentity::new(
            "A9A0A604-A4E7-4A88-A985-3F071E2C8E7D",
            1,
            "Sofía",
            "2026-11-22T00:00:00Z",
        )
        .expect("identity");
        assert_eq!(upper.core_uuid, CORE);
        let renamed = upper.with_display_name("Sofía Ada Lyra").expect("rename");
        assert_eq!(renamed.core_uuid, CORE);
    }
}

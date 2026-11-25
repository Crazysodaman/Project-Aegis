//! Fresh Core initialization contracts and failure vocabulary.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::core::{validate_core_id, validate_core_number, validate_display_name};

pub const CORE_INIT_REQUEST_SCHEMA: &str = "aegis.core.initialization.v1";
pub const CORE_INIT_CHECKPOINT_SCHEMA: &str = "aegis.core.initialization_checkpoint.v1";
pub const CORE_INIT_REPORT_SCHEMA: &str = "aegis.core.initialization_report.v1";
pub const FRESH_INITIALIZATION_MODE: &str = "fresh_no_migration";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum InitializationContractError {
    #[error("Core identity fields are invalid")]
    InvalidIdentity,
    #[error("initialization stage is invalid")]
    InvalidStage,
    #[error("initialization status is invalid")]
    InvalidStatus,
    #[error("initialization failure code is invalid")]
    InvalidFailureCode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InitializationStage {
    ValidatedIdentity,
    StoragePrepared,
    SchemaApplied,
    IdentityWritten,
    IntegritySealed,
    Registered,
    Failed,
}

impl InitializationStage {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ValidatedIdentity => "validated_identity",
            Self::StoragePrepared => "storage_prepared",
            Self::SchemaApplied => "schema_applied",
            Self::IdentityWritten => "identity_written",
            Self::IntegritySealed => "integrity_sealed",
            Self::Registered => "registered",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "validated_identity" => Some(Self::ValidatedIdentity),
            "storage_prepared" => Some(Self::StoragePrepared),
            "schema_applied" => Some(Self::SchemaApplied),
            "identity_written" => Some(Self::IdentityWritten),
            "integrity_sealed" => Some(Self::IntegritySealed),
            "registered" => Some(Self::Registered),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InitializationStatus {
    InProgress,
    Complete,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InitializationFailureCode {
    InvalidIdentity,
    StorageRejected,
    AlreadyExists,
    ForeignOwnership,
    SchemaMismatch,
    IntegrityFailure,
    RegistrationFailure,
    Interrupted,
    Internal,
}

impl InitializationFailureCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidIdentity => "invalid_identity",
            Self::StorageRejected => "storage_rejected",
            Self::AlreadyExists => "already_exists",
            Self::ForeignOwnership => "foreign_ownership",
            Self::SchemaMismatch => "schema_mismatch",
            Self::IntegrityFailure => "integrity_failure",
            Self::RegistrationFailure => "registration_failure",
            Self::Interrupted => "interrupted",
            Self::Internal => "internal",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreInitializationRequest {
    pub schema: String,
    pub core_uuid: String,
    pub core_number: u8,
    pub display_name: String,
    pub requested_at: String,
}

impl CoreInitializationRequest {
    pub fn validate(&self) -> Result<(), InitializationContractError> {
        if self.schema != CORE_INIT_REQUEST_SCHEMA || self.requested_at.trim().is_empty() {
            return Err(InitializationContractError::InvalidIdentity);
        }
        validate_core_id(&self.core_uuid)
            .and_then(|_| validate_core_number(self.core_number))
            .and_then(|_| validate_display_name(&self.display_name))
            .map_err(|_| InitializationContractError::InvalidIdentity)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitializationCheckpoint {
    pub schema: String,
    pub core_uuid: String,
    pub stage: InitializationStage,
    pub updated_at: String,
    pub failure_code: Option<InitializationFailureCode>,
}

impl InitializationCheckpoint {
    pub fn validate(&self) -> Result<(), InitializationContractError> {
        if self.schema != CORE_INIT_CHECKPOINT_SCHEMA {
            return Err(InitializationContractError::InvalidStage);
        }
        validate_core_id(&self.core_uuid)
            .map_err(|_| InitializationContractError::InvalidIdentity)?;
        if self.updated_at.trim().is_empty() {
            return Err(InitializationContractError::InvalidStage);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreInitializationReport {
    pub schema: String,
    pub core_uuid: String,
    pub status: InitializationStatus,
    pub stage: InitializationStage,
    pub schema_revision: u64,
    pub integrity_sha256: Option<String>,
    pub completed_at: Option<String>,
    pub failure_code: Option<InitializationFailureCode>,
}

impl CoreInitializationReport {
    pub fn validate(&self) -> Result<(), InitializationContractError> {
        if self.schema != CORE_INIT_REPORT_SCHEMA {
            return Err(InitializationContractError::InvalidStatus);
        }
        validate_core_id(&self.core_uuid)
            .map_err(|_| InitializationContractError::InvalidIdentity)?;
        if self.schema_revision < 1 {
            return Err(InitializationContractError::InvalidStatus);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_vocabulary_is_stable() {
        for stage in [
            InitializationStage::ValidatedIdentity,
            InitializationStage::StoragePrepared,
            InitializationStage::SchemaApplied,
            InitializationStage::IdentityWritten,
            InitializationStage::IntegritySealed,
            InitializationStage::Registered,
            InitializationStage::Failed,
        ] {
            assert_eq!(InitializationStage::parse(stage.as_str()), Some(stage));
        }
    }

    #[test]
    fn fresh_mode_is_explicit() {
        assert_eq!(FRESH_INITIALIZATION_MODE, "fresh_no_migration");
    }
}

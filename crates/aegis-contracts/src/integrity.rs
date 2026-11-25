//! Core database integrity contracts.
//!
//! A digest establishes content integrity, not authenticity, package trust, or
//! installation authorization.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::core::{validate_core_id, validate_core_number};

pub const INTEGRITY_MANIFEST_SCHEMA: &str = "aegis.core.integrity_manifest.v1";
pub const INTEGRITY_REPORT_SCHEMA: &str = "aegis.core.integrity_report.v1";
pub const DIGEST_ALGORITHM_SHA256: &str = "sha256";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum IntegrityContractError {
    #[error("integrity manifest schema is unsupported")]
    InvalidSchema,
    #[error("integrity manifest Core UUID is invalid")]
    InvalidCoreId,
    #[error("integrity digest is invalid")]
    InvalidDigest,
    #[error("integrity file name is invalid")]
    InvalidFileName,
    #[error("integrity schema revision is invalid")]
    InvalidSchemaRevision,
    #[error("integrity timestamp is invalid")]
    InvalidTimestamp,
    #[error("integrity database instance UUID is invalid")]
    InvalidDatabaseInstance,
    #[error("integrity initialization mode is invalid")]
    InvalidInitializationMode,
    #[error("integrity Core number is invalid")]
    InvalidCoreNumber,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrityStatus {
    Verified,
    MissingManifest,
    InvalidManifest,
    DatabaseMissing,
    DigestMismatch,
    SchemaMismatch,
    SqliteCorrupt,
    OwnershipMismatch,
}

impl IntegrityStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::MissingManifest => "missing_manifest",
            Self::InvalidManifest => "invalid_manifest",
            Self::DatabaseMissing => "database_missing",
            Self::DigestMismatch => "digest_mismatch",
            Self::SchemaMismatch => "schema_mismatch",
            Self::SqliteCorrupt => "sqlite_corrupt",
            Self::OwnershipMismatch => "ownership_mismatch",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntegrityManifest {
    pub schema: String,
    pub core_uuid: String,
    pub database_file: String,
    pub database_sha256: String,
    pub database_bytes: u64,
    pub schema_revision: u64,
    pub identity_sha256: String,
    pub algorithm: String,
    pub created_at: String,
    #[serde(default)]
    pub database_instance_uuid: Option<String>,
    #[serde(default)]
    pub initialization_mode: Option<String>,
    #[serde(default)]
    pub core_number: Option<u8>,
}

impl IntegrityManifest {
    pub fn validate(&self) -> Result<(), IntegrityContractError> {
        if self.schema != INTEGRITY_MANIFEST_SCHEMA {
            return Err(IntegrityContractError::InvalidSchema);
        }
        validate_core_id(&self.core_uuid).map_err(|_| IntegrityContractError::InvalidCoreId)?;
        if self.database_file.contains('/')
            || self.database_file.contains('\\')
            || self.database_file.contains("..")
            || self.database_file.trim().is_empty()
        {
            return Err(IntegrityContractError::InvalidFileName);
        }
        validate_sha256(&self.database_sha256)?;
        validate_sha256(&self.identity_sha256)?;
        if self.schema_revision < 1 {
            return Err(IntegrityContractError::InvalidSchemaRevision);
        }
        if self.algorithm != DIGEST_ALGORITHM_SHA256 {
            return Err(IntegrityContractError::InvalidSchema);
        }
        if self.created_at.trim().is_empty() {
            return Err(IntegrityContractError::InvalidTimestamp);
        }
        if let Some(instance) = &self.database_instance_uuid {
            validate_core_id(instance)
                .map_err(|_| IntegrityContractError::InvalidDatabaseInstance)?;
            if instance.eq_ignore_ascii_case(&self.core_uuid) {
                return Err(IntegrityContractError::InvalidDatabaseInstance);
            }
        }
        if let Some(mode) = &self.initialization_mode {
            if mode.trim().is_empty() || mode.len() > 64 || mode.chars().any(char::is_control) {
                return Err(IntegrityContractError::InvalidInitializationMode);
            }
        }
        if let Some(core_number) = self.core_number {
            validate_core_number(core_number)
                .map_err(|_| IntegrityContractError::InvalidCoreNumber)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntegrityReport {
    pub schema: String,
    pub core_uuid: String,
    pub status: IntegrityStatus,
    pub expected_sha256: Option<String>,
    pub actual_sha256: Option<String>,
    pub expected_schema_revision: Option<u64>,
    pub actual_schema_revision: Option<u64>,
    pub checked_at: String,
    pub detail: Option<String>,
}

impl IntegrityReport {
    pub fn validate(&self) -> Result<(), IntegrityContractError> {
        if self.schema != INTEGRITY_REPORT_SCHEMA {
            return Err(IntegrityContractError::InvalidSchema);
        }
        validate_core_id(&self.core_uuid).map_err(|_| IntegrityContractError::InvalidCoreId)?;
        if self.checked_at.trim().is_empty() {
            return Err(IntegrityContractError::InvalidTimestamp);
        }
        for digest in [&self.expected_sha256, &self.actual_sha256]
            .into_iter()
            .flatten()
        {
            validate_sha256(digest)?;
        }
        Ok(())
    }
}

pub fn validate_sha256(value: &str) -> Result<(), IntegrityContractError> {
    if value.len() != 64
        || !value
            .chars()
            .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase())
    {
        return Err(IntegrityContractError::InvalidDigest);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CORE: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
    const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn digest_must_be_lowercase_sha256() {
        assert!(validate_sha256(DIGEST).is_ok());
        assert!(validate_sha256(&DIGEST.to_uppercase()).is_err());
        assert!(validate_sha256("abc").is_err());
    }

    #[test]
    fn manifest_rejects_path_components() {
        let manifest = IntegrityManifest {
            schema: INTEGRITY_MANIFEST_SCHEMA.to_owned(),
            core_uuid: CORE.to_owned(),
            database_file: "../core.db".to_owned(),
            database_sha256: DIGEST.to_owned(),
            database_bytes: 1,
            schema_revision: 1,
            identity_sha256: DIGEST.to_owned(),
            algorithm: DIGEST_ALGORITHM_SHA256.to_owned(),
            created_at: "2026-11-22T00:00:00Z".to_owned(),
            database_instance_uuid: None,
            initialization_mode: None,
            core_number: None,
        };
        assert!(manifest.validate().is_err());
    }
}

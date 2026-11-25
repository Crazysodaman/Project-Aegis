//! Validation and mutation helpers for immutable Core identity.
//!
//! The Core UUID and Core number are immutable. Only presentation metadata
//! (the display name) may change, and doing so always increments the identity
//! revision.

use aegis_contracts::core::CoreIdentity;
use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};

use crate::error::IdentityError;

/// Validate a fresh-initialization request and derive the immutable identity.
///
/// The request timestamp becomes the identity creation timestamp and the
/// identity starts at revision 1.
pub fn validate_initialization_request(
    request: &CoreInitializationRequest,
) -> Result<CoreIdentity, IdentityError> {
    if request.schema != CORE_INIT_REQUEST_SCHEMA || request.requested_at.trim().is_empty() {
        return Err(IdentityError::Initialization(
            aegis_contracts::initialization::InitializationContractError::InvalidIdentity,
        ));
    }

    let identity = CoreIdentity::new(
        request.core_uuid.clone(),
        request.core_number,
        request.display_name.clone(),
        request.requested_at.clone(),
    )?;
    Ok(identity)
}

/// Rename the Core while preserving UUID, number, creation time, and schema.
///
/// The returned identity has an incremented `identity_revision`.
pub fn rename_core_identity(
    identity: &CoreIdentity,
    display_name: impl Into<String>,
) -> Result<CoreIdentity, IdentityError> {
    Ok(identity.with_display_name(display_name)?)
}

/// Stateless facade used by the primary orchestrator.
#[derive(Debug, Clone, Copy, Default)]
pub struct CoreIdentityService;

impl CoreIdentityService {
    pub const fn new() -> Self {
        Self
    }

    pub fn validate_request(
        &self,
        request: &CoreInitializationRequest,
    ) -> Result<CoreIdentity, IdentityError> {
        validate_initialization_request(request)
    }

    pub fn rename(
        &self,
        identity: &CoreIdentity,
        display_name: impl Into<String>,
    ) -> Result<CoreIdentity, IdentityError> {
        rename_core_identity(identity, display_name)
    }
}

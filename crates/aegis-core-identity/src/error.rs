//! Error surface for Core identity and initialization state handling.

use aegis_contracts::core::CoreContractError;
use aegis_contracts::initialization::{
    InitializationContractError, InitializationFailureCode, InitializationStage,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum IdentityError {
    #[error(transparent)]
    Core(#[from] CoreContractError),
    #[error(transparent)]
    Initialization(#[from] InitializationContractError),
    #[error("initialization stage cannot transition from {from:?} to {to:?}")]
    IllegalStageTransition {
        from: InitializationStage,
        to: InitializationStage,
    },
    #[error("Core is already fully initialized")]
    AlreadyInitialized,
    #[error("initialization was interrupted at stage {stage:?} and requires explicit cleanup")]
    InterruptedInitialization { stage: InitializationStage },
    #[error("initialization checkpoint is invalid")]
    InvalidCheckpoint,
    #[error("initialization checkpoint I/O failed: {0}")]
    CheckpointIo(#[from] std::io::Error),
    #[error("initialization checkpoint is not valid JSON: {0}")]
    CheckpointJson(#[from] serde_json::Error),
}

impl From<IdentityError> for InitializationContractError {
    fn from(error: IdentityError) -> Self {
        match error {
            IdentityError::Initialization(inner) => inner,
            IdentityError::IllegalStageTransition { .. } | IdentityError::InvalidCheckpoint => {
                Self::InvalidStage
            }
            IdentityError::AlreadyInitialized | IdentityError::InterruptedInitialization { .. } => {
                Self::InvalidStatus
            }
            IdentityError::Core(_)
            | IdentityError::CheckpointIo(_)
            | IdentityError::CheckpointJson(_) => Self::InvalidIdentity,
        }
    }
}

impl From<IdentityError> for InitializationFailureCode {
    fn from(error: IdentityError) -> Self {
        error.failure_code()
    }
}

impl IdentityError {
    /// Map any identity/initialization failure into the shared failure vocabulary.
    pub fn failure_code(&self) -> InitializationFailureCode {
        match self {
            Self::Core(_)
            | Self::Initialization(_)
            | Self::CheckpointIo(_)
            | Self::CheckpointJson(_) => InitializationFailureCode::InvalidIdentity,
            Self::IllegalStageTransition { .. } => InitializationFailureCode::Internal,
            Self::AlreadyInitialized => InitializationFailureCode::AlreadyExists,
            Self::InterruptedInitialization { .. } => InitializationFailureCode::Interrupted,
            Self::InvalidCheckpoint => InitializationFailureCode::IntegrityFailure,
        }
    }
}

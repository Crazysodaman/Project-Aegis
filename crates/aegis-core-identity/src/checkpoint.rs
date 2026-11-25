//! Standalone initialization checkpoint persistence and recovery decisions.
//!
//! Checkpoints are intentionally stored as a single small JSON document so the
//! primary orchestrator can persist progress before SQLite exists. Every write
//! goes to a named temporary file in the same directory, is flushed to disk,
//! and is atomically persisted over the destination. On failure the previous
//! valid checkpoint remains in place; a crash can leave either the old or the
//! new complete document, never a truncated one.

use std::fs;
use std::io::Write;
use std::path::Path;

use aegis_contracts::initialization::{
    InitializationCheckpoint, InitializationFailureCode, InitializationStage,
};
use tempfile::NamedTempFile;

use crate::error::IdentityError;

/// What an existing checkpoint implies for a new initialization attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryDecision {
    /// Initialization reached `Registered`.
    Complete { core_uuid: String },
    /// Initialization previously failed; an explicit retry is allowed.
    Failed {
        core_uuid: String,
        failure_code: Option<InitializationFailureCode>,
    },
    /// Initialization stopped at a non-terminal stage and is not valid.
    Interrupted {
        core_uuid: String,
        stage: InitializationStage,
    },
}

impl RecoveryDecision {
    pub fn is_complete(&self) -> bool {
        matches!(self, Self::Complete { .. })
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }

    pub fn is_interrupted(&self) -> bool {
        matches!(self, Self::Interrupted { .. })
    }

    /// A complete Core is never reinitialized; failed/interrupted runs require
    /// an explicit caller decision.
    pub fn requires_reinitialization(&self) -> bool {
        !self.is_complete()
    }

    /// Interrupted runs must be cleaned up or explicitly reinitialized and are
    /// never treated as a valid Core.
    pub fn requires_explicit_cleanup(&self) -> bool {
        self.is_interrupted()
    }

    pub fn core_uuid(&self) -> &str {
        match self {
            Self::Complete { core_uuid }
            | Self::Failed { core_uuid, .. }
            | Self::Interrupted { core_uuid, .. } => core_uuid,
        }
    }
}

/// Map a checkpoint to a recovery decision. Callers should read checkpoints
/// through [`read_checkpoint`], which validates structural consistency first.
pub fn recover_decision(checkpoint: &InitializationCheckpoint) -> RecoveryDecision {
    match checkpoint.stage {
        InitializationStage::Registered => RecoveryDecision::Complete {
            core_uuid: checkpoint.core_uuid.clone(),
        },
        InitializationStage::Failed => RecoveryDecision::Failed {
            core_uuid: checkpoint.core_uuid.clone(),
            failure_code: checkpoint.failure_code,
        },
        stage => RecoveryDecision::Interrupted {
            core_uuid: checkpoint.core_uuid.clone(),
            stage,
        },
    }
}

/// Validate both contract fields and stage/failure consistency.
fn validate_checkpoint(checkpoint: &InitializationCheckpoint) -> Result<(), IdentityError> {
    checkpoint
        .validate()
        .map_err(|_| IdentityError::InvalidCheckpoint)?;
    match (checkpoint.stage, checkpoint.failure_code) {
        (InitializationStage::Failed, Some(_)) => Ok(()),
        (InitializationStage::Failed, None) => Err(IdentityError::InvalidCheckpoint),
        (_, Some(_)) => Err(IdentityError::InvalidCheckpoint),
        (_, None) => Ok(()),
    }
}

/// Atomically persist a checkpoint, creating parent directories if needed.
pub fn write_checkpoint(
    path: impl AsRef<Path>,
    checkpoint: &InitializationCheckpoint,
) -> Result<(), IdentityError> {
    validate_checkpoint(checkpoint)?;

    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }

    let payload = serde_json::to_vec_pretty(checkpoint)?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary = NamedTempFile::new_in(parent)?;
    temporary.write_all(&payload)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| IdentityError::CheckpointIo(error.error))?;
    set_checkpoint_permissions(path)?;
    Ok(())
}

#[cfg(unix)]
fn set_checkpoint_permissions(path: &Path) -> Result<(), IdentityError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_checkpoint_permissions(_path: &Path) -> Result<(), IdentityError> {
    Ok(())
}

/// Read and validate a checkpoint. A missing file yields `Ok(None)`.
pub fn read_checkpoint(
    path: impl AsRef<Path>,
) -> Result<Option<InitializationCheckpoint>, IdentityError> {
    let bytes = match fs::read(path.as_ref()) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(IdentityError::CheckpointIo(error)),
    };

    let checkpoint: InitializationCheckpoint = serde_json::from_slice(&bytes)?;
    validate_checkpoint(&checkpoint)?;
    Ok(Some(checkpoint))
}

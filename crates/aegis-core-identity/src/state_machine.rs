//! Deterministic initialization stage progression.
//!
//! Legal order:
//! `ValidatedIdentity -> StoragePrepared -> SchemaApplied -> IdentityWritten
//! -> IntegritySealed -> Registered`.
//!
//! Any stage may transition to `Failed` through [`InitializationStateMachine::fail`].
//! Terminal stages are `Registered` and `Failed`.

use aegis_contracts::initialization::{
    InitializationCheckpoint, InitializationFailureCode, InitializationStage, InitializationStatus,
    CORE_INIT_CHECKPOINT_SCHEMA,
};

use crate::checkpoint::{recover_decision, RecoveryDecision};
use crate::error::IdentityError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitializationStateMachine {
    stage: InitializationStage,
    status: InitializationStatus,
    failure_code: Option<InitializationFailureCode>,
}

impl Default for InitializationStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl InitializationStateMachine {
    /// Start a fresh machine at `ValidatedIdentity`.
    pub fn new() -> Self {
        Self {
            stage: InitializationStage::ValidatedIdentity,
            status: InitializationStatus::InProgress,
            failure_code: None,
        }
    }

    /// Begin initialization while guarding against duplicate or interrupted runs.
    ///
    /// * No checkpoint: start fresh.
    /// * Complete checkpoint: reject as already initialized.
    /// * Failed checkpoint: allow an explicit reinitialization.
    /// * Any other checkpoint: reject as interrupted; the caller must clean up
    ///   and retry deliberately. An interrupted run is never silently resumed.
    pub fn start(checkpoint: Option<&InitializationCheckpoint>) -> Result<Self, IdentityError> {
        match checkpoint.map(recover_decision) {
            None | Some(RecoveryDecision::Failed { .. }) => Ok(Self::new()),
            Some(RecoveryDecision::Complete { .. }) => Err(IdentityError::AlreadyInitialized),
            Some(RecoveryDecision::Interrupted { stage, .. }) => {
                Err(IdentityError::InterruptedInitialization { stage })
            }
        }
    }

    pub const fn stage(&self) -> InitializationStage {
        self.stage
    }

    pub const fn status(&self) -> InitializationStatus {
        self.status
    }

    pub const fn failure_code(&self) -> Option<InitializationFailureCode> {
        self.failure_code
    }

    pub fn is_complete(&self) -> bool {
        self.stage == InitializationStage::Registered
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self.stage,
            InitializationStage::Registered | InitializationStage::Failed
        )
    }

    /// Advance exactly one legal stage.
    pub fn advance(&mut self) -> Result<InitializationStage, IdentityError> {
        let next = next_stage(self.stage).ok_or(IdentityError::IllegalStageTransition {
            from: self.stage,
            to: self.stage,
        })?;
        self.advance_to(next)?;
        Ok(self.stage)
    }

    /// Advance to a specific stage, rejecting out-of-order or backward moves.
    pub fn advance_to(&mut self, target: InitializationStage) -> Result<(), IdentityError> {
        if next_stage(self.stage) != Some(target) {
            return Err(IdentityError::IllegalStageTransition {
                from: self.stage,
                to: target,
            });
        }
        self.stage = target;
        self.status = status_for(target);
        self.failure_code = None;
        Ok(())
    }

    /// Move to `Failed` from any non-terminal stage.
    pub fn fail(&mut self, code: InitializationFailureCode) -> Result<(), IdentityError> {
        if self.is_terminal() {
            return Err(IdentityError::IllegalStageTransition {
                from: self.stage,
                to: InitializationStage::Failed,
            });
        }
        self.stage = InitializationStage::Failed;
        self.status = InitializationStatus::Failed;
        self.failure_code = Some(code);
        Ok(())
    }

    /// Snapshot the current stage as a persistable checkpoint.
    pub fn checkpoint(
        &self,
        core_uuid: impl Into<String>,
        updated_at: impl Into<String>,
    ) -> InitializationCheckpoint {
        InitializationCheckpoint {
            schema: CORE_INIT_CHECKPOINT_SCHEMA.to_owned(),
            core_uuid: core_uuid.into(),
            stage: self.stage,
            updated_at: updated_at.into(),
            failure_code: self.failure_code,
        }
    }
}

const fn next_stage(stage: InitializationStage) -> Option<InitializationStage> {
    match stage {
        InitializationStage::ValidatedIdentity => Some(InitializationStage::StoragePrepared),
        InitializationStage::StoragePrepared => Some(InitializationStage::SchemaApplied),
        InitializationStage::SchemaApplied => Some(InitializationStage::IdentityWritten),
        InitializationStage::IdentityWritten => Some(InitializationStage::IntegritySealed),
        InitializationStage::IntegritySealed => Some(InitializationStage::Registered),
        InitializationStage::Registered | InitializationStage::Failed => None,
    }
}

const fn status_for(stage: InitializationStage) -> InitializationStatus {
    match stage {
        InitializationStage::Registered => InitializationStatus::Complete,
        InitializationStage::Failed => InitializationStatus::Failed,
        _ => InitializationStatus::InProgress,
    }
}

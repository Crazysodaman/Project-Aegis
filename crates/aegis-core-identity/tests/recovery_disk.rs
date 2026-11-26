//! Disk-persisted initialization recovery tests (Batch 5 / Agent 10).
//!
//! These tests prove that checkpoint recovery decisions are stable across a
//! real write/read cycle, not only for in-memory checkpoints.

use aegis_contracts::initialization::{
    InitializationFailureCode, InitializationStage, InitializationStatus,
};
use aegis_core_identity::{
    read_checkpoint, recover_decision, write_checkpoint, IdentityError, InitializationStateMachine,
    RecoveryDecision,
};
use tempfile::tempdir;

const CORE: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const TS: &str = "2026-11-22T00:00:00Z";

#[test]
fn interrupted_initialization_on_disk_requires_explicit_cleanup() {
    let directory = tempdir().expect("tempdir");
    let path = directory.path().join("init").join("checkpoint.json");

    let mut machine = InitializationStateMachine::new();
    machine
        .advance_to(InitializationStage::StoragePrepared)
        .expect("advance");
    machine
        .advance_to(InitializationStage::SchemaApplied)
        .expect("advance");
    write_checkpoint(&path, &machine.checkpoint(CORE, TS)).expect("write checkpoint");

    let loaded = read_checkpoint(&path).expect("read").expect("present");
    let decision = recover_decision(&loaded);
    assert_eq!(
        decision,
        RecoveryDecision::Interrupted {
            core_uuid: CORE.to_owned(),
            stage: InitializationStage::SchemaApplied,
        }
    );
    assert!(decision.is_interrupted());
    assert!(decision.requires_explicit_cleanup());
    assert!(decision.requires_reinitialization());

    assert!(matches!(
        InitializationStateMachine::start(Some(&loaded)),
        Err(IdentityError::InterruptedInitialization {
            stage: InitializationStage::SchemaApplied
        })
    ));
}

#[test]
fn failed_initialization_on_disk_allows_explicit_restart() {
    let directory = tempdir().expect("tempdir");
    let path = directory.path().join("init").join("checkpoint.json");

    let mut machine = InitializationStateMachine::new();
    machine
        .advance_to(InitializationStage::StoragePrepared)
        .expect("advance");
    machine
        .fail(InitializationFailureCode::StorageRejected)
        .expect("fail");
    write_checkpoint(&path, &machine.checkpoint(CORE, TS)).expect("write checkpoint");

    let loaded = read_checkpoint(&path).expect("read").expect("present");
    let decision = recover_decision(&loaded);
    assert_eq!(
        decision,
        RecoveryDecision::Failed {
            core_uuid: CORE.to_owned(),
            failure_code: Some(InitializationFailureCode::StorageRejected),
        }
    );
    assert!(decision.is_failed());
    assert!(decision.requires_reinitialization());
    assert!(!decision.requires_explicit_cleanup());

    let restarted =
        InitializationStateMachine::start(Some(&loaded)).expect("failed run may restart");
    assert_eq!(restarted.stage(), InitializationStage::ValidatedIdentity);
    assert_eq!(restarted.status(), InitializationStatus::InProgress);
    assert_eq!(restarted.failure_code(), None);
}

#[test]
fn complete_initialization_on_disk_rejects_reinitialization() {
    let directory = tempdir().expect("tempdir");
    let path = directory.path().join("init").join("checkpoint.json");

    let mut machine = InitializationStateMachine::new();
    while !machine.is_complete() {
        machine.advance().expect("advance");
    }
    write_checkpoint(&path, &machine.checkpoint(CORE, TS)).expect("write checkpoint");

    let loaded = read_checkpoint(&path).expect("read").expect("present");
    let decision = recover_decision(&loaded);
    assert_eq!(
        decision,
        RecoveryDecision::Complete {
            core_uuid: CORE.to_owned(),
        }
    );
    assert!(decision.is_complete());
    assert!(!decision.requires_reinitialization());
    assert!(!decision.requires_explicit_cleanup());

    assert!(matches!(
        InitializationStateMachine::start(Some(&loaded)),
        Err(IdentityError::AlreadyInitialized)
    ));
}

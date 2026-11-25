use std::fs;

use aegis_contracts::core::{CoreContractError, CORE_IDENTITY_SCHEMA};
use aegis_contracts::initialization::{
    CoreInitializationRequest, InitializationCheckpoint, InitializationFailureCode,
    InitializationStage, InitializationStatus, CORE_INIT_CHECKPOINT_SCHEMA,
    CORE_INIT_REQUEST_SCHEMA,
};
use aegis_core_identity::{
    read_checkpoint, recover_decision, rename_core_identity, validate_initialization_request,
    write_checkpoint, CoreIdentityService, IdentityError, InitializationStateMachine,
    RecoveryDecision,
};
use tempfile::tempdir;

const CORE: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const TS: &str = "2026-11-22T00:00:00Z";

fn request(core_number: u8, display_name: &str) -> CoreInitializationRequest {
    CoreInitializationRequest {
        schema: CORE_INIT_REQUEST_SCHEMA.to_owned(),
        core_uuid: CORE.to_owned(),
        core_number,
        display_name: display_name.to_owned(),
        requested_at: TS.to_owned(),
    }
}

fn checkpoint_at(stage: InitializationStage) -> InitializationCheckpoint {
    InitializationCheckpoint {
        schema: CORE_INIT_CHECKPOINT_SCHEMA.to_owned(),
        core_uuid: CORE.to_owned(),
        stage,
        updated_at: TS.to_owned(),
        failure_code: None,
    }
}

#[test]
fn approved_core_numbers_are_accepted() {
    for number in [1u8, 2, 3, 6, 7] {
        let identity = validate_initialization_request(&request(number, "SofA-a"))
            .unwrap_or_else(|error| panic!("core {number} should be accepted: {error}"));
        assert_eq!(identity.core_number, number);
        assert_eq!(identity.schema, CORE_IDENTITY_SCHEMA);
        assert_eq!(identity.identity_revision, 1);
    }
}

#[test]
fn prohibited_core_numbers_are_rejected() {
    for number in [4u8, 5] {
        assert!(matches!(
            validate_initialization_request(&request(number, "SofA-a")),
            Err(IdentityError::Core(CoreContractError::ProhibitedCoreNumber))
        ));
    }
}

#[test]
fn unapproved_core_numbers_are_rejected() {
    for number in [0u8, 8, 9, 255] {
        assert!(matches!(
            validate_initialization_request(&request(number, "SofA-a")),
            Err(IdentityError::Core(CoreContractError::UnapprovedCoreNumber))
        ));
    }
}

#[test]
fn invalid_display_names_are_rejected() {
    let too_long = "a".repeat(161);
    for name in ["", "   ", "bad\nname", "bad\u{7}name", too_long.as_str()] {
        assert!(
            matches!(
                validate_initialization_request(&request(1, name)),
                Err(IdentityError::Core(CoreContractError::InvalidDisplayName))
            ),
            "display name {name:?} should be rejected"
        );
    }
}

#[test]
fn invalid_uuid_is_rejected() {
    let mut invalid = request(1, "SofA-a");
    invalid.core_uuid = "not-a-uuid".to_owned();
    assert!(matches!(
        validate_initialization_request(&invalid),
        Err(IdentityError::Core(CoreContractError::InvalidCoreId))
    ));

    invalid.core_uuid = "00000000-0000-0000-0000-000000000000".to_owned();
    assert!(matches!(
        validate_initialization_request(&invalid),
        Err(IdentityError::Core(CoreContractError::InvalidCoreId))
    ));
}

#[test]
fn wrong_request_schema_is_rejected() {
    let mut invalid = request(1, "SofA-a");
    invalid.schema = "aegis.core.initialization.v0".to_owned();
    assert!(matches!(
        validate_initialization_request(&invalid),
        Err(IdentityError::Initialization(_))
    ));
}

#[test]
fn core_uuid_is_immutable_while_display_name_is_mutable() {
    let service = CoreIdentityService::new();
    let identity = service
        .validate_request(&request(1, "SofA-a"))
        .expect("identity");
    let renamed = service
        .rename(&identity, "SofA-a Ada Lyra")
        .expect("rename");

    assert_eq!(renamed.core_uuid, identity.core_uuid);
    assert_eq!(renamed.core_number, identity.core_number);
    assert_eq!(renamed.created_at, identity.created_at);
    assert_eq!(renamed.schema, identity.schema);
    assert_ne!(renamed.display_name, identity.display_name);
    assert_eq!(renamed.identity_revision, identity.identity_revision + 1);

    let renamed_again = rename_core_identity(&renamed, "SofA-a Ada").expect("second rename");
    assert_eq!(renamed_again.core_uuid, identity.core_uuid);
    assert_eq!(renamed_again.identity_revision, 3);
}

#[test]
fn legal_stages_progress_in_order() {
    let mut machine = InitializationStateMachine::new();
    assert_eq!(machine.stage(), InitializationStage::ValidatedIdentity);
    assert_eq!(machine.status(), InitializationStatus::InProgress);
    assert_eq!(machine.failure_code(), None);

    let expected = [
        InitializationStage::StoragePrepared,
        InitializationStage::SchemaApplied,
        InitializationStage::IdentityWritten,
        InitializationStage::IntegritySealed,
        InitializationStage::Registered,
    ];
    for stage in expected {
        assert_eq!(machine.advance().expect("legal advance"), stage);
    }
    assert_eq!(machine.status(), InitializationStatus::Complete);
    assert!(machine.is_complete());
    assert!(machine.is_terminal());
}

#[test]
fn illegal_transitions_are_rejected() {
    let mut machine = InitializationStateMachine::new();
    assert!(matches!(
        machine.advance_to(InitializationStage::SchemaApplied),
        Err(IdentityError::IllegalStageTransition { .. })
    ));
    assert!(matches!(
        machine.advance_to(InitializationStage::ValidatedIdentity),
        Err(IdentityError::IllegalStageTransition { .. })
    ));
    assert!(matches!(
        machine.advance_to(InitializationStage::Failed),
        Err(IdentityError::IllegalStageTransition { .. })
    ));

    machine
        .advance_to(InitializationStage::StoragePrepared)
        .expect("first step");
    assert!(matches!(
        machine.advance_to(InitializationStage::ValidatedIdentity),
        Err(IdentityError::IllegalStageTransition { .. })
    ));
}

#[test]
fn complete_initialization_is_not_repeated() {
    let mut machine = InitializationStateMachine::new();
    while !machine.is_complete() {
        machine.advance().expect("advance");
    }

    let checkpoint = machine.checkpoint(CORE, TS);
    assert_eq!(
        recover_decision(&checkpoint),
        RecoveryDecision::Complete {
            core_uuid: CORE.to_owned()
        }
    );
    assert!(matches!(
        InitializationStateMachine::start(Some(&checkpoint)),
        Err(IdentityError::AlreadyInitialized)
    ));
    assert!(matches!(
        machine.advance(),
        Err(IdentityError::IllegalStageTransition { .. })
    ));
    assert!(matches!(
        machine.fail(InitializationFailureCode::Internal),
        Err(IdentityError::IllegalStageTransition { .. })
    ));
}

#[test]
fn fail_transitions_to_terminal_failed() {
    let mut machine = InitializationStateMachine::new();
    machine.advance().expect("advance");
    machine
        .fail(InitializationFailureCode::StorageRejected)
        .expect("fail");

    assert_eq!(machine.stage(), InitializationStage::Failed);
    assert_eq!(machine.status(), InitializationStatus::Failed);
    assert_eq!(
        machine.failure_code(),
        Some(InitializationFailureCode::StorageRejected)
    );
    assert!(machine.is_terminal());
    assert!(!machine.is_complete());
    assert!(machine.advance().is_err());
    assert!(machine.fail(InitializationFailureCode::Internal).is_err());

    let checkpoint = machine.checkpoint(CORE, TS);
    assert_eq!(
        recover_decision(&checkpoint),
        RecoveryDecision::Failed {
            core_uuid: CORE.to_owned(),
            failure_code: Some(InitializationFailureCode::StorageRejected),
        }
    );
    let restarted =
        InitializationStateMachine::start(Some(&checkpoint)).expect("failed run may restart");
    assert_eq!(restarted.stage(), InitializationStage::ValidatedIdentity);
    assert_eq!(restarted.status(), InitializationStatus::InProgress);
}

#[test]
fn interrupted_checkpoint_requires_explicit_cleanup() {
    let mut machine = InitializationStateMachine::new();
    machine
        .advance_to(InitializationStage::StoragePrepared)
        .expect("advance");

    let checkpoint = machine.checkpoint(CORE, TS);
    let decision = recover_decision(&checkpoint);
    assert!(decision.is_interrupted());
    assert!(decision.requires_explicit_cleanup());
    assert!(decision.requires_reinitialization());
    assert_eq!(
        decision,
        RecoveryDecision::Interrupted {
            core_uuid: CORE.to_owned(),
            stage: InitializationStage::StoragePrepared,
        }
    );
    assert!(matches!(
        InitializationStateMachine::start(Some(&checkpoint)),
        Err(IdentityError::InterruptedInitialization {
            stage: InitializationStage::StoragePrepared
        })
    ));
}

#[test]
fn checkpoint_round_trips_through_disk() {
    let directory = tempdir().expect("tempdir");
    let path = directory.path().join("nested").join("checkpoint.json");

    let mut machine = InitializationStateMachine::new();
    machine.advance().expect("advance");
    machine.advance().expect("advance");
    assert_eq!(machine.stage(), InitializationStage::SchemaApplied);
    let checkpoint = machine.checkpoint(CORE, TS);

    write_checkpoint(&path, &checkpoint).expect("write");
    let loaded = read_checkpoint(&path).expect("read").expect("present");
    assert_eq!(loaded, checkpoint);

    let mut progressed = InitializationStateMachine::new();
    while progressed.stage() != InitializationStage::IntegritySealed {
        progressed.advance().expect("advance");
    }
    let replacement = progressed.checkpoint(CORE, "2026-11-22T01:00:00Z");
    write_checkpoint(&path, &replacement).expect("overwrite");
    let reloaded = read_checkpoint(&path).expect("read").expect("present");
    assert_eq!(reloaded, replacement);
}

#[test]
fn missing_checkpoint_reads_as_none() {
    let directory = tempdir().expect("tempdir");
    let path = directory.path().join("absent.json");
    assert_eq!(read_checkpoint(&path).expect("read"), None);
}

#[test]
fn corrupt_checkpoint_json_is_rejected() {
    let directory = tempdir().expect("tempdir");
    let path = directory.path().join("corrupt.json");
    fs::write(&path, b"{ this is not json").expect("write");
    assert!(matches!(
        read_checkpoint(&path),
        Err(IdentityError::CheckpointJson(_))
    ));
}

#[test]
fn structurally_invalid_checkpoints_are_rejected() {
    let directory = tempdir().expect("tempdir");

    let failed_without_code = directory.path().join("failed_without_code.json");
    fs::write(
        &failed_without_code,
        serde_json::to_vec(&checkpoint_at(InitializationStage::Failed)).expect("serialize"),
    )
    .expect("write");
    assert!(matches!(
        read_checkpoint(&failed_without_code),
        Err(IdentityError::InvalidCheckpoint)
    ));

    let mut in_progress_with_code = checkpoint_at(InitializationStage::StoragePrepared);
    in_progress_with_code.failure_code = Some(InitializationFailureCode::Internal);
    let in_progress_path = directory.path().join("in_progress_with_code.json");
    fs::write(
        &in_progress_path,
        serde_json::to_vec(&in_progress_with_code).expect("serialize"),
    )
    .expect("write");
    assert!(matches!(
        read_checkpoint(&in_progress_path),
        Err(IdentityError::InvalidCheckpoint)
    ));

    let mut wrong_schema = checkpoint_at(InitializationStage::Registered);
    wrong_schema.schema = "aegis.core.initialization_checkpoint.v0".to_owned();
    let wrong_schema_path = directory.path().join("wrong_schema.json");
    fs::write(
        &wrong_schema_path,
        serde_json::to_vec(&wrong_schema).expect("serialize"),
    )
    .expect("write");
    assert!(matches!(
        read_checkpoint(&wrong_schema_path),
        Err(IdentityError::InvalidCheckpoint)
    ));
}

#[test]
fn write_rejects_invalid_checkpoints() {
    let directory = tempdir().expect("tempdir");
    let path = directory.path().join("checkpoint.json");

    let mut invalid = checkpoint_at(InitializationStage::StoragePrepared);
    invalid.updated_at = "  ".to_owned();
    assert!(matches!(
        write_checkpoint(&path, &invalid),
        Err(IdentityError::InvalidCheckpoint)
    ));
    assert!(!path.exists());

    let invalid_uuid = InitializationCheckpoint {
        core_uuid: "not-a-uuid".to_owned(),
        ..checkpoint_at(InitializationStage::Registered)
    };
    assert!(matches!(
        write_checkpoint(&path, &invalid_uuid),
        Err(IdentityError::InvalidCheckpoint)
    ));
}

#[test]
fn identity_errors_map_to_initialization_vocabulary() {
    let prohibited = validate_initialization_request(&request(4, "SofA-a")).expect_err("error");
    assert_eq!(
        InitializationFailureCode::from(prohibited),
        InitializationFailureCode::InvalidIdentity
    );

    let already = InitializationStateMachine::start(Some(&{
        let mut machine = InitializationStateMachine::new();
        while !machine.is_complete() {
            machine.advance().expect("advance");
        }
        machine.checkpoint(CORE, TS)
    }))
    .expect_err("duplicate");
    assert_eq!(
        already.failure_code(),
        InitializationFailureCode::AlreadyExists
    );
}

#[test]
fn checkpoint_replacement_is_atomic_and_leaves_no_temporary_files() {
    let temp = tempdir().expect("tempdir");
    let path = temp.path().join("checkpoint.json");
    for revision in 0..8 {
        let checkpoint = checkpoint_at(if revision % 2 == 0 {
            InitializationStage::StoragePrepared
        } else {
            InitializationStage::SchemaApplied
        });
        write_checkpoint(&path, &checkpoint).expect("atomic write");
        let loaded = read_checkpoint(&path).expect("read").expect("present");
        assert_eq!(loaded.stage, checkpoint.stage);
    }
    let entries: Vec<_> = fs::read_dir(temp.path())
        .expect("read dir")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        entries,
        vec!["checkpoint.json".to_owned()],
        "no temporary files may remain after atomic replacement"
    );
}

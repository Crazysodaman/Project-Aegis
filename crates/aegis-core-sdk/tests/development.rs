use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_contracts::living::{LIVING_CONTRACT_VERSION, MAX_REFLECTION_SUMMARY_CHARS};
use aegis_core_install::CoreInstaller;
use aegis_core_sdk::{
    CoreSdkContext, DevelopmentStore, PersonalityProfile, PersonalityProposalStatus, SdkError,
    DEVELOPMENT_EVENTS_TABLE, DEVELOPMENT_EVIDENCE_TABLE, DEVELOPMENT_PREFERENCES_TABLE,
    DEVELOPMENT_PROPOSALS_TABLE, DEVELOPMENT_REFLECTIONS_TABLE,
};
use aegis_core_storage::{CoreDatabase, CoreStorageLayout};
use aegis_platform::PlatformService;
use aegis_storage::now_rfc3339;
use rusqlite::Connection;

const SOFIA: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const MACK: &str = "11111111-1111-4111-8111-111111111111";

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    owner_token: String,
}

fn request(core_uuid: &str, core_number: u8, display_name: &str) -> CoreInitializationRequest {
    CoreInitializationRequest {
        schema: CORE_INIT_REQUEST_SCHEMA.to_owned(),
        core_uuid: core_uuid.to_owned(),
        core_number,
        display_name: display_name.to_owned(),
        requested_at: now_rfc3339().expect("timestamp"),
    }
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().to_path_buf();
    let initialized = PlatformService::initialize(&root).expect("platform initialize");
    let installer = CoreInstaller::new(&root).expect("installer");
    installer
        .initialize(
            &request(SOFIA, 1, "Sofía Ada Lyra"),
            &initialized.owner_token,
        )
        .expect("install Sofía");
    installer
        .initialize(&request(MACK, 2, "Mack Fuller"), &initialized.owner_token)
        .expect("install Mack");
    Fixture {
        _temp: temp,
        root,
        owner_token: initialized.owner_token,
    }
}

fn layout(root: &Path) -> CoreStorageLayout {
    CoreStorageLayout::new(root).expect("layout")
}

fn personal_path(root: &Path, core_uuid: &str) -> PathBuf {
    layout(root)
        .core_dir(core_uuid)
        .expect("core dir")
        .join("personal.db")
}

fn traits(pairs: &[(&str, f32)]) -> BTreeMap<String, f32> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), *value))
        .collect()
}

#[test]
fn evidence_forms_preference_and_conflicting_evidence_changes_it() {
    let fixture = fixture();
    let context = CoreSdkContext::open(&fixture.root, SOFIA).expect("sdk context");
    let development = context.development();

    assert!(development
        .get_preference("music.jazz")
        .expect("get preference")
        .is_none());

    let formed = development
        .record_preference_evidence("music.jazz", 0.8, 0.9, "conversation")
        .expect("record evidence");
    assert_eq!(formed.evidence_count, 1);
    assert_eq!(formed.revision, 1);
    assert!(formed.value > 0.0);
    assert!(formed.confidence > 0.0 && formed.confidence <= 1.0);

    let changed = development
        .record_preference_evidence("music.jazz", -0.8, 0.9, "conversation")
        .expect("record conflicting evidence");
    assert_eq!(changed.evidence_count, 2);
    assert_eq!(changed.revision, 2);
    assert!(changed.value < formed.value);
    assert!(changed.confidence >= formed.confidence);
    assert!((-1.0..=1.0).contains(&changed.value));
    assert!((0.0..=1.0).contains(&changed.confidence));

    let listed = development.list_preferences(10).expect("list preferences");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].key, "music.jazz");
}

#[test]
fn evidence_and_preference_bounds_are_enforced() {
    let fixture = fixture();
    let context = CoreSdkContext::open(&fixture.root, SOFIA).expect("sdk context");
    let development = context.development();

    let mut last = development
        .record_preference_evidence("mood", 0.0, 0.3, "test")
        .expect("initial evidence");
    for index in 0..64 {
        let valence = if index % 2 == 0 { 1.0 } else { -1.0 };
        last = development
            .record_preference_evidence("mood", valence, 1.0, "test")
            .expect("bounded evidence");
    }
    assert_eq!(last.evidence_count, 65);
    assert!(last.value >= -1.0 && last.value <= 1.0);
    assert!(last.confidence >= 0.0 && last.confidence <= 1.0);

    assert!(matches!(
        development.record_preference_evidence("mood", 1.1, 0.5, "test"),
        Err(SdkError::InvalidMemory(_))
    ));
    assert!(matches!(
        development.record_preference_evidence("mood", 0.0, 1.1, "test"),
        Err(SdkError::InvalidMemory(_))
    ));
    assert!(matches!(
        development.record_preference_evidence("bad key!", 0.0, 0.5, "test"),
        Err(SdkError::InvalidMemory(_))
    ));
    assert!(matches!(
        development.record_experience("", 0.0, 0.5, "test"),
        Err(SdkError::InvalidMemory(_))
    ));
}

#[test]
fn preferences_reflections_and_proposals_survive_restart() {
    let fixture = fixture();
    {
        let context = CoreSdkContext::open(&fixture.root, SOFIA).expect("sdk context");
        let development = context.development();
        development
            .record_preference_evidence("reading", 0.7, 0.8, "test")
            .expect("preference");
        let reflection = development.reflect().expect("reflection");
        assert!(reflection.summary.contains("reading"));
        let proposal = development
            .propose_personality_change(
                "grow from reading",
                vec!["curiosity".to_owned()],
                traits(&[("openness", 0.3)]),
            )
            .expect("proposal");
        assert_eq!(proposal.status, PersonalityProposalStatus::Proposed);
    }

    let context = CoreSdkContext::open(&fixture.root, SOFIA).expect("reopen context");
    let development = context.development();
    let preference = development
        .get_preference("reading")
        .expect("get preference")
        .expect("preference exists");
    assert_eq!(preference.evidence_count, 1);
    assert_eq!(development.list_proposals(10).expect("proposals").len(), 1);
    assert!(development.list_events(10).expect("events").len() >= 3);
}

#[test]
fn reflection_is_bounded_and_uses_strongest_preferences() {
    let fixture = fixture();
    let context = CoreSdkContext::open(&fixture.root, SOFIA).expect("sdk context");
    let development = context.development();
    for (key, value) in [("quiet", 0.2_f32), ("music", 0.9), ("crowds", -0.95)] {
        development
            .record_preference_evidence(key, value, 1.0, "test")
            .expect("preference");
    }

    let reflection = development.reflect().expect("reflection");
    assert!(reflection.summary.contains("music"));
    assert!(reflection.summary.contains("crowds"));
    assert!(reflection.summary.chars().count() <= MAX_REFLECTION_SUMMARY_CHARS);
    assert_eq!(reflection.core_uuid, SOFIA);
    assert_eq!(reflection.schema, LIVING_CONTRACT_VERSION);
}

#[test]
fn apply_requires_owner_proof_and_changes_personality_growth() {
    let fixture = fixture();
    let context = CoreSdkContext::open(&fixture.root, SOFIA).expect("sdk context");
    let identity_before = CoreDatabase::read_snapshot(&layout(&fixture.root), SOFIA)
        .expect("core snapshot")
        .identity;
    let development = context.development();
    let proposal = development
        .propose_personality_change(
            "grow from repeated reflection",
            vec!["curiosity".to_owned()],
            traits(&[("openness", 0.42)]),
        )
        .expect("proposal");

    let growth_before = context.memory.personality_growth().expect("growth").len();
    assert!(matches!(
        development.apply_proposal("not-the-owner-token", &proposal.proposal_id),
        Err(SdkError::Unauthorized)
    ));
    assert_eq!(
        context.memory.personality_growth().expect("growth").len(),
        growth_before
    );
    assert_eq!(
        development
            .get_proposal(&proposal.proposal_id)
            .expect("get proposal")
            .expect("proposal exists")
            .status,
        PersonalityProposalStatus::Proposed
    );

    let applied = development
        .apply_proposal(&fixture.owner_token, &proposal.proposal_id)
        .expect("apply proposal");
    let openness = applied.traits.get("openness").copied().expect("trait");
    assert!((openness - 0.42).abs() < 1e-6);
    assert!(applied.values.iter().any(|value| value == "curiosity"));
    assert!(context.memory.personality_growth().expect("growth").len() >= 2);
    assert_eq!(
        development
            .get_proposal(&proposal.proposal_id)
            .expect("get proposal")
            .expect("proposal exists")
            .status,
        PersonalityProposalStatus::Applied
    );

    let identity_after = CoreDatabase::read_snapshot(&layout(&fixture.root), SOFIA)
        .expect("core snapshot")
        .identity;
    assert_eq!(identity_before, identity_after);
    assert_eq!(context.identity(), &identity_after);
}

#[test]
fn proposals_can_be_rejected_and_rejected_changes_do_not_apply() {
    let fixture = fixture();
    let context = CoreSdkContext::open(&fixture.root, SOFIA).expect("sdk context");
    let development = context.development();
    let proposal = development
        .propose_personality_change(
            "a change to reject",
            vec!["patience".to_owned()],
            traits(&[("calmness", 0.2)]),
        )
        .expect("proposal");

    assert!(matches!(
        development.reject_proposal("wrong-token", &proposal.proposal_id),
        Err(SdkError::Unauthorized)
    ));
    let rejected = development
        .reject_proposal(&fixture.owner_token, &proposal.proposal_id)
        .expect("reject");
    assert_eq!(rejected.status, PersonalityProposalStatus::Rejected);
    assert!(matches!(
        development.apply_proposal(&fixture.owner_token, &proposal.proposal_id),
        Err(SdkError::InvalidMemory(_))
    ));
    assert!(matches!(
        development.reject_proposal(&fixture.owner_token, "missing-proposal"),
        Err(SdkError::NotFound)
    ));
    assert!(context
        .memory
        .personality_growth()
        .expect("growth")
        .is_empty());
}

#[test]
fn revert_restores_previous_personality_and_requires_owner_proof() {
    let fixture = fixture();
    let context = CoreSdkContext::open(&fixture.root, SOFIA).expect("sdk context");
    let development = context.development();

    let mut baseline = PersonalityProfile::neutral(SOFIA).expect("neutral profile");
    baseline.tone = "calm".to_owned();
    let baseline = context
        .memory
        .store_personality(&baseline)
        .expect("store baseline");

    let proposal = development
        .propose_personality_change(
            "warm up",
            vec!["kindness".to_owned()],
            traits(&[("warmth", 0.5)]),
        )
        .expect("proposal");
    let applied = development
        .apply_proposal(&fixture.owner_token, &proposal.proposal_id)
        .expect("apply");
    assert!(applied.traits.contains_key("warmth"));
    let growth_after_apply = context.memory.personality_growth().expect("growth").len();

    assert!(matches!(
        development.revert_personality("wrong-token"),
        Err(SdkError::Unauthorized)
    ));
    assert_eq!(
        context.memory.personality_growth().expect("growth").len(),
        growth_after_apply
    );

    let restored = development
        .revert_personality(&fixture.owner_token)
        .expect("revert");
    assert!(!restored.traits.contains_key("warmth"));
    assert_eq!(restored.values, baseline.values);
    assert_eq!(restored.tone, baseline.tone);
    assert_eq!(
        context.memory.personality_growth().expect("growth").len(),
        growth_after_apply + 1
    );
}

#[test]
fn mack_cannot_see_sofia_preferences_and_foreign_db_copy_is_rejected() {
    let fixture = fixture();
    {
        let sofia = CoreSdkContext::open(&fixture.root, SOFIA).expect("Sofía context");
        let mack = CoreSdkContext::open(&fixture.root, MACK).expect("Mack context");

        sofia
            .development()
            .record_preference_evidence("sofia.only", 0.8, 0.9, "test")
            .expect("Sofía preference");
        mack.development()
            .record_preference_evidence("mack.only", -0.5, 0.7, "test")
            .expect("Mack preference");

        assert!(mack
            .development()
            .get_preference("sofia.only")
            .expect("get")
            .is_none());
        let mack_preferences = mack.development().list_preferences(10).expect("list");
        assert_eq!(mack_preferences.len(), 1);
        assert_eq!(mack_preferences[0].key, "mack.only");
        let sofia_preferences = sofia.development().list_preferences(10).expect("list");
        assert_eq!(sofia_preferences.len(), 1);
        assert_eq!(sofia_preferences[0].key, "sofia.only");
    }

    fs::copy(
        personal_path(&fixture.root, SOFIA),
        personal_path(&fixture.root, MACK),
    )
    .expect("copy foreign personal db");
    assert!(matches!(
        DevelopmentStore::open(&fixture.root, MACK),
        Err(SdkError::OwnershipMismatch)
    ));
    assert!(matches!(
        CoreSdkContext::open(&fixture.root, MACK),
        Err(SdkError::OwnershipMismatch)
    ));
}

#[test]
fn development_tables_and_append_only_events_are_enforced() {
    let fixture = fixture();
    let context = CoreSdkContext::open(&fixture.root, SOFIA).expect("sdk context");
    let development = context.development();
    development
        .record_experience("A quiet morning", 0.4, 0.6, "test")
        .expect("experience");

    let path = development.database_path().to_path_buf();
    let connection = Connection::open(&path).expect("raw personal db");
    for table in [
        DEVELOPMENT_PREFERENCES_TABLE,
        DEVELOPMENT_EVIDENCE_TABLE,
        DEVELOPMENT_REFLECTIONS_TABLE,
        DEVELOPMENT_PROPOSALS_TABLE,
        DEVELOPMENT_EVENTS_TABLE,
    ] {
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |row| row.get(0),
            )
            .expect("table count");
        assert_eq!(count, 1, "missing table {table}");
    }
    assert!(connection
        .execute("UPDATE development_events SET action = 'tampered'", [])
        .is_err());
    assert!(connection
        .execute("DELETE FROM development_events", [])
        .is_err());
    drop(connection);

    assert_eq!(development.list_events(10).expect("events").len(), 1);
    let experiences = context
        .personal
        .continuity_summary()
        .expect("continuity")
        .recent_experiences;
    assert!(experiences
        .iter()
        .any(|summary| summary.contains("A quiet morning")));
}

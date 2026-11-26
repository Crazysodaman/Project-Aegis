use std::path::PathBuf;

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_core_install::CoreInstaller;
use aegis_core_sdk::personal::{
    EmotionDelta, EmotionDimensions, EmotionInfluence, PersonalStore, RelationshipEmotionEvent,
    EXPRESSED_EMOTION_GAIN, EXPRESSED_EMOTION_SCHEMA,
};
use aegis_platform::PlatformService;
use aegis_storage::now_rfc3339;
use chrono::Utc;

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
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
        .initialize(&request(CORE_A, 1, "Core A"), &initialized.owner_token)
        .expect("install A");
    Fixture { _temp: temp, root }
}

fn influence(source: &str, weight: f32, joy: f32) -> EmotionInfluence {
    EmotionInfluence {
        source: source.to_owned(),
        weight,
        delta: EmotionDelta {
            valence: 0.0,
            arousal: 0.0,
            joy,
            calm: 0.0,
            curiosity: 0.0,
            tension: 0.0,
        },
    }
}

fn relationship_event(trust: u8, joy: f32) -> RelationshipEmotionEvent {
    RelationshipEmotionEvent {
        relationship_id: "rel-1".to_owned(),
        event_type: "kindness".to_owned(),
        trust,
        delta: EmotionDelta {
            valence: 0.0,
            arousal: 0.0,
            joy,
            calm: 0.0,
            curiosity: 0.0,
            tension: 0.0,
        },
    }
}

fn approx(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 1e-4,
        "expected {expected}, found {actual}"
    );
}

#[test]
fn expressed_emotion_projects_persisted_state() {
    let fixture = fixture();
    let store = PersonalStore::open(&fixture.root, CORE_A).expect("store");
    let now = Utc::now();
    let state = store
        .apply_influences_at(&[influence("praise", 0.9, 0.7)], now)
        .expect("influence");

    let expressed = store.expressed_emotion().expect("expressed");
    let again = store.expressed_emotion().expect("expressed again");

    assert_eq!(expressed, again);
    assert_eq!(expressed.schema, EXPRESSED_EMOTION_SCHEMA);
    assert_eq!(expressed.core_uuid, CORE_A);
    assert_eq!(expressed.internal_revision, state.revision);
    approx(
        expressed.dimensions.joy,
        state.dimensions.joy * EXPRESSED_EMOTION_GAIN,
    );
    assert!(expressed.intensity <= state.intensity + f32::EPSILON);
    assert_eq!(store.emotion_state().expect("internal"), state);
    assert!(store.expressed_emotion_with_gain(2.0).is_err());
}

#[test]
fn relationship_event_influence_applies_and_persists() {
    let fixture = fixture();
    let now = Utc::now();
    let event = relationship_event(25, 0.8);

    {
        let store = PersonalStore::open(&fixture.root, CORE_A).expect("store");
        let state = store
            .apply_relationship_event_at(&event, now)
            .expect("relationship influence");
        approx(state.dimensions.joy, 0.2);
        let history = store.emotion_history(1).expect("history");
        assert_eq!(history.len(), 1);
        assert!(history[0].source.starts_with("relationship:rel-1:kindness"));
    }

    let reopened = PersonalStore::open(&fixture.root, CORE_A).expect("reopen");
    let history = reopened.emotion_history(1).expect("history");
    approx(history[0].state.dimensions.joy, 0.2);
    assert_eq!(history[0].state.revision, 2);
}

#[test]
fn startup_is_neutral_and_never_randomized() {
    let fixture = fixture();
    let (first_state, first_history) = {
        let store = PersonalStore::open(&fixture.root, CORE_A).expect("store");
        (
            store.emotion_state().expect("state"),
            store.emotion_history(10).expect("history"),
        )
    };

    assert_eq!(first_state.dimensions, EmotionDimensions::neutral());
    assert_eq!(first_state.intensity, 0.0);
    assert_eq!(first_state.revision, 1);
    assert!(first_history.is_empty());

    for _ in 0..3 {
        let reopened = PersonalStore::open(&fixture.root, CORE_A).expect("reopen");
        assert_eq!(reopened.emotion_state().expect("state"), first_state);
        assert_eq!(
            reopened.emotion_history(10).expect("history"),
            first_history
        );
    }
}

//! Agent 07: deterministic, idempotent event → experience → emotion pipeline.

use std::path::{Path, PathBuf};

use aegis_cognition::{
    CancelToken, ChatChunk, ChatRequest, ChatResponse, InferenceError, ModelInfo, ModelProvider,
    ProviderHealth, ProviderId, ProviderKind,
};
use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_conversation::{ConversationEngine, TurnOptions};
use aegis_core_install::CoreInstaller;
use aegis_core_sdk::{
    apply_interaction_event, InteractionEvent, PersonalStore, INTERACTION_DELTA,
    INTERACTION_INFLUENCE_WEIGHT,
};
use aegis_platform::PlatformService;
use aegis_storage::now_rfc3339;

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_B: &str = "11111111-1111-4111-8111-111111111111";

struct FakeProvider {
    id: ProviderId,
    reply: String,
}

impl FakeProvider {
    fn new(reply: &str) -> Self {
        Self {
            id: ProviderId::new("event.local").expect("provider id"),
            reply: reply.to_owned(),
        }
    }
}

impl ModelProvider for FakeProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::Local
    }

    fn health(&self) -> ProviderHealth {
        ProviderHealth {
            reachable: true,
            version: Some("test".to_owned()),
            model_count: 1,
            latency_ms: 0,
            error: None,
        }
    }

    fn list_models(&self) -> Result<Vec<ModelInfo>, InferenceError> {
        Ok(vec![ModelInfo {
            name: "test-model".to_owned(),
            ..ModelInfo::default()
        }])
    }

    fn chat(
        &self,
        request: &ChatRequest,
        cancel: &CancelToken,
    ) -> Result<ChatResponse, InferenceError> {
        if cancel.is_cancelled() {
            return Err(InferenceError::Cancelled);
        }
        Ok(ChatResponse {
            model: request.model.clone(),
            content: self.reply.clone(),
            done: true,
            total_duration_ms: 1,
            prompt_eval_count: 4,
            eval_count: 6,
            tool_calls: Vec::new(),
        })
    }

    fn chat_stream(
        &self,
        _request: &ChatRequest,
        _cancel: &CancelToken,
        _on_chunk: &mut dyn FnMut(ChatChunk) -> Result<(), InferenceError>,
    ) -> Result<ChatResponse, InferenceError> {
        Err(InferenceError::Provider(
            "streaming is not used in this test".to_owned(),
        ))
    }
}

fn setup() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("aegis-root");
    let initialized = PlatformService::initialize(&root).expect("platform");
    let installer = CoreInstaller::new(&root).expect("installer");
    for (uuid, number, name) in [(CORE_A, 1, "Core A"), (CORE_B, 2, "Core B")] {
        installer
            .initialize(
                &CoreInitializationRequest {
                    schema: CORE_INIT_REQUEST_SCHEMA.to_owned(),
                    core_uuid: uuid.to_owned(),
                    core_number: number,
                    display_name: name.to_owned(),
                    requested_at: now_rfc3339().expect("time"),
                },
                &initialized.owner_token,
            )
            .expect("core");
    }
    (temp, root)
}

fn open_engine(root: &Path, core: &str) -> ConversationEngine {
    ConversationEngine::open(root, core).expect("engine")
}

fn approx(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 1e-4,
        "expected {expected}, found {actual}"
    );
}

#[test]
fn applying_the_same_event_twice_does_not_duplicate_influence_or_experience() {
    let (_temp, root) = setup();
    let store = PersonalStore::open(&root, CORE_A).expect("store");
    let event = InteractionEvent::interaction("event-1", "A single bounded experience");

    let first = apply_interaction_event(&store, &event).expect("first apply");
    assert!(first.applied);
    approx(
        first.emotion.dimensions.joy,
        INTERACTION_DELTA.joy * INTERACTION_INFLUENCE_WEIGHT,
    );
    assert_eq!(store.emotion_history(10).expect("history").len(), 1);
    assert_eq!(
        store
            .continuity_summary()
            .expect("summary")
            .recent_experiences,
        vec!["A single bounded experience".to_owned()]
    );

    let second = apply_interaction_event(&store, &event).expect("second apply");
    assert!(!second.applied);
    assert_eq!(second.emotion, first.emotion);
    assert_eq!(store.emotion_state().expect("state"), first.emotion);
    assert_eq!(store.emotion_history(10).expect("history").len(), 1);
    assert_eq!(
        store
            .continuity_summary()
            .expect("summary")
            .recent_experiences
            .len(),
        1
    );
}

#[test]
fn the_same_event_id_is_independent_per_core() {
    let (_temp, root) = setup();
    let store_a = PersonalStore::open(&root, CORE_A).expect("store a");
    let store_b = PersonalStore::open(&root, CORE_B).expect("store b");

    let event_a = InteractionEvent::interaction("shared-event", "experience for A");
    let event_b = InteractionEvent::interaction("shared-event", "experience for B");

    let applied_a = apply_interaction_event(&store_a, &event_a).expect("apply a");
    assert!(applied_a.applied);
    let untouched_b = store_b.emotion_state().expect("state b");
    assert_eq!(untouched_b.core_uuid, CORE_B);
    assert_eq!(untouched_b.intensity, 0.0);
    assert!(store_b
        .continuity_summary()
        .expect("summary b")
        .recent_experiences
        .is_empty());

    let applied_b = apply_interaction_event(&store_b, &event_b).expect("apply b");
    assert!(applied_b.applied);
    assert_eq!(store_a.emotion_state().expect("state a"), applied_a.emotion);
    assert_eq!(store_b.emotion_state().expect("state b"), applied_b.emotion);
    assert_eq!(
        store_a
            .continuity_summary()
            .expect("summary a")
            .recent_experiences,
        vec!["experience for A".to_owned()]
    );
    assert_eq!(
        store_b
            .continuity_summary()
            .expect("summary b")
            .recent_experiences,
        vec!["experience for B".to_owned()]
    );
}

#[test]
fn experience_and_emotion_persist_and_remain_idempotent_after_reopen() {
    let (_temp, root) = setup();
    let event = InteractionEvent::interaction("persisted-event", "Persisted bounded experience");

    let first = {
        let store = PersonalStore::open(&root, CORE_A).expect("store");
        let application = apply_interaction_event(&store, &event).expect("apply");
        assert!(application.applied);
        application.emotion
    };

    let reopened = PersonalStore::open(&root, CORE_A).expect("reopened store");
    assert_eq!(reopened.emotion_state().expect("state"), first);
    assert_eq!(
        reopened
            .continuity_summary()
            .expect("summary")
            .recent_experiences,
        vec!["Persisted bounded experience".to_owned()]
    );

    let replay = apply_interaction_event(&reopened, &event).expect("replay");
    assert!(!replay.applied);
    assert_eq!(reopened.emotion_state().expect("state"), first);
    assert_eq!(
        reopened
            .continuity_summary()
            .expect("summary")
            .recent_experiences
            .len(),
        1
    );
}

#[test]
fn engine_applies_the_interaction_event_once_per_assistant_message() {
    let (_temp, root) = setup();
    let engine = open_engine(&root, CORE_A);
    let session = engine.create_session(None).expect("session");

    let turn = engine
        .send_turn(
            &session.session_id,
            "Hello",
            &FakeProvider::new("Hello back."),
            &TurnOptions::new("test-model"),
            &CancelToken::new(),
        )
        .expect("turn");
    assert!(turn.interaction_applied);
    assert!(turn.interaction_warning.is_none());

    let personal = engine.sdk().personal().expect("personal");
    let before = personal.emotion_state().expect("state");
    let event = InteractionEvent::for_message(&session.session_id, turn.assistant.message_id);
    let replay = engine.apply_interaction_event(&event).expect("replay");
    assert!(!replay.applied);
    assert_eq!(personal.emotion_state().expect("state"), before);

    let summary = personal.continuity_summary().expect("summary");
    assert_eq!(summary.recent_experiences.len(), 1);
    assert!(summary.recent_experiences[0].contains("assistant message"));
}

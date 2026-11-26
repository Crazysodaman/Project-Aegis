use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};
use std::time::Instant;

use aegis_cognition::{
    CancelToken, ChatChunk, ChatRequest, ChatResponse, InferenceError, ModelInfo, ModelProvider,
    ProviderHealth, ProviderId, ProviderKind,
};
use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_contracts::personal::{
    Activity, ActivityStatus, ContinuitySummary, EmotionState, CONTINUITY_SUMMARY_SCHEMA,
};
use aegis_contracts::world::{
    FreshnessState, Provenance, Season, TimeOfDay, WeatherObservation, WorldSnapshot,
    WORLD_SNAPSHOT_SCHEMA,
};
use aegis_conversation::{
    ConversationEngine, MessageRole, TurnOptions, DEFAULT_HISTORY_LIMIT, MAX_MESSAGE_CHARS,
    MAX_SYSTEM_PROMPT_CHARS,
};
use aegis_core_install::CoreInstaller;
use aegis_core_sdk::{MemoryKind, NewMemory, PersonalityProfile};
use aegis_platform::PlatformService;
use aegis_storage::now_rfc3339;

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_B: &str = "11111111-1111-4111-8111-111111111111";
const TIMESTAMP: &str = "2026-11-26T12:00:00+00:00";

struct FakeProvider {
    id: ProviderId,
    reply: String,
}

impl FakeProvider {
    fn new(reply: &str) -> Self {
        Self {
            id: ProviderId::new("reliability.local").expect("provider id"),
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

fn world() -> WorldSnapshot {
    WorldSnapshot {
        schema: WORLD_SNAPSHOT_SCHEMA.to_owned(),
        generated_at: TIMESTAMP.to_owned(),
        timezone: "America/Chicago".to_owned(),
        utc_offset_seconds: -6 * 3600,
        local_date: "2026-11-26".to_owned(),
        local_time: "06:15:00".to_owned(),
        time_of_day: TimeOfDay::Dawn,
        season: Season::Autumn,
        sun: None,
        weather: Some(WeatherObservation {
            condition: "clear".to_owned(),
            temperature_celsius: Some(8.0),
            feels_like_celsius: Some(6.5),
            humidity_percent: Some(62),
            wind_speed_kph: Some(9.0),
            observed_at: Some("2026-11-26T11:55:00+00:00".to_owned()),
        }),
        forecast: Vec::new(),
        calendar: Vec::new(),
        provenance: Provenance {
            source: "platform_clock".to_owned(),
            provider: Some("test".to_owned()),
            retrieved_at: Some(TIMESTAMP.to_owned()),
            note: None,
        },
        freshness: FreshnessState::Fresh,
        stale_after_seconds: 900,
    }
}

fn continuity(core: &str) -> ContinuitySummary {
    ContinuitySummary {
        schema: CONTINUITY_SUMMARY_SCHEMA.to_owned(),
        core_uuid: core.to_owned(),
        last_active_at: "2026-11-26T10:00:00+00:00".to_owned(),
        last_session_id: None,
        elapsed_offline_seconds: 7200,
        recent_experiences: vec!["reviewed the reliability backlog".to_owned()],
        open_threads: vec!["finish bounded prompt regression".to_owned()],
        activities: Vec::new(),
        updated_at: TIMESTAMP.to_owned(),
    }
}

fn emotion(core: &str) -> EmotionState {
    let mut state = EmotionState::neutral(core, TIMESTAMP).expect("emotion");
    state.dimensions.arousal = 0.3;
    state.dimensions.joy = 0.4;
    state.dimensions.tension = 0.1;
    state.intensity = 0.4;
    state.validate().expect("valid emotion");
    state
}

fn activity(title: &str) -> Activity {
    Activity {
        id: format!("activity-{}", title.len()),
        title: title.to_owned(),
        status: ActivityStatus::Active,
        started_at: "2026-11-26T11:00:00+00:00".to_owned(),
        updated_at: TIMESTAMP.to_owned(),
        notes: String::new(),
    }
}

fn set_personality(engine: &ConversationEngine, core: &str, tone: &str) {
    let mut profile = PersonalityProfile::neutral(core).expect("profile");
    profile.tone = tone.to_owned();
    profile.values = vec!["honesty".to_owned()];
    engine
        .sdk()
        .memory()
        .save_personality(&profile)
        .expect("personality");
}

#[test]
fn repeated_context_injection_keeps_prompt_bounded_while_history_grows() {
    let (_temp, root) = setup();
    let engine = open_engine(&root, CORE_A);
    set_personality(&engine, CORE_A, "warm and precise");
    let filler = "context about the aegis project reliability work ".repeat(60);
    for index in 0..12 {
        engine
            .sdk()
            .memory()
            .create(NewMemory::new(
                MemoryKind::Fact,
                format!("memory {index} about project context {filler}"),
            ))
            .expect("memory");
    }
    let session = engine.create_session(Some("bounded")).expect("session");
    let provider = FakeProvider::new("acknowledged");
    let options = TurnOptions::new("test-model")
        .with_history_limit(20)
        .with_memory_limit(4)
        .with_world(world())
        .with_continuity(continuity(CORE_A))
        .with_emotion(emotion(CORE_A))
        .with_activities(vec![activity("guarding the prompt bound")]);

    let turns = 40;
    let mut lengths = Vec::with_capacity(turns);
    let mut message_counts = Vec::with_capacity(turns);
    let started = Instant::now();
    for _ in 0..turns {
        let turn = engine
            .send_turn(
                &session.session_id,
                "Tell me about the project context.",
                &provider,
                &options,
                &CancelToken::new(),
            )
            .expect("turn");
        assert_eq!(turn.assistant.content, "acknowledged");
        let prepared = engine
            .build_request(&session.session_id, &options)
            .expect("prepared");
        let length = prepared.system_prompt.chars().count();
        assert!(length <= MAX_SYSTEM_PROMPT_CHARS);
        assert!(prepared.request.messages.len() <= options.history_limit + 1);
        assert!(prepared.world_used);
        assert!(prepared.continuity_used);
        assert!(prepared.emotion_used);
        assert!(prepared.activities_used);
        lengths.push(length);
        message_counts.push(prepared.request.messages.len());
    }
    let elapsed = started.elapsed();

    let minimum = *lengths.iter().min().expect("length");
    let maximum = *lengths.iter().max().expect("length");
    assert_eq!(
        minimum, maximum,
        "repeated context injection must not grow the prompt"
    );
    assert_eq!(
        *message_counts.last().expect("count"),
        options.history_limit + 1,
        "history is bounded by history_limit"
    );
    let stored = engine
        .store()
        .session(&session.session_id)
        .expect("stored session");
    assert_eq!(stored.message_count, (turns * 2) as u64);
    eprintln!("{turns} context-heavy turns in {elapsed:?}; prompt chars = {maximum}");
}

#[test]
fn large_but_bounded_user_and_instruction_inputs_are_capped() {
    let (_temp, root) = setup();
    let engine = open_engine(&root, CORE_A);
    let session = engine.create_session(None).expect("session");
    let provider = FakeProvider::new("ok");
    let large_user = "word ".repeat(12_000);
    let large_instructions = "i".repeat(30_000);
    let options = TurnOptions::new("test-model").with_system_instructions(large_instructions);

    let turn = engine
        .send_turn(
            &session.session_id,
            &large_user,
            &provider,
            &options,
            &CancelToken::new(),
        )
        .expect("large bounded turn");
    assert_eq!(turn.user.content.len(), large_user.len());
    let prepared = engine
        .build_request(&session.session_id, &options)
        .expect("prepared");
    assert!(prepared.system_prompt.chars().count() <= MAX_SYSTEM_PROMPT_CHARS);
    assert!(prepared.request.messages.len() <= DEFAULT_HISTORY_LIMIT + 1);
    assert!(prepared
        .request
        .messages
        .iter()
        .any(|message| message.content.len() == large_user.len()));

    let oversized = "z".repeat(MAX_MESSAGE_CHARS + 1);
    assert!(engine
        .send_turn(
            &session.session_id,
            &oversized,
            &provider,
            &options,
            &CancelToken::new(),
        )
        .is_err());
}

#[test]
fn single_long_token_leaves_an_orphan_user_message_after_retrieval_rejects_it() {
    let (_temp, root) = setup();
    let engine = open_engine(&root, CORE_A);
    let session = engine.create_session(None).expect("session");
    let provider = FakeProvider::new("never stored");
    let token = "u".repeat(60_000);
    assert!(token.len() < MAX_MESSAGE_CHARS);

    let result = engine.send_turn(
        &session.session_id,
        &token,
        &provider,
        &TurnOptions::new("test-model"),
        &CancelToken::new(),
    );
    assert!(
        result.is_err(),
        "a single over-long retrieval term is rejected after the user message is stored"
    );
    let messages = engine
        .store()
        .messages(&session.session_id, 10, 0)
        .expect("messages");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, MessageRole::User);
    assert_eq!(messages[0].content.len(), token.len());
}

#[test]
fn concurrent_multi_core_conversations_do_not_contaminate_each_other() {
    let (_temp, root) = setup();
    let barrier = Arc::new(Barrier::new(2));
    let turns = 5;
    let started = Instant::now();
    let mut handles = Vec::new();
    for (core, marker) in [(CORE_A, "alpha"), (CORE_B, "bravo")] {
        let root = root.clone();
        let barrier = Arc::clone(&barrier);
        let marker = marker.to_owned();
        handles.push(std::thread::spawn(move || {
            let engine = ConversationEngine::open(&root, core).expect("engine");
            set_personality(&engine, core, &format!("{marker} tone"));
            let session = engine.create_session(Some(&marker)).expect("session");
            let provider = FakeProvider::new(&format!("{marker} reply"));
            barrier.wait();
            for index in 0..turns {
                let turn = engine
                    .send_turn(
                        &session.session_id,
                        &format!("{marker} question {index}"),
                        &provider,
                        &TurnOptions::new("test-model"),
                        &CancelToken::new(),
                    )
                    .expect("turn");
                assert_eq!(turn.assistant.content, format!("{marker} reply"));
                assert_eq!(turn.session.core_uuid, core);
            }
            let export = engine
                .store()
                .export_session(&session.session_id)
                .expect("export");
            (core.to_owned(), session.session_id, export)
        }));
    }

    let mut session_a = None;
    for handle in handles {
        let (core, session_id, export) = handle.join().expect("core thread");
        assert_eq!(export.core_uuid, core);
        assert_eq!(export.messages.len(), turns * 2);
        let marker = if core == CORE_A { "alpha" } else { "bravo" };
        let other = if core == CORE_A { "bravo" } else { "alpha" };
        assert!(export
            .messages
            .iter()
            .all(|message| message.content.contains(marker)));
        assert!(export
            .messages
            .iter()
            .all(|message| !message.content.contains(other)));
        if core == CORE_A {
            session_a = Some(session_id);
        }
    }
    let elapsed = started.elapsed();

    let engine_b = open_engine(&root, CORE_B);
    assert!(engine_b
        .store()
        .session(&session_a.expect("session a"))
        .is_err());
    eprintln!("concurrent conversations: 2 cores x {turns} turns in {elapsed:?}");
}

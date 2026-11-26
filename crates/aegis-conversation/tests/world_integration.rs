use std::path::{Path, PathBuf};

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
use aegis_conversation::{ConversationEngine, TurnOptions, MAX_SYSTEM_PROMPT_CHARS};
use aegis_core_install::CoreInstaller;
use aegis_core_sdk::PersonalityProfile;
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
            id: ProviderId::new("test.local").expect("provider id"),
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
    for (uuid, number, name) in [(CORE_A, 1, "Sofía Ada Lyra"), (CORE_B, 2, "Mack Fuller")] {
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

fn world(freshness: FreshnessState, weather: Option<WeatherObservation>) -> WorldSnapshot {
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
        weather,
        forecast: Vec::new(),
        calendar: Vec::new(),
        provenance: Provenance {
            source: "platform_clock".to_owned(),
            provider: Some("test".to_owned()),
            retrieved_at: Some(TIMESTAMP.to_owned()),
            note: None,
        },
        freshness,
        stale_after_seconds: 900,
    }
}

fn weather() -> WeatherObservation {
    WeatherObservation {
        condition: "clear".to_owned(),
        temperature_celsius: Some(8.0),
        feels_like_celsius: Some(6.5),
        humidity_percent: Some(62),
        wind_speed_kph: Some(9.0),
        observed_at: Some("2026-11-26T11:55:00+00:00".to_owned()),
    }
}

fn continuity(core: &str, elapsed_offline_seconds: u64) -> ContinuitySummary {
    ContinuitySummary {
        schema: CONTINUITY_SUMMARY_SCHEMA.to_owned(),
        core_uuid: core.to_owned(),
        last_active_at: "2026-11-26T10:00:00+00:00".to_owned(),
        last_session_id: None,
        elapsed_offline_seconds,
        recent_experiences: Vec::new(),
        open_threads: Vec::new(),
        activities: Vec::new(),
        updated_at: TIMESTAMP.to_owned(),
    }
}

fn emotion(core: &str, joy: f32, tension: f32) -> EmotionState {
    let mut state = EmotionState::neutral(core, TIMESTAMP).expect("emotion");
    state.dimensions.arousal = 0.4;
    state.dimensions.joy = joy;
    state.dimensions.tension = tension;
    state.intensity = 0.5;
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

fn set_personality(engine: &ConversationEngine, core: &str, tone: &str, values: &[&str]) {
    let mut profile = PersonalityProfile::neutral(core).expect("profile");
    profile.tone = tone.to_owned();
    profile.values = values.iter().map(|value| (*value).to_owned()).collect();
    engine
        .sdk()
        .memory()
        .save_personality(&profile)
        .expect("personality");
}

#[test]
fn same_world_different_cores_produce_distinct_bounded_prompts() {
    let (_temp, root) = setup();
    let engine_a = open_engine(&root, CORE_A);
    set_personality(&engine_a, CORE_A, "warm and precise", &["honesty"]);
    let engine_b = open_engine(&root, CORE_B);
    set_personality(&engine_b, CORE_B, "dry and analytical", &["clarity"]);

    let session_a = engine_a.create_session(None).expect("session a");
    let session_b = engine_b.create_session(None).expect("session b");
    let shared_world = world(FreshnessState::Fresh, Some(weather()));

    let options_a = TurnOptions::new("test-model")
        .with_world(shared_world.clone())
        .with_emotion(emotion(CORE_A, 0.8, 0.1))
        .with_activities(vec![activity("reviewing prompts")]);
    let options_b = TurnOptions::new("test-model")
        .with_world(shared_world)
        .with_emotion(emotion(CORE_B, 0.2, 0.7))
        .with_activities(vec![activity("reviewing prompts")]);

    let prepared_a = engine_a
        .build_request(&session_a.session_id, &options_a)
        .expect("prepared a");
    let prepared_b = engine_b
        .build_request(&session_b.session_id, &options_b)
        .expect("prepared b");

    assert_ne!(prepared_a.system_prompt, prepared_b.system_prompt);
    assert!(prepared_a.system_prompt.contains("warm and precise"));
    assert!(prepared_b.system_prompt.contains("dry and analytical"));
    assert!(prepared_a.system_prompt.contains("Shared world context"));
    assert!(prepared_a.system_prompt.contains("America/Chicago"));
    assert!(prepared_a.world_used);
    assert!(prepared_a.emotion_used);
    assert!(prepared_a.activities_used);
    assert!(!prepared_a.continuity_used);
    assert!(prepared_b.world_used);
    assert!(prepared_b.emotion_used);
    assert!(prepared_b.activities_used);
    for prepared in [&prepared_a, &prepared_b] {
        assert!(prepared.system_prompt.chars().count() <= MAX_SYSTEM_PROMPT_CHARS);
    }
}

#[test]
fn stale_and_unavailable_world_still_produce_valid_prompts() {
    let (_temp, root) = setup();
    let engine = open_engine(&root, CORE_A);
    let session = engine.create_session(None).expect("session");

    let stale = world(FreshnessState::Stale, Some(weather()));
    let prepared = engine
        .build_request(
            &session.session_id,
            &TurnOptions::new("test-model").with_world(stale),
        )
        .expect("stale prompt");
    assert!(prepared.world_used);
    assert!(prepared.system_prompt.contains("stale"));
    assert!(prepared.system_prompt.contains("Weather: clear"));
    assert!(prepared
        .system_prompt
        .contains("may not reflect current conditions"));

    let unavailable = world(FreshnessState::Unavailable, None);
    let prepared = engine
        .build_request(
            &session.session_id,
            &TurnOptions::new("test-model").with_world(unavailable),
        )
        .expect("unavailable prompt");
    assert!(prepared.world_used);
    assert!(prepared.system_prompt.contains("unavailable"));
    assert!(prepared.system_prompt.contains("Weather: not provided"));
    assert!(!prepared.system_prompt.trim().is_empty());
    assert!(prepared.system_prompt.chars().count() <= MAX_SYSTEM_PROMPT_CHARS);
}

#[test]
fn prompt_length_is_capped_with_long_and_multibyte_inputs() {
    let (_temp, root) = setup();
    let engine = open_engine(&root, CORE_A);
    let session = engine.create_session(None).expect("session");

    let long = "🌍é".repeat(10_000);
    let mut summary = continuity(CORE_A, 7200);
    summary.recent_experiences = (0..8).map(|_| long.clone()).collect();
    summary.open_threads = (0..8).map(|_| long.clone()).collect();
    let options = TurnOptions::new("test-model")
        .with_world(world(FreshnessState::Fresh, Some(weather())))
        .with_continuity(summary)
        .with_emotion(emotion(CORE_A, 0.5, 0.5))
        .with_activities(vec![activity(&long)])
        .with_system_instructions(long);

    let prepared = engine
        .build_request(&session.session_id, &options)
        .expect("bounded prompt");
    let length = prepared.system_prompt.chars().count();
    assert!(!prepared.system_prompt.is_empty());
    assert!(length <= MAX_SYSTEM_PROMPT_CHARS);
    assert_eq!(length, MAX_SYSTEM_PROMPT_CHARS);
    assert!(prepared.system_prompt.ends_with('…'));
    assert!(prepared.world_used);
    assert!(prepared.continuity_used);
    assert!(prepared.emotion_used);
    assert!(prepared.activities_used);
}

#[test]
fn continuity_values_and_context_flags_are_included() {
    let (_temp, root) = setup();
    let engine = open_engine(&root, CORE_A);
    let session = engine.create_session(None).expect("session");

    let mut summary = continuity(CORE_A, 7200);
    summary.recent_experiences = vec!["worked on batch eight backup".to_owned()];
    summary.open_threads = vec!["finish world integration".to_owned()];
    summary.activities = vec![activity("reviewing prompts")];
    let options = TurnOptions::new("test-model")
        .with_continuity(summary)
        .with_activities(vec![activity("drafting report")]);

    let prepared = engine
        .build_request(&session.session_id, &options)
        .expect("prepared");
    assert!(prepared.continuity_used);
    assert!(prepared.activities_used);
    assert!(!prepared.world_used);
    assert!(!prepared.emotion_used);
    let prompt = &prepared.system_prompt;
    assert!(prompt.contains("7200"));
    assert!(prompt.contains("worked on batch eight backup"));
    assert!(prompt.contains("finish world integration"));
    assert!(prompt.contains("reviewing prompts"));
    assert!(prompt.contains("drafting report"));
    assert!(prompt.chars().count() <= MAX_SYSTEM_PROMPT_CHARS);
}

#[test]
fn successful_turn_updates_personal_continuity_and_reports_flags() {
    let (_temp, root) = setup();
    let engine = open_engine(&root, CORE_A);
    set_personality(&engine, CORE_A, "warm and precise", &["honesty"]);
    let session = engine.create_session(None).expect("session");
    let options = TurnOptions::new("test-model")
        .with_world(world(FreshnessState::Fresh, Some(weather())))
        .with_emotion(emotion(CORE_A, 0.7, 0.1))
        .with_activities(vec![activity("checking continuity")]);

    let turn = engine
        .send_turn(
            &session.session_id,
            "Hello",
            &FakeProvider::new("Hello back."),
            &options,
            &CancelToken::new(),
        )
        .expect("turn");
    assert!(turn.world_used);
    assert!(turn.emotion_used);
    assert!(turn.activities_used);
    assert!(!turn.continuity_used);
    assert!(turn.personal_update_warning.is_none());

    let summary = engine
        .sdk()
        .personal()
        .expect("personal")
        .continuity_summary()
        .expect("summary");
    assert_eq!(summary.last_session_id, Some(session.session_id.clone()));
}

#[test]
fn default_options_preserve_single_core_behavior() {
    let (_temp, root) = setup();
    let engine = open_engine(&root, CORE_A);
    set_personality(&engine, CORE_A, "warm and precise", &["honesty"]);
    let session = engine.create_session(None).expect("session");

    let prepared = engine
        .build_request(&session.session_id, &TurnOptions::new("test-model"))
        .expect("prepared");
    assert!(!prepared.world_used);
    assert!(!prepared.continuity_used);
    assert!(!prepared.emotion_used);
    assert!(!prepared.activities_used);
    assert!(prepared.system_prompt.contains("Sofía Ada Lyra"));
    assert!(prepared.system_prompt.contains("warm and precise"));
    assert!(!prepared.system_prompt.contains("Shared world context"));
    assert!(!prepared.system_prompt.contains("Continuity with this Core"));
    assert!(!prepared.system_prompt.contains("Internal emotional tone"));

    let turn = engine
        .send_turn(
            &session.session_id,
            "Hello",
            &FakeProvider::new("Default answer."),
            &TurnOptions::new("test-model"),
            &CancelToken::new(),
        )
        .expect("turn");
    assert_eq!(turn.assistant.content, "Default answer.");
    assert!(turn.personal_update_warning.is_none());
}

//! Batch 8 / Milestone 7 independent internal QA for conversation context,
//! bounded prompt composition, and restart/recovery across world, personal,
//! and conversation state.

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
    FreshnessState, Provenance, Season, SunTimes, TimeOfDay, WeatherForecast, WeatherObservation,
    WorldSnapshot, WORLD_SNAPSHOT_SCHEMA,
};
use aegis_conversation::{ConversationEngine, MessageRole, TurnOptions, MAX_SYSTEM_PROMPT_CHARS};
use aegis_core_install::CoreInstaller;
use aegis_core_sdk::PersonalityProfile;
use aegis_platform::{
    PlatformService, WeatherData, WeatherProvider, WeatherRequest, WeatherUnavailable, WorldService,
};
use aegis_storage::{now_rfc3339, StoragePaths};

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
            id: ProviderId::new("qa.local").expect("provider id"),
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
            version: Some("qa".to_owned()),
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
            "streaming is not used in this QA test".to_owned(),
        ))
    }
}

struct FixedWeatherProvider;

impl WeatherProvider for FixedWeatherProvider {
    fn name(&self) -> &'static str {
        "fixed_batch8_qa"
    }

    fn observe(&self, request: &WeatherRequest) -> Result<WeatherData, WeatherUnavailable> {
        Ok(WeatherData {
            observation: Some(WeatherObservation {
                condition: "clear".to_owned(),
                temperature_celsius: Some(18.0),
                feels_like_celsius: Some(17.5),
                humidity_percent: Some(45),
                wind_speed_kph: Some(4.0),
                observed_at: Some(request.now.to_rfc3339()),
            }),
            forecast: vec![WeatherForecast {
                at: request.now.format("%Y-%m-%d").to_string(),
                condition: "clear".to_owned(),
                high_celsius: Some(20.0),
                low_celsius: Some(10.0),
                precipitation_probability_percent: Some(10),
            }],
            sun: Some(SunTimes {
                sunrise: Some("06:40".to_owned()),
                sunset: Some("17:05".to_owned()),
            }),
        })
    }
}

fn paths(root: &Path) -> StoragePaths {
    StoragePaths::new(root).expect("storage paths")
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
            provider: Some("qa".to_owned()),
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
fn restart_recovery_preserves_world_personal_and_conversation_state() {
    let (_temp, root) = setup();

    let snapshot = {
        let world = WorldService::start(paths(&root)).expect("world");
        world
            .snapshot(&FixedWeatherProvider)
            .expect("world snapshot")
    };

    let (session_id, message_count) = {
        let engine = ConversationEngine::open(&root, CORE_A).expect("engine");
        let session = engine.create_session(Some("recovery")).expect("session");
        let options = TurnOptions::new("test-model")
            .with_world(snapshot.clone())
            .with_emotion(emotion(CORE_A, 0.6, 0.1))
            .with_activities(vec![activity("recovering")]);
        let turn = engine
            .send_turn(
                &session.session_id,
                "Hello before restart",
                &FakeProvider::new("Hello from before restart."),
                &options,
                &CancelToken::new(),
            )
            .expect("turn");
        assert!(turn.world_used);
        assert!(turn.emotion_used);
        assert!(turn.activities_used);
        assert!(turn.personal_update_warning.is_none());
        let messages = engine
            .store()
            .recent_messages(&session.session_id, 10)
            .expect("messages");
        (session.session_id, messages.len())
    };
    assert_eq!(message_count, 2);

    // Simulate a full process restart: reopen world, engine, and personal store.
    let reopened_world = WorldService::start(paths(&root)).expect("reopen world");
    assert_eq!(
        reopened_world
            .shared_snapshot()
            .expect("latest")
            .expect("persisted world"),
        snapshot
    );

    let engine = ConversationEngine::open(&root, CORE_A).expect("reopen engine");
    let session = engine
        .store()
        .session(&session_id)
        .expect("persisted session");
    assert_eq!(session.core_uuid, CORE_A);
    assert_eq!(session.message_count, 2);

    let messages = engine
        .store()
        .recent_messages(&session_id, 10)
        .expect("persisted messages");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, MessageRole::User);
    assert_eq!(messages[0].content, "Hello before restart");
    assert_eq!(messages[1].role, MessageRole::Assistant);
    assert_eq!(messages[1].content, "Hello from before restart.");

    let summary = engine
        .sdk()
        .personal()
        .expect("personal")
        .continuity_summary()
        .expect("continuity");
    assert_eq!(
        summary.last_session_id.as_deref(),
        Some(session_id.as_str())
    );
    assert_eq!(summary.core_uuid, CORE_A);
}

#[test]
fn same_world_with_distinct_personality_and_emotion_yields_distinct_bounded_prompts() {
    let (_temp, root) = setup();
    let engine_a = ConversationEngine::open(&root, CORE_A).expect("engine a");
    set_personality(&engine_a, CORE_A, "warm and precise", &["honesty"]);
    let engine_b = ConversationEngine::open(&root, CORE_B).expect("engine b");
    set_personality(&engine_b, CORE_B, "dry and analytical", &["clarity"]);

    let session_a = engine_a.create_session(None).expect("session a");
    let session_b = engine_b.create_session(None).expect("session b");
    let shared_world = world(FreshnessState::Fresh, Some(weather()));

    let options_a = TurnOptions::new("test-model")
        .with_world(shared_world.clone())
        .with_emotion(emotion(CORE_A, 0.9, 0.0))
        .with_activities(vec![activity("reviewing prompts")]);
    let options_b = TurnOptions::new("test-model")
        .with_world(shared_world)
        .with_emotion(emotion(CORE_B, 0.1, 0.8))
        .with_activities(vec![activity("reviewing prompts")]);

    let prepared_a = engine_a
        .build_request(&session_a.session_id, &options_a)
        .expect("prepared a");
    let prepared_b = engine_b
        .build_request(&session_b.session_id, &options_b)
        .expect("prepared b");

    assert_ne!(
        prepared_a.system_prompt, prepared_b.system_prompt,
        "the same world must still produce Core-specific prompts"
    );
    assert!(prepared_a.system_prompt.contains("warm and precise"));
    assert!(prepared_b.system_prompt.contains("dry and analytical"));
    for prepared in [&prepared_a, &prepared_b] {
        assert!(prepared.world_used);
        assert!(prepared.emotion_used);
        assert!(prepared.activities_used);
        assert!(!prepared.continuity_used);
        assert!(prepared.system_prompt.contains("Shared world context"));
        assert!(prepared.system_prompt.contains("America/Chicago"));
        assert!(prepared.system_prompt.chars().count() <= MAX_SYSTEM_PROMPT_CHARS);
    }
}

#[test]
fn unavailable_world_and_default_options_remain_valid_and_honest() {
    let (_temp, root) = setup();
    let engine = ConversationEngine::open(&root, CORE_A).expect("engine");
    let session = engine.create_session(None).expect("session");

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

    let continuity = ContinuitySummary {
        schema: CONTINUITY_SUMMARY_SCHEMA.to_owned(),
        core_uuid: CORE_A.to_owned(),
        last_active_at: "2026-11-26T10:00:00+00:00".to_owned(),
        last_session_id: None,
        elapsed_offline_seconds: 7200,
        recent_experiences: vec!["one bounded experience".to_owned()],
        open_threads: Vec::new(),
        activities: Vec::new(),
        updated_at: TIMESTAMP.to_owned(),
    };
    continuity.validate().expect("valid continuity");
    let prepared = engine
        .build_request(
            &session.session_id,
            &TurnOptions::new("test-model").with_continuity(continuity),
        )
        .expect("continuity prompt");
    assert!(prepared.continuity_used);
    assert!(prepared.system_prompt.contains("7200"));
    assert!(prepared.system_prompt.contains("one bounded experience"));

    let default = engine
        .build_request(&session.session_id, &TurnOptions::new("test-model"))
        .expect("default prompt");
    assert!(!default.world_used);
    assert!(!default.continuity_used);
    assert!(!default.emotion_used);
    assert!(!default.activities_used);
    assert!(!default.system_prompt.trim().is_empty());
}

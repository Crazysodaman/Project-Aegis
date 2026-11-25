use std::path::{Path, PathBuf};
use std::sync::Mutex;

use aegis_cognition::{
    CancelToken, ChatChunk, ChatRequest, ChatResponse, InferenceError, ModelInfo, ModelProvider,
    ProviderHealth, ProviderId, ProviderKind, ToolCall,
};
use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_conversation::{
    ConversationEngine, ConversationError, ConversationStore, EmbeddingRetriever, MemoryRetriever,
    MessageRole, NewMessage, SessionState, TurnEvent, TurnOptions,
};
use aegis_core_install::CoreInstaller;
use aegis_core_sdk::{
    CoreMemoryStore, MemoryEntry, MemoryKind, MemoryQuery, NewMemory, PersonalityProfile, SdkError,
};
use aegis_embeddings::{
    EmbeddingError, EmbeddingProvider, EmbeddingStore, HashingEmbeddingProvider,
};
use aegis_platform::PlatformService;
use aegis_storage::{now_rfc3339, StoragePaths};
use aegis_tools::{ToolExecutor, ToolRegistry};
use serde_json::json;

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_B: &str = "11111111-1111-4111-8111-111111111111";

struct FakeProvider {
    id: ProviderId,
    reply: String,
    fail: bool,
    tool_calls: Vec<ToolCall>,
    requests: Mutex<Vec<ChatRequest>>,
}

impl FakeProvider {
    fn new(reply: &str) -> Self {
        Self {
            id: ProviderId::new("test.local").expect("provider id"),
            reply: reply.to_owned(),
            fail: false,
            tool_calls: Vec::new(),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn failing() -> Self {
        Self {
            fail: true,
            ..Self::new("")
        }
    }

    fn with_tools(reply: &str, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            tool_calls,
            ..Self::new(reply)
        }
    }

    fn last_request(&self) -> Option<ChatRequest> {
        self.requests.lock().ok()?.last().cloned()
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
            reachable: !self.fail,
            version: Some("test".to_owned()),
            model_count: 1,
            latency_ms: 0,
            error: self.fail.then(|| "configured failure".to_owned()),
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
        self.requests
            .lock()
            .expect("requests")
            .push(request.clone());
        if self.fail {
            return Err(InferenceError::Provider("configured failure".to_owned()));
        }
        Ok(ChatResponse {
            model: request.model.clone(),
            content: self.reply.clone(),
            done: true,
            total_duration_ms: 1,
            prompt_eval_count: 4,
            eval_count: 6,
            tool_calls: self.tool_calls.clone(),
        })
    }

    fn chat_stream(
        &self,
        request: &ChatRequest,
        cancel: &CancelToken,
        on_chunk: &mut dyn FnMut(ChatChunk) -> Result<(), InferenceError>,
    ) -> Result<ChatResponse, InferenceError> {
        if cancel.is_cancelled() {
            return Err(InferenceError::Cancelled);
        }
        self.requests
            .lock()
            .expect("requests")
            .push(request.clone());
        if self.fail {
            return Err(InferenceError::Provider("configured failure".to_owned()));
        }
        let characters: Vec<char> = self.reply.chars().collect();
        for (index, chunk) in characters.chunks(3).enumerate() {
            on_chunk(ChatChunk {
                model: request.model.clone(),
                delta: chunk.iter().collect(),
                done: index + 1 == characters.len().div_ceil(3),
                tool_calls: Vec::new(),
            })?;
        }
        Ok(ChatResponse {
            model: request.model.clone(),
            content: self.reply.clone(),
            done: true,
            total_duration_ms: 1,
            prompt_eval_count: 4,
            eval_count: 6,
            tool_calls: self.tool_calls.clone(),
        })
    }
}

fn setup() -> (tempfile::TempDir, PathBuf, String) {
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
    (temp, root, initialized.owner_token)
}

fn open_engine(root: &Path, core: &str) -> ConversationEngine {
    ConversationEngine::open(root, core).expect("engine")
}

#[test]
fn turns_persist_across_reopen() {
    let (_temp, root, _token) = setup();
    let engine = open_engine(&root, CORE_A);
    let session = engine.create_session(Some("First")).expect("session");
    let provider = FakeProvider::new("Hello from the model.");
    let turn = engine
        .send_turn(
            &session.session_id,
            "Hello",
            &provider,
            &TurnOptions::new("test-model"),
            &CancelToken::new(),
        )
        .expect("turn");
    assert_eq!(turn.user.role, MessageRole::User);
    assert_eq!(turn.assistant.content, "Hello from the model.");
    assert_eq!(turn.assistant.prompt_tokens, Some(4));
    assert_eq!(turn.session.message_count, 2);

    let reopened = open_engine(&root, CORE_A);
    let messages = reopened
        .store()
        .messages(&session.session_id, 10, 0)
        .expect("messages");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, MessageRole::User);
    assert_eq!(messages[1].content, "Hello from the model.");
    let export = reopened
        .store()
        .export_session(&session.session_id)
        .expect("export");
    assert_eq!(export.messages.len(), 2);
    assert_eq!(export.core_uuid, CORE_A);
}

#[test]
fn memories_are_included_and_personality_is_descriptive() {
    let (_temp, root, _token) = setup();
    let engine = open_engine(&root, CORE_A);
    engine
        .sdk()
        .memory()
        .create(NewMemory::new(
            MemoryKind::Preference,
            "The owner prefers tea over coffee.",
        ))
        .expect("memory");
    let mut personality = PersonalityProfile::neutral(CORE_A).expect("profile");
    personality.tone = "warm and precise".to_owned();
    personality.values = vec!["honesty".to_owned()];
    engine
        .sdk()
        .memory()
        .save_personality(&personality)
        .expect("personality");

    let session = engine.create_session(Some("Memory")).expect("session");
    engine
        .store()
        .append_message(
            &session.session_id,
            NewMessage::new(MessageRole::User, "What does the owner drink?"),
        )
        .expect("append");
    let prepared = engine
        .build_request(&session.session_id, &TurnOptions::new("test-model"))
        .expect("prepared");
    assert!(prepared.system_prompt.contains("warm and precise"));
    assert!(prepared.system_prompt.contains("honesty"));
    assert!(prepared.system_prompt.contains("tea over coffee"));
    assert_eq!(prepared.memory_used.len(), 1);
    assert!(!prepared.memory_used[0].snippet.is_empty());
    assert_eq!(
        prepared.request.messages[0].role,
        aegis_cognition::ChatRole::System
    );
}

#[test]
fn streaming_emits_events_and_persists_reply() {
    let (_temp, root, _token) = setup();
    let engine = open_engine(&root, CORE_A);
    let session = engine.create_session(None).expect("session");
    let provider = FakeProvider::new("streamed answer");
    let mut events = Vec::new();
    let turn = engine
        .send_turn_streaming(
            &session.session_id,
            "Stream please",
            &provider,
            &TurnOptions::new("test-model"),
            &CancelToken::new(),
            &mut |event| {
                events.push(event);
                Ok(())
            },
        )
        .expect("stream");
    assert_eq!(turn.assistant.content, "streamed answer");
    assert!(events
        .iter()
        .any(|event| matches!(event, TurnEvent::MemoryRetrieved { .. })));
    let streamed: String = events
        .iter()
        .filter_map(|event| match event {
            TurnEvent::Delta { text } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(streamed, "streamed answer");
    assert!(events
        .iter()
        .any(|event| matches!(event, TurnEvent::Completed { .. })));
    let messages = engine
        .store()
        .messages(&session.session_id, 10, 0)
        .expect("messages");
    assert_eq!(messages.len(), 2);
}

#[test]
fn inference_failure_is_recorded_without_assistant_message() {
    let (_temp, root, _token) = setup();
    let engine = open_engine(&root, CORE_A);
    let session = engine.create_session(None).expect("session");
    let error = engine
        .send_turn(
            &session.session_id,
            "Will fail",
            &FakeProvider::failing(),
            &TurnOptions::new("test-model"),
            &CancelToken::new(),
        )
        .expect_err("must fail");
    assert!(matches!(error, ConversationError::Inference(_)));
    let messages = engine
        .store()
        .messages(&session.session_id, 10, 0)
        .expect("messages");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, MessageRole::User);
    let events = engine
        .store()
        .session_events(&session.session_id)
        .expect("events");
    assert!(events.iter().any(|event| event.event_type == "turn.failed"));
}

#[test]
fn tool_calls_are_stored_as_data() {
    let (_temp, root, _token) = setup();
    let engine = open_engine(&root, CORE_A);
    let session = engine.create_session(None).expect("session");
    let provider = FakeProvider::with_tools(
        "Requesting a tool.",
        vec![ToolCall {
            name: "memory.search".to_owned(),
            arguments: json!({"text": "tea"}),
        }],
    );
    let turn = engine
        .send_turn(
            &session.session_id,
            "Use a tool",
            &provider,
            &TurnOptions::new("test-model"),
            &CancelToken::new(),
        )
        .expect("turn");
    assert_eq!(turn.response.tool_calls.len(), 1);
    let metadata = turn.assistant.metadata.expect("metadata");
    assert_eq!(metadata["tool_calls"][0]["name"], "memory.search");
}

#[test]
fn archived_sessions_reject_new_messages() {
    let (_temp, root, _token) = setup();
    let engine = open_engine(&root, CORE_A);
    let session = engine.create_session(None).expect("session");
    engine
        .store()
        .archive_session(&session.session_id)
        .expect("archive");
    assert_eq!(
        engine
            .store()
            .session(&session.session_id)
            .expect("session")
            .state,
        SessionState::Archived
    );
    let error = engine
        .store()
        .append_message(
            &session.session_id,
            NewMessage::new(MessageRole::User, "late"),
        )
        .expect_err("archived must reject");
    assert!(matches!(error, ConversationError::SessionNotActive(_)));
}

#[test]
fn conversation_databases_are_isolated_per_core() {
    let (_temp, root, _token) = setup();
    let engine_a = open_engine(&root, CORE_A);
    let session = engine_a.create_session(Some("Private A")).expect("session");
    engine_a
        .store()
        .append_message(
            &session.session_id,
            NewMessage::new(MessageRole::User, "A private memory"),
        )
        .expect("append");

    let engine_b = open_engine(&root, CORE_B);
    assert!(engine_b
        .store()
        .list_sessions(true, true)
        .expect("list")
        .is_empty());
    drop(engine_b);

    // Copying A's conversation database into B's directory must not be adopted.
    let source = root.join("cores").join(CORE_A).join("conversation.db");
    let target = root.join("cores").join(CORE_B).join("conversation.db");
    std::fs::copy(&source, &target).expect("copy");
    let error = ConversationStore::open(&root, CORE_B).expect_err("must reject foreign DB");
    assert!(matches!(error, ConversationError::OwnershipMismatch));
}

#[test]
fn tool_calls_execute_only_with_owner_approval_and_persist_results() {
    let (_temp, root, token) = setup();
    let engine = open_engine(&root, CORE_A);
    let session = engine.create_session(None).expect("session");
    let provider = FakeProvider::with_tools(
        "I will remember that.",
        vec![ToolCall {
            name: "memory.create".to_owned(),
            arguments: json!({"content": "The owner likes oolong tea."}),
        }],
    );
    let turn = engine
        .send_turn(
            &session.session_id,
            "Remember my tea preference.",
            &provider,
            &TurnOptions::new("test-model"),
            &CancelToken::new(),
        )
        .expect("turn");
    assert_eq!(turn.response.tool_calls.len(), 1);

    let registry = ToolRegistry::new(StoragePaths::new(&root).expect("paths"));
    registry.initialize().expect("registry");
    registry
        .set_permission("memory.create", true, true, &token)
        .expect("grant");

    let executor = ToolExecutor::new(&root, CORE_A).expect("executor");
    let unapproved = engine
        .execute_tool_calls(
            &session.session_id,
            &turn.response.tool_calls,
            &executor,
            None,
        )
        .expect("tool calls");
    assert_eq!(unapproved.len(), 1);
    assert_eq!(unapproved[0].status, "approval_required");
    let memory = engine.sdk().memory();
    assert!(memory
        .search(&aegis_core_sdk::MemoryQuery::new().with_text("oolong"))
        .expect("search")
        .is_empty());

    let approved = engine
        .execute_tool_calls(
            &session.session_id,
            &turn.response.tool_calls,
            &executor,
            Some(&token),
        )
        .expect("tool calls");
    assert_eq!(approved[0].status, "executed");
    assert_eq!(
        memory
            .search(&aegis_core_sdk::MemoryQuery::new().with_text("oolong"))
            .expect("search")
            .len(),
        1
    );

    let follow_up = engine
        .continue_after_tools(
            &session.session_id,
            &FakeProvider::new("Saved your tea preference."),
            &TurnOptions::new("test-model"),
            &CancelToken::new(),
        )
        .expect("follow up");
    assert_eq!(follow_up.assistant.content, "Saved your tea preference.");

    let messages = engine
        .store()
        .messages(&session.session_id, 20, 0)
        .expect("messages");
    assert!(messages
        .iter()
        .any(|message| message.role == MessageRole::Tool));
    assert!(messages
        .iter()
        .any(|message| message.content == "Saved your tea preference."));
    let events = engine
        .store()
        .session_events(&session.session_id)
        .expect("events");
    assert!(events
        .iter()
        .any(|event| event.event_type == "tool.executed"));
    assert!(events
        .iter()
        .any(|event| event.event_type == "tool.approval_required"));
}

#[derive(Debug)]
struct FixedRetriever;

impl MemoryRetriever for FixedRetriever {
    fn name(&self) -> &'static str {
        "fixed-test"
    }

    fn semantic(&self) -> bool {
        true
    }

    fn retrieve(
        &self,
        memory: &CoreMemoryStore,
        _text: &str,
        limit: usize,
    ) -> Result<Vec<MemoryEntry>, SdkError> {
        memory.search(&MemoryQuery::new().with_limit(limit.max(1)))
    }
}

#[derive(Debug)]
struct FailingEmbeddingProvider;

impl EmbeddingProvider for FailingEmbeddingProvider {
    fn name(&self) -> &str {
        "failing"
    }

    fn model(&self) -> &str {
        "none"
    }

    fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        Err(EmbeddingError::Unavailable("provider down".to_owned()))
    }
}

#[test]
fn embedding_retriever_ranks_memories_and_reports_method() {
    let (_temp, root, _token) = setup();
    let engine = open_engine(&root, CORE_A);
    engine
        .sdk()
        .memory()
        .create(NewMemory::new(
            MemoryKind::Preference,
            "The owner likes oolong tea.",
        ))
        .expect("tea");
    engine
        .sdk()
        .memory()
        .create(NewMemory::new(
            MemoryKind::Fact,
            "The weather is cold today.",
        ))
        .expect("weather");
    let session = engine.create_session(None).expect("session");
    engine
        .store()
        .append_message(
            &session.session_id,
            NewMessage::new(MessageRole::User, "What tea does the owner like?"),
        )
        .expect("append");

    let embedding =
        EmbeddingRetriever::open(&root, CORE_A, Box::new(HashingEmbeddingProvider::new(256)))
            .expect("retriever");
    let engine = open_engine(&root, CORE_A).with_retriever(Box::new(embedding));
    let prepared = engine
        .build_request(&session.session_id, &TurnOptions::new("test-model"))
        .expect("prepared");
    assert!(prepared
        .retrieval_method
        .starts_with("embedding:local-hash"));
    assert!(!prepared.memory_used.is_empty());
    assert!(prepared.memory_used[0]
        .snippet
        .to_lowercase()
        .contains("tea"));

    let store = EmbeddingStore::open(&root, CORE_A).expect("embedding store");
    assert!(store.count().expect("count") >= 2);
}

#[test]
fn embedding_provider_failure_falls_back_to_keyword_honestly() {
    let (_temp, root, _token) = setup();
    let engine = open_engine(&root, CORE_A);
    engine
        .sdk()
        .memory()
        .create(NewMemory::new(
            MemoryKind::Fact,
            "The owner has a cat named Miso.",
        ))
        .expect("memory");
    let session = engine.create_session(None).expect("session");
    engine
        .store()
        .append_message(
            &session.session_id,
            NewMessage::new(MessageRole::User, "Tell me about Miso the cat"),
        )
        .expect("append");
    let retriever = EmbeddingRetriever::open(&root, CORE_A, Box::new(FailingEmbeddingProvider))
        .expect("retriever");
    let engine = open_engine(&root, CORE_A).with_retriever(Box::new(retriever));
    let prepared = engine
        .build_request(&session.session_id, &TurnOptions::new("test-model"))
        .expect("prepared");
    assert!(prepared.retrieval_method.starts_with("keyword(fallback:"));
    assert!(!prepared.memory_used.is_empty());
    assert!(prepared.memory_used[0].snippet.contains("Miso"));
}

#[test]
fn retrieval_strategy_is_pluggable_and_reported() {
    let (_temp, root, _token) = setup();
    let engine = open_engine(&root, CORE_A);
    engine
        .sdk()
        .memory()
        .create(NewMemory::new(MemoryKind::Fact, "Unrelated stored fact."))
        .expect("memory");
    let session = engine.create_session(None).expect("session");
    engine
        .store()
        .append_message(
            &session.session_id,
            NewMessage::new(MessageRole::User, "zzzz qqqq"),
        )
        .expect("append");
    let keyword = engine
        .build_request(&session.session_id, &TurnOptions::new("test-model"))
        .expect("keyword");
    assert_eq!(keyword.retrieval_method, "keyword");
    assert!(keyword.memory_used.is_empty());
    assert_eq!(engine.retriever_name(), "keyword");

    let fixed = open_engine(&root, CORE_A).with_retriever(Box::new(FixedRetriever));
    assert_eq!(fixed.retriever_name(), "fixed-test");
    let prepared = fixed
        .build_request(&session.session_id, &TurnOptions::new("test-model"))
        .expect("fixed");
    assert_eq!(prepared.retrieval_method, "fixed-test");
    assert_eq!(prepared.memory_used.len(), 1);
    assert!(prepared.memory_used[0].snippet.contains("Unrelated"));
}

#[test]
fn assembled_request_uses_real_history_and_records_provider() {
    let (_temp, root, _token) = setup();
    let engine = open_engine(&root, CORE_A);
    let session = engine.create_session(None).expect("session");
    let provider = FakeProvider::new("first");
    engine
        .send_turn(
            &session.session_id,
            "First question",
            &provider,
            &TurnOptions::new("test-model"),
            &CancelToken::new(),
        )
        .expect("turn");
    let request = provider.last_request().expect("request");
    assert_eq!(request.model, "test-model");
    assert_eq!(request.messages[0].role, aegis_cognition::ChatRole::System);
    assert!(request
        .messages
        .iter()
        .any(|message| message.content == "First question"));
}

#[test]
fn empty_provider_response_is_reported_without_assistant_message() {
    let (_temp, root, _token) = setup();
    let engine = open_engine(&root, CORE_A);
    let session = engine.create_session(None).expect("session");
    let error = engine
        .send_turn(
            &session.session_id,
            "Ask something",
            &FakeProvider::new(""),
            &TurnOptions::new("test-model"),
            &CancelToken::new(),
        )
        .expect_err("empty must fail");
    assert!(matches!(error, ConversationError::EmptyResponse));
    let messages = engine
        .store()
        .messages(&session.session_id, 10, 0)
        .expect("messages");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, MessageRole::User);
    let events = engine
        .store()
        .session_events(&session.session_id)
        .expect("events");
    assert!(events
        .iter()
        .any(|event| event.event_type == "turn.empty_response"));
}

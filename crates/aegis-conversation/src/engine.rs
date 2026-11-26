//! Provider-independent conversation interaction engine.
//!
//! The engine composes four separate responsibilities without collapsing them:
//!
//! * **Persistence** — [`ConversationStore`] keeps session/message history in
//!   the Core's private `conversation.db`.
//! * **Memory** — private per-Core memory entries are retrieved for context.
//! * **Personality** — descriptive tone/values are included in the system
//!   prompt and never consulted for authorization.
//! * **Cognition** — any [`ModelProvider`] can answer the assembled request.
//!
//! Tool calls returned by a model are stored as data and surfaced as events.
//! The engine never executes a tool and never grants a permission.

use std::path::Path;

use aegis_cognition::{
    CancelToken, ChatMessage, ChatRequest, ChatResponse, ChatRole, ModelProvider, ToolCall,
};
use aegis_contracts::personal::{Activity, ContinuitySummary, EmotionState};
use aegis_contracts::world::{FreshnessState, WorldSnapshot};
use aegis_core_sdk::{
    apply_interaction_event as apply_personal_interaction_event, CoreSdkContext, EventApplication,
    InteractionEvent, MemoryEntry, PersonalityProfile,
};
use aegis_tools::ToolExecutor;

use crate::error::ConversationError;
use crate::retrieval::{KeywordRetriever, MemoryRetriever};
use crate::store::{
    ConversationMessage, ConversationSession, ConversationStore, MessageRole, NewMessage,
};

/// Default number of recent messages included in a request.
pub const DEFAULT_HISTORY_LIMIT: usize = 40;
/// Default number of memory entries retrieved for context.
pub const DEFAULT_MEMORY_LIMIT: usize = 6;
/// Maximum characters in the assembled system prompt.
pub const MAX_SYSTEM_PROMPT_CHARS: usize = 8_000;
/// Maximum characters of one memory snippet.
pub const MAX_MEMORY_SNIPPET_CHARS: usize = 400;
/// Maximum rendered characters for one bounded prompt list item.
const MAX_PROMPT_ITEM_CHARS: usize = 160;
/// Maximum list items rendered per prompt section.
const MAX_PROMPT_LIST_ITEMS: usize = 6;

/// Per-turn assembly options.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnOptions {
    pub model: String,
    pub memory_limit: usize,
    pub history_limit: usize,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub timeout_ms: Option<u64>,
    pub think: Option<bool>,
    pub system_instructions: Option<String>,
    /// Shared, authority-free world snapshot supplied by the Platform clock.
    pub world: Option<WorldSnapshot>,
    /// This Core's bounded continuity summary from its personal store.
    pub continuity: Option<ContinuitySummary>,
    /// This Core's internal emotional state (descriptive only).
    pub emotion: Option<EmotionState>,
    /// Additional activities to surface in the prompt.
    pub activities: Vec<Activity>,
}

impl TurnOptions {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            memory_limit: DEFAULT_MEMORY_LIMIT,
            history_limit: DEFAULT_HISTORY_LIMIT,
            temperature: None,
            max_tokens: None,
            timeout_ms: None,
            think: None,
            system_instructions: None,
            world: None,
            continuity: None,
            emotion: None,
            activities: Vec::new(),
        }
    }

    pub fn with_memory_limit(mut self, limit: usize) -> Self {
        self.memory_limit = limit;
        self
    }

    pub fn with_history_limit(mut self, limit: usize) -> Self {
        self.history_limit = limit;
        self
    }

    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = Some(timeout_ms);
        self
    }

    pub fn with_system_instructions(mut self, instructions: impl Into<String>) -> Self {
        self.system_instructions = Some(instructions.into());
        self
    }

    pub fn with_think(mut self, think: Option<bool>) -> Self {
        self.think = think;
        self
    }

    /// Attach a shared world snapshot to the assembled system prompt.
    pub fn with_world(mut self, world: WorldSnapshot) -> Self {
        self.world = Some(world);
        self
    }

    /// Attach this Core's continuity summary to the assembled system prompt.
    pub fn with_continuity(mut self, continuity: ContinuitySummary) -> Self {
        self.continuity = Some(continuity);
        self
    }

    /// Attach this Core's internal emotional state to the assembled prompt.
    pub fn with_emotion(mut self, emotion: EmotionState) -> Self {
        self.emotion = Some(emotion);
        self
    }

    /// Attach additional activities to the assembled system prompt.
    pub fn with_activities(mut self, activities: Vec<Activity>) -> Self {
        self.activities = activities;
        self
    }
}

/// One memory entry referenced by an assembled turn.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MemoryReference {
    pub id: String,
    pub kind: String,
    pub provenance: String,
    pub snippet: String,
}

/// A request assembled from identity, personality, memory, and history.
#[derive(Debug, Clone)]
pub struct PreparedTurn {
    pub request: ChatRequest,
    pub system_prompt: String,
    pub memory_used: Vec<MemoryReference>,
    pub retrieval_method: String,
    pub retrieval_semantic: bool,
    pub world_used: bool,
    pub continuity_used: bool,
    pub emotion_used: bool,
    pub activities_used: bool,
}

/// A completed non-streaming or streamed turn.
#[derive(Debug, Clone)]
pub struct ConversationTurn {
    pub session: ConversationSession,
    pub user: ConversationMessage,
    pub assistant: ConversationMessage,
    pub response: ChatResponse,
    pub memory_used: Vec<MemoryReference>,
    pub retrieval_method: String,
    pub retrieval_semantic: bool,
    pub world_used: bool,
    pub continuity_used: bool,
    pub emotion_used: bool,
    pub activities_used: bool,
    /// Non-fatal warning if the post-turn personal-continuity update failed.
    pub personal_update_warning: Option<String>,
    /// Whether the deterministic post-turn interaction event was newly applied.
    pub interaction_applied: bool,
    /// Non-fatal warning if the post-turn interaction event update failed.
    pub interaction_warning: Option<String>,
}

/// Result of executing one model-proposed tool call.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ToolExecutionRecord {
    pub tool: String,
    pub status: String,
    pub sensitivity: String,
    pub message_id: u64,
    pub output: serde_json::Value,
    pub detail: Option<String>,
}

/// A follow-up assistant turn produced after tool results were added.
#[derive(Debug, Clone)]
pub struct AssistantTurn {
    pub session: ConversationSession,
    pub assistant: ConversationMessage,
    pub response: ChatResponse,
    pub memory_used: Vec<MemoryReference>,
    pub retrieval_method: String,
    pub retrieval_semantic: bool,
    pub world_used: bool,
    pub continuity_used: bool,
    pub emotion_used: bool,
    pub activities_used: bool,
    /// Non-fatal warning if the post-turn personal-continuity update failed.
    pub personal_update_warning: Option<String>,
    /// Whether the deterministic post-turn interaction event was newly applied.
    pub interaction_applied: bool,
    /// Non-fatal warning if the post-turn interaction event update failed.
    pub interaction_warning: Option<String>,
}

/// Streaming interaction event delivered to the caller.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum TurnEvent {
    MemoryRetrieved {
        count: usize,
    },
    Delta {
        text: String,
    },
    ToolRequested {
        name: String,
        arguments: serde_json::Value,
    },
    Completed {
        message_id: u64,
        content: String,
    },
    Failed {
        message: String,
    },
}

/// Conversation engine bound to one Core.
#[derive(Debug)]
pub struct ConversationEngine {
    store: ConversationStore,
    sdk: CoreSdkContext,
    retriever: Box<dyn MemoryRetriever>,
}

impl ConversationEngine {
    /// Open the conversation engine for an existing Core.
    pub fn open(root: impl AsRef<Path>, core_uuid: &str) -> Result<Self, ConversationError> {
        let root = root.as_ref();
        let store = ConversationStore::open(root, core_uuid)?;
        let sdk = CoreSdkContext::open(root, core_uuid)?;
        Ok(Self {
            store,
            sdk,
            retriever: Box::new(KeywordRetriever),
        })
    }

    /// Replace the memory retrieval strategy. The default is explicit keyword
    /// matching; semantic retrievers must implement [`MemoryRetriever`].
    pub fn with_retriever(mut self, retriever: Box<dyn MemoryRetriever>) -> Self {
        self.retriever = retriever;
        self
    }

    pub fn retriever_name(&self) -> &'static str {
        self.retriever.name()
    }

    pub fn store(&self) -> &ConversationStore {
        &self.store
    }

    pub fn sdk(&self) -> &CoreSdkContext {
        &self.sdk
    }

    pub fn create_session(
        &self,
        title: Option<&str>,
    ) -> Result<ConversationSession, ConversationError> {
        self.store.create_session(title, None, None)
    }

    /// Assemble a provider-neutral request from stored history, private memory,
    /// and descriptive personality data.
    pub fn build_request(
        &self,
        session_id: &str,
        options: &TurnOptions,
    ) -> Result<PreparedTurn, ConversationError> {
        let session = self.store.session(session_id)?;
        if session.state != crate::store::SessionState::Active {
            return Err(ConversationError::SessionNotActive(session_id.to_owned()));
        }
        let history = self
            .store
            .recent_messages(session_id, options.history_limit)?;
        let last_user = history
            .iter()
            .rev()
            .find(|message| message.role == MessageRole::User)
            .map(|message| message.content.clone());

        let (memories, retrieval_method, retrieval_semantic) =
            match (last_user.as_deref(), options.memory_limit) {
                (Some(text), limit) if limit > 0 => {
                    let outcome =
                        self.retriever
                            .retrieve_with_meta(self.sdk.memory(), text, limit)?;
                    (outcome.entries, outcome.method, outcome.semantic)
                }
                _ => (
                    Vec::new(),
                    self.retriever.name().to_owned(),
                    self.retriever.semantic(),
                ),
            };
        let memory_used: Vec<MemoryReference> = memories.iter().map(memory_reference).collect();
        let personality = self.sdk.memory().load_personality()?;
        let composed = compose_system_prompt(
            &self.sdk.identity().display_name,
            personality.as_ref(),
            &memories,
            options,
        );
        let system_prompt = composed.text;

        let mut messages = Vec::with_capacity(history.len() + 1);
        messages.push(ChatMessage::new(ChatRole::System, system_prompt.clone()));
        for message in &history {
            messages.push(ChatMessage::new(
                chat_role(message.role),
                message.content.clone(),
            ));
        }
        let mut request = ChatRequest::new(options.model.clone(), messages);
        request.temperature = options.temperature;
        request.max_tokens = options.max_tokens;
        request.timeout_ms = options.timeout_ms;
        request.think = options.think;
        Ok(PreparedTurn {
            request,
            system_prompt,
            memory_used,
            retrieval_method,
            retrieval_semantic,
            world_used: composed.world_used,
            continuity_used: composed.continuity_used,
            emotion_used: composed.emotion_used,
            activities_used: composed.activities_used,
        })
    }

    /// Append a user message, run a non-streaming completion, and persist the
    /// assistant reply.
    pub fn send_turn(
        &self,
        session_id: &str,
        user_text: &str,
        provider: &dyn ModelProvider,
        options: &TurnOptions,
        cancel: &CancelToken,
    ) -> Result<ConversationTurn, ConversationError> {
        let user = self
            .store
            .append_message(session_id, NewMessage::new(MessageRole::User, user_text))?;
        let prepared = self.build_request(session_id, options)?;
        let response = match provider.chat(&prepared.request, cancel) {
            Ok(response) => response,
            Err(error) => {
                let _ = self.store.append_event(
                    session_id,
                    "turn.failed",
                    &serde_json::json!({
                        "provider_id": provider.id().as_str(),
                        "error": error.to_string(),
                    }),
                );
                return Err(error.into());
            }
        };
        if response.content.trim().is_empty() {
            let _ = self.store.append_event(
                session_id,
                "turn.empty_response",
                &serde_json::json!({"provider_id": provider.id().as_str()}),
            );
            return Err(ConversationError::EmptyResponse);
        }
        let assistant = self.store_assistant(session_id, provider, &response, &prepared)?;
        let (interaction_applied, interaction_warning) =
            self.record_interaction(session_id, assistant.message_id);
        let personal_update_warning = self.update_personal_continuity(session_id);
        let session = self.store.session(session_id)?;
        Ok(ConversationTurn {
            session,
            user,
            assistant,
            response,
            memory_used: prepared.memory_used,
            retrieval_method: prepared.retrieval_method,
            retrieval_semantic: prepared.retrieval_semantic,
            world_used: prepared.world_used,
            continuity_used: prepared.continuity_used,
            emotion_used: prepared.emotion_used,
            activities_used: prepared.activities_used,
            personal_update_warning,
            interaction_applied,
            interaction_warning,
        })
    }

    /// Stream a completion, emitting [`TurnEvent`]s in arrival order.
    pub fn send_turn_streaming(
        &self,
        session_id: &str,
        user_text: &str,
        provider: &dyn ModelProvider,
        options: &TurnOptions,
        cancel: &CancelToken,
        on_event: &mut dyn FnMut(TurnEvent) -> Result<(), aegis_cognition::InferenceError>,
    ) -> Result<ConversationTurn, ConversationError> {
        let user = self
            .store
            .append_message(session_id, NewMessage::new(MessageRole::User, user_text))?;
        let prepared = self.build_request(session_id, options)?;
        on_event(TurnEvent::MemoryRetrieved {
            count: prepared.memory_used.len(),
        })?;

        let result = provider.chat_stream(&prepared.request, cancel, &mut |chunk| {
            for call in chunk.tool_calls {
                on_event(TurnEvent::ToolRequested {
                    name: call.name,
                    arguments: call.arguments,
                })?;
            }
            if !chunk.delta.is_empty() {
                on_event(TurnEvent::Delta { text: chunk.delta })?;
            }
            Ok(())
        });

        let response = match result {
            Ok(response) => response,
            Err(error) => {
                let _ = self.store.append_event(
                    session_id,
                    "turn.failed",
                    &serde_json::json!({
                        "provider_id": provider.id().as_str(),
                        "error": error.to_string(),
                    }),
                );
                let _ = on_event(TurnEvent::Failed {
                    message: error.to_string(),
                });
                return Err(error.into());
            }
        };
        if response.content.trim().is_empty() {
            let _ = self.store.append_event(
                session_id,
                "turn.empty_response",
                &serde_json::json!({"provider_id": provider.id().as_str()}),
            );
            return Err(ConversationError::EmptyResponse);
        }
        let assistant = self.store_assistant(session_id, provider, &response, &prepared)?;
        on_event(TurnEvent::Completed {
            message_id: assistant.message_id,
            content: assistant.content.clone(),
        })?;
        let (interaction_applied, interaction_warning) =
            self.record_interaction(session_id, assistant.message_id);
        let personal_update_warning = self.update_personal_continuity(session_id);
        let session = self.store.session(session_id)?;
        Ok(ConversationTurn {
            session,
            user,
            assistant,
            response,
            memory_used: prepared.memory_used,
            retrieval_method: prepared.retrieval_method,
            retrieval_semantic: prepared.retrieval_semantic,
            world_used: prepared.world_used,
            continuity_used: prepared.continuity_used,
            emotion_used: prepared.emotion_used,
            activities_used: prepared.activities_used,
            personal_update_warning,
            interaction_applied,
            interaction_warning,
        })
    }

    /// Execute model-proposed tool calls through the authorized tool bridge and
    /// persist each result as a `tool` message.
    ///
    /// The engine does not make the permission decision itself; the executor
    /// enforces grant, STOP, and owner approval. Sensitive tools receive
    /// `owner_token` only when the caller explicitly supplies it.
    pub fn execute_tool_calls(
        &self,
        session_id: &str,
        calls: &[ToolCall],
        executor: &ToolExecutor,
        owner_token: Option<&str>,
    ) -> Result<Vec<ToolExecutionRecord>, ConversationError> {
        let session = self.store.session(session_id)?;
        if session.state != crate::store::SessionState::Active {
            return Err(ConversationError::SessionNotActive(session_id.to_owned()));
        }
        let mut records = Vec::with_capacity(calls.len());
        for call in calls {
            let result = executor.execute(&call.name, &call.arguments, owner_token)?;
            let content = serde_json::to_string(&result)?;
            let metadata = serde_json::json!({
                "tool": call.name,
                "status": result.status.as_str(),
                "sensitivity": result.sensitivity.as_str(),
                "audited": result.audited,
            });
            let message = self.store.append_message(
                session_id,
                NewMessage {
                    role: MessageRole::Tool,
                    content: &content,
                    provider_id: None,
                    model: None,
                    prompt_tokens: None,
                    completion_tokens: None,
                    metadata: Some(&metadata),
                },
            )?;
            let event = match result.status.as_str() {
                "executed" => "tool.executed",
                "denied" => "tool.denied",
                "approval_required" => "tool.approval_required",
                _ => "tool.failed",
            };
            self.store.append_event(
                session_id,
                event,
                &serde_json::json!({
                    "tool": call.name,
                    "status": result.status.as_str(),
                }),
            )?;
            records.push(ToolExecutionRecord {
                tool: call.name.clone(),
                status: result.status.as_str().to_owned(),
                sensitivity: result.sensitivity.as_str().to_owned(),
                message_id: message.message_id,
                output: result.output,
                detail: result.detail,
            });
        }
        Ok(records)
    }

    /// Ask the provider for a follow-up assistant turn after tool results have
    /// been appended to the session. Persists the assistant reply.
    pub fn continue_after_tools(
        &self,
        session_id: &str,
        provider: &dyn ModelProvider,
        options: &TurnOptions,
        cancel: &CancelToken,
    ) -> Result<AssistantTurn, ConversationError> {
        let prepared = self.build_request(session_id, options)?;
        let response = match provider.chat(&prepared.request, cancel) {
            Ok(response) => response,
            Err(error) => {
                let _ = self.store.append_event(
                    session_id,
                    "turn.failed",
                    &serde_json::json!({
                        "provider_id": provider.id().as_str(),
                        "error": error.to_string(),
                    }),
                );
                return Err(error.into());
            }
        };
        if response.content.trim().is_empty() {
            let _ = self.store.append_event(
                session_id,
                "turn.empty_response",
                &serde_json::json!({"provider_id": provider.id().as_str()}),
            );
            return Err(ConversationError::EmptyResponse);
        }
        let assistant = self.store_assistant(session_id, provider, &response, &prepared)?;
        let (interaction_applied, interaction_warning) =
            self.record_interaction(session_id, assistant.message_id);
        let personal_update_warning = self.update_personal_continuity(session_id);
        let session = self.store.session(session_id)?;
        Ok(AssistantTurn {
            session,
            assistant,
            response,
            memory_used: prepared.memory_used,
            retrieval_method: prepared.retrieval_method,
            retrieval_semantic: prepared.retrieval_semantic,
            world_used: prepared.world_used,
            continuity_used: prepared.continuity_used,
            emotion_used: prepared.emotion_used,
            activities_used: prepared.activities_used,
            personal_update_warning,
            interaction_applied,
            interaction_warning,
        })
    }

    /// Apply one explicit interaction event to this Core's personal state.
    ///
    /// The operation is idempotent by `event_id`: replaying an event returns
    /// `applied = false` and leaves the experience log and emotion state
    /// unchanged. Callers that do not have an event can use
    /// [`InteractionEvent::for_message`] to derive the deterministic engine id.
    pub fn apply_interaction_event(
        &self,
        event: &InteractionEvent,
    ) -> Result<EventApplication, ConversationError> {
        let personal = self.sdk.personal()?;
        Ok(apply_personal_interaction_event(personal, event)?)
    }

    /// Best-effort deterministic interaction event after a successful turn.
    ///
    /// The event id is derived from the session and assistant message, so a
    /// replayed turn or a second call with the same id is skipped. A failure
    /// here never fails the turn; it is surfaced as a non-fatal warning.
    fn record_interaction(&self, session_id: &str, message_id: u64) -> (bool, Option<String>) {
        let event = InteractionEvent::for_message(session_id, message_id);
        match self.apply_interaction_event(&event) {
            Ok(application) => (application.applied, None),
            Err(error) => (
                false,
                Some(format!("personal interaction event failed: {error}")),
            ),
        }
    }

    /// Best-effort continuity update after a successful turn.
    ///
    /// A failure here never fails the turn; it is surfaced as a non-fatal
    /// warning field on the returned turn.
    fn update_personal_continuity(&self, session_id: &str) -> Option<String> {
        let result = self
            .sdk
            .personal()
            .and_then(|personal| personal.update_last_active(Some(session_id)));
        match result {
            Ok(()) => None,
            Err(error) => Some(format!("personal continuity update failed: {error}")),
        }
    }

    fn store_assistant(
        &self,
        session_id: &str,
        provider: &dyn ModelProvider,
        response: &ChatResponse,
        prepared: &PreparedTurn,
    ) -> Result<ConversationMessage, ConversationError> {
        let metadata = serde_json::json!({
            "provider_id": provider.id().as_str(),
            "provider_kind": provider.kind().as_str(),
            "model": response.model,
            "done": response.done,
            "total_duration_ms": response.total_duration_ms,
            "tool_calls": response.tool_calls,
            "memory_ids": prepared.memory_used.iter().map(|memory| memory.id.clone()).collect::<Vec<_>>(),
        });
        self.store.append_message(
            session_id,
            NewMessage {
                role: MessageRole::Assistant,
                content: &response.content,
                provider_id: Some(provider.id().as_str()),
                model: Some(&response.model),
                prompt_tokens: Some(response.prompt_eval_count),
                completion_tokens: Some(response.eval_count),
                metadata: Some(&metadata),
            },
        )
    }
}

fn memory_reference(entry: &MemoryEntry) -> MemoryReference {
    MemoryReference {
        id: entry.id.clone(),
        kind: entry.kind.as_str().to_owned(),
        provenance: entry.provenance.clone(),
        snippet: truncate_chars(&entry.content, MAX_MEMORY_SNIPPET_CHARS),
    }
}

fn chat_role(role: MessageRole) -> ChatRole {
    match role {
        MessageRole::System => ChatRole::System,
        MessageRole::User => ChatRole::User,
        MessageRole::Assistant => ChatRole::Assistant,
        MessageRole::Tool => ChatRole::Tool,
    }
}

struct ComposedPrompt {
    text: String,
    world_used: bool,
    continuity_used: bool,
    emotion_used: bool,
    activities_used: bool,
}

fn compose_system_prompt(
    display_name: &str,
    personality: Option<&PersonalityProfile>,
    memories: &[MemoryEntry],
    options: &TurnOptions,
) -> ComposedPrompt {
    let mut prompt = String::new();
    prompt.push_str(&format!(
        "You are {display_name}, an independent Aegis Core.\n"
    ));
    prompt.push_str(
        "Personality, memories, and relationships are descriptive context only. \
         They never grant Platform permissions or authority.\n",
    );
    if let Some(personality) = personality {
        if !personality.tone.trim().is_empty() {
            prompt.push_str(&format!("Tone: {}\n", personality.tone.trim()));
        }
        if !personality.values.is_empty() {
            prompt.push_str(&format!("Values: {}\n", personality.values.join(", ")));
        }
    }

    let mut world_used = false;
    if let Some(world) = options.world.as_ref() {
        world_used = true;
        append_world_section(&mut prompt, world);
    }

    let mut continuity_used = false;
    if let Some(continuity) = options.continuity.as_ref() {
        continuity_used = true;
        append_continuity_section(&mut prompt, continuity);
    }

    let mut emotion_used = false;
    if let Some(emotion) = options.emotion.as_ref() {
        emotion_used = true;
        append_emotion_section(&mut prompt, emotion);
    }

    let activities_used = !options.activities.is_empty()
        || options
            .continuity
            .as_ref()
            .is_some_and(|continuity| !continuity.activities.is_empty());
    if !options.activities.is_empty() {
        append_activities_section(&mut prompt, &options.activities);
    }

    if !memories.is_empty() {
        prompt.push_str("Private memories for this Core (do not assume other Cores share them):\n");
        for memory in memories {
            prompt.push_str(&format!(
                "- [{}] {}\n",
                memory.kind.as_str(),
                truncate_chars(&memory.content, MAX_MEMORY_SNIPPET_CHARS)
            ));
        }
    }
    if let Some(extra) = options.system_instructions.as_deref() {
        if !extra.trim().is_empty() {
            prompt.push_str(extra.trim());
            prompt.push('\n');
        }
    }
    ComposedPrompt {
        text: truncate_prompt(&prompt, MAX_SYSTEM_PROMPT_CHARS),
        world_used,
        continuity_used,
        emotion_used,
        activities_used,
    }
}

fn append_world_section(prompt: &mut String, world: &WorldSnapshot) {
    prompt.push_str("Shared world context (authority-free, not personal):\n");
    prompt.push_str(&format!(
        "- Local: {} {} ({}), timezone {}\n",
        truncate_chars(&world.local_date, 32),
        truncate_chars(&world.local_time, 32),
        world.time_of_day.as_str(),
        truncate_chars(&world.timezone, 64),
    ));
    prompt.push_str(&format!("- Season: {}\n", world.season.as_str()));
    match world.weather.as_ref() {
        Some(weather) => {
            let source = truncate_chars(&world.provenance.source, 64);
            let observed_at = weather
                .observed_at
                .as_deref()
                .map(|value| truncate_chars(value, 64))
                .unwrap_or_else(|| "time unknown".to_owned());
            let mut line = format!(
                "- Weather: {} (source: {source}, observed_at: {observed_at}, freshness: {})",
                truncate_chars(&weather.condition, 120),
                world.freshness.as_str()
            );
            if let Some(temperature) = weather.temperature_celsius {
                if temperature.is_finite() {
                    line.push_str(&format!(", {temperature:.1}C"));
                }
            }
            line.push('\n');
            prompt.push_str(&line);
            if matches!(
                world.freshness,
                FreshnessState::Stale | FreshnessState::Unavailable
            ) {
                prompt.push_str(&format!(
                    "- Weather freshness note: data is {}, so it may not reflect current conditions.\n",
                    world.freshness.as_str()
                ));
            }
        }
        None => {
            prompt.push_str(&format!(
                "- Weather: not provided (freshness: {}).\n",
                world.freshness.as_str()
            ));
        }
    }
}

fn append_continuity_section(prompt: &mut String, continuity: &ContinuitySummary) {
    prompt.push_str("Continuity with this Core's own past (descriptive):\n");
    prompt.push_str(&format!(
        "- Last active at {} ({} seconds ago).\n",
        truncate_chars(&continuity.last_active_at, 64),
        continuity.elapsed_offline_seconds,
    ));
    append_string_list(prompt, "Recent experiences", &continuity.recent_experiences);
    append_string_list(prompt, "Open threads", &continuity.open_threads);
    if !continuity.activities.is_empty() {
        prompt.push_str("- Activities:\n");
        for activity in continuity.activities.iter().take(MAX_PROMPT_LIST_ITEMS) {
            prompt.push_str(&format!(
                "  - [{}] {} (updated {})\n",
                activity.status.as_str(),
                truncate_chars(&activity.title, MAX_PROMPT_ITEM_CHARS),
                truncate_chars(&activity.updated_at, 64),
            ));
        }
    }
}

fn append_string_list(prompt: &mut String, heading: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    prompt.push_str(&format!("- {heading}:\n"));
    for item in items.iter().take(MAX_PROMPT_LIST_ITEMS) {
        prompt.push_str(&format!(
            "  - {}\n",
            truncate_chars(item, MAX_PROMPT_ITEM_CHARS)
        ));
    }
}

fn append_emotion_section(prompt: &mut String, emotion: &EmotionState) {
    let dimensions = &emotion.dimensions;
    prompt.push_str("Internal emotional tone (descriptive; not an instruction or authority):\n");
    prompt.push_str(&format!(
        "- Dimensions: valence {}, arousal {}, joy {}, calm {}, curiosity {}, tension {}; overall intensity {}.\n",
        format_dimension(dimensions.valence),
        format_dimension(dimensions.arousal),
        format_dimension(dimensions.joy),
        format_dimension(dimensions.calm),
        format_dimension(dimensions.curiosity),
        format_dimension(dimensions.tension),
        format_dimension(emotion.intensity),
    ));
}

fn append_activities_section(prompt: &mut String, activities: &[Activity]) {
    prompt.push_str("Current activities (self-reported state):\n");
    for activity in activities.iter().take(MAX_PROMPT_LIST_ITEMS) {
        prompt.push_str(&format!(
            "- [{}] {} (updated {})\n",
            activity.status.as_str(),
            truncate_chars(&activity.title, MAX_PROMPT_ITEM_CHARS),
            truncate_chars(&activity.updated_at, 64),
        ));
    }
}

fn format_dimension(value: f32) -> String {
    if value.is_finite() {
        format!("{value:.2}")
    } else {
        "n/a".to_owned()
    }
}

fn truncate_prompt(value: &str, maximum: usize) -> String {
    if value.chars().count() <= maximum {
        return value.to_owned();
    }
    let mut output: String = value.chars().take(maximum.saturating_sub(1)).collect();
    output.push('…');
    output
}

fn truncate_chars(value: &str, maximum: usize) -> String {
    if value.chars().count() <= maximum {
        return value.to_owned();
    }
    let mut output: String = value.chars().take(maximum).collect();
    output.push('…');
    output
}

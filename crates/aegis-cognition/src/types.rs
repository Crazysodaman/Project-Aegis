//! Provider-neutral cognition contracts.
//!
//! These types deliberately contain no provider-specific fields. Adapters map
//! their native wire formats into these structures and callers stay independent
//! of any single model runtime.

use std::fmt;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::error::InferenceError;

/// Maximum accepted length of a [`ProviderId`].
pub const PROVIDER_ID_MAX_LEN: usize = 128;

/// Validated, stable identifier for a cognition provider adapter.
///
/// A provider id is a short ASCII token such as `ollama` or `remote-eu-1`.
/// Validation happens at construction and deserialization so downstream
/// routing and audit records can trust the value.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct ProviderId(String);

impl ProviderId {
    /// Validate and construct a provider id.
    pub fn new(value: impl AsRef<str>) -> Result<Self, InferenceError> {
        let value = value.as_ref();
        if value.is_empty() {
            return Err(InferenceError::InvalidRequest(
                "provider id must not be empty".to_owned(),
            ));
        }
        if value.len() > PROVIDER_ID_MAX_LEN {
            return Err(InferenceError::InvalidRequest(format!(
                "provider id exceeds {PROVIDER_ID_MAX_LEN} bytes"
            )));
        }
        let valid = value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'));
        if !valid {
            return Err(InferenceError::InvalidRequest(
                "provider id may only contain ASCII letters, digits, '-', '_', '.', and ':'"
                    .to_owned(),
            ));
        }
        Ok(Self(value.to_owned()))
    }

    /// Borrow the identifier as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume the identifier and return the owned string.
    pub fn into_string(self) -> String {
        self.0
    }

    /// Construct an identifier from a compile-time-known valid token.
    pub(crate) fn from_trusted(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl<'de> Deserialize<'de> for ProviderId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(&value).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl AsRef<str> for ProviderId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl FromStr for ProviderId {
    type Err = InferenceError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

/// Coarse classification of where a provider runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// A model endpoint running on the local host or local network.
    Local,
    /// A model endpoint operated by a third party.
    Remote,
}

impl ProviderKind {
    /// Stable lowercase name suitable for diagnostics and logs.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
        }
    }
}

/// Capability metadata for one model exposed by a provider.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInfo {
    /// Provider-native model name, for example `llama3:8b`.
    pub name: String,
    /// On-disk or provider-reported model size in bytes when known.
    #[serde(default)]
    pub size_bytes: u64,
    /// Model family supplied by the provider.
    #[serde(default)]
    pub family: String,
    /// Parameter count as a display string, for example `8B`.
    #[serde(default)]
    pub parameter_size: String,
    /// Quantization label, for example `Q4_K_M`.
    #[serde(default)]
    pub quantization_level: String,
}

/// Result of a lightweight provider reachability probe.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderHealth {
    /// Whether the provider answered its health probe.
    pub reachable: bool,
    /// Provider version string when it exposes one.
    #[serde(default)]
    pub version: Option<String>,
    /// Number of models reported during the probe.
    #[serde(default)]
    pub model_count: usize,
    /// Wall-clock duration of the probe in milliseconds.
    #[serde(default)]
    pub latency_ms: u64,
    /// Normalized error text when the probe was degraded or failed.
    #[serde(default)]
    pub error: Option<String>,
}

/// Conversation role carried by a [`ChatMessage`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    /// System instructions.
    System,
    /// End-user input.
    User,
    /// Prior model output.
    Assistant,
    /// Result of a previously requested tool.
    Tool,
}

impl ChatRole {
    /// Stable lowercase wire name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }
}

/// One message in a provider-neutral conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Role of the message author.
    pub role: ChatRole,
    /// Text content of the message.
    pub content: String,
}

impl ChatMessage {
    /// Construct a message from a role and text.
    pub fn new(role: ChatRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
        }
    }

    /// Construct a system message.
    pub fn system(content: impl Into<String>) -> Self {
        Self::new(ChatRole::System, content)
    }

    /// Construct a user message.
    pub fn user(content: impl Into<String>) -> Self {
        Self::new(ChatRole::User, content)
    }

    /// Construct an assistant message.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new(ChatRole::Assistant, content)
    }

    /// Construct a tool-result message.
    pub fn tool(content: impl Into<String>) -> Self {
        Self::new(ChatRole::Tool, content)
    }
}

/// Declaration of a tool the model may request.
///
/// This crate only declares and transports tool schemas. It never executes a
/// tool. Enforcement of permissions and actual execution belong to Platform
/// and its capability broker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// Tool name exposed to the model.
    pub name: String,
    /// Human-readable description of what the tool does.
    #[serde(default)]
    pub description: String,
    /// JSON Schema describing the tool arguments.
    #[serde(default)]
    pub parameters: Value,
}

impl ToolDefinition {
    /// Construct a tool declaration with no parameter schema yet.
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters: Value::Null,
        }
    }
}

/// A tool invocation requested by the model.
///
/// The call is data only. Executing it requires an explicit Platform
/// permission decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    /// Name of the requested tool.
    pub name: String,
    /// Arguments the model supplied for the tool.
    pub arguments: Value,
}

/// One provider-neutral chat completion request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatRequest {
    /// Model name to target.
    pub model: String,
    /// Ordered conversation messages.
    #[serde(default)]
    pub messages: Vec<ChatMessage>,
    /// Sampling temperature when the caller wants to override provider default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Maximum generated tokens when the caller wants a bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Per-request timeout in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Tool declarations available to the model.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDefinition>,
    /// Provider-specific thinking-mode override where supported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub think: Option<bool>,
}

impl ChatRequest {
    /// Construct a request with provider defaults and no tools.
    pub fn new(model: impl Into<String>, messages: Vec<ChatMessage>) -> Self {
        Self {
            model: model.into(),
            messages,
            temperature: None,
            max_tokens: None,
            timeout_ms: None,
            tools: Vec::new(),
            think: None,
        }
    }

    /// Set the sampling temperature.
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }

    /// Set the maximum generated token count.
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    /// Set the per-request timeout in milliseconds.
    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = Some(timeout_ms);
        self
    }

    /// Attach tool declarations.
    pub fn with_tools(mut self, tools: Vec<ToolDefinition>) -> Self {
        self.tools = tools;
        self
    }

    /// Override provider thinking mode where supported (for example Ollama).
    pub fn with_think(mut self, think: Option<bool>) -> Self {
        self.think = think;
        self
    }
}

/// Completed response from a non-streaming chat request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatResponse {
    /// Model that produced the response.
    pub model: String,
    /// Complete generated text.
    pub content: String,
    /// Whether the provider marked generation complete.
    pub done: bool,
    /// Total model time reported by the provider, normalized to milliseconds.
    #[serde(default)]
    pub total_duration_ms: u64,
    /// Prompt token count when reported.
    #[serde(default)]
    pub prompt_eval_count: u64,
    /// Generated token count when reported.
    #[serde(default)]
    pub eval_count: u64,
    /// Tool calls requested by the model, if any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
}

/// Incremental piece of a streaming chat response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatChunk {
    /// Model that produced the chunk.
    pub model: String,
    /// Incremental text appended by this chunk.
    pub delta: String,
    /// Whether this chunk terminates the stream.
    pub done: bool,
    /// Tool calls delivered with this chunk, if any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
}

/// Cooperative cancellation handle shared with an in-flight request.
///
/// Providers check the token before opening a connection and between streamed
/// chunks, so a cancelled request stops promptly without requiring the caller
/// to abort a thread.
#[derive(Debug, Clone, Default)]
pub struct CancelToken {
    cancelled: Arc<AtomicBool>,
}

impl CancelToken {
    /// Create a fresh, uncancelled token.
    pub fn new() -> Self {
        Self::default()
    }

    /// Request cancellation.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

//! Per-Core conversation persistence and provider-independent interaction.
//!
//! See [`store`] for the isolated per-Core database and [`engine`] for the
//! assembly of identity, personality, private memory, and model output.
//!
//! Security posture:
//!
//! * Conversation data lives only in the owning Core's directory and is bound
//!   to that Core's immutable UUID and database instance UUID.
//! * Tool calls are stored and surfaced as data; this crate never executes a
//!   tool and never grants permission.
//! * Personality and memory are descriptive context. They are never
//!   authorization inputs.

pub mod embedding_retrieval;
pub mod engine;
pub mod error;
pub mod retrieval;
pub mod store;

pub use embedding_retrieval::{EmbeddingRetriever, MAX_SEMANTIC_CANDIDATES};
pub use engine::{
    AssistantTurn, ConversationEngine, ConversationTurn, MemoryReference, PreparedTurn,
    ToolExecutionRecord, TurnEvent, TurnOptions, DEFAULT_HISTORY_LIMIT, DEFAULT_MEMORY_LIMIT,
    MAX_MEMORY_SNIPPET_CHARS, MAX_SYSTEM_PROMPT_CHARS,
};
pub use error::ConversationError;
pub use retrieval::{KeywordRetriever, MemoryRetriever, RetrievalOutcome};
pub use store::{
    ConversationArchive, ConversationExport, ConversationMessage, ConversationSession,
    ConversationStore, MessageRole, NewMessage, SessionEvent, SessionState, ARCHIVE_SCHEMA,
    CONVERSATION_APPLICATION_ID, CONVERSATION_COMPONENT, CONVERSATION_DB_FILE, EXPORT_SCHEMA,
    MAX_MESSAGES_PER_READ, MAX_MESSAGE_CHARS, MAX_METADATA_BYTES, MAX_SESSIONS_PER_ARCHIVE,
    MAX_TITLE_CHARS, MESSAGE_SCHEMA, SESSION_EVENT_SCHEMA, SESSION_SCHEMA,
};

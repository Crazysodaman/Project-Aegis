//! Provider-independent cognition contracts and a local Ollama provider.
//!
//! # Provider independence
//!
//! The public surface is deliberately provider-neutral. [`ModelProvider`]
//! exposes identity, health, model discovery, non-streaming chat, and streaming
//! chat through types defined in this crate. Core identity, memory, and routing
//! therefore never depend on Ollama (or any specific model runtime): an adapter
//! for another backend only has to translate its wire format into
//! [`ChatRequest`], [`ChatResponse`], and [`ChatChunk`].
//!
//! [`ProviderId`] is validated at construction and deserialization so audit and
//! routing records always carry a well-formed identifier. [`ProviderKind`]
//! distinguishes local endpoints from remote providers without encoding
//! provider-specific detail.
//!
//! # Streaming
//!
//! [`ModelProvider::chat_stream`] consumes newline-delimited JSON and invokes a
//! callback once per chunk in arrival order. The callback may abort the stream
//! by returning an [`InferenceError`]. The method also returns an aggregated
//! [`ChatResponse`] so callers that only care about the final text can ignore
//! the chunk stream.
//!
//! # Cancellation
//!
//! Requests accept a [`CancelToken`]. Providers check it before opening a
//! socket, after writing the request, and between streamed chunks. Cancellation
//! never requires killing a thread; an in-flight call returns
//! [`InferenceError::Cancelled`] at the next checkpoint.
//!
//! # Timeouts and bounds
//!
//! Every request applies a connect timeout plus read and write timeouts derived
//! from the request deadline. Response headers, non-streaming bodies, individual
//! stream lines, and total stream size are all bounded before allocation so a
//! hostile or broken endpoint cannot exhaust memory. HTTP status codes are
//! normalized and error bodies are captured with a smaller cap.
//!
//! # Tools
//!
//! [`ToolDefinition`] and [`ToolCall`] are declaration and transport types only.
//! This crate never executes a tool. Permission checks, approval, sandboxing,
//! and audit belong to Platform.
//!
//! # Live testing
//!
//! The default test suite runs against an in-process `std::net::TcpListener`
//! mock server and never contacts a real model runtime. An opt-in live test is
//! gated behind `AEGIS_OLLAMA_LIVE=1` and `AEGIS_OLLAMA_URL`. That live test was
//! **not exercised** while producing this crate, so no claim is made about a
//! real Ollama installation in this environment.
//!
//! # Transport limitations
//!
//! The built-in Ollama adapter speaks HTTP/1.1 directly over
//! [`std::net::TcpStream`]. It supports `http://` only. `https://` is rejected
//! with [`InferenceError::UnsupportedScheme`] because this crate does not link a
//! TLS implementation; terminating TLS in a local proxy is the intended path.

#![forbid(unsafe_code)]

pub mod error;
pub mod ollama;
pub mod provider;
pub mod types;

pub use error::InferenceError;
pub use ollama::{OllamaProvider, DEFAULT_BASE_URL, DEFAULT_OLLAMA_BASE_URL};
pub use provider::ModelProvider;
pub use types::{
    CancelToken, ChatChunk, ChatMessage, ChatRequest, ChatResponse, ChatRole, ModelInfo,
    ProviderHealth, ProviderId, ProviderKind, ToolCall, ToolDefinition, PROVIDER_ID_MAX_LEN,
};

/// Version marker for the provider-neutral cognition contract.
pub const COGNITION_CONTRACT_VERSION: &str = "aegis.cognition.v1";

/// Schema identifier for serialized provider descriptors and diagnostics.
pub const PROVIDER_SCHEMA: &str = "aegis.cognition.provider.v1";

//! Provider-independent cognition trait.

use crate::error::InferenceError;
use crate::types::{
    CancelToken, ChatChunk, ChatRequest, ChatResponse, ModelInfo, ProviderHealth, ProviderId,
    ProviderKind,
};

/// A model backend that can answer chat requests.
///
/// Implementations translate these provider-neutral calls into their native
/// wire protocol. Core identity, memory, and routing code depend only on this
/// trait, so replacing a model provider never changes stored identity.
pub trait ModelProvider: Send + Sync {
    /// Stable identifier for this provider instance.
    fn id(&self) -> &ProviderId;

    /// Whether the provider is local or remote.
    fn kind(&self) -> ProviderKind;

    /// Probe reachability, version, and model count.
    ///
    /// Implementations should not return an error; degraded health is carried
    /// in [`ProviderHealth::error`] so callers can display it without special
    /// handling.
    fn health(&self) -> ProviderHealth;

    /// List the models the provider currently exposes.
    fn list_models(&self) -> Result<Vec<ModelInfo>, InferenceError>;

    /// Perform a complete, non-streaming chat request.
    fn chat(
        &self,
        request: &ChatRequest,
        cancel: &CancelToken,
    ) -> Result<ChatResponse, InferenceError>;

    /// Perform a streaming chat request.
    ///
    /// `on_chunk` is invoked once per provider chunk in arrival order. Returning
    /// an error from the callback aborts the stream and propagates the error.
    /// The returned [`ChatResponse`] aggregates the full streamed result.
    fn chat_stream(
        &self,
        request: &ChatRequest,
        cancel: &CancelToken,
        on_chunk: &mut dyn FnMut(ChatChunk) -> Result<(), InferenceError>,
    ) -> Result<ChatResponse, InferenceError>;
}

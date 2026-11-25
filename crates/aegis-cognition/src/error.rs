//! Normalized inference error surface shared by every provider adapter.

use thiserror::Error;

/// Errors returned by cognition providers.
///
/// Every provider maps its native failures into this single vocabulary so
/// callers do not depend on a particular inference backend.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InferenceError {
    /// The configured endpoint URL is syntactically invalid.
    #[error("invalid provider URL: {0}")]
    InvalidUrl(String),
    /// The URL scheme is not supported by the adapter.
    ///
    /// The built-in Ollama adapter only speaks plain `http://` because this
    /// crate does not link a TLS implementation.
    #[error("unsupported provider URL scheme: {0}")]
    UnsupportedScheme(String),
    /// A socket, DNS, or transport-level failure occurred.
    #[error("network error: {0}")]
    Network(String),
    /// A connect, read, or write operation exceeded its deadline.
    #[error("request timed out: {0}")]
    Timeout(String),
    /// The provider answered with a non-success HTTP status.
    #[error("provider returned HTTP status {status}: {body}")]
    HttpStatus {
        /// HTTP status code.
        status: u16,
        /// Bounded response body captured for diagnostics.
        body: String,
    },
    /// The provider answered with a body that could not be interpreted.
    #[error("invalid provider response: {0}")]
    InvalidResponse(String),
    /// The caller cancelled the request through a [`crate::CancelToken`].
    #[error("request was cancelled")]
    Cancelled,
    /// The provider reported a semantic error in its own response body.
    #[error("provider error: {0}")]
    Provider(String),
    /// The request itself was rejected before being sent.
    #[error("invalid request: {0}")]
    InvalidRequest(String),
}

impl InferenceError {
    /// Return `true` when this error represents cancellation.
    pub const fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }
}

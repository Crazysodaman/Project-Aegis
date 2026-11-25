//! Error types for the local Aegis dashboards.

use std::net::SocketAddr;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum UiError {
    #[error("dashboard bind address {0} is not a loopback address")]
    NonLoopback(SocketAddr),
    #[error("dashboard I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("dashboard JSON encoding failed: {0}")]
    Json(#[from] serde_json::Error),
}

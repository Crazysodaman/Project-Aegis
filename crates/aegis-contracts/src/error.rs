use thiserror::Error;

#[derive(Debug, Error)]
pub enum ContractError {
    #[error("request schema is unsupported")]
    UnsupportedSchema,
    #[error("protocol version is unsupported")]
    UnsupportedProtocol,
    #[error("request field is missing or invalid: {0}")]
    InvalidField(&'static str),
    #[error("request exceeds the maximum frame size")]
    FrameTooLarge,
    #[error("request is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
}

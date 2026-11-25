//! Versioned local JSON-RPC envelope contracts.
//!
//! The envelope is authority-free: it carries a caller-supplied owner proof,
//! but only the Safe boundary may verify it.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::error::ContractError;

pub const IPC_REQUEST_SCHEMA: &str = "aegis.ipc.request.v1";
pub const IPC_RESPONSE_SCHEMA: &str = "aegis.ipc.response.v1";
pub const IPC_PROTOCOL_VERSION: &str = "1.0";
pub const MAX_FRAME_BYTES: usize = 65_536;

pub const METHOD_PLATFORM_STATUS: &str = "platform.status";
pub const METHOD_PLATFORM_SHUTDOWN: &str = "platform.shutdown";
pub const METHOD_PLATFORM_EFFECT_PING: &str = "platform.effect.ping";
pub const METHOD_SAFE_STOP_STATUS: &str = "safe.stop.status";
pub const METHOD_SAFE_STOP_SET: &str = "safe.stop.set";
pub const METHOD_SAFE_STOP_CLEAR: &str = "safe.stop.clear";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
}

impl ClientInfo {
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthContext {
    pub owner_token: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IpcRequest {
    pub schema: String,
    pub protocol: String,
    pub request_id: String,
    pub method: String,
    pub params: Value,
    pub client: ClientInfo,
    pub auth: AuthContext,
}

impl IpcRequest {
    pub fn new(method: impl Into<String>, params: Value, owner_token: impl Into<String>) -> Self {
        Self {
            schema: IPC_REQUEST_SCHEMA.to_owned(),
            protocol: IPC_PROTOCOL_VERSION.to_owned(),
            request_id: Uuid::new_v4().to_string(),
            method: method.into(),
            params,
            client: ClientInfo::new("aegis-cli", env!("CARGO_PKG_VERSION")),
            auth: AuthContext {
                owner_token: owner_token.into(),
            },
        }
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema != IPC_REQUEST_SCHEMA {
            return Err(ContractError::UnsupportedSchema);
        }
        if self.protocol != IPC_PROTOCOL_VERSION {
            return Err(ContractError::UnsupportedProtocol);
        }
        validate_identifier(&self.request_id, 128, "request_id")?;
        validate_identifier(&self.method, 128, "method")?;
        if !self.params.is_object() {
            return Err(ContractError::InvalidField("params"));
        }
        validate_text(&self.client.name, 128, "client.name")?;
        validate_text(&self.client.version, 64, "client.version")?;
        validate_text(&self.auth.owner_token, 8192, "auth.owner_token")?;
        Ok(())
    }

    pub fn from_line(line: &str) -> Result<Self, ContractError> {
        if line.len() > MAX_FRAME_BYTES {
            return Err(ContractError::FrameTooLarge);
        }
        let request: Self = serde_json::from_str(line.trim())?;
        request.validate()?;
        Ok(request)
    }

    pub fn to_line(&self) -> Result<String, ContractError> {
        self.validate()?;
        let encoded = serde_json::to_string(self)?;
        if encoded.len() > MAX_FRAME_BYTES {
            return Err(ContractError::FrameTooLarge);
        }
        Ok(encoded)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IpcError {
    pub code: String,
    pub message: String,
}

impl IpcError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IpcResponse {
    pub schema: String,
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<IpcError>,
}

impl IpcResponse {
    pub fn success(request_id: impl Into<String>, result: Value) -> Self {
        Self {
            schema: IPC_RESPONSE_SCHEMA.to_owned(),
            request_id: request_id.into(),
            result: Some(result),
            error: None,
        }
    }

    pub fn failure(
        request_id: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            schema: IPC_RESPONSE_SCHEMA.to_owned(),
            request_id: request_id.into(),
            result: None,
            error: Some(IpcError::new(code, message)),
        }
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema != IPC_RESPONSE_SCHEMA {
            return Err(ContractError::UnsupportedSchema);
        }
        validate_identifier(&self.request_id, 128, "request_id")?;
        match (&self.result, &self.error) {
            (Some(_), None) | (None, Some(_)) => Ok(()),
            _ => Err(ContractError::InvalidField("result/error")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcMethod {
    PlatformStatus,
    PlatformShutdown,
    PlatformEffectPing,
    SafeStopStatus,
    SafeStopSet,
    SafeStopClear,
}

impl IpcMethod {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PlatformStatus => METHOD_PLATFORM_STATUS,
            Self::PlatformShutdown => METHOD_PLATFORM_SHUTDOWN,
            Self::PlatformEffectPing => METHOD_PLATFORM_EFFECT_PING,
            Self::SafeStopStatus => METHOD_SAFE_STOP_STATUS,
            Self::SafeStopSet => METHOD_SAFE_STOP_SET,
            Self::SafeStopClear => METHOD_SAFE_STOP_CLEAR,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            METHOD_PLATFORM_STATUS => Some(Self::PlatformStatus),
            METHOD_PLATFORM_SHUTDOWN => Some(Self::PlatformShutdown),
            METHOD_PLATFORM_EFFECT_PING => Some(Self::PlatformEffectPing),
            METHOD_SAFE_STOP_STATUS => Some(Self::SafeStopStatus),
            METHOD_SAFE_STOP_SET => Some(Self::SafeStopSet),
            METHOD_SAFE_STOP_CLEAR => Some(Self::SafeStopClear),
            _ => None,
        }
    }
}

fn validate_text(value: &str, maximum: usize, field: &'static str) -> Result<(), ContractError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > maximum {
        return Err(ContractError::InvalidField(field));
    }
    Ok(())
}

fn validate_identifier(
    value: &str,
    maximum: usize,
    field: &'static str,
) -> Result<(), ContractError> {
    validate_text(value, maximum, field)?;
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '/'))
    {
        return Err(ContractError::InvalidField(field));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_round_trip_is_valid() {
        let request = IpcRequest::new(METHOD_PLATFORM_STATUS, json!({}), "dev-token");
        request.validate().expect("valid request");
        let line = request.to_line().expect("serializes");
        let parsed = IpcRequest::from_line(&line).expect("parses");
        assert_eq!(parsed.method, METHOD_PLATFORM_STATUS);
    }

    #[test]
    fn wrong_protocol_is_rejected() {
        let mut request = IpcRequest::new(METHOD_PLATFORM_STATUS, json!({}), "dev-token");
        request.protocol = "9.9".to_owned();
        assert!(matches!(
            request.validate(),
            Err(ContractError::UnsupportedProtocol)
        ));
    }

    #[test]
    fn non_object_params_are_rejected() {
        let request = IpcRequest::new(METHOD_PLATFORM_STATUS, json!(["bad"]), "dev-token");
        assert!(matches!(
            request.validate(),
            Err(ContractError::InvalidField("params"))
        ));
    }

    #[test]
    fn oversized_frame_is_rejected() {
        let oversized = "x".repeat(MAX_FRAME_BYTES + 1);
        assert!(matches!(
            IpcRequest::from_line(&oversized),
            Err(ContractError::FrameTooLarge)
        ));
    }
}

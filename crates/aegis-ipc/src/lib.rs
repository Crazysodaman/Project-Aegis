//! Local JSON-RPC transport over Windows named pipes or Unix domain sockets.
//!
//! Transport authentication is not assumed here. The Platform/Safe layer
//! verifies the owner proof carried by each request.

use std::io::{BufRead, BufReader, Write};
use std::sync::Arc;

use aegis_contracts::ipc::{
    IpcRequest, IpcResponse, MAX_FRAME_BYTES, METHOD_PLATFORM_STATUS, METHOD_SAFE_STOP_CLEAR,
    METHOD_SAFE_STOP_SET, METHOD_SAFE_STOP_STATUS,
};
use aegis_contracts::ContractError;
use aegis_platform::{PlatformService, ServiceError};
use interprocess::local_socket::{
    prelude::*, GenericFilePath, GenericNamespaced, ListenerOptions, Name, Stream,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum IpcError {
    #[error("local transport failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON framing failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("IPC contract failed: {0}")]
    Contract(#[from] ContractError),
}

pub struct IpcServer {
    service: Arc<PlatformService>,
    name: String,
}

impl IpcServer {
    pub fn new(service: Arc<PlatformService>, name: impl Into<String>) -> Self {
        Self {
            service,
            name: name.into(),
        }
    }

    /// Serve accepted connections until `max_requests` is reached or a
    /// graceful shutdown is requested. `0` means serve until shutdown.
    pub fn serve(&self, max_requests: usize) -> Result<(), IpcError> {
        let listener = ListenerOptions::new()
            .name(socket_name(&self.name)?)
            .create_sync()?;
        let mut served = 0usize;
        while max_requests == 0 || served < max_requests {
            if self.service.shutdown_requested() {
                break;
            }
            let stream = listener.accept()?;
            handle_connection(stream, &self.service)?;
            served += 1;
        }
        Ok(())
    }
}

pub struct IpcClient {
    name: String,
}

impl IpcClient {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }

    pub fn call(&self, request: &IpcRequest) -> Result<IpcResponse, IpcError> {
        self.call_line(&request.to_line()?)
    }

    pub fn call_line(&self, line: &str) -> Result<IpcResponse, IpcError> {
        let mut stream = Stream::connect(socket_name(&self.name)?)?;
        stream.write_all(line.as_bytes())?;
        stream.write_all(b"\n")?;
        let mut reader = BufReader::new(stream);
        let mut response_line = String::new();
        reader.read_line(&mut response_line)?;
        if response_line.len() > MAX_FRAME_BYTES {
            return Err(IpcError::Contract(ContractError::FrameTooLarge));
        }
        let response: IpcResponse = serde_json::from_str(response_line.trim())?;
        response.validate()?;
        Ok(response)
    }
}

fn handle_connection(stream: Stream, service: &PlatformService) -> Result<(), IpcError> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let response = response_for(service, &line);
    let encoded = serde_json::to_string(&response)?;
    reader.get_mut().write_all(encoded.as_bytes())?;
    reader.get_mut().write_all(b"\n")?;
    Ok(())
}

fn response_for(service: &PlatformService, line: &str) -> IpcResponse {
    if line.trim().is_empty() {
        return IpcResponse::failure("unknown", "invalid_request", "empty request");
    }
    let request = match IpcRequest::from_line(line) {
        Ok(request) => request,
        Err(error) => {
            return IpcResponse::failure("unknown", contract_code(&error), error.to_string());
        }
    };
    match service.dispatch(&request.method, &request.params, &request.auth.owner_token) {
        Ok(result) => IpcResponse::success(request.request_id, result),
        Err(error) => {
            IpcResponse::failure(request.request_id, service_code(&error), error.to_string())
        }
    }
}

fn contract_code(error: &ContractError) -> &'static str {
    match error {
        ContractError::UnsupportedSchema => "unsupported_schema",
        ContractError::UnsupportedProtocol => "unsupported_protocol",
        ContractError::FrameTooLarge => "frame_too_large",
        ContractError::InvalidField(_) => "invalid_request",
        ContractError::Json(_) => "invalid_json",
    }
}

fn service_code(error: &ServiceError) -> &'static str {
    error.code()
}

fn socket_name(value: &str) -> Result<Name<'_>, IpcError> {
    if GenericNamespaced::is_supported() {
        Ok(value.to_ns_name::<GenericNamespaced>()?)
    } else {
        Ok(value.to_fs_name::<GenericFilePath>()?)
    }
}

pub fn request_status(token: &str) -> IpcRequest {
    IpcRequest::new(METHOD_PLATFORM_STATUS, serde_json::json!({}), token)
}

pub fn request_stop_status(token: &str) -> IpcRequest {
    IpcRequest::new(METHOD_SAFE_STOP_STATUS, serde_json::json!({}), token)
}

pub fn request_stop_set(token: &str, reason: &str) -> IpcRequest {
    IpcRequest::new(
        METHOD_SAFE_STOP_SET,
        serde_json::json!({"reason": reason}),
        token,
    )
}

pub fn request_stop_clear(token: &str) -> IpcRequest {
    IpcRequest::new(METHOD_SAFE_STOP_CLEAR, serde_json::json!({}), token)
}

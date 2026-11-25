//! Ollama provider implemented with a small blocking HTTP/1.1 client.
//!
//! The client intentionally uses only [`std::net::TcpStream`] and JSON so the
//! crate has no network dependency and no TLS stack. Consequently only plain
//! `http://` endpoints are accepted; `https://` is rejected up front.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use serde_json::{Map, Value};

use crate::error::InferenceError;
use crate::provider::ModelProvider;
use crate::types::{
    CancelToken, ChatChunk, ChatMessage, ChatRequest, ChatResponse, ModelInfo, ProviderHealth,
    ProviderId, ProviderKind, ToolCall, ToolDefinition,
};
use crate::COGNITION_CONTRACT_VERSION;

/// Default local Ollama endpoint.
pub const DEFAULT_OLLAMA_BASE_URL: &str = "http://127.0.0.1:11434";

/// Short alias for [`DEFAULT_OLLAMA_BASE_URL`].
pub const DEFAULT_BASE_URL: &str = DEFAULT_OLLAMA_BASE_URL;

const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_HEADER_LINE_BYTES: usize = 8 * 1024;
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;
const MAX_ERROR_BODY_BYTES: usize = 64 * 1024;
const MAX_STREAM_LINE_BYTES: usize = 1024 * 1024;
const MAX_STREAM_BYTES: usize = 64 * 1024 * 1024;
const DEFAULT_HEALTH_TIMEOUT_MS: u64 = 5_000;
const DEFAULT_CHAT_TIMEOUT_MS: u64 = 120_000;
const NANOS_PER_MILLI: u64 = 1_000_000;

/// Parsed base URL components for the local HTTP client.
#[derive(Debug, Clone)]
struct Endpoint {
    host: String,
    port: u16,
    host_header: String,
    path_prefix: String,
}

impl Endpoint {
    fn parse(raw: &str) -> Result<Self, InferenceError> {
        if raw.is_empty() {
            return Err(InferenceError::InvalidUrl("URL is empty".to_owned()));
        }
        if raw
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
        {
            return Err(InferenceError::InvalidUrl(
                "URL contains whitespace or control characters".to_owned(),
            ));
        }
        let (scheme, rest) = raw.split_once("://").ok_or_else(|| {
            InferenceError::InvalidUrl("URL must include an explicit scheme".to_owned())
        })?;
        let scheme = scheme.to_ascii_lowercase();
        if scheme != "http" {
            return Err(InferenceError::UnsupportedScheme(format!(
                "{scheme}:// (only http:// is supported; TLS is not implemented)"
            )));
        }
        let (authority, path) = match rest.find('/') {
            Some(index) => (&rest[..index], &rest[index..]),
            None => (rest, ""),
        };
        if authority.is_empty() {
            return Err(InferenceError::InvalidUrl(
                "URL is missing a host".to_owned(),
            ));
        }
        if authority.contains('@') {
            return Err(InferenceError::InvalidUrl(
                "URL userinfo is not supported".to_owned(),
            ));
        }
        if authority.contains(['?', '#']) {
            return Err(InferenceError::InvalidUrl(
                "URL query strings and fragments are not supported".to_owned(),
            ));
        }
        if path.contains('?') || path.contains('#') {
            return Err(InferenceError::InvalidUrl(
                "URL query strings and fragments are not supported".to_owned(),
            ));
        }
        let (host, port) = parse_authority(authority)?;
        let path_prefix = path.trim_end_matches('/').to_owned();
        Ok(Self {
            host,
            port,
            host_header: authority.to_owned(),
            path_prefix,
        })
    }

    fn path(&self, suffix: &str) -> String {
        format!("{}{}", self.path_prefix, suffix)
    }
}

fn parse_authority(authority: &str) -> Result<(String, u16), InferenceError> {
    if let Some(stripped) = authority.strip_prefix('[') {
        let end = stripped.find(']').ok_or_else(|| {
            InferenceError::InvalidUrl("IPv6 host is missing its closing bracket".to_owned())
        })?;
        let host = &stripped[..end];
        let remainder = &stripped[end + 1..];
        if host.is_empty() {
            return Err(InferenceError::InvalidUrl(
                "URL is missing a host".to_owned(),
            ));
        }
        let port = if remainder.is_empty() {
            80
        } else {
            let raw_port = remainder.strip_prefix(':').ok_or_else(|| {
                InferenceError::InvalidUrl("invalid characters after IPv6 host".to_owned())
            })?;
            parse_port(raw_port)?
        };
        Ok((host.to_owned(), port))
    } else {
        if authority.matches(':').count() > 1 {
            return Err(InferenceError::InvalidUrl(
                "IPv6 hosts must be wrapped in brackets".to_owned(),
            ));
        }
        match authority.rsplit_once(':') {
            Some((host, raw_port)) => {
                if host.is_empty() {
                    return Err(InferenceError::InvalidUrl(
                        "URL is missing a host".to_owned(),
                    ));
                }
                Ok((host.to_owned(), parse_port(raw_port)?))
            }
            None => Ok((authority.to_owned(), 80)),
        }
    }
}

fn parse_port(raw: &str) -> Result<u16, InferenceError> {
    if raw.is_empty() {
        return Err(InferenceError::InvalidUrl("URL port is empty".to_owned()));
    }
    let port: u16 = raw
        .parse()
        .map_err(|_| InferenceError::InvalidUrl(format!("invalid URL port: {raw}")))?;
    if port == 0 {
        return Err(InferenceError::InvalidUrl(
            "URL port must be non-zero".to_owned(),
        ));
    }
    Ok(port)
}

/// A blocking Ollama model provider.
#[derive(Debug, Clone)]
pub struct OllamaProvider {
    base_url: String,
    endpoint: Endpoint,
    id: ProviderId,
    kind: ProviderKind,
}

impl OllamaProvider {
    /// Construct a provider for an `http://` Ollama base URL.
    ///
    /// Pass [`DEFAULT_OLLAMA_BASE_URL`] to target a stock local installation.
    pub fn new(base_url: impl AsRef<str>) -> Result<Self, InferenceError> {
        let raw = base_url.as_ref().trim();
        let endpoint = Endpoint::parse(raw)?;
        let id = ProviderId::new("ollama")?;
        let base_url = raw.trim_end_matches('/').to_owned();
        let base_url = if base_url.is_empty() {
            raw.to_owned()
        } else {
            base_url
        };
        Ok(Self {
            base_url,
            endpoint,
            id,
            kind: ProviderKind::Local,
        })
    }

    /// Base URL this provider was constructed with.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// List models currently loaded/resident in the Ollama runtime.
    pub fn running_models(&self) -> Result<Vec<String>, InferenceError> {
        let timeout = Duration::from_millis(DEFAULT_HEALTH_TIMEOUT_MS);
        let value = self.get_json("/api/ps", timeout)?;
        let models = value
            .get("models")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                InferenceError::InvalidResponse(
                    "Ollama /api/ps response is missing a models array".to_owned(),
                )
            })?;
        Ok(models
            .iter()
            .filter_map(|model| model.get("name").and_then(Value::as_str))
            .map(str::to_owned)
            .collect())
    }

    /// Ask Ollama to unload a model immediately (`keep_alive: 0`).
    ///
    /// This is a cooperative yield mechanism. It does not guarantee that a
    /// currently generating request is preempted.
    pub fn unload_model(&self, model: &str) -> Result<(), InferenceError> {
        let model = model.trim();
        if model.is_empty() || model.chars().any(char::is_control) {
            return Err(InferenceError::InvalidRequest(
                "model name is required".to_owned(),
            ));
        }
        let body = serde_json::to_vec(&serde_json::json!({
            "model": model,
            "keep_alive": 0,
        }))
        .map_err(|error| InferenceError::InvalidRequest(error.to_string()))?;
        let cancel = CancelToken::new();
        let response = self.send(
            "POST",
            &self.endpoint.path("/api/generate"),
            Some(&body),
            Duration::from_millis(30_000),
            &cancel,
        )?;
        drop(response);
        Ok(())
    }

    fn get_json(&self, suffix: &str, timeout: Duration) -> Result<Value, InferenceError> {
        let cancel = CancelToken::new();
        let response = self.send("GET", &self.endpoint.path(suffix), None, timeout, &cancel)?;
        read_json_response(response)
    }

    fn connect(&self, timeout: Duration) -> Result<TcpStream, InferenceError> {
        let addresses = (self.endpoint.host.as_str(), self.endpoint.port)
            .to_socket_addrs()
            .map_err(|error| map_io_error("failed to resolve Ollama endpoint", error))?;
        let mut last_error = None;
        for address in addresses {
            match TcpStream::connect_timeout(&address, timeout) {
                Ok(stream) => {
                    stream
                        .set_read_timeout(Some(timeout))
                        .map_err(|error| map_io_error("failed to set read timeout", error))?;
                    stream
                        .set_write_timeout(Some(timeout))
                        .map_err(|error| map_io_error("failed to set write timeout", error))?;
                    let _ = stream.set_nodelay(true);
                    return Ok(stream);
                }
                Err(error) => last_error = Some(error),
            }
        }
        match last_error {
            Some(error) => Err(map_io_error("failed to connect to Ollama", error)),
            None => Err(InferenceError::Network(
                "Ollama endpoint resolved to no addresses".to_owned(),
            )),
        }
    }

    fn send(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        timeout: Duration,
        cancel: &CancelToken,
    ) -> Result<RawResponse, InferenceError> {
        if cancel.is_cancelled() {
            return Err(InferenceError::Cancelled);
        }
        let mut stream = self.connect(timeout)?;
        let mut request = Vec::new();
        let head = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nUser-Agent: aegis-cognition/{}\r\nAccept: application/json\r\nConnection: close\r\n",
            self.endpoint.host_header, COGNITION_CONTRACT_VERSION
        );
        request.extend_from_slice(head.as_bytes());
        if let Some(body) = body {
            let body_head = format!(
                "Content-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                body.len()
            );
            request.extend_from_slice(body_head.as_bytes());
            request.extend_from_slice(body);
        } else {
            request.extend_from_slice(b"\r\n");
        }
        stream
            .write_all(&request)
            .map_err(|error| map_io_error("failed to write Ollama request", error))?;
        stream
            .flush()
            .map_err(|error| map_io_error("failed to flush Ollama request", error))?;
        if cancel.is_cancelled() {
            return Err(InferenceError::Cancelled);
        }
        let mut reader = BufReader::new(stream);
        let (status, headers) = read_response_head(&mut reader)?;
        let body = BodyReader::new(reader, &headers)?;
        Ok(RawResponse { status, body })
    }
}

impl Default for OllamaProvider {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_OLLAMA_BASE_URL.to_owned(),
            endpoint: Endpoint {
                host: "127.0.0.1".to_owned(),
                port: 11_434,
                host_header: "127.0.0.1:11434".to_owned(),
                path_prefix: String::new(),
            },
            id: ProviderId::from_trusted("ollama"),
            kind: ProviderKind::Local,
        }
    }
}

impl ModelProvider for OllamaProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn kind(&self) -> ProviderKind {
        self.kind
    }

    fn health(&self) -> ProviderHealth {
        let started = Instant::now();
        let timeout = Duration::from_millis(DEFAULT_HEALTH_TIMEOUT_MS);
        let version = match self.get_json("/api/version", timeout) {
            Ok(value) => value
                .get("version")
                .and_then(Value::as_str)
                .map(str::to_owned),
            Err(error) => {
                return ProviderHealth {
                    reachable: false,
                    version: None,
                    model_count: 0,
                    latency_ms: elapsed_ms(started),
                    error: Some(error.to_string()),
                };
            }
        };
        let (model_count, error) = match self.get_json("/api/tags", timeout) {
            Ok(value) => (count_models(&value), None),
            Err(error) => (0, Some(error.to_string())),
        };
        ProviderHealth {
            reachable: true,
            version,
            model_count,
            latency_ms: elapsed_ms(started),
            error,
        }
    }

    fn list_models(&self) -> Result<Vec<ModelInfo>, InferenceError> {
        let timeout = Duration::from_millis(DEFAULT_HEALTH_TIMEOUT_MS);
        let value = self.get_json("/api/tags", timeout)?;
        let models = value
            .get("models")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                InferenceError::InvalidResponse(
                    "Ollama /api/tags response is missing a models array".to_owned(),
                )
            })?;
        Ok(models.iter().filter_map(parse_model).collect())
    }

    fn chat(
        &self,
        request: &ChatRequest,
        cancel: &CancelToken,
    ) -> Result<ChatResponse, InferenceError> {
        if cancel.is_cancelled() {
            return Err(InferenceError::Cancelled);
        }
        let timeout = chat_timeout(request);
        let body = encode_chat_request(request, false)?;
        let response = self.send(
            "POST",
            &self.endpoint.path("/api/chat"),
            Some(&body),
            timeout,
            cancel,
        )?;
        let value = read_json_response(response)?;
        parse_chat_response(&value, &request.model)
    }

    fn chat_stream(
        &self,
        request: &ChatRequest,
        cancel: &CancelToken,
        on_chunk: &mut dyn FnMut(ChatChunk) -> Result<(), InferenceError>,
    ) -> Result<ChatResponse, InferenceError> {
        if cancel.is_cancelled() {
            return Err(InferenceError::Cancelled);
        }
        let timeout = chat_timeout(request);
        let body = encode_chat_request(request, true)?;
        let response = self.send(
            "POST",
            &self.endpoint.path("/api/chat"),
            Some(&body),
            timeout,
            cancel,
        )?;
        if !(200..300).contains(&response.status) {
            let mut body = response.body;
            let text = read_body(&mut body, MAX_ERROR_BODY_BYTES, false)?;
            return Err(InferenceError::HttpStatus {
                status: response.status,
                body: String::from_utf8_lossy(&text).trim().to_owned(),
            });
        }
        let mut reader = BufReader::new(response.body);
        let mut line = Vec::new();
        let mut aggregate = ChatAggregate::new(request.model.clone());
        let mut total_bytes = 0usize;
        let mut saw_chunk = false;
        loop {
            if cancel.is_cancelled() {
                return Err(InferenceError::Cancelled);
            }
            line.clear();
            let read = read_stream_line(&mut reader, &mut line)?;
            if read == 0 {
                break;
            }
            total_bytes = total_bytes.saturating_add(read);
            if total_bytes > MAX_STREAM_BYTES {
                return Err(InferenceError::InvalidResponse(format!(
                    "stream exceeded {MAX_STREAM_BYTES} bytes"
                )));
            }
            let trimmed = trim_ascii(&line);
            if trimmed.is_empty() {
                continue;
            }
            let value: Value = serde_json::from_slice(trimmed).map_err(|error| {
                InferenceError::InvalidResponse(format!("stream chunk is not valid JSON: {error}"))
            })?;
            if let Some(message) = value.get("error").and_then(Value::as_str) {
                return Err(InferenceError::Provider(message.to_owned()));
            }
            let chunk = parse_stream_chunk(&value, &aggregate.model);
            aggregate.absorb(&value, &chunk);
            saw_chunk = true;
            let done = chunk.done;
            on_chunk(chunk)?;
            if cancel.is_cancelled() {
                return Err(InferenceError::Cancelled);
            }
            if done {
                break;
            }
        }
        if !saw_chunk {
            return Err(InferenceError::InvalidResponse(
                "Ollama stream contained no chunks".to_owned(),
            ));
        }
        Ok(aggregate.finish())
    }
}

fn chat_timeout(request: &ChatRequest) -> Duration {
    Duration::from_millis(request.timeout_ms.unwrap_or(DEFAULT_CHAT_TIMEOUT_MS).max(1))
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn nanos_to_millis(value: Option<&Value>) -> Option<u64> {
    value
        .and_then(Value::as_u64)
        .map(|nanos| nanos / NANOS_PER_MILLI)
}

fn count_models(value: &Value) -> usize {
    value
        .get("models")
        .and_then(Value::as_array)
        .map_or(0, Vec::len)
}

fn parse_model(value: &Value) -> Option<ModelInfo> {
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .or_else(|| value.get("model").and_then(Value::as_str))?;
    if name.is_empty() {
        return None;
    }
    let details = value.get("details");
    Some(ModelInfo {
        name: name.to_owned(),
        size_bytes: value.get("size").and_then(Value::as_u64).unwrap_or(0),
        family: detail_string(details, "family"),
        parameter_size: detail_string(details, "parameter_size"),
        quantization_level: detail_string(details, "quantization_level"),
    })
}

fn detail_string(details: Option<&Value>, key: &str) -> String {
    details
        .and_then(|details| details.get(key))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

fn parse_tool_calls(value: Option<&Value>) -> Vec<ToolCall> {
    let Some(items) = value.and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let function = item.get("function").unwrap_or(item);
            let name = function.get("name").and_then(Value::as_str)?;
            if name.is_empty() {
                return None;
            }
            let arguments = match function.get("arguments") {
                None | Some(Value::Null) => Value::Object(Map::new()),
                Some(Value::String(raw)) => {
                    serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.clone()))
                }
                Some(other) => other.clone(),
            };
            Some(ToolCall {
                name: name.to_owned(),
                arguments,
            })
        })
        .collect()
}

fn parse_chat_response(
    value: &Value,
    fallback_model: &str,
) -> Result<ChatResponse, InferenceError> {
    let message = value.get("message").ok_or_else(|| {
        InferenceError::InvalidResponse("Ollama chat response is missing message".to_owned())
    })?;
    Ok(ChatResponse {
        model: value
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or(fallback_model)
            .to_owned(),
        content: message
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        done: value.get("done").and_then(Value::as_bool).unwrap_or(false),
        total_duration_ms: nanos_to_millis(value.get("total_duration")).unwrap_or(0),
        prompt_eval_count: value
            .get("prompt_eval_count")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        eval_count: value.get("eval_count").and_then(Value::as_u64).unwrap_or(0),
        tool_calls: parse_tool_calls(message.get("tool_calls")),
    })
}

fn parse_stream_chunk(value: &Value, fallback_model: &str) -> ChatChunk {
    let message = value.get("message");
    ChatChunk {
        model: value
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or(fallback_model)
            .to_owned(),
        delta: message
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        done: value.get("done").and_then(Value::as_bool).unwrap_or(false),
        tool_calls: parse_tool_calls(message.and_then(|message| message.get("tool_calls"))),
    }
}

fn encode_chat_request(request: &ChatRequest, stream: bool) -> Result<Vec<u8>, InferenceError> {
    let messages: Vec<Value> = request
        .messages
        .iter()
        .map(|message: &ChatMessage| {
            serde_json::json!({
                "role": message.role.as_str(),
                "content": &message.content,
            })
        })
        .collect();
    let mut payload = Map::new();
    payload.insert("model".to_owned(), Value::String(request.model.clone()));
    payload.insert("messages".to_owned(), Value::Array(messages));
    payload.insert("stream".to_owned(), Value::Bool(stream));
    if let Some(think) = request.think {
        payload.insert("think".to_owned(), Value::Bool(think));
    }
    let mut options = Map::new();
    if let Some(temperature) = request.temperature {
        options.insert("temperature".to_owned(), serde_json::json!(temperature));
    }
    if let Some(max_tokens) = request.max_tokens {
        options.insert("num_predict".to_owned(), serde_json::json!(max_tokens));
    }
    if !options.is_empty() {
        payload.insert("options".to_owned(), Value::Object(options));
    }
    if !request.tools.is_empty() {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|tool: &ToolDefinition| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": &tool.name,
                        "description": &tool.description,
                        "parameters": &tool.parameters,
                    }
                })
            })
            .collect();
        payload.insert("tools".to_owned(), Value::Array(tools));
    }
    serde_json::to_vec(&Value::Object(payload)).map_err(|error| {
        InferenceError::InvalidRequest(format!("failed to encode chat request: {error}"))
    })
}

struct ChatAggregate {
    model: String,
    content: String,
    done: bool,
    total_duration_ms: u64,
    prompt_eval_count: u64,
    eval_count: u64,
    tool_calls: Vec<ToolCall>,
}

impl ChatAggregate {
    fn new(model: String) -> Self {
        Self {
            model,
            content: String::new(),
            done: false,
            total_duration_ms: 0,
            prompt_eval_count: 0,
            eval_count: 0,
            tool_calls: Vec::new(),
        }
    }

    fn absorb(&mut self, value: &Value, chunk: &ChatChunk) {
        if !chunk.model.is_empty() {
            self.model.clone_from(&chunk.model);
        }
        self.content.push_str(&chunk.delta);
        self.done = chunk.done;
        self.tool_calls.extend(chunk.tool_calls.iter().cloned());
        if let Some(duration) = nanos_to_millis(value.get("total_duration")) {
            self.total_duration_ms = duration;
        }
        if let Some(count) = value.get("prompt_eval_count").and_then(Value::as_u64) {
            self.prompt_eval_count = count;
        }
        if let Some(count) = value.get("eval_count").and_then(Value::as_u64) {
            self.eval_count = count;
        }
    }

    fn finish(self) -> ChatResponse {
        ChatResponse {
            model: self.model,
            content: self.content,
            done: self.done,
            total_duration_ms: self.total_duration_ms,
            prompt_eval_count: self.prompt_eval_count,
            eval_count: self.eval_count,
            tool_calls: self.tool_calls,
        }
    }
}

struct RawResponse {
    status: u16,
    body: BodyReader<BufReader<TcpStream>>,
}

fn read_json_response(response: RawResponse) -> Result<Value, InferenceError> {
    let RawResponse { status, mut body } = response;
    if !(200..300).contains(&status) {
        let text = read_body(&mut body, MAX_ERROR_BODY_BYTES, false)?;
        return Err(InferenceError::HttpStatus {
            status,
            body: String::from_utf8_lossy(&text).trim().to_owned(),
        });
    }
    let text = read_body(&mut body, MAX_BODY_BYTES, true)?;
    let value: Value = serde_json::from_slice(&text).map_err(|error| {
        InferenceError::InvalidResponse(format!("response is not valid JSON: {error}"))
    })?;
    if let Some(message) = value.get("error").and_then(Value::as_str) {
        return Err(InferenceError::Provider(message.to_owned()));
    }
    Ok(value)
}

fn read_body<R: Read>(
    reader: &mut R,
    limit: usize,
    strict: bool,
) -> Result<Vec<u8>, InferenceError> {
    let mut out = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let read = reader.read(&mut buffer).map_err(map_body_io_error)?;
        if read == 0 {
            break;
        }
        let remaining = limit.saturating_sub(out.len());
        if remaining == 0 {
            if strict {
                return Err(InferenceError::InvalidResponse(format!(
                    "response exceeded {limit} bytes"
                )));
            }
            break;
        }
        let take = read.min(remaining);
        out.extend_from_slice(&buffer[..take]);
        if take < read && strict {
            return Err(InferenceError::InvalidResponse(format!(
                "response exceeded {limit} bytes"
            )));
        }
    }
    Ok(out)
}

fn read_response_head<R: BufRead>(
    reader: &mut R,
) -> Result<(u16, HashMap<String, String>), InferenceError> {
    let mut line = Vec::new();
    read_header_line(reader, &mut line)?;
    if line.is_empty() {
        return Err(InferenceError::InvalidResponse(
            "Ollama returned an empty response".to_owned(),
        ));
    }
    let status_line = std::str::from_utf8(&line)
        .map_err(|_| InferenceError::InvalidResponse("HTTP status line is not UTF-8".to_owned()))?;
    let mut parts = status_line.split_whitespace();
    let version = parts.next().unwrap_or("");
    if !version.starts_with("HTTP/") {
        return Err(InferenceError::InvalidResponse(
            "response is not HTTP/1.1".to_owned(),
        ));
    }
    let status: u16 = parts
        .next()
        .ok_or_else(|| {
            InferenceError::InvalidResponse("HTTP status line has no status code".to_owned())
        })?
        .parse()
        .map_err(|_| {
            InferenceError::InvalidResponse("HTTP status code is not numeric".to_owned())
        })?;
    let mut headers: HashMap<String, String> = HashMap::new();
    let mut total = line.len();
    loop {
        line.clear();
        let read = read_header_line(reader, &mut line)?;
        if read == 0 {
            break;
        }
        total = total.saturating_add(read);
        if total > MAX_HEADER_BYTES {
            return Err(InferenceError::InvalidResponse(format!(
                "HTTP headers exceeded {MAX_HEADER_BYTES} bytes"
            )));
        }
        let header = std::str::from_utf8(&line)
            .map_err(|_| InferenceError::InvalidResponse("HTTP header is not UTF-8".to_owned()))?;
        let header = header.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim().to_owned();
            match headers.get_mut(&name) {
                Some(existing) => {
                    if !existing.is_empty() {
                        existing.push(',');
                    }
                    existing.push_str(&value);
                }
                None => {
                    headers.insert(name, value);
                }
            }
        }
    }
    Ok((status, headers))
}

fn read_header_line<R: BufRead>(
    reader: &mut R,
    out: &mut Vec<u8>,
) -> Result<usize, InferenceError> {
    read_line_io(reader, out, MAX_HEADER_LINE_BYTES).map_err(|error| {
        if error.kind() == io::ErrorKind::InvalidData {
            InferenceError::InvalidResponse(error.to_string())
        } else {
            map_io_error("failed to read Ollama response headers", error)
        }
    })
}

fn read_stream_line<R: BufRead>(
    reader: &mut R,
    out: &mut Vec<u8>,
) -> Result<usize, InferenceError> {
    read_line_io(reader, out, MAX_STREAM_LINE_BYTES).map_err(|error| {
        if matches!(
            error.kind(),
            io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof
        ) {
            InferenceError::InvalidResponse(error.to_string())
        } else {
            map_io_error("failed to read Ollama stream", error)
        }
    })
}

fn read_line_io<R: BufRead>(reader: &mut R, out: &mut Vec<u8>, limit: usize) -> io::Result<usize> {
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(out.len());
        }
        if let Some(position) = available.iter().position(|byte| *byte == b'\n') {
            let take = position + 1;
            if out.len().saturating_add(take) > limit {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("HTTP line exceeded {limit} bytes"),
                ));
            }
            out.extend_from_slice(&available[..take]);
            reader.consume(take);
            return Ok(out.len());
        }
        let take = available.len();
        if out.len().saturating_add(take) > limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("HTTP line exceeded {limit} bytes"),
            ));
        }
        out.extend_from_slice(available);
        reader.consume(take);
    }
}

fn trim_ascii(bytes: &[u8]) -> &[u8] {
    let mut start = 0;
    let mut end = bytes.len();
    while start < end && bytes[start].is_ascii_whitespace() {
        start += 1;
    }
    while end > start && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    &bytes[start..end]
}

fn map_io_error(context: &str, error: io::Error) -> InferenceError {
    match error.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => {
            InferenceError::Timeout(format!("{context}: {error}"))
        }
        _ => InferenceError::Network(format!("{context}: {error}")),
    }
}

fn map_body_io_error(error: io::Error) -> InferenceError {
    match error.kind() {
        io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof => {
            InferenceError::InvalidResponse(format!("malformed HTTP response body: {error}"))
        }
        _ => map_io_error("failed to read Ollama response", error),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BodyKind {
    Fixed,
    Chunked,
    UntilEof,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChunkState {
    NeedSize,
    Data,
    NeedCrLf,
    Done,
}

/// Decodes a bounded HTTP response body from a buffered socket.
struct BodyReader<R: BufRead> {
    inner: R,
    kind: BodyKind,
    remaining: u64,
    chunk_state: ChunkState,
    finished: bool,
}

impl<R: BufRead> BodyReader<R> {
    fn new(inner: R, headers: &HashMap<String, String>) -> Result<Self, InferenceError> {
        let chunked = headers.get("transfer-encoding").is_some_and(|value| {
            value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("chunked"))
        });
        if chunked {
            return Ok(Self {
                inner,
                kind: BodyKind::Chunked,
                remaining: 0,
                chunk_state: ChunkState::NeedSize,
                finished: false,
            });
        }
        if let Some(raw_length) = headers.get("content-length") {
            let length: u64 = raw_length.trim().parse().map_err(|_| {
                InferenceError::InvalidResponse(format!(
                    "invalid Content-Length header: {raw_length}"
                ))
            })?;
            return Ok(Self {
                inner,
                kind: BodyKind::Fixed,
                remaining: length,
                chunk_state: ChunkState::Done,
                finished: length == 0,
            });
        }
        Ok(Self {
            inner,
            kind: BodyKind::UntilEof,
            remaining: 0,
            chunk_state: ChunkState::Done,
            finished: false,
        })
    }

    fn read_chunked(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.finished {
                return Ok(0);
            }
            match self.chunk_state {
                ChunkState::Done => {
                    self.finished = true;
                    return Ok(0);
                }
                ChunkState::NeedCrLf => {
                    consume_crlf(&mut self.inner)?;
                    self.chunk_state = ChunkState::NeedSize;
                }
                ChunkState::NeedSize => {
                    let mut line = Vec::new();
                    let read = read_line_io(&mut self.inner, &mut line, MAX_HEADER_LINE_BYTES)?;
                    if read == 0 {
                        self.finished = true;
                        return Ok(0);
                    }
                    let text = std::str::from_utf8(&line).map_err(|_| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            "chunk size line is not valid UTF-8",
                        )
                    })?;
                    let size_text = text
                        .trim()
                        .split(';')
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_owned();
                    let size = u64::from_str_radix(&size_text, 16).map_err(|_| {
                        io::Error::new(io::ErrorKind::InvalidData, "invalid chunk size")
                    })?;
                    if size == 0 {
                        consume_trailers(&mut self.inner)?;
                        self.finished = true;
                        self.chunk_state = ChunkState::Done;
                        return Ok(0);
                    }
                    self.remaining = size;
                    self.chunk_state = ChunkState::Data;
                }
                ChunkState::Data => {
                    if self.remaining == 0 {
                        self.chunk_state = ChunkState::NeedCrLf;
                        continue;
                    }
                    let want = usize::try_from(self.remaining.min(buffer.len() as u64))
                        .unwrap_or(buffer.len());
                    let read = self.inner.read(&mut buffer[..want])?;
                    if read == 0 {
                        return Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "truncated chunked body",
                        ));
                    }
                    self.remaining -= read as u64;
                    if self.remaining == 0 {
                        self.chunk_state = ChunkState::NeedCrLf;
                    }
                    return Ok(read);
                }
            }
        }
    }
}

impl<R: BufRead> Read for BodyReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() || self.finished {
            return Ok(0);
        }
        match self.kind {
            BodyKind::Fixed => {
                if self.remaining == 0 {
                    self.finished = true;
                    return Ok(0);
                }
                let want = usize::try_from(self.remaining.min(buffer.len() as u64))
                    .unwrap_or(buffer.len());
                let read = self.inner.read(&mut buffer[..want])?;
                if read == 0 {
                    self.finished = true;
                    return Ok(0);
                }
                self.remaining -= read as u64;
                Ok(read)
            }
            BodyKind::UntilEof => {
                let read = self.inner.read(buffer)?;
                if read == 0 {
                    self.finished = true;
                }
                Ok(read)
            }
            BodyKind::Chunked => self.read_chunked(buffer),
        }
    }
}

fn consume_crlf<R: BufRead>(reader: &mut R) -> io::Result<()> {
    let mut byte = [0u8; 1];
    reader.read_exact(&mut byte)?;
    if byte[0] == b'\n' {
        return Ok(());
    }
    if byte[0] != b'\r' {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "chunk data is not followed by CRLF",
        ));
    }
    reader.read_exact(&mut byte)?;
    if byte[0] != b'\n' {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "chunk data is not followed by CRLF",
        ));
    }
    Ok(())
}

fn consume_trailers<R: BufRead>(reader: &mut R) -> io::Result<()> {
    loop {
        let mut line = Vec::new();
        let read = read_line_io(reader, &mut line, MAX_HEADER_LINE_BYTES)?;
        if read == 0 {
            return Ok(());
        }
        if line == b"\r\n" || line == b"\n" {
            return Ok(());
        }
    }
}

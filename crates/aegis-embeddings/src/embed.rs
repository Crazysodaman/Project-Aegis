//! Embedding providers and vector helpers.

use std::fmt::Debug;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::EmbeddingError;

/// A source of embedding vectors for text.
pub trait EmbeddingProvider: Send + Sync + Debug {
    /// Stable provider name, for example `local-hash` or `ollama`.
    fn name(&self) -> &str;

    /// Model identifier used with this provider.
    fn model(&self) -> &str;

    /// Whether this provider produces learned semantic embeddings.
    fn semantic(&self) -> bool {
        false
    }

    /// Embed each input string in order.
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError>;
}

/// SHA-256 of a memory's content, used to invalidate cached embeddings.
pub fn content_digest(content: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(content.as_bytes());
    let mut output = String::with_capacity(64);
    for byte in digest {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

/// Cosine similarity; dimension mismatch or zero vectors yield `0.0`.
pub fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.len() != right.len() || left.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f64;
    let mut left_norm = 0.0f64;
    let mut right_norm = 0.0f64;
    for (a, b) in left.iter().zip(right.iter()) {
        if !a.is_finite() || !b.is_finite() {
            return 0.0;
        }
        dot += f64::from(*a) * f64::from(*b);
        left_norm += f64::from(*a) * f64::from(*a);
        right_norm += f64::from(*b) * f64::from(*b);
    }
    if left_norm <= 0.0 || right_norm <= 0.0 {
        return 0.0;
    }
    (dot / (left_norm.sqrt() * right_norm.sqrt())) as f32
}

/// Deterministic lexical feature hashing.
///
/// This is a real vector representation, but it is **not** a learned semantic
/// model. It provides partial-match and term-overlap similarity without an
/// external provider.
#[derive(Debug, Clone)]
pub struct HashingEmbeddingProvider {
    dimensions: usize,
}

impl HashingEmbeddingProvider {
    pub const DEFAULT_DIMENSIONS: usize = 256;

    pub fn new(dimensions: usize) -> Self {
        Self {
            dimensions: dimensions.clamp(32, 4096),
        }
    }
}

impl Default for HashingEmbeddingProvider {
    fn default() -> Self {
        Self::new(Self::DEFAULT_DIMENSIONS)
    }
}

impl EmbeddingProvider for HashingEmbeddingProvider {
    fn name(&self) -> &str {
        "local-hash"
    }

    fn model(&self) -> &str {
        "feature-hash-v1"
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        Ok(texts.iter().map(|text| self.embed_one(text)).collect())
    }
}

impl HashingEmbeddingProvider {
    fn embed_one(&self, text: &str) -> Vec<f32> {
        let mut vector = vec![0.0f32; self.dimensions];
        let words: Vec<String> = text
            .split(|character: char| !character.is_alphanumeric())
            .filter(|word| word.chars().count() >= 2)
            .map(|word| word.to_lowercase())
            .collect();
        for word in &words {
            self.add_term(&mut vector, word, 1.0);
        }
        for pair in words.windows(2) {
            let bigram = format!("{} {}", pair[0], pair[1]);
            self.add_term(&mut vector, &bigram, 0.5);
        }
        let norm = vector
            .iter()
            .map(|value| f64::from(*value) * f64::from(*value))
            .sum::<f64>()
            .sqrt();
        if norm > 0.0 {
            for value in &mut vector {
                *value = (*value / norm as f32).clamp(-1.0, 1.0);
            }
        }
        vector
    }

    fn add_term(&self, vector: &mut [f32], term: &str, weight: f32) {
        let digest = Sha256::digest(term.as_bytes());
        let index = usize::from(u16::from_be_bytes([digest[0], digest[1]])) % self.dimensions;
        let sign = if digest[2] & 1 == 0 { 1.0 } else { -1.0 };
        vector[index] += weight * sign;
    }
}

/// Real HTTP adapter for Ollama's `/api/embeddings` endpoint.
///
/// Only `http://` is supported because this crate links no TLS implementation.
/// Live provider verification is required before claiming semantic retrieval is
/// active in an environment.
#[derive(Debug, Clone)]
pub struct OllamaEmbeddingProvider {
    host: String,
    port: u16,
    prefix: String,
    model: String,
    timeout: Duration,
}

impl OllamaEmbeddingProvider {
    pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;

    pub fn new(base_url: &str, model: &str) -> Result<Self, EmbeddingError> {
        let endpoint = parse_endpoint(base_url)?;
        let model = model.trim();
        if model.is_empty() || model.chars().count() > 128 || model.chars().any(char::is_control) {
            return Err(EmbeddingError::InvalidResponse(
                "invalid model name".to_owned(),
            ));
        }
        Ok(Self {
            host: endpoint.0,
            port: endpoint.1,
            prefix: endpoint.2,
            model: model.to_owned(),
            timeout: Duration::from_millis(Self::DEFAULT_TIMEOUT_MS),
        })
    }

    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout = Duration::from_millis(timeout_ms.max(1));
        self
    }
}

impl EmbeddingProvider for OllamaEmbeddingProvider {
    fn name(&self) -> &str {
        "ollama"
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn semantic(&self) -> bool {
        true
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        let mut vectors = Vec::with_capacity(texts.len());
        let mut dimensions = None;
        for text in texts {
            let vector = self.embed_one(text)?;
            if vector.is_empty() || vector.len() > 8192 {
                return Err(EmbeddingError::Dimensions);
            }
            if let Some(expected) = dimensions {
                if vector.len() != expected {
                    return Err(EmbeddingError::Dimensions);
                }
            } else {
                dimensions = Some(vector.len());
            }
            vectors.push(vector);
        }
        Ok(vectors)
    }
}

impl OllamaEmbeddingProvider {
    fn embed_one(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let body = serde_json::to_vec(&serde_json::json!({
            "model": self.model,
            "prompt": text,
        }))?;
        let path = format!("{}api/embeddings", self.prefix);
        let request = format!(
            "POST {path} HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            self.host,
            self.port,
            body.len()
        );
        let address = format!("{}:{}", self.host, self.port);
        let mut last_error = None;
        let mut stream = None;
        for socket in address
            .to_socket_addrs()
            .map_err(|error| EmbeddingError::Network(error.to_string()))?
        {
            match TcpStream::connect_timeout(&socket, self.timeout) {
                Ok(candidate) => {
                    stream = Some(candidate);
                    break;
                }
                Err(error) => last_error = Some(error),
            }
        }
        let mut stream = stream.ok_or_else(|| {
            EmbeddingError::Network(
                last_error
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "could not resolve endpoint".to_owned()),
            )
        })?;
        stream
            .set_read_timeout(Some(self.timeout))
            .map_err(|error| EmbeddingError::Network(error.to_string()))?;
        stream
            .set_write_timeout(Some(self.timeout))
            .map_err(|error| EmbeddingError::Network(error.to_string()))?;
        let mut request_bytes = request.into_bytes();
        request_bytes.extend_from_slice(&body);
        stream.write_all(&request_bytes).map_err(|error| {
            if matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) {
                EmbeddingError::Timeout(error.to_string())
            } else {
                EmbeddingError::Network(error.to_string())
            }
        })?;
        let mut bytes = Vec::new();
        stream
            .take(8 * 1024 * 1024)
            .read_to_end(&mut bytes)
            .map_err(|error| {
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) {
                    EmbeddingError::Timeout(error.to_string())
                } else {
                    EmbeddingError::Network(error.to_string())
                }
            })?;
        let split = bytes
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .ok_or_else(|| {
                EmbeddingError::InvalidResponse("response has no header terminator".to_owned())
            })?;
        let headers = String::from_utf8_lossy(&bytes[..split]);
        let status = headers
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|value| value.parse::<u16>().ok())
            .ok_or_else(|| EmbeddingError::InvalidResponse("missing status".to_owned()))?;
        let body = &bytes[split + 4..];
        if !(200..300).contains(&status) {
            return Err(EmbeddingError::HttpStatus {
                status,
                body: String::from_utf8_lossy(body).chars().take(512).collect(),
            });
        }
        let body = decode_http_body(&headers, body)?;
        let value: Value = serde_json::from_slice(&body)?;
        let vector = match value.get("embedding").and_then(Value::as_array) {
            Some(values) => values.clone(),
            None => value
                .get("embeddings")
                .and_then(Value::as_array)
                .and_then(|all| all.first())
                .and_then(Value::as_array)
                .cloned()
                .ok_or_else(|| {
                    EmbeddingError::InvalidResponse("missing embedding array".to_owned())
                })?,
        };
        let mut output = Vec::with_capacity(vector.len());
        for component in vector {
            let number = component.as_f64().ok_or_else(|| {
                EmbeddingError::InvalidResponse("embedding component is not a number".to_owned())
            })?;
            if !number.is_finite() {
                return Err(EmbeddingError::InvalidResponse(
                    "embedding component is not finite".to_owned(),
                ));
            }
            output.push(number as f32);
        }
        Ok(output)
    }
}

fn decode_http_body(headers: &str, body: &[u8]) -> Result<Vec<u8>, EmbeddingError> {
    let mut chunked = false;
    let mut content_length = None;
    for line in headers.lines().skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        match name.trim().to_ascii_lowercase().as_str() {
            "transfer-encoding" if value.to_ascii_lowercase().contains("chunked") => chunked = true,
            "content-length" => {
                content_length = value.trim().parse::<usize>().ok();
            }
            _ => {}
        }
    }
    let body = if chunked {
        decode_chunked(body)?
    } else if let Some(length) = content_length {
        if body.len() < length {
            return Err(EmbeddingError::InvalidResponse(
                "response body is shorter than Content-Length".to_owned(),
            ));
        }
        body[..length].to_vec()
    } else {
        body.to_vec()
    };
    if body.len() > 8 * 1024 * 1024 {
        return Err(EmbeddingError::InvalidResponse(
            "response body is too large".to_owned(),
        ));
    }
    Ok(body)
}

fn decode_chunked(body: &[u8]) -> Result<Vec<u8>, EmbeddingError> {
    let mut position = 0usize;
    let mut output = Vec::new();
    loop {
        let line_end = body[position..]
            .windows(2)
            .position(|window| window == b"\r\n")
            .map(|offset| position + offset)
            .ok_or_else(|| {
                EmbeddingError::InvalidResponse("chunk header is unterminated".to_owned())
            })?;
        let line = String::from_utf8_lossy(&body[position..line_end]);
        let size_text = line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|_| EmbeddingError::InvalidResponse("chunk size is invalid".to_owned()))?;
        position = line_end + 2;
        if size == 0 {
            break;
        }
        if body.len() < position + size + 2 {
            return Err(EmbeddingError::InvalidResponse(
                "chunk is truncated".to_owned(),
            ));
        }
        output.extend_from_slice(&body[position..position + size]);
        position += size;
        if &body[position..position + 2] != b"\r\n" {
            return Err(EmbeddingError::InvalidResponse(
                "chunk is not CRLF-terminated".to_owned(),
            ));
        }
        position += 2;
        if output.len() > 8 * 1024 * 1024 {
            return Err(EmbeddingError::InvalidResponse(
                "decoded body is too large".to_owned(),
            ));
        }
    }
    Ok(output)
}
fn parse_endpoint(base_url: &str) -> Result<(String, u16, String), EmbeddingError> {
    let trimmed = base_url.trim();
    if let Some(rest) = trimmed.strip_prefix("https://") {
        return Err(EmbeddingError::UnsupportedScheme(format!("https://{rest}")));
    }
    let rest = trimmed
        .strip_prefix("http://")
        .ok_or_else(|| EmbeddingError::InvalidUrl(base_url.to_owned()))?;
    let (authority, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, ""),
    };
    if authority.is_empty() {
        return Err(EmbeddingError::InvalidUrl(base_url.to_owned()));
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() => {
            let port = port
                .parse::<u16>()
                .map_err(|_| EmbeddingError::InvalidUrl(base_url.to_owned()))?;
            (host.to_owned(), port)
        }
        _ => (authority.to_owned(), 80),
    };
    if host.is_empty() {
        return Err(EmbeddingError::InvalidUrl(base_url.to_owned()));
    }
    let prefix = if path.is_empty() {
        "/".to_owned()
    } else if path.ends_with('/') {
        path.to_owned()
    } else {
        format!("{path}/")
    };
    Ok((host, port, prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunked_bodies_are_decoded() {
        let body = b"4\r\n{\"em\r\n3\r\nbed\r\n0\r\n\r\n";
        let decoded = decode_http_body("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n", body)
            .expect("decode");
        assert_eq!(decoded, b"{\"embed");
    }

    #[test]
    fn live_embedding_when_enabled() {
        if std::env::var("AEGIS_EMBED_LIVE").ok().as_deref() != Some("1") {
            eprintln!("skipping live embedding test: set AEGIS_EMBED_LIVE=1");
            return;
        }
        let url = std::env::var("AEGIS_OLLAMA_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:11434".to_owned());
        let model =
            std::env::var("AEGIS_EMBED_MODEL").unwrap_or_else(|_| "nomic-embed-text".to_owned());
        let provider = OllamaEmbeddingProvider::new(&url, &model).expect("provider");
        let vectors = provider
            .embed(&[
                "The owner prefers oolong tea.".to_owned(),
                "Tea is the owner's preferred drink.".to_owned(),
                "The weather is cold.".to_owned(),
            ])
            .expect("live embeddings");
        assert_eq!(vectors.len(), 3);
        assert!(!vectors[0].is_empty());
        assert_eq!(vectors[0].len(), vectors[2].len());
        let related = cosine_similarity(&vectors[0], &vectors[1]);
        let unrelated = cosine_similarity(&vectors[0], &vectors[2]);
        eprintln!(
            "live embedding dims={} related={related} unrelated={unrelated}",
            vectors[0].len()
        );
        assert!(related > unrelated);
    }
}

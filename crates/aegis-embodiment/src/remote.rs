//! Deliberately simple, authenticated remote artifact fetch protocol.
//!
//! This module is a protocol-level building block for pulling one embodiment
//! artifact from a Core that already owns it. It is intentionally small and
//! conservative:
//!
//! - Transport is plain [`std::net::TcpStream`] (no TLS). The shared secret must
//!   be provisioned out of band. This is suitable for a private LAN or for
//!   in-process tests, not for the public internet.
//! - Every message is a length-prefixed frame. Frame sizes are bounded on both
//!   sides and every stream gets read/write timeouts.
//! - The client proves possession of the shared secret with an HMAC-SHA256 over
//!   a domain-separated, nonce-bound request. The server verifies it in constant
//!   time, checks that the request is for the Core it serves, and that the
//!   artifact is owned by that Core.
//! - The server signs its reply. The client verifies the reply signature, checks
//!   the nonce and Core/asset binding, then verifies the SHA-256 of the received
//!   bytes before returning them. A partial or truncated transfer is rejected
//!   and never written to disk.
//!
//! Nothing here performs replication, tombstone propagation, conflict
//! resolution, or multi-host redundancy. A fetch is a read-only, idempotent
//! operation: repeating the same request for the same artifact returns the same
//! digest and bytes and never mutates the serving store.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::asset::MAX_ASSET_BYTES;
use crate::error::EmbodimentError;
use crate::store::EmbodimentStore;

/// Schema identifier for a remote artifact request.
pub const REMOTE_REQUEST_SCHEMA: &str = "aegis.embodiment.remote_request.v1";
/// Schema identifier for a remote artifact response.
pub const REMOTE_RESPONSE_SCHEMA: &str = "aegis.embodiment.remote_response.v1";
/// Domain separation label mixed into every proof.
pub const REMOTE_PROOF_DOMAIN: &str = "aegis.embodiment.artifact.v1";
/// Minimum shared-secret length in bytes.
pub const MIN_SECRET_BYTES: usize = 16;
/// Maximum size of a request or response header frame.
pub const MAX_FRAME_BYTES: usize = 64 * 1024;
/// Maximum size of an artifact data frame (aligned with [`MAX_ASSET_BYTES`]).
pub const MAX_ARTIFACT_FRAME_BYTES: usize = MAX_ASSET_BYTES as usize;
/// Default connect/read/write timeout.
pub const DEFAULT_TIMEOUT_MS: u64 = 5_000;
/// Upper bound for a caller-configured timeout.
pub const MAX_TIMEOUT_MS: u64 = 60_000;
/// Maximum accepted nonce length.
pub const MAX_NONCE_CHARS: usize = 128;

const HMAC_BLOCK: usize = 64;

/// Errors produced by the remote artifact protocol.
#[derive(Debug, Error)]
pub enum RemoteError {
    #[error("remote artifact I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("remote artifact JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("remote artifact store failed: {0}")]
    Store(String),
    #[error("remote frame exceeds the {0}-byte limit")]
    FrameTooLarge(usize),
    #[error("remote frame is empty")]
    EmptyFrame,
    #[error("remote transfer was truncated or cut short")]
    Truncated,
    #[error("remote operation timed out")]
    Timeout,
    #[error("remote artifact protocol violation: {0}")]
    Protocol(&'static str),
    #[error("remote artifact request was denied")]
    Unauthorized,
    #[error("requested Core is not served by this endpoint")]
    WrongCore,
    #[error("remote artifact was not found for the requesting Core")]
    NotFound,
    #[error("remote artifact digest verification failed")]
    IntegrityMismatch,
    #[error("shared secret must be at least {MIN_SECRET_BYTES} bytes")]
    SecretTooShort,
    #[error("nonce is invalid")]
    InvalidNonce,
    #[error("invalid remote artifact input: {0}")]
    InvalidInput(&'static str),
}

/// Outcome reported by a serving endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseStatus {
    Ok,
    Denied,
    NotFound,
    WrongCore,
    BadRequest,
    Corrupt,
}

impl ResponseStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Denied => "denied",
            Self::NotFound => "not_found",
            Self::WrongCore => "wrong_core",
            Self::BadRequest => "bad_request",
            Self::Corrupt => "corrupt",
        }
    }
}

/// One authenticated artifact request frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRequest {
    pub schema: String,
    pub core_uuid: String,
    pub asset_id: String,
    pub nonce: String,
    /// Lowercase hex HMAC-SHA256 proof.
    pub proof: String,
}

impl ArtifactRequest {
    /// Build an unsigned request. Call [`ArtifactRequest::sign`] before sending.
    pub fn new(
        core_uuid: impl Into<String>,
        asset_id: impl Into<String>,
        nonce: impl Into<String>,
    ) -> Self {
        Self {
            schema: REMOTE_REQUEST_SCHEMA.to_owned(),
            core_uuid: core_uuid.into(),
            asset_id: asset_id.into(),
            nonce: nonce.into(),
            proof: String::new(),
        }
    }

    /// Sign the request with the shared secret.
    pub fn sign(&mut self, secret: &[u8]) {
        let message = request_message(&self.core_uuid, &self.asset_id, &self.nonce);
        self.proof = to_hex(&hmac_sha256(secret, message.as_bytes()));
    }

    /// Verify the request proof in constant time.
    pub fn verify(&self, secret: &[u8]) -> bool {
        let message = request_message(&self.core_uuid, &self.asset_id, &self.nonce);
        let expected = hmac_sha256(secret, message.as_bytes());
        match from_hex(&self.proof) {
            Some(presented) => constant_time_eq(&expected, &presented),
            None => false,
        }
    }
}

/// One authenticated artifact response header frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactResponse {
    pub schema: String,
    pub status: ResponseStatus,
    pub core_uuid: String,
    pub asset_id: String,
    pub nonce: String,
    pub media_type: String,
    /// Lowercase hex SHA-256 of the artifact bytes (empty unless `status == Ok`).
    pub sha256: String,
    /// Artifact byte length (zero unless `status == Ok`).
    pub bytes: u64,
    /// Lowercase hex HMAC-SHA256 proof over the response fields.
    pub proof: String,
}

impl ArtifactResponse {
    /// Sign the response with the shared secret.
    pub fn sign(&mut self, secret: &[u8]) {
        let message = response_message(
            self.status,
            &self.core_uuid,
            &self.asset_id,
            &self.nonce,
            &self.sha256,
            self.bytes,
        );
        self.proof = to_hex(&hmac_sha256(secret, message.as_bytes()));
    }

    /// Verify the response proof in constant time.
    pub fn verify(&self, secret: &[u8]) -> bool {
        let message = response_message(
            self.status,
            &self.core_uuid,
            &self.asset_id,
            &self.nonce,
            &self.sha256,
            self.bytes,
        );
        let expected = hmac_sha256(secret, message.as_bytes());
        match from_hex(&self.proof) {
            Some(presented) => constant_time_eq(&expected, &presented),
            None => false,
        }
    }
}

/// A fully verified artifact returned by [`RemoteArtifactClient`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedArtifact {
    pub core_uuid: String,
    pub asset_id: String,
    pub media_type: String,
    pub sha256: String,
    pub bytes: Vec<u8>,
}

impl FetchedArtifact {
    /// True when the received digest matches a digest the caller already knows
    /// (for example, from a recovery manifest).
    pub fn matches_expected_sha256(&self, expected: &str) -> bool {
        self.sha256.eq_ignore_ascii_case(expected.trim())
    }

    /// Write the verified bytes to `path` atomically.
    ///
    /// The destination is only created after the temporary file has been
    /// re-hashed and matched. A failure never leaves a partial file at `path`.
    pub fn save_to(&self, path: impl AsRef<Path>) -> Result<(), RemoteError> {
        let path = path.as_ref();
        if sha256_hex(&self.bytes) != self.sha256 {
            return Err(RemoteError::IntegrityMismatch);
        }
        let parent = path
            .parent()
            .ok_or(RemoteError::InvalidInput("destination path"))?;
        std::fs::create_dir_all(parent)?;
        let temporary = path.with_file_name(format!(
            ".{}.{}.tmp",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::write(&temporary, &self.bytes)?;
        let written = std::fs::read(&temporary)?;
        if sha256_hex(&written) != self.sha256 {
            let _ = std::fs::remove_file(&temporary);
            return Err(RemoteError::IntegrityMismatch);
        }
        match std::fs::rename(&temporary, path) {
            Ok(()) => Ok(()),
            Err(error) => {
                let _ = std::fs::remove_file(&temporary);
                Err(RemoteError::Io(error))
            }
        }
    }
}

/// Client that fetches one artifact from a serving endpoint.
#[derive(Debug, Clone)]
pub struct RemoteArtifactClient {
    secret: Vec<u8>,
    timeout: Duration,
    max_artifact_bytes: u64,
}

impl RemoteArtifactClient {
    pub fn new(secret: impl AsRef<[u8]>) -> Result<Self, RemoteError> {
        let secret = secret.as_ref().to_vec();
        if secret.len() < MIN_SECRET_BYTES {
            return Err(RemoteError::SecretTooShort);
        }
        Ok(Self {
            secret,
            timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS),
            max_artifact_bytes: MAX_ASSET_BYTES,
        })
    }

    /// Override the connect/read/write timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Result<Self, RemoteError> {
        let millis = timeout.as_millis();
        if millis == 0 || millis > u128::from(MAX_TIMEOUT_MS) {
            return Err(RemoteError::InvalidInput("timeout"));
        }
        self.timeout = timeout;
        Ok(self)
    }

    /// Override the maximum accepted artifact size.
    pub fn with_max_artifact_bytes(mut self, max: u64) -> Result<Self, RemoteError> {
        if max == 0 || max > MAX_ASSET_BYTES {
            return Err(RemoteError::InvalidInput("artifact size bound"));
        }
        self.max_artifact_bytes = max;
        Ok(self)
    }

    /// Fetch an artifact, producing a fresh nonce.
    pub fn fetch(
        &self,
        addr: impl ToSocketAddrs,
        core_uuid: &str,
        asset_id: &str,
    ) -> Result<FetchedArtifact, RemoteError> {
        let nonce = new_nonce();
        self.fetch_with_nonce(addr, core_uuid, asset_id, &nonce)
    }

    /// Fetch an artifact using a caller-supplied nonce.
    ///
    /// Repeated identical requests are idempotent: the artifact is read-only,
    /// so the same artifact always yields the same digest and bytes.
    pub fn fetch_with_nonce(
        &self,
        addr: impl ToSocketAddrs,
        core_uuid: &str,
        asset_id: &str,
        nonce: &str,
    ) -> Result<FetchedArtifact, RemoteError> {
        validate_nonce(nonce)?;
        if core_uuid.trim().is_empty() {
            return Err(RemoteError::InvalidInput("core uuid"));
        }
        let max = usize::try_from(self.max_artifact_bytes)
            .map_err(|_| RemoteError::FrameTooLarge(MAX_ARTIFACT_FRAME_BYTES))?;
        let mut stream = connect(addr, self.timeout)?;
        stream.set_read_timeout(Some(self.timeout))?;
        stream.set_write_timeout(Some(self.timeout))?;

        let mut request = ArtifactRequest::new(core_uuid, asset_id, nonce);
        request.sign(&self.secret);
        let request_frame = serde_json::to_vec(&request)?;
        write_frame(&mut stream, &request_frame, MAX_FRAME_BYTES)?;

        let header_frame = read_frame(&mut stream, MAX_FRAME_BYTES)?;
        let response: ArtifactResponse = serde_json::from_slice(&header_frame)?;
        if response.schema != REMOTE_RESPONSE_SCHEMA {
            return Err(RemoteError::Protocol("response schema"));
        }
        if !response.verify(&self.secret) {
            return Err(RemoteError::Unauthorized);
        }
        if response.nonce != request.nonce {
            return Err(RemoteError::Protocol("response nonce"));
        }
        if response.core_uuid != request.core_uuid || response.asset_id != request.asset_id {
            return Err(RemoteError::Protocol("response binding"));
        }
        match response.status {
            ResponseStatus::Ok => {}
            ResponseStatus::Denied => return Err(RemoteError::Unauthorized),
            ResponseStatus::NotFound => return Err(RemoteError::NotFound),
            ResponseStatus::WrongCore => return Err(RemoteError::WrongCore),
            ResponseStatus::BadRequest => return Err(RemoteError::Protocol("request rejected")),
            ResponseStatus::Corrupt => return Err(RemoteError::IntegrityMismatch),
        }
        if response.sha256.len() != 64
            || response.bytes == 0
            || response.bytes > self.max_artifact_bytes
        {
            return Err(RemoteError::Protocol("response bounds"));
        }
        let data_frame = read_frame(&mut stream, max)?;
        if data_frame.len() as u64 != response.bytes {
            return Err(RemoteError::Protocol("artifact length"));
        }
        let digest = sha256_hex(&data_frame);
        if digest != response.sha256 {
            return Err(RemoteError::IntegrityMismatch);
        }
        Ok(FetchedArtifact {
            core_uuid: response.core_uuid,
            asset_id: response.asset_id,
            media_type: response.media_type,
            sha256: digest,
            bytes: data_frame,
        })
    }

    /// Fetch an artifact and atomically persist it only after verification.
    pub fn fetch_to_path(
        &self,
        addr: impl ToSocketAddrs,
        core_uuid: &str,
        asset_id: &str,
        path: impl AsRef<Path>,
    ) -> Result<FetchedArtifact, RemoteError> {
        let artifact = self.fetch(addr, core_uuid, asset_id)?;
        artifact.save_to(path)?;
        Ok(artifact)
    }
}

/// Server that answers one request at a time for the Core in its store.
#[derive(Debug, Clone)]
pub struct RemoteArtifactServer<'a> {
    store: &'a EmbodimentStore,
    secret: Vec<u8>,
    timeout: Duration,
}

impl<'a> RemoteArtifactServer<'a> {
    pub fn new(store: &'a EmbodimentStore, secret: impl AsRef<[u8]>) -> Result<Self, RemoteError> {
        let secret = secret.as_ref().to_vec();
        if secret.len() < MIN_SECRET_BYTES {
            return Err(RemoteError::SecretTooShort);
        }
        Ok(Self {
            store,
            secret,
            timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS),
        })
    }

    /// Override the read/write timeout for served connections.
    pub fn with_timeout(mut self, timeout: Duration) -> Result<Self, RemoteError> {
        let millis = timeout.as_millis();
        if millis == 0 || millis > u128::from(MAX_TIMEOUT_MS) {
            return Err(RemoteError::InvalidInput("timeout"));
        }
        self.timeout = timeout;
        Ok(self)
    }

    /// Read one request, reply, and close the connection.
    ///
    /// Denials and misses are reported to the client with a signed status
    /// frame. A malformed frame is answered only when it can still be parsed;
    /// otherwise this returns the framing error and the stream is dropped.
    pub fn serve_once(&self, mut stream: TcpStream) -> Result<(), RemoteError> {
        stream.set_read_timeout(Some(self.timeout))?;
        stream.set_write_timeout(Some(self.timeout))?;
        let request_frame = read_frame(&mut stream, MAX_FRAME_BYTES)?;
        let request: ArtifactRequest = serde_json::from_slice(&request_frame)?;
        let (mut response, data) = self.build_response(&request);
        response.sign(&self.secret);
        let header_frame = serde_json::to_vec(&response)?;
        write_frame(&mut stream, &header_frame, MAX_FRAME_BYTES)?;
        if let Some(data) = data {
            write_frame(&mut stream, &data, MAX_ARTIFACT_FRAME_BYTES)?;
        }
        Ok(())
    }

    fn build_response(&self, request: &ArtifactRequest) -> (ArtifactResponse, Option<Vec<u8>>) {
        let mut response = ArtifactResponse {
            schema: REMOTE_RESPONSE_SCHEMA.to_owned(),
            status: ResponseStatus::BadRequest,
            core_uuid: request.core_uuid.clone(),
            asset_id: request.asset_id.clone(),
            nonce: request.nonce.clone(),
            media_type: String::new(),
            sha256: String::new(),
            bytes: 0,
            proof: String::new(),
        };
        if request.schema != REMOTE_REQUEST_SCHEMA || validate_nonce(&request.nonce).is_err() {
            return (response, None);
        }
        if !request.verify(&self.secret) {
            response.status = ResponseStatus::Denied;
            return (response, None);
        }
        if request.core_uuid != self.store.core_uuid() {
            response.status = ResponseStatus::WrongCore;
            return (response, None);
        }
        let asset = match self.store.asset(&request.asset_id) {
            Ok(asset) => asset,
            Err(EmbodimentError::NotFound | EmbodimentError::Deleted) => {
                response.status = ResponseStatus::NotFound;
                return (response, None);
            }
            Err(EmbodimentError::InvalidInput(_) | EmbodimentError::InvalidCoreUuid) => {
                return (response, None);
            }
            Err(_) => {
                response.status = ResponseStatus::Corrupt;
                return (response, None);
            }
        };
        if asset.deleted {
            response.status = ResponseStatus::NotFound;
            return (response, None);
        }
        if asset.core_uuid != request.core_uuid {
            response.status = ResponseStatus::WrongCore;
            return (response, None);
        }
        let data = match self.store.read_asset(&request.asset_id) {
            Ok(data) => data,
            Err(_) => {
                response.status = ResponseStatus::Corrupt;
                return (response, None);
            }
        };
        if sha256_hex(&data) != asset.sha256 || data.len() as u64 != asset.bytes {
            response.status = ResponseStatus::Corrupt;
            return (response, None);
        }
        response.status = ResponseStatus::Ok;
        response.media_type = asset.media_type;
        response.sha256 = asset.sha256;
        response.bytes = data.len() as u64;
        (response, Some(data))
    }
}

/// Generate a fresh random nonce.
pub fn new_nonce() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn validate_nonce(nonce: &str) -> Result<(), RemoteError> {
    if nonce.is_empty()
        || nonce.chars().count() > MAX_NONCE_CHARS
        || nonce.chars().any(char::is_control)
    {
        return Err(RemoteError::InvalidNonce);
    }
    Ok(())
}

fn request_message(core_uuid: &str, asset_id: &str, nonce: &str) -> String {
    format!("{REMOTE_PROOF_DOMAIN}\nrequest\n{core_uuid}\n{asset_id}\n{nonce}")
}

fn response_message(
    status: ResponseStatus,
    core_uuid: &str,
    asset_id: &str,
    nonce: &str,
    sha256: &str,
    bytes: u64,
) -> String {
    format!(
        "{REMOTE_PROOF_DOMAIN}\nresponse\n{}\n{core_uuid}\n{asset_id}\n{nonce}\n{sha256}\n{bytes}",
        status.as_str()
    )
}

fn connect(addr: impl ToSocketAddrs, timeout: Duration) -> Result<TcpStream, RemoteError> {
    let mut last_error = None;
    for socket in addr.to_socket_addrs()? {
        match TcpStream::connect_timeout(&socket, timeout) {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = Some(error),
        }
    }
    Err(RemoteError::Io(last_error.unwrap_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::AddrNotAvailable, "no address")
    })))
}

/// Write one length-prefixed frame, enforcing `max` bytes.
pub fn write_frame<W: Write>(
    writer: &mut W,
    payload: &[u8],
    max: usize,
) -> Result<(), RemoteError> {
    if payload.is_empty() {
        return Err(RemoteError::EmptyFrame);
    }
    if payload.len() > max {
        return Err(RemoteError::FrameTooLarge(max));
    }
    let length = u32::try_from(payload.len()).map_err(|_| RemoteError::FrameTooLarge(max))?;
    writer.write_all(&length.to_be_bytes())?;
    writer.write_all(payload)?;
    writer.flush()?;
    Ok(())
}

/// Read one length-prefixed frame, rejecting frames larger than `max`.
pub fn read_frame<R: Read>(reader: &mut R, max: usize) -> Result<Vec<u8>, RemoteError> {
    let mut prefix = [0u8; 4];
    read_exact(reader, &mut prefix)?;
    let length = u32::from_be_bytes(prefix) as usize;
    if length == 0 {
        return Err(RemoteError::EmptyFrame);
    }
    if length > max {
        return Err(RemoteError::FrameTooLarge(max));
    }
    let mut payload = vec![0u8; length];
    read_exact(reader, &mut payload)?;
    Ok(payload)
}

fn read_exact<R: Read>(reader: &mut R, buffer: &mut [u8]) -> Result<(), RemoteError> {
    match reader.read_exact(buffer) {
        Ok(()) => Ok(()),
        Err(error) => match error.kind() {
            std::io::ErrorKind::UnexpectedEof => Err(RemoteError::Truncated),
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
                Err(RemoteError::Timeout)
            }
            _ => Err(RemoteError::Io(error)),
        },
    }
}

/// HMAC-SHA256 with the standard 64-byte block size.
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut key_block = [0u8; HMAC_BLOCK];
    if key.len() > HMAC_BLOCK {
        let digest = Sha256::digest(key);
        key_block[..digest.len()].copy_from_slice(&digest);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }
    let mut inner_pad = [0x36u8; HMAC_BLOCK];
    let mut outer_pad = [0x5cu8; HMAC_BLOCK];
    for index in 0..HMAC_BLOCK {
        inner_pad[index] ^= key_block[index];
        outer_pad[index] ^= key_block[index];
    }
    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(message);
    let inner_digest = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner_digest);
    let result = outer.finalize();
    let mut output = [0u8; 32];
    output.copy_from_slice(&result);
    output
}

/// Lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    to_hex(&Sha256::digest(bytes))
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0u8;
    for (a, b) in left.iter().zip(right.iter()) {
        difference |= a ^ b;
    }
    difference == 0
}

fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn from_hex(value: &str) -> Option<Vec<u8>> {
    if value.len() % 2 != 0 {
        return None;
    }
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len() / 2);
    let mut index = 0;
    while index < bytes.len() {
        let high = hex_digit(bytes[index])?;
        let low = hex_digit(bytes[index + 1])?;
        output.push((high << 4) | low);
        index += 2;
    }
    Some(output)
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"correct horse battery staple 0123456789";

    #[test]
    fn hmac_matches_reference_vector() {
        // RFC 4231 test case 2.
        let key = b"Jefe";
        let digest = hmac_sha256(key, b"what do ya want for nothing?");
        assert_eq!(
            to_hex(&digest),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn hex_round_trips_and_rejects_bad_input() {
        let bytes = [0u8, 1, 15, 16, 127, 128, 255];
        let hex = to_hex(&bytes);
        assert_eq!(from_hex(&hex).as_deref(), Some(bytes.as_slice()));
        assert!(from_hex("abc").is_none());
        assert!(from_hex("zz").is_none());
    }

    #[test]
    fn request_and_response_proofs_are_bound_to_fields() {
        let mut request = ArtifactRequest::new("core", "asset", "nonce");
        request.sign(SECRET);
        assert!(request.verify(SECRET));
        request.asset_id = "other".to_owned();
        assert!(!request.verify(SECRET));
        assert!(!request.verify(b"a-different-but-still-long-secret"));

        let mut response = ArtifactResponse {
            schema: REMOTE_RESPONSE_SCHEMA.to_owned(),
            status: ResponseStatus::Ok,
            core_uuid: "core".to_owned(),
            asset_id: "asset".to_owned(),
            nonce: "nonce".to_owned(),
            media_type: "application/octet-stream".to_owned(),
            sha256: sha256_hex(b"payload"),
            bytes: 7,
            proof: String::new(),
        };
        response.sign(SECRET);
        assert!(response.verify(SECRET));
        response.bytes = 8;
        assert!(!response.verify(SECRET));
    }

    #[test]
    fn short_secret_and_bad_nonce_are_rejected() {
        assert!(matches!(
            RemoteArtifactClient::new(b"short"),
            Err(RemoteError::SecretTooShort)
        ));
        assert!(matches!(validate_nonce(""), Err(RemoteError::InvalidNonce)));
        assert!(matches!(
            validate_nonce("bad\nnonce"),
            Err(RemoteError::InvalidNonce)
        ));
    }
}

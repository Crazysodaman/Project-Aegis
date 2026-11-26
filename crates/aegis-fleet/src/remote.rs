//! Authenticated remote host identity.
//!
//! Enrollment records *that* an owner trusts a host; this module proves *which*
//! host answered a probe. A verifier issues a fresh nonce challenge; the remote
//! host proves possession of a shared secret by returning an HMAC-SHA256 over a
//! canonical, domain-separated message. Responses are bound to the nonce, host
//! id, and callsign, so a captured response cannot be replayed against a new
//! challenge or relabelled for another host.
//!
//! This is a symmetric shared-secret scheme intended for a private homelab LAN.
//! It is not a replacement for TLS: the transport in this crate is plain
//! `http://`, and the shared secret must be provisioned out of band. Proofs are
//! compared in constant time.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Schema identifier for a remote identity challenge.
pub const REMOTE_CHALLENGE_SCHEMA: &str = "aegis.fleet.remote_challenge.v1";
/// Schema identifier for a remote identity response.
pub const REMOTE_RESPONSE_SCHEMA: &str = "aegis.fleet.remote_response.v1";
/// Domain separation label mixed into every proof.
pub const REMOTE_PROOF_DOMAIN: &str = "aegis.fleet.remote_identity.v1";
/// Minimum shared-secret length in bytes.
pub const MIN_SECRET_BYTES: usize = 16;
/// Maximum accepted nonce length.
const MAX_NONCE_LEN: usize = 128;

const HMAC_BLOCK: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteChallenge {
    pub schema: String,
    pub nonce: String,
    pub issued_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteResponse {
    pub schema: String,
    pub host_uuid: String,
    pub callsign: String,
    pub nonce: String,
    /// Lowercase hex HMAC-SHA256 proof.
    pub proof: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteIdentityError {
    InvalidInput(&'static str),
    SecretTooShort,
    NonceMismatch,
    HostMismatch,
    BadProof,
}

impl std::fmt::Display for RemoteIdentityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(field) => {
                write!(formatter, "invalid remote identity field: {field}")
            }
            Self::SecretTooShort => write!(
                formatter,
                "shared secret must be at least {MIN_SECRET_BYTES} bytes"
            ),
            Self::NonceMismatch => write!(formatter, "challenge nonce does not match"),
            Self::HostMismatch => {
                write!(formatter, "response host id does not match the challenge")
            }
            Self::BadProof => write!(formatter, "remote identity proof is invalid"),
        }
    }
}

impl std::error::Error for RemoteIdentityError {}

/// Local host identity capable of answering challenges.
#[derive(Debug, Clone)]
pub struct RemoteIdentity {
    host_uuid: String,
    callsign: String,
    secret: Vec<u8>,
}

impl RemoteIdentity {
    pub fn new(
        host_uuid: impl Into<String>,
        callsign: impl Into<String>,
        secret: impl AsRef<[u8]>,
    ) -> Result<Self, RemoteIdentityError> {
        let host_uuid = host_uuid.into();
        let callsign = callsign.into();
        let secret = secret.as_ref().to_vec();
        if host_uuid.trim().is_empty() || host_uuid.chars().any(char::is_control) {
            return Err(RemoteIdentityError::InvalidInput("host_uuid"));
        }
        if callsign.trim().is_empty() || callsign.chars().any(char::is_control) {
            return Err(RemoteIdentityError::InvalidInput("callsign"));
        }
        if secret.len() < MIN_SECRET_BYTES {
            return Err(RemoteIdentityError::SecretTooShort);
        }
        Ok(Self {
            host_uuid,
            callsign,
            secret,
        })
    }

    pub fn host_uuid(&self) -> &str {
        &self.host_uuid
    }

    pub fn callsign(&self) -> &str {
        &self.callsign
    }

    /// Produce a fresh challenge (a random nonce).
    pub fn challenge() -> RemoteChallenge {
        RemoteChallenge {
            schema: REMOTE_CHALLENGE_SCHEMA.to_owned(),
            nonce: Uuid::new_v4().to_string(),
            issued_at: aegis_storage::now_rfc3339().unwrap_or_default(),
        }
    }

    /// Answer a challenge with a proof bound to this identity.
    pub fn respond(
        &self,
        challenge: &RemoteChallenge,
    ) -> Result<RemoteResponse, RemoteIdentityError> {
        validate_nonce(&challenge.nonce)?;
        let message = proof_message(&challenge.nonce, &self.host_uuid, &self.callsign);
        let proof = hmac_sha256(&self.secret, message.as_bytes());
        Ok(RemoteResponse {
            schema: REMOTE_RESPONSE_SCHEMA.to_owned(),
            host_uuid: self.host_uuid.clone(),
            callsign: self.callsign.clone(),
            nonce: challenge.nonce.clone(),
            proof: to_hex(&proof),
        })
    }
}

/// Verify a remote response against the challenge, expected host, and secret.
pub fn verify_response(
    secret: &[u8],
    expected_host_uuid: &str,
    challenge: &RemoteChallenge,
    response: &RemoteResponse,
) -> Result<(), RemoteIdentityError> {
    if secret.len() < MIN_SECRET_BYTES {
        return Err(RemoteIdentityError::SecretTooShort);
    }
    validate_nonce(&challenge.nonce)?;
    if response.schema != REMOTE_RESPONSE_SCHEMA {
        return Err(RemoteIdentityError::InvalidInput("schema"));
    }
    if response.nonce != challenge.nonce {
        return Err(RemoteIdentityError::NonceMismatch);
    }
    if response.host_uuid != expected_host_uuid {
        return Err(RemoteIdentityError::HostMismatch);
    }
    if response.callsign.trim().is_empty() || response.callsign.chars().any(char::is_control) {
        return Err(RemoteIdentityError::InvalidInput("callsign"));
    }
    let message = proof_message(&response.nonce, &response.host_uuid, &response.callsign);
    let expected = hmac_sha256(secret, message.as_bytes());
    let presented = from_hex(&response.proof).ok_or(RemoteIdentityError::BadProof)?;
    if constant_time_eq(&expected, &presented) {
        Ok(())
    } else {
        Err(RemoteIdentityError::BadProof)
    }
}

fn validate_nonce(nonce: &str) -> Result<(), RemoteIdentityError> {
    if nonce.is_empty() || nonce.len() > MAX_NONCE_LEN || nonce.chars().any(char::is_control) {
        return Err(RemoteIdentityError::InvalidInput("nonce"));
    }
    Ok(())
}

fn proof_message(nonce: &str, host_uuid: &str, callsign: &str) -> String {
    format!("{REMOTE_PROOF_DOMAIN}\n{nonce}\n{host_uuid}\n{callsign}")
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
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in left.iter().zip(right.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn from_hex(value: &str) -> Option<Vec<u8>> {
    if value.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(value.len() / 2);
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let high = hex_digit(bytes[index])?;
        let low = hex_digit(bytes[index + 1])?;
        out.push((high << 4) | low);
        index += 2;
    }
    Some(out)
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

    fn host() -> RemoteIdentity {
        RemoteIdentity::new("11111111-1111-4111-8111-111111111111", "Venus", SECRET).expect("host")
    }

    #[test]
    fn valid_response_verifies() {
        let host = host();
        let challenge = RemoteIdentity::challenge();
        let response = host.respond(&challenge).expect("respond");
        verify_response(SECRET, host.host_uuid(), &challenge, &response).expect("verify");
    }

    #[test]
    fn wrong_secret_is_rejected() {
        let host = host();
        let challenge = RemoteIdentity::challenge();
        let response = host.respond(&challenge).expect("respond");
        let error = verify_response(
            b"a-different-but-still-long-secret",
            host.host_uuid(),
            &challenge,
            &response,
        );
        assert_eq!(error, Err(RemoteIdentityError::BadProof));
    }

    #[test]
    fn tampered_nonce_or_host_is_rejected() {
        let host = host();
        let challenge = RemoteIdentity::challenge();
        let mut response = host.respond(&challenge).expect("respond");

        let replay = RemoteIdentity::challenge();
        assert_eq!(
            verify_response(SECRET, host.host_uuid(), &replay, &response),
            Err(RemoteIdentityError::NonceMismatch)
        );

        response.host_uuid = "22222222-2222-4222-8222-222222222222".to_owned();
        assert_eq!(
            verify_response(
                SECRET,
                "22222222-2222-4222-8222-222222222222",
                &challenge,
                &response
            ),
            Err(RemoteIdentityError::BadProof)
        );

        let mut relabelled = host.respond(&challenge).expect("respond");
        relabelled.callsign = "Artemis".to_owned();
        assert_eq!(
            verify_response(SECRET, host.host_uuid(), &challenge, &relabelled),
            Err(RemoteIdentityError::BadProof)
        );
    }

    #[test]
    fn short_secret_and_bad_inputs_are_rejected() {
        assert!(matches!(
            RemoteIdentity::new("uuid", "Venus", b"short"),
            Err(RemoteIdentityError::SecretTooShort)
        ));
        assert!(RemoteIdentity::new("", "Venus", SECRET).is_err());
        assert!(RemoteIdentity::new("uuid", "", SECRET).is_err());
    }

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
    fn hex_round_trips() {
        let bytes = [0u8, 1, 15, 16, 127, 128, 255];
        let hex = to_hex(&bytes);
        assert_eq!(from_hex(&hex).as_deref(), Some(bytes.as_slice()));
        assert!(from_hex("abc").is_none());
        assert!(from_hex("zz").is_none());
    }
}

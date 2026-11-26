//! Endpoint transport classification and policy enforcement.
//!
//! Fleet endpoints are plain `http://` URLs today: the built-in Ollama adapter
//! speaks HTTP only and cannot itself terminate TLS. This module makes the
//! resulting transport risk explicit. An endpoint is classified as loopback,
//! private-LAN (RFC 1918, link-local, IPv6 unique-local, or `.local` mDNS), or
//! public. A [`TransportPolicy`] rejects public plain-http endpoints unless the
//! caller explicitly opts in with `allow_insecure_public`, which is intended
//! only for throwaway lab testing.
//!
//! Loopback and private-LAN plain-http endpoints are allowed by default for a
//! homelab. A public inference endpoint must be fronted by a TLS terminator (for
//! example a reverse proxy on the same host or LAN); the adapter then talks
//! plain HTTP to that local proxy. Configuring an `https://` endpoint here is
//! accepted by the policy, but the bundled adapter cannot speak TLS, so the
//! proxy must be addressed over HTTP on a loopback/private address.
//!
//! Classification is purely syntactic and offline; it performs no DNS lookup,
//! so it cannot detect a public hostname that resolves to a private address (or
//! vice versa). Treat it as a guardrail, not a firewall.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Schema identifier for a validated endpoint transport record.
pub const TRANSPORT_SECURITY_SCHEMA: &str = "aegis.fleet.transport_security.v1";
/// Default Ollama port, used only in documentation and tests.
pub const DEFAULT_OLLAMA_PORT: u16 = 11434;
/// Note describing the built-in adapter's transport limitation.
pub const OLLAMA_ADAPTER_TRANSPORT_NOTE: &str = "The built-in Ollama adapter only speaks plain \
http and cannot terminate TLS. A public inference endpoint must be fronted by a TLS terminator \
addressed over http on a loopback or private-LAN address.";

/// Where an endpoint host lives from a routing perspective.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointClass {
    Loopback,
    PrivateLan,
    Public,
}

impl EndpointClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Loopback => "loopback",
            Self::PrivateLan => "private_lan",
            Self::Public => "public",
        }
    }

    /// True for loopback and private-LAN hosts, which are allowed plain HTTP by
    /// default.
    pub const fn is_local(self) -> bool {
        matches!(self, Self::Loopback | Self::PrivateLan)
    }
}

/// URL scheme of an endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportScheme {
    Http,
    Https,
}

impl TransportScheme {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Https => "https",
        }
    }

    /// True only for `https`, i.e. the endpoint itself is TLS-fronted.
    pub const fn is_tls(self) -> bool {
        matches!(self, Self::Https)
    }
}

/// Policy controlling which endpoint transports are acceptable.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportPolicy {
    /// Allow plain HTTP to a public host. Default `false`; lab use only.
    pub allow_insecure_public: bool,
}

impl TransportPolicy {
    /// Strict default: public plain HTTP is rejected.
    pub const fn strict() -> Self {
        Self {
            allow_insecure_public: false,
        }
    }

    /// Lab escape hatch: permit public plain HTTP. Not for normal use.
    pub const fn allow_insecure_public() -> Self {
        Self {
            allow_insecure_public: true,
        }
    }

    pub const fn with_allow_insecure_public(self, allow: bool) -> Self {
        Self {
            allow_insecure_public: allow,
        }
    }
}

/// Endpoint transport rejection.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TransportError {
    #[error("invalid endpoint: {0}")]
    InvalidEndpoint(String),
    #[error("unsupported endpoint scheme: {0}")]
    UnsupportedScheme(String),
    #[error(
        "public plain-http endpoint {0} is not allowed; front it with TLS or set \
         allow_insecure_public (lab only)"
    )]
    PublicPlainHttp(String),
}

/// Parsed endpoint with its transport classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedEndpoint {
    pub endpoint: String,
    pub scheme: TransportScheme,
    pub host: String,
    pub port: Option<u16>,
    pub class: EndpointClass,
}

/// Result of a policy check for an endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointSecurity {
    pub schema: String,
    pub endpoint: String,
    pub scheme: TransportScheme,
    pub class: EndpointClass,
    pub tls_fronted: bool,
    /// False when the endpoint uses `https`; the bundled adapter cannot use it.
    pub adapter_supports_transport: bool,
    pub requires_tls_fronting: bool,
}

/// Classify an endpoint's scheme and host without any network access.
pub fn classify_endpoint(endpoint: &str) -> Result<ParsedEndpoint, TransportError> {
    let trimmed = endpoint.trim();
    if trimmed.is_empty() || trimmed.chars().any(char::is_control) {
        return Err(TransportError::InvalidEndpoint(endpoint.to_owned()));
    }
    let (scheme, rest) = trimmed
        .split_once("://")
        .ok_or_else(|| TransportError::InvalidEndpoint(endpoint.to_owned()))?;
    let scheme = match scheme.to_ascii_lowercase().as_str() {
        "http" => TransportScheme::Http,
        "https" => TransportScheme::Https,
        other => return Err(TransportError::UnsupportedScheme(other.to_owned())),
    };
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    if authority.is_empty() || authority.contains('@') {
        return Err(TransportError::InvalidEndpoint(endpoint.to_owned()));
    }
    let (host, port) = split_host_port(authority, endpoint)?;
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return Err(TransportError::InvalidEndpoint(endpoint.to_owned()));
    }
    let class = classify_host(&host);
    Ok(ParsedEndpoint {
        endpoint: trimmed.trim_end_matches('/').to_owned(),
        scheme,
        host,
        port,
        class,
    })
}

/// Enforce `policy` for `endpoint`, returning its classification on success.
///
/// Public plain-HTTP endpoints are rejected unless the policy allows insecure
/// public endpoints. Loopback and private-LAN plain HTTP are allowed by default.
pub fn validate_endpoint_security(
    endpoint: &str,
    policy: &TransportPolicy,
) -> Result<EndpointSecurity, TransportError> {
    let parsed = classify_endpoint(endpoint)?;
    if parsed.scheme == TransportScheme::Http
        && parsed.class == EndpointClass::Public
        && !policy.allow_insecure_public
    {
        return Err(TransportError::PublicPlainHttp(parsed.endpoint));
    }
    Ok(EndpointSecurity {
        schema: TRANSPORT_SECURITY_SCHEMA.to_owned(),
        tls_fronted: parsed.scheme.is_tls(),
        adapter_supports_transport: parsed.scheme == TransportScheme::Http,
        requires_tls_fronting: parsed.class == EndpointClass::Public,
        endpoint: parsed.endpoint,
        scheme: parsed.scheme,
        class: parsed.class,
    })
}

/// Split `host[:port]` from an authority, supporting bracketed IPv6 literals.
fn split_host_port<'a>(
    authority: &'a str,
    endpoint: &str,
) -> Result<(&'a str, Option<u16>), TransportError> {
    if let Some(rest) = authority.strip_prefix('[') {
        let end = rest
            .find(']')
            .ok_or_else(|| TransportError::InvalidEndpoint(endpoint.to_owned()))?;
        let host = &rest[..end];
        let tail = &rest[end + 1..];
        let port = if tail.is_empty() {
            None
        } else {
            let port = tail
                .strip_prefix(':')
                .ok_or_else(|| TransportError::InvalidEndpoint(endpoint.to_owned()))?;
            Some(parse_port(port, endpoint)?)
        };
        return Ok((host, port));
    }
    match authority.rsplit_once(':') {
        Some((host, port)) => {
            if host.contains(':') {
                return Err(TransportError::InvalidEndpoint(endpoint.to_owned()));
            }
            Ok((host, Some(parse_port(port, endpoint)?)))
        }
        None => Ok((authority, None)),
    }
}

fn parse_port(value: &str, endpoint: &str) -> Result<u16, TransportError> {
    if value.is_empty() || !value.chars().all(|character| character.is_ascii_digit()) {
        return Err(TransportError::InvalidEndpoint(endpoint.to_owned()));
    }
    match value.parse::<u16>() {
        Ok(0) | Err(_) => Err(TransportError::InvalidEndpoint(endpoint.to_owned())),
        Ok(port) => Ok(port),
    }
}

fn classify_host(host: &str) -> EndpointClass {
    if host == "localhost" || host.ends_with(".localhost") {
        return EndpointClass::Loopback;
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        return classify_ip(ip);
    }
    if host == "local" || host.ends_with(".local") {
        return EndpointClass::PrivateLan;
    }
    EndpointClass::Public
}

fn classify_ip(ip: IpAddr) -> EndpointClass {
    match ip {
        IpAddr::V4(ipv4) => classify_ipv4(ipv4),
        IpAddr::V6(ipv6) => classify_ipv6(ipv6),
    }
}

fn classify_ipv4(ip: Ipv4Addr) -> EndpointClass {
    if ip.is_loopback() {
        EndpointClass::Loopback
    } else if ip.is_private() || ip.is_link_local() {
        EndpointClass::PrivateLan
    } else {
        EndpointClass::Public
    }
}

fn classify_ipv6(ip: Ipv6Addr) -> EndpointClass {
    if ip.is_loopback() {
        return EndpointClass::Loopback;
    }
    if let Some(ipv4) = ipv4_mapped(ip) {
        return classify_ipv4(ipv4);
    }
    let segments = ip.segments();
    let unique_local = segments[0] & 0xfe00 == 0xfc00;
    let link_local = segments[0] & 0xffc0 == 0xfe80;
    if unique_local || link_local {
        EndpointClass::PrivateLan
    } else {
        EndpointClass::Public
    }
}

/// Decode an IPv4-mapped IPv6 address (`::ffff:a.b.c.d`) if present.
fn ipv4_mapped(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    let segments = ip.segments();
    if segments[..5] == [0, 0, 0, 0, 0] && segments[5] == 0xffff {
        let octets = [
            (segments[6] >> 8) as u8,
            (segments[6] & 0xff) as u8,
            (segments[7] >> 8) as u8,
            (segments[7] & 0xff) as u8,
        ];
        Some(Ipv4Addr::from(octets))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class(endpoint: &str) -> EndpointClass {
        classify_endpoint(endpoint).expect("endpoint").class
    }

    #[test]
    fn loopback_endpoints_classify_as_loopback() {
        for endpoint in [
            "http://127.0.0.1:11434",
            "http://127.1.2.3",
            "http://localhost:11434",
            "http://LOCALHOST",
            "https://[::1]:11434",
            "http://[::ffff:127.0.0.1]:11434",
        ] {
            assert_eq!(class(endpoint), EndpointClass::Loopback, "{endpoint}");
        }
    }

    #[test]
    fn private_lan_endpoints_classify_as_private() {
        for endpoint in [
            "http://10.0.0.5:11434",
            "http://172.16.4.2:11434",
            "http://172.31.255.255",
            "http://192.168.1.55:11434",
            "http://169.254.10.20",
            "http://artemis.local:11434",
            "http://[fe80::1]:11434",
            "http://[fd00::5]:11434",
            "http://[::ffff:192.168.1.55]:11434",
        ] {
            assert_eq!(class(endpoint), EndpointClass::PrivateLan, "{endpoint}");
        }
    }

    #[test]
    fn public_endpoints_classify_as_public() {
        for endpoint in [
            "http://8.8.8.8:11434",
            "https://example.com",
            "http://172.32.0.1",
            "http://192.169.0.1",
            "http://[2001:db8::1]:11434",
            "http://0.0.0.0:11434",
            "http://[::]:11434",
        ] {
            assert_eq!(class(endpoint), EndpointClass::Public, "{endpoint}");
        }
    }

    #[test]
    fn parser_extracts_host_port_and_strips_path() {
        let parsed = classify_endpoint("http://127.0.0.1:11434/api/tags?x=1").expect("parsed");
        assert_eq!(parsed.host, "127.0.0.1");
        assert_eq!(parsed.port, Some(11434));
        assert_eq!(parsed.scheme, TransportScheme::Http);
        assert_eq!(parsed.endpoint, "http://127.0.0.1:11434/api/tags?x=1");

        let parsed = classify_endpoint("https://[2001:db8::1]/v1").expect("parsed");
        assert_eq!(parsed.host, "2001:db8::1");
        assert_eq!(parsed.port, None);
        assert_eq!(parsed.scheme, TransportScheme::Https);
    }

    #[test]
    fn malformed_endpoints_are_rejected() {
        for endpoint in [
            "",
            "   ",
            "127.0.0.1:11434",
            "ftp://127.0.0.1",
            "http://",
            "http://user:pass@127.0.0.1",
            "http://127.0.0.1:notaport",
            "http://127.0.0.1:0",
            "http://::1:11434",
            "http://[::1",
        ] {
            assert!(
                classify_endpoint(endpoint).is_err(),
                "expected rejection: {endpoint}"
            );
        }
    }

    #[test]
    fn public_plain_http_is_rejected_by_default() {
        let policy = TransportPolicy::default();
        assert_eq!(
            validate_endpoint_security("http://8.8.8.8:11434", &policy),
            Err(TransportError::PublicPlainHttp(
                "http://8.8.8.8:11434".to_owned()
            ))
        );
        assert!(matches!(
            validate_endpoint_security("http://example.com:11434", &policy),
            Err(TransportError::PublicPlainHttp(_))
        ));
    }

    #[test]
    fn local_plain_http_is_allowed_by_default() {
        let policy = TransportPolicy::strict();
        for endpoint in [
            "http://127.0.0.1:11434",
            "http://192.168.1.55:11434",
            "http://artemis.local:11434",
        ] {
            let security = validate_endpoint_security(endpoint, &policy).expect("allowed");
            assert!(!security.tls_fronted);
            assert!(security.adapter_supports_transport);
            assert!(!security.requires_tls_fronting);
        }
    }

    #[test]
    fn public_https_is_allowed_but_adapter_cannot_use_it() {
        let security =
            validate_endpoint_security("https://example.com", &TransportPolicy::default())
                .expect("allowed");
        assert!(security.tls_fronted);
        assert!(security.requires_tls_fronting);
        assert!(
            !security.adapter_supports_transport,
            "bundled adapter cannot speak https"
        );
    }

    #[test]
    fn insecure_public_opt_in_permits_public_plain_http() {
        let policy = TransportPolicy::default().with_allow_insecure_public(true);
        let security =
            validate_endpoint_security("http://203.0.113.10:11434", &policy).expect("allowed");
        assert_eq!(security.class, EndpointClass::Public);
        assert!(!security.tls_fronted);
        assert!(security.requires_tls_fronting);
        assert!(security.adapter_supports_transport);
    }

    #[test]
    fn policy_constructors_and_class_helpers_are_consistent() {
        assert!(!TransportPolicy::strict().allow_insecure_public);
        assert!(!TransportPolicy::default().allow_insecure_public);
        assert!(TransportPolicy::allow_insecure_public().allow_insecure_public);
        assert!(EndpointClass::Loopback.is_local());
        assert!(EndpointClass::PrivateLan.is_local());
        assert!(!EndpointClass::Public.is_local());
        assert_eq!(EndpointClass::PrivateLan.as_str(), "private_lan");
        assert_eq!(TransportScheme::Https.as_str(), "https");
    }
}

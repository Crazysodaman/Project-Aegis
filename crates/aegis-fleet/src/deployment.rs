//! Fleet deployment readiness and safe agent configuration templates.
//!
//! Both helpers in this module are pure: they read a host record snapshot and
//! return a report or configuration text. They never touch the database, the
//! network, the environment, or the filesystem, and they never accept or emit a
//! secret *value*. A readiness check only receives a boolean indicating whether
//! the shared secret has been provisioned out of band; templates name the
//! environment variable that holds it.
//!
//! This is intentionally a pre-flight gate for an Artemis agent. It does not
//! install, enroll, or connect to anything: those steps remain owner-gated.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::capabilities::HostCapabilities;
use crate::transport::{
    classify_endpoint, validate_endpoint_security, EndpointClass, TransportError, TransportPolicy,
    OLLAMA_ADAPTER_TRANSPORT_NOTE,
};
use crate::{FleetHost, HostRole};

/// Schema identifier for a deployment readiness report.
pub const DEPLOYMENT_READINESS_SCHEMA: &str = "aegis.fleet.deployment_readiness.v1";
/// Schema identifier for a rendered agent configuration.
pub const AGENT_CONFIG_TEMPLATE_SCHEMA: &str = "aegis.fleet.agent_config.v1";
/// Default environment variable that holds the remote identity shared secret.
pub const DEFAULT_SECRET_ENV_VAR: &str = "AEGIS_REMOTE_SECRET";
/// Default loopback bind address for a Fleet identity agent.
pub const DEFAULT_AGENT_BIND: &str = "127.0.0.1:8787";
/// Default TLS-proxy guidance embedded in templates.
pub const DEFAULT_TLS_PROXY_NOTE: &str = OLLAMA_ADAPTER_TRANSPORT_NOTE;

/// Stable blocker code: no Fleet host record exists.
pub const BLOCKER_HOST_RECORD_MISSING: &str = "host_record_missing";
/// Stable blocker code: the host record is not enrolled.
pub const BLOCKER_HOST_NOT_ENROLLED: &str = "host_not_enrolled";
/// Stable blocker code: the host record is not trusted.
pub const BLOCKER_HOST_NOT_TRUSTED: &str = "host_not_trusted";
/// Stable blocker code: the endpoint transport policy rejected the endpoint.
pub const BLOCKER_TRANSPORT_REJECTED: &str = "transport_rejected";
/// Stable blocker code: the shared secret is not provisioned.
pub const BLOCKER_SECRET_NOT_PROVISIONED: &str = "secret_not_provisioned";
/// Stable blocker code: the callsign is missing.
pub const BLOCKER_CALLSIGN_MISSING: &str = "callsign_missing";

/// Severity of a readiness finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessSeverity {
    Blocker,
    Warning,
}

/// A single actionable readiness finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadinessBlocker {
    pub code: String,
    pub severity: ReadinessSeverity,
    pub message: String,
}

/// Inputs for a pure deployment readiness evaluation.
#[derive(Debug, Clone)]
pub struct DeploymentInput<'a> {
    /// Callsign the caller intends to deploy.
    pub callsign: &'a str,
    /// Snapshot of the Fleet host record, if one exists.
    pub host: Option<&'a FleetHost>,
    /// Transport policy to enforce against the host endpoint.
    pub transport_policy: TransportPolicy,
    /// Whether the shared secret is already provisioned out of band.
    ///
    /// Only a boolean is accepted so a secret value can never reach this module.
    pub secret_provisioned: bool,
}

/// Pure pre-flight report for deploying a Fleet agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentReadiness {
    pub schema: String,
    pub callsign: String,
    pub host_id: Option<String>,
    pub host_record_present: bool,
    pub enrolled: bool,
    pub trusted: bool,
    pub endpoint: Option<String>,
    pub endpoint_class: Option<EndpointClass>,
    pub transport_accepted: bool,
    pub capabilities_valid: bool,
    pub declares_inference: bool,
    pub secret_provisioned: bool,
    pub ready: bool,
    pub blockers: Vec<ReadinessBlocker>,
    pub warnings: Vec<ReadinessBlocker>,
}

impl DeploymentReadiness {
    pub fn is_ready(&self) -> bool {
        self.ready
    }

    /// Blocker codes in deterministic report order.
    pub fn blocker_codes(&self) -> Vec<&str> {
        self.blockers
            .iter()
            .map(|blocker| blocker.code.as_str())
            .collect()
    }
}

/// Evaluate deployment prerequisites for a Fleet agent. Pure and deterministic.
pub fn evaluate_deployment_readiness(input: &DeploymentInput<'_>) -> DeploymentReadiness {
    let mut blockers = Vec::new();
    let mut warnings = Vec::new();

    let record_callsign = input.host.map(|host| host.callsign.trim());
    let callsign = record_callsign.unwrap_or(input.callsign.trim()).to_owned();
    if callsign.is_empty() {
        blockers.push(finding(
            BLOCKER_CALLSIGN_MISSING,
            ReadinessSeverity::Blocker,
            "Provide a non-empty callsign for the agent.",
        ));
    }
    if let (Some(record_callsign), requested) = (record_callsign, input.callsign.trim()) {
        if !requested.is_empty() && requested != record_callsign {
            warnings.push(finding(
                "callsign_mismatch",
                ReadinessSeverity::Warning,
                format!(
                    "Requested callsign {requested} differs from record callsign {record_callsign}."
                ),
            ));
        }
    }

    let host_record_present = input.host.is_some();
    let host = input.host;
    if host.is_none() {
        blockers.push(finding(
            BLOCKER_HOST_RECORD_MISSING,
            ReadinessSeverity::Blocker,
            "Register the host with FleetStore::add_host using the owner token.",
        ));
    }

    let enrolled = host.is_some_and(|host| host.enrolled);
    let trusted = host.is_some_and(|host| host.trusted);
    if host.is_some() && !enrolled {
        blockers.push(finding(
            BLOCKER_HOST_NOT_ENROLLED,
            ReadinessSeverity::Blocker,
            "Enroll the host with FleetStore::enroll(host_id, owner_token).",
        ));
    }
    if host.is_some() && !trusted {
        blockers.push(finding(
            BLOCKER_HOST_NOT_TRUSTED,
            ReadinessSeverity::Blocker,
            "The host is not trusted; re-enroll explicitly with the owner token.",
        ));
    }

    let mut endpoint = None;
    let mut endpoint_class = None;
    let mut transport_accepted = false;
    if let Some(host) = host {
        match validate_endpoint_security(&host.endpoint, &input.transport_policy) {
            Ok(security) => {
                endpoint = Some(security.endpoint);
                endpoint_class = Some(security.class);
                transport_accepted = true;
            }
            Err(error) => {
                if let Ok(parsed) = classify_endpoint(&host.endpoint) {
                    endpoint = Some(parsed.endpoint);
                    endpoint_class = Some(parsed.class);
                }
                blockers.push(transport_finding(&error));
            }
        }
    }

    let (capabilities_valid, declares_inference) = match host {
        Some(host) => match HostCapabilities::from_value(&host.capabilities) {
            Ok(capabilities) => (true, capabilities.supports_inference()),
            Err(_) => (false, host.role == HostRole::Inference),
        },
        None => (false, false),
    };
    if let Some(host) = host {
        if !capabilities_valid {
            warnings.push(finding(
                "capabilities_not_declared",
                ReadinessSeverity::Warning,
                "Publish HostCapabilities::to_value() so inference advertisement is explicit.",
            ));
        } else if host.role == HostRole::Inference && !declares_inference {
            warnings.push(finding(
                "inference_role_without_capability",
                ReadinessSeverity::Warning,
                "The host role is inference but its capabilities do not declare inference.",
            ));
        } else if host.role != HostRole::Inference && declares_inference {
            warnings.push(finding(
                "inference_capability_without_role",
                ReadinessSeverity::Warning,
                "The capabilities declare inference but the host role is not inference.",
            ));
        }
    }

    if !input.secret_provisioned {
        blockers.push(finding(
            BLOCKER_SECRET_NOT_PROVISIONED,
            ReadinessSeverity::Blocker,
            format!(
                "Set {DEFAULT_SECRET_ENV_VAR} (at least 16 bytes) out of band on both hosts; \
                 never pass it on the command line or log it."
            ),
        ));
    }

    DeploymentReadiness {
        schema: DEPLOYMENT_READINESS_SCHEMA.to_owned(),
        callsign,
        host_id: host.map(|host| host.host_id.clone()),
        host_record_present,
        enrolled,
        trusted,
        endpoint,
        endpoint_class,
        transport_accepted,
        capabilities_valid,
        declares_inference,
        secret_provisioned: input.secret_provisioned,
        ready: blockers.is_empty(),
        blockers,
        warnings,
    }
}

pub(crate) fn finding(
    code: &str,
    severity: ReadinessSeverity,
    message: impl Into<String>,
) -> ReadinessBlocker {
    ReadinessBlocker {
        code: code.to_owned(),
        severity,
        message: message.into(),
    }
}

pub(crate) fn transport_finding(error: &TransportError) -> ReadinessBlocker {
    match error {
        TransportError::PublicPlainHttp(endpoint) => finding(
            BLOCKER_TRANSPORT_REJECTED,
            ReadinessSeverity::Blocker,
            format!(
                "Endpoint {endpoint} is public plain http. Front it with TLS or, for lab-only \
                 use, set allow_insecure_public."
            ),
        ),
        TransportError::InvalidEndpoint(endpoint) => finding(
            BLOCKER_TRANSPORT_REJECTED,
            ReadinessSeverity::Blocker,
            format!("Endpoint {endpoint} is malformed; use http://host[:port]."),
        ),
        TransportError::UnsupportedScheme(scheme) => finding(
            BLOCKER_TRANSPORT_REJECTED,
            ReadinessSeverity::Blocker,
            format!(
                "Endpoint scheme {scheme} is unsupported; use http, or https behind a local proxy."
            ),
        ),
    }
}

/// Output format for a rendered agent configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateFormat {
    Json,
    Toml,
}

impl TemplateFormat {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Toml => "toml",
        }
    }
}

/// Inputs for a secret-free agent configuration template.
///
/// There is deliberately no field for a secret value; only the name of the
/// environment variable that will hold it.
#[derive(Debug, Clone)]
pub struct AgentConfigTemplate<'a> {
    pub host_uuid: &'a str,
    pub callsign: &'a str,
    pub bind_address: &'a str,
    pub secret_env_var: &'a str,
    pub endpoint: &'a str,
    pub capabilities: &'a HostCapabilities,
    pub tls_proxy_note: &'a str,
}

/// Template rendering failure.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TemplateError {
    #[error("invalid template field: {0}")]
    InvalidField(&'static str),
    #[error("template serialization failed: {0}")]
    Serialization(String),
}

#[derive(Serialize)]
struct TemplateDocument<'a> {
    schema: &'a str,
    host_uuid: &'a str,
    callsign: &'a str,
    bind_address: &'a str,
    shared_secret_env: &'a str,
    contains_secret_values: bool,
    roles: &'a [HostRole],
    capabilities: &'a [String],
    inference: bool,
    endpoint: &'a str,
    tls_proxy_note: &'a str,
}

/// Render a deterministic, secret-free agent configuration template.
pub fn render_agent_config(
    template: &AgentConfigTemplate<'_>,
    format: TemplateFormat,
) -> Result<String, TemplateError> {
    validate_template(template)?;
    let document = TemplateDocument {
        schema: AGENT_CONFIG_TEMPLATE_SCHEMA,
        host_uuid: template.host_uuid.trim(),
        callsign: template.callsign.trim(),
        bind_address: template.bind_address.trim(),
        shared_secret_env: template.secret_env_var.trim(),
        contains_secret_values: false,
        roles: &template.capabilities.roles,
        capabilities: &template.capabilities.capabilities,
        inference: template.capabilities.inference,
        endpoint: template.endpoint.trim(),
        tls_proxy_note: template.tls_proxy_note,
    };
    match format {
        TemplateFormat::Json => serde_json::to_string_pretty(&document)
            .map_err(|error| TemplateError::Serialization(error.to_string())),
        TemplateFormat::Toml => Ok(render_toml(&document)),
    }
}

fn validate_template(template: &AgentConfigTemplate<'_>) -> Result<(), TemplateError> {
    if !is_safe_atom(template.host_uuid, 128) {
        return Err(TemplateError::InvalidField("host_uuid"));
    }
    if !is_safe_callsign(template.callsign) {
        return Err(TemplateError::InvalidField("callsign"));
    }
    if !is_safe_atom(template.bind_address, 128) {
        return Err(TemplateError::InvalidField("bind_address"));
    }
    if !is_env_var_name(template.secret_env_var) {
        return Err(TemplateError::InvalidField("secret_env_var"));
    }
    if classify_endpoint(template.endpoint).is_err() {
        return Err(TemplateError::InvalidField("endpoint"));
    }
    if template.tls_proxy_note.chars().count() > 512
        || template
            .tls_proxy_note
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return Err(TemplateError::InvalidField("tls_proxy_note"));
    }
    Ok(())
}

fn is_safe_atom(value: &str, max: usize) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed.chars().count() <= max
        && !trimmed
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
}

fn is_safe_callsign(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed.chars().count() <= 64
        && !trimmed.chars().any(char::is_control)
        && !trimmed.starts_with('-')
        && !trimmed.ends_with('-')
}

fn is_env_var_name(value: &str) -> bool {
    let value = value.trim();
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !(first.is_ascii_uppercase() || first == '_') {
        return false;
    }
    value.len() <= 64
        && characters.all(|character| {
            character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
        })
}

fn render_toml(document: &TemplateDocument<'_>) -> String {
    let mut output = String::new();
    writeln!(output, "schema = {}", toml_string(document.schema))
        .expect("writing to a String cannot fail");
    writeln!(output, "host_uuid = {}", toml_string(document.host_uuid))
        .expect("writing to a String cannot fail");
    writeln!(output, "callsign = {}", toml_string(document.callsign))
        .expect("writing to a String cannot fail");
    writeln!(
        output,
        "bind_address = {}",
        toml_string(document.bind_address)
    )
    .expect("writing to a String cannot fail");
    writeln!(
        output,
        "shared_secret_env = {}",
        toml_string(document.shared_secret_env)
    )
    .expect("writing to a String cannot fail");
    writeln!(
        output,
        "contains_secret_values = {}",
        document.contains_secret_values
    )
    .expect("writing to a String cannot fail");

    output.push_str("roles = [");
    for (index, role) in document.roles.iter().enumerate() {
        if index > 0 {
            output.push_str(", ");
        }
        output.push_str(&toml_string(role.as_str()));
    }
    output.push_str("]\n");

    output.push_str("capabilities = [");
    for (index, capability) in document.capabilities.iter().enumerate() {
        if index > 0 {
            output.push_str(", ");
        }
        output.push_str(&toml_string(capability));
    }
    output.push_str("]\n");

    writeln!(output, "inference = {}", document.inference)
        .expect("writing to a String cannot fail");
    writeln!(output, "endpoint = {}", toml_string(document.endpoint))
        .expect("writing to a String cannot fail");
    writeln!(
        output,
        "tls_proxy_note = {}",
        toml_string(document.tls_proxy_note)
    )
    .expect("writing to a String cannot fail");
    output
}

fn toml_string(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            control if control.is_control() => {
                write!(output, "\\u{:04X}", control as u32)
                    .expect("writing to a String cannot fail");
            }
            character => output.push(character),
        }
    }
    output.push('"');
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::EndpointClass;
    use crate::FLEET_HOST_SCHEMA;

    const SECRET_VALUE: &str = "correct-horse-battery-staple-do-not-leak";

    fn host() -> FleetHost {
        FleetHost {
            schema: FLEET_HOST_SCHEMA.to_owned(),
            host_id: "11111111-1111-4111-8111-111111111111".to_owned(),
            callsign: "Artemis".to_owned(),
            endpoint: "http://192.168.1.55:11434".to_owned(),
            role: HostRole::General,
            capabilities: serde_json::Value::Null,
            enrolled: true,
            trusted: true,
            priority: 50,
            created_at: "2026-01-01T00:00:00Z".to_owned(),
            updated_at: "2026-01-01T00:00:00Z".to_owned(),
            revision: 1,
            health: None,
        }
    }

    fn general_caps() -> HostCapabilities {
        HostCapabilities::new([HostRole::General], ["smb"]).expect("caps")
    }

    fn inference_caps() -> HostCapabilities {
        HostCapabilities::new([HostRole::Inference], ["ollama"]).expect("caps")
    }

    fn input<'a>(host: Option<&'a FleetHost>) -> DeploymentInput<'a> {
        DeploymentInput {
            callsign: "Artemis",
            host,
            transport_policy: TransportPolicy::strict(),
            secret_provisioned: true,
        }
    }

    #[test]
    fn ready_when_all_prerequisites_are_met() {
        let mut host = host();
        host.capabilities = general_caps().to_value().expect("value");
        let report = evaluate_deployment_readiness(&input(Some(&host)));
        assert!(report.ready, "blockers: {:?}", report.blockers);
        assert!(report.is_ready());
        assert!(report.blockers.is_empty());
        assert!(
            report.warnings.is_empty(),
            "warnings: {:?}",
            report.warnings
        );
        assert!(report.host_record_present);
        assert!(report.enrolled);
        assert!(report.trusted);
        assert!(report.transport_accepted);
        assert_eq!(report.endpoint_class, Some(EndpointClass::PrivateLan));
        assert!(!report.declares_inference);
        assert!(report.capabilities_valid);
        assert!(report.secret_provisioned);
        assert_eq!(report.callsign, "Artemis");
    }

    #[test]
    fn missing_host_record_blocks_deployment() {
        let report = evaluate_deployment_readiness(&input(None));
        assert!(!report.ready);
        assert!(report
            .blocker_codes()
            .contains(&BLOCKER_HOST_RECORD_MISSING));
        assert_eq!(report.host_id, None);
        assert!(!report.transport_accepted);
    }

    #[test]
    fn unenrolled_and_untrusted_host_blocks_deployment() {
        let mut host = host();
        host.enrolled = false;
        host.trusted = false;
        host.capabilities = general_caps().to_value().expect("value");
        let report = evaluate_deployment_readiness(&input(Some(&host)));
        assert!(!report.ready);
        assert!(report.blocker_codes().contains(&BLOCKER_HOST_NOT_ENROLLED));
        assert!(report.blocker_codes().contains(&BLOCKER_HOST_NOT_TRUSTED));
    }

    #[test]
    fn public_plain_http_blocks_unless_explicitly_allowed() {
        let mut host = host();
        host.endpoint = "http://8.8.8.8:11434".to_owned();
        host.capabilities = general_caps().to_value().expect("value");

        let blocked = evaluate_deployment_readiness(&input(Some(&host)));
        assert!(!blocked.ready);
        assert!(!blocked.transport_accepted);
        assert_eq!(blocked.endpoint_class, Some(EndpointClass::Public));
        assert!(blocked
            .blocker_codes()
            .contains(&BLOCKER_TRANSPORT_REJECTED));

        let allowed = evaluate_deployment_readiness(&DeploymentInput {
            transport_policy: TransportPolicy::default().with_allow_insecure_public(true),
            ..input(Some(&host))
        });
        assert!(allowed.ready, "blockers: {:?}", allowed.blockers);
        assert!(allowed.transport_accepted);
    }

    #[test]
    fn missing_secret_blocks_deployment() {
        let mut host = host();
        host.capabilities = general_caps().to_value().expect("value");
        let report = evaluate_deployment_readiness(&DeploymentInput {
            secret_provisioned: false,
            ..input(Some(&host))
        });
        assert!(!report.ready);
        assert!(!report.secret_provisioned);
        assert!(report
            .blocker_codes()
            .contains(&BLOCKER_SECRET_NOT_PROVISIONED));
        assert!(report
            .blockers
            .iter()
            .all(|blocker| !blocker.message.contains(SECRET_VALUE)));
    }

    #[test]
    fn capability_inference_reporting_is_explicit() {
        let mut host = host();
        host.role = HostRole::Inference;
        host.capabilities = inference_caps().to_value().expect("value");
        let report = evaluate_deployment_readiness(&input(Some(&host)));
        assert!(report.capabilities_valid);
        assert!(report.declares_inference);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);

        let mut mismatch = self::host();
        mismatch.role = HostRole::Inference;
        mismatch.capabilities = general_caps().to_value().expect("value");
        let mismatch_report = evaluate_deployment_readiness(&input(Some(&mismatch)));
        assert!(!mismatch_report.declares_inference);
        assert!(mismatch_report
            .warnings
            .iter()
            .any(|warning| warning.code == "inference_role_without_capability"));
    }

    #[test]
    fn null_capabilities_warn_but_do_not_block() {
        let host = host();
        let report = evaluate_deployment_readiness(&input(Some(&host)));
        assert!(report.ready, "blockers: {:?}", report.blockers);
        assert!(!report.capabilities_valid);
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.code == "capabilities_not_declared"));
    }

    #[test]
    fn invalid_endpoint_is_actionable() {
        let mut host = host();
        host.endpoint = "ftp://192.168.1.55".to_owned();
        host.capabilities = general_caps().to_value().expect("value");
        let report = evaluate_deployment_readiness(&input(Some(&host)));
        assert!(!report.ready);
        let blocker = report
            .blockers
            .iter()
            .find(|blocker| blocker.code == BLOCKER_TRANSPORT_REJECTED)
            .expect("transport blocker");
        assert!(blocker.message.contains("unsupported"));
    }

    #[test]
    fn missing_callsign_is_actionable() {
        let report = evaluate_deployment_readiness(&DeploymentInput {
            callsign: "   ",
            host: None,
            transport_policy: TransportPolicy::strict(),
            secret_provisioned: true,
        });
        assert!(!report.ready);
        assert!(report.blocker_codes().contains(&BLOCKER_CALLSIGN_MISSING));
    }

    #[test]
    fn readiness_report_is_deterministic() {
        let mut host = host();
        host.capabilities = general_caps().to_value().expect("value");
        let first = evaluate_deployment_readiness(&input(Some(&host)));
        let second = evaluate_deployment_readiness(&input(Some(&host)));
        assert_eq!(first, second);
    }

    fn template<'a>(capabilities: &'a HostCapabilities, note: &'a str) -> AgentConfigTemplate<'a> {
        AgentConfigTemplate {
            host_uuid: "11111111-1111-4111-8111-111111111111",
            callsign: "Artemis",
            bind_address: DEFAULT_AGENT_BIND,
            secret_env_var: DEFAULT_SECRET_ENV_VAR,
            endpoint: "http://127.0.0.1:11434",
            capabilities,
            tls_proxy_note: note,
        }
    }

    #[test]
    fn json_template_is_deterministic_and_secret_free() {
        let capabilities = inference_caps();
        let spec = template(&capabilities, DEFAULT_TLS_PROXY_NOTE);
        let first = render_agent_config(&spec, TemplateFormat::Json).expect("json");
        let second = render_agent_config(&spec, TemplateFormat::Json).expect("json");
        assert_eq!(first, second);
        assert!(first.contains(AGENT_CONFIG_TEMPLATE_SCHEMA));
        assert!(first.contains("\"shared_secret_env\": \"AEGIS_REMOTE_SECRET\""));
        assert!(first.contains("\"contains_secret_values\": false"));
        assert!(first.contains("\"inference\": true"));
        assert!(first.contains("\"roles\": ["));
        assert!(!first.contains(SECRET_VALUE));
        assert!(!first.contains("shared_secret_value"));
    }

    #[test]
    fn toml_template_is_deterministic_and_secret_free() {
        let capabilities = general_caps();
        let spec = template(&capabilities, DEFAULT_TLS_PROXY_NOTE);
        let first = render_agent_config(&spec, TemplateFormat::Toml).expect("toml");
        let second = render_agent_config(&spec, TemplateFormat::Toml).expect("toml");
        assert_eq!(first, second);
        assert!(first.contains(&format!("schema = \"{AGENT_CONFIG_TEMPLATE_SCHEMA}\"")));
        assert!(first.contains("shared_secret_env = \"AEGIS_REMOTE_SECRET\""));
        assert!(first.contains("contains_secret_values = false"));
        assert!(first.contains("inference = false"));
        assert!(first.contains("roles = [\"general\"]"));
        assert!(!first.contains(SECRET_VALUE));
        assert!(!first.contains("shared_secret_value"));
    }

    #[test]
    fn template_escapes_quotes_and_rejects_bad_fields() {
        let capabilities = general_caps();
        let spec = template(&capabilities, "note with \"quotes\" and\nnewline");
        let toml = render_agent_config(&spec, TemplateFormat::Toml).expect("toml");
        assert!(toml.contains("\\\"quotes\\\""));
        assert!(toml.contains("\\n"));

        let mut invalid = template(&capabilities, DEFAULT_TLS_PROXY_NOTE);
        invalid.host_uuid = "  ";
        assert_eq!(
            render_agent_config(&invalid, TemplateFormat::Json),
            Err(TemplateError::InvalidField("host_uuid"))
        );

        let mut invalid = template(&capabilities, DEFAULT_TLS_PROXY_NOTE);
        invalid.secret_env_var = "lower-case";
        assert_eq!(
            render_agent_config(&invalid, TemplateFormat::Toml),
            Err(TemplateError::InvalidField("secret_env_var"))
        );

        let mut invalid = template(&capabilities, DEFAULT_TLS_PROXY_NOTE);
        invalid.endpoint = "not-an-endpoint";
        assert_eq!(
            render_agent_config(&invalid, TemplateFormat::Json),
            Err(TemplateError::InvalidField("endpoint"))
        );

        let mut invalid = template(&capabilities, DEFAULT_TLS_PROXY_NOTE);
        invalid.callsign = "-Artemis-";
        assert_eq!(
            render_agent_config(&invalid, TemplateFormat::Json),
            Err(TemplateError::InvalidField("callsign"))
        );

        assert_eq!(TemplateFormat::Json.as_str(), "json");
        assert_eq!(TemplateFormat::Toml.as_str(), "toml");
    }
}

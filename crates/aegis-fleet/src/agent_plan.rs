//! Inert deployment package description for an Artemis Fleet identity agent.
//!
//! A [`FleetAgentPlan`] is data, not an action. It lists the exact files to copy,
//! the environment variable *names* (never values) the agent requires, its argv,
//! working directory, verification steps, and rollback steps. Building a plan
//! performs no filesystem, network, or process change; it never accepts or emits
//! a secret value.
//!
//! [`preflight_agent_plan`] is the only function in this module that inspects the
//! filesystem, and it does so solely to confirm the agent binary is present. It
//! never reads a secret: the caller supplies a boolean *indicator* that the
//! shared secret has already been provisioned out of band. All other preflight
//! checks (endpoint transport class, role/capability consistency) are pure.
//!
//! This module is intentionally a pre-flight gate only. It does not install,
//! enroll, start, or connect to anything on Artemis; those steps remain
//! owner-gated and blocked pending explicit approval.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::capabilities::HostCapabilities;
use crate::deployment::{
    finding, transport_finding, ReadinessBlocker, ReadinessSeverity, DEFAULT_SECRET_ENV_VAR,
};
use crate::transport::{
    classify_endpoint, validate_endpoint_security, EndpointClass, TransportPolicy,
};
use crate::{FleetError, FleetHost, HostRole};

/// Schema identifier for a Fleet agent deployment plan.
pub const AGENT_PLAN_SCHEMA: &str = "aegis.fleet.agent_plan.v1";
/// Schema identifier for a Fleet agent preflight report.
pub const AGENT_PREFLIGHT_SCHEMA: &str = "aegis.fleet.agent_preflight.v1";
/// Stable blocker code: the agent binary was not found on disk.
pub const BLOCKER_AGENT_BINARY_MISSING: &str = "agent_binary_missing";
/// Stable blocker code: the host has no endpoint to advertise.
pub const BLOCKER_ENDPOINT_MISSING: &str = "endpoint_missing";
/// Stable blocker code: inference is advertised without a usable endpoint.
pub const BLOCKER_INFERENCE_WITHOUT_ENDPOINT: &str = "inference_without_endpoint";

/// A file that must be copied to the agent host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentFileSpec {
    pub description: String,
    pub source: String,
    pub destination: String,
    pub required: bool,
}

/// An environment variable the agent expects. Only the name is recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentEnvSpec {
    pub name: String,
    pub required: bool,
    pub secret: bool,
    pub description: String,
}

/// One verification or rollback step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentStep {
    pub name: String,
    pub detail: String,
    pub command: Option<String>,
    pub expected: Option<String>,
}

/// A complete, secret-free deployment package for a Fleet identity agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FleetAgentPlan {
    pub schema: String,
    pub callsign: String,
    pub host_id: String,
    pub working_dir: String,
    pub files: Vec<AgentFileSpec>,
    pub env: Vec<AgentEnvSpec>,
    pub argv: Vec<String>,
    pub verification: Vec<AgentStep>,
    pub rollback: Vec<AgentStep>,
    /// Always false: the plan never carries secret values.
    pub contains_secret_values: bool,
}

/// Caller-supplied paths and binding for [`build_agent_plan`].
#[derive(Debug, Clone)]
pub struct AgentPlanOptions<'a> {
    /// Local path to the built identity agent binary.
    pub binary_source: &'a str,
    /// Optional local path to a rendered, secret-free agent configuration file.
    pub config_source: Option<&'a str>,
    /// Install directory that becomes the agent working directory.
    pub install_dir: &'a str,
    /// Address the agent's `serve` mode binds (for example `0.0.0.0:8787`).
    pub bind: &'a str,
    /// Address the verifier connects to (for example `192.168.1.71:8787`).
    pub connect_address: &'a str,
    /// Environment variable name holding the shared secret.
    pub secret_env_var: &'a str,
    /// Number of challenges the agent serves before exiting.
    pub accepts: u32,
}

/// Build a deterministic, secret-free plan for `host`.
pub fn build_agent_plan(
    host: &FleetHost,
    options: &AgentPlanOptions<'_>,
) -> Result<FleetAgentPlan, FleetError> {
    let callsign = host.callsign.trim();
    if callsign.is_empty() {
        return Err(FleetError::InvalidInput("callsign"));
    }
    if host.host_id.trim().is_empty() {
        return Err(FleetError::InvalidInput("host_id"));
    }
    if !is_env_var_name(options.secret_env_var) {
        return Err(FleetError::InvalidInput("secret_env_var"));
    }
    if !is_path_like(options.install_dir) {
        return Err(FleetError::InvalidInput("install_dir"));
    }
    if !is_path_like(options.binary_source) {
        return Err(FleetError::InvalidInput("binary_source"));
    }
    if !is_atom(options.bind, 128) {
        return Err(FleetError::InvalidInput("bind"));
    }
    if !is_atom(options.connect_address, 128) {
        return Err(FleetError::InvalidInput("connect_address"));
    }
    if options.accepts == 0 {
        return Err(FleetError::InvalidInput("accepts"));
    }

    let install_dir = options.install_dir.trim();
    let binary_name = Path::new(options.binary_source)
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(FleetError::InvalidInput("binary_source"))?;
    let binary_destination = Path::new(install_dir).join(binary_name);

    let mut files = vec![AgentFileSpec {
        description: "Fleet identity agent executable".to_owned(),
        source: options.binary_source.trim().to_owned(),
        destination: binary_destination.display().to_string(),
        required: true,
    }];
    let mut config_destination = None;
    if let Some(config_source) = options.config_source {
        if !is_path_like(config_source) {
            return Err(FleetError::InvalidInput("config_source"));
        }
        let config_name = Path::new(config_source)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("agent.toml");
        let destination = Path::new(install_dir).join(config_name);
        files.push(AgentFileSpec {
            description: "Secret-free agent configuration (env var name only)".to_owned(),
            source: config_source.trim().to_owned(),
            destination: destination.display().to_string(),
            required: true,
        });
        config_destination = Some(destination);
    }

    let env = vec![AgentEnvSpec {
        name: options.secret_env_var.trim().to_owned(),
        required: true,
        secret: true,
        description: "Shared remote-identity secret (at least 16 bytes). Provision it out of \
                      band on both hosts; never place it in argv, the plan, or logs."
            .to_owned(),
    }];

    let argv = vec![
        "serve".to_owned(),
        "--bind".to_owned(),
        options.bind.trim().to_owned(),
        "--host-uuid".to_owned(),
        host.host_id.clone(),
        "--callsign".to_owned(),
        callsign.to_owned(),
        "--accepts".to_owned(),
        options.accepts.to_string(),
    ];

    let verify_command = format!(
        "{binary_name} verify --connect {} --expect-host {}",
        options.connect_address.trim(),
        host.host_id
    );
    let verification = vec![
        AgentStep {
            name: "identity_handshake".to_owned(),
            detail: format!(
                "With {} set, run the verifier from a second host on the same LAN.",
                options.secret_env_var.trim()
            ),
            command: Some(verify_command.clone()),
            expected: Some("verified=true".to_owned()),
        },
        AgentStep {
            name: "wrong_secret_rejected".to_owned(),
            detail: format!(
                "Repeat the verifier with a different {} value; it must fail closed.",
                options.secret_env_var.trim()
            ),
            command: Some(verify_command),
            expected: Some("non-zero exit, proof is invalid".to_owned()),
        },
    ];

    let mut rollback = vec![
        AgentStep {
            name: "stop_agent".to_owned(),
            detail: format!("Stop the running {binary_name} process for {callsign}."),
            command: None,
            expected: Some("process no longer listening".to_owned()),
        },
        AgentStep {
            name: "revoke_host".to_owned(),
            detail: format!(
                "Call FleetStore::revoke with the owner token for host {}.",
                host.host_id
            ),
            command: None,
            expected: Some("host trusted=false".to_owned()),
        },
        AgentStep {
            name: "remove_host_record".to_owned(),
            detail: format!(
                "Call FleetStore::remove with the owner token for host {}.",
                host.host_id
            ),
            command: None,
            expected: Some("host record deleted".to_owned()),
        },
    ];
    if let Some(config_destination) = &config_destination {
        rollback.push(AgentStep {
            name: "delete_config".to_owned(),
            detail: format!(
                "Delete {} from the agent host.",
                config_destination.display()
            ),
            command: None,
            expected: Some("file absent".to_owned()),
        });
    }
    rollback.push(AgentStep {
        name: "delete_binary".to_owned(),
        detail: format!(
            "Delete {} from the agent host.",
            binary_destination.display()
        ),
        command: None,
        expected: Some("file absent".to_owned()),
    });

    Ok(FleetAgentPlan {
        schema: AGENT_PLAN_SCHEMA.to_owned(),
        callsign: callsign.to_owned(),
        host_id: host.host_id.clone(),
        working_dir: install_dir.to_owned(),
        files,
        env,
        argv,
        verification,
        rollback,
        contains_secret_values: false,
    })
}

/// Inputs for [`preflight_agent_plan`].
#[derive(Debug)]
pub struct AgentPreflightInput<'a> {
    pub plan: &'a FleetAgentPlan,
    pub host: Option<&'a FleetHost>,
    pub transport_policy: TransportPolicy,
    /// Boolean indicator only; a secret value is never accepted here.
    pub secret_provisioned: bool,
    /// Path checked with [`Path::is_file`] for agent binary presence.
    pub agent_binary_path: &'a Path,
}

/// Result of a Fleet agent preflight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentPreflight {
    pub schema: String,
    pub callsign: String,
    pub host_id: Option<String>,
    pub agent_binary_present: bool,
    pub endpoint: Option<String>,
    pub endpoint_class: Option<EndpointClass>,
    pub transport_accepted: bool,
    pub declares_inference: bool,
    pub inference_has_endpoint: bool,
    pub secret_provisioned: bool,
    pub secret_env_var: String,
    pub contains_secret_values: bool,
    pub ready: bool,
    pub blockers: Vec<ReadinessBlocker>,
    pub warnings: Vec<ReadinessBlocker>,
}

impl AgentPreflight {
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

/// Pre-flight a deployment plan against an optional host record.
///
/// Checks agent binary presence, endpoint transport class via
/// [`TransportPolicy`], role/capability consistency (inference is never
/// advertised without a policy-accepted endpoint), and a secret-provisioned
/// indicator. It never reveals a secret value.
pub fn preflight_agent_plan(input: &AgentPreflightInput<'_>) -> AgentPreflight {
    let mut blockers = Vec::new();
    let mut warnings = Vec::new();
    let plan = input.plan;

    let agent_binary_present = input.agent_binary_path.is_file();
    if !agent_binary_present {
        blockers.push(finding(
            BLOCKER_AGENT_BINARY_MISSING,
            ReadinessSeverity::Blocker,
            format!(
                "Agent binary not found at {}; copy the built identity agent before starting it.",
                input.agent_binary_path.display()
            ),
        ));
    }

    if plan.callsign.trim().is_empty() {
        blockers.push(finding(
            crate::deployment::BLOCKER_CALLSIGN_MISSING,
            ReadinessSeverity::Blocker,
            "The plan has no callsign.",
        ));
    }

    let secret_spec = plan.env.iter().find(|env| env.secret);
    let secret_env_var = secret_spec
        .map(|env| env.name.clone())
        .unwrap_or_else(|| DEFAULT_SECRET_ENV_VAR.to_owned());
    if secret_spec.is_none() {
        warnings.push(finding(
            "secret_env_missing_from_plan",
            ReadinessSeverity::Warning,
            "The plan does not name a secret environment variable.",
        ));
    }
    if !input.secret_provisioned {
        blockers.push(finding(
            crate::deployment::BLOCKER_SECRET_NOT_PROVISIONED,
            ReadinessSeverity::Blocker,
            format!(
                "Set {secret_env_var} (at least 16 bytes) out of band on both hosts; never pass \
                 it on the command line or log it."
            ),
        ));
    }
    if plan.contains_secret_values {
        blockers.push(finding(
            "secret_value_present_in_plan",
            ReadinessSeverity::Blocker,
            "The plan appears to contain a secret value; it must only name environment variables.",
        ));
    }

    let host = input.host;
    if host.is_none() {
        blockers.push(finding(
            crate::deployment::BLOCKER_HOST_RECORD_MISSING,
            ReadinessSeverity::Blocker,
            "Register the host with FleetStore::add_host using the owner token.",
        ));
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

    let mut endpoint = None;
    let mut endpoint_class = None;
    let mut transport_accepted = false;
    if let Some(host) = host {
        let raw = host.endpoint.trim();
        if raw.is_empty() {
            blockers.push(finding(
                BLOCKER_ENDPOINT_MISSING,
                ReadinessSeverity::Blocker,
                "The host has no endpoint to advertise; register one before deploying inference.",
            ));
        } else {
            match validate_endpoint_security(raw, &input.transport_policy) {
                Ok(security) => {
                    endpoint = Some(security.endpoint);
                    endpoint_class = Some(security.class);
                    transport_accepted = true;
                }
                Err(error) => {
                    if let Ok(parsed) = classify_endpoint(raw) {
                        endpoint = Some(parsed.endpoint);
                        endpoint_class = Some(parsed.class);
                    }
                    blockers.push(transport_finding(&error));
                }
            }
        }
    }

    let wants_inference =
        declares_inference || host.is_some_and(|host| host.role == HostRole::Inference);
    if wants_inference && !transport_accepted {
        blockers.push(finding(
            BLOCKER_INFERENCE_WITHOUT_ENDPOINT,
            ReadinessSeverity::Blocker,
            "Inference is advertised but no policy-accepted endpoint exists; never advertise \
             inference without a usable endpoint.",
        ));
    }
    let inference_has_endpoint = wants_inference && transport_accepted;

    AgentPreflight {
        schema: AGENT_PREFLIGHT_SCHEMA.to_owned(),
        callsign: plan.callsign.trim().to_owned(),
        host_id: host.map(|host| host.host_id.clone()),
        agent_binary_present,
        endpoint,
        endpoint_class,
        transport_accepted,
        declares_inference,
        inference_has_endpoint,
        secret_provisioned: input.secret_provisioned,
        secret_env_var,
        contains_secret_values: plan.contains_secret_values,
        ready: blockers.is_empty(),
        blockers,
        warnings,
    }
}

fn is_atom(value: &str, max: usize) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed.chars().count() <= max
        && !trimmed
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
}

fn is_path_like(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty() && trimmed.chars().count() <= 512 && !trimmed.chars().any(char::is_control)
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
            capabilities: HostCapabilities::new([HostRole::General], ["smb"])
                .expect("caps")
                .to_value()
                .expect("value"),
            enrolled: true,
            trusted: true,
            priority: 50,
            created_at: "2026-01-01T00:00:00Z".to_owned(),
            updated_at: "2026-01-01T00:00:00Z".to_owned(),
            revision: 1,
            health: None,
        }
    }

    fn inference_host() -> FleetHost {
        let mut host = host();
        host.role = HostRole::Inference;
        host.endpoint = "http://192.168.1.55:11434".to_owned();
        host.capabilities = HostCapabilities::new([HostRole::Inference], ["ollama"])
            .expect("caps")
            .to_value()
            .expect("value");
        host
    }

    fn options<'a>(binary_source: &'a str, install_dir: &'a str) -> AgentPlanOptions<'a> {
        AgentPlanOptions {
            binary_source,
            config_source: None,
            install_dir,
            bind: "0.0.0.0:8787",
            connect_address: "192.168.1.55:8787",
            secret_env_var: DEFAULT_SECRET_ENV_VAR,
            accepts: 4,
        }
    }

    fn binary_file() -> (tempfile::TempDir, std::path::PathBuf) {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("aegis-fleet-identity.exe");
        std::fs::write(&path, b"stub").expect("write");
        (directory, path)
    }

    #[test]
    fn plan_is_deterministic_and_secret_free() {
        let host = host();
        let plan = build_agent_plan(
            &host,
            &options("C:\\build\\aegis-fleet-identity.exe", "C:\\Aegis\\agent"),
        )
        .expect("plan");
        assert_eq!(plan.schema, AGENT_PLAN_SCHEMA);
        assert_eq!(plan.callsign, "Artemis");
        assert_eq!(plan.host_id, host.host_id);
        assert_eq!(plan.working_dir, "C:\\Aegis\\agent");
        assert!(!plan.contains_secret_values);
        assert_eq!(plan.files.len(), 1);
        assert!(plan.files[0]
            .destination
            .ends_with("aegis-fleet-identity.exe"));
        assert_eq!(plan.env.len(), 1);
        assert_eq!(plan.env[0].name, DEFAULT_SECRET_ENV_VAR);
        assert!(plan.env[0].secret);
        assert_eq!(
            plan.argv,
            vec![
                "serve".to_owned(),
                "--bind".to_owned(),
                "0.0.0.0:8787".to_owned(),
                "--host-uuid".to_owned(),
                host.host_id.clone(),
                "--callsign".to_owned(),
                "Artemis".to_owned(),
                "--accepts".to_owned(),
                "4".to_owned(),
            ]
        );
        assert!(plan
            .verification
            .iter()
            .any(|step| step.expected.as_deref() == Some("verified=true")));
        assert!(plan.rollback.iter().any(|step| step.name == "revoke_host"));

        let encoded = serde_json::to_string_pretty(&plan).expect("json");
        assert!(!encoded.contains(SECRET_VALUE));
        assert!(!encoded.contains("\"secret_value\""));
        assert!(encoded.contains("contains_secret_values\": false"));
    }

    #[test]
    fn plan_includes_optional_config_file() {
        let host = host();
        let mut opts = options("C:\\build\\aegis-fleet-identity.exe", "C:\\Aegis\\agent");
        opts.config_source = Some("C:\\build\\agent.toml");
        let plan = build_agent_plan(&host, &opts).expect("plan");
        assert_eq!(plan.files.len(), 2);
        assert!(plan
            .files
            .iter()
            .any(|file| file.destination.ends_with("agent.toml")));
        assert!(plan
            .rollback
            .iter()
            .any(|step| step.name == "delete_config"));
    }

    #[test]
    fn plan_rejects_invalid_inputs() {
        let base = host();
        let mut opts = options("C:\\build\\aegis-fleet-identity.exe", "C:\\Aegis\\agent");
        opts.secret_env_var = "lower-case";
        assert_eq!(
            build_agent_plan(&base, &opts)
                .err()
                .map(|error| error.to_string()),
            Some("invalid fleet input: secret_env_var".to_owned())
        );

        let mut opts = options("C:\\build\\aegis-fleet-identity.exe", "C:\\Aegis\\agent");
        opts.accepts = 0;
        assert!(build_agent_plan(&base, &opts).is_err());

        let mut opts = options("C:\\build\\aegis-fleet-identity.exe", "C:\\Aegis\\agent");
        opts.bind = "  ";
        assert!(build_agent_plan(&base, &opts).is_err());

        let mut empty_host = host();
        empty_host.callsign = "  ".to_owned();
        assert!(build_agent_plan(&empty_host, &options("b", "d")).is_err());
    }

    #[test]
    fn preflight_ready_when_everything_is_present() {
        let host = host();
        let plan = build_agent_plan(
            &host,
            &options("C:\\build\\aegis-fleet-identity.exe", "C:\\Aegis\\agent"),
        )
        .expect("plan");
        let (_directory, binary) = binary_file();
        let preflight = preflight_agent_plan(&AgentPreflightInput {
            plan: &plan,
            host: Some(&host),
            transport_policy: TransportPolicy::strict(),
            secret_provisioned: true,
            agent_binary_path: &binary,
        });
        assert!(preflight.ready, "blockers: {:?}", preflight.blockers);
        assert!(preflight.agent_binary_present);
        assert!(preflight.transport_accepted);
        assert_eq!(preflight.endpoint_class, Some(EndpointClass::PrivateLan));
        assert!(!preflight.declares_inference);
        assert!(preflight.blockers.is_empty());
    }

    #[test]
    fn preflight_flags_missing_binary_and_secret() {
        let host = host();
        let plan = build_agent_plan(
            &host,
            &options("C:\\build\\aegis-fleet-identity.exe", "C:\\Aegis\\agent"),
        )
        .expect("plan");
        let missing = std::path::Path::new("C:\\does\\not\\exist\\aegis-fleet-identity.exe");
        let preflight = preflight_agent_plan(&AgentPreflightInput {
            plan: &plan,
            host: Some(&host),
            transport_policy: TransportPolicy::strict(),
            secret_provisioned: false,
            agent_binary_path: missing,
        });
        assert!(!preflight.ready);
        assert!(!preflight.agent_binary_present);
        assert!(preflight
            .blocker_codes()
            .contains(&BLOCKER_AGENT_BINARY_MISSING));
        assert!(preflight
            .blocker_codes()
            .contains(&crate::deployment::BLOCKER_SECRET_NOT_PROVISIONED));
        assert!(preflight
            .blockers
            .iter()
            .all(|blocker| !blocker.message.contains(SECRET_VALUE)));
    }

    #[test]
    fn preflight_blocks_inference_without_endpoint() {
        let mut host = inference_host();
        host.endpoint = "  ".to_owned();
        let plan = build_agent_plan(&host, &options("bin.exe", "C:\\Aegis\\agent")).expect("plan");
        let (_directory, binary) = binary_file();
        let preflight = preflight_agent_plan(&AgentPreflightInput {
            plan: &plan,
            host: Some(&host),
            transport_policy: TransportPolicy::strict(),
            secret_provisioned: true,
            agent_binary_path: &binary,
        });
        assert!(!preflight.ready);
        assert!(preflight.declares_inference);
        assert!(preflight
            .blocker_codes()
            .contains(&BLOCKER_ENDPOINT_MISSING));
        assert!(preflight
            .blocker_codes()
            .contains(&BLOCKER_INFERENCE_WITHOUT_ENDPOINT));
        assert!(!preflight.inference_has_endpoint);
    }

    #[test]
    fn preflight_inference_with_rejected_endpoint_is_blocked() {
        let mut host = inference_host();
        host.endpoint = "http://8.8.8.8:11434".to_owned();
        let plan = build_agent_plan(&host, &options("bin.exe", "C:\\Aegis\\agent")).expect("plan");
        let (_directory, binary) = binary_file();
        let blocked = preflight_agent_plan(&AgentPreflightInput {
            plan: &plan,
            host: Some(&host),
            transport_policy: TransportPolicy::strict(),
            secret_provisioned: true,
            agent_binary_path: &binary,
        });
        assert!(!blocked.ready);
        assert!(!blocked.transport_accepted);
        assert_eq!(blocked.endpoint_class, Some(EndpointClass::Public));
        assert!(blocked
            .blocker_codes()
            .contains(&crate::deployment::BLOCKER_TRANSPORT_REJECTED));
        assert!(blocked
            .blocker_codes()
            .contains(&BLOCKER_INFERENCE_WITHOUT_ENDPOINT));

        let allowed = preflight_agent_plan(&AgentPreflightInput {
            plan: &plan,
            host: Some(&host),
            transport_policy: TransportPolicy::allow_insecure_public(),
            secret_provisioned: true,
            agent_binary_path: &binary,
        });
        assert!(allowed.ready, "blockers: {:?}", allowed.blockers);
        assert!(allowed.inference_has_endpoint);
    }

    #[test]
    fn preflight_role_capability_mismatch_warns() {
        let mut host = host();
        host.role = HostRole::Inference;
        let plan = build_agent_plan(&host, &options("bin.exe", "C:\\Aegis\\agent")).expect("plan");
        let (_directory, binary) = binary_file();
        let preflight = preflight_agent_plan(&AgentPreflightInput {
            plan: &plan,
            host: Some(&host),
            transport_policy: TransportPolicy::strict(),
            secret_provisioned: true,
            agent_binary_path: &binary,
        });
        assert!(preflight.ready, "blockers: {:?}", preflight.blockers);
        assert!(preflight
            .warnings
            .iter()
            .any(|warning| warning.code == "inference_role_without_capability"));
    }
}

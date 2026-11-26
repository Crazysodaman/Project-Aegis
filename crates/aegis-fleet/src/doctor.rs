//! Combined Fleet readiness diagnostic ("doctor") report.
//!
//! This module composes three existing views into one read-only report:
//!
//! * endpoint transport policy ([`crate::transport`]),
//! * deployment readiness ([`crate::deployment`]), and
//! * the stored host record plus its last health probe ([`crate::freshness`],
//!   [`crate::FleetHost`]).
//!
//! It optionally folds in an agent plan preflight ([`crate::agent_plan`]).
//! Findings are deduplicated by stable code, blockers are reported first, and
//! `ready` is true only when there are no blockers. The function is pure except
//! for the optional binary-presence check performed by the preflight; it never
//! touches the network and never reads a secret.

use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::agent_plan::{preflight_agent_plan, AgentPreflight, AgentPreflightInput};
use crate::deployment::{
    evaluate_deployment_readiness, finding, DeploymentInput, DeploymentReadiness, ReadinessBlocker,
    ReadinessSeverity,
};
use crate::transport::{
    classify_endpoint, validate_endpoint_security, EndpointClass, TransportPolicy,
};
use crate::FleetHost;

/// Schema identifier for a Fleet doctor report.
pub const DOCTOR_REPORT_SCHEMA: &str = "aegis.fleet.doctor_report.v1";

/// Stable warning code: the host has never been probed.
pub const WARNING_HOST_NEVER_PROBED: &str = "host_never_probed";
/// Stable warning code: the last probe failed to reach the host.
pub const WARNING_HOST_UNREACHABLE: &str = "host_unreachable";
/// Stable warning code: the last recorded probe is older than the freshness window.
pub const WARNING_HOST_STALE: &str = "host_stale";

/// Inputs for [`run_doctor`].
#[derive(Debug)]
pub struct DoctorInput<'a> {
    pub callsign: &'a str,
    pub host: Option<&'a FleetHost>,
    pub transport_policy: TransportPolicy,
    /// Boolean indicator only; a secret value is never accepted here.
    pub secret_provisioned: bool,
    pub agent_plan: Option<&'a crate::agent_plan::FleetAgentPlan>,
    pub agent_binary_path: Option<&'a Path>,
    /// Reference time (RFC 3339) used for health freshness.
    pub now: &'a str,
    pub max_age_seconds: u64,
}

/// Combined readiness report. `ready` is true only when `blockers` is empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorReport {
    pub schema: String,
    pub callsign: String,
    pub host_id: Option<String>,
    pub host_record_present: bool,
    pub enrolled: bool,
    pub trusted: bool,
    pub endpoint: Option<String>,
    pub endpoint_class: Option<EndpointClass>,
    pub transport_accepted: bool,
    pub transport_error: Option<String>,
    pub health_reachable: Option<bool>,
    pub health_stale: Option<bool>,
    pub deployment_ready: bool,
    pub agent_plan_present: bool,
    pub agent_binary_present: Option<bool>,
    pub secret_provisioned: bool,
    pub ready: bool,
    pub blockers: Vec<ReadinessBlocker>,
    pub warnings: Vec<ReadinessBlocker>,
    pub deployment: DeploymentReadiness,
    pub preflight: Option<AgentPreflight>,
}

impl DoctorReport {
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

/// Build a combined doctor report from transport, deployment, and host state.
pub fn run_doctor(input: &DoctorInput<'_>) -> DoctorReport {
    let deployment = evaluate_deployment_readiness(&DeploymentInput {
        callsign: input.callsign,
        host: input.host,
        transport_policy: input.transport_policy,
        secret_provisioned: input.secret_provisioned,
    });

    let mut blockers = Vec::new();
    let mut warnings = Vec::new();
    let mut blocker_codes = BTreeSet::new();
    let mut warning_codes = BTreeSet::new();
    merge(
        &mut blockers,
        &mut blocker_codes,
        deployment.blockers.iter().cloned(),
    );
    merge(
        &mut warnings,
        &mut warning_codes,
        deployment.warnings.iter().cloned(),
    );

    let (health_reachable, health_stale) = match input.host.and_then(|host| host.health.as_ref()) {
        Some(health) => {
            let stale = health.is_stale(input.now, input.max_age_seconds);
            if !health.reachable {
                merge(
                    &mut warnings,
                    &mut warning_codes,
                    [finding(
                        WARNING_HOST_UNREACHABLE,
                        ReadinessSeverity::Warning,
                        "The last health probe could not reach the host; re-probe before relying on it.",
                    )],
                );
            } else if stale {
                merge(
                    &mut warnings,
                    &mut warning_codes,
                    [finding(
                        WARNING_HOST_STALE,
                        ReadinessSeverity::Warning,
                        "The last health probe is older than the freshness window; re-probe before placement.",
                    )],
                );
            }
            (Some(health.reachable), Some(stale))
        }
        None => {
            if input.host.is_some() {
                merge(
                    &mut warnings,
                    &mut warning_codes,
                    [finding(
                        WARNING_HOST_NEVER_PROBED,
                        ReadinessSeverity::Warning,
                        "The host has never been probed; run FleetStore::probe before placement.",
                    )],
                );
            }
            (None, None)
        }
    };

    let preflight = input.agent_plan.map(|plan| {
        preflight_agent_plan(&AgentPreflightInput {
            plan,
            host: input.host,
            transport_policy: input.transport_policy,
            secret_provisioned: input.secret_provisioned,
            agent_binary_path: input.agent_binary_path.unwrap_or_else(|| Path::new("")),
        })
    });
    if let Some(preflight) = &preflight {
        merge(
            &mut blockers,
            &mut blocker_codes,
            preflight.blockers.iter().cloned(),
        );
        merge(
            &mut warnings,
            &mut warning_codes,
            preflight.warnings.iter().cloned(),
        );
    }

    let transport_error = input.host.and_then(|host| {
        validate_endpoint_security(&host.endpoint, &input.transport_policy)
            .err()
            .map(|error| error.to_string())
    });
    let (endpoint, endpoint_class) = input
        .host
        .and_then(|host| classify_endpoint(&host.endpoint).ok())
        .map_or(
            (deployment.endpoint.clone(), deployment.endpoint_class),
            |parsed| (Some(parsed.endpoint), Some(parsed.class)),
        );

    let ready = blockers.is_empty();
    DoctorReport {
        schema: DOCTOR_REPORT_SCHEMA.to_owned(),
        callsign: deployment.callsign.clone(),
        host_id: deployment.host_id.clone(),
        host_record_present: deployment.host_record_present,
        enrolled: deployment.enrolled,
        trusted: deployment.trusted,
        endpoint,
        endpoint_class,
        transport_accepted: deployment.transport_accepted,
        transport_error,
        health_reachable,
        health_stale,
        deployment_ready: deployment.ready,
        agent_plan_present: preflight.is_some(),
        agent_binary_present: preflight
            .as_ref()
            .map(|preflight| preflight.agent_binary_present),
        secret_provisioned: input.secret_provisioned,
        ready,
        blockers,
        warnings,
        deployment,
        preflight,
    }
}

fn merge(
    target: &mut Vec<ReadinessBlocker>,
    seen: &mut BTreeSet<String>,
    extra: impl IntoIterator<Item = ReadinessBlocker>,
) {
    for finding in extra {
        if seen.insert(finding.code.clone()) {
            target.push(finding);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_plan::{build_agent_plan, AgentPlanOptions};
    use crate::capabilities::HostCapabilities;
    use crate::deployment::DEFAULT_SECRET_ENV_VAR;
    use crate::{HostHealth, HostRole, FLEET_HOST_SCHEMA};

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

    fn healthy(mut host: FleetHost) -> FleetHost {
        host.health = Some(HostHealth {
            reachable: true,
            model_count: 1,
            latency_ms: 5,
            error: None,
            checked_at: "2026-01-01T00:04:00Z".to_owned(),
        });
        host
    }

    fn plan(host: &FleetHost) -> crate::agent_plan::FleetAgentPlan {
        build_agent_plan(
            host,
            &AgentPlanOptions {
                binary_source: "C:\\build\\aegis-fleet-identity.exe",
                config_source: None,
                install_dir: "C:\\Aegis\\agent",
                bind: "0.0.0.0:8787",
                connect_address: "192.168.1.55:8787",
                secret_env_var: DEFAULT_SECRET_ENV_VAR,
                accepts: 2,
            },
        )
        .expect("plan")
    }

    fn binary_file() -> (tempfile::TempDir, std::path::PathBuf) {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("aegis-fleet-identity.exe");
        std::fs::write(&path, b"stub").expect("write");
        (directory, path)
    }

    #[test]
    fn healthy_host_is_ready_without_plan() {
        let host = healthy(host());
        let report = run_doctor(&DoctorInput {
            callsign: "Artemis",
            host: Some(&host),
            transport_policy: TransportPolicy::strict(),
            secret_provisioned: true,
            agent_plan: None,
            agent_binary_path: None,
            now: "2026-01-01T00:05:00Z",
            max_age_seconds: 300,
        });
        assert_eq!(report.schema, DOCTOR_REPORT_SCHEMA);
        assert!(report.ready, "blockers: {:?}", report.blockers);
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
        assert_eq!(report.health_reachable, Some(true));
        assert_eq!(report.health_stale, Some(false));
        assert!(!report.agent_plan_present);
        assert_eq!(report.agent_binary_present, None);
    }

    #[test]
    fn missing_host_record_blocks_and_warns_nothing_extra() {
        let report = run_doctor(&DoctorInput {
            callsign: "Artemis",
            host: None,
            transport_policy: TransportPolicy::strict(),
            secret_provisioned: true,
            agent_plan: None,
            agent_binary_path: None,
            now: "2026-01-01T00:05:00Z",
            max_age_seconds: 300,
        });
        assert!(!report.ready);
        assert!(report
            .blocker_codes()
            .contains(&crate::deployment::BLOCKER_HOST_RECORD_MISSING));
        assert_eq!(report.health_reachable, None);
    }

    #[test]
    fn never_probed_unreachable_and_stale_are_warnings() {
        let never = host();
        let never_report = run_doctor(&DoctorInput {
            callsign: "Artemis",
            host: Some(&never),
            transport_policy: TransportPolicy::strict(),
            secret_provisioned: true,
            agent_plan: None,
            agent_binary_path: None,
            now: "2026-01-01T00:05:00Z",
            max_age_seconds: 300,
        });
        assert!(never_report.ready);
        assert!(never_report
            .warnings
            .iter()
            .any(|warning| warning.code == WARNING_HOST_NEVER_PROBED));

        let mut offline = healthy(host());
        offline.health.as_mut().expect("health").reachable = false;
        let offline_report = run_doctor(&DoctorInput {
            callsign: "Artemis",
            host: Some(&offline),
            transport_policy: TransportPolicy::strict(),
            secret_provisioned: true,
            agent_plan: None,
            agent_binary_path: None,
            now: "2026-01-01T00:05:00Z",
            max_age_seconds: 300,
        });
        assert!(offline_report
            .warnings
            .iter()
            .any(|warning| warning.code == WARNING_HOST_UNREACHABLE));

        let stale = healthy(host());
        let stale_report = run_doctor(&DoctorInput {
            callsign: "Artemis",
            host: Some(&stale),
            transport_policy: TransportPolicy::strict(),
            secret_provisioned: true,
            agent_plan: None,
            agent_binary_path: None,
            now: "2026-01-01T00:10:00Z",
            max_age_seconds: 300,
        });
        assert_eq!(stale_report.health_stale, Some(true));
        assert!(stale_report
            .warnings
            .iter()
            .any(|warning| warning.code == WARNING_HOST_STALE));
    }

    #[test]
    fn public_plain_http_is_a_transport_blocker() {
        let mut host = healthy(host());
        host.endpoint = "http://8.8.8.8:11434".to_owned();
        let report = run_doctor(&DoctorInput {
            callsign: "Artemis",
            host: Some(&host),
            transport_policy: TransportPolicy::strict(),
            secret_provisioned: true,
            agent_plan: None,
            agent_binary_path: None,
            now: "2026-01-01T00:05:00Z",
            max_age_seconds: 300,
        });
        assert!(!report.ready);
        assert!(!report.transport_accepted);
        assert_eq!(report.endpoint_class, Some(EndpointClass::Public));
        assert!(report.transport_error.is_some());
        assert!(report
            .blocker_codes()
            .contains(&crate::deployment::BLOCKER_TRANSPORT_REJECTED));
    }

    #[test]
    fn missing_secret_blocks_without_revealing_it() {
        let host = healthy(host());
        let report = run_doctor(&DoctorInput {
            callsign: "Artemis",
            host: Some(&host),
            transport_policy: TransportPolicy::strict(),
            secret_provisioned: false,
            agent_plan: None,
            agent_binary_path: None,
            now: "2026-01-01T00:05:00Z",
            max_age_seconds: 300,
        });
        assert!(!report.ready);
        assert!(report
            .blocker_codes()
            .contains(&crate::deployment::BLOCKER_SECRET_NOT_PROVISIONED));
        assert!(!report
            .blockers
            .iter()
            .any(|blocker| blocker.message.contains("staple")));
    }

    #[test]
    fn plan_preflight_findings_merge_and_deduplicate() {
        let host = healthy(host());
        let plan = plan(&host);
        let (_directory, binary) = binary_file();
        let report = run_doctor(&DoctorInput {
            callsign: "Artemis",
            host: Some(&host),
            transport_policy: TransportPolicy::strict(),
            secret_provisioned: true,
            agent_plan: Some(&plan),
            agent_binary_path: Some(&binary),
            now: "2026-01-01T00:05:00Z",
            max_age_seconds: 300,
        });
        assert!(report.ready, "blockers: {:?}", report.blockers);
        assert!(report.agent_plan_present);
        assert_eq!(report.agent_binary_present, Some(true));
        assert!(report.preflight.as_ref().expect("preflight").ready);
        // No duplicate blocker codes even though deployment and preflight overlap.
        let mut codes = report.blocker_codes();
        let unique = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), unique);

        let missing = std::path::Path::new("C:\\does\\not\\exist\\aegis-fleet-identity.exe");
        let plan_report = run_doctor(&DoctorInput {
            callsign: "Artemis",
            host: Some(&host),
            transport_policy: TransportPolicy::strict(),
            secret_provisioned: false,
            agent_plan: Some(&plan),
            agent_binary_path: Some(missing),
            now: "2026-01-01T00:05:00Z",
            max_age_seconds: 300,
        });
        assert!(!plan_report.ready);
        assert_eq!(plan_report.agent_binary_present, Some(false));
        assert!(plan_report
            .blocker_codes()
            .contains(&crate::agent_plan::BLOCKER_AGENT_BINARY_MISSING));
        assert!(plan_report
            .blocker_codes()
            .contains(&crate::deployment::BLOCKER_SECRET_NOT_PROVISIONED));
    }

    #[test]
    fn report_is_deterministic() {
        let host = healthy(host());
        let build = || {
            run_doctor(&DoctorInput {
                callsign: "Artemis",
                host: Some(&host),
                transport_policy: TransportPolicy::strict(),
                secret_provisioned: true,
                agent_plan: None,
                agent_binary_path: None,
                now: "2026-01-01T00:05:00Z",
                max_age_seconds: 300,
            })
        };
        assert_eq!(build(), build());
    }
}

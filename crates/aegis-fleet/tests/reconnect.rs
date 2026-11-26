//! Reconnect eligibility: interruption -> stale -> recovery with timestamps only.
//!
//! These tests never open a socket. Health samples are represented by their
//! `checked_at` timestamp and evaluated against an explicit `now`, so the whole
//! lifecycle is deterministic. The one store-backed test only touches the local
//! Platform SQLite database.

use aegis_fleet::{
    fleet_store, host_eligibility, FleetHost, HostEligibilityReason, HostHealth, HostRole,
    FLEET_HOST_SCHEMA,
};
use aegis_platform::PlatformService;

fn healthy(checked_at: &str, reachable: bool) -> HostHealth {
    HostHealth {
        reachable,
        model_count: 1,
        latency_ms: 5,
        error: if reachable {
            None
        } else {
            Some("connection refused".to_owned())
        },
        checked_at: checked_at.to_owned(),
    }
}

fn host(health: Option<HostHealth>) -> FleetHost {
    FleetHost {
        schema: FLEET_HOST_SCHEMA.to_owned(),
        host_id: "11111111-1111-4111-8111-111111111111".to_owned(),
        callsign: "Venus".to_owned(),
        endpoint: "http://192.168.1.71:11434".to_owned(),
        role: HostRole::Inference,
        capabilities: serde_json::Value::Null,
        enrolled: true,
        trusted: true,
        priority: 80,
        created_at: "2026-01-01T00:00:00Z".to_owned(),
        updated_at: "2026-01-01T00:00:00Z".to_owned(),
        revision: 1,
        health,
    }
}

#[test]
fn interruption_goes_stale_and_recovery_is_timestamp_driven() {
    let last_good = host(Some(healthy("2026-01-01T00:00:00Z", true)));

    let connected = host_eligibility(&last_good, "2026-01-01T00:01:00Z", 300);
    assert!(connected.is_eligible());
    assert_eq!(connected.reason, HostEligibilityReason::Eligible);
    assert_eq!(connected.last_probe_age_seconds, Some(60));

    let interrupted = host_eligibility(&last_good, "2026-01-01T00:10:00Z", 300);
    assert!(!interrupted.is_eligible());
    assert_eq!(interrupted.reason, HostEligibilityReason::Stale);
    assert!(interrupted.stale);
    assert_eq!(interrupted.last_probe_age_seconds, Some(600));

    let recovered = host_eligibility(
        &host(Some(healthy("2026-01-01T00:09:30Z", true))),
        "2026-01-01T00:10:00Z",
        300,
    );
    assert!(recovered.is_eligible());
    assert_eq!(recovered.reason, HostEligibilityReason::Eligible);
    assert!(!recovered.stale);
    assert_eq!(recovered.last_probe_age_seconds, Some(30));

    // The freshness window is inclusive: exactly at the limit is still fresh.
    assert!(host_eligibility(&last_good, "2026-01-01T00:05:00Z", 300).is_eligible());
    assert!(!host_eligibility(&last_good, "2026-01-01T00:05:01Z", 300).is_eligible());
}

#[test]
fn unreachable_probe_outranks_staleness_but_not_trust() {
    let now = "2026-01-01T00:10:00Z";

    let unreachable = host(Some(healthy("2026-01-01T00:00:00Z", false)));
    let status = host_eligibility(&unreachable, now, 300);
    assert_eq!(status.reason, HostEligibilityReason::Unreachable);
    assert!(!status.reachable);
    assert!(!status.is_eligible());

    let mut revoked = host(Some(healthy("2026-01-01T00:09:00Z", true)));
    revoked.enrolled = false;
    revoked.trusted = false;
    assert_eq!(
        host_eligibility(&revoked, now, 300).reason,
        HostEligibilityReason::NotEnrolled
    );

    let mut untrusted = host(Some(healthy("2026-01-01T00:09:00Z", true)));
    untrusted.trusted = false;
    assert_eq!(
        host_eligibility(&untrusted, now, 300).reason,
        HostEligibilityReason::NotTrusted
    );

    assert_eq!(
        host_eligibility(&host(None), now, 300).reason,
        HostEligibilityReason::NeverProbed
    );
}

#[test]
fn store_eligibility_is_offline_and_reports_never_probed() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("aegis-root");
    let initialized = PlatformService::initialize(&root).expect("platform");
    let store = fleet_store(&root).expect("store");

    let host = store
        .add_host(
            "Venus",
            "http://192.168.1.71:11434",
            HostRole::Inference,
            serde_json::Value::Null,
            80,
            &initialized.owner_token,
        )
        .expect("host");
    store
        .enroll(&host.host_id, &initialized.owner_token)
        .expect("enroll");

    let before = store
        .host_eligibility_at(&host.host_id, "2026-01-01T00:00:00Z", 300)
        .expect("eligibility");
    assert_eq!(before.reason, HostEligibilityReason::NeverProbed);
    assert!(!before.has_health);
    assert!(!before.is_eligible());

    store
        .revoke(&host.host_id, &initialized.owner_token)
        .expect("revoke");
    let revoked = store
        .host_eligibility_at(&host.host_id, "2026-01-01T00:00:00Z", 300)
        .expect("eligibility");
    assert_eq!(revoked.reason, HostEligibilityReason::NotEnrolled);
}

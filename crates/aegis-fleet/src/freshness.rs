//! Fleet host liveness, staleness, and diagnostic summaries.
//!
//! Health is recorded by explicit probes. A probe result is only meaningful for a
//! bounded window, so placement and diagnostics treat a health sample older than
//! the caller's `max_age_seconds` as stale. Timestamps use the RFC 3339 strings
//! written by [`aegis_storage::now_rfc3339`]; parsing is implemented locally with
//! `std` so this crate does not take on a new dependency.

use serde::{Deserialize, Serialize};

use crate::{FleetHost, HostHealth, HostRole};

/// Default age after which a Fleet health sample is considered stale.
pub const DEFAULT_HOST_STALE_AFTER_SECONDS: u64 = 300;
/// Schema identifier for a single host diagnostic status.
pub const HOST_STATUS_SCHEMA: &str = "aegis.fleet.host_status.v1";
/// Schema identifier for an aggregate host status summary.
pub const HOST_STATUS_SUMMARY_SCHEMA: &str = "aegis.fleet.host_status_summary.v1";
/// Schema identifier for a single host reconnect/placement eligibility record.
pub const HOST_ELIGIBILITY_SCHEMA: &str = "aegis.fleet.host_eligibility.v1";

impl HostHealth {
    /// Age of this probe relative to `now` (RFC 3339) in whole seconds.
    ///
    /// A probe timestamp in the future is clamped to zero. Returns `None` when
    /// either timestamp cannot be parsed.
    pub fn probe_age_seconds(&self, now: &str) -> Option<u64> {
        let now = rfc3339_to_unix_seconds(now)?;
        let checked = rfc3339_to_unix_seconds(&self.checked_at)?;
        Some(now.saturating_sub(checked).max(0) as u64)
    }

    /// True when the probe is no older than `max_age_seconds`.
    ///
    /// This is a temporal check only; combine it with [`HostHealth::reachable`]
    /// for placement eligibility.
    pub fn is_fresh(&self, now: &str, max_age_seconds: u64) -> bool {
        self.probe_age_seconds(now)
            .is_some_and(|age| age <= max_age_seconds)
    }

    /// Inverse of [`HostHealth::is_fresh`]; unparseable timestamps are stale.
    pub fn is_stale(&self, now: &str, max_age_seconds: u64) -> bool {
        !self.is_fresh(now, max_age_seconds)
    }
}

/// Serializable per-host diagnostic status at a point in time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostStatus {
    pub schema: String,
    pub host_id: String,
    pub callsign: String,
    pub role: HostRole,
    pub enrolled: bool,
    pub trusted: bool,
    pub has_health: bool,
    pub reachable: bool,
    /// True only when a recorded health sample is older than the limit.
    pub stale: bool,
    pub last_probe_age_seconds: Option<u64>,
    pub checked_at: Option<String>,
    pub eligible_for_placement: bool,
}

/// Why a host is or is not eligible for reconnect/placement right now.
///
/// The order of the variants is the precedence used by [`host_eligibility`]:
/// enrollment and trust are checked before health, an explicit unreachable probe
/// outranks staleness, and only an enrolled, trusted, reachable, fresh host is
/// [`HostEligibilityReason::Eligible`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostEligibilityReason {
    /// Enrolled, trusted, reachable, and within the freshness window.
    Eligible,
    /// The host record has never been enrolled.
    NotEnrolled,
    /// The host is enrolled but not trusted (never enrolled or explicitly revoked).
    NotTrusted,
    /// The host has no recorded health probe.
    NeverProbed,
    /// The most recent probe could not reach the host.
    Unreachable,
    /// The most recent reachable probe is older than the freshness window.
    Stale,
}

impl HostEligibilityReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Eligible => "eligible",
            Self::NotEnrolled => "not_enrolled",
            Self::NotTrusted => "not_trusted",
            Self::NeverProbed => "never_probed",
            Self::Unreachable => "unreachable",
            Self::Stale => "stale",
        }
    }

    /// True only for [`HostEligibilityReason::Eligible`].
    pub const fn is_eligible(self) -> bool {
        matches!(self, Self::Eligible)
    }
}

/// Additive reconnect/placement eligibility decision for one host at `now`.
///
/// Unlike [`HostStatus`], this type carries a single explicit reason, so callers
/// can distinguish a stale host from an unreachable, unenrolled, or never-probed
/// one without re-deriving the decision. Computing it is pure: no database or
/// network access is performed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostEligibility {
    pub schema: String,
    pub host_id: String,
    pub callsign: String,
    pub role: HostRole,
    pub enrolled: bool,
    pub trusted: bool,
    pub reachable: bool,
    pub has_health: bool,
    /// True only when a recorded health sample is older than the limit.
    pub stale: bool,
    pub last_probe_age_seconds: Option<u64>,
    pub checked_at: Option<String>,
    pub eligible: bool,
    pub reason: HostEligibilityReason,
}

impl HostEligibility {
    pub fn is_eligible(&self) -> bool {
        self.eligible
    }
}

/// Decide whether `host` is currently eligible for reconnect or placement.
///
/// A host is eligible only when it is enrolled, trusted, and has a reachable
/// probe no older than `max_age_seconds`. Otherwise the returned `reason`
/// states exactly why, with [`HostEligibilityReason::Stale`] reported for an
/// old-but-previously-reachable probe so an interruption is not confused with a
/// host that was never enrolled or trusted. Pure: no network, no database.
pub fn host_eligibility(host: &FleetHost, now: &str, max_age_seconds: u64) -> HostEligibility {
    let (has_health, reachable, stale, age, checked_at) = match &host.health {
        Some(health) => (
            true,
            health.reachable,
            health.is_stale(now, max_age_seconds),
            health.probe_age_seconds(now),
            Some(health.checked_at.clone()),
        ),
        None => (false, false, false, None, None),
    };

    let reason = if !host.enrolled {
        HostEligibilityReason::NotEnrolled
    } else if !host.trusted {
        HostEligibilityReason::NotTrusted
    } else if !has_health {
        HostEligibilityReason::NeverProbed
    } else if !reachable {
        HostEligibilityReason::Unreachable
    } else if stale {
        HostEligibilityReason::Stale
    } else {
        HostEligibilityReason::Eligible
    };

    HostEligibility {
        schema: HOST_ELIGIBILITY_SCHEMA.to_owned(),
        host_id: host.host_id.clone(),
        callsign: host.callsign.clone(),
        role: host.role,
        enrolled: host.enrolled,
        trusted: host.trusted,
        reachable,
        has_health,
        stale,
        last_probe_age_seconds: age,
        checked_at,
        eligible: reason.is_eligible(),
        reason,
    }
}

/// Serializable aggregate of Fleet health for diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostStatusSummary {
    pub schema: String,
    pub as_of: String,
    pub max_age_seconds: u64,
    pub total: u64,
    pub enrolled: u64,
    pub trusted: u64,
    pub healthy: u64,
    pub stale: u64,
    pub unreachable: u64,
    pub never_probed: u64,
    pub eligible: u64,
    pub hosts: Vec<HostStatus>,
}

impl HostStatusSummary {
    fn from_hosts(
        hosts: Vec<HostStatus>,
        as_of: &str,
        max_age_seconds: u64,
        counts: StatusCounts,
    ) -> Self {
        Self {
            schema: HOST_STATUS_SUMMARY_SCHEMA.to_owned(),
            as_of: as_of.to_owned(),
            max_age_seconds,
            total: hosts.len() as u64,
            enrolled: counts.enrolled,
            trusted: counts.trusted,
            healthy: counts.healthy,
            stale: counts.stale,
            unreachable: counts.unreachable,
            never_probed: counts.never_probed,
            eligible: counts.eligible,
            hosts,
        }
    }
}

struct StatusCounts {
    enrolled: u64,
    trusted: u64,
    healthy: u64,
    stale: u64,
    unreachable: u64,
    never_probed: u64,
    eligible: u64,
}

/// Compute the diagnostic status for one host at `now`.
pub fn host_status(host: &FleetHost, now: &str, max_age_seconds: u64) -> HostStatus {
    let (has_health, reachable, stale, age, checked_at) = match &host.health {
        Some(health) => (
            true,
            health.reachable,
            health.is_stale(now, max_age_seconds),
            health.probe_age_seconds(now),
            Some(health.checked_at.clone()),
        ),
        None => (false, false, false, None, None),
    };
    HostStatus {
        schema: HOST_STATUS_SCHEMA.to_owned(),
        host_id: host.host_id.clone(),
        callsign: host.callsign.clone(),
        role: host.role,
        enrolled: host.enrolled,
        trusted: host.trusted,
        has_health,
        reachable,
        stale,
        last_probe_age_seconds: age,
        checked_at,
        eligible_for_placement: host.enrolled && host.trusted && reachable && has_health && !stale,
    }
}

/// Build an aggregate summary. Pure: no database or network access.
pub fn summarize_hosts(hosts: &[FleetHost], now: &str, max_age_seconds: u64) -> HostStatusSummary {
    let statuses: Vec<HostStatus> = hosts
        .iter()
        .map(|host| host_status(host, now, max_age_seconds))
        .collect();
    let counts = StatusCounts {
        enrolled: statuses.iter().filter(|status| status.enrolled).count() as u64,
        trusted: statuses.iter().filter(|status| status.trusted).count() as u64,
        healthy: statuses
            .iter()
            .filter(|status| status.has_health && status.reachable && !status.stale)
            .count() as u64,
        stale: statuses
            .iter()
            .filter(|status| status.has_health && status.stale)
            .count() as u64,
        unreachable: statuses
            .iter()
            .filter(|status| status.has_health && !status.reachable)
            .count() as u64,
        never_probed: statuses.iter().filter(|status| !status.has_health).count() as u64,
        eligible: statuses
            .iter()
            .filter(|status| status.eligible_for_placement)
            .count() as u64,
    };
    HostStatusSummary::from_hosts(statuses, now, max_age_seconds, counts)
}

/// Parse an RFC 3339 timestamp into Unix seconds (UTC).
///
/// Accepts `Z`, `z`, or a numeric `±HH:MM` offset plus optional fractional
/// seconds. Returns `None` for malformed or out-of-range components.
fn rfc3339_to_unix_seconds(value: &str) -> Option<i64> {
    let value = value.trim();
    let (date, time) = value.split_once('T').or_else(|| value.split_once('t'))?;

    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: u32 = date_parts.next()?.parse().ok()?;
    let day: u32 = date_parts.next()?.parse().ok()?;
    if date_parts.next().is_some() || !(1..=12).contains(&month) {
        return None;
    }
    if day == 0 || day > days_in_month(year, month) {
        return None;
    }

    let (time, offset_seconds) = split_offset(time)?;
    let mut time_parts = time.split(':');
    let hour: i64 = time_parts.next()?.parse().ok()?;
    let minute: i64 = time_parts.next()?.parse().ok()?;
    let seconds_field = time_parts.next()?;
    if time_parts.next().is_some() {
        return None;
    }
    let seconds = seconds_field.split('.').next()?.parse::<i64>().ok()?;
    if !(0..=23).contains(&hour) || !(0..=59).contains(&minute) || !(0..=60).contains(&seconds) {
        return None;
    }

    let days = days_from_civil(year, month, day);
    Some(days * 86_400 + hour * 3_600 + minute * 60 + seconds - offset_seconds)
}

fn split_offset(time: &str) -> Option<(&str, i64)> {
    if time.is_empty() {
        return None;
    }
    if let Some(stripped) = time.strip_suffix('Z').or_else(|| time.strip_suffix('z')) {
        return Some((stripped, 0));
    }
    let offset_index = time
        .char_indices()
        .skip(1)
        .find(|(_, ch)| matches!(ch, '+' | '-'))
        .map(|(index, _)| index);
    match offset_index {
        Some(index) => {
            let (main, offset) = time.split_at(index);
            let sign = if offset.starts_with('-') { -1 } else { 1 };
            let (hours, minutes) = offset.get(1..)?.split_once(':')?;
            let hours: i64 = hours.parse().ok()?;
            let minutes: i64 = minutes.parse().ok()?;
            if !(0..=23).contains(&hours) || !(0..=59).contains(&minutes) {
                return None;
            }
            Some((main, sign * (hours * 3_600 + minutes * 60)))
        }
        None => Some((time, 0)),
    }
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = (i64::from(month) + 9) % 12;
    let day_of_year = (153 * month_prime + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    fn health(checked_at: &str, reachable: bool) -> HostHealth {
        HostHealth {
            reachable,
            model_count: 1,
            latency_ms: 5,
            error: None,
            checked_at: checked_at.to_owned(),
        }
    }

    fn host(health: Option<HostHealth>) -> FleetHost {
        FleetHost {
            schema: crate::FLEET_HOST_SCHEMA.to_owned(),
            host_id: "host-1".to_owned(),
            callsign: "Venus".to_owned(),
            endpoint: "http://127.0.0.1:11434".to_owned(),
            role: HostRole::Inference,
            capabilities: serde_json::Value::Null,
            enrolled: true,
            trusted: true,
            priority: 50,
            created_at: "2026-01-01T00:00:00Z".to_owned(),
            updated_at: "2026-01-01T00:00:00Z".to_owned(),
            revision: 1,
            health,
        }
    }

    #[test]
    fn parser_matches_known_offsets() {
        assert_eq!(rfc3339_to_unix_seconds("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            rfc3339_to_unix_seconds("2026-01-01T00:00:00+00:00"),
            Some(1_767_225_600)
        );
        assert_eq!(
            rfc3339_to_unix_seconds("2025-12-31T19:00:00-05:00"),
            Some(1_767_225_600)
        );
        assert_eq!(
            rfc3339_to_unix_seconds("2026-01-01T00:00:00.123456789Z"),
            Some(1_767_225_600)
        );
        assert_eq!(
            rfc3339_to_unix_seconds("2024-02-29T00:00:00Z").map(|value| value > 0),
            Some(true)
        );
    }

    #[test]
    fn parser_rejects_malformed_values() {
        for value in [
            "",
            "not-a-time",
            "2026-13-01T00:00:00Z",
            "2026-02-30T00:00:00Z",
            "2026-01-01T24:00:00Z",
            "2026-01-01T00:60:00Z",
            "2026-01-01T00:00:00+25:00",
        ] {
            assert_eq!(rfc3339_to_unix_seconds(value), None, "{value}");
        }
    }

    #[test]
    fn freshness_uses_checked_at_and_clamps_future_probes() {
        let health = health("2026-01-01T00:00:00Z", true);
        assert_eq!(health.probe_age_seconds("2026-01-01T00:05:00Z"), Some(300));
        assert!(health.is_fresh("2026-01-01T00:05:00Z", 300));
        assert!(health.is_stale("2026-01-01T00:05:01Z", 300));
        assert_eq!(
            health.probe_age_seconds("2025-12-31T23:59:00Z"),
            Some(0),
            "future probe clamps to zero"
        );
        assert!(health.is_stale("bad-time", 300));
    }

    #[test]
    fn summary_counts_status_and_eligibility() {
        let fresh = host(Some(health("2026-01-01T00:04:00Z", true)));
        let mut unfresh = host(Some(health("2025-12-31T23:59:00Z", true)));
        unfresh.host_id = "host-2".to_owned();
        unfresh.callsign = "Artemis".to_owned();
        unfresh.role = HostRole::General;
        let mut offline = host(Some(health("2026-01-01T00:04:00Z", false)));
        offline.host_id = "host-3".to_owned();
        offline.callsign = "Offline".to_owned();
        let mut never = host(None);
        never.host_id = "host-4".to_owned();
        never.callsign = "Never".to_owned();

        let summary = summarize_hosts(
            &[fresh, unfresh, offline, never],
            "2026-01-01T00:05:00Z",
            300,
        );
        assert_eq!(summary.total, 4);
        assert_eq!(summary.enrolled, 4);
        assert_eq!(summary.trusted, 4);
        assert_eq!(summary.healthy, 1);
        assert_eq!(summary.stale, 1);
        assert_eq!(summary.unreachable, 1);
        assert_eq!(summary.never_probed, 1);
        assert_eq!(summary.eligible, 1);
    }

    #[test]
    fn eligibility_reports_each_explicit_reason() {
        let now = "2026-01-01T00:05:00Z";

        let mut unenrolled = host(Some(health("2026-01-01T00:04:00Z", true)));
        unenrolled.enrolled = false;
        unenrolled.trusted = false;
        assert_eq!(
            host_eligibility(&unenrolled, now, 300).reason,
            HostEligibilityReason::NotEnrolled
        );

        let mut untrusted = host(Some(health("2026-01-01T00:04:00Z", true)));
        untrusted.trusted = false;
        assert_eq!(
            host_eligibility(&untrusted, now, 300).reason,
            HostEligibilityReason::NotTrusted
        );

        assert_eq!(
            host_eligibility(&host(None), now, 300).reason,
            HostEligibilityReason::NeverProbed
        );

        assert_eq!(
            host_eligibility(&host(Some(health("2026-01-01T00:04:00Z", false))), now, 300).reason,
            HostEligibilityReason::Unreachable
        );

        assert_eq!(
            host_eligibility(&host(Some(health("2025-12-31T23:59:00Z", true))), now, 300).reason,
            HostEligibilityReason::Stale
        );

        let eligible =
            host_eligibility(&host(Some(health("2026-01-01T00:04:00Z", true))), now, 300);
        assert_eq!(eligible.reason, HostEligibilityReason::Eligible);
        assert!(eligible.eligible);
        assert!(!eligible.stale);
        assert_eq!(HostEligibilityReason::Stale.as_str(), "stale");
        assert!(!HostEligibilityReason::Stale.is_eligible());
    }

    #[test]
    fn interruption_goes_stale_then_recovers_without_network() {
        let healthy = host(Some(health("2026-01-01T00:00:00Z", true)));

        let connected = host_eligibility(&healthy, "2026-01-01T00:01:00Z", 300);
        assert_eq!(connected.reason, HostEligibilityReason::Eligible);
        assert!(connected.is_eligible());
        assert_eq!(connected.last_probe_age_seconds, Some(60));

        let interrupted = host_eligibility(&healthy, "2026-01-01T00:10:00Z", 300);
        assert_eq!(interrupted.reason, HostEligibilityReason::Stale);
        assert!(!interrupted.eligible);
        assert!(interrupted.stale);
        assert_eq!(interrupted.last_probe_age_seconds, Some(600));
        assert_eq!(
            interrupted.checked_at.as_deref(),
            Some("2026-01-01T00:00:00Z")
        );

        let recovered = host(Some(health("2026-01-01T00:09:30Z", true)));
        let recovered = host_eligibility(&recovered, "2026-01-01T00:10:00Z", 300);
        assert_eq!(recovered.reason, HostEligibilityReason::Eligible);
        assert!(recovered.is_eligible());
        assert_eq!(recovered.last_probe_age_seconds, Some(30));
    }
}

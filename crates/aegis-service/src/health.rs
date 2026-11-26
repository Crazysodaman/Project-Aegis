//! Pure service health and diagnostics summarisation.
//!
//! The evaluator takes already-observed inputs (recorded state presence, pid
//! liveness, heartbeat age) and folds them into an [`HealthLevel`] with
//! reasons. It never reads the host, enumerates Windows services, or touches
//! IPC, so it is safe to call from tests and offline diagnostics.

use std::fmt;
use std::time::Duration;

/// Default age after which a heartbeat is considered stale.
pub const DEFAULT_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(30);

/// Overall service health, ordered from best to worst.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum HealthLevel {
    Ok,
    Warn,
    Error,
}

impl HealthLevel {
    /// Stable lower-case label.
    pub fn as_str(self) -> &'static str {
        match self {
            HealthLevel::Ok => "ok",
            HealthLevel::Warn => "warn",
            HealthLevel::Error => "error",
        }
    }
}

impl fmt::Display for HealthLevel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One explanation attached to a health evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthReason {
    pub level: HealthLevel,
    pub code: &'static str,
    pub message: String,
}

impl HealthReason {
    fn new(level: HealthLevel, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            level,
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for HealthReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "[{}] {}: {}",
            self.level, self.code, self.message
        )
    }
}

/// Snapshots of the desired and observed service state.
///
/// All fields are provided by the caller; nothing is probed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthInputs {
    pub desired_running: bool,
    pub recorded_state_present: bool,
    pub pid: Option<u32>,
    pub pid_alive: bool,
    pub heartbeat_age: Option<Duration>,
    pub heartbeat_timeout: Duration,
}

impl HealthInputs {
    /// Start from the desired state with conservative defaults.
    pub fn new(desired_running: bool) -> Self {
        Self {
            desired_running,
            recorded_state_present: false,
            pid: None,
            pid_alive: false,
            heartbeat_age: None,
            heartbeat_timeout: DEFAULT_HEARTBEAT_TIMEOUT,
        }
    }

    pub fn with_recorded_state(mut self, present: bool) -> Self {
        self.recorded_state_present = present;
        self
    }

    pub fn with_process(mut self, pid: u32, alive: bool) -> Self {
        self.pid = Some(pid);
        self.pid_alive = alive;
        self
    }

    pub fn with_heartbeat_age(mut self, age: Duration) -> Self {
        self.heartbeat_age = Some(age);
        self
    }

    pub fn with_heartbeat_timeout(mut self, timeout: Duration) -> Self {
        self.heartbeat_timeout = timeout;
        self
    }

    /// Evaluate this snapshot without side effects.
    pub fn evaluate(&self) -> ServiceHealth {
        evaluate_health(self)
    }
}

/// Summarised health result with per-input reasons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceHealth {
    pub level: HealthLevel,
    pub desired_running: bool,
    pub actual_running: bool,
    pub recorded_state_present: bool,
    pub pid: Option<u32>,
    pub heartbeat_age: Option<Duration>,
    pub reasons: Vec<HealthReason>,
}

impl ServiceHealth {
    /// True when the level is [`HealthLevel::Ok`].
    pub fn is_ok(&self) -> bool {
        self.level == HealthLevel::Ok
    }

    /// True unless a hard error was reported.
    pub fn is_healthy(&self) -> bool {
        self.level != HealthLevel::Error
    }

    /// Reason codes in evaluation order.
    pub fn codes(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.reasons.iter().map(|reason| reason.code)
    }
}

/// Fold observed inputs into a health level and reasons.
pub fn evaluate_health(inputs: &HealthInputs) -> ServiceHealth {
    let actual_running = inputs.recorded_state_present && inputs.pid.is_some() && inputs.pid_alive;
    let mut reasons = Vec::new();

    if inputs.desired_running {
        if !inputs.recorded_state_present {
            reasons.push(HealthReason::new(
                HealthLevel::Error,
                "state-missing",
                "desired running but no recorded service state exists",
            ));
        }
        match inputs.pid {
            None => reasons.push(HealthReason::new(
                HealthLevel::Error,
                "pid-missing",
                "desired running but no pid is recorded",
            )),
            Some(pid) if !inputs.pid_alive => reasons.push(HealthReason::new(
                HealthLevel::Error,
                "pid-dead",
                format!("recorded pid {pid} is not alive"),
            )),
            Some(_) => {}
        }
        match inputs.heartbeat_age {
            None => reasons.push(HealthReason::new(
                HealthLevel::Warn,
                "heartbeat-missing",
                "no heartbeat has been recorded",
            )),
            Some(age) if inputs.heartbeat_timeout.is_zero() || age > inputs.heartbeat_timeout => {
                reasons.push(HealthReason::new(
                    HealthLevel::Error,
                    "heartbeat-stale",
                    format!(
                        "heartbeat age {} ms exceeds timeout {} ms",
                        age.as_millis(),
                        inputs.heartbeat_timeout.as_millis()
                    ),
                ));
            }
            Some(_) => {}
        }
        if reasons.is_empty() {
            reasons.push(HealthReason::new(
                HealthLevel::Ok,
                "healthy",
                "service is running with a current heartbeat",
            ));
        }
    } else if inputs.recorded_state_present {
        if actual_running {
            reasons.push(HealthReason::new(
                HealthLevel::Warn,
                "unexpected-running",
                "service process is alive while the desired state is stopped",
            ));
        } else {
            reasons.push(HealthReason::new(
                HealthLevel::Warn,
                "stale-state",
                "recorded service state exists but the process is not alive",
            ));
        }
    } else {
        reasons.push(HealthReason::new(
            HealthLevel::Ok,
            "stopped",
            "service is stopped as desired",
        ));
    }

    let level = reasons
        .iter()
        .map(|reason| reason.level)
        .max()
        .unwrap_or(HealthLevel::Ok);

    ServiceHealth {
        level,
        desired_running: inputs.desired_running,
        actual_running,
        recorded_state_present: inputs.recorded_state_present,
        pid: inputs.pid,
        heartbeat_age: inputs.heartbeat_age,
        reasons,
    }
}

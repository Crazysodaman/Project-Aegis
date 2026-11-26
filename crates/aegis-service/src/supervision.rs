//! Pure restart and supervision policy modelling.
//!
//! This module contains no clocks, process handles, or OS calls. Callers feed
//! in the observed exit status, the number of prior restarts, and the elapsed
//! runtime of the attempt that just exited ("now"). The result is a plain
//! [`RestartDecision`] that a supervisor can act on.
//!
//! Note: [`RestartPolicy`] here is a supervision policy with delay, attempt
//! limits, and backoff. It is intentionally separate from the simpler
//! [`crate::registration::RestartPolicy`] enum used when rendering systemd
//! units, which is kept unchanged for backward compatibility.

use std::time::Duration;

use thiserror::Error;

/// Hard cap on configured restart attempts (0 means unlimited).
pub const MAX_RESTART_ATTEMPTS: u32 = 100;
/// Default delay between restarts.
pub const DEFAULT_RESTART_DELAY: Duration = Duration::from_secs(5);
/// Default ceiling for any computed backoff delay.
pub const DEFAULT_MAX_RESTART_DELAY: Duration = Duration::from_secs(300);
/// Default runtime after which a process is considered to have run successfully.
pub const DEFAULT_RESET_AFTER: Duration = Duration::from_secs(60);

/// When a supervisor should restart a service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartCondition {
    /// Never restart.
    Never,
    /// Restart only after a non-zero or signal exit.
    OnFailure,
    /// Restart after every exit, including a clean one.
    Always,
}

/// How the delay grows with each consecutive restart attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backoff {
    /// Constant delay.
    Fixed,
    /// Delay increases by `step` for every prior attempt.
    Linear { step: Duration },
    /// Delay is multiplied by `multiplier` for every prior attempt, capped by
    /// [`RestartPolicy::max_delay`].
    Exponential { multiplier: u32 },
}

/// Validation failures for a supervision policy.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SupervisionError {
    #[error("restart delay must be greater than zero")]
    ZeroDelay,
    #[error("maximum restart delay must be greater than zero")]
    ZeroMaxDelay,
    #[error("restart delay {delay:?} exceeds maximum delay {max_delay:?}")]
    DelayExceedsMax {
        delay: Duration,
        max_delay: Duration,
    },
    #[error("maximum restart attempts {0} exceeds the supported limit")]
    TooManyAttempts(u32),
    #[error("linear backoff step must be greater than zero")]
    ZeroBackoffStep,
    #[error("exponential backoff multiplier must be at least two")]
    InvalidBackoffMultiplier(u32),
    #[error("reset window must be greater than zero")]
    ZeroResetAfter,
}

/// Reason a supervisor decides to stop restarting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// The policy is [`RestartCondition::Never`].
    PolicyNever,
    /// The process exited successfully under [`RestartCondition::OnFailure`].
    SuccessfulExit,
}

/// Pure decision returned by [`RestartPolicy::plan_restart`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartDecision {
    /// Restart the service after `delay`, recording `attempt` as the new count.
    Restart { attempt: u32, delay: Duration },
    /// Do not restart.
    Stop {
        reason: StopReason,
        attempts_reset: bool,
    },
    /// The configured attempt budget was exhausted.
    Exhausted { attempts: u32 },
}

/// Restart and supervision policy.
///
/// `max_attempts == 0` means unlimited. `reset_after` is the runtime at or
/// above which the attempt counter is considered reset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestartPolicy {
    pub condition: RestartCondition,
    pub delay: Duration,
    pub max_attempts: u32,
    pub backoff: Backoff,
    pub max_delay: Duration,
    pub reset_after: Duration,
}

impl RestartPolicy {
    /// Build a fixed-delay policy and validate it.
    pub fn new(
        condition: RestartCondition,
        delay: Duration,
        max_attempts: u32,
    ) -> Result<Self, SupervisionError> {
        let policy = Self {
            condition,
            delay,
            max_attempts,
            backoff: Backoff::Fixed,
            max_delay: DEFAULT_MAX_RESTART_DELAY.max(delay),
            reset_after: DEFAULT_RESET_AFTER,
        };
        policy.validate()?;
        Ok(policy)
    }

    pub fn with_backoff(mut self, backoff: Backoff) -> Self {
        self.backoff = backoff;
        self
    }

    pub fn with_max_delay(mut self, max_delay: Duration) -> Self {
        self.max_delay = max_delay;
        self
    }

    pub fn with_reset_after(mut self, reset_after: Duration) -> Self {
        self.reset_after = reset_after;
        self
    }

    /// Re-check every field. Called by `plan_restart` and `delay_for_attempt`
    /// so public fields cannot bypass validation.
    pub fn validate(&self) -> Result<(), SupervisionError> {
        if self.delay.is_zero() {
            return Err(SupervisionError::ZeroDelay);
        }
        if self.max_delay.is_zero() {
            return Err(SupervisionError::ZeroMaxDelay);
        }
        if self.delay > self.max_delay {
            return Err(SupervisionError::DelayExceedsMax {
                delay: self.delay,
                max_delay: self.max_delay,
            });
        }
        if self.max_attempts > MAX_RESTART_ATTEMPTS {
            return Err(SupervisionError::TooManyAttempts(self.max_attempts));
        }
        match self.backoff {
            Backoff::Fixed => {}
            Backoff::Linear { step } if step.is_zero() => {
                return Err(SupervisionError::ZeroBackoffStep);
            }
            Backoff::Linear { .. } => {}
            Backoff::Exponential { multiplier } if multiplier < 2 => {
                return Err(SupervisionError::InvalidBackoffMultiplier(multiplier));
            }
            Backoff::Exponential { .. } => {}
        }
        if self.reset_after.is_zero() {
            return Err(SupervisionError::ZeroResetAfter);
        }
        Ok(())
    }

    /// Delay before restart attempt number `attempt` (0 for the first restart).
    pub fn delay_for_attempt(&self, attempt: u32) -> Result<Duration, SupervisionError> {
        self.validate()?;
        let base = self.delay.as_millis();
        let cap = self.max_delay.as_millis();
        let millis = match self.backoff {
            Backoff::Fixed => base,
            Backoff::Linear { step } => {
                base.saturating_add(step.as_millis().saturating_mul(u128::from(attempt)))
            }
            Backoff::Exponential { multiplier } => {
                let mut value = base;
                for _ in 0..attempt {
                    value = value.saturating_mul(u128::from(multiplier));
                    if value >= cap {
                        return Ok(self.max_delay);
                    }
                }
                value
            }
        };
        let capped = millis.min(cap);
        Ok(Duration::from_millis(
            u64::try_from(capped).unwrap_or(u64::MAX),
        ))
    }

    /// Decide whether to restart after an exit.
    ///
    /// `exit_status` is `Some(code)` for a normal exit and `None` for a signal
    /// termination. `now` is the elapsed runtime of the attempt that just
    /// exited; when it is at or above `reset_after`, or when the exit was
    /// successful, the attempt counter is reset.
    pub fn plan_restart(
        &self,
        exit_status: Option<i32>,
        attempt: u32,
        now: Duration,
    ) -> Result<RestartDecision, SupervisionError> {
        self.validate()?;

        let succeeded = matches!(exit_status, Some(0));
        let attempts_reset = succeeded || now >= self.reset_after;
        let effective_attempt = if attempts_reset { 0 } else { attempt };

        if self.condition == RestartCondition::Never {
            return Ok(RestartDecision::Stop {
                reason: StopReason::PolicyNever,
                attempts_reset,
            });
        }
        if self.condition == RestartCondition::OnFailure && succeeded {
            return Ok(RestartDecision::Stop {
                reason: StopReason::SuccessfulExit,
                attempts_reset,
            });
        }
        if self.max_attempts != 0 && effective_attempt >= self.max_attempts {
            return Ok(RestartDecision::Exhausted {
                attempts: effective_attempt,
            });
        }

        let delay = self.delay_for_attempt(effective_attempt)?;
        Ok(RestartDecision::Restart {
            attempt: effective_attempt.saturating_add(1),
            delay,
        })
    }
}

/// Free-function form of [`RestartPolicy::plan_restart`].
pub fn plan_restart(
    policy: &RestartPolicy,
    exit_status: Option<i32>,
    attempt: u32,
    now: Duration,
) -> Result<RestartDecision, SupervisionError> {
    policy.plan_restart(exit_status, attempt, now)
}

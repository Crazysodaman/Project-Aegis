//! Stateful GPU yield supervisor.
//!
//! The supervisor applies the admission policy over time with hysteresis:
//! busy samples accumulate before unloading resident models, and idle samples
//! accumulate before background admission resumes. Cooperative unloading uses
//! the model runtime's unload hook; this is not preemption of an in-flight
//! generation.

use std::time::{Duration, Instant};

use aegis_storage::now_rfc3339;
use serde::{Deserialize, Serialize};

use crate::gpu::{evaluate_admission, AdmissionDecision, GpuPolicy, GpuTelemetry};

/// Schema identifier for a supervision report.
pub const SUPERVISION_SCHEMA: &str = "aegis.fleet.gpu_supervision.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupervisorMode {
    Monitoring,
    Yielding,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupervisorAction {
    None,
    Yielded { models: Vec<String> },
    UnloadFailed { model: String, error: String },
    Resumed,
    DryRun { models: Vec<String> },
    TelemetryUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupervisionReport {
    pub schema: String,
    pub mode: SupervisorMode,
    pub action: SupervisorAction,
    pub reasons: Vec<String>,
    pub busy_samples: u32,
    pub idle_samples: u32,
    pub admit: bool,
    pub checked_at: String,
    pub telemetry: GpuTelemetry,
}

#[derive(Debug, Clone)]
pub struct SupervisorConfig {
    pub policy: GpuPolicy,
    pub busy_samples: u32,
    pub idle_samples: u32,
    pub cooldown: Duration,
    pub dry_run: bool,
}

impl Default for SupervisorConfig {
    fn default() -> Self {
        Self {
            policy: GpuPolicy::default(),
            busy_samples: 2,
            idle_samples: 2,
            cooldown: Duration::from_secs(30),
            dry_run: false,
        }
    }
}

impl SupervisorConfig {
    pub fn validate(&self) -> Result<(), String> {
        self.policy.validate()?;
        if self.busy_samples == 0 || self.busy_samples > 100 {
            return Err("busy_samples must be 1-100".to_owned());
        }
        if self.idle_samples == 0 || self.idle_samples > 100 {
            return Err("idle_samples must be 1-100".to_owned());
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct GpuSupervisor {
    config: SupervisorConfig,
    mode: SupervisorMode,
    consecutive_busy: u32,
    consecutive_idle: u32,
    last_unload: Option<Instant>,
}

impl GpuSupervisor {
    pub fn new(config: SupervisorConfig) -> Result<Self, String> {
        config.validate()?;
        Ok(Self {
            config,
            mode: SupervisorMode::Monitoring,
            consecutive_busy: 0,
            consecutive_idle: 0,
            last_unload: None,
        })
    }

    pub fn mode(&self) -> SupervisorMode {
        self.mode
    }

    /// Apply one telemetry sample. `unload` is invoked only when a yield action
    /// is required and the supervisor is not in dry-run mode.
    pub fn step(
        &mut self,
        telemetry: &GpuTelemetry,
        running_models: &[String],
        unload: &mut dyn FnMut(&str) -> Result<(), String>,
    ) -> SupervisionReport {
        let admission = evaluate_admission(&self.config.policy, telemetry);
        let mut reasons = admission.reasons.clone();
        if !telemetry.available {
            self.mode = SupervisorMode::Unknown;
            self.consecutive_busy = 0;
            self.consecutive_idle = 0;
            return self.report(
                SupervisorAction::TelemetryUnavailable,
                reasons,
                admission,
                telemetry,
            );
        }

        let busy = telemetry.utilization_percent.is_some_and(|value| {
            self.config.policy.gaming_yield && value > self.config.policy.max_utilization_percent
        });
        let mut action = SupervisorAction::None;

        if busy {
            self.consecutive_busy = self.consecutive_busy.saturating_add(1);
            self.consecutive_idle = 0;
            reasons.push("gpu_busy_sample".to_owned());
            let cooldown_elapsed = self
                .last_unload
                .map(|last| last.elapsed() >= self.config.cooldown)
                .unwrap_or(true);
            let should_attempt = match self.mode {
                SupervisorMode::Yielding => {
                    // Retry only if models remain resident and cooldown expired.
                    !running_models.is_empty() && cooldown_elapsed
                }
                _ => self.consecutive_busy >= self.config.busy_samples,
            };
            if should_attempt {
                self.mode = SupervisorMode::Yielding;
                if self.config.dry_run {
                    action = SupervisorAction::DryRun {
                        models: running_models.to_vec(),
                    };
                } else if running_models.is_empty() {
                    reasons.push("no_running_models".to_owned());
                } else {
                    let mut unloaded = Vec::new();
                    for model in running_models {
                        match unload(model) {
                            Ok(()) => unloaded.push(model.clone()),
                            Err(error) => {
                                action = SupervisorAction::UnloadFailed {
                                    model: model.clone(),
                                    error: error.chars().take(256).collect(),
                                };
                                break;
                            }
                        }
                    }
                    if matches!(action, SupervisorAction::None) {
                        self.last_unload = Some(Instant::now());
                        action = SupervisorAction::Yielded { models: unloaded };
                    }
                }
            }
        } else {
            self.consecutive_idle = self.consecutive_idle.saturating_add(1);
            self.consecutive_busy = 0;
            if self.mode == SupervisorMode::Yielding
                && self.consecutive_idle >= self.config.idle_samples
            {
                self.mode = SupervisorMode::Monitoring;
                action = SupervisorAction::Resumed;
            }
        }

        self.report(action, reasons, admission, telemetry)
    }

    fn report(
        &self,
        action: SupervisorAction,
        reasons: Vec<String>,
        admission: AdmissionDecision,
        telemetry: &GpuTelemetry,
    ) -> SupervisionReport {
        SupervisionReport {
            schema: SUPERVISION_SCHEMA.to_owned(),
            mode: self.mode,
            action,
            reasons,
            busy_samples: self.consecutive_busy,
            idle_samples: self.consecutive_idle,
            admit: admission.admit,
            checked_at: now_rfc3339().unwrap_or_default(),
            telemetry: telemetry.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::{GpuTelemetry, GPU_TELEMETRY_SCHEMA};

    fn telemetry(utilization: u8, used: u64, total: u64, temperature: u8) -> GpuTelemetry {
        GpuTelemetry {
            schema: GPU_TELEMETRY_SCHEMA.to_owned(),
            available: true,
            name: Some("Test GPU".to_owned()),
            utilization_percent: Some(utilization),
            memory_used_mb: Some(used),
            memory_total_mb: Some(total),
            temperature_c: Some(temperature),
            source: "test".to_owned(),
            checked_at: "2026-01-01T00:00:00Z".to_owned(),
            error: None,
        }
    }

    fn supervisor(busy_samples: u32, idle_samples: u32) -> GpuSupervisor {
        GpuSupervisor::new(SupervisorConfig {
            busy_samples,
            idle_samples,
            cooldown: Duration::ZERO,
            ..SupervisorConfig::default()
        })
        .expect("supervisor")
    }

    #[test]
    fn busy_hysteresis_and_unload_happen_once() {
        let mut supervisor = supervisor(2, 2);
        let unloaded = std::cell::RefCell::new(Vec::new());
        let models = vec!["m1".to_owned()];
        let mut unload = |model: &str| {
            unloaded.borrow_mut().push(model.to_owned());
            Ok(())
        };
        let first = supervisor.step(&telemetry(70, 2000, 12288, 55), &models, &mut unload);
        assert!(matches!(first.action, SupervisorAction::None));
        assert_eq!(unloaded.borrow().len(), 0);
        let second = supervisor.step(&telemetry(70, 2000, 12288, 55), &models, &mut unload);
        assert_eq!(
            second.action,
            SupervisorAction::Yielded {
                models: vec!["m1".to_owned()]
            }
        );
        assert_eq!(*unloaded.borrow(), vec!["m1".to_owned()]);
        let third = supervisor.step(&telemetry(70, 2000, 12288, 55), &[], &mut unload);
        assert!(matches!(third.action, SupervisorAction::None));
        assert_eq!(unloaded.borrow().len(), 1);
    }

    #[test]
    fn idle_hysteresis_resumes_admission() {
        let mut supervisor = supervisor(1, 2);
        let mut unload = |_model: &str| Ok(());
        supervisor.step(
            &telemetry(70, 2000, 12288, 55),
            &["m1".to_owned()],
            &mut unload,
        );
        assert_eq!(supervisor.mode(), SupervisorMode::Yielding);
        let first = supervisor.step(&telemetry(5, 2000, 12288, 55), &[], &mut unload);
        assert!(matches!(first.action, SupervisorAction::None));
        let second = supervisor.step(&telemetry(5, 2000, 12288, 55), &[], &mut unload);
        assert_eq!(second.action, SupervisorAction::Resumed);
        assert_eq!(supervisor.mode(), SupervisorMode::Monitoring);
    }

    #[test]
    fn telemetry_failure_fails_closed_without_unloading() {
        let mut supervisor = supervisor(1, 1);
        let mut called = false;
        let mut unload = |_model: &str| {
            called = true;
            Ok(())
        };
        let unavailable = GpuTelemetry::unavailable("test", "no gpu");
        let report = supervisor.step(&unavailable, &["m1".to_owned()], &mut unload);
        assert_eq!(report.action, SupervisorAction::TelemetryUnavailable);
        assert!(!report.admit);
        assert!(!called);
    }

    #[test]
    fn dry_run_does_not_unload() {
        let mut supervisor = GpuSupervisor::new(SupervisorConfig {
            busy_samples: 1,
            idle_samples: 1,
            cooldown: Duration::ZERO,
            dry_run: true,
            ..SupervisorConfig::default()
        })
        .expect("supervisor");
        let mut called = false;
        let mut unload = |_model: &str| {
            called = true;
            Ok(())
        };
        let report = supervisor.step(
            &telemetry(70, 2000, 12288, 55),
            &["m1".to_owned()],
            &mut unload,
        );
        assert_eq!(
            report.action,
            SupervisorAction::DryRun {
                models: vec!["m1".to_owned()]
            }
        );
        assert!(!called);
    }
}

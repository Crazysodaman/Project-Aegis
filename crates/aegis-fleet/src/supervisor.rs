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

use crate::gaming::GamingSignal;
use crate::gpu::{evaluate_admission_with_gaming, AdmissionDecision, GpuPolicy, GpuTelemetry};

/// Schema identifier for a supervision report.
pub const SUPERVISION_SCHEMA: &str = "aegis.fleet.gpu_supervision.v1";
/// Maximum deferred background workloads retained by the supervisor.
pub const MAX_DEFERRED_WORKLOADS: usize = 256;

/// Before/after VRAM measurement captured around a cooperative unload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceRecovery {
    pub before_free_vram_mb: Option<u64>,
    pub after_free_vram_mb: Option<u64>,
    pub reclaimed_mb: Option<u64>,
    pub meets_floor: Option<bool>,
}

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
    pub gaming_detected: bool,
    pub recovery: Option<ResourceRecovery>,
    /// Background workloads released by a resume transition in this step.
    pub resumed_workloads: Vec<String>,
    /// Background workloads still deferred after this step.
    pub deferred_pending: u32,
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
    deferred: Vec<String>,
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
            deferred: Vec::new(),
        })
    }

    pub fn mode(&self) -> SupervisorMode {
        self.mode
    }

    pub fn pending_deferred(&self) -> &[String] {
        &self.deferred
    }

    /// Queue a background workload that must wait for a safe GPU window.
    ///
    /// Duplicate ids are ignored. Returns `false` for empty/control input or
    /// when the queue is full.
    pub fn defer(&mut self, workload: &str) -> bool {
        let workload = workload.trim();
        if workload.is_empty()
            || workload.chars().count() > 128
            || workload.chars().any(char::is_control)
        {
            return false;
        }
        if self.deferred.iter().any(|existing| existing == workload) {
            return true;
        }
        if self.deferred.len() >= MAX_DEFERRED_WORKLOADS {
            return false;
        }
        self.deferred.push(workload.to_owned());
        true
    }

    /// Apply one telemetry sample. `unload` is invoked only when a yield action
    /// is required and the supervisor is not in dry-run mode.
    pub fn step(
        &mut self,
        telemetry: &GpuTelemetry,
        running_models: &[String],
        unload: &mut dyn FnMut(&str) -> Result<(), String>,
    ) -> SupervisionReport {
        self.step_full(telemetry, None, running_models, unload, None)
    }

    /// Apply one sample with an optional gaming signal and an optional re-probe
    /// used to measure VRAM recovery immediately after a cooperative unload.
    pub fn step_full(
        &mut self,
        telemetry: &GpuTelemetry,
        gaming: Option<&GamingSignal>,
        running_models: &[String],
        unload: &mut dyn FnMut(&str) -> Result<(), String>,
        mut reprobe: Option<&mut dyn FnMut() -> GpuTelemetry>,
    ) -> SupervisionReport {
        let admission = evaluate_admission_with_gaming(&self.config.policy, telemetry, gaming);
        let mut reasons = admission.reasons.clone();
        let gaming_detected = gaming.is_some_and(|signal| signal.available && signal.detected);
        if let Some(signal) = gaming {
            // Informational only: a failed gaming probe is not itself a denial,
            // because GPU telemetry is the fail-closed signal.
            if !signal.available && signal.enabled {
                reasons.push("gaming_probe_unavailable".to_owned());
            }
        }
        if !telemetry.available {
            self.mode = SupervisorMode::Unknown;
            self.consecutive_busy = 0;
            self.consecutive_idle = 0;
            return self.report(
                SupervisorAction::TelemetryUnavailable,
                reasons,
                admission,
                telemetry,
                gaming_detected,
                None,
                Vec::new(),
            );
        }

        let utilization_busy = telemetry.utilization_percent.is_some_and(|value| {
            self.config.policy.gaming_yield && value > self.config.policy.max_utilization_percent
        });
        let busy = utilization_busy || (self.config.policy.gaming_yield && gaming_detected);
        let mut action = SupervisorAction::None;
        let mut recovery = None;
        let mut resumed_workloads = Vec::new();

        if busy {
            self.consecutive_busy = self.consecutive_busy.saturating_add(1);
            self.consecutive_idle = 0;
            if utilization_busy {
                reasons.push("gpu_busy_sample".to_owned());
            }
            if gaming_detected {
                reasons.push("gaming_sample".to_owned());
            }
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
                    let before_free = telemetry.free_vram_mb();
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
                        if let Some(probe) = reprobe.as_mut() {
                            let after = probe();
                            recovery =
                                Some(recovery_from(before_free, &after, &self.config.policy));
                        }
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
                resumed_workloads = std::mem::take(&mut self.deferred);
                if let Some(probe) = reprobe.as_mut() {
                    let after = probe();
                    let before = telemetry.free_vram_mb();
                    recovery = Some(recovery_from(before, &after, &self.config.policy));
                }
            }
        }

        self.report(
            action,
            reasons,
            admission,
            telemetry,
            gaming_detected,
            recovery,
            resumed_workloads,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn report(
        &self,
        action: SupervisorAction,
        reasons: Vec<String>,
        admission: AdmissionDecision,
        telemetry: &GpuTelemetry,
        gaming_detected: bool,
        recovery: Option<ResourceRecovery>,
        resumed_workloads: Vec<String>,
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
            gaming_detected,
            recovery,
            resumed_workloads,
            deferred_pending: self.deferred.len() as u32,
            telemetry: telemetry.clone(),
        }
    }
}

fn recovery_from(
    before_free: Option<u64>,
    after: &GpuTelemetry,
    policy: &GpuPolicy,
) -> ResourceRecovery {
    let after_free = after.free_vram_mb();
    ResourceRecovery {
        before_free_vram_mb: before_free,
        after_free_vram_mb: after_free,
        reclaimed_mb: match (before_free, after_free) {
            (Some(before), Some(after)) => Some(after.saturating_sub(before)),
            _ => None,
        },
        meets_floor: after_free.map(|free| free >= u64::from(policy.min_free_vram_mb)),
    }
}

/// Schema identifier for an inference-queue decision.
pub const INFERENCE_QUEUE_SCHEMA: &str = "aegis.fleet.inference_queue.v1";
/// Maximum number of pending inference requests one queue may hold.
pub const MAX_INFERENCE_QUEUE_DEPTH: usize = 64;
/// Maximum length of an inference request identifier.
pub const MAX_INFERENCE_REQUEST_ID_LEN: usize = 128;

/// Gate controlling whether background inference may be placed now.
///
/// This is deliberately separate from [`AdmissionDecision`] so a caller can
/// make a placement decision without re-reading telemetry. A detected gaming
/// workload always wins over admission so the reported reason is stable even
/// when a busy GPU would independently deny admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlacementGate {
    /// Admission is open and no gaming workload is detected.
    Open,
    /// A gaming workload is detected; background inference must yield.
    GamingActive,
    /// GPU admission denied (busy, low VRAM, hot, or telemetry unavailable).
    AdmissionDenied,
}

impl PlacementGate {
    pub const fn is_open(self) -> bool {
        matches!(self, Self::Open)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::GamingActive => "gaming_active",
            Self::AdmissionDenied => "admission_denied",
        }
    }
}

/// Derive a [`PlacementGate`] from an admission decision and gaming signal.
///
/// An unavailable gaming probe is *not* by itself a denial: GPU telemetry is
/// the fail-closed signal, matching [`evaluate_admission_with_gaming`].
pub fn placement_gate(
    admission: &AdmissionDecision,
    gaming: Option<&GamingSignal>,
) -> PlacementGate {
    if gaming.is_some_and(|signal| signal.available && signal.detected) {
        PlacementGate::GamingActive
    } else if admission.admit {
        PlacementGate::Open
    } else {
        PlacementGate::AdmissionDenied
    }
}

/// One pending inference request. `priority` 255 is most urgent; equal
/// priorities are released in insertion order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InferenceRequest {
    pub id: String,
    pub priority: u8,
    pub sequence: u64,
}

/// Why a queue operation was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueRejection {
    InvalidId,
    QueueFull,
}

impl QueueRejection {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidId => "invalid_id",
            Self::QueueFull => "queue_full",
        }
    }
}

/// Explicit outcome of a queue operation.
///
/// Every variant states whether state changed, so callers never have to infer
/// it. Duplicate ids, a closed gate, and a full queue all leave the existing
/// ordering untouched, which is what prevents gate-driven oscillation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum QueueDecision {
    Enqueued {
        id: String,
        priority: u8,
        depth: u32,
    },
    Deferred {
        id: String,
        priority: u8,
        reason: PlacementGate,
        depth: u32,
    },
    Duplicate {
        id: String,
    },
    Dequeued {
        id: String,
        priority: u8,
        remaining: u32,
    },
    Held {
        reason: PlacementGate,
        pending: u32,
    },
    Empty,
    Rejected {
        reason: QueueRejection,
    },
}

/// Bounded, deterministic inference request queue.
///
/// The queue is pure: it performs no IO, reads no clocks, and never fabricates
/// a result. Ordering is highest `priority` first, then insertion order, so
/// repeated calls over the same inputs always produce the same dequeues.
#[derive(Debug, Clone)]
pub struct InferenceQueue {
    max_depth: usize,
    next_sequence: u64,
    ready: Vec<InferenceRequest>,
    deferred: Vec<InferenceRequest>,
}

impl Default for InferenceQueue {
    fn default() -> Self {
        Self {
            max_depth: MAX_INFERENCE_QUEUE_DEPTH,
            next_sequence: 0,
            ready: Vec::new(),
            deferred: Vec::new(),
        }
    }
}

impl InferenceQueue {
    /// Create a queue bounded to `1..=MAX_INFERENCE_QUEUE_DEPTH`.
    pub fn new(max_depth: usize) -> Result<Self, String> {
        if max_depth == 0 || max_depth > MAX_INFERENCE_QUEUE_DEPTH {
            return Err(format!("max depth must be 1..={MAX_INFERENCE_QUEUE_DEPTH}"));
        }
        Ok(Self {
            max_depth,
            ..Self::default()
        })
    }

    pub fn max_depth(&self) -> usize {
        self.max_depth
    }

    pub fn len(&self) -> usize {
        self.ready.len() + self.deferred.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ready.is_empty() && self.deferred.is_empty()
    }

    pub fn ready_len(&self) -> usize {
        self.ready.len()
    }

    pub fn deferred_len(&self) -> usize {
        self.deferred.len()
    }

    /// Add a request for an open gate.
    ///
    /// Shorthand for [`InferenceQueue::defer`] with [`PlacementGate::Open`].
    pub fn enqueue(&mut self, id: &str, priority: u8) -> QueueDecision {
        self.defer(id, priority, PlacementGate::Open)
    }

    /// Add a request when the gate is open, otherwise retain it in the bounded
    /// deferred set until a later open-gate dequeue can release it.
    ///
    /// Duplicate ids (ready or deferred) are idempotent and never reordered.
    pub fn defer(&mut self, id: &str, priority: u8, gate: PlacementGate) -> QueueDecision {
        let Some(id) = validated_request_id(id) else {
            return QueueDecision::Rejected {
                reason: QueueRejection::InvalidId,
            };
        };
        if self.contains(&id) {
            return QueueDecision::Duplicate { id };
        }
        if self.len() >= self.max_depth {
            return QueueDecision::Rejected {
                reason: QueueRejection::QueueFull,
            };
        }
        let request = InferenceRequest {
            id: id.clone(),
            priority,
            sequence: self.next_sequence,
        };
        self.next_sequence = self.next_sequence.saturating_add(1);
        let depth = queue_u32(self.len().saturating_add(1));
        if gate.is_open() {
            self.ready.push(request);
            QueueDecision::Enqueued {
                id,
                priority,
                depth,
            }
        } else {
            self.deferred.push(request);
            QueueDecision::Deferred {
                id,
                priority,
                reason: gate,
                depth,
            }
        }
    }

    /// Release the highest-priority request when the gate is open.
    ///
    /// A closed gate returns [`QueueDecision::Held`] and changes nothing, so a
    /// gaming/admission transition can never drain or reorder the queue.
    pub fn dequeue(&mut self, gate: PlacementGate) -> QueueDecision {
        if !gate.is_open() {
            return QueueDecision::Held {
                reason: gate,
                pending: queue_u32(self.len()),
            };
        }
        let Some(request) = self.take_best() else {
            return QueueDecision::Empty;
        };
        QueueDecision::Dequeued {
            id: request.id,
            priority: request.priority,
            remaining: queue_u32(self.len()),
        }
    }

    fn contains(&self, id: &str) -> bool {
        self.ready.iter().any(|request| request.id == id)
            || self.deferred.iter().any(|request| request.id == id)
    }

    fn take_best(&mut self) -> Option<InferenceRequest> {
        let ready_best = best_index(&self.ready);
        let deferred_best = best_index(&self.deferred);
        let prefer_deferred = match (ready_best, deferred_best) {
            (Some(ready), Some(deferred)) => {
                is_better(&self.deferred[deferred], &self.ready[ready])
            }
            (None, Some(_)) => true,
            _ => false,
        };
        if prefer_deferred {
            deferred_best.map(|index| self.deferred.remove(index))
        } else {
            ready_best.map(|index| self.ready.remove(index))
        }
    }
}

fn best_index(requests: &[InferenceRequest]) -> Option<usize> {
    requests
        .iter()
        .enumerate()
        .min_by(|(_, left), (_, right)| {
            right
                .priority
                .cmp(&left.priority)
                .then_with(|| left.sequence.cmp(&right.sequence))
        })
        .map(|(index, _)| index)
}

fn is_better(candidate: &InferenceRequest, current: &InferenceRequest) -> bool {
    candidate.priority > current.priority
        || (candidate.priority == current.priority && candidate.sequence < current.sequence)
}

fn validated_request_id(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > MAX_INFERENCE_REQUEST_ID_LEN
        || trimmed.chars().any(char::is_control)
    {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn queue_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
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

    fn gaming(detected: bool) -> crate::gaming::GamingSignal {
        if detected {
            crate::gaming::GamingSignal::detected("test", vec!["gears5".to_owned()])
        } else {
            crate::gaming::GamingSignal::none("test")
        }
    }

    #[test]
    fn gaming_signal_forces_yield_at_low_utilization() {
        let mut supervisor = supervisor(1, 1);
        let mut unload = |_model: &str| Ok(());
        let report = supervisor.step_full(
            &telemetry(3, 2000, 12288, 55),
            Some(&gaming(true)),
            &["m1".to_owned()],
            &mut unload,
            None,
        );
        assert!(report.gaming_detected);
        assert!(!report.admit);
        assert!(report.reasons.contains(&"gaming_detected".to_owned()));
        assert_eq!(
            report.action,
            SupervisorAction::Yielded {
                models: vec!["m1".to_owned()]
            }
        );
    }

    #[test]
    fn gaming_probe_unavailable_is_not_reported_as_detected() {
        let signal = crate::gaming::GamingSignal::unavailable("test", "no process list");
        let mut supervisor = supervisor(1, 1);
        let mut unload = |_model: &str| Ok(());
        let report = supervisor.step_full(
            &telemetry(3, 2000, 12288, 55),
            Some(&signal),
            &["m1".to_owned()],
            &mut unload,
            None,
        );
        assert!(!report.gaming_detected);
        assert!(report.admit);
        assert!(report
            .reasons
            .contains(&"gaming_probe_unavailable".to_owned()));
    }

    #[test]
    fn unload_failure_is_reported_and_yield_is_not_claimed() {
        let mut supervisor = supervisor(1, 1);
        let mut unload = |_model: &str| Err("model busy".to_owned());
        let report = supervisor.step(
            &telemetry(80, 2000, 12288, 55),
            &["m1".to_owned()],
            &mut unload,
        );
        assert_eq!(
            report.action,
            SupervisorAction::UnloadFailed {
                model: "m1".to_owned(),
                error: "model busy".to_owned()
            }
        );
        assert_eq!(supervisor.mode(), SupervisorMode::Yielding);
    }

    #[test]
    fn deferred_workloads_release_on_resume() {
        let mut supervisor = supervisor(1, 1);
        assert!(supervisor.defer("embed-batch-1"));
        assert!(supervisor.defer("embed-batch-2"));
        assert!(supervisor.defer("embed-batch-1"), "duplicate is idempotent");
        assert_eq!(supervisor.pending_deferred().len(), 2);

        let mut unload = |_model: &str| Ok(());
        supervisor.step(
            &telemetry(80, 2000, 12288, 55),
            &["m1".to_owned()],
            &mut unload,
        );
        let resumed = supervisor.step(&telemetry(2, 2000, 12288, 55), &[], &mut unload);
        assert_eq!(resumed.action, SupervisorAction::Resumed);
        assert_eq!(
            resumed.resumed_workloads,
            vec!["embed-batch-1".to_owned(), "embed-batch-2".to_owned()]
        );
        assert_eq!(resumed.deferred_pending, 0);
        assert!(supervisor.pending_deferred().is_empty());
    }

    #[test]
    fn recovery_measures_reclaimed_vram() {
        let mut supervisor = supervisor(1, 1);
        let mut unload = |_model: &str| Ok(());
        let mut after = telemetry(4, 100, 12288, 50);
        let mut reprobe = || {
            after.checked_at = "2026-01-01T00:00:01Z".to_owned();
            after.clone()
        };
        // Before: 1000 used of 12288 => 11288 free. After unload: 100 used => 12188 free.
        let report = supervisor.step_full(
            &telemetry(80, 1000, 12288, 55),
            None,
            &["m1".to_owned()],
            &mut unload,
            Some(&mut reprobe),
        );
        let recovery = report.recovery.expect("recovery");
        assert_eq!(recovery.before_free_vram_mb, Some(11288));
        assert_eq!(recovery.after_free_vram_mb, Some(12188));
        assert_eq!(recovery.reclaimed_mb, Some(900));
        assert_eq!(recovery.meets_floor, Some(true));
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

    fn admitted(utilization: u8, used: u64, total: u64, temperature: u8) -> AdmissionDecision {
        evaluate_admission_with_gaming(
            &GpuPolicy::default(),
            &telemetry(utilization, used, total, temperature),
            None,
        )
    }

    #[test]
    fn placement_gate_is_gaming_first_then_admission() {
        let idle = admitted(3, 2000, 12288, 50);
        assert_eq!(placement_gate(&idle, None), PlacementGate::Open);
        assert_eq!(
            placement_gate(&idle, Some(&gaming(false))),
            PlacementGate::Open
        );

        let busy = admitted(90, 2000, 12288, 50);
        assert!(!busy.admit);
        assert_eq!(placement_gate(&busy, None), PlacementGate::AdmissionDenied);
        assert_eq!(
            placement_gate(&busy, Some(&gaming(true))),
            PlacementGate::GamingActive
        );
        assert_eq!(
            placement_gate(&idle, Some(&gaming(true))),
            PlacementGate::GamingActive
        );

        let unavailable_gaming = crate::gaming::GamingSignal::unavailable("test", "no list");
        assert_eq!(
            placement_gate(&idle, Some(&unavailable_gaming)),
            PlacementGate::Open,
            "an unavailable gaming probe does not deny admission on its own"
        );
        assert_eq!(
            placement_gate(&busy, Some(&unavailable_gaming)),
            PlacementGate::AdmissionDenied
        );
    }

    #[test]
    fn queue_releases_by_priority_then_insertion_order() {
        let mut queue = InferenceQueue::new(8).expect("queue");
        assert_eq!(queue.len(), 0);
        assert!(queue.is_empty());
        assert_eq!(
            queue.enqueue("low", 1),
            QueueDecision::Enqueued {
                id: "low".to_owned(),
                priority: 1,
                depth: 1,
            }
        );
        assert_eq!(
            queue.enqueue("high", 9),
            QueueDecision::Enqueued {
                id: "high".to_owned(),
                priority: 9,
                depth: 2,
            }
        );
        assert_eq!(
            queue.enqueue("mid-a", 5),
            QueueDecision::Enqueued {
                id: "mid-a".to_owned(),
                priority: 5,
                depth: 3,
            }
        );
        assert_eq!(
            queue.enqueue("mid-b", 5),
            QueueDecision::Enqueued {
                id: "mid-b".to_owned(),
                priority: 5,
                depth: 4,
            }
        );
        assert_eq!(
            queue.enqueue("high", 9),
            QueueDecision::Duplicate {
                id: "high".to_owned()
            },
            "duplicate ids are idempotent"
        );
        assert_eq!(queue.len(), 4);

        let order: Vec<String> = (0..4)
            .map(|_| match queue.dequeue(PlacementGate::Open) {
                QueueDecision::Dequeued { id, .. } => id,
                other => panic!("unexpected decision: {other:?}"),
            })
            .collect();
        assert_eq!(order, vec!["high", "mid-a", "mid-b", "low"]);
        assert_eq!(queue.dequeue(PlacementGate::Open), QueueDecision::Empty);
    }

    #[test]
    fn queue_defers_while_gate_closed_without_reordering() {
        let mut queue = InferenceQueue::new(4).expect("queue");
        assert_eq!(
            queue.defer("game-blocked", 5, PlacementGate::GamingActive),
            QueueDecision::Deferred {
                id: "game-blocked".to_owned(),
                priority: 5,
                reason: PlacementGate::GamingActive,
                depth: 1,
            }
        );
        assert_eq!(queue.deferred_len(), 1);
        assert_eq!(
            queue.dequeue(PlacementGate::GamingActive),
            QueueDecision::Held {
                reason: PlacementGate::GamingActive,
                pending: 1,
            },
            "a closed gate never drains or reorders the queue"
        );
        assert_eq!(queue.len(), 1);

        assert_eq!(
            queue.defer("busy-blocked", 9, PlacementGate::AdmissionDenied),
            QueueDecision::Deferred {
                id: "busy-blocked".to_owned(),
                priority: 9,
                reason: PlacementGate::AdmissionDenied,
                depth: 2,
            }
        );
        assert_eq!(queue.deferred_len(), 2);

        assert_eq!(
            queue.dequeue(PlacementGate::Open),
            QueueDecision::Dequeued {
                id: "busy-blocked".to_owned(),
                priority: 9,
                remaining: 1,
            }
        );
        assert_eq!(
            queue.dequeue(PlacementGate::Open),
            QueueDecision::Dequeued {
                id: "game-blocked".to_owned(),
                priority: 5,
                remaining: 0,
            }
        );
        assert_eq!(queue.dequeue(PlacementGate::Open), QueueDecision::Empty);
    }

    #[test]
    fn queue_gate_toggles_do_not_oscillate() {
        let mut queue = InferenceQueue::new(4).expect("queue");
        queue.enqueue("a", 1);
        queue.enqueue("b", 2);
        for _ in 0..3 {
            assert_eq!(
                queue.dequeue(PlacementGate::AdmissionDenied),
                QueueDecision::Held {
                    reason: PlacementGate::AdmissionDenied,
                    pending: 2,
                }
            );
        }
        assert_eq!(queue.len(), 2);
        assert_eq!(
            queue.dequeue(PlacementGate::Open),
            QueueDecision::Dequeued {
                id: "b".to_owned(),
                priority: 2,
                remaining: 1,
            }
        );
        // Re-closing the gate must not resurrect or duplicate the released item.
        assert!(matches!(
            queue.dequeue(PlacementGate::GamingActive),
            QueueDecision::Held { .. }
        ));
        assert_eq!(
            queue.dequeue(PlacementGate::Open),
            QueueDecision::Dequeued {
                id: "a".to_owned(),
                priority: 1,
                remaining: 0,
            }
        );
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn queue_is_bounded_and_rejects_invalid_ids() {
        let mut queue = InferenceQueue::new(2).expect("queue");
        assert!(matches!(
            queue.enqueue("one", 0),
            QueueDecision::Enqueued { .. }
        ));
        assert!(matches!(
            queue.enqueue("two", 0),
            QueueDecision::Enqueued { .. }
        ));
        assert_eq!(
            queue.enqueue("three", 0),
            QueueDecision::Rejected {
                reason: QueueRejection::QueueFull
            }
        );
        assert_eq!(queue.len(), 2);

        for invalid in [
            "",
            "   ",
            "bad\nid",
            &"x".repeat(MAX_INFERENCE_REQUEST_ID_LEN + 1),
        ] {
            assert_eq!(
                queue.defer(invalid, 0, PlacementGate::Open),
                QueueDecision::Rejected {
                    reason: QueueRejection::InvalidId
                }
            );
        }
        assert_eq!(queue.len(), 2);

        assert!(InferenceQueue::new(0).is_err());
        assert!(InferenceQueue::new(MAX_INFERENCE_QUEUE_DEPTH + 1).is_err());
        assert_eq!(
            InferenceQueue::default().max_depth(),
            MAX_INFERENCE_QUEUE_DEPTH
        );
        assert_eq!(INFERENCE_QUEUE_SCHEMA, "aegis.fleet.inference_queue.v1");
    }
}

//! Multi-condition measurement comparison and live telemetry capture helpers.
//!
//! Everything here is pure or caller-driven. Nothing launches a game, loads a
//! model, changes the GPU policy, or talks to the GPU on its own. A
//! [`GpuProbe`] is supplied by the caller, and the caller supplies the wait
//! strategy, so tests can capture synthetic series without sleeping.
//!
//! Missing inputs are marked [`DeltaStatus::BaselineMissing`],
//! [`DeltaStatus::BaselineUnavailable`], or
//! [`DeltaStatus::ConditionUnavailable`]. No metric is invented and no frame
//! time is fabricated. Reports are deterministic: fixed metric order and no
//! hash maps or locale-dependent formatting.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::gpu::{GpuProbe, GpuTelemetry};
use crate::performance::{FrameTimeReport, PerformanceSummary, TelemetrySeries};

/// Schema identifier for a multi-condition comparison report.
pub const COMPARISON_REPORT_SCHEMA: &str = "aegis.fleet.comparison_report.v1";
/// Schema identifier for a single inference-pressure episode report.
pub const PRESSURE_REPORT_SCHEMA: &str = "aegis.fleet.inference_pressure_report.v1";
/// Maximum number of samples a single live capture may request.
pub const MAX_CAPTURE_SAMPLES: u32 = 3600;
/// Maximum interval between capture samples.
pub const MAX_CAPTURE_INTERVAL_MS: u64 = 3_600_000;

/// Physical meaning of a compared metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConditionLabel {
    Idle,
    Inference,
    Gaming,
}

impl ConditionLabel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Inference => "inference",
            Self::Gaming => "gaming",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "idle" => Some(Self::Idle),
            "inference" => Some(Self::Inference),
            "gaming" => Some(Self::Gaming),
            _ => None,
        }
    }
}

/// A telemetry series plus its optional frame-time capture, labelled by
/// condition. The caller decides what each labelled series actually represents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LabeledMeasurement {
    pub label: ConditionLabel,
    pub series: TelemetrySeries,
    pub frame_times: Option<FrameTimeReport>,
}

impl LabeledMeasurement {
    pub fn new(label: ConditionLabel, series: TelemetrySeries) -> Self {
        Self {
            label,
            series,
            frame_times: None,
        }
    }

    pub fn with_frame_times(mut self, frame_times: FrameTimeReport) -> Self {
        self.frame_times = Some(frame_times);
        self
    }
}

/// Metrics compared between the baseline condition and every other condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricKind {
    VramUsedAvgMb,
    VramFreeAvgMb,
    UtilizationAvgPercent,
    UtilizationMaxPercent,
    TemperatureMaxC,
    AvgFps,
    OnePercentLowFps,
    PointOnePercentLowFps,
    MaxFrameTimeMs,
    P95FrameTimeMs,
    P99FrameTimeMs,
}

impl MetricKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::VramUsedAvgMb => "vram_used_avg_mb",
            Self::VramFreeAvgMb => "vram_free_avg_mb",
            Self::UtilizationAvgPercent => "utilization_avg_percent",
            Self::UtilizationMaxPercent => "utilization_max_percent",
            Self::TemperatureMaxC => "temperature_max_c",
            Self::AvgFps => "avg_fps",
            Self::OnePercentLowFps => "one_percent_low_fps",
            Self::PointOnePercentLowFps => "point_one_percent_low_fps",
            Self::MaxFrameTimeMs => "max_frame_time_ms",
            Self::P95FrameTimeMs => "p95_frame_time_ms",
            Self::P99FrameTimeMs => "p99_frame_time_ms",
        }
    }
}

/// Why a delta is or is not available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeltaStatus {
    Available,
    BaselineMissing,
    BaselineUnavailable,
    ConditionUnavailable,
}

impl DeltaStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::BaselineMissing => "baseline_missing",
            Self::BaselineUnavailable => "baseline_unavailable",
            Self::ConditionUnavailable => "condition_unavailable",
        }
    }
}

/// One metric compared as `condition - baseline`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricDelta {
    pub metric: MetricKind,
    pub baseline: Option<f64>,
    pub condition: Option<f64>,
    pub absolute_delta: Option<f64>,
    pub percent_delta: Option<f64>,
    pub status: DeltaStatus,
}

/// Per-series summary, optional frame-time report, and deltas versus baseline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConditionComparison {
    pub label: ConditionLabel,
    pub summary: PerformanceSummary,
    pub frame_times: Option<FrameTimeReport>,
    pub deltas: Vec<MetricDelta>,
}

/// Deterministic comparison of labelled measurement conditions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComparisonReport {
    pub schema: String,
    pub baseline: ConditionLabel,
    pub baseline_present: bool,
    pub conditions: Vec<ConditionComparison>,
}

/// Compare every supplied series against the labelled baseline.
///
/// The baseline series is included in `conditions` with zero deltas when
/// present. If no series carries the baseline label, every delta is reported as
/// [`DeltaStatus::BaselineMissing`]. This function is pure and deterministic.
pub fn compare_conditions(
    baseline: ConditionLabel,
    measurements: &[LabeledMeasurement],
) -> ComparisonReport {
    let baseline_values = measurements
        .iter()
        .find(|measurement| measurement.label == baseline)
        .map(|measurement| {
            metric_values(
                &measurement.series.summary(),
                measurement.frame_times.as_ref(),
            )
        });
    let baseline_present = baseline_values.is_some();

    let conditions = measurements
        .iter()
        .map(|measurement| {
            let summary = measurement.series.summary();
            let frame_times = measurement.frame_times.clone();
            let condition_values = metric_values(&summary, frame_times.as_ref());
            let deltas = match &baseline_values {
                Some(baseline_values) => deltas_from(baseline_values, &condition_values, true),
                None => deltas_from(&condition_values, &condition_values, false),
            };
            ConditionComparison {
                label: measurement.label,
                summary,
                frame_times,
                deltas,
            }
        })
        .collect();

    ComparisonReport {
        schema: COMPARISON_REPORT_SCHEMA.to_owned(),
        baseline,
        baseline_present,
        conditions,
    }
}

/// Fixed-order metric extraction. Every field is optional so missing data stays
/// explicitly unavailable.
fn metric_values(
    summary: &PerformanceSummary,
    frame_times: Option<&FrameTimeReport>,
) -> Vec<(MetricKind, Option<f64>)> {
    let frame = frame_times.filter(|report| report.available);
    vec![
        (MetricKind::VramUsedAvgMb, summary.used_vram_avg_mb),
        (MetricKind::VramFreeAvgMb, summary.free_vram_avg_mb),
        (
            MetricKind::UtilizationAvgPercent,
            summary.utilization_avg_percent,
        ),
        (
            MetricKind::UtilizationMaxPercent,
            summary.utilization_max_percent.map(f64::from),
        ),
        (
            MetricKind::TemperatureMaxC,
            summary.temperature_max_c.map(f64::from),
        ),
        (MetricKind::AvgFps, frame.and_then(|report| report.avg_fps)),
        (
            MetricKind::OnePercentLowFps,
            frame.and_then(|report| report.one_percent_low_fps),
        ),
        (
            MetricKind::PointOnePercentLowFps,
            frame.and_then(|report| report.point_one_percent_low_fps),
        ),
        (
            MetricKind::MaxFrameTimeMs,
            frame.and_then(|report| report.max_frame_time_ms),
        ),
        (
            MetricKind::P95FrameTimeMs,
            frame.and_then(|report| report.p95_frame_time_ms),
        ),
        (
            MetricKind::P99FrameTimeMs,
            frame.and_then(|report| report.p99_frame_time_ms),
        ),
    ]
}

fn deltas_from(
    baseline_values: &[(MetricKind, Option<f64>)],
    condition_values: &[(MetricKind, Option<f64>)],
    baseline_present: bool,
) -> Vec<MetricDelta> {
    baseline_values
        .iter()
        .zip(condition_values.iter())
        .map(|((metric, baseline), (_, condition))| {
            let (absolute_delta, percent_delta, status) =
                match (baseline_present, baseline, condition) {
                    (false, _, _) => (None, None, DeltaStatus::BaselineMissing),
                    (true, _, None) => (None, None, DeltaStatus::ConditionUnavailable),
                    (true, None, Some(_)) => (None, None, DeltaStatus::BaselineUnavailable),
                    (true, Some(baseline), Some(condition)) => {
                        let delta = condition - baseline;
                        let percent = if *baseline == 0.0 {
                            None
                        } else {
                            Some(delta / baseline * 100.0)
                        };
                        (Some(delta), percent, DeltaStatus::Available)
                    }
                };
            MetricDelta {
                metric: *metric,
                baseline: *baseline,
                condition: *condition,
                absolute_delta,
                percent_delta,
                status,
            }
        })
        .collect()
}

/// Render a deterministic, human-readable comparison. Missing inputs are shown
/// with their explicit status rather than omitted silently.
pub fn render_comparison_report(report: &ComparisonReport) -> String {
    let mut output = String::new();
    output.push_str("Aegis multi-condition comparison (measurement notes, not ground truth)\n");
    output.push_str(&format!("baseline: {}\n", report.baseline.as_str()));
    if !report.baseline_present {
        output.push_str("baseline status: unavailable (no labelled baseline series supplied)\n");
    }
    for condition in &report.conditions {
        output.push_str(&format!(
            "\n[{}] samples total={} available={}\n",
            condition.label.as_str(),
            condition.summary.sample_count,
            condition.summary.available_samples
        ));
        output.push_str(&format!(
            "  utilization %: avg={} max={}\n",
            opt_f64(condition.summary.utilization_avg_percent),
            opt_u8(condition.summary.utilization_max_percent)
        ));
        output.push_str(&format!(
            "  VRAM MB: used avg={} free avg={}\n",
            opt_f64(condition.summary.used_vram_avg_mb),
            opt_f64(condition.summary.free_vram_avg_mb)
        ));
        output.push_str(&format!(
            "  temperature C: max={}\n",
            opt_u8(condition.summary.temperature_max_c)
        ));
        match &condition.frame_times {
            Some(frame) if frame.available => output.push_str(&format!(
                "  frame data: avg_fps={} 1%low={} 0.1%low={} p95_ms={} p99_ms={} max_ms={}\n",
                opt_f64(frame.avg_fps),
                opt_f64(frame.one_percent_low_fps),
                opt_f64(frame.point_one_percent_low_fps),
                opt_f64(frame.p95_frame_time_ms),
                opt_f64(frame.p99_frame_time_ms),
                opt_f64(frame.max_frame_time_ms)
            )),
            Some(frame) => output.push_str(&format!(
                "  frame data: unavailable ({})\n",
                frame.error.as_deref().unwrap_or("no reason recorded")
            )),
            None => output.push_str("  frame data: unavailable (not supplied)\n"),
        }
        for delta in &condition.deltas {
            let name = delta.metric.as_str();
            match delta.status {
                DeltaStatus::Available => output.push_str(&format!(
                    "  delta {name}: {:+.2} ({:+.2}%) vs baseline\n",
                    delta.absolute_delta.unwrap_or_default(),
                    delta.percent_delta.unwrap_or_default()
                )),
                status => output.push_str(&format!(
                    "  delta {name}: unavailable ({})\n",
                    status.as_str()
                )),
            }
        }
    }
    output
}

/// One labeled phase of an inference-pressure episode.
///
/// `label` is a caller-supplied, human-readable tag (for example `idle`,
/// `inference_during`, `inference_after`, `unloaded`). It is never interpreted
/// as safety evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PressurePhase {
    pub label: String,
    pub sample_count: u32,
    pub available_samples: u32,
    pub used_vram_min_mb: Option<u64>,
    pub used_vram_max_mb: Option<u64>,
    pub used_vram_avg_mb: Option<f64>,
    pub free_vram_avg_mb: Option<f64>,
    pub utilization_avg_percent: Option<f64>,
    pub utilization_max_percent: Option<u8>,
    pub temperature_max_c: Option<u8>,
}

impl PressurePhase {
    fn from_series(label: &str, series: &TelemetrySeries) -> Self {
        let summary = series.summary();
        Self {
            label: label.to_owned(),
            sample_count: summary.sample_count,
            available_samples: summary.available_samples,
            used_vram_min_mb: summary.used_vram_min_mb,
            used_vram_max_mb: summary.used_vram_max_mb,
            used_vram_avg_mb: summary.used_vram_avg_mb,
            free_vram_avg_mb: summary.free_vram_avg_mb,
            utilization_avg_percent: summary.utilization_avg_percent,
            utilization_max_percent: summary.utilization_max_percent,
            temperature_max_c: summary.temperature_max_c,
        }
    }
}

/// Deterministic comparison of four fixed phases around a real (non-gaming)
/// inference episode.
///
/// Deltas are computed from per-phase averages and stay [`None`] when either
/// operand is unavailable. They are measurement notes, not proof that a game is
/// safe to run alongside background inference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PressureReport {
    pub schema: String,
    pub idle: PressurePhase,
    pub inference_during: PressurePhase,
    pub inference_after: PressurePhase,
    pub unloaded: PressurePhase,
    /// Average used VRAM growth while inference ran (`during - idle`).
    pub inference_vram_growth_mb: Option<f64>,
    /// Average used VRAM retained after generation while the model stayed
    /// resident (`after - idle`).
    pub retained_vram_mb: Option<f64>,
    /// Average used VRAM freed by unloading the model (`after - unloaded`).
    pub recovered_vram_mb: Option<f64>,
    /// Recovered VRAM as a fraction of retained VRAM, when retention is positive.
    pub recovered_fraction: Option<f64>,
    /// Average utilization growth while inference ran (`during - idle`).
    pub utilization_growth_percent: Option<f64>,
}

/// Summarise an idle → during-inference → after-generation → unloaded episode.
///
/// Pure and deterministic: it performs no capture, starts no model, and never
/// fabricates a value. Callers supply all four series from real snapshots.
pub fn summarize_pressure(
    idle: &TelemetrySeries,
    inference_during: &TelemetrySeries,
    inference_after: &TelemetrySeries,
    unloaded: &TelemetrySeries,
) -> PressureReport {
    let idle_phase = PressurePhase::from_series("idle", idle);
    let during_phase = PressurePhase::from_series("inference_during", inference_during);
    let after_phase = PressurePhase::from_series("inference_after", inference_after);
    let unloaded_phase = PressurePhase::from_series("unloaded", unloaded);

    let inference_vram_growth_mb =
        difference(during_phase.used_vram_avg_mb, idle_phase.used_vram_avg_mb);
    let retained_vram_mb = difference(after_phase.used_vram_avg_mb, idle_phase.used_vram_avg_mb);
    let recovered_vram_mb = difference(
        after_phase.used_vram_avg_mb,
        unloaded_phase.used_vram_avg_mb,
    );
    let recovered_fraction = match (recovered_vram_mb, retained_vram_mb) {
        (Some(recovered), Some(retained)) if retained > 0.0 => Some(recovered / retained),
        _ => None,
    };
    let utilization_growth_percent = difference(
        during_phase.utilization_avg_percent,
        idle_phase.utilization_avg_percent,
    );

    PressureReport {
        schema: PRESSURE_REPORT_SCHEMA.to_owned(),
        idle: idle_phase,
        inference_during: during_phase,
        inference_after: after_phase,
        unloaded: unloaded_phase,
        inference_vram_growth_mb,
        retained_vram_mb,
        recovered_vram_mb,
        recovered_fraction,
        utilization_growth_percent,
    }
}

/// Render a deterministic inference-pressure report. Unavailable values are
/// stated explicitly rather than omitted.
pub fn render_pressure_report(report: &PressureReport) -> String {
    let mut output = String::new();
    output.push_str(
        "Aegis inference-pressure episode (measurement notes, not gameplay, not ground truth)\n",
    );
    for phase in [
        &report.idle,
        &report.inference_during,
        &report.inference_after,
        &report.unloaded,
    ] {
        output.push_str(&format!(
            "\n[{}] samples total={} available={}\n",
            phase.label, phase.sample_count, phase.available_samples
        ));
        output.push_str(&format!(
            "  utilization %: avg={} max={}\n",
            opt_f64(phase.utilization_avg_percent),
            opt_u8(phase.utilization_max_percent)
        ));
        output.push_str(&format!(
            "  VRAM MB: used avg={} min={} max={} free avg={}\n",
            opt_f64(phase.used_vram_avg_mb),
            opt_u64(phase.used_vram_min_mb),
            opt_u64(phase.used_vram_max_mb),
            opt_f64(phase.free_vram_avg_mb)
        ));
        output.push_str(&format!(
            "  temperature C: max={}\n",
            opt_u8(phase.temperature_max_c)
        ));
    }
    output.push_str(&format!(
        "\ninference VRAM growth: {} MB ({})\n",
        opt_f64(report.inference_vram_growth_mb),
        delta_note(report.inference_vram_growth_mb)
    ));
    output.push_str(&format!(
        "retained VRAM while resident: {} MB ({})\n",
        opt_f64(report.retained_vram_mb),
        delta_note(report.retained_vram_mb)
    ));
    output.push_str(&format!(
        "recovered VRAM after unload: {} MB ({})\n",
        opt_f64(report.recovered_vram_mb),
        delta_note(report.recovered_vram_mb)
    ));
    output.push_str(&format!(
        "recovered fraction of retained: {}\n",
        report
            .recovered_fraction
            .map_or_else(|| "unavailable".to_owned(), |value| format!("{value:.2}"))
    ));
    output.push_str(&format!(
        "utilization growth during inference: {} percentage points\n",
        opt_f64(report.utilization_growth_percent)
    ));
    output
}

fn difference(condition: Option<f64>, baseline: Option<f64>) -> Option<f64> {
    match (condition, baseline) {
        (Some(condition), Some(baseline)) => Some(condition - baseline),
        _ => None,
    }
}

fn delta_note(value: Option<f64>) -> &'static str {
    if value.is_some() {
        "condition - baseline"
    } else {
        "unavailable"
    }
}

/// Sample-count and interval for a live telemetry capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureConfig {
    pub sample_count: u32,
    pub interval: Duration,
}

impl CaptureConfig {
    pub fn new(sample_count: u32, interval: Duration) -> Self {
        Self {
            sample_count,
            interval,
        }
    }

    pub fn validate(&self) -> Result<(), CaptureError> {
        if self.sample_count == 0 || self.sample_count > MAX_CAPTURE_SAMPLES {
            return Err(CaptureError::InvalidSampleCount(self.sample_count));
        }
        let interval_ms = u64::try_from(self.interval.as_millis()).unwrap_or(u64::MAX);
        if interval_ms > MAX_CAPTURE_INTERVAL_MS {
            return Err(CaptureError::InvalidInterval(interval_ms));
        }
        Ok(())
    }

    pub fn interval_ms(&self) -> u64 {
        u64::try_from(self.interval.as_millis()).unwrap_or(u64::MAX)
    }
}

/// Typed capture configuration errors.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CaptureError {
    #[error("sample count must be 1..={MAX_CAPTURE_SAMPLES}, found {0}")]
    InvalidSampleCount(u32),
    #[error("interval must be <= {MAX_CAPTURE_INTERVAL_MS} ms, found {0}")]
    InvalidInterval(u64),
}

/// Capture `config.sample_count` telemetry samples from `probe`.
///
/// The `wait` closure is invoked with the configured interval between samples
/// and is the only source of delay. Production callers pass
/// `std::thread::sleep`; tests pass a recorder. This function never loads a
/// model, starts a game, or changes GPU policy, and it does not filter or
/// fabricate samples: an unavailable probe sample is stored as unavailable.
pub fn capture_telemetry_series<P, F>(
    probe: &P,
    config: CaptureConfig,
    mut wait: F,
) -> Result<TelemetrySeries, CaptureError>
where
    P: GpuProbe + ?Sized,
    F: FnMut(Duration),
{
    config.validate()?;
    let mut series = TelemetrySeries::default();
    for index in 0..config.sample_count {
        let sample: GpuTelemetry = probe.probe();
        series.push(sample);
        if index + 1 < config.sample_count {
            wait(config.interval);
        }
    }
    Ok(series)
}

fn opt_u8(value: Option<u8>) -> String {
    value.map_or_else(String::new, |value| value.to_string())
}

fn opt_u64(value: Option<u64>) -> String {
    value.map_or_else(String::new, |value| value.to_string())
}

fn opt_f64(value: Option<f64>) -> String {
    value.map_or_else(String::new, |value| format!("{value:.2}"))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::gpu::GPU_TELEMETRY_SCHEMA;
    use crate::performance::{summarize_frame_times, TELEMETRY_SERIES_SCHEMA};

    fn sample(utilization: u8, used: u64, total: u64, temperature: u8) -> GpuTelemetry {
        GpuTelemetry {
            schema: GPU_TELEMETRY_SCHEMA.to_owned(),
            available: true,
            name: Some("Synthetic GPU".to_owned()),
            utilization_percent: Some(utilization),
            memory_used_mb: Some(used),
            memory_total_mb: Some(total),
            temperature_c: Some(temperature),
            source: "synthetic".to_owned(),
            checked_at: "2026-01-01T00:00:00Z".to_owned(),
            error: None,
        }
    }

    fn repeated(value: u8, used: u64, temperature: u8) -> TelemetrySeries {
        TelemetrySeries::new(vec![sample(value, used, 12288, temperature); 3])
    }

    fn frames(times: &[f64]) -> FrameTimeReport {
        summarize_frame_times(times, "fixture").expect("frame report")
    }

    #[test]
    fn comparison_reports_per_series_summaries_and_deltas() {
        let idle = LabeledMeasurement::new(ConditionLabel::Idle, repeated(5, 2000, 50))
            .with_frame_times(frames(&[16.0, 16.0, 16.0]));
        let gaming = LabeledMeasurement::new(ConditionLabel::Gaming, repeated(60, 9000, 75))
            .with_frame_times(frames(&[16.0, 33.0, 50.0]));
        let report = compare_conditions(ConditionLabel::Idle, &[idle, gaming]);

        assert_eq!(report.schema, COMPARISON_REPORT_SCHEMA);
        assert_eq!(report.baseline, ConditionLabel::Idle);
        assert!(report.baseline_present);
        assert_eq!(report.conditions.len(), 2);

        let idle_condition = &report.conditions[0];
        assert_eq!(idle_condition.label, ConditionLabel::Idle);
        assert_eq!(idle_condition.summary.sample_count, 3);
        assert_eq!(idle_condition.summary.utilization_avg_percent, Some(5.0));
        assert_eq!(idle_condition.deltas[0].absolute_delta, Some(0.0));

        let gaming_condition = &report.conditions[1];
        let used = gaming_condition
            .deltas
            .iter()
            .find(|delta| delta.metric == MetricKind::VramUsedAvgMb)
            .expect("used vram delta");
        assert_eq!(used.status, DeltaStatus::Available);
        assert_eq!(used.absolute_delta, Some(7000.0));

        let free = gaming_condition
            .deltas
            .iter()
            .find(|delta| delta.metric == MetricKind::VramFreeAvgMb)
            .expect("free vram delta");
        assert_eq!(free.absolute_delta, Some(-7000.0));

        let avg_fps = gaming_condition
            .deltas
            .iter()
            .find(|delta| delta.metric == MetricKind::AvgFps)
            .expect("fps delta");
        assert_eq!(avg_fps.status, DeltaStatus::Available);
        assert!(avg_fps.absolute_delta.is_some());
    }

    #[test]
    fn comparison_marks_missing_frame_data_unavailable() {
        let idle = LabeledMeasurement::new(ConditionLabel::Idle, repeated(5, 2000, 50));
        let inference = LabeledMeasurement::new(ConditionLabel::Inference, repeated(20, 4000, 60));
        let report = compare_conditions(ConditionLabel::Idle, &[idle, inference]);
        let condition = &report.conditions[1];
        let fps = condition
            .deltas
            .iter()
            .find(|delta| delta.metric == MetricKind::AvgFps)
            .expect("fps delta");
        assert_eq!(fps.status, DeltaStatus::ConditionUnavailable);
        assert!(fps.baseline.is_none());
        assert!(fps.condition.is_none());
        let text = render_comparison_report(&report);
        assert!(text.contains("frame data: unavailable (not supplied)"));
        assert!(text.contains("delta avg_fps: unavailable (condition_unavailable)"));
    }

    #[test]
    fn comparison_marks_missing_baseline_unavailable() {
        let gaming = LabeledMeasurement::new(ConditionLabel::Gaming, repeated(60, 9000, 75));
        let report = compare_conditions(ConditionLabel::Idle, &[gaming]);
        assert!(!report.baseline_present);
        assert!(report
            .conditions
            .iter()
            .flat_map(|condition| condition.deltas.iter())
            .all(|delta| delta.status == DeltaStatus::BaselineMissing));
        assert!(render_comparison_report(&report).contains("baseline status: unavailable"));
    }

    #[test]
    fn comparison_is_deterministic_and_serializable() {
        let idle = LabeledMeasurement::new(ConditionLabel::Idle, repeated(5, 2000, 50));
        let gaming = LabeledMeasurement::new(ConditionLabel::Gaming, repeated(60, 9000, 75));
        let first = compare_conditions(ConditionLabel::Idle, &[idle.clone(), gaming.clone()]);
        let second = compare_conditions(ConditionLabel::Idle, &[idle, gaming]);
        assert_eq!(
            render_comparison_report(&first),
            render_comparison_report(&second)
        );

        let encoded = serde_json::to_string(&first).expect("serialize");
        let decoded: ComparisonReport = serde_json::from_str(&encoded).expect("deserialize");
        assert_eq!(first, decoded);
    }

    #[test]
    fn comparison_with_no_measurements_is_empty_but_valid() {
        let report = compare_conditions(ConditionLabel::Idle, &[]);
        assert_eq!(report.schema, COMPARISON_REPORT_SCHEMA);
        assert!(!report.baseline_present);
        assert!(report.conditions.is_empty());
    }

    #[test]
    fn condition_labels_round_trip() {
        for label in [
            ConditionLabel::Idle,
            ConditionLabel::Inference,
            ConditionLabel::Gaming,
        ] {
            assert_eq!(ConditionLabel::parse(label.as_str()), Some(label));
        }
        assert_eq!(
            ConditionLabel::parse(" GAMING "),
            Some(ConditionLabel::Gaming)
        );
        assert_eq!(ConditionLabel::parse("other"), None);
    }

    #[derive(Debug)]
    struct SyntheticProbe {
        utilizations: Vec<u8>,
        index: Cell<usize>,
    }

    impl SyntheticProbe {
        fn new(utilizations: Vec<u8>) -> Self {
            Self {
                utilizations,
                index: Cell::new(0),
            }
        }
    }

    impl GpuProbe for SyntheticProbe {
        fn probe(&self) -> GpuTelemetry {
            let index = self.index.get();
            let utilization = self
                .utilizations
                .get(index)
                .copied()
                .unwrap_or_else(|| *self.utilizations.last().unwrap_or(&0));
            self.index.set(index + 1);
            sample(utilization, 1000 + utilization as u64, 12288, 50)
        }
    }

    #[test]
    fn capture_series_uses_probe_and_wait_closure() {
        let probe = SyntheticProbe::new(vec![4, 8, 12]);
        let mut waits = Vec::new();
        let series = capture_telemetry_series(
            &probe,
            CaptureConfig::new(3, Duration::from_millis(500)),
            |duration| waits.push(duration),
        )
        .expect("capture");

        assert_eq!(series.schema, TELEMETRY_SERIES_SCHEMA);
        assert_eq!(series.samples.len(), 3);
        let utilizations: Vec<u8> = series
            .samples
            .iter()
            .map(|sample| sample.utilization_percent.unwrap_or_default())
            .collect();
        assert_eq!(utilizations, vec![4, 8, 12]);
        // Two intervals between three samples; the last sample does not wait.
        assert_eq!(waits, vec![Duration::from_millis(500); 2]);
    }

    #[test]
    fn capture_rejects_invalid_configs_without_probing() {
        let probe = SyntheticProbe::new(vec![1]);
        let config = CaptureConfig::new(0, Duration::from_millis(10));
        assert_eq!(
            capture_telemetry_series(&probe, config, |_| {}),
            Err(CaptureError::InvalidSampleCount(0))
        );
        assert_eq!(probe.index.get(), 0);

        let config = CaptureConfig::new(1, Duration::from_millis(MAX_CAPTURE_INTERVAL_MS + 1));
        assert!(matches!(
            capture_telemetry_series(&probe, config, |_| {}),
            Err(CaptureError::InvalidInterval(_))
        ));
    }

    #[test]
    fn capture_keeps_unavailable_samples_as_unavailable() {
        #[derive(Debug)]
        struct MissingProbe;

        impl GpuProbe for MissingProbe {
            fn probe(&self) -> GpuTelemetry {
                GpuTelemetry::unavailable("synthetic", "no gpu")
            }
        }

        let series =
            capture_telemetry_series(&MissingProbe, CaptureConfig::new(2, Duration::ZERO), |_| {})
                .expect("capture");
        assert_eq!(series.samples.len(), 2);
        assert!(series.samples.iter().all(|sample| !sample.available));
        assert_eq!(series.summary().available_samples, 0);
    }

    #[test]
    fn pressure_computes_growth_retained_and_recovered() {
        let idle = repeated(5, 2000, 50);
        let during = repeated(40, 6000, 70);
        let after = repeated(6, 4000, 55);
        let unloaded = repeated(4, 2100, 48);
        let report = summarize_pressure(&idle, &during, &after, &unloaded);

        assert_eq!(report.schema, PRESSURE_REPORT_SCHEMA);
        assert_eq!(report.idle.label, "idle");
        assert_eq!(report.inference_during.utilization_max_percent, Some(40));
        assert_eq!(report.inference_vram_growth_mb, Some(4000.0));
        assert_eq!(report.retained_vram_mb, Some(2000.0));
        assert_eq!(report.recovered_vram_mb, Some(1900.0));
        assert_eq!(report.recovered_fraction, Some(0.95));
        assert_eq!(report.utilization_growth_percent, Some(35.0));

        let text = render_pressure_report(&report);
        assert!(text.contains("not gameplay"));
        assert!(text.contains("retained VRAM while resident: 2000.00 MB"));
        assert!(text.contains("recovered VRAM after unload: 1900.00 MB"));
    }

    #[test]
    fn pressure_missing_phase_data_is_explicitly_unavailable() {
        let idle = repeated(5, 2000, 50);
        let during = repeated(40, 6000, 70);
        let after = TelemetrySeries::new(vec![GpuTelemetry::unavailable("synthetic", "no gpu")]);
        let unloaded = repeated(4, 2100, 48);
        let report = summarize_pressure(&idle, &during, &after, &unloaded);

        assert_eq!(report.inference_after.available_samples, 0);
        assert_eq!(report.retained_vram_mb, None);
        assert_eq!(report.recovered_vram_mb, None);
        assert_eq!(report.recovered_fraction, None);
        assert_eq!(report.inference_vram_growth_mb, Some(4000.0));
        let text = render_pressure_report(&report);
        assert!(text.contains("retained VRAM while resident:  MB (unavailable)"));
        assert!(text.contains("recovered VRAM after unload:  MB (unavailable)"));
    }

    #[test]
    fn pressure_is_deterministic_and_serializable() {
        let idle = repeated(5, 2000, 50);
        let during = repeated(40, 6000, 70);
        let after = repeated(6, 4000, 55);
        let unloaded = repeated(4, 2100, 48);
        let first = summarize_pressure(&idle, &during, &after, &unloaded);
        let second = summarize_pressure(&idle, &during, &after, &unloaded);
        assert_eq!(
            render_pressure_report(&first),
            render_pressure_report(&second)
        );

        let encoded = serde_json::to_string(&first).expect("serialize");
        let decoded: PressureReport = serde_json::from_str(&encoded).expect("deserialize");
        assert_eq!(first, decoded);
    }
}

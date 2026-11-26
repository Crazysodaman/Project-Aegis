//! Offline GPU telemetry summarisation and manual frame-time ingestion.
//!
//! Nothing in this module talks to the GPU. It is a pure post-processing layer
//! over [`GpuTelemetry`] samples captured elsewhere (for example by
//! [`crate::gpu::NvidiaSmiProbe`]) plus an optional, owner-supplied frame-time
//! CSV. No metric is invented: missing samples stay [`None`] and frame-time
//! statistics are only produced from a real capture file.
//!
//! The workload classification and reclaimed-VRAM figures are **heuristics**.
//! They are not ground truth and cannot, on their own, establish that a game is
//! safe to run alongside background inference or that a frame rate is
//! acceptable. See `docs/implementation/BATCH5_AGENT02_GPU.md`.

use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::gpu::GpuTelemetry;

/// Schema identifier for an ordered telemetry series.
pub const TELEMETRY_SERIES_SCHEMA: &str = "aegis.fleet.telemetry_series.v1";
/// Schema identifier for a telemetry performance summary.
pub const PERFORMANCE_SUMMARY_SCHEMA: &str = "aegis.fleet.performance_summary.v1";
/// Schema identifier for a heuristic workload classification.
pub const WORKLOAD_CLASSIFICATION_SCHEMA: &str = "aegis.fleet.workload_classification.v1";
/// Schema identifier for a manual frame-time capture report.
pub const FRAME_TIME_REPORT_SCHEMA: &str = "aegis.fleet.frame_time_report.v1";

/// Average utilization below which a sample set is considered idle.
pub const CLASSIFY_IDLE_AVG_UTILIZATION_PERCENT: f64 = 10.0;
/// Peak utilization below which a sample set is considered idle.
pub const CLASSIFY_IDLE_MAX_UTILIZATION_PERCENT: u8 = 25;
/// Sustained average utilization at or above which a set is gaming-like.
pub const CLASSIFY_GAMING_AVG_UTILIZATION_PERCENT: f64 = 50.0;
/// Peak utilization at or above which a set is gaming-like.
pub const CLASSIFY_GAMING_MAX_UTILIZATION_PERCENT: u8 = 85;
/// Fraction of total VRAM used that, combined with activity, is gaming-like.
pub const CLASSIFY_GAMING_VRAM_FRACTION: f64 = 0.8;
/// Minimum average utilization for the high-VRAM gaming heuristic to apply.
pub const CLASSIFY_GAMING_VRAM_UTILIZATION_PERCENT: f64 = 30.0;

const SUMMARY_CSV_HEADER: &str = "schema,sample_count,available_samples,\
utilization_min_percent,utilization_max_percent,utilization_avg_percent,\
used_vram_min_mb,used_vram_max_mb,used_vram_avg_mb,\
free_vram_min_mb,free_vram_max_mb,free_vram_avg_mb,\
temperature_min_c,temperature_max_c,vram_total_mb,\
total_reclaimed_vram_mb,reclaimed_steps";

/// An ordered sequence of GPU telemetry samples.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TelemetrySeries {
    pub schema: String,
    pub samples: Vec<GpuTelemetry>,
}

impl Default for TelemetrySeries {
    fn default() -> Self {
        Self {
            schema: TELEMETRY_SERIES_SCHEMA.to_owned(),
            samples: Vec::new(),
        }
    }
}

impl TelemetrySeries {
    pub fn new(samples: Vec<GpuTelemetry>) -> Self {
        Self {
            schema: TELEMETRY_SERIES_SCHEMA.to_owned(),
            samples,
        }
    }

    pub fn push(&mut self, sample: GpuTelemetry) {
        self.samples.push(sample);
    }

    /// Deterministic statistics over the stored samples.
    pub fn summary(&self) -> PerformanceSummary {
        summarize_telemetry(&self.samples)
    }

    /// Heuristic classification derived from [`TelemetrySeries::summary`].
    pub fn classification(&self) -> WorkloadClassification {
        classify_workload(&self.summary())
    }
}

/// Summary statistics for a telemetry series.
///
/// `total_reclaimed_vram_mb` is the sum of positive free-VRAM deltas between
/// consecutive samples. It approximates the total VRAM released across yield
/// boundaries but does not distinguish an unload from an unrelated process
/// freeing memory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerformanceSummary {
    pub schema: String,
    pub sample_count: u32,
    pub available_samples: u32,
    pub utilization_min_percent: Option<u8>,
    pub utilization_max_percent: Option<u8>,
    pub utilization_avg_percent: Option<f64>,
    pub used_vram_min_mb: Option<u64>,
    pub used_vram_max_mb: Option<u64>,
    pub used_vram_avg_mb: Option<f64>,
    pub free_vram_min_mb: Option<u64>,
    pub free_vram_max_mb: Option<u64>,
    pub free_vram_avg_mb: Option<f64>,
    pub temperature_min_c: Option<u8>,
    pub temperature_max_c: Option<u8>,
    pub vram_total_mb: Option<u64>,
    pub total_reclaimed_vram_mb: Option<u64>,
    pub reclaimed_steps: u32,
}

/// Heuristic workload labels. These are *not* ground truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkloadClass {
    Idle,
    InferenceLike,
    GamingLike,
    Unknown,
}

impl WorkloadClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::InferenceLike => "inference_like",
            Self::GamingLike => "gaming_like",
            Self::Unknown => "unknown",
        }
    }
}

/// Classification plus the evidence used to reach it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkloadClassification {
    pub schema: String,
    pub class: WorkloadClass,
    pub sample_count: u32,
    pub avg_utilization_percent: Option<f64>,
    pub max_utilization_percent: Option<u8>,
    pub avg_used_vram_mb: Option<f64>,
    pub vram_total_mb: Option<u64>,
    pub vram_fraction: Option<f64>,
    pub rationale: Vec<String>,
}

/// Summarise a slice of telemetry samples. Pure; never panics on missing data.
pub fn summarize_telemetry(samples: &[GpuTelemetry]) -> PerformanceSummary {
    let available_samples = samples.iter().filter(|sample| sample.available).count();
    let utilization: Vec<u8> = samples
        .iter()
        .filter(|sample| sample.available)
        .filter_map(|sample| sample.utilization_percent)
        .collect();
    let used: Vec<u64> = samples
        .iter()
        .filter(|sample| sample.available)
        .filter_map(|sample| sample.memory_used_mb)
        .collect();
    let free: Vec<Option<u64>> = samples
        .iter()
        .map(|sample| {
            if sample.available {
                sample.free_vram_mb()
            } else {
                None
            }
        })
        .collect();
    let free_values: Vec<u64> = free.iter().filter_map(|value| *value).collect();
    let temperature: Vec<u8> = samples
        .iter()
        .filter(|sample| sample.available)
        .filter_map(|sample| sample.temperature_c)
        .collect();
    let vram_total = samples
        .iter()
        .filter(|sample| sample.available)
        .filter_map(|sample| sample.memory_total_mb)
        .max();

    let (utilization_min, utilization_max, utilization_avg) = stats_u8(&utilization);
    let (used_min, used_max, used_avg) = stats_u64(&used);
    let (free_min, free_max, free_avg) = stats_u64(&free_values);
    let (temperature_min, temperature_max, _) = stats_u8(&temperature);
    let (total_reclaimed, reclaimed_steps) = reclaimed_vram(&free);

    PerformanceSummary {
        schema: PERFORMANCE_SUMMARY_SCHEMA.to_owned(),
        sample_count: saturating_u32(samples.len()),
        available_samples: saturating_u32(available_samples),
        utilization_min_percent: utilization_min,
        utilization_max_percent: utilization_max,
        utilization_avg_percent: utilization_avg,
        used_vram_min_mb: used_min,
        used_vram_max_mb: used_max,
        used_vram_avg_mb: used_avg,
        free_vram_min_mb: free_min,
        free_vram_max_mb: free_max,
        free_vram_avg_mb: free_avg,
        temperature_min_c: temperature_min,
        temperature_max_c: temperature_max,
        vram_total_mb: vram_total,
        total_reclaimed_vram_mb: total_reclaimed,
        reclaimed_steps,
    }
}

/// Classify a summary using documented utilization/VRAM heuristics.
///
/// Order matters: idle is checked first so a low-utilization menu does not get
/// labelled gaming-like. The label is informational and must not be used as a
/// safety gate; admission is decided by [`crate::gpu::evaluate_admission`].
pub fn classify_workload(summary: &PerformanceSummary) -> WorkloadClassification {
    let mut rationale = Vec::new();
    let vram_fraction = match (summary.used_vram_avg_mb, summary.vram_total_mb) {
        (Some(used), Some(total)) if total > 0 => Some(used / total as f64),
        _ => None,
    };

    let class = match (
        summary.utilization_avg_percent,
        summary.utilization_max_percent,
    ) {
        (Some(avg), max) => {
            let max = max.unwrap_or_default();
            let high_vram = vram_fraction.is_some_and(|fraction| {
                fraction >= CLASSIFY_GAMING_VRAM_FRACTION
                    && avg >= CLASSIFY_GAMING_VRAM_UTILIZATION_PERCENT
            });
            if avg < CLASSIFY_IDLE_AVG_UTILIZATION_PERCENT
                && max < CLASSIFY_IDLE_MAX_UTILIZATION_PERCENT
            {
                rationale.push(format!(
                    "avg utilization {avg:.2}% < {CLASSIFY_IDLE_AVG_UTILIZATION_PERCENT}% and peak {max}% < {CLASSIFY_IDLE_MAX_UTILIZATION_PERCENT}%"
                ));
                WorkloadClass::Idle
            } else if avg >= CLASSIFY_GAMING_AVG_UTILIZATION_PERCENT {
                rationale.push(format!(
                    "avg utilization {avg:.2}% >= {CLASSIFY_GAMING_AVG_UTILIZATION_PERCENT}%"
                ));
                WorkloadClass::GamingLike
            } else if max >= CLASSIFY_GAMING_MAX_UTILIZATION_PERCENT {
                rationale.push(format!(
                    "peak utilization {max}% >= {CLASSIFY_GAMING_MAX_UTILIZATION_PERCENT}%"
                ));
                WorkloadClass::GamingLike
            } else if high_vram {
                rationale.push(format!(
                    "avg VRAM fraction {:.2} >= {CLASSIFY_GAMING_VRAM_FRACTION} with avg utilization {avg:.2}%",
                    vram_fraction.unwrap_or_default()
                ));
                WorkloadClass::GamingLike
            } else {
                rationale.push(format!(
                    "avg utilization {avg:.2}% and peak {max}% below gaming thresholds"
                ));
                WorkloadClass::InferenceLike
            }
        }
        (None, _) => {
            rationale.push("no available utilization samples".to_owned());
            WorkloadClass::Unknown
        }
    };

    WorkloadClassification {
        schema: WORKLOAD_CLASSIFICATION_SCHEMA.to_owned(),
        class,
        sample_count: summary.available_samples,
        avg_utilization_percent: summary.utilization_avg_percent,
        max_utilization_percent: summary.utilization_max_percent,
        avg_used_vram_mb: summary.used_vram_avg_mb,
        vram_total_mb: summary.vram_total_mb,
        vram_fraction,
        rationale,
    }
}

/// Render a one-header/one-row CSV for a summary. Deterministic field order.
pub fn render_summary_csv(summary: &PerformanceSummary) -> String {
    let fields = [
        summary.schema.clone(),
        summary.sample_count.to_string(),
        summary.available_samples.to_string(),
        opt_u8(summary.utilization_min_percent),
        opt_u8(summary.utilization_max_percent),
        opt_f64(summary.utilization_avg_percent),
        opt_u64(summary.used_vram_min_mb),
        opt_u64(summary.used_vram_max_mb),
        opt_f64(summary.used_vram_avg_mb),
        opt_u64(summary.free_vram_min_mb),
        opt_u64(summary.free_vram_max_mb),
        opt_f64(summary.free_vram_avg_mb),
        opt_u8(summary.temperature_min_c),
        opt_u8(summary.temperature_max_c),
        opt_u64(summary.vram_total_mb),
        opt_u64(summary.total_reclaimed_vram_mb),
        summary.reclaimed_steps.to_string(),
    ];
    format!("{SUMMARY_CSV_HEADER}\n{}\n", fields.join(","))
}

/// Human-readable report combining a summary and its classification.
pub fn render_summary_report(
    summary: &PerformanceSummary,
    classification: &WorkloadClassification,
) -> String {
    let mut report = String::new();
    report.push_str("Aegis GPU performance summary (heuristic, not ground truth)\n");
    report.push_str(&format!(
        "samples: total={} available={}\n",
        summary.sample_count, summary.available_samples
    ));
    report.push_str(&format!(
        "utilization %: min={} max={} avg={}\n",
        opt_u8(summary.utilization_min_percent),
        opt_u8(summary.utilization_max_percent),
        opt_f64(summary.utilization_avg_percent)
    ));
    report.push_str(&format!(
        "used VRAM MB: min={} max={} avg={}\n",
        opt_u64(summary.used_vram_min_mb),
        opt_u64(summary.used_vram_max_mb),
        opt_f64(summary.used_vram_avg_mb)
    ));
    report.push_str(&format!(
        "free VRAM MB: min={} max={} avg={}\n",
        opt_u64(summary.free_vram_min_mb),
        opt_u64(summary.free_vram_max_mb),
        opt_f64(summary.free_vram_avg_mb)
    ));
    report.push_str(&format!(
        "temperature C: min={} max={}\n",
        opt_u8(summary.temperature_min_c),
        opt_u8(summary.temperature_max_c)
    ));
    report.push_str(&format!(
        "reclaimed VRAM MB: {} across {} increase steps\n",
        opt_u64(summary.total_reclaimed_vram_mb),
        summary.reclaimed_steps
    ));
    report.push_str(&format!(
        "classification: {}\n",
        classification.class.as_str()
    ));
    for reason in &classification.rationale {
        report.push_str(&format!("  - {reason}\n"));
    }
    report
}

/// Typed failures when ingesting frame times.
///
/// Every malformed row is rejected rather than guessed at; blank lines are
/// skipped and a single leading non-numeric header row is ignored.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum FrameTimeError {
    #[error("no frame-time samples")]
    Empty,
    #[error("line {line}: expected 2 fields, found {found}")]
    FieldCount { line: usize, found: usize },
    #[error("line {line}: invalid timestamp {value:?}")]
    Timestamp { line: usize, value: String },
    #[error("line {line}: invalid frame time {value:?}")]
    FrameTime { line: usize, value: String },
    #[error("line {line}: non-finite frame time {value}")]
    NonFiniteFrameTime { line: usize, value: f64 },
    #[error("line {line}: frame time must be positive, found {value}")]
    NonPositiveFrameTime { line: usize, value: f64 },
    #[error("frame time must be finite and positive, found {value}")]
    InvalidSample { value: f64 },
}

/// Result of ingesting an owner-captured `timestamp_ms,frame_time_ms` CSV.
///
/// Percentile frame times use the nearest-rank method over the ascending frame
/// times: `p95` is the value at rank `ceil(0.95 * count)` clamped into range.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameTimeReport {
    pub schema: String,
    pub available: bool,
    pub sample_count: u32,
    pub duration_ms: Option<f64>,
    pub avg_fps: Option<f64>,
    pub one_percent_low_fps: Option<f64>,
    pub point_one_percent_low_fps: Option<f64>,
    pub max_frame_time_ms: Option<f64>,
    pub p95_frame_time_ms: Option<f64>,
    pub p99_frame_time_ms: Option<f64>,
    pub source: String,
    pub error: Option<String>,
}

impl FrameTimeReport {
    /// Explicit "no frame data" report. Never fabricates metrics.
    pub fn unavailable(source: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            schema: FRAME_TIME_REPORT_SCHEMA.to_owned(),
            available: false,
            sample_count: 0,
            duration_ms: None,
            avg_fps: None,
            one_percent_low_fps: None,
            point_one_percent_low_fps: None,
            max_frame_time_ms: None,
            p95_frame_time_ms: None,
            p99_frame_time_ms: None,
            source: source.into(),
            error: Some(reason.into()),
        }
    }
}

/// Compute frame statistics from already-parsed frame times (milliseconds).
pub fn summarize_frame_times(
    frame_times_ms: &[f64],
    source: impl Into<String>,
) -> Result<FrameTimeReport, FrameTimeError> {
    if frame_times_ms.is_empty() {
        return Err(FrameTimeError::Empty);
    }
    if let Some(bad) = frame_times_ms
        .iter()
        .find(|value| !value.is_finite() || **value <= 0.0)
    {
        return Err(FrameTimeError::InvalidSample { value: *bad });
    }
    let count = frame_times_ms.len();
    let total_ms: f64 = frame_times_ms.iter().sum();
    let avg_fps = 1000.0 * count as f64 / total_ms;
    let max_frame_time_ms = frame_times_ms
        .iter()
        .fold(f64::MIN, |acc, value| acc.max(*value));
    Ok(FrameTimeReport {
        schema: FRAME_TIME_REPORT_SCHEMA.to_owned(),
        available: true,
        sample_count: saturating_u32(count),
        duration_ms: Some(total_ms),
        avg_fps: Some(avg_fps),
        one_percent_low_fps: worst_fraction_fps(frame_times_ms, 0.01),
        point_one_percent_low_fps: worst_fraction_fps(frame_times_ms, 0.001),
        max_frame_time_ms: Some(max_frame_time_ms),
        p95_frame_time_ms: frame_time_percentile(frame_times_ms, 95.0),
        p99_frame_time_ms: frame_time_percentile(frame_times_ms, 99.0),
        source: source.into(),
        error: None,
    })
}

/// Parse an optional `timestamp_ms,frame_time_ms` CSV.
///
/// A single leading header row is tolerated and skipped. Blank lines are
/// ignored. Every other row must contain two fields: a finite non-negative
/// timestamp and a finite positive frame time. Malformed rows are rejected with
/// a [`FrameTimeError`]; the parser never substitutes placeholder data.
pub fn parse_frame_times_csv(text: &str) -> Result<FrameTimeReport, FrameTimeError> {
    let mut frame_times = Vec::new();
    let mut saw_content = false;
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if !saw_content {
            saw_content = true;
            if !is_numeric_row(line) {
                continue;
            }
        }
        let fields: Vec<&str> = line.split(',').map(str::trim).collect();
        if fields.len() != 2 {
            return Err(FrameTimeError::FieldCount {
                line: index + 1,
                found: fields.len(),
            });
        }
        let timestamp: f64 = fields[0].parse().map_err(|_| FrameTimeError::Timestamp {
            line: index + 1,
            value: fields[0].to_owned(),
        })?;
        let frame_time: f64 = fields[1].parse().map_err(|_| FrameTimeError::FrameTime {
            line: index + 1,
            value: fields[1].to_owned(),
        })?;
        if !timestamp.is_finite() || timestamp < 0.0 {
            return Err(FrameTimeError::Timestamp {
                line: index + 1,
                value: timestamp.to_string(),
            });
        }
        if !frame_time.is_finite() {
            return Err(FrameTimeError::NonFiniteFrameTime {
                line: index + 1,
                value: frame_time,
            });
        }
        if frame_time <= 0.0 {
            return Err(FrameTimeError::NonPositiveFrameTime {
                line: index + 1,
                value: frame_time,
            });
        }
        frame_times.push(frame_time);
    }
    summarize_frame_times(&frame_times, "frame_time_csv")
}

/// Read and summarise a frame-time CSV, or report unavailable when absent.
pub fn load_frame_time_report(path: Option<&Path>) -> FrameTimeReport {
    let Some(path) = path else {
        return FrameTimeReport::unavailable("none", "no frame-time capture provided");
    };
    let source = path.display().to_string();
    match std::fs::read_to_string(path) {
        Ok(text) => match parse_frame_times_csv(&text) {
            Ok(report) => report,
            Err(error) => FrameTimeReport::unavailable(source, error.to_string()),
        },
        Err(error) => {
            FrameTimeReport::unavailable(source, format!("failed to read capture: {error}"))
        }
    }
}

fn is_numeric_row(line: &str) -> bool {
    let fields: Vec<&str> = line.split(',').map(str::trim).collect();
    fields.len() == 2 && fields.iter().all(|field| field.parse::<f64>().is_ok())
}

fn worst_fraction_fps(frame_times_ms: &[f64], fraction: f64) -> Option<f64> {
    if frame_times_ms.is_empty() {
        return None;
    }
    let mut sorted = frame_times_ms.to_vec();
    sorted.sort_by(|left, right| right.total_cmp(left));
    let count = ((sorted.len() as f64) * fraction).ceil().max(1.0) as usize;
    let count = count.min(sorted.len());
    let windows = &sorted[..count];
    let mean = windows.iter().sum::<f64>() / count as f64;
    if mean > 0.0 {
        Some(1000.0 / mean)
    } else {
        None
    }
}

/// Nearest-rank frame-time percentile in milliseconds.
fn frame_time_percentile(frame_times_ms: &[f64], percentile: f64) -> Option<f64> {
    if frame_times_ms.is_empty() {
        return None;
    }
    let mut sorted = frame_times_ms.to_vec();
    sorted.sort_by(f64::total_cmp);
    let clamped = percentile.clamp(0.0, 100.0);
    let rank = ((clamped / 100.0) * sorted.len() as f64).ceil().max(1.0) as usize;
    let index = rank.saturating_sub(1).min(sorted.len() - 1);
    Some(sorted[index])
}

fn stats_u8(values: &[u8]) -> (Option<u8>, Option<u8>, Option<f64>) {
    if values.is_empty() {
        return (None, None, None);
    }
    let min = values.iter().copied().min();
    let max = values.iter().copied().max();
    let avg = values.iter().map(|value| f64::from(*value)).sum::<f64>() / values.len() as f64;
    (min, max, Some(avg))
}

fn stats_u64(values: &[u64]) -> (Option<u64>, Option<u64>, Option<f64>) {
    if values.is_empty() {
        return (None, None, None);
    }
    let min = values.iter().copied().min();
    let max = values.iter().copied().max();
    let avg = values.iter().map(|value| *value as f64).sum::<f64>() / values.len() as f64;
    (min, max, Some(avg))
}

fn reclaimed_vram(free: &[Option<u64>]) -> (Option<u64>, u32) {
    let mut measured = false;
    let mut total = 0u64;
    let mut steps = 0u32;
    for pair in free.windows(2) {
        if let (Some(before), Some(after)) = (pair[0], pair[1]) {
            measured = true;
            if after > before {
                total = total.saturating_add(after - before);
                steps = steps.saturating_add(1);
            }
        }
    }
    if measured {
        (Some(total), steps)
    } else {
        (None, 0)
    }
}

fn saturating_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
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
    use super::*;
    use crate::gpu::GPU_TELEMETRY_SCHEMA;

    fn sample(utilization: u8, used: u64, total: u64, temperature: u8) -> GpuTelemetry {
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

    #[test]
    fn summary_computes_ranges_and_reclaimed_vram() {
        let series = TelemetrySeries::new(vec![
            sample(90, 6000, 12288, 70),
            sample(4, 1000, 12288, 50),
            sample(2, 100, 12288, 48),
        ]);
        assert_eq!(series.schema, TELEMETRY_SERIES_SCHEMA);
        let summary = series.summary();
        assert_eq!(summary.schema, PERFORMANCE_SUMMARY_SCHEMA);
        assert_eq!(summary.sample_count, 3);
        assert_eq!(summary.available_samples, 3);
        assert_eq!(summary.utilization_min_percent, Some(2));
        assert_eq!(summary.utilization_max_percent, Some(90));
        assert_eq!(summary.utilization_avg_percent, Some(32.0));
        assert_eq!(summary.used_vram_min_mb, Some(100));
        assert_eq!(summary.used_vram_max_mb, Some(6000));
        assert_eq!(summary.temperature_min_c, Some(48));
        assert_eq!(summary.temperature_max_c, Some(70));
        assert_eq!(summary.vram_total_mb, Some(12288));
        // Free jumps 6288 -> 11288 (5000) then -> 12188 (900): 5900 total.
        assert_eq!(summary.total_reclaimed_vram_mb, Some(5900));
        assert_eq!(summary.reclaimed_steps, 2);
        assert_eq!(summary.free_vram_min_mb, Some(6288));
        assert_eq!(summary.free_vram_max_mb, Some(12188));
    }

    #[test]
    fn summary_does_not_invent_missing_data() {
        let unavailable = GpuTelemetry::unavailable("test", "no gpu");
        let summary = TelemetrySeries::new(vec![unavailable]).summary();
        assert_eq!(summary.sample_count, 1);
        assert_eq!(summary.available_samples, 0);
        assert_eq!(summary.utilization_min_percent, None);
        assert_eq!(summary.utilization_avg_percent, None);
        assert_eq!(summary.total_reclaimed_vram_mb, None);
        assert_eq!(summary.reclaimed_steps, 0);
        assert_eq!(TelemetrySeries::new(Vec::new()).summary().sample_count, 0);
    }

    #[test]
    fn classification_labels_heuristics() {
        let idle = TelemetrySeries::new(vec![sample(3, 900, 12288, 45)]).classification();
        assert_eq!(idle.class, WorkloadClass::Idle);

        let inference = TelemetrySeries::new(vec![sample(20, 4000, 12288, 60)]).classification();
        assert_eq!(inference.class, WorkloadClass::InferenceLike);

        let sustained = TelemetrySeries::new(vec![sample(60, 6000, 12288, 70)]).classification();
        assert_eq!(sustained.class, WorkloadClass::GamingLike);

        let spike = TelemetrySeries::new(vec![
            sample(5, 1000, 12288, 50),
            sample(95, 1000, 12288, 50),
        ])
        .classification();
        assert_eq!(spike.class, WorkloadClass::GamingLike);

        let high_vram = TelemetrySeries::new(vec![sample(35, 11000, 12288, 65)]).classification();
        assert_eq!(high_vram.class, WorkloadClass::GamingLike);

        let unknown =
            TelemetrySeries::new(vec![GpuTelemetry::unavailable("test", "none")]).classification();
        assert_eq!(unknown.class, WorkloadClass::Unknown);
        assert!(!unknown.rationale.is_empty());
    }

    #[test]
    fn csv_renderer_is_deterministic() {
        let summary = TelemetrySeries::new(vec![sample(10, 2000, 12288, 55)]).summary();
        let first = render_summary_csv(&summary);
        let second = render_summary_csv(&summary);
        assert_eq!(first, second);
        let mut lines = first.lines();
        assert_eq!(lines.next(), Some(SUMMARY_CSV_HEADER));
        let row = lines.next().expect("row");
        assert!(row.starts_with("aegis.fleet.performance_summary.v1,1,1,10,10,10.00,"));
        assert_eq!(
            row.split(',').count(),
            SUMMARY_CSV_HEADER.split(',').count()
        );
    }

    #[test]
    fn report_mentions_heuristic_and_class() {
        let series = TelemetrySeries::new(vec![sample(3, 900, 12288, 45)]);
        let report = render_summary_report(&series.summary(), &series.classification());
        assert!(report.contains("heuristic, not ground truth"));
        assert!(report.contains("classification: idle"));
    }

    #[test]
    fn frame_time_csv_computes_fps_and_lows() {
        let csv = "timestamp_ms,frame_time_ms\n0,16.0\n16,16.0\n32,16.0\n48,50.0\n";
        let report = parse_frame_times_csv(csv).expect("parse");
        assert!(report.available);
        assert_eq!(report.sample_count, 4);
        assert_eq!(report.duration_ms, Some(98.0));
        assert_eq!(report.max_frame_time_ms, Some(50.0));
        // avg = 1000 * 4 / 98 ms.
        assert!((report.avg_fps.expect("avg") - 40.816_326_530_612_24).abs() < 1e-9);
        // 1% low = worst single frame (4 frames * 1% -> 1 frame) = 20 FPS.
        assert_eq!(report.one_percent_low_fps, Some(20.0));
        assert_eq!(report.point_one_percent_low_fps, Some(20.0));
    }

    #[test]
    fn frame_time_lows_use_worst_fraction_mean() {
        let times = vec![16.0, 16.0, 16.0, 100.0, 100.0, 100.0];
        let report = summarize_frame_times(&times, "test").expect("report");
        // 1% of 6 frames -> 1 frame -> 100 ms -> 10 FPS.
        assert_eq!(report.one_percent_low_fps, Some(10.0));
        // 50% mean of slowest three frames = 100 ms -> 10 FPS.
        let half = worst_fraction_fps(&times, 0.5);
        assert_eq!(half, Some(10.0));
    }

    #[test]
    fn frame_time_parser_rejects_bad_and_reports_empty() {
        assert!(parse_frame_times_csv("").is_err());
        assert!(parse_frame_times_csv("timestamp_ms,frame_time_ms\n").is_err());
        assert!(parse_frame_times_csv("0,16\nbad,16\n").is_err());
        assert!(parse_frame_times_csv("0,0\n").is_err());
        assert!(parse_frame_times_csv("0,16,extra\n").is_err());
        assert!(summarize_frame_times(&[], "test").is_err());
        assert!(summarize_frame_times(&[16.0, f64::NAN], "test").is_err());
    }

    #[test]
    fn frame_time_errors_are_typed_and_located() {
        assert_eq!(parse_frame_times_csv(""), Err(FrameTimeError::Empty));
        assert_eq!(
            parse_frame_times_csv("0,16\n16,16,extra\n"),
            Err(FrameTimeError::FieldCount { line: 2, found: 3 })
        );
        assert_eq!(
            parse_frame_times_csv("timestamp_ms,frame_time_ms\n0,bad\n"),
            Err(FrameTimeError::FrameTime {
                line: 2,
                value: "bad".to_owned()
            })
        );
        assert_eq!(
            parse_frame_times_csv("0,16\n16,-1\n"),
            Err(FrameTimeError::NonPositiveFrameTime {
                line: 2,
                value: -1.0
            })
        );
        assert!(matches!(
            summarize_frame_times(&[16.0, f64::INFINITY], "test"),
            Err(FrameTimeError::InvalidSample { value }) if value.is_infinite()
        ));
    }

    #[test]
    fn frame_time_percentiles_use_nearest_rank() {
        let times: Vec<f64> = (1..=100).map(|value| value as f64).collect();
        let report = summarize_frame_times(&times, "fixture").expect("report");
        assert_eq!(report.sample_count, 100);
        assert_eq!(report.max_frame_time_ms, Some(100.0));
        assert_eq!(report.p95_frame_time_ms, Some(95.0));
        assert_eq!(report.p99_frame_time_ms, Some(99.0));
        // 1% low = mean of slowest 1 frame = 100 ms -> 10 FPS.
        assert_eq!(report.one_percent_low_fps, Some(10.0));
    }

    #[test]
    fn missing_frame_capture_is_explicitly_unavailable() {
        let report = load_frame_time_report(None);
        assert!(!report.available);
        assert_eq!(report.sample_count, 0);
        assert!(report.avg_fps.is_none());
        assert!(report
            .error
            .unwrap_or_default()
            .contains("no frame-time capture"));
    }

    #[test]
    fn frame_time_csv_file_round_trips() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("frames.csv");
        std::fs::write(&path, "timestamp_ms,frame_time_ms\n0,10\n10,10\n").expect("write");
        let report = load_frame_time_report(Some(&path));
        assert!(report.available);
        assert_eq!(report.sample_count, 2);
        assert_eq!(report.max_frame_time_ms, Some(10.0));
    }
}

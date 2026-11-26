//! GPU telemetry and adaptive admission policy.
//!
//! Telemetry is read from `nvidia-smi` where available. No metric is invented:
//! missing or unparseable data is reported as unavailable and admission fails
//! closed for GPU-bound background inference.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use aegis_storage::now_rfc3339;
use serde::{Deserialize, Serialize};

/// Schema identifier for a GPU telemetry sample.
pub const GPU_TELEMETRY_SCHEMA: &str = "aegis.fleet.gpu_telemetry.v1";
/// Schema identifier for an admission decision.
pub const GPU_ADMISSION_SCHEMA: &str = "aegis.fleet.gpu_admission.v1";
/// Default maximum GPU utilization before yielding.
pub const DEFAULT_MAX_UTILIZATION_PERCENT: u8 = 25;
/// Default free-VRAM floor.
pub const DEFAULT_MIN_FREE_VRAM_MB: u32 = 2048;
/// Default maximum GPU temperature.
pub const DEFAULT_MAX_TEMPERATURE_C: u8 = 80;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuTelemetry {
    pub schema: String,
    pub available: bool,
    pub name: Option<String>,
    pub utilization_percent: Option<u8>,
    pub memory_used_mb: Option<u64>,
    pub memory_total_mb: Option<u64>,
    pub temperature_c: Option<u8>,
    pub source: String,
    pub checked_at: String,
    pub error: Option<String>,
}

impl GpuTelemetry {
    pub fn unavailable(source: &str, error: impl Into<String>) -> Self {
        Self {
            schema: GPU_TELEMETRY_SCHEMA.to_owned(),
            available: false,
            name: None,
            utilization_percent: None,
            memory_used_mb: None,
            memory_total_mb: None,
            temperature_c: None,
            source: source.to_owned(),
            checked_at: now_rfc3339().unwrap_or_default(),
            error: Some(error.into()),
        }
    }

    pub fn free_vram_mb(&self) -> Option<u64> {
        match (self.memory_used_mb, self.memory_total_mb) {
            (Some(used), Some(total)) => Some(total.saturating_sub(used)),
            _ => None,
        }
    }
}

pub trait GpuProbe: std::fmt::Debug {
    fn probe(&self) -> GpuTelemetry;
}

/// `nvidia-smi`-backed probe.
#[derive(Debug, Clone)]
pub struct NvidiaSmiProbe {
    binary: PathBuf,
}

impl Default for NvidiaSmiProbe {
    fn default() -> Self {
        Self {
            binary: PathBuf::from("nvidia-smi"),
        }
    }
}

impl NvidiaSmiProbe {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_binary(mut self, binary: impl Into<PathBuf>) -> Self {
        self.binary = binary.into();
        self
    }

    pub fn binary(&self) -> &std::path::Path {
        &self.binary
    }
}

impl GpuProbe for NvidiaSmiProbe {
    fn probe(&self) -> GpuTelemetry {
        let output = Command::new(&self.binary)
            .args([
                "--query-gpu=name,utilization.gpu,memory.used,memory.total,temperature.gpu",
                "--format=csv,noheader,nounits",
            ])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output();
        let output = match output {
            Ok(output) => output,
            Err(error) => {
                return GpuTelemetry::unavailable("nvidia-smi", error.to_string());
            }
        };
        if !output.status.success() {
            return GpuTelemetry::unavailable(
                "nvidia-smi",
                format!("nvidia-smi exited with {}", output.status),
            );
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let Some(line) = stdout.lines().find(|line| !line.trim().is_empty()) else {
            return GpuTelemetry::unavailable("nvidia-smi", "no GPU rows returned");
        };
        parse_nvidia_smi_line(line)
            .unwrap_or_else(|error| GpuTelemetry::unavailable("nvidia-smi", error))
    }
}

/// Parse one `nvidia-smi` CSV row:
/// `name, utilization.gpu, memory.used, memory.total, temperature.gpu`
pub fn parse_nvidia_smi_line(line: &str) -> Result<GpuTelemetry, String> {
    let fields: Vec<&str> = line.split(',').map(str::trim).collect();
    if fields.len() != 5 {
        return Err(format!("expected 5 fields, found {}", fields.len()));
    }
    let utilization = fields[1]
        .parse::<u8>()
        .map_err(|_| "invalid utilization".to_owned())?;
    let memory_used = fields[2]
        .parse::<u64>()
        .map_err(|_| "invalid memory used".to_owned())?;
    let memory_total = fields[3]
        .parse::<u64>()
        .map_err(|_| "invalid memory total".to_owned())?;
    let temperature = fields[4]
        .parse::<u8>()
        .map_err(|_| "invalid temperature".to_owned())?;
    if fields[0].is_empty() || memory_total == 0 || memory_used > memory_total {
        return Err("invalid GPU values".to_owned());
    }
    Ok(GpuTelemetry {
        schema: GPU_TELEMETRY_SCHEMA.to_owned(),
        available: true,
        name: Some(fields[0].to_owned()),
        utilization_percent: Some(utilization),
        memory_used_mb: Some(memory_used),
        memory_total_mb: Some(memory_total),
        temperature_c: Some(temperature),
        source: "nvidia-smi".to_owned(),
        checked_at: now_rfc3339().map_err(|error| error.to_string())?,
        error: None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuPolicy {
    pub gaming_yield: bool,
    pub max_utilization_percent: u8,
    pub min_free_vram_mb: u32,
    pub max_temperature_c: u8,
}

impl Default for GpuPolicy {
    fn default() -> Self {
        Self {
            gaming_yield: true,
            max_utilization_percent: DEFAULT_MAX_UTILIZATION_PERCENT,
            min_free_vram_mb: DEFAULT_MIN_FREE_VRAM_MB,
            max_temperature_c: DEFAULT_MAX_TEMPERATURE_C,
        }
    }
}

impl GpuPolicy {
    pub fn validate(&self) -> Result<(), String> {
        if self.max_utilization_percent > 100 {
            return Err("max utilization must be 0-100".to_owned());
        }
        if self.min_free_vram_mb == 0 || self.min_free_vram_mb > 1024 * 1024 {
            return Err("min free VRAM is out of range".to_owned());
        }
        if self.max_temperature_c < 30 || self.max_temperature_c > 110 {
            return Err("max temperature is out of range".to_owned());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdmissionDecision {
    pub schema: String,
    pub admit: bool,
    pub reasons: Vec<String>,
    pub telemetry: GpuTelemetry,
}

/// Fail-closed admission decision for background GPU inference.
pub fn evaluate_admission(policy: &GpuPolicy, telemetry: &GpuTelemetry) -> AdmissionDecision {
    evaluate_admission_with_gaming(policy, telemetry, None)
}

/// Admission decision that also honours an explicit gaming-workload signal.
///
/// A detected gaming process denies background inference when `gaming_yield`
/// is enabled, even if GPU utilization is momentarily low (menu, loading, or
/// paused game). An unavailable gaming probe is *not* by itself a denial
/// reason: the GPU telemetry path already fails closed when it is missing.
pub fn evaluate_admission_with_gaming(
    policy: &GpuPolicy,
    telemetry: &GpuTelemetry,
    gaming: Option<&crate::gaming::GamingSignal>,
) -> AdmissionDecision {
    let mut reasons = Vec::new();
    if !telemetry.available {
        reasons.push("gpu_telemetry_unavailable".to_owned());
    }
    if let Some(gaming) = gaming {
        if policy.gaming_yield && gaming.detected {
            reasons.push("gaming_detected".to_owned());
        }
    }
    let utilization = telemetry.utilization_percent;
    let free_vram = telemetry.free_vram_mb();
    let temperature = telemetry.temperature_c;
    if telemetry.available
        && (utilization.is_none() || free_vram.is_none() || temperature.is_none())
    {
        reasons.push("gpu_telemetry_incomplete".to_owned());
    }
    if let Some(free) = free_vram {
        if free < u64::from(policy.min_free_vram_mb) {
            reasons.push("vram_low".to_owned());
        }
    }
    if let Some(utilization) = utilization {
        if policy.gaming_yield && utilization > policy.max_utilization_percent {
            reasons.push("gpu_busy".to_owned());
        }
    }
    if let Some(temperature) = temperature {
        if temperature > policy.max_temperature_c {
            reasons.push("gpu_hot".to_owned());
        }
    }
    AdmissionDecision {
        schema: GPU_ADMISSION_SCHEMA.to_owned(),
        admit: reasons.is_empty(),
        reasons,
        telemetry: telemetry.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn parses_nvidia_smi_rows() {
        let parsed = parse_nvidia_smi_line("NVIDIA GeForce RTX 3080 Ti, 10, 2443, 12288, 59")
            .expect("parse");
        assert!(parsed.available);
        assert_eq!(parsed.utilization_percent, Some(10));
        assert_eq!(parsed.memory_used_mb, Some(2443));
        assert_eq!(parsed.memory_total_mb, Some(12288));
        assert_eq!(parsed.temperature_c, Some(59));
        assert_eq!(parsed.free_vram_mb(), Some(9845));
        assert!(parse_nvidia_smi_line("bad,row").is_err());
    }

    #[test]
    fn admission_yields_for_busy_low_vram_hot_or_missing_gpu() {
        let policy = GpuPolicy::default();
        assert!(evaluate_admission(&policy, &telemetry(5, 2000, 12288, 55)).admit);
        let busy = evaluate_admission(&policy, &telemetry(70, 2000, 12288, 55));
        assert!(!busy.admit);
        assert!(busy.reasons.contains(&"gpu_busy".to_owned()));

        let low_vram = evaluate_admission(&policy, &telemetry(5, 11000, 12288, 55));
        assert!(!low_vram.admit);
        assert!(low_vram.reasons.contains(&"vram_low".to_owned()));

        let hot = evaluate_admission(&policy, &telemetry(5, 2000, 12288, 90));
        assert!(!hot.admit);
        assert!(hot.reasons.contains(&"gpu_hot".to_owned()));

        let unavailable = GpuTelemetry::unavailable("test", "no gpu");
        assert!(!evaluate_admission(&policy, &unavailable).admit);
    }

    #[test]
    fn policy_validation_is_bounded() {
        assert!(GpuPolicy {
            max_utilization_percent: 101,
            ..GpuPolicy::default()
        }
        .validate()
        .is_err());
        assert!(GpuPolicy {
            max_utilization_percent: 50,
            min_free_vram_mb: 0,
            ..GpuPolicy::default()
        }
        .validate()
        .is_err());
        assert!(GpuPolicy {
            max_temperature_c: 10,
            ..GpuPolicy::default()
        }
        .validate()
        .is_err());
    }
}

//! Real data assembly for both the HTTP dashboard and the terminal dashboard.
//!
//! Every value here is read from the Platform, Core installer, Core runtime,
//! module registry, or diagnostics APIs. Nothing is fabricated.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::SystemTime;

use aegis_contracts::integrity::IntegrityReport;
use aegis_contracts::status::PlatformStatus;
use aegis_core_install::CoreInstaller;
use aegis_core_runtime::{CoreRuntime, RuntimeStatus};
use aegis_diagnostics::{Diagnostics, DiagnosticsReport, OverallStatus};
use aegis_modules::{ModuleRecord, ModuleRegistry};
use aegis_platform::PlatformService;
use aegis_storage::StoragePaths;
use serde_json::{json, Value};

/// Stable reasons shared by the terminal renderer and the machine-readable
/// summary so every surface reports the same "unavailable" cause.
pub(crate) const FLEET_UNAVAILABLE_REASON: &str = "aegis-fleet is not a dependency of aegis-ui";
pub(crate) const GPU_UNAVAILABLE_REASON: &str = "aegis-fleet GPU telemetry is not linked";
pub(crate) const INFERENCE_UNAVAILABLE_REASON: &str =
    "aegis-cognition is not a dependency of aegis-ui";
pub(crate) const SERVICE_UNAVAILABLE_REASON: &str = "aegis-service is not a dependency of aegis-ui";
pub(crate) const MEMORY_UNAVAILABLE_REASON: &str = "no memory telemetry data source is linked";

/// Operator/CLI-written external status snapshots, relative to the Aegis root.
///
/// No CLI emitter exists yet; these files are optional. When absent, stale, or
/// malformed the affected section is reported as `UNAVAILABLE`/`STALE` rather
/// than fabricated. See `docs/implementation/BATCH7_AGENT05_TUI.md` for schemas.
pub(crate) const FLEET_STATUS_SNAPSHOT_PATH: &str = "run/status/fleet.json";
pub(crate) const GPU_STATUS_SNAPSHOT_PATH: &str = "run/status/gpu.json";
pub(crate) const INFERENCE_STATUS_SNAPSHOT_PATH: &str = "run/status/inference.json";
pub(crate) const SERVICE_STATUS_SNAPSHOT_PATH: &str = "run/status/service.json";
pub(crate) const WORLD_STATUS_SNAPSHOT_PATH: &str = "run/status/world.json";
pub(crate) const PERSONAL_STATUS_SNAPSHOT_PATH: &str = "run/status/personal.json";

pub(crate) const WORLD_UNAVAILABLE_REASON: &str = "no world status snapshot is available";
pub(crate) const PERSONAL_UNAVAILABLE_REASON: &str = "no personal status snapshot is available";

/// Snapshots older than this (by `updated_at`, else file mtime) are `STALE`.
pub(crate) const STATUS_SNAPSHOT_MAX_AGE_SECS: u64 = 60;

/// Freshness/validity of one external status snapshot.
///
/// A section is only populated from a snapshot when this is [`SnapshotState::Fresh`].
/// Every other state carries an explicit, human-readable reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SnapshotState {
    /// The snapshot file is present, recent, and passed validation.
    Fresh,
    /// The snapshot file is present but older than the staleness window.
    Stale(String),
    /// The snapshot file was not found.
    Missing(String),
    /// The snapshot file is present but unreadable, invalid JSON, or invalid data.
    Malformed(String),
}

impl SnapshotState {
    pub(crate) const fn is_fresh(&self) -> bool {
        matches!(self, Self::Fresh)
    }

    /// Machine status used by `summary_json`: `available`, `stale`, or `unavailable`.
    pub(crate) const fn health_str(&self) -> &'static str {
        match self {
            Self::Fresh => "available",
            Self::Stale(_) => "stale",
            Self::Missing(_) | Self::Malformed(_) => "unavailable",
        }
    }

    /// Console label used by the TUI when a section has no data.
    pub(crate) const fn label(&self) -> &'static str {
        match self {
            Self::Fresh | Self::Missing(_) | Self::Malformed(_) => "UNAVAILABLE",
            Self::Stale(_) => "STALE",
        }
    }

    pub(crate) fn reason(&self) -> Option<&str> {
        match self {
            Self::Fresh => None,
            Self::Stale(reason) | Self::Missing(reason) | Self::Malformed(reason) => Some(reason),
        }
    }
}

/// Runtime portion of one Core's read-only view.
#[derive(Debug, Clone)]
pub(crate) struct RuntimeView {
    pub state: String,
    pub heartbeat_fresh: bool,
    pub pid: Option<u32>,
    pub last_error: Option<String>,
    pub heartbeat_at: Option<String>,
}

impl RuntimeView {
    fn from_status(status: RuntimeStatus) -> Self {
        Self {
            state: status.state.as_str().to_owned(),
            heartbeat_fresh: status.heartbeat_fresh,
            pid: status.pid,
            last_error: status.last_error,
            heartbeat_at: status.heartbeat_at,
        }
    }

    fn stopped() -> Self {
        Self {
            state: "stopped".to_owned(),
            heartbeat_fresh: false,
            pid: None,
            last_error: None,
            heartbeat_at: None,
        }
    }
}

/// Integrity portion of one Core's read-only view.
#[derive(Debug, Clone)]
pub(crate) struct IntegrityView {
    pub status: String,
    pub detail: Option<String>,
    pub expected_sha256: Option<String>,
    pub actual_sha256: Option<String>,
    pub expected_schema_revision: Option<u64>,
    pub actual_schema_revision: Option<u64>,
}

impl IntegrityView {
    fn from_report(report: &IntegrityReport) -> Self {
        Self {
            status: report.status.as_str().to_owned(),
            detail: report.detail.clone(),
            expected_sha256: report.expected_sha256.clone(),
            actual_sha256: report.actual_sha256.clone(),
            expected_schema_revision: report.expected_schema_revision,
            actual_schema_revision: report.actual_schema_revision,
        }
    }

    fn unavailable(detail: String) -> Self {
        Self {
            status: "unavailable".to_owned(),
            detail: Some(detail),
            expected_sha256: None,
            actual_sha256: None,
            expected_schema_revision: None,
            actual_schema_revision: None,
        }
    }
}

/// Read-only view of one registered Core.
#[derive(Debug, Clone)]
pub(crate) struct CoreView {
    pub core_uuid: String,
    pub core_number: u8,
    pub display_name: String,
    pub lifecycle_state: String,
    pub runtime: RuntimeView,
    pub integrity: IntegrityView,
}

impl CoreView {
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "core_uuid": self.core_uuid,
            "core_number": self.core_number,
            "display_name": self.display_name,
            "lifecycle_state": self.lifecycle_state,
            "runtime": {
                "state": self.runtime.state,
                "heartbeat_fresh": self.runtime.heartbeat_fresh,
                "pid": self.runtime.pid,
                "last_error": self.runtime.last_error,
                "heartbeat_at": self.runtime.heartbeat_at,
            },
            "integrity": {
                "status": self.integrity.status,
                "detail": self.integrity.detail,
                "expected_sha256": self.integrity.expected_sha256,
                "actual_sha256": self.integrity.actual_sha256,
                "expected_schema_revision": self.integrity.expected_schema_revision,
                "actual_schema_revision": self.integrity.actual_schema_revision,
            }
        })
    }
}

/// Read-only module registry view, including whether the registry is usable.
#[derive(Debug, Clone)]
pub(crate) struct ModulesView {
    pub status: String,
    pub detail: Option<String>,
    pub modules: Vec<ModuleRecord>,
}

impl ModulesView {
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "status": self.status,
            "detail": self.detail,
            "modules": self.modules,
        })
    }
}

/// Three-level health classification shared by the TUI and any caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StatusClass {
    Ok,
    Warn,
    Error,
}

impl StatusClass {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }

    /// Lower-case, machine-readable form used by the headless summary.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

/// Severity of a surfaced issue. Kept separate from [`StatusClass`] so the
/// error/warning lists can be filtered without re-classifying a whole section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Severity {
    Error,
    Warn,
}

impl Severity {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
        }
    }
}

/// One surfaced error/warning with a stable code and human-readable message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Issue {
    pub(crate) severity: Severity,
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl Issue {
    fn error(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            code,
            message: message.into(),
        }
    }

    fn warn(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warn,
            code,
            message: message.into(),
        }
    }
}

/// One fleet host, mirroring the fields of `aegis_fleet::HostHealth`.
///
/// `aegis-fleet` is not a dependency of `aegis-ui`, so the live collector never
/// populates this. It exists so a future caller can hand real fleet data to the
/// renderer without inventing indicators.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FleetHostView {
    pub(crate) host_id: String,
    pub(crate) reachable: bool,
    pub(crate) model_count: u64,
    pub(crate) latency_ms: u64,
    pub(crate) error: Option<String>,
}

/// Fleet membership, populated only by a caller that has a fleet data source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FleetView {
    pub(crate) hosts: Vec<FleetHostView>,
}

/// GPU availability, mirroring `aegis_fleet::GpuTelemetry`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GpuView {
    pub(crate) available: bool,
    pub(crate) name: Option<String>,
    pub(crate) utilization_percent: Option<u8>,
    pub(crate) free_vram_mb: Option<u64>,
    pub(crate) temperature_c: Option<u8>,
    pub(crate) error: Option<String>,
}

/// Active inference placement. No inference data source is currently linked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InferenceView {
    pub(crate) active: bool,
    pub(crate) model: Option<String>,
    pub(crate) host_id: Option<String>,
    pub(crate) detail: Option<String>,
}

/// Platform service process status, mirroring `aegis_service::ServiceStatus`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServiceView {
    pub(crate) running: bool,
    pub(crate) pid: Option<u32>,
    pub(crate) pid_alive: bool,
    pub(crate) socket_reachable: bool,
    pub(crate) error: Option<String>,
}

/// Host memory telemetry. No memory data source is currently linked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MemoryView {
    pub(crate) used_bytes: u64,
    pub(crate) total_bytes: u64,
}

/// Shared world snapshot, populated only from an operator-written
/// `run/status/world.json` that is fresh and valid.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WorldView {
    pub(crate) local_time: Option<String>,
    pub(crate) timezone: Option<String>,
    pub(crate) time_of_day: Option<String>,
    pub(crate) season: Option<String>,
    pub(crate) condition: Option<String>,
    pub(crate) temperature_celsius: Option<f64>,
}

/// One Core's personal state, populated only from an operator-written
/// `run/status/personal.json` that is fresh and valid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PersonalView {
    pub(crate) core_uuid: Option<String>,
    pub(crate) emotion: Option<String>,
    pub(crate) continuity: Option<String>,
    pub(crate) activity_count: Option<usize>,
    pub(crate) routine_count: Option<usize>,
}

/// Database file status derived from `aegis-storage` paths and diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DatabaseView {
    pub(crate) platform_db_bytes: Option<u64>,
    pub(crate) safe_db_bytes: Option<u64>,
    pub(crate) core_db_bytes: Option<u64>,
    pub(crate) error: Option<String>,
}

/// Complete read-only dashboard picture assembled from real backend data.
///
/// Sections with `None` are explicitly unavailable because their data source
/// crate is not a dependency of `aegis-ui`; the renderer labels them
/// `UNAVAILABLE` rather than fabricating values.
#[derive(Debug, Clone)]
pub(crate) struct DashboardModel {
    pub(crate) platform: Option<PlatformStatus>,
    pub(crate) platform_error: Option<String>,
    pub(crate) cores: Vec<CoreView>,
    pub(crate) modules: ModulesView,
    pub(crate) diagnostics: Option<DiagnosticsReport>,
    pub(crate) diagnostics_error: Option<String>,
    pub(crate) database: DatabaseView,
    pub(crate) fleet: Option<FleetView>,
    pub(crate) gpu: Option<GpuView>,
    pub(crate) inference: Option<InferenceView>,
    pub(crate) service: Option<ServiceView>,
    pub(crate) memory: Option<MemoryView>,
    pub(crate) world: Option<WorldView>,
    pub(crate) personal: Option<PersonalView>,
    pub(crate) fleet_snapshot: SnapshotState,
    pub(crate) gpu_snapshot: SnapshotState,
    pub(crate) inference_snapshot: SnapshotState,
    pub(crate) service_snapshot: SnapshotState,
    pub(crate) world_snapshot: SnapshotState,
    pub(crate) personal_snapshot: SnapshotState,
    pub(crate) errors: Vec<String>,
    pub(crate) warnings: Vec<String>,
}

impl DashboardModel {
    /// Collect every section that the current dependencies can provide.
    pub(crate) fn collect(root: &Path) -> Self {
        let (platform, platform_error) = match collect_status(root) {
            Ok(status) => (Some(status), None),
            Err(error) => (None, Some(error)),
        };
        let cores = collect_cores(root);
        let modules = collect_modules(root);
        let (diagnostics, diagnostics_error) = match collect_diagnostics(root) {
            Ok(report) => (Some(report), None),
            Err(error) => (None, Some(error)),
        };
        let database = collect_database(root, diagnostics.as_ref());

        let (fleet_snapshot, fleet) = load_status_snapshot(
            root,
            FLEET_STATUS_SNAPSHOT_PATH,
            "Fleet",
            FLEET_UNAVAILABLE_REASON,
            parse_fleet_snapshot,
        );
        let (gpu_snapshot, gpu) = load_status_snapshot(
            root,
            GPU_STATUS_SNAPSHOT_PATH,
            "GPU",
            GPU_UNAVAILABLE_REASON,
            parse_gpu_snapshot,
        );
        let (inference_snapshot, inference) = load_status_snapshot(
            root,
            INFERENCE_STATUS_SNAPSHOT_PATH,
            "Inference",
            INFERENCE_UNAVAILABLE_REASON,
            parse_inference_snapshot,
        );
        let (service_snapshot, service) = load_status_snapshot(
            root,
            SERVICE_STATUS_SNAPSHOT_PATH,
            "Service",
            SERVICE_UNAVAILABLE_REASON,
            parse_service_snapshot,
        );
        let (world_snapshot, world) = load_status_snapshot(
            root,
            WORLD_STATUS_SNAPSHOT_PATH,
            "World",
            WORLD_UNAVAILABLE_REASON,
            parse_world_snapshot,
        );
        let (personal_snapshot, personal) = load_status_snapshot(
            root,
            PERSONAL_STATUS_SNAPSHOT_PATH,
            "Personal",
            PERSONAL_UNAVAILABLE_REASON,
            parse_personal_snapshot,
        );
        let memory = None;

        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        if diagnostics.is_none() {
            if let Some(error) = &platform_error {
                errors.push(format!("Platform is unavailable: {error}"));
            }
            if let Some(error) = &diagnostics_error {
                errors.push(format!("Diagnostics are unavailable: {error}"));
            }
        }
        if let Some(report) = &diagnostics {
            errors.extend(report.reasons.iter().cloned());
        }
        if modules.status != "available" {
            warnings.push(format!(
                "Module registry is {}: {}",
                modules.status,
                modules.detail.as_deref().unwrap_or("no detail")
            ));
        }
        if database.platform_db_bytes.is_none() {
            warnings.push("Platform database file is unavailable".to_owned());
        }
        push_snapshot_warning(&mut warnings, "Fleet", &fleet_snapshot);
        push_snapshot_warning(&mut warnings, "GPU", &gpu_snapshot);
        push_snapshot_warning(&mut warnings, "Inference", &inference_snapshot);
        push_snapshot_warning(&mut warnings, "Service", &service_snapshot);
        push_snapshot_warning(&mut warnings, "World", &world_snapshot);
        push_snapshot_warning(&mut warnings, "Personal", &personal_snapshot);
        if memory.is_none() {
            warnings.push(
                "Memory telemetry is unavailable (no memory data source is linked)".to_owned(),
            );
        }

        Self {
            platform,
            platform_error,
            cores,
            modules,
            diagnostics,
            diagnostics_error,
            database,
            fleet,
            gpu,
            inference,
            service,
            memory,
            world,
            personal,
            fleet_snapshot,
            gpu_snapshot,
            inference_snapshot,
            service_snapshot,
            world_snapshot,
            personal_snapshot,
            errors,
            warnings,
        }
    }
}

/// Load one optional external status snapshot.
///
/// The section is populated only when the file exists, parses, validates, and is
/// fresh (by `updated_at`, else file mtime). Every failure is reported as a
/// [`SnapshotState`] with a concrete reason; nothing is inferred.
fn load_status_snapshot<T>(
    root: &Path,
    relative_path: &str,
    label: &str,
    missing_reason: &str,
    parse: fn(&Value) -> Result<T, String>,
) -> (SnapshotState, Option<T>) {
    let path = root.join(relative_path);
    if !path.is_file() {
        return (SnapshotState::Missing(missing_reason.to_owned()), None);
    }
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return (
                SnapshotState::Malformed(format!(
                    "{label} status snapshot at {relative_path} is unreadable: {error}"
                )),
                None,
            )
        }
    };
    let value: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(error) => {
            return (
                SnapshotState::Malformed(format!(
                    "{label} status snapshot at {relative_path} is malformed JSON: {error}"
                )),
                None,
            )
        }
    };
    match snapshot_age_secs(&value, &path) {
        Ok(age) if age > STATUS_SNAPSHOT_MAX_AGE_SECS as i64 => (
            SnapshotState::Stale(format!(
                "{label} status snapshot at {relative_path} is stale ({age}s old; limit {STATUS_SNAPSHOT_MAX_AGE_SECS}s)"
            )),
            None,
        ),
        Ok(_) => match parse(&value) {
            Ok(view) => (SnapshotState::Fresh, Some(view)),
            Err(error) => (
                SnapshotState::Malformed(format!(
                    "{label} status snapshot at {relative_path} failed validation: {error}"
                )),
                None,
            ),
        },
        Err(error) => (
            SnapshotState::Malformed(format!(
                "{label} status snapshot at {relative_path} has an unusable timestamp: {error}"
            )),
            None,
        ),
    }
}

fn push_snapshot_warning(warnings: &mut Vec<String>, label: &str, state: &SnapshotState) {
    if state.is_fresh() {
        return;
    }
    let Some(reason) = state.reason() else {
        return;
    };
    warnings.push(format!("{label} data is {}: {reason}", state.health_str()));
}

/// Age in seconds from `updated_at` (RFC 3339) or, when absent, the file mtime.
fn snapshot_age_secs(value: &Value, path: &Path) -> Result<i64, String> {
    match value.get("updated_at") {
        None | Some(Value::Null) => mtime_age_secs(path),
        Some(raw) => {
            let text = raw
                .as_str()
                .ok_or_else(|| "'updated_at' is not a string".to_owned())?;
            let parsed = chrono::DateTime::parse_from_rfc3339(text)
                .map_err(|error| format!("'updated_at' is not RFC 3339: {error}"))?;
            Ok((chrono::Utc::now() - parsed.with_timezone(&chrono::Utc)).num_seconds())
        }
    }
}

fn mtime_age_secs(path: &Path) -> Result<i64, String> {
    let modified = std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .map_err(|error| format!("file mtime is unavailable: {error}"))?;
    match SystemTime::now().duration_since(modified) {
        Ok(age) => Ok(age.as_secs() as i64),
        // A future mtime is treated as not-yet-stale rather than panicking.
        Err(_) => Ok(-1),
    }
}

fn require_bool(value: &Value, key: &str) -> Result<bool, String> {
    value
        .get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("missing boolean '{key}'"))
}

fn require_str<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing string '{key}'"))
}

fn optional_str(value: &Value, key: &str) -> Result<Option<String>, String> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(raw) => raw
            .as_str()
            .map(|text| Some(text.to_owned()))
            .ok_or_else(|| format!("'{key}' must be a string or null")),
    }
}

fn optional_u64(value: &Value, key: &str) -> Result<Option<u64>, String> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(raw) => raw
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("'{key}' must be a non-negative integer or null")),
    }
}

fn optional_u8(value: &Value, key: &str) -> Result<Option<u8>, String> {
    match optional_u64(value, key)? {
        None => Ok(None),
        Some(number) => u8::try_from(number)
            .map(Some)
            .map_err(|_| format!("'{key}' is out of range (0-255)")),
    }
}

fn optional_u32(value: &Value, key: &str) -> Result<Option<u32>, String> {
    match optional_u64(value, key)? {
        None => Ok(None),
        Some(number) => u32::try_from(number)
            .map(Some)
            .map_err(|_| format!("'{key}' is out of range")),
    }
}

fn parse_fleet_snapshot(value: &Value) -> Result<FleetView, String> {
    let hosts = value
        .get("hosts")
        .and_then(Value::as_array)
        .ok_or_else(|| "missing 'hosts' array".to_owned())?;
    let mut parsed = Vec::with_capacity(hosts.len());
    for (index, host) in hosts.iter().enumerate() {
        let host_id =
            require_str(host, "host_id").map_err(|error| format!("hosts[{index}] {error}"))?;
        let reachable =
            require_bool(host, "reachable").map_err(|error| format!("hosts[{index}] {error}"))?;
        parsed.push(FleetHostView {
            host_id: host_id.to_owned(),
            reachable,
            model_count: optional_u64(host, "model_count")?.unwrap_or(0),
            latency_ms: optional_u64(host, "latency_ms")?.unwrap_or(0),
            error: optional_str(host, "error")?,
        });
    }
    Ok(FleetView { hosts: parsed })
}

fn parse_gpu_snapshot(value: &Value) -> Result<GpuView, String> {
    Ok(GpuView {
        available: require_bool(value, "available")?,
        name: optional_str(value, "name")?,
        utilization_percent: optional_u8(value, "utilization_percent")?,
        free_vram_mb: optional_u64(value, "free_vram_mb")?,
        temperature_c: optional_u8(value, "temperature_c")?,
        error: optional_str(value, "error")?,
    })
}

fn parse_inference_snapshot(value: &Value) -> Result<InferenceView, String> {
    Ok(InferenceView {
        active: require_bool(value, "active")?,
        model: optional_str(value, "model")?,
        host_id: optional_str(value, "host_id")?,
        detail: optional_str(value, "detail")?,
    })
}

fn parse_service_snapshot(value: &Value) -> Result<ServiceView, String> {
    Ok(ServiceView {
        running: require_bool(value, "running")?,
        pid: optional_u32(value, "pid")?,
        pid_alive: require_bool(value, "pid_alive")?,
        socket_reachable: require_bool(value, "socket_reachable")?,
        error: optional_str(value, "error")?,
    })
}

fn optional_f64(value: &Value, key: &str) -> Result<Option<f64>, String> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(raw) => raw
            .as_f64()
            .map(Some)
            .ok_or_else(|| format!("'{key}' must be a finite number or null")),
    }
}

fn optional_array_len(value: &Value, key: &str) -> Result<Option<usize>, String> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => Ok(Some(items.len())),
        Some(_) => Err(format!("'{key}' must be an array or null")),
    }
}

fn optional_object<'a>(value: &'a Value, key: &str) -> Result<Option<&'a Value>, String> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(raw) if raw.is_object() => Ok(Some(raw)),
        Some(_) => Err(format!("'{key}' must be an object or null")),
    }
}

fn parse_world_snapshot(value: &Value) -> Result<WorldView, String> {
    if !value.is_object() {
        return Err("world snapshot must be a JSON object".to_owned());
    }
    let weather = optional_object(value, "weather")?;

    let condition = match optional_str(value, "condition")? {
        Some(condition) => Some(condition),
        None => match weather {
            Some(weather) => optional_str(weather, "condition")?,
            None => None,
        },
    };
    let temperature_celsius = match optional_f64(value, "temperature_celsius")? {
        Some(temperature) => Some(temperature),
        None => match weather {
            Some(weather) => optional_f64(weather, "temperature_celsius")?,
            None => None,
        },
    };

    Ok(WorldView {
        local_time: optional_str(value, "local_time")?,
        timezone: optional_str(value, "timezone")?,
        time_of_day: optional_str(value, "time_of_day")?,
        season: optional_str(value, "season")?,
        condition,
        temperature_celsius,
    })
}

fn format_emotion(value: &Value) -> Result<String, String> {
    let mut parts = Vec::new();
    for key in ["valence", "arousal", "joy", "calm", "curiosity", "tension"] {
        if let Some(dimension) = optional_f64(value, key)? {
            parts.push(format!("{key}={dimension:.2}"));
        }
    }
    if parts.is_empty() {
        Ok("no emotion dimensions reported".to_owned())
    } else {
        Ok(parts.join(" "))
    }
}

fn format_continuity(value: &Value) -> Result<String, String> {
    if let Some(summary) = optional_str(value, "summary")? {
        return Ok(summary);
    }
    if let Some(experiences) = optional_array_len(value, "recent_experiences")? {
        return Ok(format!("{experiences} recent experience(s)"));
    }
    if let Some(elapsed) = optional_u64(value, "elapsed_offline_seconds")? {
        return Ok(format!("{elapsed}s elapsed offline"));
    }
    Ok("no continuity summary reported".to_owned())
}

fn parse_personal_snapshot(value: &Value) -> Result<PersonalView, String> {
    if !value.is_object() {
        return Err("personal snapshot must be a JSON object".to_owned());
    }

    let emotion = match optional_object(value, "emotion")? {
        Some(emotion) => Some(format_emotion(emotion)?),
        None => optional_str(value, "emotion_summary")?,
    };
    let continuity = match optional_object(value, "continuity")? {
        Some(continuity) => Some(format_continuity(continuity)?),
        None => optional_str(value, "continuity_summary")?,
    };

    Ok(PersonalView {
        core_uuid: optional_str(value, "core_uuid")?,
        emotion,
        continuity,
        activity_count: optional_array_len(value, "activities")?,
        routine_count: optional_array_len(value, "routines")?,
    })
}

fn collect_database(root: &Path, diagnostics: Option<&DiagnosticsReport>) -> DatabaseView {
    let paths = match StoragePaths::new(root) {
        Ok(paths) => paths,
        Err(error) => {
            return DatabaseView {
                platform_db_bytes: None,
                safe_db_bytes: None,
                core_db_bytes: None,
                error: Some(error.to_string()),
            }
        }
    };
    let platform_db_bytes = file_len(&paths.platform_db());
    let safe_db_bytes = file_len(&paths.safe_db());
    let core_db_bytes = diagnostics.map(|report| {
        report
            .cores
            .entries
            .iter()
            .filter_map(|core| core.database_bytes)
            .sum::<u64>()
    });
    DatabaseView {
        platform_db_bytes,
        safe_db_bytes,
        core_db_bytes,
        error: None,
    }
}

fn file_len(path: &Path) -> Option<u64> {
    std::fs::metadata(path)
        .ok()
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len())
}

/// Classify overall Platform health from status and diagnostics.
pub(crate) fn classify_platform(
    platform: Option<&PlatformStatus>,
    diagnostics: Option<&DiagnosticsReport>,
) -> StatusClass {
    let Some(status) = platform else {
        return StatusClass::Error;
    };
    if status.stop_active {
        return StatusClass::Error;
    }
    match diagnostics {
        Some(report) => match report.status {
            OverallStatus::Healthy => StatusClass::Ok,
            OverallStatus::Degraded => StatusClass::Warn,
            OverallStatus::Critical => StatusClass::Error,
        },
        None => StatusClass::Warn,
    }
}

/// Classify one Core from its real runtime and integrity fields.
pub(crate) fn classify_core(runtime: &RuntimeView, integrity: &IntegrityView) -> StatusClass {
    if integrity.status != "verified" {
        return StatusClass::Error;
    }
    match runtime.state.as_str() {
        "failed" => StatusClass::Error,
        "running" if runtime.heartbeat_fresh => StatusClass::Ok,
        "running" | "starting" | "stopping" | "stale" => StatusClass::Warn,
        _ => StatusClass::Ok,
    }
}

/// Classify the service process from its real status fields.
pub(crate) fn classify_service(service: Option<&ServiceView>) -> StatusClass {
    match service {
        None => StatusClass::Warn,
        Some(service) => {
            if service.error.is_some() {
                StatusClass::Error
            } else if service.running && service.pid_alive && service.socket_reachable {
                StatusClass::Ok
            } else {
                StatusClass::Warn
            }
        }
    }
}

/// Classify GPU availability from real telemetry fields.
pub(crate) fn classify_gpu(gpu: Option<&GpuView>) -> StatusClass {
    match gpu {
        None => StatusClass::Warn,
        Some(gpu) if !gpu.available => StatusClass::Warn,
        Some(gpu) => {
            if gpu.temperature_c.is_some_and(|value| value >= 80) {
                StatusClass::Error
            } else if gpu.utilization_percent.is_some_and(|value| value >= 90) {
                StatusClass::Warn
            } else {
                StatusClass::Ok
            }
        }
    }
}

/// Classify Fleet health from per-host reachability.
pub(crate) fn classify_fleet(fleet: Option<&FleetView>) -> StatusClass {
    let Some(fleet) = fleet else {
        return StatusClass::Warn;
    };
    if fleet.hosts.is_empty() {
        return StatusClass::Warn;
    }
    let unreachable = fleet.hosts.iter().filter(|host| !host.reachable).count();
    if unreachable == fleet.hosts.len() {
        StatusClass::Error
    } else if unreachable > 0 {
        StatusClass::Warn
    } else {
        StatusClass::Ok
    }
}

/// Classify database availability from real file sizes.
pub(crate) fn classify_database(database: &DatabaseView) -> StatusClass {
    if database.error.is_some() {
        StatusClass::Error
    } else if database.platform_db_bytes.is_some() {
        StatusClass::Ok
    } else {
        StatusClass::Warn
    }
}

/// Whether a runtime state means the Core is expected to be serving.
fn runtime_is_active(state: &str) -> bool {
    matches!(state, "running" | "starting" | "stopping")
}

/// Errors and warnings derived from real per-Core fields: integrity failures,
/// runtime failures, and runtime errors surfaced by the Core runtime store.
pub(crate) fn core_issues(cores: &[CoreView]) -> Vec<Issue> {
    let mut issues = Vec::new();
    for core in cores {
        if core.integrity.status != "verified" {
            let detail = core.integrity.detail.as_deref().unwrap_or("no detail");
            issues.push(Issue::error(
                "integrity_failure",
                format!(
                    "Core {} integrity is {} ({detail})",
                    core.core_uuid, core.integrity.status
                ),
            ));
        }
        if let Some(error) = &core.runtime.last_error {
            issues.push(Issue::error(
                "runtime_error",
                format!("Core {} runtime error: {error}", core.core_uuid),
            ));
        } else if core.runtime.state == "failed" {
            issues.push(Issue::error(
                "runtime_failed",
                format!("Core {} runtime has failed", core.core_uuid),
            ));
        }
    }
    issues
}

/// Contradictions between otherwise-valid model fields, surfaced as warnings so
/// diagnostics can flag inconsistent snapshots without inventing data.
pub(crate) fn consistency_warnings(model: &DashboardModel) -> Vec<Issue> {
    let mut issues = Vec::new();
    let stop_active = model
        .platform
        .as_ref()
        .is_some_and(|status| status.stop_active);

    if let Some(platform) = &model.platform {
        if platform.stop_active {
            issues.push(Issue::warn(
                "stop_active",
                "STOP is active; Core operations must remain halted",
            ));
        }
        if platform.zero_core && !model.cores.is_empty() {
            issues.push(Issue::warn(
                "zero_core_contradiction",
                format!(
                    "Platform reports zero Cores but {} Cores are registered",
                    model.cores.len()
                ),
            ));
        }
        if platform.core_count as usize != model.cores.len() {
            issues.push(Issue::warn(
                "core_count_mismatch",
                format!(
                    "Platform reports {} Cores but {} are registered",
                    platform.core_count,
                    model.cores.len()
                ),
            ));
        }
    }

    if model.cores.is_empty() {
        issues.push(Issue::warn("no_cores", "No Cores are registered"));
    }

    let mut seen_numbers = HashSet::new();
    for core in &model.cores {
        if !seen_numbers.insert(core.core_number) {
            issues.push(Issue::warn(
                "duplicate_core_number",
                format!(
                    "Core number {} is assigned to more than one Core",
                    core.core_number
                ),
            ));
        }
        if runtime_is_active(&core.runtime.state) && !core.runtime.heartbeat_fresh {
            let kind = if core.runtime.heartbeat_at.is_none() {
                "absent"
            } else {
                "stale"
            };
            issues.push(Issue::warn(
                "heartbeat_not_fresh",
                format!(
                    "Core {} runtime is {} but its heartbeat is {kind}",
                    core.core_uuid, core.runtime.state
                ),
            ));
        }
        if stop_active && runtime_is_active(&core.runtime.state) {
            issues.push(Issue::warn(
                "stop_with_active_core",
                format!(
                    "STOP is active but Core {} runtime is {}",
                    core.core_uuid, core.runtime.state
                ),
            ));
        }
    }

    issues
}

/// Full error/warning picture: caller-supplied plus real per-Core issues plus
/// consistency contradictions, each with a severity.
pub(crate) fn issues(model: &DashboardModel) -> Vec<Issue> {
    let mut issues = Vec::new();
    issues.extend(
        model
            .errors
            .iter()
            .map(|message| Issue::error("reported_error", message.clone())),
    );
    issues.extend(
        model
            .warnings
            .iter()
            .map(|message| Issue::warn("reported_warning", message.clone())),
    );
    issues.extend(core_issues(&model.cores));
    issues.extend(consistency_warnings(model));
    issues
}

fn worst(first: StatusClass, second: StatusClass) -> StatusClass {
    match (first, second) {
        (StatusClass::Error, _) | (_, StatusClass::Error) => StatusClass::Error,
        (StatusClass::Warn, _) | (_, StatusClass::Warn) => StatusClass::Warn,
        (StatusClass::Ok, StatusClass::Ok) => StatusClass::Ok,
    }
}

/// Operational health derived only from real signals. Sections whose data
/// source is unavailable do not degrade health; they are reported separately.
pub(crate) fn overall_class(model: &DashboardModel) -> StatusClass {
    let mut class = classify_platform(model.platform.as_ref(), model.diagnostics.as_ref());
    for core in &model.cores {
        class = worst(class, classify_core(&core.runtime, &core.integrity));
    }
    class = worst(class, classify_database(&model.database));
    if let Some(service) = &model.service {
        class = worst(class, classify_service(Some(service)));
    }
    if let Some(gpu) = &model.gpu {
        class = worst(class, classify_gpu(Some(gpu)));
    }
    if let Some(fleet) = &model.fleet {
        class = worst(class, classify_fleet(Some(fleet)));
    }
    class
}

fn unavailable_section(reason: &str) -> Value {
    json!({
        "status": "unavailable",
        "unavailable_reason": reason,
    })
}

fn section_status_json(state: &SnapshotState) -> Value {
    json!({
        "status": state.health_str(),
        "unavailable_reason": state.reason(),
    })
}

fn issue_json(issue: &Issue) -> Value {
    json!({
        "severity": issue.severity.as_str(),
        "code": issue.code,
        "message": issue.message,
    })
}

/// Machine-readable snapshot shared by the CLI, TUI, and diagnostics.
///
/// Every section carries a `status` of `available`, `none`, or `unavailable`;
/// unavailable sections always carry an explicit `unavailable_reason` instead of
/// fabricated values. Errors and warnings are classified by severity and
/// consistency contradictions are listed separately.
pub(crate) fn summary_json(model: &DashboardModel) -> Value {
    let all_issues = issues(model);
    let errors: Vec<Value> = all_issues
        .iter()
        .filter(|issue| issue.severity == Severity::Error)
        .map(issue_json)
        .collect();
    let warnings: Vec<Value> = all_issues
        .iter()
        .filter(|issue| issue.severity == Severity::Warn)
        .map(issue_json)
        .collect();
    let consistency: Vec<Value> = consistency_warnings(model).iter().map(issue_json).collect();

    let generated_at = model
        .platform
        .as_ref()
        .map(|status| status.generated_at.clone())
        .or_else(|| {
            model
                .diagnostics
                .as_ref()
                .map(|report| report.generated_at.clone())
        });

    let cores: Vec<Value> = model
        .cores
        .iter()
        .map(|core| {
            let class = classify_core(&core.runtime, &core.integrity);
            json!({
                "core_uuid": core.core_uuid,
                "core_number": core.core_number,
                "display_name": core.display_name,
                "lifecycle_state": core.lifecycle_state,
                "health": class.as_str(),
                "runtime": {
                    "state": core.runtime.state,
                    "heartbeat_fresh": core.runtime.heartbeat_fresh,
                    "heartbeat_at": core.runtime.heartbeat_at,
                    "pid": core.runtime.pid,
                    "last_error": core.runtime.last_error,
                },
                "integrity": {
                    "status": core.integrity.status,
                    "detail": core.integrity.detail,
                    "expected_sha256": core.integrity.expected_sha256,
                    "actual_sha256": core.integrity.actual_sha256,
                    "expected_schema_revision": core.integrity.expected_schema_revision,
                    "actual_schema_revision": core.integrity.actual_schema_revision,
                },
            })
        })
        .collect();

    let platform = match &model.platform {
        Some(status) => json!({
            "platform_id": status.platform_id,
            "callsign": status.callsign,
            "zero_core": status.zero_core,
            "core_count": status.core_count,
            "stop_active": status.stop_active,
            "authority": status.authority,
            "generated_at": status.generated_at,
        }),
        None => Value::Null,
    };

    let diagnostics = match &model.diagnostics {
        Some(report) => json!({
            "status": "available",
            "unavailable_reason": Value::Null,
            "overall": report.status.as_str(),
            "reasons": report.reasons,
            "generated_at": report.generated_at,
        }),
        None => json!({
            "status": "unavailable",
            "unavailable_reason": model.diagnostics_error,
            "overall": Value::Null,
            "reasons": Vec::<String>::new(),
            "generated_at": Value::Null,
        }),
    };

    let database = if let Some(error) = &model.database.error {
        json!({
            "status": "error",
            "unavailable_reason": error,
            "health": StatusClass::Error.as_str(),
            "platform_db_bytes": model.database.platform_db_bytes,
            "safe_db_bytes": model.database.safe_db_bytes,
            "core_db_bytes": model.database.core_db_bytes,
        })
    } else {
        let available = model.database.platform_db_bytes.is_some();
        json!({
            "status": if available { "available" } else { "unavailable" },
            "unavailable_reason": if available {
                Value::Null
            } else {
                Value::String("platform database file is unavailable".to_owned())
            },
            "health": classify_database(&model.database).as_str(),
            "platform_db_bytes": model.database.platform_db_bytes,
            "safe_db_bytes": model.database.safe_db_bytes,
            "core_db_bytes": model.database.core_db_bytes,
        })
    };

    let fleet = match &model.fleet {
        None => section_status_json(&model.fleet_snapshot),
        Some(fleet) => json!({
            "status": if fleet.hosts.is_empty() { "none" } else { "available" },
            "unavailable_reason": Value::Null,
            "health": classify_fleet(Some(fleet)).as_str(),
            "hosts": fleet
                .hosts
                .iter()
                .map(|host| json!({
                    "host_id": host.host_id,
                    "reachable": host.reachable,
                    "model_count": host.model_count,
                    "latency_ms": host.latency_ms,
                    "error": host.error,
                }))
                .collect::<Vec<_>>(),
        }),
    };

    let gpu = match &model.gpu {
        None => section_status_json(&model.gpu_snapshot),
        Some(gpu) if !gpu.available => json!({
            "status": "unavailable",
            "unavailable_reason": gpu
                .error
                .clone()
                .unwrap_or_else(|| GPU_UNAVAILABLE_REASON.to_owned()),
            "health": classify_gpu(Some(gpu)).as_str(),
        }),
        Some(gpu) => json!({
            "status": "available",
            "unavailable_reason": Value::Null,
            "health": classify_gpu(Some(gpu)).as_str(),
            "name": gpu.name,
            "utilization_percent": gpu.utilization_percent,
            "free_vram_mb": gpu.free_vram_mb,
            "temperature_c": gpu.temperature_c,
        }),
    };

    let inference = match &model.inference {
        None => section_status_json(&model.inference_snapshot),
        Some(inference) if !inference.active => json!({
            "status": "none",
            "unavailable_reason": Value::Null,
            "active": false,
            "model": Value::Null,
            "host_id": Value::Null,
            "detail": inference.detail,
        }),
        Some(inference) => json!({
            "status": "available",
            "unavailable_reason": Value::Null,
            "active": true,
            "model": inference.model,
            "host_id": inference.host_id,
            "detail": inference.detail,
        }),
    };

    let service = match &model.service {
        None => section_status_json(&model.service_snapshot),
        Some(service) => json!({
            "status": "available",
            "unavailable_reason": Value::Null,
            "health": classify_service(Some(service)).as_str(),
            "running": service.running,
            "pid": service.pid,
            "pid_alive": service.pid_alive,
            "socket_reachable": service.socket_reachable,
            "error": service.error,
        }),
    };

    let memory = match &model.memory {
        None => unavailable_section(MEMORY_UNAVAILABLE_REASON),
        Some(memory) if memory.total_bytes == 0 => json!({
            "status": "none",
            "unavailable_reason": Value::Null,
            "used_bytes": memory.used_bytes,
            "total_bytes": memory.total_bytes,
        }),
        Some(memory) => json!({
            "status": "available",
            "unavailable_reason": Value::Null,
            "used_bytes": memory.used_bytes,
            "total_bytes": memory.total_bytes,
            "used_percent": if memory.total_bytes == 0 {
                0
            } else {
                memory.used_bytes.saturating_mul(100) / memory.total_bytes
            },
        }),
    };

    json!({
        "schema": "aegis.ui.summary.v1",
        "generated_at": generated_at,
        "overall_health": overall_class(model).as_str(),
        "platform": {
            "status": if model.platform.is_some() { "available" } else { "unavailable" },
            "unavailable_reason": model.platform_error,
            "health": classify_platform(model.platform.as_ref(), model.diagnostics.as_ref()).as_str(),
            "data": platform,
        },
        "cores": {
            "status": "available",
            "unavailable_reason": Value::Null,
            "count": model.cores.len(),
            "items": cores,
        },
        "modules": {
            "status": model.modules.status,
            "unavailable_reason": model.modules.detail,
            "count": model.modules.modules.len(),
            "items": model.modules.modules,
        },
        "diagnostics": diagnostics,
        "database": database,
        "fleet": fleet,
        "gpu": gpu,
        "inference": inference,
        "service": service,
        "memory": memory,
        "issues": {
            "errors": errors,
            "warnings": warnings,
        },
        "consistency": consistency,
    })
}

/// Collect a real snapshot and return the shared machine-readable summary.
pub fn render_summary_json(root: impl AsRef<Path>) -> Value {
    summary_json(&DashboardModel::collect(root.as_ref()))
}

/// Read the Platform status without ever reading the owner token.
pub(crate) fn collect_status(root: &Path) -> Result<PlatformStatus, String> {
    let service = PlatformService::start(root).map_err(|error| error.to_string())?;
    service.status().map_err(|error| error.to_string())
}

/// Read the full diagnostics report.
pub(crate) fn collect_diagnostics(root: &Path) -> Result<DiagnosticsReport, String> {
    let diagnostics = Diagnostics::new(root).map_err(|error| error.to_string())?;
    Ok(diagnostics.collect())
}

/// Read every registered Core. A single damaged Core never aborts the list.
pub(crate) fn collect_cores(root: &Path) -> Vec<CoreView> {
    let paths = match StoragePaths::new(root) {
        Ok(paths) => paths,
        Err(_) => return Vec::new(),
    };
    if !paths.platform_db().is_file() {
        return Vec::new();
    }
    let installer = match CoreInstaller::new(root) {
        Ok(installer) => installer,
        Err(_) => return Vec::new(),
    };
    let entries = match installer.list() {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };
    let runtime = collect_runtime(root);

    entries
        .into_iter()
        .map(|entry| {
            let runtime_view = runtime
                .get(&entry.core_uuid)
                .cloned()
                .unwrap_or_else(RuntimeView::stopped);
            let integrity = match installer.inspect(&entry.core_uuid) {
                Ok(inspection) => IntegrityView::from_report(&inspection.integrity),
                Err(error) => IntegrityView::unavailable(error.to_string()),
            };
            CoreView {
                core_uuid: entry.core_uuid,
                core_number: entry.core_number,
                display_name: entry.display_name,
                lifecycle_state: entry.lifecycle_state.as_str().to_owned(),
                runtime: runtime_view,
                integrity,
            }
        })
        .collect()
}

fn collect_runtime(root: &Path) -> HashMap<String, RuntimeView> {
    let paths = match StoragePaths::new(root) {
        Ok(paths) => paths,
        Err(_) => return HashMap::new(),
    };
    match CoreRuntime::new(paths).list() {
        Ok(statuses) => statuses
            .into_iter()
            .map(|status| {
                let core_uuid = status.core_uuid.clone();
                let view = RuntimeView::from_status(status);
                (core_uuid, view)
            })
            .collect(),
        Err(_) => HashMap::new(),
    }
}

/// Read module records. A missing registry is reported as unavailable, not an
/// error, so the dashboard can state the truth without failing.
pub(crate) fn collect_modules(root: &Path) -> ModulesView {
    let paths = match StoragePaths::new(root) {
        Ok(paths) => paths,
        Err(error) => {
            return ModulesView {
                status: "unavailable".to_owned(),
                detail: Some(error.to_string()),
                modules: Vec::new(),
            }
        }
    };
    if !paths.platform_db().is_file() {
        return ModulesView {
            status: "unavailable".to_owned(),
            detail: Some("Platform database is missing".to_owned()),
            modules: Vec::new(),
        };
    }
    match ModuleRegistry::new(paths).list() {
        Ok(modules) => ModulesView {
            status: "available".to_owned(),
            detail: None,
            modules,
        },
        Err(error) => ModulesView {
            status: "unavailable".to_owned(),
            detail: Some(error.to_string()),
            modules: Vec::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn platform(stop_active: bool) -> PlatformStatus {
        PlatformStatus {
            schema: "aegis.platform.status.v1".to_owned(),
            protocol: "aegis.platform.v1".to_owned(),
            platform_id: "platform".to_owned(),
            callsign: "Aegis".to_owned(),
            zero_core: false,
            core_count: 1,
            stop_active,
            authority: "local_single".to_owned(),
            generated_at: "2026-11-25T00:00:00Z".to_owned(),
        }
    }

    fn diagnostics(status: &str) -> DiagnosticsReport {
        serde_json::from_value(json!({
            "schema": "aegis.diagnostics.report.v1",
            "generated_at": "2026-11-25T00:00:00Z",
            "status": status,
            "reasons": [],
            "platform": {
                "available": true,
                "platform_id": "platform",
                "callsign": "Aegis",
                "zero_core": false,
                "core_count": 1,
                "stop_active": false,
                "authority": "local_single",
                "error": null
            },
            "safe": {
                "available": true,
                "owner_proof_present": true,
                "stop_active": false,
                "stop_revision": 1,
                "stop_updated_at": "2026-11-25T00:00:00Z",
                "stop_updated_by": "aegis",
                "stop_reason": "",
                "error": null
            },
            "cores": {
                "available": true,
                "count": 0,
                "error": null,
                "entries": []
            },
            "modules": {
                "status": "available",
                "count": 0,
                "detail": null
            }
        }))
        .expect("diagnostics report")
    }

    fn runtime(state: &str, heartbeat_fresh: bool) -> RuntimeView {
        RuntimeView {
            state: state.to_owned(),
            heartbeat_fresh,
            pid: Some(1234),
            last_error: None,
            heartbeat_at: None,
        }
    }

    fn integrity(status: &str) -> IntegrityView {
        IntegrityView {
            status: status.to_owned(),
            detail: None,
            expected_sha256: None,
            actual_sha256: None,
            expected_schema_revision: None,
            actual_schema_revision: None,
        }
    }

    const CORE_UUID: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";

    fn core(runtime: RuntimeView, integrity: IntegrityView) -> CoreView {
        CoreView {
            core_uuid: CORE_UUID.to_owned(),
            core_number: 1,
            display_name: "Core A".to_owned(),
            lifecycle_state: "installed".to_owned(),
            runtime,
            integrity,
        }
    }

    fn dashboard_model() -> DashboardModel {
        DashboardModel {
            platform: Some(platform(false)),
            platform_error: None,
            cores: vec![core(runtime("running", true), integrity("verified"))],
            modules: ModulesView {
                status: "available".to_owned(),
                detail: None,
                modules: Vec::new(),
            },
            diagnostics: Some(diagnostics("healthy")),
            diagnostics_error: None,
            database: DatabaseView {
                platform_db_bytes: Some(4096),
                safe_db_bytes: Some(1024),
                core_db_bytes: Some(8192),
                error: None,
            },
            fleet: Some(FleetView {
                hosts: vec![FleetHostView {
                    host_id: "host-a".to_owned(),
                    reachable: true,
                    model_count: 2,
                    latency_ms: 4,
                    error: None,
                }],
            }),
            gpu: Some(GpuView {
                available: true,
                name: Some("RTX Test".to_owned()),
                utilization_percent: Some(10),
                free_vram_mb: Some(6144),
                temperature_c: Some(50),
                error: None,
            }),
            inference: Some(InferenceView {
                active: true,
                model: Some("aegis-small".to_owned()),
                host_id: Some("host-a".to_owned()),
                detail: None,
            }),
            service: Some(ServiceView {
                running: true,
                pid: Some(99),
                pid_alive: true,
                socket_reachable: true,
                error: None,
            }),
            memory: Some(MemoryView {
                used_bytes: 2048,
                total_bytes: 8192,
            }),
            world: Some(WorldView {
                local_time: Some("08:30".to_owned()),
                timezone: Some("UTC".to_owned()),
                time_of_day: Some("morning".to_owned()),
                season: Some("autumn".to_owned()),
                condition: Some("clear".to_owned()),
                temperature_celsius: Some(12.5),
            }),
            personal: Some(PersonalView {
                core_uuid: Some(CORE_UUID.to_owned()),
                emotion: Some("calm=0.80".to_owned()),
                continuity: Some("kept a journal".to_owned()),
                activity_count: Some(2),
                routine_count: Some(1),
            }),
            fleet_snapshot: SnapshotState::Fresh,
            gpu_snapshot: SnapshotState::Fresh,
            inference_snapshot: SnapshotState::Fresh,
            service_snapshot: SnapshotState::Fresh,
            world_snapshot: SnapshotState::Fresh,
            personal_snapshot: SnapshotState::Fresh,
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn codes(issues: &[Issue]) -> Vec<&'static str> {
        issues.iter().map(|issue| issue.code).collect()
    }

    #[test]
    fn platform_classification_reflects_stop_and_diagnostics() {
        assert_eq!(
            classify_platform(None, Some(&diagnostics("healthy"))),
            StatusClass::Error
        );
        assert_eq!(
            classify_platform(Some(&platform(true)), Some(&diagnostics("healthy"))),
            StatusClass::Error
        );
        assert_eq!(
            classify_platform(Some(&platform(false)), Some(&diagnostics("healthy"))),
            StatusClass::Ok
        );
        assert_eq!(
            classify_platform(Some(&platform(false)), Some(&diagnostics("degraded"))),
            StatusClass::Warn
        );
        assert_eq!(
            classify_platform(Some(&platform(false)), Some(&diagnostics("critical"))),
            StatusClass::Error
        );
        assert_eq!(
            classify_platform(Some(&platform(false)), None),
            StatusClass::Warn
        );
    }

    #[test]
    fn core_classification_uses_integrity_then_runtime() {
        assert_eq!(
            classify_core(&runtime("running", true), &integrity("verified")),
            StatusClass::Ok
        );
        assert_eq!(
            classify_core(&runtime("stopped", false), &integrity("verified")),
            StatusClass::Ok
        );
        assert_eq!(
            classify_core(&runtime("running", false), &integrity("verified")),
            StatusClass::Warn
        );
        assert_eq!(
            classify_core(&runtime("stale", false), &integrity("verified")),
            StatusClass::Warn
        );
        assert_eq!(
            classify_core(&runtime("failed", false), &integrity("verified")),
            StatusClass::Error
        );
        assert_eq!(
            classify_core(&runtime("running", true), &integrity("digest_mismatch")),
            StatusClass::Error
        );
    }

    #[test]
    fn service_classification_handles_unavailable_and_degraded() {
        assert_eq!(classify_service(None), StatusClass::Warn);
        let healthy = ServiceView {
            running: true,
            pid: Some(1),
            pid_alive: true,
            socket_reachable: true,
            error: None,
        };
        assert_eq!(classify_service(Some(&healthy)), StatusClass::Ok);
        let degraded = ServiceView {
            running: true,
            pid: Some(1),
            pid_alive: true,
            socket_reachable: false,
            error: None,
        };
        assert_eq!(classify_service(Some(&degraded)), StatusClass::Warn);
        let failed = ServiceView {
            running: false,
            pid: None,
            pid_alive: false,
            socket_reachable: false,
            error: Some("boom".to_owned()),
        };
        assert_eq!(classify_service(Some(&failed)), StatusClass::Error);
    }

    #[test]
    fn gpu_classification_covers_unavailable_and_thresholds() {
        assert_eq!(classify_gpu(None), StatusClass::Warn);
        let unavailable = GpuView {
            available: false,
            name: None,
            utilization_percent: None,
            free_vram_mb: None,
            temperature_c: None,
            error: Some("nvidia-smi missing".to_owned()),
        };
        assert_eq!(classify_gpu(Some(&unavailable)), StatusClass::Warn);
        let healthy = GpuView {
            available: true,
            name: Some("RTX".to_owned()),
            utilization_percent: Some(10),
            free_vram_mb: Some(4096),
            temperature_c: Some(50),
            error: None,
        };
        assert_eq!(classify_gpu(Some(&healthy)), StatusClass::Ok);
        let busy = GpuView {
            utilization_percent: Some(95),
            ..healthy.clone()
        };
        assert_eq!(classify_gpu(Some(&busy)), StatusClass::Warn);
        let hot = GpuView {
            temperature_c: Some(85),
            ..healthy
        };
        assert_eq!(classify_gpu(Some(&hot)), StatusClass::Error);
    }

    #[test]
    fn fleet_classification_covers_empty_partial_and_total_outage() {
        assert_eq!(classify_fleet(None), StatusClass::Warn);
        let empty = FleetView { hosts: Vec::new() };
        assert_eq!(classify_fleet(Some(&empty)), StatusClass::Warn);
        let host = |reachable| FleetHostView {
            host_id: "host".to_owned(),
            reachable,
            model_count: 2,
            latency_ms: 5,
            error: None,
        };
        let healthy = FleetView {
            hosts: vec![host(true), host(true)],
        };
        assert_eq!(classify_fleet(Some(&healthy)), StatusClass::Ok);
        let partial = FleetView {
            hosts: vec![host(true), host(false)],
        };
        assert_eq!(classify_fleet(Some(&partial)), StatusClass::Warn);
        let down = FleetView {
            hosts: vec![host(false), host(false)],
        };
        assert_eq!(classify_fleet(Some(&down)), StatusClass::Error);
    }

    #[test]
    fn database_classification_uses_real_sizes() {
        let ok = DatabaseView {
            platform_db_bytes: Some(4096),
            safe_db_bytes: Some(1024),
            core_db_bytes: Some(8192),
            error: None,
        };
        assert_eq!(classify_database(&ok), StatusClass::Ok);
        let missing = DatabaseView {
            platform_db_bytes: None,
            safe_db_bytes: None,
            core_db_bytes: None,
            error: None,
        };
        assert_eq!(classify_database(&missing), StatusClass::Warn);
        let broken = DatabaseView {
            error: Some("bad".to_owned()),
            ..ok
        };
        assert_eq!(classify_database(&broken), StatusClass::Error);
    }

    #[test]
    fn summary_json_reports_shared_sections_and_unavailable_reasons() {
        let model = dashboard_model();
        let summary = summary_json(&model);
        assert_eq!(summary["schema"], "aegis.ui.summary.v1");
        assert_eq!(summary["overall_health"], "ok");
        assert_eq!(summary["platform"]["status"], "available");
        assert_eq!(summary["cores"]["count"], 1);
        assert_eq!(summary["cores"]["items"][0]["core_uuid"], CORE_UUID);
        assert_eq!(summary["cores"]["items"][0]["health"], "ok");
        assert_eq!(summary["fleet"]["status"], "available");
        assert_eq!(summary["gpu"]["status"], "available");
        assert_eq!(summary["service"]["status"], "available");
        assert_eq!(summary["memory"]["status"], "available");
        assert!(summary["issues"]["errors"].as_array().unwrap().is_empty());
        assert!(summary["consistency"].as_array().unwrap().is_empty());

        let mut unavailable = dashboard_model();
        unavailable.fleet = None;
        unavailable.gpu = None;
        unavailable.inference = None;
        unavailable.service = None;
        unavailable.memory = None;
        unavailable.fleet_snapshot = SnapshotState::Missing(FLEET_UNAVAILABLE_REASON.to_owned());
        unavailable.gpu_snapshot = SnapshotState::Missing(GPU_UNAVAILABLE_REASON.to_owned());
        unavailable.inference_snapshot =
            SnapshotState::Missing(INFERENCE_UNAVAILABLE_REASON.to_owned());
        unavailable.service_snapshot =
            SnapshotState::Missing(SERVICE_UNAVAILABLE_REASON.to_owned());
        let summary = summary_json(&unavailable);
        assert_eq!(summary["overall_health"], "ok");
        for (key, needle) in [
            ("fleet", "aegis-fleet"),
            ("gpu", "GPU telemetry"),
            ("inference", "aegis-cognition"),
            ("service", "aegis-service"),
            ("memory", "memory telemetry"),
        ] {
            assert_eq!(summary[key]["status"], "unavailable", "{key}");
            assert!(
                summary[key]["unavailable_reason"]
                    .as_str()
                    .unwrap()
                    .contains(needle),
                "{key}"
            );
        }
    }

    #[test]
    fn summary_json_surfaces_integrity_and_heartbeat_with_severity() {
        let mut model = dashboard_model();
        model.cores[0].integrity = integrity("digest_mismatch");
        model.cores[0].runtime = runtime("running", false);
        let summary = summary_json(&model);

        let errors = summary["issues"]["errors"].as_array().unwrap();
        assert!(errors
            .iter()
            .any(|issue| issue["code"] == "integrity_failure" && issue["severity"] == "error"));
        let warnings = summary["issues"]["warnings"].as_array().unwrap();
        assert!(warnings
            .iter()
            .any(|issue| issue["code"] == "heartbeat_not_fresh" && issue["severity"] == "warn"));
        assert_eq!(summary["cores"]["items"][0]["health"], "error");
        assert_eq!(summary["overall_health"], "error");
    }

    #[test]
    fn consistency_warnings_flag_stop_zero_cores_and_count_mismatch() {
        let mut model = dashboard_model();
        {
            let platform = model.platform.as_mut().unwrap();
            platform.stop_active = true;
            platform.zero_core = true;
            platform.core_count = 3;
        }
        let warnings = consistency_warnings(&model);
        let found = codes(&warnings);
        assert!(found.contains(&"stop_active"));
        assert!(found.contains(&"zero_core_contradiction"));
        assert!(found.contains(&"core_count_mismatch"));
        assert!(warnings
            .iter()
            .all(|issue| issue.severity == Severity::Warn));
    }

    #[test]
    fn consistency_warnings_flag_active_core_with_out_of_date_heartbeat() {
        let mut model = dashboard_model();
        model.cores[0].runtime = runtime("running", false);
        let warnings = consistency_warnings(&model);
        assert!(codes(&warnings).contains(&"heartbeat_not_fresh"));

        let mut empty = dashboard_model();
        empty.cores.clear();
        let platform = empty.platform.as_mut().unwrap();
        platform.core_count = 0;
        platform.zero_core = true;
        let warnings = consistency_warnings(&empty);
        assert!(codes(&warnings).contains(&"no_cores"));
    }

    #[test]
    fn core_issues_surface_runtime_failures_as_errors() {
        let mut model = dashboard_model();
        model.cores[0].runtime = runtime("failed", false);
        let issues = core_issues(&model.cores);
        assert!(codes(&issues).contains(&"runtime_failed"));
        assert!(issues.iter().all(|issue| issue.severity == Severity::Error));
    }

    fn write_snapshot(root: &Path, relative: &str, value: Value) {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("snapshot dir");
        std::fs::write(&path, serde_json::to_vec(&value).expect("serialize")).expect("write");
    }

    fn fresh_timestamp() -> String {
        chrono::Utc::now().to_rfc3339()
    }

    #[test]
    fn fresh_status_snapshot_populates_section() {
        let root = tempfile::tempdir().expect("tempdir");
        write_snapshot(
            root.path(),
            FLEET_STATUS_SNAPSHOT_PATH,
            json!({
                "updated_at": fresh_timestamp(),
                "hosts": [
                    {
                        "host_id": "host-a",
                        "reachable": true,
                        "model_count": 2,
                        "latency_ms": 5,
                        "error": null
                    }
                ]
            }),
        );
        let (state, fleet) = load_status_snapshot(
            root.path(),
            FLEET_STATUS_SNAPSHOT_PATH,
            "Fleet",
            FLEET_UNAVAILABLE_REASON,
            parse_fleet_snapshot,
        );
        assert!(state.is_fresh());
        let fleet = fleet.expect("fresh snapshot populates fleet");
        assert_eq!(fleet.hosts.len(), 1);
        assert_eq!(fleet.hosts[0].host_id, "host-a");
        assert_eq!(fleet.hosts[0].model_count, 2);
    }

    #[test]
    fn missing_status_snapshot_is_unavailable() {
        let root = tempfile::tempdir().expect("tempdir");
        let (state, fleet) = load_status_snapshot(
            root.path(),
            FLEET_STATUS_SNAPSHOT_PATH,
            "Fleet",
            FLEET_UNAVAILABLE_REASON,
            parse_fleet_snapshot,
        );
        assert!(fleet.is_none());
        assert_eq!(
            state,
            SnapshotState::Missing(FLEET_UNAVAILABLE_REASON.to_owned())
        );
        assert_eq!(state.health_str(), "unavailable");
    }

    #[test]
    fn stale_status_snapshot_is_rejected_with_reason() {
        let root = tempfile::tempdir().expect("tempdir");
        let stale = (chrono::Utc::now()
            - chrono::Duration::seconds(STATUS_SNAPSHOT_MAX_AGE_SECS as i64 + 30))
        .to_rfc3339();
        write_snapshot(
            root.path(),
            SERVICE_STATUS_SNAPSHOT_PATH,
            json!({
                "updated_at": stale,
                "running": true,
                "pid": 1,
                "pid_alive": true,
                "socket_reachable": true
            }),
        );
        let (state, service) = load_status_snapshot(
            root.path(),
            SERVICE_STATUS_SNAPSHOT_PATH,
            "Service",
            SERVICE_UNAVAILABLE_REASON,
            parse_service_snapshot,
        );
        assert!(service.is_none());
        assert_eq!(state.health_str(), "stale");
        assert!(state.reason().unwrap().contains("stale"));
    }

    #[test]
    fn malformed_status_snapshot_does_not_panic() {
        let root = tempfile::tempdir().expect("tempdir");
        let path = root.path().join(GPU_STATUS_SNAPSHOT_PATH);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("snapshot dir");
        std::fs::write(&path, b"{ this is not json").expect("write");
        let (state, gpu) = load_status_snapshot(
            root.path(),
            GPU_STATUS_SNAPSHOT_PATH,
            "GPU",
            GPU_UNAVAILABLE_REASON,
            parse_gpu_snapshot,
        );
        assert!(gpu.is_none());
        assert!(matches!(state, SnapshotState::Malformed(_)));
        assert_eq!(state.health_str(), "unavailable");
    }

    #[test]
    fn invalid_status_snapshot_schema_is_rejected_without_panicking() {
        let root = tempfile::tempdir().expect("tempdir");
        write_snapshot(
            root.path(),
            INFERENCE_STATUS_SNAPSHOT_PATH,
            json!({ "updated_at": fresh_timestamp() }),
        );
        let (state, inference) = load_status_snapshot(
            root.path(),
            INFERENCE_STATUS_SNAPSHOT_PATH,
            "Inference",
            INFERENCE_UNAVAILABLE_REASON,
            parse_inference_snapshot,
        );
        assert!(inference.is_none());
        assert!(matches!(state, SnapshotState::Malformed(_)));
    }
}

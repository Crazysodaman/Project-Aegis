//! Real data assembly for both the HTTP dashboard and the terminal dashboard.
//!
//! Every value here is read from the Platform, Core installer, Core runtime,
//! module registry, or diagnostics APIs. Nothing is fabricated.

use std::collections::HashMap;
use std::path::Path;

use aegis_contracts::integrity::IntegrityReport;
use aegis_contracts::status::PlatformStatus;
use aegis_core_install::CoreInstaller;
use aegis_core_runtime::{CoreRuntime, RuntimeStatus};
use aegis_diagnostics::{Diagnostics, DiagnosticsReport};
use aegis_modules::{ModuleRecord, ModuleRegistry};
use aegis_platform::PlatformService;
use aegis_storage::StoragePaths;
use serde_json::{json, Value};

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

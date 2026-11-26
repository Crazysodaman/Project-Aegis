//! Aegis Platform background service lifecycle (Stage 2).
//!
//! The service manager starts a real detached `aegis-service-host` child that
//! serves the local IPC endpoint, records a PID/state file, verifies the child
//! is reachable, and supports bounded graceful shutdown with a forced-kill
//! fallback. It never starts a Core and never auto-starts at boot.
//!
//! [`registration`] adds declarative, side-effect-free systemd and Windows SCM
//! service definitions. It only renders data; it never installs or controls an
//! OS service.

pub mod execution;
pub mod health;
pub mod logging;
pub mod registration;
pub mod runtime;
pub mod supervision;

pub use execution::{
    DryRunReport, ExecutionError, ExecutionPrecondition, ExecutionRollback, ExecutionStep,
    ExecutionVerification, InstallExecutionPlan, PlannedOperation, ServiceInstallExecutor,
};
pub use health::{
    evaluate_health, HealthInputs, HealthLevel, HealthReason, ServiceHealth,
    DEFAULT_HEARTBEAT_TIMEOUT,
};
pub use logging::{
    journal_integration, JournalIntegration, JournalTransport, LogRetentionConfig, LoggingError,
    LoggingSpecification, RetentionOwner, DEFAULT_LOG_AGE_SECS, DEFAULT_LOG_BYTES_PER_FILE,
    DEFAULT_LOG_FILES, DEFAULT_LOG_TOTAL_BYTES, MAX_LOG_AGE_SECS, MAX_LOG_BYTES_PER_FILE,
    MAX_LOG_FILES, MAX_LOG_TOTAL_BYTES,
};
pub use registration::{
    platform, InstallAction, InstallCommand, InstallPlan, InstallStep, PlatformKind,
    RegistrationError, RenderedRegistration, RestartPolicy, ServiceCommandSpec, ServicePlatform,
    ServiceRegistration, ServiceType, ServiceUnit, WindowsServiceSpecs, DEFAULT_SERVICE_NAME,
    MAX_SERVICE_NAME_LEN, SYSTEMD_UNIT_DIR,
};
pub use runtime::{
    runtime_support, runtime_support_with, OsFamily, RuntimeInputs, RuntimeSupport, RuntimeVerdict,
    SYSTEMCTL_CANDIDATES, SYSTEMD_RUNTIME_DIR,
};
pub use supervision::{
    plan_restart, Backoff, RestartCondition, RestartDecision, RestartPolicy as SupervisionPolicy,
    StopReason, SupervisionError,
};

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use aegis_contracts::ipc::{
    IpcRequest, IPC_PROTOCOL_VERSION, METHOD_PLATFORM_SHUTDOWN, METHOD_PLATFORM_STATUS,
};
use aegis_ipc::IpcClient;
use aegis_safe::{SafeError, SafeStore};
use aegis_storage::{now_rfc3339, StorageError, StoragePaths};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tempfile::NamedTempFile;
use thiserror::Error;

/// Version marker for the persisted service state document.
pub const SERVICE_STATE_SCHEMA: &str = "aegis.service.state.v1";
/// File name of the persisted service state, under `<root>/run`.
pub const SERVICE_STATE_FILE: &str = "service.json";
/// Default local IPC endpoint name.
pub const DEFAULT_SERVICE_SOCKET: &str = "aegis-platform";
/// Maximum time to wait for a started service to answer IPC.
pub const SERVICE_START_TIMEOUT_MS: u64 = 8_000;
/// Maximum time to wait for a graceful stop before forced termination.
pub const SERVICE_STOP_TIMEOUT_MS: u64 = 5_000;

const POLL_INTERVAL_MS: u64 = 50;
const HOST_FILE_NAME: &str = "aegis-service-host";

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("service storage failed: {0}")]
    Storage(#[from] StorageError),
    #[error("service state I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("service state JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Safe authorization failed: {0}")]
    Safe(#[from] SafeError),
    #[error("owner authorization failed")]
    Unauthorized,
    #[error("Platform is not initialized at {0}")]
    NotInitialized(String),
    #[error("service is already running with pid {0}")]
    AlreadyRunning(u32),
    #[error("service host executable was not found: {0}")]
    HostMissing(String),
    #[error("service failed to start: {0}")]
    StartFailed(String),
    #[error("service did not stop within the timeout")]
    StopTimeout,
    #[error("service state is invalid: {0}")]
    InvalidState(String),
}

/// Persisted process state for one service instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceState {
    pub schema: String,
    pub pid: u32,
    pub root: String,
    pub socket: String,
    pub started_at: String,
    pub executable: String,
    pub protocol: String,
}

/// Observed service status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceStatus {
    pub running: bool,
    pub pid: Option<u32>,
    pub pid_alive: bool,
    pub socket_reachable: bool,
    pub state: Option<ServiceState>,
    pub error: Option<String>,
}

/// Result of a stop request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceStopReport {
    pub stopped: bool,
    pub forced: bool,
    pub pid: Option<u32>,
}

/// Platform service lifecycle manager for one storage root.
#[derive(Debug, Clone)]
pub struct ServiceManager {
    paths: StoragePaths,
    safe: SafeStore,
    host_executable: Option<PathBuf>,
    socket_name: String,
}

impl ServiceManager {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, ServiceError> {
        let paths = StoragePaths::new(root)?;
        let safe = SafeStore::new(paths.clone());
        Ok(Self {
            paths,
            safe,
            host_executable: None,
            socket_name: DEFAULT_SERVICE_SOCKET.to_owned(),
        })
    }

    /// Use a specific `aegis-service-host` executable (tests and packaging).
    pub fn with_host_executable(mut self, executable: impl Into<PathBuf>) -> Self {
        self.host_executable = Some(executable.into());
        self
    }

    /// Use a specific local IPC endpoint name (tests and multi-root hosts).
    pub fn with_socket_name(mut self, socket: impl Into<String>) -> Self {
        self.socket_name = socket.into();
        self
    }

    pub fn root(&self) -> &Path {
        self.paths.root()
    }

    pub fn state_path(&self) -> PathBuf {
        self.paths.root().join("run").join(SERVICE_STATE_FILE)
    }

    pub fn read_state(&self) -> Result<Option<ServiceState>, ServiceError> {
        let path = self.state_path();
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let state: ServiceState = serde_json::from_slice(&bytes)?;
        if state.schema != SERVICE_STATE_SCHEMA {
            return Err(ServiceError::InvalidState(format!(
                "unsupported state schema {}",
                state.schema
            )));
        }
        Ok(Some(state))
    }

    pub fn status(&self) -> ServiceStatus {
        let state = match self.read_state() {
            Ok(state) => state,
            Err(error) => {
                return ServiceStatus {
                    running: false,
                    pid: None,
                    pid_alive: false,
                    socket_reachable: false,
                    state: None,
                    error: Some(error.to_string()),
                }
            }
        };
        let Some(state) = state else {
            return ServiceStatus {
                running: false,
                pid: None,
                pid_alive: false,
                socket_reachable: false,
                state: None,
                error: None,
            };
        };
        let pid_alive = process_is_alive(state.pid);
        let socket_reachable = pid_alive
            && self
                .safe
                .owner_token()
                .ok()
                .is_some_and(|token| ipc_status(&state.socket, &token).is_some());
        ServiceStatus {
            running: pid_alive,
            pid: Some(state.pid),
            pid_alive,
            socket_reachable,
            state: Some(state),
            error: if pid_alive && !socket_reachable {
                Some("service process is alive but IPC is not reachable".to_owned())
            } else {
                None
            },
        }
    }

    /// Start the background service. Requires owner proof.
    pub fn start(&self, owner_token: &str) -> Result<ServiceState, ServiceError> {
        self.authorize(owner_token)?;
        if !self.paths.platform_db().is_file() {
            return Err(ServiceError::NotInitialized(
                self.paths.root().display().to_string(),
            ));
        }
        if let Ok(Some(existing)) = self.read_state() {
            if process_is_alive(existing.pid) {
                return Err(ServiceError::AlreadyRunning(existing.pid));
            }
            let _ = fs::remove_file(self.state_path());
        }

        let run_dir = self.prepare_run_dir()?;
        let host = self.resolve_host_executable()?;
        let socket = self.socket_name.clone();
        let started_at = Utc::now().to_rfc3339();
        let mut child = Command::new(&host)
            .arg("--root")
            .arg(self.paths.root())
            .arg("--socket")
            .arg(&socket)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| ServiceError::StartFailed(format!("{}: {error}", host.display())))?;
        let pid = child.id();

        let state = ServiceState {
            schema: SERVICE_STATE_SCHEMA.to_owned(),
            pid,
            root: self.paths.root().display().to_string(),
            socket: socket.clone(),
            started_at,
            executable: host.display().to_string(),
            protocol: IPC_PROTOCOL_VERSION.to_owned(),
        };
        write_state(&run_dir, &state)?;

        let deadline = Instant::now() + Duration::from_millis(SERVICE_START_TIMEOUT_MS);
        loop {
            if let Some(status) = child.try_wait()? {
                let _ = fs::remove_file(self.state_path());
                return Err(ServiceError::StartFailed(format!(
                    "service host exited during startup: {status}"
                )));
            }
            if ipc_status(&socket, owner_token).is_some() {
                return Ok(state);
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                let _ = fs::remove_file(self.state_path());
                return Err(ServiceError::StartFailed(
                    "service did not become reachable before the startup timeout".to_owned(),
                ));
            }
            thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
        }
    }

    /// Stop the background service. Idempotent; requires owner proof.
    pub fn stop(&self, owner_token: &str) -> Result<ServiceStopReport, ServiceError> {
        self.authorize(owner_token)?;
        let Some(state) = self.read_state()? else {
            return Ok(ServiceStopReport {
                stopped: true,
                forced: false,
                pid: None,
            });
        };

        if !process_is_alive(state.pid) {
            let _ = fs::remove_file(self.state_path());
            return Ok(ServiceStopReport {
                stopped: true,
                forced: false,
                pid: Some(state.pid),
            });
        }

        let _ = request_shutdown(&state.socket, owner_token);
        let deadline = Instant::now() + Duration::from_millis(SERVICE_STOP_TIMEOUT_MS);
        while process_is_alive(state.pid) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
        }

        let mut forced = false;
        if process_is_alive(state.pid) {
            forced = true;
            let _ = terminate_pid(state.pid);
            let kill_deadline = Instant::now() + Duration::from_millis(SERVICE_STOP_TIMEOUT_MS);
            while process_is_alive(state.pid) && Instant::now() < kill_deadline {
                thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
            }
            if process_is_alive(state.pid) {
                return Err(ServiceError::StopTimeout);
            }
        }

        let _ = fs::remove_file(self.state_path());
        Ok(ServiceStopReport {
            stopped: true,
            forced,
            pid: Some(state.pid),
        })
    }

    fn authorize(&self, owner_token: &str) -> Result<(), ServiceError> {
        if !self.safe.verify_owner(owner_token)? {
            return Err(ServiceError::Unauthorized);
        }
        Ok(())
    }

    fn prepare_run_dir(&self) -> Result<PathBuf, ServiceError> {
        let directory = self.paths.root().join("run");
        fs::create_dir_all(&directory)?;
        let metadata = fs::symlink_metadata(&directory)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(ServiceError::InvalidState(
                "service run directory is redirected or not a directory".to_owned(),
            ));
        }
        Ok(directory)
    }

    fn resolve_host_executable(&self) -> Result<PathBuf, ServiceError> {
        if let Some(executable) = &self.host_executable {
            if executable.is_file() {
                return Ok(executable.clone());
            }
            return Err(ServiceError::HostMissing(executable.display().to_string()));
        }
        if let Ok(value) = std::env::var("CARGO_BIN_EXE_aegis-service-host") {
            let candidate = PathBuf::from(value);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
        if let Some(value) = option_env!("CARGO_BIN_EXE_aegis-service-host") {
            let candidate = PathBuf::from(value);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
        let file_name = format!("{HOST_FILE_NAME}{}", std::env::consts::EXE_SUFFIX);
        if let Ok(executable) = std::env::current_exe() {
            if let Some(directory) = executable.parent() {
                let candidate = directory.join(&file_name);
                if candidate.is_file() {
                    return Ok(candidate);
                }
                if let Some(parent) = directory.parent() {
                    let candidate = parent.join(&file_name);
                    if candidate.is_file() {
                        return Ok(candidate);
                    }
                }
            }
        }
        Err(ServiceError::HostMissing(file_name))
    }
}

fn write_state(run_dir: &Path, state: &ServiceState) -> Result<(), ServiceError> {
    let payload = serde_json::to_vec_pretty(state)?;
    let mut temporary = NamedTempFile::new_in(run_dir)?;
    use std::io::Write;
    temporary.write_all(&payload)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(run_dir.join(SERVICE_STATE_FILE))
        .map_err(|error| ServiceError::Io(error.error))?;
    Ok(())
}

fn ipc_status(socket: &str, token: &str) -> Option<()> {
    let client = IpcClient::new(socket);
    let request = IpcRequest::new(METHOD_PLATFORM_STATUS, json!({}), token);
    client
        .call(&request)
        .ok()
        .filter(|response| response.error.is_none())
        .map(|_| ())
}

fn request_shutdown(socket: &str, token: &str) -> Result<(), ServiceError> {
    let client = IpcClient::new(socket);
    let request = IpcRequest::new(METHOD_PLATFORM_SHUTDOWN, json!({}), token);
    client
        .call(&request)
        .map(|_| ())
        .map_err(|error| ServiceError::StartFailed(format!("shutdown request failed: {error}")))
}

/// Whether a process with `pid` is currently alive.
pub fn process_is_alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        let output = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
            .stdin(Stdio::null())
            .output();
        match output {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                stdout
                    .split(',')
                    .any(|field| field.trim_matches('"').trim() == pid.to_string())
            }
            _ => false,
        }
    }
    #[cfg(not(windows))]
    {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
}

#[cfg(windows)]
fn terminate_pid(pid: u32) -> Result<(), std::io::Error> {
    Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|_| ())
}

#[cfg(not(windows))]
fn terminate_pid(pid: u32) -> Result<(), std::io::Error> {
    Command::new("kill")
        .args(["-9", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|_| ())
}

/// Current RFC 3339 timestamp for external callers.
pub fn now() -> Result<String, StorageError> {
    now_rfc3339()
}

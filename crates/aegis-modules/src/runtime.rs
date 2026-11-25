//! Module process runtime: supervised execution, heartbeat, and failure
//! isolation for installed modules.
//!
//! Execution model (private-homelab appropriate):
//!
//! * Only `process` and `native` entrypoints are executed, and only when the
//!   module is in the `enabled` lifecycle state.
//! * The executable must resolve inside `<root>/modules/<module_id>`.
//! * The child runs with a cleared environment (no owner token, no Platform
//!   secrets) and its working directory set to the module package.
//! * Runtime scratch state lives outside the package under
//!   `<root>/run/modules/<module_id>`.
//! * A module is reported `running` only while its process is alive **and** its
//!   heartbeat file is fresh. Registry state is never treated as proof of a
//!   live process.
//!
//! Modules are trusted local code. The runtime provides lifecycle supervision,
//! timeouts, forced termination, and secret-free environment isolation; it is
//! not an operating-system sandbox.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use aegis_safe::{SafeError, SafeStore};
use aegis_storage::{
    now_rfc3339, open_initialized, StorageError, StoragePaths, PLATFORM_APPLICATION_ID,
};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::manifest::{validate_module_id, ModuleManifest};
use crate::registry::{ModuleError, ModuleLifecycleState, ModuleRegistry};

/// Schema identifier for a module runtime status document.
pub const MODULE_RUNTIME_SCHEMA: &str = "aegis.module.runtime.v1";
/// How long to wait for the first fresh heartbeat after spawning.
pub const MODULE_START_TIMEOUT_MS: u64 = 5_000;
/// How long to wait for a graceful stop before forced termination.
pub const MODULE_STOP_TIMEOUT_MS: u64 = 5_000;
/// A heartbeat older than this is not considered fresh.
pub const MODULE_HEARTBEAT_STALE_MS: i64 = 1_500;
/// Recommended heartbeat interval for module authors.
pub const MODULE_HEARTBEAT_INTERVAL_MS: u64 = 200;
/// Directory under the storage root that holds module runtime scratch state.
pub const MODULE_RUNTIME_DIR: &str = "modules";
/// Directory under `<root>/run` that holds per-module control files.
pub const MODULE_RUN_DIR: &str = "modules";

const POLL_INTERVAL_MS: u64 = 50;
const RUNTIME_COMPONENT: &str = "modules";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleRuntimeState {
    Starting,
    Running,
    Stopping,
    Stopped,
    Failed,
    Stale,
}

impl ModuleRuntimeState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
            Self::Stale => "stale",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "starting" => Some(Self::Starting),
            "running" => Some(Self::Running),
            "stopping" => Some(Self::Stopping),
            "stopped" => Some(Self::Stopped),
            "failed" => Some(Self::Failed),
            "stale" => Some(Self::Stale),
            _ => None,
        }
    }
}

/// Effective runtime status for one module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleRuntimeStatus {
    pub schema: String,
    pub module_id: String,
    pub lifecycle_state: String,
    pub state: ModuleRuntimeState,
    pub recorded_state: ModuleRuntimeState,
    pub pid: Option<u32>,
    pub pid_alive: bool,
    pub heartbeat_at: Option<String>,
    pub heartbeat_fresh: bool,
    pub heartbeat_age_ms: Option<i64>,
    pub started_at: Option<String>,
    pub updated_at: Option<String>,
    pub last_error: Option<String>,
}

impl ModuleRuntimeStatus {
    pub fn is_running(&self) -> bool {
        self.state == ModuleRuntimeState::Running
    }
}

#[derive(Debug, Error)]
pub enum ModuleRuntimeError {
    #[error("module runtime storage failed: {0}")]
    Storage(#[from] StorageError),
    #[error("module runtime SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Safe operation failed: {0}")]
    Safe(#[from] SafeError),
    #[error("module registry failed: {0}")]
    Registry(#[from] ModuleError),
    #[error("module runtime I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("module runtime JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Platform is not initialized at {0}")]
    NotInitialized(String),
    #[error("owner authorization failed")]
    Unauthorized,
    #[error("operator STOP is active")]
    StopActive,
    #[error("module {module_id} is {state}; enable it before starting")]
    NotEnabled { module_id: String, state: String },
    #[error("module {0} is already running")]
    AlreadyRunning(String),
    #[error("module {module_id} entrypoint kind {kind} is not executable by this runtime")]
    UnsupportedEntrypoint { module_id: String, kind: String },
    #[error("module {module_id} entrypoint is missing: {path}")]
    EntrypointMissing { module_id: String, path: String },
    #[error("invalid module runtime state: {0}")]
    InvalidState(String),
    #[error("module {module_id} failed to spawn: {message}")]
    SpawnFailed { module_id: String, message: String },
    #[error("module {module_id} did not become ready: {message}")]
    StartTimeout { module_id: String, message: String },
    #[error("module {module_id} did not stop: {message}")]
    StopTimeout { module_id: String, message: String },
}

#[derive(Debug, Clone)]
struct StoredRuntime {
    module_id: String,
    state: ModuleRuntimeState,
    pid: Option<u32>,
    started_at: Option<String>,
    updated_at: String,
    last_error: Option<String>,
}

/// Supervised module process runtime for one storage root.
#[derive(Debug, Clone)]
pub struct ModuleRuntime {
    paths: StoragePaths,
    registry: ModuleRegistry,
    safe: SafeStore,
}

impl ModuleRuntime {
    pub fn new(paths: StoragePaths) -> Self {
        Self {
            registry: ModuleRegistry::new(paths.clone()),
            safe: SafeStore::new(paths.clone()),
            paths,
        }
    }

    /// Start an enabled module and wait until it reports a fresh heartbeat.
    pub fn start(
        &self,
        module_id: &str,
        owner_token: &str,
    ) -> Result<ModuleRuntimeStatus, ModuleRuntimeError> {
        self.authorize(owner_token)?;
        validate_module_id(module_id).map_err(ModuleError::from)?;
        if self.safe.stop_status()?.active {
            return Err(ModuleRuntimeError::StopActive);
        }

        let inspection = self.registry.inspect(module_id)?;
        if inspection.record.lifecycle_state != ModuleLifecycleState::Enabled {
            return Err(ModuleRuntimeError::NotEnabled {
                module_id: module_id.to_owned(),
                state: inspection.record.lifecycle_state.as_str().to_owned(),
            });
        }
        if !matches!(
            inspection.manifest.entrypoint.kind.as_str(),
            "process" | "native"
        ) {
            return Err(ModuleRuntimeError::UnsupportedEntrypoint {
                module_id: module_id.to_owned(),
                kind: inspection.manifest.entrypoint.kind,
            });
        }

        let connection = self.open()?;
        let existing = read_row(&connection, module_id)?;
        let current = self.build_status(
            existing.clone(),
            module_id,
            inspection.record.lifecycle_state.as_str(),
        );
        if current.pid_alive {
            return Err(ModuleRuntimeError::AlreadyRunning(module_id.to_owned()));
        }

        let control = prepare_control_dir(self.paths.root(), module_id)?;
        let heartbeat = control.join(format!("{module_id}.heartbeat"));
        let sentinel = control.join(format!("{module_id}.stop"));
        let log_path = control.join(format!("{module_id}.log"));
        let config_path = control.join(format!("{module_id}.json"));
        remove_if_exists(&heartbeat)?;
        remove_if_exists(&sentinel)?;
        if !config_path.exists() {
            fs::write(&config_path, b"{}")?;
        }

        let package_dir = self.paths.root().join(MODULE_RUNTIME_DIR).join(module_id);
        let executable = resolve_entrypoint(&package_dir, module_id, &inspection.manifest)?;
        let stdout = log_file(&log_path)?;
        let stderr = log_file(&log_path)?;

        let mut command = Command::new(&executable);
        command.args(&inspection.manifest.entrypoint.args);
        command
            .arg("--module-id")
            .arg(module_id)
            .arg("--control")
            .arg(&control)
            .arg("--config")
            .arg(&config_path)
            .current_dir(&package_dir)
            .env_clear()
            .env("AEGIS_MODULE_ID", module_id)
            .env("AEGIS_MODULE_RUNTIME", MODULE_RUNTIME_SCHEMA)
            .env("AEGIS_MODULE_DIR", &package_dir)
            .env("AEGIS_MODULE_CONTROL_DIR", &control)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        #[cfg(windows)]
        if let Ok(system_root) = std::env::var("SYSTEMROOT") {
            command.env("SYSTEMROOT", system_root);
        }

        let started_at = now_rfc3339()?;
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                let message = format!("could not spawn {}: {error}", executable.display());
                self.mark_failed(&connection, module_id, &message)?;
                return Err(ModuleRuntimeError::SpawnFailed {
                    module_id: module_id.to_owned(),
                    message,
                });
            }
        };
        let pid = child.id();
        let starting = StoredRuntime {
            module_id: module_id.to_owned(),
            state: ModuleRuntimeState::Starting,
            pid: Some(pid),
            started_at: Some(started_at),
            updated_at: now_rfc3339()?,
            last_error: None,
        };
        write_row(&connection, &starting)?;

        let deadline = Instant::now() + Duration::from_millis(MODULE_START_TIMEOUT_MS);
        loop {
            if let Some(status) = child.try_wait()? {
                let message = format!("module process exited before heartbeat: {status}");
                self.mark_failed(&connection, module_id, &message)?;
                remove_if_exists(&heartbeat)?;
                return Err(ModuleRuntimeError::SpawnFailed {
                    module_id: module_id.to_owned(),
                    message,
                });
            }
            let (_, _, fresh) = heartbeat_observation(&heartbeat);
            if fresh {
                let running = StoredRuntime {
                    state: ModuleRuntimeState::Running,
                    updated_at: now_rfc3339()?,
                    last_error: None,
                    ..starting
                };
                write_row(&connection, &running)?;
                let status = self.build_status(
                    Some(running),
                    module_id,
                    inspection.record.lifecycle_state.as_str(),
                );
                reap_in_background(child);
                return Ok(status);
            }
            if Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
        }

        let _ = child.kill();
        let _ = child.wait();
        remove_if_exists(&heartbeat)?;
        let message = format!("timed out waiting for a heartbeat from {module_id}");
        self.mark_failed(&connection, module_id, &message)?;
        Err(ModuleRuntimeError::StartTimeout {
            module_id: module_id.to_owned(),
            message,
        })
    }

    /// Stop a module. Idempotent when it is not running.
    pub fn stop(
        &self,
        module_id: &str,
        owner_token: &str,
    ) -> Result<ModuleRuntimeStatus, ModuleRuntimeError> {
        self.authorize(owner_token)?;
        validate_module_id(module_id).map_err(ModuleError::from)?;
        let inspection = self.registry.inspect(module_id)?;
        let lifecycle = inspection.record.lifecycle_state.as_str();
        let connection = self.open()?;
        let existing = read_row(&connection, module_id)?;
        let pid = existing.as_ref().and_then(|row| row.pid);
        let control = prepare_control_dir(self.paths.root(), module_id)?;
        let heartbeat = control.join(format!("{module_id}.heartbeat"));
        let sentinel = control.join(format!("{module_id}.stop"));

        let Some(pid) = pid else {
            remove_if_exists(&heartbeat)?;
            remove_if_exists(&sentinel)?;
            let stopped = self.mark_stopped(&connection, module_id)?;
            return Ok(self.build_status(Some(stopped), module_id, lifecycle));
        };

        if !process_is_alive(pid) {
            remove_if_exists(&heartbeat)?;
            remove_if_exists(&sentinel)?;
            let stopped = self.mark_stopped(&connection, module_id)?;
            return Ok(self.build_status(Some(stopped), module_id, lifecycle));
        }

        fs::write(&sentinel, b"stop")?;
        let mut stopping = existing.unwrap_or(StoredRuntime {
            module_id: module_id.to_owned(),
            state: ModuleRuntimeState::Stopping,
            pid: Some(pid),
            started_at: None,
            updated_at: now_rfc3339()?,
            last_error: None,
        });
        stopping.state = ModuleRuntimeState::Stopping;
        stopping.updated_at = now_rfc3339()?;
        write_row(&connection, &stopping)?;

        let deadline = Instant::now() + Duration::from_millis(MODULE_STOP_TIMEOUT_MS);
        while process_is_alive(pid) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
        }

        if process_is_alive(pid) {
            terminate_pid(pid)?;
            let kill_deadline = Instant::now() + Duration::from_millis(MODULE_STOP_TIMEOUT_MS);
            while process_is_alive(pid) && Instant::now() < kill_deadline {
                thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
            }
            if process_is_alive(pid) {
                let message = format!("module process {pid} did not terminate");
                self.mark_failed(&connection, module_id, &message)?;
                return Err(ModuleRuntimeError::StopTimeout {
                    module_id: module_id.to_owned(),
                    message,
                });
            }
        }

        remove_if_exists(&heartbeat)?;
        remove_if_exists(&sentinel)?;
        let stopped = self.mark_stopped(&connection, module_id)?;
        Ok(self.build_status(Some(stopped), module_id, lifecycle))
    }

    /// Stop then start the module.
    pub fn restart(
        &self,
        module_id: &str,
        owner_token: &str,
    ) -> Result<ModuleRuntimeStatus, ModuleRuntimeError> {
        self.stop(module_id, owner_token)?;
        self.start(module_id, owner_token)
    }

    /// Effective status derived from the recorded row, process liveness, and
    /// heartbeat freshness.
    pub fn status(&self, module_id: &str) -> Result<ModuleRuntimeStatus, ModuleRuntimeError> {
        validate_module_id(module_id).map_err(ModuleError::from)?;
        let inspection = self.registry.inspect(module_id)?;
        let connection = self.open()?;
        let stored = read_row(&connection, module_id)?;
        Ok(self.build_status(
            stored,
            module_id,
            inspection.record.lifecycle_state.as_str(),
        ))
    }

    /// Status for every registered module.
    pub fn list(&self) -> Result<Vec<ModuleRuntimeStatus>, ModuleRuntimeError> {
        self.registry.initialize()?;
        let records = self.registry.list()?;
        let connection = self.open()?;
        let mut statuses = Vec::with_capacity(records.len());
        for record in records {
            let stored = read_row(&connection, &record.module_id)?;
            statuses.push(self.build_status(
                stored,
                &record.module_id,
                record.lifecycle_state.as_str(),
            ));
        }
        Ok(statuses)
    }

    fn authorize(&self, owner_token: &str) -> Result<(), ModuleRuntimeError> {
        if !self.paths.platform_db().is_file() {
            return Err(ModuleRuntimeError::NotInitialized(
                self.paths.root().display().to_string(),
            ));
        }
        if !self.safe.verify_owner(owner_token)? {
            return Err(ModuleRuntimeError::Unauthorized);
        }
        Ok(())
    }

    fn open(&self) -> Result<Connection, ModuleRuntimeError> {
        let connection = open_initialized(
            &self.paths.platform_db(),
            PLATFORM_APPLICATION_ID,
            RUNTIME_COMPONENT,
        )?;
        ensure_runtime_table(&connection)?;
        Ok(connection)
    }

    fn build_status(
        &self,
        stored: Option<StoredRuntime>,
        module_id: &str,
        lifecycle: &str,
    ) -> ModuleRuntimeStatus {
        let control = control_dir(self.paths.root(), module_id);
        let (heartbeat_at, heartbeat_age_ms, heartbeat_fresh) =
            heartbeat_observation(&control.join(format!("{module_id}.heartbeat")));
        let pid = stored.as_ref().and_then(|row| row.pid);
        let pid_alive = pid.is_some_and(process_is_alive);
        let recorded_state = stored
            .as_ref()
            .map(|row| row.state)
            .unwrap_or(ModuleRuntimeState::Stopped);
        let (state, last_error) = derive_state(
            recorded_state,
            pid_alive,
            heartbeat_fresh,
            stored.as_ref().and_then(|row| row.last_error.clone()),
        );
        ModuleRuntimeStatus {
            schema: MODULE_RUNTIME_SCHEMA.to_owned(),
            module_id: module_id.to_owned(),
            lifecycle_state: lifecycle.to_owned(),
            state,
            recorded_state,
            pid,
            pid_alive: pid.is_some() && pid_alive,
            heartbeat_at,
            heartbeat_fresh,
            heartbeat_age_ms,
            started_at: stored.as_ref().and_then(|row| row.started_at.clone()),
            updated_at: stored.as_ref().map(|row| row.updated_at.clone()),
            last_error,
        }
    }

    fn mark_failed(
        &self,
        connection: &Connection,
        module_id: &str,
        message: &str,
    ) -> Result<(), ModuleRuntimeError> {
        let existing = read_row(connection, module_id)?;
        write_row(
            connection,
            &StoredRuntime {
                module_id: module_id.to_owned(),
                state: ModuleRuntimeState::Failed,
                pid: existing.as_ref().and_then(|row| row.pid),
                started_at: existing.and_then(|row| row.started_at),
                updated_at: now_rfc3339()?,
                last_error: Some(message.to_owned()),
            },
        )
    }

    fn mark_stopped(
        &self,
        connection: &Connection,
        module_id: &str,
    ) -> Result<StoredRuntime, ModuleRuntimeError> {
        let existing = read_row(connection, module_id)?;
        let stopped = StoredRuntime {
            module_id: module_id.to_owned(),
            state: ModuleRuntimeState::Stopped,
            pid: None,
            started_at: existing.as_ref().and_then(|row| row.started_at.clone()),
            updated_at: now_rfc3339()?,
            last_error: None,
        };
        write_row(connection, &stopped)?;
        Ok(stopped)
    }
}

fn derive_state(
    recorded: ModuleRuntimeState,
    pid_alive: bool,
    heartbeat_fresh: bool,
    last_error: Option<String>,
) -> (ModuleRuntimeState, Option<String>) {
    match recorded {
        ModuleRuntimeState::Starting => {
            if !pid_alive {
                (ModuleRuntimeState::Failed, last_error)
            } else if heartbeat_fresh {
                (ModuleRuntimeState::Running, None)
            } else {
                (ModuleRuntimeState::Starting, last_error)
            }
        }
        ModuleRuntimeState::Running => {
            if !pid_alive {
                (
                    ModuleRuntimeState::Failed,
                    last_error.or_else(|| Some("module process exited".to_owned())),
                )
            } else if !heartbeat_fresh {
                (
                    ModuleRuntimeState::Stale,
                    last_error.or_else(|| Some("module heartbeat is stale".to_owned())),
                )
            } else {
                (ModuleRuntimeState::Running, None)
            }
        }
        ModuleRuntimeState::Stopping => {
            if pid_alive {
                (ModuleRuntimeState::Stopping, last_error)
            } else {
                (ModuleRuntimeState::Stopped, None)
            }
        }
        ModuleRuntimeState::Stopped | ModuleRuntimeState::Stale => {
            if pid_alive && heartbeat_fresh {
                (ModuleRuntimeState::Running, None)
            } else {
                (recorded, last_error)
            }
        }
        ModuleRuntimeState::Failed => (ModuleRuntimeState::Failed, last_error),
    }
}

fn ensure_runtime_table(connection: &Connection) -> Result<(), ModuleRuntimeError> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS aegis_module_runtime (
            module_id TEXT PRIMARY KEY,
            state TEXT NOT NULL CHECK(state IN (
                'starting', 'running', 'stopping', 'stopped', 'failed', 'stale'
            )),
            pid INTEGER,
            started_at TEXT,
            updated_at TEXT NOT NULL,
            last_error TEXT
        );",
    )?;
    Ok(())
}

fn read_row(
    connection: &Connection,
    module_id: &str,
) -> Result<Option<StoredRuntime>, ModuleRuntimeError> {
    let row = connection
        .query_row(
            "SELECT module_id, state, pid, started_at, updated_at, last_error
             FROM aegis_module_runtime WHERE module_id = ?1",
            [module_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            },
        )
        .optional()?;
    let Some(row) = row else {
        return Ok(None);
    };
    let state = ModuleRuntimeState::parse(&row.1)
        .ok_or_else(|| ModuleRuntimeError::InvalidState(row.1.clone()))?;
    let pid = match row.2 {
        Some(value) => Some(
            u32::try_from(value)
                .map_err(|_| ModuleRuntimeError::InvalidState("runtime pid".to_owned()))?,
        ),
        None => None,
    };
    Ok(Some(StoredRuntime {
        module_id: row.0,
        state,
        pid,
        started_at: row.3,
        updated_at: row.4,
        last_error: row.5,
    }))
}

fn write_row(connection: &Connection, row: &StoredRuntime) -> Result<(), ModuleRuntimeError> {
    connection.execute(
        "INSERT INTO aegis_module_runtime(
            module_id, state, pid, started_at, updated_at, last_error
         ) VALUES(?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(module_id) DO UPDATE SET
             state = excluded.state,
             pid = excluded.pid,
             started_at = excluded.started_at,
             updated_at = excluded.updated_at,
             last_error = excluded.last_error",
        params![
            row.module_id,
            row.state.as_str(),
            row.pid.map(i64::from),
            row.started_at,
            row.updated_at,
            row.last_error,
        ],
    )?;
    Ok(())
}

fn mod_control_root(root: &Path) -> PathBuf {
    root.join("run").join(MODULE_RUN_DIR)
}

fn control_dir(root: &Path, module_id: &str) -> PathBuf {
    mod_control_root(root).join(module_id)
}

fn prepare_control_dir(root: &Path, module_id: &str) -> Result<PathBuf, ModuleRuntimeError> {
    let directory = control_dir(root, module_id);
    fs::create_dir_all(&directory)?;
    let metadata = fs::symlink_metadata(&directory)?;
    if is_redirected(&metadata) || !metadata.is_dir() {
        return Err(ModuleRuntimeError::InvalidState(
            "module control directory is redirected or not a directory".to_owned(),
        ));
    }
    Ok(directory)
}

pub(crate) fn resolve_entrypoint(
    package_dir: &Path,
    module_id: &str,
    manifest: &ModuleManifest,
) -> Result<PathBuf, ModuleRuntimeError> {
    let metadata = match fs::symlink_metadata(package_dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Err(ModuleRuntimeError::EntrypointMissing {
                module_id: module_id.to_owned(),
                path: package_dir.display().to_string(),
            })
        }
        Err(error) => return Err(error.into()),
    };
    if is_redirected(&metadata) || !metadata.is_dir() {
        return Err(ModuleRuntimeError::InvalidState(
            "module package directory is redirected or not a directory".to_owned(),
        ));
    }
    let relative = &manifest.entrypoint.path;
    let mut candidates = vec![package_dir.join(relative)];
    let suffix = std::env::consts::EXE_SUFFIX;
    if !suffix.is_empty() && !relative.ends_with(suffix) {
        candidates.push(package_dir.join(format!("{relative}{suffix}")));
    }
    for candidate in candidates {
        if let Ok(metadata) = fs::symlink_metadata(&candidate) {
            if is_redirected(&metadata) || !metadata.is_file() {
                return Err(ModuleRuntimeError::InvalidState(
                    "module entrypoint is redirected or not a file".to_owned(),
                ));
            }
            return Ok(candidate);
        }
    }
    Err(ModuleRuntimeError::EntrypointMissing {
        module_id: module_id.to_owned(),
        path: package_dir.join(relative).display().to_string(),
    })
}

fn heartbeat_observation(path: &Path) -> (Option<String>, Option<i64>, bool) {
    let Ok(metadata) = fs::metadata(path) else {
        return (None, None, false);
    };
    let Ok(modified) = metadata.modified() else {
        return (None, None, false);
    };
    let at = DateTime::<Utc>::from(modified).to_rfc3339();
    match SystemTime::now().duration_since(modified) {
        Ok(age) => {
            let age_ms = age.as_millis().min(i64::MAX as u128) as i64;
            (Some(at), Some(age_ms), age_ms <= MODULE_HEARTBEAT_STALE_MS)
        }
        Err(_) => (Some(at), None, false),
    }
}

fn log_file(path: &Path) -> Result<fs::File, ModuleRuntimeError> {
    Ok(fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?)
}

fn remove_if_exists(path: &Path) -> Result<(), ModuleRuntimeError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn reap_in_background(mut child: Child) {
    thread::spawn(move || {
        let _ = child.wait();
    });
}

#[cfg(windows)]
fn is_redirected(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.file_type().is_symlink()
        || (metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0
}

#[cfg(not(windows))]
fn is_redirected(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
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
fn terminate_pid(pid: u32) -> Result<(), ModuleRuntimeError> {
    Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    Ok(())
}

#[cfg(not(windows))]
fn terminate_pid(pid: u32) -> Result<(), ModuleRuntimeError> {
    Command::new("kill")
        .args(["-9", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    Ok(())
}

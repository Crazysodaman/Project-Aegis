//! Supervised per-Core runtime (Stage 3).
//!
//! This crate records runtime supervision state in a dedicated
//! `core_runtime_state` table inside the Platform database. It does not replace
//! the Core registry and it never deletes Core databases or manifests.
//!
//! # Host process
//!
//! A Core is supervised by a real child process, `aegis-core-host`. The stock
//! host is resolved in this order:
//!
//! 1. the `AEGIS_CORE_HOST` environment variable, when set;
//! 2. the runtime `CARGO_BIN_EXE_aegis-core-host` environment variable (set by
//!    Cargo for integration tests);
//! 3. the compile-time `CARGO_BIN_EXE_aegis-core-host` value, when available;
//! 4. `aegis-core-host[.exe]` beside the current executable;
//! 5. `aegis-core-host[.exe]` in the parent directory of the current executable.
//!
//! The host receives `--root` and `--core-id`, verifies the Core database with
//! `CoreStorageLayout` / `CoreDatabase::read_snapshot`, writes
//! `<root>/run/<uuid>.heartbeat` every [`HEARTBEAT_INTERVAL_MS`] milliseconds,
//! watches `<root>/run/<uuid>.stop`, and exits when the sentinel appears or when
//! it can no longer write heartbeats.
//!
//! # Status
//!
//! [`CoreRuntime::status`] combines recorded state with heartbeat freshness. A
//! recorded live state whose heartbeat is missing or older than
//! [`HEARTBEAT_STALE_MS`] is reported as [`CoreRuntimeState::Stale`], never as
//! running. Heartbeats belonging to another PID are not trusted.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use aegis_contracts::core::validate_core_id;
use aegis_contracts::lifecycle::CoreLifecycleState;
use aegis_core_registry::{CoreRegistry, RegistryError};
use aegis_safe::{SafeError, SafeStore};
use aegis_storage::{
    now_rfc3339, open_initialized, StorageError, StoragePaths, PLATFORM_APPLICATION_ID,
};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use thiserror::Error;

/// Directory under the storage root that holds heartbeat and stop files.
pub const RUNTIME_RUN_DIR: &str = "run";
/// Heartbeat write interval for the supervised host process.
pub const HEARTBEAT_INTERVAL_MS: u64 = 200;
/// Age after which a recorded live heartbeat is considered stale.
pub const HEARTBEAT_STALE_MS: i64 = 1_500;
/// Maximum time to wait for the first heartbeat after spawning a host.
pub const START_TIMEOUT_MS: u64 = 3_000;
/// Maximum time to wait for a host to stop.
pub const STOP_TIMEOUT_MS: u64 = 3_000;

const RUNTIME_COMPONENT: &str = "core_runtime";
const POLL_INTERVAL_MS: u64 = 25;
static HEARTBEAT_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("runtime storage failed: {0}")]
    Storage(#[from] StorageError),
    #[error("runtime SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Core registry failed: {0}")]
    Registry(#[from] RegistryError),
    #[error("Safe authorization failed: {0}")]
    Safe(#[from] SafeError),
    #[error("owner authorization failed")]
    Unauthorized,
    #[error("operator STOP is active")]
    StopActive,
    #[error("Core is not registered: {0}")]
    UnknownCore(String),
    #[error("Core UUID is invalid")]
    InvalidCoreId,
    #[error("Core runtime transition from {from} to {to} is not permitted")]
    IllegalLifecycle { from: String, to: String },
    #[error("Core host could not be spawned: {0}")]
    SpawnFailure(String),
    #[error("Core host operation timed out: {0}")]
    Timeout(String),
    #[error("Core host process is stale: {0}")]
    StaleProcess(String),
    #[error("stored runtime state is invalid: {0}")]
    InvalidState(String),
    #[error("runtime I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("runtime JSON failed: {0}")]
    Json(#[from] serde_json::Error),
}

/// Recorded or effective state of one supervised Core.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreRuntimeState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Failed,
    Stale,
}

impl CoreRuntimeState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Stopping => "stopping",
            Self::Failed => "failed",
            Self::Stale => "stale",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "stopped" => Some(Self::Stopped),
            "starting" => Some(Self::Starting),
            "running" => Some(Self::Running),
            "stopping" => Some(Self::Stopping),
            "failed" => Some(Self::Failed),
            "stale" => Some(Self::Stale),
            _ => None,
        }
    }

    fn is_live(self) -> bool {
        matches!(self, Self::Starting | Self::Running | Self::Stopping)
    }
}

/// Effective runtime status for one Core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStatus {
    pub core_uuid: String,
    pub state: CoreRuntimeState,
    pub recorded_state: CoreRuntimeState,
    pub generation: u64,
    pub pid: Option<u32>,
    pub host_executable: Option<String>,
    pub heartbeat_at: Option<String>,
    pub started_at: Option<String>,
    pub updated_at: String,
    pub last_error: Option<String>,
    pub heartbeat_age_ms: Option<u64>,
    pub heartbeat_fresh: bool,
}

impl RuntimeStatus {
    pub fn is_running(&self) -> bool {
        self.state == CoreRuntimeState::Running && self.heartbeat_fresh
    }
}

/// Heartbeat payload written by the supervised host process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heartbeat {
    pub core_uuid: String,
    pub pid: u32,
    pub timestamp_ms: i64,
}

/// Platform-owned supervisor for per-Core host processes.
#[derive(Debug, Clone)]
pub struct CoreRuntime {
    paths: StoragePaths,
    registry: CoreRegistry,
    safe: SafeStore,
}

impl CoreRuntime {
    pub fn new(paths: StoragePaths) -> Self {
        Self {
            registry: CoreRegistry::new(paths.clone()),
            safe: SafeStore::new(paths.clone()),
            paths,
        }
    }

    /// Spawn and verify a host process for `core_uuid`.
    pub fn start(&self, core_uuid: &str, owner_token: &str) -> Result<RuntimeStatus, RuntimeError> {
        self.authorize(owner_token)?;
        let core_uuid = canonical_core_uuid(core_uuid)?;
        self.require_runtime_lifecycle(&core_uuid, "running")?;
        if self.safe.stop_status()?.active {
            return Err(RuntimeError::StopActive);
        }

        let connection = self.open()?;
        let existing = read_row(&connection, &core_uuid)?;
        let current = self.build_status(existing.clone(), &core_uuid);
        if current.recorded_state.is_live() {
            if current.state == CoreRuntimeState::Stale {
                match current.pid {
                    Some(pid) if process_is_alive(pid) => {
                        return Err(RuntimeError::StaleProcess(core_uuid))
                    }
                    _ => {
                        remove_if_exists(&heartbeat_path(self.paths.root(), &core_uuid))?;
                        remove_if_exists(&stop_sentinel_path(self.paths.root(), &core_uuid))?;
                        write_row(
                            &connection,
                            &StoredRuntime {
                                core_uuid: core_uuid.clone(),
                                state: CoreRuntimeState::Stopped,
                                generation: existing.as_ref().map_or(0, |row| row.generation),
                                pid: None,
                                host_executable: existing
                                    .as_ref()
                                    .and_then(|row| row.host_executable.clone()),
                                heartbeat_at: None,
                                started_at: existing
                                    .as_ref()
                                    .and_then(|row| row.started_at.clone()),
                                updated_at: now_rfc3339()?,
                                last_error: Some("recovered stale runtime state".to_owned()),
                            },
                        )?;
                    }
                }
            } else {
                return Err(RuntimeError::IllegalLifecycle {
                    from: current.state.as_str().to_owned(),
                    to: CoreRuntimeState::Running.as_str().to_owned(),
                });
            }
        }
        let existing = read_row(&connection, &core_uuid)?;

        let generation = existing
            .as_ref()
            .map_or(0, |row| row.generation)
            .checked_add(1)
            .ok_or_else(|| RuntimeError::InvalidState("generation".to_owned()))?;
        prepare_run_dir(self.paths.root())?;
        let heartbeat = heartbeat_path(self.paths.root(), &core_uuid);
        let sentinel = stop_sentinel_path(self.paths.root(), &core_uuid);
        remove_if_exists(&heartbeat)?;
        remove_if_exists(&sentinel)?;

        let host = resolve_host_executable()?;
        let host_executable = host.to_string_lossy().into_owned();
        let started_at = now_rfc3339()?;
        let mut starting = StoredRuntime {
            core_uuid: core_uuid.clone(),
            state: CoreRuntimeState::Starting,
            generation,
            pid: None,
            host_executable: Some(host_executable),
            heartbeat_at: None,
            started_at: Some(started_at),
            updated_at: now_rfc3339()?,
            last_error: None,
        };
        write_row(&connection, &starting)?;

        let mut child = match Command::new(&host)
            .arg("--root")
            .arg(self.paths.root())
            .arg("--core-id")
            .arg(&core_uuid)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                let message = format!("could not spawn {}: {error}", host.display());
                self.mark_failed(&connection, &core_uuid, &message)?;
                return Err(RuntimeError::SpawnFailure(message));
            }
        };
        let pid = child.id();
        starting.pid = Some(pid);
        write_row(&connection, &starting)?;

        let deadline = Instant::now() + Duration::from_millis(START_TIMEOUT_MS);
        loop {
            if let Some(status) = child.try_wait()? {
                let message = format!("Core host exited before heartbeat: {status}");
                self.mark_failed(&connection, &core_uuid, &message)?;
                remove_if_exists(&heartbeat)?;
                return Err(RuntimeError::SpawnFailure(message));
            }
            if let Some(beat) = read_heartbeat(&heartbeat) {
                if beat.core_uuid == core_uuid && beat.pid == pid {
                    let age = Utc::now()
                        .timestamp_millis()
                        .saturating_sub(beat.timestamp_ms)
                        .max(0);
                    if age <= HEARTBEAT_STALE_MS {
                        let running = StoredRuntime {
                            state: CoreRuntimeState::Running,
                            heartbeat_at: timestamp_to_rfc3339(beat.timestamp_ms),
                            updated_at: now_rfc3339()?,
                            last_error: None,
                            ..starting
                        };
                        write_row(&connection, &running)?;
                        if let Err(error) = self.activate_registry(&core_uuid) {
                            let _ = child.kill();
                            let _ = child.wait();
                            remove_if_exists(&heartbeat)?;
                            self.mark_failed(
                                &connection,
                                &core_uuid,
                                "runtime started but registry activation failed",
                            )?;
                            return Err(error);
                        }
                        let status = self.build_status(Some(running), &core_uuid);
                        reap_in_background(child);
                        return Ok(status);
                    }
                }
            }
            if Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
        }

        let _ = child.kill();
        let _ = child.wait();
        remove_if_exists(&heartbeat)?;
        let message = format!("timed out waiting for Core host heartbeat for {core_uuid}");
        self.mark_failed(&connection, &core_uuid, &message)?;
        Err(RuntimeError::Timeout(message))
    }

    /// Stop a supervised host without touching Core data.
    pub fn stop(&self, core_uuid: &str, owner_token: &str) -> Result<RuntimeStatus, RuntimeError> {
        self.authorize(owner_token)?;
        let core_uuid = canonical_core_uuid(core_uuid)?;
        self.require_runtime_lifecycle(&core_uuid, "stopped")?;

        let connection = self.open()?;
        let existing = read_row(&connection, &core_uuid)?;
        let current = self.build_status(existing.clone(), &core_uuid);
        let recorded_pid = existing.as_ref().and_then(|row| row.pid);
        prepare_run_dir(self.paths.root())?;
        let heartbeat = heartbeat_path(self.paths.root(), &core_uuid);
        let sentinel = stop_sentinel_path(self.paths.root(), &core_uuid);
        fs::write(&sentinel, b"stop")?;

        if current.recorded_state.is_live() {
            self.mark_stopping(&connection, &core_uuid)?;
            let deadline = Instant::now() + Duration::from_millis(STOP_TIMEOUT_MS);
            loop {
                if !heartbeat.exists() {
                    break;
                }
                if let Some(beat) = read_heartbeat(&heartbeat) {
                    let age = Utc::now()
                        .timestamp_millis()
                        .saturating_sub(beat.timestamp_ms);
                    if age > HEARTBEAT_STALE_MS {
                        break;
                    }
                }
                if Instant::now() >= deadline {
                    break;
                }
                thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
            }
        }

        if let Some(pid) = recorded_pid.or(current.pid) {
            if process_is_alive(pid) {
                let _ = terminate_pid(pid);
                let kill_deadline = Instant::now() + Duration::from_millis(STOP_TIMEOUT_MS);
                while process_is_alive(pid) && Instant::now() < kill_deadline {
                    thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
                }
                if process_is_alive(pid) {
                    let message = format!("Core host {pid} did not terminate for {core_uuid}");
                    self.mark_failed(&connection, &core_uuid, &message)?;
                    return Err(RuntimeError::Timeout(message));
                }
            }
        }

        remove_if_exists(&heartbeat)?;
        remove_if_exists(&sentinel)?;
        let stopped = StoredRuntime {
            core_uuid: core_uuid.clone(),
            state: CoreRuntimeState::Stopped,
            generation: existing
                .as_ref()
                .map_or(current.generation, |row| row.generation),
            pid: None,
            host_executable: existing
                .as_ref()
                .and_then(|row| row.host_executable.clone()),
            heartbeat_at: current
                .heartbeat_at
                .or_else(|| existing.as_ref().and_then(|row| row.heartbeat_at.clone())),
            started_at: existing.as_ref().and_then(|row| row.started_at.clone()),
            updated_at: now_rfc3339()?,
            last_error: None,
        };
        write_row(&connection, &stopped)?;
        self.deactivate_registry_if_active(&core_uuid)?;
        Ok(self.build_status(Some(stopped), &core_uuid))
    }

    /// Effective status for one registered Core.
    pub fn status(&self, core_uuid: &str) -> Result<RuntimeStatus, RuntimeError> {
        let core_uuid = canonical_core_uuid(core_uuid)?;
        self.registry.initialize()?;
        if self.registry.get(&core_uuid)?.is_none() {
            return Err(RuntimeError::UnknownCore(core_uuid));
        }
        let connection = self.open()?;
        let stored = read_row(&connection, &core_uuid)?;
        Ok(self.build_status(stored, &core_uuid))
    }

    /// Effective status for every registered Core.
    pub fn list(&self) -> Result<Vec<RuntimeStatus>, RuntimeError> {
        self.registry.initialize()?;
        let entries = self.registry.list()?;
        let connection = self.open()?;
        let mut statuses = Vec::with_capacity(entries.len());
        for entry in entries {
            let stored = read_row(&connection, &entry.core_uuid)?;
            statuses.push(self.build_status(stored, &entry.core_uuid));
        }
        Ok(statuses)
    }

    fn authorize(&self, owner_token: &str) -> Result<(), RuntimeError> {
        if !self.safe.verify_owner(owner_token)? {
            return Err(RuntimeError::Unauthorized);
        }
        Ok(())
    }

    fn activate_registry(&self, core_uuid: &str) -> Result<(), RuntimeError> {
        self.registry.initialize()?;
        let entry = self
            .registry
            .get(core_uuid)?
            .ok_or_else(|| RuntimeError::UnknownCore(core_uuid.to_owned()))?;
        match entry.lifecycle_state {
            CoreLifecycleState::Installed => {
                self.registry.activate(core_uuid, "runtime start")?;
            }
            CoreLifecycleState::Suspended => {
                self.registry.reactivate(core_uuid, "runtime start")?;
            }
            CoreLifecycleState::Active => {}
            other => {
                return Err(RuntimeError::IllegalLifecycle {
                    from: other.as_str().to_owned(),
                    to: CoreRuntimeState::Running.as_str().to_owned(),
                })
            }
        }
        Ok(())
    }

    fn deactivate_registry_if_active(&self, core_uuid: &str) -> Result<(), RuntimeError> {
        self.registry.initialize()?;
        let entry = self
            .registry
            .get(core_uuid)?
            .ok_or_else(|| RuntimeError::UnknownCore(core_uuid.to_owned()))?;
        if entry.lifecycle_state == CoreLifecycleState::Active {
            self.registry.deactivate(core_uuid, "runtime stop")?;
        }
        Ok(())
    }

    fn require_runtime_lifecycle(&self, core_uuid: &str, target: &str) -> Result<(), RuntimeError> {
        self.registry.initialize()?;
        let entry = self
            .registry
            .get(core_uuid)?
            .ok_or_else(|| RuntimeError::UnknownCore(core_uuid.to_owned()))?;
        match entry.lifecycle_state {
            CoreLifecycleState::Installed
            | CoreLifecycleState::Suspended
            | CoreLifecycleState::Active => Ok(()),
            other => Err(RuntimeError::IllegalLifecycle {
                from: other.as_str().to_owned(),
                to: target.to_owned(),
            }),
        }
    }

    fn open(&self) -> Result<Connection, RuntimeError> {
        let connection = open_initialized(
            &self.paths.platform_db(),
            PLATFORM_APPLICATION_ID,
            RUNTIME_COMPONENT,
        )?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS core_runtime_state (
                core_uuid TEXT PRIMARY KEY,
                state TEXT NOT NULL CHECK(state IN (
                    'stopped', 'starting', 'running', 'stopping', 'failed', 'stale'
                )),
                generation INTEGER NOT NULL CHECK(generation >= 0),
                pid INTEGER,
                host_executable TEXT,
                heartbeat_at TEXT,
                started_at TEXT,
                updated_at TEXT NOT NULL,
                last_error TEXT
            );",
        )?;
        Ok(connection)
    }

    fn mark_stopping(&self, connection: &Connection, core_uuid: &str) -> Result<(), RuntimeError> {
        connection.execute(
            "UPDATE core_runtime_state
             SET state = 'stopping', updated_at = ?2, last_error = NULL
             WHERE core_uuid = ?1",
            params![core_uuid, now_rfc3339()?],
        )?;
        Ok(())
    }

    fn mark_failed(
        &self,
        connection: &Connection,
        core_uuid: &str,
        message: &str,
    ) -> Result<(), RuntimeError> {
        connection.execute(
            "UPDATE core_runtime_state
             SET state = 'failed', pid = NULL, heartbeat_at = NULL,
                 updated_at = ?2, last_error = ?3
             WHERE core_uuid = ?1",
            params![core_uuid, now_rfc3339()?, message],
        )?;
        Ok(())
    }

    fn build_status(&self, stored: Option<StoredRuntime>, core_uuid: &str) -> RuntimeStatus {
        let now_ms = Utc::now().timestamp_millis();
        let Some(stored) = stored else {
            return RuntimeStatus {
                core_uuid: core_uuid.to_owned(),
                state: CoreRuntimeState::Stopped,
                recorded_state: CoreRuntimeState::Stopped,
                generation: 0,
                pid: None,
                host_executable: None,
                heartbeat_at: None,
                started_at: None,
                updated_at: timestamp_to_rfc3339(now_ms).unwrap_or_default(),
                last_error: None,
                heartbeat_age_ms: None,
                heartbeat_fresh: false,
            };
        };

        let mut status = RuntimeStatus {
            core_uuid: stored.core_uuid.clone(),
            state: stored.state,
            recorded_state: stored.state,
            generation: stored.generation,
            pid: stored.pid,
            host_executable: stored.host_executable.clone(),
            heartbeat_at: stored.heartbeat_at.clone(),
            started_at: stored.started_at.clone(),
            updated_at: stored.updated_at.clone(),
            last_error: stored.last_error.clone(),
            heartbeat_age_ms: None,
            heartbeat_fresh: false,
        };

        if stored.state.is_live() {
            let path = heartbeat_path(self.paths.root(), core_uuid);
            match read_heartbeat(&path) {
                Some(beat) if beat.core_uuid == core_uuid && stored.pid == Some(beat.pid) => {
                    let age = now_ms.saturating_sub(beat.timestamp_ms).max(0);
                    status.heartbeat_age_ms = Some(u64::try_from(age).unwrap_or_default());
                    status.heartbeat_at =
                        timestamp_to_rfc3339(beat.timestamp_ms).or(status.heartbeat_at);
                    if age <= HEARTBEAT_STALE_MS {
                        status.heartbeat_fresh = true;
                        if stored.state == CoreRuntimeState::Starting {
                            status.state = CoreRuntimeState::Running;
                        }
                    } else {
                        status.state = CoreRuntimeState::Stale;
                        status.last_error = Some("heartbeat is stale".to_owned());
                    }
                }
                _ => {
                    status.state = CoreRuntimeState::Stale;
                    status.last_error =
                        Some("heartbeat is missing or belongs to another process".to_owned());
                }
            }
        }
        status
    }
}

#[derive(Debug, Clone)]
struct StoredRuntime {
    core_uuid: String,
    state: CoreRuntimeState,
    generation: u64,
    pid: Option<u32>,
    host_executable: Option<String>,
    heartbeat_at: Option<String>,
    started_at: Option<String>,
    updated_at: String,
    last_error: Option<String>,
}

type RuntimeRow = (
    String,
    i64,
    Option<i64>,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
    Option<String>,
);

fn read_row(
    connection: &Connection,
    core_uuid: &str,
) -> Result<Option<StoredRuntime>, RuntimeError> {
    let row: Option<RuntimeRow> = connection
        .query_row(
            "SELECT state, generation, pid, host_executable, heartbeat_at,
                    started_at, updated_at, last_error
             FROM core_runtime_state WHERE core_uuid = ?1",
            [core_uuid],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            },
        )
        .optional()?;
    let Some((
        state_raw,
        generation,
        pid,
        host_executable,
        heartbeat_at,
        started_at,
        updated_at,
        last_error,
    )) = row
    else {
        return Ok(None);
    };

    let state = CoreRuntimeState::parse(&state_raw)
        .ok_or_else(|| RuntimeError::InvalidState(state_raw.to_owned()))?;
    let generation = u64::try_from(generation)
        .map_err(|_| RuntimeError::InvalidState("generation".to_owned()))?;
    let pid = match pid {
        Some(pid) => {
            Some(u32::try_from(pid).map_err(|_| RuntimeError::InvalidState("pid".to_owned()))?)
        }
        None => None,
    };
    Ok(Some(StoredRuntime {
        core_uuid: core_uuid.to_owned(),
        state,
        generation,
        pid,
        host_executable,
        heartbeat_at,
        started_at,
        updated_at,
        last_error,
    }))
}

fn write_row(connection: &Connection, row: &StoredRuntime) -> Result<(), RuntimeError> {
    let generation = i64::try_from(row.generation)
        .map_err(|_| RuntimeError::InvalidState("generation".to_owned()))?;
    let pid = row.pid.map(i64::from);
    connection.execute(
        "INSERT INTO core_runtime_state(
            core_uuid, state, generation, pid, host_executable, heartbeat_at,
            started_at, updated_at, last_error
         ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(core_uuid) DO UPDATE SET
            state = excluded.state,
            generation = excluded.generation,
            pid = excluded.pid,
            host_executable = excluded.host_executable,
            heartbeat_at = excluded.heartbeat_at,
            started_at = excluded.started_at,
            updated_at = excluded.updated_at,
            last_error = excluded.last_error",
        params![
            row.core_uuid.as_str(),
            row.state.as_str(),
            generation,
            pid,
            row.host_executable.as_deref(),
            row.heartbeat_at.as_deref(),
            row.started_at.as_deref(),
            row.updated_at.as_str(),
            row.last_error.as_deref(),
        ],
    )?;
    Ok(())
}

fn canonical_core_uuid(core_uuid: &str) -> Result<String, RuntimeError> {
    validate_core_id(core_uuid)
        .map(|uuid| uuid.hyphenated().to_string())
        .map_err(|_| RuntimeError::InvalidCoreId)
}

/// Directory that holds heartbeat and stop files.
pub fn run_dir(root: &Path) -> PathBuf {
    root.join(RUNTIME_RUN_DIR)
}

/// Path of the heartbeat file for one Core.
pub fn heartbeat_path(root: &Path, core_uuid: &str) -> PathBuf {
    run_dir(root).join(format!("{core_uuid}.heartbeat"))
}

/// Path of the stop sentinel for one Core.
pub fn stop_sentinel_path(root: &Path, core_uuid: &str) -> PathBuf {
    run_dir(root).join(format!("{core_uuid}.stop"))
}

/// Write a heartbeat atomically at `path`.
pub fn write_heartbeat(path: &Path, heartbeat: &Heartbeat) -> Result<(), RuntimeError> {
    let value = serde_json::json!({
        "core_uuid": heartbeat.core_uuid,
        "pid": heartbeat.pid,
        "timestamp_ms": heartbeat.timestamp_ms,
    });
    let bytes = serde_json::to_vec(&value)?;
    let counter = HEARTBEAT_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temporary = path.with_file_name(format!(
        ".{}.{}.{}.heartbeat.tmp",
        std::process::id(),
        counter,
        heartbeat.timestamp_ms
    ));
    fs::write(&temporary, bytes)?;
    match fs::rename(&temporary, path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            Err(error.into())
        }
    }
}

/// Create the runtime directory and reject redirected components.
pub fn prepare_run_dir(root: &Path) -> Result<PathBuf, RuntimeError> {
    let directory = run_dir(root);
    fs::create_dir_all(&directory)?;
    let metadata = fs::symlink_metadata(&directory)?;
    if metadata.file_type().is_symlink() {
        return Err(RuntimeError::InvalidState(
            "runtime directory is a symbolic link".to_owned(),
        ));
    }
    if !metadata.is_dir() {
        return Err(RuntimeError::InvalidState(
            "runtime directory is not a directory".to_owned(),
        ));
    }
    Ok(directory)
}

fn read_heartbeat(path: &Path) -> Option<Heartbeat> {
    let bytes = fs::read(path).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let core_uuid = value.get("core_uuid")?.as_str()?.to_owned();
    let pid = u32::try_from(value.get("pid")?.as_u64()?).ok()?;
    let timestamp_ms = value.get("timestamp_ms")?.as_i64()?;
    Some(Heartbeat {
        core_uuid,
        pid,
        timestamp_ms,
    })
}

fn timestamp_to_rfc3339(timestamp_ms: i64) -> Option<String> {
    DateTime::<Utc>::from_timestamp_millis(timestamp_ms).map(|timestamp| timestamp.to_rfc3339())
}

fn remove_if_exists(path: &Path) -> Result<(), RuntimeError> {
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

fn host_file_name() -> String {
    format!("aegis-core-host{}", std::env::consts::EXE_SUFFIX)
}

fn host_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if host_override_allowed() {
        if let Ok(value) = std::env::var("AEGIS_CORE_HOST") {
            candidates.push(PathBuf::from(value));
        }
    }
    if let Ok(value) = std::env::var("CARGO_BIN_EXE_aegis-core-host") {
        candidates.push(PathBuf::from(value));
    }
    if let Some(value) = option_env!("CARGO_BIN_EXE_aegis-core-host") {
        candidates.push(PathBuf::from(value));
    }
    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            candidates.push(directory.join(host_file_name()));
            if let Some(parent) = directory.parent() {
                candidates.push(parent.join(host_file_name()));
            }
        }
    }
    candidates
}

/// Resolve the `aegis-core-host` executable documented at the crate root.
///
/// `AEGIS_CORE_HOST` is honored only when
/// `AEGIS_ALLOW_UNVERIFIED_CORE_HOST=1` is also set. This keeps a poisoned
/// environment from silently executing an arbitrary binary in normal use.
pub fn resolve_host_executable() -> Result<PathBuf, RuntimeError> {
    for candidate in host_candidates() {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(RuntimeError::SpawnFailure(format!(
        "aegis-core-host executable could not be located; set AEGIS_CORE_HOST with AEGIS_ALLOW_UNVERIFIED_CORE_HOST=1 or place {} beside the runtime executable",
        host_file_name()
    )))
}

fn host_override_allowed() -> bool {
    matches!(
        std::env::var("AEGIS_ALLOW_UNVERIFIED_CORE_HOST").as_deref(),
        Ok("1") | Ok("true")
    )
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
                stdout.contains(&format!("\"{pid}\""))
                    || stdout
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
    std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|_| ())
}

#[cfg(not(windows))]
fn terminate_pid(pid: u32) -> Result<(), std::io::Error> {
    std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|_| ())
}

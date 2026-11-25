//! Explicit Fleet host registry, health probes, and inference placement.
//!
//! Hosts are never trusted automatically: adding a host records an untrusted
//! entry, and owner authorization is required to enroll/revoke/remove. Health
//! and model discovery are performed against the host's configured Ollama
//! endpoint; no hardware metrics are invented.

use std::path::Path;

use aegis_cognition::{InferenceError, ModelProvider, OllamaProvider};
use aegis_safe::{SafeError, SafeStore};
use aegis_storage::{
    now_rfc3339, open_initialized, StorageError, StoragePaths, PLATFORM_APPLICATION_ID,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

pub mod gpu;
pub mod supervisor;

pub use gpu::{
    evaluate_admission, parse_nvidia_smi_line, AdmissionDecision, GpuPolicy, GpuProbe,
    GpuTelemetry, NvidiaSmiProbe, DEFAULT_MAX_TEMPERATURE_C, DEFAULT_MAX_UTILIZATION_PERCENT,
    DEFAULT_MIN_FREE_VRAM_MB, GPU_ADMISSION_SCHEMA, GPU_TELEMETRY_SCHEMA,
};
pub use supervisor::{
    GpuSupervisor, SupervisionReport, SupervisorAction, SupervisorConfig, SupervisorMode,
    SUPERVISION_SCHEMA,
};

/// Schema identifier for a Fleet host record.
pub const FLEET_HOST_SCHEMA: &str = "aegis.fleet.host.v1";
/// Schema identifier for Fleet policy.
pub const FLEET_POLICY_SCHEMA: &str = "aegis.fleet.policy.v1";
/// Component name in the Platform database.
const FLEET_COMPONENT: &str = "fleet";

#[derive(Debug, Error)]
pub enum FleetError {
    #[error("fleet storage failed: {0}")]
    Storage(#[from] StorageError),
    #[error("fleet SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("fleet Safe operation failed: {0}")]
    Safe(#[from] SafeError),
    #[error("fleet JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("inference provider failed: {0}")]
    Provider(#[from] InferenceError),
    #[error("Platform is not initialized at {0}")]
    NotInitialized(String),
    #[error("owner authorization failed")]
    Unauthorized,
    #[error("host was not found: {0}")]
    HostNotFound(String),
    #[error("host callsign already exists: {0}")]
    HostExists(String),
    #[error("no enrolled healthy host can serve model {0}")]
    NoHealthyHost(String),
    #[error("invalid fleet input: {0}")]
    InvalidInput(&'static str),
    #[error("invalid GPU policy: {0}")]
    InvalidPolicy(String),
    #[error("invalid host endpoint: {0}")]
    InvalidEndpoint(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostRole {
    Inference,
    General,
    Storage,
}

impl HostRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Inference => "inference",
            Self::General => "general",
            Self::Storage => "storage",
        }
    }

    pub fn parse(value: &str) -> Result<Self, FleetError> {
        match value {
            "inference" => Ok(Self::Inference),
            "general" => Ok(Self::General),
            "storage" => Ok(Self::Storage),
            _ => Err(FleetError::InvalidInput("host role")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostHealth {
    pub reachable: bool,
    pub model_count: u64,
    pub latency_ms: u64,
    pub error: Option<String>,
    pub checked_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FleetHost {
    pub schema: String,
    pub host_id: String,
    pub callsign: String,
    pub endpoint: String,
    pub role: HostRole,
    pub capabilities: Value,
    pub enrolled: bool,
    pub trusted: bool,
    pub priority: u8,
    pub created_at: String,
    pub updated_at: String,
    pub revision: u64,
    pub health: Option<HostHealth>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FleetPolicy {
    pub schema: String,
    pub gaming_yield: bool,
    pub max_utilization_percent: u8,
    pub min_free_vram_mb: u32,
    pub max_temperature_c: u8,
    pub updated_at: String,
}

impl FleetPolicy {
    pub fn gpu_policy(&self) -> GpuPolicy {
        GpuPolicy {
            gaming_yield: self.gaming_yield,
            max_utilization_percent: self.max_utilization_percent,
            min_free_vram_mb: self.min_free_vram_mb,
            max_temperature_c: self.max_temperature_c,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlacementCandidate {
    pub host_id: String,
    pub callsign: String,
    pub endpoint: String,
    pub role: HostRole,
    pub priority: u8,
    pub latency_ms: Option<u64>,
}

/// Platform-owned Fleet registry.
#[derive(Debug, Clone)]
pub struct FleetStore {
    paths: StoragePaths,
    safe: SafeStore,
}

impl FleetStore {
    pub fn new(paths: StoragePaths) -> Self {
        let safe = SafeStore::new(paths.clone());
        Self { paths, safe }
    }

    pub fn initialize(&self) -> Result<(), FleetError> {
        self.open().map(|_| ())
    }

    /// Add an untrusted host. Requires owner proof.
    pub fn add_host(
        &self,
        callsign: &str,
        endpoint: &str,
        role: HostRole,
        capabilities: Value,
        priority: u8,
        owner_token: &str,
    ) -> Result<FleetHost, FleetError> {
        self.authorize(owner_token)?;
        let callsign = validate_callsign(callsign)?;
        let endpoint = validate_endpoint(endpoint)?;
        if priority > 100 {
            return Err(FleetError::InvalidInput("priority"));
        }
        if !capabilities.is_null() && !capabilities.is_object() {
            return Err(FleetError::InvalidInput("capabilities"));
        }
        let encoded = serde_json::to_vec(&capabilities)?;
        if encoded.len() > 8192 {
            return Err(FleetError::InvalidInput("capabilities"));
        }
        let connection = self.open()?;
        let existing: Option<String> = connection
            .query_row(
                "SELECT host_id FROM fleet_hosts WHERE callsign = ?1",
                [callsign.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        if existing.is_some() {
            return Err(FleetError::HostExists(callsign));
        }
        let host_id = Uuid::new_v4().to_string();
        let now = now_rfc3339()?;
        connection.execute(
            "INSERT INTO fleet_hosts(
                host_id, schema, callsign, endpoint, role, capabilities_json,
                enrolled, trusted, priority, created_at, updated_at, revision
             ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, 0, 0, ?7, ?8, ?8, 1)",
            params![
                host_id,
                FLEET_HOST_SCHEMA,
                callsign,
                endpoint,
                role.as_str(),
                serde_json::to_string(&capabilities)?,
                i64::from(priority),
                now
            ],
        )?;
        self.record_event(
            &host_id,
            "host.added",
            &serde_json::json!({"callsign": callsign, "role": role.as_str()}),
        )?;
        self.host(&host_id)
    }

    pub fn list_hosts(&self) -> Result<Vec<FleetHost>, FleetError> {
        let connection = self.open()?;
        let mut statement = connection.prepare(&format!(
            "SELECT {HOST_COLUMNS} FROM fleet_hosts ORDER BY priority DESC, callsign ASC"
        ))?;
        let mut rows = statement.query([])?;
        let mut hosts = Vec::new();
        while let Some(row) = rows.next()? {
            hosts.push(read_host(row)?);
        }
        Ok(hosts)
    }

    pub fn host(&self, host_id: &str) -> Result<FleetHost, FleetError> {
        let connection = self.open()?;
        connection
            .query_row(
                &format!("SELECT {HOST_COLUMNS} FROM fleet_hosts WHERE host_id = ?1"),
                [host_id],
                read_host,
            )
            .optional()?
            .ok_or_else(|| FleetError::HostNotFound(host_id.to_owned()))
    }

    /// Explicitly enroll and trust a host. Requires owner proof.
    pub fn enroll(&self, host_id: &str, owner_token: &str) -> Result<FleetHost, FleetError> {
        self.authorize(owner_token)?;
        self.set_trust(host_id, true, "host.enrolled")
    }

    /// Revoke host trust. Requires owner proof. Effective immediately.
    pub fn revoke(&self, host_id: &str, owner_token: &str) -> Result<FleetHost, FleetError> {
        self.authorize(owner_token)?;
        self.set_trust(host_id, false, "host.revoked")
    }

    /// Remove a host record. Requires owner proof.
    pub fn remove(&self, host_id: &str, owner_token: &str) -> Result<(), FleetError> {
        self.authorize(owner_token)?;
        let connection = self.open()?;
        let existing = self.host(host_id)?;
        self.record_event(
            host_id,
            "host.removed",
            &serde_json::json!({"callsign": existing.callsign}),
        )?;
        connection.execute("DELETE FROM fleet_hosts WHERE host_id = ?1", [host_id])?;
        Ok(())
    }

    /// Probe the host's Ollama endpoint and record health/model count.
    pub fn probe(&self, host_id: &str) -> Result<FleetHost, FleetError> {
        let host = self.host(host_id)?;
        let provider = OllamaProvider::new(host.endpoint.clone())?;
        let health = provider.health();
        let checked_at = now_rfc3339()?;
        let connection = self.open()?;
        connection.execute(
            "UPDATE fleet_hosts
             SET health_reachable = ?1, health_models = ?2, health_latency_ms = ?3,
                 health_error = ?4, health_checked_at = ?5,
                 updated_at = ?5, revision = revision + 1
             WHERE host_id = ?6",
            params![
                i64::from(u8::from(health.reachable)),
                health.model_count as i64,
                health.latency_ms as i64,
                health.error.as_deref(),
                checked_at,
                host_id
            ],
        )?;
        self.record_event(
            host_id,
            "host.probed",
            &serde_json::json!({
                "reachable": health.reachable,
                "model_count": health.model_count,
            }),
        )?;
        self.host(host_id)
    }

    /// Enrolled, trusted, health-known hosts ordered by priority then latency.
    pub fn placements(&self) -> Result<Vec<PlacementCandidate>, FleetError> {
        let mut candidates: Vec<PlacementCandidate> = self
            .list_hosts()?
            .into_iter()
            .filter(|host| {
                host.enrolled
                    && host.trusted
                    && host.health.as_ref().is_some_and(|health| health.reachable)
            })
            .map(|host| PlacementCandidate {
                host_id: host.host_id,
                callsign: host.callsign,
                endpoint: host.endpoint,
                role: host.role,
                priority: host.priority,
                latency_ms: host.health.map(|health| health.latency_ms),
            })
            .collect();
        candidates.sort_by(|left, right| {
            right
                .priority
                .cmp(&left.priority)
                .then_with(|| {
                    left.latency_ms
                        .unwrap_or(u64::MAX)
                        .cmp(&right.latency_ms.unwrap_or(u64::MAX))
                })
                .then_with(|| left.callsign.cmp(&right.callsign))
        });
        Ok(candidates)
    }

    /// Select an enrolled healthy host that currently exposes `model`.
    pub fn route_inference(&self, model: &str) -> Result<PlacementCandidate, FleetError> {
        let model = model.trim();
        if model.is_empty() {
            return Err(FleetError::InvalidInput("model"));
        }
        for candidate in self.placements()? {
            let provider = OllamaProvider::new(candidate.endpoint.clone())?;
            if let Ok(models) = provider.list_models() {
                if models.iter().any(|info| info.name == model) {
                    return Ok(candidate);
                }
            }
        }
        Err(FleetError::NoHealthyHost(model.to_owned()))
    }

    pub fn policy(&self) -> Result<FleetPolicy, FleetError> {
        let connection = self.open()?;
        connection
            .query_row(
                "SELECT gaming_yield, max_utilization_percent, min_free_vram_mb,
                        max_temperature_c, updated_at
                 FROM fleet_policy WHERE singleton = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()?
            .map(
                |(gaming_yield, max_util, min_free, max_temp, updated_at)| FleetPolicy {
                    schema: FLEET_POLICY_SCHEMA.to_owned(),
                    gaming_yield: gaming_yield != 0,
                    max_utilization_percent: max_util.clamp(0, 100) as u8,
                    min_free_vram_mb: min_free.max(0) as u32,
                    max_temperature_c: max_temp.clamp(0, 110) as u8,
                    updated_at,
                },
            )
            .ok_or(FleetError::InvalidInput("policy"))
    }

    pub fn set_policy(
        &self,
        gaming_yield: bool,
        owner_token: &str,
    ) -> Result<FleetPolicy, FleetError> {
        self.authorize(owner_token)?;
        let current = self.policy()?;
        self.write_policy(
            GpuPolicy {
                gaming_yield,
                ..current.gpu_policy()
            },
            owner_token,
        )
    }

    /// Update all GPU limits, preserving the current gaming-yield flag if the
    /// supplied policy matches it. Requires owner proof.
    pub fn set_gpu_policy(
        &self,
        policy: GpuPolicy,
        owner_token: &str,
    ) -> Result<FleetPolicy, FleetError> {
        self.authorize(owner_token)?;
        self.write_policy(policy, owner_token)
    }

    pub fn admission(&self, telemetry: &GpuTelemetry) -> Result<AdmissionDecision, FleetError> {
        Ok(evaluate_admission(&self.policy()?.gpu_policy(), telemetry))
    }

    fn write_policy(
        &self,
        policy: GpuPolicy,
        owner_token: &str,
    ) -> Result<FleetPolicy, FleetError> {
        self.authorize(owner_token)?;
        policy.validate().map_err(FleetError::InvalidPolicy)?;
        let connection = self.open()?;
        let now = now_rfc3339()?;
        connection.execute(
            "INSERT INTO fleet_policy(
                singleton, gaming_yield, max_utilization_percent, min_free_vram_mb,
                max_temperature_c, updated_at
             ) VALUES(1, ?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(singleton) DO UPDATE SET
                 gaming_yield = excluded.gaming_yield,
                 max_utilization_percent = excluded.max_utilization_percent,
                 min_free_vram_mb = excluded.min_free_vram_mb,
                 max_temperature_c = excluded.max_temperature_c,
                 updated_at = excluded.updated_at",
            params![
                i64::from(u8::from(policy.gaming_yield)),
                i64::from(policy.max_utilization_percent),
                i64::from(policy.min_free_vram_mb),
                i64::from(policy.max_temperature_c),
                now
            ],
        )?;
        self.safe.record_event(
            "fleet.policy.changed",
            serde_json::json!({
                "gaming_yield": policy.gaming_yield,
                "max_utilization_percent": policy.max_utilization_percent,
                "min_free_vram_mb": policy.min_free_vram_mb,
                "max_temperature_c": policy.max_temperature_c,
            }),
        )?;
        self.policy()
    }

    fn set_trust(
        &self,
        host_id: &str,
        trusted: bool,
        event: &str,
    ) -> Result<FleetHost, FleetError> {
        self.host(host_id)?;
        let connection = self.open()?;
        connection.execute(
            "UPDATE fleet_hosts
             SET enrolled = ?1, trusted = ?1, updated_at = ?2, revision = revision + 1
             WHERE host_id = ?3",
            params![i64::from(u8::from(trusted)), now_rfc3339()?, host_id],
        )?;
        self.record_event(host_id, event, &serde_json::json!({"trusted": trusted}))?;
        self.host(host_id)
    }

    fn authorize(&self, owner_token: &str) -> Result<(), FleetError> {
        if !self.paths.platform_db().is_file() {
            return Err(FleetError::NotInitialized(
                self.paths.root().display().to_string(),
            ));
        }
        if !self.safe.verify_owner(owner_token)? {
            return Err(FleetError::Unauthorized);
        }
        Ok(())
    }

    fn open(&self) -> Result<Connection, FleetError> {
        if !self.paths.platform_db().is_file() {
            return Err(FleetError::NotInitialized(
                self.paths.root().display().to_string(),
            ));
        }
        let connection = open_initialized(
            &self.paths.platform_db(),
            PLATFORM_APPLICATION_ID,
            FLEET_COMPONENT,
        )?;
        ensure_tables(&connection)?;
        Ok(connection)
    }

    fn record_event(
        &self,
        host_id: &str,
        event_type: &str,
        detail: &Value,
    ) -> Result<(), FleetError> {
        let connection = self.open()?;
        connection.execute(
            "INSERT INTO fleet_events(host_id, event_type, detail_json, created_at)
             VALUES(?1, ?2, ?3, ?4)",
            params![
                host_id,
                event_type,
                serde_json::to_string(detail)?,
                now_rfc3339()?
            ],
        )?;
        Ok(())
    }
}

fn ensure_tables(connection: &Connection) -> Result<(), FleetError> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS fleet_hosts (
            host_id TEXT PRIMARY KEY,
            schema TEXT NOT NULL,
            callsign TEXT NOT NULL UNIQUE,
            endpoint TEXT NOT NULL,
            role TEXT NOT NULL CHECK(role IN ('inference', 'general', 'storage')),
            capabilities_json TEXT NOT NULL,
            enrolled INTEGER NOT NULL CHECK(enrolled IN (0, 1)),
            trusted INTEGER NOT NULL CHECK(trusted IN (0, 1)),
            priority INTEGER NOT NULL CHECK(priority >= 0 AND priority <= 100),
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            revision INTEGER NOT NULL CHECK(revision >= 1),
            health_reachable INTEGER,
            health_models INTEGER,
            health_latency_ms INTEGER,
            health_error TEXT,
            health_checked_at TEXT
        );
        CREATE TABLE IF NOT EXISTS fleet_events (
            event_id INTEGER PRIMARY KEY AUTOINCREMENT,
            host_id TEXT NOT NULL,
            event_type TEXT NOT NULL,
            detail_json TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS fleet_events_by_host ON fleet_events(host_id, event_id);
        CREATE TABLE IF NOT EXISTS fleet_policy (
            singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
            gaming_yield INTEGER NOT NULL CHECK(gaming_yield IN (0, 1)),
            max_utilization_percent INTEGER NOT NULL DEFAULT 25
                CHECK(max_utilization_percent >= 0 AND max_utilization_percent <= 100),
            min_free_vram_mb INTEGER NOT NULL DEFAULT 2048 CHECK(min_free_vram_mb > 0),
            max_temperature_c INTEGER NOT NULL DEFAULT 80
                CHECK(max_temperature_c >= 30 AND max_temperature_c <= 110),
            updated_at TEXT NOT NULL
        );
        INSERT OR IGNORE INTO fleet_policy(
            singleton, gaming_yield, max_utilization_percent, min_free_vram_mb,
            max_temperature_c, updated_at
        ) VALUES(1, 1, 25, 2048, 80, 'initial');
        CREATE TRIGGER IF NOT EXISTS fleet_events_no_update
        BEFORE UPDATE ON fleet_events
        BEGIN
            SELECT RAISE(ABORT, 'fleet events are append-only');
        END;
        CREATE TRIGGER IF NOT EXISTS fleet_events_no_delete
        BEFORE DELETE ON fleet_events
        BEGIN
            SELECT RAISE(ABORT, 'fleet events are append-only');
        END;",
    )?;
    ensure_column(
        connection,
        "fleet_policy",
        "max_utilization_percent",
        "INTEGER NOT NULL DEFAULT 25",
    )?;
    ensure_column(
        connection,
        "fleet_policy",
        "min_free_vram_mb",
        "INTEGER NOT NULL DEFAULT 2048",
    )?;
    ensure_column(
        connection,
        "fleet_policy",
        "max_temperature_c",
        "INTEGER NOT NULL DEFAULT 80",
    )?;
    Ok(())
}

fn ensure_column(
    connection: &Connection,
    table: &str,
    column: &str,
    declaration: &str,
) -> Result<(), FleetError> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = statement.query([])?;
    let mut exists = false;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            exists = true;
            break;
        }
    }
    drop(rows);
    drop(statement);
    if !exists {
        connection.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {declaration}"),
            [],
        )?;
    }
    Ok(())
}

const HOST_COLUMNS: &str = "host_id, schema, callsign, endpoint, role, capabilities_json, \
                            enrolled, trusted, priority, created_at, updated_at, revision, \
                            health_reachable, health_models, health_latency_ms, health_error, \
                            health_checked_at";

fn read_host(row: &rusqlite::Row<'_>) -> rusqlite::Result<FleetHost> {
    let role: String = row.get(4)?;
    let role = HostRole::parse(&role).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            4,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid host role",
            )),
        )
    })?;
    let capabilities_json: String = row.get(5)?;
    let enrolled: i64 = row.get(6)?;
    let trusted: i64 = row.get(7)?;
    let priority: i64 = row.get(8)?;
    let revision: i64 = row.get(11)?;
    if revision < 1 || !(0..=100).contains(&priority) {
        return Err(rusqlite::Error::InvalidColumnType(
            8,
            "host bounds".to_owned(),
            rusqlite::types::Type::Integer,
        ));
    }
    let checked_at: Option<String> = row.get(16)?;
    let health = match checked_at {
        Some(checked_at) => {
            let reachable: Option<i64> = row.get(12)?;
            let models: Option<i64> = row.get(13)?;
            let latency: Option<i64> = row.get(14)?;
            Some(HostHealth {
                reachable: reachable.unwrap_or(0) != 0,
                model_count: models.unwrap_or(0).max(0) as u64,
                latency_ms: latency.unwrap_or(0).max(0) as u64,
                error: row.get(15)?,
                checked_at,
            })
        }
        None => None,
    };
    Ok(FleetHost {
        schema: FLEET_HOST_SCHEMA.to_owned(),
        host_id: row.get(0)?,
        callsign: row.get(2)?,
        endpoint: row.get(3)?,
        role,
        capabilities: serde_json::from_str(&capabilities_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                5,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        enrolled: enrolled != 0,
        trusted: trusted != 0,
        priority: priority as u8,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        revision: revision as u64,
        health,
    })
}

fn validate_callsign(value: &str) -> Result<String, FleetError> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > 64
        || trimmed.chars().any(char::is_control)
        || trimmed.starts_with('-')
        || trimmed.ends_with('-')
    {
        return Err(FleetError::InvalidInput("host callsign"));
    }
    Ok(trimmed.to_owned())
}

fn validate_endpoint(value: &str) -> Result<String, FleetError> {
    let trimmed = value.trim();
    if trimmed.starts_with("https://") {
        return Err(FleetError::InvalidEndpoint(
            "https is not supported by the built-in Ollama adapter".to_owned(),
        ));
    }
    let rest = trimmed.strip_prefix("http://").ok_or_else(|| {
        FleetError::InvalidEndpoint("only http:// endpoints are supported".to_owned())
    })?;
    if rest.is_empty() || rest.contains('@') || rest.chars().any(char::is_control) {
        return Err(FleetError::InvalidEndpoint(value.to_owned()));
    }
    Ok(trimmed.trim_end_matches('/').to_owned())
}

/// Convenience constructor for a Platform root.
pub fn fleet_store(root: &Path) -> Result<FleetStore, FleetError> {
    let paths = StoragePaths::new(root)?;
    let store = FleetStore::new(paths);
    store.initialize()?;
    Ok(store)
}

//! Per-Core autonomy persistence: goals, plans, scheduled tasks, settings and an
//! append-only autonomy event ledger.
//!
//! Autonomy lives in a dedicated `<core>/autonomy.db` bound to both the Core
//! UUID and the Core database instance UUID. Plan and task rows belong to
//! exactly one Core; the store never opens another Core's database and never
//! writes to `core.db`, `memory.db`, `personal.db`, `possessions.db`, or
//! `consent.db`.
//!
//! This module stores and schedules work only. It never fabricates task
//! outcomes: execution is performed by the engine against real Core stores.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use aegis_contracts::core::validate_core_id;
use aegis_contracts::living::{
    AutonomySettings, Goal, GoalStatus, Plan, Task, TaskStatus, LIVING_CONTRACT_VERSION,
    MAX_ID_CHARS, MAX_TAG_CHARS,
};
use aegis_core_storage::{CoreDatabase, CoreStorageLayout};
use aegis_storage::open_initialized;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{canonical_core_id, chars_within, now_rfc3339, SdkError};

/// File name of the per-Core autonomy database.
pub const AUTONOMY_DB_FILE: &str = "autonomy.db";
/// SQLite application marker for a per-Core autonomy database ("AEGU").
pub const AUTONOMY_APPLICATION_ID: i64 = 0x4145_4755;
/// Component name registered in the shared `aegis_schema` table.
pub const AUTONOMY_COMPONENT: &str = "core_autonomy";
/// Schema identifier for the autonomy database.
pub const AUTONOMY_SCHEMA: &str = "aegis.core.autonomy.v1";
/// Schema identifier for one autonomy event.
pub const AUTONOMY_EVENT_SCHEMA: &str = "aegis.core.autonomy.event.v1";
/// Current autonomy database schema revision.
pub const AUTONOMY_SCHEMA_REVISION: u64 = 1;

/// Ownership table for the autonomy database.
pub const AUTONOMY_OWNERSHIP_TABLE: &str = "autonomy_ownership";
/// Schema marker table for the autonomy database.
pub const AUTONOMY_SCHEMA_TABLE: &str = "autonomy_schema";
/// Goal table.
pub const AUTONOMY_GOALS_TABLE: &str = "goals";
/// Plan table.
pub const AUTONOMY_PLANS_TABLE: &str = "plans";
/// Task table.
pub const AUTONOMY_TASKS_TABLE: &str = "tasks";
/// Autonomy settings table.
pub const AUTONOMY_SETTINGS_TABLE: &str = "autonomy_settings";
/// Append-only autonomy event table.
pub const AUTONOMY_EVENTS_TABLE: &str = "autonomy_events";

/// Maximum number of goals, plans, or tasks returned by one listing.
pub const MAX_AUTONOMY_LIST: usize = 256;
/// Maximum number of autonomy events returned by one listing.
pub const MAX_AUTONOMY_EVENT_LIST: usize = 500;
/// Maximum serialized size of an autonomy event detail payload.
pub const MAX_AUTONOMY_EVENT_DETAIL_BYTES: usize = 16 * 1024;
/// Default maximum number of concurrently active tasks.
pub const DEFAULT_MAX_ACTIVE_TASKS: u8 = 4;

const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS autonomy_ownership (
    singleton INTEGER NOT NULL PRIMARY KEY CHECK (singleton = 1),
    core_uuid TEXT NOT NULL,
    database_instance_uuid TEXT NOT NULL,
    schema TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS autonomy_schema (
    singleton INTEGER NOT NULL PRIMARY KEY CHECK (singleton = 1),
    schema TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS goals (
    goal_id TEXT NOT NULL PRIMARY KEY,
    core_uuid TEXT NOT NULL,
    status TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1)
);
CREATE INDEX IF NOT EXISTS goals_core_status_idx ON goals(core_uuid, status, updated_at);
CREATE TABLE IF NOT EXISTS plans (
    plan_id TEXT NOT NULL PRIMARY KEY,
    core_uuid TEXT NOT NULL,
    goal_id TEXT,
    status TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1)
);
CREATE INDEX IF NOT EXISTS plans_core_status_idx ON plans(core_uuid, status, updated_at);
CREATE TABLE IF NOT EXISTS tasks (
    task_id TEXT NOT NULL PRIMARY KEY,
    core_uuid TEXT NOT NULL,
    plan_id TEXT,
    goal_id TEXT,
    status TEXT NOT NULL,
    priority INTEGER NOT NULL CHECK (priority BETWEEN 0 AND 255),
    due_at TEXT,
    progress_percent INTEGER NOT NULL CHECK (progress_percent BETWEEN 0 AND 100),
    payload_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1)
);
CREATE INDEX IF NOT EXISTS tasks_core_status_idx ON tasks(core_uuid, status, due_at);
CREATE INDEX IF NOT EXISTS tasks_core_plan_idx ON tasks(core_uuid, plan_id);
CREATE TABLE IF NOT EXISTS autonomy_settings (
    singleton INTEGER NOT NULL PRIMARY KEY CHECK (singleton = 1),
    core_uuid TEXT NOT NULL,
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    max_active_tasks INTEGER NOT NULL CHECK (max_active_tasks BETWEEN 1 AND 16),
    payload_json TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1)
);
CREATE TABLE IF NOT EXISTS autonomy_events (
    sequence INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    core_uuid TEXT NOT NULL,
    entity_kind TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    action TEXT NOT NULL,
    detail_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS autonomy_events_core_idx
    ON autonomy_events(core_uuid, sequence);
CREATE TRIGGER IF NOT EXISTS autonomy_ownership_no_update
BEFORE UPDATE ON autonomy_ownership
BEGIN
    SELECT RAISE(ABORT, 'autonomy_ownership is immutable');
END;
CREATE TRIGGER IF NOT EXISTS autonomy_ownership_no_delete
BEFORE DELETE ON autonomy_ownership
BEGIN
    SELECT RAISE(ABORT, 'autonomy_ownership is immutable');
END;
CREATE TRIGGER IF NOT EXISTS autonomy_events_no_update
BEFORE UPDATE ON autonomy_events
BEGIN
    SELECT RAISE(ABORT, 'autonomy_events is append-only');
END;
CREATE TRIGGER IF NOT EXISTS autonomy_events_no_delete
BEFORE DELETE ON autonomy_events
BEGIN
    SELECT RAISE(ABORT, 'autonomy_events is append-only');
END;
"#;

/// One append-only autonomy event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutonomyEvent {
    /// Event schema identifier.
    pub schema: String,
    /// Monotonic sequence within this Core's database.
    pub sequence: u64,
    /// Owning Core.
    pub core_uuid: String,
    /// Entity kind (`goal`, `plan`, `task`, `settings`).
    pub entity_kind: String,
    /// Entity identifier.
    pub entity_id: String,
    /// Action name.
    pub action: String,
    /// JSON detail payload.
    pub detail: Value,
    /// Creation timestamp.
    pub created_at: String,
}

/// Writable per-Core autonomy store.
///
/// Owns a `rusqlite::Connection` bound to `<core>/autonomy.db`. It never opens
/// another Core's database and never writes to any other per-Core database.
pub struct AutonomyStore {
    core_uuid: String,
    database_instance_uuid: String,
    core_dir: PathBuf,
    database_path: PathBuf,
    connection: Connection,
}

impl std::fmt::Debug for AutonomyStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AutonomyStore")
            .field("core_uuid", &self.core_uuid)
            .field("database_instance_uuid", &self.database_instance_uuid)
            .field("database_path", &self.database_path)
            .finish_non_exhaustive()
    }
}

impl AutonomyStore {
    /// Open (or initialize) `<core>/autonomy.db` for an existing Core.
    ///
    /// The Core database is read only; it supplies the immutable Core UUID and
    /// database instance UUID. An existing autonomy database must carry exactly
    /// one ownership row matching both values.
    pub fn open(root: impl AsRef<Path>, core_uuid: &str) -> Result<Self, SdkError> {
        let core_uuid = canonical_core_id(core_uuid)?;
        let layout = CoreStorageLayout::new(root)?;
        let snapshot = CoreDatabase::read_snapshot(&layout, &core_uuid)?;
        if snapshot.identity.core_uuid != core_uuid {
            return Err(SdkError::OwnershipMismatch);
        }
        let database_instance_uuid = snapshot.metadata.database_instance_uuid.clone();
        validate_core_id(&database_instance_uuid).map_err(|_| SdkError::OwnershipMismatch)?;

        let core_dir = layout.core_dir(&core_uuid)?;
        let autonomy_path = core_dir.join(AUTONOMY_DB_FILE);
        let existed = match fs::symlink_metadata(&autonomy_path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(SdkError::ForeignPath);
                }
                true
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => return Err(error.into()),
        };

        let connection =
            open_initialized(&autonomy_path, AUTONOMY_APPLICATION_ID, AUTONOMY_COMPONENT)?;

        let canonical_dir = fs::canonicalize(&core_dir)?;
        let canonical_db = fs::canonicalize(&autonomy_path)?;
        if canonical_db.parent() != Some(canonical_dir.as_path())
            || canonical_db.file_name() != Some(OsStr::new(AUTONOMY_DB_FILE))
        {
            return Err(SdkError::ForeignPath);
        }

        let has_ownership = table_exists(&connection, AUTONOMY_OWNERSHIP_TABLE)?;
        if existed && !has_ownership {
            return Err(SdkError::OwnershipMissing);
        }
        if has_ownership {
            verify_ownership(&connection, &core_uuid, &database_instance_uuid)?;
        }

        initialize_schema(&connection)?;
        ensure_schema_marker(&connection)?;

        if !has_ownership {
            connection.execute(
                "INSERT INTO autonomy_ownership(
                    singleton, core_uuid, database_instance_uuid, schema, created_at
                 ) VALUES(1, ?1, ?2, ?3, ?4)",
                params![
                    core_uuid,
                    database_instance_uuid,
                    AUTONOMY_SCHEMA,
                    now_rfc3339()?
                ],
            )?;
        }

        let store = Self {
            core_uuid,
            database_instance_uuid,
            core_dir: canonical_dir,
            database_path: canonical_db,
            connection,
        };
        store.ensure_default_settings()?;
        Ok(store)
    }

    /// Immutable Core UUID bound to this store.
    pub fn core_uuid(&self) -> &str {
        &self.core_uuid
    }

    /// Core database instance UUID this store is bound to.
    pub fn database_instance_uuid(&self) -> &str {
        &self.database_instance_uuid
    }

    /// Canonical Core directory.
    pub fn core_dir(&self) -> &Path {
        &self.core_dir
    }

    /// Canonical path of `autonomy.db`.
    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    /// Create a goal owned by this Core.
    pub fn create_goal(&self, goal: &Goal) -> Result<Goal, SdkError> {
        goal.validate()
            .map_err(|_| SdkError::InvalidMemory("goal failed validation"))?;
        let mut goal = goal.clone();
        goal.core_uuid = self.bound_core(&goal.core_uuid)?;
        let transaction = self.connection.unchecked_transaction()?;
        let existing: Option<i64> = transaction
            .query_row(
                "SELECT 1 FROM goals WHERE goal_id = ?1",
                params![goal.goal_id],
                |row| row.get(0),
            )
            .optional()?;
        if existing.is_some() {
            return Err(SdkError::InvalidMemory("goal already exists"));
        }
        let payload_json = serde_json::to_string(&goal)?;
        let revision = revision_value(goal.revision)?;
        transaction.execute(
            "INSERT INTO goals(
                goal_id, core_uuid, status, payload_json, created_at, updated_at, revision
             ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                goal.goal_id,
                self.core_uuid,
                goal_status_str(goal.status),
                payload_json,
                goal.created_at,
                goal.updated_at,
                revision
            ],
        )?;
        insert_event(
            &transaction,
            &self.core_uuid,
            "goal",
            &goal.goal_id,
            "created",
            &json!({
                "goal_id": goal.goal_id,
                "title": goal.title,
                "status": goal_status_str(goal.status),
                "revision": goal.revision,
            }),
            &now_rfc3339()?,
        )?;
        transaction.commit()?;
        Ok(goal)
    }

    /// Fetch one goal owned by this Core.
    pub fn get_goal(&self, goal_id: &str) -> Result<Option<Goal>, SdkError> {
        if !valid_id(goal_id) {
            return Ok(None);
        }
        read_goal(&self.connection, &self.core_uuid, goal_id)
    }

    /// Goals owned by this Core, most recently updated first.
    pub fn list_goals(&self, limit: usize) -> Result<Vec<Goal>, SdkError> {
        let Some(limit) = limit_value(limit, MAX_AUTONOMY_LIST)? else {
            return Ok(Vec::new());
        };
        let mut statement = self.connection.prepare(
            "SELECT payload_json FROM goals
             WHERE core_uuid = ?1
             ORDER BY updated_at DESC, goal_id ASC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![self.core_uuid, limit], |row| {
            row.get::<_, String>(0)
        })?;
        let mut goals = Vec::new();
        for row in rows {
            goals.push(decode_goal(&row?, &self.core_uuid)?);
        }
        Ok(goals)
    }

    /// Replace an existing goal. The revision advances automatically.
    pub fn update_goal(&self, goal: &Goal) -> Result<Goal, SdkError> {
        goal.validate()
            .map_err(|_| SdkError::InvalidMemory("goal failed validation"))?;
        let transaction = self.connection.unchecked_transaction()?;
        let existing =
            read_goal(&transaction, &self.core_uuid, &goal.goal_id)?.ok_or(SdkError::NotFound)?;
        let mut goal = goal.clone();
        goal.core_uuid = self.core_uuid.clone();
        goal.created_at = existing.created_at;
        goal.updated_at = now_rfc3339()?;
        goal.revision = existing.revision.saturating_add(1);
        let payload_json = serde_json::to_string(&goal)?;
        let revision = revision_value(goal.revision)?;
        let changed = transaction.execute(
            "UPDATE goals
             SET status = ?3, payload_json = ?4, updated_at = ?5, revision = ?6
             WHERE core_uuid = ?1 AND goal_id = ?2",
            params![
                self.core_uuid,
                goal.goal_id,
                goal_status_str(goal.status),
                payload_json,
                goal.updated_at,
                revision
            ],
        )?;
        if changed != 1 {
            return Err(SdkError::CorruptData("goal row is missing"));
        }
        insert_event(
            &transaction,
            &self.core_uuid,
            "goal",
            &goal.goal_id,
            "updated",
            &json!({
                "goal_id": goal.goal_id,
                "status": goal_status_str(goal.status),
                "revision": goal.revision,
            }),
            &goal.updated_at,
        )?;
        transaction.commit()?;
        Ok(goal)
    }

    /// Create a plan owned by this Core.
    pub fn create_plan(&self, plan: &Plan) -> Result<Plan, SdkError> {
        plan.validate()
            .map_err(|_| SdkError::InvalidMemory("plan failed validation"))?;
        let mut plan = plan.clone();
        plan.core_uuid = self.bound_core(&plan.core_uuid)?;
        let transaction = self.connection.unchecked_transaction()?;
        let existing: Option<i64> = transaction
            .query_row(
                "SELECT 1 FROM plans WHERE plan_id = ?1",
                params![plan.plan_id],
                |row| row.get(0),
            )
            .optional()?;
        if existing.is_some() {
            return Err(SdkError::InvalidMemory("plan already exists"));
        }
        let payload_json = serde_json::to_string(&plan)?;
        let revision = revision_value(plan.revision)?;
        transaction.execute(
            "INSERT INTO plans(
                plan_id, core_uuid, goal_id, status, payload_json,
                created_at, updated_at, revision
             ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                plan.plan_id,
                self.core_uuid,
                plan.goal_id,
                plan.status.as_str(),
                payload_json,
                plan.created_at,
                plan.updated_at,
                revision
            ],
        )?;
        insert_event(
            &transaction,
            &self.core_uuid,
            "plan",
            &plan.plan_id,
            "created",
            &json!({
                "plan_id": plan.plan_id,
                "goal_id": plan.goal_id,
                "status": plan.status.as_str(),
            }),
            &now_rfc3339()?,
        )?;
        transaction.commit()?;
        Ok(plan)
    }

    /// Fetch one plan owned by this Core.
    pub fn get_plan(&self, plan_id: &str) -> Result<Option<Plan>, SdkError> {
        if !valid_id(plan_id) {
            return Ok(None);
        }
        read_plan(&self.connection, &self.core_uuid, plan_id)
    }

    /// Plans owned by this Core, most recently updated first.
    pub fn list_plans(&self, limit: usize) -> Result<Vec<Plan>, SdkError> {
        let Some(limit) = limit_value(limit, MAX_AUTONOMY_LIST)? else {
            return Ok(Vec::new());
        };
        let mut statement = self.connection.prepare(
            "SELECT payload_json FROM plans
             WHERE core_uuid = ?1
             ORDER BY updated_at DESC, plan_id ASC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![self.core_uuid, limit], |row| {
            row.get::<_, String>(0)
        })?;
        let mut plans = Vec::new();
        for row in rows {
            plans.push(decode_plan(&row?, &self.core_uuid)?);
        }
        Ok(plans)
    }

    /// Create a task owned by this Core.
    pub fn create_task(&self, task: &Task) -> Result<Task, SdkError> {
        task.validate()
            .map_err(|_| SdkError::InvalidMemory("task failed validation"))?;
        let mut task = task.clone();
        task.core_uuid = self.bound_core(&task.core_uuid)?;
        let transaction = self.connection.unchecked_transaction()?;
        let existing: Option<i64> = transaction
            .query_row(
                "SELECT 1 FROM tasks WHERE task_id = ?1",
                params![task.task_id],
                |row| row.get(0),
            )
            .optional()?;
        if existing.is_some() {
            return Err(SdkError::InvalidMemory("task already exists"));
        }
        write_task_row(&transaction, &self.core_uuid, &task, true)?;
        insert_event(
            &transaction,
            &self.core_uuid,
            "task",
            &task.task_id,
            "created",
            &json!({
                "task_id": task.task_id,
                "plan_id": task.plan_id,
                "activity_kind": task.activity_kind,
                "status": task.status.as_str(),
                "due_at": task.due_at,
            }),
            &now_rfc3339()?,
        )?;
        transaction.commit()?;
        Ok(task)
    }

    /// Fetch one task owned by this Core.
    pub fn get_task(&self, task_id: &str) -> Result<Option<Task>, SdkError> {
        if !valid_id(task_id) {
            return Ok(None);
        }
        read_task(&self.connection, &self.core_uuid, task_id)
    }

    /// Tasks owned by this Core, most recently updated first.
    pub fn list_tasks(&self, limit: usize) -> Result<Vec<Task>, SdkError> {
        let Some(limit) = limit_value(limit, MAX_AUTONOMY_LIST)? else {
            return Ok(Vec::new());
        };
        let mut statement = self.connection.prepare(
            "SELECT payload_json FROM tasks
             WHERE core_uuid = ?1
             ORDER BY updated_at DESC, task_id ASC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![self.core_uuid, limit], |row| {
            row.get::<_, String>(0)
        })?;
        let mut tasks = Vec::new();
        for row in rows {
            tasks.push(decode_task(&row?, &self.core_uuid)?);
        }
        Ok(tasks)
    }

    /// Replace an existing task. The revision advances automatically.
    pub fn update_task(&self, task: &Task) -> Result<Task, SdkError> {
        task.validate()
            .map_err(|_| SdkError::InvalidMemory("task failed validation"))?;
        self.apply_task_update(task, "updated", Value::Null)
    }

    /// Due pending tasks whose `due_at` is at or before `now`, most urgent
    /// first. Tasks without a `due_at` are not scheduled and never returned.
    pub fn tick(&self, now: &str) -> Result<Vec<Task>, SdkError> {
        let mut statement = self.connection.prepare(
            "SELECT payload_json FROM tasks
             WHERE core_uuid = ?1 AND status = 'pending'
               AND due_at IS NOT NULL AND due_at <= ?2
             ORDER BY due_at ASC, priority DESC, task_id ASC
             LIMIT ?3",
        )?;
        let limit = limit_value(MAX_AUTONOMY_LIST, MAX_AUTONOMY_LIST)?.unwrap_or(0);
        let rows = statement.query_map(params![self.core_uuid, now, limit], |row| {
            row.get::<_, String>(0)
        })?;
        let mut tasks = Vec::new();
        for row in rows {
            tasks.push(decode_task(&row?, &self.core_uuid)?);
        }
        Ok(tasks)
    }

    /// Move a pending or paused task to `active`.
    pub fn start_task(&self, task_id: &str) -> Result<Task, SdkError> {
        let task = self.require_task(task_id)?;
        match task.status {
            TaskStatus::Active => return Ok(task),
            TaskStatus::Pending | TaskStatus::Paused => {}
            _ => return Err(SdkError::InvalidMemory("task cannot be started")),
        }
        self.transition(task, TaskStatus::Active, "started", Value::Null)
    }

    /// Pause an active task, preserving its persisted progress.
    pub fn pause_task(&self, task_id: &str) -> Result<Task, SdkError> {
        let task = self.require_task(task_id)?;
        match task.status {
            TaskStatus::Paused => return Ok(task),
            TaskStatus::Active | TaskStatus::Pending => {}
            _ => return Err(SdkError::InvalidMemory("task cannot be paused")),
        }
        self.transition(task, TaskStatus::Paused, "paused", Value::Null)
    }

    /// Resume a paused task, preserving its persisted progress.
    pub fn resume_task(&self, task_id: &str) -> Result<Task, SdkError> {
        let task = self.require_task(task_id)?;
        match task.status {
            TaskStatus::Active => return Ok(task),
            TaskStatus::Paused => {}
            _ => return Err(SdkError::InvalidMemory("task cannot be resumed")),
        }
        self.transition(task, TaskStatus::Active, "resumed", Value::Null)
    }

    /// Complete a task with the supplied progress percentage (0 ..= 100).
    pub fn complete_task(&self, task_id: &str, progress: u8) -> Result<Task, SdkError> {
        if progress > 100 {
            return Err(SdkError::InvalidMemory("task progress is invalid"));
        }
        let mut task = self.require_task(task_id)?;
        if matches!(
            task.status,
            TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled
        ) {
            return Err(SdkError::InvalidMemory("task is already terminal"));
        }
        task.progress_percent = progress;
        self.transition(
            task,
            TaskStatus::Completed,
            "completed",
            json!({ "progress": progress }),
        )
    }

    /// Fail a non-terminal task with a bounded reason.
    pub fn fail_task(&self, task_id: &str, reason: &str) -> Result<Task, SdkError> {
        if reason.trim().is_empty() || !chars_within(reason, MAX_TAG_CHARS) {
            return Err(SdkError::InvalidMemory("task failure reason is invalid"));
        }
        let task = self.require_task(task_id)?;
        if matches!(
            task.status,
            TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled
        ) {
            return Err(SdkError::InvalidMemory("task is already terminal"));
        }
        self.transition(
            task,
            TaskStatus::Failed,
            "failed",
            json!({ "reason": reason }),
        )
    }

    /// Cancel a non-terminal task.
    pub fn cancel_task(&self, task_id: &str) -> Result<Task, SdkError> {
        let task = self.require_task(task_id)?;
        if matches!(
            task.status,
            TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled
        ) {
            return Err(SdkError::InvalidMemory("task is already terminal"));
        }
        self.transition(task, TaskStatus::Cancelled, "cancelled", Value::Null)
    }

    /// Current autonomy settings for this Core.
    pub fn settings(&self) -> Result<AutonomySettings, SdkError> {
        let payload: String = self.connection.query_row(
            "SELECT payload_json FROM autonomy_settings WHERE singleton = 1",
            [],
            |row| row.get(0),
        )?;
        decode_settings(&payload, &self.core_uuid)
    }

    /// Replace autonomy settings. The revision advances automatically.
    pub fn set_settings(&self, settings: &AutonomySettings) -> Result<AutonomySettings, SdkError> {
        settings
            .validate()
            .map_err(|_| SdkError::InvalidMemory("autonomy settings failed validation"))?;
        let transaction = self.connection.unchecked_transaction()?;
        let existing = self.settings()?;
        let mut settings = settings.clone();
        settings.core_uuid = self.core_uuid.clone();
        settings.updated_at = now_rfc3339()?;
        settings.revision = existing.revision.saturating_add(1);
        let payload_json = serde_json::to_string(&settings)?;
        let revision = revision_value(settings.revision)?;
        let changed = transaction.execute(
            "UPDATE autonomy_settings
             SET enabled = ?1, max_active_tasks = ?2, payload_json = ?3,
                 updated_at = ?4, revision = ?5
             WHERE singleton = 1",
            params![
                i64::from(settings.enabled),
                i64::from(settings.max_active_tasks),
                payload_json,
                settings.updated_at,
                revision
            ],
        )?;
        if changed != 1 {
            return Err(SdkError::CorruptData("autonomy settings row is missing"));
        }
        insert_event(
            &transaction,
            &self.core_uuid,
            "settings",
            "autonomy_settings",
            "updated",
            &json!({
                "enabled": settings.enabled,
                "max_active_tasks": settings.max_active_tasks,
                "revision": settings.revision,
            }),
            &settings.updated_at,
        )?;
        transaction.commit()?;
        Ok(settings)
    }

    /// Alias for [`Self::list_events`].
    pub fn events(&self, limit: usize) -> Result<Vec<AutonomyEvent>, SdkError> {
        self.list_events(limit)
    }

    /// Autonomy events, newest first, capped at `limit`.
    pub fn list_events(&self, limit: usize) -> Result<Vec<AutonomyEvent>, SdkError> {
        let Some(limit) = limit_value(limit, MAX_AUTONOMY_EVENT_LIST)? else {
            return Ok(Vec::new());
        };
        let mut statement = self.connection.prepare(
            "SELECT sequence, entity_kind, entity_id, action, detail_json, created_at
             FROM autonomy_events
             WHERE core_uuid = ?1
             ORDER BY sequence DESC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![self.core_uuid, limit], |row| {
            read_event_row(row, &self.core_uuid)
        })?;
        let mut events = Vec::new();
        for row in rows {
            events.push(row??);
        }
        Ok(events)
    }

    fn ensure_default_settings(&self) -> Result<(), SdkError> {
        let existing: Option<i64> = self
            .connection
            .query_row(
                "SELECT 1 FROM autonomy_settings WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if existing.is_some() {
            return Ok(());
        }
        let now = now_rfc3339()?;
        let settings = AutonomySettings {
            schema: LIVING_CONTRACT_VERSION.to_owned(),
            core_uuid: self.core_uuid.clone(),
            enabled: true,
            max_active_tasks: DEFAULT_MAX_ACTIVE_TASKS,
            updated_at: now.clone(),
            revision: 1,
        };
        settings
            .validate()
            .map_err(|_| SdkError::CorruptData("default autonomy settings are invalid"))?;
        let payload_json = serde_json::to_string(&settings)?;
        self.connection.execute(
            "INSERT INTO autonomy_settings(
                singleton, core_uuid, enabled, max_active_tasks, payload_json,
                updated_at, revision
             ) VALUES(1, ?1, 1, ?2, ?3, ?4, 1)",
            params![
                self.core_uuid,
                i64::from(settings.max_active_tasks),
                payload_json,
                now
            ],
        )?;
        Ok(())
    }

    fn bound_core(&self, candidate: &str) -> Result<String, SdkError> {
        let candidate = canonical_core_id(candidate)?;
        if candidate != self.core_uuid {
            return Err(SdkError::OwnershipMismatch);
        }
        Ok(candidate)
    }

    fn require_task(&self, task_id: &str) -> Result<Task, SdkError> {
        if !valid_id(task_id) {
            return Err(SdkError::NotFound);
        }
        self.get_task(task_id)?.ok_or(SdkError::NotFound)
    }

    fn transition(
        &self,
        task: Task,
        status: TaskStatus,
        action: &str,
        detail: Value,
    ) -> Result<Task, SdkError> {
        let mut task = task;
        task.status = status;
        self.apply_task_update(&task, action, detail)
    }

    fn apply_task_update(
        &self,
        task: &Task,
        action: &str,
        detail: Value,
    ) -> Result<Task, SdkError> {
        let transaction = self.connection.unchecked_transaction()?;
        let existing =
            read_task(&transaction, &self.core_uuid, &task.task_id)?.ok_or(SdkError::NotFound)?;
        let mut task = task.clone();
        task.core_uuid = self.core_uuid.clone();
        task.created_at = existing.created_at;
        task.updated_at = now_rfc3339()?;
        task.revision = existing.revision.saturating_add(1);
        task.validate()
            .map_err(|_| SdkError::InvalidMemory("task failed validation"))?;
        write_task_row(&transaction, &self.core_uuid, &task, false)?;
        let mut event_detail = detail;
        if event_detail.is_null() {
            event_detail = json!({});
        }
        if let Value::Object(ref mut map) = event_detail {
            map.insert(
                "status".to_owned(),
                Value::String(task.status.as_str().to_owned()),
            );
            map.insert("progress".to_owned(), Value::from(task.progress_percent));
            map.insert("revision".to_owned(), Value::from(task.revision));
        }
        insert_event(
            &transaction,
            &self.core_uuid,
            "task",
            &task.task_id,
            action,
            &event_detail,
            &task.updated_at,
        )?;
        transaction.commit()?;
        Ok(task)
    }
}

fn goal_status_str(status: GoalStatus) -> &'static str {
    match status {
        GoalStatus::Proposed => "proposed",
        GoalStatus::Active => "active",
        GoalStatus::Paused => "paused",
        GoalStatus::Completed => "completed",
        GoalStatus::Abandoned => "abandoned",
    }
}

fn write_task_row(
    connection: &Connection,
    core_uuid: &str,
    task: &Task,
    insert: bool,
) -> Result<(), SdkError> {
    let payload_json = serde_json::to_string(task)?;
    let revision = revision_value(task.revision)?;
    let priority = i64::from(task.priority);
    let progress = i64::from(task.progress_percent);
    if insert {
        connection.execute(
            "INSERT INTO tasks(
                task_id, core_uuid, plan_id, goal_id, status, priority, due_at,
                progress_percent, payload_json, created_at, updated_at, revision
             ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                task.task_id,
                core_uuid,
                task.plan_id,
                task.goal_id,
                task.status.as_str(),
                priority,
                task.due_at,
                progress,
                payload_json,
                task.created_at,
                task.updated_at,
                revision
            ],
        )?;
    } else {
        let changed = connection.execute(
            "UPDATE tasks
             SET plan_id = ?3, goal_id = ?4, status = ?5, priority = ?6, due_at = ?7,
                 progress_percent = ?8, payload_json = ?9, updated_at = ?10, revision = ?11
             WHERE core_uuid = ?1 AND task_id = ?2",
            params![
                core_uuid,
                task.task_id,
                task.plan_id,
                task.goal_id,
                task.status.as_str(),
                priority,
                task.due_at,
                progress,
                payload_json,
                task.updated_at,
                revision
            ],
        )?;
        if changed != 1 {
            return Err(SdkError::CorruptData("task row is missing"));
        }
    }
    Ok(())
}

fn initialize_schema(connection: &Connection) -> Result<(), SdkError> {
    connection.execute_batch(SCHEMA_SQL)?;
    Ok(())
}

fn ensure_schema_marker(connection: &Connection) -> Result<(), SdkError> {
    let existing: Option<(String, i64)> = connection
        .query_row(
            "SELECT schema, revision FROM autonomy_schema WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match existing {
        None => {
            let revision = revision_value(AUTONOMY_SCHEMA_REVISION)?;
            connection.execute(
                "INSERT INTO autonomy_schema(singleton, schema, revision, created_at)
                 VALUES(1, ?1, ?2, ?3)",
                params![AUTONOMY_SCHEMA, revision, now_rfc3339()?],
            )?;
        }
        Some((schema, revision)) => {
            let revision = u64::try_from(revision)
                .map_err(|_| SdkError::CorruptData("autonomy schema revision is invalid"))?;
            if schema != AUTONOMY_SCHEMA || revision > AUTONOMY_SCHEMA_REVISION {
                return Err(SdkError::CorruptData("autonomy schema is unsupported"));
            }
        }
    }
    Ok(())
}

fn table_exists(connection: &Connection, name: &str) -> Result<bool, SdkError> {
    let found: Option<String> = connection
        .query_row(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [name],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

fn verify_ownership(
    connection: &Connection,
    expected_core: &str,
    expected_instance: &str,
) -> Result<(), SdkError> {
    let count: i64 =
        connection.query_row("SELECT COUNT(*) FROM autonomy_ownership", [], |row| {
            row.get(0)
        })?;
    if count != 1 {
        return Err(SdkError::OwnershipMissing);
    }
    let row: Option<(String, String, String)> = connection
        .query_row(
            "SELECT core_uuid, database_instance_uuid, schema
             FROM autonomy_ownership WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((stored_core, stored_instance, schema)) = row else {
        return Err(SdkError::OwnershipMissing);
    };
    if schema != AUTONOMY_SCHEMA {
        return Err(SdkError::CorruptData("autonomy schema is unsupported"));
    }
    let stored_core = canonical_core_id(&stored_core).map_err(|_| SdkError::OwnershipMismatch)?;
    if stored_core != expected_core {
        return Err(SdkError::OwnershipMismatch);
    }
    let stored_instance = validate_core_id(&stored_instance)
        .map_err(|_| SdkError::OwnershipMismatch)?
        .hyphenated()
        .to_string();
    if stored_instance != expected_instance {
        return Err(SdkError::OwnershipMismatch);
    }
    Ok(())
}

fn read_goal(
    connection: &Connection,
    expected_core: &str,
    goal_id: &str,
) -> Result<Option<Goal>, SdkError> {
    let payload: Option<String> = connection
        .query_row(
            "SELECT payload_json FROM goals WHERE core_uuid = ?1 AND goal_id = ?2",
            params![expected_core, goal_id],
            |row| row.get(0),
        )
        .optional()?;
    payload
        .map(|payload| decode_goal(&payload, expected_core))
        .transpose()
}

fn read_plan(
    connection: &Connection,
    expected_core: &str,
    plan_id: &str,
) -> Result<Option<Plan>, SdkError> {
    let payload: Option<String> = connection
        .query_row(
            "SELECT payload_json FROM plans WHERE core_uuid = ?1 AND plan_id = ?2",
            params![expected_core, plan_id],
            |row| row.get(0),
        )
        .optional()?;
    payload
        .map(|payload| decode_plan(&payload, expected_core))
        .transpose()
}

fn read_task(
    connection: &Connection,
    expected_core: &str,
    task_id: &str,
) -> Result<Option<Task>, SdkError> {
    let payload: Option<String> = connection
        .query_row(
            "SELECT payload_json FROM tasks WHERE core_uuid = ?1 AND task_id = ?2",
            params![expected_core, task_id],
            |row| row.get(0),
        )
        .optional()?;
    payload
        .map(|payload| decode_task(&payload, expected_core))
        .transpose()
}

fn decode_goal(payload: &str, expected_core: &str) -> Result<Goal, SdkError> {
    let goal: Goal = serde_json::from_str(payload)
        .map_err(|_| SdkError::CorruptData("stored goal is invalid"))?;
    let stored_core =
        canonical_core_id(&goal.core_uuid).map_err(|_| SdkError::OwnershipMismatch)?;
    if stored_core != expected_core {
        return Err(SdkError::OwnershipMismatch);
    }
    goal.validate()
        .map_err(|_| SdkError::CorruptData("stored goal is invalid"))?;
    Ok(goal)
}

fn decode_plan(payload: &str, expected_core: &str) -> Result<Plan, SdkError> {
    let plan: Plan = serde_json::from_str(payload)
        .map_err(|_| SdkError::CorruptData("stored plan is invalid"))?;
    let stored_core =
        canonical_core_id(&plan.core_uuid).map_err(|_| SdkError::OwnershipMismatch)?;
    if stored_core != expected_core {
        return Err(SdkError::OwnershipMismatch);
    }
    plan.validate()
        .map_err(|_| SdkError::CorruptData("stored plan is invalid"))?;
    Ok(plan)
}

fn decode_task(payload: &str, expected_core: &str) -> Result<Task, SdkError> {
    let task: Task = serde_json::from_str(payload)
        .map_err(|_| SdkError::CorruptData("stored task is invalid"))?;
    let stored_core =
        canonical_core_id(&task.core_uuid).map_err(|_| SdkError::OwnershipMismatch)?;
    if stored_core != expected_core {
        return Err(SdkError::OwnershipMismatch);
    }
    task.validate()
        .map_err(|_| SdkError::CorruptData("stored task is invalid"))?;
    Ok(task)
}

fn decode_settings(payload: &str, expected_core: &str) -> Result<AutonomySettings, SdkError> {
    let settings: AutonomySettings = serde_json::from_str(payload)
        .map_err(|_| SdkError::CorruptData("stored autonomy settings are invalid"))?;
    let stored_core =
        canonical_core_id(&settings.core_uuid).map_err(|_| SdkError::OwnershipMismatch)?;
    if stored_core != expected_core {
        return Err(SdkError::OwnershipMismatch);
    }
    settings
        .validate()
        .map_err(|_| SdkError::CorruptData("stored autonomy settings are invalid"))?;
    Ok(settings)
}

fn read_event_row(
    row: &rusqlite::Row<'_>,
    expected_core: &str,
) -> Result<Result<AutonomyEvent, SdkError>, rusqlite::Error> {
    let sequence: i64 = row.get(0)?;
    let entity_kind: String = row.get(1)?;
    let entity_id: String = row.get(2)?;
    let action: String = row.get(3)?;
    let detail_json: String = row.get(4)?;
    let created_at: String = row.get(5)?;
    Ok((|| {
        let detail: Value = serde_json::from_str(&detail_json)
            .map_err(|_| SdkError::CorruptData("stored autonomy event detail is invalid"))?;
        Ok(AutonomyEvent {
            schema: AUTONOMY_EVENT_SCHEMA.to_owned(),
            sequence: u64::try_from(sequence)
                .map_err(|_| SdkError::CorruptData("stored autonomy event sequence is invalid"))?,
            core_uuid: expected_core.to_owned(),
            entity_kind,
            entity_id,
            action,
            detail,
            created_at,
        })
    })())
}

fn insert_event(
    connection: &Connection,
    core_uuid: &str,
    entity_kind: &str,
    entity_id: &str,
    action: &str,
    detail: &Value,
    created_at: &str,
) -> Result<u64, SdkError> {
    let detail_json = serde_json::to_string(detail)?;
    if detail_json.len() > MAX_AUTONOMY_EVENT_DETAIL_BYTES {
        return Err(SdkError::InvalidMemory(
            "autonomy event detail is too large",
        ));
    }
    connection.execute(
        "INSERT INTO autonomy_events(
            core_uuid, entity_kind, entity_id, action, detail_json, created_at
         ) VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            core_uuid,
            entity_kind,
            entity_id,
            action,
            detail_json,
            created_at
        ],
    )?;
    let sequence = connection.last_insert_rowid();
    u64::try_from(sequence).map_err(|_| SdkError::CorruptData("autonomy event sequence is invalid"))
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && chars_within(value, MAX_ID_CHARS)
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | ':')
        })
}

fn revision_value(revision: u64) -> Result<i64, SdkError> {
    i64::try_from(revision).map_err(|_| SdkError::CorruptData("autonomy revision is out of range"))
}

fn limit_value(limit: usize, cap: usize) -> Result<Option<i64>, SdkError> {
    let limit = limit.min(cap);
    if limit == 0 {
        return Ok(None);
    }
    i64::try_from(limit)
        .map(Some)
        .map_err(|_| SdkError::InvalidMemory("limit is out of range"))
}

//! Owner-authorized tool registry and execution bridge.
//!
//! The model only *proposes* a tool call. This crate decides whether the call
//! is known, enabled, unblocked by STOP, and (for sensitive tools) explicitly
//! approved by the owner. It executes only the built-in, bounded tools listed
//! here and never runs shell commands or arbitrary code from model output.
//!
//! Every attempt is appended to the Safe audit chain. Argument *values* are
//! never written to the audit record.

use std::path::{Path, PathBuf};
use std::time::Duration;

use aegis_core_install::CoreInstaller;
use aegis_core_sdk::{CoreMemoryStore, MemoryKind, MemoryQuery, NewMemory};
use aegis_core_storage::{CoreDatabase, CoreStorageLayout};
use aegis_modules::{
    run_module_tool, ModuleManager, ModuleRegistry, ModuleToolBinding, MAX_TOOL_INPUT_BYTES,
};
use aegis_platform::PlatformService;
use aegis_safe::{SafeError, SafeStore};
use aegis_storage::{
    now_rfc3339, open_initialized, StorageError, StoragePaths, PLATFORM_APPLICATION_ID,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;

/// Schema identifier for a public tool grant.
pub const TOOL_GRANT_SCHEMA: &str = "aegis.tool.grant.v1";
/// Schema identifier for an execution result.
pub const TOOL_RESULT_SCHEMA: &str = "aegis.tool.result.v1";

const TOOL_COMPONENT: &str = "tools";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSensitivity {
    ReadOnly,
    Sensitive,
}

impl ToolSensitivity {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::Sensitive => "sensitive",
        }
    }
}

/// Static definition of one built-in tool.
#[derive(Debug, Clone, Copy)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub permission: &'static str,
    pub sensitivity: ToolSensitivity,
    pub approval_required: bool,
    pub input: &'static str,
}

/// Every tool this bridge can execute.
pub const TOOL_SPECS: [ToolSpec; 6] = [
    ToolSpec {
        name: "memory.search",
        description: "Search this Core's private memories.",
        permission: "memory.read",
        sensitivity: ToolSensitivity::ReadOnly,
        approval_required: false,
        input: "{ text?: string, kind?: string, limit?: number }",
    },
    ToolSpec {
        name: "memory.create",
        description: "Store a new memory in this Core's private memory store.",
        permission: "memory.write",
        sensitivity: ToolSensitivity::Sensitive,
        approval_required: true,
        input: "{ content: string, kind?: string, tags?: string[] }",
    },
    ToolSpec {
        name: "platform.status",
        description: "Return the Platform identity and zero/multi-Core status.",
        permission: "platform.read",
        sensitivity: ToolSensitivity::ReadOnly,
        approval_required: false,
        input: "{}",
    },
    ToolSpec {
        name: "module.list",
        description: "List installed modules and their lifecycle state.",
        permission: "module.read",
        sensitivity: ToolSensitivity::ReadOnly,
        approval_required: false,
        input: "{}",
    },
    ToolSpec {
        name: "core.inspect",
        description: "Inspect public identity/metadata of a registered Core.",
        permission: "core.read",
        sensitivity: ToolSensitivity::ReadOnly,
        approval_required: false,
        input: "{ core_uuid: string }",
    },
    ToolSpec {
        name: "time.now",
        description: "Return the current UTC time.",
        permission: "time.read",
        sensitivity: ToolSensitivity::ReadOnly,
        approval_required: false,
        input: "{}",
    },
];

#[derive(Debug, Clone)]
struct ResolvedTool {
    name: String,
    description: String,
    permission: String,
    sensitivity: ToolSensitivity,
    approval_required: bool,
    input: String,
    module: Option<ModuleToolBinding>,
    timeout_ms: u64,
}

impl ResolvedTool {
    fn from_spec(spec: &'static ToolSpec) -> Self {
        Self {
            name: spec.name.to_owned(),
            description: spec.description.to_owned(),
            permission: spec.permission.to_owned(),
            sensitivity: spec.sensitivity,
            approval_required: spec.approval_required,
            input: spec.input.to_owned(),
            module: None,
            timeout_ms: 30_000,
        }
    }

    fn from_module(binding: ModuleToolBinding) -> Self {
        Self {
            name: binding.effective_name.clone(),
            description: binding.description.clone(),
            permission: binding.permission.clone(),
            sensitivity: if binding.sensitive {
                ToolSensitivity::Sensitive
            } else {
                ToolSensitivity::ReadOnly
            },
            approval_required: binding.approval_required || binding.sensitive,
            input: binding.input_schema.to_string(),
            timeout_ms: binding.timeout_ms,
            module: Some(binding),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolGrant {
    pub schema: String,
    pub name: String,
    pub description: String,
    pub permission: String,
    pub sensitivity: ToolSensitivity,
    pub granted: bool,
    pub approval_required: bool,
    pub input: String,
    pub module_id: Option<String>,
    pub available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    Executed,
    Denied,
    ApprovalRequired,
    Failed,
}

impl ToolStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Executed => "executed",
            Self::Denied => "denied",
            Self::ApprovalRequired => "approval_required",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    pub schema: String,
    pub tool: String,
    pub status: ToolStatus,
    pub sensitivity: ToolSensitivity,
    pub output: Value,
    pub detail: Option<String>,
    pub audited: bool,
    pub module_id: Option<String>,
}

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("tool storage failed: {0}")]
    Storage(#[from] StorageError),
    #[error("tool SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Safe operation failed: {0}")]
    Safe(#[from] SafeError),
    #[error("Core SDK failed: {0}")]
    Sdk(#[from] aegis_core_sdk::SdkError),
    #[error("Core storage failed: {0}")]
    CoreStorage(#[from] aegis_core_storage::CoreStorageError),
    #[error("Core installation failed: {0}")]
    Install(#[from] aegis_core_install::InstallError),
    #[error("Platform operation failed: {0}")]
    Platform(#[from] aegis_platform::ServiceError),
    #[error("module operation failed: {0}")]
    Module(#[from] aegis_modules::ModuleError),
    #[error("tool JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Platform is not initialized at {0}")]
    NotInitialized(String),
    #[error("Core UUID is invalid")]
    InvalidCoreUuid,
    #[error("invalid tool input: {0}")]
    InvalidInput(&'static str),
}

/// Platform-owned tool permission registry.
#[derive(Debug, Clone)]
pub struct ToolRegistry {
    paths: StoragePaths,
    safe: SafeStore,
}

impl ToolRegistry {
    pub fn new(paths: StoragePaths) -> Self {
        let safe = SafeStore::new(paths.clone());
        Self { paths, safe }
    }

    pub fn initialize(&self) -> Result<(), ToolError> {
        let connection = self.open()?;
        for spec in TOOL_SPECS {
            let default_granted = spec.name != "memory.create";
            connection.execute(
                "INSERT OR IGNORE INTO aegis_tool_permissions(
                    tool_name, granted, approval_required, updated_at
                 ) VALUES(?1, ?2, ?3, ?4)",
                params![
                    spec.name,
                    i64::from(u8::from(default_granted)),
                    i64::from(u8::from(spec.approval_required)),
                    now_rfc3339()?,
                ],
            )?;
        }
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<ToolGrant>, ToolError> {
        self.initialize()?;
        let connection = self.open()?;
        let mut grants = Vec::with_capacity(TOOL_SPECS.len());
        for spec in TOOL_SPECS {
            let (granted, approval_required) = read_grant(&connection, spec.name)?;
            grants.push(ToolGrant {
                schema: TOOL_GRANT_SCHEMA.to_owned(),
                name: spec.name.to_owned(),
                description: spec.description.to_owned(),
                permission: spec.permission.to_owned(),
                sensitivity: spec.sensitivity,
                granted,
                approval_required,
                input: spec.input.to_owned(),
                module_id: None,
                available: true,
            });
        }
        let registry = ModuleRegistry::new(self.paths.clone());
        for binding in registry.tool_bindings()? {
            let (granted, approval_required) = read_grant(&connection, &binding.effective_name)?;
            grants.push(ToolGrant {
                schema: TOOL_GRANT_SCHEMA.to_owned(),
                name: binding.effective_name.clone(),
                description: binding.description.clone(),
                permission: binding.permission.clone(),
                sensitivity: if binding.sensitive {
                    ToolSensitivity::Sensitive
                } else {
                    ToolSensitivity::ReadOnly
                },
                granted,
                approval_required: approval_required || binding.approval_required,
                input: binding.input_schema.to_string(),
                module_id: Some(binding.module_id.clone()),
                available: binding.enabled,
            });
        }
        Ok(grants)
    }

    /// Change a tool grant. Requires owner proof.
    pub fn set_permission(
        &self,
        tool: &str,
        granted: bool,
        approval_required: bool,
        owner_token: &str,
    ) -> Result<ToolGrant, ToolError> {
        if !self.safe.verify_owner(owner_token)? {
            return Err(ToolError::Safe(SafeError::Unauthorized));
        }
        let resolved =
            resolve_tool(&self.paths, tool)?.ok_or(ToolError::InvalidInput("unknown tool"))?;
        let approval_required = approval_required || resolved.approval_required;
        self.initialize()?;
        let connection = self.open()?;
        connection.execute(
            "INSERT INTO aegis_tool_permissions(
                tool_name, granted, approval_required, updated_at
             ) VALUES(?1, ?2, ?3, ?4)
             ON CONFLICT(tool_name) DO UPDATE SET
                 granted = excluded.granted,
                 approval_required = excluded.approval_required,
                 updated_at = excluded.updated_at",
            params![
                tool,
                i64::from(u8::from(granted)),
                i64::from(u8::from(approval_required)),
                now_rfc3339()?,
            ],
        )?;
        self.safe.record_event(
            "tool.permission.changed",
            json!({"tool": tool, "granted": granted, "approval_required": approval_required}),
        )?;
        let (granted, approval_required) = read_grant(&connection, tool)?;
        Ok(ToolGrant {
            schema: TOOL_GRANT_SCHEMA.to_owned(),
            name: resolved.name.clone(),
            description: resolved.description.clone(),
            permission: resolved.permission.clone(),
            sensitivity: resolved.sensitivity,
            granted,
            approval_required,
            input: resolved.input.clone(),
            module_id: resolved
                .module
                .as_ref()
                .map(|binding| binding.module_id.clone()),
            available: resolved
                .module
                .as_ref()
                .map(|binding| binding.enabled)
                .unwrap_or(true),
        })
    }

    fn open(&self) -> Result<Connection, ToolError> {
        if !self.paths.platform_db().is_file() {
            return Err(ToolError::NotInitialized(
                self.paths.root().display().to_string(),
            ));
        }
        let connection = open_initialized(
            &self.paths.platform_db(),
            PLATFORM_APPLICATION_ID,
            TOOL_COMPONENT,
        )?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS aegis_tool_permissions (
                tool_name TEXT PRIMARY KEY,
                granted INTEGER NOT NULL CHECK(granted IN (0, 1)),
                approval_required INTEGER NOT NULL CHECK(approval_required IN (0, 1)),
                updated_at TEXT NOT NULL
            );",
        )?;
        Ok(connection)
    }
}

/// Executor bound to one calling Core and storage root.
#[derive(Debug, Clone)]
pub struct ToolExecutor {
    paths: StoragePaths,
    caller_core: String,
    registry: ToolRegistry,
    safe: SafeStore,
}

impl ToolExecutor {
    pub fn new(root: impl AsRef<Path>, caller_core_uuid: &str) -> Result<Self, ToolError> {
        let paths = StoragePaths::new(root.as_ref())?;
        if !paths.platform_db().is_file() {
            return Err(ToolError::NotInitialized(
                paths.root().display().to_string(),
            ));
        }
        let caller_core = aegis_contracts::core::canonicalize_core_id(caller_core_uuid)
            .map_err(|_| ToolError::InvalidCoreUuid)?;
        let layout = CoreStorageLayout::new(paths.root())?;
        let snapshot = CoreDatabase::read_snapshot(&layout, &caller_core)?;
        if snapshot.identity.core_uuid != caller_core {
            return Err(ToolError::InvalidCoreUuid);
        }
        Ok(Self {
            registry: ToolRegistry::new(paths.clone()),
            safe: SafeStore::new(paths.clone()),
            paths,
            caller_core,
        })
    }

    pub fn caller_core(&self) -> &str {
        &self.caller_core
    }

    /// Execute a proposed tool call after permission, STOP, and approval checks.
    pub fn execute(
        &self,
        tool: &str,
        arguments: &Value,
        owner_token: Option<&str>,
    ) -> Result<ToolResult, ToolError> {
        let Some(resolved) = resolve_tool(&self.paths, tool)? else {
            self.audit(tool, "denied", json!({"reason": "unknown_tool"}))?;
            return Ok(self.result(
                tool,
                ToolStatus::Denied,
                ToolSensitivity::ReadOnly,
                Value::Null,
                Some("unknown tool".to_owned()),
                None,
            ));
        };

        self.registry.initialize()?;
        let connection = self.registry.open()?;
        let (granted, approval_required) = read_grant(&connection, &resolved.name)?;
        if !granted {
            self.audit(tool, "denied", json!({"reason": "not_granted"}))?;
            return Ok(self.result(
                tool,
                ToolStatus::Denied,
                resolved.sensitivity,
                Value::Null,
                Some("tool is not granted by the owner".to_owned()),
                resolved
                    .module
                    .as_ref()
                    .map(|binding| binding.module_id.clone()),
            ));
        }

        if resolved
            .module
            .as_ref()
            .is_some_and(|binding| !binding.enabled)
        {
            self.audit(tool, "denied", json!({"reason": "module_disabled"}))?;
            return Ok(self.result(
                tool,
                ToolStatus::Denied,
                resolved.sensitivity,
                Value::Null,
                Some("module is not enabled".to_owned()),
                resolved
                    .module
                    .as_ref()
                    .map(|binding| binding.module_id.clone()),
            ));
        }

        if self.safe.stop_status()?.active {
            self.audit(tool, "denied", json!({"reason": "stop_active"}))?;
            return Ok(self.result(
                tool,
                ToolStatus::Denied,
                resolved.sensitivity,
                Value::Null,
                Some("operator STOP is active".to_owned()),
                resolved
                    .module
                    .as_ref()
                    .map(|binding| binding.module_id.clone()),
            ));
        }

        let approval_needed = approval_required
            || resolved.approval_required
            || resolved.sensitivity == ToolSensitivity::Sensitive;
        let mut approved = !approval_needed;
        if approval_needed {
            if let Some(token) = owner_token {
                approved = self.safe.verify_owner(token)?;
            }
            if !approved {
                self.audit(
                    tool,
                    "approval_required",
                    json!({"reason": "owner_approval"}),
                )?;
                return Ok(self.result(
                    tool,
                    ToolStatus::ApprovalRequired,
                    resolved.sensitivity,
                    Value::Null,
                    Some("owner approval is required for this tool".to_owned()),
                    resolved
                        .module
                        .as_ref()
                        .map(|binding| binding.module_id.clone()),
                ));
            }
        }

        let object = match arguments {
            Value::Null => serde_json::Map::new(),
            Value::Object(map) => map.clone(),
            _ => {
                self.audit(tool, "failed", json!({"reason": "arguments_not_object"}))?;
                return Ok(self.result(
                    tool,
                    ToolStatus::Failed,
                    resolved.sensitivity,
                    Value::Null,
                    Some("tool arguments must be a JSON object".to_owned()),
                    resolved
                        .module
                        .as_ref()
                        .map(|binding| binding.module_id.clone()),
                ));
            }
        };

        if let Some(binding) = &resolved.module {
            if let Err(detail) = validate_module_arguments(binding, &object) {
                self.audit(tool, "failed", json!({"reason": detail}))?;
                return Ok(self.result(
                    tool,
                    ToolStatus::Failed,
                    resolved.sensitivity,
                    Value::Null,
                    Some(detail),
                    Some(binding.module_id.clone()),
                ));
            }
        }

        let executed = if let Some(binding) = &resolved.module {
            run_module_tool(
                self.paths.root(),
                &binding.module_id,
                &binding.tool_name,
                &self.caller_core,
                &Value::Object(object.clone()),
                Duration::from_millis(resolved.timeout_ms),
            )
            .map_err(|error| error.to_string())
        } else {
            self.run(&resolved.name, &object)
                .map_err(|error| error.to_string())
        };

        match executed {
            Ok(output) => {
                self.audit(
                    tool,
                    "executed",
                    json!({
                        "approved": approved,
                        "argument_keys": object.keys().collect::<Vec<_>>(),
                        "module_id": resolved.module.as_ref().map(|binding| binding.module_id.clone()),
                    }),
                )?;
                Ok(self.result(
                    tool,
                    ToolStatus::Executed,
                    resolved.sensitivity,
                    output,
                    None,
                    resolved
                        .module
                        .as_ref()
                        .map(|binding| binding.module_id.clone()),
                ))
            }
            Err(detail) => {
                self.audit(tool, "failed", json!({"reason": detail}))?;
                Ok(self.result(
                    tool,
                    ToolStatus::Failed,
                    resolved.sensitivity,
                    Value::Null,
                    Some(detail),
                    resolved
                        .module
                        .as_ref()
                        .map(|binding| binding.module_id.clone()),
                ))
            }
        }
    }

    fn run(
        &self,
        tool: &str,
        arguments: &serde_json::Map<String, Value>,
    ) -> Result<Value, ToolError> {
        match tool {
            "memory.search" => self.memory_search(arguments),
            "memory.create" => self.memory_create(arguments),
            "platform.status" => {
                let platform = PlatformService::start(self.paths.root())?;
                Ok(serde_json::to_value(platform.status()?)?)
            }
            "module.list" => {
                let modules = ModuleManager::new(self.paths.clone()).list()?;
                Ok(serde_json::to_value(modules)?)
            }
            "core.inspect" => {
                let core_uuid = optional_str(arguments, "core_uuid", 64)?
                    .ok_or(ToolError::InvalidInput("core_uuid"))?;
                let installer = CoreInstaller::new(self.paths.root())?;
                let inspection = installer.inspect(&core_uuid)?;
                Ok(json!({
                    "identity": inspection.identity,
                    "metadata": inspection.metadata,
                    "registry_entry": inspection.registry_entry,
                    "integrity_status": inspection.integrity.status.as_str(),
                }))
            }
            "time.now" => Ok(json!({"now": now_rfc3339()?})),
            _ => Err(ToolError::InvalidInput("unknown tool")),
        }
    }

    fn memory_search(
        &self,
        arguments: &serde_json::Map<String, Value>,
    ) -> Result<Value, ToolError> {
        let mut query = MemoryQuery::new();
        if let Some(text) = optional_str(arguments, "text", 500)? {
            query.text = Some(text);
        }
        if let Some(kind) = optional_str(arguments, "kind", 64)? {
            query.kind =
                Some(MemoryKind::parse(&kind).map_err(|_| ToolError::InvalidInput("kind"))?);
        }
        let limit = optional_u64(arguments, "limit")?.unwrap_or(10).clamp(1, 50);
        query.limit = limit as usize;
        let store = CoreMemoryStore::open(self.paths.root(), &self.caller_core)?;
        let entries = store.search(&query)?;
        let output: Vec<Value> = entries
            .into_iter()
            .map(|entry| {
                json!({
                    "id": entry.id,
                    "kind": entry.kind.as_str(),
                    "content": truncate_chars(&entry.content, 1_000),
                    "source": entry.source,
                    "provenance": entry.provenance,
                    "tags": entry.tags,
                    "created_at": entry.created_at,
                    "archived": entry.archived,
                })
            })
            .collect();
        Ok(Value::Array(output))
    }

    fn memory_create(
        &self,
        arguments: &serde_json::Map<String, Value>,
    ) -> Result<Value, ToolError> {
        let content = optional_str(arguments, "content", 65_536)?
            .ok_or(ToolError::InvalidInput("content"))?;
        let kind = match optional_str(arguments, "kind", 64)? {
            Some(value) => {
                MemoryKind::parse(&value).map_err(|_| ToolError::InvalidInput("kind"))?
            }
            None => MemoryKind::Fact,
        };
        let tags = optional_string_array(arguments, "tags", 16, 64)?;
        let memory = NewMemory::new(kind, content)
            .with_source("tool")
            .with_provenance("model_tool")
            .with_tags(tags);
        let store = CoreMemoryStore::open(self.paths.root(), &self.caller_core)?;
        let entry = store.create(memory)?;
        Ok(serde_json::to_value(entry)?)
    }

    fn result(
        &self,
        tool: &str,
        status: ToolStatus,
        sensitivity: ToolSensitivity,
        output: Value,
        detail: Option<String>,
        module_id: Option<String>,
    ) -> ToolResult {
        ToolResult {
            schema: TOOL_RESULT_SCHEMA.to_owned(),
            tool: tool.to_owned(),
            status,
            sensitivity,
            output,
            detail,
            audited: true,
            module_id,
        }
    }

    fn audit(&self, tool: &str, outcome: &str, detail: Value) -> Result<(), ToolError> {
        let event = match outcome {
            "executed" => "tool.executed",
            "denied" => "tool.denied",
            "approval_required" => "tool.approval_required",
            _ => "tool.failed",
        };
        self.safe.record_event(
            event,
            json!({
                "tool": tool,
                "caller_core": self.caller_core,
                "outcome": outcome,
                "detail": detail,
            }),
        )?;
        Ok(())
    }
}

fn tool_spec(name: &str) -> Option<&'static ToolSpec> {
    TOOL_SPECS.iter().find(|spec| spec.name == name)
}

fn resolve_tool(paths: &StoragePaths, tool: &str) -> Result<Option<ResolvedTool>, ToolError> {
    if let Some(spec) = tool_spec(tool) {
        return Ok(Some(ResolvedTool::from_spec(spec)));
    }
    let registry = ModuleRegistry::new(paths.clone());
    Ok(registry
        .tool_bindings()?
        .into_iter()
        .find(|binding| binding.effective_name == tool)
        .map(ResolvedTool::from_module))
}

fn validate_module_arguments(
    binding: &ModuleToolBinding,
    arguments: &serde_json::Map<String, Value>,
) -> Result<(), String> {
    let encoded =
        serde_json::to_vec(&Value::Object(arguments.clone())).map_err(|error| error.to_string())?;
    if encoded.len() > MAX_TOOL_INPUT_BYTES {
        return Err("tool arguments are too large".to_owned());
    }
    if let Some(required) = binding
        .input_schema
        .get("required")
        .and_then(Value::as_array)
    {
        for key in required {
            let Some(key) = key.as_str() else {
                return Err("module tool schema has a non-string required key".to_owned());
            };
            if !arguments.contains_key(key) {
                return Err(format!("missing required argument: {key}"));
            }
        }
    }
    Ok(())
}

fn read_grant(connection: &Connection, tool: &str) -> Result<(bool, bool), ToolError> {
    let row: Option<(i64, i64)> = connection
        .query_row(
            "SELECT granted, approval_required FROM aegis_tool_permissions WHERE tool_name = ?1",
            [tool],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    Ok(match row {
        Some((granted, approval)) => (granted != 0, approval != 0),
        None => (false, true),
    })
}

fn optional_str(
    arguments: &serde_json::Map<String, Value>,
    key: &'static str,
    maximum: usize,
) -> Result<Option<String>, ToolError> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => {
            let trimmed = value.trim();
            if trimmed.is_empty()
                || value.chars().count() > maximum
                || value.chars().any(char::is_control)
            {
                return Err(ToolError::InvalidInput(key));
            }
            Ok(Some(value.clone()))
        }
        Some(_) => Err(ToolError::InvalidInput(key)),
    }
}

fn optional_u64(
    arguments: &serde_json::Map<String, Value>,
    key: &'static str,
) -> Result<Option<u64>, ToolError> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => number
            .as_u64()
            .map(Some)
            .ok_or(ToolError::InvalidInput(key)),
        Some(_) => Err(ToolError::InvalidInput(key)),
    }
}

fn optional_string_array(
    arguments: &serde_json::Map<String, Value>,
    key: &'static str,
    maximum_items: usize,
    maximum_chars: usize,
) -> Result<Vec<String>, ToolError> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(values)) => {
            if values.len() > maximum_items {
                return Err(ToolError::InvalidInput(key));
            }
            let mut output = Vec::with_capacity(values.len());
            for value in values {
                let Some(text) = value.as_str() else {
                    return Err(ToolError::InvalidInput(key));
                };
                let trimmed = text.trim();
                if trimmed.is_empty()
                    || text.chars().count() > maximum_chars
                    || text.chars().any(char::is_control)
                {
                    return Err(ToolError::InvalidInput(key));
                }
                output.push(text.to_owned());
            }
            Ok(output)
        }
        Some(_) => Err(ToolError::InvalidInput(key)),
    }
}

fn truncate_chars(value: &str, maximum: usize) -> String {
    if value.chars().count() <= maximum {
        return value.to_owned();
    }
    value.chars().take(maximum).collect()
}

/// Resolve a tool spec for callers that only need metadata.
pub fn spec(name: &str) -> Option<&'static ToolSpec> {
    tool_spec(name)
}

/// Convenience path helper for callers that build their own display output.
pub fn platform_db(root: &Path) -> PathBuf {
    root.join("platform.db")
}

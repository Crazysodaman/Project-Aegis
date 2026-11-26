//! One-shot module tool execution.
//!
//! A module tool is executed by launching the module's own package entrypoint
//! in tool mode with a private input file and an output file. This keeps module
//! tools on the same process-based execution path as module services and keeps
//! Platform authorization in `aegis-tools`.
//!
//! The child receives only the module ID, tool name, caller Core UUID, and the
//! tool arguments. It receives no owner token, Platform database path, or
//! private Core data path.

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use uuid::Uuid;

use crate::policy::{ExecutionPolicy, ModuleTrust, PolicyError};
use crate::registry::{ModuleError, ModuleLifecycleState, ModuleRegistry};
use crate::runtime::resolve_entrypoint;

/// Maximum serialized tool input accepted by the runner.
pub const MAX_TOOL_INPUT_BYTES: usize = 64 * 1024;
/// Maximum serialized tool output read from the module.
pub const MAX_TOOL_OUTPUT_BYTES: usize = 1024 * 1024;

const POLL_INTERVAL_MS: u64 = 25;

#[derive(Debug, thiserror::Error)]
pub enum ModuleToolError {
    #[error("module tool storage failed: {0}")]
    Storage(#[from] aegis_storage::StorageError),
    #[error("module registry failed: {0}")]
    Registry(#[from] ModuleError),
    #[error("module tool I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("module tool JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("module {module_id} is {state}; enable it before running tools")]
    NotEnabled { module_id: String, state: String },
    #[error("module {module_id} does not declare tool {tool}")]
    ToolNotFound { module_id: String, tool: String },
    #[error("module tool input is too large")]
    InputTooLarge,
    #[error("module tool output is too large")]
    OutputTooLarge,
    #[error("module execution policy is invalid: {0}")]
    Policy(#[from] PolicyError),
    #[error("module {module_id} is restricted; an explicit allow_restricted policy is required")]
    RestrictedDenied { module_id: String },
    #[error("module {module_id} declares {declared} tools but the policy allows {allowed}")]
    ToolCountExceeded {
        module_id: String,
        declared: usize,
        allowed: usize,
    },
    #[error("module entrypoint is unavailable: {0}")]
    Entrypoint(String),
    #[error("module tool {tool} failed to spawn: {message}")]
    Spawn { tool: String, message: String },
    #[error("module tool {tool} timed out after {timeout_ms} ms")]
    Timeout { tool: String, timeout_ms: u64 },
    #[error("module tool {tool} exited with {status}")]
    Failed { tool: String, status: String },
    #[error("module tool {tool} returned an invalid result: {detail}")]
    InvalidOutput { tool: String, detail: String },
    #[error("module tool {tool} reported failure: {detail}")]
    Reported { tool: String, detail: String },
}

/// Run one module-declared tool through its package entrypoint.
///
/// Uses the default `trusted_local` [`ExecutionPolicy`]. Call
/// [`run_module_tool_with_policy`] to apply a tighter or restricted policy.
pub fn run_module_tool(
    root: &Path,
    module_id: &str,
    tool_name: &str,
    caller_core_uuid: &str,
    arguments: &Value,
    timeout: Duration,
) -> Result<Value, ModuleToolError> {
    run_module_tool_with_policy(
        root,
        module_id,
        tool_name,
        caller_core_uuid,
        arguments,
        timeout,
        &ExecutionPolicy::default(),
    )
}

/// Run one module-declared tool under an explicit trust/limits policy.
///
/// The policy is validated first. A `restricted` module is denied unless the
/// policy sets `allow_restricted`. Input, output, and timeout are bounded by the
/// policy (and never above the framework hard caps). Declared-tool count is
/// checked against the manifest before spawning.
#[allow(clippy::too_many_arguments)]
pub fn run_module_tool_with_policy(
    root: &Path,
    module_id: &str,
    tool_name: &str,
    caller_core_uuid: &str,
    arguments: &Value,
    timeout: Duration,
    policy: &ExecutionPolicy,
) -> Result<Value, ModuleToolError> {
    policy.validate()?;
    if policy.trust == ModuleTrust::Restricted && !policy.allow_restricted {
        return Err(ModuleToolError::RestrictedDenied {
            module_id: module_id.to_owned(),
        });
    }
    let registry = ModuleRegistry::new(aegis_storage::StoragePaths::new(root)?);
    let inspection = registry.inspect(module_id)?;
    if inspection.record.lifecycle_state != ModuleLifecycleState::Enabled {
        return Err(ModuleToolError::NotEnabled {
            module_id: module_id.to_owned(),
            state: inspection.record.lifecycle_state.as_str().to_owned(),
        });
    }
    if inspection.manifest.tools.len() > policy.max_tool_count {
        return Err(ModuleToolError::ToolCountExceeded {
            module_id: module_id.to_owned(),
            declared: inspection.manifest.tools.len(),
            allowed: policy.max_tool_count,
        });
    }
    let tool = inspection
        .manifest
        .tools
        .iter()
        .find(|tool| tool.name == tool_name)
        .ok_or_else(|| ModuleToolError::ToolNotFound {
            module_id: module_id.to_owned(),
            tool: tool_name.to_owned(),
        })?;
    let package_dir = root.join("modules").join(module_id);
    let executable = resolve_entrypoint(&package_dir, module_id, &inspection.manifest)
        .map_err(|error| ModuleToolError::Entrypoint(error.to_string()))?;

    let input = json!({
        "module_id": module_id,
        "tool": tool.name,
        "core_uuid": caller_core_uuid,
        "arguments": arguments,
    });
    let input_bytes = serde_json::to_vec(&input)?;
    if input_bytes.len() > policy.effective_input_bytes() {
        return Err(ModuleToolError::InputTooLarge);
    }

    let run_root = root
        .join("run")
        .join("modules")
        .join(module_id)
        .join("tools")
        .join(Uuid::new_v4().to_string());
    fs::create_dir_all(&run_root)?;
    let input_path = run_root.join("input.json");
    let output_path = run_root.join("output.json");
    fs::write(&input_path, &input_bytes)?;

    let result = execute(
        &executable,
        &package_dir,
        module_id,
        tool_name,
        caller_core_uuid,
        &input_path,
        &output_path,
        policy.effective_runtime(timeout),
        policy.effective_output_bytes(),
    );
    let _ = fs::remove_dir_all(&run_root);
    result
}

#[allow(clippy::too_many_arguments)]
fn execute(
    executable: &Path,
    package_dir: &Path,
    module_id: &str,
    tool_name: &str,
    caller_core_uuid: &str,
    input_path: &Path,
    output_path: &Path,
    timeout: Duration,
    max_output_bytes: usize,
) -> Result<Value, ModuleToolError> {
    let mut command = Command::new(executable);
    command
        .arg("--aegis-tool")
        .arg(tool_name)
        .arg("--input")
        .arg(input_path)
        .arg("--output")
        .arg(output_path)
        .arg("--module-id")
        .arg(module_id)
        .current_dir(package_dir)
        .env_clear()
        .env("AEGIS_MODULE_ID", module_id)
        .env("AEGIS_MODULE_TOOL", tool_name)
        .env("AEGIS_CORE_UUID", caller_core_uuid)
        .env("AEGIS_MODULE_DIR", package_dir)
        .env("AEGIS_TOOL_INPUT", input_path)
        .env("AEGIS_TOOL_OUTPUT", output_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    if let Ok(system_root) = std::env::var("SYSTEMROOT") {
        command.env("SYSTEMROOT", system_root);
    }

    let mut child = command.spawn().map_err(|error| ModuleToolError::Spawn {
        tool: tool_name.to_owned(),
        message: format!("could not spawn {}: {error}", executable.display()),
    })?;
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                if let Some(reported) =
                    read_reported_failure(output_path, tool_name, max_output_bytes)?
                {
                    return Err(reported);
                }
                return Err(ModuleToolError::Failed {
                    tool: tool_name.to_owned(),
                    status: status.to_string(),
                });
            }
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ModuleToolError::Timeout {
                tool: tool_name.to_owned(),
                timeout_ms: timeout.as_millis().min(u64::MAX as u128) as u64,
            });
        }
        thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
    }

    let metadata = match fs::symlink_metadata(output_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(ModuleToolError::InvalidOutput {
                tool: tool_name.to_owned(),
                detail: "module produced no output file".to_owned(),
            })
        }
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(ModuleToolError::InvalidOutput {
            tool: tool_name.to_owned(),
            detail: "output is not a regular file".to_owned(),
        });
    }
    if metadata.len() > max_output_bytes as u64 {
        return Err(ModuleToolError::OutputTooLarge);
    }
    let bytes = fs::read(output_path)?;
    let value: Value = serde_json::from_slice(&bytes)?;
    let object = value
        .as_object()
        .ok_or_else(|| ModuleToolError::InvalidOutput {
            tool: tool_name.to_owned(),
            detail: "output must be a JSON object".to_owned(),
        })?;
    match object.get("ok").and_then(Value::as_bool) {
        Some(true) => Ok(object.get("output").cloned().unwrap_or(Value::Null)),
        Some(false) => Err(ModuleToolError::Reported {
            tool: tool_name.to_owned(),
            detail: object
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("module reported failure")
                .chars()
                .take(512)
                .collect(),
        }),
        None => Err(ModuleToolError::InvalidOutput {
            tool: tool_name.to_owned(),
            detail: "output must contain a boolean 'ok' field".to_owned(),
        }),
    }
}

fn read_reported_failure(
    output_path: &Path,
    tool_name: &str,
    max_output_bytes: usize,
) -> Result<Option<ModuleToolError>, ModuleToolError> {
    let Ok(bytes) = fs::read(output_path) else {
        return Ok(None);
    };
    if bytes.len() > max_output_bytes {
        return Ok(Some(ModuleToolError::OutputTooLarge));
    }
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return Ok(None);
    };
    let Some(object) = value.as_object() else {
        return Ok(None);
    };
    if object.get("ok").and_then(Value::as_bool) == Some(false) {
        return Ok(Some(ModuleToolError::Reported {
            tool: tool_name.to_owned(),
            detail: object
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("module reported failure")
                .chars()
                .take(512)
                .collect(),
        }));
    }
    Ok(None)
}

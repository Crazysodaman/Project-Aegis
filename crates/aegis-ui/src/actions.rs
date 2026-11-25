//! Real mutation handling for `POST /api/actions`.
//!
//! The session token guards the local dashboard only. Owner authority is loaded
//! from the Safe store and is never returned, logged, or echoed in an error.

use std::path::Path;

use aegis_contracts::ipc::{METHOD_SAFE_STOP_CLEAR, METHOD_SAFE_STOP_SET};
use aegis_core_install::CoreInstaller;
use aegis_core_runtime::CoreRuntime;
use aegis_platform::PlatformService;
use aegis_storage::StoragePaths;
use serde_json::{json, Value};

const UPDATED_BY: &str = "aegis-dashboard";

/// Handle one action request. The caller has already parsed the HTTP request.
pub(crate) fn handle_action(
    root: &Path,
    allow_mutations: bool,
    session_token: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> (u16, Value) {
    let provided = headers
        .iter()
        .find(|(name, _)| name == "x-aegis-token")
        .map(|(_, value)| value.as_str());
    let authorized = provided
        .map(|value| token_matches(value, session_token))
        .unwrap_or(false);
    if !authorized {
        return forbidden();
    }
    if !allow_mutations {
        return (403, json!({"ok": false, "error": "mutations_disabled"}));
    }

    let request: Value = match serde_json::from_slice(body) {
        Ok(request) => request,
        Err(_) => return (400, json!({"ok": false, "error": "invalid_json"})),
    };
    let action = match request.get("action").and_then(Value::as_str) {
        Some(action) if !action.is_empty() && action.len() <= 64 => action,
        _ => return (400, json!({"ok": false, "error": "missing_action"})),
    };

    let owner_token = match load_owner_token(root) {
        Ok(token) => token,
        Err(error) => return internal(&error, &[session_token]),
    };
    let secrets = [session_token, owner_token.as_str()];

    match action {
        "core.start" => start_core(root, &request, &owner_token, &secrets),
        "core.stop" => stop_core(root, &request, &owner_token, &secrets),
        "core.deactivate" => lifecycle(
            root,
            &request,
            &owner_token,
            &secrets,
            LifecycleAction::Deactivate,
        ),
        "core.reactivate" => lifecycle(
            root,
            &request,
            &owner_token,
            &secrets,
            LifecycleAction::Reactivate,
        ),
        "core.uninstall_preserve" => lifecycle(
            root,
            &request,
            &owner_token,
            &secrets,
            LifecycleAction::Uninstall,
        ),
        "safe.stop.set" => set_stop(root, &request, &owner_token, &secrets),
        "safe.stop.clear" => clear_stop(root, &request, &owner_token, &secrets),
        _ => (400, json!({"ok": false, "error": "unknown_action"})),
    }
}

enum LifecycleAction {
    Deactivate,
    Reactivate,
    Uninstall,
}

fn start_core(root: &Path, request: &Value, owner_token: &str, secrets: &[&str]) -> (u16, Value) {
    let Some(core_uuid) = core_uuid(request) else {
        return invalid("core_uuid");
    };
    let paths = match StoragePaths::new(root) {
        Ok(paths) => paths,
        Err(error) => return internal(&error.to_string(), secrets),
    };
    match CoreRuntime::new(paths).start(core_uuid, owner_token) {
        Ok(status) => (
            200,
            json!({
                "ok": true,
                "result": {
                    "action": "core.start",
                    "core_uuid": status.core_uuid,
                    "state": status.state.as_str(),
                    "pid": status.pid,
                    "heartbeat_fresh": status.heartbeat_fresh,
                }
            }),
        ),
        Err(error) => failure(&error.to_string(), secrets),
    }
}

fn stop_core(root: &Path, request: &Value, owner_token: &str, secrets: &[&str]) -> (u16, Value) {
    let Some(core_uuid) = core_uuid(request) else {
        return invalid("core_uuid");
    };
    let paths = match StoragePaths::new(root) {
        Ok(paths) => paths,
        Err(error) => return internal(&error.to_string(), secrets),
    };
    match CoreRuntime::new(paths).stop(core_uuid, owner_token) {
        Ok(status) => (
            200,
            json!({
                "ok": true,
                "result": {
                    "action": "core.stop",
                    "core_uuid": status.core_uuid,
                    "state": status.state.as_str(),
                    "pid": status.pid,
                    "heartbeat_fresh": status.heartbeat_fresh,
                }
            }),
        ),
        Err(error) => failure(&error.to_string(), secrets),
    }
}

fn lifecycle(
    root: &Path,
    request: &Value,
    owner_token: &str,
    secrets: &[&str],
    action: LifecycleAction,
) -> (u16, Value) {
    let Some(core_uuid) = core_uuid(request) else {
        return invalid("core_uuid");
    };
    let Some(reason) = reason(request) else {
        return invalid("reason");
    };
    let installer = match CoreInstaller::new(root) {
        Ok(installer) => installer,
        Err(error) => return internal(&error.to_string(), secrets),
    };
    let result = match action {
        LifecycleAction::Deactivate => installer.deactivate(core_uuid, reason, owner_token),
        LifecycleAction::Reactivate => installer.reactivate(core_uuid, reason, owner_token),
        LifecycleAction::Uninstall => installer.uninstall_preserve(core_uuid, reason, owner_token),
    };
    match result {
        Ok(entry) => (200, json!({"ok": true, "result": entry})),
        Err(error) => failure(&error.to_string(), secrets),
    }
}

fn set_stop(root: &Path, request: &Value, owner_token: &str, secrets: &[&str]) -> (u16, Value) {
    let Some(reason) = reason(request) else {
        return invalid("reason");
    };
    let service = match PlatformService::start(root) {
        Ok(service) => service,
        Err(error) => return internal(&error.to_string(), secrets),
    };
    let params = json!({"reason": reason, "updated_by": UPDATED_BY});
    match service.dispatch(METHOD_SAFE_STOP_SET, &params, owner_token) {
        Ok(result) => (200, json!({"ok": true, "result": result})),
        Err(error) => failure(&error.to_string(), secrets),
    }
}

fn clear_stop(root: &Path, request: &Value, owner_token: &str, secrets: &[&str]) -> (u16, Value) {
    if reason(request).is_none() {
        return invalid("reason");
    }
    let service = match PlatformService::start(root) {
        Ok(service) => service,
        Err(error) => return internal(&error.to_string(), secrets),
    };
    let params = json!({"updated_by": UPDATED_BY});
    match service.dispatch(METHOD_SAFE_STOP_CLEAR, &params, owner_token) {
        Ok(result) => (200, json!({"ok": true, "result": result})),
        Err(error) => failure(&error.to_string(), secrets),
    }
}

fn load_owner_token(root: &Path) -> Result<String, String> {
    let service = PlatformService::start(root).map_err(|error| error.to_string())?;
    service.owner_token().map_err(|error| error.to_string())
}

fn core_uuid(request: &Value) -> Option<&str> {
    request
        .get("core_uuid")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 64)
}

fn reason(request: &Value) -> Option<&str> {
    request
        .get("reason")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= 512)
}

fn token_matches(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0u8;
    for (a, b) in left.iter().zip(right) {
        difference |= a ^ b;
    }
    difference == 0
}

fn forbidden() -> (u16, Value) {
    (403, json!({"ok": false, "error": "forbidden"}))
}

fn invalid(field: &str) -> (u16, Value) {
    (
        400,
        json!({"ok": false, "error": format!("missing {field}")}),
    )
}

fn internal(message: &str, secrets: &[&str]) -> (u16, Value) {
    (500, json!({"ok": false, "error": redact(message, secrets)}))
}

fn failure(message: &str, secrets: &[&str]) -> (u16, Value) {
    (409, json!({"ok": false, "error": redact(message, secrets)}))
}

fn redact(message: &str, secrets: &[&str]) -> String {
    const MAX_CHARS: usize = 512;
    let mut redacted = message.to_owned();
    for secret in secrets {
        if !secret.is_empty() {
            redacted = redacted.replace(secret, "[redacted]");
        }
    }
    if redacted.chars().count() > MAX_CHARS {
        redacted = redacted.chars().take(MAX_CHARS).collect::<String>();
    }
    redacted
}

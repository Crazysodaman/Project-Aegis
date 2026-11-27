//! Acceptance tests for Core-facing consent tools.
//!
//! These exercise the real `ToolExecutor` (registered tools, grants, STOP,
//! owner approval, audit) against real per-Core consent storage, including
//! cross-Core isolation.

use serde_json::{json, Value};
use tempfile::TempDir;

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_core_install::CoreInstaller;
use aegis_platform::PlatformService;
use aegis_storage::now_rfc3339;
use aegis_tools::{ToolExecutor, ToolStatus};

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_B: &str = "11111111-1111-4111-8111-111111111111";

fn request(core_uuid: &str, number: u8, name: &str) -> CoreInitializationRequest {
    CoreInitializationRequest {
        schema: CORE_INIT_REQUEST_SCHEMA.to_owned(),
        core_uuid: core_uuid.to_owned(),
        core_number: number,
        display_name: name.to_owned(),
        requested_at: now_rfc3339().expect("timestamp"),
    }
}

fn setup() -> (TempDir, String) {
    let temp = tempfile::tempdir().expect("tempdir");
    let initialized = PlatformService::initialize(temp.path()).expect("platform");
    let installer = CoreInstaller::new(temp.path()).expect("installer");
    for (uuid, number, name) in [(CORE_A, 1, "Core A"), (CORE_B, 2, "Core B")] {
        installer
            .initialize(&request(uuid, number, name), &initialized.owner_token)
            .expect("install core");
    }
    (temp, initialized.owner_token)
}

fn executor(temp: &TempDir, core: &str) -> ToolExecutor {
    ToolExecutor::new(temp.path(), core).expect("executor")
}

fn exec(executor: &ToolExecutor, tool: &str, arguments: Value) -> Value {
    let result = executor.execute(tool, &arguments, None).expect("execute");
    assert_eq!(
        result.status,
        ToolStatus::Executed,
        "{tool} failed: {:?}",
        result.detail
    );
    result.output
}

fn grant_conversation(executor: &ToolExecutor, capability: &str) -> Value {
    exec(
        executor,
        "consent.grant",
        json!({
            "scope": "conversation",
            "capability": capability,
            "participants": ["owner"],
            "context": "explicit test consent",
        }),
    )
}

#[test]
fn grant_then_check_reports_active_consent() {
    let (temp, _token) = setup();
    let executor = executor(&temp, CORE_A);

    let granted = grant_conversation(&executor, "chat");
    assert_eq!(granted["scope"].as_str(), Some("conversation"));
    assert_eq!(granted["capability"].as_str(), Some("chat"));
    assert_eq!(granted["core_uuid"].as_str(), Some(CORE_A));
    assert!(granted["grant_id"].is_string());
    assert!(granted["revoked_at"].is_null());

    let matching = exec(
        &executor,
        "consent.check",
        json!({"scope": "conversation", "capability": "chat", "participant": "owner"}),
    );
    assert_eq!(matching["active"], true);

    let stranger = exec(
        &executor,
        "consent.check",
        json!({"scope": "conversation", "capability": "chat", "participant": "stranger"}),
    );
    assert_eq!(stranger["active"], false);

    let wrong_capability = exec(
        &executor,
        "consent.check",
        json!({"scope": "conversation", "capability": "sing", "participant": "owner"}),
    );
    assert_eq!(wrong_capability["active"], false);

    let without_participant = exec(
        &executor,
        "consent.check",
        json!({"scope": "conversation", "capability": "chat"}),
    );
    assert_eq!(without_participant["active"], true);
}

#[test]
fn revoke_takes_effect_immediately() {
    let (temp, _token) = setup();
    let executor = executor(&temp, CORE_A);

    let granted = grant_conversation(&executor, "chat");
    let grant_id = granted["grant_id"].as_str().expect("grant id").to_owned();
    let check = json!({"scope": "conversation", "capability": "chat", "participant": "owner"});
    assert_eq!(
        exec(&executor, "consent.check", check.clone())["active"],
        true
    );

    let revoked = exec(&executor, "consent.revoke", json!({"grant_id": grant_id}));
    assert!(revoked["revoked_at"].is_string());

    assert_eq!(exec(&executor, "consent.check", check)["active"], false);

    let again = exec(&executor, "consent.revoke", json!({"grant_id": grant_id}));
    assert_eq!(again["revoked_at"], revoked["revoked_at"]);
}

#[test]
fn list_returns_grants_and_respects_limit() {
    let (temp, _token) = setup();
    let executor = executor(&temp, CORE_A);

    grant_conversation(&executor, "chat");
    grant_conversation(&executor, "plan");

    let listed = exec(&executor, "consent.list", json!({}));
    let grants = listed.as_array().expect("grant array");
    assert_eq!(grants.len(), 2);
    assert!(grants
        .iter()
        .any(|grant| grant["capability"].as_str() == Some("chat")));
    assert!(grants
        .iter()
        .any(|grant| grant["capability"].as_str() == Some("plan")));

    let limited = exec(&executor, "consent.list", json!({"limit": 1}));
    assert_eq!(limited.as_array().expect("limited array").len(), 1);
}

#[test]
fn non_intimate_tool_rejects_intimate_and_unknown_scopes() {
    let (temp, _token) = setup();
    let executor = executor(&temp, CORE_A);

    for scope in ["affection", "romantic", "adult", "bogus"] {
        let rejected = executor
            .execute(
                "consent.grant",
                &json!({"scope": scope, "capability": "chat"}),
                None,
            )
            .expect("execute");
        assert_eq!(
            rejected.status,
            ToolStatus::Failed,
            "scope {scope} must be rejected"
        );
    }

    let listed = exec(&executor, "consent.list", json!({}));
    assert!(listed.as_array().expect("array").is_empty());
}

#[test]
fn intimate_tool_requires_owner_approval_then_records_consent() {
    let (temp, token) = setup();
    let executor = executor(&temp, CORE_A);
    let arguments = json!({
        "scope": "romantic",
        "capability": "hold_hands",
        "participants": ["owner"],
        "context": "explicit intimate consent",
    });

    let without_owner = executor
        .execute("consent.grant_intimate", &arguments, None)
        .expect("execute");
    assert_eq!(without_owner.status, ToolStatus::ApprovalRequired);
    assert!(exec(&executor, "consent.list", json!({}))
        .as_array()
        .expect("array")
        .is_empty());

    let with_owner = executor
        .execute("consent.grant_intimate", &arguments, Some(&token))
        .expect("execute");
    assert_eq!(
        with_owner.status,
        ToolStatus::Executed,
        "{:?}",
        with_owner.detail
    );
    assert_eq!(with_owner.output["scope"].as_str(), Some("romantic"));

    let check = exec(
        &executor,
        "consent.check",
        json!({"scope": "romantic", "capability": "hold_hands", "participant": "owner"}),
    );
    assert_eq!(check["active"], true);

    let rejected_scope = executor
        .execute(
            "consent.grant_intimate",
            &json!({"scope": "conversation", "capability": "chat"}),
            Some(&token),
        )
        .expect("execute");
    assert_eq!(rejected_scope.status, ToolStatus::Failed);
}

#[test]
fn consent_is_isolated_between_cores() {
    let (temp, _token) = setup();
    let core_a = executor(&temp, CORE_A);
    let core_b = executor(&temp, CORE_B);

    let granted = grant_conversation(&core_a, "chat");
    let grant_id = granted["grant_id"].as_str().expect("grant id").to_owned();

    let a_list = exec(&core_a, "consent.list", json!({}));
    assert_eq!(a_list.as_array().expect("array").len(), 1);

    let b_list = exec(&core_b, "consent.list", json!({}));
    assert!(b_list.as_array().expect("array").is_empty());

    let check = json!({"scope": "conversation", "capability": "chat", "participant": "owner"});
    assert_eq!(
        exec(&core_b, "consent.check", check.clone())["active"],
        false
    );

    let b_revoke = core_b
        .execute("consent.revoke", &json!({"grant_id": grant_id}), None)
        .expect("execute");
    assert_eq!(b_revoke.status, ToolStatus::Failed);

    // Core A's grant survives Core B's attempt.
    assert_eq!(exec(&core_a, "consent.check", check)["active"], true);
}

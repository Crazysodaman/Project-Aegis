use std::path::{Path, PathBuf};

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_core_install::CoreInstaller;
use aegis_core_sdk::{CoreMemoryStore, MemoryKind, MemoryQuery, NewMemory};
use aegis_platform::PlatformService;
use aegis_safe::SafeStore;
use aegis_storage::{now_rfc3339, StoragePaths};
use aegis_tools::{ToolExecutor, ToolRegistry, ToolStatus};
use serde_json::json;

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_B: &str = "11111111-1111-4111-8111-111111111111";

fn setup() -> (tempfile::TempDir, PathBuf, String) {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("aegis-root");
    let initialized = PlatformService::initialize(&root).expect("platform");
    let installer = CoreInstaller::new(&root).expect("installer");
    for (uuid, number, name) in [(CORE_A, 1, "Sofía Ada Lyra"), (CORE_B, 2, "Mack Fuller")] {
        installer
            .initialize(
                &CoreInitializationRequest {
                    schema: CORE_INIT_REQUEST_SCHEMA.to_owned(),
                    core_uuid: uuid.to_owned(),
                    core_number: number,
                    display_name: name.to_owned(),
                    requested_at: now_rfc3339().expect("time"),
                },
                &initialized.owner_token,
            )
            .expect("core");
    }
    (temp, root, initialized.owner_token)
}

fn executor(root: &Path, core: &str) -> ToolExecutor {
    ToolExecutor::new(root, core).expect("executor")
}

fn registry(root: &Path) -> ToolRegistry {
    let paths = StoragePaths::new(root).expect("paths");
    let registry = ToolRegistry::new(paths);
    registry.initialize().expect("registry");
    registry
}

fn safe(root: &Path) -> SafeStore {
    SafeStore::new(StoragePaths::new(root).expect("paths"))
}

fn grant(registry: &ToolRegistry, root: &Path, tool: &str, granted: bool) {
    let token = safe(root).owner_token().expect("token");
    registry
        .set_permission(tool, granted, true, &token)
        .expect("set permission");
}

#[test]
fn default_grants_allow_read_tools_and_deny_memory_write() {
    let (_temp, root, _token) = setup();
    let registry = registry(&root);
    let grants = registry.list().expect("list");
    let search = grants
        .iter()
        .find(|grant| grant.name == "memory.search")
        .expect("search");
    assert!(search.granted);
    let create = grants
        .iter()
        .find(|grant| grant.name == "memory.create")
        .expect("create");
    assert!(!create.granted);
    assert!(create.approval_required);

    let executor = executor(&root, CORE_A);
    let result = executor
        .execute("time.now", &json!({}), None)
        .expect("execute");
    assert_eq!(result.status, ToolStatus::Executed);
    assert!(result.output["now"].is_string());
    assert!(result.audited);
}

#[test]
fn sensitive_tool_requires_grant_and_owner_approval() {
    let (_temp, root, token) = setup();
    let registry = registry(&root);
    let executor = executor(&root, CORE_A);

    let denied = executor
        .execute(
            "memory.create",
            &json!({"content": "The owner likes tea."}),
            None,
        )
        .expect("execute");
    assert_eq!(denied.status, ToolStatus::Denied);

    grant(&registry, &root, "memory.create", true);
    let approval = executor
        .execute(
            "memory.create",
            &json!({"content": "The owner likes tea."}),
            None,
        )
        .expect("execute");
    assert_eq!(approval.status, ToolStatus::ApprovalRequired);

    let executed = executor
        .execute(
            "memory.create",
            &json!({"content": "The owner likes tea.", "kind": "preference"}),
            Some(&token),
        )
        .expect("execute");
    assert_eq!(executed.status, ToolStatus::Executed);
    assert_eq!(executed.output["content"], "The owner likes tea.");

    let store = CoreMemoryStore::open(&root, CORE_A).expect("memory");
    let entries = store
        .search(&MemoryQuery::new().with_text("tea"))
        .expect("search");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].provenance, "model_tool");
}

#[test]
fn grant_changes_require_owner_proof() {
    let (_temp, root, _token) = setup();
    let registry = registry(&root);
    let error = registry
        .set_permission("memory.create", true, true, "not-the-owner")
        .expect_err("must deny");
    assert!(error.to_string().contains("authorization"));
    let create = registry
        .list()
        .expect("list")
        .into_iter()
        .find(|grant| grant.name == "memory.create")
        .expect("create");
    assert!(!create.granted);
}

#[test]
fn memory_search_is_isolated_per_core() {
    let (_temp, root, _token) = setup();
    let store_a = CoreMemoryStore::open(&root, CORE_A).expect("memory a");
    store_a
        .create(NewMemory::new(
            MemoryKind::Fact,
            "Sofía knows a private fact.",
        ))
        .expect("create");
    let store_b = CoreMemoryStore::open(&root, CORE_B).expect("memory b");
    store_b
        .create(NewMemory::new(
            MemoryKind::Fact,
            "Mack knows a different fact.",
        ))
        .expect("create");

    let a = executor(&root, CORE_A)
        .execute("memory.search", &json!({"text": "Sofía"}), None)
        .expect("search a");
    assert_eq!(a.status, ToolStatus::Executed);
    let a_text = a.output.to_string();
    assert!(a_text.contains("Sofía"));
    assert!(!a_text.contains("Mack"));

    let b = executor(&root, CORE_B)
        .execute("memory.search", &json!({"text": "Sofía"}), None)
        .expect("search b");
    assert_eq!(b.status, ToolStatus::Executed);
    assert_eq!(b.output.as_array().expect("array").len(), 0);
}

#[test]
fn stop_blocks_execution_until_cleared() {
    let (_temp, root, token) = setup();
    let executor = executor(&root, CORE_A);
    safe(&root)
        .set_stop("owner", "maintenance", &token)
        .expect("stop");
    let blocked = executor
        .execute("time.now", &json!({}), None)
        .expect("execute");
    assert_eq!(blocked.status, ToolStatus::Denied);
    assert!(blocked.detail.expect("detail").contains("STOP"));

    safe(&root).clear_stop("owner", &token).expect("clear");
    let allowed = executor
        .execute("time.now", &json!({}), None)
        .expect("execute");
    assert_eq!(allowed.status, ToolStatus::Executed);
}

#[test]
fn unknown_tool_is_denied_and_audited() {
    let (_temp, root, _token) = setup();
    let before = safe(&root).audit_count().expect("count");
    let result = executor(&root, CORE_A)
        .execute("shell.exec", &json!({"command": "whoami"}), None)
        .expect("execute");
    assert_eq!(result.status, ToolStatus::Denied);
    let after = safe(&root).audit_count().expect("count");
    assert!(after > before);
    assert!(safe(&root).audit_chain_valid().expect("chain"));
}

#[test]
fn core_inspect_exposes_metadata_not_private_memory() {
    let (_temp, root, _token) = setup();
    CoreMemoryStore::open(&root, CORE_A)
        .expect("memory")
        .create(NewMemory::new(MemoryKind::Fact, "TOP-SECRET-A"))
        .expect("create");
    let result = executor(&root, CORE_B)
        .execute("core.inspect", &json!({"core_uuid": CORE_A}), None)
        .expect("inspect");
    assert_eq!(result.status, ToolStatus::Executed);
    assert_eq!(result.output["identity"]["display_name"], "Sofía Ada Lyra");
    assert!(!result.output.to_string().contains("TOP-SECRET-A"));
}

#[test]
fn revoking_a_read_tool_blocks_it() {
    let (_temp, root, _token) = setup();
    let registry = registry(&root);
    grant(&registry, &root, "time.now", false);
    let result = executor(&root, CORE_A)
        .execute("time.now", &json!({}), None)
        .expect("execute");
    assert_eq!(result.status, ToolStatus::Denied);
}

#[test]
fn platform_and_module_tools_execute() {
    let (_temp, root, _token) = setup();
    let executor = executor(&root, CORE_A);
    let status = executor
        .execute("platform.status", &json!({}), None)
        .expect("status");
    assert_eq!(status.status, ToolStatus::Executed);
    assert_eq!(status.output["core_count"], 2);

    let modules = executor
        .execute("module.list", &json!({}), None)
        .expect("modules");
    assert_eq!(modules.status, ToolStatus::Executed);
    assert!(modules.output.is_array());
}

#[test]
fn invalid_arguments_fail_without_executing() {
    let (_temp, root, token) = setup();
    let registry = registry(&root);
    grant(&registry, &root, "memory.create", true);
    let executor = executor(&root, CORE_A);
    let result = executor
        .execute("memory.create", &json!({"content": 42}), Some(&token))
        .expect("execute");
    assert_eq!(result.status, ToolStatus::Failed);
    let store = CoreMemoryStore::open(&root, CORE_A).expect("memory");
    assert!(store
        .search(&MemoryQuery::new())
        .expect("search")
        .is_empty());

    let result = executor
        .execute("memory.search", &json!({"text": 42}), None)
        .expect("execute");
    assert_eq!(result.status, ToolStatus::Failed);
}

#[test]
fn audit_chain_survives_many_tool_attempts() {
    let (_temp, root, _token) = setup();
    let executor = executor(&root, CORE_A);
    for _ in 0..5 {
        executor
            .execute("time.now", &json!({}), None)
            .expect("execute");
        executor
            .execute("memory.create", &json!({"content": "x"}), None)
            .expect("denied");
    }
    assert!(safe(&root).audit_chain_valid().expect("chain"));
}

use std::fs;
use std::path::{Path, PathBuf};

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_core_install::CoreInstaller;
use aegis_modules::{
    ModuleAsset, ModuleEntrypoint, ModuleIntegrity, ModuleManager, ModuleManifest,
    ModulePermission, ModuleResourceLimits, ModuleSdkCompatibility, ModuleToolDeclaration,
    MODULE_MANIFEST_SCHEMA, MODULE_SDK_NAME,
};
use aegis_platform::PlatformService;
use aegis_safe::SafeStore;
use aegis_storage::{now_rfc3339, StoragePaths};
use aegis_tools::{ToolExecutor, ToolRegistry, ToolStatus};
use serde_json::json;
use sha2::{Digest, Sha256};

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_B: &str = "11111111-1111-4111-8111-111111111111";
const MODULE: &str = "com.example.toolmod";

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

fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn tool_decl(
    name: &str,
    permission: &str,
    sensitive: bool,
    timeout_ms: Option<u64>,
    required: Vec<&str>,
) -> ModuleToolDeclaration {
    ModuleToolDeclaration {
        name: name.to_owned(),
        description: format!("Test tool {name}"),
        permission: permission.to_owned(),
        sensitive,
        approval_required: sensitive,
        timeout_ms,
        input_schema: json!({"type": "object", "required": required}),
    }
}

fn build_package(parent: &Path, tools: Vec<ModuleToolDeclaration>) -> PathBuf {
    let package = parent.join("module-package");
    let bin_relative = format!(
        "bin/aegis-tools-test-module{}",
        std::env::consts::EXE_SUFFIX
    );
    let bin_path = package.join(&bin_relative);
    fs::create_dir_all(bin_path.parent().expect("bin parent")).expect("bin dir");
    fs::copy(env!("CARGO_BIN_EXE_aegis-tools-test-module"), &bin_path).expect("copy bin");
    let bytes = fs::read(&bin_path).expect("bin bytes");
    let mut manifest = ModuleManifest {
        schema: MODULE_MANIFEST_SCHEMA.to_owned(),
        module_id: MODULE.to_owned(),
        name: "Tool Module".to_owned(),
        version: "0.1.0".to_owned(),
        publisher: "Aegis Tests".to_owned(),
        sdk: ModuleSdkCompatibility {
            sdk: MODULE_SDK_NAME.to_owned(),
            min_version: "1.0.0".to_owned(),
            max_version: None,
        },
        capabilities: vec!["tools".to_owned()],
        permissions: vec![ModulePermission {
            name: "tools.execute".to_owned(),
            reason: "Expose test tools.".to_owned(),
        }],
        dependencies: Vec::new(),
        entrypoint: ModuleEntrypoint {
            kind: "process".to_owned(),
            runtime: "aegis-module-runtime-v1".to_owned(),
            path: bin_relative.clone(),
            symbol: None,
            args: Vec::new(),
        },
        assets: vec![ModuleAsset {
            path: bin_relative,
            sha256: sha256_hex(&bytes),
            bytes: bytes.len() as u64,
        }],
        config_schema: json!({"type": "object", "properties": {}}),
        lifecycle: Vec::new(),
        tools,
        resources: ModuleResourceLimits {
            max_memory_bytes: 64 * 1024 * 1024,
            max_cpu_millis: 60_000,
            max_disk_bytes: 64 * 1024 * 1024,
            max_open_files: 64,
        },
        integrity: ModuleIntegrity {
            algorithm: "sha256".to_owned(),
            digest: String::new(),
        },
    };
    manifest.seal().expect("seal");
    fs::write(
        package.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).expect("manifest json"),
    )
    .expect("write manifest");
    package
}

fn install_and_enable(root: &Path, token: &str, tools: Vec<ModuleToolDeclaration>) {
    let temp = tempfile::tempdir().expect("package temp");
    let package = build_package(temp.path(), tools);
    let manager = ModuleManager::new(StoragePaths::new(root).expect("paths"));
    manager
        .install_package(&package, None, token)
        .expect("install package");
    manager.enable(MODULE, token).expect("enable");
}

fn registry(root: &Path) -> ToolRegistry {
    let registry = ToolRegistry::new(StoragePaths::new(root).expect("paths"));
    registry.initialize().expect("registry");
    registry
}

fn safe(root: &Path) -> SafeStore {
    SafeStore::new(StoragePaths::new(root).expect("paths"))
}

fn grant(root: &Path, token: &str, tool: &str) {
    registry(root)
        .set_permission(tool, true, false, token)
        .expect("grant");
}

#[test]
fn module_tools_are_listed_and_default_denied() {
    let (_temp, root, token) = setup();
    install_and_enable(
        &root,
        &token,
        vec![tool_decl(
            "echo",
            "module.echo",
            false,
            Some(2_000),
            vec!["value"],
        )],
    );
    let full = format!("module.{MODULE}.echo");
    let grants = registry(&root).list().expect("list");
    let module_grant = grants
        .iter()
        .find(|grant| grant.name == full)
        .expect("module tool listed");
    assert!(!module_grant.granted);
    assert!(module_grant.available);
    assert_eq!(module_grant.module_id.as_deref(), Some(MODULE));

    let executor = ToolExecutor::new(&root, CORE_A).expect("executor");
    let denied = executor
        .execute(&full, &json!({"value": "hi"}), None)
        .expect("execute");
    assert_eq!(denied.status, ToolStatus::Denied);

    grant(&root, &token, &full);
    let executed = executor
        .execute(&full, &json!({"value": "hi"}), None)
        .expect("execute");
    assert_eq!(executed.status, ToolStatus::Executed);
    assert_eq!(executed.module_id.as_deref(), Some(MODULE));
    assert_eq!(executed.output["arguments"]["value"], "hi");
    assert_eq!(executed.output["core_uuid"], CORE_A);
}

#[test]
fn disabled_module_tools_are_denied() {
    let (_temp, root, token) = setup();
    install_and_enable(
        &root,
        &token,
        vec![tool_decl("echo", "module.echo", false, None, vec!["value"])],
    );
    let full = format!("module.{MODULE}.echo");
    grant(&root, &token, &full);
    ModuleManager::new(StoragePaths::new(&root).expect("paths"))
        .disable(MODULE, &token)
        .expect("disable");
    let denied = ToolExecutor::new(&root, CORE_A)
        .expect("executor")
        .execute(&full, &json!({"value": "hi"}), None)
        .expect("execute");
    assert_eq!(denied.status, ToolStatus::Denied);
    assert!(denied.detail.expect("detail").contains("not enabled"));
    let listed = registry(&root).list().expect("list");
    let module_grant = listed
        .iter()
        .find(|grant| grant.name == full)
        .expect("listed");
    assert!(!module_grant.available);
}

#[test]
fn stop_and_revocation_block_module_tools() {
    let (_temp, root, token) = setup();
    install_and_enable(
        &root,
        &token,
        vec![tool_decl("echo", "module.echo", false, None, vec!["value"])],
    );
    let full = format!("module.{MODULE}.echo");
    grant(&root, &token, &full);
    safe(&root)
        .set_stop("owner", "maintenance", &token)
        .expect("stop");
    let blocked = ToolExecutor::new(&root, CORE_A)
        .expect("executor")
        .execute(&full, &json!({"value": "hi"}), None)
        .expect("execute");
    assert_eq!(blocked.status, ToolStatus::Denied);
    safe(&root).clear_stop("owner", &token).expect("clear");

    registry(&root)
        .set_permission(&full, false, false, &token)
        .expect("revoke");
    let revoked = ToolExecutor::new(&root, CORE_A)
        .expect("executor")
        .execute(&full, &json!({"value": "hi"}), None)
        .expect("execute");
    assert_eq!(revoked.status, ToolStatus::Denied);
}

#[test]
fn sensitive_module_tools_require_owner_approval() {
    let (_temp, root, token) = setup();
    install_and_enable(
        &root,
        &token,
        vec![tool_decl("echo", "module.echo", true, None, vec!["value"])],
    );
    let full = format!("module.{MODULE}.echo");
    grant(&root, &token, &full);
    let executor = ToolExecutor::new(&root, CORE_A).expect("executor");
    let approval = executor
        .execute(&full, &json!({"value": "hi"}), None)
        .expect("execute");
    assert_eq!(approval.status, ToolStatus::ApprovalRequired);
    let executed = executor
        .execute(&full, &json!({"value": "hi"}), Some(&token))
        .expect("execute");
    assert_eq!(executed.status, ToolStatus::Executed);
}

#[test]
fn module_tool_failure_timeout_and_argument_errors_are_contained() {
    let (_temp, root, token) = setup();
    install_and_enable(
        &root,
        &token,
        vec![
            tool_decl("echo", "module.echo", false, None, vec!["value"]),
            tool_decl("fail", "module.fail", false, None, vec![]),
            tool_decl("sleep", "module.sleep", false, Some(300), vec![]),
        ],
    );
    for name in ["echo", "fail", "sleep"] {
        grant(&root, &token, &format!("module.{MODULE}.{name}"));
    }
    let executor = ToolExecutor::new(&root, CORE_A).expect("executor");

    let missing = executor
        .execute(&format!("module.{MODULE}.echo"), &json!({}), None)
        .expect("execute");
    assert_eq!(missing.status, ToolStatus::Failed);
    assert!(missing
        .detail
        .expect("detail")
        .contains("missing required argument"));

    let failed = executor
        .execute(&format!("module.{MODULE}.fail"), &json!({}), None)
        .expect("execute");
    assert_eq!(failed.status, ToolStatus::Failed);
    assert!(failed.detail.expect("detail").contains("requested failure"));

    let timeout = executor
        .execute(&format!("module.{MODULE}.sleep"), &json!({}), None)
        .expect("execute");
    assert_eq!(timeout.status, ToolStatus::Failed);
    assert!(timeout.detail.expect("detail").contains("timed out"));

    // Platform remains available after module tool failures.
    assert!(
        PlatformService::start(&root)
            .expect("platform")
            .status()
            .expect("status")
            .core_count
            >= 2
    );
}

#[test]
fn module_tools_receive_only_the_calling_core_context() {
    let (_temp, root, token) = setup();
    install_and_enable(
        &root,
        &token,
        vec![tool_decl("echo", "module.echo", false, None, vec!["value"])],
    );
    let full = format!("module.{MODULE}.echo");
    grant(&root, &token, &full);
    let a = ToolExecutor::new(&root, CORE_A)
        .expect("a")
        .execute(&full, &json!({"value": "a"}), None)
        .expect("a");
    let b = ToolExecutor::new(&root, CORE_B)
        .expect("b")
        .execute(&full, &json!({"value": "b"}), None)
        .expect("b");
    assert_eq!(a.output["core_uuid"], CORE_A);
    assert_eq!(b.output["core_uuid"], CORE_B);
    assert!(!a.output.to_string().contains("TOP-SECRET"));
    assert!(safe(&root).audit_chain_valid().expect("audit"));
}

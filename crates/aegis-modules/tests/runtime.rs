use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use aegis_modules::{
    run_module_tool, run_module_tool_with_policy, ExecutionPolicy, ModuleAsset, ModuleEntrypoint,
    ModuleError, ModuleExitOutcome, ModuleIntegrity, ModuleManager, ModuleManifest,
    ModulePermission, ModuleResourceLimits, ModuleRuntime, ModuleRuntimeError, ModuleRuntimeState,
    ModuleSdkCompatibility, ModuleToolDeclaration, ModuleToolError, ModuleTrust,
    MAX_POLICY_RUNTIME_MS, MODULE_MANIFEST_SCHEMA, MODULE_SDK_NAME,
};
use aegis_platform::PlatformService;
use aegis_storage::StoragePaths;
use serde_json::json;
use sha2::{Digest, Sha256};

const MODULE_A: &str = "com.example.runtime";
const MODULE_B: &str = "com.example.second";
const MODULE_CRASH: &str = "com.example.crash";
const MODULE_STUBBORN: &str = "com.example.stubborn";
const MODULE_RECOVER: &str = "com.example.recover";
const MODULE_TREE: &str = "com.example.tree";
const MODULE_TOOL: &str = "com.example.tool";
const CALLER_CORE: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";

fn setup() -> (tempfile::TempDir, PathBuf, String) {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("aegis-root");
    let initialized = PlatformService::initialize(&root).expect("platform");
    (temp, root, initialized.owner_token)
}

fn manager(root: &Path) -> ModuleManager {
    ModuleManager::new(StoragePaths::new(root).expect("paths"))
}

fn runtime(root: &Path) -> ModuleRuntime {
    ModuleRuntime::new(StoragePaths::new(root).expect("paths"))
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

fn build_package(parent: &Path, module_id: &str, args: Vec<&str>) -> PathBuf {
    let package = parent.join(format!("package-{module_id}"));
    let bin_relative = format!("bin/aegis-example-module{}", std::env::consts::EXE_SUFFIX);
    let bin_path = package.join(&bin_relative);
    fs::create_dir_all(bin_path.parent().expect("bin parent")).expect("bin dir");
    fs::copy(env!("CARGO_BIN_EXE_aegis-example-module"), &bin_path).expect("copy example module");
    let bytes = fs::read(&bin_path).expect("read binary");

    let mut manifest = ModuleManifest {
        schema: MODULE_MANIFEST_SCHEMA.to_owned(),
        module_id: module_id.to_owned(),
        name: format!("Runtime Example {module_id}"),
        version: "0.1.0".to_owned(),
        publisher: "Aegis Tests".to_owned(),
        sdk: ModuleSdkCompatibility {
            sdk: MODULE_SDK_NAME.to_owned(),
            min_version: "1.0.0".to_owned(),
            max_version: None,
        },
        capabilities: vec!["process".to_owned()],
        permissions: vec![ModulePermission {
            name: "log.write".to_owned(),
            reason: "Write runtime diagnostics to the module log.".to_owned(),
        }],
        dependencies: Vec::new(),
        entrypoint: ModuleEntrypoint {
            kind: "process".to_owned(),
            runtime: "aegis-module-runtime-v1".to_owned(),
            path: bin_relative.clone(),
            symbol: None,
            args: args.into_iter().map(str::to_owned).collect(),
        },
        assets: vec![ModuleAsset {
            path: bin_relative,
            sha256: sha256_hex(&bytes),
            bytes: bytes.len() as u64,
        }],
        config_schema: json!({"type": "object", "properties": {}}),
        lifecycle: Vec::new(),
        tools: Vec::new(),
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
    manifest.seal().expect("seal manifest");
    fs::write(
        package.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).expect("manifest json"),
    )
    .expect("write manifest");
    package
}

fn build_tool_package(parent: &Path, module_id: &str) -> PathBuf {
    let package = build_package(parent, module_id, Vec::new());
    let manifest_path = package.join("manifest.json");
    let mut manifest: ModuleManifest =
        serde_json::from_slice(&fs::read(&manifest_path).expect("read packaged manifest"))
            .expect("parse packaged manifest");
    manifest.tools = vec![
        ModuleToolDeclaration {
            name: "echo".to_owned(),
            description: "Echo the tool input.".to_owned(),
            permission: "module.echo".to_owned(),
            sensitive: false,
            approval_required: false,
            timeout_ms: Some(2_000),
            input_schema: json!({"type": "object"}),
        },
        ModuleToolDeclaration {
            name: "sleep".to_owned(),
            description: "Sleep before returning, to exercise timeout policy.".to_owned(),
            permission: "module.sleep".to_owned(),
            sensitive: false,
            approval_required: false,
            timeout_ms: Some(10_000),
            input_schema: json!({"type": "object"}),
        },
    ];
    manifest.seal().expect("seal tool manifest");
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).expect("tool manifest json"),
    )
    .expect("write tool manifest");
    package
}

#[test]
fn package_install_materializes_and_runs() {
    let (temp, root, token) = setup();
    let package = build_package(temp.path(), MODULE_A, Vec::new());
    let manager = manager(&root);
    let record = manager
        .install_package(&package, None, &token)
        .expect("install package");
    assert_eq!(record.lifecycle_state.as_str(), "installed");

    let package_dir = root.join("modules").join(MODULE_A);
    assert!(package_dir.join("manifest.json").is_file());
    assert!(package_dir
        .join(format!(
            "bin/aegis-example-module{}",
            std::env::consts::EXE_SUFFIX
        ))
        .is_file());
    assert!(!root
        .join("modules")
        .join(".staging")
        .join(MODULE_A)
        .exists());

    manager.enable(MODULE_A, &token).expect("enable");
    let rt = runtime(&root);
    let started = rt.start(MODULE_A, &token).expect("start");
    assert_eq!(started.state, ModuleRuntimeState::Running);
    assert!(started.pid_alive);
    assert!(started.heartbeat_fresh);
    assert!(started.pid.is_some());

    let status = rt.status(MODULE_A).expect("status");
    assert!(status.is_running());
    assert_eq!(status.lifecycle_state, "enabled");

    let stopped = rt.stop(MODULE_A, &token).expect("stop");
    assert_eq!(stopped.state, ModuleRuntimeState::Stopped);
    assert!(!stopped.pid_alive);

    assert_eq!(
        manager.verify(MODULE_A).expect("verify").status,
        aegis_modules::ModuleVerificationStatus::Verified
    );
}

#[test]
fn tampered_package_is_rejected_without_side_effects() {
    let (temp, root, token) = setup();
    let package = build_package(temp.path(), MODULE_A, Vec::new());
    let bin = package.join(format!(
        "bin/aegis-example-module{}",
        std::env::consts::EXE_SUFFIX
    ));
    let mut bytes = fs::read(&bin).expect("read binary");
    bytes.push(0);
    fs::write(&bin, bytes).expect("tamper");

    let error = manager(&root)
        .install_package(&package, None, &token)
        .expect_err("tampered package must fail");
    assert!(matches!(error, ModuleError::Package(_)));
    assert!(!root.join("modules").join(MODULE_A).exists());
    assert!(manager(&root).list().expect("list").is_empty());
}

#[test]
fn start_requires_enable_and_owner_proof() {
    let (temp, root, token) = setup();
    let package = build_package(temp.path(), MODULE_A, Vec::new());
    let manager = manager(&root);
    manager
        .install_package(&package, None, &token)
        .expect("install");
    let rt = runtime(&root);

    let not_enabled = rt.start(MODULE_A, &token).expect_err("must require enable");
    assert!(matches!(not_enabled, ModuleRuntimeError::NotEnabled { .. }));

    manager.enable(MODULE_A, &token).expect("enable");
    let unauthorized = rt.start(MODULE_A, "not-the-owner").expect_err("must deny");
    assert!(matches!(unauthorized, ModuleRuntimeError::Unauthorized));
    assert_eq!(
        rt.status(MODULE_A).expect("status").state,
        ModuleRuntimeState::Stopped
    );
}

#[test]
fn installation_requires_owner_proof() {
    let (temp, root, _token) = setup();
    let package = build_package(temp.path(), MODULE_A, Vec::new());
    let error = manager(&root)
        .install_package(&package, None, "not-the-owner")
        .expect_err("must deny");
    assert!(matches!(error, ModuleError::Unauthorized));
    assert!(manager(&root).list().expect("list").is_empty());
}

#[test]
fn crash_is_reported_failed_and_platform_survives() {
    let (temp, root, token) = setup();
    let package = build_package(temp.path(), MODULE_CRASH, vec!["--fail-after-ms", "1500"]);
    let manager = manager(&root);
    manager
        .install_package(&package, None, &token)
        .expect("install");
    manager.enable(MODULE_CRASH, &token).expect("enable");
    let rt = runtime(&root);
    let started = rt.start(MODULE_CRASH, &token).expect("start");
    assert_eq!(started.state, ModuleRuntimeState::Running);

    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        let status = rt.status(MODULE_CRASH).expect("status");
        if status.state == ModuleRuntimeState::Failed {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "module did not crash (state={:?})",
            status.state
        );
        thread::sleep(Duration::from_millis(100));
    };
    assert!(!status.pid_alive);
    assert!(status.last_error.is_some());

    let platform = PlatformService::start(&root).expect("platform");
    assert!(platform.status().expect("status").zero_core);
    let stopped = rt.stop(MODULE_CRASH, &token).expect("idempotent stop");
    assert_eq!(stopped.state, ModuleRuntimeState::Stopped);
}

#[test]
fn forced_stop_kills_a_module_that_ignores_the_sentinel() {
    let (temp, root, token) = setup();
    let package = build_package(temp.path(), MODULE_STUBBORN, vec!["--ignore-stop"]);
    let manager = manager(&root);
    manager
        .install_package(&package, None, &token)
        .expect("install");
    manager.enable(MODULE_STUBBORN, &token).expect("enable");
    let rt = runtime(&root);
    let started = rt.start(MODULE_STUBBORN, &token).expect("start");
    assert!(started.pid_alive);

    let stopped = rt.stop(MODULE_STUBBORN, &token).expect("forced stop");
    assert_eq!(stopped.state, ModuleRuntimeState::Stopped);
    assert!(!stopped.pid_alive);
}

#[test]
fn two_modules_run_independently() {
    let (temp, root, token) = setup();
    let manager = manager(&root);
    manager
        .install_package(
            build_package(temp.path(), MODULE_A, Vec::new()),
            None,
            &token,
        )
        .expect("install a");
    manager
        .install_package(
            build_package(temp.path(), MODULE_B, Vec::new()),
            None,
            &token,
        )
        .expect("install b");
    manager.enable(MODULE_A, &token).expect("enable a");
    manager.enable(MODULE_B, &token).expect("enable b");

    let rt = runtime(&root);
    let a = rt.start(MODULE_A, &token).expect("start a");
    let b = rt.start(MODULE_B, &token).expect("start b");
    assert!(a.is_running());
    assert!(b.is_running());
    assert_ne!(a.pid, b.pid);

    let stopped_a = rt.stop(MODULE_A, &token).expect("stop a");
    assert_eq!(stopped_a.state, ModuleRuntimeState::Stopped);
    let status_b = rt.status(MODULE_B).expect("b status");
    assert!(status_b.is_running());
    assert!(status_b.heartbeat_fresh);

    rt.stop(MODULE_B, &token).expect("stop b");
    let all = rt.list().expect("list");
    assert_eq!(all.len(), 2);
}

#[test]
fn declarative_module_without_package_cannot_start() {
    let (temp, root, token) = setup();
    let package = build_package(temp.path(), MODULE_A, Vec::new());
    let manifest_json = fs::read_to_string(package.join("manifest.json")).expect("manifest");
    let manager = manager(&root);
    manager
        .install(&manifest_json, None, &token)
        .expect("declarative install");
    manager.enable(MODULE_A, &token).expect("enable");

    let error = runtime(&root)
        .start(MODULE_A, &token)
        .expect_err("no package directory");
    assert!(matches!(
        error,
        ModuleRuntimeError::EntrypointMissing { .. }
    ));
}

#[test]
fn status_of_unknown_module_is_rejected() {
    let (_temp, root, _token) = setup();
    let error = runtime(&root)
        .status("com.example.missing")
        .expect_err("unknown module");
    assert!(matches!(error, ModuleRuntimeError::Registry(_)));
}

#[test]
fn execution_summary_aggregates_real_running_and_crashed_modules() {
    let (temp, root, token) = setup();
    let manager = manager(&root);
    manager
        .install_package(
            build_package(temp.path(), MODULE_A, Vec::new()),
            None,
            &token,
        )
        .expect("install a");
    manager
        .install_package(
            build_package(temp.path(), MODULE_CRASH, vec!["--fail-after-ms", "4000"]),
            None,
            &token,
        )
        .expect("install crash");
    manager.enable(MODULE_A, &token).expect("enable a");
    manager.enable(MODULE_CRASH, &token).expect("enable crash");

    let rt = runtime(&root);
    let running = rt.start(MODULE_A, &token).expect("start a");
    assert!(running.is_running());
    let crashing = rt.start(MODULE_CRASH, &token).expect("start crash");
    assert!(crashing.is_running());

    let audit = rt.execution_summary().expect("audit while running");
    assert_eq!(audit.schema, "aegis.module.execution_audit.v1");
    assert_eq!(audit.module_count, 2);
    assert_eq!(audit.enabled_count, 2);
    assert_eq!(audit.running_count, 2);
    assert!(audit.running_modules.contains(&MODULE_A.to_owned()));
    assert!(audit.running_modules.contains(&MODULE_CRASH.to_owned()));
    assert_eq!(audit.heartbeat_fresh_count, 2);
    assert_eq!(audit.failed_count, 0);
    assert!(audit.last_error.is_none());
    assert!(audit.oldest_heartbeat_age_ms.is_some());
    assert!(audit.freshest_heartbeat_age_ms.is_some());

    // Wait deterministically for the crash instead of assuming a fixed delay;
    // parallel test scheduling can otherwise exceed a short crash budget.
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        let probe = rt
            .execution_summary()
            .expect("audit while waiting for crash");
        if probe.failed_count == 1 && probe.heartbeat_stale_count == 1 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "crash module did not fail within 20s (running={}, failed={})",
            probe.running_count,
            probe.failed_count
        );
        thread::sleep(Duration::from_millis(200));
    }
    let audit = rt.execution_summary().expect("audit after crash");
    assert_eq!(audit.module_count, 2);
    assert_eq!(audit.running_count, 1);
    assert!(audit.running_modules.contains(&MODULE_A.to_owned()));
    assert!(!audit.running_modules.contains(&MODULE_CRASH.to_owned()));
    assert_eq!(audit.failed_count, 1);
    assert!(audit.heartbeat_fresh_count >= 1);
    assert_eq!(audit.heartbeat_stale_count, 1);
    let last_error = audit.last_error.expect("crash error is reported");
    assert_eq!(last_error.module_id, MODULE_CRASH);
    assert_eq!(last_error.state, ModuleRuntimeState::Failed);
    assert!(!last_error.message.is_empty());

    rt.stop(MODULE_A, &token).expect("stop a");
    let audit = rt.execution_summary().expect("audit after stop");
    assert_eq!(audit.running_count, 0);
    assert_eq!(audit.heartbeat_missing_count, 1);
    assert_eq!(audit.heartbeat_stale_count, 1);
}

#[test]
fn execution_summary_is_empty_without_modules() {
    let (_temp, root, _token) = setup();
    let audit = runtime(&root).execution_summary().expect("empty audit");
    assert_eq!(audit.module_count, 0);
    assert_eq!(audit.running_count, 0);
    assert_eq!(audit.enabled_count, 0);
    assert!(audit.running_modules.is_empty());
    assert!(audit.last_error.is_none());
}

#[test]
fn disabled_or_uninstalled_module_cannot_run_a_tool() {
    let (temp, root, token) = setup();
    let package = build_tool_package(temp.path(), MODULE_TOOL);
    let manager = manager(&root);
    manager
        .install_package(&package, None, &token)
        .expect("install tool package");

    let arguments = json!({"value": "hello"});
    let not_enabled = run_module_tool(
        &root,
        MODULE_TOOL,
        "echo",
        CALLER_CORE,
        &arguments,
        Duration::from_secs(5),
    )
    .expect_err("installed but disabled tool must not run");
    assert!(matches!(
        not_enabled,
        ModuleToolError::NotEnabled { ref state, .. } if state == "installed"
    ));

    manager.enable(MODULE_TOOL, &token).expect("enable tool");
    let output = run_module_tool(
        &root,
        MODULE_TOOL,
        "echo",
        CALLER_CORE,
        &arguments,
        Duration::from_secs(5),
    )
    .expect("enabled tool runs");
    assert_eq!(output["module_id"], MODULE_TOOL);
    assert_eq!(output["tool"], "echo");
    assert_eq!(output["core_uuid"], CALLER_CORE);
    assert_eq!(output["arguments"]["value"], "hello");

    manager.disable(MODULE_TOOL, &token).expect("disable tool");
    let disabled = run_module_tool(
        &root,
        MODULE_TOOL,
        "echo",
        CALLER_CORE,
        &arguments,
        Duration::from_secs(5),
    )
    .expect_err("disabled tool must not run");
    assert!(matches!(
        disabled,
        ModuleToolError::NotEnabled { ref state, .. } if state == "disabled"
    ));

    manager.enable(MODULE_TOOL, &token).expect("re-enable");
    manager.uninstall(MODULE_TOOL, &token).expect("uninstall");
    let revoked = run_module_tool(
        &root,
        MODULE_TOOL,
        "echo",
        CALLER_CORE,
        &arguments,
        Duration::from_secs(5),
    )
    .expect_err("uninstalled tool must not run");
    assert!(matches!(
        revoked,
        ModuleToolError::NotEnabled { ref state, .. } if state == "uninstalled"
    ));
}

fn restricted_policy(allow_restricted: bool) -> ExecutionPolicy {
    let mut policy = ExecutionPolicy::for_trust(ModuleTrust::Restricted);
    policy.max_runtime_ms = MAX_POLICY_RUNTIME_MS;
    policy.allow_restricted = allow_restricted;
    policy
}

#[test]
fn restricted_policy_denies_runtime_and_tools_until_explicitly_allowed() {
    let (temp, root, token) = setup();
    let package = build_tool_package(temp.path(), MODULE_TOOL);
    let manager = manager(&root);
    manager
        .install_package(&package, None, &token)
        .expect("install tool package");
    manager.enable(MODULE_TOOL, &token).expect("enable tool");
    let rt = runtime(&root);
    let arguments = json!({"value": "restricted"});

    let denied_start = rt
        .start_with_policy(MODULE_TOOL, &token, &restricted_policy(false))
        .expect_err("restricted module must not start without opt-in");
    assert!(matches!(
        denied_start,
        ModuleRuntimeError::RestrictedDenied { ref module_id } if module_id == MODULE_TOOL
    ));

    let denied_tool = run_module_tool_with_policy(
        &root,
        MODULE_TOOL,
        "echo",
        CALLER_CORE,
        &arguments,
        Duration::from_secs(5),
        &restricted_policy(false),
    )
    .expect_err("restricted tool must not run without opt-in");
    assert!(matches!(
        denied_tool,
        ModuleToolError::RestrictedDenied { ref module_id } if module_id == MODULE_TOOL
    ));

    let started = rt
        .start_with_policy(MODULE_TOOL, &token, &restricted_policy(true))
        .expect("explicitly allowed restricted module starts");
    assert!(started.is_running());
    rt.stop(MODULE_TOOL, &token).expect("stop allowed module");

    let output = run_module_tool_with_policy(
        &root,
        MODULE_TOOL,
        "echo",
        CALLER_CORE,
        &arguments,
        Duration::from_secs(5),
        &restricted_policy(true),
    )
    .expect("explicitly allowed restricted tool runs");
    assert_eq!(output["arguments"]["value"], "restricted");
}

#[test]
fn execution_policy_limits_are_enforced_and_clamped() {
    let (temp, root, token) = setup();
    let package = build_tool_package(temp.path(), MODULE_TOOL);
    let manager = manager(&root);
    manager
        .install_package(&package, None, &token)
        .expect("install tool package");
    manager.enable(MODULE_TOOL, &token).expect("enable tool");
    let arguments = json!({"value": "hello"});

    let tiny_input = ExecutionPolicy {
        max_input_bytes: 8,
        ..ExecutionPolicy::default()
    };
    let error = run_module_tool_with_policy(
        &root,
        MODULE_TOOL,
        "echo",
        CALLER_CORE,
        &arguments,
        Duration::from_secs(5),
        &tiny_input,
    )
    .expect_err("input above policy limit must be rejected");
    assert!(matches!(error, ModuleToolError::InputTooLarge));

    let tiny_output = ExecutionPolicy {
        max_output_bytes: 8,
        ..ExecutionPolicy::default()
    };
    let error = run_module_tool_with_policy(
        &root,
        MODULE_TOOL,
        "echo",
        CALLER_CORE,
        &arguments,
        Duration::from_secs(5),
        &tiny_output,
    )
    .expect_err("output above policy limit must be rejected");
    assert!(matches!(error, ModuleToolError::OutputTooLarge));

    let short_runtime = ExecutionPolicy {
        max_runtime_ms: 200,
        ..ExecutionPolicy::default()
    };
    let error = run_module_tool_with_policy(
        &root,
        MODULE_TOOL,
        "sleep",
        CALLER_CORE,
        &arguments,
        Duration::from_secs(5),
        &short_runtime,
    )
    .expect_err("runtime must be clamped to the policy cap");
    assert!(matches!(
        error,
        ModuleToolError::Timeout { timeout_ms, .. } if timeout_ms == 200
    ));

    let one_tool = ExecutionPolicy {
        max_tool_count: 1,
        ..ExecutionPolicy::default()
    };
    let error = run_module_tool_with_policy(
        &root,
        MODULE_TOOL,
        "echo",
        CALLER_CORE,
        &arguments,
        Duration::from_secs(5),
        &one_tool,
    )
    .expect_err("declared tool count above policy must be rejected");
    assert!(matches!(
        error,
        ModuleToolError::ToolCountExceeded {
            declared: 2,
            allowed: 1,
            ..
        }
    ));

    let invalid = ExecutionPolicy {
        max_input_bytes: aegis_modules::MAX_TOOL_INPUT_BYTES + 1,
        ..ExecutionPolicy::default()
    };
    let error = run_module_tool_with_policy(
        &root,
        MODULE_TOOL,
        "echo",
        CALLER_CORE,
        &arguments,
        Duration::from_secs(5),
        &invalid,
    )
    .expect_err("invalid policy must be rejected before execution");
    assert!(matches!(error, ModuleToolError::Policy(_)));

    let output = run_module_tool_with_policy(
        &root,
        MODULE_TOOL,
        "echo",
        CALLER_CORE,
        &arguments,
        Duration::from_secs(5),
        &ExecutionPolicy::default(),
    )
    .expect("allowed execution still works");
    assert_eq!(output["arguments"]["value"], "hello");
}

#[test]
fn execution_audit_includes_policy_denial_and_exit_outcome() {
    let (temp, root, token) = setup();
    let package = build_tool_package(temp.path(), MODULE_TOOL);
    let manager = manager(&root);
    manager
        .install_package(&package, None, &token)
        .expect("install tool package");
    manager.enable(MODULE_TOOL, &token).expect("enable tool");
    let rt = runtime(&root);

    let audit = rt.execution_summary().expect("default audit");
    assert_eq!(audit.schema, "aegis.module.execution_audit.v1");
    assert_eq!(audit.policy, ExecutionPolicy::default());
    assert_eq!(audit.module_count, 1);
    assert_eq!(audit.restricted_count, 0);
    assert_eq!(audit.denied_count, 0);
    let entry = audit.entries.first().expect("entry");
    assert_eq!(entry.module_id, MODULE_TOOL);
    assert_eq!(entry.trust, ModuleTrust::TrustedLocal);
    assert!(entry.denied_reason.is_none());
    assert_eq!(entry.exit_outcome, ModuleExitOutcome::NotStarted);
    assert_eq!(entry.policy, ExecutionPolicy::default());

    let restricted = rt
        .execution_summary_with_policy(&restricted_policy(false))
        .expect("restricted audit");
    assert_eq!(restricted.restricted_count, 1);
    assert_eq!(restricted.denied_count, 1);
    assert_eq!(restricted.denied_modules, vec![MODULE_TOOL.to_owned()]);
    let restricted_entry = restricted.entries.first().expect("restricted entry");
    assert_eq!(restricted_entry.trust, ModuleTrust::Restricted);
    let reason = restricted_entry
        .denied_reason
        .as_deref()
        .expect("restricted denial reason");
    assert!(reason.contains("restricted"));
    assert!(reason.contains("allow_restricted"));

    let allowed = rt
        .execution_summary_with_policy(&restricted_policy(true))
        .expect("allowed restricted audit");
    assert_eq!(allowed.denied_count, 0);

    let started = rt.start(MODULE_TOOL, &token).expect("start tool module");
    assert!(started.is_running());
    let running = rt.execution_summary().expect("running audit");
    let entry = running.entries.first().expect("running entry");
    assert_eq!(entry.exit_outcome, ModuleExitOutcome::Running);
    assert!(entry.exit_outcome.is_running());
    assert!(entry.denied_reason.is_none());

    rt.stop(MODULE_TOOL, &token).expect("stop tool module");
    let stopped = rt.execution_summary().expect("stopped audit");
    let entry = stopped.entries.first().expect("stopped entry");
    assert_eq!(entry.exit_outcome, ModuleExitOutcome::Stopped);
    assert!(entry.last_error.is_none());

    manager.disable(MODULE_TOOL, &token).expect("disable tool");
    let disabled = rt.execution_summary().expect("disabled audit");
    assert_eq!(disabled.denied_count, 1);
    let reason = disabled.entries[0]
        .denied_reason
        .as_deref()
        .expect("disabled denial reason");
    assert!(reason.contains("disabled"));
}

#[test]
fn runtime_policy_stops_a_process_that_outlives_its_budget() {
    let (temp, root, token) = setup();
    let package = build_package(temp.path(), MODULE_A, Vec::new());
    let manager = manager(&root);
    manager
        .install_package(&package, None, &token)
        .expect("install package");
    manager.enable(MODULE_A, &token).expect("enable");
    let rt = runtime(&root);

    let policy = ExecutionPolicy {
        max_runtime_ms: 300,
        ..ExecutionPolicy::default()
    };
    let started = rt
        .start_with_policy(MODULE_A, &token, &policy)
        .expect("start under policy");
    assert!(started.is_running());

    thread::sleep(Duration::from_millis(1_400));
    let status = rt.status(MODULE_A).expect("status after budget");
    assert_eq!(status.state, ModuleRuntimeState::Failed);
    assert!(!status.pid_alive);
    let message = status.last_error.expect("policy stop reason");
    assert!(message.contains("execution policy runtime limit"));

    let stopped = rt.stop(MODULE_A, &token).expect("idempotent stop");
    assert_eq!(stopped.state, ModuleRuntimeState::Stopped);

    let invalid = ExecutionPolicy {
        max_runtime_ms: 0,
        ..ExecutionPolicy::default()
    };
    let error = rt
        .start_with_policy(MODULE_A, &token, &invalid)
        .expect_err("invalid policy must be rejected");
    assert!(matches!(error, ModuleRuntimeError::Policy(_)));
}

#[test]
fn crashed_module_can_be_recovered_after_explicit_owner_action() {
    let (temp, root, token) = setup();
    let package = build_package(temp.path(), MODULE_RECOVER, vec!["--fail-once-ms", "1000"]);
    let manager = manager(&root);
    manager
        .install_package(&package, None, &token)
        .expect("install recoverable package");
    let rt = runtime(&root);

    // A module must be explicitly enabled before recovery is possible.
    let not_enabled = rt
        .recover(MODULE_RECOVER, &token)
        .expect_err("recovery requires an enabled module");
    assert!(matches!(
        not_enabled,
        ModuleRuntimeError::NotEnabled { ref state, .. } if state == "installed"
    ));

    manager.enable(MODULE_RECOVER, &token).expect("enable");
    let started = rt.start(MODULE_RECOVER, &token).expect("start");
    assert!(started.is_running());

    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let status = rt.status(MODULE_RECOVER).expect("status while waiting");
        if status.state == ModuleRuntimeState::Failed {
            assert!(!status.pid_alive);
            assert!(status.last_error.is_some());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "module did not crash (state={:?})",
            status.state
        );
        thread::sleep(Duration::from_millis(100));
    }

    let denied = rt
        .recover(MODULE_RECOVER, "not-the-owner")
        .expect_err("recovery requires owner proof");
    assert!(matches!(denied, ModuleRuntimeError::Unauthorized));

    let recovered = rt.recover(MODULE_RECOVER, &token).expect("recover");
    assert!(recovered.is_running());
    let status = rt.status(MODULE_RECOVER).expect("status after recovery");
    assert_eq!(status.state, ModuleRuntimeState::Running);
    assert_eq!(status.lifecycle_state, "enabled");
    assert!(status.pid_alive);

    let already = rt
        .recover(MODULE_RECOVER, &token)
        .expect_err("a running module cannot be recovered");
    assert!(matches!(
        already,
        ModuleRuntimeError::AlreadyRunning(ref id) if id == MODULE_RECOVER
    ));

    let stopped = rt.stop(MODULE_RECOVER, &token).expect("stop recovered");
    assert_eq!(stopped.state, ModuleRuntimeState::Stopped);
    assert!(!stopped.pid_alive);
}

#[cfg(windows)]
#[test]
fn forced_stop_terminates_the_module_process_tree() {
    struct TreeGuard(Vec<u32>);

    impl Drop for TreeGuard {
        fn drop(&mut self) {
            for pid in &self.0 {
                let _ = std::process::Command::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/T", "/F"])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
            }
        }
    }

    let (temp, root, token) = setup();
    let package = build_package(
        temp.path(),
        MODULE_TREE,
        vec!["--ignore-stop", "--spawn-child"],
    );
    let manager = manager(&root);
    manager
        .install_package(&package, None, &token)
        .expect("install tree package");
    manager.enable(MODULE_TREE, &token).expect("enable");
    let rt = runtime(&root);
    let started = rt.start(MODULE_TREE, &token).expect("start tree module");
    let parent = started.pid.expect("parent pid");
    let mut guard = TreeGuard(vec![parent]);

    let child_file = root
        .join("run")
        .join("modules")
        .join(MODULE_TREE)
        .join(format!("{MODULE_TREE}.child"));
    let deadline = Instant::now() + Duration::from_secs(10);
    let child = loop {
        if let Ok(text) = fs::read_to_string(&child_file) {
            if let Ok(pid) = text.trim().parse::<u32>() {
                break pid;
            }
        }
        assert!(
            Instant::now() < deadline,
            "module never reported a child pid"
        );
        thread::sleep(Duration::from_millis(50));
    };
    guard.0.push(child);
    assert!(aegis_modules::runtime::process_is_alive(parent));
    assert!(aegis_modules::runtime::process_is_alive(child));

    let stopped = rt.stop(MODULE_TREE, &token).expect("forced tree stop");
    assert_eq!(stopped.state, ModuleRuntimeState::Stopped);
    assert!(!stopped.pid_alive);

    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline
        && (aegis_modules::runtime::process_is_alive(parent)
            || aegis_modules::runtime::process_is_alive(child))
    {
        thread::sleep(Duration::from_millis(50));
    }
    assert!(
        !aegis_modules::runtime::process_is_alive(parent),
        "module process survived forced stop"
    );
    assert!(
        !aegis_modules::runtime::process_is_alive(child),
        "module child process survived the process-tree termination"
    );
    drop(guard);
}

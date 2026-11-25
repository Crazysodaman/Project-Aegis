use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use aegis_modules::{
    ModuleAsset, ModuleEntrypoint, ModuleError, ModuleIntegrity, ModuleManager, ModuleManifest,
    ModulePermission, ModuleResourceLimits, ModuleRuntime, ModuleRuntimeError, ModuleRuntimeState,
    ModuleSdkCompatibility, MODULE_MANIFEST_SCHEMA, MODULE_SDK_NAME,
};
use aegis_platform::PlatformService;
use aegis_storage::StoragePaths;
use serde_json::json;
use sha2::{Digest, Sha256};

const MODULE_A: &str = "com.example.runtime";
const MODULE_B: &str = "com.example.second";
const MODULE_CRASH: &str = "com.example.crash";
const MODULE_STUBBORN: &str = "com.example.stubborn";

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
    let package = build_package(temp.path(), MODULE_CRASH, vec!["--fail-after-ms", "300"]);
    let manager = manager(&root);
    manager
        .install_package(&package, None, &token)
        .expect("install");
    manager.enable(MODULE_CRASH, &token).expect("enable");
    let rt = runtime(&root);
    let started = rt.start(MODULE_CRASH, &token).expect("start");
    assert_eq!(started.state, ModuleRuntimeState::Running);

    thread::sleep(Duration::from_millis(1_400));
    let status = rt.status(MODULE_CRASH).expect("status");
    assert_eq!(status.state, ModuleRuntimeState::Failed);
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

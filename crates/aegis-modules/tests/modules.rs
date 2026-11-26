use aegis_modules::{
    ManifestError, ModuleDependency, ModuleError, ModuleLifecycleState, ModuleManifest,
    ModuleRegistry, ModuleVerificationStatus,
};
use aegis_storage::{
    open_initialized, open_read_only, StoragePaths, PLATFORM_APPLICATION_ID, SAFE_APPLICATION_ID,
};

const EXAMPLE_MANIFEST: &str = include_str!("../examples/hello.manifest.json");
const CORE_APPLICATION_ID: i64 = 0x4145_4743; // "AEGC" marker owned by the Core agent.

fn temp_paths() -> (tempfile::TempDir, StoragePaths) {
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = StoragePaths::new(temp.path()).expect("storage paths");
    (temp, paths)
}

fn registry(paths: &StoragePaths) -> ModuleRegistry {
    let registry = ModuleRegistry::new(paths.clone());
    registry.initialize().expect("initialize modules");
    registry
}

fn example_manifest() -> ModuleManifest {
    serde_json::from_str(EXAMPLE_MANIFEST).expect("parse example manifest")
}

fn seal(manifest: &mut ModuleManifest) -> String {
    manifest.seal().expect("seal manifest");
    serde_json::to_string(manifest).expect("serialize manifest")
}

fn manifest_for(module_id: &str) -> (ModuleManifest, String) {
    let mut manifest = example_manifest();
    manifest.module_id = module_id.to_owned();
    let json = seal(&mut manifest);
    (manifest, json)
}

fn write_module_data(paths: &StoragePaths, module_id: &str) -> std::path::PathBuf {
    let data_path = paths
        .root()
        .join("modules")
        .join(module_id)
        .join("data.bin");
    std::fs::create_dir_all(data_path.parent().expect("data parent")).expect("module data dir");
    std::fs::write(&data_path, b"module-owned-data").expect("module data");
    data_path
}

#[test]
fn example_manifest_is_valid_and_installs_unchanged() {
    let (_temp, paths) = temp_paths();
    let registry = registry(&paths);
    let manifest = example_manifest();
    manifest.validate().expect("example manifest is valid");

    let record = registry
        .install(EXAMPLE_MANIFEST, Some(&manifest.integrity.digest))
        .expect("install example manifest as published");
    assert_eq!(record.module_id, "com.example.hello");
    assert_eq!(
        registry
            .verify("com.example.hello")
            .expect("verify example")
            .status,
        ModuleVerificationStatus::Verified
    );
}

#[test]
fn valid_manifest_installs_verifies_enables_and_disables() {
    let (_temp, paths) = temp_paths();
    let registry = registry(&paths);
    let (manifest, json) = manifest_for("com.example.hello");

    let record = registry
        .install(&json, Some(&manifest.integrity.digest))
        .expect("install");
    assert_eq!(record.module_id, "com.example.hello");
    assert_eq!(record.schema, "aegis.module.manifest.v1");
    assert_eq!(record.version, "0.1.0");
    assert_eq!(record.lifecycle_state, ModuleLifecycleState::Installed);
    assert_eq!(
        record.capabilities,
        vec!["log.write".to_owned(), "ui.notifications".to_owned()]
    );
    assert_eq!(
        record.permissions,
        vec!["log.write".to_owned(), "ui.notifications".to_owned()]
    );
    assert_eq!(record.manifest_sha256, manifest.integrity.digest);

    let verification = registry
        .verify("com.example.hello")
        .expect("verify installed module");
    assert_eq!(verification.status, ModuleVerificationStatus::Verified);
    assert_eq!(
        verification.expected_sha256.as_deref(),
        Some(manifest.integrity.digest.as_str())
    );

    let enabled = registry.enable("com.example.hello").expect("enable");
    assert_eq!(enabled.lifecycle_state, ModuleLifecycleState::Enabled);

    let disabled = registry.disable("com.example.hello").expect("disable");
    assert_eq!(disabled.lifecycle_state, ModuleLifecycleState::Disabled);

    let events = registry
        .lifecycle_events("com.example.hello")
        .expect("events");
    let actions: Vec<&str> = events.iter().map(|event| event.action.as_str()).collect();
    assert_eq!(actions, vec!["install", "enable", "disable"]);
}

#[test]
fn duplicate_module_ids_are_rejected() {
    let (_temp, paths) = temp_paths();
    let registry = registry(&paths);
    let (_manifest, json) = manifest_for("com.example.hello");

    registry.install(&json, None).expect("first install");
    let error = registry
        .install(&json, None)
        .expect_err("duplicate install must fail");
    assert!(matches!(
        error,
        ModuleError::DuplicateModuleId(ref id) if id == "com.example.hello"
    ));
    assert_eq!(registry.list().expect("list").len(), 1);
}

#[test]
fn invalid_schema_digest_sdk_and_reserved_ids_are_rejected() {
    let (_temp, paths) = temp_paths();
    let registry = registry(&paths);

    let mut wrong_schema = example_manifest();
    wrong_schema.schema = "aegis.module.manifest.v2".to_owned();
    let json = seal(&mut wrong_schema);
    let error = registry.install(&json, None).expect_err("wrong schema");
    assert!(matches!(
        error,
        ModuleError::Manifest(ManifestError::UnsupportedSchema(_))
    ));

    let mut stale_digest = example_manifest();
    stale_digest.integrity.digest = "f".repeat(64);
    let json = serde_json::to_string(&stale_digest).expect("serialize stale digest");
    let error = registry.install(&json, None).expect_err("stale digest");
    assert!(matches!(
        error,
        ModuleError::Manifest(ManifestError::IntegrityMismatch)
    ));

    let mut incompatible_sdk = example_manifest();
    incompatible_sdk.sdk.min_version = "2.0.0".to_owned();
    incompatible_sdk.sdk.max_version = Some("2.9.9".to_owned());
    let json = seal(&mut incompatible_sdk);
    let error = registry.install(&json, None).expect_err("incompatible SDK");
    assert!(matches!(
        error,
        ModuleError::Manifest(ManifestError::InvalidSdk(_))
    ));

    let mut reserved = example_manifest();
    reserved.module_id = "aegis.platform".to_owned();
    let json = seal(&mut reserved);
    let error = registry.install(&json, None).expect_err("reserved ID");
    assert!(matches!(
        error,
        ModuleError::Manifest(ManifestError::ReservedModuleId(_))
    ));

    let (manifest, json) = manifest_for("com.example.hello");
    let error = registry
        .install(&json, Some("not-a-digest"))
        .expect_err("invalid expected digest");
    assert!(matches!(error, ModuleError::InvalidExpectedDigest));

    let expected = "f".repeat(64);
    let error = registry
        .install(&json, Some(&expected))
        .expect_err("expected digest mismatch");
    assert!(matches!(
        error,
        ModuleError::ExpectedDigestMismatch { ref expected, ref actual }
            if expected == &"f".repeat(64) && actual == &manifest.integrity.digest
    ));

    assert!(registry.list().expect("list").is_empty());
}

#[test]
fn missing_dependencies_are_rejected() {
    let (_temp, paths) = temp_paths();
    let registry = registry(&paths);

    let mut consumer = example_manifest();
    consumer.module_id = "com.example.consumer".to_owned();
    consumer.dependencies = vec![ModuleDependency {
        module_id: "com.example.dependency".to_owned(),
        version_requirement: Some("^0.1.0".to_owned()),
        min_version: None,
        max_version: None,
    }];
    let json = seal(&mut consumer);
    let error = registry
        .install(&json, None)
        .expect_err("missing dependency");
    assert!(matches!(
        error,
        ModuleError::UnknownDependency(ref id) if id == "com.example.dependency"
    ));

    let (_dependency, dependency_json) = manifest_for("com.example.dependency");
    registry
        .install(&dependency_json, None)
        .expect("install dependency");
    registry
        .install(&json, None)
        .expect("install consumer once dependency exists");

    registry
        .uninstall("com.example.dependency")
        .expect("uninstall dependency");

    let mut second_consumer = example_manifest();
    second_consumer.module_id = "com.example.consumer2".to_owned();
    second_consumer.dependencies = vec![ModuleDependency {
        module_id: "com.example.dependency".to_owned(),
        version_requirement: None,
        min_version: None,
        max_version: None,
    }];
    let json = seal(&mut second_consumer);
    let error = registry
        .install(&json, None)
        .expect_err("unavailable dependency");
    assert!(matches!(
        error,
        ModuleError::DependencyUnavailable { ref module_id, .. }
            if module_id == "com.example.dependency"
    ));
}

#[test]
fn enable_and_disable_follow_lifecycle_rules() {
    let (_temp, paths) = temp_paths();
    let registry = registry(&paths);
    let (_manifest, json) = manifest_for("com.example.hello");
    registry.install(&json, None).expect("install");

    registry.enable("com.example.hello").expect("enable");
    let error = registry
        .enable("com.example.hello")
        .expect_err("cannot enable twice");
    assert!(matches!(
        error,
        ModuleError::IllegalTransition { ref from, ref to }
            if from == "enabled" && to == "enabled"
    ));

    registry.disable("com.example.hello").expect("disable");
    let error = registry
        .disable("com.example.hello")
        .expect_err("cannot disable twice");
    assert!(matches!(
        error,
        ModuleError::IllegalTransition { ref from, ref to }
            if from == "disabled" && to == "disabled"
    ));

    registry.uninstall("com.example.hello").expect("uninstall");
    let error = registry
        .enable("com.example.hello")
        .expect_err("cannot enable uninstalled");
    assert!(matches!(
        error,
        ModuleError::IllegalTransition { ref from, ref to }
            if from == "uninstalled" && to == "enabled"
    ));

    let record = registry
        .inspect("com.example.hello")
        .expect("inspect")
        .record;
    assert_eq!(record.lifecycle_state, ModuleLifecycleState::Uninstalled);
}

#[test]
fn enable_requires_declared_permissions() {
    let (_temp, paths) = temp_paths();
    let registry = registry(&paths);
    let mut manifest = example_manifest();
    manifest.module_id = "com.example.noperms".to_owned();
    manifest.permissions.clear();
    let json = seal(&mut manifest);

    registry
        .install(&json, None)
        .expect("install without permissions");
    let error = registry
        .enable("com.example.noperms")
        .expect_err("enable without permissions");
    assert!(matches!(
        error,
        ModuleError::NoDeclaredPermissions { ref module_id }
            if module_id == "com.example.noperms"
    ));
}

#[test]
fn list_and_inspect_expose_registry_state() {
    let (_temp, paths) = temp_paths();
    let registry = registry(&paths);
    let (_alpha, alpha_json) = manifest_for("com.example.alpha");
    let (_beta, beta_json) = manifest_for("com.example.beta");
    registry.install(&beta_json, None).expect("install beta");
    registry.install(&alpha_json, None).expect("install alpha");

    let listed = registry.list().expect("list");
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].module_id, "com.example.alpha");
    assert_eq!(listed[1].module_id, "com.example.beta");

    let inspection = registry.inspect("com.example.beta").expect("inspect");
    assert_eq!(inspection.record.module_id, "com.example.beta");
    assert_eq!(inspection.manifest.module_id, "com.example.beta");
    assert_eq!(inspection.events.len(), 1);
    assert_eq!(inspection.events[0].action, "install");
    assert_eq!(inspection.events[0].from_state, None);
    assert_eq!(
        inspection.events[0].to_state,
        ModuleLifecycleState::Installed
    );
}

#[test]
fn verify_detects_manifest_tampering() {
    let (_temp, paths) = temp_paths();
    let registry = registry(&paths);
    let (_manifest, json) = manifest_for("com.example.hello");
    registry.install(&json, None).expect("install");

    {
        let connection = open_initialized(&paths.platform_db(), PLATFORM_APPLICATION_ID, "modules")
            .expect("platform db");
        let stored: String = connection
            .query_row(
                "SELECT manifest_json FROM aegis_modules WHERE module_id = ?1",
                ["com.example.hello"],
                |row| row.get(0),
            )
            .expect("stored manifest");
        let mut value: serde_json::Value =
            serde_json::from_str(&stored).expect("parse stored manifest");
        value["name"] = serde_json::Value::String("Tampered Module".to_owned());
        let tampered = serde_json::to_string(&value).expect("serialize tampered manifest");
        connection
            .execute(
                "UPDATE aegis_modules SET manifest_json = ?1 WHERE module_id = ?2",
                rusqlite::params![tampered, "com.example.hello"],
            )
            .expect("tamper manifest");
    }

    let report = registry
        .verify("com.example.hello")
        .expect("verify tampered module");
    assert_eq!(report.status, ModuleVerificationStatus::DigestMismatch);
    assert_ne!(report.expected_sha256, report.actual_sha256);

    {
        let connection = open_initialized(&paths.platform_db(), PLATFORM_APPLICATION_ID, "modules")
            .expect("platform db");
        connection
            .execute(
                "UPDATE aegis_modules SET manifest_json = 'not-json' WHERE module_id = ?1",
                ["com.example.hello"],
            )
            .expect("corrupt manifest");
    }
    let report = registry
        .verify("com.example.hello")
        .expect("verify corrupt manifest");
    assert_eq!(report.status, ModuleVerificationStatus::InvalidManifest);
}

#[test]
fn disable_and_uninstall_preserve_module_data() {
    let (_temp, paths) = temp_paths();
    let registry = registry(&paths);
    let (_manifest, json) = manifest_for("com.example.hello");
    registry.install(&json, None).expect("install");
    let data_path = write_module_data(&paths, "com.example.hello");

    registry.enable("com.example.hello").expect("enable");
    registry.disable("com.example.hello").expect("disable");
    assert_eq!(
        std::fs::read(&data_path).expect("data after disable"),
        b"module-owned-data"
    );

    registry.uninstall("com.example.hello").expect("uninstall");
    assert_eq!(
        std::fs::read(&data_path).expect("data after uninstall"),
        b"module-owned-data"
    );

    let inspection = registry.inspect("com.example.hello").expect("inspect");
    assert_eq!(
        inspection.record.lifecycle_state,
        ModuleLifecycleState::Uninstalled
    );
    assert_eq!(inspection.manifest.module_id, "com.example.hello");
    assert_eq!(
        registry
            .verify("com.example.hello")
            .expect("verify preserved manifest")
            .status,
        ModuleVerificationStatus::Verified
    );
    assert_eq!(registry.list().expect("list").len(), 1);
}

#[test]
fn platform_safe_and_core_databases_remain_separate() {
    let (_temp, paths) = temp_paths();
    let registry = registry(&paths);
    let (_manifest, json) = manifest_for("com.example.hello");
    registry.install(&json, None).expect("install");

    let core_db = paths
        .root()
        .join("cores")
        .join("a9a0a604-a4e7-4a88-a985-3f071e2c8e7d")
        .join("core.db");
    std::fs::create_dir_all(core_db.parent().expect("core parent")).expect("core dir");
    open_initialized(&paths.safe_db(), SAFE_APPLICATION_ID, "safe").expect("safe db");
    open_initialized(&core_db, CORE_APPLICATION_ID, "core").expect("core db");

    assert_ne!(paths.platform_db(), paths.safe_db());
    assert_ne!(paths.platform_db(), core_db);
    assert_ne!(paths.safe_db(), core_db);

    let platform =
        open_read_only(&paths.platform_db(), PLATFORM_APPLICATION_ID).expect("platform marker");
    let safe = open_read_only(&paths.safe_db(), SAFE_APPLICATION_ID).expect("safe marker");
    let core = open_read_only(&core_db, CORE_APPLICATION_ID).expect("core marker");

    for (connection, expected) in [
        (&platform, PLATFORM_APPLICATION_ID),
        (&safe, SAFE_APPLICATION_ID),
        (&core, CORE_APPLICATION_ID),
    ] {
        let actual: i64 = connection
            .query_row("PRAGMA application_id", [], |row| row.get(0))
            .expect("read marker");
        assert_eq!(actual, expected);
    }

    for table in ["aegis_modules", "aegis_module_events"] {
        let platform_count: i64 = platform
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |row| row.get(0),
            )
            .expect("platform table count");
        assert_eq!(platform_count, 1);

        for connection in [&safe, &core] {
            let count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [table],
                    |row| row.get(0),
                )
                .expect("foreign table count");
            assert_eq!(count, 0);
        }
    }
}

#[test]
fn unknown_modules_are_rejected() {
    let (_temp, paths) = temp_paths();
    let registry = registry(&paths);
    let unknown = "com.example.unknown";

    for error in [
        registry.enable(unknown).expect_err("enable unknown"),
        registry.disable(unknown).expect_err("disable unknown"),
        registry.uninstall(unknown).expect_err("uninstall unknown"),
        registry.inspect(unknown).expect_err("inspect unknown"),
        registry.verify(unknown).expect_err("verify unknown"),
        registry
            .lifecycle_events(unknown)
            .expect_err("events unknown"),
    ] {
        assert!(matches!(
            error,
            ModuleError::ModuleNotFound(ref id) if id == unknown
        ));
    }

    let error = registry
        .enable("Not A Valid Module ID")
        .expect_err("invalid module ID");
    assert!(matches!(
        error,
        ModuleError::Manifest(ManifestError::InvalidModuleId(_))
    ));
}

#[test]
fn version_requirements_are_enforced() {
    assert!(aegis_modules::version_satisfies("^0.1.0", "0.1.0"));
    assert!(aegis_modules::version_satisfies("^0.1.0", "0.1.9"));
    assert!(!aegis_modules::version_satisfies("^0.1.0", "0.2.0"));
    assert!(!aegis_modules::version_satisfies("^0.1.0", "0.0.9"));
    assert!(aegis_modules::version_satisfies("*", "9.9.9"));
    assert!(aegis_modules::version_satisfies(">=1.2.0", "1.2.1"));
    assert!(!aegis_modules::version_satisfies(">=1.2.0", "1.1.9"));
    assert!(aegis_modules::version_satisfies("~1.2.3", "1.2.9"));
    assert!(!aegis_modules::version_satisfies("~1.2.3", "1.3.0"));
    assert!(aegis_modules::version_satisfies("=1.2.3", "1.2.3"));
    assert!(!aegis_modules::version_satisfies("1.2.3", "1.2.4"));
    assert!(!aegis_modules::version_satisfies("garbage", "1.2.3"));
}

fn owner_token(temp: &tempfile::TempDir) -> String {
    aegis_platform::PlatformService::initialize(temp.path())
        .expect("platform initialize")
        .owner_token
}

#[test]
fn module_mutations_require_owner_proof() {
    let (temp, paths) = temp_paths();
    let token = owner_token(&temp);
    let manager = aegis_modules::ModuleManager::new(paths.clone());
    manager.initialize().expect("manager initialize");
    let (manifest, json) = manifest_for("com.example.hello");

    let denied = manager
        .install(&json, Some(&manifest.integrity.digest), "wrong-token")
        .expect_err("wrong token must be denied");
    assert!(matches!(denied, ModuleError::Unauthorized));
    assert!(manager.list().expect("list").is_empty());

    manager
        .install(&json, Some(&manifest.integrity.digest), &token)
        .expect("authorized install");
    let denied = manager
        .enable("com.example.hello", "wrong-token")
        .expect_err("wrong token must be denied");
    assert!(matches!(denied, ModuleError::Unauthorized));
    assert_eq!(
        manager
            .list()
            .expect("list")
            .first()
            .expect("record")
            .lifecycle_state,
        ModuleLifecycleState::Installed
    );

    manager
        .check_enable("com.example.hello")
        .expect("read-only enable precondition passes");
    manager
        .check_disable("com.example.hello")
        .expect("read-only disable precondition passes");
    manager
        .enable("com.example.hello", &token)
        .expect("authorized enable");
    let error = manager
        .check_enable("com.example.hello")
        .expect_err("enabled module is not re-enableable");
    assert!(matches!(
        error,
        ModuleError::IllegalTransition { ref from, ref to }
            if from == "enabled" && to == "enabled"
    ));
}

#[test]
fn dependency_version_requirements_block_install() {
    let (_temp, paths) = temp_paths();
    let registry = registry(&paths);
    let (_dep, dep_json) = manifest_for("com.example.dep");
    registry
        .install(&dep_json, None)
        .expect("install dependency");

    let mut consumer = example_manifest();
    consumer.module_id = "com.example.consumer".to_owned();
    consumer.dependencies = vec![ModuleDependency {
        module_id: "com.example.dep".to_owned(),
        version_requirement: Some("^1.0.0".to_owned()),
        min_version: None,
        max_version: None,
    }];
    let json = seal(&mut consumer);
    let error = registry
        .install(&json, None)
        .expect_err("version mismatch must fail");
    assert!(matches!(
        error,
        ModuleError::DependencyVersionMismatch { .. }
    ));
    assert_eq!(registry.list().expect("list").len(), 1);

    let mut compatible = example_manifest();
    compatible.module_id = "com.example.consumer".to_owned();
    compatible.dependencies = vec![ModuleDependency {
        module_id: "com.example.dep".to_owned(),
        version_requirement: Some("^0.1.0".to_owned()),
        min_version: None,
        max_version: None,
    }];
    let json = seal(&mut compatible);
    registry
        .install(&json, None)
        .expect("compatible version installs");
}

fn dependency_consumer(module_id: &str, dependency_id: &str) -> (ModuleManifest, String) {
    let mut consumer = example_manifest();
    consumer.module_id = module_id.to_owned();
    consumer.dependencies = vec![ModuleDependency {
        module_id: dependency_id.to_owned(),
        version_requirement: None,
        min_version: None,
        max_version: None,
    }];
    let json = seal(&mut consumer);
    (consumer, json)
}

#[test]
fn dependency_min_and_max_versions_are_enforced_on_install() {
    let (_temp, paths) = temp_paths();
    let registry = registry(&paths);
    let (_dep, dep_json) = manifest_for("com.example.dep");
    registry
        .install(&dep_json, None)
        .expect("install dependency");

    let mut compatible = example_manifest();
    compatible.module_id = "com.example.compatible".to_owned();
    compatible.dependencies = vec![ModuleDependency {
        module_id: "com.example.dep".to_owned(),
        version_requirement: None,
        min_version: Some("0.1.0".to_owned()),
        max_version: Some("0.1.9".to_owned()),
    }];
    let json = seal(&mut compatible);
    registry
        .install(&json, None)
        .expect("in-range dependency installs");

    let mut too_new = example_manifest();
    too_new.module_id = "com.example.too-new".to_owned();
    too_new.dependencies = vec![ModuleDependency {
        module_id: "com.example.dep".to_owned(),
        version_requirement: None,
        min_version: Some("1.0.0".to_owned()),
        max_version: None,
    }];
    let json = seal(&mut too_new);
    let error = registry
        .install(&json, None)
        .expect_err("dependency below its minimum must fail");
    assert!(matches!(
        error,
        ModuleError::DependencyVersionMismatch { ref requirement, ref actual, .. }
            if requirement.contains(">=1.0.0") && actual == "0.1.0"
    ));

    let mut too_old = example_manifest();
    too_old.module_id = "com.example.too-old".to_owned();
    too_old.dependencies = vec![ModuleDependency {
        module_id: "com.example.dep".to_owned(),
        version_requirement: None,
        min_version: None,
        max_version: Some("0.0.9".to_owned()),
    }];
    let json = seal(&mut too_old);
    let error = registry
        .install(&json, None)
        .expect_err("dependency above its maximum must fail");
    assert!(matches!(
        error,
        ModuleError::DependencyVersionMismatch { ref requirement, ref actual, .. }
            if requirement.contains("<=0.0.9") && actual == "0.1.0"
    ));

    assert_eq!(registry.list().expect("list").len(), 2);
}

#[test]
fn enable_and_disable_preconditions_are_read_only_and_dependency_aware() {
    let (_temp, paths) = temp_paths();
    let registry = registry(&paths);
    let (_dep, dep_json) = manifest_for("com.example.dep");
    registry.install(&dep_json, None).expect("install dep");
    let (_consumer, consumer_json) = dependency_consumer("com.example.consumer", "com.example.dep");
    registry
        .install(&consumer_json, None)
        .expect("install consumer");

    registry
        .check_enable("com.example.consumer")
        .expect("enable precondition passes");
    registry
        .check_disable("com.example.consumer")
        .expect("disable precondition passes while installed");
    assert_eq!(
        registry
            .inspect("com.example.consumer")
            .expect("inspect")
            .record
            .lifecycle_state,
        ModuleLifecycleState::Installed
    );

    registry.enable("com.example.dep").expect("enable dep");
    registry
        .enable("com.example.consumer")
        .expect("enable consumer");
    let error = registry
        .check_enable("com.example.consumer")
        .expect_err("enabled module cannot be enabled again");
    assert!(matches!(
        error,
        ModuleError::IllegalTransition { ref from, ref to }
            if from == "enabled" && to == "enabled"
    ));

    let error = registry
        .check_disable("com.example.dep")
        .expect_err("cannot disable a provider with an enabled dependent");
    assert!(matches!(
        error,
        ModuleError::DependencyInUse { ref module_id, ref dependent }
            if module_id == "com.example.dep" && dependent == "com.example.consumer"
    ));
    let error = registry
        .disable("com.example.dep")
        .expect_err("disable must enforce the same precondition");
    assert!(matches!(error, ModuleError::DependencyInUse { .. }));
    assert_eq!(
        registry
            .inspect("com.example.dep")
            .expect("inspect dep")
            .record
            .lifecycle_state,
        ModuleLifecycleState::Enabled
    );

    registry
        .check_disable("com.example.consumer")
        .expect("consumer has no enabled dependents");
    registry
        .disable("com.example.consumer")
        .expect("disable consumer");
    registry
        .check_disable("com.example.dep")
        .expect("provider is now safe to disable");
    registry.disable("com.example.dep").expect("disable dep");
}

#[test]
fn enable_precondition_rejects_an_unavailable_dependency() {
    let (_temp, paths) = temp_paths();
    let registry = registry(&paths);
    let (_dep, dep_json) = manifest_for("com.example.dep");
    registry.install(&dep_json, None).expect("install dep");
    let (_consumer, consumer_json) = dependency_consumer("com.example.consumer", "com.example.dep");
    registry
        .install(&consumer_json, None)
        .expect("install consumer");
    registry
        .check_enable("com.example.consumer")
        .expect("dependency is available");

    registry
        .uninstall("com.example.dep")
        .expect("uninstall dependency");
    let error = registry
        .check_enable("com.example.consumer")
        .expect_err("check must reject an unavailable dependency");
    assert!(matches!(
        error,
        ModuleError::DependencyUnavailable { ref module_id, .. }
            if module_id == "com.example.dep"
    ));
    let error = registry
        .enable("com.example.consumer")
        .expect_err("enable must reject an unavailable dependency");
    assert!(matches!(
        error,
        ModuleError::DependencyUnavailable { ref module_id, .. }
            if module_id == "com.example.dep"
    ));
    assert_eq!(
        registry
            .inspect("com.example.consumer")
            .expect("inspect consumer")
            .record
            .lifecycle_state,
        ModuleLifecycleState::Installed
    );
}

#[test]
fn invalid_dependency_version_bounds_are_rejected_on_install() {
    let (_temp, paths) = temp_paths();
    let registry = registry(&paths);
    let (_dep, dep_json) = manifest_for("com.example.dep");
    registry.install(&dep_json, None).expect("install dep");

    let mut consumer = example_manifest();
    consumer.module_id = "com.example.consumer".to_owned();
    consumer.dependencies = vec![ModuleDependency {
        module_id: "com.example.dep".to_owned(),
        version_requirement: None,
        min_version: Some("2.0.0".to_owned()),
        max_version: Some("1.0.0".to_owned()),
    }];
    let json = seal(&mut consumer);
    let error = registry
        .install(&json, None)
        .expect_err("inverted bounds must fail");
    assert!(matches!(
        error,
        ModuleError::Manifest(ManifestError::InvalidDependency(_))
    ));

    consumer.dependencies[0].min_version = Some("nope".to_owned());
    consumer.dependencies[0].max_version = None;
    let json = seal(&mut consumer);
    let error = registry
        .install(&json, None)
        .expect_err("malformed minimum must fail");
    assert!(matches!(
        error,
        ModuleError::Manifest(ManifestError::InvalidDependency(_))
    ));
    assert_eq!(registry.list().expect("list").len(), 1);
}

#[test]
fn module_events_are_append_only_at_the_database_level() {
    let (_temp, paths) = temp_paths();
    let registry = registry(&paths);
    let (manifest, json) = manifest_for("com.example.hello");
    registry
        .install(&json, Some(&manifest.integrity.digest))
        .expect("install");

    let connection = open_initialized(
        &paths.platform_db(),
        PLATFORM_APPLICATION_ID,
        "modules-test",
    )
    .expect("open platform db");
    let update = connection.execute(
        "UPDATE aegis_module_events SET detail = 'rewritten' WHERE id = 1",
        [],
    );
    assert!(update.is_err(), "event update must be rejected");
    let delete = connection.execute("DELETE FROM aegis_module_events WHERE id = 1", []);
    assert!(delete.is_err(), "event delete must be rejected");
}

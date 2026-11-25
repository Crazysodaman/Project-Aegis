use std::fs;

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_core_install::CoreInstaller;
use aegis_installer::{
    install, installation_record, uninstall, upgrade, InstallMode, InstallerError, UninstallMode,
    AEGIS_VERSION,
};
use aegis_platform::PlatformService;
use aegis_storage::{now_rfc3339, open_initialized, PLATFORM_APPLICATION_ID};
use rusqlite::params;

const CORE: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";

fn temp_root() -> (tempfile::TempDir, std::path::PathBuf) {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("aegis-root");
    (temp, root)
}

fn install_core(root: &std::path::Path, token: &str) {
    let installer = CoreInstaller::new(root).expect("installer");
    installer
        .initialize(
            &CoreInitializationRequest {
                schema: CORE_INIT_REQUEST_SCHEMA.to_owned(),
                core_uuid: CORE.to_owned(),
                core_number: 1,
                display_name: "Sofía Ada Lyra".to_owned(),
                requested_at: now_rfc3339().expect("timestamp"),
            },
            token,
        )
        .expect("core install");
}

fn set_recorded_version(root: &std::path::Path, version: &str) {
    let connection = open_initialized(
        &root.join("platform.db"),
        PLATFORM_APPLICATION_ID,
        "platform",
    )
    .expect("platform db");
    connection
        .execute(
            "UPDATE platform_config SET config_value = ?1 WHERE config_key = 'installation.version'",
            params![version],
        )
        .expect("version update");
}

#[test]
fn install_initializes_platform_and_records_version() {
    let (_temp, root) = temp_root();
    let report = install(&root, InstallMode::Headless).expect("install");
    assert!(report.newly_initialized);
    assert_eq!(report.record.schema, "aegis.installation.v1");
    assert_eq!(report.record.aegis_version, AEGIS_VERSION);
    assert_eq!(report.record.state, "installed");
    assert_eq!(report.record.mode, "headless");
    assert!(root.join("platform.db").is_file());
    assert!(root.join("safe.db").is_file());
    assert!(root.join("owner.proof").is_file());

    let record = installation_record(&root)
        .expect("record")
        .expect("present");
    assert_eq!(record.platform_id, report.record.platform_id);

    let status = PlatformService::start(&root)
        .expect("platform")
        .status()
        .expect("status");
    assert!(status.zero_core);
    assert_eq!(status.core_count, 0);
}

#[test]
fn repeated_install_preserves_platform_identity() {
    let (_temp, root) = temp_root();
    let first = install(&root, InstallMode::Guided).expect("first");
    let second = install(&root, InstallMode::Headless).expect("second");
    assert!(first.newly_initialized);
    assert!(!second.newly_initialized);
    assert_eq!(first.record.platform_id, second.record.platform_id);
    assert_eq!(first.record.callsign, second.record.callsign);
}

#[test]
fn upgrade_at_current_version_is_a_noop() {
    let (_temp, root) = temp_root();
    install(&root, InstallMode::Headless).expect("install");
    let token = PlatformService::start(&root)
        .expect("platform")
        .owner_token()
        .expect("token");
    let report = upgrade(&root, &token, None).expect("upgrade");
    assert!(report.already_current);
    assert!(report.backup.is_none());
}

#[test]
fn upgrade_refuses_downgrade() {
    let (_temp, root) = temp_root();
    install(&root, InstallMode::Headless).expect("install");
    set_recorded_version(&root, "99.0.0");
    let token = PlatformService::start(&root)
        .expect("platform")
        .owner_token()
        .expect("token");
    let error = upgrade(&root, &token, None).expect_err("downgrade must fail");
    assert!(matches!(error, InstallerError::DowngradeRefused { .. }));
}

#[test]
fn upgrade_backs_up_and_preserves_core_data() {
    let (temp, root) = temp_root();
    let report = install(&root, InstallMode::Headless).expect("install");
    let token = PlatformService::start(&root)
        .expect("platform")
        .owner_token()
        .expect("token");
    install_core(&root, &token);
    set_recorded_version(&root, "0.0.1");

    let installer = CoreInstaller::new(&root).expect("installer");
    let database = installer.layout().database_path(CORE).expect("db path");
    let manifest = installer
        .layout()
        .integrity_path(CORE)
        .expect("manifest path");
    let database_before = fs::read(&database).expect("db bytes");
    let manifest_before = fs::read(&manifest).expect("manifest bytes");

    let backup = temp.path().join("pre-upgrade-backup");
    let upgraded = upgrade(&root, &token, Some(&backup)).expect("upgrade");
    assert!(!upgraded.already_current);
    assert_eq!(upgraded.from_version, "0.0.1");
    assert_eq!(upgraded.to_version, AEGIS_VERSION);
    assert_eq!(upgraded.record.state, "installed");
    assert_eq!(upgraded.record.platform_id, report.record.platform_id);

    assert!(backup.is_dir());
    assert!(aegis_backup::verify_backup(&backup).is_ok());
    assert_eq!(fs::read(&database).expect("db after"), database_before);
    assert_eq!(
        fs::read(&manifest).expect("manifest after"),
        manifest_before
    );
    installer.verify(CORE).expect("core remains verified");
}

#[test]
fn uninstall_preserve_keeps_every_file() {
    let (_temp, root) = temp_root();
    install(&root, InstallMode::Headless).expect("install");
    let token = PlatformService::start(&root)
        .expect("platform")
        .owner_token()
        .expect("token");
    install_core(&root, &token);

    let report = uninstall(&root, &token, UninstallMode::Preserve, None, None).expect("uninstall");
    assert!(report.deleted.is_empty());
    assert!(root.join("platform.db").is_file());
    assert!(root.join("safe.db").is_file());
    assert!(root.join("cores").is_dir());
    assert_eq!(report.record.state, "uninstalled_preserved");
}

#[test]
fn uninstall_delete_requires_confirmation_and_backup() {
    let (temp, root) = temp_root();
    let installed = install(&root, InstallMode::Headless).expect("install");
    let token = PlatformService::start(&root)
        .expect("platform")
        .owner_token()
        .expect("token");
    let backup = temp.path().join("delete-backup");

    let missing_backup = uninstall(
        &root,
        &token,
        UninstallMode::DeleteData,
        Some(&installed.record.callsign),
        None,
    )
    .expect_err("backup is required");
    assert!(matches!(missing_backup, InstallerError::BackupRequired));

    let wrong_confirmation = uninstall(
        &root,
        &token,
        UninstallMode::DeleteData,
        Some("not-the-callsign"),
        Some(&backup),
    )
    .expect_err("confirmation is required");
    assert!(matches!(
        wrong_confirmation,
        InstallerError::ConfirmationRequired
    ));
    assert!(root.join("platform.db").is_file());

    let report = uninstall(
        &root,
        &token,
        UninstallMode::DeleteData,
        Some(&installed.record.callsign),
        Some(&backup),
    )
    .expect("delete uninstall");
    assert!(!root.join("platform.db").exists());
    assert!(!root.join("safe.db").exists());
    assert!(!root.join("owner.proof").exists());
    assert!(!root.join("cores").exists());
    assert!(report.deleted.contains(&"platform.db".to_owned()));
    assert!(aegis_backup::verify_backup(&backup).is_ok());
}

#[test]
fn uninstall_refuses_a_running_service() {
    let (_temp, root) = temp_root();
    install(&root, InstallMode::Headless).expect("install");
    let token = PlatformService::start(&root)
        .expect("platform")
        .owner_token()
        .expect("token");
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).expect("run dir");
    fs::write(
        run_dir.join("service.json"),
        serde_json::to_vec(&serde_json::json!({
            "schema": "aegis.service.state.v1",
            "pid": std::process::id(),
            "root": root.display().to_string(),
            "socket": "aegis-platform",
            "started_at": now_rfc3339().expect("timestamp"),
            "executable": "test",
            "protocol": "aegis.ipc.v1"
        }))
        .expect("json"),
    )
    .expect("state");

    let error = uninstall(&root, &token, UninstallMode::Preserve, None, None)
        .expect_err("running service must block uninstall");
    assert!(matches!(error, InstallerError::ServiceRunning));
    assert!(root.join("platform.db").is_file());
}

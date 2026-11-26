use std::fs;
use std::path::Path;

use aegis_installer::{
    install, installation_record, preflight_install, preflight_uninstall, preflight_upgrade,
    rollback_plan_install, rollback_plan_uninstall, rollback_plan_upgrade, upgrade, InstallMode,
    InstallerError, PreflightAction, PreflightStatus, RollbackError, UninstallMode,
};
use aegis_platform::PlatformService;
use aegis_storage::{now_rfc3339, open_initialized, PLATFORM_APPLICATION_ID};
use rusqlite::params;

fn temp_root() -> (tempfile::TempDir, std::path::PathBuf) {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("aegis-root");
    (temp, root)
}

fn start_token(root: &Path) -> String {
    PlatformService::start(root)
        .expect("platform")
        .owner_token()
        .expect("token")
}

fn set_recorded_version(root: &Path, version: &str) {
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
fn install_preflight_on_fresh_root_is_ready_and_inert() {
    let (_temp, root) = temp_root();
    let report = preflight_install(&root, InstallMode::Guided);

    assert_eq!(report.action, PreflightAction::Install);
    assert!(report.ready, "{report}");
    assert!(report.is_inert());
    assert!(!report.mutations_performed);
    assert!(!root.exists(), "preflight must not create the storage root");
    assert_eq!(
        report.check("root-absolute").expect("root check").status,
        PreflightStatus::Pass
    );
    assert_eq!(
        report.check("installation").expect("install check").status,
        PreflightStatus::Pass
    );
    assert_eq!(
        report
            .check("backup-not-required")
            .expect("backup check")
            .status,
        PreflightStatus::Warn
    );
    assert_eq!(report.render(), report.to_string());
}

#[test]
fn install_preflight_flags_a_newer_recorded_version() {
    let (_temp, root) = temp_root();
    install(&root, InstallMode::Headless).expect("install");
    set_recorded_version(&root, "99.0.0");

    let report = preflight_install(&root, InstallMode::Headless);
    assert!(!report.ready);
    let version = report.check("version").expect("version check");
    assert_eq!(version.status, PreflightStatus::Fail);
    assert!(version.detail.contains("newer"), "{}", version.detail);
    assert_eq!(report.failures().count(), 1);
}

#[test]
fn upgrade_preflight_refuses_a_downgrade() {
    let (_temp, root) = temp_root();
    install(&root, InstallMode::Headless).expect("install");
    let token = start_token(&root);
    set_recorded_version(&root, "99.0.0");

    let report = preflight_upgrade(&root, &token, None);
    assert!(!report.ready);
    assert_eq!(
        report.check("version").expect("version check").status,
        PreflightStatus::Fail
    );
    assert_eq!(
        report.check("platform-present").expect("platform").status,
        PreflightStatus::Pass
    );
    assert_eq!(
        report.check("owner-token").expect("owner").status,
        PreflightStatus::Pass
    );
}

#[test]
fn upgrade_preflight_is_backup_gated() {
    let (temp, root) = temp_root();
    install(&root, InstallMode::Headless).expect("install");
    let token = start_token(&root);
    set_recorded_version(&root, "0.0.1");

    let inside = root.join("backup-inside-root");
    let blocked = preflight_upgrade(&root, &token, Some(&inside));
    assert!(!blocked.ready);
    assert_eq!(
        blocked
            .check("backup-destination")
            .expect("backup check")
            .status,
        PreflightStatus::Fail
    );

    let outside = temp.path().join("backup-outside-root");
    let ready = preflight_upgrade(&root, &token, Some(&outside));
    assert!(ready.ready, "{ready}");
    assert_eq!(
        ready.check("version").expect("version check").status,
        PreflightStatus::Pass
    );
    assert_eq!(
        ready
            .check("backup-destination")
            .expect("backup check")
            .status,
        PreflightStatus::Pass
    );
    assert!(!inside.exists(), "preflight must not create a backup");
    assert!(!outside.exists(), "preflight must not create a backup");
}

#[test]
fn upgrade_preflight_warns_but_does_not_block_on_a_running_service() {
    let (_temp, root) = temp_root();
    install(&root, InstallMode::Headless).expect("install");
    let token = start_token(&root);
    set_recorded_version(&root, "0.0.1");
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

    let report = preflight_upgrade(&root, &token, None);
    assert!(report.ready, "{report}");
    assert_eq!(
        report.check("service-stopped").expect("service").status,
        PreflightStatus::Warn
    );
}

#[test]
fn a_failed_backup_gate_leaves_the_recorded_version_unchanged() {
    let (_temp, root) = temp_root();
    install(&root, InstallMode::Headless).expect("install");
    let token = start_token(&root);
    set_recorded_version(&root, "0.0.1");

    let inside = root.join("backup-inside-root");
    let error = upgrade(&root, &token, Some(&inside)).expect_err("backup inside root must fail");
    assert!(matches!(error, InstallerError::Backup(_)), "{error:?}");

    let record = installation_record(&root)
        .expect("record")
        .expect("present");
    assert_eq!(record.aegis_version, "0.0.1");
    assert!(!inside.exists());
}

#[test]
fn uninstall_preflight_preserve_needs_no_backup() {
    let (_temp, root) = temp_root();
    install(&root, InstallMode::Headless).expect("install");
    let token = start_token(&root);

    let report = preflight_uninstall(&root, &token, UninstallMode::Preserve, None, None);
    assert!(report.ready, "{report}");
    assert_eq!(
        report
            .check("uninstall-backup")
            .expect("backup check")
            .status,
        PreflightStatus::Warn
    );
    assert_eq!(
        report.check("service-stopped").expect("service").status,
        PreflightStatus::Pass
    );
    assert!(root.join("platform.db").is_file());
    assert!(root.join("safe.db").is_file());
}

#[test]
fn uninstall_preflight_delete_requires_backup_and_confirmation() {
    let (temp, root) = temp_root();
    install(&root, InstallMode::Headless).expect("install");
    let token = start_token(&root);
    let callsign = installation_record(&root)
        .expect("record")
        .expect("present")
        .callsign;

    let no_backup = preflight_uninstall(
        &root,
        &token,
        UninstallMode::DeleteData,
        Some(&callsign),
        None,
    );
    assert!(!no_backup.ready);
    assert_eq!(
        no_backup
            .check("uninstall-backup")
            .expect("backup check")
            .status,
        PreflightStatus::Fail
    );

    let backup = temp.path().join("delete-backup");
    let wrong_confirmation = preflight_uninstall(
        &root,
        &token,
        UninstallMode::DeleteData,
        Some("not-the-callsign"),
        Some(&backup),
    );
    assert!(!wrong_confirmation.ready);
    assert_eq!(
        wrong_confirmation
            .check("uninstall-confirmation")
            .expect("confirmation")
            .status,
        PreflightStatus::Fail
    );

    let ready = preflight_uninstall(
        &root,
        &token,
        UninstallMode::DeleteData,
        Some(&callsign),
        Some(&backup),
    );
    assert!(ready.ready, "{ready}");
    assert_eq!(
        ready
            .check("uninstall-confirmation")
            .expect("confirmation")
            .status,
        PreflightStatus::Pass
    );
    assert!(!backup.exists(), "preflight must not create a backup");
}

#[test]
fn uninstall_preflight_blocks_on_a_running_service() {
    let (_temp, root) = temp_root();
    install(&root, InstallMode::Headless).expect("install");
    let token = start_token(&root);
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

    let report = preflight_uninstall(&root, &token, UninstallMode::Preserve, None, None);
    assert!(!report.ready);
    assert_eq!(
        report.check("service-stopped").expect("service").status,
        PreflightStatus::Fail
    );
}

#[test]
fn rollback_plans_are_data_preserving_and_validated() {
    let install = rollback_plan_install();
    assert_eq!(install.action, PreflightAction::Install);
    assert!(!install.requires_backup);
    assert!(install.dry_run);
    assert_eq!(install.validate(), Ok(()));

    let upgrade = rollback_plan_upgrade();
    assert_eq!(upgrade.action, PreflightAction::Upgrade);
    assert!(upgrade.requires_backup);
    assert!(upgrade
        .steps
        .iter()
        .any(|step| step.code == "restore-backup"));
    assert_eq!(upgrade.validate(), Ok(()));

    let preserve = rollback_plan_uninstall(UninstallMode::Preserve);
    assert!(!preserve.requires_backup);
    assert_eq!(preserve.validate(), Ok(()));

    let delete = rollback_plan_uninstall(UninstallMode::DeleteData);
    assert!(delete.requires_backup);
    assert!(delete
        .steps
        .iter()
        .any(|step| step.code == "restore-backup"));
    assert_eq!(delete.validate(), Ok(()));

    let mut not_preserving = upgrade.clone();
    not_preserving.steps[0].preserves_data = false;
    assert!(matches!(
        not_preserving.validate(),
        Err(RollbackError::NotDataPreserving { .. })
    ));

    let mut missing_backup = upgrade;
    missing_backup
        .steps
        .retain(|step| step.code != "restore-backup");
    missing_backup.steps[0].order = 1;
    assert_eq!(
        missing_backup.validate(),
        Err(RollbackError::MissingBackupStep)
    );
}

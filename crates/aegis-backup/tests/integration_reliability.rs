use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use aegis_backup::{
    backup, read_manifest, restore, verify_backup, BackupVerificationStatus, RestoreOptions,
};
use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_core_install::CoreInstaller;
use aegis_platform::PlatformService;
use aegis_storage::now_rfc3339;

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_B: &str = "b9a0a604-a4e7-4a88-a985-3f071e2c8e7d";

fn request(core_uuid: &str, core_number: u8, display_name: &str) -> CoreInitializationRequest {
    CoreInitializationRequest {
        schema: CORE_INIT_REQUEST_SCHEMA.to_owned(),
        core_uuid: core_uuid.to_owned(),
        core_number,
        display_name: display_name.to_owned(),
        requested_at: now_rfc3339().expect("timestamp"),
    }
}

fn install(root: &Path, token: &str, core_uuid: &str, core_number: u8, name: &str) {
    let installer = CoreInstaller::new(root).expect("installer");
    installer
        .initialize(&request(core_uuid, core_number, name), token)
        .expect("initialize Core");
}

fn setup_source(root: &Path) -> String {
    let initialized = PlatformService::initialize(root).expect("platform init");
    install(root, &initialized.owner_token, CORE_A, 1, "Core A");
    install(root, &initialized.owner_token, CORE_B, 2, "Core B");
    initialized.owner_token
}

fn verify_core(root: &Path, core_uuid: &str) {
    let installer = CoreInstaller::new(root).expect("installer");
    assert_eq!(
        installer.verify(core_uuid).expect("verify").status.as_str(),
        "verified"
    );
}

#[test]
fn repeated_backup_verify_restore_cycles_preserve_identity_and_content() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let source = workspace.path().join("source");
    let _token = setup_source(&source);

    let cycles = 3;
    let mut current = source.clone();
    let mut platform_ids = Vec::new();
    let started = Instant::now();
    for cycle in 0..cycles {
        let destination = workspace.path().join(format!("backup-{cycle}"));
        let report = backup(&current, &destination).expect("backup");
        assert_eq!(report.manifest.cores.len(), 2);
        assert!(report
            .manifest
            .cores
            .iter()
            .all(|core| core.integrity_status == "verified"));
        assert!(report.file_count > 0);
        assert_eq!(
            report.total_bytes,
            report
                .manifest
                .files
                .iter()
                .map(|file| file.bytes)
                .sum::<u64>()
        );
        platform_ids.push(report.manifest.platform_id.clone());

        let manifest = read_manifest(&destination).expect("manifest");
        assert_eq!(manifest.platform_id, report.manifest.platform_id);

        let verification = verify_backup(&destination).expect("verify");
        assert_eq!(verification.status, BackupVerificationStatus::Verified);
        assert!(verification.issues.is_empty());

        let target = workspace.path().join(format!("restored-{cycle}"));
        let restored = restore(&destination, &target, RestoreOptions::default()).expect("restore");
        assert_eq!(restored.restored_cores.len(), 2);
        assert!(restored.restored_cores.contains(&CORE_A.to_owned()));
        assert!(restored.restored_cores.contains(&CORE_B.to_owned()));
        assert!(restored.preserved_source.is_none());
        verify_core(&target, CORE_A);
        verify_core(&target, CORE_B);

        current = target;
    }
    let elapsed = started.elapsed();
    assert!(platform_ids.windows(2).all(|pair| pair[0] == pair[1]));

    let platform = PlatformService::start(&current).expect("restored platform starts");
    platform.status().expect("restored platform status");
    drop(platform);
    verify_core(&current, CORE_A);
    verify_core(&current, CORE_B);

    fs::remove_dir_all(workspace.path()).expect("no leaked backup handles");
    drop(workspace);
    eprintln!("{cycles} chained backup/verify/restore cycles in {elapsed:?}");
}

#[test]
fn repeated_restore_over_allowed_existing_target_keeps_verified_content() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let source = workspace.path().join("source");
    let _token = setup_source(&source);
    let destination = workspace.path().join("backup");
    backup(&source, &destination).expect("backup");
    let target = workspace.path().join("target");
    restore(&destination, &target, RestoreOptions::default()).expect("first restore");

    let started = Instant::now();
    for cycle in 0..3 {
        let preserved = workspace.path().join(format!("preserved-{cycle}"));
        let restored = restore(
            &destination,
            &target,
            RestoreOptions {
                allow_existing: true,
                backup_existing_to: Some(preserved.clone()),
            },
        )
        .expect("repeated restore over existing target");
        assert_eq!(restored.restored_cores.len(), 2);
        assert!(restored.preserved_source.is_some());
        assert!(preserved.is_dir());
        verify_core(&target, CORE_A);
        verify_core(&target, CORE_B);
        assert_eq!(
            verify_backup(&destination).expect("verify").status,
            BackupVerificationStatus::Verified
        );
    }
    let elapsed = started.elapsed();

    fs::remove_dir_all(workspace.path()).expect("no leaked restore handles");
    drop(workspace);
    eprintln!("3 repeated restores over existing target in {elapsed:?}");
}

#[test]
fn backup_directory_discovery_is_stable_across_repeated_cycles() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let source: PathBuf = workspace.path().join("source");
    let _token = setup_source(&source);
    for cycle in 0..3 {
        let destination = workspace.path().join(format!("snap-{cycle}"));
        backup(&source, &destination).expect("backup");
        assert_eq!(
            verify_backup(&destination).expect("verify").status,
            BackupVerificationStatus::Verified
        );
    }
    let listed = aegis_backup::list_backups(workspace.path()).expect("list backups");
    assert_eq!(listed.len(), 3);
    let mut sorted = listed.clone();
    sorted.sort();
    assert_eq!(listed, sorted);
}

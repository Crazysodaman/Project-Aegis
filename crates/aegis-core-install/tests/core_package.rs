use std::fs;
use std::path::Path;

use aegis_core_install::{
    install_package, install_package_bytes, validate_package, CoreInstaller, CorePackageFile,
    CorePackageManifest, InstallError, PackageError, CORE_PACKAGE_FORMAT_VERSION,
    CORE_PACKAGE_MANIFEST_FILE, CORE_PACKAGE_SCHEMA,
};
use aegis_core_integrity::compute_file_sha256;
use aegis_platform::PlatformService;

const MACK: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const VERA: &str = "b1b1b1b1-2222-4333-8444-555566667777";
const TWIN_FOUR: &str = "c2c2c2c2-3333-4444-8555-666677778888";
const BLANK: &str = "e4e4e4e4-5555-4666-8777-88889999aaaa";

fn setup() -> (tempfile::TempDir, CoreInstaller, String) {
    let temp = tempfile::tempdir().expect("tempdir");
    let initialized = PlatformService::initialize(temp.path()).expect("platform init");
    let installer = CoreInstaller::new(temp.path()).expect("installer");
    (temp, installer, initialized.owner_token)
}

fn package_dir(temp: &Path, name: &str) -> std::path::PathBuf {
    temp.join(name)
}

fn build_package(
    directory: &Path,
    core_uuid: &str,
    core_number: u8,
    display_name: &str,
    designation: Option<&str>,
) -> CorePackageManifest {
    fs::create_dir_all(directory.join("core")).expect("core dir");
    fs::create_dir_all(directory.join("assets")).expect("asset dir");
    let config_path = directory.join("core/config.json");
    let asset_path = directory.join("assets/avatar.bin");
    fs::write(&config_path, b"{\"initialization\":\"fresh\"}").expect("write config");
    fs::write(&asset_path, b"aegis-avatar-bytes").expect("write asset");

    let config = CorePackageFile {
        path: "core/config.json".to_owned(),
        sha256: compute_file_sha256(&config_path).expect("config digest"),
    };
    let asset = CorePackageFile {
        path: "assets/avatar.bin".to_owned(),
        sha256: compute_file_sha256(&asset_path).expect("asset digest"),
    };
    let manifest = CorePackageManifest {
        schema: CORE_PACKAGE_SCHEMA.to_owned(),
        version: CORE_PACKAGE_FORMAT_VERSION.to_owned(),
        core_uuid: core_uuid.to_owned(),
        core_number,
        display_name: display_name.to_owned(),
        designation: designation.map(str::to_owned),
        files: vec![config],
        assets: vec![asset],
    };
    write_manifest(directory, &manifest);
    manifest
}

fn write_manifest(directory: &Path, manifest: &CorePackageManifest) {
    fs::create_dir_all(directory).expect("package dir");
    fs::write(
        directory.join(CORE_PACKAGE_MANIFEST_FILE),
        manifest.to_json(),
    )
    .expect("write manifest");
}

fn manifest_only(
    core_uuid: &str,
    core_number: u8,
    display_name: &str,
    designation: Option<&str>,
) -> CorePackageManifest {
    CorePackageManifest {
        schema: CORE_PACKAGE_SCHEMA.to_owned(),
        version: CORE_PACKAGE_FORMAT_VERSION.to_owned(),
        core_uuid: core_uuid.to_owned(),
        core_number,
        display_name: display_name.to_owned(),
        designation: designation.map(str::to_owned),
        files: Vec::new(),
        assets: Vec::new(),
    }
}

#[test]
fn valid_package_installs_and_verifies() {
    let (temp, installer, token) = setup();
    let directory = package_dir(temp.path(), "mack.corepkg");
    let manifest = build_package(&directory, MACK, 2, "Mack Fuller", Some("Mack Fuller"));

    let validated = validate_package(&directory).expect("package must validate");
    assert_eq!(validated.verified_files, 2);
    assert_eq!(validated.manifest.core_uuid, MACK);
    assert_eq!(validated.manifest.core_number, 2);
    assert_eq!(validated.manifest.display_name, "Mack Fuller");
    assert!(validated.total_bytes > 0);

    let installation = install_package(temp.path(), &directory, &token).expect("install package");
    assert_eq!(installation.identity.core_uuid, MACK);
    assert_eq!(installation.identity.core_number, 2);
    assert_eq!(installation.identity.display_name, "Mack Fuller");
    assert_eq!(manifest.core_number, 2);
    assert_eq!(installer.list().expect("list").len(), 1);
    assert_eq!(
        installer.verify(MACK).expect("verify").status.as_str(),
        "verified"
    );
}

#[test]
fn digest_mismatch_is_rejected_before_any_core_is_created() {
    let (temp, installer, token) = setup();
    let directory = package_dir(temp.path(), "tampered.corepkg");
    build_package(&directory, MACK, 2, "Mack Fuller", None);

    fs::write(directory.join("assets/avatar.bin"), b"tampered-bytes").expect("tamper asset");

    let error = validate_package(&directory).expect_err("digest mismatch must fail validation");
    assert!(matches!(error, PackageError::DigestMismatch { .. }));

    let error = install_package(temp.path(), &directory, &token).expect_err("install must fail");
    assert!(matches!(error, PackageError::DigestMismatch { .. }));
    assert!(installer.list().expect("list").is_empty());
    assert!(!installer
        .layout()
        .database_path(MACK)
        .expect("path")
        .exists());
}

#[test]
fn prohibited_core_four_is_rejected() {
    let (temp, installer, token) = setup();
    let directory = package_dir(temp.path(), "twin.corepkg");
    write_manifest(
        &directory,
        &manifest_only(TWIN_FOUR, 4, "Forbidden Twin", None),
    );

    let error = validate_package(&directory).expect_err("Core #4 must be prohibited");
    assert!(matches!(error, PackageError::ProhibitedCoreNumber(4)));

    let error = install_package(temp.path(), &directory, &token).expect_err("install must fail");
    assert!(matches!(error, PackageError::ProhibitedCoreNumber(4)));
    assert!(installer.list().expect("list").is_empty());
    assert!(!installer
        .layout()
        .database_path(TWIN_FOUR)
        .expect("path")
        .exists());
}

#[test]
fn blank_core_seven_requires_an_owner_supplied_name() {
    let (temp, installer, token) = setup();
    let blank_dir = package_dir(temp.path(), "blank.corepkg");
    write_manifest(&blank_dir, &manifest_only(BLANK, 7, "   ", None));

    let error = validate_package(&blank_dir).expect_err("blank name must be rejected");
    assert!(matches!(error, PackageError::DisplayNameRequired(7)));
    let error = install_package(temp.path(), &blank_dir, &token).expect_err("install must fail");
    assert!(matches!(error, PackageError::DisplayNameRequired(7)));
    assert!(installer.list().expect("list").is_empty());

    let named_dir = package_dir(temp.path(), "named.corepkg");
    write_manifest(&named_dir, &manifest_only(BLANK, 7, "Owner Chosen", None));
    let installation =
        install_package(temp.path(), &named_dir, &token).expect("owner-supplied name installs");
    assert_eq!(installation.identity.core_number, 7);
    assert_eq!(installation.identity.display_name, "Owner Chosen");
}

#[test]
fn mismatched_approved_identity_is_rejected() {
    let (temp, installer, _token) = setup();
    let directory = package_dir(temp.path(), "impostor.corepkg");
    write_manifest(&directory, &manifest_only(MACK, 2, "Impostor", None));

    let error = validate_package(&directory).expect_err("approved name mismatch");
    assert!(matches!(
        error,
        PackageError::DisplayNameMismatch { core_number: 2, .. }
    ));

    let wrong_designation = package_dir(temp.path(), "wrong-designation.corepkg");
    write_manifest(
        &wrong_designation,
        &manifest_only(VERA, 6, "Vera", Some("Not S-039")),
    );
    let error = validate_package(&wrong_designation).expect_err("designation mismatch");
    assert!(matches!(
        error,
        PackageError::DesignationMismatch { core_number: 6, .. }
    ));
    assert!(installer.list().expect("list").is_empty());
}

#[test]
fn duplicate_install_is_refused_without_overwriting() {
    let (temp, installer, token) = setup();
    let directory = package_dir(temp.path(), "mack.corepkg");
    build_package(&directory, MACK, 2, "Mack Fuller", None);

    let first = install_package(temp.path(), &directory, &token).expect("first install");
    let database_path = installer.layout().database_path(MACK).expect("db path");
    let before = compute_file_sha256(&database_path).expect("hash before");

    let error = install_package(temp.path(), &directory, &token).expect_err("duplicate refused");
    assert!(matches!(
        error,
        PackageError::Install(InstallError::AlreadyInitialized)
    ));
    assert_eq!(
        compute_file_sha256(&database_path).expect("hash after"),
        before
    );
    assert_eq!(installer.list().expect("list").len(), 1);
    assert_eq!(first.identity.core_uuid, MACK);
}

#[test]
fn uninstall_preserve_is_not_destructive() {
    let (temp, installer, token) = setup();
    let directory = package_dir(temp.path(), "mack.corepkg");
    build_package(&directory, MACK, 2, "Mack Fuller", None);
    install_package(temp.path(), &directory, &token).expect("install");

    let database_path = installer.layout().database_path(MACK).expect("db path");
    let manifest_path = installer
        .layout()
        .integrity_path(MACK)
        .expect("manifest path");
    let database_before = compute_file_sha256(&database_path).expect("db hash");
    let manifest_before = compute_file_sha256(&manifest_path).expect("manifest hash");

    let entry = installer
        .uninstall_preserve(MACK, "owner archive", &token)
        .expect("uninstall preserve");
    assert_eq!(entry.lifecycle_state.as_str(), "retired");
    assert!(database_path.is_file());
    assert!(manifest_path.is_file());
    assert_eq!(
        compute_file_sha256(&database_path).expect("db hash after"),
        database_before
    );
    assert_eq!(
        compute_file_sha256(&manifest_path).expect("manifest hash after"),
        manifest_before
    );
    assert_eq!(
        installer.verify(MACK).expect("verify").status.as_str(),
        "verified"
    );
}

#[test]
fn byte_package_without_external_files_installs() {
    let (temp, installer, token) = setup();
    let manifest = manifest_only(VERA, 6, "Vera", Some("Vera / S-039"));
    let bytes = manifest.to_json().into_bytes();

    let validated = validate_package(&bytes).expect("byte package must validate");
    assert_eq!(validated.verified_files, 0);
    assert_eq!(validated.manifest.display_name, "Vera");

    let installation =
        install_package_bytes(temp.path(), &bytes, &token).expect("byte package install");
    assert_eq!(installation.identity.core_uuid, VERA);
    assert_eq!(installation.identity.core_number, 6);
    assert_eq!(
        installer.verify(VERA).expect("verify").status.as_str(),
        "verified"
    );
}

#[test]
fn path_traversal_is_rejected() {
    let (temp, _installer, _token) = setup();
    let directory = package_dir(temp.path(), "escape.corepkg");
    let mut manifest = manifest_only(MACK, 2, "Mack Fuller", None);
    manifest.files.push(CorePackageFile {
        path: "../escape.bin".to_owned(),
        sha256: "0".repeat(64),
    });
    write_manifest(&directory, &manifest);

    let error = validate_package(&directory).expect_err("traversal must be rejected");
    assert!(matches!(error, PackageError::UnsafePath(_)));
}

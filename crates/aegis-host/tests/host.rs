use aegis_host::{HostError, HostStore};
use aegis_platform::PlatformService;
use aegis_storage::StoragePaths;

fn setup() -> (tempfile::TempDir, std::path::PathBuf, String) {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("aegis-root");
    let initialized = PlatformService::initialize(&root).expect("platform init");
    (temp, root, initialized.owner_token)
}

fn store(root: &std::path::Path) -> HostStore {
    HostStore::new(StoragePaths::new(root).expect("paths"))
}

#[test]
fn new_host_is_present_but_unenrolled() {
    let (_temp, root, _token) = setup();
    let status = store(&root).status().expect("status");
    assert!(!status.identity.host_uuid.is_empty());
    assert!(status.identity.callsign.starts_with("AEGIS-HOST-"));
    assert!(!status.identity.os.is_empty());
    assert_eq!(status.enrollment.state, "unenrolled");
    assert_eq!(status.enrollment.revision, 0);
    assert_eq!(status.identity.schema, "aegis.host.identity.v1");
    assert_eq!(status.enrollment.schema, "aegis.host.enrollment.v1");
}

#[test]
fn host_identity_is_stable_across_reopen() {
    let (_temp, root, _token) = setup();
    let first = store(&root).status().expect("first");
    let second = store(&root).status().expect("second");
    assert_eq!(first.identity, second.identity);
}

#[test]
fn enrollment_requires_owner_proof() {
    let (_temp, root, _token) = setup();
    let error = store(&root).enroll("not-the-owner").expect_err("must deny");
    assert!(matches!(error, HostError::Unauthorized));
    assert_eq!(
        store(&root).status().expect("status").enrollment.state,
        "unenrolled"
    );
}

#[test]
fn enroll_revoke_and_history_are_append_only() {
    let (_temp, root, token) = setup();
    let store = store(&root);

    let enrolled = store.enroll(&token).expect("enroll");
    assert_eq!(enrolled.enrollment.state, "enrolled");
    assert_eq!(enrolled.enrollment.revision, 1);
    assert_eq!(store.history().expect("history").len(), 1);

    let repeated = store.enroll(&token).expect("idempotent enroll");
    assert_eq!(repeated.enrollment.revision, 1);
    assert_eq!(store.history().expect("history").len(), 1);

    let revoked = store.revoke(&token).expect("revoke");
    assert_eq!(revoked.enrollment.state, "unenrolled");
    assert_eq!(revoked.enrollment.revision, 2);
    let history = store.history().expect("history");
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].from_state, "unenrolled");
    assert_eq!(history[0].to_state, "enrolled");
    assert_eq!(history[1].from_state, "enrolled");
    assert_eq!(history[1].to_state, "unenrolled");
}

#[test]
fn separate_roots_have_distinct_host_identities() {
    let (temp, root_a, _token) = setup();
    let root_b = temp.path().join("aegis-root-b");
    PlatformService::initialize(&root_b).expect("second platform");
    let a = store(&root_a).status().expect("a");
    let b = store(&root_b).status().expect("b");
    assert_ne!(a.identity.host_uuid, b.identity.host_uuid);
    assert_ne!(a.identity.callsign, b.identity.callsign);
}

#[test]
fn uninitialized_platform_is_rejected() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("missing-root");
    std::fs::create_dir_all(&root).expect("root");
    let error = store(&root).status().expect_err("must fail");
    assert!(matches!(error, HostError::NotInitialized(_)));
}

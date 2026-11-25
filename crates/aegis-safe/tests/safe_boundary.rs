use aegis_safe::{LocalOwnerProof, SafeError, SafeStore};
use aegis_storage::{open_read_only, StoragePaths, SAFE_APPLICATION_ID};

fn store() -> (tempfile::TempDir, SafeStore) {
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = StoragePaths::new(temp.path()).expect("paths");
    let store = SafeStore::new(paths);
    store.initialize().expect("initialize safe");
    (temp, store)
}

#[test]
fn stop_is_inactive_until_owner_sets_it() {
    let (_temp, store) = store();
    let status = store.stop_status().expect("status");
    assert!(!status.active);
    assert_eq!(status.revision, 0);
}

#[test]
fn owner_proof_is_required_for_stop_changes() {
    let (_temp, store) = store();
    let token = store.initialize_owner().expect("owner");
    assert!(store.verify_owner(&token).expect("verify"));
    assert!(!store.verify_owner("wrong-token").expect("wrong token"));
    assert!(matches!(
        store.set_stop("owner", "test", "wrong-token"),
        Err(SafeError::Unauthorized)
    ));
}

#[test]
fn stop_persists_and_clears_with_audit_chain() {
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = StoragePaths::new(temp.path()).expect("paths");
    let store = SafeStore::new(paths.clone());
    store.initialize().expect("initialize");
    let token = store.initialize_owner().expect("owner");
    let set = store
        .set_stop("owner", "batch1 test", &token)
        .expect("set stop");
    assert!(set.active);
    assert_eq!(set.revision, 1);

    let reopened = SafeStore::new(paths.clone());
    let persisted = reopened.stop_status().expect("status");
    assert!(persisted.active);
    assert_eq!(persisted.revision, 1);

    let cleared = reopened.clear_stop("owner", &token).expect("clear stop");
    assert!(!cleared.active);
    assert_eq!(cleared.revision, 2);
    assert!(reopened.audit_chain_valid().expect("audit chain"));
    assert!(reopened.audit_count().expect("audit count") >= 4);
}

#[test]
fn safe_database_has_its_own_application_marker() {
    let (temp, _store) = store();
    let paths = StoragePaths::new(temp.path()).expect("paths");
    let connection =
        open_read_only(&paths.safe_db(), SAFE_APPLICATION_ID).expect("read-only safe db");
    let marker: i64 = connection
        .query_row("PRAGMA application_id", [], |row| row.get(0))
        .expect("marker");
    assert_eq!(marker, SAFE_APPLICATION_ID);
}

#[test]
fn owner_proof_file_is_created() {
    let (temp, store) = store();
    store.initialize_owner().expect("owner");
    let paths = StoragePaths::new(temp.path()).expect("paths");
    assert!(paths.owner_proof().is_file());
    let proof = LocalOwnerProof::new(paths.owner_proof());
    assert!(!proof.load_token().expect("token").is_empty());
}

#[cfg(windows)]
#[test]
fn windows_owner_proof_is_dpapi_protected() {
    use std::fs;
    let (temp, store) = store();
    let token = store.initialize_owner().expect("owner");
    let paths = StoragePaths::new(temp.path()).expect("paths");
    let bytes = fs::read(paths.owner_proof()).expect("proof bytes");
    assert!(bytes.len() > token.len());
    assert_ne!(bytes, token.as_bytes());
}

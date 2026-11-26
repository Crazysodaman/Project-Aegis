//! Adversarial owner-proof and STOP enforcement tests (Final / Agent 16).
//!
//! These complement `security_acceptance.rs` and `safe_boundary.rs`. They cover
//! hostile token shapes, owner-proof removal, audit tampering, foreign database
//! markers, and the documented limits of STOP when `safe.db` itself is removed.

use std::fs;

use aegis_safe::{SafeError, SafeStore};
use aegis_storage::{open_initialized, StorageError, StoragePaths, SAFE_APPLICATION_ID};
use rusqlite::Connection;

const CORE_APPLICATION_ID: i64 = 0x4145_4743; // "AEGC"
const MEMORY_APPLICATION_ID: i64 = 0x4145_474d; // "AEGM"

fn store() -> (tempfile::TempDir, StoragePaths, SafeStore) {
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = StoragePaths::new(temp.path()).expect("paths");
    let store = SafeStore::new(paths.clone());
    store.initialize().expect("initialize safe");
    (temp, paths, store)
}

fn assert_no_storage_secret(text: &str, token: &str, root: &str) {
    assert!(!text.contains(token), "owner token leaked: {text}");
    assert!(!text.contains("owner.proof"), "proof file leaked: {text}");
    assert!(!text.contains("safe.db"), "database file leaked: {text}");
    assert!(!text.contains(root), "storage root leaked: {text}");
}

#[test]
fn empty_prefix_and_extension_tokens_never_authorize() {
    let (_temp, _paths, store) = store();
    let token = store.initialize_owner().expect("owner");
    let prefix = &token[..token.len() - 1];
    let extension = format!("{token}0");

    for candidate in ["", prefix, extension.as_str()] {
        assert!(
            !store.verify_owner(candidate).expect("verify candidate"),
            "candidate must not verify: {candidate:?}"
        );
        let error = store
            .set_stop("tester", "hostile token", candidate)
            .expect_err("set stop must be rejected");
        assert!(matches!(error, SafeError::Unauthorized));
        let text = error.to_string();
        assert!(!text.contains(&token), "owner token leaked: {text}");
        if !candidate.is_empty() {
            assert!(!text.contains(candidate), "candidate leaked: {text}");
        }
    }

    let status = store.stop_status().expect("status");
    assert!(!status.active);
    assert_eq!(status.revision, 0);
}

#[test]
fn wrong_token_cannot_clear_an_active_stop() {
    let (_temp, paths, store) = store();
    let token = store.initialize_owner().expect("owner");
    let set = store
        .set_stop("owner", "lockdown", &token)
        .expect("set stop");
    assert!(set.active);
    assert_eq!(set.revision, 1);

    let audit_before = store.audit_count().expect("audit before");
    let root = paths.root().to_string_lossy().to_string();
    let prefix = &token[..token.len() - 1];

    for candidate in ["", "wrong-token", prefix] {
        let error = store
            .clear_stop("intruder", candidate)
            .expect_err("clear stop must be rejected");
        assert!(matches!(error, SafeError::Unauthorized));
        assert_no_storage_secret(&error.to_string(), &token, &root);
    }

    let status = store.stop_status().expect("status after rejected clears");
    assert!(status.active, "STOP must stay active");
    assert_eq!(status.revision, 1);
    assert_eq!(
        store.audit_count().expect("audit after"),
        audit_before,
        "rejected clears must not append audit events"
    );
    assert!(store.audit_chain_valid().expect("audit chain"));

    let cleared = store.clear_stop("owner", &token).expect("owner clear");
    assert!(!cleared.active);
    assert_eq!(cleared.revision, 2);
}

#[test]
fn removing_owner_proof_fails_closed_and_keeps_stop_active() {
    let (_temp, paths, store) = store();
    let token = store.initialize_owner().expect("owner");
    store
        .set_stop("owner", "lockdown", &token)
        .expect("set stop");
    fs::remove_file(paths.owner_proof()).expect("remove owner proof");
    let root = paths.root().to_string_lossy().to_string();

    let verify = store.verify_owner(&token).expect_err("proof is gone");
    assert!(matches!(verify, SafeError::OwnerNotInitialized));
    assert_no_storage_secret(&verify.to_string(), &token, &root);

    for candidate in [token.as_str(), "attacker"] {
        let error = store
            .set_stop("attacker", "clear lockdown", candidate)
            .expect_err("set stop must fail closed");
        assert!(matches!(error, SafeError::OwnerNotInitialized));
        assert_no_storage_secret(&error.to_string(), &token, &root);
    }
    let clear = store
        .clear_stop("attacker", &token)
        .expect_err("clear stop must fail closed");
    assert!(matches!(clear, SafeError::OwnerNotInitialized));

    let status = store.stop_status().expect("status");
    assert!(status.active, "STOP remains enforced without owner proof");
    assert_eq!(status.revision, 1);
}

#[test]
fn deleted_owner_proof_cannot_be_rotated_by_reinitialization() {
    let (_temp, paths, store) = store();
    let token = store.initialize_owner().expect("owner");
    fs::remove_file(paths.owner_proof()).expect("remove owner proof");

    let error = store
        .initialize_owner()
        .expect_err("owner row already exists");
    assert!(matches!(error, SafeError::OwnerAlreadyInitialized));
    assert!(
        !paths.owner_proof().exists(),
        "a rejected rotation must not leave a new proof file"
    );
    assert!(matches!(
        store.verify_owner(&token),
        Err(SafeError::OwnerNotInitialized)
    ));
}

#[test]
fn audit_chain_detects_forged_rows() {
    let (_temp, paths, store) = store();
    let token = store.initialize_owner().expect("owner");
    store
        .set_stop("owner", "lockdown", &token)
        .expect("set stop");
    store.clear_stop("owner", &token).expect("clear stop");

    let before = store.audit_count().expect("audit count");
    assert!(before >= 4);
    assert!(store.audit_chain_valid().expect("valid chain"));

    let connection = Connection::open(paths.safe_db()).expect("open safe db");
    let changed = connection
        .execute(
            "UPDATE safe_audit SET detail_json = ?1 WHERE sequence = 2",
            ["{\"forged\":true}"],
        )
        .expect("forge audit row");
    assert_eq!(changed, 1);
    drop(connection);

    assert_eq!(store.audit_count().expect("audit count after"), before);
    assert!(
        !store.audit_chain_valid().expect("verify tampered chain"),
        "a rewritten audit row must break the chain"
    );
}

#[test]
fn safe_database_rejects_core_and_memory_markers() {
    for marker in [CORE_APPLICATION_ID, MEMORY_APPLICATION_ID] {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = StoragePaths::new(temp.path()).expect("paths");
        let root = paths.root().to_string_lossy().to_string();
        {
            open_initialized(&paths.safe_db(), marker, "foreign").expect("foreign marker database");
        }

        let store = SafeStore::new(paths);
        let error = store.initialize().expect_err("foreign marker must fail");
        assert!(matches!(
            error,
            SafeError::Storage(StorageError::WrongApplicationId { expected, actual })
                if expected == SAFE_APPLICATION_ID && actual == marker
        ));
        let text = error.to_string();
        assert!(!text.contains("safe.db"), "database file leaked: {text}");
        assert!(!text.contains(&root), "storage root leaked: {text}");
    }
}

#[test]
fn destroying_safe_database_erases_stop_and_audit_documented_limitation() {
    // LIMITATION (Agent 16): STOP and the audit chain live only in safe.db.
    // Nothing outside that file anchors them, so a principal that can delete
    // safe.db and then trigger any initialize() path (for example
    // PlatformService::initialize/start) recreates an inactive STOP and an
    // empty audit history. A read without initialize() fails closed instead.
    let (_temp, paths, store) = store();
    let token = store.initialize_owner().expect("owner");
    store
        .set_stop("owner", "lockdown", &token)
        .expect("set stop");
    let audit_before = store.audit_count().expect("audit before");

    fs::remove_file(paths.safe_db()).expect("delete safe db");

    assert!(
        store.stop_status().is_err(),
        "a missing safe state must not be reported as an inactive STOP"
    );

    store.initialize().expect("reinitialize safe");
    let status = store.stop_status().expect("status after recreate");
    assert!(!status.active, "documented limitation: STOP was erased");
    assert_eq!(status.revision, 0);
    assert!(
        store.audit_count().expect("audit after") < audit_before,
        "documented limitation: audit history was erased"
    );
    assert!(store.audit_chain_valid().expect("fresh chain"));
}

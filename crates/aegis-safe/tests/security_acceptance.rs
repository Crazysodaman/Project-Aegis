use std::fs;

use aegis_safe::{SafeError, SafeStore};
use aegis_storage::{open_initialized, StoragePaths, PLATFORM_APPLICATION_ID, SAFE_APPLICATION_ID};

fn store() -> (tempfile::TempDir, SafeStore) {
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = StoragePaths::new(temp.path()).expect("paths");
    let store = SafeStore::new(paths);
    store.initialize().expect("initialize safe");
    (temp, store)
}

#[test]
fn owner_proof_failure_does_not_echo_candidate_or_real_token() {
    let (_temp, store) = store();
    let token = store.initialize_owner().expect("owner");

    let mut wrong = token.clone();
    let last = wrong.pop().expect("non-empty token");
    wrong.push(if last == '0' { '1' } else { '0' });
    assert!(!store.verify_owner(&wrong).expect("wrong token"));
    assert!(!store
        .verify_owner("definitely-not-the-owner-token")
        .expect("bogus token"));

    let error = store
        .set_stop("tester", "security acceptance", &wrong)
        .expect_err("stop must be rejected");
    assert!(matches!(error, SafeError::Unauthorized));
    let text = error.to_string();
    assert!(!text.contains(&wrong), "candidate token leaked: {text}");
    assert!(!text.contains(&token), "owner token leaked: {text}");

    let clear = store
        .clear_stop("tester", &wrong)
        .expect_err("clear must be rejected");
    assert!(matches!(clear, SafeError::Unauthorized));
    assert!(!clear.to_string().contains(&wrong));
}

#[test]
fn owner_proof_and_database_paths_are_absent_from_errors() {
    let (temp, store) = store();
    let _token = store.initialize_owner().expect("owner");
    let paths = StoragePaths::new(temp.path()).expect("paths");
    let root = paths.root().to_string_lossy().to_string();

    fs::remove_file(paths.owner_proof()).expect("remove owner proof");

    let error = store
        .verify_owner("candidate")
        .expect_err("missing owner proof");
    assert!(matches!(error, SafeError::OwnerNotInitialized));
    let text = error.to_string();
    assert!(!text.contains("owner.proof"), "proof file leaked: {text}");
    assert!(!text.contains("safe.db"), "database file leaked: {text}");
    assert!(!text.contains(&root), "storage root leaked: {text}");
}

#[test]
fn corrupt_owner_proof_failure_is_opaque() {
    let (temp, store) = store();
    let token = store.initialize_owner().expect("owner");
    let paths = StoragePaths::new(temp.path()).expect("paths");
    let root = paths.root().to_string_lossy().to_string();
    let garbage = "not-a-valid-protected-owner-proof";
    fs::write(paths.owner_proof(), garbage.as_bytes()).expect("corrupt proof");

    match store.verify_owner(&token) {
        Ok(valid) => assert!(!valid, "corrupt proof must not verify"),
        Err(error) => {
            let text = error.to_string();
            assert!(!text.contains(&token), "owner token leaked: {text}");
            assert!(!text.contains(garbage), "proof bytes leaked: {text}");
            assert!(!text.contains("owner.proof"), "proof file leaked: {text}");
            assert!(!text.contains(&root), "storage root leaked: {text}");
        }
    }
}

#[test]
fn safe_database_with_foreign_marker_is_rejected() {
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = StoragePaths::new(temp.path()).expect("paths");
    let root = paths.root().to_string_lossy().to_string();
    {
        let foreign = open_initialized(&paths.safe_db(), PLATFORM_APPLICATION_ID, "platform")
            .expect("foreign marker database");
        drop(foreign);
    }

    let store = SafeStore::new(paths);
    let error = store.initialize().expect_err("foreign marker must fail");
    let text = error.to_string();
    assert!(matches!(
        error,
        SafeError::Storage(aegis_storage::StorageError::WrongApplicationId {
            expected,
            actual,
        }) if expected == SAFE_APPLICATION_ID && actual == PLATFORM_APPLICATION_ID
    ));
    assert!(
        text.contains("application identity mismatch"),
        "unexpected message: {text}"
    );
    assert!(!text.contains("safe.db"), "database file leaked: {text}");
    assert!(!text.contains(&root), "storage root leaked: {text}");
}

#[test]
fn owner_token_is_not_persisted_or_audited_in_plaintext() {
    let (temp, store) = store();
    let token = store.initialize_owner().expect("owner");
    store
        .set_stop("owner", "token non-disclosure", &token)
        .expect("set stop");
    store.clear_stop("owner", &token).expect("clear stop");

    let paths = StoragePaths::new(temp.path()).expect("paths");
    let root = paths.root().to_string_lossy().to_string();
    let database = fs::read(paths.safe_db()).expect("read safe db");
    let haystack = String::from_utf8_lossy(&database);
    assert!(
        !haystack.contains(&token),
        "raw owner token was found in safe.db"
    );
    assert!(
        !haystack.contains(&root),
        "absolute storage root was found in safe.db"
    );
    assert!(store.audit_chain_valid().expect("audit chain"));
}

#[test]
fn uninitialized_owner_error_does_not_leak_paths() {
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = StoragePaths::new(temp.path()).expect("paths");
    let root = paths.root().to_string_lossy().to_string();
    let store = SafeStore::new(paths);
    store.initialize().expect("initialize safe");

    let error = store
        .verify_owner("candidate")
        .expect_err("owner proof is absent");
    assert!(matches!(error, SafeError::OwnerNotInitialized));
    let text = error.to_string();
    assert!(!text.contains("owner.proof"), "proof file leaked: {text}");
    assert!(!text.contains("safe.db"), "database file leaked: {text}");
    assert!(!text.contains(&root), "storage root leaked: {text}");

    let load = store.owner_token().expect_err("owner token is absent");
    let load_text = load.to_string();
    assert!(!load_text.contains("owner.proof"));
    assert!(!load_text.contains(&root));
}

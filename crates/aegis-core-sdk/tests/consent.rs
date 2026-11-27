use std::fs;
use std::path::{Path, PathBuf};

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_contracts::living::{ConsentGrant, ConsentScope, LIVING_CONTRACT_VERSION};
use aegis_core_install::CoreInstaller;
use aegis_core_sdk::{
    ConsentStore, CoreSdkContext, SdkError, CONSENT_APPLICATION_ID, CONSENT_DB_FILE,
    CONSENT_EVENTS_TABLE, CONSENT_EXPIRE_ACTION, CONSENT_GRANTS_TABLE, CONSENT_GRANT_ACTION,
    CONSENT_OWNERSHIP_TABLE, CONSENT_REVOKE_ACTION, CONSENT_SCHEMA, CONSENT_SCHEMA_REVISION,
    CONSENT_SCHEMA_TABLE, MAX_CONSENT_LIST,
};
use aegis_core_storage::{CoreDatabase, CoreStorageLayout};
use aegis_platform::PlatformService;
use aegis_storage::now_rfc3339;
use rusqlite::Connection;

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_B: &str = "11111111-1111-4111-8111-111111111111";
const GRANTED_AT: &str = "2026-01-01T00:00:00+00:00";

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
}

fn request(core_uuid: &str, core_number: u8, display_name: &str) -> CoreInitializationRequest {
    CoreInitializationRequest {
        schema: CORE_INIT_REQUEST_SCHEMA.to_owned(),
        core_uuid: core_uuid.to_owned(),
        core_number,
        display_name: display_name.to_owned(),
        requested_at: now_rfc3339().expect("timestamp"),
    }
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().to_path_buf();
    let initialized = PlatformService::initialize(&root).expect("platform initialize");
    let installer = CoreInstaller::new(&root).expect("installer");
    installer
        .initialize(&request(CORE_A, 1, "Core A"), &initialized.owner_token)
        .expect("install A");
    installer
        .initialize(&request(CORE_B, 2, "Core B"), &initialized.owner_token)
        .expect("install B");
    Fixture { _temp: temp, root }
}

fn layout(root: &Path) -> CoreStorageLayout {
    CoreStorageLayout::new(root).expect("layout")
}

fn consent_path(root: &Path, core_uuid: &str) -> PathBuf {
    layout(root)
        .core_dir(core_uuid)
        .expect("core dir")
        .join(CONSENT_DB_FILE)
}

fn sample_grant(
    core_uuid: &str,
    grant_id: &str,
    scope: ConsentScope,
    capability: &str,
    participants: &[&str],
    expires_at: Option<&str>,
) -> ConsentGrant {
    ConsentGrant {
        schema: LIVING_CONTRACT_VERSION.to_owned(),
        grant_id: grant_id.to_owned(),
        core_uuid: core_uuid.to_owned(),
        scope,
        capability: capability.to_owned(),
        participants: participants
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
        granted_at: GRANTED_AT.to_owned(),
        expires_at: expires_at.map(str::to_owned),
        revoked_at: None,
        context: "explicit test consent".to_owned(),
    }
}

fn table_count(connection: &Connection, name: &str) -> i64 {
    connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [name],
            |row| row.get(0),
        )
        .expect("table count")
}

#[test]
fn consent_database_binds_identity_and_schema() {
    let fixture = fixture();
    let snapshot =
        CoreDatabase::read_snapshot(&layout(&fixture.root), CORE_A).expect("core snapshot");
    let store = ConsentStore::open(&fixture.root, CORE_A).expect("consent store");
    assert_eq!(store.core_uuid(), CORE_A);
    assert_eq!(
        store.database_instance_uuid(),
        snapshot.metadata.database_instance_uuid
    );
    assert_eq!(
        store
            .database_path()
            .file_name()
            .and_then(|name| name.to_str()),
        Some(CONSENT_DB_FILE)
    );
    let path = store.database_path().to_path_buf();
    drop(store);

    let connection = Connection::open(&path).expect("raw consent db");
    let application_id: i64 = connection
        .query_row("PRAGMA application_id", [], |row| row.get(0))
        .expect("application id");
    assert_eq!(application_id, CONSENT_APPLICATION_ID);
    let ownership: (String, String, String) = connection
        .query_row(
            "SELECT core_uuid, database_instance_uuid, schema FROM consent_ownership",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("ownership row");
    assert_eq!(ownership.0, CORE_A);
    assert_eq!(ownership.1, snapshot.metadata.database_instance_uuid);
    assert_eq!(ownership.2, CONSENT_SCHEMA);
    let marker: (String, i64) = connection
        .query_row(
            "SELECT schema, revision FROM consent_schema WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("schema marker");
    assert_eq!(marker.0, CONSENT_SCHEMA);
    assert_eq!(marker.1, i64::try_from(CONSENT_SCHEMA_REVISION).unwrap());
    for table in [
        CONSENT_OWNERSHIP_TABLE,
        CONSENT_SCHEMA_TABLE,
        CONSENT_GRANTS_TABLE,
        CONSENT_EVENTS_TABLE,
    ] {
        assert_eq!(table_count(&connection, table), 1, "missing table {table}");
    }
}

#[test]
fn grant_is_active_and_duplicate_or_invalid_is_rejected() {
    let fixture = fixture();
    let store = ConsentStore::open(&fixture.root, CORE_A).expect("store");
    let grant = sample_grant(
        CORE_A,
        "grant-1",
        ConsentScope::Romantic,
        "hold_hands",
        &["owner", "core-b"],
        None,
    );
    let stored = store.grant(&grant).expect("grant");
    assert_eq!(stored, grant);
    assert_eq!(store.get("grant-1").expect("get"), Some(grant.clone()));
    assert_eq!(store.list(10).expect("list"), vec![grant.clone()]);
    assert_eq!(
        store
            .active(None, None, None, "2026-02-01T00:00:00+00:00")
            .expect("active"),
        vec![grant.clone()]
    );
    assert!(store.check(
        ConsentScope::Romantic,
        "hold_hands",
        "owner",
        "2026-02-01T00:00:00+00:00"
    ));

    let events = store.list_events(10).expect("events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].action, CONSENT_GRANT_ACTION);
    assert_eq!(events[0].grant_id, "grant-1");

    assert!(store.grant(&grant).is_err());
    let mut invalid = grant.clone();
    invalid.schema = "not-a-schema".to_owned();
    assert!(store.grant(&invalid).is_err());
    let mut foreign = grant.clone();
    foreign.grant_id = "grant-foreign".to_owned();
    foreign.core_uuid = CORE_B.to_owned();
    assert!(matches!(
        store.grant(&foreign),
        Err(SdkError::OwnershipMismatch)
    ));
    let mut already_revoked = grant.clone();
    already_revoked.grant_id = "grant-revoked".to_owned();
    already_revoked.revoked_at = Some("2026-01-01T01:00:00+00:00".to_owned());
    assert!(store.grant(&already_revoked).is_err());
    let mut bad_expiry = grant.clone();
    bad_expiry.grant_id = "grant-bad-expiry".to_owned();
    bad_expiry.expires_at = Some("2025-12-31T00:00:00+00:00".to_owned());
    assert!(store.grant(&bad_expiry).is_err());
    assert!(store.list(usize::MAX).expect("bounded").len() <= MAX_CONSENT_LIST);
}

#[test]
fn revoke_takes_effect_immediately() {
    let fixture = fixture();
    let store = ConsentStore::open(&fixture.root, CORE_A).expect("store");
    let grant = sample_grant(
        CORE_A,
        "grant-revoke",
        ConsentScope::Affection,
        "hug",
        &["owner"],
        None,
    );
    store.grant(&grant).expect("grant");
    let now = "2026-02-01T00:00:00+00:00";
    assert!(store.check(ConsentScope::Affection, "hug", "owner", now));

    let revoked = store.revoke("grant-revoke").expect("revoke");
    assert!(revoked.revoked_at.is_some());
    assert!(!store.check(ConsentScope::Affection, "hug", "owner", now));
    assert!(store
        .active(None, None, None, now)
        .expect("active")
        .is_empty());
    assert_eq!(
        store
            .get("grant-revoke")
            .expect("get")
            .expect("exists")
            .revoked_at,
        revoked.revoked_at
    );

    let events = store.list_events(10).expect("events");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].action, CONSENT_REVOKE_ACTION);
    assert_eq!(events[1].action, CONSENT_GRANT_ACTION);
    assert!(matches!(
        store.revoke("missing-grant"),
        Err(SdkError::NotFound)
    ));

    let connection = Connection::open(consent_path(&fixture.root, CORE_A)).expect("raw db");
    let revision: i64 = connection
        .query_row(
            "SELECT revision FROM consent_grants WHERE grant_id = 'grant-revoke'",
            [],
            |row| row.get(0),
        )
        .expect("revision");
    assert_eq!(revision, 2);
}

#[test]
fn expiry_makes_checks_false_and_records_event() {
    let fixture = fixture();
    let store = ConsentStore::open(&fixture.root, CORE_A).expect("store");
    let grant = sample_grant(
        CORE_A,
        "grant-expiry",
        ConsentScope::SharedProject,
        "co_edit",
        &["owner"],
        Some("2026-01-02T00:00:00+00:00"),
    );
    store.grant(&grant).expect("grant");
    assert!(store.check(
        ConsentScope::SharedProject,
        "co_edit",
        "owner",
        "2026-01-01T12:00:00+00:00"
    ));

    let at_expiry = "2026-01-02T00:00:00+00:00";
    assert!(!store.check(ConsentScope::SharedProject, "co_edit", "owner", at_expiry));
    assert!(store
        .active(None, None, None, at_expiry)
        .expect("active")
        .is_empty());
    assert!(store
        .active(None, None, None, "2026-01-03T00:00:00+00:00")
        .expect("active later")
        .is_empty());
    let events = store.list_events(20).expect("events");
    let expired = events
        .iter()
        .filter(|event| event.action == CONSENT_EXPIRE_ACTION)
        .count();
    assert_eq!(expired, 1, "expiry event is recorded once");
    assert_eq!(
        store
            .get("grant-expiry")
            .expect("get")
            .expect("exists")
            .revoked_at,
        None
    );
}

#[test]
fn scope_capability_and_participant_matching_is_exact() {
    let fixture = fixture();
    let store = ConsentStore::open(&fixture.root, CORE_A).expect("store");
    let grant = sample_grant(
        CORE_A,
        "grant-match",
        ConsentScope::Object,
        "borrow_book",
        &["owner", "core-b"],
        None,
    );
    store.grant(&grant).expect("grant");
    let now = "2026-02-01T00:00:00+00:00";

    assert!(store.check(ConsentScope::Object, "borrow_book", "core-b", now));
    assert!(!store.check(ConsentScope::Adult, "borrow_book", "core-b", now));
    assert!(!store.check(ConsentScope::Object, "burn_book", "core-b", now));
    assert!(!store.check(ConsentScope::Object, "borrow_book", "stranger", now));

    assert_eq!(
        store
            .active(
                Some(ConsentScope::Object),
                Some("borrow_book"),
                Some("owner"),
                now
            )
            .expect("active")
            .len(),
        1
    );
    assert!(store
        .active(Some(ConsentScope::Conversation), None, None, now)
        .expect("scope filter")
        .is_empty());
    assert!(store
        .active(None, Some("other"), None, now)
        .expect("capability filter")
        .is_empty());
    assert!(store
        .active(None, None, Some("stranger"), now)
        .expect("participant filter")
        .is_empty());
}

#[test]
fn consent_is_isolated_per_core() {
    let fixture = fixture();
    let store_a = ConsentStore::open(&fixture.root, CORE_A).expect("A store");
    let store_b = ConsentStore::open(&fixture.root, CORE_B).expect("B store");
    let grant = sample_grant(
        CORE_A,
        "grant-isolation",
        ConsentScope::Conversation,
        "chat",
        &["owner"],
        None,
    );
    store_a.grant(&grant).expect("grant");

    assert!(store_b.get("grant-isolation").expect("get").is_none());
    assert!(store_b.list(10).expect("list").is_empty());
    assert!(!store_b.check(
        ConsentScope::Conversation,
        "chat",
        "owner",
        "2026-02-01T00:00:00+00:00"
    ));
    assert!(store_b.list_events(10).expect("events").is_empty());

    let mut wrong_core = grant.clone();
    wrong_core.grant_id = "grant-wrong-core".to_owned();
    assert!(matches!(
        store_b.grant(&wrong_core),
        Err(SdkError::OwnershipMismatch)
    ));
}

#[test]
fn foreign_consent_database_copy_is_rejected() {
    let fixture = fixture();
    {
        let store = ConsentStore::open(&fixture.root, CORE_A).expect("A store");
        store
            .grant(&sample_grant(
                CORE_A,
                "grant-copy",
                ConsentScope::Adult,
                "explicit_adult_consent",
                &["owner"],
                None,
            ))
            .expect("grant");
    }
    let a_path = consent_path(&fixture.root, CORE_A);
    let b_path = consent_path(&fixture.root, CORE_B);
    assert!(!b_path.exists());
    fs::copy(&a_path, &b_path).expect("copy A consent over B");

    let copied = ConsentStore::open(&fixture.root, CORE_B).expect_err("copied ownership");
    assert!(matches!(copied, SdkError::OwnershipMismatch));
    let reopened = ConsentStore::open(&fixture.root, CORE_A).expect("A still opens");
    assert!(reopened.get("grant-copy").expect("get").is_some());
}

#[test]
fn consent_events_are_append_only() {
    let fixture = fixture();
    {
        let store = ConsentStore::open(&fixture.root, CORE_A).expect("store");
        store
            .grant(&sample_grant(
                CORE_A,
                "grant-events",
                ConsentScope::Romantic,
                "hold_hands",
                &["owner"],
                None,
            ))
            .expect("grant");
        store.revoke("grant-events").expect("revoke");
    }

    let connection = Connection::open(consent_path(&fixture.root, CORE_A)).expect("raw db");
    for statement in [
        "UPDATE consent_events SET action = 'tampered'",
        "DELETE FROM consent_events",
        "UPDATE consent_ownership SET core_uuid = 'tampered'",
        "DELETE FROM consent_ownership",
    ] {
        assert!(
            connection.execute(statement, []).is_err(),
            "statement must be rejected: {statement}"
        );
    }
    let events: i64 = connection
        .query_row("SELECT COUNT(*) FROM consent_events", [], |row| row.get(0))
        .expect("event count");
    assert_eq!(events, 2);
    let ownership: i64 = connection
        .query_row("SELECT COUNT(*) FROM consent_ownership", [], |row| {
            row.get(0)
        })
        .expect("ownership count");
    assert_eq!(ownership, 1);
}

#[test]
fn grants_and_events_survive_reopen() {
    let fixture = fixture();
    let grant = sample_grant(
        CORE_A,
        "grant-persist",
        ConsentScope::Object,
        "share_photo",
        &["owner", "core-b"],
        Some("2026-06-01T00:00:00+00:00"),
    );
    {
        let store = ConsentStore::open(&fixture.root, CORE_A).expect("store");
        store.grant(&grant).expect("grant");
    }

    let reopened = ConsentStore::open(&fixture.root, CORE_A).expect("reopen");
    assert_eq!(reopened.get("grant-persist").expect("get"), Some(grant));
    assert_eq!(reopened.list(10).expect("list").len(), 1);
    let events = reopened.list_events(10).expect("events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].action, CONSENT_GRANT_ACTION);

    let context = CoreSdkContext::open(&fixture.root, CORE_A).expect("context");
    let consent = context.consent().expect("consent accessor");
    assert_eq!(consent.core_uuid(), CORE_A);
    assert!(consent.get("grant-persist").expect("get").is_some());
}

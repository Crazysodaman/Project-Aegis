use std::path::{Path, PathBuf};

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_core_install::CoreInstaller;
use aegis_platform::PlatformService;
use aegis_social::{
    social_awareness_summary, social_awareness_summary_limited, GroupStore, MemberKind,
    RelationshipStore, RelationshipTargetKind, SocialActor, SocialError,
};
use aegis_storage::{now_rfc3339, StoragePaths};

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_B: &str = "11111111-1111-4111-8111-111111111111";

fn setup() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("aegis-root");
    let initialized = PlatformService::initialize(&root).expect("platform");
    let installer = CoreInstaller::new(&root).expect("installer");
    for (uuid, number, name) in [(CORE_A, 1, "Sofía Ada Lyra"), (CORE_B, 2, "Mack Fuller")] {
        installer
            .initialize(
                &CoreInitializationRequest {
                    schema: CORE_INIT_REQUEST_SCHEMA.to_owned(),
                    core_uuid: uuid.to_owned(),
                    core_number: number,
                    display_name: name.to_owned(),
                    requested_at: now_rfc3339().expect("time"),
                },
                &initialized.owner_token,
            )
            .expect("core");
    }
    (temp, root)
}

fn groups(root: &Path) -> GroupStore {
    let store = GroupStore::new(StoragePaths::new(root).expect("paths"));
    store.initialize().expect("groups");
    store
}

fn owner() -> SocialActor {
    SocialActor::owner()
}

fn core(uuid: &str) -> SocialActor {
    SocialActor::core(uuid).expect("core actor")
}

#[test]
fn explicit_relationship_events_are_append_only_and_revisioned() {
    let (_temp, root) = setup();
    let store = RelationshipStore::open(&root, CORE_A).expect("store");
    let relationship = store
        .upsert(
            RelationshipTargetKind::Core,
            CORE_B,
            "Mack",
            50,
            "",
            Vec::new(),
        )
        .expect("relationship");
    assert_eq!(relationship.revision, 1);

    let first = store
        .record_event(
            RelationshipTargetKind::Core,
            CORE_B,
            "interaction.shared_activity",
            serde_json::json!({"activity": "walk", "minutes": 15}),
        )
        .expect("first explicit event");
    assert_eq!(first.schema, "aegis.social.relationship_event.v1");
    assert_eq!(first.relationship_id, relationship.relationship_id);
    assert_eq!(first.revision, 2);
    assert_eq!(first.detail["activity"], "walk");

    let second = store
        .record_event(
            RelationshipTargetKind::Core,
            CORE_B,
            "interaction.conversation",
            serde_json::json!({"topic": "weather"}),
        )
        .expect("second explicit event");
    assert_eq!(second.revision, 3);
    assert_eq!(
        store
            .relationship(&relationship.relationship_id)
            .expect("relationship")
            .revision,
        3
    );

    let events = store
        .events(&relationship.relationship_id)
        .expect("event history");
    assert_eq!(events.len(), 3);
    assert_eq!(
        events
            .iter()
            .map(|event| event.revision)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(events[1].event_type, "interaction.shared_activity");
    assert_eq!(events[2].detail["topic"], "weather");

    let path = store.path().to_path_buf();
    drop(store);
    let connection = rusqlite::Connection::open(&path).expect("raw sqlite");
    assert!(connection
        .execute(
            "UPDATE relationship_events SET event_type = 'rewritten'",
            []
        )
        .is_err());
    assert!(connection
        .execute("DELETE FROM relationship_events", [])
        .is_err());
}

#[test]
fn explicit_relationship_events_validate_and_require_a_relationship() {
    let (_temp, root) = setup();
    let store = RelationshipStore::open(&root, CORE_A).expect("store");
    assert!(matches!(
        store.record_event(
            RelationshipTargetKind::Person,
            "nobody",
            "interaction.seen",
            serde_json::json!({}),
        ),
        Err(SocialError::MemberNotFound)
    ));

    store
        .upsert(
            RelationshipTargetKind::Core,
            CORE_B,
            "Mack",
            50,
            "",
            Vec::new(),
        )
        .expect("relationship");
    assert!(matches!(
        store.record_event(
            RelationshipTargetKind::Core,
            CORE_B,
            " ",
            serde_json::json!({}),
        ),
        Err(SocialError::InvalidInput("event type"))
    ));
    assert!(matches!(
        store.record_event(
            RelationshipTargetKind::Core,
            CORE_B,
            "bad\nkind",
            serde_json::json!({}),
        ),
        Err(SocialError::InvalidInput("event type"))
    ));
    let long_type = "x".repeat(65);
    assert!(matches!(
        store.record_event(
            RelationshipTargetKind::Core,
            CORE_B,
            &long_type,
            serde_json::json!({}),
        ),
        Err(SocialError::InvalidInput("event type"))
    ));
    let long_detail = serde_json::json!({"payload": "x".repeat(9 * 1024)});
    assert!(matches!(
        store.record_event(
            RelationshipTargetKind::Core,
            CORE_B,
            "interaction.large",
            long_detail,
        ),
        Err(SocialError::InvalidInput("event detail"))
    ));
}

#[test]
fn relationship_event_histories_are_independent_per_core() {
    let (_temp, root) = setup();
    let a_store = RelationshipStore::open(&root, CORE_A).expect("a store");
    let a_relationship = a_store
        .upsert(
            RelationshipTargetKind::Core,
            CORE_B,
            "A-PRIVATE-LABEL",
            88,
            "A-PRIVATE-NOTE",
            vec!["A-PRIVATE-TAG".to_owned()],
        )
        .expect("a relationship");
    let a_event = a_store
        .record_event(
            RelationshipTargetKind::Core,
            CORE_B,
            "a.event",
            serde_json::json!({"marker": "A-PRIVATE-EVENT"}),
        )
        .expect("a event");
    assert_eq!(a_event.revision, 2);

    let b_store = RelationshipStore::open(&root, CORE_B).expect("b store");
    assert!(b_store.list().expect("b relationships").is_empty());
    assert!(b_store
        .events(&a_relationship.relationship_id)
        .expect("b view of a history")
        .is_empty());

    let b_relationship = b_store
        .upsert(
            RelationshipTargetKind::Core,
            CORE_A,
            "B-PRIVATE-LABEL",
            12,
            "B-PRIVATE-NOTE",
            vec!["B-PRIVATE-TAG".to_owned()],
        )
        .expect("b relationship");
    let b_event = b_store
        .record_event(
            RelationshipTargetKind::Core,
            CORE_A,
            "b.event",
            serde_json::json!({"marker": "B-PRIVATE-EVENT"}),
        )
        .expect("b event");
    assert_eq!(b_event.revision, 2);
    assert_ne!(
        a_relationship.relationship_id,
        b_relationship.relationship_id
    );

    let a_history = serde_json::to_string(
        &a_store
            .events(&a_relationship.relationship_id)
            .expect("a history"),
    )
    .expect("a json");
    assert!(a_history.contains("A-PRIVATE-EVENT"));
    assert!(!a_history.contains("B-PRIVATE-EVENT"));

    let b_history = serde_json::to_string(
        &b_store
            .events(&b_relationship.relationship_id)
            .expect("b history"),
    )
    .expect("b json");
    assert!(b_history.contains("B-PRIVATE-EVENT"));
    assert!(!b_history.contains("A-PRIVATE-EVENT"));

    assert_eq!(a_store.list().expect("a list").len(), 1);
    assert_eq!(b_store.list().expect("b list").len(), 1);
}

#[test]
fn awareness_summary_is_membership_scoped_and_non_members_are_denied() {
    let (_temp, root) = setup();
    let a_store = RelationshipStore::open(&root, CORE_A).expect("a store");
    a_store
        .upsert(
            RelationshipTargetKind::Core,
            CORE_B,
            "A-OWN-LABEL",
            70,
            "A-PRIVATE-NOTE",
            vec!["A-OWN-TAG".to_owned()],
        )
        .expect("a relationship");

    let b_store = RelationshipStore::open(&root, CORE_B).expect("b store");
    b_store
        .upsert(
            RelationshipTargetKind::Core,
            CORE_A,
            "B-PRIVATE-LABEL",
            20,
            "B-PRIVATE-NOTE",
            vec!["B-PRIVATE-TAG".to_owned()],
        )
        .expect("b relationship");

    let store = groups(&root);
    let a_group = store.create_group("A room", &owner()).expect("a group");
    let b_group = store.create_group("B room", &owner()).expect("b group");
    store
        .add_member(&a_group.group_id, MemberKind::Core, CORE_A, &owner())
        .expect("add a");
    store
        .add_member(&b_group.group_id, MemberKind::Core, CORE_B, &owner())
        .expect("add b");
    store
        .post_message(&a_group.group_id, &core(CORE_A), "A-SHARED-CONTENT")
        .expect("a post");
    store
        .post_message(&b_group.group_id, &core(CORE_B), "B-SHARED-CONTENT")
        .expect("b post");

    assert!(matches!(
        store.context_for_core(CORE_A, &b_group.group_id, 10),
        Err(SocialError::NotMember)
    ));
    assert!(matches!(
        store.messages(&b_group.group_id, &core(CORE_A), 10, 0),
        Err(SocialError::NotMember)
    ));
    assert!(matches!(
        store.activity_summary(&b_group.group_id, &core(CORE_A)),
        Err(SocialError::NotMember)
    ));

    let summary = social_awareness_summary(&store, &a_store).expect("a summary");
    assert_eq!(summary.schema, "aegis.social.awareness_summary.v1");
    assert_eq!(summary.core_uuid, CORE_A);
    assert_eq!(summary.membership_count, 1);
    assert_eq!(summary.relationship_count, 1);
    assert!(!summary.memberships_truncated);
    assert!(!summary.relationships_truncated);
    assert_eq!(summary.memberships.len(), 1);
    assert_eq!(summary.memberships[0].group_id, a_group.group_id);
    assert!(!summary
        .memberships
        .iter()
        .any(|entry| entry.group_id == b_group.group_id));
    assert_eq!(summary.relationships.len(), 1);
    assert_eq!(summary.relationships[0].target_id, CORE_B);
    assert_eq!(summary.relationships[0].label, "A-OWN-LABEL");
    assert_eq!(summary.relationships[0].trust, 70);

    let encoded = serde_json::to_string(&summary).expect("summary json");
    assert!(!encoded.contains("B-PRIVATE-LABEL"));
    assert!(!encoded.contains("B-PRIVATE-NOTE"));
    assert!(!encoded.contains("B-PRIVATE-TAG"));
    assert!(!encoded.contains("B-SHARED-CONTENT"));
    assert!(!encoded.contains("A-PRIVATE-NOTE"));
    assert!(!encoded.contains("\"notes\""));
}

#[test]
fn awareness_summaries_never_include_other_cores_private_relationships() {
    let (_temp, root) = setup();
    let a_store = RelationshipStore::open(&root, CORE_A).expect("a store");
    a_store
        .upsert(
            RelationshipTargetKind::Core,
            CORE_B,
            "A-PRIVATE-LABEL",
            91,
            "A-PRIVATE-NOTE",
            vec!["A-PRIVATE-TAG".to_owned()],
        )
        .expect("a relationship");
    let b_store = RelationshipStore::open(&root, CORE_B).expect("b store");
    b_store
        .upsert(
            RelationshipTargetKind::Core,
            CORE_A,
            "B-PRIVATE-LABEL",
            11,
            "B-PRIVATE-NOTE",
            vec!["B-PRIVATE-TAG".to_owned()],
        )
        .expect("b relationship");

    let store = groups(&root);
    let shared = store
        .create_group("Shared", &owner())
        .expect("shared group");
    for member in [CORE_A, CORE_B] {
        store
            .add_member(&shared.group_id, MemberKind::Core, member, &owner())
            .expect("add member");
    }
    store
        .post_message(&shared.group_id, &core(CORE_A), "shared message")
        .expect("post");

    let a_summary = social_awareness_summary(&store, &a_store).expect("a summary");
    let a_json = serde_json::to_string(&a_summary).expect("a json");
    assert!(a_json.contains("A-PRIVATE-LABEL"));
    assert!(!a_json.contains("B-PRIVATE-LABEL"));
    assert!(!a_json.contains("B-PRIVATE-NOTE"));
    assert!(!a_json.contains("B-PRIVATE-TAG"));
    assert!(!a_json.contains("A-PRIVATE-NOTE"));
    assert!(!a_json.contains("\"notes\""));

    let b_summary = social_awareness_summary(&store, &b_store).expect("b summary");
    let b_json = serde_json::to_string(&b_summary).expect("b json");
    assert!(b_json.contains("B-PRIVATE-LABEL"));
    assert!(!b_json.contains("A-PRIVATE-LABEL"));
    assert!(!b_json.contains("A-PRIVATE-NOTE"));
    assert!(!b_json.contains("A-PRIVATE-TAG"));
    assert!(!b_json.contains("\"notes\""));

    assert_eq!(a_summary.core_uuid, CORE_A);
    assert_eq!(b_summary.core_uuid, CORE_B);
}

#[test]
fn awareness_summary_reports_only_recorded_data_and_is_bounded() {
    let (_temp, root) = setup();
    let store = groups(&root);
    let a_store = RelationshipStore::open(&root, CORE_A).expect("a store");

    let empty = social_awareness_summary(&store, &a_store).expect("empty summary");
    assert_eq!(empty.membership_count, 0);
    assert_eq!(empty.relationship_count, 0);
    assert!(empty.memberships.is_empty());
    assert!(empty.relationships.is_empty());
    assert!(!empty.memberships_truncated);
    assert!(!empty.relationships_truncated);

    for index in 0..3 {
        let group = store
            .create_group(&format!("Group {index}"), &owner())
            .expect("group");
        store
            .add_member(&group.group_id, MemberKind::Core, CORE_A, &owner())
            .expect("add a");
    }
    for (kind, target) in [
        (RelationshipTargetKind::Core, CORE_B),
        (RelationshipTargetKind::Person, "person-1"),
        (RelationshipTargetKind::Person, "person-2"),
    ] {
        a_store
            .upsert(kind, target, "label", 10, "", Vec::new())
            .expect("relationship");
    }

    let limited =
        social_awareness_summary_limited(&store, &a_store, 1, 2).expect("limited summary");
    assert_eq!(limited.membership_limit, 1);
    assert_eq!(limited.relationship_limit, 2);
    assert_eq!(limited.membership_count, 3);
    assert_eq!(limited.relationship_count, 3);
    assert_eq!(limited.memberships.len(), 1);
    assert_eq!(limited.relationships.len(), 2);
    assert!(limited.memberships_truncated);
    assert!(limited.relationships_truncated);

    let full = social_awareness_summary(&store, &a_store).expect("full summary");
    assert_eq!(full.memberships.len(), 3);
    assert_eq!(full.relationships.len(), 3);
    assert!(!full.memberships_truncated);
    assert!(!full.relationships_truncated);
    assert_eq!(
        full.membership_limit,
        aegis_social::MAX_AWARENESS_MEMBERSHIPS
    );
    assert_eq!(
        full.relationship_limit,
        aegis_social::MAX_AWARENESS_RELATIONSHIPS
    );

    let clamped =
        social_awareness_summary_limited(&store, &a_store, 0, usize::MAX).expect("clamped");
    assert_eq!(clamped.membership_limit, 1);
    assert_eq!(
        clamped.relationship_limit,
        aegis_social::MAX_AWARENESS_RELATIONSHIPS
    );
    assert_eq!(clamped.memberships.len(), 1);
    assert_eq!(clamped.relationships.len(), 3);
}

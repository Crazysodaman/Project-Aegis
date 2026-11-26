use std::path::PathBuf;

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_core_install::CoreInstaller;
use aegis_platform::PlatformService;
use aegis_social::{
    GroupStore, MemberKind, RelationshipStore, RelationshipTargetKind, SocialActor, SocialError,
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

fn groups(root: &std::path::Path) -> GroupStore {
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
fn group_membership_is_enforced_and_revocation_is_immediate() {
    let (_temp, root) = setup();
    let store = groups(&root);
    let group = store
        .create_group("Owner and two Cores", &owner())
        .expect("group");
    assert_eq!(group.member_count, 1);
    store
        .add_member(&group.group_id, MemberKind::Core, CORE_A, &owner())
        .expect("add a");
    store
        .add_member(&group.group_id, MemberKind::Core, CORE_B, &owner())
        .expect("add b");

    assert!(store
        .is_member(&group.group_id, &core(CORE_A))
        .expect("member a"));
    let a = core(CORE_A);
    let b = core(CORE_B);
    store
        .post_message(&group.group_id, &a, "Hello from Sofía.")
        .expect("post a");
    let messages = store
        .messages(&group.group_id, &b, 20, 0)
        .expect("b messages");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].sender_id, CORE_A);
    assert_eq!(messages[0].sender_kind, MemberKind::Core);

    store
        .remove_member(&group.group_id, MemberKind::Core, CORE_A, &owner())
        .expect("remove a");
    assert!(!store
        .is_member(&group.group_id, &a)
        .expect("member a after"));
    assert!(matches!(
        store.messages(&group.group_id, &a, 20, 0),
        Err(SocialError::NotMember)
    ));
    assert!(matches!(
        store.post_message(&group.group_id, &a, "still here?"),
        Err(SocialError::NotMember)
    ));
    assert_eq!(
        store
            .messages(&group.group_id, &b, 20, 0)
            .expect("b still member")
            .len(),
        1
    );
    let events = store.events(&group.group_id, &owner()).expect("events");
    assert!(events
        .iter()
        .any(|event| event.event_type == "member.removed"));
}

#[test]
fn non_members_cannot_see_or_read_private_groups() {
    let (_temp, root) = setup();
    let store = groups(&root);
    let group = store.create_group("Private", &owner()).expect("group");
    store
        .add_member(&group.group_id, MemberKind::Core, CORE_A, &owner())
        .expect("add");
    let b = core(CORE_B);
    assert!(store.list_groups(&b).expect("list").is_empty());
    assert!(matches!(
        store.group(&group.group_id, &b),
        Err(SocialError::NotMember)
    ));
    assert!(matches!(
        store.messages(&group.group_id, &b, 10, 0),
        Err(SocialError::NotMember)
    ));
    assert!(matches!(
        store.post_message(&group.group_id, &b, "intrusion"),
        Err(SocialError::NotMember)
    ));
    assert_eq!(store.list_groups(&core(CORE_A)).expect("list a").len(), 1);
}

#[test]
fn core_cannot_administer_membership() {
    let (_temp, root) = setup();
    let store = groups(&root);
    let group = store
        .create_group("Core-created", &core(CORE_A))
        .expect("group");
    assert_eq!(group.created_by, CORE_A);
    assert!(matches!(
        store.add_member(&group.group_id, MemberKind::Core, CORE_B, &core(CORE_A)),
        Err(SocialError::NotAuthorized)
    ));
    // The owner retains administrative access even without membership.
    let members = store
        .members(&group.group_id, &owner())
        .expect("owner read");
    assert_eq!(members.len(), 1);
    store
        .post_message(&group.group_id, &owner(), "Owner notice")
        .expect("owner post");
}

#[test]
fn duplicate_and_missing_members_are_reported() {
    let (_temp, root) = setup();
    let store = groups(&root);
    let group = store.create_group("Roster", &owner()).expect("group");
    store
        .add_member(&group.group_id, MemberKind::Core, CORE_A, &owner())
        .expect("add");
    assert!(matches!(
        store.add_member(&group.group_id, MemberKind::Core, CORE_A, &owner()),
        Err(SocialError::DuplicateMember)
    ));
    assert!(matches!(
        store.remove_member(&group.group_id, MemberKind::Core, CORE_B, &owner()),
        Err(SocialError::MemberNotFound)
    ));
    store
        .add_member(
            &group.group_id,
            MemberKind::Core,
            &CORE_B.to_uppercase(),
            &owner(),
        )
        .expect("canonical member");
    let members = store.members(&group.group_id, &owner()).expect("members");
    assert!(members.iter().any(|member| member.member_id == CORE_B));
}

#[test]
fn groups_and_messages_persist_across_reopen() {
    let (_temp, root) = setup();
    let store = groups(&root);
    let group = store.create_group("Persistent", &owner()).expect("group");
    store
        .add_member(&group.group_id, MemberKind::Core, CORE_A, &owner())
        .expect("add");
    store
        .post_message(&group.group_id, &core(CORE_A), "Remember this.")
        .expect("post");
    drop(store);
    let reopened = groups(&root);
    let messages = reopened
        .messages(&group.group_id, &core(CORE_A), 10, 0)
        .expect("messages");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, "Remember this.");
    assert_eq!(
        reopened
            .group(&group.group_id, &owner())
            .expect("group")
            .member_count,
        2
    );
}

#[test]
fn owner_sees_all_groups_cores_see_joined_only() {
    let (_temp, root) = setup();
    let store = groups(&root);
    let first = store.create_group("First", &owner()).expect("first");
    let second = store.create_group("Second", &owner()).expect("second");
    store
        .add_member(&first.group_id, MemberKind::Core, CORE_A, &owner())
        .expect("add a");
    store
        .add_member(&second.group_id, MemberKind::Core, CORE_B, &owner())
        .expect("add b");
    assert_eq!(store.list_groups(&owner()).expect("owner").len(), 2);
    let a_groups = store.list_groups(&core(CORE_A)).expect("a");
    assert_eq!(a_groups.len(), 1);
    assert_eq!(a_groups[0].group_id, first.group_id);
}

#[test]
fn relationships_are_private_persistent_and_versioned() {
    let (_temp, root) = setup();
    let store = RelationshipStore::open(&root, CORE_A).expect("a store");
    let relationship = store
        .upsert(
            RelationshipTargetKind::Core,
            CORE_B,
            "Mack",
            40,
            "Friendly but still guarded.",
            vec!["friend".to_owned()],
        )
        .expect("create relationship");
    assert_eq!(relationship.revision, 1);
    let updated = store
        .upsert(
            RelationshipTargetKind::Core,
            CORE_B,
            "Mack Fuller",
            75,
            "Trusted colleague.",
            vec!["friend".to_owned(), "trusted".to_owned()],
        )
        .expect("update");
    assert_eq!(updated.relationship_id, relationship.relationship_id);
    assert_eq!(updated.revision, 2);
    assert_eq!(updated.trust, 75);
    assert_eq!(
        store
            .events(&updated.relationship_id)
            .expect("events")
            .len(),
        2
    );

    let b_store = RelationshipStore::open(&root, CORE_B).expect("b store");
    assert!(b_store.list().expect("b list").is_empty());
    drop(b_store);
    assert_eq!(store.list().expect("list").len(), 1);
    drop(store);

    let reopened = RelationshipStore::open(&root, CORE_A).expect("reopen");
    assert_eq!(reopened.list().expect("list").len(), 1);

    let source = root.join("cores").join(CORE_A).join("social.db");
    let target = root.join("cores").join(CORE_B).join("social.db");
    std::fs::copy(&source, &target).expect("copy");
    assert!(matches!(
        RelationshipStore::open(&root, CORE_B),
        Err(SocialError::OwnershipMismatch)
    ));
}

#[test]
fn relationship_delete_requires_existing_target() {
    let (_temp, root) = setup();
    let store = RelationshipStore::open(&root, CORE_A).expect("store");
    assert!(!store
        .delete(RelationshipTargetKind::Person, "nobody")
        .expect("delete missing"));
    store
        .upsert(
            RelationshipTargetKind::Person,
            "owner",
            "Owner",
            100,
            "",
            Vec::new(),
        )
        .expect("owner relationship");
    assert!(store
        .delete(RelationshipTargetKind::Person, "owner")
        .expect("delete"));
    assert!(store.list().expect("list").is_empty());
}

#[test]
fn core_context_is_membership_scoped_and_recent() {
    let (_temp, root) = setup();
    let store = groups(&root);
    let group = store
        .create_group("Context window", &owner())
        .expect("group");
    store
        .add_member(&group.group_id, MemberKind::Core, CORE_A, &owner())
        .expect("add a");

    store
        .post_message(&group.group_id, &owner(), "Owner first.")
        .expect("owner post");
    store
        .post_message(&group.group_id, &core(CORE_A), "Sofia second.")
        .expect("core post");

    assert!(matches!(
        store.context_for_core(CORE_B, &group.group_id, 10),
        Err(SocialError::NotMember)
    ));

    let limited = store
        .context_for_core(CORE_A, &group.group_id, 1)
        .expect("member context");
    assert_eq!(limited.schema, "aegis.social.core_context.v1");
    assert_eq!(limited.core_uuid, CORE_A);
    assert_eq!(limited.messages.len(), 1);
    assert_eq!(limited.messages[0].sender_kind, MemberKind::Core);
    assert_eq!(limited.messages[0].sender_id, CORE_A);
    assert_eq!(limited.messages[0].content, "Sofia second.");

    store
        .add_member(&group.group_id, MemberKind::Core, CORE_B, &owner())
        .expect("add b");
    let recent = store
        .context_for_core(CORE_B, &group.group_id, 2)
        .expect("b context");
    assert_eq!(recent.messages.len(), 2);
    assert_eq!(recent.messages[0].content, "Owner first.");
    assert_eq!(recent.messages[1].content, "Sofia second.");

    store
        .remove_member(&group.group_id, MemberKind::Core, CORE_A, &owner())
        .expect("remove a");
    assert!(matches!(
        store.context_for_core(CORE_A, &group.group_id, 10),
        Err(SocialError::NotMember)
    ));

    let bounded = store
        .context_for_core(CORE_B, &group.group_id, usize::MAX)
        .expect("bounded context");
    assert_eq!(bounded.messages.len(), 2);
    let clamped = store
        .context_for_core(CORE_B, &group.group_id, 0)
        .expect("clamped context");
    assert_eq!(clamped.messages.len(), 1);
}

#[test]
fn group_context_never_exposes_private_relationships() {
    let (_temp, root) = setup();
    let relationships = RelationshipStore::open(&root, CORE_A).expect("relationship store");
    relationships
        .upsert(
            RelationshipTargetKind::Core,
            CORE_B,
            "PrivateLabelA",
            91,
            "PRIVATE-NOTE-A",
            vec!["PRIVATE-TAG-A".to_owned()],
        )
        .expect("private relationship");

    let store = groups(&root);
    let group = store.create_group("Shared room", &owner()).expect("group");
    for member in [CORE_A, CORE_B] {
        store
            .add_member(&group.group_id, MemberKind::Core, member, &owner())
            .expect("add member");
    }
    store
        .post_message(&group.group_id, &core(CORE_A), "Shared group message.")
        .expect("post");

    for reader in [CORE_A, CORE_B] {
        let context = store
            .context_for_core(reader, &group.group_id, 50)
            .expect("context");
        assert_eq!(context.messages.len(), 1);
        assert_eq!(context.messages[0].content, "Shared group message.");
        let encoded = serde_json::to_string(&context).expect("context json");
        assert!(!encoded.contains("PRIVATE-NOTE-A"));
        assert!(!encoded.contains("PrivateLabelA"));
        assert!(!encoded.contains("PRIVATE-TAG-A"));
        assert!(!encoded.contains("trust"));

        let summary = store
            .activity_summary(&group.group_id, &core(reader))
            .expect("summary");
        let encoded = serde_json::to_string(&summary).expect("summary json");
        assert!(!encoded.contains("PRIVATE-NOTE-A"));
        assert!(!encoded.contains("PrivateLabelA"));
        assert!(!encoded.contains("trust"));
    }
}

#[test]
fn group_activity_summary_is_bounded_and_member_scoped() {
    let (_temp, root) = setup();
    let store = groups(&root);
    let group = store.create_group("Activity", &owner()).expect("group");
    store
        .add_member(&group.group_id, MemberKind::Core, CORE_A, &owner())
        .expect("add a");
    for (index, actor) in [owner(), core(CORE_A), owner()].into_iter().enumerate() {
        store
            .post_message(
                &group.group_id,
                &actor,
                &format!("Activity message {index}"),
            )
            .expect("post");
    }

    let summary = store
        .activity_summary(&group.group_id, &core(CORE_A))
        .expect("member summary");
    assert_eq!(summary.schema, "aegis.social.group_activity.v1");
    assert_eq!(summary.member_count, 2);
    assert_eq!(summary.message_count, 3);
    assert!(summary.last_message_id.is_some());
    assert!(summary.last_message_at.is_some());
    assert_eq!(
        summary.last_message_at,
        Some(summary.last_activity_at.clone())
    );
    assert!(summary.revision >= 4);

    assert!(matches!(
        store.activity_summary(&group.group_id, &core(CORE_B)),
        Err(SocialError::NotMember)
    ));
    assert_eq!(
        store
            .activity_summary(&group.group_id, &owner())
            .expect("owner summary")
            .message_count,
        3
    );

    store
        .remove_member(&group.group_id, MemberKind::Core, CORE_A, &owner())
        .expect("remove a");
    assert!(matches!(
        store.activity_summary(&group.group_id, &core(CORE_A)),
        Err(SocialError::NotMember)
    ));
}

#[test]
fn participation_summary_is_membership_scoped_and_counts_own_messages() {
    let (_temp, root) = setup();
    let store = groups(&root);
    let shared = store.create_group("Shared", &owner()).expect("shared");
    let other = store.create_group("Other", &owner()).expect("other");
    let left = store.create_group("Left", &owner()).expect("left");

    store
        .add_member(&shared.group_id, MemberKind::Core, CORE_A, &owner())
        .expect("add a shared");
    store
        .add_member(&other.group_id, MemberKind::Core, CORE_B, &owner())
        .expect("add b other");
    store
        .add_member(&left.group_id, MemberKind::Core, CORE_A, &owner())
        .expect("add a left");

    let a = core(CORE_A);
    let b = core(CORE_B);
    store
        .post_message(&shared.group_id, &a, "a one")
        .expect("a one");
    store
        .post_message(&shared.group_id, &owner(), "owner one")
        .expect("owner one");
    store
        .post_message(&shared.group_id, &a, "a two")
        .expect("a two");
    store
        .post_message(&other.group_id, &b, "b one")
        .expect("b one");
    store
        .post_message(&left.group_id, &a, "a left")
        .expect("a left");

    let summary = store
        .participation_summary(CORE_A)
        .expect("a participation");
    assert_eq!(summary.schema, "aegis.social.participation_summary.v1");
    assert_eq!(summary.core_uuid, CORE_A);
    assert_eq!(summary.group_count, 2);
    assert_eq!(summary.memberships.len(), 2);
    assert_eq!(summary.message_count, 3);
    assert!(summary.last_activity_at.is_some());
    let shared_entry = summary
        .memberships
        .iter()
        .find(|entry| entry.group_id == shared.group_id)
        .expect("shared membership");
    assert_eq!(shared_entry.schema, "aegis.social.participation_entry.v1");
    assert_eq!(shared_entry.group_name, "Shared");
    assert_eq!(shared_entry.message_count, 2);
    assert!(!summary
        .memberships
        .iter()
        .any(|entry| entry.group_id == other.group_id));

    let b_summary = store
        .participation_summary(CORE_B)
        .expect("b participation");
    assert_eq!(b_summary.group_count, 1);
    assert_eq!(b_summary.memberships[0].group_id, other.group_id);
    assert_eq!(b_summary.message_count, 1);

    store
        .remove_member(&left.group_id, MemberKind::Core, CORE_A, &owner())
        .expect("remove a left");
    let after = store
        .participation_summary(CORE_A)
        .expect("a after removal");
    assert_eq!(after.group_count, 1);
    assert_eq!(after.message_count, 2);
    assert!(!after
        .memberships
        .iter()
        .any(|entry| entry.group_id == left.group_id));

    assert!(matches!(
        store.participation_summary("not-a-core"),
        Err(SocialError::InvalidCoreUuid)
    ));
}

#[test]
fn participation_summary_never_reads_private_relationships() {
    let (_temp, root) = setup();
    let relationships = RelationshipStore::open(&root, CORE_A).expect("relationship store");
    relationships
        .upsert(
            RelationshipTargetKind::Core,
            CORE_B,
            "PRIVATE-LABEL-A",
            93,
            "PRIVATE-NOTE-A",
            vec!["PRIVATE-TAG-A".to_owned()],
        )
        .expect("private relationship");
    drop(relationships);

    let store = groups(&root);
    let group = store
        .create_group("Shared participation", &owner())
        .expect("group");
    store
        .add_member(&group.group_id, MemberKind::Core, CORE_A, &owner())
        .expect("add a");
    store
        .add_member(&group.group_id, MemberKind::Core, CORE_B, &owner())
        .expect("add b");
    store
        .post_message(&group.group_id, &core(CORE_A), "shared message")
        .expect("post");

    for reader in [CORE_A, CORE_B] {
        let summary = store
            .participation_summary(reader)
            .expect("participation summary");
        assert_eq!(summary.group_count, 1);
        let encoded = serde_json::to_string(&summary).expect("summary json");
        assert!(!encoded.contains("PRIVATE-NOTE-A"));
        assert!(!encoded.contains("PRIVATE-LABEL-A"));
        assert!(!encoded.contains("PRIVATE-TAG-A"));
        assert!(!encoded.contains("trust"));
        assert!(!encoded.contains("notes"));
        assert!(!encoded.contains("tags"));
        assert_eq!(summary.core_uuid, reader);
    }

    let a_summary = store.participation_summary(CORE_A).expect("a summary");
    assert_eq!(a_summary.message_count, 1);
    let b_summary = store.participation_summary(CORE_B).expect("b summary");
    assert_eq!(b_summary.message_count, 0);
}

#[test]
fn participation_summary_is_empty_without_membership() {
    let (_temp, root) = setup();
    let store = groups(&root);
    let group = store.create_group("Unrelated", &owner()).expect("group");
    store
        .add_member(&group.group_id, MemberKind::Core, CORE_A, &owner())
        .expect("add a");

    let summary = store
        .participation_summary(CORE_B)
        .expect("b participation");
    assert_eq!(summary.group_count, 0);
    assert_eq!(summary.message_count, 0);
    assert!(summary.memberships.is_empty());
    assert!(summary.last_activity_at.is_none());
}

#[test]
fn removed_member_is_denied_immediately_across_all_operations() {
    let (_temp, root) = setup();
    let store = groups(&root);
    let group = store.create_group("Revocation", &owner()).expect("group");
    store
        .add_member(&group.group_id, MemberKind::Core, CORE_A, &owner())
        .expect("add a");
    store
        .add_member(&group.group_id, MemberKind::Core, CORE_B, &owner())
        .expect("add b");
    let a = core(CORE_A);
    let b = core(CORE_B);
    let posted = store
        .post_message(&group.group_id, &a, "Before removal")
        .expect("post");
    assert!(store.is_member(&group.group_id, &a).expect("member a"));

    store
        .remove_member(&group.group_id, MemberKind::Core, CORE_A, &owner())
        .expect("remove a");

    assert!(!store.is_member(&group.group_id, &a).expect("member false"));
    assert!(store
        .list_groups(&a)
        .expect("a groups")
        .iter()
        .all(|visible| visible.group_id != group.group_id));
    assert!(matches!(
        store.group(&group.group_id, &a),
        Err(SocialError::NotMember)
    ));
    assert!(matches!(
        store.members(&group.group_id, &a),
        Err(SocialError::NotMember)
    ));
    assert!(matches!(
        store.messages(&group.group_id, &a, 10, 0),
        Err(SocialError::NotMember)
    ));
    assert!(matches!(
        store.message(&group.group_id, posted.message_id, &a),
        Err(SocialError::NotMember)
    ));
    assert!(matches!(
        store.context_for_core(CORE_A, &group.group_id, 10),
        Err(SocialError::NotMember)
    ));
    assert!(matches!(
        store.activity_summary(&group.group_id, &a),
        Err(SocialError::NotMember)
    ));
    assert!(matches!(
        store.events(&group.group_id, &a),
        Err(SocialError::NotMember)
    ));
    assert!(matches!(
        store.post_message(&group.group_id, &a, "After removal"),
        Err(SocialError::NotMember)
    ));
    let participation = store
        .participation_summary(CORE_A)
        .expect("a participation");
    assert_eq!(participation.group_count, 0);
    assert!(participation
        .memberships
        .iter()
        .all(|entry| entry.group_id != group.group_id));

    // Remaining members and the owner keep their access.
    assert_eq!(
        store
            .messages(&group.group_id, &b, 10, 0)
            .expect("b reads")
            .len(),
        1
    );
    assert_eq!(
        store
            .events(&group.group_id, &owner())
            .expect("owner events")
            .iter()
            .filter(|event| event.event_type == "member.removed")
            .count(),
        1
    );
}

#[test]
fn non_members_cannot_read_membership_or_events() {
    let (_temp, root) = setup();
    let store = groups(&root);
    let group = store.create_group("Members only", &owner()).expect("group");
    store
        .add_member(&group.group_id, MemberKind::Core, CORE_A, &owner())
        .expect("add a");
    let b = core(CORE_B);

    assert!(!store.is_member(&group.group_id, &b).expect("not member"));
    assert!(matches!(
        store.members(&group.group_id, &b),
        Err(SocialError::NotMember)
    ));
    assert!(matches!(
        store.events(&group.group_id, &b),
        Err(SocialError::NotMember)
    ));
    assert!(matches!(
        store.activity_summary(&group.group_id, &b),
        Err(SocialError::NotMember)
    ));
    assert!(matches!(
        store.group(&group.group_id, &b),
        Err(SocialError::NotMember)
    ));
    // The owner always retains administrative read access.
    assert_eq!(
        store
            .members(&group.group_id, &owner())
            .expect("owner members")
            .len(),
        2
    );
}

#[test]
fn private_relationship_fields_never_leak_through_group_reads() {
    let (_temp, root) = setup();
    let a_store = RelationshipStore::open(&root, CORE_A).expect("a relationships");
    a_store
        .upsert(
            RelationshipTargetKind::Core,
            CORE_B,
            "A-PRIVATE-LABEL",
            87,
            "A-PRIVATE-NOTE",
            vec!["A-PRIVATE-TAG".to_owned()],
        )
        .expect("a relationship");
    drop(a_store);
    let b_store = RelationshipStore::open(&root, CORE_B).expect("b relationships");
    b_store
        .upsert(
            RelationshipTargetKind::Core,
            CORE_A,
            "B-PRIVATE-LABEL",
            12,
            "B-PRIVATE-NOTE",
            vec!["B-PRIVATE-TAG".to_owned()],
        )
        .expect("b relationship");
    drop(b_store);

    let store = groups(&root);
    let group_a = store.create_group("Group A", &owner()).expect("group a");
    let group_b = store.create_group("Group B", &owner()).expect("group b");
    store
        .add_member(&group_a.group_id, MemberKind::Core, CORE_A, &owner())
        .expect("add a");
    store
        .add_member(&group_b.group_id, MemberKind::Core, CORE_B, &owner())
        .expect("add b");
    store
        .post_message(&group_a.group_id, &core(CORE_A), "A shared only in A")
        .expect("post a");
    store
        .post_message(&group_b.group_id, &core(CORE_B), "B shared only in B")
        .expect("post b");

    let private_markers = [
        "A-PRIVATE-LABEL",
        "A-PRIVATE-NOTE",
        "A-PRIVATE-TAG",
        "B-PRIVATE-LABEL",
        "B-PRIVATE-NOTE",
        "B-PRIVATE-TAG",
    ];

    for (reader, own_group, other_group, own_text, other_text) in [
        (
            CORE_A,
            &group_a,
            &group_b,
            "A shared only in A",
            "B shared only in B",
        ),
        (
            CORE_B,
            &group_b,
            &group_a,
            "B shared only in B",
            "A shared only in A",
        ),
    ] {
        let actor = core(reader);
        let visible = store.list_groups(&actor).expect("visible groups");
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].group_id, own_group.group_id);

        let projections = [
            serde_json::to_string(&store.group(&own_group.group_id, &actor).expect("group"))
                .expect("group json"),
            serde_json::to_string(&store.members(&own_group.group_id, &actor).expect("members"))
                .expect("members json"),
            serde_json::to_string(
                &store
                    .messages(&own_group.group_id, &actor, 10, 0)
                    .expect("messages"),
            )
            .expect("messages json"),
            serde_json::to_string(&store.events(&own_group.group_id, &actor).expect("events"))
                .expect("events json"),
            serde_json::to_string(
                &store
                    .activity_summary(&own_group.group_id, &actor)
                    .expect("activity"),
            )
            .expect("activity json"),
            serde_json::to_string(&store.participation_summary(reader).expect("participation"))
                .expect("participation json"),
            serde_json::to_string(
                &store
                    .context_for_core(reader, &own_group.group_id, 10)
                    .expect("context"),
            )
            .expect("context json"),
        ];
        for encoded in &projections {
            assert!(!encoded.contains(other_text));
            for marker in private_markers {
                assert!(!encoded.contains(marker), "private field leaked: {marker}");
            }
            assert!(!encoded.contains("\"trust\""));
            assert!(!encoded.contains("\"notes\""));
            assert!(!encoded.contains("\"tags\""));
        }

        let own_messages = store
            .messages(&own_group.group_id, &actor, 10, 0)
            .expect("own messages");
        assert_eq!(own_messages.len(), 1);
        assert_eq!(own_messages[0].content, own_text);
        let own_context = store
            .context_for_core(reader, &own_group.group_id, 10)
            .expect("own context");
        assert_eq!(own_context.messages.len(), 1);
        assert_eq!(own_context.messages[0].content, own_text);

        // The other Core's private group is invisible and unreadable.
        assert!(matches!(
            store.group(&other_group.group_id, &actor),
            Err(SocialError::NotMember)
        ));
        assert!(matches!(
            store.messages(&other_group.group_id, &actor, 10, 0),
            Err(SocialError::NotMember)
        ));
        assert!(matches!(
            store.members(&other_group.group_id, &actor),
            Err(SocialError::NotMember)
        ));
        assert!(matches!(
            store.events(&other_group.group_id, &actor),
            Err(SocialError::NotMember)
        ));
        assert!(matches!(
            store.activity_summary(&other_group.group_id, &actor),
            Err(SocialError::NotMember)
        ));
        assert!(matches!(
            store.context_for_core(reader, &other_group.group_id, 10),
            Err(SocialError::NotMember)
        ));
    }
}

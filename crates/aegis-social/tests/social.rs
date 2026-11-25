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

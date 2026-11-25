use std::path::PathBuf;

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_core_install::CoreInstaller;
use aegis_core_profiles::{profile, validate_profiles, ProfileError};
use aegis_core_sdk::{CoreMemoryStore, PersonalityProfile};
use aegis_platform::PlatformService;
use aegis_storage::now_rfc3339;

const SOFIA: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const MACK: &str = "11111111-1111-4111-8111-111111111111";
const NOVA: &str = "22222222-2222-4222-8222-222222222222";
const VERA: &str = "66666666-6666-4666-8666-666666666666";
const BLANK: &str = "77777777-7777-4777-8777-777777777777";

fn setup() -> (tempfile::TempDir, PathBuf, String) {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("aegis-root");
    let initialized = PlatformService::initialize(&root).expect("platform");
    let installer = CoreInstaller::new(&root).expect("installer");
    let cores = [
        (SOFIA, 1u8, "Sofía Ada Lyra"),
        (MACK, 2, "Mack Fuller"),
        (NOVA, 3, "Nova Arden"),
        (VERA, 6, "Vera"),
        (BLANK, 7, "Owner Chosen"),
    ];
    for (uuid, number, name) in cores {
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
    (temp, root, initialized.owner_token)
}

fn memory(root: &std::path::Path, core: &str) -> CoreMemoryStore {
    CoreMemoryStore::open(root, core).expect("memory")
}

#[test]
fn catalog_is_valid_and_matches_roster() {
    validate_profiles().expect("profiles");
}

#[test]
fn mack_and_nova_apply_and_persist_personality() {
    let (_temp, root, _token) = setup();
    let mack_memory = memory(&root, MACK);
    let outcome = profile(2)
        .expect("mack")
        .apply_personality(&mack_memory, MACK)
        .expect("apply mack");
    assert!(outcome.applied);
    let stored = mack_memory
        .load_personality()
        .expect("load")
        .expect("personality");
    assert_eq!(stored.traits.get("exploration"), Some(&0.85));
    assert_eq!(stored.traits.get("independence"), Some(&0.8));
    drop(mack_memory);

    let reopened = memory(&root, MACK);
    assert_eq!(
        reopened
            .load_personality()
            .expect("reload")
            .expect("personality")
            .traits
            .get("stubbornness"),
        Some(&0.7)
    );
    assert!(matches!(
        profile(2).expect("mack").apply_personality(&reopened, MACK),
        Err(ProfileError::PersonalityAlreadyPresent)
    ));

    let nova_memory = memory(&root, NOVA);
    assert!(
        profile(3)
            .expect("nova")
            .apply_personality(&nova_memory, NOVA)
            .expect("apply nova")
            .applied
    );
    let nova = nova_memory.load_personality().expect("load").expect("nova");
    assert_eq!(nova.traits.get("playfulness"), Some(&0.85));
    assert!(!nova.traits.contains_key("exploration"));
}

#[test]
fn sofia_preserves_existing_identity_and_never_invents_personality() {
    let (_temp, root, _token) = setup();
    let sofia_memory = memory(&root, SOFIA);
    let fresh = profile(1)
        .expect("sofia")
        .apply_personality(&sofia_memory, SOFIA)
        .expect("apply");
    assert!(!fresh.applied);
    assert!(fresh.reason.contains("preserved"));
    assert!(sofia_memory.load_personality().expect("load").is_none());

    let mut existing = PersonalityProfile::neutral(SOFIA).expect("neutral");
    existing.tone = "established Sofía tone".to_owned();
    existing.traits.insert("warmth".to_owned(), 0.9);
    sofia_memory.save_personality(&existing).expect("save");
    assert!(matches!(
        profile(1)
            .expect("sofia")
            .apply_personality(&sofia_memory, SOFIA),
        Err(ProfileError::PersonalityAlreadyPresent)
    ));
    assert_eq!(
        sofia_memory
            .load_personality()
            .expect("load")
            .expect("personality")
            .tone,
        "established Sofía tone"
    );
}

#[test]
fn blank_and_external_profiles_do_not_invent_personality() {
    let (_temp, root, _token) = setup();
    let blank_memory = memory(&root, BLANK);
    let outcome = profile(7)
        .expect("blank")
        .apply_personality(&blank_memory, BLANK)
        .expect("apply blank");
    assert!(!outcome.applied);
    assert!(outcome.reason.contains("blank"));
    assert!(blank_memory.load_personality().expect("load").is_none());

    let vera_memory = memory(&root, VERA);
    let outcome = profile(6)
        .expect("vera")
        .apply_personality(&vera_memory, VERA)
        .expect("apply vera");
    assert!(!outcome.applied);
    assert!(outcome.reason.contains("external specification"));
    assert!(vera_memory.load_personality().expect("load").is_none());
}

#[test]
fn core_profiles_are_isolated() {
    let (_temp, root, _token) = setup();
    assert!(
        profile(2)
            .expect("mack")
            .apply_personality(&memory(&root, MACK), MACK)
            .expect("mack")
            .applied
    );
    assert!(memory(&root, NOVA)
        .load_personality()
        .expect("nova")
        .is_none());
    assert!(memory(&root, VERA)
        .load_personality()
        .expect("vera")
        .is_none());
}

#[test]
fn display_name_resolution_respects_approved_identity() {
    use aegis_core_profiles::resolve_display_name;
    assert_eq!(resolve_display_name(2, None).expect("mack"), "Mack Fuller");
    assert!(matches!(
        resolve_display_name(2, Some("Impostor")),
        Err(ProfileError::DisplayNameMismatch(2, _))
    ));
    assert!(matches!(
        resolve_display_name(4, None),
        Err(ProfileError::ProhibitedCore(4))
    ));
    assert!(matches!(
        resolve_display_name(7, None),
        Err(ProfileError::DisplayNameRequired(7))
    ));
}

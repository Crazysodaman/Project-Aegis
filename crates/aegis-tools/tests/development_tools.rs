//! Acceptance tests for Core-facing development tools.
//!
//! These exercise the real `ToolExecutor` (registered tools, grants, STOP,
//! audit) against real per-Core development, personality and personal storage,
//! including owner approval and cross-Core isolation.

use serde_json::{json, Value};
use tempfile::TempDir;

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_core_install::CoreInstaller;
use aegis_core_sdk::{CoreSdkContext, PersonalityProfile};
use aegis_platform::PlatformService;
use aegis_storage::now_rfc3339;
use aegis_tools::{ToolExecutor, ToolStatus};

const SOFIA: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const MACK: &str = "11111111-1111-4111-8111-111111111111";

fn request(core_uuid: &str, number: u8, name: &str) -> CoreInitializationRequest {
    CoreInitializationRequest {
        schema: CORE_INIT_REQUEST_SCHEMA.to_owned(),
        core_uuid: core_uuid.to_owned(),
        core_number: number,
        display_name: name.to_owned(),
        requested_at: now_rfc3339().expect("timestamp"),
    }
}

fn setup() -> (TempDir, String) {
    let temp = tempfile::tempdir().expect("tempdir");
    let initialized = PlatformService::initialize(temp.path()).expect("platform");
    let installer = CoreInstaller::new(temp.path()).expect("installer");
    for (uuid, number, name) in [(SOFIA, 1, "Sofía Ada Lyra"), (MACK, 2, "Mack Fuller")] {
        installer
            .initialize(&request(uuid, number, name), &initialized.owner_token)
            .expect("install core");
    }
    (temp, initialized.owner_token)
}

fn executor(temp: &TempDir, core: &str) -> ToolExecutor {
    ToolExecutor::new(temp.path(), core).expect("executor")
}

fn exec(executor: &ToolExecutor, tool: &str, arguments: Value, token: Option<&str>) -> Value {
    let result = executor.execute(tool, &arguments, token).expect("execute");
    assert_eq!(
        result.status,
        ToolStatus::Executed,
        "{tool} failed: {:?}",
        result.detail
    );
    result.output
}

fn id_of(value: &Value, key: &str) -> String {
    value[key].as_str().expect("id field").to_owned()
}

#[test]
fn preference_evidence_lists_and_reflects() {
    let (temp, _token) = setup();
    let sofia = executor(&temp, SOFIA);

    let preference = exec(
        &sofia,
        "preference.record",
        json!({
            "key": "music.jazz",
            "valence": 0.8,
            "weight": 0.9,
            "source": "conversation",
        }),
        None,
    );
    assert_eq!(preference["key"].as_str(), Some("music.jazz"));
    assert_eq!(preference["evidence_count"].as_u64(), Some(1));
    assert!(preference["value"].as_f64().expect("value") > 0.0);

    let listed = exec(&sofia, "preference.list", json!({}), None);
    let listed = listed.as_array().expect("preferences");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["key"].as_str(), Some("music.jazz"));

    let reflection = exec(&sofia, "development.reflect", json!({}), None);
    assert!(reflection["summary"]
        .as_str()
        .expect("summary")
        .contains("music.jazz"));
    assert_eq!(reflection["core_uuid"].as_str(), Some(SOFIA));
}

#[test]
fn proposal_apply_requires_owner_then_changes_and_reverts_personality() {
    let (temp, token) = setup();
    let sofia = executor(&temp, SOFIA);

    let context = CoreSdkContext::open(temp.path(), SOFIA).expect("context");
    let baseline = PersonalityProfile::neutral(SOFIA).expect("neutral profile");
    context
        .memory
        .store_personality(&baseline)
        .expect("store baseline");
    let growth_before = context.memory.personality_growth().expect("growth").len();
    drop(context);

    let proposal = exec(
        &sofia,
        "development.propose",
        json!({
            "rationale": "grow from repeated reflection",
            "values": ["curiosity"],
            "traits": {"openness": 0.42},
        }),
        None,
    );
    let proposal_id = id_of(&proposal, "proposal_id");
    assert_eq!(proposal["status"].as_str(), Some("proposed"));

    let listed = exec(&sofia, "development.list", json!({}), None);
    assert!(listed
        .as_array()
        .expect("proposals")
        .iter()
        .any(|entry| entry["proposal_id"].as_str() == Some(proposal_id.as_str())));

    let unapproved = sofia
        .execute(
            "development.apply",
            &json!({"proposal_id": proposal_id}),
            None,
        )
        .expect("execute");
    assert_eq!(unapproved.status, ToolStatus::ApprovalRequired);

    let applied = exec(
        &sofia,
        "development.apply",
        json!({"proposal_id": proposal_id}),
        Some(&token),
    );
    let openness = applied["traits"]["openness"]
        .as_f64()
        .expect("openness trait");
    assert!((openness - 0.42).abs() < 1e-6);
    assert!(applied["values"]
        .as_array()
        .expect("values")
        .iter()
        .any(|value| value.as_str() == Some("curiosity")));

    let context = CoreSdkContext::open(temp.path(), SOFIA).expect("context");
    let growth_after_apply = context.memory.personality_growth().expect("growth").len();
    assert!(growth_after_apply > growth_before);
    drop(context);

    let reverted = exec(&sofia, "development.revert", json!({}), Some(&token));
    assert!(!reverted["traits"]
        .as_object()
        .expect("traits")
        .contains_key("openness"));

    let context = CoreSdkContext::open(temp.path(), SOFIA).expect("context");
    assert_eq!(
        context.memory.personality_growth().expect("growth").len(),
        growth_after_apply + 1
    );
}

#[test]
fn rejected_proposals_cannot_be_applied() {
    let (temp, token) = setup();
    let sofia = executor(&temp, SOFIA);

    let proposal = exec(
        &sofia,
        "development.propose",
        json!({
            "rationale": "a change to reject",
            "values": ["patience"],
            "traits": {"calmness": 0.2},
        }),
        None,
    );
    let proposal_id = id_of(&proposal, "proposal_id");

    let unapproved = sofia
        .execute(
            "development.reject",
            &json!({"proposal_id": proposal_id}),
            None,
        )
        .expect("execute");
    assert_eq!(unapproved.status, ToolStatus::ApprovalRequired);

    let rejected = exec(
        &sofia,
        "development.reject",
        json!({"proposal_id": proposal_id}),
        Some(&token),
    );
    assert_eq!(rejected["status"].as_str(), Some("rejected"));

    let applied = sofia
        .execute(
            "development.apply",
            &json!({"proposal_id": proposal_id}),
            Some(&token),
        )
        .expect("execute");
    assert_eq!(applied.status, ToolStatus::Failed);

    let context = CoreSdkContext::open(temp.path(), SOFIA).expect("context");
    assert!(context
        .memory
        .personality_growth()
        .expect("growth")
        .is_empty());
}

#[test]
fn mack_cannot_see_sofia_preferences_or_proposals() {
    let (temp, token) = setup();
    let sofia = executor(&temp, SOFIA);

    exec(
        &sofia,
        "preference.record",
        json!({"key": "sofia.only", "valence": 0.8, "weight": 0.9}),
        None,
    );
    let proposal = exec(
        &sofia,
        "development.propose",
        json!({"rationale": "Sofía only", "values": ["curiosity"]}),
        None,
    );
    let proposal_id = id_of(&proposal, "proposal_id");

    let mack = executor(&temp, MACK);
    assert!(exec(&mack, "preference.list", json!({}), None)
        .as_array()
        .expect("mack preferences")
        .is_empty());
    assert!(exec(&mack, "development.list", json!({}), None)
        .as_array()
        .expect("mack proposals")
        .is_empty());

    let foreign = mack
        .execute(
            "development.apply",
            &json!({"proposal_id": proposal_id}),
            Some(&token),
        )
        .expect("execute");
    assert_eq!(foreign.status, ToolStatus::Failed);

    assert_eq!(
        exec(&sofia, "preference.list", json!({}), None)
            .as_array()
            .expect("sofia preferences")
            .len(),
        1
    );
}

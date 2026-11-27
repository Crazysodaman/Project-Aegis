//! Acceptance tests for Core-facing autonomy tools.
//!
//! These exercise the real `ToolExecutor` (registered tools, grants, STOP,
//! audit) against real per-Core autonomy, possessions, consent and personal
//! storage, including cross-Core isolation.

use serde_json::{json, Value};
use tempfile::TempDir;

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_contracts::living::{ConsentGrant, ConsentScope, LIVING_CONTRACT_VERSION};
use aegis_core_install::CoreInstaller;
use aegis_core_sdk::CoreSdkContext;
use aegis_platform::PlatformService;
use aegis_storage::now_rfc3339;
use aegis_tools::{ToolExecutor, ToolStatus};

const SOFIA: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const MACK: &str = "11111111-1111-4111-8111-111111111111";
const PAST: &str = "2000-01-01T00:00:00+00:00";
const FUTURE: &str = "2999-01-01T00:00:00+00:00";

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

fn exec(executor: &ToolExecutor, tool: &str, arguments: Value) -> Value {
    let result = executor.execute(tool, &arguments, None).expect("execute");
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

fn find_task(tasks: &Value, task_id: &str) -> Value {
    tasks
        .as_array()
        .expect("task array")
        .iter()
        .find(|task| task["task_id"].as_str() == Some(task_id))
        .cloned()
        .expect("task present")
}

#[test]
fn goal_and_reading_task_execute_and_persist_across_restart() {
    let (temp, _token) = setup();
    let sofia = executor(&temp, SOFIA);

    // A real document the reading task will actually touch.
    let document = exec(
        &sofia,
        "document.create",
        json!({
            "title": "Reading list",
            "body": "Chapter one: a real persisted body.",
            "format": "plain_text",
        }),
    );
    let document_id = id_of(&document, "document_id");

    let goal = exec(
        &sofia,
        "goal.create",
        json!({
            "title": "Finish a book",
            "description": "Read the whole thing",
            "priority": 3,
            "due_at": FUTURE,
        }),
    );
    let goal_id = id_of(&goal, "goal_id");
    assert_eq!(goal["core_uuid"].as_str(), Some(SOFIA));
    assert_eq!(goal["status"].as_str(), Some("active"));
    assert_eq!(goal["priority"].as_u64(), Some(3));

    let goals = exec(&sofia, "goal.list", json!({}));
    assert!(goals
        .as_array()
        .expect("goals")
        .iter()
        .any(|entry| entry["goal_id"].as_str() == Some(goal_id.as_str())));

    // Only the supplied field changes; the rest are preserved.
    let updated = exec(
        &sofia,
        "goal.update",
        json!({"goal_id": goal_id, "progress_percent": 25}),
    );
    assert_eq!(updated["progress_percent"].as_u64(), Some(25));
    assert_eq!(updated["status"].as_str(), Some("active"));
    assert_eq!(updated["priority"].as_u64(), Some(3));

    let task = exec(
        &sofia,
        "task.create",
        json!({
            "title": "Read chapter one",
            "activity_kind": "read_document",
            "payload": {"document_id": document_id, "percent_per_run": 50},
            "goal_id": goal_id,
            "due_at": PAST,
            "priority": 2,
        }),
    );
    let task_id = id_of(&task, "task_id");
    assert_eq!(task["status"].as_str(), Some("pending"));
    assert_eq!(
        task["payload"]["document_id"].as_str(),
        Some(document_id.as_str())
    );

    // Due now: the first run advances 50% to paused progress.
    let runs = exec(&sofia, "autonomy.tick", json!({}));
    let runs = runs.as_array().expect("runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["task_id"].as_str(), Some(task_id.as_str()));
    assert_eq!(runs[0]["status"].as_str(), Some("paused"));
    assert_eq!(runs[0]["progress_percent"].as_u64(), Some(50));

    // Persistence across a brand-new executor, resumed to completion.
    drop(sofia);
    let restarted = executor(&temp, SOFIA);
    let tasks = exec(&restarted, "task.list", json!({}));
    let persisted = find_task(&tasks, &task_id);
    assert_eq!(persisted["status"].as_str(), Some("paused"));
    assert_eq!(persisted["progress_percent"].as_u64(), Some(50));

    let resumed = exec(&restarted, "task.resume", json!({"task_id": task_id}));
    assert_eq!(resumed["status"].as_str(), Some("active"));

    let runs = exec(&restarted, "autonomy.tick", json!({}));
    let runs = runs.as_array().expect("runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["status"].as_str(), Some("completed"));
    assert_eq!(runs[0]["progress_percent"].as_u64(), Some(100));

    // The real document was read and the experience persisted.
    let context = CoreSdkContext::open(temp.path(), SOFIA).expect("context");
    let experiences = context
        .personal
        .continuity_summary()
        .expect("continuity")
        .recent_experiences;
    assert!(experiences
        .iter()
        .any(|entry| entry.contains("Read document Reading list")));
    let stored = context
        .possessions
        .get_document(&document_id)
        .expect("document")
        .expect("document exists");
    assert_eq!(stored.body, "Chapter one: a real persisted body.");
}

#[test]
fn tick_skips_future_tasks_and_runs_due_tasks() {
    let (temp, _token) = setup();
    let executor = executor(&temp, SOFIA);

    let future = exec(
        &executor,
        "task.create",
        json!({
            "title": "Future reflection",
            "activity_kind": "reflect",
            "payload": {"summary": "not yet"},
            "due_at": FUTURE,
        }),
    );
    let future_id = id_of(&future, "task_id");

    let runs = exec(&executor, "autonomy.tick", json!({}));
    assert!(runs.as_array().expect("runs").is_empty());
    let tasks = exec(&executor, "task.list", json!({}));
    let pending = find_task(&tasks, &future_id);
    assert_eq!(pending["status"].as_str(), Some("pending"));
    assert_eq!(pending["progress_percent"].as_u64(), Some(0));

    let due = exec(
        &executor,
        "task.create",
        json!({
            "title": "Due reflection",
            "activity_kind": "reflect",
            "payload": {"summary": "run now"},
            "due_at": PAST,
        }),
    );
    let due_id = id_of(&due, "task_id");

    let runs = exec(&executor, "autonomy.tick", json!({}));
    let runs = runs.as_array().expect("runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["task_id"].as_str(), Some(due_id.as_str()));
    assert_eq!(runs[0]["status"].as_str(), Some("completed"));
    assert_eq!(runs[0]["progress_percent"].as_u64(), Some(100));

    let tasks = exec(&executor, "task.list", json!({}));
    assert_eq!(
        find_task(&tasks, &due_id)["status"].as_str(),
        Some("completed")
    );
    assert_eq!(
        find_task(&tasks, &future_id)["status"].as_str(),
        Some("pending")
    );
}

#[test]
fn task_lifecycle_start_pause_resume_complete_cancel() {
    let (temp, _token) = setup();
    let executor = executor(&temp, SOFIA);

    let task = exec(
        &executor,
        "task.create",
        json!({"title": "Manual task", "activity_kind": "reflect", "payload": {"summary": "manual"}}),
    );
    let task_id = id_of(&task, "task_id");
    assert_eq!(task["status"].as_str(), Some("pending"));
    assert!(task["due_at"].is_null());

    let started = exec(&executor, "task.start", json!({"task_id": task_id}));
    assert_eq!(started["status"].as_str(), Some("active"));
    let paused = exec(&executor, "task.pause", json!({"task_id": task_id}));
    assert_eq!(paused["status"].as_str(), Some("paused"));
    let resumed = exec(&executor, "task.resume", json!({"task_id": task_id}));
    assert_eq!(resumed["status"].as_str(), Some("active"));

    let completed = exec(
        &executor,
        "task.complete",
        json!({"task_id": task_id, "progress": 75}),
    );
    assert_eq!(completed["status"].as_str(), Some("completed"));
    assert_eq!(completed["progress_percent"].as_u64(), Some(75));

    let other = exec(
        &executor,
        "task.create",
        json!({"title": "Cancel me", "activity_kind": "reflect"}),
    );
    let other_id = id_of(&other, "task_id");
    let cancelled = exec(&executor, "task.cancel", json!({"task_id": other_id}));
    assert_eq!(cancelled["status"].as_str(), Some("cancelled"));
}

#[test]
fn disabled_autonomy_does_not_execute_tasks() {
    let (temp, _token) = setup();
    let executor = executor(&temp, SOFIA);

    let disabled = exec(
        &executor,
        "autonomy.set_enabled",
        json!({"enabled": false, "max_active_tasks": 2}),
    );
    assert_eq!(disabled["enabled"], false);
    assert_eq!(disabled["max_active_tasks"].as_u64(), Some(2));

    let settings = exec(&executor, "autonomy.settings", json!({}));
    assert_eq!(settings["enabled"], false);

    let task = exec(
        &executor,
        "task.create",
        json!({
            "title": "Should not run",
            "activity_kind": "reflect",
            "payload": {"summary": "disabled"},
            "due_at": PAST,
        }),
    );
    let task_id = id_of(&task, "task_id");

    let runs = exec(&executor, "autonomy.tick", json!({}));
    assert!(runs.as_array().expect("runs").is_empty());
    let tasks = exec(&executor, "task.list", json!({}));
    let untouched = find_task(&tasks, &task_id);
    assert_eq!(untouched["status"].as_str(), Some("pending"));
    assert_eq!(untouched["progress_percent"].as_u64(), Some(0));

    exec(&executor, "autonomy.set_enabled", json!({"enabled": true}));
    let runs = exec(&executor, "autonomy.tick", json!({}));
    let runs = runs.as_array().expect("runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["status"].as_str(), Some("completed"));
}

#[test]
fn social_task_requires_consent_then_succeeds_after_grant() {
    let (temp, _token) = setup();
    let executor = executor(&temp, SOFIA);
    let payload = json!({
        "participant": "owner",
        "capability": "hold_hands",
        "scope": "romantic",
    });

    let denied = exec(
        &executor,
        "task.create",
        json!({
            "title": "Hold hands",
            "activity_kind": "social_interaction",
            "payload": payload,
            "due_at": PAST,
        }),
    );
    let denied_id = id_of(&denied, "task_id");
    let runs = exec(&executor, "autonomy.tick", json!({}));
    let runs = runs.as_array().expect("runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["task_id"].as_str(), Some(denied_id.as_str()));
    assert_eq!(runs[0]["status"].as_str(), Some("failed"));
    assert_eq!(runs[0]["message"].as_str(), Some("consent_required"));

    // Explicit, scoped consent recorded through the real ConsentStore.
    let context = CoreSdkContext::open(temp.path(), SOFIA).expect("context");
    context
        .consent
        .grant(&ConsentGrant {
            schema: LIVING_CONTRACT_VERSION.to_owned(),
            grant_id: "grant-hold-hands".to_owned(),
            core_uuid: SOFIA.to_owned(),
            scope: ConsentScope::Romantic,
            capability: "hold_hands".to_owned(),
            participants: vec!["owner".to_owned()],
            granted_at: now_rfc3339().expect("time"),
            expires_at: None,
            revoked_at: None,
            context: "explicit test consent".to_owned(),
        })
        .expect("grant consent");
    drop(context);

    let allowed = exec(
        &executor,
        "task.create",
        json!({
            "title": "Hold hands again",
            "activity_kind": "social_interaction",
            "payload": payload,
            "due_at": PAST,
        }),
    );
    let allowed_id = id_of(&allowed, "task_id");
    let runs = exec(&executor, "autonomy.tick", json!({}));
    let runs = runs.as_array().expect("runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["task_id"].as_str(), Some(allowed_id.as_str()));
    assert_eq!(runs[0]["status"].as_str(), Some("completed"));
}

#[test]
fn cores_cannot_see_or_touch_each_others_tasks() {
    let (temp, _token) = setup();
    let sofia = executor(&temp, SOFIA);

    let goal = exec(&sofia, "goal.create", json!({"title": "Sofía only"}));
    let goal_id = id_of(&goal, "goal_id");
    let task = exec(
        &sofia,
        "task.create",
        json!({"title": "Sofía task", "activity_kind": "reflect", "payload": {"summary": "mine"}}),
    );
    let task_id = id_of(&task, "task_id");

    let mack = executor(&temp, MACK);
    assert!(exec(&mack, "task.list", json!({}))
        .as_array()
        .expect("mack tasks")
        .is_empty());
    assert!(exec(&mack, "goal.list", json!({}))
        .as_array()
        .expect("mack goals")
        .is_empty());

    let foreign_start = mack
        .execute("task.start", &json!({"task_id": task_id}), None)
        .expect("execute");
    assert_eq!(foreign_start.status, ToolStatus::Failed);

    let foreign_goal = mack
        .execute(
            "goal.update",
            &json!({"goal_id": goal_id, "status": "completed"}),
            None,
        )
        .expect("execute");
    assert_eq!(foreign_goal.status, ToolStatus::Failed);

    // Sofía's records are untouched and still visible to Sofía.
    let sofia_tasks = exec(&sofia, "task.list", json!({}));
    assert_eq!(sofia_tasks.as_array().expect("tasks").len(), 1);
    assert_eq!(
        find_task(&sofia_tasks, &task_id)["status"].as_str(),
        Some("pending")
    );
    let sofia_goals = exec(&sofia, "goal.list", json!({}));
    assert_eq!(sofia_goals.as_array().expect("goals").len(), 1);
}

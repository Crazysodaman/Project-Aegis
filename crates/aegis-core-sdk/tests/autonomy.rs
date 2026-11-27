use std::fs;
use std::path::{Path, PathBuf};

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_contracts::living::{
    ConsentGrant, ConsentScope, DocumentFormat, GoalStatus, PlanStatus, Visibility,
    LIVING_CONTRACT_VERSION,
};
use aegis_core_install::CoreInstaller;
use aegis_core_sdk::{
    AutonomyEngine, AutonomySettings, AutonomyStore, CoreSdkContext, Goal, Plan, SdkError, Task,
    TaskStatus, AUTONOMY_APPLICATION_ID, AUTONOMY_DB_FILE, AUTONOMY_EVENTS_TABLE,
    AUTONOMY_GOALS_TABLE, AUTONOMY_OWNERSHIP_TABLE, AUTONOMY_PLANS_TABLE, AUTONOMY_SCHEMA,
    AUTONOMY_SCHEMA_REVISION, AUTONOMY_SCHEMA_TABLE, AUTONOMY_SETTINGS_TABLE, AUTONOMY_TASKS_TABLE,
};
use aegis_core_storage::{CoreDatabase, CoreStorageLayout};
use aegis_platform::PlatformService;
use aegis_storage::now_rfc3339;
use rusqlite::Connection;
use serde_json::json;

const SOFIA: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const MACK: &str = "11111111-1111-4111-8111-111111111111";
const EARLIER: &str = "2026-01-01T00:00:00+00:00";
const NOW: &str = "2026-01-02T00:00:00+00:00";

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
        .initialize(
            &request(SOFIA, 1, "Sofía Ada Lyra"),
            &initialized.owner_token,
        )
        .expect("install Sofía");
    installer
        .initialize(&request(MACK, 2, "Mack Fuller"), &initialized.owner_token)
        .expect("install Mack");
    Fixture { _temp: temp, root }
}

fn layout(root: &Path) -> CoreStorageLayout {
    CoreStorageLayout::new(root).expect("layout")
}

fn autonomy_path(root: &Path, core_uuid: &str) -> PathBuf {
    layout(root)
        .core_dir(core_uuid)
        .expect("core dir")
        .join(AUTONOMY_DB_FILE)
}

fn goal(core_uuid: &str, goal_id: &str, title: &str) -> Goal {
    Goal {
        schema: LIVING_CONTRACT_VERSION.to_owned(),
        goal_id: goal_id.to_owned(),
        core_uuid: core_uuid.to_owned(),
        title: title.to_owned(),
        description: "A durable goal".to_owned(),
        status: GoalStatus::Active,
        priority: 3,
        due_at: None,
        trigger: None,
        progress_percent: 0,
        provenance: "test".to_owned(),
        created_at: EARLIER.to_owned(),
        updated_at: EARLIER.to_owned(),
        revision: 1,
    }
}

fn plan(core_uuid: &str, plan_id: &str, goal_id: &str) -> Plan {
    Plan {
        schema: LIVING_CONTRACT_VERSION.to_owned(),
        plan_id: plan_id.to_owned(),
        core_uuid: core_uuid.to_owned(),
        goal_id: Some(goal_id.to_owned()),
        title: "Test plan".to_owned(),
        status: PlanStatus::Active,
        created_at: EARLIER.to_owned(),
        updated_at: EARLIER.to_owned(),
        revision: 1,
    }
}

fn task(
    core_uuid: &str,
    task_id: &str,
    plan_id: Option<&str>,
    activity_kind: &str,
    payload: serde_json::Value,
) -> Task {
    Task {
        schema: LIVING_CONTRACT_VERSION.to_owned(),
        task_id: task_id.to_owned(),
        core_uuid: core_uuid.to_owned(),
        plan_id: plan_id.map(str::to_owned),
        goal_id: None,
        title: format!("Task {task_id}"),
        activity_kind: activity_kind.to_owned(),
        status: TaskStatus::Pending,
        priority: 1,
        due_at: Some(NOW.to_owned()),
        progress_percent: 0,
        payload,
        created_at: EARLIER.to_owned(),
        updated_at: EARLIER.to_owned(),
        revision: 1,
    }
}

fn grant(
    core_uuid: &str,
    grant_id: &str,
    scope: ConsentScope,
    capability: &str,
    participant: &str,
) -> ConsentGrant {
    ConsentGrant {
        schema: LIVING_CONTRACT_VERSION.to_owned(),
        grant_id: grant_id.to_owned(),
        core_uuid: core_uuid.to_owned(),
        scope,
        capability: capability.to_owned(),
        participants: vec![participant.to_owned()],
        granted_at: EARLIER.to_owned(),
        expires_at: None,
        revoked_at: None,
        context: "autonomy test consent".to_owned(),
    }
}

#[test]
fn autonomy_database_binds_identity_and_schema() {
    let fixture = fixture();
    let snapshot =
        CoreDatabase::read_snapshot(&layout(&fixture.root), SOFIA).expect("core snapshot");
    let store = AutonomyStore::open(&fixture.root, SOFIA).expect("autonomy store");
    assert_eq!(store.core_uuid(), SOFIA);
    assert_eq!(
        store.database_instance_uuid(),
        snapshot.metadata.database_instance_uuid
    );
    assert_eq!(
        store
            .database_path()
            .file_name()
            .and_then(|name| name.to_str()),
        Some(AUTONOMY_DB_FILE)
    );
    let path = store.database_path().to_path_buf();
    drop(store);

    let connection = Connection::open(&path).expect("raw autonomy db");
    let application_id: i64 = connection
        .query_row("PRAGMA application_id", [], |row| row.get(0))
        .expect("application id");
    assert_eq!(application_id, AUTONOMY_APPLICATION_ID);
    let ownership: (String, String, String) = connection
        .query_row(
            "SELECT core_uuid, database_instance_uuid, schema FROM autonomy_ownership",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("ownership row");
    assert_eq!(ownership.0, SOFIA);
    assert_eq!(ownership.1, snapshot.metadata.database_instance_uuid);
    assert_eq!(ownership.2, AUTONOMY_SCHEMA);
    let marker: (String, i64) = connection
        .query_row(
            "SELECT schema, revision FROM autonomy_schema WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("schema marker");
    assert_eq!(marker.0, AUTONOMY_SCHEMA);
    assert_eq!(marker.1, i64::try_from(AUTONOMY_SCHEMA_REVISION).unwrap());
    for table in [
        AUTONOMY_OWNERSHIP_TABLE,
        AUTONOMY_SCHEMA_TABLE,
        AUTONOMY_GOALS_TABLE,
        AUTONOMY_PLANS_TABLE,
        AUTONOMY_TASKS_TABLE,
        AUTONOMY_SETTINGS_TABLE,
        AUTONOMY_EVENTS_TABLE,
    ] {
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |row| row.get(0),
            )
            .expect("table count");
        assert_eq!(count, 1, "missing table {table}");
    }
}

#[test]
fn scheduled_task_survives_restart_and_completes() {
    let fixture = fixture();
    let context = CoreSdkContext::open(&fixture.root, SOFIA).expect("sdk context");
    let document = context
        .possessions
        .create_document(
            "Reading list",
            "chapter one",
            DocumentFormat::PlainText,
            Visibility::Private,
        )
        .expect("document");
    context
        .autonomy
        .create_goal(&goal(SOFIA, "goal-1", "Finish a book"))
        .expect("goal");
    context
        .autonomy
        .create_plan(&plan(SOFIA, "plan-1", "goal-1"))
        .expect("plan");
    context
        .autonomy
        .create_task(&task(
            SOFIA,
            "task-1",
            Some("plan-1"),
            "read_document",
            json!({ "document_id": document.document_id, "percent_per_run": 50 }),
        ))
        .expect("task");

    let due = context.autonomy.tick(NOW).expect("tick");
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].task_id, "task-1");
    let started = context.autonomy.start_task("task-1").expect("start");
    assert_eq!(started.status, TaskStatus::Active);

    let engine = AutonomyEngine::new(&context);
    let runs = engine.run_due(NOW).expect("run partial");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, TaskStatus::Paused);
    assert_eq!(runs[0].progress_percent, 50);

    let paused = context.autonomy.pause_task("task-1").expect("pause");
    assert_eq!(paused.status, TaskStatus::Paused);
    assert_eq!(paused.progress_percent, 50);
    drop(context);

    let context = CoreSdkContext::open(&fixture.root, SOFIA).expect("reopen context");
    let restored = context.autonomy.get_task("task-1").expect("task").unwrap();
    assert_eq!(restored.status, TaskStatus::Paused);
    assert_eq!(restored.progress_percent, 50);

    context.autonomy.resume_task("task-1").expect("resume");
    let engine = AutonomyEngine::new(&context);
    let runs = engine.run_due(NOW).expect("run to completion");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, TaskStatus::Completed);
    assert_eq!(runs[0].progress_percent, 100);

    let completed = context.autonomy.get_task("task-1").expect("task").unwrap();
    assert_eq!(completed.status, TaskStatus::Completed);
    assert_eq!(completed.progress_percent, 100);
}

#[test]
fn reading_uses_a_real_document_and_records_experience() {
    let fixture = fixture();
    let context = CoreSdkContext::open(&fixture.root, SOFIA).expect("sdk context");
    let document = context
        .possessions
        .create_document(
            "Field notes",
            "real persisted body",
            DocumentFormat::Markdown,
            Visibility::Private,
        )
        .expect("document");
    context
        .autonomy
        .create_task(&task(
            SOFIA,
            "read-1",
            None,
            "read_document",
            json!({ "document_id": document.document_id, "percent_per_run": 100 }),
        ))
        .expect("task");

    let engine = AutonomyEngine::new(&context);
    let runs = engine.run_due(NOW).expect("run");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, TaskStatus::Completed);

    let stored = context
        .possessions
        .get_document(&document.document_id)
        .expect("document")
        .expect("document exists");
    assert_eq!(stored.body, "real persisted body");
    let experiences = context
        .personal
        .continuity_summary()
        .expect("continuity")
        .recent_experiences;
    assert!(experiences
        .iter()
        .any(|summary| summary.contains("Read document Field notes")));
}

#[test]
fn social_interaction_requires_live_consent() {
    let fixture = fixture();
    let context = CoreSdkContext::open(&fixture.root, SOFIA).expect("sdk context");
    let payload = json!({
        "participant": "owner",
        "capability": "hold_hands",
        "scope": "romantic",
    });

    context
        .autonomy
        .create_task(&task(
            SOFIA,
            "social-denied",
            None,
            "social_interaction",
            payload.clone(),
        ))
        .expect("task");
    let engine = AutonomyEngine::new(&context);
    let runs = engine.run_due(NOW).expect("run");
    assert_eq!(runs[0].status, TaskStatus::Failed);
    assert_eq!(runs[0].message, "consent_required");

    context
        .consent
        .grant(&grant(
            SOFIA,
            "grant-hand",
            ConsentScope::Romantic,
            "hold_hands",
            "owner",
        ))
        .expect("grant");
    context
        .autonomy
        .create_task(&task(
            SOFIA,
            "social-allowed",
            None,
            "social_interaction",
            payload.clone(),
        ))
        .expect("task");
    let runs = engine.run_due(NOW).expect("run");
    assert_eq!(runs[0].status, TaskStatus::Completed);

    context
        .consent
        .revoke("grant-hand")
        .expect("revoke consent");
    context
        .autonomy
        .create_task(&task(
            SOFIA,
            "social-revoked",
            None,
            "social_interaction",
            payload,
        ))
        .expect("task");
    let runs = engine.run_due(NOW).expect("run");
    assert_eq!(runs[0].status, TaskStatus::Failed);
    assert_eq!(runs[0].message, "consent_required");
}

#[test]
fn disabled_autonomy_does_not_execute() {
    let fixture = fixture();
    let context = CoreSdkContext::open(&fixture.root, SOFIA).expect("sdk context");
    let mut settings = context.autonomy.settings().expect("settings");
    settings.enabled = false;
    context
        .autonomy
        .set_settings(&settings)
        .expect("disable autonomy");
    context
        .autonomy
        .create_task(&task(
            SOFIA,
            "reflect-disabled",
            None,
            "reflect",
            json!({ "summary": "should not run" }),
        ))
        .expect("task");

    let engine = AutonomyEngine::new(&context);
    let runs = engine.run_due(NOW).expect("run");
    assert!(runs.is_empty());
    let task = context
        .autonomy
        .get_task("reflect-disabled")
        .expect("task")
        .unwrap();
    assert_eq!(task.status, TaskStatus::Pending);
    assert_eq!(task.progress_percent, 0);
    let experiences = context
        .personal
        .continuity_summary()
        .expect("continuity")
        .recent_experiences;
    assert!(!experiences
        .iter()
        .any(|entry| entry.contains("should not run")));
}

#[test]
fn cores_are_isolated_and_foreign_databases_are_rejected() {
    let fixture = fixture();
    let sofia = CoreSdkContext::open(&fixture.root, SOFIA).expect("Sofía context");
    let mack = CoreSdkContext::open(&fixture.root, MACK).expect("Mack context");

    sofia
        .autonomy
        .create_goal(&goal(SOFIA, "sofia-goal", "Sofía goal"))
        .expect("Sofía goal");
    sofia
        .autonomy
        .create_task(&task(
            SOFIA,
            "sofia-task",
            None,
            "reflect",
            json!({ "summary": "Sofía only" }),
        ))
        .expect("Sofía task");
    mack.autonomy
        .create_goal(&goal(MACK, "mack-goal", "Mack goal"))
        .expect("Mack goal");
    mack.autonomy
        .create_task(&task(
            MACK,
            "mack-task",
            None,
            "reflect",
            json!({ "summary": "Mack only" }),
        ))
        .expect("Mack task");

    assert_eq!(sofia.autonomy.list_goals(10).expect("goals").len(), 1);
    assert_eq!(mack.autonomy.list_goals(10).expect("goals").len(), 1);
    assert_eq!(
        sofia.autonomy.list_goals(10).expect("goals")[0].goal_id,
        "sofia-goal"
    );
    assert_eq!(
        mack.autonomy.list_goals(10).expect("goals")[0].goal_id,
        "mack-goal"
    );
    assert!(mack.autonomy.get_goal("sofia-goal").expect("get").is_none());
    assert!(sofia.autonomy.get_task("mack-task").expect("get").is_none());
    assert!(mack
        .autonomy
        .tick(NOW)
        .expect("tick")
        .iter()
        .all(|task| task.task_id != "sofia-task"));
    drop(sofia);
    drop(mack);

    let reopened = AutonomyStore::open(&fixture.root, SOFIA).expect("reopen");
    assert_eq!(reopened.list_goals(10).expect("goals").len(), 1);
    assert_eq!(reopened.list_tasks(10).expect("tasks").len(), 1);
    drop(reopened);

    fs::copy(
        autonomy_path(&fixture.root, SOFIA),
        autonomy_path(&fixture.root, MACK),
    )
    .expect("copy foreign autonomy db");
    assert!(matches!(
        AutonomyStore::open(&fixture.root, MACK),
        Err(SdkError::OwnershipMismatch)
    ));
}

#[test]
fn events_are_append_only_and_persisted() {
    let fixture = fixture();
    let context = CoreSdkContext::open(&fixture.root, SOFIA).expect("sdk context");
    context
        .autonomy
        .create_goal(&goal(SOFIA, "event-goal", "Event goal"))
        .expect("goal");
    let events = context.autonomy.list_events(10).expect("events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].action, "created");
    let path = context.autonomy.database_path().to_path_buf();
    drop(context);

    let connection = Connection::open(&path).expect("raw autonomy db");
    assert!(connection
        .execute("UPDATE autonomy_events SET action = 'tampered'", [])
        .is_err());
    assert!(connection
        .execute("DELETE FROM autonomy_events", [])
        .is_err());
    assert!(connection
        .execute("UPDATE autonomy_ownership SET core_uuid = 'other'", [])
        .is_err());

    let reopened = AutonomyStore::open(&fixture.root, SOFIA).expect("reopen");
    assert_eq!(reopened.list_events(10).expect("events").len(), 1);
    let settings: AutonomySettings = reopened.settings().expect("settings");
    assert!(settings.enabled);
}

#[test]
fn unsupported_activity_fails_without_fabrication() {
    let fixture = fixture();
    let context = CoreSdkContext::open(&fixture.root, SOFIA).expect("sdk context");
    context
        .autonomy
        .create_task(&task(
            SOFIA,
            "unsupported",
            None,
            "launch_rocket",
            json!({}),
        ))
        .expect("task");
    let engine = AutonomyEngine::new(&context);
    let runs = engine.run_due(NOW).expect("run");
    assert_eq!(runs[0].status, TaskStatus::Failed);
    assert_eq!(runs[0].message, "unsupported_activity");
}

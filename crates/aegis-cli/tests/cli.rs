use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_B: &str = "11111111-1111-4111-8111-111111111111";

fn aegis(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_aegis"))
        .args(args)
        .output()
        .expect("run aegis")
}

fn run_ok(args: &[&str]) -> Value {
    let output = aegis(args);
    assert!(
        output.status.success(),
        "command failed: {args:?}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid JSON for {args:?}: {error}\nstdout: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn root_arg(root: &Path) -> String {
    root.display().to_string()
}

fn init_root(root: &Path) -> Value {
    run_ok(&["init", "--root", &root_arg(root), "--print-owner-token"])
}

fn install_root(root: &Path) -> Value {
    run_ok(&["install", "--root", &root_arg(root), "--headless"])
}

fn fresh() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("aegis-root");
    (temp, root)
}

#[test]
fn help_lists_the_operational_surface() {
    let output = aegis(&["--help"]);
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "aegis install",
        "aegis upgrade",
        "aegis uninstall",
        "aegis core install",
        "aegis roster list",
        "aegis module install",
        "aegis diagnostics",
        "aegis host enroll",
        "aegis backup",
    ] {
        assert!(text.contains(expected), "help is missing {expected}");
    }
}

#[test]
fn unknown_command_fails_with_nonzero_exit() {
    let output = aegis(&["does-not-exist"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown command"));
}

#[test]
fn init_and_status_report_zero_cores() {
    let (_temp, root) = fresh();
    let init = init_root(&root);
    assert!(init["platform_id"].is_string());
    assert!(init["owner_token"].is_string());

    let status = run_ok(&["status", "--root", &root_arg(&root)]);
    assert_eq!(status["zero_core"], Value::Bool(true));
    assert_eq!(status["core_count"], Value::from(0));
    assert_eq!(status["stop_active"], Value::Bool(false));
}

#[test]
fn roster_lists_seven_cores_and_marks_prohibited_twins() {
    let list = run_ok(&["roster", "list"]);
    let cores = list["cores"].as_array().expect("cores array");
    assert_eq!(cores.len(), 7);

    let twins: Vec<&Value> = cores
        .iter()
        .filter(|core| core["core_number"] == 4 || core["core_number"] == 5)
        .collect();
    assert_eq!(twins.len(), 2);
    for twin in twins {
        assert_eq!(twin["implementation_authorized"], Value::Bool(false));
        assert_eq!(twin["status"], Value::from("prohibited"));
    }

    let blank = run_ok(&["roster", "show", "7"]);
    assert_eq!(blank["identity_defined"], Value::Bool(false));
    assert!(blank["default_display_name"].is_null());
}

#[test]
fn core_install_through_roster_persists_and_verifies() {
    let (_temp, root) = fresh();
    install_root(&root);
    let installed = run_ok(&[
        "core",
        "install",
        CORE_A,
        "--root",
        &root_arg(&root),
        "--roster",
        "1",
    ]);
    assert_eq!(installed["identity"]["core_uuid"], Value::from(CORE_A));
    assert_eq!(
        installed["identity"]["display_name"],
        Value::from("Sofía Ada Lyra")
    );

    let list = run_ok(&["core", "list", "--root", &root_arg(&root)]);
    assert_eq!(list.as_array().expect("list").len(), 1);

    let verified = run_ok(&["core", "verify", CORE_A, "--root", &root_arg(&root)]);
    assert_eq!(verified["status"], Value::from("verified"));

    let status = run_ok(&["status", "--root", &root_arg(&root)]);
    assert_eq!(status["core_count"], Value::from(1));
    assert_eq!(status["zero_core"], Value::Bool(false));
}

#[test]
fn prohibited_twins_cannot_be_installed() {
    let (_temp, root) = fresh();
    install_root(&root);
    let output = aegis(&[
        "core",
        "install",
        CORE_A,
        "--root",
        &root_arg(&root),
        "--roster",
        "4",
    ]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("prohibited"));
    let list = run_ok(&["core", "list", "--root", &root_arg(&root)]);
    assert_eq!(list.as_array().expect("list").len(), 0);
}

#[test]
fn blank_core_requires_an_owner_supplied_name() {
    let (_temp, root) = fresh();
    install_root(&root);

    let output = aegis(&[
        "core",
        "install",
        CORE_A,
        "--root",
        &root_arg(&root),
        "--roster",
        "7",
    ]);
    assert!(!output.status.success());

    let installed = run_ok(&[
        "core",
        "install",
        CORE_A,
        "--root",
        &root_arg(&root),
        "--roster",
        "7",
        "--name",
        "Owner Chosen",
    ]);
    assert_eq!(
        installed["identity"]["display_name"],
        Value::from("Owner Chosen")
    );
}

#[test]
fn approved_identity_cannot_be_replaced_through_roster_install() {
    let (_temp, root) = fresh();
    install_root(&root);
    let output = aegis(&[
        "core",
        "install",
        CORE_A,
        "--root",
        &root_arg(&root),
        "--roster",
        "1",
        "--name",
        "Impostor",
    ]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("approved identity"));
}

#[test]
fn module_runtime_status_and_start_gate() {
    let (_temp, root) = fresh();
    install_root(&root);
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("aegis-modules")
        .join("examples")
        .join("hello.manifest.json");
    run_ok(&[
        "module",
        "install",
        &manifest.display().to_string(),
        "--root",
        &root_arg(&root),
    ]);
    run_ok(&[
        "module",
        "enable",
        "com.example.hello",
        "--root",
        &root_arg(&root),
    ]);

    let status = run_ok(&[
        "module",
        "runtime-status",
        "com.example.hello",
        "--root",
        &root_arg(&root),
    ]);
    assert_eq!(status["state"], Value::from("stopped"));
    assert_eq!(status["lifecycle_state"], Value::from("enabled"));

    let output = aegis(&[
        "module",
        "start",
        "com.example.hello",
        "--root",
        &root_arg(&root),
    ]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not executable"));

    let list = run_ok(&["module", "runtime-list", "--root", &root_arg(&root)]);
    assert_eq!(list["modules"].as_array().expect("modules").len(), 1);
}

#[test]
fn conversation_commands_persist_and_export() {
    let (_temp, root) = fresh();
    install_root(&root);
    run_ok(&[
        "core",
        "install",
        CORE_A,
        "--root",
        &root_arg(&root),
        "--roster",
        "1",
    ]);

    let session = run_ok(&[
        "chat",
        "create",
        CORE_A,
        "--root",
        &root_arg(&root),
        "--title",
        "Owner chat",
    ]);
    let session_id = session["session_id"]
        .as_str()
        .expect("session id")
        .to_owned();
    assert_eq!(session["title"], Value::from("Owner chat"));
    assert_eq!(session["state"], Value::from("active"));

    let listed = run_ok(&["chat", "list", CORE_A, "--root", &root_arg(&root)]);
    assert_eq!(listed.as_array().expect("sessions").len(), 1);

    let messages = run_ok(&[
        "chat",
        "messages",
        CORE_A,
        &session_id,
        "--root",
        &root_arg(&root),
    ]);
    assert_eq!(messages.as_array().expect("messages").len(), 0);

    let export = run_ok(&[
        "chat",
        "export",
        CORE_A,
        &session_id,
        "--root",
        &root_arg(&root),
    ]);
    assert_eq!(export["core_uuid"], Value::from(CORE_A));
    assert_eq!(
        export["session"]["session_id"],
        Value::from(session_id.as_str())
    );

    let archived = run_ok(&[
        "chat",
        "archive",
        CORE_A,
        &session_id,
        "--root",
        &root_arg(&root),
    ]);
    assert_eq!(archived["state"], Value::from("archived"));
}

#[test]
fn asset_and_appearance_workflows_persist() {
    let (temp, root) = fresh();
    install_root(&root);
    run_ok(&[
        "core",
        "install",
        CORE_A,
        "--root",
        &root_arg(&root),
        "--roster",
        "1",
    ]);

    let source = temp.path().join("avatar.bin");
    std::fs::write(&source, b"fake-avatar-bytes").expect("asset bytes");
    let imported = run_ok(&[
        "asset",
        "import",
        CORE_A,
        "avatar",
        &source.display().to_string(),
        "--root",
        &root_arg(&root),
        "--name",
        "base.bin",
        "--tags",
        "default,test",
    ]);
    let asset_id = imported["asset_id"].as_str().expect("asset id").to_owned();
    assert_eq!(imported["kind"], Value::from("avatar"));
    assert_eq!(imported["bytes"], Value::from(17));

    let listed = run_ok(&["asset", "list", CORE_A, "--root", &root_arg(&root)]);
    assert_eq!(listed.as_array().expect("assets").len(), 1);

    let output_path = temp.path().join("out.bin");
    let read = run_ok(&[
        "asset",
        "read",
        CORE_A,
        &asset_id,
        "--root",
        &root_arg(&root),
        "--out",
        &output_path.display().to_string(),
    ]);
    assert_eq!(read["bytes"], Value::from(17));
    assert_eq!(
        std::fs::read(&output_path).expect("out"),
        b"fake-avatar-bytes"
    );

    let avatar = run_ok(&[
        "appearance",
        "set-avatar",
        CORE_A,
        &asset_id,
        "--root",
        &root_arg(&root),
    ]);
    assert_eq!(avatar["avatar_asset_id"], Value::from(asset_id.as_str()));

    let themed = run_ok(&[
        "appearance",
        "set-theme",
        CORE_A,
        "--primary",
        "#9400D3",
        "--accent",
        "#00C2FF",
        "--tertiary",
        "#39FF14",
        "--root",
        &root_arg(&root),
    ]);
    assert_eq!(themed["theme"]["primary"], Value::from("#9400D3"));

    let shown = run_ok(&["appearance", "show", CORE_A, "--root", &root_arg(&root)]);
    assert_eq!(shown["avatar_asset_id"], Value::from(asset_id.as_str()));
    assert_eq!(shown["theme"]["accent"], Value::from("#00C2FF"));

    let deleted = run_ok(&[
        "asset",
        "delete",
        CORE_A,
        &asset_id,
        "--root",
        &root_arg(&root),
    ]);
    assert_eq!(deleted["deleted"], Value::Bool(true));
}

#[test]
fn tool_grants_execution_and_approval_work() {
    let (_temp, root) = fresh();
    install_root(&root);
    run_ok(&[
        "core",
        "install",
        CORE_A,
        "--root",
        &root_arg(&root),
        "--roster",
        "1",
    ]);

    let grants = run_ok(&["tool", "list", "--root", &root_arg(&root)]);
    let grants = grants.as_array().expect("grants");
    let create = grants
        .iter()
        .find(|grant| grant["name"] == "memory.create")
        .expect("memory.create");
    assert_eq!(create["granted"], Value::Bool(false));
    let time = grants
        .iter()
        .find(|grant| grant["name"] == "time.now")
        .expect("time.now");
    assert_eq!(time["granted"], Value::Bool(true));

    let executed = run_ok(&[
        "tool",
        "execute",
        CORE_A,
        "time.now",
        "--root",
        &root_arg(&root),
    ]);
    assert_eq!(executed["status"], Value::from("executed"));

    run_ok(&["tool", "revoke", "time.now", "--root", &root_arg(&root)]);
    let denied = run_ok(&[
        "tool",
        "execute",
        CORE_A,
        "time.now",
        "--root",
        &root_arg(&root),
    ]);
    assert_eq!(denied["status"], Value::from("denied"));

    run_ok(&["tool", "grant", "time.now", "--root", &root_arg(&root)]);
    run_ok(&[
        "tool",
        "grant",
        "memory.create",
        "--root",
        &root_arg(&root),
        "--require-approval",
    ]);
    let memory = run_ok(&[
        "tool",
        "execute",
        CORE_A,
        "memory.create",
        "--root",
        &root_arg(&root),
        "--args",
        "{\"content\":\"Tool-created memory.\"}",
    ]);
    assert_eq!(memory["status"], Value::from("executed"));

    let unknown = run_ok(&[
        "tool",
        "execute",
        CORE_A,
        "shell.exec",
        "--root",
        &root_arg(&root),
    ]);
    assert_eq!(unknown["status"], Value::from("denied"));
}

#[test]
fn social_groups_enforce_membership_and_relationships_persist() {
    let (_temp, root) = fresh();
    install_root(&root);
    for (uuid, roster) in [(CORE_A, "1"), (CORE_B, "2")] {
        run_ok(&[
            "core",
            "install",
            uuid,
            "--root",
            &root_arg(&root),
            "--roster",
            roster,
        ]);
    }
    let group = run_ok(&[
        "social",
        "group",
        "create",
        "Owner Group",
        "--root",
        &root_arg(&root),
    ]);
    let group_id = group["group_id"].as_str().expect("group id").to_owned();
    run_ok(&[
        "social",
        "group",
        "add",
        &group_id,
        "--member-core",
        CORE_A,
        "--root",
        &root_arg(&root),
    ]);
    run_ok(&[
        "social",
        "group",
        "post",
        &group_id,
        "--content",
        "Hello from Sofía",
        "--from-core",
        CORE_A,
        "--root",
        &root_arg(&root),
    ]);

    let denied = aegis(&[
        "social",
        "group",
        "messages",
        &group_id,
        "--root",
        &root_arg(&root),
        "--as-core",
        CORE_B,
    ]);
    assert!(!denied.status.success());

    run_ok(&[
        "social",
        "group",
        "add",
        &group_id,
        "--member-core",
        CORE_B,
        "--root",
        &root_arg(&root),
    ]);
    let messages = run_ok(&[
        "social",
        "group",
        "messages",
        &group_id,
        "--root",
        &root_arg(&root),
        "--as-core",
        CORE_B,
    ]);
    assert_eq!(messages.as_array().expect("messages").len(), 1);

    run_ok(&[
        "social",
        "group",
        "remove",
        &group_id,
        "--member-core",
        CORE_A,
        "--root",
        &root_arg(&root),
    ]);
    let removed = aegis(&[
        "social",
        "group",
        "messages",
        &group_id,
        "--root",
        &root_arg(&root),
        "--as-core",
        CORE_A,
    ]);
    assert!(!removed.status.success());

    run_ok(&[
        "social",
        "relationship",
        "set",
        CORE_A,
        "--target-kind",
        "core",
        "--target",
        CORE_B,
        "--label",
        "Mack",
        "--trust",
        "60",
        "--root",
        &root_arg(&root),
    ]);
    let relationships = run_ok(&[
        "social",
        "relationship",
        "list",
        CORE_A,
        "--root",
        &root_arg(&root),
    ]);
    let relationships = relationships.as_array().expect("relationships");
    assert_eq!(relationships.len(), 1);
    assert_eq!(relationships[0]["trust"], Value::from(60));
}

#[test]
fn core_profiles_apply_approved_personality_without_invention() {
    let (_temp, root) = fresh();
    install_root(&root);
    let profiles = run_ok(&["core", "profiles"]);
    let profiles = profiles["profiles"].as_array().expect("profiles");
    assert_eq!(profiles.len(), 7);
    let twin = profiles
        .iter()
        .find(|profile| profile["core_number"] == 4)
        .expect("twin");
    assert_eq!(twin["implementation_authorized"], Value::Bool(false));
    let blank = profiles
        .iter()
        .find(|profile| profile["core_number"] == 7)
        .expect("blank");
    assert!(blank["display_name"].is_null());

    let mack = run_ok(&["core", "profile", "2"]);
    assert_eq!(mack["display_name"], "Mack Fuller");
    assert_eq!(
        mack["personality"]["traits"]["exploration"],
        Value::from(0.85)
    );
    let sofia = run_ok(&["core", "profile", "1"]);
    assert_eq!(
        sofia["voice"]["profile_id"],
        "sofia_r9_bluehour_REFERENCE_AIR_medium"
    );
    assert_eq!(sofia["voice"]["preserve_existing"], Value::Bool(true));

    run_ok(&[
        "core",
        "install",
        CORE_B,
        "--root",
        &root_arg(&root),
        "--roster",
        "2",
    ]);
    let applied = run_ok(&["core", "apply-profile", CORE_B, "--root", &root_arg(&root)]);
    assert_eq!(applied["applied"], Value::Bool(true));
    let repeated = aegis(&["core", "apply-profile", CORE_B, "--root", &root_arg(&root)]);
    assert!(!repeated.status.success());
}

#[test]
fn fleet_enrollment_requires_owner_and_policy_persists() {
    let (_temp, root) = fresh();
    install_root(&root);
    let host = run_ok(&[
        "fleet",
        "host",
        "add",
        "Venus",
        "--endpoint",
        "http://127.0.0.1:11434",
        "--role",
        "inference",
        "--priority",
        "90",
        "--root",
        &root_arg(&root),
    ]);
    let host_id = host["host_id"].as_str().expect("host id").to_owned();
    assert_eq!(host["enrolled"], Value::Bool(false));
    assert_eq!(host["trusted"], Value::Bool(false));

    let listed = run_ok(&["fleet", "host", "list", "--root", &root_arg(&root)]);
    assert_eq!(listed.as_array().expect("hosts").len(), 1);

    let enrolled = run_ok(&[
        "fleet",
        "host",
        "enroll",
        &host_id,
        "--root",
        &root_arg(&root),
    ]);
    assert_eq!(enrolled["trusted"], Value::Bool(true));
    let placements = run_ok(&["fleet", "placement", "--root", &root_arg(&root)]);
    assert_eq!(placements.as_array().expect("placements").len(), 0);

    let policy = run_ok(&["fleet", "policy", "show", "--root", &root_arg(&root)]);
    assert_eq!(policy["gaming_yield"], Value::Bool(true));
    let updated = run_ok(&[
        "fleet",
        "policy",
        "set",
        "--no-gaming-yield",
        "--root",
        &root_arg(&root),
    ]);
    assert_eq!(updated["gaming_yield"], Value::Bool(false));

    let revoked = run_ok(&[
        "fleet",
        "host",
        "revoke",
        &host_id,
        "--root",
        &root_arg(&root),
    ]);
    assert_eq!(revoked["trusted"], Value::Bool(false));

    let gpu = run_ok(&["fleet", "gpu", "status", "--root", &root_arg(&root)]);
    assert_eq!(gpu["schema"], "aegis.fleet.gpu_telemetry.v1");
    let admission = run_ok(&["fleet", "gpu", "admission", "--root", &root_arg(&root)]);
    assert_eq!(admission["schema"], "aegis.fleet.gpu_admission.v1");
    assert!(admission["admit"].is_boolean());

    let limits = run_ok(&[
        "fleet",
        "policy",
        "set",
        "--max-utilization",
        "40",
        "--min-free-vram",
        "512",
        "--max-temperature",
        "85",
        "--root",
        &root_arg(&root),
    ]);
    assert_eq!(limits["max_utilization_percent"], Value::from(40));
    assert_eq!(limits["min_free_vram_mb"], Value::from(512));
    assert_eq!(limits["max_temperature_c"], Value::from(85));

    let supervision = run_ok(&[
        "fleet",
        "gpu",
        "supervise",
        "--dry-run",
        "--iterations",
        "1",
        "--busy-samples",
        "1",
        "--idle-samples",
        "1",
        "--root",
        &root_arg(&root),
    ]);
    assert_eq!(supervision["schema"], "aegis.cli.gpu_supervision_run.v1");
    assert_eq!(
        supervision["reports"].as_array().expect("reports").len(),
        1
    );
}

#[test]
fn host_enrollment_is_explicit_and_owner_gated() {
    let (_temp, root) = fresh();
    install_root(&root);

    let initial = run_ok(&["host", "status", "--root", &root_arg(&root)]);
    assert_eq!(initial["enrollment"]["state"], Value::from("unenrolled"));
    assert!(initial["identity"]["host_uuid"].is_string());

    let enrolled = run_ok(&["host", "enroll", "--root", &root_arg(&root)]);
    assert_eq!(enrolled["enrollment"]["state"], Value::from("enrolled"));

    let history = run_ok(&["host", "history", "--root", &root_arg(&root)]);
    assert_eq!(history.as_array().expect("history").len(), 1);

    let revoked = run_ok(&["host", "revoke", "--root", &root_arg(&root)]);
    assert_eq!(revoked["enrollment"]["state"], Value::from("unenrolled"));
}

#[test]
fn diagnostics_reports_healthy_zero_core_platform() {
    let (_temp, root) = fresh();
    install_root(&root);
    let report = run_ok(&["diagnostics", "--root", &root_arg(&root)]);
    assert_eq!(report["status"], Value::from("healthy"));
    assert_eq!(report["platform"]["available"], Value::Bool(true));
    assert_eq!(report["platform"]["zero_core"], Value::Bool(true));
    assert_eq!(report["cores"]["count"], Value::from(0));
    assert_eq!(report["modules"]["status"], Value::from("available"));
}

#[test]
fn module_lifecycle_works_end_to_end() {
    let (_temp, root) = fresh();
    install_root(&root);
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("aegis-modules")
        .join("examples")
        .join("hello.manifest.json");
    assert!(manifest.is_file(), "example manifest missing");

    let installed = run_ok(&[
        "module",
        "install",
        &manifest.display().to_string(),
        "--root",
        &root_arg(&root),
    ]);
    assert_eq!(installed["module_id"], Value::from("com.example.hello"));
    assert_eq!(installed["lifecycle_state"], Value::from("installed"));

    let enabled = run_ok(&[
        "module",
        "enable",
        "com.example.hello",
        "--root",
        &root_arg(&root),
    ]);
    assert_eq!(enabled["lifecycle_state"], Value::from("enabled"));

    let verified = run_ok(&[
        "module",
        "verify",
        "com.example.hello",
        "--root",
        &root_arg(&root),
    ]);
    assert_eq!(verified["status"], Value::from("verified"));

    let disabled = run_ok(&[
        "module",
        "disable",
        "com.example.hello",
        "--root",
        &root_arg(&root),
    ]);
    assert_eq!(disabled["lifecycle_state"], Value::from("disabled"));

    let list = run_ok(&["module", "list", "--root", &root_arg(&root)]);
    assert_eq!(list.as_array().expect("modules").len(), 1);
}

#[test]
fn backup_and_verify_round_trip_with_a_core() {
    let (temp, root) = fresh();
    install_root(&root);
    run_ok(&[
        "core",
        "install",
        CORE_A,
        "--root",
        &root_arg(&root),
        "--roster",
        "1",
    ]);

    let backup = temp.path().join("backup");
    let report = run_ok(&[
        "backup",
        "--root",
        &root_arg(&root),
        "--to",
        &backup.display().to_string(),
    ]);
    assert!(report["file_count"].as_u64().unwrap_or(0) >= 3);

    let verification = run_ok(&["verify-backup", "--path", &backup.display().to_string()]);
    assert_eq!(verification["status"], Value::from("Verified"));
}

#[test]
fn install_upgrade_uninstall_cli_flow() {
    let (_temp, root) = fresh();
    let installed = install_root(&root);
    assert_eq!(installed["newly_initialized"], Value::Bool(true));
    assert!(installed["installation"]["callsign"].is_string());

    let upgraded = run_ok(&["upgrade", "--root", &root_arg(&root)]);
    assert_eq!(upgraded["already_current"], Value::Bool(true));

    let uninstalled = run_ok(&["uninstall", "--root", &root_arg(&root)]);
    assert_eq!(uninstalled["data_preserved"], Value::Bool(true));
    assert!(root.join("platform.db").is_file());
    assert!(root.join("safe.db").is_file());
}

#[test]
fn memory_create_and_search_are_owner_gated_and_persistent() {
    let (_temp, root) = fresh();
    install_root(&root);
    run_ok(&[
        "core",
        "install",
        CORE_A,
        "--root",
        &root_arg(&root),
        "--roster",
        "1",
    ]);

    let created = run_ok(&[
        "memory",
        "create",
        CORE_A,
        "--root",
        &root_arg(&root),
        "--content",
        "The owner prefers concise answers.",
        "--tags",
        "preference",
    ]);
    assert_eq!(
        created["content"],
        Value::from("The owner prefers concise answers.")
    );

    let found = run_ok(&[
        "memory",
        "search",
        CORE_A,
        "--root",
        &root_arg(&root),
        "--text",
        "concise",
    ]);
    assert_eq!(found.as_array().expect("entries").len(), 1);
}

#[test]
fn service_status_reports_not_running() {
    let (_temp, root) = fresh();
    install_root(&root);
    let status = run_ok(&["service", "status", "--root", &root_arg(&root)]);
    assert_eq!(status["running"], Value::Bool(false));
    assert_eq!(status["pid_alive"], Value::Bool(false));
}

#[test]
fn two_cores_have_separate_databases() {
    let (_temp, root) = fresh();
    install_root(&root);
    run_ok(&[
        "core",
        "install",
        CORE_A,
        "--root",
        &root_arg(&root),
        "--roster",
        "1",
    ]);
    run_ok(&[
        "core",
        "install",
        CORE_B,
        "--root",
        &root_arg(&root),
        "--roster",
        "2",
    ]);
    let first = root.join("cores").join(CORE_A).join("core.db");
    let second = root.join("cores").join(CORE_B).join("core.db");
    assert!(first.is_file());
    assert!(second.is_file());
    assert_ne!(first, second);
    assert_ne!(
        std::fs::read(&first).expect("core A"),
        std::fs::read(&second).expect("core B")
    );
}

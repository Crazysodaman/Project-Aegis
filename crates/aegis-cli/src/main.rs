use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use aegis_backup::{backup, restore, verify_backup, RestoreOptions};
use aegis_cognition::{
    CancelToken, ChatMessage, ChatRequest, ModelProvider, OllamaProvider, DEFAULT_OLLAMA_BASE_URL,
};
use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_contracts::ipc::IpcRequest;
use aegis_conversation::{ConversationEngine, EmbeddingRetriever, TurnOptions};
use aegis_core_install::{CoreInspection, CoreInstallation, CoreInstaller};
use aegis_core_runtime::CoreRuntime;
use aegis_core_sdk::{CoreMemoryStore, MemoryKind, MemoryQuery, NewMemory};
use aegis_diagnostics::Diagnostics;
use aegis_embeddings::{EmbeddingProvider, HashingEmbeddingProvider, OllamaEmbeddingProvider};
use aegis_embodiment::{AssetKind, EmbodimentStore, NewAsset, Theme, VoiceProfile};
use aegis_fleet::{
    fleet_store, GpuPolicy, GpuProbe, GpuSupervisor, HostRole, NvidiaSmiProbe, SupervisorConfig,
};
use aegis_host::HostStore;
use aegis_installer::{install, uninstall, upgrade, InstallMode, UninstallMode};
use aegis_ipc::{
    request_status, request_stop_clear, request_stop_set, request_stop_status, IpcClient, IpcServer,
};
use aegis_modules::{ModuleManager, ModuleRuntime, ModuleRuntimeStatus};
use aegis_platform::PlatformService;
use aegis_roster::{resolve_display_name, validate_roster, ApprovedCoreDesign};
use aegis_service::ServiceManager;
use aegis_social::{
    GroupStore, MemberKind, RelationshipStore, RelationshipTargetKind, SocialActor,
};
use aegis_storage::{now_rfc3339, StoragePaths};
use aegis_tools::{ToolExecutor, ToolRegistry};
use aegis_ui::{DashboardOptions, DashboardServer};
use serde_json::{json, Value};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("aegis: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("init") => command_init(&args),
        Some("install") => command_install(&args),
        Some("upgrade") => command_upgrade(&args),
        Some("uninstall") => command_uninstall(&args),
        Some("status") => command_status(&args),
        Some("serve") => command_serve(&args),
        Some("safe") => command_safe(&args),
        Some("core") => command_core(&args),
        Some("roster") => command_roster(&args),
        Some("module") => command_module(&args),
        Some("diagnostics") => command_diagnostics(&args),
        Some("host") => command_host(&args),
        Some("service") => command_service(&args),
        Some("gui") => command_gui(&args),
        Some("tui") => command_tui(&args),
        Some("backup") => command_backup(&args),
        Some("verify-backup") => command_verify_backup(&args),
        Some("restore") => command_restore(&args),
        Some("provider") => command_provider(&args),
        Some("memory") => command_memory(&args),
        Some("chat") => command_chat(&args),
        Some("asset") => command_asset(&args),
        Some("appearance") => command_appearance(&args),
        Some("tool") => command_tool(&args),
        Some("social") => command_social(&args),
        Some("fleet") => command_fleet(&args),
        Some("effect") => command_effect(&args),
        Some("ipc") => command_ipc(&args),
        Some("help") | Some("--help") | Some("-h") | None => {
            usage();
            Ok(())
        }
        Some(other) => Err(format!("unknown command: {other}")),
    }
}

fn command_init(args: &[String]) -> Result<(), String> {
    let root = require_arg(args, "--root")?;
    let result =
        PlatformService::initialize(PathBuf::from(root)).map_err(|error| error.to_string())?;
    let mut payload = json!({
        "schema": "aegis.cli.init.v1",
        "platform_id": result.platform_id,
        "callsign": result.callsign,
        "owner_token_written": true
    });
    if has_flag(args, "--print-owner-token") {
        payload["owner_token"] = Value::String(result.owner_token);
        payload["warning"] = Value::String(
            "development owner token; do not log, commit, or treat as production proof".to_owned(),
        );
    }
    print_json(&payload)
}

fn command_install(args: &[String]) -> Result<(), String> {
    let root = require_arg(args, "--root")?;
    let mode = if has_flag(args, "--headless") {
        InstallMode::Headless
    } else {
        InstallMode::Guided
    };
    let report = install(PathBuf::from(root), mode).map_err(|error| error.to_string())?;
    let payload = json!({
        "schema": "aegis.cli.install.v1",
        "newly_initialized": report.newly_initialized,
        "installation": report.record,
        "next": "use 'aegis core install <uuid> --roster <n>' to add an approved Core",
    });
    print_json(&payload)
}

fn command_upgrade(args: &[String]) -> Result<(), String> {
    let root = require_arg(args, "--root")?;
    let token = owner_token(root)?;
    let backup = optional_arg(args, "--backup-to").map(PathBuf::from);
    let report = upgrade(PathBuf::from(root), &token, backup.as_deref())
        .map_err(|error| error.to_string())?;
    let payload = json!({
        "schema": "aegis.cli.upgrade.v1",
        "from_version": report.from_version,
        "to_version": report.to_version,
        "already_current": report.already_current,
        "backup": report.backup,
        "installation": report.record,
    });
    print_json(&payload)
}

fn command_uninstall(args: &[String]) -> Result<(), String> {
    let root = require_arg(args, "--root")?;
    let token = owner_token(root)?;
    let mode = if has_flag(args, "--delete-data") {
        UninstallMode::DeleteData
    } else {
        UninstallMode::Preserve
    };
    let backup = optional_arg(args, "--backup-to").map(PathBuf::from);
    let report = uninstall(
        PathBuf::from(root),
        &token,
        mode,
        optional_arg(args, "--confirm"),
        backup.as_deref(),
    )
    .map_err(|error| error.to_string())?;
    let payload = json!({
        "schema": "aegis.cli.uninstall.v1",
        "mode": report.mode,
        "deleted": report.deleted,
        "backup": report.backup,
        "installation": report.record,
        "data_preserved": matches!(report.mode, UninstallMode::Preserve),
    });
    print_json(&payload)
}

fn command_status(args: &[String]) -> Result<(), String> {
    let root = require_arg(args, "--root")?;
    let service = PlatformService::start(PathBuf::from(root)).map_err(|error| error.to_string())?;
    let status = service.status().map_err(|error| error.to_string())?;
    print_json(&serde_json::to_value(status).map_err(|error| error.to_string())?)
}

fn command_serve(args: &[String]) -> Result<(), String> {
    let root = require_arg(args, "--root")?;
    let socket = require_arg(args, "--socket")?;
    let requests = optional_arg(args, "--requests")
        .unwrap_or("1")
        .parse::<usize>()
        .map_err(|_| "--requests must be a nonnegative integer".to_owned())?;
    let service = PlatformService::start(PathBuf::from(root)).map_err(|error| error.to_string())?;
    let server = IpcServer::new(Arc::new(service), socket.to_owned());
    server.serve(requests).map_err(|error| error.to_string())?;
    Ok(())
}

fn command_safe(args: &[String]) -> Result<(), String> {
    let subcommand = args.get(1).map(String::as_str).unwrap_or("status");
    let root = require_arg(args, "--root")?;
    let service = PlatformService::start(PathBuf::from(root)).map_err(|error| error.to_string())?;
    match subcommand {
        "status" => {
            let status = service.stop_status().map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(status).map_err(|error| error.to_string())?)
        }
        "set" => {
            let reason = require_arg(args, "--reason")?;
            let result = service
                .set_stop(reason)
                .map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(result).map_err(|error| error.to_string())?)
        }
        "clear" => {
            let result = service.clear_stop().map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(result).map_err(|error| error.to_string())?)
        }
        other => Err(format!("unknown safe command: {other}")),
    }
}

fn command_core(args: &[String]) -> Result<(), String> {
    let subcommand = args.get(1).map(String::as_str).unwrap_or("list");
    if subcommand == "profiles" {
        aegis_core_profiles::validate_profiles().map_err(|error| error.to_string())?;
        return print_json(&json!({
            "schema": "aegis.cli.core_profiles.v1",
            "profiles": aegis_core_profiles::profiles(),
        }));
    }
    if subcommand == "profile" {
        let number = args
            .get(2)
            .ok_or_else(|| "missing positional Core number".to_owned())?
            .parse::<u8>()
            .map_err(|_| "Core number must be an integer".to_owned())?;
        let profile = aegis_core_profiles::profile(number)
            .ok_or_else(|| format!("Core number {number} is not in the approved roster"))?;
        return print_json(&serde_json::to_value(profile).map_err(|error| error.to_string())?);
    }
    let root = require_arg(args, "--root")?;
    let installer = CoreInstaller::new(PathBuf::from(root)).map_err(|error| error.to_string())?;
    match subcommand {
        "apply-profile" => {
            let core_uuid = args
                .get(2)
                .ok_or_else(|| "missing positional Core UUID".to_owned())?;
            require_owner_proof(root)?;
            let inspection = installer
                .inspect(core_uuid)
                .map_err(|error| error.to_string())?;
            let profile = aegis_core_profiles::profile(inspection.identity.core_number)
                .ok_or_else(|| "Core number has no approved profile".to_owned())?;
            let memory = CoreMemoryStore::open(PathBuf::from(root), core_uuid)
                .map_err(|error| error.to_string())?;
            let outcome = profile
                .apply_personality(&memory, core_uuid)
                .map_err(|error| error.to_string())?;
            print_json(&json!({
                "schema": "aegis.cli.core_profile_apply.v1",
                "core_uuid": core_uuid,
                "core_number": profile.core_number,
                "designation": profile.designation,
                "applied": outcome.applied,
                "reason": outcome.reason,
            }))
        }
        "list" => {
            let entries = installer.list().map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(entries).map_err(|error| error.to_string())?)
        }
        "install" => {
            let core_uuid = args
                .get(2)
                .ok_or_else(|| "missing positional Core UUID".to_owned())?;
            let roster_number = require_arg(args, "--roster")?
                .parse::<u8>()
                .map_err(|_| "--roster must be an approved Core number".to_owned())?;
            let display_name = resolve_display_name(roster_number, optional_arg(args, "--name"))
                .map_err(|error| error.to_string())?;
            let platform =
                PlatformService::start(PathBuf::from(root)).map_err(|error| error.to_string())?;
            let token = platform.owner_token().map_err(|error| error.to_string())?;
            let request = CoreInitializationRequest {
                schema: CORE_INIT_REQUEST_SCHEMA.to_owned(),
                core_uuid: core_uuid.to_owned(),
                core_number: roster_number,
                display_name,
                requested_at: now_rfc3339().map_err(|error| error.to_string())?,
            };
            let installation = installer
                .initialize(&request, &token)
                .map_err(|error| error.to_string())?;
            print_json(&installation_payload(&installation))
        }
        "init" => {
            let core_uuid = args
                .get(2)
                .ok_or_else(|| "missing positional Core UUID".to_owned())?;
            let display_name = require_arg(args, "--name")?;
            let core_number = require_arg(args, "--number")?
                .parse::<u8>()
                .map_err(|_| "--number must be an integer".to_owned())?;
            let platform =
                PlatformService::start(PathBuf::from(root)).map_err(|error| error.to_string())?;
            let token = platform.owner_token().map_err(|error| error.to_string())?;
            let request = CoreInitializationRequest {
                schema: CORE_INIT_REQUEST_SCHEMA.to_owned(),
                core_uuid: core_uuid.to_owned(),
                core_number,
                display_name: display_name.to_owned(),
                requested_at: now_rfc3339().map_err(|error| error.to_string())?,
            };
            let installation = installer
                .initialize(&request, &token)
                .map_err(|error| error.to_string())?;
            print_json(&installation_payload(&installation))
        }
        "inspect" => {
            let core_uuid = args
                .get(2)
                .ok_or_else(|| "missing positional Core UUID".to_owned())?;
            let inspection = installer
                .inspect(core_uuid)
                .map_err(|error| error.to_string())?;
            print_json(&inspection_payload(&inspection))
        }
        "verify" => {
            let core_uuid = args
                .get(2)
                .ok_or_else(|| "missing positional Core UUID".to_owned())?;
            let report = installer
                .verify(core_uuid)
                .map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(report).map_err(|error| error.to_string())?)
        }
        "deactivate" => {
            let core_uuid = args
                .get(2)
                .ok_or_else(|| "missing positional Core UUID".to_owned())?;
            let reason = require_arg(args, "--reason")?;
            let platform =
                PlatformService::start(PathBuf::from(root)).map_err(|error| error.to_string())?;
            let token = platform.owner_token().map_err(|error| error.to_string())?;
            let entry = installer
                .deactivate(core_uuid, reason, &token)
                .map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(entry).map_err(|error| error.to_string())?)
        }
        "reactivate" | "activate" => {
            let core_uuid = args
                .get(2)
                .ok_or_else(|| "missing positional Core UUID".to_owned())?;
            let reason = require_arg(args, "--reason")?;
            let platform =
                PlatformService::start(PathBuf::from(root)).map_err(|error| error.to_string())?;
            let token = platform.owner_token().map_err(|error| error.to_string())?;
            let entry = installer
                .reactivate(core_uuid, reason, &token)
                .map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(entry).map_err(|error| error.to_string())?)
        }
        "uninstall-preserve" => {
            let core_uuid = args
                .get(2)
                .ok_or_else(|| "missing positional Core UUID".to_owned())?;
            let reason = require_arg(args, "--reason")?;
            let platform =
                PlatformService::start(PathBuf::from(root)).map_err(|error| error.to_string())?;
            let token = platform.owner_token().map_err(|error| error.to_string())?;
            let entry = installer
                .uninstall_preserve(core_uuid, reason, &token)
                .map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(entry).map_err(|error| error.to_string())?)
        }
        "start" => {
            let core_uuid = args
                .get(2)
                .ok_or_else(|| "missing positional Core UUID".to_owned())?;
            let runtime = CoreRuntime::new(
                StoragePaths::new(PathBuf::from(root)).map_err(|error| error.to_string())?,
            );
            let platform =
                PlatformService::start(PathBuf::from(root)).map_err(|error| error.to_string())?;
            let token = platform.owner_token().map_err(|error| error.to_string())?;
            let status = runtime
                .start(core_uuid, &token)
                .map_err(|error| error.to_string())?;
            print_json(&runtime_payload(&status))
        }
        "stop" => {
            let core_uuid = args
                .get(2)
                .ok_or_else(|| "missing positional Core UUID".to_owned())?;
            let runtime = CoreRuntime::new(
                StoragePaths::new(PathBuf::from(root)).map_err(|error| error.to_string())?,
            );
            let platform =
                PlatformService::start(PathBuf::from(root)).map_err(|error| error.to_string())?;
            let token = platform.owner_token().map_err(|error| error.to_string())?;
            let status = runtime
                .stop(core_uuid, &token)
                .map_err(|error| error.to_string())?;
            print_json(&runtime_payload(&status))
        }
        "runtime-status" => {
            let core_uuid = args
                .get(2)
                .ok_or_else(|| "missing positional Core UUID".to_owned())?;
            let runtime = CoreRuntime::new(
                StoragePaths::new(PathBuf::from(root)).map_err(|error| error.to_string())?,
            );
            let status = runtime
                .status(core_uuid)
                .map_err(|error| error.to_string())?;
            print_json(&runtime_payload(&status))
        }
        "rename" => {
            let core_uuid = args
                .get(2)
                .ok_or_else(|| "missing positional Core UUID".to_owned())?;
            let display_name = require_arg(args, "--name")?;
            let platform =
                PlatformService::start(PathBuf::from(root)).map_err(|error| error.to_string())?;
            let token = platform.owner_token().map_err(|error| error.to_string())?;
            let installation = installer
                .rename_display_name(core_uuid, display_name, &token)
                .map_err(|error| error.to_string())?;
            print_json(&installation_payload(&installation))
        }
        other => Err(format!("unknown core command: {other}")),
    }
}

fn command_roster(args: &[String]) -> Result<(), String> {
    let subcommand = args.get(1).map(String::as_str).unwrap_or("list");
    match subcommand {
        "list" => {
            validate_roster().map_err(|error| error.to_string())?;
            print_json(&json!({
                "schema": "aegis.cli.roster.v1",
                "cores": aegis_roster::roster(),
            }))
        }
        "show" => {
            let number = args
                .get(2)
                .ok_or_else(|| "missing positional Core number".to_owned())?
                .parse::<u8>()
                .map_err(|_| "Core number must be an integer".to_owned())?;
            let entry: &ApprovedCoreDesign = aegis_roster::entry(number)
                .ok_or_else(|| format!("Core number {number} is not in the approved roster"))?;
            print_json(&serde_json::to_value(entry).map_err(|error| error.to_string())?)
        }
        other => Err(format!("unknown roster command: {other}")),
    }
}

fn installation_payload(installation: &CoreInstallation) -> Value {
    json!({
        "schema": "aegis.cli.core_installation.v1",
        "identity": installation.identity,
        "metadata": installation.metadata,
        "registry_entry": installation.registry_entry,
        "integrity": installation.integrity,
    })
}

fn inspection_payload(inspection: &CoreInspection) -> Value {
    json!({
        "schema": "aegis.cli.core_inspection.v1",
        "identity": inspection.identity,
        "metadata": inspection.metadata,
        "registry_entry": inspection.registry_entry,
        "integrity": inspection.integrity,
    })
}

fn runtime_payload(status: &aegis_core_runtime::RuntimeStatus) -> Value {
    json!({
        "schema": "aegis.cli.core_runtime_status.v1",
        "core_uuid": status.core_uuid,
        "state": status.state.as_str(),
        "recorded_state": status.recorded_state.as_str(),
        "generation": status.generation,
        "pid": status.pid,
        "heartbeat_at": status.heartbeat_at,
        "heartbeat_fresh": status.heartbeat_fresh,
        "heartbeat_age_ms": status.heartbeat_age_ms,
        "started_at": status.started_at,
        "updated_at": status.updated_at,
        "last_error": status.last_error,
    })
}

fn module_runtime_payload(status: &ModuleRuntimeStatus) -> Value {
    json!({
        "schema": status.schema,
        "module_id": status.module_id,
        "lifecycle_state": status.lifecycle_state,
        "state": status.state.as_str(),
        "recorded_state": status.recorded_state.as_str(),
        "pid": status.pid,
        "pid_alive": status.pid_alive,
        "heartbeat_at": status.heartbeat_at,
        "heartbeat_fresh": status.heartbeat_fresh,
        "heartbeat_age_ms": status.heartbeat_age_ms,
        "started_at": status.started_at,
        "updated_at": status.updated_at,
        "last_error": status.last_error,
    })
}

fn command_module(args: &[String]) -> Result<(), String> {
    let subcommand = args.get(1).map(String::as_str).unwrap_or("list");
    let root = require_arg(args, "--root")?;
    let paths = StoragePaths::new(PathBuf::from(root)).map_err(|error| error.to_string())?;
    let registry = ModuleManager::new(paths.clone());
    let runtime = ModuleRuntime::new(paths);
    match subcommand {
        "list" => {
            let modules = registry.list().map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(modules).map_err(|error| error.to_string())?)
        }
        "install" => {
            let manifest_path = args
                .get(2)
                .ok_or_else(|| "missing positional module manifest path".to_owned())?;
            let manifest = fs::read_to_string(manifest_path)
                .map_err(|error| format!("could not read module manifest: {error}"))?;
            let token = owner_token(root)?;
            let expected = optional_arg(args, "--sha256");
            let record = registry
                .install(&manifest, expected, &token)
                .map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(record).map_err(|error| error.to_string())?)
        }
        "install-package" => {
            let package_dir = args
                .get(2)
                .ok_or_else(|| "missing positional module package directory".to_owned())?;
            let token = owner_token(root)?;
            let expected = optional_arg(args, "--sha256");
            let record = registry
                .install_package(package_dir, expected, &token)
                .map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(record).map_err(|error| error.to_string())?)
        }
        "start" => {
            let module_id = args
                .get(2)
                .ok_or_else(|| "missing positional module ID".to_owned())?;
            let token = owner_token(root)?;
            let status = runtime
                .start(module_id, &token)
                .map_err(|error| error.to_string())?;
            print_json(&module_runtime_payload(&status))
        }
        "stop" => {
            let module_id = args
                .get(2)
                .ok_or_else(|| "missing positional module ID".to_owned())?;
            let token = owner_token(root)?;
            let status = runtime
                .stop(module_id, &token)
                .map_err(|error| error.to_string())?;
            print_json(&module_runtime_payload(&status))
        }
        "restart" => {
            let module_id = args
                .get(2)
                .ok_or_else(|| "missing positional module ID".to_owned())?;
            let token = owner_token(root)?;
            let status = runtime
                .restart(module_id, &token)
                .map_err(|error| error.to_string())?;
            print_json(&module_runtime_payload(&status))
        }
        "runtime-status" => {
            let module_id = args
                .get(2)
                .ok_or_else(|| "missing positional module ID".to_owned())?;
            let status = runtime
                .status(module_id)
                .map_err(|error| error.to_string())?;
            print_json(&module_runtime_payload(&status))
        }
        "runtime-list" => {
            let statuses = runtime.list().map_err(|error| error.to_string())?;
            let values: Vec<Value> = statuses.iter().map(module_runtime_payload).collect();
            print_json(&json!({
                "schema": "aegis.cli.module_runtime_list.v1",
                "modules": values,
            }))
        }
        "tools" => {
            let module_id = args
                .get(2)
                .ok_or_else(|| "missing positional module ID".to_owned())?;
            let bindings = registry
                .tool_bindings()
                .map_err(|error| error.to_string())?
                .into_iter()
                .filter(|binding| binding.module_id.as_str() == module_id.as_str())
                .collect::<Vec<_>>();
            print_json(&serde_json::to_value(bindings).map_err(|error| error.to_string())?)
        }
        "enable" => {
            let module_id = args
                .get(2)
                .ok_or_else(|| "missing positional module ID".to_owned())?;
            let token = owner_token(root)?;
            let record = registry
                .enable(module_id, &token)
                .map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(record).map_err(|error| error.to_string())?)
        }
        "disable" => {
            let module_id = args
                .get(2)
                .ok_or_else(|| "missing positional module ID".to_owned())?;
            let token = owner_token(root)?;
            runtime
                .stop(module_id, &token)
                .map_err(|error| error.to_string())?;
            let record = registry
                .disable(module_id, &token)
                .map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(record).map_err(|error| error.to_string())?)
        }
        "uninstall" => {
            let module_id = args
                .get(2)
                .ok_or_else(|| "missing positional module ID".to_owned())?;
            let token = owner_token(root)?;
            runtime
                .stop(module_id, &token)
                .map_err(|error| error.to_string())?;
            let record = registry
                .uninstall(module_id, &token)
                .map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(record).map_err(|error| error.to_string())?)
        }
        "inspect" => {
            let module_id = args
                .get(2)
                .ok_or_else(|| "missing positional module ID".to_owned())?;
            let inspection = registry
                .inspect(module_id)
                .map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(inspection).map_err(|error| error.to_string())?)
        }
        "verify" => {
            let module_id = args
                .get(2)
                .ok_or_else(|| "missing positional module ID".to_owned())?;
            let verification = registry
                .verify(module_id)
                .map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(verification).map_err(|error| error.to_string())?)
        }
        other => Err(format!("unknown module command: {other}")),
    }
}

fn command_diagnostics(args: &[String]) -> Result<(), String> {
    let root = require_arg(args, "--root")?;
    let diagnostics = Diagnostics::new(PathBuf::from(root)).map_err(|error| error.to_string())?;
    let report = diagnostics.collect();
    print_json(&serde_json::to_value(report).map_err(|error| error.to_string())?)
}

fn owner_token(root: &str) -> Result<String, String> {
    let platform =
        PlatformService::start(PathBuf::from(root)).map_err(|error| error.to_string())?;
    platform.owner_token().map_err(|error| error.to_string())
}

fn command_host(args: &[String]) -> Result<(), String> {
    let subcommand = args.get(1).map(String::as_str).unwrap_or("status");
    let root = require_arg(args, "--root")?;
    let paths = StoragePaths::new(PathBuf::from(root)).map_err(|error| error.to_string())?;
    let store = HostStore::new(paths);
    match subcommand {
        "status" => {
            let status = store.status().map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(status).map_err(|error| error.to_string())?)
        }
        "enroll" => {
            let token = owner_token(root)?;
            let status = store.enroll(&token).map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(status).map_err(|error| error.to_string())?)
        }
        "revoke" => {
            let token = owner_token(root)?;
            let status = store.revoke(&token).map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(status).map_err(|error| error.to_string())?)
        }
        "history" => {
            let history = store.history().map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(history).map_err(|error| error.to_string())?)
        }
        other => Err(format!("unknown host command: {other}")),
    }
}

fn command_social(args: &[String]) -> Result<(), String> {
    let root = require_arg(args, "--root")?;
    match args.get(1).map(String::as_str).unwrap_or("group") {
        "group" => command_social_group(root, args),
        "relationship" => command_social_relationship(root, args),
        other => Err(format!("unknown social command: {other}")),
    }
}

fn social_group_store(root: &str) -> Result<GroupStore, String> {
    let paths = StoragePaths::new(PathBuf::from(root)).map_err(|error| error.to_string())?;
    let store = GroupStore::new(paths);
    store.initialize().map_err(|error| error.to_string())?;
    Ok(store)
}

fn social_actor(root: &str, args: &[String]) -> Result<SocialActor, String> {
    if let Some(core) = optional_arg(args, "--as-core") {
        SocialActor::core(core).map_err(|error| error.to_string())
    } else {
        require_owner_proof(root)?;
        Ok(SocialActor::owner())
    }
}

fn command_social_group(root: &str, args: &[String]) -> Result<(), String> {
    let subcommand = args.get(2).map(String::as_str).unwrap_or("list");
    let store = social_group_store(root)?;
    match subcommand {
        "create" => {
            require_owner_proof(root)?;
            let name = args
                .get(3)
                .ok_or_else(|| "missing positional group name".to_owned())?;
            let group = store
                .create_group(name, &SocialActor::owner())
                .map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(group).map_err(|error| error.to_string())?)
        }
        "list" => {
            let actor = social_actor(root, args)?;
            let groups = store
                .list_groups(&actor)
                .map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(groups).map_err(|error| error.to_string())?)
        }
        "show" | "members" | "messages" | "events" => {
            let group_id = args
                .get(3)
                .ok_or_else(|| "missing positional group ID".to_owned())?;
            let actor = social_actor(root, args)?;
            match subcommand {
                "show" => {
                    let group = store
                        .group(group_id, &actor)
                        .map_err(|error| error.to_string())?;
                    print_json(&serde_json::to_value(group).map_err(|error| error.to_string())?)
                }
                "members" => {
                    let members = store
                        .members(group_id, &actor)
                        .map_err(|error| error.to_string())?;
                    print_json(&serde_json::to_value(members).map_err(|error| error.to_string())?)
                }
                "messages" => {
                    let limit = parse_usize(args, "--limit", 50)?;
                    let offset = parse_usize(args, "--offset", 0)?;
                    let messages = store
                        .messages(group_id, &actor, limit, offset)
                        .map_err(|error| error.to_string())?;
                    print_json(&serde_json::to_value(messages).map_err(|error| error.to_string())?)
                }
                _ => {
                    let events = store
                        .events(group_id, &actor)
                        .map_err(|error| error.to_string())?;
                    print_json(&serde_json::to_value(events).map_err(|error| error.to_string())?)
                }
            }
        }
        "add" | "remove" => {
            require_owner_proof(root)?;
            let group_id = args
                .get(3)
                .ok_or_else(|| "missing positional group ID".to_owned())?;
            let (kind, member_id) = social_member_from_args(args)?;
            let owner = SocialActor::owner();
            let members = if subcommand == "add" {
                store.add_member(group_id, kind, member_id, &owner)
            } else {
                store.remove_member(group_id, kind, member_id, &owner)
            }
            .map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(members).map_err(|error| error.to_string())?)
        }
        "post" => {
            require_owner_proof(root)?;
            let group_id = args
                .get(3)
                .ok_or_else(|| "missing positional group ID".to_owned())?;
            let content = require_arg(args, "--content")?;
            let actor = if let Some(core) = optional_arg(args, "--from-core") {
                SocialActor::core(core).map_err(|error| error.to_string())?
            } else {
                SocialActor::owner()
            };
            let message = store
                .post_message(group_id, &actor, content)
                .map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(message).map_err(|error| error.to_string())?)
        }
        other => Err(format!("unknown social group command: {other}")),
    }
}

fn social_member_from_args(args: &[String]) -> Result<(MemberKind, &str), String> {
    if let Some(core) = optional_arg(args, "--member-core") {
        return Ok((MemberKind::Core, core));
    }
    if has_flag(args, "--member-owner") {
        return Ok((MemberKind::Owner, "owner"));
    }
    Err("specify --member-core <uuid> or --member-owner".to_owned())
}

fn command_social_relationship(root: &str, args: &[String]) -> Result<(), String> {
    let subcommand = args.get(2).map(String::as_str).unwrap_or("list");
    require_owner_proof(root)?;
    let core_uuid = args
        .get(3)
        .ok_or_else(|| "missing positional Core UUID".to_owned())?;
    let store = RelationshipStore::open(PathBuf::from(root), core_uuid)
        .map_err(|error| error.to_string())?;
    match subcommand {
        "list" => {
            let relationships = store.list().map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(relationships).map_err(|error| error.to_string())?)
        }
        "set" => {
            let kind = RelationshipTargetKind::parse(require_arg(args, "--target-kind")?)
                .map_err(|error| error.to_string())?;
            let target = require_arg(args, "--target")?;
            let label = optional_arg(args, "--label").unwrap_or(target);
            let trust = optional_arg(args, "--trust")
                .map(|value| {
                    value
                        .parse::<u8>()
                        .map_err(|_| "--trust must be an integer from 0 to 100".to_owned())
                })
                .transpose()?
                .unwrap_or(50);
            if trust > 100 {
                return Err("--trust must be between 0 and 100".to_owned());
            }
            let notes = optional_arg(args, "--notes").unwrap_or("");
            let tags = parse_tags(optional_arg(args, "--tags"));
            let relationship = store
                .upsert(kind, target, label, trust, notes, tags)
                .map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(relationship).map_err(|error| error.to_string())?)
        }
        "delete" => {
            let kind = RelationshipTargetKind::parse(require_arg(args, "--target-kind")?)
                .map_err(|error| error.to_string())?;
            let target = require_arg(args, "--target")?;
            let deleted = store
                .delete(kind, target)
                .map_err(|error| error.to_string())?;
            print_json(&json!({
                "schema": "aegis.cli.social_relationship_delete.v1",
                "deleted": deleted,
            }))
        }
        "events" => {
            let relationship_id = args
                .get(4)
                .ok_or_else(|| "missing positional relationship ID".to_owned())?;
            let events = store
                .events(relationship_id)
                .map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(events).map_err(|error| error.to_string())?)
        }
        other => Err(format!("unknown social relationship command: {other}")),
    }
}

fn command_fleet(args: &[String]) -> Result<(), String> {
    let root = require_arg(args, "--root")?;
    let store = fleet_store(std::path::Path::new(root)).map_err(|error| error.to_string())?;
    match args.get(1).map(String::as_str).unwrap_or("host") {
        "host" => match args.get(2).map(String::as_str).unwrap_or("list") {
            "add" => {
                require_owner_proof(root)?;
                let callsign = args
                    .get(3)
                    .ok_or_else(|| "missing positional host callsign".to_owned())?;
                let endpoint = require_arg(args, "--endpoint")?;
                let role = HostRole::parse(require_arg(args, "--role")?)
                    .map_err(|error| error.to_string())?;
                let priority = optional_arg(args, "--priority")
                    .map(|value| {
                        value
                            .parse::<u8>()
                            .map_err(|_| "--priority must be an integer from 0 to 100".to_owned())
                    })
                    .transpose()?
                    .unwrap_or(50);
                if priority > 100 {
                    return Err("--priority must be between 0 and 100".to_owned());
                }
                let capabilities = match optional_arg(args, "--capabilities") {
                    Some(value) => serde_json::from_str::<Value>(value)
                        .map_err(|error| format!("--capabilities must be JSON: {error}"))?,
                    None => Value::Null,
                };
                let token = owner_token(root)?;
                let host = store
                    .add_host(callsign, endpoint, role, capabilities, priority, &token)
                    .map_err(|error| error.to_string())?;
                print_json(&serde_json::to_value(host).map_err(|error| error.to_string())?)
            }
            "list" => {
                let hosts = store.list_hosts().map_err(|error| error.to_string())?;
                print_json(&serde_json::to_value(hosts).map_err(|error| error.to_string())?)
            }
            "enroll" | "revoke" | "remove" | "probe" => {
                require_owner_proof(root)?;
                let host_id = args
                    .get(3)
                    .ok_or_else(|| "missing positional host ID".to_owned())?;
                let token = owner_token(root)?;
                if args.get(2).map(String::as_str) == Some("remove") {
                    store
                        .remove(host_id, &token)
                        .map_err(|error| error.to_string())?;
                    return print_json(&json!({
                        "schema": "aegis.cli.fleet_host_remove.v1",
                        "host_id": host_id,
                        "removed": true,
                    }));
                }
                let host = match args.get(2).map(String::as_str) {
                    Some("enroll") => store.enroll(host_id, &token),
                    Some("revoke") => store.revoke(host_id, &token),
                    _ => store.probe(host_id),
                }
                .map_err(|error| error.to_string())?;
                print_json(&serde_json::to_value(host).map_err(|error| error.to_string())?)
            }
            other => Err(format!("unknown fleet host command: {other}")),
        },
        "placement" => {
            let placements = store.placements().map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(placements).map_err(|error| error.to_string())?)
        }
        "route" => {
            let model = require_arg(args, "--model")?;
            let candidate = store
                .route_inference(model)
                .map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(candidate).map_err(|error| error.to_string())?)
        }
        "gpu" => match args.get(2).map(String::as_str).unwrap_or("status") {
            "status" => print_json(
                &serde_json::to_value(NvidiaSmiProbe::new().probe())
                    .map_err(|error| error.to_string())?,
            ),
            "admission" => {
                let telemetry = NvidiaSmiProbe::new().probe();
                let decision = store
                    .admission(&telemetry)
                    .map_err(|error| error.to_string())?;
                print_json(&serde_json::to_value(decision).map_err(|error| error.to_string())?)
            }
            "running" => {
                let url = optional_arg(args, "--url")
                    .map(str::to_owned)
                    .or_else(|| std::env::var("AEGIS_OLLAMA_URL").ok())
                    .unwrap_or_else(|| DEFAULT_OLLAMA_BASE_URL.to_owned());
                let provider = OllamaProvider::new(url).map_err(|error| error.to_string())?;
                let models = provider
                    .running_models()
                    .map_err(|error| error.to_string())?;
                print_json(&json!({
                    "schema": "aegis.cli.gpu_running_models.v1",
                    "models": models,
                }))
            }
            "yield" => {
                require_owner_proof(root)?;
                let model = require_arg(args, "--model")?;
                let url = optional_arg(args, "--url")
                    .map(str::to_owned)
                    .or_else(|| std::env::var("AEGIS_OLLAMA_URL").ok())
                    .unwrap_or_else(|| DEFAULT_OLLAMA_BASE_URL.to_owned());
                let provider = OllamaProvider::new(url).map_err(|error| error.to_string())?;
                provider
                    .unload_model(model)
                    .map_err(|error| error.to_string())?;
                print_json(&json!({
                    "schema": "aegis.cli.gpu_yield.v1",
                    "model": model,
                    "unloaded": true,
                }))
            }
            "supervise" => {
                let dry_run = has_flag(args, "--dry-run");
                if !dry_run {
                    require_owner_proof(root)?;
                }
                let url = optional_arg(args, "--url")
                    .map(str::to_owned)
                    .or_else(|| std::env::var("AEGIS_OLLAMA_URL").ok())
                    .unwrap_or_else(|| DEFAULT_OLLAMA_BASE_URL.to_owned());
                let provider = OllamaProvider::new(url).map_err(|error| error.to_string())?;
                let policy = store
                    .policy()
                    .map_err(|error| error.to_string())?
                    .gpu_policy();
                let iterations = parse_usize(args, "--iterations", 1)?;
                if iterations == 0 || iterations > 1000 {
                    return Err("--iterations must be 1-1000".to_owned());
                }
                let interval_ms = parse_usize(args, "--interval-ms", 2_000)?;
                let busy_samples = parse_usize(args, "--busy-samples", 2)? as u32;
                let idle_samples = parse_usize(args, "--idle-samples", 2)? as u32;
                let cooldown_ms = parse_usize(args, "--cooldown-ms", 30_000)?;
                let supervisor = GpuSupervisor::new(SupervisorConfig {
                    policy,
                    busy_samples,
                    idle_samples,
                    cooldown: std::time::Duration::from_millis(cooldown_ms as u64),
                    dry_run,
                })
                .map_err(|error| error.to_string())?;
                let mut supervisor = supervisor;
                let mut reports = Vec::new();
                for index in 0..iterations {
                    let telemetry = NvidiaSmiProbe::new().probe();
                    let running = provider.running_models().unwrap_or_default();
                    let report = supervisor.step(&telemetry, &running, &mut |model| {
                        provider
                            .unload_model(model)
                            .map_err(|error| error.to_string())
                    });
                    reports.push(serde_json::to_value(&report).map_err(|error| error.to_string())?);
                    if index + 1 < iterations {
                        std::thread::sleep(std::time::Duration::from_millis(interval_ms as u64));
                    }
                }
                print_json(&json!({
                    "schema": "aegis.cli.gpu_supervision_run.v1",
                    "dry_run": dry_run,
                    "reports": reports,
                }))
            }
            other => Err(format!("unknown fleet gpu command: {other}")),
        },
        "policy" => match args.get(2).map(String::as_str).unwrap_or("show") {
            "show" => {
                let policy = store.policy().map_err(|error| error.to_string())?;
                print_json(&serde_json::to_value(policy).map_err(|error| error.to_string())?)
            }
            "set" => {
                require_owner_proof(root)?;
                let current = store.policy().map_err(|error| error.to_string())?;
                let mut gpu: GpuPolicy = current.gpu_policy();
                gpu.gaming_yield = !has_flag(args, "--no-gaming-yield");
                if let Some(value) = optional_arg(args, "--max-utilization") {
                    gpu.max_utilization_percent = value.parse::<u8>().map_err(|_| {
                        "--max-utilization must be an integer from 0 to 100".to_owned()
                    })?;
                }
                if let Some(value) = optional_arg(args, "--min-free-vram") {
                    gpu.min_free_vram_mb = value
                        .parse::<u32>()
                        .map_err(|_| "--min-free-vram must be a positive integer".to_owned())?;
                }
                if let Some(value) = optional_arg(args, "--max-temperature") {
                    gpu.max_temperature_c = value
                        .parse::<u8>()
                        .map_err(|_| "--max-temperature must be an integer".to_owned())?;
                }
                let token = owner_token(root)?;
                let policy = store
                    .set_gpu_policy(gpu, &token)
                    .map_err(|error| error.to_string())?;
                print_json(&serde_json::to_value(policy).map_err(|error| error.to_string())?)
            }
            other => Err(format!("unknown fleet policy command: {other}")),
        },
        other => Err(format!("unknown fleet command: {other}")),
    }
}

fn command_effect(args: &[String]) -> Result<(), String> {
    if args.get(1).map(String::as_str) != Some("ping") {
        return Err("only 'effect ping' is supported in Batch 1".to_owned());
    }
    let root = require_arg(args, "--root")?;
    let service = PlatformService::start(PathBuf::from(root)).map_err(|error| error.to_string())?;
    let result = service.effect_ping().map_err(|error| error.to_string())?;
    print_json(&serde_json::to_value(result).map_err(|error| error.to_string())?)
}

fn command_ipc(args: &[String]) -> Result<(), String> {
    let subcommand = args.get(1).map(String::as_str).unwrap_or("status");
    let root = require_arg(args, "--root")?;
    let socket = require_arg(args, "--socket")?;
    let service = PlatformService::start(PathBuf::from(root)).map_err(|error| error.to_string())?;
    let token = service.owner_token().map_err(|error| error.to_string())?;
    let client = IpcClient::new(socket.to_owned());
    let request: IpcRequest = match subcommand {
        "status" => request_status(&token),
        "stop-status" => request_stop_status(&token),
        "stop-set" => {
            let reason = require_arg(args, "--reason")?;
            request_stop_set(&token, reason)
        }
        "stop-clear" => request_stop_clear(&token),
        "effect-ping" => IpcRequest::new("platform.effect.ping", json!({}), token),
        other => return Err(format!("unknown ipc command: {other}")),
    };
    let response = client.call(&request).map_err(|error| error.to_string())?;
    let value = serde_json::to_value(response).map_err(|error| error.to_string())?;
    print_json(&value)
}

fn command_service(args: &[String]) -> Result<(), String> {
    let subcommand = args.get(1).map(String::as_str).unwrap_or("status");
    let root = require_arg(args, "--root")?;
    let manager = ServiceManager::new(PathBuf::from(root)).map_err(|error| error.to_string())?;
    match subcommand {
        "start" => {
            let token = owner_token(root)?;
            let state = manager.start(&token).map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(state).map_err(|error| error.to_string())?)
        }
        "stop" => {
            let token = owner_token(root)?;
            let report = manager.stop(&token).map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(report).map_err(|error| error.to_string())?)
        }
        "status" => {
            let status = manager.status();
            print_json(&serde_json::to_value(status).map_err(|error| error.to_string())?)
        }
        other => Err(format!("unknown service command: {other}")),
    }
}

fn command_gui(args: &[String]) -> Result<(), String> {
    let root = require_arg(args, "--root")?;
    let port = optional_arg(args, "--port")
        .unwrap_or("0")
        .parse::<u16>()
        .map_err(|_| "--port must be a valid TCP port".to_owned())?;
    let allow_mutations = has_flag(args, "--allow-mutations");
    let options = DashboardOptions {
        bind_addr: std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        allow_mutations,
    };
    let handle =
        DashboardServer::bind(PathBuf::from(root), options).map_err(|error| error.to_string())?;
    print_json(&json!({
        "schema": "aegis.cli.gui.v1",
        "url": format!("http://{}/", handle.addr()),
        "allow_mutations": allow_mutations,
        "note": if allow_mutations {
            "mutation controls are enabled and bound to this local page session"
        } else {
            "read-only dashboard; restart with --allow-mutations for lifecycle controls"
        }
    }))?;
    handle.run().map_err(|error| error.to_string())
}

fn command_tui(args: &[String]) -> Result<(), String> {
    let root = require_arg(args, "--root")?;
    let refresh_ms = optional_arg(args, "--refresh-ms")
        .unwrap_or("2000")
        .parse::<u64>()
        .map_err(|_| "--refresh-ms must be a nonnegative integer".to_owned())?;
    if has_flag(args, "--once") {
        aegis_ui::run_tui_once(root).map_err(|error| error.to_string())
    } else {
        aegis_ui::run_tui(root, refresh_ms).map_err(|error| error.to_string())
    }
}

fn command_backup(args: &[String]) -> Result<(), String> {
    let root = require_arg(args, "--root")?;
    let destination = require_arg(args, "--to")?;
    let report = backup(PathBuf::from(root), PathBuf::from(destination))
        .map_err(|error| error.to_string())?;
    print_json(&json!({
        "schema": "aegis.cli.backup.v1",
        "destination": report.destination,
        "file_count": report.file_count,
        "total_bytes": report.total_bytes,
        "manifest": report.manifest,
    }))
}

fn command_verify_backup(args: &[String]) -> Result<(), String> {
    let path = require_arg(args, "--path")?;
    let verification = verify_backup(PathBuf::from(path)).map_err(|error| error.to_string())?;
    let issues: Vec<Value> = verification
        .issues
        .iter()
        .map(|issue| {
            json!({
                "kind": format!("{:?}", issue.kind),
                "relative_path": issue.relative_path,
                "detail": issue.detail,
            })
        })
        .collect();
    print_json(&json!({
        "schema": "aegis.cli.backup_verification.v1",
        "destination": verification.destination,
        "status": format!("{:?}", verification.status),
        "file_count": verification.file_count,
        "total_bytes": verification.total_bytes,
        "issues": issues,
    }))
}

fn command_restore(args: &[String]) -> Result<(), String> {
    let source = require_arg(args, "--from")?;
    let target = require_arg(args, "--root")?;
    let options = RestoreOptions {
        allow_existing: has_flag(args, "--allow-existing"),
        backup_existing_to: optional_arg(args, "--backup-existing-to").map(PathBuf::from),
    };
    let report = restore(PathBuf::from(source), PathBuf::from(target), options)
        .map_err(|error| error.to_string())?;
    print_json(&json!({
        "schema": "aegis.cli.restore.v1",
        "source_backup": report.source_backup,
        "target": report.target,
        "restored_cores": report.restored_cores,
        "preserved_source": report.preserved_source,
        "file_count": report.file_count,
        "total_bytes": report.total_bytes,
    }))
}

fn command_provider(args: &[String]) -> Result<(), String> {
    let subcommand = args.get(1).map(String::as_str).unwrap_or("health");
    let base_url = optional_arg(args, "--url")
        .map(str::to_owned)
        .or_else(|| std::env::var("AEGIS_OLLAMA_URL").ok())
        .unwrap_or_else(|| DEFAULT_OLLAMA_BASE_URL.to_owned());
    let provider = OllamaProvider::new(base_url.clone()).map_err(|error| error.to_string())?;
    match subcommand {
        "health" => {
            let health = provider.health();
            print_json(&serde_json::to_value(health).map_err(|error| error.to_string())?)
        }
        "list" => {
            let models = provider.list_models().map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(models).map_err(|error| error.to_string())?)
        }
        "chat" => {
            let model = require_arg(args, "--model")?;
            let prompt = require_arg(args, "--prompt")?;
            let timeout_ms = optional_arg(args, "--timeout-ms").map(|value| {
                value
                    .parse::<u64>()
                    .map_err(|_| "--timeout-ms must be a nonnegative integer".to_owned())
            });
            let timeout_ms = match timeout_ms {
                Some(Ok(value)) => Some(value),
                Some(Err(error)) => return Err(error),
                None => None,
            };
            let request = ChatRequest {
                model: model.to_owned(),
                messages: vec![ChatMessage::user(prompt)],
                temperature: None,
                max_tokens: None,
                timeout_ms,
                tools: Vec::new(),
                think: if has_flag(args, "--no-think") {
                    Some(false)
                } else {
                    None
                },
            };
            let cancel = CancelToken::new();
            if has_flag(args, "--stream") {
                let mut chunks = Vec::new();
                let response = provider
                    .chat_stream(&request, &cancel, &mut |chunk| {
                        chunks.push(json!({"delta": chunk.delta, "done": chunk.done}));
                        Ok(())
                    })
                    .map_err(|error| error.to_string())?;
                print_json(&json!({
                    "schema": "aegis.cli.provider_stream.v1",
                    "response": response,
                    "chunks": chunks,
                }))
            } else {
                let response = provider
                    .chat(&request, &cancel)
                    .map_err(|error| error.to_string())?;
                print_json(&serde_json::to_value(response).map_err(|error| error.to_string())?)
            }
        }
        "embed" => {
            let model = require_arg(args, "--model")?;
            let text = require_arg(args, "--text")?;
            let embedding = OllamaEmbeddingProvider::new(&base_url, model)
                .map_err(|error| error.to_string())?;
            let vectors = embedding
                .embed(&[text.to_owned()])
                .map_err(|error| error.to_string())?;
            let vector = vectors.into_iter().next().unwrap_or_default();
            print_json(&json!({
                "schema": "aegis.cli.provider_embedding.v1",
                "provider": embedding.name(),
                "model": embedding.model(),
                "dimensions": vector.len(),
                "vector": vector,
            }))
        }
        other => Err(format!("unknown provider command: {other}")),
    }
}

fn command_memory(args: &[String]) -> Result<(), String> {
    let subcommand = args.get(1).map(String::as_str).unwrap_or("search");
    let root = require_arg(args, "--root")?;
    let core_uuid = args
        .get(2)
        .ok_or_else(|| "missing positional Core UUID".to_owned())?;
    let store =
        CoreMemoryStore::open(PathBuf::from(root), core_uuid).map_err(|error| error.to_string())?;
    match subcommand {
        "search" => {
            let text = optional_arg(args, "--text").map(str::to_owned);
            let kind = match optional_arg(args, "--kind") {
                Some(value) => Some(MemoryKind::parse(value).map_err(|error| error.to_string())?),
                None => None,
            };
            let limit = optional_arg(args, "--limit")
                .unwrap_or("100")
                .parse::<usize>()
                .map_err(|_| "--limit must be a nonnegative integer".to_owned())?;
            let query = MemoryQuery {
                text,
                kind,
                include_archived: has_flag(args, "--include-archived"),
                include_deleted: has_flag(args, "--include-deleted"),
                limit,
                offset: optional_arg(args, "--offset")
                    .unwrap_or("0")
                    .parse::<usize>()
                    .map_err(|_| "--offset must be a nonnegative integer".to_owned())?,
            };
            let entries = store.search(&query).map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(entries).map_err(|error| error.to_string())?)
        }
        "create" => {
            require_owner_proof(root)?;
            let content = require_arg(args, "--content")?;
            let source = optional_arg(args, "--source").unwrap_or("owner_cli");
            let provenance = optional_arg(args, "--provenance").unwrap_or("owner_cli");
            let kind = match optional_arg(args, "--kind") {
                Some(value) => MemoryKind::parse(value).map_err(|error| error.to_string())?,
                None => MemoryKind::Fact,
            };
            let tags: Vec<String> = optional_arg(args, "--tags")
                .map(|value| {
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|tag| !tag.is_empty())
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            let new = NewMemory::new(kind, content)
                .with_source(source)
                .with_provenance(provenance)
                .with_tags(tags);
            let entry = store.create(&new).map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(entry).map_err(|error| error.to_string())?)
        }
        "export" => {
            let export = store.export().map_err(|error| error.to_string())?;
            let encoded =
                serde_json::to_string_pretty(&export).map_err(|error| error.to_string())?;
            match optional_arg(args, "--out") {
                Some(path) => {
                    fs::write(path, encoded)
                        .map_err(|error| format!("could not write memory export: {error}"))?;
                    print_json(&json!({"schema": "aegis.cli.memory_export.v1", "path": path}))
                }
                None => {
                    println!("{encoded}");
                    Ok(())
                }
            }
        }
        "personality" => {
            let personality = store.get_personality().map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(personality).map_err(|error| error.to_string())?)
        }
        other => Err(format!("unknown memory command: {other}")),
    }
}

fn command_chat(args: &[String]) -> Result<(), String> {
    let subcommand = args.get(1).map(String::as_str).unwrap_or("list");
    let root = require_arg(args, "--root")?;
    let core_uuid = args
        .get(2)
        .ok_or_else(|| "missing positional Core UUID".to_owned())?;
    let engine = ConversationEngine::open(PathBuf::from(root), core_uuid)
        .map_err(|error| error.to_string())?;

    match subcommand {
        "create" => {
            require_owner_proof(root)?;
            let session = engine
                .create_session(optional_arg(args, "--title"))
                .map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(session).map_err(|error| error.to_string())?)
        }
        "list" => {
            require_owner_proof(root)?;
            let sessions = engine
                .store()
                .list_sessions(
                    has_flag(args, "--include-archived"),
                    has_flag(args, "--include-deleted"),
                )
                .map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(sessions).map_err(|error| error.to_string())?)
        }
        "messages" => {
            require_owner_proof(root)?;
            let session_id = args
                .get(3)
                .ok_or_else(|| "missing positional session ID".to_owned())?;
            let limit = parse_usize(args, "--limit", 50)?;
            let offset = parse_usize(args, "--offset", 0)?;
            let messages = engine
                .store()
                .messages(session_id, limit, offset)
                .map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(messages).map_err(|error| error.to_string())?)
        }
        "export" => {
            require_owner_proof(root)?;
            let session_id = args
                .get(3)
                .ok_or_else(|| "missing positional session ID".to_owned())?;
            let export = engine
                .store()
                .export_session(session_id)
                .map_err(|error| error.to_string())?;
            match optional_arg(args, "--out") {
                Some(path) => {
                    let encoded =
                        serde_json::to_string_pretty(&export).map_err(|error| error.to_string())?;
                    fs::write(path, encoded)
                        .map_err(|error| format!("could not write conversation export: {error}"))?;
                    print_json(&json!({
                        "schema": "aegis.cli.chat_export.v1",
                        "path": path,
                        "sessions": 1,
                    }))
                }
                None => {
                    print_json(&serde_json::to_value(export).map_err(|error| error.to_string())?)
                }
            }
        }
        "archive" | "delete" => {
            require_owner_proof(root)?;
            let session_id = args
                .get(3)
                .ok_or_else(|| "missing positional session ID".to_owned())?;
            let session = if subcommand == "archive" {
                engine.store().archive_session(session_id)
            } else {
                engine.store().delete_session(session_id)
            }
            .map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(session).map_err(|error| error.to_string())?)
        }
        "send" => {
            require_owner_proof(root)?;
            let session_id = args
                .get(3)
                .ok_or_else(|| "missing positional session ID".to_owned())?;
            let model = require_arg(args, "--model")?;
            let prompt = require_arg(args, "--prompt")?;
            let base_url = optional_arg(args, "--url")
                .map(str::to_owned)
                .or_else(|| std::env::var("AEGIS_OLLAMA_URL").ok())
                .unwrap_or_else(|| DEFAULT_OLLAMA_BASE_URL.to_owned());
            let provider =
                OllamaProvider::new(base_url.clone()).map_err(|error| error.to_string())?;
            let engine = match optional_arg(args, "--retrieval").unwrap_or("keyword") {
                "keyword" => engine,
                "local-hash" => engine.with_retriever(Box::new(
                    EmbeddingRetriever::open(
                        PathBuf::from(root),
                        core_uuid,
                        Box::new(HashingEmbeddingProvider::new(256)),
                    )
                    .map_err(|error| error.to_string())?,
                )),
                "ollama" => {
                    let model =
                        optional_arg(args, "--embedding-model").unwrap_or("nomic-embed-text");
                    let embedding = OllamaEmbeddingProvider::new(&base_url, model)
                        .map_err(|error| error.to_string())?;
                    engine.with_retriever(Box::new(
                        EmbeddingRetriever::open(
                            PathBuf::from(root),
                            core_uuid,
                            Box::new(embedding),
                        )
                        .map_err(|error| error.to_string())?,
                    ))
                }
                other => return Err(format!("unknown retrieval method: {other}")),
            };
            let mut options = TurnOptions::new(model)
                .with_memory_limit(parse_usize(args, "--memory-limit", 6)?)
                .with_history_limit(parse_usize(args, "--history-limit", 40)?);
            if has_flag(args, "--no-think") {
                options = options.with_think(Some(false));
            }
            if let Some(value) = optional_arg(args, "--temperature") {
                options = options.with_temperature(
                    value
                        .parse::<f32>()
                        .map_err(|_| "--temperature must be a number".to_owned())?,
                );
            }
            if let Some(value) = optional_arg(args, "--max-tokens") {
                options = options.with_max_tokens(
                    value
                        .parse::<u32>()
                        .map_err(|_| "--max-tokens must be an integer".to_owned())?,
                );
            }
            if let Some(value) = optional_arg(args, "--timeout-ms") {
                options = options.with_timeout_ms(
                    value
                        .parse::<u64>()
                        .map_err(|_| "--timeout-ms must be an integer".to_owned())?,
                );
            }
            let cancel = CancelToken::new();
            let turn = engine
                .send_turn(session_id, prompt, &provider, &options, &cancel)
                .map_err(|error| error.to_string())?;

            let mut tool_results = Value::Null;
            let mut follow_up = Value::Null;
            if has_flag(args, "--execute-tools") && !turn.response.tool_calls.is_empty() {
                let token = owner_token(root)?;
                let approval = if has_flag(args, "--approve-tools") {
                    Some(token.as_str())
                } else {
                    None
                };
                let executor = ToolExecutor::new(PathBuf::from(root), core_uuid)
                    .map_err(|error| error.to_string())?;
                let records = engine
                    .execute_tool_calls(session_id, &turn.response.tool_calls, &executor, approval)
                    .map_err(|error| error.to_string())?;
                tool_results = serde_json::to_value(&records).map_err(|error| error.to_string())?;
                if has_flag(args, "--follow-up") {
                    let follow = engine
                        .continue_after_tools(session_id, &provider, &options, &cancel)
                        .map_err(|error| error.to_string())?;
                    follow_up = json!({
                        "message_id": follow.assistant.message_id,
                        "content": follow.assistant.content,
                    });
                }
            }

            print_json(&json!({
                "schema": "aegis.cli.chat_turn.v1",
                "session_id": turn.session.session_id,
                "user_message_id": turn.user.message_id,
                "assistant_message_id": turn.assistant.message_id,
                "content": turn.assistant.content,
                "model": turn.response.model,
                "prompt_tokens": turn.response.prompt_eval_count,
                "completion_tokens": turn.response.eval_count,
                "tool_calls": turn.response.tool_calls,
                "tool_results": tool_results,
                "follow_up": follow_up,
                "memory_used": turn.memory_used,
                "retrieval_method": turn.retrieval_method,
                "retrieval_semantic": turn.retrieval_semantic,
            }))
        }
        other => Err(format!("unknown chat command: {other}")),
    }
}

fn parse_usize(args: &[String], name: &str, default: usize) -> Result<usize, String> {
    match optional_arg(args, name) {
        Some(value) => value
            .parse::<usize>()
            .map_err(|_| format!("{name} must be a nonnegative integer")),
        None => Ok(default),
    }
}

fn command_asset(args: &[String]) -> Result<(), String> {
    let subcommand = args.get(1).map(String::as_str).unwrap_or("list");
    let root = require_arg(args, "--root")?;
    let core_uuid = args
        .get(2)
        .ok_or_else(|| "missing positional Core UUID".to_owned())?;
    let store =
        EmbodimentStore::open(PathBuf::from(root), core_uuid).map_err(|error| error.to_string())?;
    match subcommand {
        "import" => {
            require_owner_proof(root)?;
            let kind = args
                .get(3)
                .ok_or_else(|| "missing positional asset kind".to_owned());
            let kind = AssetKind::parse(kind?).map_err(|error| error.to_string())?;
            let file = args
                .get(4)
                .ok_or_else(|| "missing positional asset file".to_owned())?;
            let bytes = fs::read(file).map_err(|error| format!("could not read asset: {error}"))?;
            let name = optional_arg(args, "--name")
                .map(str::to_owned)
                .or_else(|| {
                    PathBuf::from(file)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .map(str::to_owned)
                })
                .ok_or_else(|| "asset name is required".to_owned())?;
            let media_type =
                optional_arg(args, "--media-type").unwrap_or("application/octet-stream");
            let tags = parse_tags(optional_arg(args, "--tags"));
            let asset = store
                .import_asset(NewAsset::new(kind, &name, media_type, &bytes).with_tags(tags))
                .map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(asset).map_err(|error| error.to_string())?)
        }
        "list" => {
            require_owner_proof(root)?;
            let kind = match optional_arg(args, "--kind") {
                Some(value) => Some(AssetKind::parse(value).map_err(|error| error.to_string())?),
                None => None,
            };
            let assets = store
                .list_assets(kind, has_flag(args, "--include-deleted"))
                .map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(assets).map_err(|error| error.to_string())?)
        }
        "show" => {
            require_owner_proof(root)?;
            let asset_id = args
                .get(3)
                .ok_or_else(|| "missing positional asset ID".to_owned())?;
            let asset = store.asset(asset_id).map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(asset).map_err(|error| error.to_string())?)
        }
        "read" => {
            require_owner_proof(root)?;
            let asset_id = args
                .get(3)
                .ok_or_else(|| "missing positional asset ID".to_owned())?;
            let out = require_arg(args, "--out")?;
            let bytes = store
                .read_asset(asset_id)
                .map_err(|error| error.to_string())?;
            fs::write(out, &bytes).map_err(|error| format!("could not write asset: {error}"))?;
            print_json(&json!({
                "schema": "aegis.cli.asset_read.v1",
                "asset_id": asset_id,
                "path": out,
                "bytes": bytes.len(),
            }))
        }
        "delete" => {
            require_owner_proof(root)?;
            let asset_id = args
                .get(3)
                .ok_or_else(|| "missing positional asset ID".to_owned())?;
            let asset = store
                .delete_asset(asset_id)
                .map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(asset).map_err(|error| error.to_string())?)
        }
        "export" => {
            require_owner_proof(root)?;
            let export = store.export().map_err(|error| error.to_string())?;
            match optional_arg(args, "--out") {
                Some(path) => {
                    let encoded =
                        serde_json::to_string_pretty(&export).map_err(|error| error.to_string())?;
                    fs::write(path, encoded)
                        .map_err(|error| format!("could not write asset export: {error}"))?;
                    print_json(&json!({
                        "schema": "aegis.cli.asset_export.v1",
                        "path": path,
                    }))
                }
                None => {
                    print_json(&serde_json::to_value(export).map_err(|error| error.to_string())?)
                }
            }
        }
        other => Err(format!("unknown asset command: {other}")),
    }
}

fn command_appearance(args: &[String]) -> Result<(), String> {
    let subcommand = args.get(1).map(String::as_str).unwrap_or("show");
    let root = require_arg(args, "--root")?;
    let core_uuid = args
        .get(2)
        .ok_or_else(|| "missing positional Core UUID".to_owned())?;
    let store =
        EmbodimentStore::open(PathBuf::from(root), core_uuid).map_err(|error| error.to_string())?;
    require_owner_proof(root)?;
    let appearance = match subcommand {
        "show" => store.appearance(),
        "set-avatar" => {
            if has_flag(args, "--clear") {
                store.set_avatar(None)
            } else {
                let asset_id = args
                    .get(3)
                    .ok_or_else(|| "missing positional asset ID".to_owned())?;
                store.set_avatar(Some(asset_id))
            }
        }
        "set-expression" => {
            let name = args
                .get(3)
                .ok_or_else(|| "missing positional expression name".to_owned())?;
            let asset_id = args
                .get(4)
                .ok_or_else(|| "missing positional asset ID".to_owned())?;
            store.set_expression(name, asset_id)
        }
        "remove-expression" => {
            let name = args
                .get(3)
                .ok_or_else(|| "missing positional expression name".to_owned())?;
            store.remove_expression(name)
        }
        "set-gesture" => {
            let name = args
                .get(3)
                .ok_or_else(|| "missing positional gesture name".to_owned())?;
            let asset_id = args
                .get(4)
                .ok_or_else(|| "missing positional asset ID".to_owned())?;
            store.set_gesture(name, asset_id)
        }
        "remove-gesture" => {
            let name = args
                .get(3)
                .ok_or_else(|| "missing positional gesture name".to_owned())?;
            store.remove_gesture(name)
        }
        "set-wardrobe" => {
            let list = args
                .get(3)
                .ok_or_else(|| "missing positional wardrobe asset list".to_owned())?;
            let ids = list
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect();
            store.set_wardrobe(ids)
        }
        "set-theme" => store.set_theme(Theme {
            primary: require_arg(args, "--primary")?.to_owned(),
            accent: require_arg(args, "--accent")?.to_owned(),
            tertiary: require_arg(args, "--tertiary")?.to_owned(),
        }),
        "set-voice" => store.set_voice(Some(VoiceProfile {
            voice_id: require_arg(args, "--voice-id")?.to_owned(),
            display_name: require_arg(args, "--display-name")?.to_owned(),
            sample_asset_id: optional_arg(args, "--sample").map(str::to_owned),
        })),
        "clear-voice" => store.set_voice(None),
        other => return Err(format!("unknown appearance command: {other}")),
    }
    .map_err(|error| error.to_string())?;
    print_json(&serde_json::to_value(appearance).map_err(|error| error.to_string())?)
}

fn command_tool(args: &[String]) -> Result<(), String> {
    let subcommand = args.get(1).map(String::as_str).unwrap_or("list");
    let root = require_arg(args, "--root")?;
    let paths = StoragePaths::new(PathBuf::from(root)).map_err(|error| error.to_string())?;
    let registry = ToolRegistry::new(paths);
    match subcommand {
        "list" => {
            let grants = registry.list().map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(grants).map_err(|error| error.to_string())?)
        }
        "grant" => {
            let tool = args
                .get(2)
                .ok_or_else(|| "missing positional tool name".to_owned())?;
            let token = owner_token(root)?;
            let approval = has_flag(args, "--require-approval");
            let grant = registry
                .set_permission(tool, true, approval, &token)
                .map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(grant).map_err(|error| error.to_string())?)
        }
        "revoke" => {
            let tool = args
                .get(2)
                .ok_or_else(|| "missing positional tool name".to_owned())?;
            let token = owner_token(root)?;
            let grant = registry
                .set_permission(tool, false, false, &token)
                .map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(grant).map_err(|error| error.to_string())?)
        }
        "execute" => {
            let core_uuid = args
                .get(2)
                .ok_or_else(|| "missing positional Core UUID".to_owned())?;
            let tool = args
                .get(3)
                .ok_or_else(|| "missing positional tool name".to_owned())?;
            let arguments = match optional_arg(args, "--args") {
                Some(value) => serde_json::from_str::<Value>(value)
                    .map_err(|error| format!("--args must be valid JSON: {error}"))?,
                None => json!({}),
            };
            let token = owner_token(root)?;
            let executor = ToolExecutor::new(PathBuf::from(root), core_uuid)
                .map_err(|error| error.to_string())?;
            let result = executor
                .execute(tool, &arguments, Some(&token))
                .map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(result).map_err(|error| error.to_string())?)
        }
        other => Err(format!("unknown tool command: {other}")),
    }
}

fn parse_tags(value: Option<&str>) -> Vec<String> {
    value
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|tag| !tag.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn require_owner_proof(root: &str) -> Result<(), String> {
    let token = owner_token(root)?;
    let paths = StoragePaths::new(PathBuf::from(root)).map_err(|error| error.to_string())?;
    let safe = aegis_safe::SafeStore::new(paths);
    if !safe
        .verify_owner(&token)
        .map_err(|error| error.to_string())?
    {
        return Err("owner authorization failed".to_owned());
    }
    Ok(())
}

fn require_arg<'a>(args: &'a [String], name: &str) -> Result<&'a str, String> {
    optional_arg(args, name).ok_or_else(|| format!("missing required argument {name}"))
}

fn optional_arg<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|argument| argument == name)
        .and_then(|index| args.get(index + 1))
        .map(String::as_str)
}

fn has_flag(args: &[String], name: &str) -> bool {
    args.iter().any(|argument| argument == name)
}

fn print_json(value: &Value) -> Result<(), String> {
    let encoded = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    println!("{encoded}");
    Ok(())
}

fn usage() {
    println!(
        "Aegis Platform CLI\n\
         usage:\n\
         aegis install --root <path> [--headless]\n\
         aegis upgrade --root <path> [--backup-to <new-directory>]\n\
         aegis uninstall --root <path> [--delete-data --confirm <callsign> --backup-to <new-directory>]\n\
         aegis init --root <path> [--print-owner-token]\n\
         aegis status --root <path>\n\
         aegis diagnostics --root <path>\n\
         aegis host status --root <path>\n\
         aegis host enroll|revoke --root <path>\n\
         aegis host history --root <path>\n\
         aegis safe status --root <path>\n\
         aegis safe set --root <path> --reason <text>\n\
         aegis safe clear --root <path>\n\
         aegis service start --root <path>\n\
         aegis service stop --root <path>\n\
         aegis service status --root <path>\n\
         aegis gui --root <path> [--port <n>] [--allow-mutations]\n\
         aegis tui --root <path> [--refresh-ms <n>] [--once]\n\
         aegis effect ping --root <path>\n\
         aegis core list --root <path>\n\
         aegis core profiles\n\
         aegis core profile <core-number>\n\
         aegis core apply-profile <core-id> --root <path>\n\
         aegis core install <core-id> --root <path> --roster <n> [--name <name>]\n\
         aegis core init <core-id> --root <path> --number <n> --name <name>\n\
         aegis core inspect <core-id> --root <path>\n\
         aegis core verify <core-id> --root <path>\n\
         aegis core rename <core-id> --root <path> --name <name>\n\
         aegis core deactivate <core-id> --root <path> --reason <text>\n\
         aegis core reactivate <core-id> --root <path> --reason <text>\n\
         aegis core uninstall-preserve <core-id> --root <path> --reason <text>\n\
         aegis core start <core-id> --root <path>\n\
         aegis core stop <core-id> --root <path>\n\
         aegis core runtime-status <core-id> --root <path>\n\
         aegis roster list\n\
         aegis roster show <core-number>\n\
         aegis memory search <core-id> --root <path> [--text <text>] [--kind <kind>]\n\
         aegis memory create <core-id> --root <path> --content <text> [--kind <kind>]\n\
         aegis memory export <core-id> --root <path> [--out <file>]\n\
         aegis memory personality <core-id> --root <path>\n\
         aegis chat create <core-id> --root <path> [--title <text>]\n\
         aegis chat list <core-id> --root <path> [--include-archived] [--include-deleted]\n\
         aegis chat messages <core-id> <session-id> --root <path> [--limit <n>] [--offset <n>]\n\
         aegis chat send <core-id> <session-id> --root <path> --model <name> --prompt <text> [--url <url>] [--retrieval keyword|local-hash|ollama] [--embedding-model <name>] [--execute-tools] [--approve-tools] [--follow-up] [--no-think]\n\
         aegis chat export <core-id> <session-id> --root <path> [--out <file>]\n\
         aegis chat archive|delete <core-id> <session-id> --root <path>\n\
         aegis tool list --root <path>\n\
         aegis tool grant|revoke <tool-name> --root <path> [--require-approval]\n\
         aegis tool execute <core-id> <tool-name> --root <path> [--args <json>]\n\
         aegis social group create <name> --root <path>\n\
         aegis social group list|show|members|messages|events ... --root <path> [--as-core <uuid>]\n\
         aegis social group add|remove <group-id> --member-core <uuid>|--member-owner --root <path>\n\
         aegis social group post <group-id> --content <text> [--from-core <uuid>] --root <path>\n\
         aegis social relationship list|set|delete|events <core-id> ... --root <path>\n\
         aegis fleet host add <callsign> --endpoint <url> --role inference --root <path> [--priority <n>]\n\
         aegis fleet host list|enroll|revoke|remove|probe ... --root <path>\n\
         aegis fleet placement|route --root <path> [--model <name>]\n\
         aegis fleet gpu status|admission|running --root <path>\naegis fleet gpu yield --model <name> --root <path> [--url <url>]\naegis fleet gpu supervise --root <path> [--iterations <n>] [--interval-ms <n>] [--busy-samples <n>] [--idle-samples <n>] [--dry-run]\n\
         aegis fleet policy show|set --root <path> [--no-gaming-yield] [--max-utilization <n>] [--min-free-vram <n>] [--max-temperature <n>]\n\
         aegis asset import <core-id> <kind> <file> --root <path> [--name <name>] [--media-type <type>] [--tags a,b]\n\
         aegis asset list <core-id> --root <path> [--kind <kind>] [--include-deleted]\n\
         aegis asset show|delete <core-id> <asset-id> --root <path>\n\
         aegis asset read <core-id> <asset-id> --root <path> --out <file>\n\
         aegis asset export <core-id> --root <path> [--out <file>]\n\
         aegis appearance show <core-id> --root <path>\n\
         aegis appearance set-avatar <core-id> [<asset-id> | --clear] --root <path>\n\
         aegis appearance set-expression|set-gesture <core-id> <name> <asset-id> --root <path>\n\
         aegis appearance remove-expression|remove-gesture <core-id> <name> --root <path>\n\
         aegis appearance set-wardrobe <core-id> <asset-id,...> --root <path>\n\
         aegis appearance set-theme <core-id> --primary <hex> --accent <hex> --tertiary <hex> --root <path>\n\
         aegis appearance set-voice <core-id> --voice-id <id> --display-name <name> [--sample <asset-id>] --root <path>\n\
         aegis appearance clear-voice <core-id> --root <path>\n\
         aegis module list --root <path>\n\
         aegis module install <manifest> --root <path> [--sha256 <digest>]\n\
         aegis module install-package <package-dir> --root <path> [--sha256 <digest>]\n\
         aegis module enable|disable|uninstall|inspect|verify <module-id> --root <path>\n\
         aegis module tools <module-id> --root <path>\n\
         aegis module start|stop|restart|runtime-status <module-id> --root <path>\n\
         aegis module runtime-list --root <path>\n\
         aegis provider health [--url <ollama-base-url>]\n\
         aegis provider list [--url <ollama-base-url>]\n\
         aegis provider chat --model <name> --prompt <text> [--url <url>] [--stream] [--no-think]\n\
         aegis provider embed --model <name> --text <text> [--url <url>]\n\
         aegis backup --root <path> --to <new-directory>\n\
         aegis verify-backup --path <backup-directory>\n\
         aegis restore --from <backup-directory> --root <target-root> [--allow-existing]\n\
         aegis serve --root <path> --socket <name> [--requests <n>; 0 = until shutdown]\n\
         aegis ipc status|stop-status|stop-set|stop-clear|effect-ping --root <path> --socket <name>\n\
         \n\
         Local development build. Provisional items A-04, B-01, and B-06 remain open. \n\
         GUI is a loopback HTTP dashboard, not a native window."
    );
}

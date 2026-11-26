//! Snapshot terminal dashboard over the same real APIs as the HTTP dashboard.
//!
//! This is an honest console snapshot: each redraw re-reads Platform, Core,
//! module, and diagnostics state. It is not a live event stream.

use std::io::Write;
use std::path::Path;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use crate::error::UiError;
use crate::model::{self, DashboardModel, StatusClass};

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const PRIMARY: &str = "\x1b[38;2;148;0;211m";
const ACCENT: &str = "\x1b[38;2;0;194;255m";
const TERTIARY: &str = "\x1b[38;2;57;255;20m";
const MUTED: &str = "\x1b[38;2;139;129;163m";
const DANGER: &str = "\x1b[38;2;255;77;109m";
const CLEAR_SCREEN: &str = "\x1b[2J\x1b[H";

/// Render one honest snapshot of the local Platform as an ANSI terminal page.
pub fn render_dashboard(root: impl AsRef<Path>) -> String {
    let model = DashboardModel::collect(root.as_ref());
    render_model(&model)
}

/// Render a dashboard model. Sections whose data source is not linked are
/// labelled `UNAVAILABLE`; empty collections are labelled `NONE`.
pub(crate) fn render_model(model: &DashboardModel) -> String {
    let mut output = String::with_capacity(8192);
    output.push_str(CLEAR_SCREEN);
    output.push_str(&format!(
        "{BOLD}{PRIMARY}AEGIS{RESET} {MUTED}snapshot console dashboard{RESET}\n"
    ));
    output.push_str(&format!(
        "{MUTED}q quit  r refresh  digits ignored (no selection){RESET}\n\n"
    ));

    render_platform(&mut output, model);
    render_world(&mut output, model);
    render_personal(&mut output, model);
    render_cores(&mut output, model);
    render_fleet(&mut output, model);
    render_gpu(&mut output, model);
    render_inference(&mut output, model);
    render_memory(&mut output, model);
    render_database(&mut output, model);
    render_service(&mut output, model);
    render_modules(&mut output, model);
    render_issues(&mut output, model);

    output
}

fn status_color(class: StatusClass) -> &'static str {
    match class {
        StatusClass::Ok => TERTIARY,
        StatusClass::Warn => ACCENT,
        StatusClass::Error => DANGER,
    }
}

fn status_badge(class: StatusClass) -> String {
    format!("{}{}{RESET}", status_color(class), class.label())
}

fn render_platform(output: &mut String, model: &DashboardModel) {
    let class = model::classify_platform(model.platform.as_ref(), model.diagnostics.as_ref());
    output.push_str(&format!(
        "{ACCENT}Platform{RESET} {}\n",
        status_badge(class)
    ));
    match &model.platform {
        Some(status) => {
            output.push_str(&format!("  platform id : {}\n", status.platform_id));
            output.push_str(&format!(
                "  callsign    : {TERTIARY}{}{RESET}\n",
                status.callsign
            ));
            output.push_str(&format!("  zero-core   : {}\n", yes_no(status.zero_core)));
            output.push_str(&format!("  core count  : {}\n", status.core_count));
            if status.stop_active {
                output.push_str(&format!("  STOP        : {DANGER}ACTIVE{RESET}\n"));
            } else {
                output.push_str(&format!("  STOP        : {TERTIARY}inactive{RESET}\n"));
            }
            output.push_str(&format!("  authority   : {}\n", status.authority));
            output.push_str(&format!("  generated   : {}\n", status.generated_at));
            match &model.diagnostics {
                Some(report) => {
                    let color = match report.status.as_str() {
                        "healthy" => TERTIARY,
                        "degraded" => ACCENT,
                        _ => DANGER,
                    };
                    output.push_str(&format!(
                        "  diagnostics : {color}{}{RESET}\n",
                        report.status.as_str()
                    ));
                }
                None => {
                    let reason = model
                        .diagnostics_error
                        .as_deref()
                        .unwrap_or("no diagnostics data");
                    output.push_str(&format!(
                        "  diagnostics : {MUTED}UNAVAILABLE: {reason}{RESET}\n"
                    ));
                }
            }
        }
        None => {
            let reason = model
                .platform_error
                .as_deref()
                .unwrap_or("no platform data");
            output.push_str(&format!("  {DANGER}UNAVAILABLE: {reason}{RESET}\n"));
        }
    }
    output.push('\n');
}

fn render_world(output: &mut String, model: &DashboardModel) {
    output.push_str(&format!("{ACCENT}World{RESET}\n"));
    match &model.world {
        None => {
            let reason = model
                .world_snapshot
                .reason()
                .unwrap_or("no world status snapshot");
            output.push_str(&format!(
                "  {MUTED}{}: {reason}{RESET}\n",
                model.world_snapshot.label()
            ));
        }
        Some(world) => {
            if let Some(local_time) = &world.local_time {
                output.push_str(&format!("  local time  : {local_time}\n"));
            }
            if let Some(timezone) = &world.timezone {
                output.push_str(&format!("  timezone    : {timezone}\n"));
            }
            if let Some(time_of_day) = &world.time_of_day {
                output.push_str(&format!("  time of day : {time_of_day}\n"));
            }
            if let Some(season) = &world.season {
                output.push_str(&format!("  season      : {season}\n"));
            }
            if let Some(condition) = &world.condition {
                output.push_str(&format!("  weather     : {condition}\n"));
            }
            if let Some(temperature) = world.temperature_celsius {
                output.push_str(&format!("  temperature : {temperature:.1} C\n"));
            }
        }
    }
    output.push('\n');
}

fn render_personal(output: &mut String, model: &DashboardModel) {
    output.push_str(&format!("{ACCENT}Personal{RESET}\n"));
    match &model.personal {
        None => {
            let reason = model
                .personal_snapshot
                .reason()
                .unwrap_or("no personal status snapshot");
            output.push_str(&format!(
                "  {MUTED}{}: {reason}{RESET}\n",
                model.personal_snapshot.label()
            ));
        }
        Some(personal) => {
            if let Some(core_uuid) = &personal.core_uuid {
                output.push_str(&format!("  core        : {core_uuid}\n"));
            }
            if let Some(emotion) = &personal.emotion {
                output.push_str(&format!("  emotion     : {emotion}\n"));
            }
            if let Some(continuity) = &personal.continuity {
                output.push_str(&format!("  continuity  : {continuity}\n"));
            }
            if let Some(activities) = personal.activity_count {
                output.push_str(&format!("  activities  : {activities}\n"));
            }
            if let Some(routines) = personal.routine_count {
                output.push_str(&format!("  routines    : {routines}\n"));
            }
        }
    }
    output.push('\n');
}

fn render_cores(output: &mut String, model: &DashboardModel) {
    output.push_str(&format!("{ACCENT}Installed Cores{RESET}\n"));
    if model.cores.is_empty() {
        output.push_str(&format!("  {MUTED}NONE: no registered Cores{RESET}\n\n"));
        return;
    }
    for core in &model.cores {
        let class = model::classify_core(&core.runtime, &core.integrity);
        output.push_str(&format!(
            "  [{PRIMARY}{}{RESET}] {TERTIARY}{}{RESET}  {}  {}\n",
            core.core_number,
            core.core_uuid,
            core.display_name,
            status_badge(class)
        ));
        let heartbeat = if core.runtime.heartbeat_fresh {
            "fresh"
        } else {
            "not fresh"
        };
        output.push_str(&format!(
            "      lifecycle={} runtime={} heartbeat={} pid={}\n",
            core.lifecycle_state,
            core.runtime.state,
            heartbeat,
            core.runtime
                .pid
                .map_or_else(|| "-".to_owned(), |pid| pid.to_string())
        ));
        let integrity_color = status_color(class);
        output.push_str(&format!(
            "      integrity={integrity_color}{}{RESET}",
            core.integrity.status
        ));
        if let Some(detail) = &core.integrity.detail {
            output.push_str(&format!("  {MUTED}{detail}{RESET}"));
        }
        if let Some(error) = &core.runtime.last_error {
            output.push_str(&format!("  {DANGER}runtime: {error}{RESET}"));
        }
        output.push('\n');
    }
    output.push('\n');
}

fn render_fleet(output: &mut String, model: &DashboardModel) {
    let class = model::classify_fleet(model.fleet.as_ref());
    output.push_str(&format!(
        "{ACCENT}Fleet hosts{RESET} {}\n",
        status_badge(class)
    ));
    match &model.fleet {
        None => {
            let reason = model
                .fleet_snapshot
                .reason()
                .unwrap_or("no fleet status snapshot");
            output.push_str(&format!(
                "  {MUTED}{}: {reason}{RESET}\n",
                model.fleet_snapshot.label()
            ));
        }
        Some(fleet) if fleet.hosts.is_empty() => {
            output.push_str(&format!("  {MUTED}NONE: no fleet hosts reported{RESET}\n"))
        }
        Some(fleet) => {
            for host in &fleet.hosts {
                let reachability = if host.reachable {
                    "reachable"
                } else {
                    "unreachable"
                };
                let color = if host.reachable { TERTIARY } else { DANGER };
                output.push_str(&format!(
                    "  {color}{}{RESET} — {} models latency={}ms {}\n",
                    host.host_id, host.model_count, host.latency_ms, reachability
                ));
                if let Some(error) = &host.error {
                    output.push_str(&format!("      {DANGER}{error}{RESET}\n"));
                }
            }
        }
    }
    output.push('\n');
}

fn render_gpu(output: &mut String, model: &DashboardModel) {
    let class = model::classify_gpu(model.gpu.as_ref());
    output.push_str(&format!("{ACCENT}GPU{RESET} {}\n", status_badge(class)));
    match &model.gpu {
        None => {
            let reason = model
                .gpu_snapshot
                .reason()
                .unwrap_or("no GPU status snapshot");
            output.push_str(&format!(
                "  {MUTED}{}: {reason}{RESET}\n",
                model.gpu_snapshot.label()
            ));
        }
        Some(gpu) if !gpu.available => {
            let reason = gpu.error.as_deref().unwrap_or("no GPU telemetry reported");
            output.push_str(&format!("  {MUTED}UNAVAILABLE: {reason}{RESET}\n"));
        }
        Some(gpu) => {
            output.push_str(&format!(
                "  name        : {}\n",
                gpu.name.as_deref().unwrap_or("unknown")
            ));
            output.push_str(&format!(
                "  utilization : {}\n",
                option_percent(gpu.utilization_percent)
            ));
            output.push_str(&format!(
                "  free VRAM   : {}\n",
                option_mb(gpu.free_vram_mb)
            ));
            output.push_str(&format!(
                "  temperature : {}\n",
                option_celsius(gpu.temperature_c)
            ));
        }
    }
    output.push('\n');
}

fn render_inference(output: &mut String, model: &DashboardModel) {
    output.push_str(&format!("{ACCENT}Active inference{RESET}\n"));
    match &model.inference {
        None => {
            let reason = model
                .inference_snapshot
                .reason()
                .unwrap_or("no inference status snapshot");
            output.push_str(&format!(
                "  {MUTED}{}: {reason}{RESET}\n",
                model.inference_snapshot.label()
            ));
        }
        Some(inference) if !inference.active => {
            output.push_str(&format!("  {MUTED}NONE: no active inference{RESET}\n"));
        }
        Some(inference) => {
            output.push_str(&format!(
                "  model : {}\n",
                inference.model.as_deref().unwrap_or("unknown")
            ));
            output.push_str(&format!(
                "  host  : {}\n",
                inference.host_id.as_deref().unwrap_or("local")
            ));
            if let Some(detail) = &inference.detail {
                output.push_str(&format!("  detail: {detail}\n"));
            }
        }
    }
    output.push('\n');
}

fn render_memory(output: &mut String, model: &DashboardModel) {
    output.push_str(&format!("{ACCENT}Memory{RESET}\n"));
    match &model.memory {
        None => output.push_str(&format!(
            "  {MUTED}UNAVAILABLE: {}{RESET}\n",
            model::MEMORY_UNAVAILABLE_REASON
        )),
        Some(memory) if memory.total_bytes == 0 => {
            output.push_str(&format!("  {MUTED}NONE: no memory telemetry{RESET}\n"));
        }
        Some(memory) => {
            output.push_str(&format!(
                "  used : {} of {} bytes ({}%)\n",
                memory.used_bytes,
                memory.total_bytes,
                percent(memory.used_bytes, memory.total_bytes)
            ));
        }
    }
    output.push('\n');
}

fn render_database(output: &mut String, model: &DashboardModel) {
    let class = model::classify_database(&model.database);
    output.push_str(&format!(
        "{ACCENT}Database{RESET} {}\n",
        status_badge(class)
    ));
    let database = &model.database;
    output.push_str(&format!(
        "  platform.db : {}\n",
        option_bytes(database.platform_db_bytes)
    ));
    output.push_str(&format!(
        "  safe.db     : {}\n",
        option_bytes(database.safe_db_bytes)
    ));
    output.push_str(&format!(
        "  core dbs    : {}\n",
        option_bytes(database.core_db_bytes)
    ));
    if let Some(error) = &database.error {
        output.push_str(&format!("  {DANGER}{error}{RESET}\n"));
    }
    output.push('\n');
}

fn render_service(output: &mut String, model: &DashboardModel) {
    let class = model::classify_service(model.service.as_ref());
    output.push_str(&format!("{ACCENT}Service{RESET} {}\n", status_badge(class)));
    match &model.service {
        None => {
            let reason = model
                .service_snapshot
                .reason()
                .unwrap_or("no service status snapshot");
            output.push_str(&format!(
                "  {MUTED}{}: {reason}{RESET}\n",
                model.service_snapshot.label()
            ));
        }
        Some(service) => {
            output.push_str(&format!("  running : {}\n", yes_no(service.running)));
            output.push_str(&format!(
                "  pid     : {}\n",
                service
                    .pid
                    .map_or_else(|| "-".to_owned(), |pid| pid.to_string())
            ));
            output.push_str(&format!("  process : {}\n", yes_no(service.pid_alive)));
            output.push_str(&format!(
                "  socket  : {}\n",
                yes_no(service.socket_reachable)
            ));
            if let Some(error) = &service.error {
                output.push_str(&format!("  {DANGER}{error}{RESET}\n"));
            }
        }
    }
    output.push('\n');
}

fn render_modules(output: &mut String, model: &DashboardModel) {
    let modules = &model.modules;
    output.push_str(&format!("{ACCENT}Modules{RESET}\n"));
    output.push_str(&format!("  registry: {}\n", modules.status));
    if modules.modules.is_empty() {
        output.push_str(&format!("  {MUTED}NONE: no modules installed{RESET}\n"));
    }
    for module in &modules.modules {
        output.push_str(&format!(
            "  {} {} — {}\n",
            module.name,
            module.version,
            module.lifecycle_state.as_str()
        ));
    }
    if let Some(detail) = &modules.detail {
        output.push_str(&format!("  {MUTED}{detail}{RESET}\n"));
    }
    output.push('\n');
}

fn render_issues(output: &mut String, model: &DashboardModel) {
    output.push_str(&format!("{ACCENT}Errors and warnings{RESET}\n"));
    let issues = model::issues(model);
    let errors: Vec<&model::Issue> = issues
        .iter()
        .filter(|issue| issue.severity == model::Severity::Error)
        .collect();
    let warnings: Vec<&model::Issue> = issues
        .iter()
        .filter(|issue| issue.severity == model::Severity::Warn)
        .collect();

    if errors.is_empty() {
        output.push_str(&format!("  errors   : {TERTIARY}NONE{RESET}\n"));
    } else {
        for issue in errors {
            output.push_str(&format!(
                "  {DANGER}ERROR: {}{RESET} {MUTED}({}){RESET}\n",
                issue.message, issue.code
            ));
        }
    }
    if warnings.is_empty() {
        output.push_str(&format!("  warnings : {TERTIARY}NONE{RESET}\n"));
    } else {
        for issue in warnings {
            output.push_str(&format!(
                "  {ACCENT}WARN: {}{RESET} {MUTED}({}){RESET}\n",
                issue.message, issue.code
            ));
        }
    }
}

fn percent(used: u64, total: u64) -> u64 {
    if total == 0 {
        0
    } else {
        used.saturating_mul(100) / total
    }
}

fn option_bytes(value: Option<u64>) -> String {
    value.map_or_else(
        || "UNAVAILABLE".to_owned(),
        |bytes| format!("{bytes} bytes"),
    )
}

fn option_percent(value: Option<u8>) -> String {
    value.map_or_else(|| "UNAVAILABLE".to_owned(), |percent| format!("{percent}%"))
}

fn option_mb(value: Option<u64>) -> String {
    value.map_or_else(|| "UNAVAILABLE".to_owned(), |mb| format!("{mb} MB"))
}

fn option_celsius(value: Option<u8>) -> String {
    value.map_or_else(
        || "UNAVAILABLE".to_owned(),
        |celsius| format!("{celsius} C"),
    )
}

/// Print one snapshot and return.
pub fn run_tui_once(root: impl AsRef<Path>) -> std::io::Result<()> {
    let rendered = render_dashboard(root);
    let mut stdout = std::io::stdout();
    stdout.write_all(rendered.as_bytes())?;
    stdout.flush()
}

/// Run the snapshot dashboard until the user enters `q`.
pub fn run_tui(root: impl AsRef<Path>, refresh_ms: u64) -> Result<(), UiError> {
    let root = root.as_ref().to_path_buf();
    let (sender, receiver) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut lock = stdin.lock();
        loop {
            let mut line = String::new();
            match std::io::BufRead::read_line(&mut lock, &mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if sender.send(line.trim().to_owned()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let interval = Duration::from_millis(refresh_ms.max(250));
    print!("{}", render_dashboard(&root));
    let _ = std::io::stdout().flush();
    loop {
        match receiver.recv_timeout(interval) {
            Ok(command) => match command.as_str() {
                "q" => {
                    print!("{RESET}");
                    let _ = std::io::stdout().flush();
                    break;
                }
                "r" => {
                    print!("{}", render_dashboard(&root));
                    let _ = std::io::stdout().flush();
                }
                other => {
                    if !other.is_empty()
                        && other.chars().all(|character| character.is_ascii_digit())
                    {
                        print!("{}", render_dashboard(&root));
                        let _ = std::io::stdout().flush();
                    }
                }
            },
            Err(RecvTimeoutError::Timeout) => {
                print!("{}", render_dashboard(&root));
                let _ = std::io::stdout().flush();
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(())
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        CoreView, DatabaseView, FleetHostView, FleetView, GpuView, InferenceView, IntegrityView,
        MemoryView, ModulesView, PersonalView, RuntimeView, ServiceView, SnapshotState, WorldView,
    };
    use aegis_contracts::status::PlatformStatus;

    const CORE_UUID: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";

    fn platform() -> PlatformStatus {
        PlatformStatus {
            schema: "aegis.platform.status.v1".to_owned(),
            protocol: "aegis.platform.v1".to_owned(),
            platform_id: "platform-1".to_owned(),
            callsign: "Aegis".to_owned(),
            zero_core: false,
            core_count: 1,
            stop_active: false,
            authority: "local_single".to_owned(),
            generated_at: "2026-11-25T00:00:00Z".to_owned(),
        }
    }

    fn populated_model() -> DashboardModel {
        DashboardModel {
            platform: Some(platform()),
            platform_error: None,
            cores: vec![CoreView {
                core_uuid: CORE_UUID.to_owned(),
                core_number: 1,
                display_name: "Core A".to_owned(),
                lifecycle_state: "installed".to_owned(),
                runtime: RuntimeView {
                    state: "running".to_owned(),
                    heartbeat_fresh: true,
                    pid: Some(4242),
                    last_error: None,
                    heartbeat_at: Some("2026-11-25T00:00:00Z".to_owned()),
                },
                integrity: IntegrityView {
                    status: "verified".to_owned(),
                    detail: None,
                    expected_sha256: None,
                    actual_sha256: None,
                    expected_schema_revision: None,
                    actual_schema_revision: None,
                },
            }],
            modules: ModulesView {
                status: "available".to_owned(),
                detail: None,
                modules: Vec::new(),
            },
            diagnostics: None,
            diagnostics_error: None,
            database: DatabaseView {
                platform_db_bytes: Some(4096),
                safe_db_bytes: Some(1024),
                core_db_bytes: Some(8192),
                error: None,
            },
            fleet: Some(FleetView {
                hosts: vec![
                    FleetHostView {
                        host_id: "host-a".to_owned(),
                        reachable: true,
                        model_count: 3,
                        latency_ms: 4,
                        error: None,
                    },
                    FleetHostView {
                        host_id: "host-b".to_owned(),
                        reachable: true,
                        model_count: 1,
                        latency_ms: 9,
                        error: None,
                    },
                ],
            }),
            gpu: Some(GpuView {
                available: true,
                name: Some("RTX Test".to_owned()),
                utilization_percent: Some(12),
                free_vram_mb: Some(6144),
                temperature_c: Some(55),
                error: None,
            }),
            inference: Some(InferenceView {
                active: true,
                model: Some("aegis-small".to_owned()),
                host_id: Some("host-a".to_owned()),
                detail: None,
            }),
            service: Some(ServiceView {
                running: true,
                pid: Some(99),
                pid_alive: true,
                socket_reachable: true,
                error: None,
            }),
            memory: Some(MemoryView {
                used_bytes: 2_048,
                total_bytes: 8_192,
            }),
            world: Some(WorldView {
                local_time: Some("08:30".to_owned()),
                timezone: Some("UTC".to_owned()),
                time_of_day: Some("morning".to_owned()),
                season: Some("autumn".to_owned()),
                condition: Some("clear".to_owned()),
                temperature_celsius: Some(12.5),
            }),
            personal: Some(PersonalView {
                core_uuid: Some(CORE_UUID.to_owned()),
                emotion: Some("calm=0.80".to_owned()),
                continuity: Some("kept a journal".to_owned()),
                activity_count: Some(2),
                routine_count: Some(1),
            }),
            fleet_snapshot: SnapshotState::Fresh,
            gpu_snapshot: SnapshotState::Fresh,
            inference_snapshot: SnapshotState::Fresh,
            service_snapshot: SnapshotState::Fresh,
            world_snapshot: SnapshotState::Fresh,
            personal_snapshot: SnapshotState::Fresh,
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn empty_model() -> DashboardModel {
        DashboardModel {
            platform: None,
            platform_error: Some("Platform database is missing".to_owned()),
            cores: Vec::new(),
            modules: ModulesView {
                status: "unavailable".to_owned(),
                detail: Some("Platform database is missing".to_owned()),
                modules: Vec::new(),
            },
            diagnostics: None,
            diagnostics_error: Some("Diagnostics are unavailable".to_owned()),
            database: DatabaseView {
                platform_db_bytes: None,
                safe_db_bytes: None,
                core_db_bytes: None,
                error: None,
            },
            fleet: None,
            gpu: None,
            inference: None,
            service: None,
            memory: None,
            world: None,
            personal: None,
            fleet_snapshot: SnapshotState::Missing(model::FLEET_UNAVAILABLE_REASON.to_owned()),
            gpu_snapshot: SnapshotState::Missing(model::GPU_UNAVAILABLE_REASON.to_owned()),
            inference_snapshot: SnapshotState::Missing(
                model::INFERENCE_UNAVAILABLE_REASON.to_owned(),
            ),
            service_snapshot: SnapshotState::Missing(model::SERVICE_UNAVAILABLE_REASON.to_owned()),
            world_snapshot: SnapshotState::Missing(model::WORLD_UNAVAILABLE_REASON.to_owned()),
            personal_snapshot: SnapshotState::Missing(
                model::PERSONAL_UNAVAILABLE_REASON.to_owned(),
            ),
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn renders_every_section_for_populated_values() {
        let rendered = render_model(&populated_model());
        for section in [
            "Platform",
            "World",
            "Personal",
            "Installed Cores",
            "Fleet hosts",
            "GPU",
            "Active inference",
            "Memory",
            "Database",
            "Service",
            "Modules",
            "Errors and warnings",
        ] {
            assert!(rendered.contains(section), "missing section {section}");
        }
        assert!(rendered.contains(CORE_UUID));
        assert!(rendered.contains("Core A"));
        assert!(rendered.contains("host-a"));
        assert!(rendered.contains("aegis-small"));
        assert!(rendered.contains("6144 MB"));
        assert!(rendered.contains("25%"));
        assert!(rendered.contains("clear"));
        assert!(rendered.contains("kept a journal"));
        assert!(rendered.contains("errors   : \x1b[38;2;57;255;20mNONE"));
        assert!(!rendered.contains("UNAVAILABLE: aegis-fleet"));
        assert!(!rendered.contains("UNAVAILABLE: aegis-service"));
        assert!(!rendered.contains("UNAVAILABLE: aegis-cognition"));
    }

    #[test]
    fn renders_unavailable_and_none_states() {
        let rendered = render_model(&empty_model());
        assert!(rendered.contains("UNAVAILABLE: Platform database is missing"));
        assert!(rendered.contains("UNAVAILABLE: aegis-fleet is not a dependency"));
        assert!(rendered.contains("UNAVAILABLE: aegis-fleet GPU telemetry is not linked"));
        assert!(rendered.contains("UNAVAILABLE: aegis-cognition is not a dependency"));
        assert!(rendered.contains("UNAVAILABLE: no memory telemetry data source"));
        assert!(rendered.contains("UNAVAILABLE: aegis-service is not a dependency"));
        assert!(rendered.contains("UNAVAILABLE: no world status snapshot is available"));
        assert!(rendered.contains("UNAVAILABLE: no personal status snapshot is available"));
        assert!(rendered.contains("NONE: no registered Cores"));
        assert!(rendered.contains("NONE: no modules installed"));
        assert!(rendered.contains("platform.db : UNAVAILABLE"));
    }

    #[test]
    fn renders_none_for_empty_but_available_collections() {
        let mut model = empty_model();
        model.fleet = Some(FleetView { hosts: Vec::new() });
        model.inference = Some(InferenceView {
            active: false,
            model: None,
            host_id: None,
            detail: None,
        });
        model.memory = Some(MemoryView {
            used_bytes: 0,
            total_bytes: 0,
        });
        let rendered = render_model(&model);
        assert!(rendered.contains("NONE: no fleet hosts reported"));
        assert!(rendered.contains("NONE: no active inference"));
        assert!(rendered.contains("NONE: no memory telemetry"));
    }

    #[test]
    fn renders_errors_and_warnings_with_labels() {
        let mut model = populated_model();
        model.errors = vec!["core digest mismatch".to_owned()];
        model.warnings = vec!["module registry is unavailable".to_owned()];
        let rendered = render_model(&model);
        assert!(rendered.contains("ERROR: core digest mismatch"));
        assert!(rendered.contains("WARN: module registry is unavailable"));
    }

    #[test]
    fn status_badges_cover_all_three_levels() {
        assert_eq!(
            status_badge(StatusClass::Ok),
            format!("{TERTIARY}OK{RESET}")
        );
        assert_eq!(
            status_badge(StatusClass::Warn),
            format!("{ACCENT}WARN{RESET}")
        );
        assert_eq!(
            status_badge(StatusClass::Error),
            format!("{DANGER}ERROR{RESET}")
        );
    }

    #[test]
    fn percent_handles_zero_total() {
        assert_eq!(percent(0, 0), 0);
        assert_eq!(percent(1, 4), 25);
        assert_eq!(percent(8, 8), 100);
    }
}

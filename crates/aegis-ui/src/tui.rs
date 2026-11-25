//! Snapshot terminal dashboard over the same real APIs as the HTTP dashboard.
//!
//! This is an honest console snapshot: each redraw re-reads Platform, Core,
//! module, and diagnostics state. It is not a live event stream.

use std::io::Write;
use std::path::Path;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use crate::error::UiError;
use crate::model;

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
    let root = root.as_ref();
    let mut output = String::with_capacity(4096);
    output.push_str(CLEAR_SCREEN);

    output.push_str(&format!(
        "{BOLD}{PRIMARY}AEGIS{RESET} {MUTED}snapshot console dashboard{RESET}\n"
    ));
    output.push_str(&format!(
        "{MUTED}q quit  r refresh  digits ignored (no selection){RESET}\n\n"
    ));

    output.push_str(&format!("{ACCENT}Platform{RESET}\n"));
    match model::collect_status(root) {
        Ok(status) => {
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
        }
        Err(error) => {
            output.push_str(&format!("  {DANGER}unavailable: {error}{RESET}\n"));
        }
    }
    output.push('\n');

    output.push_str(&format!("{ACCENT}Core roster{RESET}\n"));
    let cores = model::collect_cores(root);
    if cores.is_empty() {
        output.push_str(&format!("  {MUTED}no registered Cores{RESET}\n"));
    }
    for core in &cores {
        output.push_str(&format!(
            "  [{PRIMARY}{}{RESET}] {TERTIARY}{}{RESET}  {}\n",
            core.core_number, core.core_uuid, core.display_name
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
        let integrity_color = if core.integrity.status == "verified" {
            TERTIARY
        } else {
            DANGER
        };
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

    output.push_str(&format!("{ACCENT}Modules{RESET}\n"));
    let modules = model::collect_modules(root);
    output.push_str(&format!("  registry: {}\n", modules.status));
    if modules.modules.is_empty() {
        output.push_str(&format!("  {MUTED}no modules installed{RESET}\n"));
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

    output.push_str(&format!("{ACCENT}Diagnostics{RESET}\n"));
    match model::collect_diagnostics(root) {
        Ok(report) => {
            let color = match report.status.as_str() {
                "healthy" => TERTIARY,
                "degraded" => ACCENT,
                _ => DANGER,
            };
            output.push_str(&format!(
                "  overall: {color}{}{RESET}\n",
                report.status.as_str()
            ));
            if report.reasons.is_empty() {
                output.push_str(&format!("  {MUTED}no active reasons{RESET}\n"));
            }
            for reason in &report.reasons {
                output.push_str(&format!("  {DANGER}- {reason}{RESET}\n"));
            }
        }
        Err(error) => {
            output.push_str(&format!("  {DANGER}unavailable: {error}{RESET}\n"));
        }
    }

    output
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

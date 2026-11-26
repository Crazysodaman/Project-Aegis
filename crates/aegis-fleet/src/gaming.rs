//! Gaming workload detection.
//!
//! GPU utilization alone is a weak signal: a game sitting in a menu, loading, or
//! paused can hold VRAM and a foreground window while issuing little GPU work.
//! This module adds an explicit, opt-in process detector so a configured game
//! executable is treated as a gaming workload even when utilization is low.
//!
//! Detection reads the real OS process list (`tasklist` on Windows, `ps` on
//! Unix). It never invents a result: a probe that cannot read the process list
//! is reported as `available = false` with an error, and the supervisor decides
//! how to treat the missing signal.

use std::collections::BTreeSet;
use std::process::{Command, Stdio};

use aegis_storage::now_rfc3339;
use serde::{Deserialize, Serialize};

/// Schema identifier for a gaming workload signal.
pub const GAMING_SIGNAL_SCHEMA: &str = "aegis.fleet.gaming_signal.v1";
/// Maximum configured game process names.
pub const MAX_GAME_PROCESSES: usize = 64;
/// Maximum length of a configured game process name.
pub const MAX_GAME_PROCESS_NAME_LEN: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GamingSignal {
    pub schema: String,
    pub available: bool,
    pub enabled: bool,
    pub detected: bool,
    pub source: String,
    pub matched_processes: Vec<String>,
    pub checked_at: String,
    pub error: Option<String>,
}

impl GamingSignal {
    pub fn none(source: &str) -> Self {
        Self {
            schema: GAMING_SIGNAL_SCHEMA.to_owned(),
            available: true,
            enabled: true,
            detected: false,
            source: source.to_owned(),
            matched_processes: Vec::new(),
            checked_at: now_rfc3339().unwrap_or_default(),
            error: None,
        }
    }

    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::none("disabled")
        }
    }

    pub fn detected(source: &str, matched: Vec<String>) -> Self {
        Self {
            detected: !matched.is_empty(),
            matched_processes: matched,
            ..Self::none(source)
        }
    }

    pub fn unavailable(source: &str, error: impl Into<String>) -> Self {
        Self {
            available: false,
            detected: false,
            source: source.to_owned(),
            error: Some(error.into()),
            ..Self::none(source)
        }
    }
}

/// Opt-in gaming detection policy.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GamingPolicy {
    pub enabled: bool,
    /// Executable stems (case-insensitive, without directory or `.exe`).
    pub processes: Vec<String>,
}

impl GamingPolicy {
    pub fn new(enabled: bool, processes: Vec<String>) -> Result<Self, String> {
        let policy = Self { enabled, processes };
        policy.validate()?;
        Ok(policy)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.processes.len() > MAX_GAME_PROCESSES {
            return Err(format!(
                "at most {MAX_GAME_PROCESSES} game processes are supported"
            ));
        }
        for process in &self.processes {
            let normalized = normalize_process_name(process);
            if normalized.is_empty() {
                return Err("game process names must not be empty".to_owned());
            }
            if process.chars().count() > MAX_GAME_PROCESS_NAME_LEN {
                return Err("game process name is too long".to_owned());
            }
            if process.chars().any(char::is_control) {
                return Err("game process name contains control characters".to_owned());
            }
        }
        Ok(())
    }

    fn normalized_set(&self) -> BTreeSet<String> {
        self.processes
            .iter()
            .map(|process| normalize_process_name(process))
            .filter(|process| !process.is_empty())
            .collect()
    }
}

pub trait GamingProbe: std::fmt::Debug {
    fn probe(&self) -> GamingSignal;
}

/// Process-list-backed gaming probe.
#[derive(Debug, Clone)]
pub struct ProcessGameProbe {
    policy: GamingPolicy,
}

impl ProcessGameProbe {
    pub fn new(policy: GamingPolicy) -> Result<Self, String> {
        policy.validate()?;
        Ok(Self { policy })
    }

    pub fn policy(&self) -> &GamingPolicy {
        &self.policy
    }
}

impl GamingProbe for ProcessGameProbe {
    fn probe(&self) -> GamingSignal {
        if !self.policy.enabled {
            return GamingSignal::disabled();
        }
        let running = match list_process_names() {
            Ok(names) => names,
            Err(error) => return GamingSignal::unavailable(process_source(), error),
        };
        let wanted = self.policy.normalized_set();
        let matched: BTreeSet<String> = running
            .into_iter()
            .filter(|name| wanted.contains(name))
            .collect();
        GamingSignal::detected(
            process_source(),
            matched.into_iter().collect::<Vec<String>>(),
        )
    }
}

fn process_source() -> &'static str {
    if cfg!(windows) {
        "tasklist"
    } else {
        "ps"
    }
}

/// Normalize an executable name to a comparison stem: basename, lowercased,
/// `.exe` stripped, trimmed.
pub fn normalize_process_name(raw: &str) -> String {
    let raw = raw.trim();
    let base = raw
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(raw)
        .trim()
        .to_ascii_lowercase();
    base.strip_suffix(".exe").unwrap_or(&base).trim().to_owned()
}

/// Read the current process list as normalized stems.
pub fn list_process_names() -> Result<Vec<String>, String> {
    let output = if cfg!(windows) {
        Command::new("tasklist")
            .args(["/FO", "CSV", "/NH"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
    } else {
        Command::new("ps")
            .args(["-eo", "comm="])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
    }
    .map_err(|error| format!("failed to read process list: {error}"))?;

    if !output.status.success() {
        return Err(format!("process listing exited with {}", output.status));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut names = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let first = if cfg!(windows) {
            parse_tasklist_first_field(line)
        } else {
            normalize_process_name(line)
        };
        if !first.is_empty() {
            names.push(first);
        }
    }
    Ok(names)
}

fn parse_tasklist_first_field(line: &str) -> String {
    if let Some(rest) = line.strip_prefix('"') {
        if let Some(end) = rest.find('"') {
            return normalize_process_name(&rest[..end]);
        }
        return normalize_process_name(rest);
    }
    line.split(',').next().unwrap_or(line).trim().to_owned()
}

/// Convenience: probe the live process list with a policy.
pub fn detect_gaming(policy: &GamingPolicy) -> GamingSignal {
    match ProcessGameProbe::new(policy.clone()) {
        Ok(probe) => probe.probe(),
        Err(error) => GamingSignal::unavailable("policy", error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_executable_names() {
        assert_eq!(normalize_process_name("Gears5.exe"), "gears5");
        assert_eq!(
            normalize_process_name(r"C:\Games\Doom\DOOM64.EXE"),
            "doom64"
        );
        assert_eq!(normalize_process_name("  steam  "), "steam");
        assert_eq!(normalize_process_name(""), "");
    }

    #[test]
    fn parses_tasklist_rows() {
        assert_eq!(
            parse_tasklist_first_field("\"Gears5.exe\",\"1234\",\"Console\",\"1\",\"1,234 K\""),
            "gears5"
        );
        assert_eq!(
            parse_tasklist_first_field("\"System Idle Process\",\"0\""),
            "system idle process"
        );
    }

    #[test]
    fn policy_validation_rejects_bad_names() {
        assert!(GamingPolicy::new(true, vec![String::new()]).is_err());
        assert!(GamingPolicy::new(true, vec!["bad\nname".to_owned()]).is_err());
        let too_long = "x".repeat(MAX_GAME_PROCESS_NAME_LEN + 1);
        assert!(GamingPolicy::new(true, vec![too_long]).is_err());
        assert!(GamingPolicy::new(true, vec!["Gears5.exe".to_owned()]).is_ok());
    }

    #[test]
    fn disabled_policy_does_not_probe() {
        let signal = detect_gaming(&GamingPolicy::default());
        assert!(signal.available);
        assert!(!signal.enabled);
        assert!(!signal.detected);
    }

    #[test]
    fn detects_a_known_live_process() {
        // `tasklist`/`ps` always include the current process image. Reading the
        // live list proves the probe path without launching anything.
        let current = std::env::current_exe()
            .ok()
            .and_then(|path| {
                path.file_stem()
                    .map(|stem| stem.to_string_lossy().to_string())
            })
            .expect("current exe name");
        let policy = GamingPolicy::new(true, vec![current]).expect("policy");
        let signal = detect_gaming(&policy);
        assert!(signal.available, "probe error: {:?}", signal.error);
        assert!(signal.detected, "current process was not detected");
        assert!(!signal.matched_processes.is_empty());
    }
}

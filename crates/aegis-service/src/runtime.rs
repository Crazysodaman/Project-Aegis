//! Read-only runtime capability detection.
//!
//! [`RuntimeSupport`] reports the operating-system family and whether systemd
//! markers exist on disk. Detection uses filesystem metadata only: it never
//! spawns a process, runs `systemctl`, opens a socket, or changes the host.
//! Presence is not proof of a working runtime, so a detected Linux systemd
//! installation is reported as [`RuntimeVerdict::Untested`], never as verified.

use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Conventional systemd system unit directory.
pub const SYSTEMD_UNIT_DIR: &str = "/etc/systemd/system";
/// Conventional marker that systemd is the running init system.
pub const SYSTEMD_RUNTIME_DIR: &str = "/run/systemd/system";
/// Candidate absolute paths for the `systemctl` client.
pub const SYSTEMCTL_CANDIDATES: &[&str] = &["/usr/bin/systemctl", "/bin/systemctl"];

/// Operating-system family relevant to service management.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OsFamily {
    Linux,
    Windows,
    Other,
}

impl OsFamily {
    /// The family of the current build target.
    pub fn current() -> Self {
        #[cfg(target_os = "linux")]
        {
            OsFamily::Linux
        }
        #[cfg(target_os = "windows")]
        {
            OsFamily::Windows
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            OsFamily::Other
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            OsFamily::Linux => "linux",
            OsFamily::Windows => "windows",
            OsFamily::Other => "other",
        }
    }
}

impl fmt::Display for OsFamily {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// How much confidence the probe has in Linux/systemd runtime support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeVerdict {
    /// Markers exist and support is expected, but the runtime was not exercised.
    Supported,
    /// The runtime cannot be supported on this host or is missing markers.
    Unsupported,
    /// Markers suggest support but nothing was started or verified.
    Untested,
}

impl RuntimeVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            RuntimeVerdict::Supported => "supported",
            RuntimeVerdict::Unsupported => "unsupported",
            RuntimeVerdict::Untested => "untested",
        }
    }
}

impl fmt::Display for RuntimeVerdict {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Injected inputs for the probe, so tests never depend on the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeInputs {
    pub os_family: OsFamily,
    pub systemd_unit_dir_present: bool,
    pub systemd_runtime_dir_present: bool,
    pub systemctl_present: bool,
}

impl RuntimeInputs {
    /// Conservative defaults for a family with no detected markers.
    pub fn for_family(os_family: OsFamily) -> Self {
        Self {
            os_family,
            systemd_unit_dir_present: false,
            systemd_runtime_dir_present: false,
            systemctl_present: false,
        }
    }

    /// Read-only path checks for the host this build is running on.
    pub fn current() -> Self {
        Self {
            os_family: OsFamily::current(),
            systemd_unit_dir_present: path_is_dir(SYSTEMD_UNIT_DIR),
            systemd_runtime_dir_present: path_is_dir(SYSTEMD_RUNTIME_DIR),
            systemctl_present: SYSTEMCTL_CANDIDATES
                .iter()
                .any(|candidate| path_is_file(candidate)),
        }
    }

    pub fn with_systemd_unit_dir(mut self, present: bool) -> Self {
        self.systemd_unit_dir_present = present;
        self
    }

    pub fn with_systemd_runtime_dir(mut self, present: bool) -> Self {
        self.systemd_runtime_dir_present = present;
        self
    }

    pub fn with_systemctl(mut self, present: bool) -> Self {
        self.systemctl_present = present;
        self
    }
}

/// Result of a read-only runtime probe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeSupport {
    pub os_family: OsFamily,
    /// The OS family has a Linux/systemd service manager at all.
    pub systemd_capable: bool,
    /// All systemd markers were found by path checks.
    pub systemd_present: bool,
    /// Confidence in the Linux runtime in this environment.
    pub linux_runtime: RuntimeVerdict,
    /// This probe only read filesystem metadata.
    pub read_only_probe: bool,
    /// This probe never spawned a process.
    pub process_spawned: bool,
    pub notes: String,
}

impl RuntimeSupport {
    /// True only when the Linux runtime was detected and is expected to work.
    pub fn linux_runtime_supported(&self) -> bool {
        self.linux_runtime == RuntimeVerdict::Supported
    }
}

/// Probe the current host using read-only path checks only.
pub fn runtime_support() -> RuntimeSupport {
    runtime_support_with(&RuntimeInputs::current())
}

/// Probe using injected inputs. Pure and deterministic.
pub fn runtime_support_with(inputs: &RuntimeInputs) -> RuntimeSupport {
    let systemd_capable = inputs.os_family == OsFamily::Linux;
    let systemd_present = systemd_capable
        && inputs.systemd_unit_dir_present
        && inputs.systemd_runtime_dir_present
        && inputs.systemctl_present;
    let (linux_runtime, notes) = match inputs.os_family {
        OsFamily::Windows => (
            RuntimeVerdict::Unsupported,
            "Non-Linux host (windows): systemd is not applicable. The Linux service runtime is \
             unsupported and untested in this environment."
                .to_owned(),
        ),
        OsFamily::Other => (
            RuntimeVerdict::Unsupported,
            "Unsupported OS family: no known service manager. The Linux service runtime is \
             unsupported and untested in this environment."
                .to_owned(),
        ),
        OsFamily::Linux if systemd_present => (
            RuntimeVerdict::Untested,
            "Linux systemd markers were found by read-only path checks; no service was started \
             or verified, so the runtime remains untested."
                .to_owned(),
        ),
        OsFamily::Linux => (
            RuntimeVerdict::Unsupported,
            "Linux host is missing one or more systemd markers; the systemd runtime is \
             unsupported."
                .to_owned(),
        ),
    };

    RuntimeSupport {
        os_family: inputs.os_family,
        systemd_capable,
        systemd_present,
        linux_runtime,
        read_only_probe: true,
        process_spawned: false,
        notes,
    }
}

fn path_is_dir(path: &str) -> bool {
    Path::new(path).is_dir()
}

fn path_is_file(path: &str) -> bool {
    Path::new(path).is_file()
}

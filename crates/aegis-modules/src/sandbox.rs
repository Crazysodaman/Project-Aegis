//! Honest isolation capability probe and a no-op sandbox abstraction.
//!
//! Project Aegis does **not** implement an operating-system sandbox for module
//! processes. This module exists so the platform can describe the truth without
//! guessing or over-claiming, and so a future sandbox can be added behind one
//! interface without changing callers.
//!
//! * [`IsolationSupport::probe`] reports exactly what is enforced today. On
//!   Windows and Linux the OS sandbox, filesystem jail, network isolation,
//!   memory limit, and CPU limit are all reported as unsupported.
//! * [`ModuleSandbox`] is the abstraction a future implementation would fill in.
//! * [`NoopSandbox`] implements that abstraction for the current baseline. It
//!   reports the same honest support matrix and refuses to claim enforcement.
//! * [`SandboxPolicy`] is pure data describing what a future sandbox *would*
//!   enforce (filesystem scope, network, child processes, memory/CPU bounds,
//!   descendant termination). It does not enforce anything by itself.
//!
//! Process-tree termination is the only capability reported as conditionally
//! available: Windows uses `taskkill /T /F`, which is process cleanup, not a
//! security boundary.

use std::process::Command;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Schema identifier for an isolation capability probe document.
pub const ISOLATION_SUPPORT_SCHEMA: &str = "aegis.module.isolation_support.v1";

/// What the current platform actually enforces for module processes.
///
/// The fields are intentionally conservative: a value is `true` only when the
/// runtime demonstrably performs that action using the standard library and
/// built-in OS commands. No field claims a security sandbox on Windows or Linux.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IsolationSupport {
    pub schema: String,
    pub platform: String,
    /// True only if a real OS sandbox is applied to module processes.
    pub os_sandbox: bool,
    /// True only if module filesystem access is confined by the OS.
    pub filesystem_jail: bool,
    /// True only if module network access is blocked by the OS.
    pub network_isolation: bool,
    /// True only if module memory usage is capped by the OS.
    pub memory_limit: bool,
    /// True only if module CPU usage is capped by the OS.
    pub cpu_limit: bool,
    /// True when forced termination also terminates descendant processes.
    pub process_tree_termination: bool,
    /// True when the child environment is cleared before spawning modules.
    pub cleared_environment: bool,
    /// True when the child working directory is set to the module package.
    pub package_working_directory: bool,
}

impl IsolationSupport {
    /// Probe the runtime's real isolation capabilities.
    ///
    /// This never consults the host for optional facilities it does not use; it
    /// reports the compile-time baseline of this crate so the answer cannot
    /// drift into a false claim.
    pub fn probe() -> Self {
        Self {
            schema: ISOLATION_SUPPORT_SCHEMA.to_owned(),
            platform: std::env::consts::OS.to_owned(),
            os_sandbox: false,
            filesystem_jail: false,
            network_isolation: false,
            memory_limit: false,
            cpu_limit: false,
            process_tree_termination: cfg!(windows),
            cleared_environment: true,
            package_working_directory: true,
        }
    }

    /// OS-level mechanisms that are not enforced on this platform.
    pub fn unsupported_mechanisms(&self) -> Vec<&'static str> {
        let mut items = Vec::new();
        if !self.os_sandbox {
            items.push("operating-system sandbox");
        }
        if !self.filesystem_jail {
            items.push("filesystem jail");
        }
        if !self.network_isolation {
            items.push("network isolation");
        }
        if !self.memory_limit {
            items.push("memory limit");
        }
        if !self.cpu_limit {
            items.push("cpu limit");
        }
        items
    }

    /// True when the platform makes no OS sandbox claim at all.
    pub fn is_unsandboxed(&self) -> bool {
        !self.os_sandbox && !self.filesystem_jail && !self.network_isolation
    }
}

/// Filesystem scope a future sandbox would enforce for a module process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilesystemScope {
    /// No filesystem access.
    None,
    /// Read access limited to the module package directory.
    ModulePackage,
    /// Write access limited to the module runtime scratch directory.
    RuntimeScratch,
    /// Unrestricted access; never a sandbox.
    Unrestricted,
}

/// Pure description of what a future sandbox would enforce.
///
/// This type does not enforce anything. It is a declarative target for an
/// owner-approved OS mechanism (Windows Job Objects/AppContainer, Linux
/// namespaces/seccomp/cgroups) and is useful for auditing an implementation
/// against the intended contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxPolicy {
    /// Read scope for the module process.
    pub filesystem_read: FilesystemScope,
    /// Write scope for the module process.
    pub filesystem_write: FilesystemScope,
    /// Whether outbound network access is allowed.
    pub allow_network: bool,
    /// Whether the module may spawn child processes.
    pub allow_process_spawn: bool,
    /// Optional OS-enforced memory cap.
    pub max_memory_bytes: Option<u64>,
    /// Optional OS-enforced CPU cap.
    pub max_cpu_millis: Option<u64>,
    /// Whether forced termination must also terminate descendants.
    pub terminate_descendants: bool,
}

impl Default for SandboxPolicy {
    fn default() -> Self {
        Self {
            filesystem_read: FilesystemScope::ModulePackage,
            filesystem_write: FilesystemScope::RuntimeScratch,
            allow_network: false,
            allow_process_spawn: false,
            max_memory_bytes: None,
            max_cpu_millis: None,
            terminate_descendants: true,
        }
    }
}

/// Why a sandbox could not be applied.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SandboxError {
    /// This build implements no operating-system sandbox.
    #[error("operating-system sandboxing is not supported on this platform")]
    Unsupported,
}

/// Abstraction a future OS sandbox would implement.
///
/// The current baseline is [`NoopSandbox`]. Adding a real implementation must
/// not change the honesty of [`IsolationSupport::probe`]: the probe lives in
/// this module and is meant to be updated only together with demonstrated
/// enforcement and dedicated security acceptance tests.
pub trait ModuleSandbox {
    /// Real capabilities of this sandbox instance.
    fn support(&self) -> IsolationSupport;

    /// Declarative policy this sandbox would enforce.
    fn policy(&self) -> &SandboxPolicy;

    /// Apply enforcement to a child command.
    ///
    /// The honest baseline returns [`SandboxError::Unsupported`]; a real
    /// implementation would attach the OS mechanism here.
    fn apply(&self, command: &mut Command) -> Result<(), SandboxError>;
}

/// Baseline sandbox that enforces nothing and says so.
#[derive(Debug, Clone, Default)]
pub struct NoopSandbox {
    policy: SandboxPolicy,
}

impl NoopSandbox {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_policy(policy: SandboxPolicy) -> Self {
        Self { policy }
    }
}

impl ModuleSandbox for NoopSandbox {
    fn support(&self) -> IsolationSupport {
        IsolationSupport::probe()
    }

    fn policy(&self) -> &SandboxPolicy {
        &self.policy
    }

    fn apply(&self, _command: &mut Command) -> Result<(), SandboxError> {
        Err(SandboxError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_never_claims_an_os_sandbox() {
        let support = IsolationSupport::probe();
        assert_eq!(support.schema, ISOLATION_SUPPORT_SCHEMA);
        assert!(!support.os_sandbox);
        assert!(!support.filesystem_jail);
        assert!(!support.network_isolation);
        assert!(!support.memory_limit);
        assert!(!support.cpu_limit);
        assert!(support.is_unsandboxed());
        assert!(support
            .unsupported_mechanisms()
            .contains(&"operating-system sandbox"));
    }

    #[test]
    fn probe_reports_environment_hygiene() {
        let support = IsolationSupport::probe();
        assert!(support.cleared_environment);
        assert!(support.package_working_directory);
        assert_eq!(support.process_tree_termination, cfg!(windows));
        assert_eq!(support.platform, std::env::consts::OS);
    }

    #[test]
    fn noop_sandbox_does_not_claim_enforcement() {
        let sandbox = NoopSandbox::new();
        assert_eq!(sandbox.support(), IsolationSupport::probe());
        assert_eq!(sandbox.policy(), &SandboxPolicy::default());
        let mut command = Command::new("aegis-noop");
        assert_eq!(sandbox.apply(&mut command), Err(SandboxError::Unsupported));
    }

    #[test]
    fn sandbox_policy_describes_the_future_contract() {
        let policy = SandboxPolicy::default();
        assert_eq!(policy.filesystem_read, FilesystemScope::ModulePackage);
        assert_eq!(policy.filesystem_write, FilesystemScope::RuntimeScratch);
        assert!(!policy.allow_network);
        assert!(!policy.allow_process_spawn);
        assert!(policy.terminate_descendants);
        let encoded = serde_json::to_string(&policy).expect("policy json");
        assert!(encoded.contains("module_package"));
        assert!(encoded.contains("runtime_scratch"));
    }
}

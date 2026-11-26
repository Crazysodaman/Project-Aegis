//! Module trust classification and execution policy.
//!
//! This layer adds an explicit, conservative contract on top of the existing
//! lifecycle and environment isolation. It does **not** introduce an operating
//! system sandbox and does not grant any data access.
//!
//! * [`ModuleTrust`] is assigned by the Platform/owner, never inferred from a
//!   manifest. A manifest cannot promote itself to `trusted_local`.
//! * [`ExecutionPolicy`] bounds wall-clock runtime, serialized input size,
//!   serialized output size, and the number of declared tools that may be
//!   executed. Limits are validated against the framework hard caps and are
//!   derived from existing manifest constants.
//! * A `restricted` module is denied by the runtime and the tool runner unless
//!   the caller passes a policy with `allow_restricted = true`. Lifecycle state
//!   `enabled` alone is not an explicit trust decision.
//!
//! True OS sandboxing (Windows Job Objects / AppContainer, Linux namespaces,
//! seccomp, cgroups, filesystem jails, network isolation) is **not**
//! implemented. See `docs/implementation/BATCH6_AGENT09_SOCIAL_MODULES.md` for
//! the available Windows/Linux isolation options and the dependency approvals
//! they would require.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::manifest::MAX_TOOLS;
use crate::tool_runner::{MAX_TOOL_INPUT_BYTES, MAX_TOOL_OUTPUT_BYTES};

/// Upper bound on a policy's wall-clock runtime limit.
///
/// This mirrors the manifest CPU budget cap so a policy can never request more
/// runtime than the manifest contract already allows.
pub const MAX_POLICY_RUNTIME_MS: u64 = crate::manifest::MAX_CPU_MILLIS;
/// Runtime limit used by the default (`trusted_local`) policy.
pub const DEFAULT_POLICY_RUNTIME_MS: u64 = MAX_POLICY_RUNTIME_MS;
/// Output byte limit used by the default (`trusted_local`) policy.
pub const DEFAULT_POLICY_OUTPUT_BYTES: usize = MAX_TOOL_OUTPUT_BYTES;
/// Input byte limit used by the default (`trusted_local`) policy.
pub const DEFAULT_POLICY_INPUT_BYTES: usize = MAX_TOOL_INPUT_BYTES;
/// Declared-tool count limit used by the default (`trusted_local`) policy.
pub const DEFAULT_POLICY_TOOL_COUNT: usize = MAX_TOOLS;
/// Runtime limit applied to the conservative `restricted` preset.
pub const RESTRICTED_POLICY_RUNTIME_MS: u64 = 60_000;
/// Output byte limit applied to the conservative `restricted` preset.
pub const RESTRICTED_POLICY_OUTPUT_BYTES: usize = 256 * 1024;
/// Input byte limit applied to the conservative `restricted` preset.
pub const RESTRICTED_POLICY_INPUT_BYTES: usize = 16 * 1024;
/// Declared-tool count limit applied to the conservative `restricted` preset.
pub const RESTRICTED_POLICY_TOOL_COUNT: usize = 8;

/// Trust assigned to a module by the Platform/owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleTrust {
    /// Locally authored code that the owner runs directly.
    TrustedLocal,
    /// Code that must be explicitly opted in before execution and is held to a
    /// tighter runtime/IO/tool budget.
    Restricted,
}

impl ModuleTrust {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TrustedLocal => "trusted_local",
            Self::Restricted => "restricted",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "trusted_local" => Some(Self::TrustedLocal),
            "restricted" => Some(Self::Restricted),
            _ => None,
        }
    }

    /// Restricted modules require an explicit `allow_restricted` policy.
    pub const fn is_restricted(self) -> bool {
        matches!(self, Self::Restricted)
    }
}

/// A validated set of execution limits for one module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionPolicy {
    pub trust: ModuleTrust,
    /// Maximum wall-clock lifetime for one process start or one tool call.
    pub max_runtime_ms: u64,
    /// Maximum serialized tool output accepted from the module.
    pub max_output_bytes: usize,
    /// Maximum serialized tool input sent to the module.
    pub max_input_bytes: usize,
    /// Maximum number of declared tools that may be executed.
    pub max_tool_count: usize,
    /// Explicit owner/Platform opt-in required to execute a restricted module.
    pub allow_restricted: bool,
}

impl Default for ExecutionPolicy {
    fn default() -> Self {
        Self::for_trust(ModuleTrust::TrustedLocal)
    }
}

impl ExecutionPolicy {
    /// Conservative preset for a trust level. `allow_restricted` always starts
    /// `false`; callers must set it explicitly to opt in.
    pub const fn for_trust(trust: ModuleTrust) -> Self {
        match trust {
            ModuleTrust::TrustedLocal => Self {
                trust,
                max_runtime_ms: DEFAULT_POLICY_RUNTIME_MS,
                max_output_bytes: DEFAULT_POLICY_OUTPUT_BYTES,
                max_input_bytes: DEFAULT_POLICY_INPUT_BYTES,
                max_tool_count: DEFAULT_POLICY_TOOL_COUNT,
                allow_restricted: false,
            },
            ModuleTrust::Restricted => Self {
                trust,
                max_runtime_ms: RESTRICTED_POLICY_RUNTIME_MS,
                max_output_bytes: RESTRICTED_POLICY_OUTPUT_BYTES,
                max_input_bytes: RESTRICTED_POLICY_INPUT_BYTES,
                max_tool_count: RESTRICTED_POLICY_TOOL_COUNT,
                allow_restricted: false,
            },
        }
    }

    /// Whether a restricted module passes the explicit opt-in gate.
    pub const fn permits_restricted(&self) -> bool {
        !self.trust.is_restricted() || self.allow_restricted
    }

    /// Rejects limits that are zero or exceed the framework hard caps.
    ///
    /// `max_tool_count` may be zero, which denies every declared tool.
    pub fn validate(&self) -> Result<(), PolicyError> {
        if self.max_runtime_ms == 0 || self.max_runtime_ms > MAX_POLICY_RUNTIME_MS {
            return Err(PolicyError::InvalidRuntime(self.max_runtime_ms));
        }
        if self.max_output_bytes == 0 || self.max_output_bytes > MAX_TOOL_OUTPUT_BYTES {
            return Err(PolicyError::InvalidOutput(self.max_output_bytes));
        }
        if self.max_input_bytes == 0 || self.max_input_bytes > MAX_TOOL_INPUT_BYTES {
            return Err(PolicyError::InvalidInput(self.max_input_bytes));
        }
        if self.max_tool_count > MAX_TOOLS {
            return Err(PolicyError::InvalidToolCount(self.max_tool_count));
        }
        Ok(())
    }

    /// Effective input limit, clamped to the framework hard cap.
    pub fn effective_input_bytes(&self) -> usize {
        self.max_input_bytes.min(MAX_TOOL_INPUT_BYTES)
    }

    /// Effective output limit, clamped to the framework hard cap.
    pub fn effective_output_bytes(&self) -> usize {
        self.max_output_bytes.min(MAX_TOOL_OUTPUT_BYTES)
    }

    /// Effective runtime for a requested timeout, clamped to the policy cap.
    pub fn effective_runtime(&self, requested: std::time::Duration) -> std::time::Duration {
        let cap = std::time::Duration::from_millis(self.max_runtime_ms);
        requested.min(cap).max(std::time::Duration::from_millis(1))
    }
}

/// Why a policy was rejected before execution.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PolicyError {
    #[error("execution policy runtime limit is invalid: {0} ms")]
    InvalidRuntime(u64),
    #[error("execution policy output limit is invalid: {0} bytes")]
    InvalidOutput(usize),
    #[error("execution policy input limit is invalid: {0} bytes")]
    InvalidInput(usize),
    #[error("execution policy tool count limit is invalid: {0}")]
    InvalidToolCount(usize),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn trust_levels_round_trip() {
        for trust in [ModuleTrust::TrustedLocal, ModuleTrust::Restricted] {
            assert_eq!(ModuleTrust::parse(trust.as_str()), Some(trust));
        }
        assert_eq!(ModuleTrust::parse("unknown"), None);
        assert!(!ModuleTrust::TrustedLocal.is_restricted());
        assert!(ModuleTrust::Restricted.is_restricted());
    }

    #[test]
    fn default_and_restricted_presets_validate() {
        let trusted = ExecutionPolicy::default();
        assert_eq!(trusted.trust, ModuleTrust::TrustedLocal);
        assert!(trusted.validate().is_ok());
        assert!(trusted.permits_restricted());

        let restricted = ExecutionPolicy::for_trust(ModuleTrust::Restricted);
        assert!(restricted.validate().is_ok());
        assert!(!restricted.permits_restricted());
        assert!(restricted.max_runtime_ms < MAX_POLICY_RUNTIME_MS);
        assert!(restricted.max_output_bytes < MAX_TOOL_OUTPUT_BYTES);
        assert!(restricted.max_input_bytes < MAX_TOOL_INPUT_BYTES);
        assert!(restricted.max_tool_count < MAX_TOOLS);

        let mut allowed = restricted.clone();
        allowed.allow_restricted = true;
        assert!(allowed.permits_restricted());
    }

    #[test]
    fn zero_limits_are_rejected() {
        let policy = ExecutionPolicy {
            max_runtime_ms: 0,
            ..ExecutionPolicy::default()
        };
        assert_eq!(policy.validate(), Err(PolicyError::InvalidRuntime(0)));

        let policy = ExecutionPolicy {
            max_output_bytes: 0,
            ..ExecutionPolicy::default()
        };
        assert_eq!(policy.validate(), Err(PolicyError::InvalidOutput(0)));

        let policy = ExecutionPolicy {
            max_input_bytes: 0,
            ..ExecutionPolicy::default()
        };
        assert_eq!(policy.validate(), Err(PolicyError::InvalidInput(0)));
    }

    #[test]
    fn limits_above_hard_caps_are_rejected() {
        let policy = ExecutionPolicy {
            max_runtime_ms: MAX_POLICY_RUNTIME_MS + 1,
            ..ExecutionPolicy::default()
        };
        assert_eq!(
            policy.validate(),
            Err(PolicyError::InvalidRuntime(MAX_POLICY_RUNTIME_MS + 1))
        );

        let policy = ExecutionPolicy {
            max_output_bytes: MAX_TOOL_OUTPUT_BYTES + 1,
            ..ExecutionPolicy::default()
        };
        assert_eq!(
            policy.validate(),
            Err(PolicyError::InvalidOutput(MAX_TOOL_OUTPUT_BYTES + 1))
        );

        let policy = ExecutionPolicy {
            max_input_bytes: MAX_TOOL_INPUT_BYTES + 1,
            ..ExecutionPolicy::default()
        };
        assert_eq!(
            policy.validate(),
            Err(PolicyError::InvalidInput(MAX_TOOL_INPUT_BYTES + 1))
        );

        let policy = ExecutionPolicy {
            max_tool_count: MAX_TOOLS + 1,
            ..ExecutionPolicy::default()
        };
        assert_eq!(
            policy.validate(),
            Err(PolicyError::InvalidToolCount(MAX_TOOLS + 1))
        );

        let policy = ExecutionPolicy {
            max_tool_count: 0,
            ..ExecutionPolicy::default()
        };
        assert!(policy.validate().is_ok());
    }

    #[test]
    fn effective_limits_are_clamped() {
        let policy = ExecutionPolicy {
            max_input_bytes: 16,
            max_output_bytes: 32,
            max_runtime_ms: 250,
            ..ExecutionPolicy::default()
        };
        assert_eq!(policy.effective_input_bytes(), 16);
        assert_eq!(policy.effective_output_bytes(), 32);
        assert_eq!(
            policy.effective_runtime(Duration::from_secs(30)),
            Duration::from_millis(250)
        );
        assert_eq!(
            policy.effective_runtime(Duration::from_millis(10)),
            Duration::from_millis(10)
        );
        assert_eq!(
            policy.effective_runtime(Duration::ZERO),
            Duration::from_millis(1)
        );
    }
}

//! Aegis module framework.
//!
//! This crate defines the versioned module manifest contract, a Platform-owned
//! registry backed by `platform.db`, a package materializer, and a supervised
//! process runtime.
//!
//! Trust model:
//!
//! * Manifests are declarative. A declared capability or permission request
//!   never grants access to Platform or Core data.
//! * Only `process`/`native` entrypoints are executed, only while the module is
//!   `enabled`, and only from inside the module's own package directory.
//! * Children run with a cleared environment, no owner token, and no Platform
//!   secrets. They are supervised with startup/stop timeouts, heartbeat
//!   freshness, crash detection, and forced termination.
//! * Disable and uninstall preserve module data.
//!
//! The manifest digest establishes content integrity only. It is not a
//! signature and does not prove publisher authenticity or installation trust.
//! Process isolation here is lifecycle and environment isolation, not an
//! operating-system sandbox.
//!
//! Batch 6 adds an explicit [`policy::ModuleTrust`] classification and
//! [`policy::ExecutionPolicy`] limits. These are enforced in the tool runner and
//! the process runtime (runtime/IO/tool budgets, restricted opt-in). They are
//! **not** an OS sandbox: Windows Job Objects / AppContainer and Linux
//! namespaces / seccomp / cgroups are not used. See
//! `docs/implementation/BATCH6_AGENT09_SOCIAL_MODULES.md`.
//!
//! Batch 7 adds [`sandbox::IsolationSupport`], the [`sandbox::ModuleSandbox`]
//! abstraction, [`sandbox::NoopSandbox`], and [`sandbox::SandboxPolicy`]. The
//! probe honestly reports OS sandboxing as unsupported on Windows and Linux;
//! [`runtime::ModuleRuntime::recover`] adds explicit owner-authorized recovery of
//! a crashed or stale module, and forced stop terminates the process tree on
//! Windows. None of this is a security sandbox. See
//! `docs/implementation/BATCH7_AGENT09_SOCIAL_MODULES.md`.

pub mod manager;
pub mod manifest;
pub mod policy;
pub mod registry;
pub mod runtime;
pub mod sandbox;
pub mod tool_runner;

pub use manager::ModuleManager;
pub use manifest::{
    is_reserved_module_id, is_sha256, validate_module_id, version_satisfies, ManifestError,
    ModuleAsset, ModuleDependency, ModuleEntrypoint, ModuleIntegrity, ModuleLifecycleHook,
    ModuleManifest, ModulePermission, ModuleResourceLimits, ModuleSdkCompatibility,
    ModuleToolDeclaration, DEFAULT_TOOL_TIMEOUT_MS, DIGEST_ALGORITHM_SHA256, MAX_TOOLS,
    MAX_TOOL_TIMEOUT_MS, MODULE_MANIFEST_SCHEMA, MODULE_SDK_NAME, MODULE_SDK_VERSION,
};
pub use policy::{
    ExecutionPolicy, ModuleTrust, PolicyError, DEFAULT_POLICY_INPUT_BYTES,
    DEFAULT_POLICY_OUTPUT_BYTES, DEFAULT_POLICY_RUNTIME_MS, DEFAULT_POLICY_TOOL_COUNT,
    MAX_POLICY_RUNTIME_MS, RESTRICTED_POLICY_INPUT_BYTES, RESTRICTED_POLICY_OUTPUT_BYTES,
    RESTRICTED_POLICY_RUNTIME_MS, RESTRICTED_POLICY_TOOL_COUNT,
};
pub use registry::{
    effective_module_tool_name, ModuleError, ModuleEvent, ModuleInspection, ModuleLifecycleState,
    ModuleRecord, ModuleRegistry, ModuleToolBinding, ModuleVerification, ModuleVerificationStatus,
    MODULE_EVENT_SCHEMA, MODULE_REGISTRY_SCHEMA, MODULE_VERIFICATION_SCHEMA,
};
pub use runtime::{
    ModuleAuditEntry, ModuleAuditError, ModuleExecutionAudit, ModuleExitOutcome, ModuleRuntime,
    ModuleRuntimeError, ModuleRuntimeState, ModuleRuntimeStatus, MODULE_EXECUTION_AUDIT_SCHEMA,
    MODULE_HEARTBEAT_INTERVAL_MS, MODULE_HEARTBEAT_STALE_MS, MODULE_RUNTIME_SCHEMA,
    MODULE_START_TIMEOUT_MS, MODULE_STOP_TIMEOUT_MS,
};
pub use sandbox::{
    FilesystemScope, IsolationSupport, ModuleSandbox, NoopSandbox, SandboxError, SandboxPolicy,
    ISOLATION_SUPPORT_SCHEMA,
};
pub use tool_runner::{
    run_module_tool, run_module_tool_with_policy, ModuleToolError, MAX_TOOL_INPUT_BYTES,
    MAX_TOOL_OUTPUT_BYTES,
};

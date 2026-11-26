use aegis_modules::{
    FilesystemScope, IsolationSupport, ModuleSandbox, NoopSandbox, SandboxError, SandboxPolicy,
};

#[test]
fn public_isolation_probe_is_honest_about_os_sandboxing() {
    let support = IsolationSupport::probe();
    assert_eq!(support.schema, "aegis.module.isolation_support.v1");
    assert_eq!(support.platform, std::env::consts::OS);
    assert!(!support.os_sandbox);
    assert!(!support.filesystem_jail);
    assert!(!support.network_isolation);
    assert!(!support.memory_limit);
    assert!(!support.cpu_limit);
    assert!(support.is_unsandboxed());
    assert!(support.cleared_environment);
    assert!(support.package_working_directory);
    assert_eq!(support.process_tree_termination, cfg!(windows));
    let unsupported = support.unsupported_mechanisms();
    assert!(unsupported.contains(&"operating-system sandbox"));
    assert!(unsupported.contains(&"filesystem jail"));
    assert!(unsupported.contains(&"network isolation"));
}

#[test]
fn noop_sandbox_reports_no_enforcement() {
    let sandbox = NoopSandbox::new();
    assert_eq!(sandbox.support(), IsolationSupport::probe());
    assert_eq!(sandbox.policy(), &SandboxPolicy::default());
    let mut command = std::process::Command::new("aegis-noop");
    assert_eq!(sandbox.apply(&mut command), Err(SandboxError::Unsupported));
}

#[test]
fn sandbox_policy_is_a_pure_future_contract() {
    let mut policy = SandboxPolicy::default();
    assert_eq!(policy.filesystem_read, FilesystemScope::ModulePackage);
    assert_eq!(policy.filesystem_write, FilesystemScope::RuntimeScratch);
    assert!(!policy.allow_network);
    assert!(!policy.allow_process_spawn);
    assert!(policy.terminate_descendants);

    policy.allow_network = true;
    policy.max_memory_bytes = Some(64 * 1024 * 1024);
    let sandbox = NoopSandbox::with_policy(policy.clone());
    assert_eq!(sandbox.policy(), &policy);
    // Changing the declarative policy must not create a false enforcement claim.
    assert!(!sandbox.support().os_sandbox);
    assert!(!sandbox.support().network_isolation);
    assert!(!sandbox.support().memory_limit);
}

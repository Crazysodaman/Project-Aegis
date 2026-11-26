//! Dry-run service installation execution planning.
//!
//! Every value in this module is inert data. [`ServiceInstallExecutor`] only
//! validates and previews an [`InstallExecutionPlan`]; it never spawns a
//! process, writes a file, calls `systemctl`/`sc.exe`, or changes the host.
//! The plan records the exact command arguments, preconditions, verification
//! checks, and rollback actions an operator (or a later, explicitly authorized
//! installer) would use.

use std::fmt;

use thiserror::Error;

use crate::registration::{
    InstallAction, InstallCommand, PlatformKind, RegistrationError, ServiceRegistration,
};

/// One precondition that must hold before a step may run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionPrecondition {
    /// Stable reason code for tests and evidence capture.
    pub code: String,
    /// Human-readable statement of the condition.
    pub description: String,
}

impl ExecutionPrecondition {
    pub fn new(code: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            description: description.into(),
        }
    }
}

/// How a step is verified after it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionVerification {
    /// Optional read-only command that performs the check.
    pub command: Option<InstallCommand>,
    /// What a successful check must observe.
    pub expectation: String,
}

impl ExecutionVerification {
    pub fn new(command: Option<InstallCommand>, expectation: impl Into<String>) -> Self {
        Self {
            command,
            expectation: expectation.into(),
        }
    }
}

/// How to undo a mutating step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionRollback {
    /// Optional command that performs the rollback.
    pub command: Option<InstallCommand>,
    /// Human-readable statement of what the rollback restores.
    pub description: String,
    /// Whether repeating the rollback is safe.
    pub idempotent: bool,
}

impl ExecutionRollback {
    pub fn new(
        command: Option<InstallCommand>,
        description: impl Into<String>,
        idempotent: bool,
    ) -> Self {
        Self {
            command,
            description: description.into(),
            idempotent,
        }
    }
}

/// One ordered, unexecuted installation step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionStep {
    pub order: u32,
    pub action: InstallAction,
    pub description: String,
    pub command: Option<InstallCommand>,
    pub preconditions: Vec<ExecutionPrecondition>,
    pub verification: ExecutionVerification,
    pub rollback: Option<ExecutionRollback>,
    pub idempotent: bool,
}

/// A validated-by-construction dry-run installation plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallExecutionPlan {
    pub platform: PlatformKind,
    pub service_name: String,
    pub unit_path: Option<String>,
    pub steps: Vec<ExecutionStep>,
}

/// One operation summarised in a [`DryRunReport`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedOperation {
    pub order: u32,
    pub action: InstallAction,
    pub command: Option<String>,
    pub verification: String,
    pub rollback: Option<String>,
}

/// The inert result of a dry run: what would run, never what ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DryRunReport {
    /// Always `false`; this executor does not execute.
    pub executed: bool,
    pub platform: PlatformKind,
    pub service_name: String,
    pub operations: Vec<PlannedOperation>,
}

impl DryRunReport {
    /// True when nothing was executed (always true for this executor).
    pub fn is_inert(&self) -> bool {
        !self.executed
    }

    pub fn render(&self) -> String {
        let mut lines = vec![
            format!("dry-run: {}", self.executed),
            format!("platform: {}", self.platform.as_str()),
            format!("service: {}", self.service_name),
        ];
        for operation in &self.operations {
            lines.push(format!(
                "{}. [{}] {}",
                operation.order,
                operation.action.as_str(),
                operation.verification
            ));
            if let Some(command) = &operation.command {
                lines.push(format!("   command: {command}"));
            }
            if let Some(rollback) = &operation.rollback {
                lines.push(format!("   rollback: {rollback}"));
            }
        }
        lines.join("\n")
    }
}

impl fmt::Display for DryRunReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.render())
    }
}

/// Validation failures for an execution plan.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ExecutionError {
    #[error("service registration is invalid: {0}")]
    Registration(#[from] RegistrationError),
    #[error("execution plan has no steps")]
    EmptyPlan,
    #[error("step order must start at 1 and be contiguous: expected {expected}, found {actual}")]
    NonContiguousOrder { expected: u32, actual: u32 },
    #[error("step {order} ({action}) requires a command")]
    MissingCommand { order: u32, action: &'static str },
    #[error("step {order} has an empty command program")]
    EmptyProgram { order: u32 },
    #[error("step {order} requires at least one precondition")]
    MissingPrecondition { order: u32 },
    #[error("step {order} requires a verification expectation")]
    MissingVerification { order: u32 },
    #[error("mutating step {order} ({action}) requires a rollback")]
    MissingRollback { order: u32, action: &'static str },
    #[error("plan targets {plan:?} but registration targets {registration:?}")]
    PlatformMismatch {
        plan: PlatformKind,
        registration: PlatformKind,
    },
    #[error("plan service name {plan:?} does not match registration {registration:?}")]
    ServiceNameMismatch { plan: String, registration: String },
    #[error("plan unit path {plan:?} does not match registration {registration:?}")]
    UnitPathMismatch {
        plan: Option<String>,
        registration: Option<String>,
    },
}

fn action_requires_command(action: InstallAction) -> bool {
    !matches!(action, InstallAction::WriteUnitFile)
}

fn action_requires_rollback(action: InstallAction) -> bool {
    matches!(
        action,
        InstallAction::WriteUnitFile
            | InstallAction::Enable
            | InstallAction::Start
            | InstallAction::CreateService
    )
}

impl InstallExecutionPlan {
    /// Build the dry-run execution plan for a registration's target platform.
    pub fn for_registration(registration: &ServiceRegistration) -> Result<Self, ExecutionError> {
        registration.validate()?;
        match registration.target {
            PlatformKind::Linux => Self::for_systemd(registration),
            PlatformKind::Windows => Self::for_windows(registration),
            PlatformKind::Unsupported => {
                Err(RegistrationError::UnsupportedPlatform(registration.target).into())
            }
        }
    }

    fn for_systemd(registration: &ServiceRegistration) -> Result<Self, ExecutionError> {
        let unit = registration.render_systemd()?;
        let unit_path = unit
            .suggested_path
            .clone()
            .unwrap_or_else(|| format!("/etc/systemd/system/{}.service", registration.name));
        let name = registration.name.clone();
        let executable = registration.executable.display().to_string();

        let write_verification = ExecutionVerification::new(
            None,
            format!("file {unit_path} exists and its bytes equal the rendered unit"),
        );
        let remove_unit = ExecutionRollback::new(
            Some(InstallCommand::new("rm", ["-f", unit_path.as_str()])),
            format!("remove {unit_path} to restore the pre-install unit set"),
            true,
        );
        let write_step = ExecutionStep {
            order: 1,
            action: InstallAction::WriteUnitFile,
            description: format!("Write the rendered unit to {unit_path}"),
            command: None,
            preconditions: vec![
                ExecutionPrecondition::new(
                    "owner-authorization",
                    "owner proof has authorized installation",
                ),
                ExecutionPrecondition::new(
                    "platform-linux",
                    "target platform is Linux with systemd",
                ),
                ExecutionPrecondition::new(
                    "unit-directory",
                    "/etc/systemd/system exists and is writable by the installer",
                ),
                ExecutionPrecondition::new(
                    "binary-present",
                    format!("{executable} exists and is executable"),
                ),
            ],
            verification: write_verification,
            rollback: Some(remove_unit),
            idempotent: true,
        };

        let reload_step = ExecutionStep {
            order: 2,
            action: InstallAction::ReloadDaemon,
            description: "Reload the systemd manager configuration".to_owned(),
            command: Some(InstallCommand::new("systemctl", ["daemon-reload"])),
            preconditions: vec![
                ExecutionPrecondition::new("unit-written", "the unit file has been written"),
                ExecutionPrecondition::new(
                    "systemctl-present",
                    "systemctl is available to the installer",
                ),
            ],
            verification: ExecutionVerification::new(
                Some(InstallCommand::new(
                    "systemctl",
                    ["show", name.as_str(), "--property=LoadState", "--no-pager"],
                )),
                "systemctl reports LoadState=loaded",
            ),
            rollback: None,
            idempotent: true,
        };

        let enable_step = ExecutionStep {
            order: 3,
            action: InstallAction::Enable,
            description: format!("Enable and start {name}"),
            command: Some(InstallCommand::new(
                "systemctl",
                ["enable", "--now", name.as_str()],
            )),
            preconditions: vec![
                ExecutionPrecondition::new("daemon-reloaded", "the systemd manager was reloaded"),
                ExecutionPrecondition::new("unit-loaded", "systemctl reports the unit as loaded"),
            ],
            verification: ExecutionVerification::new(
                Some(InstallCommand::new(
                    "systemctl",
                    ["is-active", name.as_str()],
                )),
                "systemctl is-active reports active",
            ),
            rollback: Some(ExecutionRollback::new(
                Some(InstallCommand::new(
                    "systemctl",
                    ["disable", "--now", name.as_str()],
                )),
                format!("disable and stop {name} to restore the pre-install enablement"),
                true,
            )),
            idempotent: true,
        };

        let query_step = ExecutionStep {
            order: 4,
            action: InstallAction::Query,
            description: format!("Verify that {name} is active"),
            command: Some(InstallCommand::new(
                "systemctl",
                ["status", name.as_str(), "--no-pager"],
            )),
            preconditions: vec![ExecutionPrecondition::new(
                "enabled",
                "the unit was enabled and started in the previous step",
            )],
            verification: ExecutionVerification::new(
                Some(InstallCommand::new(
                    "systemctl",
                    [
                        "show",
                        name.as_str(),
                        "--property=ActiveState",
                        "--no-pager",
                    ],
                )),
                "systemctl reports ActiveState=active",
            ),
            rollback: None,
            idempotent: true,
        };

        let plan = Self {
            platform: PlatformKind::Linux,
            service_name: name,
            unit_path: Some(unit_path),
            steps: vec![write_step, reload_step, enable_step, query_step],
        };
        plan.validate()?;
        Ok(plan)
    }

    fn for_windows(registration: &ServiceRegistration) -> Result<Self, ExecutionError> {
        let specs = registration.render_windows()?;
        let name = registration.name.clone();
        let executable = registration.executable.display().to_string();
        let query = InstallCommand::from(&specs.query);
        let query_existing = query.clone();
        let query_after_create = query.clone();
        let query_after_start = query.clone();

        let inspect_step = ExecutionStep {
            order: 1,
            action: InstallAction::Query,
            description: format!("Query {name} before creating it"),
            command: Some(query_existing),
            preconditions: vec![
                ExecutionPrecondition::new(
                    "owner-authorization",
                    "owner proof has authorized installation",
                ),
                ExecutionPrecondition::new("platform-windows", "target platform is Windows SCM"),
                ExecutionPrecondition::new("sc-available", "sc.exe is available to the installer"),
            ],
            verification: ExecutionVerification::new(
                Some(query.clone()),
                "exit status distinguishes an existing service from an absent one",
            ),
            rollback: None,
            idempotent: true,
        };

        let create_step = ExecutionStep {
            order: 2,
            action: InstallAction::CreateService,
            description: format!("Create the {name} service"),
            command: Some(InstallCommand::from(&specs.create)),
            preconditions: vec![
                ExecutionPrecondition::new(
                    "service-absent",
                    "the pre-install query reported no existing service",
                ),
                ExecutionPrecondition::new(
                    "binary-present",
                    format!("{executable} exists and is executable"),
                ),
            ],
            verification: ExecutionVerification::new(
                Some(query_after_create),
                "sc.exe query reports the newly created service",
            ),
            rollback: Some(ExecutionRollback::new(
                Some(InstallCommand::from(&specs.delete)),
                format!("delete {name} to restore the pre-install service set"),
                false,
            )),
            idempotent: false,
        };

        let start_step = ExecutionStep {
            order: 3,
            action: InstallAction::Start,
            description: format!("Start the {name} service"),
            command: Some(InstallCommand::from(&specs.start)),
            preconditions: vec![
                ExecutionPrecondition::new("service-created", "the service exists in the SCM"),
                ExecutionPrecondition::new("service-stopped", "the service is not already running"),
            ],
            verification: ExecutionVerification::new(
                Some(query_after_start),
                "sc.exe query reports STATE containing RUNNING",
            ),
            rollback: Some(ExecutionRollback::new(
                Some(InstallCommand::from(&specs.stop)),
                format!("stop {name} to restore the pre-install running state"),
                false,
            )),
            idempotent: false,
        };

        let verify_step = ExecutionStep {
            order: 4,
            action: InstallAction::Query,
            description: format!("Verify the {name} service state"),
            command: Some(query),
            preconditions: vec![ExecutionPrecondition::new(
                "started",
                "the service was started in the previous step",
            )],
            verification: ExecutionVerification::new(
                Some(InstallCommand::from(&specs.query)),
                "sc.exe query reports STATE containing RUNNING",
            ),
            rollback: None,
            idempotent: true,
        };

        let plan = Self {
            platform: PlatformKind::Windows,
            service_name: name,
            unit_path: None,
            steps: vec![inspect_step, create_step, start_step, verify_step],
        };
        plan.validate()?;
        Ok(plan)
    }

    /// Structurally validate ordering, commands, preconditions, verification,
    /// and rollback coverage. Pure; touches nothing.
    pub fn validate(&self) -> Result<(), ExecutionError> {
        if self.steps.is_empty() {
            return Err(ExecutionError::EmptyPlan);
        }
        for (index, step) in self.steps.iter().enumerate() {
            let expected = u32::try_from(index + 1).unwrap_or(u32::MAX);
            if step.order != expected {
                return Err(ExecutionError::NonContiguousOrder {
                    expected,
                    actual: step.order,
                });
            }
            if action_requires_command(step.action) && step.command.is_none() {
                return Err(ExecutionError::MissingCommand {
                    order: step.order,
                    action: step.action.as_str(),
                });
            }
            if let Some(command) = &step.command {
                if command.program.is_empty() {
                    return Err(ExecutionError::EmptyProgram { order: step.order });
                }
            }
            if step.preconditions.is_empty() {
                return Err(ExecutionError::MissingPrecondition { order: step.order });
            }
            if step.preconditions.iter().any(|precondition| {
                precondition.code.is_empty() || precondition.description.is_empty()
            }) {
                return Err(ExecutionError::MissingPrecondition { order: step.order });
            }
            if step.verification.expectation.is_empty() {
                return Err(ExecutionError::MissingVerification { order: step.order });
            }
            if action_requires_rollback(step.action) && step.rollback.is_none() {
                return Err(ExecutionError::MissingRollback {
                    order: step.order,
                    action: step.action.as_str(),
                });
            }
        }
        Ok(())
    }

    /// Verify that this plan was built for the same service as `registration`.
    pub fn validate_against(
        &self,
        registration: &ServiceRegistration,
    ) -> Result<(), ExecutionError> {
        self.validate()?;
        if self.platform != registration.target {
            return Err(ExecutionError::PlatformMismatch {
                plan: self.platform,
                registration: registration.target,
            });
        }
        if self.service_name != registration.name {
            return Err(ExecutionError::ServiceNameMismatch {
                plan: self.service_name.clone(),
                registration: registration.name.clone(),
            });
        }
        let expected_unit_path = match registration.target {
            PlatformKind::Linux => registration
                .render_systemd()
                .ok()
                .and_then(|unit| unit.suggested_path),
            _ => None,
        };
        if self.unit_path != expected_unit_path {
            return Err(ExecutionError::UnitPathMismatch {
                plan: self.unit_path.clone(),
                registration: expected_unit_path,
            });
        }
        Ok(())
    }

    /// Steps that carry a rollback, in reverse execution order.
    pub fn rollback_steps(&self) -> Vec<&ExecutionStep> {
        self.steps
            .iter()
            .rev()
            .filter(|step| step.rollback.is_some())
            .collect()
    }

    /// Render the plan as display text. Nothing is executed.
    pub fn render(&self) -> String {
        let mut lines = vec![
            format!("platform: {}", self.platform.as_str()),
            format!("service: {}", self.service_name),
            match &self.unit_path {
                Some(path) => format!("unit-path: {path}"),
                None => "unit-path: (none)".to_owned(),
            },
        ];
        for step in &self.steps {
            lines.push(format!(
                "{}. [{}] {}",
                step.order,
                step.action.as_str(),
                step.description
            ));
            if let Some(command) = &step.command {
                lines.push(format!("   command: {}", command.render()));
            }
            for precondition in &step.preconditions {
                lines.push(format!(
                    "   when[{}]: {}",
                    precondition.code, precondition.description
                ));
            }
            lines.push(format!("   verify: {}", step.verification.expectation));
            if let Some(verification) = &step.verification.command {
                lines.push(format!("   verify-command: {}", verification.render()));
            }
            if let Some(rollback) = &step.rollback {
                lines.push(format!(
                    "   rollback[{}]: {}",
                    if rollback.idempotent {
                        "idempotent"
                    } else {
                        "not-idempotent"
                    },
                    rollback.description
                ));
                if let Some(command) = &rollback.command {
                    lines.push(format!("   rollback-command: {}", command.render()));
                }
            }
        }
        lines.join("\n")
    }
}

impl fmt::Display for InstallExecutionPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.render())
    }
}

/// Dry-run install executor.
///
/// Construction validates the plan. The only output is a [`DryRunReport`];
/// no command is ever spawned and no file is ever written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceInstallExecutor {
    plan: InstallExecutionPlan,
}

impl ServiceInstallExecutor {
    /// Wrap a plan after validating it.
    pub fn new(plan: InstallExecutionPlan) -> Result<Self, ExecutionError> {
        plan.validate()?;
        Ok(Self { plan })
    }

    /// Build a validated dry-run executor for a registration.
    pub fn for_registration(registration: &ServiceRegistration) -> Result<Self, ExecutionError> {
        Self::new(InstallExecutionPlan::for_registration(registration)?)
    }

    pub fn plan(&self) -> &InstallExecutionPlan {
        &self.plan
    }

    /// This executor is always a dry run.
    pub fn is_dry_run(&self) -> bool {
        true
    }

    /// Produce the inert preview. This is the only "run" this type offers.
    pub fn dry_run(&self) -> DryRunReport {
        DryRunReport {
            executed: false,
            platform: self.plan.platform,
            service_name: self.plan.service_name.clone(),
            operations: self
                .plan
                .steps
                .iter()
                .map(|step| PlannedOperation {
                    order: step.order,
                    action: step.action,
                    command: step.command.as_ref().map(InstallCommand::render),
                    verification: step.verification.expectation.clone(),
                    rollback: step
                        .rollback
                        .as_ref()
                        .map(|rollback| rollback.description.clone()),
                })
                .collect(),
        }
    }

    pub fn into_plan(self) -> InstallExecutionPlan {
        self.plan
    }
}

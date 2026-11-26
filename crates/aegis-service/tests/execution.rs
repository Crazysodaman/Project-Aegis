use aegis_service::{
    ExecutionError, InstallAction, InstallExecutionPlan, PlatformKind, ServiceInstallExecutor,
    ServiceRegistration,
};

fn linux_registration() -> ServiceRegistration {
    ServiceRegistration::for_platform(
        PlatformKind::Linux,
        "aegis-platform",
        "/opt/aegis/bin/aegis-service-host",
    )
    .expect("linux registration")
    .with_argument("--root")
    .with_argument("/var/lib/aegis")
}

fn windows_registration() -> ServiceRegistration {
    ServiceRegistration::for_platform(
        PlatformKind::Windows,
        "aegis-platform",
        r"C:\Program Files\Aegis\aegis-service-host.exe",
    )
    .expect("windows registration")
    .with_argument("--root")
    .with_argument(r"C:\ProgramData\Aegis")
}

#[test]
fn systemd_execution_plan_is_ordered_with_commands_and_rollback() {
    let plan = InstallExecutionPlan::for_registration(&linux_registration()).expect("plan");

    assert_eq!(plan.platform, PlatformKind::Linux);
    assert_eq!(plan.service_name, "aegis-platform");
    assert_eq!(
        plan.unit_path.as_deref(),
        Some("/etc/systemd/system/aegis-platform.service")
    );
    assert_eq!(
        plan.steps.iter().map(|step| step.order).collect::<Vec<_>>(),
        [1, 2, 3, 4]
    );
    assert_eq!(
        plan.steps
            .iter()
            .map(|step| step.action)
            .collect::<Vec<_>>(),
        [
            InstallAction::WriteUnitFile,
            InstallAction::ReloadDaemon,
            InstallAction::Enable,
            InstallAction::Query,
        ]
    );
    assert!(plan.steps.iter().all(|step| !step.preconditions.is_empty()));
    assert!(plan
        .steps
        .iter()
        .all(|step| !step.verification.expectation.is_empty()));

    let write = &plan.steps[0];
    assert!(write.command.is_none());
    let write_rollback = write.rollback.as_ref().expect("write rollback");
    assert_eq!(write_rollback.command.as_ref().unwrap().program, "rm");
    assert_eq!(
        write_rollback.command.as_ref().unwrap().arguments(),
        ["-f", "/etc/systemd/system/aegis-platform.service"].map(str::to_owned)
    );

    assert_eq!(
        plan.steps[1].command.as_ref().unwrap().arguments(),
        ["daemon-reload"].map(str::to_owned)
    );
    assert_eq!(
        plan.steps[2].command.as_ref().unwrap().arguments(),
        ["enable", "--now", "aegis-platform"].map(str::to_owned)
    );
    assert_eq!(
        plan.steps[3].command.as_ref().unwrap().arguments(),
        ["status", "aegis-platform", "--no-pager"].map(str::to_owned)
    );
    assert_eq!(
        plan.steps[2]
            .rollback
            .as_ref()
            .unwrap()
            .command
            .as_ref()
            .unwrap()
            .arguments(),
        ["disable", "--now", "aegis-platform"].map(str::to_owned)
    );

    let rollback_order = plan
        .rollback_steps()
        .iter()
        .map(|step| step.order)
        .collect::<Vec<_>>();
    assert_eq!(rollback_order, [3, 1]);
    assert_eq!(plan.validate(), Ok(()));
    assert_eq!(plan.validate_against(&linux_registration()), Ok(()));
}

#[test]
fn windows_execution_plan_is_ordered_with_commands_and_rollback() {
    let plan = InstallExecutionPlan::for_registration(&windows_registration()).expect("plan");

    assert_eq!(plan.platform, PlatformKind::Windows);
    assert!(plan.unit_path.is_none());
    assert_eq!(
        plan.steps.iter().map(|step| step.order).collect::<Vec<_>>(),
        [1, 2, 3, 4]
    );
    assert_eq!(
        plan.steps
            .iter()
            .map(|step| step.action)
            .collect::<Vec<_>>(),
        [
            InstallAction::Query,
            InstallAction::CreateService,
            InstallAction::Start,
            InstallAction::Query,
        ]
    );

    let create = &plan.steps[1];
    assert_eq!(create.command.as_ref().unwrap().program, "sc.exe");
    assert_eq!(create.command.as_ref().unwrap().arguments()[0], "create");
    let create_rollback = create.rollback.as_ref().unwrap().command.as_ref().unwrap();
    assert_eq!(create_rollback.program, "sc.exe");
    assert_eq!(
        create_rollback.arguments(),
        ["delete", "aegis-platform"].map(str::to_owned)
    );

    let start = &plan.steps[2];
    assert_eq!(
        start.command.as_ref().unwrap().arguments(),
        ["start", "aegis-platform"].map(str::to_owned)
    );
    assert_eq!(
        start
            .rollback
            .as_ref()
            .unwrap()
            .command
            .as_ref()
            .unwrap()
            .arguments(),
        ["stop", "aegis-platform"].map(str::to_owned)
    );

    let rollback_order = plan
        .rollback_steps()
        .iter()
        .map(|step| step.order)
        .collect::<Vec<_>>();
    assert_eq!(rollback_order, [3, 2]);
    assert_eq!(plan.validate(), Ok(()));
    assert_eq!(plan.validate_against(&windows_registration()), Ok(()));
}

#[test]
fn invalid_plans_are_rejected() {
    let empty = InstallExecutionPlan {
        platform: PlatformKind::Linux,
        service_name: "aegis-platform".to_owned(),
        unit_path: None,
        steps: Vec::new(),
    };
    assert_eq!(empty.validate(), Err(ExecutionError::EmptyPlan));

    let mut noncontiguous = InstallExecutionPlan::for_registration(&linux_registration()).unwrap();
    noncontiguous.steps[1].order = 9;
    assert_eq!(
        noncontiguous.validate(),
        Err(ExecutionError::NonContiguousOrder {
            expected: 2,
            actual: 9,
        })
    );

    let mut missing_command =
        InstallExecutionPlan::for_registration(&linux_registration()).unwrap();
    missing_command.steps[1].command = None;
    assert_eq!(
        missing_command.validate(),
        Err(ExecutionError::MissingCommand {
            order: 2,
            action: "reload-daemon",
        })
    );

    let mut empty_program = InstallExecutionPlan::for_registration(&linux_registration()).unwrap();
    empty_program.steps[2]
        .command
        .as_mut()
        .unwrap()
        .program
        .clear();
    assert_eq!(
        empty_program.validate(),
        Err(ExecutionError::EmptyProgram { order: 3 })
    );

    let mut missing_precondition =
        InstallExecutionPlan::for_registration(&linux_registration()).unwrap();
    missing_precondition.steps[0].preconditions.clear();
    assert_eq!(
        missing_precondition.validate(),
        Err(ExecutionError::MissingPrecondition { order: 1 })
    );

    let mut missing_verification =
        InstallExecutionPlan::for_registration(&linux_registration()).unwrap();
    missing_verification.steps[0]
        .verification
        .expectation
        .clear();
    assert_eq!(
        missing_verification.validate(),
        Err(ExecutionError::MissingVerification { order: 1 })
    );

    let mut missing_rollback =
        InstallExecutionPlan::for_registration(&linux_registration()).unwrap();
    missing_rollback.steps[0].rollback = None;
    assert_eq!(
        missing_rollback.validate(),
        Err(ExecutionError::MissingRollback {
            order: 1,
            action: "write-unit-file",
        })
    );
}

#[test]
fn validation_detects_registration_mismatch() {
    let registration = linux_registration();
    let plan = InstallExecutionPlan::for_registration(&registration).unwrap();

    let mut wrong_platform = plan.clone();
    wrong_platform.platform = PlatformKind::Windows;
    assert_eq!(
        wrong_platform.validate_against(&registration),
        Err(ExecutionError::PlatformMismatch {
            plan: PlatformKind::Windows,
            registration: PlatformKind::Linux,
        })
    );

    let mut wrong_name = plan.clone();
    wrong_name.service_name = "other-service".to_owned();
    assert_eq!(
        wrong_name.validate_against(&registration),
        Err(ExecutionError::ServiceNameMismatch {
            plan: "other-service".to_owned(),
            registration: "aegis-platform".to_owned(),
        })
    );

    let mut wrong_unit = plan;
    wrong_unit.unit_path = Some("/tmp/other.service".to_owned());
    assert_eq!(
        wrong_unit.validate_against(&registration),
        Err(ExecutionError::UnitPathMismatch {
            plan: Some("/tmp/other.service".to_owned()),
            registration: Some("/etc/systemd/system/aegis-platform.service".to_owned()),
        })
    );
}

#[test]
fn executor_is_inert_and_never_executes() {
    let registration = linux_registration();
    let executor = ServiceInstallExecutor::for_registration(&registration).expect("executor");
    assert!(executor.is_dry_run());
    assert_eq!(executor.plan().validate(), Ok(()));

    let report = executor.dry_run();
    assert!(!report.executed);
    assert!(report.is_inert());
    assert_eq!(report.platform, PlatformKind::Linux);
    assert_eq!(report.service_name, "aegis-platform");
    assert_eq!(report.operations.len(), 4);
    assert_eq!(report.operations[0].order, 1);
    assert!(report.operations[2].rollback.is_some());

    let rendered = report.render();
    assert_eq!(rendered, report.to_string());
    assert!(rendered.contains("dry-run: false"));
    assert!(rendered.contains("platform: linux"));

    let plan = executor.into_plan();
    assert_eq!(
        plan,
        InstallExecutionPlan::for_registration(&registration).unwrap()
    );
}

#[test]
fn executor_rejects_invalid_plan() {
    let mut plan = InstallExecutionPlan::for_registration(&linux_registration()).unwrap();
    plan.steps.clear();
    assert_eq!(
        ServiceInstallExecutor::new(plan),
        Err(ExecutionError::EmptyPlan)
    );
}

#[test]
fn execution_plans_are_deterministic_and_render_evidence() {
    let linux = linux_registration();
    assert_eq!(
        InstallExecutionPlan::for_registration(&linux).unwrap(),
        InstallExecutionPlan::for_registration(&linux).unwrap()
    );
    let windows = windows_registration();
    assert_eq!(
        InstallExecutionPlan::for_registration(&windows).unwrap(),
        InstallExecutionPlan::for_registration(&windows).unwrap()
    );

    let plan = InstallExecutionPlan::for_registration(&linux).unwrap();
    let rendered = plan.render();
    assert_eq!(rendered, plan.to_string());
    assert!(rendered.contains("when[owner-authorization]"));
    assert!(rendered.contains("verify-command: systemctl is-active aegis-platform"));
    assert!(rendered.contains("rollback-command: systemctl disable --now aegis-platform"));
}

use aegis_service::{
    platform, InstallAction, InstallExecutionPlan, PlatformKind, ServiceRegistration, ServiceType,
    DEFAULT_SERVICE_NAME, SYSTEMD_UNIT_DIR,
};

fn linux_registration() -> ServiceRegistration {
    ServiceRegistration::for_platform(
        PlatformKind::Linux,
        DEFAULT_SERVICE_NAME,
        "/opt/aegis/bin/aegis-service-host",
    )
    .expect("linux registration")
    .with_argument("--root")
    .with_argument("/var/lib/aegis")
    .with_argument("--socket")
    .with_argument(DEFAULT_SERVICE_NAME)
}

fn windows_registration() -> ServiceRegistration {
    ServiceRegistration::for_platform(
        PlatformKind::Windows,
        DEFAULT_SERVICE_NAME,
        r"C:\Program Files\Aegis\aegis-service-host.exe",
    )
    .expect("windows registration")
    .with_argument("--root")
    .with_argument(r"C:\ProgramData\Aegis")
}

#[test]
fn systemd_unit_generation_is_deterministic_and_hardened() {
    let registration = linux_registration();
    let first = registration.render_systemd().expect("first render");
    let second = registration.render_systemd().expect("second render");

    assert_eq!(first, second);
    assert_eq!(
        first.suggested_path.as_deref(),
        Some(format!("{SYSTEMD_UNIT_DIR}/{DEFAULT_SERVICE_NAME}.service").as_str())
    );

    let text = first.render();
    assert_eq!(text, first.to_string());
    assert!(text.contains("ExecStart=/opt/aegis/bin/aegis-service-host"));
    assert!(text.contains(&format!("--socket {DEFAULT_SERVICE_NAME}")));
    for directive in [
        "NoNewPrivileges=true",
        "PrivateTmp=true",
        "ProtectSystem=full",
        "ProtectKernelTunables=true",
        "ProtectKernelModules=true",
        "ProtectControlGroups=true",
        "RestrictSUIDSGID=true",
        "LockPersonality=true",
        "RestrictRealtime=true",
    ] {
        assert!(text.contains(directive), "missing {directive}: {text}");
    }
    assert!(text.ends_with("[Install]\nWantedBy=multi-user.target\n"));
}

#[test]
fn systemd_notify_type_is_rendered_without_side_effects() {
    let unit = linux_registration()
        .with_service_type(ServiceType::Notify)
        .render_systemd()
        .expect("render");

    assert!(unit.contents.contains("Type=notify"));
    assert!(unit.contents.contains("NotifyAccess=main"));

    let report = platform();
    assert!(!report.runtime_registration_performed);
    assert!(report.generation_supported);
}

#[test]
fn windows_scm_specs_are_data_only_and_ordered() {
    let specs = windows_registration().render_windows().expect("render");

    assert_eq!(specs.name, DEFAULT_SERVICE_NAME);
    assert_eq!(specs.create.program, "sc.exe");
    assert_eq!(specs.create.arguments()[0], "create");
    assert_eq!(
        specs.create.arguments(),
        [
            "create",
            DEFAULT_SERVICE_NAME,
            "binPath=",
            r#""C:\Program Files\Aegis\aegis-service-host.exe" --root C:\ProgramData\Aegis"#,
            "start=",
            "auto",
            "DisplayName=",
            "Aegis Platform background service",
        ]
        .map(str::to_owned)
    );
    assert_eq!(
        specs.delete.arguments(),
        ["delete", DEFAULT_SERVICE_NAME].map(str::to_owned)
    );

    let rendered = specs.render();
    assert_eq!(rendered, specs.to_string());
    assert!(rendered.starts_with(&format!("sc.exe create {DEFAULT_SERVICE_NAME} binPath=")));
    assert!(rendered.contains(&format!("sc.exe stop {DEFAULT_SERVICE_NAME}")));
    assert!(!platform().runtime_registration_performed);
}

#[test]
fn install_plan_rollback_covers_every_mutating_step() {
    let plan = InstallExecutionPlan::for_registration(&linux_registration()).expect("plan");
    assert_eq!(plan.validate(), Ok(()));

    for step in &plan.steps {
        let mutating = matches!(
            step.action,
            InstallAction::WriteUnitFile
                | InstallAction::ReloadDaemon
                | InstallAction::Enable
                | InstallAction::Start
                | InstallAction::CreateService
        );
        if mutating {
            assert!(
                step.rollback.is_some() || step.action == InstallAction::ReloadDaemon,
                "step {} ({}) must document rollback",
                step.order,
                step.action.as_str()
            );
        }
    }

    let rollback_order = plan
        .rollback_steps()
        .iter()
        .map(|step| step.order)
        .collect::<Vec<_>>();
    assert_eq!(rollback_order, [3, 1]);

    let windows = InstallExecutionPlan::for_registration(&windows_registration()).expect("plan");
    let windows_rollback_order = windows
        .rollback_steps()
        .iter()
        .map(|step| step.order)
        .collect::<Vec<_>>();
    assert_eq!(windows_rollback_order, [3, 2]);
}

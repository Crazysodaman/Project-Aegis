use aegis_service::{
    platform, InstallAction, PlatformKind, RegistrationError, RenderedRegistration, RestartPolicy,
    ServiceRegistration, ServiceType, MAX_SERVICE_NAME_LEN,
};

fn linux_registration() -> ServiceRegistration {
    ServiceRegistration::for_platform(
        PlatformKind::Linux,
        "aegis-platform",
        "/opt/aegis/bin/aegis-service-host",
    )
    .expect("linux registration")
    .with_description("Aegis Platform background service")
    .with_argument("--root")
    .with_argument("/var/lib/aegis")
    .with_argument("--socket")
    .with_argument("aegis-platform")
    .with_working_directory("/var/lib/aegis")
    .with_environment("AEGIS_LOG", "info")
}

fn windows_registration() -> ServiceRegistration {
    ServiceRegistration::for_platform(
        PlatformKind::Windows,
        "aegis-platform",
        r"C:\Program Files\Aegis\aegis-service-host.exe",
    )
    .expect("windows registration")
    .with_description("Aegis Platform background service")
    .with_argument("--root")
    .with_argument(r"C:\ProgramData\Aegis")
    .with_argument("--socket")
    .with_argument("aegis-platform")
}

#[test]
fn systemd_unit_contains_expected_directives() {
    let unit = linux_registration().render_systemd().expect("render");

    assert_eq!(unit.platform, PlatformKind::Linux);
    assert_eq!(unit.name, "aegis-platform");
    assert_eq!(
        unit.suggested_path.as_deref(),
        Some("/etc/systemd/system/aegis-platform.service")
    );
    assert_eq!(unit.render(), unit.contents.as_str());

    let text = unit.render();
    assert!(text.starts_with(
        "[Unit]\nDescription=Aegis Platform background service\n\n[Service]\nType=simple\n"
    ));
    assert!(text.contains(
        "\nExecStart=/opt/aegis/bin/aegis-service-host --root /var/lib/aegis --socket aegis-platform\n"
    ));
    assert!(text.contains("\nWorkingDirectory=/var/lib/aegis\n"));
    assert!(text.contains("\nEnvironment=\"AEGIS_LOG=info\"\n"));
    assert!(text.contains("\nRestart=on-failure\n"));
    assert!(text.contains("\nRestartSec=5\n"));
    assert!(text.contains("\nNoNewPrivileges=true\n"));
    assert!(text.contains("\nPrivateTmp=true\n"));
    assert!(text.contains("\nProtectSystem=full\n"));
    assert!(text.contains("\nRestrictSUIDSGID=true\n"));
    assert!(text.ends_with("[Install]\nWantedBy=multi-user.target\n"));
}

#[test]
fn systemd_quotes_spaces_and_specifiers() {
    let unit = ServiceRegistration::for_platform(
        PlatformKind::Linux,
        "aegis-platform",
        "/opt/Aegis Platform/aegis-service-host",
    )
    .expect("registration")
    .with_argument("100%")
    .with_environment("AEGIS_HOME", "/srv/aegis home")
    .render_systemd()
    .expect("render");

    let text = unit.render();
    assert!(
        text.contains("\nExecStart=\"/opt/Aegis Platform/aegis-service-host\" \"100%%\"\n"),
        "{text}"
    );
    assert!(
        text.contains("\nEnvironment=\"AEGIS_HOME=/srv/aegis home\"\n"),
        "{text}"
    );
}

#[test]
fn notify_type_adds_notify_access() {
    let text = linux_registration()
        .with_service_type(ServiceType::Notify)
        .render_systemd()
        .expect("render")
        .contents;

    assert!(text.contains("\nType=notify\n"), "{text}");
    assert!(text.contains("\nNotifyAccess=main\n"), "{text}");
    assert!(!text.contains("\nType=simple\n"), "{text}");
}

#[test]
fn restart_policy_is_rendered_consistently() {
    let no_restart = linux_registration()
        .with_restart(RestartPolicy::No, 0)
        .render_systemd()
        .expect("render")
        .contents;
    assert!(no_restart.contains("\nRestart=no\n"));
    assert!(!no_restart.contains("RestartSec="));

    let always = linux_registration()
        .with_restart(RestartPolicy::Always, 10)
        .render_systemd()
        .expect("render")
        .contents;
    assert!(always.contains("\nRestart=always\n"));
    assert!(always.contains("\nRestartSec=10\n"));
}

#[test]
fn windows_create_arguments_are_exact() {
    let specs = windows_registration().render_windows().expect("render");

    assert_eq!(specs.name, "aegis-platform");
    assert_eq!(specs.create.program, "sc.exe");
    assert_eq!(
        specs.create.arguments(),
        [
            "create",
            "aegis-platform",
            "binPath=",
            r#""C:\Program Files\Aegis\aegis-service-host.exe" --root C:\ProgramData\Aegis --socket aegis-platform"#,
            "start=",
            "auto",
            "DisplayName=",
            "Aegis Platform background service",
        ]
        .map(str::to_owned)
    );
    assert_eq!(
        specs.start.arguments(),
        ["start", "aegis-platform"].map(str::to_owned)
    );
    assert_eq!(
        specs.stop.arguments(),
        ["stop", "aegis-platform"].map(str::to_owned)
    );
    assert_eq!(
        specs.query.arguments(),
        ["query", "aegis-platform"].map(str::to_owned)
    );
    assert_eq!(
        specs.delete.arguments(),
        ["delete", "aegis-platform"].map(str::to_owned)
    );

    let expected = r#"sc.exe create aegis-platform binPath= "\"C:\Program Files\Aegis\aegis-service-host.exe\" --root C:\ProgramData\Aegis --socket aegis-platform" start= auto DisplayName= "Aegis Platform background service""#;
    assert_eq!(specs.create.render(), expected);
}

#[test]
fn windows_specs_and_systemd_units_are_deterministic() {
    let windows = windows_registration();
    let first = windows.render_windows().expect("first");
    let second = windows.render_windows().expect("second");
    assert_eq!(first, second);
    assert_eq!(first.render(), second.render());

    let linux = linux_registration();
    assert_eq!(
        linux.render_systemd().expect("first"),
        linux.render_systemd().expect("second")
    );
    assert_eq!(
        linux.render().expect("first"),
        linux.render().expect("second")
    );
}

#[test]
fn render_dispatches_by_target_platform() {
    let linux = linux_registration().render().expect("linux render");
    assert_eq!(linux.platform(), PlatformKind::Linux);
    assert!(matches!(linux, RenderedRegistration::Systemd(_)));
    assert_eq!(linux.render(), linux.to_string());

    let windows = windows_registration().render().expect("windows render");
    assert_eq!(windows.platform(), PlatformKind::Windows);
    assert!(matches!(windows, RenderedRegistration::Windows(_)));
    assert_eq!(windows.render(), windows.to_string());
}

#[test]
fn validation_rejects_invalid_names() {
    let empty = ServiceRegistration::for_platform(PlatformKind::Linux, "", "/opt/aegis/host")
        .expect_err("empty name");
    assert!(matches!(empty, RegistrationError::EmptyName));

    for name in [
        "aegis platform",
        "-aegis",
        ".aegis",
        "aegis/service",
        "aegis:platform",
        "ågis",
    ] {
        let error = ServiceRegistration::for_platform(PlatformKind::Linux, name, "/opt/aegis/host")
            .expect_err("invalid name");
        assert!(
            matches!(error, RegistrationError::InvalidName(_)),
            "{name}: {error:?}"
        );
    }

    let long_name = "a".repeat(MAX_SERVICE_NAME_LEN + 1);
    let error =
        ServiceRegistration::for_platform(PlatformKind::Linux, long_name, "/opt/aegis/host")
            .expect_err("long name");
    assert!(matches!(error, RegistrationError::NameTooLong(_)));
}

#[test]
fn validation_rejects_relative_executables_per_target() {
    let linux = ServiceRegistration::for_platform(PlatformKind::Linux, "aegis", "opt/aegis/host")
        .expect_err("relative linux path");
    assert!(matches!(
        linux,
        RegistrationError::RelativeExecutable {
            platform: PlatformKind::Linux,
            ..
        }
    ));

    let windows = ServiceRegistration::for_platform(
        PlatformKind::Windows,
        "aegis",
        r"Program Files\Aegis\host.exe",
    )
    .expect_err("relative windows path");
    assert!(matches!(
        windows,
        RegistrationError::RelativeExecutable {
            platform: PlatformKind::Windows,
            ..
        }
    ));

    let linux_absolute =
        ServiceRegistration::for_platform(PlatformKind::Linux, "aegis", "C:\\Aegis\\host.exe")
            .expect_err("windows path is not absolute on linux");
    assert!(matches!(
        linux_absolute,
        RegistrationError::RelativeExecutable {
            platform: PlatformKind::Linux,
            ..
        }
    ));

    ServiceRegistration::for_platform(PlatformKind::Windows, "aegis", r"C:\Aegis\host.exe")
        .expect("drive absolute windows path");
    ServiceRegistration::for_platform(PlatformKind::Windows, "aegis", r"\\server\share\host.exe")
        .expect("UNC absolute windows path");
}

#[test]
fn validation_rejects_unsafe_paths_and_fields() {
    let parent =
        ServiceRegistration::for_platform(PlatformKind::Linux, "aegis", "/opt/../etc/aegis-host")
            .expect_err("parent component");
    assert!(matches!(parent, RegistrationError::ParentDirectory(_)));

    let relative_directory = linux_registration().with_working_directory("relative/dir");
    assert!(matches!(
        relative_directory.validate(),
        Err(RegistrationError::RelativeWorkingDirectory { .. })
    ));

    let bad_argument = linux_registration().with_argument("line\nbreak");
    assert_eq!(
        bad_argument.validate(),
        Err(RegistrationError::InvalidArgument)
    );

    let bad_environment_name = linux_registration().with_environment("1BAD", "value");
    assert!(matches!(
        bad_environment_name.validate(),
        Err(RegistrationError::InvalidEnvironmentName(_))
    ));

    let bad_environment_value = linux_registration().with_environment("GOOD", "line\nbreak");
    assert!(matches!(
        bad_environment_value.validate(),
        Err(RegistrationError::InvalidEnvironmentValue(_))
    ));

    let bad_description = linux_registration().with_description("line\nbreak");
    assert_eq!(
        bad_description.validate(),
        Err(RegistrationError::InvalidDescription)
    );
}

#[test]
fn validation_rejects_unsupported_and_mismatched_platforms() {
    let unsupported =
        ServiceRegistration::for_platform(PlatformKind::Unsupported, "aegis", "/opt/aegis/host")
            .expect_err("unsupported platform");
    assert!(matches!(
        unsupported,
        RegistrationError::UnsupportedPlatform(PlatformKind::Unsupported)
    ));

    assert!(matches!(
        linux_registration().render_windows(),
        Err(RegistrationError::WrongPlatform {
            expected: PlatformKind::Windows,
            actual: PlatformKind::Linux,
        })
    ));
    assert!(matches!(
        windows_registration().render_systemd(),
        Err(RegistrationError::WrongPlatform {
            expected: PlatformKind::Linux,
            actual: PlatformKind::Windows,
        })
    ));
}

#[test]
fn platform_probe_reports_generation_only() {
    let report = platform();

    assert_eq!(report.kind, PlatformKind::current());
    assert_eq!(report.os, std::env::consts::OS);
    assert_eq!(report.arch, std::env::consts::ARCH);
    assert_eq!(
        report.registration_supported,
        report.kind.supports_registration()
    );
    assert!(report.generation_supported);
    assert!(!report.runtime_registration_performed);
    assert!(report.notes.contains("does not install"));

    match report.kind {
        PlatformKind::Linux => assert_eq!(report.mechanism.as_deref(), Some("systemd")),
        PlatformKind::Windows => assert_eq!(report.mechanism.as_deref(), Some("windows-scm")),
        PlatformKind::Unsupported => assert_eq!(report.mechanism, None),
    }
}

#[test]
fn systemd_install_plan_is_ordered_and_annotated() {
    let plan = linux_registration().install_plan().expect("install plan");

    assert_eq!(plan.platform, PlatformKind::Linux);
    assert_eq!(plan.service_name, "aegis-platform");
    assert_eq!(
        plan.unit_path.as_deref(),
        Some("/etc/systemd/system/aegis-platform.service")
    );
    assert_eq!(plan.steps.len(), 4);

    let actions = plan
        .steps
        .iter()
        .map(|step| step.action)
        .collect::<Vec<_>>();
    assert_eq!(
        actions,
        [
            InstallAction::WriteUnitFile,
            InstallAction::ReloadDaemon,
            InstallAction::Enable,
            InstallAction::Query,
        ]
    );
    assert!(plan.steps.iter().all(|step| step.idempotent));
    assert!(plan.steps.iter().all(|step| !step.note.is_empty()));
    assert_eq!(
        plan.steps.iter().map(|step| step.order).collect::<Vec<_>>(),
        [1, 2, 3, 4]
    );

    let write = &plan.steps[0];
    assert!(write.command.is_none());
    assert!(write
        .description
        .contains("/etc/systemd/system/aegis-platform.service"));

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
    assert!(plan.steps[2]
        .command
        .as_ref()
        .unwrap()
        .render()
        .contains("systemctl enable --now aegis-platform"));

    let rendered = plan.render();
    assert_eq!(rendered, plan.to_string());
    assert!(rendered.contains("1. [write-unit-file]"));
    assert!(rendered.contains("2. [reload-daemon]"));
    assert!(rendered.contains("idempotent"));
}

#[test]
fn windows_install_plan_is_ordered_and_annotated() {
    let plan = windows_registration().install_plan().expect("install plan");

    assert_eq!(plan.platform, PlatformKind::Windows);
    assert_eq!(plan.service_name, "aegis-platform");
    assert!(plan.unit_path.is_none());
    assert_eq!(plan.steps.len(), 4);

    let actions = plan
        .steps
        .iter()
        .map(|step| step.action)
        .collect::<Vec<_>>();
    assert_eq!(
        actions,
        [
            InstallAction::Query,
            InstallAction::CreateService,
            InstallAction::Start,
            InstallAction::Query,
        ]
    );
    let idempotent = plan
        .steps
        .iter()
        .map(|step| step.idempotent)
        .collect::<Vec<_>>();
    assert_eq!(idempotent, [true, false, false, true]);

    let create = plan.steps[1].command.as_ref().unwrap();
    assert_eq!(create.program, "sc.exe");
    assert_eq!(create.arguments()[0], "create");
    assert!(create
        .render()
        .starts_with("sc.exe create aegis-platform binPath="));

    let start = plan.steps[2].command.as_ref().unwrap();
    assert_eq!(
        start.arguments(),
        ["start", "aegis-platform"].map(str::to_owned)
    );

    let rendered = plan.render();
    assert!(rendered.contains("1. [query]"));
    assert!(rendered.contains("2. [create-service]"));
    assert!(rendered.contains("not-idempotent"));
}

#[test]
fn install_plans_are_deterministic() {
    let linux = linux_registration();
    assert_eq!(
        linux.install_plan().expect("first"),
        linux.install_plan().expect("second")
    );

    let windows = windows_registration();
    assert_eq!(
        windows.install_plan().expect("first"),
        windows.install_plan().expect("second")
    );
}

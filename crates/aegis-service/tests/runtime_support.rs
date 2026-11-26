use aegis_service::{
    runtime_support, runtime_support_with, OsFamily, RuntimeInputs, RuntimeVerdict,
};

fn full_systemd_inputs() -> RuntimeInputs {
    RuntimeInputs::for_family(OsFamily::Linux)
        .with_systemd_unit_dir(true)
        .with_systemd_runtime_dir(true)
        .with_systemctl(true)
}

#[test]
fn windows_reports_linux_runtime_unsupported() {
    let report = runtime_support_with(&RuntimeInputs::for_family(OsFamily::Windows));

    assert_eq!(report.os_family, OsFamily::Windows);
    assert!(!report.systemd_capable);
    assert!(!report.systemd_present);
    assert_eq!(report.linux_runtime, RuntimeVerdict::Unsupported);
    assert!(!report.linux_runtime_supported());
    assert!(report.notes.contains("untested"));
}

#[test]
fn detected_systemd_is_reported_as_untested() {
    let report = runtime_support_with(&full_systemd_inputs());

    assert_eq!(report.os_family, OsFamily::Linux);
    assert!(report.systemd_capable);
    assert!(report.systemd_present);
    assert_eq!(report.linux_runtime, RuntimeVerdict::Untested);
    assert!(!report.linux_runtime_supported());
    assert!(report.notes.contains("read-only path checks"));
}

#[test]
fn missing_systemd_markers_are_unsupported() {
    let missing_unit_dir = RuntimeInputs::for_family(OsFamily::Linux)
        .with_systemd_runtime_dir(true)
        .with_systemctl(true);
    let report = runtime_support_with(&missing_unit_dir);
    assert!(!report.systemd_present);
    assert_eq!(report.linux_runtime, RuntimeVerdict::Unsupported);

    let missing_systemctl = RuntimeInputs::for_family(OsFamily::Linux)
        .with_systemd_unit_dir(true)
        .with_systemd_runtime_dir(true);
    let report = runtime_support_with(&missing_systemctl);
    assert!(!report.systemd_present);
    assert_eq!(report.linux_runtime, RuntimeVerdict::Unsupported);
}

#[test]
fn other_os_family_is_unsupported() {
    let report = runtime_support_with(&RuntimeInputs::for_family(OsFamily::Other));
    assert_eq!(report.os_family, OsFamily::Other);
    assert!(!report.systemd_capable);
    assert_eq!(report.linux_runtime, RuntimeVerdict::Unsupported);
}

#[test]
fn probe_is_read_only_and_never_spawns_processes() {
    let report = runtime_support();
    assert_eq!(report.os_family, OsFamily::current());
    assert!(report.read_only_probe);
    assert!(!report.process_spawned);
    assert!(!report.linux_runtime_supported());

    if cfg!(target_os = "windows") {
        assert_eq!(report.os_family, OsFamily::Windows);
        assert_eq!(report.linux_runtime, RuntimeVerdict::Unsupported);
        assert!(!report.systemd_present);
    }
}

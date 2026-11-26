//! Declarative, side-effect-free OS service definitions.
//!
//! This module renders service registrations as data and strings only. It
//! never writes unit files, invokes `systemctl`/`sc.exe`, or otherwise changes
//! the host. Installing a service remains the responsibility of a separate,
//! explicitly authorized packaging step.

use std::fmt;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Conventional Aegis Platform service name.
pub const DEFAULT_SERVICE_NAME: &str = "aegis-platform";
/// Conventional systemd system unit directory.
pub const SYSTEMD_UNIT_DIR: &str = "/etc/systemd/system";
/// Maximum accepted service name length in bytes.
pub const MAX_SERVICE_NAME_LEN: usize = 64;
/// Maximum accepted service description length in bytes.
pub const MAX_DESCRIPTION_LEN: usize = 256;
/// Default restart delay rendered into systemd units.
pub const DEFAULT_RESTART_SEC: u32 = 5;

const DEFAULT_DESCRIPTION: &str = "Aegis Platform background service";
const WINDOWS_SC_PROGRAM: &str = "sc.exe";

const SYSTEMD_HARDENING: &[&str] = &[
    "NoNewPrivileges=true",
    "PrivateTmp=true",
    "ProtectSystem=full",
    "ProtectKernelTunables=true",
    "ProtectKernelModules=true",
    "ProtectControlGroups=true",
    "RestrictSUIDSGID=true",
    "LockPersonality=true",
    "RestrictRealtime=true",
];

/// Operating-system family targeted by a service registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlatformKind {
    Linux,
    Windows,
    Unsupported,
}

impl PlatformKind {
    /// The operating system this build is running on.
    pub fn current() -> Self {
        #[cfg(target_os = "linux")]
        {
            PlatformKind::Linux
        }
        #[cfg(target_os = "windows")]
        {
            PlatformKind::Windows
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            PlatformKind::Unsupported
        }
    }

    /// Whether this platform has a known OS service registration mechanism.
    pub fn supports_registration(self) -> bool {
        matches!(self, PlatformKind::Linux | PlatformKind::Windows)
    }

    /// Stable short label.
    pub fn as_str(self) -> &'static str {
        match self {
            PlatformKind::Linux => "linux",
            PlatformKind::Windows => "windows",
            PlatformKind::Unsupported => "unsupported",
        }
    }
}

/// systemd service readiness model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceType {
    /// The process is considered ready once it has been spawned.
    Simple,
    /// The process must send an `sd_notify` readiness datagram. The current
    /// `aegis-service-host` does not send one, so `Simple` is the default.
    Notify,
}

impl ServiceType {
    fn systemd_value(self) -> &'static str {
        match self {
            ServiceType::Simple => "simple",
            ServiceType::Notify => "notify",
        }
    }
}

/// Restart behaviour rendered into a systemd unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartPolicy {
    No,
    OnFailure,
    Always,
}

impl RestartPolicy {
    fn systemd_value(self) -> &'static str {
        match self {
            RestartPolicy::No => "no",
            RestartPolicy::OnFailure => "on-failure",
            RestartPolicy::Always => "always",
        }
    }
}

/// Validation failures for declarative service registrations.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RegistrationError {
    #[error("service name is empty")]
    EmptyName,
    #[error("service name is too long: {0} bytes")]
    NameTooLong(usize),
    #[error("service name contains unsupported characters: {0}")]
    InvalidName(String),
    #[error("service description contains control characters")]
    InvalidDescription,
    #[error("service description is too long")]
    DescriptionTooLong,
    #[error("executable path is empty")]
    EmptyExecutable,
    #[error("executable path must be absolute for {platform:?}: {path}")]
    RelativeExecutable {
        platform: PlatformKind,
        path: String,
    },
    #[error("working directory must be absolute for {platform:?}: {path}")]
    RelativeWorkingDirectory {
        platform: PlatformKind,
        path: String,
    },
    #[error("path contains invalid characters: {0}")]
    InvalidPath(String),
    #[error("path must not contain parent-directory components: {0}")]
    ParentDirectory(String),
    #[error("service argument contains control characters")]
    InvalidArgument,
    #[error("environment variable name is invalid: {0}")]
    InvalidEnvironmentName(String),
    #[error("environment variable value contains control characters: {0}")]
    InvalidEnvironmentValue(String),
    #[error("platform does not support OS service registration: {0:?}")]
    UnsupportedPlatform(PlatformKind),
    #[error("registration targets {expected:?} but rendering requested {actual:?}")]
    WrongPlatform {
        expected: PlatformKind,
        actual: PlatformKind,
    },
}

/// Platform-neutral description of one OS service.
///
/// Construction validates the required fields. Builders intentionally do not
/// validate immediately; `validate()` and every `render_*` method re-check the
/// complete registration so callers cannot render malformed data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceRegistration {
    pub target: PlatformKind,
    pub name: String,
    pub description: String,
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    pub working_directory: Option<PathBuf>,
    pub environment: Vec<(String, String)>,
    pub service_type: ServiceType,
    pub restart: RestartPolicy,
    pub restart_sec: u32,
}

impl ServiceRegistration {
    /// Build a registration for the current operating system.
    pub fn new(
        name: impl Into<String>,
        executable: impl Into<PathBuf>,
    ) -> Result<Self, RegistrationError> {
        Self::for_platform(PlatformKind::current(), name, executable)
    }

    /// Build a registration for an explicit target platform.
    pub fn for_platform(
        target: PlatformKind,
        name: impl Into<String>,
        executable: impl Into<PathBuf>,
    ) -> Result<Self, RegistrationError> {
        let registration = Self {
            target,
            name: name.into(),
            description: DEFAULT_DESCRIPTION.to_owned(),
            executable: executable.into(),
            arguments: Vec::new(),
            working_directory: None,
            environment: Vec::new(),
            service_type: ServiceType::Simple,
            restart: RestartPolicy::OnFailure,
            restart_sec: DEFAULT_RESTART_SEC,
        };
        registration.validate()?;
        Ok(registration)
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    pub fn with_argument(mut self, argument: impl Into<String>) -> Self {
        self.arguments.push(argument.into());
        self
    }

    pub fn with_arguments<I, S>(mut self, arguments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.arguments.extend(arguments.into_iter().map(Into::into));
        self
    }

    pub fn with_working_directory(mut self, directory: impl Into<PathBuf>) -> Self {
        self.working_directory = Some(directory.into());
        self
    }

    pub fn with_environment(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.environment.push((key.into(), value.into()));
        self
    }

    pub fn with_service_type(mut self, service_type: ServiceType) -> Self {
        self.service_type = service_type;
        self
    }

    pub fn with_restart(mut self, restart: RestartPolicy, restart_sec: u32) -> Self {
        self.restart = restart;
        self.restart_sec = restart_sec;
        self
    }

    /// Validate every field for the declared target platform.
    pub fn validate(&self) -> Result<(), RegistrationError> {
        validate_name(&self.name)?;
        validate_description(&self.description)?;
        if !self.target.supports_registration() {
            return Err(RegistrationError::UnsupportedPlatform(self.target));
        }
        validate_executable(&self.executable, self.target)?;
        if let Some(directory) = &self.working_directory {
            validate_working_directory(directory, self.target)?;
        }
        for argument in &self.arguments {
            validate_argument(argument)?;
        }
        for (key, value) in &self.environment {
            validate_environment(key, value)?;
        }
        Ok(())
    }

    /// Render this registration using the mechanism for its target platform.
    pub fn render(&self) -> Result<RenderedRegistration, RegistrationError> {
        match self.target {
            PlatformKind::Linux => self.render_systemd().map(RenderedRegistration::Systemd),
            PlatformKind::Windows => self.render_windows().map(RenderedRegistration::Windows),
            PlatformKind::Unsupported => Err(RegistrationError::UnsupportedPlatform(self.target)),
        }
    }

    /// Render a systemd system unit (`Type=simple` by default).
    pub fn render_systemd(&self) -> Result<ServiceUnit, RegistrationError> {
        self.validate()?;
        if self.target != PlatformKind::Linux {
            return Err(RegistrationError::WrongPlatform {
                expected: PlatformKind::Linux,
                actual: self.target,
            });
        }

        let description = self.effective_description();
        let mut lines = vec![
            "[Unit]".to_owned(),
            format!("Description={description}"),
            String::new(),
            "[Service]".to_owned(),
            format!("Type={}", self.service_type.systemd_value()),
        ];

        let mut exec_start = systemd_quote(self.executable.to_string_lossy().as_ref());
        for argument in &self.arguments {
            exec_start.push(' ');
            exec_start.push_str(&systemd_quote(argument));
        }
        lines.push(format!("ExecStart={exec_start}"));

        if let Some(directory) = &self.working_directory {
            lines.push(format!(
                "WorkingDirectory={}",
                systemd_quote(directory.to_string_lossy().as_ref())
            ));
        }
        for (key, value) in &self.environment {
            lines.push(format!("Environment={}", systemd_environment(key, value)));
        }
        lines.push(format!("Restart={}", self.restart.systemd_value()));
        if self.restart != RestartPolicy::No {
            lines.push(format!("RestartSec={}", self.restart_sec));
        }
        if self.service_type == ServiceType::Notify {
            lines.push("NotifyAccess=main".to_owned());
        }
        for option in SYSTEMD_HARDENING {
            lines.push((*option).to_owned());
        }
        lines.push(String::new());
        lines.push("[Install]".to_owned());
        lines.push("WantedBy=multi-user.target".to_owned());
        lines.push(String::new());

        Ok(ServiceUnit {
            platform: PlatformKind::Linux,
            name: self.name.clone(),
            suggested_path: Some(format!("{SYSTEMD_UNIT_DIR}/{}.service", self.name)),
            contents: lines.join("\n"),
        })
    }

    /// Render the exact `sc.exe` command specifications for Windows SCM.
    ///
    /// The result is data. No command is executed by Aegis.
    pub fn render_windows(&self) -> Result<WindowsServiceSpecs, RegistrationError> {
        self.validate()?;
        if self.target != PlatformKind::Windows {
            return Err(RegistrationError::WrongPlatform {
                expected: PlatformKind::Windows,
                actual: self.target,
            });
        }

        let description = self.effective_description();
        let bin_path = windows_command_line(&self.executable, &self.arguments);
        let create = ServiceCommandSpec::new(vec![
            "create".to_owned(),
            self.name.clone(),
            "binPath=".to_owned(),
            bin_path,
            "start=".to_owned(),
            "auto".to_owned(),
            "DisplayName=".to_owned(),
            description,
        ]);
        let start = ServiceCommandSpec::new(vec!["start".to_owned(), self.name.clone()]);
        let stop = ServiceCommandSpec::new(vec!["stop".to_owned(), self.name.clone()]);
        let query = ServiceCommandSpec::new(vec!["query".to_owned(), self.name.clone()]);
        let delete = ServiceCommandSpec::new(vec!["delete".to_owned(), self.name.clone()]);

        Ok(WindowsServiceSpecs {
            name: self.name.clone(),
            create,
            start,
            stop,
            query,
            delete,
        })
    }

    /// Build an ordered, idempotency-annotated install plan as data.
    ///
    /// No step is executed. systemd plans write and enable a unit; Windows
    /// plans document the `sc.exe` sequence. Each step carries a note about
    /// how to repeat it safely.
    pub fn install_plan(&self) -> Result<InstallPlan, RegistrationError> {
        self.validate()?;
        match self.target {
            PlatformKind::Linux => self.install_plan_systemd(),
            PlatformKind::Windows => self.install_plan_windows(),
            PlatformKind::Unsupported => Err(RegistrationError::UnsupportedPlatform(self.target)),
        }
    }

    fn install_plan_systemd(&self) -> Result<InstallPlan, RegistrationError> {
        let unit = self.render_systemd()?;
        let name = self.name.as_str();
        let unit_path = unit
            .suggested_path
            .clone()
            .unwrap_or_else(|| format!("{SYSTEMD_UNIT_DIR}/{}.service", self.name));
        let steps = vec![
            InstallStep {
                order: 1,
                action: InstallAction::WriteUnitFile,
                description: format!("Write the rendered unit to {unit_path}"),
                command: None,
                idempotent: true,
                note: "Rendering is deterministic; overwriting with identical bytes is a no-op."
                    .to_owned(),
            },
            InstallStep {
                order: 2,
                action: InstallAction::ReloadDaemon,
                description: "Reload the systemd manager configuration".to_owned(),
                command: Some(InstallCommand::from_tokens("systemctl", &["daemon-reload"])),
                idempotent: true,
                note: "Safe to repeat; reloading an unchanged unit set has no effect.".to_owned(),
            },
            InstallStep {
                order: 3,
                action: InstallAction::Enable,
                description: format!("Enable and start {}", self.name),
                command: Some(InstallCommand::from_tokens(
                    "systemctl",
                    &["enable", "--now", name],
                )),
                idempotent: true,
                note: "`enable --now` is a no-op when the unit is already enabled and active."
                    .to_owned(),
            },
            InstallStep {
                order: 4,
                action: InstallAction::Query,
                description: format!("Verify that {} is active", self.name),
                command: Some(InstallCommand::from_tokens(
                    "systemctl",
                    &["status", name, "--no-pager"],
                )),
                idempotent: true,
                note: "Read-only verification; safe to repeat.".to_owned(),
            },
        ];
        Ok(InstallPlan {
            platform: PlatformKind::Linux,
            service_name: self.name.clone(),
            unit_path: Some(unit_path),
            steps,
        })
    }

    fn install_plan_windows(&self) -> Result<InstallPlan, RegistrationError> {
        let specs = self.render_windows()?;
        let query_existing = InstallStep {
            order: 1,
            action: InstallAction::Query,
            description: format!("Query {0} before creating it", self.name),
            command: Some(InstallCommand::from(&specs.query)),
            idempotent: true,
            note: "Read-only; use the result to skip `create` when the service already exists."
                .to_owned(),
        };
        let create = InstallStep {
            order: 2,
            action: InstallAction::CreateService,
            description: format!("Create the {0} service", self.name),
            command: Some(InstallCommand::from(&specs.create)),
            idempotent: false,
            note: "`sc.exe create` fails when the service exists; skip it when step 1 succeeds."
                .to_owned(),
        };
        let start = InstallStep {
            order: 3,
            action: InstallAction::Start,
            description: format!("Start the {0} service", self.name),
            command: Some(InstallCommand::from(&specs.start)),
            idempotent: false,
            note: "Starting an already-running service fails; query state before repeating."
                .to_owned(),
        };
        let verify = InstallStep {
            order: 4,
            action: InstallAction::Query,
            description: format!("Verify the {0} service state", self.name),
            command: Some(InstallCommand::from(&specs.query)),
            idempotent: true,
            note: "Read-only verification; safe to repeat.".to_owned(),
        };
        Ok(InstallPlan {
            platform: PlatformKind::Windows,
            service_name: self.name.clone(),
            unit_path: None,
            steps: vec![query_existing, create, start, verify],
        })
    }

    fn effective_description(&self) -> String {
        if self.description.is_empty() {
            DEFAULT_DESCRIPTION.to_owned()
        } else {
            self.description.clone()
        }
    }
}

/// The kind of action an install step represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallAction {
    WriteUnitFile,
    ReloadDaemon,
    Enable,
    Start,
    Query,
    CreateService,
}

impl InstallAction {
    /// Stable label used by [`InstallPlan::render`].
    pub fn as_str(self) -> &'static str {
        match self {
            InstallAction::WriteUnitFile => "write-unit-file",
            InstallAction::ReloadDaemon => "reload-daemon",
            InstallAction::Enable => "enable",
            InstallAction::Start => "start",
            InstallAction::Query => "query",
            InstallAction::CreateService => "create-service",
        }
    }
}

/// One install command represented as unexecuted data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallCommand {
    pub program: String,
    pub arguments: Vec<String>,
}

impl InstallCommand {
    pub fn new<I, S>(program: impl Into<String>, arguments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            program: program.into(),
            arguments: arguments.into_iter().map(Into::into).collect(),
        }
    }

    fn from_tokens(program: &str, tokens: &[&str]) -> Self {
        Self::new(program, tokens.iter().copied())
    }

    pub fn arguments(&self) -> &[String] {
        &self.arguments
    }

    /// Display-only command line; not guaranteed to round-trip through a shell.
    pub fn render(&self) -> String {
        let mut rendered = windows_quote(&self.program);
        for argument in &self.arguments {
            rendered.push(' ');
            rendered.push_str(&windows_quote(argument));
        }
        rendered
    }
}

impl From<&ServiceCommandSpec> for InstallCommand {
    fn from(spec: &ServiceCommandSpec) -> Self {
        Self {
            program: spec.program.clone(),
            arguments: spec.arguments.clone(),
        }
    }
}

impl fmt::Display for InstallCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.render())
    }
}

/// One ordered step in an install plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallStep {
    pub order: u32,
    pub action: InstallAction,
    pub description: String,
    pub command: Option<InstallCommand>,
    pub idempotent: bool,
    pub note: String,
}

/// Ordered, side-effect-free install plan for one platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallPlan {
    pub platform: PlatformKind,
    pub service_name: String,
    pub unit_path: Option<String>,
    pub steps: Vec<InstallStep>,
}

impl InstallPlan {
    /// Render the plan as human-readable text. Never executed.
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
                "{}. [{}] {} [{}]",
                step.order,
                step.action.as_str(),
                step.description,
                if step.idempotent {
                    "idempotent"
                } else {
                    "not-idempotent"
                }
            ));
            if let Some(command) = &step.command {
                lines.push(format!("   command: {}", command.render()));
            }
            lines.push(format!("   note: {}", step.note));
        }
        lines.join("\n")
    }
}

impl fmt::Display for InstallPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.render())
    }
}

/// A rendered systemd unit artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceUnit {
    pub platform: PlatformKind,
    pub name: String,
    pub suggested_path: Option<String>,
    pub contents: String,
}

impl ServiceUnit {
    /// The exact unit file text.
    pub fn render(&self) -> &str {
        &self.contents
    }
}

impl fmt::Display for ServiceUnit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.contents)
    }
}

/// One `sc.exe` command represented as unexecuted data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceCommandSpec {
    pub program: String,
    pub arguments: Vec<String>,
}

impl ServiceCommandSpec {
    fn new(arguments: Vec<String>) -> Self {
        Self {
            program: WINDOWS_SC_PROGRAM.to_owned(),
            arguments,
        }
    }

    /// Exact argv tokens, including the doubled `key=`/value form required by
    /// `sc.exe`.
    pub fn arguments(&self) -> &[String] {
        &self.arguments
    }

    /// A display-only command line. This is not guaranteed to round-trip
    /// through a shell and must not be executed blindly.
    pub fn render(&self) -> String {
        let mut rendered = self.program.clone();
        for argument in &self.arguments {
            rendered.push(' ');
            rendered.push_str(&windows_quote(argument));
        }
        rendered
    }
}

impl fmt::Display for ServiceCommandSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.render())
    }
}

/// Windows SCM lifecycle commands for one service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowsServiceSpecs {
    pub name: String,
    pub create: ServiceCommandSpec,
    pub start: ServiceCommandSpec,
    pub stop: ServiceCommandSpec,
    pub query: ServiceCommandSpec,
    pub delete: ServiceCommandSpec,
}

impl WindowsServiceSpecs {
    /// Render every command, one per line, as unexecuted display text.
    pub fn render(&self) -> String {
        [
            self.create.render(),
            self.start.render(),
            self.stop.render(),
            self.query.render(),
            self.delete.render(),
        ]
        .join("\n")
    }
}

impl fmt::Display for WindowsServiceSpecs {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.render())
    }
}

/// A registration rendered for its target platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderedRegistration {
    Systemd(ServiceUnit),
    Windows(WindowsServiceSpecs),
}

impl RenderedRegistration {
    pub fn platform(&self) -> PlatformKind {
        match self {
            RenderedRegistration::Systemd(unit) => unit.platform,
            RenderedRegistration::Windows(_) => PlatformKind::Windows,
        }
    }

    /// Render the underlying artifact as display text.
    pub fn render(&self) -> String {
        match self {
            RenderedRegistration::Systemd(unit) => unit.render().to_owned(),
            RenderedRegistration::Windows(specs) => specs.render(),
        }
    }
}

impl fmt::Display for RenderedRegistration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.render())
    }
}

/// Capability report for OS-level service registration in this build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServicePlatform {
    pub os: String,
    pub arch: String,
    pub kind: PlatformKind,
    /// The target OS has a known registration mechanism (systemd/SCM).
    pub registration_supported: bool,
    /// This build can produce declarative definitions for a supported target.
    pub generation_supported: bool,
    /// Runtime installation is never performed by this module.
    pub runtime_registration_performed: bool,
    pub mechanism: Option<String>,
    pub notes: String,
}

/// Probe service-registration capabilities without touching the host.
pub fn platform() -> ServicePlatform {
    let kind = PlatformKind::current();
    let mechanism = match kind {
        PlatformKind::Linux => Some("systemd".to_owned()),
        PlatformKind::Windows => Some("windows-scm".to_owned()),
        PlatformKind::Unsupported => None,
    };
    ServicePlatform {
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
        kind,
        registration_supported: kind.supports_registration(),
        generation_supported: true,
        runtime_registration_performed: false,
        mechanism,
        notes: "Definitions are generated as data only; this build does not install, start, \
                stop, or modify OS services."
            .to_owned(),
    }
}

fn validate_name(name: &str) -> Result<(), RegistrationError> {
    if name.is_empty() {
        return Err(RegistrationError::EmptyName);
    }
    if name.len() > MAX_SERVICE_NAME_LEN {
        return Err(RegistrationError::NameTooLong(name.len()));
    }
    let mut characters = name.chars();
    let first = characters.next().expect("non-empty name was checked");
    if !first.is_ascii_alphanumeric() {
        return Err(RegistrationError::InvalidName(name.to_owned()));
    }
    if !characters
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
    {
        return Err(RegistrationError::InvalidName(name.to_owned()));
    }
    Ok(())
}

fn validate_description(description: &str) -> Result<(), RegistrationError> {
    if description.chars().any(char::is_control) {
        return Err(RegistrationError::InvalidDescription);
    }
    if description.len() > MAX_DESCRIPTION_LEN {
        return Err(RegistrationError::DescriptionTooLong);
    }
    Ok(())
}

fn validate_executable(path: &Path, platform: PlatformKind) -> Result<(), RegistrationError> {
    if path.as_os_str().is_empty() {
        return Err(RegistrationError::EmptyExecutable);
    }
    validate_path_text(path)?;
    if !is_absolute_for(platform, path) {
        return Err(RegistrationError::RelativeExecutable {
            platform,
            path: path.display().to_string(),
        });
    }
    Ok(())
}

fn validate_working_directory(
    path: &Path,
    platform: PlatformKind,
) -> Result<(), RegistrationError> {
    validate_path_text(path)?;
    if !is_absolute_for(platform, path) {
        return Err(RegistrationError::RelativeWorkingDirectory {
            platform,
            path: path.display().to_string(),
        });
    }
    Ok(())
}

fn validate_path_text(path: &Path) -> Result<(), RegistrationError> {
    let text = path.to_string_lossy();
    if text.chars().any(char::is_control) {
        return Err(RegistrationError::InvalidPath(text.into_owned()));
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(RegistrationError::ParentDirectory(text.into_owned()));
    }
    Ok(())
}

fn validate_argument(argument: &str) -> Result<(), RegistrationError> {
    if argument.chars().any(char::is_control) {
        return Err(RegistrationError::InvalidArgument);
    }
    Ok(())
}

fn validate_environment(key: &str, value: &str) -> Result<(), RegistrationError> {
    let mut characters = key.chars();
    let valid_start = matches!(
        characters.next(),
        Some(character) if character.is_ascii_alphabetic() || character == '_'
    );
    let valid_rest =
        characters.all(|character| character.is_ascii_alphanumeric() || character == '_');
    if !valid_start || !valid_rest {
        return Err(RegistrationError::InvalidEnvironmentName(key.to_owned()));
    }
    if value.chars().any(char::is_control) {
        return Err(RegistrationError::InvalidEnvironmentValue(key.to_owned()));
    }
    Ok(())
}

fn is_absolute_for(platform: PlatformKind, path: &Path) -> bool {
    match platform {
        PlatformKind::Linux => path.to_string_lossy().starts_with('/'),
        PlatformKind::Windows => {
            let text = path.to_string_lossy();
            let bytes = text.as_bytes();
            let drive_absolute = bytes.len() >= 3
                && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':'
                && (bytes[2] == b'\\' || bytes[2] == b'/');
            drive_absolute || (text.starts_with("\\\\") && text.len() > 2)
        }
        PlatformKind::Unsupported => path.is_absolute(),
    }
}

fn systemd_quote(value: &str) -> String {
    let needs_quotes = value.is_empty()
        || value.chars().any(|character| {
            character.is_whitespace()
                || matches!(
                    character,
                    '"' | '\\'
                        | '\''
                        | '$'
                        | '%'
                        | ';'
                        | '&'
                        | '|'
                        | '<'
                        | '>'
                        | '('
                        | ')'
                        | '#'
                        | '!'
                )
        });
    if !needs_quotes {
        return value.to_owned();
    }
    format!("\"{}\"", systemd_escape(value))
}

fn systemd_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '%' => escaped.push_str("%%"),
            '$' => escaped.push_str("$$"),
            other => escaped.push(other),
        }
    }
    escaped
}

fn systemd_environment(key: &str, value: &str) -> String {
    let assignment = format!("{key}={value}");
    format!("\"{}\"", systemd_escape(&assignment))
}

fn windows_command_line(executable: &Path, arguments: &[String]) -> String {
    let mut command = windows_quote(executable.to_string_lossy().as_ref());
    for argument in arguments {
        command.push(' ');
        command.push_str(&windows_quote(argument));
    }
    command
}

fn windows_quote(value: &str) -> String {
    let needs_quotes = value.is_empty()
        || value
            .chars()
            .any(|character| character.is_whitespace() || character == '"');
    if !needs_quotes {
        return value.to_owned();
    }

    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    let mut backslashes = 0;
    for character in value.chars() {
        match character {
            '\\' => {
                backslashes += 1;
                quoted.push('\\');
            }
            '"' => {
                for _ in 0..backslashes {
                    quoted.push('\\');
                }
                quoted.push_str("\\\"");
                backslashes = 0;
            }
            other => {
                backslashes = 0;
                quoted.push(other);
            }
        }
    }
    for _ in 0..backslashes {
        quoted.push('\\');
    }
    quoted.push('"');
    quoted
}

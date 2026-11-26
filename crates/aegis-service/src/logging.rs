//! Bounded log retention and documented journal integration.
//!
//! This module is a specification expressed as data plus pure validation.
//! Nothing here writes logs, invokes `journalctl`, edits journald configuration,
//! or otherwise changes the host. Local file retention is owned by a future
//! runtime; journal capture is owned by systemd-journald.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Maximum accepted rotation files per service.
pub const MAX_LOG_FILES: u32 = 100;
/// Maximum accepted size of one log file.
pub const MAX_LOG_BYTES_PER_FILE: u64 = 1024 * 1024 * 1024;
/// Maximum accepted total bytes across retained files.
pub const MAX_LOG_TOTAL_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// Maximum accepted retention age.
pub const MAX_LOG_AGE_SECS: u64 = 90 * 24 * 60 * 60;

/// Default number of rotated log files kept.
pub const DEFAULT_LOG_FILES: u32 = 5;
/// Default size of one log file before rotation.
pub const DEFAULT_LOG_BYTES_PER_FILE: u64 = 8 * 1024 * 1024;
/// Default total bytes across retained files.
pub const DEFAULT_LOG_TOTAL_BYTES: u64 = 40 * 1024 * 1024;
/// Default retention age (seven days).
pub const DEFAULT_LOG_AGE_SECS: u64 = 7 * 24 * 60 * 60;

/// Bounded local log retention configuration. Validated as a whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogRetentionConfig {
    pub max_files: u32,
    pub max_bytes_per_file: u64,
    pub max_total_bytes: u64,
    pub max_age_secs: u64,
    pub rotate_on_restart: bool,
    pub compress_rotated: bool,
}

impl Default for LogRetentionConfig {
    fn default() -> Self {
        Self {
            max_files: DEFAULT_LOG_FILES,
            max_bytes_per_file: DEFAULT_LOG_BYTES_PER_FILE,
            max_total_bytes: DEFAULT_LOG_TOTAL_BYTES,
            max_age_secs: DEFAULT_LOG_AGE_SECS,
            rotate_on_restart: true,
            compress_rotated: true,
        }
    }
}

impl LogRetentionConfig {
    /// Build and validate a retention configuration from explicit limits.
    pub fn new(
        max_files: u32,
        max_bytes_per_file: u64,
        max_total_bytes: u64,
        max_age_secs: u64,
    ) -> Result<Self, LoggingError> {
        let config = Self {
            max_files,
            max_bytes_per_file,
            max_total_bytes,
            max_age_secs,
            ..Self::default()
        };
        config.validate()?;
        Ok(config)
    }

    /// Re-check every bound. Every consumer validates so public fields cannot
    /// bypass the limits.
    pub fn validate(&self) -> Result<(), LoggingError> {
        if self.max_files == 0 {
            return Err(LoggingError::ZeroFiles);
        }
        if self.max_files > MAX_LOG_FILES {
            return Err(LoggingError::TooManyFiles {
                limit: MAX_LOG_FILES,
                actual: self.max_files,
            });
        }
        if self.max_bytes_per_file == 0 {
            return Err(LoggingError::ZeroFileBytes);
        }
        if self.max_bytes_per_file > MAX_LOG_BYTES_PER_FILE {
            return Err(LoggingError::FileBytesTooLarge {
                limit: MAX_LOG_BYTES_PER_FILE,
                actual: self.max_bytes_per_file,
            });
        }
        if self.max_total_bytes == 0 {
            return Err(LoggingError::ZeroTotalBytes);
        }
        if self.max_total_bytes > MAX_LOG_TOTAL_BYTES {
            return Err(LoggingError::TotalBytesTooLarge {
                limit: MAX_LOG_TOTAL_BYTES,
                actual: self.max_total_bytes,
            });
        }
        if self.max_total_bytes < self.max_bytes_per_file {
            return Err(LoggingError::TotalLessThanFile {
                total: self.max_total_bytes,
                file: self.max_bytes_per_file,
            });
        }
        if self.max_age_secs == 0 {
            return Err(LoggingError::ZeroMaxAge);
        }
        if self.max_age_secs > MAX_LOG_AGE_SECS {
            return Err(LoggingError::MaxAgeTooLarge {
                limit: MAX_LOG_AGE_SECS,
                actual: self.max_age_secs,
            });
        }
        Ok(())
    }

    /// Upper bound on bytes retained locally, never exceeding `max_total_bytes`.
    pub fn effective_total_bytes(&self) -> u64 {
        self.max_bytes_per_file
            .saturating_mul(u64::from(self.max_files))
            .min(self.max_total_bytes)
    }
}

/// How the service emits journal records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JournalTransport {
    /// Plain lines on stdout/stderr, captured by systemd-journald.
    StdoutStderr,
    /// Structured records sent through the journal socket.
    NativeJournal,
}

impl JournalTransport {
    pub fn as_str(self) -> &'static str {
        match self {
            JournalTransport::StdoutStderr => "stdout-stderr",
            JournalTransport::NativeJournal => "native-journal",
        }
    }
}

/// Which component owns rotation for each sink.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RetentionOwner {
    /// systemd-journald enforces its own size/time limits.
    Journald,
    /// The service owns bounded local files.
    Aegis,
    /// Both, for journal plus a local fallback.
    Both,
}

impl RetentionOwner {
    pub fn as_str(self) -> &'static str {
        match self {
            RetentionOwner::Journald => "journald",
            RetentionOwner::Aegis => "aegis",
            RetentionOwner::Both => "both",
        }
    }
}

/// Documented journal integration approach. Data only; nothing is written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalIntegration {
    pub unit_name: String,
    /// `SyslogIdentifier` / journal identifier for filtering.
    pub identifier: String,
    pub transport: JournalTransport,
    pub retention_owner: RetentionOwner,
    /// Ordered operator-facing failure-recovery notes.
    pub failure_recovery: Vec<String>,
}

impl JournalIntegration {
    /// Validate that the integration description is usable.
    pub fn validate(&self) -> Result<(), LoggingError> {
        if self.unit_name.is_empty() || self.unit_name.chars().any(char::is_control) {
            return Err(LoggingError::InvalidUnitName);
        }
        if self.identifier.is_empty() || self.identifier.chars().any(char::is_control) {
            return Err(LoggingError::InvalidIdentifier);
        }
        if self.failure_recovery.is_empty() {
            return Err(LoggingError::MissingFailureRecovery);
        }
        if self
            .failure_recovery
            .iter()
            .any(|note| note.is_empty() || note.chars().any(char::is_control))
        {
            return Err(LoggingError::InvalidFailureRecovery);
        }
        Ok(())
    }

    /// Documented approach as display text. Nothing is executed.
    pub fn render(&self) -> String {
        let mut lines = vec![
            "journal-integration: documented".to_owned(),
            format!("unit: {}", self.unit_name),
            format!("identifier: {}", self.identifier),
            format!("transport: {}", self.transport.as_str()),
            format!("retention-owner: {}", self.retention_owner.as_str()),
            "approach: the service host writes plain lines to stdout/stderr; systemd captures \
             them in the journal under the unit and identifier above."
                .to_owned(),
            format!("query: journalctl -u {} -n 200 --no-pager", self.unit_name),
        ];
        for (index, note) in self.failure_recovery.iter().enumerate() {
            lines.push(format!("recovery-{}: {note}", index + 1));
        }
        lines.join("\n")
    }
}

impl fmt::Display for JournalIntegration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.render())
    }
}

/// Combined logging specification: bounded local retention plus journal plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoggingSpecification {
    pub retention: LogRetentionConfig,
    pub journal: JournalIntegration,
}

impl LoggingSpecification {
    /// Default, validated specification for the conventional unit name.
    pub fn default_for(unit_name: impl Into<String>) -> Result<Self, LoggingError> {
        let specification = Self {
            retention: LogRetentionConfig::default(),
            journal: journal_integration(unit_name)?,
        };
        specification.validate()?;
        Ok(specification)
    }

    pub fn validate(&self) -> Result<(), LoggingError> {
        self.retention.validate()?;
        self.journal.validate()
    }

    pub fn render(&self) -> String {
        [
            format!(
                "retention: files={} file-bytes={} total-bytes={} age-secs={} \
                 rotate-on-restart={} compress={}",
                self.retention.max_files,
                self.retention.max_bytes_per_file,
                self.retention.max_total_bytes,
                self.retention.max_age_secs,
                self.retention.rotate_on_restart,
                self.retention.compress_rotated
            ),
            self.journal.render(),
        ]
        .join("\n")
    }
}

impl fmt::Display for LoggingSpecification {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.render())
    }
}

/// Standard documented journal approach for the conventional service.
pub fn journal_integration(
    unit_name: impl Into<String>,
) -> Result<JournalIntegration, LoggingError> {
    let integration = JournalIntegration {
        unit_name: unit_name.into(),
        identifier: "aegis-platform".to_owned(),
        transport: JournalTransport::StdoutStderr,
        retention_owner: RetentionOwner::Both,
        failure_recovery: vec![
            "Inspect recent lines with `journalctl -u <unit> -n 200 --no-pager`.".to_owned(),
            "Inspect the current boot with `journalctl -b -u <unit> --no-pager`.".to_owned(),
            "Confirm `run/service.json` ownership matches the unit's User=/Group=.".to_owned(),
            "If journald rotation is too aggressive, tune journald limits outside Aegis; do not \
             raise Aegis local retention above MAX_LOG_* bounds."
                .to_owned(),
            "If the service is crash-looping, stop the unit and use the recorded health reasons \
             and restart decision before restarting."
                .to_owned(),
        ],
    };
    integration.validate()?;
    Ok(integration)
}

/// Validation failures for retention and journal specifications.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum LoggingError {
    #[error("log retention must keep at least one file")]
    ZeroFiles,
    #[error("log file count {actual} exceeds the limit of {limit}")]
    TooManyFiles { limit: u32, actual: u32 },
    #[error("log file size must be greater than zero")]
    ZeroFileBytes,
    #[error("log file size {actual} exceeds the limit of {limit}")]
    FileBytesTooLarge { limit: u64, actual: u64 },
    #[error("total log bytes must be greater than zero")]
    ZeroTotalBytes,
    #[error("total log bytes {actual} exceeds the limit of {limit}")]
    TotalBytesTooLarge { limit: u64, actual: u64 },
    #[error("total log bytes {total} is smaller than one file {file}")]
    TotalLessThanFile { total: u64, file: u64 },
    #[error("log retention age must be greater than zero")]
    ZeroMaxAge,
    #[error("log retention age {actual} exceeds the limit of {limit}")]
    MaxAgeTooLarge { limit: u64, actual: u64 },
    #[error("journal unit name is empty or contains control characters")]
    InvalidUnitName,
    #[error("journal identifier is empty or contains control characters")]
    InvalidIdentifier,
    #[error("journal integration requires at least one failure-recovery note")]
    MissingFailureRecovery,
    #[error("journal failure-recovery note is empty or contains control characters")]
    InvalidFailureRecovery,
}

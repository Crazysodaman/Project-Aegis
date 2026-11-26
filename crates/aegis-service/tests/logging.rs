use aegis_service::{
    journal_integration, JournalTransport, LogRetentionConfig, LoggingError, LoggingSpecification,
    RetentionOwner, MAX_LOG_AGE_SECS, MAX_LOG_BYTES_PER_FILE, MAX_LOG_FILES, MAX_LOG_TOTAL_BYTES,
};

#[test]
fn default_retention_is_bounded_and_valid() {
    let config = LogRetentionConfig::default();
    assert_eq!(config.validate(), Ok(()));
    assert!(config.max_total_bytes >= config.max_bytes_per_file);
    assert!(config.effective_total_bytes() <= config.max_total_bytes);
    assert!(
        config.effective_total_bytes()
            <= config
                .max_bytes_per_file
                .saturating_mul(u64::from(config.max_files))
    );
    assert!(config.compress_rotated);
    assert!(config.rotate_on_restart);
}

#[test]
fn explicit_retention_is_validated() {
    let config =
        LogRetentionConfig::new(3, 1024 * 1024, 2 * 1024 * 1024, 3600).expect("valid retention");
    assert_eq!(config.effective_total_bytes(), 2 * 1024 * 1024);
    assert_eq!(config.validate(), Ok(()));

    let invalid = LogRetentionConfig::new(3, 2 * 1024 * 1024, 1024 * 1024, 3600);
    assert_eq!(
        invalid,
        Err(LoggingError::TotalLessThanFile {
            total: 1024 * 1024,
            file: 2 * 1024 * 1024,
        })
    );
}

#[test]
fn retention_limits_are_enforced() {
    let base = LogRetentionConfig::default();

    assert_eq!(
        LogRetentionConfig {
            max_files: 0,
            ..base
        }
        .validate(),
        Err(LoggingError::ZeroFiles)
    );
    assert_eq!(
        LogRetentionConfig {
            max_files: MAX_LOG_FILES + 1,
            ..base
        }
        .validate(),
        Err(LoggingError::TooManyFiles {
            limit: MAX_LOG_FILES,
            actual: MAX_LOG_FILES + 1,
        })
    );
    assert_eq!(
        LogRetentionConfig {
            max_bytes_per_file: 0,
            ..base
        }
        .validate(),
        Err(LoggingError::ZeroFileBytes)
    );
    assert_eq!(
        LogRetentionConfig {
            max_bytes_per_file: MAX_LOG_BYTES_PER_FILE + 1,
            max_total_bytes: MAX_LOG_TOTAL_BYTES,
            ..base
        }
        .validate(),
        Err(LoggingError::FileBytesTooLarge {
            limit: MAX_LOG_BYTES_PER_FILE,
            actual: MAX_LOG_BYTES_PER_FILE + 1,
        })
    );
    assert_eq!(
        LogRetentionConfig {
            max_total_bytes: 0,
            ..base
        }
        .validate(),
        Err(LoggingError::ZeroTotalBytes)
    );
    assert_eq!(
        LogRetentionConfig {
            max_total_bytes: MAX_LOG_TOTAL_BYTES + 1,
            ..base
        }
        .validate(),
        Err(LoggingError::TotalBytesTooLarge {
            limit: MAX_LOG_TOTAL_BYTES,
            actual: MAX_LOG_TOTAL_BYTES + 1,
        })
    );
    assert_eq!(
        LogRetentionConfig {
            max_age_secs: 0,
            ..base
        }
        .validate(),
        Err(LoggingError::ZeroMaxAge)
    );
    assert_eq!(
        LogRetentionConfig {
            max_age_secs: MAX_LOG_AGE_SECS + 1,
            ..base
        }
        .validate(),
        Err(LoggingError::MaxAgeTooLarge {
            limit: MAX_LOG_AGE_SECS,
            actual: MAX_LOG_AGE_SECS + 1,
        })
    );
}

#[test]
fn journal_integration_documents_approach_without_writing() {
    let integration = journal_integration("aegis-platform").expect("journal integration");
    assert_eq!(integration.validate(), Ok(()));
    assert_eq!(integration.unit_name, "aegis-platform");
    assert_eq!(integration.transport, JournalTransport::StdoutStderr);
    assert_eq!(integration.retention_owner, RetentionOwner::Both);
    assert!(!integration.failure_recovery.is_empty());

    let rendered = integration.render();
    assert_eq!(rendered, integration.to_string());
    assert!(rendered.contains("journal-integration: documented"));
    assert!(rendered.contains("journalctl -u aegis-platform"));
    assert!(rendered.contains("retention-owner: both"));
    assert!(rendered.contains("transport: stdout-stderr"));
    assert!(rendered.contains("recovery-1:"));
}

#[test]
fn journal_integration_validation_rejects_invalid_specs() {
    let mut empty_unit = journal_integration("aegis-platform").unwrap();
    empty_unit.unit_name.clear();
    assert_eq!(empty_unit.validate(), Err(LoggingError::InvalidUnitName));

    let mut empty_identifier = journal_integration("aegis-platform").unwrap();
    empty_identifier.identifier.clear();
    assert_eq!(
        empty_identifier.validate(),
        Err(LoggingError::InvalidIdentifier)
    );

    let mut no_recovery = journal_integration("aegis-platform").unwrap();
    no_recovery.failure_recovery.clear();
    assert_eq!(
        no_recovery.validate(),
        Err(LoggingError::MissingFailureRecovery)
    );

    let mut bad_recovery = journal_integration("aegis-platform").unwrap();
    bad_recovery.failure_recovery = vec!["line\nbreak".to_owned()];
    assert_eq!(
        bad_recovery.validate(),
        Err(LoggingError::InvalidFailureRecovery)
    );
}

#[test]
fn logging_specification_combines_retention_and_journal() {
    let specification = LoggingSpecification::default_for("aegis-platform").expect("spec");
    assert_eq!(specification.validate(), Ok(()));

    let rendered = specification.render();
    assert_eq!(rendered, specification.to_string());
    assert!(rendered.contains("retention: files="));
    assert!(rendered.contains("journal-integration: documented"));

    let invalid = LoggingSpecification {
        retention: LogRetentionConfig {
            max_files: 0,
            ..LogRetentionConfig::default()
        },
        journal: specification.journal,
    };
    assert_eq!(invalid.validate(), Err(LoggingError::ZeroFiles));
}

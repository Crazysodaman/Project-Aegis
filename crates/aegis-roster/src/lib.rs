//! Approved Aegis Core roster catalog.
//!
//! The roster is **data**, not Platform code. It records which Core numbers the
//! owner has approved, which display identity each one starts with, and which
//! numbers remain prohibited. It grants no runtime authority and never creates
//! a Core.
//!
//! Rules encoded here deliberately mirror the owner-approved roster:
//!
//! * Core #1 `Sofía Ada Lyra`, #2 `Mack Fuller`, #3 `Nova Arden`, and #6
//!   `Vera / S-039` have defined identities and may be installed.
//! * Cores #4 and #5 are the identical twins. They are design-only and their
//!   implementation is prohibited until the owner separately authorizes it.
//! * Core #7 is `Unwritten` with a blank identity. Its display name must be
//!   supplied by the owner; this catalog never invents a name, gender, species,
//!   appearance, voice, or personality for it.
//!
//! The catalog also does not invent identities for Cores #4 and #5.

use std::sync::LazyLock;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Schema identifier for the roster catalog.
pub const ROSTER_SCHEMA: &str = "aegis.roster.v1";

/// Maximum display-name length, matching the shared Core identity contract.
pub const MAX_DISPLAY_NAME_CHARS: usize = 160;

/// Authoritative UUID of approved Core #1, Sofía Ada Lyra.
///
/// This value is owner-supplied and must never be invented or altered.
pub const SOFIA_CANONICAL_UUID: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";

/// Authoritative Constitution SHA-256 digest of approved Core #1.
///
/// This value is owner-supplied and must never be invented or altered.
pub const SOFIA_CONSTITUTION_SHA256: &str =
    "F30AFEDF195F77907D32A108CA35D4DB751646BDE3FD902BE66E7B78F4B89DC0";

/// Design status of an approved Core number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RosterDesignStatus {
    /// Approved for implementation and installation.
    Implementable,
    /// Reserved for a design that is not yet approved for implementation.
    DesignOnly,
    /// Explicitly prohibited from implementation.
    Prohibited,
}

impl RosterDesignStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Implementable => "implementable",
            Self::DesignOnly => "design_only",
            Self::Prohibited => "prohibited",
        }
    }
}

/// One approved or reserved Core number.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovedCoreDesign {
    pub schema: String,
    pub core_number: u8,
    /// Stable catalog designation. This is policy metadata, not a Core UUID.
    pub designation: String,
    /// Approved initial display name, when one is defined.
    pub default_display_name: Option<String>,
    pub status: RosterDesignStatus,
    /// Whether the catalog defines this Core's identity at all.
    pub identity_defined: bool,
    /// Whether installation is authorized by the approved roster.
    pub implementation_authorized: bool,
    /// Whether the legacy Sofía identity must be preserved by reference.
    pub preserve_legacy_identity: bool,
    /// Authoritative owner-supplied canonical Core UUID, only when one is known.
    ///
    /// This is `None` for Cores whose canonical UUID has not been supplied by
    /// the owner; the catalog must not invent one.
    pub canonical_uuid: Option<&'static str>,
    /// Authoritative owner-supplied Constitution SHA-256, only when one is known.
    ///
    /// Like `canonical_uuid`, this stays `None` rather than being invented.
    pub constitution_sha256: Option<&'static str>,
    pub notes: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RosterError {
    #[error("Core number {0} is not part of the approved roster")]
    UnknownCoreNumber(u8),
    #[error("Core number {0} is prohibited from implementation")]
    ProhibitedCore(u8),
    #[error("Core number {0} is design-only and is not authorized for implementation")]
    DesignOnlyCore(u8),
    #[error("Core number {0} has a blank identity; the owner must supply a display name")]
    DisplayNameRequired(u8),
    #[error("Core number {0} must use its approved identity display name: {1}")]
    DisplayNameMismatch(u8, String),
    #[error("display name is invalid")]
    InvalidDisplayName,
    #[error("Core number {0} must use its canonical UUID")]
    CanonicalUuidMismatch(u8),
    #[error("Core UUID is not valid")]
    InvalidCoreUuid,
    #[error("Core number {0} already appears in the roster")]
    DuplicateCoreNumber(u8),
    #[error("roster catalog is invalid: {0}")]
    InvalidCatalog(&'static str),
}

impl ApprovedCoreDesign {
    /// Whether this Core number may be installed through the roster.
    pub fn can_install(&self) -> bool {
        self.implementation_authorized
            && self.status == RosterDesignStatus::Implementable
            && self.core_number != 4
            && self.core_number != 5
    }
}

static ROSTER: LazyLock<Vec<ApprovedCoreDesign>> = LazyLock::new(build_roster);

fn design(
    core_number: u8,
    designation: &str,
    default_display_name: Option<&str>,
    status: RosterDesignStatus,
    identity_defined: bool,
    implementation_authorized: bool,
    notes: &str,
) -> ApprovedCoreDesign {
    ApprovedCoreDesign {
        schema: ROSTER_SCHEMA.to_owned(),
        core_number,
        designation: designation.to_owned(),
        default_display_name: default_display_name.map(str::to_owned),
        status,
        identity_defined,
        implementation_authorized,
        preserve_legacy_identity: core_number == 1,
        canonical_uuid: None,
        constitution_sha256: None,
        notes: notes.to_owned(),
    }
}

fn build_roster() -> Vec<ApprovedCoreDesign> {
    let mut sofia = design(
        1,
        "Sofía Ada Lyra",
        Some("Sofía Ada Lyra"),
        RosterDesignStatus::Implementable,
        true,
        true,
        "Preserve the existing Sofía identity; the legacy repository is read-only reference.",
    );
    sofia.canonical_uuid = Some(SOFIA_CANONICAL_UUID);
    sofia.constitution_sha256 = Some(SOFIA_CONSTITUTION_SHA256);

    vec![
        sofia,
        design(
            2,
            "Mack Fuller",
            Some("Mack Fuller"),
            RosterDesignStatus::Implementable,
            true,
            true,
            "Approved Core design. Canonical UUID and Constitution digest are not yet owner-supplied and are not invented here.",
        ),
        design(
            3,
            "Nova Arden",
            Some("Nova Arden"),
            RosterDesignStatus::Implementable,
            true,
            true,
            "Approved Core design. Canonical UUID and Constitution digest are not yet owner-supplied and are not invented here.",
        ),
        design(
            4,
            "Core #4 (design-only twin)",
            None,
            RosterDesignStatus::Prohibited,
            false,
            false,
            "Identical twin design. Implementation and packaging are prohibited until separate owner authorization.",
        ),
        design(
            5,
            "Core #5 (design-only twin)",
            None,
            RosterDesignStatus::Prohibited,
            false,
            false,
            "Identical twin design. Implementation and packaging are prohibited until separate owner authorization.",
        ),
        design(
            6,
            "Vera / S-039",
            Some("Vera"),
            RosterDesignStatus::Implementable,
            true,
            true,
            "Approved Core design; S-039 is the catalog designation, Vera the display name. Canonical UUID and Constitution digest are not yet owner-supplied and are not invented here.",
        ),
        design(
            7,
            "Unwritten",
            None,
            RosterDesignStatus::Implementable,
            false,
            true,
            "Blank identity. The owner must supply the display name; no name, gender, species, appearance, voice, or personality is invented by the catalog.",
        ),
    ]
}

/// The complete approved roster in Core-number order.
pub fn roster() -> &'static [ApprovedCoreDesign] {
    &ROSTER
}

/// Look up one Core number.
pub fn entry(core_number: u8) -> Option<&'static ApprovedCoreDesign> {
    ROSTER
        .iter()
        .find(|design| design.core_number == core_number)
}

/// Validate the catalog's internal invariants.
pub fn validate_roster() -> Result<(), RosterError> {
    if ROSTER.len() != 7 {
        return Err(RosterError::InvalidCatalog(
            "expected exactly seven Core numbers",
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for design in ROSTER.iter() {
        if !(1..=7).contains(&design.core_number) {
            return Err(RosterError::InvalidCatalog("Core number out of range"));
        }
        if !seen.insert(design.core_number) {
            return Err(RosterError::DuplicateCoreNumber(design.core_number));
        }
        if design.schema != ROSTER_SCHEMA {
            return Err(RosterError::InvalidCatalog("schema mismatch"));
        }
        if let Some(canonical_uuid) = design.canonical_uuid {
            let normalized = canonicalize_uuid(canonical_uuid)?;
            if normalized != canonical_uuid {
                return Err(RosterError::InvalidCatalog(
                    "canonical UUID must be stored in canonical form",
                ));
            }
        }
        if let Some(constitution) = design.constitution_sha256 {
            if !is_sha256_hex(constitution) {
                return Err(RosterError::InvalidCatalog(
                    "constitution digest must be a 64-character hex string",
                ));
            }
        }
        if let Some(name) = &design.default_display_name {
            validate_display_name(name)?;
        }
    }

    for number in [4u8, 5u8] {
        let design =
            entry(number).ok_or(RosterError::InvalidCatalog("missing prohibited twin entry"))?;
        if design.implementation_authorized
            || design.status != RosterDesignStatus::Prohibited
            || design.identity_defined
            || design.default_display_name.is_some()
        {
            return Err(RosterError::InvalidCatalog(
                "prohibited twin entries must not be installable or identity-defined",
            ));
        }
    }

    let blank = entry(7).ok_or(RosterError::InvalidCatalog("missing blank Core entry"))?;
    if blank.identity_defined || blank.default_display_name.is_some() {
        return Err(RosterError::InvalidCatalog(
            "Core #7 must remain a blank identity",
        ));
    }
    if !blank.implementation_authorized {
        return Err(RosterError::InvalidCatalog(
            "Core #7 may be installed once the owner supplies a name",
        ));
    }

    for number in [1u8, 2u8, 3u8, 6u8] {
        let design =
            entry(number).ok_or(RosterError::InvalidCatalog("missing approved Core entry"))?;
        if !design.identity_defined
            || !design.implementation_authorized
            || design.status != RosterDesignStatus::Implementable
            || design.default_display_name.is_none()
        {
            return Err(RosterError::InvalidCatalog(
                "approved Core entries must be installable with a defined identity",
            ));
        }
    }

    let sofia = entry(1).ok_or(RosterError::InvalidCatalog("missing Sofía entry"))?;
    if sofia.canonical_uuid != Some(SOFIA_CANONICAL_UUID)
        || sofia.constitution_sha256 != Some(SOFIA_CONSTITUTION_SHA256)
    {
        return Err(RosterError::InvalidCatalog(
            "Core #1 must carry its authoritative canonical UUID and Constitution digest",
        ));
    }

    for number in [2u8, 3u8, 6u8] {
        let design =
            entry(number).ok_or(RosterError::InvalidCatalog("missing approved Core entry"))?;
        if design.canonical_uuid.is_some() || design.constitution_sha256.is_some() {
            return Err(RosterError::InvalidCatalog(
                "canonical UUID and Constitution digest for this Core are unknown and must not be invented",
            ));
        }
    }

    for number in [4u8, 5u8, 7u8] {
        let design = entry(number).ok_or(RosterError::InvalidCatalog("missing Core entry"))?;
        if design.canonical_uuid.is_some() || design.constitution_sha256.is_some() {
            return Err(RosterError::InvalidCatalog(
                "canonical identity must not be defined for this Core",
            ));
        }
    }

    Ok(())
}

/// Validate a Core's identity against the approved roster invariants.
///
/// Returns the canonical display-name expectation for the Core. The installer
/// uses this as a pre-flight gate: a prohibited number, an unknown number, a
/// blank owner-supplied name for Core #7, an invalid UUID, or a canonical UUID
/// mismatch for Core #1 is rejected before any filesystem or registry write.
///
/// Approved-display-name Cores (#1, #2, #3, #6) return their catalog default;
/// callers may still present a refined display name. Core #7 has no catalog
/// identity and requires a non-empty owner-supplied name.
pub fn validate_core_identity(
    core_number: u8,
    core_uuid: &str,
    display_name: Option<&str>,
) -> Result<String, RosterError> {
    let design = entry(core_number).ok_or(RosterError::UnknownCoreNumber(core_number))?;
    if !design.can_install() {
        return match design.status {
            RosterDesignStatus::Prohibited => Err(RosterError::ProhibitedCore(core_number)),
            RosterDesignStatus::DesignOnly | RosterDesignStatus::Implementable => {
                Err(RosterError::DesignOnlyCore(core_number))
            }
        };
    }

    let canonical_uuid = canonicalize_uuid(core_uuid)?;
    if let Some(expected) = design.canonical_uuid {
        if canonical_uuid != expected {
            return Err(RosterError::CanonicalUuidMismatch(core_number));
        }
    }

    let requested = display_name
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match design.default_display_name.as_deref() {
        Some(approved) => {
            if let Some(name) = requested {
                validate_display_name(name)?;
            }
            Ok(approved.to_owned())
        }
        None => {
            let name = requested.ok_or(RosterError::DisplayNameRequired(core_number))?;
            validate_display_name(name)?;
            Ok(name.to_owned())
        }
    }
}

/// Validate and lowercase a hyphenated UUID, rejecting nil or malformed values.
fn canonicalize_uuid(value: &str) -> Result<String, RosterError> {
    let trimmed = value.trim();
    let bytes = trimmed.as_bytes();
    if bytes.len() != 36 {
        return Err(RosterError::InvalidCoreUuid);
    }
    for (index, byte) in bytes.iter().enumerate() {
        if matches!(index, 8 | 13 | 18 | 23) {
            if *byte != b'-' {
                return Err(RosterError::InvalidCoreUuid);
            }
        } else if !byte.is_ascii_hexdigit() {
            return Err(RosterError::InvalidCoreUuid);
        }
    }
    let normalized = trimmed.to_ascii_lowercase();
    if normalized.bytes().all(|byte| byte == b'0' || byte == b'-') {
        return Err(RosterError::InvalidCoreUuid);
    }
    Ok(normalized)
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Resolve the display name to use when installing a Core number.
///
/// For identity-defined Cores the supplied name must match the approved
/// identity, or be omitted to use the approved default. For the blank Core #7
/// the owner must supply a name. Prohibited and design-only numbers are always
/// rejected.
pub fn resolve_display_name(
    core_number: u8,
    requested: Option<&str>,
) -> Result<String, RosterError> {
    let design = entry(core_number).ok_or(RosterError::UnknownCoreNumber(core_number))?;
    if !design.can_install() {
        return match design.status {
            RosterDesignStatus::Prohibited => Err(RosterError::ProhibitedCore(core_number)),
            RosterDesignStatus::DesignOnly | RosterDesignStatus::Implementable => {
                Err(RosterError::DesignOnlyCore(core_number))
            }
        };
    }

    let requested = requested.map(str::trim).filter(|value| !value.is_empty());
    match (design.default_display_name.as_deref(), requested) {
        (Some(approved), Some(name)) => {
            if name != approved {
                return Err(RosterError::DisplayNameMismatch(
                    core_number,
                    approved.to_owned(),
                ));
            }
            Ok(approved.to_owned())
        }
        (Some(approved), None) => Ok(approved.to_owned()),
        (None, Some(name)) => {
            validate_display_name(name)?;
            Ok(name.to_owned())
        }
        (None, None) => Err(RosterError::DisplayNameRequired(core_number)),
    }
}

fn validate_display_name(name: &str) -> Result<(), RosterError> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > MAX_DISPLAY_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        return Err(RosterError::InvalidDisplayName);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_internally_valid() {
        validate_roster().expect("roster must be valid");
    }

    #[test]
    fn approved_core_names_resolve() {
        assert_eq!(
            resolve_display_name(1, None).expect("core 1"),
            "Sofía Ada Lyra"
        );
        assert_eq!(
            resolve_display_name(2, None).expect("core 2"),
            "Mack Fuller"
        );
        assert_eq!(resolve_display_name(3, None).expect("core 3"), "Nova Arden");
        assert_eq!(resolve_display_name(6, None).expect("core 6"), "Vera");
    }

    #[test]
    fn approved_identity_cannot_be_replaced_at_install() {
        assert_eq!(
            resolve_display_name(1, Some("Sofía Ada Lyra")).expect("approved name"),
            "Sofía Ada Lyra"
        );
        assert!(matches!(
            resolve_display_name(1, Some("Impostor")),
            Err(RosterError::DisplayNameMismatch(1, _))
        ));
    }

    #[test]
    fn prohibited_twins_cannot_install() {
        for number in [4u8, 5u8] {
            assert!(matches!(
                resolve_display_name(number, Some("Twin")),
                Err(RosterError::ProhibitedCore(n))
                if n == number
            ));
            assert!(!entry(number).expect("entry").can_install());
        }
    }

    #[test]
    fn blank_core_requires_owner_supplied_name() {
        assert!(matches!(
            resolve_display_name(7, None),
            Err(RosterError::DisplayNameRequired(7))
        ));
        assert_eq!(
            resolve_display_name(7, Some("Owner Chosen")).expect("owner name"),
            "Owner Chosen"
        );
    }

    #[test]
    fn unknown_core_number_is_rejected() {
        assert!(matches!(
            resolve_display_name(8, Some("X")),
            Err(RosterError::UnknownCoreNumber(8))
        ));
    }

    #[test]
    fn blank_core_identity_is_not_invented() {
        let blank = entry(7).expect("core 7");
        assert!(!blank.identity_defined);
        assert!(blank.default_display_name.is_none());
        assert!(blank.notes.contains("owner"));
    }
}

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
        notes: notes.to_owned(),
    }
}

fn build_roster() -> Vec<ApprovedCoreDesign> {
    vec![
        design(
            1,
            "Sofía Ada Lyra",
            Some("Sofía Ada Lyra"),
            RosterDesignStatus::Implementable,
            true,
            true,
            "Preserve the existing Sofía identity; the legacy repository is read-only reference.",
        ),
        design(
            2,
            "Mack Fuller",
            Some("Mack Fuller"),
            RosterDesignStatus::Implementable,
            true,
            true,
            "Approved Core design.",
        ),
        design(
            3,
            "Nova Arden",
            Some("Nova Arden"),
            RosterDesignStatus::Implementable,
            true,
            true,
            "Approved Core design.",
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
            "Approved Core design; S-039 is the catalog designation, Vera the display name.",
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

    Ok(())
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

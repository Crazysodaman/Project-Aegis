//! Core lifecycle and roster policy contracts.
//!
//! These are policy constants, not Core instances. They do not grant authority.

use serde::{Deserialize, Serialize};

pub const APPROVED_CORE_NUMBERS: [u8; 5] = [1, 2, 3, 6, 7];
pub const PROHIBITED_CORE_NUMBERS: [u8; 2] = [4, 5];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreLifecycleState {
    DesignOnly,
    Approved,
    Installed,
    Active,
    Suspended,
    Retired,
}

impl CoreLifecycleState {
    pub const ALL: [Self; 6] = [
        Self::DesignOnly,
        Self::Approved,
        Self::Installed,
        Self::Active,
        Self::Suspended,
        Self::Retired,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DesignOnly => "design_only",
            Self::Approved => "approved",
            Self::Installed => "installed",
            Self::Active => "active",
            Self::Suspended => "suspended",
            Self::Retired => "retired",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "design_only" => Some(Self::DesignOnly),
            "approved" => Some(Self::Approved),
            "installed" => Some(Self::Installed),
            "active" => Some(Self::Active),
            "suspended" => Some(Self::Suspended),
            "retired" => Some(Self::Retired),
            _ => None,
        }
    }
}

pub fn is_approved_core_number(value: u8) -> bool {
    APPROVED_CORE_NUMBERS.contains(&value)
}

pub fn is_prohibited_core_number(value: u8) -> bool {
    PROHIBITED_CORE_NUMBERS.contains(&value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roster_policy_is_explicit() {
        assert!(is_approved_core_number(1));
        assert!(is_approved_core_number(2));
        assert!(is_approved_core_number(3));
        assert!(is_approved_core_number(6));
        assert!(is_approved_core_number(7));
        assert!(is_prohibited_core_number(4));
        assert!(is_prohibited_core_number(5));
        assert!(!is_approved_core_number(4));
        assert!(!is_approved_core_number(5));
    }

    #[test]
    fn lifecycle_vocabulary_is_stable() {
        for state in CoreLifecycleState::ALL {
            assert_eq!(
                CoreLifecycleState::parse(state.as_str()),
                Some(state),
                "lifecycle state must round-trip"
            );
        }
    }
}

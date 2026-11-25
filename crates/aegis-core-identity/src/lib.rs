//! Core identity and initialization state (Batch 2 / Wave 1b).
//!
//! This crate owns immutable Core identity validation, deterministic fresh
//! initialization stage progression, and standalone checkpoint persistence.
//! It deliberately does not touch SQLite or filesystem database storage.

pub mod checkpoint;
pub mod error;
pub mod identity;
pub mod state_machine;

pub use checkpoint::{read_checkpoint, recover_decision, write_checkpoint, RecoveryDecision};
pub use error::IdentityError;
pub use identity::{rename_core_identity, validate_initialization_request, CoreIdentityService};
pub use state_machine::InitializationStateMachine;

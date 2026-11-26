//! Shared, authority-free contracts for the Aegis greenfield implementation.

pub mod core;
pub mod error;
pub mod initialization;
pub mod integrity;
pub mod ipc;
pub mod lifecycle;
pub mod personal;
pub mod roster;
pub mod status;
pub mod world;

pub use error::ContractError;

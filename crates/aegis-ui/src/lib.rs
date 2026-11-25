//! Aegis local dashboards.
//!
//! This crate provides two read/mutate surfaces over the real Platform APIs:
//!
//! - a loopback-only HTTP dashboard (`DashboardServer`, `bind`,
//!   `DashboardHandle`) with a self-contained page and JSON endpoints;
//! - an ANSI terminal snapshot dashboard (`render_dashboard`, `run_tui`,
//!   `run_tui_once`).
//!
//! Neither surface ever returns the owner token. Mutations require the local
//! session token embedded in the page and are disabled unless explicitly
//! enabled in [`DashboardOptions`].

mod actions;
mod error;
mod gui;
mod html;
mod model;
mod tui;

pub use error::UiError;
pub use gui::{bind, DashboardHandle, DashboardOptions, DashboardServer};
pub use tui::{render_dashboard, run_tui, run_tui_once};

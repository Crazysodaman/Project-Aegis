//! Aegis local dashboards.
//!
//! This crate provides two read/mutate surfaces over the real Platform APIs:
//!
//! - a loopback-only HTTP dashboard (`DashboardServer`, `bind`,
//!   `DashboardHandle`) with a self-contained page and JSON endpoints;
//! - an ANSI terminal snapshot dashboard (`render_dashboard`, `run_tui`,
//!   `run_tui_once`) plus a machine-readable headless summary
//!   (`render_summary_json`).
//!
//! Fleet, GPU, inference, and service sections are populated only from optional
//! operator/CLI-written JSON snapshots under `<root>/run/status/`. A snapshot is
//! used only when it is fresh (≤60s by `updated_at` or file mtime); otherwise the
//! section is reported as `STALE`/`UNAVAILABLE` with an explicit reason. See
//! `docs/implementation/BATCH7_AGENT05_TUI.md` for the schemas.
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
pub use model::render_summary_json;
pub use tui::{render_dashboard, run_tui, run_tui_once};

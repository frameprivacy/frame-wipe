#![allow(
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::needless_return,
    clippy::needless_borrow,
    clippy::collapsible_if,
    clippy::manual_clamp,
    clippy::should_implement_trait,
    clippy::len_without_is_empty,
    clippy::bool_comparison,
    clippy::lines_filter_map_ok
)]

//! Core library for framewipe: plan and overwrite storage on Linux, macOS, and Windows.
//!
//! All destructive work goes through [`planner::plan`] and the safety guards in [`safety`].
//! I/O paths return [`error::Error`] and do not panic on expected failure.

pub mod confirm;
pub mod device;
pub mod engine;
pub mod error;
pub mod files;
pub mod free_space;
pub mod i18n;
pub mod identity;
pub mod journal;
pub mod method;
pub mod open;
pub mod planner;
pub mod progress;
pub mod report;
pub mod safety;
pub mod sanitize;
pub mod units;

pub use engine::{wipe, WipeRequest};
pub use error::{Error, Guard};
pub use method::Method;
pub use planner::{plan, Plan, Scope, TargetSnapshot};
pub use report::{Outcome, Report};
pub use safety::SafetyFlags;

/// Library and CLI version (same as CARGO_PKG_VERSION).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

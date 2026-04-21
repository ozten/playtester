//! Metric registry + ingestion spine for Phase 1.
//!
//! A [`MetricRegistry`] is the seam between a game (or the harness)
//! and the analytics pipeline: the game declares which metrics exist
//! ([`MetricDef`]) and how to compute them from a [`GameLog`]; the
//! harness knows how to persist the resulting [`MetricValue`]s.
//!
//! The registry is intentionally small (two methods). SQLite
//! ingestion, the `report` subcommand, and Cribbage-specific metrics
//! arrive in later Phase 1 units.

pub mod builtin;
pub mod definition;
pub mod log;
pub mod registry;
pub mod value;

pub use builtin::BuiltInMetrics;
pub use definition::{MetricDef, MetricKind, MetricScope};
pub use log::{GameLog, LoadError};
pub use registry::{
    MetricRegistry, RegistryError, ScopeEvidence, validate_definitions,
    validate_values_against_defs,
};
pub use value::{MetricValue, MetricValueKind};

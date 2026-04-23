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
pub mod ingest;
pub mod log;
pub mod markdown;
pub mod query;
pub mod registry;
pub mod reporter;
pub mod stats;
pub mod value;

pub use stats::{
    StatsError, TestOutcome, benjamini_hochberg, bonferroni, standard_normal_cdf,
    two_proportion_z_test, two_sided_p_from_z, welch_t_test,
};

pub use builtin::BuiltInMetrics;
pub use definition::{MetricDef, MetricKind, MetricScope};
pub use ingest::{FileError, IngestError, IngestReport, ingest_directory, init_schema};
pub use log::{GameLog, LoadError};
pub use markdown::MarkdownBuilder;
pub use query::{
    AgentSummary, EndReasonBreakdown, WinnerBreakdown, avg_numeric_metric, games_count,
    sum_count_metric,
};
pub use registry::{
    MetricRegistry, RegistryError, ScopeEvidence, validate_definitions,
    validate_values_against_defs,
};
pub use reporter::{
    write_per_agent_section, write_subjective_critique_section, write_summary_section,
};
pub use value::{MetricValue, MetricValueKind};

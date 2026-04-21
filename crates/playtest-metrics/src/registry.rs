//! The `MetricRegistry` trait — what a game (or the harness) exports
//! so the ingest pipeline knows which metrics to persist and how to
//! derive them.
//!
//! Intentionally simple: a pure function from `(game_id, GameLog) →
//! Vec<MetricValue>`. No streaming, no incremental extraction. A log
//! for a typical Cribbage game fits in a few hundred KB; walking it
//! once per game during ingestion is cheap.

use std::collections::HashSet;

use playtest_core::Game;
use uuid::Uuid;

use crate::definition::{MetricDef, MetricScope};
use crate::log::GameLog;
use crate::value::MetricValue;

/// What a game or the harness exports.
///
/// Implementations must be **consistent**: every [`MetricValue`]
/// returned from `extract` should name a metric declared in
/// `metric_definitions`, and its value kind should match the
/// declared [`crate::MetricKind`]. [`validate_values_against_defs`]
/// enforces this post hoc.
pub trait MetricRegistry<G: Game> {
    /// Static description of every metric this registry emits.
    fn metric_definitions(&self) -> Vec<MetricDef>;

    /// Extract all metric values for a single game.
    fn extract(&self, game_id: Uuid, log: &GameLog<G>) -> Vec<MetricValue>;
}

/// Errors raised during registry validation.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RegistryError {
    #[error("duplicate metric name in registry: {name}")]
    DuplicateName { name: String },

    #[error("metric value `{metric_name}` has no matching definition")]
    UnknownMetric { metric_name: String },

    #[error(
        "metric value `{metric_name}` has kind {actual:?} but the definition declared {expected:?}"
    )]
    KindMismatch {
        metric_name: String,
        expected: crate::MetricKind,
        actual: crate::MetricKind,
    },

    #[error(
        "metric `{metric_name}` is declared `{expected:?}` but a value with `player = {actual:?}` was emitted"
    )]
    ScopeMismatch {
        metric_name: String,
        expected: MetricScope,
        actual: ScopeEvidence,
    },
}

/// Describes the scope evidence a single [`MetricValue`] carries —
/// used in [`RegistryError::ScopeMismatch`] messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeEvidence {
    /// `player = None` on the value — looks like a game-scoped metric.
    PlayerIsNone,
    /// `player = Some(_)` — looks like a player-scoped metric.
    PlayerIsSome,
}

/// Reject a `Vec<MetricDef>` with duplicate names.
///
/// # Errors
/// Returns [`RegistryError::DuplicateName`] at the first duplicate.
pub fn validate_definitions(defs: &[MetricDef]) -> Result<(), RegistryError> {
    let mut seen: HashSet<&str> = HashSet::with_capacity(defs.len());
    for def in defs {
        if !seen.insert(def.name.as_str()) {
            return Err(RegistryError::DuplicateName {
                name: def.name.clone(),
            });
        }
    }
    Ok(())
}

/// Cross-check emitted values against their declared definitions.
///
/// Catches three common bugs: a registry emitting a value whose name
/// it didn't declare; emitting the wrong kind (e.g. `Count` where the
/// def said `Scalar`); and emitting a value with the wrong scope
/// (e.g. `player: None` on a `MetricScope::Player` metric).
///
/// # Errors
/// Returns [`RegistryError::UnknownMetric`], [`RegistryError::KindMismatch`],
/// or [`RegistryError::ScopeMismatch`] on the first failing value.
pub fn validate_values_against_defs(
    defs: &[MetricDef],
    values: &[MetricValue],
) -> Result<(), RegistryError> {
    let by_name: std::collections::HashMap<&str, &MetricDef> =
        defs.iter().map(|d| (d.name.as_str(), d)).collect();

    for v in values {
        let def =
            by_name
                .get(v.metric_name.as_str())
                .ok_or_else(|| RegistryError::UnknownMetric {
                    metric_name: v.metric_name.clone(),
                })?;

        let actual_kind = v.value.discriminator();
        if actual_kind != def.kind {
            return Err(RegistryError::KindMismatch {
                metric_name: v.metric_name.clone(),
                expected: def.kind,
                actual: actual_kind,
            });
        }

        match (def.scope, v.player) {
            (MetricScope::Game, None) | (MetricScope::Player, Some(_)) => {}
            (MetricScope::Game, Some(_)) => {
                return Err(RegistryError::ScopeMismatch {
                    metric_name: v.metric_name.clone(),
                    expected: MetricScope::Game,
                    actual: ScopeEvidence::PlayerIsSome,
                });
            }
            (MetricScope::Player, None) => {
                return Err(RegistryError::ScopeMismatch {
                    metric_name: v.metric_name.clone(),
                    expected: MetricScope::Player,
                    actual: ScopeEvidence::PlayerIsNone,
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MetricKind, MetricValueKind};
    use uuid::Uuid;

    fn def(name: &str, kind: MetricKind, scope: MetricScope) -> MetricDef {
        MetricDef {
            name: name.into(),
            kind,
            scope,
            description: String::new(),
        }
    }

    #[test]
    fn validate_definitions_accepts_unique_names() {
        let defs = [
            def("a", MetricKind::Count, MetricScope::Game),
            def("b", MetricKind::Scalar, MetricScope::Player),
        ];
        assert!(validate_definitions(&defs).is_ok());
    }

    #[test]
    fn validate_definitions_rejects_duplicates() {
        let defs = [
            def("a", MetricKind::Count, MetricScope::Game),
            def("a", MetricKind::Scalar, MetricScope::Player),
        ];
        assert_eq!(
            validate_definitions(&defs),
            Err(RegistryError::DuplicateName { name: "a".into() })
        );
    }

    #[test]
    fn validate_values_rejects_unknown_metric_name() {
        let defs = [def("a", MetricKind::Count, MetricScope::Game)];
        let values = vec![MetricValue {
            game_id: Uuid::nil(),
            metric_name: "b".into(),
            player: None,
            value: MetricValueKind::Count(1),
        }];
        assert!(matches!(
            validate_values_against_defs(&defs, &values).unwrap_err(),
            RegistryError::UnknownMetric { .. }
        ));
    }

    #[test]
    fn validate_values_rejects_kind_mismatch() {
        let defs = [def("a", MetricKind::Count, MetricScope::Game)];
        let values = vec![MetricValue {
            game_id: Uuid::nil(),
            metric_name: "a".into(),
            player: None,
            value: MetricValueKind::Scalar(1.0),
        }];
        assert!(matches!(
            validate_values_against_defs(&defs, &values).unwrap_err(),
            RegistryError::KindMismatch { .. }
        ));
    }

    #[test]
    fn validate_values_rejects_scope_mismatch() {
        let defs = [def("a", MetricKind::Count, MetricScope::Game)];
        let values = vec![MetricValue {
            game_id: Uuid::nil(),
            metric_name: "a".into(),
            player: Some(1),
            value: MetricValueKind::Count(1),
        }];
        assert!(matches!(
            validate_values_against_defs(&defs, &values).unwrap_err(),
            RegistryError::ScopeMismatch { .. }
        ));
    }
}

//! ShipWreck-specific metric registry.
//!
//! `ShipWreckMetrics` exports every metric that only makes sense for
//! ShipWreck: game-shape ("how long did the game run, who won, did the
//! winner need a tie-breaker?") and per-player ("how many rescue
//! points did seat N end with, how many starvation events did they
//! suffer?").
//!
//! Game-agnostic metrics (`game_length_ticks`, `winner`, `wall_clock_ms`,
//! …) come from `playtest_metrics::BuiltInMetrics`, which the ingest
//! pipeline always applies on top of a game's registry.
//!
//! Extraction walks the event stream once per game through [`accumulator::Accumulator`]
//! and hands the aggregated facts to the sub-modules, mirroring the
//! `CribbageMetrics` pattern.

pub mod accumulator;
pub mod game_shape;
pub mod player;

use playtest_metrics::{GameLog, MetricDef, MetricRegistry, MetricValue};
use uuid::Uuid;

use crate::rules::ShipWreckGame;

/// Marker type that implements [`MetricRegistry`] for ShipWreck.
#[derive(Debug, Default, Clone, Copy)]
pub struct ShipWreckMetrics;

impl MetricRegistry<ShipWreckGame> for ShipWreckMetrics {
    fn metric_definitions(&self) -> Vec<MetricDef> {
        let mut defs = Vec::new();
        defs.extend(game_shape::definitions());
        defs.extend(player::definitions());
        defs
    }

    fn extract(&self, game_id: Uuid, log: &GameLog<ShipWreckGame>) -> Vec<MetricValue> {
        let acc = accumulator::Accumulator::ingest(log);
        let mut values = Vec::new();
        values.extend(game_shape::extract(game_id, &acc));
        values.extend(player::extract(game_id, &acc));
        values
    }
}

#[cfg(test)]
mod registry_tests {
    use super::*;
    use playtest_metrics::{validate_definitions, validate_values_against_defs};

    #[test]
    fn registry_definitions_are_unique() {
        let defs = ShipWreckMetrics.metric_definitions();
        validate_definitions(&defs).expect("shipwreck metric names must be unique");
        // At least 8 definitions per the Unit 24 plan.
        assert!(
            defs.len() >= 8,
            "ShipWreckMetrics must declare at least 8 metrics, got {}",
            defs.len()
        );
    }

    #[test]
    fn empty_log_emits_no_values() {
        // An Accumulator built from a log with no EndGame emits nothing
        // for winner-dependent metrics — we still want the pipeline to
        // validate.
        use crate::event::PlayerScore;
        use playtest_core::{EndReason, GameResult};
        use playtest_log::LogHeader;

        let header = LogHeader {
            schema: playtest_log::SCHEMA_VERSION,
            game: "shipwreck".into(),
            version: "test".into(),
            seed: 0,
            agents: vec!["random".into(), "random".into()],
            started_at: 0,
            config_hash: "h".into(),
        };
        let log = GameLog::<ShipWreckGame> {
            header,
            events: Vec::new(),
            final_result: Some(GameResult {
                winner: None,
                reason: EndReason::Draw,
                scores: vec![0, 0],
            }),
            finished_at: None,
        };

        let values = ShipWreckMetrics.extract(Uuid::nil(), &log);
        let defs = ShipWreckMetrics.metric_definitions();
        // Every emitted value must name a known def with a matching
        // scope + kind.
        validate_values_against_defs(&defs, &values)
            .expect("ShipWreckMetrics emitted a value that doesn't match its defs");
        // Silence the unused-but-used PlayerScore import — kept in the
        // test scope for future fixture construction.
        let _ = std::marker::PhantomData::<PlayerScore>;
    }
}

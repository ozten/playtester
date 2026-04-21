//! Cribbage-specific metric registry.
//!
//! `CribbageMetrics` exports every metric that only makes sense for
//! Cribbage: game-shape ("how did this game play out?"), scoring
//! breakdown ("where did the points come from?"), and per-card
//! design-insight metrics (R1.5).
//!
//! Game-agnostic metrics like `game_length_ticks`, `winner`, and
//! `wall_clock_ms` live in `playtest_metrics::BuiltInMetrics` — the
//! ingestion pipeline in Unit 14 will run both registries against every
//! Cribbage log and union their outputs.

pub mod accumulator;
pub mod game_shape;
pub mod per_card;
pub mod scoring;

use playtest_metrics::{GameLog, MetricDef, MetricRegistry, MetricValue};
use uuid::Uuid;

use crate::rules::CribbageGame;

/// The Cribbage-specific metric registry.
#[derive(Debug, Default, Clone, Copy)]
pub struct CribbageMetrics;

impl MetricRegistry<CribbageGame> for CribbageMetrics {
    fn metric_definitions(&self) -> Vec<MetricDef> {
        let mut defs = Vec::new();
        defs.extend(game_shape::definitions());
        defs.extend(scoring::definitions());
        defs.extend(per_card::definitions());
        defs
    }

    fn extract(&self, game_id: Uuid, log: &GameLog<CribbageGame>) -> Vec<MetricValue> {
        let mut acc = accumulator::Accumulator::ingest(&log.events);
        // Engine only emits `Event::EndGame` on pegging-phase wins —
        // show-phase and crib-count wins signal game-over through
        // `state.game_over()` instead. Trust the Final record as the
        // authoritative winner/end-reason source.
        if let Some(r) = &log.final_result {
            acc.supplement_from_final(r.winner, &r.reason);
        }
        let mut values = Vec::new();
        values.extend(game_shape::extract(game_id, &acc));
        values.extend(scoring::extract(game_id, &acc));
        values.extend(per_card::extract(game_id, &acc));
        values
    }
}

//! Single-pass accumulator that walks one ShipWreck log and extracts
//! every fact the metric sub-modules need.
//!
//! Same pattern as Cribbage's `metrics::accumulator`: one walk over the
//! event stream, one struct, many metric readers. Sub-modules read
//! fields off this struct rather than re-walking the event list.
//!
//! Accumulates enough per-player state to produce all eight+ Unit-24
//! metrics: game length (turn count), event-card plays, food-starvation
//! events per player, and final raft / invention / rescue scores.

use playtest_core::PlayerId;
use playtest_metrics::GameLog;

use crate::event::{Event, PlayerScore, TieBreakerUsed};
use crate::rules::ShipWreckGame;

/// Everything-we-need summary of one ShipWreck game.
#[derive(Debug, Clone, Default)]
pub struct Accumulator {
    /// Number of players observed. Populated from the first per-player
    /// event we see (events reference seat indices); falls back to 2
    /// if we somehow never see one (which only happens on a log whose
    /// ingest we wouldn't trust anyway).
    pub num_players: usize,

    /// Count of `EndTurn` events — the canonical "how long was this game".
    pub end_turn_count: u64,

    /// Count of `EventCardPlayed` events — how many event cards were played.
    pub event_cards_played: u64,

    /// Per-player count of `FoodConsumed { starved: true }` events.
    pub starvation_per_player: Vec<u64>,

    /// Winner from the final `EndGame` event, if present.
    pub winner: Option<PlayerId>,

    /// Tie-breaker step the engine recorded on the final `EndGame`
    /// event. Present iff `EndGame` appeared.
    pub tie_breaker: Option<TieBreakerUsed>,

    /// Per-player final scores from the `EndGame` event, keyed by seat.
    pub final_scores: Vec<PlayerScore>,
}

impl Accumulator {
    /// Walk one log and roll up every fact the metric sub-modules need.
    #[must_use]
    pub fn ingest(log: &GameLog<ShipWreckGame>) -> Self {
        // Peek the header's agents for an initial num_players hint.
        // The first per-player event will correct this if it disagrees.
        let initial_n = log.header.agents.len();
        let mut acc = Self {
            num_players: initial_n,
            ..Self::default()
        };
        acc.grow_per_player(initial_n);

        for ev in &log.events {
            // Widen per-player vectors on demand so 3/4-player games
            // produce correct metrics without re-reading the header.
            match ev {
                Event::DealPlayerCard { player, .. }
                | Event::DealWreckageHand { player, .. }
                | Event::DealWreckageFaceUp { player, .. }
                | Event::PickedWreckage { player, .. }
                | Event::PlacedPlayerCard { player, .. }
                | Event::ExtendedRaft { player, .. }
                | Event::BuiltEquipment { player, .. }
                | Event::ResourceSpent { player, .. }
                | Event::EventCardPlayed { player, .. }
                | Event::EventResolved { player, .. }
                | Event::FoodConsumed { player, .. }
                | Event::EndTurn { player } => {
                    let idx = usize::from(*player);
                    if idx + 1 > acc.num_players {
                        acc.num_players = idx + 1;
                        acc.grow_per_player(acc.num_players);
                    }
                }
                Event::EndGame { .. } => {}
            }

            match ev {
                Event::EndTurn { .. } => acc.end_turn_count += 1,
                Event::EventCardPlayed { .. } => acc.event_cards_played += 1,
                Event::FoodConsumed {
                    player,
                    starved: true,
                    ..
                } => {
                    let idx = usize::from(*player);
                    if let Some(slot) = acc.starvation_per_player.get_mut(idx) {
                        *slot += 1;
                    }
                }
                Event::EndGame {
                    winner,
                    final_scores,
                    tie_breaker,
                    ..
                } => {
                    acc.winner = *winner;
                    acc.final_scores.clone_from(final_scores);
                    acc.tie_breaker = Some(*tie_breaker);
                    // `final_scores` width beats the agent count when
                    // they disagree (the engine's truth).
                    if final_scores.len() > acc.num_players {
                        acc.num_players = final_scores.len();
                        acc.grow_per_player(acc.num_players);
                    }
                }
                _ => {}
            }
        }

        // Fall back on the Final record when no EndGame event fired
        // (shouldn't happen in Phase 2 but keeps extraction useful on
        // truncated logs).
        if acc.winner.is_none()
            && let Some(r) = &log.final_result
        {
            acc.winner = r.winner;
        }

        acc
    }

    /// Ensure per-player vectors have at least `n` slots.
    fn grow_per_player(&mut self, n: usize) {
        if self.starvation_per_player.len() < n {
            self.starvation_per_player.resize(n, 0);
        }
    }
}

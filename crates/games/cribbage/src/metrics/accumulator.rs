//! Single-pass accumulator that walks one Cribbage log and extracts
//! every fact the metric modules need.
//!
//! Why a shared accumulator instead of five independent passes: the
//! signals the plan asks for (dealer at each hand, lead changes after
//! every scoring event, who discarded which card to the crib, who held
//! which card in hand) are not independent — each one needs a piece of
//! state that is cheapest to reconstruct alongside the others. One
//! walk, one small struct, many metric extractors reading off it.
//!
//! The accumulator is intentionally pure data: metric modules read it
//! but do not mutate. Emission happens in the metric modules, not here,
//! so `MetricDef` and `MetricValue` stay colocated with the metric's
//! human-readable description.

use playtest_core::{EndReason, PlayerId};

use crate::card::Rank;
use crate::event::Event;
use crate::phase::Phase;

/// Per-hand facts — a Cribbage game is a sequence of hands played until
/// someone crosses 121, so many metrics are computed per hand and then
/// aggregated at the game level.
#[derive(Debug, Clone)]
pub struct HandRecord {
    pub dealer: PlayerId,
    /// Cards dealt to each player in this hand (6 per player).
    pub dealt: [Vec<Rank>; 2],
    /// Cards each player discarded to the crib in this hand (2 per player).
    /// `discards[p]` is player `p`'s discards; the crib always belongs
    /// to that hand's dealer.
    pub discards: [Vec<Rank>; 2],
    /// The 4-card hand each player kept (dealt minus discards).
    /// Populated once both discards are known.
    pub kept: [Vec<Rank>; 2],
    /// Starter rank for this hand, if a cut happened.
    pub starter: Option<Rank>,
    /// Whether the starter was a jack (nibs trigger).
    pub nibs_awarded: bool,
}

impl HandRecord {
    fn new(dealer: PlayerId) -> Self {
        Self {
            dealer,
            dealt: [Vec::with_capacity(6), Vec::with_capacity(6)],
            discards: [Vec::with_capacity(2), Vec::with_capacity(2)],
            kept: [Vec::new(), Vec::new()],
            starter: None,
            nibs_awarded: false,
        }
    }

    fn finalize_kept(&mut self) {
        for p in 0..2 {
            if !self.discards[p].is_empty() && self.kept[p].is_empty() {
                let mut dealt = self.dealt[p].clone();
                for d in &self.discards[p] {
                    if let Some(pos) = dealt.iter().position(|r| r == d) {
                        dealt.swap_remove(pos);
                    }
                }
                self.kept[p] = dealt;
            }
        }
    }
}

/// One moment when the pair of board scores changes — used to count
/// lead changes. `scores[p]` is player `p`'s total score at that moment.
#[derive(Debug, Clone, Copy)]
struct ScoreSample {
    scores: [u16; 2],
}

/// Which player is currently leading (or tied). Lead changes count any
/// transition that flips this value in a non-monotonic way: `Tied ↔ P0`
/// or `Tied ↔ P1` alone do not count, only `P0 → P1` / `P1 → P0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Leader {
    P0,
    P1,
    Tied,
}

impl Leader {
    fn of(sample: ScoreSample) -> Self {
        use core::cmp::Ordering;
        match sample.scores[0].cmp(&sample.scores[1]) {
            Ordering::Greater => Self::P0,
            Ordering::Less => Self::P1,
            Ordering::Equal => Self::Tied,
        }
    }
}

/// The everything-we-need accumulator. Built by walking the event stream
/// once via [`Accumulator::ingest`].
#[derive(Debug, Clone)]
pub struct Accumulator {
    /// One record per hand, in order. A Cribbage game has at least one
    /// hand; long games may have many.
    pub hands: Vec<HandRecord>,
    /// Total show-phase points scored by each player in their own hand.
    pub show_hand_points: [u32; 2],
    /// Total show-phase points scored from the dealer's crib, credited
    /// to whoever was the dealer at that moment.
    pub crib_points: [u32; 2],
    /// Total pegging points each player scored (includes Go/last-card).
    pub pegging_points: [u32; 2],
    /// Total nibs points awarded to the dealer (the only beneficiary).
    pub nibs_points: [u32; 2],
    /// Final board scores as recorded in the event stream (running total
    /// after every scoring event). Falls back to 0 if the log is
    /// truncated before any score event.
    pub final_scores: [u16; 2],
    /// Number of times the leader flipped between P0 and P1 across all
    /// scoring events. Ties in the middle do not count as flips.
    pub lead_changes: u32,
    /// Phase in which the final `EndGame` event was observed, if any.
    /// Used by `game_ended_in_phase`. `None` on a log with no EndGame.
    pub end_phase: Option<Phase>,
    /// Winner from the final EndGame event, if any.
    pub winner: Option<PlayerId>,
    /// End reason attached to the final EndGame event, if any.
    pub end_reason: Option<EndReason>,
    /// Number of decision-style player actions (one per `PegPlayed`,
    /// `DiscardToCrib`, and `Go`). Powers the per-player
    /// `decisions_per_player` metric that the plan promised but
    /// deferred from the game-agnostic built-ins.
    pub decisions: [u32; 2],

    // Working state used while walking the stream. Callers read the
    // finalized `hands`/scores fields above.
    current_phase: Phase,
    current_dealer: PlayerId,
    last_leader: Leader,
}

impl Accumulator {
    /// Walk every event once, folding it into the accumulator.
    #[must_use]
    pub fn ingest(events: &[Event]) -> Self {
        let mut acc = Self {
            hands: vec![HandRecord::new(0)],
            show_hand_points: [0; 2],
            crib_points: [0; 2],
            pegging_points: [0; 2],
            nibs_points: [0; 2],
            final_scores: [0; 2],
            lead_changes: 0,
            end_phase: None,
            winner: None,
            end_reason: None,
            decisions: [0; 2],
            current_phase: Phase::Deal,
            current_dealer: 0,
            last_leader: Leader::Tied,
        };

        for ev in events {
            acc.ingest_one(ev);
        }
        // Make sure the final hand's `kept` is computed even if the log
        // ended mid-hand without a fresh DealCard closing it out.
        if let Some(last) = acc.hands.last_mut() {
            last.finalize_kept();
        }
        acc
    }

    fn current_hand_mut(&mut self) -> &mut HandRecord {
        self.hands
            .last_mut()
            .expect("accumulator always holds at least one hand")
    }

    fn record_score_event(&mut self, player: PlayerId, points: u16) {
        if points == 0 {
            return;
        }
        let idx = usize::from(player);
        if idx >= 2 {
            return;
        }
        self.final_scores[idx] = self.final_scores[idx].saturating_add(points);
        let sample = ScoreSample {
            scores: self.final_scores,
        };
        let now = Leader::of(sample);
        if matches!(
            (self.last_leader, now),
            (Leader::P0, Leader::P1) | (Leader::P1, Leader::P0)
        ) {
            self.lead_changes += 1;
        }
        // Tied is a transient state — do not update `last_leader` to Tied,
        // otherwise a single score that breaks the tie would look like
        // a lead change from Tied, inflating the count.
        if now != Leader::Tied {
            self.last_leader = now;
        }
    }

    fn ingest_one(&mut self, ev: &Event) {
        match ev {
            Event::DealCard { player, card } => {
                self.current_phase = Phase::Deal;
                let p = usize::from(*player);
                if p < 2 {
                    self.current_hand_mut().dealt[p].push(card.rank);
                }
            }
            Event::DiscardToCrib { player, cards } => {
                self.current_phase = Phase::Discard;
                let p = usize::from(*player);
                if p < 2 {
                    let hand = self.current_hand_mut();
                    for c in cards {
                        hand.discards[p].push(c.rank);
                    }
                    // Both players having discarded means we can freeze
                    // the kept-cards snapshot.
                    if !hand.discards[0].is_empty() && !hand.discards[1].is_empty() {
                        hand.finalize_kept();
                    }
                }
                // Every DiscardToCrib is a single agent decision even
                // though it covers two cards in one action.
                if p < 2 {
                    self.decisions[p] += 1;
                }
            }
            Event::CutStarter { card } => {
                self.current_phase = Phase::Cut;
                self.current_hand_mut().starter = Some(card.rank);
            }
            Event::NibsScored { player, points } => {
                self.current_hand_mut().nibs_awarded = true;
                let idx = usize::from(*player);
                if idx < 2 {
                    self.nibs_points[idx] =
                        self.nibs_points[idx].saturating_add(u32::from(*points));
                }
                self.record_score_event(*player, u16::from(*points));
            }
            Event::PegPlayed { player, .. } => {
                self.current_phase = Phase::Pegging;
                let p = usize::from(*player);
                if p < 2 {
                    self.decisions[p] += 1;
                }
            }
            Event::PegScored { player, points, .. } => {
                let idx = usize::from(*player);
                if idx < 2 {
                    self.pegging_points[idx] =
                        self.pegging_points[idx].saturating_add(u32::from(*points));
                }
                self.record_score_event(*player, u16::from(*points));
            }
            Event::Go { player } => {
                let p = usize::from(*player);
                if p < 2 {
                    self.decisions[p] += 1;
                }
            }
            Event::PeggingRoundEnd | Event::PeggingComplete => {}
            Event::ShowScored {
                player,
                is_crib,
                score,
            } => {
                self.current_phase = Phase::Show;
                let idx = usize::from(*player);
                if idx < 2 {
                    if *is_crib {
                        self.crib_points[idx] =
                            self.crib_points[idx].saturating_add(u32::from(score.total));
                    } else {
                        self.show_hand_points[idx] =
                            self.show_hand_points[idx].saturating_add(u32::from(score.total));
                    }
                }
                self.record_score_event(*player, u16::from(score.total));
            }
            Event::HandComplete { next_dealer } => {
                self.current_dealer = *next_dealer;
                self.hands.push(HandRecord::new(*next_dealer));
            }
            Event::EndGame { winner, reason } => {
                self.end_phase = Some(self.current_phase);
                self.winner = Some(*winner);
                self.end_reason = Some(reason.clone());
                self.current_phase = Phase::Finished;
            }
        }
    }
}

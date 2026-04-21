//! Turn-phase enum for a single Cribbage game.
//!
//! Unit 8 implements transitions through `Deal → Discard → Cut →
//! Pegging → Show`. The `Show`, `ScoreCrib`, and `Finished` phases are
//! placeholders that Unit 9 fills in — the enum is defined here in full
//! so event records serialized by Unit 8 never need a schema migration
//! when Unit 9 lands.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Before any cards are dealt.
    Deal,
    /// Each player holds 6 cards and must discard 2 to the crib.
    /// Non-dealer discards first, then dealer.
    Discard,
    /// Crib is full; waiting on the chance cut for the starter.
    Cut,
    /// Pegging phase. Alternates between players (non-dealer first),
    /// with `SayGo` semantics when a player cannot legally play.
    Pegging,
    /// Show phase — non-dealer hand, dealer hand, dealer crib.
    /// Implemented in Unit 9.
    Show,
    /// Crib scoring happens inside `Show`; this variant exists for a
    /// future variant where crib counting is a distinct phase.
    ScoreCrib,
    /// Either player has crossed 121, or the show is complete.
    Finished,
}

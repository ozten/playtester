//! Game-end data.

use serde::{Deserialize, Serialize};

use crate::actor::PlayerId;

/// Why the game ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EndReason {
    /// A player reached the normal victory condition.
    Victory,
    /// All players conceded or the rules produced no legal continuation.
    Draw,
    /// The game ended because a player could not act (stalemate).
    Stalemate,
    /// Game-specific reason not covered above. Games that need richer
    /// taxonomy can encode it in the string; harness code should avoid
    /// pattern-matching on the contents.
    Other(String),
}

/// The result of a completed game. `scores` is game-defined — for
/// Cribbage, it's pips (0–121); for a hypothetical wargame it could be
/// any `i32`. Negative values allowed so future games with penalty
/// mechanics don't need a special case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameResult {
    /// `None` on a draw; `Some(p)` when player `p` won.
    pub winner: Option<PlayerId>,
    pub reason: EndReason,
    /// One entry per player, in player-index order.
    pub scores: Vec<i32>,
}

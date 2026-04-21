//! Players and actors.
//!
//! An [`Actor`] is "whoever is about to act next" — either a specific
//! player, or the game itself resolving a chance event. The engine asks
//! the `Game` trait which kind of actor is up, then routes accordingly.

use serde::{Deserialize, Serialize};

/// Zero-based player index. `u8` is enough for every tabletop game we
/// care about (Cribbage is 2-player; a future deck-building board game
/// will top out at 6 or so). Keeping it small and `Copy` lets it flow
/// through loops and serialized events without ceremony.
pub type PlayerId = u8;

/// Whoever is up next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Actor {
    /// The engine resolves a chance event (deck shuffle, dice roll, card
    /// cut) using the `Rng` port. The `Game` impl tells us the event
    /// shape; the loop pulls randomness through the port.
    Chance,
    /// A specific player is to choose an action.
    Player(PlayerId),
}

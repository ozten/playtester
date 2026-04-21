//! Cribbage: 2-player standard, 6-card deal, 121 points.
//!
//! Full rules — pegging + show + crib + nibs (his heels, +2 to dealer on jack
//! cut) + nobs (+1 for hand jack matching starter suit).
//!
//! This unit lands the standalone primitives (cards, deck, board, hand).
//! The `Game` trait implementation arrives in later units.

pub mod board;
pub mod card;
pub mod deck;
pub mod hand;

pub use board::{Board, NUM_PLAYERS, PlayerPins, WINNING_SCORE};
pub use card::{Card, Rank, Suit};
pub use deck::{DECK_SIZE, fresh, shuffle};
pub use hand::{Hand, HandError};

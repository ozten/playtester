//! Cribbage: 2-player standard, 6-card deal, 121 points.
//!
//! Full rules — pegging + show + crib + nibs (his heels, +2 to dealer on jack
//! cut) + nobs (+1 for hand jack matching starter suit).
//!
//! This unit lands the standalone primitives (cards, deck, board, hand).
//! The `Game` trait implementation arrives in later units.

pub mod action;
pub mod board;
pub mod card;
pub mod deck;
mod determinize;
pub mod event;
pub mod hand;
pub mod heuristic;
pub mod metrics;
pub mod pegging;
pub mod phase;
pub mod report;
pub mod rules;
pub mod scoring;
pub mod state;

pub use action::Action;
pub use board::{Board, NUM_PLAYERS, PlayerPins, WINNING_SCORE};
pub use card::{Card, Rank, Suit};
pub use deck::{DECK_SIZE, fresh, shuffle};
pub use event::Event;
pub use hand::{Hand, HandError};
pub use heuristic::cribbage_eval;
pub use metrics::CribbageMetrics;
pub use pegging::{PegReason, score_peg_play};
pub use phase::Phase;
pub use rules::{CribbageConfig, CribbageGame, PublicView};
pub use scoring::{ShowScore, score_hand};
pub use state::GameState;

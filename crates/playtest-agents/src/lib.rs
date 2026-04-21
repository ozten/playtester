//! Built-in agents: [`RandomAgent`], [`ScriptedAgent`], [`GreedyAgent`],
//! [`HeuristicAgent`].
//!
//! The `Agent` *trait* lives in `playtest-core` (see that crate's
//! lib docs and the architectural invariants memo for why). This crate
//! only provides concrete implementations.
//!
//! Agents choose one action from the engine's enumerated legal actions.
//! They never mutate game state or adjudicate rules — the engine is
//! authoritative.

pub mod eval;
pub mod greedy;
pub mod heuristic;
pub mod random;
pub mod scripted;

pub use eval::EvalFn;
pub use greedy::GreedyAgent;
pub use heuristic::{DEFAULT_TEMPERATURE, HeuristicAgent};
pub use random::RandomAgent;
pub use scripted::ScriptedAgent;

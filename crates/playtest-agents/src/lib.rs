//! Built-in agents: [`RandomAgent`] and [`ScriptedAgent`].
//!
//! The `Agent` *trait* lives in `playtest-core` (see that crate's
//! lib docs and the architectural invariants memo for why). This crate
//! only provides concrete implementations.
//!
//! Agents choose one action from the engine's enumerated legal actions.
//! They never mutate game state or adjudicate rules — the engine is
//! authoritative.

pub mod random;
pub mod scripted;

pub use random::RandomAgent;
pub use scripted::ScriptedAgent;

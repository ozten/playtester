//! Core engine: the game-agnostic `Game` trait, `GameLoop`, and result types.
//!
//! No game-specific code lives here. A `Game` implementation is provided by a
//! game crate (e.g. `playtest-cribbage`) and composed with agents and ports by
//! the harness.

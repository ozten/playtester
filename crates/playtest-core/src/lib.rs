//! Core engine: the game-agnostic `Game` trait, `GameLoop`, `Agent` trait,
//! and result types.
//!
//! No game-specific code lives here. A `Game` implementation is provided
//! by a game crate (e.g. `playtest-cribbage`) and composed with agents
//! and ports by the harness.
//!
//! The `Agent` trait lives here (not in `playtest-agents`) because its
//! signature is intrinsically coupled to `Game::PublicView` /
//! `Game::Action`. Concrete agents (`RandomAgent`, `ScriptedAgent`,
//! future `LlmAgent`) live in `playtest-agents` and depend on this
//! crate for the trait plus `playtest-ports` for any external systems
//! they consume. See the architectural invariants memo for why ports
//! and core abstractions are kept separate.

pub mod actor;
pub mod agent;
pub mod error;
pub mod game;
pub mod game_loop;
pub mod result;

pub use actor::{Actor, PlayerId};
pub use agent::Agent;
pub use error::{AgentError, GameError};
pub use game::Game;
pub use game_loop::GameLoop;
pub use result::{EndReason, GameResult};

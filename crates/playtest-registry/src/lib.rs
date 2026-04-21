//! Shared game + agent registries used by both the `playtest` CLI
//! binary and the `playtest-server` HTTP server.
//!
//! This crate is the **single dispatch point** for games and agents:
//! every subcommand in the CLI and every HTTP route in the server
//! reaches through here. Adding a new game or agent means editing
//! this crate; no consumer hardcodes game-specific names.
//!
//! Extracted from `playtest-cli` so the server can depend on it
//! without creating a package-level cycle with the CLI (the CLI
//! depends on the server for its `serve` subcommand).

pub mod agent_registry;
pub mod game_registry;
pub mod play;

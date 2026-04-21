//! Lookup table from a user-visible game name (e.g. `"cribbage"`) to a
//! constructed game instance.
//!
//! Games have per-game associated types (`State`, `Action`, `Event`,
//! `PublicView`, `Config`), so we can't expose a single `Box<dyn Game>`
//! across games. Instead this module enumerates the known games and
//! the `play` / `replay` commands dispatch on the enum's variant to
//! run game-typed generic code. Adding a new game means:
//!
//! 1. Add a variant to [`RegisteredGame`].
//! 2. Wire it into [`lookup`] and [`KNOWN_GAMES`].
//! 3. Add a matching arm in `commands::play` and `commands::replay`.

use anyhow::{Result, bail};
use playtest_cribbage::CribbageGame;

/// All games the CLI knows how to run. The variant name is the
/// user-visible string passed on the command line.
#[derive(Debug)]
pub enum RegisteredGame {
    Cribbage(CribbageGame),
}

/// The names accepted by [`lookup`], in display order. Kept as a
/// constant so the unknown-game error can list them accurately.
pub const KNOWN_GAMES: &[&str] = &["cribbage"];

/// Look up a game by name.
///
/// # Errors
/// Returns an error listing [`KNOWN_GAMES`] if `name` is not
/// registered.
pub fn lookup(name: &str) -> Result<RegisteredGame> {
    match name {
        "cribbage" => Ok(RegisteredGame::Cribbage(CribbageGame::new())),
        other => bail!("unknown game: {other}; known: {}", KNOWN_GAMES.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cribbage_lookup_succeeds() {
        let g = lookup("cribbage").unwrap();
        assert!(matches!(g, RegisteredGame::Cribbage(_)));
    }

    #[test]
    fn unknown_name_errors_with_known_list() {
        let err = lookup("jalopnik").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("unknown game"), "msg: {msg}");
        assert!(msg.contains("cribbage"), "msg should list cribbage: {msg}");
    }
}

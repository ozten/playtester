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
//! 2. Wire it into [`lookup`], [`KNOWN_GAMES`], and [`player_count_range`].
//! 3. Add a matching arm in `commands::play` and `commands::replay`.

use anyhow::{Result, bail};
use playtest_cribbage::CribbageGame;
use playtest_greatgyre::GreatGyreGame;
use playtest_shipwreck::ShipWreckGame;

/// All games the CLI knows how to run. The variant name is the
/// user-visible string passed on the command line.
#[derive(Debug)]
pub enum RegisteredGame {
    Cribbage(CribbageGame),
    ShipWreck(ShipWreckGame),
    GreatGyre(GreatGyreGame),
}

/// The names accepted by [`lookup`], in display order. Kept as a
/// constant so the unknown-game error can list them accurately.
pub const KNOWN_GAMES: &[&str] = &["cribbage", "shipwreck", "greatgyre"];

/// Inclusive range of legal agent counts for each registered game.
/// Lets the CLI validate `--agents` against per-game rules rather
/// than hardcoding "exactly 2" (which is wrong for ShipWreck's and
/// Great Gyre's 2..=4 player range).
#[must_use]
pub fn player_count_range(game: &RegisteredGame) -> (usize, usize) {
    match game {
        // Cribbage is strictly 2-player per the Phase 0 scope decision.
        RegisteredGame::Cribbage(_) => (2, 2),
        RegisteredGame::ShipWreck(_) => (
            usize::from(playtest_shipwreck::MIN_PLAYERS),
            usize::from(playtest_shipwreck::MAX_PLAYERS),
        ),
        RegisteredGame::GreatGyre(_) => (
            usize::from(playtest_greatgyre::MIN_PLAYERS),
            usize::from(playtest_greatgyre::MAX_PLAYERS),
        ),
    }
}

/// Look up a game by name.
///
/// # Errors
/// Returns an error listing [`KNOWN_GAMES`] if `name` is not
/// registered.
pub fn lookup(name: &str) -> Result<RegisteredGame> {
    match name {
        "cribbage" => Ok(RegisteredGame::Cribbage(CribbageGame::new())),
        "shipwreck" => Ok(RegisteredGame::ShipWreck(ShipWreckGame::new())),
        "greatgyre" => Ok(RegisteredGame::GreatGyre(GreatGyreGame::new())),
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
    fn shipwreck_lookup_succeeds() {
        let g = lookup("shipwreck").unwrap();
        assert!(matches!(g, RegisteredGame::ShipWreck(_)));
    }

    #[test]
    fn greatgyre_lookup_succeeds() {
        let g = lookup("greatgyre").unwrap();
        assert!(matches!(g, RegisteredGame::GreatGyre(_)));
    }

    #[test]
    fn unknown_name_errors_with_known_list() {
        let err = lookup("jalopnik").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("unknown game"), "msg: {msg}");
        assert!(msg.contains("cribbage"), "msg should list cribbage: {msg}");
        assert!(msg.contains("shipwreck"), "msg should list shipwreck: {msg}");
        assert!(msg.contains("greatgyre"), "msg should list greatgyre: {msg}");
    }

    #[test]
    fn player_count_ranges_cover_each_game() {
        assert_eq!(player_count_range(&lookup("cribbage").unwrap()), (2, 2));
        assert_eq!(player_count_range(&lookup("shipwreck").unwrap()), (2, 4));
        assert_eq!(player_count_range(&lookup("greatgyre").unwrap()), (2, 4));
    }
}

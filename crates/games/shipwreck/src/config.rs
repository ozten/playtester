//! Game configuration for ShipWreck.
//!
//! A ShipWreck game is configured by the number of players (2–4 per
//! `docs/shipwreck.md`). Default is 2 players — the smallest legal
//! configuration, used by the bulk of the tests and every
//! random-vs-random shakeout.
//!
//! The config is a plain value; it is passed to the setup flow as
//! `&ShipWreckConfig`, stored on `GameState::config`, and consulted by
//! `legal_actions` / `apply_action` for per-count rules (later units).

use serde::{Deserialize, Serialize};

/// Minimum allowed player count (per `docs/shipwreck.md`).
pub const MIN_PLAYERS: u8 = 2;

/// Maximum allowed player count (per `docs/shipwreck.md`).
pub const MAX_PLAYERS: u8 = 4;

/// Errors returned when constructing a [`ShipWreckConfig`].
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConfigError {
    /// `num_players` is outside the supported 2..=4 range.
    #[error(
        "invalid player count: {got} (supported range is {MIN_PLAYERS}..={MAX_PLAYERS})"
    )]
    InvalidPlayerCount { got: u8 },
}

/// ShipWreck per-game configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShipWreckConfig {
    /// Number of seats in the game. Must satisfy
    /// `MIN_PLAYERS <= num_players <= MAX_PLAYERS`.
    pub num_players: u8,
}

impl ShipWreckConfig {
    /// Construct a config with the given player count.
    ///
    /// # Errors
    /// Returns [`ConfigError::InvalidPlayerCount`] if `num_players`
    /// is outside `MIN_PLAYERS..=MAX_PLAYERS`.
    pub fn new(num_players: u8) -> Result<Self, ConfigError> {
        if !(MIN_PLAYERS..=MAX_PLAYERS).contains(&num_players) {
            return Err(ConfigError::InvalidPlayerCount { got: num_players });
        }
        Ok(Self { num_players })
    }

    /// The default 2-player config. Equivalent to `Default::default()`.
    #[must_use]
    pub const fn with_default_players() -> Self {
        Self {
            num_players: MIN_PLAYERS,
        }
    }
}

impl Default for ShipWreckConfig {
    fn default() -> Self {
        Self::with_default_players()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_two_players() {
        assert_eq!(ShipWreckConfig::default().num_players, 2);
        assert_eq!(ShipWreckConfig::with_default_players().num_players, 2);
    }

    #[test]
    fn new_accepts_two_three_four() {
        for n in [2, 3, 4] {
            let cfg = ShipWreckConfig::new(n).expect("valid count");
            assert_eq!(cfg.num_players, n);
        }
    }

    #[test]
    fn new_rejects_one_and_five() {
        assert_eq!(
            ShipWreckConfig::new(1),
            Err(ConfigError::InvalidPlayerCount { got: 1 })
        );
        assert_eq!(
            ShipWreckConfig::new(5),
            Err(ConfigError::InvalidPlayerCount { got: 5 })
        );
    }

    #[test]
    fn new_rejects_zero() {
        assert_eq!(
            ShipWreckConfig::new(0),
            Err(ConfigError::InvalidPlayerCount { got: 0 })
        );
    }
}

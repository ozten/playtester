//! Game configuration for Great Gyre.
//!
//! Per `docs/greatgyre.md`: "2–4 players **[A** — the rulebook never
//! states a count; component limits support up to 4 comfortably and
//! the harness targets 4**]**." Default is 2 players, matching the
//! rest of the harness's convention (`ShipWreckConfig`, `CribbageConfig`).

use serde::{Deserialize, Serialize};

/// Minimum allowed player count.
pub const MIN_PLAYERS: u8 = 2;

/// Maximum allowed player count.
pub const MAX_PLAYERS: u8 = 4;

/// Errors returned when constructing a [`GreatGyreConfig`].
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("invalid player count: {got} (supported range is {MIN_PLAYERS}..={MAX_PLAYERS})")]
    InvalidPlayerCount { got: u8 },
}

/// Great Gyre per-game configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GreatGyreConfig {
    /// Number of seats. Must satisfy `MIN_PLAYERS <= num_players <= MAX_PLAYERS`.
    #[serde(default = "GreatGyreConfig::default_num_players")]
    pub num_players: u8,
}

impl GreatGyreConfig {
    /// # Errors
    /// Returns [`ConfigError::InvalidPlayerCount`] if `num_players` is
    /// outside `MIN_PLAYERS..=MAX_PLAYERS`.
    pub fn new(num_players: u8) -> Result<Self, ConfigError> {
        if !(MIN_PLAYERS..=MAX_PLAYERS).contains(&num_players) {
            return Err(ConfigError::InvalidPlayerCount { got: num_players });
        }
        Ok(Self { num_players })
    }

    const fn default_num_players() -> u8 {
        MIN_PLAYERS
    }
}

impl Default for GreatGyreConfig {
    fn default() -> Self {
        Self {
            num_players: Self::default_num_players(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_two_players() {
        assert_eq!(GreatGyreConfig::default().num_players, 2);
    }

    #[test]
    fn new_accepts_two_through_four() {
        for n in MIN_PLAYERS..=MAX_PLAYERS {
            assert!(GreatGyreConfig::new(n).is_ok(), "should accept {n}");
        }
    }

    #[test]
    fn new_rejects_out_of_range() {
        assert_eq!(
            GreatGyreConfig::new(1),
            Err(ConfigError::InvalidPlayerCount { got: 1 })
        );
        assert_eq!(
            GreatGyreConfig::new(5),
            Err(ConfigError::InvalidPlayerCount { got: 5 })
        );
    }

    #[test]
    fn serde_round_trip_uses_default_when_missing() {
        let v: GreatGyreConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(v.num_players, 2);
    }
}

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

use crate::card::EventCard;

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
///
/// `events_enabled` (Phase 5) toggles whether event cards (Shark,
/// Typhoon, FlyingFish) are seeded into the wreckage pool during
/// setup. Phase 6 adds per-card toggles (`shark_enabled`,
/// `typhoon_enabled`, `flying_fish_enabled`) that let the R6 restricted-
/// play recipe ablate one card at a time. The two layers compose with
/// AND: a card is seeded iff `events_enabled && <per_card>_enabled`.
///
/// Changing any toggle changes the config hash, so runs with
/// different values land in separate log-aggregate buckets — they're
/// not silently mixed by the reporter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools, reason = "config surface is naturally boolean-heavy; per-card toggles map 1:1 to event-card kinds and the R6 restricted-play recipe expects direct flag access")]
pub struct ShipWreckConfig {
    /// Number of seats in the game. Must satisfy
    /// `MIN_PLAYERS <= num_players <= MAX_PLAYERS`.
    pub num_players: u8,

    /// Master switch for event cards. `false` disables all three
    /// event-card kinds regardless of the per-card flags. Default
    /// `true`.
    #[serde(default = "ShipWreckConfig::default_true")]
    pub events_enabled: bool,

    /// Per-card toggle for Shark. Requires `events_enabled = true` to
    /// take effect. Default `true`.
    #[serde(default = "ShipWreckConfig::default_true")]
    pub shark_enabled: bool,

    /// Per-card toggle for Typhoon. Default `true`.
    #[serde(default = "ShipWreckConfig::default_true")]
    pub typhoon_enabled: bool,

    /// Per-card toggle for FlyingFish. Default `true`.
    #[serde(default = "ShipWreckConfig::default_true")]
    pub flying_fish_enabled: bool,
}

impl ShipWreckConfig {
    /// Construct a config with the given player count. All event
    /// cards are enabled by default.
    ///
    /// # Errors
    /// Returns [`ConfigError::InvalidPlayerCount`] if `num_players`
    /// is outside `MIN_PLAYERS..=MAX_PLAYERS`.
    pub fn new(num_players: u8) -> Result<Self, ConfigError> {
        if !(MIN_PLAYERS..=MAX_PLAYERS).contains(&num_players) {
            return Err(ConfigError::InvalidPlayerCount { got: num_players });
        }
        Ok(Self {
            num_players,
            events_enabled: true,
            shark_enabled: true,
            typhoon_enabled: true,
            flying_fish_enabled: true,
        })
    }

    /// Return a copy of this config with `events_enabled` flipped to
    /// `value`. Used by the R5.9 benchmark to build a paired config.
    #[must_use]
    pub const fn with_events_enabled(mut self, value: bool) -> Self {
        self.events_enabled = value;
        self
    }

    /// Return a copy of this config with the per-card flag for
    /// `kind` flipped to `value`. Composes with AND against
    /// `events_enabled` at setup time.
    #[must_use]
    pub const fn with_event_card(mut self, kind: EventCard, value: bool) -> Self {
        match kind {
            EventCard::Shark => self.shark_enabled = value,
            EventCard::Typhoon => self.typhoon_enabled = value,
            EventCard::FlyingFish => self.flying_fish_enabled = value,
        }
        self
    }

    /// Return true iff a specific event card is effective — both the
    /// master and per-card toggles are `true`.
    #[must_use]
    pub const fn event_card_active(&self, kind: EventCard) -> bool {
        if !self.events_enabled {
            return false;
        }
        match kind {
            EventCard::Shark => self.shark_enabled,
            EventCard::Typhoon => self.typhoon_enabled,
            EventCard::FlyingFish => self.flying_fish_enabled,
        }
    }

    /// The default 2-player config. Equivalent to `Default::default()`.
    #[must_use]
    pub const fn with_default_players() -> Self {
        Self {
            num_players: MIN_PLAYERS,
            events_enabled: true,
            shark_enabled: true,
            typhoon_enabled: true,
            flying_fish_enabled: true,
        }
    }

    const fn default_true() -> bool {
        true
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

    #[test]
    fn default_events_enabled_is_true() {
        assert!(ShipWreckConfig::default().events_enabled);
        assert!(ShipWreckConfig::with_default_players().events_enabled);
        assert!(ShipWreckConfig::new(3).unwrap().events_enabled);
    }

    #[test]
    fn with_events_enabled_round_trips() {
        let off = ShipWreckConfig::new(2).unwrap().with_events_enabled(false);
        assert!(!off.events_enabled);
        let on = off.with_events_enabled(true);
        assert!(on.events_enabled);
    }
}

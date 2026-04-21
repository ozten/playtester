//! Cribbage board: two pins per player, tracks current and previous
//! scores, detects the winning threshold.
//!
//! # The "exactly 121 vs. overshoot" rule
//!
//! Standard Cribbage (ACC, Hoyle) awards the win to the first player
//! whose score **reaches or exceeds 121**. There is no "must peg
//! exactly" requirement; overshooting on a run or a pair is fine and
//! still counts as a win. This module implements that rule.
//!
//! We do not clamp scores at 121 — [`Board::score`] returns the raw
//! value so a game that ends on "peg 6 to reach 125" still shows 125
//! in the result. [`Board::winner`] reports the first player to cross
//! the threshold. A few casual rulesets require exact 121; if that
//! variant ever lands, it belongs behind a config flag, not in this
//! primitive.

use serde::{Deserialize, Serialize};

/// Points required to win. Standard Cribbage: 121.
pub const WINNING_SCORE: u16 = 121;

/// Number of players supported by this primitive. Two-player Cribbage
/// is the full scope of the phases-0-1 plan.
pub const NUM_PLAYERS: usize = 2;

/// Front and back pin positions for one player. Front is the current
/// score; back is what front was before the most recent call to
/// [`Board::advance`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerPins {
    pub front: u16,
    pub back: u16,
}

/// A Cribbage board: two pairs of pins, one per player.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Board {
    pub pins: [PlayerPins; NUM_PLAYERS],
}

impl Board {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pins: [
                PlayerPins { front: 0, back: 0 },
                PlayerPins { front: 0, back: 0 },
            ],
        }
    }

    /// Advance `player`'s pins by `points`. Back pin moves to where the
    /// front pin was, front pin moves forward by `points`.
    ///
    /// Saturates at `u16::MAX` on overflow. No clamping at 121 — see
    /// the module docs.
    ///
    /// # Panics
    /// Panics if `player` is not in `0..NUM_PLAYERS`.
    pub fn advance(&mut self, player: u8, points: u16) {
        let idx = usize::from(player);
        let p = &mut self.pins[idx];
        p.back = p.front;
        p.front = p.front.saturating_add(points);
    }

    /// Current score (front pin) for `player`.
    ///
    /// # Panics
    /// Panics if `player` is not in `0..NUM_PLAYERS`.
    #[must_use]
    pub fn score(&self, player: u8) -> u16 {
        self.pins[usize::from(player)].front
    }

    /// First player (lowest index) whose score has reached or crossed
    /// [`WINNING_SCORE`], if any. Returns `None` while the game is
    /// still in progress.
    #[must_use]
    pub fn winner(&self) -> Option<u8> {
        for (i, pin) in self.pins.iter().enumerate() {
            if pin.front >= WINNING_SCORE {
                return Some(u8::try_from(i).expect("NUM_PLAYERS fits in u8"));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_board_has_all_zero_pins() {
        let b = Board::new();
        assert_eq!(b.score(0), 0);
        assert_eq!(b.score(1), 0);
        assert_eq!(b.winner(), None);
    }

    #[test]
    fn advance_moves_back_pin_to_old_front_then_front_forward() {
        let mut b = Board::new();
        b.advance(0, 5);
        assert_eq!(b.pins[0], PlayerPins { front: 5, back: 0 });
        b.advance(0, 3);
        assert_eq!(b.pins[0], PlayerPins { front: 8, back: 5 });
    }

    #[test]
    fn advance_does_not_touch_other_player() {
        let mut b = Board::new();
        b.advance(0, 10);
        assert_eq!(b.score(0), 10);
        assert_eq!(b.score(1), 0);
    }
}

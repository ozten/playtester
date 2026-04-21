//! Playing-card primitives: [`Suit`], [`Rank`], [`Card`].
//!
//! The single most common Cribbage bug source is conflating two values
//! that look like "the rank":
//!
//! - **pegging value** — used for the `15` and `31` targets during
//!   pegging. Ace counts as 1; 2–10 count as their face; J/Q/K all
//!   count as 10.
//! - **rank ordering** — used for runs, pairs, and card identity. Ace
//!   = 1, 2..10 = 2..10, J/Q/K = 11/12/13.
//!
//! These collide for J/Q/K: pegging value 10, rank ord 11/12/13. We
//! expose them as two separate methods ([`Rank::value`] and
//! [`Rank::rank_ord`]) so callers cannot accidentally use the wrong
//! one. A regression test enforces the non-collision.

use serde::{Deserialize, Serialize};

/// One of the four standard French suits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Suit {
    Clubs,
    Diamonds,
    Hearts,
    Spades,
}

impl Suit {
    /// All four suits in canonical `fresh()` order.
    pub const ALL: [Suit; 4] = [Self::Clubs, Self::Diamonds, Self::Hearts, Self::Spades];

    /// Single-letter symbol used by [`Card`]'s `Display` impl.
    #[must_use]
    pub const fn symbol(self) -> char {
        match self {
            Self::Clubs => 'C',
            Self::Diamonds => 'D',
            Self::Hearts => 'H',
            Self::Spades => 'S',
        }
    }
}

/// One of thirteen ranks. Ace is low (= 1) — Cribbage never uses Ace-high.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(u8)]
pub enum Rank {
    Ace = 1,
    Two = 2,
    Three = 3,
    Four = 4,
    Five = 5,
    Six = 6,
    Seven = 7,
    Eight = 8,
    Nine = 9,
    Ten = 10,
    Jack = 11,
    Queen = 12,
    King = 13,
}

impl Rank {
    /// All thirteen ranks, Ace through King.
    pub const ALL: [Rank; 13] = [
        Self::Ace,
        Self::Two,
        Self::Three,
        Self::Four,
        Self::Five,
        Self::Six,
        Self::Seven,
        Self::Eight,
        Self::Nine,
        Self::Ten,
        Self::Jack,
        Self::Queen,
        Self::King,
    ];

    /// Pegging value: Ace = 1, 2..10 = face, J/Q/K = 10.
    ///
    /// Used for 15/31 targets during the pegging phase. Do **not** use
    /// for run detection — use [`Self::rank_ord`] there.
    #[must_use]
    pub const fn value(self) -> u8 {
        match self {
            Self::Ace => 1,
            Self::Two => 2,
            Self::Three => 3,
            Self::Four => 4,
            Self::Five => 5,
            Self::Six => 6,
            Self::Seven => 7,
            Self::Eight => 8,
            Self::Nine => 9,
            Self::Ten | Self::Jack | Self::Queen | Self::King => 10,
        }
    }

    /// Rank ordering: Ace = 1, 2..10 = face, J = 11, Q = 12, K = 13.
    ///
    /// Used for runs, pairs, and card identity. Distinct from
    /// [`Self::value`] for J/Q/K.
    #[must_use]
    pub const fn rank_ord(self) -> u8 {
        self as u8
    }

    /// Single-character symbol used by [`Card`]'s `Display` impl.
    /// `T` for ten (so every rank fits in one character).
    #[must_use]
    pub const fn symbol(self) -> char {
        match self {
            Self::Ace => 'A',
            Self::Two => '2',
            Self::Three => '3',
            Self::Four => '4',
            Self::Five => '5',
            Self::Six => '6',
            Self::Seven => '7',
            Self::Eight => '8',
            Self::Nine => '9',
            Self::Ten => 'T',
            Self::Jack => 'J',
            Self::Queen => 'Q',
            Self::King => 'K',
        }
    }
}

/// A single playing card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Card {
    pub rank: Rank,
    pub suit: Suit,
}

impl Card {
    #[must_use]
    pub const fn new(rank: Rank, suit: Suit) -> Self {
        Self { rank, suit }
    }

    /// Forward to [`Rank::value`] (pegging value, 1..10).
    #[must_use]
    pub const fn value(self) -> u8 {
        self.rank.value()
    }

    /// Forward to [`Rank::rank_ord`] (run-ordering, 1..13).
    #[must_use]
    pub const fn rank_ord(self) -> u8 {
        self.rank.rank_ord()
    }
}

impl core::fmt::Display for Card {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}{}", self.rank.symbol(), self.suit.symbol())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ace_has_value_one_and_ord_one() {
        assert_eq!(Rank::Ace.value(), 1);
        assert_eq!(Rank::Ace.rank_ord(), 1);
    }

    #[test]
    fn pip_cards_have_matching_value_and_ord() {
        for r in [
            Rank::Two,
            Rank::Three,
            Rank::Four,
            Rank::Five,
            Rank::Six,
            Rank::Seven,
            Rank::Eight,
            Rank::Nine,
            Rank::Ten,
        ] {
            assert_eq!(r.value(), r.rank_ord(), "{r:?}");
        }
    }

    #[test]
    fn face_cards_value_is_ten_but_rank_ord_is_eleven_twelve_thirteen() {
        assert_eq!(Rank::Jack.value(), 10);
        assert_eq!(Rank::Jack.rank_ord(), 11);
        assert_eq!(Rank::Queen.value(), 10);
        assert_eq!(Rank::Queen.rank_ord(), 12);
        assert_eq!(Rank::King.value(), 10);
        assert_eq!(Rank::King.rank_ord(), 13);
    }

    #[test]
    fn card_displays_as_rank_letter_plus_suit_letter() {
        assert_eq!(Card::new(Rank::Ace, Suit::Hearts).to_string(), "AH");
        assert_eq!(Card::new(Rank::Ten, Suit::Diamonds).to_string(), "TD");
        assert_eq!(Card::new(Rank::King, Suit::Spades).to_string(), "KS");
    }
}

//! Show-scoring rule: **nobs**.
//!
//! If the hand contains a Jack whose suit matches the starter's suit,
//! score 1 point. Separate from **nibs** (his heels), which is the
//! dealer's 2-point bonus when the cut starter itself is a Jack — that
//! lives in the pegging-phase logic, not here.

use crate::card::{Card, Rank};

/// Score nobs for `hand` against the cut `starter`.
#[must_use]
pub fn score(hand: [Card; 4], starter: Card) -> u8 {
    for c in hand {
        if c.rank == Rank::Jack && c.suit == starter.suit {
            return 1;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::{Rank, Suit};

    fn c(r: Rank, s: Suit) -> Card {
        Card::new(r, s)
    }

    #[test]
    fn jack_matching_starter_suit_scores_one() {
        let hand = [
            c(Rank::Jack, Suit::Hearts),
            c(Rank::Five, Suit::Clubs),
            c(Rank::Two, Suit::Diamonds),
            c(Rank::King, Suit::Spades),
        ];
        let starter = c(Rank::Eight, Suit::Hearts);
        assert_eq!(score(hand, starter), 1);
    }

    #[test]
    fn jack_not_matching_starter_suit_scores_zero() {
        let hand = [
            c(Rank::Jack, Suit::Spades),
            c(Rank::Five, Suit::Clubs),
            c(Rank::Two, Suit::Diamonds),
            c(Rank::King, Suit::Spades),
        ];
        let starter = c(Rank::Eight, Suit::Hearts);
        assert_eq!(score(hand, starter), 0);
    }

    #[test]
    fn no_jack_in_hand_scores_zero() {
        let hand = [
            c(Rank::Five, Suit::Clubs),
            c(Rank::Six, Suit::Hearts),
            c(Rank::Two, Suit::Diamonds),
            c(Rank::King, Suit::Spades),
        ];
        let starter = c(Rank::Eight, Suit::Hearts);
        assert_eq!(score(hand, starter), 0);
    }

    #[test]
    fn starter_is_a_jack_does_not_count_as_nobs_here() {
        // Starter-is-Jack is nibs (+2 to dealer at cut), not nobs.
        let hand = [
            c(Rank::Two, Suit::Hearts),
            c(Rank::Five, Suit::Clubs),
            c(Rank::Two, Suit::Diamonds),
            c(Rank::King, Suit::Spades),
        ];
        let starter = c(Rank::Jack, Suit::Hearts);
        assert_eq!(score(hand, starter), 0);
    }
}

//! Show-scoring rule: **flush**.
//!
//! *Non-crib hand*: if all 4 hand cards share a suit, score 4 points;
//! if the starter also matches, score 5.
//!
//! *Crib*: only a 5-way flush counts — all 4 crib cards AND the
//! starter must all share a suit. A 4-card flush in the crib scores 0.
//! This is a notorious bug source in novice Cribbage implementations;
//! the asymmetry exists to discourage dumping suited throwaways into
//! an opponent's crib.

use crate::card::Card;

/// Score the flush.
#[must_use]
pub fn score(hand: [Card; 4], starter: Card, is_crib: bool) -> u8 {
    let suit = hand[0].suit;
    let hand_flush = hand.iter().all(|c| c.suit == suit);
    if !hand_flush {
        return 0;
    }
    let starter_matches = starter.suit == suit;
    if is_crib {
        if starter_matches { 5 } else { 0 }
    } else if starter_matches {
        5
    } else {
        4
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::{Rank, Suit};

    fn c(r: Rank, s: Suit) -> Card {
        Card::new(r, s)
    }

    fn hand_all(s: Suit) -> [Card; 4] {
        [
            c(Rank::Two, s),
            c(Rank::Five, s),
            c(Rank::Seven, s),
            c(Rank::King, s),
        ]
    }

    #[test]
    fn non_crib_hand_flush_of_four_scores_four() {
        let hand = hand_all(Suit::Hearts);
        let starter = c(Rank::Nine, Suit::Clubs);
        assert_eq!(score(hand, starter, false), 4);
    }

    #[test]
    fn non_crib_hand_flush_of_five_scores_five() {
        let hand = hand_all(Suit::Hearts);
        let starter = c(Rank::Nine, Suit::Hearts);
        assert_eq!(score(hand, starter, false), 5);
    }

    #[test]
    fn crib_flush_of_four_scores_zero() {
        let hand = hand_all(Suit::Hearts);
        let starter = c(Rank::Nine, Suit::Clubs);
        assert_eq!(score(hand, starter, true), 0);
    }

    #[test]
    fn crib_flush_of_five_scores_five() {
        let hand = hand_all(Suit::Hearts);
        let starter = c(Rank::Nine, Suit::Hearts);
        assert_eq!(score(hand, starter, true), 5);
    }

    #[test]
    fn mixed_suit_hand_scores_zero_regardless_of_crib_flag() {
        let hand = [
            c(Rank::Two, Suit::Hearts),
            c(Rank::Five, Suit::Hearts),
            c(Rank::Seven, Suit::Hearts),
            c(Rank::King, Suit::Clubs),
        ];
        let starter = c(Rank::Nine, Suit::Hearts);
        assert_eq!(score(hand, starter, false), 0);
        assert_eq!(score(hand, starter, true), 0);
    }
}

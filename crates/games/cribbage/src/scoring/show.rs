//! Show-phase scoring composer.
//!
//! At the end of pegging, each counting unit (non-dealer hand, dealer
//! hand, dealer crib) is scored by combining its 4 cards with the
//! common starter. The five sub-rules
//! ([`fifteens`](crate::scoring::fifteens),
//! [`pairs`](crate::scoring::pairs),
//! [`runs`](crate::scoring::runs),
//! [`flush`](crate::scoring::flush),
//! [`nobs`](crate::scoring::nobs)) are pure and independent; this
//! composer just sums them.

use serde::{Deserialize, Serialize};

use crate::card::Card;
use crate::scoring::{fifteens, flush, nobs, pairs, runs};

/// Result of scoring a single 4-card hand (or crib) against a starter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShowScore {
    pub fifteens: u8,
    pub pairs: u8,
    pub runs: u8,
    pub flush: u8,
    pub nobs: u8,
    pub total: u8,
}

/// Score a 4-card hand (or crib) against the starter. `is_crib`
/// toggles the stricter crib-flush rule (see [`flush`]).
#[must_use]
pub fn score_hand(hand: [Card; 4], starter: Card, is_crib: bool) -> ShowScore {
    let all_five = [hand[0], hand[1], hand[2], hand[3], starter];
    let f = fifteens::score(&all_five);
    let p = pairs::score(&all_five);
    let r = runs::score(&all_five);
    let fl = flush::score(hand, starter, is_crib);
    let nobs_pts = if is_crib {
        0
    } else {
        nobs::score(hand, starter)
    };
    let total = f
        .saturating_add(p)
        .saturating_add(r)
        .saturating_add(fl)
        .saturating_add(nobs_pts);
    ShowScore {
        fifteens: f,
        pairs: p,
        runs: r,
        flush: fl,
        nobs: nobs_pts,
        total,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::{Rank, Suit};

    fn c(r: Rank, s: Suit) -> Card {
        Card::new(r, s)
    }

    #[test]
    fn twenty_nine_hand_is_maximum() {
        // J♠ 5♣ 5♦ 5♥ + starter 5♠
        let hand = [
            c(Rank::Jack, Suit::Spades),
            c(Rank::Five, Suit::Clubs),
            c(Rank::Five, Suit::Diamonds),
            c(Rank::Five, Suit::Hearts),
        ];
        let starter = c(Rank::Five, Suit::Spades);
        let score = score_hand(hand, starter, false);
        assert_eq!(score.fifteens, 16);
        assert_eq!(score.pairs, 12);
        assert_eq!(score.runs, 0);
        assert_eq!(score.flush, 0);
        assert_eq!(score.nobs, 1);
        assert_eq!(score.total, 29);
    }

    #[test]
    fn dead_hand_scores_zero() {
        // Plan example: 2♠ 4♥ 6♦ K♣ + starter Q♥
        let hand = [
            c(Rank::Two, Suit::Spades),
            c(Rank::Four, Suit::Hearts),
            c(Rank::Six, Suit::Diamonds),
            c(Rank::King, Suit::Clubs),
        ];
        let starter = c(Rank::Queen, Suit::Hearts);
        let score = score_hand(hand, starter, false);
        assert_eq!(score.total, 0);
    }

    #[test]
    fn crib_never_counts_nobs_even_if_jack_matches_starter() {
        // A Jack in the crib matching the starter *would* be nobs in
        // a hand, but not in the crib. (Standard cribbage rule —
        // nobs is a hand-only bonus.)
        let crib = [
            c(Rank::Jack, Suit::Hearts),
            c(Rank::Two, Suit::Clubs),
            c(Rank::Three, Suit::Diamonds),
            c(Rank::Seven, Suit::Spades),
        ];
        let starter = c(Rank::Five, Suit::Hearts);
        let score = score_hand(crib, starter, true);
        assert_eq!(score.nobs, 0);
    }
}
